//! E2E Tests: Control Flow
//!
//! Tests that If/Otherwise and While statements work correctly.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_output;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_if_true_branch() {
    assert_output(
        r#"## Main
Let x be 10.
If x is greater than 5:
    Show "big".
"#,
        "big",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_if_otherwise() {
    assert_output(
        r#"## Main
Let x be 3.
If x is greater than 5:
    Show "big".
Otherwise:
    Show "small".
"#,
        "small",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_while_loop_sum() {
    assert_output(
        r#"## Main
Let i be 1.
Let sum be 0.
While i is at most 5:
    Set sum to sum + i.
    Set i to i + 1.
Show sum.
"#,
        "15",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_compound_condition_and() {
    assert_output(
        r#"## Main
Let a be 3.
Let b be 4.
If a is less than 5 and b is less than 5:
    Show "both small".
"#,
        "both small",
    );
}

// === NEW TESTS ===

#[cfg(not(target_arch = "wasm32"))]
use common::assert_runs;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_nested_if() {
    assert_output(
        r#"## Main
Let x be 15.
If x is greater than 5:
    If x is greater than 10:
        Show "very big".
"#,
        "very big",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_if_in_while() {
    assert_output(
        r#"## Main
Let i be 1.
Let found be 0.
While i is at most 10:
    If i equals 5:
        Set found to i.
    Set i to i + 1.
Show found.
"#,
        "5",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_while_in_if() {
    assert_output(
        r#"## Main
Let should_count be true.
Let sum be 0.
If should_count:
    Let i be 1.
    While i is at most 3:
        Set sum to sum + i.
        Set i to i + 1.
Show sum.
"#,
        "6",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_multiple_sequential_if() {
    assert_output(
        r#"## Main
Let x be 5.
Let result be 0.
If x is greater than 0:
    Set result to result + 1.
If x is greater than 3:
    Set result to result + 10.
If x is greater than 10:
    Set result to result + 100.
Show result.
"#,
        "11",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_while_zero_iterations() {
    // While condition false from start - body never executes
    assert_output(
        r#"## Main
Let x be 10.
While x is less than 5:
    Set x to x + 1.
Show x.
"#,
        "10",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_while_single_iteration() {
    assert_output(
        r#"## Main
Let x be 0.
While x is less than 1:
    Set x to x + 1.
Show x.
"#,
        "1",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_condition_with_expr() {
    assert_output(
        r#"## Main
Let x be 5.
Let y be 6.
If (x + y) is greater than 10:
    Show "sum is big".
"#,
        "sum is big",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_deeply_nested() {
    assert_output(
        r#"## Main
Let a be 1.
Let b be 2.
Let c be 3.
If a is less than 5:
    If b is less than 5:
        If c is less than 5:
            Show "all small".
"#,
        "all small",
    );
}

// Phase 30c: Else as alias for Otherwise
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_if_else() {
    assert_output(
        r#"## Main
Let x be 3.
If x is greater than 5:
    Show "big".
Else:
    Show "small".
"#,
        "small",
    );
}

// Phase 30d: Otherwise If / Else If chains
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_otherwise_if_chain() {
    assert_output(
        r#"## Main
Let x be 7.
If x is greater than 10:
    Show "big".
Otherwise If x is greater than 5:
    Show "medium".
Otherwise:
    Show "small".
"#,
        "medium",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_else_if_chain() {
    assert_output(
        r#"## Main
Let x be 3.
If x is greater than 10:
    Show "big".
Else If x is greater than 5:
    Show "medium".
Else:
    Show "small".
"#,
        "small",
    );
}

// Phase 30e: elif shorthand
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_elif_chain() {
    assert_output(
        r#"## Main
Let x be 7.
If x is greater than 10:
    Show "big".
elif x is greater than 5:
    Show "medium".
Else:
    Show "small".
"#,
        "medium",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_elif_multiple() {
    assert_output(
        r#"## Main
Let x be 2.
If x is greater than 10:
    Show "big".
elif x is greater than 5:
    Show "medium".
elif x is greater than 0:
    Show "positive".
Else:
    Show "zero or negative".
"#,
        "positive",
    );
}

// =============================================================================
// Phase 23b: Equals-style assignment syntax
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_equals_assignment() {
    assert_output(
        r#"## Main
greeting = "Hello World".
Show greeting.
"#,
        "Hello World",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_mut_assignment() {
    assert_output(
        r#"## Main
mut count = 0.
Set count to 5.
Show count.
"#,
        "5",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_equals_auto_mutability() {
    assert_output(
        r#"## Main
counter = 0.
Set counter to counter + 1.
Show counter.
"#,
        "1",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_equals_in_control_flow() {
    assert_output(
        r#"## Main
x = 10.
result = 0.
If x is greater than 5:
    Set result to 1.
Show result.
"#,
        "1",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_equals_with_while() {
    assert_output(
        r#"## Main
sum = 0.
i = 1.
While i is at most 5:
    Set sum to sum + i.
    Set i to i + 1.
Show sum.
"#,
        "15",
    );
}
