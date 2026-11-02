//! A text accumulator that appends a one-character text narrowed to a byte.
//!
//! A `Let mutable ch be "a"` that is only ever set to one-character ASCII literals lowers to a `u8`
//! (`__single_char_u8`), and the in-place append passes — `Set X to X + ch` (`stringaccumreuse`) and its `Let`
//! twin `Let s be build() + ch` (`stringconcatreuse`) — must append it as a character. Appending it as a string
//! (`push_str(&(ch))`) does not compile: benchmarks/programs/string_search, which builds its text one LCG-drawn
//! letter at a time, stopped building through Rust that way. Each program here must fire its pass, build, and
//! print what the VM prints. (A `Let` that reads the character keeps it a `String` today; the twin shares the
//! append emitter, so it stays right if that narrowing rule widens.)
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test string_accum_char_append_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_all_tiers, assert_exact_output};
use logicaffeine_compile::compile::optimizations_fired;

const ACCUMULATE_NARROWED_CHAR: &str = "## Main
Let mutable text be \"\".
Let mutable seed be 42.
Repeat for pos from 1 to 10:
    Set seed to seed + 3.
    Let mutable ch be \"a\".
    If seed % 5 equals 1:
        Set ch to \"b\".
    Set text to text + ch.
Show text.
";

const ACCUMULATE_CHAR_AMONG_TEXTS: &str = "## Main
Let mutable text be \"\".
Repeat for pos from 1 to 4:
    Let mutable ch be \"x\".
    If pos equals 2:
        Set ch to \"y\".
    Set text to text + \"<\" + ch + \">\".
Show text.
";

const LET_FORM_NARROWED_CHAR: &str = "## To base (k: Int) -> Text:
    Let mutable out be \"\".
    Repeat for i from 1 to k:
        Set out to out + \"=\".
    Return out.

## Main
Let mutable k be 0.
Repeat for pos from 1 to 3:
    Let mutable ch be \"a\".
    If pos equals 3:
        Set ch to \"c\".
    Let s be base(pos) + ch.
    Set k to k + length of s.
    Show s.
Show k.
";

fn fires(source: &str, pass: &str) {
    let fired = optimizations_fired(source);
    assert!(
        fired.contains(&pass),
        "`{pass}` must fire on this program (it is the path under test); fired: {fired:?}"
    );
}

#[test]
fn a_narrowed_char_appends_in_place_as_a_character() {
    fires(ACCUMULATE_NARROWED_CHAR, "stringaccumreuse");
    assert_all_tiers(ACCUMULATE_NARROWED_CHAR, "aabaaaabaa");
    assert_exact_output(ACCUMULATE_NARROWED_CHAR, "aabaaaabaa");
}

#[test]
fn a_narrowed_char_between_text_operands_appends_in_order() {
    fires(ACCUMULATE_CHAR_AMONG_TEXTS, "stringaccumreuse");
    assert_all_tiers(ACCUMULATE_CHAR_AMONG_TEXTS, "<x><y><x><x>");
    assert_exact_output(ACCUMULATE_CHAR_AMONG_TEXTS, "<x><y><x><x>");
}

#[test]
fn the_let_form_appends_a_narrowed_char_as_a_character() {
    let expected = "=a\n==a\n===c\n9";
    fires(LET_FORM_NARROWED_CHAR, "stringconcatreuse");
    assert_all_tiers(LET_FORM_NARROWED_CHAR, expected);
    assert_exact_output(LET_FORM_NARROWED_CHAR, expected);
}
