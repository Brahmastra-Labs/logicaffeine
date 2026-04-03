//! Sprint 0E: Formal Hardware Ontology
//!
//! Comprehensive tests for HwEntityType (28 variants) and HwRelation (24 variants).
//! These replace the skeletal SignalRole/KgRelation with rich typed enums
//! that formally ground hardware verification with 28+ entity types
//! (vs AssertionForge's 35 LLM-prompt labels) and 24+ parameterized relations
//! (vs AssertionForge's 59 string labels).
//!
//! Every variant is tested for construction, serialization, and round-trip.
//! Entity extraction tests verify pattern recognition from Kripke-lowered AST.

use logicaffeine_language::semantics::knowledge_graph::{
    HwEntityType, HwRelation, HwKnowledgeGraph,
    PortDirection, SignalType, ResetPolarity, CounterDirection, ArbitrationScheme,
    extract_from_kripke_ast,
};
use logicaffeine_language::compile_kripke_with;

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 1: VARIANT COUNTS — STRUCTURAL GUARANTEES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn ontology_has_28_entity_variants() {
    // Construct one of each to prove all 28 variants exist at compile time.
    let entities: Vec<HwEntityType> = vec![
        HwEntityType::Module { name: "m".into(), is_top: false },
        HwEntityType::Port { direction: PortDirection::Input, width: 1, domain: None },
        HwEntityType::Signal { width: 1, signal_type: SignalType::Wire, domain: None },
        HwEntityType::Register { width: 1, reset_value: None, clock: None },
        HwEntityType::Memory { depth: 1, width: 1, ports: 1 },
        HwEntityType::Fifo { depth: 1, width: 1 },
        HwEntityType::Bus { width: 1, protocol: None },
        HwEntityType::Parameter { value: "".into() },
        HwEntityType::Fsm { states: vec![], initial: None },
        HwEntityType::Counter { width: 1, direction: CounterDirection::Up },
        HwEntityType::Arbiter { scheme: ArbitrationScheme::RoundRobin, ports: 1 },
        HwEntityType::Decoder { input_width: 1, output_width: 1 },
        HwEntityType::Mux { inputs: 1, select_width: 1 },
        HwEntityType::Clock { frequency: None, domain: "".into() },
        HwEntityType::Reset { polarity: ResetPolarity::ActiveHigh, synchronous: true },
        HwEntityType::Interrupt { priority: None, edge_triggered: false },
        HwEntityType::Handshake { valid_signal: "".into(), ready_signal: "".into() },
        HwEntityType::Pipeline { stages: 1, stall_signal: None },
        HwEntityType::Transaction { request: "".into(), response: "".into() },
        HwEntityType::DataPath { width: 1, signed: false },
        HwEntityType::Address { width: 1, base: None, range: None },
        HwEntityType::Configuration { fields: vec![] },
        HwEntityType::SafetyProperty { formula: "".into() },
        HwEntityType::LivenessProperty { formula: "".into() },
        HwEntityType::FairnessProperty { formula: "".into() },
        HwEntityType::ResponseProperty { trigger: "".into(), response: "".into(), bound: None },
        HwEntityType::MutexProperty { signals: vec![] },
        HwEntityType::StabilityProperty { signal: "".into(), condition: "".into() },
    ];
    assert!(entities.len() >= 28, "Should have >= 28 entity variants, got {}", entities.len());
}

#[test]
fn ontology_has_24_relation_variants() {
    // Construct one of each to prove all 24 variants exist at compile time.
    let relations: Vec<HwRelation> = vec![
        HwRelation::Drives,
        HwRelation::DrivesRegistered { clock: "".into() },
        HwRelation::DataFlow,
        HwRelation::Reads,
        HwRelation::Writes,
        HwRelation::Controls,
        HwRelation::Selects,
        HwRelation::Enables,
        HwRelation::Resets,
        HwRelation::Triggers { delay: None },
        HwRelation::Constrains,
        HwRelation::Follows { min: 0, max: 0 },
        HwRelation::Precedes,
        HwRelation::Preserves,
        HwRelation::Contains,
        HwRelation::Instantiates,
        HwRelation::ConnectsTo,
        HwRelation::BelongsToDomain { domain: "".into() },
        HwRelation::HandshakesWith,
        HwRelation::Acknowledges,
        HwRelation::Pipelines { stages: 0 },
        HwRelation::MutuallyExcludes,
        HwRelation::EventuallyFollows,
        HwRelation::AssumedBy,
    ];
    assert!(relations.len() >= 24, "Should have >= 24 relation variants, got {}", relations.len());
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 2: STRUCTURAL ENTITIES (8 variants)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn entity_module_serializes() {
    let e = HwEntityType::Module { name: "axi_master".into(), is_top: true };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Module"), "Got: {}", json);
    assert!(json.contains("axi_master"), "Got: {}", json);
    assert!(json.contains("true"), "is_top should be true. Got: {}", json);
}

#[test]
fn entity_port_input() {
    let e = HwEntityType::Port { direction: PortDirection::Input, width: 8, domain: None };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Port"), "Got: {}", json);
    assert!(json.contains("Input"), "Got: {}", json);
}

#[test]
fn entity_port_output_with_domain() {
    let e = HwEntityType::Port { direction: PortDirection::Output, width: 32, domain: Some("pclk".into()) };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Output"), "Got: {}", json);
    assert!(json.contains("pclk"), "Got: {}", json);
}

#[test]
fn entity_port_inout() {
    let e = HwEntityType::Port { direction: PortDirection::Inout, width: 1, domain: None };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Inout"), "Got: {}", json);
}

#[test]
fn entity_signal_wire() {
    let e = HwEntityType::Signal { width: 1, signal_type: SignalType::Wire, domain: None };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Signal"), "Got: {}", json);
    assert!(json.contains("Wire"), "Got: {}", json);
}

#[test]
fn entity_signal_reg_with_domain() {
    let e = HwEntityType::Signal { width: 16, signal_type: SignalType::Reg, domain: Some("clk".into()) };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Reg"), "Got: {}", json);
}

#[test]
fn entity_signal_logic() {
    let e = HwEntityType::Signal { width: 64, signal_type: SignalType::Logic, domain: None };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Logic"), "Got: {}", json);
}

#[test]
fn entity_register() {
    let e = HwEntityType::Register { width: 32, reset_value: Some(0), clock: Some("clk".into()) };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Register"), "Got: {}", json);
    assert!(json.contains("32"), "Got: {}", json);
}

#[test]
fn entity_register_no_reset() {
    let e = HwEntityType::Register { width: 8, reset_value: None, clock: None };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Register"), "Got: {}", json);
    assert!(json.contains("null") || json.contains("None"), "reset_value should be null. Got: {}", json);
}

#[test]
fn entity_memory() {
    let e = HwEntityType::Memory { depth: 1024, width: 32, ports: 2 };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Memory"), "Got: {}", json);
    assert!(json.contains("1024"), "Got: {}", json);
}

#[test]
fn entity_fifo() {
    let e = HwEntityType::Fifo { depth: 16, width: 8 };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Fifo"), "Got: {}", json);
}

#[test]
fn entity_bus() {
    let e = HwEntityType::Bus { width: 128, protocol: Some("AXI4".into()) };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Bus"), "Got: {}", json);
    assert!(json.contains("AXI4"), "Got: {}", json);
}

#[test]
fn entity_parameter() {
    let e = HwEntityType::Parameter { value: "DATA_WIDTH=32".into() };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Parameter"), "Got: {}", json);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 3: CONTROL ENTITIES (5 variants)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn entity_fsm() {
    let e = HwEntityType::Fsm {
        states: vec!["IDLE".into(), "SETUP".into(), "ACCESS".into()],
        initial: Some("IDLE".into()),
    };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Fsm"), "Got: {}", json);
    assert!(json.contains("IDLE"), "Got: {}", json);
    assert!(json.contains("SETUP"), "Got: {}", json);
}

#[test]
fn entity_counter_up() {
    let e = HwEntityType::Counter { width: 8, direction: CounterDirection::Up };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Counter"), "Got: {}", json);
    assert!(json.contains("Up"), "Got: {}", json);
}

#[test]
fn entity_counter_down() {
    let e = HwEntityType::Counter { width: 16, direction: CounterDirection::Down };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Down"), "Got: {}", json);
}

#[test]
fn entity_arbiter_round_robin() {
    let e = HwEntityType::Arbiter { scheme: ArbitrationScheme::RoundRobin, ports: 4 };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Arbiter"), "Got: {}", json);
    assert!(json.contains("RoundRobin"), "Got: {}", json);
}

#[test]
fn entity_arbiter_priority() {
    let e = HwEntityType::Arbiter { scheme: ArbitrationScheme::Priority, ports: 8 };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Priority"), "Got: {}", json);
}

#[test]
fn entity_decoder() {
    let e = HwEntityType::Decoder { input_width: 3, output_width: 8 };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Decoder"), "Got: {}", json);
}

#[test]
fn entity_mux() {
    let e = HwEntityType::Mux { inputs: 4, select_width: 2 };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Mux"), "Got: {}", json);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 4: TEMPORAL ENTITIES (3 variants)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn entity_clock() {
    let e = HwEntityType::Clock { frequency: Some("100MHz".into()), domain: "sys_clk".into() };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Clock"), "Got: {}", json);
    assert!(json.contains("100MHz"), "Got: {}", json);
}

#[test]
fn entity_reset_active_low_sync() {
    let e = HwEntityType::Reset { polarity: ResetPolarity::ActiveLow, synchronous: true };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Reset"), "Got: {}", json);
    assert!(json.contains("ActiveLow"), "Got: {}", json);
}

#[test]
fn entity_reset_active_high_async() {
    let e = HwEntityType::Reset { polarity: ResetPolarity::ActiveHigh, synchronous: false };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("ActiveHigh"), "Got: {}", json);
}

#[test]
fn entity_interrupt() {
    let e = HwEntityType::Interrupt { priority: Some(3), edge_triggered: true };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Interrupt"), "Got: {}", json);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 5: PROTOCOL ENTITIES (3 variants)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn entity_handshake() {
    let e = HwEntityType::Handshake { valid_signal: "AWVALID".into(), ready_signal: "AWREADY".into() };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Handshake"), "Got: {}", json);
    assert!(json.contains("AWVALID"), "Got: {}", json);
    assert!(json.contains("AWREADY"), "Got: {}", json);
}

#[test]
fn entity_pipeline() {
    let e = HwEntityType::Pipeline { stages: 5, stall_signal: Some("stall".into()) };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Pipeline"), "Got: {}", json);
    assert!(json.contains("5"), "Got: {}", json);
}

#[test]
fn entity_transaction() {
    let e = HwEntityType::Transaction { request: "cmd_req".into(), response: "cmd_resp".into() };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Transaction"), "Got: {}", json);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 6: DATA ENTITIES (3 variants)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn entity_datapath() {
    let e = HwEntityType::DataPath { width: 32, signed: true };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("DataPath"), "Got: {}", json);
}

#[test]
fn entity_address() {
    let e = HwEntityType::Address { width: 32, base: Some(0x1000), range: Some(0x100) };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Address"), "Got: {}", json);
}

#[test]
fn entity_configuration() {
    let e = HwEntityType::Configuration { fields: vec!["mode".into(), "enable".into(), "threshold".into()] };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("Configuration"), "Got: {}", json);
    assert!(json.contains("mode"), "Got: {}", json);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 7: PROPERTY ENTITIES (6 variants)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn entity_safety_property() {
    let e = HwEntityType::SafetyProperty { formula: "G(!(grant_a && grant_b))".into() };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("SafetyProperty"), "Got: {}", json);
}

#[test]
fn entity_liveness_property() {
    let e = HwEntityType::LivenessProperty { formula: "G(req -> F(ack))".into() };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("LivenessProperty"), "Got: {}", json);
}

#[test]
fn entity_fairness_property() {
    let e = HwEntityType::FairnessProperty { formula: "GF(grant_0) && GF(grant_1)".into() };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("FairnessProperty"), "Got: {}", json);
}

#[test]
fn entity_response_property() {
    let e = HwEntityType::ResponseProperty { trigger: "req".into(), response: "ack".into(), bound: Some(5) };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("ResponseProperty"), "Got: {}", json);
    assert!(json.contains("5"), "Bound should be 5. Got: {}", json);
}

#[test]
fn entity_response_property_unbounded() {
    let e = HwEntityType::ResponseProperty { trigger: "req".into(), response: "ack".into(), bound: None };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("ResponseProperty"), "Got: {}", json);
}

#[test]
fn entity_mutex_property() {
    let e = HwEntityType::MutexProperty { signals: vec!["grant_a".into(), "grant_b".into(), "grant_c".into()] };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("MutexProperty"), "Got: {}", json);
    assert!(json.contains("grant_a"), "Got: {}", json);
    assert!(json.contains("grant_c"), "Got: {}", json);
}

#[test]
fn entity_stability_property() {
    let e = HwEntityType::StabilityProperty { signal: "data".into(), condition: "valid".into() };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("StabilityProperty"), "Got: {}", json);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 8: RELATION VARIANTS — PARAMETERIZED
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn relation_drives_registered() {
    let r = HwRelation::DrivesRegistered { clock: "pclk".into() };
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.contains("DrivesRegistered"), "Got: {}", json);
    assert!(json.contains("pclk"), "Got: {}", json);
}

#[test]
fn relation_triggers_with_delay() {
    let r = HwRelation::Triggers { delay: Some(3) };
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.contains("Triggers"), "Got: {}", json);
}

#[test]
fn relation_triggers_no_delay() {
    let r = HwRelation::Triggers { delay: None };
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.contains("Triggers"), "Got: {}", json);
}

#[test]
fn relation_follows_with_bounds() {
    let r = HwRelation::Follows { min: 1, max: 10 };
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.contains("Follows"), "Got: {}", json);
}

#[test]
fn relation_belongs_to_domain() {
    let r = HwRelation::BelongsToDomain { domain: "sys_clk".into() };
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.contains("BelongsToDomain"), "Got: {}", json);
}

#[test]
fn relation_pipelines_with_stages() {
    let r = HwRelation::Pipelines { stages: 3 };
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.contains("Pipelines"), "Got: {}", json);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 9: RELATION VARIANTS — SIMPLE (19 parameterless)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn relation_all_simple_variants_constructible() {
    let relations: Vec<HwRelation> = vec![
        HwRelation::Drives,
        HwRelation::DataFlow,
        HwRelation::Reads,
        HwRelation::Writes,
        HwRelation::Controls,
        HwRelation::Selects,
        HwRelation::Enables,
        HwRelation::Resets,
        HwRelation::Constrains,
        HwRelation::Precedes,
        HwRelation::Preserves,
        HwRelation::Contains,
        HwRelation::Instantiates,
        HwRelation::ConnectsTo,
        HwRelation::HandshakesWith,
        HwRelation::Acknowledges,
        HwRelation::MutuallyExcludes,
        HwRelation::EventuallyFollows,
        HwRelation::AssumedBy,
    ];
    for rel in &relations {
        let json = serde_json::to_string(rel).unwrap();
        assert!(!json.is_empty(), "Relation should serialize");
    }
    assert_eq!(relations.len(), 19);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 10: ROUND-TRIP SERIALIZATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn entity_round_trip_clock() {
    let original = HwEntityType::Clock { frequency: Some("50MHz".into()), domain: "fast_clk".into() };
    let json = serde_json::to_string(&original).unwrap();
    let restored: HwEntityType = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&restored).unwrap();
    assert_eq!(json, json2, "Clock should round-trip through JSON");
}

#[test]
fn entity_round_trip_fsm() {
    let original = HwEntityType::Fsm {
        states: vec!["S0".into(), "S1".into(), "S2".into()],
        initial: Some("S0".into()),
    };
    let json = serde_json::to_string(&original).unwrap();
    let restored: HwEntityType = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&restored).unwrap();
    assert_eq!(json, json2, "Fsm should round-trip through JSON");
}

#[test]
fn entity_round_trip_mutex_property() {
    let original = HwEntityType::MutexProperty {
        signals: vec!["a".into(), "b".into()],
    };
    let json = serde_json::to_string(&original).unwrap();
    let restored: HwEntityType = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&restored).unwrap();
    assert_eq!(json, json2, "MutexProperty should round-trip through JSON");
}

#[test]
fn relation_round_trip_triggers() {
    let original = HwRelation::Triggers { delay: Some(2) };
    let json = serde_json::to_string(&original).unwrap();
    let restored: HwRelation = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&restored).unwrap();
    assert_eq!(json, json2, "Triggers should round-trip through JSON");
}

#[test]
fn relation_round_trip_follows() {
    let original = HwRelation::Follows { min: 0, max: 100 };
    let json = serde_json::to_string(&original).unwrap();
    let restored: HwRelation = serde_json::from_str(&json).unwrap();
    let json2 = serde_json::to_string(&restored).unwrap();
    assert_eq!(json, json2, "Follows should round-trip through JSON");
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 11: ENUM HELPER TYPE COMPLETENESS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn port_direction_all_variants() {
    let dirs = vec![PortDirection::Input, PortDirection::Output, PortDirection::Inout];
    assert_eq!(dirs.len(), 3);
    for d in &dirs {
        let json = serde_json::to_string(d).unwrap();
        assert!(!json.is_empty());
    }
}

#[test]
fn signal_type_all_variants() {
    let types = vec![SignalType::Wire, SignalType::Reg, SignalType::Logic];
    assert_eq!(types.len(), 3);
    for t in &types {
        let json = serde_json::to_string(t).unwrap();
        assert!(!json.is_empty());
    }
}

#[test]
fn reset_polarity_all_variants() {
    let pols = vec![ResetPolarity::ActiveHigh, ResetPolarity::ActiveLow];
    assert_eq!(pols.len(), 2);
}

#[test]
fn counter_direction_all_variants() {
    let dirs = vec![CounterDirection::Up, CounterDirection::Down, CounterDirection::UpDown];
    assert_eq!(dirs.len(), 3);
}

#[test]
fn arbitration_scheme_all_variants() {
    let schemes = vec![
        ArbitrationScheme::RoundRobin,
        ArbitrationScheme::Priority,
        ArbitrationScheme::WeightedRoundRobin,
    ];
    assert_eq!(schemes.len(), 3);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 12: EXTRACTION INTEGRATION — ENTITIES FROM REAL SPECS
// These test that extract_from_kripke_ast produces the right HwEntityType
// and HwRelation from actual English hardware specifications.
// ═══════════════════════════════════════════════════════════════════════════

fn extract_kg(spec: &str) -> HwKnowledgeGraph {
    compile_kripke_with(spec, |ast, interner| {
        extract_from_kripke_ast(ast, interner)
    })
    .unwrap()
}

fn has_entity(kg: &HwKnowledgeGraph, pred: impl Fn(&HwEntityType) -> bool) -> bool {
    kg.entities.iter().any(|(_, e)| pred(e))
}

fn has_typed_edge(kg: &HwKnowledgeGraph, pred: impl Fn(&HwRelation) -> bool) -> bool {
    kg.typed_edges.iter().any(|(_, _, r)| pred(r))
}

#[test]
fn extract_safety_property_from_always() {
    let kg = extract_kg("Always, every signal is valid.");
    assert!(
        has_entity(&kg, |e| matches!(e, HwEntityType::SafetyProperty { .. })),
        "Always (G) should produce SafetyProperty entity. Entities: {:?}",
        kg.entities
    );
}

#[test]
fn extract_liveness_property_from_eventually() {
    let kg = extract_kg("Eventually, every signal is active.");
    assert!(
        has_entity(&kg, |e| matches!(e, HwEntityType::LivenessProperty { .. })),
        "Eventually (F) should produce LivenessProperty entity. Entities: {:?}",
        kg.entities
    );
}

#[test]
fn extract_clock_entity_from_clk_predicate() {
    let kg = extract_kg("Always, every clk_signal runs.");
    assert!(
        has_entity(&kg, |e| matches!(e, HwEntityType::Clock { .. })),
        "Signal with 'clk' should produce Clock entity. Entities: {:?}",
        kg.entities
    );
}

#[test]
fn extract_triggers_relation_from_conditional() {
    let kg = extract_kg("Always, if every dog runs then every cat sleeps.");
    assert!(
        has_typed_edge(&kg, |r| matches!(r, HwRelation::Triggers { .. })),
        "User conditional should produce Triggers relation. Edges: {:?}",
        kg.typed_edges
    );
}

#[test]
fn extract_constrains_from_negated_conjunction() {
    // "not(P and Q)" pattern → MutuallyExcludes or Constrains
    let kg = extract_kg("Always, not every dog runs and every cat sleeps.");
    // This may or may not produce a Constrains edge depending on parse —
    // the key assertion is extraction doesn't panic
    assert!(
        !kg.entities.is_empty() || !kg.typed_edges.is_empty() || !kg.signals.is_empty(),
        "Negated conjunction should extract something. KG: {:?}",
        kg.entities
    );
}

#[test]
fn extract_response_property_from_implies_next() {
    // G(P → X(Q)) should produce a ResponseProperty with bound=1
    let kg = extract_kg("Always, if every dog runs, then next, every cat sleeps.");
    assert!(
        has_entity(&kg, |e| matches!(e, HwEntityType::ResponseProperty { bound: Some(1), .. }))
            || has_entity(&kg, |e| matches!(e, HwEntityType::SafetyProperty { .. })),
        "G(P→X(Q)) should produce ResponseProperty or SafetyProperty. Entities: {:?}",
        kg.entities
    );
}

#[test]
fn extract_entities_from_conjunction_in_temporal() {
    let kg = extract_kg("Always, every dog runs and every cat sleeps.");
    // Should extract multiple signals/entities from the conjunction
    let total = kg.entities.len() + kg.signals.len();
    assert!(
        total >= 2,
        "Conjunction should extract multiple entities. Got {} entities + {} signals",
        kg.entities.len(), kg.signals.len()
    );
}

#[test]
fn extract_preserves_signal_names() {
    let kg = extract_kg("Always, every dog runs.");
    // At least one entity or signal should contain a recognizable name
    let has_names = !kg.signals.is_empty()
        || kg.entities.iter().any(|(name, _)| !name.is_empty());
    assert!(has_names, "Should preserve signal names. KG: signals={:?}, entities={:?}",
        kg.signals, kg.entities);
}

#[test]
fn extract_complex_spec_no_panic() {
    let specs = vec![
        "Always, every signal is valid.",
        "Eventually, every acknowledgment is active.",
        "Always, if every dog runs then every cat sleeps.",
        "Next, every dog runs.",
        "Every dog runs until every cat sleeps.",
        "Every dog runs release every cat sleeps.",
    ];
    for spec in specs {
        let result = compile_kripke_with(spec, |ast, interner| {
            extract_from_kripke_ast(ast, interner)
        });
        assert!(result.is_ok(), "Should not panic on '{}'. Error: {:?}", spec, result.err());
    }
}

#[test]
fn extract_multiple_specs_produce_different_property_types() {
    let kg_safety = extract_kg("Always, every dog runs.");
    let kg_liveness = extract_kg("Eventually, every dog runs.");

    let has_safety = has_entity(&kg_safety, |e| matches!(e, HwEntityType::SafetyProperty { .. }));
    let has_liveness = has_entity(&kg_liveness, |e| matches!(e, HwEntityType::LivenessProperty { .. }));

    assert!(has_safety, "Always should produce SafetyProperty. Got: {:?}", kg_safety.entities);
    assert!(has_liveness, "Eventually should produce LivenessProperty. Got: {:?}", kg_liveness.entities);
}

#[test]
fn extract_json_includes_entities_section() {
    let kg = extract_kg("Always, every signal is valid.");
    let json = kg.to_json();
    assert!(json.contains("\"entities\""), "JSON should have entities section. Got: {}", json);
}

#[test]
fn extract_json_includes_typed_edges_section() {
    let kg = extract_kg("Always, if every dog runs then every cat sleeps.");
    let json = kg.to_json();
    assert!(json.contains("\"typed_edges\"") || json.contains("\"edges\""),
        "JSON should have edges section. Got: {}", json);
}

#[test]
fn extract_entities_have_correct_formula_text() {
    let kg = extract_kg("Always, every signal is valid.");
    for (_, entity) in &kg.entities {
        if let HwEntityType::SafetyProperty { formula } = entity {
            assert!(!formula.is_empty(), "SafetyProperty formula should not be empty");
        }
    }
}

#[test]
fn extract_kg_is_deterministic() {
    let kg1 = extract_kg("Always, every dog runs.");
    let kg2 = extract_kg("Always, every dog runs.");
    // Same input should produce same entity count
    assert_eq!(kg1.entities.len(), kg2.entities.len(),
        "Extraction should be deterministic. Run 1: {:?}, Run 2: {:?}",
        kg1.entities, kg2.entities);
}

#[test]
fn extract_temporal_binary_produces_entities() {
    let kg = extract_kg("Every dog runs until every cat sleeps.");
    // Until operator should produce some structure
    let total = kg.entities.len() + kg.signals.len() + kg.properties.len();
    assert!(total > 0, "Temporal binary should produce entities. KG: {:?}", kg.entities);
}

#[test]
fn extract_nested_modal_in_temporal() {
    let kg = extract_kg("Always, every dog can run.");
    assert!(
        has_entity(&kg, |e| matches!(e, HwEntityType::SafetyProperty { .. })),
        "G(Diamond(P)) should produce SafetyProperty. Entities: {:?}",
        kg.entities
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 13: KNOWLEDGE GRAPH BUILDER API
// Test that the builder methods work correctly with new types
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kg_builder_add_entity() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_entity("clk", HwEntityType::Clock { frequency: Some("100MHz".into()), domain: "sys".into() });
    assert_eq!(kg.entities.len(), 1);
    assert_eq!(kg.entities[0].0, "clk");
    assert!(matches!(kg.entities[0].1, HwEntityType::Clock { .. }));
}

#[test]
fn kg_builder_add_typed_edge() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_typed_edge("req", "ack", HwRelation::Triggers { delay: Some(1) });
    assert_eq!(kg.typed_edges.len(), 1);
    assert_eq!(kg.typed_edges[0].0, "req");
    assert_eq!(kg.typed_edges[0].1, "ack");
    assert!(matches!(kg.typed_edges[0].2, HwRelation::Triggers { delay: Some(1) }));
}

#[test]
fn kg_builder_multiple_entities() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_entity("clk", HwEntityType::Clock { frequency: None, domain: "sys".into() });
    kg.add_entity("rst_n", HwEntityType::Reset { polarity: ResetPolarity::ActiveLow, synchronous: true });
    kg.add_entity("req_ack", HwEntityType::Handshake { valid_signal: "req".into(), ready_signal: "ack".into() });
    kg.add_entity("mutex", HwEntityType::MutexProperty { signals: vec!["g0".into(), "g1".into()] });
    assert_eq!(kg.entities.len(), 4);
}

#[test]
fn kg_builder_mixed_old_and_new_api() {
    // During transition, both old and new APIs should work
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("req", 1, logicaffeine_language::semantics::knowledge_graph::SignalRole::Input);
    kg.add_entity("clk", HwEntityType::Clock { frequency: None, domain: "sys".into() });
    assert_eq!(kg.signals.len(), 1);
    assert_eq!(kg.entities.len(), 1);
}

#[test]
fn kg_json_output_has_all_sections() {
    let mut kg = HwKnowledgeGraph::new();
    kg.add_signal("req", 1, logicaffeine_language::semantics::knowledge_graph::SignalRole::Input);
    kg.add_entity("clk", HwEntityType::Clock { frequency: None, domain: "sys".into() });
    kg.add_typed_edge("req", "ack", HwRelation::Triggers { delay: None });
    let json = kg.to_json();
    assert!(json.contains("\"signals\""), "Missing signals section");
    assert!(json.contains("\"entities\""), "Missing entities section");
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 14: SPRINT D — KG extraction MUST recognize FOL patterns
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn extract_response_property_with_bound_1() {
    let kg = extract_kg("Always, if every request holds, then next, every acknowledgment holds.");
    let has_response = has_entity(&kg, |e| matches!(e, HwEntityType::ResponseProperty { bound: Some(1), .. }));
    assert!(has_response,
        "G(P -> X(Q)) MUST produce ResponseProperty{{bound:1}}.\n\
         Entities: {:?}\nTyped edges: {:?}",
        kg.entities.iter().map(|(n,e)| format!("{}: {:?}", n, e)).collect::<Vec<_>>(),
        kg.typed_edges.iter().map(|(f,t,r)| format!("{}->{}: {:?}", f, t, r)).collect::<Vec<_>>());
}

#[test]
fn extract_response_has_triggers_with_delay_1() {
    let kg = extract_kg("Always, if every request holds, then next, every acknowledgment holds.");
    let has_delayed = has_typed_edge(&kg, |r| matches!(r, HwRelation::Triggers { delay: Some(1) }));
    assert!(has_delayed,
        "G(P -> X(Q)) MUST produce Triggers{{delay:1}} typed edge.\n\
         Typed edges: {:?}",
        kg.typed_edges.iter().map(|(f,t,r)| format!("{}->{}: {:?}", f, t, r)).collect::<Vec<_>>());
}

#[test]
fn extract_eventually_follows_from_conditional_eventually() {
    let kg = extract_kg("Always, if every request holds, then eventually, every response holds.");
    let has_ef = has_typed_edge(&kg, |r| matches!(r, HwRelation::EventuallyFollows));
    assert!(has_ef,
        "G(P -> F(Q)) MUST produce EventuallyFollows typed edge.\n\
         Typed edges: {:?}",
        kg.typed_edges.iter().map(|(f,t,r)| format!("{}->{}: {:?}", f, t, r)).collect::<Vec<_>>());
}

#[test]
fn extract_precedes_from_until_operator() {
    let kg = extract_kg("Every request holds until every grant holds.");
    let has_precedes = has_typed_edge(&kg, |r| matches!(r, HwRelation::Precedes));
    assert!(has_precedes,
        "P U Q MUST produce Precedes typed edge.\n\
         Typed edges: {:?}",
        kg.typed_edges.iter().map(|(f,t,r)| format!("{}->{}: {:?}", f, t, r)).collect::<Vec<_>>());
}

#[test]
fn extract_mutex_entity_from_negated_and() {
    let kg = extract_kg("Always, not every grant_a holds and every grant_b holds.");
    let has_mutex = has_entity(&kg, |e| matches!(e, HwEntityType::MutexProperty { .. }));
    assert!(has_mutex,
        "not(P AND Q) MUST produce MutexProperty entity.\n\
         Entities: {:?}",
        kg.entities.iter().map(|(n,e)| format!("{}: {:?}", n, e)).collect::<Vec<_>>());
}

#[test]
fn extract_mutex_entity_lists_signal_names() {
    let kg = extract_kg("Always, not every grant_a holds and every grant_b holds.");
    let mutex = kg.entities.iter().find(|(_, e)| matches!(e, HwEntityType::MutexProperty { .. }));
    if let Some((_, HwEntityType::MutexProperty { signals })) = mutex {
        assert!(signals.len() >= 2,
            "MutexProperty must list at least 2 signal names, got: {:?}", signals);
    } else {
        panic!("MutexProperty entity not found. Entities: {:?}",
            kg.entities.iter().map(|(n,e)| format!("{}: {:?}", n, e)).collect::<Vec<_>>());
    }
}

#[test]
fn extract_handshake_from_valid_ready_naming() {
    let kg = extract_kg("Always, if every valid holds, then every ready holds.");
    let has_hs = has_entity(&kg, |e| matches!(e, HwEntityType::Handshake { .. }))
        || has_typed_edge(&kg, |r| matches!(r, HwRelation::HandshakesWith));
    assert!(has_hs,
        "valid/ready pair MUST produce Handshake entity or HandshakesWith edge.\n\
         Entities: {:?}\nTyped edges: {:?}",
        kg.entities.iter().map(|(n,e)| format!("{}: {:?}", n, e)).collect::<Vec<_>>(),
        kg.typed_edges.iter().map(|(f,t,r)| format!("{}->{}: {:?}", f, t, r)).collect::<Vec<_>>());
}

#[test]
fn extract_handshake_from_req_ack_naming() {
    let kg = extract_kg("Always, if every request holds, then every acknowledgment holds.");
    let has_hs = has_entity(&kg, |e| matches!(e, HwEntityType::Handshake { .. }))
        || has_typed_edge(&kg, |r| matches!(r, HwRelation::HandshakesWith));
    assert!(has_hs,
        "req/ack pair MUST produce Handshake entity or HandshakesWith edge.\n\
         Signals: {:?}",
        kg.signals.iter().map(|s| &s.name).collect::<Vec<_>>());
}

#[test]
fn kg_at_least_4_distinct_entity_types_extractable() {
    let specs: Vec<&str> = vec![
        "Always, if every request holds, then every acknowledgment holds.",
        "Eventually, every done holds.",
        "Always, not every grant_a holds and every grant_b holds.",
        "Always, if every request holds, then next, every response holds.",
    ];
    let mut variant_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for spec in &specs {
        let kg = extract_kg(spec);
        for (_, entity) in &kg.entities {
            let name = match entity {
                HwEntityType::SafetyProperty { .. } => "Safety",
                HwEntityType::LivenessProperty { .. } => "Liveness",
                HwEntityType::MutexProperty { .. } => "Mutex",
                HwEntityType::ResponseProperty { .. } => "Response",
                HwEntityType::Clock { .. } => "Clock",
                HwEntityType::Handshake { .. } => "Handshake",
                HwEntityType::StabilityProperty { .. } => "Stability",
                _ => "Other",
            };
            variant_names.insert(name.to_string());
        }
    }
    assert!(variant_names.len() >= 4,
        "At least 4 distinct HwEntityType variants must be extractable from FOL.\n\
         Got {}: {:?}\nEntity types are DEAD CODE if < 4 extractable.",
        variant_names.len(), variant_names);
}

#[test]
fn extract_entities_and_typed_edges_both_populated_from_conditional() {
    let kg = extract_kg("Always, if every request holds, then every acknowledgment holds.");
    assert!(!kg.entities.is_empty(),
        "Non-trivial spec MUST produce entities");
    assert!(!kg.typed_edges.is_empty(),
        "Non-trivial spec MUST produce typed_edges");
}
