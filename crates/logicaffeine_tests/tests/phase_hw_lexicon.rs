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
// SORT CLASSIFICATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn signal_sort_variant_exists() {
    // Sort::Signal must exist as an enum variant
    let _sort = logicaffeine_lexicon::types::Sort::Signal;
}
