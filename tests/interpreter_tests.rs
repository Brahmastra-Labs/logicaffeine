//! Interpreter tests for the guide code runner
//!
//! Tests the interpret_for_ui function which is used by the web guide.

#[cfg(not(target_arch = "wasm32"))]
use logos::interpret_for_ui;

#[cfg(not(target_arch = "wasm32"))]
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    futures::executor::block_on(f)
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn interpret_set_operations() {
    // Original guide example - "both" and "either" are now contextual keywords
    // that can be used as variable names
    let code = r#"## Main
Let a be a new Set of Int.
Let b be a new Set of Int.

Add 1 to a. Add 2 to a. Add 3 to a.
Add 2 to b. Add 3 to b. Add 4 to b.

Let both be a intersection b.
Let either be a union b.
Show "Intersection: " + both.
Show "Union: " + either."#;

    let result = block_on(interpret_for_ui(code));

    if let Some(err) = &result.error {
        panic!("Interpreter error: {}", err);
    }

    assert!(!result.lines.is_empty(), "Should have output");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn interpret_contextual_keywords_as_variables() {
    // Test that contextual keywords (both, either, combined, shared) can be used as variables
    let code = r#"## Main
Let both be 1.
Let either be 2.
Let combined be 3.
Let shared be 4.
Show both + either + combined + shared."#;

    let result = block_on(interpret_for_ui(code));

    if let Some(err) = &result.error {
        panic!("Interpreter error: {}", err);
    }

    assert_eq!(result.lines, vec!["10"], "1+2+3+4 should equal 10");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn interpret_simple_set_intersection() {
    // Simpler test to isolate the issue
    let code = r#"## Main
Let x be a new Set of Int.
Let y be a new Set of Int.
Add 1 to x.
Add 1 to y.
Let z be x intersection y.
Show length of z."#;

    let result = block_on(interpret_for_ui(code));

    if let Some(err) = &result.error {
        panic!("Interpreter error: {}", err);
    }

    assert_eq!(result.lines, vec!["1"], "Intersection of {{1}} and {{1}} should be {{1}}");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn interpret_simple_set_union() {
    let code = r#"## Main
Let x be a new Set of Int.
Let y be a new Set of Int.
Add 1 to x.
Add 2 to y.
Let z be x union y.
Show length of z."#;

    let result = block_on(interpret_for_ui(code));

    if let Some(err) = &result.error {
        panic!("Interpreter error: {}", err);
    }

    assert_eq!(result.lines, vec!["2"], "Union of {{1}} and {{2}} should have length 2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn interpret_multiple_statements_per_line() {
    // Test whether multiple statements on a single line works
    let code = r#"## Main
Let x be a new Set of Int.
Add 1 to x. Add 2 to x. Add 3 to x.
Show length of x."#;

    let result = block_on(interpret_for_ui(code));

    if let Some(err) = &result.error {
        panic!("Interpreter error: {}", err);
    }

    assert_eq!(result.lines, vec!["3"], "Set should have 3 elements");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn interpret_show_to_verb_function() {
    // Test "Show X to display" where "display" is a verb in the lexicon
    // This was a bug: lexer returned TokenType::To instead of Preposition("to")
    // causing "expected ':'" parse errors
    let code = r#"## To display (data: Text):
    Show "Displaying: " + data.

## Main
Let profile be "User Profile Data".
Show profile to display.
Show profile."#;

    let result = block_on(interpret_for_ui(code));

    // The key test: parsing should succeed (no parse error)
    if let Some(err) = &result.error {
        panic!("Parse/Interpreter error: {}", err);
    }

    // Verify at least the final Show executed
    assert!(!result.lines.is_empty(), "Should have some output");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn interpret_give_to_verb_function() {
    // Test "Give X to consume" where "consume" is a verb in the lexicon
    // This was a bug: lexer returned TokenType::To instead of Preposition("to")
    // causing "expected 'to'" parse errors
    let code = r#"## To consume (data: Text):
    Show "Consumed: " + data.

## Main
Let message be "Important data".
Give message to consume.
Show "Message was transferred"."#;

    let result = block_on(interpret_for_ui(code));

    // The key test: parsing should succeed (no parse error)
    if let Some(err) = &result.error {
        panic!("Parse/Interpreter error: {}", err);
    }

    // Verify at least the final Show executed
    assert!(!result.lines.is_empty(), "Should have some output");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn interpret_give_copy_to_verb_function() {
    // Test "Give duplicate to process" where "process" is a verb in the lexicon
    // This was a bug: lexer returned TokenType::To instead of Preposition("to")
    // causing "expected 'to'" parse errors
    let code = r#"## To process (data: Text):
    Show "Processing: " + data.

## Main
Let original be "Keep this".
Let duplicate be copy of original.
Give duplicate to process.
Show "Original still here: " + original."#;

    let result = block_on(interpret_for_ui(code));

    // The key test: parsing should succeed (no parse error)
    if let Some(err) = &result.error {
        panic!("Parse/Interpreter error: {}", err);
    }

    // Verify at least the final Show executed
    assert!(!result.lines.is_empty(), "Should have some output");
}
