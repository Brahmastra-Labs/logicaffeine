use super::*;
use crate::sat::{find_model, ModelOutcome};

fn vars(n: usize) -> Vec<ProofExpr> {
    (0..n).map(|i| atom(format!("x{i}"))).collect()
}

/// Is `formula` satisfiable once the `vars` are pinned to `assignment`? (Our solver decides;
/// the auxiliary atoms are free.)
fn sat_under(formula: &ProofExpr, vars: &[ProofExpr], assignment: &[bool]) -> bool {
    let mut f = formula.clone();
    for (v, &b) in vars.iter().zip(assignment) {
        let unit = if b { v.clone() } else { not(v.clone()) };
        f = and(f, unit);
    }
    matches!(find_model(&f), ModelOutcome::Sat(_))
}

fn for_each_assignment(n: usize, mut f: impl FnMut(&[bool], usize)) {
    for mask in 0..(1u32 << n) {
        let asg: Vec<bool> = (0..n).map(|i| (mask >> i) & 1 == 1).collect();
        let ones = asg.iter().filter(|b| **b).count();
        f(&asg, ones);
    }
}

#[test]
fn at_most_matches_brute_force() {
    for n in 1..=6 {
        let xs = vars(n);
        for k in 0..=n {
            let formula = at_most(&xs, k, "c");
            for_each_assignment(n, |asg, ones| {
                assert_eq!(
                    sat_under(&formula, &xs, asg),
                    ones <= k,
                    "at_most n={n} k={k} ones={ones} asg={asg:?}"
                );
            });
        }
    }
}

#[test]
fn at_least_matches_brute_force() {
    for n in 1..=6 {
        let xs = vars(n);
        for k in 0..=(n + 1) {
            let formula = at_least(&xs, k, "c");
            for_each_assignment(n, |asg, ones| {
                assert_eq!(
                    sat_under(&formula, &xs, asg),
                    ones >= k,
                    "at_least n={n} k={k} ones={ones} asg={asg:?}"
                );
            });
        }
    }
}

#[test]
fn exactly_matches_brute_force() {
    for n in 1..=5 {
        let xs = vars(n);
        for k in 0..=n {
            let formula = exactly(&xs, k, "c");
            for_each_assignment(n, |asg, ones| {
                assert_eq!(
                    sat_under(&formula, &xs, asg),
                    ones == k,
                    "exactly n={n} k={k} ones={ones} asg={asg:?}"
                );
            });
        }
    }
}

#[test]
fn empty_vars_are_handled() {
    let none: Vec<ProofExpr> = vec![];
    for k in 0..=3 {
        assert!(
            matches!(find_model(&at_most(&none, k, "e")), ModelOutcome::Sat(_)),
            "at_most over [] with k={k} is trivially satisfiable"
        );
    }
    assert!(matches!(
        find_model(&at_least(&none, 0, "e")),
        ModelOutcome::Sat(_)
    ));
    assert!(
        matches!(find_model(&at_least(&none, 1, "e")), ModelOutcome::Unsat),
        "at_least 1 of nothing is impossible"
    );
}

#[test]
fn larger_n_spot_check_beyond_exhaustive() {
    // n=8 is past the exhaustive sweep; check a couple of representative points.
    let xs = vars(8);
    let f = at_most(&xs, 3, "c");
    let mut three = vec![false; 8];
    three[0] = true;
    three[2] = true;
    three[5] = true;
    assert!(sat_under(&f, &xs, &three), "exactly 3 ≤ 3 must be SAT");
    let mut four = vec![false; 8];
    for b in four.iter_mut().take(4) {
        *b = true;
    }
    assert!(!sat_under(&f, &xs, &four), "4 > 3 must be UNSAT");
}
