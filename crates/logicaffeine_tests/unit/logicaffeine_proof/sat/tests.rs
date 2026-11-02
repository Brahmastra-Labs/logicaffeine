use super::*;

fn atom(s: &str) -> ProofExpr {
    ProofExpr::Atom(s.to_string())
}
fn not(e: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(e))
}
fn and(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(a), Box::new(b))
}
fn or(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(a), Box::new(b))
}
fn implies(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(a), Box::new(b))
}

/// Evaluate a Boolean `ProofExpr` under an assignment — the independent oracle that
/// proves a returned counterexample genuinely distinguishes two formulas (robust-to-
/// absurdity: we never trust the solver's model without re-checking it ourselves).
fn eval(e: &ProofExpr, env: &[(String, bool)]) -> bool {
    match e {
        ProofExpr::Atom(n) => env
            .iter()
            .find(|(k, _)| k == n)
            .map(|(_, v)| *v)
            .unwrap_or(false),
        ProofExpr::Not(p) => !eval(p, env),
        ProofExpr::And(p, q) => eval(p, env) && eval(q, env),
        ProofExpr::Or(p, q) => eval(p, env) || eval(q, env),
        ProofExpr::Implies(p, q) => !eval(p, env) || eval(q, env),
        ProofExpr::Iff(p, q) => eval(p, env) == eval(q, env),
        _ => panic!("non-boolean node in test eval"),
    }
}

#[test]
fn reflexive_equivalence_is_certified() {
    // `req |-> ack` at t0 against itself — the trivial but load-bearing identity.
    let f = implies(atom("req@0"), atom("ack@0"));
    assert_eq!(prove_equivalence(&f, &f), EquivOutcome::Equivalent);
}

#[test]
fn de_morgan_is_equivalent() {
    // ¬(a ∧ b) ≡ (¬a ∨ ¬b)
    let lhs = not(and(atom("a@0"), atom("b@0")));
    let rhs = or(not(atom("a@0")), not(atom("b@0")));
    assert_eq!(prove_equivalence(&lhs, &rhs), EquivOutcome::Equivalent);
}

#[test]
fn distributivity_is_equivalent() {
    // a ∧ (b ∨ c) ≡ (a ∧ b) ∨ (a ∧ c)
    let lhs = and(atom("a@0"), or(atom("b@0"), atom("c@0")));
    let rhs = or(and(atom("a@0"), atom("b@0")), and(atom("a@0"), atom("c@0")));
    assert_eq!(prove_equivalence(&lhs, &rhs), EquivOutcome::Equivalent);
}

#[test]
fn distinct_tautologies_are_equivalent() {
    // Two excluded-middle tautologies over different atoms are both constantly true,
    // so they are equivalent even with no shared variables.
    let lhs = or(atom("p@0"), not(atom("p@0")));
    let rhs = or(atom("q@0"), not(atom("q@0")));
    assert_eq!(prove_equivalence(&lhs, &rhs), EquivOutcome::Equivalent);
}

#[test]
fn implication_is_not_its_consequent() {
    // `req → ack`  vs  `ack` differ: at req=0, ack=0 the implication holds but ack does
    // not. The verdict must be Differ AND the counterexample must genuinely distinguish.
    let f = implies(atom("req@0"), atom("ack@0"));
    let s = atom("ack@0");
    match prove_equivalence(&f, &s) {
        EquivOutcome::Differ(model) => {
            assert_ne!(
                eval(&f, &model),
                eval(&s, &model),
                "counterexample {:?} must distinguish the two formulas",
                model
            );
        }
        other => panic!("expected Differ, got {:?}", other),
    }
}

#[test]
fn implication_is_not_its_converse() {
    // `req → ack` vs `ack → req` differ; verify the witness is real.
    let f = implies(atom("req@0"), atom("ack@0"));
    let s = implies(atom("ack@0"), atom("req@0"));
    match prove_equivalence(&f, &s) {
        EquivOutcome::Differ(model) => {
            assert_ne!(eval(&f, &model), eval(&s, &model));
        }
        other => panic!("expected Differ, got {:?}", other),
    }
}

#[test]
fn find_model_of_contradiction_is_unsat() {
    assert_eq!(
        find_model(&and(atom("a@0"), not(atom("a@0")))),
        ModelOutcome::Unsat
    );
}

#[test]
fn find_model_of_satisfiable_returns_witness() {
    // a ∧ (a → b) forces a=true, b=true.
    let e = and(atom("a@0"), implies(atom("a@0"), atom("b@0")));
    match find_model(&e) {
        ModelOutcome::Sat(model) => {
            assert!(
                eval(&e, &model),
                "returned model must actually satisfy the formula"
            );
            assert!(model.iter().any(|(k, v)| k == "a@0" && *v));
            assert!(model.iter().any(|(k, v)| k == "b@0" && *v));
        }
        other => panic!("expected Sat, got {:?}", other),
    }
}
