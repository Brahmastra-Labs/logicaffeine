//! A model as the RDU would run it: one lowered program per LAYER KIND, and a manifest.
//!
//! Two layers whose specs are equal ([`crate::model::spec::Layer`]: the mixer, its rotation and window, the
//! feed-forward block, the norms) are written by the author as the same program up to their names, so they lower to
//! the same spatial program — measured field by field, the only differences are name labels. Qwen3.8's 64-layer
//! forward is not 64 programs; it is two (its Gated DeltaNet layers and its attention layers), Gemma 3's are its
//! windowed and its global attention, DeepSeek-V2's its dense first layer and its expert layers: each applied layer
//! after layer with a different set of weights bound each time, followed by the head. This module builds exactly that
//! for any model the author writes: the kind programs, lowered and certified once each, and a manifest saying which
//! kind runs at each layer and how that kind's tensor names bind to that layer's tensors.
//!
//! Lowering cost is therefore independent of depth — one layer's worth per kind, however many layers the model has —
//! and so is the artifact a vendor ingests.
//!
//! ★ A WEIGHT MOVES AS IT IS STORED. The author declares every projection as a weight the deployment may store Q8_0;
//! a product over one folds its int8 values and block scales on the chip ([`super::passes::from_q8`]) only where the
//! checkpoint stores it so ([`Storage`]). A weight stored any other way is read dense, the host filling it through
//! its binding.

use super::ir::SpatialProgram;
use super::pipeline::compile_program_to_spatial_tp_agreeing_stored;
use super::pipeline::{compile_program_to_spatial_agreeing_stored, dense_tables_of};
use super::target::SpatialTarget;
use super::tensor_parallel::TpManifest;
use crate::model::author::{layer_input, stacked_layer_input, TpAuthored};
use crate::model::program::{written_in, written_split, Cut, Identity};
use crate::model::bind::{bind, stored_as_q8_planes, Source};
use crate::model::reference::WeightSource;
use crate::model::spec::{Ffn, Mixer, ModelSpec};
use std::collections::BTreeSet;

/// Which of the weights the author declares Q8_0 the checkpoint stores Q8_0.
#[derive(Clone, Copy)]
pub enum Storage<'a> {
    /// Every one: a built-in model's checkpoint, written as the author declares it.
    AllQ8,
    /// As this checkpoint stores each ([`stored_as_q8_planes`]), read from its description of the tensor.
    Checkpoint(&'a dyn WeightSource),
}

impl Storage<'_> {
    /// Whether `spec`'s declared weight `var` (`Wq3`) is read as it is stored, Q8_0.
    pub fn q8(&self, spec: &ModelSpec, var: &str) -> bool {
        match self {
            Storage::AllQ8 => true,
            Storage::Checkpoint(source) => match bind(spec, var) {
                Ok(Source::Weight(b)) => stored_as_q8_planes(&b, *source).unwrap_or(false),
                _ => false,
            },
        }
    }
}

/// One lowered, certified layer program, standing for every layer of its kind.
#[derive(Clone, Debug)]
pub struct LayerKind {
    /// What the kind's layers are, unique among the model's kinds: `attention`, `gated deltanet`, `attention, window
    /// 512`, `latent attention, experts`.
    pub name: String,
    /// The layer this program was lowered from; its tensor names are that layer's.
    pub representative: usize,
    /// The cut of the model's one program the representative's text is.
    pub identity: Identity,
    pub program: SpatialProgram,
    /// A tensor-parallel kind's manifest, in the representative's names: each chip-local tensor's chip and the slice
    /// of the checkpoint it is.
    pub tp: Option<TpManifest>,
    /// The state a step of this kind carries to the next, in the representative's names — what a deployment allocates
    /// once for a session. A table the step assembles whole is the step's own.
    pub carried: BTreeSet<String>,
}

/// How one layer of the forward runs: which kind, and how the kind program's tensor names map onto this
/// layer's tensors. Every tensor the kind program declares appears exactly once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerBinding {
    pub layer: usize,
    pub kind: usize,
    pub tensors: Vec<(String, String)>,
    /// The cut of the model's one program this layer's own text is.
    pub identity: Identity,
}

/// A program of the bundle that did not lower: which one, and the whole-program front end's own refusal — the
/// statements it could not lift, by index and binder, or the fault that kept the composition from certifying.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamedReject {
    /// `embed`, `layer 3 (attention)`, `head` — or `bundle` for a request no program is lowered for.
    pub program: String,
    pub why: String,
}

impl std::fmt::Display for StreamedReject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the {} program does not lower: {}",
            self.program, self.why
        )
    }
}

#[derive(Clone, Debug)]
pub struct StreamedModel {
    /// A decode bundle's embedding lookup (`x = row tok of Emb`); `None` for the authored forward, whose first
    /// hidden state is an input.
    pub embed: Option<SpatialProgram>,
    /// The cut of the model's one program the embedding program was lowered from.
    pub embed_identity: Option<Identity>,
    pub kinds: Vec<LayerKind>,
    pub head: SpatialProgram,
    /// The cut of the model's one program the head was lowered from.
    pub head_identity: Identity,
    pub layers: Vec<LayerBinding>,
    /// Where the deployment keeps every program's carried state — the SRAM or HBM/DDR — and what it reserves for the
    /// largest program's own footprint ([`super::residency::plan_deployment`]); every state tensor carries its tier.
    pub residency: super::residency::ResidencyPlan,
}

/// The name layer `to` gives the tensor that layer `from` calls `name`. The hidden-state input is named by
/// position (`x` for the first layer, `xo{l-1}` after); every per-layer tensor carries its layer's index as
/// a suffix (`Wq0`, `x2n3`); anything else — the shared RoPE tables (whose suffix is a rotation's index, never a
/// layer's), the lowerings' shared scalers — is the same tensor in every layer.
///
/// ★ AND A DERIVED NAME CARRIES ITS PARENT'S INDEX IN THE MIDDLE. The blocked lowerings name their
/// intermediates after the tensor they come from — `xn0__xsq`, `od0_u` — so the index is not at the end.
/// Renaming only a trailing index left every such intermediate named after the REPRESENTATIVE layer in the
/// manifest (`xn0__xsq` at layer 17), which no execution test sees because intermediates are computed, not
/// bound. The part before the first `_` is the name that carries the index.
pub fn rebind(name: &str, from: usize, to: usize) -> String {
    rebind_via(name, from, to, layer_input)
}

/// [`rebind`] for a bundle whose layers name their hidden-state input with `input` — the stacked `X`/`bxo{l-1}` of a
/// batch. A per-sequence tensor keeps its sequence suffix (`Kc3_s1` at layer 7 is `Kc7_s1`).
pub fn rebind_via(name: &str, from: usize, to: usize, input: fn(usize) -> String) -> String {
    if name == input(from) {
        return input(to);
    }
    // A host table is every layer's that reads it, and its suffix is no layer's: a rotary table's is its rotation's index
    // (`Rcos1`), a table of ones' its width (`Ones128`).
    if crate::model::bind::HostTable::named(name).is_some() {
        return name.to_string();
    }
    let (head, tail) = match name.find('_') {
        Some(i) => name.split_at(i),
        None => (name, ""),
    };
    let suffix = from.to_string();
    match head.strip_suffix(suffix.as_str()) {
        Some(stem) if !stem.is_empty() && !stem.ends_with(|c: char| c.is_ascii_digit()) => {
            format!("{stem}{to}{tail}")
        }
        _ => name.to_string(),
    }
}

/// One program of the deployment: its source, a tensor-parallel program's manifest, and the weights it declares that the
/// checkpoint stores Q8_0 — each read as it is stored wherever the product over it folds ([`super::passes::from_q8`]).
type Generated = (String, Option<TpManifest>, BTreeSet<String>, Identity);

/// A program's tokens: each maximal run of letters, digits and `_`, and each other character but whitespace.
fn tokens(src: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = src.trim_start();
    while let Some(c) = rest.chars().next() {
        let len = if c.is_ascii_alphanumeric() || c == '_' {
            rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len())
        } else {
            c.len_utf8()
        };
        out.push(&rest[..len]);
        rest = rest[len..].trim_start();
    }
    out
}

/// The names a program binds: every name a `Let` introduces. Every other word — a keyword, an operation (`plus`,
/// `gelu`), a function — is the language's, and two programs of one kind spell it alike.
fn binders<'a>(tokens: &[&'a str]) -> BTreeSet<&'a str> {
    tokens.windows(2).filter(|w| w[0] == "Let").map(|w| w[1]).collect()
}

/// The digest of a program up to the names it binds: every bound name replaced by the order of its first appearance and
/// whether the checkpoint stores it as Q8_0 planes (`stored`), every other token kept. Two programs share it exactly when
/// one is the other with its names consistently renamed, each weight read the same way.
fn shape_digest(src: &str, stored: &BTreeSet<String>) -> [u8; 32] {
    let toks = tokens(src);
    let names = binders(&toks);
    let mut order: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut canonical = String::with_capacity(src.len());
    for t in toks {
        if names.contains(t) {
            let next = order.len();
            canonical.push_str(&format!("\u{1}{}", order.entry(t).or_insert(next)));
            if stored.contains(t) {
                canonical.push('\u{2}');
            }
        } else {
            canonical.push_str(t);
        }
        canonical.push(' ');
    }
    crate::crypto_accel::sha256(canonical.as_bytes())
}

/// The renaming that turns program `from` into program `to`, bound name by bound name, when they share a
/// [`shape_digest`]: the tokens aligned position by position, every word but a bound name kept exactly. `None` when they
/// are not one program up to the names they bind.
pub fn renaming(from: &str, to: &str) -> Option<std::collections::BTreeMap<String, String>> {
    let (a, b) = (tokens(from), tokens(to));
    if a.len() != b.len() {
        return None;
    }
    let (names_a, names_b) = (binders(&a), binders(&b));
    let (mut forward, mut back) = (std::collections::BTreeMap::new(), std::collections::BTreeMap::new());
    for (x, y) in a.into_iter().zip(b) {
        if names_a.contains(x) != names_b.contains(y) {
            return None;
        }
        if names_a.contains(x) {
            if *forward.entry(x.to_string()).or_insert_with(|| y.to_string()) != y
                || *back.entry(y.to_string()).or_insert_with(|| x.to_string()) != x
            {
                return None;
            }
        } else if x != y {
            return None;
        }
    }
    Some(forward)
}

/// The model's layer kinds, in the order their first layer appears: each kind's name and its layers. Two layers are of
/// one kind when their authored programs (`source_of`: the text, and the weights it reads as stored Q8_0) are one
/// program up to the names they give — the same [`shape_digest`] — so one lowered program serves both, bound to each
/// layer's own tensors by the [`renaming`] between their programs.
pub fn layer_kinds(spec: &ModelSpec, source_of: &dyn Fn(usize) -> (String, BTreeSet<String>)) -> Vec<(String, Vec<usize>)> {
    let mut kinds: Vec<([u8; 32], Vec<usize>)> = Vec::new();
    for l in 0..spec.layers.len() {
        let (src, stored) = source_of(l);
        let digest = shape_digest(&src, &stored);
        match kinds.iter_mut().find(|(d, _)| *d == digest) {
            Some((_, layers)) => layers.push(l),
            None => kinds.push((digest, vec![l])),
        }
    }
    let describe = |l: usize| {
        let layer = &spec.layers[l];
        let mut name = match &layer.mixer {
            Mixer::Attention(a) => match a.window {
                Some(crate::model::spec::Window::Sliding(w)) => format!("attention, window {w}"),
                Some(crate::model::spec::Window::Chunked(c)) => format!("attention, chunks of {c}"),
                None => "attention".to_string(),
            },
            Mixer::GatedDelta(_) => "gated deltanet".to_string(),
            Mixer::Mla(_) => "latent attention".to_string(),
        };
        if matches!(layer.ffn, Ffn::Moe(_)) {
            name.push_str(", experts");
        }
        name
    };
    let names: Vec<String> = kinds.iter().map(|(_, layers)| describe(layers[0])).collect();
    kinds
        .into_iter()
        .zip(&names)
        .map(|((_, layers), name)| {
            // Kinds a description does not tell apart are told apart by their first layer.
            let unique = names.iter().filter(|n| *n == name).count() == 1;
            (if unique { name.clone() } else { format!("{name} (layer {})", layers[0]) }, layers)
        })
        .collect()
}

/// The weights a generated program stores Q8_0, from the generator's Q8_0 weight table.
/// The chip-local slices of a tensor-parallel layer whose full weight the checkpoint stores Q8_0: each chip reads its
/// slice as stored — a column slice always, a row slice when its rows are whole Q8_0 blocks (its scales are then whole
/// rows of the full weight's scales).
fn q8_slices<V>(m: &TpManifest, full: &std::collections::HashMap<String, V>, spec: &ModelSpec, storage: Storage) -> BTreeSet<String> {
    let whole_blocks = |s: &super::tensor_parallel::TpSlice| {
        s.axis == super::tensor_parallel::SliceAxis::Cols
            || s.ranges.iter().all(|(start, n)| {
                start % super::passes::from_q8::Q8_BLOCK == 0
                    && n % super::passes::from_q8::Q8_BLOCK == 0
            })
    };
    m.slices
        .iter()
        .filter(|(_, s)| full.contains_key(&s.of) && storage.q8(spec, &s.of) && whole_blocks(s))
        .map(|(name, _)| name.clone())
        .collect()
}

fn q8_names<V>(table: &std::collections::HashMap<String, V>, spec: &ModelSpec, storage: Storage) -> BTreeSet<String> {
    table.keys().filter(|v| storage.q8(spec, v)).cloned().collect()
}

fn lower_certified(
    src: &str,
    target: &SpatialTarget,
    program: &str,
    dense: &BTreeSet<String>,
    tp: Option<&TpManifest>,
    q8: &BTreeSet<String>,
) -> Result<SpatialProgram, StreamedReject> {
    lower_certified_carrying(src, target, program, dense, tp, q8).map(|(p, _)| p)
}

/// [`lower_certified`], and the state the program carries from one step to the next.
fn lower_certified_carrying(
    src: &str,
    target: &SpatialTarget,
    program: &str,
    dense: &BTreeSet<String>,
    tp: Option<&TpManifest>,
    q8: &BTreeSet<String>,
) -> Result<(SpatialProgram, BTreeSet<String>), StreamedReject> {
    let reject = |why: String| StreamedReject {
        program: program.to_string(),
        why,
    };
    let lowering = match tp {
        Some(m) => compile_program_to_spatial_tp_agreeing_stored(src, target, m, dense, q8),
        None => compile_program_to_spatial_agreeing_stored(src, target, dense, q8),
    }
    .map_err(|e| reject(format!("{e:?}")))?;
    let carried = lowering.carried.iter().cloned().collect();
    let p = lowering.into_whole().map_err(reject)?;
    if p.certificate.is_none() {
        return Err(reject("the lowering carries no certificate".to_string()));
    }
    Ok((p, carried))
}

/// Lower `spec` as one program per layer kind plus the head, and bind every layer to its kind.
///
/// `fill` turns a generated program into the source that is actually lowered (the weights' fills, when a
/// caller has them); layer programs are lowered through it so a kind program carries the fills of its
/// representative layer.
pub fn lower_forward_streamed(
    spec: &ModelSpec,
    storage: Storage,
    target: &SpatialTarget,
    fill: impl Fn(&str) -> String,
) -> Result<StreamedModel, StreamedReject> {
    let ((head, _, head_q8), head_id) = written_in(spec, Cut::ForwardHead, "/w");
    lower_streamed(
        spec,
        target,
        &fill,
        None,
        &|l| {
            let ((src, _, q8), id) = written_in(spec, Cut::ForwardLayer { layer: l }, "/w");
            (src, None, q8_names(&q8, spec, storage), id)
        },
        (&head, &q8_names(&head_q8, spec, storage), head_id),
        layer_input,
    )
}

/// Lower `spec` as an SN50 DECODE deployment: the embedding lookup, one program per layer kind in which every
/// attention layer appends to and attends over its KV cache of `ctx` positions, and a head that emits the next
/// token. `tok` and `pos` are runtime inputs — the programs read them as data — so the bundle serves every step;
/// the values the programs are lowered with (token 1, position 1) only seed their declarations.
pub fn lower_decode_streamed(
    spec: &ModelSpec,
    storage: Storage,
    ctx: usize,
    target: &SpatialTarget,
    fill: impl Fn(&str) -> String,
) -> Result<StreamedModel, StreamedReject> {
    if ctx == 0 {
        return Err(StreamedReject {
            program: "bundle".into(),
            why: "a decode bundle with an empty KV cache".into(),
        });
    }
    let ((embed, _, _), embed_id) = written_in(spec, Cut::DecodeEmbed, "/w");
    let ((head, _, head_q8), head_id) = written_in(spec, Cut::DecodeHead, "/w");
    lower_streamed(
        spec,
        target,
        &fill,
        Some((&embed, embed_id)),
        &|l| {
            let ((src, _, q8), id) = written_in(spec, Cut::DecodeLayer { layer: l, ctx }, "/w");
            (src, None, q8_names(&q8, spec, storage), id)
        },
        (&head, &q8_names(&head_q8, spec, storage), head_id),
        layer_input,
    )
}

/// Lower `spec` as an SN50 TENSOR-PARALLEL decode deployment over `tp` chips: the embedding lookup and the head on
/// the replicated spine, and each layer kind one program over the chips — every chip its own heads and FFN columns,
/// the partials summed by an all-reduce twice a layer, the chips' slices of each op in one phase (see
/// `spatial::tensor_parallel`). `tp` must be a width the model admits (`fit::admits_tp`); each kind program
/// carries its manifest's slices, so a host knows which rows or columns of which checkpoint tensor each chip loads.
pub fn lower_tp_decode_streamed(
    spec: &ModelSpec,
    storage: Storage,
    ctx: usize,
    tp: usize,
    target: &SpatialTarget,
    fill: impl Fn(&str) -> String,
) -> Result<StreamedModel, StreamedReject> {
    if ctx == 0 {
        return Err(StreamedReject {
            program: "bundle".into(),
            why: "a decode bundle with an empty KV cache".into(),
        });
    }
    if let Err(r) = super::fit::admits_tp(spec, tp) {
        return Err(StreamedReject {
            program: "bundle".into(),
            why: format!("tensor parallel over {tp} chips: {}", r.why),
        });
    }
    let ((embed, _, _), embed_id) = written_in(spec, Cut::DecodeEmbed, "/w");
    let ((head, _, head_q8), head_id) = written_in(spec, Cut::DecodeHead, "/w");
    lower_streamed(
        spec,
        target,
        &fill,
        Some((&embed, embed_id)),
        &|l| {
            let ((src, _, q8w, m), id) = split(spec, tp, written_split(spec, Cut::DecodeLayer { layer: l, ctx }, "/w", tp));
            let q8 = q8_slices(&m, &q8w, spec, storage);
            (src, Some(m), q8, id)
        },
        (&head, &q8_names(&head_q8, spec, storage), head_id),
        layer_input,
    )
}

/// Lower `spec` as an SN50 BATCHED decode deployment: `batch` sequences per step, each with its own token, position
/// and carried state, every weight read once for all of them (the model's [`Cut::Batch`]). The same three kinds of
/// program as [`lower_decode_streamed`] — the embedding lookups, one program per layer kind, the head — over
/// the stacked `batch × d` hidden states; the token ids `toks` and each sequence's `pos_s{b}` are runtime inputs.
pub fn lower_batched_decode_streamed(
    spec: &ModelSpec,
    storage: Storage,
    ctx: usize,
    batch: usize,
    target: &SpatialTarget,
    fill: impl Fn(&str) -> String,
) -> Result<StreamedModel, StreamedReject> {
    if ctx == 0 {
        return Err(StreamedReject {
            program: "bundle".into(),
            why: "a decode bundle with an empty KV cache".into(),
        });
    }
    if batch == 0 {
        return Err(StreamedReject {
            program: "bundle".into(),
            why: "a batched decode bundle serving no sequence".into(),
        });
    }
    let ((embed, _, _), embed_id) = written_in(spec, Cut::BatchEmbed { rows: batch }, "/w");
    let ((head, _, head_q8), head_id) = written_in(spec, Cut::BatchHead { rows: batch }, "/w");
    lower_streamed(
        spec,
        target,
        &fill,
        Some((&embed, embed_id)),
        &|l| {
            let ((src, _, q8), id) = written_in(spec, Cut::BatchLayer { layer: l, rows: batch, ctx }, "/w");
            (src, None, q8_names(&q8, spec, storage), id)
        },
        (&head, &q8_names(&head_q8, spec, storage), head_id),
        stacked_layer_input,
    )
}

/// Lower `spec`'s PREFILL as an SN50 deployment: `chunk` tokens of one sequence per step, the cuts of the model's
/// [`Cut::Prefill`] — the chunk's embedding lookups stacked, one program per layer kind taking the rows in order
/// over the sequence's single state (the decode step's own, so a decode bundle continues from it), and a head reading the
/// last row. Each row's token and position are runtime inputs.
pub fn lower_prefill_streamed(
    spec: &ModelSpec,
    storage: Storage,
    ctx: usize,
    chunk: usize,
    target: &SpatialTarget,
    fill: impl Fn(&str) -> String,
) -> Result<StreamedModel, StreamedReject> {
    if ctx == 0 {
        return Err(StreamedReject {
            program: "bundle".into(),
            why: "a prefill bundle with an empty KV cache".into(),
        });
    }
    if chunk == 0 || chunk > ctx {
        return Err(StreamedReject {
            program: "bundle".into(),
            why: format!("a prefill chunk of {chunk} tokens in a context of {ctx}"),
        });
    }
    let ((embed, _, _), embed_id) = written_in(spec, Cut::BatchEmbed { rows: chunk }, "/w");
    let ((head, _, head_q8), head_id) = written_in(spec, Cut::PrefillHead { rows: chunk }, "/w");
    lower_streamed(
        spec,
        target,
        &fill,
        Some((&embed, embed_id)),
        &|l| {
            let ((src, _, q8), id) = written_in(spec, Cut::PrefillLayer { layer: l, rows: chunk, ctx }, "/w");
            (src, None, q8_names(&q8, spec, storage), id)
        },
        (&head, &q8_names(&head_q8, spec, storage), head_id),
        stacked_layer_input,
    )
}

/// Lower `spec` as an SN50 BATCHED, TENSOR-PARALLEL decode deployment — `batch` sequences per step over `tp` chips,
/// the serving shape: [`lower_batched_decode_streamed`]'s stacked programs with every layer kind
/// [`lower_tp_decode_streamed`]'s program over the chips.
pub fn lower_batched_tp_decode_streamed(
    spec: &ModelSpec,
    storage: Storage,
    ctx: usize,
    batch: usize,
    tp: usize,
    target: &SpatialTarget,
    fill: impl Fn(&str) -> String,
) -> Result<StreamedModel, StreamedReject> {
    if ctx == 0 {
        return Err(StreamedReject {
            program: "bundle".into(),
            why: "a decode bundle with an empty KV cache".into(),
        });
    }
    if batch == 0 {
        return Err(StreamedReject {
            program: "bundle".into(),
            why: "a batched decode bundle serving no sequence".into(),
        });
    }
    if let Err(r) = super::fit::admits_tp(spec, tp) {
        return Err(StreamedReject {
            program: "bundle".into(),
            why: format!("tensor parallel over {tp} chips: {}", r.why),
        });
    }
    let ((embed, _, _), embed_id) = written_in(spec, Cut::BatchEmbed { rows: batch }, "/w");
    let ((head, _, head_q8), head_id) = written_in(spec, Cut::BatchHead { rows: batch }, "/w");
    lower_streamed(
        spec,
        target,
        &fill,
        Some((&embed, embed_id)),
        &|l| {
            let ((src, _, q8w, m), id) =
                split(spec, tp, written_split(spec, Cut::BatchLayer { layer: l, rows: batch, ctx }, "/w", tp));
            let q8 = q8_slices(&m, &q8w, spec, storage);
            (src, Some(m), q8, id)
        },
        (&head, &q8_names(&head_q8, spec, storage), head_id),
        stacked_layer_input,
    )
}

/// A layer of `spec` split over `tp` chips, a width the bundle's request was already checked to admit.
fn split(spec: &ModelSpec, tp: usize, form: Result<(TpAuthored, Identity), crate::model::author::NotYet>) -> (TpAuthored, Identity) {
    form.unwrap_or_else(|e| panic!("{} does not split over {tp} chips: {e}", spec.arch))
}

fn lower_streamed(
    spec: &ModelSpec,
    target: &SpatialTarget,
    fill: &dyn Fn(&str) -> String,
    embed: Option<(&str, Identity)>,
    layer: &dyn Fn(usize) -> Generated,
    (head, head_q8, head_identity): (&str, &BTreeSet<String>, Identity),
    input: fn(usize) -> String,
) -> Result<StreamedModel, StreamedReject> {
    let embed_identity = embed.map(|(_, id)| id);
    let embed = embed.map(|(src, _)| src);
    // ★ THE PROGRAMS AGREE ON EVERY TABLE THEY SHARE. The embedding program writes the first hidden state and
    // never reads it, so on its own it cannot know that layer 0 reads that state as a matrix — a stacked batch
    // is dense to the norm and the projections that read it, and would be flat to a writer that saw no reader.
    // The deployment's dense tables are therefore decided over every program: the embedding's, the head's, and
    // each layer kind's, carried to every layer of the kind under that layer's names.
    let tables = |src: &str, program: String| {
        dense_tables_of(src, target).map_err(|e| StreamedReject {
            program,
            why: format!("{e:?}"),
        })
    };
    let mut dense: BTreeSet<String> = BTreeSet::new();
    if let Some(src) = embed {
        dense.extend(tables(src, "embed".into())?);
    }
    dense.extend(tables(head, "head".into())?);
    let generated: Vec<Generated> = (0..spec.layers.len()).map(layer).collect();
    let kind_layers = layer_kinds(spec, &|l| (generated[l].0.clone(), generated[l].2.clone()));
    // Each layer's names, from its kind's representative's: the renaming between their programs, and for a name the
    // lowering derives from a source name (`xn0__xsq`), the derivation of the renamed one.
    let renames: Vec<std::collections::BTreeMap<String, String>> = (0..spec.layers.len())
        .map(|l| {
            let rep = kind_layers.iter().find(|(_, m)| m.contains(&l)).expect("every layer is of a kind").1[0];
            renaming(&generated[rep].0, &generated[l].0).expect("layers of one kind are one program up to their names")
        })
        .collect();
    let name_in = |l: usize, rep: usize, name: &str| -> String {
        if let Some(to) = renames[l].get(name) {
            return to.clone();
        }
        match name.split_once("__") {
            Some((stem, derived)) if renames[l].contains_key(stem) => format!("{}__{derived}", renames[l][stem]),
            _ => rebind_via(name, rep, l, input),
        }
    };
    for (_, members) in &kind_layers {
        let rep = members[0];
        let kind = tables(&generated[rep].0, format!("layer {rep}"))?;
        for &l in members {
            dense.extend(kind.iter().map(|n| name_in(l, rep, n)));
        }
    }
    let none = BTreeSet::new();
    let embed = embed
        .map(|src| lower_certified(&fill(src), target, "embed", &dense, None, &none))
        .transpose()?;
    let mut kinds: Vec<LayerKind> = Vec::new();
    let mut layers = Vec::with_capacity(spec.layers.len());
    for l in 0..spec.layers.len() {
        let name = &kind_layers
            .iter()
            .find(|(_, members)| members.contains(&l))
            .expect("every layer is of a kind")
            .0;
        let kind = match kinds.iter().position(|k| &k.name == name) {
            Some(k) => k,
            None => {
                let (src, tp, q8, identity) = generated[l].clone();
                let (program, carried) = lower_certified_carrying(
                    &fill(&src),
                    target,
                    &format!("layer {l} ({name})"),
                    &dense,
                    tp.as_ref(),
                    &q8,
                )?;
                kinds.push(LayerKind {
                    name: name.clone(),
                    representative: l,
                    identity,
                    program,
                    tp,
                    carried,
                });
                kinds.len() - 1
            }
        };
        let rep = kinds[kind].representative;
        let tensors = kinds[kind]
            .program
            .tensors
            .iter()
            .map(|t| (t.name.clone(), name_in(l, rep, &t.name)))
            .collect();
        layers.push(LayerBinding {
            layer: l,
            kind,
            tensors,
            identity: generated[l].3,
        });
    }
    let mut head = lower_certified(&fill(head), target, "head", &dense, None, head_q8)?;
    let mut embed = embed;
    let instances: Vec<usize> = (0..kinds.len())
        .map(|k| layers.iter().filter(|l| l.kind == k).count())
        .collect();
    let residency = {
        let mut programs: Vec<super::residency::DeployedProgram> = Vec::new();
        if let Some(e) = embed.as_mut() {
            programs.push(("embed", 1, e, None));
        }
        for (k, n) in kinds.iter_mut().zip(&instances) {
            let LayerKind { name, program, carried, .. } = k;
            programs.push((name.as_str(), *n, program, Some(carried)));
        }
        programs.push(("head", 1, &mut head, None));
        super::residency::plan_deployment(&mut programs, target)
    };
    Ok(StreamedModel {
        embed,
        embed_identity,
        kinds,
        head,
        head_identity,
        layers,
        residency,
    })
}
