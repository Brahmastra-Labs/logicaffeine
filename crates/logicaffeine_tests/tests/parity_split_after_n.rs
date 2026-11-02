//! Stdlib-parity — `splitAfterN` (Go strings.SplitAfterN / bytes.SplitAfterN): split keeping the
//! separator attached, capped at n pieces (n=0 → none, n<0 → all), keeping the final piece even if
//! empty. Byte-level text manipulation. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Split after each separator, n-limited. go: strings.SplitAfterN / bytes.SplitAfterN
#[test]
fn parity_split_after_n() {
    let src = "## Main\n\
        Show item 1 of splitAfterN(\"a,b,c\", \",\", 0 - 1).\n\
        Show item 2 of splitAfterN(\"a,b,c\", \",\", 0 - 1).\n\
        Show item 3 of splitAfterN(\"a,b,c\", \",\", 0 - 1).\n\
        Show length of splitAfterN(\"a,b,c,d\", \",\", 2).\n\
        Show item 1 of splitAfterN(\"a,b,c,d\", \",\", 2).\n\
        Show item 2 of splitAfterN(\"a,b,c,d\", \",\", 2).\n\
        Show length of splitAfterN(\"x\", \",\", 0).\n";
    assert_all_tiers(src, "a,\nb,\nc\n2\na,\nb,c,d\n0");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "a,\nb,\nc\n2\na,\nb,c,d\n0");
}

/// Trailing separator keeps the final empty piece. go: strings.SplitAfterN
#[test]
fn parity_split_after_n_trailing() {
    // SplitAfterN("a,b,", ",", -1) = ["a,", "b,", ""] — 3 pieces, last empty
    let src = "## Main\n\
        Show length of splitAfterN(\"a,b,\", \",\", 0 - 1).\n\
        Show item 1 of splitAfterN(\"a,b,\", \",\", 0 - 1).\n\
        Show item 2 of splitAfterN(\"a,b,\", \",\", 0 - 1).\n";
    assert_all_tiers(src, "3\na,\nb,");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "3\na,\nb,");
}
