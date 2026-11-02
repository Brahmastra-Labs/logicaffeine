//! `open_mutual` substitutes each projection of a mutual block under the mutual binders not yet
//! opened. A block whose bodies mention an OUTER variable (a recursor applied to local motives
//! and minors) must keep that variable pointing at the same binder after unfolding.
use super::*;

/// `mutualfix { f := λ. g (Var o) ; g := λ. f (Var o) }` where `Var o` is the outer variable at
/// de Bruijn index `outer` in the block's context. Inside each body the indices are: 0 the
/// body's own λ, 1 = `g`, 2 = `f`, 3.. the outer context.
fn block(outer: usize) -> Vec<Db> {
    let body = |call: usize| {
        Db::Lam(
            DbBox::new(Db::Sort(Universe::Prop)),
            DbBox::new(Db::App(
                DbBox::new(Db::Var(call)),
                DbBox::new(Db::Var(outer + 3)),
            )),
        )
    };
    vec![body(1), body(2)]
}

#[test]
fn opening_a_block_keeps_its_outer_variables_on_their_binders() {
    let defs = block(0);
    // Opening `f` gives `λ. (mutualfix{..}.1) (Var o)`, with the outer variable one binder up
    // (index 1 inside the λ) in the projection and in the argument alike.
    let opened = open_mutual(&defs, 0);
    let Db::Lam(_, body) = &opened else {
        panic!("an opened body stays a λ: {opened:?}")
    };
    let Db::App(head, arg) = body.as_ref() else {
        panic!("{body:?}")
    };
    assert_eq!(
        arg.as_ref(),
        &Db::Var(1),
        "the argument's outer variable keeps its binder"
    );
    let projection = shift(
        &Db::MutualFix {
            defs: defs.clone(),
            index: 1,
        },
        1,
        0,
    );
    assert_eq!(
        head.as_ref(),
        &projection,
        "the projection is the block itself, under the λ"
    );
    // Both projections open to bodies that still mention the SAME outer variable.
    for index in 0..2 {
        let Db::Lam(_, body) = open_mutual(&defs, index) else {
            panic!()
        };
        let Db::App(_, arg) = body.as_ref() else {
            panic!()
        };
        assert_eq!(arg.as_ref(), &Db::Var(1));
    }
}

#[test]
fn a_closed_block_opens_exactly_as_before() {
    let closed = vec![
        Db::Lam(DbBox::new(Db::Sort(Universe::Prop)), DbBox::new(Db::Var(1))),
        Db::Lam(DbBox::new(Db::Sort(Universe::Prop)), DbBox::new(Db::Var(2))),
    ];
    let Db::Lam(_, body) = open_mutual(&closed, 0) else {
        panic!()
    };
    assert_eq!(
        body.as_ref(),
        &shift(
            &Db::MutualFix {
                defs: closed.clone(),
                index: 1
            },
            1,
            0
        )
    );
    let Db::Lam(_, body) = open_mutual(&closed, 1) else {
        panic!()
    };
    assert_eq!(
        body.as_ref(),
        &shift(
            &Db::MutualFix {
                defs: closed.clone(),
                index: 0
            },
            1,
            0
        )
    );
}
