//! Stdlib-parity — `regexEscape` (Python re.escape): backslash-escape every regex metacharacter so
//! the string matches literally. Escaped set verified against CPython. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Escape regex metacharacters for literal matching. py: re.escape
#[test]
fn parity_regex_escape() {
    let src = "## Main\n\
        Show regexEscape(\"a.b*c\").\n\
        Show regexEscape(\"1+1=2\").\n\
        Show regexEscape(\"no_special_123\").\n";
    assert_all_tiers(src, "a\\.b\\*c\n1\\+1=2\nno_special_123");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "a\\.b\\*c\n1\\+1=2\nno_special_123");
}
