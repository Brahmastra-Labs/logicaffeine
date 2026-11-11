use super::*;

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn random_3sat(rng: &mut u64, num_vars: usize, ratio: f64) -> Vec<Vec<Lit>> {
    (0..(num_vars as f64 * ratio) as usize)
        .map(|_| (0..3).map(|_| Lit::new((xorshift(rng) % num_vars as u64) as u32, xorshift(rng) % 2 == 0)).collect())
        .collect()
}

fn models(clauses: &[Vec<Lit>], m: &[bool]) -> bool {
    clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
}

fn plain(num_vars: usize, clauses: &[Vec<Lit>]) -> SolveResult {
    let mut s = Solver::new(num_vars);
    for c in clauses {
        s.add_clause(c.clone());
    }
    s.solve()
}

fn refutes(num_vars: usize, clauses: &[Vec<Lit>], proof: &[Vec<Lit>]) -> bool {
    let steps: Vec<crate::proof::ProofStep> = proof.iter().map(|c| crate::proof::ProofStep::Rup(c.clone())).collect();
    proof.last().is_some_and(Vec::is_empty) && crate::pr::check_pr_refutation_fast(num_vars, clauses, &steps)
}

#[test]
fn cube_and_conquer_decides_as_plain_cdcl_with_checked_models_and_refutations() {
    // Random 3-SAT at the threshold, split shallow and deep, conquered by one solver and by several: each verdict the
    // plain solver's, each model a model, each refutation replaying step by step.
    let mut rng: u64 = 0xA076_1D64_78BD_642F;
    let (mut sat, mut unsat, mut refuted, mut split) = (0, 0, 0, 0);
    for trial in 0..48 {
        let num_vars = 30 + (xorshift(&mut rng) % 70) as usize;
        let clauses = random_3sat(&mut rng, num_vars, 4.26);
        let opts = Options { depth: [0, 2, 5, 9][trial % 4], preselect: [5, 40][trial % 2], threads: 1 + trial % 3 };
        let raced = solve(num_vars, &clauses, &opts);
        assert!(raced.cubes <= 1 << opts.depth, "trial {trial}: {} cubes at depth {}", raced.cubes, opts.depth);
        refuted += raced.refuted;
        split += usize::from(raced.cubes > 1);
        match (plain(num_vars, &clauses), &raced.result) {
            (SolveResult::Sat(_), SolveResult::Sat(m)) => {
                assert!(models(&clauses, m), "trial {trial}: the cube's model satisfies the formula");
                assert!(raced.cube.is_some_and(|c| c < raced.cubes), "trial {trial}: {raced:?}");
                sat += 1;
            }
            (SolveResult::Unsat, SolveResult::Unsat) => {
                assert!(refutes(num_vars, &clauses, &raced.proof), "trial {trial}: the refutation replays");
                unsat += 1;
            }
            (p, r) => panic!("trial {trial}: plain {p:?}, cube-and-conquer {r:?}"),
        }
    }
    assert!(sat > 10 && unsat > 10 && refuted > 20 && split > 15, "{sat} SAT, {unsat} UNSAT, {refuted} nodes refuted, {split} split");
}

#[test]
fn the_split_and_its_conquest_are_the_same_every_time() {
    let mut rng: u64 = 0x5851_F42D_4C95_7F2D;
    for _ in 0..12 {
        let num_vars = 60 + (xorshift(&mut rng) % 40) as usize;
        let clauses = random_3sat(&mut rng, num_vars, 4.26);
        let opts = Options { depth: 6, preselect: 20, threads: 4 };
        let first = solve(num_vars, &clauses, &opts);
        for _ in 0..3 {
            assert_eq!(solve(num_vars, &clauses, &opts), first, "threads' timing never changes the outcome");
        }
    }
}

/// The pigeonhole formula: `n + 1` pigeons in `n` holes, variable `p·n + h` pigeon `p` in hole `h`.
fn php(n: usize) -> (usize, Vec<Vec<Lit>>) {
    let var = |p: usize, h: usize| (p * n + h) as u32;
    let mut clauses: Vec<Vec<Lit>> = (0..=n).map(|p| (0..n).map(|h| Lit::pos(var(p, h))).collect()).collect();
    for h in 0..n {
        for p in 0..=n {
            for q in p + 1..=n {
                clauses.push(vec![Lit::neg(var(p, h)), Lit::neg(var(q, h))]);
            }
        }
    }
    ((n + 1) * n, clauses)
}

#[test]
fn a_formula_the_lookahead_refutes_alone_leaves_no_cube_and_its_refutation_replays() {
    // A chain of implications forcing x0 both ways: x0 fails at once and its negation fails too.
    let (x, y, z) = (Lit::pos(0), Lit::pos(1), Lit::pos(2));
    let clauses = vec![vec![x.negated(), y], vec![x.negated(), y.negated()], vec![x, z], vec![x, z.negated()]];
    let outcome = solve(3, &clauses, &Options::default());
    assert_eq!(outcome.result, SolveResult::Unsat);
    assert_eq!(outcome.cubes, 0, "{outcome:?}");
    assert!(outcome.refuted >= 1, "{outcome:?}");
    assert!(refutes(3, &clauses, &outcome.proof), "{:?}", outcome.proof);
    // Contradictory units refute the formula before any lookahead.
    let units = vec![vec![x], vec![x.negated()]];
    let outcome = solve(1, &units, &Options::default());
    assert_eq!(outcome.result, SolveResult::Unsat);
    assert!(refutes(1, &units, &outcome.proof), "{:?}", outcome.proof);
}

#[test]
fn pigeonhole_is_split_into_cubes_each_refuted_and_the_whole_refutation_replays() {
    for (n, depth) in [(5, 4), (6, 7)] {
        let (num_vars, clauses) = php(n);
        let outcome = solve(num_vars, &clauses, &Options { depth, preselect: 40, threads: 3 });
        assert_eq!(outcome.result, SolveResult::Unsat);
        assert!(outcome.cubes > 1, "php({n}): {outcome:?}");
        assert!(refutes(num_vars, &clauses, &outcome.proof), "php({n})");
    }
}

#[test]
fn a_satisfiable_formula_is_answered_by_its_lowest_satisfiable_cube_whatever_the_threads() {
    // Satisfiable formulas at the threshold, split deep, so the first cubes are often refuted: the cube answering is
    // the same however many solvers conquer, the lowest-numbered satisfiable one.
    let mut rng: u64 = 0x2545_F491_4F6C_DD1D;
    let (mut formulas, mut beyond_first) = (0, 0);
    while formulas < 20 {
        let num_vars = 60 + (xorshift(&mut rng) % 30) as usize;
        let clauses = random_3sat(&mut rng, num_vars, 4.2);
        if plain(num_vars, &clauses) == SolveResult::Unsat {
            continue;
        }
        formulas += 1;
        let opts = |threads| Options { depth: 7, preselect: 20, threads };
        let one = solve(num_vars, &clauses, &opts(1));
        for threads in [2, 3, 5] {
            let many = solve(num_vars, &clauses, &opts(threads));
            assert_eq!(many.cube, one.cube, "{threads} threads");
            let SolveResult::Sat(m) = &many.result else { panic!("satisfiable") };
            assert!(models(&clauses, m));
        }
        beyond_first += usize::from(one.cube.is_some_and(|c| c > 0));
    }
    assert!(beyond_first >= 2, "the answering cube is past the first on {beyond_first} of 20");
}
