//! End-to-end locks for MULTI-FILE / MULTI-DIRECTORY LOGOS projects, driven
//! through the real `largo` binary exactly as a user runs it — a scaffolded
//! `Largo.toml` + `src/` tree, then `largo run`/`run --interpret`/`check`/
//! `run --emit wasm`. The library-level suites (`multimodule_functions`,
//! `multimodule_audit`) prove the assembler and codegen; this proves the CLI
//! wires them into every tier and surfaces the errors.

#[path = "cli_common/mod.rs"]
mod common;

use std::path::Path;

use common::*;
use tempfile::tempdir;

/// Write a `Largo.toml` (entry `src/main.lg`) plus each `(relative-to-src, body)`
/// module — creating subdirectories as needed for multi-directory layouts.
fn project(dir: &Path, name: &str, files: &[(&str, &str)]) {
    std::fs::write(
        dir.join("Largo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n"),
    )
    .unwrap();
    for (rel, body) in files {
        let path = dir.join("src").join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
}

/// A build/run failed for lack of a Rust LINKER/sysroot, not a real wiring bug — a capability
/// `LOGOS_REQUIRE_TOOLS` cannot force (it is not a PATH binary), so it is recorded through
/// `skip_missing_capability` (counted + forceable via `LOGOS_REQUIRE_CAPS`). `rustc`/`cargo`
/// themselves are PATH tools, preflighted via `require_tool` at each AOT call site so their absence is
/// loud under `LOGOS_REQUIRE_TOOLS`. A real LOGOS-source error is never mistaken for toolchain absence.
fn linker_or_sysroot_absence(stderr: &str) -> bool {
    ["linker", "sysroot", "could not compile `"]
        .iter()
        .any(|m| stderr.contains(m))
        && !stderr.contains("in your LOGOS source")
}

// ============================================================================
// Interpreter tier (fast, no toolchain) — the assembler wired into `largo run
// --interpret`.
// ============================================================================

#[test]
fn e2e_interpret_cross_file_function() {
    let dir = tempdir().unwrap();
    project(
        dir.path(),
        "app",
        &[
            (
                "main.lg",
                "# App\n\nImport Util.\n\n## Main\nShow double(21).\n",
            ),
            (
                "util.lg",
                "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
            ),
        ],
    );
    let out = largo_in(dir.path(), &["run", "--interpret"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "42");
}

#[test]
fn e2e_interpret_qualified_call() {
    let dir = tempdir().unwrap();
    project(
        dir.path(),
        "app",
        &[
            (
                "main.lg",
                "# App\n\nImport Util.\n\n## Main\nShow Util::double(21).\n",
            ),
            (
                "util.lg",
                "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
            ),
        ],
    );
    let out = largo_in(dir.path(), &["run", "--interpret"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "42");
}

#[test]
fn e2e_interpret_bare_imported_type() {
    let dir = tempdir().unwrap();
    project(dir.path(), "app", &[
        ("main.lg", "# App\n\nImport Geometry.\n\n## Main\nLet p be a new Point with x 3 and y 4.\nShow p's x plus p's y.\n"),
        ("geometry.md", "# Geometry\n\n## A Point has:\n    An x: Int.\n    A y: Int.\n"),
    ]);
    let out = largo_in(dir.path(), &["run", "--interpret"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "7");
}

#[test]
fn e2e_interpret_multidirectory_layout() {
    let dir = tempdir().unwrap();
    // Entry in src/ imports two modules living in DIFFERENT subdirectories.
    project(dir.path(), "app", &[
        (
            "main.lg",
            "# App\n\nImport Tokens from \"lexer/tokens.lg\".\nImport Numbers from \"math/numbers.lg\".\n\n## Main\nShow tokenCount(3) plus square(4).\n",
        ),
        ("lexer/tokens.lg", "# Tokens\n\n## To tokenCount (n: Int) -> Int:\n    Return n plus 1.\n"),
        ("math/numbers.lg", "# Numbers\n\n## To square (n: Int) -> Int:\n    Return n times n.\n"),
    ]);
    let out = largo_in(dir.path(), &["run", "--interpret"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    // tokenCount(3)=4, square(4)=16 → 20.
    assert_eq!(stdout(&out).trim(), "20");
}

#[test]
fn e2e_interpret_import_cycle_mutual_recursion() {
    let dir = tempdir().unwrap();
    project(dir.path(), "app", &[
        ("main.lg", "# App\n\nImport Even.\n\n## Main\nShow pingpong(4).\n"),
        ("even.lg", "# Even\n\nImport Odd.\n\n## To pingpong (n: Int) -> Int:\n    If n is less than 1:\n        Return 0.\n    Return pong(n minus 1).\n"),
        ("odd.lg", "# Odd\n\nImport Even.\n\n## To pong (n: Int) -> Int:\n    If n is less than 1:\n        Return 1.\n    Return pingpong(n minus 1).\n"),
    ]);
    let out = largo_in(dir.path(), &["run", "--interpret"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "0");
}

#[test]
fn e2e_interpret_rejects_transitive_scope_leak() {
    let dir = tempdir().unwrap();
    // The interpreter tier enforces per-file scoping identically to `check`/build:
    // reaching an un-imported module is rejected before any execution.
    project(
        dir.path(),
        "app",
        &[
            (
                "main.lg",
                "# App\n\nImport Mid.\n\n## Main\nShow leafVal(5).\n",
            ),
            (
                "mid.lg",
                "# Mid\n\nImport Leaf.\n\n## To midVal (n: Int) -> Int:\n    Return leafVal(n).\n",
            ),
            (
                "leaf.lg",
                "# Leaf\n\n## To leafVal (n: Int) -> Int:\n    Return n plus 100.\n",
            ),
        ],
    );
    for (label, args) in [
        ("auto", &["run", "--interpret"][..]),
        ("vm", &["run", "--interpret", "--engine", "vm"][..]),
        ("jit", &["run", "--interpret", "--engine", "jit"][..]),
    ] {
        let out = largo_in(dir.path(), args);
        assert_ne!(
            out.status.code(),
            Some(0),
            "a scope leak must fail the {label} run tier"
        );
        let msg = format!("{}{}", stdout(&out), stderr(&out));
        assert!(
            msg.contains("leafVal"),
            "the {label} error must name the leaked function, got:\n{msg}"
        );
        // And it must NOT have executed the body (no leaked output).
        assert!(
            !stdout(&out).contains("105"),
            "the leaked call must not run on {label}, got:\n{msg}"
        );
    }
}

// ============================================================================
// `largo check` (fast) — scope leaks, collisions, and cross-file parse spans.
// ============================================================================

#[test]
fn e2e_check_clean_multifile_exits_zero() {
    let dir = tempdir().unwrap();
    project(
        dir.path(),
        "app",
        &[
            (
                "main.lg",
                "# App\n\nImport Util.\n\n## Main\nShow double(21).\n",
            ),
            (
                "util.lg",
                "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
            ),
        ],
    );
    let out = largo_in(dir.path(), &["check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "clean project must check clean.\nstderr: {}",
        stderr(&out)
    );
}

#[test]
fn e2e_check_rejects_transitive_scope_leak() {
    let dir = tempdir().unwrap();
    // main imports mid (which imports leaf); main reaches leaf's function
    // WITHOUT importing leaf — a per-file scope violation.
    project(
        dir.path(),
        "app",
        &[
            (
                "main.lg",
                "# App\n\nImport Mid.\n\n## Main\nShow leafVal(5).\n",
            ),
            (
                "mid.lg",
                "# Mid\n\nImport Leaf.\n\n## To midVal (n: Int) -> Int:\n    Return leafVal(n).\n",
            ),
            (
                "leaf.lg",
                "# Leaf\n\n## To leafVal (n: Int) -> Int:\n    Return n plus 100.\n",
            ),
        ],
    );
    let out = largo_in(dir.path(), &["check"]);
    assert_ne!(
        out.status.code(),
        Some(0),
        "a scope leak must fail `largo check`"
    );
    let msg = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        msg.contains("leafVal"),
        "the error must name the leaked function, got:\n{msg}"
    );
}

#[test]
fn e2e_check_reports_bare_collision_naming_both_modules() {
    let dir = tempdir().unwrap();
    // Two imported modules both export a bare `parse` with DIFFERENT bodies; a
    // bare `parse` call is ambiguous → hard error.
    project(
        dir.path(),
        "app",
        &[
            (
                "main.lg",
                "# App\n\nImport A.\nImport B.\n\n## Main\nShow parse(10).\n",
            ),
            (
                "a.lg",
                "# A\n\n## To parse (n: Int) -> Int:\n    Return n plus 1.\n",
            ),
            (
                "b.lg",
                "# B\n\n## To parse (n: Int) -> Int:\n    Return n plus 2.\n",
            ),
        ],
    );
    let out = largo_in(dir.path(), &["check"]);
    assert_ne!(
        out.status.code(),
        Some(0),
        "an ambiguous bare name must fail `largo check`"
    );
    let msg = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        msg.contains("parse"),
        "the collision error must name `parse`, got:\n{msg}"
    );
}

#[test]
fn e2e_check_parse_error_is_attributed_to_the_imported_file() {
    let dir = tempdir().unwrap();
    // The syntax error lives in `util.lg` (unclosed param paren), not the entry.
    project(
        dir.path(),
        "app",
        &[
            (
                "main.lg",
                "# App\n\nImport Util.\n\n## Main\nShow helper(1).\n",
            ),
            (
                "util.lg",
                "# Util\n\n## To helper (n: Int:\n    Return n.\n",
            ),
        ],
    );
    let out = largo_in(dir.path(), &["check"]);
    assert_ne!(
        out.status.code(),
        Some(0),
        "a parse error must fail `largo check`"
    );
    let msg = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        msg.contains("util.lg"),
        "the parse error must be attributed to `util.lg`, got:\n{msg}"
    );
}

// ============================================================================
// Import hygiene through `largo check` — the ok:true-with-warnings contract.
// ============================================================================

#[test]
fn e2e_check_warns_on_unused_import_but_exits_zero() {
    let dir = tempdir().unwrap();
    project(
        dir.path(),
        "app",
        &[
            ("main.lg", "# App\n\nImport Util.\n\n## Main\nShow 42.\n"),
            (
                "util.lg",
                "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
            ),
        ],
    );
    let out = largo_in(dir.path(), &["check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "an unused import is a WARNING, not an error"
    );
    assert!(
        stderr(&out).contains("unused"),
        "stderr must warn about the unused import:\n{}",
        stderr(&out)
    );
}

#[test]
fn e2e_check_json_warning_keeps_ok_true() {
    let dir = tempdir().unwrap();
    project(
        dir.path(),
        "app",
        &[
            ("main.lg", "# App\n\nImport Util.\n\n## Main\nShow 42.\n"),
            (
                "util.lg",
                "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
            ),
        ],
    );
    let out = largo_in(dir.path(), &["check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "warnings keep exit 0");
    let v: serde_json::Value =
        serde_json::from_str(stdout(&out).trim()).expect("one JSON document");
    assert_eq!(v["ok"], true, "warnings must keep ok:true: {v}");
    let diags = v["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diags
            .iter()
            .any(|d| d["code"] == "unused-import" && d["severity"] == "warning"),
        "the unused-import warning must be in the envelope: {v}"
    );
}

#[test]
fn e2e_check_errors_on_self_import_exits_one() {
    let dir = tempdir().unwrap();
    project(
        dir.path(),
        "app",
        &[("main.lg", "# App\n\nImport Main.\n\n## Main\nShow 42.\n")],
    );
    let out = largo_in(dir.path(), &["check"]);
    assert_ne!(out.status.code(), Some(0), "a self-import must FAIL check");
    let msg = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        msg.to_lowercase().contains("itself"),
        "the error must explain the self-import:\n{msg}"
    );
}

// ============================================================================
// AOT tier (`largo run` → cargo build + execute). Heavier; one core lock.
// ============================================================================

#[test]
fn e2e_aot_run_cross_file_function() {
    // The AOT tier compiles LOGOS→Rust→binary; preflight the toolchain so its absence is a COUNTED
    // marker (and loud under LOGOS_REQUIRE_TOOLS), never a false green.
    if !require_tool("cargo") || !require_tool("rustc") {
        return;
    }
    let dir = tempdir().unwrap();
    project(
        dir.path(),
        "app",
        &[
            (
                "main.lg",
                "# App\n\nImport Util.\n\n## Main\nShow double(21).\n",
            ),
            (
                "util.lg",
                "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
            ),
        ],
    );
    let out = largo_in(dir.path(), &["run"]);
    if !out.status.success() {
        let err = stderr(&out);
        if linker_or_sysroot_absence(&err)
            && !skip_missing_capability(
                "rust-linker",
                "rust linker / sysroot unavailable",
                "e2e_aot_run_cross_file_function",
            )
        {
            return;
        }
        panic!(
            "`largo run` (AOT) failed:\nstdout: {}\nstderr: {err}",
            stdout(&out)
        );
    }
    assert_eq!(stdout(&out).trim(), "42");
}

#[test]
fn e2e_aot_run_multidirectory_layout() {
    if !require_tool("cargo") || !require_tool("rustc") {
        return;
    }
    let dir = tempdir().unwrap();
    project(dir.path(), "app", &[
        (
            "main.lg",
            "# App\n\nImport Tokens from \"lexer/tokens.lg\".\nImport Numbers from \"math/numbers.lg\".\n\n## Main\nShow tokenCount(3) plus square(4).\n",
        ),
        ("lexer/tokens.lg", "# Tokens\n\n## To tokenCount (n: Int) -> Int:\n    Return n plus 1.\n"),
        ("math/numbers.lg", "# Numbers\n\n## To square (n: Int) -> Int:\n    Return n times n.\n"),
    ]);
    let out = largo_in(dir.path(), &["run"]);
    if !out.status.success() {
        let err = stderr(&out);
        if linker_or_sysroot_absence(&err)
            && !skip_missing_capability(
                "rust-linker",
                "rust linker / sysroot unavailable",
                "e2e_aot_run_multidirectory_layout",
            )
        {
            return;
        }
        panic!(
            "`largo run` (AOT, multi-dir) failed:\nstdout: {}\nstderr: {err}",
            stdout(&out)
        );
    }
    assert_eq!(stdout(&out).trim(), "20");
}

// ============================================================================
// WASM tier (`largo run --emit wasm`, node-gated) — same project, same answer.
// ============================================================================

#[test]
fn e2e_wasm_run_cross_file_function() {
    if !require_tool("node") {
        return;
    }
    let dir = tempdir().unwrap();
    project(
        dir.path(),
        "app",
        &[
            (
                "main.lg",
                "# App\n\nImport Util.\n\n## Main\nShow double(21).\n",
            ),
            (
                "util.lg",
                "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
            ),
        ],
    );
    let out = largo_in(dir.path(), &["run", "--emit", "wasm"]);
    if !out.status.success() {
        let err = stderr(&out);
        if linker_or_sysroot_absence(&err)
            && !skip_missing_capability(
                "rust-linker",
                "rust linker / sysroot unavailable",
                "e2e_wasm_run_cross_file_function",
            )
        {
            return;
        }
        panic!(
            "`largo run --emit wasm` failed:\nstdout: {}\nstderr: {err}",
            stdout(&out)
        );
    }
    assert_eq!(stdout(&out).trim(), "42");
}
