//! Stdlib-parity — `b2a_uu` (Python binascii.b2a_uu: one uuencoded line) and `a2b_uu` (decode).
//! The exact encoding "#0V%T" for "Cat" is CPython-verified; round-trips prove the decoder.
//! tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Uuencode one line: length byte + 3→4 six-bit chars + newline. py: binascii.b2a_uu
#[test]
fn parity_b2a_uu() {
    // b2a_uu("Cat") = "#0V%T\n"; trimEnd drops the trailing newline for a clean assertion.
    let src = "## Main\n\
        Let e be b2a_uu(text_bytes(\"Cat\")).\n\
        Show trimEnd(e).\n";
    assert_all_tiers(src, "#0V%T");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "#0V%T");
}

/// Decode a uuencoded line via the leading length byte. py: binascii.a2b_uu
#[test]
fn parity_a2b_uu() {
    let src = "## Main\n\
        Show text_from_bytes(a2b_uu(b2a_uu(text_bytes(\"Cat\")))).\n\
        Show text_from_bytes(a2b_uu(b2a_uu(text_bytes(\"hello world\")))).\n";
    assert_all_tiers(src, "Cat\nhello world");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "Cat\nhello world");
}
