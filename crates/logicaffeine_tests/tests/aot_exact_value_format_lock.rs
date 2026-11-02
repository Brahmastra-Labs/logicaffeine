//! AOT-Rust: exact values keep their type through a binding, and format-spec holes format each
//! value type the interpreter's way.
//!
//! Two defects, found by the Studio's money examples. A binding took an exact value type only from
//! a direct constructor call (`Let latte be decimal("4.50")`), so `Let subtotal be latte + muffin`
//! was untyped and `subtotal * 2` lowered to a raw `*` on a `LogosDecimal` (E0369). And a `{x:$}` /
//! `{x:.N}` hole cast its value `as f64` whatever it was, so no exact decimal could be printed as
//! currency at all (E0605). The binding now takes the value type its right-hand side produces, and a
//! spec hole renders through the runtime's `format_currency` / `format_fixed`, which mirror the
//! interpreter's `apply_format_spec` value type by value type. Each program must build and print
//! exactly what the interpreter prints.

mod common;
use common::assert_compiled_equals_interpreted;

#[test]
fn a_decimal_ledger_prints_currency_exactly() {
    assert_compiled_equals_interpreted(
        "## Main\nLet mutable balance be decimal(\"250.00\").\nShow \"opening balance: {balance:$}\".\nLet deposits be [decimal(\"75.50\"), decimal(\"12.25\")].\nRepeat for d in deposits:\n    Set balance to balance + d.\n    Show \"deposit {d:$} -> {balance:$}\".\nLet withdrawal be decimal(\"40.00\").\nSet balance to balance - withdrawal.\nShow \"withdrew {withdrawal:$} -> {balance:$}\".\n",
    );
}

#[test]
fn decimal_arithmetic_through_bindings_stays_exact() {
    assert_compiled_equals_interpreted(
        "## Main\nLet latte be decimal(\"4.50\").\nLet muffin be decimal(\"3.25\").\nLet subtotal be latte + muffin.\nShow \"subtotal: {subtotal:$}\".\nLet two_rounds be subtotal * 2.\nShow \"two rounds: {two_rounds:$}\".\nLet tip be decimal(\"1.55\").\nLet total be two_rounds + tip.\nShow \"with tip: {total:$}\".\nLet cents be decimal(\"0.1\") + decimal(\"0.2\").\nShow \"{cents}\".\n",
    );
}

#[test]
fn a_currency_hole_formats_ints_and_reals_with_cents() {
    assert_compiled_equals_interpreted(
        "## Main\nLet n be 5.\nLet r be 2.5.\nShow \"{n:$} and {r:$}\".\n",
    );
}

#[test]
fn a_precision_hole_formats_ints_reals_and_decimals() {
    assert_compiled_equals_interpreted(
        "## Main\nLet n be 7.\nLet r be 3.14159.\nLet d be decimal(\"2.5\").\nShow \"{n:.2} {r:.3} {d:.2}\".\n",
    );
}

#[test]
fn an_interpolated_string_expression_formats_like_show() {
    assert_compiled_equals_interpreted(
        "## Main\nLet price be decimal(\"19.99\").\nLet label be \"cost: {price:$}\".\nShow label.\n",
    );
}

#[test]
fn money_arithmetic_through_a_binding_stays_money() {
    assert_compiled_equals_interpreted(
        "## Main\nLet a be money(5, \"USD\").\nLet b be a + money(3, \"USD\").\nLet c be b * 2.\nShow c.\n",
    );
}
