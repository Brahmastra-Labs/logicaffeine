//! Phase breakdown of the certified CDCL path on the full Simon, so optimization is
//! data-driven rather than guessed. Run with `--nocapture`.

use logicaffeine_compile::grounded_grid_problem;
use logicaffeine_proof::cdcl::Solver;
use logicaffeine_proof::cnf::Cnf;
use logicaffeine_proof::rup::check_refutation;
use std::time::Instant;

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

#[test]
fn profile_certified_path() {
    for goal in ["Beta is in Florida.", "Beta is with Neal.", "Beta is in Maine."] {
        let (premises, g) = grounded_grid_problem(&full_simon_doc(goal)).unwrap();

        // Run a few iterations to smooth out noise (and reflect warm caches).
        let iters = 50;
        let mut d_cnf = std::time::Duration::ZERO;
        let mut d_solve = std::time::Duration::ZERO;
        let mut d_rup = std::time::Duration::ZERO;
        let (mut nv, mut nc, mut nl) = (0usize, 0usize, 0usize);
        for _ in 0..iters {
            let t = Instant::now();
            let mut cnf = Cnf::new();
            for p in &premises {
                cnf.assert(p);
            }
            cnf.assert_neg(&g);
            d_cnf += t.elapsed();
            let num_vars = cnf.num_vars();
            let na = cnf.num_atoms();
            let mut seen = std::collections::HashSet::new();
            let mut dups = 0usize;
            for c in cnf.clauses() {
                let mut k = c.clone();
                k.sort_by_key(|l| format!("{l:?}"));
                if !seen.insert(format!("{k:?}")) {
                    dups += 1;
                }
            }
            let original = cnf.clauses().to_vec();
            nv = num_vars;
            nc = original.len();
            eprintln!("  [diag {goal}] atoms={na} aux={} clauses={} dup_clauses={dups}", num_vars - na, original.len());

            let t = Instant::now();
            let mut s = Solver::new(num_vars);
            for c in &original {
                s.add_clause(c.clone());
            }
            let _ = s.solve();
            d_solve += t.elapsed();
            let learned: Vec<_> = s.learned().iter().map(|c| c.lits.clone()).collect();
            nl = learned.len();

            let t = Instant::now();
            let _ = check_refutation(num_vars, &original, &learned);
            d_rup += t.elapsed();
        }
        eprintln!(
            "{goal:<22} vars={nv:>4} clauses={nc:>4} learned={nl:>4} | cnf {:>8.2?} solve {:>8.2?} rup {:>8.2?}",
            d_cnf / iters,
            d_solve / iters,
            d_rup / iters,
        );
    }
}
