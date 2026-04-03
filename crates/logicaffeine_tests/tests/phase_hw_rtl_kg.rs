//! Sprint 2B: RTL KG + Spec-RTL Linking

use logicaffeine_compile::codegen_sva::rtl_extract::parse_verilog_module;
use logicaffeine_compile::codegen_sva::rtl_kg::{rtl_to_kg, link_kg};
use logicaffeine_language::semantics::knowledge_graph::{HwKnowledgeGraph, SignalRole};

#[test]
fn rtl_to_kg_ports_become_signals() {
    let src = "module m (\n  input clk,\n  input req,\n  output ack\n);\nendmodule";
    let module = parse_verilog_module(src).unwrap();
    let kg = rtl_to_kg(&module);
    assert_eq!(kg.signals.len(), 3);
}

#[test]
fn rtl_to_kg_input_port_is_input_role() {
    let src = "module m (\n  input req\n);\nendmodule";
    let module = parse_verilog_module(src).unwrap();
    let kg = rtl_to_kg(&module);
    assert!(kg.signals.iter().any(|s| s.name == "req" && s.role == SignalRole::Input));
}

#[test]
fn rtl_to_kg_output_port_is_output_role() {
    let src = "module m (\n  output ack\n);\nendmodule";
    let module = parse_verilog_module(src).unwrap();
    let kg = rtl_to_kg(&module);
    assert!(kg.signals.iter().any(|s| s.name == "ack" && s.role == SignalRole::Output));
}

#[test]
fn rtl_to_kg_clock_detected() {
    let src = "module m (\n  input clk\n);\n  always @(posedge clk) begin end\nendmodule";
    let module = parse_verilog_module(src).unwrap();
    let kg = rtl_to_kg(&module);
    assert!(kg.signals.iter().any(|s| s.name == "clk" && s.role == SignalRole::Clock),
        "Clock should be detected. Signals: {:?}", kg.signals);
}

#[test]
fn rtl_to_kg_preserves_width() {
    let src = "module m (\n  input [7:0] data\n);\nendmodule";
    let module = parse_verilog_module(src).unwrap();
    let kg = rtl_to_kg(&module);
    assert!(kg.signals.iter().any(|s| s.name == "data" && s.width == 8));
}

#[test]
fn rtl_to_kg_internal_signals() {
    let src = "module m;\n  wire internal_wire;\nendmodule";
    let module = parse_verilog_module(src).unwrap();
    let kg = rtl_to_kg(&module);
    assert!(kg.signals.iter().any(|s| s.name == "internal_wire" && s.role == SignalRole::Internal));
}

#[test]
fn link_exact_match() {
    let mut spec_kg = HwKnowledgeGraph::new();
    spec_kg.add_signal("clk", 1, SignalRole::Clock);
    spec_kg.add_signal("req", 1, SignalRole::Input);

    let mut rtl_kg = HwKnowledgeGraph::new();
    rtl_kg.add_signal("clk", 1, SignalRole::Clock);
    rtl_kg.add_signal("req", 1, SignalRole::Input);
    rtl_kg.add_signal("ack", 1, SignalRole::Output);

    let result = link_kg(&spec_kg, &rtl_kg);
    assert_eq!(result.matched.len(), 2, "Should match clk and req");
    assert_eq!(result.unmatched_rtl.len(), 1, "ack should be unmatched RTL");
}

#[test]
fn link_case_insensitive() {
    let mut spec_kg = HwKnowledgeGraph::new();
    spec_kg.add_signal("CLK", 1, SignalRole::Clock);

    let mut rtl_kg = HwKnowledgeGraph::new();
    rtl_kg.add_signal("clk", 1, SignalRole::Clock);

    let result = link_kg(&spec_kg, &rtl_kg);
    assert_eq!(result.matched.len(), 1, "CLK should match clk case-insensitively");
}

#[test]
fn link_unmatched_reported() {
    let mut spec_kg = HwKnowledgeGraph::new();
    spec_kg.add_signal("spec_only", 1, SignalRole::Internal);

    let mut rtl_kg = HwKnowledgeGraph::new();
    rtl_kg.add_signal("rtl_only", 1, SignalRole::Internal);

    let result = link_kg(&spec_kg, &rtl_kg);
    assert!(result.unmatched_spec.contains(&"spec_only".to_string()));
    assert!(result.unmatched_rtl.contains(&"rtl_only".to_string()));
}

#[test]
fn link_empty_kgs() {
    let spec_kg = HwKnowledgeGraph::new();
    let rtl_kg = HwKnowledgeGraph::new();
    let result = link_kg(&spec_kg, &rtl_kg);
    assert!(result.matched.is_empty());
    assert!(result.unmatched_spec.is_empty());
    assert!(result.unmatched_rtl.is_empty());
}
