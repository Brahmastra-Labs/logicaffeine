use super::*;
use crate::codegen_sva::decision_workload::{DECIDE_FN, DECIDE_NAME};
use crate::codegen_sva::fabric_rtl::{decision_fabric_from_source, eval_decision_fabric};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn word(&mut self) -> i64 {
        const EDGES: [i64; 7] = [0, 1, -1, 2, i64::MAX, i64::MIN, i64::MIN + 1];
        match self.below(4) {
            0 => EDGES[self.below(EDGES.len() as u64) as usize],
            1 => self.below(201) as i64 - 100,
            _ => self.next() as i64,
        }
    }
}

/// `df` at datapath width `width`, the semantics the pass folds by: every op on `width`-bit words, wrapping, a compare
/// on the words read signed, a shift by the width or more emptying the word (or filling it with the sign).
fn eval_width(df: &DecisionFabric, inputs: &[u64], width: u32) -> u64 {
    let mask = if width == 64 { u64::MAX } else { (1u64 << width) - 1 };
    let sext = |v: u64| ((v << (64 - width)) as i64) >> (64 - width);
    let mut vals: Vec<u64> = Vec::with_capacity(df.cells.len());
    let get = |s: Src, vals: &[u64]| match s {
        Src::In(k) => inputs[k] & mask,
        Src::Val(k) => vals[k],
        Src::Const(c) => (c as u64) & mask,
    };
    for cell in &df.cells {
        let v = match *cell {
            LoopCell::Arith { op, a, b } => {
                let (a, b) = (get(a, &vals), get(b, &vals));
                match op {
                    AluOp::Add => a.wrapping_add(b),
                    AluOp::Sub => a.wrapping_sub(b),
                    AluOp::Mul => a.wrapping_mul(b),
                    AluOp::And => a & b,
                    AluOp::Or => a | b,
                    AluOp::Xor => a ^ b,
                    AluOp::Shl => if b >= width as u64 { 0 } else { a << b },
                    AluOp::UShr => if b >= width as u64 { 0 } else { a >> b },
                    AluOp::Shr => (sext(a) >> b.min(63)) as u64,
                }
            }
            LoopCell::Select { cmp, ca, cb, t, e } => {
                if cmp.eval(sext(get(ca, &vals)), sext(get(cb, &vals))) {
                    get(t, &vals)
                } else {
                    get(e, &vals)
                }
            }
            ref other => panic!("no width semantics for {other:?}"),
        } & mask;
        vals.push(v);
    }
    get(df.result, &vals)
}

const CMPS: [CmpKind; 6] = [CmpKind::Lt, CmpKind::Gt, CmpKind::Le, CmpKind::Ge, CmpKind::Eq, CmpKind::Ne];

/// A decision fabric of `n` cells over `n_in` inputs in which selects feed arithmetic and compares often: the shape the
/// pass exists for, and every op, compare and constant it folds.
fn random_fabric(rng: &mut Rng, n_in: usize, n: usize, width: u32) -> DecisionFabric {
    const CONSTS: [i64; 6] = [0, 1, 2, 3, -1, -2];
    let mut cells = Vec::with_capacity(n);
    for i in 0..n {
        let pick = |rng: &mut Rng| -> Src {
            match rng.below(if i == 0 { 2 } else { 5 }) {
                0 => Src::In(rng.below(n_in as u64) as usize),
                1 => Src::Const(CONSTS[rng.below(CONSTS.len() as u64) as usize]),
                _ => Src::Val(rng.below(i as u64) as usize),
            }
        };
        let cell = if rng.below(2) == 0 {
            let op = [AluOp::Add, AluOp::Sub, AluOp::Mul, AluOp::And, AluOp::Or, AluOp::Xor, AluOp::Shl, AluOp::Shr, AluOp::UShr]
                [rng.below(9) as usize];
            let shift = matches!(op, AluOp::Shl | AluOp::Shr | AluOp::UShr);
            let b = if shift { Src::Const(rng.below(width.min(8) as u64) as i64) } else { pick(rng) };
            LoopCell::Arith { op, a: pick(rng), b }
        } else {
            let cmp = CMPS[rng.below(6) as usize];
            LoopCell::Select { cmp, ca: pick(rng), cb: pick(rng), t: pick(rng), e: pick(rng) }
        };
        cells.push(cell);
    }
    DecisionFabric { n_in, cells, result: Src::Val(n - 1), escaped_first_return: false }
}

/// The carry chains along the deepest path: an add, a subtract or an order compare each one, every other cell none.
fn chain_depth(df: &DecisionFabric) -> usize {
    let mut depth: Vec<usize> = Vec::with_capacity(df.cells.len());
    let at = |s: Src, depth: &[usize]| match s {
        Src::Val(k) => depth[k],
        _ => 0,
    };
    for cell in &df.cells {
        let d = match *cell {
            LoopCell::Arith { op, a, b } => {
                at(a, &depth).max(at(b, &depth)) + usize::from(matches!(op, AluOp::Add | AluOp::Sub))
            }
            LoopCell::Select { cmp, ca, cb, t, e } => {
                let decided = at(ca, &depth).max(at(cb, &depth)) + usize::from(!matches!(cmp, CmpKind::Eq | CmpKind::Ne));
                decided.max(at(t, &depth)).max(at(e, &depth))
            }
            _ => unreachable!(),
        };
        depth.push(d);
    }
    at(df.result, &depth)
}

fn decide() -> DecisionFabric {
    decision_fabric_from_source(DECIDE_FN, DECIDE_NAME)
}

/// K1's decision, hoisted: at 64 bits it is the compiled fabric on every input tried — the extremes, random words, and
/// words a step from each threshold the decision draws, the book flag on and off — and no select feeds a sum or a
/// compare any more.
#[test]
fn the_hoisted_decision_computes_the_decision() {
    let df = decide();
    let hoisted = hoist_selects(&df, 64).unwrap();
    assert!(reads_a_select(&df), "the compiled decision chains its selects into its sums");
    assert!(!reads_a_select(&hoisted), "a select still feeds arithmetic or a compare");
    let mut rng = Rng(0x5E1E_C701);
    for _ in 0..200_000 {
        let mut x: Vec<i64> = (0..df.n_in).map(|_| rng.word()).collect();
        if rng.below(2) == 0 {
            let fair = (rng.next() % 1_000_000) as i64;
            let edge = (rng.next() % 1000) as i64;
            x[0] = fair + [edge, -edge][rng.below(2) as usize] + (rng.below(5) as i64 - 2);
            x[1] = fair;
            x[2] = edge;
            x[3] = (rng.next() % 100) as i64;
            x[4] = x[3] + (rng.below(5) as i64 - 2);
            x[6] = (rng.next() % 500) as i64;
            x[5] = [x[6], -x[6]][rng.below(2) as usize] - [x[3], -x[3], x[4], -x[4]][rng.below(4) as usize]
                + (rng.below(5) as i64 - 2);
            x[7] = rng.below(3) as i64;
        }
        assert_eq!(eval_decision_fabric(&hoisted, &x), eval_decision_fabric(&df, &x), "decide{x:?}");
    }
}

/// Hoisting takes K1's decision from six carry chains in series to two: every compare reads a sum of inputs at most, and
/// the selects resolve after them.
#[test]
fn the_hoisted_decision_holds_two_carry_chains_in_series() {
    let df = decide();
    assert_eq!(chain_depth(&df), 6, "the compiled decision chains its compares through its sums");
    assert_eq!(chain_depth(&hoist_selects(&df, 64).unwrap()), 2);
}

/// The tree's root tests a predicate that arrives last: a compare of a sum, not a compare of two inputs.
#[test]
fn the_root_tests_the_latest_predicate() {
    let hoisted = hoist_selects(&decide(), 64).unwrap();
    let Src::Val(root) = hoisted.result else { panic!("the decision's value is a cell") };
    let LoopCell::Select { ca, cb, .. } = hoisted.cells[root] else { panic!("the root is a select") };
    let is_sum = |s: Src| {
        matches!(s, Src::Val(k) if matches!(hoisted.cells[k], LoopCell::Arith { op: AluOp::Add | AluOp::Sub, .. }))
    };
    assert!(is_sum(ca) || is_sum(cb), "the root compares two operands that arrive first: {:?}", hoisted.cells[root]);
}

/// On fabrics of every shape the pass is exact: at 3 and 4 bits on every input, at 64 bits on random ones; and it
/// leaves no select feeding arithmetic.
#[test]
fn every_shape_hoists_exactly_at_every_input_of_a_narrow_width() {
    let mut rng = Rng(0x5E1E_C702);
    for case in 0..400 {
        let n = 3 + rng.below(10) as usize;
        for width in [3u32, 4] {
            let df = random_fabric(&mut rng, 3, n, width);
            let hoisted = hoist_selects(&df, width).unwrap_or_else(|e| panic!("case {case}: {e}"));
            assert!(!reads_a_select(&hoisted), "case {case}: a select still feeds arithmetic: {hoisted:?}");
            for word in 0..1u64 << (3 * width) {
                let m = (1u64 << width) - 1;
                let x = [word & m, (word >> width) & m, (word >> (2 * width)) & m];
                assert_eq!(
                    eval_width(&hoisted, &x, width),
                    eval_width(&df, &x, width),
                    "case {case} at {width} bits, inputs {x:?}\n{df:?}\n{hoisted:?}"
                );
            }
        }
        let df = random_fabric(&mut rng, 3, n, 64);
        let hoisted = hoist_selects(&df, 64).unwrap();
        for _ in 0..200 {
            let x: Vec<i64> = (0..3).map(|_| rng.word()).collect();
            assert_eq!(eval_decision_fabric(&hoisted, &x), eval_decision_fabric(&df, &x), "case {case}: {x:?}");
        }
    }
}

/// The width-faithful evaluator is the fabric's own at 64 bits, so a narrow check checks the pass that runs on silicon.
#[test]
fn the_width_evaluator_is_the_fabric_at_64_bits() {
    let mut rng = Rng(0x5E1E_C703);
    for _ in 0..200 {
        let df = random_fabric(&mut rng, 3, 10, 64);
        for _ in 0..50 {
            let x: Vec<i64> = (0..3).map(|_| rng.word()).collect();
            let wide: Vec<u64> = x.iter().map(|&v| v as u64).collect();
            assert_eq!(eval_width(&df, &wide, 64) as i64, eval_decision_fabric(&df, &x));
        }
    }
}

/// A compare its operands decide folds as the width reads them: 3 is less than 4 at 64 bits and greater at 3 bits,
/// where 4 is −4.
#[test]
fn a_constant_compare_folds_at_its_width() {
    let df = DecisionFabric {
        n_in: 1,
        cells: vec![LoopCell::Select { cmp: CmpKind::Lt, ca: Src::Const(3), cb: Src::Const(4), t: Src::In(0), e: Src::Const(0) }],
        result: Src::Val(0),
        escaped_first_return: false,
    };
    assert_eq!(eval_decision_fabric(&hoist_selects(&df, 64).unwrap(), &[7]), 7);
    assert_eq!(eval_width(&hoist_selects(&df, 3).unwrap(), &[7], 3), 0);
}

/// A shift never folds: its count means different things at different widths. At 8 bits a count of 64 is past the word
/// and fills it with the sign, where a 64-bit evaluation would read the count mod 64 and not shift at all.
#[test]
fn a_constant_shift_does_not_fold() {
    for (op, a, want) in [(AluOp::Shr, 100, 0u64), (AluOp::Shr, -100, 0xff), (AluOp::Shl, 3, 0), (AluOp::UShr, -1, 0)] {
        let df = DecisionFabric {
            n_in: 1,
            cells: vec![LoopCell::Arith { op, a: Src::Const(a), b: Src::Const(64) }],
            result: Src::Val(0),
            escaped_first_return: false,
        };
        assert_eq!(eval_width(&df, &[0], 8), want, "{op:?} {a} by 64 at 8 bits");
        assert_eq!(eval_width(&hoist_selects(&df, 8).unwrap(), &[0], 8), want, "{op:?} {a} by 64, hoisted at 8 bits");
    }
}

/// Neither `a < b` nor `b < a` is `a = b`: on the path where both orders fail, an equality select takes its first arm.
#[test]
fn both_orders_failing_is_equality() {
    let (a, b) = (Src::In(0), Src::In(1));
    let cells = vec![
        LoopCell::Select { cmp: CmpKind::Eq, ca: a, cb: b, t: Src::Const(3), e: Src::Const(4) },
        LoopCell::Select { cmp: CmpKind::Lt, ca: b, cb: a, t: Src::Const(2), e: Src::Val(0) },
        LoopCell::Select { cmp: CmpKind::Lt, ca: a, cb: b, t: Src::Const(1), e: Src::Val(1) },
        LoopCell::Arith { op: AluOp::Add, a: Src::Val(2), b: Src::In(2) },
    ];
    let df = DecisionFabric { n_in: 3, cells, result: Src::Val(3), escaped_first_return: false };
    let hoisted = hoist_selects(&df, 4).unwrap();
    for word in 0..1u64 << 12 {
        let x = [word & 15, (word >> 4) & 15, (word >> 8) & 15];
        assert_eq!(eval_width(&hoisted, &x, 4), eval_width(&df, &x, 4), "{x:?}");
    }
}

/// A negation an arm brings folds into the sum that reads it — `x + (0 − y)`, `(0 − y) + x`, `x − (0 − y)` and
/// `0 − (0 − y)` — and each fold is exact on every pair of 4-bit words.
#[test]
fn a_negation_folds_into_its_sum_exactly() {
    let neg = LoopCell::Arith { op: AluOp::Sub, a: Src::Const(0), b: Src::In(1) };
    for (op, a, b) in [
        (AluOp::Add, Src::In(0), Src::Val(0)),
        (AluOp::Add, Src::Val(0), Src::In(0)),
        (AluOp::Sub, Src::In(0), Src::Val(0)),
        (AluOp::Sub, Src::Const(0), Src::Val(0)),
    ] {
        let df = DecisionFabric {
            n_in: 2,
            cells: vec![neg.clone(), LoopCell::Arith { op, a, b }],
            result: Src::Val(1),
            escaped_first_return: false,
        };
        let hoisted = hoist_selects(&df, 4).unwrap();
        assert!(hoisted.cells.len() <= 1, "{op:?} {a:?} {b:?}: the negation stays a cell of its own: {hoisted:?}");
        for word in 0..1u64 << 8 {
            let x = [word & 15, word >> 4];
            assert_eq!(eval_width(&hoisted, &x, 4), eval_width(&df, &x, 4), "{op:?} {a:?} {b:?} at {x:?}");
        }
    }
}

/// The same fabric hoists to the same fabric, cell for cell.
#[test]
fn hoisting_is_deterministic() {
    assert_eq!(format!("{:?}", hoist_selects(&decide(), 64).unwrap()), format!("{:?}", hoist_selects(&decide(), 64).unwrap()));
}

/// A tile with no combinational form — a heap read — is refused by name.
#[test]
fn a_tile_with_no_combinational_form_is_refused() {
    let df = DecisionFabric {
        n_in: 1,
        cells: vec![LoopCell::HeapRead { addr: Src::In(0) }],
        result: Src::Val(0),
        escaped_first_return: false,
    };
    let e = hoist_selects(&df, 64).unwrap_err();
    assert!(e.contains("cell 0"), "{e}");
}
