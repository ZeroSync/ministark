pub mod p18446744069414584321 {
    #[cfg(feature = "winterfell")]
    pub mod winter {
        use crate::GpuAdd;
        use crate::GpuFftField;
        use crate::GpuField;
        use crate::GpuFrom;
        use crate::GpuMul;
        use alloc::string::String;
        use alloc::string::ToString;
        use winter_math::fields::f64::BaseElement;

        impl GpuField for BaseElement {
            type FftField = Self;

            fn field_name() -> String {
                "p18446744069414584321_fp".to_string()
            }
        }

        impl GpuFrom<Fp> for Fp {}

        impl GpuMul<BaseElement> for BaseElement {}

        impl GpuMul<&BaseElement> for BaseElement {}

        impl GpuAdd<BaseElement> for BaseElement {}

        impl GpuAdd<&BaseElement> for BaseElement {}

        impl GpuFftField for BaseElement {}
    }

    #[cfg(feature = "arkworks")]
    pub mod ark {
        use crate::GpuAdd;
        use crate::GpuFftField;
        use crate::GpuField;
        use crate::GpuFrom;
        use crate::GpuMul;
        use alloc::string::ToString;
        use ark_ff::BigInt;
        use ark_ff::Field;
        use ark_ff::Fp3;
        use ark_ff::Fp3Config;
        use ark_ff::FpConfig;
        use ark_ff_optimized::fp64;
        use ark_std::string::String;
        use core::marker::PhantomData;
        use core::ops::AddAssign;
        use core::ops::MulAssign;
        pub use fp64::Fp;
        pub use fp64::FpParams;

        impl GpuField for Fp {
            type FftField = Self;

            fn field_name() -> String {
                "p18446744069414584321_fp".to_string()
            }
        }

        impl GpuFrom<Fp> for Fp {}

        impl GpuMul<Fp> for Fp {}

        impl GpuMul<&Fp> for Fp {}

        impl GpuAdd<Fp> for Fp {}

        impl GpuAdd<&fp64::Fp> for fp64::Fp {}

        impl GpuFftField for fp64::Fp {}

        const TRACE: ark_ff::BigInt<3> =
            BigInt!("1461501636310055817916238417282618014431694553085");

        pub struct Fq3Config;

        impl Fp3Config for Fq3Config {
            type Fp = Fp;
            const NONRESIDUE: Fp = /* =2 */ ark_ff::Fp(BigInt([8589934590]), PhantomData);
            const TWO_ADICITY: u32 = FpParams::TWO_ADICITY;
            const TRACE_MINUS_ONE_DIV_TWO: &'static [u64] = &TRACE.divide_by_2_round_down().0;
            const QUADRATIC_NONRESIDUE_TO_T: Fp3<Fq3Config> = Fp3::<Fq3Config>::new(
                /* =16140901060737761281 */
                ark_ff::Fp(BigInt([2305843009213693952]), PhantomData),
                Fp::ZERO,
                Fp::ZERO,
            );

            // NOTE: these are used for pairings which I don't need so they are left empty
            const FROBENIUS_COEFF_FP3_C1: &'static [Fp] = &[];
            const FROBENIUS_COEFF_FP3_C2: &'static [Fp] = &[];
        }

        wrap_field!(Fq3; Fp3<Fq3Config>);

        impl MulAssign<&Fp> for Fq3 {
            fn mul_assign(&mut self, rhs: &Fp) {
                self.0.mul_assign_by_base_field(rhs)
            }
        }

        impl MulAssign<Fp> for Fq3 {
            fn mul_assign(&mut self, rhs: Fp) {
                self.0.mul_assign_by_base_field(&rhs)
            }
        }

        impl AddAssign<Fp> for Fq3 {
            fn add_assign(&mut self, rhs: Fp) {
                *self += Fq3::from(rhs);
            }
        }

        impl AddAssign<&Fp> for Fq3 {
            fn add_assign(&mut self, rhs: &Fp) {
                *self += Fq3::from(*rhs);
            }
        }

        impl core::ops::Add<&Fp> for Fq3 {
            type Output = Fq3;

            fn add(self, rhs: &Fp) -> Self::Output {
                self + Fq3::from(*rhs)
            }
        }

        impl core::ops::Add<Fp> for Fq3 {
            type Output = Fq3;

            fn add(self, rhs: Fp) -> Self::Output {
                self + Fq3::from(rhs)
            }
        }

        impl core::ops::SubAssign<Fp> for Fq3 {
            fn sub_assign(&mut self, rhs: Fp) {
                *self -= Fq3::from(rhs);
            }
        }

        impl core::ops::SubAssign<&Fp> for Fq3 {
            fn sub_assign(&mut self, rhs: &Fp) {
                *self -= Fq3::from(*rhs);
            }
        }

        impl core::ops::Sub<&Fp> for Fq3 {
            type Output = Fq3;

            fn sub(self, rhs: &Fp) -> Self::Output {
                self - Fq3::from(*rhs)
            }
        }

        impl core::ops::Sub<Fp> for Fq3 {
            type Output = Fq3;

            fn sub(self, rhs: Fp) -> Self::Output {
                self - Fq3::from(rhs)
            }
        }

        impl core::ops::Mul<&Fp> for Fq3 {
            type Output = Fq3;

            fn mul(mut self, rhs: &Fp) -> Self::Output {
                self.0.mul_assign_by_base_field(rhs);
                self
            }
        }

        impl core::ops::Mul<Fp> for Fq3 {
            type Output = Fq3;

            fn mul(mut self, rhs: Fp) -> Self::Output {
                self.0.mul_assign_by_base_field(&rhs);
                self
            }
        }

        impl From<Fp> for Fq3 {
            fn from(value: Fp) -> Self {
                Fq3(Fp3::<Fq3Config>::from_base_prime_field(value))
            }
        }

        impl GpuFrom<Fp> for Fq3 {}

        impl GpuFrom<Fq3> for Fq3 {}

        impl GpuMul<Fp> for Fq3 {}

        impl GpuMul<&Fp> for Fq3 {}

        impl GpuMul<Fq3> for Fq3 {}

        impl GpuMul<&Fq3> for Fq3 {}

        impl GpuAdd<Fp> for Fq3 {}

        impl GpuAdd<&Fp> for Fq3 {}

        impl GpuAdd<Fq3> for Fq3 {}

        impl GpuAdd<&Fq3> for Fq3 {}

        impl GpuField for Fq3 {
            type FftField = Fp;

            fn field_name() -> String {
                "p18446744069414584321_fq3".to_string()
            }
        }
    }
}

// StarkWare field
pub mod p3618502788666131213697322783095070105623107215331596699973092056135872020481 {
    pub const MODULUS: &str =
        "3618502788666131213697322783095070105623107215331596699973092056135872020481";

    #[cfg(feature = "arkworks")]
    pub mod ark {
        use crate::GpuAdd;
        use crate::GpuFftField;
        use crate::GpuField;
        use crate::GpuFrom;
        use crate::GpuMul;
        use alloc::string::ToString;
        use ark_ff::Fp256;
        use ark_ff::MontBackend;
        use ark_ff::MontConfig;
        use ark_std::string::String;

        #[derive(MontConfig)]
        #[modulus = "3618502788666131213697322783095070105623107215331596699973092056135872020481"]
        #[generator = "3"]
        pub struct FpMontConfig;

        /// The 252-bit prime field used by StarkWare for Cairo
        /// Field has modulus `2^251 + 17 * 2^192 + 1`
        pub type Fp = Fp256<MontBackend<FpMontConfig, 4>>;

        // TODO: GPU field implementation
        impl GpuField for Fp {
            type FftField = Self;

            fn field_name() -> String {
                "p3618502788666131213697322783095070105623107215331596699973092056135872020481_fp"
                    .to_string()
            }
        }

        impl GpuFrom<Fp> for Fp {}

        impl GpuMul<Fp> for Fp {}

        impl GpuMul<&Fp> for Fp {}

        impl GpuAdd<Fp> for Fp {}

        impl GpuAdd<&Fp> for Fp {}

        impl GpuFftField for Fp {}
    }
}
