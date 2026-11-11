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

#[test]
fn the_race_decides_as_plain_cdcl_with_checked_models_and_refutations() {
    let mut rng: u64 = 0xD1B5_4A32_D192_ED03;
    let (mut sat, mut unsat, mut not_first) = (0, 0, 0);
    for _ in 0..60 {
        let num_vars = 40 + (xorshift(&mut rng) % 80) as usize;
        let clauses = random_3sat(&mut rng, num_vars, 4.26);
        let mut plain = Solver::new(num_vars);
        for c in &clauses {
            plain.add_clause(c.clone());
        }
        let truth = plain.solve();
        let raced = solve(num_vars, &clauses, &DEFAULT, 50);
        not_first += usize::from(raced.winner != 0);
        match (&truth, &raced.result) {
            (SolveResult::Sat(_), SolveResult::Sat(m)) => {
                assert!(models(&clauses, m), "the winner's model satisfies the formula");
                sat += 1;
            }
            (SolveResult::Unsat, SolveResult::Unsat) => {
                let steps: Vec<crate::proof::ProofStep> = raced.proof.iter().map(|c| crate::proof::ProofStep::Rup(c.clone())).collect();
                assert!(crate::pr::check_pr_refutation_fast(num_vars, &clauses, &steps), "the winner's learned clauses refute the formula");
                unsat += 1;
            }
            (t, r) => panic!("plain {t:?}, race {r:?}"),
        }
    }
    assert!(sat > 10 && unsat > 10 && not_first > 5, "{sat} SAT, {unsat} UNSAT, {not_first} won past the core");
}

#[test]
fn the_race_is_the_same_race_every_time() {
    let mut rng: u64 = 0x94D0_49BB_1331_11EB;
    for _ in 0..20 {
        let num_vars = 80 + (xorshift(&mut rng) % 60) as usize;
        let clauses = random_3sat(&mut rng, num_vars, 4.26);
        let first = solve(num_vars, &clauses, &DEFAULT, 40);
        for _ in 0..3 {
            assert_eq!(solve(num_vars, &clauses, &DEFAULT, 40), first, "threads' timing never changes the outcome");
        }
    }
}

#[test]
fn a_race_of_one_is_that_search_alone() {
    let mut rng: u64 = 0x2545_F491_4F6C_DD1D;
    for (i, config) in DEFAULT.iter().enumerate() {
        for _ in 0..10 {
            let num_vars = 50 + (xorshift(&mut rng) % 50) as usize;
            let clauses = random_3sat(&mut rng, num_vars, 4.26);
            let mut alone = configured(num_vars, &clauses, config);
            let expected = alone.solve();
            let raced = solve(num_vars, &clauses, std::slice::from_ref(config), 7);
            assert_eq!(raced.result, expected, "configuration {i}: the race of one reaches its search's verdict");
            assert_eq!(raced.conflicts, vec![alone.conflicts()], "configuration {i}: by the same conflicts");
        }
    }
}

#[test]
fn among_searches_deciding_in_one_round_the_lowest_numbered_wins() {
    // Three copies of one configuration decide in the same round; the first of them wins, and the others are reported
    // as the round before left them — the budgets of the rounds before the race's, summed.
    let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
    let clauses = random_3sat(&mut rng, 90, 4.26);
    let copies = [DEFAULT[1], DEFAULT[1], DEFAULT[1]];
    let raced = solve(90, &clauses, &copies, 10);
    assert_eq!(raced.winner, 0);
    assert!(raced.rounds > 2, "the race runs past its first rounds ({} rounds)", raced.rounds);
    let before: u64 = (0..raced.rounds as u32 - 1).map(|k| 10u64 << k).sum();
    assert_eq!(raced.conflicts[1..], [before, before], "the copies stand where the round before the race left them");
    assert!(raced.conflicts[0] > before, "the winner went on to decide in the race's round");
}

#[test]
fn a_sharing_race_decides_as_plain_cdcl_with_checked_models_and_merged_refutations() {
    let mut rng: u64 = 0x6C62_272E_07BB_0142;
    let (mut sat, mut unsat) = (0, 0);
    for _ in 0..40 {
        let num_vars = 60 + (xorshift(&mut rng) % 80) as usize;
        let clauses = random_3sat(&mut rng, num_vars, 4.26);
        let mut plain = Solver::new(num_vars);
        for c in &clauses {
            plain.add_clause(c.clone());
        }
        match (plain.solve(), race(num_vars, &clauses, &DEFAULT, 30, true).result.clone()) {
            (SolveResult::Sat(_), SolveResult::Sat(m)) => {
                assert!(models(&clauses, &m));
                sat += 1;
            }
            (SolveResult::Unsat, SolveResult::Unsat) => {
                let raced = race(num_vars, &clauses, &DEFAULT, 30, true);
                let steps: Vec<crate::proof::ProofStep> = raced.proof.iter().map(|c| crate::proof::ProofStep::Rup(c.clone())).collect();
                assert!(crate::pr::check_pr_refutation_fast(num_vars, &clauses, &steps), "the merged refutation checks");
                unsat += 1;
            }
            (p, r) => panic!("plain {p:?}, sharing race {r:?}"),
        }
    }
    assert!(sat > 5 && unsat > 5, "{sat} SAT, {unsat} UNSAT");
}

#[test]
fn a_sharing_race_is_the_same_race_every_time() {
    let mut rng: u64 = 0x5851_F42D_4C95_7F2D;
    let mut shared = 0;
    for _ in 0..12 {
        let num_vars = 90 + (xorshift(&mut rng) % 50) as usize;
        let clauses = random_3sat(&mut rng, num_vars, 4.26);
        let first = race(num_vars, &clauses, &DEFAULT, 10, true);
        shared += usize::from(first.rounds > 1);
        for _ in 0..3 {
            assert_eq!(race(num_vars, &clauses, &DEFAULT, 10, true), first);
        }
    }
    assert!(shared >= 6, "the races share between rounds ({shared} of 12 ran past one)");
}

#[test]
fn a_configuration_backtracking_chronologically_does_and_the_default_race_holds_a_focused_search() {
    let mut rng: u64 = 0xBF58_476D_1CE4_E5B9;
    let clauses = random_3sat(&mut rng, 160, 4.26);
    let mut chrono = configured(160, &clauses, &Config { chrono: Some(1), ..Config::CORE });
    let mut core = configured(160, &clauses, &Config::CORE);
    match (chrono.solve(), core.solve()) {
        (SolveResult::Sat(a), SolveResult::Sat(b)) => assert!(models(&clauses, &a) && models(&clauses, &b)),
        (SolveResult::Unsat, SolveResult::Unsat) => {}
        (a, b) => panic!("chronological {a:?}, core {b:?}"),
    }
    assert!(chrono.chrono_backtracks() > 0, "a backjump past one level backtracks chronologically");
    assert_eq!(core.chrono_backtracks(), 0, "the core backjumps");
    let focused = |c: &Config| c.restart == RestartMode::Glucose && c.focused_queue && c.chrono.is_some();
    assert!(DEFAULT.iter().any(focused), "circuits only a focused search decides: Glucose restarts, the queue, chronological backtracking");
}

#[test]
fn solve_is_the_race_that_shares_between_rounds() {
    let mut rng: u64 = 0xC2B2_AE3D_27D4_EB4F;
    let mut changed = 0;
    for _ in 0..12 {
        let num_vars = 90 + (xorshift(&mut rng) % 50) as usize;
        let clauses = random_3sat(&mut rng, num_vars, 4.26);
        let solved = solve(num_vars, &clauses, &DEFAULT, 10);
        assert_eq!(solved, race(num_vars, &clauses, &DEFAULT, 10, true));
        changed += usize::from(solved != race(num_vars, &clauses, &DEFAULT, 10, false));
    }
    assert!(changed >= 6, "sharing changes the race on {changed} of 12");
}
