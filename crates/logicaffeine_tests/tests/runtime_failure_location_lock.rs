//! ★ LOCK — every runtime failure kind inside an action reports the SAME located message on the
//! tree-walker and the VM. ★
//!
//! Wave 3: "runtime failures carry a location". `compile.rs::{tw_outcome_located, vm_outcome_
//! located}` prefix a failing run's own error text with `line:col — FailureSite::render(...)` —
//! the sentence's file position, and which component/action it is written in — read from
//! `Interpreter::current_site`/`Vm::failing_site` through `ui_bridge::failure_sites_of`'s table.
//!
//! A CATALOGUE, not a patch: every distinct runtime-failure MECHANISM the interpreter/VM can raise
//! from inside an ordinary action gets its own row here — an unlisted mechanism is not caught
//! automatically (there is no enum to scrape the way `diagnostic_location_lock` scrapes
//! `NotLowerable`; a runtime `Err(String)` has no such registry), so this file's own doc comment is
//! the census, kept honest by requiring each row to also appear in `semantics::failcode::ALL` where
//! one exists (the wire-visible, append-only list the WASM backend is held to) — a fourth code
//! there without a fourth row here is the drift this lock exists to catch.
//!
//! **The members of `ALL` deliberately NOT rows are `NOT_ROWS`, each with its reason.**
//! `FailCode::OutOfMemory` needs the VM's allocator to actually exhaust its arena, which has no small,
//! fast, deterministic trigger inside a plain bytecode-tier action (unlike the WASM backend's fixed
//! linear memory) — reaching it here would mean a slow, memory-pressuring fixture whose failure point
//! depends on the allocator's own internals, not a stable line the author wrote.
//! `FailCode::IntTooWide` is a compiled module's own refusal of an integer past a machine word where it
//! holds a word; the VM holds that integer exact and never fails there. Recorded here rather than
//! silently absent.

use logicaffeine_compile::compile::{tw_outcome_located, vm_outcome_located};

/// One row: a page whose action's SECOND sentence (line 7) fails for a specific reason, the phrase
/// its located message must contain, and the `semantics::failcode::FailCode` it corresponds to (if
/// any — a row with no code, like `Check`, has none to cross-check against).
struct Row {
    name: &'static str,
    src: &'static str,
    says: &'static str,
    fail_code: Option<logicaffeine_compile::semantics::failcode::FailCode>,
}

/// The members of `ALL` that are not rows, each with the reason no tree-walker or VM action can raise it.
const NOT_ROWS: &[(logicaffeine_compile::semantics::failcode::FailCode, &str)] = &[
    (
        logicaffeine_compile::semantics::failcode::FailCode::OutOfMemory,
        "needs the VM's allocator to exhaust its arena (module doc comment above)",
    ),
    (
        logicaffeine_compile::semantics::failcode::FailCode::IntTooWide,
        "a compiled module's own refusal: an integer past a machine word reaching a place the module holds \
         a word (an element, a field), which the VM holds exact",
    ),
];

fn rows() -> Vec<Row> {
    vec![
        Row {
            name: "index_out_of_bounds",
            src: "## Component App\n\
                  State:\n    Let xs be [1, 2].\n\
                  Actions:\n    To boom:\n        Let x be 1.\n        Show item 9 of xs.\n\
                  View:\n    <button on-click={boom}>go</button>\n\n\
                  ## Main\nMount App.\nClick \"button\".\n",
            says: "out of bounds",
            fail_code: Some(logicaffeine_compile::semantics::failcode::FailCode::IndexOutOfBounds),
        },
        Row {
            name: "division_by_zero",
            src: "## Component App\n\
                  State:\n    Let z be 0.\n\
                  Actions:\n    To boom:\n        Let x be 1.\n        Let y be 1 / z.\n\
                  View:\n    <button on-click={boom}>go</button>\n\n\
                  ## Main\nMount App.\nClick \"button\".\n",
            says: "Division by zero",
            fail_code: Some(logicaffeine_compile::semantics::failcode::FailCode::DivisionByZero),
        },
        Row {
            name: "list_index_past_a_word",
            src: "## Component App\n\
                  State:\n    Let xs be [1, 2].\n\
                  Actions:\n    To boom:\n        Let x be 1.\n        Show item (4611686018427387904 * 4) of xs.\n\
                  View:\n    <button on-click={boom}>go</button>\n\n\
                  ## Main\nMount App.\nClick \"button\".\n",
            says: "Cannot index List with Int",
            fail_code: Some(logicaffeine_compile::semantics::failcode::FailCode::ListIndexTooWide),
        },
        Row {
            name: "text_index_past_a_word",
            src: "## Component App\n\
                  State:\n    Let t be \"ab\".\n\
                  Actions:\n    To boom:\n        Let x be 1.\n        Show item (4611686018427387904 * 4) of t.\n\
                  View:\n    <button on-click={boom}>go</button>\n\n\
                  ## Main\nMount App.\nClick \"button\".\n",
            says: "Cannot index Text with Int",
            fail_code: Some(logicaffeine_compile::semantics::failcode::FailCode::TextIndexTooWide),
        },
        Row {
            name: "list_store_index_past_a_word",
            src: "## Component App\n\
                  State:\n    Let xs be [1, 2].\n\
                  Actions:\n    To boom:\n        Let x be 1.\n        Set item (4611686018427387904 * 4) of xs to 3.\n\
                  View:\n    <button on-click={boom}>go</button>\n\n\
                  ## Main\nMount App.\nClick \"button\".\n",
            says: "List index must be an integer",
            fail_code: Some(logicaffeine_compile::semantics::failcode::FailCode::ListStoreIndexTooWide),
        },
        Row {
            name: "invalid_utf8",
            src: "## Component App\n\
                  State:\n    Let bad be [255, 255].\n\
                  Actions:\n    To boom:\n        Let x be 1.\n        Let y be text_from_bytes(bad).\n\
                  View:\n    <button on-click={boom}>go</button>\n\n\
                  ## Main\nMount App.\nClick \"button\".\n",
            says: "invalid UTF-8",
            fail_code: Some(logicaffeine_compile::semantics::failcode::FailCode::InvalidUtf8),
        },
    ]
}

#[test]
fn every_row_matches_a_real_failcode_or_says_why_not() {
    // The census's own honesty check: every row with a `fail_code` must name a member of `ALL`
    // (reading the source, not a hand-copied list), and `ALL`'s members not covered by
    // `OutOfMemory`'s documented exemption must have a row.
    let rows = rows();
    let covered: std::collections::HashSet<_> = rows
        .iter()
        .filter_map(|r| r.fail_code)
        .map(|c| c.code())
        .collect();
    for code in logicaffeine_compile::semantics::failcode::ALL {
        if NOT_ROWS.iter().any(|(c, why)| c == code && !why.is_empty()) {
            continue; // documented exemptions, `NOT_ROWS`.
        }
        assert!(
            covered.contains(&code.code()),
            "FailCode {code:?} has no row in this census — either add one or document why not"
        );
    }
}

#[test]
fn every_runtime_failure_kind_reports_the_same_located_message_on_both_tiers() {
    let mut wrong = Vec::new();
    for row in rows() {
        logicaffeine_compile::semantics::dom::reset();
        let tw = tw_outcome_located(row.src);
        logicaffeine_compile::semantics::dom::reset();
        let vm = vm_outcome_located(row.src);
        let (Some(tw_err), Some(vm_err)) = (&tw.error, &vm.error) else {
            wrong.push(format!(
                "{}: must fail on both tiers: tw={:?} vm={:?}",
                row.name, tw.error, vm.error
            ));
            continue;
        };
        if tw_err != vm_err {
            wrong.push(format!(
                "{}: tree-walker and VM worded the location differently:\n  tw: {tw_err}\n  vm: {vm_err}",
                row.name
            ));
            continue;
        }
        if !tw_err.contains(row.says) {
            wrong.push(format!(
                "{}: {tw_err:?} does not contain {:?}",
                row.name, row.says
            ));
        }
        if !tw_err.starts_with("7:") {
            wrong.push(format!(
                "{}: must locate at line 7 (the action's second sentence): {tw_err}",
                row.name
            ));
        }
        if !tw_err.contains("To boom") {
            wrong.push(format!(
                "{}: must name the action `To boom`: {tw_err}",
                row.name
            ));
        }
        if !tw_err.contains("App") {
            wrong.push(format!(
                "{}: must name the component `App`: {tw_err}",
                row.name
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A failure OUTSIDE any action carries no site, on either tier — the located and unlocated forms
/// must be byte-identical (nothing to prefix).
#[test]
fn a_failure_outside_any_action_is_unchanged_by_locating_it() {
    let src = "## Main\nLet xs be [1, 2].\nShow item 9 of xs.\n";
    let located = tw_outcome_located(src);
    let plain = logicaffeine_compile::compile::tw_outcome(src);
    assert_eq!(
        located, plain,
        "a top-level failure has no site to locate — located must equal plain"
    );
    let located_vm = vm_outcome_located(src);
    let plain_vm = logicaffeine_compile::compile::vm_outcome(src);
    assert_eq!(located_vm, plain_vm);
}
