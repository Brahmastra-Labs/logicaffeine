//! Every Lean fixture's provenance resolves in the checkout, byte for byte.
//!
//! An export fixture carries a provenance record (`*.provenance.json`, `PROVENANCE.json`) that names the Lean
//! source it was exported from and, for a `source`, that source's SHA-256. The conformance corpus's sources ship in
//! the tree (`fixtures/lean_conformance/cases/`), as do the witness modules beside their exports, so a recorded
//! source that is a repository path — its first component is an entry of the repository root, or the ignored
//! private workspace — must be here and hash to its digest: a source that moved without its records, or changed
//! after its export, is RED. A name without a directory is resolved beside its record. A URL, a path into a local
//! reference checkout (`reference-kernels/`, `external/`) and a path inside a downloaded archive the record names
//! (`good/tutorial/…` of the Arena tests) cannot be checked from a checkout and are skipped.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test lean_fixture_provenance_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/public_manifest.rs"]
mod public_manifest;

use public_manifest::{git_ignored, repo_root, LOCAL_LOCATIONS};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The fields of a provenance record that name a file it was made from.
const SOURCE_FIELDS: [&str; 4] = [
    "source",
    "source_path",
    "original_case_source",
    "ground_truth",
];

/// The records in this checkout that name a repository source and its digest, all of them checked: raise it
/// when fixtures are added, never lower it.
const DIGESTED_SOURCES_FLOOR: usize = 379;

fn sha256_hex(bytes: &[u8]) -> String {
    logicaffeine_compile::crypto_accel::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn records(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            records(&p, out);
        } else if p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("provenance.json") || n == "PROVENANCE.json")
        {
            out.push(p);
        }
    }
}

/// `(field, recorded path, recorded digest)` for every source a record names, at any depth.
fn sources(v: &Value, out: &mut Vec<(String, String, Option<String>)>) {
    match v {
        Value::Object(map) => {
            for (k, x) in map {
                if let (true, Some(path)) = (SOURCE_FIELDS.contains(&k.as_str()), x.as_str()) {
                    let digest = if k == "source" {
                        map.get("source_sha256")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    } else {
                        None
                    };
                    out.push((k.clone(), path.to_string(), digest));
                } else {
                    sources(x, out);
                }
            }
        }
        Value::Array(xs) => xs.iter().for_each(|x| sources(x, out)),
        _ => {}
    }
}

/// Every problem with the records under `fixtures`, resolved against `root`, and how many digests were checked.
/// `in_repo` says whether a path's first component is one of the repository's own root entries.
fn problems(fixtures: &Path, root: &Path, in_repo: &dyn Fn(&str) -> bool) -> (Vec<String>, usize) {
    let mut files = Vec::new();
    records(fixtures, &mut files);
    let mut bad = Vec::new();
    let mut digested = 0;
    for file in files {
        let shown = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .display()
            .to_string();
        let value: Value = match std::fs::read_to_string(&file).map(|t| serde_json::from_str(&t)) {
            Ok(Ok(v)) => v,
            _ => {
                bad.push(format!("{shown}: not a JSON record"));
                continue;
            }
        };
        let mut named = Vec::new();
        sources(&value, &mut named);
        for (field, path, digest) in named {
            if path.contains("://") {
                continue;
            }
            let first = path.split('/').next().unwrap_or_default();
            if LOCAL_LOCATIONS.contains(&first) || path.contains('/') && !in_repo(first) {
                continue;
            }
            let resolved = if path.contains('/') {
                root.join(&path)
            } else {
                file.parent().unwrap().join(&path)
            };
            match std::fs::read(&resolved) {
                Err(_) => bad.push(format!("{shown}: {field} `{path}` is not in the checkout")),
                Ok(bytes) => {
                    if let Some(digest) = digest {
                        digested += 1;
                        let actual = sha256_hex(&bytes);
                        if actual != digest {
                            bad.push(format!(
                                "{shown}: `{path}` hashes to {actual}, the record says {digest}"
                            ));
                        }
                    }
                }
            }
        }
    }
    (bad, digested)
}

#[test]
fn every_recorded_source_is_in_the_checkout_and_unchanged() {
    let root = repo_root();
    let in_repo = |first: &str| root.join(first).exists() || git_ignored(&[first.to_string()])[0];
    let (bad, digested) = problems(
        &root.join("crates/logicaffeine_tests/tests/fixtures"),
        &root,
        &in_repo,
    );
    assert!(
        bad.is_empty(),
        "a Lean fixture's provenance no longer matches the checkout. Move a source with its records; a source \
         changed after its export is re-exported and re-recorded, never re-pinned by hand:\n  {}",
        bad.join("\n  ")
    );
    assert!(
        digested >= DIGESTED_SOURCES_FLOOR,
        "only {digested} recorded sources were checked against their digests, fewer than the {DIGESTED_SOURCES_FLOOR} \
         this checkout holds: records stopped naming their sources, or the walk stopped finding them"
    );
}

#[test]
fn the_checker_catches_a_moved_source_and_a_changed_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let fixtures = root.join("crates/t/tests/fixtures");
    std::fs::create_dir_all(fixtures.join("corpus/cases")).unwrap();
    std::fs::write(
        fixtures.join("corpus/cases/A.lean"),
        b"theorem a : True := trivial\n",
    )
    .unwrap();
    std::fs::write(fixtures.join("W.lean"), b"def w := 1\n").unwrap();
    let good = sha256_hex(b"theorem a : True := trivial\n");
    let write = |name: &str, v: Value| std::fs::write(fixtures.join(name), v.to_string()).unwrap();
    write(
        "A.provenance.json",
        serde_json::json!({"source": "crates/t/tests/fixtures/corpus/cases/A.lean", "source_sha256": good}),
    );
    write(
        "W.provenance.json",
        serde_json::json!({"source": "W.lean", "source_sha256": sha256_hex(b"def w := 1\n")}),
    );
    write(
        "U.provenance.json",
        serde_json::json!({"source": "https://example.org/x.tar.gz"}),
    );
    write(
        "R.provenance.json",
        serde_json::json!({"source_path": "reference-kernels/x/_build/y.ndjson"}),
    );
    write(
        "PROVENANCE.json",
        serde_json::json!({"download": "https://example.org/tests.tar.gz",
        "fixtures": [{"source": "good/tutorial/001_x.ndjson", "sha256": good}]}),
    );
    let in_repo = |first: &str| first == "crates" || first == "work";
    assert_eq!(
        problems(&fixtures, root, &in_repo),
        (vec![], 2),
        "an honest tree has no problem and checks both digests"
    );

    write(
        "M.provenance.json",
        serde_json::json!({"source": "work/corpus/cases/A.lean", "source_sha256": good}),
    );
    write(
        "N.provenance.json",
        serde_json::json!({"nested": {"ground_truth": "crates/t/tests/fixtures/corpus/truth/A.json"}}),
    );
    std::fs::write(fixtures.join("W.lean"), b"def w := 2\n").unwrap();
    let (bad, _) = problems(&fixtures, root, &in_repo);
    assert_eq!(bad.len(), 3, "{bad:#?}");
    assert!(
        bad.iter()
            .any(|b| b.contains("M.provenance.json") && b.contains("not in the checkout")),
        "{bad:#?}"
    );
    assert!(
        bad.iter()
            .any(|b| b.contains("N.provenance.json") && b.contains("ground_truth")),
        "{bad:#?}"
    );
    assert!(
        bad.iter()
            .any(|b| b.contains("W.provenance.json") && b.contains("hashes to")),
        "{bad:#?}"
    );
}
