//! G28 — a record type reached through `Import` (rather than declared in the same file) never
//! gets its JSON-decode codec generated for a typed fetch that is the record's ONLY reference in
//! the project, unless some `## Route` elsewhere also returns the same type.
//!
//! `parser/lgx_lower/json.rs::seed_fetch_decoders` seeds `json_encoders_needed` from every
//! `Stmt::Fetch`/`Stmt::ComponentDef` it finds in the statement list `lower_derived_json` is handed
//! — reached via `component.rs::resolve_lgx`, which runs on ONE parsed program. A multi-file
//! project is spliced into ONE combined source STRING before that parse
//! (`analysis::assemble_project`'s own doc comment), so the merged statement list should already
//! contain the imported record's declaration and the fetch that uses it. This test builds a real
//! two-file project through that exact API and DRIVES its `## Test` block through
//! `page_host::compare_tests` (a plain `tw_outcome` on a `## Main`-less page runs nothing at all)
//! to see whether the codec is generated in practice.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use tempfile::tempdir;

use logicaffeine_compile::analysis::assemble_project;
use logicaffeine_compile::compile::compile_to_wasm;
use logicaffeine_compile::page_host::compare_tests;

const BUDGET: u64 = 50_000_000;

const SHARED: &str = "\
# Shared

## A Todo has:
    an id, which is Int.
    a text, which is Text.
";

/// The entry page: imports `Todo` and uses it ONLY as a typed fetch's `as` type — no `## Route`
/// anywhere in the project returns it, which is the exact shape gauntlet2's `infinite_scroll`
/// isolated as the failing one (`fullstack_todo`'s working case also declares a matching Route).
const APP: &str = "\
# App

Import Shared.

## Page /

State:
    Let items be a new Seq of Todo.

Actions:
    To load:
        Get \"/api/todos\" as Seq of Todo, then loaded.
    To loaded (r: Result of Seq of Todo and HttpError):
        Inspect r:
            When Ok (v):
                Set items to v.
            When Err (e):
                Set items to a new Seq of Todo.

View:
    <button id=\"load\" on-click={load}>load</button>
    <p id=\"count\">{length of items}</p>

## Test: loads todos from an imported record type

Given that GET \"/api/todos\" answers 200 with \"[]\".
Click \"#load\".
Expect that the text of \"#count\" is equal to \"0\".
";

/// G28's own reproduction: an imported record used ONLY by a typed fetch (no Route anywhere)
/// must decode on the reference tree-walker exactly as the identical declaration written INLINE
/// already does (gauntlet2's own bisection: inline compiles and runs; imported fails with
/// "Unknown function: fromJsonSeq_Todo").
#[test]
fn a_record_imported_and_used_only_by_a_typed_fetch_decodes_on_the_tree_walker() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("shared.lg"), SHARED).unwrap();
    let entry = dir.path().join("app.lg");
    fs::write(&entry, APP).unwrap();
    let assembled = assemble_project(&entry).expect("assemble the two-file project");

    let inline_equivalent = format!(
        "{}\n{}",
        SHARED.trim_start_matches("# Shared\n"),
        APP.replacen("Import Shared.\n\n", "", 1)
    );
    let inline = compare_tests(&inline_equivalent, &[], 0, BUDGET, None)
        .expect("the inline-declared control case must itself parse and run");
    assert!(
        !inline.is_empty(),
        "no scenario discovered in the inline control case"
    );
    assert!(
        inline[0].reference.stopped.is_none(),
        "the inline-declared control case must itself run cleanly on the tree-walker: {:?}",
        inline[0].reference.stopped
    );

    let imported = compare_tests(&assembled.source, &[], 0, BUDGET, None).unwrap_or_else(|e| {
        panic!(
            "failed to parse/run the assembled project: {e}\n{}",
            assembled.source
        )
    });
    assert!(
        !imported.is_empty(),
        "no scenario discovered in the assembled project"
    );
    assert!(
        imported[0].reference.stopped.is_none(),
        "an imported record used only by a typed fetch must decode on the tree-walker exactly as \
         the identical inline declaration already does (G28): {:?}\nassembled source:\n{}",
        imported[0].reference.stopped,
        assembled.source
    );
}

/// The same shape, compiled for the browser — the tier `largo build --static` actually ships.
#[test]
fn a_record_imported_and_used_only_by_a_typed_fetch_compiles_for_the_browser() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("shared.lg"), SHARED).unwrap();
    let entry = dir.path().join("app.lg");
    fs::write(&entry, APP).unwrap();

    let assembled = assemble_project(&entry).expect("assemble the two-file project");
    let result = compile_to_wasm(&assembled.source);
    assert!(
        result.is_ok(),
        "an imported record used only by a typed fetch must compile for the browser: {:?}",
        result.err()
    );
}
