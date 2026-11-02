use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn app(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", Term::Global("Nat".into()));
    c.add_constructor(
        "Succ",
        "Nat",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body_type: Box::new(Term::Global("Nat".into())),
        },
    );
    c
}
fn select(disc: Db, zero: Db) -> Db {
    Db::Match {
        disc: DbBox::new(disc),
        motive: DbBox::new(g("Nat")),
        cases: vec![zero, Db::Lam(DbBox::new(g("Nat")), DbBox::new(g("Zero")))],
    }
}
fn nested_matches(depth: usize) -> Db {
    (0..depth).fold(g("Zero"), |d, _| select(d, g("Zero")))
}
struct Restore(usize, usize, bool);
impl Drop for Restore {
    fn drop(&mut self) {
        WHNF_DEPTH.with(|v| v.set(self.0));
        WHNF_FUEL.with(|v| v.set(self.1));
        WHNF_EXHAUSTED.with(|v| v.set(self.2));
    }
}
fn state() -> Restore {
    let old = Restore(
        WHNF_DEPTH.with(|v| v.get()),
        WHNF_FUEL.with(|v| v.get()),
        WHNF_EXHAUSTED.with(|v| v.get()),
    );
    WHNF_EXHAUSTED.with(|v| v.set(false));
    old
}
// f x n := match n with Zero => x | Succ k => f x k.
// The checked structural column is the SECOND ordinary argument.
fn second_column() -> Db {
    Db::Fix(FixBody::new(Db::Lam(
        DbBox::new(g("Nat")),
        DbBox::new(Db::Lam(
            DbBox::new(g("Nat")),
            DbBox::new(Db::Match {
                disc: DbBox::new(Db::Var(0)),
                motive: DbBox::new(g("Nat")),
                cases: vec![
                    Db::Var(1),
                    Db::Lam(
                        DbBox::new(g("Nat")),
                        DbBox::new(app(app(Db::Var(3), Db::Var(2)), Db::Var(0))),
                    ),
                ],
            }),
        )),
    )))
}
#[test]
fn deeply_computed_discriminants_use_bounded_native_stack() {
    let _restore = state();
    let result = whnf_in_owned(&context(), &[], nested_matches(300));
    assert_eq!(result, g("Zero"));
    assert!(
        !WHNF_EXHAUSTED.with(|v| v.get()),
        "300 legal logical demands fit the unchanged 512-depth limit"
    );
}
#[test]
fn deeply_neutral_application_spines_return_the_complete_value() {
    let _restore = state();
    let value = (0..300).fold(g("neutral"), |f, _| app(f, g("Zero")));
    let expected = value.clone();
    let result = whnf_in_owned(&context(), &[], value);
    assert_eq!(result, expected);
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
}
#[test]
fn pending_demands_still_enforce_the_original_logical_depth_limit() {
    let _restore = state();
    WHNF_DEPTH.with(|v| v.set(MAX_RECHECK_WHNF_DEPTH - 1));
    WHNF_FUEL.with(|v| v.set(100));
    let value = select(g("Zero"), g("Zero"));
    let result = whnf_in_owned(&context(), &[], value.clone());
    assert_eq!(result, value);
    assert!(WHNF_EXHAUSTED.with(|v| v.get()));
    assert_eq!(WHNF_DEPTH.with(|v| v.get()), MAX_RECHECK_WHNF_DEPTH - 1);
    assert_eq!(WHNF_FUEL.with(|v| v.get()), 99);
}
#[test]
fn child_fuel_exhaustion_reconstructs_every_outer_match_and_application() {
    let _restore = state();
    WHNF_DEPTH.with(|v| v.set(1));
    WHNF_FUEL.with(|v| v.set(2));
    let value = app(select(select(g("Zero"), g("Zero")), g("Zero")), Db::Var(0));
    let result = whnf_in_owned(&context(), &[g("Nat")], value.clone());
    assert_eq!(
        result, value,
        "a child term alone is not a safe outer result"
    );
    assert!(WHNF_EXHAUSTED.with(|v| v.get()));
    assert_eq!(WHNF_FUEL.with(|v| v.get()), 0);
    assert_eq!(WHNF_DEPTH.with(|v| v.get()), 1);
}
#[test]
fn suspended_matches_preserve_outer_de_bruijn_variables() {
    let _restore = state();
    let branch = Db::Lam(DbBox::new(g("Nat")), DbBox::new(Db::Var(1)));
    let value = select(nested_matches(100), branch.clone());
    assert_eq!(whnf_in_owned(&context(), &[g("Nat")], value), branch);
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
}
#[test]
fn fix_demands_only_its_checked_column_and_preserves_partial_applications() {
    let _restore = state();
    let c = context();
    let f = second_column();
    for value in [
        app(f.clone(), g("Zero")),
        app(app(f.clone(), g("Zero")), Db::Var(0)),
    ] {
        assert_eq!(whnf_in_owned(&c, &[g("Nat")], value.clone()), value);
    }
    let computed = select(app(g("Succ"), g("Zero")), g("Zero"));
    let value = app(app(f, Db::Var(0)), computed);
    assert_eq!(whnf_in_owned(&c, &[g("Nat")], value), Db::Var(0));
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
}
