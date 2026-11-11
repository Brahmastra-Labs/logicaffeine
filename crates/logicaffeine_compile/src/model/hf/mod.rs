//! A Hugging Face checkpoint directory read in place: `config.json` (its `text_config` merged over it as a multimodal
//! checkpoint nests it), the safetensors shards its index names, and the tokenizer files. It is presented as the GGUF
//! llama.cpp's converter (`convert_hf_to_gguf.py`) writes from it — the same architecture keys, tensor names, ggml dim
//! order and element order — so [`ModelSpec::from_source`](super::spec::ModelSpec::from_source), the binder and the
//! reference read it unchanged.
//!
//! Nothing is read whole: opening reads the shards' headers, and a tensor's rows are read when they are asked for. Most
//! tensors are their bytes as stored, dims reversed into ggml's order; a dtype other than f32, f16 and bf16 (the
//! 8-bit floats, the integers) is widened to f32, as the converter widens it. The converter's rearrangements are views
//! over the stored bytes, kept in the checkpoint's own dtype:
//!
//!   • a row permutation — Llama's query and key rows into interleaved rotation pairs; the value heads of a Gated
//!     DeltaNet block from grouped-by-key-head to tiled order;
//!   • a column permutation or pick — the same reorder over an output projection's inputs, gpt-oss's interleaved
//!     gate and up halves;
//!   • a transpose of the last two axes — gpt-oss's expert matrices, latent attention's key half;
//!   • a stack — a mixture's experts, one tensor per projection.
//!
//! Two converter steps change values: a norm stored as an offset from one (Gemma 3, Qwen3.5: the forward multiplies
//! by `1 + w`) and Gated DeltaNet's decay stored as its logarithm (`A = -exp(A_log)`). The converter computes them in
//! the dtype it holds the tensor in — f16 and f32 as stored, every other dtype widened to f32 first — and the
//! checkpoint's own forward computes them in f32 from the stored value. The two differ only for an f16 checkpoint,
//! whose converter rounds each result to f16; [`Recompute`] chooses which the reader presents: the converter's by
//! default, so a directory reads as its converted GGUF, or the forward's. Either way `1 + w` is exact and the
//! exponential correctly rounded; the converter's own exponential is torch's on the CPU (MKL's on x86 builds, chosen
//! by the processor), which lands one unit in the last place off the correctly rounded value now and then.
//!
//! A configuration key is either read, known to leave the computation unchanged, or refused by name: a checkpoint is
//! never read as something other than what it says it is.
//!
//! The tokenizer files are read as the converter reads them ([`HfCheckpoint::tokenizer_meta`]): the vocabulary, merges,
//! special tokens and chat template it writes, the byte-pair pre-tokenizer named by the digest the converter's own
//! probe gives under the directory's `tokenizer.json`, run as the `tokenizers` library runs it ([`HfTokenizer`]).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{Map, Value};

use super::gguf::MetaValue;
use super::quant::{e4m3fn_to_f32, e5m2_to_f32, f16_to_f32, f32_to_f16, GgmlType, ScaleGrid};
use super::reference::WeightSource;
use super::safetensors::{self, Dtype, Entry, Shard};
use super::spec::SpecSource;

mod encode;
mod onig;
mod tiktoken;
mod unicode_data;
mod vocab;

pub use encode::{nfc, HfPreTokenizer, HfTokenizer};
pub use tiktoken::{split as tiktoken_split, Tiktoken};
pub use unicode_data::{NORMAL_FORM_VERSION, PATTERN_VERSION};
pub use vocab::{pre_tokenizer_named, PRE_TOKENIZER_DIGESTS, PRE_TOKENIZER_PROBE};

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// An elementwise step the converter applies to a tensor's values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// `1 + w`: a norm weight stored as its offset from one.
    AddOne,
    /// `-exp(w)`: a decay stored as its logarithm.
    NegExp,
    /// `(w·a)·b`: multipliers the converter folds into the weight one after the other, each product rounded as the
    /// converter rounds it (Falcon-H1's MuP multipliers).
    Scale(f32, f32),
}

/// How the values a converter step computes (a norm's offset from one, a decay's logarithm) are presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Recompute {
    /// In the precision the converter computes them in: rounded to f16 for an f16 tensor, f32 otherwise. The converted
    /// GGUF's values.
    #[default]
    AsConverted,
    /// In f32 from the stored value, as the checkpoint's own forward computes them.
    AsForward,
}

/// How a presented tensor's elements are found in the checkpoint.
#[derive(Clone, Debug, PartialEq)]
pub enum Form {
    /// Source tensor `src` viewed as `[outer, m, n]`: output element `(b, i, j)` is source `(b, rows[i], cols[j])`,
    /// or with `transpose` output `(b, j, i)` is.
    Gather {
        src: String,
        outer: usize,
        m: usize,
        n: usize,
        rows: Vec<usize>,
        cols: Vec<usize>,
        transpose: bool,
    },
    /// The forms' elements one after another: a stack along a new outermost axis.
    Concat(Vec<Form>),
    /// The forms side by side: each output row the parts' rows of that index one after another, every part as many rows.
    Beside(Vec<Form>),
    /// `op` applied to the inner form's values, in f32.
    Map { inner: Box<Form>, op: Op },
    /// Values the converter computes rather than reads (Llama 3's rotary factors), in f32.
    Values(Vec<f32>),
    /// `n` zeros of type `ty`: the rows a converter pads a table with (Gemma 3n's embeddings, to the vocabulary).
    Zeros { n: usize, ty: GgmlType },
}

impl Form {
    /// The whole of source `src`, `m × n` (outermost dims folded into `m`).
    fn whole(src: &str, entry: &Entry) -> Form {
        let n = entry.shape.last().copied().unwrap_or(1) as usize;
        let m = (entry.elements() as usize).checked_div(n).unwrap_or(0);
        Form::Gather {
            src: src.to_string(),
            outer: 1,
            m,
            n,
            rows: (0..m).collect(),
            cols: (0..n).collect(),
            transpose: false,
        }
    }

    fn elements(&self) -> usize {
        match self {
            Form::Gather { outer, rows, cols, .. } => outer * rows.len() * cols.len(),
            Form::Concat(parts) | Form::Beside(parts) => parts.iter().map(Form::elements).sum(),
            Form::Map { inner, .. } => inner.elements(),
            Form::Values(v) => v.len(),
            Form::Zeros { n, .. } => *n,
        }
    }
}

/// One tensor as the converted GGUF holds it.
#[derive(Clone, Debug)]
struct View {
    name: String,
    /// ggml order: `dims[0]` the fastest-varying.
    dims: Vec<u64>,
    form: Form,
}

/// A Hugging Face checkpoint directory, presented as its converted GGUF.
pub struct HfCheckpoint {
    dir: PathBuf,
    recompute: Recompute,
    family: Family,
    hparams: Hparams,
    tokenizer: OnceLock<BTreeMap<String, MetaValue>>,
    arch: String,
    meta: BTreeMap<String, MetaValue>,
    views: Vec<View>,
    index: HashMap<String, usize>,
    shards: Vec<Shard>,
    located: HashMap<String, usize>,
    /// An FP8 checkpoint's scaled weights, by name: each read as the converter's `dequant_simple` writes it.
    scaled: HashMap<String, Scaled>,
    /// The tensors the converter removes once it has dequantized: the scales, and activation scales nothing reads.
    consumed: BTreeSet<String>,
    unread: Vec<String>,
    skipped: Vec<String>,
}

/// A weight stored beside its scale (an FP8 checkpoint's `<name>_scale_inv`), and the block its scale entries cover.
struct Scaled {
    scale: String,
    block: Option<Vec<usize>>,
    grid: OnceLock<Result<ScaleGrid, String>>,
}

/// The scaled weights and consumed tensors of a checkpoint whose `quantization_config` is FP8 (`quant_method` fp8), as
/// the converter's `dequant_model` finds them: a `<name>_scale_inv` scales `<name>`, a Mistral `<stem>.qscale_weight`
/// scales `<stem>.weight`, and activation scales (`.activation_scale`, `_activation_scale`, `.qscale_act`) are dropped.
fn fp8_scales(h: &Hparams, located: &HashMap<String, usize>) -> io::Result<(HashMap<String, Scaled>, BTreeSet<String>)> {
    let mut scaled = HashMap::new();
    let mut consumed = BTreeSet::new();
    let Some(q) = h.map.get("quantization_config").filter(|q| q.get("quant_method").and_then(Value::as_str) == Some("fp8")) else {
        return Ok((scaled, consumed));
    };
    let block = match q.get("weight_block_size") {
        None | Some(Value::Null) => None,
        Some(Value::Array(b)) => Some(
            b.iter()
                .map(|x| x.as_u64().map(|x| x as usize).ok_or_else(|| invalid(format!("config.json: weight_block_size {b:?}"))))
                .collect::<io::Result<Vec<usize>>>()?,
        ),
        Some(other) => return Err(invalid(format!("config.json: weight_block_size is {other}"))),
    };
    let mut names: Vec<&String> = located.keys().collect();
    names.sort();
    for name in names {
        let weight = if let Some(w) = name.strip_suffix("_scale_inv") {
            Some(w.to_string())
        } else {
            name.strip_suffix(".qscale_weight").map(|stem| format!("{stem}.weight"))
        };
        if let Some(weight) = weight {
            if !located.contains_key(&weight) {
                return Err(invalid(format!("{name} scales {weight}, which the checkpoint does not hold")));
            }
            scaled.insert(weight, Scaled { scale: name.clone(), block: block.clone(), grid: OnceLock::new() });
            consumed.insert(name.clone());
        } else if name.ends_with(".activation_scale") || name.ends_with("_activation_scale") || name.ends_with(".qscale_act") {
            consumed.insert(name.clone());
        }
    }
    Ok((scaled, consumed))
}

/// The merged hyperparameters, read as the converter's `find_hparam` reads them: the first key present wins, and a
/// present `null` reads as absent.
struct Hparams {
    map: Map<String, Value>,
    /// The checkpoint's class, as the configuration's top level names it (`Qwen3_5MoeForCausalLM`).
    architecture: String,
}

impl Hparams {
    fn find(&self, keys: &[&str]) -> Option<&Value> {
        keys.iter().find_map(|k| self.map.get(*k)).filter(|v| !v.is_null())
    }
    fn u64(&self, keys: &[&str]) -> Option<u64> {
        self.find(keys).and_then(Value::as_u64)
    }
    fn f64(&self, keys: &[&str]) -> Option<f64> {
        self.find(keys).and_then(Value::as_f64)
    }
    fn need_u64(&self, key: &str) -> io::Result<u64> {
        self.u64(&[key]).ok_or_else(|| invalid(format!("config.json: no `{key}`")))
    }
}

/// The GGUF architecture keys the converter writes, in its order of writing; a later write of one key replaces it.
struct Writer {
    arch: String,
    meta: BTreeMap<String, MetaValue>,
}

impl Writer {
    fn put(&mut self, suffix: &str, v: MetaValue) {
        self.meta.insert(format!("{}.{suffix}", self.arch), v);
    }
    fn u32(&mut self, suffix: &str, v: u64) {
        self.put(suffix, MetaValue::U32(v as u32));
    }
    fn f32(&mut self, suffix: &str, v: f64) {
        self.put(suffix, MetaValue::F32(v as f32));
    }
    fn bool(&mut self, suffix: &str, v: bool) {
        self.put(suffix, MetaValue::Bool(v));
    }
    fn str(&mut self, suffix: &str, v: &str) {
        self.put(suffix, MetaValue::Str(v.as_bytes().to_vec()));
    }
}

/// The families this reader presents, by the converter class each checkpoint's architecture selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Llama,
    Qwen2,
    Qwen3,
    Qwen3Moe,
    Qwen2Moe,
    Qwen35,
    Qwen35Moe,
    Qwen3Next,
    Gemma,
    Gemma2,
    Gemma3,
    GptOss,
    Deepseek2,
    Llama4,
    MinimaxM2,
    Phi3,
    Glm4,
    Glm4Moe,
    Qwen4Exp,
    Olmo2,
    Mistral3,
    PhiMoe,
    Deepseek32,
    GlmDsa,
    Lfm2,
    Mamba2,
    FalconH1,
    NemotronH,
    Jamba,
    Gemma3n,
    KimiLinear,
    Minimax01,
    Rwkv7,
    Gemma4,
}

impl Family {
    fn of(architecture: &str) -> Option<Family> {
        Some(match architecture {
            "LlamaForCausalLM" | "MistralForCausalLM" | "MixtralForCausalLM" => Family::Llama,
            "Qwen2ForCausalLM" => Family::Qwen2,
            "Qwen3ForCausalLM" => Family::Qwen3,
            "Qwen3MoeForCausalLM" => Family::Qwen3Moe,
            "Qwen2MoeForCausalLM" => Family::Qwen2Moe,
            "Qwen3_5ForConditionalGeneration" | "Qwen3_5ForCausalLM" => Family::Qwen35,
            "Qwen3_5MoeForConditionalGeneration" | "Qwen3_5MoeForCausalLM" => Family::Qwen35Moe,
            "Qwen3NextForCausalLM" => Family::Qwen3Next,
            "GemmaForCausalLM" => Family::Gemma,
            "Gemma2ForCausalLM" => Family::Gemma2,
            "Gemma3ForCausalLM" | "Gemma3ForConditionalGeneration" => Family::Gemma3,
            "GptOssForCausalLM" => Family::GptOss,
            "DeepseekV2ForCausalLM" | "DeepseekV3ForCausalLM" => Family::Deepseek2,
            "Llama4ForCausalLM" | "Llama4ForConditionalGeneration" => Family::Llama4,
            "MiniMaxM2ForCausalLM" => Family::MinimaxM2,
            "Phi3ForCausalLM" => Family::Phi3,
            "Glm4ForCausalLM" => Family::Glm4,
            "Glm4MoeForCausalLM" => Family::Glm4Moe,
            "Qwen4ExpForConditionalGeneration" | "Qwen4ExpForCausalLM" => Family::Qwen4Exp,
            "Olmo2ForCausalLM" | "Olmo3ForCausalLM" => Family::Olmo2,
            "Mistral3ForConditionalGeneration" | "Ministral3ForCausalLM" => Family::Mistral3,
            "PhiMoEForCausalLM" => Family::PhiMoe,
            "DeepseekV32ForCausalLM" => Family::Deepseek32,
            "GlmMoeDsaForCausalLM" => Family::GlmDsa,
            "Lfm2ForCausalLM" | "LFM2ForCausalLM" => Family::Lfm2,
            "Mamba2ForCausalLM" => Family::Mamba2,
            "FalconH1ForCausalLM" => Family::FalconH1,
            "NemotronHForCausalLM" => Family::NemotronH,
            "JambaForCausalLM" => Family::Jamba,
            "Gemma3nForCausalLM" | "Gemma3nForConditionalGeneration" => Family::Gemma3n,
            "KimiLinearForCausalLM" => Family::KimiLinear,
            "MiniMaxText01ForCausalLM" | "MiniMaxM1ForCausalLM" | "MiniMaxForCausalLM" => Family::Minimax01,
            "RWKV7ForCausalLM" | "Rwkv7ForCausalLM" => Family::Rwkv7,
            "Gemma4ForCausalLM" | "Gemma4ForConditionalGeneration" => Family::Gemma4,
            _ => return None,
        })
    }

    fn arch(self) -> &'static str {
        match self {
            Family::Llama => "llama",
            Family::Qwen2 => "qwen2",
            Family::Qwen3 => "qwen3",
            Family::Qwen3Moe => "qwen3moe",
            Family::Qwen2Moe => "qwen2moe",
            Family::Qwen35 => "qwen35",
            Family::Qwen35Moe => "qwen35moe",
            Family::Qwen3Next => "qwen3next",
            Family::Gemma => "gemma",
            Family::Gemma2 => "gemma2",
            Family::Gemma3 => "gemma3",
            Family::GptOss => "gpt-oss",
            Family::Deepseek2 => "deepseek2",
            Family::Llama4 => "llama4",
            Family::MinimaxM2 => "minimax-m2",
            Family::Phi3 => "phi3",
            Family::Glm4 => "glm4",
            Family::Glm4Moe => "glm4moe",
            Family::Qwen4Exp => "qwen4exp",
            Family::Olmo2 => "olmo2",
            Family::Mistral3 => "mistral3",
            Family::PhiMoe => "phimoe",
            Family::Deepseek32 => "deepseek32",
            Family::GlmDsa => "glm-dsa",
            Family::Lfm2 => "lfm2",
            Family::Mamba2 => "mamba2",
            Family::FalconH1 => "falcon-h1",
            Family::NemotronH => "nemotron_h",
            Family::Jamba => "jamba",
            Family::Gemma3n => "gemma3n",
            Family::KimiLinear => "kimi-linear",
            Family::Minimax01 => "minimax-01",
            Family::Rwkv7 => "rwkv7",
            Family::Gemma4 => "gemma4",
        }
    }

    /// Qwen3.5's hybrid of gated attention and Gated DeltaNet, dense or a mixture, and Qwen3-Next's: their layout rules.
    fn hybrid(self) -> bool {
        matches!(self, Family::Qwen35 | Family::Qwen35Moe | Family::Qwen3Next | Family::Qwen4Exp)
    }

    /// DeepSeek's latent attention and mixture, as the converter's `DeepseekV2Model` and its subclasses write them:
    /// DeepSeek-V2 and V3, DeepSeek-V3.2 and GLM-5 with their sparse attention.
    fn deepseek(self) -> bool {
        matches!(self, Family::Deepseek2 | Family::Deepseek32 | Family::GlmDsa)
    }

    /// Qwen's mixture: a softmax router beside a shared expert scaled by its own gate.
    fn qwen_mixture(self) -> bool {
        matches!(self, Family::Qwen2Moe | Family::Qwen35Moe | Family::Qwen3Next | Family::Qwen4Exp)
    }
}

/// Keys every family may carry that do not change what the checkpoint computes: identity, provenance, generation
/// defaults, training-only settings, and the multimodal wrapper's token ids and tower configuration (the tower's
/// tensors are skipped as the converter skips them).
const INERT: &[&str] = &[
    "_name_or_path",
    "architectures",
    "model_type",
    "transformers_version",
    "torch_dtype",
    "dtype",
    "use_cache",
    "initializer_range",
    "bos_token_id",
    "eos_token_id",
    "pad_token_id",
    "attention_dropout",
    "output_router_logits",
    "router_aux_loss_coef",
    "router_jitter_noise",
    "pretraining_tp",
    "unsloth_fixed",
    "unsloth_version",
    "boi_token_index",
    "eoi_token_index",
    "image_token_index",
    "mm_tokens_per_image",
    "cache_implementation",
    // GLM-5.2's router precision and the indexer its draft block shares, which the forward of the stack does not read.
    "moe_router_dtype",
    "index_share_for_mtp_iteration",
    "image_token_id",
    "video_token_id",
    "vision_start_token_id",
    "vision_end_token_id",
    "vision_config",
    "text_config",
    // Mistral 3's wrapper: its projector's settings and the tower layer it reads.
    "multimodal_projector_bias",
    "projector_hidden_act",
    "spatial_merge_size",
    "vision_feature_layer",
    // Which of a multimodal checkpoint's towers a server loads (Qwen3.8's); the reader reads the language model alone.
    "language_model_only",
    "tie_word_embeddings",
    "max_window_layers",
    "_sliding_window_pattern",
    "mamba_ssm_dtype",
    "mtp_use_dedicated_embeddings",
    "num_nextn_predict_layers",
    "attention_bias",
    "qkv_bias",
    // Where a checkpoint's code lives, and settings only training reads (DeepSeek's, Kimi's and PhiMoE's).
    "auto_map",
    "aux_loss_alpha",
    "seq_aux",
    "ep_size",
    "input_jitter_noise",
    "hidden_dropout",
    "expert_dropout",
    "block_mlp_init_scale",
    "block_out_init_scale",
    "block_use_xavier_init",
    "conv_use_xavier_init",
    // Mamba-2's training and kernel settings: its time steps' initialization, its residual's precision, its scan's
    // chunks.
    "residual_in_fp32",
    "time_step_rank",
    "time_step_min",
    "time_step_max",
    "time_step_floor",
    "time_step_init_scheme",
    "time_step_scale",
    "rescale_prenorm_residual",
    "chunk_size",
    // `PretrainedConfig`'s own attributes, which transformers 4 serializes whole (Kimi K2's later releases): generation
    // defaults, label maps and framework switches. `pruned_heads`, `add_cross_attention` and `is_encoder_decoder`
    // change the model and are refused unless at their defaults (`validate`).
    "_attn_implementation_autoset",
    "add_cross_attention",
    "bad_words_ids",
    "begin_suppress_tokens",
    "chunk_size_feed_forward",
    "cross_attention_hidden_size",
    "decoder_start_token_id",
    "diversity_penalty",
    "do_sample",
    "early_stopping",
    "encoder_no_repeat_ngram_size",
    "exponential_decay_length_penalty",
    "finetuning_task",
    "forced_bos_token_id",
    "forced_eos_token_id",
    "id2label",
    "is_decoder",
    "is_encoder_decoder",
    "label2id",
    "length_penalty",
    "max_length",
    "min_length",
    "no_repeat_ngram_size",
    "num_beam_groups",
    "num_beams",
    "num_return_sequences",
    "output_attentions",
    "output_hidden_states",
    "output_scores",
    "prefix",
    "problem_type",
    "pruned_heads",
    "remove_invalid_values",
    "repetition_penalty",
    "return_dict",
    "return_dict_in_generate",
    "sep_token_id",
    "suppress_tokens",
    "task_specific_params",
    "temperature",
    "tf_legacy_loss",
    "tie_encoder_decoder",
    "tokenizer_class",
    "top_k",
    "top_p",
    "torchscript",
    "typical_p",
    "use_bfloat16",
];

/// The keys a family's converter reads, beyond [`INERT`].
fn read_keys(f: Family) -> &'static [&'static str] {
    const DECODER: &[&str] = &[
        "hidden_size",
        "intermediate_size",
        "num_hidden_layers",
        "num_attention_heads",
        "num_key_value_heads",
        "head_dim",
        "rms_norm_eps",
        "max_position_embeddings",
        "model_max_length",
        "vocab_size",
        "rope_theta",
        "rope_scaling",
        "rope_parameters",
        "hidden_act",
    ];
    let own: &[&str] = match f {
        Family::Llama => &["mlp_bias", "num_local_experts", "num_experts_per_tok", "sliding_window"],
        Family::Qwen2 => &["use_sliding_window", "sliding_window", "layer_types"],
        Family::Qwen3 => &["use_sliding_window", "sliding_window", "layer_types"],
        Family::Qwen3Moe => &[
            "use_sliding_window",
            "sliding_window",
            "layer_types",
            "moe_intermediate_size",
            "num_experts",
            "num_local_experts",
            "num_experts_per_tok",
            "norm_topk_prob",
            "decoder_sparse_step",
            "mlp_only_layers",
        ],
        Family::Qwen2Moe => &[
            "use_sliding_window",
            "sliding_window",
            "layer_types",
            "moe_intermediate_size",
            "shared_expert_intermediate_size",
            "num_experts",
            "num_experts_per_tok",
            "norm_topk_prob",
            "decoder_sparse_step",
            "mlp_only_layers",
        ],
        Family::Qwen3Next => &[
            "decoder_sparse_step",
            "full_attention_interval",
            "layer_types",
            "linear_conv_kernel_dim",
            "linear_key_head_dim",
            "linear_num_key_heads",
            "linear_num_value_heads",
            "linear_value_head_dim",
            "mlp_only_layers",
            "moe_intermediate_size",
            "norm_topk_prob",
            "num_experts",
            "num_experts_per_tok",
            "partial_rotary_factor",
            "shared_expert_intermediate_size",
            "use_sliding_window",
            "output_gate_type",
        ],
        Family::Qwen4Exp => &[
            "moe_intermediate_size",
            "shared_expert_intermediate_size",
            "num_experts",
            "num_experts_per_tok",
            "norm_topk_prob",
            "full_attention_interval",
            "layer_types",
            "linear_conv_kernel_dim",
            "linear_key_head_dim",
            "linear_num_key_heads",
            "linear_num_value_heads",
            "linear_value_head_dim",
            "mtp_num_hidden_layers",
            "mtp",
            "partial_rotary_factor",
            "output_gate_type",
            "hc_count",
            "hc_lowrank",
            "indexer_n_heads",
            "indexer_kv_heads",
            "indexer_head_dim",
            "indexer_budget",
            "indexer_compress_ratio",
            "ple_layer_ids",
            "ple_embed_dim",
            "ple_conv_kernel_size",
            "ngram_size",
            "heads_per_ngram",
            "ngram_vocab_size_base",
            "make_ngram_vocab_size_divisible_by",
            "seed",
            "split_ngram_parts",
            "number_of_conv_states",
        ],
        Family::Qwen35 | Family::Qwen35Moe => &[
            "moe_intermediate_size",
            "shared_expert_intermediate_size",
            "num_experts",
            "num_experts_per_tok",
            "attn_output_gate",
            "full_attention_interval",
            "layer_types",
            "linear_conv_kernel_dim",
            "linear_key_head_dim",
            "linear_num_key_heads",
            "linear_num_value_heads",
            "linear_value_head_dim",
            "mtp_num_hidden_layers",
            "mlp_only_layers",
            "partial_rotary_factor",
            "output_gate_type",
        ],
        Family::Gemma => &["hidden_activation", "use_bidirectional_attention"],
        Family::Gemma2 => &[
            "hidden_activation",
            "use_bidirectional_attention",
            "query_pre_attn_scalar",
            "sliding_window",
            "sliding_window_size",
            "layer_types",
            "attn_logit_softcapping",
            "final_logit_softcapping",
        ],
        Family::Gemma3 => &[
            "hidden_activation",
            "query_pre_attn_scalar",
            "sliding_window",
            "sliding_window_pattern",
            "layer_types",
            "rope_local_base_freq",
            "attn_logit_softcapping",
            "final_logit_softcapping",
            "use_bidirectional_attention",
        ],
        Family::GptOss => &[
            "num_local_experts",
            "num_experts_per_tok",
            "experts_per_token",
            "initial_context_length",
            "sliding_window",
            "layer_types",
            "swiglu_limit",
            "swiglu_alpha",
        ],
        Family::Deepseek2 | Family::Deepseek32 | Family::GlmDsa => &[
            "moe_intermediate_size",
            "n_routed_experts",
            "n_shared_experts",
            "num_experts_per_tok",
            "n_group",
            "topk_group",
            "first_k_dense_replace",
            "kv_lora_rank",
            "q_lora_rank",
            "qk_nope_head_dim",
            "qk_rope_head_dim",
            "qk_head_dim",
            "v_head_dim",
            "routed_scaling_factor",
            "norm_topk_prob",
            "scoring_func",
            "topk_method",
            "rope_interleave",
            "moe_layer_freq",
        ],
        Family::Llama4 => &[
            "intermediate_size_mlp",
            "num_local_experts",
            "num_experts_per_tok",
            "interleave_moe_layer_step",
            "moe_layers",
            "use_qk_norm",
            "no_rope_layers",
            "no_rope_layer_interval",
            "attention_chunk_size",
            "layer_types",
            "attn_temperature_tuning",
            "floor_scale",
            "attn_scale",
            "attention_bias",
            "for_llm_compressor",
        ],
        Family::Glm4 => &["partial_rotary_factor"],
        Family::Mistral3 => &["sliding_window"],
        Family::Mamba2 => &[
            "num_heads",
            "state_size",
            "expand",
            "conv_kernel",
            "n_groups",
            "use_bias",
            "use_conv_bias",
            "layer_norm_epsilon",
            "time_step_limit",
            // Read by neither transformers, whose gated norm multiplies by the gate and then RMS-normalizes whatever
            // they say, nor the converter (Mamba-Codestral states `norm_before_gate`).
            "rms_norm",
            "norm_before_gate",
            "pad_vocab_size_multiple",
        ],
        Family::FalconH1 => &[
            "attention_bias",
            "attention_in_multiplier",
            "attention_out_multiplier",
            "attn_layer_indices",
            "embedding_multiplier",
            "key_multiplier",
            "lm_head_multiplier",
            "mamba_chunk_size",
            "mamba_conv_bias",
            "mamba_d_conv",
            "mamba_d_head",
            "mamba_d_ssm",
            "mamba_d_state",
            "mamba_expand",
            "mamba_n_groups",
            "mamba_n_heads",
            "mamba_norm_before_gate",
            "mamba_proj_bias",
            "mamba_rms_norm",
            "mamba_use_mlp",
            "mlp_bias",
            "mlp_expansion_factor",
            "mlp_multipliers",
            "num_logits_to_keep",
            "projectors_bias",
            "ssm_in_multiplier",
            "ssm_multipliers",
            "ssm_out_multiplier",
            "time_step_limit",
        ],
        Family::Gemma3n => &[
            "activation_sparsity_pattern",
            "altup_active_idx",
            "altup_coef_clip",
            "altup_correct_scale",
            "altup_lr_multiplier",
            "altup_num_inputs",
            "attention_bias",
            "audio_config",
            "audio_soft_tokens_per_image",
            "audio_token_id",
            "boa_token_id",
            "boi_token_id",
            "eoa_token_id",
            "eoi_token_id",
            "final_logit_softcapping",
            "hidden_activation",
            "hidden_size_per_layer_input",
            "laurel_rank",
            "layer_types",
            "num_kv_shared_layers",
            "query_pre_attn_scalar",
            "rope_local_base_freq",
            "sliding_window",
            "vision_soft_tokens_per_image",
            "vocab_size_per_layer_input",
        ],
        Family::Gemma4 => &[
            "attention_bias",
            "attention_k_eq_v",
            "audio_config",
            "audio_token_id",
            "boa_token_id",
            "boi_token_id",
            "enable_moe_block",
            "eoa_token_id",
            "eoa_token_index",
            "eoi_token_id",
            "expert_intermediate_size",
            "final_logit_softcapping",
            "global_head_dim",
            "hidden_activation",
            "hidden_size_per_layer_input",
            "layer_types",
            "moe_intermediate_size",
            "num_experts",
            "num_global_key_value_heads",
            "num_kv_shared_layers",
            "per_layer_config",
            "sliding_window",
            "top_k_experts",
            "use_bidirectional_attention",
            "use_double_wide_mlp",
            "vision_soft_tokens_per_image",
            "vocab_size_per_layer_input",
        ],
        Family::KimiLinear => &[
            "attention_bias",
            "first_k_dense_replace",
            "head_dim",
            "kv_lora_rank",
            "linear_attn_config",
            "mla_use_nope",
            "moe_intermediate_size",
            "moe_layer_freq",
            "moe_renormalize",
            "moe_router_activation_func",
            "num_expert_group",
            "num_experts",
            "num_experts_per_token",
            "num_nextn_predict_layers",
            "num_shared_experts",
            "q_lora_rank",
            "qk_nope_head_dim",
            "qk_rope_head_dim",
            "routed_scaling_factor",
            "topk_group",
            "use_grouped_topk",
            "v_head_dim",
        ],
        // flash-linear-attention's RWKV-7 configuration (`RWKV7ForCausalLM`, fla-hub's) and the Hugging Face port's
        // (`Rwkv7ForCausalLM`, BlinkDL's checkpoints), and fla's kernel choices, which compute nothing else; fla's
        // `num_heads`, which it does not read beside a `head_dim`.
        Family::Rwkv7 => &[
            "a_low_rank_dim",
            "attn",
            "attn_mode",
            "decay_low_rank_dim",
            "fuse_cross_entropy",
            "fuse_norm",
            "gate_low_rank_dim",
            "hidden_ratio",
            "norm_bias",
            "norm_eps",
            "norm_first",
            "num_heads",
            "v_low_rank_dim",
            "value_dim",
            "wkv_has_gate",
            "attention_hidden_size",
            "head_size",
            "layer_norm_epsilon",
            "lora_rank_decay",
            "lora_rank_gate",
            "lora_rank_iclr",
            "lora_rank_value_residual_mix",
        ],
        // MiniMax's own configuration (MiniMax-Text-01's and M1's) and transformers' (`MiniMaxForCausalLM`), each refused
        // the other's names (`validate`).
        Family::Minimax01 => &[
            "attn_type_list",
            "layernorm_full_attention_alpha",
            "layernorm_full_attention_beta",
            "layernorm_linear_attention_alpha",
            "layernorm_linear_attention_beta",
            "layernorm_mlp_alpha",
            "layernorm_mlp_beta",
            "layer_types",
            "full_attn_alpha_factor",
            "full_attn_beta_factor",
            "linear_attn_alpha_factor",
            "linear_attn_beta_factor",
            "mlp_alpha_factor",
            "mlp_beta_factor",
            "block_size",
            "postnorm",
            "rotary_dim",
            "num_local_experts",
            "num_experts_per_tok",
            "shared_intermediate_size",
            "shared_moe_mode",
            "sliding_window",
        ],
        Family::Jamba => &[
            "attn_layer_offset",
            "attn_layer_period",
            "expert_layer_offset",
            "expert_layer_period",
            "mamba_conv_bias",
            "mamba_d_conv",
            "mamba_d_state",
            "mamba_dt_rank",
            "mamba_expand",
            "mamba_proj_bias",
            "num_experts",
            "num_experts_per_tok",
            "num_logits_to_keep",
            "output_router_logits",
            "router_aux_loss_coef",
            "sliding_window",
            "use_associative_scan",
            "use_mamba_kernels",
            "use_mambapy",
        ],
        Family::NemotronH => &[
            "attention_bias",
            "attention_head_dim",
            "chunk_size",
            "conv_kernel",
            "expand",
            "hidden_dropout",
            "hybrid_override_pattern",
            "layers_block_type",
            "layer_norm_epsilon",
            "mamba_head_dim",
            "mamba_hidden_act",
            "mamba_num_groups",
            "mamba_num_heads",
            "mamba_proj_bias",
            "mamba_ssm_cache_dtype",
            "mamba_state_dim",
            "mlp_bias",
            "mlp_hidden_act",
            "n_groups",
            "num_logits_to_keep",
            "num_query_groups",
            "rescale_prenorm_residual",
            "residual_in_fp32",
            "sliding_window",
            "ssm_state_size",
            "time_step_floor",
            "time_step_limit",
            "time_step_max",
            "time_step_min",
            "time_step_rank",
            "use_bias",
            "use_conv_bias",
            "use_mamba_kernels",
        ],
        Family::Lfm2 => &[
            "norm_eps",
            "conv_bias",
            "conv_L_cache",
            "block_multiple_of",
            "block_ffn_dim_multiplier",
            "block_auto_adjust_ff_dim",
            "block_ff_dim",
            "full_attn_idxs",
            "layer_types",
            "tie_embedding",
            "block_dim",
            "conv_dim",
            "conv_dim_out",
            "num_heads",
            "theta",
            "block_norm_eps",
            "use_pos_enc",
            "block_use_swiglu",
        ],
        Family::PhiMoe => &[
            "num_local_experts",
            "num_experts_per_tok",
            "router_jitter_noise",
            "attention_bias",
            "lm_head_bias",
            "sliding_window",
            "original_max_position_embeddings",
            "partial_rotary_factor",
        ],
        Family::Glm4Moe => &[
            "partial_rotary_factor",
            "moe_intermediate_size",
            "n_routed_experts",
            "n_shared_experts",
            "num_experts_per_tok",
            "first_k_dense_replace",
            "routed_scaling_factor",
            "norm_topk_prob",
            "n_group",
            "topk_group",
            "use_qk_norm",
        ],
        Family::Olmo2 => &["sliding_window", "layer_types"],
        Family::Phi3 => &[
            "partial_rotary_factor",
            "original_max_position_embeddings",
            "sliding_window",
            "resid_pdrop",
            "embd_pdrop",
            "lm_head_bias",
            "mlp_bias",
            "full_attn_mod",
            "interpolate_factor",
        ],
        Family::MinimaxM2 => &[
            "num_local_experts",
            "num_experts_per_tok",
            "rotary_dim",
            "partial_rotary_factor",
            "use_qk_norm",
            "qk_norm_type",
            "scoring_func",
            "use_routing_bias",
            "shared_intermediate_size",
            "shared_moe_mode",
            "mlp_intermediate_size",
            "attn_type_list",
            "layernorm_full_attention_beta",
            "layernorm_linear_attention_beta",
            "layernorm_mlp_beta",
            "sliding_window",
            "use_mtp",
            "num_mtp_modules",
            "mtp_transformer_layers",
        ],
    };
    // DeepSeek's sparse attention: its indexer, and GLM-5's per-layer indexer and FFN kinds.
    let sparse: &[&str] = match f {
        Family::Deepseek32 => &["index_n_heads", "index_head_dim", "index_topk", "layer_types", "mlp_layer_types", "qk_head_dim", "mlp_bias"],
        Family::GlmDsa => &[
            "index_n_heads",
            "index_head_dim",
            "index_topk",
            "layer_types",
            "mlp_bias",
            "indexer_types",
            "index_topk_freq",
            "index_skip_topk_offset",
            "index_topk_pattern",
            "indexer_rope_interleave",
            "mlp_layer_types",
            "qk_head_dim",
        ],
        _ => &[],
    };
    own.iter().copied().chain(sparse.iter().copied()).chain(DECODER.iter().copied()).collect::<Vec<_>>().leak()
}

/// Refuse a configuration this reader would read as something other than what it says: an unknown key, or a known
/// key whose value the converter's GGUF (and so the spec) does not carry.
fn validate(f: Family, h: &Hparams) -> io::Result<()> {
    let known: BTreeSet<&str> =
        INERT.iter().copied().chain(read_keys(f).iter().copied()).chain(["quantization_config"]).collect();
    let mut unknown: Vec<&str> = h.map.keys().map(String::as_str).filter(|k| !known.contains(k)).collect();
    unknown.sort();
    if let Some(k) = unknown.first() {
        return Err(invalid(format!("config.json: `{k}` is a key the {} intake does not read; unknown keys: {unknown:?}", f.arch())));
    }
    // An FP8 checkpoint's weights are read beside their scales (`fp8_scales`); every other quantized format is refused.
    if let Some(q) = h.map.get("quantization_config").filter(|q| !q.is_null()) {
        let method = q.get("quant_method").and_then(Value::as_str).unwrap_or("unnamed");
        if method != "fp8" {
            return Err(invalid(format!(
                "config.json: `quantization_config` names a `{method}` checkpoint, a format the quantized-format intake does not read yet"
            )));
        }
    }
    let refuse = |k: &str, why: &str| Err(invalid(format!("config.json: `{k}` {why}")));
    let get = |k: &str| h.map.get(k).filter(|v| !v.is_null());
    let act = h.find(&["hidden_act", "hidden_activation"]).and_then(Value::as_str);
    // llama.cpp fixes a Gemma's score scale by the stack's depth: `1/√(d/heads)` at the 27B stack (46 blocks for Gemma
    // 2, 62 for Gemma 3), `1/√head_dim` otherwise; the converter writes no key for it.
    let gemma_scale = |deep: u64| -> io::Result<()> {
        let Some(scalar) = get("query_pre_attn_scalar").and_then(Value::as_u64) else { return Ok(()) };
        let layers = h.need_u64("num_hidden_layers")?;
        let fixed = if layers == deep {
            h.need_u64("hidden_size")? / h.need_u64("num_attention_heads")?
        } else {
            h.u64(&["head_dim"]).unwrap_or(256)
        };
        if scalar != fixed {
            return refuse(
                "query_pre_attn_scalar",
                &format!("is {scalar}; llama.cpp scales this stack's scores by 1/√{fixed}, and its GGUF says no other"),
            );
        }
        Ok(())
    };
    if matches!(f, Family::Gemma | Family::Gemma2 | Family::Gemma3) && get("use_bidirectional_attention").and_then(Value::as_bool) == Some(true) {
        return refuse("use_bidirectional_attention", "is true; a causal decoder is read here");
    }
    match f {
        // transformers reads Gemma's legacy `gelu` as the tanh approximation, the activation llama.cpp computes.
        Family::Gemma => {
            for key in ["hidden_act", "hidden_activation"] {
                if get(key).and_then(Value::as_str).is_some_and(|a| a != "gelu_pytorch_tanh" && a != "gelu") {
                    return refuse(key, "is not gelu_pytorch_tanh (or its legacy name gelu), the activation the gemma spec reads");
                }
            }
            h.need_u64("head_dim")?;
        }
        Family::Gemma2 => {
            for key in ["hidden_act", "hidden_activation"] {
                if get(key).and_then(Value::as_str).is_some_and(|a| a != "gelu_pytorch_tanh") {
                    return refuse(key, "is not gelu_pytorch_tanh, the activation the gemma2 spec reads");
                }
            }
            gemma_scale(46)?;
            h.need_u64("head_dim")?;
            for key in ["attn_logit_softcapping", "final_logit_softcapping"] {
                if !get(key).and_then(Value::as_f64).is_some_and(|c| c > 0.0) {
                    return refuse(key, "is not a positive number; llama.cpp caps every Gemma 2, and its GGUF says no other");
                }
            }
            let window = h.need_u64("sliding_window")?;
            if get("sliding_window_size").and_then(Value::as_u64).is_some_and(|w| w != window) {
                return refuse("sliding_window_size", &format!("is not sliding_window ({window}), the one window the GGUF carries"));
            }
            if let Some(Value::Array(types)) = h.map.get("layer_types") {
                let alternating = types.len() as u64 == h.need_u64("num_hidden_layers")?
                    && types.iter().enumerate().all(|(l, t)| t.as_str() == Some(if l % 2 == 0 { "sliding_attention" } else { "full_attention" }));
                if !alternating {
                    return refuse("layer_types", "is not a window on every other layer from the first, the pattern llama.cpp fixes for Gemma 2");
                }
            }
        }
        Family::Gemma3 => {
            if act.is_some_and(|a| a != "gelu_pytorch_tanh") {
                return refuse("hidden_activation", "is not gelu_pytorch_tanh, the activation the gemma3 spec reads");
            }
            gemma_scale(62)?;
            if get("attn_logit_softcapping").is_some() {
                return refuse("attn_logit_softcapping", "is set; Gemma 3 attention has no cap");
            }
        }
        // transformers 4's flag for experts stored one linear each (for llm-compressor), where the releases fuse them.
        Family::Llama4 => {
            if get("for_llm_compressor").and_then(Value::as_bool) == Some(true) {
                return refuse("for_llm_compressor", "is true; experts stored one linear each are a layout the llama4 intake does not read");
            }
        }
        Family::GptOss => {
            if act.is_some_and(|a| a != "silu") {
                return refuse("hidden_act", "is not silu");
            }
            if get("swiglu_limit").and_then(Value::as_f64).is_some_and(|v| v != 7.0) {
                return refuse("swiglu_limit", "is not 7.0, the clamp the gpt-oss spec reads");
            }
            if get("swiglu_alpha").and_then(Value::as_f64).is_some_and(|v| v != 1.702) {
                return refuse("swiglu_alpha", "is not 1.702, the slope the gpt-oss spec reads");
            }
            // OpenAI's own names for two settings the converter reads under transformers' names: each must agree.
            if get("experts_per_token").is_some_and(|v| Some(v) != get("num_experts_per_tok")) {
                return refuse("experts_per_token", "differs from num_experts_per_tok, which the converter reads");
            }
            let original = rope_parameters(h).get("original_max_position_embeddings").cloned();
            if get("initial_context_length").is_some_and(|v| original.as_ref().is_some_and(|o| o != v)) {
                return refuse("initial_context_length", "differs from the rotation's original_max_position_embeddings, which the converter reads");
            }
        }
        // OLMo 3's windows: each layer windowed or attending every row, as `layer_types` names it.
        Family::Olmo2 => {
            if act.is_some_and(|a| a != "silu") {
                return refuse("hidden_act", "is not silu, the activation the olmo2 spec reads");
            }
            if let Some(Value::Array(types)) = h.map.get("layer_types") {
                let named = types.len() as u64 == h.need_u64("num_hidden_layers")?
                    && types.iter().all(|t| matches!(t.as_str(), Some("sliding_attention" | "full_attention")));
                if !named {
                    return refuse("layer_types", "is not one sliding_attention or full_attention a layer");
                }
                let windowed = types.iter().any(|t| t.as_str() == Some("sliding_attention"));
                if windowed && get("sliding_window").and_then(Value::as_u64).is_none_or(|w| w == 0) {
                    return refuse("sliding_window", "is not a positive window, and layer_types windows a layer");
                }
            }
        }
        Family::Gemma3n | Family::Gemma4 => {
            if act.is_some_and(|a| a != "gelu_pytorch_tanh") {
                return refuse("hidden_activation", &format!("is not gelu_pytorch_tanh, the activation llama.cpp's {} computes", f.arch()));
            }
        }
        Family::FalconH1 => {
            if act.is_some_and(|a| a != "silu") {
                return refuse("hidden_act", "is not silu, which the converter asserts");
            }
        }
        Family::Rwkv7 => {
            if act.is_some_and(|a| a != "sqrelu") {
                return refuse("hidden_act", "is not sqrelu, the channel mix llama.cpp's rwkv7 computes");
            }
        }
        _ => {
            if act.is_some_and(|a| a != "silu") {
                return refuse("hidden_act", "is not silu, the activation this family's spec reads");
            }
        }
    }
    if get("use_sliding_window").and_then(Value::as_bool) == Some(true) {
        return refuse("use_sliding_window", "is true; this family's GGUF carries no window");
    }
    if get("pruned_heads").is_some_and(|v| v.as_object().is_none_or(|o| !o.is_empty())) {
        return refuse("pruned_heads", "names pruned heads, which the converter's GGUF does not carry");
    }
    for k in ["add_cross_attention", "is_encoder_decoder"] {
        if get(k).and_then(Value::as_bool) == Some(true) {
            return refuse(k, "is true; a decoder alone is read here");
        }
    }
    if f.deepseek() && get("moe_layer_freq").is_some_and(|v| v.as_u64() != Some(1)) {
        return refuse("moe_layer_freq", "is not 1; llama.cpp's deepseek2 reads every block past the dense lead as a mixture");
    }
    if get("mlp_bias").and_then(Value::as_bool) == Some(true) {
        return refuse("mlp_bias", "is true; the spec reads no FFN biases");
    }
    if let Some(a) = get("mlp_only_layers").and_then(Value::as_array) {
        if !a.is_empty() {
            return refuse("mlp_only_layers", "names dense layers inside a mixture, which the spec does not read");
        }
    }
    if get("decoder_sparse_step").and_then(Value::as_u64).is_some_and(|s| s != 1) {
        return refuse("decoder_sparse_step", "is not 1; every layer is read as a mixture");
    }
    // llama.cpp fixes by architecture whether the kept top-k weights are renormalized, and the converter writes no key
    // for it: Qwen2-MoE's never are, Qwen3-MoE's always (transformers' default for both is not to).
    let renormalized = get("norm_topk_prob").and_then(Value::as_bool).unwrap_or(f == Family::Qwen4Exp);
    match f {
        Family::Qwen2Moe if renormalized => {
            return refuse("norm_topk_prob", "is true; llama.cpp's qwen2moe keeps the top-k softmax weights as they are")
        }
        Family::Qwen3Moe | Family::Qwen3Next | Family::Qwen4Exp if !renormalized => {
            return refuse("norm_topk_prob", "is not true; llama.cpp's qwen3moe renormalizes the top-k softmax weights")
        }
        _ => {}
    }
    if f.hybrid() && get("attn_output_gate").and_then(Value::as_bool) == Some(false) {
        return refuse("attn_output_gate", "is false; Qwen3.5's attention is read with its output gate");
    }
    // Gated DeltaNet's gated norm multiplies by its gate's activation: SiLU (swish, as Qwen3.8 names it) in the
    // converter's qwen35 and qwen3next GGUFs, the sigmoid llama.cpp's qwen4exp fixes. Transformers gates a qwen4_exp
    // configuration that names none by `hidden_act`, which no qwen4exp GGUF can say.
    if f == Family::Qwen4Exp {
        match get("output_gate_type").and_then(Value::as_str) {
            Some("sigmoid") => {}
            Some(other) => return refuse("output_gate_type", &format!("is `{other}`; llama.cpp's qwen4exp gates Gated DeltaNet by the sigmoid")),
            None => return refuse("output_gate_type", "is unset; transformers gates by `hidden_act`, llama.cpp's qwen4exp by the sigmoid"),
        }
        if get("indexer_kv_heads").and_then(Value::as_u64).is_some_and(|k| k != 1) {
            return refuse("indexer_kv_heads", "is not 1; QSA scores every index query against one key head");
        }
        if get("ple_layer_ids").and_then(Value::as_array).is_some_and(|l| l.len() > 1) {
            return refuse("ple_layer_ids", "names more than one layer; the converter's GGUF holds one n-gram table");
        }
        // Transformers derives its cache's convolution states from the n-gram layers (the history and the dilated
        // convolution beside Gated DeltaNet's) and saves the count.
        let ple = get("ple_layer_ids").and_then(Value::as_array).is_some_and(|l| !l.is_empty());
        if get("number_of_conv_states").and_then(Value::as_u64).is_some_and(|n| n != if ple { 3 } else { 1 }) {
            return refuse("number_of_conv_states", "is not the count the n-gram layers give (3 with them, 1 without)");
        }
        if let Some(mtp) = get("mtp") {
            let stated = get("mtp_num_hidden_layers").and_then(Value::as_u64);
            if mtp.get("num_hidden_layers").and_then(Value::as_u64).is_some_and(|n| Some(n) != stated) {
                return refuse("mtp", "counts draft layers other than `mtp_num_hidden_layers`");
            }
        }
    } else if let Some(gate) = get("output_gate_type").filter(|g| !matches!(g.as_str(), Some("silu" | "swish"))) {
        return refuse("output_gate_type", &format!("is {gate}; Gated DeltaNet's norm is read gated by SiLU (swish)"));
    }
    if f == Family::Llama && get("sliding_window").is_some() {
        return refuse("sliding_window", "is set; the llama GGUF carries no window, and llama.cpp attends every position");
    }
    if matches!(f, Family::Qwen2 | Family::Qwen3 | Family::Qwen3Moe | Family::Qwen2Moe) {
        if let Some(types) = get("layer_types").and_then(Value::as_array) {
            if types.iter().any(|t| t.as_str() != Some("full_attention")) {
                return refuse("layer_types", "names a layer that is not full attention");
            }
        }
    }
    if f == Family::Llama4 {
        // The converter's GGUF carries none of these: llama.cpp fixes them, so a configuration that differs is a model
        // the GGUF cannot say.
        let n = |k: &str| get(k).and_then(Value::as_f64);
        if n("attention_chunk_size").is_some_and(|c| c != 8192.0) {
            return refuse("attention_chunk_size", "is not 8192, the chunk llama.cpp's llama4 attends within");
        }
        if n("floor_scale").is_some_and(|c| c != 8192.0) {
            return refuse("floor_scale", "is not 8192, the attention temperature's floor llama.cpp fixes");
        }
        if n("attn_scale").is_some_and(|c| c != 0.1) {
            return refuse("attn_scale", "is not 0.1, the attention temperature's scale llama.cpp fixes");
        }
        if get("attn_temperature_tuning").and_then(Value::as_bool) == Some(false) {
            return refuse("attn_temperature_tuning", "is false; llama.cpp tunes every rotation-free layer's temperature");
        }
        if get("attention_bias").and_then(Value::as_bool) == Some(true) {
            return refuse("attention_bias", "is true; the spec reads no attention biases");
        }
        let experts = get("num_local_experts").and_then(Value::as_u64).unwrap_or(0);
        if get("use_qk_norm").and_then(Value::as_bool).is_some_and(|q| q != (experts != 128)) {
            return refuse("use_qk_norm", "disagrees with llama.cpp's rule: a QK norm unless the model has 128 experts");
        }
        let layers = get("num_hidden_layers").and_then(Value::as_u64).unwrap_or(0) as usize;
        if let Some(a) = get("no_rope_layers").and_then(Value::as_array) {
            let every_fourth: Vec<u64> = (0..layers).map(|l| u64::from((l + 1) % 4 != 0)).collect();
            if a.iter().map(Value::as_u64).collect::<Option<Vec<u64>>>() != Some(every_fourth) {
                return refuse("no_rope_layers", "is not every fourth layer unrotated, the pattern llama.cpp fixes");
            }
        }
        let step = get("interleave_moe_layer_step").and_then(Value::as_u64).unwrap_or(1) as usize;
        if let Some(a) = get("moe_layers").and_then(Value::as_array) {
            let stepped: Vec<u64> = (0..layers).filter(|l| step > 0 && (l + 1) % step == 0).map(|l| l as u64).collect();
            if a.iter().map(Value::as_u64).collect::<Option<Vec<u64>>>() != Some(stepped) {
                return refuse("moe_layers", "is not every interleave_moe_layer_step-th layer, the layers the GGUF names");
            }
        }
    }
    if f.deepseek() && get("rope_interleave").and_then(Value::as_bool) == Some(false) {
        return refuse("rope_interleave", "is false; latent attention is read with interleaved rotation");
    }
    if matches!(f, Family::Deepseek32 | Family::GlmDsa) {
        dsa_layers(h)?;
    }
    if matches!(f, Family::Glm4 | Family::Glm4Moe) && rope_parameters(h).contains_key("mrope_section") {
        return refuse("rope_parameters", "names `mrope_section`: a GLM vision model's multimodal rotation is not read yet");
    }
    if f == Family::PhiMoe {
        // The converter's PhiMoE class writes neither sparsemixer's jitter nor a head width, and llama.cpp reads heads of
        // `hidden_size / num_attention_heads`: the phimoe arm fixes PhiMoE's jitter of 0.01 and its two experts a token.
        if get("num_experts_per_tok").and_then(Value::as_u64).is_some_and(|k| k != 2) {
            return refuse("num_experts_per_tok", "is not 2; PhiMoE's sparsemixer chooses two experts whatever it says");
        }
        if get("router_jitter_noise").and_then(Value::as_f64).is_some_and(|j| j != 0.01) {
            return refuse("router_jitter_noise", "is not 0.01, the sparsemixer jitter the phimoe arm fixes; the GGUF carries none");
        }
        let width = h.need_u64("hidden_size")? / h.need_u64("num_attention_heads")?;
        if get("head_dim").and_then(Value::as_u64).is_some_and(|hd| hd != width) {
            return refuse("head_dim", &format!("is not hidden_size / num_attention_heads ({width}), the head width the phimoe GGUF holds"));
        }
    }
    if matches!(f, Family::Phi3 | Family::PhiMoe) {
        for (k, one) in [("full_attn_mod", 1.0), ("interpolate_factor", 1.0)] {
            if get(k).and_then(Value::as_f64).is_some_and(|v| v != one) {
                return refuse(k, "is not 1; the converter's Phi-3 class reads no such setting");
            }
        }
        let longrope = h.map.get("rope_scaling").or_else(|| h.map.get("rope_parameters")).and_then(|r| {
            r.get("rope_type").or_else(|| r.get("type")).and_then(Value::as_str).map(|t| t == "longrope" || t == "su")
        });
        if longrope == Some(true) {
            return refuse("rope_scaling", "is LongRoPE (Phi-3.5, Phi-4-mini), whose factor tables are not read yet");
        }
    }
    if f == Family::MinimaxM2 {
        // The converter's MiniMaxM2 class writes none of these: a value other than the one its graph fixes is a model the
        // GGUF cannot say.
        if let Some(types) = get("attn_type_list").and_then(Value::as_array) {
            if types.iter().any(|t| t.as_u64() != Some(1)) {
                return refuse("attn_type_list", "names a linear-attention layer (MiniMax-01's); every MiniMax-M2 layer is full attention");
            }
        }
        for beta in ["layernorm_full_attention_beta", "layernorm_linear_attention_beta", "layernorm_mlp_beta"] {
            if get(beta).and_then(Value::as_f64).is_some_and(|b| b != 1.0) {
                return refuse(beta, "is not 1.0; the residual adds each sublayer's output unscaled");
            }
        }
        if get("shared_intermediate_size").and_then(Value::as_u64).is_some_and(|w| w > 0) {
            return refuse("shared_intermediate_size", "is not 0; the converter's MiniMax-M2 writes no shared expert");
        }
        if get("sliding_window").is_some() {
            return refuse("sliding_window", "is set; MiniMax-M2 attends every position");
        }
        let rotary = get("rotary_dim").and_then(Value::as_u64);
        let head = get("head_dim").and_then(Value::as_u64);
        let factor = h.map.get("partial_rotary_factor").or_else(|| h.map.get("rope_parameters").and_then(|r| r.get("partial_rotary_factor")));
        if let (Some(r), Some(hd), Some(f)) = (rotary, head, factor.and_then(Value::as_f64)) {
            if (r as f64 - f * hd as f64).abs() > 0.5 {
                return refuse("rotary_dim", "disagrees with partial_rotary_factor of head_dim");
            }
        }
    }
    if f == Family::Mistral3 {
        mistral3_text(h)?;
    }
    if f == Family::Mamba2 {
        mamba2_widths(h)?;
        for (key, stands) in [("use_bias", false), ("use_conv_bias", true)] {
            if get(key).and_then(Value::as_bool).is_some_and(|v| v != stands) {
                return refuse(key, &format!("is {}; llama.cpp's mamba2 reads the block as transformers' default ({stands}) builds it", !stands));
            }
        }
        if let Some(limit) = get("time_step_limit").and_then(Value::as_array) {
            let unbounded = limit.len() == 2 && json_f64(&limit[0]) == Some(0.0) && (limit[1].is_null() || json_f64(&limit[1]) == Some(f64::INFINITY));
            if !unbounded {
                return refuse("time_step_limit", "clamps the time step; llama.cpp's scan clamps none");
            }
        }
    }
    if f == Family::KimiLinear {
        kimi_linear_layers(h)?;
        for (key, stands, why) in [
            ("moe_renormalize", true, "llama.cpp's kimi-linear renormalizes the kept weights whatever the file says"),
            ("mla_use_nope", true, "llama.cpp's kimi-linear rotates no latent key"),
            ("attention_bias", false, "llama.cpp's kimi-linear reads no attention bias"),
        ] {
            if get(key).and_then(Value::as_bool).is_some_and(|v| v != stands) {
                return refuse(key, &format!("is {}; {why}", !stands));
            }
        }
        if get("moe_layer_freq").and_then(Value::as_u64).is_some_and(|f| f != 1) {
            return refuse("moe_layer_freq", "is not 1; llama.cpp's kimi-linear routes every block past the dense ones");
        }
        if get("num_expert_group").and_then(Value::as_u64).is_some_and(|g| g > 1) {
            return refuse("num_expert_group", "groups the experts; llama.cpp's kimi-linear chooses among them all");
        }
        if get("num_nextn_predict_layers").and_then(Value::as_u64).is_some_and(|n| n > 0) {
            return refuse("num_nextn_predict_layers", "names draft blocks, which llama.cpp's kimi-linear does not read");
        }
        if get("moe_router_activation_func").and_then(Value::as_str).is_some_and(|a| !matches!(a, "sigmoid" | "softmax")) {
            return refuse("moe_router_activation_func", "is neither sigmoid nor softmax, the scores the converter writes");
        }
    }
    if f == Family::Gemma3n {
        gemma3n_layers(h)?;
        if get("attention_bias").and_then(Value::as_bool) == Some(true) {
            return refuse("attention_bias", "is true; llama.cpp's gemma3n reads no attention bias");
        }
        if get("altup_correct_scale").and_then(Value::as_bool) == Some(false) {
            return refuse("altup_correct_scale", "is false; llama.cpp's gemma3n scales the corrected stream whatever the file says");
        }
        for (key, want, what) in [
            ("altup_num_inputs", 4, "four AltUp streams, which the converter asserts"),
            ("altup_active_idx", 0, "stream 0 active"),
            ("laurel_rank", 64, "LAuReL of rank 64"),
            ("hidden_size_per_layer_input", 256, "per-layer inputs of 256"),
        ] {
            if get(key).and_then(Value::as_u64).is_some_and(|v| v != want) {
                return refuse(key, &format!("is not {want}: llama.cpp's gemma3n computes {what} whatever the file says"));
            }
        }
        if get("attn_logit_softcapping").is_some() {
            return refuse("attn_logit_softcapping", "is set; Gemma 3n's attention has no cap");
        }
    }
    if f == Family::Gemma4 {
        let blocks = gemma4_blocks(h)?;
        if get("attention_bias").and_then(Value::as_bool) == Some(true) {
            return refuse("attention_bias", "is true; llama.cpp's gemma4 reads no attention bias");
        }
        if get("use_bidirectional_attention").and_then(Value::as_str) == Some("all") {
            return refuse("use_bidirectional_attention", "is `all`; llama.cpp's gemma4 attends causally on every block of a text model");
        }
        let rp = rope_parameters(h);
        let (full, local) = (rp.get("full_attention").cloned().unwrap_or_default(), rp.get("sliding_attention").cloned().unwrap_or_default());
        if full.get("rope_type").and_then(Value::as_str) != Some("proportional") || full.get("partial_rotary_factor").and_then(Value::as_f64).is_none() {
            return refuse("rope_parameters", "names no proportional rotation with a partial factor for the global blocks, which the converter asserts");
        }
        if local.get("rope_type").and_then(Value::as_str).is_some_and(|t| t != "default")
            || local.get("partial_rotary_factor").and_then(Value::as_f64).is_some_and(|p| p != 1.0)
        {
            return refuse("rope_parameters", "scale or shorten the windowed blocks' rotation; the converter writes every dimension of it unscaled");
        }
        let shared = h.u64(&["num_kv_shared_layers"]).ok_or_else(|| invalid("config.json: no `num_kv_shared_layers`, which the converter reads"))?;
        let n = blocks.swa.len() as u64;
        if shared > 0 {
            let from = n.saturating_sub(shared) as usize;
            if from < 2 || !(blocks.swa[from - 2] && !blocks.swa[from - 1]) {
                return refuse(
                    "num_kv_shared_layers",
                    &format!("shares from block {from}, whose two blocks before are not a windowed block and a global one, the caches llama.cpp's gemma4 reads"),
                );
            }
        }
        if get("enable_moe_block").and_then(Value::as_bool) == Some(true) {
            for key in ["num_experts", "top_k_experts"] {
                h.need_u64(key)?;
            }
            if h.u64(&["expert_intermediate_size", "moe_intermediate_size"]).is_none() {
                return refuse("moe_intermediate_size", "is unset beside a mixture, whose experts' width the converter reads");
            }
        }
        if h.u64(&["hidden_size_per_layer_input"]).unwrap_or(0) > 0 && h.u64(&["vocab_size_per_layer_input"]) != h.u64(&["vocab_size"]) {
            return refuse("vocab_size_per_layer_input", "is not the vocabulary; llama.cpp's gemma4 reads a per-layer row for every token");
        }
    }
    if f == Family::Rwkv7 {
        let d = h.need_u64("hidden_size")?;
        let (head, _) = rwkv7_head_and_eps(h)?;
        rwkv7_ranks(h)?;
        if head == 0 || d % head != 0 {
            let key = if rwkv7_port(h) { "head_size" } else { "head_dim" };
            return refuse(key, &format!("is {head}, which divides no width of {d} into heads"));
        }
        if get("attn").is_some() {
            return refuse("attn", "names attention blocks among the time mixes, which llama.cpp's rwkv7 reads none of");
        }
        for (key, stands, why) in [
            ("norm_first", true, "llama.cpp's rwkv7 norms the embedding before the first block"),
            ("norm_bias", true, "llama.cpp's rwkv7 reads every LayerNorm's bias"),
            ("wkv_has_gate", true, "llama.cpp's rwkv7 reads six lerps and a gate in every block"),
            ("tie_word_embeddings", false, "llama.cpp's rwkv7 reads a head of its own"),
        ] {
            if get(key).and_then(Value::as_bool).is_some_and(|v| v != stands) {
                return refuse(key, &format!("is {}; {why}", !stands));
            }
        }
        for key in ["value_dim", "attention_hidden_size"] {
            if get(key).and_then(Value::as_u64).is_some_and(|w| w != d) {
                return refuse(key, &format!("is not the width {d}; llama.cpp's rwkv7 mixes in the model's width"));
            }
        }
        if !h.map.contains_key("intermediate_size") {
            return refuse("intermediate_size", "is unset, and the converter reads it (null for four times the width)");
        }
        // fla group-norms each head's read at `head_dim·norm_eps`, llama.cpp at 64·10⁻⁵.
        if !rwkv7_port(h) && h.f64(&["norm_eps"]).is_some_and(|e| ((head as f64 * e) as f32) != 64e-5f32) {
            return refuse("norm_eps", &format!("times the head width {head} is not 64·10⁻⁵, the epsilon llama.cpp's rwkv7 group-norms each head's read at"));
        }
    }
    if f == Family::Minimax01 {
        let own = minimax01_names(h);
        let (theirs, whose) = if own.class_is_minimax {
            (MINIMAX01_TRANSFORMERS_KEYS, "transformers' MiniMaxForCausalLM")
        } else {
            (MINIMAX01_OWN_KEYS, "MiniMax's own configuration")
        };
        if let Some(k) = theirs.iter().copied().find(|k| get(k).is_some()) {
            return refuse(k, &format!("is a key of {whose}, which `{}` does not read", h.architecture));
        }
        minimax01_layers(h)?;
        minimax01_residual_scale(h)?;
        h.need_u64("rotary_dim")?;
        // transformers' class computes the normed-input residual whatever `postnorm` says; MiniMax's own code adds the
        // input itself unless it is true.
        if own.class_is_minimax {
            if get("postnorm").and_then(Value::as_bool) != Some(true) {
                return refuse("postnorm", "is not true; MiniMax's code adds each sublayer's output to its input, llama.cpp's minimax-01 to its normed input");
            }
            if !h.need_u64("num_attention_heads")?.is_power_of_two() {
                return refuse(
                    "num_attention_heads",
                    "is not a power of two; MiniMax's code takes such a count's slopes from the next power's, llama.cpp's minimax-01 from the count's own",
                );
            }
        }
        if get("shared_intermediate_size").and_then(Value::as_u64).is_some_and(|w| w > 0) {
            return refuse("shared_intermediate_size", "is not 0; the converter's MiniMax-01 writes no shared expert");
        }
        if get("sliding_window").is_some() {
            return refuse("sliding_window", "windows the attention; llama.cpp's minimax-01 attends every position");
        }
        if full_rope(&rope_parameters(h)).get("rope_type").and_then(Value::as_str).is_some_and(|t| t != "default") {
            return refuse("rope_parameters", "scale the rotation; MiniMax's code rotates unscaled, and the converter writes its base alone");
        }
    }
    if f == Family::Jamba {
        jamba_blocks(h)?;
        for (key, stands, why) in [
            ("mamba_conv_bias", true, "llama.cpp's jamba reads its convolution's bias as one every scan block holds"),
            ("mamba_proj_bias", false, "llama.cpp's jamba reads no bias on the scan's projections"),
        ] {
            if get(key).and_then(Value::as_bool).is_some_and(|v| v != stands) {
                return refuse(key, &format!("is {}; {why}", !stands));
            }
        }
        if get("sliding_window").is_some() {
            return refuse("sliding_window", "windows the attention; llama.cpp's jamba attends every position");
        }
        if get("mamba_dt_rank").is_some_and(|v| !v.is_u64()) {
            return refuse("mamba_dt_rank", "is not a rank, which the converter writes as it stands");
        }
    }
    if f == Family::NemotronH {
        nemotron_h_blocks(h)?;
        nemotron_h_widths(h)?;
        for (key, stands, why) in [
            ("use_conv_bias", true, "llama.cpp's scan adds its convolution's bias unconditionally"),
            ("use_bias", false, "llama.cpp's nemotron_h reads no bias on the scan's projections"),
            ("mamba_proj_bias", false, "llama.cpp's nemotron_h reads no bias on the scan's projections"),
            ("mlp_bias", false, "the spec's dense FFN has no biases"),
            ("attention_bias", false, "this reader presents no attention biases of Nemotron-H's"),
        ] {
            if get(key).and_then(Value::as_bool).is_some_and(|v| v != stands) {
                return refuse(key, &format!("is {}; {why}", !stands));
            }
        }
        for (key, act) in [("mamba_hidden_act", "silu"), ("mlp_hidden_act", "relu2")] {
            if get(key).and_then(Value::as_str).is_some_and(|a| a != act) {
                return refuse(key, &format!("is not {act}, the activation llama.cpp's nemotron_h computes"));
            }
        }
        if get("sliding_window").is_some() {
            return refuse("sliding_window", "windows the attention; llama.cpp's nemotron_h attends every position");
        }
        if let Some(limit) = get("time_step_limit").and_then(Value::as_array) {
            let unbounded = limit.len() == 2 && json_f64(&limit[0]) == Some(0.0) && (limit[1].is_null() || json_f64(&limit[1]) == Some(f64::INFINITY));
            if !unbounded {
                return refuse("time_step_limit", "clamps the time step; llama.cpp's scan clamps none");
            }
        }
    }
    if f == Family::FalconH1 {
        falcon_h1_widths(h)?;
        falcon_h1_multipliers(h)?;
        for (key, stands, why) in [
            ("mamba_conv_bias", true, "llama.cpp's scan adds its convolution's bias unconditionally"),
            ("mamba_proj_bias", false, "llama.cpp's falcon-h1 reads no bias on the scan's projections"),
            ("projectors_bias", false, "llama.cpp's falcon-h1 reads no bias on the projections"),
            ("attention_bias", false, "llama.cpp's falcon-h1 graph adds no attention output bias, and the reader none on the rest"),
            ("mlp_bias", false, "the spec's dense FFN has no biases"),
        ] {
            if get(key).and_then(Value::as_bool).is_some_and(|v| v != stands) {
                return refuse(key, &format!("is {}; {why}", !stands));
            }
        }
        if get("mamba_rms_norm").and_then(Value::as_bool) == Some(true) && get("mamba_norm_before_gate").and_then(Value::as_bool) != Some(false) {
            return refuse("mamba_norm_before_gate", "is true beside the gated norm; llama.cpp gates the scan's read before it norms it");
        }
        if let Some(limit) = get("time_step_limit").and_then(Value::as_array) {
            let unbounded = limit.len() == 2 && json_f64(&limit[0]) == Some(0.0) && (limit[1].is_null() || json_f64(&limit[1]) == Some(f64::INFINITY));
            if !unbounded {
                return refuse("time_step_limit", "clamps the time step; llama.cpp's scan clamps none");
            }
        }
        let rp = full_rope(&rope_parameters(h));
        if rp.get("rope_type").and_then(Value::as_str).is_some_and(|t| t != "default") {
            return refuse("rope_parameters", "scale the rotation; the converter writes its base alone");
        }
        let tied = get("tie_word_embeddings").and_then(Value::as_bool).unwrap_or(false);
        let (embed, head) = (h.f64(&["embedding_multiplier"]).unwrap_or(1.0), h.f64(&["lm_head_multiplier"]).unwrap_or(1.0));
        if tied && embed != head {
            return refuse(
                "lm_head_multiplier",
                &format!("is {head} beside a tied head; the converter folds the embedding's {embed} into the one table the head reads"),
            );
        }
    }
    if f == Family::Lfm2 {
        lfm2_layers(h)?;
        lfm2_ffn(h)?;
        if get("conv_bias").and_then(Value::as_bool) == Some(true) {
            return refuse("conv_bias", "is true; llama.cpp's lfm2 reads no short-convolution bias");
        }
        // LFM2's configurations restate settings under their training code's names, which transformers leaves unread:
        // each must say what the keys transformers reads say.
        let d = h.need_u64("hidden_size")?;
        for key in ["block_dim", "conv_dim", "conv_dim_out"] {
            if get(key).and_then(Value::as_u64).is_some_and(|w| w != d) {
                return refuse(key, &format!("is not hidden_size ({d}), the width transformers builds"));
            }
        }
        if get("num_heads").is_some_and(|v| Some(v) != get("num_attention_heads")) {
            return refuse("num_heads", "differs from num_attention_heads, which transformers reads");
        }
        let theta = full_rope(&rope_parameters(h)).get("rope_theta").and_then(Value::as_f64).unwrap_or(LFM2_THETA);
        if get("theta").and_then(Value::as_f64).is_some_and(|t| t != theta) {
            return refuse("theta", "differs from the rotation's rope_theta, which transformers reads");
        }
        if get("block_norm_eps").is_some_and(|v| Some(v) != get("norm_eps")) {
            return refuse("block_norm_eps", "differs from norm_eps, which transformers reads");
        }
        for key in ["use_pos_enc", "block_use_swiglu"] {
            if get(key).and_then(Value::as_bool) == Some(false) {
                return refuse(key, "is false; transformers' LFM2 rotates its queries and keys and gates its FFN by SiLU whatever it says");
            }
        }
    }
    Ok(())
}

/// Mamba-2's widths as the converter's `Mamba2Model` reads them: the width, the inner width (`expand` times it unless
/// stated), the head width (64 where unstated) and so the heads, refused where the configuration says otherwise
/// (`num_heads`, an inner width not `expand` times the width, which the converter asserts).
fn mamba2_widths(h: &Hparams) -> io::Result<(u64, u64, u64)> {
    let d = h.u64(&["hidden_size", "d_model", "dim"]).ok_or_else(|| invalid("config.json: no `hidden_size`"))?;
    let expand = h.u64(&["mamba_expand", "expand"]).filter(|&e| e > 0).unwrap_or(2);
    let inner = h.u64(&["mamba_d_ssm", "intermediate_size", "d_inner"]).filter(|&i| i > 0).unwrap_or(expand * d);
    let head = h.u64(&["mamba_d_head", "head_dim"]).filter(|&w| w > 0).unwrap_or(64);
    if inner != expand * d || inner % head != 0 {
        return Err(invalid(format!("config.json: an inner width of {inner} is not `expand` ({expand}) times {d} in heads of {head}")));
    }
    if h.u64(&["num_heads"]).is_some_and(|n| n != inner / head) {
        return Err(invalid(format!("config.json: `num_heads` is not the inner width over the head width ({})", inner / head)));
    }
    Ok((d, inner, head))
}

/// Kimi-Linear's blocks as the converter reads them (`KimiLinearModel`): which attend (`linear_attn_config`'s
/// `full_attn_layers`, 1-indexed; the rest, its `kda_layers`, run Kimi delta attention), every block one or the other.
fn kimi_linear_layers(h: &Hparams) -> io::Result<Vec<bool>> {
    let n = h.need_u64("num_hidden_layers")? as usize;
    let c = h.find(&["linear_attn_config"]).and_then(Value::as_object).ok_or_else(|| {
        invalid("config.json: no `linear_attn_config`, which the converter reads (a configuration as transformers 5 saves it names the layers apart)")
    })?;
    let listed = |key: &str| -> io::Result<Vec<usize>> {
        c.get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| invalid(format!("config.json: `linear_attn_config` names no `{key}`")))?
            .iter()
            .map(|v| v.as_u64().filter(|&i| i >= 1 && i as usize <= n).map(|i| i as usize - 1).ok_or_else(|| invalid(format!("config.json: `linear_attn_config.{key}` holds {v}"))))
            .collect()
    };
    let (full, kda) = (listed("full_attn_layers")?, listed("kda_layers")?);
    let attends: Vec<bool> = (0..n).map(|l| full.contains(&l)).collect();
    if (0..n).any(|l| attends[l] == kda.contains(&l)) {
        return Err(invalid("config.json: `linear_attn_config` names a block both or neither of attention and Kimi delta attention"));
    }
    Ok(attends)
}

/// The standard normal's 95th percentile as torch computes it in f32 (`Normal(0, 1).icdf(0.95)`): the converter's, and
/// transformers', activation sparsity at 0.95.
const NORMAL_95TH_PERCENTILE: f32 = f32::from_bits(0x3fd2_8a90);

/// Gemma 3n's blocks as transformers reads them (`Gemma3nTextConfig`), held to the numbers llama.cpp's gemma3n fixes:
/// whether each attends through a window (`layer_types`), each one's FFN width (`intermediate_size`, one or one a block)
/// and whether its gate is sparsified (`activation_sparsity_pattern`, 0.95 on the first ten blocks, none after); the
/// cache shared from block 20 (`num_kv_shared_layers`), blocks 18 and 19 the windowed and the global block whose caches
/// the later ones read.
fn gemma3n_layers(h: &Hparams) -> io::Result<Vec<(bool, u64, bool)>> {
    let n = h.need_u64("num_hidden_layers")? as usize;
    let windowed: Vec<bool> = match h.find(&["layer_types"]) {
        Some(Value::Array(types)) if types.len() == n => types
            .iter()
            .map(|t| match t.as_str() {
                Some("sliding_attention") => Ok(true),
                Some("full_attention") => Ok(false),
                _ => Err(invalid(format!("config.json: `layer_types` names a block {t}"))),
            })
            .collect::<io::Result<_>>()?,
        Some(other) => return Err(invalid(format!("config.json: `layer_types` is {other}, not one kind a block"))),
        None => (0..n).map(|l| (l + 1) % 5 != 0).collect(),
    };
    let widths: Vec<u64> = match h.find(&["intermediate_size"]) {
        Some(Value::Array(w)) if w.len() == n => w.iter().map(|v| v.as_u64().ok_or_else(|| invalid("config.json: `intermediate_size` holds a non-width"))).collect::<io::Result<_>>()?,
        Some(v) if v.is_u64() => vec![v.as_u64().unwrap_or(0); n],
        other => return Err(invalid(format!("config.json: `intermediate_size` is {other:?}, not a width or one a block"))),
    };
    let sparse = n.min(10);
    let pattern: Vec<f64> = match h.find(&["activation_sparsity_pattern"]) {
        Some(Value::Array(p)) => p.iter().map(|v| v.as_f64().unwrap_or(f64::NAN)).collect(),
        Some(v) if v.is_number() => vec![v.as_f64().unwrap_or(f64::NAN); n],
        _ => (0..n).map(|l| if n > 10 && l < 10 { 0.95 } else { 0.0 }).collect(),
    };
    if pattern.len() != n || pattern.iter().enumerate().any(|(l, &s)| s != if l < sparse { 0.95 } else { 0.0 }) {
        return Err(invalid(format!(
            "config.json: `activation_sparsity_pattern` is {pattern:?}: llama.cpp's gemma3n sparsifies the first ten blocks' gates at 0.95 and no others"
        )));
    }
    let shared = h.u64(&["num_kv_shared_layers"]).unwrap_or(0) as usize;
    if shared != n.saturating_sub(20) {
        return Err(invalid(format!(
            "config.json: `num_kv_shared_layers` is {shared}: llama.cpp's gemma3n shares the cache of every block from the 20th, {} of these {n}",
            n.saturating_sub(20)
        )));
    }
    if n > 20 && !(windowed[18] && !windowed[19]) {
        return Err(invalid("config.json: `layer_types` makes blocks 18 and 19 other than a windowed block and a global one, the caches llama.cpp's gemma3n shares"));
    }
    Ok((0..n).map(|l| (windowed[l], widths[l], l < sparse)).collect())
}

/// Jamba's blocks as the converter reads them (`JambaModel`): which attend (every `attn_layer_period`-th from
/// `attn_layer_offset`, the rest scan) and which route to experts (every `expert_layer_period`-th from
/// `expert_layer_offset`, where there is more than one expert).
fn jamba_blocks(h: &Hparams) -> io::Result<Vec<(bool, bool)>> {
    let n = h.need_u64("num_hidden_layers")? as usize;
    let (attn_offset, attn_period) = (h.need_u64("attn_layer_offset")? as usize, h.need_u64("attn_layer_period")? as usize);
    let (moe_offset, moe_period) = (h.need_u64("expert_layer_offset")? as usize, h.need_u64("expert_layer_period")? as usize);
    if attn_period == 0 || moe_period == 0 {
        return Err(invalid("config.json: a layer period of zero"));
    }
    let experts = h.u64(&["num_local_experts", "num_experts"]).unwrap_or(1);
    if h.u64(&["num_experts_per_tok"]).is_some_and(|k| k == 0 || k > experts) {
        return Err(invalid(format!("config.json: `num_experts_per_tok` is not a count of the {experts} experts")));
    }
    Ok((0..n)
        .map(|l| {
            let attends = l >= attn_offset && (l - attn_offset) % attn_period == 0;
            let routes = experts > 1 && l >= moe_offset && (l - moe_offset) % moe_period == 0;
            (attends, routes)
        })
        .collect())
}

/// Gemma 4's blocks as the converter reads them (`Gemma4Model.set_gguf_parameters`): which blocks attend through the
/// window (`layer_types`, the last a global block, as transformers makes it), the global blocks' head width
/// (`global_head_dim`, else the first global block's override in `per_layer_config`, which transformers saves in its
/// place) and KV heads (`num_global_key_value_heads`, else that override's, else `num_key_value_heads`), and each block's
/// FFN width (doubled from the first block that shares its cache where `use_double_wide_mlp`).
///
/// transformers reads a global block's head width and KV heads from its `per_layer_config` override where the
/// configuration holds one, the windowed blocks' where that override is silent; with none, it builds the overrides from
/// `global_head_dim` and, only beside `attention_k_eq_v`, `num_global_key_value_heads`. A global block whose widths
/// transformers reads unlike the converter's one pair, or an override on a windowed block or of another key, is refused
/// naming the key that sets it.
struct Gemma4Blocks {
    swa: Vec<bool>,
    head_full: u64,
    kv_full: Option<u64>,
    ff: Vec<u64>,
}

fn gemma4_blocks(h: &Hparams) -> io::Result<Gemma4Blocks> {
    let n = h.need_u64("num_hidden_layers")? as usize;
    let types = h.find(&["layer_types"]).and_then(Value::as_array).ok_or_else(|| invalid("config.json: no `layer_types`, which the converter reads"))?;
    let swa: Vec<bool> = types
        .iter()
        .map(|t| match t.as_str() {
            Some("sliding_attention") => Ok(true),
            Some("full_attention") => Ok(false),
            _ => Err(invalid(format!("config.json: `layer_types` names {t}, neither sliding nor full attention"))),
        })
        .collect::<io::Result<_>>()?;
    if swa.len() != n || swa.last() != Some(&false) {
        return Err(invalid(format!(
            "config.json: `layer_types` is not {n} kinds ending in a global block, the stack transformers builds and the converter reads"
        )));
    }
    let overrides: Vec<(usize, Map<String, Value>)> = match h.find(&["per_layer_config"]) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(o)) => o
            .iter()
            .map(|(l, c)| {
                let l: usize = l.parse().map_err(|_| invalid(format!("config.json: `per_layer_config` names block `{l}`")))?;
                let c = c.as_object().cloned().ok_or_else(|| invalid(format!("config.json: `per_layer_config` block {l} is not an object")))?;
                Ok((l, c))
            })
            .collect::<io::Result<_>>()?,
        Some(_) => return Err(invalid("config.json: `per_layer_config` is not an object")),
    };
    let saved = !matches!(h.find(&["per_layer_config"]), None | Some(Value::Null));
    let first = |key: &str| overrides.iter().find(|(l, c)| *l < n && !swa[*l] && c.contains_key(key)).and_then(|(_, c)| c[key].as_u64());
    let head_full = h.u64(&["global_head_dim"]).or_else(|| first("head_dim")).ok_or_else(|| invalid("config.json: no global head width, which the converter reads"))?;
    let kv_full = h.u64(&["num_global_key_value_heads"]).or_else(|| first("num_key_value_heads"));
    for (l, c) in &overrides {
        if let Some(key) = c.keys().find(|k| *l >= n || swa[*l] || !matches!(k.as_str(), "head_dim" | "num_key_value_heads")) {
            return Err(invalid(format!(
                "config.json: `per_layer_config` sets block {l}'s `{key}`; the GGUF carries a head width and KV count for the global blocks alone"
            )));
        }
    }
    let (head_local, kv_local) = (h.u64(&["head_dim"]), h.u64(&["num_key_value_heads"]));
    let gated = h.find(&["attention_k_eq_v"]).and_then(Value::as_bool) == Some(true);
    for l in (0..n).filter(|&l| !swa[l]) {
        let set = |key: &str| overrides.iter().find(|(b, _)| *b == l).and_then(|(_, c)| c.get(key)).and_then(Value::as_u64);
        let (head, kv) = if saved {
            (set("head_dim").or(head_local), set("num_key_value_heads").or(kv_local))
        } else {
            (Some(head_full), if gated { kv_full.or(kv_local) } else { kv_local })
        };
        let (key, what, theirs, ours) = if head != Some(head_full) {
            ("per_layer_config", "a head width", head, Some(head_full))
        } else if kv != kv_full.or(kv_local) {
            (if saved { "per_layer_config" } else { "num_global_key_value_heads" }, "KV heads", kv, kv_full.or(kv_local))
        } else {
            continue;
        };
        let show = |v: Option<u64>| v.map_or("none".to_string(), |v| v.to_string());
        return Err(invalid(format!(
            "config.json: `{key}` leaves transformers reading {what} of {} for block {l}, where the converter writes {} for every global block{}",
            show(theirs),
            show(ours),
            if saved { "" } else { " (transformers reads `num_global_key_value_heads` only beside `attention_k_eq_v`)" }
        )));
    }
    let ff = h.need_u64("intermediate_size")?;
    let shared_from = n.saturating_sub(h.u64(&["num_kv_shared_layers"]).unwrap_or(0) as usize);
    let double = h.find(&["use_double_wide_mlp"]).and_then(Value::as_bool) == Some(true);
    let ff = (0..n).map(|l| if double && l >= shared_from { 2 * ff } else { ff }).collect();
    Ok(Gemma4Blocks { swa, head_full, kv_full, ff })
}

/// RWKV-7's head width and LayerNorm epsilon as the converter reads them (`Rwkv7Model.set_gguf_parameters`): the Hugging
/// Face port's `head_size` and `layer_norm_epsilon` where the configuration states both, else fla's `head_dim` and
/// `norm_eps`.
fn rwkv7_head_and_eps(h: &Hparams) -> io::Result<(u64, f64)> {
    let number = |k: &str| h.map.get(k).and_then(Value::as_f64).ok_or_else(|| invalid(format!("config.json: no `{k}`, which the converter reads")));
    let (head, eps) = if rwkv7_port(h) { ("head_size", "layer_norm_epsilon") } else { ("head_dim", "norm_eps") };
    Ok((h.need_u64(head)?, number(eps)?))
}

/// Whether the converter reads an RWKV-7 configuration's head width and epsilon by the Hugging Face port's names: both
/// stated.
fn rwkv7_port(h: &Hparams) -> bool {
    h.map.contains_key("head_size") && h.map.contains_key("layer_norm_epsilon")
}

/// RWKV-7's low-rank pairs' ranks as the converter reads them: the Hugging Face port's four `lora_rank_*` where the
/// configuration states all four, else fla's four `*_low_rank_dim`; a null rank the converter's default,
/// `max(1, round(d^e·m / 32))·32` rounded half to even as Python rounds, for the decay's, the learning rate's, the value
/// mix's and the gate's `(e, m)` of (½, 1.8), (½, 1.8), (½, 1.3) and (0.8, 0.6).
fn rwkv7_ranks(h: &Hparams) -> io::Result<[u64; 4]> {
    let d = h.need_u64("hidden_size")? as f64;
    let port = ["lora_rank_decay", "lora_rank_iclr", "lora_rank_value_residual_mix", "lora_rank_gate"];
    let fla = ["decay_low_rank_dim", "a_low_rank_dim", "v_low_rank_dim", "gate_low_rank_dim"];
    let keys = if port.iter().all(|k| h.map.contains_key(*k)) { port } else { fla };
    let defaults = [(0.5, 1.8), (0.5, 1.8), (0.5, 1.3), (0.8, 0.6)];
    let mut ranks = [0; 4];
    for (i, key) in keys.iter().enumerate() {
        ranks[i] = match h.map.get(*key) {
            None => return Err(invalid(format!("config.json: no `{key}`, which the converter reads (null for its default)"))),
            Some(Value::Null) => {
                let (e, m) = defaults[i];
                (d.powf(e) * m / 32.0).round_ties_even().max(1.0) as u64 * 32
            }
            Some(v) => v.as_u64().ok_or_else(|| invalid(format!("config.json: `{key}` is not a rank")))?,
        };
    }
    Ok(ranks)
}

/// MiniMax's own configuration keys (MiniMax-Text-01's and M1's `MiniMaxText01Config`), which transformers'
/// `MiniMaxForCausalLM` names otherwise.
const MINIMAX01_OWN_KEYS: &[&str] = &[
    "attn_type_list",
    "layernorm_full_attention_alpha",
    "layernorm_full_attention_beta",
    "layernorm_linear_attention_alpha",
    "layernorm_linear_attention_beta",
    "layernorm_mlp_alpha",
    "layernorm_mlp_beta",
];

/// transformers' names for them (`MiniMaxConfig`), and its chunk length (a computation order alone).
const MINIMAX01_TRANSFORMERS_KEYS: &[&str] = &[
    "layer_types",
    "full_attn_alpha_factor",
    "full_attn_beta_factor",
    "linear_attn_alpha_factor",
    "linear_attn_beta_factor",
    "mlp_alpha_factor",
    "mlp_beta_factor",
    "block_size",
];

/// Which configuration a MiniMax-01 checkpoint states: MiniMax's own (`MiniMaxText01ForCausalLM`,
/// `MiniMaxM1ForCausalLM`, the classes the converter registers) or transformers' (`MiniMaxForCausalLM`, MiniMax-Text-01-hf's).
struct Minimax01Names {
    class_is_minimax: bool,
}

fn minimax01_names(h: &Hparams) -> Minimax01Names {
    Minimax01Names { class_is_minimax: h.architecture != "MiniMaxForCausalLM" }
}

/// MiniMax-01's lightning blocks as llama.cpp's `minimax-01` reads them with no `attention.recurrent_layers`, which the
/// converter does not write: all but every eighth. The configuration's attention kinds (`attn_type_list`, 1 for full
/// attention; transformers' `layer_types`) must say the same; transformers' default where `layer_types` is unset
/// alternates from full attention, and is refused.
fn minimax01_layers(h: &Hparams) -> io::Result<Vec<bool>> {
    let n = h.need_u64("num_hidden_layers")? as usize;
    let (key, stated): (&str, Option<Vec<Option<bool>>>) = if minimax01_names(h).class_is_minimax {
        ("attn_type_list", h.find(&["attn_type_list"]).and_then(Value::as_array).map(|a| a.iter().map(|t| t.as_u64().map(|t| t == 0)).collect()))
    } else {
        let kind = |t: &Value| match t.as_str() {
            Some("linear_attention") => Some(true),
            Some("full_attention") => Some(false),
            _ => None,
        };
        ("layer_types", h.find(&["layer_types"]).and_then(Value::as_array).map(|a| a.iter().map(kind).collect()))
    };
    let fixed: Vec<bool> = (0..n).map(|l| (l + 1) % 8 != 0).collect();
    let stated = stated.ok_or_else(|| invalid(format!("config.json: no `{key}`; llama.cpp's minimax-01 attends fully on every eighth block alone, and the file must say which blocks it means")))?;
    let stated: Vec<bool> = stated.into_iter().collect::<Option<_>>().ok_or_else(|| invalid(format!("config.json: `{key}` names a kind that is neither linear nor full attention")))?;
    if stated != fixed {
        return Err(invalid(format!(
            "config.json: `{key}` is not full attention on every eighth block alone, the one pattern llama.cpp's minimax-01 reads where the GGUF names none"
        )));
    }
    Ok(stated)
}

/// The scale MiniMax-01's residuals take (`residual_scale`): its three sublayers' `alpha`, equal and each beside a
/// `beta` of 1, as the converter asserts.
fn minimax01_residual_scale(h: &Hparams) -> io::Result<f64> {
    let names: [(&str, &str); 3] = if minimax01_names(h).class_is_minimax {
        [
            ("layernorm_full_attention_alpha", "layernorm_full_attention_beta"),
            ("layernorm_linear_attention_alpha", "layernorm_linear_attention_beta"),
            ("layernorm_mlp_alpha", "layernorm_mlp_beta"),
        ]
    } else {
        [("full_attn_alpha_factor", "full_attn_beta_factor"), ("linear_attn_alpha_factor", "linear_attn_beta_factor"), ("mlp_alpha_factor", "mlp_beta_factor")]
    };
    let mut scale = None;
    for (alpha, beta) in names {
        let a = h.f64(&[alpha]).ok_or_else(|| invalid(format!("config.json: no `{alpha}`, which the converter reads")))?;
        if scale.is_some_and(|s| s != a) {
            return Err(invalid(format!("config.json: `{alpha}` differs from the others; the converter asserts one residual scale")));
        }
        scale = Some(a);
        if h.f64(&[beta]).is_none_or(|b| b != 1.0) {
            return Err(invalid(format!("config.json: `{beta}` is not 1.0, which the converter asserts")));
        }
    }
    Ok(scale.unwrap_or(1.0))
}

/// A Nemotron-H block: Mamba-2's scan, attention, or an FFN.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NemotronBlock {
    Scan,
    Attention,
    Ffn,
}

/// Nemotron-H's blocks as the converter reads them (`NemotronHModel`): `hybrid_override_pattern`'s `M`, `*` and `-`, one
/// a block, as many as `num_hidden_layers`. Its mixture blocks (`E`, Nemotron 3's) are refused, as is a
/// `layers_block_type` list naming an FFN block (`mlp`), which the converter writes with no FFN width and llama.cpp
/// reads as a scan.
fn nemotron_h_blocks(h: &Hparams) -> io::Result<Vec<NemotronBlock>> {
    let n = h.need_u64("num_hidden_layers")? as usize;
    let blocks: Vec<NemotronBlock> = match (h.find(&["hybrid_override_pattern"]), h.find(&["layers_block_type"])) {
        (Some(Value::String(p)), _) => p
            .chars()
            .map(|c| match c {
                'M' => Ok(NemotronBlock::Scan),
                '*' => Ok(NemotronBlock::Attention),
                '-' => Ok(NemotronBlock::Ffn),
                'E' => Err(invalid("config.json: `hybrid_override_pattern` names mixture blocks (`E`), Nemotron 3's, which this reader does not read yet")),
                other => Err(invalid(format!("config.json: `hybrid_override_pattern` names a block `{other}`"))),
            })
            .collect::<io::Result<_>>()?,
        (_, Some(Value::Array(types))) => types
            .iter()
            .map(|t| match t.as_str() {
                Some("mamba" | "linear_attention") => Ok(NemotronBlock::Scan),
                Some("attention" | "full_attention") => Ok(NemotronBlock::Attention),
                Some("mlp") => Err(invalid(
                    "config.json: `layers_block_type` lists an FFN block (`mlp`), which the converter writes with no FFN width and llama.cpp reads as a scan",
                )),
                Some("moe") => Err(invalid("config.json: `layers_block_type` lists mixture blocks (`moe`), Nemotron 3's, which this reader does not read yet")),
                _ => Err(invalid(format!("config.json: `layers_block_type` lists a block {t}"))),
            })
            .collect::<io::Result<_>>()?,
        _ => return Err(invalid("config.json: no `hybrid_override_pattern`, the blocks the converter reads")),
    };
    if blocks.len() != n {
        return Err(invalid(format!("config.json: {} blocks in the pattern for {n} layers", blocks.len())));
    }
    Ok(blocks)
}

/// Nemotron-H's scan as the converter reads it: the inner width `mamba_num_heads · mamba_head_dim` (not `expand` times the
/// width), the state `ssm_state_size`, the groups `n_groups`; the heads must split into the groups.
fn nemotron_h_widths(h: &Hparams) -> io::Result<(u64, u64, u64, u64)> {
    let heads = h.need_u64("mamba_num_heads")?;
    let head = h.need_u64("mamba_head_dim")?;
    let groups = h.u64(&["n_groups", "num_groups"]).ok_or_else(|| invalid("config.json: no `n_groups`, which the converter reads"))?;
    if groups == 0 || heads % groups != 0 {
        return Err(invalid(format!("config.json: {heads} scan heads in {groups} groups")));
    }
    if h.u64(&["mamba_num_groups"]).is_some_and(|g| g != groups) {
        return Err(invalid("config.json: `mamba_num_groups` differs from `n_groups`, which the converter and transformers read"));
    }
    let state = h.u64(&["state_size", "d_state", "state_dim", "ssm_state_size"]).ok_or_else(|| invalid("config.json: no `ssm_state_size`"))?;
    Ok((heads * head, heads, state, groups))
}

/// Falcon-H1's scan widths as the converter reads them (`FalconH1Model`, `Mamba2Model`): the width, the inner width
/// `mamba_d_ssm`, the head width `mamba_d_head` (which the converter divides the inner width by, so "auto" is refused),
/// and the groups `mamba_n_groups`; `mamba_n_heads` must be the heads they make, as transformers checks.
fn falcon_h1_widths(h: &Hparams) -> io::Result<(u64, u64, u64, u64)> {
    let d = h.need_u64("hidden_size")?;
    let inner = h.need_u64("mamba_d_ssm")?;
    let head = match h.map.get("mamba_d_head") {
        Some(Value::Number(n)) => n.as_u64().filter(|&w| w > 0).ok_or_else(|| invalid("config.json: `mamba_d_head` is not a width"))?,
        Some(other) => return Err(invalid(format!("config.json: `mamba_d_head` is {other}, which the converter divides the inner width by"))),
        None => return Err(invalid("config.json: no `mamba_d_head`, which the converter reads the heads from")),
    };
    let groups = h.need_u64("mamba_n_groups")?;
    if inner % head != 0 || groups == 0 || (inner / head) % groups != 0 {
        return Err(invalid(format!("config.json: an inner width of {inner} in heads of {head} and {groups} groups")));
    }
    if h.u64(&["mamba_n_heads"]).is_some_and(|n| n != inner / head) {
        return Err(invalid(format!("config.json: `mamba_n_heads` is not the inner width over the head width ({})", inner / head)));
    }
    Ok((d, inner, head, groups))
}

/// Falcon-H1's MuP multipliers, each as the converter reads it: `mlp_multipliers` two (the gate's, the down
/// projection's), `ssm_multipliers` five (`z`, `x`, `B`, `C` and the time steps of the scan's input projection), the
/// rest one each, defaulting to one where transformers' configuration does.
struct FalconH1Multipliers {
    embedding: f64,
    head: f64,
    attention_in: f64,
    attention_out: f64,
    key: f64,
    mlp: [f64; 2],
    ssm_in: f64,
    ssm_out: f64,
    ssm: [f64; 5],
}

fn falcon_h1_multipliers(h: &Hparams) -> io::Result<FalconH1Multipliers> {
    let one = |k: &str| -> io::Result<f64> {
        match h.map.get(k) {
            None | Some(Value::Null) => Ok(1.0),
            Some(v) => v.as_f64().ok_or_else(|| invalid(format!("config.json: `{k}` is not a number"))),
        }
    };
    fn list<const N: usize>(h: &Hparams, k: &str) -> io::Result<[f64; N]> {
        match h.map.get(k) {
            None | Some(Value::Null) => Ok([1.0; N]),
            Some(Value::Array(a)) if a.len() == N => {
                let mut out = [1.0; N];
                for (o, v) in out.iter_mut().zip(a) {
                    *o = v.as_f64().ok_or_else(|| invalid(format!("config.json: `{k}` holds a non-number")))?;
                }
                Ok(out)
            }
            Some(other) => Err(invalid(format!("config.json: `{k}` is {other}, not {N} multipliers"))),
        }
    }
    Ok(FalconH1Multipliers {
        embedding: one("embedding_multiplier")?,
        head: one("lm_head_multiplier")?,
        attention_in: one("attention_in_multiplier")?,
        attention_out: one("attention_out_multiplier")?,
        key: one("key_multiplier")?,
        mlp: list::<2>(h, "mlp_multipliers")?,
        ssm_in: one("ssm_in_multiplier")?,
        ssm_out: one("ssm_out_multiplier")?,
        ssm: list::<5>(h, "ssm_multipliers")?,
    })
}

/// `form` times `a` and then `b`, as the converter folds a Falcon-H1 multiplier into a weight.
fn scaled(form: Form, a: f64, b: f64) -> Form {
    Form::Map { inner: Box::new(form), op: Op::Scale(a as f32, b as f32) }
}

/// The rotation base transformers' `Lfm2Config` takes where a configuration names none.
const LFM2_THETA: f64 = 1_000_000.0;

/// Which of LFM2's blocks attend, as `Lfm2Config` reads them: `layer_types` (`full_attention` or `conv`) as listed, else
/// the blocks `full_attn_idxs` lists, else every block; where both are stated they must agree.
fn lfm2_layers(h: &Hparams) -> io::Result<Vec<bool>> {
    let refuse = |k: &str, why: String| Err(invalid(format!("config.json: `{k}` {why}")));
    let n = h.need_u64("num_hidden_layers")? as usize;
    let listed: Option<Vec<usize>> = h
        .find(&["full_attn_idxs"])
        .map(|v| v.as_array().map(|a| a.iter().filter_map(Value::as_u64).map(|i| i as usize).collect()).unwrap_or_default());
    let attends: Vec<bool> = match h.find(&["layer_types"]).and_then(Value::as_array) {
        Some(types) => {
            if types.len() != n {
                return refuse("layer_types", format!("names {} blocks of {n}", types.len()));
            }
            types
                .iter()
                .map(|t| match t.as_str() {
                    Some("full_attention") => Ok(true),
                    Some("conv") => Ok(false),
                    other => Err(invalid(format!("config.json: `layer_types` names a block `{other:?}`, neither conv nor full_attention"))),
                })
                .collect::<io::Result<_>>()?
        }
        None => match &listed {
            Some(idxs) => (0..n).map(|l| idxs.contains(&l)).collect(),
            None => vec![true; n],
        },
    };
    if let Some(idxs) = listed {
        if (0..n).filter(|&l| attends[l]).collect::<Vec<_>>() != idxs {
            return refuse("full_attn_idxs", "names other attention blocks than layer_types, which transformers reads".into());
        }
    }
    Ok(attends)
}

/// LFM2's FFN width as the converter writes it (`LFM2Model._add_feed_forward_length`): `block_ff_dim` (else
/// `intermediate_size`), and where `block_auto_adjust_ff_dim` says so, two thirds of it times `block_ffn_dim_multiplier`,
/// up to a multiple of `block_multiple_of`. transformers rounds up only beside a multiplier; a configuration whose
/// width the two compute apart is refused.
fn lfm2_ffn(h: &Hparams) -> io::Result<u64> {
    let ff = h.u64(&["block_ff_dim", "intermediate_size"]).unwrap_or(12288);
    if !h.find(&["block_auto_adjust_ff_dim"]).and_then(Value::as_bool).unwrap_or(true) {
        return Ok(ff);
    }
    let multiple = h.u64(&["block_multiple_of"]).unwrap_or(256);
    let multiplier = match h.map.get("block_ffn_dim_multiplier") {
        None => Some(1.0),
        Some(Value::Null) => None,
        Some(v) => Some(v.as_f64().ok_or_else(|| invalid("config.json: `block_ffn_dim_multiplier` is not a number"))?),
    };
    let two_thirds = (2.0 * ff as f64 / 3.0) as u64;
    let scaled = multiplier.map_or(two_thirds, |m| (m * two_thirds as f64) as u64);
    let rounded = multiple * scaled.div_ceil(multiple);
    if multiplier.is_none() && rounded != scaled {
        return Err(invalid(format!(
            "config.json: `block_ffn_dim_multiplier` is null: transformers builds an FFN of {scaled}, the converter writes {rounded}"
        )));
    }
    Ok(rounded)
}

/// Each layer's DSA indexer kind, as transformers' configuration classes derive it (`GlmMoeDsaConfig.__post_init__`):
/// `indexer_types` as listed (`full` or `shared`), else `index_topk_pattern` (`F` and `S`, or a list), else full where
/// `max(l − index_skip_topk_offset + 1, 0)` is a multiple of `index_topk_freq` (every layer at transformers' defaults of
/// 1 and 2); DeepSeek-V3.2's every layer full. Its leading dense layers as `mlp_layer_types` lists them, where it does,
/// are `first_k_dense_replace`'s, as the GGUF says them.
fn dsa_layers(h: &Hparams) -> io::Result<Vec<bool>> {
    let refuse = |k: &str, why: String| Err(invalid(format!("config.json: `{k}` {why}")));
    let n = h.need_u64("num_hidden_layers")? as usize;
    // transformers saves every DSA layer as `indexed_attention`, the one kind it builds.
    if let Some(types) = h.find(&["layer_types"]).and_then(Value::as_array) {
        if types.len() != n || types.iter().any(|t| t.as_str() != Some("indexed_attention")) {
            return refuse("layer_types", format!("is not {n} indexed_attention layers, the attention every DSA layer is"));
        }
    }
    if let Some(types) = h.find(&["mlp_layer_types"]).and_then(Value::as_array) {
        let dense = h.u64(&["first_k_dense_replace"]).unwrap_or(0) as usize;
        let stated: Vec<Option<&str>> = types.iter().map(Value::as_str).collect();
        let lead: Vec<Option<&str>> = (0..n).map(|l| Some(if l < dense { "dense" } else { "sparse" })).collect();
        if stated != lead {
            return refuse("mlp_layer_types", format!("is not {dense} dense layers then sparse ones, the lead the GGUF carries"));
        }
    }
    if h.architecture != "GlmMoeDsaForCausalLM" {
        return Ok(vec![true; n]);
    }
    if h.find(&["indexer_rope_interleave"]).and_then(Value::as_bool) == Some(false) {
        return refuse("indexer_rope_interleave", "is false; GLM-5's indexer is read with interleaved rotation".into());
    }
    let named = |kinds: Vec<Option<&str>>, key: &str| -> io::Result<Vec<bool>> {
        if kinds.len() != n {
            return refuse(key, format!("names {} layers of {n}", kinds.len()));
        }
        kinds
            .into_iter()
            .map(|k| match k {
                Some("full" | "F") => Ok(true),
                Some("shared" | "S") => Ok(false),
                other => Err(invalid(format!("config.json: `{key}` names a layer `{other:?}`, neither full nor shared"))),
            })
            .collect()
    };
    if let Some(types) = h.find(&["indexer_types"]).and_then(Value::as_array) {
        return named(types.iter().map(Value::as_str).collect(), "indexer_types");
    }
    match h.find(&["index_topk_pattern"]) {
        Some(Value::String(p)) => return named(p.split("").filter(|c| !c.is_empty()).map(Some).collect(), "index_topk_pattern"),
        Some(Value::Array(p)) => return named(p.iter().map(Value::as_str).collect(), "index_topk_pattern"),
        _ => {}
    }
    let freq = h.u64(&["index_topk_freq"]).unwrap_or(1).max(1) as usize;
    let offset = h.u64(&["index_skip_topk_offset"]).unwrap_or(2) as usize;
    Ok((0..n).map(|l| (l + 1).saturating_sub(offset) % freq == 0).collect())
}

/// What Mistral 3's text model is, as its converter class (`conversion/mistral3.py`) reads it, and the settings of it
/// the mistral3 GGUF cannot carry. The class writes a Mistral 4 text model as llama.cpp's mistral4 (DeepSeek-V2's
/// latent attention), refused here; Ministral 3's (`ministral3`) as Mistral's with YaRN's log multiplier
/// (`mscale_all_dim`) and the attention temperature (`llama_4_scaling_beta`); Mistral Small's (`mistral`, the type a
/// wrapper's text configuration takes where it names none) as Mistral's alone. Returns whether the text model is
/// Ministral 3's.
fn mistral3_text(h: &Hparams) -> io::Result<bool> {
    let refuse = |k: &str, why: &str| Err(invalid(format!("config.json: `{k}` {why}")));
    let ministral3 = match h.map.get("model_type").and_then(Value::as_str) {
        Some("ministral3") => true,
        Some("mistral" | "mistral3") => false,
        Some("mistral4") => {
            return refuse("model_type", "is mistral4: Mistral 4's text model (llama.cpp's mistral4, DeepSeek-V2's latent attention) is not read yet")
        }
        other => return refuse("model_type", &format!("is {other:?}, a text model the converter's Mistral3 class does not write")),
    };
    match h.map.get("sliding_window") {
        Some(Value::Null) => {}
        None if ministral3 => {}
        None => return refuse("sliding_window", "is unset, which transformers' MistralConfig reads as a window of 4096; the mistral3 GGUF carries none"),
        Some(w) => return refuse("sliding_window", &format!("is {w}; the mistral3 GGUF carries no window, and llama.cpp attends every position")),
    }
    if !ministral3 {
        return Ok(false);
    }
    // The converter asserts YaRN and reads both settings without a default; llama.cpp computes YaRN's magnitude as
    // `get_mscale(f, 1)/get_mscale(f, mscale_all_dim)` (`mscale` taken as 1) where transformers takes
    // `attention_factor`, else `get_mscale(f, mscale)/get_mscale(f, mscale_all_dim)` when both are set, else
    // `get_mscale(f)`; and it floors and ceils the correction range whatever `truncate` says.
    let rope = full_rope(&rope_parameters(h));
    let number = |k: &str| rope.get(k).filter(|v| !v.is_null()).and_then(Value::as_f64);
    if rope.get("rope_type").and_then(Value::as_str) != Some("yarn") {
        return refuse("rope_parameters", "is not YaRN, which the converter's ministral3 requires");
    }
    for k in ["factor", "mscale_all_dim", "llama_4_scaling_beta", "original_max_position_embeddings"] {
        if number(k).is_none() {
            return refuse("rope_parameters", &format!("names no `{k}`, which the converter's ministral3 reads"));
        }
    }
    if rope.get("truncate").and_then(Value::as_bool) == Some(false) {
        return refuse("rope_parameters", "sets `truncate` false; llama.cpp floors and ceils YaRN's correction range");
    }
    let factor = number("factor").unwrap_or(1.0);
    let get_mscale = |m: f64| if factor <= 1.0 { 1.0 } else { 0.1 * m * factor.ln() + 1.0 };
    let all = number("mscale_all_dim").unwrap_or(0.0);
    let llama_cpp = if all != 0.0 { get_mscale(1.0) / get_mscale(all) } else { get_mscale(1.0) };
    let (stated, mscale) = (number("attention_factor"), number("mscale").unwrap_or(0.0));
    let transformers = match stated {
        Some(a) => a,
        None if mscale != 0.0 && all != 0.0 => get_mscale(mscale) / get_mscale(all),
        None => get_mscale(1.0),
    };
    if transformers != llama_cpp {
        let key = if stated.is_some() { "attention_factor" } else { "mscale" };
        return refuse(
            "rope_parameters",
            &format!("sets YaRN's magnitude to {transformers} by `{key}`, where llama.cpp, taking mscale as 1, computes {llama_cpp}"),
        );
    }
    Ok(true)
}

impl HfCheckpoint {
    /// Read the checkpoint in `dir` as its converted GGUF.
    pub fn open(dir: &Path) -> io::Result<HfCheckpoint> {
        HfCheckpoint::open_with(dir, Recompute::AsConverted)
    }

    /// The architecture and the metadata `dir`'s `config.json` alone writes, refused as [`HfCheckpoint::open`] refuses
    /// it, before any weight is read: a published configuration checked without its checkpoint. Draft blocks only the
    /// tensors count (Qwen3-Next's) are left uncounted.
    pub fn read_configuration(dir: &Path) -> io::Result<(String, BTreeMap<String, MetaValue>)> {
        let (family, h) = configuration(dir)?;
        let mut w = Writer { arch: family.arch().to_string(), meta: BTreeMap::new() };
        let rope = rope_parameters(&h);
        let block_count = h.need_u64("num_hidden_layers")? + stated_draft_blocks(family, &h).unwrap_or(0);
        base_parameters(&h, &rope, &mut w, block_count, family)?;
        family_parameters(family, &h, &rope, &mut w, block_count)?;
        Ok((family.arch().to_string(), w.meta))
    }

    /// The tokenizer keys `dir`'s tokenizer files and configuration write (`tokenizer.*`, with `general.architecture`),
    /// before any weight is read: a published tokenizer checked without its checkpoint.
    pub fn read_tokenizer(dir: &Path) -> io::Result<BTreeMap<String, MetaValue>> {
        let (family, h) = configuration(dir)?;
        vocab::tokenizer_meta(dir, &h, family)
    }

    /// Read the checkpoint in `dir`, presenting computed values as `recompute` says.
    pub fn open_with(dir: &Path, recompute: Recompute) -> io::Result<HfCheckpoint> {
        let (family, h) = configuration(dir)?;
        if family == Family::Minimax01 && !dir.join("model.safetensors.index.json").is_file() {
            return Err(invalid(format!(
                "{}: no model.safetensors.index.json, which the converter's MiniMax-01 class reads the embedding's shard from",
                dir.display()
            )));
        }

        let paths = safetensors::shard_paths(dir)?;
        let shards: Vec<Shard> = paths.iter().map(|p| Shard::open(p)).collect::<io::Result<_>>()?;
        let mut located = HashMap::new();
        for (i, s) in shards.iter().enumerate() {
            for name in s.tensors.keys() {
                if located.insert(name.clone(), i).is_some() {
                    return Err(invalid(format!("{}: tensor {name} is in two shards", dir.display())));
                }
            }
        }
        let (scaled, consumed) = fp8_scales(&h, &located)?;
        let mut c = HfCheckpoint {
            dir: dir.to_path_buf(),
            recompute,
            family,
            hparams: Hparams { map: Map::new(), architecture: String::new() },
            tokenizer: OnceLock::new(),
            arch: family.arch().to_string(),
            meta: BTreeMap::new(),
            views: Vec::new(),
            index: HashMap::new(),
            shards,
            located,
            scaled,
            consumed,
            unread: Vec::new(),
            skipped: Vec::new(),
        };
        let mut w = Writer { arch: c.arch.clone(), meta: BTreeMap::new() };
        let rope = rope_parameters(&h);
        let block_count = h.need_u64("num_hidden_layers")? + stated_draft_blocks(family, &h).unwrap_or_else(|| draft_layers(&c));
        base_parameters(&h, &rope, &mut w, block_count, family)?;
        family_parameters(family, &h, &rope, &mut w, block_count)?;
        c.ple_parameters(&mut w)?;
        c.meta = w.meta;
        c.build_views(family, &h, &rope)?;
        c.hparams = h;
        Ok(c)
    }

    /// The tokenizer keys the converter writes from the directory's tokenizer files (`tokenizer.*`), with
    /// `general.architecture`: what [`Vocab::from_meta`](super::tokenizer::Vocab::from_meta) reads. Read on first use.
    pub fn tokenizer_meta(&self) -> io::Result<&BTreeMap<String, MetaValue>> {
        if let Some(m) = self.tokenizer.get() {
            return Ok(m);
        }
        let mut m = vocab::tokenizer_meta(&self.dir, &self.hparams, self.family)?;
        if self.family == Family::Minimax01 {
            let suppressed = self.minimax01_suppressed()?;
            if !suppressed.is_empty() {
                let items = suppressed.into_iter().map(|t| MetaValue::I32(t as i32)).collect();
                m.insert("tokenizer.ggml.suppress_tokens".into(), MetaValue::Array { ty: 5, items });
            }
        }
        Ok(self.tokenizer.get_or_init(|| m))
    }

    /// MiniMax-01's suppressed tokens (`MiniMaxText01Model._get_suppress_tokens`): the rows of `model.embed_tokens` past
    /// the tokenizer's own vocabulary (its model's, the added tokens aside) that are zero throughout, which the
    /// converter writes as `tokenizer.ggml.suppress_tokens` where there are any.
    fn minimax01_suppressed(&self) -> io::Result<Vec<u32>> {
        let name = "model.embed_tokens.weight";
        let shard = &self.shards[*self.located.get(name).ok_or_else(|| invalid(format!("{}: no {name}", self.dir.display())))?];
        let e = &shard.tensors[name];
        let (rows, width) = match e.shape[..] {
            [rows, width] => (rows, width as usize),
            _ => return Err(invalid(format!("{name}: {:?} is not two-dimensional", e.shape))),
        };
        let row_bytes = width * e.dtype.bytes();
        let zero = |b: &[u8]| -> bool {
            match e.dtype {
                Dtype::F64 => b.chunks_exact(8).all(|c| u64::from_le_bytes(c.try_into().unwrap()) << 1 == 0),
                Dtype::F32 => b.chunks_exact(4).all(|c| u32::from_le_bytes(c.try_into().unwrap()) << 1 == 0),
                Dtype::F16 | Dtype::Bf16 => b.chunks_exact(2).all(|c| u16::from_le_bytes([c[0], c[1]]) << 1 == 0),
                Dtype::F8E4m3 | Dtype::F8E5m2 => b.iter().all(|&x| x << 1 == 0),
                _ => b.iter().all(|&x| x == 0),
            }
        };
        let mut suppressed = Vec::new();
        for row in vocab::base_vocab_size(&self.dir)? as u64..rows {
            if zero(&shard.read(name, row * row_bytes as u64, row_bytes)?) {
                suppressed.push(row as u32);
            }
        }
        Ok(suppressed)
    }

    /// Tensors of the checkpoint nothing presented reads and the converter does not skip.
    pub fn unread(&self) -> &[String] {
        &self.unread
    }

    /// Tensors the converter skips (a multimodal checkpoint's tower), which this reader skips too.
    pub fn skipped(&self) -> &[String] {
        &self.skipped
    }

    /// Tensors whose values a converter step computes from the stored ones, rather than reads.
    pub fn recomputed(&self) -> Vec<&str> {
        self.views.iter().filter(|v| matches!(v.form, Form::Map { .. })).map(|v| v.name.as_str()).collect()
    }

    /// The safetensors files read.
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn entry(&self, name: &str) -> Option<&Entry> {
        self.located.get(name).map(|&i| &self.shards[i].tensors[name])
    }

    fn need(&self, name: &str) -> io::Result<&Entry> {
        self.entry(name).ok_or_else(|| invalid(format!("{}: no tensor {name}", self.dir.display())))
    }

    fn present(&mut self, name: String, form: Form) -> io::Result<()> {
        let dims = self.dims_of(&form)?;
        self.push(name, dims, form)
    }

    fn push(&mut self, name: String, dims: Vec<u64>, form: Form) -> io::Result<()> {
        let n: u64 = dims.iter().product();
        if n as usize != form.elements() {
            return Err(invalid(format!("{name}: dims {dims:?} hold {n} elements, its view {}", form.elements())));
        }
        self.index.insert(name.clone(), self.views.len());
        self.views.push(View { name, dims, form });
        Ok(())
    }

    /// ggml dims of a form: a one-dimensional source stays one-dimensional; a whole tensor keeps its own shape; a
    /// rearranged one is `[outer, rows, cols]` (or `[outer, cols, rows]` transposed), an outer or row axis of one
    /// dropped; a stack adds its count outermost.
    fn dims_of(&self, form: &Form) -> io::Result<Vec<u64>> {
        Ok(match form {
            Form::Gather { src, outer, m, n, rows, cols, transpose } => {
                let e = self.need(src)?;
                let identity = *outer == 1
                    && !*transpose
                    && rows.len() == *m
                    && cols.len() == *n
                    && rows.iter().enumerate().all(|(i, &r)| i == r)
                    && cols.iter().enumerate().all(|(i, &c)| i == c);
                if e.shape.len() <= 1 {
                    vec![form.elements() as u64]
                } else if identity {
                    e.shape.iter().rev().copied().collect()
                } else {
                    let (r, c) = if *transpose { (cols.len(), rows.len()) } else { (rows.len(), cols.len()) };
                    let mut torch = Vec::new();
                    if *outer > 1 {
                        torch.push(*outer as u64);
                    }
                    if r > 1 || *outer == 1 {
                        torch.push(r as u64);
                    }
                    torch.push(c as u64);
                    torch.into_iter().rev().collect()
                }
            }
            Form::Concat(parts) => {
                let mut d = self.dims_of(&parts[0])?;
                d.push(parts.len() as u64);
                d
            }
            Form::Beside(parts) => {
                let dims = parts.iter().map(|p| self.dims_of(p)).collect::<io::Result<Vec<_>>>()?;
                if dims.iter().any(|d| d.len() != 2 || d[1] != dims[0][1]) {
                    return Err(invalid(format!("parts side by side of different rows: {dims:?}")));
                }
                vec![dims.iter().map(|d| d[0]).sum(), dims[0][1]]
            }
            Form::Map { inner, .. } => self.dims_of(inner)?,
            Form::Values(v) => vec![v.len() as u64],
            Form::Zeros { n, .. } => vec![*n as u64],
        })
    }

    /// The element type of a form: its source's, or f32 for a computed one.
    fn ty_of(&self, form: &Form) -> io::Result<GgmlType> {
        Ok(match form {
            Form::Gather { src, .. } => self.presented(src)?,
            Form::Concat(parts) | Form::Beside(parts) => {
                let t = self.ty_of(&parts[0])?;
                for p in &parts[1..] {
                    if self.ty_of(p)? != t {
                        return Err(invalid("a stack of tensors of different dtypes"));
                    }
                }
                t
            }
            Form::Map { .. } | Form::Values(_) => GgmlType::F32,
            Form::Zeros { ty, .. } => *ty,
        })
    }

    /// The type a stored tensor is presented as: a scaled weight's f32, every other [`ggml_type`]'s.
    fn presented(&self, src: &str) -> io::Result<GgmlType> {
        Ok(if self.scaled.contains_key(src) { GgmlType::F32 } else { ggml_type(self.need(src)?.dtype) })
    }

    /// `count` elements of stored tensor `src` from element `first` (row-major), as [`Self::presented`] gives them: a
    /// scaled weight widened and multiplied by its scale, as the converter's `dequant_simple` writes it; every other
    /// widened.
    fn source(&self, src: &str, first: usize, count: usize) -> io::Result<Vec<u8>> {
        let entry = self.need(src)?;
        let es = entry.dtype.bytes();
        let raw = self.shards[self.located[src]].read(src, (first * es) as u64, count * es)?;
        let Some(scaled) = self.scaled.get(src) else { return Ok(widen(entry.dtype, raw)) };
        let grid = scaled.grid.get_or_init(|| self.scale_grid(src, scaled)).as_ref().map_err(|e| invalid(e.clone()))?;
        let mut values = Vec::with_capacity(count);
        ggml_type(entry.dtype).dequantize(&widen(entry.dtype, raw), &mut values);
        grid.apply(first, &mut values);
        Ok(values.iter().flat_map(|v| v.to_le_bytes()).collect())
    }

    /// The scale of weight `src` over its elements, read whole.
    fn scale_grid(&self, src: &str, scaled: &Scaled) -> Result<ScaleGrid, String> {
        let (weight, scale) = (self.need(src).map_err(|e| e.to_string())?, self.need(&scaled.scale).map_err(|e| e.to_string())?);
        let raw = self.shards[self.located[scaled.scale.as_str()]]
            .read(&scaled.scale, 0, scale.elements() as usize * scale.dtype.bytes())
            .map_err(|e| e.to_string())?;
        let mut values = Vec::with_capacity(scale.elements() as usize);
        ggml_type(scale.dtype).dequantize(&widen(scale.dtype, raw), &mut values);
        let axes = |e: &Entry| e.shape.iter().map(|&d| d as usize).collect::<Vec<usize>>();
        ScaleGrid::new(&axes(weight), &axes(scale), values, scaled.block.as_deref()).map_err(|e| format!("{src} scaled by {}: {e}", scaled.scale))
    }

    /// `count` elements of `form` from element `start`, as bytes of its element type.
    fn read(&self, form: &Form, start: usize, count: usize) -> io::Result<Vec<u8>> {
        match form {
            Form::Gather { src, outer, m, n, rows, cols, transpose } => {
                let es = self.presented(src)?.block_bytes();
                let all_cols = cols.len() == *n && cols.iter().enumerate().all(|(i, &c)| i == c);
                let identity = !*transpose && all_cols && rows.len() == *m && rows.iter().enumerate().all(|(i, &r)| i == r);
                if identity {
                    return self.source(src, start, count);
                }
                let mut out = Vec::with_capacity(count * es);
                if !*transpose {
                    // Each output row is one source row (or columns of it): read only the rows asked for.
                    let (out_c, per) = (cols.len(), rows.len() * cols.len());
                    let mut e = start;
                    while e < start + count {
                        let (b, rem) = (e / per, e % per);
                        let (i, j0) = (rem / out_c, rem % out_c);
                        let take = (out_c - j0).min(start + count - e);
                        let source_row = b * m + rows[i];
                        if all_cols {
                            out.extend(self.source(src, source_row * n + j0, take)?);
                        } else {
                            let row = self.source(src, source_row * n, *n)?;
                            for &c in &cols[j0..j0 + take] {
                                out.extend_from_slice(&row[c * es..(c + 1) * es]);
                            }
                        }
                        e += take;
                    }
                    return Ok(out);
                }
                // A transposed output row is a source column: read the source matrix it lies in once.
                let (out_r, out_c) = (cols.len(), rows.len());
                let per = out_r * out_c;
                let mut block: Option<(usize, Vec<u8>)> = None;
                for e in start..start + count {
                    let b = e / per;
                    let rem = e % per;
                    let (i, j) = (rem / out_c, rem % out_c);
                    let (r, c) = (rows[j], cols[i]);
                    if block.as_ref().map(|(bb, _)| *bb) != Some(b) {
                        debug_assert!(b < *outer);
                        block = Some((b, self.source(src, b * m * n, m * n)?));
                    }
                    let at = (r * n + c) * es;
                    out.extend_from_slice(&block.as_ref().unwrap().1[at..at + es]);
                }
                Ok(out)
            }
            Form::Concat(parts) => {
                let mut out = Vec::new();
                let mut base = 0;
                for p in parts {
                    let len = p.elements();
                    let lo = start.max(base);
                    let hi = (start + count).min(base + len);
                    if lo < hi {
                        out.extend(self.read(p, lo - base, hi - lo)?);
                    }
                    base += len;
                }
                Ok(out)
            }
            Form::Beside(parts) => {
                let widths =
                    parts.iter().map(|p| Ok(self.dims_of(p)?[0] as usize)).collect::<io::Result<Vec<usize>>>()?;
                let row = widths.iter().sum::<usize>();
                let mut out = Vec::new();
                let mut e = start;
                while e < start + count {
                    let (r, mut c) = (e / row, e % row);
                    let mut part = 0;
                    while c >= widths[part] {
                        c -= widths[part];
                        part += 1;
                    }
                    let take = (widths[part] - c).min(start + count - e);
                    out.extend(self.read(&parts[part], r * widths[part] + c, take)?);
                    e += take;
                }
                Ok(out)
            }
            Form::Values(v) => Ok(v[start..start + count].iter().flat_map(|x| x.to_le_bytes()).collect()),
            Form::Zeros { ty, .. } => Ok(vec![0u8; count * ty.block_bytes()]),
            Form::Map { inner, op } => {
                let ty = self.ty_of(inner)?;
                let raw = self.read(inner, start, count)?;
                let mut vals = Vec::with_capacity(count);
                ty.dequantize(&raw, &mut vals);
                let round = |x: f32| match (self.recompute, ty) {
                    (Recompute::AsConverted, GgmlType::F16) => f16_to_f32(f32_to_f16(x)),
                    _ => x,
                };
                Ok(vals
                    .into_iter()
                    .map(|x| match op {
                        Op::AddOne => 1.0f32 + x,
                        // Correctly rounded: the exponential in f64, rounded to f32 once.
                        Op::NegExp => -((x as f64).exp() as f32),
                        Op::Scale(a, b) => round(x * a) * b,
                    })
                    .map(round)
                    .flat_map(f32::to_le_bytes)
                    .collect())
            }
        }
    }

    /// Present every tensor, and record what nothing reads.
    fn build_views(&mut self, f: Family, h: &Hparams, rope: &Map<String, Value>) -> io::Result<()> {
        let names: Vec<String> = {
            let mut v: Vec<String> = self.located.keys().cloned().collect();
            v.sort();
            v
        };
        // The scales an FP8 checkpoint's weights are read with, and the activation scales the converter drops.
        let mut read: BTreeSet<String> = self.consumed.clone();
        let p = decoder_prefix(&h.architecture, &names);
        let head = if p == "language_model.model." { "language_model.lm_head.weight" } else { "lm_head.weight" };
        let n_layers = h.need_u64("num_hidden_layers")? as usize;
        let tied = h.find(&["tie_word_embeddings"]).and_then(Value::as_bool).unwrap_or(false);

        // The converter skips a multimodal checkpoint's tower and, for DeepSeek, the draft layers past the stack.
        for name in &names {
            let s = name.strip_prefix("model.").unwrap_or(name);
            let tower = s.contains("visual.")
                || s.contains("vision.")
                || s.starts_with("vision_")
                || s.starts_with("multi_modal_projector.")
                || (matches!(f, Family::Gemma3n | Family::Gemma4)
                    && (s.starts_with("audio_tower.") || s.starts_with("embed_audio.") || s.starts_with("embed_vision.")));
            let deepseek_draft = f == Family::Deepseek2
                && layer_of(name, "model.layers.").is_some_and(|(l, _)| l >= n_layers);
            // Gemma's and Gemma 2's converters drop a head the checkpoint carries: llama.cpp ties it to the embedding.
            let tied_head = name == head && (tied && f.deepseek() || matches!(f, Family::Gemma | Family::Gemma2));
            if tower || deepseek_draft || tied_head {
                self.skipped.push(name.clone());
                read.insert(name.clone());
            }
        }

        if f == Family::Rwkv7 {
            self.rwkv7_views(h, &names, &mut read)?;
            self.unread = names.into_iter().filter(|n| !read.contains(n)).collect();
            return Ok(());
        }
        if matches!(f, Family::Llama | Family::Llama4 | Family::Mistral3) {
            if let Some(factors) = llama3_rope_factors(h, rope) {
                self.present("rope_freqs.weight".into(), Form::Values(factors))?;
            }
        }

        // Globals: a Mamba-2 checkpoint keeps its stack under `backbone.`, its embedding and output norm by their own
        // names.
        let backbone = matches!(f, Family::Mamba2 | Family::NemotronH);
        let (p, head) = if backbone { ("backbone.", "lm_head.weight") } else { (p, head) };
        let embed = if backbone { "backbone.embeddings.weight".to_string() } else { format!("{p}embed_tokens.weight") };
        let e = self.need(&embed)?.clone();
        let mut embed_form = Form::whole(&embed, &e);
        if f == Family::FalconH1 {
            embed_form = scaled(embed_form, falcon_h1_multipliers(h)?.embedding, 1.0);
        }
        let mut embed_dims: Vec<u64> = e.shape.iter().rev().copied().collect();
        // Gemma 3n's converter pads its embedding, and its per-layer one, with zero rows to the whole vocabulary (the
        // multimodal tokens'), and stacks AltUp's projections three to a tensor.
        if f == Family::Gemma3n {
            let vocab = h.need_u64("vocab_size")?;
            let padded = |me: &Self, name: &str| -> io::Result<(Form, Vec<u64>)> {
                let e = me.need(name)?;
                let (rows, width) = (e.shape[0], e.shape.last().copied().unwrap_or(1));
                if rows > vocab {
                    return Err(invalid(format!("{name}: {rows} rows past a vocabulary of {vocab}")));
                }
                let whole = Form::whole(name, e);
                let ty = me.ty_of(&whole)?;
                let form = if rows < vocab { Form::Concat(vec![whole, Form::Zeros { n: ((vocab - rows) * width) as usize, ty }]) } else { whole };
                Ok((form, vec![width, vocab]))
            };
            (embed_form, embed_dims) = padded(self, &embed)?;
            let per_layer = format!("{p}embed_tokens_per_layer.weight");
            let (form, dims) = padded(self, &per_layer)?;
            self.push("per_layer_token_embd.weight".into(), dims, form)?;
            read.insert(per_layer);
            for (stored, gguf) in [("per_layer_model_projection.weight", "per_layer_model_proj.weight"), ("per_layer_projection_norm.weight", "per_layer_proj_norm.weight")] {
                let name = format!("{p}{stored}");
                let form = Form::whole(&name, self.need(&name)?);
                self.present(gguf.into(), form)?;
                read.insert(name);
            }
            for (stored, gguf) in [("altup_projections", "altup_proj.weight"), ("altup_unembed_projections", "altup_unembd_proj.weight")] {
                let names: Vec<String> = (0..3).map(|i| format!("{p}{stored}.{i}.weight")).collect();
                let parts = names.iter().map(|n| Ok(Form::whole(n, self.need(n)?))).collect::<io::Result<Vec<Form>>>()?;
                self.present(gguf.into(), Form::Concat(parts))?;
                read.extend(names);
            }
        }
        if f == Family::Gemma4 {
            // Gemma 4's per-layer inputs, its global blocks' rotary factors (the converter's: each frequency of the
            // partial factor's share kept, every other divided by 10³⁰, the rotation a quarter of each head turning
            // expressed over the whole head).
            let per_layer = format!("{p}embed_tokens_per_layer.weight");
            if self.entry(&per_layer).is_some() {
                for (stored, gguf) in [
                    ("embed_tokens_per_layer.weight", "per_layer_token_embd.weight"),
                    ("per_layer_model_projection.weight", "per_layer_model_proj.weight"),
                    ("per_layer_projection_norm.weight", "per_layer_proj_norm.weight"),
                ] {
                    let name = format!("{p}{stored}");
                    let form = Form::whole(&name, self.need(&name)?);
                    self.present(gguf.into(), form)?;
                    read.insert(name);
                }
            }
            let blocks = gemma4_blocks(h)?;
            // llama.cpp takes a block's keys as its values where the GGUF holds no `attn_v`, transformers where
            // `attention_k_eq_v` names a global block (its `v_proj` then never built): a block that stores a value
            // projection unlike that, below the blocks that share caches and so store none, is one they compute apart.
            let keyed = h.find(&["attention_k_eq_v"]).and_then(Value::as_bool) == Some(true);
            let from = blocks.swa.len().saturating_sub(h.u64(&["num_kv_shared_layers"]).unwrap_or(0) as usize);
            for (l, &windowed) in blocks.swa.iter().enumerate().take(from) {
                let stored = self.entry(&format!("{p}layers.{l}.self_attn.v_proj.weight")).is_some();
                if stored != (windowed || !keyed) {
                    return Err(invalid(format!(
                        "block {l} {} a `v_proj` beside `attention_k_eq_v` {keyed}; transformers reads {}, llama.cpp {}",
                        if stored { "holds" } else { "holds no" },
                        if stored { "the keys as its values" } else { "a value projection the checkpoint lacks" },
                        if stored { "the stored projection" } else { "the keys as its values" },
                    )));
                }
            }
            let partial = full_rope(rope).get("partial_rotary_factor").and_then(Value::as_f64).unwrap_or(1.0);
            let half = (blocks.head_full / 2) as usize;
            let turning = (blocks.head_full as f64 * partial / 2.0) as usize;
            let factors: Vec<f32> = (0..half).map(|i| if i < turning { 1.0 } else { 1e30 }).collect();
            self.present("rope_freqs.weight".into(), Form::Values(factors))?;
        }
        if matches!(f, Family::Gemma3 | Family::Gemma4) {
            // The converter keeps only the vocabulary's real rows.
            let real = gemma_vocab(&self.dir, h)?;
            if let Form::Gather { rows, .. } = &mut embed_form {
                if real < rows.len() {
                    rows.truncate(real);
                    embed_dims[1] = real as u64;
                }
            }
        }
        self.push("token_embd.weight".into(), embed_dims, embed_form)?;
        read.insert(embed);
        if f == Family::Qwen4Exp {
            // Hyper-connections replace every norm: the last mix of the lanes feeds the head.
            for (stored, gguf) in [
                ("hc_norm.weight", "output_hc_norm.weight"),
                ("input_mix_weight_down.weight", "output_hc_down.weight"),
                ("input_mix_weight_up.weight", "output_hc_up.weight"),
            ] {
                let name = format!("{p}hyper_connection_mixer.{stored}");
                let form = if stored == "hc_norm.weight" { self.norm_form(f, &name)? } else { Form::whole(&name, self.need(&name)?) };
                self.present(gguf.into(), form)?;
                read.insert(name);
            }
        } else if backbone {
            let norm = "backbone.norm_f.weight".to_string();
            let nf = self.norm_form(f, &norm)?;
            self.present("output_norm.weight".into(), nf)?;
            read.insert(norm);
        } else if matches!(f, Family::FalconH1 | Family::Jamba) {
            let norm = format!("{p}final_layernorm.weight");
            let nf = self.norm_form(f, &norm)?;
            self.present("output_norm.weight".into(), nf)?;
            read.insert(norm);
        } else if f == Family::Lfm2 {
            // LFM2 norms the stack's output by `embedding_norm`, which the converter names `token_embd_norm`.
            let norm = format!("{p}embedding_norm.weight");
            let nf = self.norm_form(f, &norm)?;
            self.present("token_embd_norm.weight".into(), nf)?;
            read.insert(norm);
        } else {
            let norm = format!("{p}norm.weight");
            let nf = self.norm_form(f, &norm)?;
            self.present("output_norm.weight".into(), nf)?;
            read.insert(norm);
        }
        if !self.skipped.iter().any(|s| s == head) {
            if let Some(entry) = self.entry(head).cloned() {
                let whole = Form::whole(head, &entry);
                let form = if f == Family::FalconH1 { scaled(whole, falcon_h1_multipliers(h)?.head, 1.0) } else { whole };
                self.present("output.weight".into(), form)?;
                read.insert(head.into());
            }
        }
        // PhiMoE's LayerNorms carry biases, its head (and a Phi-3's, where `lm_head_bias` says) one too.
        if matches!(f, Family::Phi3 | Family::PhiMoe) {
            let biases = [(format!("{p}norm.bias"), "output_norm.bias"), (head.replacen(".weight", ".bias", 1), "output.bias")];
            for (stored, gguf) in biases {
                if let Some(entry) = self.entry(&stored).cloned() {
                    self.present(gguf.into(), Form::whole(&stored, &entry))?;
                    read.insert(stored);
                }
            }
        }

        // Blocks.
        let mut experts: BTreeMap<(usize, &'static str), BTreeMap<usize, String>> = BTreeMap::new();
        // Qwen3.8-Flash-Next's n-gram table, stored in shards by row: each shard's name by its index.
        let mut ngram_shards: BTreeMap<usize, String> = BTreeMap::new();
        for name in &names {
            if read.contains(name) {
                continue;
            }
            let (l, rest) = if let Some((l, rest)) = layer_of(name, &format!("{p}layers.")) {
                (l, rest.to_string())
            } else if f.hybrid() && name.starts_with("mtp.") {
                // The draft block sits after the stack: its layer is block `num_hidden_layers`, its projection and norms
                // that block's `nextn` tensors.
                let r = &name["mtp.".len()..];
                if let Some((0, rest)) = layer_of(name, "mtp.layers.") {
                    (n_layers, rest.to_string())
                } else if f == Family::Qwen4Exp && matches!(r, "fc_embedding.weight" | "fc_hidden.weight") {
                    // `eh_proj([e ; h]) = fc_embedding(e) + fc_hidden(h)`: the two projections side by side.
                    if r == "fc_embedding.weight" {
                        let (e, hd) = ("mtp.fc_embedding.weight".to_string(), "mtp.fc_hidden.weight".to_string());
                        let form = Form::Beside(vec![Form::whole(&e, self.need(&e)?), Form::whole(&hd, self.need(&hd)?)]);
                        self.present(format!("blk.{n_layers}.nextn.eh_proj.weight"), form)?;
                        read.insert(e);
                        read.insert(hd);
                    }
                    continue;
                } else {
                    let nextn = match r {
                        "hyper_connection_mixer.hc_norm.weight" if f == Family::Qwen4Exp => "nextn.hc_head_norm.weight",
                        "hyper_connection_mixer.input_mix_weight_down.weight" if f == Family::Qwen4Exp => "nextn.hc_head_down.weight",
                        "hyper_connection_mixer.input_mix_weight_up.weight" if f == Family::Qwen4Exp => "nextn.hc_head_up.weight",
                        "fc.weight" => "nextn.eh_proj.weight",
                        "norm.weight" => "nextn.shared_head_norm.weight",
                        "pre_fc_norm_embedding.weight" => "nextn.enorm.weight",
                        "pre_fc_norm_hidden.weight" => "nextn.hnorm.weight",
                        _ => continue,
                    };
                    let form = if nextn.ends_with("norm.weight") {
                        self.norm_form(f, name)?
                    } else {
                        Form::whole(name, &self.need(name)?.clone())
                    };
                    self.present(format!("blk.{n_layers}.{nextn}"), form)?;
                    read.insert(name.clone());
                    continue;
                }
            } else {
                continue;
            };
            let blk = |s: &str| format!("blk.{l}.{s}");
            // Mini-Jamba keeps its FFN under `moe.`; a Jamba block that routes to no experts may keep its FFN as
            // expert 0: the converter reads both as the block's FFN.
            let rest = match f {
                Family::Jamba => {
                    let routes = jamba_blocks(h)?.get(l).is_some_and(|&(_, routes)| routes);
                    let rest = rest.replacen("moe.", "feed_forward.", 1);
                    if routes {
                        rest
                    } else {
                        rest.replacen("feed_forward.experts.0.", "feed_forward.", 1)
                    }
                }
                _ => rest,
            };
            if let Some(proj) = expert_of(&rest) {
                // `mlp.experts.<x>.<proj>.weight`: gathered, stacked once the block is read.
                experts.entry((l, proj.1)).or_default().insert(proj.0, name.clone());
                read.insert(name.clone());
                continue;
            }
            let entry = self.need(name)?.clone();
            let whole = Form::whole(name, &entry);
            let gguf: Option<(String, Form)> = match (f, rest.as_str()) {
                (_, "input_layernorm.weight") => Some((blk("attn_norm.weight"), self.norm_form(f, name)?)),
                (
                    Family::Gemma2
                    | Family::Gemma3
                    | Family::Gemma3n
                    | Family::Gemma4
                    | Family::Qwen35
                    | Family::Qwen35Moe
                    | Family::Qwen3Next
                    | Family::GptOss
                    | Family::Glm4Moe
                    | Family::Olmo2,
                    "post_attention_layernorm.weight",
                ) => {
                    Some((blk("post_attention_norm.weight"), self.norm_form(f, name)?))
                }
                (_, "post_attention_layernorm.weight") => Some((blk("ffn_norm.weight"), self.norm_form(f, name)?)),
                (Family::Gemma2 | Family::Gemma3, "pre_feedforward_layernorm.weight") => Some((blk("ffn_norm.weight"), self.norm_form(f, name)?)),
                (Family::Gemma2 | Family::Gemma3 | Family::Olmo2, "post_feedforward_layernorm.weight") => Some((blk("post_ffw_norm.weight"), self.norm_form(f, name)?)),
                (_, "self_attn.q_norm.weight") => Some((blk("attn_q_norm.weight"), self.norm_form(f, name)?)),
                (_, "self_attn.k_norm.weight") => Some((blk("attn_k_norm.weight"), self.norm_form(f, name)?)),
                (Family::Llama | Family::Mistral3, "self_attn.q_proj.weight" | "self_attn.q_proj.bias") => {
                    let heads = h.need_u64("num_attention_heads")? as usize;
                    let suffix = if rest.ends_with("bias") { "attn_q.bias" } else { "attn_q.weight" };
                    Some((blk(suffix), llama_permute(name, &entry, heads)))
                }
                (Family::Llama | Family::Mistral3, "self_attn.k_proj.weight" | "self_attn.k_proj.bias") => {
                    let kv = h.u64(&["num_key_value_heads"]).unwrap_or(h.need_u64("num_attention_heads")?) as usize;
                    let suffix = if rest.ends_with("bias") { "attn_k.bias" } else { "attn_k.weight" };
                    Some((blk(suffix), llama_permute(name, &entry, kv)))
                }
                // Falcon-H1's MuP multipliers, folded into the weights as its converter folds them.
                (Family::FalconH1, "self_attn.q_proj.weight") => Some((blk("attn_q.weight"), scaled(whole, falcon_h1_multipliers(h)?.attention_in, 1.0))),
                (Family::FalconH1, "self_attn.k_proj.weight") => {
                    let m = falcon_h1_multipliers(h)?;
                    Some((blk("attn_k.weight"), scaled(whole, m.key, m.attention_in)))
                }
                (Family::FalconH1, "self_attn.v_proj.weight") => Some((blk("attn_v.weight"), scaled(whole, falcon_h1_multipliers(h)?.attention_in, 1.0))),
                (Family::FalconH1, "self_attn.o_proj.weight") => Some((blk("attn_output.weight"), scaled(whole, falcon_h1_multipliers(h)?.attention_out, 1.0))),
                (Family::FalconH1, "feed_forward.gate_proj.weight") => Some((blk("ffn_gate.weight"), scaled(whole, falcon_h1_multipliers(h)?.mlp[0], 1.0))),
                (Family::FalconH1, "feed_forward.up_proj.weight") => Some((blk("ffn_up.weight"), whole)),
                (Family::FalconH1, "feed_forward.down_proj.weight") => Some((blk("ffn_down.weight"), scaled(whole, falcon_h1_multipliers(h)?.mlp[1], 1.0))),
                // The converter maps `pre_ff_layernorm.weight` whole onto the FFN norm's name, which llama.cpp loads so.
                (Family::FalconH1, "pre_ff_layernorm.weight") => Some((format!("blk.{l}.ffn_norm"), self.norm_form(f, name)?)),
                (Family::FalconH1, r) if r.starts_with("mamba.") => Some(self.falcon_h1_mixer(h, name, &entry, &r["mamba.".len()..], l)?),
                // Jamba's Mamba-1 scan, its router and its dense FFN.
                // Kimi delta attention's tensors by the converter's names: each convolution `[taps, 1, channels, 1]`, the
                // decay `A = −exp(A_log)` a value a head.
                (Family::KimiLinear, r) if r.starts_with("self_attn.") && kimi_linear_layers(h)?.get(l) == Some(&false) => {
                    let named = |gguf: &str| Some((blk(gguf), Form::whole(name, &entry)));
                    match &r["self_attn.".len()..] {
                        "q_proj.weight" => named("attn_q.weight"),
                        "k_proj.weight" => named("attn_k.weight"),
                        "v_proj.weight" => named("attn_v.weight"),
                        "o_proj.weight" => named("attn_output.weight"),
                        "f_a_proj.weight" => named("ssm_f_a.weight"),
                        "f_b_proj.weight" => named("ssm_f_b.weight"),
                        "dt_bias" => named("ssm_dt.bias"),
                        "b_proj.weight" => named("ssm_beta.weight"),
                        "g_a_proj.weight" => named("ssm_g_a.weight"),
                        "g_b_proj.weight" => named("ssm_g_b.weight"),
                        "o_norm.weight" => named("ssm_norm.weight"),
                        conv @ ("q_conv1d.weight" | "k_conv1d.weight" | "v_conv1d.weight") => {
                            let (channels, taps) = (entry.shape[0], *entry.shape.last().unwrap_or(&1));
                            self.push(blk(&format!("ssm_conv1d_{}.weight", &conv[..1])), vec![taps, 1, channels, 1], whole)?;
                            Some((String::new(), Form::Values(Vec::new())))
                        }
                        "A_log" => {
                            self.push(blk("ssm_a"), vec![1, entry.elements(), 1, 1], Form::Map { inner: Box::new(whole), op: Op::NegExp })?;
                            Some((String::new(), Form::Values(Vec::new())))
                        }
                        other => return Err(invalid(format!("{name}: a Kimi delta attention tensor `{other}` this reader does not present"))),
                    }
                }
                (Family::KimiLinear, "self_attn.q_proj.weight") => Some((blk("attn_q.weight"), whole)),
                (Family::KimiLinear, "self_attn.kv_a_proj_with_mqa.weight") => Some((blk("attn_kv_a_mqa.weight"), whole)),
                (Family::KimiLinear, "self_attn.kv_a_layernorm.weight") => Some((blk("attn_kv_a_norm.weight"), whole)),
                (Family::KimiLinear, "block_sparse_moe.gate.weight") => Some((blk("ffn_gate_inp.weight"), whole)),
                (Family::KimiLinear, "block_sparse_moe.gate.e_score_correction_bias") => Some((blk("exp_probs_b.bias"), whole)),
                (Family::KimiLinear, "block_sparse_moe.shared_experts.gate_proj.weight") => Some((blk("ffn_gate_shexp.weight"), whole)),
                (Family::KimiLinear, "block_sparse_moe.shared_experts.up_proj.weight") => Some((blk("ffn_up_shexp.weight"), whole)),
                (Family::KimiLinear, "block_sparse_moe.shared_experts.down_proj.weight") => Some((blk("ffn_down_shexp.weight"), whole)),
                // Gemma 3n's norms, its AltUp, its LAuReL and its per-layer input, by the converter's names.
                (Family::Gemma3n | Family::Gemma4, "pre_feedforward_layernorm.weight") => Some((blk("ffn_norm.weight"), whole)),
                (Family::Gemma3n | Family::Gemma4, "post_feedforward_layernorm.weight") => Some((blk("post_ffw_norm.weight"), whole)),
                // Gemma 4's mixture block: its two branches' norms, its router (the weight, the input's scale and each
                // expert's), its experts' fused gate and up and their down, and the block's output scale.
                (Family::Gemma4, "post_feedforward_layernorm_1.weight") => Some((blk("post_ffw_norm_1.weight"), whole)),
                (Family::Gemma4, "pre_feedforward_layernorm_2.weight") => Some((blk("pre_ffw_norm_2.weight"), whole)),
                (Family::Gemma4, "post_feedforward_layernorm_2.weight") => Some((blk("post_ffw_norm_2.weight"), whole)),
                (Family::Gemma4, "router.proj.weight") => Some((blk("ffn_gate_inp.weight"), whole)),
                (Family::Gemma4, "router.scale") => Some((blk("ffn_gate_inp.scale"), whole)),
                (Family::Gemma4, "router.per_expert_scale") => Some((blk("ffn_down_exps.scale"), whole)),
                (Family::Gemma4, "experts.gate_up_proj") => Some((blk("ffn_gate_up_exps.weight"), whole)),
                (Family::Gemma4, "experts.down_proj") => Some((blk("ffn_down_exps.weight"), whole)),
                (Family::Gemma4, "layer_scalar") => Some((blk("layer_output_scale.weight"), whole)),
                (Family::Gemma3n, "altup.correct_output_scale") => Some((blk("altup_correct_scale.weight"), whole)),
                (Family::Gemma3n, "altup.correction_coefs.weight") => Some((blk("altup_correct_coef.weight"), whole)),
                (Family::Gemma3n, "altup.prediction_coefs.weight") => Some((blk("altup_predict_coef.weight"), whole)),
                (Family::Gemma3n, "altup.modality_router.weight") => Some((blk("altup_router.weight"), whole)),
                (Family::Gemma3n, "altup.router_norm.weight") => Some((blk("altup_router_norm.weight"), whole)),
                (Family::Gemma3n, "laurel.linear_left.weight") => Some((blk("laurel_l.weight"), whole)),
                (Family::Gemma3n, "laurel.linear_right.weight") => Some((blk("laurel_r.weight"), whole)),
                (Family::Gemma3n, "laurel.post_laurel_norm.weight") => Some((blk("laurel_post_norm.weight"), whole)),
                (Family::Gemma3n | Family::Gemma4, "per_layer_input_gate.weight") => Some((blk("inp_gate.weight"), whole)),
                (Family::Gemma3n | Family::Gemma4, "per_layer_projection.weight") => Some((blk("proj.weight"), whole)),
                (Family::Gemma3n | Family::Gemma4, "post_per_layer_input_norm.weight") => Some((blk("post_norm.weight"), whole)),
                (Family::Jamba, "mamba.A_log") => Some((blk("ssm_a"), Form::Map { inner: Box::new(whole), op: Op::NegExp })),
                (Family::Jamba, r) if r.starts_with("mamba.") => {
                    let gguf = match &r["mamba.".len()..] {
                        "in_proj.weight" => "ssm_in.weight",
                        "conv1d.weight" => "ssm_conv1d.weight",
                        "conv1d.bias" => "ssm_conv1d.bias",
                        "x_proj.weight" => "ssm_x.weight",
                        "dt_proj.weight" => "ssm_dt.weight",
                        "dt_proj.bias" => "ssm_dt.bias",
                        "D" => "ssm_d",
                        "out_proj.weight" => "ssm_out.weight",
                        "dt_layernorm.weight" => "ssm_dt_norm.weight",
                        "b_layernorm.weight" | "B_layernorm.weight" => "ssm_b_norm.weight",
                        "c_layernorm.weight" | "C_layernorm.weight" => "ssm_c_norm.weight",
                        other => return Err(invalid(format!("{name}: a Jamba scan tensor `{other}` this reader does not present"))),
                    };
                    Some((blk(gguf), whole))
                }
                (Family::Jamba, "feed_forward.router.weight") => Some((blk("ffn_gate_inp.weight"), whole)),
                (Family::Jamba, "feed_forward.gate_proj.weight") => Some((blk("ffn_gate.weight"), whole)),
                (Family::Jamba, "feed_forward.up_proj.weight") => Some((blk("ffn_up.weight"), whole)),
                (Family::Jamba, "feed_forward.down_proj.weight") => Some((blk("ffn_down.weight"), whole)),
                (Family::Jamba, "pre_ff_layernorm.weight") => Some((blk("ffn_norm.weight"), self.norm_form(f, name)?)),
                (_, "self_attn.q_proj.weight") => Some((blk("attn_q.weight"), whole)),
                (_, "self_attn.k_proj.weight") => Some((blk("attn_k.weight"), whole)),
                (_, "self_attn.v_proj.weight") => Some((blk("attn_v.weight"), whole)),
                (_, "self_attn.o_proj.weight") => Some((blk("attn_output.weight"), whole)),
                (_, "self_attn.q_proj.bias") => Some((blk("attn_q.bias"), whole)),
                (_, "self_attn.k_proj.bias") => Some((blk("attn_k.bias"), whole)),
                (_, "self_attn.v_proj.bias") => Some((blk("attn_v.bias"), whole)),
                (_, "self_attn.o_proj.bias") => Some((blk("attn_output.bias"), whole)),
                (Family::GptOss, "self_attn.sinks") => Some((blk("attn_sinks.weight"), whole)),
                (_, "mlp.gate_proj.weight") => Some((blk("ffn_gate.weight"), whole)),
                (_, "mlp.up_proj.weight") => Some((blk("ffn_up.weight"), whole)),
                (_, "mlp.down_proj.weight") => Some((blk("ffn_down.weight"), whole)),
                (
                    Family::Qwen3Moe
                    | Family::Qwen2Moe
                    | Family::Qwen35Moe
                    | Family::Qwen3Next
                    | Family::Qwen4Exp
                    | Family::Deepseek2
                    | Family::Deepseek32
                    | Family::GlmDsa
                    | Family::Glm4Moe,
                    "mlp.gate.weight",
                ) => {
                    Some((blk("ffn_gate_inp.weight"), whole))
                }
                (f, "mlp.shared_expert.gate_proj.weight") if f.qwen_mixture() => Some((blk("ffn_gate_shexp.weight"), whole)),
                (f, "mlp.shared_expert.up_proj.weight") if f.qwen_mixture() => Some((blk("ffn_up_shexp.weight"), whole)),
                (f, "mlp.shared_expert.down_proj.weight") if f.qwen_mixture() => Some((blk("ffn_down_shexp.weight"), whole)),
                // The shared expert's gate, one output row over the input: `[1, d]`, ggml `[d, 1]`.
                (f, "mlp.shared_expert_gate.weight") if f.qwen_mixture() => Some((blk("ffn_gate_inp_shexp.weight"), whole)),
                // A Qwen mixture's experts stored as one gate-and-up tensor `[experts, 2·ff, d]`, the gate's rows first:
                // each half a stack the converter splits out.
                (Family::Qwen3Moe | Family::Qwen2Moe | Family::Qwen35Moe | Family::Qwen3Next | Family::Qwen4Exp, "mlp.experts.gate_up_proj") => {
                    let (e, two, d) = dims3(&entry, name)?;
                    let ff = two / 2;
                    let half = |from: usize| Form::Gather {
                        src: name.clone(),
                        outer: e,
                        m: two,
                        n: d,
                        rows: (from..from + ff).collect(),
                        cols: (0..d).collect(),
                        transpose: false,
                    };
                    self.present(blk("ffn_gate_exps.weight"), half(0))?;
                    Some((blk("ffn_up_exps.weight"), half(ff)))
                }
                (Family::Qwen3Moe | Family::Qwen2Moe | Family::Qwen35Moe | Family::Qwen3Next | Family::Qwen4Exp, "mlp.experts.down_proj") => {
                    Some((blk("ffn_down_exps.weight"), whole))
                }
                (Family::Llama | Family::MinimaxM2 | Family::PhiMoe | Family::Minimax01, "block_sparse_moe.gate.weight") => Some((blk("ffn_gate_inp.weight"), whole)),
                // MiniMax-01's lightning attention: its fused projection, its gate, its norm and its output.
                (Family::Minimax01, "self_attn.qkv_proj.weight") => Some((blk("attn_qkv.weight"), whole)),
                (Family::Minimax01, "self_attn.output_gate.weight") => Some((blk("attn_gate.weight"), whole)),
                (Family::Minimax01, "self_attn.norm.weight") => Some((blk("attn_norm_2.weight"), whole)),
                (Family::Minimax01, "self_attn.out_proj.weight") => Some((blk("attn_output.weight"), whole)),
                (Family::PhiMoe, "input_layernorm.bias") => Some((blk("attn_norm.bias"), whole)),
                (Family::Lfm2, "operator_norm.weight") => Some((blk("attn_norm.weight"), whole)),
                (Family::Mamba2 | Family::NemotronH, "norm.weight") => Some((blk("attn_norm.weight"), whole)),
                // Nemotron-H's blocks each keep their sublayer as `mixer`: the scan's tensors, attention's (its queries
                // and keys permuted as the converter's Llama class permutes them) or the FFN's.
                (Family::NemotronH, r) if r.starts_with("mixer.") => {
                    let block = nemotron_h_blocks(h)?.get(l).copied().ok_or_else(|| invalid(format!("{name}: a block past the pattern")))?;
                    let heads = h.need_u64("num_attention_heads")? as usize;
                    let kv = h.u64(&["num_key_value_heads"]).unwrap_or(heads as u64) as usize;
                    match (block, &r["mixer.".len()..]) {
                        (NemotronBlock::Scan, rest) => Some(self.mamba2_mixer(h, name, &entry, rest, l)?),
                        (NemotronBlock::Attention, "q_proj.weight") => Some((blk("attn_q.weight"), llama_permute(name, &entry, heads))),
                        (NemotronBlock::Attention, "k_proj.weight") => Some((blk("attn_k.weight"), llama_permute(name, &entry, kv))),
                        (NemotronBlock::Attention, "v_proj.weight") => Some((blk("attn_v.weight"), whole)),
                        (NemotronBlock::Attention, "o_proj.weight") => Some((blk("attn_output.weight"), whole)),
                        (NemotronBlock::Ffn, "up_proj.weight") => Some((blk("ffn_up.weight"), whole)),
                        (NemotronBlock::Ffn, "down_proj.weight") => Some((blk("ffn_down.weight"), whole)),
                        (block, rest) => return Err(invalid(format!("{name}: a {block:?} block's tensor `{rest}` this reader does not present"))),
                    }
                }
                (Family::Mamba2, r) if r.starts_with("mixer.") => Some(self.mamba2_mixer(h, name, &entry, &r["mixer.".len()..], l)?),
                (Family::Lfm2, "ffn_norm.weight") => Some((blk("ffn_norm.weight"), whole)),
                (Family::Lfm2, "feed_forward.w1.weight") => Some((blk("ffn_gate.weight"), whole)),
                (Family::Lfm2, "feed_forward.w3.weight") => Some((blk("ffn_up.weight"), whole)),
                (Family::Lfm2, "feed_forward.w2.weight") => Some((blk("ffn_down.weight"), whole)),
                (Family::Lfm2, "self_attn.q_layernorm.weight") => Some((blk("attn_q_norm.weight"), whole)),
                (Family::Lfm2, "self_attn.k_layernorm.weight") => Some((blk("attn_k_norm.weight"), whole)),
                (Family::Lfm2, "self_attn.out_proj.weight") => Some((blk("attn_output.weight"), whole)),
                (Family::Lfm2, "conv.in_proj.weight") => Some((blk("shortconv.in_proj.weight"), whole)),
                (Family::Lfm2, "conv.out_proj.weight") => Some((blk("shortconv.out_proj.weight"), whole)),
                (Family::Lfm2, "conv.conv.weight") => Some((blk("shortconv.conv.weight"), whole)),
                (Family::PhiMoe, "post_attention_layernorm.bias") => Some((blk("ffn_norm.bias"), whole)),
                (Family::MinimaxM2, "block_sparse_moe.e_score_correction_bias") => Some((blk("exp_probs_b.bias"), whole)),
                (Family::Phi3, "self_attn.qkv_proj.weight") => Some((blk("attn_qkv.weight"), whole)),
                (Family::Phi3 | Family::Glm4, "mlp.gate_up_proj.weight") => Some((blk("ffn_up.weight"), whole)),
                (Family::Glm4, "post_self_attn_layernorm.weight") => Some((blk("post_attention_norm.weight"), self.norm_form(f, name)?)),
                (Family::Glm4, "post_mlp_layernorm.weight") => Some((blk("post_ffw_norm.weight"), self.norm_form(f, name)?)),
                (Family::Deepseek2 | Family::Deepseek32 | Family::GlmDsa | Family::Glm4Moe, "mlp.gate.e_score_correction_bias") => Some((blk("exp_probs_b.bias"), whole)),
                (Family::Deepseek2 | Family::Deepseek32 | Family::GlmDsa | Family::Glm4Moe, "mlp.shared_experts.gate_proj.weight") => Some((blk("ffn_gate_shexp.weight"), whole)),
                (Family::Deepseek2 | Family::Deepseek32 | Family::GlmDsa | Family::Glm4Moe, "mlp.shared_experts.up_proj.weight") => Some((blk("ffn_up_shexp.weight"), whole)),
                (Family::Deepseek2 | Family::Deepseek32 | Family::GlmDsa | Family::Glm4Moe, "mlp.shared_experts.down_proj.weight") => Some((blk("ffn_down_shexp.weight"), whole)),
                // GLM-4.5's draft block after the stack: its projection of [embedding | hidden], norms, embedding and head.
                (Family::Glm4Moe | Family::Deepseek32 | Family::GlmDsa, "eh_proj.weight") => Some((blk("nextn.eh_proj.weight"), whole)),
                (Family::Glm4Moe | Family::Deepseek32 | Family::GlmDsa, "embed_tokens.weight") => Some((blk("nextn.embed_tokens.weight"), whole)),
                (Family::Glm4Moe | Family::Deepseek32 | Family::GlmDsa, "shared_head.head.weight") => Some((blk("nextn.shared_head_head.weight"), whole)),
                (Family::Glm4Moe | Family::Deepseek32 | Family::GlmDsa, "enorm.weight") => Some((blk("nextn.enorm.weight"), self.norm_form(f, name)?)),
                (Family::Glm4Moe | Family::Deepseek32 | Family::GlmDsa, "hnorm.weight") => Some((blk("nextn.hnorm.weight"), self.norm_form(f, name)?)),
                (Family::Glm4Moe | Family::Deepseek32 | Family::GlmDsa, "shared_head.norm.weight") => Some((blk("nextn.shared_head_norm.weight"), self.norm_form(f, name)?)),
                (Family::Deepseek2 | Family::Deepseek32 | Family::GlmDsa, "self_attn.q_a_proj.weight") => Some((blk("attn_q_a.weight"), whole)),
                (Family::Deepseek2 | Family::Deepseek32 | Family::GlmDsa, "self_attn.q_a_layernorm.weight") => Some((blk("attn_q_a_norm.weight"), whole)),
                (Family::Deepseek2 | Family::Deepseek32 | Family::GlmDsa, "self_attn.q_b_proj.weight") => Some((blk("attn_q_b.weight"), whole)),
                (Family::Deepseek2 | Family::Deepseek32 | Family::GlmDsa, "self_attn.kv_a_proj_with_mqa.weight") => Some((blk("attn_kv_a_mqa.weight"), whole)),
                (Family::Deepseek2 | Family::Deepseek32 | Family::GlmDsa, "self_attn.kv_a_layernorm.weight") => Some((blk("attn_kv_a_norm.weight"), whole)),
                (Family::Deepseek2 | Family::Deepseek32 | Family::GlmDsa | Family::KimiLinear, "self_attn.kv_b_proj.weight") => {
                    let heads = h.u64(&["num_key_value_heads"]).unwrap_or(h.need_u64("num_attention_heads")?) as usize;
                    let nope = h.need_u64("qk_nope_head_dim")? as usize;
                    let v = h.need_u64("v_head_dim")? as usize;
                    let lora = *entry.shape.last().unwrap_or(&0) as usize;
                    if entry.elements() as usize != heads * (nope + v) * lora {
                        return Err(invalid(format!("{name}: {:?} is not {heads} heads of {nope}+{v} by {lora}", entry.shape)));
                    }
                    let k_b = Form::Gather {
                        src: name.clone(),
                        outer: heads,
                        m: nope + v,
                        n: lora,
                        rows: (0..nope).collect(),
                        cols: (0..lora).collect(),
                        transpose: true,
                    };
                    let v_b = Form::Gather {
                        src: name.clone(),
                        outer: heads,
                        m: nope + v,
                        n: lora,
                        rows: (nope..nope + v).collect(),
                        cols: (0..lora).collect(),
                        transpose: false,
                    };
                    self.present(blk("attn_k_b.weight"), k_b)?;
                    Some((blk("attn_v_b.weight"), v_b))
                }
                // DeepSeek's sparse attention's indexer (`gguf-py` `INDEXER_*`).
                (Family::Deepseek32 | Family::GlmDsa, "self_attn.indexer.wq_b.weight") => Some((blk("indexer.attn_q_b.weight"), whole)),
                (Family::Deepseek32 | Family::GlmDsa, "self_attn.indexer.wk.weight") => Some((blk("indexer.attn_k.weight"), whole)),
                (Family::Deepseek32 | Family::GlmDsa, "self_attn.indexer.k_norm.weight") => Some((blk("indexer.k_norm.weight"), whole)),
                (Family::Deepseek32 | Family::GlmDsa, "self_attn.indexer.k_norm.bias") => Some((blk("indexer.k_norm.bias"), whole)),
                (Family::Deepseek32 | Family::GlmDsa, "self_attn.indexer.weights_proj.weight") => Some((blk("indexer.proj.weight"), whole)),
                (Family::Llama4, "feed_forward.gate_proj.weight") => Some((blk("ffn_gate.weight"), whole)),
                (Family::Llama4, "feed_forward.up_proj.weight") => Some((blk("ffn_up.weight"), whole)),
                (Family::Llama4, "feed_forward.down_proj.weight") => Some((blk("ffn_down.weight"), whole)),
                (Family::Llama4, "feed_forward.router.weight") => Some((blk("ffn_gate_inp.weight"), whole)),
                (Family::Llama4, "feed_forward.shared_expert.gate_proj.weight") => Some((blk("ffn_gate_shexp.weight"), whole)),
                (Family::Llama4, "feed_forward.shared_expert.up_proj.weight") => Some((blk("ffn_up_shexp.weight"), whole)),
                (Family::Llama4, "feed_forward.shared_expert.down_proj.weight") => Some((blk("ffn_down_shexp.weight"), whole)),
                // The experts' gate and up projections in one `[experts, d, 2·ff]` tensor, the gate's columns first: each
                // half, transposed, is a stack the converter splits out.
                (Family::Llama4, "feed_forward.experts.gate_up_proj") => {
                    let (e, d, two) = dims3(&entry, name)?;
                    let ff = two / 2;
                    let half = |from: usize| Form::Gather {
                        src: name.clone(),
                        outer: e,
                        m: d,
                        n: two,
                        rows: (0..d).collect(),
                        cols: (from..from + ff).collect(),
                        transpose: true,
                    };
                    self.present(blk("ffn_gate_exps.weight"), half(0))?;
                    Some((blk("ffn_up_exps.weight"), half(ff)))
                }
                (Family::Llama4, "feed_forward.experts.down_proj") => {
                    let (e, ff, d) = dims3(&entry, name)?;
                    Some((
                        blk("ffn_down_exps.weight"),
                        Form::Gather {
                            src: name.clone(),
                            outer: e,
                            m: ff,
                            n: d,
                            rows: (0..ff).collect(),
                            cols: (0..d).collect(),
                            transpose: true,
                        },
                    ))
                }
                (Family::GptOss, "mlp.router.weight") => Some((blk("ffn_gate_inp.weight"), whole)),
                (Family::GptOss, "mlp.router.bias") => Some((blk("ffn_gate_inp.bias"), whole)),
                (Family::GptOss, "mlp.experts.gate_up_proj") => {
                    let (e, d, two) = dims3(&entry, name)?;
                    let ff = two / 2;
                    let pick = |o: usize| Form::Gather {
                        src: name.clone(),
                        outer: e,
                        m: d,
                        n: two,
                        rows: (0..d).collect(),
                        cols: (0..ff).map(|j| 2 * j + o).collect(),
                        transpose: true,
                    };
                    self.present(blk("ffn_gate_exps.weight"), pick(0))?;
                    Some((blk("ffn_up_exps.weight"), pick(1)))
                }
                (Family::GptOss, "mlp.experts.gate_up_proj_bias") => {
                    let (e, two) = (entry.shape[0] as usize, entry.shape[1] as usize);
                    let pick = |o: usize| Form::Gather {
                        src: name.clone(),
                        outer: e,
                        m: 1,
                        n: two,
                        rows: vec![0],
                        cols: (0..two / 2).map(|j| 2 * j + o).collect(),
                        transpose: false,
                    };
                    self.present(blk("ffn_gate_exps.bias"), pick(0))?;
                    Some((blk("ffn_up_exps.bias"), pick(1)))
                }
                (Family::GptOss, "mlp.experts.down_proj") => {
                    let (e, ff, d) = dims3(&entry, name)?;
                    Some((
                        blk("ffn_down_exps.weight"),
                        Form::Gather {
                            src: name.clone(),
                            outer: e,
                            m: ff,
                            n: d,
                            rows: (0..ff).collect(),
                            cols: (0..d).collect(),
                            transpose: true,
                        },
                    ))
                }
                (Family::GptOss, "mlp.experts.down_proj_bias") => Some((blk("ffn_down_exps.bias"), whole)),
                (Family::Qwen4Exp, r) if r.starts_with("attn_hyper_connection.") || r.starts_with("mlp_hyper_connection.") => {
                    let (sub, part) = r.split_once('.').unwrap();
                    let which = if sub == "attn_hyper_connection" { "hc_attn" } else { "hc_ffn" };
                    let (suffix, form) = match part {
                        "hc_norm.weight" => ("norm", self.norm_form(f, name)?),
                        "input_mix_weight_down.weight" => ("down", whole),
                        "input_mix_weight_up.weight" => ("up", whole),
                        "block_inject_weight.weight" => ("inject", whole),
                        other => return Err(invalid(format!("{name}: a hyper-connection tensor `{other}` this reader does not present"))),
                    };
                    Some((blk(&format!("{which}_{suffix}.weight")), form))
                }
                (Family::Qwen4Exp, "self_attn.indexer.index_qk_proj.weight") => {
                    // One projection feeds the index queries and the index key: split it as the converter does.
                    let heads = h.need_u64("indexer_n_heads")? as usize;
                    let hd = h.need_u64("indexer_head_dim")? as usize;
                    let n = *entry.shape.last().unwrap() as usize;
                    let rows = |rows: std::ops::Range<usize>| Form::Gather {
                        src: name.clone(),
                        outer: 1,
                        m: (heads + 1) * hd,
                        n,
                        rows: rows.collect(),
                        cols: (0..n).collect(),
                        transpose: false,
                    };
                    if entry.elements() as usize != (heads + 1) * hd * n {
                        return Err(invalid(format!("{name}: {:?} is not {heads} index queries and a key of {hd}", entry.shape)));
                    }
                    self.present(blk("indexer.q_proj.weight"), rows(0..heads * hd))?;
                    Some((blk("indexer.k_proj.weight"), rows(heads * hd..(heads + 1) * hd)))
                }
                (Family::Qwen4Exp, "self_attn.indexer.q_layernorm.weight") => Some((blk("indexer.q_norm.weight"), self.norm_form(f, name)?)),
                (Family::Qwen4Exp, "self_attn.indexer.k_layernorm.weight") => Some((blk("indexer.k_norm.weight"), self.norm_form(f, name)?)),
                (Family::Qwen4Exp, "ple.key_proj.weight") => Some((blk("ple_key.weight"), whole)),
                (Family::Qwen4Exp, "ple.value_proj.weight") => Some((blk("ple_value.weight"), whole)),
                (Family::Qwen4Exp, "ple.norm_key.weight") => Some((blk("ple_norm_key.weight"), self.norm_form(f, name)?)),
                (Family::Qwen4Exp, "ple.norm_query.weight") => Some((blk("ple_norm_query.weight"), self.norm_form(f, name)?)),
                (Family::Qwen4Exp, "ple.norm_conv.weight") => Some((blk("ple_norm_conv.weight"), self.norm_form(f, name)?)),
                (Family::Qwen4Exp, "ple.conv1d.weight") => Some((blk("ple_conv1d.weight"), whole)),
                (Family::Qwen4Exp, r) if r.starts_with("ple.ple_embedding.ngram_embedding.shard_") => {
                    let index = r["ple.ple_embedding.ngram_embedding.shard_".len()..]
                        .strip_suffix(".weight")
                        .and_then(|i| i.parse::<usize>().ok())
                        .ok_or_else(|| invalid(format!("{name}: an n-gram shard without its index")))?;
                    ngram_shards.insert(index, name.clone());
                    read.insert(name.clone());
                    None
                }
                // The hash constants: metadata ([`Self::ple_parameters`]).
                (
                    Family::Qwen4Exp,
                    "ple.ple_embedding.layer_multipliers" | "ple.ple_embedding.ngram_heads_offsets" | "ple.ple_embedding.ngram_heads_vocab_sizes",
                ) => {
                    read.insert(name.clone());
                    None
                }
                (Family::Qwen3Next, r) if r.starts_with("linear_attn.") => {
                    Some(self.grouped_gated_delta(h, name, &entry, &r["linear_attn.".len()..], l)?)
                }
                (f, r) if f.hybrid() && r.starts_with("linear_attn.") => {
                    Some(self.gated_delta(h, name, &entry, &r["linear_attn.".len()..], l)?)
                }
                _ => None,
            };
            if let Some((g, form)) = gguf {
                // A form the presenting call already pushed under its own dims.
                if g.is_empty() {
                    read.insert(name.clone());
                    continue;
                }
                if g.ends_with("ssm_conv1d.weight") || g.ends_with("ple_conv1d.weight") || g.ends_with("shortconv.conv.weight") {
                    // `[channels, 1, taps]` squeezed: ggml `[taps, channels]`.
                    let taps = *entry.shape.last().unwrap();
                    let channels = entry.elements() / taps.max(1);
                    self.push(g, vec![taps, channels], form)?;
                } else {
                    self.present(g, form)?;
                }
                read.insert(name.clone());
            }
        }

        // Stacks: every expert of a block, one tensor per projection, in expert order.
        let expert_count = h.u64(&["num_local_experts", "num_experts", "n_routed_experts"]).unwrap_or(0) as usize;
        for ((l, proj), by_expert) in experts {
            if by_expert.len() != expert_count || by_expert.keys().copied().ne(0..expert_count) {
                return Err(invalid(format!(
                    "block {l}: {} experts' {proj} of {expert_count}",
                    by_expert.len()
                )));
            }
            let parts: Vec<Form> = by_expert
                .values()
                .map(|n| Ok(Form::whole(n, self.need(n)?)))
                .collect::<io::Result<_>>()?;
            let g = match proj {
                "gate_proj" => "ffn_gate_exps.weight",
                "up_proj" => "ffn_up_exps.weight",
                _ => "ffn_down_exps.weight",
            };
            self.present(format!("blk.{l}.{g}"), Form::Concat(parts))?;
        }

        // The n-gram table: its shards' rows one after another, read a row at a time (it is the size of the model).
        if !ngram_shards.is_empty() {
            if ngram_shards.keys().copied().ne(0..ngram_shards.len()) {
                return Err(invalid(format!("the n-gram table's shards are not 0 to {}", ngram_shards.len() - 1)));
            }
            let mut rows = 0u64;
            let mut width = None;
            let mut parts = Vec::with_capacity(ngram_shards.len());
            for shard in ngram_shards.values() {
                let e = self.need(shard)?.clone();
                let (r, w) = (e.shape[0], *e.shape.last().unwrap());
                if *width.get_or_insert(w) != w {
                    return Err(invalid(format!("{shard}: rows of {w}, where the table's are {}", width.unwrap())));
                }
                rows += r;
                parts.push(Form::whole(shard, &e));
            }
            self.push("per_layer_token_embd.weight".into(), vec![width.unwrap(), rows], Form::Concat(parts))?;
        }

        self.unread = names.into_iter().filter(|n| !read.contains(n)).collect();
        Ok(())
    }

    /// Qwen3.8-Flash-Next's per-layer embedding keys the checkpoint's tensors hold: its hash constants, stored as int64
    /// buffers (exact: 45-bit multipliers), and the table's row width.
    fn ple_parameters(&self, w: &mut Writer) -> io::Result<()> {
        let find = |suffix: &str| self.located.keys().find(|n| n.ends_with(suffix)).cloned();
        let Some(multipliers) = find("ple.ple_embedding.layer_multipliers") else { return Ok(()) };
        let ints = |name: &str| -> io::Result<Vec<MetaValue>> {
            let e = self.need(name)?;
            if e.dtype != Dtype::I64 {
                return Err(invalid(format!("{name}: stored {:?}, where the hash constants are int64", e.dtype)));
            }
            let raw = self.shards[self.located[name]].read(name, 0, e.elements() as usize * 8)?;
            Ok(raw.chunks(8).map(|c| MetaValue::I64(i64::from_le_bytes(c.try_into().unwrap()))).collect())
        };
        let array = |items: Vec<MetaValue>| MetaValue::Array { ty: 11, items };
        w.put("ple.layer_multipliers", array(ints(&multipliers)?));
        let offsets = find("ple.ple_embedding.ngram_heads_offsets").ok_or_else(|| invalid("the n-gram heads' offsets are missing"))?;
        w.put("ple.head_offsets", array(ints(&offsets)?));
        let sizes = find("ple.ple_embedding.ngram_heads_vocab_sizes").ok_or_else(|| invalid("the n-gram heads' moduli are missing"))?;
        w.put("ple.head_vocab_sizes", array(ints(&sizes)?));
        let shard = find("ple.ple_embedding.ngram_embedding.shard_0.weight").ok_or_else(|| invalid("the n-gram table's first shard is missing"))?;
        w.u32("embedding_length_per_layer_input", *self.need(&shard)?.shape.last().unwrap());
        Ok(())
    }

    /// RWKV-7's tensors as the converter writes them (`Rwkv7Model`), from fla's names (`model.layers.<l>.attn.*`, each
    /// low-rank pair `<x>_lora.lora.{0,2}` stored output-major, the second's bias the pair's offset) or the Hugging Face
    /// port's (`model.blocks.<l>.attention.*`, each pair `<x>1`, `<x>2` stored input-major, which the converter
    /// transposes, and its offset `<x>0`): every tensor squeezed, `r_k` flattened, each block's six lerps stacked in the
    /// order r, w, k, v, a, g as `[d, 1, 1, 6]`, and block 0's value-mix pair, which no block reads, dropped and written as
    /// copies of its learning rate's.
    fn rwkv7_views(&mut self, h: &Hparams, names: &[String], read: &mut BTreeSet<String>) -> io::Result<()> {
        let d = h.need_u64("hidden_size")?;
        let transposed = !names.iter().any(|n| n.contains("_lora.lora"));
        let unify = |n: &str| -> String {
            let n = n.replace("blocks", "layers").replace("ffn", "feed_forward").replace("self_attn", "attention").replace("attn", "attention");
            let n = n.replace("time_mixer.", "").replace("feed_forward_norm", "ln2").replace("g_norm", "ln_x");
            n.replace("_lora.lora.0.weight", "1.weight").replace("_lora.lora.2.weight", "2.weight").replace("_lora.lora.2.bias", "0.weight")
        };
        let mut lerps: BTreeMap<usize, BTreeMap<char, String>> = BTreeMap::new();
        for name in names {
            if read.contains(name) {
                continue;
            }
            let u = unify(name);
            let entry = self.need(name)?.clone();
            let squeezed: Vec<u64> = entry.shape.iter().rev().copied().filter(|&x| x != 1).collect();
            let whole = Form::whole(name, &entry);
            let global = match u.as_str() {
                "model.embeddings.weight" => Some("token_embd.weight"),
                "lm_head.weight" | "head.weight" => Some("output.weight"),
                "model.norm.weight" | "model.ln_out.weight" => Some("output_norm.weight"),
                "model.norm.bias" | "model.ln_out.bias" => Some("output_norm.bias"),
                "model.layers.0.pre_norm.weight" | "model.pre_ln.weight" => Some("token_embd_norm.weight"),
                "model.layers.0.pre_norm.bias" | "model.pre_ln.bias" => Some("token_embd_norm.bias"),
                _ => None,
            };
            if let Some(g) = global {
                self.push(g.into(), squeezed, whole)?;
                read.insert(name.clone());
                continue;
            }
            let Some((l, rest)) = layer_of(&u, "model.layers.") else {
                return Err(invalid(format!("{name}: an RWKV-7 tensor this reader does not present")));
            };
            let rest = rest.strip_suffix(".weight").unwrap_or(rest);
            let blk = |g: &str| format!("blk.{l}.{g}");
            if let Some(x) = rest.strip_prefix("attention.x_").filter(|x| x.len() == 1).and_then(|x| x.chars().next()) {
                if x == 'x' {
                    self.push(blk("time_mix_lerp_fused.weight"), vec![d, 1, 1, entry.elements() / d], whole)?;
                } else {
                    lerps.entry(l).or_default().insert(x, name.clone());
                }
                read.insert(name.clone());
                continue;
            }
            let pair = rest.strip_prefix("attention.").and_then(|r| {
                let mut c = r.chars();
                match (c.next(), c.next(), c.next()) {
                    (Some(p @ ('w' | 'a' | 'v' | 'g')), Some(i @ ('0' | '1' | '2')), None) => Some((p, i)),
                    _ => None,
                }
            });
            let gguf = match (rest, pair) {
                (_, Some(('v', _))) if l == 0 => {
                    read.insert(name.clone());
                    continue;
                }
                (_, Some((p, i))) => format!("time_mix_{p}{i}.weight"),
                ("attention_norm" | "ln1", _) => "attn_norm.weight".to_string(),
                ("attention_norm.bias" | "ln1.bias", _) => "attn_norm.bias".to_string(),
                ("ln2", _) => "attn_norm_2.weight".to_string(),
                ("ln2.bias", _) => "attn_norm_2.bias".to_string(),
                ("attention.ln_x", _) => "time_mix_ln.weight".to_string(),
                ("attention.ln_x.bias", _) => "time_mix_ln.bias".to_string(),
                ("attention.k_k", _) => "time_mix_k_k.weight".to_string(),
                ("attention.k_a", _) => "time_mix_k_a.weight".to_string(),
                ("attention.r_k", _) => "time_mix_r_k.weight".to_string(),
                ("attention.key" | "attention.k_proj", _) => "time_mix_key.weight".to_string(),
                ("attention.value" | "attention.v_proj", _) => "time_mix_value.weight".to_string(),
                ("attention.receptance" | "attention.r_proj", _) => "time_mix_receptance.weight".to_string(),
                ("attention.output" | "attention.o_proj", _) => "time_mix_output.weight".to_string(),
                ("feed_forward.x_k", _) => "channel_mix_lerp_k.weight".to_string(),
                ("feed_forward.key", _) => "channel_mix_key.weight".to_string(),
                ("feed_forward.value", _) => "channel_mix_value.weight".to_string(),
                (other, _) => return Err(invalid(format!("{name}: an RWKV-7 tensor `{other}` this reader does not present"))),
            };
            let (form, dims) = match (&entry.shape[..], pair.is_some_and(|(_, i)| i != '0') && transposed) {
                (&[a, b], true) => (
                    Form::Gather {
                        src: name.clone(),
                        outer: 1,
                        m: a as usize,
                        n: b as usize,
                        rows: (0..a as usize).collect(),
                        cols: (0..b as usize).collect(),
                        transpose: true,
                    },
                    vec![a, b],
                ),
                _ if gguf == "time_mix_r_k.weight" => (whole, vec![entry.elements()]),
                _ => (whole, squeezed),
            };
            // The converter's block 0 holds a dummy value-mix pair: its learning rate's, written twice.
            if l == 0 && gguf.starts_with("time_mix_a") {
                self.push(blk(&gguf.replacen("time_mix_a", "time_mix_v", 1)), dims.clone(), form.clone())?;
            }
            self.push(blk(&gguf), dims, form)?;
            read.insert(name.clone());
        }
        for (l, by) in lerps {
            let order = ['r', 'w', 'k', 'v', 'a', 'g'];
            if by.len() != order.len() || order.iter().any(|x| !by.contains_key(x)) {
                return Err(invalid(format!("block {l}: lerps {:?}, not the six llama.cpp's rwkv7 reads", by.keys().collect::<Vec<_>>())));
            }
            let parts: Vec<Form> = order.iter().map(|x| Ok(Form::whole(&by[x], self.need(&by[x])?))).collect::<io::Result<_>>()?;
            self.push(format!("blk.{l}.time_mix_lerp_fused.weight"), vec![d, 1, 1, 6], Form::Concat(parts))?;
        }
        Ok(())
    }

    /// A norm weight as the forward multiplies by it: every Gemma and Qwen3.5 store it as an offset from one (Qwen3.5's
    /// gated-delta norm excepted).
    fn norm_form(&self, f: Family, name: &str) -> io::Result<Form> {
        let whole = Form::whole(name, self.need(name)?);
        let offset = (matches!(f, Family::Gemma | Family::Gemma2 | Family::Gemma3) || f.hybrid()) && !name.ends_with("linear_attn.norm.weight");
        Ok(if offset { Form::Map { inner: Box::new(whole), op: Op::AddOne } } else { whole })
    }

    /// A Mamba-2 mixer tensor (`mixer.<rest>`) as the converter writes it: the convolution squeezed, the decay `A = −exp(A_log)`
    /// and the skip `D` a column each (ggml `[1, heads]`), the gated norm a row of each group's weights.
    fn mamba2_mixer(&mut self, h: &Hparams, name: &str, entry: &Entry, rest: &str, l: usize) -> io::Result<(String, Form)> {
        let blk = |s: &str| format!("blk.{l}.{s}");
        let whole = Form::whole(name, entry);
        let column = |me: &mut Self, gguf: String, form: Form| -> io::Result<(String, Form)> {
            let heads = entry.elements();
            me.push(gguf, vec![1, heads], form)?;
            Ok((String::new(), Form::Values(Vec::new())))
        };
        Ok(match rest {
            "in_proj.weight" => (blk("ssm_in.weight"), whole),
            "conv1d.weight" => (blk("ssm_conv1d.weight"), whole),
            "conv1d.bias" => (blk("ssm_conv1d.bias"), whole),
            "dt_bias" => (blk("ssm_dt.bias"), whole),
            "A_log" => return column(self, blk("ssm_a"), Form::Map { inner: Box::new(whole), op: Op::NegExp }),
            "D" => return column(self, blk("ssm_d"), whole),
            "norm.weight" => {
                let groups = h.u64(&["n_groups", "mamba_n_groups"]).filter(|&g| g > 0).unwrap_or(1);
                self.push(blk("ssm_norm.weight"), vec![entry.elements() / groups, groups], whole)?;
                (String::new(), Form::Values(Vec::new()))
            }
            "out_proj.weight" => (blk("ssm_out.weight"), whole),
            other => return Err(invalid(format!("{name}: a Mamba-2 tensor `{other}` this reader does not present"))),
        })
    }

    /// A Falcon-H1 scan tensor (`mamba.<rest>`) as its converter writes it: Mamba-2's, with the input projection times
    /// `ssm_in_multiplier` and then each of its five parts (`z`, `x`, `B`, `C`, the time steps) times its
    /// `ssm_multipliers` entry, and the output projection times `ssm_out_multiplier`.
    fn falcon_h1_mixer(&mut self, h: &Hparams, name: &str, entry: &Entry, rest: &str, l: usize) -> io::Result<(String, Form)> {
        let m = falcon_h1_multipliers(h)?;
        let blk = |s: &str| format!("blk.{l}.{s}");
        match rest {
            "in_proj.weight" => {
                let (_, inner, _, groups) = falcon_h1_widths(h)?;
                let (inner, gs) = (inner as usize, (groups * h.need_u64("mamba_d_state")?) as usize);
                let (rows, n) = (entry.shape[0] as usize, *entry.shape.last().unwrap_or(&0) as usize);
                if rows <= 2 * inner + 2 * gs {
                    return Err(invalid(format!("{name}: {rows} rows hold no time steps past z, x, B and C")));
                }
                let bounds = [0, inner, 2 * inner, 2 * inner + gs, 2 * inner + 2 * gs, rows];
                let parts = (0..5)
                    .map(|i| {
                        let block = Form::Gather {
                            src: name.to_string(),
                            outer: 1,
                            m: rows,
                            n,
                            rows: (bounds[i]..bounds[i + 1]).collect(),
                            cols: (0..n).collect(),
                            transpose: false,
                        };
                        scaled(block, m.ssm_in, m.ssm[i])
                    })
                    .collect();
                self.push(blk("ssm_in.weight"), vec![n as u64, rows as u64], Form::Concat(parts))?;
                Ok((String::new(), Form::Values(Vec::new())))
            }
            "out_proj.weight" => Ok((blk("ssm_out.weight"), scaled(Form::whole(name, entry), m.ssm_out, 1.0))),
            _ => self.mamba2_mixer(h, name, entry, rest, l),
        }
    }

    /// A Qwen3-Next Gated DeltaNet tensor (`linear_attn.<rest>`) in the layout its converter writes: value heads grouped
    /// by key head, each key head's betas and alphas one `ssm_ba`. `model::layout` presents it in Qwen3.5's.
    fn grouped_gated_delta(&mut self, h: &Hparams, name: &str, entry: &Entry, rest: &str, l: usize) -> io::Result<(String, Form)> {
        let kh = h.need_u64("linear_num_key_heads")? as usize;
        let vh = h.need_u64("linear_num_value_heads")? as usize;
        let kd = h.need_u64("linear_key_head_dim")? as usize;
        let vd = h.need_u64("linear_value_head_dim")? as usize;
        let blk = |s: &str| format!("blk.{l}.{s}");
        let whole = Form::whole(name, entry);
        // `in_proj_qkvz` holds, for each key head, its query, its key, its value heads and their gates.
        let group = 2 * kd + 2 * (vh / kh.max(1)) * vd;
        let vz = (vh / kh.max(1)) * vd;
        let rows_with = |rows: Vec<usize>, n: usize| Form::Gather {
            src: name.to_string(),
            outer: 1,
            m: rows.len(),
            n,
            rows,
            cols: (0..n).collect(),
            transpose: false,
        };
        let part = |off: usize, len: usize| -> Vec<usize> { (0..kh).flat_map(|k| (0..len).map(move |e| k * group + off + e)).collect() };
        Ok(match rest {
            "in_proj_qkvz.weight" => {
                let n = *entry.shape.last().unwrap() as usize;
                let qkv: Vec<usize> = part(0, kd).into_iter().chain(part(kd, kd)).chain(part(2 * kd, vz)).collect();
                self.present(blk("attn_gate.weight"), rows_with(part(2 * kd + vz, vz), n))?;
                (blk("attn_qkv.weight"), rows_with(qkv, n))
            }
            "in_proj_ba.weight" => (blk("ssm_ba.weight"), whole),
            "A_log" => (blk("ssm_a"), Form::Map { inner: Box::new(whole), op: Op::NegExp }),
            "dt_bias" => (blk("ssm_dt.bias"), whole),
            "conv1d.weight" => (blk("ssm_conv1d.weight"), whole),
            "norm.weight" => (blk("ssm_norm.weight"), whole),
            "out_proj.weight" => (blk("ssm_out.weight"), whole),
            other => return Err(invalid(format!("{name}: a Gated DeltaNet tensor `{other}` this reader does not present"))),
        })
    }

    fn gated_delta(&self, h: &Hparams, name: &str, entry: &Entry, rest: &str, l: usize) -> io::Result<(String, Form)> {
        let kh = h.need_u64("linear_num_key_heads")? as usize;
        let vh = h.need_u64("linear_num_value_heads")? as usize;
        let kd = h.need_u64("linear_key_head_dim")? as usize;
        let vd = h.need_u64("linear_value_head_dim")? as usize;
        let reorder = kh != vh;
        let per_k = vh / kh.max(1);
        // Index `o` of the tiled order reads index `tiled(o)` of the grouped one: `[k, r, d] → [r, k, d]`.
        let tiled = |o: usize, d: usize| -> usize {
            let r = o / (kh * d);
            let k = (o / d) % kh;
            k * per_k * d + r * d + o % d
        };
        let perm = |len: usize, d: usize| -> Vec<usize> {
            (0..len).map(|o| if reorder { tiled(o, d) } else { o }).collect()
        };
        let blk = |s: &str| format!("blk.{l}.{s}");
        let whole = Form::whole(name, entry);
        let rows_with = |rows: Vec<usize>, n: usize| Form::Gather {
            src: name.to_string(),
            outer: 1,
            m: rows.len(),
            n,
            rows,
            cols: (0..n).collect(),
            transpose: false,
        };
        Ok(match rest {
            "in_proj_qkv.weight" => {
                let n = *entry.shape.last().unwrap() as usize;
                let qk = 2 * kh * kd;
                let rows: Vec<usize> = (0..qk).chain(perm(vh * vd, vd).into_iter().map(|o| qk + o)).collect();
                (blk("attn_qkv.weight"), rows_with(rows, n))
            }
            "in_proj_z.weight" => {
                let n = *entry.shape.last().unwrap() as usize;
                (blk("attn_gate.weight"), rows_with(perm(vh * vd, vd), n))
            }
            "in_proj_a.weight" | "in_proj_b.weight" => {
                let n = *entry.shape.last().unwrap() as usize;
                let g = if rest == "in_proj_a.weight" { "ssm_alpha.weight" } else { "ssm_beta.weight" };
                (blk(g), rows_with(perm(vh, 1), n))
            }
            "A_log" => (
                blk("ssm_a"),
                Form::Map { inner: Box::new(rows_with(perm(vh, 1), 1)), op: Op::NegExp },
            ),
            "dt_bias" => (blk("ssm_dt.bias"), rows_with(perm(vh, 1), 1)),
            "conv1d.weight" => {
                // `[channels, 1, taps]`, squeezed to `[channels, taps]`; only the value channels move.
                let taps = *entry.shape.last().unwrap() as usize;
                let qk = 2 * kh * kd;
                let rows: Vec<usize> = (0..qk).chain(perm(vh * vd, vd).into_iter().map(|o| qk + o)).collect();
                (blk("ssm_conv1d.weight"), rows_with(rows, taps))
            }
            "norm.weight" => (blk("ssm_norm.weight"), whole),
            "out_proj.weight" => {
                let m = entry.shape[0] as usize;
                (
                    blk("ssm_out.weight"),
                    Form::Gather {
                        src: name.to_string(),
                        outer: 1,
                        m,
                        n: vh * vd,
                        rows: (0..m).collect(),
                        cols: perm(vh * vd, vd),
                        transpose: false,
                    },
                )
            }
            other => return Err(invalid(format!("{name}: a Gated DeltaNet tensor `{other}` this reader does not present"))),
        })
    }
}

/// Where a checkpoint of the class `architecture`, holding the tensors `names`, keeps its decoder's tensors: a multimodal
/// checkpoint nests its text model under `model.language_model.` (as transformers writes every wrapper, and Qwen3.5's
/// releases are published) or under `language_model.model.` (as Gemma 3's and Llama 4's were), the converter reading
/// either with `language_model.` removed; a text model alone (Qwen3.8-2.4T's `Qwen3_5MoeForCausalLM`) keeps it under
/// `model.`.
fn decoder_prefix(architecture: &str, names: &[String]) -> &'static str {
    if !architecture.ends_with("ForConditionalGeneration") {
        "model."
    } else if names.iter().any(|n| n.starts_with("language_model.model.")) {
        "language_model.model."
    } else {
        "model.language_model."
    }
}

/// The text of a configuration as Python's `json` reads it: its bare `NaN`, `Infinity` and `-Infinity`, which Python
/// writes for those floats and JSON has no number for, made the strings `json_f64` reads back.
fn python_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut in_string, mut escaped) = (false, false);
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if in_string {
            (in_string, escaped) = (escaped || c != '"', !escaped && c == '\\');
        } else if c == '"' {
            in_string = true;
        } else if let Some(word) = ["-Infinity", "Infinity", "NaN"].into_iter().find(|w| rest.starts_with(w)) {
            out.push('"');
            out.push_str(word);
            out.push('"');
            rest = &rest[word.len()..];
            continue;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// `v` with every `{"__float__": "Infinity"}`, the object transformers 5 writes for a non-finite float, made the string
/// `python_json` spells for the bare float older releases write, so either spelling reads back through `json_f64`.
fn tagged_floats(v: Value) -> Value {
    match v {
        Value::Object(map) => match map.get("__float__").and_then(Value::as_str) {
            Some(tag @ ("Infinity" | "-Infinity" | "NaN")) if map.len() == 1 => Value::String(tag.to_string()),
            _ => Value::Object(map.into_iter().map(|(k, v)| (k, tagged_floats(v))).collect()),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(tagged_floats).collect()),
        v => v,
    }
}

/// A configuration's float as Python reads it: a number, or one of the non-finite floats `python_json` and
/// `tagged_floats` spell.
fn json_f64(v: &Value) -> Option<f64> {
    match v.as_str() {
        Some("Infinity") => Some(f64::INFINITY),
        Some("-Infinity") => Some(f64::NEG_INFINITY),
        Some("NaN") => Some(f64::NAN),
        _ => v.as_f64(),
    }
}

/// `dir`'s configuration, its text configuration merged in, its family and its keys refused as the reader refuses them.
fn configuration(dir: &Path) -> io::Result<(Family, Hparams)> {
    let config_path = dir.join("config.json");
    let text = std::fs::read_to_string(&config_path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", config_path.display())))?;
    let root: Value =
        tagged_floats(serde_json::from_str(&python_json(&text)).map_err(|e| invalid(format!("{}: {e}", config_path.display())))?);
    let mut map = root.as_object().cloned().ok_or_else(|| invalid("config.json is not an object"))?;
    let architecture =
        map.get("architectures").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str).unwrap_or_default().to_string();
    if let Some(Value::Object(text)) = map.get("text_config").cloned() {
        for (k, v) in text {
            map.insert(k, v);
        }
    }
    let family =
        Family::of(&architecture).ok_or_else(|| invalid(format!("config.json: architecture `{architecture}` is not one this reader presents")))?;
    let h = Hparams { map, architecture };
    validate(family, &h)?;
    Ok((family, h))
}

/// The draft blocks the converter counts in the stack, as the configuration states them: Qwen3.5's (`mtp.*`) and
/// GLM-4.5's (`model.layers.<n>.*`); `None` where only the tensors count them (a hybrid with no
/// `mtp_num_hidden_layers`).
fn stated_draft_blocks(family: Family, h: &Hparams) -> Option<u64> {
    match family {
        f if f.hybrid() => h.u64(&["mtp_num_hidden_layers"]),
        Family::Glm4Moe | Family::Deepseek32 | Family::GlmDsa => Some(h.u64(&["num_nextn_predict_layers"]).unwrap_or(0)),
        _ => Some(0),
    }
}

/// The draft blocks a checkpoint stores after its stack (`mtp.layers.<n>.*`) when its configuration does not count them,
/// as Qwen3-Next's does not: the converter counts them from the tensors.
fn draft_layers(c: &HfCheckpoint) -> u64 {
    let layers: BTreeSet<usize> = c.located.keys().filter_map(|n| layer_of(n, "mtp.layers.").map(|(l, _)| l)).collect();
    layers.len() as u64
}

/// The ggml type a stored dtype is presented as: its own for f32, f16 and bf16; every other dtype widened to f32, as
/// the converter widens it before it writes anything.
fn ggml_type(d: Dtype) -> GgmlType {
    match d {
        Dtype::F16 => GgmlType::F16,
        Dtype::Bf16 => GgmlType::Bf16,
        _ => GgmlType::F32,
    }
}

/// Elements stored as `d`, as the type [`ggml_type`] presents them: f32, f16 and bf16 as they are, every other dtype
/// converted to f32 as `torch`'s `.to(float32)` converts it.
fn widen(d: Dtype, raw: Vec<u8>) -> Vec<u8> {
    let f32s = |it: &mut dyn Iterator<Item = f32>| it.flat_map(f32::to_le_bytes).collect::<Vec<u8>>();
    match d {
        Dtype::F32 | Dtype::F16 | Dtype::Bf16 => raw,
        Dtype::F64 => f32s(&mut raw.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap()) as f32)),
        Dtype::F8E4m3 => f32s(&mut raw.iter().map(|&b| e4m3fn_to_f32(b))),
        Dtype::F8E5m2 => f32s(&mut raw.iter().map(|&b| e5m2_to_f32(b))),
        Dtype::I64 => f32s(&mut raw.chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().unwrap()) as f32)),
        Dtype::I32 => f32s(&mut raw.chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap()) as f32)),
        Dtype::I16 => f32s(&mut raw.chunks_exact(2).map(|c| i16::from_le_bytes(c.try_into().unwrap()) as f32)),
        Dtype::I8 => f32s(&mut raw.iter().map(|&b| b as i8 as f32)),
        Dtype::U8 => f32s(&mut raw.iter().map(|&b| b as f32)),
        Dtype::Bool => f32s(&mut raw.iter().map(|&b| f32::from(b != 0))),
    }
}

/// `name`'s layer and the rest of its name after `prefix<layer>.`.
fn layer_of<'a>(name: &'a str, prefix: &str) -> Option<(usize, &'a str)> {
    let rest = name.strip_prefix(prefix)?;
    let (l, tail) = rest.split_once('.')?;
    Some((l.parse().ok()?, tail))
}

/// `mlp.experts.<x>.<proj>.weight` → `(x, proj)`.
fn expert_of(rest: &str) -> Option<(usize, &'static str)> {
    let r = rest
        .strip_prefix("mlp.experts.")
        .or_else(|| rest.strip_prefix("block_sparse_moe.experts."))
        .or_else(|| rest.strip_prefix("feed_forward.experts."))?;
    let (x, tail) = r.split_once('.')?;
    let x: usize = x.parse().ok()?;
    // Mixtral names an expert's gate `w1`, its down `w2` and its up `w3`.
    let proj = match tail {
        "gate_proj.weight" | "w1.weight" => "gate_proj",
        "up_proj.weight" | "w3.weight" => "up_proj",
        "down_proj.weight" | "w2.weight" => "down_proj",
        _ => return None,
    };
    Some((x, proj))
}

/// A three-dimensional tensor's shape.
fn dims3(e: &Entry, name: &str) -> io::Result<(usize, usize, usize)> {
    match e.shape[..] {
        [a, b, c] => Ok((a as usize, b as usize, c as usize)),
        _ => Err(invalid(format!("{name}: {:?} is not three-dimensional", e.shape))),
    }
}

/// Llama's query or key rows (or bias) in the converter's order: each head's `[2, half]` halves interleaved into
/// `[half, 2]` pairs, the rotation pairing llama.cpp's interleaved rope turns.
fn llama_permute(name: &str, e: &Entry, heads: usize) -> Form {
    let rows = e.shape[0] as usize;
    let n = if e.shape.len() > 1 { (e.elements() as usize) / rows } else { 1 };
    let per = rows / heads.max(1);
    let half = per / 2;
    let src_row = |o: usize| (o / per) * per + (o % 2) * half + (o % per) / 2;
    Form::Gather {
        src: name.to_string(),
        outer: 1,
        m: rows,
        n,
        rows: (0..rows).map(src_row).collect(),
        cols: (0..n).collect(),
        transpose: false,
    }
}

/// The rotary parameters as the converter assembles them: `rope_parameters` (or `rope_scaling`), with the global
/// theta, the sliding-window theta, the type, the partial factor and the original context mirrored in when the
/// checkpoint states them beside it.
fn rope_parameters(h: &Hparams) -> Map<String, Value> {
    let mut rope = match h.map.get("rope_parameters").or_else(|| h.map.get("rope_scaling")) {
        Some(Value::Object(o)) => o.clone(),
        _ => Map::new(),
    };
    if !rope.contains_key("full_attention") && !rope.contains_key("sliding_attention") {
        let local = h.find(&["local_rope_theta", "rope_local_theta", "rope_theta_local", "swa_rope_theta", "rope_local_base_freq"]);
        if let Some(local) = local {
            let mut sliding = Map::new();
            sliding.insert("rope_theta".into(), local.clone());
            rope.insert("sliding_attention".into(), Value::Object(sliding));
        }
        let theta = h.find(&["global_rope_theta", "rope_global_theta", "rope_theta_global", "rope_theta", "rotary_emb_base"]);
        if !rope.contains_key("rope_theta") {
            if let Some(t) = theta {
                rope.insert("rope_theta".into(), t.clone());
            }
        }
        if !rope.contains_key("rope_type") {
            if let Some(t) = rope.get("type").cloned() {
                rope.insert("rope_type".into(), t);
            }
        }
        if !rope.contains_key("partial_rotary_factor") {
            if let Some(p) = h.find(&["partial_rotary_factor", "rope_pct", "rope_percent"]) {
                rope.insert("partial_rotary_factor".into(), p.clone());
            }
        }
        if !rope.contains_key("original_max_position_embeddings") {
            if let Some(o) = h.find(&["original_max_position_embeddings"]) {
                rope.insert("original_max_position_embeddings".into(), o.clone());
            }
        }
    }
    rope
}

/// The parameters the rotation of full-attention layers reads.
fn full_rope(rope: &Map<String, Value>) -> Map<String, Value> {
    match rope.get("full_attention") {
        Some(Value::Object(o)) => o.clone(),
        _ => rope.clone(),
    }
}

/// `TextModel.set_gguf_parameters`: the keys every decoder writes.
fn base_parameters(h: &Hparams, rope: &Map<String, Value>, w: &mut Writer, block_count: u64, f: Family) -> io::Result<()> {
    w.u32("block_count", block_count);
    if let Some(n) = h.u64(&["max_position_embeddings", "n_ctx", "n_positions", "max_length", "max_sequence_length", "model_max_length"]) {
        w.u32("context_length", n);
    }
    if let Some(n) = h.u64(&["hidden_size", "n_embd", "dim"]) {
        w.u32("embedding_length", n);
    }
    if let Some(n) = h.u64(&["prefix_dense_intermediate_size", "dense_intermediate_size", "intermediate_size", "n_inner", "hidden_dim"]) {
        w.u32("feed_forward_length", n);
    }
    if let Some(n) = h.u64(&["num_attention_heads", "n_head", "n_heads"]) {
        w.u32("attention.head_count", n);
    }
    // Latent attention is read as one key-value head that every query head expands.
    let kv = if f.deepseek() { Some(1) } else { h.u64(&["num_key_value_heads", "n_kv_heads"]) };
    if let Some(n) = kv {
        w.u32("attention.head_count_kv", n);
    }
    if h.map.get("is_causal").and_then(Value::as_bool) == Some(false) {
        w.bool("attention.causal", false);
    }
    let rp = full_rope(rope);
    if let Some(kind) = rp.get("rope_type").and_then(Value::as_str) {
        let factor = rp.get("factor").and_then(Value::as_f64);
        match (kind, factor) {
            ("linear", Some(factor)) => {
                w.str("rope.scaling.type", "linear");
                w.f32("rope.scaling.factor", factor);
            }
            ("yarn", Some(factor)) => {
                w.str("rope.scaling.type", "yarn");
                w.f32("rope.scaling.factor", factor);
                let orig = rp
                    .get("original_max_position_embeddings")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| invalid("config.json: a YaRN rotation without original_max_position_embeddings"))?;
                w.u32("rope.scaling.original_context_length", orig);
                if let Some(t) = rp.get("truncate").and_then(Value::as_bool) {
                    w.bool("rope.scaling.yarn_truncate", t);
                }
                if let Some(x) = rp.get("extrapolation_factor").and_then(Value::as_f64) {
                    w.f32("rope.scaling.yarn_ext_factor", x);
                }
                if let Some(x) = rp.get("attention_factor").or_else(|| rp.get("attn_factor")).and_then(Value::as_f64) {
                    w.f32("rope.scaling.yarn_attn_factor", x);
                }
                if let Some(x) = rp.get("beta_fast").and_then(Value::as_f64) {
                    w.f32("rope.scaling.yarn_beta_fast", x);
                }
                if let Some(x) = rp.get("beta_slow").and_then(Value::as_f64) {
                    w.f32("rope.scaling.yarn_beta_slow", x);
                }
            }
            ("su" | "longrope", _) => w.str("rope.scaling.type", "longrope"),
            _ => {}
        }
    }
    if let Some(Value::Array(sections)) = rope.get("mrope_section") {
        let mut v: Vec<i64> = sections.iter().filter_map(Value::as_i64).collect();
        while v.len() < 4 {
            v.push(0);
        }
        w.put(
            "rope.dimension_sections",
            MetaValue::Array { ty: 5, items: v[..4].iter().map(|&x| MetaValue::I32(x as i32)).collect() },
        );
    }
    if let Some(t) = rp.get("rope_theta").and_then(Value::as_f64) {
        w.f32("rope.freq_base", t);
    }
    if let Some(t) = rope.get("sliding_attention").and_then(|s| s.get("rope_theta")).and_then(Value::as_f64) {
        w.f32("rope.freq_base_swa", t);
    }
    let rms = if f.deepseek() { Some(h.f64(&["rms_norm_eps"]).unwrap_or(1e-6)) } else { h.f64(&["rms_norm_eps", "norm_eps"]) };
    if let Some(e) = rms {
        w.f32("attention.layer_norm_rms_epsilon", e);
    }
    if let Some(e) = h.f64(&["layer_norm_eps", "layer_norm_epsilon", "norm_epsilon"]) {
        w.f32("attention.layer_norm_epsilon", e);
    }
    if let Some(n) = h.u64(&["num_local_experts", "num_experts", "n_routed_experts"]) {
        w.u32("expert_count", n);
    }
    if let Some(n) = h.u64(&["num_experts_per_tok", "num_experts_per_token", "top_k_experts"]) {
        w.u32("expert_used_count", n);
    }
    if let Some(n) = h.map.get("n_group").and_then(Value::as_u64) {
        w.u32("expert_group_count", n);
    }
    if let Some(n) = h.map.get("topk_group").and_then(Value::as_u64) {
        w.u32("expert_group_used_count", n);
    }
    let score = h.find(&[
        "score_function",
        "scoring_func",
        "score_func",
        "moe_router_activation",
        "moe_router_activation_func",
        "expert_selection_fn",
        "router_score_func",
    ]);
    if let Some(s) = score.and_then(Value::as_str) {
        let id = match s {
            "softmax" => 1,
            "sigmoid" => 2,
            "sqrtsoftplus" => 4,
            other => return Err(invalid(format!("config.json: expert score function `{other}` is not one the converter writes"))),
        };
        w.u32("expert_gating_func", id);
    }
    if let Some(hd) = h.map.get("head_dim").and_then(Value::as_u64) {
        w.u32("attention.key_length", hd);
        w.u32("attention.value_length", hd);
    }
    Ok(())
}

/// Qwen3.8-Flash-Next's own keys (`Qwen4ExpTextModel.set_gguf_parameters`): the hyper-connections, QSA's indexer and
/// every block's compression ratio (its full-attention layers' and the draft block's), and the per-layer embedding's
/// layer, n-grams, convolution and end of document. Its hash constants and table width are the checkpoint's tensors'
/// ([`HfCheckpoint::ple_parameters`]).
fn qwen4exp_parameters(h: &Hparams, w: &mut Writer, block_count: u64, n_layers: u64) -> io::Result<()> {
    w.u32("hyper_connection.count", h.need_u64("hc_count")?);
    w.u32("hyper_connection.low_rank", h.need_u64("hc_lowrank")?);
    w.u32("attention.indexer.head_count", h.need_u64("indexer_n_heads")?);
    w.u32("attention.indexer.key_length", h.need_u64("indexer_head_dim")?);
    w.u32("attention.indexer.top_k", h.need_u64("indexer_budget")?);
    let ratio = h.need_u64("indexer_compress_ratio")?;
    // Transformers saves a QSA layer as `indexed_attention`, Qwen's configurations name it `full_attention`; QSA selects
    // on both. The converter, reading the configuration through transformers, matches `full_attention` alone and so
    // writes every QSA layer of the stack a ratio of 0 (llama.cpp then attends it densely): this writes the model.
    let full: Vec<bool> = match h.map.get("layer_types") {
        Some(Value::Array(types)) => {
            types.iter().map(|t| matches!(t.as_str(), Some("full_attention" | "indexed_attention"))).collect()
        }
        _ => {
            let every = h.u64(&["full_attention_interval"]).unwrap_or(4);
            (0..n_layers).map(|l| (l + 1) % every == 0).collect()
        }
    };
    let ratios: Vec<MetaValue> = (0..block_count as usize)
        .map(|l| MetaValue::U32(if l >= n_layers as usize || full[l] { ratio as u32 } else { 0 }))
        .collect();
    w.put("attention.compress_ratios", MetaValue::Array { ty: 4, items: ratios });
    let layers: Vec<u64> = match h.map.get("ple_layer_ids") {
        Some(Value::Array(ids)) => ids.iter().filter_map(Value::as_u64).map(|l| l - 1).collect(),
        _ => Vec::new(),
    };
    if layers.is_empty() {
        return Ok(());
    }
    w.put("ple.layers", MetaValue::Array { ty: 4, items: layers.iter().map(|&l| MetaValue::U32(l as u32)).collect() });
    w.u32("ple.ngram_size", h.need_u64("ngram_size")?);
    w.u32("ple.heads_per_ngram", h.need_u64("heads_per_ngram")?);
    w.u32("ple.conv_kernel", h.need_u64("ple_conv_kernel_size")?);
    // The hash resets its n-grams on the primary end of document: the last of a list.
    let eos = match h.map.get("eos_token_id") {
        Some(Value::Array(ids)) => ids.last().and_then(Value::as_u64),
        Some(v) => v.as_u64(),
        None => None,
    }
    .ok_or_else(|| invalid("config.json: eos_token_id is required: the n-gram hash resets on it"))?;
    w.u32("ple.eos_token_id", eos);
    if let Some(image) = h.map.get("image_token_id").and_then(Value::as_u64) {
        w.u32("ple.image_token_id", image);
    }
    Ok(())
}

/// Each family's own keys, written after the base's (a repeated key replaces the base's value).
fn family_parameters(f: Family, h: &Hparams, rope: &Map<String, Value>, w: &mut Writer, block_count: u64) -> io::Result<()> {
    let head_dim = || -> io::Result<u64> {
        Ok(match h.u64(&["head_dim"]) {
            Some(d) => d,
            None => h.need_u64("hidden_size")? / h.need_u64("num_attention_heads")?,
        })
    };
    match f {
        Family::Llama => {
            w.u32("vocab_size", h.need_u64("vocab_size")?);
            w.u32("rope.dimension_count", head_dim()?);
        }
        // `Mistral3Model.Ministral3Model`: Llama's keys, and Ministral 3's YaRN log multiplier (`mscale_all_dim` as it
        // stands) and attention temperature.
        Family::Mistral3 => {
            w.u32("vocab_size", h.need_u64("vocab_size")?);
            w.u32("rope.dimension_count", head_dim()?);
            if mistral3_text(h)? {
                let rp = full_rope(rope);
                let number = |k: &str| rp.get(k).and_then(Value::as_f64).ok_or_else(|| invalid(format!("config.json: no `{k}`")));
                w.f32("rope.scaling.yarn_log_multiplier", number("mscale_all_dim")?);
                w.f32("attention.temperature_scale", number("llama_4_scaling_beta")?);
            }
        }
        Family::Qwen2 | Family::Qwen3 => {}
        Family::Qwen3Moe | Family::Qwen2Moe => {
            if let Some(n) = h.u64(&["moe_intermediate_size"]) {
                w.u32("expert_feed_forward_length", n);
            }
            if let Some(n) = h.u64(&["shared_expert_intermediate_size"]) {
                w.u32("expert_shared_feed_forward_length", n);
            }
        }
        Family::Qwen35 | Family::Qwen35Moe | Family::Qwen3Next | Family::Qwen4Exp => {
            if let Some(n) = h.u64(&["moe_intermediate_size"]) {
                w.u32("expert_feed_forward_length", n);
            }
            if let Some(n) = h.u64(&["shared_expert_intermediate_size"]) {
                w.u32("expert_shared_feed_forward_length", n);
            }
            let n_layers = h.need_u64("num_hidden_layers")?;
            if block_count > n_layers {
                w.u32("nextn_predict_layers", block_count - n_layers);
            }
            w.u32("ssm.conv_kernel", h.need_u64("linear_conv_kernel_dim")?);
            w.u32("ssm.state_size", h.need_u64("linear_key_head_dim")?);
            w.u32("ssm.group_count", h.need_u64("linear_num_key_heads")?);
            w.u32("ssm.time_step_rank", h.need_u64("linear_num_value_heads")?);
            w.u32("ssm.inner_size", h.need_u64("linear_value_head_dim")? * h.need_u64("linear_num_value_heads")?);
            if let Some(Value::Array(types)) = h.map.get("layer_types") {
                if types.len() as u64 != n_layers {
                    return Err(invalid(format!("config.json: layer_types has {} entries for {n_layers} layers", types.len())));
                }
                let mut recurrent: Vec<MetaValue> =
                    types.iter().map(|t| MetaValue::Bool(t.as_str() == Some("linear_attention"))).collect();
                recurrent.resize(block_count as usize, MetaValue::Bool(false));
                w.put("attention.recurrent_layers", MetaValue::Array { ty: 7, items: recurrent });
            }
            w.u32("full_attention_interval", h.u64(&["full_attention_interval"]).unwrap_or(4));
            let factor = rope
                .get("partial_rotary_factor")
                .or_else(|| h.map.get("partial_rotary_factor"))
                .and_then(Value::as_f64)
                .unwrap_or(0.25);
            w.u32("rope.dimension_count", (head_dim()? as f64 * factor) as u64);
            // Qwen3.5 rotates in multimodal sections; Qwen3-Next's rotation is a plain NEOX one.
            if f != Family::Qwen3Next && !rope.contains_key("mrope_section") {
                w.put(
                    "rope.dimension_sections",
                    MetaValue::Array { ty: 5, items: [11, 11, 10, 0].iter().map(|&x| MetaValue::I32(x)).collect() },
                );
            }
            if f == Family::Qwen4Exp {
                qwen4exp_parameters(h, w, block_count, n_layers)?;
            }
        }
        // `GemmaModel.set_gguf_parameters` writes nothing past what `base_parameters` writes.
        Family::Gemma => {}
        // `Gemma2Model.set_gguf_parameters`: both caps and the window. `base_parameters` writes the heads, the head
        // width and the rotation base, which the converter leaves out and llama.cpp reads where it is written.
        Family::Gemma2 => {
            for key in ["attn_logit_softcapping", "final_logit_softcapping"] {
                w.f32(key, h.f64(&[key]).ok_or_else(|| invalid(format!("config.json: no `{key}`")))?);
            }
            w.u32("attention.sliding_window", h.need_u64("sliding_window")?);
        }
        Family::Gemma3 => {
            let get = |k: &str| h.map.get(k).filter(|v| !v.is_null());
            w.u32("context_length", get("max_position_embeddings").and_then(Value::as_u64).unwrap_or(131072));
            w.u32("attention.head_count", get("num_attention_heads").and_then(Value::as_u64).unwrap_or(8));
            w.f32("attention.layer_norm_rms_epsilon", get("rms_norm_eps").and_then(Value::as_f64).unwrap_or(1e-6));
            let hd = get("head_dim").and_then(Value::as_u64).unwrap_or(256);
            w.u32("attention.key_length", hd);
            w.u32("attention.value_length", hd);
            let theta = full_rope(rope).get("rope_theta").and_then(Value::as_f64).unwrap_or(1_000_000.0);
            w.f32("rope.freq_base", theta);
            if let Some(cap) = get("final_logit_softcapping").and_then(Value::as_f64).filter(|c| *c != 0.0) {
                w.f32("final_logit_softcapping", cap);
            }
            if get("sliding_window_pattern").and_then(Value::as_u64) != Some(1) {
                let window = get("sliding_window")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| invalid("config.json: gemma3 without sliding_window"))?;
                w.u32("attention.sliding_window", window);
            }
            w.u32("attention.head_count_kv", get("num_key_value_heads").and_then(Value::as_u64).unwrap_or(4));
        }
        Family::GptOss => {
            w.u32("attention.sliding_window", h.need_u64("sliding_window")?);
            w.u32("expert_feed_forward_length", h.need_u64("intermediate_size")?);
        }
        Family::Glm4 => {
            // `int(head_dim · partial_rotary_factor)` rotated dimensions of each head (half, the converter's default).
            let partial = rope.get("partial_rotary_factor").and_then(Value::as_f64).unwrap_or(0.5);
            w.u32("rope.dimension_count", (head_dim()? as f64 * partial) as u64);
        }
        Family::Glm4Moe => {
            // Its rotation as GLM-4's; DeepSeek's mixture keys, the experts always scored by sigmoids; the draft blocks
            // counted in the stack and named.
            let partial = rope.get("partial_rotary_factor").and_then(Value::as_f64).unwrap_or(0.5);
            w.u32("rope.dimension_count", (head_dim()? as f64 * partial) as u64);
            if let Some(n) = h.u64(&["moe_intermediate_size"]) {
                w.u32("expert_feed_forward_length", n);
            }
            if let Some(n) = h.u64(&["n_shared_experts"]) {
                w.u32("expert_shared_count", n);
            }
            if let Some(n) = h.u64(&["first_k_dense_replace"]) {
                w.u32("leading_dense_block_count", n);
            }
            w.u32("expert_gating_func", 2);
            if let Some(s) = h.f64(&["routed_scaling_factor"]) {
                w.f32("expert_weights_scale", s);
            }
            if let Some(b) = h.map.get("norm_topk_prob").and_then(Value::as_bool) {
                w.bool("expert_weights_norm", b);
            }
            if let Some(n) = h.u64(&["num_nextn_predict_layers"]) {
                w.u32("nextn_predict_layers", n);
            }
        }
        // `LFM2Model.set_gguf_parameters`: every block's KV heads, 0 for a convolution block; the vocabulary, the
        // convolution's taps, the epsilon and the adjusted FFN width; the rotation base `Lfm2Config` takes where none is
        // named.
        // `Mamba2Model.set_gguf_parameters`, which writes the scan's shape and none of the base keys but placeholders.
        Family::Mamba2 => {
            let (d, inner, head) = mamba2_widths(h)?;
            w.meta.retain(|k, _| k.ends_with(".block_count"));
            w.u32("context_length", 1 << 20);
            w.u32("embedding_length", d);
            w.u32("feed_forward_length", 0);
            w.u32("attention.head_count", 0);
            w.u32("ssm.conv_kernel", h.u64(&["conv_kernel", "d_conv"]).filter(|&k| k > 0).unwrap_or(4));
            w.u32("ssm.inner_size", inner);
            w.u32("ssm.state_size", h.u64(&["state_size", "d_state"]).filter(|&s| s > 0).unwrap_or(128));
            w.u32("ssm.time_step_rank", inner / head);
            w.u32("ssm.group_count", h.u64(&["n_groups"]).filter(|&g| g > 0).unwrap_or(1));
            w.f32("attention.layer_norm_rms_epsilon", h.f64(&["layer_norm_epsilon", "rms_norm_eps"]).filter(|&e| e != 0.0).unwrap_or(1e-5));
        }
        // `FalconH1Model.set_gguf_parameters`: `Mamba2Model`'s scan keys read under Falcon-H1's `mamba_` names, then the
        // stack's own: the vocabulary, the context, the FFN width, the heads and their widths, the rotation base.
        // `KimiLinearModel.set_gguf_parameters`: one KV head for each attention block (0 for a delta attention block), the
        // convolution's taps and the delta attention's head width, the latent attention's ranks and widths (the key the
        // latent and its shared part), the experts' widths and counts, the leading dense blocks and the routed scale.
        Family::KimiLinear => {
            let attends = kimi_linear_layers(h)?;
            let c = h.find(&["linear_attn_config"]).and_then(Value::as_object).cloned().unwrap_or_default();
            w.u32("vocab_size", h.need_u64("vocab_size")?);
            w.put("attention.head_count_kv", MetaValue::Array { ty: 5, items: attends.iter().map(|&a| MetaValue::I32(i32::from(a))).collect() });
            if let Some(taps) = c.get("short_conv_kernel_size").and_then(Value::as_u64) {
                w.u32("ssm.conv_kernel", taps);
            }
            if let Some(hd) = c.get("head_dim").and_then(Value::as_u64) {
                w.u32("kda.head_dim", hd);
            }
            if let Some(q) = h.u64(&["q_lora_rank", "n_lora_q"]) {
                w.u32("attention.q_lora_rank", q);
            }
            let lora = h.need_u64("kv_lora_rank")?;
            let rope_dims = h.need_u64("qk_rope_head_dim")?;
            w.u32("attention.kv_lora_rank", lora);
            w.u32("rope.dimension_count", rope_dims);
            w.u32("attention.key_length", lora + rope_dims);
            if let Some(nope) = h.u64(&["qk_nope_head_dim"]) {
                w.u32("attention.key_length_mla", nope + rope_dims);
            }
            if let Some(v) = h.u64(&["v_head_dim"]) {
                w.u32("attention.value_length_mla", v);
            }
            w.u32("expert_feed_forward_length", h.need_u64("moe_intermediate_size")?);
            w.u32("expert_shared_count", h.need_u64("num_shared_experts")?);
            w.u32("leading_dense_block_count", h.need_u64("first_k_dense_replace")?);
            w.f32("expert_weights_scale", h.f64(&["routed_scaling_factor"]).ok_or_else(|| invalid("config.json: no routed_scaling_factor, which the converter reads"))?);
        }
        // `Gemma3NModel.set_gguf_parameters` over `Gemma3Model`'s: each block's FFN width, the AltUp streams, the per-layer
        // input width, the shared blocks, each block's sparsity as the standard normal's quantile in f32
        // (`Normal.icdf`: −∞ for none) and which blocks attend through the window.
        Family::Gemma3n => {
            let blocks = gemma3n_layers(h)?;
            let get = |k: &str| h.map.get(k).filter(|v| !v.is_null());
            w.u32("context_length", get("max_position_embeddings").and_then(Value::as_u64).unwrap_or(131072));
            w.u32("attention.head_count", get("num_attention_heads").and_then(Value::as_u64).unwrap_or(8));
            w.f32("attention.layer_norm_rms_epsilon", get("rms_norm_eps").and_then(Value::as_f64).unwrap_or(1e-6));
            let hd = get("head_dim").and_then(Value::as_u64).unwrap_or(256);
            w.u32("attention.key_length", hd);
            w.u32("attention.value_length", hd);
            w.f32("rope.freq_base", full_rope(rope).get("rope_theta").and_then(Value::as_f64).unwrap_or(1_000_000.0));
            if let Some(cap) = get("final_logit_softcapping").and_then(Value::as_f64).filter(|c| *c != 0.0) {
                w.f32("final_logit_softcapping", cap);
            }
            w.u32("attention.sliding_window", get("sliding_window").and_then(Value::as_u64).ok_or_else(|| invalid("config.json: gemma3n without sliding_window"))?);
            w.u32("attention.head_count_kv", get("num_key_value_heads").and_then(Value::as_u64).unwrap_or(4));
            w.put("feed_forward_length", MetaValue::Array { ty: 5, items: blocks.iter().map(|&(_, ff, _)| MetaValue::I32(ff as i32)).collect() });
            w.u32("altup.active_idx", h.need_u64("altup_active_idx")?);
            w.u32("altup.num_inputs", h.need_u64("altup_num_inputs")?);
            w.u32("embedding_length_per_layer_input", h.need_u64("hidden_size_per_layer_input")?);
            w.u32("attention.shared_kv_layers", h.u64(&["num_kv_shared_layers"]).unwrap_or(0));
            let quantile = |sparse: bool| MetaValue::F32(if sparse { NORMAL_95TH_PERCENTILE } else { f32::NEG_INFINITY });
            w.put("activation_sparsity_scale", MetaValue::Array { ty: 6, items: blocks.iter().map(|&(_, _, sparse)| quantile(sparse)).collect() });
            w.put("attention.sliding_window_pattern", MetaValue::Array { ty: 7, items: blocks.iter().map(|&(windowed, _, _)| MetaValue::Bool(windowed)).collect() });
        }
        // `JambaModel.set_gguf_parameters`, which writes its own keys alone: each block's KV heads (0 for a scan block),
        // the scan's shape (an inner width of `mamba_expand` times the model's), the epsilon and the experts.
        Family::Jamba => {
            let blocks = jamba_blocks(h)?;
            let d = h.need_u64("hidden_size")?;
            let kv = h.need_u64("num_key_value_heads")?;
            w.meta.retain(|k, _| k.ends_with(".block_count"));
            w.u32("context_length", h.u64(&["max_position_embeddings", "n_ctx"]).ok_or_else(|| invalid("config.json: no max_position_embeddings"))?);
            w.u32("embedding_length", d);
            w.u32("feed_forward_length", h.need_u64("intermediate_size")?);
            w.u32("attention.head_count", h.need_u64("num_attention_heads")?);
            w.put("attention.head_count_kv", MetaValue::Array { ty: 5, items: blocks.iter().map(|&(a, _)| MetaValue::I32(if a { kv as i32 } else { 0 })).collect() });
            w.u32("ssm.conv_kernel", h.u64(&["mamba_d_conv"]).unwrap_or(4));
            w.u32("ssm.inner_size", h.need_u64("mamba_expand")? * d);
            w.u32("ssm.state_size", h.u64(&["mamba_d_state"]).unwrap_or(16));
            w.u32("ssm.time_step_rank", h.u64(&["mamba_dt_rank"]).unwrap_or(d.div_ceil(16)));
            w.f32("attention.layer_norm_rms_epsilon", h.f64(&["layer_norm_epsilon", "rms_norm_eps"]).filter(|&e| e != 0.0).unwrap_or(1e-6));
            w.u32("expert_count", h.u64(&["num_local_experts", "num_experts"]).ok_or_else(|| invalid("config.json: no num_experts, which the converter reads"))?);
            w.u32("expert_used_count", h.u64(&["num_experts_per_tok", "num_experts_per_token"]).ok_or_else(|| invalid("config.json: no num_experts_per_tok, which the converter reads"))?);
        }
        // `NemotronHModel.set_gguf_parameters` over `GraniteHybridModel`'s: Llama's keys, then each block's KV heads (0 but
        // for attention) and FFN width (0 but for an FFN), the scan's shape, the head widths, and a context of 2²⁰ for a
        // model whose attention takes no positions.
        Family::NemotronH => {
            let blocks = nemotron_h_blocks(h)?;
            let (inner, heads, state, groups) = nemotron_h_widths(h)?;
            let kv = h.u64(&["num_key_value_heads"]).unwrap_or(h.need_u64("num_attention_heads")?);
            let ff = h.need_u64("intermediate_size")?;
            let head_dim = h.u64(&["head_dim", "attention_head_dim"]).ok_or_else(|| invalid("config.json: no head_dim, which the converter reads"))?;
            let per_block = |f: &dyn Fn(NemotronBlock) -> u64| MetaValue::Array { ty: 5, items: blocks.iter().map(|&b| MetaValue::I32(f(b) as i32)).collect() };
            w.put("feed_forward_length", per_block(&|b| if b == NemotronBlock::Ffn { ff } else { 0 }));
            w.put("attention.head_count_kv", per_block(&|b| if b == NemotronBlock::Attention { kv } else { 0 }));
            w.u32("context_length", 1 << 20);
            w.u32("vocab_size", h.need_u64("vocab_size")?);
            w.u32("rope.dimension_count", head_dim);
            w.u32("ssm.conv_kernel", h.u64(&["conv_kernel", "d_conv"]).unwrap_or(4));
            w.u32("ssm.state_size", state);
            w.u32("ssm.group_count", groups);
            w.u32("ssm.inner_size", inner);
            w.u32("ssm.time_step_rank", heads);
            w.bool("rope.scaling.finetuned", false);
            w.u32("attention.key_length", head_dim);
            w.u32("attention.value_length", head_dim);
            if let Some(eps) = h.f64(&["layer_norm_epsilon"]) {
                w.f32("attention.layer_norm_epsilon", eps);
            }
        }
        Family::FalconH1 => {
            let (d, inner, head, groups) = falcon_h1_widths(h)?;
            w.meta.retain(|k, _| k.ends_with(".block_count"));
            w.u32("context_length", h.u64(&["max_position_embeddings"]).unwrap_or(0));
            w.u32("embedding_length", d);
            w.u32("feed_forward_length", h.need_u64("intermediate_size")?);
            w.u32("attention.head_count", h.need_u64("num_attention_heads")?);
            w.u32("ssm.conv_kernel", h.u64(&["mamba_d_conv"]).filter(|&k| k > 0).unwrap_or(4));
            w.u32("ssm.inner_size", inner);
            w.u32("ssm.state_size", h.u64(&["mamba_d_state"]).filter(|&s| s > 0).unwrap_or(128));
            w.u32("ssm.time_step_rank", inner / head);
            w.u32("ssm.group_count", groups);
            w.f32("attention.layer_norm_rms_epsilon", h.f64(&["layer_norm_epsilon", "rms_norm_eps"]).filter(|&e| e != 0.0).unwrap_or(1e-5));
            w.u32("vocab_size", h.need_u64("vocab_size")?);
            w.u32("attention.head_count_kv", h.need_u64("num_key_value_heads")?);
            w.u32("attention.key_length", h.need_u64("head_dim")?);
            w.u32("attention.value_length", h.need_u64("head_dim")?);
            let theta = full_rope(rope).get("rope_theta").and_then(Value::as_f64).ok_or_else(|| invalid("config.json: no rope_theta, which the converter reads"))?;
            w.f32("rope.freq_base", theta);
        }
        Family::Lfm2 => {
            let kv = h.need_u64("num_key_value_heads")?;
            let heads = lfm2_layers(h)?.into_iter().map(|a| MetaValue::I32(if a { kv as i32 } else { 0 })).collect();
            w.put("attention.head_count_kv", MetaValue::Array { ty: 5, items: heads });
            w.u32("vocab_size", h.need_u64("vocab_size")?);
            w.u32("shortconv.l_cache", h.u64(&["conv_L_cache"]).unwrap_or(3));
            w.f32("attention.layer_norm_rms_epsilon", h.f64(&["norm_eps"]).unwrap_or(1e-5));
            w.u32("feed_forward_length", lfm2_ffn(h)?);
            if full_rope(rope).get("rope_theta").is_none() {
                w.f32("rope.freq_base", LFM2_THETA);
            }
        }
        // `Phi3MiniModel.set_gguf_parameters`, which PhiMoE's class extends with its experts (`base_parameters`'s).
        Family::Phi3 | Family::PhiMoe => {
            // `int(partial_rotary_factor · hidden_size) // heads` rotated dimensions, the original context (both read
            // from the rope parameters, the configuration's own keys where they name none), and a window of 0 where the
            // configuration names none (Phi-4).
            let (d, heads) = (h.need_u64("hidden_size")?, h.need_u64("num_attention_heads")?);
            let partial = rope.get("partial_rotary_factor").and_then(Value::as_f64).unwrap_or(1.0);
            w.u32("rope.dimension_count", (partial * d as f64) as u64 / heads);
            let original = rope
                .get("original_max_position_embeddings")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid("config.json: no original_max_position_embeddings, which the converter's Phi-3 class reads"))?;
            w.u32("rope.scaling.original_context_length", original);
            w.u32("attention.sliding_window", h.u64(&["sliding_window"]).unwrap_or(0));
        }
        Family::MinimaxM2 => {
            w.u32("expert_feed_forward_length", h.need_u64("intermediate_size")?);
            w.u32("rope.dimension_count", h.need_u64("rotary_dim")?);
        }
        // `Olmo2Model.set_gguf_parameters`: OLMo 3's window, on the layers `layer_types` windows — all but every fourth
        // where it names none.
        Family::Olmo2 => {
            if let Some(window) = h.u64(&["sliding_window"]) {
                w.u32("attention.sliding_window", window);
                let pattern: Vec<MetaValue> = match h.map.get("layer_types") {
                    Some(Value::Array(types)) => types.iter().map(|t| MetaValue::Bool(t.as_str() == Some("sliding_attention"))).collect(),
                    _ => (0..block_count).map(|l| MetaValue::Bool((l + 1) % 4 != 0)).collect(),
                };
                w.put("attention.sliding_window_pattern", MetaValue::Array { ty: 7, items: pattern });
            }
        }
        // `Gemma4Model.set_gguf_parameters` over `Gemma3Model`'s: Gemma 3's keys (its defaults where unset), the shared
        // blocks, the per-layer input width, the window pattern, the global and windowed blocks' head widths, the
        // experts' width, each block's FFN width where the shared blocks' are doubled, each block's KV heads where the
        // global blocks' differ, and the global blocks' rotated width (the whole head, `rope_freqs` turning the share
        // past the partial factor) beside the windowed ones' (the whole head: the converter reads no partial factor of
        // theirs).
        Family::Gemma4 => {
            let blocks = gemma4_blocks(h)?;
            let get = |k: &str| h.map.get(k).filter(|v| !v.is_null());
            w.u32("context_length", get("max_position_embeddings").and_then(Value::as_u64).unwrap_or(131072));
            w.u32("attention.head_count", get("num_attention_heads").and_then(Value::as_u64).unwrap_or(8));
            w.f32("attention.layer_norm_rms_epsilon", get("rms_norm_eps").and_then(Value::as_f64).unwrap_or(1e-6));
            w.f32("rope.freq_base", full_rope(rope).get("rope_theta").and_then(Value::as_f64).unwrap_or(1_000_000.0));
            if let Some(cap) = get("final_logit_softcapping").and_then(Value::as_f64).filter(|c| *c != 0.0) {
                w.f32("final_logit_softcapping", cap);
            }
            w.u32("attention.sliding_window", h.need_u64("sliding_window")?);
            let kv_swa = get("num_key_value_heads").and_then(Value::as_u64);
            w.u32("attention.head_count_kv", kv_swa.unwrap_or(4));
            w.u32("attention.shared_kv_layers", h.need_u64("num_kv_shared_layers")?);
            w.u32("embedding_length_per_layer_input", h.u64(&["hidden_size_per_layer_input"]).unwrap_or(0));
            w.put("attention.sliding_window_pattern", MetaValue::Array { ty: 7, items: blocks.swa.iter().map(|&s| MetaValue::Bool(s)).collect() });
            let head_swa = h.need_u64("head_dim")?;
            w.u32("attention.key_length", blocks.head_full);
            w.u32("attention.value_length", blocks.head_full);
            w.u32("attention.key_length_swa", head_swa);
            w.u32("attention.value_length_swa", head_swa);
            if let Some(e) = h.u64(&["expert_intermediate_size", "moe_intermediate_size"]) {
                w.u32("expert_feed_forward_length", e);
            }
            if get("use_double_wide_mlp").and_then(Value::as_bool) == Some(true) {
                w.put("feed_forward_length", MetaValue::Array { ty: 5, items: blocks.ff.iter().map(|&f| MetaValue::I32(f as i32)).collect() });
            }
            if let (Some(full), Some(swa)) = (blocks.kv_full, kv_swa) {
                let kv = blocks.swa.iter().map(|&s| MetaValue::I32((if s { swa } else { full }) as i32)).collect();
                w.put("attention.head_count_kv", MetaValue::Array { ty: 5, items: kv });
            }
            w.u32("rope.dimension_count", blocks.head_full);
            w.u32("rope.dimension_count_swa", head_swa);
        }
        // `Rwkv7Model.set_gguf_parameters`, which writes its own keys alone: a context of 2²⁰ (RWKV takes no positions),
        // the width and stack, the LayerNorms' epsilon, the head width, the four low-rank pairs' ranks, the channel mix's
        // width (four times the model's where unset), and no heads.
        Family::Rwkv7 => {
            let (head, eps) = rwkv7_head_and_eps(h)?;
            let [decay, iclr, value, gate] = rwkv7_ranks(h)?;
            let d = h.need_u64("hidden_size")?;
            w.meta.retain(|k, _| k.ends_with(".block_count"));
            w.u32("context_length", 1 << 20);
            w.u32("embedding_length", d);
            w.f32("attention.layer_norm_epsilon", eps);
            w.u32("wkv.head_size", head);
            w.u32("attention.decay_lora_rank", decay);
            w.u32("attention.iclr_lora_rank", iclr);
            w.u32("attention.value_residual_mix_lora_rank", value);
            w.u32("attention.gate_lora_rank", gate);
            w.u32("feed_forward_length", h.u64(&["intermediate_size"]).unwrap_or(4 * d));
            w.u32("attention.head_count", 0);
        }
        // `MiniMaxText01Model.set_gguf_parameters`: the base's keys, then the one residual scale and the rotated width
        // (`rotary_dim`; transformers' `MiniMaxForCausalLM` rotates the whole head whatever it says, MiniMax's code and
        // llama.cpp that many dimensions of it).
        Family::Minimax01 => {
            w.f32("residual_scale", minimax01_residual_scale(h)?);
            w.u32("rope.dimension_count", h.need_u64("rotary_dim")?);
        }
        Family::Llama4 => {
            // The converter reads `intermediate_size_mlp` as the dense FFN's and `intermediate_size` as an expert's.
            w.u32("vocab_size", h.need_u64("vocab_size")?);
            w.u32("rope.dimension_count", head_dim()?);
            w.u32("feed_forward_length", h.need_u64("intermediate_size_mlp")?);
            w.u32("interleave_moe_layer_step", h.need_u64("interleave_moe_layer_step")?);
            w.u32("expert_feed_forward_length", h.need_u64("intermediate_size")?);
            if let Some(Value::Array(types)) = h.map.get("layer_types") {
                if types.iter().all(|t| t.as_str() == Some("full_attention")) {
                    w.u32("attention.sliding_window", 0);
                }
            }
        }
        Family::Deepseek2 | Family::Deepseek32 | Family::GlmDsa => {
            let n_layers = h.need_u64("num_hidden_layers")?;
            let moe = h.map.get("n_routed_experts").is_some_and(|v| !v.is_null());
            let dense = h.u64(&["first_k_dense_replace"]).unwrap_or(if moe { 0 } else { n_layers });
            w.u32("leading_dense_block_count", dense);
            let kv_lora = h.u64(&["kv_lora_rank"]).unwrap_or(512);
            w.u32("vocab_size", h.need_u64("vocab_size")?);
            if let Some(q) = h.map.get("q_lora_rank").and_then(Value::as_u64) {
                w.u32("attention.q_lora_rank", q);
            }
            let rope_dim = h.need_u64("qk_rope_head_dim")?;
            let nope = h.need_u64("qk_nope_head_dim")?;
            w.u32("attention.kv_lora_rank", kv_lora);
            w.u32("attention.key_length", kv_lora + rope_dim);
            w.u32("attention.value_length", kv_lora);
            w.u32("attention.key_length_mla", nope + rope_dim);
            w.u32("attention.value_length_mla", h.need_u64("v_head_dim")?);
            let expert_ff = h
                .u64(&["moe_intermediate_size", "intermediate_size"])
                .ok_or_else(|| invalid("config.json: no moe_intermediate_size or intermediate_size"))?;
            w.u32("expert_feed_forward_length", expert_ff);
            if let Some(n) = h.map.get("n_routed_experts").and_then(Value::as_u64) {
                w.u32("expert_count", n);
            }
            w.u32("expert_shared_count", h.map.get("n_shared_experts").and_then(Value::as_u64).unwrap_or(0));
            if let Some(s) = h.map.get("routed_scaling_factor").and_then(Value::as_f64) {
                w.f32("expert_weights_scale", s);
            }
            if h.map.get("norm_topk_prob").and_then(Value::as_bool) == Some(true) {
                w.bool("expert_weights_norm", true);
            }
            w.u32("rope.dimension_count", rope_dim);
            if let Some(m) = rope.get("mscale_all_dim").and_then(Value::as_f64) {
                w.f32("rope.scaling.yarn_log_multiplier", 0.1 * m);
            }
            // `DeepseekV32Model` and `GlmMoeDsaModel`: the draft blocks, the indexer, and GLM-5's partial rotation and
            // per-layer indexer kinds (as transformers derives them where the configuration lists none).
            if f != Family::Deepseek2 {
                if let Some(n) = h.u64(&["num_nextn_predict_layers"]) {
                    w.u32("nextn_predict_layers", n);
                }
                w.u32("attention.indexer.head_count", h.need_u64("index_n_heads")?);
                w.u32("attention.indexer.key_length", h.need_u64("index_head_dim")?);
                w.u32("attention.indexer.top_k", h.need_u64("index_topk")?);
            }
            if f == Family::GlmDsa {
                let partial = rope.get("partial_rotary_factor").and_then(Value::as_f64).unwrap_or(1.0);
                w.u32("rope.dimension_count", (rope_dim as f64 * partial) as u64);
                let full = dsa_layers(h)?;
                w.put("attention.indexer.types", MetaValue::Array { ty: 7, items: full.into_iter().map(MetaValue::Bool).collect() });
            }
        }
    }
    Ok(())
}

/// Llama 3's per-frequency rotary factors, as the converter computes them in f32: `1` above the high-frequency
/// wavelength, `factor` below the low one, and the smooth blend between.
fn llama3_rope_factors(h: &Hparams, rope: &Map<String, Value>) -> Option<Vec<f32>> {
    let rp = full_rope(rope);
    if !rp.get("rope_type").and_then(Value::as_str).is_some_and(|t| t.eq_ignore_ascii_case("llama3")) {
        return None;
    }
    let get = |k: &str, d: f64| rp.get(k).and_then(Value::as_f64).unwrap_or(d);
    let base = get("rope_theta", 10000.0) as f32;
    let dim = match h.u64(&["head_dim"]) {
        Some(d) => d,
        None => h.u64(&["hidden_size"])? / h.u64(&["num_attention_heads"])?,
    } as f32;
    let factor = get("factor", 8.0);
    let low = get("low_freq_factor", 1.0);
    let high = get("high_freq_factor", 4.0);
    let old = rp.get("original_max_position_embeddings").and_then(Value::as_f64).unwrap_or(8192.0);
    let low_wavelen = old / low;
    let high_wavelen = old / high;
    let two_pi = (2.0 * std::f64::consts::PI) as f32;
    Some(
        (0..(dim as usize).div_ceil(2))
            .map(|i| {
                let freq = 1.0f32 / base.powf((2 * i) as f32 / dim);
                let wavelen = two_pi / freq;
                if (wavelen as f64) < high_wavelen {
                    1.0
                } else if (wavelen as f64) > low_wavelen {
                    factor as f32
                } else {
                    let smooth = ((old as f32 / wavelen) - low as f32) / ((high - low) as f32);
                    1.0f32 / ((1.0f32 - smooth) / factor as f32 + smooth)
                }
            })
            .collect(),
    )
}

/// The number of real vocabulary rows Gemma's converter keeps: the configured vocabulary when the checkpoint ships a
/// sentencepiece model (its pieces past that are ignored), else the tokenizer's vocabulary and added tokens.
fn gemma_vocab(dir: &Path, h: &Hparams) -> io::Result<usize> {
    if dir.join("tokenizer.model").is_file() {
        if let Some(n) = h.u64(&["vocab_size_per_layer_input", "vocab_size"]) {
            return Ok(n as usize);
        }
        return sentencepiece_pieces(&std::fs::read(dir.join("tokenizer.model"))?);
    }
    let path = dir.join("tokenizer.json");
    let json: Value = serde_json::from_str(&std::fs::read_to_string(&path)?).map_err(|e| invalid(format!("{}: {e}", path.display())))?;
    let vocab = json["model"]["vocab"].as_object().map_or(0, Map::len);
    let added = json["added_tokens"].as_array().map_or(0, Vec::len);
    Ok(vocab + added)
}

/// The pieces of a sentencepiece `ModelProto`: its top-level field 1 entries.
fn sentencepiece_pieces(bytes: &[u8]) -> io::Result<usize> {
    let varint = |at: &mut usize| -> io::Result<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = *bytes.get(*at).ok_or_else(|| invalid("tokenizer.model ends inside a varint"))?;
            *at += 1;
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(invalid("tokenizer.model holds an overlong varint"))
    };
    let (mut at, mut pieces) = (0usize, 0usize);
    while at < bytes.len() {
        let key = varint(&mut at)?;
        match key & 7 {
            0 => {
                varint(&mut at)?;
            }
            1 => at += 8,
            2 => {
                let len = varint(&mut at)? as usize;
                at += len;
            }
            5 => at += 4,
            w => return Err(invalid(format!("tokenizer.model: wire type {w}"))),
        }
        if key >> 3 == 1 {
            pieces += 1;
        }
    }
    Ok(pieces)
}

impl SpecSource for HfCheckpoint {
    fn arch(&self) -> &str {
        &self.arch
    }
    fn arch_meta(&self, suffix: &str) -> Option<&MetaValue> {
        self.meta.get(&format!("{}.{suffix}", self.arch))
    }
    fn dims(&self, name: &str) -> Option<&[u64]> {
        self.index.get(name).map(|&i| self.views[i].dims.as_slice())
    }
    fn tensor_names(&self) -> Vec<&str> {
        self.views.iter().map(|v| v.name.as_str()).collect()
    }
}

impl WeightSource for HfCheckpoint {
    fn describe(&self, name: &str) -> io::Result<(GgmlType, Vec<u64>)> {
        let v = self.index.get(name).map(|&i| &self.views[i]).ok_or_else(|| invalid(format!("no tensor {name}")))?;
        Ok((self.ty_of(&v.form)?, v.dims.clone()))
    }
    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> io::Result<Vec<u8>> {
        let v = self.index.get(name).map(|&i| &self.views[i]).ok_or_else(|| invalid(format!("no tensor {name}")))?;
        let row = v.dims.first().copied().unwrap_or(1) as usize;
        self.read(&v.form, first * row, count * row)
    }
    fn has(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }
}
