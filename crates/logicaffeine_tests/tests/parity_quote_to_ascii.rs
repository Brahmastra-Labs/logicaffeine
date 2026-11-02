//! Stdlib-parity — `quoteToASCII` (Go strconv.QuoteToASCII): render text as a Go double-quoted,
//! ASCII-only string literal — every non-ASCII rune and non-printable byte backslash-escaped
//! (\\", \\\\, \\a\\b\\f\\n\\r\\t\\v, \\xNN, \\uNNNN, \\UNNNNNNNN). Verified byte-for-byte against a
//! faithful Go-algorithm mimic. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Go double-quoted ASCII-only string literal. go: strconv.QuoteToASCII
#[test]
fn parity_quote_to_ascii() {
    let src = "## Main\nShow quoteToASCII(\"hi\").\nShow quoteToASCII(text_from_bytes([97, 9, 98])).\nShow quoteToASCII(text_from_bytes([195, 169])).\nShow quoteToASCII(text_from_bytes([240, 159, 152, 128])).\nShow quoteToASCII(text_from_bytes([34, 120, 92])).\n";
    assert_all_tiers(
        src,
        "\"hi\"
\"a\\tb\"
\"\\u00e9\"
\"\\U0001f600\"
\"\\\"x\\\\\"",
    );
    assert_compiled_equals_interpreted(src);
    assert_c_output(
        src,
        "\"hi\"
\"a\\tb\"
\"\\u00e9\"
\"\\U0001f600\"
\"\\\"x\\\\\"",
    );
}
