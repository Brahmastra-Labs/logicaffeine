use super::*;
use crate::{Literal, Reducibility, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(f: Term, args: Vec<Term>) -> Term {
    args.into_iter().fold(f, app)
}
fn n(v: u64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_u64(v)))
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
#[test]
fn existing_recurrence_evaluator_decides_large_power_without_expanding_peano() {
    let c = context();
    let power = apps(g("Nat_pow"), vec![n(2), n(32)]);
    let less = apps(g("natLt"), vec![n(0), power]);
    let p = apps(g("Eq"), vec![g("Bool"), less.clone(), g("true")]);
    let d = apps(
        g("decide"),
        vec![p, apps(g("decEqBool"), vec![less, g("true")])],
    );
    assert!(equivalent(&c, &d, &g("true")));
    assert!(!equivalent(&c, &d, &g("false")));
}
#[test]
fn compact_resource_budget_and_single_attempt_are_explicit() {
    let c = context();
    let d = apps(g("Nat_pow"), vec![n(2), n(8)]);
    let mut used = false;
    assert!(!compact_data(&c, &d, &g("Zero"), &mut used, 0));
    assert!(!used);
    used = true;
    assert!(!compact_data(&c, &d, &g("Zero"), &mut used, 1 << 18));
    let lambda = Term::Lambda {
        param: "x".into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(Term::Var("x".into())),
    };
    used = false;
    assert!(!compact_data(&c, &lambda, &lambda, &mut used, 1 << 18));
    assert!(!used);
}
#[test]
fn unknown_proof_field_and_free_capture_cannot_create_constructor_evidence() {
    let c = context();
    let p = g("False");
    let fake = apps(g("isTrue"), vec![p.clone(), Term::Hole]);
    let d = apps(g("decide"), vec![p, fake]);
    assert!(!equivalent(&c, &d, &g("true")));
    let free = app(
        Term::Lambda {
            param: "x".into(),
            param_type: Box::new(g("Bool")),
            body: Box::new(Term::Var("free".into())),
        },
        g("true"),
    );
    assert!(!equivalent(&c, &free, &g("true")));
}
#[test]
fn context_scope_restores_exact_transparency_eligibility() {
    let mut c = context();
    assert!(c.elaborator_all_transparent());
    c.push_scope();
    c.set_reducibility("local.name", Reducibility::Irreducible);
    assert!(!c.elaborator_all_transparent());
    c.pop_scope();
    assert!(c.elaborator_all_transparent());
    c.set_reducibility("only.reducible", Reducibility::Reducible);
    assert!(c.elaborator_all_transparent());
}

#[test]
fn nullary_proof_constructor_is_not_a_compact_data_endpoint() {
    let c = context();
    let proof = g("I");
    assert!(c.is_constructor("I"));
    assert_eq!(c.get_global("I"), Some(&g("True")));
    assert_eq!(c.get_global("True"), Some(&Term::Sort(Universe::Prop)));
    let mut used = false;
    assert!(!compact_data(&c, &proof, &proof, &mut used, 1 << 18));
    assert!(
        !used,
        "a Prop constructor must not consume a data-evaluation attempt"
    );
}
