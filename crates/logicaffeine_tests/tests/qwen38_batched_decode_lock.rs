//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! BATCHED QWEN3.8 DECODE ON THE SN50 — B sequences per step, every weight read once for all of them.
//!
//! A decode step at batch 1 reads every weight to produce ONE token; serving throughput comes from batching, where
//! the same weight pass serves B sequences. `gen_qwen38_decode_batch_lg` writes that step: the hidden states stacked
//! into a `B × d` matrix, every weight read on the stack (norms row by row, projections as `B × d` matmuls, gates,
//! FFN, residuals), and what depends on one sequence's own position or carried state — conv rings, per-head norms,
//! the gated-delta memory, RoPE, the cache append, the attention — per sequence, on its row.
//!
//! Pinned:
//!   1. SEQUENCE b OF THE BATCH IS THE SINGLE STEP for `(tok_b, pos_b)` over the same state, on the VM: the same
//!      token, the same logits, the same appended cache rows.
//!   2. THE BATCHED STEP LOWERS WHOLE onto the SN50 and its evaluation emits the VM's tokens.
//!   3. EVERY PROJECTION WEIGHT IS READ BY ONE SECTION for the whole batch — not once per sequence.
//!   4. THE BATCHED DECODE BUNDLE IS THE BATCHED STEP: the embedding program, one program per layer kind and the
//!      head (`lower_batched_decode_streamed`), run program by program through the manifest's bindings, emit
//!      every sequence's token exactly and its logits and appended cache rows within the whole-forward bf16 bound —
//!      and each kind program reads each of its projection weights in one section.
//!   5. THE BATCHED STEP IN THE RDU'S OWN LINEAGE: it prints as ONE Spatial app whose host checks every sequence's
//!      token exactly and its logits and appended cache rows against the VM — and that app runs, whole, in
//!      Spatial's own simulator and passes (tool-gated: `docker` and the built toolchain; serialized in the
//!      `plasticine-chain` nextest group).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test qwen38_batched_decode_lock`
//!
//! ★ MUTATION PROBES (run 2026-09-25, each in a private copy of the workspace, each red):
//!   - `dense_tables` without its matmul arm → 2, 3, 4 (the stacks flat, the projections' operands not).
//!   - the bundle's layout agreement dropped for the embedding program → 4: every program still evaluates, and `X`
//!     is `1 × 24` in the embedding and `3 × 8` in layer 0 — two memory layouts of one tensor, which only the
//!     one-shape assertion sees, because the evaluator holds tensors as logical elements.
//!   - `validate_plan` checking a composed program against its first result and every item → 4 (`GridMismatch`).
//!   - the certificate refusing any second writer of carried state → 2, 3, 4 (`X`, `bct`, `bod` are written a row
//!     per sequence).
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen38::{
    gen_qwen38_decode_batch_embed_lg, gen_qwen38_decode_batch_head_lg,
    gen_qwen38_decode_batch_layer_lg, gen_qwen38_decode_batch_lg, gen_qwen38_decode_step_lg,
    Qwen38Cfg,
};
use logicaffeine_compile::spatial::streamed::Storage;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{
    eval_choreography, eval_choreography_every_tensor, Inputs, Outputs, Value,
};
use logicaffeine_compile::spatial::ir::{SpatialProgram, StageKind, TensorId, TensorRole};
use logicaffeine_compile::spatial::lower::spatial_lang::{
    emit_spatial_program_checked, SpatialCheck, SpatialFlavor,
};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::streamed::lower_batched_decode_streamed;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use std::collections::BTreeSet;

mod common;
#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::inputs_from_fills;

const CTX: usize = 6;
/// `(tok, pos)` per sequence: three sequences at three different positions.
const SEQS: [(usize, usize); 3] = [(5, 3), (2, 1), (7, 6)];

/// Element `i` of tensor `var`'s fill: a hash of the index and the name — with a sequence's own copy of a tensor
/// (`Kc3_s1`, `St0_s2`) filled as the single step fills the tensor it copies (`Kc3`, `St0`), so each sequence of the
/// batch starts from the single step's state.
fn fill_value(var: &str, i: usize) -> f64 {
    let base = var.split("_s").next().unwrap_or(var);
    let salt = base.bytes().map(|b| b as u64).sum::<u64>();
    let h = (i as u64)
        .wrapping_mul(2654435761)
        .wrapping_add(salt.wrapping_mul(40503));
    ((h % 97) as f64 / 97.0 - 0.5) * 0.4
}

/// Every declared `Real` tensor filled by [`fill_value`].
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

fn cfg() -> Qwen38Cfg {
    Qwen38Cfg::tiny()
}

fn batch_source() -> String {
    with_fills(&gen_qwen38_decode_batch_lg(&cfg(), "/w", CTX, &SEQS).0)
}

fn vm_numbers(src: &str) -> Vec<f64> {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "the VM runs it: {:?}", vm.error);
    vm.output
        .split_whitespace()
        .map(|t| t.parse().expect("a number"))
        .collect()
}

/// The attention layers' caches.
fn attention_layers() -> Vec<usize> {
    let c = cfg();
    (0..c.layers).filter(|l| c.is_attn(*l)).collect()
}

/// ── 1. SEQUENCE b OF THE BATCH IS THE SINGLE STEP: its token, its logits, its appended cache rows.
#[test]
fn each_sequence_of_the_batch_is_the_single_step_on_the_vm() {
    let c = cfg();
    let akv = c.akv();
    let mut shown = batch_source();
    for b in 0..SEQS.len() {
        shown.push_str(&format!("Show next_s{b}.\n"));
        for i in 1..=c.vocab {
            shown.push_str(&format!("Show item {i} of logits_s{b}.\n"));
        }
        for l in attention_layers() {
            for e in 1..=akv {
                shown.push_str(&format!(
                    "Show item {} of Kc{l}_s{b}.\nShow item {} of Vc{l}_s{b}.\n",
                    (SEQS[b].1 - 1) * akv + e,
                    (SEQS[b].1 - 1) * akv + e
                ));
            }
        }
    }
    let batched = vm_numbers(&shown);
    let per_seq = 1 + c.vocab + attention_layers().len() * akv * 2;
    for (b, (tok, pos)) in SEQS.iter().enumerate() {
        let mut single =
            qwen38_fixture_fills(&gen_qwen38_decode_step_lg(&c, "/w", CTX, *tok, *pos).0);
        single.push_str("Show next.\n");
        for i in 1..=c.vocab {
            single.push_str(&format!("Show item {i} of logits.\n"));
        }
        for l in attention_layers() {
            for e in 1..=akv {
                single.push_str(&format!(
                    "Show item {} of Kc{l}.\nShow item {} of Vc{l}.\n",
                    (pos - 1) * akv + e,
                    (pos - 1) * akv + e
                ));
            }
        }
        let want = vm_numbers(&single);
        let got = &batched[b * per_seq..(b + 1) * per_seq];
        assert_eq!(
            got[0], want[0],
            "sequence {b} (token {tok} at {pos}): the batch emits the single step's token"
        );
        for (k, (g, w)) in got.iter().zip(&want).enumerate() {
            assert!(
                (g - w).abs() <= 1e-12 * w.abs().max(1.0),
                "sequence {b}, value {k}: batched {g} vs single {w}"
            );
        }
    }
}

/// The single step's fill: the same hash, from the tensor's own name.
fn qwen38_fixture_fills(src: &str) -> String {
    with_fills(src)
}

fn composed() -> SpatialProgram {
    let target = RduChip::Sn50.facts().spatial_target();
    let pl = compile_program_to_spatial(&batch_source(), &target)
        .expect("the walk lifts the batched step");
    pl.whole()
        .unwrap_or_else(|| panic!("the batched step lowers whole: {}", pl.refusal()))
        .clone()
}

/// ── 2. THE BATCHED STEP LOWERS WHOLE AND EMITS THE VM'S TOKENS.
#[test]
fn the_batched_step_lowers_whole_and_emits_the_vms_tokens() {
    let p = composed();
    assert!(
        p.certificate.as_ref().is_some_and(|c| c.recheck(&p)),
        "the batched step certifies"
    );
    let out = eval_choreography(&p, &inputs_from_fills(&p)).expect("the batched step evaluates");
    let mut shown = batch_source();
    for b in 0..SEQS.len() {
        shown.push_str(&format!("Show next_s{b}.\n"));
    }
    let want = vm_numbers(&shown);
    for b in 0..SEQS.len() {
        let t = p
            .tensors
            .iter()
            .find(|t| t.name == format!("next_s{b}"))
            .unwrap_or_else(|| panic!("next_s{b} is a result"));
        let got = match out.tensors.get(&t.id).expect("the token is written") {
            Value::Int(v) => v[0] as f64,
            Value::Float(v) => v[0],
        };
        assert_eq!(got, want[b], "sequence {b}: the chip emits the VM's token");
    }
}

/// The projection weights of `p`, each asserted read by ONE section; returns how many there were.
fn projections_read_once(p: &SpatialProgram) -> usize {
    let projections = [
        "Wq", "Wqg", "Wk", "Wv", "Wz", "Wb", "Wa", "Wo", "Wg", "Wu", "Wd", "Wout",
    ];
    let is_projection = |name: &str| {
        projections.iter().any(|w| {
            name.strip_prefix(w)
                .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
        })
    };
    let mut checked = 0;
    for t in p.tensors.iter().filter(|t| is_projection(&t.name)) {
        let sections: BTreeSet<Option<usize>> = p
            .stages
            .iter()
            .filter(|s| matches!(&s.kind, StageKind::Load { tensor, .. } if *tensor == t.id))
            .map(|s| p.section_of_stage(s.id))
            .collect();
        assert_eq!(
            sections.len(),
            1,
            "{}: read by one section for all {} sequences, not {:?}",
            t.name,
            SEQS.len(),
            sections
        );
        checked += 1;
    }
    checked
}

/// ── 3. EVERY PROJECTION WEIGHT IS READ BY ONE SECTION FOR THE WHOLE BATCH.
#[test]
fn every_projection_weight_is_read_once_for_the_batch() {
    let checked = projections_read_once(&composed());
    assert!(checked >= 12, "the projections were found ({checked})");
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

fn floats(v: &Value) -> Vec<f64> {
    match v {
        Value::Float(f) => f.clone(),
        Value::Int(i) => i.iter().map(|x| *x as f64).collect(),
    }
}

/// The batch's token ids and every sequence's position, as the host hands them to a program.
fn runtime_input(name: &str) -> Option<Vec<f64>> {
    if name == "toks" {
        return Some(SEQS.iter().map(|(tok, _)| *tok as f64).collect());
    }
    SEQS.iter()
        .enumerate()
        .find_map(|(b, (_, pos))| (name == format!("pos_s{b}")).then(|| vec![*pos as f64]))
}

/// Run one program of the bundle as a deployment runs it: a tensor the previous program produced, each sequence's
/// token and position from the host, every other model tensor from its bound name's fill (the weight file), and the
/// compiler's own tensors as lowered. Every tensor it computes is returned, the logits a head reads on the way to its
/// tokens included.
fn run(
    p: &SpatialProgram,
    model_tensors: &[String],
    bind: &dyn Fn(&str) -> String,
    carried: &[(String, Vec<f64>)],
) -> (Outputs, Vec<(String, TensorId)>) {
    let mut inputs = Inputs::default();
    for t in &p.tensors {
        if t.role == TensorRole::Output {
            continue;
        }
        let bound = bind(&t.name);
        let n = t.elems.rows * t.elems.cols;
        let v = match (
            carried.iter().find(|(name, _)| *name == bound),
            runtime_input(&bound),
        ) {
            (Some((_, v)), _) => v.clone(),
            (None, Some(v)) => v,
            _ if model_tensors.contains(&t.name) => (0..n).map(|i| fill_value(&bound, i)).collect(),
            _ => continue,
        };
        inputs.tensors.insert(t.id, Value::Float(v));
    }
    let out = eval_choreography_every_tensor(p, &inputs)
        .unwrap_or_else(|e| panic!("{}: the program evaluates: {e:?}", p.name));
    let names = p.tensors.iter().map(|t| (bind(&t.name), t.id)).collect();
    (out, names)
}

fn result(out: &Outputs, names: &[(String, TensorId)], name: &str) -> Vec<f64> {
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

/// `qwen38_rdu_lock`'s stated bf16 bound for a whole forward, relative to the largest value.
const BF16_WHOLE_FORWARD_REL: f64 = 0.02;

fn drift(got: &[f64], want: &[f64]) -> f64 {
    let scale = want
        .iter()
        .fold(0.0f64, |m, x| m.max(x.abs()))
        .max(f64::MIN_POSITIVE);
    got.iter()
        .zip(want)
        .map(|(a, b)| (a - b).abs() / scale)
        .fold(0.0, f64::max)
}

/// ── 4. THE BATCHED DECODE BUNDLE IS THE BATCHED STEP, run program by program through its bindings.
#[test]
fn one_batched_step_from_the_bundle_is_the_vms_batched_step() {
    let c = cfg();
    let bsz = SEQS.len();
    let m = lower_batched_decode_streamed(&c.spec(), Storage::AllQ8,
        CTX,
        bsz,
        &RduChip::Sn50.facts().spatial_target(),
        with_fills,
    )
    .expect("the batched decode bundle lowers");
    assert_eq!(m.kinds.len(), 2, "one program per layer kind");
    for p in m
        .embed
        .iter()
        .chain(m.kinds.iter().map(|k| &k.program))
        .chain(std::iter::once(&m.head))
    {
        assert!(
            p.certificate.as_ref().is_some_and(|cert| cert.recheck(p)),
            "{}: the program carries a certificate that rechecks",
            p.name
        );
    }
    for k in &m.kinds {
        assert!(
            projections_read_once(&k.program) >= 7,
            "the {} kind reads its projections",
            k.name
        );
    }
    projections_read_once(&m.head);

    // ★ THE PROGRAMS AGREE ON EVERY TENSOR THEY SHARE. The evaluator holds tensors as logical elements, so a stack the
    // embedding program lays out flat and layer 0 reads as a matrix would run here and be two different memory
    // layouts on the chip. Every tensor, under the name each layer binds it to, has ONE shape across the bundle.
    let mut shapes: std::collections::BTreeMap<String, ((usize, usize), String)> =
        Default::default();
    let mut agree = |program: &str, p: &SpatialProgram, bind: &dyn Fn(&str) -> String| {
        for t in &p.tensors {
            let shape = (t.elems.rows, t.elems.cols);
            let (have, first) = shapes
                .entry(bind(&t.name))
                .or_insert((shape, program.to_string()))
                .clone();
            assert_eq!(
                have,
                shape,
                "`{}` is {have:?} in {first} and {shape:?} in {program}",
                bind(&t.name)
            );
        }
    };
    agree(
        "embed",
        m.embed.as_ref().expect("an embedding program"),
        &|n: &str| n.to_string(),
    );
    for binding in &m.layers {
        let map = binding.tensors.clone();
        agree(
            &format!("layer {}", binding.layer),
            &m.kinds[binding.kind].program,
            &move |n: &str| {
                map.iter()
                    .find(|(from, _)| from == n)
                    .map(|(_, to)| to.clone())
                    .unwrap_or_else(|| n.to_string())
            },
        );
    }
    agree("head", &m.head, &|n: &str| n.to_string());
    assert_eq!(
        shapes.get("X").map(|s| s.0),
        Some((bsz, c.d)),
        "the stacked first hidden state is a {bsz} × d matrix in every program"
    );

    let embed_src = declared(&gen_qwen38_decode_batch_embed_lg(&c, "/w", &[1; SEQS.len()]).0);
    let (embed_out, embed_names) = run(
        m.embed.as_ref().expect("an embedding program"),
        &embed_src,
        &|n: &str| n.to_string(),
        &[],
    );
    let mut hidden: (String, Vec<f64>) = ("X".to_string(), result(&embed_out, &embed_names, "X"));
    let mut rows: Vec<(String, Vec<f64>)> = Vec::new();
    for binding in &m.layers {
        let kind = &m.kinds[binding.kind];
        let model_tensors = declared(
            &gen_qwen38_decode_batch_layer_lg(&c, kind.representative, "/w", CTX, &[1; SEQS.len()])
                .0,
        );
        let map = binding.tensors.clone();
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
            for (b, (_, pos)) in SEQS.iter().enumerate() {
                for cache in ["Kc", "Vc"] {
                    let name = format!("{cache}{}_s{b}", binding.layer);
                    let all = result(&out, &names, &name);
                    rows.push((name, all[(pos - 1) * c.akv()..pos * c.akv()].to_vec()));
                }
            }
        }
        let next = format!("bxo{}", binding.layer);
        hidden = (next.clone(), result(&out, &names, &next));
    }
    let head_src = declared(&gen_qwen38_decode_batch_head_lg(&c, "/w", bsz).0);
    let (head_out, head_names) = run(
        &m.head,
        &head_src,
        &|n: &str| n.to_string(),
        std::slice::from_ref(&hidden),
    );

    let mut shown = batch_source();
    for b in 0..bsz {
        shown.push_str(&format!("Show next_s{b}.\n"));
        for i in 1..=c.vocab {
            shown.push_str(&format!("Show item {i} of logits_s{b}.\n"));
        }
    }
    for (name, _) in &rows {
        let b: usize = name.rsplit("_s").next().unwrap().parse().unwrap();
        for e in 1..=c.akv() {
            shown.push_str(&format!(
                "Show item {} of {name}.\n",
                (SEQS[b].1 - 1) * c.akv() + e
            ));
        }
    }
    let want = vm_numbers(&shown);
    let per_seq = 1 + c.vocab;
    for b in 0..bsz {
        let vm = &want[b * per_seq..(b + 1) * per_seq];
        assert_eq!(
            result(&head_out, &head_names, &format!("next_s{b}")),
            vec![vm[0]],
            "sequence {b}: the bundle emits the VM's token"
        );
        let logits = result(&head_out, &head_names, &format!("logits_s{b}"));
        let d = drift(&logits, &vm[1..]);
        assert!(
            d <= BF16_WHOLE_FORWARD_REL,
            "sequence {b}: the logits drift {d:.4} from the VM\n  ours: {logits:?}\n  vm:   {:?}",
            &vm[1..]
        );
    }
    for ((name, got), vm_row) in rows.iter().zip(want[bsz * per_seq..].chunks(c.akv())) {
        let d = drift(got, vm_row);
        assert!(
            d <= BF16_WHOLE_FORWARD_REL,
            "{name}: the appended row drifts {d:.4}\n  ours: {got:?}\n  vm:   {vm_row:?}"
        );
    }
}

/// The batched app's host checks, every gold the VM's: each sequence's token exactly, its logits, and the row
/// `pos_b` its step appended to every attention layer's key and value cache. The app computes in fp32 against the
/// VM's f64, so its floats sit within `1e-3` of the largest gold.
fn vm_checks(src: &str) -> Vec<SpatialCheck> {
    let c = cfg();
    let akv = c.akv();
    let mut shown = src.to_string();
    let mut layout: Vec<(String, usize, usize, f64)> = Vec::new();
    for (b, (_, pos)) in SEQS.iter().enumerate() {
        shown.push_str(&format!("Show next_s{b}.\n"));
        layout.push((format!("next_s{b}"), 0, 1, 0.0));
        for i in 1..=c.vocab {
            shown.push_str(&format!("Show item {i} of logits_s{b}.\n"));
        }
        layout.push((format!("logits_s{b}"), 0, c.vocab, 1e-3));
        for l in attention_layers() {
            for cache in ["Kc", "Vc"] {
                for e in 1..=akv {
                    shown.push_str(&format!(
                        "Show item {} of {cache}{l}_s{b}.\n",
                        (pos - 1) * akv + e
                    ));
                }
                layout.push((format!("{cache}{l}_s{b}"), (pos - 1) * akv, akv, 1e-3));
            }
        }
    }
    let vals = vm_numbers(&shown);
    let mut at = 0;
    layout
        .into_iter()
        .map(|(tensor, offset, n, rel_tol)| {
            let gold = vals[at..at + n].to_vec();
            at += n;
            SpatialCheck {
                tensor,
                offset,
                gold,
                rel_tol,
            }
        })
        .collect()
}

/// ── 5. THE BATCHED STEP PRINTS AS ONE SPATIAL APP whose host checks every sequence against the VM.
#[test]
fn the_batched_step_prints_as_one_checked_spatial_app() {
    let p = composed();
    let checks = vm_checks(&batch_source());
    let app = emit_spatial_program_checked(
        &p,
        &RduChip::Sn50.facts().spatial_target(),
        &checks,
        SpatialFlavor::Simulator,
    )
    .unwrap_or_else(|e| panic!("the batched step prints as Spatial source: {e:?}"));
    assert!(app.contains("@spatial class Logos_"), "a Spatial app");
    for (k, ch) in checks.iter().enumerate() {
        assert!(
            app.contains(&format!("// check {k}: {} elements", ch.gold.len()))
                && app.contains(&format!("of {} against", ch.tensor)),
            "check {k} ({}) is printed",
            ch.tensor
        );
    }
    assert!(
        app.contains(&format!(
            "val cksum = {}",
            (0..checks.len())
                .map(|k| format!("ok{k}"))
                .collect::<Vec<_>>()
                .join(" && ")
        )),
        "the PASS is every check at once"
    );
}

/// ── 5. AND THE APP RUNS, WHOLE, IN SPATIAL'S OWN SIMULATOR: every controller of every section executes and the
/// host's checks — every sequence's token, logits and appended cache rows — pass.
#[test]
fn the_batched_step_is_the_vms_in_spatials_simulator() {
    if !common::require_tool("docker") || !common::require_tool("plasticine-toolchain") {
        return;
    }
    let p = composed();
    let src = batch_source();
    let app = emit_spatial_program_checked(
        &p,
        &RduChip::Sn50.facts().spatial_target(),
        &vm_checks(&src),
        SpatialFlavor::Simulator,
    )
    .expect("the batched step prints");
    let class = app
        .split("@spatial class ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .expect("the app's class")
        .to_string();
    let regression = common::plasticine::regression_dir();
    std::fs::write(regression.join(format!("{class}.scala")), &app)
        .expect("write the regression app");
    std::fs::write(regression.join(format!("{class}.lg")), &src)
        .expect("write the LOGOS source beside it");
    let run = common::plasticine::run_sim(&class);
    println!("sim log: {}", run.log_path.display());
    assert!(
        run.passed(),
        "Spatial's simulator prints the harness's PASS for {class} — log at {}:\n{}",
        run.log_path.display(),
        run.tail(80)
    );
    for line in run.log.lines().filter(|l| l.contains("check ")) {
        println!("{line}");
    }
}
