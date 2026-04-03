//! Sprint 6A: Waveform Generation from Counterexamples

#![cfg(feature = "verification")]

use logicaffeine_compile::codegen_sva::waveform::{trace_to_vcd, trace_to_ascii_waveform, mark_divergence};
use logicaffeine_verify::equivalence::{Trace, CycleState, SignalValue};
use std::collections::HashMap;

fn make_trace(cycles: Vec<Vec<(&str, bool)>>) -> Trace {
    Trace {
        cycles: cycles.into_iter().enumerate().map(|(i, sigs)| {
            CycleState {
                cycle: i,
                signals: sigs.into_iter().map(|(n, v)| (n.to_string(), SignalValue::Bool(v))).collect(),
            }
        }).collect(),
    }
}

#[test]
fn waveform_has_all_signals() {
    let trace = make_trace(vec![
        vec![("req", true), ("ack", false)],
        vec![("req", true), ("ack", true)],
    ]);
    let waveform = trace_to_ascii_waveform(&trace);
    assert!(waveform.contains("req"), "Should contain req. Got:\n{}", waveform);
    assert!(waveform.contains("ack"), "Should contain ack. Got:\n{}", waveform);
}

#[test]
fn vcd_header_correct() {
    let trace = make_trace(vec![
        vec![("clk", true), ("data", false)],
    ]);
    let vcd = trace_to_vcd(&trace, &["clk".into(), "data".into()]);
    assert!(vcd.contains("$timescale"), "Missing timescale. Got:\n{}", vcd);
    assert!(vcd.contains("$scope"), "Missing scope. Got:\n{}", vcd);
    assert!(vcd.contains("$var"), "Missing var declarations. Got:\n{}", vcd);
    assert!(vcd.contains("clk"), "Missing clk signal. Got:\n{}", vcd);
    assert!(vcd.contains("data"), "Missing data signal. Got:\n{}", vcd);
}

#[test]
fn vcd_timestamps_correct() {
    let trace = make_trace(vec![
        vec![("sig", true)],
        vec![("sig", false)],
        vec![("sig", true)],
    ]);
    let vcd = trace_to_vcd(&trace, &["sig".into()]);
    assert!(vcd.contains("#0"), "Missing timestamp 0. Got:\n{}", vcd);
    assert!(vcd.contains("#1"), "Missing timestamp 1. Got:\n{}", vcd);
    assert!(vcd.contains("#2"), "Missing timestamp 2. Got:\n{}", vcd);
}

#[test]
fn ascii_readable() {
    let trace = make_trace(vec![
        vec![("req", false), ("ack", false)],
        vec![("req", true), ("ack", false)],
        vec![("req", true), ("ack", true)],
    ]);
    let waveform = trace_to_ascii_waveform(&trace);
    assert!(waveform.contains("^") || waveform.contains("_"),
        "Should contain waveform characters. Got:\n{}", waveform);
}

#[test]
fn ascii_marks_divergence() {
    let trace = make_trace(vec![
        vec![("sig", true)],
        vec![("sig", false)],
    ]);
    let waveform = trace_to_ascii_waveform(&trace);
    let marked = mark_divergence(&waveform, 1);
    assert!(marked.contains("*"), "Should mark divergence with *. Got:\n{}", marked);
}

#[test]
fn empty_trace_empty_output() {
    let trace = Trace { cycles: vec![] };
    let vcd = trace_to_vcd(&trace, &[]);
    assert!(vcd.is_empty(), "Empty trace should produce empty VCD");
    let ascii = trace_to_ascii_waveform(&trace);
    assert!(ascii.is_empty(), "Empty trace should produce empty ASCII");
}

#[test]
fn multi_cycle_correct_length() {
    let mut cycles = Vec::new();
    for i in 0..10 {
        cycles.push(vec![("sig", i % 2 == 0)]);
    }
    let trace = make_trace(cycles);
    let waveform = trace_to_ascii_waveform(&trace);
    let line_count = waveform.lines().count();
    assert!(line_count >= 2, "10-cycle trace should have header + signal lines. Got {} lines", line_count);
}

#[test]
fn signal_names_match_kg() {
    let trace = make_trace(vec![
        vec![("AWVALID", true), ("AWREADY", false)],
    ]);
    let vcd = trace_to_vcd(&trace, &["AWVALID".into(), "AWREADY".into()]);
    assert!(vcd.contains("AWVALID"), "Should use KG signal names");
    assert!(vcd.contains("AWREADY"), "Should use KG signal names");
}

#[test]
fn mutex_violation_both_high() {
    let trace = make_trace(vec![
        vec![("grant_a", true), ("grant_b", true)],
    ]);
    let waveform = trace_to_ascii_waveform(&trace);
    // Both grants high in same cycle — visible violation
    let grant_a_line = waveform.lines().find(|l| l.contains("grant_a")).unwrap();
    let grant_b_line = waveform.lines().find(|l| l.contains("grant_b")).unwrap();
    assert!(grant_a_line.contains("^"), "grant_a should be high");
    assert!(grant_b_line.contains("^"), "grant_b should be high");
}

#[test]
fn liveness_violation_never_asserted() {
    let mut cycles = Vec::new();
    for _ in 0..5 {
        cycles.push(vec![("ack", false)]);
    }
    let trace = make_trace(cycles);
    let waveform = trace_to_ascii_waveform(&trace);
    let ack_line = waveform.lines().find(|l| l.contains("ack")).unwrap();
    assert!(!ack_line.contains("^"), "ack should never go high");
}
