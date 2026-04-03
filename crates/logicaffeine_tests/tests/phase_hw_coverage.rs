//! Sprint 4A: Specification Coverage Metrics

use logicaffeine_compile::codegen_sva::coverage::compute_coverage;
use logicaffeine_language::semantics::knowledge_graph::{
    HwKnowledgeGraph, SignalRole, KgRelation,
};

fn build_kg(
    signals: Vec<(&str, SignalRole)>,
    properties: Vec<(&str, &str, &str)>,
    edges: Vec<(&str, &str, KgRelation, Option<String>)>,
) -> HwKnowledgeGraph {
    let mut kg = HwKnowledgeGraph::new();
    for (name, role) in signals {
        kg.add_signal(name, 1, role);
    }
    for (name, ptype, op) in properties {
        kg.add_property(name, ptype, op);
    }
    for (from, to, rel, prop) in edges {
        kg.add_edge(from, to, rel, prop);
    }
    kg
}

#[test]
fn property_coverage_nonzero_when_property_signals_covered() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output)],
        vec![("handshake", "safety", "G(req -> ack)")],
        vec![("req", "ack", KgRelation::Triggers, Some("handshake".to_string()))],
    );
    let cov = compute_coverage(&kg, &["req".to_string(), "ack".to_string()]);
    assert!(cov.property_coverage > 0.0,
        "property_coverage MUST be >0 when property signal endpoints are covered, got: {}",
        cov.property_coverage);
}

#[test]
fn edge_coverage_nonzero_when_both_endpoints_covered() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output)],
        vec![],
        vec![("req", "ack", KgRelation::Triggers, None)],
    );
    let cov = compute_coverage(&kg, &["req".to_string(), "ack".to_string()]);
    assert!(cov.edge_coverage > 0.0,
        "edge_coverage MUST be >0 when both endpoints covered, got: {}", cov.edge_coverage);
}

#[test]
fn edge_coverage_less_than_1_when_only_one_endpoint_covered() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output)],
        vec![],
        vec![("req", "ack", KgRelation::Triggers, None)],
    );
    let cov = compute_coverage(&kg, &["req".to_string()]);
    assert!(cov.edge_coverage < 1.0,
        "edge_coverage must be <1.0 when only one endpoint covered, got: {}", cov.edge_coverage);
}

#[test]
fn all_hardcoded_zeros_impossible_with_full_coverage() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output)],
        vec![("handshake", "safety", "G(req -> ack)")],
        vec![("req", "ack", KgRelation::Triggers, Some("handshake".to_string()))],
    );
    let cov = compute_coverage(&kg, &["req".to_string(), "ack".to_string()]);
    let non_signal_total = cov.property_coverage + cov.edge_coverage + cov.temporal_coverage;
    assert!(non_signal_total > 0.0,
        "IMPOSSIBLE: property={}, edge={}, temporal={} are ALL zero with full coverage. \
         At least one must be >0.",
        cov.property_coverage, cov.edge_coverage, cov.temporal_coverage);
}

#[test]
fn uncovered_properties_excludes_covered_ones() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output)],
        vec![("handshake", "safety", "G(req -> ack)")],
        vec![("req", "ack", KgRelation::Triggers, Some("handshake".to_string()))],
    );
    let cov = compute_coverage(&kg, &["req".to_string(), "ack".to_string()]);
    assert!(!cov.uncovered_properties.contains(&"handshake".to_string()),
        "Property 'handshake' with covered edge endpoints MUST NOT be in uncovered list.\n\
         Got: {:?}", cov.uncovered_properties);
}

#[test]
fn full_coverage_produces_ratio_1() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output)],
        vec![("handshake", "safety", "G(req -> ack)")],
        vec![("req", "ack", KgRelation::Triggers, Some("handshake".to_string()))],
    );
    let cov = compute_coverage(&kg, &["req".to_string(), "ack".to_string()]);
    assert_eq!(cov.signal_coverage, 1.0,
        "All signals covered -> signal_coverage must be 1.0");
    assert_eq!(cov.edge_coverage, 1.0,
        "All edge endpoints covered -> edge_coverage must be 1.0");
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT 4A BACKFILL: Boundary conditions, partial coverage, serialization
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn zero_coverage_no_signals_covered() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output), ("data", SignalRole::Internal)],
        vec![("handshake", "safety", "G(req -> ack)")],
        vec![
            ("req", "ack", KgRelation::Triggers, Some("handshake".to_string())),
            ("data", "ack", KgRelation::Temporal, None),
        ],
    );
    let cov = compute_coverage(&kg, &[]);
    assert_eq!(cov.signal_coverage, 0.0, "No covered signals -> 0.0");
    assert_eq!(cov.edge_coverage, 0.0, "No covered signals -> 0.0 edge");
    assert_eq!(cov.property_coverage, 0.0, "No covered signals -> 0.0 property");
    assert_eq!(cov.temporal_coverage, 0.0, "No covered signals -> 0.0 temporal");
}

#[test]
fn empty_kg_zero_coverage() {
    let kg = HwKnowledgeGraph::new();
    let cov = compute_coverage(&kg, &["req".to_string()]);
    assert_eq!(cov.signal_coverage, 0.0, "Empty KG -> 0.0 signal coverage");
    assert_eq!(cov.edge_coverage, 0.0);
    assert_eq!(cov.property_coverage, 0.0);
    assert_eq!(cov.temporal_coverage, 0.0);
    assert!(cov.uncovered_signals.is_empty(), "Empty KG has no signals to be uncovered");
    assert!(cov.uncovered_properties.is_empty(), "Empty KG has no properties to be uncovered");
}

#[test]
fn partial_signal_coverage() {
    let kg = build_kg(
        vec![
            ("req", SignalRole::Input),
            ("ack", SignalRole::Output),
            ("data", SignalRole::Internal),
            ("valid", SignalRole::Internal),
        ],
        vec![],
        vec![],
    );
    let cov = compute_coverage(&kg, &["req".to_string(), "ack".to_string()]);
    assert_eq!(cov.signal_coverage, 0.5, "2/4 signals covered -> 0.5");
    assert_eq!(cov.uncovered_signals.len(), 2, "2 uncovered signals");
    assert!(cov.uncovered_signals.contains(&"data".to_string()));
    assert!(cov.uncovered_signals.contains(&"valid".to_string()));
}

#[test]
fn multiple_properties_mixed_coverage() {
    let kg = build_kg(
        vec![
            ("req", SignalRole::Input),
            ("ack", SignalRole::Output),
            ("data_in", SignalRole::Input),
            ("data_out", SignalRole::Output),
            ("rst", SignalRole::Input),
            ("ready", SignalRole::Output),
        ],
        vec![
            ("handshake", "safety", "G(req -> ack)"),
            ("data_integrity", "safety", "G(data_in -> data_out)"),
            ("reset_check", "safety", "G(rst -> ready)"),
        ],
        vec![
            ("req", "ack", KgRelation::Triggers, Some("handshake".to_string())),
            ("data_in", "data_out", KgRelation::Temporal, Some("data_integrity".to_string())),
            ("rst", "ready", KgRelation::Triggers, Some("reset_check".to_string())),
        ],
    );
    let cov = compute_coverage(
        &kg,
        &["req".to_string(), "ack".to_string(), "data_in".to_string(), "data_out".to_string()],
    );
    let expected = 2.0 / 3.0;
    assert!((cov.property_coverage - expected).abs() < 0.01,
        "2/3 properties covered -> ~0.667, got {}", cov.property_coverage);
    assert_eq!(cov.uncovered_properties.len(), 1);
    assert!(cov.uncovered_properties.contains(&"reset_check".to_string()));
}

#[test]
fn temporal_coverage_safety_and_liveness() {
    let kg = build_kg(
        vec![
            ("req", SignalRole::Input),
            ("ack", SignalRole::Output),
            ("grant", SignalRole::Output),
            ("done", SignalRole::Output),
            ("start", SignalRole::Input),
            ("finish", SignalRole::Output),
        ],
        vec![
            ("handshake", "safety", "G(req -> ack)"),
            ("grant_rule", "safety", "G(grant -> done)"),
            ("liveness", "liveness", "F(start -> finish)"),
        ],
        vec![
            ("req", "ack", KgRelation::Triggers, Some("handshake".to_string())),
            ("grant", "done", KgRelation::Triggers, Some("grant_rule".to_string())),
            ("start", "finish", KgRelation::Triggers, Some("liveness".to_string())),
        ],
    );
    let cov = compute_coverage(
        &kg,
        &["req".to_string(), "ack".to_string(), "start".to_string(), "finish".to_string()],
    );
    let expected = 2.0 / 3.0;
    assert!((cov.temporal_coverage - expected).abs() < 0.01,
        "2/3 temporal properties covered -> ~0.667, got {}", cov.temporal_coverage);
}

#[test]
fn temporal_coverage_zero_when_no_temporal_properties() {
    let kg = build_kg(
        vec![("a", SignalRole::Internal), ("b", SignalRole::Internal)],
        vec![("structural", "structural", "some_op")],
        vec![("a", "b", KgRelation::Temporal, Some("structural".to_string()))],
    );
    let cov = compute_coverage(&kg, &["a".to_string(), "b".to_string()]);
    assert_eq!(cov.temporal_coverage, 0.0,
        "Non-safety/liveness properties should not count as temporal");
}

#[test]
fn edge_coverage_partial_multiple_edges() {
    let kg = build_kg(
        vec![
            ("req", SignalRole::Input),
            ("ack", SignalRole::Output),
            ("data_in", SignalRole::Input),
            ("data_out", SignalRole::Output),
            ("valid", SignalRole::Internal),
            ("ready", SignalRole::Internal),
        ],
        vec![],
        vec![
            ("req", "ack", KgRelation::Triggers, None),
            ("data_in", "data_out", KgRelation::Temporal, None),
            ("valid", "ready", KgRelation::Triggers, None),
        ],
    );
    let cov = compute_coverage(&kg, &["req".to_string(), "ack".to_string(), "valid".to_string()]);
    let expected = 1.0 / 3.0;
    assert!((cov.edge_coverage - expected).abs() < 0.01,
        "1/3 edges fully covered -> ~0.333, got {}", cov.edge_coverage);
}

#[test]
fn uncovered_signals_correct_membership() {
    let kg = build_kg(
        vec![
            ("a", SignalRole::Internal),
            ("b", SignalRole::Internal),
            ("c", SignalRole::Internal),
            ("d", SignalRole::Internal),
            ("e", SignalRole::Internal),
        ],
        vec![],
        vec![],
    );
    let cov = compute_coverage(&kg, &["a".to_string(), "c".to_string(), "e".to_string()]);
    assert_eq!(cov.uncovered_signals.len(), 2, "2 signals uncovered");
    assert!(cov.uncovered_signals.contains(&"b".to_string()));
    assert!(cov.uncovered_signals.contains(&"d".to_string()));
}

#[test]
fn uncovered_properties_when_edge_partially_covered() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output)],
        vec![("handshake", "safety", "G(req -> ack)")],
        vec![("req", "ack", KgRelation::Triggers, Some("handshake".to_string()))],
    );
    let cov = compute_coverage(&kg, &["req".to_string()]);
    assert!(cov.uncovered_properties.contains(&"handshake".to_string()),
        "Property with only one endpoint covered should be uncovered");
}

#[test]
fn coverage_with_no_edges_but_signals() {
    let kg = build_kg(
        vec![("a", SignalRole::Internal), ("b", SignalRole::Internal)],
        vec![],
        vec![],
    );
    let cov = compute_coverage(&kg, &["a".to_string(), "b".to_string()]);
    assert_eq!(cov.signal_coverage, 1.0, "All signals covered -> 1.0");
    assert_eq!(cov.edge_coverage, 0.0, "No edges -> 0.0 edge coverage");
}

#[test]
fn json_serialization_round_trip() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output)],
        vec![("handshake", "safety", "G(req -> ack)")],
        vec![("req", "ack", KgRelation::Triggers, Some("handshake".to_string()))],
    );
    let cov = compute_coverage(&kg, &["req".to_string(), "ack".to_string()]);
    let json = serde_json::to_string(&cov).expect("Should serialize");
    let deser: logicaffeine_compile::codegen_sva::coverage::SpecCoverage =
        serde_json::from_str(&json).expect("Should deserialize");
    assert_eq!(cov.signal_coverage, deser.signal_coverage);
    assert_eq!(cov.property_coverage, deser.property_coverage);
    assert_eq!(cov.edge_coverage, deser.edge_coverage);
    assert_eq!(cov.temporal_coverage, deser.temporal_coverage);
    assert_eq!(cov.uncovered_signals, deser.uncovered_signals);
    assert_eq!(cov.uncovered_properties, deser.uncovered_properties);
}

#[test]
fn coverage_with_duplicate_covered_signals() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output)],
        vec![],
        vec![("req", "ack", KgRelation::Triggers, None)],
    );
    let cov = compute_coverage(&kg, &["req".to_string(), "req".to_string(), "ack".to_string()]);
    assert_eq!(cov.signal_coverage, 1.0, "Duplicates should not cause double-counting");
    assert_eq!(cov.edge_coverage, 1.0);
}

#[test]
fn signal_not_in_kg_ignored() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output)],
        vec![],
        vec![],
    );
    let cov = compute_coverage(&kg, &["req".to_string(), "ack".to_string(), "phantom".to_string()]);
    assert_eq!(cov.signal_coverage, 1.0, "Extra covered name not in KG should be ignored");
}

#[test]
fn large_kg_coverage_correct() {
    let kg = build_kg(
        vec![
            ("s0", SignalRole::Input), ("s1", SignalRole::Output),
            ("s2", SignalRole::Internal), ("s3", SignalRole::Input),
            ("s4", SignalRole::Output), ("s5", SignalRole::Internal),
            ("s6", SignalRole::Input), ("s7", SignalRole::Output),
            ("s8", SignalRole::Internal), ("s9", SignalRole::Clock),
        ],
        vec![
            ("p0", "safety", "G(s0 -> s1)"),
            ("p1", "liveness", "F(s2 -> s3)"),
            ("p2", "safety", "G(s4 -> s5)"),
            ("p3", "safety", "G(s6 -> s7)"),
            ("p4", "liveness", "F(s8 -> s9)"),
        ],
        vec![
            ("s0", "s1", KgRelation::Triggers, Some("p0".to_string())),
            ("s2", "s3", KgRelation::Triggers, Some("p1".to_string())),
            ("s4", "s5", KgRelation::Triggers, Some("p2".to_string())),
            ("s6", "s7", KgRelation::Triggers, Some("p3".to_string())),
            ("s8", "s9", KgRelation::Triggers, Some("p4".to_string())),
            ("s0", "s2", KgRelation::Temporal, None),
            ("s3", "s5", KgRelation::Temporal, None),
            ("s7", "s9", KgRelation::Temporal, None),
        ],
    );
    let covered: Vec<String> = (0..7).map(|i| format!("s{}", i)).collect();
    let cov = compute_coverage(&kg, &covered);
    assert_eq!(cov.signal_coverage, 0.7, "7/10 signals -> 0.7");
    assert_eq!(cov.uncovered_signals.len(), 3);
}
