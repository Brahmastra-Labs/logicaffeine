//! =============================================================================
//! PHASE 102: ONE DOOR — UNIFIED THEOREM VERIFICATION
//! =============================================================================
//!
//! Every public theorem entry point must give the SAME trust guarantee: a proof
//! is reported as verified IFF it was certified AND kernel type-checked. A
//! derivation alone (backward-chaining success) must NEVER be presented as
//! verified. No door may accept what another rejects.
//!
//! Entry points under test:
//!   - logicaffeine_compile::verify_theorem          (-> Result<(Term, Context)>)
//!   - logicaffeine_compile::compile_theorem_for_ui  (-> TheoremCompileResult)
//!   - logicaffeine_language::compile_theorem         (-> Result<String>)
//!   - logicaffeine_proof::verify::prove_certify_check (the shared core)

use logicaffeine_compile::{compile_theorem_for_ui, verify_theorem};
use logicaffeine_kernel::infer_type;
use logicaffeine_language::compile_theorem;

const VALID_SOCRATES: &str = r#"
## Theorem: Socrates_Mortality_OneDoor
Given: Socrates is a man.
Given: Every man is mortal.
Prove: Socrates is mortal.
Proof: Auto.
"#;

const VALID_CHAIN: &str = r#"
## Theorem: Socrates_Doom_OneDoor
Given: Socrates is a man.
Given: Every man is mortal.
Given: Every mortal is doomed.
Prove: Socrates is doomed.
Proof: Auto.
"#;

const INVALID_MISSING_PREMISE: &str = r#"
## Theorem: Incomplete_OneDoor
Given: Every man is mortal.
Prove: Socrates is mortal.
Proof: Auto.
"#;

// -----------------------------------------------------------------------------
// A. A valid theorem is kernel-checked by EVERY door.
// -----------------------------------------------------------------------------
#[test]
fn socrates_all_entry_points_kernel_check() {
    // verify_theorem: returns a term that independently type-checks.
    let vt = verify_theorem(VALID_SOCRATES);
    assert!(vt.is_ok(), "verify_theorem rejected a valid proof: {:?}", vt);
    let (term, ctx) = vt.unwrap();
    assert!(
        infer_type(&ctx, &term).is_ok(),
        "verify_theorem returned a term that does not type-check"
    );

    // compile_theorem_for_ui: the verified flag is the honest signal.
    let ui = compile_theorem_for_ui(VALID_SOCRATES);
    assert!(ui.verified, "UI door did not report a valid proof as verified");
    assert!(
        ui.verification_error.is_none(),
        "UI door reported a verification error on a valid proof: {:?}",
        ui.verification_error
    );
    assert!(ui.derivation.is_some(), "UI door lost the derivation");
    assert!(ui.error.is_none(), "UI door reported a parse error: {:?}", ui.error);

    // compile_theorem (String door): only says "Proved" when kernel-checked.
    let ct = compile_theorem(VALID_SOCRATES);
    assert!(ct.is_ok(), "String door rejected a valid proof: {:?}", ct);
    assert!(
        ct.unwrap().contains("Proved"),
        "String door did not report the valid proof as proved"
    );
}

// -----------------------------------------------------------------------------
// B. Multi-step chain reasoning is kernel-checked by every door.
// -----------------------------------------------------------------------------
#[test]
fn chain_reasoning_all_entry_points() {
    let vt = verify_theorem(VALID_CHAIN);
    assert!(vt.is_ok(), "verify_theorem rejected the chain proof: {:?}", vt);
    let (term, ctx) = vt.unwrap();
    assert!(infer_type(&ctx, &term).is_ok());

    let ui = compile_theorem_for_ui(VALID_CHAIN);
    assert!(ui.verified, "UI door did not verify the chain proof");
    assert!(ui.verification_error.is_none());

    let ct = compile_theorem(VALID_CHAIN);
    assert!(ct.is_ok() && ct.unwrap().contains("Proved"));
}

// -----------------------------------------------------------------------------
// C. An unprovable theorem is rejected UNIFORMLY — no door accepts it.
// -----------------------------------------------------------------------------
#[test]
fn invalid_rejected_by_every_door() {
    let vt = verify_theorem(INVALID_MISSING_PREMISE);
    assert!(vt.is_err(), "verify_theorem accepted an unprovable theorem");

    let ui = compile_theorem_for_ui(INVALID_MISSING_PREMISE);
    assert!(!ui.verified, "UI door reported an unprovable theorem as verified");

    let ct = compile_theorem(INVALID_MISSING_PREMISE);
    assert!(ct.is_err(), "String door accepted an unprovable theorem");
}

// -----------------------------------------------------------------------------
// D. The UI `verified` flag is consistent with the kernel-checking door for
//    BOTH valid and invalid inputs — the core anti-divergence property.
// -----------------------------------------------------------------------------
#[test]
fn ui_verified_flag_consistent_with_verify_theorem() {
    for input in [VALID_SOCRATES, VALID_CHAIN, INVALID_MISSING_PREMISE] {
        let ui_verified = compile_theorem_for_ui(input).verified;
        let vt_ok = verify_theorem(input).is_ok();
        assert_eq!(
            ui_verified, vt_ok,
            "UI `verified` ({}) disagreed with verify_theorem ({}) on the same input",
            ui_verified, vt_ok
        );
    }
}

// -----------------------------------------------------------------------------
// E. `verified == true` is never a bare boolean — it is backed by a term that
//    re-checks in the kernel. The shared core proves this directly.
// -----------------------------------------------------------------------------
#[test]
fn shared_core_verified_is_independently_recheckable() {
    use logicaffeine_proof::verify::prove_certify_check;
    use logicaffeine_proof::{ProofExpr, ProofTerm};

    // Socrates syllogism, built directly at the ProofExpr layer:
    //   man(Socrates);  ∀x. man(x) → mortal(x);  ⊢ mortal(Socrates)
    let premise_fact = ProofExpr::Predicate {
        name: "man".to_string(),
        args: vec![ProofTerm::Constant("Socrates".to_string())],
        world: None,
    };
    let premise_rule = ProofExpr::ForAll {
        variable: "x".to_string(),
        body: Box::new(ProofExpr::Implies(
            Box::new(ProofExpr::Predicate {
                name: "man".to_string(),
                args: vec![ProofTerm::Variable("x".to_string())],
                world: None,
            }),
            Box::new(ProofExpr::Predicate {
                name: "mortal".to_string(),
                args: vec![ProofTerm::Variable("x".to_string())],
                world: None,
            }),
        )),
    };
    let goal = ProofExpr::Predicate {
        name: "mortal".to_string(),
        args: vec![ProofTerm::Constant("Socrates".to_string())],
        world: None,
    };

    let result = prove_certify_check(&[premise_fact, premise_rule], &goal);

    assert!(result.verified, "core failed to verify the syllogism: {:?}", result.verification_error);
    let term = result.proof_term.expect("verified proof must carry a term");
    // Independent re-check in the returned context — the guarantee is real.
    assert!(
        infer_type(&result.kernel_ctx, &term).is_ok(),
        "core reported verified but the term does not type-check"
    );
}
