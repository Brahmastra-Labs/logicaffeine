//! Stdlib-parity — `unicodedataCategory` (Python unicodedata.category) + `unicodeIsMark`
//! (Go unicode.IsMark) + `unicodeIsSymbol` (Go unicode.IsSymbol), via a full-plane general-category
//! range table generated from the UCD. Categories verified against CPython. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Two-letter general category of a character. py: unicodedata.category
#[test]
fn parity_unicode_category() {
    let src = "## Main\n\
        Show unicodedataCategory(text_from_bytes([65])).\n\
        Show unicodedataCategory(text_from_bytes([97])).\n\
        Show unicodedataCategory(text_from_bytes([48])).\n\
        Show unicodedataCategory(text_from_bytes([32])).\n\
        Show unicodedataCategory(text_from_bytes([226, 130, 172])).\n\
        Show unicodedataCategory(text_from_bytes([240, 159, 152, 128])).\n";
    assert_all_tiers(src, "Lu\nLl\nNd\nZs\nSc\nSo");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "Lu\nLl\nNd\nZs\nSc\nSo");
}

/// True for a mark (Mn/Mc/Me). go: unicode.IsMark
#[test]
fn parity_unicode_is_mark() {
    let src = "## Main\n\
        Show unicodeIsMark(768).\n\
        Show unicodeIsMark(65).\n";
    assert_all_tiers(src, "true\nfalse");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "true\nfalse");
}

/// True for a symbol (Sm/Sc/Sk/So). go: unicode.IsSymbol
#[test]
fn parity_unicode_is_symbol() {
    let src = "## Main\n\
        Show unicodeIsSymbol(8364).\n\
        Show unicodeIsSymbol(43).\n\
        Show unicodeIsSymbol(65).\n";
    assert_all_tiers(src, "true\ntrue\nfalse");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "true\ntrue\nfalse");
}
