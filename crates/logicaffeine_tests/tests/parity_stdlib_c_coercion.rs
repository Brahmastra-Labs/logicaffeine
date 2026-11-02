//! C-backend companion to `parity_stdlib_aot_rust.rs`'s int→Real call-site coercion lock.
//!
//! The AOT-Rust backend needed an explicit `(arg) as f64` coercion so a `Real`-parameter function
//! called with an integer literal type-checks (Rust has no implicit numeric conversion). The C
//! backend gets this for free — C implicitly converts an `int` argument to a `double` parameter —
//! so `sin(0)` and a user `Real`-parameter function called with an integer already compile and run
//! correctly through `gcc`. These tests LOCK that: they pin the C tier to the same int→Real
//! call-boundary behavior as the interpreter/VM/AOT-Rust, so a future C-emit change that broke the
//! implicit coercion (e.g. emitting a stricter prototype) would go RED here rather than silently
//! diverging on a tier the ordinary parity tests never compile.

#![allow(unused_imports, dead_code)]
mod common;
use common::assert_c_output;

/// Control: a float argument to a `Real` (`double`) parameter compiles+runs on the C backend.
#[test]
fn c_real_param_float_arg() {
    assert_c_output("## Main\nShow floor(sin(0.0) * 1000.0).\n", "0");
}

/// An INTEGER argument to a stdlib `Real` parameter (`sin(0)`) — C coerces int→double implicitly.
#[test]
fn c_stdlib_real_param_int_arg() {
    assert_c_output("## Main\nShow floor(sin(0) * 1000.0).\n", "0");
}

/// An INTEGER argument to a USER-defined `Real`-parameter function.
#[test]
fn c_user_real_param_int_arg() {
    assert_c_output(
        "## To scaleHalf (x: Real) -> Real:\n    Return x / 2.0.\n## Main\nShow floor(scaleHalf(5) * 100.0).\n",
        "250",
    );
}

/// Mixed `Int`/`Real` parameters: the `Int` parameter keeps an integer argument, the `Real`
/// parameter takes an integer that coerces — both on the C tier.
#[test]
fn c_mixed_int_real_params() {
    assert_c_output(
        "## To mix (a: Int) and (b: Real) -> Real:\n    Return b.\n## Main\nShow floor(mix(1, 2) * 100.0).\n",
        "200",
    );
}
