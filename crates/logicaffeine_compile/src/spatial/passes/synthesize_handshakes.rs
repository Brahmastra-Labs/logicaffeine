//! `synthesize_handshakes` — semaphore choreography DERIVED from the buffer/edge graph.
//!
//! For every site and engine the pass emits the exact op sequence the edges' handshake kinds demand:
//! local FIFO / valid-ready hand-offs (`Reserve → fill → Push`, `Wait → use → Pop`), the multicast sender's
//! ready-wait / reset / data multicast / \[flush\] / valid-flag multicast, the receiver's invalidate /
//! ready-increment / valid-wait, the compute engine's acquire / \[reload\] / compute / pack / release with the
//! first and last K block peeled (no runtime `if`), and the writer's wait / write / barrier / pop. Each
//! program is one ENGINE's sequential code; loops over K blocks and sub-blocks are explicit; the item loop is
//! `persist`'s.

use super::{SpatialReject, LV_KBLOCK, LV_SUB_H, LV_SUB_W};

/// Pages one work item pushes through `buf`: the producer's granularity, followed through a chain.
///
/// Bounded by the STAGE COUNT, not a literal: each step walks to the producer of the previous stage's input,
/// so exceeding the number of stages means a cycle rather than a deep chain. A constant sized to today's
/// longest chain silently returns the wrong page count the day someone fuses a longer one.
fn pages_per_item(p: &SpatialProgram, buf: BufferId, depth: usize) -> usize {
    if depth > p.stages.len() {
        return 1;
    }
    let Some(e) = p.edges.iter().find(|e| e.buffer == buf) else {
        return 1;
    };
    let producer = p.stage(e.from);
    match &producer.kind {
        StageKind::Compute(ComputeOp::MatmulTiles { sub_blocks, .. }) => {
            sub_blocks.0 * sub_blocks.1
        }
        StageKind::Compute(_) | StageKind::Scalar(_) | StageKind::AllReduce { .. } => producer
            .ins
            .first()
            .map(|b| pages_per_item(p, *b, depth + 1))
            .unwrap_or(1),
        StageKind::Load {
            pattern,
            num_blocks,
            ..
        } => {
            if pattern
                .origin
                .steps
                .iter()
                .any(|(v, _)| *v == LV_SUB_H || *v == LV_SUB_W)
            {
                p.work
                    .plan
                    .map(|pl| {
                        (pl.per_site_m / pl.block.osh.max(1))
                            * (pl.per_site_n / pl.block.osw.max(1))
                    })
                    .unwrap_or(1)
            } else {
                *num_blocks
            }
        }
        StageKind::Recv { .. } => p.work.plan.map(|pl| pl.inner.num_blocks).unwrap_or(1),
        StageKind::Store { .. } => 1,
    }
}
use crate::spatial::cost::is_streaming;
use crate::spatial::ir::*;
use crate::spatial::target::{SpatialTarget, WriteOrdering};
use std::collections::BTreeMap;

fn stage_group_semaphores(
    p: &SpatialProgram,
    stage: StageId,
) -> Option<(GroupId, SemId, SemId, usize)> {
    let s = p.stage(stage);
    let g = p.stage_group(s)?;
    for e in &p.edges {
        if let Handshake::Semaphore {
            ready,
            valid,
            ready_count,
        } = &e.handshake
        {
            let sender_is_me = e.from == stage;
            let receiver_is_me = e.to.contains(&stage);
            if (sender_is_me || receiver_is_me) && p.stage_group(p.stage(e.from)) == Some(g) {
                return Some((g, *ready, *valid, *ready_count));
            }
        }
    }
    None
}

/// The movement ops that land one block of a Load / Recv stage into its buffer.
fn reader_block(p: &SpatialProgram, target: &SpatialTarget, s: &Stage, out: &mut Vec<TaggedOp>) {
    let buf = s.outs[0];
    let tag = |op: ChoreoOp| TaggedOp { stage: s.id, op };
    match &s.kind {
        StageKind::Load {
            tensor,
            pattern,
            multicast,
            ..
        } => {
            // ★ A GATHER WAITS FOR ITS INDEX. The tile address of an `IndirectTile` load is a value another
            // stage produces; the read may not issue before that page is visible, and the page is retired
            // once the read has resolved it.
            let index = pattern.origin.indirect.map(|i| i.index);
            if let Some(ix) = index {
                out.push(tag(ChoreoOp::Wait { buf: ix, pages: 1 }));
            }
            out.push(tag(ChoreoOp::Reserve { buf, pages: 1 }));
            out.push(tag(ChoreoOp::ReadTiles {
                tensor: *tensor,
                at: pattern.origin.clone(),
                block_h: pattern.block_h,
                block_w: pattern.block_w,
                stride_w: pattern.stride_w,
                stride_h: pattern.stride_h,
                into: buf,
            }));
            out.push(tag(ChoreoOp::ReadBarrier));
            if let Some(ix) = index {
                out.push(tag(ChoreoOp::Pop { buf: ix, pages: 1 }));
            }
            if let Some(g) = multicast {
                let (_, ready, valid, n) = stage_group_semaphores(p, s.id)
                    .expect("a multicasting load has a semaphore edge");
                out.push(tag(ChoreoOp::SemWait {
                    sem: ready,
                    until: SemCond::Equals(n),
                }));
                out.push(tag(ChoreoOp::SemSet {
                    sem: ready,
                    value: SemValue::Count(0),
                }));
                // The valid flag goes to the SAME rectangle immediately after the data, so the data
                // transfer keeps the broadcast path it reserved and the flag rides it. A target whose writes
                // need an explicit flush between the two cannot hold the path across that flush.
                let hold_path = target.write_ordering != WriteOrdering::NeedsFlush;
                out.push(tag(ChoreoOp::McastSend {
                    buf,
                    pages: 1,
                    group: *g,
                    hold_path,
                }));
                if target.write_ordering == WriteOrdering::NeedsFlush {
                    out.push(tag(ChoreoOp::Flush {
                        cond: ArchCond::Blackhole,
                    }));
                }
                out.push(tag(ChoreoOp::McastSetValid {
                    sem: valid,
                    group: *g,
                }));
            }
            out.push(tag(ChoreoOp::Push { buf, pages: 1 }));
        }
        StageKind::Recv { group } => {
            let (_, ready, valid, _) =
                stage_group_semaphores(p, s.id).expect("a receiver has a semaphore edge");
            let sender = p.placement.as_ref().unwrap().groups[group.idx()].sender;
            out.push(tag(ChoreoOp::Reserve { buf, pages: 1 }));
            out.push(tag(ChoreoOp::SemSet {
                sem: valid,
                value: SemValue::Invalid,
            }));
            out.push(tag(ChoreoOp::SemInc {
                sem: ready,
                at: sender,
                by: 1,
            }));
            out.push(tag(ChoreoOp::SemWait {
                sem: valid,
                until: SemCond::Equals(SemValue::Valid.as_u32() as usize),
            }));
            out.push(tag(ChoreoOp::Push { buf, pages: 1 }));
        }
        _ => {}
    }
}

/// The body of one K block on a target whose accumulator LIVES IN REGISTERS: just the multiply.
///
/// No acquire, no pack, no spill — the caller owns one accumulate region around the whole K loop,
/// so each block adds into the accumulators already held. That is the only shape the GPU backend
/// can lower (its fragments never leave the registers), and it is what a real GEMM does.
fn sub_block_body_regs(s: &Stage, op: &ComputeOp, ins: &[BufferId], tiles: usize) -> Vec<ChoreoOp> {
    let _ = (s, tiles);
    vec![ChoreoOp::Compute {
        op: op.clone(),
        ins: ins.to_vec(),
        acc_reload: None,
    }]
}

fn sub_block_body(
    s: &Stage,
    op: &ComputeOp,
    ins: &[BufferId],
    reload: Option<BufferId>,
    spill_to: BufferId,
    tiles: usize,
    convert: crate::codegen_sva::fabric_target::Precision,
) -> Vec<ChoreoOp> {
    sub_block_body_acc(s, op, ins, reload, spill_to, tiles, convert, false)
}

/// The sub-block body, with the packer either replacing the spill page or accumulating into it.
#[allow(clippy::too_many_arguments)]
fn sub_block_body_acc(
    s: &Stage,
    op: &ComputeOp,
    ins: &[BufferId],
    reload: Option<BufferId>,
    spill_to: BufferId,
    tiles: usize,
    convert: crate::codegen_sva::fabric_target::Precision,
    accumulate: bool,
) -> Vec<ChoreoOp> {
    let _ = s;
    let mut body = vec![ChoreoOp::AcquireAcc { tiles }];
    if let Some(interm) = reload {
        body.push(ChoreoOp::Wait {
            buf: interm,
            pages: 1,
        });
    }
    body.push(ChoreoOp::Compute {
        op: op.clone(),
        ins: ins.to_vec(),
        acc_reload: reload,
    });
    if let Some(interm) = reload {
        body.push(ChoreoOp::Pop {
            buf: interm,
            pages: 1,
        });
    }
    body.push(ChoreoOp::Reserve {
        buf: spill_to,
        pages: 1,
    });
    body.push(ChoreoOp::PackAcc {
        to: spill_to,
        tiles,
        convert,
        accumulate,
    });
    body.push(ChoreoOp::Push {
        buf: spill_to,
        pages: 1,
    });
    body.push(ChoreoOp::ReleaseAcc);
    body
}

/// Tag every op of a single-stage sequence.
fn tagged(stage: StageId, ops: Vec<ChoreoOp>) -> Vec<TaggedOp> {
    ops.into_iter().map(|op| TaggedOp { stage, op }).collect()
}

/// Wrap a tagged body in the sub-block loops (the loop op itself carries the body's first stage).
fn sub_loops(sub_blocks: (usize, usize), body: Vec<TaggedOp>) -> Vec<TaggedOp> {
    let stage = body.first().map(|t| t.stage).unwrap_or(StageId::new(0));
    let inner = if sub_blocks.1 > 1 {
        vec![TaggedOp {
            stage,
            op: ChoreoOp::Loop {
                var: LV_SUB_W,
                count: LoopCount::Const(sub_blocks.1),
                body,
            },
        }]
    } else {
        body
    };
    if sub_blocks.0 > 1 {
        vec![TaggedOp {
            stage,
            op: ChoreoOp::Loop {
                var: LV_SUB_H,
                count: LoopCount::Const(sub_blocks.0),
                body: inner,
            },
        }]
    } else {
        inner
    }
}

/// Buffers an op sequence pushes to, and buffers it waits on.
fn touches(
    ops: &[TaggedOp],
) -> (
    std::collections::BTreeSet<usize>,
    std::collections::BTreeSet<usize>,
) {
    let (mut produces, mut consumes) = (
        std::collections::BTreeSet::new(),
        std::collections::BTreeSet::new(),
    );
    for t in ops {
        match &t.op {
            ChoreoOp::Push { buf, .. } | ChoreoOp::Reserve { buf, .. } => {
                produces.insert(buf.idx());
            }
            ChoreoOp::Wait { buf, .. } | ChoreoOp::Pop { buf, .. } => {
                consumes.insert(buf.idx());
            }
            ChoreoOp::Loop { body, .. } => {
                let (pr, co) = touches(body);
                produces.extend(pr);
                consumes.extend(co);
            }
            _ => {}
        }
    }
    (produces, consumes)
}

/// Interleave adjacent loop nests over the same index space into one nest.
///
/// An engine program is serial, so two stages emitted as consecutive whole loops run one after the other in
/// full. When those stages sit at opposite ends of the same pipeline — a load feeding the compute, and the
/// store draining it — running the first to completion fills the buffer between them and the pair deadlocks
/// against each other: the loader cannot place its next page because the consumer is blocked, and the store
/// that would unblock it has not started. Interleaving them per iteration is what makes bounded buffers
/// sufficient, and it is the same software-pipelining move that lets a fused compute chain hold at depth two.
///
/// Only nests over an identical index space fuse, and only when the later one does not drain a buffer the
/// earlier one fills, since that hand-off is a real intra-engine dependence rather than a pipeline.
fn fuse_adjacent_loops(ops: Vec<TaggedOp>) -> Vec<TaggedOp> {
    let mut out: Vec<TaggedOp> = Vec::with_capacity(ops.len());
    for t in ops {
        let merged = match (out.last_mut().map(|l| &mut l.op), &t.op) {
            (
                Some(ChoreoOp::Loop {
                    var: pv,
                    count: pc,
                    body: pb,
                }),
                ChoreoOp::Loop { var, count, body },
            ) if pv == var && pc == count => {
                let (produces, _) = touches(pb);
                let (_, consumes) = touches(body);
                if produces.is_disjoint(&consumes) {
                    pb.extend(body.iter().cloned());
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if !merged {
            out.push(t);
        }
    }
    // Fuse the bodies too, so a nest fuses at every level it matches.
    for t in out.iter_mut() {
        if let ChoreoOp::Loop { body, .. } = &mut t.op {
            *body = fuse_adjacent_loops(std::mem::take(body));
        }
    }
    out
}

/// A dataflow rank per stage: a stage that fills a buffer ranks before every stage that drains it.
///
/// Engine programs are serial, so the order stages take on ONE engine is a correctness property, not a
/// cosmetic one: a stage that waits on data a LATER stage of the same engine produces can never run, and the
/// program deadlocks against itself. Declaration order does not carry that guarantee — a lane that appends a
/// load after the store it feeds is perfectly reasonable to write and fatal to schedule. Ranking by dataflow
/// makes the safe order the only order any engine can be built in. Ties keep stage-id order, so a program
/// with no cross-stage dependency schedules exactly as it always did.
fn stage_ranks(p: &SpatialProgram) -> Vec<usize> {
    let n = p.stages.len();
    let mut producer_of: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for s in &p.stages {
        for b in &s.outs {
            producer_of.entry(b.idx()).or_default().push(s.id.idx());
        }
    }
    // ★ A CARRY IS READ BEFORE IT IS UPDATED. Within a K block every other body reader of a carry sees the value the
    // block started with, so it ranks AHEAD of the carry's update — the edge the update's write would otherwise add
    // (update → reader) is the wrong way round inside the block. The epilogue reads what the last update left.
    let carries = p.carries();
    let levels = if carries.is_empty() {
        vec![FoldLevel::Prologue; n]
    } else {
        p.fold_levels()
    };
    let mut indeg = vec![0usize; n];
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut arc = |from: usize, to: usize, succ: &mut Vec<Vec<usize>>| {
        succ[from].push(to);
        indeg[to] += 1;
    };
    for s in &p.stages {
        for b in &s.ins {
            let update = carries.get(b).map(|c| c.idx());
            for &pr in producer_of
                .get(&b.idx())
                .map(|v| v.as_slice())
                .unwrap_or(&[])
            {
                if pr == s.id.idx() {
                    continue;
                }
                if Some(pr) == update && levels[s.id.idx()] == FoldLevel::Body {
                    arc(s.id.idx(), pr, &mut succ);
                } else {
                    arc(pr, s.id.idx(), &mut succ);
                }
            }
        }
    }
    let mut ready: std::collections::BTreeSet<usize> = (0..n).filter(|i| indeg[*i] == 0).collect();
    let mut rank = vec![usize::MAX; n];
    let mut next = 0usize;
    while let Some(&i) = ready.iter().next() {
        ready.remove(&i);
        rank[i] = next;
        next += 1;
        for &t in &succ[i] {
            indeg[t] -= 1;
            if indeg[t] == 0 {
                ready.insert(t);
            }
        }
    }
    // A cycle among stages is a template defect the validator reports; keep declaration order for those.
    for (i, r) in rank.iter_mut().enumerate() {
        if *r == usize::MAX {
            *r = next + i;
        }
    }
    rank
}

/// One pass of a compute chain over a single page: every stage waits its input, computes, pushes, pops.
///
/// ★ A PAGE IS POPPED BY ITS LAST READER, NOT ITS FIRST. A chain stage used to pop everything it
/// read, which is right only while no two stages share an operand. The moment one does — a
/// normalization that reduces `x` and then rescales `x`, or an `x·x` square whose operand is named
/// twice — the first stage retires the page the second is still waiting on, and the site deadlocks.
/// Popping at the last use (once per buffer, however many times it is named) makes the rule hold for
/// any chain shape rather than for the shapes that happened to exist.
fn chain_ops(p: &SpatialProgram, chain: &[&Stage]) -> Vec<TaggedOp> {
    let mut last_use: std::collections::BTreeMap<BufferId, usize> = Default::default();
    for (i, s) in chain.iter().enumerate() {
        for b in operands(s) {
            last_use.insert(b, i);
        }
    }
    let mut body: Vec<TaggedOp> = Vec::new();
    for (i, s) in chain.iter().enumerate() {
        let op = match &s.kind {
            StageKind::Compute(op) => op.clone(),
            StageKind::Scalar(b) => ComputeOp::Scalar(b.clone()),
            StageKind::AllReduce { combine, .. } => (**combine).clone(),
            _ => continue,
        };
        let out = s.outs[0];
        let out_dtype = p.buffer(out).dtype;
        let tiles = p.buffer(out).kind.page_tiles();
        let reads = operands(s);
        let mut seq: Vec<ChoreoOp> = reads
            .iter()
            .map(|b| ChoreoOp::Wait { buf: *b, pages: 1 })
            .collect();
        seq.extend([
            ChoreoOp::AcquireAcc { tiles },
            ChoreoOp::Compute {
                op,
                ins: s.ins.clone(),
                acc_reload: None,
            },
            ChoreoOp::Reserve { buf: out, pages: 1 },
            ChoreoOp::PackAcc {
                to: out,
                tiles,
                convert: out_dtype,
                accumulate: false,
            },
            ChoreoOp::Push { buf: out, pages: 1 },
            ChoreoOp::ReleaseAcc,
        ]);
        seq.extend(
            reads
                .iter()
                .filter(|b| last_use.get(b) == Some(&i))
                .map(|b| ChoreoOp::Pop { buf: *b, pages: 1 }),
        );
        body.extend(tagged(s.id, seq));
    }
    body
}

/// The distinct buffers a chain stage reads, in first-mention order — `s.ins` deduplicated, plus any
/// operand the op carries inside itself rather than in `ins` (a reduction's scaler tile).
fn operands(s: &Stage) -> Vec<BufferId> {
    let mut out: Vec<BufferId> = Vec::new();
    let mut push = |b: BufferId| {
        if !out.contains(&b) {
            out.push(b);
        }
    };
    for b in &s.ins {
        push(*b);
    }
    out
}

/// One chain stage's ops: wait its operands, compute into the accumulator, pack its page, then pop `pops`.
fn stage_ops(p: &SpatialProgram, s: &Stage, pops: &[BufferId]) -> Vec<TaggedOp> {
    let op = match &s.kind {
        StageKind::Compute(op) => op.clone(),
        StageKind::Scalar(b) => ComputeOp::Scalar(b.clone()),
        StageKind::AllReduce { combine, .. } => (**combine).clone(),
        _ => return Vec::new(),
    };
    let out = s.outs[0];
    let tiles = p.buffer(out).kind.page_tiles();
    let mut seq: Vec<ChoreoOp> = operands(s)
        .into_iter()
        .map(|b| ChoreoOp::Wait { buf: b, pages: 1 })
        .collect();
    seq.extend([
        ChoreoOp::AcquireAcc { tiles },
        ChoreoOp::Compute {
            op,
            ins: s.ins.clone(),
            acc_reload: None,
        },
        ChoreoOp::Reserve { buf: out, pages: 1 },
        ChoreoOp::PackAcc {
            to: out,
            tiles,
            convert: p.buffer(out).dtype,
            accumulate: false,
        },
        ChoreoOp::Push { buf: out, pages: 1 },
        ChoreoOp::ReleaseAcc,
    ]);
    seq.extend(pops.iter().map(|b| ChoreoOp::Pop { buf: *b, pages: 1 }));
    tagged(s.id, seq)
}

/// The compute chain of a K-block FOLD: its prologue once, its body once per K block (`nb`), its epilogue once
/// ([`FoldLevel`]).
///
/// ★ EVERY PAGE IS POPPED ONCE, WHERE ITS LAST READER RUNS. A page the prologue made that the body reads stays
/// visible across the blocks and is popped after them — or by its last epilogue reader; a body page is popped by its
/// last reader in the block; a carry's page from before the block is popped by the carry's update once the new page
/// is pushed, and the page the last block leaves by its last epilogue reader.
/// The count of the K-block loops of `p`'s plan of `nb` blocks: its trip's, when its blocks run to one.
fn k_count(p: &SpatialProgram, nb: usize) -> LoopCount {
    match p.work.k_trip {
        Some(trip) => LoopCount::Trip { trip, max: nb },
        None => LoopCount::Const(nb),
    }
}

fn fold_ops(
    p: &SpatialProgram,
    chain: &[&Stage],
    levels: &[FoldLevel],
    nb: usize,
) -> Vec<TaggedOp> {
    let carries = p.carries();
    let at = |l: FoldLevel| {
        chain
            .iter()
            .copied()
            .filter(|s| levels[s.id.idx()] == l)
            .collect::<Vec<&Stage>>()
    };
    let (pro, body, epi) = (
        at(FoldLevel::Prologue),
        at(FoldLevel::Body),
        at(FoldLevel::Epilogue),
    );
    let last_reader = |stages: &[&Stage], b: BufferId| {
        stages
            .iter()
            .rev()
            .find(|s| s.ins.contains(&b))
            .map(|s| s.id)
    };
    let producer_level = |b: BufferId| {
        p.stages
            .iter()
            .filter(|s| s.outs.contains(&b))
            .map(|s| levels[s.id.idx()])
            .max()
            .unwrap_or(FoldLevel::Prologue)
    };
    let mut pops: BTreeMap<StageId, Vec<BufferId>> = BTreeMap::new();
    let mut after_blocks: Vec<BufferId> = Vec::new();
    let mut read: Vec<BufferId> = Vec::new();
    for s in chain {
        for b in operands(s) {
            if !read.contains(&b) {
                read.push(b);
            }
        }
    }
    for b in read {
        let final_pop = |pops: &mut BTreeMap<StageId, Vec<BufferId>>, after: &mut Vec<BufferId>| {
            match last_reader(&epi, b) {
                Some(r) => pops.entry(r).or_default().push(b),
                None => after.push(b),
            }
        };
        if let Some(update) = carries.get(&b) {
            pops.entry(*update).or_default().push(b);
            final_pop(&mut pops, &mut after_blocks);
            continue;
        }
        match producer_level(b) {
            FoldLevel::Body => {
                if let Some(r) = last_reader(&body, b) {
                    pops.entry(r).or_default().push(b);
                }
            }
            FoldLevel::Prologue
                if last_reader(&epi, b).is_some() || last_reader(&body, b).is_some() =>
            {
                final_pop(&mut pops, &mut after_blocks)
            }
            FoldLevel::Prologue => {
                if let Some(r) = last_reader(&pro, b) {
                    pops.entry(r).or_default().push(b);
                }
            }
            FoldLevel::Epilogue => {
                if let Some(r) = last_reader(&epi, b) {
                    pops.entry(r).or_default().push(b);
                }
            }
        }
    }
    let none: Vec<BufferId> = Vec::new();
    let ops_of = |stages: &[&Stage]| {
        stages
            .iter()
            .flat_map(|s| stage_ops(p, s, pops.get(&s.id).unwrap_or(&none)))
            .collect::<Vec<TaggedOp>>()
    };
    let mut out = ops_of(&pro);
    let blocks = ops_of(&body);
    let last_body = body.last().map(|s| s.id).unwrap_or(StageId::new(0));
    if nb > 1 {
        if let Some(first) = blocks.first().map(|t| t.stage) {
            out.push(TaggedOp {
                stage: first,
                op: ChoreoOp::Loop {
                    var: LV_KBLOCK,
                    count: k_count(p, nb),
                    body: blocks,
                },
            });
        }
    } else {
        out.extend(blocks);
    }
    // A carry's last page was pushed by the last block's update and never waited on — wait, then retire it.
    out.extend(tagged(
        last_body,
        after_blocks
            .into_iter()
            .flat_map(|b| {
                [
                    ChoreoOp::Wait { buf: b, pages: 1 },
                    ChoreoOp::Pop { buf: b, pages: 1 },
                ]
            })
            .collect(),
    ));
    out.extend(ops_of(&epi));
    out
}

/// The compute engine program of a site; `here` is the site's stages in id order, `rank` every stage's dataflow rank.
fn compute_program(
    p: &SpatialProgram,
    here: &[&Stage],
    rank: &[usize],
    levels: &[FoldLevel],
) -> Result<Vec<TaggedOp>, SpatialReject> {
    let plan = p.work.plan.expect("tiled");
    let mut ops = Vec::new();
    let mut stages: Vec<&Stage> = here
        .iter()
        .copied()
        .filter(|s| s.role == SiteRole::Compute)
        .collect();
    stages.sort_by_key(|s| rank[s.id.idx()]);
    let stages = stages;
    // The compute CHAIN after the matmul (requant → bias → relu …). Its iteration space is the matmul's
    // own sub-block nest, so it is emitted INSIDE that nest: each output page is consumed by the chain in
    // the same turn the matmul produced it. Emitting the two nests in sequence instead would have the
    // matmul push every page of an item before the chain popped any, which needs a hand-off as deep as the
    // item and deadlocks at depth two.
    let mut chain: Vec<&Stage> = here
        .iter()
        .copied()
        .filter(|s| {
            s.role == SiteRole::Compute
                && matches!(
                    s.kind,
                    StageKind::Compute(_) | StageKind::Scalar(_) | StageKind::AllReduce { .. }
                )
                && !matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. }))
        })
        .collect();
    chain.sort_by_key(|s| rank[s.id.idx()]);
    let chain = chain;
    let chain_body = chain_ops(p, &chain);
    let has_matmul = stages
        .iter()
        .any(|s| matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. })));
    let fused = has_matmul && !chain.is_empty();
    let fuse = |body: Vec<TaggedOp>| -> Vec<TaggedOp> {
        if fused {
            body.into_iter().chain(chain_body.iter().cloned()).collect()
        } else {
            body
        }
    };
    for s in stages {
        let tag = |op: ChoreoOp| TaggedOp { stage: s.id, op };
        let out = s.outs[0];
        let out_dtype = p.buffer(out).dtype;
        match &s.kind {
            StageKind::Compute(
                op @ ComputeOp::MatmulTiles {
                    block,
                    sub_blocks,
                    num_blocks,
                    interm,
                    ..
                },
            ) => {
                if p.work.k_trip.is_some() {
                    return Err(SpatialReject::NotLowered("a matmul whose K blocks run to a trip — its accumulate-and-spill forms peel blocks a count read from the data may not have"));
                }
                let ins = s.ins.clone();
                let tiles = block.acc_tiles();
                let nb = *num_blocks;
                let sid = s.id;
                if is_streaming(&plan) {
                    // Accumulate every K step in the register file, pack once.
                    let k_body = tagged(
                        sid,
                        vec![
                            ChoreoOp::Wait {
                                buf: ins[0],
                                pages: 1,
                            },
                            ChoreoOp::Wait {
                                buf: ins[1],
                                pages: 1,
                            },
                            ChoreoOp::Compute {
                                op: op.clone(),
                                ins: ins.clone(),
                                acc_reload: None,
                            },
                            ChoreoOp::Pop {
                                buf: ins[0],
                                pages: 1,
                            },
                            ChoreoOp::Pop {
                                buf: ins[1],
                                pages: 1,
                            },
                        ],
                    );
                    let mut seq = vec![tag(ChoreoOp::AcquireAcc { tiles })];
                    if nb > 1 {
                        seq.push(tag(ChoreoOp::Loop {
                            var: LV_KBLOCK,
                            count: LoopCount::Const(nb),
                            body: k_body,
                        }));
                    } else {
                        seq.extend(k_body);
                    }
                    seq.extend(fuse(tagged(
                        sid,
                        vec![
                            ChoreoOp::Reserve { buf: out, pages: 1 },
                            ChoreoOp::PackAcc {
                                to: out,
                                tiles,
                                convert: out_dtype,
                                accumulate: false,
                            },
                            ChoreoOp::Push { buf: out, pages: 1 },
                            ChoreoOp::ReleaseAcc,
                        ],
                    )));
                    ops.extend(seq);
                    continue;
                }
                let waits = tagged(
                    sid,
                    vec![
                        ChoreoOp::Wait {
                            buf: ins[0],
                            pages: 1,
                        },
                        ChoreoOp::Wait {
                            buf: ins[1],
                            pages: 1,
                        },
                    ],
                );
                let pops = tagged(
                    sid,
                    vec![
                        ChoreoOp::Pop {
                            buf: ins[0],
                            pages: 1,
                        },
                        ChoreoOp::Pop {
                            buf: ins[1],
                            pages: 1,
                        },
                    ],
                );
                if nb > 1 && interm.is_none() {
                    // ★ A REGISTER-RESIDENT ACCUMULATOR: the partial never touches memory, so the
                    // whole K reduction runs inside ONE accumulate region and packs once at the end
                    // — the same shape as streaming, but over a real block instead of one tile.
                    //
                    // This is the GPU. Its `wmma` fragments live in registers for the duration, which
                    // is why the CUDA lowering refuses an accumulator spill outright; a plan that
                    // spilled here would emit a construct the backend cannot lower. The tiling pass
                    // guarantees `sub_blocks == (1, 1)` on such a target, so this single accumulator
                    // set covers the block.
                    let k_body = waits
                        .iter()
                        .cloned()
                        .chain(tagged(sid, sub_block_body_regs(s, op, &ins, tiles)))
                        .chain(pops.iter().cloned())
                        .collect::<Vec<_>>();
                    let mut seq = vec![tag(ChoreoOp::AcquireAcc { tiles })];
                    seq.push(tag(ChoreoOp::Loop {
                        var: LV_KBLOCK,
                        count: LoopCount::Const(nb),
                        body: k_body,
                    }));
                    seq.extend(fuse(tagged(
                        sid,
                        vec![
                            ChoreoOp::Reserve { buf: out, pages: 1 },
                            ChoreoOp::PackAcc {
                                to: out,
                                tiles,
                                convert: out_dtype,
                                accumulate: false,
                            },
                            ChoreoOp::Push { buf: out, pages: 1 },
                            ChoreoOp::ReleaseAcc,
                        ],
                    )));
                    ops.extend(seq);
                    continue;
                }
                if nb == 1 {
                    let body = sub_loops(
                        *sub_blocks,
                        fuse(tagged(
                            sid,
                            sub_block_body(s, op, &ins, None, out, tiles, out_dtype),
                        )),
                    );
                    ops.extend(waits.into_iter().chain(body).chain(pops));
                } else if let Some(interm) = interm.filter(|i| p.buffer(*i).alias_of == Some(out)) {
                    // ★ THE PARTIAL IS THE RESULT. The intermediate shares the output's memory page for page, so
                    // every block accumulates in place and the last one packs into the output itself — the page
                    // the writer stores — with no reload. The item's output pages are reserved before any partial
                    // lands in their memory: the reservation waits until the writer has stored the previous
                    // item's, and the last block packs into it without reserving again.
                    let interm_dtype = p.buffer(interm).dtype;
                    let subs = sub_blocks.0 * sub_blocks.1;
                    let drain: Vec<TaggedOp> = tagged(
                        sid,
                        (0..subs)
                            .flat_map(|_| {
                                [
                                    ChoreoOp::Wait {
                                        buf: interm,
                                        pages: 1,
                                    },
                                    ChoreoOp::Pop {
                                        buf: interm,
                                        pages: 1,
                                    },
                                ]
                            })
                            .collect(),
                    );
                    let mut seq = vec![tag(ChoreoOp::Reserve {
                        buf: out,
                        pages: subs,
                    })];
                    // Block 0: fresh pages, drained so the write pointer returns to the first.
                    seq.extend(waits.clone());
                    seq.extend(sub_loops(
                        *sub_blocks,
                        tagged(
                            sid,
                            sub_block_body_acc(
                                s,
                                op,
                                &ins,
                                None,
                                interm,
                                tiles,
                                interm_dtype,
                                false,
                            ),
                        ),
                    ));
                    seq.extend(drain.clone());
                    seq.extend(pops.clone());
                    // Blocks 1…nb-2: accumulate, drained.
                    if nb > 2 {
                        let mut mid = waits.clone();
                        mid.extend(sub_loops(
                            *sub_blocks,
                            tagged(
                                sid,
                                sub_block_body_acc(
                                    s,
                                    op,
                                    &ins,
                                    None,
                                    interm,
                                    tiles,
                                    interm_dtype,
                                    true,
                                ),
                            ),
                        ));
                        mid.extend(drain.clone());
                        mid.extend(pops.clone());
                        seq.push(tag(ChoreoOp::Loop {
                            var: LV_KBLOCK,
                            count: LoopCount::Const(nb - 2),
                            body: mid,
                        }));
                    }
                    // Block nb-1: accumulate into the output's page, which holds the partial, and hand it on.
                    seq.extend(waits);
                    seq.extend(sub_loops(
                        *sub_blocks,
                        tagged(
                            sid,
                            vec![
                                ChoreoOp::AcquireAcc { tiles },
                                ChoreoOp::Compute {
                                    op: op.clone(),
                                    ins: ins.clone(),
                                    acc_reload: None,
                                },
                                ChoreoOp::PackAcc {
                                    to: out,
                                    tiles,
                                    convert: out_dtype,
                                    accumulate: true,
                                },
                                ChoreoOp::Push { buf: out, pages: 1 },
                                ChoreoOp::ReleaseAcc,
                            ],
                        ),
                    ));
                    seq.extend(pops);
                    ops.extend(seq);
                } else if plan.pack_accumulate {
                    // The packer accumulates the partials in place: blocks 0…nb-2 add into the intermediate
                    // page and the register file is reloaded exactly once, at the last block. Every block but
                    // the last two drains its pages so the intermediate write pointer returns to the page it
                    // is accumulating into (the vendor's `pack_reconfig_l1_acc` discipline).
                    let interm = interm.expect("a multi-block plan spills");
                    let interm_dtype = p.buffer(interm).dtype;
                    let subs = sub_blocks.0 * sub_blocks.1;
                    let drain: Vec<TaggedOp> = tagged(
                        sid,
                        (0..subs)
                            .flat_map(|_| {
                                [
                                    ChoreoOp::Wait {
                                        buf: interm,
                                        pages: 1,
                                    },
                                    ChoreoOp::Pop {
                                        buf: interm,
                                        pages: 1,
                                    },
                                ]
                            })
                            .collect(),
                    );
                    let mut seq = Vec::new();
                    // Block 0: fresh pages (replace), then drain.
                    seq.extend(waits.clone());
                    seq.extend(sub_loops(
                        *sub_blocks,
                        tagged(
                            sid,
                            sub_block_body_acc(
                                s,
                                op,
                                &ins,
                                None,
                                interm,
                                tiles,
                                interm_dtype,
                                false,
                            ),
                        ),
                    ));
                    seq.extend(drain.clone());
                    seq.extend(pops.clone());
                    // Blocks 1…nb-3: accumulate, then drain.
                    if nb > 3 {
                        let mut mid = waits.clone();
                        mid.extend(sub_loops(
                            *sub_blocks,
                            tagged(
                                sid,
                                sub_block_body_acc(
                                    s,
                                    op,
                                    &ins,
                                    None,
                                    interm,
                                    tiles,
                                    interm_dtype,
                                    true,
                                ),
                            ),
                        ));
                        mid.extend(drain.clone());
                        mid.extend(pops.clone());
                        seq.push(tag(ChoreoOp::Loop {
                            var: LV_KBLOCK,
                            count: LoopCount::Const(nb - 3),
                            body: mid,
                        }));
                    }
                    // Block nb-2: accumulate and LEAVE the pages for the reload.
                    seq.extend(waits.clone());
                    seq.extend(sub_loops(
                        *sub_blocks,
                        tagged(
                            sid,
                            sub_block_body_acc(
                                s,
                                op,
                                &ins,
                                None,
                                interm,
                                tiles,
                                interm_dtype,
                                true,
                            ),
                        ),
                    ));
                    seq.extend(pops.clone());
                    // Block nb-1: the single reload, packed to the output.
                    seq.extend(waits);
                    seq.extend(sub_loops(
                        *sub_blocks,
                        fuse(tagged(
                            sid,
                            sub_block_body(s, op, &ins, Some(interm), out, tiles, out_dtype),
                        )),
                    ));
                    seq.extend(pops);
                    ops.extend(seq);
                } else {
                    let interm = interm.expect("a multi-block plan spills");
                    let interm_dtype = p.buffer(interm).dtype;
                    let mut seq = Vec::new();
                    // Block 0: fresh accumulators, spill.
                    seq.extend(waits.clone());
                    seq.extend(sub_loops(
                        *sub_blocks,
                        tagged(
                            sid,
                            sub_block_body(s, op, &ins, None, interm, tiles, interm_dtype),
                        ),
                    ));
                    seq.extend(pops.clone());
                    // Middle blocks: reload, spill.
                    if nb > 2 {
                        let mut mid = waits.clone();
                        mid.extend(sub_loops(
                            *sub_blocks,
                            tagged(
                                sid,
                                sub_block_body(
                                    s,
                                    op,
                                    &ins,
                                    Some(interm),
                                    interm,
                                    tiles,
                                    interm_dtype,
                                ),
                            ),
                        ));
                        mid.extend(pops.clone());
                        seq.push(tag(ChoreoOp::Loop {
                            var: LV_KBLOCK,
                            count: LoopCount::Const(nb - 2),
                            body: mid,
                        }));
                    }
                    // Last block: reload, pack to the output.
                    seq.extend(waits);
                    seq.extend(sub_loops(
                        *sub_blocks,
                        fuse(tagged(
                            sid,
                            sub_block_body(s, op, &ins, Some(interm), out, tiles, out_dtype),
                        )),
                    ));
                    seq.extend(pops);
                    ops.extend(seq);
                }
            }
            StageKind::Compute(_) | StageKind::Scalar(_) | StageKind::AllReduce { .. } => {
                // Chain stages are emitted together below (one page loop, every stage per page).
                continue;
            }
            _ => {}
        }
    }
    // A chain that CARRIES a value across K blocks is a fold: its prologue, its blocks, its epilogue.
    let carries = p.carries();
    if !fused && chain.iter().any(|s| carries.values().any(|c| *c == s.id)) {
        let sub =
            (plan.per_site_m / plan.block.osh.max(1)) * (plan.per_site_n / plan.block.osw.max(1));
        if sub > 1 {
            return Err(SpatialReject::NotLowered("a chain that carries a value across K blocks over more than one output sub-block of its item"));
        }
        ops.extend(fold_ops(p, &chain, levels, plan.inner.num_blocks));
        return Ok(ops);
    }
    // A chain with no matmul ahead of it (a pure scalar or elementwise site) keeps its own page loop.
    if !chain.is_empty() && !fused {
        let pages = chain
            .iter()
            .flat_map(|s| s.ins.iter().map(|b| pages_per_item(p, *b, 0)))
            .max()
            .unwrap_or(1);
        let body = chain_body;
        if pages > 1 {
            let stage = body[0].stage;
            ops.push(TaggedOp {
                stage,
                op: ChoreoOp::Loop {
                    var: LV_SUB_H,
                    count: LoopCount::Const(pages),
                    body,
                },
            });
        } else {
            ops.extend(body);
        }
    }
    Ok(ops)
}

/// The reader engine program of a site: the operand loads/receives (A before B) inside the K-block loop —
/// always a loop, as the vendor's readers are — then any per-sub-block loads (a bias row) in the
/// sub-block loops, then any once-per-item loads.
fn reader_program(
    p: &SpatialProgram,
    target: &SpatialTarget,
    here: &[&Stage],
    rank: &[usize],
    engine: Engine,
) -> Vec<TaggedOp> {
    let pl = p.placement.as_ref().unwrap();
    let mut stages: Vec<&Stage> = here
        .iter()
        .copied()
        .filter(|s| {
            pl.engine_of[s.id.idx()].engine == engine
                && matches!(s.kind, StageKind::Load { .. } | StageKind::Recv { .. })
        })
        .collect();
    stages.sort_by_key(|s| rank[s.id.idx()]);
    let stages = stages;
    if stages.is_empty() {
        return Vec::new();
    }
    let plan = p.work.plan;
    let nb = plan.map(|pl| pl.inner.num_blocks).unwrap_or(1);
    let sub_blocks = plan
        .map(|pl| {
            (
                (pl.per_site_m / pl.block.osh.max(1)).max(1),
                (pl.per_site_n / pl.block.osw.max(1)).max(1),
            )
        })
        .unwrap_or((1, 1));
    let uses = |s: &Stage, var: LoopVar| match &s.kind {
        StageKind::Load { pattern, .. } => pattern.origin.steps.iter().any(|(v, _)| *v == var),
        StageKind::Recv { .. } => var == LV_KBLOCK,
        _ => false,
    };
    let (mut k_block, mut sub_block, mut once) = (Vec::new(), Vec::new(), Vec::new());
    for s in &stages {
        let is_k = uses(s, LV_KBLOCK) || matches!(s.kind, StageKind::Recv { .. });
        let is_sub = uses(s, LV_SUB_H) || uses(s, LV_SUB_W);
        let target_vec = if is_k {
            &mut k_block
        } else if is_sub {
            &mut sub_block
        } else {
            &mut once
        };
        reader_block(p, target, s, target_vec);
    }
    // ★ WHAT IS LOADED ONCE IS LOADED FIRST. A once-per-item operand may be what the compute needs before its first
    // block — a fold's prologue reads it — and a reader that issued it after its block loop would fill the block
    // buffers and wait forever on a compute that waits for it.
    let mut ops = once;
    if let Some(first) = k_block.first() {
        let stage = first.stage;
        ops.push(TaggedOp {
            stage,
            op: ChoreoOp::Loop {
                var: LV_KBLOCK,
                count: k_count(p, nb),
                body: k_block,
            },
        });
    }
    if !sub_block.is_empty() {
        ops.extend(sub_loops(sub_blocks, sub_block));
    }
    ops
}

/// The writer engine program of a site: one store per sub-block.
fn writer_program(
    p: &SpatialProgram,
    here: &[&Stage],
    rank: &[usize],
    engine: Engine,
) -> Vec<TaggedOp> {
    let pl = p.placement.as_ref().unwrap();
    let mut ops = Vec::new();
    let mut engine_stages: Vec<&Stage> = here
        .iter()
        .copied()
        .filter(|s| pl.engine_of[s.id.idx()].engine == engine)
        .collect();
    engine_stages.sort_by_key(|s| rank[s.id.idx()]);
    for s in engine_stages {
        if let StageKind::Store {
            tensor,
            pattern,
            sub_blocks,
        } = &s.kind
        {
            let buf = s.ins[0];
            // A scatter (an `IndirectTile` store) waits for its index as a gather does, and retires it after.
            let index = pattern.origin.indirect.map(|i| i.index);
            let mut body = vec![ChoreoOp::Wait { buf, pages: 1 }];
            if let Some(ix) = index {
                body.push(ChoreoOp::Wait { buf: ix, pages: 1 });
            }
            body.extend([
                ChoreoOp::WriteTiles {
                    from: buf,
                    tensor: *tensor,
                    at: pattern.origin.clone(),
                    block_h: pattern.block_h,
                    block_w: pattern.block_w,
                    stride_w: pattern.stride_w,
                    stride_h: pattern.stride_h,
                },
                ChoreoOp::WriteBarrier,
                ChoreoOp::Pop { buf, pages: 1 },
            ]);
            if let Some(ix) = index {
                body.push(ChoreoOp::Pop { buf: ix, pages: 1 });
            }
            ops.extend(sub_loops(*sub_blocks, tagged(s.id, body)));
        }
    }
    ops
}

/// The `synthesize_handshakes` pass.
pub fn synthesize_handshakes(
    p: &SpatialProgram,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    if p.schedule.is_some() {
        return Ok(p.clone());
    }
    let Some(pl) = &p.placement else {
        return Err(SpatialReject::WrongPhase {
            pass: "synthesize_handshakes",
            expected: "Sized",
            found: p.phase(),
        });
    };
    if p.memory.is_none() {
        return Err(SpatialReject::WrongPhase {
            pass: "synthesize_handshakes",
            expected: "Sized",
            found: p.phase(),
        });
    }
    // The template's abstract local hand-off becomes the kind this target provides; every other kind was put
    // there by a pass that already checked the target.
    let local = crate::spatial::passes::from_tensor_op::local_handshake(target)?;
    let mut p = p.clone();
    for e in &mut p.edges {
        if e.handshake == Handshake::Fifo && local != Handshake::Fifo {
            e.handshake = local.clone();
        }
    }
    p.requires.handshakes = p.edges.iter().map(|e| e.handshake.kind()).collect();
    let p = &p;
    for e in &p.edges {
        let k = e.handshake.kind();
        if !target.provides_handshake(k) {
            return Err(SpatialReject::MissingHandshake {
                needed: k,
                target: target.name,
            });
        }
    }
    // Every stage's dataflow rank and each site's stages, derived once: each engine program of each site is built from
    // its own site's stages, not from a walk of the whole program.
    let rank = stage_ranks(p);
    let levels = if p.carries().is_empty() {
        vec![FoldLevel::Prologue; p.stages.len()]
    } else {
        p.fold_levels()
    };
    let mut on_site: BTreeMap<Site, Vec<&Stage>> = BTreeMap::new();
    for s in &p.stages {
        if let Some(site) = s.site {
            on_site.entry(site).or_default().push(s);
        }
    }
    let mut programs = Vec::new();
    let sites: Vec<Site> = pl.items_of_site.iter().map(|(s, _)| *s).collect();
    for site in sites {
        let here: &[&Stage] = on_site.get(&site).map(Vec::as_slice).unwrap_or(&[]);
        let mut engines: Vec<Engine> = here
            .iter()
            .map(|s| pl.engine_of[s.id.idx()].engine)
            .collect();
        engines.sort();
        engines.dedup();
        for engine in engines {
            let ops = match engine {
                Engine::Compute => compute_program(p, here, &rank, &levels)?,
                Engine::Movement(_) => {
                    let mut ops = reader_program(p, target, here, &rank, engine);
                    ops.extend(writer_program(p, here, &rank, engine));
                    fuse_adjacent_loops(ops)
                }
            };
            if ops.is_empty() {
                continue;
            }
            let stages: Vec<StageId> = here
                .iter()
                .filter(|s| pl.engine_of[s.id.idx()].engine == engine)
                .map(|s| s.id)
                .collect();
            programs.push(EngineProgram {
                site,
                engine,
                stages,
                ops,
            });
        }
    }
    let mut q = p.clone();
    q.schedule = Some(Schedule { programs });
    q.validate()?;
    Ok(q)
}
