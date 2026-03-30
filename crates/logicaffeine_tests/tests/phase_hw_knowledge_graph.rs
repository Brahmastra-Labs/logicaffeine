//! Sprint F: Knowledge Graph Extraction
//!
//! Tests for the HwKnowledgeGraph types, construction, and JSON serialization.
//! These test the KG data structures that provide LLMs with formally grounded
//! context for SVA generation.

use logicaffeine_language::semantics::knowledge_graph::{
    HwKnowledgeGraph, KgRelation, SignalRole,
};

// ═══════════════════════════════════════════════════════════════════════════
// KG CONSTRUCTION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kg_constructs_with_signals() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("req", 1, SignalRole::Input);
    kg.add_signal("ack", 1, SignalRole::Output);
    kg.add_signal("data", 8, SignalRole::Internal);

    assert_eq!(kg.signals.len(), 3);
    assert!(kg.signals.iter().any(|s| s.name == "req" && s.width == 1));
    assert!(kg.signals.iter().any(|s| s.name == "data" && s.width == 8));
}

#[test]
fn kg_constructs_with_properties() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_property("Handshake", "liveness", "G(P -> F(Q))");
    kg.add_property("Mutex", "safety", "G(!(P & Q))");

    assert_eq!(kg.properties.len(), 2);
    assert!(kg.properties.iter().any(|p| p.name == "Handshake"));
    assert!(kg.properties.iter().any(|p| p.name == "Mutex"));
}

#[test]
fn kg_constructs_with_edges() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("req", 1, SignalRole::Input);
    kg.add_signal("ack", 1, SignalRole::Output);

    kg.add_edge("req", "ack", KgRelation::Triggers, Some("Handshake".into()));
    kg.add_edge("req", "ack", KgRelation::Constrains, Some("Mutex".into()));

    let temporal_edges: Vec<_> = kg
        .edges
        .iter()
        .filter(|e| e.relation == KgRelation::Triggers)
        .collect();
    assert_eq!(temporal_edges.len(), 1);

    let constraint_edges: Vec<_> = kg
        .edges
        .iter()
        .filter(|e| e.relation == KgRelation::Constrains)
        .collect();
    assert_eq!(constraint_edges.len(), 1);
}

// ═══════════════════════════════════════════════════════════════════════════
// JSON SERIALIZATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kg_serializes_to_valid_json() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("req", 1, SignalRole::Input);
    kg.add_property("Safety", "safety", "G(P)");

    let json = kg.to_json();
    assert!(json.contains("req"), "JSON should contain signal name");
    assert!(json.contains("Safety"), "JSON should contain property name");
    assert!(json.contains("\"signals\""), "JSON should have signals key");
    assert!(json.contains("\"properties\""), "JSON should have properties key");
    assert!(json.contains("\"edges\""), "JSON should have edges key");
}

#[test]
fn kg_json_contains_signal_roles() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("clk", 1, SignalRole::Clock);
    kg.add_signal("data", 8, SignalRole::Internal);

    let json = kg.to_json();
    assert!(json.contains("\"clock\""), "Clock signal should have clock role");
    assert!(
        json.contains("\"internal\""),
        "Data signal should have internal role"
    );
}

#[test]
fn kg_identifies_clock_signals() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("clk", 1, SignalRole::Clock);
    kg.add_signal("data", 8, SignalRole::Internal);

    let clk = kg.signals.iter().find(|s| s.name == "clk").unwrap();
    assert_eq!(clk.role, SignalRole::Clock);

    let data = kg.signals.iter().find(|s| s.name == "data").unwrap();
    assert_eq!(data.role, SignalRole::Internal);
}

// ═══════════════════════════════════════════════════════════════════════════
// EDGE RELATIONS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kg_edge_carries_property_reference() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_edge("req", "ack", KgRelation::Temporal, Some("Response".into()));

    let edge = &kg.edges[0];
    assert_eq!(edge.from, "req");
    assert_eq!(edge.to, "ack");
    assert_eq!(edge.relation, KgRelation::Temporal);
    assert_eq!(edge.property.as_deref(), Some("Response"));
}

#[test]
fn kg_json_includes_edge_properties() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("req", 1, SignalRole::Input);
    kg.add_signal("ack", 1, SignalRole::Output);
    kg.add_edge("req", "ack", KgRelation::Triggers, Some("Handshake".into()));

    let json = kg.to_json();
    assert!(json.contains("\"triggers\""), "Edge should have triggers relation");
    assert!(
        json.contains("\"Handshake\""),
        "Edge should reference Handshake property"
    );
}
