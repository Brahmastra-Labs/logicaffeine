//! `Pop` takes the last item of a list, and a list with no items has none to give: popping it stops
//! the program with one sentence naming the list, on every tier. What the program printed before
//! the stop stands, and a pop from a list that has items is unchanged.
//!
//! A pop proven to find an item runs with no check before it; the programs below hold that proof to
//! every way a list can be empty after all: a shadowing list, a call that borrows the list, a loop one
//! pass too long or two pops a pass, a loop that breaks early.

mod common;
use common::run_all_tiers;

fn stops_with(source: &str, printed: &str, why: &str) {
    let out = run_all_tiers(source);
    assert_eq!(out.output.trim(), printed, "printed before the stop:\n{source}");
    assert_eq!(out.error.as_deref(), Some(why), "{source}");
}

fn runs(source: &str, printed: &str) {
    let out = run_all_tiers(source);
    assert_eq!(out.error, None, "{source}");
    assert_eq!(out.output.trim(), printed, "{source}");
}

#[test]
fn popping_an_empty_list_into_a_name_stops_naming_the_list() {
    stops_with(
        "## Main\nLet mutable xs be a new Seq of Int.\nShow \"before\".\nPop from xs into v.\nShow v.\n",
        "before",
        "Cannot pop from xs: it is empty",
    );
}

#[test]
fn popping_an_empty_list_without_a_name_stops_too() {
    stops_with(
        "## Main\nLet mutable xs be a new Seq of Int.\nPop from xs.\nShow length of xs.\n",
        "",
        "Cannot pop from xs: it is empty",
    );
}

#[test]
fn popping_past_the_last_item_stops_at_that_pop() {
    stops_with(
        "## Main\nLet mutable xs be a new Seq of Int.\nPush 7 to xs.\nPop from xs into a.\nShow a.\nPop from xs into b.\nShow b.\n",
        "7",
        "Cannot pop from xs: it is empty",
    );
}

#[test]
fn a_field_list_is_named_as_the_program_writes_it() {
    stops_with(
        "## A Bag has:\n    an items, which is Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items (a new Seq of Int).\nPop from b's items into v.\nShow v.\n",
        "",
        "Cannot pop from b's items: it is empty",
    );
}

#[test]
fn a_pop_inside_a_loop_and_a_function_stops_where_the_list_runs_out() {
    stops_with(
        "## To drain (n: Int) -> Int:\n    Let mutable xs be [1, 2].\n    Let mutable total be 0.\n    Repeat for i from 1 to n:\n        Pop from xs into v.\n        Set total to total + v.\n    Return total.\n\n## Main\nShow drain(2).\nShow drain(3).\n",
        "3",
        "Cannot pop from xs: it is empty",
    );
}

#[test]
fn a_pop_from_a_list_with_items_is_unchanged() {
    runs(
        "## Main\nLet mutable xs be [4, 5].\nPop from xs into v.\nShow v * 2.\nShow xs.\nPop from xs.\nShow length of xs.\n",
        "10\n[4]\n0",
    );
    runs(
        "## A Point has:\n    An x: Int.\n\n## Main\nLet mutable ps be a new Seq of Point.\nPush a new Point with x 3 to ps.\nPop from ps into p.\nShow p.\n",
        "Point { x: 3 }",
    );
}

#[test]
fn a_list_of_lists_names_the_item_it_pops_from() {
    stops_with(
        "## Main\nLet mutable xss be [[1, 2], [3]].\nPop from item 2 of xss into v.\nShow v.\nPop from item 2 of xss into w.\nShow w.\n",
        "3",
        "Cannot pop from item 2 of xss: it is empty",
    );
}

#[test]
fn a_shadowing_list_lends_its_items_to_nothing_after_its_block() {
    stops_with(
        "## To f (c: Bool):\n    Let mutable xs be a new Seq of Int.\n    If c:\n        Let mutable xs be [1, 2, 3].\n        Pop from xs into v.\n        Show v.\n    Otherwise:\n        Push 1 to xs.\n    Pop from xs into w.\n    Show w.\n\n## Main\nCall f with false.\nCall f with true.\n",
        "1\n3",
        "Cannot pop from xs: it is empty",
    );
}

#[test]
fn a_call_that_borrows_the_list_can_empty_it() {
    stops_with(
        "## To drain (xs: mutable Seq of Int):\n    While length of xs > 0:\n        Pop from xs.\n\n## Main\nLet mutable xs be [1, 2].\nCall drain with xs.\nShow length of xs.\nPop from xs.\n",
        "0",
        "Cannot pop from xs: it is empty",
    );
}

#[test]
fn a_counted_loop_one_pass_too_long_stops_at_its_last_pass() {
    let program = |fill: u32| {
        format!(
            "## Main\nLet mutable xs be a new Seq of Int.\nRepeat for i from 1 to {fill}:\n    Push i to xs.\nLet mutable t be 0.\nRepeat for i from 1 to 1000:\n    Pop from xs into v.\n    Set t to t + v.\nShow t.\n"
        )
    };
    runs(&program(1000), "500500");
    stops_with(&program(999), "", "Cannot pop from xs: it is empty");
}

#[test]
fn two_pops_a_pass_outrun_a_loop_over_the_length() {
    runs(
        "## Main\nLet mutable xs be [3, 1, 2].\nRepeat for i from 1 to length of xs:\n    Pop from xs into v.\n    Show v.\n",
        "2\n1\n3",
    );
    stops_with(
        "## Main\nLet mutable xs be [3, 1, 2, 4].\nRepeat for i from 1 to length of xs:\n    Pop from xs into v.\n    Pop from xs into w.\n    Show v + w.\n",
        "6\n4",
        "Cannot pop from xs: it is empty",
    );
}

#[test]
fn a_loop_that_breaks_early_leaves_fewer_items() {
    stops_with(
        "## Main\nLet mutable xs be a new Seq of Int.\nRepeat for i from 1 to 5:\n    If i is 3:\n        Break.\n    Push i to xs.\nPop from xs into a.\nPop from xs into b.\nShow a + b.\nPop from xs.\n",
        "3",
        "Cannot pop from xs: it is empty",
    );
}

#[test]
fn a_branch_that_returns_on_an_empty_list_keeps_the_pop_from_running() {
    runs(
        "## To last (xs: Seq of Int) -> Int:\n    Let mutable ys be xs.\n    If length of ys is 0:\n        Return 0.\n    Pop from ys into v.\n    Return v.\n\n## Main\nShow last([4, 9]).\nShow last([]).\n",
        "9\n0",
    );
}
