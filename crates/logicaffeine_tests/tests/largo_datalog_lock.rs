//! ★ LOCK — `largo datalog`: a theory's rules evaluated bottom-up from the command line. ★
//!
//! `largo datalog FILE` reads the file's `## Theory` (and `## Axiom` blocks) as a Datalog program, evaluates it to its
//! least model, and prints every relation the rules derive. `--bdd` evaluates over binary decision diagrams and prints
//! each relation's diagram size beside it; `--certify` proves every derived fact in the kernel and says how many it
//! proved; `--json` prints the model as data. A theory that is not Datalog is refused with the axiom that is not.
//! `--contexts` numbers the call paths of the theory's `Calls` facts and evaluates the rules in every context: the
//! identity called twice keeps its two results apart, a relation over contexts is given by its size, and certifying
//! one is refused (the kernel proves from the theory's axioms, and the numbering is not among them).

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr, stdout};

const POINTS_TO: &str = "## Theory PointsTo

Axiom new_a: VP0(A, H1).
Axiom new_b: VP0(B, H2).
Axiom copy_c: Assign(C, A).
Axiom store_c: Store(C, Next, B).
Axiom load_d: Load(A, Next, D).
Axiom new_e: VP0(E, H3).
Axiom store_a: Store(A, Next, E).

Axiom alloc: for all v h, if VP0(v, h) then VP(v, h).
Axiom assign: for all v1 v2 h, if Assign(v1, v2) and VP(v2, h) then VP(v1, h).
Axiom store: for all v1 f v2 h1 h2, if Store(v1, f, v2) and VP(v1, h1) and VP(v2, h2) then HP(h1, f, h2).
Axiom load: for all v1 f v2 h1 h2, if Load(v1, f, v2) and VP(v1, h1) and HP(h1, f, h2) then VP(v2, h2).
";

fn datalog(args: &[&str], source: &str) -> (bool, String, String) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("points_to.lg"), source).expect("write source");
    let mut argv = vec!["datalog", "points_to.lg"];
    argv.extend_from_slice(args);
    let out = largo_in(dir.path(), &argv);
    (out.status.success(), stdout(&out), stderr(&out))
}

#[test]
fn it_prints_every_derived_relation() {
    let (ok, out, err) = datalog(&[], POINTS_TO);
    assert!(ok, "`largo datalog` succeeds:\n{err}");
    assert!(out.contains("VP (6)"), "VP has six facts:\n{out}");
    assert!(out.contains("(D, H3)"), "d points to h3:\n{out}");
    assert!(out.contains("HP (2)"), "HP has two facts:\n{out}");
    assert!(out.contains("(H1, Next, H3)"), "h1.next holds h3:\n{out}");
    assert!(!out.contains("(B, H1)"), "b never points to h1:\n{out}");
}

#[test]
fn with_bdd_it_prints_each_relations_diagram_size() {
    let (ok, out, err) = datalog(&["--bdd"], POINTS_TO);
    assert!(ok, "`largo datalog --bdd` succeeds:\n{err}");
    let vp = out.lines().find(|l| l.starts_with("VP (6)")).unwrap_or_else(|| panic!("a VP line:\n{out}"));
    assert!(vp.contains("BDD nodes"), "the VP line names its diagram size: {vp}");
}

#[test]
fn with_certify_every_derived_fact_is_kernel_proved() {
    let (ok, out, err) = datalog(&["--certify"], POINTS_TO);
    assert!(ok, "`largo datalog --certify` succeeds:\n{err}");
    assert!(out.contains("8 of 8 derived facts kernel-certified"), "VP's six and HP's two:\n{out}");
}

#[test]
fn with_json_it_prints_the_model_as_data() {
    let (ok, out, err) = datalog(&["--json"], POINTS_TO);
    assert!(ok, "`largo datalog --json` succeeds:\n{err}");
    let model: serde_json::Value = serde_json::from_str(out.trim()).expect("JSON");
    let vp = model["relations"]["VP"].as_array().expect("VP's tuples");
    assert_eq!(vp.len(), 6);
    assert!(vp.contains(&serde_json::json!(["D", "H3"])));
    assert_eq!(model["relations"]["HP"].as_array().map(Vec::len), Some(2));
}

#[test]
fn a_theory_that_is_not_datalog_is_refused_with_its_axiom() {
    let (ok, _, err) = datalog(&[], "## Theory Colors\n\nAxiom either: for all x, if Base(x) then Red(x) or Blue(x).\n");
    assert!(!ok, "a disjunctive conclusion is not a Horn rule");
    assert!(err.contains("either"), "the refusal names the axiom: {err}");
}

/// `x = id(a); y = id(b)` with `id(p) = p`, and Whaley and Lam's context-sensitive points-to rules.
const IDENTITY: &str = "## Theory Identity

Axiom new_a: VP0(A, H1).
Axiom new_b: VP0(B, H2).
Axiom main_a: Declares(Main, A).
Axiom main_b: Declares(Main, B).
Axiom main_x: Declares(Main, X).
Axiom main_y: Declares(Main, Y).
Axiom id_p: Declares(Id, P).
Axiom call_1: Calls(Main, S1, Id).
Axiom call_2: Calls(Main, S2, Id).
Axiom pass_1: Actual(S1, Z0, A).
Axiom pass_2: Actual(S2, Z0, B).
Axiom formal: Formal(Id, Z0, P).
Axiom returns: Returns(Id, P).
Axiom result_1: Receives(S1, X).
Axiom result_2: Receives(S2, Y).

Axiom alloc: for all v h m c, if VP0(v, h) and Declares(m, v) and Context(m, c) then VPC(c, v, h).
Axiom assign: for all c v1 v2 h, if Assign(v1, v2) and VPC(c, v2, h) then VPC(c, v1, h).
Axiom pass: for all c i d n z a f h, if CallContext(c, i, d, n) and Actual(i, z, a) and Formal(n, z, f) and VPC(c, a, h) then VPC(d, f, h).
Axiom result: for all c i d n v r h, if CallContext(c, i, d, n) and Returns(n, v) and Receives(i, r) and VPC(d, v, h) then VPC(c, r, h).
Axiom project: for all c v h, if VPC(c, v, h) then VP(v, h).
";

#[test]
fn with_contexts_each_call_path_is_its_own_context() {
    let (ok, out, err) = datalog(&["--contexts"], IDENTITY);
    assert!(ok, "`largo datalog --contexts` succeeds:\n{err}");
    assert!(out.contains("3 contexts over 2 methods, numbered in 1 bit"), "Main has one context, Id two:\n{out}");
    let vp = out.lines().find(|l| l.starts_with("VP (6)")).unwrap_or_else(|| panic!("a VP line:\n{out}"));
    assert!(vp.contains("(X, H1)") && vp.contains("(Y, H2)"), "each result holds what its call passed: {vp}");
    assert!(!vp.contains("(X, H2)") && !vp.contains("(Y, H1)"), "and nothing the other call passed: {vp}");
    let vpc = out.lines().find(|l| l.starts_with("VPC:")).unwrap_or_else(|| panic!("a VPC line:\n{out}"));
    assert!(vpc.contains("6 tuples over contexts") && vpc.contains("BDD nodes"), "counted, not listed: {vpc}");
}

#[test]
fn with_contexts_and_json_the_numbering_and_the_counts_are_data() {
    let (ok, out, err) = datalog(&["--contexts", "--json"], IDENTITY);
    assert!(ok, "`largo datalog --contexts --json` succeeds:\n{err}");
    let model: serde_json::Value = serde_json::from_str(out.trim()).expect("JSON");
    assert_eq!(model["contexts"], serde_json::json!(3));
    assert_eq!(model["methods"]["Id"], serde_json::json!(2));
    assert_eq!(model["methods"]["Main"], serde_json::json!(1));
    let vp = model["relations"]["VP"].as_array().expect("VP's tuples");
    assert_eq!(vp.len(), 6);
    assert!(vp.contains(&serde_json::json!(["X", "H1"])) && !vp.contains(&serde_json::json!(["X", "H2"])));
    assert!(model["relations"].get("VPC").is_none(), "a relation over contexts is not listed");
    assert_eq!(model["tuples"]["VPC"], serde_json::json!(6));
    assert!(model["bdd_nodes"]["VPC"].as_u64().is_some_and(|n| n > 0));
}

#[test]
fn contexts_are_not_certified() {
    let (ok, _, err) = datalog(&["--contexts", "--certify"], IDENTITY);
    assert!(!ok, "the numbering is not among the theory's axioms, so the kernel has nothing to prove VPC from");
    assert!(err.contains("--certify"), "the refusal names the flag: {err}");
}
