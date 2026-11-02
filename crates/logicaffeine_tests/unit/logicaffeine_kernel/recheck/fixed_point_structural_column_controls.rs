use super::*;

fn context() -> Context {
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    ctx
}
fn g(name: &str) -> Db {
    Db::Global(name.into())
}
fn app(f: Db, x: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(x))
}
fn binary(name: &str, x: Db, y: Db) -> Db {
    app(app(g(name), x), y)
}

#[test]
fn constructor_in_a_nonrecursive_column_does_not_unfold_a_neutral_fixpoint() {
    let ctx = context();
    for name in ["Nat_add", "Nat_mul"] {
        for first in [g("Zero"), app(g("Succ"), g("Zero"))] {
            take_budget_exhausted();
            let reduced = whnf_in(&ctx, &[g("Nat")], &binary(name, first.clone(), Db::Var(0)));
            let (head, args) = spine_ref(&reduced);
            assert!(
                matches!(head, Db::Fix(_)),
                "the recursive second argument is neutral; the fixpoint must stay folded"
            );
            assert_eq!(args.len(), 2);
            assert_eq!(args[0], &first);
            assert_eq!(args[1], &Db::Var(0));
            assert!(!take_budget_exhausted());
        }
    }
}

#[test]
fn a_partial_application_cannot_supply_a_later_structural_column() {
    let ctx = context();
    for name in ["Nat_add", "Nat_mul"] {
        take_budget_exhausted();
        let reduced = whnf(&ctx, &app(g(name), g("Zero")));
        let (head, args) = spine_ref(&reduced);
        assert!(matches!(head, Db::Fix(_)));
        assert_eq!(args.len(), 1);
        assert_eq!(args[0], &g("Zero"));
        assert!(!take_budget_exhausted());
    }
}

#[test]
fn constructor_in_the_recursive_second_column_reduces_with_a_neutral_first_column() {
    let ctx = context();
    take_budget_exhausted();
    assert_eq!(
        whnf_in(&ctx, &[g("Nat")], &binary("Nat_add", Db::Var(0), g("Zero"))),
        Db::Var(0)
    );
    assert_eq!(
        whnf_in(&ctx, &[g("Nat")], &binary("Nat_mul", Db::Var(0), g("Zero"))),
        g("Zero")
    );
    assert!(def_eq(
        &ctx,
        &[g("Nat")],
        &binary("Nat_add", Db::Var(0), app(g("Succ"), g("Zero"))),
        &app(g("Succ"), Db::Var(0))
    ));
    assert!(!take_budget_exhausted());
}
