//! A model family's SN50 deployment, as the Samba book's DEPLOY tickets hold it (`benchmarks/PREPARE_FOR_SAMBA.md`):
//!
//!   • the family's tiny checkpoint is its Hugging Face twin, a directory transformers writes from the family's own
//!     configuration class (`scripts/accel/hf-tiny-twin.py`), and three computations of its first decode step agree:
//!     transformers' own forward of the directory; the f64 reference over it, within a millionth of the largest logit;
//!     and the bundle `largo emit rdu` writes from the directory, run off disk on the SN50's evaluator, within the bf16
//!     datapath's bound (`samba_bringup::checkpoint_brings_up_close`);
//!   • the authored step on the VM computes the reference's logits over the twin's architecture, within the
//!     conformance lock's bound, its weights an in-memory checkpoint at a vocabulary the VM's literal fills hold (the
//!     twin's own vocabulary, 128,256 rows and more, is millions of literals);
//!   • each size the family publishes, at its real widths, lowers for the SN50 one program per layer kind, and every
//!     program certifies.
//!
//! The including suite declares `mod common`, `mod cli` (`cli_common/mod.rs`), `mod tiny_models`
//! (`common/tiny_models.rs`), `mod samba_bringup` (`common/samba_bringup.rs`) and `mod model_harness`
//! (`common/model_harness.rs`).

#![allow(dead_code)]

use crate::common;
use crate::model_harness::{assert_close, checkpoint, filled, run, CTX as VM_CTX};
use crate::samba_bringup::{checkpoint_brings_up_close, BoxedSource, CTX, TOK};
use logicaffeine_compile::model::reference::{argmax, Reference, WeightSource};
use logicaffeine_compile::model::spec::{Ffn, Mixer, ModelSpec, Window};
use logicaffeine_compile::model::{author, open_checkpoint};
use logicaffeine_compile::spatial::certificate::certify_choreography;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::streamed::{lower_decode_streamed, Storage};
use std::collections::HashMap;

/// The vocabulary the VM's step over a twin's architecture holds.
const VM_VOCAB: usize = 96;

/// `family`'s twin deployed: transformers, the reference, the VM and the SN50 bundle agree on its first step.
pub fn the_tiny_deploys(family: &str) {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin(family);
    let open = || open_checkpoint(&dir).unwrap_or_else(|e| panic!("{family}: {e}"));
    let spec = ModelSpec::from_source(&*open()).unwrap_or_else(|e| panic!("{family}: {e:?}"));

    let theirs = common::hf_twins::transformers_logits(&dir, &[TOK]);
    let mut reference = Reference::new(spec.clone(), BoxedSource(open() as Box<dyn WeightSource>))
        .unwrap_or_else(|e| panic!("{family}: the reference reads the twin: {e:?}"));
    let ours = reference.step(&[TOK]).expect("the reference steps");
    assert_eq!(ours.len(), theirs.len(), "{family}: the vocabulary");
    let scale = theirs.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let worst = ours.iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(worst <= scale * 1e-6, "{family}: a reference logit is {worst:e} from transformers', past a millionth of {scale}");

    let small = ModelSpec { vocab: VM_VOCAB, ..spec.clone() };
    let weights = checkpoint(&small);
    let mut held = Reference::new(small.clone(), weights.clone()).unwrap_or_else(|e| panic!("{family}: {e:?}"));
    let want = held.step(&[TOK]).expect("the reference steps");
    let prog = author::decode_step(&small, "/w", VM_CTX, TOK as usize + 1, 1);
    let (vm, next) = run(&filled(&small, &prog, &weights, &HashMap::new()), small.vocab);
    assert_close(&vm, &want, &format!("{family}: the VM's step"));
    assert_eq!(next, argmax(&want), "{family}: the VM's token");

    let d = dir.clone();
    let source = move || open_checkpoint(&d).expect("the checkpoint") as Box<dyn WeightSource>;
    checkpoint_brings_up_close(&dir, &spec, &source, &format!("{family} on the SN50"));
}

/// The family's architecture, as its twin reads, at a published size: its widths, its heads and its vocabulary set from
/// the size's configuration, every layer's features the twin's. `dff` is a dense layer's width; a mixture's and a latent
/// attention's widths are their own.
pub struct Size {
    pub name: &'static str,
    pub d: usize,
    pub heads: usize,
    pub kv_heads: usize,
    pub head_dim: usize,
    pub dff: usize,
    pub vocab: usize,
    pub tied: bool,
    pub experts: Option<Experts>,
    pub latent: Option<Latent>,
    /// The score scale where the size's configuration names one apart from `1/√head_dim` (Gemma 3 27B's 1/√168).
    pub scale: Option<f64>,
    /// The sliding window of the layers that slide, where the size's differs from the twin's.
    pub window: Option<usize>,
    /// Whether the query, key and value projections are biased, where the size's differs from the twin's.
    pub bias: Option<bool>,
    /// Whether the query and key are normed per head, where the size's differs from the twin's.
    pub qk_norm: Option<bool>,
    /// A Gated DeltaNet layer's widths, where the family has them.
    pub gdn: Option<Gdn>,
    /// The caps on the scores and on the logits, where the size's differ from the twin's (Gemma 2's 50 and 30).
    pub softcaps: Option<(f64, f64)>,
}

/// A size's Gated DeltaNet: its key heads, value heads, each head's width and the convolution's taps.
pub struct Gdn {
    pub k_heads: usize,
    pub v_heads: usize,
    pub head_dim: usize,
    pub taps: usize,
}

/// A size's mixture: its experts, the experts a token takes, each routed expert's width, the shared experts' width, and
/// the group limit, when the family routes within groups.
pub struct Experts {
    pub count: usize,
    pub top_k: usize,
    pub dff: usize,
    pub shared_dff: usize,
    pub groups: Option<(usize, usize)>,
}

/// A size's latent attention: the query's low rank, the latent's, each head's unrotated and rotated query and key
/// dimensions, and its value width.
pub struct Latent {
    pub q_lora: Option<usize>,
    pub kv_lora: usize,
    pub qk_nope: usize,
    pub qk_rope: usize,
    pub v_dim: usize,
}

/// A dense size's defaults for the fields a mixture or a latent attention sets: `Size { name, d, …, ..DENSE }`.
pub const DENSE: Size = Size {
    name: "",
    d: 0,
    heads: 0,
    kv_heads: 0,
    head_dim: 0,
    dff: 0,
    vocab: 0,
    tied: false,
    experts: None,
    latent: None,
    scale: None,
    window: None,
    bias: None,
    qk_norm: None,
    gdn: None,
    softcaps: None,
};

/// `size` of the family `twin` reads as: every attention and dense FFN at the size's widths, the rotation over the
/// same share of each head, the score scale `1/√head_dim` where the twin's is, an embedding scaled where the twin's is
/// by the size's own `√d`, and the twin's layers, so every kind of layer the family has (the deployment lowers each kind
/// once).
pub fn at_size(twin: &ModelSpec, size: &Size) -> ModelSpec {
    let mut spec = twin.clone();
    (spec.d, spec.vocab, spec.tied_output) = (size.d, size.vocab, size.tied);
    spec.embedding_scale = spec.embedding_scale.map(|_| (size.d as f64).sqrt());
    if let Some((_, logits)) = size.softcaps {
        spec.final_logit_softcap = Some(logits);
    }
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            let rotated = a.rope.as_ref().map(|r| r.dims as f64 / a.head_dim as f64);
            (a.heads, a.kv_heads) = (size.heads, size.kv_heads);
            let (old_hd, old_vd) = (a.head_dim, a.value_dim);
            a.head_dim = size.head_dim;
            a.value_dim = size.head_dim * old_vd / old_hd;
            a.scale = size.scale.unwrap_or(a.scale * (old_hd as f64 / size.head_dim as f64).sqrt());
            if let (Some(Window::Sliding(w)), Some(span)) = (a.window.as_mut(), size.window) {
                *w = span;
            }
            if let Some(bias) = size.bias {
                a.bias = bias;
            }
            if let Some(qk_norm) = size.qk_norm {
                a.qk_norm = qk_norm;
            }
            if let Some((scores, _)) = size.softcaps {
                a.softcap = Some(scores);
            }
            if let (Some(r), Some(share)) = (a.rope.as_mut(), rotated) {
                r.dims = (share * size.head_dim as f64).round() as usize;
            }
        }
        if let (Mixer::GatedDelta(g), Some(s)) = (&mut layer.mixer, &size.gdn) {
            (g.k_heads, g.v_heads, g.head_dim, g.conv_taps) = (s.k_heads, s.v_heads, s.head_dim, s.taps);
        }
        if let (Mixer::Mla(m), Some(l)) = (&mut layer.mixer, &size.latent) {
            let old_key = (m.qk_nope + m.qk_rope) as f64;
            (m.heads, m.q_lora_rank, m.kv_lora_rank) = (size.heads, l.q_lora, l.kv_lora);
            (m.qk_nope, m.qk_rope, m.v_dim) = (l.qk_nope, l.qk_rope, l.v_dim);
            m.rope.dims = l.qk_rope;
            m.scale *= (old_key / (l.qk_nope + l.qk_rope) as f64).sqrt();
        }
        match (&mut layer.ffn, &size.experts) {
            (Ffn::Dense { dff, .. }, _) => *dff = size.dff,
            (Ffn::Moe(m), Some(e)) => {
                (m.experts, m.top_k, m.expert_dff) = (e.count, e.top_k, e.dff);
                m.shared_dff = if m.shared_experts > 0 { e.shared_dff } else { 0 };
                m.groups = m.groups.and(e.groups);
            }
            (Ffn::Moe(_), None) => panic!("{}: a mixture with no experts' widths", size.name),
        }
    }
    spec
}

/// `spec` lowered for the SN50 as a decode deployment at its real widths, every weight stored Q8_0: the embedding, one
/// program per layer kind and the head, each certified.
pub fn every_layer_kind_lowers_and_certifies(spec: &ModelSpec, what: &str) {
    let target = RduChip::Sn50.facts().spatial_target();
    let m = lower_decode_streamed(spec, Storage::AllQ8, CTX, &target, |s| s.to_string())
        .unwrap_or_else(|e| panic!("{what}: the deployment lowers: {e}"));
    let programs = m.embed.iter().map(|p| ("the embedding".to_string(), p)).chain(
        m.kinds.iter().enumerate().map(|(i, k)| (format!("layer kind {i}"), &k.program)),
    );
    for (which, p) in programs.chain([("the head".to_string(), &m.head)]) {
        let mut p = p.clone();
        p.certificate = None;
        certify_choreography(&p).unwrap_or_else(|e| panic!("{what}: {which} certifies: {e:?}"));
    }
}
