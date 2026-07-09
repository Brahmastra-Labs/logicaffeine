//! E2E Codegen Tests: exact `Complex` arithmetic (the AOT compile-to-Rust tier).
//!
//! The base type, tree-walker, VM, and wire codec already carry `Complex` exactly. These
//! tests prove the SAME exactness on the compiled-to-Rust path: `complex(re, im)` becomes a
//! `LogosComplex`, `+ − × ÷` stay exact and closed, and `i·i = −1` holds with no float error.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_complex_i_squared_is_minus_one() {
    // The headline: i·i = −1, on compiled-native code.
    assert_exact_output("## Main\nLet i be complex(0, 1).\nShow i * i.", "-1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_complex_conjugate_product_is_real() {
    // (1+i)(1−i) = 1 − i² = 2.
    assert_exact_output(
        "## Main\nLet a be complex(1, 1).\nLet b be complex(1, -1).\nShow a * b.",
        "2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_complex_addition() {
    // (2+3i) + (1−i) = 3+2i.
    assert_exact_output(
        "## Main\nLet a be complex(2, 3).\nLet b be complex(1, -1).\nShow a + b.",
        "3+2i",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_complex_division_is_closed_and_exact() {
    // (3+4i)/(1+2i) = (3+4i)(1−2i)/5 = (11−2i)/5 — exact, the field is closed.
    assert_exact_output(
        "## Main\nLet a be complex(3, 4).\nLet b be complex(1, 2).\nShow a / b.",
        "11/5-2/5i",
    );
}
