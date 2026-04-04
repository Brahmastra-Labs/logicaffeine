//! Sprint 4: Z3 Equivalence Engine
//!
//! Tests for the hardware verification pipeline: structural equivalence,
//! bounded translation, and the full pipeline API.
//! Z3-dependent tests are behind #[cfg(feature = "verification")].

use logicaffeine_compile::codegen_sva::hw_pipeline::{
    check_structural_equivalence, check_bounded_equivalence,
    translate_sva_to_bounded, translate_spec_to_bounded,
    compile_hw_spec, emit_hw_sva, EquivalenceResult,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_compile::codegen_sva::SvaAssertionKind;

// ═══════════════════════════════════════════════════════════════════════════
// STRUCTURAL EQUIVALENCE (no Z3)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn structural_equiv_identical_mutex() {
    let result = check_structural_equivalence(
        "!(grant_a && grant_b)",
        "!(grant_a && grant_b)",
    )
    .unwrap();
    assert!(result, "Identical mutex SVAs should be structurally equivalent");
}

#[test]
fn structural_equiv_different_signals_fail() {
    let result = check_structural_equivalence(
        "req |-> ack",
        "req |-> done",
    )
    .unwrap();
    assert!(!result, "Different signals should not be equivalent");
}

#[test]
fn structural_equiv_different_operators_fail() {
    let result = check_structural_equivalence(
        "req |-> ack",
        "req |=> ack",
    )
    .unwrap();
    assert!(!result, "Different implication types should not be equivalent");
}

// ═══════════════════════════════════════════════════════════════════════════
// BOUNDED TRANSLATION (no Z3)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn translate_sva_to_bounded_mutex() {
    let result = translate_sva_to_bounded("!(grant_a && grant_b)", 5).unwrap();
    // Should produce a conjunction of 5 timesteps, each with !(ga && gb)
    let leaf_count = logicaffeine_compile::codegen_sva::sva_to_verify::count_and_leaves(&result.expr);
    assert_eq!(leaf_count, 5, "Mutex at bound=5 should have 5 And-leaves");
}

#[test]
fn translate_spec_to_bounded_always() {
    let result = translate_spec_to_bounded("Always, every dog runs.", 3).unwrap();
    let leaf_count = logicaffeine_compile::codegen_sva::sva_to_verify::count_and_leaves(&result.expr);
    assert!(leaf_count >= 3, "G at bound=3 should have >= 3 And-leaves, got {}", leaf_count);
}

// ═══════════════════════════════════════════════════════════════════════════
// BOUNDED EQUIVALENCE (structural, no Z3)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bounded_equiv_identical_exprs() {
    let a = BoundedExpr::And(
        Box::new(BoundedExpr::Var("req@0".into())),
        Box::new(BoundedExpr::Var("req@1".into())),
    );
    let b = BoundedExpr::And(
        Box::new(BoundedExpr::Var("req@0".into())),
        Box::new(BoundedExpr::Var("req@1".into())),
    );
    let result = check_bounded_equivalence(&a, &b, 2);
    assert!(result.equivalent, "Identical bounded exprs should be equivalent");
}

#[test]
fn bounded_equiv_different_exprs() {
    let a = BoundedExpr::Var("req@0".into());
    let b = BoundedExpr::Var("ack@0".into());
    let result = check_bounded_equivalence(&a, &b, 1);
    assert!(!result.equivalent, "Different vars should not be equivalent");
}

// ═══════════════════════════════════════════════════════════════════════════
// COMPILE + EMIT PUBLIC API
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn compile_hw_spec_returns_fol() {
    let fol = compile_hw_spec("Every dog runs.").unwrap();
    assert!(fol.contains("Run") || fol.contains("run"), "FOL should contain Run predicate");
}

#[test]
fn emit_hw_sva_generates_property() {
    let sva = emit_hw_sva("Mutex", "clk", "!(grant_a && grant_b)", SvaAssertionKind::Assert);
    assert!(sva.contains("property p_mutex"), "Should contain property name");
    assert!(sva.contains("@(posedge clk)"), "Should contain clock edge");
    assert!(sva.contains("assert property"), "Should have assert wrapper");
}

// ═══════════════════════════════════════════════════════════════════════════
// END-TO-END: SPEC → BOUNDED → SVA → BOUNDED → COMPARE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_spec_and_sva_translate_to_bounded() {
    // Both sides should translate without error
    let spec_bounded = translate_spec_to_bounded("Always, John runs.", 3);
    assert!(spec_bounded.is_ok(), "Spec should translate: {:?}", spec_bounded.err());

    let sva_bounded = translate_sva_to_bounded("req |-> ack", 3);
    assert!(sva_bounded.is_ok(), "SVA should translate: {:?}", sva_bounded.err());
}

#[test]
fn e2e_sva_parse_error_propagates() {
    let result = translate_sva_to_bounded("|||invalid|||", 5);
    assert!(result.is_err(), "Invalid SVA should error");
}

// ═══════════════════════════════════════════════════════════════════════════
// Z3 SEMANTIC EQUIVALENCE — THE CORE CONTRIBUTION
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "verification")]
mod z3_semantic {
    use logicaffeine_verify::equivalence::{check_equivalence, EquivalenceResult};
    use logicaffeine_verify::{VerifyExpr, VerifyOp};
    use logicaffeine_compile::codegen_sva::sva_to_verify::bounded_to_verify;
    use logicaffeine_compile::codegen_sva::hw_pipeline::translate_sva_to_bounded;

    // ═══════════════════════════════════════════════════
    // SIMPLE BOOLEAN EQUIVALENCE
    // ═══════════════════════════════════════════════════

    #[test]
    fn z3_identical_formulas_are_equivalent() {
        let expr = VerifyExpr::binary(VerifyOp::Implies,
            VerifyExpr::Var("req@0".into()),
            VerifyExpr::Var("ack@0".into()),
        );
        let result = check_equivalence(&expr, &expr, &["req".into(), "ack".into()], 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "Identical formulas must be equivalent. Got: {:?}", result);
    }

    #[test]
    fn z3_different_formulas_are_not_equivalent() {
        // req@0 → ack@0 ≢ ack@0 → req@0
        let fol = VerifyExpr::binary(VerifyOp::Implies,
            VerifyExpr::Var("req@0".into()),
            VerifyExpr::Var("ack@0".into()),
        );
        let sva = VerifyExpr::binary(VerifyOp::Implies,
            VerifyExpr::Var("ack@0".into()),
            VerifyExpr::Var("req@0".into()),
        );
        let result = check_equivalence(&fol, &sva, &["req".into(), "ack".into()], 1);
        assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "req→ack must NOT equal ack→req. Got: {:?}", result);
    }

    #[test]
    fn z3_tautology_equivalent_to_true() {
        // (p ∨ ¬p) ≡ true
        let taut = VerifyExpr::binary(VerifyOp::Or,
            VerifyExpr::Var("p@0".into()),
            VerifyExpr::not(VerifyExpr::Var("p@0".into())),
        );
        let t = VerifyExpr::Bool(true);
        let result = check_equivalence(&taut, &t, &["p".into()], 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "(p ∨ ¬p) must equal true. Got: {:?}", result);
    }

    #[test]
    fn z3_contradiction_not_equivalent_to_true() {
        // (p ∧ ¬p) ≢ true
        let contra = VerifyExpr::binary(VerifyOp::And,
            VerifyExpr::Var("p@0".into()),
            VerifyExpr::not(VerifyExpr::Var("p@0".into())),
        );
        let t = VerifyExpr::Bool(true);
        let result = check_equivalence(&contra, &t, &["p".into()], 1);
        assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "(p ∧ ¬p) must NOT equal true. Got: {:?}", result);
    }

    // ═══════════════════════════════════════════════════
    // COUNTEREXAMPLE EXTRACTION
    // ═══════════════════════════════════════════════════

    #[test]
    fn z3_counterexample_has_signal_values() {
        let fol = VerifyExpr::binary(VerifyOp::Implies,
            VerifyExpr::Var("req@0".into()),
            VerifyExpr::Var("ack@0".into()),
        );
        let sva = VerifyExpr::Var("ack@0".into()); // too weak — bare signal
        let result = check_equivalence(&fol, &sva, &["req".into(), "ack".into()], 1);
        match result {
            EquivalenceResult::NotEquivalent { counterexample } => {
                assert!(!counterexample.cycles.is_empty(),
                    "Counterexample must have at least one cycle");
                let first = &counterexample.cycles[0];
                assert!(!first.signals.is_empty(),
                    "Counterexample cycle must have signal assignments");
            }
            _ => panic!("req→ack should NOT equal bare ack. Got: {:?}", result),
        }
    }

    #[test]
    fn z3_counterexample_identifies_divergent_signals() {
        // FOL: req@0 → ack@0 (if req then ack)
        // SVA: ack@0 → req@0 (if ack then req) — REVERSED
        let fol = VerifyExpr::binary(VerifyOp::Implies,
            VerifyExpr::Var("req@0".into()),
            VerifyExpr::Var("ack@0".into()),
        );
        let sva = VerifyExpr::binary(VerifyOp::Implies,
            VerifyExpr::Var("ack@0".into()),
            VerifyExpr::Var("req@0".into()),
        );
        let result = check_equivalence(&fol, &sva, &["req".into(), "ack".into()], 1);
        match result {
            EquivalenceResult::NotEquivalent { counterexample } => {
                let cycle = &counterexample.cycles[0];
                // The counterexample should have values for req and ack
                assert!(cycle.signals.contains_key("req"),
                    "Counterexample must include req. Got signals: {:?}", cycle.signals);
                assert!(cycle.signals.contains_key("ack"),
                    "Counterexample must include ack. Got signals: {:?}", cycle.signals);
            }
            _ => panic!("Should be not equivalent"),
        }
    }

    // ═══════════════════════════════════════════════════
    // SVA PIPELINE EQUIVALENCE
    // ═══════════════════════════════════════════════════

    #[test]
    fn z3_identical_sva_are_semantically_equivalent() {
        // Same SVA parsed twice → should be equivalent
        let sva_a = translate_sva_to_bounded("!(grant_a && grant_b)", 3).unwrap();
        let sva_b = translate_sva_to_bounded("!(grant_a && grant_b)", 3).unwrap();
        let verify_a = bounded_to_verify(&sva_a.expr);
        let verify_b = bounded_to_verify(&sva_b.expr);
        let result = check_equivalence(
            &verify_a, &verify_b,
            &["grant_a".into(), "grant_b".into()], 3,
        );
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "Same SVA must be semantically equivalent. Got: {:?}", result);
    }

    #[test]
    fn z3_implication_direction_matters() {
        // req |-> ack ≢ ack |-> req
        let sva_a = translate_sva_to_bounded("req |-> ack", 3).unwrap();
        let sva_b = translate_sva_to_bounded("ack |-> req", 3).unwrap();
        let verify_a = bounded_to_verify(&sva_a.expr);
        let verify_b = bounded_to_verify(&sva_b.expr);
        let result = check_equivalence(
            &verify_a, &verify_b,
            &["req".into(), "ack".into()], 3,
        );
        assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "req|->ack must NOT equal ack|->req. Got: {:?}", result);
    }

    #[test]
    fn z3_overlapping_vs_nonoverlapping_implication() {
        // req |-> ack ≢ req |=> ack (different timestep semantics)
        let sva_a = translate_sva_to_bounded("req |-> ack", 3).unwrap();
        let sva_b = translate_sva_to_bounded("req |=> ack", 3).unwrap();
        let verify_a = bounded_to_verify(&sva_a.expr);
        let verify_b = bounded_to_verify(&sva_b.expr);
        let result = check_equivalence(
            &verify_a, &verify_b,
            &["req".into(), "ack".into()], 3,
        );
        assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "|-> must differ from |=> (overlapping vs non-overlapping). Got: {:?}", result);
    }

    #[test]
    fn z3_s_eventually_differs_from_immediate() {
        // req |-> s_eventually(ack) ≢ req |-> ack
        let sva_a = translate_sva_to_bounded("req |-> s_eventually(ack)", 5).unwrap();
        let sva_b = translate_sva_to_bounded("req |-> ack", 5).unwrap();
        let verify_a = bounded_to_verify(&sva_a.expr);
        let verify_b = bounded_to_verify(&sva_b.expr);
        let result = check_equivalence(
            &verify_a, &verify_b,
            &["req".into(), "ack".into()], 5,
        );
        assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "s_eventually(ack) should differ from immediate ack. Got: {:?}", result);
    }

    // ═══════════════════════════════════════════════════
    // DE MORGAN EQUIVALENCE — Z3 PROVES BOOLEAN LAWS
    // ═══════════════════════════════════════════════════

    #[test]
    fn z3_de_morgan_not_and_equiv_or_not() {
        // !(a && b) ≡ (!a || !b) — Z3 should confirm
        let lhs = translate_sva_to_bounded("!(grant_a && grant_b)", 3).unwrap();
        let rhs_a = translate_sva_to_bounded("!grant_a || !grant_b", 3).unwrap();
        let verify_lhs = bounded_to_verify(&lhs.expr);
        let verify_rhs = bounded_to_verify(&rhs_a.expr);
        let result = check_equivalence(
            &verify_lhs, &verify_rhs,
            &["grant_a".into(), "grant_b".into()], 3,
        );
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "De Morgan: !(a&&b) must equal (!a||!b). Got: {:?}", result);
    }

    #[test]
    fn z3_de_morgan_not_or_equiv_and_not() {
        // !(a || b) ≡ (!a && !b)
        let lhs = translate_sva_to_bounded("!(req || ack)", 3).unwrap();
        let rhs = translate_sva_to_bounded("!req && !ack", 3).unwrap();
        let verify_lhs = bounded_to_verify(&lhs.expr);
        let verify_rhs = bounded_to_verify(&rhs.expr);
        let result = check_equivalence(
            &verify_lhs, &verify_rhs,
            &["req".into(), "ack".into()], 3,
        );
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "De Morgan: !(a||b) must equal (!a&&!b). Got: {:?}", result);
    }

    #[test]
    fn z3_implication_equiv_not_or() {
        // (a → b) ≡ (¬a ∨ b)
        let lhs = translate_sva_to_bounded("req |-> ack", 3).unwrap();
        let rhs = translate_sva_to_bounded("!req || ack", 3).unwrap();
        let verify_lhs = bounded_to_verify(&lhs.expr);
        let verify_rhs = bounded_to_verify(&rhs.expr);
        let result = check_equivalence(
            &verify_lhs, &verify_rhs,
            &["req".into(), "ack".into()], 3,
        );
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "Implication: (a→b) must equal (¬a∨b). Got: {:?}", result);
    }

    #[test]
    fn z3_double_negation_elimination() {
        // !!p ≡ p
        let lhs = translate_sva_to_bounded("!!valid", 3).unwrap();
        let rhs = translate_sva_to_bounded("valid", 3).unwrap();
        let verify_lhs = bounded_to_verify(&lhs.expr);
        let verify_rhs = bounded_to_verify(&rhs.expr);
        let result = check_equivalence(
            &verify_lhs, &verify_rhs,
            &["valid".into()], 3,
        );
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "!!p must equal p. Got: {:?}", result);
    }

    // ═══════════════════════════════════════════════════
    // PROTOCOL PATTERN EQUIVALENCE
    // ═══════════════════════════════════════════════════

    #[test]
    fn z3_mutex_two_formulations_equivalent() {
        // Two ways to express mutex: !(a && b) vs (!a || !b)
        let form1 = translate_sva_to_bounded("!(grant_a && grant_b)", 5).unwrap();
        let form2 = translate_sva_to_bounded("!grant_a || !grant_b", 5).unwrap();
        let verify1 = bounded_to_verify(&form1.expr);
        let verify2 = bounded_to_verify(&form2.expr);
        let result = check_equivalence(
            &verify1, &verify2,
            &["grant_a".into(), "grant_b".into()], 5,
        );
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "Mutex formulations must be equivalent. Got: {:?}", result);
    }

    #[test]
    fn z3_weaker_bound_not_equiv_to_stronger() {
        // req |-> ##[1:3] ack ≢ req |-> ##[1:5] ack
        // (the 1:5 version is strictly weaker — allows ack at cycle 4 or 5)
        let strong = translate_sva_to_bounded("req |-> ##[1:3] ack", 5).unwrap();
        let weak = translate_sva_to_bounded("req |-> ##[1:5] ack", 5).unwrap();
        let verify_strong = bounded_to_verify(&strong.expr);
        let verify_weak = bounded_to_verify(&weak.expr);
        let result = check_equivalence(
            &verify_strong, &verify_weak,
            &["req".into(), "ack".into()], 5,
        );
        assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "##[1:3] must NOT equal ##[1:5]. Got: {:?}", result);
    }

    #[test]
    fn z3_delay_1_equiv_non_overlapping() {
        // req |=> ack should be equivalent to req |-> ##1 ack
        // Both mean: "if req at t, then ack at t+1"
        let a = translate_sva_to_bounded("req |=> ack", 3).unwrap();
        let b = translate_sva_to_bounded("req |-> ##1 ack", 3).unwrap();
        let verify_a = bounded_to_verify(&a.expr);
        let verify_b = bounded_to_verify(&b.expr);
        let result = check_equivalence(
            &verify_a, &verify_b,
            &["req".into(), "ack".into()], 3,
        );
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "|=> must equal |-> ##1. Got: {:?}", result);
    }

    #[test]
    fn z3_axi_write_handshake_self_consistent() {
        // Three AXI properties should be self-consistent
        let p1 = translate_sva_to_bounded("AWVALID |-> s_eventually(AWREADY)", 5).unwrap();
        let p2 = translate_sva_to_bounded("(AWVALID && AWREADY) |-> s_eventually(WVALID)", 5).unwrap();
        let p3 = translate_sva_to_bounded("(WVALID && WREADY) |-> s_eventually(BVALID)", 5).unwrap();

        // Each property should be self-equivalent
        let v1 = bounded_to_verify(&p1.expr);
        let v2 = bounded_to_verify(&p2.expr);
        let v3 = bounded_to_verify(&p3.expr);
        let r1 = check_equivalence(&v1, &v1, &["AWVALID".into(), "AWREADY".into()], 5);
        let r2 = check_equivalence(&v2, &v2, &["AWVALID".into(), "AWREADY".into(), "WVALID".into()], 5);
        let r3 = check_equivalence(&v3, &v3, &["WVALID".into(), "WREADY".into(), "BVALID".into()], 5);
        assert!(matches!(r1, EquivalenceResult::Equivalent));
        assert!(matches!(r2, EquivalenceResult::Equivalent));
        assert!(matches!(r3, EquivalenceResult::Equivalent));
    }

    // ═══════════════════════════════════════════════════
    // NEW SVA CONSTRUCTS — Z3 SEMANTIC VERIFICATION
    // ═══════════════════════════════════════════════════

    #[test]
    fn z3_stable_equiv_past_eq() {
        // $stable(sig) ≡ (sig == $past(sig, 1)) — both mean "signal unchanged"
        let stable = translate_sva_to_bounded("$stable(sig)", 3).unwrap();
        let past_eq = translate_sva_to_bounded("sig == $past(sig, 1)", 3).unwrap();
        let verify_stable = bounded_to_verify(&stable.expr);
        let verify_past = bounded_to_verify(&past_eq.expr);
        let result = check_equivalence(
            &verify_stable, &verify_past,
            &["sig".into()], 3,
        );
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "$stable(sig) must equal sig==$past(sig,1). Got: {:?}", result);
    }

    #[test]
    fn z3_changed_is_not_stable() {
        // $changed(sig) ≡ !$stable(sig)
        let changed = translate_sva_to_bounded("$changed(sig)", 3).unwrap();
        let not_stable = translate_sva_to_bounded("!$stable(sig)", 3).unwrap();
        let verify_changed = bounded_to_verify(&changed.expr);
        let verify_not_stable = bounded_to_verify(&not_stable.expr);
        let result = check_equivalence(
            &verify_changed, &verify_not_stable,
            &["sig".into()], 3,
        );
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "$changed must equal !$stable. Got: {:?}", result);
    }

    #[test]
    fn z3_nexttime_equiv_delay_1() {
        // nexttime(P) ≡ ##1 P — both shift by one timestep
        let nt = translate_sva_to_bounded("nexttime(valid)", 3).unwrap();
        let delay = translate_sva_to_bounded("##1 valid", 3).unwrap();
        let verify_nt = bounded_to_verify(&nt.expr);
        let verify_delay = bounded_to_verify(&delay.expr);
        let result = check_equivalence(
            &verify_nt, &verify_delay,
            &["valid".into()], 3,
        );
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "nexttime(P) must equal ##1 P. Got: {:?}", result);
    }

    #[test]
    fn z3_counterexample_at_larger_bound() {
        // Test counterexample extraction at bound=10
        let lhs = translate_sva_to_bounded("req |-> ##[1:3] ack", 10).unwrap();
        let rhs = translate_sva_to_bounded("req |-> ##[1:10] ack", 10).unwrap();
        let verify_lhs = bounded_to_verify(&lhs.expr);
        let verify_rhs = bounded_to_verify(&rhs.expr);
        let result = check_equivalence(
            &verify_lhs, &verify_rhs,
            &["req".into(), "ack".into()], 10,
        );
        match result {
            EquivalenceResult::NotEquivalent { counterexample } => {
                // Should have cycles up to bound
                assert!(!counterexample.cycles.is_empty(),
                    "Counterexample at bound=10 must have cycles");
            }
            _ => panic!("Should be not equivalent at different bounds. Got: {:?}", result),
        }
    }

    // ═══════════════════════════════════════════════════
    // SPRINT 0A: Z3 OVERAPPROXIMATION ELIMINATION
    // These test that encode_to_z3 handles integer ops
    // and uninterpreted functions correctly, not as `true`.
    // ═══════════════════════════════════════════════════

    #[test]
    fn z3_does_not_overapproximate_arithmetic() {
        // Gt(x, 5) is NOT equivalent to Lt(y, 3) — different variables, different predicates.
        // Before Sprint 0A fix, both would become `true` in Z3, making them falsely equivalent.
        let lhs = VerifyExpr::binary(VerifyOp::Gt,
            VerifyExpr::Var("x@0".into()),
            VerifyExpr::Int(5),
        );
        let rhs = VerifyExpr::binary(VerifyOp::Lt,
            VerifyExpr::Var("y@0".into()),
            VerifyExpr::Int(3),
        );
        let result = check_equivalence(&lhs, &rhs, &["x".into(), "y".into()], 1);
        assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "Gt(x,5) must NOT equal Lt(y,3). Got: {:?}", result);
    }

    #[test]
    fn z3_integer_equality_works() {
        // Eq(x, 5) should be equivalent to itself.
        let expr = VerifyExpr::binary(VerifyOp::Eq,
            VerifyExpr::Var("x@0".into()),
            VerifyExpr::Int(5),
        );
        let result = check_equivalence(&expr, &expr, &["x".into()], 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "Eq(x,5) must equal Eq(x,5). Got: {:?}", result);
    }

    #[test]
    fn z3_integer_inequality_detected() {
        // Gt(x, 5) is NOT equivalent to Lt(x, 5) — opposite predicates on same variable.
        let lhs = VerifyExpr::binary(VerifyOp::Gt,
            VerifyExpr::Var("x@0".into()),
            VerifyExpr::Int(5),
        );
        let rhs = VerifyExpr::binary(VerifyOp::Lt,
            VerifyExpr::Var("x@0".into()),
            VerifyExpr::Int(5),
        );
        let result = check_equivalence(&lhs, &rhs, &["x".into()], 1);
        assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "Gt(x,5) must NOT equal Lt(x,5). Got: {:?}", result);
    }

    #[test]
    fn z3_mixed_boolean_integer() {
        // And(valid, Gt(count, 0)) is NOT equivalent to And(valid, Lt(count, 0))
        let lhs = VerifyExpr::binary(VerifyOp::And,
            VerifyExpr::Var("valid@0".into()),
            VerifyExpr::binary(VerifyOp::Gt,
                VerifyExpr::Var("count@0".into()),
                VerifyExpr::Int(0),
            ),
        );
        let rhs = VerifyExpr::binary(VerifyOp::And,
            VerifyExpr::Var("valid@0".into()),
            VerifyExpr::binary(VerifyOp::Lt,
                VerifyExpr::Var("count@0".into()),
                VerifyExpr::Int(0),
            ),
        );
        let result = check_equivalence(&lhs, &rhs, &["valid".into(), "count".into()], 1);
        assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "And(valid, Gt(count,0)) must NOT equal And(valid, Lt(count,0)). Got: {:?}", result);
    }

    #[test]
    fn z3_uninterpreted_functions_not_conflated() {
        // Apply("Foo", [x]) is NOT equivalent to Apply("Bar", [x])
        // Before Sprint 0A, both became `true` in Z3.
        let lhs = VerifyExpr::apply("Foo", vec![VerifyExpr::Var("x@0".into())]);
        let rhs = VerifyExpr::apply("Bar", vec![VerifyExpr::Var("x@0".into())]);
        let result = check_equivalence(&lhs, &rhs, &["x".into()], 1);
        assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "Apply(Foo,[x]) must NOT equal Apply(Bar,[x]). Got: {:?}", result);
    }

    // ═══════════════════════════════════════════════════
    // SPRINT 0B: QUANTIFIER REJECTION
    // Quantified formulas should NOT silently become true.
    // ═══════════════════════════════════════════════════

    #[test]
    fn z3_rejects_quantified_formulas() {
        use logicaffeine_verify::ir::VerifyType;
        // forall x. x > 0 should NOT be equivalent to true.
        // Before Sprint 0B, ForAll was silently encoded as true.
        let lhs = VerifyExpr::forall(
            vec![("x".into(), VerifyType::Int)],
            VerifyExpr::binary(VerifyOp::Gt, VerifyExpr::Var("x".into()), VerifyExpr::Int(0)),
        );
        let rhs = VerifyExpr::Bool(true);
        let result = check_equivalence(&lhs, &rhs, &[], 1);
        // forall x. x > 0 is FALSE (x = -1 is a counterexample), so it must differ from true
        assert!(!matches!(result, EquivalenceResult::Equivalent),
            "forall x. x > 0 must NOT equal true. Got: {:?}", result);
    }

    #[test]
    fn z3_rejects_existential() {
        use logicaffeine_verify::ir::VerifyType;
        // exists x. x > 0 should NOT be equivalent to true.
        // (It IS actually true, but exists x. x < 0 AND x > 0 is false.)
        // Use a contradiction: exists x. (x > 0 AND x < 0) ≢ true
        let lhs = VerifyExpr::exists(
            vec![("x".into(), VerifyType::Int)],
            VerifyExpr::binary(VerifyOp::And,
                VerifyExpr::binary(VerifyOp::Gt, VerifyExpr::Var("x".into()), VerifyExpr::Int(0)),
                VerifyExpr::binary(VerifyOp::Lt, VerifyExpr::Var("x".into()), VerifyExpr::Int(0)),
            ),
        );
        let rhs = VerifyExpr::Bool(true);
        let result = check_equivalence(&lhs, &rhs, &[], 1);
        // exists x. (x > 0 AND x < 0) is FALSE, so must NOT equal true
        assert!(!matches!(result, EquivalenceResult::Equivalent),
            "exists x. (x>0 AND x<0) must NOT equal true. Got: {:?}", result);
    }

    // ═══════════════════════════════════════════════════
    // SPRINT 0B: EXPANDED QUANTIFIER TESTS (spec delta)
    // ═══════════════════════════════════════════════════

    #[test]
    fn z3_forall_valid_implication() {
        use logicaffeine_verify::ir::VerifyType;
        // forall x:Int. (x > 0 -> x >= 0) is VALID (equivalent to true)
        let lhs = VerifyExpr::forall(
            vec![("x".into(), VerifyType::Int)],
            VerifyExpr::binary(VerifyOp::Implies,
                VerifyExpr::binary(VerifyOp::Gt, VerifyExpr::Var("x".into()), VerifyExpr::Int(0)),
                VerifyExpr::binary(VerifyOp::Gte, VerifyExpr::Var("x".into()), VerifyExpr::Int(0)),
            ),
        );
        let rhs = VerifyExpr::Bool(true);
        let result = check_equivalence(&lhs, &rhs, &[], 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "forall x. (x>0 -> x>=0) should be valid. Got: {:?}", result);
    }

    #[test]
    fn z3_forall_invalid_not_equiv_true() {
        use logicaffeine_verify::ir::VerifyType;
        // forall x:Int. x > 0 is NOT valid (x could be negative)
        let lhs = VerifyExpr::forall(
            vec![("x".into(), VerifyType::Int)],
            VerifyExpr::binary(VerifyOp::Gt, VerifyExpr::Var("x".into()), VerifyExpr::Int(0)),
        );
        let rhs = VerifyExpr::Bool(true);
        let result = check_equivalence(&lhs, &rhs, &[], 1);
        assert!(!matches!(result, EquivalenceResult::Equivalent),
            "forall x. x>0 is NOT valid. Got: {:?}", result);
    }

    #[test]
    fn z3_exists_satisfiable() {
        use logicaffeine_verify::ir::VerifyType;
        // exists x:Int. x == 5 is true (satisfiable)
        let lhs = VerifyExpr::exists(
            vec![("x".into(), VerifyType::Int)],
            VerifyExpr::binary(VerifyOp::Eq, VerifyExpr::Var("x".into()), VerifyExpr::Int(5)),
        );
        let rhs = VerifyExpr::Bool(true);
        let result = check_equivalence(&lhs, &rhs, &[], 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "exists x. x==5 should be satisfiable (equiv to true). Got: {:?}", result);
    }

    #[test]
    fn z3_nested_quantifiers() {
        use logicaffeine_verify::ir::VerifyType;
        // forall x. exists y. y > x is VALID over integers
        let lhs = VerifyExpr::forall(
            vec![("x".into(), VerifyType::Int)],
            VerifyExpr::exists(
                vec![("y".into(), VerifyType::Int)],
                VerifyExpr::binary(VerifyOp::Gt, VerifyExpr::Var("y".into()), VerifyExpr::Var("x".into())),
            ),
        );
        let rhs = VerifyExpr::Bool(true);
        let result = check_equivalence(&lhs, &rhs, &[], 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "forall x. exists y. y>x should be valid. Got: {:?}", result);
    }

    #[test]
    fn z3_quantifier_alternation() {
        use logicaffeine_verify::ir::VerifyType;
        // forall x. exists y. y == x is valid (pick y = x)
        let lhs = VerifyExpr::forall(
            vec![("x".into(), VerifyType::Int)],
            VerifyExpr::exists(
                vec![("y".into(), VerifyType::Int)],
                VerifyExpr::binary(VerifyOp::Eq, VerifyExpr::Var("y".into()), VerifyExpr::Var("x".into())),
            ),
        );
        let rhs = VerifyExpr::Bool(true);
        let result = check_equivalence(&lhs, &rhs, &[], 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "forall x. exists y. y==x should be valid. Got: {:?}", result);
    }

    #[test]
    fn z3_empty_quantifier_body_only() {
        use logicaffeine_verify::ir::VerifyType;
        // forall (no vars). P ≡ P
        let body = VerifyExpr::binary(VerifyOp::Gt, VerifyExpr::Var("x".into()), VerifyExpr::Int(0));
        let lhs = VerifyExpr::forall(vec![], body.clone());
        let result = check_equivalence(&lhs, &body, &["x".into()], 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "forall (no vars). P should equal P. Got: {:?}", result);
    }

    #[test]
    fn z3_quantifier_scope_correct() {
        use logicaffeine_verify::ir::VerifyType;
        // (forall x. x > 0) AND (y > 0) — second y is free, first x is bound
        // This is NOT equivalent to true because forall x. x > 0 is false
        let lhs = VerifyExpr::binary(VerifyOp::And,
            VerifyExpr::forall(
                vec![("x".into(), VerifyType::Int)],
                VerifyExpr::binary(VerifyOp::Gt, VerifyExpr::Var("x".into()), VerifyExpr::Int(0)),
            ),
            VerifyExpr::binary(VerifyOp::Gt, VerifyExpr::Var("y".into()), VerifyExpr::Int(0)),
        );
        let rhs = VerifyExpr::Bool(true);
        let result = check_equivalence(&lhs, &rhs, &["y".into()], 1);
        assert!(!matches!(result, EquivalenceResult::Equivalent),
            "(forall x. x>0) AND (y>0) is NOT valid. Got: {:?}", result);
    }

    #[test]
    fn z3_exists_with_conjunction() {
        use logicaffeine_verify::ir::VerifyType;
        // exists x. (x > 3 AND x < 10) ≡ true
        let lhs = VerifyExpr::exists(
            vec![("x".into(), VerifyType::Int)],
            VerifyExpr::binary(VerifyOp::And,
                VerifyExpr::binary(VerifyOp::Gt, VerifyExpr::Var("x".into()), VerifyExpr::Int(3)),
                VerifyExpr::binary(VerifyOp::Lt, VerifyExpr::Var("x".into()), VerifyExpr::Int(10)),
            ),
        );
        let rhs = VerifyExpr::Bool(true);
        let result = check_equivalence(&lhs, &rhs, &[], 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "exists x. (x>3 AND x<10) is satisfiable. Got: {:?}", result);
    }

    #[test]
    fn z3_forall_with_equality() {
        use logicaffeine_verify::ir::VerifyType;
        // forall x. (x == x) is valid (reflexivity)
        let lhs = VerifyExpr::forall(
            vec![("x".into(), VerifyType::Int)],
            VerifyExpr::binary(VerifyOp::Eq, VerifyExpr::Var("x".into()), VerifyExpr::Var("x".into())),
        );
        let rhs = VerifyExpr::Bool(true);
        let result = check_equivalence(&lhs, &rhs, &[], 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "forall x. x==x should be valid. Got: {:?}", result);
    }

    #[test]
    fn z3_quantifier_with_free_var_interaction() {
        use logicaffeine_verify::ir::VerifyType;
        // forall x. (x > y) is NOT valid (depends on y)
        // It's NOT equivalent to true because if y is very large, no x > y always holds...
        // Actually forall x:Int. x > y is false for any y (pick x = y - 1).
        let lhs = VerifyExpr::forall(
            vec![("x".into(), VerifyType::Int)],
            VerifyExpr::binary(VerifyOp::Gt, VerifyExpr::Var("x".into()), VerifyExpr::Var("y".into())),
        );
        let rhs = VerifyExpr::Bool(true);
        let result = check_equivalence(&lhs, &rhs, &["y".into()], 1);
        assert!(!matches!(result, EquivalenceResult::Equivalent),
            "forall x. x>y is NOT valid. Got: {:?}", result);
    }

    #[test]
    fn z3_forall_bool_tautology() {
        use logicaffeine_verify::ir::VerifyType;
        // forall p:Bool. (p OR NOT p) is valid (excluded middle)
        let lhs = VerifyExpr::forall(
            vec![("p".into(), VerifyType::Bool)],
            VerifyExpr::binary(VerifyOp::Or,
                VerifyExpr::Var("p".into()),
                VerifyExpr::not(VerifyExpr::Var("p".into())),
            ),
        );
        let rhs = VerifyExpr::Bool(true);
        let result = check_equivalence(&lhs, &rhs, &[], 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "forall p. p OR NOT p should be valid. Got: {:?}", result);
    }

    #[test]
    fn z3_exists_unsatisfiable() {
        use logicaffeine_verify::ir::VerifyType;
        // exists p:Bool. (p AND NOT p) is unsatisfiable ≡ false
        let lhs = VerifyExpr::exists(
            vec![("p".into(), VerifyType::Bool)],
            VerifyExpr::binary(VerifyOp::And,
                VerifyExpr::Var("p".into()),
                VerifyExpr::not(VerifyExpr::Var("p".into())),
            ),
        );
        let rhs = VerifyExpr::Bool(false);
        let result = check_equivalence(&lhs, &rhs, &[], 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "exists p. (p AND NOT p) should equal false. Got: {:?}", result);
    }

    #[test]
    fn z3_quantifier_in_equivalence_check() {
        use logicaffeine_verify::ir::VerifyType;
        // Two equivalent quantified formulas:
        // forall x. (x + 0 == x) ≡ forall y. (y + 0 == y)
        let lhs = VerifyExpr::forall(
            vec![("x".into(), VerifyType::Int)],
            VerifyExpr::binary(VerifyOp::Eq,
                VerifyExpr::binary(VerifyOp::Add, VerifyExpr::Var("x".into()), VerifyExpr::Int(0)),
                VerifyExpr::Var("x".into()),
            ),
        );
        let rhs = VerifyExpr::forall(
            vec![("y".into(), VerifyType::Int)],
            VerifyExpr::binary(VerifyOp::Eq,
                VerifyExpr::binary(VerifyOp::Add, VerifyExpr::Var("y".into()), VerifyExpr::Int(0)),
                VerifyExpr::Var("y".into()),
            ),
        );
        let result = check_equivalence(&lhs, &rhs, &[], 1);
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "forall x. x+0==x should equal forall y. y+0==y. Got: {:?}", result);
    }
}
