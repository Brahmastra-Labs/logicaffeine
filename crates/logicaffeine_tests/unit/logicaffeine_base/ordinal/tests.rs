use super::*;

fn n(k: u64) -> Ordinal {
    Ordinal::from_u64(k)
}

fn w() -> Ordinal {
    Ordinal::omega()
}

/// ω^e
fn wp(e: &Ordinal) -> Ordinal {
    Ordinal::omega_pow(e.clone())
}

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// A random ordinal below ω^(ω+2) with small coefficients.
fn random_ordinal(st: &mut u64, depth: u32) -> Ordinal {
    let terms = xorshift(st) % 4;
    let mut acc = Ordinal::zero();
    for _ in 0..terms {
        let exp = if depth == 0 || !xorshift(st).is_multiple_of(3) {
            n(xorshift(st) % 4)
        } else {
            random_ordinal(st, depth - 1)
        };
        let coef = 1 + xorshift(st) % 3;
        acc = wp(&exp).mul(&n(coef)).add(&acc);
    }
    acc
}

// ── construction and display ──────────────────────────────────────────────────────────────────

#[test]
fn naturals_omega_and_their_display() {
    assert_eq!(Ordinal::zero().to_string(), "0");
    assert_eq!(n(7).to_string(), "7");
    assert_eq!(w().to_string(), "ω");
    assert_eq!(w().add(&n(1)).to_string(), "ω + 1");
    assert_eq!(w().mul(&n(2)).to_string(), "ω·2");
    assert_eq!(wp(&n(2)).mul(&n(3)).add(&w()).add(&n(5)).to_string(), "ω^2·3 + ω + 5");
    assert_eq!(wp(&w()).to_string(), "ω^ω");
    assert_eq!(wp(&w().add(&n(1))).to_string(), "ω^(ω + 1)");
    assert_eq!(wp(&w().mul(&n(2))).to_string(), "ω^(ω·2)");
    assert_eq!(wp(&wp(&w())).to_string(), "ω^ω^ω");
}

#[test]
fn finite_ordinals_are_the_naturals() {
    for a in 0..20u64 {
        assert_eq!(n(a).to_natural(), Some(BigInt::from_u64(a)));
        assert!(n(a).is_finite());
        for b in 0..20u64 {
            assert_eq!(n(a).add(&n(b)), n(a + b));
            assert_eq!(n(a).mul(&n(b)), n(a * b));
            assert_eq!(n(a).cmp(&n(b)), a.cmp(&b));
        }
    }
    assert_eq!(w().to_natural(), None);
    assert!(!w().is_finite());
}

#[test]
fn successors_and_limits() {
    assert!(w().is_limit());
    assert!(!w().is_successor());
    assert!(w().add(&n(1)).is_successor());
    assert!(!w().add(&n(1)).is_limit(), "ω + 1 is a successor, not a limit");
    assert!(!wp(&w()).add(&w()).add(&n(7)).is_limit());
    assert!(wp(&w()).add(&w()).is_limit());
    assert!(n(3).is_successor());
    assert!(!Ordinal::zero().is_successor() && !Ordinal::zero().is_limit());
    assert!(wp(&n(2)).add(&w()).is_limit());
    assert_eq!(w().add(&n(3)).predecessor(), Some(w().add(&n(2))));
    assert_eq!(w().predecessor(), None);
    assert_eq!(n(1).predecessor(), Some(Ordinal::zero()));
    assert_eq!(w().successor(), w().add(&n(1)));
}

// ── the classic non-commutativity ─────────────────────────────────────────────────────────────

#[test]
fn addition_absorbs_on_the_left_but_not_on_the_right() {
    assert_eq!(n(1).add(&w()), w());
    assert_ne!(w().add(&n(1)), w());
    assert!(w().add(&n(1)) > w());
    assert_eq!(n(5).add(&w()).add(&n(3)), w().add(&n(3)));
    assert_eq!(w().add(&wp(&n(2))), wp(&n(2)));
    assert_eq!(wp(&n(2)).add(&w()), wp(&n(2)).add(&w()));
    assert_ne!(wp(&n(2)).add(&w()), wp(&n(2)));
}

#[test]
fn multiplication_absorbs_finite_factors_on_the_left() {
    assert_eq!(n(2).mul(&w()), w());
    assert_eq!(w().mul(&n(2)), w().add(&w()));
    assert_eq!(w().add(&n(1)).mul(&n(2)), w().mul(&n(2)).add(&n(1)));
    assert_eq!(n(2).mul(&w().add(&n(1))), w().add(&n(2)));
    assert_eq!(w().add(&n(1)).mul(&w().add(&n(1))), wp(&n(2)).add(&w()).add(&n(1)));
    assert_eq!(w().mul(&w()), wp(&n(2)));
    assert_eq!(n(0).mul(&w()), Ordinal::zero());
    assert_eq!(w().mul(&n(0)), Ordinal::zero());
}

#[test]
fn distributivity_holds_on_the_left_only() {
    let mut st = 0xa5a5_5a5a_1234_4321;
    for _ in 0..300 {
        let a = random_ordinal(&mut st, 2);
        let b = random_ordinal(&mut st, 2);
        let c = random_ordinal(&mut st, 2);
        assert_eq!(a.mul(&b.add(&c)), a.mul(&b).add(&a.mul(&c)), "{a} · ({b} + {c})");
    }
    let two = n(1).add(&n(1));
    assert_ne!(two.mul(&w()), w().add(&w()), "(1 + 1)·ω = ω ≠ ω + ω");
}

#[test]
fn addition_and_multiplication_are_associative() {
    let mut st = 0x0f0f_f0f0_9876_6789;
    for _ in 0..300 {
        let a = random_ordinal(&mut st, 2);
        let b = random_ordinal(&mut st, 2);
        let c = random_ordinal(&mut st, 2);
        assert_eq!(a.add(&b).add(&c), a.add(&b.add(&c)));
        assert_eq!(a.mul(&b).mul(&c), a.mul(&b.mul(&c)));
    }
}

// ── order ─────────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_order_is_cantor_normal_form_lexicographic_and_monotone() {
    assert!(n(1_000_000) < w());
    assert!(w().mul(&n(1_000)) < wp(&n(2)));
    assert!(wp(&n(1_000)) < wp(&w()));
    assert!(wp(&w()).add(&n(1)) > wp(&w()));
    let mut st = 0x1111_aaaa_2222_bbbb;
    for _ in 0..300 {
        let a = random_ordinal(&mut st, 2);
        let b = random_ordinal(&mut st, 2);
        let c = random_ordinal(&mut st, 2);
        assert_eq!(a.cmp(&b), b.cmp(&a).reverse());
        if a < b {
            assert!(c.add(&a) < c.add(&b), "strict on the right summand");
            assert!(a.add(&c) <= b.add(&c), "weak on the left summand");
            if !c.is_zero() {
                assert!(c.mul(&a) < c.mul(&b), "strict on the right factor");
                assert!(a.mul(&c) <= b.mul(&c), "weak on the left factor");
            }
        }
        assert!(a.add(&b) >= b, "a + b ≥ b");
        assert!(a.add(&b) >= a);
    }
}

// ── exponentiation ────────────────────────────────────────────────────────────────────────────

#[test]
fn exponentiation_by_naturals_and_by_omega() {
    assert_eq!(n(2).pow(&n(10)), n(1024));
    assert_eq!(n(2).pow(&w()), w(), "2^ω = ω");
    assert_eq!(n(10).pow(&w()), w());
    assert_eq!(n(2).pow(&w().add(&n(1))), w().mul(&n(2)), "2^(ω+1) = ω·2");
    assert_eq!(w().pow(&n(3)), wp(&n(3)));
    assert_eq!(w().pow(&w()), wp(&w()));
    assert_eq!(w().add(&n(1)).pow(&n(2)), wp(&n(2)).add(&w()).add(&n(1)));
    assert_eq!(w().add(&n(1)).pow(&w()), wp(&w()), "(ω+1)^ω = ω^ω");
    assert_eq!(n(2).pow(&w().mul(&w())), wp(&w()), "2^(ω²) = ω^ω");
    assert_eq!(n(1).pow(&wp(&w())), n(1));
    assert_eq!(Ordinal::zero().pow(&w()), Ordinal::zero());
    assert_eq!(w().pow(&Ordinal::zero()), n(1));
    assert_eq!(Ordinal::zero().pow(&Ordinal::zero()), n(1));
}

#[test]
fn exponent_laws_hold() {
    let mut st = 0x3333_dddd_4444_eeee;
    let bases = [n(2), n(3), w(), w().add(&n(1)), wp(&n(2)).add(&n(3))];
    for _ in 0..120 {
        let a = &bases[(xorshift(&mut st) % bases.len() as u64) as usize];
        let b = random_ordinal(&mut st, 1);
        let c = random_ordinal(&mut st, 1);
        assert_eq!(a.pow(&b.add(&c)), a.pow(&b).mul(&a.pow(&c)), "{a}^({b} + {c})");
        assert_eq!(a.pow(&b).pow(&c), a.pow(&b.mul(&c)), "({a}^{b})^{c}");
    }
}

#[test]
fn omega_powers_compose() {
    assert_eq!(wp(&n(1)), w());
    assert_eq!(wp(&Ordinal::zero()), n(1));
    assert_eq!(wp(&n(2)).mul(&wp(&n(3))), wp(&n(5)));
    assert_eq!(wp(&w()).mul(&w()), wp(&w().add(&n(1))));
    assert_eq!(w().mul(&wp(&w())), wp(&w()), "ω·ω^ω = ω^(1+ω) = ω^ω");
}
