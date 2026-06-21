//! Fixed-window byte-compare → element-wise compares (the `bcmp` idiom).
//!
//! A needle-length window walked byte-by-byte with an early break and a `match`
//! flag:
//!
//! ```text
//! While j < needleLen:
//!     If item (i+j) of text is not item (j+1) of needle:
//!         Set match to 0.
//!         Set j to needleLen.   # break
//!     Set j to j + 1.
//! ```
//!
//! is exactly `match = 0 unless text[i..i+k] == needle[..k]`. A CONSTANT window
//! length lowers to C's unrolled element-wise byte compares (which LLVM keeps
//! inline), eliminating the scalar inner loop and branch.
//!
//! NOTE: when the *whole* nest is a naive-occurrence-COUNT (`If match { count =
//! count + 1 }` over every start position), the more aggressive SIMD kernel in
//! `peephole::try_emit_naive_search` claims the entire nest instead — see
//! `phase_strsearch`. This idiom still fires for window compares that are not a
//! full count, e.g. recording the last match position below.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::compile_to_rust;

const SEARCH: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let text be "abcabcabcXXXXXabcdeXXXXX".
Let needle be "XXXXX".
Let needleLen be 5.
Let textLen be length of text.
Let mutable lastMatch be 0.
Let mutable i be 1.
While i is at most textLen - needleLen + 1:
    Let mutable match be 1.
    Let mutable j be 0.
    While j is less than needleLen:
        If item (i + j) of text is not item (j + 1) of needle:
            Set match to 0.
            Set j to needleLen.
        Set j to j + 1.
    If match equals 1:
        Set lastMatch to i.
    Set i to i + 1.
Show lastMatch.
"#;

/// The needle-length byte-compare loop collapses, and because the window is a
/// CONSTANT length, it lowers to C's UNROLLED element-wise byte compares
/// (`text.as_bytes()[..] == needle.as_bytes()[..] && …`) — which LLVM keeps
/// inline — NOT the slice inequality (LLVM lowers that to a runtime `bcmp` CALL
/// per position, the original string_search loss). The scalar inner `while`
/// over `j` is gone either way.
#[test]
fn fixed_window_byte_compare_becomes_element_wise() {
    let rust = compile_to_rust(SEARCH).unwrap();
    assert!(
        rust.contains("text.as_bytes()[") && rust.contains("== needle.as_bytes()["),
        "the constant-length window must lower to element-wise byte compares \
         (`text.as_bytes()[..] == needle.as_bytes()[..] && …`). Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("] != &needle.as_bytes()"),
        "the slower slice inequality (LLVM → runtime `bcmp` call) must be gone. Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("while (j"),
        "the scalar inner byte-compare loop over `j` must be gone. Got:\n{}",
        rust
    );
}
