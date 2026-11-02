//! What the decode engine runs: a checkpoint held, feature by feature and tensor by tensor, to the program
//! [`super::program::lower`] reads off the model's one program and the units compute, and every gap refused by name.
//!
//! The lowering reads a [`ModelSpec`] and writes the commands of a Qwen3.5/3.8-shaped step: RMSNorm before each
//! block, a Gated-DeltaNet or grouped-query attention mixer, a dense SwiGLU feed-forward block, the residual adds, the
//! final norm and the head. A model with anything else would lower to a program that computes something other than the
//! model, and nothing downstream would notice: the tensors of a GELU block are the tensors of a SwiGLU block. So
//! before a checkpoint becomes a program, [`refusals`] names each feature the engine does not carry
//! ([`spec_refusals`]), and then each tensor the program reads that the checkpoint lacks or stores in a format no unit
//! reads: the array reads [`numerics::reads`]'s block formats, the vector unit plain floats.
//!
//! An empty list is the engine's claim that it computes the model; [`admit`] is that claim as a gate.
//!
//! A checkpoint the engine computes runs on an engine build when the build has every unit, width and format its step
//! needs ([`needs`]): [`covers`] holds them against the build's [`EngineParams`], the dual of [`EngineParams::union`]
//! (a build covers a step exactly when the union of the two is the build), and names every gap. [`runs_on`] is both
//! at once: whether a checkpoint's header runs on a build, in the time it takes to read the header.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::model::quant::GgmlType;
use crate::model::spec::{Act, Ffn, GateActivation, Mixer, ModelSpec, RopeScaling};

use super::machine::{self, Layout, StoreLayout};
use super::model::{EngineNumerics, FACTORS};
use super::numerics;
use super::program::lower;
use super::rtl::engine::EngineParams;
use crate::spatial::decode::GgufHeader;
use crate::spatial::serving::StateFlush;

/// One thing a checkpoint needs that the engine does not do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A block's token mixer the engine has no unit for.
    Mixer { layer: usize, what: &'static str },
    /// A feed-forward block other than the dense SwiGLU the lowering writes.
    Ffn { layer: usize, what: &'static str },
    /// A norm the lowering does not apply.
    Norm { layer: usize, what: &'static str },
    /// An attention variant the attention unit does not compute.
    Attention { layer: usize, what: &'static str },
    /// A scale the token embedding is read with.
    EmbeddingScale,
    /// Soft-capped logits.
    LogitSoftcap,
    /// A tensor the program reads that the checkpoint does not hold.
    MissingTensor(String),
    /// A tensor the program reads in a format no unit reads.
    Format { tensor: String, format: &'static str },
    /// The model's one program does not lower to the engine, for the reason given.
    Program(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Mixer { layer, .. } | Refusal::Ffn { layer, .. } | Refusal::Norm { layer, .. } | Refusal::Attention { layer, .. } => {
                write!(f, "layer {layer}: {}", self.gap())
            }
            Refusal::EmbeddingScale => write!(f, "a scaled token embedding"),
            Refusal::LogitSoftcap => write!(f, "soft-capped logits"),
            Refusal::MissingTensor(t) => write!(f, "tensor `{t}` is not in the checkpoint"),
            Refusal::Format { tensor, format } => write!(f, "tensor `{tensor}` is {format}, which no unit reads"),
            Refusal::Program(why) => write!(f, "the model's program does not lower to the engine: {why}"),
        }
    }
}

/// Each feature of `spec` the lowering does not carry, layer by layer, then the model's own.
pub fn spec_refusals(spec: &ModelSpec) -> Vec<Refusal> {
    let mut out = Vec::new();
    for (layer, l) in spec.layers.iter().enumerate() {
        match &l.mixer {
            Mixer::GatedDelta(g) if g.gate == GateActivation::Sigmoid => out.push(Refusal::Mixer { layer, what: "a Gated DeltaNet gated by a sigmoid" }),
            Mixer::GatedDelta(_) => {}
            Mixer::Mla(_) => out.push(Refusal::Mixer { layer, what: "multi-head latent attention" }),
            Mixer::Attention(a) => {
                let mut attention = |cond: bool, what: &'static str| {
                    if cond {
                        out.push(Refusal::Attention { layer, what });
                    }
                };
                attention(a.bias || a.output_bias, "projection biases");
                attention(a.fused_qkv, "one fused query, key and value projection");
                attention(a.window.is_some(), "a sliding window");
                attention(a.sinks, "attention sinks");
                attention(a.softcap.is_some(), "soft-capped scores");
                attention(a.temperature.is_some(), "a per-position query temperature");
                attention(a.qk_unit_norm, "a weightless QK norm");
                attention(a.qk_norm_whole, "a QK norm across every head");
            }
        }
        match &l.ffn {
            Ffn::Dense { act: Act::Silu, .. } => {}
            Ffn::Dense { act: Act::GeluTanh, .. } => out.push(Refusal::Ffn { layer, what: "GELU" }),
            Ffn::Dense { act: Act::SwigluOai { .. }, .. } => out.push(Refusal::Ffn { layer, what: "gpt-oss's clamped SwiGLU" }),
            Ffn::Moe(_) => out.push(Refusal::Ffn { layer, what: "experts" }),
        }
        if l.post_mixer_norm {
            out.push(Refusal::Norm { layer, what: "a norm on the mixer's output" });
        }
        if l.post_ffn_norm {
            out.push(Refusal::Norm { layer, what: "a norm on the feed-forward output" });
        }
    }
    if spec.embedding_scale.is_some() {
        out.push(Refusal::EmbeddingScale);
    }
    if spec.final_logit_softcap.is_some() {
        out.push(Refusal::LogitSoftcap);
    }
    out
}

/// A format of plain floats, which the vector unit reads.
fn float(ty: GgmlType) -> bool {
    matches!(ty, GgmlType::F32 | GgmlType::F16 | GgmlType::Bf16)
}

/// Every gap between the checkpoint and the engine: [`spec_refusals`], and when there are none, the frequency factors a
/// rotation scaled by them turns by (`rope_freqs.weight`, written into its rotary tables) when `tensors` (the
/// checkpoint's directory, name and format) lacks them, and each tensor the program reads that `tensors` lacks or
/// stores in a format no unit reads.
pub fn refusals(spec: &ModelSpec, tensors: &[(String, GgmlType)]) -> Vec<Refusal> {
    let mut out = spec_refusals(spec);
    if !out.is_empty() {
        return out;
    }
    let program = match lower(spec, EngineNumerics::precise(), spec.ctx_len) {
        Ok(lowered) => lowered.program,
        Err(e) => {
            out.push(Refusal::Program(e.to_string()));
            return out;
        }
    };
    let factored = spec.layers.iter().any(|l| match &l.mixer {
        Mixer::Attention(a) => a.rope.as_ref().is_some_and(|r| r.scaling == RopeScaling::Factors),
        _ => false,
    });
    if factored && !tensors.iter().any(|(n, _)| n == FACTORS) {
        out.push(Refusal::MissingTensor(FACTORS.to_string()));
    }
    for name in program.tensors() {
        match tensors.iter().find(|(n, _)| n == name) {
            None => out.push(Refusal::MissingTensor(name.to_string())),
            Some((_, ty)) if !numerics::reads(*ty) && !float(*ty) => {
                out.push(Refusal::Format { tensor: name.to_string(), format: ty.name() })
            }
            Some(_) => {}
        }
    }
    out
}

/// The engine's claim that it computes the checkpoint, or the gaps that stand in the way.
pub fn admit(spec: &ModelSpec, tensors: &[(String, GgmlType)]) -> Result<(), Vec<Refusal>> {
    let r = refusals(spec, tensors);
    if r.is_empty() {
        Ok(())
    } else {
        Err(r)
    }
}

impl Refusal {
    /// The layer the refusal is about, for those that are a layer's.
    fn layer(&self) -> Option<usize> {
        match self {
            Refusal::Mixer { layer, .. } | Refusal::Ffn { layer, .. } | Refusal::Norm { layer, .. } | Refusal::Attention { layer, .. } => {
                Some(*layer)
            }
            _ => None,
        }
    }

    /// The refusal without its layer: the words the engine's shim ([`super::shim::EngineShim`]) refuses the op that
    /// carries it in.
    pub fn gap(&self) -> String {
        match self {
            Refusal::Mixer { what, .. } | Refusal::Norm { what, .. } => what.to_string(),
            Refusal::Ffn { what, .. } => format!("a feed-forward block of {what}"),
            Refusal::Attention { what, .. } => format!("attention with {what}"),
            other => other.to_string(),
        }
    }
}

/// `0–4, 6–10, 12` for a sorted list of layers.
fn ranges(layers: &[usize]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < layers.len() {
        let mut j = i;
        while j + 1 < layers.len() && layers[j + 1] == layers[j] + 1 {
            j += 1;
        }
        out.push(if i == j { layers[i].to_string() } else { format!("{}–{}", layers[i], layers[j]) });
        i = j + 1;
    }
    out.join(", ")
}

/// Each distinct gap once, in the order first met, with the layers it stands in: `every layer` when it is all of a
/// model's `layers`.
pub fn summarize(refusals: &[Refusal], layers: usize) -> Vec<String> {
    let mut gaps: Vec<(String, Vec<usize>)> = Vec::new();
    for r in refusals {
        let gap = r.gap();
        let k = match gaps.iter().position(|(g, _)| *g == gap) {
            Some(k) => k,
            None => {
                gaps.push((gap, Vec::new()));
                gaps.len() - 1
            }
        };
        if let Some(l) = r.layer() {
            gaps[k].1.push(l);
        }
    }
    gaps.into_iter()
        .map(|(gap, ls)| match ls.len() {
            0 => gap,
            n if n == layers => format!("{gap} (every layer)"),
            1 => format!("{gap} (layer {})", ls[0]),
            _ => format!("{gap} (layers {})", ranges(&ls)),
        })
        .collect()
}

/// The engine's verdict on a checkpoint described by its header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    /// The engine computes it.
    Admitted,
    /// What stands in the way, each a sentence.
    Refused(Vec<String>),
    /// The header describes no model this crate reads.
    Undescribed(String),
}

/// The verdict on the checkpoint `header` describes: its architecture read as a [`ModelSpec`], its tensor directory
/// as the formats it stores. A tensor of a type id no reader knows is refused as such.
pub fn judge(header: &GgufHeader) -> Verdict {
    let spec = match header.spec() {
        Ok(spec) => spec,
        Err(e) => return Verdict::Undescribed(e.to_string()),
    };
    let mut known = Vec::with_capacity(header.tensors.len());
    let mut unknown = Vec::new();
    for t in &header.tensors {
        match GgmlType::from_id(t.type_id) {
            Ok(ty) => known.push((t.name.clone(), ty)),
            Err(id) => unknown.push((t.name.clone(), id)),
        }
    }
    match admit(&spec, &known) {
        Ok(()) => Verdict::Admitted,
        Err(refused) => Verdict::Refused(
            summarize(&refused, spec.layers.len())
                .into_iter()
                .map(|line| match unknown.iter().find(|(n, _)| line == Refusal::MissingTensor(n.clone()).to_string()) {
                    Some((name, id)) => format!("tensor `{name}` is of type id {id}, which no reader knows"),
                    None => line,
                })
                .collect(),
        ),
    }
}

/// One checkpoint's row of the capability matrix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    /// The header ledger's file stem.
    pub checkpoint: String,
    pub verdict: Verdict,
}

/// The verdict on every header ledger (`*.json`) in `dir`, by file name.
pub fn matrix(dir: &std::path::Path) -> Result<Vec<Row>, String> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|p| {
            let header = GgufHeader::read_ledger(p).map_err(|e| format!("{}: {e}", p.display()))?;
            let checkpoint = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            Ok(Row { checkpoint, verdict: judge(&header) })
        })
        .collect()
}

/// What a checkpoint's step needs of an engine build: the engine its jobs need ([`EngineParams::of`]) and the block
/// formats of the matrices a hybrid build streams, in their ggml ids' order.
#[derive(Clone, Debug, PartialEq)]
pub struct Needs {
    pub engine: EngineParams,
    pub streamed: Vec<GgmlType>,
}

/// What the step of the checkpoint `header` describes needs, at the engine's fastest point, one sequence over at most
/// `ctx` positions.
pub fn needs(header: &GgufHeader, ctx: usize) -> Result<Needs, String> {
    let spec = header.spec().map_err(|e| e.to_string())?;
    let n = EngineNumerics::fastest();
    let steps = lower(&spec, n, spec.ctx_len.min(ctx)).map_err(|e| e.to_string())?.program;
    let layout = Layout::of(&spec, n, &steps, header, 1, spec.ctx_len.min(ctx)).map_err(|e| format!("{e:?}"))?;
    let jobs = machine::lower(&steps, &layout, spec.rms_eps as f32);
    let mut streamed: Vec<GgmlType> = layout.tensors.iter().filter(|t| t.streamed).map(|t| t.ty).collect();
    streamed.sort_by_key(|t| t.id());
    streamed.dedup();
    Ok(Needs { engine: EngineParams::of(&layout, &StoreLayout::of(&jobs, &layout, 1), &jobs), streamed })
}

/// One thing a step needs that an engine build does not have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gap {
    /// More of something the build holds a fixed number of.
    Short { what: &'static str, has: usize, needs: usize },
    /// Rows of a block format the build's matrix unit does not read, or reads only in rows of fewer blocks.
    Format { format: GgmlType, has: Option<usize>, needs: usize },
    /// A block format the build's streaming unit does not decode.
    Streamed { format: GgmlType },
    /// A recurrence unit of other heads (their width and how their memories are carried), or none.
    Recurrence { has: Option<(usize, StateFlush)>, needs: (usize, StateFlush) },
    /// An attention unit of other heads (query width, value width, positions, whether the math runs on the array), or
    /// none, or one over fewer positions.
    Attention { has: Option<(usize, usize, usize, bool)>, needs: (usize, usize, usize, bool) },
    /// A step whose matrix jobs fold their sums into their output slots, on a build whose matrix unit writes each as it is.
    Folds,
}

/// A recurrence unit's log as a gap names it: the tokens a log holds before they are replayed (one for a memory rewritten
/// every pass), or the windows a trailing flush replays a window behind.
fn log_of(flush: StateFlush) -> String {
    match flush {
        StateFlush::EveryPass => "a log of 1 token(s)".into(),
        StateFlush::Every { tokens } => format!("a log of {tokens} token(s)"),
        StateFlush::Trailing { tokens } => format!("a log of {tokens}-token windows replayed a window behind"),
    }
}

impl fmt::Display for Gap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Gap::Short { what, has, needs } => write!(f, "{needs} {what}, where the build holds {has}"),
            Gap::Format { format, has: None, needs } => {
                write!(f, "{} rows of {needs} blocks, a format the build's matrix unit does not read", format.name())
            }
            Gap::Format { format, has: Some(has), needs } => {
                write!(f, "{} rows of {needs} blocks, where the build's matrix unit reads rows of {has}", format.name())
            }
            Gap::Streamed { format } => write!(f, "{} matrices, a format the build's streaming unit does not decode", format.name()),
            Gap::Recurrence { has, needs: (width, flush) } => {
                write!(f, "a recurrence unit for heads of {width} with {}", log_of(*flush))?;
                match has {
                    None => write!(f, ", where the build has none"),
                    Some((w, flush)) => write!(f, ", where the build's is for heads of {w} with {}", log_of(*flush)),
                }
            }
            Gap::Attention { has, needs: (q, v, ctx, array) } => {
                let on = |array: bool| if array { "on the array" } else { "on the vector unit" };
                write!(f, "an attention unit for heads of {q} and {v} over {ctx} positions, its math {}", on(*array))?;
                match has {
                    None => write!(f, ", where the build has none"),
                    Some((q2, v2, ctx2, array2)) => {
                        write!(f, ", where the build's is for heads of {q2} and {v2} over {ctx2} positions, its math {}", on(*array2))
                    }
                }
            }
            Gap::Folds => write!(f, "matrix sums folded into their output slots, where the build's matrix unit writes each as it is"),
        }
    }
}

/// Whether the engine `build` runs the step `step` needs, or every gap: each width it holds at least as many of, each
/// block format its matrix unit reads in rows at least as long (the vector side's registers among them, as deep as the
/// step's deepest job that cannot run a chunk at a time), the recurrence unit of the same heads, the attention
/// unit of the same heads over at least as many positions, and on a hybrid build every streamed format decoded. A
/// build covers a step exactly when the union of the two ([`EngineParams::union`]) is the build. The vector unit's
/// function units set how fast a function row runs, not what runs, so they leave no gap. Every field of the step's
/// engine is named here, so a field `EngineParams` gains does not compile until it is judged.
pub fn covers(build: &EngineParams, step: &Needs) -> Result<(), Vec<Gap>> {
    let EngineParams {
        rows,
        vector_rows,
        side_rows,
        pair_rows,
        store_beats,
        job_words,
        tensors,
        layers,
        ropes,
        formats,
        delta,
        attend,
        function_units: _,
        folds,
        streaming: _,
    } = &step.engine;
    let mut gaps = Vec::new();
    for (what, has, needs) in [
        ("token-rows", build.rows, *rows),
        ("vector rows", build.vector_rows, *vector_rows),
        ("pair rows", build.pair_rows, *pair_rows),
        ("side rows", build.side_rows, *side_rows),
        ("store beats", build.store_beats, *store_beats),
        ("job words", build.job_words, *job_words),
        ("tensor-table entries", build.tensors, *tensors),
        ("layers", build.layers, *layers),
        ("rotary tables", build.ropes, *ropes),
    ] {
        if needs > has {
            gaps.push(Gap::Short { what, has, needs });
        }
    }
    for &(format, needs) in formats {
        let has = build.formats.iter().find(|(t, _)| *t == format).map(|&(_, b)| b);
        if has.is_none_or(|b| b < needs) {
            gaps.push(Gap::Format { format, has, needs });
        }
    }
    if let Some(needs) = *delta {
        if build.delta != Some(needs) {
            gaps.push(Gap::Recurrence { has: build.delta, needs });
        }
    }
    if let Some(needs @ (q, v, ctx, array)) = *attend {
        if !build.attend.is_some_and(|(q2, v2, ctx2, array2)| (q2, v2, array2) == (q, v, array) && ctx2 >= ctx) {
            gaps.push(Gap::Attention { has: build.attend, needs });
        }
    }
    if *folds && !build.folds {
        gaps.push(Gap::Folds);
    }
    if let Some(streaming) = &build.streaming {
        for &format in &step.streamed {
            if !streaming.set.formats.contains(&format) {
                gaps.push(Gap::Streamed { format });
            }
        }
    }
    if gaps.is_empty() {
        Ok(())
    } else {
        Err(gaps)
    }
}

/// Whether the checkpoint `header` describes runs on the engine `build` at `ctx` positions: the engine computes it
/// ([`judge`]) and the build covers its step ([`covers`]). Otherwise every reason, a sentence each.
pub fn runs_on(header: &GgufHeader, build: &EngineParams, ctx: usize) -> Result<(), Vec<String>> {
    match judge(header) {
        Verdict::Admitted => {}
        Verdict::Refused(gaps) => return Err(gaps),
        Verdict::Undescribed(why) => return Err(vec![why]),
    }
    let step = needs(header, ctx).map_err(|e| vec![e])?;
    covers(build, &step).map_err(|gaps| gaps.iter().map(Gap::to_string).collect())
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/spatial/decode_engine/capability/tests.rs"]
mod tests;
