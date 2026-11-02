//! Stdlib-parity — `unquote` (Go strconv.Unquote): parse a Go quoted string literal back to its raw
//! text. Handles `"..."`/`'...'` escapes (\a\b\f\n\r\t\v\\\'\", \xNN, \uNNNN, \UNNNNNNNN, octal) and
//! backtick raw literals. Verified against a Go-algorithm mimic + round-trip with quoteToASCII.
//! tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Parse a Go quoted string literal back to raw text. go: strconv.Unquote
#[test]
fn parity_unquote() {
    // inputs built via text_from_bytes to pass exact literal bytes (backslashes) unambiguously:
    //   "hi"           -> hi
    //   "a\tb"         -> a<TAB>b  (len 3, item 2 = 9)
    //   "\x41"         -> A
    //   roundtrip 😀   -> unquote(quoteToASCII(😀)) len 4, item 1 = 240
    //   `raw\n`        -> raw\n literal (len 5)
    let src = "## Main\n\
        Show unquote(text_from_bytes([34, 104, 105, 34])).\n\
        Show length of text_bytes(unquote(text_from_bytes([34, 97, 92, 116, 98, 34]))).\n\
        Show item 2 of text_bytes(unquote(text_from_bytes([34, 97, 92, 116, 98, 34]))).\n\
        Show unquote(text_from_bytes([34, 92, 120, 52, 49, 34])).\n\
        Show length of text_bytes(unquote(quoteToASCII(text_from_bytes([240, 159, 152, 128])))).\n\
        Show item 1 of text_bytes(unquote(quoteToASCII(text_from_bytes([240, 159, 152, 128])))).\n\
        Show length of text_bytes(unquote(text_from_bytes([96, 114, 97, 119, 92, 110, 96]))).\n";
    assert_all_tiers(src, "hi\n3\n9\nA\n4\n240\n5");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "hi\n3\n9\nA\n4\n240\n5");
}
