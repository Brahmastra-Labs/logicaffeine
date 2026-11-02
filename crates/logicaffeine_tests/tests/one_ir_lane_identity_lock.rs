//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE CUDA LANE'S PROGRAMS, PINNED BY DIGEST FOR ITS MOVE ONTO THE ONE IR (MIGRATE-1).
//!
//! `one_ir_identity_lock` pins what the engine, the spatial backends and the older forward print; this suite pins what
//! the lane prints, in `tests/fixtures/one_ir_identity/lane.txt`: every tiny architecture × the three numerics × the
//! decode step alone and with prefill chunks of 4 and 2 and a batch of 2 × `fuse_quant` on and off × the rules held
//! {none, all, each alone}, and the two storage knobs at their non-default points — the `.cu`, the sidecar, and each
//! form's schedule and arena. The migration (benchmarks/FUSIONS_WE_NEED.md, MIGRATE-1) changes none of them.
//!
//! The lane is the performance lane's while it moves: its pins were recorded at the start of the lane's freeze window,
//! on the tree before the migration, which the fixture's header names.
//!
//!   cargo nextest run -p logicaffeine-tests --test one_ir_lane_identity_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

#[path = "common/identity_pins.rs"]
mod identity_pins;
#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use identity_pins::{digest, hold, parallel, read_pins, refused, Pins};
use logicaffeine_compile::codegen_cuda::knobs::{CachePrecision, CudaKnobs, StatePrecision};
use logicaffeine_compile::codegen_cuda::lane::plan::RULES;
use logicaffeine_compile::codegen_cuda::lane::{compile_batch_holding, compile_holding, Lane, LaneOptions};
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::model::spec::ModelSpec;
use tiny_models::{Tiny, ALL};

const CTX: usize = 24;
const NUMERICS: [Q8MatKind; 3] = [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8];

/// A compiled lane's artifact: its program, its sidecar, and each form's schedule and arena.
fn lane_digest(lane: &Lane) -> String {
    let forms: Vec<String> = lane
        .forms
        .iter()
        .map(|f| format!("{}\u{0}{:?}\u{0}{:?}\u{0}{:?}", f.tag(), f.identity, f.schedule, f.arena))
        .collect();
    let mut parts: Vec<&[u8]> = vec![lane.program.cu.as_bytes(), &lane.program.sidecar];
    parts.extend(forms.iter().map(|f| f.as_bytes()));
    digest(&parts)
}

/// The two forms of a lane the pins compile: the decode step alone, and with prefill chunks of 4 and 2 and a batch of 2.
#[derive(Clone, Copy)]
enum Forms {
    Decode,
    ChunksAndBatch,
}

impl Forms {
    fn label(self) -> &'static str {
        match self {
            Forms::Decode => "p1",
            Forms::ChunksAndBatch => "p4b2",
        }
    }
}

fn lane_pin(spec: &ModelSpec, ckpt: &Path, forms: Forms, knobs: CudaKnobs, held: &[&str]) -> String {
    let compiled = match forms {
        Forms::Decode => compile_holding(spec, ckpt, &LaneOptions { ctx: CTX, knobs, prefill: 1 }, held),
        Forms::ChunksAndBatch => compile_batch_holding(spec, ckpt, &LaneOptions { ctx: CTX, knobs, prefill: 4 }, 2, held),
    };
    match compiled {
        Ok(lane) => lane_digest(&lane),
        Err(e) => refused(&e.to_string()),
    }
}

/// The held sets every lane is pinned under: none, all, and each rule alone.
fn held_sets() -> Vec<(String, Vec<&'static str>)> {
    let mut sets = vec![("none".to_string(), Vec::new()), ("all".to_string(), RULES.to_vec())];
    sets.extend(RULES.iter().map(|r| (format!("alone:{r}"), vec![*r])));
    sets
}

/// Every pin of one tiny architecture.
fn lane_pins_of(t: Tiny, tag: &str) -> Pins {
    let spec = t.spec(CTX);
    let ckpt = scratch::dir("logos_one_ir_identity").join(format!("{}-{tag}.gguf", t.name()));
    tiny_models::write_checkpoint(&spec, &ckpt);
    let mut out = Pins::new();
    for numerics in NUMERICS {
        for forms in [Forms::Decode, Forms::ChunksAndBatch] {
            let at = format!("{}/{numerics:?}/{}", t.name(), forms.label());
            for fuse_quant in [true, false] {
                let knobs = CudaKnobs { activations: numerics, fuse_quant, ..CudaKnobs::default() };
                let fq = if fuse_quant { "fq" } else { "nofq" };
                for (name, held) in held_sets() {
                    out.insert(format!("{at}/{fq}/{name}"), lane_pin(&spec, &ckpt, forms, knobs, &held));
                }
            }
            let state = CudaKnobs { activations: numerics, recurrent_state: StatePrecision::Bf16, ..CudaKnobs::default() };
            out.insert(format!("{at}/state-bf16/none"), lane_pin(&spec, &ckpt, forms, state, &[]));
            let cache = CudaKnobs { activations: numerics, cache_precision: CachePrecision::F16, ..CudaKnobs::default() };
            out.insert(format!("{at}/cache-f16/none"), lane_pin(&spec, &ckpt, forms, cache, &[]));
        }
    }
    out
}

fn lane_pins(tag: &str) -> Pins {
    parallel(ALL, |&t| lane_pins_of(t, tag))
}

#[test]
fn every_lane_program_is_the_pinned_one() {
    hold("lane", lane_pins);
}

/// The lane's pins hold each rule alone, for every architecture, numerics, form and quantize fusion: the held-alone
/// set is today's rule table, so a rule added to the lane has no pin until its family is re-recorded with it, and a
/// rule dropped from the pins is caught.
#[test]
fn the_pins_cover_every_rule_held_alone() {
    let pins = read_pins("lane");
    let mut alone: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for key in pins.keys() {
        let (at, held) = key.rsplit_once('/').expect("a lane key ends in its held set");
        if let Some(rule) = held.strip_prefix("alone:") {
            alone.entry(at.to_string()).or_default().insert(rule.to_string());
        }
    }
    let rules: BTreeSet<String> = RULES.iter().map(|r| r.to_string()).collect();
    let configurations = ALL.len() * NUMERICS.len() * 2 * 2;
    assert_eq!(alone.len(), configurations, "every architecture × numerics × form × quantize fusion holds each rule alone");
    for (at, held) in &alone {
        assert_eq!(held, &rules, "{at}: the rules held alone are the lane's rule table");
    }
}
