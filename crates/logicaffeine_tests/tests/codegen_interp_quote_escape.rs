//! Regression lock: an interpolated string whose LITERAL segments contain a `"` (or `\`) must
//! lower correctly on AOT-Rust. The `codegen_interpolated_string` with-holes path escaped
//! `{}`/`\n`/`\t`/`\r` but forgot `"` and `\`, so `"key \"{k}\" ok"` emitted a broken
//! `format!("key "{}" ok", …)` — the unescaped quote closed the Rust string literal early
//! (`error: expected ','`). Discovered via json.lg's strict duplicate-key error message.
//! The no-holes `String::from` path always escaped them; this test pins the with-holes path to
//! the same contract. tw ≡ VM ≡ AOT-Rust.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted};

/// A quote around an interpolation hole. Before the fix, AOT-Rust failed to COMPILE.
#[test]
fn interpolated_quote_around_hole_lowers_on_aot() {
    // LOGOS: Let k be "name".  Show "key \"{k}\" ok".   → key "name" ok
    let src = "## Main\n    Let k be \"name\".\n    Show \"key \\\"{k}\\\" ok\".\n";
    assert_all_tiers(src, "key \"name\" ok");
    assert_compiled_equals_interpreted(src);
}

/// A backslash in a literal segment beside a hole (the other char the path forgot).
#[test]
fn interpolated_backslash_beside_hole_lowers_on_aot() {
    // LOGOS: Let n be 3.  Show "path\\to {n}".   → path\to 3
    let src = "## Main\n    Let n be 3.\n    Show \"path\\\\to {n}\".\n";
    assert_all_tiers(src, "path\\to 3");
    assert_compiled_equals_interpreted(src);
}
