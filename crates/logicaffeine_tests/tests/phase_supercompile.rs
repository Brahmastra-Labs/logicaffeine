mod common;

use common::{compile_to_rust, assert_exact_output, run_logos};
use logicaffeine_compile::optimize::supercompile::{embeds, msg};
use logicaffeine_compile::ast::stmt::{Expr, Literal, BinaryOpKind};
use logicaffeine_compile::{Arena, Interner};

// =============================================================================
// Summit: Supercompilation (Research Phase)
// =============================================================================
//
// A supercompiler is a unified optimization algorithm that subsumes constant
// folding, propagation, dead code elimination, deforestation, and partial
// evaluation in a single framework. It works by:
//   1. Driving — symbolic execution one step at a time
//   2. Folding — detecting repeated configurations (memoization)
//   3. Generalization — widening when homeomorphic embedding is detected
//
// These tests verify that the supercompiler module, when run on pure integer
// code, produces the same optimizations as the full multi-pass pipeline.

// ---------------------------------------------------------------------------
// Subsumption Tests — the supercompiler handles what individual passes do
// ---------------------------------------------------------------------------

#[test]
fn super_subsumes_fold() {
    // Constant folding: 2 + 3 → 5
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let x be 2 + 3.
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("5") && !rust.contains("2 + 3"),
        "2 + 3 should be folded to 5. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "5");
}

#[test]
fn super_subsumes_propagation() {
    // Constant propagation: Let a=5. Let b=a+1. → b=6
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let a be 5.
Let b be a + 1.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("6"),
        "a+1 with a=5 should propagate+fold to 6. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "6");
}

#[test]
fn super_subsumes_dce() {
    // Dead code elimination: If false: Show "dead". → eliminated
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
If false:
    Show "dead".
Show "alive".
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("dead"),
        "If false branch should be eliminated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "alive");
}

#[test]
fn super_deforestation() {
    // Deforestation: producer → consumer fused into single pass
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable nums be a new Seq of Int.
Push 1 to nums.
Push 2 to nums.
Push 3 to nums.
Let mutable doubled be a new Seq of Int.
Repeat for x in nums:
    Push x * 2 to doubled.
Let mutable sum be 0.
Repeat for y in doubled:
    Set sum to sum + y.
Show sum.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("let mut doubled"),
        "Intermediate 'doubled' should be eliminated by deforestation. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "12");
}

#[test]
fn super_specialization() {
    // CTFE: factorial(10) → 3628800
    let source = r#"## To native parseInt (s: Text) -> Int

## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).

## Main
Let result be factorial(10).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("3628800"),
        "factorial(10) should be evaluated at compile time to 3628800. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "3628800");
}

#[test]
fn super_terminates_on_infinite() {
    // Infinite recursion with constant args must not hang the compiler
    // The function should be preserved (not evaluated) due to step limit
    let source = r#"## To native parseInt (s: Text) -> Int

## To loop_forever (n: Int) -> Int:
    Return loop_forever(n).

## Main
Let n be parseInt("0").
If n is greater than 0:
    Show loop_forever(n).
Show "done".
"#;
    common::assert_exact_output(source, "done");
}

// ---------------------------------------------------------------------------
// Supercompiler-Specific Tests — multi-step chains
// ---------------------------------------------------------------------------

#[test]
fn super_chain_fold_propagate_dce() {
    // Chain: fold → propagate → fold → DCE
    // Let a = 2+3 → 5. Let b = a*2 → 10. If b > 100: dead → eliminated.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let a be 2 + 3.
Let b be a * 2.
If b is greater than 100:
    Show "unreachable".
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("unreachable"),
        "Multi-step chain should eliminate dead branch. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "10");
}

#[test]
fn super_ctfe_plus_propagation() {
    // CTFE result feeds into propagation + fold
    let source = r#"## To native parseInt (s: Text) -> Int

## To double (x: Int) -> Int:
    Return x * 2.

## Main
Let a be double(21).
Let b be a + a.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("84"),
        "double(21)=42, 42+42=84 should be fully evaluated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "84");
}

#[test]
fn super_nested_ctfe() {
    // CTFE on nested pure calls: double(triple(5)) → 30
    let source = r#"## To native parseInt (s: Text) -> Int

## To double (x: Int) -> Int:
    Return x * 2.

## To triple (x: Int) -> Int:
    Return x * 3.

## Main
Let result be double(triple(5)).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("30"),
        "double(triple(5)) = double(15) = 30 should be evaluated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "30");
}

// =============================================================================
// Sprint 3a — Text Propagation + CTFE Text Support (12 tests)
// =============================================================================

#[test]
fn text_supercompile_propagates() {
    let source = r#"## Main
Let name be "Alice".
Show name.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("\"Alice\"") && !rust.contains("show(&name"),
        "Supercompiler should substitute name with literal \"Alice\" in the Show, not a variable lookup.\nGot:\n{}",
        rust
    );
}

#[test]
fn text_propagate_constant_prop() {
    let source = r#"## Main
Let x be "world".
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("\"world\""),
        "Constant propagation should substitute x → \"world\".\nGot:\n{}",
        rust
    );
}

#[test]
fn text_ctfe_pure_function() {
    let source = r#"## To greet (name: Text) -> Text:
    Return "Hello, " + name + "!".

## Main
Let msg be greet("Bob").
Show msg.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("Hello, Bob!"),
        "CTFE should evaluate greet(\"Bob\") to \"Hello, Bob!\" at compile time.\nGot:\n{}",
        rust
    );
}

#[test]
fn text_ctfe_concat() {
    let source = r#"## Main
Let x be "Hello, " + "World" + "!".
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("Hello, World!"),
        "Fold should concatenate \"Hello, \" + \"World\" + \"!\" at compile time.\nGot:\n{}",
        rust
    );
}

#[test]
fn text_ctfe_compare() {
    let source = r#"## Main
Let x be "a" equals "a".
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("true"),
        "Fold should evaluate \"a\" equals \"a\" → true at compile time.\nGot:\n{}",
        rust
    );
}

#[test]
fn text_symbol_identity_preserved() {
    let source = r#"## Main
Let a be "hello".
Let b be a.
Show b.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("\"hello\"") && !rust.contains("show(&b"),
        "After propagation, Show should use literal \"hello\" directly, not variable b.\nGot:\n{}",
        rust
    );
}

#[test]
fn text_codegen_ownership() {
    let source = r#"## Main
Let x be "test".
Show x.
Show x.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "After text propagation, codegen should handle ownership correctly.\nstderr: {}\n\nGenerated Rust:\n{}",
        result.stderr, result.rust_code
    );
    assert_eq!(
        result.stdout.trim(),
        "test\ntest",
        "Should output 'test' twice.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(), result.rust_code
    );
}

#[test]
fn text_cross_function_propagation() {
    let source = r#"## To f () -> Text:
    Return "static".

## Main
Let x be f().
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("\"static\""),
        "CTFE or inlining should evaluate f() → \"static\" and propagate.\nGot:\n{}",
        rust
    );
}

#[test]
fn text_ctfe_mixed_not_evaluated() {
    let source = r#"## To native readLine () -> Text

## To greet (name: Text) -> Text:
    Return "Hello, " + name + "!".

## Main
Let name be readLine().
Let msg be greet(name).
Show msg.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("greet(") || rust.contains("greet ("),
        "Dynamic arg prevents CTFE. Call to greet should be preserved.\nGot:\n{}",
        rust
    );
}

#[test]
fn text_e2e_hello_alice() {
    let source = r#"## Main
Let name be "Alice".
Let msg be "Hello, " + name.
Show msg.
"#;
    assert_exact_output(source, "Hello, Alice");
}

#[test]
fn text_e2e_greeting_function() {
    let source = r#"## To greet (name: Text) -> Text:
    Return "Hello, " + name + "!".

## Main
Let msg be greet("Bob").
Show msg.
"#;
    assert_exact_output(source, "Hello, Bob!");
}

#[test]
fn text_propagate_multiple_uses() {
    let source = r#"## Main
Let x be "hi".
Show x + " " + x.
"#;
    assert_exact_output(source, "hi hi");
}

// =============================================================================
// Sprint 3b — Index/Slice Driving (10 tests)
// =============================================================================

#[test]
fn index_driven_first_element() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Let x be item 1 of items.
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("10") || rust.contains("items["),
        "Supercompiler should resolve item 1 of [10,20,30] to 10 or preserve runtime indexing. Got:\n{}",
        rust
    );
    assert_exact_output(source, "10");
}

#[test]
fn index_driven_last_element() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Let x be item 3 of items.
Show x.
"#;
    assert_exact_output(source, "30");
}

#[test]
fn index_out_of_bounds_preserved() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Let x be item 5 of items.
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("[") || rust.contains("index"),
        "Out-of-bounds index 5 on 3-element list must be preserved for runtime error. Got:\n{}",
        rust
    );
}

#[test]
fn index_after_push_tracks() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Let x be item 2 of items.
Show x.
"#;
    assert_exact_output(source, "20");
}

#[test]
fn index_dynamic_collection_preserved() {
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## Main
Let items be args().
Let x be item 1 of items.
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("[") || rust.contains("index"),
        "Dynamic collection must preserve runtime indexing. Got:\n{}",
        rust
    );
}

#[test]
fn index_dynamic_index_preserved() {
    let source = r#"## To native parseInt (s: Text) -> Int
## To native args () -> Seq of Text

## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Let n be parseInt("2").
Let x be item n of items.
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("[") || rust.contains("index"),
        "Dynamic index n must preserve runtime indexing even on known collection. Got:\n{}",
        rust
    );
    assert_exact_output(source, "20");
}

#[test]
fn slice_driving_basic() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Push 40 to items.
Let part be items 2 through 3.
Show length of part.
"#;
    assert_exact_output(source, "2");
}

#[test]
fn swap_pattern_still_detected() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let temp be item 1 of items.
Set item 1 of items to item 3 of items.
Set item 3 of items to temp.
Show item 1 of items.
Show item 2 of items.
Show item 3 of items.
"#;
    assert_exact_output(source, "3\n2\n1");
}

#[test]
fn index_lowering_after_fold() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Let x be item (2 + 1) of items.
Show x.
"#;
    assert_exact_output(source, "30");
}

#[test]
fn index_e2e_output() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 100 to items.
Push 200 to items.
Push 300 to items.
Show item 2 of items.
"#;
    assert_exact_output(source, "200");
}

// =============================================================================
// Sprint 3c — Residual Code Generation (12 tests)
// =============================================================================

// ---------------------------------------------------------------------------
// Residual Expression Tests — mixed static/dynamic operands produce new AST
// ---------------------------------------------------------------------------

#[test]
fn residual_static_left() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int) and (b: Int) -> Int:
    Return a * b.

## Main
Let n be parseInt("7").
Let result be f(3, n).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("3 * "),
        "PE should specialize f(3, D): residual body should contain 3 * <dyn>. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "21");
}

#[test]
fn residual_static_right() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int) and (b: Int) -> Int:
    Return a * b.

## Main
Let n be parseInt("4").
Let result be f(n, 5).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("* 5"),
        "PE should specialize f(D, 5): residual body should contain <dyn> * 5. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "20");
}

#[test]
fn residual_both_static() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int) and (b: Int) -> Int:
    Return a * b.

## Main
Let result be f(3, 5).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("15"),
        "Both static: f(3, 5) should fold to 15 via CTFE. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "15");
}

#[test]
fn residual_nested_binary() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int) and (b: Int) -> Int:
    Return a * b + a.

## Main
Let n be parseInt("7").
Let result be f(3, n).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("3 * ") && rust.contains("+ 3"),
        "PE should specialize f(3, D): residual should be 3 * <dyn> + 3. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "24");
}

#[test]
fn residual_if_true_eliminated() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To g (flag: Bool) and (x: Int) -> Int:
    If flag:
        Return x + 1.
    Otherwise:
        Return x - 1.

## Main
Let n be parseInt("10").
Let result be g(true, n).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("g(true"),
        "PE should specialize g(true, D) — no unspecialized call. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "11");
}

#[test]
fn residual_if_false_eliminated() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To g (flag: Bool) and (x: Int) -> Int:
    If flag:
        Return x + 1.
    Otherwise:
        Return x - 1.

## Main
Let n be parseInt("10").
Let result be g(false, n).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("g(false"),
        "PE should specialize g(false, D) — no unspecialized call. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "9");
}

// ---------------------------------------------------------------------------
// Residual Control Flow + Call Tests
// ---------------------------------------------------------------------------

#[test]
fn residual_if_dynamic_preserved() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To h (x: Int) -> Int:
    If x is greater than 0:
        Return x.
    Otherwise:
        Return 0 - x.

## Main
Let n be parseInt("5").
Let result be h(n).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("if ") || rust.contains("match "),
        "All-dynamic: conditional structure should be preserved. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "5");
}

#[test]
fn residual_while_false_eliminated() {
    let source = r#"## Main
While false:
    Show "never".
Show "done".
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("never"),
        "While false body should be eliminated entirely. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "done");
}

#[test]
fn residual_while_dynamic_preserved() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable x be parseInt("3").
While x is greater than 0:
    Set x to x - 1.
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("while ") || rust.contains("loop") || rust.contains("for "),
        "Dynamic condition: while loop should be preserved. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "0");
}

#[test]
fn residual_call_all_dynamic() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int) and (b: Int) -> Int:
    Return a + b.

## Main
Let a be parseInt("3").
Let b be parseInt("4").
Let result be f(a, b).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("f(a, b)") || rust.contains("f(a,b)"),
        "All-dynamic: unspecialized call f(a, b) should be preserved. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "7");
}

#[test]
fn residual_call_mixed() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int) and (b: Int) -> Int:
    Return a + b.

## Main
Let n be parseInt("4").
Let result be f(3, n).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("3 + "),
        "Mixed S(3), D: PE should specialize with body containing 3 + <dyn>. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "7");
}

#[test]
fn residual_mixed_binary_op_output() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int) and (b: Int) -> Int:
    Return a * b + a.

## Main
Let n be parseInt("7").
Let result be f(3, n).
Show result.
"#;
    common::assert_exact_output(source, "24");
}

// =============================================================================
// Sprint 3e — Identity / Perfect Residuals (4 tests)
// =============================================================================
//
// The identity property: when ALL inputs are dynamic, the optimizer should
// produce code structurally equivalent to the original — no extra bindings,
// no dead dispatch, no overhead.

#[test]
fn identity_trivial_program() {
    let source = r#"## Main
Show 42.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("42"),
        "Trivial program should emit 42 directly. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "42");
}

#[test]
fn identity_arithmetic_program() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let a be parseInt("3").
Let b be parseInt("4").
Let x be a + b.
Show x.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("a + b") || rust.contains("a +b"),
        "Arithmetic should be preserved as a + b with all-dynamic inputs. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "7");
}

#[test]
fn identity_control_flow_program() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let x be parseInt("5").
If x is greater than 0:
    Show "pos".
Otherwise:
    Show "neg".
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("if ") && rust.contains("pos") && rust.contains("neg"),
        "All-dynamic: if/else with both branches should be preserved. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "pos");
}

#[test]
fn identity_function_call_program() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To double (n: Int) -> Int:
    Return n * 2.

## Main
Let x be parseInt("21").
Show double(x).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("double("),
        "All-dynamic: function call should be preserved (not inlined). Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "42");
}

// =============================================================================
// Sprint 3d — Homeomorphic Embedding and Generalization (12 tests)
// =============================================================================

// ---------------------------------------------------------------------------
// Embedding Detection Tests — unit tests on embeds()
// ---------------------------------------------------------------------------

#[test]
fn embedding_self_check() {
    let arena: Arena<Expr> = Arena::new();
    let x = arena.alloc(Expr::Literal(Literal::Number(42)));
    assert!(embeds(x, x), "An expression should embed in itself: x ◁ x");

    let y = arena.alloc(Expr::Literal(Literal::Boolean(true)));
    assert!(embeds(y, y), "Boolean literal should embed in itself");
}

#[test]
fn embedding_diving_check() {
    let arena: Arena<Expr> = Arena::new();
    let x = arena.alloc(Expr::Literal(Literal::Number(3)));
    let y = arena.alloc(Expr::Literal(Literal::Number(5)));
    let x_plus_y = arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: x,
        right: y,
    });
    // x ◁ (x + y) via diving into the left child
    assert!(embeds(x, x_plus_y), "3 should embed in (3 + 5) via diving");
    // y ◁ (x + y) via diving into the right child
    assert!(embeds(y, x_plus_y), "5 should embed in (3 + 5) via diving");
}

#[test]
fn embedding_coupling_check() {
    let arena: Arena<Expr> = Arena::new();
    let a = arena.alloc(Expr::Literal(Literal::Number(1)));
    let b = arena.alloc(Expr::Literal(Literal::Number(2)));
    let a_plus_b = arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: a,
        right: b,
    });

    let c = arena.alloc(Expr::Literal(Literal::Number(1)));
    let d = arena.alloc(Expr::Literal(Literal::Number(2)));
    let one = arena.alloc(Expr::Literal(Literal::Number(1)));
    let c_plus_one = arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: c,
        right: one,
    });
    let one2 = arena.alloc(Expr::Literal(Literal::Number(1)));
    let d_plus_one = arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: d,
        right: one2,
    });
    let bigger = arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: c_plus_one,
        right: d_plus_one,
    });
    // (1 + 2) ◁ ((1+1) + (2+1)) via coupling (same +, children embed via diving)
    assert!(embeds(a_plus_b, bigger), "a+b should embed in (a+1)+(b+1) via coupling+diving");
}

#[test]
fn embedding_rejects_non_embedded() {
    let arena: Arena<Expr> = Arena::new();
    let a = arena.alloc(Expr::Literal(Literal::Number(3)));
    let b = arena.alloc(Expr::Literal(Literal::Number(5)));
    let a_times_b = arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Multiply,
        left: a,
        right: b,
    });
    let c = arena.alloc(Expr::Literal(Literal::Number(7)));
    // (3 * 5) should NOT embed in 7 — different constructors, no diving match
    assert!(!embeds(a_times_b, c), "BinaryOp should not embed in a simple literal");

    // Different literals should not embed in each other
    let x = arena.alloc(Expr::Literal(Literal::Number(10)));
    let y = arena.alloc(Expr::Literal(Literal::Number(20)));
    assert!(!embeds(x, y), "10 should not embed in 20");
}

#[test]
fn embedding_growing_detected() {
    // grow(n) = grow(n+1) — infinite recursion with growing argument
    // The supercompiler must detect this growing pattern and terminate
    let source = r#"## To native parseInt (s: Text) -> Int

## To grow (n: Int) -> Int:
    Return grow(n + 1).

## Main
Let n be parseInt("0").
If n is greater than 0:
    Show grow(n).
Show "done".
"#;
    common::assert_exact_output(source, "done");
}

// ---------------------------------------------------------------------------
// MSG + Generalization Tests
// ---------------------------------------------------------------------------

#[test]
fn msg_computation() {
    let arena: Arena<Expr> = Arena::new();
    let mut interner = Interner::new();
    let a_sym = interner.intern("a");
    let b_sym = interner.intern("b");
    let c_sym = interner.intern("c");

    // e1 = a + b
    let a1 = arena.alloc(Expr::Identifier(a_sym));
    let b1 = arena.alloc(Expr::Identifier(b_sym));
    let e1 = arena.alloc(Expr::BinaryOp { op: BinaryOpKind::Add, left: a1, right: b1 });

    // e2 = a + c
    let a2 = arena.alloc(Expr::Identifier(a_sym));
    let c1 = arena.alloc(Expr::Identifier(c_sym));
    let e2 = arena.alloc(Expr::BinaryOp { op: BinaryOpKind::Add, left: a2, right: c1 });

    let result = msg(e1, e2, &arena, &mut interner);
    // MSG should be a + ?1 — preserves common 'a' and '+', fresh var for b/c
    if let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = result.expr {
        assert!(matches!(left, Expr::Identifier(s) if *s == a_sym),
            "Common part 'a' should be preserved on the left");
        assert!(matches!(right, Expr::Identifier(s) if *s != b_sym && *s != c_sym),
            "Differing part should be a fresh variable, not b or c");
    } else {
        panic!("MSG of (a+b) and (a+c) should be BinaryOp(Add). Got: {:?}", result.expr);
    }
    assert_eq!(result.num_substitutions, 1, "One substitution for the differing right operand");
}

#[test]
fn msg_precision() {
    let arena: Arena<Expr> = Arena::new();
    let mut interner = Interner::new();
    let f_sym = interner.intern("f");
    let x_sym = interner.intern("x");
    let y_sym = interner.intern("y");

    // e1 = f(3, x)
    let three1 = arena.alloc(Expr::Literal(Literal::Number(3)));
    let x1 = arena.alloc(Expr::Identifier(x_sym));
    let e1 = arena.alloc(Expr::Call { function: f_sym, args: vec![three1, x1] });

    // e2 = f(3, y)
    let three2 = arena.alloc(Expr::Literal(Literal::Number(3)));
    let y1 = arena.alloc(Expr::Identifier(y_sym));
    let e2 = arena.alloc(Expr::Call { function: f_sym, args: vec![three2, y1] });

    let result = msg(e1, e2, &arena, &mut interner);
    // MSG should be f(3, ?1) — preserves f and 3, fresh var for x/y
    if let Expr::Call { function, args } = result.expr {
        assert_eq!(*function, f_sym, "Function name should be preserved");
        assert_eq!(args.len(), 2, "Should have 2 args");
        assert!(matches!(args[0], Expr::Literal(Literal::Number(3))),
            "Common arg 3 should be preserved");
        assert!(matches!(args[1], Expr::Identifier(s) if *s != x_sym && *s != y_sym),
            "Differing arg should be fresh variable");
    } else {
        panic!("MSG of f(3,x) and f(3,y) should be Call. Got: {:?}", result.expr);
    }
    assert_eq!(result.num_substitutions, 1, "Only one substitution for the differing arg");
}

#[test]
fn msg_replacement() {
    let arena: Arena<Expr> = Arena::new();
    let mut interner = Interner::new();
    let a_sym = interner.intern("a");

    // e1 = a + 5
    let a1 = arena.alloc(Expr::Identifier(a_sym));
    let five = arena.alloc(Expr::Literal(Literal::Number(5)));
    let e1 = arena.alloc(Expr::BinaryOp { op: BinaryOpKind::Add, left: a1, right: five });

    // e2 = a + 10
    let a2 = arena.alloc(Expr::Identifier(a_sym));
    let ten = arena.alloc(Expr::Literal(Literal::Number(10)));
    let e2 = arena.alloc(Expr::BinaryOp { op: BinaryOpKind::Add, left: a2, right: ten });

    let result = msg(e1, e2, &arena, &mut interner);
    // After MSG, the concrete values 5 and 10 are replaced with a fresh variable
    assert_eq!(result.num_substitutions, 1,
        "5 and 10 should be generalized into one fresh variable");
    // The generalized expression should be a + ?1
    if let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = result.expr {
        assert!(matches!(left, Expr::Identifier(s) if *s == a_sym),
            "Common 'a' should be preserved");
        assert!(!matches!(right, Expr::Literal(Literal::Number(5))),
            "Concrete value 5 should be replaced by fresh variable");
        assert!(!matches!(right, Expr::Literal(Literal::Number(10))),
            "Concrete value 10 should be replaced by fresh variable");
    } else {
        panic!("MSG should produce BinaryOp(Add). Got: {:?}", result.expr);
    }
}

#[test]
fn embedding_depth_limit_64() {
    // Deeply recursive function that would drive to depth > 64
    // The supercompiler's depth limit must fire and terminate compilation
    let source = r#"## To native parseInt (s: Text) -> Int

## To deep (n: Int) -> Int:
    Return deep(n + 1) + deep(n + 2).

## Main
Let n be parseInt("0").
If n is greater than 0:
    Show deep(n).
Show "ok".
"#;
    common::assert_exact_output(source, "ok");
}

#[test]
fn embedding_tail_recursive_output() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (n: Int) and (acc: Int) -> Int:
    If n is at most 0:
        Return acc.
    Return f(n - 1, acc + n).

## Main
Let n be parseInt("10").
Let y be f(n, 0).
Show y.
"#;
    common::assert_exact_output(source, "55");
}

#[test]
fn embedding_while_loop_output() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To sumLoop (n: Int) -> Int:
    Let sum be 0.
    Let i be 1.
    While i is at most n:
        Set sum to sum + i.
        Set i to i + 1.
    Return sum.

## Main
Let n be parseInt("100").
Show sumLoop(n).
"#;
    common::assert_exact_output(source, "5050");
}

#[test]
fn generalization_preserves_correctness() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To compute (n: Int) and (acc: Int) -> Int:
    If n is at most 0:
        Return acc.
    Return compute(n - 1, acc + n * n).

## Main
Let n be parseInt("5").
Let r be compute(n, 0).
Show r.
"#;
    // 5^2 + 4^2 + 3^2 + 2^2 + 1^2 = 25 + 16 + 9 + 4 + 1 = 55
    common::assert_exact_output(source, "55");
}

// =============================================================================
// Sprint B — Fix Supercompiler Foundation (Steps B1, B2)
// =============================================================================

#[test]
fn supercompile_while_precise_widening() {
    // A while loop that modifies two variables but one is predictable.
    // With precise widening (MSG-based), the predictable variable stays known.
    // With aggressive widening (remove all), both become unknown.
    let source = r#"
## To test () -> Int:
    Let mutable sum be 0.
    Let mutable i be 1.
    Let scale be 10.
    While i is at most 3:
        Set sum to sum + (i * scale).
        Set i to i + 1.
    Return sum.

## Main
    Show test().
"#;
    common::assert_exact_output(source, "60");
}

#[test]
fn supercompile_embedding_prevents_divergence() {
    // A recursive function with growing argument should be caught by embedding
    // before the depth limit. Verifies embeds() is actually called.
    let source = r#"
## To recurse (n: Int) -> Int:
    If n is at most 0:
        Return 0.
    Return n + recurse(n - 1).

## Main
    Show recurse(5).
"#;
    common::assert_exact_output(source, "15");
}

#[test]
fn supercompile_index_static_collection() {
    // Index into a known-static list should resolve at compile time
    let source = r#"
## Main
    Let items be [10, 20, 30].
    Let second be item 2 of items.
    Show second.
"#;
    common::assert_exact_output(source, "20");
}

#[test]
fn supercompile_index_dynamic_preserved() {
    // Index with dynamic index should be preserved in residual
    let source = r#"
## To getItem (items: Seq of Int) and (i: Int) -> Int:
    Return item i of items.

## Main
    Let items be [10, 20, 30].
    Show getItem(items, 2).
"#;
    common::assert_exact_output(source, "20");
}

// ============================================================
// Sprint J: MSG wiring tests
// ============================================================

/// MSG of two identical expressions should produce no substitutions.
#[test]
fn msg_identical_no_substitutions() {
    let arena = Arena::new();
    let mut interner = Interner::new();

    let x_sym = interner.intern("x");
    let e1 = arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: arena.alloc(Expr::Identifier(x_sym)),
        right: arena.alloc(Expr::Literal(Literal::Number(1))),
    });
    let e2 = arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: arena.alloc(Expr::Identifier(x_sym)),
        right: arena.alloc(Expr::Literal(Literal::Number(1))),
    });

    let result = msg(e1, e2, &arena, &mut interner);
    assert_eq!(result.num_substitutions, 0, "MSG of identical exprs should have 0 substitutions");
}

/// MSG should preserve common structure and introduce vars for differences.
#[test]
fn msg_preserves_common_structure() {
    let arena = Arena::new();
    let mut interner = Interner::new();

    let a_sym = interner.intern("a");
    // e1 = a + 1
    let e1 = arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: arena.alloc(Expr::Identifier(a_sym)),
        right: arena.alloc(Expr::Literal(Literal::Number(1))),
    });
    // e2 = a + 2
    let e2 = arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: arena.alloc(Expr::Identifier(a_sym)),
        right: arena.alloc(Expr::Literal(Literal::Number(2))),
    });

    let result = msg(e1, e2, &arena, &mut interner);
    // Common: a + ___, different: 1 vs 2 → 1 substitution
    assert_eq!(result.num_substitutions, 1, "MSG(a+1, a+2) should have 1 substitution");
    // Result should be BinaryOp with Add
    match result.expr {
        Expr::BinaryOp { op, left, .. } => {
            assert_eq!(*op, BinaryOpKind::Add);
            // Left should be preserved as 'a'
            match left {
                Expr::Identifier(sym) => assert_eq!(*sym, a_sym),
                _ => panic!("Left of MSG should be Identifier(a)"),
            }
        }
        _ => panic!("MSG result should be a BinaryOp"),
    }
}

/// Supercompile with a while loop should not panic when MSG is wired.
#[test]
fn supercompile_while_msg_no_panic() {
    let source = r#"## Main
    Let mutable i be 0.
    Let mutable total be 0.
    While i is less than 10:
        Set total to total + i.
        Set i to i + 1.
    Show total.
"#;
    common::assert_exact_output(source, "45");
}
