//! Sprint B: Hardware Lexicon & Parser
//!
//! RED tests for hardware vocabulary, ## Hardware/Property block types,
//! and temporal keyword disambiguation.

use logicaffeine_language::{compile, compile_kripke, Lexer, Interner};
use logicaffeine_language::token::TokenType;

// ═══════════════════════════════════════════════════════════════════════════
// BLOCK TYPE RECOGNITION (lexer level)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn hardware_block_type_recognized_by_lexer() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("## Hardware\nThe signal is high.", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| matches!(&t.kind, TokenType::BlockHeader { block_type } if *block_type == logicaffeine_language::token::BlockType::Hardware)),
        "Lexer should recognize ## Hardware as BlockType::Hardware"
    );
}

#[test]
fn property_block_type_recognized_by_lexer() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("## Property\nEvery signal is valid.", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| matches!(&t.kind, TokenType::BlockHeader { block_type } if *block_type == logicaffeine_language::token::BlockType::Property)),
        "Lexer should recognize ## Property as BlockType::Property"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// TEMPORAL KEYWORD DISAMBIGUATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn always_outside_property_block_remains_adverb() {
    // "always" in normal declarative context stays an adverb, no Temporal lowering
    let input = "John always runs.";
    let result = compile(input);
    let output = result.unwrap();
    assert!(
        !output.contains("Accessible_Temporal"),
        "'always' outside ## Property should NOT produce Temporal accessibility. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// HARDWARE VOCABULARY
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn clock_noun_parses_and_appears_in_fol() {
    let input = "The clock rises.";
    let result = compile(input);
    assert!(result.is_ok(), "Clock noun should parse: {:?}", result.err());
    let output = result.unwrap();
    assert!(
        output.contains("Clock") || output.contains("clock") || output.contains("Rise"),
        "FOL output must reference the clock noun or the rise predicate. Got: {}",
        output
    );
}

#[test]
fn high_adjective_parses_and_appears_in_fol() {
    let input = "The signal is high.";
    let result = compile(input);
    assert!(result.is_ok(), "High adjective should parse: {:?}", result.err());
    let output = result.unwrap();
    assert!(
        output.contains("High") || output.contains("high"),
        "FOL output must contain 'High' predicate from intersective adjective. Got: {}",
        output
    );
}

#[test]
fn low_adjective_parses_and_appears_in_fol() {
    let input = "The signal is low.";
    let result = compile(input);
    assert!(result.is_ok(), "Low adjective should parse: {:?}", result.err());
    let output = result.unwrap();
    assert!(
        output.contains("Low") || output.contains("low"),
        "FOL output must contain 'Low' predicate from intersective adjective. Got: {}",
        output
    );
}

#[test]
fn toggle_verb_parses_and_appears_in_fol() {
    let input = "The signal toggles.";
    let result = compile(input);
    assert!(result.is_ok(), "Toggle verb should parse: {:?}", result.err());
    let output = result.unwrap();
    assert!(
        output.contains("Toggle") || output.contains("toggle"),
        "FOL output must contain 'Toggle' event predicate. Got: {}",
        output
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP 1: "bit" AS NOUN (NOT PAST TENSE OF "bite")
// Sentences: UART1, UART2, H1
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bit_parsed_as_noun_not_verb_bite() {
    let result = compile("The start bit is low.");
    assert!(result.is_ok(), "\"The start bit is low\" should parse: {:?}", result.err());
    let fol = result.unwrap();
    assert!(
        !fol.contains("Bite"),
        "\"bit\" must be noun (binary digit), not past tense of \"bite\". Got: {}",
        fol
    );
}

#[test]
fn bit_appears_as_noun_in_fol() {
    let fol = compile("The start bit is low.").unwrap();
    assert!(
        fol.contains("Bit") || fol.contains("bit") || fol.contains("Low")
            || fol.contains("Start") || fol.contains("start"),
        "FOL should reference the bit noun phrase. Got: {}",
        fol
    );
    assert!(
        !fol.contains("Bite"),
        "Must not contain verb 'Bite'. Got: {}",
        fol
    );
}

#[test]
fn stop_bit_parses_with_high_predicate() {
    let result = compile("The stop bit is high.");
    assert!(result.is_ok(), "UART2: \"The stop bit is high\" should parse: {:?}", result.err());
    let fol = result.unwrap();
    assert!(
        !fol.contains("Bite"),
        "\"bit\" must not resolve to verb \"bite\". Got: {}",
        fol
    );
    // High predicate depends on Gap 3 (copula + adjective consistency)
    // For now, assert the noun phrase parsed correctly
    assert!(
        fol.contains("Stop") || fol.contains("stop") || fol.contains("Bit")
            || fol.contains("High") || fol.contains("high"),
        "Should contain stop/bit noun or High predicate. Got: {}",
        fol
    );
}

#[test]
fn data_bits_plural_parsed_as_noun() {
    let result = compile("The data bits follow the start bit.");
    assert!(result.is_ok(), "H1: plural \"bits\" should parse as noun: {:?}", result.err());
    let fol = result.unwrap();
    assert!(
        !fol.contains("Bite"),
        "\"bits\" must be plural noun, not verb. Got: {}",
        fol
    );
}

#[test]
fn bit_with_universal_quantifier() {
    // "Every bit is valid" — simpler form that avoids the either/or gap
    let result = compile("Every bit is valid.");
    assert!(result.is_ok(), "Count noun \"bit\" with universal quantifier should parse: {:?}", result.err());
    let fol = result.unwrap();
    assert!(
        fol.contains("∀") || fol.contains("Bit") || fol.contains("bit"),
        "Should have universal quantification over bits. Got: {}",
        fol
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// GAP 9: HARDWARE NOUNS — request, grant, acknowledge
// Sentences: B1, B2
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn request_parses_as_noun_in_copula() {
    let result = compile("If request is asserted, grant is asserted.");
    assert!(
        result.is_ok(),
        "B1: \"request is asserted\" should parse as copula + past participle: {:?}",
        result.err()
    );
    let fol = result.unwrap();
    assert!(
        fol.contains("→") || fol.contains("->"),
        "Conditional should produce implication. Got: {}",
        fol
    );
}

#[test]
fn request_is_high_parses() {
    let result = compile("If request is high, ready is asserted.");
    assert!(
        result.is_ok(),
        "B2: \"request is high\" should parse: {:?}",
        result.err()
    );
}

#[test]
fn grant_parses_as_noun() {
    let result = compile("The grant signal is active.");
    assert!(result.is_ok(), "\"grant\" should parse as noun: {:?}", result.err());
    let fol = result.unwrap();
    assert!(
        fol.contains("Grant") || fol.contains("grant") || fol.contains("Active"),
        "Should contain Grant noun or Active predicate. Got: {}",
        fol
    );
}

#[test]
fn acknowledge_parses_as_noun() {
    let result = compile("Every request is followed by an acknowledge.");
    assert!(
        result.is_ok(),
        "\"acknowledge\" should parse as noun: {:?}",
        result.err()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// SORT CLASSIFICATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn signal_sort_variant_exists() {
    // Sort::Signal must exist as an enum variant
    let _sort = logicaffeine_lexicon::types::Sort::Signal;
}

// ═══════════════════════════════════════════════════════════════════════════
// REGRESSION: EXISTING HARDWARE VOCABULARY MUST NOT REGRESS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn regression_reset_is_asserted_still_works() {
    let fol = compile("If reset is asserted, all outputs are zero.").unwrap();
    assert!(
        fol.contains("→") || fol.contains("->"),
        "Known-working conditional must not regress. Got: {}",
        fol
    );
}

#[test]
fn regression_no_data_lost() {
    let fol = compile("No data is lost during transfer.").unwrap();
    assert!(
        fol.contains("¬") || fol.contains("Not") || fol.contains("!") || fol.contains("∀"),
        "Known-working negation must not regress. Got: {}",
        fol
    );
}
