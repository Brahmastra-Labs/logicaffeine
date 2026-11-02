//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! QWEN3.8 TENSOR-PARALLEL OVER RDUs — Megatron's split of a decode step, as ONE certified spatial program.
//!
//! An SN50 serves a model the size of Qwen3.8-27B split over the chips of a node: every layer's input projections by
//! COLUMN (each chip owns whole heads and whole FFN columns), its output projections by ROW (each chip holds a
//! partial of the full output), and an all-reduce summing the partials twice a layer. `gen_qwen38_tp_decode_step_lg`
//! writes that decomposition as LOGOS — each chip's slice under its own tensors, each all-reduce as the fold of
//! `plus` it means — with a manifest naming every chip-local tensor, the slice of the checkpoint it is, and every
//! collective.
//!
//! Pinned:
//!   1. THE DECOMPOSITION IS THE STEP: the tensor-parallel source, its slices filled from the full tensors, runs on
//!      the VM to the unsharded step's token, logits and every layer's output — with the KV heads split over the
//!      chips, replicated over them (`tp > kv`), and at four chips.
//!   2. THE COMPILER LOWERS IT AS ONE PROGRAM OVER THE CHIPS: every section on the chip of what it computes, every
//!      chip-local tensor read on its own chip, one all-reduce per collective folding a partial from EVERY chip (two
//!      a layer); the program certifies, and it emits the VM's token and logits.
//!   3. AN ALL-REDUCE IS THE ONLY WAY A CHIP'S DATA REACHES THE REPLICATED SPINE: the same program with its folds
//!      lowered as ordinary additions on one chip, or with one all-reduce dropping a chip's partial, does not certify
//!      (`UnreducedShard`).
//!   4. EVERY CHIP HOLDS EXACTLY ITS SLICES: the weight and state slices a chip reads are the manifest's for that
//!      chip, and over the chips they tile every full tensor exactly once — the KV projections and caches `tp/kv`
//!      times when their heads are replicated.
//!   5. THE CHIPS RUN AT ONCE: the sections computing one op's per-chip slices share a phase (no barrier between
//!      them), no phase runs two sections on one chip, and a barrier stands only where data crosses.
//!   6. THE TENSOR-PARALLEL DECODE BUNDLE (`lower_tp_decode_streamed`): the embedding and head on the
//!      replicated spine, each layer kind one program over the chips — run program by program through the kind
//!      bindings, it emits the VM's token.
//!   7. BATCHED AND TENSOR-PARALLEL — the serving shape: `gen_qwen38_decode_batch_tp_lg` is the batched step on the
//!      VM, lowers as one program over the chips emitting every sequence's token, and each chip reads each of its
//!      weight slices in ONE section for the whole batch.
//!   8. A WIDTH THE HEADS DO NOT DIVIDE: Qwen3.8-27B's 24 query heads over a rack's 16 chips, one KV head on every
//!      chip, fewer Gated-DeltaNet heads than chips — every head on exactly one chip, ⌊h/tp⌋ or ⌈h/tp⌉ a chip, each
//!      chip with the KV (key) heads its query (value) heads read, a chip with no share of a layer folding a zero
//!      partial; the step and the batched step are the VM's unsharded ones, and lower to one certified program over
//!      the 16 chips that emits their tokens. No fusion spans two chips, a chip and the spine, or a collective's fold.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test qwen38_tensor_parallel_lock`
//!
//! ★ MUTATION PROBES (run 2026-09-25, each in a private copy of the workspace, each red):
//!   - the Gated-DeltaNet value heads split in one contiguous run per chip → 1 (3e-4 off at layer 0: this
//!     checkpoint tiles its keys across the value heads, so a chip's value heads are one block per recurrence).
//!   - a chip's KV head chosen as `c mod kv` → 1 (four chips).
//!   - relocation leaving the stages on chip 0 → 2, 3, 4, 5, 6.
//!   - the certificate not checking what reaches the replicated spine → 3 (both) and `spatial_tensor_parallel_lock`'s
//!     skipped all-reduce.
//!   - an incomplete all-reduce counted as a reduction → 3 (the missing chip).
//!   - the phases ignoring dependencies → 2..6 (the composition no longer certifies); always a barrier → 5.
//!   - `barriers_between` requiring a barrier at EVERY boundary again → 2 (concurrent chip sections between a
//!     writer and its reader).
//!   - a partial leaving its chip kept as a local `Fifo` → 2 (`FifoProducerCount`).
//!   - the manifest without its slices → `emit_rdu_e2e::the_tensor_parallel_bundle_on_disk_runs_the_step_the_vm_runs`.
//!   - admissibility without the KV-head rule → `rdu_fit_report_lock::the_tensor_parallel_width_is_one_the_model_admits`.
//!   - the batched tensor-parallel layer giving every sequence sequence 0's RoPE tables → 7 (the third sequence's
//!     logits).
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen38::{
    gen_qwen38_decode_batch_lg, gen_qwen38_decode_batch_tp_lg, gen_qwen38_decode_step_lg,
    gen_qwen38_tp_decode_step_lg, Qwen38Cfg,
};
use logicaffeine_compile::spatial::streamed::Storage;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::certificate::{certify_choreography, CertifyFault};
use logicaffeine_compile::spatial::eval::Inputs;
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::fit::qwen38_admits_tp;
use logicaffeine_compile::spatial::ir::{ComputeOp, SpatialProgram, StageKind, TensorBody};
use logicaffeine_compile::spatial::ir::{TensorId, TensorRole};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial_tp;
use logicaffeine_compile::spatial::streamed::lower_tp_decode_streamed;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::tensor_parallel::{TensorParallelShim, TpManifest};
use logicaffeine_compile::tensor_graph::fuse::candidate;
use logicaffeine_compile::tensor_graph::graph::{Op, Storage as GraphStorage, TensorGraph};
use logicaffeine_compile::tensor_graph::shim::{ReferenceShim, Shim};
use logicaffeine_compile::tensor_graph::symbols::ClassId;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::inputs_from_fills;

const CTX: usize = 6;
const TOK: usize = 5;
const POS: usize = 3;

/// Element `i` of tensor `var`'s fill: a hash of the index and the name.
fn fill_value(var: &str, i: usize) -> f64 {
    let salt = var.bytes().map(|b| b as u64).sum::<u64>();
    let h = (i as u64)
        .wrapping_mul(2654435761)
        .wrapping_add(salt.wrapping_mul(40503));
    ((h % 97) as f64 / 97.0 - 0.5) * 0.4
}

/// Every declared `Real` tensor filled — a chip's slice with the values of the full tensor it cuts, at their
/// positions there; every other tensor from its own name.
fn with_fills(src: &str, m: Option<&TpManifest>) -> String {
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
        let (rows, cols) = (r.parse::<usize>().unwrap(), c.parse::<usize>().unwrap());
        let slice = m.and_then(|m| m.slices.get(var));
        for i in 0..rows * cols {
            let v = match slice {
                Some(s) => fill_value(&s.of, s.full_index(i / cols, i % cols)),
                None => fill_value(var, i),
            };
            out.push_str(&format!("Set item {} of {var} to {v:?}.\n", i + 1));
        }
    }
    out
}

/// A miniature whose heads split over two chips: 4 query heads over 2 KV heads, 2/4 Gated-DeltaNet heads.
fn split_kv() -> Qwen38Cfg {
    Qwen38Cfg {
        h: 4,
        kv: 2,
        gdn_k_heads: 2,
        gdn_v_heads: 4,
        ..Qwen38Cfg::tiny()
    }
}

/// One KV head for 4 query heads: at two chips it is REPLICATED, each chip's two query heads reading it.
fn replicated_kv() -> Qwen38Cfg {
    Qwen38Cfg {
        h: 4,
        kv: 1,
        gdn_k_heads: 2,
        gdn_v_heads: 4,
        ..Qwen38Cfg::tiny()
    }
}

/// Four chips: 8 query heads over 2 KV heads (each replicated on two chips), 4/8 Gated-DeltaNet heads.
fn four_chips() -> Qwen38Cfg {
    Qwen38Cfg::mini()
}

fn vm_numbers(src: &str) -> Vec<f64> {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "the VM runs it: {:?}", vm.error);
    vm.output
        .split_whitespace()
        .map(|t| t.parse().expect("a number"))
        .collect()
}

/// What both steps show: the token, every logit, and every layer's output.
fn shown(c: &Qwen38Cfg) -> String {
    let mut s = String::from("Show next.\n");
    for i in 1..=c.vocab {
        s.push_str(&format!("Show item {i} of logits.\n"));
    }
    for l in 0..c.layers {
        for i in 1..=c.d {
            s.push_str(&format!("Show item {i} of xo{l}.\n"));
        }
    }
    s
}

/// ── 1. THE DECOMPOSITION IS THE STEP, on the VM: split KV heads, replicated KV heads, four chips.
#[test]
fn the_tensor_parallel_step_is_the_step_on_the_vm() {
    for (what, c, tp) in [
        ("split KV heads", split_kv(), 2),
        ("replicated KV heads", replicated_kv(), 2),
        ("four chips", four_chips(), 4),
    ] {
        qwen38_admits_tp(&c, tp)
            .unwrap_or_else(|r| panic!("{what}: the test config admits {tp} chips: {}", r.why));
        let (tp_src, _, _, m) = gen_qwen38_tp_decode_step_lg(&c, "/w", CTX, TOK, POS, tp);
        assert_eq!(m.chips, tp);
        assert_eq!(
            m.collectives.len(),
            2 * c.layers,
            "{what}: two all-reduces a layer"
        );
        let whole = vm_numbers(&format!(
            "{}{}",
            with_fills(&gen_qwen38_decode_step_lg(&c, "/w", CTX, TOK, POS).0, None),
            shown(&c)
        ));
        let split = vm_numbers(&format!("{}{}", with_fills(&tp_src, Some(&m)), shown(&c)));
        assert_eq!(
            split.len(),
            whole.len(),
            "{what}: the same values are shown"
        );
        assert_eq!(
            split[0], whole[0],
            "{what}: the tensor-parallel step emits the step's token"
        );
        let scale = whole.iter().fold(0.0f64, |a, x| a.max(x.abs()));
        for (k, (s, w)) in split.iter().zip(&whole).enumerate() {
            assert!((s - w).abs() <= 1e-9 * scale, "{what}: value {k}: tensor-parallel {s} vs whole {w} — only the order of the output projections' sums may differ");
        }
    }
}

/// The three configurations every lock here runs.
fn configs() -> [(&'static str, Qwen38Cfg, usize); 3] {
    [
        ("split KV heads", split_kv(), 2),
        ("replicated KV heads", replicated_kv(), 2),
        ("four chips", four_chips(), 4),
    ]
}

/// The filled tensor-parallel step, its manifest, and its lowering on the SN50.
fn lowered(c: &Qwen38Cfg, tp: usize) -> (String, TpManifest, SpatialProgram) {
    let (src, _, _, m) = gen_qwen38_tp_decode_step_lg(c, "/w", CTX, TOK, POS, tp);
    let src = with_fills(&src, Some(&m));
    let pl = compile_program_to_spatial_tp(&src, &RduChip::Sn50.facts().spatial_target(), &m)
        .expect("the walk lifts the tensor-parallel step");
    let p = pl
        .whole()
        .unwrap_or_else(|| panic!("the tensor-parallel step lowers whole: {}", pl.refusal()))
        .clone();
    (src, m, p)
}

/// `qwen38_rdu_lock`'s stated bf16 bound for a whole forward, relative to the largest logit.
const BF16_WHOLE_FORWARD_REL: f64 = 0.02;

/// ── 2. THE COMPILER LOWERS IT AS ONE PROGRAM OVER THE CHIPS, and it emits the VM's token.
#[test]
fn the_tensor_parallel_step_lowers_over_the_chips_and_emits_the_vms_token() {
    for (what, c, tp) in configs() {
        let (src, m, p) = lowered(&c, tp);
        assert!(
            p.certificate.as_ref().is_some_and(|cert| cert.recheck(&p)),
            "{what}: the program certifies"
        );
        let chips: BTreeSet<u16> = p
            .stages
            .iter()
            .filter_map(|s| s.site.map(|x| x.chip))
            .collect();
        assert_eq!(
            chips,
            (0..tp as u16).collect(),
            "{what}: the program runs on all {tp} chips"
        );
        for s in &p.stages {
            if let StageKind::Load { tensor, .. } = &s.kind {
                if let Some(chip) = m.chip_of.get(&p.tensor(*tensor).name) {
                    assert_eq!(
                        s.site.map(|x| x.chip),
                        Some(*chip),
                        "{what}: `{}` is read on its own chip",
                        p.tensor(*tensor).name
                    );
                }
            }
        }
        let reduced: BTreeSet<String> = p
            .stages
            .iter()
            .filter_map(|s| match &s.kind {
                StageKind::AllReduce { tensor, .. } => {
                    let from: BTreeSet<u16> = s
                        .ins
                        .iter()
                        .filter_map(|b| p.buffer(*b).site.map(|x| x.chip))
                        .collect();
                    assert_eq!(
                        from,
                        (0..tp as u16).collect(),
                        "{what}: the all-reduce into `{}` folds a partial from every chip",
                        p.tensor(*tensor).name
                    );
                    Some(p.tensor(*tensor).name.clone())
                }
                _ => None,
            })
            .collect();
        let expected: BTreeSet<String> = m.collectives.iter().map(|col| col.out.clone()).collect();
        assert_eq!(
            reduced, expected,
            "{what}: one all-reduce per collective, two a layer"
        );

        let out = eval_choreography_every_tensor(&p, &inputs_from_fills(&p))
            .unwrap_or_else(|e| panic!("{what}: the tensor-parallel program evaluates: {e:?}"));
        let value = |name: &str| -> Vec<f64> {
            let id = p
                .tensors
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("{what}: `{name}` is a tensor of the program"))
                .id;
            match out
                .tensors
                .get(&id)
                .unwrap_or_else(|| panic!("{what}: `{name}` is written"))
            {
                Value::Float(v) => v.clone(),
                Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
            }
        };
        let vm = vm_numbers(&format!("{src}{}", shown(&c)));
        assert_eq!(
            value("next"),
            vec![vm[0]],
            "{what}: the chips emit the VM's token"
        );
        let logits = value("logits");
        let scale = vm[1..=c.vocab].iter().fold(0.0f64, |a, x| a.max(x.abs()));
        let drift = logits
            .iter()
            .zip(&vm[1..=c.vocab])
            .map(|(a, b)| (a - b).abs() / scale)
            .fold(0.0, f64::max);
        assert!(
            drift <= BF16_WHOLE_FORWARD_REL,
            "{what}: the logits drift {drift:.4} from the VM"
        );
    }
}

/// ── 3. THE FOLDS LOWERED AS ADDITIONS ON ONE CHIP ARE AN UNREDUCED SHARD: the partials read into the replicated spine
/// without an all-reduce.
#[test]
fn a_fold_lowered_on_one_chip_does_not_certify() {
    for (what, c, tp) in configs() {
        let (src, _, _, mut m) = gen_qwen38_tp_decode_step_lg(&c, "/w", CTX, TOK, POS, tp);
        let src = with_fills(&src, Some(&m));
        m.collectives.clear();
        let pl = compile_program_to_spatial_tp(&src, &RduChip::Sn50.facts().spatial_target(), &m)
            .expect("the walk lifts it");
        assert!(
            pl.whole().is_none(),
            "{what}: partials summed on one chip without an all-reduce must not certify"
        );
        let why = pl.refusal();
        assert!(
            why.contains("UnreducedShard"),
            "{what}: refused as an unreduced shard: {why}"
        );
    }
}

/// ── 3. AN ALL-REDUCE THAT DROPS A CHIP'S PARTIAL is refused: presence is not completeness.
#[test]
fn an_all_reduce_missing_a_chip_does_not_certify() {
    let (_, _, p) = lowered(&split_kv(), 2);
    let at = p
        .stages
        .iter()
        .position(|s| matches!(s.kind, StageKind::AllReduce { .. }))
        .expect("an all-reduce");
    let mut q = p.clone();
    q.certificate = None;
    if let StageKind::AllReduce { combine, .. } = &mut q.stages[at].kind {
        if let ComputeOp::Generic { body, .. } = combine.as_mut() {
            *body = TensorBody::Operand(0);
        }
    }
    match certify_choreography(&q) {
        Err(CertifyFault::UnreducedShard { .. }) => {}
        other => panic!("an all-reduce folding one chip's partial of two must be refused `UnreducedShard`, not {:?}", other.map(|_| "a certificate")),
    }
}

/// ── 4. EVERY CHIP HOLDS EXACTLY ITS SLICES, and over the chips they tile every full tensor exactly once — the
/// replicated KV heads `tp/kv` times.
#[test]
fn every_chip_holds_exactly_its_slices() {
    for (what, c, tp) in configs() {
        let (_, m, p) = lowered(&c, tp);
        for chip in 0..tp as u16 {
            let read: BTreeSet<String> = p
                .stages
                .iter()
                .filter(|s| s.site.map(|x| x.chip) == Some(chip))
                .filter_map(|s| match &s.kind {
                    StageKind::Load { tensor, .. } => Some(p.tensor(*tensor).name.clone()),
                    _ => None,
                })
                .filter(|n| m.slices.contains_key(n))
                .collect();
            let own: BTreeSet<String> = m
                .slices
                .keys()
                .filter(|n| m.chip_of[*n] == chip)
                .cloned()
                .collect();
            assert_eq!(
                read, own,
                "{what}: chip {chip} reads exactly its own slices"
            );
        }
        let mut covered: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for sl in m.slices.values() {
            let e = covered
                .entry(sl.of.clone())
                .or_insert((0, sl.of_shape.0 * sl.of_shape.1));
            let (r, k) = sl.shape();
            e.0 += r * k;
        }
        let kv_copies = if c.kv % tp == 0 { 1 } else { tp / c.kv };
        for (of, (sum, whole)) in &covered {
            let attention_kv = ["Wk", "Wv", "Kc", "Vc"].iter().any(|b| {
                of.strip_prefix(b)
                    .and_then(|l| l.parse::<usize>().ok())
                    .is_some_and(|l| c.is_attn(l))
            });
            let copies = if attention_kv { kv_copies } else { 1 };
            assert_eq!(
                *sum,
                whole * copies,
                "{what}: the slices of `{of}` cover it {copies} time(s)"
            );
        }
    }
}

/// ── 5. THE CHIPS RUN AT ONCE: one op's per-chip slices in one phase, one section per chip per phase.
#[test]
fn the_chips_run_their_slices_at_once() {
    for (what, c, tp) in configs() {
        let (_, m, p) = lowered(&c, tp);
        let mut phase = Vec::with_capacity(p.sections.len());
        let mut at = 0usize;
        for sp in &p.sections {
            phase.push(at);
            if sp.barrier_after.is_some() {
                at += 1;
            }
        }
        let phases = at + 1;
        assert!(
            phases < p.sections.len(),
            "{what}: {} sections run in {phases} phases, not one after another",
            p.sections.len()
        );
        let mut chips_of_phase: BTreeMap<usize, Vec<u16>> = BTreeMap::new();
        for (k, _) in p.sections.iter().enumerate() {
            let chips: BTreeSet<u16> = p
                .stages
                .iter()
                .filter(|s| p.section_of_stage(s.id) == Some(k))
                .filter_map(|s| s.site.map(|x| x.chip))
                .collect();
            chips_of_phase.entry(phase[k]).or_default().extend(chips);
        }
        for (ph, chips) in &chips_of_phase {
            let distinct: BTreeSet<u16> = chips.iter().copied().collect();
            assert_eq!(
                distinct.len(),
                chips.len(),
                "{what}: phase {ph} runs one section per chip: {chips:?}"
            );
        }
        let storing = |name: &str| -> Option<usize> {
            p.stages.iter().find(|s| matches!(&s.kind, StageKind::Store { tensor, .. } if p.tensor(*tensor).name == name)).and_then(|s| p.section_of_stage(s.id))
        };
        let mut checked = 0;
        for name in m
            .chip_of
            .keys()
            .filter(|n| n.ends_with("_c0") && !m.slices.contains_key(*n))
        {
            let base = name.trim_end_matches("_c0");
            let sections: Vec<usize> = (0..tp)
                .filter_map(|chip| storing(&format!("{base}_c{chip}")))
                .collect();
            if sections.len() == tp {
                let ph: BTreeSet<usize> = sections.iter().map(|k| phase[*k]).collect();
                assert_eq!(
                    ph.len(),
                    1,
                    "{what}: every chip computes its slice of `{base}` in one phase: {ph:?}"
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 10,
            "{what}: the per-chip ops were found ({checked})"
        );
    }
}

/// The value of flat element `i` of the tensor a deployment binds as `bound`: a chip's slice from the full tensor it
/// cuts (its elements row-major in the slice's own shape, however a lowering lays the tensor out).
fn bound_value(m: &TpManifest, bound: &str, i: usize) -> f64 {
    match m.slices.get(bound) {
        Some(s) => {
            let cols = s.shape().1;
            fill_value(&s.of, s.full_index(i / cols, i % cols))
        }
        None => fill_value(bound, i),
    }
}

/// ── 6. THE TENSOR-PARALLEL DECODE BUNDLE, run program by program, emits the VM's token.
#[test]
fn one_decode_step_from_the_tensor_parallel_bundle_is_the_vms() {
    let (c, tp) = (four_chips(), 4);
    let (step, _, _, whole) = gen_qwen38_tp_decode_step_lg(&c, "/w", CTX, TOK, POS, tp);
    // The model's own tensors — every one the step declares; the compiler's (scalers, row tables) keep their values.
    let declared: BTreeSet<String> = step
        .lines()
        .filter_map(|l| {
            l.strip_prefix("Let ")
                .and_then(|r| r.split_once(" be a "))
                .map(|(v, _)| v.to_string())
        })
        .collect();
    let fill = |src: &str| with_fills(src, Some(&whole));
    let m =
        lower_tp_decode_streamed(&c.spec(), Storage::AllQ8, CTX, tp, &RduChip::Sn50.facts().spatial_target(), fill)
            .expect("the tensor-parallel decode bundle lowers");
    for k in &m.kinds {
        let chips: BTreeSet<u16> = k
            .program
            .stages
            .iter()
            .filter_map(|s| s.site.map(|x| x.chip))
            .collect();
        assert_eq!(
            chips.len(),
            tp,
            "the {} kind runs over all {tp} chips",
            k.name
        );
        assert!(k.tp.is_some(), "the {} kind carries its manifest", k.name);
    }
    let run = |p: &SpatialProgram,
               bind: &dyn Fn(&str) -> String,
               carried: &[(String, Vec<f64>)]|
     -> (
        logicaffeine_compile::spatial::eval::Outputs,
        Vec<(String, TensorId)>,
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
                _ if declared.contains(&bound) => {
                    (0..n).map(|i| bound_value(&whole, &bound, i)).collect()
                }
                _ => continue,
            };
            inputs.tensors.insert(t.id, Value::Float(v));
        }
        let out = eval_choreography_every_tensor(p, &inputs)
            .unwrap_or_else(|e| panic!("{}: evaluates: {e:?}", p.name));
        (
            out,
            p.tensors.iter().map(|t| (bind(&t.name), t.id)).collect(),
        )
    };
    let result = |out: &logicaffeine_compile::spatial::eval::Outputs,
                  names: &[(String, TensorId)],
                  name: &str|
     -> Vec<f64> {
        let id = names
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("`{name}` is in the program"))
            .1;
        match out
            .tensors
            .get(&id)
            .unwrap_or_else(|| panic!("`{name}` is a result"))
        {
            Value::Float(v) => v.clone(),
            Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
        }
    };
    let (embed_out, embed_names) = run(
        m.embed.as_ref().expect("an embedding program"),
        &|n: &str| n.to_string(),
        &[],
    );
    let mut hidden = ("x".to_string(), result(&embed_out, &embed_names, "x"));
    for b in &m.layers {
        let map = b.tensors.clone();
        let bind = move |name: &str| {
            map.iter()
                .find(|(from, _)| from == name)
                .map(|(_, to)| to.clone())
                .unwrap_or_else(|| name.to_string())
        };
        let (out, names) = run(
            &m.kinds[b.kind].program,
            &bind,
            std::slice::from_ref(&hidden),
        );
        let next = format!("xo{}", b.layer);
        hidden = (next.clone(), result(&out, &names, &next));
    }
    let (head_out, head_names) = run(
        &m.head,
        &|n: &str| n.to_string(),
        std::slice::from_ref(&hidden),
    );
    let src = with_fills(&gen_qwen38_decode_step_lg(&c, "/w", CTX, TOK, POS).0, None);
    let vm = vm_numbers(&format!("{src}Show next.\n"));
    assert_eq!(
        result(&head_out, &head_names, "next"),
        vec![vm[0]],
        "the tensor-parallel bundle emits the VM's token"
    );
}

/// `(tok, pos)` per sequence of the batched tensor-parallel step.
const SEQS: [(usize, usize); 3] = [(5, 3), (2, 1), (7, 6)];

/// ── 7. BATCHED AND TENSOR-PARALLEL: the batched step on the VM, one certified program over the chips emitting every
/// sequence's token, every weight slice read by one section for the whole batch.
#[test]
fn a_batched_tensor_parallel_step_is_the_batched_step() {
    let (c, tp) = (four_chips(), 4);
    let (src, _, _, m) = gen_qwen38_decode_batch_tp_lg(&c, "/w", CTX, &SEQS, tp);
    let src = with_fills(&src, Some(&m));
    let whole = with_fills(&gen_qwen38_decode_batch_lg(&c, "/w", CTX, &SEQS).0, None);
    let mut shown = String::new();
    for b in 0..SEQS.len() {
        shown.push_str(&format!("Show next_s{b}.\n"));
        for i in 1..=c.vocab {
            shown.push_str(&format!("Show item {i} of logits_s{b}.\n"));
        }
    }
    let (split, want) = (
        vm_numbers(&format!("{src}{shown}")),
        vm_numbers(&format!("{whole}{shown}")),
    );
    let scale = want.iter().fold(0.0f64, |a, x| a.max(x.abs()));
    for (k, (s, w)) in split.iter().zip(&want).enumerate() {
        assert!(
            (s - w).abs() <= 1e-9 * scale,
            "value {k}: batched tensor-parallel {s} vs batched {w}"
        );
    }

    let pl = compile_program_to_spatial_tp(&src, &RduChip::Sn50.facts().spatial_target(), &m)
        .expect("the walk lifts it");
    let p = pl
        .whole()
        .unwrap_or_else(|| {
            panic!(
                "the batched tensor-parallel step lowers whole: {}",
                pl.refusal()
            )
        })
        .clone();
    assert!(
        p.certificate.as_ref().is_some_and(|cert| cert.recheck(&p)),
        "it certifies"
    );
    let chips: BTreeSet<u16> = p
        .stages
        .iter()
        .filter_map(|s| s.site.map(|x| x.chip))
        .collect();
    assert_eq!(chips.len(), tp, "it runs on all {tp} chips");
    let out = eval_choreography_every_tensor(&p, &inputs_from_fills(&p)).expect("it evaluates");
    let per_seq = 1 + c.vocab;
    for b in 0..SEQS.len() {
        let id = p
            .tensors
            .iter()
            .find(|t| t.name == format!("next_s{b}"))
            .expect("the token")
            .id;
        let got = match out.tensors.get(&id).expect("written") {
            Value::Float(v) => v[0],
            Value::Int(v) => v[0] as f64,
        };
        assert_eq!(
            got,
            want[b * per_seq],
            "sequence {b}: the chips emit the VM's token"
        );
    }
    let mut checked = 0;
    for (name, chip) in m.chip_of.iter().filter(|(n, _)| {
        ["Wq", "Wk", "Wv", "Wo", "Wg", "Wu", "Wd"]
            .iter()
            .any(|w| n.starts_with(w))
            && m.slices.contains_key(*n)
    }) {
        let Some(t) = p.tensors.iter().find(|t| t.name == *name) else {
            continue;
        };
        let sections: BTreeSet<Option<usize>> = p
            .stages
            .iter()
            .filter(|s| matches!(&s.kind, StageKind::Load { tensor, .. } if *tensor == t.id))
            .map(|s| p.section_of_stage(s.id))
            .collect();
        assert_eq!(
            sections.len(),
            1,
            "`{name}` is read by one section on chip {chip} for all {} sequences",
            SEQS.len()
        );
        checked += 1;
    }
    assert!(
        checked >= 7 * tp,
        "the chips' weight slices were found ({checked})"
    );
}

/// Qwen3.8-27B's heads at the miniature's widths — 24 query heads over 4 KV heads, 16/48 Gated-DeltaNet heads — and an
/// FFN of 24 columns: over 16 chips none of them splits evenly.
fn real_heads() -> Qwen38Cfg {
    Qwen38Cfg {
        h: 24,
        kv: 4,
        gdn_k_heads: 16,
        gdn_v_heads: 48,
        dff: 24,
        ..Qwen38Cfg::tiny()
    }
}

/// The widths of a rack: the real model's heads, one KV head every chip reads, and fewer Gated-DeltaNet heads than
/// chips (most chips' share of those layers is nothing).
fn uneven() -> [(&'static str, Qwen38Cfg); 3] {
    [
        ("Qwen3.8-27B's heads", real_heads()),
        ("one KV head on every chip", Qwen38Cfg { kv: 1, ..real_heads() }),
        ("fewer Gated-DeltaNet heads than chips", Qwen38Cfg { gdn_k_heads: 2, gdn_v_heads: 4, ..real_heads() }),
    ]
}

const RACK: usize = 16;

/// The units — heads of `width` columns (or rows), FFN columns at width 1 — chip `chip` holds of the full tensor `of`.
fn units_of(m: &TpManifest, of: &str, chip: usize, width: usize) -> Vec<usize> {
    let mut units: Vec<usize> = m
        .slices
        .iter()
        .filter(|(n, s)| s.of == of && m.chip_of[*n] as usize == chip)
        .flat_map(|(n, s)| {
            s.ranges.iter().flat_map(move |(start, len)| {
                assert!(start % width == 0 && len % width == 0, "`{n}` holds whole units of {width} of `{of}`: {:?}", s.ranges);
                start / width..(start + len) / width
            })
        })
        .collect();
    units.sort_unstable();
    units
}

/// Every unit of `of` on exactly one chip, each chip `⌊n/tp⌋` or `⌈n/tp⌉` of them when `balanced`; returns each chip's.
fn tiled_once(what: &str, m: &TpManifest, of: &str, n: usize, width: usize, balanced: bool) -> Vec<Vec<usize>> {
    let per_chip: Vec<Vec<usize>> = (0..m.chips).map(|c| units_of(m, of, c, width)).collect();
    let mut all: Vec<usize> = per_chip.iter().flatten().copied().collect();
    all.sort_unstable();
    assert_eq!(all, (0..n).collect::<Vec<_>>(), "{what}: every unit of `{of}` is on exactly one chip: {per_chip:?}");
    if balanced {
        let (lo, hi) = (n / m.chips, n.div_ceil(m.chips));
        for (c, u) in per_chip.iter().enumerate() {
            assert!((lo..=hi).contains(&u.len()), "{what}: chip {c} holds {} of `{of}`'s {n}, not {lo} or {hi}", u.len());
        }
    }
    per_chip
}

/// ── 8. A WIDTH THE HEADS DO NOT DIVIDE (MULTI-4): every chip of a 16-chip rack holds its share of one model — the query
/// heads ⌊24/16⌋ or ⌈24/16⌉ a chip, each with the KV heads it reads, the Gated-DeltaNet value heads with the key heads
/// they read, the FFN's columns — every head on one chip; a chip with no share of a layer contributes a zero partial
/// to its all-reduce. On the VM the step is the unsharded step's; lowered, it is one certified program over the 16 chips
/// that emits the VM's token.
#[test]
fn twenty_four_heads_shard_over_sixteen_chips() {
    for (what, c) in uneven() {
        let (src, _, _, m) = gen_qwen38_tp_decode_step_lg(&c, "/w", CTX, TOK, POS, RACK);
        assert_eq!(m.chips, RACK);
        let (group, hd, ghd) = (c.h / c.kv, c.hd, c.gdn_head_dim);
        for l in 0..c.layers {
            if c.is_attn(l) {
                let q = tiled_once(what, &m, &format!("Wq{l}"), c.h, hd, true);
                assert_eq!(tiled_once(what, &m, &format!("Wqg{l}"), c.h, hd, true), q, "{what}: the gate's heads are the query's");
                assert_eq!(tiled_once(what, &m, &format!("Wo{l}"), c.h, hd, true), q, "{what}: the output rows are the query heads'");
                for (chip, heads) in q.iter().enumerate() {
                    let mut kv: Vec<usize> = heads.iter().map(|h| h / group).collect();
                    kv.dedup();
                    for of in [format!("Wk{l}"), format!("Wv{l}"), format!("Kc{l}"), format!("Vc{l}")] {
                        assert_eq!(units_of(&m, &of, chip, hd), kv, "{what}: chip {chip}'s `{of}` is the KV heads its query heads {heads:?} read");
                    }
                }
            } else {
                let v = tiled_once(what, &m, &format!("Wv{l}"), c.gdn_v_heads, ghd, c.gdn_v_heads >= RACK);
                for (chip, heads) in v.iter().enumerate() {
                    let mut k: Vec<usize> = heads.iter().map(|h| h % c.gdn_k_heads).collect();
                    k.sort_unstable();
                    k.dedup();
                    assert_eq!(units_of(&m, &format!("Wq{l}"), chip, ghd), k, "{what}: chip {chip}'s key heads are those its value heads {heads:?} read");
                    assert_eq!(units_of(&m, &format!("St{l}"), chip, ghd), *heads, "{what}: chip {chip} carries its value heads' memory");
                }
            }
            let f = tiled_once(what, &m, &format!("Wg{l}"), c.dff, 1, true);
            assert_eq!(tiled_once(what, &m, &format!("Wd{l}"), c.dff, 1, true), f, "{what}: the down rows are the gate's columns");
        }
        assert_eq!(m.collectives.len(), 2 * c.layers, "{what}: two all-reduces a layer");
        for col in &m.collectives {
            let chips: Vec<usize> = col.partials.iter().map(|p| m.chip_of[p] as usize).collect();
            assert_eq!(chips, (0..RACK).collect::<Vec<_>>(), "{what}: `{}` folds one partial from every chip, in chip order", col.out);
        }

        let src = with_fills(&src, Some(&m));
        let whole = vm_numbers(&format!("{}{}", with_fills(&gen_qwen38_decode_step_lg(&c, "/w", CTX, TOK, POS).0, None), shown(&c)));
        let split = vm_numbers(&format!("{src}{}", shown(&c)));
        assert_eq!(split.len(), whole.len(), "{what}: the same values are shown");
        assert_eq!(split[0], whole[0], "{what}: the step's token");
        let scale = whole.iter().fold(0.0f64, |a, x| a.max(x.abs()));
        for (k, (s, w)) in split.iter().zip(&whole).enumerate() {
            assert!((s - w).abs() <= 1e-9 * scale, "{what}: value {k}: over 16 chips {s} vs whole {w}");
        }

        let pl = compile_program_to_spatial_tp(&src, &RduChip::Sn50.facts().spatial_target(), &m).expect("the walk lifts it");
        let p = pl.whole().unwrap_or_else(|| panic!("{what}: lowers whole over 16 chips: {}", pl.refusal())).clone();
        assert!(p.certificate.as_ref().is_some_and(|cert| cert.recheck(&p)), "{what}: the program certifies");
        let chips: BTreeSet<u16> = p.stages.iter().filter_map(|s| s.site.map(|x| x.chip)).collect();
        assert_eq!(chips, (0..RACK as u16).collect(), "{what}: the program runs on all 16 chips");
        let out = eval_choreography_every_tensor(&p, &inputs_from_fills(&p)).unwrap_or_else(|e| panic!("{what}: evaluates: {e:?}"));
        let value = |name: &str| -> Vec<f64> {
            let id = p.tensors.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("{what}: `{name}`")).id;
            match out.tensors.get(&id).unwrap_or_else(|| panic!("{what}: `{name}` is written")) {
                Value::Float(v) => v.clone(),
                Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
            }
        };
        assert_eq!(value("next"), vec![whole[0]], "{what}: the 16 chips emit the VM's token");
        let logits = value("logits");
        let top = whole[1..=c.vocab].iter().fold(0.0f64, |a, x| a.max(x.abs()));
        let drift = logits.iter().zip(&whole[1..=c.vocab]).map(|(a, b)| (a - b).abs() / top).fold(0.0, f64::max);
        assert!(drift <= BF16_WHOLE_FORWARD_REL, "{what}: the logits drift {drift:.4} from the VM");
    }
}

/// ── 8. …and batched: the serving shape over the 16 chips is the batched step on the VM, and lowered it emits every
/// sequence's token. Two layers, a Gated-DeltaNet layer and an attention layer: the VM runs the whole batched program in
/// one frame, and sixteen chips' work for every sequence of four layers is past its registers.
#[test]
fn twenty_four_heads_shard_over_sixteen_chips_in_a_batch() {
    for (what, c) in uneven() {
        let c = Qwen38Cfg { layers: 2, full_attn_every: 2, ..c };
        let (src, _, _, m) = gen_qwen38_decode_batch_tp_lg(&c, "/w", CTX, &SEQS, RACK);
        let src = with_fills(&src, Some(&m));
        let whole = with_fills(&gen_qwen38_decode_batch_lg(&c, "/w", CTX, &SEQS).0, None);
        let mut shown = String::new();
        for b in 0..SEQS.len() {
            shown.push_str(&format!("Show next_s{b}.\n"));
            for i in 1..=c.vocab {
                shown.push_str(&format!("Show item {i} of logits_s{b}.\n"));
            }
        }
        let (split, want) = (vm_numbers(&format!("{src}{shown}")), vm_numbers(&format!("{whole}{shown}")));
        assert_eq!(split.len(), want.len(), "{what}: the same values are shown");
        let scale = want.iter().fold(0.0f64, |a, x| a.max(x.abs()));
        for (k, (s, w)) in split.iter().zip(&want).enumerate() {
            assert!((s - w).abs() <= 1e-9 * scale, "{what}: value {k}: batched over 16 chips {s} vs batched {w}");
        }
        let pl = compile_program_to_spatial_tp(&src, &RduChip::Sn50.facts().spatial_target(), &m).expect("the walk lifts it");
        let p = pl.whole().unwrap_or_else(|| panic!("{what}: the batch lowers whole over 16 chips: {}", pl.refusal())).clone();
        assert!(p.certificate.as_ref().is_some_and(|cert| cert.recheck(&p)), "{what}: it certifies");
        let out = eval_choreography_every_tensor(&p, &inputs_from_fills(&p)).unwrap_or_else(|e| panic!("{what}: evaluates: {e:?}"));
        for b in 0..SEQS.len() {
            let id = p.tensors.iter().find(|t| t.name == format!("next_s{b}")).expect("the token").id;
            let got = match out.tensors.get(&id).expect("written") {
                Value::Float(v) => v[0],
                Value::Int(v) => v[0] as f64,
            };
            assert_eq!(got, want[b * (1 + c.vocab)], "{what}: sequence {b}: the 16 chips emit the VM's token");
        }
    }
}

/// ── 8. …and the optimizer's fusions keep to one chip (`TensorParallelShim`): a fused op over a chip's op and another
/// chip's, or a chip's and the replicated spine's, or one holding a collective's fold, is refused — whatever the chip
/// beneath allows; a fusion within one chip, or within the spine, is the chip beneath's to answer.
#[test]
fn a_fusion_never_spans_two_chips_nor_holds_a_fold() {
    let c = Qwen38Cfg { gdn_k_heads: 2, gdn_v_heads: 4, ..real_heads() };
    let (src, _, _, m) = gen_qwen38_tp_decode_step_lg(&c, "/w", CTX, TOK, POS, RACK);
    let lifted = TensorGraph::lift_source(&src, &|_| GraphStorage::Values).expect("the walk lifts it");
    let g = &lifted.graph;
    let op = |out: &str| g.ops.iter().find(|o| o.out == out).unwrap_or_else(|| panic!("`{out}` is an op of the program")).clone();
    let shim = TensorParallelShim { base: &ReferenceShim, manifest: &m };
    let fits = |outs: &[&str]| {
        let parts: Vec<Op> = outs.iter().map(|o| op(o)).collect();
        let last = parts.last().expect("a member").clone();
        let ins: Vec<String> = parts.iter().flat_map(|p| p.ins.clone()).collect();
        shim.fits(&candidate(ClassId::ElementwiseRegions, parts, ins, &last.out, last.dst.clone()), g)
    };
    // Chip 5 holds no Gated-DeltaNet head of layer 0 (its partial is a zero) and two of the FFN's 24 columns.
    for (outs, why) in [
        (&["ao0_c5", "xr0"][..], "a chip's op and the spine's"),
        (&["gt0_c5", "gt0_c6"][..], "two chips' ops"),
        (&["ao0_f1", "xr0"][..], "a collective's fold"),
    ] {
        assert!(fits(outs).is_err(), "{why} ({outs:?}) are never one op");
    }
    for (outs, why) in [(&["gt0_c5", "up0_c5"][..], "two ops of one chip"), (&["xr0", "x2n0"][..], "two ops of the spine")] {
        assert_eq!(fits(outs), Ok(()), "{why} ({outs:?}) fuse as the chip beneath allows");
    }
}
