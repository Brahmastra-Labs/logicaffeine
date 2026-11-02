//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! HANDSHAKE LOCK — semaphore choreography is DERIVED from the buffer/edge graph, per target primitive.
//!
//! `synthesize_handshakes` turns every edge's handshake kind into the exact per-engine op sequence: FIFO
//! reserve/push/wait/pop on local edges; for a multicast edge the sender's ready-wait / reset / data
//! multicast / valid-flag multicast and the receiver's invalidate / ready-increment / valid-wait; the
//! compute engine's acquire / reload / compute / pack / release with a peeled first and last K block; the
//! writer's wait / write / barrier / pop. `persist` wraps the per-block bodies in the K loop and, where a
//! site owns several work items, an outer item loop with affine tile addresses. This lock proves:
//!   A. on a 2×2 mesh the derived sender and receiver sequences are EXACTLY the transcribed vendor protocol
//!      (read verbatim from tt-metal's multicast readers) — derived, not templated;
//!   B. the compute program reloads spilled partials after the first block and packs to the output on the
//!      last; the writer waits a sub-block, writes it, barriers, pops;
//!   C. Blackhole inserts a write flush between the data multicast and the flag multicast; Wormhole does not;
//!   D. handshake kinds used ⊆ the target's; a fabric program uses only FIFO/valid-ready, a GPU program only
//!      barrier/FIFO/semaphore, and the requirement is recorded on the program;
//!   E. persist's loops expand to exactly the per-item addresses; uneven groups are refused typed;
//!   F. the schedule is per ENGINE with a per-stage view, every op tagged with its stage.
//!
//! Run: `cargo test -p logicaffeine-tests --test spatial_handshake_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::ir::{
    walk_ops, ArchCond, ChoreoOp, Engine, EnginePacking, NumericsPolicy, Operand, SemCond, SemRole,
    SemValue, Site, SpatialProgram, StageKind, TensorId,
};
use logicaffeine_compile::spatial::passes::persist::persist;
use logicaffeine_compile::spatial::passes::{
    fuse_readers::fuse_readers, place::place, size_buffers::size_buffers,
    synthesize_handshakes::synthesize_handshakes, tile::tile,
};
use logicaffeine_compile::spatial::passes::{LV_ITEM, LV_ITEM_X, LV_ITEM_Y, LV_KBLOCK};
use logicaffeine_compile::spatial::pipeline::{
    calibration_candidates_matching, compile_to_spatial, template_from_source, CandidateKind,
    PlanFilter,
};
use logicaffeine_compile::spatial::target::{HandshakeKind, SpatialTarget};
use spatial_support::*;

/// Render an op in a site-independent vocabulary: buffer by template name, semaphore by (operand, role).
fn render(p: &SpatialProgram, op: &ChoreoOp) -> String {
    let pl = p.placement.as_ref().unwrap();
    let bname = |b: logicaffeine_compile::spatial::ir::BufferId| {
        p.buffers[b.idx()]
            .name
            .split('@')
            .next()
            .unwrap()
            .to_string()
    };
    let sname = |s: logicaffeine_compile::spatial::ir::SemId| {
        let sem = &p.semaphores[s.idx()];
        let op = sem.operand.map(|o| format!("{o:?}")).unwrap_or_default();
        let role = match sem.role {
            SemRole::ReadyCounter { .. } => "ready",
            SemRole::ValidFlag => "valid",
            SemRole::Barrier { .. } => "barrier",
        };
        format!("{op}:{role}")
    };
    match op {
        ChoreoOp::Reserve { buf, pages } => format!("Reserve({},{pages})", bname(*buf)),
        ChoreoOp::Push { buf, pages } => format!("Push({},{pages})", bname(*buf)),
        ChoreoOp::Wait { buf, pages } => format!("Wait({},{pages})", bname(*buf)),
        ChoreoOp::Pop { buf, pages } => format!("Pop({},{pages})", bname(*buf)),
        ChoreoOp::ReadTiles { tensor, into, .. } => format!(
            "ReadTiles({}->{})",
            p.tensors[tensor.idx()].name,
            bname(*into)
        ),
        ChoreoOp::ReadBarrier => "ReadBarrier".into(),
        ChoreoOp::WriteTiles { from, tensor, .. } => format!(
            "WriteTiles({}->{})",
            bname(*from),
            p.tensors[tensor.idx()].name
        ),
        ChoreoOp::WriteBarrier => "WriteBarrier".into(),
        ChoreoOp::McastSend { buf, .. } => format!("McastSend({})", bname(*buf)),
        ChoreoOp::McastSetValid { sem, .. } => format!("McastSetValid({})", sname(*sem)),
        ChoreoOp::Flush { cond } => format!("Flush({cond:?})"),
        ChoreoOp::SemSet { sem, value } => format!("SemSet({},{value:?})", sname(*sem)),
        ChoreoOp::SemInc { sem, by, .. } => format!("SemInc({},{by})", sname(*sem)),
        ChoreoOp::SemWait { sem, until } => format!("SemWait({},{until:?})", sname(*sem)),
        ChoreoOp::AcquireAcc { tiles } => format!("AcquireAcc({tiles})"),
        ChoreoOp::ReleaseAcc => "ReleaseAcc".into(),
        ChoreoOp::Compute { acc_reload, .. } => format!("Compute(reload={})", acc_reload.is_some()),
        ChoreoOp::PackAcc { to, .. } => format!("PackAcc({})", bname(*to)),
        ChoreoOp::Barrier { .. } => "Barrier".into(),
        ChoreoOp::Loop { var, count, body } => format!(
            "Loop({:?},{}){{{}}}",
            var,
            count.bound(),
            body.iter()
                .map(|t| render(p, &t.op))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    }
}

fn program_text(p: &SpatialProgram, site: Site, engine: Engine) -> String {
    let prog = p
        .schedule
        .as_ref()
        .unwrap()
        .program_at(site, engine)
        .unwrap_or_else(|| panic!("no program at {site:?} {engine:?}"));
    prog.ops
        .iter()
        .map(|t| render(p, &t.op))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The best plan of an arrangement, built EXPLICITLY. A structure lock states what a given arrangement's
/// choreography is; whether the cost model prefers that arrangement is a separate question with its own lock,
/// and reading the compiler's free choice here would make this test drift every time the model learns
/// something.
fn arrangement(
    m: usize,
    n: usize,
    k: usize,
    target: &SpatialTarget,
    kind: CandidateKind,
    want: PlanFilter,
    numerics: NumericsPolicy,
) -> SpatialProgram {
    calibration_candidates_matching(&gemm_program(m, n, k), target, numerics, want)
        .unwrap_or_else(|e| panic!("{}: {e:?}", target.name))
        .into_iter()
        .find(|(k2, _)| *k2 == kind)
        .map(|(_, p)| p)
        .unwrap_or_else(|| panic!("no {kind:?} candidate for {m}x{n}x{k} on {}", target.name))
}

/// A 2×2-site fused program: 64×64×64 on the multicast-cheap Wormhole — Mt = Nt = Kt = 2, one tile per site.
/// Both operand loads share a movement engine, which is the packing whose sequences this file transcribes.
fn two_by_two(target: &SpatialTarget) -> SpatialProgram {
    let want = PlanFilter {
        engines: Some(EnginePacking::LoadsTogether),
        ..PlanFilter::default()
    };
    let p = arrangement(
        64,
        64,
        64,
        target,
        CandidateKind::Fused,
        want,
        NumericsPolicy::Exact,
    );
    let s = p.summary();
    assert_eq!(s.grid, (2, 2), "{s:?}");
    assert_eq!((s.row_groups, s.col_groups), (2, 2), "{s:?}");
    p
}

#[test]
fn the_derived_sender_sequence_is_the_vendor_protocol() {
    let p = two_by_two(&fusing_wormhole());
    let corner = program_text(
        &p,
        Site {
            x: 0,
            y: 0,
            chip: 0,
        },
        Engine::Movement(1),
    );
    let nb = p.summary().num_blocks;
    let expected = format!(
        "Loop(LoopVar(2),{nb}){{Reserve(in0,1) ReadTiles(a->in0) ReadBarrier SemWait(A:ready,Equals(1)) SemSet(A:ready,Count(0)) McastSend(in0) McastSetValid(A:valid) Push(in0,1) \
Reserve(in1,1) ReadTiles(b->in1) ReadBarrier SemWait(B:ready,Equals(1)) SemSet(B:ready,Count(0)) McastSend(in1) McastSetValid(B:valid) Push(in1,1)}}"
    );
    assert_eq!(
        corner, expected,
        "the corner site sends both operands with the transcribed sender protocol"
    );
    // A row sender that is a column receiver: send A, receive B.
    let mixed = program_text(
        &p,
        Site {
            x: 0,
            y: 1,
            chip: 0,
        },
        Engine::Movement(1),
    );
    let expected_mixed = format!(
        "Loop(LoopVar(2),{nb}){{Reserve(in0,1) ReadTiles(a->in0) ReadBarrier SemWait(A:ready,Equals(1)) SemSet(A:ready,Count(0)) McastSend(in0) McastSetValid(A:valid) Push(in0,1) \
Reserve(in1,1) SemSet(B:valid,Invalid) SemInc(B:ready,1) SemWait(B:valid,Equals(1)) Push(in1,1)}}"
    );
    assert_eq!(mixed, expected_mixed);
}

#[test]
fn the_derived_receiver_sequence_is_the_vendor_protocol() {
    let p = two_by_two(&fusing_wormhole());
    let nb = p.summary().num_blocks;
    let receiver = program_text(
        &p,
        Site {
            x: 1,
            y: 1,
            chip: 0,
        },
        Engine::Movement(1),
    );
    let expected = format!(
        "Loop(LoopVar(2),{nb}){{Reserve(in0,1) SemSet(A:valid,Invalid) SemInc(A:ready,1) SemWait(A:valid,Equals(1)) Push(in0,1) \
Reserve(in1,1) SemSet(B:valid,Invalid) SemInc(B:ready,1) SemWait(B:valid,Equals(1)) Push(in1,1)}}"
    );
    assert_eq!(
        receiver, expected,
        "invalidate, report ready to the sender, wait valid, publish the page"
    );
    // The ready increment targets the SENDER's site, never the receiver's own.
    let prog = p
        .schedule
        .as_ref()
        .unwrap()
        .program_at(
            Site {
                x: 1,
                y: 1,
                chip: 0,
            },
            Engine::Movement(1),
        )
        .unwrap();
    let pl = p.placement.as_ref().unwrap();
    let ops: Vec<ChoreoOp> = prog.ops.iter().map(|t| t.op.clone()).collect();
    let mut targets = Vec::new();
    walk_ops(&ops, 1, &mut |op, _| {
        if let ChoreoOp::SemInc { sem, at, .. } = op {
            targets.push((p.semaphores[sem.idx()].operand.unwrap(), *at));
        }
    });
    assert_eq!(
        targets,
        vec![
            (
                Operand::A,
                Site {
                    x: 0,
                    y: 1,
                    chip: 0
                }
            ),
            (
                Operand::B,
                Site {
                    x: 1,
                    y: 0,
                    chip: 0
                }
            )
        ]
    );
}

#[test]
fn the_compute_program_reloads_after_the_first_block_and_packs_out_on_the_last() {
    // Force two K blocks at a small shape with a small local memory.
    let mut t = fusing_wormhole();
    t.site.local_mem_bytes = 40 * 1024;
    // The spill-and-reload arrangement explicitly: a plan that keeps its partials in local memory across
    // blocks has a different compute program, and that one has its own lock.
    let want = PlanFilter {
        engines: Some(EnginePacking::LoadsTogether),
        pack_accumulate: Some(false),
        ..PlanFilter::default()
    };
    // Relaxed numerics: this test states the SHAPE of a spill-and-reload compute program, and on this target
    // an exact spill is refused outright because the device cannot reload a partial of the accumulator's own
    // width. That refusal has its own lock; conflating the two here would hide both.
    let p = arrangement(
        64,
        64,
        256,
        &t,
        CandidateKind::Unfused,
        want,
        NumericsPolicy::AllowSpillRounding,
    );
    let s = p.summary();
    assert!(s.num_blocks >= 2, "{s:?}");
    let site = *p.sites().iter().next().unwrap();
    let text = program_text(&p, site, Engine::Compute);
    let prog = p
        .schedule
        .as_ref()
        .unwrap()
        .program_at(site, Engine::Compute)
        .unwrap();
    let ops: Vec<ChoreoOp> = prog.ops.iter().map(|t| t.op.clone()).collect();
    let (mut reloads, mut fresh, mut pack_out, mut pack_interm) = (0u64, 0u64, 0u64, 0u64);
    walk_ops(&ops, 1, &mut |op, m| match op {
        ChoreoOp::Compute {
            acc_reload: Some(_),
            ..
        } => reloads += m,
        ChoreoOp::Compute {
            acc_reload: None, ..
        } => fresh += m,
        ChoreoOp::PackAcc { to, .. } if p.buffers[to.idx()].name.starts_with("out") => {
            pack_out += m
        }
        ChoreoOp::PackAcc { .. } => pack_interm += m,
        _ => {}
    });
    let sub_blocks =
        s.per_site_block.0 / s.block.unwrap().osh * (s.per_site_block.1 / s.block.unwrap().osw);
    let nb = s.num_blocks as u64;
    assert_eq!(
        fresh, sub_blocks as u64,
        "the first block computes from a zero accumulator: {text}"
    );
    assert_eq!(
        reloads,
        (nb - 1) * sub_blocks as u64,
        "every later block reloads the spilled partial"
    );
    assert_eq!(
        pack_out, sub_blocks as u64,
        "only the last block packs to the output"
    );
    assert_eq!(
        pack_interm,
        (nb - 1) * sub_blocks as u64,
        "every other block spills"
    );
    assert!(text.starts_with("Wait(in0,1) Wait(in1,1)"), "{text}");
    assert!(
        text.contains("AcquireAcc(") && text.contains("ReleaseAcc"),
        "{text}"
    );
    let single = two_by_two(&fusing_wormhole());
    let site = *single.sites().iter().next().unwrap();
    let text = program_text(&single, site, Engine::Compute);
    assert!(
        !text.contains("reload=true") && !text.contains("PackAcc(interm"),
        "one K block never reloads or spills: {text}"
    );
}

#[test]
fn the_writer_waits_a_sub_block_writes_barriers_and_pops() {
    let p = two_by_two(&fusing_wormhole());
    let text = program_text(
        &p,
        Site {
            x: 1,
            y: 0,
            chip: 0,
        },
        Engine::Movement(0),
    );
    assert_eq!(
        text,
        "Wait(out,1) WriteTiles(out->c) WriteBarrier Pop(out,1)"
    );
    let streamed = streamed_program(640, 640, 640, &SpatialTarget::wormhole());
    let site = *streamed.sites().iter().next().unwrap();
    let text = program_text(&streamed, site, Engine::Movement(0));
    let items = streamed.placement.as_ref().unwrap().items_at(site).len();
    assert_eq!(
        text,
        format!(
            "Loop(LoopVar(5),{items}){{Wait(out,1) WriteTiles(out->c) WriteBarrier Pop(out,1)}}"
        )
    );
}

#[test]
fn blackhole_inserts_a_flush_between_data_and_flag_multicast_wormhole_does_not() {
    let mut bh = SpatialTarget::blackhole();
    bh.cycle = fusing_wormhole().cycle.clone();
    let p = two_by_two(&bh);
    let corner = program_text(
        &p,
        Site {
            x: 0,
            y: 0,
            chip: 0,
        },
        Engine::Movement(1),
    );
    assert!(
        corner.contains("McastSend(in0) Flush(Blackhole) McastSetValid(A:valid)"),
        "{corner}"
    );
    let wh = two_by_two(&fusing_wormhole());
    let corner = program_text(
        &wh,
        Site {
            x: 0,
            y: 0,
            chip: 0,
        },
        Engine::Movement(1),
    );
    assert!(!corner.contains("Flush"), "{corner}");
    let _ = ArchCond::Always;
}

#[test]
fn handshake_kinds_used_are_a_subset_of_the_target_and_recorded() {
    for target in SpatialTarget::presets() {
        let p = compile_to_spatial(&gemm_program(256, 256, 128), &target).unwrap();
        for e in &p.edges {
            assert!(
                target.provides_handshake(e.handshake.kind()),
                "{}: edge uses {:?}",
                target.name,
                e.handshake.kind()
            );
            assert!(
                p.requires.handshakes.contains(&e.handshake.kind()),
                "{}: requirement not recorded",
                target.name
            );
        }
        for k in &p.requires.handshakes {
            assert!(target.provides_handshake(*k));
        }
        for i in &p.requires.interconnect {
            assert!(target.provides_interconnect(*i), "{}: {i:?}", target.name);
        }
    }
    let sky = compile_to_spatial(
        &gemm_program(64, 64, 64),
        &SpatialTarget::sky_fabric(64, 64),
    )
    .unwrap();
    assert!(sky.edges.iter().all(|e| matches!(
        e.handshake.kind(),
        HandshakeKind::Fifo | HandshakeKind::ValidReady
    )));
    let gpu = compile_to_spatial(&gemm_program(256, 256, 128), &SpatialTarget::rtx3060()).unwrap();
    assert!(gpu.edges.iter().all(|e| matches!(
        e.handshake.kind(),
        HandshakeKind::Fifo | HandshakeKind::Barrier | HandshakeKind::Semaphore
    )));
}

#[test]
fn persist_wraps_blocks_and_items_in_loops_with_affine_tile_addresses() {
    let wh = SpatialTarget::wormhole();
    let p = streamed_program(640, 640, 640, &wh);
    assert!(p.summary().streaming);
    let pl = p.placement.as_ref().unwrap();
    let sched = p.schedule.as_ref().unwrap();
    for prog in sched
        .programs
        .iter()
        .filter(|q| q.engine == Engine::Movement(1))
    {
        let items = pl.items_at(prog.site).len();
        assert!(items >= 6);
        assert!(
            matches!(&prog.ops[0].op, ChoreoOp::Loop { var, count, .. } if *var == LV_ITEM && count.bound() == items),
            "outer item loop at {:?}",
            prog.site
        );
        let ops: Vec<ChoreoOp> = prog.ops.iter().map(|t| t.op.clone()).collect();
        let mut reads = 0u64;
        let mut addr_vars = std::collections::BTreeSet::new();
        walk_ops(&ops, 1, &mut |op, m| {
            if let ChoreoOp::ReadTiles { at, .. } = op {
                reads += m;
                for (v, _) in &at.steps {
                    addr_vars.insert(*v);
                }
            }
        });
        assert_eq!(
            reads,
            2 * items as u64 * 20,
            "two operand tiles per K step per item, expanded through the loops"
        );
        assert!(
            addr_vars.contains(&LV_ITEM_Y) || addr_vars.contains(&LV_ITEM_X),
            "addresses are affine in the item coordinates"
        );
        assert!(addr_vars.contains(&LV_KBLOCK), "and in the K block");
    }
    // A blocked plan with one item per site has no item loop.
    let blocked = fused_2048_program();
    let _ = &wh;
    for prog in &blocked.schedule.as_ref().unwrap().programs {
        assert!(
            !matches!(&prog.ops[0].op, ChoreoOp::Loop { var, .. } if *var == LV_ITEM),
            "one item per site needs no item loop"
        );
    }
}

#[test]
fn group_item_count_mismatch_is_rejected_before_verify() {
    let t = fusing_wormhole();
    let template = template_from_source(&gemm_program(2048, 1024, 512), &t).unwrap();
    let mut p = size_buffers(
        &fuse_readers(&place(&tile(&template, &t).unwrap(), &t).unwrap(), &t).unwrap(),
        &t,
    )
    .unwrap();
    let pl = p.placement.as_mut().unwrap();
    let g0 = pl.groups[0].clone();
    let victim = g0
        .rect
        .sites()
        .into_iter()
        .find(|s| *s != g0.sender)
        .unwrap();
    let mut extra = p.work.items[0].clone();
    extra.id = logicaffeine_compile::spatial::ir::WorkItemId(p.work.items.len() as u32);
    for (s, v) in pl.items_of_site.iter_mut() {
        if *s == victim {
            v.push(extra.id);
        }
    }
    p.work.items.push(extra);
    let scheduled = synthesize_handshakes(&p, &t);
    let outcome = scheduled.and_then(|q| persist(&q, &t));
    match outcome {
        Err(logicaffeine_compile::spatial::passes::SpatialReject::GroupCountMismatch { .. }) | Err(logicaffeine_compile::spatial::passes::SpatialReject::Invalid(_)) => {}
        other => panic!("uneven item counts across a multicast group must be refused before verification: {other:?}"),
    }
}

#[test]
fn schedule_is_per_engine_with_a_per_stage_view() {
    let p = two_by_two(&fusing_wormhole());
    let sched = p.schedule.as_ref().unwrap();
    let sites = p.sites();
    assert_eq!(
        sched.programs.len(),
        sites.len() * 3,
        "one program per (site, engine): two movement engines + compute"
    );
    for prog in &sched.programs {
        assert!(!prog.ops.is_empty());
        for t in &prog.ops {
            let st = &p.stages[t.stage.idx()];
            assert_eq!(
                st.site,
                Some(prog.site),
                "an op is tagged with a stage placed on its own site"
            );
            assert!(prog.stages.contains(&t.stage));
        }
    }
    for s in &p.stages {
        let view = sched.per_stage(s.id);
        assert!(!view.is_empty(), "stage {} has no ops", s.name);
        let mut n = 0;
        for q in &sched.programs {
            logicaffeine_compile::spatial::ir::walk_tagged(&q.ops, 1, &mut |t, _| {
                if t.stage == s.id {
                    n += 1;
                }
            });
        }
        assert_eq!(
            view.len(),
            n,
            "the per-stage view lists every op of the stage, loop bodies included"
        );
    }
    let _ = (
        TensorId(0),
        SemCond::AtLeast(0),
        SemValue::Valid,
        StageKind::Recv {
            group: logicaffeine_compile::spatial::ir::GroupId(0),
        },
    );
}
