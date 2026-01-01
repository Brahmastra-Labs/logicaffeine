//! Phase 10: Standard Library (IO) Tests
//!
//! Tests for console input/output and file I/O operations.

mod common;
use common::compile_to_rust;

// =============================================================================
// Tokenization Tests
// =============================================================================

#[test]
fn test_read_console_tokens() {
    use logos::lexer::Lexer;
    use logos::intern::Interner;
    use logos::token::TokenType;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new("## Main\nRead input from the console.", &mut interner);
    let tokens = lexer.tokenize();

    assert!(tokens.iter().any(|t| matches!(t.kind, TokenType::Read)),
            "Should have Read token");
    assert!(tokens.iter().any(|t| matches!(t.kind, TokenType::Console)),
            "Should have Console token");
}

#[test]
fn test_read_file_tokens() {
    use logos::lexer::Lexer;
    use logos::intern::Interner;
    use logos::token::TokenType;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new("## Main\nRead data from file \"test.txt\".", &mut interner);
    let tokens = lexer.tokenize();

    assert!(tokens.iter().any(|t| matches!(t.kind, TokenType::Read)),
            "Should have Read token");
    assert!(tokens.iter().any(|t| matches!(t.kind, TokenType::File)),
            "Should have File token");
}

#[test]
fn test_write_file_tokens() {
    use logos::lexer::Lexer;
    use logos::intern::Interner;
    use logos::token::TokenType;

    let mut interner = Interner::new();
    let mut lexer = Lexer::new("## Main\nWrite \"hello\" to file \"out.txt\".", &mut interner);
    let tokens = lexer.tokenize();

    assert!(tokens.iter().any(|t| matches!(t.kind, TokenType::Write)),
            "Should have Write token");
    assert!(tokens.iter().any(|t| matches!(t.kind, TokenType::File)),
            "Should have File token");
}

// =============================================================================
// Parse + Codegen Tests (Console)
// =============================================================================

#[test]
fn test_read_from_console_codegen() {
    let source = "## Main\nRead input from the console.\nShow input.";
    let rust = compile_to_rust(source).expect("Should compile");

    assert!(rust.contains("read_line()"),
            "Should call read_line().\nGot:\n{}", rust);
}

#[test]
fn test_read_from_console_without_article() {
    // Should work with or without "the"
    let source = "## Main\nRead input from console.\nShow input.";
    let rust = compile_to_rust(source).expect("Should compile");

    assert!(rust.contains("read_line()"),
            "Should call read_line() without 'the'.\nGot:\n{}", rust);
}

// =============================================================================
// Parse + Codegen Tests (File Read)
// =============================================================================

#[test]
fn test_read_from_file_codegen() {
    let source = r#"## Main
Read data from file "config.txt".
Show data."#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Phase 53: File operations now use VFS abstraction
    assert!(rust.contains("read_to_string"),
            "Should call vfs.read_to_string.\nGot:\n{}", rust);
    assert!(rust.contains("config.txt"),
            "Should include file path.\nGot:\n{}", rust);
}

#[test]
fn test_read_from_file_variable_path() {
    let source = r#"## Main
Let path be "data.txt".
Read content from file path.
Show content."#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Phase 53: File operations now use VFS abstraction
    assert!(rust.contains("read_to_string"),
            "Should call vfs.read_to_string.\nGot:\n{}", rust);
    assert!(rust.contains("path"),
            "Should use path variable.\nGot:\n{}", rust);
}

// =============================================================================
// Parse + Codegen Tests (File Write)
// =============================================================================

#[test]
fn test_write_to_file_codegen() {
    let source = r#"## Main
Write "hello world" to file "output.txt"."#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Phase 53: File operations now use VFS abstraction
    assert!(rust.contains("vfs.write"),
            "Should call vfs.write.\nGot:\n{}", rust);
    assert!(rust.contains("output.txt"),
            "Should include file path.\nGot:\n{}", rust);
}

#[test]
fn test_write_variable_to_file() {
    let source = r#"## Main
Let message be "Hello, World!".
Write message to file "greeting.txt"."#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Phase 53: File operations now use VFS abstraction
    assert!(rust.contains("vfs.write"),
            "Should call vfs.write.\nGot:\n{}", rust);
}

#[test]
fn test_read_and_write_file() {
    let source = r#"## Main
Read original from file "input.txt".
Write original to file "output.txt"."#;
    let rust = compile_to_rust(source).expect("Should compile");

    // Phase 53: File operations now use VFS abstraction
    assert!(rust.contains("read_to_string"),
            "Should have vfs.read_to_string.\nGot:\n{}", rust);
    assert!(rust.contains("vfs.write"),
            "Should have vfs.write.\nGot:\n{}", rust);
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[test]
fn test_read_missing_from_fails() {
    let source = "## Main\nRead input the console.";
    let result = compile_to_rust(source);

    assert!(result.is_err(), "Should fail without 'from' keyword");
}

#[test]
fn test_read_missing_source_fails() {
    let source = "## Main\nRead input from.";
    let result = compile_to_rust(source);

    assert!(result.is_err(), "Should fail without source (console/file)");
}

#[test]
fn test_write_missing_to_fails() {
    let source = r#"## Main
Write "hello" file "out.txt"."#;
    let result = compile_to_rust(source);

    assert!(result.is_err(), "Should fail without 'to' keyword");
}

#[test]
fn test_write_missing_file_fails() {
    let source = r#"## Main
Write "hello" to "out.txt"."#;
    let result = compile_to_rust(source);

    assert!(result.is_err(), "Should fail without 'file' keyword");
}

// =============================================================================
// Integration Tests
// =============================================================================

#[test]
fn test_io_in_function() {
    let source = r#"## To greet:
    Read name from the console.
    Write name to file "name.txt".
    Return.

## Main
Call greet."#;
    let rust = compile_to_rust(source).expect("Should compile");

    assert!(rust.contains("fn greet"),
            "Should have function definition.\nGot:\n{}", rust);
    assert!(rust.contains("read_line()"),
            "Should have read_line in function.\nGot:\n{}", rust);
    // Phase 53: File operations now use VFS abstraction
    assert!(rust.contains("vfs.write"),
            "Should have vfs.write in function.\nGot:\n{}", rust);
}

#[test]
fn test_io_in_conditional() {
    let source = r#"## Main
Let mode be 1.
If mode == 1:
    Read input from the console.
    Show input.
Otherwise:
    Write "default" to file "output.txt"."#;
    let rust = compile_to_rust(source).expect("Should compile");

    assert!(rust.contains("read_line()"),
            "Should have read_line in if branch.\nGot:\n{}", rust);
    // Phase 53: File operations now use VFS abstraction
    assert!(rust.contains("vfs.write"),
            "Should have vfs.write in else branch.\nGot:\n{}", rust);
}

#[test]
fn test_io_in_loop() {
    let source = r#"## Main
Let count be 0.
While count < 3:
    Read line from the console.
    Set count to count + 1."#;
    let rust = compile_to_rust(source).expect("Should compile");

    assert!(rust.contains("while"),
            "Should have while loop.\nGot:\n{}", rust);
    assert!(rust.contains("read_line()"),
            "Should have read_line in loop.\nGot:\n{}", rust);
}
