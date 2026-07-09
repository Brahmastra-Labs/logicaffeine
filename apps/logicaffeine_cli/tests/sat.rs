//! `largo sat <file.cnf> [--proof out.drat] [--stats]` — the certified SAT
//! solver as a CLI verb. Competition exit codes: 10 SAT, 20 UNSAT, 1 error.

mod common;

use common::*;
use tempfile::tempdir;

const SAT_CNF: &str = "p cnf 2 2\n1 2 0\n-1 0\n";
const UNSAT_CNF: &str = "p cnf 1 2\n1 0\n-1 0\n";

/// SAT: `s SATISFIABLE`, a `v` model line, exit 10.
#[test]
fn satisfiable_cnf_exits_10_with_model() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("sat.cnf");
    std::fs::write(&path, SAT_CNF).unwrap();

    let out = largo_in(dir.path(), &["sat", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(10), "sat: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("s SATISFIABLE"), "{text}");
    assert!(text.lines().any(|l| l.starts_with("v ")), "{text}");
}

/// UNSAT: `s UNSATISFIABLE`, exit 20.
#[test]
fn unsatisfiable_cnf_exits_20() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("unsat.cnf");
    std::fs::write(&path, UNSAT_CNF).unwrap();

    let out = largo_in(dir.path(), &["sat", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(20), "unsat: {}", stderr(&out));
    assert!(stdout(&out).contains("s UNSATISFIABLE"));
}

/// `--proof` writes a DRAT refutation for UNSAT instances.
#[test]
fn proof_flag_writes_drat() {
    let dir = tempdir().unwrap();
    let cnf = dir.path().join("unsat.cnf");
    let drat = dir.path().join("out.drat");
    std::fs::write(&cnf, UNSAT_CNF).unwrap();

    let out = largo_in(
        dir.path(),
        &["sat", cnf.to_str().unwrap(), "--proof", drat.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(20));
    let proof = std::fs::read_to_string(&drat).expect("DRAT file written");
    assert!(!proof.trim().is_empty());
}

/// Missing input file is an error (exit 1).
#[test]
fn missing_cnf_is_an_error() {
    let dir = tempdir().unwrap();
    let out = largo_in(dir.path(), &["sat", "no_such.cnf"]);
    assert_eq!(out.status.code(), Some(1));
}

/// `largo sat` output equals the shared satcli driver's output on the same
/// input — both wrappers must stay in lockstep.
#[test]
fn output_matches_shared_driver() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("sat.cnf");
    std::fs::write(&path, SAT_CNF).unwrap();

    let mut oracle_out = Vec::new();
    let mut oracle_err = Vec::new();
    let oracle_code =
        logicaffeine_proof::satcli::run(&path, None, false, &mut oracle_out, &mut oracle_err);

    let out = largo_in(dir.path(), &["sat", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(oracle_code as i32));
    assert_eq!(stdout(&out), String::from_utf8(oracle_out).unwrap());
}
