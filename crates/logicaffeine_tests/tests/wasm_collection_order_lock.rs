//! A collection shows its elements in the SAME order on every tier.
//!
//! A `Set` and a `Map` display in insertion order (the VM's `Set` is a `Vec`, its `Map` an
//! `IndexMap`), and removing an element must keep the order of the rest. A `SharedSet` — a CRDT, whose
//! replicas agree on contents but not on arrival order — displays SORTED, on the reference tier by
//! `CrdtValue::render`. Order is observable (`Show`, `format`, iteration), so a backend that stores
//! elements in another order gives a different answer, not a different layout.

#![cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

#[path = "compile_common/mod.rs"]
mod common;

/// The tree-walker, the VM and the WebAssembly module all run `src` and print `expected`.
fn all_three(src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker refused:\n{src}");
    assert_eq!(tw.output.trim_end(), expected, "tree-walker answer:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM refused:\n{src}");
    assert_eq!(vm.output.trim_end(), expected, "VM answer:\n{src}");
    match common::wasm_host::run_show(src) {
        Ok(got) => assert_eq!(got.trim_end(), expected, "wasm answer:\n{src}"),
        Err(why) => panic!("wasm could not run it ({why}):\n{src}"),
    }
}

#[test]
fn removing_from_a_set_keeps_the_order_of_the_rest() {
    all_three(
        "## Main\n\
         Let mutable s be a new Set of Int.\n\
         Add 3 to s.\n\
         Add 1 to s.\n\
         Add 2 to s.\n\
         Remove 3 from s.\n\
         Show s.\n\
         Add 3 to s.\n\
         Show s.\n",
        "{1, 2}\n{1, 2, 3}",
    );
}

#[test]
fn removing_from_a_set_of_text_keeps_the_order_of_the_rest() {
    all_three(
        "## Main\n\
         Let mutable s be a new Set of Text.\n\
         Add \"c\" to s.\n\
         Add \"a\" to s.\n\
         Add \"b\" to s.\n\
         Remove \"c\" from s.\n\
         Show s.\n",
        "{a, b}",
    );
}

#[test]
fn removing_from_a_map_keeps_the_order_of_the_rest() {
    all_three(
        "## Main\n\
         Let mutable m be a new Map of Int to Int.\n\
         Set item 5 of m to 50.\n\
         Set item 1 of m to 10.\n\
         Set item 7 of m to 70.\n\
         Remove 5 from m.\n\
         Show m.\n",
        "{1: 10, 7: 70}",
    );
}

#[test]
fn removing_the_last_element_of_a_set_leaves_the_rest_in_place() {
    // The swap-free case: the removed slot is the last one, so no element moves at all.
    all_three(
        "## Main\n\
         Let mutable s be a new Set of Int.\n\
         Add 3 to s.\n\
         Add 1 to s.\n\
         Remove 1 from s.\n\
         Show s.\n",
        "{3}",
    );
}

#[test]
fn a_shared_set_shows_its_elements_sorted() {
    all_three(
        "## Definition\n\
         A Doc is Shared and has:\n\
         \x20a tags, which is a SharedSet of Text.\n\
         \n\
         ## Main\n\
         Let mutable d be a new Doc.\n\
         Add \"pear\" to d's tags.\n\
         Add \"apple\" to d's tags.\n\
         Add \"fig\" to d's tags.\n\
         Show d's tags.\n\
         Remove \"apple\" from d's tags.\n\
         Show d's tags.\n\
         Add \"banana\" to d's tags.\n\
         Show d's tags.\n\
         Show length of d's tags.\n",
        "{apple, fig, pear}\n{fig, pear}\n{banana, fig, pear}\n3",
    );
}
