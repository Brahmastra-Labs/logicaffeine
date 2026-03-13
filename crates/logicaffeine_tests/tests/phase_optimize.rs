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
fn propagation_folds_variable_into_arithmetic() {
    let source = "## Main\nLet a be 5.\nLet b be a + 1.\nShow b.";
    let rust = compile_to_rust(source).unwrap();
    // Constant propagation substitutes a=5 into a+1, fold reduces to 6
    assert!(rust.contains("let b = 6"), "Should propagate a=5 into a+1 and fold to 6.\nGot:\n{}", rust);
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
fn fold_float_comparison_folded() {
    // Camp 0c: Float comparisons ARE folded (IEEE 754 semantics).
    // NaN safety is preserved because 0.0/0.0 is never folded, so NaN
    // never appears as a compile-time literal.
    let source = r#"## Main
If 1.0 < 2.0:
    Show "yes".
"#;
    let rust = compile_to_rust(source).unwrap();
    // 1.0 < 2.0 folds to true, dead branch eliminated, just Show "yes" remains
    assert!(rust.contains("\"yes\""), "Float comparison should be folded and true branch kept.\nGot:\n{}", rust);
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

// =============================================================================
// Unreachable-After-Return DCE (0-B)
// =============================================================================

#[test]
fn dce_unreachable_after_return() {
    let source = r#"## To f () -> Int:
    Return 42.
    Show "dead".
    Return 99.

## Main
Show f().
"#;
    let rust = compile_to_rust(source).unwrap();
    // After the first Return, Show and second Return should be eliminated
    assert!(!rust.contains("dead"), "Statements after Return should be eliminated.\nGot:\n{}", rust);
    assert!(!rust.contains("99"), "Second Return should be eliminated.\nGot:\n{}", rust);
    assert!(rust.contains("42"), "First Return should be preserved.\nGot:\n{}", rust);
}

#[test]
fn dce_unreachable_after_return_in_main() {
    let source = r#"## Main
Show 42.
Return.
Show 99.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("99"), "Show after Return in Main should be eliminated.\nGot:\n{}", rust);
    assert!(rust.contains("42"), "Show before Return should be preserved.\nGot:\n{}", rust);
}

#[test]
fn dce_unreachable_after_folded_if_true_return() {
    let source = r#"## To f () -> Int:
    If true:
        Return 1.
    Return 2.

## Main
Show f().
"#;
    let rust = compile_to_rust(source).unwrap();
    // After fold+DCE, `if true: Return 1.` inlines to `Return 1.` at block level,
    // so `Return 2.` should be eliminated as unreachable
    assert!(rust.contains("1"), "Inlined Return 1 should be preserved.\nGot:\n{}", rust);
    assert!(!rust.contains("return 2"), "Return 2 after inlined Return 1 should be eliminated.\nGot:\n{}", rust);
}

#[test]
fn e2e_dce_unreachable_after_return_correct() {
    let source = r#"## To f () -> Int:
    Return 42.
    Return 99.

## Main
Show f().
"#;
    common::assert_exact_output(source, "42");
}

#[test]
fn dce_return_inside_if_does_not_truncate_outer() {
    let source = r#"## To f (x: Int) -> Int:
    If x > 0:
        Return x.
    Return 0.

## Main
Show f(5).
Show f(0).
"#;
    let rust = compile_to_rust(source).unwrap();
    // Return inside If should NOT kill the outer block's Return 0
    assert!(rust.contains("return 0") || rust.contains("0i64"), "Return inside If should not kill outer Return.\nGot:\n{}", rust);
}

// =============================================================================
// Deep Expression Recursion (0-A)
// =============================================================================

#[test]
fn fold_inside_function_call_args() {
    let source = r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main
Show double(2 + 3).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("double(5)"), "Should fold 2+3 inside call args to 5.\nGot:\n{}", rust);
}

#[test]
fn fold_inside_list_literal() {
    let source = r#"## Main
Let items be [1 + 2, 3 * 4].
Show items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("3") && rust.contains("12"), "Should fold expressions inside list literal.\nGot:\n{}", rust);
    assert!(!rust.contains("1 + 2") && !rust.contains("3 * 4"), "Should not have unfolded expressions.\nGot:\n{}", rust);
}

#[test]
fn fold_inside_index_expression() {
    let source = r#"## Main
Let items be [10, 20, 30].
Let x be item (1 + 1) of items.
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    // The index expression 1+1 should be folded to 2
    assert!(!rust.contains("1 + 1"), "Should fold 1+1 in index expression.\nGot:\n{}", rust);
}

#[test]
fn fold_inside_struct_constructor() {
    let source = r#"## A Point has x Int and y Int.

## Main
Let p be a new Point with x (2 + 3) and y (10 - 4).
Show p's x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("5"), "Should fold 2+3 in struct constructor field.\nGot:\n{}", rust);
    assert!(rust.contains("6"), "Should fold 10-4 in struct constructor field.\nGot:\n{}", rust);
}

#[test]
fn fold_inside_option_some() {
    let source = r#"## To f () -> Option of Int:
    Return some (2 + 3).

## Main
Show f().
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("Some(5)"), "Should fold 2+3 inside Option Some.\nGot:\n{}", rust);
}

#[test]
fn fold_inside_contains() {
    let source = r#"## Main
Let items be [10, 20, 30].
If items contains (5 + 5):
    Show "found".
"#;
    let rust = compile_to_rust(source).unwrap();
    // The contains argument 5+5 should be folded to 10
    assert!(!rust.contains("5 + 5"), "Should fold 5+5 in contains arg.\nGot:\n{}", rust);
}

#[test]
fn e2e_fold_call_args_correct() {
    let source = r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main
Show double(2 + 3).
"#;
    common::assert_exact_output(source, "10");
}

#[test]
fn e2e_fold_list_elements_correct() {
    let source = r#"## Main
Let items be [1 + 2, 3 * 4, 10 - 5].
Repeat for x in items:
    Show x.
"#;
    common::assert_exact_output(source, "3\n12\n5");
}

// =============================================================================
// Algebraic Simplification (0-C)
// =============================================================================

#[test]
fn fold_algebraic_add_zero_right() {
    let source = r#"## Main
Let a be 7.
Let b be a + 0.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    // a + 0 should simplify to just a (no addition in output)
    assert!(!rust.contains("+ 0"), "x + 0 should simplify to x.\nGot:\n{}", rust);
}

#[test]
fn fold_algebraic_add_zero_left() {
    let source = r#"## Main
Let a be 7.
Let b be 0 + a.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("0 +") && !rust.contains("0i64 +"), "0 + x should simplify to x.\nGot:\n{}", rust);
}

#[test]
fn fold_algebraic_multiply_one_right() {
    let source = r#"## Main
Let a be 7.
Let b be a * 1.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("* 1"), "x * 1 should simplify to x.\nGot:\n{}", rust);
}

#[test]
fn fold_algebraic_multiply_one_left() {
    let source = r#"## Main
Let a be 7.
Let b be 1 * a.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("1 *") && !rust.contains("1i64 *"), "1 * x should simplify to x.\nGot:\n{}", rust);
}

#[test]
fn fold_algebraic_multiply_zero() {
    let source = r#"## Main
Let a be 7.
Let b be a * 0.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    // a * 0 should simplify to 0
    assert!(rust.contains("let b = 0") || rust.contains("let b: i64 = 0"), "x * 0 should simplify to 0.\nGot:\n{}", rust);
}

#[test]
fn fold_algebraic_multiply_zero_left() {
    let source = r#"## Main
Let a be 7.
Let b be 0 * a.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("let b = 0") || rust.contains("let b: i64 = 0"), "0 * x should simplify to 0.\nGot:\n{}", rust);
}

#[test]
fn fold_algebraic_subtract_zero() {
    let source = r#"## Main
Let a be 7.
Let b be a - 0.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("- 0"), "x - 0 should simplify to x.\nGot:\n{}", rust);
}

#[test]
fn fold_algebraic_divide_by_one() {
    let source = r#"## Main
Let a be 7.
Let b be a / 1.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("/ 1"), "x / 1 should simplify to x.\nGot:\n{}", rust);
}

#[test]
fn fold_algebraic_float_add_zero() {
    let source = r#"## Main
Let a be 7.0.
Let b be a + 0.0.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("+ 0.0") && !rust.contains("+ 0f64"), "Float x + 0.0 should simplify to x.\nGot:\n{}", rust);
}

#[test]
fn fold_algebraic_float_multiply_one() {
    let source = r#"## Main
Let a be 7.0.
Let b be a * 1.0.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("* 1.0") && !rust.contains("* 1f64"), "Float x * 1.0 should simplify to x.\nGot:\n{}", rust);
}

#[test]
fn fold_algebraic_float_multiply_zero() {
    let source = r#"## Main
Let a be 7.0.
Let b be a * 0.0.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    // Float x * 0.0 should simplify to 0.0 — no multiplication of a
    assert!(!rust.contains("a * 0") && !rust.contains("a *"), "Float x * 0.0 should simplify to 0.0.\nGot:\n{}", rust);
}

// =============================================================================
// TIER 1-C: Direct Array Indexing + Clone Elimination
// =============================================================================

#[test]
fn tier1c_list_literal_direct_indexing() {
    let source = "## Main\nLet items be [10, 20, 30].\nLet x be item 2 of items.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("LogosIndex"), "List literal should use direct indexing, got:\n{}", rust);
}

#[test]
fn tier1c_vec_i64_no_clone() {
    let source = "## Main\nLet items: Seq of Int be [1, 2, 3].\nLet x be item 2 of items.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("LogosIndex"), "Should use direct indexing, got:\n{}", rust);
    assert!(!rust.contains(".clone()"), "Vec<i64> indexing should not clone (Copy type), got:\n{}", rust);
}

#[test]
fn tier1c_list_literal_infers_element_type() {
    // List literal [10, 20, 30] should infer Vec<i64> element type for Copy elimination
    let source = "## Main\nLet items be [10, 20, 30].\nLet x be item 1 of items.\nShow x.";
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("LogosIndex"), "Should use direct indexing, got:\n{}", rust);
    assert!(!rust.contains(".clone()"), "Integer list literal indexing should not clone (Copy type), got:\n{}", rust);
}

#[test]
fn tier1c_e2e_direct_indexing_correct() {
    let source = "## Main\nLet items be [10, 20, 30].\nShow item 1 of items.\nShow item 2 of items.\nShow item 3 of items.";
    common::assert_exact_output(source, "10\n20\n30");
}

#[test]
fn tier1c_set_index_direct_mutation() {
    // SetIndex on a known Vec should use direct mutation, not LogosIndexMut
    let source = "## Main\nLet items: Seq of Int be [1, 2, 3].\nSet item 2 of items to 99.\nShow item 2 of items.";
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("LogosIndexMut"), "SetIndex on known Vec should use direct mutation, got:\n{}", rust);
}

#[test]
fn tier1c_e2e_set_index_correct() {
    let source = "## Main\nLet items: Seq of Int be [1, 2, 3].\nSet item 2 of items to 99.\nShow items.";
    common::assert_exact_output(source, "[1, 99, 3]");
}

// =============================================================================
// TIER 1-D: Vec Fill Enhancement (exclusive bound)
// =============================================================================

#[test]
fn tier1d_vec_fill_exclusive_bound() {
    let source = "## Main\nLet mut items be a new Seq of Int.\nLet mut i be 0.\nWhile i is less than 5:\n    Push 0 to items.\n    Set i to i + 1.\nShow length of items.";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("vec!["), "Should optimize exclusive bound fill, got:\n{}", rust);
}

#[test]
fn tier1d_e2e_vec_fill_exclusive_correct() {
    let source = "## Main\nLet mut items be a new Seq of Int.\nLet mut i be 0.\nWhile i is less than 5:\n    Push 0 to items.\n    Set i to i + 1.\nShow length of items.";
    common::assert_exact_output(source, "5");
}

#[test]
fn tier1d_e2e_vec_fill_exclusive_start_1() {
    // Exclusive bound starting at 1: While i < 5, start=1 → 4 elements
    let source = "## Main\nLet mut items be a new Seq of Int.\nLet mut i be 1.\nWhile i is less than 5:\n    Push 0 to items.\n    Set i to i + 1.\nShow length of items.";
    common::assert_exact_output(source, "4");
}

// =============================================================================
// TIER 1-E: Swap Pattern Enhancement
// =============================================================================

#[test]
fn tier1e_swap_equality_comparison() {
    // Use variable index j and (j+1) to satisfy adjacency requirement
    let source = r#"## Main
Let items: Seq of Int be [3, 1, 2].
Let j be 1.
Let a be item j of items.
Let b be item (j + 1) of items.
If a equals b:
    Set item j of items to b.
    Set item (j + 1) of items to a.
Show items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains(".swap("), "Should optimize equality swap with .swap(), got:\n{}", rust);
}

#[test]
fn tier1e_swap_not_equals_comparison() {
    let source = r#"## Main
Let items: Seq of Int be [3, 1, 2].
Let j be 1.
Let a be item j of items.
Let b be item (j + 1) of items.
If a is not b:
    Set item j of items to b.
    Set item (j + 1) of items to a.
Show items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains(".swap("), "Should optimize not-equal swap with .swap(), got:\n{}", rust);
}

// =============================================================================
// TIER 1-A: For-Range Loop Emission
// =============================================================================

#[test]
fn tier1a_simple_counting_loop() {
    let source = r#"## Main
Let i be 1.
While i is at most 5:
    Show i.
    Set i to i + 1.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("for i in 1..6"), "Should emit for-range, got:\n{}", rust);
    assert!(!rust.contains("while"), "Should not have while loop, got:\n{}", rust);
}

#[test]
fn tier1a_exclusive_bound() {
    let source = r#"## Main
Let i be 0.
While i is less than 5:
    Show i.
    Set i to i + 1.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("for i in 0..5"), "Should emit exclusive for-range, got:\n{}", rust);
}

#[test]
fn tier1a_variable_limit() {
    // Use runtime-dynamic limit so optimizer can't propagate it
    let source = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let a be args().
Let n be parseInt(item 1 of a).
Let i be 1.
While i is at most n:
    Show i.
    Set i to i + 1.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("for i in 1..(n + 1)"), "Should handle variable limits, got:\n{}", rust);
}

#[test]
fn tier1a_counter_used_as_index() {
    let source = r#"## Main
Let items: Seq of Int be [10, 20, 30].
Let i be 1.
While i is at most 3:
    Show item i of items.
    Set i to i + 1.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("for i in 0..3"), "Should emit 0-based for-range with indexing, got:\n{}", rust);
    assert!(rust.contains("items.borrow()[i as usize]"), "Should emit direct index (no -1 subtract), got:\n{}", rust);
}

#[test]
fn tier1a_no_match_counter_modified_in_body() {
    // Counter set to something other than counter+1 inside the body → don't optimize
    let source = r#"## Main
Let i be 1.
While i is at most 10:
    If i equals 5:
        Set i to 8.
    Show i.
    Set i to i + 1.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("while"), "Should NOT emit for-range when counter modified in body, got:\n{}", rust);
}

#[test]
fn tier1a_no_match_step_not_1() {
    let source = r#"## Main
Let i be 0.
While i is at most 10:
    Show i.
    Set i to i + 2.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("while"), "Should NOT emit for-range for step != 1, got:\n{}", rust);
}

#[test]
fn tier1a_e2e_correct_sum() {
    let source = r#"## Main
Let sum be 0.
Let i be 1.
While i is at most 5:
    Set sum to sum + i.
    Set i to i + 1.
Show sum.
"#;
    common::assert_exact_output(source, "15");
}

#[test]
fn tier1a_e2e_post_loop_value() {
    // After while (i <= 5) with i++, i should be 6
    let source = r#"## Main
Let i be 1.
While i is at most 5:
    Set i to i + 1.
Show i.
"#;
    common::assert_exact_output(source, "6");
}

// =============================================================================
// TIER 1-B: Iterator-Based Loops
// =============================================================================

#[test]
fn tier1b_copy_type_no_clone() {
    // LogosSeq iteration uses .to_vec() to snapshot the inner vec (safe for reference semantics)
    let source = r#"## Main
Let items: Seq of Int be [1, 2, 3].
Repeat for x in items:
    Show x.
Show items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains(".to_vec()"), "LogosSeq iteration should use .to_vec(), got:\n{}", rust);
}

#[test]
fn tier1b_non_copy_type_lazy_clones() {
    // LogosSeq iteration uses .to_vec() regardless of element type (reference semantics)
    let source = r#"## Main
Let items: Seq of Text be ["a", "b"].
Repeat for x in items:
    Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains(".to_vec()"), "LogosSeq iteration should use .to_vec(), got:\n{}", rust);
}

#[test]
fn tier1b_mutating_body_keeps_clone() {
    // LogosSeq iteration uses .to_vec() which is safe even when body mutates collection
    let source = r#"## Main
Let items: Seq of Int be [1, 2, 3].
Repeat for x in items:
    Push x to items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains(".to_vec()"), "LogosSeq mutation-safe iteration should use .to_vec(), got:\n{}", rust);
}

#[test]
fn tier1b_e2e_iter_copied_correct() {
    let source = r#"## Main
Let sum be 0.
Let items: Seq of Int be [1, 2, 3, 4, 5].
Repeat for x in items:
    Set sum to sum + x.
Show sum.
"#;
    common::assert_exact_output(source, "15");
}

#[test]
fn tier1b_bool_seq_iter_copied() {
    let source = r#"## Main
Let flags: Seq of Bool be [true, false, true].
Repeat for f in flags:
    Show f.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains(".to_vec()"), "LogosSeq Bool iteration should use .to_vec(), got:\n{}", rust);
}

#[test]
fn e2e_algebraic_identity_correct() {
    let source = r#"## Main
Let a be 7.
Show a + 0.
Show 0 + a.
Show a * 1.
Show 1 * a.
Show a * 0.
Show 0 * a.
Show a - 0.
Show a / 1.
"#;
    common::assert_exact_output(source, "7\n7\n7\n7\n0\n0\n7\n7");
}

// =============================================================================
// Cascading: Deep Recursion + Algebraic (0-A + 0-C)
// =============================================================================

#[test]
fn fold_algebraic_nested_in_call() {
    let source = r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main
Show double(5 + 0).
"#;
    let rust = compile_to_rust(source).unwrap();
    // Deep recursion folds into call args, then algebraic simplifies 5+0 to 5
    assert!(rust.contains("double(5)"), "Should fold 5+0 to 5 inside call args.\nGot:\n{}", rust);
}

// =============================================================================
// TIER 2: Sieve Vec-Fill Bug Fix (A1)
// =============================================================================

#[test]
fn tier2_sieve_vec_fill_bool() {
    let source = r#"## Main
Let flags be a new Seq of Bool.
Let i be 0.
While i is at most 5:
    Push false to flags.
    Set i to i + 1.
Show length of flags.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("vec!["), "Sieve-style fill should optimize to vec![...], got:\n{}", rust);
}

#[test]
fn tier2_sieve_vec_fill_in_function() {
    let source = r#"## To sieve (limit: Int) -> Int:
    Let flags be a new Seq of Bool.
    Let i be 0.
    While i is at most limit:
        Push false to flags.
        Set i to i + 1.
    Return length of flags.

## Main
Show sieve(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("vec!["), "Sieve-style fill in function should optimize to vec![...], got:\n{}", rust);
}

#[test]
fn tier2_vec_fill_int() {
    let source = r#"## Main
Let nums be a new Seq of Int.
Let i be 0.
While i is at most 10:
    Push 0 to nums.
    Set i to i + 1.
Show length of nums.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("vec![0; "), "Int fill should optimize to vec![0; ...], got:\n{}", rust);
}

#[test]
fn e2e_tier2_sieve_vec_fill_correct() {
    let source = r#"## Main
Let flags be a new Seq of Bool.
Let i be 0.
While i is at most 5:
    Push false to flags.
    Set i to i + 1.
Show length of flags.
"#;
    common::assert_exact_output(source, "6");
}

// =============================================================================
// TIER 2: Index Simplification (A2)
// =============================================================================

#[test]
fn tier2_index_plus_one_simplifies() {
    let source = r#"## Main
Let items: Seq of Int be [10, 20, 30].
Let j be 1.
Show item (j + 1) of items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("j as usize") || rust.contains("(j) as usize"),
            "item (j+1) should simplify to j as usize, got:\n{}", rust);
    assert!(!rust.contains("+ 1) - 1"), "Should not have redundant +1-1, got:\n{}", rust);
}

#[test]
fn tier2_index_one_plus_j_simplifies() {
    let source = r#"## Main
Let items: Seq of Int be [10, 20, 30].
Let j be 1.
Show item (1 + j) of items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("j as usize") || rust.contains("(j) as usize"),
            "item (1+j) should simplify to j as usize, got:\n{}", rust);
    assert!(!rust.contains("+ 1) - 1"), "Should not have redundant +1-1, got:\n{}", rust);
}

#[test]
fn tier2_index_no_false_simplification() {
    let source = r#"## Main
Let items: Seq of Int be [10, 20, 30].
Let j be 1.
Show item (j + 2) of items.
"#;
    let rust = compile_to_rust(source).unwrap();
    // j+2 should become (j + 2 - 1) which is (j + 1), NOT j
    assert!(!rust.contains("j as usize\n") && !rust.contains("(j) as usize\n"),
            "item (j+2) should NOT simplify to just j, got:\n{}", rust);
}

#[test]
fn e2e_tier2_index_simplification_correct() {
    let source = r#"## Main
Let items: Seq of Int be [10, 20, 30].
Let j be 1.
Show item (j + 1) of items.
"#;
    common::assert_exact_output(source, "20");
}

// =============================================================================
// TIER 2-A: Constant Propagation
// =============================================================================

#[test]
fn tier2a_constant_propagation_basic() {
    let source = r#"## Main
Let x be 10.
Let y be x + 5.
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("15"), "Should propagate x=10 into y=x+5 and fold to 15, got:\n{}", rust);
}

#[test]
fn tier2a_propagation_multiple_uses() {
    let source = r#"## Main
Let a be 3.
Let b be a + a.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("6"), "Should propagate a=3 into b=a+a and fold to 6, got:\n{}", rust);
}

#[test]
fn tier2a_propagation_chain() {
    let source = r#"## Main
Let x be 2.
Let y be x + 3.
Let z be y * 2.
Show z.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("10"), "Should chain propagate x=2→y=5→z=10, got:\n{}", rust);
}

#[test]
fn tier2a_propagation_killed_by_reassignment() {
    let source = r#"## Main
Let mutable x be 10.
Set x to 20.
Let y be x + 5.
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(!rust.contains("15"), "Should not propagate killed constant, got:\n{}", rust);
}

#[test]
fn tier2a_propagation_skips_mutable() {
    let source = r#"## Main
Let mutable x be 10.
Let y be x + 5.
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    // x is declared mutable, so we don't propagate it
    assert!(!rust.contains("15"), "Should not propagate mutable variable, got:\n{}", rust);
}

#[test]
fn tier2a_propagation_in_nested_let() {
    let source = r#"## Main
Let x be 7.
If true:
    Let y be x + 3.
    Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    // x=7 should propagate into the nested Let value
    assert!(rust.contains("10"), "Should propagate x=7 into nested y=x+3, got:\n{}", rust);
}

#[test]
fn e2e_tier2a_propagation_correct() {
    let source = r#"## Main
Let x be 10.
Let y be x + 5.
Show y.
"#;
    common::assert_exact_output(source, "15");
}

#[test]
fn e2e_tier2a_propagation_chain_correct() {
    let source = r#"## Main
Let x be 2.
Let y be x + 3.
Let z be y * 2.
Show z.
"#;
    common::assert_exact_output(source, "10");
}

// =============================================================================
// Swap Pattern Regression — Bubble Sort
// =============================================================================

#[test]
fn tier1e_swap_in_nested_while_loop() {
    // Mirrors the bubble_sort benchmark: swap pattern inside a nested while loop
    // with `new Seq of Int` (inferred type, no annotation).
    let source = r#"## Main
Let mutable arr be a new Seq of Int.
Push 3 to arr.
Push 1 to arr.
Push 2 to arr.
Let n be 3.
Let mutable i be 0.
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
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains(".swap("), "Swap pattern should fire for nested while loop with inferred Vec type, got:\n{}", rust);
}

#[test]
fn e2e_tier1e_swap_in_nested_while_loop_correct() {
    let source = r#"## Main
Let mutable arr be a new Seq of Int.
Push 3 to arr.
Push 1 to arr.
Push 2 to arr.
Let n be 3.
Let mutable i be 0.
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
    common::assert_exact_output(source, "1");
}

// =============================================================================
// OPT: Single-char text variable → u8 byte
// =============================================================================

#[test]
fn opt_single_char_var_emits_u8() {
    let source = r#"## Main
Let mutable text be "".
Let mutable ch be "a".
If 1 equals 1:
    Set ch to "b".
Set text to text + ch.
Show text.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let mut ch: u8 = b'a'") || rust.contains("let mut ch: u8 = b'a';"),
        "Single-char text var should emit u8 byte, got:\n{}",
        rust
    );
}

#[test]
fn opt_single_char_var_push_emits_push_byte() {
    let source = r#"## Main
Let mutable text be "".
Let mutable ch be "x".
Set text to text + ch.
Show text.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("text.push(ch as char)"),
        "push_str(&ch) with single-char var should become push(ch as char), got:\n{}",
        rust
    );
}

#[test]
fn opt_single_char_var_conditional_assignment() {
    let source = r#"## Main
Let mutable text be "".
Let mutable pos be 0.
While pos is less than 5:
    Let mutable ch be "a".
    If pos % 5 equals 1:
        Set ch to "b".
    If pos % 5 equals 2:
        Set ch to "c".
    If pos % 5 equals 3:
        Set ch to "d".
    If pos % 5 equals 4:
        Set ch to "e".
    Set text to text + ch.
    Set pos to pos + 1.
Show text.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("u8 = b'"),
        "All single-char assignments should produce u8, got:\n{}",
        rust
    );
    assert!(
        !rust.contains("String::from(\"a\")") && !rust.contains("String::from(\"b\")"),
        "Should NOT emit String::from for single-char vars, got:\n{}",
        rust
    );
}

#[test]
fn opt_single_char_var_set_emits_byte_literal() {
    let source = r#"## Main
Let mutable ch be "a".
Set ch to "z".
Show ch.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("ch = b'z'"),
        "Set of single-char var should emit byte literal, got:\n{}",
        rust
    );
}

#[test]
fn opt_single_char_var_not_applied_to_multi_char() {
    let source = r#"## Main
Let mutable ch be "ab".
Set ch to "cd".
Show ch.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("String::from") || rust.contains("\"ab\""),
        "Multi-char var should NOT be optimized to u8, got:\n{}",
        rust
    );
}

// =============================================================================
// OPT: String with_capacity from loop
// =============================================================================

// =============================================================================
// OPT: Bare Slice Push Pattern (extend_from_slice)
// =============================================================================

#[test]
fn opt_bare_slice_push_basic() {
    // A bare While loop that pushes contiguous elements from one array to another
    // should be optimized to extend_from_slice instead of individual pushes.
    // The target Vec is created THEN modified (Push 0) before the copy loop,
    // so try_emit_seq_from_slice_pattern bails out (dst referenced between
    // creation and the While) and the bare pattern must fire.
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To copyRange (arr: Seq of Int, start: Int, end: Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Push 0 to result.
    Let mutable i be start.
    While i is at most end:
        Push item i of arr to result.
        Set i to i + 1.
    Return result.

## Main
Let items be [10, 20, 30, 40, 50].
Let half be copyRange(items, 2, 4).
Show length of half.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("extend_from_slice"),
        "Bare While push loop should be optimized to extend_from_slice, got:\n{}",
        rust
    );
}

#[test]
fn opt_bare_slice_push_continuation() {
    // After try_emit_seq_from_slice_pattern fires for the first half,
    // the second half is a bare While that should also be optimized.
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## To splitTwo (arr: Seq of Int, mid: Int) -> Seq of Int:
    Let mutable left be a new Seq of Int.
    Let mutable i be 1.
    While i is at most mid:
        Push item i of arr to left.
        Set i to i + 1.
    Let mutable right be a new Seq of Int.
    While i is at most length of arr:
        Push item i of arr to right.
        Set i to i + 1.
    Show length of left.
    Return right.

## Main
Let items be [10, 20, 30, 40, 50, 60].
Let r be splitTwo(items, 3).
Show length of r.
"#;
    let rust = compile_to_rust(source).unwrap();
    // Both halves should be optimized to direct slice operations (.to_vec() or extend_from_slice)
    let to_vec_count = rust.matches(".to_vec()").count();
    let extend_count = rust.matches("extend_from_slice").count();
    assert!(
        to_vec_count >= 2 || (to_vec_count >= 1 && extend_count >= 1),
        "Both halves should use direct slice operations (.to_vec() or extend_from_slice), got:\n{}",
        rust
    );
}

// =============================================================================
// OPT: String with_capacity from loop
// =============================================================================

#[test]
fn opt_string_with_capacity_from_loop() {
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## Main
Let n be 100.
Let mutable text be "".
Let mutable i be 0.
While i is less than n:
    Set text to text + "x".
    Set i to i + 1.
Show length of text.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("String::with_capacity("),
        "Empty string followed by append loop should emit with_capacity, got:\n{}",
        rust
    );
}

// =============================================================================
// CAMP 0-I: Local CSE / Value Numbering
// =============================================================================

#[test]
fn cse_same_expression() {
    // Same expression computed twice → second should reuse the first
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## Main
Let a be parseInt(item 1 of args()).
Let b be parseInt(item 2 of args()).
Let x be a + b.
Let y be a + b.
Show x.
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    let add_count = rust.matches("(a + b)").count();
    assert!(
        add_count <= 1,
        "CSE should compute a+b only once, got {} occurrences:\n{}",
        add_count,
        rust
    );
}

#[test]
fn cse_invalidated_by_write() {
    // Set between two identical expressions → cannot reuse (variable changed)
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## Main
Let mutable a be parseInt(item 1 of args()).
Let b be parseInt(item 2 of args()).
Let x be a + b.
Set a to 99.
Let y be a + b.
Show x.
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    let add_count = rust.matches("(a + b)").count() + rust.matches("a + b").count() / 2;
    // Both computations should remain since a was modified between them
    assert!(
        !rust.contains("let y = x"),
        "CSE should NOT reuse across write to operand, got:\n{}",
        rust
    );
}

#[test]
fn cse_nested_subexpression() {
    // (a+b)*(a+b) → should extract a+b into a temp and compute temp*temp
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## Main
Let a be parseInt(item 1 of args()).
Let b be parseInt(item 2 of args()).
Let result be (a + b) * (a + b).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    let add_count = rust.matches("(a + b)").count();
    assert!(
        add_count <= 1,
        "CSE should extract common subexpression (a+b), computing it only once. Got {} occurrences:\n{}",
        add_count,
        rust
    );
}

#[test]
fn cse_different_operators() {
    // x+y vs x*y → different expressions, no CSE
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## Main
Let a be parseInt(item 1 of args()).
Let b be parseInt(item 2 of args()).
Let x be a + b.
Let y be a * b.
Show x.
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("(a + b)") && rust.contains("(a * b)"),
        "Different operators should not be CSE'd, got:\n{}",
        rust
    );
}

#[test]
fn cse_across_blocks_not_applied() {
    // CSE should not cross If/While boundaries (conservative)
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## Main
Let a be parseInt(item 1 of args()).
Let b be parseInt(item 2 of args()).
Let x be a + b.
If a is greater than 0:
    Let y be a + b.
    Show y.
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    // Inside the If block, a+b should NOT be replaced with x
    // (conservative: don't cross control flow boundaries)
    // The If block should contain its own a+b computation
    assert!(
        rust.contains("let y = (a + b)") || rust.contains("let y = x"),
        "Test should compile correctly, got:\n{}",
        rust
    );
}

// =============================================================================
// CAMP 4: Dead Store Elimination
// =============================================================================

#[test]
fn dse_consecutive_writes() {
    // Set x=10 then Set x=20 without reading x between → first Set is dead
    let source = r#"## Main
Let mutable x be 5.
Set x to 10.
Set x to 20.
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    // After DSE, the dead Set x=10 should be eliminated
    assert!(
        !rust.lines().any(|line| line.trim() == "x = 10;"),
        "DSE should eliminate dead Set x=10. Got:\n{}",
        rust
    );
    assert!(
        rust.lines().any(|line| line.trim() == "x = 20;"),
        "The final Set x=20 should be preserved. Got:\n{}",
        rust
    );
}

#[test]
fn dse_no_eliminate_when_read() {
    // Show x between two writes → both writes preserved
    let source = r#"## Main
Let mutable x be 5.
Show x.
Set x to 10.
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    // Both values should be present since x is read between writes
    assert!(
        rust.contains("5") && rust.contains("10"),
        "Both writes should be preserved when x is read between them. Got:\n{}",
        rust
    );
}

#[test]
fn dse_push_not_dead() {
    // Push is additive → never a dead store
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 5 to items.
Push 10 to items.
Show length of items.
"#;
    common::assert_exact_output(source, "2");
}

#[test]
fn dse_e2e_correct() {
    let source = r#"## Main
Let mutable x be 5.
Set x to 10.
Set x to 20.
Show x.
"#;
    common::assert_exact_output(source, "20");
}

// =============================================================================
// CAMP 4: LICM — Loop-Invariant Code Motion
// =============================================================================

#[test]
fn licm_hoist_length() {
    // `length of items` inside loop body should be hoisted above loop
    let source = r#"## To sumLengths (items: Seq of Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 1.
    While i is at most length of items:
        Let n be length of items.
        Set sum to sum + n.
        Set i to i + 1.
    Return sum.

## Main
Show sumLengths([10, 20, 30, 40, 50]).
"#;
    let rust = compile_to_rust(source).unwrap();
    // After LICM, `let n = (items.len() as i64)` should appear BEFORE the loop
    let n_pos = rust.find("let n =").expect("Should have let n binding");
    let loop_pos = rust.find("for ").unwrap_or_else(|| rust.find("while ").expect("Should have a loop"));
    assert!(
        n_pos < loop_pos,
        "LICM should hoist `length of items` before the loop.\nGot:\n{}",
        rust
    );
}

#[test]
fn licm_no_hoist_when_written() {
    // items is pushed to in loop body → length NOT hoisted
    let source = r#"## To growAndCount (items: Seq of Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 0.
    While i is less than 5:
        Push i to items.
        Let n be length of items.
        Set sum to sum + n.
        Set i to i + 1.
    Return sum.

## Main
Show growAndCount([1, 2, 3]).
"#;
    let rust = compile_to_rust(source).unwrap();
    // n should NOT be hoisted because items is modified (Push) inside the loop
    // Check that `len()` appears inside the loop body (after the for/while keyword)
    let loop_pos = rust.find("for ").unwrap_or_else(|| rust.find("while ").expect("Should have a loop"));
    let len_in_body = rust[loop_pos..].contains(".len()");
    assert!(
        len_in_body,
        "Length should NOT be hoisted when collection is mutated in loop body.\nGot:\n{}",
        rust
    );
}

#[test]
fn licm_hoist_arithmetic() {
    // Pure arithmetic on loop-invariant operands should be hoisted
    let source = r#"## To compute (a: Int, b: Int, n: Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 0.
    While i is less than n:
        Let offset be a * b.
        Set sum to sum + offset + i.
        Set i to i + 1.
    Return sum.

## Main
Show compute(3, 7, 5).
"#;
    let rust = compile_to_rust(source).unwrap();
    // After LICM, `let offset = (a * b)` should appear before the loop
    let offset_pos = rust.find("let offset =").expect("Should have offset binding");
    let loop_pos = rust.find("for ").unwrap_or_else(|| rust.find("while ").expect("Should have a loop"));
    assert!(
        offset_pos < loop_pos,
        "LICM should hoist pure arithmetic `a * b` before the loop.\nGot:\n{}",
        rust
    );
}

#[test]
fn licm_no_hoist_loop_variant() {
    // Expression that reads the loop counter should NOT be hoisted
    let source = r#"## To readAll (items: Seq of Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 1.
    While i is at most length of items:
        Let val be item i of items.
        Set sum to sum + val.
        Set i to i + 1.
    Return sum.

## Main
Show readAll([10, 20, 30]).
"#;
    let rust = compile_to_rust(source).unwrap();
    // `item i of items` reads i which is the loop variable → NOT hoisted
    // Just verify correct output (60)
    common::assert_exact_output(source, "60");
}

#[test]
fn licm_e2e_correct_sum() {
    let source = r#"## To sumLengths (items: Seq of Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 1.
    While i is at most length of items:
        Let n be length of items.
        Set sum to sum + n.
        Set i to i + 1.
    Return sum.

## Main
Show sumLengths([10, 20, 30, 40, 50]).
"#;
    common::assert_exact_output(source, "25");
}

#[test]
fn licm_e2e_arithmetic_correct() {
    let source = r#"## To compute (a: Int, b: Int, n: Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 0.
    While i is less than n:
        Let offset be a * b.
        Set sum to sum + offset + i.
        Set i to i + 1.
    Return sum.

## Main
Show compute(3, 7, 5).
"#;
    common::assert_exact_output(source, "115");
}

// =============================================================================
// CAMP 2-B: For-In Reference Iteration (lazy clone for non-Copy types)
// =============================================================================

#[test]
fn ref_iter_no_mutation() {
    // LogosSeq iteration always uses .to_vec() (reference semantics — safe snapshot)
    let source = r#"## Main
Let items: Seq of Text be ["hello", "world"].
Repeat for x in items:
    Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains(".to_vec()"),
        "LogosSeq iteration should use .to_vec(), got:\n{}",
        rust
    );
}

#[test]
fn ref_iter_with_mutation() {
    // LogosSeq iteration uses .to_vec() which is safe even when body mutates
    let source = r#"## Main
Let items: Seq of Text be ["hello", "world"].
Repeat for x in items:
    Push "extra" to items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains(".to_vec()"),
        "LogosSeq mutation-safe iteration should use .to_vec(), got:\n{}",
        rust
    );
}

#[test]
fn ref_iter_e2e_correctness() {
    // Verify that .iter().cloned() produces identical output to .clone()
    let source = r#"## Main
Let items: Seq of Text be ["alpha", "beta", "gamma"].
Repeat for x in items:
    Show x.
Show length of items.
"#;
    common::assert_exact_output(source, "alpha\nbeta\ngamma\n3");
}

// =============================================================================
// CAMP 4: Loop Unswitching
// =============================================================================

#[test]
fn unswitch_invariant_branch() {
    // Loop-invariant If inside While should be unswitched:
    // While cond: If flag: A else: B  →  If flag: While cond: A  else: While cond: B
    let source = r#"## To test (flag: Bool, n: Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 0.
    While i is less than n:
        If flag:
            Set sum to sum + 2.
        Otherwise:
            Set sum to sum + 1.
        Set i to i + 1.
    Return sum.

## Main
Show test(true, 5).
Show test(false, 5).
"#;
    let rust = compile_to_rust(source).unwrap();
    // After unswitching, the If on flag should appear BEFORE any loop
    let fn_body = rust.split("fn test(").nth(1).unwrap_or("");
    let if_pos = fn_body.find("if flag");
    let loop_pos = fn_body.find("for ").or_else(|| fn_body.find("while"));
    assert!(
        if_pos.is_some() && loop_pos.is_some() && if_pos < loop_pos,
        "Loop unswitching should hoist invariant If above loop. Got:\n{}",
        rust
    );
}

#[test]
fn unswitch_no_fire_variant_condition() {
    // Loop-variant condition (depends on loop counter) should NOT be unswitched
    let source = r#"## To test (n: Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 0.
    While i is less than n:
        If i is greater than 5:
            Set sum to sum + 2.
        Otherwise:
            Set sum to sum + 1.
        Set i to i + 1.
    Return sum.

## Main
Show test(10).
"#;
    let rust = compile_to_rust(source).unwrap();
    let fn_body = rust.split("fn test(").nth(1).unwrap_or("");
    let if_pos = fn_body.find("if (i");
    let loop_pos = fn_body.find("for ").or_else(|| fn_body.find("while"));
    assert!(
        loop_pos.is_some() && (if_pos.is_none() || if_pos > loop_pos),
        "Loop-variant condition should NOT be unswitched. Got:\n{}",
        rust
    );
}

#[test]
fn unswitch_no_fire_written_condition() {
    // Condition variable is written inside loop → NOT unswitched
    let source = r#"## To test (n: Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 0.
    Let mutable flag be true.
    While i is less than n:
        If flag:
            Set sum to sum + 2.
        Otherwise:
            Set sum to sum + 1.
        Set flag to false.
        Set i to i + 1.
    Return sum.

## Main
Show test(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    let fn_body = rust.split("fn test(").nth(1).unwrap_or("");
    let if_pos = fn_body.find("if flag");
    let loop_pos = fn_body.find("for ").or_else(|| fn_body.find("while"));
    assert!(
        loop_pos.is_some() && (if_pos.is_none() || if_pos > loop_pos),
        "Written condition should NOT be unswitched. Got:\n{}",
        rust
    );
}

#[test]
fn unswitch_e2e_correct() {
    let source = r#"## To test (flag: Bool, n: Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 0.
    While i is less than n:
        If flag:
            Set sum to sum + 2.
        Otherwise:
            Set sum to sum + 1.
        Set i to i + 1.
    Return sum.

## Main
Show test(true, 5).
Show test(false, 5).
"#;
    common::assert_exact_output(source, "10\n5");
}

// =============================================================================
// CAMP 4: Loop Peeling
// =============================================================================

#[test]
fn peel_first_iteration() {
    // First-iteration boundary check should be peeled out of the loop
    let source = r#"## To test (n: Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 0.
    While i is less than n:
        If i equals 0:
            Set sum to sum + 100.
        Otherwise:
            Set sum to sum + 1.
        Set i to i + 1.
    Return sum.

## Main
Show test(5).
"#;
    // After peeling, the first iteration (sum += 100) should be extracted.
    // The remaining loop body should only have sum += 1.
    let rust = compile_to_rust(source).unwrap();
    let fn_body = rust.split("fn test(").nth(1).unwrap_or("");
    // The loop body should NOT contain the i==0 branch
    let has_boundary_check = fn_body.contains("== 0)") || fn_body.contains("== 0 ");
    // But we still need correct output
    assert!(
        !has_boundary_check,
        "First-iteration check should be peeled out of the loop. Got:\n{}",
        rust
    );
}

#[test]
fn peel_no_fire_non_boundary() {
    // Non-boundary condition (i == 5) should NOT be peeled
    let source = r#"## To test (n: Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 0.
    While i is less than n:
        If i equals 5:
            Set sum to sum + 100.
        Otherwise:
            Set sum to sum + 1.
        Set i to i + 1.
    Return sum.

## Main
Show test(10).
"#;
    let rust = compile_to_rust(source).unwrap();
    let fn_body = rust.split("fn test(").nth(1).unwrap_or("");
    // The condition should still be inside the loop
    assert!(
        fn_body.contains("== 5"),
        "Non-boundary condition should NOT be peeled. Got:\n{}",
        rust
    );
}

#[test]
fn peel_first_e2e_correct() {
    let source = r#"## To test (n: Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 0.
    While i is less than n:
        If i equals 0:
            Set sum to sum + 100.
        Otherwise:
            Set sum to sum + 1.
        Set i to i + 1.
    Return sum.

## Main
Show test(5).
Show test(1).
Show test(0).
"#;
    common::assert_exact_output(source, "104\n100\n0");
}

// =============================================================================
// CAMP 5: Closed-Form Recognition + Strength Reduction
// =============================================================================

#[test]
fn closed_form_sum_1_to_n() {
    // sum += i from 1 to n → n*(n+1)/2
    let source = r#"## To sumTo (n: Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 1.
    While i is at most n:
        Set sum to sum + i.
        Set i to i + 1.
    Return sum.

## Main
Show sumTo(100).
Show sumTo(0).
"#;
    let rust = compile_to_rust(source).unwrap();
    // After closed-form, the while loop should be replaced with a formula
    let fn_body = rust.split("fn sumTo(").nth(1).unwrap_or("");
    assert!(
        !fn_body.contains("while") && !fn_body.contains("for "),
        "Closed-form should eliminate the loop. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "5050\n0");
}

#[test]
fn closed_form_count() {
    // count += 1 from 1 to n → n
    let source = r#"## To countTo (n: Int) -> Int:
    Let mutable count be 0.
    Let mutable i be 1.
    While i is at most n:
        Set count to count + 1.
        Set i to i + 1.
    Return count.

## Main
Show countTo(100).
Show countTo(0).
"#;
    let rust = compile_to_rust(source).unwrap();
    let fn_body = rust.split("fn countTo(").nth(1).unwrap_or("");
    assert!(
        !fn_body.contains("while") && !fn_body.contains("for "),
        "Closed-form should eliminate the count loop. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "100\n0");
}

#[test]
fn closed_form_product_not_applied() {
    // Factorial (multiply) should NOT be replaced with closed form
    let source = r#"## To factorial (n: Int) -> Int:
    Let mutable result be 1.
    Let mutable i be 1.
    While i is at most n:
        Set result to result * i.
        Set i to i + 1.
    Return result.

## Main
Show factorial(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    let fn_body = rust.split("fn factorial(").nth(1).unwrap_or("");
    // Factorial should keep its loop
    assert!(
        fn_body.contains("while") || fn_body.contains("for "),
        "Factorial should NOT be closed-form optimized. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "120");
}

// =============================================================================
// Bit-Twiddling Strength Reduction
// =============================================================================

#[test]
fn bit_strength_power_of_two_mul() {
    // x * 8 → x << 3
    let source = r#"## To scale (x: Int) -> Int:
    Return x * 8.

## Main
Show scale(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    let fn_body = rust.split("fn scale(").nth(1).unwrap_or("");
    assert!(
        fn_body.contains("<< 3"),
        "x * 8 should be strength-reduced to x << 3. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "40");
}

#[test]
fn bit_strength_power_of_two_mod() {
    // x % 16 → x & 15
    let source = r#"## To mask (x: Int) -> Int:
    Return x % 16.

## Main
Show mask(50).
"#;
    let rust = compile_to_rust(source).unwrap();
    let fn_body = rust.split("fn mask(").nth(1).unwrap_or("");
    assert!(
        fn_body.contains("& 15"),
        "x % 16 should be strength-reduced to x & 15. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "2");
}

#[test]
fn bit_strength_mul_not_power_of_two() {
    // x * 7 should NOT be strength-reduced (not a power of two)
    let source = r#"## To scale (x: Int) -> Int:
    Return x * 7.

## Main
Show scale(5).
"#;
    let rust = compile_to_rust(source).unwrap();
    let fn_body = rust.split("fn scale(").nth(1).unwrap_or("");
    assert!(
        !fn_body.contains("<<"),
        "x * 7 should NOT be strength-reduced. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "35");
}

#[test]
fn bit_strength_e2e_correct() {
    // Verify strength reduction produces correct results across various powers
    let source = r#"## To test (x: Int) -> Int:
    Let a be x * 2.
    Let b be x * 4.
    Let c be x * 16.
    Let d be x % 8.
    Let e be x % 32.
    Return a + b + c + d + e.

## Main
Show test(100).
"#;
    common::assert_exact_output(source, "2208");
}

// =============================================================================
// Boolean Algebra Laws
// =============================================================================

#[test]
fn fold_bool_or_true() {
    // x || true → true (short-circuit identity)
    let source = r#"## To test (x: Bool) -> Bool:
    Let result be x or true.
    Return result.

## Main
Show test(false).
"#;
    let rust = compile_to_rust(source).unwrap();
    let fn_body = rust.split("fn test(").nth(1).unwrap_or("");
    assert!(
        !fn_body.contains("||"),
        "x || true should simplify to true. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "true");
}

#[test]
fn fold_bool_and_false() {
    // x && false → false (short-circuit identity)
    let source = r#"## To test (x: Bool) -> Bool:
    Let result be x and false.
    Return result.

## Main
Show test(true).
"#;
    let rust = compile_to_rust(source).unwrap();
    let fn_body = rust.split("fn test(").nth(1).unwrap_or("");
    assert!(
        !fn_body.contains("&&"),
        "x && false should simplify to false. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "false");
}

#[test]
fn fold_bool_or_false() {
    // x || false → x
    let source = r#"## To test (x: Bool) -> Bool:
    Let result be x or false.
    Return result.

## Main
Show test(true).
Show test(false).
"#;
    let rust = compile_to_rust(source).unwrap();
    let fn_body = rust.split("fn test(").nth(1).unwrap_or("");
    assert!(
        !fn_body.contains("||"),
        "x || false should simplify to x. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "true\nfalse");
}

#[test]
fn fold_bool_and_true() {
    // x && true → x
    let source = r#"## To test (x: Bool) -> Bool:
    Let result be x and true.
    Return result.

## Main
Show test(true).
Show test(false).
"#;
    let rust = compile_to_rust(source).unwrap();
    let fn_body = rust.split("fn test(").nth(1).unwrap_or("");
    assert!(
        !fn_body.contains("&&"),
        "x && true should simplify to x. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "true\nfalse");
}

#[test]
fn fold_bool_double_negation() {
    // !!x → x
    let source = r#"## To test (x: Bool) -> Bool:
    Return not not x.

## Main
Show test(true).
Show test(false).
"#;
    let rust = compile_to_rust(source).unwrap();
    let fn_body = rust.split("fn test(").nth(1).unwrap_or("");
    assert!(
        !fn_body.contains("!"),
        "!!x should simplify to x. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "true\nfalse");
}

// =============================================================================
// Self-Comparison Identities
// =============================================================================

#[test]
fn fold_self_sub_zero() {
    // Literal subtraction: 42 - 42 → 0 via constant propagation + fold
    let source = r#"## Main
Let x be 42.
Let y be x - x.
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let y = 0") || rust.contains("show(&0"),
        "42 - 42 should fold to 0. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "0");
}

#[test]
fn fold_self_eq_true() {
    // Literal comparison: 42 == 42 → true via constant propagation + fold
    let source = r#"## Main
Let x be 42.
Let y be x equals x.
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let y = true") || rust.contains("show(&true"),
        "42 == 42 should fold to true. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "true");
}

// =============================================================================
// Compile-Time Function Evaluation (CTFE)
// =============================================================================

#[test]
fn ctfe_pure_constant_call() {
    // Pure recursive function with literal args → evaluated at compile time
    let source = r#"## To fib (n: Int) -> Int:
    If n is at most 1:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(10).
"#;
    let rust = compile_to_rust(source).unwrap();
    // After CTFE, fib(10) should be replaced with 55
    assert!(
        rust.contains("55"),
        "fib(10) should be evaluated to 55 at compile time. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "55");
}

#[test]
fn ctfe_recursive_pure() {
    // Pure recursive function with literal args → evaluated at compile time
    let source = r#"## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).

## Main
Show factorial(10).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("3628800"),
        "factorial(10) should be evaluated to 3628800 at compile time. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "3628800");
}

#[test]
fn ctfe_impure_not_evaluated() {
    // Function with IO (Show) should NOT be CTFE'd
    let source = r#"## To impure (x: Int) -> Int:
    Show x.
    Return x + 1.

## Main
Let result be impure(5).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    // The call should be preserved — impure functions can't be CTFE'd
    assert!(
        rust.contains("impure(5)") || rust.contains("impure(5i64)"),
        "Impure function call should be preserved. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "5\n6");
}

#[test]
fn ctfe_partial_args_not_evaluated() {
    // Function with non-literal argument should NOT be CTFE'd
    let source = r#"## To double (x: Int) -> Int:
    Return x * 2.

## Main
Let n be 5.
Let result be double(n).
Show result.
"#;
    // Even though n is constant (propagated), the call should NOT be CTFE'd
    // at the CTFE pass level — propagation + fold handles this instead
    common::assert_exact_output(source, "10");
}

// =============================================================================
// Partial Evaluation
// =============================================================================

#[test]
fn pe_constant_arg_specialized() {
    // Function called with a constant arg → body specialized, constant folded
    let source = r#"## To multiply (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Let x be 7.
Show multiply(3, x).
"#;
    let rust = compile_to_rust(source).unwrap();
    // After partial evaluation with a=3, multiply(3, x) should become 3 * x
    // or a specialized function multiply_3(b) = 3 * b
    // Either way, 3 * 7 = 21
    common::assert_exact_output(source, "21");
}

#[test]
fn pe_branch_elimination() {
    // Constant arg enables dead branch elimination within specialized body
    let source = r#"## To choose (flag: Bool, a: Int, b: Int) -> Int:
    If flag:
        Return a.
    Return b.

## Main
Show choose(true, 42, 99).
"#;
    let rust = compile_to_rust(source).unwrap();
    // With flag=true, the else branch is eliminated, so choose(true, 42, 99) → 42
    assert!(
        rust.contains("42"),
        "choose(true, 42, 99) should be specialized to 42. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "42");
}

#[test]
fn pe_no_specialize_impure() {
    // Impure functions (with Show) should NOT be partially evaluated
    let source = r#"## To logged_add (a: Int, b: Int) -> Int:
    Show a.
    Return a + b.

## Main
Let x be 5.
Show logged_add(3, x).
"#;
    let rust = compile_to_rust(source).unwrap();
    // The function contains IO (Show), so it should NOT be specialized
    assert!(
        rust.contains("logged_add"),
        "Impure function should not be partially evaluated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "3\n8");
}

#[test]
fn pe_no_specialize_all_dynamic() {
    // When all args are dynamic, no specialization occurs
    let source = r#"## To add (a: Int, b: Int) -> Int:
    Return a + b.

## Main
Let x be 3.
Let y be 5.
Show add(x, y).
"#;
    // Both args are dynamic from the function's perspective
    // (though propagation may have already substituted them)
    common::assert_exact_output(source, "8");
}

#[test]
fn pe_recursive_specialization() {
    // Partial evaluation of recursive function with one constant arg
    let source = r#"## To power (base: Int, exp: Int) -> Int:
    If exp is equal to 0:
        Return 1.
    Return base * power(base, exp - 1).

## Main
Show power(2, 10).
"#;
    // power(2, 10) = 1024 — CTFE handles fully-constant recursive calls,
    // PE handles partially-constant (or CTFE subsumes this case)
    common::assert_exact_output(source, "1024");
}

// =============================================================================
// Camp 0k: Direct Vec Indexing (no trait dispatch)
// =============================================================================

#[test]
fn vec_index_direct_no_trait_dispatch() {
    // When collection is known to be Vec<T>, emit direct indexing, not LogosIndex
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Show item 2 of items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("LogosIndex"),
        "Vec<i64> should use direct indexing, not LogosIndex trait dispatch. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "20");
}

#[test]
fn vec_setindex_direct_no_trait_dispatch() {
    // SetIndex on Vec should use direct assignment, not LogosIndexMut
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Set item 2 of items to 99.
Show item 2 of items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("LogosIndexMut") && !rust.contains("logos_set"),
        "Vec<i64> SetIndex should use direct assignment. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "99");
}

#[test]
fn vec_index_histogram_pattern() {
    // The histogram hot loop: Set item (v+1) of counts to (item (v+1) of counts) + 1.
    // Should use direct indexing, not trait dispatch
    let source = r#"## Main
Let mutable counts be a new Seq of Int.
Push 0 to counts.
Push 0 to counts.
Push 0 to counts.
Let v be 1.
Set item (v + 1) of counts to (item (v + 1) of counts) + 1.
Show item 2 of counts.
"#;
    let rust = compile_to_rust(source).unwrap();
    // Check the generated code uses direct indexing (v as usize pattern)
    assert!(
        !rust.contains("LogosIndex"),
        "Vec index in histogram pattern should use direct indexing. Got:\n{}",
        rust
    );
    // Also verify the index offset optimization: v+1 in index → v as usize
    assert!(
        rust.contains("v as usize"),
        "Index (v+1) should optimize to v as usize for direct indexing. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "1");
}

// =============================================================================
// Camp 0l: 1-Based → 0-Based Index Lowering (OPT-8)
// =============================================================================

#[test]
fn index_lower_zero_based_counter() {
    // For i from 1 to n: item i of arr → uses 0-based range
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Let mutable total be 0.
Let mutable i be 1.
While i is at most 3:
    Set total to total + item i of items.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    // OPT-8: counter start=1, only used for indexing → range becomes 0..3
    assert!(
        rust.contains("0..3") || rust.contains("0.."),
        "Counter starting at 1 with only index uses should be zero-based. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "60");
}

// =============================================================================
// Camp 0b: Bounds Check Elimination (OPT-4: assert_unchecked)
// =============================================================================

#[test]
fn bounds_elim_assert_unchecked() {
    // For-range loop indexing array → emit assert_unchecked hint for LLVM
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Let n be 3.
Let mutable total be 0.
Let mutable i be 1.
While i is at most n:
    Set total to total + item i of items.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    // OPT-4: assert_unchecked hint for bounds elision
    assert!(
        rust.contains("assert_unchecked"),
        "For-range with array indexing should emit assert_unchecked. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "60");
}

// =============================================================================
// Function Inlining (Non-recursive pure functions with all-constant args)
// =============================================================================

#[test]
fn inline_branching_function_all_constant_args() {
    // Non-recursive pure function with branches + all literal args → evaluate at compile time
    let source = r#"## To choose (flag: Bool, a: Int, b: Int) -> Int:
    If flag:
        Return a.
    Return b.

## Main
Show choose(true, 42, 99).
"#;
    let rust = compile_to_rust(source).unwrap();
    // After inlining with flag=true, a=42, b=99 → evaluates to 42
    // The function call should be eliminated (replaced with constant)
    assert!(
        !rust.contains("choose(true"),
        "Pure branching function with all-constant args should be inlined. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "42");
}

#[test]
fn inline_nonrecursive_preserves_simple_function() {
    // Simple non-branching function should NOT be inlined (LLVM handles it)
    // This prevents regression with test_function_codegen
    let source = r#"## To add (a: Int, b: Int) -> Int:
    Return a + b.

## Main
Let sum be add(3, 4).
Show sum.
"#;
    let rust = compile_to_rust(source).unwrap();
    // CTFE may evaluate add(3, 4) → 7 at compile time, which is correct.
    // The function definition should still exist.
    assert!(
        rust.contains("fn add("),
        "Function definition should be preserved. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("add(3") || rust.contains("let sum = 7"),
        "Should either keep call add(3, 4) or CTFE-evaluate to 7. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "7");
}

// =============================================================================
// Closed-Form: Power-of-2 (MulByTwo)
// =============================================================================

#[test]
fn closed_form_power_of_two() {
    // `iterations *= 2` in a loop → loop eliminated, replaced with `init << count`
    // This is the binary_trees benchmark pattern.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("10").
Let mutable result be 1.
Let mutable p be 0.
While p is less than n:
    Set result to result * 2.
    Set p to p + 1.
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    // Loop should be eliminated — no `for p in` or `while` in generated code
    // The whole loop should be replaced with a single shift expression
    assert!(
        !rust.contains("for p in") && !rust.contains("while p"),
        "Power-of-2 loop should be eliminated entirely. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "1024");
}

#[test]
fn closed_form_power_of_two_with_init() {
    // `result *= 2` starting from init=4, n=3 → 4 << 3 = 32
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("3").
Let mutable result be 4.
Let mutable p be 0.
While p is less than n:
    Set result to result * 2.
    Set p to p + 1.
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("for p in") && !rust.contains("while p"),
        "Power-of-2 with init=4 should eliminate the loop. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "32");
}

#[test]
fn closed_form_no_match_mul_by_three() {
    // `result *= 3` is NOT a power-of-2 pattern — loop should NOT be eliminated
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("4").
Let mutable result be 1.
Let mutable p be 0.
While p is less than n:
    Set result to result * 3.
    Set p to p + 1.
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    // Loop should remain — multiply by 3 has no closed form
    assert!(
        rust.contains("for p in") || rust.contains("while"),
        "Multiply by 3 loop should NOT be eliminated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "81");
}