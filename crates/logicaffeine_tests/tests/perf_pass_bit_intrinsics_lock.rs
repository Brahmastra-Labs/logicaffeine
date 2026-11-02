//! Pass — native-width bit intrinsics for the stdlib `bits` ops.
//!
//! WHY. `rotateLeft(x, n, 32)` / `rotateRight` / `reverseBits(x, 32)` / `reverseBytes(x, 32)` are
//! width-parameterized and compile to a masked LOGOS loop that LLVM will NOT pattern-match to a
//! rotate/bit-reverse/byte-swap. C/Rust rivals lower the same operation to a single CPU
//! instruction (ROL/ROR, bit-reverse, BSWAP) — so LOGOS was ~21x slower (`bits_rotate_left`
//! measured 0.048x vs C). With a COMPILE-TIME native word width (8/16/32/64) the body is EXACTLY
//! one Rust intrinsic; the codegen emits `(x as uW).rotate_left(n)` / `.reverse_bits()` /
//! `.swap_bytes()` at the call site.
//!
//! SOUNDNESS. The rewrite is exact — a rotate/reverse of the low W bits has a single universal
//! meaning at a fixed native width — and the cross-tier partners below pin interpreter ≡ VM ≡ AOT
//! on the boundary values (top-bit wrap, byte permutation). A non-native or non-literal width
//! falls through to the general stdlib loop unchanged.

mod common;
use common::{assert_all_tiers, compile_to_rust};

// 1. SHAPE: a native-width rotate lowers to the Rust intrinsic, not a masked call.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_rotate_lowers_to_intrinsic() {
    let rust = compile_to_rust("## Main\nShow rotateLeft(1, 7, 32).\n").expect("compiles");
    assert!(
        rust.contains(".rotate_left(") && rust.contains("as u32"),
        "rotateLeft(_, _, 32) must lower to `(x as u32).rotate_left(..)` — the single-instruction \
         intrinsic. Emitted:\n{rust}"
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_reverse_bits_and_bytes_lower_to_intrinsics() {
    let rb = compile_to_rust("## Main\nShow reverseBits(1, 32).\n").expect("compiles");
    assert!(
        rb.contains(".reverse_bits()") && rb.contains("as u32"),
        "reverseBits(_, 32) must lower to `.reverse_bits()`. Emitted:\n{rb}"
    );
    let by = compile_to_rust("## Main\nShow reverseBytes(305419896, 32).\n").expect("compiles");
    assert!(
        by.contains(".swap_bytes()") && by.contains("as u32"),
        "reverseBytes(_, 32) must lower to `.swap_bytes()`. Emitted:\n{by}"
    );
}

// 2. CORRECTNESS across tiers — the intrinsic must equal the masked-loop semantics exactly.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn rotate_left_intrinsic_all_tiers() {
    // 1<<7 = 128
    assert_all_tiers("## Main\nShow rotateLeft(1, 7, 32).\n", "128");
    // 0x80000000 rotated left 1 within 32 bits wraps the top bit to the bottom -> 1
    assert_all_tiers("## Main\nShow rotateLeft(2147483648, 1, 32).\n", "1");
    // wrap-around shift count (n >= width): n mod width
    assert_all_tiers("## Main\nShow rotateLeft(1, 39, 32).\n", "128");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn rotate_right_intrinsic_all_tiers() {
    // 1 rotated right 1 within 32 bits -> top bit set -> 0x80000000
    assert_all_tiers("## Main\nShow rotateRight(1, 1, 32).\n", "2147483648");
    assert_all_tiers("## Main\nShow rotateRight(128, 7, 32).\n", "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn reverse_bits_intrinsic_all_tiers() {
    // reverse low 32 bits of 1 -> bit 0 moves to bit 31 -> 0x80000000
    assert_all_tiers("## Main\nShow reverseBits(1, 32).\n", "2147483648");
    // width 8: bit 0 -> bit 7 -> 128
    assert_all_tiers("## Main\nShow reverseBits(1, 8).\n", "128");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn reverse_bytes_intrinsic_all_tiers() {
    // 0x12345678 -> 0x78563412 = 2018915346
    assert_all_tiers("## Main\nShow reverseBytes(305419896, 32).\n", "2018915346");
    // width 16: 0x1234 -> 0x3412 = 13330
    assert_all_tiers("## Main\nShow reverseBytes(4660, 16).\n", "13330");
}

// 3. GUARD: a NON-native width keeps the general stdlib loop (no intrinsic), still correct.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn non_native_width_keeps_general_path() {
    let rust = compile_to_rust("## Main\nShow rotateLeft(1, 2, 20).\n").expect("compiles");
    // width 20 is not a native u-type, so no `as u20` / intrinsic at the call
    assert!(
        !rust.contains("as u20"),
        "a non-native width must NOT synthesize a bogus `u20` cast. Emitted:\n{rust}"
    );
    // 1 << 2 = 4 within 20 bits
    assert_all_tiers("## Main\nShow rotateLeft(1, 2, 20).\n", "4");
}
