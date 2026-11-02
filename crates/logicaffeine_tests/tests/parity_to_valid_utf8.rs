//! Stdlib-parity — `toValidUTF8Bytes` (Go bytes.ToValidUTF8 / strings.ToValidUTF8, over the byte
//! view): replace each maximal run of invalid UTF-8 bytes with a replacement, copying valid
//! sequences through. Rejects overlong forms + surrogates (matches Go's utf8 acceptance).
//! Reference values from a faithful Go-algorithm mimic. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Replace invalid-UTF-8 runs with a replacement. go: bytes.ToValidUTF8 / strings.ToValidUTF8
#[test]
fn parity_to_valid_utf8() {
    let src = "## Main\n\
        Show length of toValidUTF8Bytes([97, 98, 99], [63]).\n\
        Show length of toValidUTF8Bytes([97, 255, 254, 98], [63]).\n\
        Show item 1 of toValidUTF8Bytes([97, 255, 254, 98], [63]).\n\
        Show item 2 of toValidUTF8Bytes([97, 255, 254, 98], [63]).\n\
        Show item 3 of toValidUTF8Bytes([97, 255, 254, 98], [63]).\n\
        Show length of toValidUTF8Bytes([237, 160, 128], [63]).\n\
        Show item 1 of toValidUTF8Bytes([237, 160, 128], [63]).\n\
        Show length of toValidUTF8Bytes([195, 169], [63]).\n";
    assert_all_tiers(src, "3\n3\n97\n63\n98\n1\n63\n2");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "3\n3\n97\n63\n98\n1\n63\n2");
}

/// A multi-byte replacement is emitted once per invalid run. go: bytes.ToValidUTF8
#[test]
fn parity_to_valid_utf8_multi_repl() {
    // two invalid bytes → ONE replacement of "??"; then valid ASCII 'x'
    let src = "## Main\n\
        Show length of toValidUTF8Bytes([255, 254, 120], [63, 63]).\n\
        Show item 1 of toValidUTF8Bytes([255, 254, 120], [63, 63]).\n\
        Show item 2 of toValidUTF8Bytes([255, 254, 120], [63, 63]).\n\
        Show item 3 of toValidUTF8Bytes([255, 254, 120], [63, 63]).\n";
    assert_all_tiers(src, "3\n63\n63\n120");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "3\n63\n63\n120");
}
