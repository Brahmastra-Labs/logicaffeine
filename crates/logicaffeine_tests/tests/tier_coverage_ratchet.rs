//! ════════════════════════════════════════════════════════════════════════════
//! TIER-COVERAGE RATCHET — locks Invariant II (differential-by-default) in place.
//!
//! A behavioral test that pins a SINGLE execution tier (the interpreter-only
//! `assert_interpreter_output*` / `run_interpreter` helpers) can silently diverge
//! from the other tiers without anyone noticing. The cross-tier replacement is
//! `assert_all_tiers` / `run_all_tiers` (common/mod.rs), which runs tree-walker AND
//! VM and asserts they agree.
//!
//! This ratchet freezes the count of interpreter-only call sites and lets it ONLY
//! SHRINK: you may not ADD a new single-tier behavioral test (write it with
//! `assert_all_tiers` instead), and as existing tests migrate the ceiling drops.
//! Mirrors the monotone `SUPPORTED_PROGRAM_FLOOR` ratchet in wasm_aot_lock.rs.
//!
//! ⚠️ YOU DO NOT GET TO RAISE THE CEILING TO GO GREEN. ⚠️  If this fails because the
//! count grew, convert your new test to `assert_all_tiers`. Only ever LOWER the
//! ceiling — after a real migration removes single-tier call sites.

use std::path::{Path, PathBuf};

/// The frozen ceiling of interpreter-only behavioral call sites (the count this file's
/// own `count_calls_filtered` sees today — it catches every `assert_interpreter_output*` and
/// `run_interpreter*` variant that is NOT the interpreter leg of a multi-tier comparison, so it is
/// the authoritative single-tier baseline). Monotone: only ever LOWERED by a human after migrating
/// tests to the cross-tier helpers (or, as here, after the counter stopped false-flagging the
/// interpreter legs of genuine multi-tier CRDT parity tests, and again once it recognised the
/// AOT-Rust and C comparison helpers as other tiers — the accurate count is now 772).
const SINGLE_TIER_CEILING: usize = 772;

/// The other-tier oracles: a `fn` referencing ANY of these runs the VM and/or the compiled/AOT tier
/// alongside the interpreter, so an `assert_interpreter_output*`/`run_interpreter*` call inside it is
/// the interpreter LEG of a genuine MULTI-TIER comparison — NOT a single-tier-only behavioral test.
/// The ratchet must not false-flag those legs (that would push a real cross-tier lock past the
/// ceiling). This is the "pit of success" refinement: a single-tier call is one whose enclosing test
/// does NOT also exercise another tier.
const OTHER_TIER_ORACLES: &[&str] = &[
    "run_all_tiers",
    "assert_all_tiers",
    "vm_outcome", // covers vm_outcome / vm_outcome_net / vm_outcome_concurrent / vm_outcome_with_args
    "run_logos",  // the compiled/native-AOT oracle
    "assert_compiled_equals_interpreted", // AOT-Rust against the interpreter, through `run_logos`
    "assert_c_output", // the C tier
];

/// The `{`-balanced body of the `fn` enclosing byte `idx`, or the whole tail from the `fn` header if
/// unbalanced. Used to decide whether a single-tier call sits inside a multi-tier test.
fn enclosing_fn_body(content: &str, idx: usize) -> &str {
    let bytes = content.as_bytes();
    // The nearest `fn ` at column 0 or one indent level (free fn or an inherent/test fn).
    let fn_start = content[..idx]
        .rfind("\nfn ")
        .max(content[..idx].rfind("\n    fn "))
        .map(|p| p + 1)
        .unwrap_or(0);
    let Some(brace) = content[fn_start..].find('{').map(|b| fn_start + b) else {
        return &content[fn_start..];
    };
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate().skip(brace) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &content[fn_start..=i];
                }
            }
            _ => {}
        }
    }
    &content[fn_start..]
}

/// Count call sites of `base` (base name + any identifier suffix, then `(`) in `content`, ignoring
/// bare mentions in comments/strings without a call. When `single_tier_only` is set, a call whose
/// enclosing `fn` ALSO runs another tier (an interpreter leg of a multi-tier comparison) is NOT
/// counted — that is the single-tier ratchet's semantics; the helper-liveness count leaves it off.
fn count_calls_filtered(content: &str, base: &str, single_tier_only: bool) -> usize {
    let bytes = content.as_bytes();
    let mut count = 0usize;
    let mut start = 0usize;
    while let Some(rel) = content[start..].find(base) {
        let idx = start + rel;
        let mut j = idx + base.len();
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
        if j < bytes.len() && bytes[j] == b'(' {
            let is_multi_tier_leg = single_tier_only && {
                let body = enclosing_fn_body(content, idx);
                OTHER_TIER_ORACLES.iter().any(|o| body.contains(o))
            };
            if !is_multi_tier_leg {
                count += 1;
            }
        }
        start = idx + base.len();
    }
    count
}

/// Count ALL call sites of `base` (the un-filtered form — used by the helper-liveness check).
fn count_calls(content: &str, base: &str) -> usize {
    count_calls_filtered(content, base, false)
}

/// Every `.rs` file under `tests/`, except the shared helper definitions
/// (`common/mod.rs`) and this ratchet file itself.
fn test_source_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let is_common = p.ends_with(Path::new("common/mod.rs"));
                if !is_common && name != "tier_coverage_ratchet.rs" {
                    out.push(p);
                }
            }
        }
    }
    out
}

fn single_tier_call_count() -> usize {
    let mut total = 0usize;
    for f in test_source_files() {
        let Ok(content) = std::fs::read_to_string(&f) else {
            continue;
        };
        total += count_calls_filtered(&content, "assert_interpreter_output", true);
        total += count_calls_filtered(&content, "run_interpreter", true);
    }
    total
}

/// The single-tier interpreter-only call count must never exceed the frozen ceiling.
#[test]
fn single_tier_behavioral_tests_do_not_grow() {
    let count = single_tier_call_count();
    assert!(
        count <= SINGLE_TIER_CEILING,
        "TIER-COVERAGE REGRESSED: {count} interpreter-only call sites, ceiling is \
         {SINGLE_TIER_CEILING}. You added a single-tier behavioral test — write it with \
         `assert_all_tiers` / `run_all_tiers` (tree-walker ≡ VM) instead. ⚠️ DO NOT raise \
         the ceiling to go green. ⚠️"
    );
    // If a migration lowered the count well below the ceiling, ratchet the ceiling down.
    assert!(
        count + 25 >= SINGLE_TIER_CEILING,
        "Single-tier count dropped to {count} (ceiling {SINGLE_TIER_CEILING}). A migration \
         removed call sites — LOWER `SINGLE_TIER_CEILING` to {count} to lock in the progress."
    );
}

/// The cross-tier helper must stay live — it cannot be silently deleted, which would
/// quietly disable differential-by-default for every test that uses it.
#[test]
fn differential_by_default_helper_is_alive() {
    let uses: usize = test_source_files()
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .map(|c| count_calls(&c, "run_all_tiers") + count_calls(&c, "assert_all_tiers"))
        .sum();
    assert!(
        uses >= 1,
        "The differential-by-default helper `run_all_tiers`/`assert_all_tiers` has NO call \
         sites — Invariant II's default path is dead. New behavioral tests must adopt it."
    );
}
