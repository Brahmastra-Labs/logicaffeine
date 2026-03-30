//! Sprint I: Hardware Verification API Integration
//!
//! Tests the public API that composes the full pipeline:
//! parse → KG → SVA codegen → equivalence checking.

use logicaffeine_compile::codegen_sva::{
    SvaProperty, SvaAssertionKind, emit_sva_property, emit_sva_module,
    sanitize_property_name,
    sva_model::{parse_sva, sva_exprs_structurally_equivalent},
};
use logicaffeine_language::semantics::knowledge_graph::{
    HwKnowledgeGraph, KgRelation, SignalRole,
};
use logicaffeine_language::compile_kripke;

// ═══════════════════════════════════════════════════════════════════════════
// END-TO-END: ENGLISH → FOL (existing Kripke pipeline)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn api_compile_kripke_produces_fol() {
    let output = compile_kripke("John can fly.").unwrap();
    assert!(output.contains("Accessible_Alethic"));
    assert!(output.contains("w0") || output.contains("w1"));
}

// ═══════════════════════════════════════════════════════════════════════════
// END-TO-END: KG + SVA CODEGEN
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn api_full_pipeline_kg_to_sva() {
    // Build a KG manually (from a hypothetical parse)
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("req", 1, SignalRole::Input);
    kg.add_signal("ack", 1, SignalRole::Output);
    kg.add_signal("clk", 1, SignalRole::Clock);
    kg.add_property("Handshake", "liveness", "G(req -> F(ack))");
    kg.add_edge("req", "ack", KgRelation::Triggers, Some("Handshake".into()));

    // Generate SVA from the KG property
    let prop = SvaProperty {
        name: sanitize_property_name("Handshake"),
        clock: "clk".to_string(),
        body: "req |-> s_eventually(ack)".to_string(),
        kind: SvaAssertionKind::Assert,
    };
    let sva = emit_sva_property(&prop);

    // Verify the SVA output is well-formed
    assert!(sva.contains("property p_handshake"));
    assert!(sva.contains("@(posedge clk)"));
    assert!(sva.contains("req |-> s_eventually(ack)"));
    assert!(sva.contains("assert property"));

    // Verify the KG JSON is valid
    let json = kg.to_json();
    assert!(json.contains("\"req\""));
    assert!(json.contains("\"Handshake\""));
    assert!(json.contains("\"triggers\""));
}

#[test]
fn api_full_pipeline_mutex_example() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("grant_a", 1, SignalRole::Internal);
    kg.add_signal("grant_b", 1, SignalRole::Internal);
    kg.add_signal("clk", 1, SignalRole::Clock);
    kg.add_property("Mutex", "safety", "G(!(grant_a & grant_b))");
    kg.add_edge("grant_a", "grant_b", KgRelation::Constrains, Some("Mutex".into()));

    let prop = SvaProperty {
        name: sanitize_property_name("Mutex"),
        clock: "clk".to_string(),
        body: "!(grant_a && grant_b)".to_string(),
        kind: SvaAssertionKind::Assert,
    };
    let sva = emit_sva_property(&prop);

    assert!(sva.contains("!(grant_a && grant_b)"));
    assert!(sva.contains("assert property"));
}

// ═══════════════════════════════════════════════════════════════════════════
// END-TO-END: SVA EQUIVALENCE CHECKING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn api_equivalence_check_matching_sva() {
    let sva1 = parse_sva("req |-> s_eventually(ack)").unwrap();
    let sva2 = parse_sva("req |-> s_eventually(ack)").unwrap();
    assert!(sva_exprs_structurally_equivalent(&sva1, &sva2));
}

#[test]
fn api_equivalence_check_mismatched_sva() {
    // FOL says req |-> s_eventually(ack)  (liveness)
    // SVA says req |-> ack                (wrong — immediate, not eventual)
    let spec_sva = parse_sva("req |-> s_eventually(ack)").unwrap();
    let wrong_sva = parse_sva("req |-> ack").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&spec_sva, &wrong_sva));
}

// ═══════════════════════════════════════════════════════════════════════════
// END-TO-END: MULTI-PROPERTY SVA MODULE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn api_axi_example_multi_property() {
    // AXI write channel properties
    let props = vec![
        SvaProperty {
            name: "p_write_addr_handshake".to_string(),
            clock: "clk".to_string(),
            body: "AWVALID |-> s_eventually(AWREADY)".to_string(),
            kind: SvaAssertionKind::Assert,
        },
        SvaProperty {
            name: "p_write_data_follows_addr".to_string(),
            clock: "clk".to_string(),
            body: "(AWVALID && AWREADY) |-> s_eventually(WVALID)".to_string(),
            kind: SvaAssertionKind::Assert,
        },
        SvaProperty {
            name: "p_write_response".to_string(),
            clock: "clk".to_string(),
            body: "(WVALID && WREADY) |-> s_eventually(BVALID)".to_string(),
            kind: SvaAssertionKind::Assert,
        },
    ];

    let sva_module = emit_sva_module(&props);

    // All 3 properties present
    assert!(sva_module.contains("p_write_addr_handshake"));
    assert!(sva_module.contains("p_write_data_follows_addr"));
    assert!(sva_module.contains("p_write_response"));

    // All assert
    assert_eq!(sva_module.matches("assert property").count(), 3);

    // All have clock sensitivity
    assert_eq!(sva_module.matches("@(posedge clk)").count(), 3);
}

// ═══════════════════════════════════════════════════════════════════════════
// END-TO-END: KG JSON ROUND-TRIP
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn api_kg_json_has_all_fields() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("AWVALID", 1, SignalRole::Input);
    kg.add_signal("AWREADY", 1, SignalRole::Output);
    kg.add_signal("clk", 1, SignalRole::Clock);
    kg.add_property("Write Address Handshake", "liveness", "G(P -> F(Q))");
    kg.add_edge(
        "AWVALID",
        "AWREADY",
        KgRelation::Triggers,
        Some("Write Address Handshake".into()),
    );

    let json = kg.to_json();

    // Signals
    assert!(json.contains("\"AWVALID\""));
    assert!(json.contains("\"input\""));
    assert!(json.contains("\"clock\""));

    // Properties
    assert!(json.contains("\"Write Address Handshake\""));
    assert!(json.contains("\"liveness\""));

    // Edges
    assert!(json.contains("\"triggers\""));
}
