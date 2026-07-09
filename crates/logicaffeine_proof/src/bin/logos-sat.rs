//! `logos-sat` — competition-style command-line SAT solver.
//!
//! Invocation mirrors the SAT Competition main-track interface:
//!
//! ```text
//! logos-sat <input.cnf> [proof.drat]
//! ```
//!
//! It reads a DIMACS CNF, runs the certified CDCL engine, and writes the standard solution to
//! stdout: a `s SATISFIABLE` / `s UNSATISFIABLE` line, and for SAT a `v …` model terminated by
//! `0`. When a second path is given and the verdict is UNSAT, a DRAT refutation (built from the
//! engine's learned-clause log) is written there for `drat-trim`. Exit status follows the
//! competition convention: `10` = SAT, `20` = UNSAT, `1` = usage/parse/IO error.
//!
//! Wall-clock limits are the harness's job (`timeout`/`runsolver`), exactly as in the
//! competition; the binary itself runs the search to completion. The driver itself lives in
//! [`logicaffeine_proof::satcli`], shared verbatim with `largo sat`.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let positional: Vec<&String> = args[1..].iter().filter(|a| !a.starts_with('-')).collect();

    let Some(input) = positional.first() else {
        eprintln!("usage: logos-sat <input.cnf> [proof.drat]");
        return ExitCode::from(1);
    };
    let proof_path = positional.get(1).map(|p| Path::new(p.as_str()));
    let stats = std::env::var("LOGOS_STATS").is_ok();

    let code = logicaffeine_proof::satcli::run(
        Path::new(input.as_str()),
        proof_path,
        stats,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    );
    ExitCode::from(code)
}
