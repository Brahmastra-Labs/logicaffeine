//! `ReplSession` — the replay-based interactive session behind `largo repl`.
//!
//! Architecture under test: the session accumulates source (definitions +
//! Main statements), re-runs the composed program through the REAL engine
//! (`interpret_for_ui_with_args` — VM+JIT with prelude auto-import) on every
//! eval, and surfaces only the output lines past a high-water mark. A failing
//! input rolls back to a no-op, so the session never wedges and never
//! duplicates output. `source()` is always a valid, runnable LOGOS program.

use logicaffeine_compile::repl::ReplSession;

/// A bare expression statement shows its value.
#[test]
fn eval_shows_arithmetic() {
    let mut s = ReplSession::new();
    let out = s.eval_sync("Show 1 + 1.");
    assert!(out.error.is_none(), "error: {:?}", out.error);
    assert_eq!(out.new_lines, vec!["2"]);
}

/// Bindings persist across evals.
#[test]
fn bindings_persist_across_lines() {
    let mut s = ReplSession::new();
    assert!(s.eval_sync("Let x be 5.").error.is_none());
    let out = s.eval_sync("Show x.");
    assert!(out.error.is_none(), "error: {:?}", out.error);
    assert_eq!(out.new_lines, vec!["5"]);
}

/// Prior output is never re-printed (high-water mark).
#[test]
fn no_reprint_of_prior_output() {
    let mut s = ReplSession::new();
    assert_eq!(s.eval_sync("Show \"a\".").new_lines, vec!["a"]);
    assert_eq!(s.eval_sync("Show \"b\".").new_lines, vec!["b"]);
}

/// Mutation via Set carries forward.
#[test]
fn mutation_carries_forward() {
    let mut s = ReplSession::new();
    assert!(s.eval_sync("Let x be 5.").error.is_none());
    assert!(s.eval_sync("Set x to x + 1.").error.is_none());
    assert_eq!(s.eval_sync("Show x.").new_lines, vec!["6"]);
}

/// A function defined in one eval is callable in later evals.
#[test]
fn function_defined_then_called() {
    let mut s = ReplSession::new();
    let def = s.eval_sync("## To double (n: Int) -> Int:\n    Return n * 2.");
    assert!(def.error.is_none(), "def error: {:?}", def.error);
    let out = s.eval_sync("Show double(21).");
    assert!(out.error.is_none(), "call error: {:?}", out.error);
    assert_eq!(out.new_lines, vec!["42"]);
}

/// A failing statement reports its error, rolls back, and later inputs work
/// with no duplicated output.
#[test]
fn runtime_error_rolls_back() {
    let mut s = ReplSession::new();
    assert_eq!(s.eval_sync("Show \"before\".").new_lines, vec!["before"]);
    let bad = s.eval_sync("Show undefined_variable_xyz.");
    assert!(bad.error.is_some(), "must report the failure");
    let after = s.eval_sync("Show 7.");
    assert!(after.error.is_none(), "session must recover: {:?}", after.error);
    assert_eq!(after.new_lines, vec!["7"], "no duplicates, no loss");
}

/// A parse failure likewise rolls back with a message.
#[test]
fn parse_error_rolls_back() {
    let mut s = ReplSession::new();
    let bad = s.eval_sync("Blorp the glorp frobnicate.");
    assert!(bad.error.is_some(), "garbage must fail");
    let after = s.eval_sync("Show 3.");
    assert!(after.error.is_none(), "session must recover: {:?}", after.error);
    assert_eq!(after.new_lines, vec!["3"]);
}

/// Definitions arriving AFTER statements still work (defs float to the top
/// of the composed program).
#[test]
fn definition_after_statements() {
    let mut s = ReplSession::new();
    assert!(s.eval_sync("Let x be 10.").error.is_none());
    assert!(s
        .eval_sync("## To triple (n: Int) -> Int:\n    Return n * 3.")
        .error
        .is_none());
    assert_eq!(s.eval_sync("Show triple(x).").new_lines, vec!["30"]);
}

/// A definition-only session (no statements yet) is valid.
#[test]
fn definition_before_any_statement() {
    let mut s = ReplSession::new();
    let def = s.eval_sync("## To half (n: Int) -> Int:\n    Return n / 2.");
    assert!(def.error.is_none(), "def-only session: {:?}", def.error);
    assert_eq!(s.eval_sync("Show half(84).").new_lines, vec!["42"]);
}

/// A multi-statement chunk evaluates atomically.
#[test]
fn multi_statement_chunk() {
    let mut s = ReplSession::new();
    let out = s.eval_sync("Let a be 1.\nLet b be 2.\nShow a + b.");
    assert!(out.error.is_none(), "chunk: {:?}", out.error);
    assert_eq!(out.new_lines, vec!["3"]);
}

/// `vars()` reports the session's global bindings with types and values.
#[test]
fn vars_reports_global_bindings() {
    let mut s = ReplSession::new();
    assert!(s.eval_sync("Let x be 5.").error.is_none());
    assert!(s.eval_sync("Let name be \"logos\".").error.is_none());
    let vars = s.vars();
    assert!(
        vars.contains(&("x".to_string(), "Int".to_string(), "5".to_string())),
        "vars: {vars:?}"
    );
    assert!(
        vars.iter().any(|(n, t, _)| n == "name" && t == "Text"),
        "vars: {vars:?}"
    );
}

/// `source()` is a valid program reproducing the whole ledger, and
/// `load_source` restores the session from it.
#[test]
fn source_round_trips() {
    let mut s = ReplSession::new();
    s.eval_sync("## To double (n: Int) -> Int:\n    Return n * 2.");
    s.eval_sync("Let x be 4.");
    s.eval_sync("Show double(x).");

    let program = s.source();
    let rerun = logicaffeine_compile::interpret_for_ui_sync_with_args(&program, &["repl".into()]);
    assert!(rerun.error.is_none(), "saved program must run: {:?}", rerun.error);
    assert_eq!(rerun.lines, vec!["8"], "saved program reproduces the ledger");

    let mut restored = ReplSession::new();
    let load = restored.load_source(&program);
    assert!(load.error.is_none(), "load: {:?}", load.error);
    assert_eq!(restored.eval_sync("Show double(5).").new_lines, vec!["10"]);
}

/// `binding_names()` lists bound names WITHOUT executing anything — it is a
/// textual scan (the completion feed must not replay side effects).
#[test]
fn binding_names_scan_without_execution() {
    let mut s = ReplSession::new();
    s.eval_sync("Let counter be 1.");
    s.eval_sync("## To double (n: Int) -> Int:\n    Return n * 2.");
    s.eval_sync("Let name be \"x\".");
    let names = s.binding_names();
    for expected in ["counter", "double", "name"] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing `{expected}` in {names:?}"
        );
    }
}

/// A `## Main`-prefixed header that is NOT exactly `## Main` (e.g.
/// `## Mainframe`) must be treated as a definition, not swallowed as the
/// Main marker.
#[test]
fn load_source_distinguishes_main_from_main_prefixed_headers() {
    let program = "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\n\nShow double(4).\n";
    let mut s = ReplSession::new();
    let load = s.load_source(program);
    assert!(load.error.is_none(), "load: {:?}", load.error);
    assert_eq!(load.new_lines, vec!["8"]);
}

/// A hand-written file with no `## ` headers at all loads as pure Main
/// body — not silently as nothing.
#[test]
fn load_source_headerless_file_is_main_body() {
    let mut s = ReplSession::new();
    let load = s.load_source("Let x be 5.\nShow x.\n");
    assert!(load.error.is_none(), "load: {:?}", load.error);
    assert_eq!(load.new_lines, vec!["5"]);
    assert_eq!(s.eval_sync("Show x + 1.").new_lines, vec!["6"]);
}

/// `reset()` clears everything.
#[test]
fn reset_clears_session() {
    let mut s = ReplSession::new();
    assert!(s.eval_sync("Let x be 5.").error.is_none());
    s.reset();
    let out = s.eval_sync("Show x.");
    assert!(out.error.is_some(), "x must be gone after reset");
}
