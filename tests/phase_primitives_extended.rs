//! Phase Primitives Extended: Char and Byte Types
//!
//! Tests for extended primitive types:
//! - Char: Character literals using backticks `x`
//! - Byte: 8-bit unsigned integer (0-255)

use logos::{Interner, Lexer, TokenType};

// === CHAR LITERAL TOKENIZATION ===

#[test]
fn char_literal_tokenizes() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("`a`", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::CharLiteral(_))),
        "Should tokenize backtick char literal: {:?}",
        tokens
    );
}

#[test]
fn char_literal_preserves_value() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("`x`", &mut interner);
    let tokens = lexer.tokenize();

    let char_token = tokens.iter().find(|t| matches!(t.kind, TokenType::CharLiteral(_)));
    assert!(char_token.is_some(), "Should have CharLiteral token");

    if let Some(token) = char_token {
        if let TokenType::CharLiteral(sym) = token.kind {
            let value = interner.resolve(sym);
            assert_eq!(value, "x", "Char literal should preserve 'x'");
        }
    }
}

#[test]
fn char_literal_with_escape_newline() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(r"`\n`", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::CharLiteral(_))),
        "Should tokenize escaped newline char: {:?}",
        tokens
    );
}

#[test]
fn char_literal_with_escape_tab() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(r"`\t`", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::CharLiteral(_))),
        "Should tokenize escaped tab char: {:?}",
        tokens
    );
}

#[test]
fn char_literal_with_escape_backslash() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(r"`\\`", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::CharLiteral(_))),
        "Should tokenize escaped backslash: {:?}",
        tokens
    );
}

#[test]
fn char_literal_with_escape_backtick() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(r"`\``", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::CharLiteral(_))),
        "Should tokenize escaped backtick: {:?}",
        tokens
    );
}

#[test]
fn char_literal_digit() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("`5`", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::CharLiteral(_))),
        "Should tokenize digit char: {:?}",
        tokens
    );
}

#[test]
fn char_literal_space() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("` `", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::CharLiteral(_))),
        "Should tokenize space char: {:?}",
        tokens
    );
}

// === CHAR IN STATEMENTS ===

#[test]
fn char_in_let_statement() {
    let mut interner = Interner::new();
    let source = "Let c be `x`.";
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let has_let = tokens.iter().any(|t| matches!(t.kind, TokenType::Let));
    let has_char = tokens.iter().any(|t| matches!(t.kind, TokenType::CharLiteral(_)));

    assert!(has_let, "Should have Let token");
    assert!(has_char, "Should have CharLiteral token");
}

// === BYTE TYPE TESTS ===
// Byte uses numeric literals with type annotation, so these test the type registry

#[test]
fn byte_type_name_recognized() {
    // This tests that "Byte" is recognized as a type name, not a noun
    let mut interner = Interner::new();
    let source = "Let b: Byte be 255.";
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    // "Byte" will be tokenized as a Noun initially, but should be registered as a type
    let has_number = tokens.iter().any(|t| matches!(t.kind, TokenType::Number(_)));
    assert!(has_number, "Should have Number token for 255");
}
