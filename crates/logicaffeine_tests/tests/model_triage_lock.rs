//! model_triage_lock — a checkpoint whose architecture name no arm reads is refused with what it carries
//! (`model::triage`; INTAKE-9 and exam row E1.9 of `benchmarks/PREPARE_FOR_SAMBA.md`).
//!
//! The checkpoint is read under every architecture an arm of `ModelSpec::from_source` reads — the names its match
//! arms carry, read from its source. A reading whose decode step reads exactly the checkpoint's tensors names a family
//! the checkpoint carries under another name — Llama's, Qwen2's and Qwen3's tensors are alike and only their names
//! choose the rotation's pairing, so a Llama-shaped checkpoint names all three; every other tiny family is the one
//! reading that fits. With no exact reading the refusal names the nearest and the tensors it lacks and leaves unread.
//! The whole triage of a checkpoint takes well under a second.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test model_triage_lock`

#![cfg(feature = "codegen")]

#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::model::gguf::{self, Gguf, MetaValue, TensorOut};
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::spec::{ModelSpec, SpecError};
use logicaffeine_compile::model::triage::{refusal, triage, ARCHITECTURES};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};
use tiny_models::{checkpoint_meta, tensors, Tiny, ALL};

const CTX: usize = 8;

/// `spec`'s checkpoint under the architecture name `arch` — its keys renamed with it — holding the tensors `keep`
/// leaves and the `extra` ones, every value zero (a triage reads names, shapes and keys).
fn renamed(spec: &ModelSpec, arch: &str, keep: &dyn Fn(&str) -> bool, extra: &[&str], dir: &Path) -> Gguf {
    let own = format!("{}.", spec.arch);
    let meta: Vec<(String, MetaValue)> = checkpoint_meta(spec)
        .into_iter()
        .map(|(k, v)| match k.strip_prefix(&own) {
            _ if k == "general.architecture" => (k, MetaValue::Str(arch.as_bytes().to_vec())),
            Some(rest) => (format!("{arch}.{rest}"), v),
            None => (k, v),
        })
        .collect();
    let mut list: Vec<(String, Vec<u64>)> = tensors(spec).into_iter().filter(|(n, _)| keep(n)).collect();
    list.extend(extra.iter().map(|n| (n.to_string(), vec![spec.d as u64])));
    let outs: Vec<TensorOut> = list.iter().map(|(n, d)| TensorOut { name: n.clone(), dims: d.clone(), ty: GgmlType::F32 }).collect();
    let path = dir.join(format!("{}-as-{arch}.gguf", spec.arch));
    gguf::write(&path, &meta, &outs, |k| Ok(vec![0u8; 4 * list[k].1.iter().product::<u64>() as usize])).expect("the checkpoint writes");
    Gguf::open(&path).expect("the checkpoint opens")
}

/// The architectures whose arms read exactly `t`'s tensors.
fn readers(t: Tiny) -> BTreeSet<&'static str> {
    match t {
        Tiny::Llama | Tiny::LlamaTied | Tiny::Qwen3 => ["llama", "qwen2", "qwen3"].into(),
        // `llama` reads a checkpoint that counts experts as Mixtral's, and a QK-norm where its tensors are.
        Tiny::Qwen3Moe => ["llama", "qwen3moe"].into(),
        Tiny::Qwen35 => ["qwen35"].into(),
        Tiny::Gemma3 => ["gemma3"].into(),
        Tiny::GptOss => ["gpt-oss"].into(),
        Tiny::DeepSeek2 | Tiny::DeepSeek2Split => ["deepseek2"].into(),
    }
}

#[test]
fn every_family_under_another_name_is_named_by_its_tensors() {
    let dir = tempfile::tempdir().unwrap();
    for &t in ALL {
        let spec = t.spec(CTX);
        let g = renamed(&spec, "frobnicator", &|_| true, &[], dir.path());
        assert_eq!(ModelSpec::from_gguf(&g).err(), Some(SpecError::UnknownArchitecture("frobnicator".into())), "{t:?}: no arm reads the name");
        let exact: BTreeSet<&str> = triage(&g).iter().filter(|r| r.exact()).map(|r| r.arch).collect();
        assert_eq!(exact, readers(t), "{t:?}: the architectures whose step reads exactly its tensors");
        let why = refusal(&g);
        assert!(why.contains("`frobnicator`"), "{t:?}: the refusal names the architecture: {why}");
        for a in readers(t) {
            assert!(why.contains(&format!("`{a}`")), "{t:?}: the refusal names `{a}`, whose tensors it carries: {why}");
        }
    }
}

#[test]
fn a_checkpoint_no_family_reads_is_refused_with_the_nearest_and_its_differences() {
    // A Llama without its down projections and with a tensor no family reads.
    let dir = tempfile::tempdir().unwrap();
    let spec = Tiny::Llama.spec(CTX);
    let g = renamed(&spec, "frobnicator", &|n| !n.contains("ffn_down"), &["blk.0.frob_gate.weight"], dir.path());
    let readings = triage(&g);
    assert!(readings.iter().all(|r| !r.exact()), "no family reads exactly these tensors");
    let nearest = &readings[0];
    assert_eq!(nearest.arch, "llama", "the nearest reading, Llama's, first of the three that tie");
    let downs: Vec<String> = (0..spec.layers.len()).map(|l| format!("blk.{l}.ffn_down.weight")).collect();
    assert_eq!(nearest.lacks, downs, "it lacks every layer's down projection");
    assert_eq!(nearest.unread, vec!["blk.0.frob_gate.weight".to_string()], "and leaves the extra tensor unread");
    let why = refusal(&g);
    for piece in ["`frobnicator`", "`llama`", "blk.0.ffn_down.weight", "blk.2.ffn_down.weight", "blk.0.frob_gate.weight"] {
        assert!(why.contains(piece), "the refusal names {piece}: {why}");
    }
}

#[test]
fn a_reading_the_checkpoint_cannot_satisfy_names_what_it_needs() {
    // A Llama read as DeepSeek-V2 needs a latent rank the checkpoint has no key for.
    let dir = tempfile::tempdir().unwrap();
    let g = renamed(&Tiny::Llama.spec(CTX), "frobnicator", &|_| true, &[], dir.path());
    let deepseek = triage(&g).into_iter().find(|r| r.arch == "deepseek2").expect("a DeepSeek-V2 reading");
    assert!(!deepseek.exact());
    assert!(deepseek.lacks.iter().any(|w| w.contains("kv_lora_rank")), "it lacks the latent rank's key: {:?}", deepseek.lacks);
}

/// The architecture names the arms of `ModelSpec::from_source` match, read from its source: the string patterns of the
/// arms of its match on the architecture, up to the arm that refuses every other name.
fn arms() -> BTreeSet<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/src/model/spec.rs");
    let src = std::fs::read_to_string(&path).expect("spec.rs");
    let start = src.find("let layers: Vec<Layer> = match arch.as_str() {").expect("from_source's match on the architecture");
    let body = &src[start..];
    let end = body.find("other => return Err(SpecError::UnknownArchitecture").expect("the arm that refuses every other name");
    let arm = "            \"";
    let names: BTreeSet<String> = body[..end]
        .lines()
        .filter(|l| l.starts_with(arm) && l.trim_end().ends_with("=> {"))
        .flat_map(|l| l.trim().trim_end_matches("=> {").split('|').map(|n| n.trim().trim_matches('"').to_string()).collect::<Vec<_>>())
        .collect();
    assert!(names.len() >= 9, "the census reads the arms' names: {names:?}");
    names
}

#[test]
fn every_architecture_an_arm_reads_is_triaged() {
    let tiny: BTreeSet<String> = ALL.iter().map(|t| t.spec(CTX).arch).collect();
    let listed: BTreeSet<String> = ARCHITECTURES.iter().map(|a| a.to_string()).collect();
    assert_eq!(listed, arms(), "the triage reads under every architecture an arm of `from_source` reads");
    assert!(tiny.is_subset(&listed), "every tiny checkpoint's architecture is triaged: {tiny:?}");
    let dir = tempfile::tempdir().unwrap();
    for &a in ARCHITECTURES {
        let g = renamed(&Tiny::Llama.spec(CTX), a, &|_| true, &[], dir.path());
        assert_ne!(ModelSpec::from_gguf(&g).err(), Some(SpecError::UnknownArchitecture(a.into())), "`{a}` reaches an arm");
    }
    for &t in ALL {
        let spec = t.spec(CTX);
        let g = renamed(&spec, &spec.arch.clone(), &|_| true, &[], dir.path());
        assert!(ModelSpec::from_gguf(&g).is_ok(), "{t:?}: its own arm reads it");
    }
}

#[test]
fn a_triage_takes_well_under_a_second() {
    let dir = tempfile::tempdir().unwrap();
    let g = renamed(&Tiny::Qwen35.wide(CTX), "frobnicator", &|_| true, &[], dir.path());
    let start = Instant::now();
    let why = refusal(&g);
    let took = start.elapsed();
    assert!(why.contains("`qwen35`"), "{why}");
    assert!(took < Duration::from_millis(250), "the triage took {took:?}");
}
