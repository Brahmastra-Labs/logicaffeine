//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE CERTIFICATE'S DRY RUN IS WINDOWED, AND WHAT IT CANNOT SEE IT MUST NOT REFUSE.
//!
//! `certify_choreography` runs the choreography token-only, and walks each site's work-item loop only as far
//! as `item_window` (the deepest FIFO plus two): the token protocol is periodic past that window, so the
//! window proves it for every round. The window is sound for TOKENS. It says nothing about DATA a later
//! section reads: when a section places more items on a site than the window, the items past it never run in
//! the dry run, and the activation tiles they write are never written there.
//!
//! A later section reading one of those tiles is not a fault of the program — the full evaluator runs every
//! item and writes it — and refusing it refused real programs: Qwen3.8's masked attention at a 4096-position
//! cache (1024 blocks, a five-level merge tree, ~6 merge items per site) was refused with `ReadOutOfRange` on a
//! partial the elided items write. So the dry run records which tensors had writes elided by the window, and an
//! in-range tile of exactly those is not provably absent. Read-before-write ACROSS sections is proved elsewhere
//! and structurally: the ordering obligations and the rendezvous placement the IR validates.
//!
//! Both halves are pinned: a writer past the window certifies and the full evaluator agrees with the VM; a writer
//! the window covers still faults on a premature read in the dry run, and the same premature read against an
//! elided writer is refused by the IR before any run.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_windowed_dry_run_lock`
//!
//! ★ MUTATION PROBES (run when this landed): never relax (`Some(None) if false` in `eval.rs`'s `ReadTiles`) —
//! both elided-writer tests red with the spurious `ReadOutOfRange` on `Y`'s tile 4, the first the window of 4
//! elides. Always relax (every store marked elided) — the fully-run-writer control reds: the presence check is
//! dead exactly where it still proves something.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::spatial::certificate::{certify_choreography, item_window};
use logicaffeine_compile::spatial::eval::{eval_choreography, eval_choreography_dry, ChoreoFault};
use logicaffeine_compile::spatial::ir::{IrViolation, SpatialProgram, TaggedOp};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::target::{MeshShape, SpatialTarget};

#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::inputs_from_fills;

/// The SN50 descriptor on ONE site, so every work item of a section lands on it.
fn one_site() -> SpatialTarget {
    let mut t = RduChip::Sn50.facts().spatial_target();
    t.sites = MeshShape { x: 1, y: 1 };
    t
}

/// Bf16-exact fills: small integers over 64.
fn x_at(i: usize) -> f64 {
    (((i * 37) % 23) as f64 - 11.0) / 64.0
}
fn w_at(i: usize) -> f64 {
    (((i * 11) % 19) as f64 - 9.0) / 64.0
}

/// A row-wide elementwise section writing `Y` one tile per item — the shape of every activation in the forward —
/// then a matmul whose item reads the whole row of `Y`: tiles written by items past the writer's window as well
/// as before it.
fn chain(k: usize) -> String {
    let mut s = format!(
        "## Main\nLet X be a 1 by {k} tensor of Real.\nLet W be a {k} by 32 tensor of Real.\n"
    );
    for i in 1..=k {
        s.push_str(&format!("Set item {i} of X to {:?}.\n", x_at(i)));
    }
    for i in 1..=32 * k {
        s.push_str(&format!("Set item {i} of W to {:?}.\n", w_at(i)));
    }
    s.push_str("Let Y be sigmoid of X.\nLet Z be Y matmul W.\n");
    s
}

fn composed(k: usize) -> SpatialProgram {
    let l = compile_program_to_spatial(&chain(k), &one_site()).expect("a tensor program");
    l.program
        .unwrap_or_else(|| panic!("the chain lowers whole and certifies: {:?}", l.missing))
}

/// Items of section `k` on the one site.
fn items_in(p: &SpatialProgram, k: usize) -> usize {
    let (a, b) = p.sections[k].items;
    b - a
}

/// ── A WRITER PAST THE WINDOW CERTIFIES, and the full run agrees with the VM.
#[test]
fn a_reader_of_tiles_the_window_elided_is_not_refused() {
    let p = composed(512);
    let window = item_window(&p);
    assert!(
        items_in(&p, 0) > window,
        "the precondition: the writer places {} items on its one site, past a window of {window}",
        items_in(&p, 0)
    );
    let mut q = p.clone();
    q.certificate = None;
    certify_choreography(&q).expect("the composed chain certifies");
    let out =
        eval_choreography(&p, &inputs_from_fills(&p)).expect("the full run executes every item");
    let z = p.tensors.iter().find(|t| t.name == "Z").expect("Z").id;
    let got = match out.tensors.get(&z).expect("Z is written") {
        logicaffeine_compile::spatial::eval::Value::Float(v) => v.clone(),
        other => panic!("Z is float, got {other:?}"),
    };
    // The f64 reference the VM computes, and the bf16 bound: (sections + 1) ulps of the magnitude Σ|y·w|.
    let k = 512;
    let ulp = 2f64.powi(-8);
    let sections = p.sections.len() as f64;
    let mut worst = 0.0f64;
    for c in 0..32 {
        let (mut want, mut mag) = (0.0, 0.0);
        for j in 0..k {
            let y = 1.0 / (1.0 + (-x_at(j + 1)).exp());
            let w = w_at(j * 32 + c + 1);
            want += y * w;
            mag += (y * w).abs();
        }
        let err = (got[c] - want).abs();
        assert!(
            err <= (sections + 1.0) * ulp * mag,
            "Z[{c}]: {} vs {want} (bound {})",
            got[c],
            (sections + 1.0) * ulp * mag
        );
        worst = worst.max(err / mag.max(f64::MIN_POSITIVE));
    }
    println!("Z within {worst:.2e} of the f64 reference, relative to its magnitude");
}

/// Hoist every program's ops of section 1 above everything else it runs: the reader reads `Y` before any
/// section-0 op of any program — so before `Y`'s writer — whatever order the programs are scheduled in.
fn next_section_hoisted(p: &SpatialProgram) -> SpatialProgram {
    let mut q = p.clone();
    q.certificate = None;
    let sections = q.sections.clone();
    for prog in &mut q.schedule.as_mut().expect("scheduled").programs {
        let (next, rest): (Vec<TaggedOp>, Vec<TaggedOp>) = prog
            .ops
            .drain(..)
            .partition(|t| sections.iter().position(|s| s.holds_stage(t.stage)) == Some(1));
        prog.ops = next.into_iter().chain(rest).collect();
    }
    q
}

/// ── AND A WRITER THE WINDOW COVERS STILL FAULTS on a premature read — the presence check is only relaxed for
/// tensors the window elided.
#[test]
fn a_premature_read_of_a_fully_run_writer_still_faults_in_the_dry_run() {
    let p = composed(32);
    assert!(
        items_in(&p, 0) <= item_window(&p),
        "the precondition: the writer runs in full inside the window"
    );
    let q = next_section_hoisted(&p);
    match eval_choreography_dry(&q) {
        Err(ChoreoFault::ReadOutOfRange { .. }) => {}
        other => panic!("reading Y before its writer ran must fault in the dry run, got {other:?}"),
    }
}

/// ── AND AGAINST AN ELIDED WRITER THE SAME TAMPER IS REFUSED BY THE IR, before any run.
#[test]
fn a_premature_read_of_an_elided_writer_is_refused_by_the_rendezvous_placement() {
    let p = composed(512);
    let q = next_section_hoisted(&p);
    match q.validate() {
        Err(IrViolation::SectionBarrierMisplaced { .. }) => {}
        other => panic!("next-section work above the rendezvous that admits it must be refused by placement, got {other:?}"),
    }
    assert!(
        certify_choreography(&q).is_err(),
        "and no certificate is issued"
    );
}
