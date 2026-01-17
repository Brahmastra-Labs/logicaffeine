//! Phase 33: Sum Types & Pattern Matching
//!
//! Tests for enum definitions, variant discovery, Inspect statements, and pattern matching.

use logicaffeine_compile::compile::compile_to_rust;
use logicaffeine_language::Lexer;
use logicaffeine_base::Interner;
use logicaffeine_language::token::TokenType;

#[test]
fn test_enum_definition_tokenized() {
    let source = "## Definition\nA Shape is either:\n    A Circle.\n    A Point.";
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    // Debug: print all tokens
    for (i, tok) in tokens.iter().enumerate() {
        eprintln!("{}: {:?} ({})", i, tok.kind, interner.resolve(tok.lexeme));
    }

    // Should tokenize "either" as Either keyword
    let has_either = tokens.iter().any(|t| {
        matches!(t.kind, TokenType::Either)
    });
    assert!(has_either, "Should tokenize 'either' as Either keyword: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>());
}

#[test]
fn test_unit_variants_discovered() {
    let source = r#"
## Definition
A Color is either:
    A Red.
    A Green.
    A Blue.

## Main
Return.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    // Registry should discover Color with 3 unit variants
    assert!(rust.contains("enum Color"), "Should define enum Color: {}", rust);
    assert!(rust.contains("Red"), "Should have Red variant: {}", rust);
    assert!(rust.contains("Green"), "Should have Green variant: {}", rust);
    assert!(rust.contains("Blue"), "Should have Blue variant: {}", rust);
}

#[test]
fn test_payload_variants_discovered() {
    let source = r#"
## Definition
A Shape is either:
    A Circle with a radius, which is Int.
    A Rectangle with a width, which is Int, and a height, which is Int.
    A Point.

## Main
Return.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    // Registry should discover Shape with payload variants
    assert!(rust.contains("enum Shape"), "Should define enum Shape: {}", rust);
    assert!(rust.contains("Circle"), "Should have Circle variant: {}", rust);
    assert!(rust.contains("radius"), "Circle should have radius field: {}", rust);
    assert!(rust.contains("Rectangle"), "Should have Rectangle variant: {}", rust);
    assert!(rust.contains("Point"), "Should have Point variant: {}", rust);
}

#[test]
fn test_enum_codegen() {
    let source = r#"
## Definition
A Shape is either:
    A Circle with a radius, which is Int.
    A Point.

## Main
Return.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("enum Shape {"), "Should emit enum declaration: {}", rust);
    assert!(rust.contains("Circle { radius: i64 }"), "Should emit Circle with i64 radius: {}", rust);
    assert!(rust.contains("Point,") || rust.contains("Point }"), "Should emit Point as unit variant: {}", rust);
}

#[test]
fn test_inspect_statement_parsed() {
    let source = r#"
## Definition
A Shape is either:
    A Circle with a radius, which is Int.
    A Point.

## Main
Let s be a new Point.
Inspect s:
    If it is a Circle (radius):
        Show radius.
    If it is a Point:
        Show "point".
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("match s {") || rust.contains("match s{"),
        "Should emit match expression: {}", rust);
}

#[test]
fn test_variant_constructor() {
    let source = r#"
## Definition
A Shape is either:
    A Circle with a radius, which is Int.
    A Point.

## Main
Let c be a new Circle with radius 10.
Let p be a new Point.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Shape::Circle { radius: 10 }"),
        "Should emit variant constructor with field: {}", rust);
    assert!(rust.contains("Shape::Point"),
        "Should emit unit variant constructor: {}", rust);
}

#[test]
fn test_full_pattern_matching_codegen() {
    let source = r#"
## Definition
A Shape is either:
    A Circle with a radius, which is Int.
    A Point.

## Main
Let s be a new Circle with radius 5.
Inspect s:
    If it is a Circle (radius: r):
        Show r.
    If it is a Point:
        Show "point".
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Shape::Circle { radius: ref r }") ||
            rust.contains("Shape::Circle { radius: r }"),
        "Should emit pattern with binding: {}", rust);
    assert!(rust.contains("Shape::Point =>"),
        "Should emit Point pattern: {}", rust);
}

#[test]
fn test_otherwise_clause() {
    let source = r#"
## Definition
A Color is either:
    A Red.
    A Green.
    A Blue.

## Main
Let c be a new Red.
Inspect c:
    If it is a Red:
        Show "red".
    Otherwise:
        Show "not red".
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Color::Red =>"), "Should emit Red pattern: {}", rust);
    assert!(rust.contains("_ =>"), "Should emit wildcard for Otherwise: {}", rust);
}

#[test]
fn test_concise_variant_syntax() {
    let source = r#"
## Definition
A Result is either:
    A Success (value: Int).
    A Failure (message: Text).

## Main
Return.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("Success { value: i64 }"),
        "Should parse concise syntax for Success: {}", rust);
    assert!(rust.contains("Failure { message: String }"),
        "Should parse concise syntax for Failure: {}", rust);
}
