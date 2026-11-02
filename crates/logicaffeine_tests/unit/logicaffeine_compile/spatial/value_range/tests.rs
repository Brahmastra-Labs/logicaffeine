//! Each interval rule against the operation it bounds, exhaustively over small intervals: the rule's interval is the
//! hull of the operation over every point of its operands' intervals — no result outside it, both of its ends reached —
//! where the rule is exact, and holds every result where it is only sound. The reference is the operation itself,
//! computed apart from the rule: floor division through `f64`, sums and folds by enumerating every sequence.

use super::{body_nodes, floor_div, fold, join, relu, requant, signed_bits, sums, wrapped, Interval, UNBOUNDED};
use crate::spatial::ir::{LoopVar, Pred, PredRhs, ReducePool, TensorBody};

/// Every interval with both ends in `lo..=hi`.
fn intervals(lo: i64, hi: i64) -> Vec<Interval> {
    (lo..=hi).flat_map(|a| (a..=hi).map(move |b| (a, b))).collect()
}

/// The least and greatest of `vals`; `None` for none.
fn hull(vals: impl IntoIterator<Item = i64>) -> Option<Interval> {
    vals.into_iter().fold(None, |h: Option<Interval>, v| Some(h.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v)))))
}

fn points(iv: Interval) -> impl Iterator<Item = i64> + Clone {
    iv.0..=iv.1
}

/// `⌊x / d⌋` through `f64`, exact at these magnitudes.
fn floor_by_float(x: i64, d: i64) -> i64 {
    (x as f64 / d as f64).floor() as i64
}

#[test]
fn floor_division_holds_exactly_every_quotient_by_a_nonzero_divisor() {
    for a in intervals(-9, 9) {
        for b in intervals(-4, 4) {
            let quotients = points(a).flat_map(|x| points(b).filter(|d| *d != 0).map(move |d| floor_by_float(x, d)));
            assert_eq!(floor_div(a, b), hull(quotients).unwrap_or((0, 0)), "⌊{a:?} / {b:?}⌋");
        }
    }
}

#[test]
fn floor_division_at_the_ends_of_i64_stays_exact() {
    assert_eq!(floor_div((i64::MIN, i64::MIN), (-1, -1)), UNBOUNDED, "i64::MIN / -1 leaves i64: bounded by nothing");
    assert_eq!(floor_div((i64::MIN, i64::MAX), (1, 1)), UNBOUNDED);
    assert_eq!(floor_div((-7, -7), (i64::MAX, i64::MAX)), (-1, -1));
    assert_eq!(floor_div((7, 7), (i64::MIN, i64::MIN)), (-1, -1));
}

fn operand(i: usize) -> Box<TensorBody> {
    Box::new(TensorBody::Operand(i))
}

/// The interval `body_nodes` gives `b` over operands in `x` and `y`, and every node's.
fn rule(b: &TensorBody, x: Interval, y: Interval) -> (Interval, Vec<Interval>) {
    let mut nodes = Vec::new();
    let iv = body_nodes(b, &|i| if i == 0 { x } else { y }, &mut nodes);
    (iv, nodes)
}

#[test]
fn each_binary_node_holds_exactly_its_operation_over_its_operands() {
    type Op = fn(i64, i64) -> Option<i64>;
    let ops: [(&str, TensorBody, Op); 6] = [
        ("+", TensorBody::Add(operand(0), operand(1)), |a, b| Some(a + b)),
        ("-", TensorBody::Sub(operand(0), operand(1)), |a, b| Some(a - b)),
        ("·", TensorBody::Mul(operand(0), operand(1)), |a, b| Some(a * b)),
        ("min", TensorBody::Min(operand(0), operand(1)), |a, b| Some(a.min(b))),
        ("max", TensorBody::Max(operand(0), operand(1)), |a, b| Some(a.max(b))),
        ("⌊/⌋", TensorBody::FloorDiv(operand(0), operand(1)), |a, b| (b != 0).then(|| floor_by_float(a, b))),
    ];
    let boxes = intervals(-5, 5);
    for (name, body, op) in &ops {
        for &x in &boxes {
            for &y in &boxes {
                let want = hull(points(x).flat_map(|a| points(y).filter_map(move |b| op(a, b)))).unwrap_or((0, 0));
                let (got, nodes) = rule(body, x, y);
                assert_eq!(got, want, "{x:?} {name} {y:?}");
                assert_eq!(nodes, vec![x, y, got], "{x:?} {name} {y:?}: each node in post order, the operands first");
            }
        }
    }
}

#[test]
fn a_shift_holds_exactly_its_arithmetic_shift() {
    for x in intervals(-20, 20) {
        for by in 0..5u32 {
            let body = TensorBody::Shr { arg: operand(0), by };
            assert_eq!(rule(&body, x, (0, 0)).0, hull(points(x).map(|v| v >> by)).unwrap(), "{x:?} >> {by}");
        }
    }
}

#[test]
fn a_select_holds_both_of_its_arms() {
    for x in intervals(-4, 4) {
        for y in intervals(-4, 4) {
            let body = TensorBody::Select { cond: Pred::AxisLt(LoopVar(0), PredRhs::Const(1)), then: operand(0), otherwise: operand(1) };
            assert_eq!(rule(&body, x, y).0, join(x, y), "either arm, {x:?} or {y:?}");
        }
    }
}

/// Every sequence of `n` values from `iv`.
fn sequences(iv: Interval, n: usize) -> Vec<Vec<i64>> {
    (0..n).fold(vec![Vec::new()], |seqs, _| seqs.into_iter().flat_map(|s| points(iv).map(move |v| [s.clone(), vec![v]].concat())).collect())
}

#[test]
fn a_sum_holds_exactly_its_total_and_every_partial_sum_on_the_way() {
    for iv in intervals(-3, 4) {
        for n in 1..=4 {
            let seqs = sequences(iv, n);
            let total = hull(seqs.iter().map(|s| s.iter().sum::<i64>())).unwrap();
            let partial = hull(seqs.iter().flat_map(|s| (1..=n).map(|j| s[..j].iter().sum::<i64>()))).unwrap();
            assert_eq!(sums(iv, n), (total, partial), "{n} values in {iv:?}: the total, then every partial sum");
            assert_eq!(fold(ReducePool::Sum, iv, n), (total, partial));
        }
    }
}

#[test]
fn a_max_fold_holds_exactly_its_values_and_an_argmax_every_index() {
    for iv in intervals(-3, 3) {
        for n in 1..=4 {
            let seqs = sequences(iv, n);
            let max = hull(seqs.iter().map(|s| *s.iter().max().unwrap())).unwrap();
            assert_eq!(fold(ReducePool::Max, iv, n), (max, iv), "the max of {n} values in {iv:?}, its register every one");
            // The first strictly greatest — every index a run can name lies in the rule's; a point interval only ever
            // names the first.
            let argmax = hull(seqs.iter().map(|s| s.iter().enumerate().fold(0, |b, (i, v)| if *v > s[b] { i } else { b }) as i64)).unwrap();
            let (index, held) = fold(ReducePool::ArgMax, iv, n);
            assert!(index.0 <= argmax.0 && argmax.1 <= index.1, "argmax of {n} values in {iv:?}: {argmax:?} within {index:?}");
            assert_eq!(index, (0, n as i64 - 1));
            assert_eq!(held, iv);
        }
    }
}

#[test]
fn a_requant_holds_exactly_its_product_and_its_shifted_product() {
    for x in intervals(-12, 12) {
        for m in -7..=7i64 {
            for s in 0..4u32 {
                let prod = hull(points(x).map(|v| v * m)).unwrap();
                let out = hull(points(x).map(|v| (v * m) >> s)).unwrap();
                assert_eq!(requant(x, m, s), (prod, out), "({x:?} · {m}) >> {s}");
            }
        }
    }
    let (prod, out) = requant((i64::MIN, i64::MAX), 3, 2);
    assert_eq!(prod, UNBOUNDED, "a product past i64 is bounded by nothing");
    assert_eq!(out, (i64::MIN >> 2, i64::MAX >> 2), "and its shift by what an i64 shifted holds");
}

#[test]
fn a_relu_holds_exactly_its_values() {
    for x in intervals(-6, 6) {
        assert_eq!(relu(x), hull(points(x).map(|v| v.max(0))).unwrap(), "relu over {x:?}");
    }
}

/// `v` stored into `bits` bits of two's complement and read back.
fn stored(v: i64, bits: u32) -> i64 {
    (v << (64 - bits)) >> (64 - bits)
}

#[test]
fn a_store_into_fewer_bits_holds_every_value_it_can_wrap_to() {
    for bits in 2..=6u32 {
        for iv in intervals(-40, 40) {
            let w = wrapped(iv, bits);
            for v in points(iv) {
                assert!(w.0 <= stored(v, bits) && stored(v, bits) <= w.1, "{v} in {bits} bits is {}, outside {w:?}", stored(v, bits));
            }
            let fits = iv.0 >= -(1 << (bits - 1)) && iv.1 < (1 << (bits - 1));
            assert_eq!(w == iv, fits, "{iv:?} in {bits} bits: itself exactly when it fits");
        }
    }
    assert_eq!(wrapped(UNBOUNDED, 64), UNBOUNDED);
}

/// The least two's-complement width holding `v`.
fn least_width(v: i64) -> u32 {
    (1..=64).find(|b| *b == 64 || (v >= -(1i64 << (b - 1)) && v < (1i64 << (b - 1)))).unwrap()
}

#[test]
fn signed_bits_is_the_least_width_holding_both_ends() {
    for v in -600..=600 {
        assert_eq!(signed_bits((v, v)), least_width(v), "{v}");
    }
    for iv in intervals(-70, 70) {
        assert_eq!(signed_bits(iv), least_width(iv.0).max(least_width(iv.1)), "{iv:?}");
    }
    for (iv, bits) in [((-128, 127), 8), ((-129, 0), 9), ((0, 128), 9), ((i64::MIN, 0), 64), ((0, i64::MAX), 64), (UNBOUNDED, 64)] {
        assert_eq!(signed_bits(iv), bits, "{iv:?}");
    }
}

/// A zero joined into an interval never costs a bit: the width a register needs to hold its operand's values, or zero
/// where it reads outside its page, is the operand's own.
#[test]
fn joining_zero_never_widens_a_signed_width() {
    for iv in intervals(-300, 300).into_iter().chain([(i64::MIN, -5), (7, i64::MAX), (i64::MIN, i64::MIN), (i64::MAX, i64::MAX)]) {
        assert_eq!(signed_bits(join(iv, (0, 0))), signed_bits(iv), "{iv:?}");
    }
}
