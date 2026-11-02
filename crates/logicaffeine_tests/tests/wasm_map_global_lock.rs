//! ★ LOCK — a Map held in a module global is readable and writable inside a FUNCTION. ★
//!
//! A map a routine touches — a counter per name, a cache, a lookup a helper fills in — is ordinary
//! LOGOS with no frontend anywhere near it. The WebAssembly backend refused every one of them with
//! "map value of unknown/unsupported kind", a message about the backend's own tables rather than
//! about anything the author wrote, and only when the access was inside a FUNCTION: the identical
//! map read at top level compiles and runs.
//!
//! ★ THE CAUSE is exactly the one `wasm_record_global_lock` records for records, one collection
//! along. A map's VALUE KIND is not carried by `Kind::Map`; it is resolved from the register that
//! was written into the map, which exists only in the region that BUILT the map. A function that
//! merely reads a global built in `Main` has no such register, so the value kind is unknown and the
//! load has no width to load at. A map PARAMETER already had the answer threaded in from its
//! declared type (`index_value_kind`); a map GLOBAL had nothing.
//!
//! ★ WHY IT MATTERS BEYOND THE BUG: this is the primitive an instance store is built from. A
//! component placed twice needs `Map of Text to <its state>` keyed by instance path, read and
//! written by every action — which is a map global touched inside a function, on every turn. The
//! frontend gap and this one are the same gap, and it is fixed HERE, in plain LOGOS, where it is
//! actually reachable — a lock on the component would pin the symptom and leave the cause.
//!
//! Every case RUNS on all three tiers and the answers are held equal. Compiling is not enough:
//! `compile_to_wasm` returns `Ok` for bytes no engine accepts, so `run_show` validates and executes.

#![cfg(feature = "wasm-jit")]

#[path = "compile_common/mod.rs"]
mod common;

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// All three tiers run it and agree — the reference tier says what the program means, and a backend
/// is only correct when it answers the same thing.
fn all_three(src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker refused:\n{src}");
    assert_eq!(tw.output.trim_end(), expected, "tree-walker answer:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM refused:\n{src}");
    assert_eq!(vm.output.trim_end(), expected, "VM answer:\n{src}");
    match crate::common::wasm_host::run_show(src) {
        Ok(got) => assert_eq!(got.trim_end(), expected, "wasm answer:\n{src}"),
        Err(why) => panic!("wasm could not run it ({why}):\n{src}"),
    }
}

#[test]
fn a_function_reads_a_map_global() {
    all_three(
        "## To look (k: Text) -> Int:\n\
        \x20   Return item k of m.\n\
         \n\
         ## Main\n\
         Let mutable m be a new Map of Text to Int.\n\
         Set item \"a\" of m to 7.\n\
         Show look(\"a\").\n",
        "7",
    );
}

#[test]
fn a_function_writes_a_map_global() {
    // ★ THE SHAPE AN INSTANCE STORE HAS: read, change, write back, twice, and the second call must
    // see the first. A backend that silently wrote to a copy answers 1.
    all_three(
        "## To bump (k: Text):\n\
        \x20   Set item k of m to (item k of m) + 1.\n\
         \n\
         ## Main\n\
         Let mutable m be a new Map of Text to Int.\n\
         Set item \"a\" of m to 0.\n\
         bump(\"a\").\n\
         bump(\"a\").\n\
         Show item \"a\" of m.\n",
        "2",
    );
}

#[test]
fn keys_do_not_bleed_into_each_other() {
    // Isolation between keys is the property an instance store actually rests on.
    all_three(
        "## To bump (k: Text):\n\
        \x20   Set item k of m to (item k of m) + 1.\n\
         \n\
         ## Main\n\
         Let mutable m be a new Map of Text to Int.\n\
         Set item \"a\" of m to 0.\n\
         Set item \"b\" of m to 0.\n\
         bump(\"a\").\n\
         bump(\"a\").\n\
         bump(\"b\").\n\
         Show item \"a\" of m.\n\
         Show item \"b\" of m.\n",
        "2\n1",
    );
}

#[test]
fn a_text_valued_map_global_works_in_a_function() {
    // A handle-valued map takes a different store/load width than an Int one, so it is its own case.
    all_three(
        // (`put` is not usable as a function name — `Put … into …` is a statement form — which is
        // its own small finding and nothing to do with maps.)
        "## To remember (k: Text) and (v: Text):\n\
        \x20   Set item k of m to v.\n\
         \n\
         ## Main\n\
         Let mutable m be a new Map of Text to Text.\n\
         remember(\"greeting\", \"hello\").\n\
         Show item \"greeting\" of m.\n",
        "hello",
    );
}

#[test]
fn a_record_valued_map_global_works_in_a_function() {
    // ★ THE ONE THE INSTANCE STORE NEEDS: a map from a key to a RECORD, read into a local, changed,
    // and written back — which is precisely how a component's turn will touch its own instance.
    all_three(
        "## A S has:\n\
        \x20   a n, which is Int.\n\
         \n\
         ## To bump (k: Text):\n\
        \x20   Let mutable r be item k of m.\n\
        \x20   Set r's n to r's n + 1.\n\
        \x20   Set item k of m to r.\n\
         \n\
         ## Main\n\
         Let mutable m be a new Map of Text to S.\n\
         Set item \"a\" of m to a new S with n 0.\n\
         bump(\"a\").\n\
         bump(\"a\").\n\
         Show (item \"a\" of m)'s n.\n",
        "2",
    );
}

#[test]
fn a_record_valued_map_parameter_reads_a_field() {
    // ★ THE DISCRIMINATOR. If a record-valued map fails as a PARAMETER too, the gap is general (a
    // map's values carry a kind but no LAYOUT) rather than anything to do with globals — and the two
    // want different fixes. Written down so the next reader inherits the distinction rather than
    // re-deriving it.
    all_three(
        "## A S has:\n\
        \x20   a n, which is Int.\n\
         \n\
         ## To look (t: Map of Text to S) and (k: Text) -> Int:\n\
        \x20   Return (item k of t)'s n.\n\
         \n\
         ## Main\n\
         Let mutable m be a new Map of Text to S.\n\
         Set item \"a\" of m to a new S with n 4.\n\
         Show look(m, \"a\").\n",
        "4",
    );
}

#[test]
fn a_map_parameter_still_works() {
    // The case that already worked — held so the fix cannot cost it, since both routes end at the
    // same value-kind question and a change to one can quietly claim the other.
    all_three(
        "## To look (t: Map of Text to Int) and (k: Text) -> Int:\n\
        \x20   Return item k of t.\n\
         \n\
         ## Main\n\
         Let mutable m be a new Map of Text to Int.\n\
         Set item \"a\" of m to 5.\n\
         Show look(m, \"a\").\n",
        "5",
    );
}

#[test]
fn a_map_local_to_a_function_still_works() {
    // The other control: a map BUILT inside the function has its value kind from the register that
    // wrote it, which is the path that always worked.
    all_three(
        "## To run () -> Int:\n\
        \x20   Let mutable t be a new Map of Text to Int.\n\
        \x20   Set item \"a\" of t to 9.\n\
        \x20   Return item \"a\" of t.\n\
         \n\
         ## Main\n\
         Show run().\n",
        "9",
    );
}

#[test]
fn the_same_map_read_at_top_level_was_never_broken() {
    // Stated explicitly because it is what made the bug so hard to see: the identical access one
    // scope out compiles and runs, so the failure looked like it was about the map and was about
    // where it was read from.
    all_three(
        "## Main\n\
         Let mutable m be a new Map of Text to Int.\n\
         Set item \"a\" of m to 3.\n\
         Show item \"a\" of m.\n",
        "3",
    );
}
