//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A DECODE STEP FOLDS THE BLOCKS ITS POSITION REACHES — THE LOOP'S COUNT IS THE PROGRAM'S OWN DATUM.
//!
//! A decode step at position `P` attends the first `P` positions of a cache declared for the whole context. Folded
//! over a count fixed when the program is built, it streams all `⌈ctx/32⌉` tile rows of every K/V head and masks
//! away the rows past `P`: at a 4,096-position cache and `P = 1,000` three quarters of the K/V it reads contribute
//! nothing, and at a long context nearly all of it. So the fold's K-block loops run to a TRIP — a count the program
//! reads from one of its inputs before it runs, `⌈P / 32⌉` blocks, never more than the cache holds. It is the loop
//! bound a Plasticine counter chain takes from a scalar input, and the one a GPU decode kernel reads from its
//! sequence length.
//!
//! Pinned:
//!   1. At real width the fold's loops — the reader's and the compute's — all run to ONE trip read from `pos`:
//!      `⌈P/32⌉`, admitted from 1 to the cache's `⌈ctx/32⌉` tile rows. The static bound is the whole cache; the
//!      traffic at the position is that bound less exactly the blocks past it.
//!   2. At every position — the first, the last of a tile row, the first of the next, one inside a tail tile row, the
//!      whole cache — the fold computes the attention on the chip and on the fp32 twin, and reads exactly `⌈P/32⌉`
//!      blocks of each K/V head.
//!   3. Two sequences in one program fold to their own positions, each through its own trip.
//!   4. The certificate proves every count of the trip's window, section by section, and a certificate missing a
//!      count or naming another is refused.
//!   5. A deadlock only a longer trip reaches is refused — the reason the window exists — and the window's verdict
//!      is the verdict of proving every count one at a time.
//!   6. A position the trip does not admit faults by name before anything runs.
//!   7. A malformed trip, or a loop that misuses one, is refused by name.
//!   8. A count the program fixes stays fixed: the whole cache and a literal prefix fold to constant counts.
//!   9. Every printer prints the trip or refuses it by name; the RDU ships it.
//!
//! Also: a buffer balanced only at the greatest count is refused; a hand-built fold run to a trip sums exactly its first
//! `n` blocks; Tensix and the fabric lower the fixed fold and refuse the tripped one by name.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_trip_count_lock`
//!
//! ★ MUTATION PROBES (run 2026-09-27 in a private copy; logs `logs/sn50/probe_trip_*.txt`) — every one reds:
//!   • the fold never declares its trip → 12 of 15;
//!   • the evaluator runs every trip loop to its bound → 6 (the hand fold's sums; P = 17 reads 4 tiles of Kc, not 2);
//!   • the certificate proves only the least count → the window test (the bound's proof still catches the deadlock);
//!   • `recheck` stops matching the case count → a certificate missing a count passes: the window test;
//!   • a count outside the range is clamped instead of refused → P = 0 runs;
//!   • no per-round balance → the buffer balanced only at the bound validates;
//!   • a store allowed in a trip loop, or a trip read from any tensor → the malformed-trip test;
//!   • compose leaves a section's trip ids unshifted → the two-sequence program is refused (`a trip no loop runs to`);
//!   • `work_at` takes the bound → the real-width traffic test;
//!   • Tensix prints a trip loop at its bound → the printer test.
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::certificate::{
    certify_choreography, trip_window, CertifyFault, GraphProof,
};
use logicaffeine_compile::spatial::eval::{
    eval_choreography, eval_choreography_traced, trip_counts, ChoreoFault, Inputs, Outputs, Trace,
    Value,
};
use logicaffeine_compile::spatial::ir::{
    ChoreoOp, IrViolation, LoopCount, Phase, SemCond, SpatialProgram, TaggedOp, TensorRole, TripId,
};
use logicaffeine_compile::spatial::lower::{
    cuda::CudaPersistentBackend, fabric::FabricBackend, rdu::RduSpatialBackend,
    tensix::TensixBackend, SpatialArtifact, SpatialBackend, SpatialLowerError,
};
use logicaffeine_compile::spatial::passes::LV_KBLOCK;
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::target::{precision_bytes, ComputeCap, SpatialTarget};
use logicaffeine_compile::spatial::work::{work_at, work_of};

#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::inputs_from_fills;

fn sn50() -> SpatialTarget {
    RduChip::Sn50.facts().spatial_target()
}

/// The SN50 with fp32 datapaths: fp32 in place of bf16 on the matmul, fp32 beside it on the pointwise and row units.
fn fp32_twin() -> SpatialTarget {
    let mut t = sn50();
    for c in &mut t.compute {
        match c {
            ComputeCap::MatmulTile { precisions, .. } => {
                if precisions.remove(&Precision::Bf16) {
                    precisions.insert(Precision::Fp32);
                }
            }
            ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } => {
                if precisions.contains(&Precision::Bf16) {
                    precisions.insert(Precision::Fp32);
                }
            }
            _ => {}
        }
    }
    t.precisions.insert(Precision::Fp32);
    t
}

/// Bf16-exact fills: small multiples of 1/64, different per tensor.
fn fill(name: &str, i: usize) -> f64 {
    let salt = name.bytes().map(|b| b as usize).sum::<usize>();
    ((((i + 1) * 37 + salt * 11) % 29) as f64 - 14.0) / 64.0
}

/// One decode sequence: its cache of `ctx` positions, `h` query heads over `kv` K/V heads of `hd`, and the position
/// it appends its row at and attends up to. Every name it declares ends in `sfx`.
#[derive(Clone, Copy, Debug)]
struct Seq {
    sfx: &'static str,
    ctx: usize,
    h: usize,
    kv: usize,
    hd: usize,
    pos: usize,
}

impl Seq {
    fn new(ctx: usize, h: usize, kv: usize, hd: usize, pos: usize) -> Seq {
        Seq {
            sfx: "",
            ctx,
            h,
            kv,
            hd,
            pos,
        }
    }
    fn name(&self, base: &str) -> String {
        format!("{base}{}", self.sfx)
    }
}

/// The program of `seqs`: each appends its row of K and V at its position and attends up to it. `filled` sets every
/// tensor's elements (the numeric cases); unfilled programs are for structure and traffic.
fn source(seqs: &[Seq], filled: bool) -> String {
    let mut src = String::from("## Main\n");
    for s in seqs {
        let (aq, akv) = (s.h * s.hd, s.kv * s.hd);
        let n = |b: &str| s.name(b);
        src += &format!("Let {} be {}.\n", n("pos"), s.pos);
        src += &format!(
            "Let {} be a {} by {akv} tensor of Real.\nLet {} be a {} by {akv} tensor of Real.\n",
            n("Kc"),
            s.ctx,
            n("Vc"),
            s.ctx
        );
        src += &format!("Let {} be a 1 by {aq} tensor of Real.\nLet {} be a 1 by {akv} tensor of Real.\nLet {} be a 1 by {akv} tensor of Real.\n", n("qp"), n("kp"), n("vr"));
        if filled {
            for (base, count) in [
                ("Kc", s.ctx * akv),
                ("Vc", s.ctx * akv),
                ("qp", aq),
                ("kp", akv),
                ("vr", akv),
            ] {
                for i in 0..count {
                    src += &format!(
                        "Set item {} of {} to {:?}.\n",
                        i + 1,
                        n(base),
                        fill(&n(base), i)
                    );
                }
            }
        }
    }
    for s in seqs {
        let n = |b: &str| s.name(b);
        src += &format!(
            "Set row {pos} of {kc} to {kp}.\nSet row {pos} of {vc} to {vr}.\nLet {ct} be attention of {qp} over {kc} and {vc} with {h} heads {kv} groups up to {pos}.\n",
            pos = n("pos"),
            kc = n("Kc"),
            vc = n("Vc"),
            kp = n("kp"),
            vr = n("vr"),
            ct = n("ct"),
            qp = n("qp"),
            h = s.h,
            kv = s.kv
        );
    }
    src
}

/// The f64 attention of `s` at `pos`: row `pos` of each cache replaced by the appended row, each query head over the
/// first `pos` positions of its K/V head.
fn reference(s: &Seq, pos: usize) -> Vec<f64> {
    let akv = s.kv * s.hd;
    let mut k: Vec<f64> = (0..s.ctx * akv).map(|i| fill(&s.name("Kc"), i)).collect();
    let mut v: Vec<f64> = (0..s.ctx * akv).map(|i| fill(&s.name("Vc"), i)).collect();
    for c in 0..akv {
        k[(pos - 1) * akv + c] = fill(&s.name("kp"), c);
        v[(pos - 1) * akv + c] = fill(&s.name("vr"), c);
    }
    let group = s.h / s.kv;
    let mut ct = vec![0.0; s.h * s.hd];
    for head in 0..s.h {
        let x = head / group;
        let scores: Vec<f64> = (0..pos)
            .map(|j| {
                (0..s.hd)
                    .map(|e| fill(&s.name("qp"), head * s.hd + e) * k[j * akv + x * s.hd + e])
                    .sum::<f64>()
                    / (s.hd as f64).sqrt()
            })
            .collect();
        let m = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let w: Vec<f64> = scores.iter().map(|sc| (sc - m).exp()).collect();
        let sum: f64 = w.iter().sum();
        for e in 0..s.hd {
            ct[head * s.hd + e] = (0..pos)
                .map(|j| w[j] * v[j * akv + x * s.hd + e])
                .sum::<f64>()
                / sum;
        }
    }
    ct
}

fn composed(src: &str, target: &SpatialTarget) -> SpatialProgram {
    let l = compile_program_to_spatial(src, target).expect("a tensor program");
    let p = l
        .program
        .unwrap_or_else(|| panic!("the program composes and certifies: {:?}", l.missing));
    assert_eq!(p.phase(), Phase::Verified, "the composed program certifies");
    assert!(
        p.certificate.as_ref().is_some_and(|c| c.recheck(&p)),
        "its certificate rechecks"
    );
    p
}

fn tensor_named(p: &SpatialProgram, name: &str) -> logicaffeine_compile::spatial::ir::TensorId {
    p.tensors
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("a tensor `{name}`"))
        .id
}

/// The flash fold sections, in section order, each with the cache it streams (`Kc…`).
fn folds(p: &SpatialProgram) -> Vec<(usize, String)> {
    (0..p.sections.len())
        .filter(|k| p.sections[*k].name.contains("flash_blocks"))
        .map(|k| {
            let caches: BTreeSet<String> = p
                .stages
                .iter()
                .filter(|st| p.section_of_stage(st.id) == Some(k))
                .filter_map(|st| match &st.kind {
                    logicaffeine_compile::spatial::ir::StageKind::Load { tensor, .. } => {
                        Some(p.tensor(*tensor).name.clone())
                    }
                    _ => None,
                })
                .filter(|n| n.starts_with("Kc"))
                .collect();
            assert_eq!(
                caches.len(),
                1,
                "fold section {k} streams one K cache: {caches:?}"
            );
            (k, caches.into_iter().next().unwrap())
        })
        .collect()
}

fn the_fold(p: &SpatialProgram) -> usize {
    let f = folds(p);
    assert_eq!(
        f.len(),
        1,
        "one fold section: {:?}",
        p.sections.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    f[0].0
}

/// Every K-block loop count of `section`'s ops, across every engine program.
fn k_loops(p: &SpatialProgram, section: usize) -> Vec<LoopCount> {
    fn walk(p: &SpatialProgram, ops: &[TaggedOp], section: usize, out: &mut Vec<LoopCount>) {
        for t in ops {
            if let ChoreoOp::Loop { var, count, body } = &t.op {
                if *var == LV_KBLOCK && p.section_of_stage(t.stage) == Some(section) {
                    out.push(*count);
                }
                walk(p, body, section, out);
            }
        }
    }
    let mut out = Vec::new();
    for prog in &p.schedule.as_ref().expect("scheduled").programs {
        walk(p, &prog.ops, section, &mut out);
    }
    out
}

/// The one trip `section`'s K-block loops all run to, with the static bound every one carries.
fn fold_trip(p: &SpatialProgram, section: usize) -> (TripId, usize) {
    let loops = k_loops(p, section);
    assert!(
        !loops.is_empty(),
        "section {section} folds its blocks in a loop"
    );
    let trips: BTreeSet<(TripId, usize)> = loops
        .iter()
        .map(|c| match c {
            LoopCount::Trip { trip, max } => (*trip, *max),
            LoopCount::Const(n) => panic!("section {section}: a K-block loop fixed at {n} blocks — the fold must run to its position"),
        })
        .collect();
    assert_eq!(
        trips.len(),
        1,
        "section {section}: every K-block loop runs to the same trip and bound: {trips:?}"
    );
    trips.into_iter().next().unwrap()
}

/// Bytes of `tensor` the schedule reads inside `section`, each loop run `count.at(counts)` times.
fn read_in(p: &SpatialProgram, section: usize, tensor: &str, counts: &[usize]) -> u64 {
    fn walk(
        p: &SpatialProgram,
        ops: &[TaggedOp],
        mult: u64,
        section: usize,
        tensor: &str,
        counts: &[usize],
        n: &mut u64,
    ) {
        let tile = (p.tile.rows * p.tile.cols) as u64;
        for t in ops {
            match &t.op {
                ChoreoOp::Loop { count, body, .. } => walk(
                    p,
                    body,
                    mult * count.at(counts) as u64,
                    section,
                    tensor,
                    counts,
                    n,
                ),
                ChoreoOp::ReadTiles {
                    tensor: id,
                    block_h,
                    block_w,
                    ..
                } if p.tensor(*id).name == tensor
                    && p.section_of_stage(t.stage) == Some(section) =>
                {
                    *n += mult
                        * (*block_h * *block_w) as u64
                        * tile
                        * precision_bytes(p.tensor(*id).dtype) as u64;
                }
                _ => {}
            }
        }
    }
    let mut n = 0;
    for prog in &p.schedule.as_ref().expect("scheduled").programs {
        walk(p, &prog.ops, 1, section, tensor, counts, &mut n);
    }
    n
}

/// Tiles of `tensor` a run actually read inside `section`.
fn tiles_read(trace: &Trace, p: &SpatialProgram, tensor: &str, section: usize) -> u64 {
    trace
        .tiles_read
        .get(&(tensor_named(p, tensor), section))
        .copied()
        .unwrap_or(0)
}

fn values(p: &SpatialProgram, out: &Outputs, name: &str) -> Vec<f64> {
    match out
        .tensors
        .get(&tensor_named(p, name))
        .unwrap_or_else(|| panic!("`{name}` is a result"))
    {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

/// The fills of `p` with `pos{sfx}` set to `pos`.
fn at_position(p: &SpatialProgram, name: &str, pos: usize) -> Inputs {
    let mut inputs = inputs_from_fills(p);
    let id = tensor_named(p, name);
    let v = match inputs.tensors.get(&id) {
        Some(Value::Float(_)) => Value::Float(vec![pos as f64]),
        _ => Value::Int(vec![pos as i64]),
    };
    inputs.tensors.insert(id, v);
    inputs
}

/// `got` against the f64 reference within `(sections + 1)` datapath units of the largest value.
fn close(p: &SpatialProgram, got: &[f64], want: &[f64], unit: f64, vmax: f64, what: &str) {
    let bound = (p.sections.len() as f64 + 1.0) * unit * vmax;
    assert_eq!(got.len(), want.len(), "{what}: the result's length");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(
            (g - w).abs() <= bound,
            "{what}: ct[{i}] = {g}, reference {w} (bound {bound})"
        );
    }
}

fn vmax(s: &Seq) -> f64 {
    let akv = s.kv * s.hd;
    (0..s.ctx * akv)
        .map(|i| fill(&s.name("Vc"), i).abs())
        .chain((0..akv).map(|i| fill(&s.name("vr"), i).abs()))
        .fold(0.0f64, f64::max)
}

/// `p` with every loop that runs to a trip fixed at `n` instead — the program the trip IS at count `n`.
fn pinned(p: &SpatialProgram, n: usize) -> SpatialProgram {
    fn pin(ops: &mut [TaggedOp], n: usize) {
        for t in ops {
            if let ChoreoOp::Loop { count, body, .. } = &mut t.op {
                if matches!(count, LoopCount::Trip { .. }) {
                    *count = LoopCount::Const(n);
                }
                pin(body, n);
            }
        }
    }
    let mut q = p.clone();
    for prog in &mut q.schedule.as_mut().expect("scheduled").programs {
        pin(&mut prog.ops, n);
    }
    q.trips.clear();
    q.certificate = None;
    q
}

/// `p` with the fold's reader issuing its once-per-item loads AFTER its block loop instead of before. The compute's
/// prologue needs them before its first block, so the reader can run ahead only as far as its block buffers hold:
/// two blocks, and a third deadlocks.
fn reader_after_its_blocks(p: &SpatialProgram, fold: usize) -> SpatialProgram {
    fn reorder(p: &SpatialProgram, ops: &mut Vec<TaggedOp>, fold: usize) -> bool {
        let reads_blocks = |t: &TaggedOp| match &t.op {
            ChoreoOp::Loop {
                var,
                count: LoopCount::Trip { .. },
                body,
            } if *var == LV_KBLOCK => body
                .iter()
                .any(|b| matches!(b.op, ChoreoOp::ReadTiles { .. })),
            _ => false,
        };
        if let Some(i) = ops
            .iter()
            .position(|t| reads_blocks(t) && p.section_of_stage(t.stage) == Some(fold))
        {
            let loads = |t: &TaggedOp| {
                matches!(
                    t.op,
                    ChoreoOp::Reserve { .. }
                        | ChoreoOp::ReadTiles { .. }
                        | ChoreoOp::ReadBarrier
                        | ChoreoOp::Push { .. }
                )
            };
            let first = (0..i)
                .rev()
                .take_while(|j| p.section_of_stage(ops[*j].stage) == Some(fold) && loads(&ops[*j]))
                .last()
                .unwrap_or(i);
            if first == i {
                return false;
            }
            let once: Vec<TaggedOp> = ops.drain(first..i).collect();
            let at = first + 1;
            for (k, t) in once.into_iter().enumerate() {
                ops.insert(at + k, t);
            }
            return true;
        }
        ops.iter_mut().any(|t| match &mut t.op {
            ChoreoOp::Loop { body, .. } => reorder(p, body, fold),
            _ => false,
        })
    }
    let mut q = p.clone();
    let snapshot = q.clone();
    let mut moved = 0;
    for prog in &mut q.schedule.as_mut().expect("scheduled").programs {
        moved += usize::from(reorder(&snapshot, &mut prog.ops, fold));
    }
    assert!(
        moved > 0,
        "some reader of the fold issues once-per-item loads before its block loop"
    );
    q.certificate = None;
    q
}

// ───────────────────────────── 1. at real width, one trip from the position ─────────────────────────────

#[test]
fn at_real_width_the_fold_runs_to_its_position_not_to_its_cache() {
    let s = Seq::new(4096, 24, 4, 256, 1000);
    let target = sn50();
    let p = composed(&source(&[s], false), &target);
    let fold = the_fold(&p);
    let (id, max) = fold_trip(&p, fold);
    let nb = s.ctx / 32;
    assert_eq!(
        max, nb,
        "every K-block loop is bounded by the cache's {nb} tile rows"
    );
    let trip = &p.trips[id.idx()];
    let source = p.tensor(trip.tensor);
    assert_eq!(source.name, "pos", "the trip is read from the position");
    assert_eq!(
        source.role,
        TensorRole::Input,
        "the trip's source is a program input, set before the program runs"
    );
    assert_eq!(
        (trip.elem, trip.admit, trip.bias, trip.per),
        (0, (1, s.ctx as i64), 0, 32),
        "the trip is ⌈P / 32⌉ of a position the cache holds"
    );
    assert_eq!(
        (trip.lo(), trip.hi()),
        (1, nb),
        "from one block to the whole cache"
    );

    let whole = (s.ctx * s.kv * s.hd * 2) as u64;
    for cache in ["Kc", "Vc"] {
        assert_eq!(
            read_in(&p, fold, cache, &p.bound_counts()),
            whole,
            "{cache}: the static bound is the whole cache"
        );
    }
    let counts = trip_counts(&p, &Inputs::default()).expect("the position is admitted");
    let blocks = s.pos.div_ceil(32);
    assert_eq!(
        counts[id.idx()],
        blocks,
        "P = {} folds ⌈P/32⌉ = {blocks} blocks",
        s.pos
    );
    let at = (blocks * 32 * s.kv * s.hd * 2) as u64;
    for cache in ["Kc", "Vc"] {
        assert_eq!(
            read_in(&p, fold, cache, &counts),
            at,
            "{cache}: at P = {} the fold reads its {blocks} blocks of each K/V head",
            s.pos
        );
    }
    // Every tile the program moves, on the chip or off it: where the caches live is the deployment's choice.
    let moved = |w: logicaffeine_compile::spatial::work::Work| w.dram_bytes + w.onchip_bytes;
    let (bound, run) = (
        moved(work_of(&p, &target)),
        moved(work_at(&p, &target, &counts)),
    );
    let skipped = ((nb - blocks) * 32 * s.kv * s.hd * 2 * 2) as u64;
    assert_eq!(
        bound - run,
        skipped,
        "the modeled traffic at the position is the bound less exactly the K and V blocks past it"
    );
    println!("real width, P = {}: {blocks} of {nb} blocks; modeled bytes {bound} at the bound, {run} at the position", s.pos);
}

// ───────────────────────────── 2. every position computes, reading only its blocks ─────────────────────────────

fn computes_at(s: &Seq, positions: &[usize]) {
    let want_max = vmax(s);
    for (target, unit, label) in [
        (sn50(), 2f64.powi(-8), "CHIP"),
        (fp32_twin(), 2f64.powi(-20), "WIRING"),
    ] {
        let p = composed(&source(&[*s], true), &target);
        let fold = the_fold(&p);
        let (id, _) = fold_trip(&p, fold);
        for &pos in positions {
            let what = format!(
                "{label} {} heads over {} K/V heads of {}, ctx {}, P = {pos}",
                s.h, s.kv, s.hd, s.ctx
            );
            let inputs = at_position(&p, "pos", pos);
            let blocks = pos.div_ceil(32);
            assert_eq!(
                trip_counts(&p, &inputs).expect("admitted")[id.idx()],
                blocks,
                "{what}: the trip"
            );
            let (out, trace) = eval_choreography_traced(&p, &inputs)
                .unwrap_or_else(|e| panic!("{what}: eval: {e:?}"));
            close(
                &p,
                &values(&p, &out, "ct"),
                &reference(s, pos),
                unit,
                want_max,
                &what,
            );
            let tiles = (blocks * (s.hd / 32) * s.kv) as u64;
            for cache in ["Kc", "Vc"] {
                assert_eq!(tiles_read(&trace, &p, cache, fold), tiles, "{what}: {cache} — {blocks} block(s) of each K/V head's {} tile(s) a row, no more", s.hd / 32);
            }
        }
    }
}

#[test]
fn every_position_of_a_tile_row_cache_computes_and_reads_only_its_blocks() {
    computes_at(&Seq::new(160, 6, 2, 32, 1), &[1, 31, 32, 33, 100, 160]);
}

#[test]
fn a_cache_ending_inside_a_tile_row_computes_at_its_tail() {
    computes_at(&Seq::new(50, 6, 2, 32, 1), &[17, 33, 50]);
}

#[test]
fn two_tile_wide_heads_compute_at_their_position() {
    computes_at(&Seq::new(96, 12, 2, 64, 1), &[5, 70]);
}

// ───────────────────────────── 3. two sequences, two trips ─────────────────────────────

#[test]
fn two_sequences_fold_to_their_own_positions() {
    let a = Seq {
        sfx: "_a",
        ..Seq::new(160, 6, 2, 32, 20)
    };
    let b = Seq {
        sfx: "_b",
        ..Seq::new(160, 6, 2, 32, 150)
    };
    let p = composed(&source(&[a, b], true), &sn50());
    let folds = folds(&p);
    assert_eq!(folds.len(), 2, "one fold per sequence");
    let inputs = inputs_from_fills(&p);
    let counts = trip_counts(&p, &inputs).expect("both positions admitted");
    let (out, trace) = eval_choreography_traced(&p, &inputs).expect("the program runs");
    let mut trips = BTreeSet::new();
    for (section, cache) in &folds {
        let s = if cache.ends_with("_a") { a } else { b };
        let (id, _) = fold_trip(&p, *section);
        trips.insert(id);
        assert_eq!(
            p.tensor(p.trips[id.idx()].tensor).name,
            s.name("pos"),
            "{cache}'s fold runs to its own sequence's position"
        );
        let blocks = s.pos.div_ceil(32);
        assert_eq!(counts[id.idx()], blocks, "{cache}: ⌈{}/32⌉ blocks", s.pos);
        assert_eq!(
            tiles_read(&trace, &p, cache, *section),
            (blocks * s.kv) as u64,
            "{cache}: only its own sequence's blocks are read"
        );
        close(
            &p,
            &values(&p, &out, &s.name("ct")),
            &reference(&s, s.pos),
            2f64.powi(-8),
            vmax(&s),
            &format!("sequence {}", s.sfx),
        );
    }
    assert_eq!(trips.len(), 2, "each sequence's fold has its own trip");
}

// ───────────────────────────── 4. the certificate covers every count ─────────────────────────────

#[test]
fn the_certificate_proves_every_count_of_the_trips_window() {
    let s = Seq::new(320, 6, 2, 32, 40);
    let p = composed(&source(&[s], false), &sn50());
    let fold = the_fold(&p);
    let (id, max) = fold_trip(&p, fold);
    let window = trip_window(&p, id);
    assert_eq!(
        *window.start(),
        1,
        "the window starts at the trip's least count"
    );
    assert!(
        *window.end() >= 3 && *window.end() <= max,
        "the window reaches past a block buffer's depth and stays inside the bound: {window:?}"
    );
    let c = p.certificate.clone().expect("certified");
    let GraphProof::PerSection { sections, .. } = &c.proof else {
        panic!("a composed program is proved per section")
    };
    let want: Vec<Vec<usize>> = window.clone().map(|n| vec![n]).collect();
    for sp in sections {
        let got: Vec<Vec<usize>> = sp.cases.iter().map(|case| case.counts.clone()).collect();
        if sp.section == fold {
            assert_eq!(
                got, want,
                "the fold's proof holds one case per count of the window"
            );
            assert!(
                sp.cases.iter().all(|case| case.transitions > 0),
                "no count is proved vacuously"
            );
        } else {
            assert!(
                got.is_empty(),
                "section {} runs to no trip and proves no counts",
                sp.section
            );
        }
    }
    let mut dropped = c.clone();
    let GraphProof::PerSection { sections, .. } = &mut dropped.proof else {
        unreachable!()
    };
    sections[fold].cases.pop();
    assert!(
        !dropped.recheck(&p),
        "a certificate missing a count of the window is refused"
    );
    let mut moved = c.clone();
    let GraphProof::PerSection { sections, .. } = &mut moved.proof else {
        unreachable!()
    };
    sections[fold].cases[0].counts = vec![max];
    assert!(
        !moved.recheck(&p),
        "a certificate that proves another count in place of one of the window's is refused"
    );
}

// ───────────────────────────── 5. a deadlock only a longer trip reaches ─────────────────────────────

#[test]
fn a_deadlock_only_a_longer_trip_reaches_is_refused() {
    let s = Seq::new(320, 6, 2, 32, 40);
    let p = composed(&source(&[s], true), &sn50());
    let fold = the_fold(&p);
    let mutant = reader_after_its_blocks(&p, fold);
    assert!(
        mutant.validate().is_ok(),
        "the reordered reader is a well-formed program: {:?}",
        mutant.validate()
    );
    // Two blocks fit the reader's block buffers; a third waits on a compute that waits on the reader.
    let two = eval_choreography(&mutant, &at_position(&mutant, "pos", 64));
    assert!(
        two.is_ok(),
        "at P = 64 (two blocks) the reordered reader still runs: {two:?}"
    );
    let three = eval_choreography(&mutant, &at_position(&mutant, "pos", 65));
    assert!(
        matches!(three, Err(ChoreoFault::Deadlock { .. })),
        "at P = 65 (three blocks) it deadlocks: {three:?}"
    );
    match certify_choreography(&mutant) {
        Err(CertifyFault::Deadlock { .. } | CertifyFault::Starved { .. } | CertifyFault::Eval(ChoreoFault::Deadlock { .. })) => {}
        other => panic!("a program that deadlocks at three blocks earns no certificate, whatever its compiled position: {other:?}"),
    }
}

#[test]
fn the_windows_verdict_is_the_verdict_of_every_count() {
    let s = Seq::new(320, 6, 2, 32, 40);
    let p = composed(&source(&[s], false), &sn50());
    let fold = the_fold(&p);
    let (id, max) = fold_trip(&p, fold);
    assert!(
        *trip_window(&p, id).end() < max,
        "the window is shorter than the bound, or this compares nothing"
    );
    let mutant = reader_after_its_blocks(&p, fold);
    for n in 1..=max {
        assert!(
            certify_choreography(&pinned(&p, n)).is_ok(),
            "the fold certifies at every count: {n}"
        );
        assert_eq!(certify_choreography(&pinned(&mutant, n)).is_ok(), n <= 2, "the reordered reader certifies at count {n} exactly when two block pages hold the reader's lead");
    }
    assert!(
        certify_choreography(&p).is_ok(),
        "every count certifies, and so does the trip"
    );
    assert!(
        certify_choreography(&mutant).is_err(),
        "counts past two deadlock, and so the trip is refused"
    );
}

// ───────────────────────────── 6. a position the trip does not admit ─────────────────────────────

#[test]
fn a_position_the_trip_does_not_admit_faults_by_name_before_anything_runs() {
    let s = Seq::new(320, 6, 2, 32, 40);
    let p = composed(&source(&[s], true), &sn50());
    let (id, _) = fold_trip(&p, the_fold(&p));
    for pos in [0usize, 321, 4096] {
        let inputs = at_position(&p, "pos", pos);
        match trip_counts(&p, &inputs) {
            Err(ChoreoFault::TripOutOfRange {
                trip,
                value,
                admitted,
            }) => {
                assert_eq!(trip, id, "P = {pos}: the fault names the trip");
                assert_eq!(
                    value, pos as f64,
                    "P = {pos}: the fault carries the value it read"
                );
                assert_eq!(
                    admitted,
                    (1, s.ctx as i64),
                    "P = {pos}: refused against the cache's positions"
                );
            }
            other => {
                panic!("P = {pos}: a position outside the cache is refused by name, got {other:?}")
            }
        }
        let run = eval_choreography(&p, &inputs);
        assert!(
            matches!(run, Err(ChoreoFault::TripOutOfRange { .. })),
            "P = {pos}: the run faults on the trip before any op, got {run:?}"
        );
    }
}

// ───────────────────────────── 7. a malformed trip ─────────────────────────────

#[test]
fn a_malformed_trip_or_a_loop_that_misuses_one_is_refused_by_name() {
    let s = Seq::new(320, 6, 2, 32, 40);
    let p = composed(&source(&[s], false), &sn50());
    let fold = the_fold(&p);
    let (id, _) = fold_trip(&p, fold);
    let ct = tensor_named(&p, "ct");
    let sem = p
        .semaphores
        .first()
        .map(|s| s.id)
        .expect("a composed program holds a barrier semaphore");
    fn first_trip_loop(ops: &mut [TaggedOp]) -> Option<&mut TaggedOp> {
        for t in ops {
            if matches!(
                t.op,
                ChoreoOp::Loop {
                    count: LoopCount::Trip { .. },
                    ..
                }
            ) {
                return Some(t);
            }
            if let ChoreoOp::Loop { body, .. } = &mut t.op {
                if let Some(x) = first_trip_loop(body) {
                    return Some(x);
                }
            }
        }
        None
    }
    let with_loop = |f: &dyn Fn(&mut TaggedOp)| {
        let mut q = p.clone();
        for prog in &mut q.schedule.as_mut().unwrap().programs {
            if let Some(t) = first_trip_loop(&mut prog.ops) {
                f(t);
                return q;
            }
        }
        panic!("a loop that runs to the trip");
    };
    let with_trip = |f: &dyn Fn(&mut logicaffeine_compile::spatial::ir::Trip)| {
        let mut q = p.clone();
        f(&mut q.trips[id.idx()]);
        q
    };
    let cases: Vec<(&str, SpatialProgram)> = vec![
        (
            "read from a tensor the program writes",
            with_trip(&|t| t.tensor = ct),
        ),
        ("an element past its tensor", with_trip(&|t| t.elem = 1)),
        ("a least count of zero", with_trip(&|t| t.admit.0 = 0)),
        (
            "no admitted value",
            with_trip(&|t| t.admit.0 = t.admit.1 + 1),
        ),
        ("counted per zero", with_trip(&|t| t.per = 0)),
        ("a trip no loop runs to", {
            let mut q = p.clone();
            let spare = q.trips[id.idx()].clone();
            q.trips.push(spare);
            q
        }),
        (
            "a loop bounded apart from its trip",
            with_loop(&|t| {
                if let ChoreoOp::Loop {
                    count: LoopCount::Trip { max, .. },
                    ..
                } = &mut t.op
                {
                    *max -= 1;
                }
            }),
        ),
        (
            "a loop naming a trip the program does not declare",
            with_loop(&|t| {
                if let ChoreoOp::Loop {
                    count: LoopCount::Trip { trip, .. },
                    ..
                } = &mut t.op
                {
                    *trip = TripId::new(7);
                }
            }),
        ),
        (
            "a semaphore inside a trip loop",
            with_loop(&|t| {
                if let ChoreoOp::Loop { body, .. } = &mut t.op {
                    let stage = body[0].stage;
                    body.insert(
                        0,
                        TaggedOp {
                            stage,
                            op: ChoreoOp::SemWait {
                                sem,
                                until: SemCond::AtLeast(1),
                            },
                        },
                    );
                }
            }),
        ),
        (
            "a store inside a trip loop",
            with_loop(&|t| {
                if let ChoreoOp::Loop { body, .. } = &mut t.op {
                    let stage = body[0].stage;
                    let into = body.iter().find_map(|b| match &b.op {
                        ChoreoOp::ReadTiles { into, .. } => Some(*into),
                        ChoreoOp::Compute { ins, .. } => ins.first().copied(),
                        _ => None,
                    });
                    let from = into.expect("a trip loop reads or computes on a page");
                    body.push(TaggedOp {
                        stage,
                        op: ChoreoOp::WriteTiles {
                            from,
                            tensor: ct,
                            at: logicaffeine_compile::spatial::ir::TileAddr {
                                base: 0,
                                steps: Vec::new(),
                                indirect: None,
                            },
                            block_h: 1,
                            block_w: 1,
                            stride_w: 1,
                            stride_h: 1,
                        },
                    });
                }
            }),
        ),
        (
            "a trip loop inside another",
            with_loop(&|t| {
                if let ChoreoOp::Loop { count, body, var } = &mut t.op {
                    let inner = TaggedOp {
                        stage: t.stage,
                        op: ChoreoOp::Loop {
                            var: *var,
                            count: *count,
                            body: std::mem::take(body),
                        },
                    };
                    *body = vec![inner];
                }
            }),
        ),
    ];
    let mut reasons = BTreeSet::new();
    for (what, q) in &cases {
        let reason = match q.validate() {
            Err(IrViolation::TripMalformed { reason, .. })
            | Err(IrViolation::TripLoopMalformed { reason }) => reason,
            other => panic!("{what}: refused by name as a malformed trip, got {other:?}"),
        };
        assert!(
            reason.split_whitespace().count() >= 3,
            "{what}: the refusal names what it refused, got `{reason}`"
        );
        reasons.insert(reason);
    }
    assert_eq!(
        reasons.len(),
        cases.len(),
        "each misuse is refused for its own reason: {reasons:?}"
    );
}

/// A buffer whose pages balance at the greatest count and nowhere else: the reader's block pages pushed a round at a
/// time and popped all at once after the compute's blocks. Every total agrees at the bound; every other count leaves
/// pages behind — refused by the per-round tally, not by the totals.
#[test]
fn a_buffer_that_balances_only_at_the_greatest_count_is_refused() {
    let s = Seq::new(320, 6, 2, 32, 40);
    let p = composed(&source(&[s], false), &sn50());
    let fold = the_fold(&p);
    let (_, max) = fold_trip(&p, fold);
    let mut q = p.clone();
    let mut moved = None;
    fn retime(
        ops: &mut Vec<TaggedOp>,
        max: usize,
        moved: &mut Option<logicaffeine_compile::spatial::ir::BufferId>,
    ) -> bool {
        for i in 0..ops.len() {
            let found = match &mut ops[i].op {
                ChoreoOp::Loop {
                    count: LoopCount::Trip { .. },
                    body,
                    ..
                } => body
                    .iter()
                    .position(|t| matches!(t.op, ChoreoOp::Pop { .. }))
                    .map(|j| body.remove(j)),
                ChoreoOp::Loop { body, .. } => {
                    if retime(body, max, moved) {
                        return true;
                    }
                    None
                }
                _ => None,
            };
            if let Some(TaggedOp {
                stage,
                op: ChoreoOp::Pop { buf, .. },
            }) = found
            {
                ops.insert(
                    i + 1,
                    TaggedOp {
                        stage,
                        op: ChoreoOp::Pop { buf, pages: max },
                    },
                );
                *moved = Some(buf);
                return true;
            }
        }
        false
    }
    for prog in &mut q.schedule.as_mut().unwrap().programs {
        if retime(&mut prog.ops, max, &mut moved) {
            break;
        }
    }
    let buf = moved.expect("a trip loop that pops a page");
    match q.validate() {
        Err(IrViolation::CountingImbalance { buffer, .. }) => {
            assert_eq!(buffer, buf, "the retimed buffer is the one refused")
        }
        other => panic!(
            "a buffer balanced only at the greatest count is a counting imbalance, got {other:?}"
        ),
    }
}

// ───────────────────────────── 8. a count the program fixes stays fixed ─────────────────────────────

#[test]
fn a_count_the_program_fixes_stays_a_constant() {
    for (reach, what) in [("", "the whole cache"), (" up to 45", "a literal prefix")] {
        let src = format!(
            "## Main\nLet Kc be a 96 by 64 tensor of Real.\nLet Vc be a 96 by 64 tensor of Real.\nLet qp be a 1 by 192 tensor of Real.\n\
             Let ct be attention of qp over Kc and Vc with 6 heads 2 groups{reach}.\n"
        );
        let p = composed(&src, &sn50());
        assert!(
            p.trips.is_empty(),
            "{what}: nothing is read from the data, so there is no trip"
        );
        let loops = k_loops(&p, the_fold(&p));
        assert!(
            loops.iter().all(|c| matches!(c, LoopCount::Const(_))),
            "{what}: the fold's counts are constants: {loops:?}"
        );
    }
}

// ───────────────────────────── 9. every printer ─────────────────────────────

#[test]
fn every_printer_prints_the_trip_or_refuses_it_by_name() {
    let backends: Vec<(&str, Box<dyn SpatialBackend>)> = vec![
        (
            "Tensix",
            Box::new(TensixBackend::new(SpatialTarget::wormhole())),
        ),
        (
            "fabric",
            Box::new(FabricBackend::new(SpatialTarget::sky_fabric(64, 64))),
        ),
        (
            "CUDA",
            Box::new(CudaPersistentBackend::new(SpatialTarget::rtx3060())),
        ),
        ("RDU", Box::new(RduSpatialBackend::new())),
    ];
    let mut matrix = Vec::new();
    for (name, backend) in backends {
        let target = backend.target().clone();
        let hd = 2 * target.tile.cols;
        let src = format!(
            "## Main\nLet pos be 40.\nLet Kc be a 96 by {w} tensor of Real.\nLet Vc be a 96 by {w} tensor of Real.\nLet qp be a 1 by {w} tensor of Real.\n\
             Let ct be attention of qp over Kc and Vc with 2 heads 2 groups up to pos.\n",
            w = 2 * hd
        );
        let lowered = compile_program_to_spatial(&src, &target).expect("a tensor program");
        let Some(p) = lowered.program else {
            assert_ne!(
                name, "RDU",
                "the SN50 lowers a decode attention: {:?}",
                lowered.missing
            );
            matrix.push(format!(
                "{name}: the front end does not compose it ({:?})",
                lowered.missing
            ));
            continue;
        };
        if p.trips.is_empty() {
            assert_ne!(name, "RDU", "the SN50's decode fold runs to its position");
            matrix.push(format!("{name}: no trip on this target"));
            continue;
        }
        match (name, backend.lower(&p)) {
            ("RDU", Ok(SpatialArtifact::SdkGated(art))) => {
                assert!(art.certificate.recheck(&p), "RDU: the shipped certificate rechecks");
                let json: serde_json::Value = serde_json::from_str(&art.json).expect("the neutral artifact is JSON");
                let text = json.to_string();
                assert!(text.contains("\"trips\""), "RDU: the neutral artifact carries the program's trips");
                matrix.push("RDU: ships the trip (SdkGated)".to_string());
            }
            ("RDU", other) => panic!("RDU: the neutral lane ships the decode fold, got {other:?}"),
            ("CUDA", Ok(SpatialArtifact::Cuda(art))) => {
                let pos = p.tensor(p.trips[0].tensor).name.clone();
                assert!(art.program_cu.contains("trip_0"), "CUDA: the kernel computes the trip it loops to");
                assert!(art.program_cu.contains(&pos) || art.program_cu.contains("trip_src_0"), "CUDA: the trip is read from `{pos}`");
                matrix.push("CUDA: prints the trip".to_string());
            }
            (_, Ok(_)) => panic!("{name}: a printer that lowers a trip program must print the trip — this lock names each one that does"),
            (_, Err(SpatialLowerError::Unsupported(reason))) => {
                assert!(reason.split_whitespace().count() >= 3, "{name}: a refusal names what it refused, got `{reason}`");
                matrix.push(format!("{name}: refuses: {reason}"));
            }
            (_, Err(other)) => panic!("{name}: must print the trip or refuse it with `Unsupported(<reason>)`, got {other:?}"),
        }
    }
    println!("trip × backend:\n  {}", matrix.join("\n  "));
    assert_eq!(matrix.len(), 4, "every backend answers: {matrix:?}");
}

// ───────────────────────────── a fold of any kind, run to a trip ─────────────────────────────

const ITEMS: usize = 2;
const BLOCKS: usize = 6;

/// A value exact in bf16 and in every sum the fold forms: a multiple of 1/8 in `[-2, 2]`.
fn val(seed: usize) -> f64 {
    (((seed.wrapping_mul(7919).wrapping_add(13)) % 33) as f64 - 16.0) / 8.0
}

/// The carried sum `y[i] = Σ_{k < n} a[i] ⊙ b[i, k]` over `BLOCKS` blocks of `b` per item, built through the template
/// on `target`'s tile. With `trip`, its blocks run to the count held by the input `n` (`1 ≤ n ≤ BLOCKS`); without, to
/// all of them. Returns the program with `a` and `b`'s values.
fn counted_fold(target: &SpatialTarget, trip: bool) -> (SpatialProgram, Vec<f64>, Vec<f64>) {
    use logicaffeine_compile::spatial::ir::*;
    use logicaffeine_compile::spatial::passes::from_tensor_op::{ident_map, Template};
    use logicaffeine_compile::spatial::passes::{LV_GEN_COL, LV_GEN_ROW, LV_ITEM_Y};
    let t = target.tile.rows;
    let int = target.name.contains("fabric");
    let dtype = if int {
        Precision::Int64
    } else {
        Precision::Fp32
    };
    let fill = |salt: usize, n: usize| -> (TensorInit, Vec<f64>) {
        let v: Vec<f64> = (0..n)
            .map(|p| {
                if int {
                    ((p * 7 + salt) % 5) as f64 - 2.0
                } else {
                    val(p * 31 + salt)
                }
            })
            .collect();
        let init = if int {
            TensorInit::Values(v.iter().map(|x| *x as i64).collect())
        } else {
            TensorInit::FloatValues(v.iter().map(|x| x.to_bits()).collect())
        };
        (init, v)
    };
    let pointwise = |n_ins: usize, body: TensorBody| {
        let iters = vec![
            IterSpec {
                var: LV_GEN_ROW,
                extent: t,
                kind: IterKind::Parallel,
            },
            IterSpec {
                var: LV_GEN_COL,
                extent: t,
                kind: IterKind::Parallel,
            },
        ];
        StageKind::Compute(ComputeOp::Generic {
            iters,
            ins: vec![ident_map(t); n_ins],
            out: ident_map(t),
            body,
        })
    };
    let mut tp = Template::new(target).expect("a template");
    let (a_init, a) = fill(1, ITEMS * t * t);
    let (b_init, b) = fill(2, ITEMS * BLOCKS * t * t);
    let at = tp.tensor("a", TensorRole::Input, ITEMS * t, t, dtype, a_init);
    let bt = tp.tensor("b", TensorRole::Input, ITEMS * BLOCKS * t, t, dtype, b_init);
    let yt = tp.tensor(
        "y",
        TensorRole::Output,
        ITEMS * t,
        t,
        dtype,
        TensorInit::Zero,
    );
    let n_init = if int {
        TensorInit::Values(vec![BLOCKS as i64])
    } else {
        TensorInit::FloatValues(vec![(BLOCKS as f64).to_bits()])
    };
    let nt = tp.tensor("n", TensorRole::Input, 1, 1, dtype, n_init);
    let a_in = tp.buffer_deep("in_a", 1, 1, dtype);
    let b_in = tp.buffer("in_b", 1, dtype);
    let prod = tp.scratch_deep("prod", 1, 1, dtype);
    let acc = tp.scratch_deep("acc", 2, 1, dtype);
    let out = tp.buffer("out", 1, dtype);
    let la = tp.load_block_at("load_a", at, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, a_in);
    let origin = TileAddr {
        base: 0,
        steps: vec![(LV_ITEM_Y, BLOCKS), (LV_KBLOCK, 1)],
        indirect: None,
    };
    let lb = tp.stage(
        "load_b",
        StageKind::Load {
            tensor: bt,
            pattern: AccessPattern {
                origin,
                block_h: 1,
                block_w: 1,
                stride_w: 1,
                stride_h: 1,
            },
            num_blocks: BLOCKS,
            multicast: None,
        },
        vec![],
        vec![b_in],
    );
    let s_init = tp.stage(
        "acc_init",
        pointwise(0, TensorBody::Const(0f64.to_bits())),
        vec![],
        vec![acc],
    );
    let s_prod = tp.stage(
        "prod",
        pointwise(
            2,
            TensorBody::Mul(
                Box::new(TensorBody::Operand(0)),
                Box::new(TensorBody::Operand(1)),
            ),
        ),
        vec![a_in, b_in],
        vec![prod],
    );
    let s_fold = tp.stage(
        "acc_fold",
        pointwise(
            2,
            TensorBody::Add(
                Box::new(TensorBody::Operand(0)),
                Box::new(TensorBody::Operand(1)),
            ),
        ),
        vec![acc, prod],
        vec![acc],
    );
    let s_out = tp.stage(
        "acc_out",
        pointwise(1, TensorBody::Operand(0)),
        vec![acc],
        vec![out],
    );
    let st = tp.store_block("store_y", yt, 0, vec![(LV_ITEM_Y, 1)], 1, 1, 1, out);
    tp.edge(la, a_in, s_prod);
    tp.edge(lb, b_in, s_prod);
    tp.edge(s_prod, prod, s_fold);
    tp.edge(s_init, acc, s_fold);
    tp.edge(s_out, out, st);
    let k_trip = trip.then(|| {
        tp.trip(Trip {
            tensor: nt,
            elem: 0,
            admit: (1, BLOCKS as i64),
            bias: 0,
            per: 1,
        })
    });
    let items = (0..ITEMS)
        .map(|y| WorkItem {
            id: WorkItemId::new(y),
            coord: (y, 0),
            out: TileRect {
                row0_t: y,
                col0_t: 0,
                h_t: 1,
                w_t: 1,
            },
        })
        .collect();
    let mut p = tp
        .finish(
            if trip {
                "fold_to_a_trip".into()
            } else {
                "fold_fixed".into()
            },
            items,
            AccPolicy::F32,
            NumericsClaim::Exact,
            target,
        )
        .unwrap_or_else(|e| panic!("the fold template validates: {e:?}"));
    p.work.plan = Some(BlockPlan {
        per_site_m: 1,
        per_site_n: 1,
        grid: (ITEMS, 1),
        inner: InnerLoop {
            num_blocks: BLOCKS,
            block_k_tiles: 1,
        },
        block: BlockShape {
            osh: 1,
            osw: 1,
            ibw: 1,
        },
        acc: AccPolicy::F32,
        pack_accumulate: false,
        engines: EnginePacking::LoadsTogether,
        granularity: Granularity::PerTile,
    });
    p.work.k_tiles = BLOCKS;
    p.work.k_trip = k_trip;
    (p, a, b)
}

/// ★ A TRIP IS A LOOP BOUND, NOT A MASK. A carried sum run to `n` blocks is the sum of its first `n` blocks — nothing
/// masks the rest, so every count is its own answer, and the run reads `n` blocks and no more.
#[test]
fn a_fold_run_to_a_trip_folds_exactly_its_count_of_blocks() {
    let target = sn50();
    let (p, a, b) = counted_fold(&target, true);
    let v = logicaffeine_compile::spatial::pipeline::run_passes(p, &target)
        .unwrap_or_else(|e| panic!("the fold run to a trip verifies: {e:?}"));
    assert_eq!(v.phase(), Phase::Verified);
    assert!(
        v.certificate.as_ref().is_some_and(|c| c.recheck(&v)),
        "its certificate rechecks"
    );
    let t = target.tile.rows;
    let n = t * t;
    for count in 1..=BLOCKS {
        let inputs = at_position(&v, "n", count);
        let (out, trace) = eval_choreography_traced(&v, &inputs)
            .unwrap_or_else(|e| panic!("count {count}: {e:?}"));
        let got = values(&v, &out, "y");
        for y in 0..ITEMS {
            for e in 0..n {
                let want: f64 = (0..count)
                    .map(|k| a[y * n + e] * b[(y * BLOCKS + k) * n + e])
                    .sum();
                assert_eq!(
                    got[y * n + e],
                    want,
                    "count {count}: y[{y}][{e}] is the sum of the first {count} blocks"
                );
            }
        }
        assert_eq!(
            tiles_read(&trace, &v, "b", 0),
            (count * ITEMS) as u64,
            "count {count}: the run reads {count} block(s) of each item and no more"
        );
    }
}

/// A printer that lowers a fold must refuse the same fold run to a trip by name — never print it with a count it no
/// longer has. The RDU ships it.
#[test]
fn a_printer_refuses_by_name_the_fold_it_lowers_once_its_count_is_read_from_the_data() {
    let backends: Vec<(&str, Box<dyn SpatialBackend>)> = vec![
        (
            "Tensix",
            Box::new(TensixBackend::new(SpatialTarget::wormhole())),
        ),
        (
            "fabric",
            Box::new(FabricBackend::new(SpatialTarget::sky_fabric(64, 64))),
        ),
        (
            "CUDA",
            Box::new(CudaPersistentBackend::new(SpatialTarget::rtx3060())),
        ),
        ("RDU", Box::new(RduSpatialBackend::new())),
    ];
    let mut matrix = Vec::new();
    for (name, backend) in backends {
        let target = backend.target().clone();
        let fixed = logicaffeine_compile::spatial::pipeline::run_passes(
            counted_fold(&target, false).0,
            &target,
        );
        let lowers = fixed.as_ref().is_ok_and(|v| backend.lower(v).is_ok());
        let tripped = match logicaffeine_compile::spatial::pipeline::run_passes(
            counted_fold(&target, true).0,
            &target,
        ) {
            Ok(v) => v,
            Err(e) => {
                assert!(!lowers, "{name}: the passes that verify the fixed fold verify it run to a trip, got {e:?}");
                matrix.push(format!("{name}: the passes refuse the fold either way"));
                continue;
            }
        };
        match (name, backend.lower(&tripped)) {
            ("RDU", Ok(SpatialArtifact::SdkGated(art))) => {
                assert!(
                    art.json.contains("\"trips\""),
                    "RDU: the neutral artifact carries the trip"
                );
                matrix.push("RDU: ships the trip".into());
            }
            ("RDU", other) => panic!("RDU: ships a fold run to a trip, got {other:?}"),
            (_, Err(SpatialLowerError::Unsupported(reason))) if reason.contains("trip") => {
                matrix.push(format!("{name}: refuses the trip"))
            }
            (_, Err(SpatialLowerError::Unsupported(reason))) => {
                assert!(!lowers, "{name}: lowers the fixed fold, so its refusal of the tripped one names the trip, got `{reason}`");
                matrix.push(format!("{name}: refuses the fold either way: {reason}"));
            }
            (_, other) => panic!(
                "{name}: a count read from the data is refused by name, got {:?}",
                other.map(|_| "an artifact")
            ),
        }
    }
    println!("fold run to a trip × backend:\n  {}", matrix.join("\n  "));
    for printer in ["Tensix", "fabric"] {
        assert!(
            matrix.contains(&format!("{printer}: refuses the trip")),
            "{printer} lowers the fixed fold, so it reaches its trip refusal: {matrix:?}"
        );
    }
}
