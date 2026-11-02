use super::*;

#[test]
fn test_rational_arithmetic() {
    let half = Rational::from_ratio_i64(1, 2).unwrap();
    let third = Rational::from_ratio_i64(1, 3).unwrap();
    let sum = half.add(&third);
    assert_eq!(sum, Rational::from_ratio_i64(5, 6).unwrap());
}

#[test]
fn test_linear_expr_add() {
    let x = LinearExpr::var(0);
    let y = LinearExpr::var(1);
    let sum = x.add(&y);
    assert!(!sum.is_constant());
    assert_eq!(sum.get_coeff(0), Rational::from_i64(1));
    assert_eq!(sum.get_coeff(1), Rational::from_i64(1));
}

#[test]
fn test_linear_expr_cancel() {
    let x = LinearExpr::var(0);
    let neg_x = x.neg();
    let zero = x.add(&neg_x);
    assert!(zero.is_constant());
    assert!(zero.constant.is_zero());
}

#[test]
fn test_constraint_satisfied() {
    // -1 <= 0 is satisfied
    let c1 = Constraint {
        expr: LinearExpr::constant(Rational::from_i64(-1)),
        strict: false,
    };
    assert!(c1.is_satisfied_constant());

    // 1 <= 0 is NOT satisfied
    let c2 = Constraint {
        expr: LinearExpr::constant(Rational::from_i64(1)),
        strict: false,
    };
    assert!(!c2.is_satisfied_constant());

    // 0 <= 0 is satisfied
    let c3 = Constraint {
        expr: LinearExpr::constant(Rational::zero()),
        strict: false,
    };
    assert!(c3.is_satisfied_constant());

    // 0 < 0 is NOT satisfied (strict)
    let c4 = Constraint {
        expr: LinearExpr::constant(Rational::zero()),
        strict: true,
    };
    assert!(!c4.is_satisfied_constant());
}

#[test]
fn test_fourier_motzkin_constant() {
    // Single constraint: 1 <= 0 (false)
    let constraints = vec![Constraint {
        expr: LinearExpr::constant(Rational::from_i64(1)),
        strict: false,
    }];
    assert!(fourier_motzkin_unsat(&constraints));

    // Single constraint: -1 <= 0 (true)
    let constraints2 = vec![Constraint {
        expr: LinearExpr::constant(Rational::from_i64(-1)),
        strict: false,
    }];
    assert!(!fourier_motzkin_unsat(&constraints2));
}

// A constraint `c·x + d <= 0` (or `< 0`) from an integer triple.
fn c(cx: i64, d: i64, strict: bool) -> Constraint {
    let mut e = LinearExpr::constant(Rational::from_i64(d));
    e = e.add(&LinearExpr::var(0).scale(&Rational::from_i64(cx)));
    Constraint { expr: e, strict }
}

#[test]
fn nonunit_coeff_satisfiable_is_not_unsat() {
    // 2x + 4 <= 0  (x <= -2)  ∧  -x - 3 <= 0  (x >= -3).  x = -2 satisfies
    // both, so the system is SATISFIABLE — `unsat` MUST be false. The
    // dropped-division bug derives a spurious contradiction here (UNSOUND).
    let sys = vec![c(2, 4, false), c(-1, -3, false)];
    assert!(
        !fourier_motzkin_unsat(&sys),
        "satisfiable non-unit system wrongly reported unsatisfiable (unsound FM)"
    );
}

#[test]
fn nonunit_coeff_unsat_is_detected() {
    // 3x - 6 <= 0  (x <= 2)  ∧  -x + 3 <= 0  (x >= 3).  No integer (or
    // rational) x satisfies both → UNSAT must be true. The bug misses it.
    let sys = vec![c(3, -6, false), c(-1, 3, false)];
    assert!(
        fourier_motzkin_unsat(&sys),
        "unsatisfiable non-unit system not detected (incomplete FM)"
    );
}

#[test]
fn nonunit_coeff_strict_and_larger() {
    // 5x <= 12  (x <= 2.4)  ∧  3x >= 9  (x >= 3): unsat (2.4 < 3).
    let sys = vec![c(5, -12, false), c(-3, 9, false)];
    assert!(fourier_motzkin_unsat(&sys));
    // 5x <= 15 (x <= 3) ∧ 3x >= 9 (x >= 3): x = 3 satisfies — SAT.
    let sys2 = vec![c(5, -15, false), c(-3, 9, false)];
    assert!(!fourier_motzkin_unsat(&sys2));
}

#[test]
fn overflow_fails_closed_to_satisfiable() {
    // `4e9·x - 1 <= 0` (x <= 1/4e9) ∧ `-3e9·x - 1 <= 0` (x >= -1/3e9): the
    // interval contains 0, so the system is SATISFIABLE. Isolating x makes
    // bounds with denominators 4e9 and 3e9; combining them needs the
    // product 4e9·3e9 = 1.2e19, which exceeds i64. Exact arbitrary-
    // precision arithmetic must compute straight through it and report the
    // correct verdict.
    let sys = vec![c(4_000_000_000, -1, false), c(-3_000_000_000, -1, false)];
    assert!(
        !fourier_motzkin_unsat(&sys),
        "large denominators must be computed exactly, and this system is satisfiable"
    );
}

#[test]
fn test_x_lt_x_plus_1() {
    // x < x + 1 is always true
    // Negation: x >= x + 1, i.e., x - x - 1 >= 0, i.e., -1 >= 0
    // Constraint: -(-1) <= 0 => 1 <= 0 which is unsat => goal is valid
    let x = LinearExpr::var(0);
    let one = LinearExpr::constant(Rational::from_i64(1));
    let _xp1 = x.add(&one);

    // For Lt(x, x+1): negation constraint is (x+1 - x) <= 0 = 1 <= 0
    let constraint = Constraint {
        expr: LinearExpr::constant(Rational::from_i64(1)),
        strict: false,
    };
    // 1 <= 0 is unsat, so goal is valid
    assert!(fourier_motzkin_unsat(&[constraint]));
}
