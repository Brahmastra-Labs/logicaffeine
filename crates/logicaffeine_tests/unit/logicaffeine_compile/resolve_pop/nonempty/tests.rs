use crate::ast::stmt::{Expr, Literal, Stmt};
use crate::ui_bridge::with_parsed_program;

/// The lists named by the checks the seam puts before pops, in program order: one entry per pop that
/// may meet an empty list.
fn checked(source: &str) -> Vec<String> {
    with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        let mut out = Vec::new();
        logicaffeine_language::ast_depth::for_each_node(stmts, |node| {
            if let logicaffeine_language::ast_depth::Visited::Stmt(Stmt::Fail {
                reason: Expr::Literal(Literal::Text(t)),
            }) = node
            {
                let text = interner.resolve(*t);
                if let Some(rest) = text.strip_prefix("Cannot pop from ") {
                    out.push(rest.trim_end_matches(": it is empty").to_string());
                }
            }
        });
        out
    })
}

fn main(body: &str) -> String {
    let indented: String = body.lines().map(|l| format!("    {l}\n")).collect();
    format!("## Main\n{indented}")
}

#[test]
fn a_literal_list_holds_its_items() {
    assert!(checked(&main("Let mutable xs be [1, 2, 3].\nPop from xs into v.\nShow v.")).is_empty());
    assert_eq!(
        checked(&main("Let mutable xs be [1].\nPop from xs.\nPop from xs.")),
        vec!["xs"],
        "the second pop may find the list empty"
    );
}

#[test]
fn a_push_gives_the_next_pop_an_item() {
    assert!(checked(&main("Let mutable xs be a new Seq of Int.\nPush 4 to xs.\nPop from xs into v.\nShow v.")).is_empty());
    assert_eq!(checked(&main("Let mutable xs be a new Seq of Int.\nPop from xs.")), vec!["xs"]);
    assert_eq!(
        checked(&main("Let mutable xs be a new Seq of Int.\nPush 4 to xs.\nPop from xs.\nPop from xs.")),
        vec!["xs"]
    );
}

#[test]
fn a_copy_carries_the_length_and_a_concatenation_adds_them() {
    assert!(checked(&main("Let xs be [1, 2].\nLet mutable ys be xs.\nPop from ys.\nPop from ys.")).is_empty());
    assert!(checked(&main("Let mutable ys be [1] followed by [2].\nPop from ys.\nPop from ys.")).is_empty());
    assert_eq!(checked(&main("Let xs be [1, 2].\nLet mutable ys be xs.\nPop from ys.\nPop from ys.\nPop from ys.")), vec!["ys"]);
}

#[test]
fn a_loop_on_the_length_being_positive_pops_unchecked() {
    for cond in [
        "length of xs is greater than 0",
        "length of xs > 0",
        "0 < length of xs",
        "length of xs is at least 1",
        "length of xs is not 0",
        "not (length of xs is 0)",
    ] {
        let program = main(&format!(
            "Let mutable xs be [1, 2, 3].\nLet mutable total be 0.\nWhile {cond}:\n    Pop from xs into v.\n    Set total to total + v.\nShow total."
        ));
        assert!(checked(&program).is_empty(), "{program}");
    }
    assert_eq!(
        checked(&main("Let mutable xs be [1, 2, 3].\nWhile length of xs > 0:\n    Pop from xs.\n    Pop from xs.")),
        vec!["xs"],
        "the second pop of a pass may find the list empty"
    );
    assert_eq!(
        checked(&main("Let mutable xs be [1, 2, 3].\nLet mutable ys be [1].\nWhile length of ys > 0:\n    Pop from xs.\n    Pop from ys.")),
        vec!["xs"],
        "a condition on one list says nothing of another"
    );
}

#[test]
fn a_branch_that_leaves_proves_the_rest_of_the_block() {
    let leaves = "## To last (xs: Seq of Int) -> Int:\n    Let mutable ys be xs.\n    If length of ys is 0:\n        Return 0.\n    Pop from ys into v.\n    Return v.\n\n## Main\nShow last([1, 2]).\n";
    assert!(checked(leaves).is_empty());
    let falls_through = "## To last (xs: Seq of Int) -> Int:\n    Let mutable ys be xs.\n    If length of ys is 0:\n        Show \"empty\".\n    Pop from ys into v.\n    Return v.\n\n## Main\nShow last([1, 2]).\n";
    assert_eq!(checked(falls_through), vec!["ys"]);
    let either = "## To last (xs: Seq of Int) and (c: Bool) -> Int:\n    Let mutable ys be xs.\n    If length of ys is 0 or c:\n        Return 0.\n    Pop from ys into v.\n    Return v.\n\n## Main\nShow last([1, 2], false).\n";
    assert!(checked(either).is_empty(), "both parts fail when the branch is not taken");
    let both = "## To last (xs: Seq of Int) and (c: Bool) -> Int:\n    Let mutable ys be xs.\n    If length of ys is 0 and c:\n        Return 0.\n    Pop from ys into v.\n    Return v.\n\n## Main\nShow last([1, 2], false).\n";
    assert_eq!(checked(both), vec!["ys"], "only one part need fail when the branch is not taken");
    let positive_and = "## To last (xs: Seq of Int) and (c: Bool) -> Int:\n    Let mutable ys be xs.\n    If length of ys > 0 and c:\n        Return 0.\n    Pop from ys into v.\n    Return v.\n\n## Main\nShow last([1, 2], false).\n";
    assert_eq!(
        checked(positive_and),
        vec!["ys"],
        "the list may be empty where `length of ys > 0 and c` failed"
    );
}

/// `body` as the body of `f (c: Bool)`, so a branch on `c` is decided by no constant.
fn on_flag(body: &str) -> String {
    let indented: String = body.lines().map(|l| format!("    {l}\n")).collect();
    format!("## To f (c: Bool):\n{indented}\n## Main\nCall f with true.\n")
}

#[test]
fn branches_meet_on_what_both_leave() {
    let program = on_flag("Let mutable xs be [1].\nIf c:\n    Push 2 to xs.\nOtherwise:\n    Pop from xs.\nPop from xs.");
    assert_eq!(checked(&program), vec!["xs"], "the Otherwise leaves the list empty");
    let program = on_flag("Let mutable xs be [1].\nIf c:\n    Push 2 to xs.\nOtherwise:\n    Push 3 to xs.\nPop from xs.\nPop from xs.");
    assert!(checked(&program).is_empty());
}

#[test]
fn a_list_declared_in_a_block_lends_nothing_to_the_one_it_shadows() {
    let program = on_flag("Let mutable xs be a new Seq of Int.\nIf c:\n    Let mutable xs be [1, 2, 3].\n    Pop from xs.\nOtherwise:\n    Push 1 to xs.\n    Push 2 to xs.\n    Push 3 to xs.\nPop from xs.");
    assert_eq!(checked(&program), vec!["xs"], "only the pop after the If may meet the empty outer list");
}

#[test]
fn a_counted_loop_carries_its_net_change_over_its_passes() {
    let fill_then_drain = |fill: u32, drain: u32| {
        main(&format!(
            "Let mutable xs be a new Seq of Int.\nRepeat for i from 1 to {fill}:\n    Push i to xs.\nLet mutable t be 0.\nRepeat for i from 1 to {drain}:\n    Pop from xs into v.\n    Set t to t + v.\nShow t."
        ))
    };
    assert!(checked(&fill_then_drain(1000, 1000)).is_empty());
    assert!(checked(&fill_then_drain(1000, 3)).is_empty());
    assert_eq!(checked(&fill_then_drain(999, 1000)), vec!["xs"]);
    let pairs = |passes: u32| {
        main(&format!(
            "Let mutable xs be a new Seq of Int.\nRepeat for i from 1 to 1000:\n    Push i to xs.\nRepeat for i from 1 to {passes}:\n    Pop from xs.\n    Pop from xs."
        ))
    };
    assert!(checked(&pairs(500)).is_empty());
    assert_eq!(checked(&pairs(501)), vec!["xs", "xs"]);
}

#[test]
fn a_loop_to_the_length_of_its_own_list_pops_one_item_a_pass() {
    assert!(checked(&main("Let mutable xs be [3, 1, 2].\nRepeat for i from 1 to length of xs:\n    Pop from xs into v.\n    Show v.")).is_empty());
    assert!(checked(&main("Let mutable xs be [3, 1, 2].\nRepeat for i from 2 to length of xs:\n    Pop from xs into v.\n    Push v to xs.\n    Pop from xs.")).is_empty());
    assert_eq!(
        checked(&main("Let mutable xs be [3, 1, 2].\nRepeat for i from 1 to length of xs:\n    Pop from xs.\n    Pop from xs.")),
        vec!["xs", "xs"],
        "two pops a pass outrun one item a pass"
    );
    assert_eq!(
        checked(&main("Let mutable xs be [3, 1, 2].\nLet ys be [1, 2, 3, 4].\nRepeat for i from 1 to length of ys:\n    Pop from xs.")),
        vec!["xs"],
        "a range over another list's length bounds nothing here"
    );
}

#[test]
fn a_loop_that_can_break_keeps_nothing_it_changed() {
    let program = main("Let mutable xs be a new Seq of Int.\nRepeat for i from 1 to 5:\n    If i is 3:\n        Break.\n    Push i to xs.\nPop from xs.\nPop from xs.\nPop from xs.");
    assert_eq!(checked(&program), vec!["xs", "xs", "xs"]);
}

#[test]
fn a_change_inside_a_branch_of_the_loop_is_not_counted() {
    let program = on_flag("Let mutable xs be [1, 2, 3, 4, 5, 6].\nRepeat for i from 1 to 3:\n    If c:\n        Pop from xs.\n    Pop from xs.");
    assert_eq!(checked(&program), vec!["xs", "xs"]);
}

#[test]
fn a_call_that_borrows_the_list_takes_its_bound() {
    let lent = "## To drain (xs: mutable Seq of Int):\n    While length of xs > 0:\n        Pop from xs.\n\n## Main\nLet mutable xs be [1, 2].\nCall drain with xs.\nPop from xs.\n";
    assert_eq!(checked(lent), vec!["xs"], "the loop in `drain` is proven, the pop after the call is not");
    let copied = "## To total (xs: Seq of Int) -> Int:\n    Return length of xs.\n\n## Main\nLet mutable xs be [1, 2].\nShow total(xs).\nPop from xs.\n";
    assert!(checked(copied).is_empty(), "a call handed a copy leaves the list as it was");
}

#[test]
fn a_record_field_list_is_followed_through_its_record() {
    let bag = "## A Bag has:\n    an items, which is Seq of Int.\n\n";
    assert!(checked(&format!("{bag}{}", main("Let mutable b be a new Bag with items [1, 2].\nPop from b's items.\nPop from b's items."))).is_empty());
    assert!(checked(&format!("{bag}{}", main("Let mutable b be a new Bag with items [1, 2].\nLet mutable c be b.\nPop from c's items."))).is_empty());
    assert_eq!(
        checked(&format!("{bag}{}", main("Let mutable b be a new Bag with items [1, 2].\nSet b's items to a new Seq of Int.\nPop from b's items."))),
        vec!["b's items"]
    );
    assert_eq!(
        checked(&format!("{bag}{}", main("Let mutable b be a new Bag with items [1].\nPop from b's items.\nPop from b's items."))),
        vec!["b's items"]
    );
}

#[test]
fn an_item_of_a_list_is_always_checked_and_named() {
    assert_eq!(
        checked(&main("Let mutable xss be [[1, 2], [3]].\nPop from item 2 of xss into v.\nShow v.")),
        vec!["item 2 of xss"]
    );
}

#[test]
fn a_zone_ends_with_only_what_it_leaves_alone() {
    let program = main("Let mutable xs be [1, 2].\nLet mutable ys be [1].\nInside a zone called \"z\":\n    Pop from xs.\nPop from ys.\nPop from xs.\nPop from xs.");
    assert_eq!(checked(&program), vec!["xs", "xs"], "the zone popped xs; ys it left alone");
}

#[test]
fn a_function_starts_knowing_nothing_of_its_lists() {
    let program = "## To first (xs: Seq of Int) -> Int:\n    Let mutable ys be xs.\n    Pop from ys into v.\n    Return v.\n\n## Main\nShow first([1]).\n";
    assert_eq!(checked(program), vec!["ys"]);
}

#[test]
fn a_loop_ends_knowing_only_what_it_left_alone() {
    let program = on_flag("Let mutable xs be [1, 2, 3].\nLet mutable ys be [1].\nWhile length of xs > 0:\n    If c:\n        Pop from xs.\n    Push 1 to ys.\nPop from ys.\nPop from xs.");
    assert_eq!(
        checked(&program),
        vec!["xs"],
        "the pop in the loop runs while xs has items, and only the pop after it may find xs empty: the loop pops \
         xs in a branch, and only pushes ys"
    );
}
