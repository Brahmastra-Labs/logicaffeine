mod common;

use logicaffeine_compile::compile::interpret_program;

// =============================================================================
// Integer Map Keys — the interpreter currently hardcodes HashMap<String, _>
// =============================================================================

#[test]
fn interpreter_map_int_key_set_and_get() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 1 of m to 100.
Show item 1 of m.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "100");
}

#[test]
fn interpreter_map_int_key_multiple_entries() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 1 of m to 10.
Set item 2 of m to 20.
Set item 3 of m to 30.
Show item 1 of m.
Show item 2 of m.
Show item 3 of m.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "10\n20\n30");
}

#[test]
fn interpreter_map_int_key_overwrite() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 5 of m to 100.
Set item 5 of m to 200.
Show item 5 of m.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "200");
}

#[test]
fn interpreter_map_int_key_in_loop() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Let mutable i be 1.
While i is less than 6:
    Set item i of m to i * 10.
    Set i to i + 1.
Show item 1 of m.
Show item 5 of m.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "10\n50");
}

#[test]
fn interpreter_map_int_key_contains() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Text.
Set item 42 of m to "answer".
If m contains 42:
    Show "found".
If m contains 99:
    Show "oops".
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "found");
}

#[test]
fn interpreter_map_int_key_length() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 1 of m to 10.
Set item 2 of m to 20.
Show length of m.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "2");
}

#[test]
fn interpreter_map_int_key_collect_pattern() {
    let source = r#"## To benchmark (n: Int) -> Int:
    Let mutable m be a new Map of Int to Int.
    Let mutable i be 0.
    While i is less than n:
        Set item i of m to i * 2.
        Set i to i + 1.
    Let mutable found be 0.
    Set i to 0.
    While i is less than n:
        If item i of m equals i * 2:
            Set found to found + 1.
        Set i to i + 1.
    Return found.

## Main
Show benchmark(100).
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "100");
}

// =============================================================================
// Bool Map Keys
// =============================================================================

#[test]
fn interpreter_map_bool_key() {
    let source = r#"## Main
Let mutable m be a new Map of Bool to Text.
Set item true of m to "yes".
Set item false of m to "no".
Show item true of m.
Show item false of m.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "yes\nno");
}

// =============================================================================
// Int-to-Text Map Keys
// =============================================================================

#[test]
fn interpreter_map_int_key_to_text_values() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Text.
Set item 1 of m to "one".
Set item 2 of m to "two".
Show item 1 of m.
Show item 2 of m.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "one\ntwo");
}

// =============================================================================
// Text Map Keys (regression — must still work)
// =============================================================================

#[test]
fn interpreter_map_text_key_still_works() {
    let source = r#"## Main
Let mutable m be a new Map of Text to Int.
Set item "hello" of m to 1.
Set item "world" of m to 2.
Show item "hello" of m.
Show item "world" of m.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "1\n2");
}

#[test]
fn interpreter_map_text_key_contains_still_works() {
    let source = r#"## Main
Let mutable m be a new Map of Text to Int.
Set item "key" of m to 42.
If m contains "key":
    Show "found".
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "found");
}

// =============================================================================
// Map iteration with non-String keys
// =============================================================================

#[test]
fn interpreter_map_int_key_for_in() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Text.
Set item 1 of m to "one".
Set item 2 of m to "two".
Let mutable count be 0.
Repeat for entry in m:
    Set count to count + 1.
Show count.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "2");
}
