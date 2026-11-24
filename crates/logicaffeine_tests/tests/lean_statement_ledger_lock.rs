//! The statement ledger of the pinned corpus's 200 open problems, regression-locked.
//!
//! `fixtures/lean/ledgers/alejandro200.statement.json` records, for every problem of the pinned
//! formalization-lab corpus, what `largo lean import --project … --statement` decided
//! (`scripts/lean/statement-ledger.py`): a verified statement package, or a typed refusal. This lock
//! holds the record honest — every problem exactly once, every outcome one of the contract's, a
//! verified problem verified at every stage with its package's digest and `proof_status:
//! not_claimed`, every refusal a typed, located diagnostic of the reader's own vocabulary, the refusal
//! table the entries' own — and holds the verified count to a floor that only rises.

use logicaffeine_kernel::interface::lean::DiagnosticCode;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Verified statement packages the recorded ledger holds. Raise it as the ledger improves; never
/// lower it.
const VERIFIED_FLOOR: usize = 19;

const OUTCOMES: [&str; 6] = [
    "verified",
    "invalid_source",
    "unsupported_feature",
    "disallowed_assumptions",
    "resource_limit",
    "infrastructure_failure",
];

const STAGES: [&str; 7] = [
    "lean_accepted",
    "translated",
    "primary_check",
    "independent_recheck",
    "assumption_policy",
    "emission_reparsed",
    "fresh_replay",
];

fn ledger() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lean/ledgers/alejandro200.statement.json");
    serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .unwrap()
}

fn entries(ledger: &Value) -> &Vec<Value> {
    ledger["entries"].as_array().unwrap()
}

/// Every recorded problem is a corpus problem, once, in corpus order; a ledger that says it is
/// complete records all 200. A ledger still being run (`complete: false`) says so, and resumes with
/// `statement-ledger.py --resume`.
#[test]
fn the_ledger_records_corpus_problems_exactly_once() {
    let ledger = ledger();
    assert_eq!(ledger["schema"], "logos-lean-statement-ledger/1");
    assert_eq!(ledger["corpus"]["problems"], 200);
    let ids: Vec<&str> = entries(&ledger)
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    let corpus: Vec<String> = (1..=200).map(|i| format!("OP{i:03}")).collect();
    let mut at = 0;
    for id in &ids {
        let found = corpus[at..]
            .iter()
            .position(|c| c == id)
            .unwrap_or_else(|| {
                panic!(
                    "{id} is not a corpus problem after {}",
                    corpus[at.saturating_sub(1)]
                )
            });
        at += found + 1;
    }
    let complete = ledger["complete"]
        .as_bool()
        .expect("the ledger says whether it is complete");
    assert_eq!(complete, ids.len() == corpus.len());
}

#[test]
fn every_outcome_is_the_contracts_and_every_refusal_is_typed() {
    let ledger = ledger();
    for entry in entries(&ledger) {
        let id = entry["id"].as_str().unwrap();
        let outcome = entry["outcome"].as_str().unwrap();
        assert!(OUTCOMES.contains(&outcome), "{id}: {outcome}");
        assert_eq!(entry["schema"], "logos-lean-result/2", "{id}");
        assert_eq!(entry["proof_status"], "not_claimed", "{id}");
        let stages = entry["stages"].as_object().unwrap();
        assert_eq!(stages.len(), STAGES.len(), "{id}");
        if outcome == "verified" {
            for stage in STAGES {
                assert_eq!(stages[stage], "passed", "{id} {stage}");
            }
            let digest = entry["package_manifest_sha256"].as_str().unwrap();
            assert_eq!(digest.len(), 64, "{id}");
            assert!(digest.bytes().all(|b| b.is_ascii_hexdigit()), "{id}");
            let declaration = entry["declaration"].as_str().unwrap();
            assert_eq!(
                entry["statement"]["artifact"],
                format!("{declaration}.logos_statement"),
                "{id}"
            );
            let origin = match entry["declaration_kind"].as_str().unwrap() {
                "def" => "definition_value",
                _ => "theorem_type",
            };
            assert_eq!(entry["statement"]["origin"], origin, "{id}");
            assert!(
                entry["assumptions"].as_array().unwrap().iter().all(|a| [
                    "propext",
                    "Quot.sound",
                    "Classical.choice"
                ]
                .contains(&a.as_str().unwrap())),
                "{id}: a verified statement rests on Lean's three axioms at most"
            );
            continue;
        }
        let first = &entry["first"];
        let code = first["code"].as_str().unwrap();
        let parsed = DiagnosticCode::from_wire(code)
            .unwrap_or_else(|| panic!("{id}: {code} is no diagnostic code"));
        assert_ne!(parsed, DiagnosticCode::Unclassified, "{id}");
        assert!(
            STAGES.contains(&first["stage"].as_str().unwrap()),
            "{id}: {first}"
        );
        assert!(!first["message"].as_str().unwrap().is_empty(), "{id}");
        assert!(
            stages.values().any(|s| s == "failed" || s == "unsupported"),
            "{id}: a refusal names the stage that did not pass"
        );
        assert!(entry["package_manifest_sha256"].is_null(), "{id}");
    }
}

#[test]
fn the_counts_and_the_refusal_table_are_the_entries() {
    let ledger = ledger();
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut refusals: BTreeMap<String, u64> = BTreeMap::new();
    for entry in entries(&ledger) {
        *counts
            .entry(entry["outcome"].as_str().unwrap().to_string())
            .or_default() += 1;
        if entry["outcome"] != "verified" {
            let key = format!(
                "{}:{}",
                entry["first"]["stage"].as_str().unwrap(),
                entry["first"]["code"].as_str().unwrap()
            );
            *refusals.entry(key).or_default() += 1;
        }
    }
    let recorded: BTreeMap<String, u64> = serde_json::from_value(ledger["counts"].clone()).unwrap();
    assert_eq!(recorded, counts);
    let recorded: BTreeMap<String, u64> =
        serde_json::from_value(ledger["refusals"].clone()).unwrap();
    assert_eq!(recorded, refusals);
}

#[test]
fn the_verified_count_only_rises() {
    let ledger = ledger();
    let verified = entries(&ledger)
        .iter()
        .filter(|e| e["outcome"] == "verified")
        .count();
    assert!(
        verified >= VERIFIED_FLOOR,
        "{verified} verified statement packages, below the floor of {VERIFIED_FLOOR}"
    );
    let distinct: BTreeSet<&str> = entries(&ledger)
        .iter()
        .filter_map(|e| e["package_manifest_sha256"].as_str())
        .collect();
    assert_eq!(distinct.len(), verified, "every package is its own");
}
