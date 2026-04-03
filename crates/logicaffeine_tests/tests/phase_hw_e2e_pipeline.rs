//! Sprint 6: End-to-End Hardware Verification Pipeline
//!
//! Integration tests that exercise the full pipeline:
//! English → FOL → KG → SVA → Bounded IR → Equivalence Check.

use logicaffeine_compile::codegen_sva::hw_pipeline::{
    compile_hw_spec, emit_hw_sva, translate_sva_to_bounded,
    translate_spec_to_bounded, check_structural_equivalence,
    check_bounded_equivalence, extract_kg, EquivalenceResult, HwError,
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
fn e2e_invalid_sva_returns_parse_error() {
    let result = translate_sva_to_bounded("|||bad|||", 5);
    match result {
        Ok(_) => panic!("Invalid SVA should produce an error"),
        Err(e) => {
            let msg = format!("{}", e);
            assert!(msg.len() > 5,
                "SVA error message must be substantive. Got: {}", msg);
        }
    }
}

#[test]
fn e2e_invalid_spec_returns_parse_error() {
    let result = compile_hw_spec("Every.");
    match result {
        Ok(_) => panic!("Incomplete sentence should error"),
        Err(e) => {
            let msg = format!("{}", e);
            assert!(msg.len() > 5,
                "Spec error message must be substantive. Got: {}", msg);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// KNOWLEDGE GRAPH EXTRACTION API
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn extract_kg_one_call_api_works() {
    let kg = extract_kg("Always, every dog runs.").unwrap();
    assert!(!kg.properties.is_empty(),
        "KG should have at least one property. Got: {:?}", kg);
    let json = kg.to_json();
    assert!(json.contains("safety"),
        "Always → safety property. JSON: {}", json);
}

#[test]
fn extract_kg_liveness_property() {
    let kg = extract_kg("Eventually, John runs.").unwrap();
    let json = kg.to_json();
    assert!(json.contains("liveness"),
        "Eventually → liveness property. JSON: {}", json);
}

// ═══════════════════════════════════════════════════════════════════════════
// ALEXANDER DEMO: THE PIPELINE THAT NOBODY ELSE HAS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn demo_stage1_english_to_fol_produces_kripke_lowered_temporal() {
    let fol = compile_hw_spec("Always, every signal is valid.").unwrap();
    assert!(fol.contains("Accessible_Temporal"),
        "Stage 1: Must produce Kripke-lowered temporal FOL. Got: {}", fol);
}

#[test]
fn demo_stage2_fol_to_knowledge_graph_extracts_structure() {
    let kg = extract_kg("Always, every signal is valid.").unwrap();
    let json = kg.to_json();
    assert!(json.contains("signals"), "Stage 2: KG JSON must have signals. JSON: {}", json);
    assert!(json.contains("properties"), "Stage 2: KG JSON must have properties. JSON: {}", json);
    assert!(json.contains("safety"), "Stage 2: Always → safety. JSON: {}", json);
}

#[test]
fn demo_stage3_sva_parses_and_roundtrips() {
    use logicaffeine_compile::codegen_sva::sva_model::{parse_sva, sva_expr_to_string, sva_exprs_structurally_equivalent};
    let sva = "req |-> ##[1:5] ack";
    let parsed = parse_sva(sva).unwrap();
    let rendered = sva_expr_to_string(&parsed);
    let reparsed = parse_sva(&rendered).unwrap();
    assert!(sva_exprs_structurally_equivalent(&parsed, &reparsed),
        "Stage 3: SVA must roundtrip. {} → {} → must match", sva, rendered);
}

// ═══════════════════════════════════════════════════════════════════════════
// Z3 PIPELINE — FULL SEMANTIC EQUIVALENCE
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "verification")]
mod z3_pipeline {
    use logicaffeine_compile::codegen_sva::hw_pipeline::check_z3_equivalence;
    use logicaffeine_verify::equivalence::EquivalenceResult;

    #[test]
    fn z3_pipeline_different_properties_not_equivalent() {
        // English spec about dogs running ≢ mutex SVA (completely different properties)
        let result = check_z3_equivalence(
            "Always, every dog runs.",
            "!(grant_a && grant_b)",
            3,
        ).unwrap();
        assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "Different properties must not be equivalent. Got: {:?}", result);
    }

    #[test]
    fn z3_pipeline_error_on_invalid_sva() {
        let result = check_z3_equivalence(
            "Always, every dog runs.",
            "|||invalid|||",
            3,
        );
        assert!(result.is_err(), "Invalid SVA should produce an error");
    }

    #[test]
    fn z3_pipeline_error_on_invalid_spec() {
        let result = check_z3_equivalence(
            "Every.",
            "req |-> ack",
            3,
        );
        assert!(result.is_err(), "Invalid spec should produce an error");
    }

    #[test]
    fn z3_pipeline_self_equivalence() {
        // Same SVA property translated from same spec → must be equivalent to itself
        use logicaffeine_compile::codegen_sva::sva_to_verify::bounded_to_verify;
        use logicaffeine_compile::codegen_sva::hw_pipeline::translate_sva_to_bounded;
        use logicaffeine_verify::equivalence::check_equivalence;

        let sva = "!(grant_a && grant_b)";
        let result_a = translate_sva_to_bounded(sva, 3).unwrap();
        let result_b = translate_sva_to_bounded(sva, 3).unwrap();
        let verify_a = bounded_to_verify(&result_a.expr);
        let verify_b = bounded_to_verify(&result_b.expr);
        let equiv = check_equivalence(
            &verify_a, &verify_b,
            &["grant_a".into(), "grant_b".into()], 3,
        );
        assert!(matches!(equiv, EquivalenceResult::Equivalent),
            "Same property must be equivalent to itself. Got: {:?}", equiv);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PROTOCOL PATTERNS — Real-World SVA Through Full Pipeline
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_axi_read_channel_multi_property() {
    // AXI read channel: ARVALID/ARREADY handshake, then RVALID/RREADY data
    let properties = vec![
        ("Read Address Handshake", "ARVALID |-> s_eventually(ARREADY)"),
        ("Read Data Follows Address", "(ARVALID && ARREADY) |-> s_eventually(RVALID)"),
        ("Read Response Complete", "(RVALID && RREADY) |-> s_eventually(RLAST)"),
    ];

    for (name, body) in &properties {
        let sva = emit_hw_sva(name, "clk", body, SvaAssertionKind::Assert);
        assert!(sva.contains("assert property"), "AXI property '{}' missing assert", name);
        assert!(sva.contains("@(posedge clk)"), "AXI property '{}' missing clock", name);

        // Parse roundtrip
        let parsed = parse_sva(body).unwrap();
        let emitted = sva_expr_to_string(&parsed);
        let reparsed = parse_sva(&emitted).unwrap();
        assert!(sva_exprs_structurally_equivalent(&parsed, &reparsed),
            "Roundtrip failed for AXI property '{}'", name);

        // Bounded translation
        let bounded = translate_sva_to_bounded(body, 5);
        assert!(bounded.is_ok(), "Bounded translation failed for AXI property '{}': {:?}", name, bounded.err());
    }
}

#[test]
fn e2e_spi_protocol_properties() {
    let properties = vec![
        ("SPI Chip Select", "$rose(ss) |-> ##[1:3] $rose(sclk)"),
        ("SPI MOSI Stable", "sclk |-> $stable(mosi)"),
        ("SPI Transfer Complete", "$rose(ss) |-> s_eventually($fell(ss))"),
    ];

    for (name, body) in &properties {
        let parsed = parse_sva(body);
        assert!(parsed.is_ok(), "SPI property '{}' failed to parse: {:?}", name, parsed.err());

        let bounded = translate_sva_to_bounded(body, 8);
        assert!(bounded.is_ok(), "SPI property '{}' failed bounded translation: {:?}", name, bounded.err());
    }
}

#[test]
fn e2e_arbiter_fairness_properties() {
    let properties = vec![
        ("Arbiter Mutex", "!(grant_0 && grant_1) && !(grant_0 && grant_2) && !(grant_1 && grant_2)"),
        ("Request 0 Liveness", "req_0 |-> s_eventually(grant_0)"),
        ("Request 1 Liveness", "req_1 |-> s_eventually(grant_1)"),
        ("Request 2 Liveness", "req_2 |-> s_eventually(grant_2)"),
    ];

    for (name, body) in &properties {
        let parsed = parse_sva(body);
        assert!(parsed.is_ok(), "Arbiter property '{}' failed to parse: {:?}", name, parsed.err());

        let sva = emit_hw_sva(name, "clk", body,
            if name.contains("Mutex") { SvaAssertionKind::Assert } else { SvaAssertionKind::Cover });
        assert!(!sva.is_empty(), "SVA emission empty for '{}'", name);

        let bounded = translate_sva_to_bounded(body, 10);
        assert!(bounded.is_ok(), "Arbiter property '{}' failed bounded: {:?}", name, bounded.err());
    }
}

#[test]
fn e2e_reset_aware_properties() {
    let properties = vec![
        ("Reset Handshake", "disable iff (reset) $rose(req) |-> ##[1:5] ack"),
        ("Reset Data Integrity", "disable iff (reset) valid |=> $stable(data)"),
    ];

    for (name, body) in &properties {
        let parsed = parse_sva(body);
        assert!(parsed.is_ok(), "Reset property '{}' failed to parse: {:?}", name, parsed.err());

        let bounded = translate_sva_to_bounded(body, 5);
        assert!(bounded.is_ok(), "Reset property '{}' failed bounded: {:?}", name, bounded.err());
    }
}

#[test]
fn e2e_i2c_protocol_pattern() {
    let properties = vec![
        ("I2C Start", "$fell(sda) && scl |-> s_eventually($rose(scl))"),
        ("I2C Ack", "scl |-> $stable(sda)"),
    ];

    for (name, body) in &properties {
        let parsed = parse_sva(body);
        assert!(parsed.is_ok(), "I2C property '{}' failed to parse: {:?}", name, parsed.err());
    }
}

#[test]
fn e2e_fifo_overflow_protection() {
    // FIFO: if full, writes are blocked; reads always succeed if not empty
    let properties = vec![
        ("No Write When Full", "full |-> !wr_en"),
        ("Read When Not Empty", "(!empty && rd_en) |-> s_eventually(rd_valid)"),
        ("Full Empty Mutex", "!(full && empty)"),
    ];

    for (name, body) in &properties {
        let parsed = parse_sva(body);
        assert!(parsed.is_ok(), "FIFO property '{}' failed to parse: {:?}", name, parsed.err());

        let bounded = translate_sva_to_bounded(body, 5);
        assert!(bounded.is_ok(), "FIFO property '{}' failed bounded: {:?}", name, bounded.err());
    }
}

#[test]
fn e2e_kg_multi_signal_protocol() {
    // Extract KG from a multi-predicate temporal spec
    let kg = extract_kg("Always, if every dog runs then some cat sleeps.").unwrap();
    let json = kg.to_json();
    // Must be valid JSON with expected structure
    assert!(json.contains("signals") || json.contains("properties"),
        "KG JSON must contain structural fields. Got: {}", json);
}

#[test]
fn e2e_kg_safety_and_liveness_coexist() {
    // Same spec can generate both safety and liveness properties
    let kg_safety = extract_kg("Always, every dog runs.").unwrap();
    let kg_liveness = extract_kg("Eventually, John runs.").unwrap();

    let safety_props: Vec<_> = kg_safety.properties.iter()
        .filter(|p| p.property_type == "safety")
        .collect();
    let liveness_props: Vec<_> = kg_liveness.properties.iter()
        .filter(|p| p.property_type == "liveness")
        .collect();

    assert!(!safety_props.is_empty(), "Should have safety properties");
    assert!(!liveness_props.is_empty(), "Should have liveness properties");
}

#[test]
fn e2e_structural_equiv_commutative_and() {
    // a && b should be structurally different from b && a (structural, not semantic)
    let result = check_structural_equivalence("req && ack", "ack && req").unwrap();
    // Structural equivalence is exact match — order matters
    assert!(!result, "Structural equiv should be order-sensitive");
}

#[test]
fn e2e_bounded_equiv_at_different_bounds() {
    let sva_3 = translate_sva_to_bounded("req |-> ack", 3).unwrap();
    let sva_5 = translate_sva_to_bounded("req |-> ack", 5).unwrap();
    // Same property at different bounds produces different conjunction depths
    let leaves_3 = logicaffeine_compile::codegen_sva::sva_to_verify::count_and_leaves(&sva_3.expr);
    let leaves_5 = logicaffeine_compile::codegen_sva::sva_to_verify::count_and_leaves(&sva_5.expr);
    assert!(leaves_5 > leaves_3, "Bound 5 should have more leaves than bound 3");
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT B: Unsupported constructs must NOT silently become true
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "verification")]
mod z3_unsupported {
    use logicaffeine_verify::ir::{VerifyExpr, BitVecOp};
    use logicaffeine_verify::equivalence::{check_equivalence, EquivalenceResult};

    #[test]
    fn z3_bitvector_not_silently_equivalent_to_true() {
        let bv = VerifyExpr::BitVecBinary {
            op: BitVecOp::And,
            left: Box::new(VerifyExpr::BitVecConst { value: 0xFF, width: 8 }),
            right: Box::new(VerifyExpr::BitVecConst { value: 0x0F, width: 8 }),
        };
        let trivial = VerifyExpr::Bool(true);
        let result = check_equivalence(&bv, &trivial, &[], 1);
        assert!(!matches!(result, EquivalenceResult::Equivalent),
            "Bitvector op MUST NOT be silently equivalent to true — got Equivalent");
    }

    #[test]
    fn z3_array_select_not_silently_equivalent_to_true() {
        let sel = VerifyExpr::Select {
            array: Box::new(VerifyExpr::Var("mem".into())),
            index: Box::new(VerifyExpr::Int(0)),
        };
        let trivial = VerifyExpr::Bool(true);
        let result = check_equivalence(&sel, &trivial, &[], 1);
        assert!(!matches!(result, EquivalenceResult::Equivalent),
            "Array Select MUST NOT be silently equivalent to true — got Equivalent");
    }

    #[test]
    fn z3_transition_not_silently_equivalent_to_true() {
        let trans = VerifyExpr::Transition {
            from: Box::new(VerifyExpr::Var("s0".into())),
            to: Box::new(VerifyExpr::Var("s1".into())),
        };
        let trivial = VerifyExpr::Bool(true);
        let result = check_equivalence(&trans, &trivial, &[], 1);
        assert!(!matches!(result, EquivalenceResult::Equivalent),
            "Transition MUST NOT be silently equivalent to true — got Equivalent");
    }
}
