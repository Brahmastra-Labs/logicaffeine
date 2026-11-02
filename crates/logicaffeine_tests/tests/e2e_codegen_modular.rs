//! E2E Codegen Tests: modular arithmetic ℤ/nℤ (the AOT compile-to-Rust tier).
//!
//! The base type, tree-walker, VM, and wire codec already carry `Modular` exactly. These tests
//! prove the SAME on the compiled-to-Rust path: `modular(v, n)` becomes a `LogosModular`, `+ − ×`
//! wrap in the ring, `pow` is fast modular exponentiation, and `÷` multiplies by the inverse.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{assert_all_tiers, assert_exact_output};

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_modular_reduces_on_construction() {
    assert_exact_output("## Main\nLet a be modular(10, 7).\nShow a.", "3 (mod 7)");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_modular_multiplication_wraps() {
    // 3 · 5 = 15 ≡ 1 (mod 7).
    assert_exact_output(
        "## Main\nLet a be modular(3, 7).\nLet b be modular(5, 7).\nShow a * b.",
        "1 (mod 7)",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_modular_exponentiation() {
    // 3^4 = 81 ≡ 1 (mod 5) — fast modular exponentiation on compiled-native code.
    assert_exact_output(
        "## Main\nLet g be modular(3, 5).\nShow pow(g, 4).",
        "1 (mod 5)",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_modular_division_by_inverse() {
    // 1 / 3 ≡ 5 (mod 7) since 3·5 = 15 ≡ 1.
    assert_exact_output(
        "## Main\nLet a be modular(1, 7).\nLet b be modular(3, 7).\nShow a / b.",
        "5 (mod 7)",
    );
}

/// Direct tree-walker ≡ VM lock for the full ring/field/exponentiation surface — the one tier pair
/// `assert_exact_output` (AOT-Rust) and `linked_modular` (WASM≡VM) don't assert against each other.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn modular_ring_field_pow_are_tier_identical() {
    assert_all_tiers(
        "## Main\n    Show modular(5, 7) + modular(4, 7).\n    Show modular(2, 7) - modular(5, 7).\n    Show modular(6, 7) * modular(6, 7).\n    Show modular(3, 7) / modular(5, 7).\n    Show pow(modular(2, 7), 5).\n",
        "2 (mod 7)\n4 (mod 7)\n1 (mod 7)\n2 (mod 7)\n4 (mod 7)",
    );
}
