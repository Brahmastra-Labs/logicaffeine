//! O8a gate — modulus deferral.
//!
//! `Set acc to (acc + X) % p` in a counted loop applies `% p` every
//! iteration, serializing a magic-number modulo chain that neither gcc nor
//! clang may reassociate. When intervals prove `k` additions fit in i64
//! before overflow, defer the modulo to every k-th iteration: the result is
//! identical (truncated remainder ≡ math mod while all partial sums are
//! non-negative) but runs one modulo per k elements.
//!
//! The rewrite is guarded at runtime (`If limit <= K_SAFE`) so it only takes
//! the deferred path when overflow is impossible; otherwise the original
//! loop runs unchanged. Soundness rests on: `acc` init ≥ 0, counter start
//! ≥ 0, `p` a literal ≥ 1, and `X` the counter (or a non-negative
//! loop-invariant). Negative values break truncated-remainder deferral and
//! are excluded by construction.

mod common;

use common::compile_to_rust;

/// K_SAFE for p = 1000000007, k = 16: (i64::MAX - (p-1)) / 16.
const KSAFE_1E9P7: &str = "576460752240923487";

// =============================================================================
// Fires: a loop_sum-shaped accumulator gets the deferral guard
// =============================================================================

#[test]
fn o8_loop_sum_fires() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("100").
Let mutable sum be 0.
Let mutable i be 1.
While i is at most n:
    Set sum to (sum + i) % 1000000007.
    Set i to i + 1.
Show sum.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains(KSAFE_1E9P7),
        "modulus deferral should emit the K_SAFE overflow guard. Got:\n{}",
        rust
    );
    // sum of 1..100 = 5050, mod 1e9+7 = 5050.
    common::assert_exact_output(source, "5050");
}

// =============================================================================
// Semantic equivalence across edge cases
// =============================================================================

fn defer_program(n: &str, p: &str) -> String {
    format!(
        r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("{}").
Let mutable sum be 0.
Let mutable i be 1.
While i is at most n:
    Set sum to (sum + i) % {}.
    Set i to i + 1.
Show sum.
"#,
        n, p
    )
}

#[test]
fn o8_semantic_battery() {
    // Trip counts straddling the chunk size (16): 0, 15, 16, 17, plus large.
    common::assert_exact_output(&defer_program("0", "1000000007"), "0"); // never runs
    common::assert_exact_output(&defer_program("15", "1000000007"), "120"); // 1+..+15
    common::assert_exact_output(&defer_program("16", "1000000007"), "136");
    common::assert_exact_output(&defer_program("17", "1000000007"), "153");
    common::assert_exact_output(&defer_program("100", "1000000007"), "5050");
    // p = 1: everything is 0.
    common::assert_exact_output(&defer_program("50", "1"), "0");
    // p = 2: parity of the running sum.
    common::assert_exact_output(&defer_program("17", "2"), "1"); // 153 % 2 = 1
    // Modulo actually wraps: sum of 1..100000 = 5000050000, mod 1e9+7.
    common::assert_exact_output(&defer_program("100000", "1000000007"), "5000050000".parse::<i128>().map(|v| (v % 1000000007).to_string()).unwrap().as_str());
}

#[test]
fn o8_accumulator_read_after_loop_is_correct() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("20").
Let mutable sum be 0.
Let mutable i be 1.
While i is at most n:
    Set sum to (sum + i) % 7.
    Set i to i + 1.
Let twice be sum * 2.
Show twice.
"#;
    // 1+..+20 = 210, 210 % 7 = 0, twice = 0.
    common::assert_exact_output(source, "0");
}

#[test]
fn o8_counter_read_after_loop_is_correct() {
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("20").
Let mutable sum be 0.
Let mutable i be 1.
While i is at most n:
    Set sum to (sum + i) % 1000.
    Set i to i + 1.
Show i.
"#;
    // After `While i <= 20`, i == 21.
    common::assert_exact_output(source, "21");
}

#[test]
fn o8_differential_equals_interpreted() {
    common::assert_compiled_equals_interpreted(&defer_program("33", "1000000007"));
    common::assert_compiled_equals_interpreted(&defer_program("16", "13"));
}

// =============================================================================
// Controls: these must NOT be rewritten (no K_SAFE guard)
// =============================================================================

fn assert_not_deferred(source: &str) {
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains(KSAFE_1E9P7) && !rust.contains("/ 16"),
        "this loop must NOT get modulus deferral. Got:\n{}",
        rust
    );
}

#[test]
fn o8_control_lcg_not_deferred() {
    // X references the accumulator (seed) multiplicatively — deferral is
    // unsound (and this is the lcg_chain control). Must not rewrite.
    assert_not_deferred(
        r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("100").
Let mutable seed be 1.
Let mutable i be 1.
While i is at most n:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Set i to i + 1.
Show seed.
"#,
    );
}

#[test]
fn o8_control_accumulator_in_addend_not_deferred() {
    // `acc + acc` — the addend references acc, so chunked deferral changes
    // the value. Must not rewrite.
    assert_not_deferred(
        r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("30").
Let mutable sum be 1.
Let mutable i be 1.
While i is at most n:
    Set sum to (sum + sum) % 1000000007.
    Set i to i + 1.
Show sum.
"#,
    );
}

#[test]
fn o8_control_variable_modulus_not_deferred() {
    // p is a runtime variable — K_SAFE can't be a compile-time literal.
    assert_not_deferred(
        r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("30").
Let p be parseInt("1000000007").
Let mutable sum be 0.
Let mutable i be 1.
While i is at most n:
    Set sum to (sum + i) % p.
    Set i to i + 1.
Show sum.
"#,
    );
}

#[test]
fn o8_control_extra_body_statement_not_deferred() {
    // An extra statement reading acc inside the body breaks the pattern.
    assert_not_deferred(
        r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("30").
Let mutable sum be 0.
Let mutable total be 0.
Let mutable i be 1.
While i is at most n:
    Set sum to (sum + i) % 1000000007.
    Set total to total + sum.
    Set i to i + 1.
Show total.
"#,
    );
}

#[test]
fn o8_control_step_two_not_deferred() {
    // Counter steps by 2 — not the unit-stride pattern.
    assert_not_deferred(
        r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("30").
Let mutable sum be 0.
Let mutable i be 1.
While i is at most n:
    Set sum to (sum + i) % 1000000007.
    Set i to i + 2.
Show sum.
"#,
    );
}
