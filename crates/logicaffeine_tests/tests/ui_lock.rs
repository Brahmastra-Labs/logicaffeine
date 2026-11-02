//! ★ LOCK — `ui.lg`: the arithmetic and the wording a page needs, in one place. ★
//!
//! ★ WHAT THIS MODULE DELIBERATELY IS NOT. The audit asked for "toast, modal, pagination". Two of
//! those three are not functions: a toast and a modal are a VIEW plus state plus a dismissal
//! action, which in this language is a `## Component`, not a `## To`. Shipping them as functions
//! would mean returning markup as `Text` — the one shape the framework spends its whole design
//! avoiding, because it is how a page starts concatenating HTML and how `{raw}` becomes routine.
//! So this module holds the part that genuinely IS a function: the page arithmetic every list
//! view gets wrong by one, and the wording every list view writes by hand.
//!
//! ★ AND THE ARITHMETIC IS WHERE THE BUGS ARE. Pagination is off-by-one country: LOGOS sequences
//! are 1-INDEXED, a page count is a ceiling division, an empty collection still has a page to
//! show, and the last page is short. Each of those is a line of code and each is wrong in
//! somebody's shipped app right now.

use logicaffeine_compile::compile::tw_outcome;

fn shows(subject: &str, body: &str) -> String {
    let src = format!("## Main\n{body}\n");
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "`{subject}` must run:\n{src}");
    out.output.trim().to_string()
}

#[test]
fn a_page_count_is_a_ceiling_division() {
    assert_eq!(shows("exact", "Show pageCount(20, 10)."), "2");
    assert_eq!(shows("short last page", "Show pageCount(21, 10)."), "3");
    assert_eq!(shows("one short page", "Show pageCount(3, 10)."), "1");
}

#[test]
fn an_empty_collection_still_has_one_page() {
    // ★ NOT ZERO. A list view with no items still renders — it shows "nothing here" — and a page
    // count of 0 makes `clampPage` answer 0, which indexes nothing and reads as a bug to whoever
    // is looking at an empty screen wondering why.
    assert_eq!(shows("empty", "Show pageCount(0, 10)."), "1");
    assert_eq!(shows("clamped", "Show clampPage(1, 0, 10)."), "1");
}

#[test]
fn a_page_number_is_clamped_into_range() {
    assert_eq!(shows("below", "Show clampPage(0, 35, 10)."), "1");
    assert_eq!(shows("above", "Show clampPage(9, 35, 10)."), "4");
    assert_eq!(shows("inside", "Show clampPage(2, 35, 10)."), "2");
}

#[test]
fn page_bounds_are_one_indexed_and_the_last_page_is_short() {
    // ★ 1-INDEXED, because LOGOS sequences are. A module that answered 0-based bounds would be
    // correct in isolation and wrong at every call site, which is the worst kind of correct.
    assert_eq!(
        shows(
            "first page",
            "Show pageStart(1, 10).\nShow pageEnd(1, 10, 35)."
        ),
        "1\n10"
    );
    assert_eq!(
        shows(
            "second page",
            "Show pageStart(2, 10).\nShow pageEnd(2, 10, 35)."
        ),
        "11\n20"
    );
    assert_eq!(
        shows(
            "short last",
            "Show pageStart(4, 10).\nShow pageEnd(4, 10, 35)."
        ),
        "31\n35"
    );
    // An empty collection: the range is deliberately EMPTY (start > end), so a caller's loop
    // simply does not run rather than reading item 1 of nothing and trapping.
    assert_eq!(
        shows(
            "empty range",
            "Show pageStart(1, 10).\nShow pageEnd(1, 10, 0)."
        ),
        "1\n0"
    );
}

#[test]
fn truncating_marks_that_it_cut() {
    // A cut with no mark is a lie about the content: the reader cannot tell "Ada" from "Ada…".
    assert_eq!(shows("cut", "Show truncate(\"hello world\", 5)."), "hell…");
    assert_eq!(shows("fits", "Show truncate(\"hello\", 5)."), "hello");
    assert_eq!(shows("just over", "Show truncate(\"hello!\", 5)."), "hell…");
    assert_eq!(shows("tiny budget", "Show truncate(\"hello\", 1)."), "…");
}

#[test]
fn plural_agrees_with_its_number() {
    assert_eq!(
        shows("one", "Show plural(1, \"item\", \"items\")."),
        "1 item"
    );
    assert_eq!(
        shows("many", "Show plural(2, \"item\", \"items\")."),
        "2 items"
    );
    assert_eq!(
        shows("none", "Show plural(0, \"item\", \"items\")."),
        "0 items"
    );
}

#[test]
fn initials_take_the_first_letter_of_each_word() {
    assert_eq!(shows("two names", "Show initials(\"Ada Lovelace\")."), "AL");
    assert_eq!(shows("one name", "Show initials(\"Ada\")."), "A");
    assert_eq!(
        shows("extra spaces", "Show initials(\"  Ada   Lovelace \")."),
        "AL"
    );
    assert_eq!(shows("empty", "Show \"[{initials(\\\"\\\")}]\"."), "[]");
}

#[test]
fn the_helpers_can_actually_fail() {
    // ★ ANTI-VACUITY. If `pageCount` always answered 1 every count assertion above about an empty
    // collection would still pass, and if `truncate` always returned its input the "fits" case
    // would too. Prove each distinguishes.
    assert_ne!(
        shows("a", "Show pageCount(20, 10)."),
        shows("b", "Show pageCount(0, 10).")
    );
    assert_ne!(
        shows("c", "Show truncate(\"hello world\", 5)."),
        shows("d", "Show truncate(\"hello\", 5).")
    );
}
