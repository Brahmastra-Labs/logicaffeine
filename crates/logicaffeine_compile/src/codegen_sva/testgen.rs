//! Formal-to-Simulation Bridge: Testbench Generation from Counterexamples
//!
//! Extracts directed test vectors from Z3 counterexample traces and generates
//! SystemVerilog testbenches that reproduce the violation scenario.

use logicaffeine_verify::equivalence::{Trace, CycleState, SignalValue};
use logicaffeine_verify::kinduction::SignalDecl;

/// Generate a complete SystemVerilog testbench from a counterexample trace.
///
/// The testbench instantiates the DUT, generates a clock, and drives signals
/// per the counterexample cycle values.
pub fn trace_to_testbench(trace: &Trace, module_name: &str) -> String {
    let mut tb = String::new();
    let tb_name = format!("tb_{}", module_name);

    // Module header
    tb.push_str(&format!("module {};\n\n", tb_name));

    // Clock generation
    tb.push_str("  reg clk;\n");
    tb.push_str("  initial clk = 0;\n");
    tb.push_str("  always #5 clk = ~clk;\n\n");

    // Collect all signal names from trace
    let mut all_signals: Vec<String> = Vec::new();
    for cycle in &trace.cycles {
        for name in cycle.signals.keys() {
            if !all_signals.contains(name) {
                all_signals.push(name.clone());
            }
        }
    }
    all_signals.sort();

    // Declare signals
    for sig in &all_signals {
        // Determine width from first occurrence
        let width = trace.cycles.iter()
            .find_map(|c| c.signals.get(sig))
            .map(|v| match v {
                SignalValue::BitVec { width, .. } => *width,
                _ => 1,
            })
            .unwrap_or(1);

        if width > 1 {
            tb.push_str(&format!("  reg [{:}:0] {};\n", width - 1, sig));
        } else {
            tb.push_str(&format!("  reg {};\n", sig));
        }
    }
    tb.push('\n');

    // DUT instantiation
    tb.push_str(&format!("  {} dut(.*);\n\n", module_name));

    // Stimulus
    tb.push_str("  initial begin\n");

    if trace.cycles.is_empty() {
        tb.push_str("    // No stimulus (empty trace)\n");
    } else {
        for cycle in &trace.cycles {
            tb.push_str(&format!("    // Cycle {}\n", cycle.cycle));
            tb.push_str("    @(posedge clk);\n");
            for sig in &all_signals {
                if let Some(val) = cycle.signals.get(sig) {
                    let formatted = format_signal_value(sig, val);
                    tb.push_str(&format!("    {} = {};\n", sig, formatted));
                }
            }
        }

        // Display violation at last cycle
        if let Some(last) = trace.cycles.last() {
            tb.push_str(&format!("\n    $display(\"Violation at cycle {}\");\n", last.cycle));
        }
    }

    tb.push_str("    $finish;\n");
    tb.push_str("  end\n\n");
    tb.push_str("endmodule\n");

    tb
}

/// Generate stimulus-only assignments from a trace (no module wrapper).
pub fn trace_to_stimulus(trace: &Trace, signals: &[SignalDecl]) -> String {
    let mut stim = String::new();

    for cycle in &trace.cycles {
        stim.push_str(&format!("// Cycle {}\n", cycle.cycle));
        stim.push_str("#10;\n");
        for sig_decl in signals {
            if let Some(val) = cycle.signals.get(&sig_decl.name) {
                let formatted = format_signal_value(&sig_decl.name, val);
                stim.push_str(&format!("{} = {};\n", sig_decl.name, formatted));
            }
        }
    }

    stim
}

/// Format a signal value as a SystemVerilog literal.
fn format_signal_value(_name: &str, val: &SignalValue) -> String {
    match val {
        SignalValue::Bool(true) => "1'b1".into(),
        SignalValue::Bool(false) => "1'b0".into(),
        SignalValue::Int(n) => n.to_string(),
        SignalValue::BitVec { width, value } => {
            let hex_digits = ((*width + 3) / 4) as usize;
            format!("{}'h{:0>width$X}", width, value, width = hex_digits)
        }
        SignalValue::Unknown => "1'bx".into(),
    }
}
