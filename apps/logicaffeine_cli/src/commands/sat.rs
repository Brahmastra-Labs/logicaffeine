//! `largo sat` — the certified SAT solver as a CLI verb.
//!
//! A thin wrapper over [`logicaffeine_proof::satcli`], the exact driver
//! behind the competition `logos-sat` binary — same output, same
//! competition exit codes (10 SAT, 20 UNSAT, 1 error).

use std::path::PathBuf;

/// Handle `largo sat <file.cnf> [--proof out.drat] [--stats]`.
///
/// Never returns on completion: the process exits with the competition
/// code so scripts can rely on `10`/`20` exactly as with `logos-sat`.
pub(crate) fn cmd_sat(
    file: PathBuf,
    proof: Option<PathBuf>,
    stats: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let code = logicaffeine_proof::satcli::run(
        &file,
        proof.as_deref(),
        stats || std::env::var("LOGOS_STATS").is_ok(),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    );
    std::process::exit(code as i32);
}
