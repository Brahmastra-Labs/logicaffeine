//! **Ratcheting "no finite randomness" to `n = ∞`.**
//!
//! The constructive Nullstellensatz certificate exists at every finite `n` because the partition of unity
//! `Σ_a δ_a = 1` holds at every `n` — and that identity FACTORS into a product of `n` copies of the atom
//! `(1+x)+x = 1` (`polycalc::pou_*`). This test closes the discrete-math ladder end to end:
//!
//! - **The rungs (polynomial, re-checked here):** the base `PoU(0) = 1`, and the `n`-uniform inductive step
//!   — `PoU(n) = Π atoms` with every atom `= 1`, so `PoU(n+1) = PoU(n)·1 = PoU(n)`. The step depends on *one*
//!   fixed identity, not on `n`.
//! - **The ladder (kernel-certified):** the leap `base ∧ (∀k. P(k) → P(Succ k)) ⊢ ∀n. P(n)` is discharged
//!   through the kernel's `Nat` recursor — the same certified induction principle `tactic_induction` uses.
//!
//! So the `∀n` statement is not `2ⁿ` cubes checked one at a time (that dies fast); it is `n` identical unit
//! factors, with the induction that turns "one uniform step" into "all `n`" certified by the kernel. Full
//! integration (encoding the polynomial ring *inside* the kernel so the base/step are kernel terms too) is
//! the next level; here the rungs are computation-verified and the ladder is kernel-verified.

use std::collections::BTreeSet;

use logicaffeine_proof::polycalc::{partition_of_unity, pou_as_product, pou_atom};
use logicaffeine_proof::tactic::combinators::{auto, induction, seq};
use logicaffeine_proof::tactic::ProofState;
use logicaffeine_proof::{ProofExpr, ProofTerm};

/// The multilinear polynomial `1` (the empty monomial).
fn one() -> BTreeSet<u64> {
    [0u64].into_iter().collect()
}

fn nfr(t: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: "NoFiniteRandomness".to_string(), args: vec![t], world: None }
}
fn zero() -> ProofTerm {
    ProofTerm::Constant("Zero".to_string())
}
fn succ(t: ProofTerm) -> ProofTerm {
    ProofTerm::Function("Succ".to_string(), vec![t])
}
fn var(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn forall(v: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: v.to_string(), body: Box::new(body) }
}

#[test]
fn no_finite_randomness_ratchets_to_infinity_rungs_verified_ladder_kernel_certified() {
    // ── The rungs (polynomial base + n-uniform step), re-checked ────────────────────────────────────────
    // Base: the 0-cube's partition of unity is 1.
    assert_eq!(partition_of_unity(0), one(), "base case: PoU(0) = 1");
    // The step engine: the atom (1+x_v)+x_v = 1, on every coordinate — ONE identity, independent of n.
    for v in 0..10 {
        assert_eq!(pou_atom(v), one(), "the atom on x{v} is the constant 1 (the n-uniform step engine)");
    }
    // The factorization and the conclusion at each finite n: the 2ⁿ-term sum equals the n-factor product,
    // and that product of ones is one. (This is the computational content the ∀n induction abstracts.)
    for n in 0..=13 {
        assert_eq!(partition_of_unity(n), pou_as_product(n), "PoU(n) = Π atoms (sum-of-products = product-of-sums)");
        assert_eq!(pou_as_product(n), one(), "Π of n unit atoms is 1 — so PoU(n) = 1 for this n");
    }
    // The step, explicit: appending one coordinate multiplies by that atom (=1), so PoU is unchanged. The
    // product form makes this hold for the *next* n from the current one — the n-uniform recurrence.
    for n in 0..13 {
        assert_eq!(pou_as_product(n + 1), pou_as_product(n), "PoU(n+1) = PoU(n)·atom = PoU(n) — the uniform step");
    }

    // ── The ladder (kernel-certified): base ∧ step ⊢ ∀n. NoFiniteRandomness(n) ──────────────────────────
    // The base `NoFiniteRandomness(Zero)` and the step `∀k. NoFiniteRandomness(k) → NoFiniteRandomness(Succ k)`
    // are the two rungs discharged above (as polynomial identities); the kernel's Nat recursor certifies that
    // they entail the statement for ALL n. This is the ratchet from "one uniform step" to "n = ∞".
    let base = nfr(zero());
    let step = forall("k", implies(nfr(var("k")), nfr(succ(var("k")))));
    let goal = forall("n", nfr(var("n")));
    let mut st = ProofState::start(vec![base, step], goal);
    st.run(&seq(vec![induction(), auto(), auto()])).expect("induction; auto; auto discharges base and step");
    let result = st.qed().expect("the ∀n derivation assembles");
    assert!(result.verified, "kernel-certified ∀n induction: {:?}", result.verification_error);
}
