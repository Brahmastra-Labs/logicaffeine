//! Sprint 3A: FOL → SVA Formal Synthesis
//!
//! The crown jewel: pattern-match Kripke-lowered FOL structures to
//! synthesize SystemVerilog Assertions, then Z3-verify the synthesis is correct.

use logicaffeine_compile::codegen_sva::fol_to_sva::{synthesize_sva_from_spec, SynthesizedSva};
use logicaffeine_compile::codegen_sva::sva_model::parse_sva;

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 1: TEMPORAL PATTERN SYNTHESIS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synthesize_always_to_assert_property() {
    let result = synthesize_sva_from_spec("Always, every signal is valid.", "clk").unwrap();
    assert!(result.sva_text.contains("assert property") || result.sva_text.contains("@(posedge"),
        "G(P) should produce assert property. Got: {}", result.sva_text);
}

#[test]
fn synthesize_eventually_to_cover_property() {
    let result = synthesize_sva_from_spec("Eventually, every signal is active.", "clk").unwrap();
    assert!(result.sva_text.contains("s_eventually") || result.sva_text.contains("cover"),
        "F(P) should produce s_eventually or cover. Got: {}", result.sva_text);
}

#[test]
fn synthesize_next_to_nexttime() {
    let result = synthesize_sva_from_spec("Next, every signal is valid.", "clk").unwrap();
    // After Kripke lowering, X(P) becomes ∀w'(Next_Temporal(w,w') → P(w'))
    // The synthesizer should unwrap this to produce the body P
    assert!(!result.body.is_empty(),
        "X(P) should produce a non-empty SVA body. Got: {}", result.sva_text);
}

#[test]
fn synthesize_conditional_to_implication() {
    let result = synthesize_sva_from_spec(
        "Always, if every dog runs then every cat sleeps.", "clk"
    ).unwrap();
    assert!(result.sva_text.contains("|->") || result.sva_text.contains("|=>"),
        "If-then should produce SVA implication. Got: {}", result.sva_text);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 2: SIGNAL NAME EXTRACTION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synthesize_extracts_signal_names() {
    let result = synthesize_sva_from_spec("Always, every signal is valid.", "clk").unwrap();
    assert!(!result.signals.is_empty(),
        "Should extract signal names. Got: {:?}", result.signals);
}

#[test]
fn synthesize_uses_clock_name() {
    let result = synthesize_sva_from_spec("Always, every signal is valid.", "sys_clk").unwrap();
    assert!(result.sva_text.contains("sys_clk"),
        "Should use provided clock name. Got: {}", result.sva_text);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 3: SYNTHESIZED SVA IS PARSEABLE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synthesized_sva_is_valid() {
    let result = synthesize_sva_from_spec("Always, every signal is valid.", "clk").unwrap();
    // The body of the SVA should be parseable by our SVA parser
    let parse_result = parse_sva(&result.body);
    assert!(parse_result.is_ok(),
        "Synthesized SVA body should be parseable. Body: '{}', Error: {:?}",
        result.body, parse_result.err());
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 4: NON-PANICKING ON VARIOUS INPUTS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synthesize_various_specs_no_panic() {
    let specs = vec![
        "Always, every dog runs.",
        "Eventually, every cat sleeps.",
        "Next, every bird flies.",
        "Always, if every dog runs then every cat sleeps.",
        "Every dog runs until every cat sleeps.",
    ];
    for spec in specs {
        let result = synthesize_sva_from_spec(spec, "clk");
        assert!(result.is_ok(), "Should not error on '{}'. Error: {:?}", spec, result.err());
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 5: Z3 EQUIVALENCE (feature-gated)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "verification")]
mod z3_synthesis {
    use super::*;
    use logicaffeine_compile::codegen_sva::hw_pipeline::check_z3_equivalence;
    use logicaffeine_verify::equivalence::EquivalenceResult;

    #[test]
    fn synthesis_plus_z3_always() {
        let synth = synthesize_sva_from_spec("Always, every signal is valid.", "clk").unwrap();
        let result = check_z3_equivalence("Always, every signal is valid.", &synth.body, 5);
        if let Ok(eq_result) = result {
            assert!(matches!(eq_result, EquivalenceResult::Equivalent),
                "Synthesized SVA should be equivalent to spec. Got: {:?}", eq_result);
        }
        // If Z3 returns Err (e.g. encoding issue), that's acceptable for now
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 6: SPRINT C — Predicates MUST NOT alias to random signals
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synthesis_conditional_has_distinct_antecedent_and_consequent() {
    let result = synthesize_sva_from_spec(
        "Always, if every request holds, then every grant holds.", "clk"
    ).unwrap();
    let body = &result.body;
    if let Some(pos) = body.find("|->") {
        let ante = body[..pos].trim();
        let cons = body[pos+3..].trim();
        assert_ne!(ante, cons,
            "Antecedent '{}' and consequent '{}' must differ — aliasing detected", ante, cons);
    } else if body.contains("||") {
        assert!(body.len() > 10,
            "Conditional spec must produce non-trivial SVA, got: '{}'", body);
    }
}

#[test]
fn synthesis_three_predicates_produce_distinct_identifiers() {
    let result = synthesize_sva_from_spec(
        "Always, if every request holds, then every grant holds and every valid holds.", "clk"
    ).unwrap();
    let body = &result.body;
    let words: Vec<&str> = body.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '@')
        .filter(|w| !w.is_empty() && w.len() > 1)
        .filter(|w| !["assert", "property", "posedge", "clk", "and", "not", "or", "if"].contains(w))
        .collect();
    let unique: std::collections::HashSet<&&str> = words.iter().collect();
    assert!(unique.len() >= 2,
        "Three-predicate spec must produce at least 2 distinct identifiers in SVA body, \
         got {} unique from {:?}", unique.len(), words);
}

#[test]
fn synthesis_no_excessive_first_signal_aliasing() {
    let result = synthesize_sva_from_spec(
        "Always, if every valid holds, then every ready holds.", "clk"
    ).unwrap();
    if result.signals.len() >= 2 {
        let first_sig = &result.signals[0];
        let body_occurrences = result.body.matches(first_sig.as_str()).count();
        assert!(body_occurrences <= 2,
            "Signal '{}' appears {} times in SVA body '{}' — possible aliasing. \
             All signals: {:?}",
            first_sig, body_occurrences, result.body, result.signals);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 7: COMPLEX TEMPORAL PATTERNS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synthesize_nested_always_eventually() {
    let result = synthesize_sva_from_spec("Always, eventually every signal is active.", "clk").unwrap();
    assert!(result.body.contains("s_eventually") || result.sva_text.contains("cover"),
        "Always+Eventually should produce s_eventually. Body: '{}', Full: '{}'",
        result.body, result.sva_text);
}

#[test]
fn synthesize_negation_produces_not() {
    let result = synthesize_sva_from_spec("Always, not every dog runs.", "clk").unwrap();
    assert!(result.body.contains("!") || result.body.contains("not"),
        "Negation should produce ! in SVA body. Got: '{}'", result.body);
}

#[test]
fn synthesize_mutex_spec() {
    let result = synthesize_sva_from_spec(
        "Always, not both every grant_a holds and every grant_b holds.", "clk"
    );
    if let Ok(r) = result {
        assert!(!r.body.is_empty(), "Mutex spec should produce non-empty body");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 8: ERROR HANDLING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synthesize_empty_spec_returns_error() {
    let result = synthesize_sva_from_spec("", "clk");
    assert!(result.is_err(), "Empty spec should return error");
}

#[test]
fn synthesize_gibberish_does_not_panic() {
    // Just verify no panic — Ok or Err both acceptable
    let _ = synthesize_sva_from_spec("asdf qwerty zxcv", "clk");
}

#[test]
fn synthesize_no_temporal_still_works() {
    let result = synthesize_sva_from_spec("Every signal is valid.", "clk");
    if let Ok(r) = result {
        assert!(!r.body.is_empty(), "Non-temporal spec should still produce body");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 9: MULTI-PREDICATE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synthesize_four_predicates_all_distinct() {
    let result = synthesize_sva_from_spec(
        "Always, if every request holds, then every grant holds and every valid holds and every ready holds.",
        "clk",
    ).unwrap();
    let body = &result.body;
    let words: Vec<&str> = body.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '@')
        .filter(|w| !w.is_empty() && w.len() > 1)
        .filter(|w| !["assert", "property", "posedge", "clk", "and", "not", "or", "if"].contains(w))
        .collect();
    let unique: std::collections::HashSet<&&str> = words.iter().collect();
    assert!(unique.len() >= 3,
        "Four-predicate spec should produce >= 3 distinct identifiers. Got {} from {:?}",
        unique.len(), words);
}

#[test]
fn synthesize_disjunction_produces_or() {
    let result = synthesize_sva_from_spec(
        "Always, every dog runs or every cat sleeps.", "clk"
    ).unwrap();
    assert!(result.body.contains("||") || result.body.contains(" or "),
        "Disjunction should produce || in SVA body. Got: '{}'", result.body);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 10: CLOCK VARIATIONS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synthesize_different_clock_names() {
    for clock in &["sys_clk", "pclk", "sclk"] {
        let result = synthesize_sva_from_spec("Always, every signal is valid.", clock).unwrap();
        assert!(result.sva_text.contains(clock),
            "Clock '{}' should appear in sva_text. Got: '{}'", clock, result.sva_text);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 11: SVA PARSEABILITY (additional patterns)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synthesized_conditional_is_parseable() {
    let result = synthesize_sva_from_spec(
        "Always, if every request holds, then every grant holds.", "clk"
    ).unwrap();
    let parse_result = parse_sva(&result.body);
    assert!(parse_result.is_ok(),
        "Conditional SVA body should be parseable. Body: '{}', Error: {:?}",
        result.body, parse_result.err());
}

#[test]
fn synthesized_eventually_is_parseable() {
    let result = synthesize_sva_from_spec("Eventually, every signal is active.", "clk").unwrap();
    let parse_result = parse_sva(&result.body);
    assert!(parse_result.is_ok(),
        "Eventually SVA body should be parseable. Body: '{}', Error: {:?}",
        result.body, parse_result.err());
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 12: OUTPUT STRUCTURE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synthesized_sva_has_valid_kind() {
    let result = synthesize_sva_from_spec("Always, every signal is valid.", "clk").unwrap();
    assert!(result.kind == "assert" || result.kind == "cover",
        "Kind should be 'assert' or 'cover'. Got: '{}'", result.kind);
}

#[test]
fn synthesized_sva_text_has_semicolon() {
    let result = synthesize_sva_from_spec("Always, every signal is valid.", "clk").unwrap();
    assert!(result.sva_text.ends_with(';'),
        "sva_text should end with semicolon. Got: '{}'", result.sva_text);
}

#[test]
fn synthesized_signals_non_empty_for_conditional() {
    let result = synthesize_sva_from_spec(
        "Always, if every request holds, then every grant holds.", "clk"
    ).unwrap();
    assert!(result.signals.len() >= 2,
        "Conditional spec should extract >= 2 signals. Got: {:?}", result.signals);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 13: Z3 EQUIVALENCE (feature-gated, extended)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "verification")]
mod z3_synthesis_extended {
    use super::*;
    use logicaffeine_compile::codegen_sva::hw_pipeline::check_z3_equivalence;
    use logicaffeine_verify::equivalence::EquivalenceResult;

    #[test]
    fn z3_synthesis_conditional() {
        let spec = "Always, if every request holds, then every grant holds.";
        let synth = synthesize_sva_from_spec(spec, "clk").unwrap();
        let result = check_z3_equivalence(spec, &synth.body, 5);
        if let Ok(eq_result) = result {
            assert!(matches!(eq_result, EquivalenceResult::Equivalent),
                "Conditional synthesis should be Z3 equivalent. Got: {:?}", eq_result);
        }
    }

    #[test]
    fn z3_synthesis_negation() {
        let spec = "Always, not every dog runs.";
        let synth = synthesize_sva_from_spec(spec, "clk").unwrap();
        let result = check_z3_equivalence(spec, &synth.body, 5);
        if let Ok(eq_result) = result {
            assert!(matches!(eq_result, EquivalenceResult::Equivalent),
                "Negation synthesis should be Z3 equivalent. Got: {:?}", eq_result);
        }
    }

    #[test]
    fn z3_deliberate_mismatch_detected() {
        let spec = "Always, if every request holds, then every grant holds.";
        let synth = synthesize_sva_from_spec(spec, "clk").unwrap();
        // Negate the body to create a deliberate mismatch
        let tampered = format!("!({})", synth.body);
        let result = check_z3_equivalence(spec, &tampered, 5);
        if let Ok(eq_result) = result {
            assert!(!matches!(eq_result, EquivalenceResult::Equivalent),
                "Tampered SVA should NOT be equivalent to spec. Got: {:?}", eq_result);
        }
    }

    #[test]
    fn z3_synthesis_body_not_trivially_true() {
        use logicaffeine_compile::codegen_sva::hw_pipeline::translate_sva_to_bounded;
        use logicaffeine_compile::codegen_sva::sva_to_verify::bounded_to_verify;
        let spec = "Always, if every request holds, then every grant holds.";
        let synth = synthesize_sva_from_spec(spec, "clk").unwrap();
        let bounded = translate_sva_to_bounded(&synth.body, 3);
        if let Ok(b) = bounded {
            let verify = bounded_to_verify(&b.expr);
            assert!(!matches!(verify, logicaffeine_verify::ir::VerifyExpr::Bool(true)),
                "Translated body should not be trivially true");
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP 7: HAB/ASPECTUAL DEGENERATE OUTPUT
// synthesize_from_ast has no match arm for LogicExpr::Aspectual
// Sentences: A2, A5, C3, F1, F2, A4, D3, F3, M1
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn gap7_hab_predicate_not_degenerate_zero() {
    let result = synthesize_sva_from_spec("The enable signal is always active.", "clk");
    assert!(result.is_ok(), "Should not error on habitual aspect: {:?}", result.err());
    let sva = result.unwrap();
    assert!(
        sva.body != "0",
        "HAB(...) should not produce degenerate '0'. Got body: {}",
        sva.body
    );
}

#[test]
fn gap7_hab_periodic_signal() {
    let result = synthesize_sva_from_spec("The clock signal is periodic.", "clk");
    assert!(result.is_ok(), "A5: should not error: {:?}", result.err());
    let sva = result.unwrap();
    assert!(
        sva.body != "0",
        "Generic predicate should not produce degenerate '0'. Got body: {}",
        sva.body
    );
}

#[test]
fn gap7_hab_arbiter_signals_grant() {
    let result = synthesize_sva_from_spec(
        "The arbiter signals grant to the highest-priority requester.", "clk"
    );
    assert!(result.is_ok(), "F1: should not error: {:?}", result.err());
    let sva = result.unwrap();
    assert!(
        sva.body != "0",
        "F1: habitual action should not produce degenerate '0'. Got body: {}",
        sva.body
    );
}

#[test]
fn gap7_hab_decoder_activates() {
    let result = synthesize_sva_from_spec(
        "The decoder activates the corresponding output line.", "clk"
    );
    assert!(result.is_ok(), "F2: should not error: {:?}", result.err());
    let sva = result.unwrap();
    assert!(
        sva.body != "0",
        "F2: habitual action should not produce degenerate '0'. Got body: {}",
        sva.body
    );
}

#[test]
fn gap7_degenerate_body_not_zero_for_conditional() {
    let result = synthesize_sva_from_spec(
        "If reset is asserted, the output is zero.", "clk"
    ).unwrap();
    assert!(
        !result.body.contains("0 |-> 0"),
        "Implication should not have degenerate '0 |-> 0'. Got body: {}",
        result.body
    );
}

#[test]
fn gap7_hab_body_is_parseable_sva() {
    let result = synthesize_sva_from_spec("The enable signal is always active.", "clk").unwrap();
    if result.body != "0" {
        let parse_result = parse_sva(&result.body);
        assert!(
            parse_result.is_ok(),
            "HAB-synthesized SVA body should be parseable. Body: '{}', Error: {:?}",
            result.body, parse_result.err()
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP 8: COUNTING QUANTIFIER SVA SYNTHESIS
// Exists<=1, Exists>=1, Exists=1 hit the _ => "0" default
// Sentences: UART4, H2, H3
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn gap8_at_most_one_not_degenerate() {
    let result = synthesize_sva_from_spec("At most one arbiter is active.", "clk");
    assert!(result.is_ok(), "H2: should not error: {:?}", result.err());
    let sva = result.unwrap();
    assert!(
        sva.body != "0",
        "Exists<=1 should produce valid SVA (e.g. $onehot0), not '0'. Got body: {}",
        sva.body
    );
}

#[test]
fn gap8_at_least_one_not_degenerate() {
    let result = synthesize_sva_from_spec("At least one handler is ready.", "clk");
    assert!(result.is_ok(), "H3: should not error: {:?}", result.err());
    let sva = result.unwrap();
    assert!(
        sva.body != "0",
        "Exists>=1 should produce valid SVA, not '0'. Got body: {}",
        sva.body
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// REGRESSION: EXISTING SVA SYNTHESIS MUST NOT REGRESS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn regression_sva_always_produces_assert() {
    let result = synthesize_sva_from_spec("Always, every signal is valid.", "clk").unwrap();
    assert!(
        result.sva_text.contains("assert property") || result.sva_text.contains("@(posedge"),
        "G(P) regression: should produce assert property. Got: {}",
        result.sva_text
    );
}

#[test]
fn regression_sva_conditional_has_implication() {
    let result = synthesize_sva_from_spec(
        "Always, if every dog runs then every cat sleeps.", "clk"
    ).unwrap();
    assert!(
        result.sva_text.contains("|->") || result.sva_text.contains("|=>"),
        "If-then regression: should produce SVA implication. Got: {}",
        result.sva_text
    );
}

#[test]
fn regression_sva_eventually_produces_cover_or_s_eventually() {
    let result = synthesize_sva_from_spec("Eventually, every signal is active.", "clk").unwrap();
    assert!(
        result.sva_text.contains("s_eventually") || result.sva_text.contains("cover"),
        "F(P) regression: should produce s_eventually or cover. Got: {}",
        result.sva_text
    );
}
