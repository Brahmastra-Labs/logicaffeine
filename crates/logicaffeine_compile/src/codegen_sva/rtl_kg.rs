//! RTL KG + Spec-RTL Linking
//!
//! Converts RtlModule → HwKnowledgeGraph and links spec KG to RTL KG.

use super::rtl_extract::{RtlModule, PortDirection};
use logicaffeine_language::semantics::knowledge_graph::{
    HwKnowledgeGraph, SignalRole, KgRelation,
};

/// Convert an RtlModule to a HwKnowledgeGraph.
pub fn rtl_to_kg(module: &RtlModule) -> HwKnowledgeGraph {
    let mut kg = HwKnowledgeGraph::new();

    for port in &module.ports {
        let role = match &port.direction {
            PortDirection::Input => SignalRole::Input,
            PortDirection::Output => SignalRole::Output,
            PortDirection::Inout => SignalRole::Internal,
        };
        // Override for detected clocks
        let role = if module.clocks.contains(&port.name) {
            SignalRole::Clock
        } else {
            role
        };
        kg.add_signal(&port.name, port.width, role);
    }

    for sig in &module.signals {
        kg.add_signal(&sig.name, sig.width, SignalRole::Internal);
    }

    kg
}

/// Link result reporting unmatched signals.
#[derive(Debug)]
pub struct LinkResult {
    pub matched: Vec<(String, String)>,
    pub unmatched_spec: Vec<String>,
    pub unmatched_rtl: Vec<String>,
}

/// Link a spec KG to an RTL KG by signal name matching.
/// Strategy: exact match → case-insensitive match.
pub fn link_kg(spec_kg: &HwKnowledgeGraph, rtl_kg: &HwKnowledgeGraph) -> LinkResult {
    let mut matched = Vec::new();
    let mut used_rtl: std::collections::HashSet<String> = std::collections::HashSet::new();

    let rtl_names: Vec<String> = rtl_kg.signals.iter().map(|s| s.name.clone()).collect();

    for spec_sig in &spec_kg.signals {
        // Exact match
        if let Some(rtl_name) = rtl_names.iter().find(|n| **n == spec_sig.name) {
            matched.push((spec_sig.name.clone(), rtl_name.clone()));
            used_rtl.insert(rtl_name.clone());
            continue;
        }
        // Case-insensitive match
        if let Some(rtl_name) = rtl_names.iter().find(|n| n.to_lowercase() == spec_sig.name.to_lowercase()) {
            matched.push((spec_sig.name.clone(), rtl_name.clone()));
            used_rtl.insert(rtl_name.clone());
            continue;
        }
    }

    let unmatched_spec: Vec<String> = spec_kg.signals.iter()
        .filter(|s| !matched.iter().any(|(spec, _)| spec == &s.name))
        .map(|s| s.name.clone())
        .collect();

    let unmatched_rtl: Vec<String> = rtl_kg.signals.iter()
        .filter(|s| !used_rtl.contains(&s.name))
        .map(|s| s.name.clone())
        .collect();

    LinkResult { matched, unmatched_spec, unmatched_rtl }
}
