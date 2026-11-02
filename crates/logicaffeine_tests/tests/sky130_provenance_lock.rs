//! sky130_provenance_lock — a recorded Sky130 signoff claim must still be RE-DERIVABLE from an
//! OpenLane run directory on disk.
//!
//! Every other signoff lock (`sky130_signoff_lock`, `sky130_deep_signoff_lock`,
//! `caravel_signoff_lock`, `golden_precheck_lock`) reads the checked-in JSON and asserts on the
//! numbers it finds there. None of them checks that those numbers still correspond to a real run.
//! That gap is not hypothetical: a `gpu_simt` entry recording setup +13.51 ns / antenna 0 survived
//! in `benchmarks/silicon/sky130_signoff.json` after the run that produced it was overwritten by a
//! re-sign that FAILS setup at −4.36 ns with 4 antenna violations. Every lock stayed green, because
//! every lock was reading the ledger the dead run had written.
//!
//! This lock closes that loop. For each block in each signoff ledger it re-runs
//! `scripts/record-sky130-signoff.py --verify` — the recorder's OWN extractors, not a
//! reimplementation — against the run directories under the ledger's run root (`tools/OpenLane/designs/`
//! unless its registry entry names another, as the bench's stock-OpenLane controls do) and asserts the
//! recorded entry reproduces field-for-field.
//!
//! GATING: the OpenLane run dirs are build products in the machine-local EDA tree under `/tools/`, which a
//! clone does not carry. Each test asks `common::require_tool("eda-tools")` first: a clone records a counted
//! skip, and a gate that requires the tree fails loud without it. Where the tree is, a block with no run
//! directory is SKIPPED and a block WITH a run directory that disagrees is a hard failure. So this bites on the
//! machine that did the tape-out — exactly where the evidence lives and where a stale claim can be caught.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;
#[path = "common/sky130_ledgers.rs"]
mod sky130_ledgers;

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every registered block ledger, of every class: signoff and comparative blocks alike must re-derive from
/// their runs.
///
/// A hand-maintained array here made "add a ledger and forget to register it" a silent hole in exactly the
/// lock whose job is to catch silent holes. The registry closes it: `sky130_ledger_registry_lock` holds
/// `benchmarks/silicon/sky130_ledgers.json` equal to the block ledgers on disk, so every ledger carrying a
/// `blocks` object is one this lock re-derives.
fn ledgers() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> =
        sky130_ledgers::registry(&root()).into_iter().map(|l| (l.path, l.run_root)).collect();
    out.sort();
    assert!(
        !out.is_empty(),
        "no block ledgers registered in benchmarks/silicon/sky130_ledgers.json — this lock would otherwise \
         vacuously pass"
    );
    out
}

/// Run the recorder in verify mode over one ledger, against the run directories under its run root, and return
/// its JSON report.
fn verify(ledger: &str, run_root: &str) -> serde_json::Value {
    let r = root();
    let out = Command::new("python3")
        .arg(r.join("scripts/record-sky130-signoff.py"))
        .arg("--verify")
        .arg(r.join(ledger))
        .arg(r.join(run_root))
        .current_dir(&r)
        .output()
        .unwrap_or_else(|e| panic!("cannot run record-sky130-signoff.py --verify {ledger}: {e}"));

    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "record-sky130-signoff.py --verify {ledger} did not emit a JSON report on stdout ({e})\n\
             --- stdout ---\n{stdout}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

/// THE lock: no recorded block may disagree with the run directory that backs it.
///
/// A mismatch means the ledger is describing a run that no longer exists on disk — the claim is
/// unfalsifiable until it is re-signed. Fail loudly with the exact per-field delta so the fix is
/// obvious: either re-run the flow, or correct the record.
#[test]
fn every_recorded_block_reproduces_from_its_run_directory() {
    if !common::require_tool("eda-tools") {
        return;
    }
    let mut failures = Vec::new();
    let mut checked = 0usize;
    let mut skipped = 0usize;

    for (ledger, run_root) in ledgers().iter().map(|(l, r)| (l.as_str(), r.as_str())) {
        let report = verify(ledger, run_root);
        let blocks = report["blocks"]
            .as_object()
            .unwrap_or_else(|| panic!("{ledger}: verify report has no `blocks` object"));

        for (name, b) in blocks {
            match b["status"].as_str().unwrap_or("") {
                "reproduces" => checked += 1,
                "no_run_dir" => skipped += 1,
                "mismatch" => {
                    let deltas = b["mismatches"]
                        .as_object()
                        .map(|m| {
                            m.iter()
                                .map(|(k, v)| {
                                    format!("      {k}: ledger={} disk={}", v["ledger"], v["disk"])
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    failures.push(format!(
                        "  {ledger} / {name}\n    run_dir: {}\n{deltas}",
                        b["run_dir"].as_str().unwrap_or("?")
                    ));
                }
                other => panic!("{ledger}/{name}: unknown verify status {other:?}"),
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} recorded Sky130 block(s) do NOT reproduce from their run directory — the ledger is \
         describing runs that are gone or were overwritten. Re-sign the block (scripts/openlane-run.sh) \
         or correct the record; NEVER edit the ledger to match a claim you cannot reproduce:\n{}",
        failures.len(),
        failures.join("\n")
    );

    // A verify pass that silently checked nothing is a false green. On a machine with run dirs this
    // must have actually re-derived something.
    assert!(
        checked > 0 || skipped > 0,
        "the verifier examined no blocks at all — the ledgers or the run-root glob are wrong"
    );
}

/// The verifier must be able to FIND the run directories on a box that has them. If every single
/// block is skipped on a machine whose EDA tree holds the hardened designs, the gating logic has
/// silently disabled the whole lock — a green that proves nothing.
#[test]
fn the_verifier_finds_run_directories_when_they_exist() {
    if !common::require_tool("eda-tools") {
        return;
    }
    let report = verify("benchmarks/silicon/sky130_signoff.json", sky130_ledgers::DEFAULT_RUN_ROOT);
    let blocks = report["blocks"]
        .as_object()
        .expect("verify report has no `blocks`");
    let found = blocks
        .values()
        .filter(|b| b["status"] != "no_run_dir")
        .count();
    assert!(
        found > 0,
        "tools/OpenLane/designs/ is populated but the verifier located a run directory for ZERO \
         blocks — the run-root glob is broken and this lock is silently inert"
    );
}

/// A run that flowed in an image its tree's checkout does not name (the comparative bench's OpenLane image, which
/// rebuilds the same OpenROAD commit and prints the same version line) cannot say so itself, so its launcher writes
/// `<run>/openlane_image` beside it, and the recorder copies that into the entry as `openlane_image`. A run whose
/// launcher wrote none records no such field: every entry recorded before the marker existed re-derives unchanged.
/// And the verifier holds an entry to the image it names: an entry naming one whose run no longer carries it is a
/// mismatch, while an entry naming none is not held to a marker.
///
/// Checked on one real signoff run and a mirror of it (every file of the run linked, plus the marker, under the run's
/// own name: the recorder maps the container's `/runs/<run>/` paths by it), through the recorder's own `extract` and
/// `compare`.
#[test]
fn a_run_records_the_image_its_launcher_names_and_only_then() {
    if !common::require_tool("eda-tools") {
        return;
    }
    let report = verify("benchmarks/silicon/sky130_signoff.json", sky130_ledgers::DEFAULT_RUN_ROOT);
    let (block, run_dir) = report["blocks"]
        .as_object()
        .expect("verify report has no `blocks`")
        .iter()
        .find(|(_, b)| b["status"] == "reproduces")
        .map(|(n, b)| (n.clone(), b["run_dir"].as_str().expect("a reproducing block names its run").to_string()))
        .expect("no block of sky130_signoff.json reproduces from its run");
    let run = root().join(&run_dir);
    assert!(!run.join("openlane_image").exists(), "{run_dir} already carries a marker; the lock needs an unmarked run");
    let parent = std::env::temp_dir().join(format!("logos-openlane-image-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent);
    let mirror = parent.join(run.file_name().expect("a run dir has a name"));
    std::fs::create_dir_all(&mirror).expect("mirror dir");
    for entry in std::fs::read_dir(&run).expect("run dir") {
        let entry = entry.expect("run entry");
        std::os::unix::fs::symlink(entry.path(), mirror.join(entry.file_name())).expect("link a run entry");
    }
    let image = "logos-openlane:00000000-openroad-000000000000";
    std::fs::write(mirror.join("openlane_image"), format!("{image}\n")).expect("marker");
    let script = r#"
import importlib.util, json, sys
spec = importlib.util.spec_from_file_location("recorder", sys.argv[1])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
plain, marked = m.extract(sys.argv[2], sys.argv[4]), m.extract(sys.argv[3], sys.argv[4])
print(json.dumps({
    "plain": plain,
    "marked": marked,
    "gone": m.compare(dict(plain, openlane_image=sys.argv[5]), plain),
    "unnamed": m.compare(plain, marked),
}))
"#;
    let out = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(root().join("scripts/record-sky130-signoff.py"))
        .arg(&run)
        .arg(&mirror)
        .arg(&block)
        .arg(image)
        .current_dir(root())
        .output()
        .expect("run the recorder's extractors");
    let _ = std::fs::remove_dir_all(&parent);
    let r: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!("the recorder's extractors emitted no JSON ({e}): {}", String::from_utf8_lossy(&out.stderr))
    });
    let plain = r["plain"].as_object().expect("plain extract");
    let marked = r["marked"].as_object().expect("marked extract");
    assert!(!plain.contains_key("openlane_image"), "{block}: a run whose launcher named no image records one");
    assert_eq!(marked.get("openlane_image").and_then(|v| v.as_str()), Some(image), "{block}: the marker is not recorded");
    let mut rest = marked.clone();
    rest.remove("openlane_image");
    assert_eq!(&rest, plain, "{block}: the marker changed a field other than openlane_image");
    assert!(
        r["gone"].as_object().is_some_and(|d| d.contains_key("openlane_image")),
        "an entry naming an image its run does not carry verifies: {}",
        r["gone"]
    );
    assert!(
        r["unnamed"].as_object().is_some_and(|d| d.is_empty()),
        "an entry naming no image is held to its run's marker: {}",
        r["unnamed"]
    );
}
