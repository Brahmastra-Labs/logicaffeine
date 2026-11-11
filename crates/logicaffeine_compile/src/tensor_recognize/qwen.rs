//! The Qwen-family recognizers — the ops beyond matmul / elementwise / fused-row / swiglu that an authored
//! transformer forward is made of, each lifted from the EXACT statement shape its parser sugar desugars to
//! (`parser/mod.rs`, the `desugar_tensor_*` family) into a typed spec.
//!
//! Shared the way [`super`] is: the CUDA orchestrator's stage walk adapts these specs into its tuples, and
//! the spatial front end's whole-program walk composes them. Recognition is structural — no markers, no
//! naming conventions — and every recognizer here is keyed on its own sugar's body, so a composite block is
//! never claimed as one of its parts and a neighbouring sugar with the same skeleton (silu / gelu, rope /
//! transpose) is told apart by the sub-expression that differs. Each answers `None` for anything else.

use std::collections::{HashMap, HashSet};

use logicaffeine_language::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};

use super::{as_binop, as_ident, as_index, Epsilon, Factor};
use crate::intern::{Interner, Symbol};

/// `Qr = rope(Q, cos, sin)` — RoPE-NEOX over `h` heads of `hd` dims, rotating each head's leading `2·half`
/// dims as `(i, i+half)` pairs and passing the remaining `hd − 2·half` through (Qwen3.8's partial rotary;
/// `hd == 2·half` is the classic full form). `hd` is read off the pass-through loop's bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RopeSpec {
    pub q: Symbol,
    pub rc: Symbol,
    pub rs: Symbol,
    pub out: Symbol,
    pub h: usize,
    pub half: usize,
    pub hd: usize,
    /// The tables' rows: 1 — every head turns by the one row of angles — or one per row of `Q`, head `v` turning by
    /// table row `v / (h / rows)`. With [`from`](Self::from), the rows of `Q`.
    pub rows: usize,
    /// `from position P`: the tables are a cache of angles for this many positions, and row `r` of `Q` (0-based) turns
    /// by cache row `P + r` (1-based) — `P` checked to keep the block's rows among the cache's.
    pub from: Option<(RowIndex, usize)>,
}

/// `Y = silu(X)` — `x / (1 + e^-x)` over `n` entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiluSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub n: usize,
}

/// `Y = sigmoid(X)` — `1 / (1 + e^-x)` over `n` entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigmoidSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub n: usize,
}

/// `Y = gelu(X)` — ggml's tanh approximation in its one-exponential form `x / (1 + e^-2z)`,
/// `z = √(2/π)·(x + 0.044715·x³)`, over `n` entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeluSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub n: usize,
}

/// `Y = X scaled by C` — `x·C` over `n` entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub n: usize,
    pub factor: Factor,
}

/// `Y = X capped at C` — `C·tanh(x/C)` over `n` entries (Gemma 2's logits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoftcapSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub n: usize,
    pub cap: Factor,
}

/// `M = the mean of X over L lanes` — each row of `X` (`rows × lanes·cols`) as the mean of its `lanes` lanes of `cols`:
/// `M[t, c] = (Σ_l X[t, l·cols + c]) / lanes`. A hyper-connected residual's lanes mixed into the one row a sublayer
/// reads (the gate a product before it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneMeanSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub rows: usize,
    pub lanes: usize,
    pub cols: usize,
}

/// `P = O spread over L lanes [by S]` — each row of `O` (`rows × cols`) laid in every one of `lanes` lanes, lane `l` of
/// row `t` times `S[t, l]` (`S` `rows × lanes`) when `by` is given: `P[t, l·cols + c] = O[t, c]·S[t, l]`. A
/// hyper-connected residual's injection, which the residual adds after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneSpreadSpec {
    pub x: Symbol,
    pub by: Option<Symbol>,
    pub out: Symbol,
    pub rows: usize,
    pub lanes: usize,
    pub cols: usize,
}

/// `G = the lane sums of X over L lanes` — each of a row's `lanes` lanes of `cols` (`X` `rows × lanes·cols`) summed on
/// its own: `G[t, l] = Σ_c X[t, l·cols + c]`. A per-lane dot after a product (the n-gram embedding's gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneSumsSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub rows: usize,
    pub lanes: usize,
    pub cols: usize,
}

/// `Y = the signed root of X` — every element `v` as `sign(v)·√max(|v|, eps)` over `n` entries (the n-gram embedding's
/// per-lane gate before its sigmoid).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedRootSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub n: usize,
    pub eps: super::Epsilon,
}

/// `I = the ngram ids of T after R by M modulo V from O ending at E` — for each of `tokens` token rows (a token's id
/// plus one), the table rows its n-grams of order 2 to `ngram` hash to, `per_order` heads an order:
/// `((t₀·M₀ ⊕ t₁·M₁ ⊕ …) mod V[h]) + O[h] + 1` over the ids `t_k`, a place where no token was, or beyond an end of
/// document (row `eos`), reading as the end of document. `R`, the `ngram − 1` token rows before the first (`0` where
/// none was), is read AND written: the carried state of a decode step, zero for a fresh sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NgramIdsSpec {
    pub tokens_in: Symbol,
    pub history: Symbol,
    pub multipliers: Symbol,
    pub moduli: Symbol,
    pub offsets: Symbol,
    pub out: Symbol,
    pub tokens: usize,
    pub ngram: usize,
    pub per_order: usize,
    pub eos: i64,
}

impl NgramIdsSpec {
    /// The heads, every order's together: the ids a token names.
    pub fn heads(&self) -> usize {
        (self.ngram - 1) * self.per_order
    }
}

/// `A = decay(Ar, Dt, Alog)` — Gated DeltaNet's per-head forget gate `(1 + e^(Ar+Dt))^Alog` over `n` heads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecaySpec {
    pub ar: Symbol,
    pub dt: Symbol,
    pub alog: Symbol,
    pub out: Symbol,
    pub n: usize,
    /// The rows of `Ar`: 1, or a block of rows decayed by one bias and rate per head, `n / rows` heads to a row.
    pub rows: usize,
}

/// `Y = l2norm(X)` — per-row `X / sqrt(Σ X² + eps)` over a `rows × cols` view (the `across H heads` form is
/// `rows == H`, `cols == |X| / H`), with the program's epsilon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L2NormSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub rows: usize,
    pub cols: usize,
    pub eps: Epsilon,
}

/// Where a row sugar's row comes from: a NAMED scalar (the data-indexed case — a token id, a cache position,
/// an arg-max) or a literal. An arbitrary expression is not lifted; the walk lists the statement instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowIndex {
    Named(Symbol),
    Literal(i64),
}

/// `x = row i of T` — the embedding lookup: row `i` (1-based) of a `rows × cols` table, as `1 × cols`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowGatherSpec {
    pub table: Symbol,
    pub index: RowIndex,
    pub out: Symbol,
    pub rows: usize,
    pub cols: usize,
}

/// `X = rows I of T` — the block gather: row `k` of `X` is row `I[k]` (1-based) of a `rows × cols` table, for each of
/// `I`'s `count` indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowsGatherSpec {
    pub table: Symbol,
    pub indices: Symbol,
    pub out: Symbol,
    pub rows: usize,
    pub cols: usize,
    pub count: usize,
}

/// `t = argmax of X` — the 1-based index of the first maximal element of `X`'s `n` elements, an Int.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgMaxSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub n: usize,
}

/// `Set row i of T to S` — the KV-cache append: row `i` (1-based) of a `rows × cols` table overwritten by
/// the `cols` elements of `S`. `T` is read AND written: carried state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetRowSpec {
    pub table: Symbol,
    pub index: RowIndex,
    pub source: Symbol,
    pub rows: usize,
    pub cols: usize,
    /// The rows written: 1 for `Set row`, `R` for the block `Set rows from P`, rows `P … P + R − 1`.
    pub count: usize,
}

/// A scatter of rows: for `k` from 1 to `count` in turn, row `k` of the `source_rows × cols` source `X` lands on row
/// `I[k]` (1-based) of the `rows × cols` table `T`. Without a `fold` it replaces the row, so a row named twice keeps the
/// later write; with one it is folded into the row it lands on (`old ⊕ r`), so a row named twice takes both. `T` is read
/// AND written: carried state. The meaning of the loop the program writes, which is the VM's own:
///
/// `Repeat for k from 1 to N:` `Let at be item k of I.` `Let r be row k of X.` `Set row at of T to r.` — or, folding,
/// `Let old be row at of T.` `Let sum be old plus r.` `Set row at of T to sum.` after the reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScatterSpec {
    pub table: Symbol,
    pub indices: Symbol,
    pub source: Symbol,
    pub rows: usize,
    pub cols: usize,
    /// The source's rows: the loop reads its first `count`.
    pub source_rows: usize,
    /// The index tensor's length: the loop reads its first `count`.
    pub index_len: usize,
    pub count: usize,
    pub fold: Option<super::ElementwiseOp>,
}

/// `C = full attention of Q over K and V with H heads` — bidirectional multi-head self-attention over `t`
/// positions; `Q`/`K`/`V`/`C` are `t × (h·hd)`. Every query row attends to every position, unmasked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullAttentionSpec {
    pub q: Symbol,
    pub k: Symbol,
    pub v: Symbol,
    pub out: Symbol,
    pub h: usize,
    pub hd: usize,
    pub t: usize,
}

/// `Kt = transpose(X)` — an `rows × cols` tensor becomes `cols × rows` (`Kt[j·rows + i] = X[i·cols + j]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransposeSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub rows: usize,
    pub cols: usize,
}

/// `P = the first K of every N of X` / `the last K of every N of X` / `the K after the first F of every N of X` — one
/// part of every group of `every` consecutive elements of `X`'s `n`: the `width` elements from offset `from` of each
/// group, in order (`P[g·width + i] = X[g·every + from + i]`). A head's part of a row of heads, or a part of every
/// cached row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadSliceSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub every: usize,
    pub from: usize,
    pub width: usize,
    pub n: usize,
}

/// `J = A joined with B [across H heads]` — `rows` rows of `heads` heads, each head's part of `A` then its part of `B`:
/// `A` is `wa` wide and `B` `wb` wide per row, a head's parts `wa / heads` and `wb / heads` wide. One head joins the rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinSpec {
    pub a: Symbol,
    pub b: Symbol,
    pub out: Symbol,
    pub rows: usize,
    pub heads: usize,
    pub wa: usize,
    pub wb: usize,
}

/// `Y = causal conv(X, WC, CS)` — the depthwise causal conv1d over `chan` channels with a `taps`-deep decode
/// ring `CS` (shifted up, appended with `X`, reduced against `WC`). `CS` is read AND written: carried state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CausalConvSpec {
    pub x: Symbol,
    pub wc: Symbol,
    pub cs: Symbol,
    pub out: Symbol,
    pub taps: usize,
    pub chan: usize,
    /// The rows of `X`: 1, or a block of rows entering the ring one after another.
    pub rows: usize,
}

/// `O = gated delta(Q, K, V, Z, B, A, S, GN)` — the Gated DeltaNet step over `hv` value heads reading `hk`
/// key heads at head dim `d`: `u = S·k`, `S ← α·S + β·(v − α·u)·kᵀ`, `o = S·q`, RMSNorm'd, scaled by `GN`,
/// gated by `silu(z)`, the norm taking the program's epsilon. `S` (`st`) is read AND written: the carried memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatedDeltaSpec {
    pub q: Symbol,
    pub k: Symbol,
    pub v: Symbol,
    pub z: Symbol,
    pub b: Symbol,
    pub a: Symbol,
    pub st: Symbol,
    pub gn: Symbol,
    pub out: Symbol,
    pub hv: usize,
    pub hk: usize,
    pub d: usize,
    /// The rows: 1, or a block of tokens each taking the step in turn against the one memory.
    pub rows: usize,
    /// The epsilon of the gated output norm.
    pub eps: Epsilon,
    /// What gates the normed head: `silu(z)` (`with gate Z`) or `σ(z)` (`with sigmoid gate Z`).
    pub gate: GateAct,
}

/// The activation of a Gated DeltaNet's output gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GateAct {
    Silu,
    Sigmoid,
}

/// The positions of the `n` an attention query attends that it keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Window {
    /// The last `W`: `max(1, n − W + 1) ..= n` (`within a window of W`).
    Sliding(usize),
    /// Those of the query's own chunk of `C`, Llama 4's chunked-local attention: `⌊(n − 1)/C⌋·C + 1 ..= n`
    /// (`within chunks of C`).
    Chunked(usize),
}

impl Window {
    /// The first position kept, 1-based, by a query attending `n` positions.
    pub fn first(self, n: usize) -> usize {
        match self {
            Window::Sliding(w) => n.saturating_sub(w - 1).max(1),
            Window::Chunked(c) => (n - 1) / c * c + 1,
        }
    }

    /// The most positions a query keeps.
    pub fn span(self) -> usize {
        match self {
            Window::Sliding(w) | Window::Chunked(w) => w,
        }
    }

    /// The clause the language writes it as, after an attention's length.
    pub fn clause(self) -> String {
        match self {
            Window::Sliding(w) => format!(" within a window of {w}"),
            Window::Chunked(c) => format!(" within chunks of {c}"),
        }
    }
}

/// `ctx = attention(q over kc, vc)` — single-query multi-head GQA attention: `q` is `1 × (h·hd)`, `kc` is `l × (kv·hd)`
/// and `vc` `l × (kv·vd)` (the cached keys/values), query head `g` attends to kv head `g / (h/kv)` and writes `vd`
/// values. `pos` is the position bound of the masked `up to P` form (the KV cache as carried state); `None` is the
/// unmasked form that attends to every cached row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionSpec {
    pub q: Symbol,
    pub kc: Symbol,
    pub vc: Symbol,
    pub out: Symbol,
    pub h: usize,
    pub kv: usize,
    pub hd: usize,
    /// The value head width: `hd`, or narrower (`V` holding `kv·vd` columns).
    pub vd: usize,
    /// The score scale a program gives (`scaled by S`); `None` is the default `1/√hd`, which a given scale equal to
    /// it also lifts as.
    pub scale: Option<Factor>,
    /// The `capped at C` form's `C`: every scaled score `s` taken as `C·tanh(s/C)` before the softmax (Gemma 2).
    pub softcap: Option<Factor>,
    pub l: usize,
    /// The masked `up to P` form's bound: attend over the first `P` of the `l` cached positions. For a block of `rows`
    /// query rows (`causal attention … from P`) it is the FIRST row's: row `r` (0-based) attends `P + r` positions.
    pub pos: Option<RowIndex>,
    /// Query rows: 1 for the single-query form, `T` for a causal block.
    pub rows: usize,
    /// The positions a query keeps of the `n` it attends: the last `W` (`within a window of W`), or those of its own chunk
    /// of `C` (`within chunks of C`).
    pub window: Option<Window>,
    /// The `with sinks S` form's `S`: a logit per query head joining its softmax, taking no value.
    pub sinks: Option<Symbol>,
}

/// A recognized **Mixture-of-Experts FFN** — `Let out be moe of x with router wr gate wg up wu down wd
/// choosing K of E`. Lifted from the desugared block the SAME way `recognize_matmul`/`recognize_rope` lift
/// theirs — purely structural, no markers, no naming conventions: the OPERAND tensors are the indexed reads
/// NOT `Let`-bound inside the block (the temps are), and the dims fall straight out of the weight index
/// arithmetic (`wr[(t-1)·E+e]` yields E; `wg[((e-1)·dff+(o-1))·d+t]` yields d and dff); `out` is the tensor the
/// weighted accumulation writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoeSpec {
    pub out: Symbol,
    pub x: Symbol,
    pub wr: Symbol,
    pub wg: Symbol,
    pub wu: Symbol,
    pub wd: Symbol,
    pub d: usize,
    pub dff: usize,
    pub ne: usize,
    pub k: usize,
    /// The router's bias, added to each logit before the softmax (`router Wr plus Br`).
    pub router_bias: Option<Symbol>,
    /// The experts' gate, up and down biases (`gate Wg plus Bg`, …), each a row per expert.
    pub biases: [Option<Symbol>; 3],
    /// The kept experts' weights are renormalized to sum to one; `false` (`without renormalizing`) weighs each by its
    /// softmax over every expert.
    pub renormalize: bool,
    /// The experts' gated activation.
    pub act: MoeAct,
    /// How the experts are scored: the softmax over every logit, or each logit's own sigmoid.
    pub score: MoeScore,
    /// A kept expert's weight scales the input it reads, and its output joins the sum unweighted (Llama 4).
    pub weigh_inputs: bool,
    /// The selection bias (`biased by B`): each expert chosen by its score plus `B[e]`, weighed by its score alone.
    pub selection_bias: Option<Symbol>,
    /// The group limit (`within the top G of N groups`).
    pub groups: Option<MoeGroups>,
    /// The routed scale (`scaled by S`) as its bits: every kept weight times `S`, after the renormalization.
    pub scale: Option<u64>,
}

/// A mixture's experts in `count` groups of `ne / count`, each scored by the sum of its two best selection scores (its
/// one where a group holds one expert), the experts chosen only among the top `kept` groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MoeGroups {
    pub count: usize,
    pub kept: usize,
}

impl MoeSpec {
    /// The routed scale, one when the mixture has none.
    pub fn scale(&self) -> f64 {
        self.scale.map_or(1.0, f64::from_bits)
    }

    /// Whether the router is a softmax over every expert weighting the kept experts' outputs — chosen by their scores
    /// alone, among every expert, unscaled: the routing a lowering that knows no other computes.
    pub fn weighs_outputs_by_softmax(&self) -> bool {
        self.score == MoeScore::Softmax && self.selection_bias.is_none() && self.groups.is_none() && self.scale.is_none() && !self.weigh_inputs
    }

    /// Whether the renormalizing sum's clamp at 2⁻¹⁴ can bind: a softmax's kept weights sum to at least `K/E`, so only a
    /// sigmoid's — or a softmax over 16,384 experts or more — can fall below it.
    pub fn sum_can_clamp(&self) -> bool {
        self.renormalize && (self.score == MoeScore::Sigmoid || self.ne >= 1 << 14)
    }
}

/// A mixture's gated activation: SwiGLU, or gpt-oss's SwiGLU with the gate clamped to at most `limit`, the up to
/// `[−limit, limit]`, and the sigmoid sharpened by `slope` (both kept as their bits, so the spec stays `Eq`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MoeAct {
    Swiglu,
    ClampedSwiglu { limit: u64, slope: u64 },
}

/// The activation a unary elementwise map applies: the three sugars that share the one-loop `Push f(X[i])`
/// skeleton and differ only in `f`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Activation {
    Silu,
    Sigmoid,
    Gelu,
}

impl Activation {
    /// `x · σ(u(x))` — the input itself gated by a sigmoid of some function of it (silu: `u = x`; gelu:
    /// `u = 2z`). Sigmoid is the gate alone.
    pub fn is_self_gated(self) -> bool {
        matches!(self, Activation::Silu | Activation::Gelu)
    }
}

/// The one-loop unary map `Y[i] = f(X[i])` over `n` entries, with `f` classified exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnaryMapSpec {
    pub act: Activation,
    pub x: Symbol,
    pub out: Symbol,
    pub n: usize,
}

const SQRT_2_OVER_PI: f64 = 0.797_884_560_802_865_4;
const GELU_K: f64 = 0.044_715;

// ── expression and statement views ─────────────────────────────────────────────────────────────────────

fn as_int(e: &Expr) -> Option<i64> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        _ => None,
    }
}

fn as_float(e: &Expr) -> Option<f64> {
    match e {
        Expr::Literal(Literal::Float(f)) => Some(*f),
        _ => None,
    }
}

fn is_float(e: &Expr, want: f64) -> bool {
    as_float(e)
        .map(|f| (f - want).abs() <= 1e-12 * want.abs().max(1.0))
        .unwrap_or(false)
}

fn is_e(e: &Expr) -> bool {
    is_float(e, std::f64::consts::E)
}

fn is_int(e: &Expr, want: i64) -> bool {
    as_int(e) == Some(want)
}

fn is_new(e: &Expr) -> bool {
    matches!(e, Expr::New { .. })
}

fn is_ident(e: &Expr, s: Symbol) -> bool {
    as_ident(e) == Some(s)
}

fn want(ok: bool) -> Option<()> {
    ok.then_some(())
}

/// `coll[idx]` with a named collection.
fn indexed<'a>(e: &'a Expr<'a>) -> Option<(Symbol, &'a Expr<'a>)> {
    let (c, i) = as_index(e)?;
    Some((as_ident(c)?, i))
}

/// `X[iv]` — a named collection read at exactly the loop variable.
fn read_at(e: &Expr, iv: Symbol) -> Option<Symbol> {
    let (c, i) = indexed(e)?;
    is_ident(i, iv).then_some(c)
}

/// `X[i]·X[i]` — a square of one element read.
fn square_of(e: &Expr) -> Option<Symbol> {
    let (l, r) = as_binop(e, BinaryOpKind::Multiply)?;
    let (x, _) = indexed(l)?;
    let (y, _) = indexed(r)?;
    (x == y).then_some(x)
}

/// A counted loop: its variable, its literal bound, whether the bound is inclusive (`<=`), and its body.
struct Loop<'a> {
    iv: Symbol,
    bound: i64,
    inclusive: bool,
    body: &'a [Stmt<'a>],
}

fn as_loop<'a>(s: &'a Stmt<'a>) -> Option<Loop<'a>> {
    let Stmt::While { cond, body, .. } = s else {
        return None;
    };
    let Expr::BinaryOp { op, left, right } = *cond else {
        return None;
    };
    let iv = as_ident(left)?;
    let bound = as_int(right)?;
    let inclusive = match op {
        BinaryOpKind::LtEq => true,
        BinaryOpKind::Lt => false,
        _ => return None,
    };
    Some(Loop {
        iv,
        bound,
        inclusive,
        body: *body,
    })
}

/// A sequential reader over a statement list — each desugar lays its statements out in a fixed order, and a
/// matcher consumes them in that order, refusing at the first statement that is not the expected one.
#[derive(Clone)]
struct Cursor<'a> {
    stmts: &'a [Stmt<'a>],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(stmts: &'a [Stmt<'a>], at: usize) -> Self {
        Cursor { stmts, at }
    }
    fn next(&mut self) -> Option<&'a Stmt<'a>> {
        let s = self.stmts.get(self.at)?;
        self.at += 1;
        Some(s)
    }
    /// `Let var be <value>`.
    fn let_(&mut self) -> Option<(Symbol, &'a Expr<'a>)> {
        match self.next()? {
            Stmt::Let { var, value, .. } => Some((*var, *value)),
            _ => None,
        }
    }
    /// `Let var be <int literal>`.
    fn int_let(&mut self, value: i64) -> Option<Symbol> {
        let (var, v) = self.let_()?;
        is_int(v, value).then_some(var)
    }
    /// `Let var be <float literal>`.
    fn float_let(&mut self, value: f64) -> Option<Symbol> {
        let (var, v) = self.let_()?;
        is_float(v, value).then_some(var)
    }
    /// `Let var be a new Seq`.
    fn new_let(&mut self) -> Option<Symbol> {
        let (var, v) = self.let_()?;
        is_new(v).then_some(var)
    }
    /// A counted loop over `iv` with the given bound form.
    fn loop_(&mut self, iv: Symbol, inclusive: bool) -> Option<Loop<'a>> {
        let l = as_loop(self.next()?)?;
        (l.iv == iv && l.inclusive == inclusive).then_some(l)
    }
    /// `Push <value> to <collection>`.
    fn push(&mut self) -> Option<(&'a Expr<'a>, Symbol)> {
        match self.next()? {
            Stmt::Push { value, collection } => Some((*value, as_ident(collection)?)),
            _ => None,
        }
    }
    /// `Set target to <value>`.
    fn set(&mut self) -> Option<(Symbol, &'a Expr<'a>)> {
        match self.next()? {
            Stmt::Set { target, value } => Some((*target, *value)),
            _ => None,
        }
    }
    /// `Set item <index> of <collection> to <value>`.
    fn set_index(&mut self) -> Option<(Symbol, &'a Expr<'a>, &'a Expr<'a>)> {
        match self.next()? {
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => Some((as_ident(collection)?, *index, *value)),
            _ => None,
        }
    }
}

/// Every `Let var be a new Seq` in a statement list — the binder each tensor desugar opens with — as
/// `(position, var)`; a matcher reads the sugar's body from the statement after it.
fn anchors<'a>(body: &'a [Stmt<'a>]) -> impl Iterator<Item = (usize, Symbol)> + 'a {
    body.iter().enumerate().filter_map(|(i, s)| match s {
        Stmt::Let { var, value, .. } if is_new(value) => Some((i, *var)),
        _ => None,
    })
}

/// Try `f` on the statement list and on every nested block (through `Splice`, loops and branches), first
/// match wins — so a recognizer accepts a whole program, one top-level statement, or a bare desugar body.
fn find_in_bodies<'a, T, F>(stmts: &'a [Stmt<'a>], f: F) -> Option<T>
where
    F: Fn(&'a [Stmt<'a>]) -> Option<T> + Copy,
{
    if let Some(t) = f(stmts) {
        return Some(t);
    }
    for s in stmts {
        let found = match s {
            Stmt::Splice { body } | Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                find_in_bodies(body, f)
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => find_in_bodies(then_block, f)
                .or_else(|| else_block.and_then(|eb| find_in_bodies(eb, f))),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

// ── the one-loop unary maps: silu / sigmoid / gelu ─────────────────────────────────────────────────────

/// `1 + E^(0 − X[iv])` → `X`.
fn sigmoid_denominator(e: &Expr, iv: Symbol) -> Option<Symbol> {
    let (one, pow) = as_binop(e, BinaryOpKind::Add)?;
    want(is_float(one, 1.0))?;
    let (base, expo) = as_binop(pow, BinaryOpKind::Pow)?;
    want(is_e(base))?;
    let (zero, xi) = as_binop(expo, BinaryOpKind::Subtract)?;
    want(is_float(zero, 0.0))?;
    read_at(xi, iv)
}

/// `1 + E^(0 − 2·z)` with `z = √(2/π)·(X[iv] + 0.044715·(X[iv]·(X[iv]·X[iv])))` → `X`.
fn gelu_denominator(e: &Expr, iv: Symbol) -> Option<Symbol> {
    let (one, pow) = as_binop(e, BinaryOpKind::Add)?;
    want(is_float(one, 1.0))?;
    let (base, expo) = as_binop(pow, BinaryOpKind::Pow)?;
    want(is_e(base))?;
    let (zero, two_z) = as_binop(expo, BinaryOpKind::Subtract)?;
    want(is_float(zero, 0.0))?;
    let (two, z) = as_binop(two_z, BinaryOpKind::Multiply)?;
    want(is_float(two, 2.0))?;
    let (c, inner) = as_binop(z, BinaryOpKind::Multiply)?;
    want(is_float(c, SQRT_2_OVER_PI))?;
    let (x1, kx3) = as_binop(inner, BinaryOpKind::Add)?;
    let x = read_at(x1, iv)?;
    let (k, x3) = as_binop(kx3, BinaryOpKind::Multiply)?;
    want(is_float(k, GELU_K))?;
    let (xa, xx) = as_binop(x3, BinaryOpKind::Multiply)?;
    let (xb, xc) = as_binop(xx, BinaryOpKind::Multiply)?;
    want(read_at(xa, iv) == Some(x) && read_at(xb, iv) == Some(x) && read_at(xc, iv) == Some(x))?;
    Some(x)
}

/// The pushed value of a unary map, classified: `X[iv] / (1 + e^-X[iv])` (silu), `1 / (1 + e^-X[iv])`
/// (sigmoid), `X[iv] / (1 + e^-2z)` (gelu).
fn classify_unary(value: &Expr, iv: Symbol) -> Option<(Activation, Symbol)> {
    let (num, den) = as_binop(value, BinaryOpKind::Divide)?;
    if is_float(num, 1.0) {
        return sigmoid_denominator(den, iv).map(|x| (Activation::Sigmoid, x));
    }
    let x = read_at(num, iv)?;
    if sigmoid_denominator(den, iv) == Some(x) {
        return Some((Activation::Silu, x));
    }
    if gelu_denominator(den, iv) == Some(x) {
        return Some((Activation::Gelu, x));
    }
    None
}

fn match_unary_map<'a>(body: &'a [Stmt<'a>]) -> Option<UnaryMapSpec> {
    for (i, out) in anchors(body) {
        let mut c = Cursor::new(body, i + 1);
        let Some(iv) = c.int_let(1) else { continue };
        let Some(l) = c.loop_(iv, true) else { continue };
        let Some((value, into)) = Cursor::new(l.body, 0).push() else {
            continue;
        };
        if into != out {
            continue;
        }
        if let Some((act, x)) = classify_unary(value, iv) {
            return Some(UnaryMapSpec {
                act,
                x,
                out,
                n: l.bound as usize,
            });
        }
    }
    None
}

/// Recognize the one-loop unary map `Let Y be silu/sigmoid/gelu of X` anywhere in the statement list.
pub fn recognize_unary_map(stmts: &[Stmt]) -> Option<UnaryMapSpec> {
    find_in_bodies(stmts, match_unary_map)
}

/// Recognize `Let Y be silu of X` — `Push X[i] / (1 + E^(0 − X[i]))` in one counted loop.
pub fn recognize_silu(stmts: &[Stmt]) -> Option<SiluSpec> {
    recognize_unary_map(stmts)
        .filter(|m| m.act == Activation::Silu)
        .map(|m| SiluSpec {
            x: m.x,
            out: m.out,
            n: m.n,
        })
}

/// Recognize `Let Y be sigmoid of X` — `Push 1 / (1 + E^(0 − X[i]))` in one counted loop.
pub fn recognize_sigmoid(stmts: &[Stmt]) -> Option<SigmoidSpec> {
    recognize_unary_map(stmts)
        .filter(|m| m.act == Activation::Sigmoid)
        .map(|m| SigmoidSpec {
            x: m.x,
            out: m.out,
            n: m.n,
        })
}

/// Recognize `Let Y be gelu of X` — `Push X[i] / (1 + E^(0 − 2z))` with the 0.044715 cubic in `z`.
pub fn recognize_gelu(stmts: &[Stmt]) -> Option<GeluSpec> {
    recognize_unary_map(stmts)
        .filter(|m| m.act == Activation::Gelu)
        .map(|m| GeluSpec {
            x: m.x,
            out: m.out,
            n: m.n,
        })
}

/// `x` soft-capped at `c` as the sugar writes it (`capped at C`): `c·(1 − 2/(E^(x·(2/c)) + 1))`. The capped expression
/// and the cap.
fn as_capped<'a>(e: &'a Expr<'a>) -> Option<(&'a Expr<'a>, f64)> {
    let (c, rest) = as_binop(e, BinaryOpKind::Multiply)?;
    let c = as_float(c).filter(|c| c.is_finite() && *c > 0.0)?;
    let (one, fraction) = as_binop(rest, BinaryOpKind::Subtract)?;
    let (two, den) = as_binop(fraction, BinaryOpKind::Divide)?;
    let (pow, one_more) = as_binop(den, BinaryOpKind::Add)?;
    let (base, exponent) = as_binop(pow, BinaryOpKind::Pow)?;
    let (x, k) = as_binop(exponent, BinaryOpKind::Multiply)?;
    let exact = |e: &Expr, v: f64| as_float(e).is_some_and(|f| f.to_bits() == v.to_bits());
    (exact(one, 1.0) && exact(two, 2.0) && exact(one_more, 1.0) && exact(base, std::f64::consts::E) && exact(k, 2.0 / c))
        .then_some((x, c))
}

/// The one-loop `Let Y be X capped at C` — `Push` of `X[i]` capped, [`as_capped`]'s form.
fn match_softcap<'a>(body: &'a [Stmt<'a>]) -> Option<SoftcapSpec> {
    for (i, out) in anchors(body) {
        let mut c = Cursor::new(body, i + 1);
        let Some(iv) = c.int_let(1) else { continue };
        let Some(l) = c.loop_(iv, true) else { continue };
        let Some((value, into)) = Cursor::new(l.body, 0).push() else {
            continue;
        };
        if into != out {
            continue;
        }
        let Some((xi, cap)) = as_capped(value) else { continue };
        if let Some(x) = read_at(xi, iv) {
            return Some(SoftcapSpec { x, out, n: l.bound as usize, cap: Factor::new(cap) });
        }
    }
    None
}

/// Recognize `Let Y be X capped at C` anywhere in the statement list.
pub fn recognize_softcap(stmts: &[Stmt]) -> Option<SoftcapSpec> {
    find_in_bodies(stmts, match_softcap)
}

/// The one-loop `Let Y be X scaled by C` — `Push X[i] · C`, `C` a number literal.
fn match_scale<'a>(body: &'a [Stmt<'a>]) -> Option<ScaleSpec> {
    for (i, out) in anchors(body) {
        let mut c = Cursor::new(body, i + 1);
        let Some(iv) = c.int_let(1) else { continue };
        let Some(l) = c.loop_(iv, true) else { continue };
        let Some((value, into)) = Cursor::new(l.body, 0).push() else {
            continue;
        };
        if into != out {
            continue;
        }
        let Some((xi, factor)) = as_binop(value, BinaryOpKind::Multiply) else { continue };
        if let (Some(x), Some(factor)) = (read_at(xi, iv), as_float(factor)) {
            return Some(ScaleSpec { x, out, n: l.bound as usize, factor: Factor::new(factor) });
        }
    }
    None
}

/// Recognize `Let Y be X scaled by C` anywhere in the statement list.
pub fn recognize_scale(stmts: &[Stmt]) -> Option<ScaleSpec> {
    find_in_bodies(stmts, match_scale)
}

// ── the integer maps: requant / clamp / relu ───────────────────────────────────────────────────────────

/// `Y = X times M shifted right by S` — an int8 model's requant `(x · M) >> S` over `n` Int entries, an arithmetic
/// shift of the exact product.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntRequantSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub n: usize,
    pub m: i64,
    pub s: u32,
}

/// `Y = X clamped between LO and HI` — saturation, `min(max(x, LO), HI)` over `n` Int entries, `LO ≤ HI`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntClampSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub n: usize,
    pub lo: i64,
    pub hi: i64,
}

/// `Y = relu of X` — `max(x, 0)` over `n` entries, in `X`'s element kind (`int`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReluSpec {
    pub x: Symbol,
    pub out: Symbol,
    pub n: usize,
    pub int: bool,
}

/// The integer map a one-loop push applies.
enum IntMap {
    Requant { m: i64, s: u32 },
    Clamp { lo: i64, hi: i64 },
    Relu { int: bool },
}

/// `name(args…)` with `name` resolving to `want`.
fn call_of<'a>(e: &'a Expr<'a>, interner: &Interner, want: &str) -> Option<&'a [&'a Expr<'a>]> {
    match e {
        Expr::Call { function, args } if interner.resolve(*function) == want => Some(args),
        _ => None,
    }
}

/// The pushed value of a one-loop integer map, classified: `(X[iv] * M) >> S` (requant), `min(max(X[iv], LO), HI)`
/// (clamp) and `max(X[iv], 0)` (ReLU over Int) or `max(X[iv], 0.0)` (over Real).
fn classify_int_map<'a>(value: &'a Expr<'a>, iv: Symbol, interner: &Interner) -> Option<(IntMap, Symbol)> {
    if let Some((prod, shift)) = as_binop(value, BinaryOpKind::Shr) {
        let s = u32::try_from(as_int(shift)?).ok().filter(|s| *s <= 63)?;
        let (xi, m) = as_binop(prod, BinaryOpKind::Multiply)?;
        return Some((IntMap::Requant { m: as_int(m)?, s }, read_at(xi, iv)?));
    }
    if let Some([lower, hi]) = call_of(value, interner, "min") {
        let [xi, lo] = call_of(lower, interner, "max")? else {
            return None;
        };
        let (lo, hi) = (as_int(lo)?, as_int(hi)?);
        want(lo <= hi)?;
        return Some((IntMap::Clamp { lo, hi }, read_at(xi, iv)?));
    }
    let [xi, zero] = call_of(value, interner, "max")? else {
        return None;
    };
    let int = if is_int(zero, 0) {
        true
    } else if is_float(zero, 0.0) {
        false
    } else {
        return None;
    };
    Some((IntMap::Relu { int }, read_at(xi, iv)?))
}

/// The one-loop map `Let Y be … X …` its sugar desugars to: the output's `New`, `Let p be 1`, then `While p ≤ n:
/// Push map(X[p]) to Y; Set p to p + 1`.
fn match_int_map<'a>(body: &'a [Stmt<'a>], interner: &Interner) -> Option<(IntMap, Symbol, Symbol, usize)> {
    for (i, out) in anchors(body) {
        let mut c = Cursor::new(body, i + 1);
        let Some(iv) = c.int_let(1) else { continue };
        let Some(l) = c.loop_(iv, true) else { continue };
        let Some((value, into)) = Cursor::new(l.body, 0).push() else {
            continue;
        };
        if into != out || l.body.len() != 2 {
            continue;
        }
        if let Some((map, x)) = classify_int_map(value, iv, interner) {
            return Some((map, x, out, l.bound as usize));
        }
    }
    None
}

fn recognize_int_map(stmts: &[Stmt], interner: &Interner) -> Option<(IntMap, Symbol, Symbol, usize)> {
    find_in_bodies(stmts, |b| match_int_map(b, interner))
}

/// Recognize `Let Y be X times M shifted right by S` — `Push (X[p] * M) shifted right by S` in one counted loop.
pub fn recognize_int_requant(stmts: &[Stmt], interner: &Interner) -> Option<IntRequantSpec> {
    match recognize_int_map(stmts, interner)? {
        (IntMap::Requant { m, s }, x, out, n) => Some(IntRequantSpec { x, out, n, m, s }),
        _ => None,
    }
}

/// Recognize `Let Y be X clamped between LO and HI` — `Push min(max(X[p], LO), HI)` in one counted loop.
pub fn recognize_int_clamp(stmts: &[Stmt], interner: &Interner) -> Option<IntClampSpec> {
    match recognize_int_map(stmts, interner)? {
        (IntMap::Clamp { lo, hi }, x, out, n) => Some(IntClampSpec { x, out, n, lo, hi }),
        _ => None,
    }
}

/// Recognize `Let Y be relu of X` — `Push max(X[p], 0)` in one counted loop, over Int or Real.
pub fn recognize_relu(stmts: &[Stmt], interner: &Interner) -> Option<ReluSpec> {
    match recognize_int_map(stmts, interner)? {
        (IntMap::Relu { int }, x, out, n) => Some(ReluSpec { x, out, n, int }),
        _ => None,
    }
}

// ── decay ──────────────────────────────────────────────────────────────────────────────────────────────

/// `Push Pow(Add(1, Pow(E, Ar[i] + Dt[i])), Alog[i])` → `(ar, dt, alog)`.
/// `(1 + E^(Ar[at] + Dt[i]))^Alog[i]` — `at` the element `Ar` is read at (`i` itself, or `row_item_at`'s). Returns
/// `(Ar, Dt, Alog)`.
fn decay_push(
    value: &Expr,
    iv: Symbol,
    ar_at: &dyn Fn(&Expr) -> bool,
) -> Option<(Symbol, Symbol, Symbol)> {
    let (base, rate) = as_binop(value, BinaryOpKind::Pow)?;
    let alog = read_at(rate, iv)?;
    let (one, pow) = as_binop(base, BinaryOpKind::Add)?;
    want(is_float(one, 1.0))?;
    let (e, u) = as_binop(pow, BinaryOpKind::Pow)?;
    want(is_e(e))?;
    let (ar_i, dt_i) = as_binop(u, BinaryOpKind::Add)?;
    let (ar, at) = indexed(ar_i)?;
    want(ar_at(at))?;
    Some((ar, read_at(dt_i, iv)?, alog))
}

/// `(row − 1)·width + i` — element `i` of row `row` of a block `width` wide. Returns `width`.
fn row_item_at(e: &Expr, row: Symbol, i: Symbol) -> Option<i64> {
    let (base, i_id) = as_binop(e, BinaryOpKind::Add)?;
    want(is_ident(i_id, i))?;
    let (row_less_one, width) = as_binop(base, BinaryOpKind::Multiply)?;
    let (r, one) = as_binop(row_less_one, BinaryOpKind::Subtract)?;
    want(is_ident(r, row) && is_int(one, 1))?;
    as_int(width)
}

fn match_decay<'a>(body: &'a [Stmt<'a>]) -> Option<DecaySpec> {
    for (i, out) in anchors(body) {
        let mut c = Cursor::new(body, i + 1);
        let Some(iv) = c.int_let(1) else { continue };
        let Some(l) = c.loop_(iv, true) else { continue };
        let mut lb = Cursor::new(l.body, 0);
        match lb.push() {
            // One row: `Push … Ar[i] …` over the row.
            Some((value, into)) => {
                if into != out {
                    continue;
                }
                if let Some((ar, dt, alog)) = decay_push(value, iv, &|at| is_ident(at, iv)) {
                    return Some(DecaySpec {
                        ar,
                        dt,
                        alog,
                        out,
                        n: l.bound as usize,
                        rows: 1,
                    });
                }
            }
            // A block: the row loop around the head loop, `Ar` read at `(row − 1)·heads + i`.
            None => {
                let spec = (|| {
                    let mut rb = Cursor::new(l.body, 0);
                    let hv = rb.int_let(1)?;
                    let heads = rb.loop_(hv, true)?;
                    let (inc, _) = rb.set()?;
                    want(inc == iv && rb.next().is_none())?;
                    let mut hb = Cursor::new(heads.body, 0);
                    let (value, into) = hb.push()?;
                    want(into == out)?;
                    let ar_at = |at: &Expr| row_item_at(at, iv, hv) == Some(heads.bound);
                    let (ar, dt, alog) = decay_push(value, hv, &ar_at)?;
                    Some(DecaySpec {
                        ar,
                        dt,
                        alog,
                        out,
                        n: (l.bound * heads.bound) as usize,
                        rows: l.bound as usize,
                    })
                })();
                if spec.is_some() {
                    return spec;
                }
            }
        }
    }
    None
}

/// Recognize `Let A be decay of Ar with bias Dt rate Alog` — `Push (1 + E^(Ar[i]+Dt[i]))^Alog[i]`.
pub fn recognize_decay(stmts: &[Stmt]) -> Option<DecaySpec> {
    find_in_bodies(stmts, match_decay)
}

// ── l2norm ─────────────────────────────────────────────────────────────────────────────────────────────

fn match_l2norm<'a>(body: &'a [Stmt<'a>]) -> Option<L2NormSpec> {
    for (i, out) in anchors(body) {
        let mut c = Cursor::new(body, i + 1);
        let Some(r) = c.int_let(1) else { continue };
        let Some(rows) = c.loop_(r, true) else {
            continue;
        };
        if let Some(spec) = l2norm_row(rows.body, r, out).map(|(x, cols, eps)| L2NormSpec {
            x,
            out,
            rows: rows.bound as usize,
            cols,
            eps: Epsilon::new(eps),
        }) {
            return Some(spec);
        }
    }
    None
}

/// One row of the l2norm desugar: `base = (r−1)·cols`, the sum of squares over `cols`, `denom = sqrt(ss + eps)`,
/// then the normalize loop `Push X[base+q] / denom`. Returns `(x, cols, eps)`.
fn l2norm_row<'a>(body: &'a [Stmt<'a>], r: Symbol, out: Symbol) -> Option<(Symbol, usize, f64)> {
    let mut c = Cursor::new(body, 0);
    let (base, base_v) = c.let_()?;
    let (r_minus, cols_e) = as_binop(base_v, BinaryOpKind::Multiply)?;
    let (r_id, one) = as_binop(r_minus, BinaryOpKind::Subtract)?;
    want(is_ident(r_id, r) && is_int(one, 1))?;
    let cols = as_int(cols_e)?;
    let ss = c.float_let(0.0)?;
    let p = c.int_let(1)?;
    let ss_loop = c.loop_(p, true)?;
    want(ss_loop.bound == cols)?;
    let (ss_target, ss_v) = Cursor::new(ss_loop.body, 0).set()?;
    want(ss_target == ss)?;
    let (ss_id, sq) = as_binop(ss_v, BinaryOpKind::Add)?;
    want(is_ident(ss_id, ss))?;
    let x = square_of(sq)?;
    let (denom, call) = c.let_()?;
    let Expr::Call { args, .. } = call else {
        return None;
    };
    let [arg] = args.as_slice() else { return None };
    let (ss_id, eps) = as_binop(arg, BinaryOpKind::Add)?;
    want(is_ident(ss_id, ss))?;
    let eps = as_float(eps).filter(|e| *e > 0.0)?;
    let q = c.int_let(1)?;
    let n_loop = c.loop_(q, true)?;
    want(n_loop.bound == cols)?;
    let (value, into) = Cursor::new(n_loop.body, 0).push()?;
    want(into == out)?;
    let (xq, den) = as_binop(value, BinaryOpKind::Divide)?;
    want(is_ident(den, denom))?;
    let (x2, idx) = indexed(xq)?;
    let (base_id, q_id) = as_binop(idx, BinaryOpKind::Add)?;
    want(x2 == x && is_ident(base_id, base) && is_ident(q_id, q))?;
    Some((x, cols as usize, eps))
}

/// Recognize `Let Y be l2norm of X [across H heads]` — the per-row `sqrt` denominator bound in a `Let` and
/// the `X[base+q] / denom` normalize loop.
pub fn recognize_l2norm(stmts: &[Stmt]) -> Option<L2NormSpec> {
    find_in_bodies(stmts, match_l2norm)
}

// ── transpose ──────────────────────────────────────────────────────────────────────────────────────────

fn match_transpose<'a>(body: &'a [Stmt<'a>]) -> Option<TransposeSpec> {
    for (i, out) in anchors(body) {
        let mut c = Cursor::new(body, i + 1);
        let Some(tj) = c.int_let(0) else { continue };
        let Some(outer) = c.loop_(tj, false) else {
            continue;
        };
        let cols = outer.bound;
        let mut ob = Cursor::new(outer.body, 0);
        let Some(ti) = ob.int_let(0) else { continue };
        let Some(inner) = ob.loop_(ti, false) else {
            continue;
        };
        let Some((value, into)) = Cursor::new(inner.body, 0).push() else {
            continue;
        };
        if into != out {
            continue;
        }
        let Some((x, idx)) = indexed(value) else {
            continue;
        };
        // X[ti·cols + tj + 1]
        let shape = (|| {
            let (sum, one) = as_binop(idx, BinaryOpKind::Add)?;
            want(is_int(one, 1))?;
            let (prod, tj_id) = as_binop(sum, BinaryOpKind::Add)?;
            want(is_ident(tj_id, tj))?;
            let (ti_id, cols_e) = as_binop(prod, BinaryOpKind::Multiply)?;
            want(is_ident(ti_id, ti) && is_int(cols_e, cols))
        })();
        if shape.is_some() {
            return Some(TransposeSpec {
                x,
                out,
                rows: inner.bound as usize,
                cols: cols as usize,
            });
        }
    }
    None
}

/// Recognize `Let Kt be transpose of X` — the `cols`-outer / `rows`-inner loop nest pushing `X[ti·cols+tj+1]`.
pub fn recognize_transpose(stmts: &[Stmt]) -> Option<TransposeSpec> {
    find_in_bodies(stmts, match_transpose)
}

// ── head slices and joins ──────────────────────────────────────────────────────────────────────────────

/// `Let iv be 1. While iv ≤ width: Push <value> to out. Set iv to iv + 1.` — one part's copy: `(value, iv, width)`.
fn part_copy<'a>(c: &mut Cursor<'a>, out: Symbol) -> Option<(&'a Expr<'a>, Symbol, i64)> {
    let iv = c.int_let(1)?;
    let l = c.loop_(iv, true)?;
    let mut b = Cursor::new(l.body, 0);
    let (value, into) = b.push()?;
    want(into == out && matches!(b.set(), Some((inc, _)) if inc == iv) && b.next().is_none() && l.bound >= 1)?;
    Some((value, iv, l.bound))
}

/// `T[(r·stride + h·width) + iv]` — element `iv` (1-based) of head `h`'s part, `width` wide, of row `r`. Returns `T`.
fn part_read(e: &Expr, r: Symbol, stride: i64, h: Symbol, width: i64, iv: Symbol) -> Option<Symbol> {
    let (t, idx) = indexed(e)?;
    let (at, iv_id) = as_binop(idx, BinaryOpKind::Add)?;
    let (row, part) = as_binop(at, BinaryOpKind::Add)?;
    let (r_id, s) = as_binop(row, BinaryOpKind::Multiply)?;
    let (h_id, w) = as_binop(part, BinaryOpKind::Multiply)?;
    want(is_ident(iv_id, iv) && is_ident(r_id, r) && is_int(s, stride) && is_ident(h_id, h) && is_int(w, width))?;
    Some(t)
}

fn match_head_slice<'a>(body: &'a [Stmt<'a>]) -> Option<HeadSliceSpec> {
    for (i, out) in anchors(body) {
        let found = (|| {
            let mut c = Cursor::new(body, i + 1);
            let g = c.int_let(0)?;
            let groups = c.loop_(g, false)?;
            let mut gb = Cursor::new(groups.body, 0);
            let (value, iv, width) = part_copy(&mut gb, out)?;
            want(matches!(gb.set(), Some((inc, _)) if inc == g) && gb.next().is_none() && groups.bound >= 1)?;
            // `X[(g·N + iv) + from]`: the group's `N` elements, `from` past their first.
            let (x, idx) = indexed(value)?;
            let (at, from) = as_binop(idx, BinaryOpKind::Add)?;
            let (row, iv_id) = as_binop(at, BinaryOpKind::Add)?;
            let (g_id, every) = as_binop(row, BinaryOpKind::Multiply)?;
            want(is_ident(iv_id, iv) && is_ident(g_id, g))?;
            let (every, from) = (as_int(every)?, as_int(from)?);
            want(from >= 0 && from + width <= every)?;
            Some(HeadSliceSpec {
                x,
                out,
                every: every as usize,
                from: from as usize,
                width: width as usize,
                n: (groups.bound * every) as usize,
            })
        })();
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Recognize `Let P be the first K of every N of X` / `the last K of every N of X` / `the K after the first F of every N
/// of X` — a loop over the groups, each pushing its `K` elements `X[(g·N + i) + from]`.
pub fn recognize_head_slice(stmts: &[Stmt]) -> Option<HeadSliceSpec> {
    find_in_bodies(stmts, match_head_slice)
}

fn match_join<'a>(body: &'a [Stmt<'a>]) -> Option<JoinSpec> {
    for (i, out) in anchors(body) {
        let found = (|| {
            let mut c = Cursor::new(body, i + 1);
            let r = c.int_let(0)?;
            let rows = c.loop_(r, false)?;
            let mut rb = Cursor::new(rows.body, 0);
            let h = rb.int_let(0)?;
            let heads = rb.loop_(h, false)?;
            want(matches!(rb.set(), Some((inc, _)) if inc == r) && rb.next().is_none())?;
            // Each head: its part of `A`, then its part of `B`, each read at its row and head.
            let mut hb = Cursor::new(heads.body, 0);
            let (va, ia, ha) = part_copy(&mut hb, out)?;
            let (vb, ib, hw) = part_copy(&mut hb, out)?;
            want(matches!(hb.set(), Some((inc, _)) if inc == h) && hb.next().is_none())?;
            let a = part_read(va, r, ha * heads.bound, h, ha, ia)?;
            let b = part_read(vb, r, hw * heads.bound, h, hw, ib)?;
            want(rows.bound >= 1 && heads.bound >= 1)?;
            Some(JoinSpec {
                a,
                b,
                out,
                rows: rows.bound as usize,
                heads: heads.bound as usize,
                wa: (ha * heads.bound) as usize,
                wb: (hw * heads.bound) as usize,
            })
        })();
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Recognize `Let J be A joined with B [across H heads]` — a loop over the rows and in it a loop over the heads, each
/// head pushing its part of `A` (`A[(r·wa + h·ha) + i]`) then its part of `B`.
pub fn recognize_join(stmts: &[Stmt]) -> Option<JoinSpec> {
    find_in_bodies(stmts, match_join)
}

// ── a hyper-connected residual's lanes ─────────────────────────────────────────────────────────────────

fn match_lane_spread<'a>(body: &'a [Stmt<'a>]) -> Option<LaneSpreadSpec> {
    for (i, out) in anchors(body) {
        let found = (|| {
            let mut c = Cursor::new(body, i + 1);
            let t = c.int_let(0)?;
            let rows = c.loop_(t, false)?;
            let mut rb = Cursor::new(rows.body, 0);
            let l = rb.int_let(0)?;
            let lanes = rb.loop_(l, false)?;
            want(matches!(rb.set(), Some((inc, _)) if inc == t) && rb.next().is_none())?;
            let mut lb = Cursor::new(lanes.body, 0);
            let (value, iv, cols) = part_copy(&mut lb, out)?;
            want(matches!(lb.set(), Some((inc, _)) if inc == l) && lb.next().is_none())?;
            // `O[t·C + i]`, times `S[(t·L + l) + 1]` with `by S`.
            let (read, weight) = match as_binop(value, BinaryOpKind::Multiply) {
                Some((read, weight)) => (read, Some(weight)),
                None => (value, None),
            };
            let (x, at) = indexed(read)?;
            let (row, iv_id) = as_binop(at, BinaryOpKind::Add)?;
            let (t_id, width) = as_binop(row, BinaryOpKind::Multiply)?;
            want(is_ident(t_id, t) && is_int(width, cols) && is_ident(iv_id, iv))?;
            let by = match weight {
                Some(w) => {
                    let (s, at) = indexed(w)?;
                    let (lane, one) = as_binop(at, BinaryOpKind::Add)?;
                    let (row, l_id) = as_binop(lane, BinaryOpKind::Add)?;
                    let (t_id, stride) = as_binop(row, BinaryOpKind::Multiply)?;
                    want(is_int(one, 1) && is_ident(l_id, l) && is_ident(t_id, t) && is_int(stride, lanes.bound))?;
                    Some(s)
                }
                None => None,
            };
            want(rows.bound >= 1 && lanes.bound >= 1)?;
            Some(LaneSpreadSpec { x, by, out, rows: rows.bound as usize, lanes: lanes.bound as usize, cols: cols as usize })
        })();
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Recognize `Let P be O spread over L lanes [by S]` — a loop over the rows, in it a loop over the lanes, each lane
/// pushing the row of `O` (`O[t·C + i]`), times the lane's weight `S[(t·L + l) + 1]` with `by S`.
pub fn recognize_lane_spread(stmts: &[Stmt]) -> Option<LaneSpreadSpec> {
    find_in_bodies(stmts, match_lane_spread)
}

fn match_lane_mean<'a>(body: &'a [Stmt<'a>]) -> Option<LaneMeanSpec> {
    for (i, out) in anchors(body) {
        let found = (|| {
            let mut c = Cursor::new(body, i + 1);
            let s = c.float_let(0.0)?;
            let t = c.int_let(0)?;
            let rows = c.loop_(t, false)?;
            want(c.next().is_none())?;
            let mut rb = Cursor::new(rows.body, 0);
            let col = rb.int_let(0)?;
            let cols = rb.loop_(col, false)?;
            want(matches!(rb.set(), Some((inc, _)) if inc == t) && rb.next().is_none())?;
            // Each column: `s = 0`, the sum over its lanes, then `s / L` pushed.
            let mut cb = Cursor::new(cols.body, 0);
            want(matches!(cb.set(), Some((z, zero)) if z == s && is_float(zero, 0.0)))?;
            let l = cb.int_let(0)?;
            let lanes = cb.loop_(l, false)?;
            let (value, into) = cb.push()?;
            want(into == out && matches!(cb.set(), Some((inc, _)) if inc == col) && cb.next().is_none())?;
            let (sum, by) = as_binop(value, BinaryOpKind::Divide)?;
            want(is_ident(sum, s) && is_float(by, lanes.bound as f64))?;
            // `s = s + X[((t·(L·C) + l·C) + c) + 1]`, then the lane counter.
            let mut lb = Cursor::new(lanes.body, 0);
            let (acc, value) = lb.set()?;
            want(acc == s && matches!(lb.set(), Some((inc, _)) if inc == l) && lb.next().is_none())?;
            let (prior, read) = as_binop(value, BinaryOpKind::Add)?;
            want(is_ident(prior, s))?;
            let (x, at) = indexed(read)?;
            let (at, one) = as_binop(at, BinaryOpKind::Add)?;
            let (at, c_id) = as_binop(at, BinaryOpKind::Add)?;
            let (row, lane) = as_binop(at, BinaryOpKind::Add)?;
            let (t_id, row_width) = as_binop(row, BinaryOpKind::Multiply)?;
            let (l_id, width) = as_binop(lane, BinaryOpKind::Multiply)?;
            want(
                is_int(one, 1)
                    && is_ident(c_id, col)
                    && is_ident(t_id, t)
                    && is_int(row_width, lanes.bound * cols.bound)
                    && is_ident(l_id, l)
                    && is_int(width, cols.bound),
            )?;
            want(rows.bound >= 1 && lanes.bound >= 1 && cols.bound >= 1)?;
            Some(LaneMeanSpec { x, out, rows: rows.bound as usize, lanes: lanes.bound as usize, cols: cols.bound as usize })
        })();
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Recognize `Let M be the mean of X over L lanes` — a loop over the rows, in it a loop over the columns, each column
/// summing its lanes `X[((t·(L·C) + l·C) + c) + 1]` and pushing the sum over `L`.
pub fn recognize_lane_mean(stmts: &[Stmt]) -> Option<LaneMeanSpec> {
    find_in_bodies(stmts, match_lane_mean)
}

fn match_lane_sums<'a>(body: &'a [Stmt<'a>]) -> Option<LaneSumsSpec> {
    for (i, out) in anchors(body) {
        let found = (|| {
            let mut c = Cursor::new(body, i + 1);
            let s = c.float_let(0.0)?;
            let t = c.int_let(0)?;
            let rows = c.loop_(t, false)?;
            want(c.next().is_none())?;
            let mut rb = Cursor::new(rows.body, 0);
            let l = rb.int_let(0)?;
            let lanes = rb.loop_(l, false)?;
            want(matches!(rb.set(), Some((inc, _)) if inc == t) && rb.next().is_none())?;
            // Each lane: `s = 0`, the sum over its columns, then `s` pushed.
            let mut lb = Cursor::new(lanes.body, 0);
            want(matches!(lb.set(), Some((z, zero)) if z == s && is_float(zero, 0.0)))?;
            let col = lb.int_let(0)?;
            let cols = lb.loop_(col, false)?;
            let (value, into) = lb.push()?;
            want(into == out && is_ident(value, s) && matches!(lb.set(), Some((inc, _)) if inc == l) && lb.next().is_none())?;
            // `s = s + X[((t·(L·C) + l·C) + c) + 1]`, then the column counter.
            let mut cb = Cursor::new(cols.body, 0);
            let (acc, value) = cb.set()?;
            want(acc == s && matches!(cb.set(), Some((inc, _)) if inc == col) && cb.next().is_none())?;
            let (prior, read) = as_binop(value, BinaryOpKind::Add)?;
            want(is_ident(prior, s))?;
            let (x, at) = indexed(read)?;
            let (at, one) = as_binop(at, BinaryOpKind::Add)?;
            let (at, c_id) = as_binop(at, BinaryOpKind::Add)?;
            let (row, lane) = as_binop(at, BinaryOpKind::Add)?;
            let (t_id, row_width) = as_binop(row, BinaryOpKind::Multiply)?;
            let (l_id, width) = as_binop(lane, BinaryOpKind::Multiply)?;
            want(
                is_int(one, 1)
                    && is_ident(c_id, col)
                    && is_ident(t_id, t)
                    && is_int(row_width, lanes.bound * cols.bound)
                    && is_ident(l_id, l)
                    && is_int(width, cols.bound),
            )?;
            want(rows.bound >= 1 && lanes.bound >= 1 && cols.bound >= 1)?;
            Some(LaneSumsSpec { x, out, rows: rows.bound as usize, lanes: lanes.bound as usize, cols: cols.bound as usize })
        })();
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Recognize `Let G be the lane sums of X over L lanes` — a loop over the rows, in it a loop over the lanes, each lane
/// summing its columns `X[((t·(L·C) + l·C) + c) + 1]` and pushing the sum.
pub fn recognize_lane_sums(stmts: &[Stmt]) -> Option<LaneSumsSpec> {
    find_in_bodies(stmts, match_lane_sums)
}

/// `Set g to <value>` alone in a block: the value.
fn sets_only<'a>(block: &'a [Stmt<'a>], g: Symbol) -> Option<&'a Expr<'a>> {
    match block {
        [Stmt::Set { target, value }] if *target == g => Some(*value),
        _ => None,
    }
}

fn match_signed_root<'a>(body: &'a [Stmt<'a>], interner: &Interner) -> Option<SignedRootSpec> {
    let call = |e: &'a Expr<'a>, name: &str| call_of(e, interner, name);
    for (i, out) in anchors(body) {
        let found = (|| {
            let mut c = Cursor::new(body, i + 1);
            let v = c.float_let(0.0)?;
            let g = c.float_let(0.0)?;
            let p = c.int_let(1)?;
            let elems = c.loop_(p, true)?;
            want(c.next().is_none())?;
            let mut b = Cursor::new(elems.body, 0);
            let (vv, read) = b.set()?;
            want(vv == v)?;
            let (x, at) = indexed(read)?;
            want(is_ident(at, p))?;
            // The sign: 1 above zero, −1 below, 0 at it.
            let Some(Stmt::If { cond, then_block, else_block: Some(otherwise) }) = b.next() else { return None };
            let (cv, zero) = as_binop(cond, BinaryOpKind::Gt)?;
            want(is_ident(cv, v) && is_float(zero, 0.0) && sets_only(then_block, g).is_some_and(|e| is_float(e, 1.0)))?;
            let [Stmt::If { cond, then_block, else_block: Some(at_zero) }] = otherwise else { return None };
            let (cv, zero) = as_binop(cond, BinaryOpKind::Lt)?;
            want(is_ident(cv, v) && is_float(zero, 0.0))?;
            want(sets_only(then_block, g).is_some_and(|e| is_float(e, -1.0)) && sets_only(at_zero, g).is_some_and(|e| is_float(e, 0.0)))?;
            // `g · sqrt(max(abs(v), eps))`, pushed, then the counter.
            let (value, into) = b.push()?;
            let (gs, root) = as_binop(value, BinaryOpKind::Multiply)?;
            want(into == out && is_ident(gs, g))?;
            let [inner] = call(root, "sqrt")? else { return None };
            let [magnitude, eps] = call(inner, "max")? else { return None };
            let [arg] = call(magnitude, "abs")? else { return None };
            want(is_ident(arg, v))?;
            let eps = as_float(eps).filter(|e| *e > 0.0)?;
            want(matches!(b.set(), Some((inc, _)) if inc == p) && b.next().is_none() && elems.bound >= 1)?;
            Some(SignedRootSpec { x, out, n: elems.bound as usize, eps: super::Epsilon::new(eps) })
        })();
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Recognize `Let Y be the signed root of X` — a loop over the elements, each taking its sign by two comparisons and
/// pushing the sign times `sqrt(max(abs(v), E))`.
pub fn recognize_signed_root(stmts: &[Stmt], interner: &Interner) -> Option<SignedRootSpec> {
    find_in_bodies(stmts, |b| match_signed_root(b, interner))
}

/// `If <ident> == <int>: Set <target> to <value>` with no else: `(ident, int, target, value)`.
fn if_equal_sets<'a>(s: &'a Stmt<'a>) -> Option<(Symbol, i64, Symbol, &'a Expr<'a>)> {
    let Stmt::If { cond, then_block, else_block: None } = s else { return None };
    let (l, r) = as_binop(cond, BinaryOpKind::Eq)?;
    let [Stmt::Set { target, value }] = then_block else { return None };
    Some((as_ident(l)?, as_int(r)?, *target, *value))
}

/// `v − 1`.
fn less_one(e: &Expr, v: Symbol) -> bool {
    as_binop(e, BinaryOpKind::Subtract).is_some_and(|(a, one)| is_ident(a, v) && is_int(one, 1))
}

/// `((k · per) + h) + 1` → `per`.
fn head_at(e: &Expr, k: Symbol, h: Symbol) -> Option<i64> {
    let (at, one) = as_binop(e, BinaryOpKind::Add)?;
    let (kp, hh) = as_binop(at, BinaryOpKind::Add)?;
    let (kk, per) = as_binop(kp, BinaryOpKind::Multiply)?;
    want(is_int(one, 1) && is_ident(hh, h) && is_ident(kk, k))?;
    as_int(per)
}

fn match_ngram_ids<'a>(body: &'a [Stmt<'a>]) -> Option<NgramIdsSpec> {
    for (i, out) in anchors(body) {
        let found = (|| {
            let mut c = Cursor::new(body, i + 1);
            let (x, b, seen, cur) = (c.int_let(0)?, c.int_let(0)?, c.int_let(0)?, c.int_let(0)?);
            let ti = c.int_let(0)?;
            let tokens = c.loop_(ti, false)?;
            // Each token row: `cur = T[ti + 1]`, `seen = 0`, `x = (cur − 1) · M[1]`.
            let mut tb = Cursor::new(tokens.body, 0);
            let (cv, read) = tb.set()?;
            want(cv == cur)?;
            // A token tensor read at `ti + 1`, or one token's row in a variable.
            let tokens_in = match indexed(read) {
                Some((t, at)) => {
                    let (at_i, one) = as_binop(at, BinaryOpKind::Add)?;
                    want(is_ident(at_i, ti) && is_int(one, 1))?;
                    t
                }
                None => {
                    want(tokens.bound == 1)?;
                    as_ident(read)?
                }
            };
            want(matches!(tb.set(), Some((sv, zero)) if sv == seen && is_int(zero, 0)))?;
            let (xv, first) = tb.set()?;
            let (cur_less, m1) = as_binop(first, BinaryOpKind::Multiply)?;
            let (multipliers, m_at) = indexed(m1)?;
            want(xv == x && less_one(cur_less, cur) && is_int(m_at, 1))?;
            // Each place `k` back: the row there — `E` where none was, and once an `E` lies between — less one, times
            // `M[k + 2]`, exclusive-ored in.
            let k = tb.int_let(0)?;
            let places = tb.loop_(k, false)?;
            let mut pb = Cursor::new(places.body, 0);
            let (bv, back) = pb.set()?;
            let (history, back_at) = indexed(back)?;
            let (last, kk) = as_binop(back_at, BinaryOpKind::Subtract)?;
            want(bv == b && is_ident(kk, k) && is_int(last, places.bound))?;
            let (b0, zero, b0v, eos_e) = if_equal_sets(pb.next()?)?;
            let eos = as_int(eos_e)?;
            want(b0 == b && zero == 0 && b0v == b && eos >= 1)?;
            let (bb, at_eos, sv, one) = if_equal_sets(pb.next()?)?;
            want(bb == b && at_eos == eos && sv == seen && is_int(one, 1))?;
            let (ss, one, bv2, to_eos) = if_equal_sets(pb.next()?)?;
            want(ss == seen && one == 1 && bv2 == b && is_int(to_eos, eos))?;
            let (xv, mixed) = pb.set()?;
            let (prior, term) = as_binop(mixed, BinaryOpKind::BitXor)?;
            let (b_less, mk) = as_binop(term, BinaryOpKind::Multiply)?;
            let (mm, mk_at) = indexed(mk)?;
            let (k_id, two) = as_binop(mk_at, BinaryOpKind::Add)?;
            want(xv == x && is_ident(prior, x) && less_one(b_less, b) && mm == multipliers && is_ident(k_id, k) && is_int(two, 2))?;
            // Each head of the order: the table row `((x mod V[head]) + O[head]) + 1` pushed.
            let h = pb.int_let(0)?;
            let heads = pb.loop_(h, false)?;
            want(matches!(pb.set(), Some((inc, _)) if inc == k) && pb.next().is_none())?;
            let mut hb = Cursor::new(heads.body, 0);
            let (id, into) = hb.push()?;
            want(into == out && matches!(hb.set(), Some((inc, _)) if inc == h) && hb.next().is_none())?;
            let (id, one) = as_binop(id, BinaryOpKind::Add)?;
            want(is_int(one, 1))?;
            let (modded, offset) = as_binop(id, BinaryOpKind::Add)?;
            let (x_id, modulus) = as_binop(modded, BinaryOpKind::Modulo)?;
            let (moduli, v_at) = indexed(modulus)?;
            let (offsets, o_at) = indexed(offset)?;
            want(is_ident(x_id, x) && head_at(v_at, k, h) == Some(heads.bound) && head_at(o_at, k, h) == Some(heads.bound))?;
            // The history shifts left and takes the token: `R[s + 1] = R[s + 2]`, then `R[N − 1] = cur`.
            let s = tb.int_let(0)?;
            let shift = tb.loop_(s, false)?;
            want(shift.bound == places.bound - 1)?;
            let mut sb = Cursor::new(shift.body, 0);
            let (rr, to, from) = sb.set_index()?;
            let (rr2, from_at) = indexed(from)?;
            let (to_s, to_one) = as_binop(to, BinaryOpKind::Add)?;
            let (from_s, from_two) = as_binop(from_at, BinaryOpKind::Add)?;
            want(rr == history && rr2 == history && is_ident(to_s, s) && is_int(to_one, 1) && is_ident(from_s, s) && is_int(from_two, 2))?;
            want(matches!(sb.set(), Some((inc, _)) if inc == s) && sb.next().is_none())?;
            let (rr, newest, value) = tb.set_index()?;
            want(rr == history && is_int(newest, places.bound) && is_ident(value, cur))?;
            want(matches!(tb.set(), Some((inc, _)) if inc == ti) && tb.next().is_none() && c.next().is_none())?;
            want(tokens.bound >= 1 && places.bound >= 1 && heads.bound >= 1)?;
            Some(NgramIdsSpec {
                tokens_in,
                history,
                multipliers,
                moduli,
                offsets,
                out,
                tokens: tokens.bound as usize,
                ngram: places.bound as usize + 1,
                per_order: heads.bound as usize,
                eos,
            })
        })();
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Recognize `Let I be the ngram ids of T after R by M modulo V from O ending at E` — a loop over the tokens, in it a
/// loop over the places back folding each place's product into the hash and pushing the order's heads' ids, then the
/// history's shift.
pub fn recognize_ngram_ids(stmts: &[Stmt]) -> Option<NgramIdsSpec> {
    find_in_bodies(stmts, match_ngram_ids)
}

// ── the row sugars: gather, arg-max, scatter ───────────────────────────────────────────────────────────

/// `Let at be <index>. Require that at ≥ 1 and at ≤ rows.` — the checked row index every row sugar opens
/// with. Returns `(at, index, rows)`.
fn checked_row_index<'a>(c: &mut Cursor<'a>) -> Option<(Symbol, RowIndex, usize)> {
    let (at, value) = c.let_()?;
    let index = match value {
        Expr::Identifier(s) => RowIndex::Named(*s),
        Expr::Literal(Literal::Number(n)) => RowIndex::Literal(*n),
        _ => return None,
    };
    let Stmt::RuntimeAssert {
        condition,
        hard: true,
        ..
    } = c.next()?
    else {
        return None;
    };
    let (lo, hi) = as_binop(condition, BinaryOpKind::And)?;
    let (lo_v, one) = as_binop(lo, BinaryOpKind::GtEq)?;
    let (hi_v, rows) = as_binop(hi, BinaryOpKind::LtEq)?;
    want(is_ident(lo_v, at) && is_int(one, 1) && is_ident(hi_v, at))?;
    let rows = as_int(rows)?;
    want(rows >= 1)?;
    Some((at, index, rows as usize))
}

/// `((at − 1)·cols + j) + 1` — the flat item of element `j` of row `at`. Returns `cols`.
fn row_item(e: &Expr, at: Symbol, j: Symbol) -> Option<i64> {
    let (sum, one) = as_binop(e, BinaryOpKind::Add)?;
    want(is_int(one, 1))?;
    let (base, j_id) = as_binop(sum, BinaryOpKind::Add)?;
    want(is_ident(j_id, j))?;
    let (at_less_one, cols) = as_binop(base, BinaryOpKind::Multiply)?;
    let (at_id, one) = as_binop(at_less_one, BinaryOpKind::Subtract)?;
    want(is_ident(at_id, at) && is_int(one, 1))?;
    as_int(cols)
}

fn match_row_gather<'a>(body: &'a [Stmt<'a>]) -> Option<RowGatherSpec> {
    let mut c = Cursor::new(body, 0);
    let (at, index, rows) = checked_row_index(&mut c)?;
    let out = c.new_let()?;
    let j = c.int_let(0)?;
    let l = c.loop_(j, false)?;
    want(c.next().is_none())?;
    let mut lb = Cursor::new(l.body, 0);
    let (value, into) = lb.push()?;
    want(into == out)?;
    let (table, item) = indexed(value)?;
    let cols = row_item(item, at, j)?;
    want(cols == l.bound && cols >= 1)?;
    let (inc, _) = lb.set()?;
    want(inc == j && lb.next().is_none())?;
    Some(RowGatherSpec {
        table,
        index,
        out,
        rows,
        cols: cols as usize,
    })
}

/// Recognize `Let x be row i of T`.
pub fn recognize_row_gather(stmts: &[Stmt]) -> Option<RowGatherSpec> {
    find_in_bodies(stmts, match_row_gather)
}

fn match_rows_gather<'a>(body: &'a [Stmt<'a>]) -> Option<RowsGatherSpec> {
    let mut c = Cursor::new(body, 0);
    let out = c.new_let()?;
    let k = c.int_let(1)?;
    let outer = c.loop_(k, true)?;
    want(c.next().is_none())?;
    // Per index: `Let at be I[k]`, its range check, then the row's copy.
    let mut rb = Cursor::new(outer.body, 0);
    let (at, value) = rb.let_()?;
    let (indices, k_at) = indexed(value)?;
    want(is_ident(k_at, k))?;
    let Stmt::RuntimeAssert {
        condition,
        hard: true,
        ..
    } = rb.next()?
    else {
        return None;
    };
    let (lo, hi) = as_binop(condition, BinaryOpKind::And)?;
    let (lo_v, one) = as_binop(lo, BinaryOpKind::GtEq)?;
    let (hi_v, rows) = as_binop(hi, BinaryOpKind::LtEq)?;
    want(is_ident(lo_v, at) && is_int(one, 1) && is_ident(hi_v, at))?;
    let rows = as_int(rows)?;
    let j = rb.int_let(0)?;
    let copy = rb.loop_(j, false)?;
    let mut cb = Cursor::new(copy.body, 0);
    let (read, into) = cb.push()?;
    want(into == out)?;
    let (table, item) = indexed(read)?;
    let cols = row_item(item, at, j)?;
    want(cols == copy.bound && cols >= 1 && rows >= 1 && table != indices)?;
    let (inc_j, _) = cb.set()?;
    want(inc_j == j && cb.next().is_none())?;
    let (inc_k, _) = rb.set()?;
    want(inc_k == k && rb.next().is_none())?;
    Some(RowsGatherSpec {
        table,
        indices,
        out,
        rows: rows as usize,
        cols: cols as usize,
        count: outer.bound as usize,
    })
}

/// Recognize `Let X be rows I of T`.
pub fn recognize_rows_gather(stmts: &[Stmt]) -> Option<RowsGatherSpec> {
    find_in_bodies(stmts, match_rows_gather)
}

fn match_set_row<'a>(body: &'a [Stmt<'a>]) -> Option<SetRowSpec> {
    let mut c = Cursor::new(body, 0);
    let (at, index, rows) = checked_row_index(&mut c)?;
    let j = c.int_let(0)?;
    let l = c.loop_(j, false)?;
    want(c.next().is_none())?;
    let mut lb = Cursor::new(l.body, 0);
    let (table, item, value) = lb.set_index()?;
    // The item's `(at − 1)·cols` is the table's row width; the loop writes `count` whole rows from `at` on, and the
    // check keeps the last of them in the table (`at ≤ rows − count + 1`).
    let cols = row_item(item, at, j)?;
    want(cols >= 1 && l.bound >= cols && l.bound % cols == 0)?;
    let count = l.bound / cols;
    let (source, at_j) = indexed(value)?;
    let (j_id, one) = as_binop(at_j, BinaryOpKind::Add)?;
    want(is_ident(j_id, j) && is_int(one, 1) && source != table)?;
    let (inc, _) = lb.set()?;
    want(inc == j && lb.next().is_none())?;
    Some(SetRowSpec {
        table,
        index,
        source,
        rows: rows + count as usize - 1,
        cols: cols as usize,
        count: count as usize,
    })
}

/// Recognize `Set row i of T to S`.
pub fn recognize_set_row(stmts: &[Stmt]) -> Option<SetRowSpec> {
    find_in_bodies(stmts, match_set_row)
}

/// The scatter a counted loop the program wrote means, when its turn is exactly a scatter's ([`ScatterSpec`]): `Repeat
/// for k from 1 to N` whose turn reads its index `at = I[k]` and the source row `r = row k of X` (in either order), then
/// — folding — reads `old = row at of T` and takes `sum = old plus r` (either way round), and writes row `at` of `T` last.
/// `len_of` is the length of a tensor the program has bound before the loop, so a turn past the index tensor's end, which
/// fails on every tier, is no scatter. Whatever else a turn does — another statement, another name, a row read twice —
/// is not one either.
///
/// ★ THE LOOP IS LIFTED BY WHAT IT MEANS, NEVER BY AN OP IT HOLDS. Each turn's row read, sum and row write are ops of
/// their own, read once a turn; the loop's meaning is the scatter they make together over every turn.
pub fn recognize_scatter(
    s: &Stmt,
    interner: &Interner,
    len_of: &dyn Fn(Symbol) -> Option<usize>,
) -> Option<ScatterSpec> {
    use logicaffeine_language::ast::stmt::Pattern;
    let Stmt::Repeat { pattern: Pattern::Identifier(k), iterable, body } = s else {
        return None;
    };
    let Expr::Range { start, end } = iterable else {
        return None;
    };
    want(is_int(start, 1))?;
    let count = as_int(end).filter(|n| *n >= 1)? as usize;
    let k = *k;
    let (write, reads) = body.split_last()?;
    let Stmt::Splice { body: write } = write else {
        return None;
    };
    let set = match_set_row(write)?;
    let RowIndex::Named(at) = set.index else {
        return None;
    };
    want(set.count == 1)?;
    let (mut index, mut row, mut old, mut sum) = (None, None, None, None);
    for st in reads {
        match st {
            Stmt::Let { var, value, .. } if *var == at && index.is_none() => {
                let (i, by) = indexed(value)?;
                want(is_ident(by, k))?;
                index = Some(i);
            }
            Stmt::Splice { body } => match (match_row_gather(body), super::recognize_elementwise(body, interner)) {
                (Some(g), _) if g.index == RowIndex::Named(k) && row.is_none() => row = Some(g),
                (Some(g), _) if g.index == RowIndex::Named(at) && index.is_some() && old.is_none() => old = Some(g),
                (None, Ok(e)) if row.is_some() && old.is_some() && sum.is_none() => sum = Some(e),
                _ => return None,
            },
            _ => return None,
        }
    }
    let (indices, row) = (index?, row?);
    let (table, source) = (set.table, row.table);
    want(set.cols == row.cols && table != source && indices != table && indices != source)?;
    let r = row.out;
    let fold = match (old, sum) {
        (None, None) => {
            want(set.source == r)?;
            None
        }
        (Some(old), Some(sum)) => {
            want(old.table == table && old.rows == set.rows && old.cols == set.cols)?;
            let operands = [sum.a, sum.b];
            want(sum.row.is_none() && sum.n == set.cols && sum.c == set.source)?;
            want(operands.contains(&old.out) && operands.contains(&r) && old.out != r)?;
            Some(sum.op)
        }
        _ => return None,
    };
    let index_len = len_of(indices)?;
    want(count <= row.rows && count <= index_len)?;
    Some(ScatterSpec {
        table,
        indices,
        source,
        rows: set.rows,
        cols: set.cols,
        source_rows: row.rows,
        index_len,
        count,
        fold,
    })
}

fn match_argmax<'a>(body: &'a [Stmt<'a>]) -> Option<ArgMaxSpec> {
    let mut c = Cursor::new(body, 0);
    let (best, first) = c.let_()?;
    let (x, one) = indexed(first)?;
    want(is_int(one, 1))?;
    let at = c.int_let(1)?;
    let i = c.int_let(2)?;
    let l = c.loop_(i, true)?;
    let (out, result) = c.let_()?;
    want(is_ident(result, at) && c.next().is_none())?;
    let mut lb = Cursor::new(l.body, 0);
    let Stmt::If {
        cond,
        then_block,
        else_block: None,
    } = lb.next()?
    else {
        return None;
    };
    let (read, cur) = as_binop(cond, BinaryOpKind::Gt)?;
    want(read_at(read, i) == Some(x) && is_ident(cur, best))?;
    let mut tb = Cursor::new(then_block, 0);
    let (b, bv) = tb.set()?;
    want(b == best && read_at(bv, i) == Some(x))?;
    let (a, av) = tb.set()?;
    want(a == at && is_ident(av, i) && tb.next().is_none())?;
    let (inc, _) = lb.set()?;
    want(inc == i && lb.next().is_none() && l.bound >= 1)?;
    Some(ArgMaxSpec {
        x,
        out,
        n: l.bound as usize,
    })
}

/// Recognize `Let t be argmax of X`.
pub fn recognize_argmax(stmts: &[Stmt]) -> Option<ArgMaxSpec> {
    find_in_bodies(stmts, match_argmax)
}

// ── full attention ─────────────────────────────────────────────────────────────────────────────────────

/// Every collection a statement list READS by index that it does not itself bind, in first-read order.
fn read_collections(body: &[Stmt], locals: &HashSet<Symbol>, out: &mut Vec<Symbol>) {
    fn ex(e: &Expr, locals: &HashSet<Symbol>, out: &mut Vec<Symbol>) {
        match e {
            Expr::Index { collection, index } => {
                if let Expr::Identifier(s) = &**collection {
                    if !locals.contains(s) && !out.contains(s) {
                        out.push(*s);
                    }
                }
                ex(index, locals, out);
            }
            Expr::BinaryOp { left, right, .. } => {
                ex(left, locals, out);
                ex(right, locals, out);
            }
            Expr::Call { args, .. } => args.iter().for_each(|a| ex(a, locals, out)),
            _ => {}
        }
    }
    for s in body {
        match s {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } | Stmt::Push { value, .. } => {
                ex(value, locals, out)
            }
            Stmt::SetIndex { index, value, .. } => {
                ex(index, locals, out);
                ex(value, locals, out);
            }
            Stmt::While { body, .. } => read_collections(body, locals, out),
            _ => {}
        }
    }
}

/// Every name a statement list binds with `Let`, at any depth.
fn let_bound(body: &[Stmt], into: &mut HashSet<Symbol>) {
    for s in body {
        match s {
            Stmt::Let { var, .. } => {
                into.insert(*var);
            }
            Stmt::While { body, .. } => let_bound(body, into),
            _ => {}
        }
    }
}

/// The full-attention desugar: zero-fill the output, then a query-row loop over a head loop that builds a
/// fresh per-head score row, takes its max (`max(·,·)`), its exp-sum, and writes the weighted V sum. The
/// decode `attention` desugar differs in the one place that matters: it `Let`s a GQA k-head
/// (`head / group`) and reads a single query row — so neither recognizer can claim the other's program.
fn match_full_attention<'a>(body: &'a [Stmt<'a>]) -> Option<FullAttentionSpec> {
    let mut c = Cursor::new(body, 0);
    let out = c.new_let()?;
    let ozi = c.int_let(1)?;
    let fill = c.loop_(ozi, true)?;
    let (pushed, into) = Cursor::new(fill.body, 0).push()?;
    want(into == out && is_float(pushed, 0.0))?;
    let row = c.int_let(1)?;
    let rows = c.loop_(row, true)?;
    want(c.next().is_none())?;
    let t = rows.bound;
    let mut rb = Cursor::new(rows.body, 0);
    let head = rb.int_let(1)?;
    let heads = rb.loop_(head, true)?;
    let h = heads.bound;
    // The per-head body: a fresh score row, the score loop (its inner loop's bound is the head dim), the
    // max, the exp-sum, the weighted V loop — and no GQA `Let … be head / group`.
    let hb = heads.body;
    want(!hb.iter().any(|st| {
        matches!(
            st,
            Stmt::Let {
                value: Expr::BinaryOp {
                    op: BinaryOpKind::Divide,
                    ..
                },
                ..
            }
        )
    }))?;
    let mut hc = Cursor::new(hb, 0);
    hc.new_let()?;
    let j = hc.int_let(1)?;
    let scores = hc.loop_(j, true)?;
    want(scores.bound == t)?;
    let hd = scores
        .body
        .iter()
        .find_map(|st| as_loop(st).map(|l| l.bound))?;
    let uses_max = |b: &[Stmt]| {
        b.iter().any(|st| matches!(st, Stmt::While { body, .. } if body.iter().any(|x| matches!(x, Stmt::Set { value: Expr::Call { .. }, .. }))))
    };
    want(uses_max(hb))?;
    want(t >= 1 && h >= 1 && hd >= 1 && fill.bound == t * h * hd)?;
    let mut locals = HashSet::new();
    let_bound(body, &mut locals);
    let mut reads = Vec::new();
    read_collections(body, &locals, &mut reads);
    reads.retain(|r| *r != out);
    let [q, k, v] = reads.as_slice() else {
        return None;
    };
    Some(FullAttentionSpec {
        q: *q,
        k: *k,
        v: *v,
        out,
        h: h as usize,
        hd: hd as usize,
        t: t as usize,
    })
}

/// Recognize `Let C be full attention of Q over K and V with H heads`.
pub fn recognize_full_attention(stmts: &[Stmt]) -> Option<FullAttentionSpec> {
    find_in_bodies(stmts, match_full_attention)
}

// ── rope ───────────────────────────────────────────────────────────────────────────────────────────────

/// `Q[..]·T1[..] ∘ Q[..]·T2[..]` — a rotation term: the two `Q` reads and the two table reads, in order.
/// `rh·hd + i + shift + 1` — the 1-based item of element `i + shift` of head `rh`. Returns `hd`.
fn head_item(e: &Expr, rh: Symbol, i: Symbol, shift: i64) -> Option<i64> {
    let (sum, one) = as_binop(e, BinaryOpKind::Add)?;
    want(is_int(one, 1))?;
    let sum = if shift == 0 {
        sum
    } else {
        let (s, by) = as_binop(sum, BinaryOpKind::Add)?;
        want(is_int(by, shift))?;
        s
    };
    let (base, i_id) = as_binop(sum, BinaryOpKind::Add)?;
    want(is_ident(i_id, i))?;
    let (rh_id, hd) = as_binop(base, BinaryOpKind::Multiply)?;
    want(is_ident(rh_id, rh))?;
    as_int(hd)
}

/// A rope table's 1-based index: `ri + 1` — one row for every head (`None`) — or `(rh / per_row)·half + (ri + 1)` — a
/// row per row of `Q`, `per_row` heads to a row (`Some(per_row)`). Reading a cache from the checked position `at`, the
/// only form is `((at − 1) + rh / per_row)·half + (ri + 1)`.
fn rope_table_index(
    e: &Expr,
    rh: Symbol,
    ri: Symbol,
    half: i64,
    at: Option<Symbol>,
) -> Option<Option<i64>> {
    let within = |x: &Expr| {
        as_binop(x, BinaryOpKind::Add).is_some_and(|(r, one)| is_ident(r, ri) && is_int(one, 1))
    };
    if within(e) {
        return at.is_none().then_some(None);
    }
    let (row_base, rest) = as_binop(e, BinaryOpKind::Add)?;
    want(within(rest))?;
    let (row, by) = as_binop(row_base, BinaryOpKind::Multiply)?;
    want(is_int(by, half))?;
    let row = match at {
        None => row,
        Some(at) => {
            let (at_less_one, row) = as_binop(row, BinaryOpKind::Add)?;
            let (at_id, one) = as_binop(at_less_one, BinaryOpKind::Subtract)?;
            want(is_ident(at_id, at) && is_int(one, 1))?;
            row
        }
    };
    let (rh_id, per_row) = as_binop(row, BinaryOpKind::Divide)?;
    want(is_ident(rh_id, rh))?;
    let per_row = as_int(per_row)?;
    want(per_row >= 1)?;
    Some(Some(per_row))
}

/// `Q[a]·T1[t] ∘ Q[b]·T2[t]` — head `rh`'s rotation of the pair `(ri, ri + half)`, every index the desugar's: `a` and `b`
/// the pair's items ([`head_item`]) and `t` the table index ([`rope_table_index`]), the same for both tables. Returns
/// `(Q, T1, T2, hd, per_row)`.
fn rotation_terms<'a>(
    value: &'a Expr<'a>,
    op: BinaryOpKind,
    rh: Symbol,
    ri: Symbol,
    half: i64,
    at: Option<Symbol>,
) -> Option<(Symbol, Symbol, Symbol, i64, Option<i64>)> {
    let (m1, m2) = as_binop(value, op)?;
    let (qa, t1) = as_binop(m1, BinaryOpKind::Multiply)?;
    let (qb, t2) = as_binop(m2, BinaryOpKind::Multiply)?;
    let (q1, a) = indexed(qa)?;
    let (q2, b) = indexed(qb)?;
    want(q1 == q2)?;
    let hd = head_item(a, rh, ri, 0)?;
    want(head_item(b, rh, ri, half)? == hd)?;
    let (c1, i1) = indexed(t1)?;
    let (c2, i2) = indexed(t2)?;
    let per_row = rope_table_index(i1, rh, ri, half, at)?;
    want(rope_table_index(i2, rh, ri, half, at)? == per_row)?;
    Some((q1, c1, c2, hd, per_row))
}

fn match_rope<'a>(body: &'a [Stmt<'a>]) -> Option<RopeSpec> {
    for (i, out) in anchors(body) {
        let mut c = Cursor::new(body, i + 1);
        let Some(rh) = c.int_let(0) else { continue };
        let Some(outer) = c.loop_(rh, false) else {
            continue;
        };
        // A cache read from a position opens with that position, checked: `Let at be P`, then its range assertion — so a
        // checked index before the anchor is tried as the rope's position first, and as nothing of the rope's after.
        let checked = i
            .checked_sub(2)
            .and_then(|k| checked_row_index(&mut Cursor::new(body, k)));
        let try_spec = |checked: Option<(Symbol, RowIndex, usize)>| {
            let at = checked.map(|(at, _, _)| at);
            let mut ob = Cursor::new(outer.body, 0);
            let ri = ob.int_let(0)?;
            let loop1 = ob.loop_(ri, false)?;
            let half = loop1.bound;
            let (push1, into1) = Cursor::new(loop1.body, 0).push()?;
            want(into1 == out)?;
            let (q, rc, rs, hd, per_row) =
                rotation_terms(push1, BinaryOpKind::Subtract, rh, ri, half, at)?;
            let (reset, zero) = ob.set()?;
            want(reset == ri && is_int(zero, 0))?;
            let loop2 = ob.loop_(ri, false)?;
            want(loop2.bound == half)?;
            let (push2, into2) = Cursor::new(loop2.body, 0).push()?;
            want(into2 == out)?;
            want(
                rotation_terms(push2, BinaryOpKind::Add, rh, ri, half, at)?
                    == (q, rs, rc, hd, per_row),
            )?;
            let rp = ob.int_let(2 * half)?;
            let loop3 = ob.loop_(rp, false)?;
            let (push3, into3) = Cursor::new(loop3.body, 0).push()?;
            let (q3, item) = indexed(push3)?;
            want(
                into3 == out && q3 == q && head_item(item, rh, rp, 0)? == hd && loop3.bound == hd,
            )?;
            let h = outer.bound;
            let rows = match per_row {
                None => 1,
                Some(p) => {
                    want(h % p == 0)?;
                    h / p
                }
            };
            // The check keeps the block's last row among the cache's: `at ≤ cache − rows + 1`.
            let from = checked.map(|(_, index, last)| (index, last + rows as usize - 1));
            Some(RopeSpec {
                q,
                rc,
                rs,
                out,
                h: h as usize,
                half: half as usize,
                hd: hd as usize,
                rows: rows as usize,
                from,
            })
        };
        let spec = checked
            .and_then(|c| try_spec(Some(c)))
            .or_else(|| try_spec(None));
        if spec.is_some() {
            return spec;
        }
    }
    None
}

/// Recognize `Let Qr be rope of Q with cos RC and sin RS [from position P] [across H heads]` — per head, the two
/// rotation loops over `half` and the pass-through loop up to `hd`, after the position's check when there is one.
pub fn recognize_rope(stmts: &[Stmt]) -> Option<RopeSpec> {
    find_in_bodies(stmts, match_rope)
}

// ── causal conv ────────────────────────────────────────────────────────────────────────────────────────

/// The single step's three loops from `c` — the ring shift, the append of `X`, the tap reduce into `Y` — over rows
/// `chan` wide, `X` and `Y` indexed through `at` (the loop variable itself, or row `row` of a block). Returns
/// `(X, WC, CS, taps)`.
fn conv_step<'a>(
    c: &mut Cursor<'a>,
    out: Symbol,
    chan: i64,
    at: &dyn Fn(&Expr, Symbol) -> bool,
) -> Option<(Symbol, Symbol, Symbol, i64)> {
    // 1. shift the ring up one row: CS[(t1−1)·chan + c1] ← CS[t1·chan + c1]
    let t1 = c.int_let(1)?;
    let shift = c.loop_(t1, true)?;
    let mut sb = Cursor::new(shift.body, 0);
    let c1 = sb.int_let(1)?;
    let shift_inner = sb.loop_(c1, true)?;
    want(shift_inner.bound == chan)?;
    let (cs, _, moved) = Cursor::new(shift_inner.body, 0).set_index()?;
    want(indexed(moved)?.0 == cs)?;
    // 2. this token becomes the newest row: CS[(taps−1)·chan + c2] ← X[at]
    let c2 = c.int_let(1)?;
    let append = c.loop_(c2, true)?;
    want(append.bound == chan)?;
    let (cs2, _, newest) = Cursor::new(append.body, 0).set_index()?;
    want(cs2 == cs)?;
    let (x, x_at) = indexed(newest)?;
    want(at(x_at, c2))?;
    // 3. Y[at] = Σ_t CS[t,c3]·WC[t,c3]
    let c3 = c.int_let(1)?;
    let reduce = c.loop_(c3, true)?;
    want(reduce.bound == chan)?;
    let mut rb = Cursor::new(reduce.body, 0);
    let acc = rb.float_let(0.0)?;
    let t2 = rb.int_let(1)?;
    let taps_loop = rb.loop_(t2, true)?;
    let taps = taps_loop.bound;
    want(shift.bound == taps - 1)?;
    let (acc_target, acc_v) = Cursor::new(taps_loop.body, 0).set()?;
    want(acc_target == acc)?;
    let (acc_id, term) = as_binop(acc_v, BinaryOpKind::Add)?;
    want(is_ident(acc_id, acc))?;
    let (cs_read, wc_read) = as_binop(term, BinaryOpKind::Multiply)?;
    want(indexed(cs_read)?.0 == cs)?;
    let wc = indexed(wc_read)?.0;
    let (out2, idx, stored) = rb.set_index()?;
    want(out2 == out && at(idx, c3) && is_ident(stored, acc))?;
    Some((x, wc, cs, taps))
}

fn match_causal_conv<'a>(body: &'a [Stmt<'a>]) -> Option<CausalConvSpec> {
    for (i, out) in anchors(body) {
        let mut c = Cursor::new(body, i + 1);
        let Some(zi) = c.int_let(1) else { continue };
        let Some(zero_fill) = c.loop_(zi, true) else {
            continue;
        };
        let filled = zero_fill.bound;
        let spec = (|| {
            let (zero, into) = Cursor::new(zero_fill.body, 0).push()?;
            want(into == out && is_float(zero, 0.0))?;
            let mut block = c.clone();
            // One row: the step's loops follow the zero fill.
            if let Some((x, wc, cs, taps)) = conv_step(&mut c, out, filled, &|e, v| is_ident(e, v))
            {
                return Some(CausalConvSpec {
                    x,
                    wc,
                    cs,
                    out,
                    taps: taps as usize,
                    chan: filled as usize,
                    rows: 1,
                });
            }
            // A block: the step inside a row loop, `X` and `Y` read and written at `(row − 1)·chan + c`.
            let row = block.int_let(1)?;
            let rows = block.loop_(row, true)?;
            want(rows.bound > 1 && filled % rows.bound == 0)?;
            let chan = filled / rows.bound;
            let mut rb = Cursor::new(rows.body, 0);
            let (x, wc, cs, taps) = conv_step(&mut rb, out, chan, &|e, v| {
                row_item_at(e, row, v) == Some(chan)
            })?;
            let (inc, _) = rb.set()?;
            want(inc == row && rb.next().is_none())?;
            Some(CausalConvSpec {
                x,
                wc,
                cs,
                out,
                taps: taps as usize,
                chan: chan as usize,
                rows: rows.bound as usize,
            })
        })();
        if spec.is_some() {
            return spec;
        }
    }
    None
}

/// Recognize `Let Y be causal conv of X with weight WC state CS over T taps` — the three loops: the ring
/// shift `CS[(t−1)·chan+c] ← CS[t·chan+c]`, the append of `X`, and the tap reduce against `WC`.
pub fn recognize_causal_conv(stmts: &[Stmt]) -> Option<CausalConvSpec> {
    find_in_bodies(stmts, match_causal_conv)
}

// ── gated delta ────────────────────────────────────────────────────────────────────────────────────────

/// `Set acc to acc + S[..]·K[..]` — a matvec accumulation step; returns the two reads, each as its
/// collection and index.
fn matvec_step<'a>(
    s: Option<&'a Stmt<'a>>,
    acc: Symbol,
) -> Option<((Symbol, &'a Expr<'a>), (Symbol, &'a Expr<'a>))> {
    let Stmt::Set { target, value } = s? else {
        return None;
    };
    want(*target == acc)?;
    let (acc_id, term) = as_binop(value, BinaryOpKind::Add)?;
    want(is_ident(acc_id, acc))?;
    let (l, r) = as_binop(term, BinaryOpKind::Multiply)?;
    Some((indexed(l)?, indexed(r)?))
}

/// `base + i` — a head-relative read index.
fn offset_by(idx: &Expr, base: Symbol, i: Symbol) -> bool {
    as_binop(idx, BinaryOpKind::Add)
        .map(|(b, x)| is_ident(b, base) && is_ident(x, i))
        .unwrap_or(false)
}

/// `V / (1 + E^(0 − V))` where `V = Z[i]` and both reads' `i` satisfy `at` — the silu gate of the head; returns `Z`.
fn silu_of_read(e: &Expr, at: &dyn Fn(&Expr) -> bool) -> Option<Symbol> {
    let (num, den) = as_binop(e, BinaryOpKind::Divide)?;
    let (z, i) = indexed(num)?;
    let (one, pow) = as_binop(den, BinaryOpKind::Add)?;
    want(is_float(one, 1.0))?;
    let (base, expo) = as_binop(pow, BinaryOpKind::Pow)?;
    want(is_e(base))?;
    let (zero, again) = as_binop(expo, BinaryOpKind::Subtract)?;
    let (z2, i2) = indexed(again)?;
    want(is_float(zero, 0.0) && z2 == z && at(i) && at(i2))?;
    Some(z)
}

/// `1 / (1 + E^(0 − z[i]))` — the sigmoid of an element read where `at` says.
fn sigmoid_of_read(e: &Expr, at: &dyn Fn(&Expr) -> bool) -> Option<Symbol> {
    let (num, den) = as_binop(e, BinaryOpKind::Divide)?;
    want(is_float(num, 1.0))?;
    let (one, pow) = as_binop(den, BinaryOpKind::Add)?;
    want(is_float(one, 1.0))?;
    let (base, expo) = as_binop(pow, BinaryOpKind::Pow)?;
    want(is_e(base))?;
    let (zero, read) = as_binop(expo, BinaryOpKind::Subtract)?;
    let (z, i) = indexed(read)?;
    want(is_float(zero, 0.0) && at(i))?;
    Some(z)
}

/// `(head − 1) · per` — a head's base in a run of `per`-wide heads.
fn head_times(e: &Expr, head: Symbol, per: i64) -> bool {
    as_binop(e, BinaryOpKind::Multiply).is_some_and(|(h, w)| {
        as_binop(h, BinaryOpKind::Subtract)
            .is_some_and(|(h, one)| is_ident(h, head) && is_int(one, 1))
            && is_int(w, per)
    })
}

/// The Gated DeltaNet step's head loop from `c` — the key/value bases, the gates, the memory update and the gated norm —
/// writing `out`, which the zero fill made `filled` long. A block's step runs inside a row loop: `row` is its
/// `(rq, rv, rh)` bases, added to the key base and to the gates' index.
fn gd_heads<'a>(
    c: &mut Cursor<'a>,
    out: Symbol,
    row: Option<(Symbol, Symbol, Symbol)>,
    filled: i64,
) -> Option<GatedDeltaSpec> {
    let head = c.int_let(1)?;
    let head_read = |e: &Expr| -> Option<Symbol> {
        let (coll, at) = indexed(e)?;
        match row {
            None => is_ident(at, head).then_some(coll),
            Some((_, _, rh)) => {
                let (rh_id, head_id) = as_binop(at, BinaryOpKind::Add)?;
                (is_ident(rh_id, rh) && is_ident(head_id, head)).then_some(coll)
            }
        }
    };
    let heads = c.loop_(head, true)?;
    let hv = heads.bound;
    let mut hb = Cursor::new(heads.body, 0);
    // Let kb be ((head − 1) mod hk) · d — the key head this value head reads.
    let (kb, kb_v) = hb.let_()?;
    let kb_v = match row {
        None => kb_v,
        Some((rq, _, _)) => {
            let (rq_id, rest) = as_binop(kb_v, BinaryOpKind::Add)?;
            want(is_ident(rq_id, rq))?;
            rest
        }
    };
    let (modulo, d_e) = as_binop(kb_v, BinaryOpKind::Multiply)?;
    let d = as_int(d_e)?;
    let (head_minus, hk_e) = as_binop(modulo, BinaryOpKind::Modulo)?;
    let hk = as_int(hk_e)?;
    let (head_id, one) = as_binop(head_minus, BinaryOpKind::Subtract)?;
    want(is_ident(head_id, head) && is_int(one, 1) && filled == hv * d)?;
    let (vb, vb_v) = hb.let_()?;
    let vb_v = match row {
        None => vb_v,
        Some((_, rv, _)) => {
            let (rv_id, rest) = as_binop(vb_v, BinaryOpKind::Add)?;
            want(is_ident(rv_id, rv))?;
            rest
        }
    };
    want(head_times(vb_v, head, d))?;
    let (_sb, sb_v) = hb.let_()?;
    want(head_times(sb_v, head, d * d))?;
    let (alpha, alpha_v) = hb.let_()?;
    let a = head_read(alpha_v)?;
    let (beta, beta_v) = hb.let_()?;
    let b = head_read(beta_v)?;
    // 1. u = S·k
    let uvec = hb.new_let()?;
    let ua = hb.int_let(1)?;
    let u_loop = hb.loop_(ua, true)?;
    want(u_loop.bound == d)?;
    let mut ub_c = Cursor::new(u_loop.body, 0);
    let uacc = ub_c.float_let(0.0)?;
    let ub = ub_c.int_let(1)?;
    let u_inner = ub_c.loop_(ub, true)?;
    want(u_inner.bound == d)?;
    let ((st, _), (k, k_idx)) = matvec_step(u_inner.body.first(), uacc)?;
    want(offset_by(k_idx, kb, ub))?;
    let (pushed, into) = ub_c.push()?;
    want(is_ident(pushed, uacc) && into == uvec)?;
    // 2. S = α·S + β·(v − α·u)·kᵀ
    let sa = hb.int_let(1)?;
    let s_loop = hb.loop_(sa, true)?;
    want(s_loop.bound == d)?;
    let mut sc = Cursor::new(s_loop.body, 0);
    let (coef, coef_v) = sc.let_()?;
    let (beta_id, err) = as_binop(coef_v, BinaryOpKind::Multiply)?;
    want(is_ident(beta_id, beta))?;
    let (v_read, alpha_u) = as_binop(err, BinaryOpKind::Subtract)?;
    let (v, v_idx) = indexed(v_read)?;
    want(offset_by(v_idx, vb, sa))?;
    let (alpha_id, u_read) = as_binop(alpha_u, BinaryOpKind::Multiply)?;
    want(is_ident(alpha_id, alpha) && indexed(u_read)?.0 == uvec)?;
    let sbi = sc.int_let(1)?;
    let s_inner = sc.loop_(sbi, true)?;
    want(s_inner.bound == d)?;
    let (st2, _, update) = Cursor::new(s_inner.body, 0).set_index()?;
    want(st2 == st)?;
    let (decayed, corrected) = as_binop(update, BinaryOpKind::Add)?;
    let (alpha_id, st_read) = as_binop(decayed, BinaryOpKind::Multiply)?;
    want(is_ident(alpha_id, alpha) && indexed(st_read)?.0 == st)?;
    let (coef_id, k_read) = as_binop(corrected, BinaryOpKind::Multiply)?;
    want(is_ident(coef_id, coef) && indexed(k_read)?.0 == k)?;
    // 3. o = S·q, read from the updated memory
    let ovec = hb.new_let()?;
    let oa = hb.int_let(1)?;
    let o_loop = hb.loop_(oa, true)?;
    want(o_loop.bound == d)?;
    let mut ob_c = Cursor::new(o_loop.body, 0);
    let oacc = ob_c.float_let(0.0)?;
    let ob = ob_c.int_let(1)?;
    let o_inner = ob_c.loop_(ob, true)?;
    want(o_inner.bound == d)?;
    let ((st3, _), (q, q_idx)) = matvec_step(o_inner.body.first(), oacc)?;
    want(st3 == st && offset_by(q_idx, kb, ob))?;
    let (pushed, into) = ob_c.push()?;
    let (oacc_id, _scale) = as_binop(pushed, BinaryOpKind::Multiply)?;
    want(is_ident(oacc_id, oacc) && into == ovec)?;
    // 4. RMSNorm of the ungated head, scale by GN, gate by silu(z)
    let ssq = hb.float_let(0.0)?;
    let ga = hb.int_let(1)?;
    let gate_loop = hb.loop_(ga, true)?;
    want(gate_loop.bound == d)?;
    let (den, den_v) = hb.let_()?;
    let Expr::Call { args, .. } = den_v else {
        return None;
    };
    let [arg] = args.as_slice() else { return None };
    let (mean, eps) = as_binop(arg, BinaryOpKind::Add)?;
    let (ssq_id, d_f) = as_binop(mean, BinaryOpKind::Divide)?;
    want(is_ident(ssq_id, ssq) && is_float(d_f, d as f64))?;
    let eps = as_float(eps).filter(|e| *e > 0.0)?;
    let na = hb.int_let(1)?;
    let norm_loop = hb.loop_(na, true)?;
    want(norm_loop.bound == d)?;
    let (out2, idx, stored) = Cursor::new(norm_loop.body, 0).set_index()?;
    want(out2 == out)?;
    let (vb_id, na_id) = as_binop(idx, BinaryOpKind::Add)?;
    want(is_ident(vb_id, vb) && is_ident(na_id, na))?;
    let (normed, gate) = as_binop(stored, BinaryOpKind::Multiply)?;
    let at = |i: &Expr| offset_by(i, vb, na);
    let (z, gate) = match silu_of_read(gate, &at) {
        Some(z) => (z, GateAct::Silu),
        None => (sigmoid_of_read(gate, &at)?, GateAct::Sigmoid),
    };
    let (gn_read, o_over_den) = as_binop(normed, BinaryOpKind::Multiply)?;
    let gn = read_at(gn_read, na)?;
    let (o_read, den_id) = as_binop(o_over_den, BinaryOpKind::Divide)?;
    want(indexed(o_read)?.0 == ovec && is_ident(den_id, den))?;
    Some(GatedDeltaSpec {
        q,
        k,
        v,
        z,
        b,
        a,
        st,
        gn,
        out,
        hv: hv as usize,
        hk: hk as usize,
        d: d as usize,
        rows: 1,
        eps: Epsilon::new(eps),
        gate,
    })
}

fn match_gated_delta<'a>(body: &'a [Stmt<'a>]) -> Option<GatedDeltaSpec> {
    for (i, out) in anchors(body) {
        let mut c = Cursor::new(body, i + 1);
        let Some(zi) = c.int_let(1) else { continue };
        let Some(zero_fill) = c.loop_(zi, true) else {
            continue;
        };
        let spec = (|| {
            let (zero, into) = Cursor::new(zero_fill.body, 0).push()?;
            want(into == out && is_float(zero, 0.0))?;
            // One row: the head loop follows the zero fill.
            if let Some(spec) = gd_heads(&mut c.clone(), out, None, zero_fill.bound) {
                return Some(spec);
            }
            // A block: a row loop binding the row's bases `(row − 1)·HK·D`, `(row − 1)·HV·D` and `(row − 1)·HV`, then the
            // head loop over them.
            let row = c.int_let(1)?;
            let rows = c.loop_(row, true)?;
            want(rows.bound > 1 && zero_fill.bound % rows.bound == 0)?;
            let mut rb = Cursor::new(rows.body, 0);
            let (rq, rq_v) = rb.let_()?;
            let (rv, rv_v) = rb.let_()?;
            let (rh, rh_v) = rb.let_()?;
            let spec = gd_heads(
                &mut rb,
                out,
                Some((rq, rv, rh)),
                zero_fill.bound / rows.bound,
            )?;
            let (inc, _) = rb.set()?;
            want(inc == row && rb.next().is_none())?;
            let base = |e: &Expr, per: usize| {
                as_binop(e, BinaryOpKind::Multiply).is_some_and(|(r, w)| {
                    as_binop(r, BinaryOpKind::Subtract)
                        .is_some_and(|(r, one)| is_ident(r, row) && is_int(one, 1))
                        && is_int(w, per as i64)
                })
            };
            want(
                base(rq_v, spec.hk * spec.d) && base(rv_v, spec.hv * spec.d) && base(rh_v, spec.hv),
            )?;
            Some(GatedDeltaSpec {
                rows: rows.bound as usize,
                ..spec
            })
        })();
        if spec.is_some() {
            return spec;
        }
    }
    None
}

/// Recognize `Let O be gated delta of Q over K and V with gate Z beta B decay A state S norm GN with HV heads
/// HK groups` — the `head` loop opened by the `(head−1) mod HK` key-head binding, the in-place `S` update,
/// and the gated norm that writes the head.
pub fn recognize_gated_delta(stmts: &[Stmt]) -> Option<GatedDeltaSpec> {
    find_in_bodies(stmts, match_gated_delta)
}

// ── attention ──────────────────────────────────────────────────────────────────────────────────────────

/// Does a block mask with the `-1e30` sentinel — the top-K argmax only the `moe` desugar performs?
fn masks(body: &[Stmt]) -> bool {
    body.iter().any(|s| match s {
        Stmt::SetIndex { value, .. } => {
            matches!(value, Expr::Literal(Literal::Float(f)) if *f < -1.0e29)
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => masks(body),
        Stmt::If {
            then_block,
            else_block,
            ..
        } => masks(then_block) || else_block.map(masks).unwrap_or(false),
        _ => false,
    })
}

/// Every `Let`-bound symbol in a block, recursively — the block's temporaries.
fn collect_lets(body: &[Stmt], out: &mut HashSet<Symbol>) {
    for s in body {
        match s {
            Stmt::Let { var, .. } => {
                out.insert(*var);
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => {
                collect_lets(body, out)
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_lets(then_block, out);
                if let Some(eb) = else_block {
                    collect_lets(eb, out);
                }
            }
            _ => {}
        }
    }
}

/// The literal bound of a `While v ≤ N` loop.
fn attention_while_bound(cond: &Expr) -> Option<i64> {
    if let Expr::BinaryOp {
        op: BinaryOpKind::LtEq,
        right,
        ..
    } = cond
    {
        return as_int(right);
    }
    None
}

/// An attention's operands `(Q, K, V)`: the query and the keys, the first two tensors its block indexes from outside in
/// program order, and the values, the one tensor the loop writing the output weighs — the keys again when a tensor is
/// both (a latent cache read as its keys and its values).
fn attention_qkv(body: &[Stmt], locals: &HashSet<Symbol>) -> Option<(Symbol, Symbol, Symbol)> {
    fn values(body: &[Stmt], locals: &HashSet<Symbol>) -> Option<Symbol> {
        body.iter().find_map(|st| match st {
            Stmt::While { body: wb, .. } if wb.iter().any(|s| matches!(s, Stmt::SetIndex { .. })) => {
                let mut read = Vec::new();
                attention_operands(wb, locals, &mut read);
                match read[..] {
                    [v] => Some(v),
                    _ => None,
                }
            }
            Stmt::While { body: wb, .. } | Stmt::Repeat { body: wb, .. } | Stmt::Splice { body: wb } => values(wb, locals),
            _ => None,
        })
    }
    let mut ops = Vec::new();
    attention_operands(body, locals, &mut ops);
    let v = values(body, locals)?;
    match ops[..] {
        [q, k, third, ..] if third == v => Some((q, k, v)),
        [q, k] if k == v => Some((q, k, v)),
        _ => None,
    }
}

/// The externally-indexed operands of an attention desugar, in program order (`Q, K, V`).
fn attention_operands(body: &[Stmt], locals: &HashSet<Symbol>, out: &mut Vec<Symbol>) {
    fn ex(e: &Expr, locals: &HashSet<Symbol>, out: &mut Vec<Symbol>) {
        match e {
            Expr::Index { collection, index } => {
                if let Expr::Identifier(s) = &**collection {
                    if !locals.contains(s) && !out.contains(s) {
                        out.push(*s);
                    }
                }
                ex(index, locals, out);
            }
            Expr::BinaryOp { left, right, .. } => {
                ex(left, locals, out);
                ex(right, locals, out);
            }
            Expr::Call { args, .. } => {
                for a in args {
                    ex(a, locals, out);
                }
            }
            _ => {}
        }
    }
    for s in body {
        match s {
            Stmt::Set { value, .. } => ex(value, locals, out),
            Stmt::SetIndex { index, value, .. } => {
                ex(index, locals, out);
                ex(value, locals, out);
            }
            Stmt::Push { value, .. } => ex(value, locals, out),
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => {
                attention_operands(body, locals, out)
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                ex(cond, locals, out);
                attention_operands(then_block, locals, out);
                if let Some(eb) = else_block {
                    attention_operands(eb, locals, out);
                }
            }
            _ => {}
        }
    }
}

/// `((n − 1)/C)·C + 1` over the attended length `n`: the first position of a query's chunk of `C`.
fn chunk_start(value: &Expr, bound: &Expr) -> Option<usize> {
    let (scaled, one) = as_binop(value, BinaryOpKind::Add)?;
    let (whole, c) = as_binop(scaled, BinaryOpKind::Multiply)?;
    let (before, c_again) = as_binop(whole, BinaryOpKind::Divide)?;
    let (n, one_again) = as_binop(before, BinaryOpKind::Subtract)?;
    let c = as_int(c)?;
    (c > 0 && as_int(c_again) == Some(c) && as_int(one) == Some(1) && as_int(one_again) == Some(1) && same_length(n, bound))
        .then_some(c as usize)
}

/// Whether `a` and `b` name the same attended length: one scalar, or one literal.
fn same_length(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Identifier(x), Expr::Identifier(y)) => x == y,
        _ => as_int(a).is_some() && as_int(a) == as_int(b),
    }
}

/// The head loop of an attention desugar, as its recognizers read it.
struct Heads<'a> {
    /// The query heads: the loop's bound.
    h: usize,
    /// The query heads per key/value head: the `kvh` divisor.
    group: usize,
    /// The positions attended: the score loop's bound.
    bound: &'a Expr<'a>,
    /// The head dimension: the score loop's inner bound.
    hd: usize,
    /// The value head width: the weighted sum's bound.
    vd: usize,
    /// The score scale, when it is not `1/√hd`: the factor each score is pushed times.
    scale: Option<Factor>,
    /// The cap each scaled score is pushed under, when it is capped.
    softcap: Option<Factor>,
    /// The window, when the head loop binds one.
    window: Option<Window>,
    /// The sinks' tensor, when the softmax's maximum starts from one.
    sinks: Option<Symbol>,
}

/// The loops of an attention desugar's heads, whatever its softmax: the head loop (the `While` among `body`'s statements
/// whose body Lets the per-head `scores`), its body, its bound, the `kvh` divisor, and the score loop's index, bound and
/// inner bound.
struct HeadLoops<'a> {
    /// The head loop's body.
    body: &'a [Stmt<'a>],
    /// The query heads: the loop's bound.
    h: usize,
    /// The query heads per key/value head: the `kvh` divisor.
    group: usize,
    /// The positions attended: the score loop's bound.
    bound: &'a Expr<'a>,
    /// The score loop's index, the position each score is of.
    at: Symbol,
    /// The head dimension: the score loop's inner bound.
    hd: usize,
}

fn head_loops<'a>(body: &'a [Stmt<'a>]) -> Option<HeadLoops<'a>> {
    let head = body.iter().find_map(|st| match st {
        Stmt::While { cond, body: wb, .. }
            if wb
                .iter()
                .any(|s| matches!(s, Stmt::Let { value, .. } if is_new(value))) =>
        {
            Some((cond, wb))
        }
        _ => None,
    })?;
    let h = attention_while_bound(head.0)? as usize;
    let hbody = head.1;
    let group = hbody.iter().find_map(|st| match st {
        Stmt::Let {
            value:
                Expr::BinaryOp {
                    op: BinaryOpKind::Divide,
                    right,
                    ..
                },
            ..
        } => as_int(right).map(|g| g as usize),
        _ => None,
    })?;
    let (at, bound, hd) = hbody.iter().find_map(|st| match st {
        Stmt::While {
            cond:
                Expr::BinaryOp {
                    op: BinaryOpKind::LtEq,
                    left: Expr::Identifier(at),
                    right,
                },
            body: sb,
            ..
        } => sb
            .iter()
            .find_map(|s| {
                if let Stmt::While { cond: ic, .. } = s {
                    attention_while_bound(ic)
                } else {
                    None
                }
            })
            .map(|hd| (*at, *right, hd as usize)),
        _ => None,
    })?;
    Some(HeadLoops { body: hbody, h, group, bound, at, hd })
}

/// The head loop of a float attention desugar among `body`'s statements, with its value width, its score scale, its
/// window and its sinks.
fn attention_heads<'a>(body: &'a [Stmt<'a>]) -> Option<Heads<'a>> {
    let HeadLoops { body: hbody, h, group, bound, at, hd } = head_loops(body)?;
    // The weighted sum writes the output: its loop runs over the value head width.
    let vd = hbody.iter().find_map(|st| match st {
        Stmt::While { cond, body: wb, .. } if wb.iter().any(|s| matches!(s, Stmt::SetIndex { .. })) => {
            attention_while_bound(cond)
        }
        _ => None,
    })? as usize;
    // Each score is pushed times the scale, capped where the program caps it.
    let (pushed, softcap) = hbody.iter().find_map(|st| match st {
        Stmt::While { body: sb, .. } => sb.iter().find_map(|s| match s {
            Stmt::Push { value, .. } => {
                let (scaled, cap) = match as_capped(value) {
                    Some((scaled, c)) => (scaled, Some(Factor::new(c))),
                    None => (*value, None),
                };
                as_binop(scaled, BinaryOpKind::Multiply).and_then(|(_, f)| as_float(f)).map(|f| (f, cap))
            }
            _ => None,
        }),
        _ => None,
    })?;
    let scale = (pushed.to_bits() != (1.0f64 / (hd as f64).sqrt()).to_bits()).then(|| Factor::new(pushed));
    // A window binds its first position in the head loop, and the score loop starts there: `max(1, n − (W − 1))` for
    // the last `W`, `((n − 1)/C)·C + 1` for a chunk of `C`.
    let windowed = hbody.iter().find_map(|st| match st {
        Stmt::Let { var, value: Expr::Call { args, .. }, .. } => match args.as_slice() {
            [one, Expr::BinaryOp { op: BinaryOpKind::Subtract, left, right }] if as_int(one) == Some(1) && same_length(left, bound) => {
                as_int(right).map(|k| (*var, Window::Sliding(k as usize + 1)))
            }
            _ => None,
        },
        Stmt::Let { var, value, .. } => chunk_start(value, bound).map(|c| (*var, Window::Chunked(c))),
        _ => None,
    });
    // The score loop starts at the first position, or at the window's: one starting anywhere else (a row's own start,
    // `with rows starting at R`) is no attention this spec holds.
    let start = hbody.iter().find_map(|st| match st {
        Stmt::Let { var, value, .. } if *var == at => Some(*value),
        _ => None,
    })?;
    let from_window_or_one = match (windowed, start) {
        (Some((lo, _)), Expr::Identifier(x)) => *x == lo,
        (None, first) => as_int(first) == Some(1),
        _ => false,
    };
    if !from_window_or_one {
        return None;
    }
    let window = windowed.map(|(_, w)| w);
    // Sinks start the softmax's maximum at `max(scores[1], S[head])`.
    let sinks = hbody.iter().find_map(|st| match st {
        Stmt::Let { value: Expr::Call { args, .. }, .. } => match args.as_slice() {
            [first, sink] => {
                let (_, one) = as_index(first)?;
                let (s, at) = as_index(sink)?;
                (as_int(one) == Some(1) && matches!(at, Expr::Identifier(_))).then_some(as_ident(s)?)
            }
            _ => None,
        },
        _ => None,
    });
    Some(Heads { h, group, bound, hd, vd, scale, softcap, window, sinks })
}

/// Recognize the single-query GQA `attention` block. The desugar is a `Splice` that: Lets the output `Seq`
/// (first `New`), zero-fills it, then a head-loop (`While` ≤ H whose body Lets a `scores` `New(Seq)`) computes
/// `scores = Q·K / √HD`, a softmax (`Pow(E, …)`), and the weighted sum of `V`, `SetIndex`ing the output. It
/// does NOT mask with the `-1e30` sentinel (that's `moe`). Dims come from the loop bounds (H = head-loop
/// bound, L = score-loop bound, HD = score-inner bound) and the `kvh` divisor (`group = H/KV`); the three
/// externally-indexed operands, in program order, are `Q, K, V`. The masked `up to P` form sets `pos`.
pub fn recognize_attention(stmts: &[Stmt]) -> Option<AttentionSpec> {
    for s in stmts {
        let Stmt::Splice { body } = s else { continue };
        if masks(body) || !exp_softmax(body) {
            continue;
        }
        // Output = the first Let with a New(Seq) value.
        let out = body.iter().find_map(|st| match st {
            Stmt::Let { var, value, .. } if is_new(value) => Some(*var),
            _ => None,
        })?;
        // Head-loop = the While whose body Lets a New(Seq) (the per-head `scores`); group = the `kvh` divisor; the score
        // loop runs over the cache — to the literal L, or (the masked `up to P` form) to a length bound at the top.
        let Heads { h, group, bound, hd, vd, scale, softcap, window, sinks } = attention_heads(body)?;
        let kv = if group > 0 { h / group } else { h };
        let (l, pos) = match bound {
            Expr::Identifier(len) => match masked_length(body, *len) {
                Some(masked) => masked,
                None => continue,
            },
            other => (as_int(other).unwrap_or(0) as usize, None),
        };
        let mut locals = HashSet::new();
        collect_lets(body, &mut locals);
        let Some((q, kc, vc)) = attention_qkv(body, &locals) else { continue };
        if h == 0 || hd == 0 || vd == 0 || l == 0 {
            continue;
        }
        return Some(AttentionSpec {
            q,
            kc,
            vc,
            out,
            h,
            kv,
            hd,
            vd,
            scale,
            softcap,
            l,
            pos,
            rows: 1,
            window,
            sinks,
        });
    }
    None
}

/// Recognize `Let C be causal attention of Q over K and V with H heads KV groups from P` — a block of `T` query rows:
/// the `Let from be P` and its block check `from ≥ 1 and from + (T − 1) ≤ L` at the top, the output's zero fill, then a
/// row loop (to `T`) whose body binds the row's length `from + row − 1` and runs the single-query head loop over it.
pub fn recognize_causal_attention(stmts: &[Stmt]) -> Option<AttentionSpec> {
    for s in stmts {
        let Stmt::Splice { body } = s else { continue };
        if masks(body) || !exp_softmax(body) {
            continue;
        }
        let Some(out) = body.iter().find_map(|st| match st {
            Stmt::Let { var, value, .. } if is_new(value) => Some(*var),
            _ => None,
        }) else {
            continue;
        };
        // The row loop: a top-level While whose body holds the head loop.
        let Some((rows, rbody)) = body.iter().find_map(|st| match st {
            Stmt::While { cond, body: wb, .. } if attention_heads(wb).is_some() => {
                Some((attention_while_bound(cond)? as usize, wb))
            }
            _ => None,
        }) else {
            continue;
        };
        let Some(Heads { h, group, bound: Expr::Identifier(row_len), hd, vd, scale, softcap, window, sinks }) = attention_heads(rbody) else {
            continue;
        };
        let kv = if group > 0 { h / group } else { h };
        // The row's length is `from + row − 1`; `from` is bound to `P` at the top and the block checked against `L`.
        let from = rbody.iter().find_map(|st| match st {
            Stmt::Let {
                var,
                value:
                    Expr::BinaryOp {
                        op: BinaryOpKind::Subtract,
                        left:
                            Expr::BinaryOp {
                                op: BinaryOpKind::Add,
                                left: Expr::Identifier(f),
                                ..
                            },
                        ..
                    },
                ..
            } if var == row_len => Some(*f),
            _ => None,
        });
        let Some(from) = from else { continue };
        let Some(pos) = body.iter().find_map(|st| match st {
            Stmt::Let { var, value, .. } if *var == from => match value {
                Expr::Identifier(p) => Some(RowIndex::Named(*p)),
                other => as_int(other).map(RowIndex::Literal),
            },
            _ => None,
        }) else {
            continue;
        };
        let l = body.iter().find_map(|st| match st {
            Stmt::RuntimeAssert {
                condition:
                    Expr::BinaryOp {
                        op: BinaryOpKind::And,
                        right:
                            Expr::BinaryOp {
                                op: BinaryOpKind::LtEq,
                                left:
                                    Expr::BinaryOp {
                                        op: BinaryOpKind::Add,
                                        left: Expr::Identifier(f),
                                        right: span,
                                    },
                                right: cap,
                            },
                        ..
                    },
                ..
            } if *f == from && as_int(span) == Some(rows as i64 - 1) => as_int(cap),
            _ => None,
        });
        let Some(l) = l.filter(|l| *l > 0) else {
            continue;
        };
        let mut locals = HashSet::new();
        collect_lets(body, &mut locals);
        let Some((q, kc, vc)) = attention_qkv(body, &locals) else { continue };
        if h == 0 || hd == 0 || vd == 0 || rows < 2 {
            continue;
        }
        return Some(AttentionSpec {
            q,
            kc,
            vc,
            out,
            h,
            kv,
            hd,
            vd,
            scale,
            softcap,
            l: l as usize,
            pos: Some(pos),
            rows,
            window,
            sinks,
        });
    }
    None
}

/// The masked attention's bound: `Let len be P` and the check `len ≥ 1 and len ≤ L` at the top of the block —
/// `(L, P)`, with `P` a named scalar or a literal (an expression bound is not a lifted form).
fn masked_length(body: &[Stmt], len: Symbol) -> Option<(usize, Option<RowIndex>)> {
    let value = body.iter().find_map(|s| match s {
        Stmt::Let { var, value, .. } if *var == len => Some(*value),
        _ => None,
    })?;
    let pos = match value {
        Expr::Identifier(p) => RowIndex::Named(*p),
        other => RowIndex::Literal(as_int(other)?),
    };
    let rows = body.iter().find_map(|s| match s {
        Stmt::RuntimeAssert {
            condition:
                Expr::BinaryOp {
                    op: BinaryOpKind::And,
                    right,
                    ..
                },
            ..
        } => match right {
            Expr::BinaryOp {
                op: BinaryOpKind::LtEq,
                left: Expr::Identifier(v),
                right: r,
            } if *v == len => as_int(r),
            _ => None,
        },
        _ => None,
    })?;
    (rows > 0).then_some((rows as usize, Some(pos)))
}

// ── lut attention: the int8 model's softmax ───────────────────────────────────────────────────────────

/// `Let C be lut attention of Q over K and V with H heads KV groups [up to P] with table E and scale M shifted right
/// by S` — single-query attention over Int tensors whose softmax is an exp table: per head, scores `q · k_j`, their
/// maximum, weights `E[clamp(((max − s_j) · M) >> S, 0, table_len − 1)]` and the context `⌊Σ w_j · v_j / Σ w_j⌋`.
/// The shapes and `pos` are [`AttentionSpec`]'s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LutAttentionSpec {
    pub q: Symbol,
    pub kc: Symbol,
    pub vc: Symbol,
    pub table: Symbol,
    pub out: Symbol,
    pub h: usize,
    pub kv: usize,
    pub hd: usize,
    pub l: usize,
    pub table_len: usize,
    pub pos: Option<RowIndex>,
    pub m: i64,
    pub s: u32,
}

/// Whether any expression a statement list holds, at any depth, satisfies `f`.
fn any_expr(body: &[Stmt], f: &dyn Fn(&Expr) -> bool) -> bool {
    fn ex(e: &Expr, f: &dyn Fn(&Expr) -> bool) -> bool {
        f(e) || match e {
            Expr::BinaryOp { left, right, .. } => ex(left, f) || ex(right, f),
            Expr::Index { collection, index } => ex(collection, f) || ex(index, f),
            Expr::Call { args, .. } => args.iter().any(|a| ex(a, f)),
            _ => false,
        }
    }
    body.iter().any(|s| match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } | Stmt::Push { value, .. } => ex(value, f),
        Stmt::SetIndex { index, value, .. } => ex(index, f) || ex(value, f),
        Stmt::While { cond, body, .. } => ex(cond, f) || any_expr(body, f),
        Stmt::Repeat { body, .. } | Stmt::Splice { body } => any_expr(body, f),
        Stmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => ex(cond, f) || any_expr(then_block, f) || else_block.is_some_and(|eb| any_expr(eb, f)),
        _ => false,
    })
}

/// A float softmax's exponent `E^(…)` somewhere in the block — the float attention forms' own signature.
fn exp_softmax(body: &[Stmt]) -> bool {
    any_expr(body, &|e| {
        as_binop(e, BinaryOpKind::Pow).is_some_and(|(base, _)| is_e(base))
    })
}

/// `E[min(max(((mx − sc[j]) · M) >> S, 0), N − 1) + 1]` → `(E, M, S, N)`: the LUT weight read.
fn lut_read<'a>(e: &'a Expr<'a>, interner: &Interner) -> Option<(Symbol, i64, u32, usize)> {
    let (table, at) = indexed(e)?;
    let (idx, one) = as_binop(at, BinaryOpKind::Add)?;
    want(is_int(one, 1))?;
    let [lower, last] = call_of(idx, interner, "min")? else {
        return None;
    };
    let [shifted, zero] = call_of(lower, interner, "max")? else {
        return None;
    };
    want(is_int(zero, 0))?;
    let (prod, shift) = as_binop(shifted, BinaryOpKind::Shr)?;
    let (diff, m) = as_binop(prod, BinaryOpKind::Multiply)?;
    as_binop(diff, BinaryOpKind::Subtract)?;
    let s = u32::try_from(as_int(shift)?).ok().filter(|s| *s <= 63)?;
    let n = usize::try_from(as_int(last)? + 1).ok().filter(|n| *n >= 1)?;
    Some((table, as_int(m)?, s, n))
}

/// The LUT weight read a block binds, at any depth.
fn find_lut_read<'a>(body: &'a [Stmt<'a>], interner: &Interner) -> Option<(Symbol, i64, u32, usize)> {
    body.iter().find_map(|s| match s {
        Stmt::Let { value, .. } => lut_read(value, interner),
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => find_lut_read(body, interner),
        _ => None,
    })
}

/// Whether a block writes an element by floor division, at any depth — the integer softmax's normalization.
fn floor_divides(body: &[Stmt]) -> bool {
    body.iter().any(|s| match s {
        Stmt::SetIndex { value, .. } => as_binop(value, BinaryOpKind::FloorDivide).is_some(),
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => floor_divides(body),
        _ => false,
    })
}

/// Recognize the `lut attention` block: a `Splice` holding the LUT weight read and a floor-division write, its head
/// loop and cache bound the single-query attention's (`attention_heads`, `masked_length`). Its operands, the
/// tensors it reads by index and does not bind, are `Q, K, E, V` in program order; the table is the one the LUT read
/// names.
pub fn recognize_lut_attention(stmts: &[Stmt], interner: &Interner) -> Option<LutAttentionSpec> {
    for s in stmts {
        let Stmt::Splice { body } = s else { continue };
        let Some((table, m, shift, table_len)) = find_lut_read(body, interner) else {
            continue;
        };
        if !floor_divides(body) || exp_softmax(body) {
            continue;
        }
        let Some(out) = body.iter().find_map(|st| match st {
            Stmt::Let { var, value, .. } if is_new(value) => Some(*var),
            _ => None,
        }) else {
            continue;
        };
        let Some(HeadLoops { h, group, bound, hd, .. }) = head_loops(body) else {
            continue;
        };
        if group == 0 || h % group != 0 {
            continue;
        }
        let (l, pos) = match bound {
            Expr::Identifier(len) => match masked_length(body, *len) {
                Some(masked) => masked,
                None => continue,
            },
            other => (as_int(other).unwrap_or(0) as usize, None),
        };
        let mut locals = HashSet::new();
        collect_lets(body, &mut locals);
        let mut ops: Vec<Symbol> = Vec::new();
        attention_operands(body, &locals, &mut ops);
        ops.retain(|o| *o != table);
        if ops.len() != 3 || h == 0 || hd == 0 || l == 0 {
            continue;
        }
        return Some(LutAttentionSpec {
            q: ops[0],
            kc: ops[1],
            vc: ops[2],
            table,
            out,
            h,
            kv: h / group,
            hd,
            l,
            table_len,
            pos,
            m,
            s: shift,
        });
    }
    None
}

// ── moe ────────────────────────────────────────────────────────────────────────────────────────────────

fn moe_ident(e: &Expr) -> Option<Symbol> {
    match e {
        Expr::Identifier(s) => Some(*s),
        _ => None,
    }
}
fn moe_num(e: &Expr) -> Option<i64> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        _ => None,
    }
}
/// The distinct identifier symbols an index expression mentions (its loop-var arity distinguishes the
/// operands: `x[t]` = 1, `wr[(t-1)·E+e]` = 2, `wg[((e-1)·dff+(o-1))·d+t]` = 3).
fn moe_idx_vars(e: &Expr, out: &mut HashSet<Symbol>) {
    match e {
        Expr::Identifier(s) => {
            out.insert(*s);
        }
        Expr::BinaryOp { left, right, .. } => {
            moe_idx_vars(left, out);
            moe_idx_vars(right, out);
        }
        _ => {}
    }
}
/// `E` from a router index `(t-1)·E + e` = `Add(Mul(_, E), _)`.
fn moe_ne_from(idx: &Expr) -> Option<usize> {
    if let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        ..
    } = idx
    {
        if let Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            right,
            ..
        } = &**left
        {
            return moe_num(right).map(|n| n as usize);
        }
    }
    None
}
/// `(d, dff)` from a gate index `((e-1)·dff + (o-1))·d + t` = `Add(Mul(Add(Mul(_, dff), _), d), _)`.
fn moe_d_dff_from(idx: &Expr) -> Option<(usize, usize)> {
    if let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        ..
    } = idx
    {
        if let Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            left: inner,
            right: d,
        } = &**left
        {
            if let Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left: mul2,
                ..
            } = &**inner
            {
                if let Expr::BinaryOp {
                    op: BinaryOpKind::Multiply,
                    right: dff,
                    ..
                } = &**mul2
                {
                    return Some((moe_num(d)? as usize, moe_num(dff)? as usize));
                }
            }
        }
    }
    None
}

/// Recognize a MoE block anywhere in the statement list (through `Splice`). `None` if not the `moe` shape.
pub fn recognize_moe(stmts: &[Stmt]) -> Option<MoeSpec> {
    // The MoE desugar is the ONLY block that masks its router probs with a large negative sentinel via
    // `SetIndex` — the top-K argmax. Find that block's body.
    fn find_body<'a, 'b>(stmts: &'b [Stmt<'a>]) -> Option<&'b [Stmt<'a>]> {
        for s in stmts {
            if let Stmt::Splice { body } = s {
                if masks(body) {
                    return Some(body);
                }
                if let Some(b) = find_body(body) {
                    return Some(b);
                }
            }
        }
        None
    }
    let body = find_body(stmts)?;

    // Temps are Let-bound inside the block; operands are the tensors indexed but bound OUTSIDE.
    let mut locals = HashSet::new();
    collect_lets(body, &mut locals);

    // Every operand index access, in program order: (operand symbol, its index expression).
    let mut accesses: Vec<(Symbol, &Expr)> = Vec::new();
    fn walk_idx<'a, 'b>(
        body: &'b [Stmt<'a>],
        locals: &HashSet<Symbol>,
        out: &mut Vec<(Symbol, &'b Expr<'a>)>,
    ) {
        fn expr<'a, 'b>(
            e: &'b Expr<'a>,
            locals: &HashSet<Symbol>,
            out: &mut Vec<(Symbol, &'b Expr<'a>)>,
        ) {
            if let Expr::Index { collection, index } = e {
                if let Some(s) = moe_ident(collection) {
                    if !locals.contains(&s) {
                        out.push((s, index));
                    }
                }
                expr(index, locals, out);
            }
            if let Expr::BinaryOp { left, right, .. } = e {
                expr(left, locals, out);
                expr(right, locals, out);
            }
        }
        for s in body {
            match s {
                Stmt::Set { value, .. } => expr(value, locals, out),
                Stmt::SetIndex { index, value, .. } => {
                    expr(index, locals, out);
                    expr(value, locals, out);
                }
                Stmt::Push { value, .. } => expr(value, locals, out),
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => {
                    walk_idx(body, locals, out)
                }
                Stmt::If {
                    cond,
                    then_block,
                    else_block,
                } => {
                    expr(cond, locals, out);
                    walk_idx(then_block, locals, out);
                    if let Some(eb) = else_block {
                        walk_idx(eb, locals, out);
                    }
                }
                _ => {}
            }
        }
    }
    walk_idx(body, &locals, &mut accesses);

    // Classify operands by index arity: x=1, wr=2, {wg,wu,wd}=3 (in first-appearance order).
    let arity = |idx: &Expr| {
        let mut v = HashSet::new();
        moe_idx_vars(idx, &mut v);
        v.len()
    };
    let mut x = None;
    let mut wr = None;
    let mut wr_idx = None;
    let mut three: Vec<(Symbol, &Expr)> = Vec::new();
    let mut seen3 = HashSet::new();
    for &(s, idx) in &accesses {
        match arity(idx) {
            1 => {
                if x.is_none() {
                    x = Some(s);
                }
            }
            2 => {
                if wr.is_none() {
                    wr = Some(s);
                    wr_idx = Some(idx);
                }
            }
            3 => {
                if seen3.insert(s) {
                    three.push((s, idx));
                }
            }
            _ => {}
        }
    }
    let (x, wr) = (x?, wr?);
    let ne = moe_ne_from(wr_idx?)?;
    let (wg, wg_idx) = three.first().copied()?;
    let (wu, _) = three.get(1).copied()?;
    let (wd, _) = three.get(2).copied()?;
    let (d, dff) = moe_d_dff_from(wg_idx)?;

    // `out` = the tensor written by the weighted accumulation `SetIndex out[o] = out[o] + w[e]·dacc`.
    fn find_out(body: &[Stmt]) -> Option<Symbol> {
        for s in body {
            match s {
                Stmt::SetIndex {
                    collection, value, ..
                } => {
                    if let (
                        Some(c),
                        Expr::BinaryOp {
                            op: BinaryOpKind::Add,
                            left,
                            ..
                        },
                    ) = (moe_ident(collection), value)
                    {
                        if let Expr::Index { collection: lc, .. } = &**left {
                            if moe_ident(lc) == Some(c) {
                                return Some(c);
                            }
                        }
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => {
                    if let Some(o) = find_out(body) {
                        return Some(o);
                    }
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    if let Some(o) =
                        find_out(then_block).or_else(|| else_block.and_then(|eb| find_out(eb)))
                    {
                        return Some(o);
                    }
                }
                _ => {}
            }
        }
        None
    }
    let out = find_out(body)?;

    // `k` = the bound of the round loop: the `While` whose body masks a score with the negative sentinel and writes the
    // weight it picked (`W[b] = P[b]`) — a group limit's rounds mask their groups and write no weight.
    fn find_k(body: &[Stmt]) -> Option<usize> {
        for s in body {
            if let Stmt::While { cond, body: wb, .. } = s {
                let masks_here = wb.iter().any(|s| matches!(s, Stmt::SetIndex { value: Expr::Literal(Literal::Float(f)), .. } if *f < -1.0e29))
                    && wb.iter().any(|s| matches!(s, Stmt::SetIndex { value: Expr::Index { .. }, .. }));
                if masks_here {
                    if let Expr::BinaryOp {
                        op: BinaryOpKind::LtEq,
                        right,
                        ..
                    } = cond
                    {
                        return moe_num(right).map(|n| n as usize);
                    }
                }
            }
            if let Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } = s
            {
                if let Some(k) = find_k(body) {
                    return Some(k);
                }
            }
        }
        None
    }
    let k = find_k(body)?;
    let (score, weigh_inputs) = moe_routing(body);
    let (router_bias, biases, act) = moe_options(body, &locals, wg, wu);
    // The renormalization divides each kept weight by their sum: `W[i] = W[i] / sum`, in a loop of its own.
    fn renormalizes(body: &[Stmt], locals: &HashSet<Symbol>) -> bool {
        body.iter().any(|s| match s {
            Stmt::While { body, .. } => body.iter().any(|s| {
                matches!(s, Stmt::SetIndex { collection, value: Expr::BinaryOp { op: BinaryOpKind::Divide, left, right }, .. }
                    if as_index(left).and_then(|(c, _)| as_ident(c)) == as_ident(collection)
                        && as_ident(right).is_some_and(|r| locals.contains(&r)))
            }),
            _ => false,
        })
    }
    let renormalize = renormalizes(body, &locals);
    let (selection_bias, groups, scale) = moe_choice(body, &locals);

    Some(MoeSpec {
        out,
        x,
        wr,
        wg,
        wu,
        wd,
        d,
        dff,
        ne,
        k,
        router_bias,
        biases,
        renormalize,
        act,
        score,
        weigh_inputs,
        selection_bias,
        groups,
        scale,
    })
}

/// A mixture's choice, read off its desugar: the selection bias is the operand added to a score as it is copied for the
/// choosing (`P[i] + B[i]`, pushed); the group limit's rounds are the loop that marks a group chosen with `−2e30`, its
/// bound `G`, the bound of the scan inside it `N`; the routed scale is the literal a weight is multiplied by in place
/// (`W[i] = W[i]·S`).
fn moe_choice(body: &[Stmt], locals: &HashSet<Symbol>) -> (Option<Symbol>, Option<MoeGroups>, Option<u64>) {
    fn bound(cond: &Expr) -> Option<usize> {
        match cond {
            Expr::BinaryOp { op: BinaryOpKind::LtEq, right, .. } => moe_num(right).and_then(|n| usize::try_from(n).ok()),
            _ => None,
        }
    }
    fn walk(body: &[Stmt], locals: &HashSet<Symbol>, found: &mut (Option<Symbol>, Option<MoeGroups>, Option<u64>)) {
        for s in body {
            match s {
                Stmt::Push { value, collection } if moe_ident(collection).is_some_and(|c| locals.contains(&c)) => {
                    if let Some((score, bias)) = as_binop(value, BinaryOpKind::Add) {
                        let local_score = as_index(score).and_then(|(c, _)| as_ident(c)).is_some_and(|c| locals.contains(&c));
                        let operand = as_index(bias).and_then(|(c, _)| as_ident(c)).filter(|b| !locals.contains(b));
                        if let (true, Some(b)) = (local_score, operand) {
                            found.0 = Some(b);
                        }
                    }
                }
                Stmt::SetIndex { collection, value, .. } => {
                    if let Some((read, by)) = as_binop(value, BinaryOpKind::Multiply) {
                        if let (Some((c, _)), Some(v)) = (as_index(read), as_float(by)) {
                            if as_ident(c).is_some() && as_ident(c) == moe_ident(collection) {
                                found.2 = Some(v.to_bits());
                            }
                        }
                    }
                }
                Stmt::While { cond, body: wb, .. } => {
                    let marks = wb.iter().any(|s| matches!(s, Stmt::SetIndex { value: Expr::Literal(Literal::Float(f)), .. } if *f < -1.5e30));
                    if marks {
                        let count = wb.iter().find_map(|s| match s {
                            Stmt::While { cond, .. } => bound(cond),
                            _ => None,
                        });
                        if let (Some(kept), Some(count)) = (bound(cond), count) {
                            found.1 = Some(MoeGroups { count, kept });
                        }
                    }
                    walk(wb, locals, found);
                }
                Stmt::Repeat { body, .. } | Stmt::Splice { body } => walk(body, locals, found),
                Stmt::If { then_block, else_block, .. } => {
                    walk(then_block, locals, found);
                    if let Some(eb) = else_block {
                        walk(eb, locals, found);
                    }
                }
                _ => {}
            }
        }
    }
    let mut found = (None, None, None);
    walk(body, locals, &mut found);
    found
}

/// How a mixture scores its experts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MoeScore {
    /// The softmax over every router logit.
    Softmax,
    /// Each logit's own sigmoid (`by sigmoid`).
    Sigmoid,
}

/// A mixture's scoring and whether a kept expert's weight scales its input (`weighting the inputs`), read off its
/// desugar: the sigmoid's score is `1 / (1 + e^(0 − L[e]))`, the one push of the block whose value divides the literal
/// one (the softmax divides an exponential by the sum); the weighted inputs mark each kept expert in a selection, the
/// one `SetIndex` of the block that writes the literal one.
fn moe_routing(body: &[Stmt]) -> (MoeScore, bool) {
    fn walk(body: &[Stmt], sigmoid: &mut bool, selects: &mut bool) {
        for s in body {
            match s {
                Stmt::Push { value: Expr::BinaryOp { op: BinaryOpKind::Divide, left, .. }, .. } if as_float(left) == Some(1.0) => {
                    *sigmoid = true
                }
                Stmt::SetIndex { value, .. } if as_float(value) == Some(1.0) => *selects = true,
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => walk(body, sigmoid, selects),
                _ => {}
            }
        }
    }
    let (mut sigmoid, mut selects) = (false, false);
    walk(body, &mut sigmoid, &mut selects);
    (if sigmoid { MoeScore::Sigmoid } else { MoeScore::Softmax }, selects)
}

/// A mixture's options, read off its desugar: the router's bias is the operand its logit push adds (`acc + Br[e]`), the
/// gate's and the up's the operands their pushes add to the accumulator of `wg` and of `wu` (`acc + Bg[…]`, the `acc`
/// summing `x[t]·Wg[…]`), the down's the operand its weighted accumulation adds to the expert's sum
/// (`out[o] + w[e]·(acc + Bd[…])`); the activation is clamped when the gate it sharpens is `min(g, L)` (`A·min(g, L)`).
fn moe_options(body: &[Stmt], locals: &HashSet<Symbol>, wg: Symbol, wu: Symbol) -> (Option<Symbol>, [Option<Symbol>; 3], MoeAct) {
    /// `acc + B[i]` or `W[e]·acc + B[i]`: the accumulator, `B`, and whether `i` is a plain loop variable (the router's
    /// logit `e`) rather than an expert's offset.
    fn added(e: &Expr, locals: &HashSet<Symbol>) -> Option<(Symbol, Symbol, bool)> {
        let (acc, read) = as_binop(e, BinaryOpKind::Add)?;
        let acc = as_ident(acc).or_else(|| as_binop(acc, BinaryOpKind::Multiply).and_then(|(_, a)| as_ident(a)))?;
        let (b, at) = as_index(read)?;
        let b = as_ident(b).filter(|b| !locals.contains(b))?;
        Some((acc, b, as_ident(at).is_some()))
    }
    /// `acc + v·W[i]` (either factor order): the weight `W` an accumulation reads.
    fn summed_weight(e: &Expr) -> Option<Symbol> {
        let (_, term) = as_binop(e, BinaryOpKind::Add)?;
        let (l, r) = as_binop(term, BinaryOpKind::Multiply)?;
        [r, l].into_iter().find_map(|f| as_index(f).and_then(|(w, _)| as_ident(w)))
    }
    fn clamp_of(e: &Expr) -> Option<(f64, f64)> {
        match e {
            Expr::BinaryOp { op: BinaryOpKind::Multiply, left, right } => {
                if let (Some(slope), Expr::Call { args, .. }) = (as_float(left), &**right) {
                    if let [_, limit] = args.as_slice() {
                        if let Some(limit) = as_float(limit) {
                            return Some((limit, slope));
                        }
                    }
                }
                clamp_of(left).or_else(|| clamp_of(right))
            }
            Expr::BinaryOp { left, right, .. } => clamp_of(left).or_else(|| clamp_of(right)),
            Expr::Call { args, .. } => args.iter().find_map(|a| clamp_of(a)),
            _ => None,
        }
    }
    #[derive(Default)]
    struct Found {
        pushes: Vec<(Symbol, Symbol, bool)>,
        /// Each accumulator and the weight it sums.
        sums: HashMap<Symbol, Symbol>,
        down: Option<Symbol>,
        clamp: Option<(f64, f64)>,
    }
    fn walk(body: &[Stmt], locals: &HashSet<Symbol>, found: &mut Found) {
        for s in body {
            match s {
                Stmt::Push { value, .. } => found.pushes.extend(added(value, locals)),
                Stmt::SetIndex { value, .. } => {
                    if let Some((_, weighted)) = as_binop(value, BinaryOpKind::Add) {
                        if let Some((_, sum)) = as_binop(weighted, BinaryOpKind::Multiply) {
                            if let Some((_, b, _)) = added(sum, locals) {
                                found.down = Some(b);
                            }
                        }
                    }
                }
                Stmt::Set { target, value } => {
                    if let Some(w) = summed_weight(value) {
                        found.sums.insert(*target, w);
                    }
                    if found.clamp.is_none() {
                        found.clamp = clamp_of(value);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => walk(body, locals, found),
                _ => {}
            }
        }
    }
    let mut found = Found::default();
    walk(body, locals, &mut found);
    let router_bias = found.pushes.iter().find(|(_, _, plain)| *plain).map(|(_, b, _)| *b);
    let bias_of = |w: Symbol| {
        found.pushes.iter().find(|(acc, _, plain)| !*plain && found.sums.get(acc) == Some(&w)).map(|(_, b, _)| *b)
    };
    let act = match found.clamp {
        Some((limit, slope)) => MoeAct::ClampedSwiglu { limit: limit.to_bits(), slope: slope.to_bits() },
        None => MoeAct::Swiglu,
    };
    (router_bias, [bias_of(wg), bias_of(wu), found.down], act)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/tensor_recognize/qwen/tests.rs"]
mod tests;
