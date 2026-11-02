//! Stdlib-parity — `indentText` (Python textwrap.indent): prefixes each line that is not entirely
//! whitespace, keeping line endings. A blank line is NOT prefixed; a trailing newline is preserved
//! (checked via `length`, since assert_all_tiers trims). Byte-identical tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Prefix content lines, skip blank lines, preserve line endings. py: textwrap.indent
#[test]
fn parity_str_indent() {
    // indentText("a\n\nb", "> ") = "> a\n\n> b"  (blank middle line unprefixed)
    // indentText("hi\n", "**") = "**hi\n"        (length 5 → trailing \n kept)
    let src = "## Main\n\
        Show indentText(\"a\\n\\nb\", \"> \").\n\
        Let b be indentText(\"hi\\n\", \"**\").\n\
        Show length of b.\n";
    let expected = "> a\n\n> b\n5";
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, expected);
}
