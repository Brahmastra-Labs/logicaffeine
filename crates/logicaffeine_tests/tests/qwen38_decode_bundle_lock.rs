//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! QWEN3.8 AS AN SN50 DECODE DEPLOYMENT: ONE PROGRAM PER LAYER KIND, A KV CACHE PER ATTENTION LAYER.
//!
//! The streamed model bundle lowers the AUTHORED forward: its attention reads only the current token's key and
//! value. That is a forward, not a deployment — serving Qwen3.8 is decode steps against a KV cache. The decode
//! bundle is the deployment: an EMBEDDING program (`x = row tok of Emb`), one program per layer kind in which
//! every attention layer appends this token's key and value to its cache as row `pos` and attends over the first
//! `pos` rows, and a HEAD ending in the arg-max. `tok` and `pos` are runtime inputs; the caches, the Gated-DeltaNet
//! memories and the conv rings are the carried state.
//!
//! Pinned: executed from its kind programs and manifest, one decode step emits the token the VM's whole decode
//! step emits — exactly — and leaves every attention layer's cache row `pos` holding what the VM's does.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test qwen38_decode_bundle_lock`
//!
//! ★ MUTATION PROBE (run when this landed): make `rebind` keep the representative layer's index — every layer then
//! runs its kind's representative tensors and the step never reaches `xo1`. The executor binds the model's
//! DECLARED tensors by name and leaves the compiler's own (scalers, block tables) as lowered — binding every
//! `Param` from the kind program's init ran every layer on its representative's weights, which is how this file's
//! first draft read layer 3's weights at layer 7.
//!
//! ★ MUTATION PROBE: have `streamed::lower_certified` drop the front end's refusal for a fixed sentence
//! (what it said before this lock: "the layer did not lower as a whole program", for the head as for a layer) —
//! `a_program_that_does_not_lower_is_named_with_its_refusal` goes red.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen38::{
    gen_qwen38_decode_embed_lg, gen_qwen38_decode_head_lg, gen_qwen38_decode_layer_lg,
    gen_qwen38_decode_step_lg, Qwen38Cfg,
};
use logicaffeine_compile::spatial::streamed::Storage;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Outputs, Value};
use logicaffeine_compile::spatial::ir::{SpatialProgram, TensorRole};
use logicaffeine_compile::spatial::streamed::{lower_decode_streamed, StreamedModel};
use logicaffeine_compile::spatial::rdu_chip::RduChip;

const CTX: usize = 6;
const TOK: usize = 5;
const POS: usize = 3;

fn fill_value(var: &str, i: usize) -> f64 {
    let salt = var.bytes().map(u64::from).sum::<u64>();
    let h = (i as u64)
        .wrapping_mul(2_654_435_761)
        .wrapping_add(salt.wrapping_mul(40503));
    ((h % 97) as f64 / 97.0 - 0.5) * 0.4
}

/// Fill every declared Real tensor from its NAME, so the whole step and each standalone program see the same values.
fn with_fills(src: &str) -> String {
    let mut out = String::new();
    for line in src.lines() {
        out.push_str(line);
        out.push('\n');
        let Some(rest) = line.strip_prefix("Let ") else {
            continue;
        };
        let Some((var, tail)) = rest.split_once(" be a ") else {
            continue;
        };
        let Some(dims) = tail.strip_suffix(" tensor of Real.") else {
            continue;
        };
        let (r, c) = dims.split_once(" by ").expect("R by C");
        let n = r.parse::<usize>().unwrap() * c.parse::<usize>().unwrap();
        for i in 0..n {
            out.push_str(&format!(
                "Set item {} of {var} to {:?}.\n",
                i + 1,
                fill_value(var, i)
            ));
        }
    }
    out
}

fn floats(v: &Value) -> Vec<f64> {
    match v {
        Value::Float(f) => f.clone(),
        Value::Int(i) => i.iter().map(|x| *x as f64).collect(),
    }
}

fn cfg() -> Qwen38Cfg {
    Qwen38Cfg {
        layers: 8,
        ..Qwen38Cfg::tiny()
    }
}

fn model() -> StreamedModel {
    lower_decode_streamed(&(&cfg()).spec(), Storage::AllQ8,
        CTX,
        &RduChip::Sn50.facts().spatial_target(),
        with_fills,
    )
    .expect("the decode bundle lowers")
}

/// The tensors a generated program DECLARES — the model's own tensors, which a deployment binds from its weight
/// files. Every other tensor of a lowered program is the compiler's (a scaler, a block table) and keeps its value.
fn declared(src: &str) -> Vec<String> {
    src.lines()
        .filter_map(|l| {
            l.strip_prefix("Let ")
                .and_then(|r| r.split_once(" be "))
                .map(|(v, _)| v.to_string())
        })
        .collect()
}

/// Run one program as a deployment runs it: a carried tensor from the previous program, `tok`/`pos` from the host,
/// every other model tensor from its name's fill (the weight file), and the compiler's own tensors as lowered.
/// Returns the outputs and a name lookup.
fn run(
    p: &SpatialProgram,
    model_tensors: &[String],
    bind: &dyn Fn(&str) -> String,
    carried: &[(String, Vec<f64>)],
) -> (
    Outputs,
    Vec<(String, logicaffeine_compile::spatial::ir::TensorId)>,
) {
    let mut inputs = Inputs::default();
    for t in &p.tensors {
        if t.role == TensorRole::Output {
            continue;
        }
        let bound = bind(&t.name);
        let n = t.elems.rows * t.elems.cols;
        let v = match (
            carried.iter().find(|(name, _)| *name == bound),
            bound.as_str(),
        ) {
            (Some((_, v)), _) => v.clone(),
            (None, "tok") => vec![TOK as f64],
            (None, "pos") => vec![POS as f64],
            _ if model_tensors.contains(&t.name) => (0..n).map(|i| fill_value(&bound, i)).collect(),
            _ => continue,
        };
        inputs.tensors.insert(t.id, Value::Float(v));
    }
    let out = eval_choreography(p, &inputs).expect("the program evaluates");
    let names = p.tensors.iter().map(|t| (bind(&t.name), t.id)).collect();
    (out, names)
}

fn result(
    out: &Outputs,
    names: &[(String, logicaffeine_compile::spatial::ir::TensorId)],
    name: &str,
) -> Vec<f64> {
    let id = names
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("`{name}` is in the program"))
        .1;
    floats(
        out.tensors
            .get(&id)
            .unwrap_or_else(|| panic!("`{name}` is a result")),
    )
}

/// ── THE BUNDLE IS A DECODE DEPLOYMENT: an embedding program, layer kinds whose attention reads a KV cache, a
/// head that emits a token.
#[test]
fn the_decode_bundle_embeds_attends_a_cache_and_emits_a_token() {
    let cfg = cfg();
    let m = model();
    let embed = m
        .embed
        .as_ref()
        .expect("a decode bundle has an embedding program");
    assert!(
        embed.tensors.iter().any(|t| t.name == "Emb"),
        "the embedding program reads the table"
    );
    assert!(
        embed
            .tensors
            .iter()
            .any(|t| t.name == "x" && t.role == TensorRole::Output),
        "and writes the first hidden state"
    );
    assert!(
        m.head
            .tensors
            .iter()
            .any(|t| t.name == "next" && t.role == TensorRole::Output),
        "the head emits the next token"
    );
    let attn = m
        .kinds
        .iter()
        .find(|k| k.name == "attention")
        .expect("an attention kind");
    let l = attn.representative;
    for cache in [format!("Kc{l}"), format!("Vc{l}")] {
        let t = attn
            .program
            .tensors
            .iter()
            .find(|t| t.name == cache)
            .unwrap_or_else(|| panic!("the attention kind carries `{cache}`"));
        assert_eq!(
            t.role,
            TensorRole::State,
            "`{cache}` is carried from step to step"
        );
        assert_eq!(
            t.elems.rows * t.elems.cols,
            CTX * cfg.akv(),
            "`{cache}` holds {CTX} positions of every head"
        );
    }
    assert!(
        attn.program.tensors.iter().any(|t| t.name == "pos"),
        "the attention reads the runtime position"
    );
    for p in std::iter::once(embed)
        .chain(m.kinds.iter().map(|k| &k.program))
        .chain(std::iter::once(&m.head))
    {
        assert!(
            p.certificate.as_ref().is_some_and(|c| c.recheck(p)),
            "{}: the program carries a certificate that rechecks",
            p.name
        );
    }
}

/// ── A PROGRAM OF THE BUNDLE THAT DOES NOT LOWER IS NAMED, WITH THE FRONT END'S OWN REFUSAL: which program, which
/// statement, and why — here a literal write to the final norm's weight after the norm has read it, which no
/// section lifts.
#[test]
fn a_program_that_does_not_lower_is_named_with_its_refusal() {
    let broken = |src: &str| {
        if src.contains("argmax of logits") {
            format!("{src}Set item 1 of on to 0.5.\n")
        } else {
            src.to_string()
        }
    };
    let e =
        lower_decode_streamed(&(&cfg()).spec(), Storage::AllQ8, CTX, &RduChip::Sn50.facts().spatial_target(), broken)
            .expect_err("a head with a write no section lifts does not lower");
    let msg = e.to_string();
    assert!(msg.contains("head"), "the refusal names the program: {msg}");
    assert!(
        msg.contains("`on`") && msg.contains("after an op has used the tensor"),
        "and the statement, in the front end's words: {msg}"
    );
}

/// ── ONE DECODE STEP FROM THE BUNDLE IS THE VM'S DECODE STEP: the same token, the same cache rows.
#[test]
fn one_decode_step_from_the_bundle_is_the_vms_decode_step() {
    let cfg = cfg();
    let m = model();
    let embed_src = declared(&gen_qwen38_decode_embed_lg(&cfg, "/w", 1).0);
    let (embed_out, embed_names) = run(
        m.embed.as_ref().expect("an embedding program"),
        &embed_src,
        &|n: &str| n.to_string(),
        &[],
    );
    let mut hidden: (String, Vec<f64>) = ("x".to_string(), result(&embed_out, &embed_names, "x"));
    let mut rows: Vec<(String, Vec<f64>)> = Vec::new();
    for b in &m.layers {
        let kind = &m.kinds[b.kind];
        let model_tensors =
            declared(&gen_qwen38_decode_layer_lg(&cfg, kind.representative, "/w", CTX, 1).0);
        let map = b.tensors.clone();
        let bind = move |name: &str| {
            map.iter()
                .find(|(from, _)| from == name)
                .map(|(_, to)| to.clone())
                .unwrap_or_else(|| name.to_string())
        };
        let (out, names) = run(
            &kind.program,
            &model_tensors,
            &bind,
            std::slice::from_ref(&hidden),
        );
        if kind.name == "attention" {
            for c in ["Kc", "Vc"] {
                let cache = result(&out, &names, &format!("{c}{}", b.layer));
                rows.push((
                    format!("{c}{}", b.layer),
                    cache[(POS - 1) * cfg.akv()..POS * cfg.akv()].to_vec(),
                ));
            }
        }
        let next = format!("xo{}", b.layer);
        hidden = (next.clone(), result(&out, &names, &next));
    }
    let head_src = declared(&gen_qwen38_decode_head_lg(&cfg, "/w").0);
    let (head_out, head_names) = run(
        &m.head,
        &head_src,
        &|n: &str| n.to_string(),
        std::slice::from_ref(&hidden),
    );
    let next = result(&head_out, &head_names, "next");

    let (step, _, _) = gen_qwen38_decode_step_lg(&cfg, "/w", CTX, TOK, POS);
    let mut src = with_fills(&step);
    src.push_str("Show next.\n");
    for (name, _) in &rows {
        for e in 1..=cfg.akv() {
            src.push_str(&format!(
                "Show round(item {} of {name} * 1000000).\n",
                (POS - 1) * cfg.akv() + e
            ));
        }
    }
    let vm = vm_outcome(&src);
    assert!(
        vm.error.is_none(),
        "the whole decode step runs on the VM: {:?}",
        vm.error
    );
    let want: Vec<i64> = vm
        .output
        .split_whitespace()
        .map(|t| t.parse().expect("an integer"))
        .collect();
    assert_eq!(
        next,
        vec![want[0] as f64],
        "the bundle's decode step emits the VM's token"
    );
    for ((name, got), want_row) in rows.iter().zip(want[1..].chunks(cfg.akv())) {
        let scale = want_row.iter().map(|x| x.abs()).max().unwrap_or(1).max(1) as f64;
        let ours: Vec<i64> = got.iter().map(|x| (x * 1e6).round() as i64).collect();
        let worst = ours
            .iter()
            .zip(want_row)
            .map(|(a, b)| ((a - b) as f64).abs() / scale)
            .fold(0.0, f64::max);
        assert!(worst <= 0.02, "{name} row {POS}: the appended row drifts {worst:.4}\n  ours: {ours:?}\n  vm:   {want_row:?}");
    }
    println!(
        "{} layers from {} kind programs + embed + head: token {} agrees",
        cfg.layers,
        m.kinds.len(),
        want[0]
    );
}
