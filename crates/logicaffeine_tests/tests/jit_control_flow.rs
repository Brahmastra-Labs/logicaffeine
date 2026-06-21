//! J2 differentials: jumps, branches, back-edges — real loops in native code.

use logicaffeine_forge::jit::{compile_straightline, reference_eval, JitCompileError, MicroOp};

fn run_both(ops: &[MicroOp], inputs: &[i64]) -> (i64, i64, Vec<i64>, Vec<i64>) {
    let chain = compile_straightline(ops).expect("compile");
    let mut jit_frame = vec![0i64; 64];
    jit_frame[..inputs.len()].copy_from_slice(inputs);
    let jit = chain.run_with_frame(&mut jit_frame).expect_return();

    let mut ref_frame = vec![0i64; 64];
    ref_frame[..inputs.len()].copy_from_slice(inputs);
    let reference = reference_eval(ops, &mut ref_frame, 10_000_000).expect("reference ran out of fuel");
    (jit, reference, jit_frame, ref_frame)
}

#[test]
fn jit_forward_jump_skips_code() {
    let ops = [
        MicroOp::LoadConst { dst: 0, value: 1 },
        MicroOp::Jump { target: 3 },
        MicroOp::LoadConst { dst: 0, value: 99 }, // skipped
        MicroOp::Return { src: 0 },
    ];
    let (jit, reference, _, _) = run_both(&ops, &[]);
    assert_eq!(jit, reference);
    assert_eq!(jit, 1);
}

#[test]
fn jit_branch_takes_both_arms() {
    // if (slot0 != 0) slot1 = 10 else slot1 = 20; return slot1
    let ops = [
        MicroOp::JumpIfFalse { cond: 0, target: 3 },
        MicroOp::LoadConst { dst: 1, value: 10 },
        MicroOp::Jump { target: 4 },
        MicroOp::LoadConst { dst: 1, value: 20 },
        MicroOp::Return { src: 1 },
    ];
    for c in [0i64, 1, -7] {
        let (jit, reference, _, _) = run_both(&ops, &[c]);
        assert_eq!(jit, reference, "cond {c}");
        assert_eq!(jit, if c != 0 { 10 } else { 20 });
    }
}

/// factorial(n) as a real back-edge loop:
///   acc = 1; while (n > 1) { acc *= n; n -= 1 } return acc
fn factorial_ops() -> Vec<MicroOp> {
    vec![
        MicroOp::LoadConst { dst: 1, value: 1 },              // 0: acc = 1
        MicroOp::LoadConst { dst: 2, value: 1 },              // 1: one = 1
        MicroOp::Gt { dst: 3, lhs: 0, rhs: 2 },               // 2: t = n > 1   <- loop head
        MicroOp::JumpIfFalse { cond: 3, target: 7 },          // 3: exit when done
        MicroOp::Mul { dst: 1, lhs: 1, rhs: 0 },              // 4: acc *= n
        MicroOp::Sub { dst: 0, lhs: 0, rhs: 2 },              // 5: n -= 1
        MicroOp::Jump { target: 2 },                          // 6: back-edge
        MicroOp::Return { src: 1 },                           // 7
    ]
}

#[test]
fn jit_factorial_loop_with_back_edge() {
    let ops = factorial_ops();
    let expect = [1i64, 1, 2, 6, 24, 120, 720, 5040, 40320, 362880, 3628800];
    for (n, want) in expect.iter().enumerate() {
        let (jit, reference, jf, rf) = run_both(&ops, &[n as i64]);
        assert_eq!(jit, reference, "factorial({n})");
        assert_eq!(jit, *want, "factorial({n})");
        assert_eq!(jf, rf, "factorial({n}) frame");
    }
    // 20! is the largest fitting i64; 21! wraps — wrapping is the spec.
    let (jit, reference, _, _) = run_both(&ops, &[21]);
    assert_eq!(jit, reference);
}

#[test]
fn jit_loop_iteration_count_is_exact() {
    // count = 0; i = n; while (i > 0) { count += 1; i -= 1 }  → count == n
    let ops = vec![
        MicroOp::LoadConst { dst: 1, value: 0 },              // count
        MicroOp::LoadConst { dst: 2, value: 1 },              // one
        MicroOp::LoadConst { dst: 3, value: 0 },               // zero
        MicroOp::Gt { dst: 4, lhs: 0, rhs: 3 },               // 3: i > 0   <- head
        MicroOp::JumpIfFalse { cond: 4, target: 8 },
        MicroOp::Add { dst: 1, lhs: 1, rhs: 2 },
        MicroOp::Sub { dst: 0, lhs: 0, rhs: 2 },
        MicroOp::Jump { target: 3 },
        MicroOp::Return { src: 1 },                           // 8
    ];
    for n in [0i64, 1, 2, 100, 10_000] {
        let (jit, reference, _, _) = run_both(&ops, &[n]);
        assert_eq!(jit, reference, "n={n}");
        assert_eq!(jit, n, "n={n}");
    }
}

#[test]
fn jit_structural_validation_for_jumps() {
    assert_eq!(
        compile_straightline(&[MicroOp::Jump { target: 5 }]).unwrap_err(),
        JitCompileError::BadJumpTarget { op_index: 0, target: 5 }
    );
    assert_eq!(
        compile_straightline(&[MicroOp::LoadConst { dst: 0, value: 1 }]).unwrap_err(),
        JitCompileError::FallsOffTheEnd
    );
    // A trailing Jump (e.g. an infinite-loop tail) is structurally fine.
    assert!(compile_straightline(&[MicroOp::Jump { target: 0 }]).is_ok());
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

/// Structured generation: random straight-line bodies inside COUNTED loops
/// (a dedicated counter slot guarantees termination), nested up to depth 2.
#[test]
fn jit_two_thousand_seeded_loop_programs_match_reference() {
    run_loop_fuzz(0..2_000);
}

/// The long campaign: `cargo test -p logicaffeine-forge jit_mega_fuzz -- --ignored`.
/// Bigger bodies + higher seeds drive multi-page pools and dense CFGs.
#[test]
#[ignore]
fn jit_mega_fuzz() {
    run_loop_fuzz(0..2_000_000);
}

fn run_loop_fuzz(seeds: std::ops::Range<u64>) {
    const BODY_SLOTS: u64 = 8; // slots 0..8 for data; 8/9 reserved for counters
    for seed in seeds {
        let mut rng = SplitMix64::new(seed);
        let mut ops: Vec<MicroOp> = Vec::new();

        // init data slots
        for k in 0..BODY_SLOTS {
            ops.push(MicroOp::LoadConst { dst: k as u16, value: rng.below(50) as i64 });
        }
        let gen_line = |rng: &mut SplitMix64, ops: &mut Vec<MicroOp>| {
            let dst = rng.below(BODY_SLOTS) as u16;
            let lhs = rng.below(BODY_SLOTS) as u16;
            let rhs = rng.below(BODY_SLOTS) as u16;
            ops.push(match rng.below(6) {
                0 => MicroOp::Add { dst, lhs, rhs },
                1 => MicroOp::Sub { dst, lhs, rhs },
                2 => MicroOp::Mul { dst, lhs, rhs },
                3 => MicroOp::Lt { dst, lhs, rhs },
                4 => MicroOp::Move { dst, src: lhs },
                _ => MicroOp::LoadConst { dst, value: rng.below(100) as i64 },
            });
        };

        // counted loop: slot 8 = counter, slot 9 = zero
        let trip = rng.below(6) as i64; // 0..5 iterations (0 = never entered)
        ops.push(MicroOp::LoadConst { dst: 8, value: trip });
        ops.push(MicroOp::LoadConst { dst: 9, value: 0 });
        let head = ops.len();
        ops.push(MicroOp::Gt { dst: 10, lhs: 8, rhs: 9 });
        let branch_at = ops.len();
        ops.push(MicroOp::JumpIfFalse { cond: 10, target: usize::MAX }); // patched
        // High seeds grow bodies enough to push code+pool past 4/8/16 KiB.
        let body_len = 1 + rng.below(6) + if seed > 10_000 { rng.below(60) } else { 0 };
        for _ in 0..body_len {
            gen_line(&mut rng, &mut ops);
        }
        ops.push(MicroOp::LoadConst { dst: 11, value: 1 });
        ops.push(MicroOp::Sub { dst: 8, lhs: 8, rhs: 11 });
        ops.push(MicroOp::Jump { target: head });
        let after = ops.len();
        ops[branch_at] = MicroOp::JumpIfFalse { cond: 10, target: after };

        ops.push(MicroOp::Return { src: rng.below(BODY_SLOTS) as u16 });

        let chain = compile_straightline(&ops).expect("compile");
        let mut jit_frame = vec![0i64; 64];
        let jit = chain.run_with_frame(&mut jit_frame).expect_return();

        let mut ref_frame = vec![0i64; 64];
        let reference =
            reference_eval(&ops, &mut ref_frame, 10_000_000).expect("reference fuel");

        assert_eq!(jit, reference, "seed {seed}: result diverged");
        assert_eq!(jit_frame, ref_frame, "seed {seed}: frame diverged");
    }
}
