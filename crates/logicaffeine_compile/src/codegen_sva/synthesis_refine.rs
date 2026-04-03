//! CEGAR Refinement for SVA Synthesis
//!
//! When synthesized SVA doesn't match the spec, classify the divergence
//! as too-strong or too-weak, apply transformations, and re-check.

use super::sva_model::SvaExpr;

#[cfg(feature = "verification")]
use super::hw_pipeline::check_z3_equivalence;

/// Classification of synthesis divergence.
#[derive(Debug, Clone, PartialEq)]
pub enum Divergence {
    /// SVA is stricter than the spec — it rejects valid behaviors.
    TooStrong,
    /// SVA is more permissive than the spec — it allows invalid behaviors.
    TooWeak,
    /// Cannot classify.
    Unknown,
}

/// Refinement result.
#[derive(Debug)]
pub struct RefinementResult {
    pub converged: bool,
    pub iterations: u32,
    pub final_sva: String,
    pub divergence: Option<Divergence>,
}

/// Classify whether an SVA is too strong or too weak relative to the spec.
///
/// Too strong: spec allows a behavior that SVA rejects.
/// Too weak: SVA allows a behavior that spec rejects.
pub fn classify_divergence(
    spec_allows_ce: bool,
    sva_allows_ce: bool,
) -> Divergence {
    match (spec_allows_ce, sva_allows_ce) {
        (true, false) => Divergence::TooStrong,
        (false, true) => Divergence::TooWeak,
        _ => Divergence::Unknown,
    }
}

/// Apply a weakening transformation to an SVA body.
/// Converts overlapping implication to non-overlapping (adds delay).
pub fn weaken_implication(sva: &str) -> String {
    sva.replace("|->", "|=>")
}

/// Apply a strengthening transformation to an SVA body.
/// Converts non-overlapping to overlapping (removes delay).
pub fn strengthen_implication(sva: &str) -> String {
    sva.replace("|=>", "|->")
}

/// Apply an eventual-response transformation.
/// Converts immediate response to eventual.
pub fn weaken_to_eventual(sva: &str) -> String {
    if sva.contains("|-> ") && !sva.contains("s_eventually") {
        sva.replace("|-> ", "|-> s_eventually(") + ")"
    } else {
        sva.to_string()
    }
}

/// Run a CEGAR refinement loop: check equivalence, classify divergence,
/// apply transformation, re-check. Bounded to max_iterations.
#[cfg(feature = "verification")]
pub fn refine_sva(spec: &str, initial_sva: &str, clock: &str, bound: u32) -> RefinementResult {
    use logicaffeine_verify::equivalence::EquivalenceResult;

    let max_iterations: u32 = 5;
    let mut current_sva = initial_sva.to_string();
    let transformations: Vec<fn(&str) -> String> = vec![
        weaken_implication,
        strengthen_implication,
        weaken_to_eventual,
    ];

    for i in 0..max_iterations {
        let result = check_z3_equivalence(spec, &current_sva, bound);
        match result {
            Ok(EquivalenceResult::Equivalent) => {
                return RefinementResult {
                    converged: true,
                    iterations: i + 1,
                    final_sva: current_sva,
                    divergence: None,
                };
            }
            Ok(EquivalenceResult::NotEquivalent { .. }) => {
                // Try next transformation
                let transform_idx = i as usize % transformations.len();
                let new_sva = transformations[transform_idx](&current_sva);
                if new_sva == current_sva {
                    // Transformation had no effect — try next one
                    if transform_idx + 1 < transformations.len() {
                        let alt_sva = transformations[transform_idx + 1](&current_sva);
                        if alt_sva != current_sva {
                            current_sva = alt_sva;
                            continue;
                        }
                    }
                    // No transformation helped
                    return RefinementResult {
                        converged: false,
                        iterations: i + 1,
                        final_sva: current_sva,
                        divergence: Some(Divergence::Unknown),
                    };
                }
                current_sva = new_sva;
            }
            Ok(EquivalenceResult::Unknown) | Err(_) => {
                return RefinementResult {
                    converged: false,
                    iterations: i + 1,
                    final_sva: current_sva,
                    divergence: Some(Divergence::Unknown),
                };
            }
        }
    }

    RefinementResult {
        converged: false,
        iterations: max_iterations,
        final_sva: current_sva,
        divergence: Some(Divergence::Unknown),
    }
}
