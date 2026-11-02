//! Full-audit: the `compress` checksums on the NATIVE tiers (AOT-Rust + C), beyond the tree-walker/VM
//! `assert_all_tiers` cover. crc32/adler32 use the Word32 ring ops (`word32`/`word_and`/`word32Shr`/
//! `word_not`/generic `xor`/`intOfWord32`), so this proves those lower correctly through the AOT-Rust and
//! C backends. Expected values are AUTHORITATIVE (real zlib): crc32("123456789")=3421780262,
//! adler32("123456789")=152961502.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted};

const NINE_CRC: &str = "## Main
Let mutable d be a new Seq of Int.
Repeat for c from 49 to 57:
    Push c to d.
Show crc32(d).
";
const NINE_ADLER: &str = "## Main
Let mutable d be a new Seq of Int.
Repeat for c from 49 to 57:
    Push c to d.
Show adler32(d).
";

#[test]
fn crc32_on_aot_rust() {
    assert_compiled_equals_interpreted(NINE_CRC);
    // full 256-byte range too (reference crc32=688229491)
    assert_compiled_equals_interpreted(
        "## Main
Let mutable r be a new Seq of Int.
Repeat for i from 0 to 255:
    Push i to r.
Show crc32(r).
",
    );
}

#[test]
fn crc32_on_c() {
    assert_c_output(NINE_CRC, "3421780262");
}

#[test]
fn adler32_on_aot_rust() {
    assert_compiled_equals_interpreted(NINE_ADLER);
}

#[test]
fn adler32_on_c() {
    assert_c_output(NINE_ADLER, "152961502");
}
