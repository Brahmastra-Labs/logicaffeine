//! Stdlib-parity — `shortenText` (Python textwrap.shorten): collapse whitespace, then keep whole
//! words that fit `width`, appending ` [...]` when text is dropped (or `[...]` when nothing fits).
//! Expected outputs verified against CPython. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Collapse whitespace + word-boundary truncate with a placeholder. py: textwrap.shorten
#[test]
fn parity_str_shorten() {
    let src = "## Main\n\
        Show shortenText(\"Hello  world  this is long\", 15).\n\
        Show shortenText(\"Hello world\", 20).\n\
        Show shortenText(\"a b c d e f g h\", 7).\n\
        Show shortenText(\"aaaaaaaaaaaaaaaaaa\", 5).\n";
    let expected = "Hello [...]\nHello world\na [...]\n[...]";
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, expected);
}
