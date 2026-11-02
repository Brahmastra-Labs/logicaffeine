//! G14 — honest external-tool preflights, `largo` (CLI) side. The CLI e2e suite
//! spawns real toolchains (node for the WASM host shim, rustc/cargo for the AOT
//! tier). A test that needs one of those must NOT print `SKIP` and `return` — that
//! reports PASS to cargo, so a default green run establishes nothing. The shared
//! `common::require_tool` seam replaces that: a missing tool is a MACHINE-READABLE,
//! COUNTED marker (`LOGOS_TOOL_SKIP: <tool>`) and, under `LOGOS_REQUIRE_TOOLS` (the
//! release gate), a HARD FAILURE — never a silent SKIP-as-PASS.
//!
//! A second seam, `common::skip_missing_capability`, is for the capabilities
//! `LOGOS_REQUIRE_TOOLS` CANNOT force because they are not PATH binaries — a rustup
//! `wasm32` target, a wasm32 sysroot, an lld/linker shipped with the toolchain. Its
//! absence is a counted `LOGOS_CAP_SKIP: …` marker, forceable via `LOGOS_REQUIRE_CAPS`.
//!
//! The `_strict` cores take the requirement SPEC as an argument (not a
//! process-global env var), so this contract is deterministic and parallel-safe.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK.  ⚠️  A silent SKIP-as-PASS is the
//!  false-green this gate exists to kill.
#![allow(unused_imports, dead_code)]
#[path = "cli_common/mod.rs"]
mod common;
use common::{
    cap_skip_marker, require_tool, require_tool_strict, skip_missing_capability,
    skip_missing_capability_strict, tool_available, tool_required_by, tool_skip_marker,
};

/// The pure availability probe: true for a tool on PATH, false for a bogus name.
#[test]
fn tool_available_detects_presence() {
    assert!(tool_available("sh"), "sh must be detectable on PATH");
    assert!(
        !tool_available("largo-definitely-not-a-real-tool-xyzzy"),
        "a nonexistent tool must not be reported available"
    );
}

/// The requirement spec: `"1"`/`"all"` requires every tool; otherwise a
/// comma/space list requires exactly the named tools.
#[test]
fn tool_required_by_parses_the_spec() {
    assert!(tool_required_by("1", "node"));
    assert!(tool_required_by("all", "anything"));
    assert!(tool_required_by("node,cargo", "cargo"));
    assert!(tool_required_by("node rustc", "node"));
    assert!(!tool_required_by("node,cargo", "rustc"));
    assert!(!tool_required_by("", "node"));
}

/// (b) A skip emits the STRUCTURED, COUNTED line — the format a CI budget greps.
/// The pure formatter pins the exact bytes so the marker can never drift.
#[test]
fn skip_emits_the_structured_counted_marker() {
    assert_eq!(tool_skip_marker("node"), "LOGOS_TOOL_SKIP: node");
    assert_eq!(
        cap_skip_marker("wasm32-base", "wasm32 sysroot unavailable", "some_test"),
        "LOGOS_CAP_SKIP: cap=wasm32-base reason=\"wasm32 sysroot unavailable\" test=some_test",
    );
}

/// (c) The DEFAULT (non-strict) mode: a missing, NOT-required tool is a machine-
/// readable skip (returns false), never a panic — the dev box stays runnable and
/// the run is green with the skip counted.
#[test]
fn missing_unrequired_tool_skips_not_panics() {
    assert!(
        !require_tool_strict("largo-definitely-not-a-real-tool-xyzzy", ""),
        "a missing unrequired tool must return false (skip), not true"
    );
    assert!(
        !skip_missing_capability_strict("no-such-cap", "absent", "t", ""),
        "a missing unrequired capability must return false (skip), not true"
    );
}

/// (a) STRICT MODE: a missing REQUIRED tool FAILS LOUD. We simulate the release
/// gate by pointing the tool lookup at a NON-EXISTENT binary while the spec demands
/// every tool (`"1"`) — the panic is the whole point (no silent pass).
#[test]
#[should_panic(expected = "largo-definitely-not-a-real-tool-xyzzy")]
fn missing_required_tool_panics_under_the_gate() {
    require_tool_strict("largo-definitely-not-a-real-tool-xyzzy", "1");
}

/// A missing tool named EXPLICITLY in the spec panics too (the subset form of the
/// release gate: `LOGOS_REQUIRE_TOOLS="node cargo"`).
#[test]
#[should_panic(expected = "largo-definitely-not-a-real-tool-xyzzy")]
fn missing_listed_required_tool_panics() {
    require_tool_strict(
        "largo-definitely-not-a-real-tool-xyzzy",
        "node,largo-definitely-not-a-real-tool-xyzzy",
    );
}

/// A missing REQUIRED capability FAILS LOUD under an explicit caps spec — the
/// capability analogue of the tool gate, for the release box that HAS the wasm32
/// base and demands the linked tier actually run.
#[test]
#[should_panic(expected = "wasm32-base")]
fn missing_required_capability_panics_under_the_caps_gate() {
    skip_missing_capability_strict(
        "wasm32-base",
        "wasm32 sysroot unavailable",
        "t",
        "wasm32-base",
    );
}

/// A PRESENT tool returns true regardless of the requirement spec, and the
/// env-reading wrapper agrees for a present tool.
#[test]
fn present_tool_is_true() {
    assert!(require_tool_strict("sh", ""));
    assert!(require_tool_strict("sh", "1"));
    assert!(
        require_tool("sh"),
        "the env-reading wrapper agrees for a present tool"
    );
}
