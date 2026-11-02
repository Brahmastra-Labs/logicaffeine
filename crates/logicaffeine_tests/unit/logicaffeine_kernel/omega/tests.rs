use super::*;

#[test]
fn test_int_expr_add() {
    let x = IntExpr::var(0);
    let y = IntExpr::var(1);
    let sum = x.add(&y);
    assert!(!sum.is_constant());
    assert_eq!(sum.get_coeff(0), BigInt::from_i64(1));
    assert_eq!(sum.get_coeff(1), BigInt::from_i64(1));
}

#[test]
fn test_int_expr_cancel() {
    let x = IntExpr::var(0);
    let neg_x = x.neg();
    let zero = x.add(&neg_x);
    assert!(zero.is_constant());
    assert!(zero.constant.is_zero());
}

#[test]
fn test_constraint_satisfied() {
    // -1 <= 0 is satisfied
    let c1 = IntConstraint {
        expr: IntExpr::constant(-1),
        strict: false,
    };
    assert!(c1.is_satisfied_constant());

    // 1 <= 0 is NOT satisfied
    let c2 = IntConstraint {
        expr: IntExpr::constant(1),
        strict: false,
    };
    assert!(!c2.is_satisfied_constant());

    // 0 <= 0 is satisfied
    let c3 = IntConstraint {
        expr: IntExpr::constant(0),
        strict: false,
    };
    assert!(c3.is_satisfied_constant());
}

#[test]
fn test_omega_constant() {
    // 1 <= 0 is unsat
    let constraints = vec![IntConstraint {
        expr: IntExpr::constant(1),
        strict: false,
    }];
    assert!(omega_unsat(&constraints));

    // -1 <= 0 is sat
    let constraints2 = vec![IntConstraint {
        expr: IntExpr::constant(-1),
        strict: false,
    }];
    assert!(!omega_unsat(&constraints2));
}

#[test]
fn test_x_lt_x_plus_1() {
    // x < x + 1 is always true for integers
    // To prove: negation x >= x + 1 is unsat
    // x >= x + 1 means x - x >= 1 means 0 >= 1 which is false

    // Negation constraint: (x+1) - x <= 0 = 1 <= 0
    let constraint = IntConstraint {
        expr: IntExpr::constant(1),
        strict: false,
    };
    assert!(omega_unsat(&[constraint]));
}
