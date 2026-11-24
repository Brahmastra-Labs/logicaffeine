//! ★ LOCK — Int arithmetic on the AOT-Rust tier never wraps, whatever bound the operand. ★
//!
//! A LOGOS `Int` is unbounded. The Rust tier holds most integers as machine words and keeps their arithmetic exact or
//! loud: an operation the range oracle cannot prove stays in the word is the checked or exact helper, which promotes
//! past the word or stops with the overflow sentence. A raw `*` or `+` on an `i64` the oracle knows nothing of wraps
//! in a release build and prints a wrong number with no sign of it.
//!
//! Each row binds an `Int` one way the language binds one (a parameter, a record's field read from a local or from a
//! parameter, a variant's field, a `Pop`, a loop over a list parameter, a local list, a slice or a set, a map's key or
//! value in a loop over it, an item read, a loop that accumulates items, a closure's parameter, a call's result, a map
//! lookup, a tuple's item, an item of a list of lists, a field of a list's record, of a call's record and of a record a
//! loop binds, an item of a record's list, a generic function's result, a `Maybe`'s payload) to a value the program reads from its command line, 2^62, so no range proof
//! reaches it. Seven operations then push it
//! past the word, in a `Show` and stored in a word: `x * 3`, `x + x + x`, `0 - x - x - x`, each stored in a binding,
//! an accumulation, a product into a binding the value started. Built debug and built release, the Rust tier prints
//! what the tree-walker prints, or stops with the overflow sentence after printing a prefix of it; never a wrapped
//! number, never Rust's own overflow panic.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test rust_word_arithmetic_never_wraps_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::time::Duration;

use common::RustProfile;
use logicaffeine_compile::compile::{compile_program_full, tw_lines_with_args};

/// 2^62: three of it, or minus three of it, is past the word.
const PAST_HALF: &str = "4611686018427387904";

/// How many operations each row runs, one per run, picked by the program's second argument.
const OPERATIONS: usize = 7;

/// The operations on `x` (an expression of the bound `Int`), each under `If s equals k:`, at `indent`.
fn operations(x: &str, indent: &str) -> String {
    let rows = [
        format!("Show {x} * 3."),
        format!("Show {x} + {x} + {x}."),
        format!("Show 0 - {x} - {x} - {x}."),
        format!("Let w be {x} * 3.\n{indent}    Show w."),
        format!("Let w be {x} + {x} + {x}.\n{indent}    Show w."),
        format!("Let mutable t be 0.\n{indent}    Set t to t + {x} * 3.\n{indent}    Show t."),
        format!("Let mutable t be {x}.\n{indent}    Set t to t * 3.\n{indent}    Show t."),
    ];
    rows.iter()
        .enumerate()
        .map(|(k, row)| format!("{indent}If s equals {}:\n{indent}    {row}\n", k + 1))
        .collect()
}

/// A program reading `n` and `s` from its command line whose `probe` binds an `Int` the way `body` does and runs
/// operation `s` on it; `decls` declares what the binding needs.
fn program(decls: &str, body: &str) -> String {
    format!(
        "## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

{decls}
## To probe (n: Int) and (s: Int) -> Int:
{body}    Return 0.

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let s be parseInt(item 3 of arguments).
Let done be probe(n, s).
"
    )
}

/// Every operation of `src`, built debug and built release, prints what the tree-walker prints or stops with the
/// overflow sentence after a prefix of it.
fn never_wraps(form: &str, src: &str) {
    let compiled =
        compile_program_full(src).unwrap_or_else(|e| panic!("the Rust generator refused {form} ({e:?}):\n{src}"));
    let reference: Vec<String> = (1..=OPERATIONS)
        .map(|s| {
            let argv = ["probe".to_string(), PAST_HALF.to_string(), s.to_string()];
            let tw = tw_lines_with_args(src, &argv);
            assert_eq!(tw.error, None, "the tree-walker refused {form}, operation {s}:\n{src}");
            assert!(!tw.lines.is_empty(), "{form}, operation {s}, prints its value on the tree-walker:\n{src}");
            tw.lines.iter().map(|line| format!("{line}\n")).collect()
        })
        .collect();
    for profile in [RustProfile::Debug, RustProfile::Release] {
        let built = common::build_generated(&compiled.rust_code, &compiled.dependencies, profile)
            .unwrap_or_else(|why| panic!("{form} ({profile:?}) does not build:\n{why}\n{src}\n{}", compiled.rust_code));
        for (k, expected) in reference.iter().enumerate() {
            let s = (k + 1).to_string();
            let run = common::run_within(&built.binary, &[PAST_HALF, &s], Duration::from_secs(60));
            assert!(!run.timed_out, "{form}, operation {s} ({profile:?}), still runs after 60 s");
            if run.success {
                assert_eq!(
                    &run.stdout, expected,
                    "{form}, operation {s} ({profile:?}), printed another number:\n{src}\n{}",
                    compiled.rust_code
                );
            } else {
                assert!(
                    run.stderr.contains("Integer overflow: ") && run.stderr.contains("does not fit a 64-bit Int"),
                    "{form}, operation {s} ({profile:?}), failed some other way ({}):\n{src}\n{}",
                    run.stderr,
                    compiled.rust_code
                );
                assert!(
                    expected.starts_with(&run.stdout),
                    "{form}, operation {s} ({profile:?}), printed other bytes before it stopped:\n{}",
                    run.stdout
                );
            }
        }
    }
}

#[test]
fn a_parameter_never_wraps() {
    let decls = format!("## To op (x: Int) and (s: Int) -> Int:\n{}    Return 0.\n", operations("x", "    "));
    never_wraps("a parameter", &program(&decls, "    Let r be op(n, s).\n"));
}

#[test]
fn a_records_field_read_from_a_local_never_wraps() {
    let decls = "## A Cell has:\n    a v, which is Int.\n";
    let body = format!("    Let c be a new Cell with v n.\n{}", operations("c's v", "    "));
    never_wraps("a record's field read from a local", &program(decls, &body));
}

#[test]
fn a_records_field_read_from_a_parameter_never_wraps() {
    let decls = format!(
        "## A Cell has:\n    a v, which is Int.\n\n## To op (c: Cell) and (s: Int) -> Int:\n{}    Return 0.\n",
        operations("c's v", "    ")
    );
    never_wraps("a record's field read from a parameter", &program(&decls, "    Let r be op(a new Cell with v n, s).\n"));
}

#[test]
fn a_variants_field_never_wraps() {
    let decls = "## A Shape is one of:\n    A Box with side Int.\n    A Dot.\n";
    let body = format!(
        "    Let sh be a new Box with side n.\n    Inspect sh:\n        When Box (x):\n{}        Otherwise:\n            Show \"none\".\n",
        operations("x", "            ")
    );
    never_wraps("a variant's field", &program(decls, &body));
}

#[test]
fn a_popped_item_never_wraps() {
    let body = format!(
        "    Let mutable ys be a new Seq of Int.\n    Push n to ys.\n    Pop from ys into x.\n{}",
        operations("x", "    ")
    );
    never_wraps("a popped item", &program("", &body));
}

#[test]
fn a_loop_over_a_list_parameter_never_wraps() {
    let decls = format!(
        "## To op (xs: Seq of Int) and (s: Int) -> Int:\n    Repeat for x in xs:\n{}    Return 0.\n",
        operations("x", "        ")
    );
    let body = "    Let mutable ys be a new Seq of Int.\n    Push n to ys.\n    Let r be op(ys, s).\n";
    never_wraps("a loop over a list parameter", &program(&decls, body));
}

#[test]
fn a_loop_over_a_local_list_never_wraps() {
    let body = format!(
        "    Let mutable ys be a new Seq of Int.\n    Push n to ys.\n    Repeat for x in ys:\n{}",
        operations("x", "        ")
    );
    never_wraps("a loop over a local list", &program("", &body));
}

#[test]
fn a_maps_value_in_a_loop_over_it_never_wraps() {
    let body = format!(
        "    Let mutable ms be a new Map of Int to Int.\n    Set item 1 of ms to n.\n    Repeat for (k, x) in ms:\n{}",
        operations("x", "        ")
    );
    never_wraps("a map's value in a loop over it", &program("", &body));
}

#[test]
fn a_maps_key_in_a_loop_over_it_never_wraps() {
    let body = format!(
        "    Let mutable ms be a new Map of Int to Int.\n    Set item n of ms to 1.\n    Repeat for (x, v) in ms:\n{}",
        operations("x", "        ")
    );
    never_wraps("a map's key in a loop over it", &program("", &body));
}

#[test]
fn an_item_read_never_wraps() {
    let body = format!(
        "    Let mutable ys be a new Seq of Int.\n    Push n to ys.\n{}",
        operations("(item 1 of ys)", "    ")
    );
    never_wraps("an item read", &program("", &body));
}

#[test]
fn an_item_read_from_a_list_parameter_in_a_counted_loop_never_wraps() {
    let decls = format!(
        "## To op (xs: Seq of Int) and (s: Int) -> Int:\n    Let mutable i be 1.\n    While i is at most length of xs:\n{}        Set i to i + 1.\n    Return 0.\n",
        operations("(item i of xs)", "        ")
    );
    let body = "    Let mutable ys be a new Seq of Int.\n    Push n to ys.\n    Let r be op(ys, s).\n";
    never_wraps("an item read from a list parameter in a counted loop", &program(&decls, body));
}

#[test]
fn a_closures_parameter_never_wraps() {
    let body = "    Let triple be (x: Int) -> x * 3.\n    Let sum3 be (x: Int) -> x + x + x.\n    Let neg3 be (x: Int) -> 0 - x - x - x.\n\
                \x20   If s equals 1:\n        Show triple(n).\n    If s equals 2:\n        Show sum3(n).\n    If s equals 3:\n        Show neg3(n).\n\
                \x20   If s is greater than 3:\n        Let w be triple(n).\n        Show w.\n";
    never_wraps("a closure's parameter", &program("", body));
}

#[test]
fn a_calls_result_never_wraps() {
    let decls = "## To same (x: Int) -> Int:\n    Return x.\n";
    let body = format!("    Let x be same(n).\n{}", operations("x", "    "));
    never_wraps("a call's result", &program(decls, &body));
}

#[test]
fn a_map_lookup_never_wraps() {
    let body = format!(
        "    Let mutable ms be a new Map of Int to Int.\n    Set item 1 of ms to n.\n{}",
        operations("(item 1 of ms)", "    ")
    );
    never_wraps("a map lookup", &program("", &body));
}

#[test]
fn an_accumulated_item_read_in_a_counted_loop_never_wraps() {
    let decls = "## To op (xs: Seq of Int) -> Int:\n    Let mutable i be 1.\n    Let mutable t be 0.\n    While i is at most length of xs:\n        Set t to t + (item i of xs) * 3.\n        Set i to i + 1.\n    Return t.\n";
    let body = "    Let mutable ys be a new Seq of Int.\n    Push n to ys.\n    Show op(ys).\n";
    never_wraps("an accumulated item read in a counted loop", &program(decls, body));
}

#[test]
fn a_tuples_item_never_wraps() {
    let body = format!("    Let pr be (n, 1).\n{}", operations("(item 1 of pr)", "    "));
    never_wraps("a tuple's item", &program("", &body));
}

#[test]
fn a_loop_over_a_slice_never_wraps() {
    let body = format!(
        "    Let mutable ys be a new Seq of Int.\n    Push n to ys.\n    Push 1 to ys.\n    Repeat for x in items 1 through 1 of ys:\n{}",
        operations("x", "        ")
    );
    never_wraps("a loop over a slice", &program("", &body));
}

#[test]
fn a_loop_over_a_set_never_wraps() {
    let body = format!(
        "    Let mutable st be a new Set of Int.\n    Add n to st.\n    Repeat for x in st:\n{}",
        operations("x", "        ")
    );
    never_wraps("a loop over a set", &program("", &body));
}

#[test]
fn an_item_of_a_list_of_lists_never_wraps() {
    let body = format!(
        "    Let mutable ys be a new Seq of Int.\n    Push n to ys.\n    Let mutable grid be a new Seq of (Seq of Int).\n    Push ys to grid.\n{}",
        operations("(item 1 of (item 1 of grid))", "    ")
    );
    never_wraps("an item of a list of lists", &program("", &body));
}

#[test]
fn a_field_of_a_lists_record_never_wraps() {
    let decls = "## A Cell has:\n    a v, which is Int.\n";
    let body = format!(
        "    Let mutable cells be a new Seq of Cell.\n    Push a new Cell with v n to cells.\n{}",
        operations("(item 1 of cells)'s v", "    ")
    );
    never_wraps("a field of a list's record", &program(decls, &body));
}

#[test]
fn a_field_of_a_calls_record_never_wraps() {
    let decls = "## A Cell has:\n    a v, which is Int.\n\n## To mk (x: Int) -> Cell:\n    Return a new Cell with v x.\n";
    let body = operations("mk(n)'s v", "    ");
    never_wraps("a field of a call's record", &program(decls, &body));
}

#[test]
fn an_item_of_a_records_list_never_wraps() {
    let decls = "## A Bag has:\n    An items: Seq of Int.\n";
    let body = format!(
        "    Let mutable ys be a new Seq of Int.\n    Push n to ys.\n    Let b be a new Bag with items ys.\n{}",
        operations("(item 1 of b's items)", "    ")
    );
    never_wraps("an item of a record's list", &program(decls, &body));
}

#[test]
fn a_field_of_a_record_a_loop_binds_never_wraps() {
    let decls = "## A Cell has:\n    a v, which is Int.\n";
    let body = format!(
        "    Let mutable cells be a new Seq of Cell.\n    Push a new Cell with v n to cells.\n    Repeat for c in cells:\n{}",
        operations("c's v", "        ")
    );
    never_wraps("a field of a record a loop binds", &program(decls, &body));
}

#[test]
fn a_generic_functions_result_never_wraps() {
    let decls = "## To same of [T] (x: T) -> T:\n    Return x.\n";
    never_wraps("a generic function's result", &program(decls, &operations("same(n)", "    ")));
}

#[test]
fn a_binding_of_a_generic_functions_result_never_wraps() {
    let decls = "## To same of [T] (x: T) -> T:\n    Return x.\n";
    let body = format!("    Let x be same(n).\n{}", operations("x", "    "));
    never_wraps("a binding of a generic function's result", &program(decls, &body));
}

#[test]
fn a_maybes_payload_never_wraps() {
    let decls = "## To maybeOf (x: Int) -> Maybe Int:\n    Return some x.\n";
    let body = format!(
        "    Let m be maybeOf(n).\n    Inspect m:\n        When Some (v):\n{}        Otherwise:\n            Show \"none\".\n",
        operations("v", "            ")
    );
    never_wraps("a Maybe's payload", &program(decls, &body));
}
