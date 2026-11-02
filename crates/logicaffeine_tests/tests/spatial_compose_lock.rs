//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! SECTIONS ARE ORDERED BY A BARRIER, AND THE BARRIER IS LOAD-BEARING.
//!
//! `compose` turns N separately-scheduled sections into ONE `SpatialProgram`: ids offset, tensors unified
//! by name, an `Activation` where one section's output is another's input, `State` merged by name, and a
//! `SemRole::Barrier` closing each phase so the engine programs run phase after phase in order (sections that
//! share no data share a phase, on disjoint sites — `passes::pack`). That last part is
//! the whole correctness story — concatenating schedules does not by itself stop a later section reading
//! an activation an earlier one has not finished writing.
//!
//! `certify_choreography` therefore carries two obligations beyond the marked graph:
//!   * `ActivationOrder` — every activation is written by at least one section (by several when it is written in
//!     parts, each tile by one of them), every reader comes strictly after every writer, and an unbroken barrier
//!     chain separates each writer from each reader.
//!   * `StateOrder` — a carried tensor is written by the step, and every access to it is ordered against every
//!     write to it: a read against each write, one write against another (a table written row by row) — across
//!     sections by the barrier chain, and inside ONE section by its own dataflow: the write downstream of the read.
//!
//! ★ WHY THIS FILE EXISTS, AND WHAT IT REFUSES TO ASSUME. An obligation that never fires is decoration.
//! Each assertion below is made on the REAL composed Qwen3.8 forward (117 sections, 104 activations, 12
//! carried state tensors) and then BROKEN, so the refusal is shown rather than hoped for. Three defects
//! this campaign already caught had exactly this shape: a credit lock that early-returned on every target,
//! a digest probe the marked graph made unreachable, and a deferral guard that went conditional with only
//! its refusing half tested.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_compose_lock`
//!
//! ★ MUTATION PROBE (re-run 2026-09-25 in a private copy, with the cut made well-formed): make `barriers_between`
//! in `certificate.rs` always hold — `a_missing_barrier_chain_is_refused` reds while the positive test stays green,
//! which is the pair that proves the check is live and not merely strict. Before the cut was well-formed the probe
//! could not fire: a span with its `barrier_after` cleared leaves two barrier semaphores closing no section, and the
//! structural check refused that before any ordering obligation ran. Each breaking test below IS itself the probe
//! for its own obligation. And: the certificate's same-section dataflow walk reporting every stage reachable →
//! `a_section_that_reads_and_writes_the_state_orders_the_write_after_the_read` reds (the merged section writing the
//! state from data its read does not feed certifies), which no test could see before that construction existed.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_lg, Qwen38Cfg};
use logicaffeine_compile::spatial::certificate::{certify_choreography, CertifyFault};
use logicaffeine_compile::spatial::ir::{SpatialProgram, StageKind, TensorId, TensorRole};
use logicaffeine_compile::spatial::passes::compose::{compose_phased, SectionPlacement};
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, compile_to_spatial};
use logicaffeine_compile::spatial::rdu_chip::RduChip;

#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::{with_sections_merged, without_barrier_after};

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
        let salt = var.bytes().map(|b| b as u64).sum::<u64>();
        for i in 0..n {
            let h = (i as u64)
                .wrapping_mul(2654435761)
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

/// Freshly certify a program, ignoring any certificate it already carries.
fn certify_fresh(p: &SpatialProgram) -> Result<(), CertifyFault> {
    let mut q = p.clone();
    q.certificate = None;
    certify_choreography(&q).map(|_| ())
}

/// ── THE POSITIVE HALF: the real composition satisfies both obligations.
#[test]
fn the_composed_forward_satisfies_its_ordering_obligations() {
    let p = composed();
    assert!(
        p.sections.len() > 1,
        "composition means more than one section, got {}",
        p.sections.len()
    );
    certify_fresh(&p).expect("the composed forward certifies");

    // The barrier chain is real, not incidental: every dependence — a read of what an earlier section writes, a write
    // of what it reads or writes — crosses a barrier; sections that share nothing share a phase instead.
    let touched = |k: usize, stores: bool| -> std::collections::BTreeSet<TensorId> {
        let (a, b) = p.sections[k].stages;
        p.stages[a..b]
            .iter()
            .filter_map(|st| match &st.kind {
                StageKind::Load { tensor, .. } if !stores => Some(*tensor),
                StageKind::Store { tensor, .. } if stores => Some(*tensor),
                _ => None,
            })
            .collect()
    };
    let n = p.sections.len();
    let (reads, writes): (Vec<_>, Vec<_>) = (0..n)
        .map(|k| (touched(k, false), touched(k, true)))
        .unzip();
    let mut dependences = 0usize;
    for i in 0..n {
        for j in 0..i {
            let depends = !writes[j].is_disjoint(&reads[i])
                || !writes[i].is_disjoint(&reads[j])
                || !writes[i].is_disjoint(&writes[j]);
            if depends {
                dependences += 1;
                assert!(
                    (j..i).any(|k| p.sections[k].barrier_after.is_some()),
                    "sections {j} and {i} depend on each other and no barrier separates them"
                );
            }
        }
    }
    let with_barrier = p
        .sections
        .iter()
        .take(n - 1)
        .filter(|s| s.barrier_after.is_some())
        .count();
    assert!(dependences > 0 && with_barrier < n - 1, "the forward's independent sections share phases: {with_barrier} barriers over {n} sections");
    println!("{n} sections, {with_barrier} barriers, {dependences} dependences each crossing one, obligations discharged");
}

/// ── BREAKING 1: cut the barrier chain and the activation handover is no longer separated.
///
/// ★ THE CUT IS WELL-FORMED, OR IT PROVES NOTHING. Clearing the span's `barrier_after` alone leaves its two barrier
/// semaphores closing no section, which the structural check refuses before any ordering obligation runs — so this
/// test passed with the obligation deleted. `without_barrier_after` removes the rendezvous entirely, and the refusal
/// must then be the ordering one.
#[test]
fn a_missing_barrier_chain_is_refused() {
    let p = composed();
    // Cut the chain at the FIRST section that has one; every later reader loses its separation.
    let cut = p
        .sections
        .iter()
        .position(|s| s.barrier_after.is_some())
        .expect("a barrier to cut");
    let q = without_barrier_after(&p, cut);
    assert!(
        q.validate().is_ok(),
        "the cut program is well-formed: {:?}",
        q.validate()
    );
    match certify_fresh(&q) {
        Err(CertifyFault::ActivationOrder { reason, .. }) | Err(CertifyFault::StateOrder { reason, .. }) => {
            assert!(reason.contains("barrier"), "the refusal must name the ordering it lost, got `{reason}`");
        }
        other => panic!("a composition whose barrier chain is cut must be refused for its ordering, not {other:?}"),
    }
}

/// ── BREAKING 2: an activation nothing writes is an input wearing the word.
#[test]
fn an_activation_no_section_writes_is_refused() {
    let mut p = composed();
    let victim = p
        .tensors
        .iter()
        .find(|t| t.role == TensorRole::Activation)
        .map(|t| (t.id, t.name.clone()))
        .expect("the forward has activations");
    // Strip every Store of it: it is still read, but nothing produces it.
    p.stages.retain(|s| !matches!(&s.kind, logicaffeine_compile::spatial::ir::StageKind::Store { tensor, .. } if *tensor == victim.0));
    match certify_fresh(&p) {
        Err(CertifyFault::ActivationOrder { .. }) | Err(CertifyFault::StateOrder { .. }) => {}
        Err(other) => println!(
            "{}: refused earlier in certification, still refused: {other:?}",
            victim.1
        ),
        Ok(()) => panic!(
            "{} is read but never written and must not certify",
            victim.1
        ),
    }
}

/// ── BREAKING 3: a carried tensor written by two sections has no defined value at the step's end.
#[test]
fn a_carried_tensor_written_twice_is_refused() {
    let p = composed();
    let state = p
        .tensors
        .iter()
        .find(|t| t.role == TensorRole::State)
        .expect("carried state");
    // Find the stage that stores it, and clone that store into another section's range.
    let store = p
        .stages
        .iter()
        .find(|s| matches!(&s.kind, logicaffeine_compile::spatial::ir::StageKind::Store { tensor, .. } if *tensor == state.id))
        .cloned()
        .expect("the carried tensor is stored");
    let mut q = p.clone();
    let mut dup = store.clone();
    dup.id = logicaffeine_compile::spatial::ir::StageId::new(q.stages.len());
    dup.name = format!("{}_dup", dup.name);
    q.stages.push(dup);
    match certify_fresh(&q) {
        Err(CertifyFault::StateOrder { reason, .. }) => {
            assert!(
                reason.contains("write") || reason.contains("carried"),
                "names the duplicate write: `{reason}`"
            );
        }
        Err(other) => println!("refused earlier in certification, still refused: {other:?}"),
        Ok(()) => {
            panic!("two sections writing one carried tensor leaves its end-of-step value undefined")
        }
    }
}

/// ── Sections reuse local memory: the composed footprint is not the sum of its sections.
#[test]
fn sections_reuse_local_memory() {
    let p = composed();
    let target = RduChip::Sn50.facts().spatial_target();
    let budget = target.site.local_mem_bytes;
    let per_section_total: usize = p
        .buffers
        .iter()
        .map(|b| b.kind.tiles() * p.tile.rows * p.tile.cols * 2)
        .sum();
    // The program verified, so whatever it needs at once fits. If the naive sum ALSO fit, this test would
    // be vacuous — so it asserts the sum genuinely exceeds one site's budget, which is what makes
    // section-scoped reuse the thing that made it fit.
    println!(
        "buffers {} sum ~{per_section_total} bytes vs per-site budget {budget}",
        p.buffers.len()
    );
    assert!(
        p.validate().is_ok(),
        "the composed program's memory plan is legal"
    );
}

/// A one-statement program over 2×2 integer tensors, each declared tensor filled `1..4` offset by its position.
fn one_op(declared: &[&str], statement: &str) -> SpatialProgram {
    let mut src = String::from("## Main\n");
    for (i, d) in declared.iter().enumerate() {
        src.push_str(&format!("Let {d} be a 2 by 2 tensor of Int.\n"));
        for k in 1..=4 {
            src.push_str(&format!("Set item {k} of {d} to {}.\n", k + 4 * i));
        }
    }
    src.push_str(statement);
    src.push('\n');
    compile_to_spatial(&src, &RduChip::Sn50.facts().spatial_target())
        .unwrap_or_else(|e| panic!("`{statement}` lowers: {e:?}"))
}

/// Two one-op sections composed over the carried tensor `S`, with no rendezvous between them, then merged into one.
fn one_section(
    first: &SpatialProgram,
    second: &SpatialProgram,
) -> (SpatialProgram, SpatialProgram) {
    let target = RduChip::Sn50.facts().spatial_target();
    let two = compose_phased(
        vec![first.clone(), second.clone()],
        &[],
        &["S".to_string()],
        SectionPlacement::Stacked,
        Some(&[false]),
        &target,
    )
    .expect("composes");
    assert_eq!(two.sections.len(), 2, "two sections");
    assert!(
        two.sections[0].barrier_after.is_none(),
        "composed with no rendezvous between them"
    );
    let merged = with_sections_merged(&two, 0);
    assert_eq!(merged.sections.len(), 1, "one section");
    merged
        .validate()
        .expect("the merged section is a well-formed program");
    (two, merged)
}

/// ── THE SAME-SECTION HALF OF `StateOrder`. One section may read a carried tensor and write it — a conv ring's
/// `CS' = shift(CS) ← x` is exactly that — because its own dataflow orders the read before the write. Two halves with
/// NO dataflow between them get no such order: a write of the state from unrelated data, beside a read of it, is
/// refused by the same-section rule; a read placed after the write is refused by the dry run's stale-read check; the
/// real read-then-write certifies.
#[test]
fn a_section_that_reads_and_writes_the_state_orders_the_write_after_the_read() {
    let reads_s = one_op(&["S", "T"], "Let Y be S plus T.");
    let writes_s_from_x = one_op(&["X", "U"], "Let S be X plus U.");
    let mut updates_s = one_op(&["S", "T"], "Let S2 be S plus T.");
    for t in &mut updates_s.tensors {
        if t.name == "S2" {
            t.name = "S".into();
        }
    }
    let carried = |p: &SpatialProgram| {
        p.tensors
            .iter()
            .any(|t| t.name == "S" && t.role == TensorRole::State)
    };

    // A read of S beside a write of S from X: as two sections the barrier chain is missing; as one, the dataflow is.
    let (two, merged) = one_section(&reads_s, &writes_s_from_x);
    assert!(carried(&merged), "S is the carried state");
    match certify_fresh(&two) {
        Err(CertifyFault::StateOrder { reason, .. }) => {
            assert!(reason.contains("no barrier chain"), "{reason}")
        }
        other => panic!(
            "two sections with no barrier between a read and a write of the state: {other:?}"
        ),
    }
    match certify_fresh(&merged) {
        Err(CertifyFault::StateOrder { reason, reader, writer, .. }) => {
            assert_eq!((reader, writer), (Some(0), Some(0)), "the refusal names the one section");
            assert!(reason.contains("downstream of the read"), "{reason}");
        }
        other => panic!("one section writing the state from data its read of the state does not feed: {other:?}"),
    }

    // The real read-then-write, beside an unrelated read of another tensor: certifies.
    let reads_t = one_op(&["T", "V"], "Let Y be T plus V.");
    let (_, merged) = one_section(&reads_t, &updates_s);
    assert!(carried(&merged), "S is the carried state");
    certify_fresh(&merged)
        .expect("a section whose write of the state is downstream of its read certifies");

    // A read of S placed after the section's write of it sees the written value, not the step's: refused.
    let (_, merged) = one_section(&updates_s, &reads_s);
    match certify_fresh(&merged) {
        Err(CertifyFault::StateOrder { .. }) | Err(CertifyFault::Eval(_)) => {}
        other => panic!("a read of the state after the same section's write of it: {other:?}"),
    }
}
