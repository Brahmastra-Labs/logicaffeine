//! ★ LOCK — a name a function only WRITES is still a name that function references. ★
//!
//! The bytecode compiler keeps Main's top-level `Let` bindings in registers, and promotes one to
//! a GLOBAL slot only when some function or closure body refers to it. That promotion is driven
//! by a free-variable walk, and the walk used to look at a `Set`'s VALUE and not its TARGET —
//! so a function whose only use of an outer name was `Set x to …` left `x` unpromoted, and the
//! write then landed on a global that had never been created:
//!
//! ```text
//! Undefined variable: x
//! ```
//!
//! The shape is not exotic. It is a counter a routine bumps, a flag a routine raises, a cache a
//! routine fills — anything written in one place and read in another, or written and not read at
//! all. It reached me through the frontend (a component whose only use of its mount point was to
//! record it), but nothing about it is about the frontend, so it is pinned here in plain LOGOS
//! where the next person to trip it will be.
//!
//! The tree-walker has always got this right, which is what makes it the reference: every test
//! below runs on BOTH tiers and demands they agree.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Run `src` on both tiers: neither may fail, and both must print exactly `want`.
///
/// Compared line by line, the way every other cross-tier lock in this crate does it, so a
/// trailing newline is never the thing a soundness test turns on.
fn both(src: &str, want: &[&str]) {
    let tw = tw_outcome(src);
    let vm = vm_outcome(src);
    assert_eq!(
        tw.error, None,
        "the tree-walker must run it cleanly:\n{src}"
    );
    assert_eq!(vm.error, None, "the VM must run it cleanly:\n{src}");
    assert_eq!(
        tw.output.lines().collect::<Vec<_>>(),
        want,
        "the tree-walker printed something else:\n{src}"
    );
    assert_eq!(
        vm.output.lines().collect::<Vec<_>>(),
        want,
        "the VM printed something else:\n{src}"
    );
}

#[test]
fn a_function_that_only_writes_an_outer_name_still_binds_it() {
    // `touch` never READS `g`. Before the fix the VM never made `g` a global, and the write
    // failed with "Undefined variable: g" while the tree-walker ran it happily.
    let src = "## To touch (p: Int):\n\
        \x20   Set g to p.\n\
        \n## Main\n\
        Let mutable g be 0.\n\
        Call touch with 5.\n\
        Show g.\n";
    both(src, &["5"]);
}

#[test]
fn a_write_only_global_survives_when_nothing_ever_reads_it() {
    // The degenerate case: written, never read. It must still be a legal program — the write
    // has to land somewhere, and "somewhere" cannot be a slot that was never made.
    let src = "## To touch (p: Int):\n\
        \x20   Set g to p.\n\
        \n## Main\n\
        Let mutable g be 0.\n\
        Call touch with 5.\n\
        Show \"done\".\n";
    both(src, &["done"]);
}

#[test]
fn one_function_writes_what_another_reads() {
    // The ordinary shape this bug hides in: a writer and a reader that never meet.
    let src = "## To store (p: Int):\n\
        \x20   Set slot to p.\n\
        \n## To recall () -> Int:\n\
        \x20   Return slot.\n\
        \n## Main\n\
        Let mutable slot be 0.\n\
        Call store with 41.\n\
        Show recall() + 1.\n";
    both(src, &["42"]);
}

#[test]
fn a_write_inside_a_branch_binds_the_name_too() {
    // The walk has to descend into nested blocks, or a conditional write is invisible to it.
    let src = "## To maybe (p: Int):\n\
        \x20   If p is greater than 0:\n\
        \x20       Set seen to p.\n\
        \n## Main\n\
        Let mutable seen be 0.\n\
        Call maybe with 7.\n\
        Show seen.\n";
    both(src, &["7"]);
}

#[test]
fn a_write_inside_a_loop_binds_the_name_too() {
    let src = "## To total (n: Int):\n\
        \x20   For each i from 1 to n:\n\
        \x20       Set running to running + i.\n\
        \n## Main\n\
        Let mutable running be 0.\n\
        Call total with 4.\n\
        Show running.\n";
    both(src, &["10"]);
}

#[test]
fn a_function_local_of_the_same_name_is_not_the_outer_one() {
    // The other half of the rule: a name the function BINDS itself is not a reference to the
    // outer one. `bump`'s `g` is its own, so Main's `g` must come back untouched.
    let src = "## To bump ():\n\
        \x20   Let mutable g be 100.\n\
        \x20   Set g to g + 1.\n\
        \x20   Show g.\n\
        \n## Main\n\
        Let mutable g be 7.\n\
        Call bump.\n\
        Show g.\n";
    both(src, &["101", "7"]);
}

#[test]
fn a_closure_captures_by_value_so_its_write_does_not_escape() {
    // The BOUNDARY of the rule above, and the reason it stops at functions.
    //
    // A `## To` body writing an outer name is a reference to that name, which is what the
    // promotion walk had to learn. A CLOSURE is different: it captures by value, so `tally`
    // inside is a copy and the write never reaches Main's binding. Both tiers agree on that, and
    // pinning it here is what stops someone "fixing" the promotion walk to cover closures too —
    // which would silently change closure semantics rather than fix a bug.
    let src = "## Main\n\
        Let mutable tally be 0.\n\
        Let accrue be (n: Int) ->:\n\
        \x20   Set tally to tally + n.\n\
        Call accrue with 3.\n\
        Call accrue with 4.\n\
        Show tally.\n";
    both(src, &["0"]);
}

#[test]
fn a_written_text_global_keeps_its_type_across_the_boundary() {
    // Promotion carries the binding's type across the function boundary; a Text global written
    // from a function must still be a Text when Main reads it back.
    let src = "## To greet (who: Text):\n\
        \x20   Set message to \"hello \" + who.\n\
        \n## Main\n\
        Let mutable message be \"\".\n\
        Call greet with \"ada\".\n\
        Show message.\n";
    both(src, &["hello ada"]);
}

#[test]
fn a_written_list_global_keeps_its_contents_across_the_boundary() {
    // A composite is the case where getting promotion wrong is most expensive, because the
    // value crossing the boundary is a whole structure rather than a scalar.
    let src = "## To stockUp ():\n\
        \x20   Let mutable built be a new Seq of Int.\n\
        \x20   Push 1 to built.\n\
        \x20   Push 2 to built.\n\
        \x20   Set xs to built.\n\
        \n## Main\n\
        Let mutable xs be a new Seq of Int.\n\
        Call stockUp.\n\
        Show length of xs.\n";
    both(src, &["2"]);
}

#[test]
fn writes_from_two_functions_reach_the_same_binding() {
    // Two writers, one binding: whichever ran last is what is there. If each write had gone to
    // a slot of its own this would print 1.
    let src = "## To writerA ():\n\
        \x20   Set n to 1.\n\
        \n## To writerB ():\n\
        \x20   Set n to 2.\n\
        \n## Main\n\
        Let mutable n be 0.\n\
        Call writerA.\n\
        Call writerB.\n\
        Show n.\n";
    both(src, &["2"]);
}
