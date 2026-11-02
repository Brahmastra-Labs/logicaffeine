use super::*;
use crate::ast::stmt::{BinaryOpKind, Literal};

/// The bare reduction loop `While i<n { Set item 1 of acc to item 1 of acc + i; Set i to i+1 }`
/// must strength-promote in isolation — proving detection + transform,
/// independent of any pipeline interference.
#[test]
fn promotes_invariant_indexed_rmw() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let (i, n, acc) = (it.intern("i"), it.intern("n"), it.intern("acc"));
    let id = |s| ea.alloc(Expr::Identifier(s)) as &Expr;
    let one = ea.alloc(Expr::Literal(Literal::Number(1)));
    let read = ea.alloc(Expr::Index {
        collection: id(acc),
        index: one,
    });
    let rmw_val = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: read,
        right: id(i),
    });
    let set_elem = Stmt::SetIndex {
        collection: id(acc),
        index: one,
        value: rmw_val,
    };
    let inc = Stmt::Set {
        target: i,
        value: ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left: id(i),
            right: ea.alloc(Expr::Literal(Literal::Number(1))),
        }),
    };
    let body = sa.alloc_slice(vec![set_elem, inc]);
    let cond = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Lt,
        left: id(i),
        right: id(n),
    });
    let loop_stmt = Stmt::While {
        cond,
        body,
        decreasing: None,
    };

    let (out, changed) = reg_promote_stmts(vec![loop_stmt], &ea, &sa, &mut it);
    assert!(changed, "invariant-indexed RMW must register-promote");
    // Output is a guarded `If { load; While; write-back }`.
    match &out[0] {
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            assert!(else_block.is_none(), "guard has no else");
            assert!(
                matches!(then_block[0], Stmt::Let { .. }),
                "loads the element"
            );
            assert!(matches!(then_block[1], Stmt::While { .. }), "the loop");
            assert!(
                matches!(then_block[2], Stmt::SetIndex { .. }),
                "writes back"
            );
        }
        other => panic!("expected guarded If, got {other:?}"),
    }
}

/// A varying index must NOT promote.
#[test]
fn declines_varying_index() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let (i, n, acc) = (it.intern("i"), it.intern("n"), it.intern("acc"));
    let id = |s| ea.alloc(Expr::Identifier(s)) as &Expr;
    // index = i + 1 (varies with the loop counter)
    let vidx = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: id(i),
        right: ea.alloc(Expr::Literal(Literal::Number(1))),
    });
    let read = ea.alloc(Expr::Index {
        collection: id(acc),
        index: vidx,
    });
    let val = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: read,
        right: id(i),
    });
    let set_elem = Stmt::SetIndex {
        collection: id(acc),
        index: vidx,
        value: val,
    };
    let inc = Stmt::Set {
        target: i,
        value: ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left: id(i),
            right: ea.alloc(Expr::Literal(Literal::Number(1))),
        }),
    };
    let body = sa.alloc_slice(vec![set_elem, inc]);
    let cond = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Lt,
        left: id(i),
        right: id(n),
    });
    let (_, changed) = reg_promote_stmts(
        vec![Stmt::While {
            cond,
            body,
            decreasing: None,
        }],
        &ea,
        &sa,
        &mut it,
    );
    assert!(!changed, "a varying index must not promote");
}
