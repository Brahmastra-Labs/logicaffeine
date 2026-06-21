//! The three trust tiers, end to end on the FULL 4×4 Simon, over the SAME grounded grid
//! problem `compile_theorem_for_ui` solves:
//!   - **Untrusted** — `cnf::cdcl_entails` (CDCL solve only, no proof).
//!   - **Rup**       — `rup::entails_certified` (CDCL + independent linear RUP check).
//!   - **Kernel**    — `compile_theorem_for_ui` (DerivationTree + dependent-type kernel).
//! Every tier must AGREE on every cell (a certifying engine is only as good as its
//! cross-checks), and the trace prints each tier's wall-clock so the speedup is visible.

use logicaffeine_compile::{compile_theorem_for_ui, grounded_grid_problem};
use logicaffeine_proof::cnf::Cnf;
use logicaffeine_proof::rup::Verdict;

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

/// For each cell, all three tiers agree, and the entailed ones are RUP-certified. We run a
/// spread of cells — anchored, deep-deduction, true AND false — so the cross-check is real.
#[test]
fn trust_tiers_agree_on_full_simon() {
    // (goal, is-it-true-in-the-unique-solution)
    let cases = [
        ("Beta is in Florida.", true),
        ("Gamma is in Maine.", true),
        ("Delta is in Connecticut.", true),
        ("Alpha is in Kentucky.", true),
        ("Beta is with Neal.", true),
        ("Gamma is kayaking.", true),
        ("Beta is in Maine.", false),   // Beta is in Florida
        ("Alpha is with Yvonne.", false), // Alpha is with Lillie
    ];

    let trace = std::env::var("LOGOS_TRACE").is_ok();
    for (goal, truth) in cases {
        let doc = full_simon_doc(goal);
        let (premises, g) = grounded_grid_problem(&doc).expect("Simon is an Auto grid");

        // Tier 0 — Untrusted CDCL.
        let t0 = std::time::Instant::now();
        let untrusted = logicaffeine_proof::cnf::cdcl_entails(&premises, &g);
        let d_untrusted = t0.elapsed();

        // Tier 1 — RUP-certified.
        let t1 = std::time::Instant::now();
        let rup = logicaffeine_proof::rup::entails_certified(&premises, &g);
        let d_rup = t1.elapsed();

        // Tier 2 — Kernel (the existing dependent-type path).
        let t2 = std::time::Instant::now();
        let kernel = compile_theorem_for_ui(&doc).verified;
        let d_kernel = t2.elapsed();

        // Z3 (uncertified) on the SAME grounded problem — the bar we must beat.
        #[cfg(feature = "verification")]
        let z3 = {
            let t3 = std::time::Instant::now();
            let v = logicaffeine_proof::oracle::oracle_entails(&premises, &g);
            (t3.elapsed(), v)
        };

        if trace {
            #[cfg(feature = "verification")]
            eprintln!(
                "{goal:<28} untrusted {:>8.2?} | rup {:>8.2?} | kernel {:>8.2?} | z3 {:>8.2?} ({:?})",
                d_untrusted, d_rup, d_kernel, z3.0, z3.1
            );
            #[cfg(not(feature = "verification"))]
            eprintln!(
                "{goal:<28} untrusted {:>8.2?} | rup {:>8.2?} | kernel {:>8.2?}",
                d_untrusted, d_rup, d_kernel
            );
        }

        // The Untrusted verdict must match ground truth.
        assert_eq!(untrusted, Some(truth), "Untrusted CDCL wrong on `{goal}`");
        // The RUP verdict must match ground truth AND be certified for entailed cells.
        let rup_truth = if truth { Verdict::Entailed } else { Verdict::NotEntailed };
        assert_eq!(rup, Some(rup_truth), "RUP-certified verdict wrong on `{goal}`");
        // The kernel tier proves the TRUE cells (it does not disprove false ones).
        if truth {
            assert!(kernel, "Kernel tier failed to certify true cell `{goal}`");
        }

        // Independent cross-check against Z3 on the same grounded problem: Z3 must reach
        // the same verdict our certified tiers do (an oracle disagreement would expose a
        // bug in either engine — neither has ever disagreed).
        #[cfg(feature = "verification")]
        {
            use logicaffeine_proof::oracle::SmtVerdict;
            let expected = if truth { SmtVerdict::Entailed } else { SmtVerdict::NotEntailed };
            assert_eq!(z3.1, expected, "Z3 disagrees with our tiers on `{goal}`");
        }
    }
}

/// Incremental solving: clausify the shared premises ONCE, then certify every value of a
/// category against the prepared CNF. Must agree with the fresh-CNF path AND respect the
/// bijection (exactly one state holds for Beta) — and amortizes the of-pair Tseitin cost.
#[test]
fn prepared_cnf_matches_fresh_and_respects_bijection() {
    let states = ["Connecticut", "Florida", "Kentucky", "Maine"];
    let problems: Vec<_> = states
        .iter()
        .map(|s| {
            grounded_grid_problem(&full_simon_doc(&format!("Beta is in {s}.")))
                .expect("Auto grid")
        })
        .collect();
    let premises = &problems[0].0;

    let prepared = Cnf::from_premises(premises).expect("premises clausify");

    let mut entailed = 0;
    for (i, s) in states.iter().enumerate() {
        let goal = &problems[i].1;
        let via_prepared = logicaffeine_proof::rup::entails_certified_prepared(&prepared, goal);
        let via_fresh = logicaffeine_proof::rup::entails_certified(premises, goal);
        assert_eq!(via_prepared, via_fresh, "prepared vs fresh disagree on Beta in {s}");
        if via_prepared == Some(Verdict::Entailed) {
            entailed += 1;
        }
    }
    assert_eq!(entailed, 1, "Beta is in exactly one state (a bijection)");
}
