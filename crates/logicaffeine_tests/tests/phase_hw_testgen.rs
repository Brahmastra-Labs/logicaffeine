//! SUPERCRUSH Sprint S4C: Formal-to-Simulation Bridge (Testbench Generation)

#![cfg(feature = "verification")]

use logicaffeine_compile::codegen_sva::testgen::{trace_to_testbench, trace_to_stimulus};
use logicaffeine_verify::equivalence::{Trace, CycleState, SignalValue};
use logicaffeine_verify::kinduction::SignalDecl;
use std::collections::HashMap;

fn make_trace(cycles: Vec<Vec<(&str, SignalValue)>>) -> Trace {
    Trace {
        cycles: cycles.into_iter().enumerate().map(|(i, sigs)| {
            CycleState {
                cycle: i,
                signals: sigs.into_iter().map(|(n, v)| (n.to_string(), v)).collect(),
            }
        }).collect(),
    }
}

fn sig(name: &str) -> SignalDecl {
    SignalDecl { name: name.into(), width: None }
}

#[test]
fn testgen_module_instantiation() {
    let trace = make_trace(vec![vec![("req", SignalValue::Bool(true))]]);
    let tb = trace_to_testbench(&trace, "my_dut");
    assert!(tb.contains("my_dut dut"), "Should instantiate DUT. Got:\n{}", tb);
}

#[test]
fn testgen_clock_generation() {
    let trace = make_trace(vec![vec![("req", SignalValue::Bool(true))]]);
    let tb = trace_to_testbench(&trace, "dut");
    assert!(tb.contains("always #5 clk"), "Should have clock driver. Got:\n{}", tb);
}

#[test]
fn testgen_stimulus_per_cycle() {
    let trace = make_trace(vec![
        vec![("sig", SignalValue::Bool(true))],
        vec![("sig", SignalValue::Bool(false))],
        vec![("sig", SignalValue::Bool(true))],
    ]);
    let tb = trace_to_testbench(&trace, "dut");
    let posedge_count = tb.matches("@(posedge clk)").count();
    assert_eq!(posedge_count, 3, "3 cycles should have 3 posedge blocks. Got:\n{}", tb);
}

#[test]
fn testgen_boolean_signal() {
    let trace = make_trace(vec![vec![("en", SignalValue::Bool(true))]]);
    let tb = trace_to_testbench(&trace, "dut");
    assert!(tb.contains("1'b1"), "Bool true should be 1'b1. Got:\n{}", tb);
}

#[test]
fn testgen_bitvec_signal() {
    let trace = make_trace(vec![vec![("data", SignalValue::BitVec { width: 8, value: 0xAB })]]);
    let tb = trace_to_testbench(&trace, "dut");
    assert!(tb.contains("8'h"), "BitVec should use width'h format. Got:\n{}", tb);
    assert!(tb.contains("AB") || tb.contains("ab"), "Should contain hex value AB. Got:\n{}", tb);
}

#[test]
fn testgen_integer_signal() {
    let trace = make_trace(vec![vec![("count", SignalValue::Int(42))]]);
    let tb = trace_to_testbench(&trace, "dut");
    assert!(tb.contains("42"), "Int should be decimal. Got:\n{}", tb);
}

#[test]
fn testgen_multi_cycle() {
    let mut cycles = Vec::new();
    for i in 0..10 {
        cycles.push(vec![("sig", SignalValue::Bool(i % 2 == 0))]);
    }
    let trace = make_trace(cycles);
    let tb = trace_to_testbench(&trace, "dut");
    let posedge_count = tb.matches("@(posedge clk)").count();
    assert_eq!(posedge_count, 10, "10 cycles should have 10 posedge blocks");
}

#[test]
fn testgen_valid_systemverilog() {
    let trace = make_trace(vec![vec![("req", SignalValue::Bool(true))]]);
    let tb = trace_to_testbench(&trace, "dut");
    assert!(tb.contains("module tb_dut"), "Should have module declaration");
    assert!(tb.contains("endmodule"), "Should have endmodule");
    assert!(tb.contains("initial begin"), "Should have initial block");
    assert!(tb.contains("end"), "Should have end");
}

#[test]
fn testgen_display_violation() {
    let trace = make_trace(vec![vec![("sig", SignalValue::Bool(false))]]);
    let tb = trace_to_testbench(&trace, "dut");
    assert!(tb.contains("$display"), "Should display violation. Got:\n{}", tb);
}

#[test]
fn testgen_finish_after_trace() {
    let trace = make_trace(vec![vec![("sig", SignalValue::Bool(true))]]);
    let tb = trace_to_testbench(&trace, "dut");
    assert!(tb.contains("$finish"), "Should have $finish. Got:\n{}", tb);
}

#[test]
fn testgen_multiple_signals() {
    let trace = make_trace(vec![vec![
        ("a", SignalValue::Bool(true)),
        ("b", SignalValue::Bool(false)),
        ("c", SignalValue::Int(7)),
        ("d", SignalValue::BitVec { width: 8, value: 0xFF }),
        ("e", SignalValue::Bool(true)),
    ]]);
    let tb = trace_to_testbench(&trace, "dut");
    for sig_name in &["a", "b", "c", "d", "e"] {
        assert!(tb.contains(sig_name), "Should contain signal {}. Got:\n{}", sig_name, tb);
    }
}

#[test]
fn testgen_empty_trace() {
    let trace = Trace { cycles: vec![] };
    let tb = trace_to_testbench(&trace, "dut");
    assert!(tb.contains("module"), "Empty trace should still produce valid module");
    assert!(tb.contains("$finish"), "Should still have $finish");
    assert!(!tb.contains("@(posedge clk)"), "No stimulus for empty trace");
}

#[test]
fn testgen_counterexample_driven() {
    let trace = make_trace(vec![
        vec![("grant_a", SignalValue::Bool(true)), ("grant_b", SignalValue::Bool(true))],
    ]);
    let tb = trace_to_testbench(&trace, "arbiter");
    assert!(tb.contains("grant_a = 1'b1"), "Should drive grant_a high");
    assert!(tb.contains("grant_b = 1'b1"), "Should drive grant_b high");
}

#[test]
fn testgen_serializable() {
    let trace = make_trace(vec![vec![("x", SignalValue::Bool(true))]]);
    let tb = trace_to_testbench(&trace, "dut");
    assert!(!tb.is_empty(), "Output should be a non-empty string");
}

#[test]
fn testgen_stimulus_only() {
    let trace = make_trace(vec![
        vec![("req", SignalValue::Bool(true))],
        vec![("req", SignalValue::Bool(false))],
    ]);
    let stim = trace_to_stimulus(&trace, &[sig("req")]);
    assert!(stim.contains("req ="), "Should have signal assignment");
    assert!(stim.contains("#10"), "Should have timing");
    assert!(!stim.contains("module"), "Stimulus-only should NOT have module wrapper");
}
