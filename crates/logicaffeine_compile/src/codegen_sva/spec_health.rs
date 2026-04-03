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
/// Splits the input into individual sentences, compiles each to Kripke FOL,
/// translates each to VerifyExpr, and runs full consistency analysis
/// (satisfiability, MUS extraction, vacuity, redundancy).
pub fn check_english_spec(
    spec: &str,
    config: ConsistencyConfig,
) -> Result<ConsistencyReport, HwError> {
    let sentences = split_sentences(spec);

    if sentences.is_empty() {
        return Ok(check_spec_consistency(&[], &config));
    }

    let mut labeled_formulas = Vec::new();

    for (i, sentence) in sentences.iter().enumerate() {
        let verify_expr = logicaffeine_language::compile_kripke_with(sentence, |ast, interner| {
            let mut translator = FolTranslator::new(interner, config.temporal_bound);
            translator.set_collapse_truth_predicates(true);
            let result = translator.translate_property(ast);
            bounded_to_verify(&result.expr)
        })
        .map_err(|e| HwError::ParseError(format!("{:?}", e)))?;

        labeled_formulas.push(LabeledFormula {
            index: i,
            label: sentence.to_string(),
            expr: verify_expr,
        });
    }

    Ok(check_spec_consistency(&labeled_formulas, &config))
}

/// Split a spec into individual sentences.
///
/// Splits on `. ` followed by an uppercase letter, or `. ` at end of string.
/// Preserves the period in each sentence for LOGOS parsing.
fn split_sentences(spec: &str) -> Vec<&str> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Vec::new();
    }

    let mut sentences = Vec::new();
    let mut start = 0;

    let bytes = spec.as_bytes();
    let len = bytes.len();

    let mut i = 0;
    while i < len {
        if bytes[i] == b'.' {
            // Check if this is a sentence boundary:
            // ". " followed by uppercase, or "." at end of string
            let at_end = i + 1 >= len || (i + 2 >= len && bytes[i + 1] == b' ');
            let followed_by_space_upper = i + 2 < len
                && bytes[i + 1] == b' '
                && bytes[i + 2].is_ascii_uppercase();

            if at_end || followed_by_space_upper {
                let sentence = spec[start..=i].trim();
                if !sentence.is_empty() {
                    sentences.push(sentence);
                }
                // Skip the space after the period
                if i + 1 < len && bytes[i + 1] == b' ' {
                    start = i + 2;
                } else {
                    start = i + 1;
                }
            }
        }
        i += 1;
    }

    // Handle trailing text without period
    if start < len {
        let remainder = spec[start..].trim();
        if !remainder.is_empty() {
            sentences.push(remainder);
        }
    }

    sentences
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_single_sentence() {
        assert_eq!(split_sentences("Req is valid."), vec!["Req is valid."]);
    }

    #[test]
    fn split_two_sentences() {
        assert_eq!(
            split_sentences("Req is valid. Ack is valid."),
            vec!["Req is valid.", "Ack is valid."]
        );
    }

    #[test]
    fn split_three_sentences() {
        assert_eq!(
            split_sentences("Req is valid. Ack is valid. Data is valid."),
            vec!["Req is valid.", "Ack is valid.", "Data is valid."]
        );
    }

    #[test]
    fn split_empty() {
        assert!(split_sentences("").is_empty());
        assert!(split_sentences("   ").is_empty());
    }

    #[test]
    fn split_no_period() {
        assert_eq!(split_sentences("Req is valid"), vec!["Req is valid"]);
    }

    #[test]
    fn split_preserves_internal_periods() {
        // "e.g." should not be split
        assert_eq!(
            split_sentences("If req is valid. Then ack follows."),
            vec!["If req is valid.", "Then ack follows."]
        );
    }
}
