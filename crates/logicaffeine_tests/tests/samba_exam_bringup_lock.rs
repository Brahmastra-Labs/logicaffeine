//! samba_exam_bringup_lock — the rows of the Samba book's exam E1 (section 10.1 of `benchmarks/PREPARE_FOR_SAMBA.md`,
//! cold bring-up of configurations the compiler was not built for) that hold today.
//!
//! A row is brought up the way it would be on the day: its checkpoint is written to disk, handed to
//! `largo emit rdu --chip sn50 --model <checkpoint> --ctx 4 -o DIR`, and the bundle is read back off disk — every program
//! parsed and re-certified against the digest its manifest records — and run through the spatial evaluator for one
//! decode step from a fresh state, each tensor the manifest binds filled as a host fills it from the checkpoint
//! (`model::bind`). The token it emits is held to the token the f64 reference (`model::reference`) computes from the
//! same checkpoint (`common/samba_bringup.rs`). An architecture no arm reads is held to its refusal instead: within a
//! second, naming the family whose tensors it carries (`model::triage`).
//!
//! Every row was a handed RED its ticket moved here unchanged when it landed (its `frozen_reds_lock` row keeping its digest
//! under the new path); the last, E1.12, lowers an exported graph where the others lower a checkpoint: `largo emit rdu
//! --graph` on the archive `torch.export` writes, which needs PyTorch (`LOGOS_PYTHON`).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_bringup_lock`

#![cfg(feature = "codegen")]

mod common;
#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_bringup.rs"]
mod samba_bringup;
#[path = "common/samba_handoff.rs"]
#[allow(dead_code)]
mod samba_handoff;
#[path = "common/samba_exam.rs"]
#[allow(dead_code)]
mod samba_exam;

use logicaffeine_compile::model::hf::HfCheckpoint;
use logicaffeine_compile::model::open_checkpoint;
use logicaffeine_compile::model::program::Cut;
use logicaffeine_compile::model::reference::WeightSource;
use logicaffeine_compile::model::spec::{Attention, Mixer, ModelSpec};
use logicaffeine_compile::spatial::handoff::contract;
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::target::SpatialTarget;
use samba_bringup::{bring_up, brings_up, brings_up_close, checkpoint_brings_up_close, every, CTX};
use samba_exam::{reference_after, within_the_datapath};
use samba_handoff::{authored, run_at, Model, NEXT};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};
use tiny_models::{write_checkpoint, Tiny};

/// Every attention layer of `spec`, mutably.
fn attentions(spec: &mut ModelSpec) -> impl Iterator<Item = &mut Attention> {
    spec.layers.iter_mut().filter_map(|l| match &mut l.mixer {
        Mixer::Attention(a) => Some(a),
        _ => None,
    })
}

#[test]
fn every_published_vocabulary_brings_up() {
    // Four padded to a multiple of 128 (Qwen 2.5 and 3, Kimi K2, gpt-oss, Qwen 3.5), and GPT-2's 50,257, which no tile
    // divides: the head's last column tile is partly padding the arg-max must never pick.
    every([151_936, 163_840, 201_088, 248_320, 50_257].map(|v| (format!("a vocabulary of {v}"), v)), |vocab| {
        let mut spec = Tiny::Qwen35.spec(CTX);
        spec.vocab = vocab;
        brings_up(spec, &format!("a vocabulary of {vocab}"));
    });
}

#[test]
fn one_kv_head_and_all_kv_heads_bring_up() {
    let heads = Tiny::Qwen35
        .spec(CTX)
        .layers
        .iter()
        .find_map(|l| match &l.mixer {
            Mixer::Attention(a) => Some(a.heads),
            _ => None,
        })
        .expect("an attention layer");
    every([1, heads].map(|kv| (format!("{kv} KV heads of {heads}"), kv)), |kv| {
        let mut spec = Tiny::Qwen35.spec(CTX);
        for a in attentions(&mut spec) {
            a.kv_heads = kv;
        }
        brings_up(spec, &format!("{kv} KV heads of {heads}"));
    });
}

#[test]
fn a_hugging_face_directory_brings_up_with_no_gguf() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("llama");
    let spec = ModelSpec::from_source(&HfCheckpoint::open(&dir).unwrap()).expect("the twin's spec");
    let d = dir.clone();
    let (got, want) = bring_up(&dir, &spec, &move || Box::new(HfCheckpoint::open(&d).expect("the directory")) as Box<dyn WeightSource>);
    assert_eq!(got, want, "the Llama directory's bundle emits token {got}, the reference {want}");
}

#[test]
fn hidden_widths_of_2880_and_7168_bring_up() {
    every([2880, 7168].map(|d| (format!("a hybrid {d} wide"), d)), |d| {
        let mut spec = Tiny::Qwen35.spec(CTX);
        spec.d = d;
        brings_up(spec, &format!("a hybrid {d} wide"));
    });
}

#[test]
fn every_published_head_dim_brings_up() {
    let mut shapes: Vec<(String, ModelSpec)> = [64, 80, 96, 112, 128, 256]
        .map(|head_dim| {
            let mut spec = Tiny::Qwen35.spec(CTX);
            for a in attentions(&mut spec) {
                a.head_dim = head_dim;
                a.value_dim = head_dim;
                a.scale = 1.0 / (head_dim as f64).sqrt();
                if let Some(rope) = a.rope.as_mut() {
                    rope.dims = rope.dims.min(head_dim);
                }
            }
            (format!("attention heads of {head_dim}"), spec)
        })
        .into();
    // Latent attention's 192 = 128 + 64: the query and key heads, unrotated and rotated.
    let mut spec = Tiny::DeepSeek2.spec(CTX);
    for layer in &mut spec.layers {
        if let Mixer::Mla(m) = &mut layer.mixer {
            (m.qk_nope, m.qk_rope) = (128, 64);
            m.rope.as_mut().expect("DeepSeek's latent attention rotates").dims = 64;
        }
    }
    shapes.push(("latent attention of 128 + 64".to_string(), spec));
    every(shapes.into_iter().map(|(what, spec)| (what.clone(), (what, spec))), |(what, spec)| brings_up(spec, &what));
}

#[test]
fn heads_that_are_not_whole_tiles_turned_whole_bring_up() {
    // Heads of 80 and of 112 under a full rotation, NeoX's (Qwen 3) and Llama's interleaved one: each head shares tiles
    // with its neighbours and each rotary row (40 and 56 wide) starts mid-tile, so the decode turns its heads as one flat
    // stage reading the row at the step's position.
    let mut shapes = Vec::new();
    for t in [Tiny::Qwen3, Tiny::Llama] {
        for head_dim in [80, 112] {
            let mut spec = t.spec(CTX);
            for a in attentions(&mut spec) {
                a.head_dim = head_dim;
                a.value_dim = head_dim;
                a.scale = 1.0 / (head_dim as f64).sqrt();
                if let Some(rope) = a.rope.as_mut() {
                    rope.dims = head_dim;
                }
            }
            shapes.push((format!("{t:?} heads of {head_dim}, turned whole"), spec));
        }
    }
    every(shapes.into_iter().map(|(what, spec)| (what.clone(), (what, spec))), |(what, spec)| brings_up(spec, &what));
}

/// The cache E1.17's prompts are prefilled into: 64 positions, whose keys of 2 K/V heads of 80 are 320 tiles as a flat row,
/// past what one SN50 site holds.
const LONG: usize = 64;

/// Whether `p` attends through its heads' windows: the table of the windowed flash fold's windows is among its tensors.
fn attends_through_windows(p: &SpatialProgram) -> bool {
    p.tensors.iter().any(|t| t.name.ends_with("__flash_windows"))
}

#[test]
fn heads_that_are_not_whole_tiles_bring_up_over_a_cache_past_a_site() {
    // Heads of 80 and of 112 under a full rotation, over a cache of LONG positions: a prompt of 37 — past a tile of query
    // rows and of cached positions — prefilled on the SN50, and the token after it decoded there, every layer's attention
    // streaming the cache past the flash fold with each head read through the run of tiles around it. Qwen 3's QK-norm
    // over grouped K/V heads, Gemma 3's sliding windows turning by their own base, Gemma 2's capped scores.
    let sn50 = SpatialTarget::sn50_rdu();
    let mut shapes = Vec::new();
    for t in [Tiny::Qwen3, Tiny::Gemma3, Tiny::Gemma2] {
        for head_dim in [80, 112] {
            let mut spec = t.spec(LONG);
            for a in attentions(&mut spec) {
                a.head_dim = head_dim;
                a.value_dim = head_dim;
                a.scale = 1.0 / (head_dim as f64).sqrt();
                if let Some(rope) = a.rope.as_mut() {
                    rope.dims = head_dim;
                }
            }
            shapes.push((format!("{t:?} heads of {head_dim} over a cache of {LONG}"), spec));
        }
    }
    every(shapes.into_iter().map(|(what, spec)| (what.clone(), (what, spec))), |(what, spec)| {
        let m = Model::write(spec, &what);
        let prompt: Vec<u32> = (0..37u32).map(|i| (i * 5 + 3) % m.spec.vocab as u32).collect();
        let named = |n: &str| Some(n.to_string());
        let prefill = authored(&m, Cut::Prefill { rows: prompt.len(), ctx: LONG }, &sn50);
        assert!(attends_through_windows(&prefill), "{what}: the prefill streams its cache through its heads' windows");
        // The program counts tokens and positions from 1.
        let given = BTreeMap::from([
            ("toks".to_string(), prompt.iter().map(|t| f64::from(t + 1)).collect()),
            ("pos_s0".to_string(), vec![1.0]),
        ]);
        let left = run_at(&m, &prefill, LONG, &named, &given);
        let handed = contract(&m.spec, LONG).expect("the contract").take(prompt.len(), &left).unwrap_or_else(|e| panic!("{what}: {e}"));
        let mut given = handed.state.clone();
        given.insert("tok".into(), vec![f64::from(NEXT + 1)]);
        given.insert("pos".into(), vec![(prompt.len() + 1) as f64]);
        let x = run_at(&m, &authored(&m, Cut::DecodeEmbed, &sn50), LONG, &named, &given)["x"].clone();
        given.insert("x".into(), x);
        for layer in 0..m.spec.layers.len() {
            let p = authored(&m, Cut::DecodeLayer { layer, ctx: LONG }, &sn50);
            assert!(attends_through_windows(&p), "{what}: layer {layer}'s decode streams its cache through its heads' windows");
            let out = format!("xo{layer}");
            let v = run_at(&m, &p, LONG, &named, &given)[&out].clone();
            given.insert(out, v);
        }
        let logits = run_at(&m, &authored(&m, Cut::DecodeHead, &sn50), LONG, &named, &given)["logits"].clone();
        within_the_datapath(&m, &logits, &reference_after(&m, &prompt, NEXT), &what);
    });
}

#[test]
fn a_fused_projection_and_values_unlike_the_keys_bring_up() {
    // Phi-3's one query-key-value projection, GLM-4's with its bias, both under Llama's interleaved rotation, and values
    // half and twice as wide as the keys: each part of the fused projection read through its own rows, each value head
    // its own width.
    let shapes: [(&str, fn(&mut Attention)); 4] = [
        ("a fused projection", |a| a.fused_qkv = true),
        ("a fused, biased projection", |a| (a.fused_qkv, a.bias) = (true, true)),
        ("values half as wide as the keys", |a| a.value_dim = a.head_dim / 2),
        ("values twice as wide as the keys", |a| a.value_dim = a.head_dim * 2),
    ];
    every(shapes.map(|(what, change)| (what.to_string(), (what, change))), |(what, change)| {
        let mut spec = Tiny::Llama.spec(CTX);
        attentions(&mut spec).for_each(change);
        brings_up(spec, what);
    });
}

#[test]
fn an_unknown_architecture_is_refused_with_its_missing_pieces() {
    // A Llama checkpoint under an architecture name nothing reads: the refusal comes within a second and names the
    // architecture and the family whose tensors it carries, so what is missing is a name, not a model.
    let mut spec = Tiny::Llama.spec(CTX);
    spec.arch = "frobnicator".to_string();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frobnicator.gguf");
    write_checkpoint(&spec, &path);
    let start = Instant::now();
    let out = cli::largo_in(dir.path(), &["emit", "rdu", "--chip", "sn50", "--model", path.to_str().unwrap(), "--ctx", "4", "-o", "bundle"]);
    let took = start.elapsed();
    assert_ne!(out.status.code(), Some(0), "an unknown architecture is refused");
    let message = cli::stderr(&out);
    assert!(took < Duration::from_secs(1), "the refusal took {took:?}");
    for piece in ["frobnicator", "llama"] {
        assert!(message.contains(piece), "the refusal names `{piece}`:\n{message}");
    }
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

#[test]
fn an_nvfp4_release_brings_up_from_its_directory() {
    // NVIDIA's NVFP4 releases read in place, in ModelOpt's layout over Qwen3-MoE's experts and in compressed-tensors' over
    // Llama: every weight bound from its NVFP4 blocks times its own scale (`<weight>.scale`, one a weight or one an
    // expert), as llama.cpp's graph multiplies its products, and the bundle's first token within the datapath of the f64
    // reference's.
    if !common::require_tool("hf-twins") {
        return;
    }
    every(["qwen3moe-nvfp4", "llama-nvfp4"].map(|family| (family.to_string(), family)), |family| {
        let (dir, _) = common::hf_twins::twin(family);
        let spec = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the checkpoint")).expect("the spec");
        let d = dir.clone();
        let source = move || open_checkpoint(&d).expect("the checkpoint") as Box<dyn WeightSource>;
        checkpoint_brings_up_close(&dir, &spec, &source, &format!("{family} from its directory"));
    });
}

// ── E1.11: the bring-up clock, recorded for every Tier A model (its rows measured by `samba_bringup_clock_lock`) ──

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
fn tied_untied_and_biased_checkpoints_bring_up() {
    // gpt-oss: biases on the query, key, value and output projections, and on every expert projection. Each is held as
    // every bring-up is (Tristen, 2026-11-12): its logits within the bf16 datapath's bound, and the reference's token
    // wherever it leads by more than twice that.
    let shapes = [
        ("a head tied to the embedding", Tiny::LlamaTied.spec(CTX)),
        ("a head of its own", Tiny::Llama.spec(CTX)),
        ("biased projections", Tiny::GptOss.spec(CTX)),
    ];
    every(shapes.map(|(w, s)| (w.to_string(), (w, s))), |(what, spec)| brings_up_close(spec, what));
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
