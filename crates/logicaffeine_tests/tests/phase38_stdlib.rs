//! Phase 38: Standard Library (IO & System)
//!
//! Tests for native function support and standard library modules.

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
