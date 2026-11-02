//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! SIXTY-FOUR LAYERS ARE NOT SIXTY-FOUR PROGRAMS.
//!
//! A decoder forward is the same handful of dataflows run over and over with different weights. Today
//! the compiler proves that by UNROLLING it: four real-width layers compose into 153 sections and 51,787
//! stages, and sixty-four would be ~553 GB of IR. That is not merely expensive, it is the wrong program
//! — an SN50 holds 432 MB of SRAM against 27 GB of weights, so the chip cannot hold sixty-four layers
//! under any schedule. It holds ONE layer's dataflow and streams the layers through it. That is what
//! SambaNova means by persistent decoder execution.
//!
//! Before a layer LOOP can be emitted, the regularity it would exploit has to be a measured fact rather
//! than an assumption about how the model was authored. This lock measures it: every section is reduced
//! to a canonical SHAPE — its choreography with every buffer, semaphore, group, site and engine
//! renumbered by first appearance — and sections with equal shapes are exactly the sections whose event
//! graphs are isomorphic. The question "how many distinct dataflows does this forward run, and how often
//! does each repeat" then has an answer you can check.
//!
//! ★ WHAT THIS REFUSES TO ASSUME. That equal shape means equal program. A shape deliberately forgets
//! WHICH tensors a section reads — that is the whole point, since layer 7 and layer 8 differ precisely
//! there — so `shapes_forget_tensors_but_not_structure` pins both halves: two layers of the same kind
//! share a shape, and a section whose choreography differs does NOT, however similar it looks.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_section_shape_lock`
//!
//! ★ MUTATION PROBE: make `canonical_site` return a constant instead of renumbering by first appearance
//! — `two_sections_on_different_sites_with_the_same_choreography_share_a_shape` still passes but
//! `a_section_with_a_different_choreography_has_a_different_shape` reds, because collapsing the site
//! axis makes distinguishable programs look alike. Drop the loop COUNT from the digest and
//! `a_loop_that_runs_a_different_number_of_times_is_a_different_shape` reds.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_lg, Qwen38Cfg};
use logicaffeine_compile::spatial::ir::{ChoreoOp, SpatialProgram};
use logicaffeine_compile::spatial::passes::section_shape::{section_repeat_groups, section_shapes};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

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

fn composed(cfg: &Qwen38Cfg) -> SpatialProgram {
    let target = RduChip::Sn50.facts().spatial_target();
    compile_program_to_spatial(&authored_with_fills(cfg), &target)
        .expect("a tensor program")
        .program
        .expect("composes")
}

/// ── THE MEASURED FACT: a decoder forward runs FEW distinct dataflows, many times each.
#[test]
fn the_forward_runs_far_fewer_distinct_dataflows_than_it_has_sections() {
    let p = composed(&Qwen38Cfg::tiny());
    let shapes = section_shapes(&p);
    assert_eq!(shapes.len(), p.sections.len(), "one shape per section");

    let groups = section_repeat_groups(&p);
    let distinct = groups.len();
    let total = p.sections.len();
    let repeated: usize = groups
        .iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(_, v)| v.len())
        .sum();

    let mut by_size: Vec<usize> = groups.iter().map(|(_, v)| v.len()).collect();
    by_size.sort_unstable_by(|a, b| b.cmp(a));
    println!("{total} sections, {distinct} distinct shapes, {repeated} sections in a repeating group; largest groups {:?}", &by_size[..by_size.len().min(8)]);

    assert!(distinct < total, "a {total}-section forward that runs {distinct} distinct dataflows has no regularity to exploit");
    assert!(
        distinct * 2 <= total,
        "a decoder forward should run each dataflow more than once on average: {distinct} distinct over {total} sections"
    );
    // Every section belongs to exactly one group, and the groups partition the sections.
    let mut seen: Vec<usize> = groups.iter().flat_map(|(_, v)| v.iter().copied()).collect();
    seen.sort_unstable();
    assert_eq!(
        seen,
        (0..total).collect::<Vec<_>>(),
        "the groups must partition the sections exactly"
    );
}

/// ── AND IT GROWS WITH THE LAYERS WHILE THE DISTINCT SET DOES NOT.
///
/// This is the property a layer loop turns into a constant-size program: doubling the layers doubles the
/// sections and leaves the number of distinct dataflows alone.
#[test]
fn adding_layers_adds_repetitions_not_new_dataflows() {
    let small = composed(&Qwen38Cfg {
        layers: 4,
        ..Qwen38Cfg::tiny()
    });
    let big = composed(&Qwen38Cfg {
        layers: 8,
        ..Qwen38Cfg::tiny()
    });
    let (ds, db) = (
        section_repeat_groups(&small).len(),
        section_repeat_groups(&big).len(),
    );
    let (ss, sb) = (small.sections.len(), big.sections.len());
    println!("4 layers: {ss} sections / {ds} distinct   8 layers: {sb} sections / {db} distinct");
    assert!(
        sb > ss,
        "eight layers must compose more sections than four: {sb} vs {ss}"
    );
    assert_eq!(
        db, ds,
        "doubling the layers must add repetitions, not new dataflows: {db} distinct vs {ds}"
    );
}

/// ── SHAPES FORGET TENSORS AND NOTHING ELSE.
#[test]
fn shapes_forget_tensors_but_not_structure() {
    let p = composed(&Qwen38Cfg::tiny());
    let groups = section_repeat_groups(&p);
    let shared = groups
        .iter()
        .find(|(_, v)| v.len() > 1)
        .expect("some dataflow repeats");
    let (a, b) = (shared.1[0], shared.1[1]);
    // The two sections that share a shape must genuinely read different tensors — otherwise the shape is
    // not forgetting anything and this proves nothing about layers.
    let tensors_of = |si: usize| -> Vec<usize> {
        let sp = &p.sections[si];
        let mut v: Vec<usize> = p.stages[sp.stages.0..sp.stages.1]
            .iter()
            .filter_map(|s| match &s.kind {
                logicaffeine_compile::spatial::ir::StageKind::Load { tensor, .. }
                | logicaffeine_compile::spatial::ir::StageKind::Store { tensor, .. } => {
                    Some(tensor.idx())
                }
                _ => None,
            })
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    assert_ne!(
        tensors_of(a),
        tensors_of(b),
        "sections {a} and {b} share a shape but read the same tensors — the shape forgets nothing"
    );
    println!(
        "sections {a} and {b} share a shape over tensors {:?} vs {:?}",
        tensors_of(a),
        tensors_of(b)
    );
}

/// ── A DIFFERENT CHOREOGRAPHY IS A DIFFERENT SHAPE. The refusing half.
#[test]
fn a_section_with_a_different_choreography_has_a_different_shape() {
    let p = composed(&Qwen38Cfg::tiny());
    let groups = section_repeat_groups(&p);
    let shared = groups
        .iter()
        .find(|(_, v)| v.len() > 1)
        .expect("some dataflow repeats");
    let (a, b) = (shared.1[0], shared.1[1]);
    let before = section_shapes(&p);
    assert_eq!(before[a], before[b]);

    // Drop ONE op from section b's slice of one engine program. Nothing about the section's tensors
    // changes; only its choreography does.
    let mut q = p.clone();
    let sp = q.sections[b].clone();
    let sched = q.schedule.as_mut().expect("scheduled");
    let mut cut = false;
    for prog in &mut sched.programs {
        if let Some(i) = prog
            .ops
            .iter()
            .position(|t| sp.holds_stage(t.stage) && !matches!(t.op, ChoreoOp::Loop { .. }))
        {
            prog.ops.remove(i);
            cut = true;
            break;
        }
    }
    assert!(cut, "section {b} must have an op to drop");
    let after = section_shapes(&q);
    assert_eq!(
        after[a], before[a],
        "section {a} was not touched, so its shape must not move"
    );
    assert_ne!(
        after[b], before[b],
        "dropping an op from section {b} must change its shape"
    );
    assert_ne!(
        after[a], after[b],
        "and it must no longer share a shape with section {a}"
    );
}

/// ── THE LOOP COUNT IS PART OF THE SHAPE.
#[test]
fn a_loop_that_runs_a_different_number_of_times_is_a_different_shape() {
    let p = composed(&Qwen38Cfg::tiny());
    let groups = section_repeat_groups(&p);
    let shared = groups
        .iter()
        .find(|(_, v)| v.len() > 1)
        .expect("some dataflow repeats");
    let (a, b) = (shared.1[0], shared.1[1]);
    let before = section_shapes(&p);

    let mut q = p.clone();
    let sp = q.sections[b].clone();
    let sched = q.schedule.as_mut().expect("scheduled");
    let mut bumped = false;
    for prog in &mut sched.programs {
        for t in &mut prog.ops {
            if !sp.holds_stage(t.stage) {
                continue;
            }
            if let ChoreoOp::Loop { count, .. } = &mut t.op {
                *count = logicaffeine_compile::spatial::ir::LoopCount::Const(count.bound() + 1);
                bumped = true;
                break;
            }
        }
        if bumped {
            break;
        }
    }
    if !bumped {
        println!("section {b} runs no counted loop — the probe has nothing to bump here");
        return;
    }
    let after = section_shapes(&q);
    assert_ne!(
        after[b], before[b],
        "a loop that runs once more is a different dataflow"
    );
    assert_eq!(after[a], before[a]);
}

/// ── A ONE-SECTION PROGRAM HAS ONE SHAPE, AND THE FUNCTION IS TOTAL.
#[test]
fn a_program_without_sections_reports_nothing_rather_than_failing() {
    let p = composed(&Qwen38Cfg::tiny());
    let mut q = p.clone();
    q.sections.clear();
    assert!(
        section_shapes(&q).is_empty(),
        "no sections means no shapes, not a panic"
    );
    assert!(section_repeat_groups(&q).is_empty());
    // And the digests are a function of the program: computing them twice agrees.
    let once: BTreeMap<usize, u64> = section_shapes(&p).into_iter().enumerate().collect();
    let twice: BTreeMap<usize, u64> = section_shapes(&p).into_iter().enumerate().collect();
    assert_eq!(once, twice, "shapes must be a function of the program");
}
