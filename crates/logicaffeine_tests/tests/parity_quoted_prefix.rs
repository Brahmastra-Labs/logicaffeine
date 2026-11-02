//! Stdlib-parity — `quotedPrefix` (Go strconv.QuotedPrefix): the quoted-string literal at the start
//! of the input, through its matching close quote (escapes respected). Verified against a Go mimic.
//! tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// The quoted-string prefix of s. go: strconv.QuotedPrefix
#[test]
fn parity_quoted_prefix() {
    // inputs via text_from_bytes for exact bytes:
    //   "abc"def            -> "abc"
    //   "a\"b"x  (34,97,92,34,98,34,120) -> "a\"b" (34,97,92,34,98,34)
    //   `raw`tail           -> `raw`
    let src = "## Main\n\
        Show quotedPrefix(text_from_bytes([34, 97, 98, 99, 34, 100, 101, 102])).\n\
        Show length of text_bytes(quotedPrefix(text_from_bytes([34, 97, 92, 34, 98, 34, 120]))).\n\
        Show quotedPrefix(text_from_bytes([96, 114, 97, 119, 96, 116, 97, 105, 108])).\n";
    assert_all_tiers(src, "\"abc\"\n6\n`raw`");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "\"abc\"\n6\n`raw`");
}
