//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! FUSE-READERS LOCK — reader fusion / multicast is a GENERAL pass over the IR, decided by cost.
//!
//! Sites whose Load stages read the identical tile stream (a row of the mesh shares its A blocks, a column
//! its B blocks) are grouped: ONE site loads and multicasts, the others become receivers with a semaphore
//! pair. This lock proves:
//!   A. on the 2048×1024×512 plan the pass forms exactly 8 row groups (senders in column 0) and 8 column
//!      groups (senders in row 0) with 7 receivers each; receivers never read the fused tensor from DRAM;
//!   B. fusion is a cost decision (the calibrated preset leaves 640³ unfused; a multicast-cheap model fuses
//!      it; a multicast-expensive model unfuses the 2048 class), a 1-D grid fuses only its shared operand;
//!   C. roles carry the target's NOC rule (senders NOC 0, receivers NOC 1, first-column writers NOC 1),
//!      A and B share one movement engine in A,B order, the store owns the other, one engine → typed refusal;
//!   D. a target without semaphores refuses fusion typed; group members share buffer kind, dtype and offset;
//!      the three DRAM-read formulas (streamed, blocked-unfused, fused) EMERGE from structure;
//!   E. engine-program classes: 6 kernel sources for a fused 2-D plan, 3 for a streamed plan.
//!
//! Run: `cargo test -p logicaffeine-tests --test spatial_fuse_readers_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::ir::{
    walk_ops, ChoreoOp, Direction, Engine, Handshake, NocId, Operand, Site, StageKind, TensorId,
};
use logicaffeine_compile::spatial::passes::fuse_readers::fuse_readers;
use logicaffeine_compile::spatial::passes::place::place;
use logicaffeine_compile::spatial::passes::size_buffers::size_buffers;
use logicaffeine_compile::spatial::passes::tile::{best_candidate_for, tile};
use logicaffeine_compile::spatial::passes::SpatialReject;
use logicaffeine_compile::spatial::pipeline::{
    compile_to_spatial, run_passes, template_from_source,
};
use logicaffeine_compile::spatial::target::{HandshakeKind, SpatialTarget};
use logicaffeine_compile::spatial::work::Objective;
use spatial_support::*;

fn fused_2048() -> logicaffeine_compile::spatial::ir::SpatialProgram {
    fused_2048_program()
}

#[test]
fn readers_fuse_along_rows_for_a_and_columns_for_b_on_a_2d_grid() {
    let p = fused_2048();
    let pl = p.placement.as_ref().unwrap();
    let rows: Vec<_> = pl
        .groups
        .iter()
        .filter(|g| g.direction == Direction::Row)
        .collect();
    let cols: Vec<_> = pl
        .groups
        .iter()
        .filter(|g| g.direction == Direction::Col)
        .collect();
    assert_eq!(
        (rows.len(), cols.len()),
        (8, 8),
        "8 row groups + 8 column groups"
    );
    for g in &rows {
        assert_eq!(g.operand, Operand::A);
        assert_eq!(g.sender.x, 0, "row senders sit in the left column");
        assert_eq!(g.rect.len(), 8);
    }
    for g in &cols {
        assert_eq!(g.operand, Operand::B);
        assert_eq!(g.sender.y, 0, "column senders sit in the top row");
        assert_eq!(g.rect.len(), 8);
    }
    let recv_a = p.stages.iter().filter(|s| matches!(&s.kind, StageKind::Recv { group } if pl.groups[group.idx()].operand == Operand::A)).count();
    let load_a = p
        .stages
        .iter()
        .filter(|s| {
            matches!(
                &s.kind,
                StageKind::Load {
                    tensor: TensorId(0),
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        (load_a, recv_a),
        (8, 56),
        "A is loaded from DRAM by 8 senders and received by 56 sites"
    );
    for e in p
        .edges
        .iter()
        .filter(|e| matches!(e.handshake, Handshake::Semaphore { .. }))
    {
        let Handshake::Semaphore { ready_count, .. } = e.handshake else {
            unreachable!()
        };
        assert_eq!(
            ready_count, 7,
            "seven receivers must report ready before a multicast"
        );
        assert_eq!(e.to.len(), 7);
    }
    // Every Recv stage records the Load it replaced.
    for s in p
        .stages
        .iter()
        .filter(|s| matches!(s.kind, StageKind::Recv { .. }))
    {
        let o = s.origin.expect("a receiver remembers its template load");
        assert!(matches!(p.stages[o.idx()].kind, StageKind::Load { .. }));
    }
    // Receivers never touch DRAM for the fused operand.
    let sched = p.schedule.as_ref().unwrap();
    for prog in &sched.programs {
        let site_is_a_sender = pl
            .groups
            .iter()
            .any(|g| g.sender == prog.site && g.operand == Operand::A);
        let ops: Vec<ChoreoOp> = prog.ops.iter().map(|t| t.op.clone()).collect();
        let mut reads_a = 0u64;
        walk_ops(&ops, 1, &mut |op, m| {
            if let ChoreoOp::ReadTiles {
                tensor: TensorId(0),
                ..
            } = op
            {
                reads_a += m;
            }
        });
        if !site_is_a_sender {
            assert_eq!(
                reads_a, 0,
                "site {:?} receives A by multicast and must never read it from DRAM",
                prog.site
            );
        }
    }
}

#[test]
fn fusion_is_a_cost_decision_not_a_flag() {
    let cheap = fusing_wormhole();
    let fused = compile_to_spatial(&gemm_program(640, 640, 640), &cheap).unwrap();
    assert!(
        fused.summary().multicast_groups > 0,
        "a multicast-cheap model fuses the same shape"
    );
    let dear = non_fusing_wormhole();
    let unfused = compile_to_spatial(&gemm_program(2048, 1024, 512), &dear).unwrap();
    assert_eq!(
        unfused.summary().multicast_groups,
        0,
        "a target without a multicast interconnect unfuses the 2048 class"
    );
    assert_eq!(fused_2048().summary().multicast_groups, 16);
    // ★ AND IT IS A DECISION, not a capability check: on the SAME capable target, asking for a different
    // thing gets a different arrangement. `Transactions` counts NOC requests, and fusion buys its byte
    // saving with a handshake per receiver per block — so where that handshake dominates, the planner
    // declines to fuse. Same chip, same shape, same lattice; different question, different plan.
    let wh = SpatialTarget::wormhole();
    let t = template_from_source(&gemm_program(512, 512, 512), &wh).unwrap();
    let balanced = best_candidate_for(&t, &wh, Objective::Balanced).expect("a balanced candidate");
    let by_txns =
        best_candidate_for(&t, &wh, Objective::Transactions).expect("a transaction candidate");
    assert!(
        balanced.fused,
        "the default objective fuses 512x512x512 on a capable target"
    );
    assert!(
        by_txns.work.noc_txns < balanced.work.noc_txns,
        "the transaction objective must actually issue fewer than the balanced one: {} vs {}",
        by_txns.work.noc_txns,
        balanced.work.noc_txns
    );
    assert_ne!(
        (
            by_txns.plan.per_site_m,
            by_txns.plan.per_site_n,
            by_txns.fused
        ),
        (
            balanced.plan.per_site_m,
            balanced.plan.per_site_n,
            balanced.fused
        ),
        "asking a different question must reach a different plan, or fusion is a flag after all"
    );
}

#[test]
fn a_1d_grid_fuses_only_the_shared_operand() {
    // A one-column mesh: every site shares the same B column blocks and owns a distinct A row block.
    let mut cheap = fusing_wormhole();
    cheap.sites = logicaffeine_compile::spatial::target::MeshShape { x: 1, y: 8 };
    let p = compile_to_spatial(&gemm_program(2048, 256, 512), &cheap).unwrap();
    let s = p.summary();
    assert_eq!(s.grid.1, 1, "one column of sites: {s:?}");
    assert_eq!(
        s.row_groups, 0,
        "rows have a single member: nothing to fuse"
    );
    assert_eq!(s.col_groups, 1, "the one column shares B");
    let pl = p.placement.as_ref().unwrap();
    assert_eq!(pl.groups[0].operand, Operand::B);
    assert_eq!(pl.groups[0].rect.len(), s.grid.0);
}

#[test]
fn senders_and_receivers_take_the_target_noc_rule() {
    let p = fused_2048();
    let pl = p.placement.as_ref().unwrap();
    let rule = SpatialTarget::wormhole().noc_rule.unwrap();
    for s in &p.stages {
        let slot = &pl.engine_of[s.id.idx()];
        let site = s.site.unwrap();
        match &s.kind {
            StageKind::Load {
                multicast: Some(g), ..
            } if pl.groups[g.idx()].direction == Direction::Row => {
                assert_eq!(
                    slot.noc,
                    Some(NocId(rule.sender_noc)),
                    "row senders multicast on NOC {}",
                    rule.sender_noc
                );
            }
            StageKind::Load { .. } | StageKind::Recv { .. } => {
                let a_sender = pl
                    .groups
                    .iter()
                    .any(|g| g.sender == site && g.operand == Operand::A);
                let expected = if a_sender {
                    rule.sender_noc
                } else {
                    rule.receiver_noc
                };
                assert_eq!(
                    slot.noc,
                    Some(NocId(expected)),
                    "reader engine NOC at {site:?}"
                );
            }
            StageKind::Store { .. } => {
                let expected = if site.x == 0 {
                    rule.writer_noc_first_column
                } else {
                    rule.writer_noc_default
                };
                assert_eq!(slot.noc, Some(NocId(expected)), "writer NOC at {site:?}");
            }
            _ => {}
        }
    }
    for g in &pl.groups {
        assert_eq!(g.noc, NocId(rule.sender_noc));
    }
}

#[test]
fn a_and_b_movement_share_one_engine_in_ab_order_and_store_takes_the_other() {
    let p = fused_2048();
    let pl = p.placement.as_ref().unwrap();
    for site in p.sites() {
        let a = p
            .stages
            .iter()
            .find(|s| {
                s.site == Some(site)
                    && matches!(
                        &s.kind,
                        StageKind::Load {
                            tensor: TensorId(0),
                            ..
                        } | StageKind::Recv { .. }
                    )
                    && p.stage_group(s)
                        .map(|g| pl.groups[g.idx()].operand == Operand::A)
                        .unwrap_or(true)
            })
            .unwrap();
        let b = p
            .stages
            .iter()
            .find(|s| {
                s.site == Some(site)
                    && s.id != a.id
                    && matches!(&s.kind, StageKind::Load { .. } | StageKind::Recv { .. })
            })
            .unwrap();
        let st = p
            .stages
            .iter()
            .find(|s| s.site == Some(site) && matches!(s.kind, StageKind::Store { .. }))
            .unwrap();
        let (ea, eb, es) = (
            pl.engine_of[a.id.idx()].engine,
            pl.engine_of[b.id.idx()].engine,
            pl.engine_of[st.id.idx()].engine,
        );
        assert_eq!(ea, eb, "A and B movement share one engine at {site:?}");
        assert_ne!(ea, es, "the store owns the other engine at {site:?}");
        assert!(matches!(ea, Engine::Movement(_)) && matches!(es, Engine::Movement(_)));
        let prog = p.schedule.as_ref().unwrap().program_at(site, ea).unwrap();
        // Pre-order over the tagged ops, loop bodies included: the first op of each stage.
        let mut order = Vec::new();
        logicaffeine_compile::spatial::ir::walk_tagged(&prog.ops, 1, &mut |t, _| {
            order.push(t.stage)
        });
        let first_a = order.iter().position(|s| *s == a.id).unwrap();
        let first_b = order.iter().position(|s| *s == b.id).unwrap();
        assert!(
            first_a < first_b,
            "A precedes B in the engine program at {site:?}"
        );
    }
    // One movement engine: loads and the store share it (sequentially, per item); none: typed refusal.
    let mut one_engine = SpatialTarget::wormhole();
    one_engine.site.movement_engines = 1;
    let t = template_from_source(&gemm_program(2048, 1024, 512), &one_engine).unwrap();
    let placed = place(&tile(&t, &one_engine).unwrap(), &one_engine).unwrap();
    let pl = placed.placement.as_ref().unwrap();
    assert!(
        pl.engine_of.iter().all(|e| e.engine != Engine::Movement(1)),
        "a single movement engine hosts every movement stage"
    );
    let mut none = SpatialTarget::wormhole();
    none.site.movement_engines = 0;
    match place(&tile(&t, &none).unwrap(), &none) {
        Err(SpatialReject::TooManyEngines {
            needed: 1,
            available: 0,
            ..
        }) => {}
        other => panic!("no movement engine at all must refuse typed: {other:?}"),
    }
}

#[test]
fn fusion_requires_multicast_and_semaphores_typed() {
    let gpu = SpatialTarget::rtx3060();
    let p = compile_to_spatial(&gemm_program(2048, 1024, 512), &gpu).unwrap();
    assert_eq!(
        p.summary().multicast_groups,
        0,
        "no rectangle multicast on sm_86: no groups, ever"
    );
    let mut no_sems = fusing_wormhole();
    no_sems.handshakes = [HandshakeKind::Fifo].into_iter().collect();
    let t = template_from_source(&gemm_program(2048, 1024, 512), &no_sems).unwrap();
    match fuse_readers(
        &place(&tile(&t, &no_sems).unwrap(), &no_sems).unwrap(),
        &no_sems,
    ) {
        Err(SpatialReject::MissingHandshake {
            needed: HandshakeKind::Semaphore,
            ..
        }) => {}
        other => {
            panic!("fusion that wins on cost but lacks semaphores must refuse typed: {other:?}")
        }
    }
}

#[test]
fn group_members_share_buffer_kind_dtype_and_offset() {
    let p = fused_2048();
    let pl = p.placement.as_ref().unwrap();
    let mm = p.memory.as_ref().unwrap();
    for g in &pl.groups {
        let mut kinds = std::collections::BTreeSet::new();
        let mut offsets = std::collections::BTreeSet::new();
        for site in g.rect.sites() {
            let member = p
                .stages
                .iter()
                .find(|s| s.site == Some(site) && p.stage_group(s) == Some(g.id))
                .unwrap();
            let b = &p.buffers[member.outs[0].idx()];
            kinds.insert((b.kind, b.dtype));
            offsets.insert(mm.buffer_at[b.id.idx()].offset_bytes);
        }
        assert_eq!(
            kinds.len(),
            1,
            "group {:?} members differ in buffer shape",
            g.id
        );
        assert_eq!(
            offsets.len(),
            1,
            "group {:?}: a multicast lands at ONE local address on every receiver",
            g.id
        );
    }
    let again = fused_2048_program();
    assert_eq!(
        again.memory, p.memory,
        "the memory map is a deterministic function of the placed program"
    );
    let wh = SpatialTarget::wormhole();
    let pipeline = compile_to_spatial(&gemm_program(2048, 1024, 512), &wh).unwrap();
    let by_pass = size_buffers(
        &fuse_readers(
            &place(
                &tile(
                    &template_from_source(&gemm_program(2048, 1024, 512), &wh).unwrap(),
                    &wh,
                )
                .unwrap(),
                &wh,
            )
            .unwrap(),
            &wh,
        )
        .unwrap(),
        &wh,
    )
    .unwrap();
    assert_eq!(
        by_pass.memory, pipeline.memory,
        "the pipeline and the pass-by-pass path agree"
    );
}

#[test]
fn engine_program_classes_equal_the_kernel_count_a_lowering_needs() {
    assert_eq!(
        fused_2048().summary().engine_program_classes,
        6,
        "4 reader roles + 1 writer + 1 compute"
    );
    let streamed = streamed_program(640, 640, 640, &SpatialTarget::wormhole());
    assert_eq!(
        streamed.summary().engine_program_classes,
        3,
        "reader + writer + compute"
    );
}

#[test]
fn the_dram_read_tile_formulas_emerge_from_structure() {
    let (mt, nt, kt) = (64u64, 32u64, 16u64);
    let fused = fused_2048().summary();
    assert_eq!(
        fused.dram_tiles_read,
        (mt + nt) * kt,
        "fused: every operand tile read once"
    );
    let streamed =
        compile_to_spatial(&gemm_program(2048, 1024, 512), &non_fusing_wormhole()).unwrap();
    let s = streamed.summary();
    if s.streaming {
        assert_eq!(
            s.dram_tiles_read,
            2 * mt * nt * kt,
            "streamed: both operands re-read per output tile"
        );
    } else {
        let (gy, gx) = (s.grid.0 as u64, s.grid.1 as u64);
        assert_eq!(
            s.dram_tiles_read,
            gx * mt * kt + gy * nt * kt,
            "blocked-unfused: each operand re-read per site row/column"
        );
    }
    // A crossbar target has no rectangle multicast: its blocked plan is the unfused formula by construction.
    let rdu =
        compile_to_spatial(&gemm_program(2048, 1024, 512), &SpatialTarget::sn40l_rdu()).unwrap();
    let r = rdu.summary();
    assert_eq!(r.multicast_groups, 0);
    let (gy, gx) = (r.grid.0 as u64, r.grid.1 as u64);
    let expected = if r.streaming {
        2 * mt * nt * kt
    } else {
        gx * mt * kt + gy * nt * kt
    };
    assert_eq!(r.dram_tiles_read, expected);
}

#[test]
fn uneven_item_counts_never_form_a_group() {
    // A 6×6 mesh cannot hold the 8×8 block grid: sites wrap with 1, 2 or 4 items. Row groups over sites with
    // different item counts would pair multicast blocks with nothing on the far side — the pass must not form
    // them, and the invariant is enforced structurally.
    let mut small = fusing_wormhole();
    small.sites = logicaffeine_compile::spatial::target::MeshShape { x: 6, y: 6 };
    let p = run_passes(
        template_from_source(&gemm_program(2048, 2048, 512), &small).unwrap(),
        &small,
    )
    .unwrap();
    p.validate().unwrap();
    let pl = p.placement.as_ref().unwrap();
    for g in &pl.groups {
        let counts: std::collections::BTreeSet<usize> = g
            .rect
            .sites()
            .iter()
            .map(|s| pl.items_at(*s).len())
            .collect();
        assert_eq!(
            counts.len(),
            1,
            "group {:?} spans sites with different item counts: {counts:?}",
            g.id
        );
    }
    // Forced mismatch → typed at validate and at persist.
    let mut broken = fused_2048();
    broken.certificate = None;
    let pl = broken.placement.as_mut().unwrap();
    let g0 = pl.groups[0].clone();
    let victim = g0
        .rect
        .sites()
        .into_iter()
        .find(|s| *s != g0.sender)
        .unwrap();
    let extra = broken.work.items[0].clone();
    let mut extra = extra;
    extra.id = logicaffeine_compile::spatial::ir::WorkItemId(broken.work.items.len() as u32);
    broken.work.items.push(extra.clone());
    for (s, v) in pl.items_of_site.iter_mut() {
        if *s == victim {
            v.push(extra.id);
        }
    }
    assert!(
        matches!(
            broken.validate(),
            Err(logicaffeine_compile::spatial::ir::IrViolation::ItemCountMismatch { .. })
                | Err(logicaffeine_compile::spatial::ir::IrViolation::OutputOverlap { .. })
        ),
        "{:?}",
        broken.validate()
    );
    let _ = Site {
        x: 0,
        y: 0,
        chip: 0,
    };
}
