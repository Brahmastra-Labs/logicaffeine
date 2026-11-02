//! Mutable Value Semantics for RECORD parameters, on the tree-walker, the VM, and compiled Rust.
//!
//! A plain parameter is a copy: nothing the callee writes through it reaches the caller, whatever
//! the field's type and whatever the write's shape. A `mutable` parameter is in-out: every write
//! the callee makes through it — a scalar field, an indexed element of a `Seq` field, a push onto
//! a `Seq` field — is the caller's own value when the call returns, including across a nested
//! call that passes the parameter on as `mutable` again.
//!
//! One row per (parameter mode, write shape), each asserting the caller's value on every tier
//! through `every_tier`, so a tier that leaks a plain write or drops an in-out write fails its row
//! by name. `value_semantics.rs` pins the same two rules for `Seq` parameters; this file
//! is their record-parameter census.

mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted_eq};

/// `src` shows `expected` on the tree-walker and the VM, and as a compiled Rust binary.
fn every_tier(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted_eq(src, expected);
}

const BOX: &str = "\
## A Box has:
    a tape, which is Seq of Int.
    a n, which is Int.
";

/// Runs `body` as the callee over a caller-owned `Box` (tape `[10, 20]`, n `0`) passed in `mode`
/// (`""` or `"mutable "`), then shows the caller's tape and n.
fn caller_after(mode: &str, body: &str) -> String {
    format!(
        "{BOX}
## To poke(b: {mode}Box):
{body}

## Main
Let mutable b be a new Box with tape (a new Seq of Int) and n 0.
Push 10 to b's tape.
Push 20 to b's tape.
poke(b).
Show b's tape.
Show b's n.
"
    )
}

#[test]
fn a_plain_record_param_keeps_an_indexed_field_write_to_itself() {
    every_tier(
        &caller_after("", "    Set item 1 of b's tape to 99."),
        "[10, 20]\n0",
    );
}

#[test]
fn a_plain_record_param_keeps_a_pushed_field_to_itself() {
    every_tier(&caller_after("", "    Push 7 to b's tape."), "[10, 20]\n0");
}

#[test]
fn a_plain_record_param_keeps_a_scalar_field_write_to_itself() {
    every_tier(&caller_after("", "    Set b's n to 5."), "[10, 20]\n0");
}

#[test]
fn a_plain_record_param_keeps_a_whole_field_replacement_to_itself() {
    every_tier(&caller_after("", "    Set b's tape to [1]."), "[10, 20]\n0");
}

#[test]
fn a_plain_record_param_keeps_a_popped_field_to_itself() {
    every_tier(&caller_after("", "    Pop from b's tape."), "[10, 20]\n0");
}

#[test]
fn a_mutable_record_param_hands_back_a_popped_field() {
    every_tier(
        &caller_after("mutable ", "    Pop from b's tape."),
        "[10]\n0",
    );
}

#[test]
fn a_mutable_record_param_hands_back_an_indexed_field_write() {
    every_tier(
        &caller_after("mutable ", "    Set item 1 of b's tape to 99."),
        "[99, 20]\n0",
    );
}

#[test]
fn a_mutable_record_param_hands_back_a_pushed_field() {
    every_tier(
        &caller_after("mutable ", "    Push 7 to b's tape."),
        "[10, 20, 7]\n0",
    );
}

#[test]
fn a_mutable_record_param_hands_back_a_scalar_field_write() {
    every_tier(
        &caller_after("mutable ", "    Set b's n to 5."),
        "[10, 20]\n5",
    );
}

#[test]
fn a_mutable_record_param_hands_back_a_whole_field_replacement() {
    every_tier(
        &caller_after("mutable ", "    Set b's tape to [1]."),
        "[1]\n0",
    );
}

#[test]
fn a_mutable_record_param_hands_back_every_write_through_a_nested_mutable_call() {
    let src = format!(
        "{BOX}
## To bump(b: mutable Box):
    Set item 1 of b's tape to (item 1 of b's tape) + 1.
    Push 7 to b's tape.
    Set b's n to b's n + 1.

## To bumpTwice(b: mutable Box):
    bump(b).
    bump(b).

## Main
Let mutable b be a new Box with tape (a new Seq of Int) and n 0.
Push 10 to b's tape.
Push 20 to b's tape.
bumpTwice(b).
Show b's tape.
Show b's n.
"
    );
    every_tier(&src, "[12, 20, 7, 7]\n2");
}

#[test]
fn a_plain_record_param_passed_on_as_mutable_changes_only_the_callees_copy() {
    let src = format!(
        "{BOX}
## To bump(b: mutable Box):
    Push 7 to b's tape.
    Set b's n to b's n + 1.

## To viaCopy(b: Box) -> Int:
    bump(b).
    Return b's n.

## Main
Let mutable b be a new Box with tape (a new Seq of Int) and n 0.
Push 10 to b's tape.
Show viaCopy(b).
Show b's tape.
Show b's n.
"
    );
    every_tier(&src, "1\n[10]\n0");
}

// ---- The whole value of a `mutable` parameter: replaced, copied out, shown, passed on. ----

#[test]
fn a_mutable_record_param_replaced_whole_is_the_callers_new_value() {
    let src = format!(
        "{BOX}
## To reset(b: mutable Box):
    Set b to a new Box with tape [1, 2, 3] and n 9.

## Main
Let mutable b be a new Box with tape (a new Seq of Int) and n 0.
reset(b).
Show b's tape.
Show b's n.
"
    );
    every_tier(&src, "[1, 2, 3]\n9");
}

/// A copy taken out of a `mutable` parameter — returned, kept in a local, pushed into a list — is
/// a value of its own: the writes made through the parameter after it was taken do not reach it.
#[test]
fn a_copy_taken_out_of_a_mutable_record_param_is_its_own_value() {
    let src = format!(
        "{BOX}
## To snapshotThenBump(b: mutable Box) -> Box:
    Let before be b.
    Let mutable kept be a new Seq of Box.
    Push b to kept.
    Push 7 to b's tape.
    Set b's n to 1.
    Show length of (item 1 of kept)'s tape.
    Return before.

## Main
Let mutable b be a new Box with tape (a new Seq of Int) and n 0.
Push 10 to b's tape.
Let old be snapshotThenBump(b).
Show old's tape.
Show old's n.
Show b's tape.
Show b's n.
"
    );
    every_tier(&src, "1\n[10]\n0\n[10, 7]\n1");
}

#[test]
fn a_mutable_record_param_is_shown_whole_and_passed_on_by_value() {
    let src = format!(
        "{BOX}
## To total(b: Box) -> Int:
    Return b's n + length of b's tape.

## To report(b: mutable Box):
    Set b's n to 2.
    Show total(b).

## Main
Let mutable b be a new Box with tape [4, 5] and n 0.
report(b).
Show b's n.
"
    );
    every_tier(&src, "4\n2");
}

// ---- Every other type a `mutable` parameter lends: a scalar, a Text, a Set. ----

#[test]
fn a_mutable_int_param_hands_back_its_writes() {
    every_tier(
        "## To inc(n: mutable Int):\n    Set n to n + 1.\n    Set n to n * 10.\n\n\
         ## To incTwice(n: mutable Int):\n    inc(n).\n    inc(n).\n\n\
         ## Main\nLet x be 1.\nincTwice(x).\nShow x.\n",
        "210",
    );
}

#[test]
fn a_mutable_text_param_hands_back_its_writes() {
    every_tier(
        "## To shout(s: mutable Text):\n    Set s to s + \"!\".\n\n\
         ## Main\nLet mutable greeting be \"hi\".\nLet saved be greeting.\nshout(greeting).\nshout(greeting).\n\
         Show greeting.\nShow saved.\n",
        "hi!!\nhi",
    );
}

#[test]
fn a_mutable_set_param_hands_back_its_writes() {
    every_tier(
        "## To note(s: mutable Set of Int) and (v: Int):\n    Add v to s.\n\n\
         ## Main\nLet seen be a new Set of Int.\nLet before be seen.\nnote(seen, 3).\nnote(seen, 4).\n\
         Show length of seen.\nShow length of before.\n",
        "2\n0",
    );
}

/// Pushing a record stores a copy of it: the variable is still the program's to write, and the
/// write does not reach the copy in the list.
#[test]
fn a_record_pushed_into_a_list_is_a_copy() {
    let src = format!(
        "{BOX}
## Main
Let mutable b be a new Box with tape [1] and n 0.
Let mutable kept be a new Seq of Box.
Push b to kept.
Set b's n to 5.
Show (item 1 of kept)'s n.
Show b's n.
"
    );
    every_tier(&src, "0\n5");
}

/// Returned whole, set into another variable, or listed, a `mutable` parameter hands out a copy;
/// the writes made through it before are in the copy, the ones after are not.
#[test]
fn a_mutable_record_param_handed_out_whole_is_a_copy() {
    let src = format!(
        "{BOX}
## To keep(b: mutable Box) -> Box:
    Set b's n to 3.
    Let mutable c be a new Box with tape [0] and n 0.
    Set c to b.
    Let pair be [b, c].
    Set b's n to 4.
    Show c's n.
    Show (item 1 of pair)'s n.
    Return b.

## Main
Let mutable b be a new Box with tape [9] and n 0.
Let k be keep(b).
Set b's n to 5.
Show k's n.
Show b's n.
"
    );
    every_tier(&src, "3\n3\n4\n5");
}

// ---- A place-write stores a copy: the binding it stores is still the program's to read. ----

const CARD: &str = "\
## A Card has:
    a name, which is Text.
    a tags, which is Seq of Text.
";

#[test]
fn a_field_write_stores_a_copy_of_the_binding() {
    let src = format!(
        "{CARD}
## To keep (who: Text) -> Text:
    Let mutable c be a new Card with name \"\" and tags (a new Seq of Text).
    Set c's name to who.
    Return who + c's name.

## Main
Let ts be [\"a\"].
Let mutable c be a new Card with name \"\" and tags (a new Seq of Text).
Set c's tags to ts.
Push \"b\" to c's tags.
Show keep(\"ada\").
Show ts.
Show c's tags.
"
    );
    every_tier(&src, "adaada\n[a]\n[a, b]");
}

#[test]
fn an_indexed_store_a_push_and_an_add_store_copies_of_the_binding() {
    every_tier(
        "## To stores (who: Text) -> Text:\n    Let mutable names be [\"x\", \"y\"].\n    Set item 1 of names to who.\n    \
         Let mutable more be a new Seq of Text.\n    Push who to more.\n    Let mutable seen be a new Set of Text.\n    \
         Add who to seen.\n    Return who + item 1 of names + item 1 of more + who.\n\n\
         ## Main\nShow stores(\"ada\").\n",
        "adaadaadaada",
    );
}

#[test]
fn a_nested_field_write_through_a_mutable_record_param_stores_a_copy() {
    every_tier(
        "## A Session has:\n    a name, which is Text.\n\n## An App has:\n    a session, which is Session.\n\n\
         ## To login (a: mutable App) and (who: Text) -> Text:\n    Set a's session's name to who.\n    Return who + a's session's name.\n\n\
         ## Main\nLet mutable app be a new App with session (a new Session with name \"\").\n\
         Show login(app, \"ada\").\nShow app's session's name.\n",
        "adaada\nada",
    );
}
