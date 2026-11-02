//! Batch collection must preserve callers, returned values, and iterator snapshots.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::run_outcome;
use logicaffeine_compile::compile::vm_outcome;

const CHURN: &str = r#"## To churn (n: Int) -> Int:
    Let mutable total be 0.
    Repeat for i from 1 to n:
        Let xs be [i, i + 1].
        Set total to total + item 1 of xs.
    Return total.

"#;

fn flat(template: &str) {
    let mut heaps = Vec::new();
    for n in [100, 1000] {
        let source = template.replace("$N", &n.to_string());
        let oracle = vm_outcome(&source);
        assert_eq!(oracle.error, None, "VM oracle must run:\n{source}");
        let got = run_outcome(&source).unwrap_or_else(|e| panic!("wasm: {e}\n{source}"));
        assert_eq!(got.failure, None, "wasm must run:\n{source}");
        assert_eq!(
            got.output.lines().collect::<Vec<_>>(),
            oracle.output.lines().collect::<Vec<_>>(),
            "collection must preserve every output line:\n{source}"
        );
        heaps.push(got.heap_ptr.expect("allocating program has a heap"));
    }
    assert_eq!(
        heaps[0], heaps[1],
        "ten times the work must keep the same bounded heap"
    );
}

#[test]
fn a_callers_list_survives_a_callees_collection() {
    flat(&format!(
        "{CHURN}## Main\nLet kept be [73, 74].\nShow churn($N).\nShow kept.\n"
    ));
}

#[test]
fn a_list_returned_from_the_last_iteration_survives() {
    flat(
        r#"## To build (n: Int) -> Seq of Int:
    Let mutable kept be [0].
    Repeat for i from 1 to n:
        Set kept to [i, i + 1].
    Return kept.

## Main
Let answer be build($N).
Show answer.
"#,
    );
}

#[test]
fn each_suspended_caller_keeps_its_own_roots() {
    flat(&format!(
        r#"{CHURN}## To middle (n: Int) -> Int:
    Let kept be [41, 42].
    Let value be churn(n).
    Return value + item 1 of kept.

## Main
Let kept be [73, 74].
Show middle($N).
Show kept.
"#
    ));
}

/// A local bound to a new list after its root was taken is rooted again before the next collection: the
/// root a call before held the old list.
#[test]
fn a_rebound_local_is_rooted_again_before_the_next_collection() {
    flat(&format!(
        "{CHURN}## Main\nLet mutable kept be [1, 2].\nShow churn($N).\nSet kept to [3, 4].\nShow churn($N).\nShow kept.\n"
    ));
}

/// A write in place gives a shared list a copy of its own in the same register: the copy, not the list the
/// root took before the write, is what the next collection must keep.
#[test]
fn a_list_copied_on_write_after_its_root_is_rooted_again() {
    flat(&format!(
        "{CHURN}## Main\nLet kept be [1, 2].\nLet mutable mine be kept.\nShow churn($N).\nPush 3 to mine.\nShow churn($N).\nShow mine.\nShow kept.\n"
    ));
}

/// A call that can never collect takes no roots, and the list it was handed is rooted at the next call that
/// can.
#[test]
fn a_list_passed_to_a_leaf_is_rooted_at_the_next_collection() {
    flat(&format!(
        "{CHURN}## To size (xs: Seq of Int) -> Int:\n    Return length of xs.\n\n## Main\nLet kept be [5, 6].\nShow size(kept).\nShow churn($N).\nShow kept.\n"
    ));
}

/// A function with no loop of its own still collects through what it calls: its caller roots its live lists.
#[test]
fn a_call_that_collects_only_through_its_callee_roots_the_caller() {
    flat(&format!(
        "{CHURN}## To relay (n: Int) -> Int:\n    Return churn(n).\n\n## Main\nLet kept be [7, 8].\nShow relay($N).\nShow kept.\n"
    ));
}

#[test]
fn a_handle_iterator_snapshot_survives_nested_collection() {
    flat(&format!(
        r#"{CHURN}## Main
Let labels be ["a" combined with 1, "b" combined with 2].
Repeat for label in labels:
    Show churn($N).
    Show label.
"#
    ));
}

#[test]
fn a_returned_child_survives_its_parent_frame() {
    flat(
        r#"## A Box has:
    a tape, which is Seq of Int.

## To build (n: Int) -> Seq of Int:
    Let mutable box be a new Box with tape [0].
    Repeat for i from 1 to n:
        Set box to a new Box with tape [i, i + 1].
    Return box's tape.

## Main
Let answer be build($N).
Show answer.
"#,
    );
}

#[test]
fn temporary_lists_from_repeated_calls_are_collected() {
    flat(
        r#"## To pair (n: Int) -> Seq of Int:
    Return [n, n + 1].

## Main
Let mutable total be 0.
Repeat for i from 1 to $N:
    Let xs be pair(i).
    Set total to total + item 1 of xs.
Show total.
"#,
    );
}

#[test]
fn a_scalar_loop_keeps_its_answer() {
    let src = "## Main\nLet mutable total be 0.\nRepeat for i from 1 to 1000:\n    Set total to total + i.\nShow total.\n";
    let got = run_outcome(src).expect("wasm scalar program");
    assert_eq!(got.failure, None);
    assert_eq!(got.output.trim_end(), "500500");
}

#[test]
fn fixed_capacity_local_buffers_are_reclaimed_between_calls() {
    let source = r#"## No Inline
## To measure (n: Int) -> Int:
    Let mutable values be a new Seq of Int.
    Repeat for k from 1 to 8:
        Push n + k to values.
    Let answer be item 8 of values.
    Return answer.

## Main
Let mutable total be 0.
Repeat for i from 1 to $N:
    Set total to total + measure(i).
Show total.
"#;
    let program = logicaffeine_compile::compile::compile_to_program(&source.replace("$N", "100"))
        .expect("fixed-capacity program compiles");
    assert!(
        program.code.iter().any(|op| matches!(
            op,
            logicaffeine_compile::vm::Op::NewEmptyListFixed { capacity: 8, .. }
        )),
        "this reclamation control must exercise the fixed-capacity allocation opcode"
    );
    flat(source);
}

#[test]
fn an_implicit_mutable_text_result_survives_collection() {
    flat(
        r#"## To churn (value: mutable Text):
    Set value to value + "!".
    Let mutable scratch be "".
    Let mutable i be 0.
    While i is less than $N:
        Set scratch to "garbage" + (i % 3).
        Set i to i + 1.
    Show scratch.

## Main
Let mutable value be "kept".
churn(value).
Show value.
"#,
    );
}

#[test]
fn an_implicit_mutable_list_result_keeps_its_children() {
    flat(
        r#"## To churn (value: mutable Seq of Text):
    Set value to ["kept", "child"].
    Let mutable scratch be "".
    Let mutable i be 0.
    While i is less than $N:
        Set scratch to "garbage" + (i % 3).
        Set i to i + 1.
    Show scratch.

## Main
Let mutable value be ["old"].
churn(value).
Show value.
"#,
    );
}

#[test]
fn an_implicit_mutable_record_result_keeps_its_fields() {
    flat(
        r#"## A Box has:
    a tape, which is Seq of Int.

## To churn (value: mutable Box):
    Set value to a new Box with tape [73, 74].
    Let mutable scratch be "".
    Let mutable i be 0.
    While i is less than $N:
        Set scratch to "garbage" + (i % 3).
        Set i to i + 1.
    Show scratch.

## Main
Let mutable value be a new Box with tape [0].
churn(value).
Show value's tape.
"#,
    );
}
