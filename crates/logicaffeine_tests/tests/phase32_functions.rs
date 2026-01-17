//! Phase 32: Function Definitions & Inference
//!
//! Tests for function definitions, call expressions, and return type inference.

use logicaffeine_compile::compile::compile_to_rust;
use logicaffeine_language::Lexer;
use logicaffeine_base::Interner;
use logicaffeine_language::token::{TokenType, BlockType};
use logicaffeine_language::mwe;

#[test]
fn test_function_block_tokenized() {
    // Use exact source from failing test (with leading newline)
    let source = r#"
## To add (a: Int) and (b: Int):
    Return a + b.

## Main
Return.
"#;
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    // Debug: print all tokens with spans
    for (i, tok) in tokens.iter().enumerate() {
        eprintln!("{}: {:?} span={:?} ({})", i, tok.kind, tok.span, interner.resolve(tok.lexeme));
    }

    let has_function_block = tokens.iter().any(|t| {
        matches!(t.kind, TokenType::BlockHeader { block_type: BlockType::Function })
    });
    assert!(has_function_block, "Should tokenize ## To as Function block: {:?}", tokens);
}

#[test]
fn test_function_definition_parsed() {
    let source = r#"
## To add (a: Int) and (b: Int):
    Return a + b.

## Main
Return.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("fn add"), "Should define function add: {}", rust);
}

#[test]
fn test_return_type_inference() {
    let source = r#"
## To add (a: Int) and (b: Int):
    Return a + b.

## Main
Return.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("-> i64"), "Should infer return type as i64: {}", rust);
}

#[test]
fn test_function_call_expression() {
    let source = r#"
## To double (x: Int):
    Return x + x.

## Main
Let result be double(5).
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("double(5)"), "Should parse f(x) call syntax: {}", rust);
}

#[test]
fn test_function_codegen() {
    let source = r#"
## To add (a: Int) and (b: Int):
    Return a + b.

## Main
Let sum be add(3, 4).
Show sum.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("fn add(a: i64, b: i64) -> i64"), "Should emit full function signature: {}", rust);
    assert!(rust.contains("let sum = add(3, 4)"), "Should emit function call: {}", rust);
}

#[test]
fn test_unit_return_type() {
    let source = r#"
## To greet (name: Text):
    Show name.

## Main
Return.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("fn greet(name: String)"), "Should emit function with String param: {}", rust);
    // Unit return type should not have -> () in idiomatic Rust
    let fn_sig_end = rust.find("fn greet(name: String)").unwrap();
    let after_sig = &rust[fn_sig_end..];
    let brace_pos = after_sig.find('{').unwrap_or(0);
    let between = &after_sig[..brace_pos];
    assert!(!between.contains("->"), "Unit return should not have -> in signature: {}", between);
}

#[test]
fn test_call_statement_with_defined_function() {
    let source = r#"
## To greet (name: Text):
    Show name.

## Main
Call greet with "World".
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("greet("), "Should emit call to greet: {}", rust);
}
