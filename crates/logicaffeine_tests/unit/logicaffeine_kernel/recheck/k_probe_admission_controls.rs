use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn context() -> Context {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    for (name, constructors) in [
        ("Flag", vec!["on", "off"]),
        ("Unit", vec!["star"]),
        ("Empty", vec![]),
    ] {
        c.add_mutual_inductives(&[crate::context::MutualInductive {
            name: name.into(),
            sort: Term::Sort(Universe::Type(0)),
            num_params: 0,
            constructors: constructors
                .into_iter()
                .map(|n| (n.into(), Term::Global(name.into())))
                .collect(),
        }])
        .unwrap();
    }
    c
}
fn measured(c: &Context, locals: &dyn LocalTypes, t: &Db) -> (Db, usize) {
    K_PROBE_ATTEMPTS.with(|n| n.set(Some(0)));
    let result = whnf_in(c, locals, t);
    let attempts = K_PROBE_ATTEMPTS.with(|n| n.replace(None).unwrap());
    (result, attempts)
}
fn neutral(cases: Vec<Db>) -> Db {
    Db::Match {
        disc: DbBox::new(Db::Var(0)),
        motive: DbBox::new(g("Flag")),
        cases,
    }
}
#[test]
fn multi_constructor_neutral_match_does_not_attempt_singleton_inference() {
    let c = context();
    let term = neutral(vec![g("on"), g("off")]);
    assert_eq!(infer(&c, &[g("Flag")], &term).unwrap(), g("Flag"));
    let (result, attempts) = measured(&c, &[g("Flag")], &term);
    assert_eq!(result, term);
    assert_eq!(
        attempts, 0,
        "multiple branches exclude singleton K reduction"
    );
}
#[test]
fn empty_elimination_does_not_attempt_to_reconstruct_a_constructor() {
    let c = context();
    let term = neutral(vec![]);
    assert_eq!(infer(&c, &[g("Empty")], &term).unwrap(), g("Flag"));
    let (result, attempts) = measured(&c, &[g("Empty")], &term);
    assert_eq!(result, term);
    assert_eq!(
        attempts, 0,
        "an empty branch inventory cannot select a K branch"
    );
}
#[test]
fn a_fieldless_singleton_still_reduces_its_neutral_eliminator() {
    let c = context();
    let term = neutral(vec![g("on")]);
    assert_eq!(infer(&c, &[g("Unit")], &term).unwrap(), g("Flag"));
    let (result, attempts) = measured(&c, &[g("Unit")], &term);
    assert_eq!(result, g("on"));
    assert_eq!(attempts, 1);
}
#[test]
fn probe_admission_does_not_waive_coverage_or_branch_typing() {
    let c = context();
    for cases in [
        vec![],
        vec![g("on"), g("off")],
        vec![Db::Sort(Universe::Type(0))],
    ] {
        assert!(infer(&c, &[g("Unit")], &neutral(cases)).is_err());
    }
    assert!(infer(&c, &[g("Flag")], &neutral(vec![g("on")])).is_err());
}
