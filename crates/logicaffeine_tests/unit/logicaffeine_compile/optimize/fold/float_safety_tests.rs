use super::*;

/// CRITIQUE #8 — IEEE float safety. `x * 0.0 → 0.0` is wrong for `Inf`/`NaN` (both give `NaN`, not
/// `0.0`), and `x + 0.0 → x` / `0.0 + x → x` is wrong for `-0.0` (`-0.0 + 0.0 == +0.0`, flipping the
/// sign bit). The project's own e-graph already refuses float rewrites for exactly this reason
/// (`egraph/rules.rs`: "Float operands never rewrite"); the scalar fold pass must not apply them
/// either. A non-literal float operand `x` must leave these expressions untouched.
#[test]
fn float_zero_algebraic_identities_are_not_applied() {
    let ea: Arena<Expr> = Arena::new();
    let mut it = Interner::new();
    let x = ea.alloc(Expr::Identifier(it.intern("x")));
    let zero_f = ea.alloc(Expr::Literal(Literal::Float(0.0)));

    assert!(
        try_simplify_algebraic(BinaryOpKind::Multiply, x, zero_f, &ea, &BoolSyms::new()).is_none(),
        "x * 0.0 must NOT fold to 0.0 — inf*0.0 = NaN"
    );
    assert!(
        try_simplify_algebraic(BinaryOpKind::Multiply, zero_f, x, &ea, &BoolSyms::new()).is_none(),
        "0.0 * x must NOT fold to 0.0"
    );
    assert!(
        try_simplify_algebraic(BinaryOpKind::Add, x, zero_f, &ea, &BoolSyms::new()).is_none(),
        "x + 0.0 must NOT fold to x — -0.0 + 0.0 = +0.0 (sign flip)"
    );
    assert!(
        try_simplify_algebraic(BinaryOpKind::Add, zero_f, x, &ea, &BoolSyms::new()).is_none(),
        "0.0 + x must NOT fold to x"
    );
}

/// Regression: the INTEGER identities are exact (`+` promotes to BigInt; `*` likewise) and must keep
/// folding. Removing the float rewrites must not touch these.
#[test]
fn integer_zero_algebraic_identities_still_apply() {
    let ea: Arena<Expr> = Arena::new();
    let mut it = Interner::new();
    let x = ea.alloc(Expr::Identifier(it.intern("x")));
    let zero_i = ea.alloc(Expr::Literal(Literal::Number(0)));

    assert!(
        try_simplify_algebraic(BinaryOpKind::Add, x, zero_i, &ea, &BoolSyms::new()).is_some(),
        "x + 0 → x still folds (int)"
    );
    assert!(
        try_simplify_algebraic(BinaryOpKind::Multiply, x, zero_i, &ea, &BoolSyms::new()).is_some(),
        "x * 0 → 0 still folds (int, x total)"
    );
}
