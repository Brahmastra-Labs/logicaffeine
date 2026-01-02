//! Phase CRDT Language: Tests for Language Integration (Wave 5)
//!
//! Wave 5 of CRDT Expansion: Language integration for new CRDT types.
//!
//! TDD: These are RED tests - they define the spec before implementation.

mod common;

use logos::compile::compile_to_rust;
use logos::lexer::Lexer;
use logos::token::TokenType;
use logos::Interner;

// Helper to tokenize and collect token types
fn tokenize(source: &str) -> Vec<TokenType> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    tokens.into_iter().map(|t| t.kind).collect()
}

// ===== WAVE 5.1: TOKEN TESTS =====

#[test]
fn test_tokenize_decrease() {
    let source = "## Main\nDecrease x's count by 5.";
    let tokens = tokenize(source);
    assert!(
        tokens.iter().any(|t| *t == TokenType::Decrease),
        "Expected Decrease token in {:?}",
        tokens
    );
}

#[test]
fn test_tokenize_tally() {
    let source = "## Definition\nA Game is Shared and has:\n    a score, which is a Tally.";
    let tokens = tokenize(source);
    assert!(
        tokens.iter().any(|t| *t == TokenType::Tally),
        "Expected Tally token in {:?}",
        tokens
    );
}

#[test]
fn test_tokenize_shared_set() {
    let source = "## Definition\nA Party is Shared and has:\n    a guests, which is a SharedSet of Text.";
    let tokens = tokenize(source);
    assert!(
        tokens.iter().any(|t| *t == TokenType::SharedSet),
        "Expected SharedSet token in {:?}",
        tokens
    );
}

#[test]
fn test_tokenize_shared_sequence() {
    let source = "## Definition\nA Doc is Shared and has:\n    a lines, which is a SharedSequence of Text.";
    let tokens = tokenize(source);
    assert!(
        tokens.iter().any(|t| *t == TokenType::SharedSequence),
        "Expected SharedSequence token in {:?}",
        tokens
    );
}

#[test]
fn test_tokenize_divergent() {
    let source = "## Definition\nA Page is Shared and has:\n    a title, which is a Divergent Text.";
    let tokens = tokenize(source);
    assert!(
        tokens.iter().any(|t| *t == TokenType::Divergent),
        "Expected Divergent token in {:?}",
        tokens
    );
}

#[test]
fn test_tokenize_append() {
    let source = "## Main\nAppend \"hello\" to doc's lines.";
    let tokens = tokenize(source);
    assert!(
        tokens.iter().any(|t| *t == TokenType::Append),
        "Expected Append token in {:?}",
        tokens
    );
}

#[test]
fn test_tokenize_resolve() {
    let source = "## Main\nResolve page's title to \"Final\".";
    let tokens = tokenize(source);
    assert!(
        tokens.iter().any(|t| *t == TokenType::Resolve),
        "Expected Resolve token in {:?}",
        tokens
    );
}

// ===== WAVE 5.2: CODEGEN TYPE TESTS =====

#[test]
fn test_codegen_tally_type() {
    let source = r#"
## Definition
A Game is Shared and has:
    a score, which is a Tally.

## Main
Let g be a new Game.
Show "done".
"#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logos_core::crdt::PNCounter"),
        "Expected PNCounter in generated code:\n{}",
        rust
    );
}

#[test]
fn test_codegen_shared_set_type() {
    let source = r#"
## Definition
A Party is Shared and has:
    a guests, which is a SharedSet of Text.

## Main
Let p be a new Party.
Show "done".
"#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logos_core::crdt::ORSet<String>"),
        "Expected ORSet<String> in generated code:\n{}",
        rust
    );
}

#[test]
fn test_codegen_shared_sequence_type() {
    let source = r#"
## Definition
A Document is Shared and has:
    a lines, which is a SharedSequence of Text.

## Main
Let d be a new Document.
Show "done".
"#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logos_core::crdt::RGA<String>"),
        "Expected RGA<String> in generated code:\n{}",
        rust
    );
}

#[test]
fn test_codegen_divergent_type() {
    let source = r#"
## Definition
A WikiPage is Shared and has:
    a title, which is a Divergent Text.

## Main
Let p be a new WikiPage.
Show "done".
"#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logos_core::crdt::MVRegister<String>"),
        "Expected MVRegister<String> in generated code:\n{}",
        rust
    );
}

// ===== WAVE 5.3: STATEMENT CODEGEN TESTS =====

#[test]
fn test_codegen_decrease_statement() {
    let source = r#"
## Definition
A Game is Shared and has:
    a score, which is a Tally.

## Main
Let g be a new Game.
Increase g's score by 10.
Decrease g's score by 3.
Show "done".
"#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains(".decrement(3"),
        "Expected .decrement(3 in generated code:\n{}",
        rust
    );
}

#[test]
fn test_codegen_append_statement() {
    let source = r#"
## Definition
A Document is Shared and has:
    a lines, which is a SharedSequence of Text.

## Main
Let d be a new Document.
Append "Hello" to d's lines.
Show "done".
"#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains(".append("),
        "Expected .append( in generated code:\n{}",
        rust
    );
}

#[test]
fn test_codegen_add_to_set_statement() {
    let source = r#"
## Definition
A Party is Shared and has:
    a guests, which is a SharedSet of Text.

## Main
Let p be a new Party.
Add "Alice" to p's guests.
Show "done".
"#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains(".insert("),
        "Expected .insert( in generated code:\n{}",
        rust
    );
}

#[test]
fn test_codegen_resolve_statement() {
    let source = r#"
## Definition
A WikiPage is Shared and has:
    a title, which is a Divergent Text.

## Main
Let p be a new WikiPage.
Resolve p's title to "Final".
Show "done".
"#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains(".resolve("),
        "Expected .resolve( in generated code:\n{}",
        rust
    );
}

// ===== WAVE 5.4: E2E EXECUTION TESTS =====

fn run_logos_e2e(source: &str) -> Result<String, String> {
    let result = common::run_logos(source);
    if result.success {
        Ok(result.stdout)
    } else {
        Err(result.stderr)
    }
}

#[test]
fn test_tally_e2e() {
    let source = r#"
## Definition
A Game is Shared and has:
    a score, which is a Tally.

## Main
Let g be a new Game.
Increase g's score by 100.
Decrease g's score by 30.
Show g's score.
"#;

    let result = run_logos_e2e(source);
    assert!(result.is_ok(), "Compile failed: {:?}", result.err());
    let output = result.unwrap();
    assert!(
        output.contains("70"),
        "Expected 70 in output:\n{}",
        output
    );
}

#[test]
fn test_shared_set_contains_e2e() {
    let source = r#"
## Definition
A Party is Shared and has:
    a guests, which is a SharedSet of Text.

## Main
Let p be a new Party.
Add "Alice" to p's guests.
If p's guests contains "Alice":
    Show "Found Alice".
Otherwise:
    Show "Not found".
"#;

    let result = run_logos_e2e(source);
    assert!(result.is_ok(), "Compile failed: {:?}", result.err());
    let output = result.unwrap();
    assert!(
        output.contains("Found Alice"),
        "Expected 'Found Alice' in output:\n{}",
        output
    );
}

#[test]
fn test_shared_sequence_append_e2e() {
    let source = r#"
## Definition
A Document is Shared and has:
    a lines, which is a SharedSequence of Text.

## Main
Let d be a new Document.
Append "Line 1" to d's lines.
Append "Line 2" to d's lines.
Show length of d's lines.
"#;

    let result = run_logos_e2e(source);
    assert!(result.is_ok(), "Compile failed: {:?}", result.err());
    let output = result.unwrap();
    assert!(
        output.contains("2"),
        "Expected 2 in output:\n{}",
        output
    );
}

#[test]
fn test_divergent_values_e2e() {
    // Test basic Divergent (MVRegister) usage - set and show
    let source = r#"
## Definition
A WikiPage is Shared and has:
    a title, which is a Divergent Text.

## Main
Let p be a new WikiPage.
Set p's title to "Draft".
Show p's title.
"#;

    let result = run_logos_e2e(source);
    assert!(result.is_ok(), "Compile failed: {:?}", result.err());
    let output = result.unwrap();
    assert!(
        output.contains("Draft"),
        "Expected 'Draft' in output:\n{}",
        output
    );
}
