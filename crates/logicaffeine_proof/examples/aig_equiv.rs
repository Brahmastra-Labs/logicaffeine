//! Decide whether two sequential circuits compute the same outputs, each a number of clocks late, from AIGER files:
//! `seq_equiv::aligned` (a pipeline against a pipeline, each from any state, inputs shared and outputs paired by name)
//! through the certified sweep, with its progress as it goes, its queries by the cut they were put at, its hardest
//! queries, and its certificate re-checked.
//!
//!     aig_equiv <left.aig> <left latency> <right.aig> <right latency> [--sweep-budget <conflicts>]
//!               [--final-budget <conflicts>] [--sim-words <words>] [--no-sweep] [--every <candidates>]
//!               [--miter <out.aig>] [--fraig [--rounds <n>] [--recycle <variables>] [--collect <clauses>]]
//!
//! A budget of 0 is unlimited; the defaults are `sweep::Options::default()`'s, or `fraig::Options::default()`'s with
//! `--fraig`. `--every` is how many candidates pass
//! between progress lines (1000 unless given). `--miter` writes the miter the decision is over instead of deciding it:
//! both sides unrolled to their latencies, inputs shared by name, each output pair's difference an output, which is 0
//! for every input exactly when the two agree (`&cec -m` in ABC reads it so). `--fraig` decides it by incremental SAT
//! sweeping instead (`fraig`: one solver, structural merges, one derivation), `--sweep-budget` its first round's query
//! budget and `--rounds` the most rounds it takes.

use std::process::exit;
use std::sync::Mutex;
use std::time::Instant;

use logicaffeine_proof::aig::Aig;
use logicaffeine_proof::fraig;
use logicaffeine_proof::seq_equiv::{aligned, aligned_pair};
use logicaffeine_proof::sweep::{miter, Options, Progress, Stats, Verdict, CUT_DEPTHS};

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("aig_equiv: {why}");
    exit(1)
}

fn number<T: std::str::FromStr>(flag: &str, v: Option<&String>) -> T {
    let v = v.unwrap_or_else(|| fail(format!("{flag} takes a value")));
    v.parse().unwrap_or_else(|_| fail(format!("{flag}: not a number: {v}")))
}

fn read(path: &str) -> Aig {
    let bytes = std::fs::read(path).unwrap_or_else(|e| fail(format!("{path}: {e}")));
    Aig::parse(&bytes).unwrap_or_else(|e| fail(format!("{path}: {e:?}")))
}

static START: Mutex<Option<Instant>> = Mutex::new(None);

fn elapsed() -> f64 {
    START.lock().ok().and_then(|t| *t).map_or(0.0, |t| t.elapsed().as_secs_f64())
}

/// The queries by the cut they were put at: queries, conflicts, refuted, models and budgets run out, each kind in turn.
fn by_cut(s: &Stats) -> String {
    let kinds = CUT_DEPTHS.iter().map(|d| format!("depth {d}")).chain(["meeting".to_string(), "whole".to_string()]);
    kinds
        .zip(&s.by_cut)
        .map(|(k, c)| format!("{k} {}q/{}c/{}r/{}m/{}b/{}v", c.queries, c.conflicts, c.refuted, c.models, c.budget, c.vars))
        .collect::<Vec<_>>()
        .join(", ")
}

fn progress(s: &Stats) {
    eprintln!(
        "  {:.1} s: swept {} of {}, {} candidates, {} merges, {} differences, {} unknown; {}",
        elapsed(),
        s.swept,
        s.cone,
        s.sat_calls,
        s.merges,
        s.counterexamples,
        s.unknown,
        by_cut(s)
    );
}

fn fraig_progress(s: &fraig::Stats) {
    eprintln!(
        "  {:.1} s: round {}, swept {} of {}, {} structural, {} queries, {} merges, {} counterexamples, {} unknown, {} conflicts, {} solvers, {} learned, {} forgotten, log {} ({} hints); queries by conflicts 0/≤10/≤100/≤1000/more {:?}",
        elapsed(),
        s.rounds,
        s.swept,
        s.cone,
        s.structural,
        s.queries,
        s.merges,
        s.counterexamples,
        s.unknown,
        s.conflicts,
        s.solvers,
        s.learned,
        s.forgotten,
        s.log,
        s.hints,
        s.by_conflicts
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!("usage: aig_equiv <left.aig> <left latency> <right.aig> <right latency> [--sweep-budget N] [--final-budget N] [--sim-words N] [--no-sweep] [--every N] [--miter <out.aig>]");
        exit(2);
    }
    let (left, right) = (read(&args[0]), read(&args[2]));
    let (ll, rl): (usize, usize) = (number("left latency", args.get(1)), number("right latency", args.get(3)));
    let mut opts = Options::default();
    let mut fopts = fraig::Options::default();
    let mut every = 1000usize;
    let mut write_miter: Option<String> = None;
    let mut by_fraig = false;

    let mut rest = args[4..].iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--sweep-budget" => {
                opts.sweep_budget = number(flag, rest.next());
                fopts.budget = opts.sweep_budget;
            }
            "--final-budget" => {
                opts.final_budget = number(flag, rest.next());
                fopts.final_budget = opts.final_budget;
            }
            "--sim-words" => {
                opts.sim_words = number(flag, rest.next());
                fopts.sim_words = opts.sim_words;
            }
            "--rounds" => fopts.rounds = number(flag, rest.next()),
            "--no-sweep" => opts.sweep = false,
            "--every" => every = number(flag, rest.next()),
            "--miter" => write_miter = Some(rest.next().unwrap_or_else(|| fail("--miter takes a file")).clone()),
            "--fraig" => by_fraig = true,
            "--recycle" => fopts.recycle = number(flag, rest.next()),
            "--collect" => fopts.collect = number(flag, rest.next()),
            other => fail(format!("unknown flag {other}")),
        }
    }
    opts.progress = Some(Progress { every, report: progress });
    println!(
        "left {}: {} inputs, {} latches, {} ands, {} outputs, {ll} clocks; right {}: {} inputs, {} latches, {} ands, {} outputs, {rl} clocks",
        args[0],
        left.inputs().len(),
        left.latches().len(),
        left.num_ands(),
        left.outputs().len(),
        args[2],
        right.inputs().len(),
        right.latches().len(),
        right.num_ands(),
        right.outputs().len()
    );
    if let Some(out) = write_miter {
        let (l, r) = aligned_pair(&left, ll, &right, rl);
        let m = miter(&l, &r).unwrap_or_else(|e| fail(format!("{e:?}")));
        std::fs::write(&out, m.aig.to_binary()).unwrap_or_else(|e| fail(format!("{out}: {e}")));
        println!("wrote {out}: {} inputs, {} ands, {} differences", m.aig.inputs().len(), m.aig.num_ands(), m.aig.outputs().len());
        return;
    }
    if let Ok(mut s) = START.lock() {
        *s = Some(Instant::now());
    }
    if by_fraig {
        let (l, r) = aligned_pair(&left, ll, &right, rl);
        fopts.progress = Some(fraig::Progress { every, report: fraig_progress });
        let report = fraig::equivalent(&l, &r, &fopts).unwrap_or_else(|e| fail(format!("{e:?}")));
        let decided = elapsed();
        let checks = report.check();
        let equal = report.verdicts.iter().filter(|(_, v)| *v == Verdict::Equal).count();
        println!(
            "{equal} of {} outputs equal in {decided:.1} s, the derivation {} ({:.1} s to check); {:?}",
            report.verdicts.len(),
            if checks { "checks" } else { "DOES NOT CHECK" },
            elapsed() - decided,
            report.stats
        );
        for (name, v) in report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal) {
            println!("  {name}: {v:?}");
        }
        if equal != report.verdicts.len() || !checks {
            exit(1);
        }
        return;
    }
    let report = aligned(&left, ll, &right, rl, &opts).unwrap_or_else(|e| fail(format!("{e:?}")));
    let decided = elapsed();
    let checks = report.check();
    let equal = report.verdicts.iter().filter(|(_, v)| *v == Verdict::Equal).count();
    let s = &report.stats;
    println!(
        "{equal} of {} outputs equal in {decided:.1} s, the certificate {} ({:.1} s to check); {} lemmas, {} candidates, {} merges, {} differences, {} unknown, miter {} ands",
        report.verdicts.len(),
        if checks { "checks" } else { "DOES NOT CHECK" },
        elapsed() - decided,
        report.certificate.lemmas.len(),
        s.sat_calls,
        s.merges,
        s.counterexamples,
        s.unknown,
        report.miter.aig.num_ands()
    );
    println!("queries by cut (queries/conflicts/refuted/models/budget/vars): {}", by_cut(s));
    for h in &s.hardest {
        println!("  hardest: {} conflicts over {} variables, cut of {}, {}", h.conflicts, h.vars, h.cut, h.verdict);
    }
    for (name, v) in report.verdicts.iter().filter(|(_, v)| *v != Verdict::Equal) {
        println!("  {name}: {v:?}");
    }
    if equal != report.verdicts.len() || !checks {
        exit(1);
    }
}
