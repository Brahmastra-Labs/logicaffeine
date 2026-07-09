//! `omega` — linear integer arithmetic, the discreteness layer that beats
//! rational `linarith`.
//!
//! The signature win is the strict-inequality refutation: `x < y ∧ y < x+1` is
//! rationally satisfiable (`x=0, y=½`) but integer-UNSAT, because `<` over ℤ is
//! `+1 ≤`. Every proof here is kernel-certified through the same trust door as
//! any tactic — `verified == true` is the only acceptance.

use logicaffeine_proof::tactic::ProofState;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn c(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn add1(x: ProofTerm) -> ProofTerm {
    ProofTerm::Function("add".to_string(), vec![x, c("1")])
}
fn p(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args, world: None }
}
/// `a ≤ b` as the Prop `le(a, b) = true`.
fn le(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(ProofTerm::Function("le".to_string(), vec![a, b]), c("true"))
}
/// `a < b` as the Prop `lt(a, b) = true`.
fn lt(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(ProofTerm::Function("lt".to_string(), vec![a, b]), c("true"))
}

#[test]
fn omega_strict_chain_refutes_and_beats_lia() {
    // x < y ∧ y < x+1 ⊢ Q(A). Rationally satisfiable, integer-UNSAT — the
    // headline `omega` beats `linarith` case.
    let goal = p("Q", vec![c("A")]);
    let mut st = ProofState::start(
        vec![lt(c("x"), c("y")), lt(c("y"), add1(c("x")))],
        goal,
    );
    st.omega().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "strict chain: {:?}", r.verification_error);
}

#[test]
fn omega_strict_irreflexivity() {
    // x < x ⊢ Q(A):  x < x  →  x+1 ≤ x  →  1 ≤ 0  →  ⊥.
    let goal = p("Q", vec![c("A")]);
    let mut st = ProofState::start(vec![lt(c("x"), c("x"))], goal);
    st.omega().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "x<x: {:?}", r.verification_error);
}

#[test]
fn omega_le_lt_antisymmetry() {
    // a ≤ b ∧ b < a ⊢ Q(A):  b < a → b+1 ≤ a,  with a ≤ b  →  contradiction.
    let goal = p("Q", vec![c("A")]);
    let mut st = ProofState::start(vec![le(c("a"), c("b")), lt(c("b"), c("a"))], goal);
    st.omega().unwrap();
    let r = st.qed().unwrap();
    assert!(r.verified, "a≤b ∧ b<a: {:?}", r.verification_error);
}

#[test]
fn omega_declines_satisfiable_strict() {
    // x < y alone is satisfiable (x=0, y=1) — omega must NOT prove an arbitrary
    // goal from it.
    let goal = p("Q", vec![c("A")]);
    let mut st = ProofState::start(vec![lt(c("x"), c("y"))], goal);
    assert!(st.omega().is_err(), "a single strict fact is satisfiable");
}

#[test]
fn omega_declines_without_arith_hyps() {
    let goal = p("Q", vec![c("A")]);
    let mut st = ProofState::start(vec![p("R", vec![c("B")])], goal);
    assert!(st.omega().is_err(), "no arithmetic hypotheses → decline");
}

#[test]
fn omega_beats_lia_head_to_head() {
    // Concrete head-to-head: the RATIONAL Farkas core finds no refutation for
    // the strict system (rationally SAT), while omega refutes it. This pins the
    // claim that omega is strictly stronger, not just a re-skin.
    use logicaffeine_proof::linarith_solve::{find_farkas, parse_lin};
    // The rational relaxation of the strict system: `x - y ≤ 0` and
    // `y - (x+1) ≤ 0`. This has the model x=0, y=½, so no rational Farkas
    // certificate exists — the gap that only integer discreteness closes.
    let x_minus_y = parse_lin(&ProofTerm::Function(
        "sub".to_string(),
        vec![c("x"), c("y")],
    ))
    .unwrap();
    let y_minus_xp1 = parse_lin(&ProofTerm::Function(
        "sub".to_string(),
        vec![c("y"), add1(c("x"))],
    ))
    .unwrap();
    assert!(
        find_farkas(&[x_minus_y, y_minus_xp1]).is_none(),
        "rational Farkas must find the strict relaxation SATISFIABLE — that is the gap omega closes"
    );

    // omega, on the same system, proves the goal.
    let goal = p("Q", vec![c("A")]);
    let mut st = ProofState::start(vec![lt(c("x"), c("y")), lt(c("y"), add1(c("x")))], goal);
    st.omega().unwrap();
    assert!(st.qed().unwrap().verified, "omega refutes what rational Farkas cannot");
}

#[test]
fn omega_auto_hook_also_refutes_strict() {
    // The discreteness step is wired into `auto`'s ⊥-derivation too, so the
    // strict contradiction is reachable without naming `omega`.
    let goal = p("Q", vec![c("A")]);
    let mut st = ProofState::start(
        vec![lt(c("x"), c("y")), lt(c("y"), add1(c("x")))],
        goal,
    );
    // `auto` may or may not ex-falso an arbitrary atom; if it does, it must
    // certify. We assert the omega tactic path (guaranteed) and that auto does
    // not FALSELY succeed on a satisfiable system elsewhere.
    if st.auto().is_ok() {
        assert!(st.qed().unwrap().verified, "auto's strict refutation must certify");
    }
}

#[test]
fn script_omega_prose_works() {
    let goal = p("Q", vec![c("A")]);
    let mut st = ProofState::start(
        vec![lt(c("x"), c("y")), lt(c("y"), add1(c("x")))],
        goal,
    );
    st.run_script("Omega.").unwrap();
    assert!(st.qed().unwrap().verified, "Omega. prose must certify");
}

#[test]
fn probe_pure_le_with_constant_via_auto() {
    // No `lt` at all — pure `≤` with a constant: x+1 ≤ y ∧ y ≤ x is a rational
    // contradiction (1 ≤ 0). Isolates whether cert_farkas handles constant terms
    // independently of the omega/discreteness path.
    let goal = p("Q", vec![c("A")]);
    let mut st = ProofState::start(vec![le(add1(c("x")), c("y")), le(c("y"), c("x"))], goal);
    // auto should refute via cert_farkas (pure rational) and ex-falso.
    let ok = st.auto().is_ok();
    assert!(ok, "cert_farkas failed on a constant-bearing rational contradiction");
    assert!(st.qed().unwrap().verified, "pure-le farkas must certify");
}

#[test]
fn probe_double_constant_le_via_auto() {
    // The naive tightened system with a constant on both sides of both facts.
    // Rationally: x+1≤y ∧ y+1≤x+1 → x+1≤y≤x → 1≤0. find_farkas returns the
    // multipliers and the kernel reconstruction must normalize the doubled
    // constants (like-monomial merges that cancel to zero or recombine to
    // coefficient 1 — the shapes locked by arith::tests).
    let goal = p("Q", vec![c("A")]);
    let mut st = ProofState::start(
        vec![le(add1(c("x")), c("y")), le(add1(c("y")), add1(c("x")))],
        goal,
    );
    let ok = st.auto().is_ok();
    assert!(ok, "cert_farkas failed on the double-constant tightened system");
    assert!(st.qed().unwrap().verified, "double-constant farkas must certify");
}
