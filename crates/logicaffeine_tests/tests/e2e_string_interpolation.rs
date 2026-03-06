//! E2E Tests: Scientific Notation + String Interpolation
//!
//! GitHub Issue #17: Best-in-Class String Formatting and Interpolation.
//!
//! Every feature is tested via BOTH:
//!   - `assert_exact_output` (Rust codegen path)
//!   - `assert_interpreter_output` (interpreter path)

mod common;
use common::{assert_exact_output, assert_interpreter_output, assert_c_output, assert_compile_fails, assert_interpreter_fails};

// =============================================================================
// Scientific Notation — Codegen
// =============================================================================

#[test]
fn e2e_scientific_notation_basic() {
    assert_exact_output(
        "## Main\nLet x be 1.5e3.\nShow x.",
        "1500",
    );
}

#[test]
fn e2e_scientific_notation_negative_exp() {
    assert_exact_output(
        "## Main\nLet x be 2.5e-2.\nShow x.",
        "0.025",
    );
}

#[test]
fn e2e_scientific_notation_positive_sign() {
    assert_exact_output(
        "## Main\nLet x be 3.0e+2.\nShow x.",
        "300",
    );
}

#[test]
fn e2e_scientific_notation_no_decimal() {
    assert_exact_output(
        "## Main\nLet x be 1e5.\nShow x.",
        "100000",
    );
}

// =============================================================================
// Scientific Notation — Interpreter
// =============================================================================

#[test]
fn e2e_scientific_notation_basic_interp() {
    assert_interpreter_output(
        "## Main\nLet x be 1.5e3.\nShow x.",
        "1500",
    );
}

#[test]
fn e2e_scientific_notation_negative_exp_interp() {
    assert_interpreter_output(
        "## Main\nLet x be 2.5e-2.\nShow x.",
        "0.025",
    );
}

// =============================================================================
// String Interpolation — Basic (Codegen)
// =============================================================================

#[test]
fn e2e_interpolation_simple() {
    assert_exact_output(
        r#"## Main
Let name be "world".
Show "Hello, {name}!"."#,
        "Hello, world!",
    );
}

#[test]
fn e2e_interpolation_expression() {
    assert_exact_output(
        r#"## Main
Let x be 5.
Show "Value: {x + 1}"."#,
        "Value: 6",
    );
}

#[test]
fn e2e_interpolation_multiple_holes() {
    assert_exact_output(
        r#"## Main
Let a be 1.
Let b be 2.
Show "{a} + {b} = {a + b}"."#,
        "1 + 2 = 3",
    );
}

#[test]
fn e2e_interpolation_int_and_text() {
    assert_exact_output(
        r#"## Main
Let n be 42.
Let s be "hi".
Show "{s}: {n}"."#,
        "hi: 42",
    );
}

#[test]
fn e2e_interpolation_no_holes() {
    assert_exact_output(
        r#"## Main
Show "plain string"."#,
        "plain string",
    );
}

// =============================================================================
// String Interpolation — Basic (Interpreter)
// =============================================================================

#[test]
fn e2e_interpolation_simple_interp() {
    assert_interpreter_output(
        r#"## Main
Let name be "world".
Show "Hello, {name}!"."#,
        "Hello, world!",
    );
}

#[test]
fn e2e_interpolation_expression_interp() {
    assert_interpreter_output(
        r#"## Main
Let x be 5.
Show "Value: {x + 1}"."#,
        "Value: 6",
    );
}

#[test]
fn e2e_interpolation_multiple_holes_interp() {
    assert_interpreter_output(
        r#"## Main
Let a be 1.
Let b be 2.
Show "{a} + {b} = {a + b}"."#,
        "1 + 2 = 3",
    );
}

#[test]
fn e2e_interpolation_int_and_text_interp() {
    assert_interpreter_output(
        r#"## Main
Let n be 42.
Let s be "hi".
Show "{s}: {n}"."#,
        "hi: 42",
    );
}

// =============================================================================
// Interpolation in Let (not just Show)
// =============================================================================

#[test]
fn e2e_interpolation_in_let() {
    assert_exact_output(
        r#"## Main
Let name be "world".
Let msg be "Hello, {name}!".
Show msg."#,
        "Hello, world!",
    );
}

#[test]
fn e2e_interpolation_in_let_interp() {
    assert_interpreter_output(
        r#"## Main
Let name be "world".
Let msg be "Hello, {name}!".
Show msg."#,
        "Hello, world!",
    );
}

// =============================================================================
// Escaped Braces
// =============================================================================

#[test]
fn e2e_interpolation_escaped_braces() {
    assert_exact_output(
        r#"## Main
Show "Use {{braces}}."."#,
        "Use {braces}.",
    );
}

#[test]
fn e2e_interpolation_escaped_and_real() {
    assert_exact_output(
        r#"## Main
Let x be 42.
Show "{{x}} = {x}"."#,
        "{x} = 42",
    );
}

#[test]
fn e2e_interpolation_escaped_braces_interp() {
    assert_interpreter_output(
        r#"## Main
Show "Use {{braces}}."."#,
        "Use {braces}.",
    );
}

#[test]
fn e2e_interpolation_escaped_and_real_interp() {
    assert_interpreter_output(
        r#"## Main
Let x be 42.
Show "{{x}} = {x}"."#,
        "{x} = 42",
    );
}

// =============================================================================
// Format Specifiers — Precision (Codegen)
// =============================================================================

#[test]
fn e2e_format_precision_2() {
    assert_exact_output(
        r#"## Main
Let pi be 3.14159.
Show "{pi:.2}"."#,
        "3.14",
    );
}

#[test]
fn e2e_format_precision_9() {
    assert_exact_output(
        r#"## Main
Show "{1.2742199912349306:.9}"."#,
        "1.274219991",
    );
}

#[test]
fn e2e_format_precision_0() {
    assert_exact_output(
        r#"## Main
Let x be 3.7.
Show "{x:.0}"."#,
        "4",
    );
}

#[test]
fn e2e_format_precision_with_variable() {
    assert_exact_output(
        r#"## Main
Let energy be 0.123456789.
Show "{energy:.9}"."#,
        "0.123456789",
    );
}

// =============================================================================
// Format Specifiers — Precision (Interpreter)
// =============================================================================

#[test]
fn e2e_format_precision_2_interp() {
    assert_interpreter_output(
        r#"## Main
Let pi be 3.14159.
Show "{pi:.2}"."#,
        "3.14",
    );
}

#[test]
fn e2e_format_precision_9_interp() {
    assert_interpreter_output(
        r#"## Main
Show "{1.2742199912349306:.9}"."#,
        "1.274219991",
    );
}

// =============================================================================
// Format Specifiers — Alignment (Codegen)
// Use delimiters to prevent test harness trimming from eating alignment spaces.
// =============================================================================

#[test]
fn e2e_format_right_align() {
    assert_exact_output(
        r#"## Main
Let s be "hi".
Show "|{s:>10}|"."#,
        "|        hi|",
    );
}

#[test]
fn e2e_format_left_align() {
    assert_exact_output(
        r#"## Main
Let s be "hi".
Show "|{s:<10}|"."#,
        "|hi        |",
    );
}

#[test]
fn e2e_format_center_align() {
    assert_exact_output(
        r#"## Main
Let s be "hi".
Show "|{s:^10}|"."#,
        "|    hi    |",
    );
}

// =============================================================================
// Format Specifiers — Alignment (Interpreter)
// =============================================================================

#[test]
fn e2e_format_right_align_interp() {
    assert_interpreter_output(
        r#"## Main
Let s be "hi".
Show "|{s:>10}|"."#,
        "|        hi|",
    );
}

#[test]
fn e2e_format_left_align_interp() {
    assert_interpreter_output(
        r#"## Main
Let s be "hi".
Show "|{s:<10}|"."#,
        "|hi        |",
    );
}

#[test]
fn e2e_format_center_align_interp() {
    assert_interpreter_output(
        r#"## Main
Let s be "hi".
Show "|{s:^10}|"."#,
        "|    hi    |",
    );
}

// =============================================================================
// Debug Format (`{var=}`) — Codegen
// =============================================================================

#[test]
fn e2e_format_debug_simple() {
    assert_exact_output(
        r#"## Main
Let v be 42.
Show "{v=}"."#,
        "v=42",
    );
}

#[test]
fn e2e_format_debug_with_precision() {
    assert_exact_output(
        r#"## Main
Let pi be 3.14159.
Show "{pi=:.2}"."#,
        "pi=3.14",
    );
}

// =============================================================================
// Debug Format (`{var=}`) — Interpreter
// =============================================================================

#[test]
fn e2e_format_debug_simple_interp() {
    assert_interpreter_output(
        r#"## Main
Let v be 42.
Show "{v=}"."#,
        "v=42",
    );
}

#[test]
fn e2e_format_debug_with_precision_interp() {
    assert_interpreter_output(
        r#"## Main
Let pi be 3.14159.
Show "{pi=:.2}"."#,
        "pi=3.14",
    );
}

// =============================================================================
// Currency Format (`{price:$}`) — Codegen
// =============================================================================

#[test]
fn e2e_format_currency() {
    assert_exact_output(
        r#"## Main
Let price be 19.99.
Show "{price:$}"."#,
        "$19.99",
    );
}

#[test]
fn e2e_format_currency_round() {
    assert_exact_output(
        r#"## Main
Let price be 5.0.
Show "{price:$}"."#,
        "$5.00",
    );
}

// =============================================================================
// Currency Format (`{price:$}`) — Interpreter
// =============================================================================

#[test]
fn e2e_format_currency_interp() {
    assert_interpreter_output(
        r#"## Main
Let price be 19.99.
Show "{price:$}"."#,
        "$19.99",
    );
}

#[test]
fn e2e_format_currency_round_interp() {
    assert_interpreter_output(
        r#"## Main
Let price be 5.0.
Show "{price:$}"."#,
        "$5.00",
    );
}

// =============================================================================
// Triple-Quote Strings — Codegen
// =============================================================================

#[test]
fn e2e_triple_quote_multiline() {
    assert_exact_output(
        "## Main\nLet msg be \"\"\"\n    Hello\n    World\n\"\"\".\nShow msg.",
        "Hello\nWorld",
    );
}

#[test]
fn e2e_triple_quote_with_interpolation() {
    assert_exact_output(
        "## Main\nLet name be \"Alice\".\nLet greeting be \"\"\"\n    Hello, {name}!\n    Welcome.\n\"\"\".\nShow greeting.",
        "Hello, Alice!\nWelcome.",
    );
}

// =============================================================================
// Triple-Quote Strings — Interpreter
// =============================================================================

#[test]
fn e2e_triple_quote_multiline_interp() {
    assert_interpreter_output(
        "## Main\nLet msg be \"\"\"\n    Hello\n    World\n\"\"\".\nShow msg.",
        "Hello\nWorld",
    );
}

#[test]
fn e2e_triple_quote_with_interpolation_interp() {
    assert_interpreter_output(
        "## Main\nLet name be \"Alice\".\nLet greeting be \"\"\"\n    Hello, {name}!\n    Welcome.\n\"\"\".\nShow greeting.",
        "Hello, Alice!\nWelcome.",
    );
}

// =============================================================================
// Benchmark Integration Tests (Codegen only — these verify the benchmarks work)
// =============================================================================

#[test]
fn e2e_benchmark_spectral_norm_format() {
    assert_exact_output(
        r#"## Main
Let result be 1.2742199912349306.
Show "{result:.9}"."#,
        "1.274219991",
    );
}

#[test]
fn e2e_benchmark_pi_leibniz_format() {
    assert_exact_output(
        r#"## Main
Let result be 3.14158265358972.
Show "{result:.15}"."#,
        "3.141582653589720",
    );
}

#[test]
fn e2e_benchmark_nbody_format() {
    assert_exact_output(
        r#"## Main
Let e1 be 0.0 - 0.169075164.
Let e2 be 0.0 - 0.169087605.
Show "{e1:.9}".
Show "{e2:.9}"."#,
        "-0.169075164\n-0.169087605",
    );
}

// =============================================================================
// C Codegen — String Interpolation
// =============================================================================

#[test]
fn e2e_c_interpolation_simple() {
    assert_c_output(
        r#"## Main
Let name be "world".
Show "Hello, {name}!"."#,
        "Hello, world!",
    );
}

#[test]
fn e2e_c_interpolation_int() {
    assert_c_output(
        r#"## Main
Let x be 42.
Show "Value: {x}"."#,
        "Value: 42",
    );
}

#[test]
fn e2e_c_interpolation_multiple() {
    assert_c_output(
        r#"## Main
Let a be 1.
Let b be 2.
Show "{a} + {b}"."#,
        "1 + 2",
    );
}

#[test]
fn e2e_c_interpolation_float() {
    assert_c_output(
        r#"## Main
Let x be 3.14.
Show "Pi is {x}"."#,
        "Pi is 3.14",
    );
}

#[test]
fn e2e_c_format_precision() {
    assert_c_output(
        r#"## Main
Let pi be 3.14159.
Show "{pi:.2}"."#,
        "3.14",
    );
}

#[test]
fn e2e_c_format_precision_9() {
    assert_c_output(
        r#"## Main
Let e be 0.123456789.
Show "{e:.9}"."#,
        "0.123456789",
    );
}

#[test]
fn e2e_c_format_debug() {
    assert_c_output(
        r#"## Main
Let v be 42.
Show "{v=}"."#,
        "v=42",
    );
}

#[test]
fn e2e_c_format_currency() {
    assert_c_output(
        r#"## Main
Let price be 19.99.
Show "{price:$}"."#,
        "$19.99",
    );
}

#[test]
fn e2e_c_escaped_braces() {
    assert_c_output(
        r#"## Main
Show "Use {{braces}}."."#,
        "Use {braces}.",
    );
}

#[test]
fn e2e_c_scientific_notation() {
    assert_c_output(
        "## Main\nLet x be 1.5e3.\nShow x.",
        "1500",
    );
}

// =============================================================================
// C Codegen — Alignment (verifying center alignment fix)
// =============================================================================

#[test]
fn e2e_c_format_right_align() {
    assert_c_output(
        r#"## Main
Let s be "hi".
Show "|{s:>10}|"."#,
        "|        hi|",
    );
}

#[test]
fn e2e_c_format_left_align() {
    assert_c_output(
        r#"## Main
Let s be "hi".
Show "|{s:<10}|"."#,
        "|hi        |",
    );
}

#[test]
fn e2e_c_format_center_align() {
    assert_c_output(
        r#"## Main
Let s be "hi".
Show "|{s:^10}|"."#,
        "|    hi    |",
    );
}

#[test]
fn e2e_c_format_center_align_int() {
    assert_c_output(
        r#"## Main
Let n be 42.
Show "|{n:^10}|"."#,
        "|    42    |",
    );
}

#[test]
fn e2e_c_format_center_align_odd() {
    assert_c_output(
        r#"## Main
Let s be "abc".
Show "|{s:^10}|"."#,
        "|   abc    |",
    );
}

// =============================================================================
// Edge Case: Adjacent Holes (no separator between holes)
// =============================================================================

#[test]
fn e2e_interpolation_adjacent_holes() {
    assert_exact_output(
        r#"## Main
Let a be "hello".
Let b be "world".
Show "{a}{b}"."#,
        "helloworld",
    );
}

#[test]
fn e2e_interpolation_adjacent_holes_interp() {
    assert_interpreter_output(
        r#"## Main
Let a be "hello".
Let b be "world".
Show "{a}{b}"."#,
        "helloworld",
    );
}

// =============================================================================
// Edge Case: Only-Hole String (string is entirely one hole)
// =============================================================================

#[test]
fn e2e_interpolation_only_hole() {
    assert_exact_output(
        r#"## Main
Let x be 42.
Show "{x}"."#,
        "42",
    );
}

#[test]
fn e2e_interpolation_only_hole_interp() {
    assert_interpreter_output(
        r#"## Main
Let x be 42.
Show "{x}"."#,
        "42",
    );
}

// =============================================================================
// Edge Case: Three Adjacent Holes
// =============================================================================

#[test]
fn e2e_interpolation_three_adjacent() {
    assert_exact_output(
        r#"## Main
Let a be 1.
Let b be 2.
Let c be 3.
Show "{a}{b}{c}"."#,
        "123",
    );
}

#[test]
fn e2e_interpolation_three_adjacent_interp() {
    assert_interpreter_output(
        r#"## Main
Let a be 1.
Let b be 2.
Let c be 3.
Show "{a}{b}{c}"."#,
        "123",
    );
}

// =============================================================================
// Edge Case: Integer with Currency Format
// =============================================================================

#[test]
fn e2e_format_currency_integer() {
    assert_exact_output(
        r#"## Main
Let price be 42.
Show "{price:$}"."#,
        "$42.00",
    );
}

#[test]
fn e2e_format_currency_integer_interp() {
    assert_interpreter_output(
        r#"## Main
Let price be 42.
Show "{price:$}"."#,
        "$42.00",
    );
}

// =============================================================================
// Edge Case: Bool in Hole
// =============================================================================

#[test]
fn e2e_interpolation_bool() {
    assert_exact_output(
        r#"## Main
Let flag be true.
Show "flag is {flag}"."#,
        "flag is true",
    );
}

#[test]
fn e2e_interpolation_bool_interp() {
    assert_interpreter_output(
        r#"## Main
Let flag be true.
Show "flag is {flag}"."#,
        "flag is true",
    );
}

// =============================================================================
// Edge Case: Interpolation in Let, Then Used in Another Interpolation
// =============================================================================

#[test]
fn e2e_interpolation_chained_let() {
    assert_exact_output(
        r#"## Main
Let name be "Alice".
Let greeting be "Hello, {name}".
Show "{greeting}!"."#,
        "Hello, Alice!",
    );
}

#[test]
fn e2e_interpolation_chained_let_interp() {
    assert_interpreter_output(
        r#"## Main
Let name be "Alice".
Let greeting be "Hello, {name}".
Show "{greeting}!"."#,
        "Hello, Alice!",
    );
}

// =============================================================================
// Edge Case: Integer with Precision (should display as float)
// =============================================================================

#[test]
fn e2e_format_precision_integer() {
    assert_exact_output(
        r#"## Main
Let n be 42.
Show "{n:.2}"."#,
        "42.00",
    );
}

#[test]
fn e2e_format_precision_integer_interp() {
    assert_interpreter_output(
        r#"## Main
Let n be 42.
Show "{n:.2}"."#,
        "42.00",
    );
}

// =============================================================================
// Edge Case: Center Alignment Consistency (Rust codegen vs Interpreter)
// =============================================================================

#[test]
fn e2e_format_center_align_int() {
    assert_exact_output(
        r#"## Main
Let n be 42.
Show "|{n:^10}|"."#,
        "|    42    |",
    );
}

#[test]
fn e2e_format_center_align_int_interp() {
    assert_interpreter_output(
        r#"## Main
Let n be 42.
Show "|{n:^10}|"."#,
        "|    42    |",
    );
}

#[test]
fn e2e_format_center_align_odd_width() {
    assert_exact_output(
        r#"## Main
Let s be "abc".
Show "|{s:^10}|"."#,
        "|   abc    |",
    );
}

#[test]
fn e2e_format_center_align_odd_width_interp() {
    assert_interpreter_output(
        r#"## Main
Let s be "abc".
Show "|{s:^10}|"."#,
        "|   abc    |",
    );
}

// =============================================================================
// Negative Tests: Parse Errors
// =============================================================================

#[test]
fn e2e_interpolation_empty_hole_error() {
    assert_compile_fails(
        r#"## Main
Show "Hello {}!"."#,
        "Empty interpolation hole",
    );
}

#[test]
fn e2e_interpolation_empty_hole_error_interp() {
    assert_interpreter_fails(
        r#"## Main
Show "Hello {}!"."#,
        "Empty interpolation hole",
    );
}

#[test]
fn e2e_interpolation_invalid_format_spec_precision() {
    assert_compile_fails(
        r#"## Main
Let x be 3.14.
Show "{x:.abc}"."#,
        "Invalid format specifier",
    );
}

#[test]
fn e2e_interpolation_invalid_format_spec_precision_interp() {
    assert_interpreter_fails(
        r#"## Main
Let x be 3.14.
Show "{x:.abc}"."#,
        "Invalid format specifier",
    );
}

#[test]
fn e2e_interpolation_invalid_format_spec_alignment() {
    assert_compile_fails(
        r#"## Main
Let x be 42.
Show "{x:>abc}"."#,
        "Invalid format specifier",
    );
}

#[test]
fn e2e_interpolation_unclosed_brace_error() {
    assert_compile_fails(
        "## Main\nShow \"Hello {name\".",
        "Unclosed interpolation brace",
    );
}

#[test]
fn e2e_interpolation_unclosed_brace_error_interp() {
    assert_interpreter_fails(
        "## Main\nShow \"Hello {name\".",
        "Unclosed interpolation brace",
    );
}

// =============================================================================
// C Codegen — Currency with Integer
// =============================================================================

#[test]
fn e2e_c_format_currency_integer() {
    assert_c_output(
        r#"## Main
Let price be 42.
Show "{price:$}"."#,
        "$42.00",
    );
}

// =============================================================================
// Edge Case: Interpolation Used in Function Argument
// =============================================================================

#[test]
fn e2e_interpolation_in_function_body() {
    assert_exact_output(
        r#"## To greet (name: Text) -> Text:
    Let msg be "Hello, {name}!".
    Return msg.

## Main
Let result be greet("Bob").
Show result."#,
        "Hello, Bob!",
    );
}

#[test]
fn e2e_interpolation_in_function_body_interp() {
    assert_interpreter_output(
        r#"## To greet (name: Text) -> Text:
    Let msg be "Hello, {name}!".
    Return msg.

## Main
Let result be greet("Bob").
Show result."#,
        "Hello, Bob!",
    );
}

// =============================================================================
// Edge Case: Debug Format with Underscore Variable Name
// =============================================================================

#[test]
fn e2e_format_debug_underscore_name() {
    assert_exact_output(
        r#"## Main
Let my_var be 99.
Show "{my_var=}"."#,
        "my_var=99",
    );
}

#[test]
fn e2e_format_debug_underscore_name_interp() {
    assert_interpreter_output(
        r#"## Main
Let my_var be 99.
Show "{my_var=}"."#,
        "my_var=99",
    );
}

// =============================================================================
// Edge Case: Mixed Format Specifiers in One String
// =============================================================================

#[test]
fn e2e_format_mixed_specifiers() {
    assert_exact_output(
        r#"## Main
Let pi be 3.14159.
Let price be 19.99.
Show "pi={pi:.2} cost={price:$}"."#,
        "pi=3.14 cost=$19.99",
    );
}

#[test]
fn e2e_format_mixed_specifiers_interp() {
    assert_interpreter_output(
        r#"## Main
Let pi be 3.14159.
Let price be 19.99.
Show "pi={pi:.2} cost={price:$}"."#,
        "pi=3.14 cost=$19.99",
    );
}
