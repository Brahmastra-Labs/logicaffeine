//! J1 differentials: the micro-op compiler vs the reference evaluator,
//! and the frame-slot machinery in isolation.

use logicaffeine_forge::jit::{compile_straightline, reference_eval, JitCompileError, MicroOp};

fn run_both(ops: &[MicroOp], inputs: &[i64]) -> (i64, i64) {
    let chain = compile_straightline(ops).expect("compile");
    let mut jit_frame = vec![0i64; 64];
    jit_frame[..inputs.len()].copy_from_slice(inputs);
    let jit = chain.run_with_frame(&mut jit_frame).expect_return();

    let mut ref_frame = vec![0i64; 64];
    ref_frame[..inputs.len()].copy_from_slice(inputs);
    let reference = reference_eval(ops, &mut ref_frame, 1_000_000).expect("fuel");
    (jit, reference)
}

#[test]
fn jit_add_two_arguments() {
    // The J1 headline: add(a, b) compiled from patched stencils.
    let ops = [
        MicroOp::Add { dst: 2, lhs: 0, rhs: 1 },
        MicroOp::Return { src: 2 },
    ];
    let chain = compile_straightline(&ops).expect("compile");
    let mut frame = vec![0i64; 8];
    frame[0] = 3;
    frame[1] = 5;
    assert_eq!(chain.run_with_frame(&mut frame).expect_return(), 8);
    frame[0] = i64::MAX;
    frame[1] = 1;
    assert_eq!(chain.run_with_frame(&mut frame).expect_return(), i64::MIN);
}

#[test]
fn jit_slot_set_writes_are_visible_in_the_frame() {
    // The frame is shared state: the caller sees writes after the run.
    let ops = [
        MicroOp::LoadConst { dst: 5, value: 42 },
        MicroOp::Move { dst: 6, src: 5 },
        MicroOp::Return { src: 6 },
    ];
    let chain = compile_straightline(&ops).expect("compile");
    let mut frame = vec![0i64; 8];
    assert_eq!(chain.run_with_frame(&mut frame).expect_return(), 42);
    assert_eq!(frame[5], 42);
    assert_eq!(frame[6], 42);
}

#[test]
fn jit_same_register_as_src_and_dst() {
    // dst == lhs == rhs — the in-place patterns a register VM emits constantly.
    let ops = [
        MicroOp::Add { dst: 0, lhs: 0, rhs: 0 },
        MicroOp::Add { dst: 0, lhs: 0, rhs: 1 },
        MicroOp::Return { src: 0 },
    ];
    let (jit, reference) = run_both(&ops, &[7, 100]);
    assert_eq!(jit, reference);
    assert_eq!(jit, 114);
}

#[test]
fn jit_comparisons_and_gt_swap() {
    for (a, b) in [(1i64, 2i64), (2, 1), (2, 2), (i64::MIN, i64::MAX)] {
        for op in [
            MicroOp::Lt { dst: 2, lhs: 0, rhs: 1 },
            MicroOp::Gt { dst: 2, lhs: 0, rhs: 1 },
            MicroOp::Eq { dst: 2, lhs: 0, rhs: 1 },
        ] {
            let ops = [op, MicroOp::Return { src: 2 }];
            let (jit, reference) = run_both(&ops, &[a, b]);
            assert_eq!(jit, reference, "{op:?} on ({a}, {b})");
        }
    }
}

#[test]
fn jit_structural_validation() {
    assert_eq!(compile_straightline(&[]).unwrap_err(), JitCompileError::Empty);
    // Execution must not run off the end.
    assert_eq!(
        compile_straightline(&[MicroOp::LoadConst { dst: 0, value: 1 }]).unwrap_err(),
        JitCompileError::FallsOffTheEnd
    );
    assert_eq!(
        compile_straightline(&[
            MicroOp::Return { src: 0 },
            MicroOp::LoadConst { dst: 0, value: 1 },
        ])
        .unwrap_err(),
        JitCompileError::FallsOffTheEnd
    );
    // An early Return mid-program is legal (J2: multiple exits).
    let early = [
        MicroOp::Return { src: 0 },
        MicroOp::Return { src: 1 },
    ];
    assert!(compile_straightline(&early).is_ok());
}

struct SplitMix64 {
    state: u64,
}
impl SplitMix64 {
    fn new(seed: u64) -> Self {
        SplitMix64 { state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15) }
    }
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

#[test]
fn jit_ten_thousand_seeded_programs_match_reference() {
    const SLOTS: u64 = 16;
    for seed in 0..10_000u64 {
        let mut rng = SplitMix64::new(seed);
        let mut ops: Vec<MicroOp> = Vec::new();
        let len = 1 + rng.below(24);
        for _ in 0..len {
            let dst = rng.below(SLOTS) as u16;
            let lhs = rng.below(SLOTS) as u16;
            let rhs = rng.below(SLOTS) as u16;
            ops.push(match rng.below(8) {
                0 => MicroOp::LoadConst {
                    dst,
                    value: match rng.below(4) {
                        0 => i64::MAX,
                        1 => i64::MIN,
                        _ => rng.below(2000) as i64 - 1000,
                    },
                },
                1 => MicroOp::Move { dst, src: lhs },
                2 => MicroOp::Add { dst, lhs, rhs },
                3 => MicroOp::Sub { dst, lhs, rhs },
                4 => MicroOp::Mul { dst, lhs, rhs },
                5 => MicroOp::Lt { dst, lhs, rhs },
                6 => MicroOp::Gt { dst, lhs, rhs },
                _ => MicroOp::Eq { dst, lhs, rhs },
            });
        }
        ops.push(MicroOp::Return { src: rng.below(SLOTS) as u16 });

        // Random initial frame contents, identical for both engines.
        let inputs: Vec<i64> = (0..SLOTS).map(|_| rng.next_u64() as i64).collect();
        let chain = compile_straightline(&ops).expect("compile");
        let mut jit_frame = vec![0i64; 64];
        jit_frame[..inputs.len()].copy_from_slice(&inputs);
        let jit = chain.run_with_frame(&mut jit_frame).expect_return();

        let mut ref_frame = vec![0i64; 64];
        ref_frame[..inputs.len()].copy_from_slice(&inputs);
        let reference = reference_eval(&ops, &mut ref_frame, 1_000_000).expect("fuel");

        assert_eq!(jit, reference, "seed {seed}: result diverged");
        // The FULL frame must match too — every register write, not just the
        // returned value.
        assert_eq!(jit_frame, ref_frame, "seed {seed}: frame diverged");
    }
}
