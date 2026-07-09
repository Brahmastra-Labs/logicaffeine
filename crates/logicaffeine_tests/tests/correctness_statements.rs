//! Part I correctness: statement semantics agree across engines.
//!
//! The audit rows: `x = e` on an existing binding silently SHADOWED (a
//! loop-body `total = total + i` updated a loop-local ghost and the outer
//! variable never moved); `item -1 of xs` wrapped through usize instead of
//! reading end-relative; `item 0` hit the raw bounds machinery instead of a
//! friendly 1-based error; a `Let today be 5` was hijacked by the temporal
//! builtin; tuple destructure silently TRUNCATED on arity mismatch; and the
//! AOT hardcoded every inferred return type to i64.

mod common;
use common::{assert_compiled_equals_interpreted_eq, run_interpreter, run_logos};

// =====================================================================
// `=` mutates an existing binding (shadowing was the silent footgun)
// =====================================================================

#[test]
fn eq_on_existing_binding_mutates_not_shadows() {
    // Under shadowing, the loop body updates a loop-local ghost and this
    // prints 0. Under mutation it prints 6.
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable total be 0.
Repeat for i from 1 to 3:
    total = total + i.
Show total.
"#,
        "6",
    );
}

#[test]
fn eq_on_unbound_name_creates_a_binding() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
x = 5.
Show x.
"#,
        "5",
    );
}

#[test]
fn eq_mutation_needs_no_mutable_keyword() {
    // `x = e` auto-marks the binding mutable — the Part VI
    // mutable-inference row rides this.
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let t be 1.
t = t + 1.
Show t.
"#,
        "2",
    );
}

// =====================================================================
// Indexing: negative = end-relative, zero = friendly 1-based error
// =====================================================================

#[test]
fn negative_index_reads_end_relative() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let xs be [10, 20, 30].
Show item -1 of xs.
"#,
        "30",
    );
}

#[test]
fn negative_index_bracket_form_agrees() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let xs be [10, 20, 30].
Show xs[-2].
"#,
        "20",
    );
}

#[test]
fn index_zero_is_a_loud_error_everywhere() {
    let src = r#"## Main
Let xs be [10, 20, 30].
Let k be 0.
Show item k of xs.
"#;
    let interp = run_interpreter(src);
    assert!(!interp.success, "interp must reject index 0 (1-based)");
    let compiled = run_logos(src);
    assert!(
        !compiled.success,
        "compiled must reject index 0 — not wrap through usize: {}",
        compiled.stdout
    );
}

// =====================================================================
// A user binding beats the temporal builtins
// =====================================================================

#[test]
fn local_binding_beats_today_builtin() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let today be 5.
Show today.
"#,
        "5",
    );
}

// =====================================================================
// Tuple destructure arity is LOUD, never a silent truncation
// =====================================================================

#[test]
fn tuple_destructure_arity_mismatch_is_loud() {
    let src = r#"## Main
Let ps be [(1, 2, 3)].
Repeat for (a, b) in ps:
    Show a.
"#;
    let interp = run_interpreter(src);
    assert!(
        !interp.success,
        "interp must reject 2-binder destructure of a 3-tuple, got: {}",
        interp.output
    );
}

// =====================================================================
// Struct defaults agree across engines
// =====================================================================

#[test]
fn struct_defaults_agree_across_engines() {
    assert_compiled_equals_interpreted_eq(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point.
Show p's x.
"#,
        "0",
    );
}

// =====================================================================
// AOT return-type inference from the body (was hardcoded i64)
// =====================================================================

#[test]
fn aot_infers_text_return_type() {
    assert_compiled_equals_interpreted_eq(
        r#"## To greet:
    Return "hi".

## Main
Show greet().
"#,
        "hi",
    );
}

#[test]
fn aot_infers_bool_return_type() {
    assert_compiled_equals_interpreted_eq(
        r#"## To flag:
    Return true.

## Main
Show flag().
"#,
        "true",
    );
}

// =====================================================================
// `=` mutation is SCOPED — a name bound in another scope stays out
// =====================================================================

#[test]
fn eq_in_function_does_not_mutate_a_main_binding() {
    // `total` is bound in Main. Inside `f`, `total = 5` must create a LOCAL
    // binding (fresh), not emit a `Set` on the out-of-scope Main `total`
    // (which would be an "undefined variable" at runtime).
    assert_compiled_equals_interpreted_eq(
        r#"## To f () -> Int:
    total = 5.
    Return total.

## Main
Let total be 99.
Show f().
Show total.
"#,
        "5\n99",
    );
}

#[test]
fn eq_mutates_a_function_param() {
    // `n = n + 1` inside the function mutates the PARAMETER.
    assert_compiled_equals_interpreted_eq(
        r#"## To bump (n: Int) -> Int:
    n = n + 1.
    Return n.

## Main
Show bump(10).
"#,
        "11",
    );
}
