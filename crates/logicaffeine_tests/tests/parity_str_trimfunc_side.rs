//! Stdlib-parity — one-sided predicate trims: `trimLeftFuncText` (go strings.TrimLeftFunc) and
//! `trimRightFuncText` (go strings.TrimRightFunc). Each is `trimFuncText` with a single trim loop.
//! Beyond tw/VM these PROBE the native tier (assert_compiled_equals_interpreted) — the existing
//! `text.trim_func` claims aot but is only assert_all_tiers-tested, so whether a predicate CLOSURE
//! lowers to AOT-Rust was unverified; this settles it for the one-sided variants.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Trim leading bytes matching the predicate. go: strings.TrimLeftFunc ('x'=120)
#[test]
fn parity_str_trim_left_func() {
    let src = "## Main\nShow trimLeftFuncText((c: Int) -> c is equal to 120, \"xxhixx\").\n";
    assert_all_tiers(src, "hixx");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "hixx");
}

/// Trim trailing bytes matching the predicate. go: strings.TrimRightFunc ('x'=120)
#[test]
fn parity_str_trim_right_func() {
    let src = "## Main\nShow trimRightFuncText((c: Int) -> c is equal to 120, \"xxhixx\").\n";
    assert_all_tiers(src, "xxhi");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "xxhi");
}
