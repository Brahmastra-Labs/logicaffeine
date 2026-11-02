//! In first-order logic a predicate and an individual may share a spelling: "the butler" reaches
//! the prover as the constant `butler`, and `butler` is also the noun. The kernel has one
//! namespace, so it files such a predicate under a name of its own; a proof that uses both — the
//! noun defined, used as a relation, or presupposed — certifies as it would with distinct names.

use logicaffeine_proof::verify::{prove_certify_check, prove_certify_check_with_defs, Definition};
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn predicate(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate { name: name.into(), args, world: None }
}

fn constant(name: &str) -> ProofTerm {
    ProofTerm::Constant(name.into())
}

fn x() -> ProofTerm {
    ProofTerm::Variable("x".into())
}

fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}

fn every_x(body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: "x".into(), body: Box::new(body) }
}

/// "A butler is one who serves. Every butler is loyal. The butler serves." — so the butler is
/// loyal. The definiendum `butler` and the butler it names are two symbols to the kernel.
#[test]
fn a_defined_noun_and_the_individual_it_names_certify_together() {
    let butler = Definition {
        name: "butler".into(),
        params: vec!["x".into()],
        definiens: predicate("serves", vec![constant("x")]),
    };
    let premises = [
        every_x(implies(predicate("butler", vec![x()]), predicate("loyal", vec![x()]))),
        predicate("serves", vec![constant("butler")]),
    ];
    let proof = prove_certify_check_with_defs(&premises, &predicate("loyal", vec![constant("butler")]), &[butler]);
    assert!(proof.verified, "{:?}", proof.verification_error);
}

/// "The butler is Wooster's butler. Whoever is Wooster's butler is trusted." — so the butler is
/// trusted. The relation `butler(x, y)` and the individual `butler` are two symbols.
#[test]
fn a_relational_noun_and_the_individual_it_names_certify_together() {
    let premises = [
        predicate("butler", vec![constant("butler"), constant("Wooster")]),
        every_x(implies(
            predicate("butler", vec![x(), constant("Wooster")]),
            predicate("trusted", vec![x()]),
        )),
    ];
    let proof = prove_certify_check(&premises, &predicate("trusted", vec![constant("butler")]));
    assert!(proof.verified, "{:?}", proof.verification_error);
}

/// The kernel's name for the noun is its own business: the derivation the prover returns still
/// speaks of `butler`, as the premises do.
#[test]
fn the_derivation_keeps_the_premises_names() {
    let premises = [every_x(implies(predicate("butler", vec![x()]), predicate("loyal", vec![x()])))];
    let proof = prove_certify_check(&premises, &predicate("loyal", vec![constant("butler")]));
    assert!(proof.verified, "{:?}", proof.verification_error);
    let derivation = proof.derivation.expect("a verified proof carries its derivation");
    let shown = derivation.display_tree();
    assert!(shown.contains("butler(butler)"), "the derivation lost the premises' names: {shown}");
    assert!(!shown.contains("is_butler"), "the kernel's name leaked into the derivation: {shown}");
}
