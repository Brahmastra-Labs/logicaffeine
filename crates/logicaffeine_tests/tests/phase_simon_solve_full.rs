#![cfg(feature = "verification")]
//! Scaling the SOLVE to a full logic grid: `answer_question` routes its per-cell
//! checks through the Z3 oracle (tense-erased — a grid is one static scenario),
//! which scales where the kernel prover does not. Same English-in/answer-out
//! contract; the engine under the question is what changes with the grid size.

use logicaffeine_compile::answer_question;

/// Build the bijection clauses for one category: a 4-way domain closure plus an
/// exactly-one per value — the general grid constraint, no puzzle knowledge.
fn category(prep: &str, items: &[&str]) -> String {
    let disjuncts = items
        .iter()
        .map(|i| format!("{prep} {i}"))
        .collect::<Vec<_>>()
        .join(" or ");
    let mut s = format!("Given: Every trip is {disjuncts}.\n");
    for i in items {
        s.push_str(&format!("Given: Exactly one trip is {prep} {i}.\n"));
    }
    s
}

/// Same bijection clauses for a PREDICATE category (activities: cycling, hunting…)
/// — "Every trip is cycling or hunting …" / "Exactly one trip is cycling.".
fn predicate_category(items: &[&str]) -> String {
    let disjuncts = items.join(" or ");
    let mut s = format!("Given: Every trip is {disjuncts}.\n");
    for i in items {
        s.push_str(&format!("Given: Exactly one trip is {i}.\n"));
    }
    s
}

/// THE FULL 4×4 SIMON PUZZLE, solved by a question. Four trips, four categories
/// (year, state, friend, activity), the bijection (closure + exactly-one per
/// value), and the SIX real PuzzleBaron clues — verbatim, parsed and converged to
/// In/With/Hunt constraints. Names are anchored to years to break the cell-name
/// symmetry; the clues then pin the rest. The unique solution puts the 2002 trip
/// (Beta) in Florida with Neal, hunting — so "Who is in Florida?" → Beta.
#[test]
fn full_simon_solves_by_question() {
    let mut doc = String::from(
        "## Theorem: Simon\nGiven: Alpha, Beta, Gamma, and Delta are four different trips.\n",
    );
    // Category DECLARATIONS — establish each item's category so the labels CONVERGE
    // ("the Florida trip" → In(x, Florida), not a fused Florida_trip). Without these
    // the clues never link to the grids.
    doc.push_str("Given: 2001, 2002, 2003, and 2004 are four different years.\n");
    doc.push_str("Given: Connecticut, Florida, Kentucky, and Maine are four different states.\n");
    doc.push_str("Given: Bill, Lillie, Neal, and Yvonne are four different friends.\n");
    doc.push_str("Given: Cycling, hunting, kayaking, and skydiving are four different activities.\n");
    doc.push_str(&category("in", &["2001", "2002", "2003", "2004"]));
    doc.push_str(&category("in", &["Connecticut", "Florida", "Kentucky", "Maine"]));
    doc.push_str(&category("with", &["Bill", "Lillie", "Neal", "Yvonne"]));
    doc.push_str(&predicate_category(&["cycling", "hunting", "kayaking", "skydiving"]));
    // Anchor names to years (break the name symmetry).
    doc.push_str("Given: Alpha is in 2001.\nGiven: Beta is in 2002.\n");
    doc.push_str("Given: Gamma is in 2003.\nGiven: Delta is in 2004.\n");
    // The six real clues, verbatim.
    doc.push_str("Given: Of the hunting trip and the 2004 trip, one was with Neal and the other was in Connecticut.\n");
    doc.push_str("Given: The Florida trip was the hunting trip.\n");
    doc.push_str("Given: Neither the trip with Bill nor the Florida trip is the 2001 trip.\n");
    doc.push_str("Given: The trip with Yvonne is not in Kentucky.\n");
    doc.push_str("Given: Of the skydiving trip and the Maine trip, one was in 2003 and the other was with Bill.\n");
    doc.push_str("Given: The 2003 trip is not the cycling trip.\n");
    // Solve the WHOLE grid by question and check it against the puzzle's UNIQUE
    // solution (2001/Lillie/cycling/Kentucky, 2002/Neal/hunting/Florida,
    // 2003/Yvonne/kayaking/Maine, 2004/Bill/skydiving/Connecticut). With the year
    // anchor that is Alpha/…/Kentucky, Beta/…/Florida, Gamma/…/Maine,
    // Delta/…/Connecticut — not one lucky cell, the full grid correctly derived.
    let ask = |q: &str| -> Vec<String> {
        answer_question(&format!("{doc}Prove: {q}\nProof: Auto.\n"))
            .expect("the full Simon puzzle should be answerable")
    };
    assert_eq!(ask("Who is in Florida?"), vec!["Beta".to_string()], "2002 ↔ Florida");
    assert_eq!(ask("Who is in Kentucky?"), vec!["Alpha".to_string()], "2001 ↔ Kentucky");
    assert_eq!(ask("Who is in Maine?"), vec!["Gamma".to_string()], "2003 ↔ Maine");
    assert_eq!(ask("Who is in Connecticut?"), vec!["Delta".to_string()], "2004 ↔ Connecticut");
    // Cross-category: the 2002 trip (Beta) is the one with Neal and hunting.
    assert!(ask("Who is with Neal?").contains(&"Beta".to_string()), "2002 ↔ Neal");
    assert!(ask("Who is hunting?").contains(&"Beta".to_string()), "2002 ↔ hunting");
}

/// The 2-category grid that the kernel prover could NOT do (see
/// phase_simon_solve_e2e::solve_two_category_grid_by_question, #[ignore]) now
/// solves through the oracle: "Who is in Maine?" → Beta.
#[test]
fn two_category_grid_solves_via_oracle() {
    let ans = answer_question(
        "## Theorem: Trips\n\
         Given: Alpha is a trip.\n\
         Given: Beta is a trip.\n\
         Given: Alpha is not Beta.\n\
         Given: Every trip is in 2003 or in 2004.\n\
         Given: Every trip is in Florida or in Maine.\n\
         Given: Exactly one trip is in 2003.\n\
         Given: Exactly one trip is in Florida.\n\
         Given: Alpha is in 2003.\n\
         Given: Alpha is in Florida.\n\
         Prove: Who is in Maine?\n\
         Proof: Auto.\n",
    )
    .expect("the two-category grid should be answerable");
    assert!(ans.contains(&"Beta".to_string()), "Beta is in Maine; got: {ans:?}");
}

/// FULL-SIZE category: 4 trips, 4 states (the real Simon column width). Three
/// states are pinned; the fourth trip must be in Maine by elimination over the
/// 4-way closure + four exactly-ones. Solved by a question.
#[test]
fn four_value_state_grid_solves() {
    let ans = answer_question(
        "## Theorem: Trips\n\
         Given: Alpha, Beta, Gamma, and Delta are four different trips.\n\
         Given: Every trip is in Connecticut or in Florida or in Kentucky or in Maine.\n\
         Given: Exactly one trip is in Connecticut.\n\
         Given: Exactly one trip is in Florida.\n\
         Given: Exactly one trip is in Kentucky.\n\
         Given: Exactly one trip is in Maine.\n\
         Given: Alpha is in Florida.\n\
         Given: Beta is in Connecticut.\n\
         Given: Gamma is in Kentucky.\n\
         Prove: Who is in Maine?\n\
         Proof: Auto.\n",
    )
    .expect("the four-value grid should be answerable");
    assert!(ans.contains(&"Delta".to_string()), "Delta is in Maine; got: {ans:?}");
}

/// FULL SIMON SIZE: 4 trips × 3 categories (year, state, friend), each a 4-value
/// bijection — the 39-minute-hang regime, now grounded. Three states pinned; the
/// fourth trip is in Maine by elimination, even with the year and friend
/// categories' constraints present. Solved by a question.
#[test]
fn three_category_four_value_grid_solves() {
    let mut doc =
        String::from("## Theorem: Trips\nGiven: Alpha, Beta, Gamma, and Delta are four different trips.\n");
    doc.push_str(&category("in", &["2001", "2002", "2003", "2004"]));
    doc.push_str(&category("in", &["Connecticut", "Florida", "Kentucky", "Maine"]));
    doc.push_str(&category("with", &["Bill", "Lillie", "Neal", "Yvonne"]));
    doc.push_str("Given: Alpha is in Connecticut.\n");
    doc.push_str("Given: Beta is in Florida.\n");
    doc.push_str("Given: Gamma is in Kentucky.\n");
    doc.push_str("Prove: Who is in Maine?\n");
    doc.push_str("Proof: Auto.\n");
    let ans = answer_question(&doc).expect("the multi-category grid should be answerable");
    assert!(ans.contains(&"Delta".to_string()), "Delta is in Maine; got: {ans:?}");
}
