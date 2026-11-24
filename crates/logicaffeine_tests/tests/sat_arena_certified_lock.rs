//! ★ LOCK — every answer the SAT arena credits to LOGOS is checked, or says why it is not. ★
//!
//! `benchmarks/results/arena/sat.json` is what the README's SAT chart and its "machine-checked" count are drawn from.
//! A satisfiable answer is checked by its model against the formula. An unsatisfiable one is checked by its proof:
//! after the timed run the arena (`crates/logicaffeine_proof/examples/arena_bench.rs`) re-solves it, untimed, writing
//! a DRAT proof that `drat-trim` must accept and turn into an LRAT certificate the verified `lrat-check` must accept
//! too. An answer whose proof the solver declines to write (past its proof budget) or a checker cannot finish is
//! recorded with that reason rather than counted. Held here:
//!
//! - every LOGOS answer is checked (`verified: true`) or names why not (`unchecked`), and a check that failed
//!   (`verified: false`) is a solver bug, of which there are none;
//! - the summary's count of checked answers is the instances' own.

use std::path::PathBuf;

fn results() -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/results/arena/sat.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).expect("sat.json is JSON")
}

fn ours(instance: &serde_json::Value) -> &serde_json::Value {
    &instance["ours"]
}

#[test]
fn every_answer_logos_gives_is_checked_or_says_why_not() {
    let r = results();
    let instances = r["instances"].as_array().expect("instances");
    let mut silent = Vec::new();
    let mut rejected = Vec::new();
    for inst in instances {
        let o = ours(inst);
        let verdict = o["verdict"].as_str().unwrap_or("");
        if verdict != "SAT" && verdict != "UNSAT" {
            continue;
        }
        match o["verified"].as_bool() {
            Some(true) => {}
            Some(false) => rejected.push(format!("{} ({verdict})", inst["name"])),
            None => {
                if o["unchecked"].as_str().is_none_or(str::is_empty) {
                    silent.push(format!("{} ({verdict})", inst["name"]));
                }
            }
        }
    }
    assert!(rejected.is_empty(), "a checker rejected these answers — a solver bug: {rejected:?}");
    assert!(
        silent.is_empty(),
        "{} answers are neither checked nor say why not: {:?}",
        silent.len(),
        &silent[..silent.len().min(12)]
    );
}

#[test]
fn the_summary_counts_the_checked_answers_the_instances_hold() {
    let r = results();
    let checked = r["instances"]
        .as_array()
        .expect("instances")
        .iter()
        .filter(|i| ours(i)["verified"].as_bool() == Some(true))
        .count() as u64;
    assert_eq!(r["summary"]["ours"]["verified"].as_u64(), Some(checked), "summary {:?}", r["summary"]["ours"]);
}
