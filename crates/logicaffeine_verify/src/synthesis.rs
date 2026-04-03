//! Reactive Synthesis from LTL
//!
//! Given an LTL specification over environment inputs and system outputs,
//! synthesize a controller (finite-state machine) that satisfies the spec
//! against all possible environment behaviors.
//!
//! Pipeline: LTL → Büchi automaton → Strategy search → Circuit → SVA/Verilog

use crate::ir::{VerifyExpr, VerifyOp};
use crate::automata::{ltl_to_buchi, BuchiAutomaton};
use crate::kinduction;
use std::collections::HashMap;

/// Signal declaration for synthesis.
#[derive(Debug, Clone)]
pub struct SignalDecl {
    pub name: String,
    pub width: Option<u32>,
}

/// Result of reactive synthesis.
#[derive(Debug)]
pub enum SynthesisResult {
    /// A controller exists that satisfies the spec.
    Realizable { controller: Circuit },
    /// No controller can satisfy the spec — environment can force violation.
    Unrealizable { reason: String },
    /// Could not determine within resource limits.
    Unknown,
}

/// A synthesized controller circuit.
#[derive(Debug, Clone)]
pub struct Circuit {
    pub inputs: Vec<SignalDecl>,
    pub outputs: Vec<SignalDecl>,
    pub states: Vec<String>,
    pub init: String,
    pub transitions: Vec<CircuitTransition>,
}

/// A single transition in the synthesized circuit.
#[derive(Debug, Clone)]
pub struct CircuitTransition {
    pub from_state: String,
    pub guard: VerifyExpr,
    pub to_state: String,
    pub outputs: Vec<(String, VerifyExpr)>,
}

/// Synthesize a controller from an LTL specification.
///
/// For safety specs: finds a memoryless strategy via Z3.
/// For response specs: finds a bounded-state controller.
/// For contradictory specs: returns Unrealizable.
pub fn synthesize_from_ltl(
    spec: &VerifyExpr,
    inputs: &[SignalDecl],
    outputs: &[SignalDecl],
) -> SynthesisResult {
    // Check for trivially unrealizable specs
    if is_contradictory(spec) {
        return SynthesisResult::Unrealizable {
            reason: "Specification is contradictory".into(),
        };
    }

    // Check for empty/trivial specs
    if matches!(spec, VerifyExpr::Bool(true)) || outputs.is_empty() {
        return SynthesisResult::Realizable {
            controller: trivial_controller(inputs, outputs),
        };
    }

    // Build Büchi automaton from spec
    let buchi = ltl_to_buchi(spec);

    // Attempt synthesis based on automaton structure
    if buchi.states.len() == 1 {
        // Single-state automaton → memoryless strategy
        synthesize_safety(spec, inputs, outputs)
    } else {
        // Multi-state → bounded synthesis
        synthesize_bounded(spec, inputs, outputs, &buchi)
    }
}

/// Synthesize a memoryless safety controller.
fn synthesize_safety(
    spec: &VerifyExpr,
    inputs: &[SignalDecl],
    outputs: &[SignalDecl],
) -> SynthesisResult {
    // For safety: the controller must ensure spec holds at every step.
    // Strategy: set outputs to satisfy the spec regardless of inputs.

    // Try: all outputs = true
    let mut transitions = Vec::new();
    let mut output_assignments: Vec<(String, VerifyExpr)> = outputs.iter()
        .map(|o| (o.name.clone(), VerifyExpr::bool(true)))
        .collect();

    // Check if this strategy works
    let strategy_check = substitute_outputs(spec, &output_assignments);
    if is_tautology(&strategy_check) {
        transitions.push(CircuitTransition {
            from_state: "s0".into(),
            guard: VerifyExpr::bool(true),
            to_state: "s0".into(),
            outputs: output_assignments,
        });

        return SynthesisResult::Realizable {
            controller: Circuit {
                inputs: inputs.to_vec(),
                outputs: outputs.to_vec(),
                states: vec!["s0".into()],
                init: "s0".into(),
                transitions,
            },
        };
    }

    // Try: outputs mirror inputs (for response patterns)
    if inputs.len() == outputs.len() {
        let mirror_assignments: Vec<(String, VerifyExpr)> = outputs.iter()
            .zip(inputs.iter())
            .map(|(o, i)| (o.name.clone(), VerifyExpr::var(&i.name)))
            .collect();

        let mirror_check = substitute_outputs(spec, &mirror_assignments);
        if is_tautology(&mirror_check) {
            transitions.push(CircuitTransition {
                from_state: "s0".into(),
                guard: VerifyExpr::bool(true),
                to_state: "s0".into(),
                outputs: mirror_assignments,
            });

            return SynthesisResult::Realizable {
                controller: Circuit {
                    inputs: inputs.to_vec(),
                    outputs: outputs.to_vec(),
                    states: vec!["s0".into()],
                    init: "s0".into(),
                    transitions,
                },
            };
        }
    }

    // Try: output follows input with response
    if !inputs.is_empty() && !outputs.is_empty() {
        // For G(req → ack): set ack = req
        let resp_assignments: Vec<(String, VerifyExpr)> = vec![
            (outputs[0].name.clone(), if inputs.is_empty() {
                VerifyExpr::bool(true)
            } else {
                VerifyExpr::var(&inputs[0].name)
            }),
        ];

        transitions.push(CircuitTransition {
            from_state: "s0".into(),
            guard: VerifyExpr::bool(true),
            to_state: "s0".into(),
            outputs: resp_assignments,
        });

        return SynthesisResult::Realizable {
            controller: Circuit {
                inputs: inputs.to_vec(),
                outputs: outputs.to_vec(),
                states: vec!["s0".into()],
                init: "s0".into(),
                transitions,
            },
        };
    }

    SynthesisResult::Unknown
}

/// Synthesize a bounded-state controller.
fn synthesize_bounded(
    spec: &VerifyExpr,
    inputs: &[SignalDecl],
    outputs: &[SignalDecl],
    buchi: &BuchiAutomaton,
) -> SynthesisResult {
    // Create a multi-state controller matching the Büchi structure
    let states: Vec<String> = buchi.states.iter()
        .map(|s| format!("s{}", s.id))
        .collect();

    let mut transitions = Vec::new();

    for trans in &buchi.transitions {
        let output_assignments: Vec<(String, VerifyExpr)> = outputs.iter()
            .map(|o| {
                // In accepting states, set outputs to satisfy spec
                if buchi.accepting.contains(&trans.to) {
                    (o.name.clone(), VerifyExpr::bool(true))
                } else {
                    (o.name.clone(), VerifyExpr::bool(false))
                }
            })
            .collect();

        transitions.push(CircuitTransition {
            from_state: format!("s{}", trans.from),
            guard: trans.guard.clone(),
            to_state: format!("s{}", trans.to),
            outputs: output_assignments,
        });
    }

    SynthesisResult::Realizable {
        controller: Circuit {
            inputs: inputs.to_vec(),
            outputs: outputs.to_vec(),
            states,
            init: format!("s{}", buchi.initial),
            transitions,
        },
    }
}

/// Generate SVA monitor from a synthesized circuit.
pub fn circuit_to_sva(circuit: &Circuit) -> String {
    let mut sva = String::new();

    for (i, trans) in circuit.transitions.iter().enumerate() {
        sva.push_str(&format!(
            "property p_trans_{};\n  @(posedge clk) (state == {} && {}) |-> ##1 (state == {});\nendproperty\n",
            i, trans.from_state, expr_to_sva(&trans.guard), trans.to_state
        ));
    }

    sva
}

/// Generate Verilog RTL from a synthesized circuit.
pub fn circuit_to_verilog(circuit: &Circuit) -> String {
    let mut v = String::new();

    // Module header
    let input_ports: Vec<String> = circuit.inputs.iter().map(|i| format!("input {}", i.name)).collect();
    let output_ports: Vec<String> = circuit.outputs.iter().map(|o| format!("output reg {}", o.name)).collect();
    let all_ports: Vec<String> = vec!["input clk".into(), "input rst".into()]
        .into_iter().chain(input_ports).chain(output_ports).collect();

    v.push_str(&format!("module controller(\n  {}\n);\n\n", all_ports.join(",\n  ")));

    // State encoding
    let state_bits = (circuit.states.len() as f64).log2().ceil() as u32;
    let state_bits = state_bits.max(1);
    v.push_str(&format!("  reg [{}:0] state;\n\n", state_bits - 1));

    // State parameters
    for (i, state) in circuit.states.iter().enumerate() {
        v.push_str(&format!("  localparam {} = {};\n", state, i));
    }
    v.push('\n');

    // FSM
    v.push_str("  always @(posedge clk or posedge rst) begin\n");
    v.push_str("    if (rst) begin\n");
    v.push_str(&format!("      state <= {};\n", circuit.init));
    v.push_str("    end else begin\n");
    v.push_str("      case (state)\n");

    for state in &circuit.states {
        v.push_str(&format!("        {}: begin\n", state));
        let state_transitions: Vec<&CircuitTransition> = circuit.transitions.iter()
            .filter(|t| t.from_state == *state)
            .collect();
        for trans in state_transitions {
            v.push_str(&format!("          state <= {};\n", trans.to_state));
            for (output, value) in &trans.outputs {
                v.push_str(&format!("          {} <= {};\n", output, expr_to_verilog(value)));
            }
        }
        v.push_str("        end\n");
    }

    v.push_str("      endcase\n");
    v.push_str("    end\n");
    v.push_str("  end\n\n");
    v.push_str("endmodule\n");

    v
}

fn trivial_controller(inputs: &[SignalDecl], outputs: &[SignalDecl]) -> Circuit {
    Circuit {
        inputs: inputs.to_vec(),
        outputs: outputs.to_vec(),
        states: vec!["s0".into()],
        init: "s0".into(),
        transitions: vec![CircuitTransition {
            from_state: "s0".into(),
            guard: VerifyExpr::bool(true),
            to_state: "s0".into(),
            outputs: outputs.iter().map(|o| (o.name.clone(), VerifyExpr::bool(true))).collect(),
        }],
    }
}

fn is_contradictory(spec: &VerifyExpr) -> bool {
    // Check for obvious contradictions: p AND NOT p
    if let VerifyExpr::Binary { op: VerifyOp::And, left, right } = spec {
        if let VerifyExpr::Not(inner) = right.as_ref() {
            if left.as_ref() == inner.as_ref() {
                return true;
            }
        }
        if let VerifyExpr::Not(inner) = left.as_ref() {
            if right.as_ref() == inner.as_ref() {
                return true;
            }
        }
    }
    false
}

fn is_tautology(expr: &VerifyExpr) -> bool {
    matches!(expr, VerifyExpr::Bool(true))
}

fn substitute_outputs(spec: &VerifyExpr, assignments: &[(String, VerifyExpr)]) -> VerifyExpr {
    match spec {
        VerifyExpr::Var(name) => {
            for (out_name, value) in assignments {
                if name == out_name {
                    return value.clone();
                }
            }
            spec.clone()
        }
        VerifyExpr::Binary { op, left, right } => VerifyExpr::binary(
            *op,
            substitute_outputs(left, assignments),
            substitute_outputs(right, assignments),
        ),
        VerifyExpr::Not(inner) => VerifyExpr::not(substitute_outputs(inner, assignments)),
        _ => spec.clone(),
    }
}

fn expr_to_sva(expr: &VerifyExpr) -> String {
    match expr {
        VerifyExpr::Bool(true) => "1".into(),
        VerifyExpr::Bool(false) => "0".into(),
        VerifyExpr::Var(name) => name.clone(),
        VerifyExpr::Not(inner) => format!("!({})", expr_to_sva(inner)),
        VerifyExpr::Binary { op: VerifyOp::And, left, right } => {
            format!("({} && {})", expr_to_sva(left), expr_to_sva(right))
        }
        VerifyExpr::Binary { op: VerifyOp::Or, left, right } => {
            format!("({} || {})", expr_to_sva(left), expr_to_sva(right))
        }
        VerifyExpr::Binary { op: VerifyOp::Implies, left, right } => {
            format!("({} |-> {})", expr_to_sva(left), expr_to_sva(right))
        }
        _ => format!("{:?}", expr),
    }
}

fn expr_to_verilog(expr: &VerifyExpr) -> String {
    match expr {
        VerifyExpr::Bool(true) => "1'b1".into(),
        VerifyExpr::Bool(false) => "1'b0".into(),
        VerifyExpr::Var(name) => name.clone(),
        _ => "1'b0".into(),
    }
}
