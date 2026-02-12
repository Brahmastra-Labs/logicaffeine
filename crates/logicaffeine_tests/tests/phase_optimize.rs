mod common;

use common::compile_to_rust;

// =============================================================================
// Constant Folding
// =============================================================================

#[test]
fn fold_integer_addition() {
    let source = "## Main\nLet x be 2 + 3.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 5;"), "Should fold 2+3 to 5.\nGot:\n{}", rust);
}

#[test]
fn fold_integer_multiplication() {
    let source = "## Main\nLet x be 2 * 3.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 6;"), "Should fold 2*3 to 6.\nGot:\n{}", rust);
}

#[test]
fn fold_integer_subtraction() {
    let source = "## Main\nLet x be 10 - 3.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 7;"), "Should fold 10-3 to 7.\nGot:\n{}", rust);
}

#[test]
fn fold_integer_division() {
    let source = "## Main\nLet x be 10 / 2.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 5;"), "Should fold 10/2 to 5.\nGot:\n{}", rust);
}

#[test]
fn fold_integer_modulo() {
    let source = "## Main\nLet x be 10 % 3.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 1;"), "Should fold 10%3 to 1.\nGot:\n{}", rust);
}

#[test]
fn fold_nested_arithmetic() {
    let source = "## Main\nLet x be (2 + 3) * 4.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 20;"), "Should fold (2+3)*4 to 20.\nGot:\n{}", rust);
}

#[test]
fn fold_comparison_eq() {
    let source = "## Main\nLet x be 5.\nIf 3 == 3:\n    Show x.";
    let rust = compile_to_rust(source).unwrap();
    // The comparison 3==3 should fold to true, and DCE should inline the body
    assert!(!rust.contains("=="), "Should fold 3==3 away.\nGot:\n{}", rust);
}

#[test]
fn fold_comparison_lt() {
    let source = "## Main\nLet x be 5.\nIf 1 < 2:\n    Show x.";
    let rust = compile_to_rust(source).unwrap();
    // 1 < 2 folds to true, DCE inlines the body, no if remains
    assert!(!rust.contains("if"), "Should fold 1<2 and eliminate if.\nGot:\n{}", rust);
}

#[test]
fn fold_chained_arithmetic() {
    let source = "## Main\nLet x be 1 + 2 + 3.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 6;"), "Should fold 1+2+3 to 6.\nGot:\n{}", rust);
}

#[test]
fn fold_does_not_touch_variables() {
    let source = "## Main\nLet a be 5.\nLet b be a + 1.\nShow b.";
    let rust = compile_to_rust(source).unwrap();
    // Should NOT fold `a + 1` since `a` is a variable
    assert!(!rust.contains("let b = 6;"), "Should NOT fold variable expressions.\nGot:\n{}", rust);
}

#[test]
fn fold_no_division_by_zero() {
    let source = "## Main\nLet x be 10 / 0.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    // Should NOT fold division by zero
    assert!(!rust.contains("let x = ;"), "Should NOT fold division by zero.\nGot:\n{}", rust);
}

// =============================================================================
// Constant Folding — Block Recursion
// =============================================================================

#[test]
fn fold_inside_if_body() {
    let source = "## Main\nLet x be 5.\nIf x > 0:\n    Let y be 2 + 3.\n    Show y.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let y = 5"), "Should fold 2+3 inside if body.\nGot:\n{}", rust);
}

#[test]
fn fold_inside_while_body() {
    let source = r#"## Main
Let mut i be 3.
While i > 0:
    Let x be 10 * 2.
    Show x.
    Set i to i - 1.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 20"), "Should fold 10*2 inside while body.\nGot:\n{}", rust);
}

// =============================================================================
// Dead Code Elimination
// =============================================================================

#[test]
fn dce_false_condition_no_else() {
    let source = r#"## Main
If false:
    Show "dead".
Show "alive".
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("dead"), "Should eliminate dead branch.\nGot:\n{}", rust);
    assert!(rust.contains("alive"), "Should keep live code.\nGot:\n{}", rust);
}

#[test]
fn dce_false_condition_with_else() {
    let source = r#"## Main
If false:
    Show "dead".
Otherwise:
    Show "alive".
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("dead"), "Should eliminate dead branch.\nGot:\n{}", rust);
    assert!(rust.contains("alive"), "Should keep else branch.\nGot:\n{}", rust);
}

#[test]
fn dce_true_condition() {
    let source = r#"## Main
If true:
    Show "alive".
Otherwise:
    Show "dead".
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("alive"), "Should keep true branch.\nGot:\n{}", rust);
    assert!(!rust.contains("dead"), "Should eliminate else branch.\nGot:\n{}", rust);
}

#[test]
fn dce_folded_condition() {
    let source = r#"## Main
If 1 > 2:
    Show "dead".
Otherwise:
    Show "alive".
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("dead"), "Should eliminate dead branch after folding 1>2 to false.\nGot:\n{}", rust);
    assert!(rust.contains("alive"), "Should keep else branch.\nGot:\n{}", rust);
}

#[test]
fn dce_true_branch_with_push() {
    // Push IS handled in clone_stmt_ref, so this should always work.
    // This verifies basic DCE inlining of true branches.
    let source = r#"## Main
Let items be a new Seq of Int.
If true:
    Push 1 to items.
Show items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("push"), "DCE should preserve Push inside true branch.\nGot:\n{}", rust);
    assert!(!rust.contains("if true"), "DCE should inline true branch.\nGot:\n{}", rust);
}

#[test]
fn dce_true_branch_no_spurious_return() {
    // The catch-all in clone_stmt_ref replaces unhandled Stmt variants with
    // `return;`. If any unhandled variant ends up in a true branch, DCE
    // would silently replace it. This test uses an Inspect (match) inside
    // If true: — Inspect is NOT in clone_stmt_ref's match arms.
    let source = r#"## A Shape is one of:
    A Circle with radius Int.
    A Square with side Int.

## Main
Let s be a new Circle with radius 5.
If true:
    Inspect s:
        When Circle (r): Show r.
        When Square (n): Show n.
"#;
    let rust = compile_to_rust(source).unwrap();
    // After DCE inlines the true branch, the Inspect should be preserved,
    // not replaced with `return;`
    assert!(rust.contains("match") || rust.contains("radius"),
        "DCE should preserve Inspect inside true branch, not replace with return.\nGot:\n{}", rust);
    assert!(!rust.contains("return;"),
        "DCE should not inject spurious `return;` when inlining.\nGot:\n{}", rust);
}

#[test]
fn dce_true_branch_preserves_all_stmt_types() {
    // Zone, Concurrent, etc. inside If true: should survive DCE
    let source = r#"## Main
Let items be a new Seq of Int.
If true:
    Let x be 1.
    Push x to items.
    Show items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("push"), "Should preserve Push.\nGot:\n{}", rust);
    assert!(rust.contains("let x = 1"), "Should preserve Let.\nGot:\n{}", rust);
}

// =============================================================================
// Constant Folding — Float & Boolean
// =============================================================================

#[test]
fn fold_float_addition() {
    let source = "## Main\nLet x be 2.5 + 1.5.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 4") || rust.contains("let x: f64 = 4"), "Should fold 2.5+1.5 to 4.0.\nGot:\n{}", rust);
}

#[test]
fn fold_float_multiplication() {
    let source = "## Main\nLet x be 3.0 * 2.0.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 6") || rust.contains("let x: f64 = 6"), "Should fold 3.0*2.0 to 6.0.\nGot:\n{}", rust);
}

#[test]
fn fold_float_no_division_by_zero() {
    let source = "## Main\nLet x be 1.0 / 0.0.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    // Should NOT fold — division by zero
    assert!(rust.contains("/"), "Should NOT fold float division by zero.\nGot:\n{}", rust);
}

#[test]
fn fold_boolean_and_to_false() {
    let source = r#"## Main
If true and false:
    Show "dead".
Otherwise:
    Show "alive".
"#;
    let rust = compile_to_rust(source).unwrap();
    // true && false folds to false, DCE keeps else
    assert!(!rust.contains("dead"), "Should fold true&&false to false and eliminate.\nGot:\n{}", rust);
    assert!(rust.contains("alive"), "Should keep else branch.\nGot:\n{}", rust);
}

#[test]
fn fold_boolean_or_to_true() {
    let source = r#"## Main
If false or true:
    Show "alive".
Otherwise:
    Show "dead".
"#;
    let rust = compile_to_rust(source).unwrap();
    // false || true folds to true, DCE inlines then
    assert!(rust.contains("alive"), "Should fold false||true to true and inline.\nGot:\n{}", rust);
    assert!(!rust.contains("dead"), "Should eliminate else.\nGot:\n{}", rust);
}

#[test]
fn dce_does_not_eliminate_variable_conditions() {
    let source = r#"## Main
Let flag be true.
If flag:
    Show "maybe".
"#;
    let rust = compile_to_rust(source).unwrap();
    // Should NOT eliminate — flag is a variable
    assert!(rust.contains("maybe"), "Should NOT eliminate variable conditions.\nGot:\n{}", rust);
}

// =============================================================================
// Constant Folding — Additional Coverage
// =============================================================================

#[test]
fn fold_inside_repeat_body() {
    let source = r#"## Main
Let items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Repeat for x in items:
    Let y be 10 + 20.
    Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let y = 30"), "Should fold 10+20 inside Repeat body.\nGot:\n{}", rust);
}

#[test]
fn fold_float_comparison_not_folded() {
    // Float comparisons are intentionally NOT folded (IEEE NaN safety)
    let source = r#"## Main
If 1.0 < 2.0:
    Show "yes".
"#;
    let rust = compile_to_rust(source).unwrap();
    // Float comparisons should NOT be folded, so the `if` should remain
    assert!(rust.contains("if") || rust.contains("<"), "Float comparisons should NOT be folded.\nGot:\n{}", rust);
}

#[test]
fn fold_mixed_type_not_folded() {
    // 2 + 3.0 should not fold (different types: Number vs Float)
    let source = "## Main\nLet x be 2 + 3.0.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("+"), "Mixed-type operations should NOT be folded.\nGot:\n{}", rust);
}

#[test]
fn fold_inside_give_expression() {
    let source = r#"## To process (data: Int) -> Int is exported:
    Let result be 2 + 3.
    Return result.

## Main
Show 42.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let result = 5"), "Should fold 2+3 inside exported function body.\nGot:\n{}", rust);
}

// =============================================================================
// Dead Code Elimination — Additional Coverage
// =============================================================================

#[test]
fn dce_while_false_eliminated() {
    let source = r#"## Main
While false:
    Show "dead".
Show "alive".
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("dead"), "While false body should be eliminated.\nGot:\n{}", rust);
    assert!(rust.contains("alive"), "Code after while false should remain.\nGot:\n{}", rust);
}

#[test]
fn dce_inside_function_body() {
    let source = r#"## To compute () -> Int:
    If false:
        Let x be 1.
        Return x.
    Return 42.

## Main
Let result be compute().
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    // The if false branch should be eliminated even inside a function body
    assert!(!rust.contains("let x = 1"), "DCE should eliminate dead code inside function body.\nGot:\n{}", rust);
}
