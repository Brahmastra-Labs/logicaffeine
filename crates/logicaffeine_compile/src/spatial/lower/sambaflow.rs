//! `largo emit rdu --format sambaflow` — the authored tensor program as the PyTorch module SambaFlow ingests.
//!
//! SambaFlow's front end takes PyTorch, so the vendor-format artifact for that front door is a
//! `torch.nn.Module` whose `forward` mirrors the program's tensor statements ONE OP PER STATEMENT, in dataflow
//! order, plus a weights manifest (variable → shape, role, checkpoint tensor name where the naming convention
//! knows it) and a `compile.py` scaffold marked TEMPLATE where it calls the vendor API. It is an emitted
//! vendor-ingestion artifact — like an emitted `.cu` — and never our compute path: nothing of ours runs from
//! the Python, and the header says so.
//!
//! The reader ([`read_tensor_sentences`]) walks the program's sentences in the tensor sugar the parser
//! defines (`Let Y be rmsnorm of X with weight W across H heads.` and its siblings); the real parser is asked
//! to accept the program first, and the one IR's lift of it (`TensorGraph::lift_source`) is asked to agree on the ops
//! they both know, so the sentence reader can never drift silently from the language. Every
//! spelling mirrors the parser's DESUGAR — the rmsnorm eps, the NEOX rope pairs with the partial-rotary
//! pass-through, attention's `1/√hd` with `kvh = (head-1)/(H/KV)`, decay's `(1+e^u)^{alog}`, the conv ring
//! shift, the Gated DeltaNet recurrence with `kb = ((h-1) mod hk)·d` — and `rdu_sambaflow_emit_lock` checks
//! the module against the VM's logits when torch is present. An op without a spelling is refused BY NAME.

use std::collections::{BTreeMap, BTreeSet};

use crate::tensor_graph::graph::{Storage, TensorGraph};

/// The first line of every emitted file.
pub const VENDOR_HEADER: &str =
    "# vendor-ingestion artifact emitted by LOGOS — not our compute path";
/// The mark on every line of `compile.py` that calls an API nobody here has run.
pub const TEMPLATE_MARK: &str = "# TEMPLATE — SambaFlow API unverified";
/// The ops the authored Qwen3.8 decode step needs; every one has a spelling in [`torch_spelling`].
pub const QWEN38_OPS: &[&str] = &[
    "rmsnorm",
    "rope",
    "attention",
    "sigmoid",
    "hadamard",
    "causal conv",
    "silu",
    "l2norm",
    "decay",
    "gated delta",
    "swiglu",
    "matmul",
    "argmax",
    "embedding",
];

pub struct TorchArtifact {
    pub model_py: String,
    pub compile_py: String,
    pub manifest_json: String,
}

/// One sentence of a tensor program.
#[derive(Clone, Debug, PartialEq)]
pub enum TensorSentence {
    /// `Let V be a R by C tensor of Int|Real.`
    Decl {
        var: String,
        rows: usize,
        cols: usize,
        real: bool,
    },
    /// `Set item N of V to VALUE.`
    Fill {
        var: String,
        index: usize,
        value: String,
    },
    /// `Let OUT be <op> …`, or `Set row P of T to R.`
    Op { out: String, kind: OpKind },
    /// `Show …` — host output, nothing for the module.
    Show,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OpKind {
    Matmul {
        a: String,
        b: String,
    },
    Plus {
        a: String,
        b: String,
    },
    Hadamard {
        a: String,
        b: String,
    },
    Rmsnorm {
        x: String,
        residual: Option<String>,
        weight: Option<String>,
        heads: Option<usize>,
    },
    Rope {
        q: String,
        rc: String,
        rs: String,
        heads: Option<usize>,
    },
    Attention {
        q: String,
        k: String,
        v: String,
        heads: usize,
        groups: usize,
    },
    FullAttention {
        q: String,
        k: String,
        v: String,
        heads: usize,
    },
    Sigmoid {
        x: String,
    },
    Silu {
        x: String,
    },
    Gelu {
        x: String,
    },
    Softmax {
        x: String,
    },
    Transpose {
        x: String,
    },
    Layernorm {
        x: String,
        weight: String,
        bias: String,
    },
    CausalConv {
        x: String,
        weight: String,
        state: String,
        taps: usize,
    },
    L2norm {
        x: String,
        heads: Option<usize>,
    },
    Decay {
        ar: String,
        dt: String,
        alog: String,
    },
    GatedDelta {
        q: String,
        k: String,
        v: String,
        gate: String,
        beta: String,
        decay: String,
        state: String,
        norm: String,
        hv: usize,
        hk: usize,
    },
    Swiglu {
        gate: String,
        up: String,
    },
    Moe {
        x: String,
    },
    Argmax {
        x: String,
    },
    /// `Let V be row TOK of TABLE.` — one row of a table by a data index (the embedding lookup).
    RowOf {
        table: String,
        tok: String,
    },
    /// `Set row P of TABLE to ROW.` — a scatter into a table (the KV-cache append).
    SetRow {
        table: String,
        pos: String,
        row: String,
    },
}

impl OpKind {
    /// The op's name — the key of [`torch_spelling`].
    pub fn name(&self) -> &'static str {
        match self {
            OpKind::Matmul { .. } => "matmul",
            OpKind::Plus { .. } => "plus",
            OpKind::Hadamard { .. } => "hadamard",
            OpKind::Rmsnorm { .. } => "rmsnorm",
            OpKind::Rope { .. } => "rope",
            OpKind::Attention { .. } => "attention",
            OpKind::FullAttention { .. } => "full attention",
            OpKind::Sigmoid { .. } => "sigmoid",
            OpKind::Silu { .. } => "silu",
            OpKind::Gelu { .. } => "gelu",
            OpKind::Softmax { .. } => "softmax",
            OpKind::Transpose { .. } => "transpose",
            OpKind::Layernorm { .. } => "layernorm",
            OpKind::CausalConv { .. } => "causal conv",
            OpKind::L2norm { .. } => "l2norm",
            OpKind::Decay { .. } => "decay",
            OpKind::GatedDelta { .. } => "gated delta",
            OpKind::Swiglu { .. } => "swiglu",
            OpKind::Moe { .. } => "moe",
            OpKind::Argmax { .. } => "argmax",
            OpKind::RowOf { .. } => "embedding",
            OpKind::SetRow { .. } => "set row",
        }
    }

    /// The variables the op reads.
    pub fn inputs(&self) -> Vec<&str> {
        match self {
            OpKind::Matmul { a, b } | OpKind::Plus { a, b } | OpKind::Hadamard { a, b } => {
                vec![a, b]
            }
            OpKind::Rmsnorm {
                x,
                residual,
                weight,
                ..
            } => {
                let mut v = vec![x.as_str()];
                v.extend(residual.as_deref());
                v.extend(weight.as_deref());
                v
            }
            OpKind::Rope { q, rc, rs, .. } => vec![q, rc, rs],
            OpKind::Attention { q, k, v, .. } | OpKind::FullAttention { q, k, v, .. } => {
                vec![q, k, v]
            }
            OpKind::Sigmoid { x }
            | OpKind::Silu { x }
            | OpKind::Gelu { x }
            | OpKind::Softmax { x }
            | OpKind::Transpose { x }
            | OpKind::L2norm { x, .. }
            | OpKind::Moe { x }
            | OpKind::Argmax { x } => vec![x],
            OpKind::Layernorm { x, weight, bias } => vec![x, weight, bias],
            OpKind::CausalConv {
                x, weight, state, ..
            } => vec![x, weight, state],
            OpKind::Decay { ar, dt, alog } => vec![ar, dt, alog],
            OpKind::GatedDelta {
                q,
                k,
                v,
                gate,
                beta,
                decay,
                state,
                norm,
                ..
            } => vec![q, k, v, gate, beta, decay, state, norm],
            OpKind::Swiglu { gate, up } => vec![gate, up],
            OpKind::RowOf { table, tok } => vec![table, tok],
            OpKind::SetRow { table, pos, row } => vec![table, pos, row],
        }
    }

    /// The carried state the op writes back (a conv ring, a GDN memory, a KV table), if any.
    pub fn writes(&self) -> Option<&str> {
        match self {
            OpKind::CausalConv { state, .. } | OpKind::GatedDelta { state, .. } => Some(state),
            OpKind::SetRow { table, .. } => Some(table),
            _ => None,
        }
    }
}

// ── The sentence reader ──────────────────────────────────────────────────────────────────────────

fn num(w: &str, what: &str, line: &str) -> Result<usize, String> {
    w.parse::<usize>()
        .map_err(|_| format!("`{line}`: {what} must be an integer literal, got `{w}`"))
}

/// Read one sentence of tensor sugar. `None` when the line is not a sentence at all (a heading, a blank).
fn read_sentence(line: &str) -> Result<Option<TensorSentence>, String> {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') {
        return Ok(None);
    }
    let Some(body) = t.strip_suffix('.') else {
        return Err(format!("`{t}`: a sentence ends with a period"));
    };
    let w: Vec<&str> = body.split_whitespace().collect();
    let s = |i: usize| w.get(i).copied().unwrap_or("");
    let own = |i: usize| s(i).to_string();
    let unknown = || {
        Err(format!("`{t}`: not a tensor sentence the SambaFlow reader knows (a declaration, a fill, a tensor op, or a Show)"))
    };
    if s(0) == "Show" {
        return Ok(Some(TensorSentence::Show));
    }
    if s(0) == "Set" && s(1) == "item" && s(3) == "of" && s(5) == "to" && w.len() == 7 {
        return Ok(Some(TensorSentence::Fill {
            var: own(4),
            index: num(s(2), "the item index", t)?,
            value: own(6),
        }));
    }
    if s(0) == "Set" && s(1) == "row" && s(3) == "of" && s(5) == "to" && w.len() == 7 {
        return Ok(Some(TensorSentence::Op {
            out: own(4),
            kind: OpKind::SetRow {
                table: own(4),
                pos: own(2),
                row: own(6),
            },
        }));
    }
    if s(0) != "Let" || s(2) != "be" {
        return unknown();
    }
    let out = own(1);
    let rest = &w[3..];
    let r = |i: usize| rest.get(i).copied().unwrap_or("");
    let ro = |i: usize| r(i).to_string();
    let n = rest.len();
    // `a R by C tensor of T`
    if r(0) == "a" && r(2) == "by" && r(4) == "tensor" && r(5) == "of" && n == 7 {
        let real = match r(6) {
            "Real" => true,
            "Int" => false,
            other => return Err(format!("`{t}`: a tensor is of Int or Real, not `{other}`")),
        };
        return Ok(Some(TensorSentence::Decl {
            var: out,
            rows: num(r(1), "the row count", t)?,
            cols: num(r(3), "the column count", t)?,
            real,
        }));
    }
    // `A matmul B` / `A plus B` / `A hadamard B` — three words with the operator in the middle.
    if n == 3 && matches!(r(1), "matmul" | "plus" | "hadamard") {
        let (a, b) = (ro(0), ro(2));
        let kind = match r(1) {
            "matmul" => OpKind::Matmul { a, b },
            "plus" => OpKind::Plus { a, b },
            _ => OpKind::Hadamard { a, b },
        };
        return Ok(Some(TensorSentence::Op { out, kind }));
    }
    let heads_at = |i: usize| -> Result<Option<usize>, String> {
        if r(i) == "across" && r(i + 2) == "heads" {
            Ok(Some(num(r(i + 1), "the head count", t)?))
        } else if r(i).is_empty() {
            Ok(None)
        } else {
            Err(format!(
                "`{t}`: expected `across H heads` or the end of the sentence, got `{}`",
                r(i)
            ))
        }
    };
    let kind = match (r(0), r(1)) {
        ("rmsnorm", "of") => {
            let x = ro(2);
            let mut i = 3;
            let residual = if r(i) == "plus" {
                i += 2;
                Some(ro(i - 1))
            } else {
                None
            };
            let weight = if r(i) == "with" && r(i + 1) == "weight" {
                i += 3;
                Some(ro(i - 1))
            } else {
                None
            };
            let heads = heads_at(i)?;
            if residual.is_some() && weight.is_none() {
                return Err(format!("`{t}`: a fused-add rmsnorm carries a weight"));
            }
            OpKind::Rmsnorm {
                x,
                residual,
                weight,
                heads,
            }
        }
        ("rope", "of") => {
            if !(r(3) == "with" && r(4) == "cos" && r(6) == "and" && r(7) == "sin") {
                return Err(format!(
                    "`{t}`: `rope of Q with cos RC and sin RS [across H heads]`"
                ));
            }
            OpKind::Rope {
                q: ro(2),
                rc: ro(5),
                rs: ro(8),
                heads: heads_at(9)?,
            }
        }
        ("attention", "of") => {
            if !(r(3) == "over"
                && r(5) == "and"
                && r(7) == "with"
                && r(9) == "heads"
                && r(11) == "groups"
                && n == 12)
            {
                return Err(format!(
                    "`{t}`: `attention of Q over K and V with H heads KV groups`"
                ));
            }
            OpKind::Attention {
                q: ro(2),
                k: ro(4),
                v: ro(6),
                heads: num(r(8), "H", t)?,
                groups: num(r(10), "KV", t)?,
            }
        }
        ("full", "attention") => {
            if !(r(2) == "of"
                && r(4) == "over"
                && r(6) == "and"
                && r(8) == "with"
                && r(10) == "heads"
                && n == 11)
            {
                return Err(format!(
                    "`{t}`: `full attention of Q over K and V with H heads`"
                ));
            }
            OpKind::FullAttention {
                q: ro(3),
                k: ro(5),
                v: ro(7),
                heads: num(r(9), "H", t)?,
            }
        }
        ("sigmoid", "of") if n == 3 => OpKind::Sigmoid { x: ro(2) },
        ("silu", "of") if n == 3 => OpKind::Silu { x: ro(2) },
        ("gelu", "of") if n == 3 => OpKind::Gelu { x: ro(2) },
        ("softmax", "of") if n == 3 => OpKind::Softmax { x: ro(2) },
        ("transpose", "of") if n == 3 => OpKind::Transpose { x: ro(2) },
        ("argmax", "of") if n == 3 => OpKind::Argmax { x: ro(2) },
        ("row", _) if r(2) == "of" && n == 4 => OpKind::RowOf {
            table: ro(3),
            tok: ro(1),
        },
        ("layernorm", "of") => {
            if !(r(3) == "with" && r(4) == "weight" && r(6) == "and" && r(7) == "bias" && n == 9) {
                return Err(format!("`{t}`: `layernorm of X with weight W and bias B`"));
            }
            OpKind::Layernorm {
                x: ro(2),
                weight: ro(5),
                bias: ro(8),
            }
        }
        ("causal", "conv") => {
            if !(r(2) == "of"
                && r(4) == "with"
                && r(5) == "weight"
                && r(7) == "state"
                && r(9) == "over"
                && r(11) == "taps"
                && n == 12)
            {
                return Err(format!(
                    "`{t}`: `causal conv of X with weight WC state CS over T taps`"
                ));
            }
            OpKind::CausalConv {
                x: ro(3),
                weight: ro(6),
                state: ro(8),
                taps: num(r(10), "T", t)?,
            }
        }
        ("l2norm", "of") => OpKind::L2norm {
            x: ro(2),
            heads: heads_at(3)?,
        },
        ("decay", "of") => {
            if !(r(3) == "with" && r(4) == "bias" && r(6) == "rate" && n == 8) {
                return Err(format!("`{t}`: `decay of Ar with bias Dt rate Alog`"));
            }
            OpKind::Decay {
                ar: ro(2),
                dt: ro(5),
                alog: ro(7),
            }
        }
        ("gated", "delta") => {
            let shape = [
                "gated", "delta", "of", "Q", "over", "K", "and", "V", "with", "gate", "Z", "beta",
                "B", "decay", "A", "state", "S", "norm", "GN", "with", "HV", "heads", "HK",
                "groups",
            ];
            let fixed = |i: usize| {
                shape[i]
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_lowercase())
                    .unwrap_or(false)
            };
            if n != shape.len() || (0..n).any(|i| fixed(i) && r(i) != shape[i]) {
                return Err(format!("`{t}`: `gated delta of Q over K and V with gate Z beta B decay A state S norm GN with HV heads HK groups`"));
            }
            OpKind::GatedDelta {
                q: ro(3),
                k: ro(5),
                v: ro(7),
                gate: ro(10),
                beta: ro(12),
                decay: ro(14),
                state: ro(16),
                norm: ro(18),
                hv: num(r(20), "HV", t)?,
                hk: num(r(22), "HK", t)?,
            }
        }
        ("swiglu", "of") => {
            if !(r(3) == "and" && n == 5) {
                return Err(format!("`{t}`: `swiglu of Gate and Up`"));
            }
            OpKind::Swiglu {
                gate: ro(2),
                up: ro(4),
            }
        }
        ("moe", "of") => OpKind::Moe { x: ro(2) },
        _ => return unknown(),
    };
    Ok(Some(TensorSentence::Op { out, kind }))
}

/// Every sentence of a tensor program, in order. A sentence outside the tensor sugar is an error naming it.
pub fn read_tensor_sentences(source: &str) -> Result<Vec<TensorSentence>, String> {
    let mut out = Vec::new();
    for line in source.lines() {
        if let Some(s) = read_sentence(line)? {
            out.push(s);
        }
    }
    Ok(out)
}

// ── The spellings ────────────────────────────────────────────────────────────────────────────────

/// The torch expression an op is spelled with (its helper's call shape), or a refusal naming the op.
pub fn torch_spelling(op: &str) -> Result<&'static str, String> {
    Ok(match op {
        "matmul" => "a @ b",
        "plus" => "a + b",
        "hadamard" => "a * b",
        "rmsnorm" => "rmsnorm(x, w, nr, nc)",
        "rope" => "rope(q, rc, rs, heads, hd, half)",
        "attention" => "attention(q, k, v, heads, groups, hd)",
        "full attention" => "full_attention(q, k, v, heads, hd)",
        "sigmoid" => "torch.sigmoid(x)",
        "silu" => "F.silu(x)",
        "gelu" => "gelu_tanh(x)",
        "softmax" => "softmax_rows(x)",
        "transpose" => "x.transpose(0, 1)",
        "layernorm" => "layernorm(x, w, b)",
        "causal conv" => "causal_conv(x, wc, cs)",
        "l2norm" => "l2norm(x, nr, nc)",
        "decay" => "decay(ar, dt, alog)",
        "gated delta" => "gated_delta(q, k, v, z, b, a, s, gn, hv, hk, d)",
        "swiglu" => "swiglu(g, u)",
        "argmax" => "torch.argmax(x).reshape(1, 1).to(x.dtype)",
        "embedding" => "table[int(tok):int(tok) + 1]",
        "set row" => "table[int(pos)] = row.reshape(-1)",
        other => return Err(format!("`{other}` has no torch spelling in the SambaFlow emitter — the routed expert FFN (moe) and anything outside the tensor sugar are refused, never dropped")),
    })
}

/// The checkpoint tensor a LOGOS weight variable comes from, by the extractor's naming convention
/// (`examples/extract_qwen38.rs`): `Wq3` → `blk.3.attn_q.weight` on an attention layer, `Wq0` →
/// `blk.0.attn_qkv.weight` on a Gated DeltaNet layer. `None` for a name the convention does not cover.
pub fn gguf_name(var: &str, attention_layers: &BTreeSet<usize>) -> Option<String> {
    match var {
        "on" => return Some("output_norm.weight".into()),
        "Wout" => return Some("output.weight".into()),
        "emb" | "Emb" => return Some("token_embd.weight".into()),
        _ => {}
    }
    let digits: String = var
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if digits.is_empty() {
        return None;
    }
    let stem = &var[..var.len() - digits.len()];
    let l: usize = digits.parse().ok()?;
    let attn = attention_layers.contains(&l);
    let name = match (stem, attn) {
        ("an", _) => "attn_norm.weight",
        ("fn", _) => "post_attention_norm.weight",
        ("Qn", _) => "attn_q_norm.weight",
        ("Kn", _) => "attn_k_norm.weight",
        ("Wq", true) | ("Wqg", true) => "attn_q.weight",
        ("Wk", true) => "attn_k.weight",
        ("Wv", true) => "attn_v.weight",
        ("Wo", true) => "attn_output.weight",
        ("Wq", false) | ("Wk", false) | ("Wv", false) => "attn_qkv.weight",
        ("Wz", false) => "attn_gate.weight",
        ("Wo", false) => "ssm_out.weight",
        ("Wcq", _) | ("Wck", _) | ("Wcv", _) => "ssm_conv1d.weight",
        ("Wb", _) => "ssm_beta.weight",
        ("Wa", _) => "ssm_alpha.weight",
        ("Dt", _) => "ssm_dt.bias",
        ("Al", _) => "ssm_a",
        ("Gn", _) => "ssm_norm.weight",
        ("Wg", _) => "ffn_gate.weight",
        ("Wu", _) => "ffn_up.weight",
        ("Wd", _) => "ffn_down.weight",
        _ => return None,
    };
    Some(format!("blk.{l}.{name}"))
}

// ── Emission ─────────────────────────────────────────────────────────────────────────────────────

const PY_HELPERS: &str = r#"
EPS = 1e-6


def rmsnorm(x, w, nr, nc):
    """Per-row RMSNorm over an (nr, nc) view: y = x / sqrt(mean(x^2) + eps) * w."""
    v = x.reshape(nr, nc)
    y = v * torch.rsqrt((v * v).mean(-1, keepdim=True) + EPS) * w.reshape(1, nc)
    return y.reshape(x.shape)


def rope(q, rc, rs, heads, hd, half):
    """NEOX rotary on the leading 2*half dims of each head (pairs (i, i+half)); the rest passes through."""
    v = q.reshape(heads, hd)
    a, b = v[:, :half], v[:, half:2 * half]
    c, s = rc.reshape(1, half), rs.reshape(1, half)
    return torch.cat([a * c - b * s, a * s + b * c, v[:, 2 * half:]], dim=1).reshape(q.shape)


def attention(q, k, v, heads, groups, hd):
    """Single-query GQA decode attention: query head h reads kv head h // (heads // groups)."""
    L = k.shape[0]
    qh, kh, vh = q.reshape(heads, hd), k.reshape(L, groups, hd), v.reshape(L, groups, hd)
    out = []
    for h in range(heads):
        g = h // (heads // groups)
        scores = (kh[:, g, :] @ qh[h]) / math.sqrt(hd)
        out.append(torch.softmax(scores, dim=0) @ vh[:, g, :])
    return torch.stack(out).reshape(1, heads * hd)


def full_attention(q, k, v, heads, hd):
    """Bidirectional T x T self-attention per head, no mask."""
    T = q.shape[0]
    qh, kh, vh = q.reshape(T, heads, hd), k.reshape(T, heads, hd), v.reshape(T, heads, hd)
    out = []
    for h in range(heads):
        scores = (qh[:, h, :] @ kh[:, h, :].transpose(0, 1)) / math.sqrt(hd)
        out.append(torch.softmax(scores, dim=1) @ vh[:, h, :])
    return torch.stack(out, dim=1).reshape(T, heads * hd)


def gelu_tanh(x):
    """GELU, ggml's tanh approximation, in its one-exponential arrangement x / (1 + e^-2z)."""
    z = math.sqrt(2.0 / math.pi) * (x + 0.044715 * x * x * x)
    return x / (1.0 + torch.exp(-2.0 * z))


def softmax_rows(x):
    return torch.softmax(x, dim=1)


def layernorm(x, w, b):
    mean = x.mean(-1, keepdim=True)
    var = ((x - mean) * (x - mean)).mean(-1, keepdim=True)
    return (x - mean) / torch.sqrt(var + EPS) * w.reshape(1, -1) + b.reshape(1, -1)


def causal_conv(x, wc, cs):
    """Depthwise causal conv1d over the decode ring: shift the ring up, append this token, reduce over taps.
    Returns (y, new ring)."""
    ring = torch.cat([cs[1:], x.reshape(1, -1)], dim=0)
    return (ring * wc).sum(dim=0, keepdim=True), ring


def l2norm(x, nr, nc):
    v = x.reshape(nr, nc)
    y = v / torch.sqrt((v * v).sum(-1, keepdim=True) + EPS)
    return y.reshape(x.shape)


def decay(ar, dt, alog):
    """Gated DeltaNet's forget gate: (1 + e^(ar + dt)) ^ alog, as the LOGOS desugar computes it."""
    return torch.pow(1.0 + torch.exp(ar + dt), alog)


def gated_delta(q, k, v, z, b, a, s, gn, hv, hk, d):
    """One Gated DeltaNet step over hv value heads sharing hk key heads (head h reads key head h mod hk):
    u = S k;  S = alpha S + beta (v - alpha u) k^T;  o = (S q) / sqrt(d);  out = gn * rmsnorm(o) * silu(z).
    Returns (out, new memory)."""
    S = s.reshape(hv, d, d)
    idx = torch.tensor([h % hk for h in range(hv)])
    qh, kh = q.reshape(hk, d)[idx], k.reshape(hk, d)[idx]
    vh, zh = v.reshape(hv, d), z.reshape(hv, d)
    alpha, beta = a.reshape(hv, 1), b.reshape(hv, 1)
    u = torch.einsum('hab,hb->ha', S, kh)
    coef = beta * (vh - alpha * u)
    S = alpha.reshape(hv, 1, 1) * S + coef.reshape(hv, d, 1) * kh.reshape(hv, 1, d)
    o = torch.einsum('hab,hb->ha', S, qh) / math.sqrt(d)
    den = torch.sqrt((o * o).mean(-1, keepdim=True) + EPS)
    out = gn.reshape(1, d) * (o / den) * F.silu(zh)
    return out.reshape(1, hv * d), S.reshape(s.shape)


def swiglu(g, u):
    return F.silu(g) * u
"#;

struct Program {
    sentences: Vec<TensorSentence>,
    shapes: BTreeMap<String, (usize, usize)>,
    real: BTreeMap<String, bool>,
    /// Declared tensors no op produces, in declaration order.
    leaves: Vec<String>,
    /// Leaves an op writes back.
    states: BTreeSet<String>,
    /// The layers whose projections feed an `attention` op (the naming convention's layer index).
    attention_layers: BTreeSet<usize>,
    ops: Vec<(String, OpKind)>,
    output: String,
}

fn layer_of(var: &str) -> Option<usize> {
    let digits: String = var
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().ok()
}

fn read_program(source: &str) -> Result<Program, String> {
    let sentences = read_tensor_sentences(source)?;
    let mut shapes: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut real: BTreeMap<String, bool> = BTreeMap::new();
    let mut leaves = Vec::new();
    let mut produced: BTreeSet<String> = BTreeSet::new();
    let mut states = BTreeSet::new();
    let mut attention_layers = BTreeSet::new();
    let mut ops = Vec::new();
    let shape = |shapes: &BTreeMap<String, (usize, usize)>,
                 v: &str,
                 op: &str|
     -> Result<(usize, usize), String> {
        shapes.get(v).copied().ok_or_else(|| {
            format!("`{op}` reads `{v}`, which is not a declared or computed tensor")
        })
    };
    for s in &sentences {
        match s {
            TensorSentence::Decl {
                var,
                rows,
                cols,
                real: is_real,
            } => {
                shapes.insert(var.clone(), (*rows, *cols));
                real.insert(var.clone(), *is_real);
                leaves.push(var.clone());
            }
            TensorSentence::Fill { var, .. } => {
                if !shapes.contains_key(var) {
                    return Err(format!("`Set item … of {var}` fills an undeclared tensor"));
                }
            }
            TensorSentence::Show => {}
            TensorSentence::Op { out, kind } => {
                torch_spelling(kind.name())?;
                for input in kind.inputs() {
                    if !matches!(kind, OpKind::RowOf { .. } | OpKind::SetRow { .. })
                        || shapes.contains_key(input)
                    {
                        shape(&shapes, input, kind.name())?;
                    }
                }
                let out_shape = match kind {
                    OpKind::Matmul { a, b } => {
                        let ((m, ka), (kb, n)) =
                            (shape(&shapes, a, "matmul")?, shape(&shapes, b, "matmul")?);
                        if ka != kb {
                            return Err(format!(
                                "`{out} = {a} matmul {b}`: inner dimensions {ka} and {kb} disagree"
                            ));
                        }
                        (m, n)
                    }
                    OpKind::Plus { a, .. } | OpKind::Hadamard { a, .. } => {
                        shape(&shapes, a, kind.name())?
                    }
                    OpKind::Rmsnorm { x, .. }
                    | OpKind::Rope { q: x, .. }
                    | OpKind::Attention { q: x, .. }
                    | OpKind::FullAttention { q: x, .. }
                    | OpKind::Sigmoid { x }
                    | OpKind::Silu { x }
                    | OpKind::Gelu { x }
                    | OpKind::Softmax { x }
                    | OpKind::Layernorm { x, .. }
                    | OpKind::L2norm { x, .. }
                    | OpKind::Moe { x } => shape(&shapes, x, kind.name())?,
                    OpKind::Transpose { x } => {
                        let (r, c) = shape(&shapes, x, "transpose")?;
                        (c, r)
                    }
                    OpKind::CausalConv { x, .. } => shape(&shapes, x, "causal conv")?,
                    OpKind::Decay { ar, .. } => shape(&shapes, ar, "decay")?,
                    OpKind::GatedDelta { q, hv, hk, .. } => {
                        let (_, qc) = shape(&shapes, q, "gated delta")?;
                        (1, hv * (qc / hk.max(&1)))
                    }
                    OpKind::Swiglu { gate, .. } => shape(&shapes, gate, "swiglu")?,
                    OpKind::Argmax { .. } => (1, 1),
                    OpKind::RowOf { table, .. } => (1, shape(&shapes, table, "embedding")?.1),
                    OpKind::SetRow { table, .. } => shape(&shapes, table, "set row")?,
                };
                if let OpKind::Attention { q, .. } = kind {
                    if let Some(l) = layer_of(q) {
                        attention_layers.insert(l);
                    }
                }
                if let Some(w) = kind.writes() {
                    states.insert(w.to_string());
                }
                if !matches!(kind, OpKind::SetRow { .. }) {
                    let is_real = kind
                        .inputs()
                        .first()
                        .and_then(|i| real.get(*i).copied())
                        .unwrap_or(true);
                    shapes.insert(out.clone(), out_shape);
                    real.insert(out.clone(), is_real);
                    produced.insert(out.clone());
                }
                ops.push((out.clone(), kind.clone()));
            }
        }
    }
    let leaves: Vec<String> = leaves
        .into_iter()
        .filter(|l| !produced.contains(l))
        .collect();
    let output = ops
        .iter()
        .rev()
        .find(|(_, k)| !matches!(k, OpKind::SetRow { .. }))
        .map(|(o, _)| o.clone())
        .ok_or("a tensor program with no op has nothing to emit")?;
    Ok(Program {
        sentences,
        shapes,
        real,
        leaves,
        states,
        attention_layers,
        ops,
        output,
    })
}

/// The real parser must accept the program, and the one IR's lift of it must agree on the ops they both know — so the
/// sentence reader cannot drift from the language unnoticed.
fn cross_check(source: &str, p: &Program) -> Result<(), String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, _| parsed.map(|_| ()).map_err(|e| format!("the program does not parse: {e}")))?;
    let lifted = TensorGraph::lift_source(source, &|_| Storage::Values).map_err(|e| format!("the front end does not walk the program: {e:?}"))?;
    let count = |kind: &str| lifted.graph.ops.iter().filter(|op| op.kind() == kind).count();
    let counted = (count("Matmul"), count("Rope"), count("Attention"));
    let mine = |name: &str| p.ops.iter().filter(|(_, k)| k.name() == name).count();
    let mine_counts = (mine("matmul"), mine("rope"), mine("attention"));
    if counted != mine_counts {
        return Err(format!("the sentence reader and the one IR disagree on (matmul, rope, attention) counts: the IR {counted:?}, sentences {mine_counts:?}"));
    }
    Ok(())
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn py_call(p: &Program, kind: &OpKind) -> Result<String, String> {
    let sh = |v: &str| p.shapes.get(v).copied().unwrap_or((1, 1));
    Ok(match kind {
        OpKind::Matmul { a, b } => format!("{a} @ {b}"),
        OpKind::Plus { a, b } => format!("{a} + {b}"),
        OpKind::Hadamard { a, b } => format!("{a} * {b}"),
        OpKind::Rmsnorm {
            x,
            residual,
            weight,
            heads,
        } => {
            let (r, c) = sh(x);
            let (nr, nc) = match heads {
                Some(h) => (*h, (r * c) / h.max(&1)),
                None => (r, c),
            };
            let src = match residual {
                Some(res) => format!("({x} + {res})"),
                None => x.clone(),
            };
            match weight {
                Some(w) => format!("rmsnorm({src}, {w}, {nr}, {nc})"),
                None => format!("rmsnorm({src}, torch.ones({nc}, dtype={src}.dtype), {nr}, {nc})"),
            }
        }
        OpKind::Rope { q, rc, rs, heads } => {
            let (_, qc) = sh(q);
            let (_, half) = sh(rc);
            let hd = match heads {
                Some(h) => qc / h.max(&1),
                None => 2 * half,
            };
            let heads = if hd > 0 { qc / hd } else { 1 };
            format!("rope({q}, {rc}, {rs}, {heads}, {hd}, {half})")
        }
        OpKind::Attention {
            q,
            k,
            v,
            heads,
            groups,
        } => {
            let (_, qc) = sh(q);
            format!(
                "attention({q}, {k}, {v}, {heads}, {groups}, {})",
                qc / heads.max(&1)
            )
        }
        OpKind::FullAttention { q, k, v, heads } => {
            let (_, qc) = sh(q);
            format!(
                "full_attention({q}, {k}, {v}, {heads}, {})",
                qc / heads.max(&1)
            )
        }
        OpKind::Sigmoid { x } => format!("torch.sigmoid({x})"),
        OpKind::Silu { x } => format!("F.silu({x})"),
        OpKind::Gelu { x } => format!("gelu_tanh({x})"),
        OpKind::Softmax { x } => format!("softmax_rows({x})"),
        OpKind::Transpose { x } => format!("{x}.transpose(0, 1)"),
        OpKind::Layernorm { x, weight, bias } => format!("layernorm({x}, {weight}, {bias})"),
        OpKind::CausalConv {
            x, weight, state, ..
        } => format!("causal_conv({x}, {weight}, {state})"),
        OpKind::L2norm { x, heads } => {
            let (r, c) = sh(x);
            let (nr, nc) = match heads {
                Some(h) => (*h, (r * c) / h.max(&1)),
                None => (r, c),
            };
            format!("l2norm({x}, {nr}, {nc})")
        }
        OpKind::Decay { ar, dt, alog } => format!("decay({ar}, {dt}, {alog})"),
        OpKind::GatedDelta {
            q,
            k,
            v,
            gate,
            beta,
            decay,
            state,
            norm,
            hv,
            hk,
        } => {
            let (_, qc) = sh(q);
            format!("gated_delta({q}, {k}, {v}, {gate}, {beta}, {decay}, {state}, {norm}, {hv}, {hk}, {})", qc / hk.max(&1))
        }
        OpKind::Swiglu { gate, up } => format!("swiglu({gate}, {up})"),
        OpKind::Argmax { x } => format!("torch.argmax({x}).reshape(1, 1).to({x}.dtype)"),
        OpKind::RowOf { table, tok } => format!("{table}[int({tok}):int({tok}) + 1]"),
        OpKind::SetRow { .. } => String::new(),
        OpKind::Moe { .. } => return Err(torch_spelling("moe").unwrap_err()),
    })
}

/// The program as a PyTorch module, its weights manifest and a SambaFlow compile scaffold, or a refusal
/// naming what has no spelling.
pub fn emit_torch_module(source: &str) -> Result<TorchArtifact, String> {
    let p = read_program(source)?;
    cross_check(source, &p)?;
    let name = format!("logos_{:016x}", fnv(source));
    let leaf_role = |v: &str| -> &'static str {
        if p.states.contains(v) {
            "state"
        } else if gguf_name(v, &p.attention_layers).is_some() {
            "weight"
        } else {
            "input"
        }
    };

    // ── model.py ──
    let mut m = String::new();
    m.push_str(VENDOR_HEADER);
    m.push('\n');
    m.push_str(&format!(
        "# Generated by LOGOS (logicaffeine_compile::spatial::lower::sambaflow) from the tensor program `{name}`:\n# {} tensors, {} ops, output `{}`. The forward mirrors the program one op per statement, in dataflow order;\n# every helper spells the LOGOS desugar of its op. Weights are raw little-endian files named by variable\n# (`<var>.f64` or `<var>.f32`, row-major, shape from MANIFEST) — see `load_weights`.\n",
        p.leaves.len(),
        p.ops.len(),
        p.output
    ));
    m.push_str("import argparse\nimport json\nimport math\nimport os\nimport sys\n\nimport numpy as np\nimport torch\nimport torch.nn as nn\nimport torch.nn.functional as F\n");
    m.push_str(PY_HELPERS);
    m.push_str("\n\nLEAVES = {\n");
    for l in &p.leaves {
        let (r, c) = p.shapes[l];
        m.push_str(&format!(
            "    \"{l}\": (({r}, {c}), \"{}\", {}),\n",
            leaf_role(l),
            if p.real[l] { "\"real\"" } else { "\"int\"" }
        ));
    }
    m.push_str("}\n");
    m.push_str(&format!("OUTPUT = \"{}\"\n", p.output));
    m.push_str("\n\nclass LogosModule(nn.Module):\n");
    m.push_str("    \"\"\"The tensor program. Leaves present in `weights` become buffers; the rest are forward inputs.\"\"\"\n\n");
    m.push_str("    def __init__(self, weights):\n        super().__init__()\n        self.free = []\n        for name, (shape, role, kind) in LEAVES.items():\n            if name in weights:\n                self.register_buffer(name, weights[name].reshape(shape))\n            else:\n                self.free.append(name)\n\n");
    m.push_str("    def _write_state(self, name, value, inputs):\n        if name not in inputs:\n            with torch.no_grad():\n                getattr(self, name).copy_(value)\n\n");
    m.push_str("    def forward(self, **inputs):\n");
    m.push_str("        for name in self.free:\n            if name not in inputs:\n                raise KeyError(f\"forward needs `{name}` (not among the weights)\")\n");
    for l in &p.leaves {
        m.push_str(&format!(
            "        {l} = inputs[\"{l}\"] if \"{l}\" in inputs else self.{l}\n"
        ));
    }
    for (i, (out, kind)) in p.ops.iter().enumerate() {
        m.push_str(&format!("        # stage {i}: {out} = {}\n", kind.name()));
        let call = py_call(&p, kind)?;
        match kind {
            OpKind::CausalConv { state, .. } | OpKind::GatedDelta { state, .. } => {
                m.push_str(&format!("        {out}, {state} = {call}\n"));
                m.push_str(&format!(
                    "        self._write_state(\"{state}\", {state}, inputs)\n"
                ));
            }
            OpKind::SetRow { table, pos, row } => {
                m.push_str(&format!("        {table} = {table}.clone()\n        {table}[int({pos})] = {row}.reshape(-1)\n"));
                m.push_str(&format!(
                    "        self._write_state(\"{table}\", {table}, inputs)\n"
                ));
            }
            _ => m.push_str(&format!("        {out} = {call}\n")),
        }
    }
    m.push_str(&format!("        return {}\n", p.output));
    m.push_str("\n\ndef load_weights(directory, dtype):\n    \"\"\"Every leaf with a `<var>.f64` / `<var>.f32` file in `directory`, as tensors of `dtype`.\"\"\"\n    weights = {}\n    for name, (shape, role, kind) in LEAVES.items():\n        for ext, np_dtype in ((\".f64\", np.float64), (\".f32\", np.float32)):\n            path = os.path.join(directory, name + ext)\n            if os.path.exists(path):\n                arr = np.fromfile(path, dtype=np_dtype).reshape(shape)\n                weights[name] = torch.from_numpy(arr.copy()).to(dtype)\n                break\n    return weights\n");
    m.push_str("\n\ndef main():\n    ap = argparse.ArgumentParser(description=\"run the LOGOS tensor program in torch\")\n    ap.add_argument(\"--weights\", required=True, help=\"directory of <var>.f64/<var>.f32 files\")\n    ap.add_argument(\"--dtype\", default=\"float32\", choices=[\"float32\", \"float64\"])\n    ap.add_argument(\"--out\", default=None, help=\"write the output tensor as raw little-endian values of --dtype\")\n    args = ap.parse_args()\n    dtype = torch.float64 if args.dtype == \"float64\" else torch.float32\n    torch.set_default_dtype(dtype)\n    model = LogosModule(load_weights(args.weights, dtype))\n    if model.free:\n        raise SystemExit(f\"missing inputs: {model.free}\")\n    with torch.no_grad():\n        out = model()\n    flat = out.detach().to(dtype).reshape(-1).cpu().numpy()\n    if args.out:\n        flat.astype(np.float64 if args.dtype == \"float64\" else np.float32).tofile(args.out)\n    print(json.dumps({\"output\": OUTPUT, \"shape\": list(out.shape), \"first\": float(flat[0]), \"last\": float(flat[-1])}))\n\n\nif __name__ == \"__main__\":\n    main()\n");

    // ── manifest.json ──
    let tensors: Vec<serde_json::Value> = p
        .sentences
        .iter()
        .filter_map(|s| match s {
            TensorSentence::Decl {
                var,
                rows,
                cols,
                real,
            } => Some(serde_json::json!({
                "var": var,
                "shape": [rows, cols],
                "dtype": if *real { "Real" } else { "Int" },
                "role": if p.leaves.contains(var) { leaf_role(var) } else { "computed" },
                "gguf": gguf_name(var, &p.attention_layers),
                "file": format!("{var}.f64"),
            })),
            _ => None,
        })
        .collect();
    let ops: Vec<serde_json::Value> = p
        .ops
        .iter()
        .enumerate()
        .map(|(i, (out, kind))| serde_json::json!({ "index": i, "out": out, "op": kind.name(), "inputs": kind.inputs(), "writes": kind.writes(), "torch": torch_spelling(kind.name()).unwrap_or("") }))
        .collect();
    let manifest = serde_json::json!({
        "schema": "logos.sambaflow.manifest",
        "version": 1,
        "provenance": "vendor-ingestion artifact emitted by LOGOS — not our compute path",
        "program": name,
        "inputs": p.leaves.iter().filter(|l| leaf_role(l) == "input").collect::<Vec<_>>(),
        "states": p.leaves.iter().filter(|l| leaf_role(l) == "state").collect::<Vec<_>>(),
        "output": p.output,
        "tensors": tensors,
        "ops": ops,
    });
    let manifest_json = serde_json::to_string_pretty(&manifest).expect("the manifest serializes");

    // ── compile.py ──
    let mut c = String::new();
    c.push_str(VENDOR_HEADER);
    c.push('\n');
    c.push_str(&format!("{TEMPLATE_MARK}\n"));
    c.push_str(&format!(
        "# Compile `model.py` (the LOGOS tensor program `{name}`) for the RDU through SambaFlow, then run it.\n# Every call into the vendor SDK below is a TEMPLATE: the documented shape of the SambaFlow front end as\n# last publicly described, to be replaced by the actual entry points the day the SDK is in hand.\nimport argparse\nimport json\n\nimport torch\n\nfrom model import LogosModule, load_weights, LEAVES, OUTPUT\n\n\ndef main():\n    ap = argparse.ArgumentParser(description=\"SambaFlow compile/run scaffold for a LOGOS tensor program\")\n    ap.add_argument(\"command\", choices=[\"compile\", \"run\"])\n    ap.add_argument(\"--weights\", required=True)\n    ap.add_argument(\"--pef\", default=\"{name}.pef\")\n    args = ap.parse_args()\n    weights = load_weights(args.weights, torch.float32)\n    model = LogosModule(weights)\n    inputs = {{name: torch.zeros(LEAVES[name][0]) for name in model.free}}\n    {TEMPLATE_MARK}\n    from sambaflow import samba  # noqa: E402\n    {TEMPLATE_MARK}\n    samba.from_torch_model_(model)\n    if args.command == \"compile\":\n        {TEMPLATE_MARK}\n        samba.session.compile(model, inputs=list(inputs.values()), pef_name=args.pef)\n        print(json.dumps({{\"pef\": args.pef, \"output\": OUTPUT}}))\n    else:\n        {TEMPLATE_MARK}\n        out = samba.session.run(input_tensors=list(inputs.values()), output_tensors=[model(**inputs)], pef=args.pef)\n        print(json.dumps({{\"output\": OUTPUT, \"shape\": list(out[0].shape)}}))\n\n\nif __name__ == \"__main__\":\n    main()\n"
    ));

    Ok(TorchArtifact {
        model_py: m,
        compile_py: c,
        manifest_json,
    })
}
