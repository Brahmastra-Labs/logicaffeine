//! Stdlib-parity — `zlibCompress` (Python zlib.compress): produce a valid zlib stream (stored
//! DEFLATE blocks + Adler-32), round-trippable by any zlib decompressor. Verified: CPython
//! zlib.decompress accepts the output, and zlibDecompress round-trips it. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Compress into a valid (interoperable) zlib stream. py: zlib.compress
#[test]
fn parity_zlib_compress() {
    // round-trip through our own inflater + structural checks (header 0x78 0x01; "hi" -> 13 bytes)
    let src = "## Main\n\
        Show text_from_bytes(zlibDecompress(zlibCompress(text_bytes(\"round trip test\")))).\n\
        Show length of zlibCompress(text_bytes(\"hi\")).\n\
        Show item 1 of zlibCompress(text_bytes(\"hi\")).\n\
        Show item 2 of zlibCompress(text_bytes(\"hi\")).\n";
    assert_all_tiers(src, "round trip test\n13\n120\n1");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "round trip test\n13\n120\n1");
}
