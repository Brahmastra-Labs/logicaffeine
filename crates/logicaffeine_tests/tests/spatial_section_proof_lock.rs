//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE LIVENESS PROOF IS CUT ALONG THE SECTIONS, AND THE CUT IS EXACT.
//!
//! The unrolled event graph of a FOUR-layer Qwen3.8 forward is 196,211,828 places and 102,936,753
//! transitions — 35 GB, two thirds of peak, and the reason a sixty-four-layer forward did not fit on a
//! 125 GB box. That size is not irreducible. A composed forward is a chain of sections joined only by
//! the program-counter arcs: no buffer, no semaphore and no multicast group is touched from two of them.
//! So the graph is a disjoint union, Commoner's condition holds on the union iff it holds on each part,
//! and a certificate can carry one witness per section instead of one graph for everything.
//!
//! ★ WHAT THIS FILE REFUSES TO ASSUME. That the decomposition is free. A cut that quietly dropped a
//! dependence would prove a deadlocking program live — the single worst thing this compiler could do —
//! so the exactness is not argued here, it is MEASURED against the whole graph built the old way:
//! every transition survives, and the only places that appear are the extra program-counter endpoints
//! the cut creates. The preconditions are checked rather than trusted, and each one is broken below to
//! show the checker refuses instead of guessing.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_section_proof_lock`
//!
//! ★ MUTATION PROBE: make `section_ranges` skip its resource-ownership loop (never return `None` for a
//! shared buffer or semaphore) — `a_resource_shared_across_sections_forces_the_whole_form` reds while
//! every other test here stays green. Make it skip the non-decreasing-order check — `sections_out_of_
//! order_force_the_whole_form` reds. Drop the `verify_witness` call from the per-section arm of
//! `recheck_proof` — `a_tampered_section_witness_is_refused` reds.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_lg, Qwen38Cfg};
use logicaffeine_compile::spatial::certificate::{
    build_marked_graph, certify_choreography, decomposes_by_section, GraphProof,
};
use logicaffeine_compile::spatial::ir::{ChoreoOp, SemId, SpatialProgram};
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, compile_to_spatial};
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::target::SpatialTarget;
use spatial_support::gemm_program;

fn authored_with_fills(cfg: &Qwen38Cfg) -> String {
    let (authored, _f, _q) = gen_qwen38_lg(cfg, "/w");
    let mut src = String::new();
    for line in authored.lines() {
        src.push_str(line);
        src.push('\n');
        let Some(rest) = line.strip_prefix("Let ") else {
            continue;
        };
        let Some((var, tail)) = rest.split_once(" be a ") else {
            continue;
        };
        let Some(dims) = tail.strip_suffix(" tensor of Real.") else {
            continue;
        };
        let (r, cc) = dims.split_once(" by ").expect("R by C");
        let n = r.parse::<usize>().unwrap() * cc.parse::<usize>().unwrap();
        let salt = var.bytes().map(u64::from).sum::<u64>();
        for i in 0..n {
            let h = (i as u64)
                .wrapping_mul(2_654_435_761)
                .wrapping_add(salt.wrapping_mul(40503));
            let v = ((h % 97) as f64 / 97.0 - 0.5) * 0.4;
            src.push_str(&format!("Set item {} of {var} to {v:?}.\n", i + 1));
        }
    }
    src
}

fn composed() -> SpatialProgram {
    let src = authored_with_fills(&Qwen38Cfg::tiny());
    let target = RduChip::Sn50.facts().spatial_target();
    compile_program_to_spatial(&src, &target)
        .expect("a tensor program")
        .program
        .expect("composes")
}

fn fresh(
    p: &SpatialProgram,
) -> logicaffeine_compile::spatial::certificate::ChoreographyCertificate {
    let mut q = p.clone();
    q.certificate = None;
    certify_choreography(&q).expect("certifies")
}

/// ── THE FORM. A composed forward is proved one section at a time, and it re-checks.
#[test]
fn the_composed_forward_is_proved_one_section_at_a_time() {
    let p = composed();
    assert!(
        p.sections.len() > 1,
        "composition means more than one section"
    );
    assert!(
        decomposes_by_section(&p),
        "no place crosses a section boundary on the composed forward"
    );

    let c = fresh(&p);
    let GraphProof::PerSection {
        sections,
        places,
        transitions,
        ..
    } = &c.proof
    else {
        panic!(
            "a {}-section forward must be proved per section, not whole",
            p.sections.len()
        );
    };
    assert_eq!(sections.len(), p.sections.len(), "one proof per section");
    assert_eq!(sections.iter().map(|s| s.places).sum::<usize>(), *places);
    assert_eq!(
        sections.iter().map(|s| s.transitions).sum::<usize>(),
        *transitions
    );
    assert!(
        sections.iter().all(|s| s.transitions > 0),
        "no section is proved vacuously"
    );
    assert!(c.recheck(&p), "the per-section proof must re-check");

    // The point of the whole exercise: peak is the LARGEST section, not the sum of them.
    let widest = sections.iter().map(|s| s.transitions).max().unwrap_or(0);
    assert!(
        widest * 4 < *transitions,
        "a {}-section proof whose widest section is {widest} of {transitions} transitions has not bought anything",
        sections.len()
    );
    println!(
        "{} sections, {places} places / {transitions} transitions total, widest section {widest} transitions",
        sections.len()
    );
}

/// ── THE CUT IS EXACT. Every transition of the whole graph survives, and the only places the split adds
/// are the extra program-counter endpoints — one per section boundary per engine program that spans it.
#[test]
fn the_section_cut_loses_no_transition_and_adds_only_program_counter_places() {
    let p = composed();
    let c = fresh(&p);
    let GraphProof::PerSection {
        sections,
        places,
        transitions,
        ..
    } = &c.proof
    else {
        panic!("per-section form")
    };

    let whole = build_marked_graph(&p).expect("the whole graph still builds for the tiny forward");
    assert_eq!(
        *transitions,
        whole.transitions.len(),
        "the cut partitions the events: every transition of the whole graph belongs to exactly one section"
    );

    // Each engine program contributes `ops + 1` program-counter places whole, and `ops + (sections it
    // spans)` split — so the surplus is exactly one per extra section each program touches.
    let sched = p.schedule.as_ref().expect("scheduled");
    let mut extra = 0usize;
    for prog in &sched.programs {
        let mut touched: BTreeSet<usize> = BTreeSet::new();
        for t in &prog.ops {
            for (si, sp) in p.sections.iter().enumerate() {
                if sp.holds_stage(t.stage) {
                    touched.insert(si);
                }
            }
        }
        extra += touched.len().saturating_sub(1);
    }
    assert_eq!(
        *places,
        whole.places.len() + extra,
        "the split must add exactly {extra} program-counter endpoints and nothing else \
         (split {places}, whole {}, surplus {})",
        whole.places.len(),
        *places as i64 - whole.places.len() as i64
    );
    println!(
        "whole {} places / {} transitions; split {places} / {transitions} (+{extra} Pc endpoints)",
        whole.places.len(),
        whole.transitions.len()
    );
    assert_eq!(sections.len(), p.sections.len());
}

/// ── THE WITNESS IS LOAD-BEARING IN THE NEW FORM TOO. This is the probe that would silently lose its
/// teeth if the per-section arm forgot to verify what it carries.
#[test]
fn a_tampered_section_witness_is_refused() {
    let p = composed();
    let c = fresh(&p);
    assert!(
        matches!(c.proof, GraphProof::PerSection { .. }),
        "the tamper must exercise the per-section arm"
    );
    assert!(c.recheck(&p));
    let t = c.with_witness_tampered();
    assert_ne!(
        t.proof, c.proof,
        "the tamper must actually change the carried proof"
    );
    assert!(
        !t.recheck(&p),
        "a permuted section witness must fail recheck"
    );
}

/// ── A SINGLE-SECTION PROGRAM KEEPS THE OLD PROOF, GRAPH AND ALL.
#[test]
fn a_single_section_program_is_still_proved_whole() {
    for target in SpatialTarget::presets() {
        let p = compile_to_spatial(&gemm_program(64, 64, 64), &target).expect("a gemm lowers");
        assert!(
            !decomposes_by_section(&p),
            "{}: one section does not decompose",
            target.name
        );
        let c = p.certificate.as_ref().expect("a certificate");
        let g = c.proof.whole_graph().unwrap_or_else(|| {
            panic!(
                "{}: a single-section program carries its graph",
                target.name
            )
        });
        assert_eq!(c.places(), g.places.len());
        assert_eq!(c.transitions(), g.transitions.len());
        assert!(c.recheck(&p));
        assert!(
            !c.with_witness_tampered().recheck(&p),
            "{}: the whole-form tamper still bites",
            target.name
        );
    }
}

/// Retarget the first `SemInc` whose stage sits in a late section onto a semaphore an early section owns,
/// so one semaphore is touched from two sections. Returns the pair it joined.
fn share_a_semaphore_across_sections(p: &mut SpatialProgram) -> Option<(SemId, usize, usize)> {
    let sections = p.sections.clone();
    let sched = p.schedule.as_mut()?;
    let early = sched.programs.iter().find_map(|pr| {
        pr.ops.iter().find_map(|t| match &t.op {
            ChoreoOp::SemInc { sem, .. }
                if sections.first().is_some_and(|s| s.holds_stage(t.stage)) =>
            {
                Some(*sem)
            }
            _ => None,
        })
    })?;
    for pr in &mut sched.programs {
        for t in &mut pr.ops {
            let Some(si) = sections.iter().position(|s| s.holds_stage(t.stage)) else {
                continue;
            };
            if si == 0 {
                continue;
            }
            if let ChoreoOp::SemInc { sem, .. } = &mut t.op {
                if *sem == early {
                    continue;
                }
                *sem = early;
                return Some((early, 0, si));
            }
        }
    }
    None
}

/// ── BREAKING 1: a semaphore touched from two sections is exactly the arc the cut would hide, so the
/// checker must refuse to cut at all and fall back to proving the program whole.
#[test]
fn a_resource_shared_across_sections_forces_the_whole_form() {
    let mut p = composed();
    assert!(decomposes_by_section(&p), "it decomposes before the tamper");
    let joined = share_a_semaphore_across_sections(&mut p).expect(
        "the composed forward has a SemInc in its first section and in a later one to join",
    );
    assert!(
        !decomposes_by_section(&p),
        "semaphore {:?} is now touched from sections {} and {} — the proof must not be cut",
        joined.0,
        joined.1,
        joined.2
    );
}

/// ── BREAKING 2: ops that run out of section order are not a contiguous slice of anything, so the
/// checker refuses rather than inventing a range.
#[test]
fn sections_out_of_order_force_the_whole_form() {
    let mut p = composed();
    assert!(decomposes_by_section(&p));
    let n = p.sections.len();
    assert!(n >= 2);
    p.sections.swap(0, n - 1);
    assert!(
        !decomposes_by_section(&p),
        "ops running against section order must refuse the cut"
    );
}

/// ── BREAKING 3: a stage claimed by two sections has no section to be proved in.
#[test]
fn overlapping_section_spans_force_the_whole_form() {
    let mut p = composed();
    assert!(decomposes_by_section(&p));
    let end = p.sections[0].stages.1;
    p.sections[1].stages.0 = p.sections[0].stages.0;
    assert!(p.sections[1].stages.0 < end, "the spans now overlap");
    assert!(
        !decomposes_by_section(&p),
        "a stage in two sections must refuse the cut"
    );
}

fn composed_layers(layers: usize) -> SpatialProgram {
    let cfg = Qwen38Cfg {
        layers,
        ..Qwen38Cfg::tiny()
    };
    let src = logicaffeine_compile::codegen_cuda::qwen38::gen_qwen38_filled_lg(&cfg, "/w");
    let target = RduChip::Sn50.facts().spatial_target();
    compile_program_to_spatial(&src, &target)
        .expect("a tensor program")
        .program
        .expect("composes")
}

/// ── THE WITNESS TABLE DOES NOT GROW WITH DEPTH. A decoder repeats its dataflows, so doubling the layers
/// doubles the sections and adds no new witness. This is what keeps a 64-layer certificate from being
/// sixteen times a 4-layer one.
#[test]
fn the_witness_table_does_not_grow_with_depth() {
    let (a, b) = (fresh(&composed_layers(4)), fresh(&composed_layers(8)));
    let table =
        |c: &logicaffeine_compile::spatial::certificate::ChoreographyCertificate| match &c.proof {
            GraphProof::PerSection {
                sections,
                witnesses,
                ..
            } => (sections.len(), witnesses.len()),
            GraphProof::Whole { .. } => panic!("a composed forward is proved per section"),
        };
    let ((s4, w4), (s8, w8)) = (table(&a), table(&b));
    println!(
        "4 layers: {s4} sections / {w4} witnesses    8 layers: {s8} sections / {w8} witnesses"
    );
    assert!(s8 > s4, "eight layers compose more sections than four");
    assert!(
        w4 < s4,
        "sections that share a dataflow share a witness: {w4} witnesses for {s4} sections"
    );
    assert_eq!(
        w8, w4,
        "doubling the layers must not add witnesses: {w4} -> {w8}"
    );
}

/// ★ MUTATION PROBE (run when this landed): make `recheck` accept a section whose graph fits ANY witness
/// in the table rather than the one it names — this test alone reds, the other eight stay green.
///
/// ── AND SHARING NEVER VOUCHES FOR A SECTION. Point one section at a DIFFERENT entry of the table: every
/// entry is a valid witness for some section, so a checker that only asked "is this a real witness" would
/// accept it. `recheck` verifies each section's own rebuilt graph against the witness that section names,
/// so it must refuse.
#[test]
fn a_section_pointed_at_another_sections_witness_is_refused() {
    let p = composed();
    let c = fresh(&p);
    assert!(c.recheck(&p));
    let mut t = c.clone();
    let GraphProof::PerSection {
        sections,
        witnesses,
        ..
    } = &mut t.proof
    else {
        panic!("per-section form")
    };
    assert!(
        witnesses.len() > 1,
        "the table must hold more than one witness for this probe to mean anything"
    );
    let (i, other) = sections
        .iter()
        .enumerate()
        .find_map(|(i, sp)| {
            (0..witnesses.len())
                .find(|w| {
                    *w != sp.witness
                        && witnesses[*w].topo_order.len() == witnesses[sp.witness].topo_order.len()
                        && witnesses[*w] != witnesses[sp.witness]
                })
                .map(|w| (i, w))
        })
        .or_else(|| {
            sections.iter().enumerate().find_map(|(i, sp)| {
                (0..witnesses.len())
                    .find(|w| *w != sp.witness)
                    .map(|w| (i, w))
            })
        })
        .expect("some section can be pointed elsewhere");
    sections[i].witness = other;
    assert!(!t.recheck(&p), "section {i} now names witness {other}, which is not an order of its graph — recheck must refuse");
    let mut u = c.clone();
    let GraphProof::PerSection {
        sections,
        witnesses,
        ..
    } = &mut u.proof
    else {
        unreachable!()
    };
    sections[0].witness = witnesses.len();
    assert!(
        !u.recheck(&p),
        "a witness index past the end of the table must be refused, not panic"
    );
}
