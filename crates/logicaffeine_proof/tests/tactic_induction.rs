//! The `induction` tactic (ROOT R5): structural induction over `Nat`. A goal
//! `∀n. P(n)` splits into the base `P(Zero)` and the step `P(Succ k)` with the
//! induction hypothesis `P(k)` in scope; the assembled proof is a `Fix`/`Match`
//! over the kernel's `Nat` recursor, kernel-certified.

use logicaffeine_proof::tactic::combinators::{auto, induction, induction_over, seq};
use logicaffeine_proof::tactic::{CtorSpec, ProofState};
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn c(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn succ(t: ProofTerm) -> ProofTerm {
    ProofTerm::Function("Succ".to_string(), vec![t])
}
fn p_of(t: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: "P".to_string(), args: vec![t], world: None }
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: var.to_string(), body: Box::new(body) }
}

#[test]
fn induction_over_nat_with_base_and_step() {
    // P(Zero), ∀k. P(k) → P(Succ k) ⊢ ∀n. P(n), by `induction; auto; auto`.
    // induction yields the base goal P(Zero) and the step goal P(Succ k) with the
    // hypothesis ih : P(k); `auto` discharges each from the premises (and the IH).
    let base = p_of(c("Zero"));
    let step = forall("k", implies(p_of(v("k")), p_of(succ(v("k")))));
    let goal = forall("n", p_of(v("n")));
    let mut st = ProofState::start(vec![base, step], goal);
    st.run(&seq(vec![induction(), auto(), auto()])).unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "induction over Nat: {:?}", r.verification_error);
}

fn cons(h: ProofTerm, t: ProofTerm) -> ProofTerm {
    ProofTerm::Function("ECons".to_string(), vec![h, t])
}

#[test]
fn induction_over_list_with_nil_and_cons() {
    // P(ENil), ∀h t. P(t) → P(ECons h t) ⊢ ∀l. P(l), by generic structural induction
    // over the prelude's monomorphic `EList`. `induction_over` yields the base goal
    // P(ENil) and the step goal P(ECons h t) with the induction hypothesis ih : P(t)
    // (the tail) in scope. ECons has TWO binders — a shape the Nat-fixed `induction`
    // cannot express — so this exercises the generic `InductionScheme` eliminator:
    // `fix rec. λl. match l { ENil => …, ECons h t => … }`, kernel-certified for
    // coverage, case types, and termination.
    let base = p_of(c("ENil"));
    let step = forall("h", forall("t", implies(p_of(v("t")), p_of(cons(v("h"), v("t"))))));
    let goal = forall("l", p_of(v("l")));
    let mut st = ProofState::start(vec![base, step], goal);
    st.run(&seq(vec![
        induction_over(
            "EList",
            vec![
                CtorSpec { constructor: "ENil".to_string(), recursive: vec![] },
                CtorSpec { constructor: "ECons".to_string(), recursive: vec![false, true] },
            ],
        ),
        auto(),
        auto(),
    ]))
    .unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "induction over EList: {:?}", r.verification_error);
}

#[test]
fn induction_over_with_a_missing_case_is_rejected_by_the_kernel() {
    // The de Bruijn safety net at the tactic level: an induction scheme that omits a
    // constructor builds a NON-EXHAUSTIVE `match`, which the kernel's coverage check
    // must reject — even though the single emitted subgoal P(ENil) closes cleanly. A
    // bad eliminator never becomes a theorem; soundness is kernel-enforced, not
    // tactic-trusted.
    let base = p_of(c("ENil"));
    let step = forall("h", forall("t", implies(p_of(v("t")), p_of(cons(v("h"), v("t"))))));
    let goal = forall("l", p_of(v("l")));
    let mut st = ProofState::start(vec![base, step], goal);
    // Only the ENil case — ECons deliberately omitted.
    st.run(&seq(vec![
        induction_over(
            "EList",
            vec![CtorSpec { constructor: "ENil".to_string(), recursive: vec![] }],
        ),
        auto(),
    ]))
    .unwrap();
    let r = st.qed().unwrap();
    assert!(
        !r.verified,
        "a match missing the ECons case must fail the kernel's coverage check"
    );
}

#[test]
fn induction_over_requires_a_forall_goal() {
    let goal = p_of(c("ENil"));
    let mut st = ProofState::start(vec![], goal);
    assert!(
        st.induction_over("EList", vec![]).is_err(),
        "generic induction on a non-∀ goal must fail"
    );
}

#[test]
fn induction_requires_a_forall_goal() {
    let goal = p_of(c("Zero"));
    let mut st = ProofState::start(vec![], goal);
    assert!(st.induction().is_err(), "induction on a non-∀ goal must fail");
}
