//! Stdlib-parity north-star corpus — domain: compress (RFC 1950/1951/1952 + checksums).
//! Each test mirrors one deduplicated capability of the C/Python/Rust/Go superset and is a real
//! `assert_all_tiers` behavioral spec whose expected values are AUTHORITATIVE — computed from real
//! zlib (Python's `zlib.crc32` / `zlib.adler32`). Byte-identical across tree-walker ≡ VM ≡ AOT ≡ C ≡ WASM.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// CRC-32 (IEEE 802.3, reflected poly 0xEDB88320) — the checksum used by gzip/PNG/ZIP.
/// provenance — python: zlib.crc32 / binascii.crc32 · go: hash/crc32.ChecksumIEEE · rust: (crc crates)
/// LOGOS surface: `crc32(data)`
#[test]
fn parity_compress_crc32() {
    // Reference (zlib): crc32("")=0, "a"=3904355907, "123456789"=3421780262, bytes(0..255)=688229491.
    assert_all_tiers(
        "## Main
Let mutable e be a new Seq of Int.
Show crc32(e).
Let mutable a be a new Seq of Int.
Push 97 to a.
Show crc32(a).
Let mutable nine be a new Seq of Int.
Repeat for c from 49 to 57:
    Push c to nine.
Show crc32(nine).
Let mutable r be a new Seq of Int.
Repeat for i from 0 to 255:
    Push i to r.
Show crc32(r).
",
        "0
3904355907
3421780262
688229491",
    );
}

/// Adler-32 (RFC 1950) — the checksum zlib streams carry in their trailer.
/// provenance — python: zlib.adler32 · go: hash/adler32.Checksum · rust: adler crate
/// LOGOS surface: `adler32(data)`
#[test]
fn parity_compress_adler32() {
    // Reference (zlib): adler32("")=1, "a"=6422626, "Wikipedia"=300286872, bytes(0..255)=2918612865.
    assert_all_tiers(
        "## Main
Let mutable e be a new Seq of Int.
Show adler32(e).
Let mutable a be a new Seq of Int.
Push 97 to a.
Show adler32(a).
Let mutable w be a new Seq of Int.
Push 87 to w.
Push 105 to w.
Push 107 to w.
Push 105 to w.
Push 112 to w.
Push 101 to w.
Push 100 to w.
Push 105 to w.
Push 97 to w.
Show adler32(w).
Let mutable r be a new Seq of Int.
Repeat for i from 0 to 255:
    Push i to r.
Show adler32(r).
",
        "1
6422626
300286872
2918612865",
    );
}
