//! ★ LOCK — a record reached through ANY nesting of containers resolves its field slots. ★
//!
//! The wasm backend resolves a record field to a static SLOT. Inside `Main` it can trace the
//! construction; inside a FUNCTION that merely reads a global there is nothing to trace, so the
//! layout is recovered from the global's DECLARED type. That recovery is a hand-written match on
//! the shape of the type, and it has been extended three times, each time by one shape, each time
//! after a real program was refused with:
//!
//! ```text
//! wasm AOT backend: unsupported field access with no static slot
//! ```
//!
//! `wasm_record_global_lock` added `Memo`. `wasm_record_list_lock` added `Seq of Memo`.
//! `wasm_map_global_lock` added `Map of Text to Memo`. `stateful_list_page_lock` added
//! `Map of Text to (Seq of Memo)` — the shape per-instance component state produces, which took
//! out every stateful page showing a list of records.
//!
//! ★ FOUR TIMES IS A PATTERN, AND THE PATTERN IS THE POINT. Each fix was correct and each left the
//! next nesting broken, because the resolver enumerates shapes while the type is a TREE. Nothing
//! anywhere said which trees are meant to work, so "is this shape covered?" had no answer except to
//! find a program that failed. This file is that answer: the shapes are enumerated ONCE, here, as
//! programs — so a nesting the resolver cannot see through fails by NAME instead of surfacing later
//! as an unrelated app that will not build.
//!
//! ★ THEY RUN, THEY DO NOT MERELY COMPILE. A missing slot is a refusal and is loud; a WRONG slot is
//! silent and reads a neighbouring field. `compile_to_wasm` returns `Ok` for bytes no engine
//! accepts, so every case below validates, executes, and is held equal to the two tiers that define
//! what the program means.
//!
//! ★★ AND THE MATRIX FOUND THE DISEASE TWICE, AT TWO LAYERS. Fixing the layout resolver made
//! `Map of Text to (Map of Text to Memo)` work and left the two SEQ-rooted cases failing — with a
//! DIFFERENT message, which is what said they had a different cause: *"push to a sequence of
//! unknown element kind"*. The `Kind` lattice enumerates nestings too. It had exactly one,
//! `SeqSeqInt`, whose own comment read *"No `SeqSeqFloat`/… analog exists"*, and everything else
//! fell to `None` — so a `Seq of Seq of <record>` could not be BUILT on this tier at all, never
//! mind read. `Kind::SeqHandle` answers that the way the layout fix answered its layer: the slots
//! are handles, eight bytes, `i32`, which is all a push needs; what they POINT AT comes from the
//! declared type. Two tables had the identical hole (`boundary_to_kind` and the push-refinement).
//!
//! ★ THE LESSON IS THE SECOND FIND, NOT THE FIRST. Fixing one layer does not mean the remaining
//! failures share its cause — re-run the matrix and read the new message rather than assuming.
//!
//! ★ THE FIRST FOUR ARE CONTROLS and are expected to pass. They are load-bearing anyway: without
//! them a failure here reads as "nested types are broken" rather than naming the exact depth at
//! which seeing-through stops, and that distinction is the whole reason this class kept hiding.

#![cfg(feature = "wasm-jit")]

#[path = "compile_common/mod.rs"]
mod common;

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

const MEMO: &str = "## A Memo has:\n\
    \x20   an id, which is Int.\n\
    \x20   a title, which is Text.\n\n";

/// All three tiers run it and agree. The reference tier says what the program means; a backend is
/// correct only when it answers the same thing, and a wrong slot answers something else rather
/// than refusing.
fn all_three(shape: &str, body: &str, expected: &str) {
    let src = format!("{MEMO}{body}");
    let tw = tw_outcome(&src);
    assert_eq!(tw.error, None, "tree-walker refused `{shape}`:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        expected,
        "tree-walker answer for `{shape}`:\n{src}"
    );
    let vm = vm_outcome(&src);
    assert_eq!(vm.error, None, "VM refused `{shape}`:\n{src}");
    assert_eq!(
        vm.output.trim_end(),
        expected,
        "VM answer for `{shape}`:\n{src}"
    );
    match crate::common::wasm_host::run_show(&src) {
        Ok(got) => assert_eq!(
            got.trim_end(),
            expected,
            "a record reached through `{shape}` read the WRONG FIELD on wasm:\n{src}"
        ),
        Err(why) => {
            panic!("a record reached through `{shape}` has no field slot on wasm ({why}):\n{src}")
        }
    }
}

// ── The four shapes that already work — controls ────────────────────────────────────────────────

#[test]
fn a_record_global() {
    all_three(
        "Memo",
        "## To look () -> Text:\n\
        \x20   Return m's title.\n\
         \n\
         ## Main\n\
         Let m be a new Memo with id 1 and title \"one\".\n\
         Show look().\n",
        "one",
    );
}

#[test]
fn a_list_of_records() {
    all_three(
        "Seq of Memo",
        "## To look () -> Text:\n\
        \x20   Repeat for r in rows:\n\
        \x20       Return r's title.\n\
        \x20   Return \"none\".\n\
         \n\
         ## Main\n\
         Let mutable rows be a new Seq of Memo.\n\
         Push (a new Memo with id 1 and title \"one\") to rows.\n\
         Show look().\n",
        "one",
    );
}

#[test]
fn a_map_of_records() {
    all_three(
        "Map of Text to Memo",
        "## To look (k: Text) -> Text:\n\
        \x20   Return (item k of m)'s title.\n\
         \n\
         ## Main\n\
         Let mutable m be a new Map of Text to Memo.\n\
         Set item \"a\" of m to a new Memo with id 1 and title \"one\".\n\
         Show look(\"a\").\n",
        "one",
    );
}

#[test]
fn a_map_of_lists_of_records() {
    // ★ The shape per-instance component state produces: a field declared `Seq of Memo` inside a
    // component with `Actions:` reaches this backend as `Map of Text to (Seq of Memo)`, keyed by
    // instance path. Every stateful page showing a list is this shape.
    all_three(
        "Map of Text to (Seq of Memo)",
        "## To look (k: Text) -> Text:\n\
        \x20   Let rows be item k of m.\n\
        \x20   Repeat for r in rows:\n\
        \x20       Return r's title.\n\
        \x20   Return \"none\".\n\
         \n\
         ## Main\n\
         Let mutable m be a new Map of Text to (Seq of Memo).\n\
         Let mutable rows be a new Seq of Memo.\n\
         Push (a new Memo with id 1 and title \"one\") to rows.\n\
         Set item \"a\" of m to rows.\n\
         Show look(\"a\").\n",
        "one",
    );
}

// ── The shapes nothing has ever looked at ───────────────────────────────────────────────────────

#[test]
fn a_list_of_lists_of_records() {
    // A grid, a table of rows, a page of groups — `Seq of Seq of X` is ordinary LOGOS and the
    // stdlib itself holds several (`postgres.lg` builds `Seq of Seq of Text`).
    //
    // Sequences are 1-INDEXED, so the outer row is `item 1`. `item 0` is a runtime error with its
    // own entry in the error catalog, and it fails identically on every tier — which reads exactly
    // like a backend refusing the shape.
    all_three(
        "Seq of Seq of Memo",
        "## To look (i: Int) -> Text:\n\
        \x20   Let inner be item i of grid.\n\
        \x20   Repeat for r in inner:\n\
        \x20       Return r's title.\n\
        \x20   Return \"none\".\n\
         \n\
         ## Main\n\
         Let mutable grid be a new Seq of Seq of Memo.\n\
         Let mutable row be a new Seq of Memo.\n\
         Push (a new Memo with id 1 and title \"one\") to row.\n\
         Push row to grid.\n\
         Show look(1).\n",
        "one",
    );
}

#[test]
fn a_map_of_maps_of_records() {
    // ★ The shape a stateful component whose state field is itself a MAP produces — the instance
    // map wraps it, giving `Map of Text to (Map of Text to Memo)`. It is also how anyone writes a
    // two-level index by hand.
    all_three(
        "Map of Text to (Map of Text to Memo)",
        "## To look (a: Text, b: Text) -> Text:\n\
        \x20   Let inner be item a of outer.\n\
        \x20   Return (item b of inner)'s title.\n\
         \n\
         ## Main\n\
         Let mutable outer be a new Map of Text to (Map of Text to Memo).\n\
         Let mutable inner be a new Map of Text to Memo.\n\
         Set item \"x\" of inner to a new Memo with id 1 and title \"one\".\n\
         Set item \"a\" of outer to inner.\n\
         Show look(\"a\", \"x\").\n",
        "one",
    );
}

#[test]
fn a_list_of_maps_of_records() {
    all_three(
        "Seq of (Map of Text to Memo)",
        "## To look (i: Int, k: Text) -> Text:\n\
        \x20   Let inner be item i of rows.\n\
        \x20   Return (item k of inner)'s title.\n\
         \n\
         ## Main\n\
         Let mutable rows be a new Seq of (Map of Text to Memo).\n\
         Let mutable inner be a new Map of Text to Memo.\n\
         Set item \"x\" of inner to a new Memo with id 1 and title \"one\".\n\
         Push inner to rows.\n\
         Show look(1, \"x\").\n",
        "one",
    );
}

#[test]
fn a_second_field_reads_its_own_slot_through_the_nesting() {
    // ★ A slot that RESOLVES can still resolve WRONG, and one field cannot show it — reading
    // `title` at offset 0 looks perfect when the record happens to start there. Reading the
    // SECOND field through the deepest nesting is what separates "found a layout" from "found
    // the right one".
    all_three(
        "Map of Text to (Seq of Memo), second field",
        "## To look (k: Text) -> Int:\n\
        \x20   Let rows be item k of m.\n\
        \x20   Repeat for r in rows:\n\
        \x20       Return r's id.\n\
        \x20   Return 0.\n\
         \n\
         ## Main\n\
         Let mutable m be a new Map of Text to (Seq of Memo).\n\
         Let mutable rows be a new Seq of Memo.\n\
         Push (a new Memo with id 42 and title \"one\") to rows.\n\
         Set item \"a\" of m to rows.\n\
         Show look(\"a\").\n",
        "42",
    );
}
