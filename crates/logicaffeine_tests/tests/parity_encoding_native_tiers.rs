//! Native-tier (AOT-Rust + C) byte-identity lock for the pure-LOGOS `encoding.lg` codecs.
//!
//! The ordinary `parity_encoding.rs` tests run through `assert_all_tiers` (tree-walker ≡ VM only).
//! The codecs also claim `aot_native` and `aot_c`, but those tiers compile the LOGOS bit-shift /
//! mask arithmetic to real Rust / C — and a wide accumulator shift is exactly where an integer
//! codec can silently diverge (e.g. base32 packs five bytes into a 40-bit accumulator and shifts
//! by up to 35). These tests pin the encoders/decoders byte-for-byte on both native backends, so a
//! codegen change to shift or mask lowering goes RED here rather than corrupting encoded bytes on a
//! tier the tree-walker/VM differential never exercises.

#![allow(unused_imports, dead_code)]
mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted};

/// Base32 (RFC 4648) on the AOT-Rust backend — the 35-bit accumulator shift must lower correctly,
/// and a full round-trip must reproduce the input byte-for-byte.
#[test]
fn base32_on_aot_rust() {
    assert_compiled_equals_interpreted("## Main\nShow base32Encode(\"foobar\").\n");
    assert_compiled_equals_interpreted("## Main\nShow base32Decode(\"MZXW6YTBOI======\").\n");
    assert_compiled_equals_interpreted(
        "## Main\nShow base32Decode(base32Encode(\"AOT-Rust base32 round-trip\")).\n",
    );
}

/// Base32 (RFC 4648) on the C backend.
#[test]
fn base32_on_c() {
    assert_c_output(
        "## Main\nShow base32Encode(\"foobar\").\n",
        "MZXW6YTBOI======",
    );
    assert_c_output("## Main\nShow base32Decode(\"MY======\").\n", "f");
    assert_c_output(
        "## Main\nShow base32Decode(base32Encode(\"C base32 round-trip\")).\n",
        "C base32 round-trip",
    );
}

/// Extended-hex Base32 (RFC 4648 §7) on both native backends — same 35-bit shift, `0-9A-V` alphabet.
#[test]
fn base32hex_on_native_tiers() {
    assert_compiled_equals_interpreted("## Main\nShow base32hexEncode(\"foobar\").\n");
    assert_compiled_equals_interpreted(
        "## Main\nShow base32hexDecode(base32hexEncode(\"AOT b32hex round-trip\")).\n",
    );
    assert_c_output(
        "## Main\nShow base32hexEncode(\"foobar\").\n",
        "CPNMUOJ1E8======",
    );
    assert_c_output(
        "## Main\nShow base32hexDecode(base32hexEncode(\"C b32hex round-trip\")).\n",
        "C b32hex round-trip",
    );
}

/// Ascii85 (radix-85) on both native backends — the `v * 52200625` group arithmetic and the
/// `z` zero-abbreviation must lower correctly (a codec whose accumulator is built by MULTIPLICATION
/// rather than bit-shifts, so it exercises a different codegen path from base32/base64).
#[test]
fn ascii85_on_native_tiers() {
    assert_compiled_equals_interpreted("## Main\nShow ascii85Encode(\"Hello, World!\").\n");
    assert_compiled_equals_interpreted(
        "## Main\nShow ascii85Decode(ascii85Encode(\"AOT ascii85 round-trip\")).\n",
    );
    assert_c_output("## Main\nShow ascii85Encode(\"foobar\").\n", "AoDTs@<)");
    assert_c_output(
        "## Main\nShow ascii85Decode(ascii85Encode(\"C ascii85 round-trip\")).\n",
        "C ascii85 round-trip",
    );
}

/// Base85 (RFC 1924) on both native backends — exercises a Seq-literal alphabet lookup
/// (`item (d+1) of alphabet`) on encode and a linear-search `b85Val` on decode, a different shape
/// again from the arithmetic-only codecs.
#[test]
fn base85_on_native_tiers() {
    assert_compiled_equals_interpreted("## Main\nShow base85Encode(\"Hello, World!\").\n");
    assert_compiled_equals_interpreted(
        "## Main\nShow base85Decode(base85Encode(\"AOT base85 round-trip\")).\n",
    );
    assert_c_output("## Main\nShow base85Encode(\"foobar\").\n", "W^Zp|VR8");
    assert_c_output(
        "## Main\nShow base85Decode(base85Encode(\"C base85 round-trip\")).\n",
        "C base85 round-trip",
    );
}

/// Base16 (uppercase hex) + URL-safe Base64 on both native backends — base64url reuses
/// `base64Encode`/`base64Decode` then remaps `+/` ↔ `-_`, so this also locks the same-module reuse.
#[test]
fn base16_and_base64url_on_native_tiers() {
    assert_compiled_equals_interpreted("## Main\nShow base16Encode(\"Hello?>\").\n");
    assert_compiled_equals_interpreted(
        "## Main\nShow base64UrlDecode(base64UrlEncode(\"AOT url?>\")).\n",
    );
    assert_c_output("## Main\nShow base16Encode(\"foo\").\n", "666F6F");
    assert_c_output("## Main\nShow base64UrlEncode(\"???\").\n", "Pz8_");
    assert_c_output(
        "## Main\nShow base64UrlDecode(base64UrlEncode(\"C url?>\")).\n",
        "C url?>",
    );
}

/// Base64 + hex on both native backends — the sibling codecs share the same byte-bridge and
/// shift/mask machinery, so locking them here guards the whole `encoding.lg` native-tier surface.
#[test]
fn base64_and_hex_on_native_tiers() {
    assert_compiled_equals_interpreted(
        "## Main\nShow base64Decode(base64Encode(\"native round-trip!\")).\n",
    );
    assert_compiled_equals_interpreted("## Main\nShow fromHex(toHex(\"native hex!\")).\n");
    assert_c_output("## Main\nShow base64Encode(\"Man\").\n", "TWFu");
    assert_c_output("## Main\nShow toHex(\"AB\").\n", "4142");
}
