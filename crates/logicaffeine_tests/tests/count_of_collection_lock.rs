//! ★ LOCK — `the count of <collection>` is its size, on every collection. ★
//!
//! `the count of` is one English phrase with two readings: over a COLLECTION it is that
//! collection's size, and over a literal SELECTOR it is how many elements of the page match. The
//! reference tier settles which by looking at the value — the only honest way, since neither
//! reading is decidable from the sentence alone.
//!
//! It settled it by asking whether the value was a LIST. So a Set and a Map — collections by every
//! other measure in the language — fell through to the selector reading, where a set's display form
//! (`{5, 6}`) matches no element on any page, and the answer came back **0**.
//!
//! ★ ZERO IS THE WORST POSSIBLE WRONG ANSWER HERE, because it is a plausible one. An empty
//! collection really does count 0, so nothing about the result looks wrong — a guard reading
//! `If the count of seen is 0:` takes the empty branch forever, on a set with a thousand things in
//! it. There is no program for which the old answer was right: a Set has no selector reading at all.
//!
//! ★ A TEXT IS STILL A SELECTOR, deliberately and locked below. `the count of ".row"` is the page
//! query the whole test vocabulary rests on, and reading it as a string length would break every
//! page test in the suite while looking like a tidy-up.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn tw(src: &str) -> String {
    let o = tw_outcome(src);
    assert_eq!(o.error, None, "tree-walker refused:\n{src}");
    o.output.trim_end().to_string()
}

#[test]
fn the_count_of_a_set_is_its_size() {
    assert_eq!(
        tw("## Main\nLet mutable s be a new Set of Int.\nAdd 5 to s.\nAdd 6 to s.\nShow the count of s.\n"),
        "2"
    );
}

#[test]
fn the_count_of_a_map_is_its_size() {
    assert_eq!(
        tw("## Main\nLet mutable m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nSet item \"b\" of m to 2.\nShow the count of m.\n"),
        "2"
    );
}

#[test]
fn the_count_of_a_sequence_is_its_length() {
    // Already correct — held here so the widening cannot cost the case that worked.
    assert_eq!(
        tw("## Main\nLet xs be [1, 2, 3].\nShow the count of xs.\n"),
        "3"
    );
}

#[test]
fn an_empty_collection_counts_zero_and_a_full_one_does_not() {
    // ★ THE PAIRING THAT GIVES THE FIX ITS TEETH: 0 was the OLD answer for every set, so a lock
    // asserting only the empty case would have passed against the bug it exists to close.
    assert_eq!(
        tw("## Main\nLet mutable s be a new Set of Int.\nShow the count of s.\nAdd 7 to s.\nShow the count of s.\n"),
        "0\n1"
    );
}

#[test]
fn the_count_agrees_with_length_on_every_collection() {
    // The two spellings are the same question, so they must never answer differently — which is a
    // stronger claim than either number on its own, and the one a reader relies on.
    for build in [
        "Let mutable c be a new Set of Int.\nAdd 1 to c.\nAdd 2 to c.\nAdd 3 to c.\n",
        "Let mutable c be a new Map of Text to Int.\nSet item \"a\" of c to 1.\nSet item \"b\" of c to 2.\n",
        "Let c be [9, 8, 7, 6].\n",
    ] {
        let src = format!("## Main\n{build}Show the count of c.\nShow length of c.\n");
        let out = tw(&src);
        let (a, b) = out.split_once('\n').expect("two lines");
        assert_eq!(a, b, "`the count of` and `length of` disagree:\n{src}");
    }
}

#[test]
fn a_literal_selector_is_still_a_page_query() {
    // ★ THE OTHER READING, held so the widening cannot quietly swallow it: a Text names elements of
    // the page, and `the count of ".row"` must stay that question rather than becoming a string
    // length. Nothing is mounted, so no element matches and the answer is 0 — by the SELECTOR
    // reading, which a string-length reading would have answered 4.
    assert_eq!(tw("## Main\nShow the count of \".row\".\n"), "0");
}

#[test]
fn the_bytecode_tier_refuses_a_page_query_by_name() {
    // ★ IT REFUSED WITH "vm: unsupported expression" — the generic catch-all every unhandled
    // expression falls into, which tells the reader nothing about which sentence it means or what
    // to write instead. A page query really is a reference-tier form (it asks about a live
    // document), so the refusal is correct; being anonymous was not.
    let o = vm_outcome("## Main\nShow the count of \".row\".\n");
    let msg = o.error.expect("the bytecode tier must refuse a page query");
    assert!(
        msg.contains("count") || msg.contains("page"),
        "the refusal must name the sentence it refused, got: {msg}"
    );
    assert!(
        !msg.contains("unsupported expression"),
        "the generic catch-all says nothing about what was refused: {msg}"
    );
}
