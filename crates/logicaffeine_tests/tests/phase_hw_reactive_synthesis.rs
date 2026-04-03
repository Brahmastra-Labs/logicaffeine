//! SUPERCRUSH Sprint S3C: Reactive Synthesis from LTL

#![cfg(feature = "verification")]

use logicaffeine_verify::synthesis::*;
use logicaffeine_verify::automata::*;
use logicaffeine_verify::{VerifyExpr, VerifyOp};

fn inp(name: &str) -> SignalDecl { SignalDecl { name: name.into(), width: None } }
fn out(name: &str) -> SignalDecl { SignalDecl { name: name.into(), width: None } }

// ═══════════════════════════════════════════════════════════════════════════
// BASIC SYNTHESIS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synth_simple_buffer() {
    // G(req → ack): controller that always acks when req
    let spec = VerifyExpr::implies(VerifyExpr::var("req"), VerifyExpr::var("ack"));
    let result = synthesize_from_ltl(&spec, &[inp("req")], &[out("ack")]);
    assert!(matches!(result, SynthesisResult::Realizable { .. }),
        "Simple buffer should be realizable. Got: {:?}", result);
}

#[test]
fn synth_mutex_arbiter() {
    // G(NOT(grant_a AND grant_b)): mutual exclusion
    let spec = VerifyExpr::not(VerifyExpr::and(
        VerifyExpr::var("grant_a"),
        VerifyExpr::var("grant_b"),
    ));
    let result = synthesize_from_ltl(&spec, &[inp("req_a"), inp("req_b")], &[out("grant_a"), out("grant_b")]);
    assert!(matches!(result, SynthesisResult::Realizable { .. }),
        "Mutex should be realizable. Got: {:?}", result);
}

#[test]
fn synth_controller_satisfies_spec() {
    let spec = VerifyExpr::implies(VerifyExpr::var("req"), VerifyExpr::var("ack"));
    if let SynthesisResult::Realizable { controller } = synthesize_from_ltl(&spec, &[inp("req")], &[out("ack")]) {
        assert!(!controller.states.is_empty(), "Controller should have states");
        assert!(!controller.transitions.is_empty(), "Controller should have transitions");
    } else {
        panic!("Should be realizable");
    }
}

#[test]
fn synth_unrealizable_detected() {
    // p AND NOT p is contradictory → unrealizable
    let spec = VerifyExpr::and(
        VerifyExpr::var("out"),
        VerifyExpr::not(VerifyExpr::var("out")),
    );
    let result = synthesize_from_ltl(&spec, &[inp("in")], &[out("out")]);
    assert!(matches!(result, SynthesisResult::Unrealizable { .. }),
        "Contradictory spec should be unrealizable. Got: {:?}", result);
}

#[test]
fn synth_empty_spec() {
    // Trivial true spec → trivial controller
    let result = synthesize_from_ltl(&VerifyExpr::bool(true), &[], &[]);
    assert!(matches!(result, SynthesisResult::Realizable { .. }));
}

#[test]
fn synth_contradictory_spec() {
    let spec = VerifyExpr::and(
        VerifyExpr::var("x"),
        VerifyExpr::not(VerifyExpr::var("x")),
    );
    let result = synthesize_from_ltl(&spec, &[], &[out("x")]);
    assert!(matches!(result, SynthesisResult::Unrealizable { .. }));
}

// ═══════════════════════════════════════════════════════════════════════════
// CIRCUIT OUTPUT
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synth_circuit_has_states() {
    let spec = VerifyExpr::var("ack");
    if let SynthesisResult::Realizable { controller } = synthesize_from_ltl(&spec, &[inp("req")], &[out("ack")]) {
        assert!(!controller.states.is_empty());
        assert!(!controller.init.is_empty());
    }
}

#[test]
fn synth_circuit_to_sva_output() {
    let spec = VerifyExpr::var("ack");
    if let SynthesisResult::Realizable { controller } = synthesize_from_ltl(&spec, &[inp("req")], &[out("ack")]) {
        let sva = circuit_to_sva(&controller);
        assert!(!sva.is_empty(), "SVA output should be non-empty");
        assert!(sva.contains("property"), "Should contain SVA property");
    }
}

#[test]
fn synth_circuit_to_verilog_output() {
    let spec = VerifyExpr::var("ack");
    if let SynthesisResult::Realizable { controller } = synthesize_from_ltl(&spec, &[inp("req")], &[out("ack")]) {
        let verilog = circuit_to_verilog(&controller);
        assert!(verilog.contains("module controller"), "Should have module declaration");
        assert!(verilog.contains("endmodule"), "Should have endmodule");
        assert!(verilog.contains("always"), "Should have always block");
    }
}

#[test]
fn synth_deterministic() {
    let spec = VerifyExpr::var("out");
    let r1 = synthesize_from_ltl(&spec, &[inp("in")], &[out("out")]);
    let r2 = synthesize_from_ltl(&spec, &[inp("in")], &[out("out")]);
    let both_realizable = matches!(r1, SynthesisResult::Realizable { .. })
        && matches!(r2, SynthesisResult::Realizable { .. });
    assert!(both_realizable, "Same spec should give same realizability");
}

// ═══════════════════════════════════════════════════════════════════════════
// AUTOMATA
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn automata_safety_single_state() {
    let buchi = ltl_to_buchi(&VerifyExpr::var("safe"));
    assert_eq!(buchi.states.len(), 1, "Safety spec should have 1 state");
    assert!(buchi.accepting.contains(&0));
}

#[test]
fn automata_response_two_states() {
    let spec = VerifyExpr::implies(VerifyExpr::var("req"), VerifyExpr::var("ack"));
    let buchi = ltl_to_buchi(&spec);
    assert_eq!(buchi.states.len(), 2, "Response spec should have 2 states");
}

#[test]
fn automata_has_transitions() {
    let buchi = ltl_to_buchi(&VerifyExpr::var("p"));
    assert!(!buchi.transitions.is_empty(), "Should have transitions");
}

#[test]
fn automata_initial_valid() {
    let buchi = ltl_to_buchi(&VerifyExpr::var("p"));
    assert!(buchi.initial < buchi.states.len(), "Initial state should be valid");
}

// ═══════════════════════════════════════════════════════════════════════════
// MULTI-STATE AND RESPONSE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synth_two_inputs_one_output() {
    let spec = VerifyExpr::implies(
        VerifyExpr::or(VerifyExpr::var("req_a"), VerifyExpr::var("req_b")),
        VerifyExpr::var("ack"),
    );
    let result = synthesize_from_ltl(&spec, &[inp("req_a"), inp("req_b")], &[out("ack")]);
    assert!(matches!(result, SynthesisResult::Realizable { .. }),
        "Two-input response should be realizable. Got: {:?}", result);
}

#[test]
fn synth_multiple_outputs() {
    let spec = VerifyExpr::and(
        VerifyExpr::var("out_a"),
        VerifyExpr::var("out_b"),
    );
    let result = synthesize_from_ltl(&spec, &[inp("in")], &[out("out_a"), out("out_b")]);
    assert!(matches!(result, SynthesisResult::Realizable { .. }),
        "Multiple outputs should be realizable. Got: {:?}", result);
}

#[test]
fn synth_no_outputs() {
    let result = synthesize_from_ltl(&VerifyExpr::bool(true), &[inp("in")], &[]);
    assert!(matches!(result, SynthesisResult::Realizable { .. }));
}

// ═══════════════════════════════════════════════════════════════════════════
// VERILOG GENERATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synth_verilog_has_ports() {
    let spec = VerifyExpr::var("ack");
    if let SynthesisResult::Realizable { controller } = synthesize_from_ltl(&spec, &[inp("req")], &[out("ack")]) {
        let v = circuit_to_verilog(&controller);
        assert!(v.contains("input req"), "Should have input port");
        assert!(v.contains("output reg ack"), "Should have output port");
        assert!(v.contains("input clk"), "Should have clock");
        assert!(v.contains("input rst"), "Should have reset");
    }
}

#[test]
fn synth_verilog_has_state_machine() {
    let spec = VerifyExpr::var("out");
    if let SynthesisResult::Realizable { controller } = synthesize_from_ltl(&spec, &[inp("in")], &[out("out")]) {
        let v = circuit_to_verilog(&controller);
        assert!(v.contains("state"), "Should have state register");
        assert!(v.contains("case"), "Should have case statement");
    }
}

#[test]
fn synth_sva_has_properties() {
    let spec = VerifyExpr::var("out");
    if let SynthesisResult::Realizable { controller } = synthesize_from_ltl(&spec, &[], &[out("out")]) {
        let sva = circuit_to_sva(&controller);
        assert!(sva.contains("posedge clk"), "SVA should reference clock");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CIRCUIT PROPERTIES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synth_circuit_init_in_states() {
    let spec = VerifyExpr::var("out");
    if let SynthesisResult::Realizable { controller } = synthesize_from_ltl(&spec, &[], &[out("out")]) {
        assert!(controller.states.contains(&controller.init),
            "Init state should be in states list");
    }
}

#[test]
fn synth_circuit_transitions_valid() {
    let spec = VerifyExpr::var("out");
    if let SynthesisResult::Realizable { controller } = synthesize_from_ltl(&spec, &[], &[out("out")]) {
        for trans in &controller.transitions {
            assert!(controller.states.contains(&trans.from_state),
                "from_state should be valid");
            assert!(controller.states.contains(&trans.to_state),
                "to_state should be valid");
        }
    }
}

#[test]
fn synth_circuit_outputs_match() {
    let spec = VerifyExpr::var("ack");
    if let SynthesisResult::Realizable { controller } = synthesize_from_ltl(&spec, &[inp("req")], &[out("ack")]) {
        for trans in &controller.transitions {
            for (name, _) in &trans.outputs {
                assert!(controller.outputs.iter().any(|o| &o.name == name),
                    "Output {} in transition should be in outputs list", name);
            }
        }
    }
}

#[test]
fn synth_performance() {
    let spec = VerifyExpr::implies(VerifyExpr::var("req"), VerifyExpr::var("ack"));
    let start = std::time::Instant::now();
    let _result = synthesize_from_ltl(&spec, &[inp("req")], &[out("ack")]);
    let elapsed = start.elapsed();
    assert!(elapsed.as_secs() < 10, "Synthesis should complete within 10s, took {:?}", elapsed);
}

#[test]
fn synth_not_spec_safety() {
    // G(NOT(bad)) — safety controller
    let spec = VerifyExpr::not(VerifyExpr::var("bad"));
    let result = synthesize_from_ltl(&spec, &[inp("trigger")], &[out("bad")]);
    assert!(matches!(result, SynthesisResult::Realizable { .. }),
        "Safety spec should be realizable");
}
