//! The SAT command-line driver shared by `logos-sat` and `largo sat`.
//!
//! One implementation of the SAT Competition main-track interface — read a
//! DIMACS CNF, run the certified solve, print `s SATISFIABLE`/`s
//! UNSATISFIABLE` (+ a `v` model for SAT), optionally export an UNSAT
//! certificate — with the output streams injected so both binaries stay in
//! lockstep and the driver is directly testable.
//!
//! Exit codes follow the competition convention: `10` = SAT, `20` = UNSAT,
//! `1` = usage/parse/IO error.

use std::io::Write;
use std::path::Path;

use crate::cdcl::Lit;
use crate::dimacs;
use crate::proof_emit;
use crate::solve::{certified_proof, solve_structured, Answer, Solved};

/// Run the solver on `input`, writing the solution to `out` and comments /
/// diagnostics to `err`. When `proof_path` is given and the verdict is
/// UNSAT, an exportable certificate is written there. `stats` prints a
/// `c stats …` line to `err`.
pub fn run(
    input: &Path,
    proof_path: Option<&Path>,
    stats: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let text = match std::fs::read_to_string(input) {
        Ok(t) => t,
        Err(e) => {
            let _ = writeln!(err, "c error: cannot read {}: {e}", input.display());
            return 1;
        }
    };
    let cnf = match dimacs::parse(&text) {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(err, "c error: malformed DIMACS: {e:?}");
            return 1;
        }
    };

    let t = std::time::Instant::now();
    let solved = solve_structured(cnf.num_vars, &cnf.clauses);
    let secs = t.elapsed().as_secs_f64();
    if stats {
        let _ = writeln!(
            err,
            "c stats vars={} clauses={} time={:.3}s via={:?} conflicts={}",
            cnf.num_vars,
            cnf.clauses.len(),
            secs,
            solved.via,
            solved.conflicts
        );
    }
    match solved.answer {
        Answer::Sat(model) => {
            let _ = writeln!(out, "s SATISFIABLE");
            print_model(out, &model, cnf.num_vars);
            10
        }
        Answer::Unsat => {
            let _ = writeln!(out, "s UNSATISFIABLE");
            if let Some(path) = proof_path {
                write_proof(err, path, cnf.num_vars, &cnf.clauses, &solved);
            }
            20
        }
    }
}

/// Write the UNSAT proof [`certified_proof`] gives, checked against the formula alone: DRAT for RUP additions, DPR
/// when it carries PR steps whose witnesses reduce to assignments, else Heule's `.sr` for `sr2drat`. A route with no
/// clausal proof gets a note saying what its verdict rests on.
fn write_proof(err: &mut dyn Write, path: &Path, num_vars: usize, clauses: &[Vec<Lit>], solved: &Solved) {
    let display = path.display();
    let via = solved.via;
    let Some(steps) = certified_proof(num_vars, clauses, solved) else {
        let _ = writeln!(
            err,
            "c note: no clausal proof written — the {via:?} route's verdict rests on its own certificate, which has no clausal form here"
        );
        return;
    };
    let steps = steps.as_slice();
    if let Ok(drat) = proof_emit::emit_drat(num_vars, clauses, steps) {
        if let Err(e) = std::fs::write(path, drat) {
            let _ = writeln!(err, "c error: cannot write proof {display}: {e}");
        }
        return;
    }
    if let Ok(dpr) = proof_emit::emit_dpr(num_vars, clauses, steps) {
        if let Err(e) = std::fs::write(path, dpr) {
            let _ = writeln!(err, "c error: cannot write proof {display}: {e}");
        }
        return;
    }
    // Substitution-redundancy proof (the symmetry route): emit the `.sr` format, which `sr2drat`
    // expands to plain DRAT for `drat-trim` to verify externally.
    match proof_emit::emit_sr(num_vars, clauses, steps) {
        Ok(sr) => {
            if let Err(e) = std::fs::write(path, sr) {
                let _ = writeln!(err, "c error: cannot write proof {display}: {e}");
            } else {
                let _ = writeln!(err, "c proof: substitution-redundancy (.sr) — verify with `sr2drat {display} | drat-trim`");
            }
        }
        Err(e) => {
            let _ = writeln!(err, "c warning: the checked proof has no standard text form here ({e:?})");
        }
    }
}

/// Print the model as wrapped `v` lines over variables `1..=num_vars`, terminated by `0`, per the
/// DIMACS solution convention. Variable `i` (0-based in the model) is literal `i+1`.
fn print_model(out: &mut dyn Write, model: &[bool], num_vars: usize) {
    const PER_LINE: usize = 20;
    let mut line = String::from("v");
    let mut on_line = 0usize;
    for v in 0..num_vars {
        let lit = if model.get(v).copied().unwrap_or(false) {
            (v + 1) as i64
        } else {
            -((v + 1) as i64)
        };
        line.push(' ');
        line.push_str(&lit.to_string());
        on_line += 1;
        if on_line == PER_LINE {
            let _ = writeln!(out, "{line}");
            line = String::from("v");
            on_line = 0;
        }
    }
    line.push_str(" 0");
    let _ = writeln!(out, "{line}");
}

/// The `logos-sat` process: `logos-sat <input.cnf> [proof.drat]`, with `LOGOS_STATS` for solver
/// statistics. `src/bin/logos-sat.rs` is exactly this call, and so is the test crate's
/// `logos-sat-under-test`.
pub fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let positional: Vec<&String> = args[1..].iter().filter(|a| !a.starts_with('-')).collect();

    let Some(input) = positional.first() else {
        eprintln!("usage: logos-sat <input.cnf> [proof.drat]");
        return std::process::ExitCode::from(1);
    };
    let proof_path = positional.get(1).map(|p| std::path::Path::new(p.as_str()));
    let stats = std::env::var("LOGOS_STATS").is_ok();

    let code = run(
        std::path::Path::new(input.as_str()),
        proof_path,
        stats,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    );
    std::process::ExitCode::from(code)
}
