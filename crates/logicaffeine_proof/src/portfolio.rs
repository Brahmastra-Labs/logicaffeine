//! **Parallel portfolio search, deterministic.** Configurations of the CDCL core search one formula side by side, a
//! thread each, in rounds: in every round each search continues for a budget of conflicts that doubles from round to
//! round ([`Solver::resume`]), and the first round in which any search decides ends the race, won by the
//! lowest-numbered configuration that decided in it. Between rounds every search takes the units and binaries each
//! other learned in the round ([`Solver::import`]). What a search does in a round follows from the formula, the
//! configurations and the rounds before it, so the winner, its model or its refutation never follow from the threads'
//! timing; a search that decides stops the higher-numbered ones of its round at once, since none of them can win it.
//! A refutation is every search's learned clauses round by round, the searches in order, ending with the winner's last
//! round, a shared clause after the clauses that derived it, each RUP against the formula and those before it.
//!
//! [`race`] can keep the searches apart instead, each then the search it would be alone and a refutation the winner's
//! learned clauses. On the SAT Competition 2024 sample at 60 s of wall time sharing decides 60 of 62 formulas against
//! 59 apart (PAR-2 769 against 853, and 770 against 860 in a second run).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::cdcl::{Lit, RestartMode, SolveResult, Solver};

/// One search's heuristics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub restart: RestartMode,
    /// Bump the reason side of each learned clause ([`Solver::set_bump_reasons`]).
    pub bump_reasons: bool,
    /// Decide by the move-to-front queue in focused mode ([`Solver::set_focused_queue`]).
    pub focused_queue: bool,
    /// Decide each variable true first, where the core decides false first.
    pub positive_phases: bool,
    /// Backtrack chronologically past a backjump of more than this many levels ([`Solver::set_chrono`]).
    pub chrono: Option<u32>,
}

impl Config {
    /// The core's own heuristics.
    pub const CORE: Config = Config {
        restart: RestartMode::Adaptive,
        bump_reasons: false,
        focused_queue: false,
        positive_phases: false,
        chrono: None,
    };
}

/// Four searches that win on different families: the core; reason-side bumping, which decides the SAT Competition
/// tilings; a focused search, Glucose restarts deciding by the move-to-front queue and backtracking chronologically,
/// which decides the circuits that only focused mode does; and the calm Luby restarts with positive phases.
pub const DEFAULT: [Config; 4] = [
    Config::CORE,
    Config { bump_reasons: true, ..Config::CORE },
    Config { restart: RestartMode::Glucose, focused_queue: true, chrono: Some(100), ..Config::CORE },
    Config { restart: RestartMode::Luby, positive_phases: true, ..Config::CORE },
];

/// The conflicts each search may spend in the first round.
pub const FIRST_BUDGET: u64 = 1000;

/// How a race ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub result: SolveResult,
    /// The configuration that decided.
    pub winner: usize,
    /// With [`SolveResult::Unsat`], a RUP refutation of the formula: the winner's learned clauses, or, in a race that
    /// shares, every search's round by round.
    pub proof: Vec<Vec<Lit>>,
    /// The winner's conflicts, and each other search's at the end of the last round before the race's.
    pub conflicts: Vec<u64>,
    /// The rounds run; 0 when a search decided before its first.
    pub rounds: usize,
}

/// Race `configs` on `clauses` over `num_vars` variables, `first_budget` conflicts each in the first round, sharing
/// between rounds.
pub fn solve(num_vars: usize, clauses: &[Vec<Lit>], configs: &[Config], first_budget: u64) -> Outcome {
    race(num_vars, clauses, configs, first_budget, true)
}

/// [`solve`], sharing each round's units and binaries between the searches when `share` says so, apart otherwise.
pub fn race(num_vars: usize, clauses: &[Vec<Lit>], configs: &[Config], first_budget: u64, share: bool) -> Outcome {
    assert!(!configs.is_empty(), "a race needs a configuration");
    let stops: Vec<Arc<AtomicBool>> = configs.iter().map(|_| Arc::new(AtomicBool::new(false))).collect();
    let mut solvers: Vec<Solver> = configs.iter().map(|c| configured(num_vars, clauses, c)).collect();
    for (solver, stop) in solvers.iter_mut().zip(&stops) {
        solver.set_stop(Some(stop.clone()));
    }
    let mut decided: Vec<Option<SolveResult>> = solvers.iter_mut().map(|s| s.begin(0)).collect();
    let mut before: Vec<u64> = vec![0; configs.len()];
    // The proof so far: every search's learned clauses through the last round, round by round; and where each search's
    // log stood then.
    let mut proof: Vec<Vec<Lit>> = Vec::new();
    let mut logged: Vec<usize> = vec![0; configs.len()];
    let (mut budget, mut rounds) = (first_budget.max(1), 0);
    loop {
        if let Some(winner) = decided.iter().position(Option::is_some) {
            let result = decided.swap_remove(winner).expect("the winner decided");
            before[winner] = solvers[winner].conflicts();
            proof.extend(solvers[winner].learned()[logged[winner]..].iter().map(|c| c.lits.clone()));
            return Outcome { result, winner, proof, conflicts: before, rounds };
        }
        let mut learned: Vec<Vec<Vec<Lit>>> = Vec::with_capacity(solvers.len());
        for (s, at) in solvers.iter().zip(logged.iter_mut()) {
            let round = &s.learned()[*at..];
            proof.extend(round.iter().map(|c| c.lits.clone()));
            learned.push(round.iter().filter(|c| c.lits.len() <= 2).map(|c| c.lits.clone()).collect());
            *at = s.learned().len();
        }
        if share {
            for (i, s) in solvers.iter_mut().enumerate() {
                let others: Vec<Vec<Lit>> =
                    learned.iter().enumerate().filter(|&(j, _)| j != i).flat_map(|(_, c)| c.iter().cloned()).collect();
                if !s.import(&others) {
                    decided[i] = Some(SolveResult::Unsat);
                }
            }
            for (s, at) in solvers.iter().zip(logged.iter_mut()) {
                proof.extend(s.learned()[*at..].iter().map(|c| c.lits.clone()));
                *at = s.learned().len();
            }
            if decided.iter().any(Option::is_some) {
                continue;
            }
        }
        before = solvers.iter().map(Solver::conflicts).collect();
        for stop in &stops {
            stop.store(false, Ordering::Relaxed);
        }
        rounds += 1;
        decided = std::thread::scope(|scope| {
            let stops = &stops;
            let searches: Vec<_> = solvers
                .iter_mut()
                .enumerate()
                .map(|(i, s)| {
                    scope.spawn(move || {
                        let verdict = s.resume(&mut [], budget);
                        if verdict.is_some() {
                            for later in &stops[i + 1..] {
                                later.store(true, Ordering::Relaxed);
                            }
                        }
                        verdict
                    })
                })
                .collect();
            searches.into_iter().map(|search| search.join().expect("a search thread")).collect()
        });
        budget = budget.saturating_mul(2);
    }
}

fn configured(num_vars: usize, clauses: &[Vec<Lit>], config: &Config) -> Solver {
    let mut solver = Solver::new(num_vars);
    solver.set_restart_mode(config.restart);
    solver.set_bump_reasons(config.bump_reasons);
    solver.set_focused_queue(config.focused_queue);
    solver.set_chrono(config.chrono);
    if config.positive_phases {
        solver.set_initial_phase(&vec![true; num_vars]);
    }
    for c in clauses {
        solver.add_clause(c.clone());
    }
    solver
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/portfolio/tests.rs"]
mod tests;
