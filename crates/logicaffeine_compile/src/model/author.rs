//! A model's computation written as a LOGOS program, from its [`ModelSpec`].
//!
//! Every program here is ordinary LOGOS tensor sugar — the claim "the model is authored in LOGOS" rests on it — and
//! nothing in it is written for one model: a layer's statements follow from its mixer, its feed-forward block and
//! its norms. The forms are the ones a deployment runs: the single-token forward, a decode step over KV caches and
//! recurrent state, a prefill chunk, a batch of decode steps; each also cut into its embedding, its layers and its
//! head, so a backend can lower a layer at a time. Weights are declared as tensors the host fills, and recorded with
//! the file each is loaded from: `<var>.f` (f32) or `<var>.q`/`<var>.s` (Q8_0 quants and block scales).
//!
//! A stacked form (prefill, batch) carries its tensors with a `b` prefix (`bxn0`); per-sequence state carries a
//! `_s{b}` suffix (`Kc3_s1`).

use std::collections::HashMap;

use super::bind::{rotation_of, rotations, HostTable, RotaryTable};
use super::spec::{Act, Attention, Ffn, GateActivation, GatedDelta, Gating, Mixer, Mla, ModelSpec};

/// A program and its weight manifest: `(source, f32 weights → (file, count), q8 weights → file stem)`.
pub type Authored = (
    String,
    HashMap<String, (String, usize)>,
    HashMap<String, String>,
);

/// Why a spec cannot be written as a program yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotYet(pub &'static str);

impl std::fmt::Display for NotYet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the language cannot write {} yet", self.0)
    }
}

/// Can every layer of `spec` be written in the language as it stands? The sugar's rotation is NEOX-paired (an
/// interleaved checkpoint is bound through a per-head permutation of its Q and K rows, a fused projection through each
/// part's rows); an attention reads values of any width at the scale it gives, its scores and the logits soft-capped
/// where the model caps them, and a latent attention its latent at the scale it gives.
pub fn check(spec: &ModelSpec) -> Result<(), NotYet> {
    if !(spec.rms_eps.is_finite() && spec.rms_eps > 0.0) {
        return Err(NotYet("an RMSNorm epsilon that is not a positive number"));
    }
    if spec.final_logit_softcap.is_some_and(|cap| !(cap.is_finite() && cap > 0.0)) {
        return Err(NotYet("a final logit softcap that is not a positive number"));
    }
    // Qwen3.8-Flash-Next's parts: a program written without them would compute another model.
    if spec.hyper.is_some() {
        return Err(NotYet("hyper-connections in place of the norms"));
    }
    if spec.ple.is_some() {
        return Err(NotYet("a per-layer n-gram embedding"));
    }
    if spec.indexer.as_ref().is_some_and(|ix| !ix.layers.is_empty()) {
        return Err(NotYet("a sparse selection of the attended positions (QSA)"));
    }
    for layer in &spec.layers {
        if let Mixer::Attention(a) = &layer.mixer {
            if a.output_gate && (a.fused_qkv || a.value_dim != a.head_dim) {
                return Err(NotYet("an output gate on a fused projection or over values unlike the keys in width"));
            }
            if a.softcap.is_some_and(|cap| !(cap.is_finite() && cap > 0.0)) {
                return Err(NotYet("an attention softcap that is not a positive number"));
            }
            if a.temperature.is_some() && a.head_dim % 2 != 0 {
                return Err(NotYet("an attention temperature over an odd head width"));
            }
        }
        match &layer.ffn {
            Ffn::Dense { act, .. } if !matches!(act, Act::Silu | Act::GeluTanh) => {
                return Err(NotYet("a gated activation other than SiLU or GeLU"))
            }
            Ffn::Moe(m) if !matches!(m.act, Act::Silu | Act::SwigluOai { .. }) => {
                return Err(NotYet("an expert mixture other than top-k experts gated by a SwiGLU"))
            }
            Ffn::Moe(m) if !(m.weights_scale.is_finite() && m.weights_scale >= 0.0) => {
                return Err(NotYet("a routed scale that is not a positive number"))
            }
            Ffn::Moe(m) if m.shared_experts > 0 && m.act != Act::Silu => {
                return Err(NotYet("shared experts gated other than by a SwiGLU"))
            }
            // A softmax over the kept logits chooses by the logits themselves, so its groups would rank by logits where
            // the sugar ranks them by scores.
            Ffn::Moe(m) if m.gating == Gating::SoftmaxOfTopK && m.groups.is_some() => {
                return Err(NotYet("a group limit over a softmax of the kept logits"))
            }
            Ffn::Moe(m)
                if m.groups.is_some_and(|(count, used)| {
                    count == 0 || m.experts % count != 0 || !(1..=count).contains(&used) || used * (m.experts / count) < m.top_k
                }) =>
            {
                return Err(NotYet("a group limit that does not split the experts evenly or keeps fewer than it chooses"))
            }
            _ => {}
        }
    }
    Ok(())
}

/// The program `src` with every norm statement given the model's epsilon `eps` — the RMSNorms, the gated delta rule's
/// output norm and the L2 norms of its queries and keys, all of which the model computes with its one epsilon. The
/// sugar's own epsilon is 1e-6, so a model with that one is written without the clause.
fn with_epsilon(src: &str, eps: f64) -> String {
    if eps == 1e-6 {
        return src.to_string();
    }
    src.lines()
        .map(|line| {
            let norm = [" be rmsnorm of ", " be l2norm of ", " be gated delta of "].iter().any(|n| line.contains(n));
            match line.strip_suffix('.') {
                Some(stmt) if norm && line.starts_with("Let ") => format!("{stmt} with epsilon {eps:?}.\n"),
                _ => format!("{line}\n"),
            }
        })
        .collect()
}

/// How an attention layer reads its keys and values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttnForm {
    /// The current token's own key and value — the forward of one token with no history.
    Current,
    /// A decode step's KV cache of `ctx` positions: this token's key and value are appended as row `pos`, and the
    /// layer attends over the first `pos` rows. The caches are the step's carried state.
    Cached { ctx: usize },
}

/// Whose carried state the rows of a stacked step update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    /// Each row is its own sequence with its own state — a batched decode step (`Kc3_s1`).
    PerSequence,
    /// Every row is the next token of ONE sequence — a prefill chunk: the rows update the sequence's single state
    /// (`Kc3`, the decode step's own), one after another.
    Shared,
}

impl RowState {
    /// The name row `b`'s copy of the state tensor `base` goes by.
    fn state(self, base: &str, b: usize) -> String {
        match self {
            RowState::PerSequence => format!("{base}_s{b}"),
            RowState::Shared => base.to_string(),
        }
    }

    /// The rows whose copy of the state is declared: every row's, or the one the rows share.
    fn holders(self, bsz: usize) -> std::ops::Range<usize> {
        match self {
            RowState::PerSequence => 0..bsz,
            RowState::Shared => 0..1,
        }
    }
}

/// A program being written, and the weights it declares.
struct Program<'a> {
    spec: &'a ModelSpec,
    dir: &'a str,
    s: String,
    f32w: HashMap<String, (String, usize)>,
    q8w: HashMap<String, String>,
}

/// The widths a layer's statements use: the queries' and keys' `aq` and `akv`, the values' `avd` and the attended
/// rows' `ao` (`kv_heads` and `heads` of `value_dim`).
struct Dims {
    d: usize,
    aq: usize,
    akv: usize,
    avd: usize,
    ao: usize,
    hd: usize,
    h: usize,
    kv: usize,
}

fn attn_dims(spec: &ModelSpec, a: &Attention) -> Dims {
    Dims {
        d: spec.d,
        aq: a.heads * a.head_dim,
        akv: a.kv_heads * a.head_dim,
        avd: a.kv_heads * a.value_dim,
        ao: a.heads * a.value_dim,
        hd: a.head_dim,
        h: a.heads,
        kv: a.kv_heads,
    }
}

impl<'a> Program<'a> {
    fn new(spec: &'a ModelSpec, dir: &'a str) -> Self {
        Program {
            spec,
            dir,
            s: String::from("## Main\n"),
            f32w: HashMap::new(),
            q8w: HashMap::new(),
        }
    }

    fn finish(self) -> Authored {
        (with_epsilon(&self.s, self.spec.rms_eps), self.f32w, self.q8w)
    }

    fn decl_f32(&mut self, var: &str, r: usize, c: usize) {
        self.s.push_str(&format!("Let {var} be a {r} by {c} tensor of Real.\n"));
        self.f32w
            .insert(var.to_string(), (format!("{}/{var}.f", self.dir), r * c));
    }

    fn decl_q8(&mut self, var: &str, r: usize, c: usize) {
        self.s.push_str(&format!("Let {var} be a {r} by {c} tensor of Real.\n"));
        self.q8w.insert(var.to_string(), format!("{}/{var}", self.dir));
    }

    /// The rotary tables of every rotation the layers `only` names (all of them when `None`) turn by: a decode step's
    /// caches of `ctx` positions (`cached`), or a forward's one row — `dims / 2` angles a position.
    fn rotary_tables(&mut self, only: Option<usize>, cached: bool, ctx: usize) {
        let spec = self.spec;
        for (rotation, rope) in rotations(spec).into_iter().enumerate() {
            if only.is_some_and(|l| rotation_of(spec, l) != Some(rotation)) {
                continue;
            }
            for cos in [true, false] {
                let name = RotaryTable { rotation, cached, cos }.name();
                self.decl_f32(&name, if cached { ctx } else { 1 }, rope.dims / 2);
            }
        }
    }

    /// A host table (`model::bind::HostTable`) declared once for the program, `rows` by `cols`: its name.
    fn host_table(&mut self, table: HostTable, rows: usize, cols: usize) -> String {
        let name = table.name();
        if !self.f32w.contains_key(&name) {
            self.decl_f32(&name, rows, cols);
        }
        name
    }

    /// Llama 4's query and key after the rotation: each normed per head (`norm_heads` query and key heads) by an RMSNorm
    /// whose weight is a row of ones, when the layer norms them; on a rotation-free layer the query (`rope_heads` heads)
    /// scaled by its position's attention temperature — a rotation whose cosine is the factor and whose sine is zero,
    /// read from position `from` of a decode step's tables of `ctx` positions, or a forward's one row when `from` is
    /// `None`. `name` names each intermediate from its base.
    fn llama4_qk(
        &mut self,
        a: &Attention,
        (q, k): (String, String),
        norm_heads: (usize, usize),
        rope_heads: usize,
        from: Option<(String, usize)>,
        name: &dyn Fn(&str) -> String,
    ) -> (String, String) {
        let (mut q, mut k) = (q, k);
        if a.qk_unit_norm {
            let ones = self.host_table(HostTable::Ones(a.head_dim), 1, a.head_dim);
            let (qu, ku, (hq, hkv)) = (name("qu"), name("ku"), norm_heads);
            self.s.push_str(&format!(
                "Let {qu} be rmsnorm of {q} with weight {ones} across {hq} heads.\n\
                 Let {ku} be rmsnorm of {k} with weight {ones} across {hkv} heads.\n"
            ));
            (q, k) = (qu, ku);
        }
        if a.temperature.is_some() {
            let half = a.head_dim / 2;
            let (cached, rows) = from.as_ref().map_or((false, 1), |(_, ctx)| (true, *ctx));
            let cos = self.host_table(HostTable::Temperature { cached, cos: true, half }, rows, half);
            let sin = self.host_table(HostTable::Temperature { cached, cos: false, half }, rows, half);
            let at = from.as_ref().map_or(String::new(), |(pos, _)| format!(" from position {pos}"));
            let qt = name("qt");
            self.s.push_str(&format!("Let {qt} be rope of {q} with cos {cos} and sin {sin}{at} across {rope_heads} heads.\n"));
            q = qt;
        }
        (q, k)
    }

    /// The angles layer `l`'s rope reads, as the sugar names them: the forward's one-row tables, or a decode step's
    /// caches from the position `pos` names.
    fn angles(&self, l: usize, form: AttnForm, pos: &str) -> String {
        let rotation = rotation_of(self.spec, l).expect("a layer that rotates");
        let cached = matches!(form, AttnForm::Cached { .. });
        let (cos, sin) = (RotaryTable { rotation, cached, cos: true }.name(), RotaryTable { rotation, cached, cos: false }.name());
        match form {
            AttnForm::Current => format!("cos {cos} and sin {sin}"),
            AttnForm::Cached { .. } => format!("cos {cos} and sin {sin} from position {pos}"),
        }
    }

    /// `out = act(gate)·up`, the gated activation of layer `l`'s dense FFN, written into the program; the names it
    /// binds, `out` last.
    fn glu(&mut self, l: usize, gate: &str, up: &str, out: &str) -> Vec<String> {
        match &self.spec.layers[l].ffn {
            Ffn::Dense { act: Act::GeluTanh, .. } => {
                let act = format!("{out}_a");
                self.s.push_str(&format!("Let {act} be gelu of {gate}.\nLet {out} be {act} hadamard {up}.\n"));
                vec![act, out.to_string()]
            }
            _ => {
                self.s.push_str(&format!("Let {out} be swiglu of {gate} and {up}.\n"));
                vec![out.to_string()]
            }
        }
    }

    /// The block output `x` as the residual adds it: `x` itself, or — after a mixer (`after_mixer`) or an FFN whose
    /// layer norms its output too (Gemma's sandwich) — `x` normed by the layer's post-norm weight.
    fn post_norm(&mut self, l: usize, x: &str, after_mixer: bool) -> String {
        let layer = &self.spec.layers[l];
        let (on, weight) = if after_mixer { (layer.post_mixer_norm, "pan") } else { (layer.post_ffn_norm, "pfn") };
        if !on {
            return x.to_string();
        }
        let w = format!("{weight}{l}");
        if !self.f32w.contains_key(&w) {
            self.decl_f32(&w, 1, self.spec.d);
        }
        self.s.push_str(&format!("Let {x}n be rmsnorm of {x} with weight {w}.\n"));
        format!("{x}n")
    }

    /// `Let out be x matmul w`, plus the row `bias` (declared once, `width` wide) when the layer has one — the matmul then
    /// binds `out_r` and `out` is the biased sum, so every later statement reads `out` alike.
    fn project(&mut self, out: &str, x: &str, w: &str, bias: Option<String>, width: usize) {
        match bias {
            Some(b) => {
                if !self.f32w.contains_key(&b) {
                    self.decl_f32(&b, 1, width);
                }
                self.s.push_str(&format!("Let {out}_r be {x} matmul {w}.\nLet {out} be {out}_r plus {b}.\n"));
            }
            None => self.s.push_str(&format!("Let {out} be {x} matmul {w}.\n")),
        }
    }

    /// An attention's sinks clause, ` with sinks Sk{l}` (the sinks declared once), or nothing.
    fn sinks(&mut self, l: usize, a: &Attention) -> String {
        if !a.sinks {
            return String::new();
        }
        let sk = format!("Sk{l}");
        if !self.f32w.contains_key(&sk) {
            self.decl_f32(&sk, 1, a.heads);
        }
        format!(" with sinks {sk}")
    }

    /// The embedding rows `rows` as the first layer reads them, bound to `out`: scaled by the model's embedding scale
    /// when it has one.
    fn embedded(&mut self, rows: &str, out: &str) {
        match self.spec.embedding_scale {
            Some(scale) => self.s.push_str(&format!("Let {out}e be {rows}.\nLet {out} be {out}e scaled by {scale:?}.\n")),
            None => self.s.push_str(&format!("Let {out} be {rows}.\n")),
        }
    }

    /// Layer `l`, reading the hidden state `cur` and producing `xo{l}`.
    fn layer(&mut self, l: usize, cur: &str, form: AttnForm) {
        let spec = self.spec;
        self.decl_f32(&format!("an{l}"), 1, spec.d);
        match &spec.layers[l].mixer {
            Mixer::Attention(a) => self.attention(l, a, cur, form),
            Mixer::GatedDelta(g) => self.gated_delta(l, g, cur),
            Mixer::Mla(m) => self.mla(l, m, cur, form),
        }
        self.ffn(l, cur, None);
    }

    /// A latent attention layer's weights and, in a decode step, its caches of `ctx` rows of the latent and the shared
    /// rotated key (`kv_lora_rank + qk_rope` wide) — one, or one a sequence of a batch of `bsz`.
    fn mla_weights(&mut self, l: usize, m: &Mla, ctx: Option<(usize, usize, RowState)>) {
        let d = self.spec.d;
        let (h, qk, lora, kw) = (m.heads, m.qk_nope + m.qk_rope, m.kv_lora_rank, m.kv_lora_rank + m.qk_rope);
        match m.q_lora_rank {
            None => self.decl_q8(&format!("Wq{l}"), d, h * qk),
            Some(r) => {
                self.decl_q8(&format!("Wqa{l}"), d, r);
                self.decl_f32(&format!("Qan{l}"), 1, r);
                self.decl_q8(&format!("Wqb{l}"), r, h * qk);
            }
        }
        self.decl_q8(&format!("Wkva{l}"), d, kw);
        self.decl_f32(&format!("Kan{l}"), 1, lora);
        self.decl_q8(&format!("Wkb{l}"), h * m.qk_nope, lora);
        self.decl_q8(&format!("Wvb{l}"), h * lora, m.v_dim);
        self.decl_q8(&format!("Wo{l}"), h * m.v_dim, d);
        if let Some((ctx, bsz, rows)) = ctx {
            for b in rows.holders(bsz) {
                self.s.push_str(&format!("Let {} be a {ctx} by {kw} tensor of Real.\n", rows.state(&format!("Kc{l}"), b)));
            }
        }
    }

    /// The position-free half of a latent attention, from its normed input `{p}xn{l}` (a stack when `p` is `b`): the
    /// query (through its low-rank halves when it has them) cut into each head's unrotated part — absorbed into the
    /// latent's space by `Wkb` as `{p}ql{l}` — and its rotated part `{p}qe{l}`; the latent `{p}kvn{l}`, RMS-normed; the
    /// shared key's rotated part `{p}kpe{l}`.
    fn mla_projections(&mut self, l: usize, m: &Mla, p: &str) {
        let (h, nope, rope, lora) = (m.heads, m.qk_nope, m.qk_rope, m.kv_lora_rank);
        let (qk, kw) = (nope + rope, lora + rope);
        match m.q_lora_rank {
            None => self.s.push_str(&format!("Let {p}qr{l} be {p}xn{l} matmul Wq{l}.\n")),
            Some(_) => self.s.push_str(&format!(
                "Let {p}qx{l} be {p}xn{l} matmul Wqa{l}.\n\
                 Let {p}qxn{l} be rmsnorm of {p}qx{l} with weight Qan{l}.\n\
                 Let {p}qr{l} be {p}qxn{l} matmul Wqb{l}.\n"
            )),
        }
        self.s.push_str(&format!(
            "Let {p}kva{l} be {p}xn{l} matmul Wkva{l}.\n\
             Let {p}kvl{l} be the first {lora} of every {kw} of {p}kva{l}.\n\
             Let {p}kpe{l} be the last {rope} of every {kw} of {p}kva{l}.\n\
             Let {p}kvn{l} be rmsnorm of {p}kvl{l} with weight Kan{l}.\n\
             Let {p}qn{l} be the first {nope} of every {qk} of {p}qr{l}.\n\
             Let {p}qe{l} be the last {rope} of every {qk} of {p}qr{l}.\n\
             Let {p}ql{l} be {p}qn{l} matmul Wkb{l} across {h} heads.\n"
        ));
    }

    /// The position's half: the shared key's part `{x}kpe{t}` and the queries' rotated parts `{x}qe{t}` turned by
    /// `angles`, joined after the latent `{x}kvn{t}` into the cache row `{x}kc{t}` and after the absorbed queries
    /// `{x}ql{t}` into each head's query `{x}qa{t}` — `x` the stack prefix and `t` the layer and sequence suffix.
    fn mla_turned(&mut self, m: &Mla, x: &str, t: &str, angles: &str) {
        let h = m.heads;
        self.s.push_str(&format!(
            "Let {x}kpp{t} be rope of {x}kpe{t} with {angles} across 1 heads.\n\
             Let {x}kc{t} be {x}kvn{t} joined with {x}kpp{t}.\n\
             Let {x}qpp{t} be rope of {x}qe{t} with {angles} across {h} heads.\n\
             Let {x}qa{t} be {x}ql{t} joined with {x}qpp{t} across {h} heads.\n"
        ));
    }

    /// MULTI-HEAD LATENT ATTENTION, ABSORBED (llama.cpp `models/deepseek2.cpp`'s MLA path): the cache holds one row a
    /// position, the RMS-normed latent then the shared rotated key; each head's query reaches the latent's space
    /// through `Wkb` (the key half of the expansion), so every head attends over the same rows, one key/value group —
    /// its values the rows' latent part — at the score scale the model gives; each head's attended latent leaves
    /// through `Wvb` (the value half), and `Wo` projects the heads. The same scores the expanded attention takes, over a
    /// cache `heads · (qk_nope + v_dim) / (kv_lora_rank + qk_rope)` times smaller.
    fn mla(&mut self, l: usize, m: &Mla, cur: &str, form: AttnForm) {
        let (d, h, lora, kw, scale) = (self.spec.d, m.heads, m.kv_lora_rank, m.kv_lora_rank + m.qk_rope, m.scale);
        let ctx = match form {
            AttnForm::Cached { ctx } => Some((ctx, 1, RowState::Shared)),
            AttnForm::Current => None,
        };
        self.mla_weights(l, m, ctx);
        self.s.push_str(&format!("Let xn{l} be rmsnorm of {cur} with weight an{l}.\n"));
        let angles = self.angles(l, form, "pos");
        self.mla_projections(l, m, "");
        self.mla_turned(m, "", &l.to_string(), &angles);
        match form {
            AttnForm::Current => self.s.push_str(&format!(
                "Let Vl{l} be the first {lora} of every {kw} of kc{l}.\n\
                 Let ol{l} be attention of qa{l} over kc{l} and Vl{l} with {h} heads 1 groups scaled by {scale:?}.\n"
            )),
            AttnForm::Cached { .. } => self.s.push_str(&format!(
                "Set row pos of Kc{l} to kc{l}.\n\
                 Let Vl{l} be the first {lora} of every {kw} of Kc{l}.\n\
                 Let ol{l} be attention of qa{l} over Kc{l} and Vl{l} with {h} heads 1 groups up to pos scaled by {scale:?}.\n"
            )),
        }
        self.s.push_str(&format!("Let ct{l} be ol{l} matmul Wvb{l} across {h} heads.\n"));
        self.project(&format!("ao{l}"), &format!("ct{l}"), &format!("Wo{l}"), None, d);
    }

    /// [`Self::mla`] on a stack of `bsz` rows: the position-free half on the whole stack, then one sequence's chunk
    /// turning and appending its rows as a block and attending as one causal block, or each row its own sequence's
    /// step.
    fn stacked_mla(&mut self, l: usize, m: &Mla, cur: &str, ctx: usize, bsz: usize, rows: RowState) {
        let (d, h, lora, kw, scale) = (self.spec.d, m.heads, m.kv_lora_rank, m.kv_lora_rank + m.qk_rope, m.scale);
        self.mla_weights(l, m, Some((ctx, bsz, rows)));
        self.s.push_str(&format!("Let bxn{l} be rmsnorm of {cur} with weight an{l}.\n"));
        self.mla_projections(l, m, "b");
        match rows {
            RowState::Shared => {
                let angles = self.angles(l, AttnForm::Cached { ctx }, "pos_s0");
                self.mla_turned(m, "b", &l.to_string(), &angles);
                let kc = rows.state(&format!("Kc{l}"), 0);
                self.s.push_str(&format!(
                    "Set rows from pos_s0 of {kc} to bkc{l}.\n\
                     Let Vl{l} be the first {lora} of every {kw} of {kc}.\n\
                     Let bol{l} be causal attention of bqa{l} over {kc} and Vl{l} with {h} heads 1 groups from pos_s0 scaled by {scale:?}.\n"
                ));
            }
            // Each sequence's row turns by its own position, appends to its own cache and attends it.
            RowState::PerSequence => {
                self.s.push_str(&format!("Let bol{l} be a {bsz} by {} tensor of Real.\n", h * lora));
                for b in 0..bsz {
                    let (r, t) = (b + 1, format!("{l}_s{b}"));
                    let angles = self.angles(l, AttnForm::Cached { ctx }, &format!("pos_s{b}"));
                    for part in ["kvn", "kpe", "ql", "qe"] {
                        self.s.push_str(&format!("Let {part}{t} be row {r} of b{part}{l}.\n"));
                    }
                    self.mla_turned(m, "", &t, &angles);
                    let kc = rows.state(&format!("Kc{l}"), b);
                    self.s.push_str(&format!(
                        "Set row pos_s{b} of {kc} to kc{t}.\n\
                         Let Vl{t} be the first {lora} of every {kw} of {kc}.\n\
                         Let ol{t} be attention of qa{t} over {kc} and Vl{t} with {h} heads 1 groups up to pos_s{b} scaled by {scale:?}.\n\
                         Set row {r} of bol{l} to ol{t}.\n"
                    ));
                }
            }
        }
        self.s.push_str(&format!("Let bct{l} be bol{l} matmul Wvb{l} across {h} heads.\n"));
        self.project(&format!("bao{l}"), &format!("bct{l}"), &format!("Wo{l}"), None, d);
    }

    fn attention(&mut self, l: usize, a: &Attention, cur: &str, form: AttnForm) {
        let Dims { d, aq, akv, avd, ao, h, kv, .. } = attn_dims(self.spec, a);
        self.decl_q8(&format!("Wq{l}"), d, aq);
        if a.output_gate {
            self.decl_q8(&format!("Wqg{l}"), d, aq);
        }
        self.decl_q8(&format!("Wk{l}"), d, akv);
        self.decl_q8(&format!("Wv{l}"), d, avd);
        let ((nq, nk), (wq, wk)) = qk_norm_segments(a);
        if a.qk_norm {
            self.decl_f32(&format!("Qn{l}"), 1, wq);
            self.decl_f32(&format!("Kn{l}"), 1, wk);
        }
        self.decl_q8(&format!("Wo{l}"), ao, d);
        if let AttnForm::Cached { ctx } = form {
            self.s.push_str(&format!(
                "Let Kc{l} be a {ctx} by {akv} tensor of Real.\nLet Vc{l} be a {ctx} by {avd} tensor of Real.\n"
            ));
        }
        let bias = |b: &str| a.bias.then(|| format!("{b}{l}"));
        self.s.push_str(&format!("Let xn{l} be rmsnorm of {cur} with weight an{l}.\n"));
        self.project(&format!("qr{l}"), &format!("xn{l}"), &format!("Wq{l}"), bias("Bq"), aq);
        if a.output_gate {
            self.s.push_str(&format!("Let gr{l} be xn{l} matmul Wqg{l}.\n"));
        }
        self.project(&format!("kr{l}"), &format!("xn{l}"), &format!("Wk{l}"), bias("Bk"), akv);
        self.project(&format!("vr{l}"), &format!("xn{l}"), &format!("Wv{l}"), bias("Bv"), avd);
        let (mut q, mut k) = (format!("qr{l}"), format!("kr{l}"));
        if a.qk_norm {
            self.s.push_str(&format!(
                "Let qm{l} be rmsnorm of qr{l} with weight Qn{l} across {nq} heads.\n\
                 Let km{l} be rmsnorm of kr{l} with weight Kn{l} across {nk} heads.\n"
            ));
            (q, k) = (format!("qm{l}"), format!("km{l}"));
        }
        if a.rope.is_some() {
            let angles = self.angles(l, form, "pos");
            self.s.push_str(&format!(
                "Let qp{l} be rope of {q} with {angles} across {h} heads.\n\
                 Let kp{l} be rope of {k} with {angles} across {kv} heads.\n"
            ));
            (q, k) = (format!("qp{l}"), format!("kp{l}"));
        }
        let from = match form {
            AttnForm::Current => None,
            AttnForm::Cached { ctx } => Some(("pos".to_string(), ctx)),
        };
        (q, k) = self.llama4_qk(a, (q, k), (h, kv), h, from, &|base| format!("{base}{l}"));
        let (w, sk) = (score_clauses(a), self.sinks(l, a));
        match form {
            AttnForm::Current => self.s.push_str(&format!(
                "Let ct{l} be attention of {q} over {k} and vr{l} with {h} heads {kv} groups{w}{sk}.\n"
            )),
            AttnForm::Cached { .. } => self.s.push_str(&format!(
                "Set row pos of Kc{l} to {k}.\n\
                 Set row pos of Vc{l} to vr{l}.\n\
                 Let ct{l} be attention of {q} over Kc{l} and Vc{l} with {h} heads {kv} groups up to pos{w}{sk}.\n"
            )),
        }
        let gated = if a.output_gate {
            self.s.push_str(&format!("Let gs{l} be sigmoid of gr{l}.\nLet cg{l} be ct{l} hadamard gs{l}.\n"));
            format!("cg{l}")
        } else {
            format!("ct{l}")
        };
        self.project(&format!("ao{l}"), &gated, &format!("Wo{l}"), a.output_bias.then(|| format!("Bo{l}")), d);
    }

    fn gated_delta(&mut self, l: usize, g: &GatedDelta, cur: &str) {
        let gate = gate_clause(g);
        let d = self.spec.d;
        let (qk, vw, taps) = (g.k_heads * g.head_dim, g.v_heads * g.head_dim, g.conv_taps);
        let (nk, nv, ghd) = (g.k_heads, g.v_heads, g.head_dim);
        self.decl_q8(&format!("Wq{l}"), d, qk);
        self.decl_q8(&format!("Wk{l}"), d, qk);
        self.decl_q8(&format!("Wv{l}"), d, vw);
        self.decl_q8(&format!("Wz{l}"), d, vw);
        self.decl_f32(&format!("Wcq{l}"), taps, qk);
        self.decl_f32(&format!("Wck{l}"), taps, qk);
        self.decl_f32(&format!("Wcv{l}"), taps, vw);
        self.decl_f32(&format!("Csq{l}"), taps, qk);
        self.decl_f32(&format!("Csk{l}"), taps, qk);
        self.decl_f32(&format!("Csv{l}"), taps, vw);
        self.decl_q8(&format!("Wb{l}"), d, nv);
        self.decl_q8(&format!("Wa{l}"), d, nv);
        self.decl_f32(&format!("Dt{l}"), 1, nv);
        self.decl_f32(&format!("Al{l}"), 1, nv);
        self.decl_f32(&format!("St{l}"), vw, ghd);
        self.decl_f32(&format!("Gn{l}"), 1, ghd);
        self.decl_q8(&format!("Wo{l}"), vw, d);
        self.s.push_str(&format!(
            "Let xn{l} be rmsnorm of {cur} with weight an{l}.\n\
             Let qr{l} be xn{l} matmul Wq{l}.\n\
             Let kr{l} be xn{l} matmul Wk{l}.\n\
             Let vr{l} be xn{l} matmul Wv{l}.\n\
             Let zz{l} be xn{l} matmul Wz{l}.\n\
             Let qc{l} be causal conv of qr{l} with weight Wcq{l} state Csq{l} over {taps} taps.\n\
             Let kc{l} be causal conv of kr{l} with weight Wck{l} state Csk{l} over {taps} taps.\n\
             Let vc{l} be causal conv of vr{l} with weight Wcv{l} state Csv{l} over {taps} taps.\n\
             Let qs{l} be silu of qc{l}.\n\
             Let ks{l} be silu of kc{l}.\n\
             Let vs{l} be silu of vc{l}.\n\
             Let qn{l} be l2norm of qs{l} across {nk} heads.\n\
             Let kn{l} be l2norm of ks{l} across {nk} heads.\n\
             Let br{l} be xn{l} matmul Wb{l}.\n\
             Let bt{l} be sigmoid of br{l}.\n\
             Let ar{l} be xn{l} matmul Wa{l}.\n\
             Let dy{l} be decay of ar{l} with bias Dt{l} rate Al{l}.\n\
             Let od{l} be gated delta of qn{l} over kn{l} and vs{l} with {gate} zz{l} beta bt{l} decay dy{l} state St{l} norm Gn{l} with {nv} heads {nk} groups.\n\
             Let ao{l} be od{l} matmul Wo{l}.\n",
        ));
    }

    /// The block's tail: the residual, the pre-FFN norm, the FFN and the second residual — for one row, or for a
    /// stack of `rows` (its tensors prefixed `b`).
    fn ffn(&mut self, l: usize, cur: &str, rows: Option<usize>) {
        let d = self.spec.d;
        let p = if rows.is_some() { "b" } else { "" };
        self.decl_f32(&format!("fn{l}"), 1, d);
        match self.spec.layers[l].ffn.clone() {
            Ffn::Dense { dff, .. } => {
                self.decl_q8(&format!("Wg{l}"), d, dff);
                self.decl_q8(&format!("Wu{l}"), d, dff);
                self.decl_q8(&format!("Wd{l}"), dff, d);
                let ao = self.post_norm(l, &format!("{p}ao{l}"), true);
                self.s.push_str(&format!(
                    "Let {p}xr{l} be {cur} plus {ao}.\n\
                     Let {p}x2n{l} be rmsnorm of {p}xr{l} with weight fn{l}.\n\
                     Let {p}gt{l} be {p}x2n{l} matmul Wg{l}.\n\
                     Let {p}up{l} be {p}x2n{l} matmul Wu{l}.\n"
                ));
                self.glu(l, &format!("{p}gt{l}"), &format!("{p}up{l}"), &format!("{p}hh{l}"));
                self.s.push_str(&format!("Let {p}dn{l} be {p}hh{l} matmul Wd{l}.\n"));
                let dn = self.post_norm(l, &format!("{p}dn{l}"), false);
                self.s.push_str(&format!("Let {p}xo{l} be {p}xr{l} plus {dn}.\n"));
            }
            Ffn::Moe(m) => {
                let (e, k, dff) = (m.experts, m.top_k, m.expert_dff);
                self.decl_q8(&format!("Wr{l}"), d, e);
                self.decl_q8(&format!("Wg{l}"), e * dff, d);
                self.decl_q8(&format!("Wu{l}"), e * dff, d);
                self.decl_q8(&format!("Wd{l}"), e * d, dff);
                let ao = self.post_norm(l, &format!("{p}ao{l}"), true);
                self.s.push_str(&format!(
                    "Let {p}xr{l} be {cur} plus {ao}.\n\
                     Let {p}x2n{l} be rmsnorm of {p}xr{l} with weight fn{l}.\n"
                ));
                if m.biases {
                    self.decl_f32(&format!("Br{l}"), 1, e);
                    self.decl_f32(&format!("Bg{l}"), e, dff);
                    self.decl_f32(&format!("Bu{l}"), e, dff);
                    self.decl_f32(&format!("Bd{l}"), e, d);
                }
                if m.gating == (Gating::Sigmoid { bias: true }) {
                    self.decl_f32(&format!("Bs{l}"), 1, e);
                }
                let plus = |b: &str| if m.biases { format!(" plus {b}{l}") } else { String::new() };
                let (br, bg, bu, bd) = (plus("Br"), plus("Bg"), plus("Bu"), plus("Bd"));
                let act = match m.act {
                    Act::SwigluOai { alpha, limit } => format!(" using a swiglu clamped at {limit:?} with slope {alpha:?}"),
                    _ => String::new(),
                };
                // A softmax over the kept experts is the softmax over all, renormalized over them. Sigmoid scores are each
                // expert's own, DeepSeek-V3's chosen by each plus its selection bias, and Llama 4's weigh the inputs; a
                // routed scale of zero is llama.cpp's absent one.
                let score = match m.gating {
                    Gating::Sigmoid { bias: true } => format!(" by sigmoid biased by Bs{l}"),
                    Gating::Sigmoid { .. } | Gating::SigmoidScaledInput => " by sigmoid".to_string(),
                    _ => String::new(),
                };
                let groups = m.groups.map_or(String::new(), |(count, used)| format!(" within the top {used} of {count} groups"));
                let norm = match !matches!(m.gating, Gating::SoftmaxOfTopK) && !m.norm_top_k {
                    true => " without renormalizing",
                    false => "",
                };
                let scale = match m.weights_scale {
                    s if s == 0.0 || s == 1.0 => String::new(),
                    s => format!(" scaled by {s:?}"),
                };
                let inputs = match m.gating {
                    Gating::SigmoidScaledInput => " weighting the inputs",
                    _ => "",
                };
                let route = |x: &str, out: &str| {
                    format!(
                        "Let {out} be moe of {x} with router Wr{l}{br} gate Wg{l}{bg} up Wu{l}{bu} down Wd{l}{bd} choosing {k} of {e}{score}{groups}{norm}{scale}{inputs}{act}.\n"
                    )
                };
                match rows {
                    None => self.s.push_str(&route(&format!("x2n{l}"), &format!("mo{l}"))),
                    // The router chooses experts per token: each row of the stack is routed on its own.
                    Some(bsz) => {
                        self.s.push_str(&format!("Let bmo{l} be a {bsz} by {d} tensor of Real.\n"));
                        for b in 0..bsz {
                            let r = b + 1;
                            self.s.push_str(&format!("Let x2n{l}_s{b} be row {r} of bx2n{l}.\n"));
                            self.s.push_str(&route(&format!("x2n{l}_s{b}"), &format!("mo{l}_s{b}")));
                            self.s.push_str(&format!("Set row {r} of bmo{l} to mo{l}_s{b}.\n"));
                        }
                    }
                }
                // The shared experts, one SwiGLU FFN every token takes, add to the routed experts' sum.
                let routed = match m.shared_experts {
                    0 => format!("{p}mo{l}"),
                    _ => {
                        let sdff = m.shared_dff;
                        self.decl_q8(&format!("Wsg{l}"), d, sdff);
                        self.decl_q8(&format!("Wsu{l}"), d, sdff);
                        self.decl_q8(&format!("Wsd{l}"), sdff, d);
                        self.s.push_str(&format!(
                            "Let {p}sg{l} be {p}x2n{l} matmul Wsg{l}.\n\
                             Let {p}su{l} be {p}x2n{l} matmul Wsu{l}.\n\
                             Let {p}sh{l} be swiglu of {p}sg{l} and {p}su{l}.\n\
                             Let {p}sd{l} be {p}sh{l} matmul Wsd{l}.\n"
                        ));
                        // Qwen's shared expert is scaled by `σ(x·g)`: the gate a column over the input, its sigmoid spread
                        // across the row by the host's row of ones (`s·1` is `s`), then multiplied in.
                        let shared = if m.shared_gate {
                            self.decl_f32(&format!("Wsgi{l}"), d, 1);
                            let ones = self.host_table(HostTable::Ones(d), 1, d);
                            self.s.push_str(&format!(
                                "Let {p}sgi{l} be {p}x2n{l} matmul Wsgi{l}.\n\
                                 Let {p}sgw{l} be sigmoid of {p}sgi{l}.\n\
                                 Let {p}sgr{l} be {p}sgw{l} matmul {ones}.\n\
                                 Let {p}sdg{l} be {p}sd{l} hadamard {p}sgr{l}.\n"
                            ));
                            format!("{p}sdg{l}")
                        } else {
                            format!("{p}sd{l}")
                        };
                        self.s.push_str(&format!("Let {p}me{l} be {p}mo{l} plus {shared}.\n"));
                        format!("{p}me{l}")
                    }
                };
                let mo = self.post_norm(l, &routed, false);
                self.s.push_str(&format!("Let {p}xo{l} be {p}xr{l} plus {mo}.\n"));
            }
        }
    }

    /// The final norm and the LM head.
    fn head(&mut self, cur: &str) {
        let (d, vocab) = (self.spec.d, self.spec.vocab);
        self.decl_f32("on", 1, d);
        self.decl_q8("Wout", d, vocab);
        self.s.push_str(&format!("Let xf be rmsnorm of {cur} with weight on.\n"));
        self.s.push_str(&capped_logits("xf matmul Wout", "logits", self.spec.final_logit_softcap));
    }

    /// The decode step's first hidden state: row `tok` of the embedding table.
    fn embed(&mut self) {
        let (vocab, d) = (self.spec.vocab, self.spec.d);
        self.decl_q8("Emb", vocab, d);
        self.embedded("row tok of Emb", "x");
    }

    /// The token a greedy decode emits: the arg-max of the logits.
    fn next_token(&mut self) {
        self.s.push_str("Let next be argmax of logits.\n");
    }

    /// The rotary caches a decode step reads its angles from: `ctx` positions of `dims / 2` angles each — none for a
    /// model no layer of which rotates.
    fn rope_cache(&mut self, ctx: usize) {
        self.rotary_tables(None, true, ctx);
    }

    /// The stack's first hidden states: row `toks[k]` of the embedding table as row `k + 1` of `X`.
    fn batched_embed(&mut self, toks: &[usize]) {
        let (vocab, d) = (self.spec.vocab, self.spec.d);
        self.decl_q8("Emb", vocab, d);
        self.s.push_str(&format!("Let toks be a 1 by {} tensor of Int.\n", toks.len()));
        for (k, tok) in toks.iter().enumerate() {
            self.s.push_str(&format!("Set item {} of toks to {tok}.\n", k + 1));
        }
        self.embedded("rows toks of Emb", "X");
    }

    /// The batch's head: the final norm and the LM head on the stack, then each sequence's logits and arg-max.
    fn batched_head(&mut self, cur: &str, bsz: usize) {
        let (d, vocab) = (self.spec.d, self.spec.vocab);
        self.decl_f32("on", 1, d);
        self.decl_q8("Wout", d, vocab);
        self.s.push_str(&format!("Let bxf be rmsnorm of {cur} with weight on.\n"));
        self.s.push_str(&capped_logits("bxf matmul Wout", "blogits", self.spec.final_logit_softcap));
        for b in 0..bsz {
            self.s.push_str(&format!(
                "Let logits_s{b} be row {} of blogits.\nLet next_s{b} be argmax of logits_s{b}.\n",
                b + 1
            ));
        }
    }

    /// The next token after a stack's last row: its row `rows` alone through the head and the arg-max.
    fn last_row_head(&mut self, cur: &str, rows: usize) {
        self.s.push_str(&format!("Let xl be row {rows} of {cur}.\n"));
        self.head("xl");
        self.next_token();
    }

    /// Layer `l` of a stacked step: the stacked hidden states `cur` (`B × d`) in, `bxo{l}` out — the single step's
    /// layer with its weight reads on the stack and its per-sequence state per row.
    fn stacked_layer(&mut self, l: usize, cur: &str, ctx: usize, bsz: usize, rows: RowState) {
        let spec = self.spec;
        self.decl_f32(&format!("an{l}"), 1, spec.d);
        match &spec.layers[l].mixer {
            Mixer::Attention(a) => self.stacked_attention(l, a, cur, ctx, bsz, rows),
            Mixer::GatedDelta(g) => self.stacked_gated_delta(l, g, cur, bsz, rows),
            Mixer::Mla(m) => self.stacked_mla(l, m, cur, ctx, bsz, rows),
        }
        self.ffn(l, cur, Some(bsz));
    }

    fn stacked_attention(&mut self, l: usize, a: &Attention, cur: &str, ctx: usize, bsz: usize, rows: RowState) {
        let st = |base: String, b: usize| rows.state(&base, b);
        let Dims { d, aq, akv, avd, ao, h, kv, .. } = attn_dims(self.spec, a);
        self.decl_q8(&format!("Wq{l}"), d, aq);
        if a.output_gate {
            self.decl_q8(&format!("Wqg{l}"), d, aq);
        }
        self.decl_q8(&format!("Wk{l}"), d, akv);
        self.decl_q8(&format!("Wv{l}"), d, avd);
        let ((nq, nk), (wq, wk)) = qk_norm_segments(a);
        if a.qk_norm {
            self.decl_f32(&format!("Qn{l}"), 1, wq);
            self.decl_f32(&format!("Kn{l}"), 1, wk);
        }
        self.decl_q8(&format!("Wo{l}"), ao, d);
        for b in rows.holders(bsz) {
            self.s.push_str(&format!(
                "Let {} be a {ctx} by {akv} tensor of Real.\nLet {} be a {ctx} by {avd} tensor of Real.\n",
                st(format!("Kc{l}"), b),
                st(format!("Vc{l}"), b)
            ));
        }
        let bias = |b: &str| a.bias.then(|| format!("{b}{l}"));
        self.s.push_str(&format!("Let bxn{l} be rmsnorm of {cur} with weight an{l}.\n"));
        self.project(&format!("bqr{l}"), &format!("bxn{l}"), &format!("Wq{l}"), bias("Bq"), aq);
        if a.output_gate {
            self.s.push_str(&format!("Let bgr{l} be bxn{l} matmul Wqg{l}.\n"));
        }
        self.project(&format!("bkr{l}"), &format!("bxn{l}"), &format!("Wk{l}"), bias("Bk"), akv);
        self.project(&format!("bvr{l}"), &format!("bxn{l}"), &format!("Wv{l}"), bias("Bv"), avd);
        match rows {
            // ★ ONE SEQUENCE'S ROWS ARE ONE BLOCK, END TO END. The QK-norm runs over every row's heads at once, the rope
            // turns row `i` by row `i` of the chunk's tables, the block's keys and values go into the cache in one write
            // each from the chunk's first position, and the block attends once — row `i` up to `pos_s0 + i`, its own
            // position, so no row sees a later one. No statement of the layer is per row.
            RowState::Shared => {
                let (kc, vc) = (st(format!("Kc{l}"), 0), st(format!("Vc{l}"), 0));
                let (mut q, mut k) = (format!("bqr{l}"), format!("bkr{l}"));
                if a.qk_norm {
                    let (qh, kh) = (bsz * nq, bsz * nk);
                    self.s.push_str(&format!(
                        "Let bqm{l} be rmsnorm of bqr{l} with weight Qn{l} across {qh} heads.\n\
                         Let bkm{l} be rmsnorm of bkr{l} with weight Kn{l} across {kh} heads.\n"
                    ));
                    (q, k) = (format!("bqm{l}"), format!("bkm{l}"));
                }
                if a.rope.is_some() {
                    let angles = self.angles(l, AttnForm::Cached { ctx }, "pos_s0");
                    self.s.push_str(&format!(
                        "Let bqp{l} be rope of {q} with {angles} across {h} heads.\n\
                         Let bkp{l} be rope of {k} with {angles} across {kv} heads.\n"
                    ));
                    (q, k) = (format!("bqp{l}"), format!("bkp{l}"));
                }
                (q, k) = self.llama4_qk(a, (q, k), (bsz * h, bsz * kv), h, Some(("pos_s0".to_string(), ctx)), &|base| format!("b{base}{l}"));
                let (w, sk) = (score_clauses(a), self.sinks(l, a));
                self.s.push_str(&format!(
                    "Set rows from pos_s0 of {kc} to {k}.\n\
                     Set rows from pos_s0 of {vc} to bvr{l}.\n\
                     Let bct{l} be causal attention of {q} over {kc} and {vc} with {h} heads {kv} groups from pos_s0{w}{sk}.\n"
                ));
            }
            // Rows of different sequences append to and attend their own caches, one row each.
            RowState::PerSequence => {
                self.s.push_str(&format!("Let bct{l} be a {bsz} by {ao} tensor of Real.\n"));
                for b in 0..bsz {
                    let r = b + 1;
                    let (kc, vc) = (st(format!("Kc{l}"), b), st(format!("Vc{l}"), b));
                    self.s.push_str(&format!(
                        "Let qr{l}_s{b} be row {r} of bqr{l}.\n\
                         Let kr{l}_s{b} be row {r} of bkr{l}.\n\
                         Let vr{l}_s{b} be row {r} of bvr{l}.\n"
                    ));
                    let (mut q, mut k) = (format!("qr{l}_s{b}"), format!("kr{l}_s{b}"));
                    if a.qk_norm {
                        self.s.push_str(&format!(
                            "Let qm{l}_s{b} be rmsnorm of qr{l}_s{b} with weight Qn{l} across {nq} heads.\n\
                             Let km{l}_s{b} be rmsnorm of kr{l}_s{b} with weight Kn{l} across {nk} heads.\n"
                        ));
                        (q, k) = (format!("qm{l}_s{b}"), format!("km{l}_s{b}"));
                    }
                    if a.rope.is_some() {
                        let angles = self.angles(l, AttnForm::Cached { ctx }, &format!("pos_s{b}"));
                        self.s.push_str(&format!(
                            "Let qp{l}_s{b} be rope of {q} with {angles} across {h} heads.\n\
                             Let kp{l}_s{b} be rope of {k} with {angles} across {kv} heads.\n"
                        ));
                        (q, k) = (format!("qp{l}_s{b}"), format!("kp{l}_s{b}"));
                    }
                    (q, k) = self.llama4_qk(a, (q, k), (h, kv), h, Some((format!("pos_s{b}"), ctx)), &|base| format!("{base}{l}_s{b}"));
                    let (w, sk) = (score_clauses(a), self.sinks(l, a));
                    self.s.push_str(&format!(
                        "Set row pos_s{b} of {kc} to {k}.\n\
                         Set row pos_s{b} of {vc} to vr{l}_s{b}.\n\
                         Let ct{l}_s{b} be attention of {q} over {kc} and {vc} with {h} heads {kv} groups up to pos_s{b}{w}{sk}.\n\
                         Set row {r} of bct{l} to ct{l}_s{b}.\n"
                    ));
                }
            }
        }
        let gated = if a.output_gate {
            self.s.push_str(&format!("Let bgs{l} be sigmoid of bgr{l}.\nLet bcg{l} be bct{l} hadamard bgs{l}.\n"));
            format!("bcg{l}")
        } else {
            format!("bct{l}")
        };
        self.project(&format!("bao{l}"), &gated, &format!("Wo{l}"), a.output_bias.then(|| format!("Bo{l}")), d);
    }

    fn stacked_gated_delta(&mut self, l: usize, g: &GatedDelta, cur: &str, bsz: usize, rows: RowState) {
        let gate = gate_clause(g);
        let st = |base: String, b: usize| rows.state(&base, b);
        let d = self.spec.d;
        let (qk, vw, taps) = (g.k_heads * g.head_dim, g.v_heads * g.head_dim, g.conv_taps);
        let (nk, nv, ghd) = (g.k_heads, g.v_heads, g.head_dim);
        self.decl_q8(&format!("Wq{l}"), d, qk);
        self.decl_q8(&format!("Wk{l}"), d, qk);
        self.decl_q8(&format!("Wv{l}"), d, vw);
        self.decl_q8(&format!("Wz{l}"), d, vw);
        self.decl_f32(&format!("Wcq{l}"), taps, qk);
        self.decl_f32(&format!("Wck{l}"), taps, qk);
        self.decl_f32(&format!("Wcv{l}"), taps, vw);
        for b in rows.holders(bsz) {
            self.decl_f32(&st(format!("Csq{l}"), b), taps, qk);
            self.decl_f32(&st(format!("Csk{l}"), b), taps, qk);
            self.decl_f32(&st(format!("Csv{l}"), b), taps, vw);
            self.decl_f32(&st(format!("St{l}"), b), vw, ghd);
        }
        self.decl_q8(&format!("Wb{l}"), d, nv);
        self.decl_q8(&format!("Wa{l}"), d, nv);
        self.decl_f32(&format!("Dt{l}"), 1, nv);
        self.decl_f32(&format!("Al{l}"), 1, nv);
        self.decl_f32(&format!("Gn{l}"), 1, ghd);
        self.decl_q8(&format!("Wo{l}"), vw, d);
        self.s.push_str(&format!(
            "Let bxn{l} be rmsnorm of {cur} with weight an{l}.\n\
             Let bqr{l} be bxn{l} matmul Wq{l}.\n\
             Let bkr{l} be bxn{l} matmul Wk{l}.\n\
             Let bvr{l} be bxn{l} matmul Wv{l}.\n\
             Let bzz{l} be bxn{l} matmul Wz{l}.\n\
             Let bbr{l} be bxn{l} matmul Wb{l}.\n\
             Let bar{l} be bxn{l} matmul Wa{l}.\n"
        ));
        match rows {
            // ★ ONE SEQUENCE'S ROWS TAKE THE RECURRENCE AS A BLOCK. The rings take the block's rows in order, the gates and
            // norms run over every row's heads at once, and the gated delta steps the one memory row after row — each
            // sugar the rows fed in turn, so no statement of the layer is per row.
            RowState::Shared => {
                let bnk = bsz * nk;
                let (csq, csk, csv, stt) = (
                    st(format!("Csq{l}"), 0),
                    st(format!("Csk{l}"), 0),
                    st(format!("Csv{l}"), 0),
                    st(format!("St{l}"), 0),
                );
                self.s.push_str(&format!(
                    "Let bqc{l} be causal conv of bqr{l} with weight Wcq{l} state {csq} over {taps} taps.\n\
                     Let bkc{l} be causal conv of bkr{l} with weight Wck{l} state {csk} over {taps} taps.\n\
                     Let bvc{l} be causal conv of bvr{l} with weight Wcv{l} state {csv} over {taps} taps.\n\
                     Let bqs{l} be silu of bqc{l}.\n\
                     Let bks{l} be silu of bkc{l}.\n\
                     Let bvs{l} be silu of bvc{l}.\n\
                     Let bqn{l} be l2norm of bqs{l} across {bnk} heads.\n\
                     Let bkn{l} be l2norm of bks{l} across {bnk} heads.\n\
                     Let bbt{l} be sigmoid of bbr{l}.\n\
                     Let bdy{l} be decay of bar{l} with bias Dt{l} rate Al{l}.\n\
                     Let bod{l} be gated delta of bqn{l} over bkn{l} and bvs{l} with {gate} bzz{l} beta bbt{l} decay bdy{l} state {stt} norm Gn{l} with {nv} heads {nk} groups.\n\
                     Let bao{l} be bod{l} matmul Wo{l}.\n"
                ));
            }
            RowState::PerSequence => {
                self.s.push_str(&format!("Let bod{l} be a {bsz} by {vw} tensor of Real.\n"));
                for b in 0..bsz {
                    let r = b + 1;
                    let (csq, csk, csv, stt) = (
                        st(format!("Csq{l}"), b),
                        st(format!("Csk{l}"), b),
                        st(format!("Csv{l}"), b),
                        st(format!("St{l}"), b),
                    );
                    self.s.push_str(&format!(
                        "Let qr{l}_s{b} be row {r} of bqr{l}.\n\
                         Let kr{l}_s{b} be row {r} of bkr{l}.\n\
                         Let vr{l}_s{b} be row {r} of bvr{l}.\n\
                         Let zz{l}_s{b} be row {r} of bzz{l}.\n\
                         Let br{l}_s{b} be row {r} of bbr{l}.\n\
                         Let ar{l}_s{b} be row {r} of bar{l}.\n\
                         Let qc{l}_s{b} be causal conv of qr{l}_s{b} with weight Wcq{l} state {csq} over {taps} taps.\n\
                         Let kc{l}_s{b} be causal conv of kr{l}_s{b} with weight Wck{l} state {csk} over {taps} taps.\n\
                         Let vc{l}_s{b} be causal conv of vr{l}_s{b} with weight Wcv{l} state {csv} over {taps} taps.\n\
                         Let qs{l}_s{b} be silu of qc{l}_s{b}.\n\
                         Let ks{l}_s{b} be silu of kc{l}_s{b}.\n\
                         Let vs{l}_s{b} be silu of vc{l}_s{b}.\n\
                         Let qn{l}_s{b} be l2norm of qs{l}_s{b} across {nk} heads.\n\
                         Let kn{l}_s{b} be l2norm of ks{l}_s{b} across {nk} heads.\n\
                         Let bt{l}_s{b} be sigmoid of br{l}_s{b}.\n\
                         Let dy{l}_s{b} be decay of ar{l}_s{b} with bias Dt{l} rate Al{l}.\n\
                         Let od{l}_s{b} be gated delta of qn{l}_s{b} over kn{l}_s{b} and vs{l}_s{b} with {gate} zz{l}_s{b} beta bt{l}_s{b} decay dy{l}_s{b} state {stt} norm Gn{l} with {nv} heads {nk} groups.\n\
                         Set row {r} of bod{l} to od{l}_s{b}.\n"
                    ));
                }
                self.s.push_str(&format!("Let bao{l} be bod{l} matmul Wo{l}.\n"));
            }
        }
    }
}

/// An attention's clauses before its sinks: ` within a window of W` (or ` within chunks of C`) for an attention over
/// part of its history, ` scaled by S` for a score scale other than the sugar's `1/√hd`, and ` capped at C` for scores
/// soft-capped before their softmax.
fn score_clauses(a: &Attention) -> String {
    let mut clauses = a.window.map_or_else(String::new, |w| w.clause());
    if (a.scale - 1.0 / (a.head_dim as f64).sqrt()).abs() > 1e-15 {
        clauses.push_str(&format!(" scaled by {:?}", a.scale));
    }
    if let Some(cap) = a.softcap {
        clauses.push_str(&format!(" capped at {cap:?}"));
    }
    clauses
}

/// `Let logits be <product>.`, or for logits the model soft-caps the product first and `Let logits be <raw> capped at
/// C.` after it, the raw product bound as `<logits>_raw`.
fn capped_logits(product: &str, logits: &str, cap: Option<f64>) -> String {
    match cap {
        Some(cap) => format!("Let {logits}_raw be {product}.\nLet {logits} be {logits}_raw capped at {cap:?}.\n"),
        None => format!("Let {logits} be {product}.\n"),
    }
}

/// An attention's QK norm as the sugar takes it: the query's and the key's segments a row and each segment's width — a
/// head each, or (MiniMax-M2) the whole projection as one.
fn qk_norm_segments(a: &Attention) -> ((usize, usize), (usize, usize)) {
    if a.qk_norm_whole {
        ((1, 1), (a.heads * a.head_dim, a.kv_heads * a.head_dim))
    } else {
        ((a.heads, a.kv_heads), (a.head_dim, a.head_dim))
    }
}

/// The hidden-state tensor layer `l` reads: `x` for the first layer, the previous layer's output after.
pub fn layer_input(l: usize) -> String {
    if l == 0 {
        "x".to_string()
    } else {
        format!("xo{}", l - 1)
    }
}

/// The stacked hidden states layer `l` of a stacked step reads: `X` for the first layer, the previous `bxo` after.
pub fn stacked_layer_input(l: usize) -> String {
    if l == 0 {
        "X".to_string()
    } else {
        format!("bxo{}", l - 1)
    }
}

/// The single-token forward with no history: the token's hidden state `x` in, the logits out.
pub fn forward(spec: &ModelSpec, dir: &str) -> Authored {
    let mut p = Program::new(spec, dir);
    p.s.push_str(&format!("Let x be a 1 by {} tensor of Real.\n", spec.d));
    p.rotary_tables(None, false, 1);
    let mut cur = "x".to_string();
    for l in 0..spec.layers.len() {
        p.layer(l, &cur, AttnForm::Current);
        cur = format!("xo{l}");
    }
    p.head(&cur);
    p.finish()
}

/// ONE DECODE STEP: token `tok` at position `pos` (1-based) in, the next token out. Every attention layer appends
/// this token's key and value to its KV cache (`ctx` positions, the step's carried state alongside any recurrent
/// memories and conv rings) and attends over the first `pos` rows; the step ends with the arg-max of the logits.
pub fn decode_step(spec: &ModelSpec, dir: &str, ctx: usize, tok: usize, pos: usize) -> Authored {
    let mut p = Program::new(spec, dir);
    p.s.push_str(&format!("Let tok be {tok}.\nLet pos be {pos}.\n"));
    p.embed();
    p.rope_cache(ctx);
    let mut cur = "x".to_string();
    for l in 0..spec.layers.len() {
        p.layer(l, &cur, AttnForm::Cached { ctx });
        cur = format!("xo{l}");
    }
    p.head(&cur);
    p.next_token();
    p.finish()
}

/// A PREFILL CHUNK: the tokens `toks` of ONE sequence at positions `pos0, pos0 + 1, …` (1-based) as one step, the
/// next token after the last of them out. The hidden states are stacked into a `T × d` matrix, so every projection,
/// norm, FFN and residual reads its weights once for the chunk; the chunk's keys and values append to the cache as
/// one block from `pos0`, and the rows attend as one causal block. The state is the decode step's own, so a decode
/// step continues where the chunk stops; the LM head reads the last row only.
pub fn prefill(spec: &ModelSpec, dir: &str, ctx: usize, toks: &[usize], pos0: usize) -> Authored {
    let t = toks.len();
    assert!(
        t > 0 && pos0 >= 1 && pos0 + t - 1 <= ctx,
        "a chunk of {t} tokens from position {pos0} fits a context of {ctx}"
    );
    let mut p = Program::new(spec, dir);
    p.s.push_str(&format!("Let pos_s0 be {pos0}.\n"));
    p.batched_embed(toks);
    p.rope_cache(ctx);
    let mut cur = stacked_layer_input(0);
    for l in 0..spec.layers.len() {
        p.stacked_layer(l, &cur, ctx, t, RowState::Shared);
        cur = stacked_layer_input(l + 1);
    }
    p.last_row_head(&cur, t);
    p.finish()
}

/// B DECODE STEPS AT ONCE: sequence `b` brings token `tok_b` at position `pos_b` (1-based) and its own carried
/// state, and every sequence emits its own next token. The hidden states are stacked into a `B × d` matrix and
/// everything that reads a weight runs on the stack; what depends on one sequence's position or state runs per
/// sequence, on that sequence's row, exactly as the single step writes it.
pub fn decode_batch(spec: &ModelSpec, dir: &str, ctx: usize, seqs: &[(usize, usize)]) -> Authored {
    let bsz = seqs.len();
    let mut p = Program::new(spec, dir);
    for (b, (_, pos)) in seqs.iter().enumerate() {
        p.s.push_str(&format!("Let pos_s{b} be {pos}.\n"));
    }
    p.batched_embed(&seqs.iter().map(|(tok, _)| *tok).collect::<Vec<_>>());
    p.rope_cache(ctx);
    let mut cur = stacked_layer_input(0);
    for l in 0..spec.layers.len() {
        p.stacked_layer(l, &cur, ctx, bsz, RowState::PerSequence);
        cur = stacked_layer_input(l + 1);
    }
    p.batched_head(&cur, bsz);
    p.finish()
}

/// Layer `l` alone, as the forward writes it: its incoming hidden state declared as an input, the RoPE tables when
/// it is an attention layer, then the layer.
pub fn layer(spec: &ModelSpec, l: usize, dir: &str) -> Authored {
    let mut p = Program::new(spec, dir);
    let input = layer_input(l);
    p.s.push_str(&format!("Let {input} be a 1 by {} tensor of Real.\n", spec.d));
    p.rotary_tables(Some(l), false, 1);
    p.layer(l, &input, AttnForm::Current);
    p.finish()
}

/// Layer `l` of a decode step alone, its attention reading a KV cache of `ctx` positions up to `pos`.
pub fn decode_layer(spec: &ModelSpec, l: usize, dir: &str, ctx: usize, pos: usize) -> Authored {
    let mut p = Program::new(spec, dir);
    let input = layer_input(l);
    p.s.push_str(&format!("Let pos be {pos}.\n"));
    p.s.push_str(&format!("Let {input} be a 1 by {} tensor of Real.\n", spec.d));
    p.rotary_tables(Some(l), true, ctx);
    p.layer(l, &input, AttnForm::Cached { ctx });
    p.finish()
}

/// The final norm and LM head alone, reading the last layer's output.
pub fn head(spec: &ModelSpec, dir: &str) -> Authored {
    let mut p = Program::new(spec, dir);
    let input = layer_input(spec.layers.len());
    p.s.push_str(&format!("Let {input} be a 1 by {} tensor of Real.\n", spec.d));
    p.head(&input);
    p.finish()
}

/// A decode step's embedding lookup alone: token `tok` in, the first hidden state `x` out.
pub fn decode_embed(spec: &ModelSpec, dir: &str, tok: usize) -> Authored {
    let mut p = Program::new(spec, dir);
    p.s.push_str(&format!("Let tok be {tok}.\n"));
    p.embed();
    p.finish()
}

/// A decode step's head alone: the final norm, the LM head and the arg-max.
pub fn decode_head(spec: &ModelSpec, dir: &str) -> Authored {
    let mut p = Program::new(spec, dir);
    let input = layer_input(spec.layers.len());
    p.s.push_str(&format!("Let {input} be a 1 by {} tensor of Real.\n", spec.d));
    p.head(&input);
    p.next_token();
    p.finish()
}

/// A batched decode step's embedding lookups alone: the stacked `X` out.
pub fn decode_batch_embed(spec: &ModelSpec, dir: &str, toks: &[usize]) -> Authored {
    let mut p = Program::new(spec, dir);
    p.batched_embed(toks);
    p.finish()
}

/// Layer `l` of a stacked step alone, its rows' state held as `rows` says: the positions the rows read, the stacked
/// input, the RoPE tables when it is an attention layer, then the layer.
pub fn stacked_layer(spec: &ModelSpec, l: usize, dir: &str, ctx: usize, poss: &[usize], rows: RowState) -> Authored {
    let bsz = poss.len();
    let mut p = Program::new(spec, dir);
    // A chunk's rows sit at consecutive positions and read only the first; a batch's sequences each read their own.
    let read = if rows == RowState::Shared { &poss[..1] } else { poss };
    for (b, pos) in read.iter().enumerate() {
        p.s.push_str(&format!("Let pos_s{b} be {pos}.\n"));
    }
    let input = stacked_layer_input(l);
    p.s.push_str(&format!("Let {input} be a {bsz} by {} tensor of Real.\n", spec.d));
    p.rotary_tables(Some(l), true, ctx);
    p.stacked_layer(l, &input, ctx, bsz, rows);
    p.finish()
}

/// A prefill chunk's head alone: the last layer's stacked output in, the next token after the chunk's last out.
pub fn prefill_head(spec: &ModelSpec, dir: &str, chunk: usize) -> Authored {
    let mut p = Program::new(spec, dir);
    let input = stacked_layer_input(spec.layers.len());
    p.s.push_str(&format!("Let {input} be a {chunk} by {} tensor of Real.\n", spec.d));
    p.last_row_head(&input, chunk);
    p.finish()
}

/// A batched decode step's head alone: the last layer's stacked output in, every sequence's next token out.
pub fn decode_batch_head(spec: &ModelSpec, dir: &str, bsz: usize) -> Authored {
    let mut p = Program::new(spec, dir);
    let input = stacked_layer_input(spec.layers.len());
    p.s.push_str(&format!("Let {input} be a {bsz} by {} tensor of Real.\n", spec.d));
    p.batched_head(&input, bsz);
    p.finish()
}

// ── Tensor parallelism ─────────────────────────────────────────────────────────────────────────────────────────────
//
// A program spread over `tp` chips, Megatron's split: the norms and the residual spine replicated; each chip's
// column-parallel projections onto its own heads (its own FFN columns), every per-head operation, its slices of the KV
// caches and recurrent state, then its row-parallel output projection — a PARTIAL of the full row; the partials
// summed (the all-reduce, written as the left fold of `plus` it means). The manifest records which chip holds each
// chip-local tensor, which full tensor each slice is cut from, and each collective.
//
// Any width splits a model: a chip holds ⌊n/tp⌋ or ⌈n/tp⌉ of each head count and of the FFN's columns ([`share`]),
// and a chip with no share of a layer — fewer heads than chips — folds the empty sum, a zero partial, so every
// all-reduce still draws a partial from every chip.

use crate::spatial::tensor_parallel::{SliceAxis, TpCollective, TpManifest, TpSlice};

/// A tensor-parallel program, its weight manifest, and its placement.
pub type TpAuthored = (
    String,
    HashMap<String, (String, usize)>,
    HashMap<String, String>,
    TpManifest,
);

/// Can `spec` be split over `tp` chips? At any width of at least one chip ([`share`]), when every attention is
/// unbiased and sinkless and every FFN is dense (its columns split; an expert mixture would split by experts).
pub fn check_tp(spec: &ModelSpec, tp: usize) -> Result<(), NotYet> {
    check(spec)?;
    if tp == 0 {
        return Err(NotYet("a tensor-parallel width of zero chips"));
    }
    for layer in &spec.layers {
        match &layer.mixer {
            Mixer::Attention(a) if a.bias || a.output_bias || a.sinks => {
                return Err(NotYet("a tensor-parallel attention with biases or sinks"))
            }
            // A norm across every head would sum each chip's heads before it scales any of them.
            Mixer::Attention(a) if a.qk_norm_whole => return Err(NotYet("a tensor-parallel QK norm across every head")),
            Mixer::Mla(_) => return Err(NotYet("a tensor-parallel latent attention")),
            _ => {}
        }
        if let Ffn::Moe(_) = &layer.ffn {
            return Err(NotYet("a tensor-parallel expert mixture"));
        }
    }
    Ok(())
}

/// The `c`-th of `tp` balanced runs of `n` units, `(first, count)`: `[⌊c·n/tp⌋, ⌊(c+1)·n/tp⌋)` — every unit in exactly
/// one run, each ⌊n/tp⌋ or ⌈n/tp⌉ long.
fn run_of(n: usize, tp: usize, c: usize) -> (usize, usize) {
    let (lo, hi) = (c * n / tp, (c + 1) * n / tp);
    (lo, hi - lo)
}

/// Chip `c`'s share of `units` units of `parts` parts each over `tp` chips: `(first unit, units held)` and, within
/// each unit held, `(first part, parts held)`.
///
/// ★ A UNIT'S PARTS STAY WITH IT. With no more chips than units, chip `c` holds a balanced run of the units, every part
/// of each. With more, unit `u` is held by a balanced run of the chips — chip `c` by unit `⌊c·units/tp⌋` — each holding
/// it whole and a balanced run of its parts; a chip whose unit has fewer parts than chips holding it may hold none, and
/// then holds no unit. Over the chips every part of every unit is held exactly once. When `tp` divides `units` (or
/// `units` divides `tp` and `tp/units` divides `parts`), chip `c`'s parts are `[c·n/tp, (c+1)·n/tp)` of the `n` in
/// unit order — Megatron's split.
fn share(units: usize, parts: usize, tp: usize, c: usize) -> ((usize, usize), (usize, usize)) {
    if tp <= units {
        return (run_of(units, tp, c), (0, parts));
    }
    let u = c * units / tp;
    let first = (u * tp).div_ceil(units);
    let holders = ((u + 1) * tp).div_ceil(units) - first;
    let (p0, pn) = run_of(parts, holders, c - first);
    ((u, usize::from(pn > 0)), (p0, pn))
}

/// Chip `c`'s attention heads of `tp`: its query heads and the KV heads they read, each `(first, count)`.
///
/// ★ WHERE THE KV HEADS GO. Query head `q` reads KV head `⌊q / g⌋`, `g = h/kv`: the KV heads are the units of a
/// [`share`] and their `g` query heads its parts. A chip holds whole groups, or — past one chip a KV head — one KV
/// head REPLICATED on the chips sharing its group (vLLM's rule when `tp/kv` divides `g`), each computing and caching
/// the same key and value. Either way a chip's query heads read its KV heads `count_q / count_kv` apiece: one grouped
/// attention.
fn attn_heads_of_chip(a: &Attention, tp: usize, c: usize) -> ((usize, usize), (usize, usize)) {
    let g = a.heads / a.kv_heads;
    let ((k0, kn), (p0, pn)) = share(a.kv_heads, g, tp, c);
    ((k0 * g + p0, kn * pn), (k0, kn))
}

/// Chip `c`'s Gated-DeltaNet heads of `tp`: its key-head column run (per head of `head_dim`) and its value heads as
/// `(first head, count)` blocks.
///
/// ★ THE KEYS ARE TILED. Value head `h` reads key head `h mod HK`, so the value heads reading key heads `[k0, k0+kn)`
/// are one block per recurrence `r < HV/HK`, `r·HK + [k0, k0+kn)`: the key heads are the units of a [`share`] and the
/// recurrences its parts. In recurrence order the blocks keep the tiling on the chip; a key head shared by several
/// chips is computed alike on each, and its recurrences split among them.
/// A gated delta's gate clause in the sugar: `gate` for SiLU, `sigmoid gate` for the sigmoid (Qwen3.8-Flash-Next).
fn gate_clause(g: &GatedDelta) -> &'static str {
    match g.gate {
        GateActivation::Silu => "gate",
        GateActivation::Sigmoid => "sigmoid gate",
    }
}

fn gdn_heads_of_chip(g: &GatedDelta, tp: usize, c: usize) -> (Vec<(usize, usize)>, Vec<(usize, usize)>) {
    let (nk, ghd) = (g.k_heads, g.head_dim);
    let ((k0, kn), (r0, rn)) = share(nk, g.v_heads / nk, tp, c);
    (vec![(k0 * ghd, kn * ghd)], (r0..r0 + rn).map(|r| (r * nk + k0, kn)).collect())
}

/// The heads in `(first, count)` blocks.
fn heads_in(blocks: &[(usize, usize)]) -> usize {
    blocks.iter().map(|(_, n)| n).sum()
}

/// The per-head ranges scaled to columns of `width` each.
fn head_cols(heads: &[(usize, usize)], width: usize) -> Vec<(usize, usize)> {
    heads.iter().map(|(h, n)| (h * width, n * width)).collect()
}

impl Program<'_> {
    /// Chip `c`'s slice of a full tensor `of` (`of_shape` whole): declared under its own name, recorded in the manifest.
    #[allow(clippy::too_many_arguments)]
    fn decl_slice(
        &mut self,
        m: &mut TpManifest,
        var: &str,
        c: usize,
        of: &str,
        of_shape: (usize, usize),
        axis: SliceAxis,
        ranges: Vec<(usize, usize)>,
    ) {
        let slice = TpSlice {
            of: of.to_string(),
            of_shape,
            axis,
            ranges,
        };
        let (r, cols) = slice.shape();
        self.s.push_str(&format!("Let {var} be a {r} by {cols} tensor of Real.\n"));
        m.place(var, c, Some(slice));
    }

    /// [`Program::decl_slice`] of a weight the checkpoint stores Q8_0: the full tensor is recorded as `decl_q8` records it.
    #[allow(clippy::too_many_arguments)]
    fn decl_q8_slice(
        &mut self,
        m: &mut TpManifest,
        var: &str,
        c: usize,
        of: &str,
        of_shape: (usize, usize),
        axis: SliceAxis,
        ranges: Vec<(usize, usize)>,
    ) {
        self.q8w.insert(of.to_string(), format!("{}/{of}", self.dir));
        self.decl_slice(m, var, c, of, of_shape, axis, ranges);
    }

    /// `out = Σ partials` as the left fold of `plus` it means — `out_f1 = p0 + p1`, `out_f2 = out_f1 + p2`, …, the
    /// last binding `out` — recorded as a collective.
    fn fold(&mut self, m: &mut TpManifest, out: &str, partials: Vec<String>) {
        let mut folds = Vec::new();
        let mut acc = partials[0].clone();
        for (i, p) in partials.iter().enumerate().skip(1) {
            let binder = if i + 1 == partials.len() {
                out.to_string()
            } else {
                format!("{out}_f{i}")
            };
            self.s.push_str(&format!("Let {binder} be {acc} plus {p}.\n"));
            folds.push(binder.clone());
            acc = binder;
        }
        m.collectives.push(TpCollective {
            out: out.to_string(),
            partials,
            folds,
        });
    }

    /// Chip `c`'s partial `out` of a layer it holds no share of — the empty sum: the replicated input `like` scaled by
    /// zero, a zero for each of its entries.
    fn zero_partial(&mut self, m: &mut TpManifest, out: &str, like: &str, c: usize) -> String {
        self.s.push_str(&format!("Let {out} be {like} scaled by 0.0.\n"));
        m.place(out, c, None);
        out.to_string()
    }

    /// Chip `c`'s slices of an attention layer's weights: its query (and gate) columns, its KV heads' key and value
    /// columns, its query heads' rows of the output projection.
    fn decl_attn_weight_slices(&mut self, m: &mut TpManifest, a: &Attention, l: usize, c: usize, tp: usize) {
        let Dims { d, aq, akv, avd, ao, hd, .. } = attn_dims(self.spec, a);
        let vd = a.value_dim;
        let ((q0, hq), (kv0, hkv)) = attn_heads_of_chip(a, tp, c);
        let (q, k) = (vec![(q0 * hd, hq * hd)], vec![(kv0 * hd, hkv * hd)]);
        let (o, v) = (vec![(q0 * vd, hq * vd)], vec![(kv0 * vd, hkv * vd)]);
        self.decl_q8_slice(m, &format!("Wq{l}_c{c}"), c, &format!("Wq{l}"), (d, aq), SliceAxis::Cols, q.clone());
        if a.output_gate {
            self.decl_q8_slice(m, &format!("Wqg{l}_c{c}"), c, &format!("Wqg{l}"), (d, aq), SliceAxis::Cols, q.clone());
        }
        self.decl_q8_slice(m, &format!("Wk{l}_c{c}"), c, &format!("Wk{l}"), (d, akv), SliceAxis::Cols, k.clone());
        self.decl_q8_slice(m, &format!("Wv{l}_c{c}"), c, &format!("Wv{l}"), (d, avd), SliceAxis::Cols, v);
        self.decl_q8_slice(m, &format!("Wo{l}_c{c}"), c, &format!("Wo{l}"), (ao, d), SliceAxis::Rows, o);
    }

    /// Chip `c`'s slice of an attention layer's KV caches `Kc{l}{sfx}` / `Vc{l}{sfx}`: its KV heads' columns.
    #[allow(clippy::too_many_arguments)]
    fn decl_attn_cache_slices(
        &mut self,
        m: &mut TpManifest,
        a: &Attention,
        l: usize,
        c: usize,
        tp: usize,
        ctx: usize,
        sfx: &str,
    ) {
        let Dims { akv, avd, hd, .. } = attn_dims(self.spec, a);
        let (_, (kv0, hkv)) = attn_heads_of_chip(a, tp, c);
        let (k, v) = (vec![(kv0 * hd, hkv * hd)], vec![(kv0 * a.value_dim, hkv * a.value_dim)]);
        self.decl_slice(m, &format!("Kc{l}{sfx}_c{c}"), c, &format!("Kc{l}{sfx}"), (ctx, akv), SliceAxis::Cols, k);
        self.decl_slice(m, &format!("Vc{l}{sfx}_c{c}"), c, &format!("Vc{l}{sfx}"), (ctx, avd), SliceAxis::Cols, v);
    }

    /// Chip `c`'s slices of a Gated-DeltaNet layer's weights: its key heads' q/k projection and conv columns, its
    /// value heads' v/z projection and conv columns, gate columns, decay parameters and output-projection rows.
    fn decl_gdn_weight_slices(&mut self, m: &mut TpManifest, g: &GatedDelta, l: usize, c: usize, tp: usize) {
        let d = self.spec.d;
        let (taps, ghd, nv) = (g.conv_taps, g.head_dim, g.v_heads);
        let (qk, vw) = (g.k_heads * ghd, nv * ghd);
        let (k, heads) = gdn_heads_of_chip(g, tp, c);
        let v = head_cols(&heads, ghd);
        let n = |base: &str| format!("{base}{l}_c{c}");
        let f = |base: &str| format!("{base}{l}");
        self.decl_q8_slice(m, &n("Wq"), c, &f("Wq"), (d, qk), SliceAxis::Cols, k.clone());
        self.decl_q8_slice(m, &n("Wk"), c, &f("Wk"), (d, qk), SliceAxis::Cols, k.clone());
        self.decl_q8_slice(m, &n("Wv"), c, &f("Wv"), (d, vw), SliceAxis::Cols, v.clone());
        self.decl_q8_slice(m, &n("Wz"), c, &f("Wz"), (d, vw), SliceAxis::Cols, v.clone());
        self.decl_slice(m, &n("Wcq"), c, &f("Wcq"), (taps, qk), SliceAxis::Cols, k.clone());
        self.decl_slice(m, &n("Wck"), c, &f("Wck"), (taps, qk), SliceAxis::Cols, k);
        self.decl_slice(m, &n("Wcv"), c, &f("Wcv"), (taps, vw), SliceAxis::Cols, v.clone());
        self.decl_q8_slice(m, &n("Wb"), c, &f("Wb"), (d, nv), SliceAxis::Cols, heads.clone());
        self.decl_q8_slice(m, &n("Wa"), c, &f("Wa"), (d, nv), SliceAxis::Cols, heads.clone());
        self.decl_slice(m, &n("Dt"), c, &f("Dt"), (1, nv), SliceAxis::Cols, heads.clone());
        self.decl_slice(m, &n("Al"), c, &f("Al"), (1, nv), SliceAxis::Cols, heads);
        self.decl_q8_slice(m, &n("Wo"), c, &f("Wo"), (vw, d), SliceAxis::Rows, v);
    }

    /// Chip `c`'s slice of a Gated-DeltaNet layer's carried state `…{l}{sfx}`: the conv rings' columns and the
    /// memory's rows of its heads.
    #[allow(clippy::too_many_arguments)]
    fn decl_gdn_state_slices(&mut self, m: &mut TpManifest, g: &GatedDelta, l: usize, c: usize, tp: usize, sfx: &str) {
        let (taps, ghd) = (g.conv_taps, g.head_dim);
        let (qk, vw) = (g.k_heads * ghd, g.v_heads * ghd);
        let (k, heads) = gdn_heads_of_chip(g, tp, c);
        let v = head_cols(&heads, ghd);
        self.decl_slice(m, &format!("Csq{l}{sfx}_c{c}"), c, &format!("Csq{l}{sfx}"), (taps, qk), SliceAxis::Cols, k.clone());
        self.decl_slice(m, &format!("Csk{l}{sfx}_c{c}"), c, &format!("Csk{l}{sfx}"), (taps, qk), SliceAxis::Cols, k);
        self.decl_slice(m, &format!("Csv{l}{sfx}_c{c}"), c, &format!("Csv{l}{sfx}"), (taps, vw), SliceAxis::Cols, v.clone());
        self.decl_slice(m, &format!("St{l}{sfx}_c{c}"), c, &format!("St{l}{sfx}"), (vw, ghd), SliceAxis::Rows, v);
    }

    /// Chip `c`'s slices of a layer's dense FFN: its gate and up columns and its down-projection rows.
    fn decl_ffn_slices(&mut self, m: &mut TpManifest, l: usize, dff: usize, c: usize, tp: usize) {
        let d = self.spec.d;
        let f = vec![run_of(dff, tp, c)];
        self.decl_q8_slice(m, &format!("Wg{l}_c{c}"), c, &format!("Wg{l}"), (d, dff), SliceAxis::Cols, f.clone());
        self.decl_q8_slice(m, &format!("Wu{l}_c{c}"), c, &format!("Wu{l}"), (d, dff), SliceAxis::Cols, f.clone());
        self.decl_q8_slice(m, &format!("Wd{l}_c{c}"), c, &format!("Wd{l}"), (dff, d), SliceAxis::Rows, f);
    }

    /// Layer `l` of a decode step over `tp` chips.
    fn tp_layer(&mut self, m: &mut TpManifest, l: usize, cur: &str, tp: usize, ctx: usize) {
        let spec = self.spec;
        let d = spec.d;
        self.decl_f32(&format!("an{l}"), 1, d);
        self.s.push_str(&format!("Let xn{l} be rmsnorm of {cur} with weight an{l}.\n"));
        let mut ao = Vec::new();
        match &spec.layers[l].mixer {
            Mixer::Attention(a) => {
                if a.qk_norm {
                    self.decl_f32(&format!("Qn{l}"), 1, a.head_dim);
                    self.decl_f32(&format!("Kn{l}"), 1, a.head_dim);
                }
                for c in 0..tp {
                    let ((_, hq), (_, hkv)) = attn_heads_of_chip(a, tp, c);
                    if hq == 0 {
                        ao.push(self.zero_partial(m, &format!("ao{l}_c{c}"), &format!("xn{l}"), c));
                        continue;
                    }
                    self.decl_attn_weight_slices(m, a, l, c, tp);
                    self.decl_attn_cache_slices(m, a, l, c, tp, ctx, "");
                    let mut bases = vec!["qr", "kr", "vr", "ct", "ao"];
                    if a.output_gate {
                        bases.extend(["gr", "gs", "cg"]);
                    }
                    if a.qk_norm {
                        bases.extend(["qm", "km"]);
                    }
                    if a.rope.is_some() {
                        bases.extend(["qp", "kp"]);
                    }
                    if a.qk_unit_norm {
                        bases.extend(["qu", "ku"]);
                    }
                    if a.temperature.is_some() {
                        bases.push("qt");
                    }
                    for base in bases {
                        m.place(&format!("{base}{l}_c{c}"), c, None);
                    }
                    self.s.push_str(&format!("Let qr{l}_c{c} be xn{l} matmul Wq{l}_c{c}.\n"));
                    if a.output_gate {
                        self.s.push_str(&format!("Let gr{l}_c{c} be xn{l} matmul Wqg{l}_c{c}.\n"));
                    }
                    self.s.push_str(&format!(
                        "Let kr{l}_c{c} be xn{l} matmul Wk{l}_c{c}.\nLet vr{l}_c{c} be xn{l} matmul Wv{l}_c{c}.\n"
                    ));
                    let (mut q, mut k) = (format!("qr{l}_c{c}"), format!("kr{l}_c{c}"));
                    if a.qk_norm {
                        self.s.push_str(&format!(
                            "Let qm{l}_c{c} be rmsnorm of qr{l}_c{c} with weight Qn{l} across {hq} heads.\n\
                             Let km{l}_c{c} be rmsnorm of kr{l}_c{c} with weight Kn{l} across {hkv} heads.\n"
                        ));
                        (q, k) = (format!("qm{l}_c{c}"), format!("km{l}_c{c}"));
                    }
                    if a.rope.is_some() {
                        let angles = self.angles(l, AttnForm::Cached { ctx }, "pos");
                        self.s.push_str(&format!(
                            "Let qp{l}_c{c} be rope of {q} with {angles} across {hq} heads.\n\
                             Let kp{l}_c{c} be rope of {k} with {angles} across {hkv} heads.\n"
                        ));
                        (q, k) = (format!("qp{l}_c{c}"), format!("kp{l}_c{c}"));
                    }
                    (q, k) = self.llama4_qk(a, (q, k), (hq, hkv), hq, Some(("pos".to_string(), ctx)), &|base| format!("{base}{l}_c{c}"));
                    let w = score_clauses(a);
                    self.s.push_str(&format!(
                        "Set row pos of Kc{l}_c{c} to {k}.\n\
                         Set row pos of Vc{l}_c{c} to vr{l}_c{c}.\n\
                         Let ct{l}_c{c} be attention of {q} over Kc{l}_c{c} and Vc{l}_c{c} with {hq} heads {hkv} groups up to pos{w}.\n"
                    ));
                    if a.output_gate {
                        self.s.push_str(&format!(
                            "Let gs{l}_c{c} be sigmoid of gr{l}_c{c}.\n\
                             Let cg{l}_c{c} be ct{l}_c{c} hadamard gs{l}_c{c}.\n\
                             Let ao{l}_c{c} be cg{l}_c{c} matmul Wo{l}_c{c}.\n"
                        ));
                    } else {
                        self.s.push_str(&format!("Let ao{l}_c{c} be ct{l}_c{c} matmul Wo{l}_c{c}.\n"));
                    }
                    ao.push(format!("ao{l}_c{c}"));
                }
            }
            Mixer::GatedDelta(g) => {
                let gate = gate_clause(g);
                let (taps, ghd) = (g.conv_taps, g.head_dim);
                self.decl_f32(&format!("Gn{l}"), 1, ghd);
                for c in 0..tp {
                    let (k, heads) = gdn_heads_of_chip(g, tp, c);
                    let (nk_c, nv_c) = (k[0].1 / ghd, heads_in(&heads));
                    if nv_c == 0 {
                        ao.push(self.zero_partial(m, &format!("ao{l}_c{c}"), &format!("xn{l}"), c));
                        continue;
                    }
                    self.decl_gdn_weight_slices(m, g, l, c, tp);
                    self.decl_gdn_state_slices(m, g, l, c, tp, "");
                    for base in [
                        "qr", "kr", "vr", "zz", "qc", "kc", "vc", "qs", "ks", "vs", "qn", "kn", "br", "bt", "ar",
                        "dy", "od", "ao",
                    ] {
                        m.place(&format!("{base}{l}_c{c}"), c, None);
                    }
                    self.s.push_str(&format!(
                        "Let qr{l}_c{c} be xn{l} matmul Wq{l}_c{c}.\n\
                         Let kr{l}_c{c} be xn{l} matmul Wk{l}_c{c}.\n\
                         Let vr{l}_c{c} be xn{l} matmul Wv{l}_c{c}.\n\
                         Let zz{l}_c{c} be xn{l} matmul Wz{l}_c{c}.\n\
                         Let qc{l}_c{c} be causal conv of qr{l}_c{c} with weight Wcq{l}_c{c} state Csq{l}_c{c} over {taps} taps.\n\
                         Let kc{l}_c{c} be causal conv of kr{l}_c{c} with weight Wck{l}_c{c} state Csk{l}_c{c} over {taps} taps.\n\
                         Let vc{l}_c{c} be causal conv of vr{l}_c{c} with weight Wcv{l}_c{c} state Csv{l}_c{c} over {taps} taps.\n\
                         Let qs{l}_c{c} be silu of qc{l}_c{c}.\n\
                         Let ks{l}_c{c} be silu of kc{l}_c{c}.\n\
                         Let vs{l}_c{c} be silu of vc{l}_c{c}.\n\
                         Let qn{l}_c{c} be l2norm of qs{l}_c{c} across {nk_c} heads.\n\
                         Let kn{l}_c{c} be l2norm of ks{l}_c{c} across {nk_c} heads.\n\
                         Let br{l}_c{c} be xn{l} matmul Wb{l}_c{c}.\n\
                         Let bt{l}_c{c} be sigmoid of br{l}_c{c}.\n\
                         Let ar{l}_c{c} be xn{l} matmul Wa{l}_c{c}.\n\
                         Let dy{l}_c{c} be decay of ar{l}_c{c} with bias Dt{l}_c{c} rate Al{l}_c{c}.\n\
                         Let od{l}_c{c} be gated delta of qn{l}_c{c} over kn{l}_c{c} and vs{l}_c{c} with {gate} zz{l}_c{c} beta bt{l}_c{c} decay dy{l}_c{c} state St{l}_c{c} norm Gn{l} with {nv_c} heads {nk_c} groups.\n\
                         Let ao{l}_c{c} be od{l}_c{c} matmul Wo{l}_c{c}.\n"
                    ));
                    ao.push(format!("ao{l}_c{c}"));
                }
            }
            Mixer::Mla(_) => unreachable!("author::check_tp refuses latent attention"),
        }
        self.fold(m, &format!("ao{l}"), ao);
        self.decl_f32(&format!("fn{l}"), 1, d);
        let ao = self.post_norm(l, &format!("ao{l}"), true);
        self.s.push_str(&format!(
            "Let xr{l} be {cur} plus {ao}.\nLet x2n{l} be rmsnorm of xr{l} with weight fn{l}.\n"
        ));
        let Ffn::Dense { dff, .. } = spec.layers[l].ffn else {
            unreachable!("check_tp refuses a tensor-parallel expert mixture")
        };
        let mut dn = Vec::new();
        for c in 0..tp {
            if run_of(dff, tp, c).1 == 0 {
                dn.push(self.zero_partial(m, &format!("dn{l}_c{c}"), &format!("x2n{l}"), c));
                continue;
            }
            self.decl_ffn_slices(m, l, dff, c, tp);
            self.s.push_str(&format!(
                "Let gt{l}_c{c} be x2n{l} matmul Wg{l}_c{c}.\n\
                 Let up{l}_c{c} be x2n{l} matmul Wu{l}_c{c}.\n"
            ));
            let bound = self.glu(l, &format!("gt{l}_c{c}"), &format!("up{l}_c{c}"), &format!("hh{l}_c{c}"));
            self.s.push_str(&format!("Let dn{l}_c{c} be hh{l}_c{c} matmul Wd{l}_c{c}.\n"));
            for name in [format!("gt{l}_c{c}"), format!("up{l}_c{c}")].into_iter().chain(bound).chain([format!("dn{l}_c{c}")]) {
                m.place(&name, c, None);
            }
            dn.push(format!("dn{l}_c{c}"));
        }
        self.fold(m, &format!("dn{l}"), dn);
        let dn = self.post_norm(l, &format!("dn{l}"), false);
        self.s.push_str(&format!("Let xo{l} be xr{l} plus {dn}.\n"));
    }

    /// Layer `l` of a BATCHED decode step over `tp` chips: the stack and its norms replicated; chip `c`'s
    /// column-parallel projections on the whole stack; each sequence's per-head work on its row of the chip's stack;
    /// the chip's row-parallel output projection a `bsz × d` partial; the partials summed by one all-reduce of the
    /// stack. Per-sequence chip tensors are `…{l}_s{b}_c{c}`; chip stacks are `b…{l}_c{c}`.
    #[allow(clippy::too_many_arguments)]
    fn stacked_tp_layer(&mut self, m: &mut TpManifest, l: usize, cur: &str, tp: usize, ctx: usize, bsz: usize) {
        let spec = self.spec;
        let d = spec.d;
        self.decl_f32(&format!("an{l}"), 1, d);
        self.s.push_str(&format!("Let bxn{l} be rmsnorm of {cur} with weight an{l}.\n"));
        let mut ao = Vec::new();
        match &spec.layers[l].mixer {
            Mixer::Attention(a) => {
                let hd = a.head_dim;
                if a.qk_norm {
                    self.decl_f32(&format!("Qn{l}"), 1, hd);
                    self.decl_f32(&format!("Kn{l}"), 1, hd);
                }
                for c in 0..tp {
                    let ((_, hq), (_, hkv)) = attn_heads_of_chip(a, tp, c);
                    if hq == 0 {
                        ao.push(self.zero_partial(m, &format!("bao{l}_c{c}"), &format!("bxn{l}"), c));
                        continue;
                    }
                    self.decl_attn_weight_slices(m, a, l, c, tp);
                    for b in 0..bsz {
                        self.decl_attn_cache_slices(m, a, l, c, tp, ctx, &format!("_s{b}"));
                    }
                    let mut bases = vec!["bqr", "bkr", "bvr", "bct", "bao"];
                    if a.output_gate {
                        bases.extend(["bgr", "bgs", "bcg"]);
                    }
                    for base in bases {
                        m.place(&format!("{base}{l}_c{c}"), c, None);
                    }
                    self.s.push_str(&format!("Let bqr{l}_c{c} be bxn{l} matmul Wq{l}_c{c}.\n"));
                    if a.output_gate {
                        self.s.push_str(&format!("Let bgr{l}_c{c} be bxn{l} matmul Wqg{l}_c{c}.\n"));
                    }
                    self.s.push_str(&format!(
                        "Let bkr{l}_c{c} be bxn{l} matmul Wk{l}_c{c}.\n\
                         Let bvr{l}_c{c} be bxn{l} matmul Wv{l}_c{c}.\n\
                         Let bct{l}_c{c} be a {bsz} by {} tensor of Real.\n",
                        hq * a.value_dim
                    ));
                    for b in 0..bsz {
                        let r = b + 1;
                        let mut bases = vec!["qr", "kr", "vr", "ct"];
                        if a.qk_norm {
                            bases.extend(["qm", "km"]);
                        }
                        if a.rope.is_some() {
                            bases.extend(["qp", "kp"]);
                        }
                        if a.qk_unit_norm {
                            bases.extend(["qu", "ku"]);
                        }
                        if a.temperature.is_some() {
                            bases.push("qt");
                        }
                        for base in bases {
                            m.place(&format!("{base}{l}_s{b}_c{c}"), c, None);
                        }
                        self.s.push_str(&format!(
                            "Let qr{l}_s{b}_c{c} be row {r} of bqr{l}_c{c}.\n\
                             Let kr{l}_s{b}_c{c} be row {r} of bkr{l}_c{c}.\n\
                             Let vr{l}_s{b}_c{c} be row {r} of bvr{l}_c{c}.\n"
                        ));
                        let (mut q, mut k) = (format!("qr{l}_s{b}_c{c}"), format!("kr{l}_s{b}_c{c}"));
                        if a.qk_norm {
                            self.s.push_str(&format!(
                                "Let qm{l}_s{b}_c{c} be rmsnorm of qr{l}_s{b}_c{c} with weight Qn{l} across {hq} heads.\n\
                                 Let km{l}_s{b}_c{c} be rmsnorm of kr{l}_s{b}_c{c} with weight Kn{l} across {hkv} heads.\n"
                            ));
                            (q, k) = (format!("qm{l}_s{b}_c{c}"), format!("km{l}_s{b}_c{c}"));
                        }
                        if a.rope.is_some() {
                            let angles = self.angles(l, AttnForm::Cached { ctx }, &format!("pos_s{b}"));
                            self.s.push_str(&format!(
                                "Let qp{l}_s{b}_c{c} be rope of {q} with {angles} across {hq} heads.\n\
                                 Let kp{l}_s{b}_c{c} be rope of {k} with {angles} across {hkv} heads.\n"
                            ));
                            (q, k) = (format!("qp{l}_s{b}_c{c}"), format!("kp{l}_s{b}_c{c}"));
                        }
                        (q, k) = self.llama4_qk(a, (q, k), (hq, hkv), hq, Some((format!("pos_s{b}"), ctx)), &|base| format!("{base}{l}_s{b}_c{c}"));
                        let w = score_clauses(a);
                        self.s.push_str(&format!(
                            "Set row pos_s{b} of Kc{l}_s{b}_c{c} to {k}.\n\
                             Set row pos_s{b} of Vc{l}_s{b}_c{c} to vr{l}_s{b}_c{c}.\n\
                             Let ct{l}_s{b}_c{c} be attention of {q} over Kc{l}_s{b}_c{c} and Vc{l}_s{b}_c{c} with {hq} heads {hkv} groups up to pos_s{b}{w}.\n\
                             Set row {r} of bct{l}_c{c} to ct{l}_s{b}_c{c}.\n"
                        ));
                    }
                    if a.output_gate {
                        self.s.push_str(&format!(
                            "Let bgs{l}_c{c} be sigmoid of bgr{l}_c{c}.\n\
                             Let bcg{l}_c{c} be bct{l}_c{c} hadamard bgs{l}_c{c}.\n\
                             Let bao{l}_c{c} be bcg{l}_c{c} matmul Wo{l}_c{c}.\n"
                        ));
                    } else {
                        self.s.push_str(&format!("Let bao{l}_c{c} be bct{l}_c{c} matmul Wo{l}_c{c}.\n"));
                    }
                    ao.push(format!("bao{l}_c{c}"));
                }
            }
            Mixer::GatedDelta(g) => {
                let gate = gate_clause(g);
                let (taps, ghd) = (g.conv_taps, g.head_dim);
                self.decl_f32(&format!("Gn{l}"), 1, ghd);
                for c in 0..tp {
                    let (k, heads) = gdn_heads_of_chip(g, tp, c);
                    let (nk_c, nv_c) = (k[0].1 / ghd, heads_in(&heads));
                    if nv_c == 0 {
                        ao.push(self.zero_partial(m, &format!("bao{l}_c{c}"), &format!("bxn{l}"), c));
                        continue;
                    }
                    self.decl_gdn_weight_slices(m, g, l, c, tp);
                    for b in 0..bsz {
                        self.decl_gdn_state_slices(m, g, l, c, tp, &format!("_s{b}"));
                    }
                    for base in ["bqr", "bkr", "bvr", "bzz", "bbr", "bar", "bod", "bao"] {
                        m.place(&format!("{base}{l}_c{c}"), c, None);
                    }
                    self.s.push_str(&format!(
                        "Let bqr{l}_c{c} be bxn{l} matmul Wq{l}_c{c}.\n\
                         Let bkr{l}_c{c} be bxn{l} matmul Wk{l}_c{c}.\n\
                         Let bvr{l}_c{c} be bxn{l} matmul Wv{l}_c{c}.\n\
                         Let bzz{l}_c{c} be bxn{l} matmul Wz{l}_c{c}.\n\
                         Let bbr{l}_c{c} be bxn{l} matmul Wb{l}_c{c}.\n\
                         Let bar{l}_c{c} be bxn{l} matmul Wa{l}_c{c}.\n\
                         Let bod{l}_c{c} be a {bsz} by {} tensor of Real.\n",
                        nv_c * ghd
                    ));
                    for b in 0..bsz {
                        let r = b + 1;
                        for base in [
                            "qr", "kr", "vr", "zz", "br", "ar", "qc", "kc", "vc", "qs", "ks", "vs", "qn", "kn", "bt",
                            "dy", "od",
                        ] {
                            m.place(&format!("{base}{l}_s{b}_c{c}"), c, None);
                        }
                        self.s.push_str(&format!(
                            "Let qr{l}_s{b}_c{c} be row {r} of bqr{l}_c{c}.\n\
                             Let kr{l}_s{b}_c{c} be row {r} of bkr{l}_c{c}.\n\
                             Let vr{l}_s{b}_c{c} be row {r} of bvr{l}_c{c}.\n\
                             Let zz{l}_s{b}_c{c} be row {r} of bzz{l}_c{c}.\n\
                             Let br{l}_s{b}_c{c} be row {r} of bbr{l}_c{c}.\n\
                             Let ar{l}_s{b}_c{c} be row {r} of bar{l}_c{c}.\n\
                             Let qc{l}_s{b}_c{c} be causal conv of qr{l}_s{b}_c{c} with weight Wcq{l}_c{c} state Csq{l}_s{b}_c{c} over {taps} taps.\n\
                             Let kc{l}_s{b}_c{c} be causal conv of kr{l}_s{b}_c{c} with weight Wck{l}_c{c} state Csk{l}_s{b}_c{c} over {taps} taps.\n\
                             Let vc{l}_s{b}_c{c} be causal conv of vr{l}_s{b}_c{c} with weight Wcv{l}_c{c} state Csv{l}_s{b}_c{c} over {taps} taps.\n\
                             Let qs{l}_s{b}_c{c} be silu of qc{l}_s{b}_c{c}.\n\
                             Let ks{l}_s{b}_c{c} be silu of kc{l}_s{b}_c{c}.\n\
                             Let vs{l}_s{b}_c{c} be silu of vc{l}_s{b}_c{c}.\n\
                             Let qn{l}_s{b}_c{c} be l2norm of qs{l}_s{b}_c{c} across {nk_c} heads.\n\
                             Let kn{l}_s{b}_c{c} be l2norm of ks{l}_s{b}_c{c} across {nk_c} heads.\n\
                             Let bt{l}_s{b}_c{c} be sigmoid of br{l}_s{b}_c{c}.\n\
                             Let dy{l}_s{b}_c{c} be decay of ar{l}_s{b}_c{c} with bias Dt{l}_c{c} rate Al{l}_c{c}.\n\
                             Let od{l}_s{b}_c{c} be gated delta of qn{l}_s{b}_c{c} over kn{l}_s{b}_c{c} and vs{l}_s{b}_c{c} with {gate} zz{l}_s{b}_c{c} beta bt{l}_s{b}_c{c} decay dy{l}_s{b}_c{c} state St{l}_s{b}_c{c} norm Gn{l} with {nv_c} heads {nk_c} groups.\n\
                             Set row {r} of bod{l}_c{c} to od{l}_s{b}_c{c}.\n"
                        ));
                    }
                    self.s.push_str(&format!("Let bao{l}_c{c} be bod{l}_c{c} matmul Wo{l}_c{c}.\n"));
                    ao.push(format!("bao{l}_c{c}"));
                }
            }
            Mixer::Mla(_) => unreachable!("author::check_tp refuses latent attention"),
        }
        self.fold(m, &format!("bao{l}"), ao);
        self.decl_f32(&format!("fn{l}"), 1, d);
        let ao = self.post_norm(l, &format!("bao{l}"), true);
        self.s.push_str(&format!(
            "Let bxr{l} be {cur} plus {ao}.\nLet bx2n{l} be rmsnorm of bxr{l} with weight fn{l}.\n"
        ));
        let Ffn::Dense { dff, .. } = spec.layers[l].ffn else {
            unreachable!("check_tp refuses a tensor-parallel expert mixture")
        };
        let mut dn = Vec::new();
        for c in 0..tp {
            if run_of(dff, tp, c).1 == 0 {
                dn.push(self.zero_partial(m, &format!("bdn{l}_c{c}"), &format!("bx2n{l}"), c));
                continue;
            }
            self.decl_ffn_slices(m, l, dff, c, tp);
            self.s.push_str(&format!(
                "Let bgt{l}_c{c} be bx2n{l} matmul Wg{l}_c{c}.\n\
                 Let bup{l}_c{c} be bx2n{l} matmul Wu{l}_c{c}.\n"
            ));
            let bound = self.glu(l, &format!("bgt{l}_c{c}"), &format!("bup{l}_c{c}"), &format!("bhh{l}_c{c}"));
            self.s.push_str(&format!("Let bdn{l}_c{c} be bhh{l}_c{c} matmul Wd{l}_c{c}.\n"));
            for name in [format!("bgt{l}_c{c}"), format!("bup{l}_c{c}")].into_iter().chain(bound).chain([format!("bdn{l}_c{c}")]) {
                m.place(&name, c, None);
            }
            dn.push(format!("bdn{l}_c{c}"));
        }
        self.fold(m, &format!("bdn{l}"), dn);
        let dn = self.post_norm(l, &format!("bdn{l}"), false);
        self.s.push_str(&format!("Let bxo{l} be bxr{l} plus {dn}.\n"));
    }

    fn finish_tp(self, m: TpManifest) -> TpAuthored {
        let (s, f, q) = self.finish();
        (s, f, q, m)
    }
}

/// A whole decode step over `tp` chips: the embedding lookup and the head replicated, every layer split.
pub fn tp_decode_step(spec: &ModelSpec, dir: &str, ctx: usize, tok: usize, pos: usize, tp: usize) -> Result<TpAuthored, NotYet> {
    check_tp(spec, tp)?;
    let mut p = Program::new(spec, dir);
    let mut m = TpManifest::new(tp);
    p.s.push_str(&format!("Let tok be {tok}.\nLet pos be {pos}.\n"));
    p.embed();
    p.rope_cache(ctx);
    let mut cur = "x".to_string();
    for l in 0..spec.layers.len() {
        p.tp_layer(&mut m, l, &cur, tp, ctx);
        cur = format!("xo{l}");
    }
    p.head(&cur);
    p.next_token();
    Ok(p.finish_tp(m))
}

/// A BATCHED decode step over `tp` chips — the serving shape: the stacked sequences with every layer split, the
/// embedding and the head on the replicated spine.
pub fn tp_decode_batch(spec: &ModelSpec, dir: &str, ctx: usize, seqs: &[(usize, usize)], tp: usize) -> Result<TpAuthored, NotYet> {
    check_tp(spec, tp)?;
    let bsz = seqs.len();
    let mut p = Program::new(spec, dir);
    let mut m = TpManifest::new(tp);
    for (b, (_, pos)) in seqs.iter().enumerate() {
        p.s.push_str(&format!("Let pos_s{b} be {pos}.\n"));
    }
    p.batched_embed(&seqs.iter().map(|(tok, _)| *tok).collect::<Vec<_>>());
    p.rope_cache(ctx);
    let mut cur = stacked_layer_input(0);
    for l in 0..spec.layers.len() {
        p.stacked_tp_layer(&mut m, l, &cur, tp, ctx, bsz);
        cur = stacked_layer_input(l + 1);
    }
    p.batched_head(&cur, bsz);
    Ok(p.finish_tp(m))
}

/// Layer `l` of a tensor-parallel decode step alone: its input and position declared, the rotary tables for an
/// attention layer, then the split layer.
pub fn tp_decode_layer(spec: &ModelSpec, l: usize, dir: &str, ctx: usize, pos: usize, tp: usize) -> Result<TpAuthored, NotYet> {
    check_tp(spec, tp)?;
    let mut p = Program::new(spec, dir);
    let mut m = TpManifest::new(tp);
    let input = layer_input(l);
    p.s.push_str(&format!("Let pos be {pos}.\n"));
    p.s.push_str(&format!("Let {input} be a 1 by {} tensor of Real.\n", spec.d));
    p.rotary_tables(Some(l), true, ctx);
    p.tp_layer(&mut m, l, &input, tp, ctx);
    Ok(p.finish_tp(m))
}

/// Layer `l` of a batched tensor-parallel decode step alone.
pub fn tp_stacked_layer(spec: &ModelSpec, l: usize, dir: &str, ctx: usize, poss: &[usize], tp: usize) -> Result<TpAuthored, NotYet> {
    check_tp(spec, tp)?;
    let bsz = poss.len();
    let mut p = Program::new(spec, dir);
    let mut m = TpManifest::new(tp);
    for (b, pos) in poss.iter().enumerate() {
        p.s.push_str(&format!("Let pos_s{b} be {pos}.\n"));
    }
    let input = stacked_layer_input(l);
    p.s.push_str(&format!("Let {input} be a {bsz} by {} tensor of Real.\n", spec.d));
    p.rotary_tables(Some(l), true, ctx);
    p.stacked_tp_layer(&mut m, l, &input, tp, ctx, bsz);
    Ok(p.finish_tp(m))
}
