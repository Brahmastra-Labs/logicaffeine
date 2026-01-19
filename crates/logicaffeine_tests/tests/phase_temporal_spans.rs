//! Phase: Temporal Spans
//!
//! Tests for Span parsing - calendar-aware durations with variable length units.
//! Spans are compound types, not kernel primitives, because calendar arithmetic
//! is context-dependent (e.g., "1 month" varies 28-31 days).

use logicaffeine_base::Interner;
use logicaffeine_language::lexer::Lexer;
use logicaffeine_language::token::TokenType;

fn tokenize(input: &str) -> Vec<logicaffeine_language::token::Token> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    lexer.tokenize()
}

// =============================================================================
// Test 4.1: Calendar unit words are recognized
// =============================================================================

#[test]
fn lex_day_unit() {
    let tokens = tokenize("3 days");

    assert!(matches!(tokens[0].kind, TokenType::Number(_)), "Expected Number, got {:?}", tokens[0].kind);
    assert!(matches!(tokens[1].kind, TokenType::CalendarUnit(_)), "Expected CalendarUnit, got {:?}", tokens[1].kind);
}

#[test]
fn lex_week_unit() {
    let tokens = tokenize("2 weeks");

    assert!(matches!(tokens[0].kind, TokenType::Number(_)));
    assert!(matches!(tokens[1].kind, TokenType::CalendarUnit(_)));
}

#[test]
fn lex_month_unit() {
    let tokens = tokenize("1 month");

    assert!(matches!(tokens[0].kind, TokenType::Number(_)));
    assert!(matches!(tokens[1].kind, TokenType::CalendarUnit(_)));
}

#[test]
fn lex_year_unit() {
    let tokens = tokenize("5 years");

    assert!(matches!(tokens[0].kind, TokenType::Number(_)));
    assert!(matches!(tokens[1].kind, TokenType::CalendarUnit(_)));
}

#[test]
fn lex_singular_units() {
    // Singular forms should also work
    let tokens_day = tokenize("1 day");
    assert!(matches!(tokens_day[1].kind, TokenType::CalendarUnit(_)));

    let tokens_week = tokenize("1 week");
    assert!(matches!(tokens_week[1].kind, TokenType::CalendarUnit(_)));

    let tokens_month = tokenize("1 month");
    assert!(matches!(tokens_month[1].kind, TokenType::CalendarUnit(_)));

    let tokens_year = tokenize("1 year");
    assert!(matches!(tokens_year[1].kind, TokenType::CalendarUnit(_)));
}

// =============================================================================
// Test 4.2: Ago and Hence keywords
// =============================================================================

#[test]
fn lex_ago_keyword() {
    let tokens = tokenize("3 days ago");

    assert!(matches!(tokens[0].kind, TokenType::Number(_)));
    assert!(matches!(tokens[1].kind, TokenType::CalendarUnit(_)));
    assert!(matches!(tokens[2].kind, TokenType::Ago), "Expected Ago, got {:?}", tokens[2].kind);
}

#[test]
fn lex_hence_keyword() {
    let tokens = tokenize("3 days hence");

    assert!(matches!(tokens[0].kind, TokenType::Number(_)));
    assert!(matches!(tokens[1].kind, TokenType::CalendarUnit(_)));
    assert!(matches!(tokens[2].kind, TokenType::Hence), "Expected Hence, got {:?}", tokens[2].kind);
}

// =============================================================================
// Test 4.3: Calendar units are NOT duration literals
// =============================================================================

#[test]
fn calendar_units_are_not_durations() {
    // "3 days" should be Number + CalendarUnit, NOT DurationLiteral
    // This is because calendar durations are context-dependent
    let tokens = tokenize("3 days");

    assert!(!matches!(tokens[0].kind, TokenType::DurationLiteral { .. }),
        "Calendar span components should not be DurationLiteral: {:?}", tokens[0].kind);
}

#[test]
fn duration_suffix_is_still_duration() {
    // But "3s" should still be a DurationLiteral (SI time unit)
    let tokens = tokenize("3s");

    assert!(matches!(tokens[0].kind, TokenType::DurationLiteral { .. }),
        "SI duration suffix should be DurationLiteral: {:?}", tokens[0].kind);
}

// =============================================================================
// Test 4.4: Mixed span expressions
// =============================================================================

#[test]
fn lex_mixed_span_units() {
    let tokens = tokenize("2 months and 3 days");

    // Should produce: Number, CalendarUnit, And, Number, CalendarUnit
    assert!(matches!(tokens[0].kind, TokenType::Number(_)));
    assert!(matches!(tokens[1].kind, TokenType::CalendarUnit(_)));
    assert!(matches!(tokens[2].kind, TokenType::And));
    assert!(matches!(tokens[3].kind, TokenType::Number(_)));
    assert!(matches!(tokens[4].kind, TokenType::CalendarUnit(_)));
}

// =============================================================================
// Test 4.5: Before and After as binary operators
// =============================================================================

#[test]
fn lex_before_keyword() {
    let tokens = tokenize("3 days before 2026-05-20");

    // Number, CalendarUnit, Before, DateLiteral
    assert!(matches!(tokens[0].kind, TokenType::Number(_)));
    assert!(matches!(tokens[1].kind, TokenType::CalendarUnit(_)));
    assert!(matches!(tokens[2].kind, TokenType::Before), "Expected Before, got {:?}", tokens[2].kind);
    assert!(matches!(tokens[3].kind, TokenType::DateLiteral { .. }));
}

// Note: "After" already exists in token.rs for timeout branches
// We may need to reuse it for span arithmetic

// =============================================================================
// Step 1: LogosSpan Type Tests (RED → GREEN)
// =============================================================================

#[test]
fn logos_span_display_days_only() {
    use logicaffeine_system::temporal::LogosSpan;
    let span = LogosSpan::new(0, 3);
    assert_eq!(span.to_string(), "3 days");
}

#[test]
fn logos_span_display_singular_day() {
    use logicaffeine_system::temporal::LogosSpan;
    let span = LogosSpan::new(0, 1);
    assert_eq!(span.to_string(), "1 day");
}

#[test]
fn logos_span_display_months_only() {
    use logicaffeine_system::temporal::LogosSpan;
    let span = LogosSpan::new(2, 0);
    assert_eq!(span.to_string(), "2 months");
}

#[test]
fn logos_span_display_compound() {
    use logicaffeine_system::temporal::LogosSpan;
    let span = LogosSpan::new(2, 5);
    assert_eq!(span.to_string(), "2 months and 5 days");
}

#[test]
fn logos_span_display_years_and_months() {
    use logicaffeine_system::temporal::LogosSpan;
    // 14 months = 1 year and 2 months
    let span = LogosSpan::new(14, 3);
    assert_eq!(span.to_string(), "1 year and 2 months and 3 days");
}

#[test]
fn logos_span_from_weeks() {
    use logicaffeine_system::temporal::LogosSpan;
    // 2 weeks = 14 days
    let span = LogosSpan::from_weeks_days(2, 3);
    assert_eq!(span.days, 17); // 2*7 + 3
    assert_eq!(span.months, 0);
}

#[test]
fn logos_span_from_years_months_days() {
    use logicaffeine_system::temporal::LogosSpan;
    // 1 year, 2 months, 5 days
    let span = LogosSpan::from_years_months_days(1, 2, 5);
    assert_eq!(span.months, 14); // 12 + 2
    assert_eq!(span.days, 5);
}

#[test]
fn logos_span_negate() {
    use logicaffeine_system::temporal::LogosSpan;
    let span = LogosSpan::new(2, 5);
    let negated = span.negate();
    assert_eq!(negated.months, -2);
    assert_eq!(negated.days, -5);
}

// =============================================================================
// Step 2: Literal::Span and RuntimeValue::Span Tests (RED → GREEN)
// =============================================================================

// Helper function to run interpreter and get output
fn run_interpreter(code: &str) -> InterpreterTestResult {
    use logicaffeine_compile::interpret_for_ui;
    use futures::executor::block_on;

    let result = block_on(interpret_for_ui(code));
    InterpreterTestResult {
        output: result.lines.join("\n"),
        error: result.error,
    }
}

struct InterpreterTestResult {
    output: String,
    error: Option<String>,
}

#[test]
fn interpreter_simple_span_days() {
    let result = run_interpreter("## Main\nLet x be 3 days.\nShow x.");
    if let Some(ref err) = result.error {
        panic!("Error: {}", err);
    }
    assert!(result.output.contains("3 days"), "Output was: {}", result.output);
}

#[test]
fn interpreter_simple_span_months() {
    let result = run_interpreter("## Main\nLet x be 2 months.\nShow x.");
    if let Some(ref err) = result.error {
        panic!("Error: {}", err);
    }
    assert!(result.output.contains("2 months"), "Output was: {}", result.output);
}

#[test]
fn interpreter_compound_span() {
    let result = run_interpreter("## Main\nLet x be 2 months and 5 days.\nShow x.");
    if let Some(ref err) = result.error {
        panic!("Error: {}", err);
    }
    assert!(result.output.contains("2 months and 5 days"), "Output was: {}", result.output);
}

#[test]
fn interpreter_span_with_weeks() {
    let result = run_interpreter("## Main\nLet x be 2 weeks.\nShow x.");
    if let Some(ref err) = result.error {
        panic!("Error: {}", err);
    }
    // 2 weeks = 14 days
    assert!(result.output.contains("14 days"), "Output was: {}", result.output);
}

#[test]
fn interpreter_span_with_years() {
    let result = run_interpreter("## Main\nLet x be 1 year.\nShow x.");
    if let Some(ref err) = result.error {
        panic!("Error: {}", err);
    }
    assert!(result.output.contains("1 year"), "Output was: {}", result.output);
}

// =============================================================================
// Step 4: "today" Builtin Tests (RED → GREEN)
// =============================================================================

#[test]
fn interpreter_today_builtin() {
    let result = run_interpreter("## Main\nLet d be today.\nShow d.");
    if let Some(ref err) = result.error {
        panic!("Error: {}", err);
    }
    // Should show current date in YYYY-MM-DD format
    // The output should contain a year in valid range
    assert!(
        result.output.contains("202") || result.output.contains("203"),
        "Expected a date in the 2020s-2030s, got: {}",
        result.output
    );
}

// =============================================================================
// Step 5: Date +/- Span Arithmetic Tests (RED → GREEN)
// =============================================================================

#[test]
fn interpreter_date_minus_span() {
    let result = run_interpreter(r#"## Main
Let graduation be 2026-05-20.
Let reminder be graduation - 3 days.
Show reminder.
"#);
    if let Some(ref err) = result.error {
        panic!("Error: {}", err);
    }
    assert!(result.output.contains("2026-05-17"), "Output was: {}", result.output);
}

#[test]
fn interpreter_date_plus_span() {
    let result = run_interpreter(r#"## Main
Let start be 2026-01-15.
Let deadline be start + 2 months.
Show deadline.
"#);
    if let Some(ref err) = result.error {
        panic!("Error: {}", err);
    }
    assert!(result.output.contains("2026-03-15"), "Output was: {}", result.output);
}

#[test]
fn interpreter_date_plus_compound_span() {
    let result = run_interpreter(r#"## Main
Let start be 2026-01-10.
Let end be start + 1 month and 5 days.
Show end.
"#);
    if let Some(ref err) = result.error {
        panic!("Error: {}", err);
    }
    assert!(result.output.contains("2026-02-15"), "Output was: {}", result.output);
}

// =============================================================================
// Step 6: Time-of-Day Lexing Tests (RED → GREEN)
// =============================================================================

#[test]
fn lex_time_12hour_pm() {
    use logicaffeine_base::Interner;
    use logicaffeine_language::lexer::Lexer;
    use logicaffeine_language::token::TokenType;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new("4pm", &mut interner);
    let tokens = lexer.tokenize();

    match &tokens[0].kind {
        TokenType::TimeLiteral { nanos_from_midnight } => {
            // 4pm = 16:00 = 16 * 3600 * 1_000_000_000 nanoseconds
            let expected = 16i64 * 3600 * 1_000_000_000;
            assert_eq!(*nanos_from_midnight, expected);
        }
        other => panic!("Expected TimeLiteral, got {:?}", other),
    }
}

#[test]
fn lex_time_12hour_am() {
    use logicaffeine_base::Interner;
    use logicaffeine_language::lexer::Lexer;
    use logicaffeine_language::token::TokenType;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new("9am", &mut interner);
    let tokens = lexer.tokenize();

    match &tokens[0].kind {
        TokenType::TimeLiteral { nanos_from_midnight } => {
            // 9am = 09:00 = 9 * 3600 * 1_000_000_000 nanoseconds
            let expected = 9i64 * 3600 * 1_000_000_000;
            assert_eq!(*nanos_from_midnight, expected);
        }
        other => panic!("Expected TimeLiteral, got {:?}", other),
    }
}

#[test]
fn lex_time_with_minutes() {
    use logicaffeine_base::Interner;
    use logicaffeine_language::lexer::Lexer;
    use logicaffeine_language::token::TokenType;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new("9:30am", &mut interner);
    let tokens = lexer.tokenize();

    match &tokens[0].kind {
        TokenType::TimeLiteral { nanos_from_midnight } => {
            // 9:30am = 09:30 = (9 * 3600 + 30 * 60) * 1_000_000_000 nanoseconds
            let expected = (9i64 * 3600 + 30 * 60) * 1_000_000_000;
            assert_eq!(*nanos_from_midnight, expected);
        }
        other => panic!("Expected TimeLiteral, got {:?}", other),
    }
}

#[test]
fn lex_time_noon() {
    use logicaffeine_base::Interner;
    use logicaffeine_language::lexer::Lexer;
    use logicaffeine_language::token::TokenType;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new("noon", &mut interner);
    let tokens = lexer.tokenize();

    match &tokens[0].kind {
        TokenType::TimeLiteral { nanos_from_midnight } => {
            // noon = 12:00 = 12 * 3600 * 1_000_000_000 nanoseconds
            let expected = 12i64 * 3600 * 1_000_000_000;
            assert_eq!(*nanos_from_midnight, expected);
        }
        other => panic!("Expected TimeLiteral, got {:?}", other),
    }
}

#[test]
fn lex_time_midnight() {
    use logicaffeine_base::Interner;
    use logicaffeine_language::lexer::Lexer;
    use logicaffeine_language::token::TokenType;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new("midnight", &mut interner);
    let tokens = lexer.tokenize();

    match &tokens[0].kind {
        TokenType::TimeLiteral { nanos_from_midnight } => {
            // midnight = 00:00 = 0 nanoseconds
            assert_eq!(*nanos_from_midnight, 0);
        }
        other => panic!("Expected TimeLiteral, got {:?}", other),
    }
}

// =============================================================================
// Step 7: Date + Time Combination Tests (RED → GREEN)
// =============================================================================

#[test]
fn interpreter_time_literal() {
    let result = run_interpreter("## Main\nLet t be 4pm.\nShow t.");
    if let Some(ref err) = result.error {
        panic!("Error: {}", err);
    }
    // Should show time-of-day
    assert!(
        result.output.contains("16:00") || result.output.contains("4pm"),
        "Output was: {}",
        result.output
    );
}

#[test]
fn interpreter_date_at_time() {
    let result = run_interpreter(r#"## Main
Let meeting be 2026-05-20 at 4pm.
Show meeting.
"#);
    if let Some(ref err) = result.error {
        panic!("Error: {}", err);
    }
    // Should show as Moment with both date and time
    assert!(
        result.output.contains("2026-05-20") && result.output.contains("16:00"),
        "Output was: {}",
        result.output
    );
}

// =============================================================================
// Step 8: Time-of-Day Comparisons (RED → GREEN)
// =============================================================================

#[test]
fn interpreter_time_comparison_less_than() {
    let result = run_interpreter(r#"## Main
Let meeting be 9am.
Let deadline be 5pm.
If meeting < deadline:
    Show "morning meeting".
"#);
    if let Some(ref err) = result.error {
        panic!("Error: {}", err);
    }
    assert!(result.output.contains("morning meeting"), "Output was: {}", result.output);
}

#[test]
fn interpreter_time_comparison_greater_than() {
    let result = run_interpreter(r#"## Main
Let evening be 8pm.
Let noon_time be noon.
If evening > noon_time:
    Show "after noon".
"#);
    if let Some(ref err) = result.error {
        panic!("Error: {}", err);
    }
    assert!(result.output.contains("after noon"), "Output was: {}", result.output);
}

#[test]
fn interpreter_moment_vs_time_comparison() {
    // Compare a moment's time-of-day against a time literal
    let result = run_interpreter(r#"## Main
Let meeting be 2026-05-20 at 3pm.
If meeting < 4pm:
    Show "before 4pm".
"#);
    if let Some(ref err) = result.error {
        panic!("Error: {}", err);
    }
    assert!(result.output.contains("before 4pm"), "Output was: {}", result.output);
}
