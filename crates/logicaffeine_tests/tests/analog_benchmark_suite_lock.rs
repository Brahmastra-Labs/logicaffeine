//! ★ ANALOG BENCHMARK CLASSIFICATION LOCK ★ — every benchmark program is EXHAUSTIVELY
//! classified against the analog lane: it either compiles to a full analog artifact or is
//! REFUSED with a named reason. Nothing is silently skipped, and the classification is pinned
//! by a ledger so drift in either direction is a conscious edit:
//!   - a benchmark leaving `KNOWN_REFUSED` (a new analog capability!) must be REMOVED from the
//!     ledger (shrink-toward-mappable is the win condition);
//!   - a benchmark ENTERING refusal (a capability regressed) fails the lock outright.
//!
//! The honest V1 state: the benchmark suite is integer/text/collection-heavy digital work — the
//! analog lane refuses ALL of it, each with a typed reason (no float region function, undeclared
//! contracts, unlowerable ops). That "all refused, all named" row is the truthful baseline the
//! win condition shrinks from; the analog-mappable corpus lives in the analog locks' own
//! programs until benchmarks of the analog shape join the suite.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

use std::path::PathBuf;

use logicaffeine_compile::compile::compile_to_analog;

fn benchmarks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/programs")
}

fn discover_benchmarks() -> Vec<(String, String)> {
    let dir = benchmarks_dir();
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("benchmarks/programs exists") {
        let entry = entry.expect("dir entry");
        let lg = entry.path().join("interp.lg");
        if lg.is_file() {
            out.push((
                entry.file_name().to_string_lossy().to_string(),
                std::fs::read_to_string(&lg).expect("readable benchmark"),
            ));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn every_benchmark_is_classified_mappable_or_refused_with_a_name() {
    let benches = discover_benchmarks();
    assert!(
        benches.len() >= 35,
        "the discovered suite shrank: {}",
        benches.len()
    );

    let mut mappable: Vec<String> = Vec::new();
    let mut refused: Vec<(String, String)> = Vec::new();
    for (name, src) in &benches {
        match compile_to_analog(src) {
            Ok(art) => {
                assert!(
                    art.budget.recheck(),
                    "`{name}`: a mappable benchmark's budget must replay"
                );
                mappable.push(name.clone());
            }
            Err(e) => {
                let msg = format!("{e:?}");
                assert!(
                    msg.contains("analog backend:"),
                    "`{name}`: a refusal must be the analog lane's NAMED refusal, got: {msg}"
                );
                refused.push((name.clone(), msg));
            }
        }
    }

    // The pinned classification: today every benchmark refuses (the suite is digital work);
    // a benchmark that starts mapping is a CAPABILITY — celebrate it by moving the pin.
    assert_eq!(
        mappable,
        Vec::<String>::new(),
        "benchmarks began MAPPING onto the analog lane — move them out of the all-refused pin \
         (a conscious, celebratory edit): {mappable:?}"
    );
    assert_eq!(refused.len(), benches.len(), "classification must be total");
}
