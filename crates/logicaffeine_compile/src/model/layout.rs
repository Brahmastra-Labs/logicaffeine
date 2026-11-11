//! A checkpoint presented in the layout the spec's arms read.
//!
//! Qwen3-Next and Qwen3.5 compute one Gated DeltaNet, but a Qwen3-Next checkpoint (llama.cpp's `qwen3next`, as its
//! converter writes it) pairs its value heads with its key heads grouped — value head `k·r + i` reads key head `k`, `r`
//! value heads a key head — and keeps each key head's betas and alphas in one projection, `ssm_ba`. Qwen3.5's tiles them
//! — value head `t` reads key head `t mod k` — and keeps `ssm_beta` and `ssm_alpha` apart. [`presented`] gives a
//! Qwen3-Next checkpoint Qwen3.5's layout: every value-head axis reordered so tiled position `t` holds grouped head
//! `(t mod k)·r + ⌊t/k⌋`, the head its key head pairs it with either way, and `ssm_ba` split. The spec, the author, the
//! binder, the reference and every lowering then read one Gated DeltaNet; the spec refuses a checkpoint still holding
//! `ssm_ba`, so a grouped layout never reaches them unpresented.
//!
//! A Phi-3 or GLM-4 checkpoint (llama.cpp's `phi3`, `glm4`) keeps each block's gate and up projections as one `ffn_up`
//! twice `feed_forward_length` wide, the gate's rows first; [`presented`] gives it `ffn_gate` and `ffn_up` apart, as
//! every other dense SwiGLU stores them, and the spec refuses such a checkpoint read raw.
//!
//! A GLM-4.5 checkpoint (llama.cpp's `glm4moe`) stores each block's FFN norm as `post_attention_norm`, the name other
//! families give the norm on the attention's output; [`presented`] names it `ffn_norm`, and the spec refuses such a
//! checkpoint read raw.
//!
//! An LFM2 checkpoint (llama.cpp's `lfm2`) stores its output norm as `token_embd_norm` (the converter maps LFM2's
//! `embedding_norm` there, and llama.cpp reads it as the output norm); [`presented`] names it `output_norm`, and the spec
//! refuses such a checkpoint read raw.
//!
//! A Falcon-H1 checkpoint (llama.cpp's `falcon-h1`) stores each block's FFN norm as `ffn_norm`, with no `.weight` (the
//! converter maps `pre_ff_layernorm.weight` whole, and llama.cpp loads the name so); [`presented`] names it
//! `ffn_norm.weight`, and the spec refuses such a checkpoint read raw.

use std::collections::BTreeMap;

use super::gguf::MetaValue;
use super::quant::GgmlType;
use super::reference::WeightSource;
use super::spec::SpecSource;
use super::Checkpoint;

/// `c` presented in the layout the spec's arms read: a Qwen3-Next checkpoint in Qwen3.5's, a Phi-3 or GLM-4 one with its
/// gate and up projections apart, a GLM-4.5 one with its FFN norms named `ffn_norm`, an LFM2 one with its output norm named
/// `output_norm`, a Falcon-H1 one with its FFN norms named `ffn_norm.weight`, every other as it is.
pub fn presented(c: Box<dyn Checkpoint>) -> std::io::Result<Box<dyn Checkpoint>> {
    if c.arch() == "qwen3next" && c.tensor_names().iter().any(|n| n.ends_with(".ssm_ba.weight")) {
        Ok(Box::new(Presented::tiled(c)?))
    } else if matches!(c.arch(), "phi3" | "glm4") && fused_gate_up(&*c) {
        Ok(Box::new(Presented::split_gate_up(c)?))
    } else if c.arch() == "glm4moe" && c.tensor_names().iter().any(|n| n.ends_with(".post_attention_norm.weight")) {
        Ok(Box::new(Presented::renamed(c, ".post_attention_norm.weight", ".ffn_norm.weight")))
    } else if c.arch() == "lfm2" && c.tensor_names().contains(&"token_embd_norm.weight") {
        Ok(Box::new(Presented::renamed(c, "token_embd_norm.weight", "output_norm.weight")))
    } else if c.arch() == "falcon-h1" && c.tensor_names().iter().any(|n| n.ends_with(".ffn_norm")) {
        Ok(Box::new(Presented::renamed(c, ".ffn_norm", ".ffn_norm.weight")))
    } else {
        Ok(c)
    }
}

/// Whether some block of `c` keeps its gate and up projections as one `ffn_up`, with no `ffn_gate` of its own.
fn fused_gate_up(c: &dyn Checkpoint) -> bool {
    let names = c.tensor_names();
    names.iter().any(|n| n.ends_with(".ffn_up.weight") && !names.contains(&n.replace(".ffn_up.", ".ffn_gate.").as_str()))
}

/// What one presented tensor reads of the stored one.
#[derive(Clone, Debug)]
enum Read {
    /// Presented row `r` is stored row `rows[r]`.
    Rows { src: String, rows: Vec<usize> },
    /// Every row's elements in chunks of `chunk`, presented chunk `c` the stored chunk `order[c]`.
    Chunks { src: String, chunk: usize, order: Vec<usize> },
    /// The stored tensor `src` as it is, under another name.
    Same { src: String },
}

/// A checkpoint presented in the layout the spec's arms read.
struct Presented {
    inner: Box<dyn Checkpoint>,
    names: Vec<String>,
    dims: BTreeMap<String, Vec<u64>>,
    reads: BTreeMap<String, Read>,
}

impl Presented {
    /// A Phi-3 or GLM-4 checkpoint with each fused `ffn_up` read as `ffn_gate` (its first `feed_forward_length` rows) and
    /// `ffn_up` (the rest).
    fn split_gate_up(inner: Box<dyn Checkpoint>) -> std::io::Result<Presented> {
        let bad = |why: String| std::io::Error::new(std::io::ErrorKind::InvalidData, why);
        let ff = inner
            .arch_meta("feed_forward_length")
            .and_then(MetaValue::as_u64)
            .ok_or_else(|| bad(format!("{}.feed_forward_length is missing", inner.arch())))?;
        let all: Vec<String> = inner.tensor_names().iter().map(|n| n.to_string()).collect();
        let mut names = Vec::new();
        let mut dims = BTreeMap::new();
        let mut reads = BTreeMap::new();
        for name in &all {
            let stored: Vec<u64> = inner.dims(name).map(<[u64]>::to_vec).unwrap_or_default();
            let gate = name.replace(".ffn_up.", ".ffn_gate.");
            if name.ends_with(".ffn_up.weight") && !all.contains(&gate) {
                if stored.len() != 2 || stored[1] != 2 * ff {
                    return Err(bad(format!("{name} is {stored:?}, not a gate and an up projection of {ff} each")));
                }
                let ff = ff as usize;
                for (presented, rows) in [(gate.clone(), 0..ff), (name.clone(), ff..2 * ff)] {
                    names.push(presented.clone());
                    dims.insert(presented.clone(), vec![stored[0], ff as u64]);
                    reads.insert(presented, Read::Rows { src: name.clone(), rows: rows.collect() });
                }
                continue;
            }
            names.push(name.clone());
            dims.insert(name.clone(), stored);
        }
        Ok(Presented { inner, names, dims, reads })
    }

    /// `inner` with every tensor named `…<from>` presented as `…<to>`.
    fn renamed(inner: Box<dyn Checkpoint>, from: &str, to: &str) -> Presented {
        let mut names = Vec::new();
        let mut dims = BTreeMap::new();
        let mut reads = BTreeMap::new();
        for name in inner.tensor_names() {
            let stored: Vec<u64> = inner.dims(name).map(<[u64]>::to_vec).unwrap_or_default();
            let presented = match name.strip_suffix(from) {
                Some(stem) => {
                    let presented = format!("{stem}{to}");
                    reads.insert(presented.clone(), Read::Same { src: name.to_string() });
                    presented
                }
                None => name.to_string(),
            };
            names.push(presented.clone());
            dims.insert(presented, stored);
        }
        Presented { inner, names, dims, reads }
    }

    /// A Qwen3-Next checkpoint in Qwen3.5's tiled layout.
    fn tiled(inner: Box<dyn Checkpoint>) -> std::io::Result<Presented> {
        let bad = |why: String| std::io::Error::new(std::io::ErrorKind::InvalidData, why);
        let key = |k: &str| -> std::io::Result<usize> {
            inner
                .arch_meta(k)
                .and_then(MetaValue::as_u64)
                .map(|v| v as usize)
                .ok_or_else(|| bad(format!("qwen3next.{k} is missing")))
        };
        let (kh, vh, kd) = (key("ssm.group_count")?, key("ssm.time_step_rank")?, key("ssm.state_size")?);
        let inner_size = key("ssm.inner_size")?;
        if kh == 0 || vh % kh != 0 || inner_size % vh.max(1) != 0 {
            return Err(bad(format!("{vh} value heads of {inner_size} over {kh} key heads do not pair")));
        }
        let (r, vd) = (vh / kh, inner_size / vh);
        // Tiled position `t` holds grouped head `(t mod kh)·r + ⌊t/kh⌋`.
        let head: Vec<usize> = (0..vh).map(|t| (t % kh) * r + t / kh).collect();
        let heads_of = |d: usize| -> Vec<usize> { head.iter().flat_map(|&g| (0..d).map(move |e| g * d + e)).collect() };
        let qk = 2 * kh * kd;
        let mut names = Vec::new();
        let mut dims = BTreeMap::new();
        let mut reads = BTreeMap::new();
        for name in inner.tensor_names() {
            let stored: Vec<u64> = inner.dims(name).map(<[u64]>::to_vec).unwrap_or_default();
            let suffix = name.splitn(3, '.').nth(2).unwrap_or("");
            let read = match suffix {
                "attn_qkv.weight" | "ssm_conv1d.weight" => {
                    Some(Read::Rows { src: name.into(), rows: (0..qk).chain(heads_of(vd).into_iter().map(|o| qk + o)).collect() })
                }
                "attn_gate.weight" => Some(Read::Rows { src: name.into(), rows: heads_of(vd) }),
                "ssm_dt.bias" | "ssm_a" => Some(Read::Chunks { src: name.into(), chunk: 1, order: head.clone() }),
                "ssm_out.weight" => Some(Read::Chunks { src: name.into(), chunk: vd, order: head.clone() }),
                "ssm_ba.weight" => {
                    // Each key head's `r` betas, then its `r` alphas: grouped head `g` is key head `g / r`'s `g mod r`-th.
                    let block = name.trim_end_matches("ssm_ba.weight");
                    for (part, off) in [("ssm_beta.weight", 0), ("ssm_alpha.weight", r)] {
                        let rows = head.iter().map(|&g| (g / r) * 2 * r + off + g % r).collect();
                        let presented = format!("{block}{part}");
                        names.push(presented.clone());
                        dims.insert(presented.clone(), vec![stored[0], vh as u64]);
                        reads.insert(presented, Read::Rows { src: name.into(), rows });
                    }
                    continue;
                }
                _ => None,
            };
            names.push(name.to_string());
            dims.insert(name.to_string(), stored);
            if let Some(read) = read {
                reads.insert(name.to_string(), read);
            }
        }
        Ok(Presented { inner, names, dims, reads })
    }
}

impl SpecSource for Presented {
    fn arch(&self) -> &str {
        self.inner.arch()
    }
    fn arch_meta(&self, suffix: &str) -> Option<&MetaValue> {
        self.inner.arch_meta(suffix)
    }
    fn dims(&self, name: &str) -> Option<&[u64]> {
        self.dims.get(name).map(Vec::as_slice)
    }
    fn tensor_names(&self) -> Vec<&str> {
        self.names.iter().map(String::as_str).collect()
    }
}

impl WeightSource for Presented {
    fn describe(&self, name: &str) -> std::io::Result<(GgmlType, Vec<u64>)> {
        let dims = self
            .dims
            .get(name)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("no tensor {name}")))?;
        let src = match self.reads.get(name) {
            Some(Read::Rows { src, .. } | Read::Chunks { src, .. } | Read::Same { src }) => src.as_str(),
            None => name,
        };
        Ok((self.inner.describe(src)?.0, dims))
    }

    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>> {
        match self.reads.get(name).cloned() {
            None => self.inner.row_bytes(name, first, count),
            Some(Read::Same { src }) => self.inner.row_bytes(&src, first, count),
            Some(Read::Rows { src, rows }) => {
                let mut out = Vec::new();
                let picked = rows.get(first..first + count).ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{name}: rows {first}..{} of {}", first + count, rows.len()))
                })?;
                // Runs of consecutive stored rows read at once.
                let mut i = 0;
                while i < picked.len() {
                    let mut j = i + 1;
                    while j < picked.len() && picked[j] == picked[j - 1] + 1 {
                        j += 1;
                    }
                    out.extend(self.inner.row_bytes(&src, picked[i], j - i)?);
                    i = j;
                }
                Ok(out)
            }
            Some(Read::Chunks { src, chunk, order }) => {
                let (ty, dims) = self.inner.describe(&src)?;
                let inner = dims.first().copied().unwrap_or(1) as usize;
                let (len, bytes) = (ty.block_len(), ty.block_bytes());
                if chunk % len != 0 || inner != chunk * order.len() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("{name}: chunks of {chunk} are not whole {} blocks of {len}", ty.name()),
                    ));
                }
                let chunk_bytes = chunk / len * bytes;
                let stored = self.inner.row_bytes(&src, first, count)?;
                let row = chunk_bytes * order.len();
                let mut out = Vec::with_capacity(stored.len());
                for r in stored.chunks_exact(row) {
                    for &c in &order {
                        out.extend_from_slice(&r[c * chunk_bytes..(c + 1) * chunk_bytes]);
                    }
                }
                Ok(out)
            }
        }
    }

    fn has(&self, name: &str) -> bool {
        self.dims.contains_key(name)
    }
}
