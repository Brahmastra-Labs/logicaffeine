//! Soundness of the arithmetic decision procedures under large coefficients.
//!
//! `ring`, `lia`, and `omega` verdicts feed the trusted Derivation-reflection
//! reducers, so their coefficient arithmetic must be exact at every magnitude:
//! a wrapped multiply can equate unequal polynomials, a wrapped Fourier-Motzkin
//! combination can flip a verdict, and a hash collision on variable names can
//! identify two distinct variables. These tests pin the exact-arithmetic
//! contract at the sizes where machine integers break.

use logicaffeine_kernel::lia::{self, Constraint, LinearExpr, Rational};
use logicaffeine_kernel::omega::{self, IntConstraint, IntExpr};
use logicaffeine_kernel::ring::{self, Polynomial};
use logicaffeine_kernel::{Literal, Term, VarInterner};

fn sname(s: &str) -> Term {
    Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(Literal::Text(s.to_string()))),
    )
}

// --- ring -------------------------------------------------------------------

#[test]
fn ring_mul_overflow_does_not_equate_distinct_polynomials() {
    // (2^32·x) · (2^32·x) = 2^64·x² — the coefficient exceeds i64, but the
    // polynomial is emphatically not zero.
    let x = Polynomial::var(0);
    let a = Polynomial::constant(1i64 << 32).mul(&x);
    let p = a.mul(&a);
    assert!(!p.canonical_eq(&Polynomial::zero()));
    assert!(!p.canonical_eq(&Polynomial::constant(0)));
}

#[test]
fn ring_add_overflow_sound() {
    // MAX + MAX must not wrap into -2.
    let m = Polynomial::constant(i64::MAX);
    let s = m.add(&m);
    assert!(!s.canonical_eq(&Polynomial::constant(-2)));
}

#[test]
fn ring_neg_min_sound() {
    // -(i64::MIN) is representable exactly; it is not i64::MIN.
    let p = Polynomial::constant(i64::MIN).neg();
    assert!(!p.canonical_eq(&Polynomial::constant(i64::MIN)));
}

#[test]
fn ring_exponent_no_wrap() {
    // Squaring x sixty-four times yields x^(2^64). A machine-width exponent
    // wraps that to x^0 = 1; the canonical form must keep them distinct.
    let mut p = Polynomial::var(0);
    for _ in 0..64 {
        p = p.mul(&p);
    }
    assert!(!p.canonical_eq(&Polynomial::constant(1)));
    assert!(!p.canonical_eq(&Polynomial::var(0)));
}

#[test]
fn ring_large_exact_equality_holds() {
    // Completeness at scale: (2^40·x)² and 2^60·(2^20·x²) are the same
    // polynomial (2^80·x²) and must compare equal.
    let x = Polynomial::var(0);
    let a = Polynomial::constant(1i64 << 40).mul(&x);
    let lhs = a.mul(&a);
    let x2 = x.mul(&x);
    let rhs = Polynomial::constant(1i64 << 60).mul(&Polynomial::constant(1i64 << 20).mul(&x2));
    assert!(lhs.canonical_eq(&rhs));
}

#[test]
fn ring_name_hash_collision_distinct_vars() {
    // "Aa" and "BB" collide under a 31-based byte hash. Two distinct global
    // names must reify to distinct ring variables, so their difference is a
    // nonzero polynomial — otherwise `ring` proves Aa = BB.
    let mut vars = VarInterner::new();
    let pa = ring::reify(&sname("Aa"), &mut vars).expect("SName reifies");
    let pb = ring::reify(&sname("BB"), &mut vars).expect("SName reifies");
    assert!(!pa.sub(&pb).canonical_eq(&Polynomial::zero()));
}

// --- omega ------------------------------------------------------------------

#[test]
fn omega_scale_overflow_never_wrong_verdict() {
    // { -(2^40+1)·x + 1 ≤ 0,  (2^40+3)·x - (2^62+1) ≤ 0 } is satisfiable
    // (x = 1). Eliminating x multiplies the residues past i64; a wrapped
    // combination flips the verdict to unsat.
    let x = IntExpr::var(0);
    let c1 = IntConstraint {
        expr: x.scale(-((1i64 << 40) + 1)).add(&IntExpr::constant(1)),
        strict: false,
    };
    let c2 = IntConstraint {
        expr: x
            .scale((1i64 << 40) + 3)
            .add(&IntExpr::constant(-((1i64 << 62) + 1))),
        strict: false,
    };
    assert!(!omega::omega_unsat(&[c1, c2]));
}

#[test]
fn omega_name_hash_collision_distinct_vars() {
    let mut vars = VarInterner::new();
    let a = omega::reify_int_linear(&sname("Aa"), &mut vars).expect("SName reifies");
    let b = omega::reify_int_linear(&sname("BB"), &mut vars).expect("SName reifies");
    assert!(!a.sub(&b).is_constant());
}

// --- lia --------------------------------------------------------------------

#[test]
fn lia_fourier_motzkin_total_on_big_coefficients() {
    // { (2^40+1)·x + 1 ≤ 0,  -(2^40+3)·x + (2^62+1) ≤ 0 } forces x < 0 and
    // x ≥ (2^62+1)/(2^40+3) > 0 simultaneously: unsatisfiable. The elimination
    // products exceed i64, so a solver that declines on overflow reports
    // "satisfiable" and loses a refutation it should find.
    let x = LinearExpr::var(0);
    let c1 = Constraint {
        expr: x
            .scale(&Rational::from_i64((1i64 << 40) + 1))
            .add(&LinearExpr::constant(Rational::from_i64(1))),
        strict: false,
    };
    let c2 = Constraint {
        expr: x
            .scale(&Rational::from_i64(-((1i64 << 40) + 3)))
            .add(&LinearExpr::constant(Rational::from_i64((1i64 << 62) + 1))),
        strict: false,
    };
    assert!(lia::fourier_motzkin_unsat(&[c1, c2]));
}

#[test]
fn lia_name_hash_collision_distinct_vars() {
    let mut vars = VarInterner::new();
    let a = lia::reify_linear(&sname("Aa"), &mut vars).expect("SName reifies");
    let b = lia::reify_linear(&sname("BB"), &mut vars).expect("SName reifies");
    assert!(!a.sub(&b).is_constant());
}
