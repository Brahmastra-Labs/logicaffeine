// Phase 43C: Refinement Types Tests
//
// Tests for parsing `Type where predicate` syntax and automatic enforcement
// of refinement constraints via debug_assert!().

use logos::ast::stmt::TypeExpr;
use logos::compile::compile_to_rust;

#[test]
fn refinement_type_variant_exists() {
    // Verify the Refinement variant is defined in TypeExpr
    fn _check_variant<'a>(_te: TypeExpr<'a>) {
        match _te {
            TypeExpr::Primitive(_) => {}
            TypeExpr::Named(_) => {}
            TypeExpr::Generic { .. } => {}
            TypeExpr::Function { .. } => {}
            TypeExpr::Refinement { base: _, var: _, predicate: _ } => {}
        }
    }
}

// ============================================================================
// Part 1: Parser Tests (RED)
// ============================================================================

#[test]
fn test_parse_int_where_positive() {
    let source = r#"
## Main
Let x: Int where x > 0 be 5.
"#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse refinement type: {:?}", result.err());
}

#[test]
fn test_parse_refinement_with_variable_name() {
    let source = r#"
## Main
Let count: Int where count > 0 be 10.
"#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse refinement with custom var name: {:?}", result.err());
}

// ============================================================================
// Part 2: Codegen Tests (RED)
// ============================================================================

#[test]
fn test_refinement_generates_debug_assert_at_let() {
    let source = r#"
## Main
Let x: Int where x > 0 be 5.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("let x: i64 = 5;"), "Should have let binding: {}", rust);
    assert!(rust.contains("debug_assert!((x > 0));"), "Should have refinement check: {}", rust);
}

#[test]
fn test_refinement_enforced_on_set() {
    let source = r#"
## Main
Let x: Int where x > 0 be 5.
Set x to 10.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    let count = rust.matches("debug_assert!((x > 0));").count();
    assert_eq!(count, 2, "Should have 2 refinement checks (Let + Set): {}", rust);
}

#[test]
fn test_refinement_variable_substitution() {
    let source = r#"
## Main
Let count: Int where count > 0 be 10.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("debug_assert!((count > 0));"), "Should use actual variable name: {}", rust);
}

#[test]
fn test_refinement_respects_block_scope() {
    let source = r#"
## Main
Let x: Int where x > 0 be 5.
If true:
    Let y: Int where y < 100 be 42.
    Set y to 50.
Set x to 3.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("debug_assert!((x > 0));"), "x should have its constraint: {}", rust);
    assert!(rust.contains("debug_assert!((y < 100));"), "y should have its constraint: {}", rust);
}

// ============================================================================
// Part 3: Extended Coverage
// ============================================================================

// === COMPOUND PREDICATES ===

#[test]
fn test_refinement_and_predicate() {
    let source = r#"
## Main
Let x: Int where x > 0 and x < 100 be 50.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("debug_assert!(((x > 0) && (x < 100)));"),
        "Should have compound AND check: {}", rust);
}

#[test]
fn test_refinement_or_predicate() {
    let source = r#"
## Main
Let flag: Int where flag == 0 or flag == 1 be 1.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("debug_assert!(((flag == 0) || (flag == 1)));"),
        "Should have compound OR check: {}", rust);
}

// === ALL COMPARISON OPERATORS ===

#[test]
fn test_refinement_less_than() {
    let source = r#"
## Main
Let x: Int where x < 10 be 5.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("debug_assert!((x < 10));"), "Less than: {}", rust);
}

#[test]
fn test_refinement_less_equal() {
    let source = r#"
## Main
Let x: Int where x <= 10 be 10.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("debug_assert!((x <= 10));"), "Less equal: {}", rust);
}

#[test]
fn test_refinement_greater_equal() {
    let source = r#"
## Main
Let x: Int where x >= 0 be 0.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("debug_assert!((x >= 0));"), "Greater equal: {}", rust);
}

#[test]
fn test_refinement_not_equal() {
    let source = r#"
## Main
Let x: Int where x != 0 be 1.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("debug_assert!((x != 0));"), "Not equal: {}", rust);
}

// === LOOP SCOPES ===

#[test]
fn test_refinement_while_scope() {
    let source = r#"
## Main
Let x: Int where x > 0 be 5.
While x > 0:
    Let y: Int where y < 50 be 10.
    Set y to 20.
    Set x to 0.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("debug_assert!((x > 0));"), "x constraint: {}", rust);
    assert!(rust.contains("debug_assert!((y < 50));"), "y constraint: {}", rust);
}

// === MULTIPLE REFINED VARS ===

#[test]
fn test_multiple_refined_vars_same_scope() {
    let source = r#"
## Main
Let x: Int where x > 0 be 5.
Let y: Int where y < 100 be 42.
Set x to 10.
Set y to 50.
"#;
    let rust = compile_to_rust(source).expect("Should compile");
    let x_count = rust.matches("debug_assert!((x > 0));").count();
    let y_count = rust.matches("debug_assert!((y < 100));").count();
    assert_eq!(x_count, 2, "x should have 2 checks: {}", rust);
    assert_eq!(y_count, 2, "y should have 2 checks: {}", rust);
}
