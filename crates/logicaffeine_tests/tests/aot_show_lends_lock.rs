//! AOT-Rust: `Show x to f` LENDS `x` — the recipient reads it and the caller keeps it.
//!
//! The statement always passed `&x`, which is right for the built-in `show` and for a parameter the
//! borrow pass lowered to a reference, and wrong for a user function whose parameter owns its value:
//! `## To display (data: Text)` lowered to `fn display(data: String)` and `display(&profile)` failed
//! to build (E0308, the Studio's "Memory and Ownership" example). An owning recipient now receives a
//! copy. Each program must build and print exactly what the interpreter prints.

mod common;
use common::assert_compiled_equals_interpreted;

#[test]
fn lending_to_functions_of_every_parameter_shape() {
    assert_compiled_equals_interpreted(
        "## To display (data: Text):\n    Show \"Viewing: {data}\".\n\n## To report (n: Int):\n    Show n * 2.\n\n## Main\nLet profile be \"User Profile Data\".\nShow profile to display.\nShow \"Still usable: {profile}\".\nLet xs be [1, 2, 3].\nShow xs to show.\nLet n be 21.\nShow n to report.\nShow n.\n",
    );
}

#[test]
fn lend_then_copy_then_give() {
    assert_compiled_equals_interpreted(
        "## To display (data: Text):\n    Show \"Viewing: {data}\".\n\n## To consume (data: Text):\n    Show \"Consumed: {data}\".\n\n## Main\nLet profile be \"User Profile Data\".\nShow profile to display.\nShow \"Still usable after lending: {profile}\".\nLet duplicate be copy of profile.\nGive duplicate to consume.\nShow \"Original intact: {profile}\".\n",
    );
}
