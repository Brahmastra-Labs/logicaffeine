use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, normalize, Context, Literal, Term};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn name(n: &str) -> Term {
    app(g("SName"), Term::Lit(Literal::Text(n.into())))
}
fn var(n: i64) -> Term {
    app(g("SVar"), Term::Lit(Literal::Int(n)))
}
fn sa(f: Term, x: Term) -> Term {
    app(app(g("SApp"), f), x)
}
fn eq(a: Term, b: Term) -> Term {
    sa(sa(sa(name("Eq"), name("Nat")), a), b)
}
fn forall(body: Term) -> Term {
    sa(
        sa(name("Forall"), name("Nat")),
        app(app(g("SLam"), name("Nat")), body),
    )
}
fn implies(a: Term, b: Term) -> Term {
    sa(sa(name("implies"), a), b)
}
fn context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn proves(ctx: &Context, goal: &Term, steps: &[&str]) -> bool {
    let list = steps.iter().rev().fold(name("end"), |rest, step| {
        sa(sa(name("step"), name(step)), rest)
    });
    let proof = app(app(g("try_script"), list), goal.clone());
    infer_type(ctx, &proof).expect("the script is a well-typed derivation");
    normalize(ctx, &app(g("concludes"), proof)) == normalize(ctx, goal)
}

#[test]
fn universal_intro_proves_reflexivity_for_an_arbitrary_field() {
    assert!(proves(
        &context(),
        &forall(eq(var(0), var(0))),
        &["intro", "refl"]
    ));
}
#[test]
fn universal_intro_does_not_choose_a_witness() {
    assert!(!proves(
        &context(),
        &forall(eq(var(0), name("Zero"))),
        &["intro", "refl"]
    ));
}
#[test]
fn nested_universal_intros_keep_their_variables_distinct() {
    let ctx = context();
    assert!(proves(
        &ctx,
        &forall(forall(eq(var(1), var(1)))),
        &["intro", "intro", "refl"]
    ));
    assert!(!proves(
        &ctx,
        &forall(forall(eq(var(1), var(0)))),
        &["intro", "intro", "refl"]
    ));
}
#[test]
fn hypotheses_shift_when_a_universal_binder_is_opened() {
    let ctx = context();
    let hyp = eq(var(0), name("Zero"));
    let valid = forall(implies(hyp.clone(), forall(eq(var(1), name("Zero")))));
    let invalid = forall(implies(hyp, forall(eq(var(0), name("Zero")))));
    let steps = ["intro", "intro", "intro", "assumption"];
    assert!(proves(&ctx, &valid, &steps));
    assert!(!proves(&ctx, &invalid, &steps));
}
#[test]
fn universal_introduction_cannot_capture_a_free_variable() {
    let ctx = context();
    let goal = implies(eq(var(0), name("Zero")), forall(eq(var(0), name("Zero"))));
    assert!(!proves(&ctx, &goal, &["intro", "intro", "assumption"]));
}
#[test]
fn universal_introduction_refuses_an_unrepresentable_lift() {
    let ctx = context();
    let goal = implies(eq(var(i64::MAX), name("Zero")), forall(eq(var(0), var(0))));
    assert!(!proves(&ctx, &goal, &["intro", "intro", "refl"]));
}
