//! End-to-end tests for the `logos-sat` competition binary: the DIMACS-in / `s …` + `v …` /
//! DRAT-out contract the SAT-Competition harness depends on. We run the actual built binary and
//! verify its solution and its emitted proof with the library's own checkers.

use std::path::PathBuf;
use std::process::Command;

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::dimacs;
use logicaffeine_proof::rup::check_refutation;

/// Path to the `logos-sat` binary under test.
///
/// `env!("CARGO_BIN_EXE_logos-sat")` bakes the *build-time* target path; when the
/// suite runs from a nextest archive (CI) that path doesn't exist in the fresh
/// test-job checkout. nextest re-exports the extracted binary at runtime via
/// `CARGO_BIN_EXE_logos-sat`, so prefer that, falling back to the compile-time
/// constant for a plain `cargo test`.
fn bin() -> std::ffi::OsString {
    std::env::var_os("CARGO_BIN_EXE_logos-sat")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_logos-sat").into())
}

fn tmp(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("logos_sat_{}_{}", std::process::id(), name));
    p
}

fn write(name: &str, contents: &str) -> PathBuf {
    let p = tmp(name);
    std::fs::write(&p, contents).unwrap();
    p
}

/// Run the binary; return (exit_code, stdout).
fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new(bin()).args(args).output().expect("spawn logos-sat");
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parse `v …` model lines from stdout into a per-variable assignment (1-based vars).
fn parse_model(stdout: &str, num_vars: usize) -> Vec<bool> {
    let mut assign = vec![false; num_vars];
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("v") else { continue };
        for tok in rest.split_whitespace() {
            let n: i64 = tok.parse().unwrap();
            if n == 0 {
                continue;
            }
            assign[(n.unsigned_abs() - 1) as usize] = n > 0;
        }
    }
    assign
}

fn lit(n: i64) -> Lit {
    Lit::new((n.unsigned_abs() - 1) as u32, n > 0)
}

#[test]
fn sat_instance_reports_satisfiable_with_a_real_model() {
    // (x1 ∨ x2) ∧ (¬x1 ∨ x3): satisfiable.
    let cnf = "p cnf 3 2\n1 2 0\n-1 3 0\n";
    let path = write("sat.cnf", cnf);
    let (code, out) = run(&[path.to_str().unwrap()]);

    assert_eq!(code, 10, "SAT must exit 10; stdout:\n{out}");
    assert!(out.lines().any(|l| l == "s SATISFIABLE"), "missing verdict line:\n{out}");

    let model = parse_model(&out, 3);
    let parsed = dimacs::parse(cnf).unwrap();
    for clause in &parsed.clauses {
        assert!(
            clause.iter().any(|l| model[l.var() as usize] == l.is_positive()),
            "reported model does not satisfy clause {clause:?}; stdout:\n{out}"
        );
    }
}

#[test]
fn unsat_instance_reports_unsatisfiable_and_writes_a_valid_drat_proof() {
    // (x1) ∧ (¬x1): unsatisfiable.
    let cnf = "p cnf 1 2\n1 0\n-1 0\n";
    let path = write("unsat.cnf", cnf);
    let proof = tmp("unsat.drat");
    let (code, out) = run(&[path.to_str().unwrap(), proof.to_str().unwrap()]);

    assert_eq!(code, 20, "UNSAT must exit 20; stdout:\n{out}");
    assert!(out.lines().any(|l| l == "s UNSATISFIABLE"), "missing verdict line:\n{out}");

    let drat = std::fs::read_to_string(&proof).expect("proof file written");
    let learned = parse_drat_additions(&drat);
    let parsed = dimacs::parse(cnf).unwrap();
    assert!(
        check_refutation(parsed.num_vars, &parsed.clauses, &learned),
        "emitted DRAT is not a valid refutation:\n{drat}"
    );
}

/// Parse the addition clauses of a DRAT proof (ignore `d …` deletion lines), preserving order.
fn parse_drat_additions(drat: &str) -> Vec<Vec<Lit>> {
    let mut clauses = Vec::new();
    for line in drat.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('d') {
            continue;
        }
        let mut clause = Vec::new();
        for tok in line.split_whitespace() {
            let n: i64 = tok.parse().unwrap();
            if n == 0 {
                break;
            }
            clause.push(lit(n));
        }
        clauses.push(clause);
    }
    clauses
}

#[test]
fn missing_argument_is_a_usage_error() {
    let (code, _) = run(&[]);
    assert_eq!(code, 1);
}

#[test]
fn malformed_dimacs_is_an_error_not_a_verdict() {
    let path = write("bad.cnf", "this is not dimacs\n");
    let (code, out) = run(&[path.to_str().unwrap()]);
    assert_eq!(code, 1, "stdout:\n{out}");
    assert!(!out.contains("s SATISFIABLE") && !out.contains("s UNSATISFIABLE"));
}
