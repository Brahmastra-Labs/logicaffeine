//! Shared helpers for `largo` end-to-end tests.
//!
//! Every e2e test spawns the real binary (`CARGO_BIN_EXE_largo-under-test`) against a
//! scaffolded project in a temp directory, mirroring exactly what a user runs.

pub mod http;

use std::path::Path;
use std::process::{Command, Output};

// ============================================================
// G14 — honest external-tool preflight (mirror of the tests-crate seam).
// ============================================================

/// Is `tool` present on PATH? Probes `<tool> --version`, then a POSIX
/// `command -v <tool>`. Pure — deterministic and parallel-safe.
pub fn tool_available(tool: &str) -> bool {
    let via_version = Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    via_version
        || Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {tool}"))
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// Does the `LOGOS_REQUIRE_TOOLS` `spec` demand `tool`? `"1"`/`"all"` requires
/// every tool; otherwise a comma/space list requires exactly its named tools.
pub fn tool_required_by(spec: &str, tool: &str) -> bool {
    let s = spec.trim();
    s == "1"
        || s.eq_ignore_ascii_case("all")
        || s.split([',', ' ', '\t'])
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .any(|t| t == tool)
}

/// The exact machine-readable marker a tool-skip emits — the greppable line a CI
/// budget counts. Pure (no env, no IO) so the marker FORMAT is unit-lockable.
pub fn tool_skip_marker(tool: &str) -> String {
    format!("LOGOS_TOOL_SKIP: {tool}")
}

/// Preflight `tool` against an EXPLICIT requirement spec (the parallel-safe core of
/// [`require_tool`]). Present → true. Absent and REQUIRED by the spec → PANIC (the
/// release gate fails loud). Absent and not required → emit [`tool_skip_marker`] and
/// return false. Because the spec is passed in (not read from a process-global env),
/// this is deterministic and safe to unit-test under parallel execution.
pub fn require_tool_strict(tool: &str, require_spec: &str) -> bool {
    if tool_available(tool) {
        return true;
    }
    if tool_required_by(require_spec, tool) {
        panic!(
            "required external tool '{tool}' is not available, but LOGOS_REQUIRE_TOOLS \
             demands it — the selected gate fails loud (never a silent SKIP-as-PASS)"
        );
    }
    eprintln!("{}", tool_skip_marker(tool));
    false
}

/// Honest preflight for `tool`, reading the requirement from `LOGOS_REQUIRE_TOOLS`
/// (unset = nothing required, the dev-box default). Replaces the dishonest silent
/// `if let Ok(child) = Command::new(tool)…` skip, so a missing validator is COUNTED,
/// never a false green.
pub fn require_tool(tool: &str) -> bool {
    let spec = std::env::var("LOGOS_REQUIRE_TOOLS").unwrap_or_default();
    require_tool_strict(tool, &spec)
}

/// The exact machine-readable marker a CAPABILITY-skip emits. Pure so the marker
/// FORMAT is unit-lockable (a harness greps `LOGOS_CAP_SKIP:` and parses the fields).
pub fn cap_skip_marker(cap: &str, reason: &str, test: &str) -> String {
    format!("LOGOS_CAP_SKIP: cap={cap} reason={reason:?} test={test}")
}

/// Record a capability-skip against an EXPLICIT spec (the parallel-safe core of
/// [`skip_missing_capability`]). Required by the spec → PANIC; otherwise emit
/// [`cap_skip_marker`] and return false.
pub fn skip_missing_capability_strict(
    cap: &str,
    reason: &str,
    test: &str,
    require_spec: &str,
) -> bool {
    if tool_required_by(require_spec, cap) {
        panic!(
            "required capability '{cap}' is absent ({reason}) but LOGOS_REQUIRE_CAPS \
             demands it — the selected gate fails loud (never a silent SKIP-as-PASS): test={test}"
        );
    }
    eprintln!("{}", cap_skip_marker(cap, reason, test));
    false
}

/// Honestly record that `test` is skipping because optional CAPABILITY `cap` is
/// absent — a capability `LOGOS_REQUIRE_TOOLS` CANNOT force because it is not a
/// PATH binary (a rustup `wasm32` target, a wasm32 sysroot, an lld shipped with
/// the toolchain, a physical board). Emits the machine-readable, greppable
/// [`cap_skip_marker`] line so a harness counts it against a budget. Under
/// `LOGOS_REQUIRE_CAPS` (the release gate that HAS these capabilities — `1`/`all`,
/// or a comma/space list) a demanded-but-absent capability PANICS instead, so the
/// false-green cannot survive that gate. Returns `false` (the caller returns) —
/// never a silent SKIP-as-PASS.
pub fn skip_missing_capability(cap: &str, reason: &str, test: &str) -> bool {
    let spec = std::env::var("LOGOS_REQUIRE_CAPS").unwrap_or_default();
    skip_missing_capability_strict(cap, reason, test, &spec)
}

#[path = "../support/under_test.rs"]
mod under_test;

/// A `Command` for the freshly-built `largo` binary, found where the run put it (`under_test`).
pub fn largo() -> Command {
    Command::new(under_test::largo())
}

/// Run `largo` with `args` in `dir`, capturing output.
pub fn largo_in(dir: &Path, args: &[&str]) -> Output {
    largo()
        .args(args)
        .current_dir(dir)
        .output()
        .expect("largo should spawn")
}

/// Scaffold a minimal LOGOS project (Largo.toml + src/main.lg) in `dir`.
pub fn scaffold(dir: &Path, name: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Largo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n"),
    )
    .unwrap();
    std::fs::write(
        dir.join("src/main.lg"),
        "# Main\n\n## Main\n\nShow \"Hello, world!\".\n",
    )
    .unwrap();
}

/// Strip ANSI escape sequences from a string.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip a CSI sequence: ESC [ ... final-byte (0x40..=0x7e)
            if chars.peek() == Some(&'[') {
                chars.next();
                for f in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&f) {
                        break;
                    }
                }
                continue;
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Whether a string contains any ANSI escape sequence.
pub fn has_ansi(s: &str) -> bool {
    s.contains('\x1b')
}

/// UTF-8 stdout of an `Output`.
pub fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// UTF-8 stderr of an `Output`.
pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}
