use super::*;

fn delayed_sort() -> Db {
    let mut body = Db::Sort(Universe::Type(0));
    for _ in 0..40 {
        body = Db::Let(
            DbBox::new(Db::Sort(Universe::Type(1))),
            DbBox::new(Db::Sort(Universe::Type(0))),
            DbBox::new(body),
        );
    }
    body
}

#[test]
fn reflexive_checked_types_need_no_beta_or_zeta_work() {
    let ctx = Context::new();
    let ty = delayed_sort();
    assert!(infer_sort(&ctx, &[], &ty).is_ok());
    BETA_WORK.with(|work| work.set(Some((0, 0))));
    let answer = is_sub(&ctx, &[], &ty, &ty);
    let work = BETA_WORK.with(|work| work.replace(None).unwrap());
    assert!(answer);
    assert_eq!(
        work,
        (0, 0),
        "reflexivity of an already checked type requires no reduction or copying"
    );
}

#[test]
fn distinct_types_still_observe_conversion_and_cumulativity() {
    let ctx = Context::new();
    let low = Db::Sort(Universe::Type(0));
    let high = Db::Sort(Universe::Type(1));
    assert!(is_sub(&ctx, &[], &delayed_sort(), &low));
    assert!(is_sub(&ctx, &[], &low, &high));
    assert!(!is_sub(&ctx, &[], &high, &low));
    assert!(!is_sub(
        &ctx,
        &[],
        &delayed_sort(),
        &Db::Sort(Universe::Prop)
    ));
    let left = Db::Pi(DbBox::new(low.clone()), DbBox::new(low.clone()));
    let right = Db::Pi(DbBox::new(high), DbBox::new(low));
    assert!(
        !is_sub(&ctx, &[], &left, &right),
        "function domains must remain invariant"
    );
}

#[test]
fn reflexive_conversion_does_not_skip_let_value_checking() {
    let ctx = Context::new();
    let malformed = Db::Let(
        DbBox::new(Db::Sort(Universe::Type(0))),
        DbBox::new(Db::Sort(Universe::Type(0))),
        DbBox::new(Db::Var(0)),
    );
    assert!(
        infer(&ctx, &[], &malformed).is_err(),
        "Type 0 has type Type 1 and cannot inhabit a Type 0 annotation"
    );
}
