//! The cost of one token: batch-1 decode, priced on a machine from the checkpoint itself.
//!
//! ★ AT BATCH ONE, EVERY TOKEN REREADS EVERY WEIGHT. A decode step multiplies one activation vector through
//! every projection of the model, so its arithmetic intensity is one multiply-add per weight byte read and the
//! matrix engines idle: the step is bounded by where the weights live and how fast that memory streams, and by
//! how long its chain of dependent stages takes to walk. That is the regime a GPU is worst at and an
//! SRAM-resident dataflow machine is built for, so it is the regime this module prices for both, by one rule.
//!
//! The step is read off a GGUF header ([`GgufHeader`]): every tensor's bytes from ggml's block arithmetic
//! ([`GgmlType`]), every tensor's role from llama.cpp's tensor names, the KV cache and the recurrent state from
//! the header's metadata. Nothing is per model: a block's mixer is attention when it carries attention
//! projections and a recurrence when it carries `ssm_*` tensors; a tensor no role names is refused by name.
//!
//! The machine is facts with sources ([`Chip`]), by the rule the roofs keep: a GPU's rates are its roofs
//! preset's, its capacities and links its vendor's table's. What the record does not give is absent, never
//! guessed — an undeclared per-stage overhead or link latency leaves the FLOOR priced (it is a true lower
//! bound with the term at zero) and the PREDICTION refused.
//!
//! [`price`] lays the step's stages onto the chips Megatron's way — projections split, attention split by
//! heads, norms replicated, an all-reduce after each block's attention output and FFN down projection, an
//! arg-max across vocabulary shards at the head — and returns both numbers: the floor no implementation can
//! beat (the largest of the memory, compute and collective sums), and, where every term is declared, the
//! prediction (each stage's roofline plus its collective and overhead, summed along the chain).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::rdu_chip::{Fact, Provenance};
use super::roofs::{Engine, MemLevel, Rate, Roofs, StreamMix};
use crate::codegen_sva::fabric_target::Precision;

// ── ggml's storage types ────────────────────────────────────────────────────────────────────────────

/// Where ggml's block arithmetic is read from.
pub const GGML_TYPES_SOURCE: &str = "github.com/ggml-org/llama.cpp: ggml/include/ggml.h `enum ggml_type`; \
     ggml/src/ggml.c `type_traits` (.blck_size, .type_size = sizeof(block_*)); ggml/src/ggml-common.h \
     `static_assert(sizeof(block_*) == …)`";

/// A ggml tensor storage type: the id GGUF stores and the block it packs elements into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[allow(non_camel_case_types)]
pub enum GgmlType {
    F32,
    F16,
    Q4_0,
    Q4_1,
    Q5_0,
    Q5_1,
    Q8_0,
    Q8_1,
    Q2K,
    Q3K,
    Q4K,
    Q5K,
    Q6K,
    Q8K,
    Iq2Xxs,
    Iq2Xs,
    Iq3Xxs,
    Iq1S,
    Iq4Nl,
    Iq3S,
    Iq2S,
    Iq4Xs,
    I8,
    I16,
    I32,
    I64,
    F64,
    Iq1M,
    Bf16,
    Tq1_0,
    Tq2_0,
    Mxfp4,
    Nvfp4,
    Q1_0,
    Q2_0,
}

impl GgmlType {
    /// Every type ggml currently defines. Ids 4, 5, 31–33 and 36–38 are retired and absent.
    pub const ALL: [GgmlType; 35] = [
        GgmlType::F32,
        GgmlType::F16,
        GgmlType::Q4_0,
        GgmlType::Q4_1,
        GgmlType::Q5_0,
        GgmlType::Q5_1,
        GgmlType::Q8_0,
        GgmlType::Q8_1,
        GgmlType::Q2K,
        GgmlType::Q3K,
        GgmlType::Q4K,
        GgmlType::Q5K,
        GgmlType::Q6K,
        GgmlType::Q8K,
        GgmlType::Iq2Xxs,
        GgmlType::Iq2Xs,
        GgmlType::Iq3Xxs,
        GgmlType::Iq1S,
        GgmlType::Iq4Nl,
        GgmlType::Iq3S,
        GgmlType::Iq2S,
        GgmlType::Iq4Xs,
        GgmlType::I8,
        GgmlType::I16,
        GgmlType::I32,
        GgmlType::I64,
        GgmlType::F64,
        GgmlType::Iq1M,
        GgmlType::Bf16,
        GgmlType::Tq1_0,
        GgmlType::Tq2_0,
        GgmlType::Mxfp4,
        GgmlType::Nvfp4,
        GgmlType::Q1_0,
        GgmlType::Q2_0,
    ];

    /// `(id, elements per block, bytes per block)`, from [`GGML_TYPES_SOURCE`].
    fn spec(self) -> (u32, u64, u64) {
        use GgmlType::*;
        match self {
            F32 => (0, 1, 4),
            F16 => (1, 1, 2),
            Q4_0 => (2, 32, 18),
            Q4_1 => (3, 32, 20),
            Q5_0 => (6, 32, 22),
            Q5_1 => (7, 32, 24),
            Q8_0 => (8, 32, 34),
            Q8_1 => (9, 32, 36),
            Q2K => (10, 256, 84),
            Q3K => (11, 256, 110),
            Q4K => (12, 256, 144),
            Q5K => (13, 256, 176),
            Q6K => (14, 256, 210),
            Q8K => (15, 256, 292),
            Iq2Xxs => (16, 256, 66),
            Iq2Xs => (17, 256, 74),
            Iq3Xxs => (18, 256, 98),
            Iq1S => (19, 256, 50),
            Iq4Nl => (20, 32, 18),
            Iq3S => (21, 256, 110),
            Iq2S => (22, 256, 82),
            Iq4Xs => (23, 256, 136),
            I8 => (24, 1, 1),
            I16 => (25, 1, 2),
            I32 => (26, 1, 4),
            I64 => (27, 1, 8),
            F64 => (28, 1, 8),
            Iq1M => (29, 256, 56),
            Bf16 => (30, 1, 2),
            Tq1_0 => (34, 256, 54),
            Tq2_0 => (35, 256, 66),
            Mxfp4 => (39, 32, 17),
            Nvfp4 => (40, 64, 36),
            Q1_0 => (41, 128, 18),
            Q2_0 => (42, 64, 18),
        }
    }

    pub fn id(self) -> u32 {
        self.spec().0
    }

    pub fn block_elems(self) -> u64 {
        self.spec().1
    }

    pub fn block_bytes(self) -> u64 {
        self.spec().2
    }

    /// The type GGUF stores as `id`, or `None` for a retired or unknown id — never a guess.
    pub fn from_id(id: u32) -> Option<GgmlType> {
        GgmlType::ALL.into_iter().find(|t| t.id() == id)
    }

    /// Bytes `elems` elements take, or `None` when they do not fill whole blocks.
    pub fn bytes(self, elems: u64) -> Option<u64> {
        (elems % self.block_elems() == 0).then(|| elems / self.block_elems() * self.block_bytes())
    }
}

// ── The header ──────────────────────────────────────────────────────────────────────────────────────

/// One row of a GGUF tensor table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TensorRow {
    pub name: String,
    /// Dimensions, innermost first (ggml's order): a projection is `[in, out]`, an expert stack `[in, out, n]`.
    pub dims: Vec<u64>,
    #[serde(rename = "type")]
    pub type_id: u32,
    /// Byte offset of the tensor's data from [`GgufHeader::data_offset`].
    pub offset: u64,
}

/// A GGUF header: the metadata and the whole tensor table, and where in the file the tensor data lives.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GgufHeader {
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub file_bytes: u64,
    pub data_offset: u64,
    pub tensors: Vec<TensorRow>,
}

#[derive(Deserialize)]
struct HeaderLedger {
    header: GgufHeader,
}

/// A header ledger read as a checkpoint's description: its metadata as GGUF values — an integer unsigned, or signed
/// when negative; a decimal that is exactly a binary32 one, as the converters store metadata decimals, any other
/// binary64; an array of its first item's type; a value the ledger summarized (a vocabulary as `<n strings>`) the
/// summary's string — and its tensor directory.
pub struct HeaderSource {
    arch: String,
    meta: BTreeMap<String, crate::model::gguf::MetaValue>,
    dims: BTreeMap<String, Vec<u64>>,
    names: Vec<String>,
}

impl crate::model::spec::SpecSource for HeaderSource {
    fn arch(&self) -> &str {
        &self.arch
    }
    fn arch_meta(&self, suffix: &str) -> Option<&crate::model::gguf::MetaValue> {
        self.meta.get(&format!("{}.{suffix}", self.arch))
    }
    fn dims(&self, name: &str) -> Option<&[u64]> {
        self.dims.get(name).map(|d| d.as_slice())
    }
    fn tensor_names(&self) -> Vec<&str> {
        self.names.iter().map(|n| n.as_str()).collect()
    }
}

/// A header as a checkpoint's weights: every tensor's format and dims and none of its bytes, what lays the checkpoint
/// out on the card and lowers its step.
impl crate::model::reference::WeightSource for GgufHeader {
    fn describe(&self, name: &str) -> std::io::Result<(crate::model::quant::GgmlType, Vec<u64>)> {
        let t = self.tensors.iter().find(|t| t.name == name).ok_or_else(|| std::io::Error::other(format!("the header holds no {name}")))?;
        let ty = crate::model::quant::GgmlType::from_id(t.type_id).map_err(|id| std::io::Error::other(format!("{name}: ggml type {id}")))?;
        Ok((ty, t.dims.clone()))
    }
    fn row_bytes(&mut self, name: &str, _first: usize, _count: usize) -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::other(format!("a header holds no bytes of {name}")))
    }
    fn has(&self, name: &str) -> bool {
        self.tensors.iter().any(|t| t.name == name)
    }
}

impl GgufHeader {
    /// A header ledger (`benchmarks/silicon/decode/headers/*.json`, written by
    /// `scripts/accel/gguf_remote_header.py --tensors`).
    pub fn read_ledger(path: &Path) -> Result<GgufHeader, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let ledger: HeaderLedger = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        Ok(ledger.header)
    }

    /// The header as a checkpoint's description ([`crate::model::spec::SpecSource`]).
    pub fn source(&self) -> HeaderSource {
        use crate::model::gguf::MetaValue;
        fn value(v: &serde_json::Value) -> Option<MetaValue> {
            Some(match v {
                serde_json::Value::Bool(b) => MetaValue::Bool(*b),
                serde_json::Value::Number(n) => match (n.as_u64(), n.as_i64()) {
                    (Some(u), _) => MetaValue::U64(u),
                    (None, Some(i)) => MetaValue::I64(i),
                    _ => {
                        let v = n.as_f64()?;
                        if (v as f32) as f64 == v {
                            MetaValue::F32(v as f32)
                        } else {
                            MetaValue::F64(v)
                        }
                    }
                },
                serde_json::Value::String(s) => MetaValue::Str(s.as_bytes().to_vec()),
                serde_json::Value::Array(a) => {
                    let items: Vec<MetaValue> = a.iter().map(value).collect::<Option<_>>()?;
                    MetaValue::Array { ty: items.first().map_or(8, MetaValue::gguf_type), items }
                }
                serde_json::Value::Null | serde_json::Value::Object(_) => return None,
            })
        }
        HeaderSource {
            arch: self.arch().unwrap_or("").to_string(),
            meta: self.metadata.iter().filter_map(|(k, v)| Some((k.clone(), value(v)?))).collect(),
            dims: self.tensors.iter().map(|t| (t.name.clone(), t.dims.clone())).collect(),
            names: self.tensors.iter().map(|t| t.name.clone()).collect(),
        }
    }

    /// The architecture the header describes, read as [`crate::model::spec::ModelSpec::from_gguf`] reads its
    /// checkpoint's.
    pub fn spec(&self) -> Result<crate::model::spec::ModelSpec, crate::model::spec::SpecError> {
        crate::model::spec::ModelSpec::from_source(&self.source())
    }

    /// `general.architecture` — the prefix every architecture key carries.
    pub fn arch(&self) -> Option<&str> {
        self.metadata.get("general.architecture")?.as_str()
    }

    /// The architecture key `{arch}.{key}` as an unsigned integer.
    pub fn meta_u64(&self, key: &str) -> Option<u64> {
        self.metadata.get(&format!("{}.{key}", self.arch()?))?.as_u64()
    }

    fn require(&self, key: &str) -> Result<u64, DecodeRefusal> {
        self.meta_u64(key)
            .ok_or_else(|| DecodeRefusal::MissingMetadata(format!("{}.{key}", self.arch().unwrap_or("?"))))
    }

    /// The storage type of `t`.
    pub fn tensor_type(&self, t: &TensorRow) -> Result<GgmlType, DecodeRefusal> {
        GgmlType::from_id(t.type_id).ok_or_else(|| DecodeRefusal::UnknownType {
            tensor: t.name.clone(),
            id: t.type_id,
        })
    }

    /// Bytes `t` occupies in the file.
    pub fn tensor_bytes(&self, t: &TensorRow) -> Result<u64, DecodeRefusal> {
        let ty = self.tensor_type(t)?;
        let elems: u64 = t.dims.iter().product();
        ty.bytes(elems).ok_or_else(|| DecodeRefusal::RaggedBlock {
            tensor: t.name.clone(),
            elems,
            block: ty.block_elems(),
        })
    }

    /// Holds the tensor table to the file: in offset order each tensor's bytes fill the gap to the next up to
    /// the alignment padding, and the last one ends exactly where the file does. A wrong block size, a wrong
    /// type id or a missing tensor cannot pass.
    pub fn byte_audit(&self) -> Result<(), String> {
        let align = self
            .metadata
            .get("general.alignment")
            .and_then(|v| v.as_u64())
            .unwrap_or(32);
        let mut order: Vec<&TensorRow> = self.tensors.iter().collect();
        order.sort_by_key(|t| t.offset);
        let size = |t: &TensorRow| self.tensor_bytes(t).map_err(|e| format!("{e:?}"));
        for pair in order.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (bytes, gap) = (size(a)?, b.offset - a.offset);
            if !(bytes <= gap && gap < bytes + align) {
                return Err(format!(
                    "`{}` is {bytes} bytes but the next tensor starts {gap} bytes later",
                    a.name
                ));
            }
        }
        let last = order.last().ok_or("an empty tensor table")?;
        let end = self.data_offset + last.offset + size(last)?;
        if end != self.file_bytes {
            return Err(format!(
                "the last tensor `{}` ends at byte {end}, the file at {}",
                last.name, self.file_bytes
            ));
        }
        Ok(())
    }
}

// ── The deployment ──────────────────────────────────────────────────────────────────────────────────

/// How the matrix weights are stored for the run.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum WeightBytes {
    /// As the checkpoint stores them, block scales included.
    AsStored,
    /// Re-quantized to this many bytes per weight (fp8 = 1, bf16 = 2); vectors stay as stored.
    PerWeight(f64),
}

/// What a deployment of a checkpoint holds per token.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Deployment {
    /// The context the KV cache has reached.
    pub ctx: u64,
    pub weights: WeightBytes,
    /// Bytes per KV-cache element.
    pub kv_bytes: f64,
    /// Bytes per recurrent-state element.
    pub state_bytes: f64,
    /// Bytes per activation element a reduction across chips carries.
    pub activation_bytes: f64,
    /// Whether the speculative (multi-token-prediction) blocks run as part of the step.
    pub speculative: bool,
}

impl Deployment {
    /// The checkpoint as stored, with llama.cpp's defaults: an f16 KV cache and an f32 recurrent state.
    pub fn as_stored(ctx: u64) -> Deployment {
        Deployment {
            ctx,
            weights: WeightBytes::AsStored,
            kv_bytes: 2.0,
            state_bytes: 4.0,
            activation_bytes: 2.0,
            speculative: false,
        }
    }
}

// ── The step ────────────────────────────────────────────────────────────────────────────────────────

/// What a stage of the chain does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum StageKind {
    /// One embedding row.
    Embed,
    /// A normalization over the whole hidden vector: a full-vector reduction, so a dependent step.
    Norm,
    /// Matrix-vector products that share one input vector.
    Project,
    /// Scores and the weighted sum over the KV cache.
    Attention,
    /// A linear recurrence over a carried state (Gated DeltaNet, Mamba-class).
    Recurrence,
    /// The expert router: its top-k picks which experts the next stage reads.
    Router,
    /// The vocabulary projection and the arg-max over it.
    Head,
}

/// How far a stage's work divides across chips.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Split {
    /// Rows or columns of its projections: any number of chips.
    Any,
    /// By heads: its arithmetic over `q` query heads, its carried state over `kv` heads.
    Heads { q: u64, kv: u64 },
    /// Every chip does all of it (a norm, a router).
    Replicated,
    /// One chip does it (an embedding row).
    One,
}

/// One dependent stage of the decode chain.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Stage {
    /// The block, or `None` for the embedding and the head.
    pub layer: Option<u32>,
    pub kind: StageKind,
    pub tensors: Vec<String>,
    /// Weight bytes the stage reads per token.
    pub weight_bytes: f64,
    /// The held bytes those weight bytes are read from, and so the tiers they stream from: the embedding table for
    /// the embedding row and for a head tied to the table, the weights for every other stage.
    pub weight_class: HeldClass,
    /// Carried-state bytes read per token: the KV cache, a recurrent state.
    pub state_read: f64,
    /// Carried-state bytes written per token.
    pub state_write: f64,
    /// Matrix-engine operations per token (a multiply-add counts two).
    pub ops: f64,
    pub split: Split,
    /// Whether the stage's output is summed across the chips it is split over (Megatron's row-split
    /// projections) or reduced to one answer (the head's arg-max).
    pub reduces: bool,
    /// Bytes that reduction carries.
    pub reduce_bytes: f64,
}

/// What one token of a deployment reads, writes and computes, as the chain of its dependent stages.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecodeStep {
    pub deployment: Deployment,
    pub stages: Vec<Stage>,
    /// Bytes the deployment holds resident: every weight the step reads (the whole embedding table, not one
    /// row), the KV cache at the deployment's context and the recurrent state, by how often a token moves them.
    pub held: Vec<Held>,
}

/// A class of resident bytes and the bytes of it one token moves: what decides which memory tier keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Held {
    pub class: HeldClass,
    pub bytes: f64,
    pub moved_per_token: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum HeldClass {
    Weights,
    EmbeddingTable,
    KvCache,
    RecurrentState,
}

/// Why a step or a price could not be computed.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum DecodeRefusal {
    /// A tensor whose name no role names.
    UnknownTensor(String),
    /// A storage type id ggml does not define (or retired).
    UnknownType { tensor: String, id: u32 },
    /// A tensor whose element count does not fill whole blocks of its type.
    RaggedBlock { tensor: String, elems: u64, block: u64 },
    /// A metadata key the step needs.
    MissingMetadata(String),
    /// A block whose tensors name no mixer, or a shape the role table cannot read.
    Unreadable(String),
    /// The held bytes exceed every tier of every chip.
    NotResident { needed: f64, capacity: f64 },
}

/// A tensor's job, from llama.cpp's tensor names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Embed,
    RopeFreqs,
    OutputNorm,
    Output,
    PreMixerNorm,
    MixerIn,
    MixerParam,
    MixerOut,
    PostMixerNorm,
    PreFfnNorm,
    Router,
    FfnIn,
    FfnInExperts,
    FfnOut,
    FfnOutExperts,
    PostFfnNorm,
    DraftNorm,
    DraftIn,
    DraftHeadNorm,
}

fn block_role(key: &str) -> Option<Role> {
    use Role::*;
    Some(match key {
        "attn_norm" => PreMixerNorm,
        "attn_q" | "attn_k" | "attn_v" | "attn_qkv" | "attn_gate" | "ssm_alpha" | "ssm_beta" => MixerIn,
        "attn_q_norm" | "attn_k_norm" | "ssm_norm" | "ssm_conv1d" | "ssm_a" | "ssm_dt" => MixerParam,
        "attn_output" | "ssm_out" => MixerOut,
        "post_attention_norm" => PostMixerNorm,
        "ffn_norm" => PreFfnNorm,
        "ffn_gate_inp" | "exp_probs_b" | "ffn_gate_inp_shexp" => Router,
        "ffn_gate" | "ffn_up" | "ffn_gate_shexp" | "ffn_up_shexp" => FfnIn,
        "ffn_gate_exps" | "ffn_up_exps" => FfnInExperts,
        "ffn_down" | "ffn_down_shexp" => FfnOut,
        "ffn_down_exps" => FfnOutExperts,
        "post_ffw_norm" | "ffn_post_norm" => PostFfnNorm,
        "nextn.enorm" | "nextn.hnorm" => DraftNorm,
        "nextn.eh_proj" => DraftIn,
        "nextn.shared_head_norm" => DraftHeadNorm,
        _ => return None,
    })
}

fn global_role(name: &str) -> Option<Role> {
    Some(match name {
        "token_embd.weight" => Role::Embed,
        "rope_freqs.weight" => Role::RopeFreqs,
        "output_norm.weight" => Role::OutputNorm,
        "output.weight" => Role::Output,
        _ => return None,
    })
}

/// `blk.7.attn_q.weight` → `(7, "attn_q")`.
fn split_block_name(name: &str) -> Option<(u32, &str)> {
    let rest = name.strip_prefix("blk.")?;
    let (idx, tail) = rest.split_once('.')?;
    let key = tail
        .strip_suffix(".weight")
        .or_else(|| tail.strip_suffix(".bias"))
        .unwrap_or(tail);
    Some((idx.parse().ok()?, key))
}

/// The roles whose weights a matrix engine reads, and so the ones a re-quantization re-stores.
fn is_matrix_role(r: Role) -> bool {
    matches!(
        r,
        Role::Embed | Role::Output | Role::MixerIn | Role::MixerOut | Role::FfnIn | Role::FfnInExperts | Role::FfnOut | Role::FfnOutExperts | Role::DraftIn
    )
}

struct Reader<'a> {
    h: &'a GgufHeader,
    d: &'a Deployment,
}

impl Reader<'_> {
    /// Bytes `t` takes in the deployment, and its element count.
    fn stored(&self, t: &TensorRow, role: Role) -> Result<(f64, f64), DecodeRefusal> {
        let elems: u64 = t.dims.iter().product();
        let bytes = match self.d.weights {
            WeightBytes::PerWeight(b) if is_matrix_role(role) && t.dims.len() >= 2 => elems as f64 * b,
            _ => self.h.tensor_bytes(t)? as f64,
        };
        Ok((bytes, elems as f64))
    }
}

impl DecodeStep {
    /// The step of `d` over the checkpoint `h` describes.
    pub fn of(h: &GgufHeader, d: &Deployment) -> Result<DecodeStep, DecodeRefusal> {
        let r = Reader { h, d };
        let blocks = h.require("block_count")? as u32;
        let drafts = h.meta_u64("nextn_predict_layers").unwrap_or(0) as u32;
        let embd = h.require("embedding_length")?;
        let heads = h.require("attention.head_count")?;
        let kv_heads = h.require("attention.head_count_kv")?;
        let key_len = h.meta_u64("attention.key_length").unwrap_or(embd / heads);
        let value_len = h.meta_u64("attention.value_length").unwrap_or(embd / heads);
        let routed = match (h.meta_u64("expert_count"), h.meta_u64("expert_used_count")) {
            (Some(n), Some(k)) if n > 0 => k as f64 / n as f64,
            _ => 1.0,
        };

        let mut globals: BTreeMap<&'static str, &TensorRow> = BTreeMap::new();
        let mut per_block: BTreeMap<u32, Vec<(Role, &TensorRow)>> = BTreeMap::new();
        for t in &h.tensors {
            h.tensor_type(t)?;
            if let Some(role) = global_role(&t.name) {
                let key = match role {
                    Role::Embed => "embed",
                    Role::RopeFreqs => "rope",
                    Role::OutputNorm => "output_norm",
                    _ => "output",
                };
                globals.insert(key, t);
                continue;
            }
            let Some((b, key)) = split_block_name(&t.name) else {
                return Err(DecodeRefusal::UnknownTensor(t.name.clone()));
            };
            let Some(role) = block_role(key) else {
                return Err(DecodeRefusal::UnknownTensor(t.name.clone()));
            };
            if b >= blocks {
                return Err(DecodeRefusal::Unreadable(format!("`{}` is past block_count {blocks}", t.name)));
            }
            per_block.entry(b).or_default().push((role, t));
        }

        let mut stages = Vec::new();
        let mut held: BTreeMap<HeldClass, (f64, f64)> = BTreeMap::new();
        let mut hold = |class: HeldClass, bytes: f64, moved: f64| {
            let e = held.entry(class).or_insert((0.0, 0.0));
            e.0 += bytes;
            e.1 += moved;
        };
        let mut weights_held = 0.0;
        let embed = globals
            .get("embed")
            .ok_or_else(|| DecodeRefusal::Unreadable("no token_embd.weight".into()))?;
        let (table_bytes, _) = r.stored(embed, Role::Embed)?;
        let vocab = *embed.dims.get(1).ok_or_else(|| DecodeRefusal::Unreadable("a one-dimensional embedding".into()))?;
        let mut embed_stage = stage(None, StageKind::Embed, Split::One);
        embed_stage.tensors.push(embed.name.clone());
        embed_stage.weight_bytes = table_bytes / vocab as f64;
        // RoPE's frequency table is a weight every attention block rotates by: a token reads it once, with the first.
        let mut rope = globals.get("rope").copied();
        hold(HeldClass::EmbeddingTable, table_bytes, 0.0);
        stages.push(embed_stage);

        let reduce_bytes = embd as f64 * d.activation_bytes;
        for b in 0..blocks {
            let draft = b >= blocks.saturating_sub(drafts);
            if draft && !d.speculative {
                continue;
            }
            let Some(tensors) = per_block.get(&b) else { continue };
            if !draft {
                if let Some((_, t)) = tensors.iter().find(|(role, _)| matches!(role, Role::DraftNorm | Role::DraftIn | Role::DraftHeadNorm)) {
                    return Err(DecodeRefusal::Unreadable(format!("`{}` is a draft tensor in block {b}, which is not a draft block", t.name)));
                }
            }
            let of = |want: &[Role]| -> Vec<(Role, &TensorRow)> { tensors.iter().filter(|(role, _)| want.contains(role)).copied().collect() };
            let has = |key: &str| tensors.iter().any(|(_, t)| split_block_name(&t.name).is_some_and(|(_, k)| k == key));
            let recurrent = tensors.iter().any(|(_, t)| split_block_name(&t.name).is_some_and(|(_, k)| k.starts_with("ssm_")));
            let attention = has("attn_q") || has("attn_k") || has("attn_qkv");

            let push = |kind: StageKind, split: Split, reduces: bool, rows: Vec<(Role, &TensorRow)>, stages: &mut Vec<Stage>, weights_held: &mut f64| -> Result<(), DecodeRefusal> {
                if rows.is_empty() {
                    return Ok(());
                }
                let mut s = stage(Some(b), kind, split);
                for (role, t) in rows {
                    let (bytes, elems) = r.stored(t, role)?;
                    let share = if matches!(role, Role::FfnInExperts | Role::FfnOutExperts) { routed } else { 1.0 };
                    *weights_held += bytes;
                    s.weight_bytes += bytes * share;
                    if is_matrix_role(role) {
                        s.ops += 2.0 * elems * share;
                    }
                    s.tensors.push(t.name.clone());
                }
                s.reduces = reduces;
                if reduces {
                    s.reduce_bytes = reduce_bytes;
                }
                stages.push(s);
                Ok(())
            };

            if draft {
                push(StageKind::Norm, Split::Replicated, false, of(&[Role::DraftNorm]), &mut stages, &mut weights_held)?;
                push(StageKind::Project, Split::Any, false, of(&[Role::DraftIn]), &mut stages, &mut weights_held)?;
            }
            push(StageKind::Norm, Split::Replicated, false, of(&[Role::PreMixerNorm]), &mut stages, &mut weights_held)?;
            push(StageKind::Project, Split::Any, false, of(&[Role::MixerIn]), &mut stages, &mut weights_held)?;

            let mut mixer = if recurrent {
                let v_heads = tensors
                    .iter()
                    .find(|(_, t)| split_block_name(&t.name).is_some_and(|(_, k)| k == "ssm_a"))
                    .and_then(|(_, t)| t.dims.first().copied())
                    .ok_or_else(|| DecodeRefusal::Unreadable(format!("block {b}: a recurrence without ssm_a")))?;
                let head_v = tensors
                    .iter()
                    .find(|(_, t)| split_block_name(&t.name).is_some_and(|(_, k)| k == "ssm_norm"))
                    .and_then(|(_, t)| t.dims.first().copied())
                    .ok_or_else(|| DecodeRefusal::Unreadable(format!("block {b}: a recurrence without ssm_norm")))?;
                let head_k = h.require("ssm.state_size")?;
                let conv_kernel = h.require("ssm.conv_kernel")?;
                let conv_channels = tensors
                    .iter()
                    .find(|(_, t)| split_block_name(&t.name).is_some_and(|(_, k)| k == "ssm_conv1d"))
                    .and_then(|(_, t)| t.dims.get(1).copied())
                    .ok_or_else(|| DecodeRefusal::Unreadable(format!("block {b}: a recurrence without ssm_conv1d")))?;
                let state = (v_heads * head_k * head_v + (conv_kernel - 1) * conv_channels) as f64 * d.state_bytes;
                let mut s = stage(Some(b), StageKind::Recurrence, Split::Heads { q: v_heads, kv: v_heads });
                s.state_read = state;
                s.state_write = state;
                // Decay, rank-one update and readout: one multiply-add per state element each; the depthwise
                // convolution one per tap per channel.
                s.ops = 2.0 * (3 * v_heads * head_k * head_v + conv_kernel * conv_channels) as f64;
                hold(HeldClass::RecurrentState, state, 2.0 * state);
                s
            } else if attention {
                let per_token = (kv_heads * (key_len + value_len)) as f64 * d.kv_bytes;
                let mut s = stage(Some(b), StageKind::Attention, Split::Heads { q: heads, kv: kv_heads });
                s.state_read = per_token * d.ctx as f64;
                s.state_write = per_token;
                s.ops = 2.0 * (heads * d.ctx * (key_len + value_len)) as f64;
                hold(HeldClass::KvCache, per_token * d.ctx as f64, s.state_read + s.state_write);
                s
            } else {
                return Err(DecodeRefusal::Unreadable(format!("block {b}: neither attention nor a recurrence")));
            };
            let first_rotation = rope.filter(|_| mixer.kind == StageKind::Attention);
            for (role, t) in of(&[Role::MixerParam]).into_iter().chain(first_rotation.map(|t| (Role::RopeFreqs, t))) {
                let bytes = r.stored(t, role)?.0;
                weights_held += bytes;
                mixer.weight_bytes += bytes;
                mixer.tensors.push(t.name.clone());
            }
            if first_rotation.is_some() {
                rope = None;
            }
            stages.push(mixer);

            push(StageKind::Project, Split::Any, true, of(&[Role::MixerOut]), &mut stages, &mut weights_held)?;
            push(StageKind::Norm, Split::Replicated, false, of(&[Role::PostMixerNorm]), &mut stages, &mut weights_held)?;
            push(StageKind::Norm, Split::Replicated, false, of(&[Role::PreFfnNorm]), &mut stages, &mut weights_held)?;
            push(StageKind::Router, Split::Replicated, false, of(&[Role::Router]), &mut stages, &mut weights_held)?;
            push(StageKind::Project, Split::Any, false, of(&[Role::FfnIn, Role::FfnInExperts]), &mut stages, &mut weights_held)?;
            push(StageKind::Project, Split::Any, true, of(&[Role::FfnOut, Role::FfnOutExperts]), &mut stages, &mut weights_held)?;
            push(StageKind::Norm, Split::Replicated, false, of(&[Role::PostFfnNorm]), &mut stages, &mut weights_held)?;
            if draft {
                push(StageKind::Norm, Split::Replicated, false, of(&[Role::DraftHeadNorm]), &mut stages, &mut weights_held)?;
            }
        }

        if let Some(t) = rope {
            return Err(DecodeRefusal::Unreadable(format!("`{}` with no attention block that rotates by it", t.name)));
        }
        let output_norm = globals
            .get("output_norm")
            .ok_or_else(|| DecodeRefusal::Unreadable("no output_norm.weight".into()))?;
        let mut norm = stage(None, StageKind::Norm, Split::Replicated);
        norm.weight_bytes = r.stored(output_norm, Role::OutputNorm)?.0;
        weights_held += norm.weight_bytes;
        norm.tensors.push(output_norm.name.clone());
        stages.push(norm);

        // A checkpoint without `output.weight` ties its head to the embedding table: the head reads all of it.
        let (head_tensor, head_role) = match globals.get("output") {
            Some(t) => (*t, Role::Output),
            None => (*embed, Role::Embed),
        };
        let (head_bytes, head_elems) = r.stored(head_tensor, head_role)?;
        let mut head = stage(None, StageKind::Head, Split::Any);
        head.weight_bytes = head_bytes;
        head.ops = 2.0 * head_elems;
        head.tensors.push(head_tensor.name.clone());
        head.reduces = true;
        // The arg-max across vocabulary shards carries a value and an index.
        head.reduce_bytes = 8.0;
        // A tied head reads the embedding table, which is already held: its bytes stream from wherever the table
        // lives.
        if head_role == Role::Embed {
            head.weight_class = HeldClass::EmbeddingTable;
        } else {
            weights_held += head_bytes;
        }
        stages.push(head);

        let moved = |class: HeldClass| -> f64 { stages.iter().filter(|s| s.weight_class == class).map(|s| s.weight_bytes).sum() };
        let (weights_moved, table_moved) = (moved(HeldClass::Weights), moved(HeldClass::EmbeddingTable));
        hold(HeldClass::Weights, weights_held, weights_moved);
        hold(HeldClass::EmbeddingTable, 0.0, table_moved);
        let held = held
            .into_iter()
            .map(|(class, (bytes, moved_per_token))| Held { class, bytes, moved_per_token })
            .collect();
        Ok(DecodeStep { deployment: *d, stages, held })
    }

    pub fn weight_bytes(&self) -> f64 {
        self.stages.iter().map(|s| s.weight_bytes).sum()
    }

    pub fn state_read_bytes(&self) -> f64 {
        self.stages.iter().map(|s| s.state_read).sum()
    }

    pub fn state_write_bytes(&self) -> f64 {
        self.stages.iter().map(|s| s.state_write).sum()
    }

    pub fn ops(&self) -> f64 {
        self.stages.iter().map(|s| s.ops).sum()
    }

    /// Every byte the deployment keeps resident.
    pub fn held_bytes(&self) -> f64 {
        self.held.iter().map(|h| h.bytes).sum()
    }
}

fn stage(layer: Option<u32>, kind: StageKind, split: Split) -> Stage {
    Stage {
        layer,
        kind,
        tensors: Vec::new(),
        weight_bytes: 0.0,
        weight_class: match kind {
            StageKind::Embed => HeldClass::EmbeddingTable,
            _ => HeldClass::Weights,
        },
        state_read: 0.0,
        state_write: 0.0,
        ops: 0.0,
        split,
        reduces: false,
        reduce_bytes: 0.0,
    }
}

// ── The machine ─────────────────────────────────────────────────────────────────────────────────────

/// A kind of memory a chip holds bytes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum TierKind {
    Sram,
    Hbm,
    Gddr,
    Lpddr,
    Ddr,
}

/// One memory tier of a chip: how much it holds and how fast it streams.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Tier {
    pub kind: TierKind,
    /// Bytes.
    pub capacity: Fact<f64>,
    /// Bytes per second.
    pub bandwidth: Fact<f64>,
}

/// The chip-to-chip network. Either rate may be undeclared: the floor then prices it at zero, which is a true
/// lower bound, and the prediction refuses.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Link {
    /// Bytes per second one chip injects.
    pub bandwidth: Option<Fact<f64>>,
    /// Seconds one hop of a collective takes, end to end.
    pub hop_latency: Option<Fact<f64>>,
}

/// A chip as a decode sees it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Chip {
    pub name: &'static str,
    /// Fastest first.
    pub tiers: Vec<Tier>,
    /// Dense matrix-engine operations per second, by operand precision.
    pub matrix: Vec<(Precision, Fact<f64>)>,
    /// What a dependent stage costs beyond its roofline: a kernel launch, a pipeline sync. `None` is
    /// undeclared, NOT zero.
    pub stage_overhead: Option<Fact<f64>>,
    pub link: Option<Link>,
    /// Watts.
    pub power: Option<Fact<f64>>,
    pub sources: &'static [&'static str],
}

const NV_RTX3060_PAGE: &str = "nvidia.com/en-us/geforce/graphics-cards/30-series/rtx-3060-3060ti";
const GROQ_SPEC_SHEET: &str = "groq.com/product-spec-sheet-groqchip-processor";
const CEREBRAS_LAUNCH: &str = "cerebras.ai/blog/introducing-cerebras-inference-ai-at-instant-speed";

const RTX3060_SOURCES: &[&str] = &[
    NV_RTX3060_PAGE,
    "nvidia.com/content/PDF/nvidia-ampere-ga-102-gpu-architecture-whitepaper-v2.pdf",
];
const A100_SOURCES: &[&str] = &[super::roofs::NV_A100_PAGE];
const H100_SXM_SOURCES: &[&str] = &[super::roofs::NV_H100_PAGE];
const H100_NVL_SOURCES: &[&str] = &[super::roofs::NV_H100_PAGE, super::roofs::NV_H100_NVL_BRIEF];
const B200_SOURCES: &[&str] = &[super::roofs::NV_DGX_B200_PAGE, super::roofs::NV_HGX_PAGE];
const GROQ_SOURCES: &[&str] = &[GROQ_SPEC_SHEET];
const CEREBRAS_SOURCES: &[&str] = &[CEREBRAS_LAUNCH];

/// A GPU whose rates are its roofs preset's: one memory tier and the matrix roofs, converted per second with
/// the roof's own citation.
fn gpu(
    name: &'static str,
    roofs: Roofs,
    kind: TierKind,
    capacity: Fact<f64>,
    link: Option<Link>,
    power: Option<Fact<f64>>,
    sources: &'static [&'static str],
) -> Chip {
    let dram = roofs
        .mem(MemLevel::Dram, StreamMix::Any)
        .expect("a GPU roofs preset prices its memory");
    let per_second = |rate: &Fact<Rate>| -> Fact<f64> {
        let c = roofs.per_second(rate);
        Fact { value: c.value, provenance: c.provenance, cite: rate.cite }
    };
    Chip {
        name,
        tiers: vec![Tier { kind, capacity, bandwidth: per_second(&dram.rate) }],
        matrix: roofs
            .compute
            .iter()
            .filter(|c| c.engine == Engine::Matrix)
            .map(|c| (c.precision, per_second(&c.ops)))
            .collect(),
        stage_overhead: None,
        link,
        power,
        sources,
    }
}

fn nvlink(bytes_per_s: f64, cite: &'static str) -> Option<Link> {
    Some(Link {
        bandwidth: Some(Fact::published(bytes_per_s, cite)),
        hop_latency: None,
    })
}

impl Chip {
    /// The local RTX 3060, the one GPU whose per-stage overhead is measured here.
    pub fn rtx3060() -> Chip {
        Chip {
            stage_overhead: Some(Fact::measured(
                0.559e-6,
                "benchmarks/silicon/decode/gpu_stage_overhead.json: graph_node_us_median 0.559 — the slope of a CUDA graph's \
                 replay time over the empty kernel nodes it chains (scripts/accel/graph_node_overhead.cu), three runs on an idle GPU",
            )),
            ..gpu(
                "rtx3060",
                Roofs::rtx3060(),
                TierKind::Gddr,
                Fact::published(12e9, "nvidia.com/en-us/geforce/graphics-cards/30-series/rtx-3060-3060ti: RTX 3060 \"12 GB GDDR6\""),
                None,
                None,
                RTX3060_SOURCES,
            )
        }
    }

    pub fn a100_sxm4_80gb() -> Chip {
        gpu(
            "a100-sxm4-80gb",
            Roofs::a100_sxm4_80gb(),
            TierKind::Hbm,
            Fact::published(80e9, "nvidia.com/en-us/data-center/a100: A100 80GB SXM \"GPU Memory 80GB HBM2e\""),
            nvlink(600e9, "nvidia.com/en-us/data-center/a100: A100 80GB SXM \"Interconnect NVLink: 600 GB/s\""),
            Some(Fact::published(400.0, "nvidia.com/en-us/data-center/a100: A100 80GB SXM \"Max Thermal Design Power (TDP) 400W\"")),
            A100_SOURCES,
        )
    }

    pub fn h100_sxm() -> Chip {
        gpu(
            "h100-sxm",
            Roofs::h100_sxm(),
            TierKind::Hbm,
            Fact::published(80e9, "nvidia.com/en-us/data-center/h100: H100 SXM \"GPU Memory 80GB\""),
            nvlink(900e9, "nvidia.com/en-us/data-center/h100: H100 SXM \"Interconnect NVIDIA NVLink: 900GB/s\""),
            Some(Fact::published(700.0, "nvidia.com/en-us/data-center/h100: H100 SXM \"Max Thermal Design Power (TDP) Up to 700W (configurable)\"")),
            H100_SXM_SOURCES,
        )
    }

    pub fn h100_nvl() -> Chip {
        gpu(
            "h100-nvl",
            Roofs::h100_nvl(),
            TierKind::Hbm,
            Fact::published(94e9, "nvidia.com/content/dam/en-zz/Solutions/Data-Center/h100/PB-11773-001_v01.pdf: Table 2 \"Memory size 94 GB\""),
            nvlink(600e9, "nvidia.com/en-us/data-center/h100: H100 NVL \"Interconnect NVIDIA NVLink: 600GB/s\""),
            Some(Fact::published(400.0, "nvidia.com/content/dam/en-zz/Solutions/Data-Center/h100/PB-11773-001_v01.pdf: Table 1 \"Total board power … 400 W maximum (default)\"")),
            H100_NVL_SOURCES,
        )
    }

    /// One GPU of a DGX/HGX B200.
    pub fn b200() -> Chip {
        gpu(
            "b200",
            Roofs::b200(),
            TierKind::Hbm,
            Fact::derived(1440e9 / 8.0, "nvidia.com/en-us/data-center/dgx-b200: \"8x NVIDIA Blackwell GPUs\", \"1,440 GB total, 64 TB/s HBM3e bandwidth\"; 1,440 GB over 8 GPUs"),
            nvlink(1.8e12, "nvidia.com/en-us/data-center/hgx: HGX B200 \"NVIDIA NVLink GPU-to-GPU Bandwidth 1.8 TB/s\""),
            None,
            B200_SOURCES,
        )
    }

    /// The first-generation GroqChip: weights live in its on-die SRAM. Its chip-to-chip links are counted but
    /// their rates are not published.
    pub fn groq_lpu_v1() -> Chip {
        Chip {
            name: "groq-lpu-v1",
            tiers: vec![Tier {
                kind: TierKind::Sram,
                capacity: Fact::published(230e6, "groq.com/product-spec-sheet-groqchip-processor (GroqChip Processor v1.5): \"230 MB SRAM per chip\""),
                bandwidth: Fact::published(80e12, "groq.com/product-spec-sheet-groqchip-processor (GroqChip Processor v1.5): \"Up to 80 TB/s on-die memory bandwidth\""),
            }],
            matrix: vec![
                (Precision::Int8, Fact::published(750e12, "groq.com/product-spec-sheet-groqchip-processor (GroqChip Processor v1.5): \"Up to 750 TOPs, 188 TFLOPs (INT8, FP16 @900 MHz)\"")),
                (Precision::Fp16, Fact::published(188e12, "groq.com/product-spec-sheet-groqchip-processor (GroqChip Processor v1.5): \"Up to 750 TOPs, 188 TFLOPs (INT8, FP16 @900 MHz)\"")),
            ],
            stage_overhead: None,
            link: Some(Link { bandwidth: None, hop_latency: None }),
            power: Some(Fact::published(215.0, "groq.com/product-spec-sheet-groqchip-processor (GroqChip Processor v1.5): \"Power Max: 300W; TDP: 215W; Average: 185W\"")),
            sources: GROQ_SOURCES,
        }
    }

    /// Cerebras WSE-3, one wafer. Its matrix rate is published without a precision, so it is not priced.
    pub fn cerebras_wse3() -> Chip {
        Chip {
            name: "cerebras-wse3",
            tiers: vec![Tier {
                kind: TierKind::Sram,
                capacity: Fact::published(44e9, "cerebras.ai/blog/introducing-cerebras-inference-ai-at-instant-speed: \"44GB of SRAM on a single chip\""),
                bandwidth: Fact::published(21e15, "cerebras.ai/blog/introducing-cerebras-inference-ai-at-instant-speed: \"21 petabytes/s of aggregate memory bandwidth\""),
            }],
            matrix: Vec::new(),
            stage_overhead: None,
            link: Some(Link { bandwidth: None, hop_latency: None }),
            power: None,
            sources: CEREBRAS_SOURCES,
        }
    }

    /// Every chip a decode is priced on.
    pub fn catalog() -> Vec<Chip> {
        vec![
            Chip::rtx3060(),
            Chip::a100_sxm4_80gb(),
            Chip::h100_sxm(),
            Chip::h100_nvl(),
            Chip::b200(),
            Chip::groq_lpu_v1(),
            Chip::cerebras_wse3(),
            super::fpga_decode::zcu104(),
            super::d5005::chip(),
        ]
    }

    pub fn by_name(name: &str) -> Option<Chip> {
        Chip::catalog().into_iter().find(|c| c.name == name)
    }

    /// The dense matrix rate at `p`.
    pub fn matrix_rate(&self, p: Precision) -> Option<Fact<f64>> {
        self.matrix.iter().find(|(q, _)| *q == p).map(|(_, f)| *f)
    }

    /// Every fact the chip carries, with the path a lock names it by.
    pub fn facts(&self) -> Vec<(String, Provenance, &'static str)> {
        let mut out = Vec::new();
        for t in &self.tiers {
            out.push((format!("tier.{:?}.capacity", t.kind), t.capacity.provenance, t.capacity.cite));
            out.push((format!("tier.{:?}.bandwidth", t.kind), t.bandwidth.provenance, t.bandwidth.cite));
        }
        for (p, f) in &self.matrix {
            out.push((format!("matrix.{p:?}"), f.provenance, f.cite));
        }
        if let Some(f) = &self.stage_overhead {
            out.push(("stage_overhead".into(), f.provenance, f.cite));
        }
        if let Some(l) = &self.link {
            if let Some(f) = &l.bandwidth {
                out.push(("link.bandwidth".into(), f.provenance, f.cite));
            }
            if let Some(f) = &l.hop_latency {
                out.push(("link.hop_latency".into(), f.provenance, f.cite));
            }
        }
        if let Some(f) = &self.power {
            out.push(("power".into(), f.provenance, f.cite));
        }
        out
    }
}

/// A number of identical chips running one step, tensor-parallel.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Machine {
    pub chip: Chip,
    pub chips: u32,
}

impl Machine {
    pub fn of(chip: Chip, chips: u32) -> Machine {
        assert!(chips >= 1, "a machine has a chip");
        Machine { chip, chips }
    }
}

// ── The price ───────────────────────────────────────────────────────────────────────────────────────

/// The term that binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Bind {
    Memory,
    Compute,
    Collective,
    Overhead,
}

/// One stage's terms, in seconds, on the machine it was priced for.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct StageCost {
    /// Over how many chips the stage's arithmetic divides.
    pub ways: u32,
    pub memory_s: f64,
    pub compute_s: f64,
    /// The stage's collective with undeclared terms at zero: what the floor counts.
    pub collective_floor_s: f64,
    /// The collective with every term declared, or `None`.
    pub collective_s: Option<f64>,
    pub overhead_s: Option<f64>,
}

/// One token of a step on a machine.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecodeCost {
    pub machine: &'static str,
    pub chips: u32,
    pub stages: Vec<StageCost>,
    /// No implementation decodes a token faster: the largest of the summed memory, compute and collective
    /// terms, with undeclared terms at zero.
    pub floor_s: f64,
    pub floor_binds: Bind,
    /// Each stage's roofline plus its collective and its overhead, summed along the chain — `None` while any
    /// term it needs is undeclared.
    pub time_s: Option<f64>,
    /// Reductions across chips per token.
    pub collectives: usize,
    /// Of each tier, the share of the held bytes it keeps, per chip.
    pub residency: Vec<(TierKind, f64)>,
    /// The weakest fact the price rests on.
    pub provenance: Provenance,
}

impl DecodeCost {
    pub fn floor_tokens_per_s(&self) -> f64 {
        1.0 / self.floor_s
    }

    pub fn tokens_per_s(&self) -> Option<f64> {
        self.time_s.map(|t| 1.0 / t)
    }
}

/// The fastest collective of `bytes` over `p` chips the link allows: recursive doubling (`⌈log₂ p⌉` hops,
/// each carrying the whole vector) or a ring (`2(p−1)` hops of a `1/p` share). Undeclared rates count zero.
fn collective(p: u32, bytes: f64, link: Option<&Link>) -> (f64, Option<f64>) {
    if p <= 1 {
        return (0.0, Some(0.0));
    }
    let lat = link.and_then(|l| l.hop_latency).map(|f| f.value);
    let bw = link.and_then(|l| l.bandwidth).map(|f| f.value);
    let cost = |lat: f64, inv_bw: f64| -> f64 {
        let steps = (p as f64).log2().ceil();
        let doubling = steps * (lat + bytes * inv_bw);
        let ring = 2.0 * (p as f64 - 1.0) * (lat + bytes / p as f64 * inv_bw);
        doubling.min(ring)
    };
    let floor = cost(lat.unwrap_or(0.0), bw.map_or(0.0, |b| 1.0 / b));
    let declared = match (lat, bw) {
        (Some(l), Some(b)) => Some(cost(l, 1.0 / b)),
        _ => None,
    };
    (floor, declared)
}

/// Prices `step` on `m`, with matrix arithmetic at `precision`.
pub fn price(step: &DecodeStep, m: &Machine, precision: Precision) -> Result<DecodeCost, DecodeRefusal> {
    let chip = &m.chip;
    let p = m.chips;

    // Residency: fill the fastest tier first with what a token moves the most of per byte held.
    let mut classes = step.held.clone();
    classes.sort_by(|a, b| (b.moved_per_token / b.bytes.max(1.0)).total_cmp(&(a.moved_per_token / a.bytes.max(1.0))));
    let needed = step.held_bytes();
    let capacity: f64 = chip.tiers.iter().map(|t| t.capacity.value).sum::<f64>() * p as f64;
    if needed > capacity {
        return Err(DecodeRefusal::NotResident { needed, capacity });
    }
    let mut free: Vec<f64> = chip.tiers.iter().map(|t| t.capacity.value * p as f64).collect();
    // For each class, the share of it each tier holds.
    let mut share: BTreeMap<HeldClass, Vec<f64>> = BTreeMap::new();
    for c in &classes {
        let mut left = c.bytes;
        let mut row = vec![0.0; chip.tiers.len()];
        for (i, f) in free.iter_mut().enumerate() {
            let take = left.min(*f);
            *f -= take;
            left -= take;
            row[i] = if c.bytes > 0.0 { take / c.bytes } else { (i == 0) as u8 as f64 };
        }
        share.insert(c.class, row);
    }
    let residency = chip
        .tiers
        .iter()
        .zip(free.iter())
        .map(|(t, f)| (t.kind, (t.capacity.value * p as f64 - f) / p as f64))
        .collect();
    let seconds = |bytes: f64, class: HeldClass| -> f64 {
        share[&class]
            .iter()
            .zip(&chip.tiers)
            .map(|(s, t)| bytes * s / t.bandwidth.value)
            .sum()
    };

    let rate = chip.matrix_rate(precision);
    let link = chip.link.as_ref();
    let mut provenance = chip.tiers.iter().map(|t| t.bandwidth.provenance).max().unwrap_or(Provenance::Published);
    if let Some(r) = rate {
        provenance = provenance.max(r.provenance);
    }
    let mut costs = Vec::with_capacity(step.stages.len());
    let mut collectives = 0;
    for s in &step.stages {
        let (ways, state_ways) = match s.split {
            Split::Any => (p, p),
            Split::Heads { q, kv } => (p.min(q as u32), p.min(kv as u32)),
            Split::Replicated | Split::One => (1, 1),
        };
        let state_class = match s.kind {
            StageKind::Recurrence => HeldClass::RecurrentState,
            _ => HeldClass::KvCache,
        };
        let memory_s = seconds(s.weight_bytes / ways as f64, s.weight_class)
            + if s.state_read + s.state_write > 0.0 {
                seconds((s.state_read + s.state_write) / state_ways as f64, state_class)
            } else {
                0.0
            };
        let compute_s = rate.map_or(0.0, |r| s.ops / ways as f64 / r.value);
        let (collective_floor_s, collective_s) = if s.reduces && ways > 1 {
            collectives += 1;
            collective(ways, s.reduce_bytes, link)
        } else {
            (0.0, Some(0.0))
        };
        costs.push(StageCost {
            ways,
            memory_s,
            compute_s,
            collective_floor_s,
            collective_s,
            overhead_s: chip.stage_overhead.map(|f| f.value),
        });
    }

    let memory: f64 = costs.iter().map(|c| c.memory_s).sum();
    let compute: f64 = costs.iter().map(|c| c.compute_s).sum();
    let collective_sum: f64 = costs.iter().map(|c| c.collective_floor_s).sum();
    let (floor_s, floor_binds) = [(memory, Bind::Memory), (compute, Bind::Compute), (collective_sum, Bind::Collective)]
        .into_iter()
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .expect("three terms");
    let time_s = costs
        .iter()
        .map(|c| Some(c.memory_s.max(c.compute_s) + c.collective_s? + c.overhead_s?))
        .sum::<Option<f64>>();
    if time_s.is_some() {
        if let Some(o) = chip.stage_overhead {
            provenance = provenance.max(o.provenance);
        }
        if p > 1 {
            if let Some(l) = link {
                for f in [l.bandwidth, l.hop_latency].into_iter().flatten() {
                    provenance = provenance.max(f.provenance);
                }
            }
        }
    }
    Ok(DecodeCost {
        machine: chip.name,
        chips: p,
        stages: costs,
        floor_s,
        floor_binds,
        time_s,
        collectives,
        residency,
        provenance,
    })
}
