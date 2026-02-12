mod common;

use logicaffeine_compile::compile::interpret_program;

#[test]
fn interpret_hello_world() {
    let source = r#"## Main

Show "Hello, world!".
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "Hello, world!");
}

#[test]
fn interpret_arithmetic() {
    let source = r#"## Main

Let x be 5.
Let y be 10.
Show x + y.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "15");
}

#[test]
fn interpret_function_call() {
    let source = r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main

Show double(3).
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "6");
}

#[test]
fn interpret_conditional() {
    let source = r#"## Main

Let x be 10.
If x > 5:
    Show "big".
Otherwise:
    Show "small".
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "big");
}

#[test]
fn interpret_loop() {
    let source = r#"## Main

Let items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.

Let total be 0.
Repeat for n in items:
    Set total to total + n.
Show total.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "6");
}

#[test]
fn interpret_list_operations() {
    let source = r#"## Main

Let items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Show items.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "[10, 20, 30]");
}

#[test]
fn interpret_struct() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## Main

Let p be a new Point with x 3 and y 4.
Show p's x.
Show p's y.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "3\n4");
}

#[test]
fn interpret_multi_line_output() {
    let source = r#"## Main

Show "line1".
Show "line2".
Show "line3".
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret without error: {:?}", result);
    assert_eq!(result.unwrap().trim(), "line1\nline2\nline3");
}

#[test]
fn interpret_parse_error_returns_err() {
    let source = r#"## Main

This is not valid LOGOS code ???
"#;
    let result = interpret_program(source);
    assert!(result.is_err(), "Should return error for invalid code");
}

#[test]
fn interpret_string_index_basic() {
    let source = "## Main\nLet s be \"hello\".\nShow item 2 of s.";
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should index strings: {:?}", result);
    assert_eq!(result.unwrap().trim(), "e");
}

#[test]
fn interpret_string_index_utf8() {
    // "éé" has 2 chars but 4 bytes.
    // Indexing char 3 passes the byte-length check (3 <= 4) but chars().nth(2) is None → panic.
    // After fix: should return out-of-bounds error.
    let source = "## Main\nLet s be \"éé\".\nShow item 3 of s.";
    let result = interpret_program(source);
    assert!(result.is_err(), "Should return error for out-of-bounds char index on multi-byte string: {:?}", result);
}

#[test]
fn interpret_recursion() {
    let source = r#"## To factorial (n: Int) -> Int:
    If n <= 1:
        Return 1.
    Return n * factorial(n - 1).

## Main

Show factorial(5).
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret recursion: {:?}", result);
    assert_eq!(result.unwrap().trim(), "120");
}

#[test]
fn interpret_float_arithmetic() {
    let source = r#"## Main

Let x be 3.14.
Let y be 2.0.
Show x + y.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret float arithmetic: {:?}", result);
    assert_eq!(result.unwrap().trim(), "5.14");
}

#[test]
fn interpret_nested_if_else() {
    let source = r#"## Main

Let x be 15.
If x > 20:
    Show "big".
Otherwise:
    If x > 10:
        Show "medium".
    Otherwise:
        Show "small".
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret nested if-else: {:?}", result);
    assert_eq!(result.unwrap().trim(), "medium");
}

#[test]
fn interpret_empty_list_iteration() {
    let source = r#"## Main

Let items be a new Seq of Int.
Repeat for n in items:
    Show n.
Show "done".
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should iterate empty list: {:?}", result);
    assert_eq!(result.unwrap().trim(), "done");
}

#[test]
fn interpret_set_field() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## Main

Let mut p be a new Point with x 1 and y 2.
Set p's x to 10.
Show p's x.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should set field: {:?}", result);
    assert_eq!(result.unwrap().trim(), "10");
}

#[test]
fn interpret_string_concat() {
    let source = r#"## Main

Let a be "Hello".
Let b be " World".
Show a combined with b.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should concat strings: {:?}", result);
    assert_eq!(result.unwrap().trim(), "Hello World");
}

#[test]
fn interpret_negative_numbers() {
    let source = r#"## Main

Let x be 0 - 5.
Show x.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should handle negative: {:?}", result);
    assert_eq!(result.unwrap().trim(), "-5");
}

#[test]
fn interpret_while_loop() {
    let source = r#"## Main

Let mut i be 3.
Let mut sum be 0.
While i > 0:
    Set sum to sum + i.
    Set i to i - 1.
Show sum.
"#;
    let result = interpret_program(source);
    assert!(result.is_ok(), "Should interpret while: {:?}", result);
    assert_eq!(result.unwrap().trim(), "6");
}

#[test]
fn interpret_matches_compiled_output() {
    let source = r#"## Main

Let x be 42.
Let y be x * 2.
Show y.
"#;
    let interp_result = interpret_program(source);
    assert!(interp_result.is_ok(), "Should interpret without error: {:?}", interp_result);
    assert_eq!(interp_result.unwrap().trim(), "84");
}
