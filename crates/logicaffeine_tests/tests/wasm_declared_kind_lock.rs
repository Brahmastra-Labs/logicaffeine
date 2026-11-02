//! ★ LOCK — a value's kind on wasm comes from what the program DECLARES. ★
//!
//! The wasm backend types every register statically (an `i64` Int, an `i32` handle to a
//! `Seq of Text`, …) and it is compiled from VM bytecode, where types are gone. Inferring a kind
//! from what the ops later do to a value runs out exactly where a program is most ordinary: a list
//! declared EMPTY has no element to infer from, a module-level list that Main never pushes to is
//! element-less in every function that reads it, a map that is only ever read has no value written
//! into it. Each of those states its type where it is declared — `Let names be a new Seq of Text.`,
//! `a tape, which is Seq of Int`, `Map of Text to Int` — and the declaration is the answer; inference
//! fills in only where nothing is declared.
//!
//! Every program row runs on three tiers and compares whole outcomes (output AND failure) against
//! the tree-walker, the reference. A row the wasm backend refuses fails as loudly as a wrong answer.
//! Page rows compile a page and ask a real engine to load the module.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};

#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::run_outcome;

/// `(name, program)`.
fn rows() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "a_module_text_list_declared_untyped_never_pushed_and_read_only_in_a_function",
            "## To disp (ev: Text) -> Bool:\n    Let oe be qe.\n    Repeat for s in oe:\n        Show s.\n    Return true.\n\n\
             ## Main\nLet mutable qe be a new Seq of Text.\nLet r be disp(\"flush\").\nShow r.\n",
        ),
        (
            "a_module_text_list_declared_with_its_type_never_pushed_and_read_only_in_a_function",
            "## To disp (ev: Text) -> Bool:\n    Let oe be qe.\n    Repeat for s in oe:\n        Show s.\n    Return true.\n\n\
             ## Main\nLet mutable qe: Seq of Text be a new Seq of Text.\nLet r be disp(\"flush\").\nShow r.\n",
        ),
        (
            "a_module_text_list_only_ever_pushed_a_literal_inside_a_function",
            "## To note (s: Text) -> Int:\n    Push \"seen\" to log.\n    Return length of log.\n\n\
             ## To dump -> Int:\n    Let n be 0.\n    Repeat for line in log:\n        Show line.\n        Set n to n + 1.\n    Return n.\n\n\
             ## Main\nLet mutable log be a new Seq of Text.\nShow dump().\nLet a be note(\"x\").\nLet b be note(\"y\").\nShow dump().\n",
        ),
        (
            "a_module_int_list_never_pushed_is_summed_in_a_function",
            "## To total -> Int:\n    Let t be 0.\n    Repeat for x in xs:\n        Set t to t + x.\n    Return t + length of xs.\n\n\
             ## Main\nLet xs be a new Seq of Int.\nShow total().\n",
        ),
        (
            "a_module_record_list_never_pushed_is_walked_in_a_function",
            "## A Todo has:\n    a text, which is Text.\n    a done, which is Bool.\n\n\
             ## To open -> Int:\n    Let n be 0.\n    Repeat for t in todos:\n        Show t's text.\n        If not t's done:\n            Set n to n + 1.\n    Return n.\n\n\
             ## Main\nLet todos be a new Seq of Todo.\nShow open().\n",
        ),
        (
            "a_module_map_never_written_is_read_in_a_function",
            "## To known (k: Text) -> Bool:\n    Return seen contains k.\n\n\
             ## To count -> Int:\n    Return length of seen.\n\n\
             ## Main\nLet seen be a new Map of Text to Int.\nShow known(\"a\").\nShow count().\n",
        ),
        (
            "a_local_text_list_declared_and_never_pushed",
            "## Main\nLet names be a new Seq of Text.\nRepeat for n in names:\n    Show n.\nShow length of names.\n",
        ),
        (
            "a_function_local_text_list_declared_and_never_pushed",
            "## To listed -> Int:\n    Let names be a new Seq of Text.\n    Repeat for n in names:\n        Show n.\n    Return length of names.\n\n\
             ## Main\nShow listed().\n",
        ),
        (
            "a_bare_local_map_is_written_and_read",
            "## Main\nLet seen be a new Map of Text to Int.\nSet item \"a\" of seen to 1.\n\
             Set item \"b\" of seen to 2.\nShow item \"a\" of seen.\nShow length of seen.\n",
        ),
        ("a_bare_local_map_that_is_never_written", "## Main\nLet seen be a new Map of Text to Int.\nShow length of seen.\n"),
        (
            "a_bare_local_map_of_int_to_text",
            "## Main\nLet names be a new Map of Int to Text.\nSet item 3 of names to \"three\".\n\
             Show item 3 of names.\nShow names contains 3.\nShow names contains 4.\n",
        ),
        (
            "a_bare_local_map_only_read_with_contains",
            "## Main\nLet seen be a new Map of Text to Int.\nIf seen contains \"x\":\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\n",
        ),
        (
            "a_records_text_field_interpolated_after_a_copy",
            "## A Doc has:\n    a name, which is Text.\n    a size, which is Int.\n\n## Main\n\
             Let d be a new Doc with name \"photo.png\" and size 12.\nLet n be d's name.\nLet s be d's size.\n\
             Show \"{n} is {s} bytes\".\n",
        ),
        (
            "a_records_fields_interpolated_directly",
            "## A Doc has:\n    a name, which is Text.\n    a size, which is Int.\n\n## Main\n\
             Let d be a new Doc with name \"photo.png\" and size 12.\nShow \"{d's name}:{d's size}\".\n",
        ),
        (
            "a_record_built_out_of_declaration_order_reads_the_same_through_a_parameter",
            "## A Pair has:\n    a first, which is Int.\n    a second, which is Text.\n\n\
             ## To describe(p: Pair) -> Text:\n    Return \"{p's second}={p's first}\".\n\n\
             ## Main\nLet p be a new Pair with second \"x\" and first 3.\nShow describe(p).\nShow p's first.\n",
        ),
        (
            "a_record_built_out_of_declaration_order_reads_the_same_through_a_list",
            "## A Pair has:\n    a first, which is Int.\n    a second, which is Text.\n\n\
             ## To firstOf(ps: Seq of Pair) -> Int:\n    Return (item 1 of ps)'s first.\n\n\
             ## Main\nLet ps be a new Seq of Pair.\nPush a new Pair with second \"y\" and first 8 to ps.\n\
             Show firstOf(ps).\nShow (item 1 of ps)'s second.\n",
        ),
        (
            "a_record_with_a_defaulted_field_reads_the_same_through_a_parameter",
            "## A Pair has:\n    a first, which is Int.\n    a second, which is Text.\n\n\
             ## To describe(p: Pair) -> Text:\n    Return \"[{p's second}]{p's first}\".\n\n\
             ## Main\nLet p be a new Pair with second \"z\".\nShow describe(p).\n",
        ),
        (
            "a_records_empty_list_field_is_walked",
            "## A Log has:\n    a lines, which is Seq of Text.\n\n## Main\n\
             Let l be a new Log with lines (a new Seq of Text).\nRepeat for s in l's lines:\n    Show s.\nShow length of l's lines.\n",
        ),
        (
            "a_records_defaulted_list_field_is_walked_through_a_parameter",
            "## A Log has:\n    a lines, which is Seq of Text.\n\n\
             ## To size(l: Log) -> Int:\n    Repeat for s in l's lines:\n        Show s.\n    Return length of l's lines.\n\n\
             ## Main\nLet l be a new Log.\nShow size(l).\n",
        ),
    ]
}

/// One tier's whole outcome: what it showed, and the failure it stopped with.
type Outcome = (String, Option<String>);

fn three_tiers(src: &str) -> [Outcome; 3] {
    let tw = tw_outcome(src);
    let vm = vm_outcome(src);
    let wasm = match run_outcome(src) {
        Ok(run) => (run.output, run.failure),
        Err(why) => (String::new(), Some(why)),
    };
    let trim = |s: String| s.trim_end_matches('\n').to_string();
    [
        (trim(tw.output), tw.error),
        (trim(vm.output), vm.error),
        (trim(wasm.0), wasm.1),
    ]
}

#[test]
fn every_declared_kind_row_runs_identically_on_wasm() {
    let rows = rows();
    let mut failures = Vec::new();
    for (name, src) in &rows {
        let outcomes = three_tiers(src);
        let [tw, vm, wasm] = &outcomes;
        if tw.1.is_some() || vm != tw || wasm != tw {
            let tiers = ["tree-walker", "VM", "wasm"]
                .iter()
                .zip(&outcomes)
                .map(|(tier, (out, err))| format!("  {tier}: {out:?} {err:?}"))
                .collect::<Vec<_>>()
                .join("\n");
            failures.push(format!("── {name}\n{tiers}\n{src}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} rows diverge:\n{}",
        failures.len(),
        rows.len(),
        failures.join("\n")
    );
}

/// A page whose outbox (the list its `Publish` statements queue into) is declared `Seq of Text` and
/// is only ever read inside the dispatcher — Main never pushes to it, and the page's one publish
/// sends a literal. The same page `browser_tier_lock::a_publish_is_observed_in_chrome` drives.
const PUBLISHES: &str = r##"## Page /

Actions:
    To send:
        Publish "hello" on "room".
View:
    <button id="go" on-click={send}>send</button>
"##;

/// A page that reads a dropped file's fields into text it shows.
const DROPPED_FILE: &str = "\
## Page /

State:
    Let fileName be \"\".
    Let fileSize be 0.

Actions:
    When a file is dropped on \"#zone\" (file: File):
        Set fileName to file's name.
        Set fileSize to file's size.

View:
    <div id=\"zone\">drop here</div>
    <p id=\"name\">{fileName}</p>
    <p id=\"size\">{fileSize}</p>
";

#[test]
fn every_declared_kind_page_compiles_to_a_module_an_engine_loads() {
    let engine = wasmi::Engine::default();
    let mut failures = Vec::new();
    for (name, src) in [("publishes", PUBLISHES), ("dropped_file", DROPPED_FILE)] {
        match compile_to_wasm(src) {
            Ok(bytes) => {
                if let Err(e) = wasmi::Module::new(&engine, &bytes[..]) {
                    failures.push(format!("{name}: invalid module: {e}"));
                }
            }
            Err(e) => failures.push(format!("{name}: refused: {e:?}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
