use super::*;
use crate::{ProofExpr, ProofTerm};

fn c(s: &str) -> ProofTerm {
    ProofTerm::Constant(s.to_string())
}
fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args,
        world: None,
    }
}
fn not(e: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(e))
}
fn or(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(a), Box::new(b))
}
fn and(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(a), Box::new(b))
}
fn imp(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(a), Box::new(b))
}
fn id(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(a, b)
}

#[test]
fn reflexive_entailment() {
    let p = pred("P", vec![c("a")]);
    assert_eq!(cdcl_entails(&[p.clone()], &p), Some(true));
}

#[test]
fn disjunctive_syllogism() {
    // P ∨ Q, ¬P ⊨ Q
    let p = pred("P", vec![c("a")]);
    let q = pred("Q", vec![c("a")]);
    let prem = vec![or(p.clone(), q.clone()), not(p.clone())];
    assert_eq!(cdcl_entails(&prem, &q), Some(true));
    // …but P ∨ Q alone does NOT entail Q.
    assert_eq!(cdcl_entails(&[or(p.clone(), q.clone())], &q), Some(false));
}

#[test]
fn modus_ponens() {
    // P, P → Q ⊨ Q
    let p = pred("P", vec![c("a")]);
    let q = pred("Q", vec![c("a")]);
    let prem = vec![p.clone(), imp(p.clone(), q.clone())];
    assert_eq!(cdcl_entails(&prem, &q), Some(true));
}

#[test]
fn non_entailment_is_false_not_none() {
    let p = pred("P", vec![c("a")]);
    let q = pred("Q", vec![c("a")]);
    assert_eq!(cdcl_entails(&[p], &q), Some(false));
}

#[test]
fn identity_is_symmetric() {
    // a = b ⊨ b = a (same variable, so ¬(b=a) clashes immediately).
    let prem = vec![id(c("a"), c("b"))];
    assert_eq!(cdcl_entails(&prem, &id(c("b"), c("a"))), Some(true));
}

#[test]
fn two_value_grid_forces_the_cell() {
    // A 2-trip × 2-state bijection: each trip in FL or ME; at most one trip per state;
    // Alpha in FL ⇒ Beta in ME. The Untrusted CDCL tier must force it.
    let in_ = |t: &str, s: &str| pred("In", vec![c(t), c(s)]);
    let fl = "Florida";
    let me = "Maine";
    let prem = vec![
        // closures: each trip takes a state
        or(in_("Alpha", fl), in_("Alpha", me)),
        or(in_("Beta", fl), in_("Beta", me)),
        // at-most-one per state (functionality): two trips can't share a state
        imp(in_("Alpha", fl), not(in_("Beta", fl))),
        imp(in_("Alpha", me), not(in_("Beta", me))),
        // each state taken by some trip (column closure)
        or(in_("Alpha", fl), in_("Beta", fl)),
        or(in_("Alpha", me), in_("Beta", me)),
        // the pin
        in_("Alpha", fl),
    ];
    assert_eq!(
        cdcl_entails(&prem, &in_("Beta", me)),
        Some(true),
        "Beta∈Maine is forced"
    );
    assert_eq!(
        cdcl_entails(&prem, &in_("Beta", fl)),
        Some(false),
        "Beta∈Florida is refuted"
    );
}

#[test]
fn negated_conjunction_clausifies_directly_without_aux() {
    // ¬(P ∧ Q) ≡ ¬P ∨ ¬Q — one binary clause, NO Tseitin auxiliary. Every at-most-one
    // constraint is this shape; minting an aux per pair bloated the CNF and the solve.
    let p = pred("P", vec![c("a")]);
    let q = pred("Q", vec![c("a")]);
    let mut cnf = Cnf::new();
    cnf.assert(&not(and(p, q))).unwrap();
    assert_eq!(cnf.num_atoms(), 2, "only P and Q are atoms");
    assert_eq!(cnf.num_vars(), 2, "no auxiliary variable should be minted");
    assert_eq!(cnf.clauses().len(), 1, "exactly one clause");
    assert_eq!(cnf.clauses()[0].len(), 2, "the binary clause ¬P ∨ ¬Q");
}

#[test]
fn negated_conjunction_preserves_models() {
    // Soundness guard for the De Morgan clause: P ∧ ¬(P ∧ Q) ⊨ ¬Q, but ¬(P ∧ Q) alone does not.
    let p = pred("P", vec![c("a")]);
    let q = pred("Q", vec![c("a")]);
    assert_eq!(
        cdcl_entails(
            &[p.clone(), not(and(p.clone(), q.clone()))],
            &not(q.clone())
        ),
        Some(true)
    );
    assert_eq!(
        cdcl_entails(&[not(and(p, q.clone()))], &not(q)),
        Some(false)
    );
}

#[test]
fn nested_demorgan_and_double_negation_clausify() {
    // ¬((P ∧ Q) ∧ R) ≡ ¬P ∨ ¬Q ∨ ¬R (one ternary clause, no aux); ¬¬P ≡ P.
    let (p, q, r) = (
        pred("P", vec![c("a")]),
        pred("Q", vec![c("a")]),
        pred("R", vec![c("a")]),
    );
    let mut cnf = Cnf::new();
    cnf.assert(&not(and(and(p, q), r))).unwrap();
    assert_eq!(cnf.num_vars(), 3, "no aux for a nested negated conjunction");
    assert_eq!(cnf.clauses().len(), 1);
    assert_eq!(cnf.clauses()[0].len(), 3, "ternary clause ¬P ∨ ¬Q ∨ ¬R");

    let p2 = pred("P", vec![c("a")]);
    let mut cnf2 = Cnf::new();
    cnf2.assert(&not(not(p2))).unwrap();
    assert_eq!(cnf2.num_vars(), 1, "¬¬P is just P");
}
