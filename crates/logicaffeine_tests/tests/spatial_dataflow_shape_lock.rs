//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE SHAPE OF THE FORWARD AGAINST THE SHAPE OF ITS DEPENDENCES — the ratchet toward dataflow.
//!
//! `compose` separates the sections with global rendezvous. On a spatial dataflow chip that is the expensive
//! shape: the premise of the machine is that stage k+1 begins on stage k's FIRST tile; a barrier makes it
//! wait for k's LAST, so the fabric drains once per barrier per token. Tokens per second is decided here more
//! than anywhere else in the compiler. Once a barrier followed EVERY section — 116 for the 117-section Qwen3.8
//! forward; now sections that share no data run in one PHASE on disjoint sites (`passes::pack`), and the
//! forward runs in 65 phases with 64 barriers.
//!
//! ★ AND THE OBVIOUS METRIC IS THE WRONG ONE. Counting barriers whose two ADJACENT sections do not
//! depend on each other gives 54 of 116, which reads like half the rendezvous are free to delete. They
//! are not. A barrier orders everything before it against everything after it, so the chain is what
//! keeps a producer in section 3 ahead of a consumer in section 21 — and this forward has producers as
//! far as 18 sections back. Asking the right question instead — does ANY dependence cross this cut —
//! gives ZERO removable barriers out of 116. Every one is load-bearing transitively, and deleting any
//! is a wrong answer rather than an optimization.
//!
//! ★ THE REAL PRIZE IS SHAPE, NOT COUNT. The 148 producer→consumer edges form a graph only 65 deep. Run
//! section after section, the forward was 1.80x longer than its own dependences require; packed into phases
//! it is 1.00x — exactly as long as its critical path, before any tile-level pipelining is considered. That
//! ratio — phases over critical-path depth — is what this file ratchets, because it is the number a dataflow
//! compose actually moves and the one that cannot be gamed by removing ordering: every dependence (a read of
//! what an earlier section writes, a write of what it reads or writes) must still cross a barrier.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_dataflow_shape_lock`
//!
//! ★ MUTATION PROBE: make `compose` emit a barrier only for even-numbered sections — the serialization
//! ratio is untouched (it measures the DEPENDENCE graph, not the barriers) while `no_barrier_in_the_
//! chain_orders_nothing` still reads zero, and `the_dependence_carrying_barriers_are_still_there` reds
//! on the missing handovers. That is the point of splitting the three: the ratchet cannot be satisfied
//! by removing ordering, because the thing it measures is not the ordering.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_lg, Qwen38Cfg};
use logicaffeine_compile::spatial::ir::{SpatialProgram, StageKind, TensorId};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

/// How many times longer the composed schedule is than its own dependence graph is deep, in hundredths: its PHASES
/// against the depth. 100 = the forward runs in exactly as many phases as its dependences are deep — the floor, since
/// no schedule is shorter than its critical path. (It was 180 while every section had a phase of its own.) MAY ONLY
/// FALL.
const SERIALIZATION_CEILING: usize = 100;

/// Barriers that no dependence crosses, and which a dataflow compose could therefore drop outright.
/// Zero today: every barrier in the chain is load-bearing transitively. MAY ONLY RISE — if this ever
/// exceeds zero, `compose` is emitting rendezvous that order nothing.
const REMOVABLE_BARRIERS: usize = 0;

fn composed() -> SpatialProgram {
    let (authored, _f, _q) = gen_qwen38_lg(&Qwen38Cfg::tiny(), "/w");
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
            src.push_str(&format!(
                "Set item {} of {var} to {:?}.\n",
                i + 1,
                ((h % 97) as f64 / 97.0 - 0.5) * 0.4
            ));
        }
    }
    let target = RduChip::Sn50.facts().spatial_target();
    compile_program_to_spatial(&src, &target)
        .expect("a tensor program")
        .program
        .expect("composes")
}

/// `(reads, writes)` per section, by tensor.
fn flow(p: &SpatialProgram) -> (Vec<BTreeSet<TensorId>>, Vec<BTreeSet<TensorId>>) {
    let mut reads = vec![BTreeSet::new(); p.sections.len()];
    let mut writes = vec![BTreeSet::new(); p.sections.len()];
    for (si, sp) in p.sections.iter().enumerate() {
        for st in &p.stages[sp.stages.0.min(p.stages.len())..sp.stages.1.min(p.stages.len())] {
            match &st.kind {
                StageKind::Load { tensor, .. } => {
                    reads[si].insert(*tensor);
                }
                StageKind::Store { tensor, .. } => {
                    writes[si].insert(*tensor);
                }
                _ => {}
            }
        }
    }
    (reads, writes)
}

/// The composed program's phases: runs of sections a barrier closes — what actually runs one after another.
fn phases(p: &SpatialProgram) -> usize {
    1 + p
        .sections
        .iter()
        .take(p.sections.len().saturating_sub(1))
        .filter(|s| s.barrier_after.is_some())
        .count()
}

/// Whether section `i` depends on the earlier section `j`: it reads what `j` writes, or writes what `j` reads or writes.
fn depends(
    reads: &[BTreeSet<TensorId>],
    writes: &[BTreeSet<TensorId>],
    j: usize,
    i: usize,
) -> bool {
    writes[j].intersection(&reads[i]).next().is_some()
        || writes[i].intersection(&reads[j]).next().is_some()
        || writes[i].intersection(&writes[j]).next().is_some()
}

/// ── THE RATCHET. The schedule may only get closer to the shape of its own dependences: its PHASES — what runs one
/// after another — against the depth of its producer→consumer graph.
#[test]
fn the_forward_only_gets_closer_to_its_critical_path() {
    let p = composed();
    let (reads, writes) = flow(&p);
    let n = p.sections.len();
    // Every true producer→consumer edge, and the longest chain through them.
    let mut depth = vec![0usize; n];
    let mut edges = 0usize;
    for i in 0..n {
        let mut best = 0usize;
        for j in 0..i {
            if writes[j].intersection(&reads[i]).next().is_some() {
                edges += 1;
                best = best.max(depth[j] + 1);
            }
        }
        depth[i] = best;
    }
    let critical = depth.iter().copied().max().unwrap_or(0) + 1;
    let serial = phases(&p);
    let ratio = serial * 100 / critical.max(1);
    println!("{n} sections in {serial} phases over {edges} dependence edges; critical path {critical} deep => {}.{:02}x longer than required", ratio / 100, ratio % 100);
    assert!(
        ratio <= SERIALIZATION_CEILING,
        "the forward got MORE serialized relative to its dependences: {}.{:02}x against a ceiling of {}.{:02}x",
        ratio / 100,
        ratio % 100,
        SERIALIZATION_CEILING / 100,
        SERIALIZATION_CEILING % 100
    );
}

/// ── AND NO BARRIER ORDERS NOTHING. A rendezvous no dependence crosses is pure cost.
#[test]
fn no_barrier_in_the_chain_orders_nothing() {
    let p = composed();
    let (reads, writes) = flow(&p);
    let n = p.sections.len();
    let edges: Vec<(usize, usize)> = (0..n)
        .flat_map(|i| (0..i).map(move |j| (j, i)))
        .filter(|(j, i)| depends(&reads, &writes, *j, *i))
        .collect();
    let barriers: Vec<usize> = (0..n.saturating_sub(1))
        .filter(|k| p.sections[*k].barrier_after.is_some())
        .collect();
    let removable = barriers
        .iter()
        .filter(|k| !edges.iter().any(|(j, i)| j <= *k && *k < i))
        .count();
    println!(
        "{removable} of {} barriers are crossed by no dependence",
        barriers.len()
    );
    assert_eq!(
        removable, REMOVABLE_BARRIERS,
        "compose is emitting {removable} rendezvous that order nothing — each one drains the fabric for free"
    );
}

/// ── AND THE REAL ONES ARE STILL THERE. The ceiling must not be satisfiable by deleting barriers: every dependence —
/// a read of what an earlier section writes, a write of what it reads or writes — crosses a barrier.
#[test]
fn the_dependence_carrying_barriers_are_still_there() {
    let p = composed();
    let (reads, writes) = flow(&p);
    let n = p.sections.len();
    let edges: Vec<(usize, usize)> = (0..n)
        .flat_map(|i| (0..i).map(move |j| (j, i)))
        .filter(|(j, i)| depends(&reads, &writes, *j, *i))
        .collect();
    assert!(
        !edges.is_empty(),
        "a forward whose sections never feed each other is not a forward"
    );
    for (j, i) in &edges {
        assert!(
            (*j..*i).any(|k| p.sections[k].barrier_after.is_some()),
            "sections {j} and {i} depend on each other and no barrier separates them"
        );
    }
}

/// ── HOW FAR THE WORK COULD MOVE. A section whose producer sits several sections back is a section
/// that is waiting on strangers. Recorded, because it is the size of the prize a dataflow compose wins.
#[test]
fn the_distance_to_a_real_producer_is_recorded() {
    let p = composed();
    let (reads, writes) = flow(&p);
    let mut far = 0usize;
    let mut worst = 0usize;
    for si in 0..p.sections.len() {
        let nearest = (0..si)
            .filter(|j| writes[*j].intersection(&reads[si]).next().is_some())
            .max();
        if let Some(j) = nearest {
            let d = si - j;
            worst = worst.max(d);
            if d > 1 {
                far += 1;
            }
        }
    }
    println!("{far} sections wait on a producer more than one section back; the furthest is {worst} back");
    assert!(worst >= 1, "a composed forward has producers");
}
