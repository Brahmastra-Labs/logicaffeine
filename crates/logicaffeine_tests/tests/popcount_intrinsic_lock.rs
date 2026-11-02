//! `popcount` (and the shift-and-mask population-count idiom generally) must
//! lower to the hardware `count_ones` intrinsic, not an O(bit-width) loop — the
//! general recognizer in `optimize::popcount_leaf`. Correctness is preserved on
//! every tier: the collapse (`while v>0 {total += v&1; v>>=1}` →
//! `if v>0 {total += count_ones(v); v=0}`) is behavior-identical for every input.

mod common;
use common::{assert_all_tiers, assert_c_output, compile_to_rust};

/// A runtime argument keeps `popcount` from being constant-folded, so the
/// stdlib function's body is emitted — and it must use `count_ones`, not a
/// `>> 1` shift loop.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn popcount_lowers_to_count_ones_intrinsic() {
    let code = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show popcount(n).
"#;
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("count_ones"),
        "popcount must lower to the count_ones intrinsic, not a shift loop. Emitted:\n{rust}"
    );
}

/// Correctness across all tiers for a spread of values (0, powers, all-ones,
/// wide) — the collapse must be exact everywhere.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn popcount_is_correct_on_all_tiers() {
    assert_all_tiers("## Main\nShow popcount(0).\n", "0");
    assert_all_tiers("## Main\nShow popcount(1).\n", "1");
    assert_all_tiers("## Main\nShow popcount(255).\n", "8");
    assert_all_tiers("## Main\nShow popcount(1023).\n", "10");
    assert_all_tiers("## Main\nShow popcount(305419896).\n", "13"); // 0x12345678
}

/// `bitLength` → `leading_zeros` (LZCNT) and `trailingZeros` → `trailing_zeros`
/// (TZCNT): distinct idioms, each collapsed to its own hardware intrinsic and
/// correct across every tier.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bitlength_and_trailingzeros_stay_correct() {
    assert_all_tiers("## Main\nShow bitLength(255).\n", "8");
    assert_all_tiers("## Main\nShow bitLength(256).\n", "9");
    assert_all_tiers("## Main\nShow bitLength(0).\n", "0");
    assert_all_tiers("## Main\nShow bitLength(1).\n", "1");
    assert_all_tiers("## Main\nShow trailingZeros(8, 64).\n", "3");
    assert_all_tiers("## Main\nShow trailingZeros(1, 64).\n", "0");
    assert_all_tiers("## Main\nShow trailingZeros(0, 64).\n", "64");
    assert_all_tiers("## Main\nShow trailingZeros(48, 64).\n", "4"); // 0b110000
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bitlength_lowers_to_leading_zeros_intrinsic() {
    let code = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt(item 2 of args()).
Show bitLength(n).
"#;
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("leading_zeros"),
        "bitLength must lower to the leading_zeros (LZCNT) intrinsic. Emitted:\n{rust}"
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn trailingzeros_lowers_to_trailing_zeros_intrinsic() {
    let code = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt(item 2 of args()).
Show trailingZeros(n, 64).
"#;
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("trailing_zeros"),
        "trailingZeros must lower to the trailing_zeros (TZCNT) intrinsic. Emitted:\n{rust}"
    );
}

/// The C tier (`__builtin_popcountll`/`clzll`/`ctzll`, including the 0→64 guards)
/// must compile, run, and produce the identical counts. Exercised end-to-end
/// through the C backend, not just the tree-walker/VM.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bit_intrinsics_are_correct_on_the_c_tier() {
    assert_c_output("## Main\nShow popcount(305419896).\n", "13"); // 0x12345678
    assert_c_output("## Main\nShow popcount(0).\n", "0");
    assert_c_output("## Main\nShow bitLength(255).\n", "8");
    assert_c_output("## Main\nShow bitLength(256).\n", "9");
    assert_c_output("## Main\nShow bitLength(0).\n", "0");
    assert_c_output("## Main\nShow trailingZeros(48, 64).\n", "4");
    assert_c_output("## Main\nShow trailingZeros(0, 64).\n", "64");
}
