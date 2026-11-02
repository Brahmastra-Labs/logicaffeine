use super::*;

fn predicate(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args,
        world: None,
    }
}

#[test]
fn test_direct_match_hint() {
    let goal = predicate("Human", vec![ProofTerm::Constant("Socrates".to_string())]);
    let kb = vec![goal.clone()];

    let hint = suggest_hint(&goal, &kb, &[]);
    assert!(hint.suggested_tactic == Some(SuggestedTactic::Assumption));
}

#[test]
fn test_modus_ponens_hint() {
    let p = predicate("Human", vec![ProofTerm::Constant("Socrates".to_string())]);
    let q = predicate("Mortal", vec![ProofTerm::Constant("Socrates".to_string())]);
    let implication = ProofExpr::Implies(Box::new(p.clone()), Box::new(q.clone()));

    let kb = vec![p, implication];

    let hint = suggest_hint(&q, &kb, &[]);
    assert!(hint.suggested_tactic == Some(SuggestedTactic::ModusPonens));
}

#[test]
fn test_conjunction_hint() {
    let p = predicate("P", vec![]);
    let q = predicate("Q", vec![]);
    let goal = ProofExpr::And(Box::new(p.clone()), Box::new(q.clone()));

    let kb = vec![p, q];

    let hint = suggest_hint(&goal, &kb, &[]);
    assert!(hint.suggested_tactic == Some(SuggestedTactic::AndIntro));
}

#[test]
fn test_reflexivity_hint() {
    let term = ProofTerm::Constant("x".to_string());
    let goal = ProofExpr::Identity(term.clone(), term);

    let hint = suggest_hint(&goal, &[], &[]);
    assert!(hint.suggested_tactic == Some(SuggestedTactic::Reflexivity));
}
