//! LOCK — removing and keeping list items by a condition, without a hand-written rebuild loop (S18).
//!
//! `Remove every <x> from <list> where <condition>.` takes every item the condition holds for out
//! of the list, in place; the rest keep their order. `every <x> in <list> where <condition>` — as the
//! value of a `Let` or a `Set` — is a new list of the items the condition holds for, in order, and
//! leaves `<list>` as it was. `<x>` names each item in the condition, which may read anything else
//! in scope. Every tier runs the same statements: the tree-walker, the VM, the lowered page, and the
//! browser build.

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};
use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TierRun};
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

/// `src`'s output on the tree-walker, held equal to the VM's, and its browser build.
fn shown_on_every_tier(src: &str) -> Vec<String> {
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker runs it:\n{src}");
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM runs it:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        vm.output.trim_end(),
        "the tiers agree:\n{src}"
    );
    let built = compile_to_wasm(src);
    assert!(
        built.is_ok(),
        "it builds for the browser: {:?}",
        built.err()
    );
    tw.output.lines().map(str::to_string).collect()
}

/// Every `## Test` in the page `source` passes on the reference, runs step for step the same on
/// the lowered page on the tree-walker and on the VM, and the page builds for the browser.
fn a_page_passes_on_every_tier(source: &str) {
    let report = run_tests(source, &TestConfig::default())
        .unwrap_or_else(|e| panic!("the page parses: {e}"));
    assert!(
        !report.outcomes.is_empty(),
        "the page has its tests:\n{source}"
    );
    for outcome in &report.outcomes {
        assert_eq!(
            outcome.verdict,
            Verdict::Passed,
            "`{}`: {:?}",
            outcome.name,
            outcome.failures
        );
    }
    let compared = compare_tests(
        source,
        &[PageTierKind::LoweredHere, PageTierKind::LoweredVm],
        0,
        50_000_000,
        None,
    )
    .unwrap_or_else(|e| panic!("the page parses: {e}"));
    assert_eq!(
        compared.len(),
        report.outcomes.len(),
        "every test works the page"
    );
    for test in &compared {
        for (kind, run) in &test.candidates {
            assert!(
                matches!(run, TierRun::Traced(_)),
                "`{}` runs lowered on the {}: {run:?}",
                test.test,
                kind.label()
            );
            let differs = test.divergence(*kind).expect("a traced run");
            assert!(
                differs.is_empty(),
                "`{}` on the {} runs as the reference does: {differs:?}",
                test.test,
                kind.label()
            );
        }
    }
    let built = compile_to_wasm(source);
    assert!(
        built.is_ok(),
        "the page builds for the browser: {:?}",
        built.err()
    );
}

#[test]
fn remove_every_takes_out_what_the_condition_holds_for_and_keeps_the_order() {
    let out = shown_on_every_tier(
        "## Main\nLet mutable xs be [5, 1, 4, 2, 3].\nRemove every n from xs where n is greater than 2.\nShow xs.\n",
    );
    assert_eq!(out, ["[1, 2]"]);
}

#[test]
fn remove_every_removes_every_equal_item() {
    let out = shown_on_every_tier(
        "## Main\nLet mutable xs be [3, 1, 3, 2, 3].\nRemove every n from xs where n equals 3.\nShow xs.\n",
    );
    assert_eq!(out, ["[1, 2]"]);
}

#[test]
fn the_condition_reads_what_is_in_scope() {
    let out = shown_on_every_tier(
        "## Main\nLet limit be 3.\nLet mutable xs be [1, 2, 3, 4].\nRemove every n from xs where n is at least limit.\nShow xs.\n",
    );
    assert_eq!(out, ["[1, 2]"]);
}

#[test]
fn removing_nothing_and_removing_everything_are_both_answers() {
    let out = shown_on_every_tier(
        "## Main\nLet mutable xs be [1, 2].\nRemove every n from xs where n is greater than 9.\nShow xs.\n\
         Remove every n from xs where n is greater than 0.\nShow length of xs.\n",
    );
    assert_eq!(out, ["[1, 2]", "0"]);
}

#[test]
fn every_in_where_is_a_new_list_and_leaves_the_old_one_alone() {
    let out = shown_on_every_tier(
        "## Main\nLet xs be [5, 1, 4, 2, 3].\nLet small be every n in xs where n is less than 4.\nShow small.\nShow xs.\n",
    );
    assert_eq!(out, ["[1, 2, 3]", "[5, 1, 4, 2, 3]"]);
}

#[test]
fn set_takes_every_in_where_too() {
    let out = shown_on_every_tier(
        "## Main\nLet xs be [\"a\", \"bb\", \"ccc\"].\nLet mutable long be a new Seq of Text.\n\
         Set long to every w in xs where length of w is greater than 1.\nShow long.\n",
    );
    assert_eq!(out, ["[bb, ccc]"]);
}

/// The name `<x>` belongs to the condition: a name already in scope keeps its value after the
/// statement, and the name is not defined after it.
#[test]
fn the_items_name_neither_shadows_nor_outlives_the_statement() {
    let out = shown_on_every_tier(
        "## Main\nLet n be 7.\nLet mutable xs be [1, 9].\nRemove every n from xs where n is greater than 5.\n\
         Show n.\nShow xs.\nLet ys be every n in xs where n equals 1.\nShow n.\n",
    );
    assert_eq!(out, ["7", "[1]", "7"]);
    for run in [tw_outcome, vm_outcome] {
        dom::reset();
        let out = run("## Main\nLet mutable xs be [1, 9].\nRemove every q from xs where q is greater than 5.\nShow q.\n");
        assert!(
            out.error.is_some(),
            "`q` is not defined after the statement: {:?}",
            out.output
        );
    }
}

/// A list of records, filtered on a field and kept in a new list, on every tier.
#[test]
fn a_seq_of_records_is_filtered_on_a_field() {
    let out = shown_on_every_tier(
        "## A Todo has:\n    a title, which is Text.\n    a done, which is Bool.\n\n\
         ## Main\nLet todos be [a new Todo with title \"milk\" and done false, a new Todo with title \"tea\" and done true].\n\
         Let open be every t in todos where not t's done.\nShow length of open.\nShow (item 1 of open)'s title.\n\
         Let mutable all be todos.\nRemove every t from all where t's done.\nShow length of all.\n",
    );
    assert_eq!(out, ["1", "milk", "1"]);
}

/// A store's own action removes from its own field, through the write-back a `Set` of the field
/// makes.
const STORE: &str = r##"## Store Cart
State:
    Let items: Seq of Int be [1, 2, 3, 4].
Actions:
    To dropBig:
        Remove every i from items where i is greater than 2.

## Page /

Use Cart.

Actions:
    To go:
        Call Cart's dropBig.

View:
    <p id="n">{length of Cart's items}</p>
    <button id="b" on-click={go}>go</button>

## Test: a store drops what is big
    Expect that the text of "#n" is equal to "4".
    Click "#b".
    Expect that the text of "#n" is equal to "2".
"##;

#[test]
fn a_store_action_removes_from_its_own_field() {
    a_page_passes_on_every_tier(STORE);
}

#[path = "common/mod.rs"]
mod common;

/// The same statements as compiled Rust: the desugar is ordinary LOGOS, so the AOT build runs it.
#[test]
fn e2e_remove_every_and_every_in_where_run_as_compiled_rust() {
    common::assert_output_lines(
        "## Main\nLet mutable xs be [5, 1, 4, 2, 3].\nRemove every n from xs where n is greater than 2.\nShow xs.\n\
         Let ys be every n in [3, 1, 3] where n equals 3.\nShow ys.\n",
        &["[1, 2]", "[3, 3]"],
    );
}

/// The item's name is the condition's own even where a binding of another type already has it:
/// after the statement the Text binding is Text again, on every tier and as compiled Rust.
#[test]
fn the_items_name_leaves_a_binding_of_another_type_alone() {
    let src = "## Main\nLet n be \"seven\".\nLet mutable xs be [1, 9].\nRemove every n from xs where n is greater than 5.\n\
               Let ys be every n in xs where n is less than 5.\nShow n + \"!\".\nShow xs.\nShow ys.\n";
    let out = shown_on_every_tier(src);
    assert_eq!(out, ["seven!", "[1]", "[1]"]);
    common::assert_output_lines(src, &["seven!", "[1]", "[1]"]);
}

/// The condition speaks of each item by the name the author gave it: what the condition says about
/// `t` — here its self-describing interpolation `{t=}` — says `t`, never a name the desugar made up.
#[test]
fn the_condition_names_each_item_as_the_author_wrote_it() {
    let src = "## To noted (s: Text) -> Bool:\n    Show s.\n    Return false.\n\n\
               ## Main\nLet mutable xs be [1, 2].\nRemove every t from xs where noted(\"{t=}\").\n\
               Let kept be every k in xs where noted(\"{k=}\").\nShow xs.\nShow length of kept.\n";
    let out = shown_on_every_tier(src);
    assert_eq!(out, ["t=1", "t=2", "k=1", "k=2", "[1, 2]", "0"]);
    common::assert_output_lines(src, &["t=1", "t=2", "k=1", "k=2", "[1, 2]", "0"]);
}

/// The page shape S18 exists for: a todo list clearing what is done.
const TODOS: &str = r##"## A Todo has:
    a title, which is Text.
    a done, which is Bool.

## Page /

State:
    Let todos be [a new Todo with title "milk" and done false, a new Todo with title "eggs" and done true, a new Todo with title "tea" and done true].

Actions:
    To clear:
        Remove every t from todos where t's done.

View:
    <ul>{for each t in todos:}<li>{t's title}</li>{end}</ul>
    <p id="left">{length of todos}</p>
    <button id="clear" on-click={clear}>clear</button>

## Test: clearing what is done
    Expect that the text of "#left" is equal to "3".
    Click "#clear".
    Expect that the text of "#left" is equal to "1".
    Expect that the text of "li" is equal to "milk".
"##;

#[test]
fn a_page_clears_what_is_done_on_every_tier() {
    a_page_passes_on_every_tier(TODOS);
}
