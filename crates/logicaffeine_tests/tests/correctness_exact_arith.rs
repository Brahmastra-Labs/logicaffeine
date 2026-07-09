//! Overflow ruling v2 (oracle-gated, staged): compiled arithmetic NEVER
//! silently wraps.
//!
//! The engines today: interp/VM promote i64 overflow to BigInt (exact); the
//! JIT side-exits so the exact tier recomputes; audit-era AOT emitted bare
//! `+` — debug-compiled PANICS and release-compiled SILENTLY WRAPS
//! (`i64::MAX + 1` returned `i64::MIN`). The staged fix:
//!
//!   STAGE 1 (this suite's active tests): AOT emits CHECKED arithmetic that
//!   raises a catchable overflow error — loud, never wrong, zero
//!   happy-path cost.
//!
//!   STAGE 2 (the promotion tests below): constant overflow promotes at
//!   COMPILE time (a `LogosInt` literal); Oracle-proven-in-range arithmetic
//!   stays raw i64 (hot loops pay nothing); unproven arithmetic emits the
//!   checked-exact helpers (`logos_add_exact` …) whose overflow spills to
//!   the promoting `LogosInt` — full exactness, wins intact.

mod common;
use common::{assert_compiled_equals_interpreted_eq, run_interpreter, run_logos};

// =====================================================================
// Stage 1: overflow is LOUD in compiled code — never a silent wrap
// =====================================================================

#[test]
fn add_overflow_is_never_a_silent_wrap_compiled() {
    let compiled = run_logos(
        r#"## Main
Show 9223372036854775807 + 1.
"#,
    );
    if compiled.success {
        // If it runs, the answer must be the EXACT one (stage 2 landed) —
        // the audited failure mode was printing i64::MIN.
        assert_eq!(
            compiled.stdout.trim(),
            "9223372036854775808",
            "compiled overflow must never silently wrap"
        );
    }
    // Otherwise: a loud failure is the stage-1 contract. Either way the
    // interpreter's exact answer is locked:
    let interp = run_interpreter(
        r#"## Main
Show 9223372036854775807 + 1.
"#,
    );
    assert!(interp.success);
    assert_eq!(interp.output.trim(), "9223372036854775808");
}

#[test]
fn mul_overflow_is_never_a_silent_wrap_compiled() {
    let compiled = run_logos(
        r#"## Main
Show 9223372036854775807 * 2.
"#,
    );
    if compiled.success {
        assert_eq!(
            compiled.stdout.trim(),
            "18446744073709551614",
            "compiled overflow must never silently wrap"
        );
    }
    let interp = run_interpreter(
        r#"## Main
Show 9223372036854775807 * 2.
"#,
    );
    assert!(interp.success);
    assert_eq!(interp.output.trim(), "18446744073709551614");
}

#[test]
fn division_by_zero_fails_identically_everywhere() {
    let compiled = run_logos(
        r#"## Main
Show 1 / 0.
"#,
    );
    let interp = run_interpreter(
        r#"## Main
Show 1 / 0.
"#,
    );
    assert!(!interp.success, "interp must reject division by zero");
    assert!(
        !compiled.success,
        "compiled must reject division by zero (not wrap or return garbage)"
    );
}

#[test]
fn in_range_arithmetic_is_untouched() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 2147483647 + 1.
"#,
        "2147483648",
    );
}

// =====================================================================
// Stage 2 ratchets: full oracle-gated promotion (un-ignore as it lands)
// =====================================================================

#[test]
fn add_overflow_promotes_exactly_compiled() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 9223372036854775807 + 1.
"#,
        "9223372036854775808",
    );
}

#[test]
fn min_div_negative_one_promotes_exactly_compiled() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let a be -9223372036854775808.
Show a / -1.
"#,
        "9223372036854775808",
    );
}
