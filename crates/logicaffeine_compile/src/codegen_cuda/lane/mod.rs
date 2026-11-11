//! The compiled generation lane: a model's decode step, written in LOGOS from its [`ModelSpec`] and compiled to one
//! CUDA program that loads the checkpoint, runs the prompt and generates.
//!
//! The path is the compiler's own, end to end, and nothing on it is written for one model:
//!
//!   1. [`author::decode_step`] writes the step as a LOGOS program — the token and position in, every layer's KV
//!      cache, convolution ring and delta-rule memory carried, the next token out — a cut of the model's one program
//!      ([`program`]), whose identity the emitted program prints above the text it compiled;
//!   2. the shared tensor front end lifts it into a [`StepGraph`](graph::StepGraph): every statement an op, or the
//!      compile refused by name;
//!   3. [`weights`] places every constant — quantized projections as rows read in place from the checkpoint, the
//!      rest computed once into a sidecar;
//!   4. [`plan`] gives each op a kernel from the [`kernels`] library, applies the fusion rules and lays out the
//!      buffers;
//!   5. [`arena`] places every array a step computes and reads within the step in one arena, by its lifetime;
//!   6. [`emit`] prints the program, each step captured as one CUDA graph and replayed per token or per chunk.
//!
//! A lane has several forms of the step, compiled the same way: the decode step, and [`author::prefill`]'s chunks of
//! 2, 4, … rows — the same layers over a block of prompt tokens, every projection reading its weights once for the
//! block. They share the model's constants and its carried state; the prompt runs as the largest chunks that fit.
//!
//! The lane's knobs are [`CudaKnobs`]': `activations` picks the projections' numerics — `f32` activations with `f64`
//! sums, a two-word ~14-bit int8 activation (the default), or llama.cpp's one int8 word — `cuda_graphs` captures each
//! step, `fuse_quant` quantizes a projection's activation in its producer.

pub mod arena;
pub mod branches;
pub mod emit;
pub mod graph;
pub mod kernels;
pub mod plan;
pub mod residence;
pub mod weights;

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use crate::codegen_cuda::knobs::CudaKnobs;
use crate::model::author::{self, NotYet};
use crate::model::program::{self, Cut, Identity};
use crate::model::gguf::Gguf;
use crate::model::spec::{Mixer, ModelSpec};
use crate::spatial::passes::recognize_stages::recognize_stages;

use emit::{Checkpoint, Program};
use graph::{GraphError, Role, StepGraph};
use plan::{Schedule, SeqIo, StepIo};

/// Why a model's step does not compile to the lane, by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaneError {
    /// The language cannot write the model yet.
    NotYet(NotYet),
    /// The authored step does not parse.
    Parse(String),
    /// The front end does not lift the step whole.
    Graph(GraphError),
    /// A constant the step declares that nothing binds.
    Unbound(String),
    /// A constant whose bound values are not the size the step declares.
    Shape { var: String, declared: usize, bound: usize },
    /// A checkpoint tensor in a format the lane reads no way.
    Format { tensor: String, ty: &'static str, why: &'static str },
    /// An op the library has no kernel for, in the form the step writes it.
    NoKernel { op: usize, kind: &'static str, out: String, why: String },
    /// A prefill chunk that is not a power of two up to [`plan::MAX_CHUNK`] rows and the caches' positions.
    Chunk { rows: usize, ctx: usize },
    /// A batch of fewer than two sequences: the decode step is the batch of one.
    Batch { batch: usize },
    /// The checkpoint could not be read.
    Io(String),
    /// Arrays the device must hold — its caches, arena and buffers, every matrix moved to host memory — past its memory.
    Memory { needed: usize, budget: usize },
}

impl fmt::Display for LaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LaneError::NotYet(n) => write!(f, "{n}"),
            LaneError::Parse(e) => write!(f, "the authored step does not parse: {e}"),
            LaneError::Graph(e) => write!(f, "{e}"),
            LaneError::Unbound(v) => write!(f, "nothing binds `{v}`"),
            LaneError::Shape { var, declared, bound } => {
                write!(f, "`{var}` is declared with {declared} values and bound to {bound}")
            }
            LaneError::Format { tensor, ty, why } => write!(f, "`{tensor}` is {ty}: {why}"),
            LaneError::NoKernel { op, kind, out, why } => write!(f, "op {op} ({kind} into `{out}`) has no kernel: {why}"),
            LaneError::Chunk { rows, ctx } => write!(
                f,
                "a prefill chunk of {rows} rows: a chunk is a power of two up to {} rows and the caches' {ctx} positions",
                plan::MAX_CHUNK
            ),
            LaneError::Batch { batch } => write!(f, "a batch of {batch}: a batch step decodes two sequences or more"),
            LaneError::Io(e) => write!(f, "the checkpoint: {e}"),
            LaneError::Memory { needed, budget } => {
                write!(f, "the caches, arena and buffers take {needed} bytes of the device's {budget}, every matrix moved to the host")
            }
        }
    }
}

impl std::error::Error for LaneError {}

impl From<GraphError> for LaneError {
    fn from(e: GraphError) -> Self {
        LaneError::Graph(e)
    }
}

/// How a lane is compiled: its caches' length in positions, its knobs, and its largest prefill chunk (a
/// power of two up to [`plan::MAX_CHUNK`]; 1 runs the prompt a decode step at a time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneOptions {
    pub ctx: usize,
    pub knobs: CudaKnobs,
    pub prefill: usize,
}

/// What a form of a lane computes: the decode step, a prefill chunk of one sequence's tokens, or a batch's step of one
/// token for each of its sequences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormKind {
    Decode,
    Prefill,
    Batch,
}

/// One form of a lane: the step of `rows` rows (1 for the decode step; a batch's, one a sequence), its LOGOS source, its
/// graph, its schedule and where its step-local arrays live in the program's arena.
#[derive(Clone, Debug)]
pub struct Form {
    pub rows: usize,
    pub kind: FormKind,
    /// The cut of the model's one program this form compiles, and the digest of its text.
    pub identity: Identity,
    pub source: String,
    pub graph: StepGraph,
    pub schedule: Schedule,
    pub arena: arena::Arena,
}

/// A compiled lane: its forms, the decode step first, and the one program that runs them.
#[derive(Clone, Debug)]
pub struct Lane {
    pub forms: Vec<Form>,
    pub program: Program,
    /// What the program allocates, and the order its matrices leave the device in when the memory it finds runs short.
    pub plan: residence::Plan,
    /// Where the program's matrices live on the chip the lane is compiled for (`CudaChip::mem_bytes`, less what the
    /// driver, the context and the program's graphs take): the placement the program makes when it finds that memory.
    pub residence: residence::Residence,
}

impl Form {
    /// The name the program knows the form's step by: its rows, a batch's prefixed `b` (a batch of four and the chunk of
    /// four are different steps).
    pub fn tag(&self) -> String {
        match self.kind {
            FormKind::Batch => format!("b{}", self.rows),
            FormKind::Decode | FormKind::Prefill => self.rows.to_string(),
        }
    }
}

impl Lane {
    /// The decode step.
    pub fn decode(&self) -> &Form {
        &self.forms[0]
    }

    /// The prefill chunk of `rows` rows.
    pub fn chunk(&self, rows: usize) -> Option<&Form> {
        self.forms.iter().find(|f| f.kind == FormKind::Prefill && f.rows == rows)
    }

    /// The batch step, where the lane was compiled with one ([`compile_batch`]).
    pub fn batch(&self) -> Option<&Form> {
        self.forms.iter().find(|f| f.kind == FormKind::Batch)
    }
}

/// The names the authored decode step reads and writes across steps.
pub fn decode_io() -> StepIo {
    StepIo { token: "tok".into(), position: "pos".into(), logits: "logits".into(), next: "next".into(), more: Vec::new(), logits_read: true }
}

/// The names an authored prefill chunk reads and writes across steps.
pub fn prefill_io() -> StepIo {
    StepIo { token: "toks".into(), position: "pos_s0".into(), logits: "logits".into(), next: "next".into(), more: Vec::new(), logits_read: true }
}

/// The names an authored batch of `rows` sequences reads and writes across steps: its tokens, one a sequence, and
/// sequence `b`'s position `pos_s<b>`, logits `logits_s<b>` and next token `next_s<b>`.
pub fn batch_io(rows: usize) -> StepIo {
    StepIo {
        token: "toks".into(),
        position: "pos_s0".into(),
        logits: "logits_s0".into(),
        next: "next_s0".into(),
        more: (1..rows).map(|b| SeqIo { position: format!("pos_s{b}"), logits: format!("logits_s{b}"), next: format!("next_s{b}") }).collect(),
        logits_read: true,
    }
}

/// Lower one authored form: lift it, place its constants, plan it; its arena takes the states `own` names, the ones no
/// other form carries.
#[allow(clippy::too_many_arguments)]
fn form(
    spec: &ModelSpec,
    g: &mut Gguf,
    rows: usize,
    kind: FormKind,
    cut: Cut,
    opts: &LaneOptions,
    io: StepIo,
    held: &[&str],
    own: &dyn Fn(&str) -> bool,
) -> Result<Form, LaneError> {
    let ((source, _, q8), identity) = program::written(spec, cut);
    let rp = recognize_stages(&source).map_err(|e| LaneError::Parse(format!("{e:?}")))?;
    let graph = StepGraph::build(&rp)?;
    let mut constants = BTreeMap::new();
    for t in graph.of_role(Role::Constant) {
        let placed = weights::place(spec, g, &t.name, t.len(), q8.contains_key(&t.name), opts.ctx)?;
        constants.insert(t.name.clone(), placed);
    }
    let schedule = plan::plan(&graph, &|s| rp.name(s).to_string(), constants, opts.knobs.activations, &opts.knobs, io, held)?;
    let arena = arena::place(&schedule, own);
    Ok(Form { rows, kind, identity, source, graph, schedule, arena })
}

/// Lower the decode step of `spec` split over `tp` chips ([`program::written_split`]) as one device runs it: every chip's
/// slice of a weight placed as its own constant, every chip's caches its own arrays, every all-reduce the left fold of
/// its chips' partials the step spells — each partial read where it lives.
fn split_form(spec: &ModelSpec, g: &mut Gguf, tp: usize, opts: &LaneOptions, held: &[&str]) -> Result<Form, LaneError> {
    let ((source, _, q8, manifest), identity) = program::written_split(spec, Cut::Decode { ctx: opts.ctx }, "", tp).map_err(LaneError::NotYet)?;
    let rp = recognize_stages(&source).map_err(|e| LaneError::Parse(format!("{e:?}")))?;
    let graph = StepGraph::build(&rp)?;
    let mut constants = BTreeMap::new();
    for t in graph.of_role(Role::Constant) {
        // A slice is a quantized matrix when the matrix it cuts is one: the author declares the whole matrix.
        let placed = match manifest.slices.get(&t.name) {
            Some(slice) => weights::place_slice(spec, g, &t.name, t.len(), slice, q8.contains_key(&slice.of))?,
            None => weights::place(spec, g, &t.name, t.len(), q8.contains_key(&t.name), opts.ctx)?,
        };
        constants.insert(t.name.clone(), placed);
    }
    let schedule = plan::plan(&graph, &|s| rp.name(s).to_string(), constants, opts.knobs.activations, &opts.knobs, decode_io(), held)?;
    // The decode step's states are every form's.
    let arena = arena::place(&schedule, &|_| false);
    Ok(Form { rows: 1, kind: FormKind::Decode, identity, source, graph, schedule, arena })
}

/// Compile the lane of `spec`'s decode step split over `tp` chips (tensor parallelism) as one device runs it — the chips'
/// slices, caches and partials each their own arrays, each all-reduce read across them — against the checkpoint at
/// `checkpoint`; the prompt runs a decode step at a time (`opts.prefill` 1).
pub fn compile_split(spec: &ModelSpec, checkpoint: &Path, opts: &LaneOptions, tp: usize) -> Result<Lane, LaneError> {
    compile_split_holding(spec, checkpoint, opts, tp, &[])
}

/// [`compile_split`] with the fusion rules `held` held back.
pub fn compile_split_holding(spec: &ModelSpec, checkpoint: &Path, opts: &LaneOptions, tp: usize, held: &[&str]) -> Result<Lane, LaneError> {
    author::check_tp(spec, tp).map_err(LaneError::NotYet)?;
    lane_check(spec).map_err(LaneError::NotYet)?;
    if opts.prefill != 1 {
        return Err(LaneError::Chunk { rows: opts.prefill, ctx: opts.ctx });
    }
    let io = |e: std::io::Error| LaneError::Io(e.to_string());
    let mut g = Gguf::open(checkpoint).map_err(io)?;
    let forms = vec![split_form(spec, &mut g, tp, opts, held)?];
    finish(forms, checkpoint, &mut g, opts)
}

/// Compile the lane of `spec` against the checkpoint at `checkpoint`: its decode step and its prefill chunks.
pub fn compile(spec: &ModelSpec, checkpoint: &Path, opts: &LaneOptions) -> Result<Lane, LaneError> {
    compile_holding(spec, checkpoint, opts, &[])
}

/// [`compile`] with the fusion rules `held` (of [`plan::RULES`]) held back, their launches kept apart: the program each
/// fusion is checked against, computing what the fused one computes.
pub fn compile_holding(spec: &ModelSpec, checkpoint: &Path, opts: &LaneOptions, held: &[&str]) -> Result<Lane, LaneError> {
    compile_forms(spec, checkpoint, opts, 1, held)
}

/// [`compile`] with a batch step beside the decode step and the chunks: the model's batch cut (`Cut::Batch`) of `batch`
/// sequences, one token each a step, every weight read once for all of them, each sequence its own position, caches and
/// next token. The program decodes up to `batch` prompts together, each prefilled into its own sequence's state.
pub fn compile_batch(spec: &ModelSpec, checkpoint: &Path, opts: &LaneOptions, batch: usize) -> Result<Lane, LaneError> {
    compile_batch_holding(spec, checkpoint, opts, batch, &[])
}

/// [`compile_batch`] with the fusion rules `held` held back.
pub fn compile_batch_holding(spec: &ModelSpec, checkpoint: &Path, opts: &LaneOptions, batch: usize, held: &[&str]) -> Result<Lane, LaneError> {
    if batch < 2 {
        return Err(LaneError::Batch { batch });
    }
    compile_forms(spec, checkpoint, opts, batch, held)
}

/// What the author writes that the lane's kernels do not take yet: one fused query, key and value projection, and
/// values unlike the keys in width.
fn lane_check(spec: &ModelSpec) -> Result<(), NotYet> {
    for layer in &spec.layers {
        if let Mixer::Attention(a) = &layer.mixer {
            if a.fused_qkv {
                return Err(NotYet("one fused query, key and value projection on the CUDA lane"));
            }
            if a.value_dim != a.head_dim {
                return Err(NotYet("values unlike the keys in width on the CUDA lane"));
            }
        }
    }
    Ok(())
}

fn compile_forms(spec: &ModelSpec, checkpoint: &Path, opts: &LaneOptions, batch: usize, held: &[&str]) -> Result<Lane, LaneError> {
    author::check(spec).map_err(LaneError::NotYet)?;
    lane_check(spec).map_err(LaneError::NotYet)?;
    if !opts.prefill.is_power_of_two() || opts.prefill > plan::MAX_CHUNK || opts.prefill > opts.ctx {
        return Err(LaneError::Chunk { rows: opts.prefill, ctx: opts.ctx });
    }
    let io = |e: std::io::Error| LaneError::Io(e.to_string());
    let mut g = Gguf::open(checkpoint).map_err(io)?;
    // The decode step's states are every form's (`residence::shared_names`); a state only another form holds is that
    // form's own.
    let mut forms = vec![form(spec, &mut g, 1, FormKind::Decode, Cut::Decode { ctx: opts.ctx }, opts, decode_io(), held, &|_| false)?];
    let carried: std::collections::BTreeSet<String> = forms[0].schedule.buffers.keys().cloned().collect();
    let own = |name: &str| !carried.contains(name);
    let mut rows = opts.prefill;
    while rows > 1 {
        forms.push(form(spec, &mut g, rows, FormKind::Prefill, Cut::Prefill { rows, ctx: opts.ctx }, opts, prefill_io(), held, &own)?);
        rows /= 2;
    }
    if batch > 1 {
        forms.push(form(spec, &mut g, batch, FormKind::Batch, Cut::Batch { rows: batch, ctx: opts.ctx }, opts, batch_io(batch), held, &own)?);
    }
    finish(forms, checkpoint, &mut g, opts)
}

/// The lane of `forms` against the checkpoint at `checkpoint` (opened as `g`): its matrices placed, its program printed.
fn finish(forms: Vec<Form>, checkpoint: &Path, g: &mut Gguf, opts: &LaneOptions) -> Result<Lane, LaneError> {
    let io = |e: std::io::Error| LaneError::Io(e.to_string());
    let ckpt = Checkpoint {
        size: std::fs::metadata(checkpoint).map_err(io)?.len(),
        data_start: g.data_start(),
        digest: g.header_digest().map_err(io)?,
    };
    let mut plan = residence::plan(&forms, opts.ctx, emit::captured_nodes(&forms, &opts.knobs));
    plan.copies = emit::captured_copies(&forms, &plan);
    let residence = residence::place(&plan, opts.knobs.chip.mem_bytes.saturating_sub(residence::CONTEXT_RESERVE + plan.spare))?;
    let program = emit::emit(&forms, &opts.knobs, opts.ctx, ckpt, &plan)?;
    Ok(Lane { forms, program, plan, residence })
}
