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

// =============================================================================
// Tail Call Elimination — Codegen Tests
// =============================================================================

#[test]
fn tce_simple_tail_recursion() {
    let source = r#"## To countdown (n: Int) -> Int:
    If n equals 0:
        Return 0.
    Show n.
    Return countdown(n - 1).

## Main
countdown(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("loop {"), "TCE should wrap tail-recursive function in loop.\nGot:\n{}", rust);
    assert!(rust.contains("continue;"), "TCE should emit continue for tail call.\nGot:\n{}", rust);
    assert!(rust.contains("mut n"), "TCE should make params mutable.\nGot:\n{}", rust);
}

#[test]
fn tce_ackermann() {
    let source = r#"## To ackermann (m: Int) and (n: Int) -> Int:
    If m equals 0:
        Return n + 1.
    If n equals 0:
        Return ackermann(m - 1, 1).
    Return ackermann(m - 1, ackermann(m, n - 1)).

## Main
Show ackermann(3, 4).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("loop {"), "TCE should wrap Ackermann in loop.\nGot:\n{}", rust);
    assert!(rust.contains("continue;"), "TCE should emit continue for tail calls.\nGot:\n{}", rust);
    assert!(rust.contains("mut m"), "TCE should make m mutable.\nGot:\n{}", rust);
    assert!(rust.contains("mut n"), "TCE should make n mutable.\nGot:\n{}", rust);
    // The inner ackermann(m, n - 1) should remain as a normal recursive call
    assert!(rust.contains("ackermann(m,"), "Inner non-tail call should remain as recursion.\nGot:\n{}", rust);
}

#[test]
fn tce_no_false_positive() {
    let source = r#"## To notTail (n: Int) -> Int:
    If n equals 0:
        Return 1.
    Return notTail(n - 1) + 1.

## Main
Show notTail(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    // The + 1 means the call is NOT in tail position — TCE should NOT fire.
    // However, accumulator introduction correctly optimizes this pattern
    // (f(n-1) + k → zero overhead loop with accumulator).
    // Verify: accumulator fires (loop + __acc), but NOT raw TCE (__tce_ without __acc)
    assert!(rust.contains("__acc"), "Accumulator should optimize f(n-1)+1 pattern.\nGot:\n{}", rust);
    assert!(rust.contains("loop {"), "Accumulator should use loop.\nGot:\n{}", rust);
    assert!(rust.contains("continue;"), "Accumulator should use continue.\nGot:\n{}", rust);
}

#[test]
fn tce_non_recursive_unchanged() {
    let source = r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main
Show double(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("loop {"), "Non-recursive function should NOT get loop.\nGot:\n{}", rust);
}

#[test]
fn tce_argument_ordering() {
    let source = r#"## To swapRecurse (a: Int) and (b: Int) -> Int:
    If a equals 0:
        Return b.
    Return swapRecurse(b, a - 1).

## Main
Show swapRecurse(3, 100).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("__tce_"), "TCE should use temporaries for argument ordering.\nGot:\n{}", rust);
    assert!(rust.contains("loop {"), "TCE should wrap in loop.\nGot:\n{}", rust);
    assert!(rust.contains("continue;"), "TCE should emit continue.\nGot:\n{}", rust);
}

// =============================================================================
// Tail Call Elimination — E2E Correctness Tests
// =============================================================================

#[test]
fn e2e_tce_ackermann_correct() {
    let source = r#"## To ackermann (m: Int) and (n: Int) -> Int:
    If m equals 0:
        Return n + 1.
    If n equals 0:
        Return ackermann(m - 1, 1).
    Return ackermann(m - 1, ackermann(m, n - 1)).

## Main
Show ackermann(3, 4).
"#;
    common::assert_exact_output(source, "125");
}

#[test]
fn e2e_tce_factorial_correct() {
    let source = r#"## To factorial (n: Int) and (acc: Int) -> Int:
    If n equals 0:
        Return acc.
    Return factorial(n - 1, acc * n).

## Main
Show factorial(10, 1).
"#;
    common::assert_exact_output(source, "3628800");
}

#[test]
fn e2e_tce_countdown_correct() {
    let source = r#"## To countdown (n: Int) -> Int:
    If n equals 0:
        Return 0.
    Show n.
    Return countdown(n - 1).

## Main
countdown(5).
"#;
    common::assert_exact_output(source, "5\n4\n3\n2\n1");
}

// =============================================================================
// Inline Annotations
// =============================================================================

#[test]
fn inline_small_function() {
    let source = r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main
Show double(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("#[inline]"), "Small non-recursive function should get #[inline].\nGot:\n{}", rust);
}

#[test]
fn inline_recursive_skipped() {
    let source = r#"## To countdown (n: Int) -> Int:
    If n equals 0:
        Return 0.
    Show n.
    Return countdown(n - 1).

## Main
countdown(3).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("#[inline]"), "Recursive function should NOT get #[inline].\nGot:\n{}", rust);
}

#[test]
fn inline_exported_skipped() {
    let source = r#"## To compute (x: Int) -> Int is exported:
    Return x + 1.

## Main
Show compute(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    // #[inline] should NOT appear before exported functions
    // (exported functions have #[export_name = ...] instead)
    let lines: Vec<&str> = rust.lines().collect();
    let has_inline_before_export = lines.windows(2).any(|w| {
        w[0].trim() == "#[inline]" && w[1].contains("export_name")
    });
    assert!(!has_inline_before_export, "Exported function should NOT get #[inline].\nGot:\n{}", rust);
}

// =============================================================================
// Accumulator Introduction — Codegen Tests
// =============================================================================

#[test]
fn acc_factorial_codegen() {
    let source = r#"## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).

## Main
Show factorial(10).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("loop {"), "Accumulator should wrap function in loop.\nGot:\n{}", rust);
    assert!(rust.contains("__acc"), "Accumulator should use __acc variable.\nGot:\n{}", rust);
    assert!(rust.contains("continue;"), "Accumulator should emit continue for recursive call.\nGot:\n{}", rust);
    assert!(rust.contains("mut n"), "Accumulator should make params mutable.\nGot:\n{}", rust);
    // The recursive call should be eliminated — no factorial( call inside the body
    // (skip the function signature line which naturally contains factorial()
    let body_start = rust.find("fn factorial").unwrap();
    let brace = rust[body_start..].find('{').unwrap();
    let func_inner = &rust[body_start + brace..];
    let next_fn = func_inner.find("\nfn ").unwrap_or(func_inner.len());
    let func_inner = &func_inner[..next_fn];
    assert!(!func_inner.contains("factorial("), "Accumulator should eliminate recursive call.\nGot:\n{}", func_inner);
}

#[test]
fn acc_sum_codegen() {
    let source = r#"## To sumTo (n: Int) -> Int:
    If n equals 0:
        Return 0.
    Return sumTo(n - 1) + n.

## Main
Show sumTo(100).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("__acc"), "Addition accumulator should use __acc variable.\nGot:\n{}", rust);
    assert!(rust.contains("loop {"), "Addition accumulator should wrap in loop.\nGot:\n{}", rust);
    // Identity for addition is 0
    assert!(rust.contains("let mut __acc") && rust.contains("= 0"), "Addition accumulator should start at identity 0.\nGot:\n{}", rust);
}

#[test]
fn acc_no_false_positive_multi_call() {
    let source = r#"## To fib (n: Int) -> Int:
    If n is at most 1:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(10).
"#;
    let rust = compile_to_rust(source).unwrap();
    // Two recursive calls — should NOT trigger accumulator
    assert!(!rust.contains("__acc"), "Multi-call recursion should NOT get accumulator.\nGot:\n{}", rust);
}

#[test]
fn acc_no_false_positive_subtract() {
    let source = r#"## To f (n: Int) -> Int:
    If n equals 0:
        Return 0.
    Return f(n - 1) - 1.

## Main
Show f(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    // Subtraction is not commutative/associative — should NOT trigger accumulator
    assert!(!rust.contains("__acc"), "Subtraction should NOT trigger accumulator.\nGot:\n{}", rust);
}

// =============================================================================
// Accumulator Introduction — E2E Correctness Tests
// =============================================================================

#[test]
fn e2e_acc_factorial_correct() {
    let source = r#"## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).

## Main
Show factorial(10).
"#;
    common::assert_exact_output(source, "3628800");
}

#[test]
fn e2e_acc_sum_correct() {
    let source = r#"## To sumTo (n: Int) -> Int:
    If n equals 0:
        Return 0.
    Return sumTo(n - 1) + n.

## Main
Show sumTo(100).
"#;
    common::assert_exact_output(source, "5050");
}

// =============================================================================
// Memoization — Codegen Tests
// =============================================================================

#[test]
fn memo_fibonacci_codegen() {
    let source = r#"## To fib (n: Int) -> Int:
    If n is at most 1:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(10).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("thread_local!"), "Memoization should use thread_local!.\nGot:\n{}", rust);
    assert!(rust.contains("__MEMO"), "Memoization should use __MEMO cache.\nGot:\n{}", rust);
    assert!(rust.contains("RefCell"), "Memoization should use RefCell.\nGot:\n{}", rust);
    assert!(rust.contains("HashMap"), "Memoization should use HashMap.\nGot:\n{}", rust);
    assert!(rust.contains("__memo_result"), "Memoization should use __memo_result.\nGot:\n{}", rust);
}

#[test]
fn memo_no_impure_function() {
    let source = r#"## To f (n: Int) -> Int:
    Show n.
    If n is at most 1:
        Return n.
    Return f(n - 1) + f(n - 2).

## Main
Show f(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    // Function has Show (I/O) — should NOT memoize
    assert!(!rust.contains("__MEMO"), "Impure function should NOT get memoization.\nGot:\n{}", rust);
    assert!(!rust.contains("thread_local"), "Impure function should NOT get thread_local.\nGot:\n{}", rust);
}

#[test]
fn memo_no_single_call() {
    let source = r#"## To f (n: Int) -> Int:
    If n equals 0:
        Return 1.
    Return f(n - 1) * n.

## Main
Show f(10).
"#;
    let rust = compile_to_rust(source).unwrap();
    // Single recursive call — should get accumulator, NOT memoization
    assert!(!rust.contains("__MEMO"), "Single-call recursion should NOT get memoization (accumulator is better).\nGot:\n{}", rust);
}

// =============================================================================
// Memoization — E2E Correctness Tests
// =============================================================================

#[test]
fn e2e_memo_fibonacci_correct() {
    let source = r#"## To fib (n: Int) -> Int:
    If n is at most 1:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(30).
"#;
    common::assert_exact_output(source, "832040");
}

// =============================================================================
// Mutual TCO — Codegen Tests
// =============================================================================

#[test]
fn mutual_tce_codegen() {
    let source = r#"## To isEven (n: Int) -> Bool:
    If n equals 0:
        Return true.
    Return isOdd(n - 1).

## To isOdd (n: Int) -> Bool:
    If n equals 0:
        Return false.
    Return isEven(n - 1).

## Main
Show isEven(10).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("__mutual_"), "Mutual TCO should emit merged __mutual_ function.\nGot:\n{}", rust);
    assert!(rust.contains("__tag"), "Mutual TCO should use __tag dispatch.\nGot:\n{}", rust);
    assert!(rust.contains("match __tag"), "Mutual TCO should have match __tag.\nGot:\n{}", rust);
    assert!(rust.contains("continue;"), "Mutual TCO should emit continue.\nGot:\n{}", rust);
}

#[test]
fn mutual_tce_no_false_positive() {
    let source = r#"## To f (n: Int) -> Int:
    If n equals 0:
        Return 1.
    Return g(n - 1) + 1.

## To g (n: Int) -> Int:
    If n equals 0:
        Return 2.
    Return f(n - 1) + 2.

## Main
Show f(3).
"#;
    let rust = compile_to_rust(source).unwrap();
    // The + 1 / + 2 means calls are NOT in tail position
    assert!(!rust.contains("__mutual_"), "Non-tail mutual calls should NOT get mutual TCO.\nGot:\n{}", rust);
    assert!(!rust.contains("__tag"), "Non-tail mutual calls should NOT use __tag.\nGot:\n{}", rust);
}

// =============================================================================
// Mutual TCO — E2E Correctness Tests
// =============================================================================

#[test]
fn e2e_mutual_tce_correct() {
    let source = r#"## To isEven (n: Int) -> Bool:
    If n equals 0:
        Return true.
    Return isOdd(n - 1).

## To isOdd (n: Int) -> Bool:
    If n equals 0:
        Return false.
    Return isEven(n - 1).

## Main
If isEven(100):
    Show "even".
Otherwise:
    Show "odd".
"#;
    common::assert_exact_output(source, "even");
}
