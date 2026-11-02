use super::*;

fn iv(lo: i64, hi: i64) -> Interval {
    Interval {
        lo: Bound::Finite(lo),
        hi: Bound::Finite(hi),
    }
}

#[test]
fn interval_top_and_bottom() {
    let t = <Interval as AbstractDomain>::top();
    assert!(matches!(t.lo, Bound::NegInf));
    assert!(matches!(t.hi, Bound::PosInf));
    assert!(!t.is_bottom());

    let b = <Interval as AbstractDomain>::bottom();
    assert!(b.is_bottom());
}

#[test]
fn interval_join_is_least_upper_bound() {
    // [1,3] ⊔ [5,7] = [1,7]
    let j = iv(1, 3).join(&iv(5, 7));
    assert_eq!(j.lo, Bound::Finite(1));
    assert_eq!(j.hi, Bound::Finite(7));
    // bottom is the identity for join: ⊥ ⊔ x = x
    let jb = Interval::bottom().join(&iv(2, 4));
    assert_eq!(jb.lo, Bound::Finite(2));
    assert_eq!(jb.hi, Bound::Finite(4));
    let jb2 = iv(2, 4).join(&Interval::bottom());
    assert_eq!(jb2.lo, Bound::Finite(2));
    assert_eq!(jb2.hi, Bound::Finite(4));
}

#[test]
fn interval_meet_is_greatest_lower_bound() {
    // [1,5] ⊓ [3,7] = [3,5]
    let m = iv(1, 5).meet(&iv(3, 7));
    assert_eq!(m.lo, Bound::Finite(3));
    assert_eq!(m.hi, Bound::Finite(5));
    // disjoint intervals meet to ⊥
    assert!(iv(1, 2).meet(&iv(5, 6)).is_bottom());
    // top is the identity for meet: ⊤ ⊓ x = x
    let t = Interval::top().meet(&iv(2, 4));
    assert_eq!(t.lo, Bound::Finite(2));
    assert_eq!(t.hi, Bound::Finite(4));
}

#[test]
fn interval_widen_climbs_the_threshold_ladder_then_to_infinity() {
    // CONTRACT UPDATE (EXODIA M9, plan-approved): widening no longer
    // jumps straight to ±∞ — an unstable bound snaps to the next rung
    // of the threshold ladder [-1000..1000], and only past the last
    // rung falls off to ±∞. Loop counters keep finite bounds.
    // rising upper bound snaps to the next rung, stable lower preserved
    let w = iv(0, 5).widen(&iv(0, 10));
    assert_eq!(w.lo, Bound::Finite(0));
    assert_eq!(w.hi, Bound::Finite(10));
    let w_mid = iv(0, 5).widen(&iv(0, 11));
    assert_eq!(w_mid.hi, Bound::Finite(100));
    // beyond the last rung → +∞
    let w_inf = iv(0, 5).widen(&iv(0, 1001));
    assert!(matches!(w_inf.hi, Bound::PosInf));
    // falling lower bound snaps to the next rung down, upper preserved
    let w2 = iv(0, 5).widen(&iv(-3, 5));
    assert_eq!(w2.lo, Bound::Finite(-10));
    assert_eq!(w2.hi, Bound::Finite(5));
    // below the last rung → -∞
    let w2_inf = iv(0, 5).widen(&iv(-1001, 5));
    assert!(matches!(w2_inf.lo, Bound::NegInf));
    // stable interval is a fixpoint (no spurious widening)
    let w3 = iv(0, 5).widen(&iv(0, 5));
    assert_eq!(w3.lo, Bound::Finite(0));
    assert_eq!(w3.hi, Bound::Finite(5));
    // widening from ⊥ yields the new value
    let w4 = Interval::bottom().widen(&iv(2, 4));
    assert_eq!(w4.lo, Bound::Finite(2));
    assert_eq!(w4.hi, Bound::Finite(4));
    // the ascent is FINITE: iterated widening against ever-growing
    // inputs stabilizes within ladder-length steps (termination).
    let mut cur = iv(0, 1);
    let mut steps = 0;
    loop {
        let grown = Interval {
            lo: cur.lo.clone(),
            hi: match cur.hi {
                Bound::Finite(v) => Bound::Finite(v.saturating_add(v.abs() + 1)),
                ref b => b.clone(),
            },
        };
        let next = cur.widen(&grown);
        steps += 1;
        if next.leq(&cur) && cur.leq(&next) {
            break;
        }
        cur = next;
        assert!(steps <= 12, "widening must terminate within the ladder");
    }
    assert!(matches!(cur.hi, Bound::PosInf));
}

#[test]
fn interval_leq_is_the_subset_order() {
    assert!(iv(3, 5).leq(&iv(1, 7)));
    assert!(!iv(1, 7).leq(&iv(3, 5)));
    // ⊥ ⊑ everything; everything ⊑ ⊤
    assert!(Interval::bottom().leq(&iv(1, 2)));
    assert!(iv(1, 2).leq(&<Interval as AbstractDomain>::top()));
    // reflexive
    assert!(iv(1, 2).leq(&iv(1, 2)));
}

#[test]
fn interval_lattice_laws_hold() {
    let a = iv(1, 5);
    let b = iv(3, 9);
    // a ⊑ a⊔b and b ⊑ a⊔b (join is an upper bound)
    assert!(a.leq(&a.join(&b)));
    assert!(b.leq(&a.join(&b)));
    // a⊓b ⊑ a and a⊓b ⊑ b (meet is a lower bound)
    assert!(a.meet(&b).leq(&a));
    assert!(a.meet(&b).leq(&b));
    // commutativity of join and meet
    assert!(a.join(&b).leq(&b.join(&a)) && b.join(&a).leq(&a.join(&b)));
    assert!(a.meet(&b).leq(&b.meet(&a)) && b.meet(&a).leq(&a.meet(&b)));
}

/// Deterministic xorshift64*.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn word(&mut self) -> i64 {
        match self.next() % 5 {
            0 => (self.next() % 40) as i64 - 20,
            1 => (self.next() % (1 << 20)) as i64 - (1 << 19),
            2 => self.next() as i64 >> 20,
            3 => [i64::MIN, i64::MAX, -1, 0, 1, (1 << 31) - 1, 1 << 32][(self.next() % 7) as usize],
            _ => self.next() as i64,
        }
    }
}

/// An interval and values it stands for: finite bounds sample inside; an infinite bound also stands for big
/// integers past the word on its side.
fn arbitrary_interval(rng: &mut Rng) -> (Interval, Vec<crate::interpreter::RuntimeValue>) {
    use crate::interpreter::RuntimeValue;
    let (a, b) = (rng.word(), rng.word());
    let (lo, hi) = (a.min(b), a.max(b));
    let open_lo = rng.next() % 6 == 0;
    let open_hi = rng.next() % 6 == 0;
    let interval = Interval {
        lo: if open_lo { Bound::NegInf } else { Bound::Finite(lo) },
        hi: if open_hi { Bound::PosInf } else { Bound::Finite(hi) },
    };
    let mut values: Vec<RuntimeValue> = vec![RuntimeValue::Int(lo), RuntimeValue::Int(hi)];
    for _ in 0..6 {
        let span = (hi as i128 - lo as i128 + 1) as u128;
        let v = lo as i128 + (rng.next() as u128 % span) as i128;
        values.push(RuntimeValue::Int(v as i64));
    }
    let huge = logicaffeine_base::BigInt::from_i64(i64::MAX).mul(&logicaffeine_base::BigInt::from_i64(4));
    if open_hi {
        values.push(RuntimeValue::from_bigint(huge.add(&logicaffeine_base::BigInt::from_i64(rng.word()))));
        values.push(RuntimeValue::Int(i64::MAX));
    }
    if open_lo {
        values.push(RuntimeValue::from_bigint(huge.negated()));
        values.push(RuntimeValue::Int(i64::MIN));
    }
    (interval, values)
}

fn contains(i: &Interval, v: &crate::interpreter::RuntimeValue) -> bool {
    use crate::interpreter::RuntimeValue;
    let above_lo = |n: i128| match i.lo {
        Bound::Finite(lo) => n >= lo as i128,
        Bound::NegInf => true,
        Bound::PosInf => false,
    };
    let below_hi = |n: i128| match i.hi {
        Bound::Finite(hi) => n <= hi as i128,
        Bound::PosInf => true,
        Bound::NegInf => false,
    };
    match v {
        RuntimeValue::Int(n) => above_lo(*n as i128) && below_hi(*n as i128),
        RuntimeValue::BigInt(b) if b.is_negative() => matches!(i.lo, Bound::NegInf),
        RuntimeValue::BigInt(_) => matches!(i.hi, Bound::PosInf),
        other => panic!("not an integer: {other:?}"),
    }
}

/// Soundness of the bitwise and shift transfers: every value the language's own `&`, `|`, `^` and `<<` give for
/// operands inside two intervals lies inside the interval the transfer computes. A refused pairing gives no value.
#[test]
fn bitwise_and_shift_intervals_hold_every_value_the_reference_computes() {
    use crate::ast::stmt::BinaryOpKind;
    let mut rng = Rng(0x0dd_ba11_c0ff_ee42);
    let mut bounded = 0usize;
    for _ in 0..4000 {
        let (l, ls) = arbitrary_interval(&mut rng);
        let (r, rs) = if rng.next() % 3 == 0 {
            let k = (rng.next() % 80) as i64 - 8;
            (Interval::exact(k), vec![crate::interpreter::RuntimeValue::Int(k)])
        } else {
            arbitrary_interval(&mut rng)
        };
        for (op, out) in [
            (BinaryOpKind::BitAnd, l.bitand(&r)),
            (BinaryOpKind::BitOr, l.bitor_or_xor(&r, true)),
            (BinaryOpKind::BitXor, l.bitor_or_xor(&r, false)),
            (BinaryOpKind::Shl, l.shl(&r)),
        ] {
            if matches!(out.lo, Bound::Finite(_)) || matches!(out.hi, Bound::Finite(_)) {
                bounded += 1;
            }
            for a in &ls {
                for b in &rs {
                    if let Ok(v) = crate::semantics::arith::binary_op(op, a.clone(), b.clone()) {
                        assert!(
                            contains(&out, &v),
                            "{a:?} {op:?} {b:?} = {v:?}, outside {out:?} from {l:?} and {r:?}"
                        );
                    }
                }
            }
        }
    }
    assert!(bounded > 2500, "the transfers bounded only {bounded} results");
}

#[test]
fn a_mask_bounds_whatever_it_masks() {
    let top = <Interval as AbstractDomain>::top();
    let m = top.bitand(&Interval::exact(1048575));
    assert_eq!((m.lo, m.hi), (Bound::Finite(0), Bound::Finite(1048575)));
    let both = iv(0, 300).bitand(&iv(0, 4095));
    assert_eq!((both.lo, both.hi), (Bound::Finite(0), Bound::Finite(300)));
    let signed = iv(-5, 5).bitand(&iv(-7, 3));
    assert!(matches!(signed.lo, Bound::NegInf) && matches!(signed.hi, Bound::PosInf));
    assert_eq!(iv(12, 12).bitand(&iv(10, 10)).is_exact(), Some(8));
}

#[test]
fn or_and_xor_stay_below_the_next_power_of_two() {
    let or = iv(3, 100).bitor_or_xor(&iv(5, 9), true);
    assert_eq!((or.lo, or.hi), (Bound::Finite(5), Bound::Finite(127)));
    let xor = iv(0, 4095).bitor_or_xor(&iv(0, 31), false);
    assert_eq!((xor.lo, xor.hi), (Bound::Finite(0), Bound::Finite(4095)));
    let zero = iv(0, 0).bitor_or_xor(&iv(0, 0), true);
    assert_eq!((zero.lo, zero.hi), (Bound::Finite(0), Bound::Finite(0)));
    let negative = iv(-1, 4).bitor_or_xor(&iv(0, 4), false);
    assert!(matches!(negative.lo, Bound::NegInf));
}

#[test]
fn a_constant_shift_scales_until_it_would_wrap() {
    let s = iv(-50, 56).shl(&Interval::exact(30));
    assert_eq!((s.lo, s.hi), (Bound::Finite(-50 << 30), Bound::Finite(56 << 30)));
    let wrapped = iv(0, 56).shl(&Interval::exact(60));
    assert!(matches!(wrapped.hi, Bound::PosInf), "56 << 60 wraps");
    let mod64 = iv(1, 3).shl(&Interval::exact(65));
    assert_eq!((mod64.lo, mod64.hi), (Bound::Finite(2), Bound::Finite(6)), "a shift counts mod 64");
    let by_name = iv(1, 3).shl(&iv(0, 4));
    assert!(matches!(by_name.lo, Bound::NegInf));
}

/// `left op right` over two names (or a name and a literal), holding or failing, applied to a state where `x` and
/// `y` take the given intervals: the narrowed intervals of `x` and `y`.
fn narrowed(
    op: crate::ast::stmt::BinaryOpKind,
    negated: bool,
    x_iv: Interval,
    y_iv: Interval,
    y_literal: Option<i64>,
) -> (Interval, Interval) {
    use crate::arena::Arena;
    let ea: Arena<Expr> = Arena::new();
    let mut it = crate::intern::Interner::new();
    let (x, y) = (it.intern("x"), it.intern("y"));
    let right: &Expr = match y_literal {
        Some(n) => ea.alloc(Expr::Literal(crate::ast::stmt::Literal::Number(n))),
        None => ea.alloc(Expr::Identifier(y)),
    };
    let cond = ea.alloc(Expr::BinaryOp {
        op,
        left: ea.alloc(Expr::Identifier(x)),
        right,
    });
    let mut st = AbstractState::new();
    st.set_var(x, x_iv);
    st.set_var(y, y_iv);
    if negated {
        narrow_state_negated(cond, &mut st);
    } else {
        narrow_state(cond, &mut st);
    }
    (st.get_var(&x), st.get_var(&y))
}

fn holds_in(i: &Interval, v: i64) -> bool {
    let lo_ok = match i.lo {
        Bound::Finite(lo) => lo <= v,
        Bound::NegInf => true,
        Bound::PosInf => false,
    };
    let hi_ok = match i.hi {
        Bound::Finite(hi) => v <= hi,
        Bound::PosInf => true,
        Bound::NegInf => false,
    };
    lo_ok && hi_ok
}

/// A comparison that holds or fails narrows the names it compares by the other side's interval and keeps every pair
/// of values that satisfies it: for random intervals of `x` and `y` and random values inside them, every pair for
/// which the comparison holds (or, negated, fails) stays inside the narrowed intervals, over every comparison, with
/// `y` a name and with it a literal.
#[test]
fn a_comparison_drops_no_values_that_satisfy_it() {
    use crate::ast::stmt::BinaryOpKind::*;
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut checked = 0u32;
    for _ in 0..4000 {
        let span = |rng: &mut Rng| {
            let (a, b) = (rng.word(), rng.word());
            (a.min(b), a.max(b))
        };
        let ((xl, xh), (yl, yh)) = (span(&mut rng), span(&mut rng));
        let samples = |rng: &mut Rng, lo: i64, hi: i64| -> Vec<i64> {
            let width = (hi as i128 - lo as i128) as u128;
            let mut v = vec![lo, hi];
            for _ in 0..3 {
                v.push((lo as i128 + (rng.next() as u128 % (width + 1)) as i128) as i64);
            }
            v
        };
        let (xs, ys) = (samples(&mut rng, xl, xh), samples(&mut rng, yl, yh));
        for op in [Lt, LtEq, Gt, GtEq, Eq, NotEq] {
            for negated in [false, true] {
                for literal in [false, true] {
                    let (yl, yh) = if literal { (ys[2], ys[2]) } else { (yl, yh) };
                    let (nx, ny) = narrowed(op, negated, iv(xl, xh), iv(yl, yh), literal.then_some(yl));
                    for &vx in &xs {
                        for &vy in &ys {
                            if literal && vy != yl {
                                continue;
                            }
                            let relation = match op {
                                Lt => vx < vy,
                                LtEq => vx <= vy,
                                Gt => vx > vy,
                                GtEq => vx >= vy,
                                Eq => vx == vy,
                                _ => vx != vy,
                            };
                            if relation != negated {
                                checked += 1;
                                assert!(
                                    holds_in(&nx, vx) && (literal || holds_in(&ny, vy)),
                                    "{op:?} negated={negated}: x={vx} in [{xl}, {xh}], y={vy} in [{yl}, {yh}] narrowed to {nx:?}, {ny:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 10_000, "the sweep satisfied comparisons often enough to mean something: {checked}");
}

fn same(a: &Interval, b: &Interval) -> bool {
    a.lo == b.lo && a.hi == b.hi
}

/// What a comparison narrows by: a name compared with another name is bounded by that name's interval, not only by a
/// constant (`j <= limit` with `limit <= 2^31` bounds `j`; it bounded nothing while the other side had to be exact).
#[test]
fn a_comparison_with_a_name_narrows_by_its_interval() {
    use crate::ast::stmt::BinaryOpKind::*;
    let top = Interval { lo: Bound::NegInf, hi: Bound::PosInf };
    let up_to = |hi: i64| Interval { lo: Bound::NegInf, hi: Bound::Finite(hi) };
    let (j, _) = narrowed(LtEq, false, iv(4, i64::MAX), up_to(1 << 31), None);
    assert!(same(&j, &iv(4, 1 << 31)), "j <= limit, limit <= 2^31: {j:?}");
    let (i, n) = narrowed(Lt, false, top.clone(), iv(0, 10), None);
    assert_eq!(i.hi, Bound::Finite(9), "i < n with n <= 10: {i:?}");
    assert!(same(&n, &iv(0, 10)), "n keeps its own interval when i is unbounded below: {n:?}");
    let (x, y) = narrowed(Gt, false, top.clone(), iv(5, 50), None);
    assert_eq!(x.lo, Bound::Finite(6), "x > y, y >= 5: {x:?}");
    assert!(same(&y, &iv(5, 50)), "{y:?}");
    let (x, y) = narrowed(Eq, false, iv(0, 100), iv(3, 7), None);
    assert!(same(&x, &iv(3, 7)) && same(&y, &iv(3, 7)), "x == y meets both: {x:?}, {y:?}");
    let (x, y) = narrowed(LtEq, true, iv(0, 100), iv(3, 7), None);
    assert!(same(&x, &iv(4, 100)), "not x <= y is x > y, y >= 3: {x:?}");
    assert!(same(&y, &iv(3, 7)), "{y:?}");
    let (x, _) = narrowed(Lt, false, iv(0, 100), Interval { lo: Bound::NegInf, hi: Bound::Finite(i64::MIN) }, None);
    assert!(same(&x, &iv(0, 100)), "a strict bound past the word's edge narrows nothing: {x:?}");
}
