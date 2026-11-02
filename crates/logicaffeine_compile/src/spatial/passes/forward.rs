//! `forward` — one section's result handed to the next section's compute, never through memory.
//!
//! ★ A RESULT ONLY ITS NEXT READER NEEDS NEVER LEAVES THE SITE. Section k computes a tensor and stores it; section
//! k+1 loads the same tiles back and computes on them. When nothing but section k+1 reads the tensor — no later
//! section, no `Show`, no carried state — the round trip moves nothing anyone keeps: the store, the load, the boundary
//! between the two sections and the second section's own acquire are all overhead on a chain of dependent sections.
//! Forwarding joins the two TEMPLATES into one section: the buffer the producer's store read now feeds the consumer's
//! compute, and the tensor is gone. It joins templates (`Phase::Template`), before `tile`, so the joined section is
//! tiled, placed, scheduled and certified as one and `fuse_generic` then folds a map into the map it follows; and it
//! joins two sections `tile` has planned (`Phase::Tiled`), where a matmul's items — a work item per output tile in its
//! template — meet the one item a row map walks only once the plan has grouped them.
//!
//! ★ THE STORE AND THE LOAD AGREE, OR NOTHING IS JOINED. The producer stores the tensor once, from a buffer nothing
//! else reads; the consumer loads it once, into a buffer only computes read; the two sections walk the same work items
//! on the same tile, and for every one of them the store and the load address the same tiles of the same tensor in the
//! same order ([`AccessPattern::tiles_at`]). Then the page the producer packs is exactly the page the consumer would
//! have loaded. A store whose address steps with its item or its sub-block — a matmul's — agrees only under the plan
//! the template's items were laid out for: one item, packed once. Two templates join through one only where each walks
//! one item, a tile, which `tile` can only plan as one item packed once: a wider pair, whose plan packs a block of tiles
//! an item where the consumer was laid out a tile an item, would compute its first tile alone. Two planned sections join
//! through one only under a plan of one item whose every matmul packs once ([`one_block`]). Whether the tensor is dead
//! after the consumer is the caller's to know (it reads the whole program); this pass never decides it. Nor is whether
//! the join keeps what the value was proven to hold: the buffer that now carries it
//! ([`Joint::carrier`]) holds what the producer's compute proves, and the caller, holding the front end's intervals,
//! judges whether that is wider than what a load of the tensor would have read.

use crate::spatial::ir::*;

/// Two sections joined into one.
#[derive(Clone, Debug)]
pub struct Joint {
    /// The joined section, named `first+second`: a template, or planned as the producer was.
    pub program: SpatialProgram,
    /// The buffer the producer's store read, which now feeds the consumer's compute: the forwarded tensor's values.
    pub carrier: BufferId,
}

/// Whether `p`, a section the passes have planned, holds a join through a store that steps with its item or sub-block:
/// one work item, and every matmul a single sub-block.
pub fn one_block(p: &SpatialProgram) -> bool {
    p.work.items.len() == 1
        && p.stages.iter().all(|s| match &s.kind {
            StageKind::Compute(ComputeOp::MatmulTiles { sub_blocks, .. }) => *sub_blocks == (1, 1),
            _ => true,
        })
}

/// `first` and `second` joined through `tensor`, the one `first` stores and `second` loads, or `None` where the store
/// and the load do not agree (see the module doc).
pub fn forward(first: &SpatialProgram, second: &SpatialProgram, tensor: &str) -> Option<Joint> {
    // Two templates, or two sections `tile` has planned and nothing has placed: a planned consumer runs under the
    // producer's plan, so it brings no plan of its own that counts trips.
    let tiled = match (first.phase(), second.phase()) {
        (Phase::Template, Phase::Template) => false,
        (Phase::Tiled, Phase::Tiled) if second.trips.is_empty() && second.work.k_trip.is_none() => true,
        _ => return None,
    };
    // The consumer walks the producer's items; a contraction is the producer's alone (a map's `k_tiles` is 1), and the
    // joined section keeps it.
    if first.tile != second.tile
        || first.work.items != second.work.items
        || (second.work.k_tiles != 1 && second.work.k_tiles != first.work.k_tiles)
        || first.mem_banks != second.mem_banks
        || first.numerics != second.numerics
    {
        return None;
    }
    let t1 = first.tensors.iter().find(|t| t.name == tensor)?;
    let t2 = second.tensors.iter().find(|t| t.name == tensor)?;
    if (t1.elems, t1.tiles, t1.dtype, t1.layout) != (t2.elems, t2.tiles, t2.dtype, t2.layout) {
        return None;
    }
    // The producer's one store: of this tensor, and the only stage that reads its buffer.
    let stores: Vec<&Stage> = first.stages.iter().filter(|s| matches!(s.kind, StageKind::Store { .. })).collect();
    let [store] = stores.as_slice() else { return None };
    let StageKind::Store { tensor: st, pattern: sp, sub_blocks } = &store.kind else { return None };
    if *st != t1.id || store.ins.len() != 1 {
        return None;
    }
    let o = store.ins[0];
    if first.stages.iter().any(|s| s.id != store.id && s.ins.contains(&o)) {
        return None;
    }
    // The tensor leaves both sections whole: no other stage of either stores, loads or reduces it.
    let names = |s: &Stage, t: TensorId| {
        matches!(&s.kind, StageKind::Load { tensor, .. } | StageKind::Store { tensor, .. } | StageKind::AllReduce { tensor, .. } if *tensor == t)
    };
    // The consumer's one load of it, into a buffer only computes read.
    let loads: Vec<&Stage> = second
        .stages
        .iter()
        .filter(|s| matches!(&s.kind, StageKind::Load { tensor: x, .. } if *x == t2.id))
        .collect();
    let [load] = loads.as_slice() else { return None };
    if first.stages.iter().any(|s| s.id != store.id && names(s, t1.id)) || second.stages.iter().any(|s| s.id != load.id && names(s, t2.id)) {
        return None;
    }
    let StageKind::Load { pattern: lp, num_blocks, multicast, .. } = &load.kind else { return None };
    if *num_blocks != 1 || multicast.is_some() || load.outs.len() != 1 || first.work.items.is_empty() {
        return None;
    }
    // The same tiles in the same order for every item the two sections walk.
    for item in &first.work.items {
        let stored = sp.tiles_at(item.coord, *sub_blocks)?;
        let loaded = lp.tiles_at(item.coord, (1, 1))?;
        if stored != loaded {
            return None;
        }
    }
    // A store that steps with its item or its sub-block packs, under the plan `tile` gives the joined section, a block of
    // the producer's tiles an item, where the consumer's compute was laid out a tile an item: two templates join through
    // one only where each walks one item — a tile, planned as one item packed once — and a wider pair is left to the
    // planned join, which compares the planned items.
    let one_block = !sp.origin.steps.is_empty();
    if one_block && !tiled && first.work.items.len() != 1 {
        return None;
    }
    let i = load.outs[0];
    let readers: Vec<&Stage> = second.stages.iter().filter(|s| s.ins.contains(&i)).collect();
    if readers.is_empty() || !readers.iter().all(|s| matches!(s.kind, StageKind::Compute(_))) {
        return None;
    }
    let (bo, bi) = (first.buffer(o), second.buffer(i));
    if bo.kind != bi.kind || bo.dtype != bi.dtype {
        return None;
    }

    // Tensors: the producer's but the forwarded one, then the consumer's not already there. A tensor both read is one
    // tensor, and must be declared alike.
    let mut tensors: Vec<TensorDecl> = Vec::new();
    let mut tmap1 = std::collections::BTreeMap::new();
    for t in first.tensors.iter().filter(|t| t.id != t1.id) {
        tmap1.insert(t.id, TensorId::new(tensors.len()));
        tensors.push(TensorDecl { id: TensorId::new(tensors.len()), ..t.clone() });
    }
    let mut tmap2 = std::collections::BTreeMap::new();
    for t in second.tensors.iter().filter(|t| t.id != t2.id) {
        match tensors.iter().find(|x| x.name == t.name) {
            Some(x) => {
                if (TensorDecl { id: x.id, ..t.clone() }) != *x {
                    return None;
                }
                tmap2.insert(t.id, x.id);
            }
            None => {
                tmap2.insert(t.id, TensorId::new(tensors.len()));
                tensors.push(TensorDecl { id: TensorId::new(tensors.len()), ..t.clone() });
            }
        }
    }

    // Buffers: every one of the producer's, then the consumer's but its load's, which the producer's store buffer is.
    let mut buffers: Vec<Buffer> = first.buffers.clone();
    let mut bmap2 = std::collections::BTreeMap::new();
    bmap2.insert(i, o);
    for b in second.buffers.iter().filter(|b| b.id != i) {
        // A name is a buffer's identity in every printer: the consumer's own `out` beside the producer's takes a suffix.
        let mut name = b.name.clone();
        let mut k = 1;
        while buffers.iter().any(|x| x.name == name) {
            name = format!("{}_f{k}", b.name);
            k += 1;
        }
        bmap2.insert(b.id, BufferId::new(buffers.len()));
        buffers.push(Buffer { id: BufferId::new(buffers.len()), name, ..b.clone() });
    }

    // Stages: the producer's but its store, then the consumer's but its load.
    let mut stages: Vec<Stage> = Vec::new();
    let mut smap1 = std::collections::BTreeMap::new();
    for s in first.stages.iter().filter(|s| s.id != store.id) {
        let id = StageId::new(stages.len());
        smap1.insert(s.id, id);
        let mut kind = s.kind.clone();
        retensor(&mut kind, &tmap1);
        stages.push(Stage { id, kind, ..s.clone() });
    }
    let mut smap2 = std::collections::BTreeMap::new();
    for s in second.stages.iter().filter(|s| s.id != load.id) {
        let id = StageId::new(stages.len());
        smap2.insert(s.id, id);
        let mut kind = s.kind.clone();
        retensor(&mut kind, &tmap2);
        kind.remap_buffers(&|b| bmap2[&b]);
        stages.push(Stage {
            id,
            kind,
            ins: s.ins.iter().map(|b| bmap2[b]).collect(),
            outs: s.outs.iter().map(|b| bmap2[b]).collect(),
            ..s.clone()
        });
    }

    // Edges: the store buffer's edge now reaches the consumer's readers; the load's edge is gone.
    let mut edges: Vec<Edge> = Vec::new();
    for e in &first.edges {
        let to: Vec<StageId> = if e.buffer == o {
            readers.iter().map(|s| smap2[&s.id]).collect()
        } else {
            e.to.iter().map(|s| smap1[s]).collect()
        };
        edges.push(Edge { id: EdgeId::new(edges.len()), from: smap1[&e.from], to, ..e.clone() });
    }
    for e in second.edges.iter().filter(|e| e.buffer != i) {
        edges.push(Edge {
            id: EdgeId::new(edges.len()),
            from: smap2[&e.from],
            buffer: bmap2[&e.buffer],
            to: e.to.iter().map(|s| smap2[s]).collect(),
            ..e.clone()
        });
    }

    let claim = match (&first.claim, &second.claim) {
        (NumericsClaim::Exact, c) | (c, NumericsClaim::Exact) => c.clone(),
        (c, _) => c.clone(),
    };
    let mut requires = first.requires.clone();
    requires.handshakes.extend(second.requires.handshakes.iter().copied());
    requires.interconnect.extend(second.requires.interconnect.iter().copied());
    let program = SpatialProgram {
        name: format!("{}+{}", first.name, second.name),
        tensors,
        stages,
        buffers,
        edges,
        claim,
        requires,
        acc_tiles_used: first.acc_tiles_used.max(second.acc_tiles_used),
        ..first.clone()
    };
    // A planned join is checked against the plan it has: one item, packed once, or nothing.
    if tiled && one_block && !self::one_block(&program) {
        return None;
    }
    Some(Joint { program, carrier: o })
}

/// Point a stage kind's tensor at its id in the joined program.
fn retensor(kind: &mut StageKind, map: &std::collections::BTreeMap<TensorId, TensorId>) {
    match kind {
        StageKind::Load { tensor, .. } | StageKind::Store { tensor, .. } | StageKind::AllReduce { tensor, .. } => *tensor = map[tensor],
        StageKind::Recv { .. } | StageKind::Compute(_) | StageKind::Scalar(_) => {}
    }
}
