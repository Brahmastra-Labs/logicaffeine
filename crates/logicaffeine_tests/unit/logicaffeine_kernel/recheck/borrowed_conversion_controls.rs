use super::*;

fn sort(n: u32) -> Db {
    Db::Sort(Universe::Type(n))
}
fn app(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
fn identity_arg(value: Db) -> Db {
    Db::Let(
        DbBox::new(sort(1)),
        DbBox::new(value),
        DbBox::new(Db::Var(0)),
    )
}
fn context() -> Context {
    let mut ctx = Context::new();
    let ty = Db::Pi(DbBox::new(sort(1)), DbBox::new(sort(1)));
    let mut body = Db::Lam(DbBox::new(sort(1)), DbBox::new(Db::Var(0)));
    for _ in 0..40 {
        body = Db::Let(DbBox::new(sort(1)), DbBox::new(sort(0)), DbBox::new(body));
    }
    let body_term = from_db(&body, 0);
    let ty_term = from_db(&ty, 0);
    ctx.add_definition("DelayedIdentity".into(), ty_term, body_term);
    assert!(ctx.get_definition_body("DelayedIdentity").is_some());
    ctx
}
fn delayed(value: Db) -> Db {
    app(Db::Global("DelayedIdentity".into()), value)
}
fn measured(f: impl FnOnce() -> bool) -> (bool, (usize, usize)) {
    BETA_WORK.with(|w| w.set(Some((0, 0))));
    let answer = f();
    let work = BETA_WORK.with(|w| w.replace(None).unwrap());
    (answer, work)
}

#[test]
fn equal_application_heads_do_not_repeat_their_delayed_computation() {
    let ctx = context();
    let a = delayed(sort(0));
    let b = delayed(identity_arg(sort(0)));
    assert!(infer(&ctx, &[], &a).is_ok());
    assert!(infer(&ctx, &[], &b).is_ok());
    for (left, right) in [(&a, &b), (&b, &a)] {
        let (answer, work) = measured(|| def_eq(&ctx, &[], left, right));
        assert!(answer);
        assert!(
            work.0 <= 32 && work.1 <= 32,
            "equivalent small arguments must not normalize their common function: {work:?}"
        );
    }
}

#[test]
fn dependent_pi_congruence_keeps_outer_indices_and_avoids_common_delta_work() {
    let ctx = context();
    let a = Db::Pi(DbBox::new(sort(1)), DbBox::new(delayed(Db::Var(0))));
    let b = Db::Pi(
        DbBox::new(sort(1)),
        DbBox::new(delayed(identity_arg(Db::Var(0)))),
    );
    assert!(infer_sort(&ctx, &[], &a).is_ok());
    assert!(infer_sort(&ctx, &[], &b).is_ok());
    let (answer, work) = measured(|| is_sub(&ctx, &[], &a, &b));
    assert!(answer);
    assert!(
        work.0 <= 32 && work.1 <= 32,
        "already matching Pi heads require only their small argument conversion: {work:?}"
    );
}

#[test]
fn beta_under_a_binder_does_not_capture_the_outer_argument() {
    let ctx = Context::new();
    let left = app(
        Db::Lam(
            DbBox::new(sort(1)),
            DbBox::new(Db::Lam(DbBox::new(sort(1)), DbBox::new(Db::Var(1)))),
        ),
        Db::Var(0),
    );
    let right = Db::Lam(DbBox::new(sort(1)), DbBox::new(Db::Var(1)));
    let captured = Db::Lam(DbBox::new(sort(1)), DbBox::new(Db::Var(0)));
    assert!(infer(&ctx, &[sort(1)], &left).is_ok());
    assert!(def_eq(&ctx, &[sort(1)], &left, &right));
    assert!(!def_eq(&ctx, &[sort(1)], &left, &captured));
}

#[test]
fn conversion_keeps_universes_exact_and_function_domains_invariant() {
    let ctx = Context::new();
    assert!(!def_eq(&ctx, &[], &sort(0), &sort(1)));
    assert!(is_sub(&ctx, &[], &sort(0), &sort(1)));
    assert!(!is_sub(&ctx, &[], &sort(1), &sort(0)));
    let a = Db::Pi(DbBox::new(sort(0)), DbBox::new(sort(0)));
    let b = Db::Pi(DbBox::new(sort(1)), DbBox::new(sort(0)));
    assert!(!is_sub(&ctx, &[], &a, &b));
    assert!(!is_sub(&ctx, &[], &b, &a));
}

#[test]
fn unsuccessful_congruence_still_uses_full_conversion() {
    let ctx = Context::new();
    let erases = Db::Lam(DbBox::new(sort(2)), DbBox::new(sort(0)));
    let a = app(erases.clone(), sort(0));
    let b = app(erases, sort(1));
    assert!(infer(&ctx, &[], &a).is_ok());
    assert!(infer(&ctx, &[], &b).is_ok());
    assert!(def_eq(&ctx, &[], &a, &b));
    assert!(def_eq(&ctx, &[], &b, &a));
}

#[test]
fn congruence_cannot_skip_argument_or_let_annotation_checking() {
    let ctx = context();
    let wrong_argument = delayed(sort(2));
    let wrong_let = Db::Let(
        DbBox::new(sort(0)),
        DbBox::new(sort(0)),
        DbBox::new(Db::Var(0)),
    );
    assert!(infer(&ctx, &[], &wrong_argument).is_err());
    assert!(infer(&ctx, &[], &delayed(wrong_let)).is_err());
}
