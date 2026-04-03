//! Waveform Generation from Counterexamples
//!
//! When Z3 finds a spec-SVA divergence, renders the counterexample
//! as VCD (Value Change Dump) or ASCII timing diagrams.

use logicaffeine_verify::equivalence::{Trace, CycleState};
use std::collections::HashMap;

/// Generate a VCD (Value Change Dump) file from a counterexample trace.
///
/// VCD is the industry-standard waveform format readable by GTKWave,
/// Synopsys DVE, and other waveform viewers.
pub fn trace_to_vcd(trace: &Trace, signal_names: &[String]) -> String {
    if trace.cycles.is_empty() {
        return String::new();
    }

    let mut vcd = String::new();

    // Header
    vcd.push_str("$timescale 1ns $end\n");
    vcd.push_str("$scope module top $end\n");

    // Declare signals (use single-char identifiers)
    let id_chars: Vec<char> = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
        .chars().collect();
    let mut signal_ids: HashMap<String, char> = HashMap::new();
    for (i, name) in signal_names.iter().enumerate() {
        let id = id_chars.get(i).copied().unwrap_or('?');
        signal_ids.insert(name.clone(), id);
        vcd.push_str(&format!("$var wire 1 {} {} $end\n", id, name));
    }

    vcd.push_str("$upscope $end\n");
    vcd.push_str("$enddefinitions $end\n");

    // Value changes
    for cycle in &trace.cycles {
        vcd.push_str(&format!("#{}\n", cycle.cycle));
        for name in signal_names {
            if let Some(&id) = signal_ids.get(name) {
                let value = cycle.signals.get(name).copied().unwrap_or(false);
                vcd.push_str(&format!("{}{}\n", if value { '1' } else { '0' }, id));
            }
        }
    }

    vcd
}

/// Generate an ASCII waveform diagram from a counterexample trace.
pub fn trace_to_ascii_waveform(trace: &Trace) -> String {
    if trace.cycles.is_empty() {
        return String::new();
    }

    // Collect all signal names
    let mut all_signals: Vec<String> = Vec::new();
    for cycle in &trace.cycles {
        for name in cycle.signals.keys() {
            if !all_signals.contains(name) {
                all_signals.push(name.clone());
            }
        }
    }
    all_signals.sort();

    // Find max signal name length for alignment
    let max_name_len = all_signals.iter().map(|s| s.len()).max().unwrap_or(0);

    let mut output = String::new();

    // Cycle header
    output.push_str(&format!("{:width$} |", "", width = max_name_len));
    for cycle in &trace.cycles {
        output.push_str(&format!(" {} |", cycle.cycle));
    }
    output.push('\n');

    // Signal waveforms
    for sig_name in &all_signals {
        output.push_str(&format!("{:>width$} |", sig_name, width = max_name_len));
        for cycle in &trace.cycles {
            let value = cycle.signals.get(sig_name).copied().unwrap_or(false);
            let ch = if value { " ^ " } else { " _ " };
            output.push_str(&format!("{}|", ch));
        }
        output.push('\n');
    }

    output
}

/// Mark divergence cycles in an ASCII waveform.
/// A divergence cycle is where the FOL and SVA differ.
pub fn mark_divergence(waveform: &str, divergence_cycle: usize) -> String {
    let mut lines: Vec<String> = waveform.lines().map(|l| l.to_string()).collect();
    if !lines.is_empty() {
        // Add a marker line
        let header = &lines[0];
        let mut marker = String::new();
        let parts: Vec<&str> = header.split('|').collect();
        for (i, part) in parts.iter().enumerate() {
            if i == divergence_cycle + 1 {
                // +1 because first part is signal name column
                marker.push_str(&"*".repeat(part.len()));
            } else {
                marker.push_str(&" ".repeat(part.len()));
            }
            if i < parts.len() - 1 {
                marker.push('|');
            }
        }
        lines.push(marker);
    }
    lines.join("\n")
}
