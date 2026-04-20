//! Specification Self-Consistency Checking
//!
//! Orchestrates the English → FOL → VerifyExpr → Z3 consistency pipeline.
//! Detects contradictions, vacuity, and redundancy in English hardware specs.

use logicaffeine_verify::consistency::{
    check_spec_consistency, ConsistencyConfig, ConsistencyReport, LabeledFormula,
};
use super::fol_to_verify::FolTranslator;
use super::hw_pipeline::HwError;
use super::sva_to_verify::bounded_to_verify;

/// Check consistency of an English hardware specification.
///
/// Parses the spec through [`logicaffeine_language::hw_spec::parse_hw_spec_with`]
/// so headerless inputs and `## Hardware` preambles both route through the
/// typed HW pipeline. Each property sentence becomes its own `LabeledFormula`
/// so MUS extraction, vacuity, and redundancy can attribute findings per
/// sentence.
pub fn check_english_spec(
    spec: &str,
    config: ConsistencyConfig,
) -> Result<ConsistencyReport, HwError> {
    if spec.trim().is_empty() {
        return Ok(check_spec_consistency(&[], &config));
    }

    logicaffeine_language::hw_spec::parse_hw_spec_with(spec, |hw_spec, interner| {
        if hw_spec.properties.is_empty() {
            return check_spec_consistency(&[], &config);
        }

        let mut labeled_formulas = Vec::with_capacity(hw_spec.properties.len());
        for (i, prop) in hw_spec.properties.iter().enumerate() {
            let mut translator = FolTranslator::new(interner, config.temporal_bound);
            translator.set_collapse_truth_predicates(true);
            let result = translator.translate_property(prop);
            let verify_expr = bounded_to_verify(&result.expr);
            labeled_formulas.push(LabeledFormula {
                index: i,
                label: format!("property[{}]", i),
                expr: verify_expr,
            });
        }
        check_spec_consistency(&labeled_formulas, &config)
    })
    .map_err(|e| HwError::ParseError(format!("{:?}", e)))
}
