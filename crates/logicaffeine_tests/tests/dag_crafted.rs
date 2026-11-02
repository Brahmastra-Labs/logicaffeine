//! Hand-built exports that each isolate one kernel rule (`scripts/lean/arena-crown/craft-cases.py`).
//!
//! The mutation matrix found rules the Arena corpus never exercises alone. Each `bad` case here
//! differs from an admissible export only in the rule it targets — its claimed recursors and flags
//! are exactly what the kernel derives — so it must be rejected, and rejected BY THAT RULE (its
//! message is checked). Each `good` case is a control proving the surrounding construction is
//! admissible on its own.

use logicaffeine_kernel::dag::{check_export, Verdict};
use std::io::BufReader;
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crafted")
}

#[test]
fn every_crafted_case_is_decided_by_its_rule() {
    let text = std::fs::read_to_string(dir().join("manifest.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&text).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert!(cases.len() >= 7);
    let mut failures = Vec::new();
    for c in cases {
        let (name, outcome) = (c["name"].as_str().unwrap(), c["outcome"].as_str().unwrap());
        let path = dir().join(c["path"].as_str().unwrap());
        let verdict = std::thread::Builder::new()
            .stack_size(1 << 30)
            .spawn(move || check_export(BufReader::new(std::fs::File::open(path).unwrap())).0)
            .unwrap()
            .join()
            .unwrap();
        match (outcome, verdict) {
            ("good", Verdict::Accept) => {}
            ("bad", Verdict::Reject(e)) => {
                let rule = c["rejected_by"]
                    .as_str()
                    .expect("a bad case names the rule that rejects it");
                if !e.to_string().contains(rule) {
                    failures.push(format!("{name}: rejected, but not by '{rule}': {e}"));
                }
            }
            (o, v) => failures.push(format!(
                "{name}: expected {o}, got {}",
                match v {
                    Verdict::Accept => "ACCEPT".to_string(),
                    Verdict::Reject(e) => format!("reject ({e})"),
                    Verdict::Decline(e) => format!("decline ({e})"),
                }
            )),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
