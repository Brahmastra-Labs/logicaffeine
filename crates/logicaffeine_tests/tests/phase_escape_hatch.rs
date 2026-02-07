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

// ─────────────────────────────────────────────────────────
// Tier 5: Expression-Level Escape — Codegen
// ─────────────────────────────────────────────────────────

#[test]
fn escape_expr_codegen_let_with_type_annotation() {
    let source = r#"## Main
Let x: Int be Escape to Rust:
    42_i64
Show x.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("let x: i64"), "Should have typed let binding:\n{}", rust);
    assert!(rust.contains("42_i64"), "Should contain raw code:\n{}", rust);
    assert!(rust.contains("{"), "Should have block expression:\n{}", rust);
}

#[test]
fn escape_expr_codegen_multiline_block() {
    let source = r#"## Main
Let x: Int be Escape to Rust:
    let a = 10_i64;
    let b = 32_i64;
    a + b
Show x.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("let a = 10_i64;"), "Line 1:\n{}", rust);
    assert!(rust.contains("let b = 32_i64;"), "Line 2:\n{}", rust);
    assert!(rust.contains("a + b"), "Line 3:\n{}", rust);
}

#[test]
fn escape_expr_codegen_in_set_statement() {
    let source = r#"## Main
Let mut x: Int be 0.
Set x to Escape to Rust:
    42 * 2
Show x.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("42 * 2"), "Should contain escape code:\n{}", rust);
    assert!(rust.contains("x ="), "Should have assignment:\n{}", rust);
}

#[test]
fn escape_expr_codegen_coexists_with_statement_escape() {
    let source = r#"## Main
Escape to Rust:
    println!("side-effect");
Let y: Int be Escape to Rust:
    99_i64
Show y.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains(r#"println!("side-effect")"#), "Statement escape:\n{}", rust);
    assert!(rust.contains("99_i64"), "Expression escape:\n{}", rust);
}

// ─────────────────────────────────────────────────────────
// Tier 5b: Expression-Level Escape — E2E
// ─────────────────────────────────────────────────────────

#[test]
fn e2e_escape_expr_basic_int() {
    assert_exact_output(
        r#"## Main
Let x: Int be Escape to Rust:
    42_i64
Show x.
"#,
        "42",
    );
}

#[test]
fn e2e_escape_expr_basic_text() {
    assert_exact_output(
        r#"## Main
Let msg: Text be Escape to Rust:
    format!("hello {}", "world")
Show msg.
"#,
        "hello world",
    );
}

#[test]
fn e2e_escape_expr_basic_bool() {
    assert_exact_output(
        r#"## Main
Let flag: Bool be Escape to Rust:
    10 > 5
Show flag.
"#,
        "true",
    );
}

#[test]
fn e2e_escape_expr_basic_real() {
    let result = run_logos(
        r#"## Main
Let pi: Real be Escape to Rust:
    std::f64::consts::PI
Show pi.
"#,
    );
    assert!(result.success, "Should run.\nstderr: {}\nGenerated Rust:\n{}", result.stderr, result.rust_code);
    assert!(
        result.stdout.trim().starts_with("3.14159"),
        "Expected pi to start with 3.14159, got: {}", result.stdout.trim()
    );
}

#[test]
fn e2e_escape_expr_multi_step_computation() {
    assert_exact_output(
        r#"## Main
Let answer: Int be Escape to Rust:
    let a = 10_i64;
    let b = 32_i64;
    a + b
Show answer.
"#,
        "42",
    );
}

#[test]
fn e2e_escape_expr_reads_logos_variables() {
    assert_exact_output(
        r#"## Main
Let items be [10, 20, 30].
Let total: Int be Escape to Rust:
    items.iter().sum::<i64>()
Show total.
"#,
        "60",
    );
}

// ─────────────────────────────────────────────────────────
// Tier 5c: Expression-Level Escape — Real-World Computations
// ─────────────────────────────────────────────────────────

#[test]
fn e2e_escape_expr_integer_square_root() {
    assert_output_lines(
        r#"## To isqrt (n: Int) -> Int:
    Let result: Int be Escape to Rust:
        let mut x = n;
        if x < 0 { x = 0; }
        if x <= 1 { return x; }
        let mut r = x;
        loop {
            let next = (r + x / r) / 2;
            if next >= r { break; }
            r = next;
        }
        r
    Return result.

## Main
Show isqrt(625).
Show isqrt(2).
"#,
        &["25", "1"],
    );
}

#[test]
fn e2e_escape_expr_gcd_euclidean() {
    assert_exact_output(
        r#"## To gcd (a: Int, b: Int) -> Int:
    Let result: Int be Escape to Rust:
        let mut x = a;
        let mut y = b;
        while y != 0 {
            let temp = y;
            y = x % y;
            x = temp;
        }
        x
    Return result.

## Main
Show gcd(48, 18).
"#,
        "6",
    );
}

#[test]
fn e2e_escape_expr_sieve_prime_count() {
    assert_exact_output(
        r#"## To count_primes (limit: Int) -> Int:
    Let result: Int be Escape to Rust:
        let n = limit as usize;
        if n < 2 { return 0; }
        let mut sieve = vec![true; n + 1];
        sieve[0] = false;
        sieve[1] = false;
        let mut i = 2;
        while i * i <= n {
            if sieve[i] {
                let mut j = i * i;
                while j <= n {
                    sieve[j] = false;
                    j += i;
                }
            }
            i += 1;
        }
        sieve.iter().filter(|&&x| x).count() as i64
    Return result.

## Main
Show count_primes(30).
"#,
        "10",
    );
}

#[test]
fn e2e_escape_expr_binary_search() {
    assert_exact_output(
        r#"## To bsearch (haystack: Seq of Int, target: Int) -> Int:
    Let result: Int be Escape to Rust:
        let mut lo: usize = 0;
        let mut hi: usize = haystack.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if haystack[mid] == target {
                return (mid as i64) + 1;
            } else if haystack[mid] < target {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        0_i64
    Return result.

## Main
Show bsearch([10, 20, 30, 40, 50], 30).
"#,
        "3",
    );
}

#[test]
fn e2e_escape_expr_fnv1a_hash() {
    assert_exact_output(
        r#"## To fnv_hash (data: Text) -> Int:
    Let result: Int be Escape to Rust:
        let mut hash: u64 = 14695981039346656037;
        for byte in data.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(1099511628211);
        }
        hash as i64
    Return result.

## Main
Show fnv_hash("hello").
"#,
        "-6615550055289275125",
    );
}

#[test]
fn e2e_escape_expr_leibniz_pi() {
    let result = run_logos(
        r#"## To approx_pi (iterations: Int) -> Real:
    Let result: Real be Escape to Rust:
        let mut sum = 0.0_f64;
        for k in 0..iterations {
            let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
            sum += sign / (2 * k + 1) as f64;
        }
        sum * 4.0
    Return result.

## Main
Escape to Rust:
    let pi = approx_pi(10000);
    println!("{:.4}", pi);
"#,
    );
    assert!(result.success, "Should run.\nstderr: {}\nGenerated Rust:\n{}", result.stderr, result.rust_code);
    assert!(
        result.stdout.trim().starts_with("3.1415"),
        "Expected pi approximation starting with 3.1415, got: {}", result.stdout.trim()
    );
}

// ─────────────────────────────────────────────────────────
// Tier 5d: Expression-Level Escape — Composability
// ─────────────────────────────────────────────────────────

#[test]
fn e2e_escape_expr_in_set_statement() {
    assert_exact_output(
        r#"## Main
Let mut x: Int be 0.
Set x to Escape to Rust:
    42 * 2
Show x.
"#,
        "84",
    );
}

#[test]
fn e2e_escape_expr_as_function_return() {
    assert_exact_output(
        r#"## To compute () -> Int:
    Let result: Int be Escape to Rust:
        (7 + 3) * 5_i64
    Return result.

## Main
Show compute().
"#,
        "50",
    );
}

#[test]
fn e2e_escape_expr_in_if_branch() {
    assert_exact_output(
        r#"## Main
Let x be 10.
If x > 5:
    Let msg: Text be Escape to Rust:
        format!("big: {}", x)
    Show msg.
Otherwise:
    Let msg: Text be Escape to Rust:
        format!("small: {}", x)
    Show msg.
"#,
        "big: 10",
    );
}

#[test]
fn e2e_escape_expr_inside_repeat_loop() {
    assert_output_lines(
        r#"## Main
Repeat for i from 1 to 3:
    Let sq: Int be Escape to Rust:
        i * i
    Show sq.
"#,
        &["1", "4", "9"],
    );
}

#[test]
fn e2e_escape_expr_multiple_sequential() {
    assert_exact_output(
        r#"## Main
Let a: Int be Escape to Rust:
    10_i64
Let b: Int be Escape to Rust:
    a + 20
Let c: Int be Escape to Rust:
    a + b
Show c.
"#,
        "40",
    );
}

#[test]
fn e2e_escape_expr_alongside_statement_escape() {
    assert_output_lines(
        r#"## Main
Escape to Rust:
    println!("side-effect");
Let val: Int be Escape to Rust:
    42_i64
Show val.
"#,
        &["side-effect", "42"],
    );
}

// ─────────────────────────────────────────────────────────
// Tier 5e: Expression-Level Escape — Type Interaction
// ─────────────────────────────────────────────────────────

#[test]
fn e2e_escape_expr_constructs_logos_struct() {
    assert_exact_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p: Point be Escape to Rust:
    Point { x: 10, y: 20, ..Default::default() }
Show p's x + p's y.
"#,
        "30",
    );
}

#[test]
fn e2e_escape_expr_constructs_logos_list() {
    assert_output_lines(
        r#"## Main
Let items: Seq of Int be Escape to Rust:
    vec![1_i64, 2, 3, 4, 5]
Repeat for n in items:
    Show n.
"#,
        &["1", "2", "3", "4", "5"],
    );
}

#[test]
fn e2e_escape_expr_derives_from_logos_struct() {
    assert_exact_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point with x 3 and y 4.
Let dist_sq: Int be Escape to Rust:
    p.x * p.x + p.y * p.y
Show dist_sq.
"#,
        "25",
    );
}

#[test]
fn e2e_escape_expr_string_processing() {
    assert_exact_output(
        r#"## Main
Let msg be "hello world".
Let reversed: Text be Escape to Rust:
    msg.chars().rev().collect::<String>()
Show reversed.
"#,
        "dlrow olleh",
    );
}

// ─────────────────────────────────────────────────────────
// Tier 5f: Expression-Level Escape — Edge Cases
// ─────────────────────────────────────────────────────────

#[test]
fn e2e_escape_expr_single_expression_block() {
    assert_exact_output(
        r#"## Main
Let x: Int be Escape to Rust:
    7_i64
Show x.
"#,
        "7",
    );
}

#[test]
fn e2e_escape_expr_return_exits_enclosing_function() {
    assert_exact_output(
        r#"## To safe_div (a: Int, b: Int) -> Int:
    Let result: Int be Escape to Rust:
        if b == 0 { return -1; }
        a / b
    Return result.

## Main
Show safe_div(10, 0).
"#,
        "-1",
    );
}

#[test]
fn e2e_escape_expr_deeply_nested_rust() {
    assert_exact_output(
        r#"## Main
Let x: Int be Escape to Rust:
    match 42_i64 % 3 {
        0 => {
            if 42 > 10 { 100_i64 } else { 0 }
        }
        _ => -1,
    }
Show x.
"#,
        "100",
    );
}

#[test]
fn e2e_escape_expr_no_trailing_period_needed() {
    assert_exact_output(
        r#"## Main
Let x: Int be Escape to Rust:
    42_i64
Show x.
"#,
        "42",
    );
}

// ─────────────────────────────────────────────────────────
// Tier 5g: Interpreter Rejection
// ─────────────────────────────────────────────────────────

#[test]
fn interpreter_rejects_escape_expressions() {
    let result = run_interpreter(r#"## Main
Let x: Int be Escape to Rust:
    42_i64
Show x.
"#);
    assert!(!result.success, "Interpreter should reject escape expressions");
    assert!(
        result.error.contains("compil") || result.error.contains("Escape")
            || result.error.contains("interpreted"),
        "Error should mention compilation requirement: {}", result.error,
    );
}
