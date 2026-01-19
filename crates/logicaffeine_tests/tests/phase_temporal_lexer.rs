//! Phase: Temporal Lexer
//!
//! Tests for lexing duration literals with SI prefixes and ISO-8601 dates.

use logicaffeine_base::Interner;
use logicaffeine_language::lexer::Lexer;
use logicaffeine_language::token::TokenType;

fn tokenize(input: &str) -> Vec<logicaffeine_language::token::Token> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    lexer.tokenize()
}

fn assert_duration_nanos(input: &str, expected_nanos: i64) {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();
    match &tokens[0].kind {
        TokenType::DurationLiteral { nanos, .. } => {
            assert_eq!(*nanos, expected_nanos, "Input '{}': expected {} nanos, got {}", input, expected_nanos, nanos);
        }
        other => panic!("Input '{}': Expected DurationLiteral, got {:?}", input, other),
    }
}

// =============================================================================
// Test 2.1: Basic duration token with milliseconds suffix
// =============================================================================

#[test]
fn lex_milliseconds_suffix() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("500ms", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        matches!(tokens[0].kind, TokenType::DurationLiteral { .. }),
        "Expected DurationLiteral, got {:?}",
        tokens[0].kind
    );
}

// =============================================================================
// Test 2.2: Full SI prefix table
// =============================================================================

#[test]
fn lex_nanoseconds() {
    assert_duration_nanos("50ns", 50);
    assert_duration_nanos("1ns", 1);
    assert_duration_nanos("999ns", 999);
}

#[test]
fn lex_microseconds_ascii() {
    assert_duration_nanos("250us", 250_000);
    assert_duration_nanos("1us", 1_000);
    assert_duration_nanos("999us", 999_000);
}

#[test]
fn lex_microseconds_greek_mu() {
    assert_duration_nanos("250μs", 250_000);
    assert_duration_nanos("1μs", 1_000);
}

#[test]
fn lex_milliseconds() {
    assert_duration_nanos("500ms", 500_000_000);
    assert_duration_nanos("1ms", 1_000_000);
    assert_duration_nanos("999ms", 999_000_000);
}

#[test]
fn lex_seconds() {
    assert_duration_nanos("2s", 2_000_000_000);
    assert_duration_nanos("1s", 1_000_000_000);
    assert_duration_nanos("60s", 60_000_000_000);
}

#[test]
fn lex_seconds_alternate_suffix() {
    assert_duration_nanos("2sec", 2_000_000_000);
    assert_duration_nanos("1sec", 1_000_000_000);
}

#[test]
fn lex_minutes() {
    assert_duration_nanos("5min", 300_000_000_000);
    assert_duration_nanos("1min", 60_000_000_000);
    assert_duration_nanos("60min", 3_600_000_000_000);
}

#[test]
fn lex_hours() {
    assert_duration_nanos("1h", 3_600_000_000_000);
    assert_duration_nanos("2h", 7_200_000_000_000);
    assert_duration_nanos("24h", 86_400_000_000_000);
}

#[test]
fn lex_hours_alternate_suffix() {
    assert_duration_nanos("1hr", 3_600_000_000_000);
    assert_duration_nanos("2hr", 7_200_000_000_000);
}

// =============================================================================
// Test 2.3: Compound durations (lexer emits separate tokens, parser combines)
// =============================================================================

#[test]
fn lex_compound_duration_as_separate_tokens() {
    let tokens = tokenize("1s 500ms");

    // Lexer emits TWO tokens; parser will combine them
    assert!(matches!(tokens[0].kind, TokenType::DurationLiteral { nanos: 1_000_000_000, .. }));
    assert!(matches!(tokens[1].kind, TokenType::DurationLiteral { nanos: 500_000_000, .. }));
}

// =============================================================================
// Test 2.4: Greek mu and ASCII u are equivalent
// =============================================================================

#[test]
fn lex_greek_mu_and_ascii_u_are_equivalent() {
    let mut interner = Interner::new();

    let mut lexer1 = Lexer::new("250μs", &mut interner);
    let tokens1 = lexer1.tokenize();

    let mut lexer2 = Lexer::new("250us", &mut interner);
    let tokens2 = lexer2.tokenize();

    // Both produce identical nanosecond values
    match (&tokens1[0].kind, &tokens2[0].kind) {
        (TokenType::DurationLiteral { nanos: n1, .. }, TokenType::DurationLiteral { nanos: n2, .. }) => {
            assert_eq!(n1, n2, "Greek μ and ASCII u should produce same nanos");
        }
        _ => panic!("Expected DurationLiteral tokens"),
    }
}

// =============================================================================
// Test 2.5: Duration in context (sentence)
// =============================================================================

#[test]
fn lex_duration_in_sentence() {
    let tokens = tokenize("Wait for 100ms.");

    // Find the duration token
    let duration_token = tokens.iter().find(|t| matches!(t.kind, TokenType::DurationLiteral { .. }));
    assert!(duration_token.is_some(), "Should find a DurationLiteral in 'Wait for 100ms.'");

    match &duration_token.unwrap().kind {
        TokenType::DurationLiteral { nanos, .. } => {
            assert_eq!(*nanos, 100_000_000);
        }
        _ => unreachable!(),
    }
}

// =============================================================================
// Test 2.6: Edge cases
// =============================================================================

#[test]
fn lex_zero_duration() {
    assert_duration_nanos("0ms", 0);
    assert_duration_nanos("0s", 0);
    assert_duration_nanos("0ns", 0);
}

#[test]
fn lex_large_duration() {
    // 292 years in nanoseconds (near i64 limit)
    assert_duration_nanos("9223372036s", 9_223_372_036_000_000_000);
}

// =============================================================================
// Test 2.7: Non-duration numbers remain as Number tokens
// =============================================================================

#[test]
fn lex_plain_number_not_duration() {
    let tokens = tokenize("42");

    assert!(
        matches!(tokens[0].kind, TokenType::Number(_)),
        "Plain number should be Number, not DurationLiteral: {:?}",
        tokens[0].kind
    );
}

#[test]
fn lex_number_with_unknown_suffix_is_not_duration() {
    // "5xyz" should not be recognized as a duration
    let tokens = tokenize("5xyz");

    // This should either be a Number followed by identifier, or an error
    // NOT a DurationLiteral
    assert!(
        !matches!(tokens[0].kind, TokenType::DurationLiteral { .. }),
        "Unknown suffix should not create DurationLiteral: {:?}",
        tokens[0].kind
    );
}

// =============================================================================
// Phase 3: ISO-8601 Date Parsing
// =============================================================================

fn assert_date_days(input: &str, expected_days: i32) {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();
    match &tokens[0].kind {
        TokenType::DateLiteral { days } => {
            assert_eq!(*days, expected_days, "Input '{}': expected {} days, got {}", input, expected_days, days);
        }
        other => panic!("Input '{}': Expected DateLiteral, got {:?}", input, other),
    }
}

/// Test 3.1: Basic ISO-8601 date literal
#[test]
fn lex_iso8601_date() {
    let tokens = tokenize("2026-05-20");
    assert!(
        matches!(tokens[0].kind, TokenType::DateLiteral { .. }),
        "Expected DateLiteral, got {:?}",
        tokens[0].kind
    );
}

/// Test 3.2: Date vs subtraction disambiguation
#[test]
fn date_vs_subtraction() {
    // Date literal (no spaces around hyphens)
    let tokens1 = tokenize("2026-05-20");
    assert!(
        matches!(tokens1[0].kind, TokenType::DateLiteral { .. }),
        "Compact form should be DateLiteral: {:?}",
        tokens1[0].kind
    );

    // Subtraction expression (spaces)
    let tokens2 = tokenize("2026 - 5 - 20");
    assert!(
        matches!(tokens2[0].kind, TokenType::Number(_)),
        "Spaced form should be Number: {:?}",
        tokens2[0].kind
    );
    assert!(
        matches!(tokens2[1].kind, TokenType::Minus),
        "Should have Minus operator: {:?}",
        tokens2[1].kind
    );
}

/// Test 3.3: Unix epoch date
#[test]
fn lex_unix_epoch_date() {
    // 1970-01-01 is day 0
    assert_date_days("1970-01-01", 0);
}

/// Test 3.4: Various dates
#[test]
fn lex_various_dates() {
    // 2026-05-20 is 20593 days since Unix epoch
    assert_date_days("2026-05-20", 20593);

    // 2000-01-01 is 10957 days since Unix epoch
    assert_date_days("2000-01-01", 10957);

    // 1969-12-31 is -1 days (before Unix epoch)
    assert_date_days("1969-12-31", -1);
}

/// Test 3.5: Date in sentence context
#[test]
fn lex_date_in_sentence() {
    let tokens = tokenize("Let graduation be 2026-05-20.");

    let date_token = tokens.iter().find(|t| matches!(t.kind, TokenType::DateLiteral { .. }));
    assert!(date_token.is_some(), "Should find a DateLiteral in sentence");
}

/// Test 3.6: Leap year date
#[test]
fn lex_leap_year_date() {
    // Feb 29, 2024 (leap year)
    let tokens = tokenize("2024-02-29");
    assert!(
        matches!(tokens[0].kind, TokenType::DateLiteral { .. }),
        "Leap year date should be valid: {:?}",
        tokens[0].kind
    );
}
