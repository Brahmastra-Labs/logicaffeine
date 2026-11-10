//! Hand-built exports that each isolate one kernel rule (`scripts/lean/arena-crown/craft-cases.py`).
//!
//! The mutation matrix found rules the Arena corpus never exercises alone. Each `bad` case here
//! differs from an admissible export only in the rule it targets — its claimed recursors and flags
//! are exactly what the kernel derives — so it must be rejected, and rejected BY THAT RULE (its
//! message is checked). Each `good` case is a control proving the surrounding construction is
//! admissible on its own. Every case is decided by every engine: substitution, evaluation, and both
//! at once. The fixtures are exactly what the generator writes.

use logicaffeine_kernel::dag::tc::Engine;
use logicaffeine_kernel::dag::{check_export_with, Verdict};
use std::collections::BTreeMap;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::Command;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crafted")
}

#[test]
fn every_crafted_case_is_decided_by_its_rule() {
    let text = std::fs::read_to_string(dir().join("manifest.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&text).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert!(cases.len() >= 19);
    let mut failures = Vec::new();
    for c in cases {
        let (name, outcome) = (c["name"].as_str().unwrap(), c["outcome"].as_str().unwrap());
        for engine in [Engine::Classic, Engine::Sem, Engine::Dual] {
            let path = dir().join(c["path"].as_str().unwrap());
            let verdict = std::thread::Builder::new()
                .stack_size(1 << 30)
                .spawn(move || {
                    check_export_with(BufReader::new(std::fs::File::open(path).unwrap()), |k| {
                        k.engine = engine
                    })
                    .0
                })
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
                        failures.push(format!(
                            "{name} ({engine:?}): rejected, but not by '{rule}': {e}"
                        ));
                    }
                }
                (o, v) => failures.push(format!(
                    "{name} ({engine:?}): expected {o}, got {}",
                    match v {
                        Verdict::Accept => "ACCEPT".to_string(),
                        Verdict::Reject(e) => format!("reject ({e})"),
                        Verdict::Decline(e) => format!("decline ({e})"),
                    }
                )),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every file under `dir`, by its path below it, with its bytes.
fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(d) = pending.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                out.insert(path.strip_prefix(dir).unwrap().to_path_buf(), bytes);
            }
        }
    }
    out
}

#[test]
fn the_crafted_fixtures_are_what_the_generator_writes() {
    let generator = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/lean/arena-crown/craft-cases.py");
    let out = tempfile::tempdir().unwrap();
    let status = Command::new("python3")
        .arg(&generator)
        .arg("--out")
        .arg(out.path())
        .status()
        .expect("python3 runs craft-cases.py");
    assert!(status.success(), "craft-cases.py failed: {status}");
    let (written, held) = (tree(out.path()), tree(&dir()));
    let names = |t: &BTreeMap<PathBuf, Vec<u8>>| t.keys().cloned().collect::<Vec<_>>();
    assert_eq!(
        names(&held),
        names(&written),
        "fixtures/crafted holds other files than craft-cases.py writes"
    );
    for (path, bytes) in &written {
        assert!(
            held[path] == *bytes,
            "fixtures/crafted/{} is not what craft-cases.py writes: run it again",
            path.display()
        );
    }
}
