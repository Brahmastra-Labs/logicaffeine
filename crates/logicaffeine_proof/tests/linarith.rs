//! Linear arithmetic over `Int`, end-to-end through the prover and kernel-certified.
//! An inequality `a ≤ b` is the Prop `le(a, b) = true`; the engine chains the known
//! `≤` facts transitively and reconstructs the proof from the `le_trans`/`le_refl`
//! order axioms (each kernel-rechecked). Symbolic operands are typed `Int` by the
//! arithmetic typing bridge. `≤` is DIRECTED — these tests pin both that real chains
//! are found and that unsound non-chains (e.g. symmetry) are refused.

use logicaffeine_proof::verify::prove_certify_check;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn c(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn add(x: ProofTerm, y: ProofTerm) -> ProofTerm {
    ProofTerm::Function("add".to_string(), vec![x, y])
}
/// `a ≤ b`
fn le(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(
        ProofTerm::Function("le".to_string(), vec![a, b]),
        ProofTerm::Constant("true".to_string()),
    )
}

#[test]
fn le_transitivity_two_steps() {
    // x ≤ y, y ≤ z ⊢ x ≤ z
    let r = prove_certify_check(
        &[le(c("x"), c("y")), le(c("y"), c("z"))],
        &le(c("x"), c("z")),
    );
    assert!(r.verified, "x≤y, y≤z ⊢ x≤z: {:?}", r.verification_error);
}

#[test]
fn le_transitivity_long_chain() {
    // a ≤ b ≤ m ≤ d ⊢ a ≤ d
    let r = prove_certify_check(
        &[le(c("a"), c("b")), le(c("b"), c("m")), le(c("m"), c("d"))],
        &le(c("a"), c("d")),
    );
    assert!(r.verified, "a≤b≤m≤d ⊢ a≤d: {:?}", r.verification_error);
}

#[test]
fn le_direct_hypothesis() {
    let r = prove_certify_check(&[le(c("p"), c("q"))], &le(c("p"), c("q")));
    assert!(r.verified, "p≤q ⊢ p≤q: {:?}", r.verification_error);
}

#[test]
fn le_reflexivity() {
    // ⊢ w ≤ w  (no hypotheses, by le_refl)
    let r = prove_certify_check(&[], &le(c("w"), c("w")));
    assert!(r.verified, "⊢ w≤w: {:?}", r.verification_error);
}

#[test]
fn le_is_not_symmetric() {
    // x ≤ y must NOT prove y ≤ x.
    let r = prove_certify_check(&[le(c("x"), c("y"))], &le(c("y"), c("x")));
    assert!(!r.verified, "y≤x must NOT follow from x≤y (≤ is directed)");
}

#[test]
fn le_no_spurious_link() {
    // x ≤ y and a ≤ b (disconnected) must NOT prove x ≤ b.
    let r = prove_certify_check(
        &[le(c("x"), c("y")), le(c("a"), c("b"))],
        &le(c("x"), c("b")),
    );
    assert!(!r.verified, "x≤b must NOT follow from x≤y and a≤b (no chain)");
}

#[test]
fn ground_le_holds_by_computation() {
    // ⊢ 2 ≤ 5  — decided by computation (le 2 5 ⇝ true), no hypotheses.
    let r = prove_certify_check(&[], &le(c("2"), c("5")));
    assert!(r.verified, "2 ≤ 5 by computation: {:?}", r.verification_error);
}

#[test]
fn ground_le_reflexive() {
    let r = prove_certify_check(&[], &le(c("7"), c("7")));
    assert!(r.verified, "7 ≤ 7: {:?}", r.verification_error);
}

#[test]
fn ground_le_false_is_unprovable() {
    // ⊢ 5 ≤ 2  — false (le 5 2 ⇝ false), must NOT be provable.
    let r = prove_certify_check(&[], &le(c("5"), c("2")));
    assert!(!r.verified, "5 ≤ 2 must be unprovable (it is false)");
}

#[test]
fn mixed_ground_and_symbolic_chain() {
    // 2 ≤ x, x ≤ y ⊢ 2 ≤ y  — a literal start chained with symbolic Int facts.
    let r = prove_certify_check(
        &[le(c("2"), c("x")), le(c("x"), c("y"))],
        &le(c("2"), c("y")),
    );
    assert!(r.verified, "2≤x, x≤y ⊢ 2≤y: {:?}", r.verification_error);
}

#[test]
fn le_add_mono_two_inequalities() {
    // a ≤ b, p ≤ q ⊢ a + p ≤ b + q
    let r = prove_certify_check(
        &[le(c("a"), c("b")), le(c("p"), c("q"))],
        &le(add(c("a"), c("p")), add(c("b"), c("q"))),
    );
    assert!(r.verified, "a≤b, p≤q ⊢ a+p ≤ b+q: {:?}", r.verification_error);
}

#[test]
fn le_add_mono_nested() {
    // a ≤ b, c ≤ d, e ≤ f ⊢ (a + c) + e ≤ (b + d) + f
    let r = prove_certify_check(
        &[
            le(c("a"), c("b")),
            le(c("cc"), c("dd")),
            le(c("e"), c("f")),
        ],
        &le(
            add(add(c("a"), c("cc")), c("e")),
            add(add(c("b"), c("dd")), c("f")),
        ),
    );
    assert!(r.verified, "nested addition: {:?}", r.verification_error);
}

#[test]
fn le_add_mono_with_ground_operands() {
    // 2 ≤ 3, 4 ≤ 5 ⊢ 2 + 4 ≤ 3 + 5  (ground operands decided by computation)
    let r = prove_certify_check(&[], &le(add(c("2"), c("4")), add(c("3"), c("5"))));
    assert!(r.verified, "2+4 ≤ 3+5: {:?}", r.verification_error);
}

fn happy(who: &str) -> ProofExpr {
    ProofExpr::Predicate {
        name: "happy".to_string(),
        args: vec![c(who)],
        world: None,
    }
}

#[test]
fn contradictory_bounds_prove_anything() {
    // 5 ≤ x, x ≤ 3  ⟹  5 ≤ 3 (false)  ⟹  ⊥  ⟹  any goal (ex falso).
    let r = prove_certify_check(&[le(c("5"), c("x")), le(c("x"), c("3"))], &happy("Bob"));
    assert!(
        r.verified,
        "contradictory bounds 5≤x≤3 should prove anything: {:?}",
        r.verification_error
    );
}

#[test]
fn consistent_bounds_do_not_prove_unrelated_goal() {
    // 1 ≤ x, x ≤ 5 is consistent — must NOT prove an arbitrary unrelated goal.
    let r = prove_certify_check(&[le(c("1"), c("x")), le(c("x"), c("5"))], &happy("Bob"));
    assert!(
        !r.verified,
        "consistent bounds must NOT prove an unrelated goal"
    );
}

fn mul(k: &str, x: ProofTerm) -> ProofTerm {
    ProofTerm::Function("mul".to_string(), vec![c(k), x])
}

#[test]
fn farkas_scaling_contradiction_proves_anything() {
    // 2 ≤ x and 2x ≤ 3  ⟹  4 ≤ 2x ≤ 3  ⟹  4 ≤ 3 (false)  ⟹  anything.
    // Requires SCALING `2 ≤ x` by 2 (Farkas multiplier 2) — beyond a plain ≤-chain.
    let r = prove_certify_check(
        &[le(c("2"), c("x")), le(mul("2", c("x")), c("3"))],
        &happy("Bob"),
    );
    assert!(
        r.verified,
        "2≤x, 2x≤3 (needs scaling) should prove anything: {:?}",
        r.verification_error
    );
}

#[test]
fn farkas_consistent_scaled_system_is_not_a_contradiction() {
    // 2 ≤ x, 2x ≤ 5  ⟹  4 ≤ 2x ≤ 5  — consistent (x = 2 works). No contradiction.
    let r = prove_certify_check(
        &[le(c("2"), c("x")), le(mul("2", c("x")), c("5"))],
        &happy("Bob"),
    );
    assert!(
        !r.verified,
        "2≤x, 2x≤5 is consistent — must NOT prove an unrelated goal"
    );
}

#[test]
fn farkas_multiplier_three() {
    // 2 ≤ x, 3x ≤ 5  ⟹  6 ≤ 3x ≤ 5  ⟹  6 ≤ 5 (false). Needs multiplier 3 on `2 ≤ x`.
    let r = prove_certify_check(
        &[le(c("2"), c("x")), le(mul("3", c("x")), c("5"))],
        &happy("Bob"),
    );
    assert!(r.verified, "2≤x, 3x≤5 (multiplier 3): {:?}", r.verification_error);
}

#[test]
fn three_variable_contradiction_with_ground_endpoints() {
    // x ≤ 2, 3 ≤ y, y ≤ x  ⟹  3 ≤ y ≤ x ≤ 2  ⟹  3 ≤ 2 (false): a three-variable
    // contradiction closed end-to-end (the ≤-chain reaches the ground-false 3 ≤ 2).
    let r = prove_certify_check(
        &[le(c("x"), c("2")), le(c("3"), c("y")), le(c("y"), c("x"))],
        &happy("Bob"),
    );
    assert!(
        r.verified,
        "x≤2, 3≤y, y≤x ⊢ ⊥: {:?}",
        r.verification_error
    );
}

#[test]
fn farkas_mixed_coefficients_two_vars() {
    // 3x + 2y ≤ 5, 1 ≤ x, 1 ≤ y  ⟹  3·1 + 2·1 = 5 ≤ 3x+2y ≤ 5 is tight; push it:
    // 2 ≤ x, 1 ≤ y, 2x + y ≤ 4  ⟹  2·2 + 1 = 5 ≤ 2x+y ≤ 4  ⟹  5 ≤ 4 (false).
    let two_x_plus_y = add(mul("2", c("x")), c("y"));
    let r = prove_certify_check(
        &[le(c("2"), c("x")), le(c("1"), c("y")), le(two_x_plus_y, c("4"))],
        &happy("Bob"),
    );
    assert!(
        r.verified,
        "2≤x, 1≤y, 2x+y≤4: {:?}",
        r.verification_error
    );
}

#[test]
fn farkas_consistent_three_variable_cycle() {
    // x ≤ y, y ≤ z, z ≤ x  — consistent (x = y = z). Must NOT prove an unrelated goal.
    let r = prove_certify_check(
        &[le(c("x"), c("y")), le(c("y"), c("z")), le(c("z"), c("x"))],
        &happy("Bob"),
    );
    assert!(!r.verified, "x≤y≤z≤x is consistent — must NOT prove anything");
}
