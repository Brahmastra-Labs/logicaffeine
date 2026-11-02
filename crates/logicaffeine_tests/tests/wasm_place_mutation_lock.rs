//! ★ LOCK — every mutating statement × place shape runs on wasm, agreeing with the tree-walker. ★
//!
//! The VM lowers every mutation target — `Push`, `Add to`, `Remove from`, `Set item … of …` into a
//! local, a field, an item, a field of a field, an item of a field, a field of an item — through one
//! general place lowering (`Compiler::compile_place_mutation`): read each level out, clear the
//! parent's reference, mutate the leaf, write each level back. The write-back of a FIELD level is an
//! `Op::SetIndex` whose collection is a record and whose index is the field's NAME — so the wasm
//! backend, which types every register statically, has to know at each level what the extracted
//! value is (a `Seq of Int`, a `Set of Text`, a record of a known type) and which slot of the parent
//! a field name lands in. Both answers come from what the program DECLARES — the record's field
//! types — not from what the ops happen to do to the register afterwards.
//!
//! Every row runs on three tiers and compares whole outcomes (output AND failure) against the
//! tree-walker, the reference. A row the wasm backend refuses fails here as loudly as a row it
//! miscompiles: refusing an ordinary mutation is a gap, not a verdict.
//!
//! [`every_vm_place_mutation_row_has_a_wasm_row`] reads `vm_place_mutation_lock.rs`'s SOURCE, so a
//! shape the VM lock gains cannot go unexercised here.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::run_outcome;

const BOX_TAPE: &str = "## A Box has:\n    a tape, which is Seq of Int.\n\n";
const INNER_OUTER: &str =
    "## An Inner has:\n    a tape, which is Seq of Int.\n\n## An Outer has:\n    an inner, which is Inner.\n\n";

/// `(name, program)`. The first eleven names are `vm_place_mutation_lock`'s own tests, fixture for
/// fixture; the rest extend the matrix across element kinds, statements and roots the VM lock does
/// not enumerate.
fn rows() -> Vec<(&'static str, String)> {
    vec![
        (
            "set_item_of_a_records_field_mutates_the_field",
            format!(
                "{BOX_TAPE}## Main\nLet b be a new Box with tape (a new Seq of Int).\nPush 10 to b's tape.\n\
                 Push 20 to b's tape.\nSet item 2 of b's tape to 999.\nShow item 2 of b's tape.\n"
            ),
        ),
        (
            "set_item_of_a_field_leaves_sibling_fields_alone",
            "## A Pair has:\n    a first, which is Seq of Int.\n    a second, which is Seq of Int.\n\n## Main\n\
             Let p be a new Pair with first (a new Seq of Int) and second (a new Seq of Int).\n\
             Push 1 to p's first.\nPush 2 to p's first.\nPush 100 to p's second.\n\
             Set item 1 of p's first to 999.\nShow item 1 of p's first.\nShow item 1 of p's second.\n"
                .to_string(),
        ),
        (
            "push_to_a_records_field_still_mutates_the_field",
            format!(
                "{BOX_TAPE}## Main\nLet b be a new Box with tape (a new Seq of Int).\nPush 5 to b's tape.\n\
                 Push 6 to b's tape.\nShow item 2 of b's tape.\n"
            ),
        ),
        (
            "push_to_a_field_of_a_field_mutates_the_nested_field",
            format!(
                "{INNER_OUTER}## Main\n\
                 Let o be a new Outer with inner (a new Inner with tape (a new Seq of Int)).\n\
                 Push 7 to o's inner's tape.\nPush 8 to o's inner's tape.\nShow item 2 of o's inner's tape.\n"
            ),
        ),
        (
            "set_item_of_a_field_of_a_field_mutates_the_deepest_field",
            format!(
                "{INNER_OUTER}## Main\n\
                 Let o be a new Outer with inner (a new Inner with tape (a new Seq of Int)).\n\
                 Push 1 to o's inner's tape.\nPush 2 to o's inner's tape.\nPush 3 to o's inner's tape.\n\
                 Set item 2 of o's inner's tape to 999.\nShow item 1 of o's inner's tape.\n\
                 Show item 2 of o's inner's tape.\nShow item 3 of o's inner's tape.\n"
            ),
        ),
        (
            "set_item_of_an_item_mutates_the_nested_list_element",
            "## Main\nLet row0 be a new Seq of Int.\nPush 1 to row0.\nPush 2 to row0.\n\
             Let row1 be a new Seq of Int.\nPush 10 to row1.\nPush 20 to row1.\n\
             Let grid be a new Seq of (Seq of Int).\nPush row0 to grid.\nPush row1 to grid.\n\
             Set item 2 of (item 2 of grid) to 999.\nShow item 1 of (item 1 of grid).\n\
             Show item 2 of (item 2 of grid).\n"
                .to_string(),
        ),
        (
            "set_item_of_a_field_of_an_item_mutates_the_records_field",
            "## A Row has:\n    a tape, which is Seq of Int.\n\n## Main\n\
             Let r0 be a new Row with tape (a new Seq of Int).\nPush 1 to r0's tape.\n\
             Let rows be a new Seq of Row.\nPush r0 to rows.\nPush 2 to (item 1 of rows)'s tape.\n\
             Set item 2 of (item 1 of rows)'s tape to 999.\nShow item 1 of (item 1 of rows)'s tape.\n\
             Show item 2 of (item 1 of rows)'s tape.\n"
                .to_string(),
        ),
        (
            "add_to_a_records_set_field_mutates_the_field",
            "## A Bag has:\n    members, which is Set of Int.\n\n## Main\n\
             Let b be a new Bag with members (a new Set of Int).\nAdd 1 to b's members.\n\
             Add 2 to b's members.\nShow length of b's members.\n"
                .to_string(),
        ),
        (
            "remove_from_a_records_set_field_mutates_the_field",
            "## A Bag has:\n    members, which is Set of Int.\n\n## Main\n\
             Let b be a new Bag with members (a new Set of Int).\nAdd 1 to b's members.\n\
             Add 2 to b's members.\nRemove 1 from b's members.\nShow length of b's members.\n"
                .to_string(),
        ),
        (
            "repeated_writes_to_the_same_field_all_land",
            format!(
                "{BOX_TAPE}## Main\nLet b be a new Box with tape (a new Seq of Int).\nPush 0 to b's tape.\n\
                 Push 0 to b's tape.\nPush 0 to b's tape.\nSet item 1 of b's tape to 1.\n\
                 Set item 2 of b's tape to 2.\nSet item 3 of b's tape to 3.\nShow item 1 of b's tape.\n\
                 Show item 2 of b's tape.\nShow item 3 of b's tape.\n"
            ),
        ),
        (
            "a_single_level_field_push_compiles_to_list_push_field",
            format!("{BOX_TAPE}## Main\nLet b be a new Box with tape (a new Seq of Int).\nPush 5 to b's tape.\nShow length of b's tape.\n"),
        ),
        (
            "push_text_to_a_records_text_field",
            "## A Log has:\n    a lines, which is Seq of Text.\n\n## Main\n\
             Let l be a new Log with lines (a new Seq of Text).\nPush \"a\" to l's lines.\n\
             Push \"b\" to l's lines.\nShow item 2 of l's lines.\nShow length of l's lines.\n"
                .to_string(),
        ),
        (
            "set_item_of_a_records_text_field",
            "## A Log has:\n    a lines, which is Seq of Text.\n\n## Main\n\
             Let l be a new Log with lines (a new Seq of Text).\nPush \"a\" to l's lines.\n\
             Push \"b\" to l's lines.\nSet item 1 of l's lines to \"z\".\nShow item 1 of l's lines.\n\
             Show item 2 of l's lines.\n"
                .to_string(),
        ),
        (
            "push_text_to_a_field_of_a_field",
            "## A Page has:\n    a lines, which is Seq of Text.\n\n## A Book has:\n    a page, which is Page.\n\n\
             ## Main\nLet b be a new Book with page (a new Page with lines (a new Seq of Text)).\n\
             Push \"x\" to b's page's lines.\nPush \"y\" to b's page's lines.\nShow item 2 of b's page's lines.\n"
                .to_string(),
        ),
        (
            "push_a_record_to_a_records_list_field",
            "## A Cell has:\n    a n, which is Int.\n\n## A Board has:\n    a cells, which is Seq of Cell.\n\n\
             ## Main\nLet b be a new Board with cells (a new Seq of Cell).\nPush a new Cell with n 4 to b's cells.\n\
             Push a new Cell with n 9 to b's cells.\nShow (item 2 of b's cells)'s n.\nShow length of b's cells.\n"
                .to_string(),
        ),
        (
            "set_item_of_a_records_map_field",
            "## A Tally has:\n    a counts, which is Map of Text to Int.\n\n## Main\n\
             Let t be a new Tally with counts (a new Map of Text to Int).\nSet item \"a\" of t's counts to 1.\n\
             Set item \"b\" of t's counts to 2.\nSet item \"a\" of t's counts to 7.\nShow item \"a\" of t's counts.\n\
             Show item \"b\" of t's counts.\nShow length of t's counts.\n"
                .to_string(),
        ),
        (
            "add_text_to_a_records_text_set_field",
            "## A Tags has:\n    names, which is Set of Text.\n\n## Main\n\
             Let t be a new Tags with names (a new Set of Text).\nAdd \"x\" to t's names.\nAdd \"y\" to t's names.\n\
             Add \"x\" to t's names.\nShow length of t's names.\nShow t's names contains \"y\".\n"
                .to_string(),
        ),
        (
            "push_to_a_parameters_field_and_return_it",
            format!(
                "{BOX_TAPE}## To withTape(b: Box, v: Int) -> Box:\n    Push v to b's tape.\n    Return b.\n\n\
                 ## Main\nLet b be a new Box with tape (a new Seq of Int).\nLet c be withTape(b, 3).\n\
                 Let d be withTape(c, 4).\nShow length of b's tape.\nShow item 2 of d's tape.\n"
            ),
        ),
        (
            "a_field_mutation_does_not_reach_an_earlier_copy",
            format!(
                "{BOX_TAPE}## Main\nLet b be a new Box with tape (a new Seq of Int).\nPush 1 to b's tape.\n\
                 Let c be b.\nPush 2 to b's tape.\nShow length of c's tape.\nShow length of b's tape.\n"
            ),
        ),
        (
            "a_records_field_mutated_inside_a_loop",
            format!(
                "{BOX_TAPE}## Main\nLet b be a new Box with tape (a new Seq of Int).\n\
                 Repeat for i from 1 to 5:\n    Push i * i to b's tape.\n\
                 Set item 3 of b's tape to 0.\nLet total be 0.\nRepeat for x in b's tape:\n    Set total to total + x.\n\
                 Show total.\n"
            ),
        ),
        (
            // A `## Store` action's shape: read a global record's list of records into a local, push
            // onto the local, write the local back as the field.
            "a_global_records_list_of_records_field_written_back_from_a_local",
            "## A Toast has:\n    a number, which is Int.\n\n## A Shelf has:\n    a toasts, which is Seq of Toast.\n    a next, which is Int.\n\n\
             ## To post():\n    Let toasts be shelf's toasts.\n    Push a new Toast with number (shelf's next) to toasts.\n    \
             Set shelf's toasts to toasts.\n    Set shelf's next to shelf's next + 1.\n\n\
             ## Main\nLet shelf be a new Shelf with toasts (a new Seq of Toast) and next 1.\npost().\npost().\n\
             Show length of shelf's toasts.\nShow (item 2 of shelf's toasts)'s number.\n"
                .to_string(),
        ),
    ]
}

/// One tier's whole outcome: what it showed, and the failure it stopped with.
type Outcome = (String, Option<String>);

/// The three tiers' outcomes, in order tree-walker, VM, wasm. A program the wasm backend refuses or
/// cannot load answers that reason as its failure, so a refusal compares unequal like any wrong answer.
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

fn describe(outcomes: &[Outcome; 3]) -> String {
    ["tree-walker", "VM", "wasm"]
        .iter()
        .zip(outcomes)
        .map(|(tier, (out, err))| format!("  {tier}: {out:?} {err:?}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_place_mutation_row_runs_identically_on_wasm() {
    let rows = rows();
    let mut failures = Vec::new();
    for (name, src) in &rows {
        let outcomes = three_tiers(src);
        let [tw, vm, wasm] = &outcomes;
        if tw.1.is_some() || vm != tw || wasm != tw {
            failures.push(format!("── {name}\n{}\n{src}", describe(&outcomes)));
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

/// Mutations whose answer is fixed by VALUE SEMANTICS, written out because the tiers do not agree on
/// them and none can be the oracle for the others. A record, a list or a field bound to a second name
/// is a copy: writing through one name never shows through the other. A PLAIN (non-`mutable`)
/// parameter is a copy too (`e2e_ref_semantics`); `mutable` is the one way to reach the caller's value.
fn value_semantics_rows() -> Vec<(&'static str, String, &'static str)> {
    vec![
        (
            "a_callees_field_item_write_to_a_plain_parameter",
            format!(
                "{BOX_TAPE}## To zeroFirst(b: Box) -> Box:\n    Set item 1 of b's tape to 0.\n    Return b.\n\n\
                 ## Main\nLet b be a new Box with tape (a new Seq of Int).\nPush 5 to b's tape.\n\
                 Push 6 to b's tape.\nLet c be zeroFirst(b).\nShow item 1 of b's tape.\nShow item 1 of c's tape.\n"
            ),
            "5\n0",
        ),
        (
            "a_callees_nested_field_push_to_a_plain_parameter",
            format!(
                "{INNER_OUTER}## To grow(o: Outer) -> Outer:\n    Push 1 to o's inner's tape.\n    Return o.\n\n\
                 ## Main\nLet o be a new Outer with inner (a new Inner with tape (a new Seq of Int)).\n\
                 Let p be grow(grow(o)).\nShow length of p's inner's tape.\nShow length of o's inner's tape.\n"
            ),
            "2\n0",
        ),
        (
            "a_field_push_and_item_write_after_a_record_copy",
            format!(
                "{BOX_TAPE}## Main\nLet b be a new Box with tape (a new Seq of Int).\nPush 1 to b's tape.\n\
                 Let c be b.\nPush 2 to b's tape.\nSet item 1 of b's tape to 7.\nShow length of c's tape.\n\
                 Show item 1 of c's tape.\nShow item 1 of b's tape.\n"
            ),
            "1\n1\n7",
        ),
        (
            "a_nested_field_write_after_a_record_copy",
            format!(
                "{INNER_OUTER}## Main\n\
                 Let o be a new Outer with inner (a new Inner with tape (a new Seq of Int)).\n\
                 Push 1 to o's inner's tape.\nLet p be o.\nPush 2 to o's inner's tape.\n\
                 Set item 1 of o's inner's tape to 9.\nShow length of p's inner's tape.\n\
                 Show item 1 of p's inner's tape.\nShow item 1 of o's inner's tape.\n"
            ),
            "1\n1\n9",
        ),
        (
            "a_field_bound_to_a_name_then_the_record_mutated",
            format!(
                "{BOX_TAPE}## Main\nLet b be a new Box with tape (a new Seq of Int).\nPush 1 to b's tape.\n\
                 Let t be b's tape.\nPush 2 to b's tape.\nSet item 1 of b's tape to 5.\nShow length of t.\n\
                 Show item 1 of t.\nPush 3 to t.\nShow length of b's tape.\n"
            ),
            "1\n1\n2",
        ),
        (
            "a_record_pushed_into_a_list_then_mutated",
            format!(
                "{BOX_TAPE}## Main\nLet b be a new Box with tape (a new Seq of Int).\nPush 1 to b's tape.\n\
                 Let boxes be a new Seq of Box.\nPush b to boxes.\nPush 2 to b's tape.\n\
                 Set item 1 of b's tape to 8.\nShow length of (item 1 of boxes)'s tape.\n\
                 Show item 1 of (item 1 of boxes)'s tape.\nShow item 1 of b's tape.\n"
            ),
            "1\n1\n8",
        ),
        (
            "a_list_row_mutated_after_the_row_was_bound_to_a_name",
            "## A Row has:\n    a tape, which is Seq of Int.\n\n## Main\n\
             Let rows be a new Seq of Row.\nPush a new Row with tape (a new Seq of Int) to rows.\n\
             Let r be item 1 of rows.\nPush 4 to (item 1 of rows)'s tape.\nShow length of r's tape.\n\
             Show length of (item 1 of rows)'s tape.\n"
                .to_string(),
            "0\n1",
        ),
    ]
}

/// The wasm tier gives every value-semantics row its written answer.
#[test]
fn every_value_semantics_row_holds_on_wasm() {
    let mut wrong = Vec::new();
    for (name, src, expected) in value_semantics_rows() {
        let outcomes = three_tiers(&src);
        if outcomes[2] != (expected.to_string(), None) {
            wrong.push(format!(
                "── {name} (want {expected:?})\n{}\n{src}",
                describe(&outcomes)
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "wasm broke value semantics:\n{}",
        wrong.join("\n")
    );
}

/// Every tier gives every value-semantics row its written answer.
#[test]
fn every_value_semantics_row_holds_on_every_tier() {
    let mut wrong = Vec::new();
    for (name, src, expected) in value_semantics_rows() {
        let outcomes = three_tiers(&src);
        if outcomes.iter().any(|o| *o != (expected.to_string(), None)) {
            wrong.push(format!(
                "── {name} (want {expected:?})\n{}\n{src}",
                describe(&outcomes)
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "a tier broke value semantics:\n{}",
        wrong.join("\n")
    );
}

/// Every `#[test] fn` in `vm_place_mutation_lock.rs` names a row here, so a place shape the VM lock
/// gains is run on wasm too — read from that file's SOURCE, not from a list someone keeps in step.
#[test]
fn every_vm_place_mutation_row_has_a_wasm_row() {
    let vm_lock = include_str!("vm_place_mutation_lock.rs");
    let mut vm_names = Vec::new();
    let mut lines = vm_lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() == "#[test]" {
            let decl = lines.next().unwrap_or_default().trim();
            let name = decl
                .strip_prefix("fn ")
                .and_then(|d| d.split('(').next())
                .unwrap_or_else(|| panic!("a #[test] not followed by `fn name(`: {decl}"));
            vm_names.push(name.to_string());
        }
    }
    assert!(
        !vm_names.is_empty(),
        "no tests read out of vm_place_mutation_lock.rs — the reader is broken"
    );
    let ours: Vec<&str> = rows().iter().map(|(n, _)| *n).collect();
    let missing: Vec<&String> = vm_names
        .iter()
        .filter(|n| !ours.contains(&n.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "vm_place_mutation_lock rows with no wasm row here: {missing:?}"
    );
}

/// How far the heap's bump pointer moved running `src` on wasm.
fn heap_after(src: &str) -> u32 {
    let run =
        run_outcome(src).unwrap_or_else(|why| panic!("the program runs on wasm: {why}\n{src}"));
    assert_eq!(run.failure, None, "{src}");
    run.heap_ptr
        .expect("a program that builds a list has a heap")
}

/// ★ A MODULE-LEVEL COLLECTION WRITTEN FROM A FUNCTION COSTS MEMORY LINEAR IN WHAT IS WRITTEN. The write
/// reads the global into a register, mutates it and stores it back; the global's own hold moves to the
/// register and back, so the mutation copies nothing. Counting the global as a second holder copied the
/// whole collection on every write — quadratic in bytes, and a page's registries grow on every turn.
/// Measured by the heap's bump pointer, not by time: 8x the writes may move it about 8x, never 64x.
#[test]
fn a_global_collection_written_from_a_function_allocates_memory_linear_in_its_writes() {
    let shapes: [(&str, fn(usize) -> String); 4] = [
        ("an author's global list pushed", |n| {
            format!("## To fill(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Push i to items.\n        Set i to i + 1.\n\n## Main\nLet mutable items be [0].\nfill({n}).\nShow length of items.\n")
        }),
        ("a compiler-written global list pushed", |n| {
            format!("## To fill(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Push i to __T.\n        Set i to i + 1.\n\n## Main\nLet mutable __T be [0].\nfill({n}).\nShow length of __T.\n")
        }),
        ("an author's global list read, then written", |n| {
            format!(
                "## To fill(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Push i to items.\n        Set i to i + 1.\n\n\
                 ## To poke(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Let v be item i of items.\n        \
                 Set item i of items to v + 1.\n        Set i to i + 1.\n\n## Main\nLet mutable items be [0].\nfill({n}).\npoke({n}).\nShow item {n} of items.\n"
            )
        }),
        ("an author's global record's list field pushed", |n| {
            format!(
                "{BOX_TAPE}## To fill(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Push i to (item 1 of boxes)'s tape.\n        \
                 Set i to i + 1.\n\n## Main\nLet mutable boxes be a new Seq of Box.\nPush a new Box with tape (a new Seq of Int) to boxes.\n\
                 fill({n}).\nShow length of (item 1 of boxes)'s tape.\n"
            )
        }),
    ];
    let (small, large) = (1_000, 8_000);
    let mut grew = Vec::new();
    for (what, src) in shapes {
        let (a, b) = (heap_after(&src(small)), heap_after(&src(large)));
        let growth = f64::from(b) / f64::from(a);
        if growth > 1.5 * (large / small) as f64 {
            grew.push(format!(
                "{what}: {a} -> {b} bytes ({growth:.1}x for 8x the writes)"
            ));
        }
    }
    assert!(
        grew.is_empty(),
        "a write copies the whole collection again:\n{}",
        grew.join("\n")
    );
}
