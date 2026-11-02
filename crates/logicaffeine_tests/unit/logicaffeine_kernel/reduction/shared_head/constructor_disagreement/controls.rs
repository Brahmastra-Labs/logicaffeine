use super::*;
use crate::{BigInt, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn succ(t: Term) -> Term {
    app(g("Succ"), t)
}
fn env() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
const FUEL: u64 = 1 << 20;

#[test]
fn different_constructors_and_different_naturals_disagree() {
    let c = env();
    assert!(constructors_disagree(&c, &g("true"), &g("false"), FUEL));
    assert!(constructors_disagree(&c, &lit(3), &lit(4), FUEL));
    assert!(constructors_disagree(&c, &lit(0), &succ(g("Zero")), FUEL));
    assert!(constructors_disagree(
        &c,
        &succ(succ(lit(1))),
        &lit(4),
        FUEL
    ));
}

#[test]
fn a_literal_and_its_constructor_spelling_never_disagree() {
    let c = env();
    for (a, b) in [
        (lit(0), g("Zero")),
        (lit(3), succ(lit(2))),
        (lit(3), succ(succ(succ(g("Zero"))))),
        (g("true"), g("true")),
        (lit(7), lit(7)),
    ] {
        assert!(
            !constructors_disagree(&c, &a, &b, FUEL),
            "{a} and {b} are equal"
        );
        assert!(
            !constructors_disagree(&c, &b, &a, FUEL),
            "{b} and {a} are equal"
        );
    }
}

#[test]
fn a_computation_is_observed_through_its_weak_head_value() {
    let c = env();
    // `(λn. match n with | 0 => false | _+1 => true) 5` is `true`.
    let positive = lam(
        "n",
        g("Nat"),
        Term::Match {
            discriminant: Box::new(v("n")),
            motive: Box::new(lam("_", g("Nat"), g("Bool"))),
            cases: vec![g("false"), lam("p", g("Nat"), g("true"))],
        },
    );
    assert!(constructors_disagree(
        &c,
        &app(positive.clone(), lit(5)),
        &g("false"),
        FUEL
    ));
    assert!(!constructors_disagree(
        &c,
        &app(positive, lit(5)),
        &g("true"),
        FUEL
    ));
}

#[test]
fn a_proposition_family_is_never_refuted() {
    let mut c = env();
    // Two constructors of a `Prop`: equal proofs by proof irrelevance.
    c.add_inductive("TwoProofs", Term::Sort(Universe::Prop));
    c.add_constructor("TwoProofs.left", "TwoProofs", g("TwoProofs"));
    c.add_constructor("TwoProofs.right", "TwoProofs", g("TwoProofs"));
    assert!(!constructors_disagree(
        &c,
        &g("TwoProofs.left"),
        &g("TwoProofs.right"),
        FUEL
    ));
    // A family whose sort is a bare universe variable may be a proposition at level 0.
    c.add_inductive("MaybeProp", Term::Sort(Universe::Var("u".into())));
    c.add_constructor("MaybeProp.a", "MaybeProp", g("MaybeProp"));
    c.add_constructor("MaybeProp.b", "MaybeProp", g("MaybeProp"));
    assert!(!constructors_disagree(
        &c,
        &g("MaybeProp.a"),
        &g("MaybeProp.b"),
        FUEL
    ));
}

#[test]
fn open_terms_holes_and_an_exhausted_allowance_give_no_answer() {
    let c = env().extend("x", g("Bool"));
    assert!(
        !constructors_disagree(&c, &v("x"), &g("false"), FUEL),
        "an open term is never refuted"
    );
    assert!(!constructors_disagree(&c, &Term::Hole, &g("false"), FUEL));
    assert!(!constructors_disagree(&env(), &g("true"), &g("false"), 0));
}

/// The legacy raw API can file a constructor under an inductive its type does not return: the
/// registry's claim is not refuted on, the constructor's actual type is (two inhabitants of the
/// unit type `ActualUnit` are equal by unit η).
#[test]
fn a_constructor_filed_under_another_family_is_never_refuted() {
    let mut c = env();
    c.add_inductive("ActualUnit", Term::Sort(Universe::Type(0)));
    c.add_constructor("actual_unit", "ActualUnit", g("ActualUnit"));
    c.add_inductive("ClaimedData", Term::Sort(Universe::Type(0)));
    c.add_constructor("raw_unit_a", "ClaimedData", g("ActualUnit"));
    c.add_constructor("raw_unit_b", "ClaimedData", g("ActualUnit"));
    assert!(!constructors_disagree(
        &c,
        &g("raw_unit_a"),
        &g("raw_unit_b"),
        FUEL
    ));
    assert!(
        crate::defeq_for_test(&c, &g("raw_unit_a"), &g("raw_unit_b")),
        "unit η"
    );
    c.add_constructor("claimed_a", "ClaimedData", g("ClaimedData"));
    c.add_constructor("claimed_b", "ClaimedData", g("ClaimedData"));
    assert!(
        constructors_disagree(&c, &g("claimed_a"), &g("claimed_b"), FUEL),
        "constructors whose type is their family are still refuted"
    );
}

#[test]
fn equal_constructors_descend_into_their_fields() {
    let c = env();
    // `Succ (Succ 3)` against `Succ (Succ 4)`: the disagreement is two fields down.
    assert!(constructors_disagree(
        &c,
        &succ(succ(lit(3))),
        &succ(succ(lit(4))),
        FUEL
    ));
    assert!(!constructors_disagree(
        &c,
        &succ(succ(lit(3))),
        &succ(succ(lit(3))),
        FUEL
    ));
}

fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}

/// `Truth : Prop` with two proofs, and `Tagged : Type` whose constructor carries a natural and a
/// proof of `Truth` — Lean's `Fin.mk val isLt` in miniature.
fn with_a_proof_field() -> Context {
    let mut c = env();
    c.add_inductive("Truth", Term::Sort(Universe::Prop));
    c.add_constructor("Truth.left", "Truth", g("Truth"));
    c.add_constructor("Truth.right", "Truth", g("Truth"));
    c.add_inductive("Tagged", Term::Sort(Universe::Type(0)));
    c.add_constructor(
        "Tagged.mk",
        "Tagged",
        arrow(g("Nat"), arrow(g("Truth"), g("Tagged"))),
    );
    c
}
fn tagged(n: u64, proof: &str) -> Term {
    app(app(g("Tagged.mk"), lit(n)), g(proof))
}

/// The evaluator never observes a proof field, and the last field is compared first: an
/// unobservable proof pair is passed over and the data field beside it still refutes.
#[test]
fn a_data_field_beside_an_unobservable_proof_field_still_disagrees() {
    let c = with_a_proof_field();
    assert!(constructors_disagree(
        &c,
        &tagged(4, "Truth.left"),
        &tagged(0, "Truth.left"),
        FUEL
    ));
    assert!(constructors_disagree(
        &c,
        &tagged(4, "Truth.left"),
        &tagged(0, "Truth.right"),
        FUEL
    ));
    assert!(!crate::defeq_for_test(
        &c,
        &tagged(4, "Truth.left"),
        &tagged(0, "Truth.left")
    ));
}

/// Passing over a proof pair is never evidence: equal data under different proofs is equal by
/// proof irrelevance, and is never refuted.
#[test]
fn a_proof_field_alone_never_disagrees() {
    let c = with_a_proof_field();
    assert!(!constructors_disagree(
        &c,
        &tagged(4, "Truth.left"),
        &tagged(4, "Truth.right"),
        FUEL
    ));
    assert!(!constructors_disagree(
        &c,
        &tagged(4, "Truth.left"),
        &tagged(4, "Truth.left"),
        FUEL
    ));
    assert!(
        crate::defeq_for_test(&c, &tagged(4, "Truth.left"), &tagged(4, "Truth.right")),
        "proof irrelevance"
    );
}
