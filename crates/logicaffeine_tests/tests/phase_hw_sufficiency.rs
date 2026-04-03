//! Sprint 6D: Property Sufficiency Analysis

use logicaffeine_compile::codegen_sva::sufficiency::analyze_sufficiency;
use logicaffeine_language::semantics::knowledge_graph::{HwKnowledgeGraph, SignalRole, KgRelation};

fn build_kg(signals: Vec<(&str, SignalRole)>, edges: Vec<(&str, &str, KgRelation)>) -> HwKnowledgeGraph {
    let mut kg = HwKnowledgeGraph::new();
    for (name, role) in signals {
        kg.add_signal(name, 1, role);
    }
    for (from, to, rel) in edges {
        kg.add_edge(from, to, rel, None);
    }
    kg
}

#[test]
fn all_covered_ratio_positive() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output)],
        vec![("req", "ack", KgRelation::Triggers)],
    );
    let report = analyze_sufficiency(&kg);
    assert!(report.coverage_ratio > 0.0, "Covered signals should have positive ratio");
}

#[test]
fn lonely_signal_detected() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output), ("orphan", SignalRole::Internal)],
        vec![("req", "ack", KgRelation::Triggers)],
    );
    let report = analyze_sufficiency(&kg);
    assert!(report.lonely_signals.contains(&"orphan".to_string()),
        "Orphan signal should be lonely. Got: {:?}", report.lonely_signals);
}

#[test]
fn unconstrained_output() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("dangling_out", SignalRole::Output)],
        vec![],
    );
    let report = analyze_sufficiency(&kg);
    assert!(report.unconstrained_outputs.contains(&"dangling_out".to_string()),
        "Output with no edge should be unconstrained. Got: {:?}", report.unconstrained_outputs);
}

#[test]
fn recommendations_actionable() {
    let kg = build_kg(
        vec![("orphan", SignalRole::Internal)],
        vec![],
    );
    let report = analyze_sufficiency(&kg);
    assert!(report.recommendations.iter().any(|r| r.contains("orphan")),
        "Recommendations should mention signal names. Got: {:?}", report.recommendations);
}

#[test]
fn empty_kg_zero() {
    let kg = HwKnowledgeGraph::new();
    let report = analyze_sufficiency(&kg);
    assert_eq!(report.coverage_ratio, 0.0);
    assert!(report.lonely_signals.is_empty());
}

#[test]
fn report_serializable() {
    let kg = build_kg(
        vec![("req", SignalRole::Input)],
        vec![],
    );
    let report = analyze_sufficiency(&kg);
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("coverage_ratio"));
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT H: Sufficiency MUST detect missing handshake pairs
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn sufficiency_detects_missing_req_ack_pair() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output), ("data", SignalRole::Internal)],
        vec![("req", "data", KgRelation::Triggers)],
    );
    let report = analyze_sufficiency(&kg);
    assert!(!report.missing_handshakes.is_empty(),
        "req without ack edge MUST be flagged as missing handshake.\n\
         Got missing_handshakes: {:?}", report.missing_handshakes);
}

#[test]
fn sufficiency_no_false_positive_when_paired() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output)],
        vec![("req", "ack", KgRelation::Triggers)],
    );
    let report = analyze_sufficiency(&kg);
    assert!(report.missing_handshakes.is_empty(),
        "req->ack edge exists, MUST NOT flag as missing. Got: {:?}", report.missing_handshakes);
}

#[test]
fn sufficiency_detects_valid_without_ready() {
    let kg = build_kg(
        vec![("valid", SignalRole::Input), ("ready", SignalRole::Output), ("data", SignalRole::Internal)],
        vec![("valid", "data", KgRelation::Triggers)],
    );
    let report = analyze_sufficiency(&kg);
    assert!(!report.missing_handshakes.is_empty(),
        "valid without ready edge MUST be flagged. Got: {:?}", report.missing_handshakes);
}
