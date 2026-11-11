//! cdcl_stats — the raw CDCL core on DIMACS files, with its search counters and throughput.
//!
//! Every file is solved by [`Solver::solve`] alone — the path the dispatcher's CDCL fallback takes, without the
//! structure recognizers or a proof re-check — so the numbers isolate the search engine: conflicts, decisions,
//! propagations, restarts, conflicts per second and propagations per second, the figures Kissat prints, for an A/B
//! against it and against an earlier build of ours. A SAT model is checked against the formula before the line is
//! printed; a model that fails is reported, never counted.
//!
//! Usage: cargo run --release -p logicaffeine-proof --example cdcl_stats -- <file.cnf>...
//! A wall-clock limit is the caller's (`timeout 60 …`, one file per call): the solve runs to its verdict.
//! Env:   CDCL_TARGET=0  CDCL_WALK=0  CDCL_LUCKY=0  CDCL_INPROCESS=0  CDCL_INPROCESS_INTERVAL=<conflicts>
//!        CDCL_RESTART=adaptive|glucose|luby  CDCL_REDUCE=tiered  CDCL_VMTF=1  CDCL_CHRONO=<levels>
//!        CDCL_BUMPREASONS=1 — the A/B switches.
//!        CDCL_CONFLICTS=<n> stops the search after n conflicts (verdict `budget`), for the search rate on an instance
//!        that would run long.
//!        CDCL_PREPROCESS=1 first simplifies the formula ([`logicaffeine_proof::preprocess`]), solves the simplified
//!        one and extends its model; the seconds then include the preprocessing, and a `preprocess:` line reports what
//!        it removed. CDCL_FACTOR=1 adds bounded variable addition to it, CDCL_EFFORT=<visits> sets its effort and
//!        CDCL_BOUND=<clauses> the clauses an elimination may add.
//!        CDCL_SHUFFLE=<seed> solves a copy of each file with its variables renamed and their signs flipped, its
//!        clauses and their literals reordered, all by a seeded permutation — the same formula up to names, for
//!        sampling a configuration's spread over runs that differ only in the order the search meets it.
//!        CDCL_PORTFOLIO=1 races `portfolio::DEFAULT` on the (preprocessed) formula instead, one thread a configuration,
//!        and reports the winner, the rounds and every search's conflicts in a `portfolio:` line; the counters of the
//!        verdict line are then the unused solver's. They share between rounds, as `portfolio::solve` has them;
//!        CDCL_PORTFOLIO_SHARE=0 keeps them apart.
//!        CDCL_CUBE=<depth> splits the (preprocessed) formula into cubes by lookahead and conquers them instead
//!        ([`logicaffeine_proof::cube`]), CDCL_CUBE_THREADS=<n> solvers on threads (4 unless given) and
//!        CDCL_CUBE_PRESELECT=<vars> looked ahead on at a node (40 unless given); a `cube:` line reports the cubes, the
//!        nodes the lookahead refuted, the answering cube, the refutation's clauses and the seconds the split and the
//!        conquest took together, and the counters of the verdict line are then the unused solver's.
//!        CDCL_PREFIX=1 first runs the dispatcher's specialist chain
//!        ([`logicaffeine_proof::solve::structured_prefix`]) on the formula and reports its seconds and the route that
//!        fired, if any, in a `prefix:` line, outside the solve's seconds: the cost every instance pays before CDCL.
//!        CDCL_XOR=watched|incremental reads the formula's XOR equations ([`logicaffeine_proof::lyapunov::extract_xor`])
//!        and solves with them as a Gauss–Jordan theory: the watched matrix
//!        ([`logicaffeine_proof::xor_engine::WatchedXor`]) or the incremental one
//!        ([`logicaffeine_proof::xor_engine::IncXor`]); an `xor:` line reports the equations and the seconds reading
//!        them. CDCL_XOR_KERNEL=1 decides only the variables the matrix leaves non-basic. Not with CDCL_CONFLICTS.

use std::time::Instant;

use logicaffeine_proof::cdcl::{BudgetedResult, Lit, ReducePolicy, RestartMode, SolveResult, Solver, Theory};
use logicaffeine_proof::xor_engine::{IncXor, WatchedXor};
use logicaffeine_proof::{dimacs, lyapunov, preprocess, solve};

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: cdcl_stats <file.cnf>...");
        std::process::exit(2);
    }
    println!(
        "{:<28} {:>7} {:>10} {:>10} {:>12} {:>8} {:>8} {:>10} {:>11}",
        "file", "verdict", "seconds", "conflicts", "propagations", "restarts", "dec/conf", "confl/s", "props/s"
    );
    for path in &files {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{path}: {e}");
                continue;
            }
        };
        let cnf = match dimacs::parse(&text) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{path}: {e:?}");
                continue;
            }
        };
        let cnf = match std::env::var("CDCL_SHUFFLE").ok().and_then(|v| v.parse::<u64>().ok()) {
            Some(seed) => shuffled(cnf, seed),
            None => cnf,
        };
        if std::env::var("CDCL_PREFIX").is_ok_and(|v| v == "1") {
            let start = Instant::now();
            let fired = solve::structured_prefix(cnf.num_vars, &cnf.clauses);
            eprintln!(
                "prefix: {:.3}s via={}",
                start.elapsed().as_secs_f64(),
                fired.map_or_else(|| "none".to_string(), |s| format!("{:?}", s.via))
            );
        }
        let start = Instant::now();
        let preprocessed = std::env::var("CDCL_PREPROCESS").is_ok_and(|v| v == "1").then(|| {
            let factor = std::env::var("CDCL_FACTOR").is_ok_and(|v| v == "1");
            let mut options = preprocess::Options { factor, ..Default::default() };
            if let Some(effort) = std::env::var("CDCL_EFFORT").ok().and_then(|v| v.parse().ok()) {
                options.effort = effort;
            }
            if let Some(bound) = std::env::var("CDCL_BOUND").ok().and_then(|v| v.parse().ok()) {
                options.bound = bound;
            }
            let p = preprocess::preprocess(cnf.num_vars, &cnf.clauses, &options);
            eprintln!(
                "preprocess: {:.3}s eliminated={} substituted={} congruent={} factored={} subsumed={} strengthened={} fixed={} clauses {}→{}",
                start.elapsed().as_secs_f64(),
                p.eliminated,
                p.substituted,
                p.congruent,
                p.factored,
                p.subsumed,
                p.strengthened,
                p.fixed,
                cnf.clauses.len(),
                p.clauses.len()
            );
            p
        });
        let num_vars = preprocessed.as_ref().map_or(cnf.num_vars, |p| p.num_vars);
        let clauses = preprocessed.as_ref().map_or(&cnf.clauses, |p| &p.clauses);
        let mut solver = Solver::new(num_vars);
        configure(&mut solver);
        for c in clauses {
            solver.add_clause(c.clone());
        }
        let xor = std::env::var("CDCL_XOR").ok().map(|engine| {
            let read = Instant::now();
            let equations = lyapunov::extract_xor(num_vars, clauses);
            let kernel = std::env::var("CDCL_XOR_KERNEL").is_ok_and(|v| v == "1");
            let theory: Box<dyn Theory> = match engine.as_str() {
                "watched" => {
                    let watched = WatchedXor::new(num_vars, &equations);
                    if kernel {
                        solver.set_decision_vars(&watched.decision_vars());
                    }
                    Box::new(watched)
                }
                "incremental" => {
                    let incremental = IncXor::new(num_vars, &equations);
                    if kernel {
                        solver.set_decision_vars(&incremental.decision_vars());
                    }
                    Box::new(incremental)
                }
                other => panic!("CDCL_XOR={other}: watched or incremental"),
            };
            eprintln!("xor: {:.3}s equations={}", read.elapsed().as_secs_f64(), equations.len());
            theory
        });
        let budget: Option<u64> = std::env::var("CDCL_CONFLICTS").ok().and_then(|v| v.parse().ok());
        let portfolio = std::env::var("CDCL_PORTFOLIO").is_ok_and(|v| v == "1");
        let cube: Option<usize> = std::env::var("CDCL_CUBE").ok().and_then(|v| v.parse().ok());
        let result = match (budget, xor) {
            (_, Some(theory)) => Some(solver.solve_with(&mut [theory])),
            (None, None) if cube.is_some() => {
                let number =
                    |key: &str, default: usize| std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default);
                let opts = logicaffeine_proof::cube::Options {
                    depth: cube.unwrap_or_default(),
                    threads: number("CDCL_CUBE_THREADS", 4),
                    preselect: number("CDCL_CUBE_PRESELECT", 40),
                };
                let cubed = Instant::now();
                let outcome = logicaffeine_proof::cube::solve(num_vars, clauses, &opts);
                eprintln!(
                    "cube: cubes={} refuted={} cube={:?} proof={} seconds={:.3}",
                    outcome.cubes,
                    outcome.refuted,
                    outcome.cube,
                    outcome.proof.len(),
                    cubed.elapsed().as_secs_f64()
                );
                Some(outcome.result)
            }
            (None, None) if portfolio => {
                let share = std::env::var("CDCL_PORTFOLIO_SHARE").map_or(true, |v| v != "0");
                let raced = logicaffeine_proof::portfolio::race(num_vars, clauses, &logicaffeine_proof::portfolio::DEFAULT, logicaffeine_proof::portfolio::FIRST_BUDGET, share);
                eprintln!("portfolio: winner={} rounds={} conflicts={:?}", raced.winner, raced.rounds, raced.conflicts);
                Some(raced.result)
            }
            (Some(n), None) => match solver.solve_budgeted(n) {
                BudgetedResult::Sat(model) => Some(SolveResult::Sat(model)),
                BudgetedResult::Unsat => Some(SolveResult::Unsat),
                BudgetedResult::Budget => None,
            },
            (None, None) => Some(solver.solve()),
        };
        let verdict = match (&preprocessed, result) {
            (Some(p), _) if p.unsat => "UNSAT",
            (_, None) => "budget",
            (Some(p), Some(SolveResult::Sat(mut model))) => {
                p.extend(&mut model);
                if satisfies(&cnf.clauses, &model) { "SAT" } else { "BADMODEL" }
            }
            (None, Some(SolveResult::Sat(model))) if satisfies(&cnf.clauses, &model) => "SAT",
            (None, Some(SolveResult::Sat(_))) => "BADMODEL",
            (_, Some(SolveResult::Unsat)) => "UNSAT",
        };
        let secs = start.elapsed().as_secs_f64();
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        println!(
            "{:<28} {:>7} {:>10.3} {:>10} {:>12} {:>8} {:>8.2} {:>10.0} {:>11.0}",
            name,
            verdict,
            secs,
            solver.conflicts(),
            solver.propagations(),
            solver.restarts(),
            solver.decisions() as f64 / solver.conflicts().max(1) as f64,
            solver.conflicts() as f64 / secs.max(1e-9),
            solver.propagations() as f64 / secs.max(1e-9),
        );
    }
}

fn configure(solver: &mut Solver) {
    let off = |name: &str| std::env::var(name).is_ok_and(|v| v == "0");
    if off("CDCL_TARGET") {
        solver.set_target_phases(false);
    }
    if off("CDCL_WALK") {
        solver.set_walk(false);
    }
    if off("CDCL_LUCKY") {
        solver.set_lucky(false);
    }
    if std::env::var("CDCL_VMTF").is_ok_and(|v| v == "1") {
        solver.set_focused_queue(true);
    }
    if std::env::var("CDCL_BUMPREASONS").is_ok_and(|v| v == "1") {
        solver.set_bump_reasons(true);
    }
    if let Some(levels) = std::env::var("CDCL_CHRONO").ok().and_then(|v| v.parse().ok()) {
        solver.set_chrono(Some(levels));
    }
    if off("CDCL_INPROCESS") {
        solver.set_inprocess(false);
    }
    if let Some(interval) = std::env::var("CDCL_INPROCESS_INTERVAL").ok().and_then(|v| v.parse().ok()) {
        solver.set_inprocess_interval(interval);
    }
    if std::env::var("CDCL_REDUCE").is_ok_and(|v| v == "tiered") {
        solver.set_reduce_policy(ReducePolicy::Tiered);
    }
    match std::env::var("CDCL_RESTART").as_deref() {
        Ok("glucose") => solver.set_restart_mode(RestartMode::Glucose),
        Ok("luby") => solver.set_restart_mode(RestartMode::Luby),
        _ => {}
    }
}

/// `cnf` with its variables renamed by a seeded permutation and their signs flipped, its clauses and each clause's
/// literals reordered: satisfiable exactly when `cnf` is.
fn shuffled(mut cnf: dimacs::DimacsCnf, seed: u64) -> dimacs::DimacsCnf {
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let shuffle = |n: usize, next: &mut dyn FnMut() -> u64| -> Vec<usize> {
        let mut order: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            order.swap(i, (next() % (i as u64 + 1)) as usize);
        }
        order
    };
    let name = shuffle(cnf.num_vars, &mut next);
    let flip: Vec<bool> = (0..cnf.num_vars).map(|_| next() & 1 == 1).collect();
    for c in cnf.clauses.iter_mut() {
        for l in c.iter_mut() {
            let v = l.var() as usize;
            *l = Lit::new(name[v] as u32, l.is_positive() != flip[v]);
        }
        let order = shuffle(c.len(), &mut next);
        *c = order.into_iter().map(|i| c[i]).collect();
    }
    let order = shuffle(cnf.clauses.len(), &mut next);
    cnf.clauses = order.into_iter().map(|i| std::mem::take(&mut cnf.clauses[i])).collect();
    cnf
}

fn satisfies(clauses: &[Vec<Lit>], model: &[bool]) -> bool {
    clauses.iter().all(|c| c.iter().any(|l| model.get(l.var() as usize).copied() == Some(l.is_positive())))
}
