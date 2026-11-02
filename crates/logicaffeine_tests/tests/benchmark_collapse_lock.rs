//! A benchmark on which the LOGOS optimizer changes the algorithm says so, and says what it did.
//!
//! The /benchmarks page compares languages running the same algorithm. Some LOGOS optimizations replace the algorithm
//! instead — memoize a recursion, evaluate a loop in closed form, search half of a symmetric board — and each registry
//! row declares whether it does ([`OptWork`]). Tristen's rule: a collapse is a fair demonstration when the compiler
//! genuinely does less work, and it is disclosed. So for every benchmark, over every path `largo opts` traces (the
//! compiled path, the run path, the VM compile):
//!
//! - a benchmark any `OptWork::Less` optimization fires on carries a collapse note in `benchmarks/lib/benchmarks.tsv`
//!   (the page shows it and leaves the benchmark out of the same-algorithm geometric mean), and one that carries a
//!   note has such an optimization firing — no undisclosed collapse, no stale note;
//! - the note names what the compiler did, in the words of the optimization that fired: a note that says "closed
//!   form" over a memoized recursion (fib's once did) describes a transform that is not there;
//! - the note's word on each LOGOS row the run times is that row's own trace: a row it says runs the program "as
//!   written" changes no algorithm, a row the optimizer collapses it on is named (or the note speaks for "every LOGOS
//!   tier"), and a note that names no row speaks for all of them. fib's note once said LOGOS (C) runs the recursion as
//!   written long after the C tiers began memoizing it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test benchmark_collapse_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::path::PathBuf;

use logicaffeine_compile::compile::{
    compile_to_c_traced, compile_to_c_via_bytecode_traced, optimizations_fired,
    optimizations_fired_run, optimizations_fired_vm,
};
use logicaffeine_compile::optimization::{BackendOptimizationTrace, OptWork, REGISTRY};
use logicaffeine_jit::optimization_trace_jit;

fn benchmarks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks")
}

/// `column` of benchmark `id`'s row in benchmarks.tsv.
fn registry_field(id: &str, column: &str) -> String {
    let text = std::fs::read_to_string(benchmarks_dir().join("lib/benchmarks.tsv"))
        .expect("benchmarks.tsv");
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().expect("a header").split('\t').collect();
    let at = header
        .iter()
        .position(|h| *h == column)
        .unwrap_or_else(|| panic!("a {column} column"));
    lines
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .find(|f| f[0] == id)
        .unwrap_or_else(|| panic!("a {id} row"))[at]
        .to_string()
}

/// `(id, collapse note)` for every row of benchmarks.tsv.
fn collapse_notes() -> Vec<(String, String)> {
    let text = std::fs::read_to_string(benchmarks_dir().join("lib/benchmarks.tsv"))
        .expect("benchmarks.tsv");
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().expect("a header").split('\t').collect();
    let id = header
        .iter()
        .position(|h| *h == "id")
        .expect("an id column");
    let collapse = header
        .iter()
        .position(|h| *h == "collapse")
        .expect("a collapse column");
    lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f[id].to_string(), f[collapse].to_string())
        })
        .collect()
}

/// What a note must say when each algorithm-changing optimization fires — a stem, matched case-insensitively.
fn named_by(keyword: &str) -> &'static str {
    match keyword {
        "memo" => "memoiz",
        "closedform" => "closed form",
        "symmetry" => "symmetr",
        "densemap" => "array",
        "popcount" => "popcount",
        "comptime" => "compile time",
        "sortnest" => "heap sort",
        "divisorsieve" => "sieve",
        "tracememo" => "finished start",
        other => panic!(
            "`{other}` is an OptWork::Less optimization this lock has no wording for — add it"
        ),
    }
}

fn less_work() -> BTreeSet<&'static str> {
    REGISTRY
        .iter()
        .filter(|m| m.work == OptWork::Less)
        .map(|m| m.keyword)
        .collect()
}

#[test]
fn every_algorithm_changing_optimization_is_disclosed_in_its_own_words() {
    let less = less_work();
    for kw in &less {
        named_by(kw);
    }
    let mut wrong = Vec::new();
    for (id, note) in collapse_notes() {
        let src = std::fs::read_to_string(benchmarks_dir().join(format!("programs/{id}/main.lg")))
            .expect("main.lg");
        let fired: BTreeSet<&'static str> = optimizations_fired(&src)
            .into_iter()
            .chain(optimizations_fired_run(&src))
            .chain(optimizations_fired_vm(&src))
            .collect();
        let changed: Vec<&str> = fired.intersection(&less).copied().collect();
        match (changed.is_empty(), note.trim().is_empty()) {
            (false, true) => wrong.push(format!(
                "{id}: {changed:?} change the algorithm, and the row carries no collapse note"
            )),
            (true, false) => wrong.push(format!(
                "{id}: carries a collapse note, and nothing that fires changes the algorithm"
            )),
            _ => {}
        }
        let lower = note.to_lowercase();
        for kw in &changed {
            if !note.is_empty() && !lower.contains(named_by(kw)) {
                wrong.push(format!(
                    "{id}: `{kw}` fires, and the note never says \"{}\": {note:?}",
                    named_by(kw)
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "collapse notes out of step with the optimizer:\n  {}",
        wrong.join("\n  ")
    );
}

/// The LOGOS rows the publishable run times, by the label a note names them with, and the optimizations that fire for
/// `src` on each: the Rust and both C compiles, the JIT running the program at `n` through its eager optimizer, the VM
/// compile and the run path the tree-walker executes.
fn fired_by_tier(src: &str, n: &str) -> Vec<(&'static str, BTreeSet<&'static str>)> {
    let keywords = |trace: BackendOptimizationTrace| -> BTreeSet<&'static str> {
        trace.all_fired().keywords().into_iter().collect()
    };
    vec![
        (
            "LOGOS (Rust)",
            optimizations_fired(src).into_iter().collect(),
        ),
        (
            "LOGOS (C)",
            keywords(compile_to_c_traced(src).expect("the C compile").1),
        ),
        (
            "LOGOS (C, bytecode)",
            keywords(
                compile_to_c_via_bytecode_traced(src)
                    .expect("the bytecode C compile")
                    .1,
            ),
        ),
        (
            "LOGOS (JIT)",
            keywords(optimization_trace_jit(src, &[n.to_string()]).shared),
        ),
        (
            "LOGOS (VM)",
            optimizations_fired_vm(src).into_iter().collect(),
        ),
        (
            "LOGOS (tree-walker)",
            optimizations_fired_run(src).into_iter().collect(),
        ),
    ]
}

/// What a collapse note says about each LOGOS row, checked against that row's own trace. A clause (split at `;`, `.`
/// and `, and`) that says "as written" claims the rows it names change no algorithm; every row an algorithm-changing
/// optimization fires on is named outside such a clause, unless the note speaks for "every LOGOS tier"; and a note
/// that names no row speaks for every row, so each must change the algorithm.
#[test]
fn every_collapse_note_names_the_rows_it_collapses_on() {
    let less = less_work();
    let mut wrong = Vec::new();
    for (id, note) in collapse_notes() {
        if note.trim().is_empty() {
            continue;
        }
        let src = std::fs::read_to_string(benchmarks_dir().join(format!("programs/{id}/main.lg")))
            .expect("main.lg");
        let tiers = fired_by_tier(&src, &registry_field(&id, "small_size"));
        let collapsing = |fired: &BTreeSet<&'static str>| -> Vec<&'static str> {
            fired.intersection(&less).copied().collect()
        };
        let every = note.contains("every LOGOS tier");
        let mut named = BTreeSet::new();
        let mut named_collapsing = BTreeSet::new();
        for clause in note.split([';', '.']).flat_map(|c| c.split(", and ")) {
            let as_written = clause.contains("as written");
            for (tier, fired) in &tiers {
                if !clause.contains(tier) {
                    continue;
                }
                named.insert(*tier);
                let changed = collapsing(fired);
                if as_written && !changed.is_empty() {
                    wrong.push(format!(
                        "{id}: the note says {tier} runs it as written, and {changed:?} change the algorithm there"
                    ));
                }
                if !as_written {
                    named_collapsing.insert(*tier);
                }
            }
        }
        for (tier, fired) in &tiers {
            let changed = collapsing(fired);
            if named.is_empty() || every {
                if changed.is_empty() {
                    wrong.push(format!(
                        "{id}: the note speaks for every LOGOS row, and nothing that changes the algorithm fires on {tier}"
                    ));
                }
            } else if !changed.is_empty() && !named_collapsing.contains(tier) {
                wrong.push(format!(
                    "{id}: {changed:?} change the algorithm on {tier}, which the note never names: {note:?}"
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "collapse notes out of step with the rows they describe:\n  {}",
        wrong.join("\n  ")
    );
}
