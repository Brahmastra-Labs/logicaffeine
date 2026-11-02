//! Every tiny architecture's compiled lane forms as cuts of the one IR, with the lane's shim for each and what the
//! lane's link reads beside it. `shim_lock` judges the lane's shim on them; `tensor_rule_proof_lock` proves the lane's
//! own rules on them.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::Path;

use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane::plan::{Fill as LaneFill, StepIo as LaneIo};
use logicaffeine_compile::codegen_cuda::lane::weights::Placement;
use logicaffeine_compile::codegen_cuda::lane::{batch_io, compile_batch_holding, decode_io, prefill_io, FormKind, LaneOptions};
use logicaffeine_compile::codegen_cuda::lane_shim::LaneShim;
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::tensor_graph::graph::TensorGraph;

use super::tiny_models::Tiny;
use super::{lane_cuts, scratch, tiny_models};

/// A compiled lane form's cut, with what the lane's link reads beside it: where each constant lives and the names the
/// step carries across steps.
pub struct LaneCut {
    pub what: String,
    pub g: TensorGraph,
    pub shim: LaneShim,
    pub constants: BTreeMap<String, Placement>,
    pub io: LaneIo,
}

/// Every tiny architecture's decode step, prefill chunk of 4 and batch of 2 under each numerics, and the latent
/// architectures' wide chunks of 64 at their wide shape under the quantized numerics — the chunks the lane attends a
/// latent expanded in — as lane cuts.
pub fn every_lane_cut() -> Vec<LaneCut> {
    let mut out = Vec::new();
    let all = [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8];
    for &t in tiny_models::ALL {
        let ckpt = scratch::dir("logos_lane_corpus").join(format!("{}.gguf", t.name()));
        cuts(&mut out, t.name(), &t.spec(24), &ckpt, &LaneOptions { ctx: 24, knobs: CudaKnobs::default(), prefill: 4 }, &all);
    }
    for t in [Tiny::DeepSeek2, Tiny::DeepSeek2Split] {
        let ckpt = scratch::dir("logos_lane_corpus").join(format!("{}-wide.gguf", t.name()));
        let options = LaneOptions { ctx: 128, knobs: CudaKnobs::default(), prefill: 64 };
        cuts(&mut out, &format!("{}-wide", t.name()), &t.wide(128), &ckpt, &options, &[Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8]);
    }
    out
}

/// `spec`'s lane, written to `ckpt` and compiled under each of `numerics` with a batch of 2, as one cut a form.
fn cuts(out: &mut Vec<LaneCut>, name: &str, spec: &ModelSpec, ckpt: &Path, options: &LaneOptions, numerics: &[Q8MatKind]) {
    tiny_models::write_checkpoint(spec, ckpt);
    for &activations in numerics {
        let knobs = CudaKnobs { activations, ..options.knobs };
        let lane = compile_batch_holding(spec, ckpt, &LaneOptions { knobs, ..*options }, 2, &["sequences"])
            .unwrap_or_else(|e| panic!("{name} {activations:?}: {e}"));
        for form in &lane.forms {
            let what = format!("{name} {activations:?} {}", form.tag());
            let g = lane_cuts::lifted(form, &what);
            let constants = form
                .schedule
                .buffers
                .iter()
                .filter_map(|(n, b)| match &b.fill {
                    LaneFill::Constant(p) => Some((n.clone(), p.clone())),
                    _ => None,
                })
                .collect();
            let io = match form.kind {
                FormKind::Decode => decode_io(),
                FormKind::Prefill => prefill_io(),
                FormKind::Batch => batch_io(form.rows),
            };
            out.push(LaneCut { what, g, shim: LaneShim { knobs, batch: form.kind == FormKind::Batch, held: Vec::new() }, constants, io });
        }
    }
}
