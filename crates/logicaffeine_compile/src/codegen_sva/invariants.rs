//! Invariant Discovery from Knowledge Graph
//!
//! Automatically extract candidate invariants from KG structure,
//! verify them with Z3.

use logicaffeine_verify::ir::VerifyExpr;
use logicaffeine_language::semantics::knowledge_graph::{
    HwKnowledgeGraph, KgRelation, HwEntityType, HwRelation,
};

/// Source of a discovered invariant.
#[derive(Debug, Clone, PartialEq)]
pub enum InvariantSource {
    MutexPattern,
    HandshakePattern,
    PipelineStability,
    ResetInit,
}

/// A candidate invariant discovered from the KG.
#[derive(Debug, Clone)]
pub struct CandidateInvariant {
    pub expr: VerifyExpr,
    pub source: InvariantSource,
    pub verified: Option<bool>,
}

/// Discover candidate invariants from a KG.
pub fn discover_invariants(kg: &HwKnowledgeGraph) -> Vec<CandidateInvariant> {
    let mut invariants = Vec::new();

    // Pattern 1: Constrains edge -> mutex invariant not(P AND Q)
    for edge in &kg.edges {
        if edge.relation == KgRelation::Constrains {
            let p = VerifyExpr::var(&edge.from);
            let q = VerifyExpr::var(&edge.to);
            invariants.push(CandidateInvariant {
                expr: VerifyExpr::not(VerifyExpr::and(p, q)),
                source: InvariantSource::MutexPattern,
                verified: None,
            });
        }
    }

    // Pattern 2: Triggers edge -> response invariant (P -> Q)
    for edge in &kg.edges {
        if edge.relation == KgRelation::Triggers {
            let p = VerifyExpr::var(&edge.from);
            let q = VerifyExpr::var(&edge.to);
            invariants.push(CandidateInvariant {
                expr: VerifyExpr::implies(p, q),
                source: InvariantSource::HandshakePattern,
                verified: None,
            });
        }
    }

    // Pattern 3: Pipeline entity -> stage stability invariant
    for (_, entity) in &kg.entities {
        if let HwEntityType::Pipeline { stages, .. } = entity {
            // Find typed edges that connect pipeline stages
            for typed_edge in &kg.typed_edges {
                if matches!(typed_edge.2, HwRelation::Triggers { .. }) {
                    let from = VerifyExpr::var(&typed_edge.0);
                    let to = VerifyExpr::var(&typed_edge.1);
                    // Pipeline stability: stage output implies next stage input
                    invariants.push(CandidateInvariant {
                        expr: VerifyExpr::implies(from, to),
                        source: InvariantSource::PipelineStability,
                        verified: None,
                    });
                }
            }
        }
    }

    // Pattern 4: Reset entity -> initialization invariant
    for (_, entity) in &kg.entities {
        if let HwEntityType::Reset { .. } = entity {
            // Find typed edges with Resets relation
            for typed_edge in &kg.typed_edges {
                if matches!(typed_edge.2, HwRelation::Resets) {
                    let rst = VerifyExpr::var(&typed_edge.0);
                    let state = VerifyExpr::var(&typed_edge.1);
                    // Reset invariant: reset implies state is initialized (negated)
                    invariants.push(CandidateInvariant {
                        expr: VerifyExpr::implies(rst, VerifyExpr::not(state)),
                        source: InvariantSource::ResetInit,
                        verified: None,
                    });
                }
            }
        }
    }

    invariants
}

/// Verify a candidate invariant using Z3.
///
/// Checks whether the invariant's expression is satisfiable (not trivially false
/// or contradictory). Updates the invariant's `verified` field.
///
/// Returns true if the invariant is satisfiable (verified), false if unsatisfiable
/// or unknown.
pub fn verify_invariant(inv: &mut CandidateInvariant, bound: u32) -> bool {
    use logicaffeine_verify::consistency::{check_consistency, ConsistencyResult};

    let result = check_consistency(&[inv.expr.clone()], &[], bound as usize);
    let verified = matches!(result, ConsistencyResult::Consistent);
    inv.verified = Some(verified);
    verified
}
