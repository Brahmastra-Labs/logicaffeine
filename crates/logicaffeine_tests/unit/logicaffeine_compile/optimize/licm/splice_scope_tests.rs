//! Unswitching splices a branch in front of the rest of the loop body. A name the branch declares
//! ends with the branch; spliced, it would reach the reads after it. So the unswitch fires only when
//! none of the branch's own names is one the rest of the body mentions.
use super::*;
use crate::ast::stmt::Literal;

fn names(it: &mut Interner) -> (Symbol, Symbol, Symbol, Symbol, Symbol) {
    (
        it.intern("flag"),
        it.intern("x"),
        it.intern("i"),
        it.intern("t"),
        it.intern("show"),
    )
}

/// `While i < 3: If flag: Let <bound> be 5. Show <bound>. Otherwise: Show 0. Show x. Set i to i + 1.`
fn loop_with_branch<'a>(
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    (flag, x, i, _, show_fn): (Symbol, Symbol, Symbol, Symbol, Symbol),
    bound: Symbol,
) -> Stmt<'a> {
    let id = |s: Symbol| -> &'a Expr<'a> { ea.alloc(Expr::Identifier(s)) };
    let num = |n: i64| -> &'a Expr<'a> { ea.alloc(Expr::Literal(Literal::Number(n))) };
    let show = |e: &'a Expr<'a>| Stmt::Show {
        object: e,
        recipient: id(show_fn),
    };
    let then_block = sa.alloc_slice(vec![
        Stmt::Let {
            var: bound,
            ty: None,
            value: num(5),
            mutable: false,
        },
        show(id(bound)),
    ]);
    let else_block = sa.alloc_slice(vec![show(num(0))]);
    let body = sa.alloc_slice(vec![
        Stmt::If {
            cond: id(flag),
            then_block,
            else_block: Some(else_block),
        },
        show(id(x)),
        Stmt::Set {
            target: i,
            value: ea.alloc(Expr::BinaryOp {
                op: crate::ast::stmt::BinaryOpKind::Add,
                left: id(i),
                right: num(1),
            }),
        },
    ]);
    Stmt::While {
        cond: ea.alloc(Expr::BinaryOp {
            op: crate::ast::stmt::BinaryOpKind::Lt,
            left: id(i),
            right: num(3),
        }),
        body,
        decreasing: None,
    }
}

fn unswitched(out: &[Stmt]) -> bool {
    matches!(out, [Stmt::If { then_block, .. }] if matches!(then_block, [Stmt::While { .. }]))
}

#[test]
fn a_branch_whose_name_the_rest_reads_is_not_spliced() {
    let (ea, sa) = (Arena::new(), Arena::new());
    let mut it = Interner::new();
    let n = names(&mut it);
    let out = licm_stmts_run(vec![loop_with_branch(&ea, &sa, n, n.1)], &ea, &sa, &mut it);
    assert!(
        !unswitched(&out),
        "unswitching spliced `Let x` in front of the loop's `Show x`: {out:?}"
    );
}

#[test]
fn a_branch_whose_names_the_rest_never_reads_still_unswitches() {
    let (ea, sa) = (Arena::new(), Arena::new());
    let mut it = Interner::new();
    let n = names(&mut it);
    let out = licm_stmts_run(vec![loop_with_branch(&ea, &sa, n, n.3)], &ea, &sa, &mut it);
    assert!(
        unswitched(&out),
        "the guard must not stop an unswitch that is sound: {out:?}"
    );
}
