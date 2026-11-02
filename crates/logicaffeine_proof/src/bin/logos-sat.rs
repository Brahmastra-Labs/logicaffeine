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

use std::process::ExitCode;

fn main() -> ExitCode {
    logicaffeine_proof::satcli::main()
}
