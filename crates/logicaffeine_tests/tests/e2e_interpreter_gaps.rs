//! E2E Interpreter Tests: Feature Coverage Gaps
//!
//! Mirrors e2e_codegen_gaps.rs through the interpreter pipeline.
//! Escape blocks (Section I) are omitted — they are codegen-only.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_interpreter_output;

// =============================================================================
// A. Float Operations
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_float_literal() {
    assert_interpreter_output("## Main\nShow 3.14.", "3.14");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_float_addition() {
    assert_interpreter_output("## Main\nShow 1.5 + 2.5.", "4");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_float_subtraction() {
    assert_interpreter_output("## Main\nShow 10.5 - 3.2.", "7.3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_float_multiplication() {
    assert_interpreter_output("## Main\nShow 2.5 * 4.0.", "10");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_float_division() {
    assert_interpreter_output("## Main\nShow 7.5 / 2.5.", "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_float_comparison() {
    assert_interpreter_output(
        r#"## Main
Let x be 3.14.
If x is greater than 3.0:
    Show "bigger".
"#,
        "bigger",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_float_in_function() {
    assert_interpreter_output(
        r#"## To double (x: Float) -> Float:
    Return x * 2.0.

## Main
Show double(3.5).
"#,
        "7",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_float_negative() {
    assert_interpreter_output("## Main\nShow 0.0 - 3.14.", "-3.14");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_float_equality() {
    assert_interpreter_output(
        r#"## Main
Let a be 2.5.
Let b be 2.5.
If a equals b:
    Show "same".
"#,
        "same",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_float_typed_variable() {
    assert_interpreter_output(
        r#"## Main
Let x be 2.718.
Show x.
"#,
        "2.718",
    );
}

// =============================================================================
// B. Modulo Operator
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_modulo_basic() {
    assert_interpreter_output("## Main\nShow 10 % 3.", "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_modulo_even_check() {
    assert_interpreter_output(
        r#"## Main
Let x be 4.
If x % 2 equals 0:
    Show "even".
Otherwise:
    Show "odd".
"#,
        "even",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_modulo_in_loop() {
    assert_interpreter_output(
        r#"## Main
Let count be 0.
Repeat for i from 1 to 20:
    If i % 5 equals 0:
        Set count to count + 1.
Show count.
"#,
        "4",
    );
}

// =============================================================================
// C. Option Type
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_option_some() {
    assert_interpreter_output(
        r#"## Main
Let x be some 42.
Show x.
"#,
        "42",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_option_none() {
    assert_interpreter_output(
        r#"## To maybe (n: Int) -> Option of Int:
    If n is greater than 0:
        Return some n.
    Return none.

## Main
Let x be maybe(0 - 1).
Show x.
"#,
        "nothing",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_option_some_in_function() {
    assert_interpreter_output(
        r#"## To find (n: Int) -> Option of Int:
    If n is greater than 0:
        Return some n.
    Return none.

## Main
Let result be find(5).
Show result.
"#,
        "5",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_option_none_in_function() {
    assert_interpreter_output(
        r#"## To find (n: Int) -> Option of Int:
    If n is greater than 0:
        Return some n.
    Return none.

## Main
Let result be find(0 - 1).
Show result.
"#,
        "nothing",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_option_inspect_some() {
    assert_interpreter_output(
        r#"## A Maybe is one of:
    A Nothing.
    A Just with value Int.

## Main
Let x be a new Just with value 42.
Inspect x:
    When Nothing: Show "empty".
    When Just (v): Show v.
"#,
        "42",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_option_inspect_none() {
    assert_interpreter_output(
        r#"## A Maybe is one of:
    A Nothing.
    A Just with value Int.

## Main
Let x be a new Nothing.
Inspect x:
    When Nothing: Show "empty".
    When Just (v): Show v.
"#,
        "empty",
    );
}

// =============================================================================
// D. Nothing/Unit Type
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_nothing_literal() {
    assert_interpreter_output(
        r#"## Main
Let x be nothing.
Show "after".
"#,
        "after",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_void_function() {
    assert_interpreter_output(
        r#"## To doWork (x: Int):
    Let y be x * 2.

## Main
doWork(5).
Show "done".
"#,
        "done",
    );
}

// =============================================================================
// E. Collection Type Combinations
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_seq_of_bool() {
    assert_interpreter_output(
        r#"## Main
Let flags be a new Seq of Bool.
Push true to flags.
Push false to flags.
Push true to flags.
Let count be 0.
Repeat for f in flags:
    If f:
        Set count to count + 1.
Show count.
"#,
        "2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_seq_of_text() {
    assert_interpreter_output(
        r#"## Main
Let words be a new Seq of Text.
Push "hello" to words.
Push "world" to words.
Show length of words.
"#,
        "2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_set_of_text() {
    assert_interpreter_output(
        r#"## Main
Let tags be a new Set of Text.
Add "rust" to tags.
Add "logos" to tags.
Add "rust" to tags.
Show length of tags.
"#,
        "2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_map_text_to_text() {
    assert_interpreter_output(
        r#"## Main
Let mut config be a new Map of Text to Text.
Set item "name" of config to "logos".
Show item "name" of config.
"#,
        "logos",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_map_text_to_bool() {
    assert_interpreter_output(
        r#"## Main
Let mut flags be a new Map of Text to Bool.
Set item "debug" of flags to true.
If item "debug" of flags:
    Show "debug on".
"#,
        "debug on",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_map_int_to_text() {
    assert_interpreter_output(
        r#"## Main
Let mut names be a new Map of Int to Text.
Set item 1 of names to "Alice".
Set item 2 of names to "Bob".
Show item 1 of names.
"#,
        "Alice",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_empty_map_length() {
    assert_interpreter_output(
        r#"## Main
Let m be a new Map of Text to Int.
Show length of m.
"#,
        "0",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_empty_set_length() {
    assert_interpreter_output(
        r#"## Main
Let s be a new Set of Int.
Show length of s.
"#,
        "0",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_map_length_after_mutations() {
    assert_interpreter_output(
        r#"## Main
Let mut m be a new Map of Text to Int.
Set item "a" of m to 1.
Set item "b" of m to 2.
Set item "c" of m to 3.
Show length of m.
"#,
        "3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_seq_of_struct() {
    assert_interpreter_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let points be a new Seq of Point.
Push a new Point with x 1 and y 2 to points.
Push a new Point with x 3 and y 4 to points.
Let sum be 0.
Repeat for p in points:
    Set sum to sum + p's x.
Show sum.
"#,
        "4",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_set_text_iterate() {
    assert_interpreter_output(
        r#"## Main
Let words be a new Set of Text.
Add "a" to words.
Add "b" to words.
Let count be 0.
Repeat for w in words:
    Set count to count + 1.
Show count.
"#,
        "2",
    );
}

// =============================================================================
// F. Struct/Enum Patterns
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_struct_mixed_fields() {
    assert_interpreter_output(
        r#"## A Person has:
    A name: Text.
    An age: Int.
    An active: Bool.

## Main
Let p be a new Person with name "Alice" and age 30 and active true.
Show p's name.
"#,
        "Alice",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_struct_default() {
    assert_interpreter_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point.
Show p's x + p's y.
"#,
        "0",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_enum_three_variants() {
    assert_interpreter_output(
        r#"## A Traffic is one of:
    A Red.
    A Yellow.
    A Green.

## Main
Let light be a new Yellow.
Inspect light:
    When Red: Show "stop".
    When Yellow: Show "caution".
    When Green: Show "go".
"#,
        "caution",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_enum_mixed_unit_payload() {
    assert_interpreter_output(
        r#"## A Result is one of:
    A Success with value Int.
    A Failure.

## Main
Let r be a new Success with value 42.
Inspect r:
    When Success (v): Show v.
    When Failure: Show "failed".
"#,
        "42",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_enum_mixed_failure_branch() {
    assert_interpreter_output(
        r#"## A Result is one of:
    A Success with value Int.
    A Failure.

## Main
Let r be a new Failure.
Inspect r:
    When Success (v): Show v.
    When Failure: Show "failed".
"#,
        "failed",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_struct_copy_independence() {
    assert_interpreter_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let mutable p be a new Point with x 1 and y 2.
Let q be copy of p.
Set p's x to 99.
Show q's x.
"#,
        "1",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_struct_deep_field() {
    assert_interpreter_output(
        r#"## A Inner has:
    A value: Int.

## A Middle has:
    A inner: Inner.

## A Outer has:
    A middle: Middle.

## Main
Let i be a new Inner with value 42.
Let m be a new Middle with inner i.
Let o be a new Outer with middle m.
Show o's middle's inner's value.
"#,
        "42",
    );
}

// =============================================================================
// G. Advanced Control Flow
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_if_no_otherwise() {
    assert_interpreter_output(
        r#"## Main
Let x be 3.
If x is greater than 10:
    Show "big".
Show "done".
"#,
        "done",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_triple_nested_loop() {
    assert_interpreter_output(
        r#"## Main
Let count be 0.
Repeat for i from 1 to 2:
    Repeat for j from 1 to 3:
        Repeat for k from 1 to 4:
            Set count to count + 1.
Show count.
"#,
        "24",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_return_from_while() {
    assert_interpreter_output(
        r#"## To findFirst (items: Seq of Int) -> Int:
    Let i be 1.
    While i is at most length of items:
        If item i of items is greater than 10:
            Return item i of items.
        Set i to i + 1.
    Return 0 - 1.

## Main
Show findFirst([1, 5, 15, 20]).
"#,
        "15",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_while_compound_or() {
    assert_interpreter_output(
        r#"## Main
Let x be 0.
While x is less than 5 or x equals 5:
    Set x to x + 1.
Show x.
"#,
        "6",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_cascading_elif() {
    assert_interpreter_output(
        r#"## Main
Let x be 42.
If x is less than 10:
    Show "small".
Otherwise If x is less than 50:
    Show "medium".
Otherwise If x is less than 100:
    Show "large".
Otherwise:
    Show "huge".
"#,
        "medium",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_inspect_inside_if() {
    assert_interpreter_output(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let flag be true.
Let c be a new Green.
If flag:
    Inspect c:
        When Red: Show "red".
        When Green: Show "green".
        When Blue: Show "blue".
"#,
        "green",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_two_sequential_while() {
    assert_interpreter_output(
        r#"## Main
Let a be 0.
While a is less than 3:
    Set a to a + 1.
Let b be 0.
While b is less than 4:
    Set b to b + 1.
Show a + b.
"#,
        "7",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_range_variable_bounds() {
    assert_interpreter_output(
        r#"## Main
Let lo be 3.
Let hi be 7.
Let sum be 0.
Repeat for i from lo to hi:
    Set sum to sum + i.
Show sum.
"#,
        "25",
    );
}

// =============================================================================
// H. Function Patterns
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_function_returns_struct() {
    assert_interpreter_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## To makePoint (a: Int) and (b: Int) -> Point:
    Return a new Point with x a and y b.

## Main
Let p be makePoint(10, 20).
Show p's x + p's y.
"#,
        "30",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_three_function_chain() {
    assert_interpreter_output(
        r#"## To inc (x: Int) -> Int:
    Return x + 1.

## To double (x: Int) -> Int:
    Return x * 2.

## To square (x: Int) -> Int:
    Return x * x.

## Main
Show square(double(inc(2))).
"#,
        "36",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_function_builds_map() {
    assert_interpreter_output(
        r#"## To makeConfig -> Map of Text to Int:
    Let mut m be a new Map of Text to Int.
    Set item "timeout" of m to 30.
    Set item "retries" of m to 3.
    Return m.

## Main
Let config be makeConfig().
Show item "timeout" of config.
"#,
        "30",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_side_effect_function() {
    assert_interpreter_output(
        r#"## To greet (name: Text):
    Show name.

## Main
greet("Alice").
greet("Bob").
"#,
        "Alice\nBob",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_function_four_params() {
    assert_interpreter_output(
        r#"## To sum4 (a: Int) and (b: Int) and (c: Int) and (d: Int) -> Int:
    Return a + b + c + d.

## Main
Show sum4(1, 2, 3, 4).
"#,
        "10",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_mutual_recursion() {
    assert_interpreter_output(
        r#"## To isEven (n: Int) -> Bool:
    If n equals 0:
        Return true.
    Return isOdd(n - 1).

## To isOdd (n: Int) -> Bool:
    If n equals 0:
        Return false.
    Return isEven(n - 1).

## Main
Show isEven(10).
"#,
        "true",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_function_returns_option() {
    assert_interpreter_output(
        r#"## To safeDivide (a: Int) and (b: Int) -> Option of Int:
    If b equals 0:
        Return none.
    Return some a / b.

## Main
Let result be safeDivide(10, 2).
Show result.
"#,
        "5",
    );
}

// =============================================================================
// J. String Operations
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_string_equals() {
    assert_interpreter_output(
        r#"## Main
Let a be "hello".
Let b be "hello".
If a equals b:
    Show "same".
Otherwise:
    Show "different".
"#,
        "same",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_string_not_equals() {
    assert_interpreter_output(
        r#"## Main
Let a be "hello".
Let b be "world".
If a is not b:
    Show "different".
"#,
        "different",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_string_contains() {
    assert_interpreter_output(
        r#"## Main
Let s be "hello world".
If s contains "world":
    Show "found".
"#,
        "found",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_string_empty_length() {
    assert_interpreter_output(
        r#"## Main
Let s be "".
Show length of s.
"#,
        "0",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_string_combined_with() {
    assert_interpreter_output(
        r#"## Main
Let a be "foo".
Let b be "bar".
Let c be a combined with b.
Show c.
"#,
        "foobar",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_interp_string_in_seq() {
    assert_interpreter_output(
        r#"## Main
Let words be a new Seq of Text.
Push "hello" to words.
Push "world" to words.
Pop from words into last.
Show last.
"#,
        "world",
    );
}
