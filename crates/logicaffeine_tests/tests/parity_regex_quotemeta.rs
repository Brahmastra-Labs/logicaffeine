//! Stdlib-parity — `regexQuoteMeta` (Go regexp.QuoteMeta): escape Go regexp metacharacters
//! (`\.+*?()|[]{}^$`) — a tighter set than re.escape (space and `#` are left alone). tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Escape Go regexp metacharacters for literal matching. go: regexp.QuoteMeta
#[test]
fn parity_regex_quote_meta() {
    // the {2} input is built via text_from_bytes because LOGOS string literals treat {…} as
    // interpolation ([91,97,45,122,93,123,50,125] = "[a-z]{2}").
    let src = "## Main\n\
        Show regexQuoteMeta(\"a.b*c\").\n\
        Show regexQuoteMeta(\"(x)|y\").\n\
        Show regexQuoteMeta(\"a b#c\").\n\
        Show regexQuoteMeta(text_from_bytes([91, 97, 45, 122, 93, 123, 50, 125])).\n";
    assert_all_tiers(src, "a\\.b\\*c\n\\(x\\)\\|y\na b#c\n\\[a-z\\]\\{2\\}");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "a\\.b\\*c\n\\(x\\)\\|y\na b#c\n\\[a-z\\]\\{2\\}");
}
