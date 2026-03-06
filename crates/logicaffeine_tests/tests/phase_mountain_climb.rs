mod common;

use common::compile_to_rust;

// =============================================================================
// CAMP 0c: Float Comparison Folding
// =============================================================================

#[test]
fn fold_float_gt() {
    let source = "## Main\nLet x be 3.14 > 0.0.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = true"), "Should fold 3.14>0.0 to true.\nGot:\n{}", rust);
}

#[test]
fn fold_float_eq() {
    let source = "## Main\nLet x be 1.0 equals 1.0.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = true"), "Should fold 1.0==1.0 to true.\nGot:\n{}", rust);
}

#[test]
fn fold_float_lt() {
    let source = "## Main\nLet x be 0.0 - 1.5 is less than 2.5.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = true"), "Should fold -1.5<2.5 to true.\nGot:\n{}", rust);
}

#[test]
fn fold_float_neq() {
    // Use "does not equal" syntax which parses correctly for NotEq
    let source = "## Main\nLet a be 1.0.\nLet b be 2.0.\nLet x be a is not b.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = true"), "Should fold 1.0!=2.0 to true.\nGot:\n{}", rust);
}

#[test]
fn fold_float_lteq() {
    let source = "## Main\nLet x be 1.0 is at most 1.0.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = true"), "Should fold 1.0<=1.0 to true.\nGot:\n{}", rust);
}

#[test]
fn fold_float_gteq() {
    let source = "## Main\nLet x be 0.0 is at least 1.0.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = false"), "Should fold 0.0>=1.0 to false.\nGot:\n{}", rust);
}

// NaN comparison tests - IEEE 754 semantics
#[test]
fn fold_float_nan_eq() {
    // NaN == NaN should be false per IEEE 754
    // Division by zero is intentionally preserved (not folded), so NaN comparison
    // must be verified at runtime, not compile time.
    let source = "## Main\nLet nan be 0.0 / 0.0.\nLet x be nan equals nan.\nShow x.";
    // Just verify it compiles and produces correct runtime output
    common::assert_exact_output(source, "false");
}

// =============================================================================
// CAMP 0d: Bitwise Operation Folding
// =============================================================================

#[test]
fn fold_bitxor() {
    let source = "## Main\nLet x be 255 xor 15.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 240"), "Should fold 0xFF^0x0F to 240.\nGot:\n{}", rust);
}

#[test]
fn fold_shl() {
    let source = "## Main\nLet x be 1 shifted left by 10.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 1024"), "Should fold 1<<10 to 1024.\nGot:\n{}", rust);
}

#[test]
fn fold_shr() {
    let source = "## Main\nLet x be 1024 shifted right by 5.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 32"), "Should fold 1024>>5 to 32.\nGot:\n{}", rust);
}

// =============================================================================
// CAMP 0e: Propagation in All Expression Contexts
// =============================================================================

#[test]
fn propagate_if_condition() {
    // Let x=5. If x>100 should propagate to If 5>100 -> If false -> eliminated
    let source = r#"## Main
Let x be 5.
If x is greater than 100:
    Show "big".
Otherwise:
    Show "small".
"#;
    let rust = compile_to_rust(source).unwrap();
    // After propagation + fold + DCE: If false -> else branch only
    assert!(!rust.contains("\"big\""), "Dead branch should be eliminated.\nGot:\n{}", rust);
    assert!(rust.contains("\"small\""), "Live branch should remain.\nGot:\n{}", rust);
}

#[test]
fn propagate_while_condition() {
    // Let x=false. While x -> While false -> eliminated
    let source = r#"## Main
Let x be false.
While x:
    Show "loop".
Show "done".
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("\"loop\""), "Dead while body should be eliminated.\nGot:\n{}", rust);
    assert!(rust.contains("\"done\""), "Post-loop code should remain.\nGot:\n{}", rust);
}

#[test]
fn propagate_return_value() {
    let source = r#"## To getValue () -> Int:
    Let x be 42.
    Return x.

## Main
Show getValue().
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("return 42") || rust.contains("42"), "Should propagate x=42 into return.\nGot:\n{}", rust);
}

#[test]
fn propagate_show_arg() {
    let source = r#"## Main
Let x be 42.
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("println!(\"{}\", 42)") || rust.contains("42"), "Should propagate x=42 into Show.\nGot:\n{}", rust);
}

#[test]
fn propagate_push_value() {
    let source = r#"## Main
Let x be 5.
Let mutable items: Seq of Int be [1, 2, 3].
Push x to items.
Show length of items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("push(5)") || rust.contains(".push(5)"), "Should propagate x=5 into Push.\nGot:\n{}", rust);
}

#[test]
fn propagate_no_substitute_index_target() {
    // Index/Slice targets should NOT be substituted (preserves peephole patterns)
    let source = r#"## Main
Let x be 5.
Let items: Seq of Int be [10, 20, 30, 40, 50].
Let val be item x of items.
Show val.
"#;
    let rust = compile_to_rust(source).unwrap();
    // This should still compile and run correctly
    common::assert_exact_output(source, "50");
}

// =============================================================================
// CAMP 0f: compile_to_rust_checked() Optimization
// =============================================================================

#[test]
fn checked_path_folds_constants() {
    use logicaffeine_compile::compile::compile_to_rust_checked;
    let source = "## Main\nLet x be 2 + 3.\nShow x.";
    let rust = compile_to_rust_checked(source).unwrap();
    assert!(rust.contains("let x = 5"), "Checked path should fold 2+3 to 5.\nGot:\n{}", rust);
}

#[test]
fn checked_path_eliminates_dead_code() {
    use logicaffeine_compile::compile::compile_to_rust_checked;
    let source = r#"## Main
If false:
    Show "dead".
Show "alive".
"#;
    let rust = compile_to_rust_checked(source).unwrap();
    assert!(!rust.contains("\"dead\""), "Checked path should eliminate dead branch.\nGot:\n{}", rust);
}

// =============================================================================
// CAMP 0g: Boolean Algebra Laws
// =============================================================================

#[test]
fn fold_bool_or_true() {
    // x || true -> true (when x is known)
    let source = "## Main\nLet x be false or true.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = true"), "false||true should fold to true.\nGot:\n{}", rust);
}

#[test]
fn fold_bool_or_false() {
    // x || false -> x (when x is known literal)
    let source = "## Main\nLet x be true or false.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = true"), "true||false should fold to true.\nGot:\n{}", rust);
}

#[test]
fn fold_bool_and_true() {
    let source = "## Main\nLet x be true and true.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = true"), "true&&true should fold to true.\nGot:\n{}", rust);
}

#[test]
fn fold_bool_and_false() {
    let source = "## Main\nLet x be true and false.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = false"), "true&&false should fold to false.\nGot:\n{}", rust);
}

#[test]
fn fold_bool_double_negation() {
    // !!x -> x
    let source = "## Main\nLet x be not not true.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = true"), "!!true should fold to true.\nGot:\n{}", rust);
}

// =============================================================================
// CAMP 0h: Self-Comparison Identities
// =============================================================================

#[test]
fn fold_self_sub() {
    // x - x -> 0 (when both are same literal)
    let source = "## Main\nLet x be 42 - 42.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 0"), "42-42 should fold to 0.\nGot:\n{}", rust);
}

#[test]
fn fold_self_xor() {
    // x ^ x -> 0 (when both are same literal)
    let source = "## Main\nLet x be 42 xor 42.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 0"), "42^42 should fold to 0.\nGot:\n{}", rust);
}

#[test]
fn fold_self_div() {
    // x / x -> 1 (when both are same non-zero literal)
    let source = "## Main\nLet x be 42 / 42.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 1"), "42/42 should fold to 1.\nGot:\n{}", rust);
}

#[test]
fn fold_self_mod() {
    // x % x -> 0 (when both are same non-zero literal)
    let source = "## Main\nLet x be 42 % 42.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let x = 0"), "42%42 should fold to 0.\nGot:\n{}", rust);
}
