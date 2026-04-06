//! Hardware Parser Gap Tests
//!
//! RED tests for parser-structural gaps identified in the 48-sentence
//! hardware spec gap analysis. Each section targets a specific parser gap:
//!
//! - Gap 2: Modal verb `shall` (IEEE standard deontic)
//! - Gap 3: Copula + temporal adverb (`is always`, `is never`)
//! - Gap 4: `after` as temporal subordinator
//! - Gap 5: `and` between copula clauses in conditionals
//! - Gap 6: `when`/`whenever` as conditional subordinator

use logicaffeine_language::compile;

// ═══════════════════════════════════════════════════════════════════════════
// GAP 2: MODAL VERB `SHALL`
// modal.rs:82-85 check gate omits Shall despite modal vector at 374-378
// Sentences: UART3, I1
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn gap2_shall_parses_as_modal() {
    let result = compile("The transmitter shall send data.");
    assert!(
        result.is_ok(),
        "\"shall\" must be recognized as modal verb: {:?}",
        result.err()
    );
}

#[test]
fn gap2_shall_produces_deontic_operator() {
    let fol = compile("The receiver shall acknowledge every request.").unwrap();
    assert!(
        fol.contains("□") || fol.contains("O_") || fol.contains("◇")
            || fol.contains("Deontic") || fol.contains("Shall"),
        "\"shall\" should produce deontic modal operator. Got: {}",
        fol
    );
}

#[test]
fn gap2_shall_not_produces_negated_modal() {
    let result = compile("The device shall not enter low-power mode during transfer.");
    assert!(
        result.is_ok(),
        "\"shall not\" should parse: {:?}",
        result.err()
    );
    let fol = result.unwrap();
    assert!(
        fol.contains("¬") || fol.contains("Not") || fol.contains("!"),
        "\"shall not\" should produce negation. Got: {}",
        fol
    );
}

#[test]
fn gap2_shall_never_produces_temporal_negation() {
    let result = compile("The transmitter shall never send data without a start bit.");
    assert!(
        result.is_ok(),
        "UART3: \"shall never\" should parse: {:?}",
        result.err()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP 3: COPULA + TEMPORAL ADVERB (`is always`, `is never`)
// mod.rs:~7089 — Not check exists but no Always/Never check
// Sentences: A1, A3, K1, D1
// ═══════════════════════════════════════════════════════════════════════════

// --- Symptom A: "is never" ---

#[test]
fn gap3_is_never_high_parses() {
    let result = compile("The error flag is never high.");
    assert!(
        result.is_ok(),
        "A3: \"is never\" + adjective should parse: {:?}",
        result.err()
    );
}

#[test]
fn gap3_is_never_produces_negation() {
    let fol = compile("The error flag is never high.").unwrap();
    assert!(
        fol.contains("¬") || fol.contains("Not"),
        "\"is never\" should produce negation. Got: {}",
        fol
    );
}

#[test]
fn gap3_is_never_low_parses() {
    let result = compile("The clock signal is never low.");
    assert!(
        result.is_ok(),
        "K1: \"is never low\" should parse: {:?}",
        result.err()
    );
}

#[test]
fn gap3_is_never_idle_parses() {
    let result = compile("The bus is never idle.");
    assert!(
        result.is_ok(),
        "D1: \"is never idle\" should parse: {:?}",
        result.err()
    );
}

#[test]
fn gap3_never_before_verb_still_works() {
    // Existing behavior: "never" before verbs must not regress
    let result = compile("The arbiter never grants two requests simultaneously.");
    assert!(
        result.is_ok(),
        "\"never\" before verb should still parse: {:?}",
        result.err()
    );
}

// --- Symptom B: "is always" ---

#[test]
fn gap3_is_always_high_parses() {
    let result = compile("The output is always high.");
    assert!(
        result.is_ok(),
        "A1: \"is always high\" should parse: {:?}",
        result.err()
    );
}

#[test]
fn gap3_is_always_high_contains_predicate() {
    let fol = compile("The output is always high.").unwrap();
    assert!(
        fol.contains("High") || fol.contains("high") || fol.contains("H("),
        "Should contain High predicate. Got: {}",
        fol
    );
}

#[test]
fn gap3_is_always_low_parses() {
    let result = compile("The output is always low.");
    assert!(
        result.is_ok(),
        "\"is always low\" should parse: {:?}",
        result.err()
    );
    let fol = result.unwrap();
    assert!(
        fol.contains("Low") || fol.contains("low") || fol.contains("L("),
        "Should contain Low predicate. Got: {}",
        fol
    );
}

#[test]
fn gap3_is_always_valid_parses() {
    let result = compile("The data is always valid.");
    assert!(
        result.is_ok(),
        "\"is always valid\" should parse: {:?}",
        result.err()
    );
}

#[test]
fn gap3_is_always_active_still_works() {
    // A2: "The enable signal is always active" reportedly works — regression guard
    let result = compile("The enable signal is always active.");
    assert!(
        result.is_ok(),
        "\"is always active\" should still parse: {:?}",
        result.err()
    );
}

#[test]
fn gap3_is_always_produces_temporal_wrapper() {
    let fol = compile("The output is always high.").unwrap();
    assert!(
        fol.contains("G(") || fol.contains("∀w") || fol.contains("Always")
            || fol.contains("Accessible") || fol.contains("HAB"),
        "\"is always\" should produce temporal invariant G(P). Got: {}",
        fol
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP 4: `AFTER` AS TEMPORAL SUBORDINATOR
// clause.rs:136-248 handles Always/Eventually/Never but not after/before
// Sentences: C4, J1
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn gap4_after_reset_parses() {
    let result = compile("After reset, the state machine returns to idle.");
    assert!(
        result.is_ok(),
        "C4: sentence-initial \"After X, Y\" should parse: {:?}",
        result.err()
    );
}

#[test]
fn gap4_after_produces_temporal_ordering() {
    let fol = compile("After reset, the output is zero.").unwrap();
    // Should produce temporal sequence or implication, not just a bare preposition
    assert!(
        fol.contains("→") || fol.contains("->") || fol.contains("|->")
            || fol.contains("Implies") || fol.contains("After")
            || fol.contains("Response"),
        "\"After X, Y\" should produce temporal ordering. Got: {}",
        fol
    );
}

#[test]
fn gap4_after_initialization_parses() {
    let result = compile("After initialization, the device enters normal mode.");
    assert!(
        result.is_ok(),
        "J1: \"After initialization\" should parse: {:?}",
        result.err()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP 5: `AND` BETWEEN COPULA CLAUSES IN CONDITIONALS
// clause.rs:~495 parse_counterfactual_antecedent — no And after first clause
// Sentences: E2, L2
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn gap5_if_x_is_y_and_z_is_w_parses() {
    let result = compile("If valid is high and ready is high, data is transferred.");
    assert!(
        result.is_ok(),
        "E2: conjunction of copula clauses in conditional should parse: {:?}",
        result.err()
    );
}

#[test]
fn gap5_conjunction_produces_and_in_fol() {
    let fol = compile("If valid is high and ready is high, data is transferred.").unwrap();
    assert!(
        fol.contains("∧") || fol.contains("And") || fol.contains("&&") || fol.contains("&"),
        "Should contain conjunction of two conditions. Got: {}",
        fol
    );
    assert!(
        fol.contains("→") || fol.contains("->"),
        "Should contain conditional implication. Got: {}",
        fol
    );
}

#[test]
fn gap5_enable_and_mode_parses() {
    let result = compile("If enable is active and mode is set, the output follows the input.");
    assert!(
        result.is_ok(),
        "L2: compound conditional antecedent should parse: {:?}",
        result.err()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP 6: `WHEN` / `WHENEVER` AS CONDITIONAL SUBORDINATOR
// clause.rs:227-241 treats When as wh-question, not subordinator
// Sentences: L3, L4
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn gap6_when_as_conditional_parses() {
    let result = compile("When the buffer is full, backpressure is applied.");
    assert!(
        result.is_ok(),
        "L3: \"When X, Y\" should parse: {:?}",
        result.err()
    );
}

#[test]
fn gap6_when_produces_conditional_structure() {
    let fol = compile("When the buffer is full, backpressure is applied.").unwrap();
    assert!(
        fol.contains("→") || fol.contains("->") || fol.contains("Implies") || fol.contains("∀"),
        "\"When X, Y\" should produce conditional, not degenerate literal. Got: {}",
        fol
    );
    assert!(
        fol != "When" && fol.trim() != "When",
        "Must not produce bare \"When\" keyword. Got: {}",
        fol
    );
}

#[test]
fn gap6_whenever_as_universal_conditional() {
    let result = compile("Whenever the clock rises, the register is updated.");
    assert!(
        result.is_ok(),
        "L4: \"Whenever X, Y\" should parse: {:?}",
        result.err()
    );
    let fol = result.unwrap();
    assert!(
        fol.trim() != "When" && fol.trim() != "Whenever" && !fol.trim().is_empty(),
        "\"Whenever\" should produce structured FOL, not a bare keyword. Got: {}",
        fol
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// REGRESSION: KNOWN-WORKING PATTERNS MUST NOT REGRESS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn regression_if_reset_asserted() {
    let fol = compile("If reset is asserted, all outputs are zero.").unwrap();
    assert!(
        fol.contains("→") || fol.contains("->"),
        "Known-working conditional must not regress. Got: {}",
        fol
    );
}

#[test]
fn regression_if_valid_asserted() {
    let fol = compile("If valid is asserted, ready is asserted.").unwrap();
    assert!(
        fol.contains("→") || fol.contains("->"),
        "Known-working conditional must not regress. Got: {}",
        fol
    );
}

#[test]
fn regression_read_write_not_both_active() {
    let fol = compile("Read and write are not both active.").unwrap();
    assert!(
        fol.contains("¬") || fol.contains("Not") || fol.contains("!"),
        "Known-working negation must not regress. Got: {}",
        fol
    );
}

#[test]
fn regression_every_request_followed_by_acknowledge() {
    let fol = compile("Every request is followed by an acknowledge.").unwrap();
    assert!(
        fol.contains("∀") || fol.contains("→"),
        "Known-working universal must not regress. Got: {}",
        fol
    );
}

#[test]
fn regression_receiver_must_acknowledge() {
    let fol = compile("The receiver must acknowledge.").unwrap();
    assert!(
        fol.contains("□") || fol.contains("Must") || fol.contains("O_"),
        "Known-working modal must not regress. Got: {}",
        fol
    );
}

#[test]
fn regression_never_before_verb() {
    let fol = compile("The arbiter never grants two requests simultaneously.").unwrap();
    assert!(
        fol.contains("¬") || fol.contains("Not"),
        "Known-working \"never\" before verb must not regress. Got: {}",
        fol
    );
}
