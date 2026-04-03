//! Specification Coverage Metrics
//!
//! Computes how well a set of SVA properties covers the specification KG.

use logicaffeine_language::semantics::knowledge_graph::HwKnowledgeGraph;
use serde::{Serialize, Deserialize};

/// Coverage metrics for a specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecCoverage {
    pub signal_coverage: f64,
    pub property_coverage: f64,
    pub edge_coverage: f64,
    pub temporal_coverage: f64,
    pub uncovered_signals: Vec<String>,
    pub uncovered_properties: Vec<String>,
}

/// Compute specification coverage given a KG and a set of covered signal names.
pub fn compute_coverage(kg: &HwKnowledgeGraph, covered_signals: &[String]) -> SpecCoverage {
    let total_signals = kg.signals.len();
    let total_properties = kg.properties.len();
    let total_edges = kg.edges.len();

    let covered_count = kg.signals.iter()
        .filter(|s| covered_signals.iter().any(|c| c == &s.name))
        .count();

    let uncovered_signals: Vec<String> = kg.signals.iter()
        .filter(|s| !covered_signals.iter().any(|c| c == &s.name))
        .map(|s| s.name.clone())
        .collect();

    let signal_coverage = if total_signals > 0 {
        covered_count as f64 / total_signals as f64
    } else {
        0.0
    };

    // Edge coverage: fraction of edges where BOTH endpoints are covered
    let covered_edges = kg.edges.iter()
        .filter(|e| covered_signals.iter().any(|c| c == &e.from)
            && covered_signals.iter().any(|c| c == &e.to))
        .count();
    let edge_coverage = if total_edges > 0 {
        covered_edges as f64 / total_edges as f64
    } else {
        0.0
    };

    // Property coverage: fraction of properties whose associated edge signals are covered
    let covered_props = kg.properties.iter()
        .filter(|p| {
            // A property is covered if its associated edge has both endpoints covered
            kg.edges.iter().any(|e| {
                e.property.as_deref() == Some(&p.name)
                    && covered_signals.iter().any(|c| c == &e.from)
                    && covered_signals.iter().any(|c| c == &e.to)
            })
        })
        .count();
    let property_coverage = if total_properties > 0 {
        covered_props as f64 / total_properties as f64
    } else {
        0.0
    };

    // Temporal coverage: temporal properties (safety/liveness) that are covered
    let temporal_props = kg.properties.iter()
        .filter(|p| p.property_type == "safety" || p.property_type == "liveness")
        .count();
    let covered_temporal = kg.properties.iter()
        .filter(|p| {
            (p.property_type == "safety" || p.property_type == "liveness")
                && kg.edges.iter().any(|e| {
                    e.property.as_deref() == Some(&p.name)
                        && covered_signals.iter().any(|c| c == &e.from)
                        && covered_signals.iter().any(|c| c == &e.to)
                })
        })
        .count();
    let temporal_coverage = if temporal_props > 0 {
        covered_temporal as f64 / temporal_props as f64
    } else {
        0.0
    };

    // Uncovered properties: only those NOT covered
    let uncovered_properties: Vec<String> = kg.properties.iter()
        .filter(|p| {
            !kg.edges.iter().any(|e| {
                e.property.as_deref() == Some(&p.name)
                    && covered_signals.iter().any(|c| c == &e.from)
                    && covered_signals.iter().any(|c| c == &e.to)
            })
        })
        .map(|p| p.name.clone())
        .collect();

    SpecCoverage {
        signal_coverage,
        property_coverage,
        edge_coverage,
        temporal_coverage,
        uncovered_signals,
        uncovered_properties,
    }
}
