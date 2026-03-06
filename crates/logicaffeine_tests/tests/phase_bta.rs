mod common;

use common::compile_to_rust;
use logicaffeine_compile::ast::Literal;
use logicaffeine_compile::optimize::bta::{BindingTime, BtaEnv};

// =============================================================================
// Sprint 1.1 — Expression Classification (12 tests)
// =============================================================================

#[test]
fn bta_literal_int_static() {
    let source = "## Main\nLet x be 42.\nShow x.";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_main();
    let x = env.lookup("x").unwrap();
    assert_eq!(result.division[&x], BindingTime::Static(Literal::Number(42)));
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("42"), "Generated Rust should inline 42.\nGot:\n{}", rust);
}

#[test]
fn bta_literal_float_static() {
    let source = "## Main\nLet x be 3.14.\nShow x.";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_main();
    let x = env.lookup("x").unwrap();
    match &result.division[&x] {
        BindingTime::Static(Literal::Float(f)) => {
            assert!((f - 3.14).abs() < 1e-10, "Expected 3.14, got {}", f);
        }
        other => panic!("Expected x=S(3.14), got {:?}", other),
    }
}

#[test]
fn bta_literal_bool_static() {
    let source = "## Main\nLet x be true.\nShow x.";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_main();
    let x = env.lookup("x").unwrap();
    assert_eq!(result.division[&x], BindingTime::Static(Literal::Boolean(true)));
}

#[test]
fn bta_literal_text_static() {
    let source = "## Main\nLet x be \"hello\".\nShow x.";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_main();
    let x = env.lookup("x").unwrap();
    match &result.division[&x] {
        BindingTime::Static(Literal::Text(_)) => {}
        other => panic!("Expected x=S(Text), got {:?}", other),
    }
}

#[test]
fn bta_literal_nothing_static() {
    let source = "## Main\nLet x be nothing.\nShow x.";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_main();
    let x = env.lookup("x").unwrap();
    assert_eq!(result.division[&x], BindingTime::Static(Literal::Nothing));
}

#[test]
fn bta_identifier_tracks_division() {
    let source = "\
## To identity (x: Int) -> Int:
    Return x.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("identity", vec![
        BindingTime::Static(Literal::Number(10)),
    ]);
    let x = env.lookup("x").unwrap();
    assert_eq!(result.division[&x], BindingTime::Static(Literal::Number(10)));
}

#[test]
fn bta_binop_static_static() {
    let source = "## Main\nLet x be 2 + 3.\nShow x.";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_main();
    let x = env.lookup("x").unwrap();
    assert_eq!(result.division[&x], BindingTime::Static(Literal::Number(5)));
}

#[test]
fn bta_binop_static_dynamic() {
    let source = "\
## To f (a: Int, b: Int) -> Int:
    Return a + b.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("f", vec![
        BindingTime::Static(Literal::Number(3)),
        BindingTime::Dynamic,
    ]);
    assert_eq!(result.return_bt, BindingTime::Dynamic);
}

#[test]
fn bta_binop_dynamic_dynamic() {
    let source = "\
## To f (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("f", vec![
        BindingTime::Dynamic,
        BindingTime::Dynamic,
    ]);
    assert_eq!(result.return_bt, BindingTime::Dynamic);
}

#[test]
fn bta_not_static() {
    let source = "## Main\nLet x be not true.\nShow x.";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_main();
    let x = env.lookup("x").unwrap();
    assert_eq!(result.division[&x], BindingTime::Static(Literal::Boolean(false)));
}

#[test]
fn bta_length_is_dynamic() {
    let source = "\
## To f (items: Seq of Int) -> Int:
    Let n be length of items.
    Return n.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("f", vec![BindingTime::Dynamic]);
    let n = env.lookup("n").unwrap();
    assert_eq!(result.division[&n], BindingTime::Dynamic);
}

#[test]
fn bta_index_is_dynamic() {
    let source = "\
## To f (items: Seq of Int) -> Int:
    Let x be item 1 of items.
    Return x.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("f", vec![BindingTime::Dynamic]);
    let x = env.lookup("x").unwrap();
    assert_eq!(result.division[&x], BindingTime::Dynamic);
}

// =============================================================================
// Sprint 1.2 — Control Flow Analysis (6 tests)
// =============================================================================

#[test]
fn bta_if_static_true_only_then() {
    let source = "\
## To select (flag: Bool, x: Int, y: Int) -> Int:
    Let mutable result be -1.
    If flag:
        Set result to x.
    Otherwise:
        Set result to y.
    Return result.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("select", vec![
        BindingTime::Static(Literal::Boolean(true)),
        BindingTime::Static(Literal::Number(42)),
        BindingTime::Static(Literal::Number(99)),
    ]);
    // S(true) → only then-branch analyzed. result = x = S(42).
    assert_eq!(result.return_bt, BindingTime::Static(Literal::Number(42)));
}

#[test]
fn bta_if_static_false_only_else() {
    let source = "\
## To select (flag: Bool, x: Int, y: Int) -> Int:
    Let mutable result be -1.
    If flag:
        Set result to x.
    Otherwise:
        Set result to y.
    Return result.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("select", vec![
        BindingTime::Static(Literal::Boolean(false)),
        BindingTime::Static(Literal::Number(42)),
        BindingTime::Static(Literal::Number(99)),
    ]);
    // S(false) → only else-branch analyzed. result = y = S(99).
    assert_eq!(result.return_bt, BindingTime::Static(Literal::Number(99)));
}

#[test]
fn bta_if_dynamic_both_branches() {
    let source = "\
## To abs (x: Int) -> Int:
    Let mutable result be -1.
    If x > 0:
        Set result to x.
    Otherwise:
        Set result to 0 - x.
    Return result.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("abs", vec![BindingTime::Dynamic]);
    // D condition → both branches analyzed, join. x=D → result=D in both.
    assert_eq!(result.return_bt, BindingTime::Dynamic);
}

#[test]
fn bta_if_join_same_value() {
    let source = "\
## To f (flag: Bool) -> Int:
    Let mutable y be 0.
    If flag:
        Set y to 5.
    Otherwise:
        Set y to 5.
    Return y.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("f", vec![BindingTime::Dynamic]);
    // Both branches set y to S(5). Join: S(5) ⊔ S(5) = S(5).
    assert_eq!(result.return_bt, BindingTime::Static(Literal::Number(5)));
}

#[test]
fn bta_while_fixpoint_converges() {
    let source = "\
## To f (n: Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 1.
    While i <= n:
        Set sum to sum + i.
        Set i to i + 1.
    Return sum.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("f", vec![BindingTime::Dynamic]);
    let sum = env.lookup("sum").unwrap();
    let i = env.lookup("i").unwrap();
    // n=D, loop bound D → i and sum become D after fixpoint.
    assert_eq!(result.division[&sum], BindingTime::Dynamic);
    assert_eq!(result.division[&i], BindingTime::Dynamic);
    assert_eq!(result.return_bt, BindingTime::Dynamic);
}

#[test]
fn bta_nested_if() {
    let source = "\
## To f (flag: Bool) -> Int:
    Let mutable result be 0.
    If flag:
        If true:
            Set result to 5.
        Otherwise:
            Set result to 99.
    Otherwise:
        Set result to 5.
    Return result.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("f", vec![BindingTime::Dynamic]);
    // Outer D → both branches. Inner S(true) → only inner then.
    // Then: result = S(5). Else: result = S(5). Join: S(5).
    assert_eq!(result.return_bt, BindingTime::Static(Literal::Number(5)));
}

// =============================================================================
// Sprint 1.3 — Function Call Analysis (6 tests)
// =============================================================================

#[test]
fn bta_all_static_args() {
    let source = "\
## To add5 (x: Int) -> Int:
    Return x + 5.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("add5", vec![
        BindingTime::Static(Literal::Number(10)),
    ]);
    let x = env.lookup("x").unwrap();
    assert_eq!(result.division[&x], BindingTime::Static(Literal::Number(10)));
    assert_eq!(result.return_bt, BindingTime::Static(Literal::Number(15)));
}

#[test]
fn bta_all_dynamic_args() {
    let source = "\
## To double (x: Int) -> Int:
    Return x * 2.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("double", vec![
        BindingTime::Dynamic,
    ]);
    let x = env.lookup("x").unwrap();
    assert_eq!(result.division[&x], BindingTime::Dynamic);
    assert_eq!(result.return_bt, BindingTime::Dynamic);
}

#[test]
fn bta_mixed_args() {
    let source = "\
## To multiply (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("multiply", vec![
        BindingTime::Static(Literal::Number(3)),
        BindingTime::Dynamic,
    ]);
    let a = env.lookup("a").unwrap();
    let b = env.lookup("b").unwrap();
    assert_eq!(result.division[&a], BindingTime::Static(Literal::Number(3)));
    assert_eq!(result.division[&b], BindingTime::Dynamic);
    assert_eq!(result.return_bt, BindingTime::Dynamic);
}

#[test]
fn bta_recursive_static() {
    let source = "\
## To factorial (n: Int) -> Int:
    If n equals 0:
        Return 1.
    Return n * factorial(n - 1).

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("factorial", vec![
        BindingTime::Static(Literal::Number(5)),
    ]);
    assert_eq!(result.return_bt, BindingTime::Static(Literal::Number(120)));
}

#[test]
fn bta_mutual_recursion_scc() {
    let source = "\
## To f (x: Int) -> Int:
    If x <= 0:
        Return 0.
    Return g(x - 1).

## To g (x: Int) -> Int:
    If x <= 0:
        Return 0.
    Return f(x - 1).

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("f", vec![BindingTime::Dynamic]);
    assert_eq!(result.return_bt, BindingTime::Dynamic);
    let result_g = env.analyze_function("g", vec![BindingTime::Dynamic]);
    assert_eq!(result_g.return_bt, BindingTime::Dynamic);
}

#[test]
fn bta_nested_call_chain() {
    let source = "\
## To g (x: Int) -> Int:
    Return x + 10.

## To h (x: Int) -> Int:
    Return x.

## To outer (x: Int) -> Int:
    Let a be g(3).
    Let b be h(x).
    Return a + b.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("outer", vec![BindingTime::Dynamic]);
    let a = env.lookup("a").unwrap();
    let b = env.lookup("b").unwrap();
    // g(3) should be analyzed: g with S(3) → return S(13). So a=S(13).
    assert_eq!(result.division[&a], BindingTime::Static(Literal::Number(13)));
    // h(x) with x=D → return D. So b=D.
    assert_eq!(result.division[&b], BindingTime::Dynamic);
    // S(13) + D = D
    assert_eq!(result.return_bt, BindingTime::Dynamic);
}

// =============================================================================
// Sprint 1.4 — Edge Cases + Polyvariant (9 tests)
// =============================================================================

#[test]
fn bta_mutable_s_to_d_transition() {
    let source = "\
## To f (y: Int) -> Int:
    Let mutable x be 5.
    Set x to y.
    Return x.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("f", vec![BindingTime::Dynamic]);
    let x = env.lookup("x").unwrap();
    assert_eq!(result.division[&x], BindingTime::Dynamic);
}

#[test]
fn bta_collection_params_always_d() {
    let source = "\
## To f (items: Seq of Int) -> Int:
    Let n be length of items.
    Return n.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("f", vec![
        BindingTime::Static(Literal::Number(0)),
    ]);
    let items = env.lookup("items").unwrap();
    assert_eq!(result.division[&items], BindingTime::Dynamic);
}

#[test]
fn bta_set_makes_dynamic() {
    let source = "\
## To f (x: Int) -> Int:
    Let mutable y be 5.
    Set y to x.
    Return y + 1.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("f", vec![BindingTime::Dynamic]);
    let y = env.lookup("y").unwrap();
    assert_eq!(result.division[&y], BindingTime::Dynamic);
    assert_eq!(result.return_bt, BindingTime::Dynamic);
}

#[test]
fn bta_branch_dynamic_condition() {
    let source = "\
## To abs (x: Int) -> Int:
    If x > 0:
        Return x.
    Otherwise:
        Return 0 - x.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("abs", vec![BindingTime::Dynamic]);
    assert_eq!(result.return_bt, BindingTime::Dynamic);
}

#[test]
fn bta_loop_static_bound() {
    let source = "\
## To sumTo (n: Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 1.
    While i <= n:
        Set sum to sum + i.
        Set i to i + 1.
    Return sum.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("sumTo", vec![
        BindingTime::Static(Literal::Number(10)),
    ]);
    assert_eq!(result.return_bt, BindingTime::Static(Literal::Number(55)));
}

#[test]
fn bta_loop_dynamic_bound() {
    let source = "\
## To sumTo (n: Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 1.
    While i <= n:
        Set sum to sum + i.
        Set i to i + 1.
    Return sum.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result = env.analyze_function("sumTo", vec![BindingTime::Dynamic]);
    let sum = env.lookup("sum").unwrap();
    let i = env.lookup("i").unwrap();
    assert_eq!(result.division[&sum], BindingTime::Dynamic);
    assert_eq!(result.division[&i], BindingTime::Dynamic);
    assert_eq!(result.return_bt, BindingTime::Dynamic);
}

#[test]
fn bta_polyvariant_different_sites() {
    let source = "\
## To multiply (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result1 = env.analyze_function("multiply", vec![
        BindingTime::Static(Literal::Number(3)),
        BindingTime::Dynamic,
    ]);
    let a = env.lookup("a").unwrap();
    let b = env.lookup("b").unwrap();
    assert_eq!(result1.division[&a], BindingTime::Static(Literal::Number(3)));
    assert_eq!(result1.division[&b], BindingTime::Dynamic);
    assert_eq!(result1.return_bt, BindingTime::Dynamic);

    let result2 = env.analyze_function("multiply", vec![
        BindingTime::Dynamic,
        BindingTime::Static(Literal::Number(5)),
    ]);
    assert_eq!(result2.division[&a], BindingTime::Dynamic);
    assert_eq!(result2.division[&b], BindingTime::Static(Literal::Number(5)));
    assert_eq!(result2.return_bt, BindingTime::Dynamic);
}

#[test]
fn bta_polyvariant_cache_hit() {
    let source = "\
## To add (a: Int, b: Int) -> Int:
    Return a + b.

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result1 = env.analyze_function("add", vec![
        BindingTime::Static(Literal::Number(10)),
        BindingTime::Dynamic,
    ]);
    let result2 = env.analyze_function("add", vec![
        BindingTime::Static(Literal::Number(10)),
        BindingTime::Dynamic,
    ]);
    let result3 = env.analyze_function("add", vec![
        BindingTime::Static(Literal::Number(10)),
        BindingTime::Dynamic,
    ]);
    assert_eq!(result1.return_bt, result2.return_bt);
    assert_eq!(result2.return_bt, result3.return_bt);
    assert_eq!(result1.return_bt, BindingTime::Dynamic);
}

#[test]
fn bta_polyvariant_recursive_distinct() {
    let source = "\
## To power (base: Int, exp: Int) -> Int:
    If exp <= 0:
        Return 1.
    Return base * power(base, exp - 1).

## Main
Show 0.
";
    let mut env = BtaEnv::analyze_source(source).unwrap();
    let result1 = env.analyze_function("power", vec![
        BindingTime::Static(Literal::Number(2)),
        BindingTime::Static(Literal::Number(5)),
    ]);
    assert_eq!(result1.return_bt, BindingTime::Static(Literal::Number(32)));

    let result2 = env.analyze_function("power", vec![
        BindingTime::Static(Literal::Number(3)),
        BindingTime::Static(Literal::Number(4)),
    ]);
    assert_eq!(result2.return_bt, BindingTime::Static(Literal::Number(81)));
}
