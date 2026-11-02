//! ★ LOCK — the VM's compiled "silent wrong answer" on a place-expression mutation. ★
//!
//! `Set item I of RECORD's FIELD to V.` used to silently no-op on the bytecode VM (the tier
//! `largo run` actually executes): `Stmt::SetIndex`'s compiler arm had no write-back path for a
//! `FieldAccess` collection, so `GetField` cloned the field's `Rc` (bumping its `strong_count` to
//! 2), `ensure_reg_owned` saw the inflated count and deep-cloned the list, and the mutation landed
//! on the throwaway clone — never on `RECORD`'s own field. The tree-walker was unaffected.
//!
//! The fix is ONE general lowering (`Compiler::compile_place_mutation`) every mutating statement's
//! target goes through: `Push`, `Add to`, `Remove from` and `Set item ... of ...` all resolve their
//! place — a local, a field, an item, a field of a field, an item of a field, a field of an item,
//! arbitrarily nested — through it, instead of each carrying its own special-cased opcode.
//!
//! ONE exception, kept deliberately: `Push V to <name>'s <field>.` (a single-level field of a bare
//! identifier) still emits `Op::ListPushField` directly, unchanged from before this session — every
//! OTHER backend (RISC-V/C/wasm/SVA/DRAM) still lowers that op, so letting it go unemitted silently
//! broke their own coverage censuses (`wasm_aot_lock`, `c_bytecode_lock`) even though the general
//! mechanism reaches the identical place, just at strictly greater cost. Every OTHER place shape —
//! anything with real nesting, or a non-`Push` mutation — goes through `compile_place_mutation`.
//! `a_single_level_field_push_compiles_to_list_push_field` pins the fast path so it cannot silently
//! fall away again the way it did once already.
//!
//! Every row here is checked against the tree-walker (the oracle) via `tw_outcome`/`vm_outcome`,
//! never a hand-written expected string — the tree-walker was never wrong; only the VM was.

use logicaffeine_compile::compile::{compile_to_program, tw_outcome, vm_outcome, RunOutcome};

/// `tw_outcome`'s output is its lines `join`ed (no final newline); `vm_outcome`'s is the VM's own
/// output buffer (one trailing newline per `Show`, including the last) — a harmless, pre-existing
/// convention difference between the two entry points, unrelated to place mutation. `error` is
/// still compared byte for byte.
fn assert_tiers_agree(src: &str) -> RunOutcome {
    let tw = tw_outcome(src);
    let vm = vm_outcome(src);
    assert_eq!(
        vm.error, tw.error,
        "error diverged for:\n{src}\ntree-walker={tw:?}\nvm={vm:?}"
    );
    assert_eq!(
        vm.output.trim_end_matches('\n'),
        tw.output.trim_end_matches('\n'),
        "VM diverged from the tree-walker oracle for:\n{src}\ntree-walker={tw:?}\nvm={vm:?}"
    );
    tw
}

const BOX_TAPE: &str = "## A Box has:\n    a tape, which is Seq of Int.\n\n";

/// The coordinator's own minimal repro, byte for byte: `Set item 2 of b's tape to 999.` must
/// actually change what `item 2 of b's tape` reads back as, not silently no-op.
#[test]
fn set_item_of_a_records_field_mutates_the_field() {
    let src = format!(
        "{BOX_TAPE}## Main\n\
         Let b be a new Box with tape (a new Seq of Int).\n\
         Push 10 to b's tape.\n\
         Push 20 to b's tape.\n\
         Set item 2 of b's tape to 999.\n\
         Show item 2 of b's tape.\n"
    );
    let out = assert_tiers_agree(&src);
    assert_eq!(
        out.output.trim(),
        "999",
        "the write must land, not the pre-mutation 20"
    );
}

/// The read-back is unaffected by any OTHER field on the same record — the write targets exactly
/// the named field, not the whole struct's storage.
#[test]
fn set_item_of_a_field_leaves_sibling_fields_alone() {
    let src =
        "## A Pair has:\n    a first, which is Seq of Int.\n    a second, which is Seq of Int.\n\n\
               ## Main\n\
               Let p be a new Pair with first (a new Seq of Int) and second (a new Seq of Int).\n\
               Push 1 to p's first.\n\
               Push 2 to p's first.\n\
               Push 100 to p's second.\n\
               Set item 1 of p's first to 999.\n\
               Show item 1 of p's first.\n\
               Show item 1 of p's second.\n";
    let out = assert_tiers_agree(src);
    assert_eq!(out.output.trim(), "999\n100");
}

/// `Push` to a single-level field access is the ALREADY-working case (`ListPushField` covered it)
/// — locked here so the new unified lowering does not regress it.
#[test]
fn push_to_a_records_field_still_mutates_the_field() {
    let src = format!(
        "{BOX_TAPE}## Main\n\
         Let b be a new Box with tape (a new Seq of Int).\n\
         Push 5 to b's tape.\n\
         Push 6 to b's tape.\n\
         Show item 2 of b's tape.\n"
    );
    let out = assert_tiers_agree(&src);
    assert_eq!(out.output.trim(), "6");
}

/// `Push` to a FIELD OF A FIELD — the tree-walker used to refuse this outright ("Push to nested
/// field access not supported"); both tiers now support it, agreeing with each other.
#[test]
fn push_to_a_field_of_a_field_mutates_the_nested_field() {
    let src = "## An Inner has:\n    a tape, which is Seq of Int.\n\n\
               ## An Outer has:\n    an inner, which is Inner.\n\n\
               ## Main\n\
               Let o be a new Outer with inner (a new Inner with tape (a new Seq of Int)).\n\
               Push 7 to o's inner's tape.\n\
               Push 8 to o's inner's tape.\n\
               Show item 2 of o's inner's tape.\n";
    let out = assert_tiers_agree(src);
    assert_eq!(out.output.trim(), "8");
}

/// `Set item ... of ...` through a FIELD OF A FIELD — the general nested-place shape the
/// coordinator's design explicitly names.
#[test]
fn set_item_of_a_field_of_a_field_mutates_the_deepest_field() {
    let src = "## An Inner has:\n    a tape, which is Seq of Int.\n\n\
               ## An Outer has:\n    an inner, which is Inner.\n\n\
               ## Main\n\
               Let o be a new Outer with inner (a new Inner with tape (a new Seq of Int)).\n\
               Push 1 to o's inner's tape.\n\
               Push 2 to o's inner's tape.\n\
               Push 3 to o's inner's tape.\n\
               Set item 2 of o's inner's tape to 999.\n\
               Show item 1 of o's inner's tape.\n\
               Show item 2 of o's inner's tape.\n\
               Show item 3 of o's inner's tape.\n";
    let out = assert_tiers_agree(src);
    assert_eq!(out.output.trim(), "1\n999\n3");
}

/// `Set item ... of (item ... of grid)` — an ITEM OF AN ITEM: a top-level Local list of lists,
/// mutating one element's own nested list by index.
#[test]
fn set_item_of_an_item_mutates_the_nested_list_element() {
    let src = "## Main\n\
               Let row0 be a new Seq of Int.\n\
               Push 1 to row0.\n\
               Push 2 to row0.\n\
               Let row1 be a new Seq of Int.\n\
               Push 10 to row1.\n\
               Push 20 to row1.\n\
               Let grid be a new Seq of (Seq of Int).\n\
               Push row0 to grid.\n\
               Push row1 to grid.\n\
               Set item 2 of (item 2 of grid) to 999.\n\
               Show item 1 of (item 1 of grid).\n\
               Show item 2 of (item 2 of grid).\n";
    let out = assert_tiers_agree(src);
    assert_eq!(out.output.trim(), "1\n999");
}

/// A FIELD OF AN ITEM: a list of records, mutating one element's own field.
#[test]
fn set_item_of_a_field_of_an_item_mutates_the_records_field() {
    let src = "## A Row has:\n    a tape, which is Seq of Int.\n\n\
               ## Main\n\
               Let r0 be a new Row with tape (a new Seq of Int).\n\
               Push 1 to r0's tape.\n\
               Let rows be a new Seq of Row.\n\
               Push r0 to rows.\n\
               Push 2 to (item 1 of rows)'s tape.\n\
               Set item 2 of (item 1 of rows)'s tape to 999.\n\
               Show item 1 of (item 1 of rows)'s tape.\n\
               Show item 2 of (item 1 of rows)'s tape.\n";
    let out = assert_tiers_agree(src);
    assert_eq!(out.output.trim(), "1\n999");
}

/// `Add to` a Set-typed field must add to the SAME field, not a throwaway clone.
#[test]
fn add_to_a_records_set_field_mutates_the_field() {
    let src = "## A Bag has:\n    members, which is Set of Int.\n\n\
               ## Main\n\
               Let b be a new Bag with members (a new Set of Int).\n\
               Add 1 to b's members.\n\
               Add 2 to b's members.\n\
               Show length of b's members.\n";
    let out = assert_tiers_agree(src);
    assert_eq!(out.output.trim(), "2");
}

/// `Remove from` a records field must remove from the SAME field, not a throwaway clone.
#[test]
fn remove_from_a_records_set_field_mutates_the_field() {
    let src = "## A Bag has:\n    members, which is Set of Int.\n\n\
               ## Main\n\
               Let b be a new Bag with members (a new Set of Int).\n\
               Add 1 to b's members.\n\
               Add 2 to b's members.\n\
               Remove 1 from b's members.\n\
               Show length of b's members.\n";
    let out = assert_tiers_agree(src);
    assert_eq!(out.output.trim(), "1");
}

/// Repeated writes to the SAME field across several statements — the steady-state shape the
/// performance concern is about (no reason to deep-clone once per write when nothing else aliases
/// the field).
#[test]
fn repeated_writes_to_the_same_field_all_land() {
    let src = format!(
        "{BOX_TAPE}## Main\n\
         Let b be a new Box with tape (a new Seq of Int).\n\
         Push 0 to b's tape.\n\
         Push 0 to b's tape.\n\
         Push 0 to b's tape.\n\
         Set item 1 of b's tape to 1.\n\
         Set item 2 of b's tape to 2.\n\
         Set item 3 of b's tape to 3.\n\
         Show item 1 of b's tape.\n\
         Show item 2 of b's tape.\n\
         Show item 3 of b's tape.\n"
    );
    let out = assert_tiers_agree(&src);
    assert_eq!(out.output.trim(), "1\n2\n3");
}

/// Pins the ONE deliberate exception noted at the top of this file: a single-level field push
/// compiles to exactly `Op::ListPushField`, not the general mechanism's extract/clear/restore
/// chain — every other backend still lowers this op, so it must keep being emitted for the ONE
/// shape it exists for, or their own coverage censuses (`wasm_aot_lock`, `c_bytecode_lock`) go
/// red again the way they did the first time this fast path was folded away.
#[test]
fn a_single_level_field_push_compiles_to_list_push_field() {
    let src = format!("{BOX_TAPE}## Main\nLet b be a new Box with tape (a new Seq of Int).\nPush 5 to b's tape.\n");
    let program = compile_to_program(&src).expect("the fixture must compile");
    let names: Vec<&'static str> = program.code.iter().map(|op| op.name()).collect();
    assert!(
        names.contains(&"ListPushField"),
        "a single-level field push must still compile to `Op::ListPushField`, not the general \
         place-mutation mechanism — every other backend still lowers this op: {names:?}"
    );
    assert!(
        !names.contains(&"GetField"),
        "the fast path must not ALSO run the general mechanism's extract/clear/restore dance \
         for this shape: {names:?}"
    );
}
