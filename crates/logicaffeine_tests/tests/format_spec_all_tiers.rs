//! A `{x:$}` or `{x:.N}` interpolation hole formats every kind of value the same on every tier.
//!
//! The rule is the interpreter's `semantics::format::apply_format_spec`: `$` gives an Int or a Real
//! two decimals and any other kind its own text after the `$` (an exact decimal keeps its digits,
//! a money its currency); `.N` rounds an Int or a Real to N places and leaves any other kind as it
//! shows. The compiled tiers cast every hole to a float, so a decimal could not be printed as
//! currency at all (AOT-Rust: E0605; C: a struct cast to `double`). Each program must print the
//! same on the tree-walker, the VM, AOT-Rust and C.

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

fn on_every_tier(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, expected);
}

#[test]
fn currency_of_ints_and_reals() {
    on_every_tier(
        "## Main\nLet n be 5.\nLet r be 2.5.\nLet big be 1234567.\nShow \"{n:$} {r:$} {big:$}\".\n",
        "$5.00 $2.50 $1234567.00",
    );
}

#[test]
fn currency_of_a_decimal_keeps_its_digits() {
    on_every_tier(
        "## Main\nLet a be decimal(\"4.50\").\nLet b be decimal(\"3.25\").\nLet total be a + b.\nShow \"{a:$} + {b:$} = {total:$}\".\n",
        "$4.50 + $3.25 = $7.75",
    );
}

#[test]
fn precision_of_ints_and_reals() {
    on_every_tier(
        "## Main\nLet n be 7.\nLet r be 3.14159.\nShow \"{n:.2} {r:.3} {r:.0}\".\n",
        "7.00 3.142 3",
    );
}

#[test]
fn precision_leaves_a_decimal_as_it_shows() {
    on_every_tier(
        "## Main\nLet d be decimal(\"2.5\").\nShow \"{d:.2}\".\n",
        "2.5",
    );
}

#[test]
fn a_spec_hole_inside_a_let_formats_like_show() {
    on_every_tier(
        "## Main\nLet price be decimal(\"19.99\").\nLet label be \"cost: {price:$}\".\nShow label.\nLet n be 3.\nLet line be \"n = {n:.1}\".\nShow line.\n",
        "cost: $19.99\nn = 3.0",
    );
}
