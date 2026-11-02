//! AOT-Rust `Text + Text` concat: every operand-shape pairing must lower to Rust that actually
//! compiles. `is_definitely_string_expr_with_vars` (in `codegen/expr.rs`) is the single oracle the
//! concat-vs-raw-`+` decision and the operand-flattening both consult; a shape it cannot see as
//! "definitely a string" falls through to a bare `+`, and `String + String` is `rustc E0308`
//! (missing `&` on the right operand) — surfaced by `hexByteText`'s own body,
//! `charAtText(...) + charAtText(...)`, where BOTH operands are calls returning owned `Text`.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_compiled_equals_interpreted;

const SRC: &str = "\
## A Parcel has:
    A label: Text.

## To showConcats (p: Parcel):
    Let v be \"V\".
    Show \"lit\" + \"lit2\".
    Show v + \"lit\".
    Show \"lit\" + v.
    Show v + v.
    Show charAtText(\"ABC\", 1) + \"lit\".
    Show \"lit\" + charAtText(\"ABC\", 1).
    Show charAtText(\"ABC\", 1) + charAtText(\"ABC\", 2).
    Show v + charAtText(\"ABC\", 1).
    Show charAtText(\"ABC\", 1) + v.
    Show p's label + charAtText(\"ABC\", 1).
    Show charAtText(\"ABC\", 1) + p's label.
    Show (charAtText(\"ABC\", 1) + charAtText(\"ABC\", 2)) + charAtText(\"ABC\", 3).
    Show charAtText(\"ABC\", 1) + (charAtText(\"ABC\", 2) + charAtText(\"ABC\", 3)).
    Show hexByteText(0) + hexByteText(255).

## Main
    Let p be a new Parcel with label \"Z\".
    showConcats(p).
";

/// Every `literal | variable | call | field | concat` pairing on each side of `+`, compiled to
/// Rust and diffed against the tree-walker. The `call + call` rows are the exact shape that
/// failed to compile before the fix — `charAtText(...) + charAtText(...)` and, closing the loop
/// on the original repro, `hexByteText(0) + hexByteText(255)` (each `hexByteText` call is itself
/// a function whose OWN body is `charAtText(...) + charAtText(...)`), all two owned-`Text`-
/// returning calls as their two `+` operands.
///
/// NOT in this matrix: `field + field` (`p's label + p's label`, no call on either side). It
/// fails independently of this fix (confirmed still red with the fix applied, and still red with
/// a genuine typed PARAMETER — not a local `Let` — as the field's owner, ruling out the first
/// hypothesis this investigation tried). The exact mechanism was not isolated within this lane's
/// `codegen/expr.rs`-only grant; filed in `tiers.md` as found-but-not-fixed rather than guessed at.
#[test]
fn text_concat_matrix_of_operand_shapes_compiles_and_matches_interpreter() {
    assert_compiled_equals_interpreted(SRC);
}
