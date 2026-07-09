//! Counterexample search: when a goal is false, exhibit a re-verified model.
//! The load-bearing property is ZERO false alarms — a returned witness
//! provably satisfies every premise and falsifies the goal, and a true goal
//! yields no witness.

use logicaffeine_proof::counterexample::{find_counterexample, Counterexample};
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn vt(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn f(name: &str, args: Vec<ProofTerm>) -> ProofTerm {
    ProofTerm::Function(name.to_string(), args)
}
fn pr(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args, world: None }
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: var.to_string(), body: Box::new(body) }
}
/// `a op b` = true.
fn cmp(op: &str, a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(f(op, vec![a, b]), k("true"))
}

#[test]
fn quickcheck_int_witness_found() {
    // ⊢ ∀x. 0 ≤ x is false — witness x = -1 (the smallest by |·| among the
    // grid values that falsify it).
    let goal = forall("x", cmp("le", k("0"), vt("x")));
    let cex = find_counterexample(&[], &goal).expect("0 ≤ x is false for some x");
    match cex {
        Counterexample::Witness(b) => {
            assert_eq!(b, vec![("x".to_string(), -1)], "smallest falsifying witness");
        }
        other => panic!("expected an integer witness, got {other:?}"),
    }
}

#[test]
fn quickcheck_respects_premises() {
    // Premise 1 ≤ x, goal 2 ≤ x: false, and the witness must SATISFY 1 ≤ x
    // (so x = 1, not x = 0 or x = -1).
    let cex = find_counterexample(
        &[cmp("le", k("1"), vt("x"))],
        &cmp("le", k("2"), vt("x")),
    )
    .expect("2 ≤ x does not follow from 1 ≤ x");
    match cex {
        Counterexample::Witness(b) => {
            assert_eq!(b, vec![("x".to_string(), 1)], "witness satisfies the premise");
        }
        other => panic!("expected witness, got {other:?}"),
    }
}

#[test]
fn no_counterexample_for_true_arithmetic_goal() {
    // ∀x. x ≤ x is true — no witness. (Zero false alarms.)
    let goal = forall("x", cmp("le", vt("x"), vt("x")));
    assert!(find_counterexample(&[], &goal).is_none(), "a true goal has no counterexample");
}

#[test]
fn no_counterexample_when_goal_follows_from_premises() {
    // 1 ≤ x ⊢ 0 ≤ x is TRUE — no witness anywhere on the grid.
    assert!(
        find_counterexample(&[cmp("le", k("1"), vt("x"))], &cmp("le", k("0"), vt("x"))).is_none()
    );
}

#[test]
fn sat_model_names_failing_atoms() {
    // P(A) → Q(A) ⊢ Q(A) is false when P(A)=false, Q(A)=false.
    let cex = find_counterexample(
        &[implies(pr("P", vec![k("A")]), pr("Q", vec![k("A")]))],
        &pr("Q", vec![k("A")]),
    )
    .expect("Q(A) does not follow from P(A)→Q(A)");
    match cex {
        Counterexample::Valuation(b) => {
            // The goal atom Q(A) must be false in the model.
            let q = b.iter().find(|(a, _)| a.contains("Q")).expect("Q(A) present");
            assert!(!q.1, "the goal atom must be falsified");
            // And the premise must hold: P(A)→Q(A) with Q(A) false ⟹ P(A) false.
            let p = b.iter().find(|(a, _)| a.contains("P")).expect("P(A) present");
            assert!(!p.1, "P(A) must be false so the implication premise holds");
        }
        other => panic!("expected a propositional valuation, got {other:?}"),
    }
}

#[test]
fn no_counterexample_for_valid_propositional_goal() {
    // P(A) ∧ (P(A)→Q(A)) ⊢ Q(A) is valid — no model falsifies it.
    let cex = find_counterexample(
        &[pr("P", vec![k("A")]), implies(pr("P", vec![k("A")]), pr("Q", vec![k("A")]))],
        &pr("Q", vec![k("A")]),
    );
    assert!(cex.is_none(), "modus ponens goal has no counterexample");
}

#[test]
fn render_reads_cleanly() {
    let w = Counterexample::Witness(vec![("x".to_string(), -1)]);
    assert_eq!(w.render(), "false when x = -1");
    let v = Counterexample::Valuation(vec![("P(A)".to_string(), false)]);
    assert_eq!(v.render(), "false when P(A) = false");
}
