mod common;
use common::*;

// ─────────────────────────────────────────────────────────
// Tier 1: Codegen — verify generated Rust contains raw code
// ─────────────────────────────────────────────────────────

#[test]
fn escape_block_parses_and_generates_rust() {
    let source = r#"## Main
Escape to Rust:
    println!("hello from rust");
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains(r#"println!("hello from rust");"#), "Generated:\n{}", rust);
}

#[test]
fn escape_block_multiline_preserves_all_lines() {
    let source = r#"## Main
Escape to Rust:
    let x = 42;
    let y = x * 2;
    println!("{}", y);
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("let x = 42;"), "Line 1:\n{}", rust);
    assert!(rust.contains("let y = x * 2;"), "Line 2:\n{}", rust);
    assert!(rust.contains(r#"println!("{}", y);"#), "Line 3:\n{}", rust);
}

#[test]
fn escape_block_surrounded_by_logos_code() {
    let source = r#"## Main
Let a be 5.
Escape to Rust:
    let b = a + 10;
Let c be 20.
Show c.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("let a = 5"), "Before escape:\n{}", rust);
    assert!(rust.contains("let b = a + 10;"), "Inside escape:\n{}", rust);
    assert!(rust.contains("let c = 20"), "After escape:\n{}", rust);
}

#[test]
fn escape_block_in_function_body() {
    let source = r#"## To greet (name: Text):
    Escape to Rust:
        println!("Hello, {}!", name);

## Main
Call greet with "World".
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn greet"), "Has function:\n{}", rust);
    assert!(rust.contains(r#"println!("Hello, {}!", name)"#), "Escape in body:\n{}", rust);
}

#[test]
fn escape_block_preserves_rust_generics_and_closures() {
    let source = r#"## Main
Escape to Rust:
    let v: Vec<i32> = vec![1, 2, 3];
    let sum: i64 = v.iter().map(|x| *x as i64).sum();
    println!("{}", sum);
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Vec<i32>"), "Generics:\n{}", rust);
    assert!(rust.contains("vec![1, 2, 3]"), "Macros:\n{}", rust);
    assert!(rust.contains(".iter().map("), "Chains:\n{}", rust);
}

#[test]
fn escape_block_with_use_statement() {
    let source = r#"## Main
Escape to Rust:
    use std::collections::HashMap;
    let mut map = HashMap::new();
    map.insert("key", "value");
    println!("{:?}", map);
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("use std::collections::HashMap;"), "Use:\n{}", rust);
}

#[test]
fn escape_block_with_rust_control_flow() {
    let source = r#"## Main
Escape to Rust:
    for i in 0..5 {
        if i % 2 == 0 {
            println!("{} is even", i);
        }
    }
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("for i in 0..5"), "For loop:\n{}", rust);
    assert!(rust.contains("i % 2 == 0"), "Condition:\n{}", rust);
}

#[test]
fn escape_block_wrapped_in_braces_for_hygiene() {
    let source = r#"## Main
Let x be 5.
Escape to Rust:
    let y = 1;
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    let x_pos = rust.find("let x = 5").unwrap();
    let y_pos = rust.find("let y = 1;").unwrap();
    let between = &rust[x_pos..y_pos];
    assert!(between.contains("{"), "Escape block wrapped in braces:\n{}", rust);
}

// ─────────────────────────────────────────────────────────
// Tier 2: Error handling — verify proper rejection
// ─────────────────────────────────────────────────────────

#[test]
fn escape_rejects_unsupported_language() {
    let source = r#"## Main
Escape to Python:
    print("hello")
"#;
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should reject unsupported escape target");
}

#[test]
fn escape_missing_language_produces_error() {
    let source = r#"## Main
Escape:
    println!("hello");
"#;
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should fail without language target");
}

// ─────────────────────────────────────────────────────────
// Tier 3: E2E — full compile + run + output verification
// ─────────────────────────────────────────────────────────

#[test]
fn e2e_escape_basic_println() {
    assert_exact_output(
        r#"## Main
Escape to Rust:
    println!("escape works");
"#,
        "escape works",
    );
}

#[test]
fn e2e_escape_accesses_logos_int() {
    assert_exact_output(
        r#"## Main
Let x be 42.
Escape to Rust:
    println!("x is {}", x);
"#,
        "x is 42",
    );
}

#[test]
fn e2e_escape_accesses_logos_string() {
    assert_exact_output(
        r#"## Main
Let name be "Alice".
Escape to Rust:
    println!("Hello, {}!", name);
"#,
        "Hello, Alice!",
    );
}

#[test]
fn e2e_escape_accesses_logos_list() {
    assert_exact_output(
        r#"## Main
Let items be [10, 20, 30].
Escape to Rust:
    let sum: i64 = items.iter().sum();
    println!("{}", sum);
"#,
        "60",
    );
}

#[test]
fn e2e_escape_with_logos_before_and_after() {
    assert_output_lines(
        r#"## Main
Show 5.
Escape to Rust:
    println!("middle");
Show 10.
"#,
        &["5", "middle", "10"],
    );
}

#[test]
fn e2e_escape_in_function_with_param() {
    assert_exact_output(
        r#"## To double (n: Int) -> Int:
    Escape to Rust:
        return n * 2;

## Main
Let x be double(21).
Show x.
"#,
        "42",
    );
}

#[test]
fn e2e_escape_modifies_mutable_variable() {
    assert_exact_output(
        r#"## Main
Let mut count be 0.
Escape to Rust:
    count = 100;
Show count.
"#,
        "100",
    );
}

#[test]
fn e2e_escape_multiple_blocks_share_via_logos_vars() {
    assert_exact_output(
        r#"## Main
Let mut x be 1.
Escape to Rust:
    x = x + 10;
Escape to Rust:
    x = x * 2;
Show x.
"#,
        "22",
    );
}

#[test]
fn e2e_escape_std_collections() {
    assert_output(
        r#"## Main
Escape to Rust:
    use std::collections::BTreeSet;
    let mut set = BTreeSet::new();
    set.insert(3);
    set.insert(1);
    set.insert(2);
    for v in &set {
        println!("{}", v);
    }
"#,
        "1\n2\n3",
    );
}

#[test]
fn e2e_escape_in_if_branch() {
    assert_exact_output(
        r#"## Main
Let x be 5.
If x > 0:
    Escape to Rust:
        println!("positive");
Otherwise:
    Escape to Rust:
        println!("non-positive");
"#,
        "positive",
    );
}

#[test]
fn e2e_escape_fibonacci_full_function() {
    assert_exact_output(
        r#"## To fibonacci (n: Int) -> Int:
    Escape to Rust:
        if n <= 1 {
            return n;
        }
        let mut a: i64 = 0;
        let mut b: i64 = 1;
        for _ in 2..=n {
            let temp = b;
            b = a + b;
            a = temp;
        }
        return b;

## Main
Show fibonacci(10).
"#,
        "55",
    );
}

// ─────────────────────────────────────────────────────────
// Tier 3b: Logos↔Rust type interaction tests
// ─────────────────────────────────────────────────────────

#[test]
fn e2e_escape_accesses_logos_struct() {
    assert_exact_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point with x 10 and y 20.
Escape to Rust:
    println!("{}", p.x + p.y);
"#,
        "30",
    );
}

#[test]
fn e2e_escape_inside_repeat_loop() {
    assert_output_lines(
        r#"## Main
Let items be [10, 20, 30].
Repeat for n in items:
    Escape to Rust:
        println!("item={}", n);
"#,
        &["item=10", "item=20", "item=30"],
    );
}

#[test]
fn e2e_escape_accesses_logos_map() {
    assert_exact_output(
        r#"## Main
Let mut scores be a new Map of Text to Int.
Set item "alice" of scores to 100.
Set item "bob" of scores to 200.
Escape to Rust:
    let total: i64 = scores.values().sum();
    println!("{}", total);
"#,
        "300",
    );
}

#[test]
fn escape_empty_body_produces_error() {
    let source = "## Main\nEscape to Rust:\n";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Empty escape body should produce an error");
}

#[test]
fn e2e_escape_return_value_in_function() {
    assert_exact_output(
        r#"## To square (n: Int) -> Int:
    Escape to Rust:
        return n * n;

## Main
Show square(7).
"#,
        "49",
    );
}

#[test]
fn e2e_escape_after_give_caught_by_rustc() {
    let source = r#"## To consume (data: Seq of Int):
    Show length of data.

## Main
Let items be [1, 2, 3].
Call consume with Give items.
Escape to Rust:
    println!("{}", items.len());
"#;
    let result = compile_logos(source);
    assert!(
        !result.success,
        "Should fail: items was moved by Give, so rustc should reject use in escape.\nstderr: {}",
        result.stderr
    );
}

#[test]
fn e2e_escape_in_helper_function_multi_block() {
    assert_output_lines(
        r#"## To report (n: Int):
    Escape to Rust:
        println!("half={}", n / 2);
    Escape to Rust:
        println!("double={}", n * 2);

## Main
Call report with 10.
"#,
        &["half=5", "double=20"],
    );
}

#[test]
fn e2e_escape_deeply_nested_rust() {
    assert_output_lines(
        r#"## Main
Escape to Rust:
    let values = vec![1, 2, 3, 4, 5, 6];
    for &v in &values {
        match v % 3 {
            0 => {
                if v > 3 {
                    println!("big-triple");
                } else {
                    println!("triple");
                }
            }
            _ => {}
        }
    }
"#,
        &["triple", "big-triple"],
    );
}

// ─────────────────────────────────────────────────────────
// Tier 3c: Edge cases and robustness
// ─────────────────────────────────────────────────────────

#[test]
fn e2e_escape_adjacent_blocks() {
    assert_output_lines(
        r#"## Main
Escape to Rust:
    println!("first");
Escape to Rust:
    println!("second");
"#,
        &["first", "second"],
    );
}

#[test]
fn e2e_escape_accesses_logos_bool() {
    assert_exact_output(
        r#"## Main
Let flag be true.
Escape to Rust:
    if flag {
        println!("yes");
    } else {
        println!("no");
    }
"#,
        "yes",
    );
}

#[test]
fn e2e_escape_in_otherwise_branch() {
    assert_exact_output(
        r#"## Main
Let x be 0.
If x > 0:
    Show 999.
Otherwise:
    Escape to Rust:
        println!("took else");
"#,
        "took else",
    );
}

#[test]
fn e2e_escape_rust_comments_only() {
    assert_exact_output(
        r#"## Main
Escape to Rust:
    // This is a Rust comment
    /* block comment */
Show 42.
"#,
        "42",
    );
}

#[test]
fn e2e_escape_string_containing_escape_syntax() {
    assert_exact_output(
        r#"## Main
Escape to Rust:
    let s = "Escape to Rust:";
    println!("{}", s);
"#,
        "Escape to Rust:",
    );
}

// ─────────────────────────────────────────────────────────
// Tier 4: Interpreter — verify proper error message
// ─────────────────────────────────────────────────────────

#[test]
fn interpreter_rejects_escape_blocks() {
    let result = run_interpreter(r#"## Main
Escape to Rust:
    println!("hello");
"#);
    assert!(!result.success, "Interpreter should reject escape blocks");
    assert!(
        result.error.contains("compil") || result.error.contains("Escape")
            || result.error.contains("interpreted"),
        "Error should mention compilation requirement: {}", result.error,
    );
}
