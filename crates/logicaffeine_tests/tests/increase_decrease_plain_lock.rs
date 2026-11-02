//! ★ LOCK — `Increase n by 3.` on an ordinary name. ★
//!
//! `Increase`/`Decrease` is in the action vocabulary, it is the first sentence anyone reaches for to
//! move a counter, and on a PLAIN NAME it did not parse at all — on either tier. It worked on a
//! record field (`Increase c's points by 10.`, the CRDT form) and on the retiring dialect's signals,
//! and nowhere else.
//!
//! ★ THE ERROR WAS THE WORST PART: "This construction needs the word 'field access (e.g., 'x's
//! count')' here". A reader who wrote a perfectly ordinary sentence was told to write a field access
//! — sent to add a record they do not have, for a counter that is just a number.
//!
//! ★ AND IT WAS ON THE CRITICAL PATH. The dialect is being retired; a signal's `Increase` goes with
//! it. Without this, retiring the surface would have SILENTLY REMOVED a documented capability, with
//! the only sign being a parse error in somebody's program long afterwards. Found by migrating the
//! signal tests to LGX twins, which is what that migration is for.
//!
//! Locked in PLAIN LOGOS, because that is where the gap is reachable — a counter a routine bumps has
//! no frontend anywhere near it. The component case is here too, for the reason below.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// Both tiers run it and agree.
fn both(src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM:\n{src}");
    assert_eq!(tw.output.trim_end(), expected, "tree-walker answer:\n{src}");
    assert_eq!(vm.output.trim_end(), expected, "VM answer:\n{src}");
}

#[test]
fn increase_moves_a_plain_number_up() {
    both(
        "## Main\nLet mutable n be 5.\nIncrease n by 3.\nShow n.\n",
        "8",
    );
}

#[test]
fn decrease_moves_a_plain_number_down() {
    both(
        "## Main\nLet mutable n be 5.\nDecrease n by 1.\nShow n.\n",
        "4",
    );
}

#[test]
fn the_two_directions_are_not_the_same_direction() {
    // ★ A sign error would pass either test above taken alone if the reader happened to pick
    // symmetric numbers; run both from one start so the pair pins the DIRECTION.
    both(
        "## Main\nLet mutable n be 10.\nIncrease n by 4.\nShow n.\nDecrease n by 6.\nShow n.\n",
        "14\n8",
    );
}

#[test]
fn the_step_is_an_expression_not_only_a_literal() {
    // `by <expr>` has to parse the whole expression — a version reading a single number token
    // would work on every example anybody writes down and fail on the first real one.
    both(
        "## Main\nLet mutable n be 5.\nLet k be 2.\nIncrease n by k * 3.\nShow n.\n",
        "11",
    );
}

#[test]
fn it_steps_a_real_and_keeps_the_fraction() {
    // A Real must not be quietly truncated to the whole number the step looked like.
    both(
        "## Main\nLet mutable r be 1.5.\nIncrease r by 0.25.\nShow r.\n",
        "1.75",
    );
}

#[test]
fn it_works_on_a_local_inside_a_function() {
    both(
        "## To run () -> Int:\n\
        \x20   Let mutable n be 1.\n\
        \x20   Increase n by 41.\n\
        \x20   Return n.\n\
         \n\
         ## Main\n\
         Show run().\n",
        "42",
    );
}

#[test]
fn it_works_on_an_outer_name_a_function_writes() {
    // ★ THE PROMOTION PATH. A function whose only use of an outer name is a WRITE is exactly the
    // shape `vm_global_promotion_lock` was written for — a write the promotion walk cannot see
    // lands on a global that was never created. A new way to spell a write has to be visible to it.
    both(
        "## To bump ():\n\
        \x20   Increase total by 1.\n\
         \n\
         ## Main\n\
         Let mutable total be 0.\n\
         bump().\n\
         bump().\n\
         Show total.\n",
        "2",
    );
}

#[test]
fn the_record_field_form_still_means_the_crdt_increment() {
    // ★ THE OTHER READING, held so widening the plain case cannot swallow it: `Increase c's points
    // by 3` is the CRDT counter increment and must keep meaning that.
    both(
        "## A C has:\n\
        \x20   a points, which is Int.\n\
         \n\
         ## Main\n\
         Let mutable c be a new C with points 5.\n\
         Increase c's points by 3.\n\
         Show c's points.\n",
        "8",
    );
}

#[test]
fn a_turn_that_increases_a_state_field_repaints_the_page() {
    // ★★ THE HALF THAT WOULD HAVE SHIPPED INERT, and the reason this test is not only plain LOGOS.
    // A component's writes are collected so the fine-grained patcher knows which sites to update; a
    // write spelled a NEW way is invisible to that collection unless it is taught. `lgx::deps`
    // records exactly this failure for `Toggle`: "a write the compiler cannot see is a site that
    // never updates, and the page looks broken while the state is perfectly correct."
    //
    // So this asserts the PAGE, not the field: parsing `Increase` correctly and never repainting is
    // a pass on every other test in this file.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let count be 5.\n\
       \n\
       Actions:\n\
      \x20   To act:\n\
      \x20       Increase count by 3.\n\
      \x20       Decrease count by 1.\n\
       \n\
       View:\n\
      \x20   <main><p id=\"c\">{count}</p><button id=\"b\" on-click={act}>x</button></main>\n\
       \n\
       ## Main\n\
       Mount App.\n\
       Click \"#b\".\n";

    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run the page:\n{src}");
    let tw_doc = dom::render();
    assert!(
        tw_doc.contains(">7<"),
        "the page must show the stepped value:\n{tw_doc}"
    );
    assert!(
        !tw_doc.contains(">5<"),
        "the old value must be gone:\n{tw_doc}"
    );

    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must run the page too:\n{src}");
    assert_eq!(
        tw_doc,
        dom::render(),
        "the two tiers rendered the turn differently"
    );
}

#[test]
fn the_step_means_exactly_what_the_assignment_it_desugars_to_means() {
    // ★ THIS TEST REPLACES A WRONG ONE OF MINE, and the replacement is the stronger claim.
    //
    // I first asserted that `Increase n by "three"` must be REFUSED, reasoning that adding a Text to
    // a number has no plausible answer. Measured, it answers `"5three"` — and so does
    // `Set n to n + "three"`, which is what the sentence desugars to. My expectation was about `+`,
    // not about `Increase`, and `+` on a number and a Text is a decision this language already made
    // somewhere else.
    //
    // So the property worth locking is not a refusal I guessed at: it is that the two spellings
    // agree. `Increase` must not be stricter than the `Set` it means, or the language would hold two
    // different definitions of `+` and an author would have to know which sentence invoked which.
    // A refusal test would have frozen that divergence in place.
    for (step, expected) in [("3", "8"), ("0 - 2", "3"), ("\"three\"", "5three")] {
        let via_increase = tw_outcome(&format!(
            "## Main\nLet mutable n be 5.\nIncrease n by {step}.\nShow n.\n"
        ));
        let via_set = tw_outcome(&format!(
            "## Main\nLet mutable n be 5.\nSet n to n + {step}.\nShow n.\n"
        ));
        assert_eq!(
            via_increase.error, via_set.error,
            "the two spellings disagree about `{step}`"
        );
        assert_eq!(
            via_increase.output.trim_end(),
            via_set.output.trim_end(),
            "step `{step}`"
        );
        assert_eq!(via_increase.output.trim_end(), expected, "step `{step}`");
    }
}
