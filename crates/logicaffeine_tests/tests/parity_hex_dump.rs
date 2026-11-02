//! Stdlib-parity — `hexDump` (Go encoding/hex.Dump): a `hexdump -C` style dump — 8-digit hex
//! offset, up to 16 space-separated hex bytes (extra space after the 8th), then the printable-ASCII
//! gutter between `|` bars. Verified byte-for-byte against Go's documented output. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// hexdump -C style dump (single line). go: encoding/hex.Dump
#[test]
fn parity_hex_dump() {
    let src = "## Main\nShow hexDump(text_bytes(\"Go is fun\")).\n";
    assert_all_tiers(
        src,
        "00000000  47 6f 20 69 73 20 66 75  6e                       |Go is fun|",
    );
    assert_compiled_equals_interpreted(src);
    assert_c_output(
        src,
        "00000000  47 6f 20 69 73 20 66 75  6e                       |Go is fun|",
    );
}

/// Multi-line dump with a second offset row. go: encoding/hex.Dump
#[test]
fn parity_hex_dump_multiline() {
    let src = "## Main\nShow hexDump(text_bytes(\"0123456789abcdefghij\")).\n";
    assert_all_tiers(src, "00000000  30 31 32 33 34 35 36 37  38 39 61 62 63 64 65 66  |0123456789abcdef|\n00000010  67 68 69 6a                                       |ghij|");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "00000000  30 31 32 33 34 35 36 37  38 39 61 62 63 64 65 66  |0123456789abcdef|\n00000010  67 68 69 6a                                       |ghij|");
}
