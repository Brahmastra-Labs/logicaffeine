use super::*;
use crate::ast::stmt::{Literal, TypeExpr};

fn expr_has_call(e: &Expr, target: Symbol) -> bool {
    match e {
        Expr::Call { function, args } => {
            *function == target || args.iter().any(|a| expr_has_call(a, target))
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_has_call(left, target) || expr_has_call(right, target)
        }
        Expr::Not { operand } => expr_has_call(operand, target),
        Expr::Index { collection, index } => {
            expr_has_call(collection, target) || expr_has_call(index, target)
        }
        Expr::Length { collection } => expr_has_call(collection, target),
        _ => false,
    }
}

fn stmt_has_call(s: &Stmt, target: Symbol) -> bool {
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_has_call(value, target),
        Stmt::Show { object, .. } => expr_has_call(object, target),
        _ => false,
    }
}

/// `dec(a) { Let mutable v be a. While v > 100: Set v to v - 100. Return v. }`
/// — a loopy leaf — called as `Let y be dec(5).` inlines: the call is
/// replaced by an identifier and the body is spliced in ahead of the use,
/// alpha-renamed.
#[test]
fn inlines_statement_body_leaf_and_removes_call() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();

    let dec = it.intern("dec");
    let a = it.intern("a");
    let v = it.intern("v");
    let y = it.intern("y");
    let int_ty: &TypeExpr = ta.alloc(TypeExpr::Primitive(it.intern("Int")));

    let loop_stmt = Stmt::While {
        cond: ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Gt,
            left: ea.alloc(Expr::Identifier(v)),
            right: ea.alloc(Expr::Literal(Literal::Number(100))),
        }),
        body: sa.alloc_slice(vec![Stmt::Set {
            target: v,
            value: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Subtract,
                left: ea.alloc(Expr::Identifier(v)),
                right: ea.alloc(Expr::Literal(Literal::Number(100))),
            }),
        }]),
        decreasing: None,
    };
    let body = sa.alloc_slice(vec![
        Stmt::Let {
            var: v,
            ty: None,
            value: ea.alloc(Expr::Identifier(a)),
            mutable: true,
        },
        loop_stmt,
        Stmt::Return {
            value: Some(ea.alloc(Expr::Identifier(v))),
        },
    ]);
    let func = Stmt::FunctionDef {
        name: dec,
        generics: vec![],
        params: vec![(a, int_ty)],
        body,
        return_type: Some(int_ty),
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    };
    let call = &*ea.alloc(Expr::Call {
        function: dec,
        args: vec![ea.alloc(Expr::Literal(Literal::Number(5)))],
    });
    let main_let = Stmt::Let {
        var: y,
        ty: None,
        value: call,
        mutable: false,
    };

    let out = inline_leaf_fns(vec![func, main_let], &ea, &sa, &mut it);

    // func + (param bind, `Let v`, `While`, result bind) + `Let y` = 6.
    assert_eq!(
        out.len(),
        6,
        "expected the loopy body spliced ahead of the use"
    );
    // No call to `dec` survives outside the (retained) definition.
    assert!(
        out[1..].iter().all(|s| !stmt_has_call(s, dec)),
        "the call should be inlined away"
    );
    // The final `Let y` now binds an identifier (the inlined result), not a call.
    match out.last().unwrap() {
        Stmt::Let { value, var, .. } => {
            assert_eq!(*var, y);
            assert!(
                matches!(value, Expr::Identifier(_)),
                "y should bind the result temp"
            );
        }
        other => panic!("expected `Let y`, got {other:?}"),
    }
}

/// A loop-free leaf (`sq(n) { Let t be n * n. Return t. }`) inlines too —
/// every eligible pure leaf folds in so a hot-path call never crosses the
/// frame boundary.
#[test]
fn loop_free_leaf_also_inlines() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();

    let sq = it.intern("sq");
    let n = it.intern("n");
    let t = it.intern("t");
    let y = it.intern("y");
    let int_ty: &TypeExpr = ta.alloc(TypeExpr::Primitive(it.intern("Int")));

    let body = sa.alloc_slice(vec![
        Stmt::Let {
            var: t,
            ty: None,
            value: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Multiply,
                left: ea.alloc(Expr::Identifier(n)),
                right: ea.alloc(Expr::Identifier(n)),
            }),
            mutable: false,
        },
        Stmt::Return {
            value: Some(ea.alloc(Expr::Identifier(t))),
        },
    ]);
    let func = Stmt::FunctionDef {
        name: sq,
        generics: vec![],
        params: vec![(n, int_ty)],
        body,
        return_type: Some(int_ty),
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    };
    let call = &*ea.alloc(Expr::Call {
        function: sq,
        args: vec![ea.alloc(Expr::Literal(Literal::Number(5)))],
    });
    let main_let = Stmt::Let {
        var: y,
        ty: None,
        value: call,
        mutable: false,
    };

    let out = inline_leaf_fns(vec![func, main_let], &ea, &sa, &mut it);

    // func + (param bind, renamed `Let t`, result bind) + `Let y` = 5.
    assert_eq!(out.len(), 5, "loop-free leaf body should be spliced in");
    assert!(
        out[1..].iter().all(|s| !stmt_has_call(s, sq)),
        "the call to a loop-free leaf must be inlined away"
    );
}

/// A helper with a `While` loop and a trailing return (the gcd shape) is a
/// candidate; one with an early return inside the loop is rejected.
#[test]
fn early_return_body_is_not_a_candidate() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let mut it = Interner::new();
    let n = it.intern("n");

    // `If n > 0: Return n.  Return 0.` — an early return → rejected.
    let early = sa.alloc_slice(vec![
        Stmt::If {
            cond: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Gt,
                left: ea.alloc(Expr::Identifier(n)),
                right: ea.alloc(Expr::Literal(Literal::Number(0))),
            }),
            then_block: sa.alloc_slice(vec![Stmt::Return {
                value: Some(ea.alloc(Expr::Identifier(n))),
            }]),
            else_block: None,
        },
        Stmt::Return {
            value: Some(ea.alloc(Expr::Literal(Literal::Number(0)))),
        },
    ]);
    assert!(
        as_candidate(early).is_none(),
        "early return must not be inlinable"
    );
}
