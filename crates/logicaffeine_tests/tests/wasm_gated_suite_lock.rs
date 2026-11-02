//! ★ PIT-OF-SUCCESS LOCK — no `wasm-jit`-gated test binary can ever go dark again. ★
//!
//! The browser-facing half of the frontend suite (`wasm_dom`'s wasm tier, the G17
//! reconciliation lock, the full-stack crown lock, JSON-on-WASM, …) is compiled only
//! under `--features wasm-jit`, which the runners execute as an ISOLATED pass with a
//! nextest filterset. For months that filterset hand-listed three binaries, so eight
//! gated binaries (56+ tests) compiled nowhere and ran never — the input-focus bug
//! shipped with a passing suite. A hand-maintained list drifts; this lock makes the
//! drift impossible:
//!
//!  1. The filterset lives in ONE file, `scripts/test/wasm-jit-filter.txt`, and every runner
//!     (`scripts/run-all-tests-fast.sh`, `.github/workflows/test.yml`,
//!     `scripts/test/run-coverage.sh`) reads it — an inline `binary(...)` in any runner's
//!     wasm-jit pass is a failure.
//!  2. The file must name EXACTLY the set of natively-gated test binaries in this
//!     crate, derived from the sources themselves (a new gated file that is not listed
//!     fails; a listed name whose file is gone or no longer gated fails), plus
//!     `kind(lib)`: every lib unit test the feature builds runs, whatever it is named.
//!     A lib test module gated on the feature (`vm::wasm::link`, `vm::wasm::reloc`,
//!     `vm::wasm::stackify`, …) runs in this pass or in none, and nothing ties its test
//!     names to the feature's, so the clause selects by kind, never by name.
//!  3. The failure message prints the exact file content that fixes it.
//!  4. Every runner builds ONLY the pass's own binaries (`--lib $(scripts/test/wasm-jit-targets.sh)`,
//!     derived from the same file), so an unrelated compile-crate test file mid-edit in
//!     another session cannot take the frontend pass down with it.
//!
//! This test itself is NOT feature-gated, so it runs in the default pass — the pass
//! that a forgotten binary would otherwise slip past.

use std::fs;
use std::path::{Path, PathBuf};

const FILTER_FILE: &str = "scripts/test/wasm-jit-filter.txt";
const TARGETS_SCRIPT: &str = "scripts/test/wasm-jit-targets.sh";
const RUNNERS: &[&str] = &[
    "scripts/run-all-tests-fast.sh",
    ".github/workflows/test.yml",
    "scripts/test/run-coverage.sh",
];
const FEATURE_FLAG: &str = "--features wasm-jit";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Every integration-test binary in this crate whose source is gated on the `wasm-jit`
/// feature for the NATIVE target. A file that is `target_arch = "wasm32"`-only (the
/// browser-engine suite) is inert under host nextest and is excluded; a
/// `not(target_arch = "wasm32")` guard is the native-only spelling and counts.
fn gated_binaries() -> Vec<String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).expect("tests dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = read(&path);
        let gated = src.contains("feature = \"wasm-jit\"") || src.contains("feature=\"wasm-jit\"");
        if !gated {
            continue;
        }
        let without_native_guard = src.replace("not(target_arch = \"wasm32\")", "");
        if without_native_guard.contains("target_arch = \"wasm32\"") {
            continue;
        }
        out.push(path.file_stem().unwrap().to_str().unwrap().to_string());
    }
    out.sort();
    out
}

fn binaries_in(filter: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = filter;
    while let Some(i) = rest.find("binary(") {
        let after = &rest[i + "binary(".len()..];
        let end = after.find(')').expect("binary( … ) closes");
        out.push(after[..end].trim().to_string());
        rest = &after[end..];
    }
    out.sort();
    out
}

fn expected_filter(gated: &[String]) -> String {
    let mut parts: Vec<String> = gated.iter().map(|b| format!("binary({b})")).collect();
    parts.push("kind(lib)".to_string());
    parts.join(" + ")
}

/// Every shell command in `runner` that mentions the wasm-jit feature flag — the real
/// `cargo nextest run` pass AND any echoed hint that repeats it — each as the line
/// holding the flag plus its continuation lines (a trailing `\`). Checking every
/// occurrence, not just the first, is deliberate: a hand-listed filter in a help echo
/// is the same drift in a different costume.
fn wasm_jit_commands(runner: &str) -> Vec<String> {
    let lines: Vec<&str> = runner.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].contains(FEATURE_FLAG) {
            i += 1;
            continue;
        }
        let mut cmd = String::new();
        loop {
            cmd.push_str(lines[i]);
            cmd.push('\n');
            let continues = lines[i].trim_end().ends_with('\\');
            i += 1;
            if !continues || i >= lines.len() {
                break;
            }
        }
        out.push(cmd);
    }
    assert!(!out.is_empty(), "runner has no `{FEATURE_FLAG}` pass");
    out
}

#[test]
fn the_detector_sees_the_crown_frontend_binaries() {
    // Guards the detector itself: if the cfg spelling ever changes, this fails loudly
    // instead of the gated set silently shrinking to empty (which would make the
    // completeness test below vacuously happy).
    let gated = gated_binaries();
    for must in [
        "wasm_dom",
        "dom_reconciliation_lock",
        "fullstack_todo_lock",
        "json_on_wasm_lock",
    ] {
        assert!(
            gated.iter().any(|b| b == must),
            "detector missed `{must}`; saw {gated:?}"
        );
    }
}

#[test]
fn the_filter_file_names_exactly_the_gated_binaries() {
    let gated = gated_binaries();
    let expected = expected_filter(&gated);
    let path = repo_root().join(FILTER_FILE);
    let filter = fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("{FILTER_FILE} is missing. Create it with exactly this content:\n{expected}\n")
    });
    let listed = binaries_in(&filter);
    let missing: Vec<&String> = gated.iter().filter(|b| !listed.contains(b)).collect();
    let stale: Vec<&String> = listed.iter().filter(|b| !gated.contains(b)).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "{FILTER_FILE} drifted from the gated sources.\n  gated but not listed (DARK): {missing:?}\n  listed but not gated (stale): {stale:?}\nReplace the file content with exactly:\n{expected}\n"
    );
    assert!(
        filter.contains("kind(lib)"),
        "{FILTER_FILE} must keep `kind(lib)` so the gated lib unit tests run too:\n{expected}\n"
    );
    assert_eq!(
        filter.trim(),
        expected,
        "{FILTER_FILE} must be exactly the canonical expression (one line, alphabetical, `kind(lib)` last)"
    );
}

/// A lib test gated on the feature exists only in this pass, so the pass must take every lib
/// test, not the ones a name happens to match: a `test(…)` clause selects by name, and a gated
/// module whose test names do not carry the pattern would compile in this pass and run in none.
#[test]
fn the_filter_selects_lib_tests_by_kind_not_by_name() {
    let filter = read(&repo_root().join(FILTER_FILE));
    assert!(
        !filter.contains("test("),
        "{FILTER_FILE} selects tests by name — a feature-gated lib test whose name misses the pattern runs nowhere:\n{filter}"
    );
}

#[test]
fn the_targets_script_derives_its_binaries_from_the_filter_file() {
    let out = std::process::Command::new("bash")
        .arg(repo_root().join(TARGETS_SCRIPT))
        .output()
        .expect("targets script runs");
    assert!(
        out.status.success(),
        "{TARGETS_SCRIPT} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let mut listed: Vec<String> = text
        .split_whitespace()
        .filter(|w| *w != "--test")
        .map(str::to_string)
        .collect();
    listed.sort();
    assert_eq!(
        listed,
        gated_binaries(),
        "{TARGETS_SCRIPT} must name exactly the gated binaries"
    );
    assert_eq!(
        text.matches("--test").count(),
        listed.len(),
        "one `--test` per binary"
    );
}

/// The spellings of a nextest pass a runner uses: plain, and under coverage instrumentation.
const NEXTEST_PASSES: &[&str] = &["cargo nextest run", "cargo llvm-cov --no-report nextest"];

#[test]
fn every_runner_reads_the_single_filter_file() {
    for runner_path in RUNNERS {
        let runner = read(&repo_root().join(runner_path));
        let cmds = wasm_jit_commands(&runner);
        for cmd in &cmds {
            assert!(
                !cmd.contains("binary("),
                "{runner_path}: a wasm-jit pass hand-lists binaries inline — that is the list that went dark. Read `{FILTER_FILE}` instead:\n{cmd}"
            );
        }
        let real: Vec<&String> = cmds
            .iter()
            .filter(|c| NEXTEST_PASSES.iter().any(|p| c.contains(p)))
            .collect();
        assert!(
            !real.is_empty(),
            "{runner_path}: no nextest pass with `{FEATURE_FLAG}` found"
        );
        for cmd in real {
            assert!(
                cmd.contains(FILTER_FILE),
                "{runner_path}: the wasm-jit pass must read `{FILTER_FILE}` (e.g. `-E \"$(cat {FILTER_FILE})\"`), got:\n{cmd}"
            );
            assert!(
                cmd.contains(TARGETS_SCRIPT) && cmd.contains("--lib"),
                "{runner_path}: the wasm-jit pass must build ONLY its own binaries — `--lib $({TARGETS_SCRIPT})` — so an unrelated test file mid-edit cannot take the frontend pass down, got:\n{cmd}"
            );
        }
    }
}
