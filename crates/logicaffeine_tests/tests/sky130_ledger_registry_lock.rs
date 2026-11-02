//! sky130_ledger_registry_lock — every Sky130 block ledger is named, with its class, in one registry, and
//! nothing finds a ledger by its file name.
//!
//! `scripts/record-sky130-signoff.py` writes block ledgers: JSON documents whose `blocks` map holds one
//! recorded OpenLane run per block. They come in two classes, listed in `benchmarks/silicon/sky130_ledgers.json`:
//! `signoff` (LOGOS blocks hardened as deliverables — registered in `codegen_sva::signoff_blocks` at the period
//! their slow corner meets, every one held by the signoff locks) and `comparative` (the comparative-synthesis
//! bench's candidates, LOGOS and third-party RTL alike, at a period the bench sets — held to provenance,
//! uniqueness, electrical completeness and power binding, never to signoff registration). The lock holds:
//!
//! A. the registry and the ledgers on disk are the same set: every block ledger on disk is registered, and every
//!    registered path is a block ledger on disk — written, with no runs yet, when it is registered, so the recorder
//!    (which writes only a registered ledger) and every reader of the registry find it before its first run — each of
//!    a known class, public (a clone carries every ledger a lock reads through the registry), its runs under an
//!    OpenLane tree's `designs/` in the machine-local `tools/` (`tools/OpenLane/designs` unless its entry names a
//!    `run_root`);
//! B. no lock and no flow script discovers ledgers by matching `signoff` in a file name — the idiom that would
//!    let a comparative ledger be read as a signoff one, or a new ledger be seen by one check and missed by
//!    another;
//! C. the recorder refuses to write a ledger the registry does not name, so none comes into existence
//!    unregistered; a staging copy outside the repository named as one registered ledger (`MACRO_LEDGER_OUT` and its
//!    kin, where a batch of re-signs files its records before it lands them) records as that ledger;
//! D. no comparative ledger's runs are under `tools/OpenLane/designs`, the tree the signoff blocks and every session
//!    flow in, each of whose designs `rtl_provenance_lock` traces to RTL the repository tracks: the bench flows in
//!    trees of its own under `tools/qor/`;
//! E. the recorder writes a ledger whole and under one lock, so flows recording into one ledger at once (the bench
//!    runs several side by side) never lose a record, and a reader never sees half a file.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/sky130_ledgers.rs"]
mod sky130_ledgers;

#[path = "common/public_manifest.rs"]
mod public_manifest;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The runs a block ledger at `path` holds: a JSON document whose `blocks` map has one row per run, each carrying the
/// flow's status, and none before its first run. `None` when `path` is no such document.
fn ledger_runs(path: &Path) -> Option<usize> {
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let blocks = doc["blocks"].as_object()?;
    blocks.values().all(|row| row.get("flow_status").is_some()).then_some(blocks.len())
}

/// A JSON document the signoff recorder wrote a run into: found on disk by its content, never by its name.
fn is_block_ledger(path: &Path) -> bool {
    std::fs::read_to_string(path).map_or(false, |t| t.contains("\"flow_status\"")) && ledger_runs(path).unwrap_or(0) > 0
}

#[test]
fn the_registry_names_every_block_ledger_on_disk_and_only_those() {
    let root = root();
    let on_disk: BTreeSet<String> = std::fs::read_dir(root.join("benchmarks/silicon"))
        .expect("benchmarks/silicon")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map_or(false, |x| x == "json") && is_block_ledger(p))
        .map(|p| format!("benchmarks/silicon/{}", p.file_name().unwrap().to_string_lossy()))
        .collect();
    let registry = sky130_ledgers::registry(&root);
    let registered: BTreeSet<String> = registry.iter().map(|l| l.path.clone()).collect();
    assert_eq!(registered.len(), registry.len(), "a ledger is registered once");
    let unregistered: Vec<&String> = on_disk.difference(&registered).collect();
    assert!(unregistered.is_empty(), "every block ledger on disk is registered: {unregistered:?}");
    let ignored = public_manifest::git_ignored(&registry.iter().map(|l| l.path.clone()).collect::<Vec<_>>());
    for (l, ignored) in registry.iter().zip(ignored) {
        assert!(
            ledger_runs(&root.join(&l.path)).is_some(),
            "{}: registered, and no block ledger on disk — a ledger is written, with an empty `blocks` map, when it is \
             registered",
            l.path
        );
        assert!(!ignored, "{}: registered, and gitignored — a clone would not carry it", l.path);
        assert!(sky130_ledgers::CLASSES.contains(&l.class.as_str()), "{}: class {}", l.path, l.class);
        assert!(
            l.run_root.starts_with("tools/") && l.run_root.ends_with("/designs"),
            "{}: its runs are under {} — an OpenLane tree's designs/ in the machine-local tools/",
            l.path,
            l.run_root
        );
    }
    assert!(registry.iter().any(|l| l.class == "signoff"), "the signoff ledgers are registered");
    for class in sky130_ledgers::CLASSES {
        assert_eq!(
            sky130_ledgers::ledgers(&root, Some(class)).len(),
            registry.iter().filter(|l| l.class == *class).count(),
            "the helper reads the registry's {class} ledgers"
        );
    }
}

#[test]
fn no_lock_or_flow_script_finds_a_ledger_by_its_file_name() {
    let root = root();
    let mut sources: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.join("crates/logicaffeine_tests/tests"), root.join("scripts")];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().map_or(false, |x| x == "rs" || x == "py" || x == "sh") {
                sources.push(p);
            }
        }
    }
    let this = Path::new(file!()).file_name().unwrap();
    let mut finds = Vec::new();
    for p in sources.iter().filter(|p| p.file_name() != Some(this)) {
        let text = std::fs::read_to_string(p).unwrap_or_default();
        for (n, line) in text.lines().enumerate() {
            let by_name = line.contains("*signoff*")
                || (line.contains("contains(\"signoff\")") && line.contains(".json"))
                || (line.contains("\"signoff\" in") && line.contains(".json"));
            if by_name {
                finds.push(format!("{}:{}: {}", p.strip_prefix(&root).unwrap().display(), n + 1, line.trim()));
            }
        }
    }
    assert!(
        finds.is_empty(),
        "these find ledgers by matching a file name — read benchmarks/silicon/sky130_ledgers.json instead:\n{}",
        finds.join("\n")
    );
}

#[test]
fn the_recorder_refuses_a_ledger_the_registry_does_not_name() {
    let root = root();
    let tmp = tempfile::tempdir().expect("a scratch directory");
    let out = tmp.path().join("unregistered_signoff.json");
    let run = std::process::Command::new("python3")
        .arg(root.join("scripts/record-sky130-signoff.py"))
        .arg(tmp.path())
        .arg("a_block")
        .arg(&out)
        .output()
        .expect("python3 runs the recorder");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(!run.status.success(), "the recorder wrote an unregistered ledger");
    assert!(stderr.contains("is not a registered block ledger") && stderr.contains("sky130_ledgers.json"), "{stderr}");
    assert!(!out.exists(), "nothing is written");
}

/// The ledger each path records as, by the recorder's own reading: a registered path is itself; a copy outside the
/// repository named as exactly one registered ledger is a staging copy of it; any other path, inside the tree or out,
/// is none.
#[test]
fn a_staging_copy_outside_the_tree_records_as_the_ledger_it_is_named_for() {
    let root = root();
    let tmp = tempfile::tempdir().expect("a scratch directory");
    let script = root.join("scripts/record-sky130-signoff.py");
    let cases = [
        (root.join("benchmarks/silicon/caravel_macro_signoff.json"), Some("benchmarks/silicon/caravel_macro_signoff.json")),
        (tmp.path().join("caravel_macro_signoff.json"), Some("benchmarks/silicon/caravel_macro_signoff.json")),
        (tmp.path().join("stage/sky130_signoff.json"), Some("benchmarks/silicon/sky130_signoff.json")),
        (tmp.path().join("unregistered_signoff.json"), None),
        (tmp.path().join("sky130_ledgers.json"), None),
        (root.join("benchmarks/caravel_macro_signoff.json"), None),
        (root.join("benchmarks/silicon/stage/caravel_macro_signoff.json"), None),
    ];
    for (out, want) in cases {
        let prog = format!(
            "import importlib.util,json,sys\n\
             s=importlib.util.spec_from_file_location('r',{script:?}); r=importlib.util.module_from_spec(s); s.loader.exec_module(r)\n\
             print(json.dumps(r.registered_as(sys.argv[1])))",
            script = script.display().to_string()
        );
        let run = std::process::Command::new("python3").arg("-c").arg(&prog).arg(&out).output().expect("python3");
        assert!(run.status.success(), "{}: {}", out.display(), String::from_utf8_lossy(&run.stderr));
        let got: Option<String> = serde_json::from_slice(&run.stdout).unwrap();
        assert_eq!(got.as_deref(), want, "{}", out.display());
    }
    let registry = tmp.path().join("ambiguous_ledgers.json");
    std::fs::write(&registry, r#"{"ledgers": [{"path": "a/twin.json"}, {"path": "b/twin.json"}]}"#).unwrap();
    for (out, want) in [(tmp.path().join("twin.json"), None), (root.join("a/twin.json"), Some("a/twin.json"))] {
        let prog = format!(
            "import importlib.util,json,sys\n\
             s=importlib.util.spec_from_file_location('r',{script:?}); r=importlib.util.module_from_spec(s); s.loader.exec_module(r)\n\
             r.REGISTRY=sys.argv[2]; print(json.dumps(r.registered_as(sys.argv[1])))",
            script = script.display().to_string()
        );
        let run = std::process::Command::new("python3").arg("-c").arg(&prog).arg(&out).arg(&registry).output().expect("python3");
        assert!(run.status.success(), "{}: {}", out.display(), String::from_utf8_lossy(&run.stderr));
        let got: Option<String> = serde_json::from_slice(&run.stdout).unwrap();
        assert_eq!(got.as_deref(), want, "a name two registered ledgers share stands for neither: {}", out.display());
    }
}

#[test]
fn no_comparative_ledger_runs_in_the_shared_openlane_tree() {
    let registry = sky130_ledgers::registry(&root());
    let comparative: Vec<_> = registry.iter().filter(|l| l.class == "comparative").collect();
    assert!(!comparative.is_empty(), "the comparative ledgers are registered");
    for l in comparative {
        assert!(
            l.run_root != sky130_ledgers::DEFAULT_RUN_ROOT && l.run_root.starts_with("tools/qor/"),
            "{}: its runs are under {} — a comparative ledger names a run_root in a tree of the bench's own, under tools/qor/",
            l.path,
            l.run_root
        );
    }
}

#[test]
fn the_recorder_writes_a_ledger_whole_under_its_lock() {
    let root = root();
    let tmp = tempfile::tempdir().expect("a scratch directory");
    let probe = r#"
import fcntl, importlib.util, json, os, sys
from multiprocessing import Process
spec = importlib.util.spec_from_file_location("recorder", sys.argv[1])
rec = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rec)
out = os.path.join(sys.argv[2], "ledger.json")
seen = []
replace = os.replace
def probed(src, dst):
    with open(rec.LEDGER_LOCK, "a") as f:
        try:
            fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
            fcntl.flock(f, fcntl.LOCK_UN)
            held = False
        except BlockingIOError:
            held = True
    seen.append((held, dst == out, sorted(json.load(open(src))["blocks"])))
    replace(src, dst)
rec.os.replace = probed
rec.write_record(out, "b1", {"signoff_clean": True})
rec.write_record(out, "b2", {"signoff_clean": False})
rec.os.replace = replace
print(json.dumps(seen))
def many(tag):
    for i in range(40):
        rec.write_record(out, "%s%d" % (tag, i), {"signoff_clean": True})
ps = [Process(target=many, args=(t,)) for t in ("x", "y", "z")]
[p.start() for p in ps]
[p.join() for p in ps]
print(len(json.load(open(out))["blocks"]), sorted(os.listdir(sys.argv[2])))
"#;
    let run = std::process::Command::new("python3")
        .arg("-c")
        .arg(probe)
        .arg(root.join("scripts/record-sky130-signoff.py"))
        .arg(tmp.path())
        .output()
        .expect("python3 runs");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let out = String::from_utf8_lossy(&run.stdout);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[0],
        r#"[[true, true, ["b1"]], [true, true, ["b1", "b2"]]]"#,
        "each record replaces the whole ledger at once, under the lock, keeping every record before it"
    );
    assert_eq!(lines[1], "122 ['ledger.json']", "three writers at once lose no record and leave no scratch file");
}
