//! E2E Codegen Tests: exact Rational division (the AOT compile-to-Rust tier).
//!
//! The interpreter and VM already keep `Let x: Rational be 7 / 2` exact (`7/2`) while a
//! bare `7 / 2` floors (`3`). These tests prove the SAME type-directed semantics on the
//! compiled-to-Rust path: a `Rational`-typed binding compiles to an exact `LogosRational`,
//! and the floor default is byte-for-byte unchanged.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_rational_fraction_stays_exact() {
    assert_exact_output("## Main\nLet x: Rational be 7 / 2.\nShow x.", "7/2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_rational_one_third_is_not_a_float() {
    assert_exact_output("## Main\nLet x: Rational be 1 / 3.\nShow x.", "1/3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_rational_that_reduces_to_a_whole_shows_as_an_int() {
    assert_exact_output("## Main\nLet x: Rational be 6 / 2.\nShow x.", "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_rational_plain_integer_value_coerces() {
    assert_exact_output("## Main\nLet x: Rational be 5.\nShow x.", "5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_rational_constant_chain_lightning_folds() {
    // 1/3 + 1/6 = 1/2 — collapses to its closed form at compile time, compiled exact.
    assert_exact_output("## Main\nLet x: Rational be 1 / 3 + 1 / 6.\nShow x.", "1/2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_bare_division_still_floors() {
    // The floor default is untouched on the compiled path — no Rational type, no change.
    assert_exact_output("## Main\nShow 7 / 2.", "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_int_widens_when_mixed_with_a_rational() {
    // Compiled path: the Int operand widens to LogosRational in mixed arithmetic.
    // A runtime Rational variable + an Int:
    assert_exact_output("## Main\nLet r: Rational be 1 / 2.\nLet s be r + 3.\nShow s.", "7/2");
    // An Int literal + a Rational sub-expression in a Rational binding:
    assert_exact_output("## Main\nLet x: Rational be 3 + 1 / 2.\nShow x.", "7/2");
    // Times/minus mix, one reducing to a whole:
    assert_exact_output("## Main\nLet r: Rational be 1 / 2.\nLet a be r * 4.\nShow a.", "2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_every_rational_division_combination_is_exact() {
    // Rational / Int, Int / Rational, Rational / Rational — all exact in compiled Rust.
    assert_exact_output("## Main\nLet r: Rational be 1 / 2.\nLet q be r / 2.\nShow q.", "1/4");
    assert_exact_output("## Main\nLet r: Rational be 1 / 2.\nLet q be 3 / r.\nShow q.", "6");
    assert_exact_output(
        "## Main\nLet a: Rational be 1 / 2.\nLet b: Rational be 1 / 3.\nLet q be a / b.\nShow q.",
        "3/2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_floor_ceiling_round_abs_of_a_rational_are_exact() {
    // The compiled path uses LogosRational's EXACT floor/ceil/round (BigInt num/den),
    // never a lossy `as f64`. abs of a rational stays a rational.
    assert_exact_output("## Main\nLet r: Rational be 7 / 2.\nShow floor(r).", "3");
    assert_exact_output("## Main\nLet r: Rational be 7 / 2.\nShow ceil(r).", "4");
    assert_exact_output("## Main\nLet r: Rational be 7 / 2.\nShow round(r).", "4");
    assert_exact_output("## Main\nLet r: Rational be -7 / 2.\nShow floor(r).", "-4");
    assert_exact_output("## Main\nLet r: Rational be -7 / 2.\nShow ceil(r).", "-3");
    assert_exact_output("## Main\nLet r: Rational be -7 / 2.\nShow abs(r).", "7/2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_rational_fold_obeys_the_comptime_toggle() {
    // The closed-form "lightning chaining" fold is the `Opt::Comptime` (CTFE) optimization,
    // threaded through `resolve_divisions`. Default ON: `1/3 + 1/6` collapses to a single
    // closed-form divide — no runtime `.add(`. The VALUE is exact `1/2` either way.
    let on = common::run_logos("## Main\nLet x: Rational be 1 / 3 + 1 / 6.\nShow x.");
    assert!(on.success, "should compile+run:\n{}", on.stderr);
    assert_eq!(on.stdout.trim(), "1/2");
    assert!(
        !on.rust_code.contains(".add(&"),
        "the fold should remove the runtime add:\n{}",
        on.rust_code
    );

    // `## No comptime`: the fold is OFF — the chain stays a runtime add — yet the value is
    // identical. An output-neutral optimization, properly toggleable like every other.
    let off = common::run_logos(
        "## No comptime\n\n## Main\nLet x: Rational be 1 / 3 + 1 / 6.\nShow x.",
    );
    assert!(off.success, "should compile+run:\n{}", off.stderr);
    assert_eq!(off.stdout.trim(), "1/2");
    assert!(
        off.rust_code.contains(".add(&"),
        "without comptime the constant chain stays a runtime add:\n{}",
        off.rust_code
    );
}
