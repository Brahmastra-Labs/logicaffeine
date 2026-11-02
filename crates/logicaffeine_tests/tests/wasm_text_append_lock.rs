//! ★ LOCK — appending to a Text on wasm is linear in what it writes, and every alias keeps its value. ★
//!
//! `Set out to out + piece` is how a program builds a large Text: a serializer, a report, a page's
//! markup. The VM appends in place when the Text is its own; the wasm module used to allocate a fresh
//! Text of the whole new length on every append, so building `n` bytes in `k` pieces cost O(n·k) bytes
//! — `vdom.lg` serializing a 10,000-deep tree ran the module out of memory.
//!
//! A Text is an immutable view `[len][cap][data_ptr]` that several names may share. The module's append
//! (`emit_text_append`) extends a view's buffer in place only when the buffer is growable, has room, and
//! the view is its FRONTIER (no view has written past it), and otherwise copies into a fresh buffer of
//! twice the new length. So the rows here hold both halves: every alias of an appended Text answers what
//! the tree-walker answers, and a loop of appends allocates memory linear in the Text it builds — measured
//! by the heap's bump pointer, not by time.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::run_outcome;

/// `(name, program)` — each an aliasing shape an in-place append could get wrong.
fn rows() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "a_snapshot_taken_mid_loop_keeps_its_length",
            "## Main\nLet mutable out be \"\".\nLet mutable i be 1.\nWhile i <= 5:\n    Set out to out + \"ab\".\n    Set i to i + 1.\n\
             Let snap be out.\nSet out to out + \"X\".\nShow snap.\nShow out.\n",
        ),
        (
            "two_appends_from_one_base_do_not_see_each_other",
            "## Main\nLet mutable base be \"\".\nSet base to base + \"root\".\nLet mutable left be base.\nLet mutable right be base.\n\
             Set left to left + \"-L\".\nSet right to right + \"-R\".\nSet left to left + \"!\".\nShow base.\nShow left.\nShow right.\n",
        ),
        (
            "an_older_view_appended_after_a_newer_one",
            "## Main\nLet mutable a be \"\".\nSet a to a + \"x\".\nLet b be a.\nSet a to a + \"yy\".\nLet mutable c be b.\n\
             Set c to c + \"z\".\nShow a.\nShow b.\nShow c.\n",
        ),
        (
            "a_text_appended_to_itself",
            "## Main\nLet mutable s be \"\".\nSet s to s + \"ab\".\nSet s to s + s.\nSet s to s + s.\nShow s.\nShow length of s.\n",
        ),
        (
            "every_step_of_a_loop_pushed_into_a_list",
            "## Main\nLet mutable s be \"\".\nLet steps be a new Seq of Text.\nLet mutable i be 1.\nWhile i <= 6:\n    \
             Set s to s + \"{i}\".\n    Push s to steps.\n    Set i to i + 1.\nRepeat for t in steps:\n    Show t.\n",
        ),
        (
            "an_append_of_numbers_and_interpolation",
            "## Main\nLet mutable s be \"n=\".\nLet mutable i be 1.\nWhile i <= 4:\n    Set s to s + i + \",\".\n    \
             Set i to i + 1.\nSet s to s + \"{true}\".\nShow s.\n",
        ),
        (
            "a_literal_is_never_extended_in_place",
            "## Main\nLet mutable a be \"lit\".\nLet mutable b be \"lit\".\nSet a to a + \"A\".\nSet b to b + \"B\".\nShow a.\nShow b.\n",
        ),
        (
            "an_append_inside_a_function_to_its_parameter",
            "## To tagged(s: Text, n: Int) -> Text:\n    Let mutable out be s.\n    Let mutable i be 1.\n    While i <= n:\n        \
             Set out to out + \"<\" + i + \">\".\n        Set i to i + 1.\n    Return out.\n\n\
             ## Main\nLet base be tagged(\"\", 2).\nShow tagged(base, 3).\nShow base.\n",
        ),
    ]
}

#[test]
fn every_alias_of_an_appended_text_keeps_its_value_on_every_tier() {
    let mut failures = Vec::new();
    for (name, src) in rows() {
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        let wasm = run_outcome(src)
            .map(|r| (r.output, r.failure))
            .unwrap_or_else(|why| (String::new(), Some(why)));
        let t = |s: &str| s.trim_end().to_string();
        let tiers = [
            (t(&tw.output), tw.error.clone()),
            (t(&vm.output), vm.error.clone()),
            (t(&wasm.0), wasm.1.clone()),
        ];
        if tiers[0].1.is_some() || tiers[1] != tiers[0] || tiers[2] != tiers[0] {
            failures.push(format!(
                "── {name}\n  tree-walker {:?}\n  VM {:?}\n  wasm {:?}\n{src}",
                tiers[0], tiers[1], tiers[2]
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} row(s) diverge:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// A loop appending `pieces` pieces of `<div>x</div>` (12 bytes each) — how far the heap's bump
/// pointer moved.
fn heap_after_appends(pieces: usize) -> u32 {
    let src = format!(
        "## Main\nLet mutable out be \"\".\nLet mutable i be 1.\nWhile i <= {pieces}:\n    \
         Set out to out + \"<div>\" + \"x\" + \"</div>\".\n    Set i to i + 1.\nShow length of out.\n"
    );
    let run = run_outcome(&src).unwrap_or_else(|why| panic!("the append loop runs: {why}"));
    assert_eq!(run.failure, None);
    assert_eq!(
        run.output.trim(),
        (pieces * 12).to_string(),
        "the loop builds the whole Text"
    );
    run.heap_ptr.expect("a program that builds Text has a heap")
}

#[test]
fn a_loop_of_appends_allocates_memory_linear_in_the_text_it_builds() {
    let (small, large) = (1_000, 8_000);
    let (at_small, at_large) = (heap_after_appends(small), heap_after_appends(large));
    let growth = f64::from(at_large) / f64::from(at_small);
    let size = (large / small) as f64;
    // Linear: headers are 16 bytes an append and the buffer doubles, so the heap grows with the Text
    // (8x here). Copying the whole Text on every append grows with its square (64x).
    assert!(
        growth <= 1.5 * size,
        "8x the pieces moved the heap {growth:.1}x ({at_small} -> {at_large} bytes): an append copies the whole Text again"
    );
}
