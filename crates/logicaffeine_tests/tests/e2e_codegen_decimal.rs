//! E2E Codegen Tests: exact base-10 `Decimal` money (the AOT compile-to-Rust tier).
//!
//! The base type, tree-walker, VM, and wire codec already carry `Decimal` exactly. These
//! tests prove the SAME exactness on the compiled-to-Rust path: `decimal("19.99")` becomes
//! a `LogosDecimal`, `+ − ×` stay exact (scale preserved), and money never float-drifts.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_decimal_money_multiplies_exactly() {
    // 3 items at $19.99 → exactly 59.97 on compiled-native code.
    assert_exact_output(
        "## Main\nLet price be decimal(\"19.99\").\nLet total be price * 3.\nShow total.",
        "59.97",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_decimal_has_no_float_drift() {
    // 0.1 + 0.2 == 0.3 EXACTLY — the classic f64 trap, gone, on the compiled path.
    assert_exact_output(
        "## Main\nLet a be decimal(\"0.1\").\nLet b be decimal(\"0.2\").\nShow a + b.",
        "0.3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_decimal_addition_preserves_scale() {
    // 19.99 + 0.01 = 20.00, scale preserved (not "20").
    assert_exact_output(
        "## Main\nLet x be decimal(\"19.99\").\nShow x + decimal(\"0.01\").",
        "20.00",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_decimal_division_widens_to_exact_rational() {
    // 1 / 8 = 0.125 → the exact rational 1/8 (Decimal ÷ widens to Rational, never a float).
    assert_exact_output(
        "## Main\nLet a be decimal(\"1\").\nLet b be decimal(\"8\").\nShow a / b.",
        "1/8",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_decimal_comparison() {
    // Cross-value comparison compiles and runs: 19.99 > 10.00.
    assert_exact_output(
        "## Main\nLet price be decimal(\"19.99\").\nShow price > decimal(\"10\").",
        "true",
    );
}
