//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE SPATIAL COMPILER ANSWERS FROM INDEXES — and every index answers what the scan it replaced answered.
//!
//! A batched tensor-parallel decode layer composes thousands of sections, and the compiler asks per-stage, per-op and
//! per-section questions of it: which section holds a stage, which items a site runs in a section, which later
//! statements read a result, which phase a section may join. Each answered by a scan of the program would make the
//! compiler quadratic, so each is read from an index built once per program — and this lock holds every index to its
//! DEFINITION, the scan, computed here the slow way, on real composed programs and on seeded random inputs.
//! (`spatial_compile_scaling_timing_lock` holds the pipeline to linear time; this lock holds it to the right answers.)
//!
//! Pinned:
//!   1. THE SECTION OF AN ID — `section_of_stage`, `section_of_buffer`, `section_of_item`, a search in the partition —
//!      is the first section whose span holds it, for every id of a composed single-chip program and a composed
//!      tensor-parallel one, one past the last id, and in a program without sections.
//!   2. ON SPANS THAT DO NOT PARTITION — reversed, or with a section cut out — the answer is still the scan's.
//!   3. `SiteItems::at` IS `items_at_in`, for every section (and none, and one out of range) and every site (and one
//!      the placement does not have).
//!   4. `phase_order` IS THE PAIRWISE RULE: a section joins the earliest phase past every earlier section it depends
//!      on (a read of an earlier write, a write of an earlier read or write) where its chips are free — on 4,000
//!      seeded random programs of reads, writes and chips; and its phases are sound (no phase holds two sections on
//!      one chip, or two that conflict, and a dependency always crosses a barrier).
//!   5. `links` IS THE PAIRWISE READER RELATION and `state_tensors` the carried state in first-appearance order, on
//!      the decode-step, batched and tensor-parallel sources.
//!   6. CERTIFYING ONCE, AS THE COMPOSITION, IS CERTIFYING EACH SECTION AND THEN THE COMPOSITION: the lowering that
//!      only schedules its sections returns the same program, certificate and counts as the one that certifies every
//!      section, on single-chip, batched, tensor-parallel and batched tensor-parallel programs; and on a program that
//!      does not lower whole, the lowering's answer is the section-by-section one, which names what refused.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_compile_index_lock`
//!
//! ★ MUTATION PROBES (run 2026-09-25, each in a private copy of the workspace, each red):
//!   - the partition search trusting its candidate section without checking it holds the id → 2 (an id in the cut
//!     section is given the section after it).
//!   - `SiteItems::at` answering an out-of-range section from its per-section map → 3.
//!   - `phase_order` placing a section past its reads' writers only (no write-after-read, no write-after-write) → 4.
//!   - a chip's taken phase left free (two sections on one chip in one phase) → 4.
//!   - `links` naming every reader, earlier stages included → 5; `state_tensors` without its first-appearance dedupe → 5.
//!   The other indexes are pinned where their answers are observable:
//!   - the certificate's barrier prefix count holding for any range → `spatial_compose_lock::a_missing_barrier_chain_is_
//!     refused`, `spatial_stacked_rows_lock::a_table_written_row_by_row_holds_every_row_and_orders_its_writes`.
//!   - its successor walk reporting every stage reachable → `spatial_compose_lock::a_section_that_reads_and_writes_the_
//!     state_orders_the_write_after_the_read`.
//!   - the liveness graphs' item window one wider, or a section's first program dropped from its selection → every
//!     byte-pinned certificate (`rdu_fit_report_lock`'s reports; `qwen38_rdu_lock`, 17 tests).
//!   - the evaluator reading a site's items across every section → `qwen38_decode_step_lock::a_real_width_cache_append_
//!     and_masked_attention_lower_and_certify`.
//!   - the printer ordering a site's stages by id rather than by dataflow → `qwen38_fabric_segments_lock::every_segment_
//!     on_the_ledger_is_todays_app_and_passed_on_the_fabric`.
//!   - the uncertified pass list also dropping `fuse_generic` → 6 (the tiny step's program differs). Dropping `persist`
//!     instead stays green: these programs hold one work item per site, where `persist` wraps nothing.
//!   - `lower_program` never falling back to the section-by-section lowering → stays green, and is not claimed as
//!     covered: every section the front end produces certifies on its own, so the two lowerings agree even on a
//!     program that does not lower whole. The fallback keeps the per-statement refusal for the day one does not.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen38::{
    gen_qwen38_decode_batch_layer_lg, gen_qwen38_decode_batch_tp_layer_lg,
    gen_qwen38_decode_step_lg, gen_qwen38_tp_decode_layer_lg, Qwen38Cfg,
};
use logicaffeine_compile::spatial::ir::{
    BufferId, SectionSpan, Site, SpatialProgram, StageId, WorkItemId,
};
use logicaffeine_compile::spatial::passes::recognize_stages::{
    links, recognize_stages, state_tensors,
};
use logicaffeine_compile::spatial::pipeline::{
    compile_program_to_spatial, compile_program_to_spatial_tp, compile_to_spatial,
    lower_program_with, ProgramLowering, SectionCertification,
};
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::tensor_parallel::{phase_order, SectionTouch};
use std::collections::BTreeSet;

#[path = "common/spatial.rs"]
#[allow(dead_code)]
mod spatial_common;

fn single_chip() -> SpatialProgram {
    let target = RduChip::Sn50.facts().spatial_target();
    let (src, _, _) = gen_qwen38_decode_batch_layer_lg(&Qwen38Cfg::mini(), 0, "/w", 6, &[1, 2, 3]);
    compile_program_to_spatial(&src, &target)
        .expect("the batched layer lowers")
        .into_whole()
        .expect("every statement lowers")
}

fn tensor_parallel() -> SpatialProgram {
    let target = RduChip::Sn50.facts().spatial_target();
    let (src, _, _, m) =
        gen_qwen38_decode_batch_tp_layer_lg(&Qwen38Cfg::mini(), 3, "/w", 6, &[0, 1, 2], 2);
    compile_program_to_spatial_tp(&src, &target, &m)
        .expect("the tensor-parallel layer lowers")
        .into_whole()
        .expect("every statement lowers")
}

/// The definition: the first section whose `span` holds `id`.
fn scan(p: &SpatialProgram, id: usize, span: fn(&SectionSpan) -> (usize, usize)) -> Option<usize> {
    p.sections.iter().position(|sp| {
        let (a, b) = span(sp);
        a <= id && id < b
    })
}

/// Every stage, buffer and item id of `p` (and one past each), looked up both ways.
fn assert_lookups_are_the_scans(p: &SpatialProgram, what: &str) {
    for id in 0..=p.stages.len() {
        assert_eq!(
            p.section_of_stage(StageId::new(id)),
            scan(p, id, |sp| sp.stages),
            "{what}: the section of stage {id}"
        );
    }
    for id in 0..=p.buffers.len() {
        assert_eq!(
            p.section_of_buffer(BufferId::new(id)),
            scan(p, id, |sp| sp.buffers),
            "{what}: the section of buffer {id}"
        );
    }
    for id in 0..=p.work.items.len() {
        assert_eq!(
            p.section_of_item(WorkItemId::new(id)),
            scan(p, id, |sp| sp.items),
            "{what}: the section of item {id}"
        );
    }
}

// ───────────────────────────── 1 ─────────────────────────────

#[test]
fn the_section_of_every_id_is_the_first_section_holding_it() {
    for (what, p) in [
        ("single-chip", single_chip()),
        ("tensor-parallel", tensor_parallel()),
    ] {
        assert!(
            p.sections.len() > 50,
            "{what}: a composed layer of many sections ({})",
            p.sections.len()
        );
        assert_lookups_are_the_scans(&p, what);
        // Every stage has its section: the spans partition the stages.
        assert!(
            p.stages.iter().all(|s| p.section_of_stage(s.id).is_some()),
            "{what}: every stage lies in a section"
        );
    }
    let target = RduChip::Sn50.facts().spatial_target();
    let one = compile_to_spatial(&spatial_common::gemm_program(64, 64, 64), &target)
        .expect("one matmul lowers");
    assert!(
        one.sections.is_empty(),
        "the single-op front end composes no sections"
    );
    assert_lookups_are_the_scans(&one, "a program without sections");
    assert_eq!(
        one.section_of_stage(StageId::new(0)),
        None,
        "no section holds anything in a program without sections"
    );
}

// ───────────────────────────── 2 ─────────────────────────────

#[test]
fn a_lookup_over_spans_that_do_not_partition_is_still_the_scans() {
    let p = tensor_parallel();
    let mut reversed = p.clone();
    reversed.sections.reverse();
    assert_lookups_are_the_scans(&reversed, "reversed spans");
    let mut cut = p.clone();
    let middle = cut.sections.len() / 2;
    let gone = cut.sections.remove(middle);
    assert_lookups_are_the_scans(&cut, "a section cut out");
    assert_eq!(
        cut.section_of_stage(StageId::new(gone.stages.0)),
        None,
        "an id in the cut section lies in no section"
    );
}

// ───────────────────────────── 3 ─────────────────────────────

#[test]
fn every_site_items_query_is_items_at_in() {
    for (what, p) in [
        ("single-chip", single_chip()),
        ("tensor-parallel", tensor_parallel()),
    ] {
        let index = p.site_items();
        let mut sites: Vec<Site> = p
            .placement
            .as_ref()
            .expect("placed")
            .items_of_site
            .iter()
            .map(|(s, _)| *s)
            .collect();
        sites.push(Site {
            x: 1000,
            y: 1000,
            chip: 0,
        });
        let sections = (0..p.sections.len())
            .map(Some)
            .chain([None, Some(p.sections.len() + 5)]);
        let mut nonempty = 0usize;
        for k in sections {
            for site in &sites {
                let want = p.items_at_in(k, *site);
                assert_eq!(
                    index.at(k, *site),
                    want.as_slice(),
                    "{what}: the items of site {site:?} in section {k:?}"
                );
                nonempty += usize::from(!want.is_empty() && k.is_some());
            }
        }
        assert!(
            nonempty >= p.sections.len(),
            "{what}: every section runs items on some site ({nonempty} non-empty queries)"
        );
    }
}

// ───────────────────────────── 4 ─────────────────────────────

/// The definition: a section depends on every earlier section it reads a write of, or writes a read or a write of.
fn conflicts(a: &SectionTouch, b: &SectionTouch) -> bool {
    !a.writes.is_disjoint(&b.reads)
        || !a.writes.is_disjoint(&b.writes)
        || !a.reads.is_disjoint(&b.writes)
}

/// The definition of the placement, pairwise: each section, in order, takes the earliest phase past every earlier
/// section it depends on in which its chips are free.
fn pairwise_phase_order(t: &[SectionTouch]) -> (Vec<usize>, Vec<bool>) {
    let mut phase_of: Vec<usize> = Vec::new();
    let mut busy: Vec<BTreeSet<u16>> = Vec::new();
    for (j, tj) in t.iter().enumerate() {
        let after = (0..j)
            .filter(|&i| conflicts(&t[i], tj))
            .map(|i| phase_of[i] + 1)
            .max()
            .unwrap_or(0);
        let mut ph = after;
        while busy.get(ph).is_some_and(|b| !b.is_disjoint(&tj.chips)) {
            ph += 1;
        }
        if busy.len() <= ph {
            busy.resize(ph + 1, BTreeSet::new());
        }
        busy[ph].extend(tj.chips.iter().copied());
        phase_of.push(ph);
    }
    let mut order: Vec<usize> = (0..t.len()).collect();
    order.sort_by_key(|&k| (phase_of[k], k));
    let barrier_after = order
        .iter()
        .enumerate()
        .map(|(pos, &k)| {
            order
                .get(pos + 1)
                .is_some_and(|&next| phase_of[next] != phase_of[k])
        })
        .collect();
    (order, barrier_after)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn random_touches(rng: &mut Rng) -> Vec<SectionTouch> {
    const NAMES: [&str; 7] = ["x", "xn", "q", "k", "v", "S", "o"];
    let n = 1 + rng.below(40) as usize;
    let chips = 1 + rng.below(4) as u16;
    let names = 1 + rng.below(NAMES.len() as u64) as usize;
    let pick = |rng: &mut Rng, most: u64| -> BTreeSet<String> {
        (0..rng.below(most + 1))
            .map(|_| NAMES[rng.below(names as u64) as usize].to_string())
            .collect()
    };
    (0..n)
        .map(|_| SectionTouch {
            reads: pick(rng, 3),
            writes: pick(rng, 2),
            chips: (0..rng.below(u64::from(chips) + 1))
                .map(|_| rng.below(u64::from(chips)) as u16)
                .collect(),
        })
        .collect()
}

#[test]
fn the_phase_order_is_the_pairwise_rule_and_its_phases_are_sound() {
    let mut rng = Rng(0x5eed_2026_0925);
    let (mut shared, mut crossed) = (0usize, 0usize);
    for case in 0..4000 {
        let t = random_touches(&mut rng);
        let (order, barrier_after) = phase_order(&t);
        assert_eq!(
            (order.clone(), barrier_after.clone()),
            pairwise_phase_order(&t),
            "case {case}: {t:?}"
        );
        // Sound on its own terms: number the phases from the barriers and check them.
        let mut phase = vec![0usize; t.len()];
        let mut ph = 0usize;
        for (pos, &k) in order.iter().enumerate() {
            phase[k] = ph;
            ph += usize::from(barrier_after[pos]);
        }
        assert!(
            !barrier_after.last().copied().unwrap_or(false),
            "case {case}: no barrier after the last section"
        );
        for j in 0..t.len() {
            for i in 0..j {
                if phase[i] == phase[j] {
                    shared += 1;
                    assert!(
                        t[i].chips.is_disjoint(&t[j].chips),
                        "case {case}: sections {i} and {j} share a phase and a chip"
                    );
                    assert!(
                        !conflicts(&t[i], &t[j]) && !conflicts(&t[j], &t[i]),
                        "case {case}: sections {i} and {j} share a phase and data"
                    );
                } else if conflicts(&t[i], &t[j]) {
                    crossed += 1;
                    assert!(
                        phase[i] < phase[j],
                        "case {case}: section {j} depends on {i} and runs no later than it"
                    );
                }
            }
        }
    }
    assert!(shared > 10_000 && crossed > 10_000, "the random programs exercise both sharing a phase ({shared}) and dependencies across one ({crossed})");
}

// ───────────────────────────── 5 ─────────────────────────────

#[test]
fn the_links_are_the_pairwise_readers_and_the_state_is_in_first_appearance_order() {
    let mini = Qwen38Cfg::mini();
    let sources = [
        (
            "tiny decode step",
            gen_qwen38_decode_step_lg(&Qwen38Cfg::tiny(), "/w", 6, 3, 2).0,
        ),
        (
            "batched layer",
            gen_qwen38_decode_batch_layer_lg(&mini, 3, "/w", 6, &[0, 1, 2]).0,
        ),
        (
            "tensor-parallel layer",
            gen_qwen38_tp_decode_layer_lg(&mini, 0, "/w", 6, 2, 2).0,
        ),
        (
            "batched tensor-parallel layer",
            gen_qwen38_decode_batch_tp_layer_lg(&mini, 0, "/w", 6, &[1, 2], 4).0,
        ),
    ];
    for (what, src) in &sources {
        let stages = recognize_stages(src).expect("the source walks").stages;
        let got = links(&stages);
        assert_eq!(got.len(), stages.len(), "{what}: one link per stage");
        for (i, l) in got.iter().enumerate() {
            let want: Vec<usize> = (i + 1..stages.len())
                .filter(|&j| stages[j].ins.contains(&stages[i].out))
                .collect();
            assert_eq!(
                (l.producer, &l.tensor, &l.consumers),
                (i, &stages[i].out, &want),
                "{what}: the readers of stage {i}"
            );
        }
        assert!(
            got.iter().filter(|l| !l.consumers.is_empty()).count() * 2 > got.len(),
            "{what}: most results are read later"
        );
        let mut want: Vec<String> = Vec::new();
        for s in &stages {
            for pos in s.op.state_inputs() {
                if !want.contains(&s.ins[pos]) {
                    want.push(s.ins[pos].clone());
                }
            }
        }
        assert_eq!(state_tensors(&stages), want, "{what}: the carried state");
        assert!(!want.is_empty(), "{what}: a decode layer carries state");
    }
}

// ───────────────────────────── 6 ─────────────────────────────

fn assert_same(what: &str, a: &ProgramLowering, b: &ProgramLowering) {
    assert_eq!(
        (a.statements, a.lowered, a.sections, &a.state),
        (b.statements, b.lowered, b.sections, &b.state),
        "{what}: the counts"
    );
    assert_eq!(a.missing, b.missing, "{what}: what did not lower, and why");
    assert!(
        a.program == b.program,
        "{what}: the program and its certificate"
    );
}

#[test]
fn certifying_once_as_the_composition_is_certifying_each_section_then_the_composition() {
    let target = RduChip::Sn50.facts().spatial_target();
    let mini = Qwen38Cfg::mini();
    let none = std::collections::BTreeSet::new();
    let single = [
        (
            "tiny decode step",
            gen_qwen38_decode_step_lg(&Qwen38Cfg::tiny(), "/w", 6, 3, 2).0,
        ),
        (
            "batched layer",
            gen_qwen38_decode_batch_layer_lg(&mini, 0, "/w", 6, &[1, 2, 3]).0,
        ),
    ];
    for (what, src) in &single {
        let once = lower_program_with(src, &target, &none, None, SectionCertification::Composed)
            .expect("lowers");
        let each = lower_program_with(src, &target, &none, None, SectionCertification::EachSection)
            .expect("lowers");
        let whole = once.whole().expect("certifies whole");
        assert!(
            whole.certificate.as_ref().is_some_and(|c| c.recheck(whole)),
            "{what}: the composed certificate rechecks"
        );
        assert_same(what, &once, &each);
        assert_same(
            what,
            &compile_program_to_spatial(src, &target).expect("lowers"),
            &each,
        );
    }
    let parallel = [
        (
            "tensor-parallel layer",
            gen_qwen38_tp_decode_layer_lg(&mini, 3, "/w", 6, 2, 2),
        ),
        (
            "batched tensor-parallel layer",
            gen_qwen38_decode_batch_tp_layer_lg(&mini, 0, "/w", 6, &[1, 2], 4),
        ),
    ];
    for (what, (src, _, _, m)) in &parallel {
        let once = lower_program_with(src, &target, &none, Some(m), SectionCertification::Composed)
            .expect("lowers");
        let each = lower_program_with(
            src,
            &target,
            &none,
            Some(m),
            SectionCertification::EachSection,
        )
        .expect("lowers");
        assert!(once.whole().is_some(), "{what}: certifies whole");
        assert_same(what, &once, &each);
        assert_same(
            what,
            &compile_program_to_spatial_tp(src, &target, m).expect("lowers"),
            &each,
        );
    }
    // A write to a declared tensor that is not its fill: the program does not lower whole, and the answer names it.
    let partial = "## Main\nLet T be a 4 by 3 tensor of Int.\nSet item 4 of T to 40.\nLet tok be 2.\nLet v be 7.\nSet item 6 of T to v.\nLet x be row tok of T.\nShow item 3 of x.\n";
    let answer = compile_program_to_spatial(partial, &target).expect("a tensor program");
    assert!(
        answer.whole().is_none() && !answer.missing.is_empty(),
        "the stray write keeps the program from lowering whole: {:?}",
        answer.missing
    );
    assert_same(
        "a program that does not lower whole",
        &answer,
        &lower_program_with(
            partial,
            &target,
            &none,
            None,
            SectionCertification::EachSection,
        )
        .expect("lowers"),
    );
}
