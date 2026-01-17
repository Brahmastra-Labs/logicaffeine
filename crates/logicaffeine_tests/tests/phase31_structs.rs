//! Phase 31: User-Defined Types (Encapsulated Records)
//!
//! Tests for struct definitions, field access, and visibility.

use logicaffeine_compile::compile::compile_to_rust;

#[test]
fn test_struct_definition_parsed() {
    let source = r#"
## Definition
A Point has:
    an x, which is Int.
    a y, which is Int.

## Main
Return.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("struct Point"), "Should define Point struct: {}", rust);
}

#[test]
fn test_public_field_codegen() {
    let source = r#"
## Definition
A Point has:
    a public x, which is Int.
    a public y, which is Int.

## Main
Let p be a new Point.
Set p's x to 10.
Show p.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("pub struct Point"), "Should have pub struct: {}", rust);
    assert!(rust.contains("pub x: i64"), "x should be pub: {}", rust);
    assert!(rust.contains("pub y: i64"), "y should be pub: {}", rust);
    assert!(rust.contains("mod user_types"), "Should wrap in module: {}", rust);
}

#[test]
fn test_field_visibility_codegen() {
    // Phase 50: Both concise and natural syntax fields are public by default
    let source = r#"
## Definition
A User has:
    a name, which is Text.
    an age, which is Nat.

## Main
Return.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("pub name: String"), "name should be pub: {}", rust);
    assert!(rust.contains("pub age: u64"), "age should be pub: {}", rust);
}

#[test]
fn test_new_constructor() {
    let source = r#"
## Definition
A Point has:
    a public x, which is Int.

## Main
Let p be a new Point.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("Point::default()"), "Should use default constructor: {}", rust);
}

#[test]
fn test_field_access_possessive() {
    let source = r#"
## Definition
A Point has:
    a public x, which is Int.

## Main
Let p be a new Point.
Set p's x to 42.
Show p's x.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("p.x = 42"), "Should emit field assignment: {}", rust);
    assert!(rust.contains("show(&p.x)"), "Should emit field access in show: {}", rust);
}

#[test]
fn test_struct_derives() {
    let source = r#"
## Definition
A Point has:
    a public x, which is Int.

## Main
Return.
"#;
    let rust = compile_to_rust(source).expect("Compiles");
    assert!(rust.contains("#[derive(Default, Debug, Clone, PartialEq)]"), "Should have derive macros: {}", rust);
}
