//! M3 RED gate: 3-address fused stencils. Every ALU/move/const/branch
//! micro-op lowers to ONE stencil piece reading and writing frame slots
//! directly — the operand stack disappears from the hot path, cutting the
//! tail-jump count ~4× and the memory traffic ~3×. Compare-and-branch pairs
//! whose comparison scratch is dead fuse into a single branch piece.
//!
//! The gates here pin (a) the piece economics via `JitChain::bytes()` size
//! (a chain's code shrinks materially vs the stack model), and (b) full
//! semantic agreement of the fused forms with `reference_eval` and the VM.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_forge::jit::{compile_straightline, ChainOutcome, MicroOp, reference_eval};
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// SplitMix64 — deterministic seeds for the differential grids.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Random straight-line ALU programs: chain output must equal
/// `reference_eval` for the same frame, across many seeds.
#[test]
fn random_alu_programs_match_reference() {
    for seed in 0..200u64 {
        let mut rng = Rng(seed);
        let nslots = 6u16;
        let mut ops: Vec<MicroOp> = Vec::new();
        for _ in 0..12 {
            let dst = (rng.next() % nslots as u64) as u16;
            let lhs = (rng.next() % nslots as u64) as u16;
            let rhs = (rng.next() % nslots as u64) as u16;
            ops.push(match rng.next() % 7 {
                0 => MicroOp::Add { dst, lhs, rhs },
                1 => MicroOp::Sub { dst, lhs, rhs },
                2 => MicroOp::Mul { dst, lhs, rhs },
                3 => MicroOp::Lt { dst, lhs, rhs },
                4 => MicroOp::Eq { dst, lhs, rhs },
                5 => MicroOp::Move { dst, src: lhs },
                _ => MicroOp::LoadConst { dst, value: (rng.next() as i64) % 1000 },
            });
        }
        ops.push(MicroOp::Return { src: (seed % nslots as u64) as u16 });

        let chain = compile_straightline(&ops).expect("compile");
        let mut jit_frame: Vec<i64> = (0..nslots as i64 + 2).map(|i| i * 7 - 3).collect();
        let mut ref_frame = jit_frame.clone();
        let jit = chain.run_with_frame(&mut jit_frame);
        let reference = reference_eval(&ops, &mut ref_frame, 10_000).expect("reference");
        assert_eq!(jit, ChainOutcome::Return(reference), "seed {seed} diverged");
        assert_eq!(jit_frame, ref_frame, "seed {seed}: frame contents diverged");
    }
}

/// The piece-economics gate: EXACTLY one stencil piece per micro-op (the
/// stack model emitted four per binop). Pieces are tail-jump count — the
/// dispatch overhead the fusion exists to kill.
#[test]
fn fused_chain_code_size_reflects_single_piece_per_op() {
    let mut ops: Vec<MicroOp> = Vec::new();
    for i in 0..40u16 {
        ops.push(MicroOp::Add { dst: 2 + (i % 3), lhs: 0, rhs: 1 });
    }
    ops.push(MicroOp::Return { src: 2 });
    let chain = compile_straightline(&ops).expect("compile");
    // Each op lowers to exactly one stencil piece. Integer math is now EXACT, so
    // the Adds are checked (overflow → deopt) and share a single deopt terminal
    // across the whole chain — exactly one extra piece, not one per op.
    assert_eq!(
        chain.piece_count(),
        ops.len() + 1,
        "every micro-op lowers to one stencil piece (+ one shared deopt terminal)"
    );
    // And a checked op adds exactly the one shared deopt terminal.
    let ops2 = [
        MicroOp::Div { dst: 2, lhs: 0, rhs: 1 },
        MicroOp::Mod { dst: 3, lhs: 0, rhs: 1 },
        MicroOp::Return { src: 2 },
    ];
    let chain2 = compile_straightline(&ops2).expect("compile");
    assert_eq!(chain2.piece_count(), ops2.len() + 1, "one shared deopt terminal");
}

/// Value-producing LtEq/GtEq/NotEq lower directly (no scratch-slot pair),
/// and agree with the kernel across the signed edge grid.
#[test]
fn direct_comparison_values_match_reference() {
    const EDGES: [i64; 8] = [i64::MIN, i64::MIN + 1, -2, -1, 0, 1, 2, i64::MAX];
    for cmp in 0..3 {
        for &a in &EDGES {
            for &b in &EDGES {
                let op = match cmp {
                    0 => MicroOp::LtEq { dst: 2, lhs: 0, rhs: 1 },
                    1 => MicroOp::GtEq { dst: 2, lhs: 0, rhs: 1 },
                    _ => MicroOp::Neq { dst: 2, lhs: 0, rhs: 1 },
                };
                let ops = [op, MicroOp::Return { src: 2 }];
                let chain = compile_straightline(&ops).expect("compile");
                let mut jit_frame = [a, b, 0];
                let mut ref_frame = [a, b, 0];
                let jit = chain.run_with_frame(&mut jit_frame).expect_return();
                let reference = reference_eval(&ops, &mut ref_frame, 100).expect("ref");
                assert_eq!(jit, reference, "cmp {cmp} ({a}, {b})");
            }
        }
    }
}

/// Fused compare-branches: a hot loop whose condition scratch is dead after
/// the branch must still produce exact VM agreement (the fusion drops the
/// dead scratch write).
#[test]
fn fused_compare_branch_loops_agree_with_vm() {
    for (cond, expected) in [
        ("i is less than 4000", "7998000"),
        ("i is at most 3999", "7998000"),
        ("i is not 4000", "7998000"),
    ] {
        let src = format!(
            "## Main\n\
             Let mutable sum be 0.\n\
             Let mutable i be 0.\n\
             While {cond}:\n\
             \x20   Set sum to sum + i.\n\
             \x20   Set i to i + 1.\n\
             Show sum.\n"
        );
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "fused branch diverged for: {cond}"
        );
        assert_eq!(norm(&vm.output), expected);
        let (_, region_ok) = tier.region_counts();
        assert!(region_ok >= 1, "loop with '{cond}' must still tier up");
    }
}
