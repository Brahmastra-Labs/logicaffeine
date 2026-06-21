#![cfg(feature = "verification")]
//! A grid whose CLUES use occasion SYNONYMS for the declared row sort: the domain
//! is declared with "trip", but clues say "vacation" / "holiday". The same entity
//! can be a trip, a vacation, or a holiday (an OCCASION soft type — the modifier
//! does the referring), so the solve must treat the synonym-headed descriptions as
//! ranging over the one declared row domain. This is NOT global synonymy ("a work
//! trip" is not a vacation) — it is the occasion soft-type principle applied where
//! a category modifier identifies the row.

use logicaffeine_compile::answer_question;

const GRID: &str = "## Theorem: Trips\n\
     Given: Alpha is a trip.\n\
     Given: Beta is a trip.\n\
     Given: Alpha is not Beta.\n\
     Given: Every trip is in Florida or in Maine.\n\
     Given: Every trip is in 2003 or in 2004.\n\
     Given: Exactly one trip is in Florida.\n\
     Given: Exactly one trip is in 2003.\n\
     Given: Alpha is in Florida.\n";

/// CONTROL: head noun is the declared "trip" — must solve (sanity for the harness).
#[test]
fn control_trip_in_florida_links() {
    let q = format!("{GRID}Given: The trip in Florida is in 2003.\nProve: Who is in 2004?\nProof: Auto.\n");
    let ans = answer_question(&q).expect("answerable");
    assert!(ans.contains(&"Beta".to_string()), "control: Beta in 2004; got: {ans:?}");
}

/// ISOLATION (occasion-domain only): PP-form modifier ("the vacation IN Florida"),
/// so no label→In convergence is needed — only the occasion synonym "vacation"
/// must range over the declared trip domain.
#[test]
fn pp_form_vacation_links_to_florida_trip() {
    let q = format!("{GRID}Given: The vacation in Florida is in 2003.\nProve: Who is in 2004?\nProof: Auto.\n");
    let ans = answer_question(&q).expect("answerable");
    assert!(ans.contains(&"Beta".to_string()), "vacation(PP): Beta in 2004; got: {ans:?}");
}

/// The grid plus category DECLARATIONS, so a state LABEL ("the Florida X") converges
/// to `In(x, Florida)` — the form the real JSON clues use.
const GRID_DECL: &str = "## Theorem: Trips\n\
     Given: Alpha is a trip.\n\
     Given: Beta is a trip.\n\
     Given: Alpha is not Beta.\n\
     Given: Florida and Maine are two different states.\n\
     Given: 2003 and 2004 are two different years.\n\
     Given: Every trip is in Florida or in Maine.\n\
     Given: Every trip is in 2003 or in 2004.\n\
     Given: Exactly one trip is in Florida.\n\
     Given: Exactly one trip is in 2003.\n\
     Given: Alpha is in Florida.\n";

/// LABEL-form modifier under a synonym head: "the Florida VACATION" must converge to
/// `In(x, Florida)` exactly as "the Florida trip" does, then link to the row domain.
#[test]
fn label_form_vacation_converges_and_links() {
    let q = format!("{GRID_DECL}Given: The Florida vacation is in 2003.\nProve: Who is in 2004?\nProof: Auto.\n");
    let ans = answer_question(&q).expect("answerable");
    assert!(ans.contains(&"Beta".to_string()), "Florida vacation(label): Beta in 2004; got: {ans:?}");
}

/// FUSED year label under a synonym head: "the 2003 HOLIDAY" must un-fuse to
/// `In(x, 2003)` and link, forcing the Florida trip into 2003 → the other into 2004.
#[test]
fn fused_year_holiday_converges_and_links() {
    let q = format!("{GRID_DECL}Given: The 2003 holiday is in Florida.\nProve: Who is in Maine?\nProof: Auto.\n");
    let ans = answer_question(&q).expect("answerable");
    assert!(ans.contains(&"Beta".to_string()), "2003 holiday(fused): Beta in Maine; got: {ans:?}");
}
