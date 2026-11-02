//! layer_tour_lock — Demo 1's line from English to silicon is generated, and every statement lands.
//!
//! For one gated-delta layer and one gated-attention layer of the Qwen3.5 architecture (Qwen3.8's), every statement of
//! the layer's cut of the one program is matched to the launch the CUDA lane's decode step computes it in, with the
//! fusions that made it. Each statement lands in exactly one launch; the tour regenerates byte for byte
//! (`tests/fixtures/layer_tour_qwen35.md`; the real Qwen3.8-27B's, `benchmarks/results/demo/layer_tour_qwen38.md`, on
//! demand).

#[path = "common/layer_tour.rs"]
mod layer_tour;
#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::path::{Path, PathBuf};

use layer_tour::{declarations, to_markdown, tour};
use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane::{compile, Lane, LaneOptions};
use logicaffeine_compile::model::spec::{Mixer, ModelSpec};
use tiny_models::Tiny;

const CTX: usize = 24;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The first gated-delta layer and the first attention layer.
fn the_two_layers(spec: &ModelSpec) -> (usize, usize) {
    let delta = spec.layers.iter().position(|l| matches!(l.mixer, Mixer::GatedDelta(_))).expect("a gated-delta layer");
    let attention = spec.layers.iter().position(|l| matches!(l.mixer, Mixer::Attention(_))).expect("an attention layer");
    (delta, attention)
}

fn tiny() -> (ModelSpec, Lane) {
    let spec = Tiny::Qwen35.spec(CTX);
    let path = scratch::dir("logos_layer_tour").join("qwen35.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    let lane = compile(&spec, &path, &LaneOptions { ctx: CTX, knobs: CudaKnobs::default(), prefill: 1 }).expect("the tiny lane");
    (spec, lane)
}

fn tour_markdown(spec: &ModelSpec, lane: &Lane, ctx: usize, name: &str) -> String {
    let (delta, attention) = the_two_layers(spec);
    let mut md = String::new();
    for (layer, what) in [(delta, "a gated-delta layer"), (attention, "a gated-attention layer")] {
        let stops = tour(spec, lane, layer, ctx).unwrap_or_else(|e| panic!("{name} layer {layer}: {e}"));
        md.push_str(&to_markdown(&format!("{name}, layer {layer}: {what}"), declarations(spec, layer, ctx), &stops));
        md.push('\n');
    }
    md
}

#[test]
fn every_statement_of_the_layer_lands_in_exactly_one_launch() {
    let (spec, lane) = tiny();
    let (delta, attention) = the_two_layers(&spec);
    for layer in [delta, attention] {
        let stops = tour(&spec, &lane, layer, CTX).unwrap_or_else(|e| panic!("layer {layer}: {e}"));
        assert!(stops.len() >= 10, "layer {layer}: only {} statements", stops.len());
        assert!(stops.iter().any(|s| !s.rules.is_empty()), "layer {layer}: no statement was fused");
    }
}

#[test]
fn the_tour_regenerates_byte_identical() {
    let (spec, lane) = tiny();
    let md = tour_markdown(&spec, &lane, CTX, "Qwen3.5 (tiny)");
    let fixture = root().join("crates/logicaffeine_tests/tests/fixtures/layer_tour_qwen35.md");
    if std::env::var("LOGOS_UPDATE_FIXTURES").is_ok() {
        std::fs::write(&fixture, &md).expect("write the fixture");
        return;
    }
    let pinned = std::fs::read_to_string(&fixture)
        .unwrap_or_else(|e| panic!("{} ({e}): record it with LOGOS_UPDATE_FIXTURES=1", fixture.display()));
    assert_eq!(pinned, md, "the tour moved: a statement lands in another launch than the pinned tour says");
}

/// The real Qwen3.8-27B's tour, written to `benchmarks/results/demo/layer_tour_qwen38.md`. On demand.
#[test]
#[ignore = "reads the real checkpoint: cargo nextest run -p logicaffeine-tests --test layer_tour_lock --run-ignored only"]
fn the_real_qwen38_tour_regenerates() {
    use logicaffeine_compile::model::gguf::Gguf;
    let dir = root().join("models/qwen3.8-27b");
    let gguf = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|x| x == "gguf"))
        .expect("the Qwen3.8-27B checkpoint");
    let spec = ModelSpec::from_gguf(&Gguf::open(&gguf).unwrap()).unwrap();
    let ctx = 4096;
    let lane = compile(&spec, &gguf, &LaneOptions { ctx, knobs: CudaKnobs::default(), prefill: 1 }).expect("the lane");
    let md = tour_markdown(&spec, &lane, ctx, "Qwen3.8-27B");
    let out = root().join("benchmarks/results/demo/layer_tour_qwen38.md");
    if std::env::var("LOGOS_UPDATE_FIXTURES").is_ok() {
        std::fs::write(&out, &md).unwrap();
        return;
    }
    assert_eq!(std::fs::read_to_string(&out).unwrap(), md, "the real tour moved");
}
