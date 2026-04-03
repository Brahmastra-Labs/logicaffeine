//! Sprint 3B: CEGAR Refinement

use logicaffeine_compile::codegen_sva::synthesis_refine::{
    classify_divergence, Divergence,
    weaken_implication, strengthen_implication, weaken_to_eventual,
};

#[test]
fn classify_too_strong() {
    let result = classify_divergence(true, false);
    assert_eq!(result, Divergence::TooStrong);
}

#[test]
fn classify_too_weak() {
    let result = classify_divergence(false, true);
    assert_eq!(result, Divergence::TooWeak);
}

#[test]
fn classify_unknown() {
    let result = classify_divergence(true, true);
    assert_eq!(result, Divergence::Unknown);
}

#[test]
fn weaken_overlapping_to_nonoverlapping() {
    let sva = "req |-> ack";
    let result = weaken_implication(sva);
    assert!(result.contains("|=>"), "Should convert |-> to |=>. Got: {}", result);
    assert!(!result.contains("|->"), "Should not contain |-> anymore");
}

#[test]
fn strengthen_nonoverlapping_to_overlapping() {
    let sva = "req |=> ack";
    let result = strengthen_implication(sva);
    assert!(result.contains("|->"), "Should convert |=> to |->. Got: {}", result);
}

#[test]
fn weaken_to_eventual_response() {
    let sva = "req |-> ack";
    let result = weaken_to_eventual(sva);
    assert!(result.contains("s_eventually"), "Should add s_eventually. Got: {}", result);
}

#[test]
fn weaken_to_eventual_idempotent() {
    let sva = "req |-> s_eventually(ack)";
    let result = weaken_to_eventual(sva);
    // Already eventual — should not double-wrap
    assert_eq!(result, sva, "Should be idempotent on already-eventual SVA");
}

#[test]
fn classify_same_behavior() {
    let result = classify_divergence(false, false);
    assert_eq!(result, Divergence::Unknown,
        "Same behavior should be Unknown (equivalent)");
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT G: CEGAR must have an ACTUAL refinement loop
// ═══════════════════════════════════════════════════════════════════════════

use logicaffeine_compile::codegen_sva::synthesis_refine::RefinementResult;

#[test]
fn refine_sva_result_struct_exists() {
    let result = RefinementResult {
        converged: false, iterations: 0, final_sva: "test".into(), divergence: None,
    };
    assert!(result.iterations <= 100, "Sanity check");
}

#[cfg(feature = "verification")]
mod cegar_loop {
    use logicaffeine_compile::codegen_sva::synthesis_refine::refine_sva;

    #[test]
    fn cegar_terminates_within_bounded_iterations() {
        let spec = "Always, if every request holds, then every acknowledgment holds.";
        let sva = "req |-> ack";
        let result = refine_sva(spec, sva, "clk", 5);
        assert!(result.iterations <= 10,
            "CEGAR MUST terminate within bounded iterations, got: {}", result.iterations);
        assert!(result.iterations >= 1,
            "CEGAR MUST attempt at least 1 iteration, got: {}", result.iterations);
    }

    #[test]
    fn cegar_reports_divergence_for_unrefinable() {
        let spec = "Always, if every request holds, then every acknowledgment holds.";
        let unrelated_sva = "grant_a && grant_b";
        let result = refine_sva(spec, unrelated_sva, "clk", 5);
        assert!(!result.converged,
            "CEGAR MUST report non-convergence for fundamentally wrong SVA");
    }

    #[test]
    fn cegar_final_sva_differs_from_input_when_refined() {
        let spec = "Always, if every request holds, then every acknowledgment holds.";
        let sva = "req |-> ack";
        let result = refine_sva(spec, sva, "clk", 5);
        if !result.converged && result.iterations > 1 {
            assert_ne!(result.final_sva, sva,
                "CEGAR must transform the SVA during refinement");
        }
    }
}
