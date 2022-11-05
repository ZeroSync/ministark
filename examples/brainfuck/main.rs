#![feature(allocator_api, const_if_match)]

use air::BrainfuckAir;
use air::ExecutionInfo;
// use ark_ff::CubicExtConfig;
// use ark_ff::Fp3;
// use ark_ff::Fp3Config;
use ark_ff::One;
use ark_ff_optimized::fp64::Fp;
use ministark::ProofOptions;
use ministark::Prover;
use ministark::Trace;
use std::time::Instant;
use trace::BrainfuckTrace;
use vm::compile;
use vm::simulate;

mod air;
mod constraints;
mod cubic_extension;
mod tables;
mod trace;
mod vm;

/// Source: http://esoteric.sange.fi/brainfuck/bf-source/prog/fibonacci.txt
const FIB_TO_55_SOURCE: &str = "
This determines how many numbers to generate:
    +++++++++

Program:
    >+>>>>++++++++++++++++++++++++++++++++++++++++++++
    >++++++++++++++++++++++++++++++++<<<<<<[>[>>>>>>+>
    +<<<<<<<-]>>>>>>>[<<<<<<<+>>>>>>>-]<[>++++++++++[-
    <-[>>+>+<<<-]>>>[<<<+>>>-]+<[>[-]<[-]]>[<<[>>>+<<<
    -]>>[-]]<<]>>>[>>+>+<<<-]>>>[<<<+>>>-]+<[>[-]<[-]]
    >[<<+>>[-]]<<<<<<<]>>>>>[+++++++++++++++++++++++++
    +++++++++++++++++++++++.[-]]++++++++++<[->-<]>++++
    ++++++++++++++++++++++++++++++++++++++++++++.[-]<<
    <<<<<<<<<<[>>>+>+<<<<-]>>>>[<<<<+>>>>-]<-[>>.>.<<<
    [-]]<<[>>+>+<<<-]>>>[<<<+>>>-]<<[<+>-]>[<+>-]<<<-]
";

/// Source: https://esolangs.org/wiki/Brainfuck
const HELLO_WORLD_SOURCE: &str = "
+++++ +++++             initialize counter (cell #0) to 10
[                       use loop to set 70/100/30/10
    > +++++ ++              add  7 to cell #1
    > +++++ +++++           add 10 to cell #2
    > +++                   add  3 to cell #3
    > +                     add  1 to cell #4
<<<< -                  decrement counter (cell #0)
]
> ++ .                  print 'H'
> + .                   print 'e'
+++++ ++ .              print 'l'
.                       print 'l'
+++ .                   print 'o'
> ++ .                  print ' '
<< +++++ +++++ +++++ .  print 'W'
> .                     print 'o'
+++ .                   print 'r'
----- - .               print 'l'
----- --- .             print 'd'
> + .                   print '!'
> .                     print '\n'
";

// pub type Fq3 = Fp3<Fq3Config>;

// pub struct Fq3Config;

// impl Fp3Config for Fq3Config {
//     type Fp;

//     const NONRESIDUE: Self::Fp;

//     const FROBENIUS_COEFF_FP3_C1: &'static [Self::Fp];

//     const FROBENIUS_COEFF_FP3_C2: &'static [Self::Fp];

//     const TWO_ADICITY: u32;

//     const TRACE_MINUS_ONE_DIV_TWO: &'static [u64];

//     const QUADRATIC_NONRESIDUE_TO_T: ark_ff::Fp3<Self>;
// }

struct BrainfuckProver(ProofOptions);

impl Prover for BrainfuckProver {
    type Fp = Fp;
    type Fq = cubic_extension::WrappedFq3;
    type Air = BrainfuckAir;
    type Trace = BrainfuckTrace;

    fn new(options: ProofOptions) -> Self {
        BrainfuckProver(options)
    }

    fn options(&self) -> ProofOptions {
        self.0
    }

    fn get_pub_inputs(&self, trace: &BrainfuckTrace) -> ExecutionInfo {
        ExecutionInfo {
            execution_len: trace.base_columns().num_rows(),
            // TODO: add inputs
            input: Vec::new(),
            output: Vec::new(),
        }
    }
}

fn main() {
    println!("{:?}", Fp::one());

    let now = Instant::now();
    let program = compile(HELLO_WORLD_SOURCE);
    let mut output = Vec::new();
    let trace = simulate(&program, &mut std::io::empty(), &mut output);
    println!("Output: {}", String::from_utf8(output).unwrap());

    let options = ProofOptions::new(32, 16, 8, 8, 64);
    let prover = BrainfuckProver::new(options);
    let proof = prover.generate_proof(trace);
    println!("Runtime: {:?}", now.elapsed());
    let proof = proof.unwrap();
    // let mut proof_bytes = Vec::new();
    //     .serialize_compressed(&mut proof_bytes)
    //     .unwrap();
    // println!("Result: {:?}kb", proof_bytes.len() / 1024);
    proof.verify().unwrap();
}
