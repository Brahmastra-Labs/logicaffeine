//! The studio `Simon` example, end to end through the real UI compile entry
//! ([`compile_theorem_for_ui`]). Simon is a 2 trip × 2 category logic grid (year
//! {2003, 2004}, state {Florida, Maine}); "Beta is in Maine" is forced by the
//! exactly-one / closure bijection. This pins that the incremental certified grid
//! solver (`logicaffeine_proof::grid_solver`, now wired as the grid path's primary)
//! parses, grounds, solves, and KERNEL-CERTIFIES the cell — no Z3, runnable in WASM.

use logicaffeine_compile::compile_theorem_for_ui;

const SIMON: &str = r#"## Theorem: Simon
Given: Alpha is a trip.
Given: Beta is a trip.
Given: Alpha is not Beta.
Given: Every trip is in 2003 or in 2004.
Given: Every trip is in Florida or in Maine.
Given: Exactly one trip is in 2003.
Given: Exactly one trip is in Florida.
Given: Alpha is in 2003.
Given: Alpha is in Florida.
Prove: Beta is in Maine.
Proof: Auto.
"#;

#[test]
fn studio_simon_solves_and_certifies() {
    let result = compile_theorem_for_ui(SIMON);
    assert!(result.error.is_none(), "parse/compile error: {:?}", result.error);
    assert!(
        result.verified,
        "Simon must be kernel-certified by the grid solver; err: {:?}",
        result.verification_error
    );
    assert!(
        result.derivation.is_some(),
        "a certified derivation tree must be produced for the proof trace"
    );
}

fn full_simon_doc(goal: &str) -> String {
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

/// The FULL 4×4 Simon (all four categories, six clues including both of-pair clues),
/// proved for a cell that genuinely depends on an of-pair, through the no-Z3 studio
/// path — exercised here in the forge-free `logicaffeine-compile` crate for fast
/// feedback (mirrors `phase_studio_simon_full` in the integration crate).
#[test]
fn full_simon_florida_cell_proves_via_studio_path() {
    let r = compile_theorem_for_ui(&full_simon_doc("Beta is in Florida."));
    assert!(r.error.is_none(), "compile error: {:?}", r.error);
    assert!(
        r.verified,
        "the full-Simon Florida cell must prove kernel-certified (no Z3); err: {:?}",
        r.verification_error
    );
    // Regression lock on the proof SIZE (the certification cost is ~linear in it). The
    // hidden-single column closures + min-live closure selection keep the search shallow:
    // the cell proves in a few hundred derivation nodes, not the ~8 000 a row-only,
    // first-closure search produced. A blow-up here means propagation/closure-selection
    // regressed and certification will crawl again.
    fn nodes(t: &logicaffeine_proof::DerivationTree) -> usize {
        1 + t.premises.iter().map(nodes).sum::<usize>()
    }
    let n = r.derivation.as_ref().map(nodes).unwrap_or(usize::MAX);
    assert!(
        n < 2000,
        "full-Simon certified proof must stay small (was {n} nodes; expected a few hundred)"
    );
}

/// `Gamma is in Maine` — Maine is the LAST state of the closure, so the cell is the
/// RIGHTMOST disjunct: proving it positively requires refuting the whole `CT ∨ FL ∨ KY`
/// sub-disjunction to its left, which `prove_neg_by_search` must handle as a disjunction
/// (not a single literal). This cell was unprovable before that fix — a completeness lock.
#[test]
fn full_simon_rightmost_cell_certifies() {
    let r = compile_theorem_for_ui(&full_simon_doc("Gamma is in Maine."));
    assert!(r.error.is_none(), "compile error: {:?}", r.error);
    assert!(
        r.verified,
        "a cell that is its closure's last disjunct must still certify (sub-disjunction \
         refutation); err: {:?}",
        r.verification_error
    );
}

/// A DEEP-deduction cell (a friend assignment needing the whole grid resolved). The
/// minimum-remaining-values split heuristic (`pick_split` takes the fewest-live clause)
/// keeps its case-analysis shallow: this cell was ~44 000 derivation nodes with a
/// first-clause split, ~2 500 with MRV. Lock the proof size so a split-order regression
/// (which would 10× the certification time) is caught.
#[test]
fn full_simon_deep_cell_stays_shallow() {
    fn nodes(t: &logicaffeine_proof::DerivationTree) -> usize {
        1 + t.premises.iter().map(nodes).sum::<usize>()
    }
    let r = compile_theorem_for_ui(&full_simon_doc("Beta is with Neal."));
    assert!(r.verified, "deep friend cell must certify; err: {:?}", r.verification_error);
    let n = r.derivation.as_ref().map(nodes).unwrap_or(usize::MAX);
    assert!(n < 6000, "deep cell must stay shallow under MRV (was {n} nodes; expected ~2 500)");
}
