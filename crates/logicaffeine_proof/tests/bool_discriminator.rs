//! Bool no-confusion: `false ≠ true`. This is the primitive that turns a
//! linearly-derived ground-FALSE inequality (`le(5,3) = true`, where `le 5 3 ⇝
//! false`, i.e. `Eq Bool false true`) into `False` — so contradictory linear bounds
//! prove anything (ex falso). Built as `Eq_rec Bool false P I true h` with the
//! discriminating motive `P b = match b with true ⇒ False | false ⇒ True`.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, normalize, Context, Term, Universe};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, xs: Vec<Term>) -> Term {
    xs.into_iter()
        .fold(f, |acc, x| Term::App(Box::new(acc), Box::new(x)))
}
fn eq_bool(a: Term, b: Term) -> Term {
    app(g("Eq"), vec![g("Bool"), a, b])
}

/// `P b = match b return Prop with | true ⇒ False | false ⇒ True`  (cases in Bool's
/// definition order: true, then false).
fn discriminating_motive() -> Term {
    Term::Lambda {
        param: "b".to_string(),
        param_type: Box::new(g("Bool")),
        body: Box::new(Term::Match {
            discriminant: Box::new(Term::Var("b".to_string())),
            motive: Box::new(Term::Lambda {
                param: "_".to_string(),
                param_type: Box::new(g("Bool")),
                body: Box::new(Term::Sort(Universe::Prop)),
            }),
            cases: vec![g("False"), g("True")],
        }),
    }
}

#[test]
fn false_eq_true_discriminates_to_false() {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    // h : Eq Bool false true  (only inhabited from a contradiction; here a hypothesis)
    c.add_declaration("h", eq_bool(g("false"), g("true")));

    // Eq_rec Bool false P I true h : False
    let cert = app(
        g("Eq_rec"),
        vec![g("Bool"), g("false"), discriminating_motive(), g("I"), g("true"), g("h")],
    );
    let ty = normalize(&c, &infer_type(&c, &cert).expect("the discriminator must type-check"));
    assert_eq!(ty, g("False"), "false = true must yield False, got {}", ty);
}

#[test]
fn discriminator_accepts_a_ground_false_le_proof() {
    // A proof of `le(5,3) = true` is definitionally `Eq Bool false true` (le 5 3 ⇝
    // false), so the SAME discriminator turns it into False.
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    let le_5_3 = app(g("le"), vec![Term::Lit(logicaffeine_kernel::Literal::Int(5)), Term::Lit(logicaffeine_kernel::Literal::Int(3))]);
    // hle : Eq Bool (le 5 3) true
    c.add_declaration("hle", eq_bool(le_5_3, g("true")));
    let cert = app(
        g("Eq_rec"),
        vec![g("Bool"), g("false"), discriminating_motive(), g("I"), g("true"), g("hle")],
    );
    let ty = normalize(&c, &infer_type(&c, &cert).expect("a ground-false `le` proof must discriminate"));
    assert_eq!(ty, g("False"), "le(5,3)=true must yield False, got {}", ty);
}
