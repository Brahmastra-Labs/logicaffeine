//! Sprint 6E: Invariant Discovery from Knowledge Graph

#![cfg(feature = "verification")]

use logicaffeine_compile::codegen_sva::invariants::{
    discover_invariants, CandidateInvariant, InvariantSource,
};
use logicaffeine_language::semantics::knowledge_graph::{HwKnowledgeGraph, SignalRole, KgRelation};
use logicaffeine_verify::ir::VerifyExpr;

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
fn discover_mutex_from_constrains() {
    let kg = build_kg(
        vec![("grant_a", SignalRole::Internal), ("grant_b", SignalRole::Internal)],
        vec![("grant_a", "grant_b", KgRelation::Constrains)],
    );
    let invariants = discover_invariants(&kg);
    assert!(invariants.iter().any(|i| i.source == InvariantSource::MutexPattern),
        "Constrains edge should produce MutexPattern invariant. Got: {:?}",
        invariants.iter().map(|i| &i.source).collect::<Vec<_>>());
}

#[test]
fn discover_handshake_from_triggers() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output)],
        vec![("req", "ack", KgRelation::Triggers)],
    );
    let invariants = discover_invariants(&kg);
    assert!(invariants.iter().any(|i| i.source == InvariantSource::HandshakePattern),
        "Triggers edge should produce HandshakePattern invariant");
}

#[test]
fn empty_kg_no_invariants() {
    let kg = HwKnowledgeGraph::new();
    let invariants = discover_invariants(&kg);
    assert!(invariants.is_empty(), "Empty KG should produce no invariants");
}

#[test]
fn multiple_from_complex_kg() {
    let kg = build_kg(
        vec![
            ("grant_a", SignalRole::Internal),
            ("grant_b", SignalRole::Internal),
            ("req", SignalRole::Input),
            ("ack", SignalRole::Output),
        ],
        vec![
            ("grant_a", "grant_b", KgRelation::Constrains),
            ("req", "ack", KgRelation::Triggers),
        ],
    );
    let invariants = discover_invariants(&kg);
    assert!(invariants.len() >= 2, "Complex KG should produce 2+ invariants. Got: {}", invariants.len());
}

#[test]
fn source_correctly_tagged() {
    let kg = build_kg(
        vec![("a", SignalRole::Internal), ("b", SignalRole::Internal)],
        vec![("a", "b", KgRelation::Constrains)],
    );
    let invariants = discover_invariants(&kg);
    for inv in &invariants {
        if matches!(inv.expr, VerifyExpr::Not(_)) {
            assert_eq!(inv.source, InvariantSource::MutexPattern,
                "Negation invariant should be MutexPattern");
        }
    }
}

#[test]
fn invariant_expr_is_well_formed() {
    let kg = build_kg(
        vec![("req", SignalRole::Input), ("ack", SignalRole::Output)],
        vec![("req", "ack", KgRelation::Triggers)],
    );
    let invariants = discover_invariants(&kg);
    for inv in &invariants {
        // Every invariant should have a non-trivial expression
        assert!(!matches!(inv.expr, VerifyExpr::Bool(true)),
            "Invariant should not be vacuously true");
    }
}

#[test]
fn verified_starts_as_none() {
    let kg = build_kg(
        vec![("a", SignalRole::Internal), ("b", SignalRole::Internal)],
        vec![("a", "b", KgRelation::Constrains)],
    );
    let invariants = discover_invariants(&kg);
    for inv in &invariants {
        assert!(inv.verified.is_none(), "Unverified invariants should have verified=None");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT H: Invariants MUST discover Pipeline + Reset patterns
// ═══════════════════════════════════════════════════════════════════════════

use logicaffeine_language::semantics::knowledge_graph::{
    HwEntityType, HwRelation, ResetPolarity,
};

#[test]
fn invariant_pipeline_from_entity() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("stage1", 1, SignalRole::Internal);
    kg.add_signal("stage2", 1, SignalRole::Internal);
    kg.add_entity("pipe", HwEntityType::Pipeline { stages: 2, stall_signal: None });
    kg.add_typed_edge("stage1", "stage2", HwRelation::Triggers { delay: Some(1) });
    let invariants = discover_invariants(&kg);
    assert!(invariants.iter().any(|i| i.source == InvariantSource::PipelineStability),
        "Pipeline entity MUST produce PipelineStability invariant.\n\
         Sources found: {:?}",
        invariants.iter().map(|i| &i.source).collect::<Vec<_>>());
}

#[test]
fn invariant_reset_from_entity() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("rst", 1, SignalRole::Input);
    kg.add_signal("state", 1, SignalRole::Internal);
    kg.add_entity("reset", HwEntityType::Reset {
        polarity: ResetPolarity::ActiveHigh, synchronous: true,
    });
    kg.add_typed_edge("rst", "state", HwRelation::Resets);
    let invariants = discover_invariants(&kg);
    assert!(invariants.iter().any(|i| i.source == InvariantSource::ResetInit),
        "Reset entity MUST produce ResetInit invariant.\n\
         Sources found: {:?}",
        invariants.iter().map(|i| &i.source).collect::<Vec<_>>());
}

#[test]
fn invariant_all_four_sources_discoverable() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("ga", 1, SignalRole::Output);
    kg.add_signal("gb", 1, SignalRole::Output);
    kg.add_edge("ga", "gb", KgRelation::Constrains, None);
    kg.add_signal("req", 1, SignalRole::Input);
    kg.add_signal("ack", 1, SignalRole::Output);
    kg.add_edge("req", "ack", KgRelation::Triggers, None);
    kg.add_entity("pipe", HwEntityType::Pipeline { stages: 2, stall_signal: None });
    kg.add_typed_edge("ga", "gb", HwRelation::Triggers { delay: Some(1) });
    kg.add_entity("reset", HwEntityType::Reset {
        polarity: ResetPolarity::ActiveHigh, synchronous: true,
    });
    kg.add_typed_edge("req", "ack", HwRelation::Resets);
    let invariants = discover_invariants(&kg);
    let sources: std::collections::HashSet<_> = invariants.iter()
        .map(|i| format!("{:?}", i.source)).collect();
    assert!(sources.len() >= 4,
        "All 4 InvariantSource variants MUST be discoverable.\n\
         Got {} sources: {:?}", sources.len(), sources);
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT 6E BACKFILL: Z3 Invariant Verification
// ═══════════════════════════════════════════════════════════════════════════

use logicaffeine_compile::codegen_sva::invariants::verify_invariant;

#[test]
fn z3_verifies_discovered_invariant() {
    let kg = build_kg(
        vec![("grant_a", SignalRole::Internal), ("grant_b", SignalRole::Internal)],
        vec![("grant_a", "grant_b", KgRelation::Constrains)],
    );
    let mut invariants = discover_invariants(&kg);
    assert!(!invariants.is_empty());
    let result = verify_invariant(&mut invariants[0], 1);
    assert!(result, "Mutex invariant not(A AND B) should be satisfiable");
    assert_eq!(invariants[0].verified, Some(true));
}

#[test]
fn verified_marked_true() {
    let mut inv = CandidateInvariant {
        expr: VerifyExpr::implies(VerifyExpr::var("P"), VerifyExpr::var("Q")),
        source: InvariantSource::HandshakePattern,
        verified: None,
    };
    let result = verify_invariant(&mut inv, 1);
    assert!(result, "Implication P -> Q should be satisfiable");
    assert_eq!(inv.verified, Some(true));
}

#[test]
fn unverifiable_marked_false() {
    let mut inv = CandidateInvariant {
        expr: VerifyExpr::and(
            VerifyExpr::var("P"),
            VerifyExpr::not(VerifyExpr::var("P")),
        ),
        source: InvariantSource::MutexPattern,
        verified: None,
    };
    let result = verify_invariant(&mut inv, 1);
    assert!(!result, "P AND NOT(P) must be unsatisfiable");
    assert_eq!(inv.verified, Some(false));
}
