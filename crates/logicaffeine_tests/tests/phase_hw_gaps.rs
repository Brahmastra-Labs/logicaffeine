//! Hardware Spec Gaps — RED Tests
//!
//! Comprehensive RED tests for all remaining gaps (A–H) identified in the
//! hw-parser-gaps benchmark re-baseline. Each gap section contains tests
//! covering parse acceptance, FOL structure, and SVA synthesis semantics.
//!
//! These tests define the spec. Implementation must make them GREEN without
//! modifying any test in this file.

use logicaffeine_compile::codegen_sva::fol_to_sva::{synthesize_sva_from_spec, SynthesizedSva};
use logicaffeine_compile::codegen_sva::sva_model::parse_sva;
use logicaffeine_language::compile;

// ═══════════════════════════════════════════════════════════════════════════
// HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Synthesize, assert parse success + non-degenerate + parseable SVA body.
fn synth(spec: &str) -> SynthesizedSva {
    let r = synthesize_sva_from_spec(spec, "clk")
        .unwrap_or_else(|e| panic!("Synthesis failed for '{}': {}", spec, e));
    assert!(
        r.body.trim() != "0" && r.body.trim() != "1",
        "Degenerate body for '{}': '{}'",
        spec,
        r.body
    );
    r
}

/// Assert that SVA body is parseable by the SVA model parser.
fn assert_parseable_sva(r: &SynthesizedSva, spec: &str) {
    let parse = parse_sva(&r.body);
    assert!(
        parse.is_ok(),
        "Unparseable SVA for '{}': body='{}' err={:?}",
        spec,
        r.body,
        parse.err()
    );
}

/// Assert that signals were extracted (not empty).
fn assert_signals_extracted(r: &SynthesizedSva, spec: &str) {
    assert!(
        !r.signals.is_empty(),
        "No signals extracted for '{}': body='{}'",
        spec,
        r.body
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP A: CONSEQUENT EVENTUALLY
//
// "If request is asserted, grant is eventually asserted."
//
// The word "eventually" must work inside consequent clauses (after if),
// not just sentence-initially. The SVA must produce a liveness implication:
//   request |-> s_eventually(grant)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn gap_a_consequent_eventually_parses() {
    let result = compile("If request is asserted, grant is eventually asserted.");
    assert!(
        result.is_ok(),
        "\"eventually\" in consequent must parse: {:?}",
        result.err()
    );
}

#[test]
fn gap_a_consequent_eventually_synthesizes() {
    let r = synth("If request is asserted, grant is eventually asserted.");
    assert_parseable_sva(&r, "consequent eventually");
}

#[test]
fn gap_a_consequent_eventually_produces_liveness() {
    let r = synth("If request is asserted, grant is eventually asserted.");
    assert!(
        r.body.contains("s_eventually"),
        "Consequent eventually must produce s_eventually in SVA body. Got: '{}'",
        r.body
    );
}

#[test]
fn gap_a_consequent_eventually_produces_implication() {
    let r = synth("If request is asserted, grant is eventually asserted.");
    assert!(
        r.body.contains("|->") || r.body.contains("|=>"),
        "Must produce implication operator. Got: '{}'",
        r.body
    );
}

#[test]
fn gap_a_consequent_eventually_has_both_signals() {
    let r = synth("If request is asserted, grant is eventually asserted.");
    assert_signals_extracted(&r, "consequent eventually");
    let signals_lower: Vec<String> = r.signals.iter().map(|s| s.to_lowercase()).collect();
    let has_request = signals_lower.iter().any(|s| s.contains("request"));
    let has_grant = signals_lower.iter().any(|s| s.contains("grant"));
    assert!(
        has_request && has_grant,
        "Must extract both request and grant signals. Got: {:?}",
        r.signals
    );
}

#[test]
fn gap_a_consequent_eventually_is_cover_or_liveness() {
    let r = synth("If request is asserted, grant is eventually asserted.");
    // Liveness properties can be either assert with s_eventually or cover
    assert!(
        r.body.contains("s_eventually") || r.kind == "cover",
        "Liveness implication should use s_eventually or be cover property. Got kind='{}' body='{}'",
        r.kind,
        r.body
    );
}

// Variation: "always, if X, then eventually Y"
#[test]
fn gap_a_always_if_then_eventually() {
    let r = synth("Always, if every request holds, then eventually, every grant holds.");
    assert!(
        r.body.contains("s_eventually"),
        "Nested always-if-eventually must produce s_eventually. Got: '{}'",
        r.body
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP B: WHILE AS TEMPORAL DURATION
//
// "While valid is asserted and ready is not asserted, data is stable."
// "The transmitter shall not send data while idle."
//
// "while" introduces duration: Y must hold over the entire interval where
// X is true, not just at the moment X becomes true.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn gap_b_while_parses() {
    let result = compile("While valid is asserted and ready is not asserted, data is stable.");
    assert!(
        result.is_ok(),
        "\"While X, Y\" must parse: {:?}",
        result.err()
    );
}

#[test]
fn gap_b_while_synthesizes() {
    let r = synth("While valid is asserted and ready is not asserted, data is stable.");
    assert_parseable_sva(&r, "while stability");
}

#[test]
fn gap_b_while_not_plain_implication() {
    // While semantics must NOT collapse to a one-step implication.
    // The SVA must express duration: the consequent holds throughout the condition.
    let r = synth("While valid is asserted and ready is not asserted, data is stable.");
    // Duration can be expressed as:
    // - X throughout Y
    // - X |-> (Y s_until !X)
    // - (X && Y) style conjunction that holds at every cycle
    // At minimum, the body must reference both the condition and the consequent
    let body_lower = r.body.to_lowercase();
    assert!(
        body_lower.contains("valid") && body_lower.contains("data"),
        "While body must reference both condition (valid) and consequent (data). Got: '{}'",
        r.body
    );
}

#[test]
fn gap_b_while_extracts_signals() {
    let r = synth("While valid is asserted and ready is not asserted, data is stable.");
    assert_signals_extracted(&r, "while stability");
}

#[test]
fn gap_b_while_postposed_parses() {
    // "shall not send data while idle" — while appears after the main clause
    let result = compile("The transmitter shall not send data while idle.");
    assert!(
        result.is_ok(),
        "Postposed \"while\" must parse: {:?}",
        result.err()
    );
}

#[test]
fn gap_b_while_postposed_synthesizes() {
    let r = synth("The transmitter shall not send data while idle.");
    assert_parseable_sva(&r, "postposed while");
}

#[test]
fn gap_b_while_simple() {
    // Simpler while case
    let r = synth("While reset is asserted, the output is low.");
    assert_parseable_sva(&r, "simple while");
    let body_lower = r.body.to_lowercase();
    assert!(
        body_lower.contains("reset") && body_lower.contains("output"),
        "While body must reference condition and consequent. Got: '{}'",
        r.body
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP C: AFTER X, Y WITHIN N CYCLES
//
// "After request, grant follows within 3 cycles."
//
// This tests the combination of after-subordinator with bounded temporal
// follow. Must NOT reduce to plain implication — must preserve the bound.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn gap_c_after_follows_within_parses() {
    let result = compile("After request, grant follows within 3 cycles.");
    assert!(
        result.is_ok(),
        "\"After X, Y follows within N cycles\" must parse: {:?}",
        result.err()
    );
}

#[test]
fn gap_c_after_follows_within_synthesizes() {
    let r = synth("After request, grant follows within 3 cycles.");
    assert_parseable_sva(&r, "after follows within");
}

#[test]
fn gap_c_after_follows_within_has_bound() {
    let r = synth("After request, grant follows within 3 cycles.");
    // The SVA body must contain a bounded delay: ##[0:3] or ##[1:3]
    assert!(
        r.body.contains("##") || r.body.contains("[0:3]") || r.body.contains("[1:3]"),
        "After+within must produce bounded delay in SVA. Got: '{}'",
        r.body
    );
}

#[test]
fn gap_c_after_follows_within_extracts_signals() {
    let r = synth("After request, grant follows within 3 cycles.");
    assert_signals_extracted(&r, "after follows within");
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP D: COUNTING QUANTIFIER WITH EXPLICIT LIST
//
// "At most one of grant0, grant1, and grant2 is asserted."
//
// The parser must handle "of" + comma-separated signal list after
// counting quantifiers, producing $onehot0 over the listed signals.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn gap_d_counting_of_list_parses() {
    let result = compile("At most one of grant0, grant1, and grant2 is asserted.");
    assert!(
        result.is_ok(),
        "Counting quantifier with 'of' + list must parse: {:?}",
        result.err()
    );
}

#[test]
fn gap_d_counting_of_list_synthesizes() {
    let r = synth("At most one of grant0, grant1, and grant2 is asserted.");
    assert_parseable_sva(&r, "counting of list");
}

#[test]
fn gap_d_counting_of_list_produces_onehot() {
    let r = synth("At most one of grant0, grant1, and grant2 is asserted.");
    let body_lower = r.body.to_lowercase();
    assert!(
        body_lower.contains("onehot0") || body_lower.contains("onehot"),
        "At-most-one of list should produce $onehot0. Got: '{}'",
        r.body
    );
}

#[test]
fn gap_d_counting_of_list_contains_all_signals() {
    let r = synth("At most one of grant0, grant1, and grant2 is asserted.");
    let body_lower = r.body.to_lowercase();
    assert!(
        body_lower.contains("grant0") && body_lower.contains("grant1") && body_lower.contains("grant2"),
        "SVA body must reference all listed signals. Got: '{}'",
        r.body
    );
}

#[test]
fn gap_d_counting_of_list_extracts_signals() {
    let r = synth("At most one of grant0, grant1, and grant2 is asserted.");
    assert_signals_extracted(&r, "counting of list");
}

// Two-element list
#[test]
fn gap_d_counting_of_two_elements() {
    let result = compile("At most one of read and write is asserted.");
    assert!(
        result.is_ok(),
        "Counting with two-element list must parse: {:?}",
        result.err()
    );
}

// Exactly-one variant
#[test]
fn gap_d_exactly_one_of_list() {
    let result = compile("Exactly one of grant0, grant1, and grant2 is asserted.");
    assert!(
        result.is_ok(),
        "Exactly-one of list must parse: {:?}",
        result.err()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP E: COUNTING QUANTIFIER + PASSIVE PREDICATE
//
// "At most one request is granted at any time."
//
// Counting quantifier with passive voice predicate must work correctly.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn gap_e_counting_passive_parses() {
    let result = compile("At most one request is granted at any time.");
    assert!(
        result.is_ok(),
        "Counting + passive must parse: {:?}",
        result.err()
    );
}

#[test]
fn gap_e_counting_passive_synthesizes() {
    let r = synth("At most one request is granted at any time.");
    assert_parseable_sva(&r, "counting passive");
}

#[test]
fn gap_e_counting_passive_produces_onehot() {
    let r = synth("At most one request is granted at any time.");
    let body_lower = r.body.to_lowercase();
    assert!(
        body_lower.contains("onehot0") || body_lower.contains("countones"),
        "Counting+passive should produce $onehot0 or $countones. Got: '{}'",
        r.body
    );
}

#[test]
fn gap_e_counting_passive_extracts_signals() {
    let r = synth("At most one request is granted at any time.");
    assert_signals_extracted(&r, "counting passive");
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP F: SIGNAL EXTRACTION FOR COUNTING SVAs
//
// These specs already synthesize valid SVA but extract zero signals.
// The signal extractor must inspect $onehot0(...) / $countones(...) args.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn gap_f_at_most_one_signal_extracts() {
    let r = synth("At most one signal is valid.");
    assert_signals_extracted(&r, "at most one signal");
}

#[test]
fn gap_f_at_most_one_grant_extracts() {
    let r = synth("At most one grant is valid.");
    assert_signals_extracted(&r, "at most one grant");
}

#[test]
fn gap_f_at_most_two_signals_extracts() {
    let r = synth("At most two signals are valid.");
    assert_signals_extracted(&r, "at most two signals");
}

#[test]
fn gap_f_at_least_one_signal_extracts() {
    let r = synth("At least one signal is valid.");
    assert_signals_extracted(&r, "at least one signal");
}

#[test]
fn gap_f_at_least_two_signals_extracts() {
    let r = synth("At least two signals are valid.");
    assert_signals_extracted(&r, "at least two signals");
}

#[test]
fn gap_f_at_most_one_grant_asserted_extracts() {
    let r = synth("At most one grant is asserted at any time.");
    assert_signals_extracted(&r, "at most one grant asserted");
}

#[test]
fn gap_f_at_most_one_request_granted_extracts() {
    // This is also Gap E — but the signal extraction aspect is Gap F
    let r = synth("At most one request is granted at any time.");
    assert_signals_extracted(&r, "at most one request granted");
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP G: NEOEVENT / ACTION SENTENCE TAXONOMY
//
// "The bus acknowledges the request."
//
// This is an action sentence, not a temporal property. It currently
// degenerates to body="0". The system should either:
// 1. Reject with a structured error explaining why
// 2. Reclassify as not_a_property
// 3. Provide a real property interpretation
//
// We test for option 1 or 3: it must not silently degenerate.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn gap_g_neoevent_does_not_degenerate() {
    let result = synthesize_sva_from_spec("The bus acknowledges the request.", "clk");
    match result {
        Ok(r) => {
            assert!(
                r.body.trim() != "0",
                "NeoEvent must not degenerate to '0'. Either synthesize or reject explicitly. Got body: '{}'",
                r.body
            );
        }
        Err(e) => {
            // Explicit rejection is acceptable — but it should indicate
            // this is not a property, not just a generic parse error
            assert!(
                e.to_lowercase().contains("property")
                    || e.to_lowercase().contains("action")
                    || e.to_lowercase().contains("not a")
                    || e.to_lowercase().contains("temporal"),
                "Rejection message should explain why this is not a property. Got: '{}'",
                e
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP H: NEVER UNKNOWN IN CONSEQUENT
//
// "If dma_en and dma_we are asserted, dma_din is never unknown."
// "If smclk_en and cpu_en are asserted, smclk is never unknown."
//
// The word "never" must work in consequent clauses, and "unknown" must
// be recognized as a valid adjective/predicate.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn gap_h_never_unknown_parses() {
    let result = compile("If dma_en and dma_we are asserted, dma_din is never unknown.");
    assert!(
        result.is_ok(),
        "\"never unknown\" in consequent must parse: {:?}",
        result.err()
    );
}

#[test]
fn gap_h_never_unknown_synthesizes() {
    let r = synth("If dma_en and dma_we are asserted, dma_din is never unknown.");
    assert_parseable_sva(&r, "never unknown dma");
}

#[test]
fn gap_h_never_unknown_has_negation() {
    let r = synth("If dma_en and dma_we are asserted, dma_din is never unknown.");
    let body_lower = r.body.to_lowercase();
    assert!(
        body_lower.contains("!") || body_lower.contains("~") || body_lower.contains("not"),
        "\"never unknown\" must produce negation in SVA. Got: '{}'",
        r.body
    );
}

#[test]
fn gap_h_never_unknown_extracts_signals() {
    let r = synth("If dma_en and dma_we are asserted, dma_din is never unknown.");
    assert_signals_extracted(&r, "never unknown dma");
    let signals_lower: Vec<String> = r.signals.iter().map(|s| s.to_lowercase()).collect();
    let has_dma_en = signals_lower.iter().any(|s| s.contains("dma_en"));
    let has_dma_din = signals_lower.iter().any(|s| s.contains("dma_din"));
    assert!(
        has_dma_en && has_dma_din,
        "Must extract dma_en and dma_din signals. Got: {:?}",
        r.signals
    );
}

#[test]
fn gap_h_never_unknown_variant_smclk() {
    let result = compile("If smclk_en and cpu_en are asserted, smclk is never unknown.");
    assert!(
        result.is_ok(),
        "smclk variant must parse: {:?}",
        result.err()
    );
}

#[test]
fn gap_h_never_unknown_variant_smclk_synthesizes() {
    let r = synth("If smclk_en and cpu_en are asserted, smclk is never unknown.");
    assert_parseable_sva(&r, "never unknown smclk");
}

// ═══════════════════════════════════════════════════════════════════════════
// BONUS: NOT-BOTH MUTEX (discovered in baseline as additional parse-fail)
//
// "Always, not both every request is valid and every grant is valid."
//
// This was not in the original gap list but showed up as PARSE-FAIL.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bonus_not_both_parses() {
    let result = compile("Always, not both every request is valid and every grant is valid.");
    assert!(
        result.is_ok(),
        "\"not both X and Y\" must parse: {:?}",
        result.err()
    );
}

#[test]
fn bonus_not_both_synthesizes() {
    let r = synth("Always, not both every request is valid and every grant is valid.");
    assert_parseable_sva(&r, "not both mutex");
}

#[test]
fn bonus_not_both_produces_negated_conjunction() {
    let r = synth("Always, not both every request is valid and every grant is valid.");
    let body_lower = r.body.to_lowercase();
    // "not both A and B" = !(A && B) = !A || !B
    assert!(
        (body_lower.contains("!") && body_lower.contains("&&"))
            || body_lower.contains("||")
            || body_lower.contains("nand"),
        "Not-both should produce negated conjunction or NAND. Got: '{}'",
        r.body
    );
}
