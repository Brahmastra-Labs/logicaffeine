//! samba_bringup_clock_lock — the bring-up clock behind the Samba book's exam row E1.11: each Tier A family's twin
//! brought up from its directory read cold to a first token the f64 reference holds, inside the budget, and its time
//! recorded as a measured row of the cell ledger (`benchmarks/silicon/samba_cells.json`, cells `C-SN50-*-BRINGUP`,
//! measure `bringup_s`), which `samba_exam_bringup_lock::the_bring_up_clock_is_recorded_for_every_tier_a_model` holds the
//! ledger to.
//!
//! A bring-up here is the day's: the twin's Hugging Face directory opened, its spec read, its SN50 bundle written by
//! `largo emit rdu` and run, its first token held to the reference within the bf16 datapath (`common/samba_bringup.rs`).
//! Each row names the twin, the spec and the bundle it timed by their digests, and the host, boot and load it ran on;
//! the test fails on a row of another twin, on a row that is not the slowest of its runs, and past the budget.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_bringup_clock_lock`
//! Record the rows: `LOGOS_RECORD_SAMBA_CELLS=1 cargo nextest run -p logicaffeine-tests --cargo-profile test-opt
//! --test samba_bringup_clock_lock`

#![cfg(feature = "codegen")]

mod common;
#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
#[allow(dead_code)]
mod tiny_models;
#[path = "common/samba_bringup.rs"]
#[allow(dead_code)]
mod samba_bringup;
#[path = "common/samba_cells.rs"]
#[allow(dead_code)]
mod samba_cells;

use logicaffeine_compile::model::reference::WeightSource;
use logicaffeine_compile::model::spec::ModelSpec;
use samba_bringup::{checkpoint_brings_up_close, CTX};
use std::time::Instant;

/// Section 6.1's Tier A families, each brought up from its twin: the cell its bring-up clock is recorded under, and the
/// twin's family.
const TIER_A_TWINS: [(&str, &str); 7] = [
    ("C-SN50-LLAMA-BRINGUP", "llama"),
    ("C-SN50-LLAMA4-BRINGUP", "llama4"),
    ("C-SN50-MIXTRAL-BRINGUP", "mixtral"),
    ("C-SN50-QWEN2-BRINGUP", "qwen2"),
    ("C-SN50-GPTOSS-BRINGUP", "gpt-oss"),
    ("C-SN50-DEEPSEEKV3-BRINGUP", "deepseek2"),
    ("C-SN50-MINIMAXM2-BRINGUP", "minimax_m2"),
];
const BRINGUP_MEASURE: &str = "bringup_s";
/// A tiny twin's bring-up, its directory read cold to its first token checked, fails past this.
const BRINGUP_BUDGET_S: f64 = 300.0;
/// The bring-ups a recording times; the row's value is the slowest.
const BRINGUP_RUNS: usize = 3;
const CELLS: &str = "benchmarks/silicon/samba_cells.json";
const BRINGUP_LOG: &str = "logs/samba/bringup-e1_11.log";

fn repo() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Whether this run records E1.11's rows into the committed ledger: the explicit `LOGOS_RECORD_SAMBA_CELLS=1`.
fn recording() -> bool {
    std::env::var_os("LOGOS_RECORD_SAMBA_CELLS").is_some_and(|v| v == "1")
}

/// The digest of what a twin's bring-up reads: its configuration and every weight file, in name order.
fn twin_digest(dir: &std::path::Path) -> String {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().is_some_and(|n| n == "config.json") || p.extension().is_some_and(|x| x == "safetensors"))
        .collect();
    files.sort();
    let mut bytes = Vec::new();
    for f in &files {
        bytes.extend(f.file_name().unwrap().to_string_lossy().as_bytes());
        bytes.extend(std::fs::read(f).unwrap_or_else(|e| panic!("{}: {e}", f.display())));
    }
    logicaffeine_kernel::interface::lean::package::sha256_hex(&bytes)
}

/// One bring-up of `family`'s twin, timed: its directory opened cold, its spec read, its SN50 bundle written and run,
/// and the bundle's first token held to the f64 reference within the bf16 datapath. The wall seconds.
fn brought_up(dir: &std::path::Path, family: &str) -> f64 {
    let start = Instant::now();
    let spec = ModelSpec::from_source(&*logicaffeine_compile::model::open_checkpoint(dir).unwrap_or_else(|e| panic!("{family}: {e}")))
        .unwrap_or_else(|e| panic!("{family}: {e:?}"));
    let d = dir.to_path_buf();
    let source = move || logicaffeine_compile::model::open_checkpoint(&d).expect("the checkpoint") as Box<dyn WeightSource>;
    checkpoint_brings_up_close(dir, &spec, &source, &format!("{family} on the SN50"));
    start.elapsed().as_secs_f64()
}

/// The digests of `family`'s bring-up: its spec as read, and the manifest of the bundle `largo emit rdu` writes for it
/// (a bundle compiled twice is the same bytes, E8.3).
fn bringup_digests(dir: &std::path::Path, family: &str) -> (String, String) {
    let spec = ModelSpec::from_source(&*logicaffeine_compile::model::open_checkpoint(dir).unwrap()).unwrap();
    let out = tempfile::tempdir().unwrap();
    let bundle = out.path().join("bundle");
    let run = cli::largo_in(
        out.path(),
        &["emit", "rdu", "--chip", "sn50", "--model", dir.to_str().unwrap(), "--ctx", &CTX.to_string(), "-o", bundle.to_str().unwrap()],
    );
    assert_eq!(run.status.code(), Some(0), "{family}: the bundle:\n{}", cli::stderr(&run));
    let manifest = std::fs::read(bundle.join("manifest.json")).expect("manifest.json");
    let sha = logicaffeine_kernel::interface::lean::package::sha256_hex;
    (sha(format!("{spec:?}").as_bytes()), sha(&manifest))
}

/// E1.11's rows, measured: for each family the wall seconds of `runs` bring-ups, the slowest its value, with what it
/// brought up (the twin's digest, its spec's, its bundle manifest's), this host's boot and the log written beside it.
fn record_bringups(measured: &[(&str, &str, String, Vec<f64>)]) {
    let root = repo();
    let date = samba_cells::today();
    let (boot, host) = (samba_cells::kernel_fact("/proc/sys/kernel/random/boot_id"), samba_cells::kernel_fact("/proc/sys/kernel/hostname"));
    let mut log = format!("E1.11 bring-up clocks, {date}, {host}, boot {boot}, load average {}\n", samba_cells::kernel_fact("/proc/loadavg"));
    let path = root.join(CELLS);
    let mut ledger = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    for (cell, family, source, runs) in measured {
        let (dir, _) = common::hf_twins::twin(family);
        let (spec_digest, bundle_digest) = bringup_digests(&dir, family);
        let load = {
            let start = Instant::now();
            ModelSpec::from_source(&*logicaffeine_compile::model::open_checkpoint(&dir).unwrap()).unwrap();
            start.elapsed().as_secs_f64()
        };
        let slowest = runs.iter().copied().fold(0.0f64, f64::max);
        log += &format!(
            "{family}: twin sha256 {source}, spec sha256 {spec_digest}, manifest sha256 {bundle_digest}, spec read {load:.6} s, bring-ups (s): {}, slowest {slowest:.3} s of {BRINGUP_BUDGET_S} s\n",
            runs.iter().map(|r| format!("{r:.3}")).collect::<Vec<_>>().join(" ")
        );
        let row = serde_json::json!({
            "cell": cell,
            "side": "ours",
            "provenance": "measured",
            "date": date,
            "values": { BRINGUP_MEASURE: slowest },
            "runs": { BRINGUP_MEASURE: runs },
            "conditions": format!("the {family} twin of section 6.1's Tier A row, read cold from its Hugging Face directory: its spec, its SN50 bundle written and run, its first token held to the f64 reference within the bf16 datapath; the slowest of {BRINGUP_RUNS}, on this host"),
            "won": false,
            "program": { "spec_digest": spec_digest, "cut": format!("bringup:{family}"), "source_sha256": source },
            "bundle_digest": bundle_digest,
            "boot": { "id": boot, "host": host, "load_seconds": load },
            "log": BRINGUP_LOG
        });
        ledger = samba_cells::with_row(&ledger, row);
    }
    std::fs::create_dir_all(root.join(BRINGUP_LOG).parent().expect("the log has a directory")).expect("the log's directory");
    std::fs::write(root.join(BRINGUP_LOG), log).expect("the recording's log");
    std::fs::write(&path, ledger).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

#[test]
fn every_tier_a_family_brings_up_on_the_clock() {
    // E1.11: each Tier A family's twin, from its directory read cold to a first token the reference holds, inside the
    // budget; its bring-up recorded as a measured row of the cell ledger, of the twin as it is today.
    if !common::require_tool("hf-twins") {
        return;
    }
    let mut measured = Vec::new();
    for (cell, family) in TIER_A_TWINS {
        let (dir, _) = common::hf_twins::twin(family);
        let seconds = brought_up(&dir, family);
        eprintln!("RECEIPT E1.11 {family}: brought up in {seconds:.3} s");
        assert!(seconds <= BRINGUP_BUDGET_S, "{family} brings up in {seconds:.1} s, past {BRINGUP_BUDGET_S} s");
        if recording() {
            let runs: Vec<f64> = std::iter::once(seconds).chain((1..BRINGUP_RUNS).map(|_| brought_up(&dir, family))).collect();
            measured.push((cell, family, twin_digest(&dir), runs));
        }
    }
    if recording() {
        record_bringups(&measured);
    }
    let path = repo().join(CELLS);
    let ledger: samba_cells::Ledger =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).expect("the ledger reads");
    for (cell, family) in TIER_A_TWINS {
        let row = ledger.rows.iter().find(|r| r.cell == cell && r.side == "ours").unwrap_or_else(|| {
            panic!(
                "{CELLS} holds no row of {cell}; record E1.11's rows: LOGOS_RECORD_SAMBA_CELLS=1 cargo nextest run -p \
                 logicaffeine-tests --cargo-profile test-opt --test samba_bringup_clock_lock"
            )
        });
        assert_eq!(row.provenance, samba_cells::Provenance::Measured, "{cell}'s row is measured");
        let (dir, _) = common::hf_twins::twin(family);
        let program = row.program.as_ref().expect("the row names what it brought up");
        assert_eq!(program.source_sha256, twin_digest(&dir), "{cell}'s row is of another twin: re-record it");
        let runs = row.runs.get(BRINGUP_MEASURE).unwrap_or_else(|| panic!("{cell}'s row has no runs of `{BRINGUP_MEASURE}`"));
        assert!(runs.len() >= BRINGUP_RUNS, "{cell}'s row stands on {} runs", runs.len());
        let recorded = row.values[BRINGUP_MEASURE];
        assert_eq!(recorded, runs.iter().copied().fold(0.0f64, f64::max), "{cell}'s value is its slowest run");
        assert!(recorded <= BRINGUP_BUDGET_S, "{cell} was recorded at {recorded:.1} s, past {BRINGUP_BUDGET_S} s");
    }
}
