//! `Show x to f` hands `x` to the function `f` on every tier; only `show` itself prints.
//!
//! The C tier printed the value instead: it trims the program to the functions its text calls, and a
//! recipient named at the end of a `Show … to f.` sentence was not counted as a call, so `f` was
//! pruned and the statement fell back to printing — a different answer, with no error. A recipient
//! is a call position now (`Give … to f.` too), and a recipient C has no function for is refused at
//! generation. Each program must print the same on the tree-walker, the VM, AOT-Rust and C.

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

fn on_every_tier(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, expected);
}

#[test]
fn a_value_is_handed_to_functions_of_every_parameter_kind() {
    on_every_tier(
        "## To display (data: Text):\n    Show \"Viewing: {data}\".\n\n## To report (n: Int):\n    Show n * 2.\n\n## Main\nLet profile be \"User Profile Data\".\nShow profile to display.\nShow \"Still usable: {profile}\".\nLet n be 21.\nShow n to report.\nShow n.\n",
        "Viewing: User Profile Data\nStill usable: User Profile Data\n42\n21",
    );
}

#[test]
fn show_to_show_prints() {
    on_every_tier("## Main\nLet n be 7.\nShow n to show.\n", "7");
}

#[test]
fn a_variable_named_like_a_function_after_to_is_not_a_call() {
    on_every_tier(
        "## To twice (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nLet mutable twice_seen be 0.\nLet total be 5.\nSet twice_seen to total.\nShow twice(twice_seen).\n",
        "10",
    );
}

/// A standard-library function named only as a recipient is still spliced in — here beside a local
/// value of the same name, where only a call position may demand the library's function.
#[test]
fn a_library_function_named_only_as_a_recipient_is_spliced_in() {
    on_every_tier(
        "## Main\nLet words be 3.\nShow \"a b c\" to words.\nShow words.\n",
        "3",
    );
}
