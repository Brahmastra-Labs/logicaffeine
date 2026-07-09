//! Regression: the literal pool straddling a 4 KiB boundary.
//!
//! An arm64 GOT load is an ADRP+LDR pair — TWO relocations resolving ONE
//! hole. They must share a single pointer slot: the ADRP contributes the
//! 4 KiB page and the LDR the low 12 bits, so two adjacent per-reloc slots
//! compose into a garbage address as soon as they land on different pages.
//! This drove a SIGSEGV the moment a chain's pool crossed 4 KiB (~22 ops).

use logicaffeine_forge::jit::{compile_straightline, reference_eval, MicroOp};

#[test]
fn chains_run_correctly_across_the_4k_page_boundary() {
    // Sweep sizes so the code+pool boundary crosses 4 KiB inside the sweep
    // (and 8 KiB for good measure), verifying results — not just survival.
    for n in 1..=60usize {
        let mut ops: Vec<MicroOp> = Vec::new();
        for k in 0..n {
            ops.push(MicroOp::LoadConst { dst: 0, value: k as i64 });
            ops.push(MicroOp::Add { dst: 1, lhs: 1, rhs: 0 });
        }
        ops.push(MicroOp::Return { src: 1 });

        let chain = compile_straightline(&ops).expect("compile");
        let mut jit_frame = vec![0i64; 8];
        let jit = chain.run_with_frame(&mut jit_frame).expect_return();

        let mut ref_frame = vec![0i64; 8];
        let reference = reference_eval(&ops, &mut ref_frame, 1_000_000).expect("fuel");

        assert_eq!(jit, reference, "n={n} (mapping {} bytes)", chain.bytes().len());
        assert_eq!(jit_frame, ref_frame, "n={n} frame");
    }
}
