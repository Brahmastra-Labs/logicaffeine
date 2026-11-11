//! What a checkpoint of an architecture no arm reads carries.
//!
//! A checkpoint names its architecture (`general.architecture`) and holds its tensors. When no arm of
//! [`ModelSpec::from_source`] reads the name, the tensors still say what the model is: [`triage`] reads the checkpoint
//! under each architecture an arm reads ([`ARCHITECTURES`]) and, for every reading that succeeds, sets the tensors the
//! author's decode step reads for it beside the checkpoint's own. A reading whose step reads exactly the checkpoint's
//! tensors is a family the checkpoint carries under another name; the rest are ranked by the tensors they would read
//! that the checkpoint lacks and the checkpoint's tensors they would leave unread.
//!
//! Two families can read the same tensors and differ in what only the name selects — Llama's and Qwen3's rotations
//! pair different dimensions — so a refusal names every exact reading, never a guess among them.

use std::collections::BTreeSet;

use super::bind::{bind, Source};
use super::gguf::MetaValue;
use super::program::{written, Cut};
use super::spec::{ModelSpec, SpecError, SpecSource};

/// Every architecture an arm of [`ModelSpec::from_source`] reads, in the order a tie between two readings is broken.
pub const ARCHITECTURES: &[&str] = &[
    "llama", "qwen2", "qwen3", "qwen3moe", "qwen2moe", "qwen35", "qwen35moe", "qwen3next", "gemma3", "gpt-oss", "deepseek2",
    "llama4", "gemma", "gemma2", "glm4", "glm4moe", "minimax-m2", "phi3", "qwen4exp", "granite", "minicpm", "arcee", "olmo2",
    "mistral3", "phimoe", "deepseek32", "glm-dsa", "lfm2", "mamba2", "falcon-h1", "nemotron_h", "nemotron_h_moe", "jamba",
    "gemma3n", "kimi-linear", "minimax-01", "rwkv7", "gemma4",
];

/// A checkpoint read under another architecture's name: its keys, still under its own, and its tensors.
struct Named<'a, S: SpecSource + ?Sized> {
    inner: &'a S,
    arch: &'static str,
}

impl<S: SpecSource + ?Sized> SpecSource for Named<'_, S> {
    fn arch(&self) -> &str {
        self.arch
    }
    fn arch_meta(&self, suffix: &str) -> Option<&MetaValue> {
        self.inner.arch_meta(suffix)
    }
    fn dims(&self, name: &str) -> Option<&[u64]> {
        self.inner.dims(name)
    }
    fn tensor_names(&self) -> Vec<&str> {
        self.inner.tensor_names()
    }
    fn has(&self, name: &str) -> bool {
        self.inner.has(name)
    }
}

/// One architecture's reading of a checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    pub arch: &'static str,
    /// Whether the checkpoint reads as `arch` and the author writes its step; when it does not, `lacks` says why.
    pub reads: bool,
    /// What reading it as `arch` needs and the checkpoint lacks: a key or a tensor ([`SpecError`]'s words) or what the
    /// author does not write yet, or the tensors the step would read that it does not hold.
    pub lacks: Vec<String>,
    /// The checkpoint's tensors the step would not read.
    pub unread: Vec<String>,
}

impl Reading {
    /// Whether the checkpoint is this architecture's under another name: every tensor read, none lacking.
    pub fn exact(&self) -> bool {
        self.reads && self.lacks.is_empty() && self.unread.is_empty()
    }
}

/// The checkpoint tensors a decode step of `spec` reads: every weight its program declares, through its binding, and
/// the rotation's frequency factors when the spec rotates by them.
pub fn tensors_read(spec: &ModelSpec) -> BTreeSet<String> {
    let ((_, f32s, q8s), _) = written(spec, Cut::Decode { ctx: 1 });
    let mut read: BTreeSet<String> = f32s
        .keys()
        .chain(q8s.keys())
        .filter_map(|var| match bind(spec, var) {
            Ok(Source::Weight(b)) => Some(b.tensor),
            _ => None,
        })
        .collect();
    if super::bind::rotations(spec).iter().any(|r| r.scaling == super::spec::RopeScaling::Factors) {
        read.insert("rope_freqs.weight".to_string());
    }
    read
}

/// Every architecture's reading of `g`: the exact ones first, then those that read it by how much each lacks and leaves
/// unread, then those that do not read it.
pub fn triage<S: SpecSource + ?Sized>(g: &S) -> Vec<Reading> {
    let held: BTreeSet<String> = g.tensor_names().into_iter().map(str::to_string).collect();
    let mut readings: Vec<Reading> = ARCHITECTURES
        .iter()
        .map(|&arch| {
            let refused = |why: String| Reading { arch, reads: false, lacks: vec![why], unread: Vec::new() };
            match ModelSpec::from_source(&Named { inner: g, arch }) {
                Ok(spec) => match super::author::check(&spec) {
                    Ok(()) => {
                        let read = tensors_read(&spec);
                        Reading {
                            arch,
                            reads: true,
                            lacks: read.difference(&held).cloned().collect(),
                            unread: held.difference(&read).cloned().collect(),
                        }
                    }
                    Err(e) => refused(format!("the author's {}", e.0)),
                },
                Err(e) => refused(e.to_string()),
            }
        })
        .collect();
    readings.sort_by_key(|r| (!r.exact(), !r.reads, r.lacks.len() + r.unread.len()));
    readings
}

/// Why `g`, of an architecture no arm reads, is refused, and what it carries: the families whose tensors it holds
/// exactly, or the nearest reading and what it lacks and adds.
pub fn refusal<S: SpecSource + ?Sized>(g: &S) -> String {
    let arch = g.arch();
    let readings = triage(g);
    let exact: Vec<&str> = readings.iter().filter(|r| r.exact()).map(|r| r.arch).collect();
    let head = SpecError::UnknownArchitecture(arch.to_string()).to_string();
    if !exact.is_empty() {
        return format!(
            "{head}; its tensors are exactly what {} read{} — the architecture's name selects what the tensors do not \
             (the rotation's pairing, the norms' placement)",
            exact.iter().map(|a| format!("`{a}`")).collect::<Vec<_>>().join(" and "),
            if exact.len() == 1 { "s" } else { "" }
        );
    }
    let list = |v: &[String]| if v.is_empty() { "nothing".to_string() } else { v.join(", ") };
    match readings.first() {
        Some(r) if r.reads => format!(
            "{head}; the nearest architecture an arm reads is `{}`, which lacks {} and leaves unread {}",
            r.arch,
            list(&r.lacks),
            list(&r.unread)
        ),
        _ => format!(
            "{head}; no architecture an arm reads takes it: {}",
            readings.iter().map(|r| format!("`{}` needs {}", r.arch, list(&r.lacks))).collect::<Vec<_>>().join("; ")
        ),
    }
}
