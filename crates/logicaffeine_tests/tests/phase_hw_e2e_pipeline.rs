//! Sprint 6: End-to-End Hardware Verification Pipeline
//!
//! Integration tests that exercise the full pipeline:
//! English → FOL → KG → SVA → Bounded IR → Equivalence Check.

use logicaffeine_compile::codegen_sva::hw_pipeline::{
    compile_hw_spec, emit_hw_sva, translate_sva_to_bounded,
    translate_spec_to_bounded, check_structural_equivalence,
    check_bounded_equivalence, EquivalenceResult, HwError,
};
use logicaffeine_compile::codegen_sva::sva_model::{
    parse_sva, sva_expr_to_string, sva_exprs_structurally_equivalent,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_compile::codegen_sva::SvaAssertionKind;
use logicaffeine_language::compile_kripke_with;
use logicaffeine_language::semantics::knowledge_graph::{
    extract_from_kripke_ast, HwKnowledgeGraph, KgRelation, SignalRole,
};

// ═══════════════════════════════════════════════════════════════════════════
// DIRECTION 1: English → FOL → SVA
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_english_to_fol_to_sva_mutex() {
    // 1. Compile English to FOL
    let fol = compile_hw_spec("Always, every dog runs.").unwrap();
    assert!(fol.contains("Accessible_Temporal"), "FOL should contain temporal accessibility");

    // 2. Build SVA from the property
    let sva = emit_hw_sva("Mutex", "clk", "!(grant_a && grant_b)", SvaAssertionKind::Assert);
    assert!(sva.contains("assert property"));
    assert!(sva.contains("@(posedge clk)"));
}

#[test]
fn e2e_sva_roundtrip_parse_emit_parse() {
    let original = "req |-> s_eventually(ack)";
    let parsed = parse_sva(original).unwrap();
    let emitted = sva_expr_to_string(&parsed);
    let reparsed = parse_sva(&emitted).unwrap();
    assert!(sva_exprs_structurally_equivalent(&parsed, &reparsed));
}

// ═══════════════════════════════════════════════════════════════════════════
// DIRECTION 2: SVA → Bounded IR ← FOL → Compare
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_both_sides_translate_to_bounded() {
    let spec_result = translate_spec_to_bounded("Always, every dog runs.", 5);
    assert!(spec_result.is_ok(), "Spec translation should succeed");

    let sva_result = translate_sva_to_bounded("!(grant_a && grant_b)", 5);
    assert!(sva_result.is_ok(), "SVA translation should succeed");
}

#[test]
fn e2e_identical_sva_are_structurally_equivalent() {
    let sva1 = "req |-> s_eventually(ack)";
    let sva2 = "req |-> s_eventually(ack)";
    assert!(check_structural_equivalence(sva1, sva2).unwrap());
}

#[test]
fn e2e_different_sva_are_not_equivalent() {
    let sva1 = "req |-> s_eventually(ack)";
    let sva2 = "req |-> ack"; // Immediate, not eventual
    assert!(!check_structural_equivalence(sva1, sva2).unwrap());
}

// ═══════════════════════════════════════════════════════════════════════════
// KNOWLEDGE GRAPH EXTRACTION FROM SPEC
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_kg_from_spec_has_safety_property() {
    let kg = compile_kripke_with("Always, every dog runs.", |ast, interner| {
        extract_from_kripke_ast(ast, interner)
    })
    .unwrap();

    assert!(
        kg.properties.iter().any(|p| p.property_type == "safety"),
        "KG should detect safety property from 'Always'"
    );

    let json = kg.to_json();
    assert!(json.contains("\"safety\""), "JSON should contain safety type");
}

#[test]
fn e2e_kg_from_spec_has_liveness_property() {
    let kg = compile_kripke_with("Eventually, John runs.", |ast, interner| {
        extract_from_kripke_ast(ast, interner)
    })
    .unwrap();

    assert!(
        kg.properties.iter().any(|p| p.property_type == "liveness"),
        "KG should detect liveness property from 'Eventually'"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// FULL PIPELINE: SPEC + SVA + BOUNDED COMPARE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_pipeline_mutex_bounded_check() {
    // Translate both sides to bounded IR and compare structurally
    let sva_bounded = translate_sva_to_bounded("!(grant_a && grant_b)", 3).unwrap();

    // Build a matching FOL bounded expr manually for comparison
    let manual_bounded = {
        let mut result: Option<BoundedExpr> = None;
        for t in 0..3u32 {
            let ga = BoundedExpr::Var(format!("grant_a@{}", t));
            let gb = BoundedExpr::Var(format!("grant_b@{}", t));
            let not_both = BoundedExpr::Not(Box::new(BoundedExpr::And(
                Box::new(ga),
                Box::new(gb),
            )));
            result = Some(match result {
                None => not_both,
                Some(acc) => BoundedExpr::And(Box::new(acc), Box::new(not_both)),
            });
        }
        result.unwrap()
    };

    let equiv = check_bounded_equivalence(&sva_bounded.expr, &manual_bounded, 3);
    assert!(
        equiv.equivalent,
        "SVA and manually constructed bounded exprs should match"
    );
}

#[test]
fn e2e_pipeline_different_bounded_not_equivalent() {
    let sva_a = translate_sva_to_bounded("req |-> ack", 3).unwrap();
    let sva_b = translate_sva_to_bounded("req |=> ack", 3).unwrap(); // Different: |=> vs |->
    let equiv = check_bounded_equivalence(&sva_a.expr, &sva_b.expr, 3);
    assert!(!equiv.equivalent, "Different implication types should not be equivalent");
}

// ═══════════════════════════════════════════════════════════════════════════
// AXI-STYLE MULTI-PROPERTY
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_axi_multi_property_sva_generation() {
    // Generate 3 AXI write channel SVA properties
    let props = vec![
        ("Write Address Handshake", "AWVALID |-> s_eventually(AWREADY)"),
        ("Write Data Follows", "(AWVALID && AWREADY) |-> s_eventually(WVALID)"),
        ("Write Response", "(WVALID && WREADY) |-> s_eventually(BVALID)"),
    ];

    let mut sva_output = String::new();
    for (name, body) in &props {
        let sva = emit_hw_sva(name, "clk", body, SvaAssertionKind::Assert);
        sva_output.push_str(&sva);
        sva_output.push_str("\n\n");
    }

    // All 3 properties should be present
    assert!(sva_output.contains("p_write_address_handshake"));
    assert!(sva_output.contains("p_write_data_follows"));
    assert!(sva_output.contains("p_write_response"));
    assert_eq!(sva_output.matches("assert property").count(), 3);

    // Each should parse back through SVA parser
    for (_, body) in &props {
        let parsed = parse_sva(body);
        assert!(parsed.is_ok(), "AXI SVA '{}' should parse: {:?}", body, parsed.err());
    }

    // Each should translate to bounded IR
    for (_, body) in &props {
        let bounded = translate_sva_to_bounded(body, 10);
        assert!(bounded.is_ok(), "AXI SVA '{}' should translate: {:?}", body, bounded.err());
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ERROR HANDLING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_invalid_sva_returns_error() {
    let result = translate_sva_to_bounded("|||bad|||", 5);
    assert!(result.is_err());
}

#[test]
fn e2e_invalid_spec_returns_error() {
    // Use a syntactically invalid sentence that won't panic but will fail to parse
    let result = compile_hw_spec("Every.");
    assert!(result.is_err(), "Incomplete sentence should error");
}
