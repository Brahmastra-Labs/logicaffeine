//! The front end of the one IR: the **whole-program walk** — every tensor statement of a program lifted, in order,
//! into a typed spec over the shared recognizers, with its dataflow and its carried state named. Every tensor backend
//! enters here (the CUDA lane, the older forward, the spatial pipeline, the D5005 engine); it is also reached by its
//! older path, `spatial::passes::recognize_stages`.
//!
//! `recognize_any` in [`crate::spatial::pipeline`] lifts ONE op out of a source; an authored forward is a hundred.
//! This walk reads the program's top-level statements one at a time (each tensor sugar desugars to one
//! `Stmt::Splice`, so a statement is an op) and tries the recognizers in [`RECOGNIZER_ORDER`] — the CUDA
//! orchestrator's order, composite blocks first so their inner loops are never claimed by a simpler shape.
//! What it cannot lift it RETURNS: `unrecognized` lists the statement by index and binder, so a composition
//! refuses by name and a ledger can print the gap. Nothing is skipped silently.
//!
//! The result names everything by variable: [`links`] is the dataflow (the producer stage of every tensor
//! and the stages that read it) and [`state_tensors`] the carried state (what a step reads AND writes in
//! place — the conv rings, the GDN memories; the KV cache joins through the masked attention form). These
//! are the inputs the section composer takes; the lowering of each spec is the composer's other half.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use logicaffeine_language::ast::stmt::{Expr, Literal, Stmt};

use crate::spatial::passes::SpatialReject;
use crate::intern::{Interner, Symbol};
use crate::tensor_recognize::qwen::{
    recognize_argmax, recognize_attention, recognize_causal_attention, recognize_causal_conv,
    recognize_decay, recognize_full_attention, recognize_gated_delta, recognize_gelu, recognize_head_slice,
    recognize_int_clamp, recognize_int_requant, recognize_join, recognize_l2norm, recognize_lut_attention, recognize_moe,
    recognize_relu, recognize_rope, recognize_row_gather, recognize_rows_gather, recognize_scale, recognize_set_row,
    recognize_sigmoid, recognize_silu, recognize_softcap, recognize_transpose, ArgMaxSpec, AttentionSpec, CausalConvSpec,
    DecaySpec,
    FullAttentionSpec, GatedDeltaSpec, GeluSpec, HeadSliceSpec, IntClampSpec, IntRequantSpec, JoinSpec, L2NormSpec,
    LutAttentionSpec, MoeSpec, ReluSpec, RopeSpec, RowGatherSpec, RowIndex, RowsGatherSpec, ScaleSpec, SetRowSpec,
    SigmoidSpec, SiluSpec, SoftcapSpec, TransposeSpec,
};
use crate::tensor_recognize::{
    recognize_elementwise, recognize_fused_row, recognize_head_matmul, recognize_matmul, Fills,
    recognize_swiglu, ElementwiseSpec, Fill, FusedRowSpec, HeadMatmulSpec, MatmulSpec, SwiGluSpec,
};

/// One recognized tensor op — the typed spec the shared recognizers lift. The variant set is the language's
/// tensor sugar; the lowering of each to a spatial template lives with the composer.
#[derive(Clone, Debug)]
pub enum TensorOpSpec {
    Matmul(MatmulSpec),
    Elementwise(ElementwiseSpec),
    SwiGlu(SwiGluSpec),
    FusedRow(FusedRowSpec),
    Rope(RopeSpec),
    Silu(SiluSpec),
    Sigmoid(SigmoidSpec),
    Gelu(GeluSpec),
    Scale(ScaleSpec),
    Softcap(SoftcapSpec),
    Decay(DecaySpec),
    L2Norm(L2NormSpec),
    Transpose(TransposeSpec),
    HeadSlice(HeadSliceSpec),
    Join(JoinSpec),
    HeadMatmul(HeadMatmulSpec),
    CausalConv(CausalConvSpec),
    GatedDelta(GatedDeltaSpec),
    Attention(AttentionSpec),
    FullAttention(FullAttentionSpec),
    Moe(MoeSpec),
    RowGather(RowGatherSpec),
    RowsGather(RowsGatherSpec),
    ArgMax(ArgMaxSpec),
    SetRow(SetRowSpec),
    IntRequant(IntRequantSpec),
    IntClamp(IntClampSpec),
    Relu(ReluSpec),
    LutAttention(LutAttentionSpec),
}

impl TensorOpSpec {
    /// The variant's name — the kind a ledger or a schedule reads.
    pub fn kind(&self) -> &'static str {
        match self {
            TensorOpSpec::Matmul(_) => "Matmul",
            TensorOpSpec::Elementwise(_) => "Elementwise",
            TensorOpSpec::SwiGlu(_) => "SwiGlu",
            TensorOpSpec::FusedRow(_) => "FusedRow",
            TensorOpSpec::Rope(_) => "Rope",
            TensorOpSpec::Silu(_) => "Silu",
            TensorOpSpec::Sigmoid(_) => "Sigmoid",
            TensorOpSpec::Gelu(_) => "Gelu",
            TensorOpSpec::Scale(_) => "Scale",
            TensorOpSpec::Softcap(_) => "Softcap",
            TensorOpSpec::Decay(_) => "Decay",
            TensorOpSpec::L2Norm(_) => "L2Norm",
            TensorOpSpec::Transpose(_) => "Transpose",
            TensorOpSpec::HeadSlice(_) => "HeadSlice",
            TensorOpSpec::Join(_) => "Join",
            TensorOpSpec::HeadMatmul(_) => "HeadMatmul",
            TensorOpSpec::CausalConv(_) => "CausalConv",
            TensorOpSpec::GatedDelta(_) => "GatedDelta",
            TensorOpSpec::Attention(_) => "Attention",
            TensorOpSpec::FullAttention(_) => "FullAttention",
            TensorOpSpec::Moe(_) => "Moe",
            TensorOpSpec::RowGather(_) => "RowGather",
            TensorOpSpec::RowsGather(_) => "RowsGather",
            TensorOpSpec::ArgMax(_) => "ArgMax",
            TensorOpSpec::SetRow(_) => "SetRow",
            TensorOpSpec::IntRequant(_) => "IntRequant",
            TensorOpSpec::IntClamp(_) => "IntClamp",
            TensorOpSpec::Relu(_) => "Relu",
            TensorOpSpec::LutAttention(_) => "LutAttention",
        }
    }

    /// The tensor the op binds.
    pub fn out(&self) -> Symbol {
        match self {
            TensorOpSpec::Matmul(m) => m.c,
            TensorOpSpec::Elementwise(e) => e.c,
            TensorOpSpec::SwiGlu(s) => s.y,
            TensorOpSpec::FusedRow(r) => r.y,
            TensorOpSpec::Rope(r) => r.out,
            TensorOpSpec::Silu(s) => s.out,
            TensorOpSpec::Sigmoid(s) => s.out,
            TensorOpSpec::Gelu(g) => g.out,
            TensorOpSpec::Scale(s) => s.out,
            TensorOpSpec::Softcap(s) => s.out,
            TensorOpSpec::Decay(d) => d.out,
            TensorOpSpec::L2Norm(l) => l.out,
            TensorOpSpec::Transpose(t) => t.out,
            TensorOpSpec::HeadSlice(s) => s.out,
            TensorOpSpec::Join(j) => j.out,
            TensorOpSpec::HeadMatmul(m) => m.out,
            TensorOpSpec::CausalConv(c) => c.out,
            TensorOpSpec::GatedDelta(g) => g.out,
            TensorOpSpec::Attention(a) => a.out,
            TensorOpSpec::FullAttention(a) => a.out,
            TensorOpSpec::Moe(m) => m.out,
            TensorOpSpec::RowGather(r) => r.out,
            TensorOpSpec::RowsGather(r) => r.out,
            TensorOpSpec::ArgMax(a) => a.out,
            TensorOpSpec::SetRow(r) => r.table,
            TensorOpSpec::IntRequant(r) => r.out,
            TensorOpSpec::IntClamp(c) => c.out,
            TensorOpSpec::Relu(r) => r.out,
            TensorOpSpec::LutAttention(a) => a.out,
        }
    }

    /// The tensors the op reads, in the spec's operand order: matmul `[a, b]`; elementwise `[a, b]`; swiglu
    /// `[gate, up]`; fused row `[x, weight?, residual?, bias?]` (the present ones); rope `[q, rc, rs, position?]`; the
    /// unary maps and a scale `[x]`; decay `[ar, dt, alog]`; l2norm / transpose / head slice `[x]`; join `[a, b]`;
    /// per-head matmul `[x, w]`; causal conv `[x, wc, cs]`; gated
    /// delta `[q, k, v, z, b, a, st, gn]`; attention `[q, k, v, position?, sinks?]` and full attention `[q, k, v]`; moe `[x, wr, wg, wu, wd, router bias?, gate, up and down biases?]`;
    /// row gather `[table, index?]`; rows gather `[table, indices]`; arg-max `[x]`; set row `[table, source, index?]` (a NAMED index is an
    /// operand — the data the row comes from; a literal one is part of the op).
    pub fn ins(&self) -> Vec<Symbol> {
        match self {
            TensorOpSpec::Matmul(m) => vec![m.a, m.b],
            TensorOpSpec::Elementwise(e) => vec![e.a, e.b],
            TensorOpSpec::SwiGlu(s) => vec![s.gate, s.up],
            TensorOpSpec::FusedRow(r) => std::iter::once(r.x)
                .chain(r.weight)
                .chain(r.residual)
                .chain(r.bias)
                .collect(),
            TensorOpSpec::Rope(r) => [r.q, r.rc, r.rs]
                .into_iter()
                .chain(r.from.and_then(|(p, _)| named(p)))
                .collect(),
            TensorOpSpec::Silu(s) => vec![s.x],
            TensorOpSpec::Sigmoid(s) => vec![s.x],
            TensorOpSpec::Gelu(g) => vec![g.x],
            TensorOpSpec::Scale(s) => vec![s.x],
            TensorOpSpec::Softcap(s) => vec![s.x],
            TensorOpSpec::Decay(d) => vec![d.ar, d.dt, d.alog],
            TensorOpSpec::L2Norm(l) => vec![l.x],
            TensorOpSpec::Transpose(t) => vec![t.x],
            TensorOpSpec::HeadSlice(s) => vec![s.x],
            TensorOpSpec::Join(j) => vec![j.a, j.b],
            TensorOpSpec::HeadMatmul(m) => vec![m.x, m.w],
            TensorOpSpec::CausalConv(c) => vec![c.x, c.wc, c.cs],
            TensorOpSpec::GatedDelta(g) => vec![g.q, g.k, g.v, g.z, g.b, g.a, g.st, g.gn],
            TensorOpSpec::Attention(a) => [a.q, a.kc, a.vc]
                .into_iter()
                .chain(a.pos.and_then(named))
                .chain(a.sinks)
                .collect(),
            TensorOpSpec::FullAttention(a) => vec![a.q, a.k, a.v],
            TensorOpSpec::Moe(m) => [m.x, m.wr, m.wg, m.wu, m.wd]
                .into_iter()
                .chain(m.router_bias)
                .chain(m.biases.into_iter().flatten())
                .chain(m.selection_bias)
                .collect(),
            TensorOpSpec::RowGather(r) => std::iter::once(r.table).chain(named(r.index)).collect(),
            TensorOpSpec::RowsGather(r) => vec![r.table, r.indices],
            TensorOpSpec::ArgMax(a) => vec![a.x],
            TensorOpSpec::SetRow(r) => [r.table, r.source]
                .into_iter()
                .chain(named(r.index))
                .collect(),
            TensorOpSpec::IntRequant(r) => vec![r.x],
            TensorOpSpec::IntClamp(c) => vec![c.x],
            TensorOpSpec::Relu(r) => vec![r.x],
            TensorOpSpec::LutAttention(a) => [a.q, a.kc, a.vc, a.table]
                .into_iter()
                .chain(a.pos.and_then(named))
                .collect(),
        }
    }

    /// The positions in [`Self::ins`] of the operands the op writes back in place — its carried state: the
    /// conv ring, the GDN memory, and the table a row is written into (the KV cache's append). An attention only
    /// READS its cache — masked or not — so it carries nothing; the cache is carried because `Set row` writes it.
    pub fn state_inputs(&self) -> Vec<usize> {
        match self {
            TensorOpSpec::CausalConv(_) => vec![2],
            TensorOpSpec::GatedDelta(_) => vec![6],
            TensorOpSpec::SetRow(_) => vec![0],
            _ => Vec::new(),
        }
    }

    /// The rows of a result the op writes a row at a time into a table it builds, or `None`: a block gated delta,
    /// whose rows each take the step in turn and write their output row. The table is state inside the step — each
    /// row's write updates it in place — which the step assembles whole.
    pub fn assembled_rows(&self) -> Option<usize> {
        match self {
            TensorOpSpec::GatedDelta(g) if g.rows > 1 => Some(g.rows),
            _ => None,
        }
    }
}

/// One lifted statement: the spec, where it sits in the top-level statement list, and its binder and operands
/// by name (in [`TensorOpSpec::ins`] order).
#[derive(Clone, Debug)]
pub struct RecognizedStage {
    pub op: TensorOpSpec,
    pub stmt_index: usize,
    pub out: String,
    pub ins: Vec<String>,
}

/// A tensor the program declares (`Let A be a R by C tensor of T`): its flat length and the fill the
/// program gives it (the declaration's zero fill plus every `Set item` override).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredTensor {
    pub name: String,
    pub len: usize,
    pub fill: Fill,
}

/// The walk's result: the stages in program order, the tensor statements it could not lift (by top-level
/// index and binder), the declared tensors, and the names of every symbol the specs mention.
#[derive(Clone, Debug, Default)]
pub struct RecognizedProgram {
    pub stages: Vec<RecognizedStage>,
    pub unrecognized: Vec<(usize, String)>,
    /// Top-level writes to a declared tensor that no section computes — `(statement, tensor, why)`. Each is a
    /// reason the program is NOT lowered: dropping it would leave the tensor holding values the program never had.
    pub unlifted_writes: Vec<(usize, String, &'static str)>,
    pub declared: Vec<DeclaredTensor>,
    pub symbols: HashMap<Symbol, String>,
    /// Every tensor the program binds, by name, as `(rows, cols)` — the parser's table; the desugared statements
    /// carry only flat lengths.
    pub shapes: HashMap<String, (usize, usize)>,
    /// Every name the program's top-level statements touch, by name, with the statements that touch it — a `Show`, a
    /// scalar read and a stage alike. What reads a value is here, so a pass can know a value nothing else reads.
    pub mentions: HashMap<String, std::collections::BTreeSet<usize>>,
    /// The top-level statements that run foreign code, which may touch any name.
    pub opaque: Vec<usize>,
    /// The top-level statements that only declare a tensor or a scalar, or give a declared tensor a literal value before
    /// any op reads it: the statements that make the program's inputs, which observe nothing.
    pub quiet: BTreeSet<usize>,
}

impl RecognizedProgram {
    /// The `(rows, cols)` the program gives tensor `name`, when that accounts for the `n` elements a recognizer
    /// counted — and the `1 × n` row otherwise.
    pub fn shape_of(&self, name: &str, n: usize) -> (usize, usize) {
        match self.shapes.get(name) {
            Some(&(r, c)) if r * c == n => (r, c),
            _ => (1, n),
        }
    }

    /// The name of a symbol one of this program's specs mentions.
    pub fn name(&self, s: Symbol) -> &str {
        self.symbols
            .get(&s)
            .map(String::as_str)
            .unwrap_or_else(|| panic!("{s:?} is not a symbol of this program's specs"))
    }
}

/// Which tensors of a walked program hold FLOAT data, propagated forward from the declarations, each name as its last
/// write leaves it.
///
/// ★ AN ACTIVATION HAS NO FILL TO READ. A backend decides between an integer and a float datapath from the declared
/// fill of its operands — right for a declared tensor, blind for one another op produces. The walk knows: a sugar that
/// computes an exponential or a normalization is float by construction, and everything else is float exactly when
/// something feeding it is.
pub fn float_tensors(rp: &RecognizedProgram) -> BTreeSet<String> {
    let mut float: BTreeSet<String> = rp
        .declared
        .iter()
        .filter(|d| matches!(d.fill, Fill::ConstF(_) | Fill::FloatOverrides { .. } | Fill::RealPattern { .. }))
        .map(|d| d.name.clone())
        .collect();
    for st in &rp.stages {
        let is_float = match st.op {
            // An index is an integer whatever it indexes.
            TensorOpSpec::ArgMax(_) => false,
            // A row moved out of — or into — a table has the TABLE's element kind; the index is not data here.
            TensorOpSpec::RowGather(_) | TensorOpSpec::RowsGather(_) | TensorOpSpec::SetRow(_) => float.contains(&st.ins[0]),
            TensorOpSpec::Matmul(_)
            | TensorOpSpec::Elementwise(_)
            | TensorOpSpec::Transpose(_)
            | TensorOpSpec::HeadSlice(_)
            | TensorOpSpec::Join(_)
            | TensorOpSpec::HeadMatmul(_) => st.ins.iter().any(|n| float.contains(n)),
            // An integer model's forms compute integers by construction; a ReLU keeps its operand's kind.
            TensorOpSpec::IntRequant(_) | TensorOpSpec::IntClamp(_) | TensorOpSpec::LutAttention(_) => false,
            TensorOpSpec::Relu(ref r) => !r.int,
            _ => true,
        };
        if is_float {
            float.insert(st.out.clone());
        } else {
            float.remove(&st.out);
        }
    }
    float
}

/// A tensor's producer stage and the stages that read it — the dataflow edge by variable name. A stage whose
/// output nothing reads (the program's result) has no consumers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub producer: usize,
    pub consumers: Vec<usize>,
    pub tensor: String,
}

/// The operand a row index contributes: a named scalar is data, a literal is part of the op.
fn named(index: RowIndex) -> Option<Symbol> {
    match index {
        RowIndex::Named(s) => Some(s),
        RowIndex::Literal(_) => None,
    }
}

type Recognizer = fn(&[Stmt], &Interner) -> Option<TensorOpSpec>;

fn r_moe(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_moe(s).map(TensorOpSpec::Moe)
}
fn r_attention(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_attention(s)
        .or_else(|| recognize_causal_attention(s))
        .map(TensorOpSpec::Attention)
}
fn r_lut_attention(s: &[Stmt], i: &Interner) -> Option<TensorOpSpec> {
    recognize_lut_attention(s, i).map(TensorOpSpec::LutAttention)
}
fn r_int_requant(s: &[Stmt], i: &Interner) -> Option<TensorOpSpec> {
    recognize_int_requant(s, i).map(TensorOpSpec::IntRequant)
}
fn r_int_clamp(s: &[Stmt], i: &Interner) -> Option<TensorOpSpec> {
    recognize_int_clamp(s, i).map(TensorOpSpec::IntClamp)
}
fn r_relu(s: &[Stmt], i: &Interner) -> Option<TensorOpSpec> {
    recognize_relu(s, i).map(TensorOpSpec::Relu)
}
fn r_full_attention(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_full_attention(s).map(TensorOpSpec::FullAttention)
}
fn r_row_gather(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_row_gather(s).map(TensorOpSpec::RowGather)
}
fn r_rows_gather(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_rows_gather(s).map(TensorOpSpec::RowsGather)
}
fn r_argmax(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_argmax(s).map(TensorOpSpec::ArgMax)
}
fn r_set_row(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_set_row(s).map(TensorOpSpec::SetRow)
}
fn r_matmul(s: &[Stmt], i: &Interner) -> Option<TensorOpSpec> {
    recognize_matmul(s, i).ok().map(TensorOpSpec::Matmul)
}
fn r_elementwise(s: &[Stmt], i: &Interner) -> Option<TensorOpSpec> {
    recognize_elementwise(s, i)
        .ok()
        .map(TensorOpSpec::Elementwise)
}
fn r_rope(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_rope(s).map(TensorOpSpec::Rope)
}
fn r_silu(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_silu(s).map(TensorOpSpec::Silu)
}
fn r_sigmoid(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_sigmoid(s).map(TensorOpSpec::Sigmoid)
}
fn r_gelu(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_gelu(s).map(TensorOpSpec::Gelu)
}
fn r_scale(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_scale(s).map(TensorOpSpec::Scale)
}
fn r_softcap(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_softcap(s).map(TensorOpSpec::Softcap)
}
fn r_decay(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_decay(s).map(TensorOpSpec::Decay)
}
fn r_causal_conv(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_causal_conv(s).map(TensorOpSpec::CausalConv)
}
fn r_gated_delta(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_gated_delta(s).map(TensorOpSpec::GatedDelta)
}
fn r_l2norm(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_l2norm(s).map(TensorOpSpec::L2Norm)
}
fn r_fused_row(s: &[Stmt], i: &Interner) -> Option<TensorOpSpec> {
    recognize_fused_row(s, i).ok().map(TensorOpSpec::FusedRow)
}
fn r_transpose(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_transpose(s).map(TensorOpSpec::Transpose)
}
fn r_head_slice(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_head_slice(s).map(TensorOpSpec::HeadSlice)
}
fn r_join(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_join(s).map(TensorOpSpec::Join)
}
fn r_head_matmul(s: &[Stmt], _: &Interner) -> Option<TensorOpSpec> {
    recognize_head_matmul(s).map(TensorOpSpec::HeadMatmul)
}
fn r_swiglu(s: &[Stmt], i: &Interner) -> Option<TensorOpSpec> {
    recognize_swiglu(s, i).ok().map(TensorOpSpec::SwiGlu)
}

/// The recognizers a statement is tried against, in order — the CUDA orchestrator's order, extended with the
/// exact-shape recognizers it has no stage for. The composite blocks (moe, attention) go first: they contain
/// matmul-like loops and a softmax the simpler recognizers would otherwise claim.
pub const RECOGNIZER_ORDER: &[(&str, Recognizer)] = &[
    ("moe", r_moe),
    ("lut_attention", r_lut_attention),
    ("attention", r_attention),
    ("full_attention", r_full_attention),
    ("row_gather", r_row_gather),
    ("rows_gather", r_rows_gather),
    ("argmax", r_argmax),
    ("set_row", r_set_row),
    ("head_matmul", r_head_matmul),
    ("matmul", r_matmul),
    ("softcap", r_softcap),
    ("elementwise", r_elementwise),
    ("int_requant", r_int_requant),
    ("int_clamp", r_int_clamp),
    ("relu", r_relu),
    ("rope", r_rope),
    ("silu", r_silu),
    ("sigmoid", r_sigmoid),
    ("gelu", r_gelu),
    ("scale", r_scale),
    ("decay", r_decay),
    ("causal_conv", r_causal_conv),
    ("gated_delta", r_gated_delta),
    ("l2norm", r_l2norm),
    ("fused_row", r_fused_row),
    ("transpose", r_transpose),
    ("head_slice", r_head_slice),
    ("join", r_join),
    ("swiglu", r_swiglu),
];

/// Walk a program's top-level statements into typed specs. `Err` only when the source does not parse; a
/// statement that parses but is no op the walk knows is reported in `unrecognized`.
pub fn recognize_stages(source: &str) -> Result<RecognizedProgram, SpatialReject> {
    crate::ui_bridge::with_parsed_tensor_program(source, |parsed, interner, shapes| {
        let (stmts, _types, _policies) = parsed.map_err(SpatialReject::Parse)?;
        let mut rp = walk(stmts, interner);
        rp.shapes = shapes
            .iter()
            .filter(|(_, (r, c))| *r > 0 && *c > 0)
            .map(|(sym, (r, c))| {
                (
                    interner.resolve(*sym).to_string(),
                    (*r as usize, *c as usize),
                )
            })
            .collect();
        Ok(rp)
    })
}

/// [`recognize_stages`] over statements already parsed — every backend's front end: the CUDA forward refuses what
/// this walk reports as `unrecognized` or `unlifted_writes`, so both lanes agree on what a program's statements are.
/// The shape table is the caller's to add (it comes from the parse, not the statements).
pub fn walk(stmts: &[Stmt], interner: &Interner) -> RecognizedProgram {
    let mut rp = RecognizedProgram::default();
    let mut tensors: std::collections::HashSet<Symbol> = Default::default();
    let (fills, assigned) = (Fills::of(stmts), assigned(stmts));
    // The names declared, and the names a recognized stage reads or writes.
    let (mut declared, mut staged): (std::collections::HashSet<String>, std::collections::HashSet<String>) = Default::default();
    for (i, s) in stmts.iter().enumerate() {
        let touched = crate::optimize::mentions::Mentions::of_stmt(s);
        if touched.is_opaque() {
            rp.opaque.push(i);
        }
        for sym in touched.symbols() {
            rp.mentions.entry(interner.resolve(sym).to_string()).or_default().insert(i);
        }
        // ★ A NAME IS BOUND ONCE. Every section reads the one value a name was declared with — its fill is folded
        // from the whole statement list — so a second binding of a declared or computed name is refused, never
        // lowered as the first.
        let bound = |t: Symbol| {
            let name = interner.resolve(t);
            declared.contains(name) || staged.contains(name)
        };
        let declaration = tensor_declaration(s)
            .map(|(var, len)| (var, len, None))
            .or_else(|| scalar_declaration(s, &assigned).map(|(var, fill)| (var, 1, Some(fill))));
        if let Some((var, len, scalar)) = declaration {
            if bound(var) {
                rp.unlifted_writes
                    .push((i, interner.resolve(var).to_string(), SECOND_BINDING));
                continue;
            }
            let fill = scalar.unwrap_or_else(|| {
                tensors.insert(var);
                fills.fill(var)
            });
            declared.insert(interner.resolve(var).to_string());
            rp.quiet.insert(i);
            rp.declared.push(DeclaredTensor {
                name: interner.resolve(var).to_string(),
                len,
                fill,
            });
            continue;
        }
        if !is_compound(s) {
            let touched = |t: Symbol| staged.contains(interner.resolve(t));
            match unlifted_write(s, &tensors, &bound, &touched) {
                Some((t, why)) => rp.unlifted_writes.push((i, interner.resolve(t).to_string(), why)),
                None if matches!(s, Stmt::SetIndex { collection: Expr::Identifier(t), .. } if tensors.contains(t)) => {
                    rp.quiet.insert(i);
                }
                None => {}
            }
            continue;
        }
        let one = std::slice::from_ref(s);
        match RECOGNIZER_ORDER.iter().find_map(|(_, r)| r(one, interner)) {
            Some(op) => {
                let out = op.out();
                let ins = op.ins();
                for sym in std::iter::once(out).chain(ins.iter().copied()) {
                    rp.symbols
                        .entry(sym)
                        .or_insert_with(|| interner.resolve(sym).to_string());
                }
                staged.extend(std::iter::once(out).chain(ins.iter().copied()).map(|t| interner.resolve(t).to_string()));
                rp.stages.push(RecognizedStage {
                    out: interner.resolve(out).to_string(),
                    ins: ins
                        .iter()
                        .map(|s| interner.resolve(*s).to_string())
                        .collect(),
                    op,
                    stmt_index: i,
                });
            }
            None => rp.unrecognized.push((i, describe(s, interner))),
        }
    }
    rp
}

/// `Let A be a R by C tensor of T` — the declaration's desugar: the binder, a zero counter, and one counted
/// loop pushing the element type's zero `R·C` times. Returns `(A, R·C)`.
fn tensor_declaration(s: &Stmt) -> Option<(Symbol, usize)> {
    let Stmt::Splice { body } = s else {
        return None;
    };
    let [Stmt::Let {
        var, value: new, ..
    }, Stmt::Let {
        var: ctr,
        value: start,
        ..
    }, Stmt::While {
        cond, body: fill, ..
    }] = body
    else {
        return None;
    };
    if !matches!(new, Expr::New { .. }) || !matches!(start, Expr::Literal(Literal::Number(0))) {
        return None;
    }
    let Expr::BinaryOp { left, right, .. } = cond else {
        return None;
    };
    let (Expr::Identifier(iv), Expr::Literal(Literal::Number(len))) = (*left, *right) else {
        return None;
    };
    let [Stmt::Push { value, collection }, Stmt::Set { .. }] = fill else {
        return None;
    };
    let zero = matches!(value, Expr::Literal(Literal::Number(0)))
        || matches!(value, Expr::Literal(Literal::Float(f)) if *f == 0.0);
    (iv == ctr && zero && matches!(collection, Expr::Identifier(c) if c == var) && *len >= 0)
        .then_some((*var, *len as usize))
}

/// `Let tok be 2.` at the top level — an Int scalar, the one-element tensor a row sugar's NAMED index reads.
/// Its value is a compile-time constant only if nothing assigns it again; a scalar that is re-`Set` is
/// data the compiler does not know (`Fill::Unknown`), never a stale constant.
fn scalar_declaration(s: &Stmt, assigned: &std::collections::HashSet<Symbol>) -> Option<(Symbol, Fill)> {
    let Stmt::Let {
        var,
        value: Expr::Literal(Literal::Number(n)),
        ..
    } = s
    else {
        return None;
    };
    Some((
        *var,
        if assigned.contains(var) {
            Fill::Unknown
        } else {
            Fill::Const(*n)
        },
    ))
}

/// Every name a `Set` assigns anywhere in the statements, nested blocks and loops included.
fn assigned(stmts: &[Stmt]) -> std::collections::HashSet<Symbol> {
    fn walk(stmts: &[Stmt], into: &mut std::collections::HashSet<Symbol>) {
        for s in stmts {
            match s {
                Stmt::Set { target, .. } => {
                    into.insert(*target);
                }
                Stmt::Splice { body } | Stmt::While { body, .. } | Stmt::Repeat { body, .. } => walk(body, into),
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    walk(then_block, into);
                    if let Some(eb) = else_block {
                        walk(eb, into);
                    }
                }
                _ => {}
            }
        }
    }
    let mut into = std::collections::HashSet::new();
    walk(stmts, &mut into);
    into
}

/// Why a declaration or `Let` of a name already bound is refused.
const SECOND_BINDING: &str = "a second binding of a name already bound — every section reads the value it was first bound to";

/// ★ A WRITE THE WALK WOULD SKIP IS A WRITE THE PROGRAM WOULD LOSE. Top-level simple statements are passed over,
/// and a tensor's declared fill is the value it holds when an op first reads it — so the only writes that fill may
/// absorb are literal `Set item i of T to v` statements made BEFORE any op touches `T` (`recognize_fill` folds
/// exactly those). Any other top-level write to a declared tensor is data no section computes; it is reported, and
/// the program is refused by name, rather than lowered with the tensor holding values the program never had. So is
/// any `Let` of a name already `bound`, and a `Set` of a value an op has already read (`touched`): a scalar the
/// sections read is one value for the whole program.
fn unlifted_write(
    s: &Stmt,
    tensors: &std::collections::HashSet<Symbol>,
    bound: &dyn Fn(Symbol) -> bool,
    touched: &dyn Fn(Symbol) -> bool,
) -> Option<(Symbol, &'static str)> {
    let ident = |e: &Expr| match e {
        Expr::Identifier(t) => Some(*t),
        _ => None,
    };
    match s {
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            let t = ident(collection).filter(|t| tensors.contains(t))?;
            let literal = matches!(index, Expr::Literal(Literal::Number(_)))
                && matches!(value, Expr::Literal(Literal::Number(_) | Literal::Float(_)));
            if !literal {
                Some((
                    t,
                    "a write whose position or value is not a literal — data no section computes",
                ))
            } else if touched(t) {
                Some((t, "a literal write after an op has used the tensor — folding it into the fill would move it before that op"))
            } else {
                None
            }
        }
        Stmt::Push { collection, .. } => ident(collection)
            .filter(|t| tensors.contains(t))
            .map(|t| (t, "a push that grows a declared tensor")),
        Stmt::Set { target, .. } | Stmt::Let { var: target, .. } if tensors.contains(target) => {
            Some((*target, "a rebinding of a declared tensor"))
        }
        Stmt::Let { var, .. } if bound(*var) => Some((*var, SECOND_BINDING)),
        Stmt::Set { target, .. } if touched(*target) => {
            Some((*target, "a write to a value an op has already read"))
        }
        _ => None,
    }
}

/// A statement that can hold a tensor op: a desugar's `Splice`, or a hand-written loop nest or branch.
fn is_compound(s: &Stmt) -> bool {
    matches!(
        s,
        Stmt::Splice { .. } | Stmt::While { .. } | Stmt::Repeat { .. } | Stmt::If { .. }
    )
}

/// How an unrecognized statement is reported: the binder of a desugared `Let`, or the loop variable of a
/// hand-written loop.
fn describe(s: &Stmt, interner: &Interner) -> String {
    match s {
        Stmt::Splice { body } => body
            .iter()
            .find_map(|st| match st {
                Stmt::Let { var, .. } => Some(interner.resolve(*var).to_string()),
                _ => None,
            })
            .unwrap_or_else(|| "Splice".to_string()),
        Stmt::While { cond, .. } => match cond {
            Expr::BinaryOp {
                left: Expr::Identifier(iv),
                ..
            } => format!("While {}", interner.resolve(*iv)),
            _ => "While".to_string(),
        },
        Stmt::Repeat { .. } => "Repeat".to_string(),
        Stmt::If { .. } => "If".to_string(),
        _ => "statement".to_string(),
    }
}

/// The dataflow by variable name: for every stage, the later stages that read its output.
pub fn links(stages: &[RecognizedStage]) -> Vec<Link> {
    let mut readers: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (j, s) in stages.iter().enumerate() {
        for name in &s.ins {
            let r = readers.entry(name.as_str()).or_default();
            if r.last() != Some(&j) {
                r.push(j);
            }
        }
    }
    stages
        .iter()
        .enumerate()
        .map(|(i, s)| Link {
            producer: i,
            consumers: readers
                .get(s.out.as_str())
                .map(|r| r.iter().copied().filter(|&j| j > i).collect())
                .unwrap_or_default(),
            tensor: s.out.clone(),
        })
        .collect()
}

/// The state a step CARRIES to the next — [`state_tensors`] without the tables the step assembles whole. A table every
/// row of which the step writes at a literal row (`Set row 2 of X`, `Set rows from 1 of X`, a block gated delta's
/// output) before any stage reads it begins each step with nothing the step can see: a chunk's stacked rows, its RoPE
/// block. The KV cache is carried —
/// it is written at a position the data gives and read far beyond it — and so are the conv rings and the memories,
/// which every step reads before it writes.
pub fn carried_state(rp: &RecognizedProgram) -> Vec<String> {
    let assembled = |table: &str| -> bool {
        let mut rows: Option<usize> = None;
        let mut covered: BTreeSet<usize> = BTreeSet::new();
        for s in &rp.stages {
            let writes = matches!(&s.op, TensorOpSpec::SetRow(_)) && s.ins[0] == table;
            let reads = s
                .ins
                .iter()
                .enumerate()
                .any(|(k, n)| n == table && !(writes && k == 0));
            if reads && rows.is_none_or(|r| covered.len() < r) {
                return false;
            }
            if let (true, TensorOpSpec::SetRow(r)) = (writes, &s.op) {
                let RowIndex::Literal(first) = r.index else {
                    return false;
                };
                rows = Some(r.rows);
                covered.extend(
                    (first as usize).saturating_sub(1)
                        ..(first as usize).saturating_sub(1) + r.count,
                );
            }
            if let Some(n) = s.op.assembled_rows().filter(|_| s.out == table) {
                rows = Some(n);
                covered.extend(0..n);
            }
        }
        rows.is_some_and(|r| (0..r).all(|y| covered.contains(&y)))
    };
    state_tensors(&rp.stages)
        .into_iter()
        .filter(|t| !assembled(t))
        .collect()
}

/// The state, in first-appearance order: every tensor some stage reads and writes back in place, and every table an
/// op assembles a row at a time ([`TensorOpSpec::assembled_rows`]).
pub fn state_tensors(stages: &[RecognizedStage]) -> Vec<String> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut out: Vec<String> = Vec::new();
    for s in stages {
        for pos in s.op.state_inputs() {
            let name = &s.ins[pos];
            if seen.insert(name.as_str()) {
                out.push(name.clone());
            }
        }
        if s.op.assembled_rows().is_some() && seen.insert(s.out.as_str()) {
            out.push(s.out.clone());
        }
    }
    out
}
