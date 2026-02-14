//! Phase 38: Standard Library (IO & System)
//!
//! Tests for native function support and standard library modules.

mod common;

use logicaffeine_compile::compile::compile_to_rust;

/// Test that arrow -> is tokenized correctly.
#[test]
fn test_arrow_tokenization() {
    use logicaffeine_language::lexer::Lexer;
    use logicaffeine_base::Interner;
    use logicaffeine_language::token::TokenType;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new("(x: Text) -> Result", &mut interner);
    let tokens = lexer.tokenize();

    // Check that Arrow token is present
    let has_arrow = tokens.iter().any(|t| matches!(t.kind, TokenType::Arrow));
    assert!(has_arrow, "Should tokenize -> as Arrow. Tokens: {:?}", tokens.iter().map(|t| &t.kind).collect::<Vec<_>>());
}

/// Debug test for "now" tokenization
#[test]
fn test_now_tokenization() {
    use logicaffeine_language::lexer::Lexer;
    use logicaffeine_base::Interner;
    use logicaffeine_language::token::TokenType;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new("## To native now -> Nat", &mut interner);
    let tokens = lexer.tokenize();

    // Print all tokens
    for t in &tokens {
        let lexeme = interner.resolve(t.lexeme);
        eprintln!("Token: {:?}, lexeme: {:?}, span: {:?}", t.kind, lexeme, t.span);
    }

    // Check that "now" can be used as an identifier (noun, proper name, adjective, or adverb)
    let now_is_identifier = tokens.iter().any(|t| {
        let lexeme = interner.resolve(t.lexeme);
        matches!(t.kind,
            TokenType::Noun(_) | TokenType::ProperName(_) | TokenType::Adjective(_) |
            TokenType::TemporalAdverb(_) | TokenType::ScopalAdverb(_) | TokenType::Adverb(_))
            && lexeme == "now"
    });
    assert!(now_is_identifier, "now should be usable as an identifier");
}

/// Test that native function syntax is parsed correctly.
#[test]
fn test_native_function_parse() {
    let source = r#"
# Test

## To native read (path: Text) -> Result of Text and Text

## Main
Let x be 1.
"#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse native function: {:?}", result);
}

/// Test that native functions generate logicaffeine_system calls.
#[test]
fn test_file_read_codegen() {
    let source = r#"
# Test

## To native read (path: Text) -> Result of Text and Text

## Main
Let content be read("data.txt").
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    // Should generate wrapper function
    assert!(rust.contains("fn read"), "Should generate read function");
    // Should delegate to logicaffeine_system
    assert!(rust.contains("logicaffeine_system::file::read"), "Should call logicaffeine_system::file::read");
}

/// Test time module native functions.
#[test]
fn test_time_now_codegen() {
    let source = r#"
# Test

## To native now -> Nat

## Main
Let timestamp be now.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn now"), "Should generate now function");
    assert!(rust.contains("logicaffeine_system::time::now"), "Should call logicaffeine_system::time::now");
}

/// Test random module native functions.
#[test]
fn test_random_int_codegen() {
    let source = r#"
# Test

## To native randomInt (min: Int) and (max: Int) -> Int

## Main
Let n be randomInt(1, 100).
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn randomInt"), "Should generate randomInt function");
    assert!(rust.contains("logicaffeine_system::random::randomInt"), "Should call logicaffeine_system::random::randomInt");
}

/// Test env module native functions.
#[test]
fn test_env_args_codegen() {
    let source = r#"
# Test

## To native args -> Seq of Text

## Main
Let arguments be args.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn args"), "Should generate args function");
    assert!(rust.contains("logicaffeine_system::env::args"), "Should call logicaffeine_system::env::args");
}

/// Test Result type mapping.
#[test]
fn test_result_type_mapping() {
    let source = r#"
# Test

## To native read (path: Text) -> Result of Text and Text

## Main
Let x be 1.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    // Result of Text and Text -> Result<String, String>
    assert!(
        rust.contains("Result<String, String>"),
        "Should map Result of Text and Text to Rust Result<String, String>"
    );
}

/// Test Option type mapping.
#[test]
fn test_option_type_mapping() {
    let source = r#"
# Test

## To native get (key: Text) -> Option of Text

## Main
Let val be get("HOME").
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    // Option of Text -> Option<String>
    assert!(
        rust.contains("Option<String>"),
        "Should map Option of Text to Rust Option<String>"
    );
}

/// Test Seq type mapping.
#[test]
fn test_seq_type_mapping() {
    let source = r#"
# Test

## To native args -> Seq of Text

## Main
Let x be 1.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    // Seq of Text -> Vec<String>
    assert!(
        rust.contains("Vec<String>"),
        "Should map Seq of Text to Rust Vec<String>"
    );
}

/// Test native function with no parameters (nullary).
#[test]
fn test_native_nullary_function() {
    let source = r#"
# Test

## To native now -> Nat

## Main
Let t be now.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn now() -> u64"), "Should generate nullary function with return type");
}

/// Test parseInt generates codegen call to logicaffeine_system::text::parseInt.
#[test]
fn test_parseInt_codegen() {
    let source = r#"
# Test

## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("42").
Show n.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn parseInt"), "Should generate parseInt function");
    assert!(rust.contains("logicaffeine_system::text::parseInt"), "Should call logicaffeine_system::text::parseInt");
}

/// Test parseFloat generates codegen call to logicaffeine_system::text::parseFloat.
#[test]
fn test_parseFloat_codegen() {
    let source = r#"
# Test

## To native parseFloat (s: Text) -> Float

## Main
Let f be parseFloat("3.14").
Show f.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn parseFloat"), "Should generate parseFloat function");
    assert!(rust.contains("logicaffeine_system::text::parseFloat"), "Should call logicaffeine_system::text::parseFloat");
}

/// Test format generates codegen call to logicaffeine_system::fmt::format.
#[test]
fn test_format_codegen() {
    let source = r#"
# Test

## To native format (x: Int) -> Text

## Main
Let s be format(42).
Show s.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("fn format"), "Should generate format function");
    assert!(rust.contains("logicaffeine_system::fmt::format"), "Should call logicaffeine_system::fmt::format");
}

/// Test parseInt works in the interpreter.
#[test]
fn test_parseInt_interpreter() {
    use logicaffeine_compile::interpret_for_ui;
    use futures::executor::block_on;

    let source = r#"
## Main
Let n be parseInt("42").
Show n.
"#;
    let result = block_on(interpret_for_ui(source));
    assert!(result.error.is_none(), "Should succeed: {:?}", result.error);
    assert_eq!(result.lines.join("\n").trim(), "42");
}

/// Test parseFloat works in the interpreter.
#[test]
fn test_parseFloat_interpreter() {
    use logicaffeine_compile::interpret_for_ui;
    use futures::executor::block_on;

    let source = r#"
## Main
Let f be parseFloat("3.14").
Show f.
"#;
    let result = block_on(interpret_for_ui(source));
    assert!(result.error.is_none(), "Should succeed: {:?}", result.error);
    assert_eq!(result.lines.join("\n").trim(), "3.14");
}

/// Test fib benchmark compiles and produces correct output.
#[test]
fn test_fib_benchmark_e2e() {
    let source = r#"
## To native parseInt (s: Text) -> Int

## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Let n be parseInt("10").
Show fib(n).
"#;
    let result = common::run_logos(source);
    assert!(result.success, "fib benchmark should compile and run.\nstderr: {}\nrust:\n{}", result.stderr, result.rust_code);
    assert_eq!(result.stdout.trim(), "55");
}

/// Test ackermann benchmark compiles and produces correct output.
#[test]
fn test_ackermann_benchmark_e2e() {
    let source = r#"
## To native parseInt (s: Text) -> Int

## To ackermann (m: Int) and (n: Int) -> Int:
    If m equals 0:
        Return n + 1.
    If n equals 0:
        Return ackermann(m - 1, 1).
    Return ackermann(m - 1, ackermann(m, n - 1)).

## Main
Let n be parseInt("6").
Show ackermann(3, n).
"#;
    let result = common::run_logos(source);
    assert!(result.success, "ackermann benchmark should compile and run.\nstderr: {}\nrust:\n{}", result.stderr, result.rust_code);
    assert_eq!(result.stdout.trim(), "509");
}

/// Test sieve benchmark compiles and produces correct output.
#[test]
fn test_sieve_benchmark_e2e() {
    let source = r#"
## To native parseInt (s: Text) -> Int

## To sieve (limit: Int) -> Int:
    Let mutable flags be a new Seq of Bool.
    Let mutable i be 0.
    While i is at most limit:
        Push false to flags.
        Set i to i + 1.
    Let mutable count be 0.
    Set i to 2.
    While i is at most limit:
        If item (i + 1) of flags equals false:
            Set count to count + 1.
            Let mutable j be i * i.
            While j is at most limit:
                Set item (j + 1) of flags to true.
                Set j to j + i.
        Set i to i + 1.
    Return count.

## Main
Let limit be parseInt("100").
Show sieve(limit).
"#;
    let result = common::run_logos(source);
    assert!(result.success, "sieve benchmark should compile and run.\nstderr: {}\nrust:\n{}", result.stderr, result.rust_code);
    assert_eq!(result.stdout.trim(), "25");
}

/// Test collect benchmark compiles and produces correct output.
#[test]
fn test_collect_benchmark_e2e() {
    let source = r#"
## To native parseInt (s: Text) -> Int
## To native format (x: Int) -> Text

## Main
Let n be parseInt("100").
Let mutable m be a new Map of Text to Int.
Let mutable i be 0.
While i is less than n:
    Set item (format(i)) of m to i * 2.
    Set i to i + 1.
Let mutable found be 0.
Set i to 0.
While i is less than n:
    If item (format(i)) of m equals i * 2:
        Set found to found + 1.
    Set i to i + 1.
Show found.
"#;
    let result = common::run_logos(source);
    assert!(result.success, "collect benchmark should compile and run.\nstderr: {}\nrust:\n{}", result.stderr, result.rust_code);
    assert_eq!(result.stdout.trim(), "100");
}

/// Test bubble_sort benchmark compiles and produces correct output.
#[test]
fn test_bubble_sort_benchmark_e2e() {
    let source = r#"
## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("10").
Let mutable arr be a new Seq of Int.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than n:
    Set seed to (seed * 1103515245 + 12345) % 4294967296.
    Push (seed / 65536) % 32768 to arr.
    Set i to i + 1.
Set i to 0.
While i is less than n - 1:
    Let mutable j be 1.
    While j is at most n - 1 - i:
        Let a be item j of arr.
        Let b be item (j + 1) of arr.
        If a is greater than b:
            Set item j of arr to b.
            Set item (j + 1) of arr to a.
        Set j to j + 1.
    Set i to i + 1.
Show item 1 of arr.
"#;
    let result = common::run_logos(source);
    assert!(result.success, "bubble_sort benchmark should compile and run.\nstderr: {}\nrust:\n{}", result.stderr, result.rust_code);
    let first: i64 = result.stdout.trim().parse().expect("Should produce an integer");
    assert!(first >= 0 && first < 32768, "First element should be in range [0, 32768)");
}

/// Test strings benchmark compiles and produces correct output.
#[test]
fn test_strings_benchmark_e2e() {
    let source = r#"
## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("10").
Let mutable result be "".
Let mutable i be 0.
While i is less than n:
    Set result to result + i + " ".
    Set i to i + 1.
Let count: Int be Escape to Rust:
    result.chars().filter(|c| *c == ' ').count() as i64
Show count.
"#;
    let result = common::run_logos(source);
    assert!(result.success, "strings benchmark should compile and run.\nstderr: {}\nrust:\n{}", result.stderr, result.rust_code);
    assert_eq!(result.stdout.trim(), "10");
}

/// Test native function with multiple parameters.
#[test]
fn test_native_multi_param_function() {
    let source = r#"
# Test

## To native write (path: Text) and (content: Text) -> Result of Unit and Text

## Main
Let x be 1.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("fn write(path: String, content: String)"),
        "Should generate function with multiple params"
    );
}
