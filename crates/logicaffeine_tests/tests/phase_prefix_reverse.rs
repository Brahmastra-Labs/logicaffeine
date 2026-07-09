//! Phase: converging-swap → slice `.reverse()` recognition.
//!
//! The in-place prefix-reversal idiom
//! ```text
//! While lo < hi:
//!     Let tmp be item lo of C.
//!     Set item lo of C to item hi of C.
//!     Set item hi of C to tmp.
//!     Set lo to lo + 1.
//!     Set hi to hi - 1.
//! ```
//! is lowered to a single slice `C[(lo-1)..hi].reverse()`. This both removes the
//! per-element bounds checks (one slice check instead of N) AND lets LLVM emit a
//! vectorized reverse (the SIMD `pshufd`/`movdqu` loop that gcc/clang generate for
//! C's fannkuch flip), instead of a scalar swap loop whose panic branches block
//! auto-vectorization. General: it fires on any in-place reversal.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_exact_output, compile_to_rust};

/// The fannkuch flip loop is a converging swap on `perm` (prefix [1, k]); it must
/// lower to `.reverse()`. (n constant-folds to 7; golden output 228 / 16.)
const FANNKUCH_N7: &str = r#"## Main
Let n be 7.
Let mutable perm1 be a new Seq of Int.
Let mutable count be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to perm1.
    Push 0 to count.
    Set i to i + 1.
Let mutable maxFlips be 0.
Let mutable checksum be 0.
Let mutable permCount be 0.
Let mutable r be n.
Let mutable done be 0.
While done equals 0:
    While r is greater than 1:
        Set item r of count to r.
        Set r to r - 1.
    Let mutable perm be a new Seq of Int.
    Set i to 1.
    While i is at most n:
        Push item i of perm1 to perm.
        Set i to i + 1.
    Let mutable flips be 0.
    While item 1 of perm is not 0:
        Let k be item 1 of perm + 1.
        Let mutable lo be 1.
        Let mutable hi be k.
        While lo is less than hi:
            Let tmp be item lo of perm.
            Set item lo of perm to item hi of perm.
            Set item hi of perm to tmp.
            Set lo to lo + 1.
            Set hi to hi - 1.
        Set flips to flips + 1.
    If flips is greater than maxFlips:
        Set maxFlips to flips.
    If permCount % 2 equals 0:
        Set checksum to checksum + flips.
    Otherwise:
        Set checksum to checksum - flips.
    Set permCount to permCount + 1.
    Set done to 1.
    While done equals 1:
        If r equals n:
            Set done to 2.
        Otherwise:
            Let perm0 be item 1 of perm1.
            Set i to 1.
            While i is at most r:
                Set item i of perm1 to item (i + 1) of perm1.
                Set i to i + 1.
            Set item (r + 1) of perm1 to perm0.
            Set item (r + 1) of count to (item (r + 1) of count) - 1.
            If item (r + 1) of count is greater than 0:
                Set done to 0.
            Otherwise:
                Set r to r + 1.
Show checksum.
Show maxFlips.
"#;

/// RED: the scalar swap loop must become a slice `.reverse()`.
#[test]
fn prefix_reverse_fannkuch_flip_emits_reverse() {
    let rust = compile_to_rust(FANNKUCH_N7).unwrap();
    assert!(
        rust.contains(".reverse()"),
        "the converging-swap flip loop must lower to a slice .reverse(), got:\n{rust}"
    );
}

/// Value-equivalence: the reverse must produce the identical result (228 / 16).
#[test]
fn prefix_reverse_fannkuch_value_equivalence() {
    assert_exact_output(FANNKUCH_N7, "228\n16");
}

/// General (non-fannkuch) in-place full reversal driven by a data-dependent
/// bound (so it is not unrolled/folded away). `a = [6,0,1,2,3,4,5]`; reverse the
/// suffix positions [2, 7] (1-based) → a becomes [6,5,4,3,2,1,0]; show item 2 = 5.
const GENERIC_REVERSE: &str = r#"## Main
Let mutable a be a new Seq of Int.
Push 6 to a.
Let mutable i be 0.
While i is less than 6:
    Push i to a.
    Set i to i + 1.
Let mutable lo be 2.
Let mutable hi be item 1 of a + 1.
While lo is less than hi:
    Let tmp be item lo of a.
    Set item lo of a to item hi of a.
    Set item hi of a to tmp.
    Set lo to lo + 1.
    Set hi to hi - 1.
Show item 2 of a.
"#;

#[test]
fn prefix_reverse_generic_emits_reverse() {
    let rust = compile_to_rust(GENERIC_REVERSE).unwrap();
    assert!(
        rust.contains(".reverse()"),
        "a general in-place reversal must lower to .reverse(), got:\n{rust}"
    );
}

#[test]
fn prefix_reverse_generic_value_equivalence() {
    assert_exact_output(GENERIC_REVERSE, "5");
}

/// Negative: a loop that increments BOTH indices (a copy/scan, not a converging
/// swap) must NOT be turned into a reverse.
const NOT_A_REVERSE: &str = r#"## Main
Let mutable a be a new Seq of Int.
Let mutable i be 0.
While i is less than 6:
    Push i to a.
    Set i to i + 1.
Let mutable lo be 1.
Let mutable hi be 2.
Let mutable sum be 0.
While lo is less than 6:
    Set sum to sum + item lo of a.
    Set lo to lo + 1.
    Set hi to hi + 1.
Show sum.
"#;

#[test]
fn prefix_reverse_skips_non_converging_loop() {
    let rust = compile_to_rust(NOT_A_REVERSE).unwrap();
    assert!(
        !rust.contains(".reverse()"),
        "a non-converging scan loop must not become a reverse, got:\n{rust}"
    );
    assert_exact_output(NOT_A_REVERSE, "10");
}
