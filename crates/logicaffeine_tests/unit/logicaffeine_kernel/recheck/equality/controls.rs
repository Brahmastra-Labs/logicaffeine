use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn app(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
fn lam(b: Db) -> Db {
    Db::Lam(DbBox::new(g("Nat")), DbBox::new(b))
}
fn pi(b: Db) -> Db {
    Db::Pi(DbBox::new(g("Nat")), DbBox::new(b))
}
fn lit(n: u64) -> Db {
    Db::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}
fn call(a: Db, b: Db) -> Db {
    app(app(g("independent.compare"), a), b)
}
fn mat(d: Db, a: Db, b: Db) -> Db {
    Db::Match {
        disc: DbBox::new(d),
        motive: DbBox::new(lam(g("Choice"))),
        cases: vec![a, b],
    }
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", Term::Global("Nat".into()));
    c.add_constructor("Succ", "Nat", from_db_with_external(&pi(g("Nat")), 0, &[]));
    c.add_inductive("Choice", Term::Sort(Universe::Type(0)));
    c.add_constructor("Choice.yes", "Choice", Term::Global("Choice".into()));
    c.add_constructor("Choice.no", "Choice", Term::Global("Choice".into()));
    let recursive = app(app(Db::Var(4), Db::Var(1)), Db::Var(0));
    let body = Db::Fix(FixBody::new(lam(lam(mat(
        Db::Var(1),
        mat(Db::Var(0), g("Choice.yes"), lam(g("Choice.no"))),
        lam(mat(Db::Var(1), g("Choice.no"), lam(recursive))),
    )))));
    c.declare(
        "independent.compare",
        from_db_with_external(&pi(pi(g("Choice"))), 0, &[]),
        from_db_with_external(&body, 0, &[]),
    )
    .unwrap();
    c
}
struct Restore(usize, usize, bool);
impl Drop for Restore {
    fn drop(&mut self) {
        WHNF_DEPTH.with(|x| x.set(self.0));
        WHNF_FUEL.with(|x| x.set(self.1));
        WHNF_EXHAUSTED.with(|x| x.set(self.2));
    }
}
fn observe(c: &Context, t: &Db, fuel: usize) -> Db {
    let _restore = Restore(
        WHNF_DEPTH.with(|x| x.replace(1)),
        WHNF_FUEL.with(|x| x.replace(fuel)),
        WHNF_EXHAUSTED.with(|x| x.replace(false)),
    );
    let _scope = arithmetic::ConversionScope::enter();
    whnf_in(c, &[], t)
}
#[test]
fn independent_equality_finishes_with_a_bounded_budget() {
    let c = context();
    for (a, b, equal) in [
        (0, 0, true),
        (u64::MAX, u64::MAX, true),
        (u64::MAX, u64::MAX - 1, false),
        (0, u64::MAX, false),
        (u64::MAX, 0, false),
    ] {
        assert_eq!(
            observe(&c, &call(lit(a), lit(b)), 20_000),
            g(if equal { "Choice.yes" } else { "Choice.no" }),
            "{a}, {b}"
        );
    }
}
#[test]
fn independent_equality_observes_definition_mutation() {
    let c = context();
    let term = call(lit(u64::MAX), lit(u64::MAX));
    assert_eq!(observe(&c, &term, 20_000), g("Choice.yes"));
    let mut changed = c.clone();
    changed
        .declare(
            "independent.compare",
            from_db_with_external(&pi(pi(g("Choice"))), 0, &[]),
            from_db_with_external(&lam(lam(g("Choice.no"))), 0, &[]),
        )
        .unwrap();
    assert_eq!(observe(&changed, &term, 20_000), g("Choice.no"));
}
#[test]
fn independent_equality_needs_fuel_and_known_values() {
    let mut c = context();
    c.add_declaration("unknown", Term::Global("Nat".into()));
    let unknown = observe(&c, &call(g("unknown"), g("unknown")), 20_000);
    assert_ne!(unknown, g("Choice.yes"));
    assert_ne!(unknown, g("Choice.no"));
    assert_ne!(
        observe(&c, &call(lit(u64::MAX), lit(u64::MAX)), 0),
        g("Choice.yes")
    );
}

#[test]
fn independent_certificate_rejects_correct_bases_with_a_false_step() {
    let mut c = context();
    let body = Db::Fix(FixBody::new(lam(lam(mat(
        Db::Var(1),
        mat(Db::Var(0), g("Choice.yes"), lam(g("Choice.no"))),
        lam(mat(Db::Var(1), g("Choice.no"), lam(g("Choice.yes")))),
    )))));
    c.declare(
        "independent.compare",
        from_db_with_external(&pi(pi(g("Choice"))), 0, &[]),
        from_db_with_external(&body, 0, &[]),
    )
    .unwrap();
    let _restore = Restore(
        WHNF_DEPTH.with(|x| x.replace(1)),
        WHNF_FUEL.with(|x| x.replace(20_000)),
        WHNF_EXHAUSTED.with(|x| x.replace(false)),
    );
    assert!(reduce(&c, &[], &call(lit(u64::MAX), lit(u64::MAX - 1))).is_none());
}
#[test]
fn independent_certificate_refuses_pending_dependencies_and_negative_values() {
    let c = context();
    let _restore = Restore(
        WHNF_DEPTH.with(|x| x.replace(1)),
        WHNF_FUEL.with(|x| x.replace(20_000)),
        WHNF_EXHAUSTED.with(|x| x.replace(false)),
    );
    assert!(reduce(
        &c,
        &[],
        &call(Db::Lit(Literal::Nat(crate::BigInt::from_i64(-1))), lit(0))
    )
    .is_none());
    let mut pending = c.clone();
    pending.add_definition(
        "pending".into(),
        Term::Global("Nat".into()),
        Term::Global("missing".into()),
    );
    let body = Term::Let {
        name: "unused".into(),
        ty: Box::new(Term::Global("Nat".into())),
        value: Box::new(Term::Global("pending".into())),
        body: Box::new(
            c.get_definition_body("independent.compare")
                .unwrap()
                .clone(),
        ),
    };
    pending.add_definition(
        "independent.compare".into(),
        c.get_definition_type("independent.compare")
            .unwrap()
            .clone(),
        body,
    );
    assert!(reduce(&pending, &[], &call(lit(0), lit(0))).is_none());
}

fn direct(c: &Context, locals: &dyn LocalTypes, t: &Db, fuel: usize) -> Option<Db> {
    let _restore = Restore(
        WHNF_DEPTH.with(|x| x.replace(1)),
        WHNF_FUEL.with(|x| x.replace(fuel)),
        WHNF_EXHAUSTED.with(|x| x.replace(false)),
    );
    let _scope = arithmetic::ConversionScope::enter();
    reduce(c, locals, t)
}
#[test]
fn equality_cache_reuses_independent_certificate_across_calls_and_local_scopes() {
    let c = context();
    CERTIFICATES.with(|n| n.set(0));
    assert_eq!(
        direct(&c, &[], &call(lit(9), lit(9)), 20_000),
        Some(g("Choice.yes"))
    );
    assert_eq!(
        direct(&c, &[], &call(lit(9), lit(8)), 20_000),
        Some(g("Choice.no"))
    );
    let local = c.extend("extra", Term::Global("Nat".into()));
    assert_eq!(
        direct(&local, &vec![g("Nat")], &call(lit(7), lit(7)), 20_000),
        Some(g("Choice.yes"))
    );
    assert_eq!(
        CERTIFICATES.with(|n| n.get()),
        1,
        "one independent attestation for one registry generation"
    );
}
#[test]
fn equality_cache_keeps_unknown_and_negative_arguments_inconclusive() {
    let c = context();
    CERTIFICATES.with(|n| n.set(0));
    assert_eq!(
        direct(&c, &[], &call(lit(1), lit(1)), 20_000),
        Some(g("Choice.yes"))
    );
    assert!(direct(&c, &vec![g("Nat")], &call(Db::Var(0), Db::Var(0)), 20_000).is_none());
    assert!(direct(
        &c,
        &[],
        &call(Db::Lit(Literal::Nat(crate::BigInt::from_i64(-1))), lit(0)),
        20_000
    )
    .is_none());
    assert_eq!(
        CERTIFICATES.with(|n| n.get()),
        1,
        "argument refusal must reuse the closed certificate"
    );
}
#[test]
fn equality_cache_cannot_turn_exhaustion_into_a_certificate() {
    let c = context();
    CERTIFICATES.with(|n| n.set(0));
    assert!(direct(&c, &[], &call(lit(1), lit(1)), 1).is_none());
    assert_eq!(
        direct(&c, &[], &call(lit(1), lit(1)), 20_000),
        Some(g("Choice.yes"))
    );
    assert!(direct(&c, &[], &call(lit(1), lit(1)), 0).is_none());
    assert_eq!(
        direct(&c, &[], &call(lit(1), lit(2)), 20_000),
        Some(g("Choice.no"))
    );
}
#[test]
fn equality_cache_revalidates_changed_bodies_and_preserves_the_original_context() {
    let c = context();
    assert_eq!(
        direct(&c, &[], &call(lit(4), lit(4)), 20_000),
        Some(g("Choice.yes"))
    );
    let mut changed = c.clone();
    changed
        .declare(
            "independent.compare",
            from_db_with_external(&pi(pi(g("Choice"))), 0, &[]),
            from_db_with_external(&lam(lam(g("Choice.no"))), 0, &[]),
        )
        .unwrap();
    for _ in 0..2 {
        assert!(direct(&changed, &[], &call(lit(4), lit(4)), 20_000).is_none());
    }
    assert_eq!(
        direct(&c, &[], &call(lit(4), lit(4)), 20_000),
        Some(g("Choice.yes"))
    );
}
#[test]
fn equality_cache_does_not_consume_a_main_kernel_certificate() {
    let c = context();
    c.cached_order_recurrence("independent.compare", || {
        Some(["Choice.yes".into(), "Choice.no".into()])
    })
    .unwrap();
    CERTIFICATES.with(|n| n.set(0));
    assert_eq!(
        direct(&c, &[], &call(lit(5), lit(5)), 20_000),
        Some(g("Choice.yes"))
    );
    assert_eq!(
        direct(&c, &[], &call(lit(5), lit(6)), 20_000),
        Some(g("Choice.no"))
    );
    assert_eq!(
        CERTIFICATES.with(|n| n.get()),
        1,
        "the independent kernel must establish its own equations"
    );
}

#[test]
fn equality_cache_warm_hits_preserve_depth_and_exhaustion_gates() {
    let c = context();
    let term = call(lit(2), lit(2));
    assert_eq!(direct(&c, &[], &term, 20_000), Some(g("Choice.yes")));
    for (depth, exhausted) in [(0, false), (MAX_RECHECK_WHNF_DEPTH, false), (1, true)] {
        let _restore = Restore(
            WHNF_DEPTH.with(|x| x.replace(depth)),
            WHNF_FUEL.with(|x| x.replace(20_000)),
            WHNF_EXHAUSTED.with(|x| x.replace(exhausted)),
        );
        assert!(reduce(&c, &[], &term).is_none());
        assert_eq!(WHNF_DEPTH.with(|x| x.get()), depth);
        assert_eq!(WHNF_FUEL.with(|x| x.get()), 20_000);
        assert_eq!(WHNF_EXHAUSTED.with(|x| x.get()), exhausted);
    }
}
