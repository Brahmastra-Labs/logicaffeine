//! E2E Tests: Show Temporal Values
//!
//! Tests that temporal values can be displayed.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{run_logos, assert_output};

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_show_duration_ms() {
    assert_output(
        r#"## Main
Let timeout be 500ms.
Show timeout.
"#,
        "500ms",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_show_duration_seconds() {
    assert_output(
        r#"## Main
Let delay be 2s.
Show delay.
"#,
        "2s",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_show_duration_ns() {
    assert_output(
        r#"## Main
Let precise be 50ns.
Show precise.
"#,
        "50ns",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_show_date() {
    assert_output(
        r#"## Main
Let graduation be 2026-05-20.
Show graduation.
"#,
        "2026-05-20",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_show_date_new_years() {
    assert_output(
        r#"## Main
Let ny be 2026-01-01.
Show ny.
"#,
        "2026-01-01",
    );
}

// === SLEEP WITH DURATION TESTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleep_duration_ms() {
    assert_output(
        r#"## Main
Show "before".
Sleep 50ms.
Show "after".
"#,
        "after",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleep_duration_seconds() {
    // Using a short duration to not slow down tests
    assert_output(
        r#"## Main
Show "start".
Sleep 100ms.
Show "end".
"#,
        "end",
    );
}

// === DURATION MATH TESTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_addition() {
    assert_output(
        r#"## Main
Let a be 500ms.
Let b be 500ms.
Let total be a + b.
Show total.
"#,
        "1s",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_addition_different_units() {
    assert_output(
        r#"## Main
Let a be 1s.
Let b be 500ms.
Let total be a + b.
Show total.
"#,
        "1s",  // 1500ms shows as 1s (truncated display)
    );
}
