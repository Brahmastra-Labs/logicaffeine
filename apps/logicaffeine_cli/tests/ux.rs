//! UX substrate suite: help quality, global flags, color discipline,
//! exit codes, and the reserved `test` verb.
//!
//! These tests define the CLI's polish contract:
//! - bare `largo` gives a full command overview (exit 2)
//! - every visible subcommand documents itself with an `about` AND a
//!   usage-examples section in `--help`
//! - `--quiet` / `--verbose` / `--color` are global flags
//! - ANSI never leaks into pipes unless `--color always`; `NO_COLOR` wins
//! - errors render as `error: <message>` with an actionable `help:` hint
//! - `largo test` is reserved for the future LOGOS test framework

mod common;

use clap::CommandFactory;
use common::*;
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// Bare invocation + help quality
// ---------------------------------------------------------------------------

/// Bare `largo` prints the full command overview (not a terse error) and
/// exits 2 (usage).
#[test]
fn bare_largo_prints_command_overview() {
    let out = largo().output().expect("largo should spawn");
    assert_eq!(out.status.code(), Some(2), "bare largo is a usage error");
    let all = format!("{}{}", stdout(&out), stderr(&out));
    let all = strip_ansi(&all);
    assert!(all.contains("Commands:"), "must list the command overview:\n{all}");
    for cmd in ["new", "init", "build", "run", "check", "publish"] {
        assert!(all.contains(cmd), "overview must mention `{cmd}`:\n{all}");
    }
}

/// Every visible subcommand has an `about` string AND an Examples section
/// (via `after_help`). Hidden commands (reserved verbs) still need `about`.
/// This is a ratchet: any future command must arrive polished.
#[test]
fn every_subcommand_has_about_and_examples() {
    let cmd = logicaffeine_cli::cli::Cli::command();
    for sub in cmd.get_subcommands() {
        let name = sub.get_name();
        if name == "help" {
            continue;
        }
        assert!(
            sub.get_about().is_some(),
            "subcommand `{name}` is missing an about string"
        );
        if sub.is_hide_set() {
            continue;
        }
        let after = sub
            .get_after_help()
            .or_else(|| sub.get_after_long_help())
            .map(|s| s.to_string())
            .unwrap_or_default();
        assert!(
            after.contains("Examples"),
            "subcommand `{name}` is missing an Examples section in after_help"
        );
    }
}

// ---------------------------------------------------------------------------
// Global flags
// ---------------------------------------------------------------------------

/// `--quiet` is a global flag: `largo check -q` succeeds and prints nothing
/// on stdout (errors would still go to stderr).
#[test]
fn quiet_flag_silences_check() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "quiet_proj");
    let out = largo_in(dir.path(), &["check", "-q"]);
    assert_eq!(out.status.code(), Some(0), "check -q should succeed: {}", stderr(&out));
    assert_eq!(stdout(&out), "", "quiet check must print nothing on stdout");
}

/// `--verbose` is accepted globally (used by build passthrough).
#[test]
fn verbose_flag_accepted() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "verbose_proj");
    let out = largo_in(dir.path(), &["-v", "check"]);
    assert_eq!(out.status.code(), Some(0), "-v check should succeed: {}", stderr(&out));
}

// ---------------------------------------------------------------------------
// Color discipline
// ---------------------------------------------------------------------------

/// Piped output carries no ANSI by default (auto mode, not a TTY).
#[test]
fn default_piped_output_has_no_ansi() {
    let dir = tempdir().unwrap();
    let out = largo_in(dir.path(), &["build"]);
    assert!(
        !has_ansi(&stderr(&out)),
        "piped stderr must not contain ANSI by default:\n{:?}",
        stderr(&out)
    );
}

/// `--color always` forces ANSI even into a pipe (CI logs, `less -R`).
#[test]
fn color_always_forces_ansi_on_piped_error() {
    let dir = tempdir().unwrap();
    let out = largo_in(dir.path(), &["--color", "always", "build"]);
    assert!(
        has_ansi(&stderr(&out)),
        "--color always must emit ANSI into the pipe:\n{:?}",
        stderr(&out)
    );
}

/// `NO_COLOR=1` strips ANSI in auto mode (https://no-color.org).
#[test]
fn no_color_env_wins_over_auto() {
    let dir = tempdir().unwrap();
    let out = largo()
        .args(["build"])
        .current_dir(dir.path())
        .env("NO_COLOR", "1")
        .output()
        .expect("largo should spawn");
    assert!(
        !has_ansi(&stderr(&out)),
        "NO_COLOR must strip ANSI:\n{:?}",
        stderr(&out)
    );
}

// ---------------------------------------------------------------------------
// Exit codes + error style
// ---------------------------------------------------------------------------

/// An unknown subcommand is a usage error (exit 2).
#[test]
fn unknown_subcommand_exits_2() {
    let out = largo().arg("frobnicate").output().expect("largo should spawn");
    assert_eq!(out.status.code(), Some(2));
}

/// Command failures render as `error: <message>` plus an actionable
/// `help: <hint>` line, and exit 1.
#[test]
fn error_style_has_error_and_help_lines() {
    let dir = tempdir().unwrap();
    let out = largo_in(dir.path(), &["build"]);
    assert_eq!(out.status.code(), Some(1));
    let err = strip_ansi(&stderr(&out));
    assert!(
        err.lines().next().is_some_and(|l| l.starts_with("error:")),
        "first stderr line must start with `error:`:\n{err}"
    );
    assert!(
        err.lines().any(|l| l.starts_with("help:")),
        "stderr must carry a `help:` hint line:\n{err}"
    );
}

// ---------------------------------------------------------------------------
// Project-name validation
// ---------------------------------------------------------------------------

/// `largo new` rejects names that break downstream layers: empty, path
/// separators, leading dashes, quotes/apostrophes (which would corrupt the
/// generated wasm host shim), and whitespace.
#[test]
fn new_rejects_hostile_project_names() {
    for bad in ["", "x/y", "x\\y", "-flag", "a b", "o'brien", "..", "a\"b"] {
        let dir = tempdir().unwrap();
        let out = largo()
            .args(["new", "--", bad])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "name {bad:?} must be rejected, got: {}",
            stdout(&out)
        );
        let err = strip_ansi(&stderr(&out));
        assert!(err.contains("name"), "error must explain the name rule for {bad:?}:\n{err}");
    }
}

/// Good names still work, including underscores and digits.
#[test]
fn new_accepts_conventional_names() {
    for good in ["hello", "my_project", "app2", "wire-codec"] {
        let dir = tempdir().unwrap();
        let out = largo().args(["new", good]).current_dir(dir.path()).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "name {good:?}: {}", stderr(&out));
        assert!(dir.path().join(good).join("Largo.toml").exists());
    }
}

// ---------------------------------------------------------------------------
// AST depth gate (the round-2 audit's stack-overflow abort, fixed for real)
// ---------------------------------------------------------------------------

/// The shape that used to SIGABRT every surface: a 5000-term expression
/// chain. It must now fail with a graceful diagnostic that teaches both
/// fixes (split into Lets, or raise LOGOS_MAX_AST_DEPTH) — never a signal
/// death.
#[test]
fn deep_expression_chain_is_a_diagnostic_not_a_crash() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "deep_chain");
    let mut program = String::from("# Main\n\n## Main\n\nShow 1");
    for _ in 1..5000 {
        program.push_str(" + 1");
    }
    program.push_str(".\n");
    std::fs::write(dir.path().join("src/main.lg"), program).unwrap();

    let out = largo_in(dir.path(), &["check"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "must exit 1 (a signal death reports None): {:?}",
        out.status
    );
    let err = strip_ansi(&stderr(&out));
    assert!(err.contains("deep"), "must name the problem:\n{err}");
    assert!(
        err.contains("LOGOS_MAX_AST_DEPTH"),
        "must teach the environment override:\n{err}"
    );
}

/// The environment override honors a bigger machine: the same 200-term
/// chain that passes by default also passes with the limit raised, and a
/// chain over the default passes when LOGOS_MAX_AST_DEPTH allows it.
#[test]
fn depth_limit_is_environment_tunable() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "tunable");
    let mut program = String::from("# Main\n\n## Main\n\nShow 1");
    for _ in 1..200 {
        program.push_str(" + 1");
    }
    program.push_str(".\n");
    std::fs::write(dir.path().join("src/main.lg"), program).unwrap();

    // Over the default (128) → rejected…
    let default_run = largo_in(dir.path(), &["check"]);
    assert_eq!(default_run.status.code(), Some(1), "200 > default limit");

    // …but a raised limit accepts it.
    let raised = largo()
        .args(["check"])
        .current_dir(dir.path())
        .env("LOGOS_MAX_AST_DEPTH", "1024")
        .output()
        .unwrap();
    assert_eq!(
        raised.status.code(),
        Some(0),
        "raised limit must accept the 200-term chain: {}",
        stderr(&raised)
    );
}

/// Raising the limit must actually WORK, not re-arm the abort: largo sizes
/// its worker stack FROM the limit, so a 3000-term chain compiles clean
/// under LOGOS_MAX_AST_DEPTH=4096 — the "supercomputer" contract.
#[test]
fn raised_depth_limit_carries_its_own_stack() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "big_machine");
    let mut program = String::from("# Main\n\n## Main\n\nShow 1");
    for _ in 1..5000 {
        program.push_str(" + 1");
    }
    program.push_str(".\n");
    std::fs::write(dir.path().join("src/main.lg"), program).unwrap();

    let out = largo()
        .args(["check"])
        .current_dir(dir.path())
        .env("LOGOS_MAX_AST_DEPTH", "8192")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "a raised limit must genuinely work (None = signal death): {:?}\n{}",
        out.status,
        stderr(&out)
    );
}

// ---------------------------------------------------------------------------
// check --deep (the rustc flycheck pass)
// ---------------------------------------------------------------------------

/// `largo check --deep` runs rustc over the generated code for a clean
/// program and reports success. (Slow: a real cargo check.)
#[test]
fn check_deep_passes_on_a_clean_program() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "deep_clean");
    let out = largo_in(dir.path(), &["check", "--deep"]);
    assert_eq!(out.status.code(), Some(0), "deep check: {}", stderr(&out));
    assert!(
        stdout(&out).contains("Deep check passed"),
        "must confirm the rustc pass:\n{}",
        stdout(&out)
    );
}

// ---------------------------------------------------------------------------
// Reserved `test` verb
// ---------------------------------------------------------------------------

/// `largo test` is reserved: it exits 2 with a message pointing at the future
/// test framework, and stays hidden from `--help`.
#[test]
fn test_verb_is_reserved_and_hidden() {
    let out = largo().arg("test").output().expect("largo should spawn");
    assert_eq!(out.status.code(), Some(2), "reserved verb is a usage error");
    let err = strip_ansi(&stderr(&out));
    assert!(
        err.contains("reserved") && err.contains("test framework"),
        "must explain the verb is reserved for the LOGOS test framework:\n{err}"
    );

    let cmd = logicaffeine_cli::cli::Cli::command();
    let test_sub = cmd
        .get_subcommands()
        .find(|s| s.get_name() == "test")
        .expect("the `test` verb must exist (reserved)");
    assert!(test_sub.is_hide_set(), "`test` must be hidden from help");
}
