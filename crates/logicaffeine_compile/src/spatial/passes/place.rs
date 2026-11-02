//! `place` — pin every work item, stage and buffer to a site and an engine.
//!
//! Blocked plans map output block `(oy, ox)` to site `(x = ox, y = oy)` — the alignment that lets a row of
//! sites share its A blocks and a column its B blocks (a sequential `n % grid_x` mapping would break reader
//! fusion). Streaming plans reproduce tt-metal's `split_work_to_cores` exactly (column-major core-range
//! selection, x-fastest offsets inside each range), so the streamed program is the vendor's cycle-for-cycle.
//! After placement stages and buffers are explicit PER-SITE INSTANCES (`origin` remembers the template), and
//! every engine slot carries the NOC the target's rule assigns.

use super::SpatialReject;
use crate::spatial::cost::is_streaming;
use crate::spatial::ir::*;
use crate::spatial::target::{Interconnect, MeshShape, SpatialTarget};

/// tt-metal's `split_work_to_cores(grid, units, row_wise = false)` + the host's per-range iteration:
/// `(site, first unit, unit count)` in assignment order.
pub fn split_work_to_sites(mesh: MeshShape, units: usize) -> Vec<(Site, usize, usize)> {
    let (nx, ny) = (mesh.x, mesh.y);
    let max_cores = nx * ny;
    if units == 0 || max_cores == 0 {
        return Vec::new();
    }
    let target = units.min(max_cores);
    // Column-major enumeration of the first `n` cores from `start`, as num_cores_to_corerangeset (row_wise=false):
    // a partial column to finish the start column, then full columns, then a partial column.
    fn ranges_from(start: (usize, usize), n: usize, nx: usize, ny: usize) -> Vec<Rect> {
        let mut out = Vec::new();
        let (mut sx, mut sy) = start;
        let mut left = n;
        if sy != 0 && left > ny - sy {
            out.push(Rect {
                x0: sx as u16,
                y0: sy as u16,
                x1: sx as u16,
                y1: (ny - 1) as u16,
            });
            left -= ny - sy;
            sx += 1;
            sy = 0;
        }
        if left > ny {
            let full = left / ny;
            out.push(Rect {
                x0: sx as u16,
                y0: sy as u16,
                x1: (sx + full - 1) as u16,
                y1: (ny - 1) as u16,
            });
            left -= full * ny;
            sx += full;
            sy = 0;
        }
        if left > 0 {
            out.push(Rect {
                x0: sx as u16,
                y0: sy as u16,
                x1: sx as u16,
                y1: (sy + left - 1) as u16,
            });
        }
        let _ = nx;
        out
    }
    let (g1, g2, u1, u2) = if units % target == 0 {
        (
            ranges_from((0, 0), target, nx, ny),
            Vec::new(),
            units / target,
            0,
        )
    } else {
        let more = units % target;
        let g1 = ranges_from((0, 0), more, nx, ny);
        let last = g1.last().unwrap();
        let start2 = if (last.y1 as usize) != ny - 1 {
            (last.x1 as usize, last.y1 as usize + 1)
        } else {
            (last.x1 as usize + 1, 0)
        };
        (
            g1,
            ranges_from(start2, target - more, nx, ny),
            units / target + 1,
            units / target,
        )
    };
    let mut out = Vec::new();
    let mut offset = 0;
    for (ranges, per) in [(g1, u1), (g2, u2)] {
        for r in ranges {
            for site in r.sites() {
                out.push((site, offset, per));
                offset += per;
            }
        }
    }
    out
}

/// The `place` pass.
pub fn place(p: &SpatialProgram, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    if p.placement.is_some() {
        return Ok(p.clone());
    }
    let Some(plan) = p.work.plan else {
        return Err(SpatialReject::WrongPhase {
            pass: "place",
            expected: "Tiled",
            found: p.phase(),
        });
    };
    let has_movement = p
        .stages
        .iter()
        .any(|s| matches!(s.kind, StageKind::Load { .. } | StageKind::Store { .. }));
    if has_movement && target.site.movement_engines == 0 {
        return Err(SpatialReject::TooManyEngines {
            site: Site::new(0, 0),
            needed: 1,
            available: 0,
        });
    }
    // Two engines run concurrently, so the plan's packing decides what overlaps what. One engine: everything
    // on it, loads then stores per item.
    let two = target.site.movement_engines >= 2;
    let (load_engine, store_engine) = if two { (1u8, 0u8) } else { (0u8, 0u8) };
    // The second operand's loader, and the store, share an engine under `OperandWithStore`.
    let split = two && plan.engines == EnginePacking::OperandWithStore;
    // The operand a Load stage carries: the tensors are declared A, B, …, so the first input tensor is A.
    let operand_of = |kind: &StageKind| -> Option<usize> {
        match kind {
            StageKind::Load { tensor, .. } => p
                .tensors
                .iter()
                .filter(|t| t.role != TensorRole::Output)
                .position(|t| t.id == *tensor),
            _ => None,
        }
    };
    // Items → sites.
    let mut items_of_site: Vec<(Site, Vec<WorkItemId>)> = Vec::new();
    // ★ WHERE SITE ADJACENCY MEANS NOTHING, SPREAD THE ITEMS LINEARLY.
    //
    // The 2-D `(col % sites.x, row % sites.y)` map exists because a Tensix mesh is PHYSICAL: a row
    // of sites shares a multicast panel, so which site an item lands on decides what it can be
    // handed rather than fetch. A target with no multicast has no such structure — a site is just
    // an index — and the 2-D map then actively hurts: a 16×16 block grid on a 112×1 mesh collapses
    // onto **16** sites, because every item in a column takes `col % 112 = col`. Sixteen CUDA
    // blocks on a 28-SM GPU leaves most of the machine idle.
    //
    // Linear placement is the same `split_work_to_cores` transcription the streaming family
    // already uses, and it fills every site it is given.
    let flat_sites = !target.provides_interconnect(Interconnect::MulticastRect);
    if flat_sites
        || is_streaming(&plan)
        || p.stages
            .iter()
            .all(|s| !matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. })))
    {
        for (site, start, count) in split_work_to_sites(target.sites, p.work.items.len()) {
            let ids: Vec<WorkItemId> = (start..start + count).map(WorkItemId::new).collect();
            items_of_site.push((site, ids));
        }
    } else {
        let mut by_site: std::collections::BTreeMap<Site, Vec<WorkItemId>> = Default::default();
        for it in &p.work.items {
            let site = Site::new(
                it.coord.1 % target.sites.x.max(1),
                it.coord.0 % target.sites.y.max(1),
            );
            by_site.entry(site).or_default().push(it.id);
        }
        items_of_site = by_site.into_iter().collect();
    }
    // Deterministic site order: y outer, x inner.
    items_of_site.sort_by_key(|(s, _)| (s.y, s.x));
    let rule = target.noc_rule;
    let result = p.output_tensor().map(|t| t.id);
    // A result stored in parts — each store its own part of the item's block — keeps every store's shape.
    let result_in_parts = p
        .stages
        .iter()
        .filter(|s| matches!(&s.kind, StageKind::Store { tensor, .. } if Some(*tensor) == result))
        .count()
        > 1;
    let out_tiles = p
        .output_tensor()
        .map(|t| (t.tiles.rows_t, t.tiles.cols_t))
        .unwrap_or((0, 0));
    let mut stages = Vec::new();
    let mut buffers = Vec::new();
    let mut edges = Vec::new();
    let mut site_of = Vec::new();
    let mut engine_of = Vec::new();
    // Which buffer each operand and the matmul's output live in, so a site's PAGES can be sized from the
    // block that site actually covers.
    let (mut a_buf, mut b_buf, mut out_buf, mut spill_buf) = (None, None, None, None);
    for st in &p.stages {
        match &st.kind {
            StageKind::Load { .. } => match operand_of(&st.kind) {
                Some(0) => a_buf = st.outs.first().copied(),
                Some(1) => b_buf = st.outs.first().copied(),
                _ => {}
            },
            StageKind::Compute(ComputeOp::MatmulTiles { interm, .. }) => {
                out_buf = st.outs.first().copied();
                spill_buf = *interm;
            }
            _ => {}
        }
    }
    // The output shares its pages with the spill where the reduction finishes in place (`tile::finishes_in_place`).
    let out_shared =
        spill_buf.is_some_and(|s| out_buf.is_some() && p.buffer(s).alias_of == out_buf);
    // ★ AND THE PANEL RESIZE BELONGS TO THE MATMUL THAT DEFINES A PANEL.
    //
    // The operand pages below are re-sized to `h · ibw` and `ibw · w` — the block of A and of B one
    // site's K-block consumes. `ibw` is the INNER-BLOCK WIDTH of a matmul plan and means nothing
    // without one: a fused row op still gets a plan (its grid, its per-site block) but `ibw` is 1, so
    // the formula silently shrank a `cols_t`-tile row operand to a SINGLE tile while every access map
    // reading it still stepped a whole row. Nothing downstream could see the row was now narrower than
    // its own stride — the Tensix printer refused it by name ("an offset, a stride or an indirect term
    // the broadcast family cannot address"), and a printer that could not name it would have read past
    // the page instead. A program with no matmul has no panel, so it keeps the pages it declared.
    if out_buf.is_none() {
        a_buf = None;
        b_buf = None;
    }
    for (site, items) in &items_of_site {
        let sname = |n: &str| format!("{n}@{},{}", site.x, site.y);
        let buf_base = buffers.len();
        // The grid position this site's work sits at — only defined when the site holds exactly one item,
        // which is the case for every mesh-FITTING plan. A wrapped plan's site holds several blocks and
        // its patterns stay nominal, which is correct because wrapped grids are divisor-derived.
        //
        // ★ AND IT COMES FROM THE ITEM'S RECT, BECAUSE THAT IS WHAT SAYS WHAT THE ITEM COVERS.
        //
        // `site_block(coord)` re-derives the extent from the item's GRID POSITION, which is only the same
        // answer while `coord` IS a grid position. A section whose items carry a LOGICAL coordinate — an
        // attention step indexes its work by `(rep, kv-head)` so its loads can reach `k` without an
        // integer division, a gated-delta step by `(head, a-block)` — then has its block computed from
        // the wrong axis. `site_block` reads the second index as a column block and returns WIDTH ZERO
        // past the first, so `place` emits a degenerate clone and those items silently write NOTHING:
        // measured as 384 of 512 attention outputs coming back as zero, with exactly the `rep == 0` heads
        // correct.
        //
        // `WorkItem::out` is not a second opinion about the extent — it IS the extent, and `site_block`
        // is one way of computing it from the plan. Taking the rect directly agrees wherever the two
        // agree (every mesh-fitting single-op plan, where `coord` is the grid position) and is right
        // where they do not.
        let site_block = if items.len() == 1 {
            let it = &p.work.items[items[0].idx()];
            p.work.plan.map(|pl| {
                let (h, w) = (it.out.h_t.max(1), it.out.w_t.max(1));
                let fit = |nominal: usize, extent: usize| {
                    (1..=nominal.min(extent))
                        .rev()
                        .find(|d| extent % d == 0)
                        .unwrap_or(1)
                };
                (h, w, fit(pl.block.osh, h), fit(pl.block.osw, w))
            })
        } else {
            None
        };
        for b in &p.buffers {
            // ★ AND THE PAGE IS SIZED FROM THE BLOCK THIS SITE COVERS.
            //
            // A boundary site's page was still the NOMINAL block, so its multicast sent a full panel where
            // a shorter one exists — 7 rows of 3 tiles where the last holds 2, which is 420 tiles moved
            // for 400 that exist. The closed form (correctly) predicted 400 and the choreography did 420,
            // and the 20-tile gap per axis is exactly the 40 tiles `the_closed_form_agrees_with_the_
            // choreography_it_summarizes` reported. Sending bytes that are not there is not a modelling
            // gap; it is the emitter reading past the operand.
            let kind = match (site_block, p.work.plan) {
                (Some((h, w, osh, osw)), Some(pl)) => match b.kind {
                    BufferKind::Fifo { depth_pages, .. } if Some(b.id) == a_buf => {
                        BufferKind::Fifo {
                            depth_pages,
                            page_tiles: h * pl.block.ibw,
                        }
                    }
                    BufferKind::Fifo { depth_pages, .. } if Some(b.id) == b_buf => {
                        BufferKind::Fifo {
                            depth_pages,
                            page_tiles: pl.block.ibw * w,
                        }
                    }
                    // A spill accumulated in place, and an output it shares, hold one page per sub-block THIS site
                    // walks: the packer's write pointer has to come back to the first page every K block, and the
                    // two views have to meet page for page when the last block packs.
                    BufferKind::Fifo { .. } if Some(b.id) == out_buf && out_shared => {
                        BufferKind::Fifo {
                            depth_pages: (h / osh.max(1)) * (w / osw.max(1)),
                            page_tiles: osh * osw,
                        }
                    }
                    BufferKind::Fifo {
                        depth_pages,
                        page_tiles,
                    } if Some(b.id) == out_buf => BufferKind::Fifo {
                        depth_pages,
                        page_tiles: page_tiles.min(osh * osw).max(1),
                    },
                    BufferKind::Interm { .. } if Some(b.id) == spill_buf => BufferKind::Interm {
                        depth_pages: (h / osh.max(1)) * (w / osw.max(1)),
                        page_tiles: osh * osw,
                    },
                    other => other,
                },
                _ => b.kind,
            };
            buffers.push(Buffer {
                id: BufferId::new(buffers.len()),
                name: sname(&b.name),
                kind,
                dtype: b.dtype,
                space: b.space,
                site: Some(*site),
                origin: Some(b.origin.unwrap_or(b.id)),
                alias_of: b.alias_of.map(|a| BufferId::new(buf_base + a.idx())),
            });
        }
        let stage_base = stages.len();
        for s in &p.stages {
            let remap = |b: &BufferId| BufferId::new(buf_base + b.idx());
            let mut kind = s.kind.clone();
            // ★ THIS SITE'S PATTERNS DESCRIBE THIS SITE'S WORK.
            //
            // Every stage is cloned per site here, so a boundary site can carry an extent narrower than
            // the plan's nominal block — which is what a mesh whose dimensions do not divide the tile
            // count requires. `BlockPlan::site_block` is the one definition of what a site covers, and
            // `work_of_plan` PRICES from the same function, so the planner cannot rank on a block the
            // emitter does not lay out. On a dividing grid every site gets the nominal block and all of
            // this is the identity.
            if let Some((h, w, osh, osw)) = site_block {
                // ★ AN OPERAND PANEL EXISTS ONLY WHERE A MATMUL DEFINES ONE. A's rows and B's columns follow
                // the output block; in a program with no matmul a load's block has nothing to do with the
                // item's rect, and narrowing "the first two inputs" to it silently cut a gathered K block, an
                // expert's weight chunk or a partial tree's fan-in down to one tile.
                if let (Some(_), StageKind::Load { pattern, .. }) = (out_buf, &mut kind) {
                    match operand_of(&s.kind) {
                        Some(0) => pattern.block_h = pattern.block_h.min(h),
                        Some(1) => pattern.block_w = pattern.block_w.min(w),
                        _ => {}
                    }
                }
                match &mut kind {
                    // ★ THE ITEM'S BLOCK IS A BLOCK OF THE RESULT, so only the result's store takes its shape. A
                    // section writing a second tensor of another shape — a split softmax's statistics beside its
                    // values — addresses it by its own pattern, which the item's rectangle does not describe; so does
                    // each store of a result stored in parts.
                    StageKind::Store {
                        tensor,
                        pattern,
                        sub_blocks,
                    } if Some(*tensor) == result && !result_in_parts => {
                        // ★ THE ORIGIN'S STRIDES MOVE WITH THE SUB-BLOCK. Narrowing the extent alone left
                        // `LV_SUB_H` striding by the NOMINAL sub-block height, so a boundary site stepped
                        // over rows it does not own and wrote past the tensor —
                        // `ReadOutOfRange { tensor: 2, tile: 1024 }` on a 1024-tile output. An extent and
                        // the stride that walks it are one fact, and must change together.
                        for (v, step) in pattern.origin.steps.iter_mut() {
                            if *v == super::LV_SUB_H {
                                *step = osh * out_tiles.1;
                            } else if *v == super::LV_SUB_W {
                                *step = osw;
                            }
                        }
                        pattern.block_h = osh;
                        pattern.block_w = osw;
                        *sub_blocks = (h / osh.max(1), w / osw.max(1));
                    }
                    StageKind::Compute(ComputeOp::MatmulTiles {
                        block, sub_blocks, ..
                    }) => {
                        block.osh = osh;
                        block.osw = osw;
                        *sub_blocks = (h / osh.max(1), w / osw.max(1));
                    }
                    _ => {}
                }
            }
            kind.remap_buffers(&|b| remap(&b));
            let (engine, noc) = match s.kind.role() {
                SiteRole::Compute => (Engine::Compute, None),
                SiteRole::Movement => {
                    let e = match &s.kind {
                        // Under the split packing the second operand's loader joins the store's engine.
                        StageKind::Store { .. } => store_engine,
                        _ if split && operand_of(&s.kind) == Some(1) => store_engine,
                        _ => load_engine,
                    };
                    (
                        Engine::Movement(e),
                        rule.map(|r| {
                            NocId(if e == load_engine && two {
                                r.receiver_noc
                            } else {
                                r.writer_noc_default
                            })
                        }),
                    )
                }
            };
            stages.push(Stage {
                id: StageId::new(stages.len()),
                name: sname(&s.name),
                kind,
                role: s.role,
                ins: s.ins.iter().map(remap).collect(),
                outs: s.outs.iter().map(remap).collect(),
                site: Some(*site),
                item: if items.len() == 1 {
                    Some(items[0])
                } else {
                    None
                },
                origin: Some(s.origin.unwrap_or(s.id)),
            });
            site_of.push(*site);
            engine_of.push(EngineSlot {
                site: *site,
                engine,
                noc,
            });
        }
        for e in &p.edges {
            edges.push(Edge {
                id: EdgeId::new(edges.len()),
                from: StageId::new(stage_base + e.from.idx()),
                buffer: BufferId::new(buf_base + e.buffer.idx()),
                to: e
                    .to
                    .iter()
                    .map(|t| StageId::new(stage_base + t.idx()))
                    .collect(),
                handshake: e.handshake.clone(),
            });
        }
    }
    let mut q = p.clone();
    q.stages = stages;
    q.buffers = buffers;
    q.edges = edges;
    q.placement = Some(Placement {
        mesh: target.sites,
        site_of,
        engine_of,
        groups: Vec::new(),
        items_of_site,
    });
    q.validate()?;
    Ok(q)
}
