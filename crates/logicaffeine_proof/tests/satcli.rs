//! `satcli` — the shared SAT command-line driver behind both `logos-sat`
//! and `largo sat`. Competition conventions: exit 10 = SAT (with a `v`
//! model), 20 = UNSAT (optional DRAT proof), 1 = usage/parse/IO error.

use logicaffeine_proof::satcli;

fn run_on(
    content: Option<&str>,
    proof: bool,
) -> (u8, String, String, Option<String>) {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.cnf");
    if let Some(text) = content {
        std::fs::write(&input, text).unwrap();
    }
    let proof_path = dir.path().join("proof.drat");
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = satcli::run(
        &input,
        proof.then(|| proof_path.as_path()),
        false,
        &mut out,
        &mut err,
    );
    let proof_text = std::fs::read_to_string(&proof_path).ok();
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
        proof_text,
    )
}

#[test]
fn satisfiable_instance_returns_10_with_model() {
    let (code, out, _, _) = run_on(Some("p cnf 2 2\n1 2 0\n-1 0\n"), false);
    assert_eq!(code, 10);
    assert!(out.contains("s SATISFIABLE"), "out:\n{out}");
    let v_line = out.lines().find(|l| l.starts_with("v ")).expect("model line");
    assert!(v_line.ends_with(" 0"), "model terminated by 0: {v_line}");
    assert!(v_line.contains("-1"), "x1 must be false: {v_line}");
    assert!(v_line.contains(" 2"), "x2 must be true: {v_line}");
}

#[test]
fn unsatisfiable_instance_returns_20() {
    let (code, out, _, _) = run_on(Some("p cnf 1 2\n1 0\n-1 0\n"), false);
    assert_eq!(code, 20);
    assert!(out.contains("s UNSATISFIABLE"), "out:\n{out}");
}

#[test]
fn unsat_with_proof_writes_nonempty_drat() {
    let (code, _, _, proof) = run_on(Some("p cnf 1 2\n1 0\n-1 0\n"), true);
    assert_eq!(code, 20);
    let drat = proof.expect("a DRAT proof must be written");
    assert!(!drat.trim().is_empty(), "proof must be non-empty");
}

#[test]
fn missing_file_is_an_error() {
    let (code, _, err, _) = run_on(None, false);
    assert_eq!(code, 1);
    assert!(err.contains("cannot read"), "err:\n{err}");
}

#[test]
fn malformed_dimacs_is_an_error() {
    let (code, _, err, _) = run_on(Some("this is not dimacs"), false);
    assert_eq!(code, 1);
    assert!(err.contains("DIMACS"), "err:\n{err}");
}
