//! Property Sufficiency Analysis
//!
//! Before running verification, check if your properties are sufficient
//! to cover the spec.

use logicaffeine_language::semantics::knowledge_graph::HwKnowledgeGraph;
use serde::{Serialize, Deserialize};

/// Sufficiency analysis report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SufficiencyReport {
    pub lonely_signals: Vec<String>,
    pub unconstrained_outputs: Vec<String>,
    pub missing_handshakes: Vec<(String, String)>,
    pub coverage_ratio: f64,
    pub recommendations: Vec<String>,
}

/// Known handshake pairing patterns: (trigger_pattern, response_patterns)
const HANDSHAKE_PAIRS: &[(&str, &[&str])] = &[
    ("req", &["ack", "gnt", "grant"]),
    ("request", &["acknowledge", "acknowledgment", "response", "grant"]),
    ("valid", &["ready", "rdy"]),
    ("cmd", &["resp", "response"]),
    ("start", &["done", "complete"]),
];

/// Analyze property sufficiency from a KG.
pub fn analyze_sufficiency(kg: &HwKnowledgeGraph) -> SufficiencyReport {
    use logicaffeine_language::semantics::knowledge_graph::SignalRole;

    let signal_names: std::collections::HashSet<String> = kg.signals.iter().map(|s| s.name.clone()).collect();
    let edge_signals: std::collections::HashSet<String> = kg.edges.iter()
        .flat_map(|e| vec![e.from.clone(), e.to.clone()])
        .collect();

    // Lonely signals: not mentioned in any edge
    let lonely_signals: Vec<String> = signal_names.iter()
        .filter(|s| !edge_signals.contains(*s))
        .cloned()
        .collect();

    // Unconstrained outputs: output signals with no driving edge
    let unconstrained_outputs: Vec<String> = kg.signals.iter()
        .filter(|s| s.role == SignalRole::Output)
        .filter(|s| !kg.edges.iter().any(|e| e.to == s.name))
        .map(|s| s.name.clone())
        .collect();

    // Missing handshake pairs: detect common naming patterns without edges
    let mut missing_handshakes: Vec<(String, String)> = Vec::new();
    for (trigger_pattern, response_patterns) in HANDSHAKE_PAIRS {
        let trigger_match = kg.signals.iter().find(|s| {
            s.name.to_lowercase().contains(trigger_pattern)
        });
        if let Some(trigger_sig) = trigger_match {
            for resp_pattern in *response_patterns {
                let resp_match = kg.signals.iter().find(|s| {
                    let lower = s.name.to_lowercase();
                    lower.contains(resp_pattern) && s.name != trigger_sig.name
                });
                if let Some(resp_sig) = resp_match {
                    // Check if there's already a direct edge between them
                    let has_edge = kg.edges.iter().any(|e| {
                        (e.from == trigger_sig.name && e.to == resp_sig.name)
                        || (e.from == resp_sig.name && e.to == trigger_sig.name)
                    });
                    if !has_edge {
                        missing_handshakes.push((trigger_sig.name.clone(), resp_sig.name.clone()));
                    }
                    break; // Only match first response pattern
                }
            }
        }
    }

    // Coverage ratio
    let total = signal_names.len() + kg.edges.len();
    let covered = edge_signals.len();
    let coverage_ratio = if total > 0 { covered as f64 / total as f64 } else { 0.0 };

    // Recommendations
    let mut recommendations = Vec::new();
    for sig in &lonely_signals {
        recommendations.push(format!("Signal '{}' is not constrained by any property", sig));
    }
    for sig in &unconstrained_outputs {
        recommendations.push(format!("Output '{}' has no driving property", sig));
    }
    for (trigger, response) in &missing_handshakes {
        recommendations.push(format!("Missing handshake: '{}' has no edge to '{}'", trigger, response));
    }

    SufficiencyReport {
        lonely_signals,
        unconstrained_outputs,
        missing_handshakes,
        coverage_ratio,
        recommendations,
    }
}
