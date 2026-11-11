use super::*;
use crate::cdcl::{SolveResult, Solver};
use crate::families::{php, random_3sat, random_ksat};

fn false_count(clauses: &[Vec<Lit>], phases: &[bool]) -> usize {
    clauses.iter().filter(|c| !c.iter().any(|l| phases[l.var() as usize] == l.is_positive())).count()
}

fn options(seed: u64, step_limit: u64) -> WalkOptions {
    WalkOptions { cb: fitted_cb(3.0), seed, step_limit }
}

fn free(_: Var) -> Option<bool> {
    None
}

fn is_sat(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    let mut s = Solver::new(num_vars);
    for c in clauses {
        s.add_clause(c.clone());
    }
    matches!(s.solve(), SolveResult::Sat(_))
}

fn rng(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn random_formula(next: &mut impl FnMut() -> u64, num_vars: usize, num_clauses: usize, max_width: usize) -> Vec<Vec<Lit>> {
    (0..num_clauses)
        .map(|_| {
            let width = 1 + (next() % max_width as u64) as usize;
            (0..width).map(|_| Lit::new((next() % num_vars as u64) as Var, next() & 1 == 0)).collect()
        })
        .collect()
}

#[test]
fn fitted_cb_meets_every_table_point_and_interpolates_between_them() {
    for (size, cb) in [(0.0, 2.00), (3.0, 2.50), (4.0, 2.85), (5.0, 3.70), (6.0, 5.10), (7.0, 7.40)] {
        assert!((fitted_cb(size) - cb).abs() < 1e-12, "cb at width {size}");
    }
    assert!((fitted_cb(1.5) - 2.25).abs() < 1e-12);
    assert!((fitted_cb(3.5) - 2.675).abs() < 1e-12);
    assert!((fitted_cb(6.5) - 6.25).abs() < 1e-12);
    assert!((fitted_cb(8.0) - 9.70).abs() < 1e-12, "past the table the last segment extends");
    let mut previous = 0.0;
    for tenth in 0..=100 {
        let cb = fitted_cb(tenth as f64 / 10.0);
        assert!(cb >= previous, "the fit is monotone in the width");
        previous = cb;
    }
}

#[test]
fn the_reported_unsatisfied_count_is_the_false_clauses_of_the_returned_phases_on_every_path() {
    // Small step limits stop walks mid-trail, before and after the trail folds into its snapshot and after it is
    // invalidated; large ones run to a model or to the limit. Every outcome must recount exactly.
    let mut next = rng(0xA11CE);
    for round in 0..600u64 {
        let num_vars = 1 + (next() % 24) as usize;
        let num_clauses = (next() % 120) as usize;
        let clauses = random_formula(&mut next, num_vars, num_clauses, 4);
        let start: Vec<bool> = (0..num_vars).map(|_| next() & 1 == 0).collect();
        let step_limit = [0, 1, 7, 40, 300, 5_000, 200_000][(round % 7) as usize];
        let w = walk(num_vars, clauses.iter().map(|c| c.as_slice()), free, &start, options(round, step_limit));
        assert_eq!(w.phases.len(), num_vars);
        assert_eq!(w.unsatisfied, false_count(&clauses, &w.phases), "round {round}: best tracking lost the best");
        assert_eq!(w.started_unsatisfied, false_count(&clauses, &start), "round {round}: the start is counted as given");
        assert!(w.unsatisfied <= w.started_unsatisfied, "round {round}: the best is never worse than the start");
        if w.unsatisfied == w.started_unsatisfied {
            assert_eq!(w.phases, start, "round {round}: with no improvement the best is the start itself");
        }
    }
}

#[test]
fn a_walk_never_reports_a_model_of_an_unsatisfiable_formula() {
    let (n, clauses) = {
        let (cnf, _) = php(5);
        (cnf.num_vars, cnf.clauses)
    };
    for seed in 0..20u64 {
        let w = walk(n, clauses.iter().map(|c| c.as_slice()), free, &vec![false; n], options(seed, 100_000));
        assert!(w.unsatisfied >= 1);
        assert_eq!(w.unsatisfied, false_count(&clauses, &w.phases));
    }
    let mut next = rng(7);
    let mut unsat_seen = 0;
    for seed in 0..400u64 {
        let n = 1 + (next() % 8) as usize;
        let m = 4 + (next() % 40) as usize;
        let clauses = random_formula(&mut next, n, m, 3);
        let w = walk(n, clauses.iter().map(|c| c.as_slice()), free, &vec![true; n], options(seed, 50_000));
        if !is_sat(n, &clauses) {
            unsat_seen += 1;
            assert!(w.unsatisfied >= 1, "seed {seed}: a model claimed for an unsatisfiable formula");
        }
    }
    assert!(unsat_seen > 40, "the sample holds unsatisfiable formulas ({unsat_seen})");
}

#[test]
fn on_every_tiny_satisfiable_formula_a_long_walk_finds_a_model() {
    let mut next = rng(0x5EED);
    let mut sat_seen = 0;
    for seed in 0..1500u64 {
        let n = 1 + (next() % 8) as usize;
        let m = (next() % 30) as usize;
        let clauses = random_formula(&mut next, n, m, 3);
        if !is_sat(n, &clauses) {
            continue;
        }
        sat_seen += 1;
        let w = walk(n, clauses.iter().map(|c| c.as_slice()), free, &vec![false; n], options(seed, 2_000_000));
        assert_eq!(w.unsatisfied, 0, "seed {seed}: ProbSAT is probabilistically complete on 8 variables");
        assert_eq!(false_count(&clauses, &w.phases), 0);
    }
    assert!(sat_seen > 200, "the sample holds satisfiable formulas ({sat_seen})");
}

#[test]
fn a_walk_finds_a_model_of_satisfiable_random_3sat_below_the_threshold() {
    // Ratio 4.0 sits below the 4.27 threshold, but at n=150 a seed can still draw an unsatisfiable formula; the
    // walk is held to a model on every seed CDCL shows satisfiable, and the sample must hold enough of them.
    let mut satisfiable = 0;
    for seed in 0..10u64 {
        let cnf = random_3sat(150, 600, 0xC0FFEE ^ seed);
        if !is_sat(cnf.num_vars, &cnf.clauses) {
            continue;
        }
        satisfiable += 1;
        let w = walk(
            cnf.num_vars,
            cnf.clauses.iter().map(|c| c.as_slice()),
            free,
            &vec![false; cnf.num_vars],
            options(seed, 50_000_000),
        );
        assert_eq!(w.unsatisfied, 0, "seed {seed}");
        assert_eq!(false_count(&cnf.clauses, &w.phases), 0);
    }
    assert!(satisfiable >= 6, "the sample holds satisfiable formulas ({satisfiable} of 10)");
}

#[test]
fn wider_clauses_walk_with_their_fitted_base() {
    let cnf = random_ksat(5, 120, 1800, 0xFACE);
    assert!(is_sat(cnf.num_vars, &cnf.clauses));
    let w = walk(
        cnf.num_vars,
        cnf.clauses.iter().map(|c| c.as_slice()),
        free,
        &vec![true; cnf.num_vars],
        WalkOptions { cb: fitted_cb(5.0), seed: 3, step_limit: 100_000_000 },
    );
    assert_eq!(w.unsatisfied, 0);
    assert_eq!(false_count(&cnf.clauses, &w.phases), 0);
}

#[test]
fn fixed_variables_hold_their_value_and_shape_the_walked_formula() {
    // x0 fixed true satisfies (x0 ∨ x1) and drops it; x2 fixed false leaves (x2 ∨ x3) as the unit (x3); x4 fixed
    // false makes (x4) a clause no flip can satisfy.
    let l = |v: Var, p: bool| Lit::new(v, p);
    let clauses = vec![vec![l(0, true), l(1, true)], vec![l(2, true), l(3, true)], vec![l(4, true)], vec![l(1, false), l(3, false)]];
    let fixed = |v: Var| match v {
        0 => Some(true),
        2 | 4 => Some(false),
        _ => None,
    };
    for seed in 0..50u64 {
        let w = walk(5, clauses.iter().map(|c| c.as_slice()), fixed, &[false, true, true, false, true], options(seed, 10_000));
        assert!(w.phases[0] && !w.phases[2] && !w.phases[4], "seed {seed}: fixed variables never flip");
        assert!(w.phases[3] && !w.phases[1], "seed {seed}: (x3) forced, then ¬x1 for (¬x1 ∨ ¬x3)");
        assert_eq!(w.unsatisfied, 1, "seed {seed}: only the dead clause (x4) stays false");
        assert_eq!(w.unsatisfied, false_count(&clauses, &w.phases));
    }
}

#[test]
fn an_empty_clause_is_false_under_every_assignment() {
    let clauses = vec![vec![], vec![Lit::pos(0)]];
    let w = walk(1, clauses.iter().map(|c| c.as_slice()), free, &[false], options(1, 1_000));
    assert_eq!(w.unsatisfied, 1);
    assert!(w.phases[0], "the satisfiable remainder is still satisfied");
}

#[test]
fn equal_seeds_walk_equal_paths_and_different_seeds_need_not() {
    let cnf = random_3sat(150, 639, 99);
    let run = |seed: u64| {
        walk(cnf.num_vars, cnf.clauses.iter().map(|c| c.as_slice()), free, &vec![false; cnf.num_vars], options(seed, 300_000))
    };
    assert_eq!(run(11), run(11));
    let distinct = (0..8u64).map(run).map(|w| w.phases).collect::<std::collections::HashSet<_>>().len();
    assert!(distinct > 1, "the seed steers the walk");
}

#[test]
fn the_step_limit_bounds_the_effort_to_one_flip_past_it() {
    let cnf = random_3sat(300, 1290, 5);
    let max_occ = {
        let mut count = vec![0u64; cnf.num_vars * 2];
        for c in &cnf.clauses {
            for l in c {
                count[l.index()] += 1;
            }
        }
        count.into_iter().max().unwrap_or(0)
    };
    for limit in [0u64, 1, 10, 1_000, 50_000] {
        let w = walk(cnf.num_vars, cnf.clauses.iter().map(|c| c.as_slice()), free, &vec![true; cnf.num_vars], options(limit, limit));
        let one_flip = 3 * (max_occ + 1) + 2 * max_occ;
        assert!(w.steps <= limit + one_flip, "limit {limit}: {} steps", w.steps);
        if limit == 0 {
            assert_eq!(w.flips, 0);
            assert_eq!(w.phases, vec![true; cnf.num_vars]);
        }
    }
}

#[test]
fn a_formula_with_no_clauses_is_satisfied_by_the_start() {
    let start = vec![true, false, true];
    let w = walk(3, std::iter::empty(), free, &start, options(0, 1_000));
    assert_eq!(w, Walk { phases: start, unsatisfied: 0, started_unsatisfied: 0, flips: 0, steps: 0 });
}
