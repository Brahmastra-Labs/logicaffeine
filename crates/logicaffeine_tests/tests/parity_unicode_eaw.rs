//! Stdlib-parity — `unicodedataEastAsianWidth` (Python unicodedata.east_asian_width): the East
//! Asian Width property (N/Na/A/W/H/F) via a range table from the UCD. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// East Asian Width class of a codepoint. py: unicodedata.east_asian_width
#[test]
fn parity_unicode_east_asian_width() {
    // 'A'=Na, U+4E00 一 =W, U+FF21 Ａ =F
    let src = "## Main\n\
        Show unicodedataEastAsianWidth(65).\n\
        Show unicodedataEastAsianWidth(19968).\n\
        Show unicodedataEastAsianWidth(65313).\n";
    assert_all_tiers(src, "Na\nW\nF");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "Na\nW\nF");
}
