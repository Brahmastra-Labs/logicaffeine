//! Soundness of the literate tactic-proof surface.
//!
//! A `## Theorem … Proof: <tactic>.` is stored as `name : Derivation := <tactic-term>`. The kernel
//! confirms the body is a well-formed `Derivation`, but a FAILED tactic still yields a well-typed
//! *error* derivation — so type-checking alone would (and historically did) accept a FALSE theorem.
//! The REPL now additionally requires the derivation to CONCLUDE its stated goal (`concludes body`
//! must reduce to exactly the goal syntax). These tests pin: true goals prove, false/malformed goals
//! are rejected, and raw `Derivation` definitions (which legitimately construct error derivations)
//! are left untouched.

use logicaffeine_kernel::interface::Repl;

fn prove(src: &str) -> Result<String, impl std::fmt::Debug> {
    Repl::new().execute(src)
}

// ---- ring ---------------------------------------------------------------------------------

/// The theorem prover certifies the Gaussian-integer norm identity |a+bi|² = a² + b² — the
/// algebraic backbone of the complex modulus that `math.lg`'s `cConj`/`hypot` compute numerically.
/// For 3+4i: 3·3 + 4·4 = 25 = 5². This is the LOGIC-pole companion to the imperative cmath caps.
#[test]
fn ring_certifies_complex_norm_identity() {
    assert!(
        prove("## Theorem: Norm3plus4i\n    Statement: (add (mul 3 3) (mul 4 4)) equals 25.\n    Proof: ring.").is_ok(),
        "ring must certify |3+4i|² = 25"
    );
    // …and REJECT a wrong norm (soundness of the certified complex fact).
    assert!(
        prove("## Theorem: WrongNorm\n    Statement: (add (mul 3 3) (mul 4 4)) equals 24.\n    Proof: ring.").is_err(),
        "ring must reject the false |3+4i|² = 24"
    );
}

#[test]
fn ring_proves_true_closed_equality() {
    assert!(prove("## Theorem: T\n    Statement: 48 equals (mul 6 8).\n    Proof: ring.").is_ok());
}

#[test]
fn ring_rejects_false_closed_equality() {
    let r = prove("## Theorem: F\n    Statement: 48 equals (mul 6 9).\n    Proof: ring.");
    assert!(r.is_err(), "ring must REJECT 48 = 6*9, got {r:?}");
}

#[test]
fn ring_proves_symbolic_identity() {
    assert!(
        prove("## Theorem: C\n    Statement: (add x y) equals (add y x).\n    Proof: ring.")
            .is_ok()
    );
}

#[test]
fn ring_rejects_non_identity() {
    let r = prove("## Theorem: NE\n    Statement: x equals y.\n    Proof: ring.");
    assert!(r.is_err(), "ring must REJECT x = y, got {r:?}");
}

// ---- lia / omega --------------------------------------------------------------------------

#[test]
fn lia_proves_true_inequality() {
    assert!(prove("## Theorem: T\n    Statement: 2 < 5.\n    Proof: lia.").is_ok());
}

#[test]
fn lia_rejects_false_inequality() {
    let r = prove("## Theorem: F\n    Statement: 5 < 2.\n    Proof: lia.");
    assert!(r.is_err(), "lia must REJECT 5 < 2, got {r:?}");
}

#[test]
fn lia_rejects_strict_reflexive() {
    let r = prove("## Theorem: SR\n    Statement: x < x.\n    Proof: lia.");
    assert!(r.is_err(), "lia must REJECT x < x, got {r:?}");
}

#[test]
fn omega_rejects_false_inequality() {
    let r = prove("## Theorem: F\n    Statement: 5 < 2.\n    Proof: omega.");
    assert!(r.is_err(), "omega must REJECT 5 < 2, got {r:?}");
}

// ---- auto genuinely verifies (well-formed goal), and rejects false ------------------------

#[test]
fn auto_proves_wellformed_ring_identity() {
    // `auto` dispatches to ring; with a WELL-FORMED goal (`equals` → `Eq Int A B`) it concludes.
    assert!(
        prove("## Theorem: AC\n    Statement: (add a b) equals (add b a).\n    Proof: auto.")
            .is_ok(),
        "auto should prove a+b = b+a via ring"
    );
}

#[test]
fn auto_rejects_false_goal() {
    let r = prove("## Theorem: AF\n    Statement: 48 equals (mul 6 9).\n    Proof: auto.");
    assert!(r.is_err(), "auto must REJECT the false 48 = 6*9, got {r:?}");
}

// ---- simp decides by computation (unfold user defs + evaluate arithmetic) -----------------

#[test]
fn simp_proves_user_definition_unfold() {
    let mut repl = Repl::new();
    repl.execute("## To double (n: Int) -> Int:\n    Yield (add n n).")
        .unwrap();
    assert!(
        repl.execute("## Theorem: D\n    Statement: (double 3) equals 6.\n    Proof: simp.")
            .is_ok(),
        "simp should decide double(3)=6 by reflection+normalization"
    );
}

#[test]
fn simp_rejects_false_computation() {
    let mut repl = Repl::new();
    repl.execute("## To double (n: Int) -> Int:\n    Yield (add n n).")
        .unwrap();
    let r = repl.execute("## Theorem: DF\n    Statement: (double 3) equals 7.\n    Proof: simp.");
    assert!(
        r.is_err(),
        "simp must REJECT the false double(3)=7, got {r:?}"
    );
}

#[test]
fn simp_proves_hypothesis_guarded_equality() {
    // x = 0 -> x + 1 = 1 : peel the implication, rewrite x := 0, normalize.
    assert!(
        prove("## Theorem: H\n    Statement: (implies (Eq Int x 0) (Eq Int (add x 1) 1)).\n    Proof: simp.").is_ok(),
        "simp should decide x=0 -> x+1=1"
    );
}

#[test]
fn simp_rejects_false_hypothesis_guarded_equality() {
    let r = prove("## Theorem: HF\n    Statement: (implies (Eq Int x 0) (Eq Int (add x 1) 2)).\n    Proof: simp.");
    assert!(r.is_err(), "simp must REJECT x=0 -> x+1=2, got {r:?}");
}

// ---- auto over propositional connectives (And / Or / implies) -----------------------------

#[test]
fn auto_proves_propositional_connectives() {
    assert!(prove("## Theorem: A\n    Statement: (And True True).\n    Proof: auto.").is_ok());
    assert!(prove("## Theorem: O\n    Statement: (Or True True).\n    Proof: auto.").is_ok());
    assert!(
        prove("## Theorem: ImpliesTT\n    Statement: (implies True True).\n    Proof: auto.")
            .is_ok()
    );
    assert!(prove(
        "## Theorem: AE\n    Statement: (And (Eq Int 2 2) (Eq Int 3 3)).\n    Proof: auto."
    )
    .is_ok());
}

#[test]
fn auto_rejects_false_connectives() {
    assert!(prove("## Theorem: AF\n    Statement: (And True False).\n    Proof: auto.").is_err());
    assert!(prove("## Theorem: OF\n    Statement: (Or False False).\n    Proof: auto.").is_err());
    // True -> False is not valid.
    assert!(
        prove("## Theorem: IF\n    Statement: (implies True False).\n    Proof: auto.").is_err()
    );
    assert!(prove(
        "## Theorem: AEF\n    Statement: (And (Eq Int 2 2) (Eq Int 3 4)).\n    Proof: auto."
    )
    .is_err());
}

// ---- negation / disequality (auto and the new `decide` tactic) ----------------------------

#[test]
fn auto_proves_negation_and_disequality() {
    assert!(prove("## Theorem: D\n    Statement: (not (Eq Int 2 3)).\n    Proof: auto.").is_ok());
    assert!(prove("## Theorem: N\n    Statement: (not (Lt 5 2)).\n    Proof: auto.").is_ok());
}

#[test]
fn auto_rejects_false_negation() {
    // not (3 = 3) and not (2 < 5) are FALSE (their inner prop is true).
    assert!(prove("## Theorem: F1\n    Statement: (not (Eq Int 3 3)).\n    Proof: auto.").is_err());
    assert!(prove("## Theorem: F2\n    Statement: (not (Lt 2 5)).\n    Proof: auto.").is_err());
}

#[test]
fn decide_tactic_decides_decidable_propositions() {
    assert!(prove("## Theorem: A\n    Statement: (not (Eq Int 2 3)).\n    Proof: decide.").is_ok());
    assert!(prove("## Theorem: B\n    Statement: (And True True).\n    Proof: decide.").is_ok());
    assert!(prove("## Theorem: C\n    Statement: 3 equals 3.\n    Proof: decide.").is_ok());
}

#[test]
fn decide_tactic_rejects_false_and_undecided() {
    assert!(prove("## Theorem: F1\n    Statement: 3 equals 4.\n    Proof: decide.").is_err());
    assert!(prove("## Theorem: F2\n    Statement: (And True False).\n    Proof: decide.").is_err());
    // Symbolic (undecided) goals are not decidable and must be rejected, not falsely accepted.
    assert!(prove("## Theorem: U\n    Statement: x equals y.\n    Proof: decide.").is_err());
}

// ---- directed introduction tactics: split (∧), left / right (∨) ---------------------------

#[test]
fn split_proves_conjunction() {
    assert!(prove("## Theorem: S\n    Statement: (And True True).\n    Proof: split.").is_ok());
    assert!(prove(
        "## Theorem: S2\n    Statement: (And (Eq Int 2 2) (Lt 3 7)).\n    Proof: split."
    )
    .is_ok());
}

#[test]
fn split_rejects_non_conjunction_and_false_conjunct() {
    assert!(prove("## Theorem: F1\n    Statement: (And True False).\n    Proof: split.").is_err());
    assert!(prove("## Theorem: F2\n    Statement: (Lt 2 5).\n    Proof: split.").is_err());
}

#[test]
fn left_and_right_are_directed() {
    // left proves via the LEFT disjunct; right via the RIGHT.
    assert!(
        prove("## Theorem: L\n    Statement: (Or (Lt 3 7) (Lt 5 2)).\n    Proof: left.").is_ok()
    );
    assert!(
        prove("## Theorem: R\n    Statement: (Or (Lt 5 2) (Lt 3 7)).\n    Proof: right.").is_ok()
    );
    // …and FAIL when the chosen disjunct is false, even though the whole Or holds via the other.
    assert!(
        prove("## Theorem: LF\n    Statement: (Or (Lt 5 2) (Lt 3 7)).\n    Proof: left.").is_err()
    );
    assert!(
        prove("## Theorem: RF\n    Statement: (Or (Lt 3 7) (Lt 5 2)).\n    Proof: right.").is_err()
    );
}

// ---- hypothesis context: equality-rewrite into any conclusion + the `assumption` tactic ---

#[test]
fn implication_rewrites_equality_hypothesis_into_any_conclusion() {
    // x = 5 -> x < 10 : assume the equality, rewrite the (non-equality!) conclusion, decide 5 < 10.
    assert!(prove(
        "## Theorem: R\n    Statement: (implies (Eq Int x 5) (Lt x 10)).\n    Proof: auto."
    )
    .is_ok());
    assert!(prove("## Theorem: R2\n    Statement: (implies (Eq Int x 5) (not (Eq Int x 6))).\n    Proof: auto.").is_ok());
}

#[test]
fn implication_rejects_false_under_hypothesis() {
    // x = 5 -> x < 3 is false at x = 5, so it is not a theorem.
    assert!(prove(
        "## Theorem: F\n    Statement: (implies (Eq Int x 5) (Lt x 3)).\n    Proof: auto."
    )
    .is_err());
}

#[test]
fn assumption_tactic_closes_by_hypothesis() {
    assert!(prove(
        "## Theorem: A\n    Statement: (implies (Lt x y) (Lt x y)).\n    Proof: assumption."
    )
    .is_ok());
    assert!(prove("## Theorem: W\n    Statement: (implies (Lt a b) (implies (Lt x y) (Lt a b))).\n    Proof: assumption.").is_ok());
}

#[test]
fn assumption_tactic_is_directed() {
    // Directed: `assumption` proves ONLY by a hypothesis, not by general decidability.
    assert!(
        prove("## Theorem: T\n    Statement: (And True True).\n    Proof: assumption.").is_err()
    );
    assert!(prove(
        "## Theorem: N\n    Statement: (implies (Lt x y) (Lt a b)).\n    Proof: assumption."
    )
    .is_err());
}

// ---- contradiction: jointly-unsatisfiable hypotheses ⇒ vacuously true (any conclusion) ----

#[test]
fn contradictory_hypotheses_prove_any_conclusion() {
    // x = 5 AND x = 6 is unsatisfiable, so the implication holds for ANY (even symbolic) conclusion.
    assert!(prove("## Theorem: C\n    Statement: (implies (Eq Int x 5) (implies (Eq Int x 6) (Lt a b))).\n    Proof: auto.").is_ok());
    assert!(prove("## Theorem: C2\n    Statement: (implies (Eq Int x 1) (implies (Eq Int x 2) (Eq Int p q))).\n    Proof: auto.").is_ok());
    // A single decidably-false hypothesis is the degenerate case (already vacuous).
    assert!(prove(
        "## Theorem: C3\n    Statement: (implies (Lt 5 2) (Eq Int p q)).\n    Proof: auto."
    )
    .is_ok());
}

#[test]
fn consistent_hypotheses_do_not_prove_a_symbolic_conclusion() {
    // SOUNDNESS GUARD: x=5 and y=6 are consistent, so `Lt a b` does NOT follow — must be rejected.
    assert!(prove("## Theorem: S\n    Statement: (implies (Eq Int x 5) (implies (Eq Int y 6) (Lt a b))).\n    Proof: auto.").is_err());
    // And a hypothesis that is TRUE at the substituted point must not be treated as contradictory.
    assert!(prove("## Theorem: S2\n    Statement: (implies (Eq Int x 5) (implies (Eq Int x 5) (Lt a b))).\n    Proof: auto.").is_err());
}

// ---- the gate is scoped to theorem proofs, NOT raw derivation objects ---------------------

#[test]
fn raw_error_derivation_definition_is_still_allowed() {
    // The derivation layer intentionally lets you DEFINE an error derivation as a first-class
    // value (failure is inspected separately via `concludes`). The soundness gate must NOT fire
    // here — only on `## Theorem … Proof:` proofs.
    let mut repl = Repl::new();
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) (SLit 48)) (SApp (SApp (SName \"mul\") (SLit 6)) (SLit 9)).")
        .expect("define goal");
    let r = repl.execute("Definition d : Derivation := try_ring goal.");
    assert!(
        r.is_ok(),
        "raw `d : Derivation := try_ring goal` must remain allowed, got {r:?}"
    );
}
