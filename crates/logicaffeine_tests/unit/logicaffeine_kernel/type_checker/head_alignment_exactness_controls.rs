use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn constant() -> Term {
    Term::Lambda {
        param: "unused".into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(g("Zero")),
    }
}
#[test]
fn opportunistic_head_alignment_does_not_certify_erased_holes() {
    let c = Context::new();
    assert!(!conversion_by_unfolded_head(
        &c,
        &app(constant(), Term::Hole),
        &g("Zero")
    ));
}
