use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn lam(body: Term) -> Term {
    Term::Lambda {
        param: "x".into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(body),
    }
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
#[test]
fn unapplied_lambda_is_not_its_body_head() {
    let mut budget = 256;
    assert!(!delta_exposes_head(
        &app(lam(lam(g("target"))), g("arg")),
        &g("target"),
        &mut budget
    ));
}
#[test]
fn variable_head_does_not_guess_substituted_function() {
    let mut budget = 256;
    assert!(!delta_exposes_head(
        &app(lam(Term::Var("x".into())), g("target")),
        &g("target"),
        &mut budget
    ));
}
#[test]
fn constant_universes_must_match() {
    let c = |u| Term::Const {
        name: "target".into(),
        levels: vec![Universe::Type(u)],
    };
    let mut budget = 256;
    assert!(!delta_exposes_head(
        &app(lam(c(0)), g("arg")),
        &c(1),
        &mut budget
    ));
}
#[test]
fn prefix_observation_spends_the_same_fuel() {
    let candidate = app(lam(g("target")), g("arg"));
    let mut budget = 3;
    assert!(!delta_exposes_head(&candidate, &g("target"), &mut budget));
    assert_eq!(budget, 0);
    let mut budget = 4;
    assert!(delta_exposes_head(&candidate, &g("target"), &mut budget));
    assert_eq!(budget, 0);
}
