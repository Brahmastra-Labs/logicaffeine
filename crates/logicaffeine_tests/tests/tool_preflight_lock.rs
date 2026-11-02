//! G14 — honest external-tool preflights. A test that needs an external tool
//! (python3, node, nvcc, iverilog, an AOT toolchain) must NOT print `SKIP` and
//! `return` — that reports PASS to cargo, so a default green run establishes
//! nothing. The shared `require_tool` seam replaces that pattern: a missing tool
//! is a MACHINE-READABLE skip marker (`LOGOS_TOOL_SKIP: <tool>`) counted against
//! a budget, and under `LOGOS_REQUIRE_TOOLS` (the release gate) a missing tool
//! FAILS LOUD instead of silently passing.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK.  ⚠️  A silent SKIP-as-PASS is the
//!  false-green this gate exists to kill.
#![allow(unused_imports, dead_code)]
mod common;
use common::{
    require_tool, require_tool_strict, rust_target_installed, tool_available, tool_required_by,
    tool_skip_marker,
};

/// The pure availability probe: true for a tool on PATH, false for a bogus name.
#[test]
fn tool_available_detects_presence() {
    // `sh` is present on every POSIX box the suite runs on.
    assert!(tool_available("sh"), "sh must be detectable on PATH");
    assert!(
        !tool_available("logos-definitely-not-a-real-tool-xyzzy"),
        "a nonexistent tool must not be reported available"
    );
}

/// The requirement spec: `"1"`/`"all"` requires every tool; otherwise a
/// comma/space list requires exactly the named tools.
#[test]
fn tool_required_by_parses_the_spec() {
    assert!(tool_required_by("1", "nvcc"));
    assert!(tool_required_by("all", "anything"));
    assert!(tool_required_by("nvcc,node", "node"));
    assert!(tool_required_by("nvcc node", "nvcc"));
    assert!(!tool_required_by("nvcc,node", "iverilog"));
    assert!(!tool_required_by("", "node"));
}

/// A missing, NOT-required tool is a machine-readable skip (returns false), never
/// a panic — the dev-box default stays runnable.
#[test]
fn missing_unrequired_tool_skips_not_panics() {
    assert!(
        !require_tool_strict("logos-definitely-not-a-real-tool-xyzzy", ""),
        "a missing unrequired tool must return false (skip), not true"
    );
}

/// The STRUCTURED, COUNTED skip line: a CI budget greps `LOGOS_TOOL_SKIP: <tool>`,
/// so its exact bytes are pinned here (the format can never drift out from under
/// the harness that counts it).
#[test]
fn skip_emits_the_structured_counted_marker() {
    assert_eq!(tool_skip_marker("node"), "LOGOS_TOOL_SKIP: node");
    assert_eq!(tool_skip_marker("nvcc"), "LOGOS_TOOL_SKIP: nvcc");
}

/// A missing REQUIRED tool (the release gate) FAILS LOUD — the whole point.
#[test]
#[should_panic(expected = "logos-definitely-not-a-real-tool-xyzzy")]
fn missing_required_tool_panics_under_the_gate() {
    // `"1"` = require every tool; the missing one must panic, not skip-pass.
    require_tool_strict("logos-definitely-not-a-real-tool-xyzzy", "1");
}

/// A missing tool named EXPLICITLY in the spec panics too.
#[test]
#[should_panic(expected = "logos-definitely-not-a-real-tool-xyzzy")]
fn missing_listed_required_tool_panics() {
    require_tool_strict(
        "logos-definitely-not-a-real-tool-xyzzy",
        "node,logos-definitely-not-a-real-tool-xyzzy",
    );
}

/// A PRESENT tool returns true regardless of the requirement spec.
#[test]
fn present_tool_is_true() {
    assert!(require_tool_strict("sh", ""));
    assert!(require_tool_strict("sh", "1"));
    assert!(
        require_tool("sh"),
        "the env-reading wrapper agrees for a present tool"
    );
}

/// A rustup target counts as installed only when its standard library is there. `rustc --print target-libdir`
/// names a directory for any triple rustc knows, installed or not, so a probe that trusts its exit status says
/// yes to every known target and its tests never skip on a box without the target. The host's own target is
/// installed; a tier-3 triple rustc knows but rustup never ships is not; a name rustc rejects is not.
#[test]
fn a_rust_target_is_installed_only_when_its_std_is() {
    let host = std::process::Command::new("rustc")
        .arg("-vV")
        .output()
        .expect("rustc -vV");
    let host = String::from_utf8_lossy(&host.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("host: ").map(str::to_string))
        .expect("rustc -vV names the host");
    assert!(rust_target_installed(&host), "the host target {host} must count as installed");
    let known = std::process::Command::new("rustc")
        .args(["--print", "target-libdir", "--target", "mips64-openwrt-linux-musl"])
        .output()
        .expect("rustc --print target-libdir");
    assert!(
        known.status.success(),
        "precondition: rustc must know mips64-openwrt-linux-musl, or this row no longer tests the trap"
    );
    assert!(
        !rust_target_installed("mips64-openwrt-linux-musl"),
        "a target rustc knows but whose std is absent must not count as installed"
    );
    assert!(!rust_target_installed("logos-no-such-triple"), "an unknown triple must not count as installed");
}
