//! A Samba exam's bring-up: a checkpoint on disk handed to `largo emit rdu --chip sn50 --model <checkpoint> --ctx 4 -o
//! DIR` as on the day, the bundle read back off disk (each program parsed and re-certified against the digest its manifest
//! records) and run through the spatial evaluator for one decode step from a fresh state, every weight the manifest binds
//! filled as a host fills it from the checkpoint (`model::bind`: a weight through its view, a weight read as stored Q8_0
//! as the planes the host repacks, a rotary table from the spec), and the token it emits set beside the f64
//! reference's from the same checkpoint (`model::reference`).
//!
//! The including suite declares `mod cli` (`cli_common/mod.rs`) and `mod tiny_models` (`common/tiny_models.rs`).

#![allow(dead_code)]

use crate::cli;
use crate::tiny_models::write_checkpoint;
use logicaffeine_compile::model::bind::{bind, host_values, q8_planes, values, HostTable, Source};
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::reference::{argmax, Reference, WeightSource};
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::spatial::certificate::certify_choreography;
use logicaffeine_compile::spatial::eval::{eval_choreography, eval_choreography_every_tensor, Inputs, Value};
use logicaffeine_compile::spatial::ir::{SpatialProgram, TensorRole};
use logicaffeine_compile::spatial::lower::rdu::parse_neutral;
use logicaffeine_compile::spatial::passes::from_q8::{stored_part, Q8Part};
use std::path::Path;

pub const CTX: usize = 4;
pub const TOK: u32 = 3;

/// The token the bundle `largo emit rdu` writes for `checkpoint` emits for [`TOK`] at position 1, and the token the
/// reference computes from the same checkpoint.
pub fn bring_up(checkpoint: &Path, spec: &ModelSpec, source: &dyn Fn() -> Box<dyn WeightSource>) -> (u32, u32) {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("bundle");
    let out = cli::largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            "--chip",
            "sn50",
            "--model",
            checkpoint.to_str().unwrap(),
            "--ctx",
            &CTX.to_string(),
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "largo emit rdu --model {} failed:\n{}", checkpoint.display(), cli::stderr(&out));

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(bundle.join("manifest.json")).expect("manifest.json")).unwrap();
    let load = |sub: &str, digest: &serde_json::Value| -> SpatialProgram {
        let text = std::fs::read_to_string(bundle.join(sub).join("neutral.json")).unwrap_or_else(|e| panic!("{sub}: {e}"));
        let mut p = parse_neutral(&text).unwrap_or_else(|e| panic!("{sub}/neutral.json parses: {e}"));
        p.certificate = None;
        let c = certify_choreography(&p).unwrap_or_else(|e| panic!("{sub} certifies on its own: {e:?}"));
        assert_eq!(serde_json::json!(format!("{:016x}", c.schedule_digest)), *digest, "{sub}: not the program the manifest records");
        p
    };
    let embed = load("embed", &manifest["embed"]["schedule_digest"]);
    let kinds: Vec<SpatialProgram> = manifest["kinds"]
        .as_array()
        .expect("the manifest's kinds")
        .iter()
        .map(|k| load(k["dir"].as_str().unwrap(), &k["schedule_digest"]))
        .collect();
    let head = load("head", &manifest["head"]["schedule_digest"]);

    // One program run as the manifest binds it: the host's token and position, the hidden state carried in, a weight
    // read from the checkpoint through its binding, a carried state at zero.
    let run = |p: &SpatialProgram, entries: &[serde_json::Value], hidden: Option<&(String, Vec<f64>)>, out: &str| -> Vec<f64> {
        let entry = |name: &str| entries.iter().find(|e| e["program"] == name).unwrap_or_else(|| panic!("the manifest binds `{name}`"));
        let mut inputs = Inputs::default();
        for t in &p.tensors {
            if t.role == TensorRole::Output {
                continue;
            }
            let e = entry(&t.name);
            let bound = e["layer"].as_str().unwrap();
            let n = t.elems.rows * t.elems.cols;
            // The program counts tokens and positions from 1 (`author::decode_step`): token `TOK` at the first position.
            let v = match bound {
                "tok" => vec![f64::from(TOK + 1)],
                "pos" => vec![1.0],
                _ if hidden.is_some_and(|h| h.0 == bound) => hidden.unwrap().1.clone(),
                _ if e["role"] == "state" => vec![0.0; n],
                _ if HostTable::named(bound).is_some() => host_values(spec, bound, CTX, &mut *source())
                    .unwrap_or_else(|e| panic!("`{bound}`: {e}"))
                    .expect("a host table"),
                _ if !e["file"].is_null() => {
                    // A weight read as it is stored Q8_0 is one of its two planes, repacked from the checkpoint's blocks.
                    let (weight, part) = match stored_part(bound) {
                        Some((w, part)) => {
                            assert_eq!(e["repack"], part.repack(), "`{bound}` names its repack");
                            (w, Some(part))
                        }
                        None => (bound, None),
                    };
                    match bind(spec, weight).unwrap_or_else(|e| panic!("`{weight}` is unbound: {e:?}")) {
                        Source::Weight(b) => match part {
                            None => values(&b, &mut *source()),
                            Some(Q8Part::Values) => q8_planes(&b, &mut *source()).map(|p| p.values),
                            Some(Q8Part::Scales) => q8_planes(&b, &mut *source()).map(|p| p.scales),
                        }
                        .unwrap_or_else(|e| panic!("`{bound}`: {e}")),
                        Source::State => vec![0.0; n],
                    }
                }
                _ => continue,
            };
            assert_eq!(v.len(), n, "`{bound}` fills {} of {n}", v.len());
            inputs.tensors.insert(t.id, Value::Float(v));
        }
        let r = eval_choreography(p, &inputs).expect("the program evaluates");
        let o = p.tensors.iter().find(|t| entry(&t.name)["layer"] == out).unwrap_or_else(|| panic!("writes `{out}`"));
        match r.tensors.get(&o.id).expect("written") {
            Value::Float(v) => v.clone(),
            Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
        }
    };
    let mut hidden = ("x".to_string(), run(&embed, manifest["embed"]["tensors"].as_array().unwrap(), None, "x"));
    for b in manifest["layers"].as_array().unwrap() {
        let l = b["layer"].as_u64().unwrap();
        let kind = &kinds[b["kind"].as_u64().unwrap() as usize];
        let out = format!("xo{l}");
        hidden = (out.clone(), run(kind, b["tensors"].as_array().unwrap(), Some(&hidden), &out));
    }
    let next = run(&head, manifest["head"]["tensors"].as_array().unwrap(), Some(&hidden), "next");
    assert_eq!(next.len(), 1, "the head emits one token");
    assert!(next[0] >= 1.0, "the head emits a token counted from 1, not {}", next[0]);

    let mut reference = Reference::new(spec.clone(), BoxedSource(source())).expect("the reference reads the checkpoint");
    let want = argmax(&reference.step(&[TOK]).expect("the reference steps"));
    (next[0] as u32 - 1, want)
}

/// A boxed weight source as one the reference owns.
pub struct BoxedSource(pub Box<dyn WeightSource>);

impl WeightSource for BoxedSource {
    fn describe(&self, name: &str) -> std::io::Result<(GgmlType, Vec<u64>)> {
        self.0.describe(name)
    }
    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>> {
        self.0.row_bytes(name, first, count)
    }
    fn has(&self, name: &str) -> bool {
        self.0.has(name)
    }
}

/// `spec` written as a GGUF in `dir`, brought up, its token held to the reference's.
pub fn brings_up(spec: ModelSpec, what: &str) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{}.gguf", what.replace(' ', "-")));
    write_checkpoint(&spec, &path);
    // The checkpoint is the one source: its spec is what it reads back as.
    let spec = ModelSpec::from_gguf(&Gguf::open(&path).expect("the checkpoint")).expect("the checkpoint reads");
    let gguf = path.clone();
    let (got, want) = bring_up(&path, &spec, &move || Box::new(Gguf::open(&gguf).expect("the checkpoint")) as Box<dyn WeightSource>);
    assert_eq!(got, want, "{what}: the bundle emits token {got}, the reference {want}");
}


/// `check` run on every case, each whatever the others do: a row over several shapes fails naming every shape that
/// does not hold and why, not only the first.
pub fn every<T>(cases: impl IntoIterator<Item = (String, T)>, check: impl Fn(T)) {
    let mut total = 0;
    let failed: Vec<String> = cases
        .into_iter()
        .inspect(|_| total += 1)
        .filter_map(|(what, case)| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(case))).err().map(|e| {
                let why = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                format!("{what}: {why}")
            })
        })
        .collect();
    assert!(failed.is_empty(), "{} of {total} shapes do not hold:\n\n{}", failed.len(), failed.join("\n\n"));
}

/// The bf16 SN50 datapath's stated bound on a tiny model's step against the f64 reference, at a width of 64: the largest
/// logit difference over the largest reference logit (the bound `emit_rdu_e2e` holds a whole forward to). A dot
/// product's rounding error grows with the square root of its length, so a model `d` wide is held to
/// `BF16_LOGIT_REL · √(d / 64)`.
pub const BF16_LOGIT_REL: f64 = 0.02;

/// The bundle `largo emit rdu` writes for `checkpoint`, read back off disk, certified and run for one
/// decode step as [`bring_up`] runs it; the token its head emits and the logits the token is the arg-max of.
fn run_bundle(checkpoint: &Path, spec: &ModelSpec, source: &dyn Fn() -> Box<dyn WeightSource>) -> (u32, Vec<f64>) {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("bundle");
    let out = cli::largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            "--chip",
            "sn50",
            "--model",
            checkpoint.to_str().unwrap(),
            "--ctx",
            &CTX.to_string(),
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "largo emit rdu --model {} failed:\n{}", checkpoint.display(), cli::stderr(&out));

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(bundle.join("manifest.json")).expect("manifest.json")).unwrap();
    let load = |sub: &str, digest: &serde_json::Value| -> SpatialProgram {
        let text = std::fs::read_to_string(bundle.join(sub).join("neutral.json")).unwrap_or_else(|e| panic!("{sub}: {e}"));
        let mut p = parse_neutral(&text).unwrap_or_else(|e| panic!("{sub}/neutral.json parses: {e}"));
        p.certificate = None;
        let c = certify_choreography(&p).unwrap_or_else(|e| panic!("{sub} certifies on its own: {e:?}"));
        assert_eq!(serde_json::json!(format!("{:016x}", c.schedule_digest)), *digest, "{sub}: not the program the manifest records");
        p
    };
    let embed = load("embed", &manifest["embed"]["schedule_digest"]);
    let kinds: Vec<SpatialProgram> = manifest["kinds"]
        .as_array()
        .expect("the manifest's kinds")
        .iter()
        .map(|k| load(k["dir"].as_str().unwrap(), &k["schedule_digest"]))
        .collect();
    let head = load("head", &manifest["head"]["schedule_digest"]);

    // One program run as the manifest binds it: the host's token and position, the hidden state carried in, a weight
    // read from the checkpoint through its binding, a carried state at zero.
    let run = |p: &SpatialProgram, entries: &[serde_json::Value], hidden: Option<&(String, Vec<f64>)>, out: &str| -> Vec<f64> {
        let entry = |name: &str| entries.iter().find(|e| e["program"] == name).unwrap_or_else(|| panic!("the manifest binds `{name}`"));
        let mut inputs = Inputs::default();
        for t in &p.tensors {
            if t.role == TensorRole::Output {
                continue;
            }
            let e = entry(&t.name);
            let bound = e["layer"].as_str().unwrap();
            let n = t.elems.rows * t.elems.cols;
            // The program counts tokens and positions from 1 (`author::decode_step`): token `TOK` at the first position.
            let v = match bound {
                "tok" => vec![f64::from(TOK + 1)],
                "pos" => vec![1.0],
                _ if hidden.is_some_and(|h| h.0 == bound) => hidden.unwrap().1.clone(),
                _ if e["role"] == "state" => vec![0.0; n],
                _ if HostTable::named(bound).is_some() => host_values(spec, bound, CTX, &mut *source())
                    .unwrap_or_else(|e| panic!("`{bound}`: {e}"))
                    .expect("a host table"),
                _ if !e["file"].is_null() => {
                    // A weight read as it is stored Q8_0 is one of its two planes, repacked from the checkpoint's blocks.
                    let (weight, part) = match stored_part(bound) {
                        Some((w, part)) => {
                            assert_eq!(e["repack"], part.repack(), "`{bound}` names its repack");
                            (w, Some(part))
                        }
                        None => (bound, None),
                    };
                    match bind(spec, weight).unwrap_or_else(|e| panic!("`{weight}` is unbound: {e:?}")) {
                        Source::Weight(b) => match part {
                            None => values(&b, &mut *source()),
                            Some(Q8Part::Values) => q8_planes(&b, &mut *source()).map(|p| p.values),
                            Some(Q8Part::Scales) => q8_planes(&b, &mut *source()).map(|p| p.scales),
                        }
                        .unwrap_or_else(|e| panic!("`{bound}`: {e}")),
                        Source::State => vec![0.0; n],
                    }
                }
                _ => continue,
            };
            assert_eq!(v.len(), n, "`{bound}` fills {} of {n}", v.len());
            inputs.tensors.insert(t.id, Value::Float(v));
        }
        let r = eval_choreography_every_tensor(p, &inputs).expect("the program evaluates");
        let o = p.tensors.iter().find(|t| entry(&t.name)["layer"] == out).unwrap_or_else(|| panic!("writes `{out}`"));
        match r.tensors.get(&o.id).expect("written") {
            Value::Float(v) => v.clone(),
            Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
        }
    };
    let mut hidden = ("x".to_string(), run(&embed, manifest["embed"]["tensors"].as_array().unwrap(), None, "x"));
    for b in manifest["layers"].as_array().unwrap() {
        let l = b["layer"].as_u64().unwrap();
        let kind = &kinds[b["kind"].as_u64().unwrap() as usize];
        let out = format!("xo{l}");
        hidden = (out.clone(), run(kind, b["tensors"].as_array().unwrap(), Some(&hidden), &out));
    }
    let next = run(&head, manifest["head"]["tensors"].as_array().unwrap(), Some(&hidden), "next");
    assert_eq!(next.len(), 1, "the head emits one token");
    assert!(next[0] >= 1.0, "the head emits a token counted from 1, not {}", next[0]);
    let logits = run(&head, manifest["head"]["tensors"].as_array().unwrap(), Some(&hidden), "logits");
    assert_eq!(logits.len(), spec.vocab, "the head's logits cover the vocabulary");
    (next[0] as u32 - 1, logits)
}


/// `spec` written as a GGUF and brought up, held to the f64 reference within the bf16 datapath's bound: every logit
/// within [`BF16_LOGIT_REL`] (scaled to the model's width) of the largest reference logit, and the reference's own token wherever the reference
/// leads its next token by more than twice that — a token the datapath's rounding cannot reorder. Below that lead the
/// bundle's token is one the reference cannot tell from its own within the bound.
pub fn brings_up_close(spec: ModelSpec, what: &str) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{}.gguf", what.replace(' ', "-")));
    write_checkpoint(&spec, &path);
    let spec = ModelSpec::from_gguf(&Gguf::open(&path).expect("the checkpoint")).expect("the checkpoint reads");
    let gguf = path.clone();
    let source = move || Box::new(Gguf::open(&gguf).expect("the checkpoint")) as Box<dyn WeightSource>;
    let (got, logits) = run_bundle(&path, &spec, &source);
    let mut reference = Reference::new(spec.clone(), BoxedSource(source())).expect("the reference reads the checkpoint");
    let want = reference.step(&[TOK]).expect("the reference steps");
    let scale = want.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let bound = BF16_LOGIT_REL * (spec.d as f64 / 64.0).sqrt().max(1.0) * scale;
    let worst = logits.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(worst <= bound, "{what}: a logit is {worst:.5} from the reference's, past the datapath's {bound:.5}");
    let top = argmax(&want);
    let lead = want[top as usize] - want.iter().enumerate().filter(|(i, _)| *i != top as usize).map(|(_, v)| *v).fold(f64::NEG_INFINITY, f64::max);
    if lead > 2.0 * bound {
        assert_eq!(got, top, "{what}: the bundle emits token {got}, the reference {top}, which leads by {lead:.5}");
    } else {
        assert!(
            want[top as usize] - want[got as usize] <= 2.0 * bound,
            "{what}: the bundle's token {got} trails the reference's {top} by {:.5}, past twice the datapath's {bound:.5}",
            want[top as usize] - want[got as usize]
        );
    }
}

/// The checkpoint at `checkpoint`, whose spec is `spec` and whose weights `source` reads, brought up and held to the f64
/// reference as [`brings_up_close`] holds a written one, a head that caps its logits held to the bound of its raw
/// logits: the cap is 1-Lipschitz, and the datapath's error is made before it.
pub fn checkpoint_brings_up_close(checkpoint: &Path, spec: &ModelSpec, source: &dyn Fn() -> Box<dyn WeightSource>, what: &str) {
    let (got, logits) = run_bundle(checkpoint, spec, source);
    let mut reference = Reference::new(spec.clone(), BoxedSource(source())).expect("the reference reads the checkpoint");
    let want = reference.step(&[TOK]).expect("the reference steps");
    let largest = |v: &[f64]| v.iter().fold(0.0f64, |m, x| m.max(x.abs()));
    let scale = match spec.final_logit_softcap {
        None => largest(&want),
        Some(_) => {
            let raw = ModelSpec { final_logit_softcap: None, ..spec.clone() };
            let mut uncapped = Reference::new(raw, BoxedSource(source())).expect("the reference reads the checkpoint");
            largest(&uncapped.step(&[TOK]).expect("the reference steps"))
        }
    };
    let bound = BF16_LOGIT_REL * (spec.d as f64 / 64.0).sqrt().max(1.0) * scale;
    let worst = logits.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(worst <= bound, "{what}: a logit is {worst:.5} from the reference's, past the datapath's {bound:.5}");
    let top = argmax(&want);
    let lead = want[top as usize] - want.iter().enumerate().filter(|(i, _)| *i != top as usize).map(|(_, v)| *v).fold(f64::NEG_INFINITY, f64::max);
    if lead > 2.0 * bound {
        assert_eq!(got, top, "{what}: the bundle emits token {got}, the reference {top}, which leads by {lead:.5}");
    } else {
        assert!(
            want[top as usize] - want[got as usize] <= 2.0 * bound,
            "{what}: the bundle's token {got} trails the reference's {top} by {:.5}, past twice the datapath's {bound:.5}",
            want[top as usize] - want[got as usize]
        );
    }
}
