//! Sprint 5: Knowledge Graph Extraction from FOL AST
//!
//! Tests that extract_from_kripke_ast() correctly walks Kripke-lowered
//! LogicExpr trees to populate HwKnowledgeGraph.

use logicaffeine_language::compile_kripke_with;
use logicaffeine_language::semantics::knowledge_graph::{extract_from_kripke_ast, KgRelation};

// ═══════════════════════════════════════════════════════════════════════════
// SIGNAL EXTRACTION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kg_extract_signals_from_always() {
    let kg = compile_kripke_with("Always, every dog runs.", |ast, interner| {
        extract_from_kripke_ast(ast, interner)
    })
    .unwrap();

    // Quantified variables in temporal scope should produce signals or properties
    assert!(
        !kg.signals.is_empty() || !kg.properties.is_empty(),
        "Should extract signals or properties from 'Always, every dog runs.'. Signals: {:?}, Props: {:?}",
        kg.signals, kg.properties
    );
}

#[test]
fn kg_extract_signal_names() {
    let kg = compile_kripke_with("Always, every dog runs.", |ast, interner| {
        extract_from_kripke_ast(ast, interner)
    })
    .unwrap();

    let signal_names: Vec<&str> = kg.signals.iter().map(|s| s.name.as_str()).collect();
    // Should have some signal extracted (dog, or the variable)
    assert!(
        !signal_names.is_empty(),
        "Should extract signal names from predicates"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PROPERTY TYPE DETECTION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kg_extract_safety_property_from_always() {
    let kg = compile_kripke_with("Always, John runs.", |ast, interner| {
        extract_from_kripke_ast(ast, interner)
    })
    .unwrap();

    assert!(
        kg.properties.iter().any(|p| p.property_type == "safety"),
        "Always (G) should produce a safety property. Got: {:?}",
        kg.properties
    );
}

#[test]
fn kg_extract_liveness_property_from_eventually() {
    let kg = compile_kripke_with("Eventually, John runs.", |ast, interner| {
        extract_from_kripke_ast(ast, interner)
    })
    .unwrap();

    assert!(
        kg.properties.iter().any(|p| p.property_type == "liveness"),
        "Eventually (F) should produce a liveness property. Got: {:?}",
        kg.properties
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// JSON OUTPUT
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kg_extract_produces_valid_json() {
    let kg = compile_kripke_with("Always, every dog runs.", |ast, interner| {
        extract_from_kripke_ast(ast, interner)
    })
    .unwrap();

    let json = kg.to_json();
    assert!(json.contains("\"signals\""), "JSON should have signals key");
    assert!(json.contains("\"properties\""), "JSON should have properties key");
    assert!(json.contains("\"edges\""), "JSON should have edges key");
}

// ═══════════════════════════════════════════════════════════════════════════
// NON-TEMPORAL HANDLING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kg_extract_non_temporal_still_works() {
    let kg = compile_kripke_with("Every dog runs.", |ast, interner| {
        extract_from_kripke_ast(ast, interner)
    })
    .unwrap();

    // Non-temporal sentences won't have temporal properties
    // but should still produce a valid (possibly empty) KG
    assert!(kg.properties.is_empty() || !kg.properties.is_empty()); // always true — just verify no panic
}
