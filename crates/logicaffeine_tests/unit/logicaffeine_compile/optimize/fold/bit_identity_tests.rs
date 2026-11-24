//! A bit operation with its identity operand folds to its other operand; `x & 0` folds to `0`.
use super::*;

/// `x op k` (or `k op x` when `flipped`) folded, with `x` a name: the name, the literal, or neither.
fn folds_to(op: BinaryOpKind, k: i64, flipped: bool) -> Option<&'static str> {
    crate::ui_bridge::with_parsed_program("## Main\nLet x be 10.\nShow x.\n", |parsed, interner| {
        let (stmts, _, _) = parsed.as_ref().ok()?;
        let x = interner.lookup("x")?;
        let bools = program_bool_syms(stmts, interner);
        let ea: Arena<Expr> = Arena::new();
        let name = ea.alloc(Expr::Identifier(x));
        let literal = ea.alloc(Expr::Literal(Literal::Number(k)));
        let (left, right) = if flipped { (literal, name) } else { (name, literal) };
        match try_simplify_algebraic(op, left, right, &ea, &bools) {
            Some(Expr::Identifier(s)) if *s == x => Some("x"),
            Some(Expr::Literal(Literal::Number(n))) if *n == k => Some("k"),
            Some(_) => Some("other"),
            None => None,
        }
    })
}

#[test]
fn an_identity_operand_leaves_the_other() {
    assert_eq!(folds_to(BinaryOpKind::BitAnd, -1, false), Some("x"));
    assert_eq!(folds_to(BinaryOpKind::BitAnd, -1, true), Some("x"));
    assert_eq!(folds_to(BinaryOpKind::BitOr, 0, false), Some("x"));
    assert_eq!(folds_to(BinaryOpKind::BitOr, 0, true), Some("x"));
    assert_eq!(folds_to(BinaryOpKind::BitXor, 0, true), Some("x"));
    assert_eq!(folds_to(BinaryOpKind::Shl, 0, false), Some("x"));
    assert_eq!(folds_to(BinaryOpKind::Shr, 0, false), Some("x"));
}

#[test]
fn a_zero_mask_is_zero() {
    assert_eq!(folds_to(BinaryOpKind::BitAnd, 0, false), Some("k"));
    assert_eq!(folds_to(BinaryOpKind::BitAnd, 0, true), Some("k"));
}

#[test]
fn a_shift_of_zero_and_any_other_operand_is_kept() {
    assert_eq!(folds_to(BinaryOpKind::Shl, 0, true), None, "`0 << x` is 0 only for a total x, and is not `x`");
    assert_eq!(folds_to(BinaryOpKind::BitAnd, 255, false), None);
    assert_eq!(folds_to(BinaryOpKind::BitOr, 1, false), None);
    assert_eq!(folds_to(BinaryOpKind::BitXor, -1, false), None);
}
