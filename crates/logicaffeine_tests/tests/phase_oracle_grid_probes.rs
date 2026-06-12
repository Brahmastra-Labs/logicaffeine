#![cfg(feature = "verification")]
//! ============================================================================
//! ORACLE GRID PROBES — the reasoning kernel a logic-grid puzzle needs
//! ============================================================================
//!
//! A puzzle document compiles to premises whose solving requires, at minimum:
//! equality congruence, finite-domain closure with case splits, disjunction
//! reasoning (either/or and of-pair XOR clues), integer offset arithmetic
//! ("scored 3 points higher"), event-semantic premises, and — since every
//! clue is past tense — all of it SURVIVING a `Temporal(Past)` wrapper.
//!
//! Each probe pins one capability of `oracle_entails`. The Past-wrapped
//! arithmetic probe is the load-bearing one: if the world-indexed modal
//! translation cannot force values through tense, the puzzle door must
//! erase tense (a grid puzzle is a single static scenario).

use logicaffeine_proof::oracle::{
    oracle_consistent, oracle_entails, SmtConsistency, SmtVerdict,
};
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn c(name: &str) -> ProofTerm {
    ProofTerm::Constant(name.to_string())
}

fn v(name: &str) -> ProofTerm {
    ProofTerm::Variable(name.to_string())
}

fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args,
        world: None,
    }
}

fn func(name: &str, args: Vec<ProofTerm>) -> ProofTerm {
    ProofTerm::Function(name.to_string(), args)
}

fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}

fn or(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(l), Box::new(r))
}

fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}

fn not(e: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(e))
}

fn ident(l: ProofTerm, r: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(l, r)
}

fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll {
        variable: var.to_string(),
        body: Box::new(body),
    }
}

fn exists(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::Exists {
        variable: var.to_string(),
        body: Box::new(body),
    }
}

fn past(e: ProofExpr) -> ProofExpr {
    ProofExpr::Temporal {
        operator: "Past".to_string(),
        body: Box::new(e),
    }
}

/// {a = b, P(a)} ⊨ P(b) — definite descriptions resolve through equality.
#[test]
fn equality_congruence() {
    let premises = vec![
        ident(c("A"), c("B")),
        pred("Dancer", vec![c("A")]),
    ];
    let goal = pred("Dancer", vec![c("B")]);
    assert_eq!(oracle_entails(&premises, &goal), SmtVerdict::Entailed);
}

/// Domain closure + an elimination fact force the remaining candidate:
/// every person is B or P; someone danced; P didn't — so B did.
#[test]
fn domain_closure_forces_the_witness() {
    let premises = vec![
        pred("Person", vec![c("Bessie")]),
        pred("Person", vec![c("Patti")]),
        not(ident(c("Bessie"), c("Patti"))),
        forall(
            "x",
            implies(
                pred("Person", vec![v("x")]),
                or(ident(v("x"), c("Bessie")), ident(v("x"), c("Patti"))),
            ),
        ),
        exists(
            "y",
            and(pred("Person", vec![v("y")]), pred("Danced", vec![v("y")])),
        ),
        not(pred("Danced", vec![c("Patti")])),
    ];
    let goal = pred("Danced", vec![c("Bessie")]);
    assert_eq!(oracle_entails(&premises, &goal), SmtVerdict::Entailed);
    assert_eq!(oracle_consistent(&premises), SmtConsistency::Consistent);
}

/// Disjunction elimination — the case split an either/or clue requires.
#[test]
fn disjunction_case_split() {
    let premises = vec![
        or(
            pred("Architect", vec![c("Colin")]),
            pred("Dentist", vec![c("Colin")]),
        ),
        implies(
            pred("Architect", vec![c("Colin")]),
            pred("Employed", vec![c("Colin")]),
        ),
        implies(
            pred("Dentist", vec![c("Colin")]),
            pred("Employed", vec![c("Colin")]),
        ),
    ];
    let goal = pred("Employed", vec![c("Colin")]);
    assert_eq!(oracle_entails(&premises, &goal), SmtVerdict::Entailed);
}

/// The of-pair XOR shape: (P(a) ∧ Q(b)) ∨ (P(b) ∧ Q(a)) plus one refutation
/// resolves the distribution.
#[test]
fn of_pair_xor_resolves_by_refutation() {
    let p_a = pred("Scored190", vec![c("Sixth")]);
    let p_b = pred("Scored190", vec![c("Patti")]);
    let q_a = pred("Lindy", vec![c("Sixth")]);
    let q_b = pred("Lindy", vec![c("Patti")]);
    let premises = vec![
        or(and(p_a.clone(), q_b.clone()), and(p_b.clone(), q_a.clone())),
        not(p_b),
    ];
    assert_eq!(oracle_entails(&premises, &p_a), SmtVerdict::Entailed);
    assert_eq!(oracle_entails(&premises, &q_b), SmtVerdict::Entailed);
}

/// Integer offset + finite range force exact values:
/// pts(A) = pts(B) + 3, both in {181, 184} ⟹ pts(B) = 181 and pts(A) = 184.
#[test]
fn arithmetic_offset_forces_values() {
    let pts_a = func("pts", vec![c("A")]);
    let pts_b = func("pts", vec![c("B")]);
    let premises = vec![
        pred(
            "Eq",
            vec![pts_a.clone(), func("Add", vec![pts_b.clone(), c("3")])],
        ),
        or(
            pred("Eq", vec![pts_a.clone(), c("181")]),
            pred("Eq", vec![pts_a.clone(), c("184")]),
        ),
        or(
            pred("Eq", vec![pts_b.clone(), c("181")]),
            pred("Eq", vec![pts_b.clone(), c("184")]),
        ),
    ];
    let goal_a = pred("Eq", vec![pts_a, c("184")]);
    let goal_b = pred("Eq", vec![pts_b, c("181")]);
    assert_eq!(oracle_entails(&premises, &goal_a), SmtVerdict::Entailed);
    assert_eq!(oracle_entails(&premises, &goal_b), SmtVerdict::Entailed);
}

/// An event-semantic premise entails itself (the NeoEvent encoding is sound
/// end-to-end through the SMT translation).
#[test]
fn neo_event_premise_entails_itself() {
    let event = ProofExpr::NeoEvent {
        event_var: "e".to_string(),
        verb: "Dance".to_string(),
        roles: vec![("Agent".to_string(), c("Bessie"))],
    };
    let premises = vec![event.clone()];
    assert_eq!(oracle_entails(&premises, &event), SmtVerdict::Entailed);
}

/// Vacuity guard: an inconsistent premise set must be detected, never used
/// to fake entailments.
#[test]
fn inconsistent_premises_are_flagged() {
    let premises = vec![
        pred("Dancer", vec![c("A")]),
        not(pred("Dancer", vec![c("A")])),
    ];
    assert_eq!(oracle_consistent(&premises), SmtConsistency::Inconsistent);
}

/// Tense survival, part 1: a Past-wrapped fact entails its Past-wrapped self.
#[test]
fn past_wrapped_fact_entails_itself() {
    let fact = past(pred("Danced", vec![c("Bessie")]));
    let premises = vec![fact.clone()];
    assert_eq!(oracle_entails(&premises, &fact), SmtVerdict::Entailed);
}

/// Tense survival, part 2 — THE LOAD-BEARING PROBE: every puzzle clue is past
/// tense, and solving needs arithmetic + case splits to keep working under the
/// wrapper. If this cannot hold through the world-indexed translation, the
/// puzzle door must erase tense (a grid puzzle is one static scenario).
#[test]
fn past_wrapped_arithmetic_still_forces_values() {
    let pts_a = func("pts", vec![c("A")]);
    let pts_b = func("pts", vec![c("B")]);
    let premises = vec![
        past(pred(
            "Eq",
            vec![pts_a.clone(), func("Add", vec![pts_b.clone(), c("3")])],
        )),
        past(or(
            pred("Eq", vec![pts_a.clone(), c("181")]),
            pred("Eq", vec![pts_a.clone(), c("184")]),
        )),
        past(or(
            pred("Eq", vec![pts_b.clone(), c("181")]),
            pred("Eq", vec![pts_b.clone(), c("184")]),
        )),
    ];
    let goal = past(pred("Eq", vec![pts_a, c("184")]));
    assert_eq!(oracle_entails(&premises, &goal), SmtVerdict::Entailed);
}

/// Exactly-one (the bijection cell): ∃x(P(x) ∧ ∀y(P(y) → y = x)) plus a
/// second positive instance forces identity.
#[test]
fn exactly_one_forces_identity() {
    let premises = vec![
        exists(
            "x",
            and(
                pred("Hustle", vec![v("x")]),
                forall(
                    "y",
                    implies(pred("Hustle", vec![v("y")]), ident(v("y"), v("x"))),
                ),
            ),
        ),
        pred("Hustle", vec![c("Bessie")]),
        pred("Hustle", vec![c("Patti")]),
    ];
    let goal = ident(c("Bessie"), c("Patti"));
    assert_eq!(oracle_entails(&premises, &goal), SmtVerdict::Entailed);
}
