//! `Repeat for i from a to b` takes any Int bounds. An Int that grew past 64 bits is still an Int, and a range walks it
//! exactly on every engine: the tree-walker, the VM's counted loop (`Fact::IntRange`), and the VM's range value (a bound
//! the optimizer cannot prove an Int). The tree-walker and the range value refused such a bound ("Range requires Int
//! bounds") while the counted loop counted on, so one program printed on the VM what it raised on the tree-walker.

use logicaffeine_compile::compile::{interpret_program, vm_outcome};

fn walks(src: &str, expected: &[&str]) {
    let shown = interpret_program(src).unwrap_or_else(|e| panic!("the tree-walker raised {e:?} on:\n{src}"));
    assert_eq!(shown.lines().collect::<Vec<_>>(), expected, "the tree-walker on:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM raised on:\n{src}");
    assert_eq!(vm.output.lines().collect::<Vec<_>>(), expected, "the VM on:\n{src}");
}

#[test]
fn a_range_past_i64_walks_every_int_in_it() {
    walks(
        "## Main\nLet big be 9223372036854775807 + 1.\nRepeat for i from big to big + 2:\n    Show i.\n",
        &["9223372036854775808", "9223372036854775809", "9223372036854775810"],
    );
}

#[test]
fn a_range_crossing_the_top_of_i64_walks_across_it() {
    walks(
        "## Main\nRepeat for i from 9223372036854775806 to 9223372036854775807 + 2:\n    Show i.\n",
        &["9223372036854775806", "9223372036854775807", "9223372036854775808", "9223372036854775809"],
    );
}

#[test]
fn a_range_below_i64_walks_up_to_its_bottom() {
    walks(
        "## Main\nLet low be (0 - 9223372036854775807) - 3.\nRepeat for i from low to low + 2:\n    Show i.\n",
        &["-9223372036854775810", "-9223372036854775809", "-9223372036854775808"],
    );
}

#[test]
fn a_range_past_i64_whose_start_is_past_its_end_is_empty() {
    walks(
        "## Main\nLet big be 9223372036854775807 + 1.\nRepeat for i from big + 1 to big:\n    Show i.\nShow 0.\n",
        &["0"],
    );
}

#[test]
fn a_range_value_past_i64_walks_every_int_in_it() {
    walks(
        "## To two_on (n: Int) -> Int:\n    Return n + 2.\n\n## Main\nLet big be 9223372036854775807 + 1.\n\
         Repeat for i from big to two_on(big):\n    Show i.\n",
        &["9223372036854775808", "9223372036854775809", "9223372036854775810"],
    );
}

#[test]
fn a_range_counted_in_a_function_takes_a_bound_past_i64() {
    walks(
        "## To count (a: Int) -> Int:\n    Let mutable t be 0.\n    Repeat for i from a to a + 2:\n        Set t to t + 1.\n    \
         Return t.\n\n## Main\nLet big be 9223372036854775807 + 1.\nShow count(5).\nShow count(big).\n",
        &["3", "3"],
    );
}
