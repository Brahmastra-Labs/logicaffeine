//! Sprint 5: Knowledge Graph Extraction from FOL AST
//!
//! Tests that extract_from_kripke_ast() correctly walks Kripke-lowered
//! LogicExpr trees to populate HwKnowledgeGraph.

use logicaffeine_language::compile_kripke_with;
use logicaffeine_language::semantics::knowledge_graph::{extract_from_kripke_ast, KgRelation, SignalRole};

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

    // Non-temporal sentences produce a valid KG without panicking.
    // The KG may or may not have properties depending on quantifier patterns.
    // Key assertion: the KG is structurally valid and serializes to JSON.
    let json = kg.to_json();
    assert!(json.contains("signals") || json.contains("properties"),
        "KG must produce valid JSON structure. Got: {}", json);
    assert!(json.starts_with('{') && json.ends_with('}'),
        "KG JSON must be a valid object. Got: {}", json);
}

// ═══════════════════════════════════════════════════════════════════════════
// EDGE EXTRACTION — Implication & Constraint Patterns
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kg_extract_implication_produces_edges_or_signals() {
    // Conditionals in FOL may produce Triggers edges or may decompose into
    // quantifier structures that the extractor handles as signals/properties.
    let kg = compile_kripke_with(
        "If every dog runs then every cat sleeps.",
        |ast, interner| extract_from_kripke_ast(ast, interner),
    )
    .unwrap();

    // The conditional should produce some structure — either edges or signals
    assert!(
        !kg.edges.is_empty() || !kg.signals.is_empty() || !kg.properties.is_empty(),
        "Conditional spec should extract some KG structure. Got: signals={:?}, properties={:?}, edges={:?}",
        kg.signals, kg.properties, kg.edges
    );
}

#[test]
fn kg_extract_conjunction_in_temporal_produces_signals() {
    // Multi-predicate temporal: more signals should be extracted
    let kg = compile_kripke_with(
        "Always, every dog runs and every cat sleeps.",
        |ast, interner| extract_from_kripke_ast(ast, interner),
    )
    .unwrap();

    assert!(
        kg.signals.len() >= 2,
        "Conjunction of predicates should extract multiple signals. Got: {:?}",
        kg.signals
    );
}

#[test]
fn kg_extract_nested_modal_in_temporal() {
    // "Always, every dog can run." — temporal + alethic nesting
    let kg = compile_kripke_with(
        "Always, every dog can run.",
        |ast, interner| extract_from_kripke_ast(ast, interner),
    )
    .unwrap();

    // Should still classify as safety (G is outermost)
    assert!(
        kg.properties.iter().any(|p| p.property_type == "safety"),
        "G(Diamond(P)) should still be classified as safety. Got: {:?}",
        kg.properties
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// JSON STRUCTURE QUALITY
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kg_json_signals_have_required_fields() {
    let kg = compile_kripke_with("Always, every dog runs.", |ast, interner| {
        extract_from_kripke_ast(ast, interner)
    })
    .unwrap();

    let json = kg.to_json();
    // Each signal in JSON should have name, width, and role
    if json.contains("\"name\"") {
        assert!(
            json.contains("\"width\"") && json.contains("\"role\""),
            "Signal JSON should have name, width, and role fields. Got: {}",
            json
        );
    }
}

#[test]
fn kg_json_properties_have_required_fields() {
    let kg = compile_kripke_with("Always, John runs.", |ast, interner| {
        extract_from_kripke_ast(ast, interner)
    })
    .unwrap();

    let json = kg.to_json();
    if !kg.properties.is_empty() {
        assert!(
            json.contains("\"property_type\"") || json.contains("\"type\""),
            "Property JSON should have type field. Got: {}",
            json
        );
    }
}

#[test]
fn kg_json_edges_have_relation_field() {
    let kg = compile_kripke_with(
        "Always, if every dog runs then every cat sleeps.",
        |ast, interner| extract_from_kripke_ast(ast, interner),
    )
    .unwrap();

    let json = kg.to_json();
    if !kg.edges.is_empty() {
        assert!(
            json.contains("\"relation\""),
            "Edge JSON should have relation field. Got: {}",
            json
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PROPERTY COUNT & MULTIPLICITY
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kg_extract_separate_temporal_specs_accumulate() {
    // Two separate temporal specs should produce two separate properties
    let kg_always = compile_kripke_with("Always, John runs.", |ast, interner| {
        extract_from_kripke_ast(ast, interner)
    })
    .unwrap();

    let kg_eventually = compile_kripke_with("Eventually, John runs.", |ast, interner| {
        extract_from_kripke_ast(ast, interner)
    })
    .unwrap();

    // Each should have at least one property
    assert!(
        !kg_always.properties.is_empty(),
        "Always spec should have properties"
    );
    assert!(
        !kg_eventually.properties.is_empty(),
        "Eventually spec should have properties"
    );

    // They should be different types
    let always_type = &kg_always.properties[0].property_type;
    let eventually_type = &kg_eventually.properties[0].property_type;
    assert_ne!(
        always_type, eventually_type,
        "G and F should produce different property types"
    );
}

#[test]
fn kg_extract_complex_spec_does_not_panic() {
    // Stress test: complex multi-clause temporal spec
    let specs = vec![
        "Always, every dog runs.",
        "Eventually, some cat sleeps.",
        "Always, if John runs then Mary walks.",
        "Eventually, every student reads.",
    ];

    for spec in specs {
        let result = compile_kripke_with(spec, |ast, interner| {
            extract_from_kripke_ast(ast, interner)
        });
        assert!(
            result.is_ok(),
            "KG extraction should not panic on '{}'. Error: {:?}",
            spec,
            result.err()
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT 0C: SIGNAL ROLE INFERENCE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kg_clock_signal_detected_by_name() {
    // A signal whose name contains "clk" should get Clock role.
    // We use the predicate name "valid" and signal name containing "clk".
    let kg = compile_kripke_with("Always, every clk_signal runs.", |ast, interner| {
        extract_from_kripke_ast(ast, interner)
    })
    .unwrap();

    let clock_signals: Vec<_> = kg.signals.iter()
        .filter(|s| s.role == SignalRole::Clock)
        .collect();
    assert!(
        !clock_signals.is_empty(),
        "Signal with 'clk' in name should have Clock role. Got signals: {:?}",
        kg.signals
    );
}

#[test]
fn kg_antecedent_is_input() {
    // In "if dog runs then cat sleeps", dog is in antecedent → Input.
    let kg = compile_kripke_with(
        "Always, if every dog runs then every cat sleeps.",
        |ast, interner| extract_from_kripke_ast(ast, interner),
    )
    .unwrap();

    // The antecedent variable (from "every dog runs") should be Input
    let input_signals: Vec<_> = kg.signals.iter()
        .filter(|s| s.role == SignalRole::Input)
        .collect();
    assert!(
        !input_signals.is_empty(),
        "Antecedent signals should be Input. Got signals: {:?}",
        kg.signals
    );
}

#[test]
fn kg_consequent_is_output() {
    // In "if dog runs then cat sleeps", cat is in consequent → Output.
    let kg = compile_kripke_with(
        "Always, if every dog runs then every cat sleeps.",
        |ast, interner| extract_from_kripke_ast(ast, interner),
    )
    .unwrap();

    let output_signals: Vec<_> = kg.signals.iter()
        .filter(|s| s.role == SignalRole::Output)
        .collect();
    assert!(
        !output_signals.is_empty(),
        "Consequent signals should be Output. Got signals: {:?}",
        kg.signals
    );
}

#[test]
fn kg_bidirectional_stays_internal() {
    // A signal appearing in both antecedent and consequent stays Internal.
    let kg = compile_kripke_with(
        "Always, if every dog runs then every dog sleeps.",
        |ast, interner| extract_from_kripke_ast(ast, interner),
    )
    .unwrap();

    let internal_signals: Vec<_> = kg.signals.iter()
        .filter(|s| s.role == SignalRole::Internal)
        .collect();
    assert!(
        !internal_signals.is_empty(),
        "Signal in both positions should be Internal. Got signals: {:?}",
        kg.signals
    );
}

#[test]
fn kg_property_name_from_predicate() {
    // Property names should come from predicate names, not be hardcoded "Safety".
    let kg = compile_kripke_with(
        "Always, every handshake is complete.",
        |ast, interner| extract_from_kripke_ast(ast, interner),
    )
    .unwrap();

    // At least one property should have a name derived from the predicate,
    // not just the generic "Safety" or "Liveness".
    let has_descriptive_name = kg.properties.iter().any(|p| {
        let name_lower = p.name.to_lowercase();
        name_lower != "safety" && name_lower != "liveness"
    });
    assert!(
        has_descriptive_name,
        "Properties should have descriptive names from predicates, not just 'Safety'. Got: {:?}",
        kg.properties
    );
}
