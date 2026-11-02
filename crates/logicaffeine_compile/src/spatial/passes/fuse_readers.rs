//! `fuse_readers` — reader fusion / multicast as a GENERAL pass, decided by the cost model.
//!
//! Sites whose Load stages read the identical tile stream (a mesh row shares its A blocks, a column its B
//! blocks) form a GROUP: one site keeps its Load and becomes the sender (it multicasts each block after
//! landing it), the others become receivers whose Load is replaced by a `Recv`. Every group gets a semaphore
//! pair (ready counter at the sender, valid flag at the receivers) and its members' NOCs follow the target's
//! direction rule. The decision is the cost model's comparison of the fused and unfused arrangements of the
//! SAME plan — the comparison `tile` already made — never a flag.

use super::SpatialReject;
use crate::spatial::cost::is_streaming;
use crate::spatial::ir::*;
use crate::spatial::target::{HandshakeKind, Interconnect, SpatialTarget};

/// Whether the cost model prefers the fused (multicast) arrangement for this placed plan.
pub fn fusion_wins(p: &SpatialProgram, target: &SpatialTarget) -> bool {
    let Some(plan) = p.work.plan else {
        return false;
    };
    if is_streaming(&plan) {
        return false;
    }
    // Fusion cannot win on a target with no multicast at all. Costing it first and discovering the missing
    // interconnect afterwards turns an option the target simply lacks into a hard refusal of a program it
    // could run perfectly well unfused.
    //
    // The SEMAPHORE capability is deliberately NOT checked here. A target that can multicast but cannot
    // synchronise is an inconsistent description rather than a target without a feature, and `fuse_readers`
    // refuses it typed so the contradiction surfaces instead of silently degrading to unfused.
    if !target.provides_interconnect(Interconnect::MulticastRect) {
        return false;
    }
    let (ny, nx) = plan.grid;
    if nx <= 1 && ny <= 1 {
        return false;
    }
    // A multicast group shares ONE operand panel along a mesh row or column. Once the block grid outgrows the
    // mesh, `place` wraps items onto sites, so a mesh row holds blocks from several block-rows that share no
    // panel and there is nothing left to broadcast. Past that point fusion is not a losing option, it is not
    // an option — which is why the tile pass offers no fused candidate there either.
    if ny > target.sites.y || nx > target.sites.x {
        return false;
    }
    // ★ THE SAME QUESTION THE PLAN SEARCH ASKED, ASKED THE SAME WAY. Fusing shares an operand across a
    // rectangle: each shared tile is read from DRAM once and multicast once, instead of being fetched by
    // every site that needs it. Whether that wins is a movement question, and it is settled by the work
    // the two choreographies will actually issue — not by a counter that does not track work.
    let objective = crate::spatial::work::Objective::default();
    let (Some(fused), Some(unfused)) = (
        crate::spatial::work::work_of_plan(p, &plan, true, target),
        crate::spatial::work::work_of_plan(p, &plan, false, target),
    ) else {
        return false;
    };
    objective.key(&fused) < objective.key(&unfused)
}

/// The pass: fuse iff the cost model says so.
pub fn fuse_readers(
    p: &SpatialProgram,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let has_matmul = p
        .stages
        .iter()
        .any(|s| matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. })));
    fuse_readers_forced(p, target, has_matmul && fusion_wins(p, target))
}

/// Fuse (or not) regardless of the cost model — a calibration facility, never a pass.
pub fn fuse_readers_forced(
    p: &SpatialProgram,
    target: &SpatialTarget,
    fused: bool,
) -> Result<SpatialProgram, SpatialReject> {
    let Some(pl) = &p.placement else {
        return Err(SpatialReject::WrongPhase {
            pass: "fuse_readers",
            expected: "Placed",
            found: p.phase(),
        });
    };
    if !pl.groups.is_empty() || !fused {
        return Ok(p.clone());
    }
    if !target.provides_interconnect(Interconnect::MulticastRect) {
        return Err(SpatialReject::MissingInterconnect {
            needed: Interconnect::MulticastRect,
            target: target.name,
        });
    }
    if !target.provides_handshake(HandshakeKind::Semaphore) {
        return Err(SpatialReject::MissingHandshake {
            needed: HandshakeKind::Semaphore,
            target: target.name,
        });
    }
    let plan = p.work.plan.expect("placed programs are tiled");
    let (ny, nx) = plan.grid;
    let rule = target.noc_rule;
    let mut q = p.clone();
    let pl = q.placement.as_mut().unwrap();
    let sites: Vec<Site> = pl.items_of_site.iter().map(|(s, _)| *s).collect();
    let mut groups = Vec::new();
    // Row groups share A (operand 0); column groups share B (operand 1).
    let mut specs: Vec<(Direction, Operand, Rect, Site)> = Vec::new();
    if nx > 1 {
        for y in 0..ny {
            let rect = Rect {
                x0: 0,
                y0: y as u16,
                x1: (nx - 1) as u16,
                y1: y as u16,
            };
            specs.push((Direction::Row, Operand::A, rect, Site::new(0, y)));
        }
    }
    if ny > 1 {
        for x in 0..nx {
            let rect = Rect {
                x0: x as u16,
                y0: 0,
                x1: x as u16,
                y1: (ny - 1) as u16,
            };
            specs.push((Direction::Col, Operand::B, rect, Site::new(x, 0)));
        }
    }
    let mut sems_of: std::collections::BTreeMap<Operand, (SemId, SemId)> = Default::default();
    for (direction, operand, rect, sender) in specs {
        let members = rect.sites();
        if !members.iter().all(|m| sites.contains(m)) {
            continue;
        }
        let counts: std::collections::BTreeSet<usize> =
            members.iter().map(|m| pl.items_at(*m).len()).collect();
        if counts.len() != 1 {
            return Err(SpatialReject::GroupCountMismatch {
                group: GroupId::new(groups.len()),
                counts: members
                    .iter()
                    .map(|m| (*m, pl.items_at(*m).len()))
                    .collect(),
            });
        }
        let gid = GroupId::new(groups.len());
        let noc = NocId(rule.map(|r| r.sender_noc).unwrap_or(0));
        groups.push(Group {
            id: gid,
            rect,
            sender,
            direction,
            operand,
            noc,
        });
        let tensor = match operand {
            Operand::A => TensorId(0),
            Operand::B => TensorId(1),
        };
        let (ready, valid) = match sems_of.get(&operand) {
            Some(pair) => *pair,
            None => {
                let ready = SemId::new(q.semaphores.len());
                q.semaphores.push(Semaphore {
                    id: ready,
                    init: 0,
                    role: SemRole::ReadyCounter {
                        expected: members.len() - 1,
                    },
                    operand: Some(operand),
                    sites: Vec::new(),
                });
                let valid = SemId::new(q.semaphores.len());
                q.semaphores.push(Semaphore {
                    id: valid,
                    init: 0,
                    role: SemRole::ValidFlag,
                    operand: Some(operand),
                    sites: Vec::new(),
                });
                sems_of.insert(operand, (ready, valid));
                (ready, valid)
            }
        };
        for sem in [ready, valid] {
            for m in &members {
                if !q.semaphores[sem.idx()].sites.contains(m) {
                    q.semaphores[sem.idx()].sites.push(*m);
                }
            }
        }
        let mut sender_stage = None;
        let mut sender_buf = None;
        let mut receivers = Vec::new();
        for s in q.stages.iter_mut() {
            let Some(site) = s.site else { continue };
            if !rect.contains(site) {
                continue;
            }
            let is_load = matches!(&s.kind, StageKind::Load { tensor: t, .. } if *t == tensor);
            if !is_load {
                continue;
            }
            if site == sender {
                if let StageKind::Load { multicast, .. } = &mut s.kind {
                    *multicast = Some(gid);
                }
                sender_stage = Some(s.id);
                sender_buf = s.outs.first().copied();
            } else {
                s.kind = StageKind::Recv { group: gid };
                s.name = s.name.replace("load_", "recv_");
                s.ins.clear();
                receivers.push(s.id);
            }
        }
        let (Some(from), Some(buffer)) = (sender_stage, sender_buf) else {
            return Err(SpatialReject::Invalid(IrViolation::GroupMalformed {
                group: gid,
                reason: "no sender load stage",
            }));
        };
        q.edges.push(Edge {
            id: EdgeId::new(q.edges.len()),
            from,
            buffer,
            to: receivers,
            handshake: Handshake::Semaphore {
                ready,
                valid,
                ready_count: members.len() - 1,
            },
        });
    }
    let pl = q.placement.as_mut().unwrap();
    pl.groups = groups;
    // NOC assignment is keyed off the SITE's role in its multicast groups, which only exist once fusion has
    // run — `place` cannot know it. A row sender drives the sender interface; a site that senders operand A
    // keeps it for its own reads; everything else reads on the receiver interface; and the writer takes the
    // complement, with the first column swapped because its multicasts travel the other way.
    //
    // This is the vendor's documented convention. We briefly keyed it off the ENGINE instead, on the strength
    // of a simulator measurement that looked like a 30% win — and that counter was later proved not to track
    // work at all, so the evidence for the change evaporated. Reverted rather than kept on a number we can no
    // longer stand behind.
    if let Some(r) = rule {
        let groups = pl.groups.clone();
        let a_senders: std::collections::BTreeSet<Site> = groups
            .iter()
            .filter(|g| g.operand == Operand::A)
            .map(|g| g.sender)
            .collect();
        for (i, s) in q.stages.iter().enumerate() {
            let Some(site) = s.site else { continue };
            let noc = match &s.kind {
                StageKind::Load {
                    multicast: Some(g), ..
                } if groups[g.idx()].direction == Direction::Row => r.sender_noc,
                StageKind::Load { .. } | StageKind::Recv { .. } => {
                    if a_senders.contains(&site) {
                        r.sender_noc
                    } else {
                        r.receiver_noc
                    }
                }
                StageKind::Store { .. } => {
                    if site.x == 0 {
                        r.writer_noc_first_column
                    } else {
                        r.writer_noc_default
                    }
                }
                _ => continue,
            };
            pl.engine_of[i].noc = Some(NocId(noc));
        }
        // ★ TWO MOVEMENT ENGINES ON ONE SITE MAY NOT SHARE A NOC.
        //
        // The role rule above assigns by ROLE, and a site can hold two roles: an A-multicast sender is
        // also its own B reader, so both of its engines asked for `sender_noc`; a receiver site asked for
        // `receiver_noc` twice. Each RISC-V keeps its OWN software count of issued NOC transactions but
        // they then share one hardware counter, so `noc_async_read_barrier()` compares its count against a
        // counter carrying the other engine's traffic and NEVER CLEARS.
        //
        // Measured: at 128³ the multicast program wedged with `dm1_a_send` parked in
        // `noc_async_read_barrier()` (tt-metal's watcher: core (0,0) `NRBW,NRBW`), while the blocked
        // program at the same shape — whose engines land on NOC 0 and NOC 1 — ran in seconds at PCC 1.0.
        //
        // The role preference is kept where it is free, and only the COLLIDING engine is moved, so the
        // vendor's convention still holds wherever it can.
        let mut taken: std::collections::BTreeMap<(Site, NocId), Engine> = Default::default();
        for slot in pl.engine_of.iter_mut() {
            let Some(noc) = slot.noc else { continue };
            match taken.get(&(slot.site, noc)) {
                Some(e) if *e != slot.engine => {
                    let other = NocId(1 - noc.0);
                    slot.noc = Some(other);
                    taken.insert((slot.site, other), slot.engine);
                }
                _ => {
                    taken.insert((slot.site, noc), slot.engine);
                }
            }
        }
    }
    q.requires.handshakes.insert(HandshakeKind::Semaphore);
    q.requires.interconnect.insert(Interconnect::MulticastRect);
    q.validate()?;
    Ok(q)
}
