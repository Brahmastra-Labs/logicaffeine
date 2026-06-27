//! The easter-egg full-grid solve: when the premises describe a declared finite-domain
//! grid (the structural form `looks_like_grid` recognizes), `solve_grid` fills EVERY cell
//! it can prove — each an entailment closed by the no-Z3 certified prover (CDCL+RUP /
//! kernel). The full 4×4 Simon has a unique solution, so all sixteen category cells are
//! determined and must match the published grid.

use logicaffeine_compile::solve_grid;
use std::collections::BTreeSet;

/// The full 4×4 Simon document (mirrors the studio `LOGIC_SIMON` example and the
/// `grid_solver_studio_simon` proof test). The bijection scaffold is synthesized per
/// category; the six clues are verbatim.
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

/// Every (row, value) cell the grid solve determined, lowercased so the assertion is
/// independent of predicate-name casing.
fn assignment(input: &str) -> BTreeSet<(String, String)> {
    let grid = solve_grid(input).expect("the full Simon premises describe a grid");
    assert_eq!(grid.rows.len(), 4, "four trips");
    let mut got = BTreeSet::new();
    for (ri, row) in grid.rows.iter().enumerate() {
        for col in &grid.columns {
            if let Some(v) = &col.cells[ri] {
                got.insert((row.to_lowercase(), v.to_lowercase()));
            }
        }
    }
    got
}

#[test]
fn full_simon_grid_solves_every_cell_no_z3() {
    let got = assignment(&full_simon("Beta is in Florida."));

    let expect: &[(&str, &str)] = &[
        ("alpha", "2001"), ("alpha", "kentucky"), ("alpha", "lillie"), ("alpha", "cycling"),
        ("beta", "2002"), ("beta", "florida"), ("beta", "neal"), ("beta", "hunting"),
        ("gamma", "2003"), ("gamma", "maine"), ("gamma", "yvonne"), ("gamma", "kayaking"),
        ("delta", "2004"), ("delta", "connecticut"), ("delta", "bill"), ("delta", "skydiving"),
    ];
    for (r, v) in expect {
        assert!(
            got.contains(&(r.to_string(), v.to_string())),
            "grid missing certified cell {r} -> {v}; got {got:?}"
        );
    }
    assert_eq!(got.len(), 16, "the unique-solution grid must determine all 16 cells: {got:?}");
}

#[test]
fn solve_grid_is_none_for_a_plain_syllogism() {
    let doc = "## Theorem: S\nGiven: All men are mortal.\nGiven: Socrates is a man.\nProve: Socrates is mortal.\nProof: Auto.\n";
    assert!(solve_grid(doc).is_none(), "a non-grid theorem has no grid to fill");
}
