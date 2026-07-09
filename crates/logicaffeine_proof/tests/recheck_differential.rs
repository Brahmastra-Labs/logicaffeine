//! R1 differential — re-check REAL certified proof terms with both kernels.
//!
//! The hand-built terms in the kernel's own `recheck` suite prove the algorithm; this
//! suite proves it on the genuine article: terms the prover actually emits, certified
//! through the full `ProofState → qed → check_derivation` pipeline. For every one, the
//! two independently-written kernels are cross-checked. The non-negotiable invariant is
//! that `Disagree` NEVER fires on a real proof — that would mean our trust base is split.
//! Purely propositional proofs (`λ`/application of hypotheses and constructors) must land
//! in the core fragment and earn full `Agreed`; proofs that go through the inductive
//! eliminator (`Match`/`Fix`) are honestly reported as single-checked, not faked green.

use logicaffeine_kernel::{double_check, DoubleCheck};
use logicaffeine_proof::tactic::combinators::{
    assumption, auto, cases, induction, induction_over, intro, seq, split,
};
use logicaffeine_proof::tactic::{CtorSpec, ProofState};
use logicaffeine_proof::tactic::Tactic;
use logicaffeine_proof::{ProofExpr, ProofTerm};

/// An atomic proposition `s`, modeled as the unary predicate `s` applied to a fixed
/// entity `Tom` — the pipeline registers predicates/constants but not bare `Atom`s, so
/// this is the faithful way to exercise propositional shapes end-to-end.
fn atom(s: &str) -> ProofExpr {
    pred(s, c("Tom"))
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}
fn or(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(l), Box::new(r))
}
fn pred(name: &str, t: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args: vec![t], world: None }
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn c(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: var.to_string(), body: Box::new(body) }
}
fn p_of(t: ProofTerm) -> ProofExpr {
    pred("P", t)
}

/// Prove `goal` from `premises` with `tac`, certify, and return the cross-check verdict.
fn verdict(premises: Vec<ProofExpr>, goal: ProofExpr, tac: Tactic) -> DoubleCheck {
    let mut st = ProofState::start(premises, goal);
    st.run(&tac).expect("tactic script closes the goal");
    let vp = st.qed().expect("qed assembles");
    assert!(vp.verified, "main kernel must certify: {:?}", vp.verification_error);
    let term = vp.proof_term.as_ref().expect("a verified proof carries its term");
    double_check(&vp.kernel_ctx, term)
}

fn assert_not_disagree(verdict: DoubleCheck, what: &str) {
    assert!(
        !matches!(verdict, DoubleCheck::Disagree(_)),
        "{what}: the two kernels DISAGREED on a real proof — {verdict:?}"
    );
}

// ===========================================================================
// Propositional proofs — pure λ/application, fully within the re-checker's core
// ===========================================================================

#[test]
fn implication_intro_is_fully_agreed() {
    // ⊢ P → P, proved by `intro h; assumption` → λh:P. h. Both kernels must concur.
    let v = verdict(
        vec![],
        implies(atom("P"), atom("P")),
        seq(vec![intro("h"), assumption()]),
    );
    assert_eq!(v, DoubleCheck::Agreed, "P→P should be fully double-verified");
}

#[test]
fn modus_ponens_is_fully_agreed() {
    // P, P→Q ⊢ Q  → application of a hypothesis to a hypothesis. Core fragment.
    let v = verdict(
        vec![atom("P"), implies(atom("P"), atom("Q"))],
        atom("Q"),
        auto(),
    );
    assert_eq!(v, DoubleCheck::Agreed, "modus ponens should be fully double-verified");
}

#[test]
fn conjunction_intro_is_fully_agreed() {
    // P, Q ⊢ P ∧ Q  → the `And` constructor applied to two proofs (application of a
    // global). Still the core fragment — no Match needed to BUILD a conjunction.
    let v = verdict(
        vec![atom("P"), atom("Q")],
        and(atom("P"), atom("Q")),
        seq(vec![split(), assumption(), assumption()]),
    );
    assert_eq!(v, DoubleCheck::Agreed, "∧-intro should be fully double-verified");
}

#[test]
fn nested_implication_chain_is_fully_agreed() {
    // ⊢ P → Q → P  (the K combinator's type) — nested λ, application of the right hyp.
    let v = verdict(
        vec![],
        implies(atom("P"), implies(atom("Q"), atom("P"))),
        seq(vec![intro("hp"), intro("hq"), assumption()]),
    );
    assert_eq!(v, DoubleCheck::Agreed);
}

// ===========================================================================
// Match-eliminator proofs — now fully double-checked by the re-checker's Match layer
// ===========================================================================

#[test]
fn disjunction_cases_is_fully_agreed() {
    // P∨Q, P→R, Q→R ⊢ R  → certifies via a `Match` on the disjunction (a PARAMETRIC
    // inductive `Or P Q`). The re-checker's Match layer independently re-derives the
    // case types by de Bruijn telescope instantiation and CONCURS — a real ∨-elim proof
    // is now two-kernel-verified, not merely single-checked.
    let v = verdict(
        vec![or(atom("P"), atom("Q")), implies(atom("P"), atom("R")), implies(atom("Q"), atom("R"))],
        atom("R"),
        seq(vec![cases("hp0"), auto(), auto()]),
    );
    assert_eq!(v, DoubleCheck::Agreed, "∨-elim should now be fully double-verified");
}

#[test]
fn conjunction_elim_is_fully_agreed() {
    // P∧Q ⊢ P  → eliminating a conjunction (the parametric inductive `And P Q`). A
    // different constructor shape (one case, two value args) that the telescope must
    // also get right.
    let v = verdict(
        vec![and(atom("P"), atom("Q"))],
        atom("P"),
        seq(vec![cases("hp0"), assumption()]),
    );
    assert_eq!(v, DoubleCheck::Agreed, "∧-elim should be fully double-verified");
}

#[test]
fn existential_elim_is_fully_agreed() {
    // ∃x.P(x), ∀x.P(x)→Q(Tom) ⊢ Q(Tom)  → eliminating an existential (`Ex A P`), whose
    // constructor is DEPENDENT (a later value-argument type `P w` mentions an earlier
    // value parameter `w`). If the de Bruijn telescope mishandled that dependency the
    // kernels would disagree; agreement proves it threads correctly.
    let exists = ProofExpr::Exists { variable: "x".to_string(), body: Box::new(p_of(v("x"))) };
    let major = forall("x", implies(p_of(v("x")), pred("Q", c("Tom"))));
    let v_ = verdict(
        vec![exists, major],
        pred("Q", c("Tom")),
        seq(vec![cases("hp0"), auto()]),
    );
    assert_eq!(v_, DoubleCheck::Agreed, "∃-elim should be fully double-verified");
}

// ===========================================================================
// Fix-eliminator proofs — induction is now FULLY double-checked: the re-checker's
// own structural-termination guard accepts these genuine recursions, so the second
// kernel independently re-derives the dependent eliminator and concurs.
// ===========================================================================

#[test]
fn nat_induction_is_fully_agreed() {
    // P(Zero), ∀k. P(k)→P(Succ k) ⊢ ∀n. P(n)  → a Fix over a Match. The re-checker now
    // covers BOTH: it certifies coverage + case types (Match) AND that the recursive
    // call `rec k` decreases on `k` (the Succ-bound, structurally-smaller variable).
    let succ_k = ProofTerm::Function("Succ".to_string(), vec![v("k")]);
    let v_ = verdict(
        vec![p_of(c("Zero")), forall("k", implies(p_of(v("k")), p_of(succ_k)))],
        forall("n", p_of(v("n"))),
        seq(vec![induction(), auto(), auto()]),
    );
    assert_eq!(v_, DoubleCheck::Agreed, "Nat induction should now be fully double-verified");
}

#[test]
fn list_induction_is_fully_agreed() {
    // P(ENil), ∀h t. P(t)→P(ECons h t) ⊢ ∀l. P(l)  → the generic InductionScheme, a Fix
    // over an N-ary Match. The whole reason for R1, now realized: the recursive call
    // `rec t` on the ECons-bound tail `t` is verified structurally smaller, and the two
    // independently-written kernels — on different representations — concur. The
    // induction proof we built is two-kernel-verified end to end.
    let cons = ProofTerm::Function("ECons".to_string(), vec![v("h"), v("t")]);
    let v_ = verdict(
        vec![p_of(c("ENil")), forall("h", forall("t", implies(p_of(v("t")), p_of(cons))))],
        forall("l", p_of(v("l"))),
        seq(vec![
            induction_over(
                "EList",
                vec![
                    CtorSpec { constructor: "ENil".to_string(), recursive: vec![] },
                    CtorSpec { constructor: "ECons".to_string(), recursive: vec![false, true] },
                ],
            ),
            auto(),
            auto(),
        ]),
    );
    assert_eq!(v_, DoubleCheck::Agreed, "List induction should now be fully double-verified");
}
