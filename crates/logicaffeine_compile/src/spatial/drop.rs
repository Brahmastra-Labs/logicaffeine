//! A self-checking artifact drop: an SN50 decode bundle that proves itself on a machine with only `largo`.
//!
//! [`write_drop`] lowers a checkpoint's decode step to the SN50 bundle `largo emit rdu --model --ctx` writes, carries the
//! checkpoint whose weights the manifest binds (or, with `fetch`, its digest only), records the goldens of its prompts —
//! the tokens [`decode_bundle`] decodes on the spatial evaluator from an empty state, the digest of their last logits, and
//! the f64 reference's tokens beside them — and writes `drop.json`, the sha256 of every file.
//!
//! [`check_drop`] is the runner's side. It refuses, naming the file, a drop with a file missing, extra or changed; it
//! rechecks every program's certificate against the schedule digest the manifest records, so a program changed and
//! digested again is refused by its own certificate; and it decodes every golden again and refuses any token or logit
//! bit that differs.
//!
//! [`decode_bundle`] is the bundle's host: it binds every tensor a program reads by the manifest's names — a runtime
//! input, the carried state of the step before, a host table, or a weight read through the binding the program was
//! authored with — and runs the embedding, each layer's kind and the head in turn, a decode step a token.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::model::bind::{bind, host_values, q8_planes, values, HostTable, Source};
use crate::model::reference::{argmax, Reference, WeightSource};
use crate::model::spec::ModelSpec;
use crate::model::{open_checkpoint, Checkpoint};
use crate::spatial::certificate::certify_choreography;
use crate::spatial::eval::{eval_choreography_every_tensor, Inputs, Value};
use crate::spatial::ir::{SpatialProgram, TensorInit, TensorRole};
use crate::spatial::lower::rdu::parse_neutral;
use crate::spatial::passes::from_q8::{stored_part, Q8Part};

/// The format a drop's `drop.json` declares.
pub const DROP_FORMAT: &str = "logos.rdu.drop/1";

/// Why a drop is refused: what failed, naming the file or the golden it failed on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropRefusal(pub String);

impl std::fmt::Display for DropRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a drop's check verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    pub files: usize,
    pub certificates: usize,
    pub goldens: usize,
}

/// A checkpoint as the reference and the host read it.
struct Opened(Box<dyn Checkpoint>);

impl WeightSource for Opened {
    fn describe(&self, name: &str) -> std::io::Result<(crate::model::quant::GgmlType, Vec<u64>)> {
        self.0.describe(name)
    }
    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>> {
        self.0.row_bytes(name, first, count)
    }
    fn has(&self, name: &str) -> bool {
        WeightSource::has(&*self.0, name)
    }
}

fn refuse(why: impl Into<String>) -> DropRefusal {
    DropRefusal(why.into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_file(path: &Path) -> Result<String, DropRefusal> {
    let bytes = std::fs::read(path).map_err(|e| refuse(format!("{}: {e}", path.display())))?;
    Ok(hex(&crate::crypto_accel::sha256(&bytes)))
}

/// The sha256 of every file under `path` (a file, or a directory walked in order), by name relative to `root`.
fn digests(root: &Path, path: &Path, out: &mut BTreeMap<String, String>) -> Result<(), DropRefusal> {
    if path.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
            .map_err(|e| refuse(format!("{}: {e}", path.display())))?
            .map(|e| e.map(|e| e.path()).map_err(|e| refuse(format!("{}: {e}", path.display()))))
            .collect::<Result<_, _>>()?;
        entries.sort();
        for p in entries {
            digests(root, &p, out)?;
        }
    } else {
        let name = path.strip_prefix(root).expect("under the root").to_string_lossy().replace('\\', "/");
        out.insert(name, sha256_file(path)?);
    }
    Ok(())
}

/// The sha256 of a checkpoint: its file's, or the digests of a directory's files, digested in order.
fn checkpoint_digest(path: &Path) -> Result<String, DropRefusal> {
    if !path.is_dir() {
        return sha256_file(path);
    }
    let mut files = BTreeMap::new();
    digests(path, path, &mut files)?;
    let joined: String = files.iter().map(|(n, d)| format!("{n}\t{d}\n")).collect();
    Ok(hex(&crate::crypto_accel::sha256(joined.as_bytes())))
}

fn copy_into(from: &Path, to: &Path) -> Result<(), DropRefusal> {
    if from.is_dir() {
        std::fs::create_dir_all(to).map_err(|e| refuse(format!("{}: {e}", to.display())))?;
        for e in std::fs::read_dir(from).map_err(|e| refuse(format!("{}: {e}", from.display())))? {
            let p = e.map_err(|e| refuse(e.to_string()))?.path();
            copy_into(&p, &to.join(p.file_name().expect("a named entry")))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ()).map_err(|e| refuse(format!("{} to {}: {e}", from.display(), to.display())))
    }
}

/// A bundle's programs loaded and rechecked: each `neutral.json` parsed, its certificate made again and its schedule
/// digest held to the one the manifest records. `bundle` is the bundle directory, `rel` how the refusals name it.
struct Loaded {
    embed: SpatialProgram,
    kinds: Vec<SpatialProgram>,
    head: SpatialProgram,
}

fn load(bundle: &Path, rel: &str, manifest: &serde_json::Value) -> Result<Loaded, DropRefusal> {
    let one = |dir: &str, digest: &serde_json::Value| -> Result<SpatialProgram, DropRefusal> {
        let file = format!("{rel}{dir}/neutral.json");
        let text = std::fs::read_to_string(bundle.join(dir).join("neutral.json")).map_err(|e| refuse(format!("{file}: {e}")))?;
        let mut p = parse_neutral(&text)
            .map_err(|e| refuse(format!("{file}: the program does not parse, so its certificate cannot be checked: {e}")))?;
        p.certificate = None;
        let c = certify_choreography(&p).map_err(|e| refuse(format!("{file}: its certificate cannot be made again: {e:?}")))?;
        let want = digest.as_str().ok_or_else(|| refuse(format!("{rel}manifest.json: no schedule digest for `{dir}`")))?;
        let got = format!("{:016x}", c.schedule_digest);
        if got != want {
            return Err(refuse(format!(
                "{file}: its certificate's schedule digest is {got}, the manifest's {want}: the program is not the one certified"
            )));
        }
        Ok(p)
    };
    let embed = one("embed", &manifest["embed"]["schedule_digest"])?;
    let kinds = manifest["kinds"]
        .as_array()
        .ok_or_else(|| refuse(format!("{rel}manifest.json: no layer kinds")))?
        .iter()
        .map(|k| one(k["dir"].as_str().unwrap_or_default(), &k["schedule_digest"]))
        .collect::<Result<Vec<_>, _>>()?;
    let head = one("head", &manifest["head"]["schedule_digest"])?;
    Ok(Loaded { embed, kinds, head })
}

/// A program's bindings: its tensor names to the names the manifest gives them for this use of it.
fn binding(entries: &serde_json::Value) -> impl Fn(&str) -> Option<String> {
    let entries = entries.as_array().cloned().unwrap_or_default();
    move |name: &str| entries.iter().find(|e| e["program"] == name).and_then(|e| e["layer"].as_str().map(String::from))
}

/// Run `p` as a host does: every tensor it reads by its bound name — `given` (runtime inputs and carried state), a host
/// table, a weight read through its binding — or its declared value; every tensor it leaves, by bound name.
fn run(
    spec: &ModelSpec,
    ctx: usize,
    source: &mut dyn WeightSource,
    p: &SpatialProgram,
    bound: &dyn Fn(&str) -> Option<String>,
    given: &BTreeMap<String, Vec<f64>>,
) -> Result<BTreeMap<String, Vec<f64>>, DropRefusal> {
    let mut inputs = Inputs::default();
    for t in &p.tensors {
        if t.role == TensorRole::Output {
            continue;
        }
        let Some(name) = bound(&t.name) else { continue };
        let n = t.elems.rows * t.elems.cols;
        let io = |e: std::io::Error| refuse(format!("`{name}`: {e}"));
        let v = if let Some(v) = given.get(&name) {
            v.clone()
        } else if HostTable::named(&name).is_some() {
            host_values(spec, &name, ctx, source).map_err(io)?.ok_or_else(|| refuse(format!("`{name}`: no host table")))?
        } else if let Some((w, part)) = stored_part(&name) {
            let Ok(Source::Weight(b)) = bind(spec, w) else { return Err(refuse(format!("`{w}` binds no weight"))) };
            let planes = q8_planes(&b, source).map_err(io)?;
            match part {
                Q8Part::Values => planes.values,
                Q8Part::Scales => planes.scales,
            }
        } else {
            match bind(spec, &name) {
                Ok(Source::Weight(b)) => values(&b, source).map_err(io)?,
                // A compile-time constant keeps its declared value; carried state starts empty.
                _ if !matches!(t.init, TensorInit::External | TensorInit::Zero) => continue,
                _ => vec![0.0; n],
            }
        };
        if v.len() != n {
            return Err(refuse(format!("`{name}` fills {} of its {n} values", v.len())));
        }
        inputs.tensors.insert(t.id, Value::Float(v));
    }
    let r = eval_choreography_every_tensor(p, &inputs).map_err(|e| refuse(format!("the evaluator: {e:?}")))?;
    Ok(p.tensors
        .iter()
        .filter_map(|t| {
            let name = bound(&t.name)?;
            let v = match r.tensors.get(&t.id)? {
                Value::Float(v) => v.clone(),
                Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
            };
            Some((name, v))
        })
        .collect())
}

/// What a bundle decodes: the tokens after the prompt and the logits of the last step.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub tokens: Vec<u32>,
    pub logits: Vec<f64>,
}

impl Decoded {
    /// The sha256 of the last logits' f64 bits, little-endian.
    pub fn logits_sha256(&self) -> String {
        let bytes: Vec<u8> = self.logits.iter().flat_map(|x| x.to_bits().to_le_bytes()).collect();
        hex(&crate::crypto_accel::sha256(&bytes))
    }
}

/// Decode `prompt` and then `tokens` greedy tokens on the bundle at `bundle` with the spatial evaluator, from an empty
/// state: a decode step a token, each step's carried state handed to the next.
fn decode_loaded(
    spec: &ModelSpec,
    manifest: &serde_json::Value,
    loaded: &Loaded,
    source: &mut dyn WeightSource,
    prompt: &[u32],
    tokens: usize,
) -> Result<Decoded, DropRefusal> {
    let ctx = manifest["ctx"].as_u64().ok_or_else(|| refuse("manifest.json: no ctx: not a decode bundle"))? as usize;
    if prompt.is_empty() {
        return Err(refuse("an empty prompt: a decode step reads a token"));
    }
    if prompt.len() + tokens > ctx {
        return Err(refuse(format!("{} prompt tokens and {tokens} more pass the {ctx}-position cache", prompt.len())));
    }
    let carried: BTreeSet<String> = manifest["layers"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|b| b["tensors"].as_array().cloned().unwrap_or_default())
        .filter(|e| e["role"] == "state")
        .filter_map(|e| e["layer"].as_str().map(String::from))
        .collect();
    let mut state: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let (mut out, mut logits, mut next) = (Vec::new(), Vec::new(), prompt[0]);
    for pos in 0..prompt.len() + tokens {
        if pos < prompt.len() {
            next = prompt[pos];
        }
        let mut given = state.clone();
        given.insert("tok".into(), vec![f64::from(next + 1)]);
        given.insert("pos".into(), vec![(pos + 1) as f64]);
        let x = run(spec, ctx, source, &loaded.embed, &binding(&manifest["embed"]["tensors"]), &given)?;
        given.insert("x".into(), x.get("x").cloned().ok_or_else(|| refuse("the embedding leaves no `x`"))?);
        for b in manifest["layers"].as_array().into_iter().flatten() {
            let l = b["layer"].as_u64().unwrap_or_default();
            let kind = b["kind"].as_u64().unwrap_or_default() as usize;
            let p = loaded.kinds.get(kind).ok_or_else(|| refuse(format!("manifest.json: layer {l} names kind {kind}")))?;
            let left = run(spec, ctx, source, p, &binding(&b["tensors"]), &given)?;
            for (name, v) in left {
                if carried.contains(&name) || name == format!("xo{l}") {
                    given.insert(name, v);
                }
            }
        }
        let head = run(spec, ctx, source, &loaded.head, &binding(&manifest["head"]["tensors"]), &given)?;
        logits = head.get("logits").cloned().ok_or_else(|| refuse("the head leaves no `logits`"))?;
        state = given.into_iter().filter(|(n, _)| carried.contains(n)).collect();
        if pos + 1 >= prompt.len() {
            next = argmax(&logits);
            if out.len() < tokens {
                out.push(next);
            }
        }
    }
    Ok(Decoded { tokens: out, logits })
}

fn read_manifest(bundle: &Path, rel: &str) -> Result<serde_json::Value, DropRefusal> {
    let text = std::fs::read_to_string(bundle.join("manifest.json")).map_err(|e| refuse(format!("{rel}manifest.json: {e}")))?;
    serde_json::from_str(&text).map_err(|e| refuse(format!("{rel}manifest.json: {e}")))
}

/// Decode `prompt` and `tokens` more on the decode bundle at `bundle`, its weights from `checkpoint`, every program's
/// certificate rechecked first.
pub fn decode_bundle(bundle: &Path, checkpoint: &Path, prompt: &[u32], tokens: usize) -> Result<Decoded, DropRefusal> {
    let manifest = read_manifest(bundle, "")?;
    let loaded = load(bundle, "", &manifest)?;
    let mut ckpt = open_checkpoint(checkpoint).map_err(|e| refuse(format!("{}: {e}", checkpoint.display())))?;
    let spec = ModelSpec::from_source(ckpt.as_mut()).map_err(|e| refuse(format!("{}: {e}", checkpoint.display())))?;
    decode_loaded(&spec, &manifest, &loaded, ckpt.as_mut(), prompt, tokens)
}

/// Write a drop of `checkpoint` at `ctx` positions into `out`: the SN50 decode bundle under `bundle/`, the checkpoint
/// under `bundle/weights/` (or its digest only, with `fetch`), the goldens of `prompts` with `tokens` tokens each, and
/// `drop.json`.
pub fn write_drop(checkpoint: &Path, ctx: usize, prompts: &[Vec<u32>], tokens: usize, fetch: bool, out: &Path) -> Result<serde_json::Value, DropRefusal> {
    use crate::compile::{compile_model_to_rdu_streamed, RduModel, StepShape};
    use crate::spatial::rdu_chip::RduChip;
    let bundle = out.join("bundle");
    std::fs::create_dir_all(&bundle).map_err(|e| refuse(format!("{}: {e}", bundle.display())))?;
    let name = checkpoint.file_name().map(|s| s.to_string_lossy().into_owned()).ok_or_else(|| refuse("a checkpoint with no name"))?;
    let stem = checkpoint.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.clone());
    let mut ckpt = open_checkpoint(checkpoint).map_err(|e| refuse(format!("{}: {e}", checkpoint.display())))?;
    let model = RduModel::from_checkpoint(&stem, ckpt.as_mut()).map_err(refuse)?;
    let write = |path: PathBuf, bytes: &[u8]| -> Result<(), DropRefusal> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| refuse(format!("{}: {e}", dir.display())))?;
        }
        std::fs::write(&path, bytes).map_err(|e| refuse(format!("{}: {e}", path.display())))
    };
    let mut programs: Vec<(PathBuf, String)> = Vec::new();
    let rdu = compile_model_to_rdu_streamed(&model, RduChip::Sn50, "weights", Some(StepShape { ctx, batch: 1, tp: 1, prefill: None }), |f| {
        let dir = bundle.join(&f.dir);
        programs.extend(f.files().map(|(name, text)| (dir.join(name), text.to_string())));
        Ok(())
    })
    .map_err(refuse)?;
    for (path, text) in programs {
        write(path, text.as_bytes())?;
    }
    for (path, bytes) in &rdu.constants {
        write(bundle.join(path), bytes)?;
    }
    write(bundle.join("manifest.json"), rdu.manifest.as_bytes())?;
    write(bundle.join("program.lg"), rdu.program.as_bytes())?;
    let weights_digest = checkpoint_digest(checkpoint)?;
    let weights = if fetch {
        serde_json::json!({ "fetched": checkpoint.display().to_string(), "sha256": weights_digest })
    } else {
        let carried = format!("bundle/weights/{name}");
        copy_into(checkpoint, &out.join(&carried))?;
        serde_json::json!({ "carried": carried, "sha256": weights_digest })
    };
    let manifest = read_manifest(&bundle, "bundle/")?;
    let loaded = load(&bundle, "bundle/", &manifest)?;
    let spec = &model.spec;
    let mut goldens = Vec::new();
    for prompt in prompts {
        let d = decode_loaded(spec, &manifest, &loaded, ckpt.as_mut(), prompt, tokens)?;
        let reference = open_checkpoint(checkpoint)
            .map_err(|e| refuse(format!("{}: {e}", checkpoint.display())))
            .and_then(|c| Reference::new(spec.clone(), Opened(c)).map_err(|e| refuse(format!("the reference: {e}"))))?
            .generate(prompt, tokens)
            .map_err(|e| refuse(format!("the reference: {e}")))?;
        goldens.push(serde_json::json!({
            "prompt": prompt, "tokens": d.tokens, "logits_sha256": d.logits_sha256(), "reference_tokens": reference,
        }));
    }
    let mut files = BTreeMap::new();
    digests(out, out, &mut files)?;
    files.remove("drop.json");
    let drop = serde_json::json!({
        "format": DROP_FORMAT,
        "model": spec.arch,
        "chip": "sn50",
        "ctx": ctx,
        "runner": "largo check-drop",
        "weights": weights,
        "goldens": goldens,
        "files": files,
    });
    write(out.join("drop.json"), format!("{}\n", serde_json::to_string_pretty(&drop).expect("JSON")).as_bytes())?;
    Ok(drop)
}

/// Check the drop at `dir`: every file's digest, every program's certificate, every golden decoded again bit for bit.
/// A drop that fetches its weights reads them from `weights`, which must have the digest it records.
pub fn check_drop(dir: &Path, weights: Option<&Path>) -> Result<Verified, DropRefusal> {
    let text = std::fs::read_to_string(dir.join("drop.json")).map_err(|e| refuse(format!("drop.json: {e}")))?;
    let drop: serde_json::Value = serde_json::from_str(&text).map_err(|e| refuse(format!("drop.json: {e}")))?;
    if drop["format"] != DROP_FORMAT {
        return Err(refuse(format!("drop.json: format {}, where this runner reads {DROP_FORMAT}", drop["format"])));
    }
    let want: BTreeMap<String, String> = drop["files"]
        .as_object()
        .ok_or_else(|| refuse("drop.json: no files"))?
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
        .collect();
    let mut have = BTreeMap::new();
    digests(dir, dir, &mut have)?;
    have.remove("drop.json");
    for name in want.keys() {
        if !have.contains_key(name) {
            return Err(refuse(format!("{name}: missing from the drop")));
        }
    }
    for (name, digest) in &have {
        match want.get(name) {
            None => return Err(refuse(format!("{name}: not part of the drop"))),
            Some(w) if w != digest => return Err(refuse(format!("{name}: its sha256 is {digest}, the drop's {w}"))),
            Some(_) => {}
        }
    }
    let wanted = drop["weights"]["sha256"].as_str().ok_or_else(|| refuse("drop.json: no weights digest"))?;
    let checkpoint = match (drop["weights"]["carried"].as_str(), weights) {
        (Some(carried), _) => dir.join(carried),
        (None, Some(path)) => {
            let got = checkpoint_digest(path)?;
            if got != wanted {
                return Err(refuse(format!("{}: its sha256 is {got}, the drop's weights' {wanted}", path.display())));
            }
            path.to_path_buf()
        }
        (None, None) => {
            return Err(refuse(format!(
                "the drop fetches its weights ({}, sha256 {wanted}): pass them with --weights",
                drop["weights"]["fetched"].as_str().unwrap_or("a checkpoint")
            )))
        }
    };
    let bundle = dir.join("bundle");
    let manifest = read_manifest(&bundle, "bundle/")?;
    let loaded = load(&bundle, "bundle/", &manifest)?;
    let mut ckpt = open_checkpoint(&checkpoint).map_err(|e| refuse(format!("{}: {e}", checkpoint.display())))?;
    let spec = ModelSpec::from_source(ckpt.as_mut()).map_err(|e| refuse(format!("{}: {e}", checkpoint.display())))?;
    let goldens = drop["goldens"].as_array().cloned().unwrap_or_default();
    for (i, g) in goldens.iter().enumerate() {
        let prompt: Vec<u32> = g["prompt"].as_array().into_iter().flatten().filter_map(|t| t.as_u64()).map(|t| t as u32).collect();
        let tokens: Vec<u32> = g["tokens"].as_array().into_iter().flatten().filter_map(|t| t.as_u64()).map(|t| t as u32).collect();
        let d = decode_loaded(&spec, &manifest, &loaded, ckpt.as_mut(), &prompt, tokens.len())?;
        if d.tokens != tokens {
            return Err(refuse(format!("golden {i}: the bundle decodes {:?}, the drop recorded {tokens:?}", d.tokens)));
        }
        let digest = d.logits_sha256();
        if g["logits_sha256"] != digest.as_str() {
            return Err(refuse(format!("golden {i}: the last logits' sha256 is {digest}, the drop's {}", g["logits_sha256"])));
        }
    }
    Ok(Verified { files: have.len(), certificates: loaded.kinds.len() + 2, goldens: goldens.len() })
}
