//! Which checkpoint tensor fills each weight an authored program declares, and how.
//!
//! The author names a layer's weights the same way for every model (`an3`, `Wq3`, `Qn3`, `Wo3`, `fn3`, `Wg3` …), in
//! the layout the language reads them: a projection `Y = X matmul W` holds `W` as `in × out`, row-major. A GGUF
//! checkpoint holds the same projection as `out` rows of `in` weights. The binding names the tensor and the view
//! that turns one into the other:
//!
//!   • a projection is transposed;
//!   • a stacked expert tensor (`ffn_gate_exps` `[d, dff, E]`) is already the `(E·dff) × d` stack the `moe` sugar
//!     reads, and the embedding table already `vocab × d`;
//!   • a checkpoint whose rotation pairs neighbouring dimensions (`2i`, `2i+1` — Llama) has each head's query and key
//!     rows (and their norm weights) permuted so the pairs fall `dims/2` apart, the pairing the `rope` sugar turns.
//!     Attention scores are unchanged: the same orthogonal permutation multiplies every query and every key.
//!
//!   • a projection that holds several outputs per head (Qwen3.5's gated attention: each head's queries, then its
//!     gate) is read a part of every head at a time, and one that stacks several projections (Gated DeltaNet's
//!     `attn_qkv`, its convolution kernel) a range of rows at a time.
//!
//! The rotary tables (`RC`/`RS`, `Rcos`/`Rsin`) are not weights: [`rope_tables`] computes them from the spec. The
//! carried state — a KV cache, a convolution ring, a delta-rule memory — is not read from the checkpoint at all: it
//! starts at zero ([`Source::State`]).
//!
//! A weight a spatial program reads as it is stored Q8_0 is two tensors, its int8 values and its block scales
//! ([`q8_planes`]), repacked from the checkpoint's blocks through the same view. A rotary table's values are
//! [`rotary_values`].

use super::quant::{f16_to_f32, GgmlType};
use super::reference::WeightSource;
use super::spec::{Attention, Ffn, GatedDelta, Gating, Mixer, Mla, ModelSpec, Rope, RopeScaling};

/// How a declared weight is read out of its tensor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    /// The tensor's elements in order.
    Same,
    /// The tensor's `out × in` rows as an `in × out` matrix.
    Transposed,
    /// Within every head of `head_dim` rows (or elements), the `dims` rotated ones from `from` reordered from
    /// interleaved pairs to halves; then transposed when `transpose`.
    NeoxPairs {
        head_dim: usize,
        from: usize,
        dims: usize,
        transpose: bool,
    },
    /// Rows `first .. first + count`, then transposed when `transpose`.
    RowRange {
        first: usize,
        count: usize,
        transpose: bool,
    },
    /// In every group of `stride` rows, the `width` rows starting at `offset` — one part of every head — then
    /// transposed when `transpose`.
    HeadPart {
        stride: usize,
        offset: usize,
        width: usize,
        transpose: bool,
    },
    /// In every group of `stride` rows, the `width` rows starting at `offset` — one head's block — each block
    /// transposed on its own and the transposes stacked: the weight of a per-head matmul whose checkpoint rows are
    /// each head's outputs.
    HeadBlocks {
        stride: usize,
        offset: usize,
        width: usize,
    },
    /// One part of a fused query-key-value projection: rows `first .. first + count` of its weight, read transposed, or
    /// elements `first .. first + count` of its bias (`bias`); within the part, each head of `head_dim` with its first
    /// `dims` dimensions reordered from interleaved pairs to halves (none when `dims` is 0).
    FusedPart {
        first: usize,
        count: usize,
        head_dim: usize,
        dims: usize,
        bias: bool,
    },
    /// A dilated convolution's kernel — `rows` channels of `inner` taps — as the kernel of the ordinary causal convolution
    /// over every position its taps span: `(inner − 1)·by + 1` rows by the channels, tap `k` in row `k·by` and the rows
    /// between zero.
    Dilated {
        by: usize,
    },
}

/// One declared weight's source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub tensor: String,
    pub view: View,
}

/// Where a declared name's values come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A checkpoint tensor, through a view.
    Weight(Binding),
    /// Carried state, zero before the first position.
    State,
}

/// Why a declared name has no source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unbound(pub String);

/// The base and layer of an authored weight name: `Wq12` is `("Wq", Some(12))`, `Emb` is `("Emb", None)`.
fn split(var: &str) -> (&str, Option<usize>) {
    let digits = var.len() - var.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return (var, None);
    }
    let (base, l) = var.split_at(var.len() - digits);
    (base, l.parse().ok())
}

/// Where `var` is read from, for a model of `spec`.
pub fn bind(spec: &ModelSpec, var: &str) -> Result<Source, Unbound> {
    let unbound = || Unbound(var.to_string());
    let t = |name: String, view: View| Ok(Source::Weight(Binding { tensor: name, view }));
    let (base, layer) = split(var);
    let Some(l) = layer else {
        return match base {
            "Emb" => t("token_embd.weight".into(), View::Same),
            "on" => t("output_norm.weight".into(), View::Same),
            "Wout" if spec.tied_output => t("token_embd.weight".into(), View::Transposed),
            "Wout" => t("output.weight".into(), View::Transposed),
            // The last mix of a hyper-connected residual's lanes, in place of the output norm.
            "HOn" if spec.hyper.is_some() => t("output_hc_norm.weight".into(), View::Same),
            "HOd" if spec.hyper.is_some() => t("output_hc_down.weight".into(), View::Transposed),
            "HOu" if spec.hyper.is_some() => t("output_hc_up.weight".into(), View::Transposed),
            _ => Err(unbound()),
        };
    };
    let layer = spec.layers.get(l).ok_or_else(unbound)?;
    let blk = |suffix: &str| format!("blk.{l}.{suffix}.weight");
    // The per-layer n-gram embedding on its layer: the table, the key's and the value's projections, the key's, the
    // query's and the convolution's norms, the dilated convolution, and the carried token history and ring.
    if let Some(ple) = spec.ple.as_ref().filter(|p| p.layer == l) {
        match base {
            "Pe" => {
                let count = super::author::ple_table_rows(ple);
                return t("per_layer_token_embd.weight".into(), View::RowRange { first: 0, count, transpose: false });
            }
            "Wpk" => return t(blk("ple_key"), View::Transposed),
            "Wpv" => return t(blk("ple_value"), View::Transposed),
            "Pnk" => return t(blk("ple_norm_key"), View::Same),
            "Pnq" => return t(blk("ple_norm_query"), View::Same),
            "Pnc" => return t(blk("ple_norm_conv"), View::Same),
            "Wpc" => return t(blk("ple_conv1d"), View::Dilated { by: ple.ngram }),
            "Ph" | "Cp" => return Ok(Source::State),
            _ => {}
        }
    }
    // A hyper-connected residual's mixes before the mixer (`HA`) and the FFN (`HF`): the lanes' norm, the gate's down
    // and up projections, and the injection's.
    if spec.hyper.is_some() && (base.starts_with("HA") || base.starts_with("HF")) {
        let which = if base.starts_with("HA") { "hc_attn" } else { "hc_ffn" };
        return match &base[2..] {
            "n" => t(blk(&format!("{which}_norm")), View::Same),
            "d" => t(blk(&format!("{which}_down")), View::Transposed),
            "u" => t(blk(&format!("{which}_up")), View::Transposed),
            "j" => t(blk(&format!("{which}_inject")), View::Transposed),
            _ => Err(unbound()),
        };
    }
    match base {
        "an" => return t(blk("attn_norm"), View::Same),
        "fn" => return t(blk(spec.ffn_norm), View::Same),
        "pan" if layer.post_mixer_norm => return t(blk("post_attention_norm"), View::Same),
        "pfn" if layer.post_ffn_norm => return t(blk("post_ffw_norm"), View::Same),
        _ => {}
    }
    let mixer = match &layer.mixer {
        Mixer::Attention(a) => attention(a, base, l, &blk),
        Mixer::GatedDelta(g) => gated_delta(g, base, l, &blk),
        Mixer::Mla(m) => mla(m, base, &blk),
        Mixer::Recurrent(_) | Mixer::None => None,
    };
    if let Some(found) = mixer {
        return Ok(found);
    }
    match (base, &layer.ffn) {
        ("Wg", Ffn::Dense { .. }) => t(blk("ffn_gate"), View::Transposed),
        ("Wu", Ffn::Dense { .. }) => t(blk("ffn_up"), View::Transposed),
        ("Wd", Ffn::Dense { .. }) => t(blk("ffn_down"), View::Transposed),
        ("Wr", Ffn::Moe(_)) => t(blk("ffn_gate_inp"), View::Transposed),
        ("Wg", Ffn::Moe(_)) => t(blk("ffn_gate_exps"), View::Same),
        ("Wu", Ffn::Moe(_)) => t(blk("ffn_up_exps"), View::Same),
        ("Wd", Ffn::Moe(_)) => t(blk("ffn_down_exps"), View::Same),
        ("Wsg", Ffn::Moe(m)) if m.shared_experts > 0 => t(blk("ffn_gate_shexp"), View::Transposed),
        ("Wsu", Ffn::Moe(m)) if m.shared_experts > 0 => t(blk("ffn_up_shexp"), View::Transposed),
        ("Wsd", Ffn::Moe(m)) if m.shared_experts > 0 => t(blk("ffn_down_shexp"), View::Transposed),
        // The shared experts' gate: one output row over the input, read as the column the program declares.
        ("Wsgi", Ffn::Moe(m)) if m.shared_gate => t(blk("ffn_gate_inp_shexp"), View::Transposed),
        ("Br", Ffn::Moe(m)) if m.biases => t(format!("blk.{l}.ffn_gate_inp.bias"), View::Same),
        ("Bg", Ffn::Moe(m)) if m.biases => t(format!("blk.{l}.ffn_gate_exps.bias"), View::Same),
        ("Bu", Ffn::Moe(m)) if m.biases => t(format!("blk.{l}.ffn_up_exps.bias"), View::Same),
        ("Bd", Ffn::Moe(m)) if m.biases => t(format!("blk.{l}.ffn_down_exps.bias"), View::Same),
        ("Bs", Ffn::Moe(m)) if m.gating == (Gating::Sigmoid { bias: true }) => t(format!("blk.{l}.exp_probs_b.bias"), View::Same),
        _ => Err(unbound()),
    }
}

fn weight(tensor: String, view: View) -> Option<Source> {
    Some(Source::Weight(Binding { tensor, view }))
}

/// An attention block's names: its projections and their biases, QK-norm weights, sinks and KV cache.
fn attention(a: &Attention, base: &str, l: usize, blk: &dyn Fn(&str) -> String) -> Option<Source> {
    let interleaved = a.rope.as_ref().filter(|r| !r.neox).map(|r| r.dims);
    let hd = a.head_dim;
    // A per-head vector of an interleaved checkpoint reaches the program in the halves basis its projections do.
    let per_head = || match interleaved {
        Some(dims) => View::NeoxPairs { head_dim: hd, from: 0, dims, transpose: false },
        None => View::Same,
    };
    match base {
        // A fused projection holds each output row's queries, then its keys, then its values; its bias likewise.
        "Wq" | "Wk" | "Wv" | "Bq" | "Bk" | "Bv" if a.fused_qkv && (a.bias || base.starts_with('W')) => {
            let (aq, akv, vd) = (a.heads * hd, a.kv_heads * hd, a.value_dim);
            let (first, count, head_dim, dims) = match &base[1..] {
                "q" => (0, aq, hd, interleaved.unwrap_or(0)),
                "k" => (aq, akv, hd, interleaved.unwrap_or(0)),
                _ => (aq + akv, a.kv_heads * vd, vd, 0),
            };
            let bias = base.starts_with('B');
            let tensor = if bias { format!("blk.{l}.attn_qkv.bias") } else { blk("attn_qkv") };
            weight(tensor, View::FusedPart { first, count, head_dim, dims, bias })
        }
        "Bq" | "Bk" if a.bias => weight(format!("blk.{l}.{}.bias", if base == "Bq" { "attn_q" } else { "attn_k" }), per_head()),
        "Bv" if a.bias => weight(format!("blk.{l}.attn_v.bias"), View::Same),
        "Bo" if a.output_bias => weight(format!("blk.{l}.attn_output.bias"), View::Same),
        "Sk" if a.sinks => weight(blk("attn_sinks"), View::Same),
        "Wq" if a.output_gate => {
            weight(blk("attn_q"), View::HeadPart { stride: 2 * hd, offset: 0, width: hd, transpose: true })
        }
        "Wqg" if a.output_gate => {
            weight(blk("attn_q"), View::HeadPart { stride: 2 * hd, offset: hd, width: hd, transpose: true })
        }
        "Wq" | "Wk" => {
            let view = match interleaved {
                Some(dims) => View::NeoxPairs { head_dim: hd, from: 0, dims, transpose: true },
                None => View::Transposed,
            };
            weight(blk(if base == "Wq" { "attn_q" } else { "attn_k" }), view)
        }
        "Qn" | "Kn" => weight(blk(if base == "Qn" { "attn_q_norm" } else { "attn_k_norm" }), per_head()),
        "Wv" => weight(blk("attn_v"), View::Transposed),
        "Wo" => weight(blk("attn_output"), View::Transposed),
        "Kc" | "Vc" => Some(Source::State),
        _ => None,
    }
}

/// A latent attention block's names: the query projection (or its two halves and their norm), the latent's projection
/// and norm, the expansion's key and value halves as per-head matmul weights, the output projection and the cache.
///
///   • `Wq` (or `Wqa`, `Qan`, `Wqb`) each head's `qk_nope` unrotated then `qk_rope` rotated query dimensions, and `Wkva`
///     the latent's `kv_lora_rank` then the shared rotated key's `qk_rope` — an interleaved rotation's dimensions
///     reordered to halves where they sit, as an attention's are;
///   • `Wkb` (`(heads·qk_nope) × kv_lora_rank`) absorbs each head's unrotated query into the latent: the key half of
///     `attn_kv_b` read as it is stored (its rows are the absorption's inputs), or `attn_k_b`'s blocks transposed;
///   • `Wvb` (`(heads·kv_lora_rank) × v_dim`) expands each head's attended latent into its values: the value half of
///     `attn_kv_b`, or `attn_v_b`, each head's block transposed.
fn mla(m: &Mla, base: &str, blk: &dyn Fn(&str) -> String) -> Option<Source> {
    let (nope, rope, lora, vd) = (m.qk_nope, m.qk_rope, m.kv_lora_rank, m.v_dim);
    let rotated = |head_dim: usize, from: usize| match &m.rope {
        Some(r) if !r.neox => View::NeoxPairs { head_dim, from, dims: rope, transpose: true },
        _ => View::Transposed,
    };
    match base {
        "Wq" if m.q_lora_rank.is_none() => weight(blk("attn_q"), rotated(nope + rope, nope)),
        "Wqa" if m.q_lora_rank.is_some() => weight(blk("attn_q_a"), View::Transposed),
        "Qan" if m.q_lora_rank.is_some() => weight(blk("attn_q_a_norm"), View::Same),
        "Wqb" if m.q_lora_rank.is_some() => weight(blk("attn_q_b"), rotated(nope + rope, nope)),
        "Wkva" => weight(blk("attn_kv_a_mqa"), rotated(lora + rope, lora)),
        "Kan" => weight(blk("attn_kv_a_norm"), View::Same),
        "Wkb" if m.kv_b_split => weight(blk("attn_k_b"), View::HeadBlocks { stride: lora, offset: 0, width: lora }),
        "Wkb" => weight(blk("attn_kv_b"), View::HeadPart { stride: nope + vd, offset: 0, width: nope, transpose: false }),
        "Wvb" if m.kv_b_split => weight(blk("attn_v_b"), View::HeadBlocks { stride: vd, offset: 0, width: vd }),
        "Wvb" => weight(blk("attn_kv_b"), View::HeadBlocks { stride: nope + vd, offset: nope, width: vd }),
        "Wo" => weight(blk("attn_output"), View::Transposed),
        "Kc" => Some(Source::State),
        _ => None,
    }
}

/// A Gated DeltaNet block's names: the `q|k|v` projection and convolution split by rows, the gate, the decay and
/// write-strength projections, and the carried convolution rings and memory.
fn gated_delta(g: &GatedDelta, base: &str, l: usize, blk: &dyn Fn(&str) -> String) -> Option<Source> {
    let (qk, vw) = (g.k_heads * g.head_dim, g.v_heads * g.head_dim);
    let rows = |first, count| View::RowRange { first, count, transpose: true };
    match base {
        "Wq" => weight(blk("attn_qkv"), rows(0, qk)),
        "Wk" => weight(blk("attn_qkv"), rows(qk, qk)),
        "Wv" => weight(blk("attn_qkv"), rows(2 * qk, vw)),
        "Wz" => weight(blk("attn_gate"), View::Transposed),
        "Wcq" => weight(blk("ssm_conv1d"), rows(0, qk)),
        "Wck" => weight(blk("ssm_conv1d"), rows(qk, qk)),
        "Wcv" => weight(blk("ssm_conv1d"), rows(2 * qk, vw)),
        "Wb" => weight(blk("ssm_beta"), View::Transposed),
        "Wa" => weight(blk("ssm_alpha"), View::Transposed),
        "Dt" => weight(format!("blk.{l}.ssm_dt.bias"), View::Same),
        "Al" => weight(format!("blk.{l}.ssm_a"), View::Same),
        "Gn" => weight(blk("ssm_norm"), View::Same),
        "Wo" => weight(blk("ssm_out"), View::Transposed),
        "Csq" | "Csk" | "Csv" | "St" => Some(Source::State),
        _ => None,
    }
}

/// The source position of element `i` of a head's rotated block under [`View::NeoxPairs`]: halves `[0, dims/2)` and
/// `[dims/2, dims)` take the even and the odd members of the interleaved pairs; dimensions past `dims` stay put.
pub(crate) fn neox_source(i: usize, dims: usize) -> usize {
    let half = dims / 2;
    match i {
        i if i < half => 2 * i,
        i if i < dims => 2 * (i - half) + 1,
        i => i,
    }
}

/// [`neox_source`] for a head whose rotated block starts at dimension `from`: the dimensions before it stay put.
pub(crate) fn neox_source_from(i: usize, from: usize, dims: usize) -> usize {
    if i < from {
        i
    } else {
        from + neox_source(i - from, dims)
    }
}

/// `v` (whole heads of `head_dim`) with each head's rotated `dims` reordered from interleaved pairs to halves — the
/// basis a [`View::NeoxPairs`]-bound program's queries and keys live in.
pub fn neox_order(v: &[f64], head_dim: usize, dims: usize) -> Vec<f64> {
    (0..v.len())
        .map(|i| v[(i / head_dim) * head_dim + neox_source(i % head_dim, dims)])
        .collect()
}

/// A declared weight's values, in the layout the program declares it (`rows × cols` row-major, [`View::declared`]).
pub fn values(b: &Binding, source: &mut dyn WeightSource) -> std::io::Result<Vec<f64>> {
    let (ty, inner, rows, raw) = stored(b, source)?;
    let mut flat = Vec::with_capacity(inner * rows);
    ty.dequantize(&raw, &mut flat);
    Ok(b.view.apply(flat.into_iter().map(f64::from).collect(), inner, rows))
}

/// The tensor `b` binds as it is stored: its format, its row length, its row count and its bytes.
fn stored(b: &Binding, source: &mut dyn WeightSource) -> std::io::Result<(GgmlType, usize, usize, Vec<u8>)> {
    let (ty, dims) = source.describe(&b.tensor)?;
    let inner = dims.first().copied().unwrap_or(1) as usize;
    let rows = dims.iter().product::<u64>() as usize / inner.max(1);
    let raw = source.row_bytes(&b.tensor, 0, rows)?;
    Ok((ty, inner, rows, raw))
}

impl View {
    /// The `rows × cols` layout the view reads a tensor of `rows` rows of `inner` elements into: the layout the program
    /// declares the weight.
    pub fn declared(&self, inner: usize, rows: usize) -> (usize, usize) {
        let laid = |n: usize, transpose: bool| if transpose { (inner, n) } else { (n, inner) };
        match *self {
            View::Same => (rows, inner),
            View::Transposed => (inner, rows),
            View::NeoxPairs { transpose, .. } => laid(rows, transpose),
            View::RowRange { count, transpose, .. } => laid(count, transpose),
            View::HeadPart { stride, offset, width, transpose } => {
                laid((0..rows).filter(|r| (offset..offset + width).contains(&(r % stride))).count(), transpose)
            }
            View::HeadBlocks { stride, width, .. } => (rows / stride.max(1) * inner, width),
            View::FusedPart { count, bias: true, .. } => (1, count),
            View::FusedPart { count, bias: false, .. } => (inner, count),
            View::Dilated { by } => ((inner.max(1) - 1) * by + 1, rows),
        }
    }

    /// Whether the view lays each checkpoint row down a declared column, so a Q8_0 tensor's blocks run down the declared
    /// rows: the layout a spatial program reads a Q8_0 weight in ([`q8_planes`]).
    pub fn keeps_blocks_down_rows(&self) -> bool {
        match *self {
            View::Same | View::Dilated { .. } => false,
            View::Transposed | View::HeadBlocks { .. } => true,
            View::NeoxPairs { transpose, .. } | View::RowRange { transpose, .. } | View::HeadPart { transpose, .. } => transpose,
            View::FusedPart { bias, .. } => !bias,
        }
    }

    /// `flat`, a tensor of `rows` rows of `inner` elements, in the layout the view reads it into.
    fn apply(&self, flat: Vec<f64>, inner: usize, rows: usize) -> Vec<f64> {
        let transpose = |m: &[f64]| -> Vec<f64> {
            (0..inner)
                .flat_map(|c| (0..rows).map(move |r| (r, c)))
                .map(|(r, c)| m[r * inner + c])
                .collect()
        };
        match self {
            View::Same => flat,
            View::Transposed => transpose(&flat),
            View::NeoxPairs {
                head_dim,
                from,
                dims,
                transpose: t,
            } => {
                let source = |i: usize| (i / head_dim) * head_dim + neox_source_from(i % head_dim, *from, *dims);
                let permuted: Vec<f64> = if *t {
                    // Rows are output channels: row `h·head_dim + i` takes the source row `h·head_dim + neox_source(i)`.
                    (0..rows).flat_map(|r| flat[source(r) * inner..(source(r) + 1) * inner].to_vec()).collect()
                } else {
                    (0..flat.len()).map(|i| flat[source(i)]).collect()
                };
                if *t {
                    transpose(&permuted)
                } else {
                    permuted
                }
            }
            &View::RowRange { first, count, transpose: t } => {
                let picked = flat[first * inner..(first + count) * inner].to_vec();
                if t {
                    transpose_rows(&picked, count, inner)
                } else {
                    picked
                }
            }
            &View::HeadPart { stride, offset, width, transpose: t } => {
                let picked: Vec<f64> = (0..rows)
                    .filter(|r| (offset..offset + width).contains(&(r % stride)))
                    .flat_map(|r| flat[r * inner..(r + 1) * inner].to_vec())
                    .collect();
                let n = picked.len() / inner.max(1);
                if t {
                    transpose_rows(&picked, n, inner)
                } else {
                    picked
                }
            }
            &View::HeadBlocks { stride, offset, width } => (0..rows / stride.max(1))
                .flat_map(|h| {
                    let first = h * stride + offset;
                    transpose_rows(&flat[first * inner..(first + width) * inner], width, inner)
                })
                .collect(),
            &View::FusedPart { first, count, head_dim, dims, bias } => {
                let source = |i: usize| first + (i / head_dim) * head_dim + neox_source(i % head_dim, dims);
                if bias {
                    (0..count).map(|i| flat[source(i)]).collect()
                } else {
                    let picked: Vec<f64> = (0..count).flat_map(|r| flat[source(r) * inner..(source(r) + 1) * inner].to_vec()).collect();
                    transpose_rows(&picked, count, inner)
                }
            }
            &View::Dilated { by } => {
                let mut kernel = vec![0.0; ((inner.max(1) - 1) * by + 1) * rows];
                for c in 0..rows {
                    for k in 0..inner {
                        kernel[k * by * rows + c] = flat[c * inner + k];
                    }
                }
                kernel
            }
        }
    }
}

/// Whether the weight `b` binds is stored as the two planes [`q8_planes`] repacks — Q8_0, its blocks down its declared
/// rows — read from the checkpoint's description of the tensor alone, never its bytes.
pub fn stored_as_q8_planes(b: &Binding, source: &dyn WeightSource) -> std::io::Result<bool> {
    Ok(source.describe(&b.tensor)?.0 == GgmlType::Q8_0 && b.view.keeps_blocks_down_rows())
}

/// A Q8_0 weight as a spatial program reads it (`spatial::passes::from_q8`): the int8 values in the layout the program
/// declares the weight, `K × N` row-major, and one scale per 32 rows of each column, `K/32 × N`.
#[derive(Clone, Debug, PartialEq)]
pub struct Q8Planes {
    /// `K`, the declared rows.
    pub rows: usize,
    /// `N`, the declared columns.
    pub cols: usize,
    /// The int8 values, `K × N`.
    pub values: Vec<f64>,
    /// The scales, `K/32 × N`.
    pub scales: Vec<f64>,
}

/// The planes of the Q8_0 weight `b` binds, repacked from the checkpoint's blocks: what a bundle manifest names
/// `q8_0.values` and `q8_0.scales`. The view moves each block's int8 values as [`values`] moves the weights they
/// scale, and a plane block's scale is the scale of the one checkpoint block its 32 rows come from, so
/// `values[k][n] · scales[k/32][n]` is the weight [`values`] reads. Refused: a tensor stored in another format,
/// declared rows that are not whole blocks of 32, and a view that lays a checkpoint block along the declared columns.
pub fn q8_planes(b: &Binding, source: &mut dyn WeightSource) -> std::io::Result<Q8Planes> {
    let refuse = |why: String| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("`{}`: {why}", b.tensor));
    let (ty, inner, rows, raw) = stored(b, source)?;
    if ty != GgmlType::Q8_0 {
        return Err(refuse(format!("stored {}, so it has no Q8_0 planes to repack", ty.name())));
    }
    let (k, n) = b.view.declared(inner, rows);
    let len = ty.block_len();
    if k % len != 0 {
        return Err(refuse(format!("declared {k} rows, not whole Q8_0 blocks of {len}")));
    }
    // Each element's int8 value, and the index of the block it is stored in: the view moves both as it moves a weight.
    let (mut q, mut block, mut scale) = (Vec::with_capacity(inner * rows), Vec::with_capacity(inner * rows), Vec::new());
    for (i, bytes) in raw.chunks_exact(ty.block_bytes()).enumerate() {
        scale.push(f64::from(f16_to_f32(u16::from_le_bytes([bytes[0], bytes[1]]))));
        q.extend(bytes[2..].iter().map(|&v| f64::from(v as i8)));
        block.extend(std::iter::repeat(i as f64).take(len));
    }
    let (values, block) = (b.view.apply(q, inner, rows), b.view.apply(block, inner, rows));
    let mut scales = Vec::with_capacity(k / len * n);
    for kb in 0..k / len {
        for c in 0..n {
            let id = block[len * kb * n + c];
            if let Some(r) = (len * kb..len * (kb + 1)).find(|&r| block[r * n + c] != id) {
                return Err(refuse(format!(
                    "declared rows {}..{} of column {c} are not one Q8_0 block of the checkpoint: row {r} comes from another",
                    len * kb,
                    len * (kb + 1)
                )));
            }
            scales.push(scale[id as usize]);
        }
    }
    Ok(Q8Planes { rows: k, cols: n, values, scales })
}

/// `rows × cols` row-major as `cols × rows` row-major.
fn transpose_rows(m: &[f64], rows: usize, cols: usize) -> Vec<f64> {
    (0..cols).flat_map(|c| (0..rows).map(move |r| m[r * cols + c])).collect()
}

/// The rotation a layer's mixer turns by: an attention's, when it has one, and a latent attention's.
fn rope_of(mixer: &Mixer) -> Option<&Rope> {
    match mixer {
        Mixer::Attention(Attention { rope, .. }) => rope.as_ref(),
        Mixer::Mla(m) => m.rope.as_ref(),
        Mixer::GatedDelta(_) | Mixer::Recurrent(_) | Mixer::None => None,
    }
}

/// The distinct rotations `spec`'s attention layers turn by, in the order of the first layer to turn by each: a
/// program declares one pair of rotary tables per rotation (Gemma's local and global layers turn by two bases).
pub fn rotations(spec: &ModelSpec) -> Vec<&Rope> {
    let mut out: Vec<&Rope> = Vec::new();
    for r in spec.layers.iter().filter_map(|l| rope_of(&l.mixer)) {
        if !out.contains(&r) {
            out.push(r);
        }
    }
    out
}

/// The rotation layer `l` turns by, as its index in [`rotations`] — `None` for a layer that does not rotate.
pub fn rotation_of(spec: &ModelSpec, l: usize) -> Option<usize> {
    let r = rope_of(&spec.layers.get(l)?.mixer)?;
    rotations(spec).iter().position(|x| *x == r)
}

/// A rotary table's role: which rotation, a decode step's caches (`cached`) or a forward's one row, cosines or sines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RotaryTable {
    pub rotation: usize,
    pub cached: bool,
    pub cos: bool,
}

impl RotaryTable {
    /// The table's declared name: `Rcos`/`Rsin` (a forward's `RC`/`RS`) for the first rotation, the rotation's index
    /// after it for each later one (`Rcos1`, `RS2`, …).
    pub fn name(self) -> String {
        let base = match (self.cached, self.cos) {
            (true, true) => "Rcos",
            (true, false) => "Rsin",
            (false, true) => "RC",
            (false, false) => "RS",
        };
        match self.rotation {
            0 => base.to_string(),
            k => format!("{base}{k}"),
        }
    }

    /// The table `name` names, when it names one.
    pub fn named(name: &str) -> Option<RotaryTable> {
        let (cached, cos, rest) = [("Rcos", true, true), ("Rsin", true, false), ("RC", false, true), ("RS", false, false)]
            .into_iter()
            .find_map(|(base, cached, cos)| name.strip_prefix(base).map(|rest| (cached, cos, rest)))?;
        let rotation = match rest {
            "" => 0,
            k if !k.starts_with('0') && k.bytes().all(|b| b.is_ascii_digit()) => k.parse().ok()?,
            _ => return None,
        };
        Some(RotaryTable { rotation, cached, cos })
    }
}

/// A table the host computes from the spec rather than reads from the checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostTable {
    /// A rotation's cosines or sines ([`RotaryTable`]).
    Rotary(RotaryTable),
    /// The attention temperature (Llama 4's, Ministral 3's) as a rotation of `half` pairs: cosines `Temperature::at(p)`
    /// at every pair, sines zero — a decode step's caches (`cached`, row `p` for position `p`) or a forward's one row
    /// (position 0). Read by the rope sugar, it scales each query row by its own position's factor.
    Temperature { cached: bool, cos: bool, half: usize },
    /// A row of `n` ones: the weight of an RMSNorm that has none (Llama 4's QK norm), and the row a shared expert's
    /// gate is spread across (Qwen's mixtures).
    Ones(usize),
}

impl HostTable {
    /// The table's declared name: a rotary table's own, `Tcos{half}`/`Tsin{half}` (a forward's `TC{half}`/`TS{half}`),
    /// `Ones{n}`.
    pub fn name(self) -> String {
        match self {
            HostTable::Rotary(r) => r.name(),
            HostTable::Temperature { cached, cos, half } => {
                let base = match (cached, cos) {
                    (true, true) => "Tcos",
                    (true, false) => "Tsin",
                    (false, true) => "TC",
                    (false, false) => "TS",
                };
                format!("{base}{half}")
            }
            HostTable::Ones(n) => format!("Ones{n}"),
        }
    }

    /// The table `name` names, when it names one.
    pub fn named(name: &str) -> Option<HostTable> {
        if let Some(r) = RotaryTable::named(name) {
            return Some(HostTable::Rotary(r));
        }
        let count = |rest: &str| -> Option<usize> {
            (!rest.is_empty() && !rest.starts_with('0') && rest.bytes().all(|b| b.is_ascii_digit())).then(|| rest.parse().ok()).flatten()
        };
        if let Some(n) = name.strip_prefix("Ones").and_then(count) {
            return Some(HostTable::Ones(n));
        }
        [("Tcos", true, true), ("Tsin", true, false), ("TC", false, true), ("TS", false, false)]
            .into_iter()
            .find_map(|(base, cached, cos)| name.strip_prefix(base).and_then(count).map(|half| HostTable::Temperature { cached, cos, half }))
    }
}

/// The values of the host table `var` names ([`HostTable::named`]) for a model of `spec`: a decode step's caches of
/// `positions` rows, or a forward's one row (position 0). `None` when `var` names no table.
pub fn host_values(spec: &ModelSpec, var: &str, positions: usize, source: &mut dyn WeightSource) -> std::io::Result<Option<Vec<f64>>> {
    match HostTable::named(var) {
        None => Ok(None),
        Some(HostTable::Rotary(_)) => rotary_values(spec, var, positions, source),
        Some(table) => computed_values(spec, table, positions).map(Some).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("`{var}`: the model has no attention temperature"))
        }),
    }
}

/// The values of a host table the spec alone gives (every one but a rotary table, whose frequency factors a checkpoint
/// may hold): a decode step's caches of `positions` rows, or a forward's one row. `None` for a rotary table, or for a
/// temperature table of a model that has none.
pub fn computed_values(spec: &ModelSpec, table: HostTable, positions: usize) -> Option<Vec<f64>> {
    match table {
        HostTable::Rotary(_) => None,
        HostTable::Ones(n) => Some(vec![1.0; n]),
        HostTable::Temperature { cached, cos, half } => {
            let t = temperature(spec)?;
            let rows = if cached { positions } else { 1 };
            Some((0..rows).flat_map(|p| std::iter::repeat_n(if cos { t.at(p) } else { 0.0 }, half)).collect())
        }
    }
}

/// The host tables beyond the rotary ones that a decode step of `spec` reads, each once: a row of ones as wide as each
/// head that a layer norms without a weight, a row of ones as wide as the model that spreads a shared expert's gate
/// across its row, and the cosines and sines of the attention temperature for each head width that takes one.
pub fn computed_tables(spec: &ModelSpec) -> Vec<HostTable> {
    let mut out = Vec::new();
    for l in &spec.layers {
        let mut add = |t: HostTable| {
            if !out.contains(&t) {
                out.push(t);
            }
        };
        if matches!(&l.ffn, Ffn::Moe(m) if m.shared_gate) {
            add(HostTable::Ones(spec.d));
        }
        let Mixer::Attention(a) = &l.mixer else { continue };
        if a.qk_unit_norm {
            add(HostTable::Ones(a.head_dim));
        }
        if a.temperature.is_some() {
            for cos in [true, false] {
                add(HostTable::Temperature { cached: true, cos, half: a.head_dim / 2 });
            }
        }
    }
    out
}

/// The attention temperature `spec`'s layers take — one for the model, as every family that has one sets it.
pub fn temperature(spec: &ModelSpec) -> Option<crate::model::spec::Temperature> {
    spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Attention(a) => a.temperature,
        _ => None,
    })
}

/// The rotary tables for positions `0..rows`: row `p` holds the `(cos, sin)` of each of the `dims/2` frequencies
/// ([`Rope::rotation`] — the checkpoint's per-frequency factors, its sections, linear and YaRN scaling, the
/// magnitude folded into both).
pub fn rope_tables(rope: &Rope, rows: usize, factors: Option<&[f64]>) -> (Vec<f64>, Vec<f64>) {
    let half = rope.dims / 2;
    let (mut cos, mut sin) = (Vec::with_capacity(rows * half), Vec::with_capacity(rows * half));
    for p in 0..rows {
        for i in 0..half {
            let factor = match (&rope.scaling, factors) {
                (RopeScaling::Factors, Some(fs)) => Some(fs[i]),
                _ => None,
            };
            let (c, s) = rope.rotation(i, p, factor);
            cos.push(c);
            sin.push(s);
        }
    }
    (cos, sin)
}

/// The values of the rotary table `var` names ([`RotaryTable::named`]) for a model of `spec`: a decode step's caches of
/// `positions` rows, or a forward's one row (position 0), of the rotation the name gives, the frequency factors read
/// from `source` when the rotation reads the checkpoint's (`rope_freqs.weight`). `None` when `var` names no table.
pub fn rotary_values(spec: &ModelSpec, var: &str, positions: usize, source: &mut dyn WeightSource) -> std::io::Result<Option<Vec<f64>>> {
    let Some(table) = RotaryTable::named(var) else { return Ok(None) };
    let rope = *rotations(spec).get(table.rotation).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("`{var}`: the model has no rotation {}", table.rotation))
    })?;
    let factors = match rope.scaling {
        RopeScaling::Factors => Some(values(&Binding { tensor: "rope_freqs.weight".into(), view: View::Same }, source)?),
        _ => None,
    };
    let (cos, sin) = rope_tables(rope, if table.cached { positions } else { 1 }, factors.as_deref());
    Ok(Some(if table.cos { cos } else { sin }))
}
