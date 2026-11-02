//! The committed Lean Kernel Arena scoreboard is a ratchet.
//!
//! `scripts/lean/arena-crown/scoreboard.py --commit` measures logos-kernel on every live Arena test
//! exactly as the Arena does (instructions under `perf stat`) and writes
//! `benchmarks/results/lean-kernel-arena/{scoreboard,ceilings}.json`. This lock holds the
//! committed state to its promises: every measured verdict is correct, no test exceeds its
//! instruction ceiling, the first-place count never falls below its floor, and only the tests
//! pinned here may be missing from the measurement. A measurement and its ceiling belong to the
//! exact input bytes they were taken on: the corpus case's SHA-256 at the pinned revision.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Tests the committed scoreboard may leave unmeasured. Shrinks, never grows.
const MAY_BE_UNMEASURED: &[&str] = &[];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json(rel: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    serde_json::from_str(&text).unwrap()
}

#[test]
fn the_scoreboard_is_measured_against_the_pinned_live_revision() {
    let board = json("benchmarks/results/lean-kernel-arena/scoreboard.json");
    let ceilings = json("benchmarks/results/lean-kernel-arena/ceilings.json");
    let corpus = json("crates/logicaffeine_tests/tests/fixtures/arena_corpus/manifest.json");
    assert_eq!(board["schema"], "logos-arena-scoreboard/1");
    assert_eq!(ceilings["schema"], "logos-arena-ceilings/2");
    assert_eq!(
        board["live_revision"], corpus["revision"],
        "scoreboard and corpus pin the same Arena revision"
    );
    assert_eq!(
        ceilings["live_revision"], corpus["revision"],
        "the ceilings and the first-place floor belong to that revision"
    );
}

/// The SHA-256 of every corpus case, by name: the bytes a measurement is about.
fn corpus_digests() -> std::collections::BTreeMap<String, String> {
    let corpus = json("crates/logicaffeine_tests/tests/fixtures/arena_corpus/manifest.json");
    corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap().to_string(),
                c["upstream_sha256"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn every_measured_verdict_is_correct_and_within_its_ceiling() {
    let board = json("benchmarks/results/lean-kernel-arena/scoreboard.json");
    let ceilings = json("benchmarks/results/lean-kernel-arena/ceilings.json");
    let digests = corpus_digests();
    let results = board["results"].as_object().unwrap();
    assert!(!results.is_empty());
    for (name, r) in results {
        assert_eq!(r["status"], "correct", "{name}: {r}");
        let input = digests
            .get(name)
            .unwrap_or_else(|| panic!("{name} is not a corpus case"));
        assert_eq!(
            r["input_sha256"].as_str(),
            Some(input.as_str()),
            "{name}: measured on other bytes than the corpus case"
        );
        let instr = r["instructions"]
            .as_u64()
            .unwrap_or_else(|| panic!("{name} has no instruction count"));
        let ceiling = &ceilings["ceilings"][name];
        assert_eq!(
            ceiling["input_sha256"].as_str(),
            Some(input.as_str()),
            "{name}: the ceiling belongs to other bytes"
        );
        let limit = ceiling["instructions"]
            .as_u64()
            .unwrap_or_else(|| panic!("{name} has no ceiling"));
        assert!(
            instr <= limit,
            "{name}: {instr} instructions exceeds its ceiling {limit}"
        );
    }
    let firsts = board["first_place"].as_u64().unwrap();
    let floor = ceilings["first_place_floor"].as_u64().unwrap();
    assert!(
        firsts >= floor,
        "first place on {firsts} tests, below the floor of {floor}"
    );
}

#[test]
fn only_pinned_tests_may_be_unmeasured() {
    let board = json("benchmarks/results/lean-kernel-arena/scoreboard.json");
    let corpus = json("crates/logicaffeine_tests/tests/fixtures/arena_corpus/manifest.json");
    let measured: BTreeSet<&str> = board["results"]
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    let allowed: BTreeSet<&str> = MAY_BE_UNMEASURED.iter().copied().collect();
    for case in corpus["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        assert!(
            measured.contains(name) || allowed.contains(name),
            "{name} is not on the committed scoreboard (run scoreboard.py --commit)"
        );
    }
    for name in board["not_measured"].as_object().unwrap().keys() {
        assert!(
            allowed.contains(name.as_str()),
            "{name} was left unmeasured but is not pinned as allowed"
        );
    }
}
