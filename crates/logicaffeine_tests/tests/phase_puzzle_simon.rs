//! End-to-end spec for ONE puzzle — PuzzleBaron "Simon's adventure holidays"
//! (`puzzles/3cf932ef3e458bf4fcba3081d831f5ae.json`). We thread it the whole
//! way: every clue, the option declarations, the all-different rule, and the
//! goal must parse to FIRST-ORDER LOGIC with ZERO meaning loss — pinned here as
//! the exact logical form, not merely `Ok`. The grid solver (in
//! logicaffeine_nano, Z3-gated) consumes exactly this FOL.
//!
//! The 4×4 grid: years {2001,2002,2003,2004}, friends {Bill,Lillie,Neal,Yvonne},
//! activities {cycling,hunting,kayaking,skydiving}, states
//! {Connecticut,Florida,Kentucky,Maine}. Unique solution:
//!   2001/Lillie/cycling/Kentucky · 2002/Neal/hunting/Florida
//!   2003/Yvonne/kayaking/Maine   · 2004/Bill/skydiving/Connecticut

use logicaffeine_language::compile;

fn fol(s: &str) -> String {
    compile(s).unwrap_or_else(|e| panic!("expected OK for {s:?}, got {e:?}"))
}

// ── The six clues — exact FOL ────────────────────────────────────────────────

#[test]
fn clue1_of_pair_with_distinctness() {
    // "Of A and B, one P the other Q" — the A≠B presupposition (`¬x = y`) is
    // load-bearing: without it the puzzle has 3 solutions, with it exactly 1.
    let out = fol("Of the hunting vacation and the 2004 holiday, one was with Neal and the other was in Connecticut.");
    assert_eq!(
        out,
        "∃x(∃y(((Vacation(x) ∧ Hunt(x)) ∧ (2004_holiday(y) ∧ (¬x = y ∧ ((P(With(x, Neal)) ∧ P(In(y, Connecticut))) ∨ (P(With(y, Neal)) ∧ P(In(x, Connecticut)))))))))"
    );
}

#[test]
fn clue2_identity_of_two_descriptions() {
    let out = fol("The Florida trip was the hunting trip.");
    assert_eq!(
        out,
        "∃x((((Trip(x) ∧ Florida(x)) ∧ ∀y(((Trip(y) ∧ Florida(y)) → y = x))) ∧ (Hunt(x) ∧ Trip(x))))"
    );
}

#[test]
fn clue3_neither_nor() {
    let out = fol("Neither the holiday with Bill nor the Florida vacation is the 2001 trip.");
    assert_eq!(
        out,
        "1) ∃x(((Holiday(x) ∧ With(x, Bill)) ∧ ¬2001_trip(x)))\n2) ∃y(((Vacation(y) ∧ Florida(y)) ∧ ¬2001_trip(y)))"
    );
}

#[test]
fn clue4_negated_copula_with_pp_subject() {
    let out = fol("The holiday with Yvonne wasn't in Kentucky.");
    assert_eq!(
        out,
        "∃x((((Holiday(x) ∧ With(x, Yvonne)) ∧ ∀y(((Holiday(y) ∧ With(y, Yvonne)) → y = x))) ∧ ¬P(In(x, Kentucky))))"
    );
}

#[test]
fn clue5_of_pair_keeps_skydiving_modifier() {
    // Regression for the modifier-drop bug: "the skydiving trip" (verb-ambiguous
    // head "trip") must stay `Skydive_trip(x)`, NOT collapse to bare `Trip`.
    let out = fol("Of the skydiving trip and the Maine holiday, one was in 2003 and the other was with Bill.");
    assert_eq!(
        out,
        "∃x(∃y(((Trip(x) ∧ Skydive(x)) ∧ ((Holiday(y) ∧ Maine(y)) ∧ (¬x = y ∧ ((P(In(x, 2003)) ∧ P(With(y, Bill))) ∨ (P(In(y, 2003)) ∧ P(With(x, Bill)))))))))"
    );
}

#[test]
fn clue6_negated_identity() {
    let out = fol("The 2003 holiday wasn't the cycling trip.");
    assert_eq!(
        out,
        "∃x(((2003_holiday(x) ∧ ∀y((2003_holiday(y) → y = x))) ∧ ¬(Cycle(x) ∧ Trip(x))))"
    );
}
