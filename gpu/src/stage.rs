#![cfg(all(target_arch = "aarch64", target_os = "macos"))]
use super::GpuField;
use crate::plan::get_planner;
use crate::prelude::buffer_mut_no_copy;
use crate::utils::buffer_no_copy;
#[cfg(feature = "arkworks")]
use crate::utils::distribute_powers;
use crate::utils::page_aligned_uninit_vector;
use crate::utils::void_ptr;
use crate::GpuAdd;
use crate::GpuFrom;
use crate::GpuMul;
use alloc::string::String;
use alloc::vec::Vec;
use core::marker::PhantomData;
use core::mem::size_of;
use metal::NSUInteger;

#[derive(Clone, Copy, Debug)]
pub enum FftVariant {
    Multiple,
    Single,
}

/// GPU FFT kernel name as declared at the bottom of `fft.metal`
fn fft_kernel_name<F: GpuField>(variant: FftVariant) -> String {
    alloc::format!(
        "fft_{}_{}",
        match variant {
            FftVariant::Multiple => "multiple",
            FftVariant::Single => "single",
        },
        F::field_name()
    )
}

pub struct FftGpuStage<E> {
    variant: FftVariant,
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    threadgroup_fft_size: usize,
    _phantom: PhantomData<E>,
}

impl<F: GpuField> FftGpuStage<F> {
    pub fn new(
        library: &metal::LibraryRef,
        n: usize,
        num_boxes: usize,
        variant: FftVariant,
        threadgroup_fft_size: usize,
    ) -> FftGpuStage<F> {
        use metal::MTLDataType::UInt;
        assert!(n.is_power_of_two());
        assert!(num_boxes.is_power_of_two());
        assert!(threadgroup_fft_size.is_power_of_two());
        assert!(num_boxes < n);
        assert!((2048..=1073741824).contains(&n));

        // Create the compute pipeline
        let fft_constants = metal::FunctionConstantValues::new();
        let n = n as u32;
        let num_boxes = num_boxes as u32;
        let tg_fft_size = threadgroup_fft_size as u32;
        fft_constants.set_constant_value_at_index(void_ptr(&n), UInt, 0);
        fft_constants.set_constant_value_at_index(void_ptr(&num_boxes), UInt, 1);
        fft_constants.set_constant_value_at_index(void_ptr(&tg_fft_size), UInt, 2);
        let func = library
            .get_function(&fft_kernel_name::<F>(variant), Some(fft_constants))
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();
        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        // TODO: figure out a solution to handle if this arises
        assert!(threadgroup_fft_size / 2 <= max_threadgroup_threads as usize);

        // each thread operates on two values each round
        let threadgroup_dim = metal::MTLSize::new((tg_fft_size / 2).try_into().unwrap(), 1, 1);
        let grid_dim = metal::MTLSize::new((n / 2).try_into().unwrap(), 1, 1);

        FftGpuStage {
            variant,
            pipeline,
            threadgroup_dim,
            grid_dim,
            threadgroup_fft_size,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        input_buffer: &mut metal::BufferRef,
        twiddles_buffer: &metal::BufferRef,
    ) {
        let command_encoder = command_buffer.new_compute_command_encoder();
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        if let FftVariant::Multiple = self.variant {
            let field_size = size_of::<F>();
            let num_bytes = (self.threadgroup_fft_size * field_size).try_into().unwrap();
            command_encoder.set_threadgroup_memory_length(0, num_bytes);
        }
        command_encoder.set_buffer(0, Some(input_buffer), 0);
        command_encoder.set_buffer(1, Some(twiddles_buffer), 0);
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[input_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct MulIntoStage<LhsF, RhsF = LhsF> {
    n: u32,
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<(LhsF, RhsF)>,
}

impl<LhsF: GpuField + GpuMul<RhsF>, RhsF: GpuField> MulIntoStage<LhsF, RhsF> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let constants = metal::FunctionConstantValues::new();
        let n = n as u32;
        constants.set_constant_value_at_index(void_ptr(&n), metal::MTLDataType::UInt, 0);
        let kernel_name = alloc::format!(
            "mul_into_LHS_{}_RHS_{}",
            LhsF::field_name(),
            RhsF::field_name()
        );
        let func = library.get_function(&kernel_name, Some(constants)).unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        MulIntoStage {
            n,
            pipeline,
            threadgroup_dim,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst: &metal::BufferRef,
        lhs: &metal::BufferRef,
        rhs: &metal::BufferRef,
        shift: isize,
    ) {
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst), 0);
        command_encoder.set_buffer(1, Some(lhs), 0);
        command_encoder.set_buffer(2, Some(rhs), 0);
        let shift = ((self.n as isize + shift) % (self.n as isize)) as u32;
        command_encoder.set_bytes(3, size_of::<u32>().try_into().unwrap(), void_ptr(&shift));
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst, lhs, rhs]);
        command_encoder.end_encoding()
    }
}

pub struct MulAssignStage<LhsF, RhsF = LhsF> {
    n: u32,
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<(LhsF, RhsF)>,
}

impl<LhsF: GpuField + GpuMul<RhsF>, RhsF: GpuField> MulAssignStage<LhsF, RhsF> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let constants = metal::FunctionConstantValues::new();
        let n = n as u32;
        constants.set_constant_value_at_index(void_ptr(&n), metal::MTLDataType::UInt, 0);
        let kernel_name = alloc::format!(
            "mul_assign_LHS_{}_RHS_{}",
            LhsF::field_name(),
            RhsF::field_name()
        );
        let func = library.get_function(&kernel_name, Some(constants)).unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        MulAssignStage {
            n,
            pipeline,
            threadgroup_dim,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        lhs: &metal::BufferRef,
        rhs: &metal::BufferRef,
        shift: isize,
    ) {
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(lhs), 0);
        command_encoder.set_buffer(1, Some(rhs), 0);
        let shift = ((self.n as isize + shift) % (self.n as isize)) as u32;
        command_encoder.set_bytes(2, size_of::<u32>().try_into().unwrap(), void_ptr(&shift));
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[lhs, rhs]);
        command_encoder.end_encoding()
    }
}

#[cfg(feature = "arkworks")]
pub struct ScaleAndNormalizeGpuStage<LhsF, RhsF = LhsF> {
    mul_assign_stage: MulAssignStage<LhsF, RhsF>,
    _scale_factors: Vec<RhsF>,
    scale_factors_buffer: metal::Buffer,
}

// TODO: replace `Field` with `One + PartialEq` to support multiple libraries
#[cfg(feature = "arkworks")]
impl<LhsF: GpuField + GpuMul<RhsF>, RhsF: GpuField + ark_ff::Field + PartialEq + Copy>
    ScaleAndNormalizeGpuStage<LhsF, RhsF>
{
    pub fn new(
        library: &metal::LibraryRef,
        command_queue: &metal::CommandQueue,
        n: usize,
        scale_factor: RhsF,
        norm_factor: RhsF,
    ) -> Self {
        let mul_assign_stage = MulAssignStage::<LhsF, RhsF>::new(library, n);
        let mut _scale_factors = unsafe { page_aligned_uninit_vector(n) };
        _scale_factors.fill(norm_factor);
        if !scale_factor.is_one() {
            distribute_powers(&mut _scale_factors, scale_factor);
        }
        let scale_factors_buffer = buffer_no_copy(command_queue.device(), &_scale_factors);

        ScaleAndNormalizeGpuStage {
            mul_assign_stage,
            _scale_factors,
            scale_factors_buffer,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        input_buffer: &metal::BufferRef,
    ) {
        self.mul_assign_stage
            .encode(command_buffer, input_buffer, &self.scale_factors_buffer, 0);
    }
}

/// FFT stage to perform a bit reversal of an input array in place
pub struct BitReverseGpuStage<F> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<F>,
}

impl<F: GpuField> BitReverseGpuStage<F> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        use metal::MTLDataType::UInt;
        assert!(n.is_power_of_two());
        assert!((2048..=1073741824).contains(&n));

        // Create the compute pipeline
        let fft_constants = metal::FunctionConstantValues::new();
        let n = n as u32;
        let num_boxes = 5u32;
        fft_constants.set_constant_value_at_index(void_ptr(&n), UInt, 0);
        fft_constants.set_constant_value_at_index(void_ptr(&num_boxes), UInt, 1);
        let kernel_name = alloc::format!("bit_reverse_{}", F::field_name());
        let func = library
            .get_function(&kernel_name, Some(fft_constants))
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        BitReverseGpuStage {
            pipeline,
            threadgroup_dim,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        input_buffer: &mut metal::BufferRef,
    ) {
        let command_encoder = command_buffer.new_compute_command_encoder();
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(input_buffer), 0);
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[input_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct MulPowStage<LhsF, RhsF = LhsF> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<(LhsF, RhsF)>,
}

impl<LhsF: GpuField + GpuMul<RhsF>, RhsF: GpuField> MulPowStage<LhsF, RhsF> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let constants = metal::FunctionConstantValues::new();
        let n = n as u32;
        constants.set_constant_value_at_index(void_ptr(&n), metal::MTLDataType::UInt, 0);
        let kernel_name = alloc::format!(
            "mul_pow_LHS_{}_RHS_{}",
            LhsF::field_name(),
            RhsF::field_name()
        );
        let func = library.get_function(&kernel_name, Some(constants)).unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        // TODO: remove
        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        MulPowStage {
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst_buffer: &mut metal::BufferRef,
        src_buffer: &metal::BufferRef,
        power: usize,
        shift: usize,
    ) {
        let command_encoder = command_buffer.new_compute_command_encoder();
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.set_buffer(1, Some(src_buffer), 0);
        let power = power as u32;
        command_encoder.set_bytes(2, size_of::<u32>().try_into().unwrap(), void_ptr(&power));
        let shift = shift as u32;
        command_encoder.set_bytes(3, size_of::<u32>().try_into().unwrap(), void_ptr(&shift));
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[src_buffer, dst_buffer]);
        command_encoder.end_encoding();
    }
}

pub struct AddAssignStage<LhsF, RhsF = LhsF> {
    n: u32,
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<(LhsF, RhsF)>,
}

impl<LhsF: GpuField + GpuAdd<RhsF>, RhsF: GpuField> AddAssignStage<LhsF, RhsF> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        let constants = metal::FunctionConstantValues::new();
        let n = n as u32;
        constants.set_constant_value_at_index(void_ptr(&n), metal::MTLDataType::UInt, 0);
        // Create the compute pipeline
        let func = library
            .get_function(
                &alloc::format!(
                    "add_assign_LHS_{}_RHS_{}",
                    LhsF::field_name(),
                    RhsF::field_name()
                ),
                Some(constants),
            )
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        AddAssignStage {
            n,
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst_buffer: &metal::BufferRef,
        src_buffer: &metal::BufferRef,
        shift: isize,
    ) {
        // TODO: why is `metal::MTLDispatchType::Concurrent` slower?
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.set_buffer(1, Some(src_buffer), 0);
        let shift = ((self.n as isize + shift) % (self.n as isize)) as u32;
        command_encoder.set_bytes(2, size_of::<u32>().try_into().unwrap(), void_ptr(&shift));
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer, src_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct AddIntoStage<LhsF, RhsF = LhsF> {
    n: u32,
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<(LhsF, RhsF)>,
}

impl<LhsF: GpuField + GpuAdd<RhsF>, RhsF: GpuField> AddIntoStage<LhsF, RhsF> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        let constants = metal::FunctionConstantValues::new();
        let n = n as u32;
        constants.set_constant_value_at_index(void_ptr(&n), metal::MTLDataType::UInt, 0);
        // Create the compute pipeline
        let func = library
            .get_function(
                &alloc::format!(
                    "add_into_LHS_{}_RHS_{}",
                    LhsF::field_name(),
                    RhsF::field_name()
                ),
                Some(constants),
            )
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        AddIntoStage {
            n,
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst_buffer: &metal::BufferRef,
        lhs_buffer: &metal::BufferRef,
        rhs_buffer: &metal::BufferRef,
        shift: isize,
    ) {
        // TODO: why is `metal::MTLDispatchType::Concurrent` slower?
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.set_buffer(1, Some(lhs_buffer), 0);
        command_encoder.set_buffer(2, Some(rhs_buffer), 0);
        let shift = ((self.n as isize + shift) % (self.n as isize)) as u32;
        command_encoder.set_bytes(3, size_of::<u32>().try_into().unwrap(), void_ptr(&shift));
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer, lhs_buffer, rhs_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct AddIntoConstStage<LhsF, RhsF = LhsF> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<(LhsF, RhsF)>,
}

impl<LhsF: GpuField + GpuAdd<RhsF>, RhsF: GpuField> AddIntoConstStage<LhsF, RhsF> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let func = library
            .get_function(
                &alloc::format!(
                    "add_into_const_LHS_{}_RHS_{}",
                    LhsF::field_name(),
                    RhsF::field_name()
                ),
                None,
            )
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        AddIntoConstStage {
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst_buffer: &metal::BufferRef,
        lhs_buffer: &metal::BufferRef,
        rhs_val: RhsF,
    ) {
        // TODO: why is `metal::MTLDispatchType::Concurrent` slower?
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.set_buffer(1, Some(lhs_buffer), 0);
        command_encoder.set_bytes(2, size_of::<RhsF>().try_into().unwrap(), void_ptr(&rhs_val));
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer, lhs_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct ConvertIntoStage<LhsF, RhsF = LhsF> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<(LhsF, RhsF)>,
}

impl<LhsF: GpuField + GpuFrom<RhsF>, RhsF: GpuField> ConvertIntoStage<LhsF, RhsF> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let func = library
            .get_function(
                &alloc::format!(
                    "convert_into_LHS_{}_RHS_{}",
                    LhsF::field_name(),
                    RhsF::field_name()
                ),
                None,
            )
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        ConvertIntoStage {
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst_buffer: &metal::BufferRef,
        src_buffer: &metal::BufferRef,
    ) {
        // TODO: why is `metal::MTLDispatchType::Concurrent` slower?
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.set_buffer(1, Some(src_buffer), 0);
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer, src_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct AddAssignConstStage<LhsF, RhsF = LhsF> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<(LhsF, RhsF)>,
}

impl<LhsF: GpuField + GpuAdd<RhsF>, RhsF: GpuField> AddAssignConstStage<LhsF, RhsF> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let func = library
            .get_function(
                &alloc::format!(
                    "add_assign_const_LHS_{}_RHS_{}",
                    LhsF::field_name(),
                    RhsF::field_name()
                ),
                None,
            )
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        AddAssignConstStage {
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst_buffer: &metal::BufferRef,
        val: &RhsF,
    ) {
        // TODO: why is `metal::MTLDispatchType::Concurrent` slower?
        // let command_encoder = command_buffer.new_compute_command_encoder();
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.set_bytes(1, size_of::<RhsF>().try_into().unwrap(), void_ptr(val));
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct MulIntoConstStage<LhsF, RhsF = LhsF> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<(LhsF, RhsF)>,
}

impl<LhsF: GpuField + GpuMul<RhsF>, RhsF: GpuField> MulIntoConstStage<LhsF, RhsF> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let func = library
            .get_function(
                &alloc::format!(
                    "mul_into_const_LHS_{}_RHS_{}",
                    LhsF::field_name(),
                    RhsF::field_name()
                ),
                None,
            )
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        MulIntoConstStage {
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst_buffer: &metal::BufferRef,
        lhs_buffer: &metal::BufferRef,
        rhs_val: &RhsF,
    ) {
        // TODO: why is `metal::MTLDispatchType::Concurrent` slower?
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.set_buffer(1, Some(lhs_buffer), 0);
        command_encoder.set_bytes(2, size_of::<RhsF>().try_into().unwrap(), void_ptr(rhs_val));
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct MulAssignConstStage<LhsF, RhsF = LhsF> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<(LhsF, RhsF)>,
}

impl<LhsF: GpuField + GpuMul<RhsF>, RhsF: GpuField> MulAssignConstStage<LhsF, RhsF> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let func = library
            .get_function(
                &alloc::format!(
                    "mul_assign_const_LHS_{}_RHS_{}",
                    LhsF::field_name(),
                    RhsF::field_name()
                ),
                None,
            )
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        MulAssignConstStage {
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst_buffer: &metal::BufferRef,
        val: RhsF,
    ) {
        // TODO: why is `metal::MTLDispatchType::Concurrent` slower?
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.set_bytes(1, size_of::<RhsF>().try_into().unwrap(), void_ptr(&val));
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct InverseInPlaceStage<F> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<F>,
}

impl<F: GpuField> InverseInPlaceStage<F> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let func = library
            .get_function(
                &alloc::format!("inverse_in_place_{}", F::field_name()),
                None,
            )
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let n = n as u32;
        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        InverseInPlaceStage {
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(&self, command_buffer: &metal::CommandBufferRef, dst_buffer: &metal::BufferRef) {
        // TODO: why is `metal::MTLDispatchType::Concurrent` slower?
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct NegInPlaceStage<F> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<F>,
}

impl<F: GpuField> NegInPlaceStage<F> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let func = library
            .get_function(&alloc::format!("neg_in_place_{}", F::field_name()), None)
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let n = n as u32;
        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        NegInPlaceStage {
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(&self, command_buffer: &metal::CommandBufferRef, dst_buffer: &metal::BufferRef) {
        // TODO: why is `metal::MTLDispatchType::Concurrent` slower?
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct NegIntoStage<F> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<F>,
}

impl<F: GpuField> NegIntoStage<F> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let func = library
            .get_function(&alloc::format!("neg_into_{}", F::field_name()), None)
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let n = n as u32;
        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        NegIntoStage {
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst_buffer: &metal::BufferRef,
        src_buffer: &metal::BufferRef,
    ) {
        // TODO: why is `metal::MTLDispatchType::Concurrent` slower?
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.set_buffer(1, Some(src_buffer), 0);
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer, src_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct InverseIntoStage<F> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<F>,
}

impl<F: GpuField> InverseIntoStage<F> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let func = library
            .get_function(&alloc::format!("inverse_into_{}", F::field_name()), None)
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let n = n as u32;
        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        InverseIntoStage {
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst_buffer: &metal::BufferRef,
        src_buffer: &metal::BufferRef,
    ) {
        // TODO: why is `metal::MTLDispatchType::Concurrent` slower?
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.set_buffer(1, Some(src_buffer), 0);
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer, src_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct ExpIntoStage<F> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<F>,
}

impl<F: GpuField> ExpIntoStage<F> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let func = library
            .get_function(&alloc::format!("exp_into_{}", F::field_name()), None)
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let n = n as u32;
        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        ExpIntoStage {
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst_buffer: &metal::BufferRef,
        src_buffer: &metal::BufferRef,
        exponent: usize,
    ) {
        // TODO: why is `metal::MTLDispatchType::Concurrent` slower?
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.set_buffer(1, Some(src_buffer), 0);
        let expoonent = u32::try_from(exponent).unwrap();
        command_encoder.set_bytes(
            2,
            size_of::<u32>().try_into().unwrap(),
            void_ptr(&expoonent),
        );
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer, src_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct ExpInPlaceStage<F> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<F>,
}

impl<F: GpuField> ExpInPlaceStage<F> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let func = library
            .get_function(&alloc::format!("exp_in_place_{}", F::field_name()), None)
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let n = n as u32;
        let max_threadgroup_threads = pipeline.max_total_threads_per_threadgroup();
        let threadgroup_dim = metal::MTLSize::new(max_threadgroup_threads, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        ExpInPlaceStage {
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst_buffer: &metal::BufferRef,
        exponent: usize,
    ) {
        // TODO: why is `metal::MTLDispatchType::Concurrent` slower?
        // let command_encoder = command_buffer.new_compute_command_encoder();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        let expoonent = u32::try_from(exponent).unwrap();
        command_encoder.set_bytes(
            1,
            size_of::<u32>().try_into().unwrap(),
            void_ptr(&expoonent),
        );
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct FillBuffStage<F> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<F>,
}

impl<F: GpuField> FillBuffStage<F> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let func = library
            .get_function(&alloc::format!("fill_buff_{}", F::field_name()), None)
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let n = n as u32;
        let threadgroup_dim = metal::MTLSize::new(1024, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        FillBuffStage {
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst_buffer: &mut metal::BufferRef,
        value: F,
    ) {
        let command_encoder = command_buffer.new_compute_command_encoder();
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.set_bytes(1, size_of::<F>().try_into().unwrap(), void_ptr(&value));
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct GenerateTwiddlesStage<F> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<F>,
}

impl<F: GpuField> GenerateTwiddlesStage<F> {
    pub fn new(library: &metal::LibraryRef, n: usize) -> Self {
        // Create the compute pipeline
        let constants = metal::FunctionConstantValues::new();
        let n = n as u32;
        constants.set_constant_value_at_index(
            &n as *const u32 as *const core::ffi::c_void,
            metal::MTLDataType::UInt,
            0,
        );
        let func = library
            .get_function(
                &alloc::format!("generate_twiddles_{}", F::field_name()),
                Some(constants),
            )
            .unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let threadgroup_dim = metal::MTLSize::new(1024, 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        GenerateTwiddlesStage {
            threadgroup_dim,
            pipeline,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        dst_buffer: &mut metal::BufferRef,
        value: F,
    ) {
        let command_encoder = command_buffer.new_compute_command_encoder();
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(dst_buffer), 0);
        command_encoder.set_bytes(1, size_of::<F>().try_into().unwrap(), void_ptr(&value));
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[dst_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct Rpo256AbsorbColumnsStage<F: GpuField> {
    n: usize,
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _states: Vec<[F; 4]>,
    states_buffer: metal::Buffer,
    pub digests: Vec<[F; 4]>,
    digests_buffer: metal::Buffer,
}

impl<F: GpuField + From<u32> + Copy> Rpo256AbsorbColumnsStage<F> {
    const HASHERS_PER_THREADGROUP: usize = 64;

    pub fn new(library: &metal::LibraryRef, n: usize, requires_padding: bool) -> Self {
        let kernel_name = alloc::format!("rpo_256_absorb_columns_and_permute_{}", F::field_name());
        let func = library.get_function(&kernel_name, None).unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let threadgroup_dim =
            metal::MTLSize::new(Self::HASHERS_PER_THREADGROUP.try_into().unwrap(), 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        // TODO: creating page aligned vectors in this fashion is rather brittle.
        // If the vector is resized there is no garuntee that the new memory will be
        // page aligned. Rust's Allocator api would be great but it's not currently
        // available on Rust Stable.
        let mut digests = unsafe { page_aligned_uninit_vector(n) };
        let digests_buffer = buffer_mut_no_copy(library.device(), &mut digests);

        let mut _states = unsafe { page_aligned_uninit_vector(n) };
        _states.fill([
            // apply RPO's padding rule
            F::from(if requires_padding { 1 } else { 0 }),
            F::from(0),
            F::from(0),
            F::from(0),
        ]);
        let states_buffer = buffer_mut_no_copy(library.device(), &mut _states);

        Rpo256AbsorbColumnsStage {
            n,
            threadgroup_dim,
            pipeline,
            grid_dim,
            digests,
            digests_buffer,
            _states,
            states_buffer,
        }
    }

    pub fn encode(&self, command_buffer: &metal::CommandBufferRef, columns: [&[F]; 8]) {
        let [col0, col1, col2, col3, col4, col5, col6, col7] = columns;
        assert_eq!(self.n, col1.len());
        assert_eq!(self.n, col2.len());
        assert_eq!(self.n, col3.len());
        assert_eq!(self.n, col4.len());
        assert_eq!(self.n, col5.len());
        assert_eq!(self.n, col6.len());
        assert_eq!(self.n, col7.len());

        let planner = get_planner();
        let device = planner.library.device();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        #[cfg(debug_assertions)]
        command_encoder.set_label("rpo absorb and permute 8 columns");
        let state_width = 16;
        let field_size = size_of::<F>() as NSUInteger;
        let mem_per_hasher = state_width * field_size;
        let hashers_per_tg = Self::HASHERS_PER_THREADGROUP as NSUInteger;
        command_encoder.set_threadgroup_memory_length(0, mem_per_hasher * hashers_per_tg * 2);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(&buffer_no_copy(device, col0)), 0);
        command_encoder.set_buffer(1, Some(&buffer_no_copy(device, col1)), 0);
        command_encoder.set_buffer(2, Some(&buffer_no_copy(device, col2)), 0);
        command_encoder.set_buffer(3, Some(&buffer_no_copy(device, col3)), 0);
        command_encoder.set_buffer(4, Some(&buffer_no_copy(device, col4)), 0);
        command_encoder.set_buffer(5, Some(&buffer_no_copy(device, col5)), 0);
        command_encoder.set_buffer(6, Some(&buffer_no_copy(device, col6)), 0);
        command_encoder.set_buffer(7, Some(&buffer_no_copy(device, col7)), 0);
        command_encoder.set_buffer(8, Some(&self.states_buffer), 0);
        command_encoder.set_buffer(9, Some(&self.digests_buffer), 0);
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[&self.states_buffer, &self.digests_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct Rpo256AbsorbRowsStage<F: GpuField> {
    n: usize,
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _states: Vec<[F; 4]>,
    states_buffer: metal::Buffer,
    pub digests: Vec<[F; 4]>,
    digests_buffer: metal::Buffer,
}

impl<F: GpuField + From<u32> + Copy> Rpo256AbsorbRowsStage<F> {
    const HASHERS_PER_THREADGROUP: usize = 128;

    pub fn new(library: &metal::LibraryRef, n: usize, requires_padding: bool) -> Self {
        let kernel_name = alloc::format!("rpo_256_absorb_rows_and_permute_{}", F::field_name());
        let func = library.get_function(&kernel_name, None).unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let threadgroup_dim =
            metal::MTLSize::new(Self::HASHERS_PER_THREADGROUP.try_into().unwrap(), 1, 1);
        let grid_dim = metal::MTLSize::new(n.try_into().unwrap(), 1, 1);

        let mut digests = unsafe { page_aligned_uninit_vector(n) };
        let digests_buffer = buffer_mut_no_copy(library.device(), &mut digests);

        let mut _states = unsafe { page_aligned_uninit_vector(n) };
        _states.fill([
            // apply RPO's padding rule
            F::from(if requires_padding { 1 } else { 0 }),
            F::from(0),
            F::from(0),
            F::from(0),
        ]);
        let states_buffer = buffer_mut_no_copy(library.device(), &mut _states);

        Rpo256AbsorbRowsStage {
            n,
            threadgroup_dim,
            pipeline,
            grid_dim,
            digests,
            digests_buffer,
            _states,
            states_buffer,
        }
    }

    pub fn encode(&self, command_buffer: &metal::CommandBufferRef, rows: &[[F; 8]]) {
        assert_eq!(self.n, rows.len());
        let planner = get_planner();
        let device = planner.library.device();
        let command_encoder = command_buffer
            .compute_command_encoder_with_dispatch_type(metal::MTLDispatchType::Concurrent);
        #[cfg(debug_assertions)]
        command_encoder.set_label("rpo absorb and permute 8 column rows");
        let state_width = 16;
        let field_size = size_of::<F>() as NSUInteger;
        let mem_per_hasher = state_width * field_size;
        let hashers_per_tg = Self::HASHERS_PER_THREADGROUP as NSUInteger;
        command_encoder.set_threadgroup_memory_length(0, mem_per_hasher * hashers_per_tg * 2);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(&buffer_no_copy(device, rows)), 0);
        command_encoder.set_buffer(1, Some(&self.states_buffer), 0);
        command_encoder.set_buffer(2, Some(&self.digests_buffer), 0);
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[&self.states_buffer, &self.digests_buffer]);
        command_encoder.end_encoding()
    }
}

pub struct Rpo256GenMerkleNodesFirstRowStage<F: GpuField> {
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    grid_dim: metal::MTLSize,
    _phantom: PhantomData<F>,
}

impl<F: GpuField> Rpo256GenMerkleNodesFirstRowStage<F> {
    pub const HASHERS_PER_THREADGROUP: usize = 64;

    pub fn new(library: &metal::LibraryRef, num_leaves: usize) -> Self {
        use metal::MTLDataType::UInt;
        assert!(num_leaves.is_power_of_two());
        assert!((num_leaves / 2) >= Self::HASHERS_PER_THREADGROUP);

        let constants = metal::FunctionConstantValues::new();
        constants.set_constant_value_at_index(void_ptr(&(num_leaves as u32)), UInt, 0);
        let kernel_name = alloc::format!("rpo_128_gen_merkle_nodes_first_row_{}", F::field_name());
        let func = library.get_function(&kernel_name, Some(constants)).unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let threadgroup_dim =
            metal::MTLSize::new(Self::HASHERS_PER_THREADGROUP.try_into().unwrap(), 1, 1);
        let grid_dim = metal::MTLSize::new((num_leaves / 2).try_into().unwrap(), 1, 1);

        Rpo256GenMerkleNodesFirstRowStage {
            pipeline,
            threadgroup_dim,
            grid_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        leaves: &metal::Buffer,
        nodes: &metal::Buffer,
    ) {
        let command_encoder = command_buffer.new_compute_command_encoder();
        // TODO: use param
        let state_width = 12;
        let field_size: NSUInteger = 8;
        assert_eq!(field_size as usize, size_of::<F>());
        let mem_per_hasher = state_width * field_size;
        let hashers_per_tg = Self::HASHERS_PER_THREADGROUP as NSUInteger;
        command_encoder.set_threadgroup_memory_length(0, mem_per_hasher * hashers_per_tg * 2);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(leaves), 0);
        command_encoder.set_buffer(1, Some(nodes), 0);
        command_encoder.dispatch_threads(self.grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[nodes]);
        command_encoder.end_encoding()
    }
}

pub struct Rpo256GenMerkleNodesRowStage<F: GpuField> {
    num_leaves: usize,
    pipeline: metal::ComputePipelineState,
    threadgroup_dim: metal::MTLSize,
    _phantom: PhantomData<F>,
}

impl<F: GpuField> Rpo256GenMerkleNodesRowStage<F> {
    pub const HASHERS_PER_THREADGROUP: usize = 32;

    pub fn new(library: &metal::LibraryRef, num_leaves: usize) -> Self {
        use metal::MTLDataType::UInt;
        assert!(num_leaves.is_power_of_two());

        let constants = metal::FunctionConstantValues::new();
        constants.set_constant_value_at_index(void_ptr(&(num_leaves as u32)), UInt, 0);
        let kernel_name = alloc::format!("rpo_128_gen_merkle_nodes_row_{}", F::field_name());
        let func = library.get_function(&kernel_name, Some(constants)).unwrap();
        let pipeline = library
            .device()
            .new_compute_pipeline_state_with_function(&func)
            .unwrap();

        let threadgroup_dim =
            metal::MTLSize::new(Self::HASHERS_PER_THREADGROUP as NSUInteger, 1, 1);

        Rpo256GenMerkleNodesRowStage {
            num_leaves,
            pipeline,
            threadgroup_dim,
            _phantom: PhantomData,
        }
    }

    pub fn encode(
        &self,
        command_buffer: &metal::CommandBufferRef,
        nodes: &metal::Buffer,
        row: u32,
    ) {
        assert_ne!(1, row, "use Rpo256GenMerkleNodesFirstRowStage");
        let command_encoder = command_buffer.new_compute_command_encoder();
        #[cfg(debug_assertions)]
        command_encoder.set_label(&alloc::format!("rpo merkle tree row={row}"));
        // TODO: use param
        let state_width = 12;
        let field_size: NSUInteger = 8;
        assert_eq!(field_size as usize, size_of::<F>());
        let mem_per_hasher = state_width * field_size;
        let hashers_per_tg = Self::HASHERS_PER_THREADGROUP as NSUInteger;
        command_encoder.set_threadgroup_memory_length(0, mem_per_hasher * hashers_per_tg * 2);
        command_encoder.set_compute_pipeline_state(&self.pipeline);
        command_encoder.set_buffer(0, Some(nodes), 0);
        command_encoder.set_bytes(1, size_of::<u32>() as NSUInteger, void_ptr(&row));
        let grid_dim = metal::MTLSize::new((self.num_leaves >> row).try_into().unwrap(), 1, 1);
        command_encoder.dispatch_threads(grid_dim, self.threadgroup_dim);
        command_encoder.memory_barrier_with_resources(&[nodes]);
        command_encoder.end_encoding()
    }
}
