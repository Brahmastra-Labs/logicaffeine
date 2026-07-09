//! The FULL Simon puzzle — 4 categories, the bijection, and ALL SIX clues including
//! the two of-pair clues — proved for a cell that genuinely depends on an of-pair,
//! through the in-browser, no-Z3 path (`compile_theorem_for_ui` → grounded kernel
//! solve). This is the headline: the kernel-certified prover solves the real grid,
//! of-pair clues and all, in the browser.

use logicaffeine_compile::compile_theorem_for_ui;

fn simon_doc(goal: &str) -> String {
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
fn full_simon_florida_cell_proves_via_studio_path() {
    // 2002 ↔ Florida is the puzzle's solution (Beta = 2002 by the anchor). Reaching
    // it needs clue 2 (Florida = hunting) and clue 1 (the of-pair over the hunting
    // and 2004 trips).
    let r = compile_theorem_for_ui(&simon_doc("Beta is in Florida."));
    assert!(r.error.is_none(), "compile error: {:?}", r.error);
    assert!(
        r.verified,
        "the full-Simon Florida cell must prove kernel-certified (no Z3); err: {:?}",
        r.verification_error
    );
}
