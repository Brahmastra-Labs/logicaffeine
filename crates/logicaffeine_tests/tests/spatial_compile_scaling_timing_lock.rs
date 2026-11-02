//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE SPATIAL COMPILER IS LINEAR IN THE PROGRAM — every pass over a decode layer grows with the layer, not with its
//! square.
//!
//! A served decode layer is batched and tensor-parallel: at Qwen3.8-27B's real width, TP 8 and the batch the serving
//! plan picks, one Gated-DeltaNet layer composes 4,676 sections and 169k stages. A pass that answers a per-stage or
//! per-section question by scanning the whole program is quadratic in that — the certificate's ordering obligations
//! and liveness graphs, validation's coverage count, the phase placement, the composition, the dataflow links, the
//! evaluator's item lookup and the Spatial printer all ask such questions, and all read indexes
//! (`spatial_compile_index_lock` holds each index to its scan).
//!
//! Pinned, for the mini config's GDN layer at batch 4 and at batch 96 (26× the program): each pass's time grows by at
//! most `HEADROOM` times its `clean` ratio — the growth it shows here per unit growth of what it walks, measured on
//! this box (i9-14900K, debug build) — so a pass that turns quadratic outgrows it:
//!   1. the tensor-parallel layer (TP 4): lowering (which certifies), validation, certification, the certificate's
//!      dry run;
//!   2. the single-chip layer: lowering, and printing it as a Spatial app;
//!   3. the front-end passes too small to move the whole, timed on their own: the phase placement over the composed
//!      layer's sections, and the dataflow links over its recognized statements.
//!
//! The yardstick is a COUNT, not a time — the program's stages, buffers, edges, semaphores, sections, items and every
//! schedule op — so nothing but the pass is timed. The small and the large sample are taken back to back and last
//! about as long (the small input run as many times as the large is larger), the fastest of each over the rounds, so
//! load on the box lands on both alike. A `clean` ratio above 1 is the memory hierarchy (the
//! small program fits in cache, the large one does not) and ordered maps' logarithms, never a scan: on a machine with
//! smaller caches the ratios fall. A red here is re-run alone before it is believed.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_compile_scaling_timing_lock`
//!
//! ★ MUTATION PROBES (run 2026-09-25 on a quiet box, each in a private copy of the workspace, each red; ratio against
//! ceiling):
//!   - the certificate's graph context rebuilt for every section's liveness graph → 1, certify 2.92 (2.32).
//!   - the section of an id found by scanning the sections → 1, dry run 6.02 (3.00) — validate 3.34 stays just under
//!     its 3.44, which is why the dry run is timed on its own.
//!   - validation finding each tensor's loads and stores by scanning the stages → 1, validate 6.52 (3.44).
//!   - the printer walking every program's ops for each stage's K blocks → 2, print 2.64 (1.56).
//!   - the phase placement's dependences found pairwise → 3, 21.1 (2.72); its free phase found phase by phase → 3,
//!     8.84 (2.72).
//!   - the dataflow links found pairwise → 3, 17.1 (2.74).
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen38::{
    gen_qwen38_decode_batch_layer_lg, gen_qwen38_decode_batch_tp_layer_lg, Qwen38Cfg,
};
use logicaffeine_compile::spatial::certificate::certify_choreography;
use logicaffeine_compile::spatial::eval::eval_choreography_dry;
use logicaffeine_compile::spatial::ir::{ChoreoOp, SpatialProgram, StageKind, TaggedOp};
use logicaffeine_compile::spatial::lower::spatial_lang::emit_spatial_program;
use logicaffeine_compile::spatial::passes::recognize_stages::{
    links, recognize_stages, RecognizedStage,
};
use logicaffeine_compile::spatial::pipeline::{
    compile_program_to_spatial, compile_program_to_spatial_tp,
};
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::tensor_parallel::{phase_order, SectionTouch, TpManifest};
use std::hint::black_box;
use std::time::{Duration, Instant};

/// How far past its clean ratio a pass may grow.
const HEADROOM: f64 = 2.0;
const SMALL: usize = 4;
const LARGE: usize = 96;
const ROUNDS: usize = 3;

/// A program's size, as the passes walk it: its stages, buffers, edges, semaphores, sections and work items, and every
/// op of its schedule, loop bodies included — not the tensors' fills, whose weights are the same size at every batch.
fn size(p: &SpatialProgram) -> usize {
    fn ops(ts: &[TaggedOp]) -> usize {
        ts.iter()
            .map(|t| {
                1 + if let ChoreoOp::Loop { body, .. } = &t.op {
                    ops(body)
                } else {
                    0
                }
            })
            .sum()
    }
    let schedule = p
        .schedule
        .as_ref()
        .map_or(0, |s| s.programs.iter().map(|q| ops(&q.ops)).sum());
    p.stages.len()
        + p.buffers.len()
        + p.edges.len()
        + p.semaphores.len()
        + p.sections.len()
        + p.work.items.len()
        + schedule
}

/// Time a pass over the small and the large input — the fastest of `ROUNDS` samples of each, taken back to back — and
/// hold its growth per unit growth of `sizes` to `HEADROOM` times `clean`.
///
/// A sample of the small input runs it as many times as the large input is larger, and counts one run as its time: the
/// two samples then last about as long, so contention on the box lands on both alike. A single short run is far likelier
/// to fall in a quiet moment than a long one, which reads load as superlinear growth.
fn growth(
    what: &str,
    clean: f64,
    sizes: (usize, usize),
    mut small: impl FnMut(),
    mut large: impl FnMut(),
    table: &mut String,
) {
    let reps = (sizes.1 / sizes.0.max(1)).max(1);
    let (mut s, mut l) = (Duration::MAX, Duration::MAX);
    for _ in 0..ROUNDS {
        let t = Instant::now();
        for _ in 0..reps {
            small();
        }
        s = s.min(t.elapsed() / reps as u32);
        let t = Instant::now();
        large();
        l = l.min(t.elapsed());
    }
    let (g_size, g_pass) = (
        sizes.1 as f64 / sizes.0 as f64,
        l.as_secs_f64() / s.as_secs_f64().max(1e-9),
    );
    let ratio = g_pass / g_size;
    table.push_str(&format!("  {what:<44} {s:>11.2?} {l:>11.2?} {g_pass:>7.1}x   size {g_size:>5.1}x  {ratio:>5.2} (clean {clean:.2})\n"));
    assert!(
        ratio <= HEADROOM * clean,
        "{what}: batch {SMALL} → {LARGE} grew {g_pass:.1}× where what it walks grew {g_size:.1}× — {ratio:.2} per unit, past \
         {HEADROOM}× its clean {clean:.2}: a pass scans the program per stage again (re-run alone before believing it){table}"
    );
}

fn header() -> String {
    format!("\n  pass                                         batch {SMALL:<5}   batch {LARGE:<4}  growth        size  ratio\n")
}

fn tp_layer(batch: usize) -> (String, TpManifest) {
    let (src, _, _, m) =
        gen_qwen38_decode_batch_tp_layer_lg(&Qwen38Cfg::mini(), 0, "/w", 6, &vec![1; batch], 4);
    (src, m)
}

fn lower_tp((src, m): &(String, TpManifest)) -> SpatialProgram {
    compile_program_to_spatial_tp(src, &RduChip::Sn50.facts().spatial_target(), m)
        .expect("the layer lowers")
        .into_whole()
        .expect("every statement lowers")
}

// ───────────────────────────── 1 ─────────────────────────────

#[test]
fn a_tensor_parallel_layer_lowers_validates_and_certifies_in_linear_time() {
    let (small, large) = (tp_layer(SMALL), tp_layer(LARGE));
    let (p, q) = (lower_tp(&small), lower_tp(&large));
    assert!(
        q.sections.len() >= 20 * p.sections.len(),
        "the large layer is the larger program ({} vs {} sections)",
        q.sections.len(),
        p.sections.len()
    );
    let uncertified = |x: &SpatialProgram| SpatialProgram {
        certificate: None,
        ..x.clone()
    };
    let (pu, qu) = (uncertified(&p), uncertified(&q));
    let sizes = (size(&p), size(&q));
    let mut table = header();
    growth(
        "lower (tensor-parallel, certification incl.)",
        0.97,
        sizes,
        || drop(black_box(lower_tp(&small))),
        || drop(black_box(lower_tp(&large))),
        &mut table,
    );
    growth(
        "validate",
        1.72,
        sizes,
        || p.validate().expect("valid"),
        || q.validate().expect("valid"),
        &mut table,
    );
    growth(
        "certify",
        1.16,
        sizes,
        || drop(black_box(certify_choreography(&pu).expect("certifies"))),
        || drop(black_box(certify_choreography(&qu).expect("certifies"))),
        &mut table,
    );
    growth(
        "dry run",
        1.50,
        sizes,
        || drop(black_box(eval_choreography_dry(&p).expect("runs"))),
        || drop(black_box(eval_choreography_dry(&q).expect("runs"))),
        &mut table,
    );
    println!("{table}");
}

// ───────────────────────────── 2 ─────────────────────────────

#[test]
fn a_single_chip_layer_lowers_and_prints_in_linear_time() {
    let target = RduChip::Sn50.facts().spatial_target();
    let layer =
        |b: usize| gen_qwen38_decode_batch_layer_lg(&Qwen38Cfg::mini(), 0, "/w", 6, &vec![1; b]).0;
    let (small, large) = (layer(SMALL), layer(LARGE));
    let lower = |src: &str| {
        compile_program_to_spatial(src, &target)
            .expect("the layer lowers")
            .into_whole()
            .expect("every statement lowers")
    };
    let (p, q) = (lower(&small), lower(&large));
    let sizes = (size(&p), size(&q));
    let mut table = header();
    growth(
        "lower (single-chip)",
        0.77,
        sizes,
        || drop(black_box(lower(&small))),
        || drop(black_box(lower(&large))),
        &mut table,
    );
    growth(
        "print as a Spatial app",
        0.78,
        sizes,
        || {
            drop(black_box(
                emit_spatial_program(&p, &target, None).expect("prints"),
            ))
        },
        || {
            drop(black_box(
                emit_spatial_program(&q, &target, None).expect("prints"),
            ))
        },
        &mut table,
    );
    println!("{table}");
}

// ───────────────────────────── 3 ─────────────────────────────

/// What each section of a composed program touches — the input the phase placement reads.
fn touches(p: &SpatialProgram) -> Vec<SectionTouch> {
    p.sections
        .iter()
        .map(|sp| {
            let mut t = SectionTouch::default();
            for s in &p.stages[sp.stages.0..sp.stages.1] {
                match &s.kind {
                    StageKind::Load { tensor, .. } => {
                        t.reads.insert(p.tensor(*tensor).name.clone());
                    }
                    StageKind::Store { tensor, .. } => {
                        t.writes.insert(p.tensor(*tensor).name.clone());
                    }
                    _ => {}
                }
                if let Some(site) = s.site {
                    t.chips.insert(site.chip);
                }
            }
            t
        })
        .collect()
}

#[test]
fn the_phase_placement_and_the_dataflow_links_grow_in_linear_time() {
    let (small, large) = (tp_layer(SMALL), tp_layer(LARGE));
    let (ts, tl) = (touches(&lower_tp(&small)), touches(&lower_tp(&large)));
    let (rs, rl) = (
        recognize_stages(&small.0).expect("walks").stages,
        recognize_stages(&large.0).expect("walks").stages,
    );
    assert!(
        tl.len() >= 20 * ts.len() && rl.len() >= 20 * rs.len(),
        "the large layer is the larger program"
    );
    let touched = |t: &[SectionTouch]| {
        t.iter()
            .map(|x| 1 + x.reads.len() + x.writes.len() + x.chips.len())
            .sum::<usize>()
    };
    let named = |r: &[RecognizedStage]| r.iter().map(|x| 1 + x.ins.len()).sum::<usize>();
    let mut table = header();
    growth(
        "phase placement",
        1.36,
        (touched(&ts), touched(&tl)),
        || drop(black_box(phase_order(&ts))),
        || drop(black_box(phase_order(&tl))),
        &mut table,
    );
    growth(
        "dataflow links",
        1.37,
        (named(&rs), named(&rl)),
        || drop(black_box(links(&rs))),
        || drop(black_box(links(&rl))),
        &mut table,
    );
    println!("{table}");
}
