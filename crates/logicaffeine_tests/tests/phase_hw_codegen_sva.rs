//! Sprint D: SVA/PSL Code Generation
//!
//! Tests for SystemVerilog Assertion generation from temporal FOL.
//! The codegen_sva module lives in logicaffeine_compile behind the
//! codegen-sva feature flag.

// ═══════════════════════════════════════════════════════════════════════════
// SVA EMISSION — STRUCTURAL
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn sva_emits_property_block() {
    use logicaffeine_compile::codegen_sva::{SvaProperty, SvaAssertionKind, emit_sva_property};
    let prop = SvaProperty {
        name: "p_mutex".to_string(),
        clock: "clk".to_string(),
        body: "!(grant_a && grant_b)".to_string(),
        kind: SvaAssertionKind::Assert,
    };
    let sva = emit_sva_property(&prop);
    assert!(sva.contains("property p_mutex"), "Should contain property name. Got: {}", sva);
    assert!(sva.contains("@(posedge clk)"), "Should contain clock edge. Got: {}", sva);
    assert!(sva.contains("endproperty"), "Should close property block. Got: {}", sva);
    assert!(sva.contains("assert property"), "Safety should use assert. Got: {}", sva);
}

#[test]
fn sva_emits_cover_for_liveness() {
    use logicaffeine_compile::codegen_sva::{SvaProperty, SvaAssertionKind, emit_sva_property};
    let prop = SvaProperty {
        name: "p_progress".to_string(),
        clock: "clk".to_string(),
        body: "req |-> s_eventually(ack)".to_string(),
        kind: SvaAssertionKind::Cover,
    };
    let sva = emit_sva_property(&prop);
    assert!(sva.contains("cover property"), "Liveness should use cover. Got: {}", sva);
}

#[test]
fn sva_emits_assume_for_environment() {
    use logicaffeine_compile::codegen_sva::{SvaProperty, SvaAssertionKind, emit_sva_property};
    let prop = SvaProperty {
        name: "p_valid_input".to_string(),
        clock: "clk".to_string(),
        body: "valid".to_string(),
        kind: SvaAssertionKind::Assume,
    };
    let sva = emit_sva_property(&prop);
    assert!(sva.contains("assume property"), "Environment should use assume. Got: {}", sva);
}

// ═══════════════════════════════════════════════════════════════════════════
// SVA PROPERTY NAMING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn sva_sanitizes_property_name() {
    use logicaffeine_compile::codegen_sva::sanitize_property_name;
    assert_eq!(sanitize_property_name("Data Integrity"), "p_data_integrity");
    assert_eq!(sanitize_property_name("Handshake"), "p_handshake");
    assert_eq!(sanitize_property_name("Mutual Exclusion"), "p_mutual_exclusion");
}

// ═══════════════════════════════════════════════════════════════════════════
// MULTIPLE PROPERTIES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn sva_emits_multiple_properties() {
    use logicaffeine_compile::codegen_sva::{SvaProperty, SvaAssertionKind, emit_sva_module};
    let props = vec![
        SvaProperty {
            name: "p_safety".to_string(),
            clock: "clk".to_string(),
            body: "valid".to_string(),
            kind: SvaAssertionKind::Assert,
        },
        SvaProperty {
            name: "p_liveness".to_string(),
            clock: "clk".to_string(),
            body: "s_eventually(done)".to_string(),
            kind: SvaAssertionKind::Cover,
        },
    ];
    let sva = emit_sva_module(&props);
    assert!(sva.contains("p_safety"), "Should contain first property. Got: {}", sva);
    assert!(sva.contains("p_liveness"), "Should contain second property. Got: {}", sva);
    assert!(sva.contains("assert property"), "Should have assert. Got: {}", sva);
    assert!(sva.contains("cover property"), "Should have cover. Got: {}", sva);
}

// ═══════════════════════════════════════════════════════════════════════════
// PSL OUTPUT
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn psl_emits_assert_always() {
    use logicaffeine_compile::codegen_sva::{SvaProperty, SvaAssertionKind, emit_psl_property};
    let prop = SvaProperty {
        name: "p_invariant".to_string(),
        clock: "clk".to_string(),
        body: "valid".to_string(),
        kind: SvaAssertionKind::Assert,
    };
    let psl = emit_psl_property(&prop);
    assert!(psl.contains("assert always"), "PSL should use 'assert always'. Got: {}", psl);
}

// ═══════════════════════════════════════════════════════════════════════════
// MONITOR GENERATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn monitor_generates_struct_and_check() {
    use logicaffeine_compile::codegen_sva::{SvaProperty, SvaAssertionKind, emit_rust_monitor};
    let prop = SvaProperty {
        name: "p_data_check".to_string(),
        clock: "clk".to_string(),
        body: "valid |=> (data_out == data_in)".to_string(),
        kind: SvaAssertionKind::Assert,
    };
    let monitor = emit_rust_monitor(&prop);
    assert!(monitor.contains("struct"), "Monitor should contain struct. Got: {}", monitor);
    assert!(monitor.contains("fn check"), "Monitor should contain check fn. Got: {}", monitor);
}
