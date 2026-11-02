//! Common-subexpression elimination must not reuse a value across a loop that mutates one of the
//! expression's inputs — however deep the mutation sits.
//!
//! Found through the stdlib: json.lg's rewritten `jsonParseNumber` reads `item p of bytes` once BEFORE its
//! digit loop (`Let first be …`) and again INSIDE it (`Let d be …`), advancing `p` in a nested `If`. The
//! run-path CSE reused `first` for `d`, so `"123"` parsed as 111 on the VM (with and without the JIT) —
//! every digit was the first digit. `## No cse` gave 123. These are the minimal shapes, each asserted
//! against the LITERAL expected output (so a shared front-end bug cannot hide behind tier agreement)
//! and across tree-walker ≡ VM.

use logicaffeine_compile::compile::{
    compile_to_program_optimized, program_outcome, tw_outcome, vm_outcome,
};

/// Tree-walker, raw VM, and the OPTIMIZED VM (`compile_to_program_optimized`, the lattice the CLI's
/// `--interpret` path runs) must all print `expected` — the literal, so a shared front-end bug cannot hide
/// behind tier agreement, and the optimized leg so a pass cannot hide behind the raw compile.
fn assert_prints(label: &str, src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "{label}: tree-walker must run cleanly");
    assert_eq!(tw.output.trim(), expected, "{label}: tree-walker output");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "{label}: VM must run cleanly");
    assert_eq!(
        vm.output.trim(),
        expected,
        "{label}: VM output (raw compile)"
    );
    let program = compile_to_program_optimized(src).unwrap_or_else(|e| {
        panic!("{label}: the optimizer pipeline must compile the program: {e:?}")
    });
    let opt = program_outcome(&program);
    assert_eq!(opt.error, None, "{label}: optimized VM must run cleanly");
    assert_eq!(
        opt.output.trim(),
        expected,
        "{label}: optimized VM output (the run-path lattice)"
    );
}

/// The json.lg shape: a pre-loop read of `item p of xs`, then the same read inside a `While` whose body
/// advances `p` inside a nested `If`.
#[test]
fn a_pre_loop_index_read_is_not_reused_inside_a_loop_that_advances_the_index() {
    let src = "## Main\n    Let xs be [1, 2, 3].\n    Let mutable p be 1.\n    Let first be item p of xs.\n    Let mutable m be 0.\n    While p is at most 3:\n        Let d be item p of xs.\n        If d is at least 0:\n            If d is at most 9:\n                Set m to (m * 10) + d.\n                Set p to p + 1.\n    Show first.\n    Show m.\n";
    assert_prints("nested-if advance", src, "1\n123");
}

/// The mutation at the top level of the loop body (no nesting).
#[test]
fn a_pre_loop_index_read_is_not_reused_when_the_loop_advances_the_index_directly() {
    let src = "## Main\n    Let xs be [4, 2, 7].\n    Let mutable p be 1.\n    Let first be item p of xs.\n    Let mutable total be 0.\n    While p is at most 3:\n        Let d be item p of xs.\n        Set total to total + d.\n        Set p to p + 1.\n    Show first.\n    Show total.\n";
    assert_prints("direct advance", src, "4\n13");
}

/// A `Repeat for` counted loop over the same index, mutated through the loop variable.
#[test]
fn a_pre_loop_read_is_not_reused_across_a_counted_loop_over_the_index() {
    let src = "## Main\n    Let xs be [5, 6, 7, 8].\n    Let i be 1.\n    Let head be item i of xs.\n    Let mutable acc be 0.\n    Repeat for k from 1 to 4:\n        Let v be item k of xs.\n        Set acc to acc + v.\n    Show head.\n    Show acc.\n";
    assert_prints("counted loop", src, "5\n26");
}

/// The collection itself mutated inside the loop (an element write) also invalidates the read.
#[test]
fn a_pre_loop_read_is_not_reused_when_the_loop_writes_the_element() {
    let src = "## Main\n    Let xs be [1, 1, 1].\n    Let p be 2.\n    Let before be item p of xs.\n    Let mutable n be 0.\n    While n is less than 2:\n        Set item p of xs to (item p of xs) + 10.\n        Set n to n + 1.\n    Show before.\n    Show item p of xs.\n";
    assert_prints("element write", src, "1\n21");
}

/// The exact production shape, through the exact production entry: the CLI's `largo run --interpret`
/// (`interpret_for_ui_sync_with_args`) on a FUNCTION whose parameters feed the read — `bytes`/`pos`
/// params, a pre-loop `first`, a nested-`If` advance — is what json.lg's `jsonParseNumber` does.
#[test]
fn the_cli_interpret_path_keeps_a_function_local_index_read_fresh_across_the_digit_loop() {
    use logicaffeine_compile::ui_bridge::interpret_for_ui_sync_with_args;
    let src = "## To parseDigits (bytes: Seq of Int) and (pos: Int) -> Int:\n    Let total be length of bytes.\n    Let mutable p be pos.\n    Let first be item p of bytes.\n    If first is less than 48:\n        Return 0 - 1.\n    Let mutable m be 0.\n    Let mutable go be true.\n    While go is equal to true:\n        If p is greater than total:\n            Set go to false.\n        Otherwise:\n            Let d be item p of bytes.\n            If d is at least 48:\n                If d is at most 57:\n                    Set m to (m * 10) + (d - 48).\n                    Set p to p + 1.\n                Otherwise:\n                    Set go to false.\n            Otherwise:\n                Set go to false.\n    Return m.\n\n## Main\n    Show parseDigits([49, 50, 51], 1).\n";
    let r = interpret_for_ui_sync_with_args(src, &[]);
    assert_eq!(r.error, None, "must run cleanly");
    assert_eq!(
        r.lines,
        vec!["123".to_string()],
        "every digit must be read fresh — 111 means CSE reused the pre-loop read"
    );
}

/// And the stdlib itself, end to end: `jsonParseNumber` on "123" is 123.
#[test]
fn the_stdlib_json_number_parser_reads_every_digit_on_the_cli_interpret_path() {
    use logicaffeine_compile::ui_bridge::interpret_for_ui_sync_with_args;
    let src = "## Main\n    Let st be jsonParseNumber([49, 50, 51], 1).\n    Show st's node.\n    Show st's nextpos.\n";
    let r = interpret_for_ui_sync_with_args(src, &[]);
    assert_eq!(r.error, None, "must run cleanly");
    assert_eq!(r.lines, vec!["JNum(123)".to_string(), "4".to_string()]);
}

/// A mutation inside an `Inspect` arm invalidates a cached read after the `Inspect`, exactly like a
/// mutation inside an `If` branch does.
#[test]
fn a_mutation_inside_an_inspect_arm_invalidates_a_cached_read_after_it() {
    use logicaffeine_compile::ui_bridge::interpret_for_ui_sync_with_args;
    let src = "## A Mode is one of:\n    A Skip.\n    An Advance.\n\n## Main\n    Let xs be [10, 20, 30].\n    Let mutable p be 1.\n    Let a be item p of xs.\n    Let mode be a new Advance.\n    Inspect mode:\n        When Skip:\n            Show 0.\n        When Advance:\n            Set p to 3.\n    Let b be item p of xs.\n    Show a.\n    Show b.\n";
    let r = interpret_for_ui_sync_with_args(src, &[]);
    assert_eq!(r.error, None, "must run cleanly");
    assert_eq!(
        r.lines,
        vec!["10".to_string(), "30".to_string()],
        "`b` must be re-read after the arm mutated `p`"
    );
}

/// The variable HOLDING a cached value is reassigned: the cache entry must die with it, or the next
/// occurrence of the expression reads the overwritten holder instead of recomputing.
#[test]
fn reassigning_the_variable_that_holds_a_cached_value_drops_the_cache_entry() {
    let src = "## Main\n    Let x be 3.\n    Let y be 4.\n    Let mutable d be x * y.\n    Set d to 0.\n    Let e be x * y.\n    Show d.\n    Show e.\n";
    assert_prints("holder reassigned", src, "0\n12");
}

/// A collection mutated THROUGH a field (`Push … to b's items`) mutates the root `b`; a cached read rooted
/// at `b` must not survive it.
#[test]
fn a_push_through_a_field_invalidates_a_cached_read_rooted_at_the_same_object() {
    let src = "## A Bag has:\n    An items: Seq of Int.\n## Main\n    Let mutable b be a new Bag with items [1, 2].\n    Let n be length of b's items.\n    Push 5 to b's items.\n    Let m be length of b's items.\n    Show n.\n    Show m.\n";
    assert_prints("push through a field", src, "2\n3");
}
