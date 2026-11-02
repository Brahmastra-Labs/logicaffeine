//! Models as data. A checkpoint's own metadata describes its architecture ([`spec`]); this module reads it — a GGUF
//! file ([`gguf`]), or a Hugging Face directory of safetensors shards in place ([`hf`], [`safetensors`]) —,
//! dequantizes its weights in every format a model ships in ([`quant`]), tokenizes text as the checkpoint's vocabulary
//! does ([`tokenizer`]), hashes Qwen3.8-Flash-Next's n-gram embedding rows ([`ngram_hash`]), writes what it computes as
//! a LOGOS program ([`author`]) whose weights it binds to the checkpoint's tensors ([`bind`]) as the layout its arms
//! read presents them ([`layout`]), names each cut of that
//! one program every backend lowers ([`program`]), and evaluates that computation in `f64` as the oracle every lane is
//! measured against ([`reference`](mod@reference)).

pub mod author;
pub mod bind;
pub mod gguf;
pub mod hf;
pub mod layout;
pub mod ngram_hash;
pub mod program;
pub mod quant;
pub mod reference;
pub mod safetensors;
pub mod spec;
pub mod tokenizer;
pub mod triage;

/// A checkpoint a model is brought up from: its metadata, which [`spec::ModelSpec::from_source`] reads, and its tensors,
/// which the reference and every host read through their bindings — a GGUF file or a Hugging Face directory.
pub trait Checkpoint: spec::SpecSource + reference::WeightSource {}

impl<T: spec::SpecSource + reference::WeightSource> Checkpoint for T {}

/// The checkpoint at `path`: a Hugging Face directory (`config.json` beside its safetensors shards) or a GGUF file,
/// presented in the layout the spec's arms read ([`layout::presented`]).
pub fn open_checkpoint(path: &std::path::Path) -> std::io::Result<Box<dyn Checkpoint>> {
    let opened: Box<dyn Checkpoint> =
        if path.is_dir() { Box::new(hf::HfCheckpoint::open(path)?) } else { Box::new(gguf::Gguf::open(path)?) };
    layout::presented(opened)
}
