//! Stdlib-parity — `base64Encodebytes` (Python base64.encodebytes: base64 with a newline every 76
//! chars + a trailing newline) and `base64Decodebytes` (Python base64.decodebytes: decode, ignoring
//! embedded newlines). Expected lengths verified against CPython. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

// 59-byte input → 80 base64 chars → wrapped "76\n4\n" = 82 bytes (CPython-verified).
const LONG: &str = "The quick brown fox jumps over the lazy dog, again for wrap";

/// Line-wrapped encode: newline every 76 chars + trailing newline. py: base64.encodebytes
#[test]
fn parity_base64_encodebytes() {
    let src = format!(
        "## Main\n\
         Let a be base64Encodebytes(\"hi\").\n\
         Show length of a.\n\
         Let b be base64Encodebytes(\"{LONG}\").\n\
         Show length of b.\n"
    );
    assert_all_tiers(&src, "5\n82");
    assert_compiled_equals_interpreted(&src);
    assert_c_output(&src, "5\n82");
}

/// Decode ignoring embedded newlines — round-trips the wrapped encoder's output. py: base64.decodebytes
#[test]
fn parity_base64_decodebytes() {
    let src = format!(
        "## Main\n\
         Show base64Decodebytes(base64Encodebytes(\"hi\")).\n\
         Show base64Decodebytes(base64Encodebytes(\"{LONG}\")).\n"
    );
    let expected = format!("hi\n{LONG}");
    assert_all_tiers(&src, &expected);
    assert_compiled_equals_interpreted(&src);
    assert_c_output(&src, &expected);
}
