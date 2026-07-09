//! Execute differentials: chains of REAL extracted stencils, patched and run,
//! against plain-Rust reference computations. This is copy-and-patch end to
//! end — `rustc --emit=obj` → extraction → layout → relocation patching →
//! native execution.

use logicaffeine_forge::buffer::{HoleValue, JitBuffer};
use logicaffeine_forge::{ST_ADDI, ST_BRANCH_IF, ST_CONST, ST_LTI, ST_MULI, ST_RETURN, ST_SUBI};

/// Build a straight-line chain: each op continues to the next piece, ending
/// in `return`.
fn linear_chain(ops: &[(&'static logicaffeine_forge::Stencil, Option<i64>)]) -> i64 {
    let mut buf = JitBuffer::new();
    for (k, (stencil, konst)) in ops.iter().enumerate() {
        let next = buf.label(k + 1);
        let mut holes = vec![HoleValue::Cont(0, next)];
        if let Some(v) = konst {
            holes.push(HoleValue::Const(0, *v));
        }
        buf.push_stencil(stencil, &holes);
    }
    buf.push_stencil(&ST_RETURN, &[]);
    buf.finish().expect("finish").run()
}

#[test]
fn jit_const_const_add_return_is_42() {
    let r = linear_chain(&[(&ST_CONST, Some(7)), (&ST_CONST, Some(35)), (&ST_ADDI, None)]);
    assert_eq!(r, 42);
}

#[test]
fn jit_sub_mul_wrap_at_boundaries() {
    assert_eq!(
        linear_chain(&[(&ST_CONST, Some(5)), (&ST_CONST, Some(9)), (&ST_SUBI, None)]),
        -4
    );
    assert_eq!(
        linear_chain(&[(&ST_CONST, Some(i64::MAX)), (&ST_CONST, Some(2)), (&ST_MULI, None)]),
        -2
    );
    assert_eq!(
        linear_chain(&[(&ST_CONST, Some(i64::MAX)), (&ST_CONST, Some(1)), (&ST_ADDI, None)]),
        i64::MIN
    );
}

#[test]
fn jit_lt_yields_one_and_zero() {
    assert_eq!(
        linear_chain(&[(&ST_CONST, Some(3)), (&ST_CONST, Some(5)), (&ST_LTI, None)]),
        1
    );
    assert_eq!(
        linear_chain(&[(&ST_CONST, Some(5)), (&ST_CONST, Some(3)), (&ST_LTI, None)]),
        0
    );
}

#[test]
fn jit_branch_if_routes_both_ways() {
    // Layout: [0]=const c, [1]=branch_if → (then=2, else=4),
    //         [2]=const 111, [3]=return, [4]=const 222, [5]=return.
    let build = |c: i64| -> i64 {
        let mut buf = JitBuffer::new();
        let then_l = buf.label(2);
        let else_l = buf.label(4);
        let l1 = buf.label(1);
        let l3 = buf.label(3);
        let l5 = buf.label(5);
        buf.push_stencil(&ST_CONST, &[HoleValue::Const(0, c), HoleValue::Cont(0, l1)]);
        buf.push_stencil(
            &ST_BRANCH_IF,
            &[HoleValue::Cont(0, then_l), HoleValue::Cont(1, else_l)],
        );
        buf.push_stencil(&ST_CONST, &[HoleValue::Const(0, 111), HoleValue::Cont(0, l3)]);
        buf.push_stencil(&ST_RETURN, &[]);
        buf.push_stencil(&ST_CONST, &[HoleValue::Const(0, 222), HoleValue::Cont(0, l5)]);
        buf.push_stencil(&ST_RETURN, &[]);
        buf.finish().expect("finish").run()
    };
    assert_eq!(build(1), 111);
    assert_eq!(build(0), 222);
    assert_eq!(build(-7), 111);
}

#[test]
fn jit_missing_hole_and_bad_label_are_errors() {
    let mut buf = JitBuffer::new();
    buf.push_stencil(&ST_CONST, &[HoleValue::Const(0, 1)]);
    buf.push_stencil(&ST_RETURN, &[]);
    assert!(buf.finish().is_err());

    let mut buf = JitBuffer::new();
    let bogus = buf.label(99);
    buf.push_stencil(&ST_CONST, &[HoleValue::Const(0, 1), HoleValue::Cont(0, bogus)]);
    assert!(buf.finish().is_err());

    assert!(JitBuffer::new().finish().is_err());
}

#[derive(Clone, Copy)]
enum RefOp {
    Const(i64),
    Add,
    Sub,
    Mul,
    Lt,
}

/// The independent model for the seeded differential.
fn reference_eval(ops: &[RefOp]) -> i64 {
    let mut stack: Vec<i64> = Vec::new();
    for op in ops {
        match op {
            RefOp::Const(v) => stack.push(*v),
            RefOp::Add => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(a.wrapping_add(b));
            }
            RefOp::Sub => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(a.wrapping_sub(b));
            }
            RefOp::Mul => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(a.wrapping_mul(b));
            }
            RefOp::Lt => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push((a < b) as i64);
            }
        }
    }
    *stack.last().unwrap()
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
fn jit_ten_thousand_seeded_chains_match_reference() {
    for seed in 0..10_000u64 {
        let mut rng = SplitMix64::new(seed);
        // Random RPN chain: depth ≥ 2 before each binop.
        let mut ops: Vec<RefOp> = Vec::new();
        let mut depth = 0usize;
        let len = 3 + rng.below(12);
        for _ in 0..len {
            if depth < 2 || rng.below(3) == 0 {
                let v = match rng.below(5) {
                    0 => i64::MAX,
                    1 => i64::MIN,
                    2 => -1,
                    _ => rng.below(1000) as i64 - 500,
                };
                ops.push(RefOp::Const(v));
                depth += 1;
            } else {
                ops.push(match rng.below(4) {
                    0 => RefOp::Add,
                    1 => RefOp::Sub,
                    2 => RefOp::Mul,
                    _ => RefOp::Lt,
                });
                depth -= 1;
            }
        }
        if depth == 0 {
            ops.push(RefOp::Const(7));
        }

        let chain: Vec<(&'static logicaffeine_forge::Stencil, Option<i64>)> = ops
            .iter()
            .map(|op| match op {
                RefOp::Const(v) => (&ST_CONST, Some(*v)),
                RefOp::Add => (&ST_ADDI, None),
                RefOp::Sub => (&ST_SUBI, None),
                RefOp::Mul => (&ST_MULI, None),
                RefOp::Lt => (&ST_LTI, None),
            })
            .collect();

        let jit = linear_chain(&chain);
        let reference = reference_eval(&ops);
        assert_eq!(jit, reference, "seed {seed}: chain diverged from reference");
    }
}
