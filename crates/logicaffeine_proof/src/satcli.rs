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
use crate::proof::ProofStep;
use crate::proof_emit;
use crate::solve::{solve_structured, Answer, Route};

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
                write_proof(err, path, solved.via, cnf.num_vars, &cnf.clauses, &solved.proof);
            }
            20
        }
    }
}

/// Compile an algebraic route's refutation (GF(2) parity or GF(p) modular) to DRAT resolvent steps,
/// or `None` if the resolution route blows past its budget (then the verdict stands on the native
/// algebraic certificate, but no clausal proof is exported — see [`crate::xor_drat`]).
fn algebraic_drat(via: Route, num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<ProofStep>> {
    use crate::xor_drat::{emit_modp_drat, emit_xor_drat};
    let resolvents = match via {
        Route::Parity => {
            let eqs = crate::lyapunov::extract_xor(num_vars, clauses);
            let refutation = match crate::xorsat::solve(&eqs, num_vars) {
                crate::xorsat::XorOutcome::Unsat(s) => s,
                crate::xorsat::XorOutcome::Sat(_) => return None,
            };
            emit_xor_drat(&eqs, &refutation)?
        }
        Route::ModP => emit_modp_drat(num_vars, clauses)?,
        _ => return None,
    };
    Some(resolvents.into_iter().map(ProofStep::Rup).collect())
}

/// Write the UNSAT certificate: DRAT for a RUP-only proof (the CDCL route), DPR for a proof that
/// carries PR symmetry steps (the symmetry route), or the algebraic GF(2)/GF(p) bridge compiled to
/// DRAT (the parity / mod-p routes). The internal solve already verified it.
fn write_proof(
    err: &mut dyn Write,
    path: &Path,
    via: Route,
    num_vars: usize,
    clauses: &[Vec<Lit>],
    steps: &[ProofStep],
) {
    let display = path.display();
    // Parity / mod-p routes certify internally with a native algebraic witness and carry no clausal
    // steps; compile their linear-dependency refutation to a strict DRAT proof on demand.
    if matches!(via, Route::Parity | Route::ModP) {
        match algebraic_drat(via, num_vars, clauses) {
            Some(alg) => match proof_emit::emit_drat(num_vars, clauses, &alg) {
                Ok(drat) => {
                    if let Err(e) = std::fs::write(path, drat) {
                        let _ = writeln!(err, "c error: cannot write proof {display}: {e}");
                    } else {
                        let _ = writeln!(err, "c proof: DRAT via the {via:?} algebraic bridge — verify with `drat-trim {display}`");
                    }
                }
                Err(e) => {
                    let _ = writeln!(err, "c warning: algebraic proof not RUP-emittable ({e:?}) — verified internally");
                }
            },
            None => {
                let _ = writeln!(
                    err,
                    "c warning: {via:?} DRAT exceeded the resolution budget (would blow up) — verdict certified internally only"
                );
            }
        }
        return;
    }
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
            let _ = writeln!(err, "c warning: proof not exportable ({e:?}) — verified internally");
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
        let lit = if model.get(v).copied().unwrap_or(false) { (v + 1) as i64 } else { -((v + 1) as i64) };
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
