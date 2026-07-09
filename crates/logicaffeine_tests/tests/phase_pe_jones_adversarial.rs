//! Phase 1 — Adversarial hardening of the Jones-optimality oracle.
//!
//! `count_dispatch` is the gate that decides a PE residual has fully dissolved the
//! interpreter. If it can be fooled, every downstream Jones lock is fooled with it.
//! Each program below carries GENUINE interpreter overhead — a `coreEval` dispatch
//! call, an `env` lookup, or a Core-IR constructor — hidden inside a syntactic
//! position the naive counter walks straight past (`_ => {}`).
//!
//! A hardened oracle scores each `> 0` and never `usize::MAX` (which means the probe
//! failed to PARSE rather than being detected — a false pass we refuse to accept).

use logicaffeine_compile::compile::count_dispatch;

/// (name, program) — each hides exactly one dispatch unit the naive oracle misses.
fn cheater_corpus() -> Vec<(&'static str, &'static str)> {
    vec![
        // Core dispatch call nested in container / operator expressions that the naive
        // `count_expr_dispatch` falls through with `_ => {}`.
        ("list_nesting", "## Main\nLet xs be [coreEval(a, b, c)].\nShow \"ok\"."),
        ("tuple_nesting", "## Main\nLet t be (coreEval(a, b, c), 1).\nShow \"ok\"."),
        ("option_some_nesting", "## Main\nLet o be some coreEval(a, b, c).\nShow \"ok\"."),
        ("union_nesting", "## Main\nLet u be p union coreEval(a, b, c).\nShow \"ok\"."),
        ("copy_nesting", "## Main\nLet s be copy of coreEval(a, b, c).\nShow \"ok\"."),
        (
            "contains_nesting",
            "## Main\nLet flag be s contains coreEval(a, b, c).\nShow \"ok\".",
        ),
        // Interpolated string interpolant — walked past by `_ => {}`.
        ("interp_string", "## Main\nShow \"v={coreEval(a, b, c)}\"."),
        // Constructor whose FIELD value carries a Core call — the naive `New` arm checks
        // only the type name and never recurses into `init_fields`.
        (
            "new_field",
            "## A Widget has:\n    A slot: Int.\n\n## Main\nLet w be a new Widget with slot coreEval(a, b, c).\nShow \"ok\".",
        ),
        // Environment lookup with a NON-literal key: the naive check only trips on a
        // literal Text key, so a variable key threads the interpreter env untouched.
        (
            "env_nonliteral_key",
            "## Main\nLet env be a new Map of Text to Int.\nLet k be \"x\".\nLet v be item k of env.\nShow \"ok\".",
        ),
        // Core-IR construction smuggled inside an `Escape` block — opaque to the AST walk,
        // so it must be caught by a lexical scan of the raw foreign code.
        (
            "escape_core_ir",
            "## Main\nEscape to Rust:\n    let x = CInt(42);\nShow \"ok\".",
        ),
        // A RENAMED dispatcher (not on the `DISPATCH_FN_NAMES` allowlist) is still caught
        // structurally: its body inspects a Core variant, which the FunctionDef recursion counts.
        (
            "renamed_dispatcher",
            "## To myEval (e: CExpr) -> CVal:\n    Inspect e:\n        When CInt (v):\n            Return a new VInt with value v.\n        Otherwise:\n            Return a new VNothing.\n\n## Main\nLet r be myEval(x).\nShow \"ok\".",
        ),
        // Constructing the interpreter's environment map — `a new Map of Text to CVal` — is
        // surviving interpreter state even with no dispatch call in sight.
        (
            "env_map_construction",
            "## Main\nLet m be a new Map of Text to CVal.\nShow \"ok\".",
        ),
        // Binding a variable at a Core carrier type is likewise residual interpreter state.
        (
            "core_typed_binding",
            "## Main\nLet m: Map of Text to CVal be existing.\nShow \"ok\".",
        ),
    ]
}

/// Every hidden dispatch unit must be detected: `0 < count < usize::MAX`.
#[test]
fn cheaters_carry_detectable_overhead() {
    let mut failures = Vec::new();
    for (name, program) in cheater_corpus() {
        let d = count_dispatch(program);
        if d == 0 {
            failures.push(format!("[{name}] overhead SLIPPED PAST the oracle (count 0)"));
        } else if d == usize::MAX {
            failures.push(format!("[{name}] probe failed to parse (count MAX) — fix the probe"));
        }
    }
    assert!(
        failures.is_empty(),
        "oracle evasion — {} vector(s) uncaught:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Over-counting guard: genuinely clean, residual-shaped code stays at exactly 0, so the
/// hardening tightens detection without flagging legitimate specialized output.
#[test]
fn clean_programs_score_zero() {
    let clean = [
        "## Main\nShow 2 + 3 * 4.",
        "## Main\nLet xs be [1, 2, 3].\nShow item 2 of xs.",
        "## Main\nLet t be (5, 6, 7).\nShow item 3 of t.",
        "## Main\nLet m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nShow item \"a\" of m.",
    ];
    for p in clean {
        assert_eq!(count_dispatch(p), 0, "clean program falsely flagged as overhead:\n{p}");
    }
}
