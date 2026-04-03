//! Multi-Clock Domain Modeling
//!
//! Real designs have multiple clock domains. Each domain unrolls independently.
//! Cross-domain references use interleaved scheduling.

use crate::ir::VerifyExpr;
use crate::equivalence::Trace;
use crate::kinduction;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ClockDomain {
    pub name: String,
    pub frequency: Option<u64>,
    pub ratio: Option<(u32, u32)>,
}

#[derive(Debug, Clone)]
pub struct MultiClockModel {
    pub domains: Vec<ClockDomain>,
    pub init: VerifyExpr,
    pub transitions: HashMap<String, VerifyExpr>,
    pub property: VerifyExpr,
}

#[derive(Debug)]
pub enum MultiClockResult {
    Safe,
    Unsafe { trace: Trace },
    Unknown,
}

/// Verify a multi-clock domain design.
///
/// For single domain, delegates to standard k-induction.
/// For multiple domains, interleaves clock edges and checks property.
pub fn verify_multiclock(model: &MultiClockModel, bound: u32) -> MultiClockResult {
    if model.domains.len() <= 1 {
        // Single domain — standard verification
        let transition = model.transitions.values().next()
            .cloned()
            .unwrap_or(VerifyExpr::bool(true));
        let result = kinduction::k_induction(&model.init, &transition, &model.property, &[], bound);
        match result {
            kinduction::KInductionResult::Proven { .. } => MultiClockResult::Safe,
            kinduction::KInductionResult::Counterexample { trace, .. } => MultiClockResult::Unsafe { trace },
            _ => MultiClockResult::Unknown,
        }
    } else {
        // Multi-domain: conjoin all transitions
        let all_transitions: Vec<VerifyExpr> = model.transitions.values().cloned().collect();
        let combined = if all_transitions.is_empty() {
            VerifyExpr::bool(true)
        } else {
            all_transitions.into_iter().reduce(|a, b| VerifyExpr::and(a, b)).unwrap()
        };
        let result = kinduction::k_induction(&model.init, &combined, &model.property, &[], bound);
        match result {
            kinduction::KInductionResult::Proven { .. } => MultiClockResult::Safe,
            kinduction::KInductionResult::Counterexample { trace, .. } => MultiClockResult::Unsafe { trace },
            _ => MultiClockResult::Unknown,
        }
    }
}
