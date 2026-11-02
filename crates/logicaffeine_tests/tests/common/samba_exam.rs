//! The Samba exam's harness beside the hand-off's (`samba_handoff.rs`): the reference's continuation of any prompt, the
//! bf16 datapath's bound on a decode's logits, and a batch decoded on the SN50 bundle `largo emit rdu --batch N`
//! writes — each sequence's carried state handed in under its own names, `{name}_s{b}`.

use crate::cli;
use crate::samba_handoff::{authored, decode_inputs, layers_on, run, Model, BF16_LOGIT_REL, CTX};
use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::model::program::Cut;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::reference::{argmax, Reference};
use logicaffeine_compile::spatial::certificate::certify_choreography;
use logicaffeine_compile::spatial::handoff::Handoff;
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::lower::rdu::parse_neutral;
use std::collections::BTreeMap;
use std::path::Path;

/// The f64 reference's logits for `next` after `prompt`.
pub fn reference_after(m: &Model, prompt: &[u32], next: u32) -> Vec<f64> {
    let mut r = Reference::new(m.spec.clone(), Gguf::open(&m.path).expect("the checkpoint")).expect("the reference reads it");
    if !prompt.is_empty() {
        r.step(prompt).expect("the reference prefills");
    }
    r.step(&[next]).expect("the reference steps")
}

/// `logits` held to `want` as `close_to_reference` holds a decode: every logit within the bf16 datapath's bound scaled
/// to the width, and the reference's token wherever it leads by more than twice that.
pub fn within_the_datapath(m: &Model, logits: &[f64], want: &[f64], what: &str) {
    assert_eq!(logits.len(), want.len(), "{what}: the logits cover the vocabulary");
    let scale = want.iter().fold(0.0f64, |acc, v| acc.max(v.abs()));
    let bound = BF16_LOGIT_REL * (m.spec.d as f64 / 64.0).sqrt().max(1.0) * scale;
    let worst = logits.iter().zip(want).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(worst <= bound, "{what}: a logit is {worst:.5} from the reference's, past the datapath's {bound:.5}");
    let (top, got) = (argmax(want) as usize, argmax(logits) as usize);
    let lead = want[top] - want.iter().enumerate().filter(|(i, _)| *i != top).map(|(_, v)| *v).fold(f64::NEG_INFINITY, f64::max);
    if lead > 2.0 * bound {
        assert_eq!(got, top, "{what}: the decode emits token {got}, the reference {top}, which leads by {lead:.5}");
    }
}

/// The SN50 with fp32 beside bf16 in every datapath that takes bf16: a chip whose numbers are not the question.
pub fn fp32_twin() -> SpatialTarget {
    let mut t = SpatialTarget::sn50_rdu();
    for c in &mut t.compute {
        if let ComputeCap::MatmulTile { precisions, .. } | ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } = c {
            if precisions.contains(&Precision::Bf16) {
                precisions.insert(Precision::Fp32);
            }
        }
    }
    t.precisions.insert(Precision::Fp32);
    t
}

/// The token after the prompt decoded with every layer on `target` and the embedding and head on the GPU target, its
/// carried state `handed`: the logits — `target`'s layer arithmetic alone.
pub fn decode_with_layers_on(m: &Model, target: &SpatialTarget, handed: &Handoff) -> Vec<f64> {
    let gpu = SpatialTarget::rtx3060();
    let mut given = decode_inputs(handed);
    let x = run(m, &authored(m, Cut::DecodeEmbed, &gpu), &|n| Some(n.to_string()), &given)["x"].clone();
    given.insert("x".into(), x);
    layers_on(m, target, 0..m.spec.layers.len(), &mut given);
    run(m, &authored(m, Cut::DecodeHead, &gpu), &|n| Some(n.to_string()), &given)["logits"].clone()
}

/// The bundle's program in `bundle/sub`, read off disk and certified on its own, its schedule the manifest's `digest`.
fn load(bundle: &Path, sub: &str, digest: &serde_json::Value) -> SpatialProgram {
    let text = std::fs::read_to_string(bundle.join(sub).join("neutral.json")).unwrap_or_else(|e| panic!("{sub}: {e}"));
    let mut p = parse_neutral(&text).unwrap_or_else(|e| panic!("{sub}/neutral.json parses: {e}"));
    p.certificate = None;
    let c = certify_choreography(&p).unwrap_or_else(|e| panic!("{sub} certifies on its own: {e:?}"));
    assert_eq!(serde_json::json!(format!("{:016x}", c.schedule_digest)), *digest, "{sub}: not the program the manifest records");
    p
}

/// A manifest entry list as the names a program's tensors bind to.
fn binding(entries: &serde_json::Value) -> impl Fn(&str) -> Option<String> {
    let entries = entries.as_array().unwrap().clone();
    move |name: &str| entries.iter().find(|e| e["program"] == name).and_then(|e| e["layer"].as_str().map(String::from))
}

/// Tokens `toks` decoded at once on the SN50 bundle `largo emit rdu --batch toks.len()` writes, sequence `b` after
/// `handed[b]`: each sequence's logits.
pub fn decode_batch_on_bundle(m: &Model, handed: &[Handoff], toks: &[u32]) -> Vec<Vec<f64>> {
    assert_eq!(handed.len(), toks.len(), "a token a sequence");
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
            m.path.to_str().unwrap(),
            "--ctx",
            &CTX.to_string(),
            "--batch",
            &toks.len().to_string(),
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "largo emit rdu --batch failed:\n{}", cli::stderr(&out));
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(bundle.join("manifest.json")).expect("manifest.json")).unwrap();
    // The program counts tokens and positions from 1.
    let mut given: BTreeMap<String, Vec<f64>> = BTreeMap::from([("toks".to_string(), toks.iter().map(|t| f64::from(t + 1)).collect())]);
    for (b, h) in handed.iter().enumerate() {
        given.insert(format!("pos_s{b}"), vec![(h.position + 1) as f64]);
        for (name, v) in &h.state {
            given.insert(format!("{name}_s{b}"), v.clone());
        }
    }
    let embed = load(&bundle, "embed", &manifest["embed"]["schedule_digest"]);
    let x = run(m, &embed, &binding(&manifest["embed"]["tensors"]), &given)["X"].clone();
    given.insert("X".into(), x);
    let kinds: Vec<SpatialProgram> =
        manifest["kinds"].as_array().unwrap().iter().map(|k| load(&bundle, k["dir"].as_str().unwrap(), &k["schedule_digest"])).collect();
    for b in manifest["layers"].as_array().unwrap() {
        let l = b["layer"].as_u64().unwrap();
        let out = run(m, &kinds[b["kind"].as_u64().unwrap() as usize], &binding(&b["tensors"]), &given);
        let name = format!("bxo{l}");
        given.insert(name.clone(), out[&name].clone());
    }
    let head = load(&bundle, "head", &manifest["head"]["schedule_digest"]);
    let out = run(m, &head, &binding(&manifest["head"]["tensors"]), &given);
    (0..toks.len()).map(|b| out[&format!("logits_s{b}")].clone()).collect()
}

/// The byte offset of `tensor`'s bytes in `m`'s checkpoint, and its GGUF type.
fn located(m: &Model, tensor: &str) -> (u64, u32) {
    let g = Gguf::open(&m.path).expect("the checkpoint");
    let t = g.info(tensor).unwrap_or_else(|| panic!("the checkpoint holds `{tensor}`"));
    (g.data_start() + t.offset, t.ggml_type)
}

fn overwrite(m: &Model, at: u64, bytes: &[u8]) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::OpenOptions::new().write(true).open(&m.path).expect("the checkpoint opens for writing");
    f.seek(SeekFrom::Start(at)).unwrap();
    f.write_all(bytes).unwrap();
}

/// `m`'s checkpoint with element `at` of the `f32` tensor `tensor` set to `v`, in place.
pub fn poison_f32(m: &Model, tensor: &str, at: usize, v: f32) {
    let (start, ty) = located(m, tensor);
    assert_eq!(ty, 0, "`{tensor}` is stored f32");
    overwrite(m, start + 4 * at as u64, &v.to_le_bytes());
}

/// `m`'s checkpoint with element `at` of the Q8_0 tensor `tensor` set to about `v` — its block's scale `v/127` (as
/// f16) and its level 127 — and the block's other elements to zero, in place.
pub fn poison_q8(m: &Model, tensor: &str, at: usize, v: f32) {
    let (start, ty) = located(m, tensor);
    assert_eq!(ty, 8, "`{tensor}` is stored Q8_0");
    let mut block = [0u8; 34];
    block[..2].copy_from_slice(&f16_bits(v / 127.0).to_le_bytes());
    block[2 + at % 32] = 127;
    overwrite(m, start + 34 * (at / 32) as u64, &block);
}

/// The IEEE half nearest a normal `f32` within half's range.
fn f16_bits(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    assert!((1..31).contains(&exp), "{x} is a normal half");
    let mant = b & 0x7f_ffff;
    let rounded = (mant + 0x1000) >> 13;
    let (exp, mant) = if rounded == 0x400 { (exp + 1, 0) } else { (exp, rounded) };
    sign | ((exp as u16) << 10) | mant as u16
}
