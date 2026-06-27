//! A wh-question goal ("Prove: Who is in Florida?") typed into the studio runs through
//! `compile_theorem_for_ui`. The closed-goal prover cannot answer an open ∃-goal — it
//! grinds a deep, futile search for over a minute and fails. The studio entry must instead
//! RECOGNIZE the wh-form (an `Exists` goal) and answer it by enumeration through the
//! no-Z3 certified path, returning the witness ("Beta") promptly.

use logicaffeine_compile::compile_theorem_for_ui;
use std::time::Instant;

fn full_simon(goal: &str) -> String {
    let mut doc = String::from(
        "## Theorem: Simon\nGiven: Alpha, Beta, Gamma, and Delta are four different trips.\n",
    );
    doc.push_str("Given: 2001, 2002, 2003, and 2004 are four different years.\n");
    doc.push_str("Given: Connecticut, Florida, Kentucky, and Maine are four different states.\n");
    doc.push_str("Given: Bill, Lillie, Neal, and Yvonne are four different friends.\n");
    doc.push_str("Given: Cycling, hunting, kayaking, and skydiving are four different activities.\n");
    for (prep, items) in [
        ("in", ["2001", "2002", "2003", "2004"]),
        ("in", ["Connecticut", "Florida", "Kentucky", "Maine"]),
        ("with", ["Bill", "Lillie", "Neal", "Yvonne"]),
    ] {
        doc.push_str(&format!(
            "Given: Every trip is {} {} or {} {} or {} {} or {} {}.\n",
            prep, items[0], prep, items[1], prep, items[2], prep, items[3]
        ));
        for i in items {
            doc.push_str(&format!("Given: Exactly one trip is {prep} {i}.\n"));
        }
    }
    doc.push_str("Given: Every trip is cycling or hunting or kayaking or skydiving.\n");
    for a in ["cycling", "hunting", "kayaking", "skydiving"] {
        doc.push_str(&format!("Given: Exactly one trip is {a}.\n"));
    }
    doc.push_str("Given: Alpha is in 2001.\nGiven: Beta is in 2002.\n");
    doc.push_str("Given: Gamma is in 2003.\nGiven: Delta is in 2004.\n");
    doc.push_str("Given: Of the hunting trip and the 2004 trip, one was with Neal and the other was in Connecticut.\n");
    doc.push_str("Given: The Florida trip was the hunting trip.\n");
    doc.push_str("Given: Neither the trip with Bill nor the Florida trip is the 2001 trip.\n");
    doc.push_str("Given: The trip with Yvonne is not in Kentucky.\n");
    doc.push_str("Given: Of the skydiving trip and the Maine trip, one was in 2003 and the other was with Bill.\n");
    doc.push_str("Given: The 2003 trip is not the cycling trip.\n");
    doc.push_str(&format!("Prove: {goal}\nProof: Auto.\n"));
    doc
}

#[test]
fn studio_answers_who_is_in_florida_promptly() {
    let t = Instant::now();
    let r = compile_theorem_for_ui(&full_simon("Who is in Florida?"));
    let elapsed = t.elapsed();

    assert!(r.error.is_none(), "wh-question must parse: {:?}", r.error);
    assert_eq!(
        r.answer.as_deref(),
        Some(["Beta".to_string()].as_slice()),
        "the studio must answer 'Who is in Florida?' with the certified witness"
    );
    assert!(r.verified, "an answered wh-question is a certified result");
    // The closed-goal path took 71s and failed; the wh-route must be quick. A generous
    // ceiling that still fails loudly if the slow path ever runs again.
    assert!(
        elapsed.as_secs() < 10,
        "wh-question must not grind the closed-goal search (took {elapsed:?})"
    );
}

#[test]
fn studio_grid_is_attached_for_simon() {
    let r = compile_theorem_for_ui(&full_simon("Beta is in Florida."));
    let grid = r.grid.expect("a recognized grid theorem carries its solved grid");
    assert_eq!(grid.rows.len(), 4);
    let filled: usize = grid.columns.iter().flat_map(|c| &c.cells).filter(|c| c.is_some()).count();
    assert_eq!(filled, 16, "the easter-egg grid fills every determined cell");
}
