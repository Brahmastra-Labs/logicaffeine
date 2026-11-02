//! Stdlib-parity — `base64B2a` (Python binascii.b2a_base64): single-line base64 with a trailing
//! newline and NO line wrapping (60 bytes → 80 base64 chars + `\n` = 81, vs encodebytes' 82).
//! `a2b_base64` is covered by base64Decodebytes (decode ignoring newlines). tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Single-line base64 + trailing newline, no wrapping. py: binascii.b2a_base64
#[test]
fn parity_b2a_base64() {
    let x60 = "x".repeat(60);
    let src = format!(
        "## Main\n\
         Let a be base64B2a(\"hi\").\n\
         Show length of a.\n\
         Show base64Decodebytes(a).\n\
         Let b be base64B2a(\"{x60}\").\n\
         Show length of b.\n"
    );
    assert_all_tiers(&src, "5\nhi\n81");
    assert_compiled_equals_interpreted(&src);
    assert_c_output(&src, "5\nhi\n81");
}
