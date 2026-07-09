//! `decide` — proof by evaluation for closed decidable goals.
//!
//! Ground arithmetic identities close through the proof-producing arithmetic
//! oracle; ground comparisons and Bool/Nat equalities close through
//! `native_decide` (the kernel's trusted-evaluator route, gated by the
//! `reduceBool` hook); propositional combinations recurse through the intro
//! rules. Everything is re-checked by the kernel — a lying leaf is rejected,
//! and a false or open goal is declined, never "proved".

use logicaffeine_proof::tactic::ProofState;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn f(name: &str, args: Vec<ProofTerm>) -> ProofTerm {
    ProofTerm::Function(name.to_string(), args)
}
fn p(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args, world: None }
}
fn eq(l: ProofTerm, r: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(l, r)
}
/// `a OP b` in the system's canonical encoding: `Identity(op(a, b), true)`.
fn cmp(op: &str, a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(f(op, vec![a, b]), k("true"))
}

#[test]
fn decide_ground_int_equality() {
    // ⊢ add(2, 3) = 5
    let goal = eq(f("add", vec![k("2"), k("3")]), k("5"));
    let mut st = ProofState::start(vec![], goal);
    st.decide().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "2+3=5: {:?}", r.verification_error);
}

#[test]
fn decide_nested_ground_arithmetic() {
    // ⊢ mul(add(2, 3), 4) = 20
    let goal = eq(f("mul", vec![f("add", vec![k("2"), k("3")]), k("4")]), k("20"));
    let mut st = ProofState::start(vec![], goal);
    st.decide().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "(2+3)*4=20: {:?}", r.verification_error);
}

#[test]
fn decide_false_equality_declines() {
    let goal = eq(k("2"), k("3"));
    let mut st = ProofState::start(vec![], goal);
    assert!(st.decide().is_err(), "2=3 must be declined, not proved");
}

#[test]
fn decide_ground_comparison() {
    // ⊢ le(2, 5) — closed through native_decide (Eq Bool (le 2 5) true).
    let goal = cmp("le", k("2"), k("5"));
    let mut st = ProofState::start(vec![], goal);
    st.decide().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "le(2,5): {:?}", r.verification_error);
}

#[test]
fn decide_false_comparison_declines() {
    let goal = cmp("lt", k("5"), k("2"));
    let mut st = ProofState::start(vec![], goal);
    assert!(st.decide().is_err(), "lt(5,2) must be declined");
}

#[test]
fn decide_conjunction_of_ground_atoms() {
    // ⊢ le(2, 3) ∧ add(1, 1) = 2
    let goal = ProofExpr::And(
        Box::new(cmp("le", k("2"), k("3"))),
        Box::new(eq(f("add", vec![k("1"), k("1")]), k("2"))),
    );
    let mut st = ProofState::start(vec![], goal);
    st.decide().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "conjunction: {:?}", r.verification_error);
}

#[test]
fn decide_disjunction_picks_the_true_side() {
    // ⊢ 2 = 3 ∨ le(1, 2): the left disjunct is false; decide must pick right.
    let goal = ProofExpr::Or(
        Box::new(eq(k("2"), k("3"))),
        Box::new(cmp("le", k("1"), k("2"))),
    );
    let mut st = ProofState::start(vec![], goal);
    st.decide().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "disjunction: {:?}", r.verification_error);
}

#[test]
fn decide_implication_via_true_consequent() {
    // ⊢ le(9, 2) → le(1, 2): the consequent is decidably true, so the
    // implication holds by weakening.
    let goal = ProofExpr::Implies(
        Box::new(cmp("le", k("9"), k("2"))),
        Box::new(cmp("le", k("1"), k("2"))),
    );
    let mut st = ProofState::start(vec![], goal);
    st.decide().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "implication: {:?}", r.verification_error);
}

#[test]
fn decide_declines_open_goal() {
    // A free variable means the goal is not closed — decline, never guess.
    let goal = eq(f("add", vec![v("x"), k("0")]), v("x"));
    let mut st = ProofState::start(vec![], goal);
    assert!(st.decide().is_err(), "open goal must be declined");
}

#[test]
fn decide_never_proves_false_goal_through_the_kernel() {
    // A hand-built lying NativeDecide leaf claims lt(5, 2). The tactic layer
    // never builds this; the kernel must reject it anyway — the trust door.
    use logicaffeine_proof::verify::check_derivation;
    use logicaffeine_proof::{DerivationTree, InferenceRule};
    let lie = cmp("lt", k("5"), k("2"));
    let tree = DerivationTree::leaf(lie.clone(), InferenceRule::NativeDecide);
    let r = check_derivation(&[], &lie, tree);
    assert!(!r.verified, "a lying NativeDecide leaf must be rejected");
}

#[test]
fn script_decide_prose_works() {
    let goal = eq(f("add", vec![k("20"), k("22")]), k("42"));
    let mut st = ProofState::start(vec![], goal);
    st.run_script("Decide.").unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "Decide. prose: {:?}", r.verification_error);
}
