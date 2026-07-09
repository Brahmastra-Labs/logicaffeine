//! A tensed premise must not defeat an otherwise-propositional proof. "He was
//! seen" lowers to `Past(see(x))`; the certifier treats a temporal operator as
//! an opaque `Op : Prop → Prop` modality, so `Past(P)` is a distinct proposition
//! from `P` and a modus-tollens chain over tensed premises still kernel-certifies.

use logicaffeine_proof::verify::prove_certify_check;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn c(name: &str) -> ProofTerm {
    ProofTerm::Constant(name.to_string())
}
fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args, world: None }
}
fn past(p: ProofExpr) -> ProofExpr {
    ProofExpr::Temporal { operator: "Past".to_string(), body: Box::new(p) }
}
fn implies(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(a), Box::new(b))
}
fn not(a: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(a))
}

#[test]
fn modus_tollens_over_a_tensed_premise_certifies() {
    // do(Butler) → Past(see(Butler)), Past(see(Butler)) → caught(Butler), ¬caught(Butler) ⊢ ¬do(Butler)
    let did_it = pred("do", vec![c("Butler")]);
    let seen = past(pred("see", vec![c("Butler")]));
    let caught = pred("caught", vec![c("Butler")]);
    let premises = vec![
        implies(did_it.clone(), seen.clone()),
        implies(seen, caught.clone()),
        not(caught),
    ];
    let goal = not(did_it);

    let vp = prove_certify_check(&premises, &goal);
    assert!(
        vp.verified,
        "temporal modus tollens must kernel-certify: {:?}",
        vp.verification_error
    );
}

#[test]
fn a_tensed_hypothesis_is_distinct_from_its_untensed_body() {
    // Past(rain) alone must NOT prove rain — the modality is opaque, not erased.
    let rain = pred("rain", vec![]);
    let premises = vec![past(rain.clone())];
    let vp = prove_certify_check(&premises, &rain);
    assert!(
        !vp.verified,
        "Past(P) must not certify P — erasing the modality would be unsound"
    );
}
