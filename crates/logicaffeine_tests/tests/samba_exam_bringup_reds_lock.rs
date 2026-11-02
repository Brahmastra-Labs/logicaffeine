//! samba_exam_bringup_reds_lock — the rows of the Samba book's exam E1 (section 10.1 of
//! `benchmarks/PREPARE_FOR_SAMBA.md`, cold bring-up of configurations the compiler was not built for) still open, each a
//! handed RED frozen in `fixtures/frozen_reds.tsv`.
//!
//! A row is brought up as `samba_exam_bringup_lock`'s are (`common/samba_bringup.rs`): the checkpoint on disk, the
//! bundle `largo emit rdu --chip sn50 --model <checkpoint> --ctx 4 -o DIR` writes read back off disk and re-certified,
//! one decode step through the spatial evaluator with every tensor filled from the checkpoint, the token held to the
//! f64 reference's. A row over several shapes fails naming every shape that does not hold, and why.
//!
//! Each row's ticket is the book's; the configuration key row (E1.10) is green since INTAKE-1
//! (`model_hf_intake_lock::an_unknown_key_is_refused_by_name`). A ticket that lands a row moves its test, unchanged,
//! into `samba_exam_bringup_lock`.
//!
//! Run on purpose: `scripts/run-handed-reds-tests.sh`, or
//! `cargo nextest run -p logicaffeine-tests --features handed-reds --cargo-profile test-opt --test samba_exam_bringup_reds_lock`.

#![cfg(feature = "codegen")]
#![cfg(feature = "handed-reds")]

mod common;
#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_bringup.rs"]
mod samba_bringup;

use logicaffeine_compile::model::open_checkpoint;
use logicaffeine_compile::model::reference::WeightSource;
use logicaffeine_compile::model::spec::{Attention, Mixer, ModelSpec};
use samba_bringup::{brings_up, checkpoint_brings_up_close, every, CTX};
use std::path::Path;
use tiny_models::{write_checkpoint, Tiny};

/// Every attention layer of `spec`, mutably.
fn attentions(spec: &mut ModelSpec) -> impl Iterator<Item = &mut Attention> {
    spec.layers.iter_mut().filter_map(|l| match &mut l.mixer {
        Mixer::Attention(a) => Some(a),
        _ => None,
    })
}

#[test]
fn head_counts_no_width_divides_bring_up_at_16_sockets() {
    // 24, 40 and 56 query heads over 16 sockets: no socket count divides the heads, and the plan still places every
    // head once and the bundle still emits the reference's token.
    every([24, 40, 56].map(|h| (format!("{h} heads at 16 sockets"), h)), |heads| {
        let mut spec = Tiny::Qwen35.spec(CTX);
        for a in attentions(&mut spec) {
            a.heads = heads;
            a.kv_heads = 8;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("heads-{heads}.gguf"));
        write_checkpoint(&spec, &path);
        let bundle = dir.path().join("bundle");
        let out = cli::largo_in(
            dir.path(),
            &["emit", "rdu", "--chip", "sn50", "--model", path.to_str().unwrap(), "--ctx", "4", "--tp", "16", "-o", bundle.to_str().unwrap()],
        );
        assert_eq!(out.status.code(), Some(0), "{heads} heads at 16 sockets:\n{}", cli::stderr(&out));
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(bundle.join("manifest.json")).expect("manifest.json")).unwrap();
        assert_eq!(manifest["tensor_parallel"], 16, "{heads} heads: the bundle is the 16-socket plan");
    });
}

#[test]
fn prime_and_single_layer_counts_bring_up() {
    // 61 layers, as DeepSeek-V3: the hybrid's period leaves a partial period at the end. And one layer.
    let mut deep = Tiny::Qwen35.spec(CTX);
    let period = deep.layers.len();
    deep.layers = (0..61).map(|l| deep.layers[l % period].clone()).collect();
    let mut single = Tiny::Llama.spec(CTX);
    single.layers.truncate(1);
    every([("61 layers".to_string(), deep), ("a single layer".to_string(), single)].map(|(w, s)| (w.clone(), (w, s))), |(what, spec)| {
        brings_up(spec, &what)
    });
}

#[test]
fn tied_untied_and_biased_checkpoints_bring_up() {
    // gpt-oss: biases on the query, key, value and output projections, and on every expert projection.
    let shapes = [
        ("a head tied to the embedding", Tiny::LlamaTied.spec(CTX)),
        ("a head of its own", Tiny::Llama.spec(CTX)),
        ("biased projections", Tiny::GptOss.spec(CTX)),
    ];
    every(shapes.map(|(w, s)| (w.to_string(), (w, s))), |(what, spec)| brings_up(spec, what));
}

/// The rows of section 6.1's Tier A table: one model family each.
fn tier_a_models(book: &str) -> Vec<String> {
    let at = book.find("**Tier A:").expect("section 6.1's Tier A");
    let rest = &book[at..];
    let end = rest[1..].find("**Tier ").map(|i| i + 1).unwrap_or(rest.len());
    rest[..end]
        .lines()
        .filter(|l| l.starts_with("| ") && !l.starts_with("| Model ") && !l.starts_with("|---"))
        .map(|l| l.trim_start_matches("| ").split(" | ").next().unwrap_or("").to_string())
        .collect()
}

#[test]
fn the_bring_up_clock_is_recorded_for_every_tier_a_model() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let book = std::fs::read_to_string(repo.join("benchmarks/PREPARE_FOR_SAMBA.md")).unwrap();
    let models = tier_a_models(&book);
    assert!(!models.is_empty(), "section 6.1 names its Tier A models");
    let ledger: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo.join("benchmarks/silicon/samba_cells.json")).unwrap()).unwrap();
    let clocks: Vec<&serde_json::Value> = ledger["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["side"] == "ours" && r["cell"].as_str().is_some_and(|c| c.contains("-BRINGUP")) && r["values"]["bringup_s"].is_number())
        .collect();
    assert!(
        clocks.len() >= models.len(),
        "{} bring-up clocks recorded for {} Tier A models ({models:?})",
        clocks.len(),
        models.len()
    );
}

#[test]
fn an_exported_graph_brings_up_without_a_spec_arm() {
    if !common::require_tool("torch") {
        return;
    }
    // A two-layer network no spec arm describes, exported by torch.export, lowered from the graph alone.
    let dir = tempfile::tempdir().unwrap();
    let graph = dir.path().join("net.pt2");
    let script = format!(
        "import torch\nclass Net(torch.nn.Module):\n    def __init__(s):\n        super().__init__()\n        s.a = torch.nn.Linear(64, 96)\n        s.b = torch.nn.Linear(96, 32)\n    def forward(s, x):\n        return s.b(torch.nn.functional.gelu(s.a(x)))\ntorch.manual_seed(0)\nep = torch.export.export(Net(), (torch.randn(1, 64),))\ntorch.export.save(ep, r'{}')\n",
        graph.display()
    );
    let made = std::process::Command::new(common::python_for_tools()).args(["-c", &script]).output().expect("python");
    assert!(made.status.success(), "the export: {}", String::from_utf8_lossy(&made.stderr));
    let bundle = dir.path().join("bundle");
    let out = cli::largo_in(dir.path(), &["emit", "rdu", "--chip", "sn50", "--graph", graph.to_str().unwrap(), "-o", bundle.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "an exported graph lowers:\n{}", cli::stderr(&out));
    assert!(bundle.join("manifest.json").is_file(), "the graph's bundle has its manifest");
}

#[test]
fn a_qwen3next_checkpoint_brings_up_from_its_directory_and_its_gguf() {
    // Qwen3-Next as its converter writes it: value heads grouped by key head, each key head's betas and alphas one
    // projection (`ssm_ba`), a mixture with a gated shared expert in every block and a draft block after the last.
    // `largo emit rdu` opens either through `model::open_checkpoint`, which presents it in Qwen3.5's tiled layout.
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, gguf) = common::hf_twins::twin("qwen3next");
    every([("its directory", dir), ("its converted GGUF", gguf)].map(|(what, path)| (what.to_string(), (what, path))), |(what, path)| {
        let spec = ModelSpec::from_source(&*open_checkpoint(&path).expect("the checkpoint")).expect("Qwen3-Next's spec");
        let p = path.clone();
        let source = move || open_checkpoint(&p).expect("the checkpoint") as Box<dyn WeightSource>;
        checkpoint_brings_up_close(&path, &spec, &source, &format!("Qwen3-Next from {what}"));
    });
}
