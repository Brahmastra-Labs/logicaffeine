//! The hand-off harness: a model written as a GGUF checkpoint; its prompt prefilled by the authored prefill program on a
//! chip; its state handed, through `spatial::handoff`'s contract, to a decode on another — the SN50 bundle `largo emit
//! rdu` writes, read back off disk, or any target's authored layer programs — and the decode's logits held to the f64
//! reference's continuation of the same prompt.
//!
//! A program runs as a host runs it: every weight read from the checkpoint through its binding (a Q8_0 weight as the
//! two planes its program names), a rotary table from the spec, every named input (tokens, position, hidden state,
//! handed state) from the map the caller gives, a carried state not handed at zero, and every compile-time constant at
//! its declared value.

use crate::cli;
use crate::tiny_models::write_checkpoint;
use logicaffeine_compile::model::bind::{bind, host_values, q8_planes, values, HostTable, Source};
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::open_checkpoint;
use logicaffeine_compile::model::program::{written, Cut};
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::reference::{argmax, Reference, WeightSource};
use logicaffeine_compile::model::Checkpoint;
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::spatial::certificate::certify_choreography;
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Inputs, Value};
use logicaffeine_compile::spatial::handoff::{contract, Handoff};
use logicaffeine_compile::spatial::ir::{SpatialProgram, TensorInit, TensorRole};
use logicaffeine_compile::spatial::lower::rdu::parse_neutral;
use logicaffeine_compile::spatial::passes::from_q8::{stored_part, Q8Part};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The cache every hand-off's programs are written over: the prompt and the token after it, with room to spare.
pub const CTX: usize = 8;
/// The prompt a chip prefills.
pub const PROMPT: [u32; 3] = [3, 7, 1];
/// The token decoded after the prompt.
pub const NEXT: u32 = 5;
/// The bf16 datapath's bound on a logit, relative to the largest reference logit at a width of 64 (`rdu_bringup_lock`'s).
pub const BF16_LOGIT_REL: f64 = 0.02;

/// A model written as a checkpoint, and its spec as the checkpoint reads back.
pub struct Model {
    _dir: Option<tempfile::TempDir>,
    pub path: PathBuf,
    pub spec: ModelSpec,
}

impl Model {
    pub fn write(spec: ModelSpec, what: &str) -> Model {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("{}.gguf", what.replace(' ', "-")));
        write_checkpoint(&spec, &path);
        let spec = ModelSpec::from_gguf(&Gguf::open(&path).expect("the checkpoint")).expect("the checkpoint reads");
        Model { _dir: Some(dir), path, spec }
    }

    /// The checkpoint at `path` (a GGUF or a Hugging Face directory), opened as `largo` opens it: through
    /// `model::open_checkpoint`, in the layout the spec's arms read.
    pub fn open(path: &Path) -> Model {
        let spec = ModelSpec::from_source(&*open_checkpoint(path).expect("the checkpoint")).expect("the checkpoint reads");
        Model { _dir: None, path: path.to_path_buf(), spec }
    }

    fn source(&self) -> Opened {
        Opened(open_checkpoint(&self.path).expect("the checkpoint"))
    }
}

/// A checkpoint `open_checkpoint` opened, as the weight source a program's bindings and the reference read.
pub struct Opened(pub Box<dyn Checkpoint>);

impl WeightSource for Opened {
    fn describe(&self, name: &str) -> std::io::Result<(GgmlType, Vec<u64>)> {
        self.0.describe(name)
    }
    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>> {
        self.0.row_bytes(name, first, count)
    }
    fn has(&self, name: &str) -> bool {
        WeightSource::has(&*self.0, name)
    }
}

/// Run `p` as a host does, its tensors named through `bound` (a bundle program's names bind to a layer's); `given` holds
/// every named input. Returns every tensor the run leaves, by bound name.
pub fn run(m: &Model, p: &SpatialProgram, bound: &dyn Fn(&str) -> Option<String>, given: &BTreeMap<String, Vec<f64>>) -> BTreeMap<String, Vec<f64>> {
    run_at(m, p, CTX, bound, given)
}

/// [`run`] for a program written over a cache of `ctx` positions: its host tables are `ctx` rows.
pub fn run_at(m: &Model, p: &SpatialProgram, ctx: usize, bound: &dyn Fn(&str) -> Option<String>, given: &BTreeMap<String, Vec<f64>>) -> BTreeMap<String, Vec<f64>> {
    let mut inputs = Inputs::default();
    for t in &p.tensors {
        if t.role == TensorRole::Output {
            continue;
        }
        let Some(name) = bound(&t.name) else { continue };
        let n = t.elems.rows * t.elems.cols;
        let v = if let Some(v) = given.get(&name) {
            v.clone()
        } else if HostTable::named(&name).is_some() {
            host_values(&m.spec, &name, ctx, &mut m.source()).unwrap_or_else(|e| panic!("`{name}`: {e}")).expect("a host table")
        } else if let Some((w, part)) = stored_part(&name) {
            let Ok(Source::Weight(b)) = bind(&m.spec, w) else { panic!("`{w}` is a weight") };
            let planes = q8_planes(&b, &mut m.source()).unwrap_or_else(|e| panic!("`{name}`: {e}"));
            match part {
                Q8Part::Values => planes.values,
                Q8Part::Scales => planes.scales,
            }
        } else {
            match bind(&m.spec, &name) {
                Ok(Source::Weight(b)) => values(&b, &mut m.source()).unwrap_or_else(|e| panic!("`{name}`: {e}")),
                // A compile-time constant keeps its declared value.
                _ if !matches!(t.init, TensorInit::External | TensorInit::Zero) => continue,
                _ => vec![0.0; n],
            }
        };
        assert_eq!(v.len(), n, "`{name}` fills {} of {n}", v.len());
        inputs.tensors.insert(t.id, Value::Float(v));
    }
    let r = eval_choreography_every_tensor(p, &inputs).expect("the program evaluates");
    p.tensors
        .iter()
        .filter_map(|t| {
            let name = bound(&t.name)?;
            let v = match r.tensors.get(&t.id)? {
                Value::Float(v) => v.clone(),
                Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
            };
            Some((name, v))
        })
        .collect()
}

/// The authored program of `cut`, lowered whole for `target`.
pub fn authored(m: &Model, cut: Cut, target: &SpatialTarget) -> SpatialProgram {
    let ((src, _, _), _) = written(&m.spec, cut);
    compile_program_to_spatial(&src, target)
        .expect("the program walks")
        .into_whole()
        .unwrap_or_else(|why| panic!("{cut:?} lowers whole for {}: {why}", target.name))
}

/// The prompt prefilled on `target` by the authored prefill program, and the state it leaves, taken by the contract.
pub fn prefill_on(m: &Model, target: &SpatialTarget, prompt: &[u32]) -> Handoff {
    let p = authored(m, Cut::Prefill { rows: prompt.len(), ctx: CTX }, target);
    let given = BTreeMap::from([
        // The program counts tokens and positions from 1.
        ("toks".to_string(), prompt.iter().map(|t| f64::from(t + 1)).collect()),
        ("pos_s0".to_string(), vec![1.0]),
    ]);
    let tensors = run(m, &p, &|n| Some(n.to_string()), &given);
    contract(&m.spec, CTX).expect("the contract").take(prompt.len(), &tensors).unwrap_or_else(|e| panic!("the prefill on {}: {e}", target.name))
}

/// `largo emit rdu --chip sn50 --model <checkpoint> --ctx CTX` into a directory of its own: the bundle and its manifest.
pub fn emit_bundle(m: &Model) -> (tempfile::TempDir, serde_json::Value) {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("bundle");
    let out = cli::largo_in(
        dir.path(),
        &["emit", "rdu", "--chip", "sn50", "--model", m.path.to_str().unwrap(), "--ctx", &CTX.to_string(), "-o", bundle.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "largo emit rdu failed:\n{}", cli::stderr(&out));
    let manifest = serde_json::from_str(&std::fs::read_to_string(bundle.join("manifest.json")).expect("manifest.json")).unwrap();
    (dir, manifest)
}

/// The token after the prompt decoded on the SN50 bundle, its carried state `handed` (or empty, for the prompt's first
/// token): the head's logits.
pub fn decode_on_bundle(m: &Model, handed: Option<&Handoff>) -> Vec<f64> {
    let (dir, manifest) = emit_bundle(m);
    let bundle = dir.path().join("bundle");
    let load = |sub: &str, digest: &serde_json::Value| -> SpatialProgram {
        let text = std::fs::read_to_string(bundle.join(sub).join("neutral.json")).unwrap_or_else(|e| panic!("{sub}: {e}"));
        let mut p = parse_neutral(&text).unwrap_or_else(|e| panic!("{sub}/neutral.json parses: {e}"));
        p.certificate = None;
        let c = certify_choreography(&p).unwrap_or_else(|e| panic!("{sub} certifies on its own: {e:?}"));
        assert_eq!(serde_json::json!(format!("{:016x}", c.schedule_digest)), *digest, "{sub}: not the program the manifest records");
        p
    };
    let position = handed.map_or(0, |h| h.position);
    let mut given: BTreeMap<String, Vec<f64>> = handed.map(|h| h.state.clone()).unwrap_or_default();
    given.insert("tok".into(), vec![f64::from(NEXT + 1)]);
    given.insert("pos".into(), vec![(position + 1) as f64]);
    let binding = |entries: &serde_json::Value| {
        let entries = entries.as_array().unwrap().clone();
        move |name: &str| entries.iter().find(|e| e["program"] == name).and_then(|e| e["layer"].as_str().map(String::from))
    };
    let embed = load("embed", &manifest["embed"]["schedule_digest"]);
    let x = run(m, &embed, &binding(&manifest["embed"]["tensors"]), &given)["x"].clone();
    given.insert("x".into(), x);
    let kinds: Vec<SpatialProgram> = manifest["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| load(k["dir"].as_str().unwrap(), &k["schedule_digest"]))
        .collect();
    for b in manifest["layers"].as_array().unwrap() {
        let l = b["layer"].as_u64().unwrap();
        let out = run(m, &kinds[b["kind"].as_u64().unwrap() as usize], &binding(&b["tensors"]), &given);
        let name = format!("xo{l}");
        given.insert(name.clone(), out[&name].clone());
    }
    let head = load("head", &manifest["head"]["schedule_digest"]);
    run(m, &head, &binding(&manifest["head"]["tensors"]), &given)["logits"].clone()
}

/// The decode step's layers `layers` on `target`, their carried state from `given`, the hidden state named by position.
pub fn layers_on(m: &Model, target: &SpatialTarget, layers: std::ops::Range<usize>, given: &mut BTreeMap<String, Vec<f64>>) {
    for l in layers {
        let p = authored(m, Cut::DecodeLayer { layer: l, ctx: CTX }, target);
        let out = run(m, &p, &|n| Some(n.to_string()), given);
        let name = format!("xo{l}");
        given.insert(name.clone(), out[&name].clone());
    }
}

/// The inputs of a decode after `handed`: its state, the token and the position.
pub fn decode_inputs(handed: &Handoff) -> BTreeMap<String, Vec<f64>> {
    let mut given = handed.state.clone();
    given.insert("tok".into(), vec![f64::from(NEXT + 1)]);
    given.insert("pos".into(), vec![(handed.position + 1) as f64]);
    given
}

/// The token after the prompt decoded on `target` by the authored embedding, layer and head programs: the logits.
pub fn decode_on_layers(m: &Model, target: &SpatialTarget, handed: &Handoff) -> Vec<f64> {
    split_decode(m, handed, m.spec.layers.len(), (target, target))
}

/// The token after the prompt decoded with layers below `cut` (and the embedding) on `chips.0` and the rest (and the
/// head) on `chips.1`, the hidden state handed on at the cut: the logits.
pub fn split_decode(m: &Model, handed: &Handoff, cut: usize, chips: (&SpatialTarget, &SpatialTarget)) -> Vec<f64> {
    let mut given = decode_inputs(handed);
    let embed = authored(m, Cut::DecodeEmbed, chips.0);
    let x = run(m, &embed, &|n| Some(n.to_string()), &given)["x"].clone();
    given.insert("x".into(), x);
    layers_on(m, chips.0, 0..cut, &mut given);
    layers_on(m, chips.1, cut..m.spec.layers.len(), &mut given);
    let head = authored(m, Cut::DecodeHead, chips.1);
    run(m, &head, &|n| Some(n.to_string()), &given)["logits"].clone()
}

/// `logits` held to the f64 reference's after the prompt and [`NEXT`]: every logit within the bf16 datapath's bound
/// (scaled to the width), and the reference's token wherever it leads by more than twice that.
pub fn close_to_reference(m: &Model, logits: &[f64], what: &str) {
    let mut reference = Reference::new(m.spec.clone(), m.source()).expect("the reference reads the checkpoint");
    reference.step(&PROMPT).expect("the reference prefills");
    let want = reference.step(&[NEXT]).expect("the reference steps");
    assert_eq!(logits.len(), want.len(), "{what}: the logits cover the vocabulary");
    let scale = want.iter().fold(0.0f64, |acc, v| acc.max(v.abs()));
    let bound = BF16_LOGIT_REL * (m.spec.d as f64 / 64.0).sqrt().max(1.0) * scale;
    let worst = logits.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(worst <= bound, "{what}: a logit is {worst:.5} from the reference's, past the datapath's {bound:.5}");
    let (top, got) = (argmax(&want) as usize, argmax(logits) as usize);
    let lead = want[top] - want.iter().enumerate().filter(|(i, _)| *i != top).map(|(_, v)| *v).fold(f64::NEG_INFINITY, f64::max);
    if lead > 2.0 * bound {
        assert_eq!(got, top, "{what}: the decode emits token {got}, the reference {top}, which leads by {lead:.5}");
    }
}
