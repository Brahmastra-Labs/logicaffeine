//! Discourse-level spec: a definite-description LABEL un-fuses to the SAME
//! relation its prepositional-phrase counterpart produces, with the relation
//! chosen by the item's DECLARED category.
//!
//! Two surface forms of one fact must CONVERGE so the prover connects them:
//!   PP form:    "the holiday was in 2003"   → Holiday(x) ∧ In(x, 2003)
//!   LABEL form: "the 2003 holiday was fun"  → Holiday(x) ∧ In(x, 2003)
//!
//! The un-fusing is THEOREM/context dependent — it fires ONLY when an earlier
//! sentence in the SAME input DECLARES the item's category ("2003 is a year",
//! coordinated as "…are four different years"). A single sentence with no
//! declaration MUST stay fused (`2003_holiday`). This is what keeps the
//! single-sentence puzzle pins (phase_puzzle_simon) untouched.

use logicaffeine_language::compile;

fn fol(s: &str) -> String {
    compile(s).unwrap_or_else(|e| panic!("expected OK for {s:?}, got {e:?}"))
}

// ── TEMPORAL: a declared YEAR drives the cardinal label → In(x, <year>) ──────

#[test]
fn declared_year_unfuses_cardinal_label_to_in() {
    // The years are declared FIRST; the later "the 2003 holiday" must read its
    // category back out of the shared DRS and un-fuse to In(x, 2003), NOT the
    // fused `2003_holiday`.
    let out = fol("2001, 2002, 2003, and 2004 are four different years. The 2003 holiday was fun.");
    assert!(
        out.contains("In(x, 2003)"),
        "expected the label clause to un-fuse to In(x, 2003); got:\n{out}"
    );
    assert!(
        !out.contains("2003_holiday"),
        "the label must NOT stay fused once the year is declared; got:\n{out}"
    );
    // The head noun survives as its own predicate.
    assert!(out.contains("Holiday(x)"), "head noun Holiday(x) must survive; got:\n{out}");
}

// ── LOCATIVE: a declared STATE drives the proper-name label → In(x, <state>) ─

#[test]
fn declared_state_unfuses_proper_name_label_to_in() {
    let out = fol("Connecticut, Florida, Kentucky, and Maine are four different states. The Florida trip was fun.");
    assert!(
        out.contains("In(x, Florida)"),
        "expected the label clause to un-fuse to In(x, Florida); got:\n{out}"
    );
    assert!(
        !out.contains("Florida(x)"),
        "the label must NOT stay a bare predicate once the state is declared; got:\n{out}"
    );
    assert!(out.contains("Trip(x)"), "head noun Trip(x) must survive; got:\n{out}");
}

// ── PERSONAL: a declared FRIEND drives the proper-name label → With(x, <p>) ──

#[test]
fn declared_friend_unfuses_proper_name_label_to_with() {
    let out = fol("Bill, Lillie, Neal, and Yvonne are four different friends. The Bill trip was fun.");
    assert!(
        out.contains("With(x, Bill)"),
        "expected the label clause to un-fuse to With(x, Bill); got:\n{out}"
    );
    assert!(
        !out.contains("Bill(x)"),
        "the label must NOT stay a bare predicate once the friend is declared; got:\n{out}"
    );
    assert!(out.contains("Trip(x)"), "head noun Trip(x) must survive; got:\n{out}");
}

// ── BOTH-FORMS CONVERGENCE: label clause and PP clause yield IDENTICAL In(·,·) ─

#[test]
fn label_and_pp_forms_converge_on_identical_relation() {
    // In ONE input: declare the years, then state the fact in BOTH forms. The
    // label clause and the PP clause must BOTH carry In(·, 2003) with a
    // byte-identical object term, so the prover can unify them.
    let out = fol(
        "2001, 2002, 2003, and 2004 are four different years. \
         The 2003 holiday was fun. The holiday was in 2003.",
    );
    let count = out.matches("In(").filter(|_| true).count();
    let in_2003 = out.matches("In(x, 2003)").count();
    assert!(
        in_2003 >= 2,
        "both the label clause and the PP clause must emit In(x, 2003); \
         saw {in_2003} occurrences (total In( = {count}) in:\n{out}"
    );
}

// ── SAFETY: a single sentence with NO declaration stays FUSED ────────────────

#[test]
fn single_sentence_cardinal_label_stays_fused() {
    // No declarations in this input's DRS, so the label MUST stay fused.
    let out = fol("The 2003 holiday was fun.");
    assert!(
        out.contains("2003_holiday"),
        "with no declaration, the cardinal label must stay fused; got:\n{out}"
    );
    assert!(!out.contains("In(x, 2003)"), "no un-fuse without a declaration; got:\n{out}");
}

#[test]
fn single_sentence_proper_name_label_stays_predicate() {
    let out = fol("The Florida trip was fun.");
    assert!(
        out.contains("Florida(x)"),
        "with no declaration, the proper-name label stays a bare predicate; got:\n{out}"
    );
    assert!(!out.contains("In(x, Florida)"), "no un-fuse without a declaration; got:\n{out}");
}

#[test]
fn woodard_family_possessor_intact() {
    // "The Woodard family" is an UNDECLARED proper-name label; it must keep its
    // bare-predicate behavior (Family(x) ∧ Woodard(x)) — the possessor reading
    // is not disturbed by the category machinery.
    let out = fol("The Woodard family's house is large.");
    assert!(
        out.contains("Family") && out.contains("Woodard"),
        "the Woodard family possessor must stay intact; got:\n{out}"
    );
    assert!(!out.contains("With("), "an undeclared label must not gain a With relation; got:\n{out}");
}

#[test]
fn undeclared_count_label_stays_fused() {
    // "the 7 dwarves" with the 7 declared as something NON-temporal/locative/
    // personal (here: a vegetable) must NOT un-fuse — its category maps to no
    // preposition, so the count label stays fused.
    let out = fol("5, 6, 7, and 8 are four different vegetables. The 7 dwarves danced.");
    assert!(
        out.contains("7_dwarve"),
        "a label whose category maps to no preposition stays fused; got:\n{out}"
    );
    assert!(!out.contains("In(x, 7)") && !out.contains("With(x, 7)"), "no un-fuse for a non-mapping category; got:\n{out}");
}
