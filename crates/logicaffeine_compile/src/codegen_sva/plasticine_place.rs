//! **Mesh placement + static routing** for a Plasticine configuration — P3 of the Plasticine lane.
//!
//! Given a [`Metapipeline`] (stages with their datapath depth and cell count) and the words each inter-stage
//! buffer must hold, this allocates PCUs and PMUs, places them on the checkerboard, routes every edge with
//! dimension-order (X then Y) static routing, and reports what an RDU engineer asks for: PCU/PMU used vs
//! total, FU utilization, hop counts, per-link occupancy against the switch's link budget, and whether the
//! static routing fit. Every stage's datapath is still the SAT-certified placement of
//! [`FabricTarget::plasticine_shaped`](crate::codegen_sva::fabric_target::FabricTarget::plasticine_shaped) (P0) — this module only decides WHERE on the mesh and HOW MANY.
//!
//! Model (ISCA'17 §3.1–3.3, §3.6 "hierarchical binding"; Spade defaults for link counts): a scalar body of
//! critical depth `D` and `C` cells chains `ceil(max(D, C) / pcu_stages)` PCUs (a lane owns one FU per
//! stage: depth needs stages, cells need slots), each PCU running `pcu_lanes` iterations in parallel; a buffer written in stage `i` and read in stage `j` is `j-i+1`-deep N-buffered inside one PMU
//! when it fits `words_per_pmu / depth`, else spans `ceil(words / (words_per_pmu / depth))` PMUs; the
//! checkerboard alternates PCU/PMU cells; routes follow X-Y order and each DIRECTED mesh link carries at most
//! `vector_links` vector routes; a stage that takes no PCU (a perimeter address generator) enters the mesh at
//! the edge switch of its neighbour's row, one extra hop. MODELED; never a Plasticine simulation.
//!
//! [`FabricTarget::plasticine_shaped`]: crate::codegen_sva::fabric_target::FabricTarget::plasticine_shaped

use super::fabric_target::{Capability, Unplaceable};
use super::metapipeline::Metapipeline;
use super::plasticine::PlasticineTarget;

/// Switch/link budget per mesh segment (Spade `SpadeConfig` defaults: `vlink 2`, `slink 4`, `vc 4`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct LinkBudget {
    pub vector_links: usize,
    pub scalar_links: usize,
}

impl LinkBudget {
    pub fn spade_default() -> LinkBudget {
        LinkBudget {
            vector_links: 2,
            scalar_links: 4,
        }
    }
}

/// A net endpoint on the mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// A unit on cell `(row, col)`.
    Cell(usize, usize),
    /// The DRAM side (address generators behind the perimeter switches).
    Dram,
}

/// One vector net to route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshNet {
    pub from: Endpoint,
    pub to: Endpoint,
}

/// A mesh link `((r,c),(r',c'))`; a link whose ends coincide is a perimeter port of that edge cell.
pub type Link = ((usize, usize), (usize, usize));

/// The vector nets of one placement, routed. For every net, in order: the mesh links it occupies, and its
/// occupancy keys — `(link, 0)` for a channel of a directed link, `((cell, cell), 1)` for a perimeter port
/// carrying DRAM → mesh and `((cell, cell), 2)` for mesh → DRAM — so that the largest count over keys of a
/// legal routing never exceeds the channel budget.
pub struct MeshRouting {
    pub links: Vec<Vec<Link>>,
    pub occupancy: Vec<Vec<(Link, u8)>>,
    /// Every net connected and no resource shared, by a check independent of the router.
    pub legal: bool,
    /// Which router (and rung) produced the routes, as a [`Placement`] reports it.
    pub router: &'static str,
}

/// A router for the mesh's vector nets. [`place_unrolled`] places with row-balanced cells and routes through
/// the installed router ([`place_routed`]), or falls back to [`place`]'s xy-static routes when none is.
pub trait MeshRouter: Send + Sync {
    fn route(&self, t: &PlasticineTarget, budget: LinkBudget, nets: &[MeshNet]) -> MeshRouting;
}

static MESH_ROUTER: std::sync::OnceLock<&'static dyn MeshRouter> = std::sync::OnceLock::new();

/// Install this process's mesh router, once; `Err` hands back the argument when one is already installed.
/// Certified place-and-route installs its congestion-negotiating router this way, from its own side: the
/// compiler never depends on it, so the published tree builds without it.
pub fn install_mesh_router(router: &'static dyn MeshRouter) -> Result<(), &'static dyn MeshRouter> {
    MESH_ROUTER.set(router)
}

/// The installed mesh router, if any.
pub fn mesh_router() -> Option<&'static dyn MeshRouter> {
    MESH_ROUTER.get().copied()
}

/// PCUs one stage needs.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct StageAllocation {
    pub stage: usize,
    pub pcus: usize,
    /// FU stage-slots used in the LAST PCU of the chain (the others are full).
    pub stages_in_last: usize,
    /// Cells the stage places (each cell occupies one FU stage across `lanes` lanes).
    pub cells: usize,
}

/// PMUs one buffer edge needs.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct BufferAllocation {
    pub edge: (usize, usize),
    /// N-buffer depth (Spatial's writer→reader rule).
    pub depth: usize,
    pub words: usize,
    pub pmus: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Allocation {
    pub stages: Vec<StageAllocation>,
    pub buffers: Vec<BufferAllocation>,
}

impl Allocation {
    pub fn pcus(&self) -> usize {
        self.stages.iter().map(|s| s.pcus).sum()
    }
    pub fn pmus(&self) -> usize {
        self.buffers.iter().map(|b| b.pmus).sum()
    }
}

/// 32-bit words one PMU holds (ISCA'17: 16 banks × 16 KB = 256 KB = 65,536 words).
pub fn words_per_pmu(t: &PlasticineTarget) -> usize {
    t.pmu_kib * 1024 / 4
}

/// PMUs a `depth`-deep N-buffer of `words` needs on `t`: a PMU's address space is partitioned `depth` ways
/// (Spatial's N-buffering), so one PMU holds `words_per_pmu / depth` words per buffer; an empty tile still
/// occupies one PMU (the buffer exists).
pub fn pmus_for(words: usize, depth: usize, t: &PlasticineTarget) -> usize {
    let per_buffer = (words_per_pmu(t) / depth.max(1)).max(1);
    if words == 0 {
        1
    } else {
        (words + per_buffer - 1) / per_buffer
    }
}

/// Allocate PCUs/PMUs for `mp` on `t`; `buffer_words[i]` is the tile size flowing over `mp.edges[i]`. Every
/// stage is hosted on PCUs.
pub fn allocate(
    mp: &Metapipeline,
    buffer_words: &[usize],
    t: &PlasticineTarget,
) -> Result<Allocation, Unplaceable> {
    allocate_hosted(mp, buffer_words, t, &[])
}

/// [`allocate`] with the stages listed in `ag_stages` hosted on the PERIMETER ADDRESS GENERATORS (ISCA'17
/// §3.4: the AGs at the chip's edges drive the DRAM tile loads and stores) rather than on PCUs: such a stage
/// takes no PCU and places no FU stage; [`place`] routes it from the mesh edge.
pub fn allocate_hosted(
    mp: &Metapipeline,
    buffer_words: &[usize],
    t: &PlasticineTarget,
    ag_stages: &[usize],
) -> Result<Allocation, Unplaceable> {
    assert_eq!(buffer_words.len(), mp.edges.len(), "one tile size per edge");
    mp.fits(t)?;
    let per_pcu = t.pcu_stages.max(1);
    let stages = mp
        .stages
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if ag_stages.contains(&i) {
                return StageAllocation {
                    stage: i,
                    pcus: 0,
                    stages_in_last: 0,
                    cells: 0,
                };
            }
            // A lane owns one FU per PCU stage, so a scalar body needs an FU SLOT per cell as well as a stage per
            // level of depth: the chain is sized by whichever is larger (a wide, shallow body still fills slots).
            let slots = s.latency.max(s.cells).max(1);
            let pcus = (slots + per_pcu - 1) / per_pcu;
            StageAllocation {
                stage: i,
                pcus,
                stages_in_last: slots - (pcus - 1) * per_pcu,
                cells: s.cells,
            }
        })
        .collect();
    let mut buffers = Vec::new();
    for (k, &(w, r)) in mp.edges.iter().enumerate() {
        let depth = Metapipeline::buffer_depth(w, r);
        let words = buffer_words[k];
        buffers.push(BufferAllocation {
            edge: (w, r),
            depth,
            words,
            pmus: pmus_for(words, depth, t),
        });
    }
    let alloc = Allocation { stages, buffers };
    if alloc.pcus() > t.pcus() || alloc.pmus() > t.pmus() {
        return Err(Unplaceable::TooSmall {
            needed_rows: alloc.pcus(),
            needed_cols: alloc.pmus(),
            target_rows: t.pcus(),
            target_cols: t.pmus(),
            target: t.name,
        });
    }
    Ok(alloc)
}

/// A placed unit on the checkerboard.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Unit {
    Pcu { stage: usize, index: usize },
    Pmu { edge: (usize, usize), index: usize },
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Placed {
    pub unit: Unit,
    pub row: usize,
    pub col: usize,
}

/// A routed edge: PCU (writer chain tail) → PMU → PCU (reader chain head), with its hop count.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Route {
    pub edge: (usize, usize),
    pub hops: usize,
    /// The mesh links `((r,c),(r',c'))` the route occupies.
    pub links: Vec<((usize, usize), (usize, usize))>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Placement {
    pub target: &'static str,
    pub alloc: Allocation,
    pub placed: Vec<Placed>,
    pub routes: Vec<Route>,
    pub pcu_total: usize,
    pub pmu_total: usize,
    pub budget: LinkBudget,
    /// Most vector routes sharing one DIRECTED mesh link.
    pub max_link_occupancy: usize,
    /// Every link within the budget.
    pub routed_statically: bool,
    /// Which router produced the routes: `xy-static` ([`place`]) or the installed [`MeshRouter`]'s label
    /// ([`place_routed`]).
    pub router: &'static str,
}

impl Placement {
    pub fn pcu_used(&self) -> usize {
        self.alloc.pcus()
    }
    pub fn pmu_used(&self) -> usize {
        self.alloc.pmus()
    }
    pub fn pcu_utilization(&self) -> f64 {
        self.pcu_used() as f64 / self.pcu_total as f64
    }
    pub fn pmu_utilization(&self) -> f64 {
        self.pmu_used() as f64 / self.pmu_total as f64
    }
    /// FU utilization inside the allocated PCUs: cells placed / (PCUs × stages per PCU).
    pub fn fu_utilization(&self, pcu_stages: usize) -> f64 {
        let cells: usize = self.alloc.stages.iter().map(|s| s.cells).sum();
        cells as f64 / (self.pcu_used() * pcu_stages.max(1)) as f64
    }
    pub fn total_hops(&self) -> usize {
        self.routes.iter().map(|r| r.hops).sum()
    }
    pub fn max_hops(&self) -> usize {
        self.routes.iter().map(|r| r.hops).max().unwrap_or(0)
    }
}

fn is_pcu_cell(row: usize, col: usize) -> bool {
    (row + col) % 2 == 0
}

fn manhattan(a: (usize, usize), b: (usize, usize)) -> usize {
    a.0.abs_diff(b.0) + a.1.abs_diff(b.1)
}

/// Dimension-order (X first, then Y) route from `a` to `b` as the list of unit links.
fn xy_route(a: (usize, usize), b: (usize, usize)) -> Vec<((usize, usize), (usize, usize))> {
    let mut links = Vec::new();
    let (mut r, mut c) = a;
    while c != b.1 {
        let nc = if b.1 > c { c + 1 } else { c - 1 };
        links.push(((r, c), (r, nc)));
        c = nc;
    }
    while r != b.0 {
        let nr = if b.0 > r { r + 1 } else { r - 1 };
        links.push(((r, c), (nr, c)));
        r = nr;
    }
    links
}

/// Place the allocation on the checkerboard in pipeline order (a row-major snake over PCU cells, each
/// stage's chain contiguous, each edge's PMU(s) taken from the PMU cells nearest the writer's tail), then
/// route every edge X-Y and count link occupancy against the budget.
pub fn place(
    alloc: &Allocation,
    t: &PlasticineTarget,
    budget: LinkBudget,
) -> Result<Placement, Unplaceable> {
    let (rows, cols) = (t.pcu_rows, t.pcu_cols);
    let mut pcu_cells: Vec<(usize, usize)> = Vec::new();
    let mut pmu_cells: Vec<(usize, usize)> = Vec::new();
    for r in 0..rows {
        let cs: Vec<usize> = if r % 2 == 0 {
            (0..cols).collect()
        } else {
            (0..cols).rev().collect()
        };
        for c in cs {
            if is_pcu_cell(r, c) {
                pcu_cells.push((r, c))
            } else {
                pmu_cells.push((r, c))
            }
        }
    }
    if alloc.pcus() > pcu_cells.len() || alloc.pmus() > pmu_cells.len() {
        return Err(Unplaceable::TooSmall {
            needed_rows: alloc.pcus(),
            needed_cols: alloc.pmus(),
            target_rows: pcu_cells.len(),
            target_cols: pmu_cells.len(),
            target: t.name,
        });
    }
    let mut placed = Vec::new();
    let mut next_pcu = 0usize;
    let mut stage_head: Vec<(usize, usize)> = Vec::new();
    let mut stage_tail: Vec<(usize, usize)> = Vec::new();
    // Per stage: the extra hop a perimeter-hosted (0-PCU, address-generator) stage pays to enter the mesh.
    let mut perimeter_hops: Vec<usize> = Vec::new();
    for s in &alloc.stages {
        if s.pcus == 0 {
            // An address-generator stage sits off-mesh at the perimeter and enters at the edge switch of its
            // mesh neighbour's row (the next PCU to be placed, or the last one placed), on the nearer side.
            let neighbour = pcu_cells[next_pcu.min(pcu_cells.len() - 1)];
            let col = if neighbour.1 * 2 < cols { 0 } else { cols - 1 };
            stage_head.push((neighbour.0, col));
            stage_tail.push((neighbour.0, col));
            perimeter_hops.push(1);
            continue;
        }
        stage_head.push(pcu_cells[next_pcu]);
        for i in 0..s.pcus {
            let (r, c) = pcu_cells[next_pcu];
            placed.push(Placed {
                unit: Unit::Pcu {
                    stage: s.stage,
                    index: i,
                },
                row: r,
                col: c,
            });
            next_pcu += 1;
        }
        stage_tail.push(pcu_cells[next_pcu - 1]);
        perimeter_hops.push(0);
    }
    let mut free_pmu: Vec<(usize, usize)> = pmu_cells.clone();
    let mut routes = Vec::new();
    let mut link_use: std::collections::HashMap<((usize, usize), (usize, usize)), usize> =
        std::collections::HashMap::new();
    for b in &alloc.buffers {
        let (w, r) = b.edge;
        let tail = stage_tail[w];
        let head = stage_head[r];
        let mut links = Vec::new();
        let mut hops = 0usize;
        for i in 0..b.pmus {
            // Nearest free PMU cell to the writer's tail.
            let (idx, _) = free_pmu
                .iter()
                .enumerate()
                .min_by_key(|(_, p)| manhattan(**p, tail) + manhattan(**p, head))
                .expect("a free PMU cell");
            let cell = free_pmu.remove(idx);
            placed.push(Placed {
                unit: Unit::Pmu {
                    edge: b.edge,
                    index: i,
                },
                row: cell.0,
                col: cell.1,
            });
            let l1 = xy_route(tail, cell);
            let l2 = xy_route(cell, head);
            hops += l1.len() + l2.len() + perimeter_hops[w] + perimeter_hops[r];
            links.extend(l1);
            links.extend(l2);
        }
        // Links are DIRECTED: a switch port carries `vector_links` channels in each direction (ISCA'17 §3.5:
        // the static network's registered links are unidirectional per port), so a tile flowing into a PMU
        // and the same PMU's read-out on the reverse segment never share a channel.
        for l in &links {
            *link_use.entry(*l).or_insert(0) += 1;
        }
        routes.push(Route {
            edge: b.edge,
            hops,
            links,
        });
    }
    let max_link_occupancy = link_use.values().copied().max().unwrap_or(0);
    Ok(Placement {
        target: t.name,
        alloc: alloc.clone(),
        placed,
        routes,
        pcu_total: pcu_cells.len(),
        pmu_total: pmu_cells.len(),
        budget,
        max_link_occupancy,
        routed_statically: max_link_occupancy <= budget.vector_links,
        router: "xy-static",
    })
}

/// Allocate + place in one call.
pub fn place_metapipeline(
    mp: &Metapipeline,
    buffer_words: &[usize],
    t: &PlasticineTarget,
    budget: LinkBudget,
) -> Result<Placement, Unplaceable> {
    let alloc = allocate(mp, buffer_words, t)?;
    place(&alloc, t, budget)
}

/// Replicate one instance's allocation `par` times — Spatial's outer-loop unrolling by a par factor
/// (ISCA'17 §4: `par` independent copies of the whole metapipeline run concurrent outer iterations). Instance
/// `k`'s stage `s` is reported as stage `k * mp.stages.len() + s`; its buffers are shifted the same way.
fn replicate(one: &Allocation, n_stages: usize, par: usize) -> Allocation {
    let mut stages = Vec::with_capacity(one.stages.len() * par);
    let mut buffers = Vec::with_capacity(one.buffers.len() * par);
    for k in 0..par {
        let off = k * n_stages;
        stages.extend(one.stages.iter().map(|s| StageAllocation {
            stage: off + s.stage,
            ..s.clone()
        }));
        buffers.extend(one.buffers.iter().map(|b| BufferAllocation {
            edge: (off + b.edge.0, off + b.edge.1),
            ..b.clone()
        }));
    }
    Allocation { stages, buffers }
}

/// Allocate `mp` (with `ag_stages` on the perimeter address generators) once, unroll it `par` ways, and
/// place every instance — a typed `TooSmall` when the replicated allocation exceeds the mesh.
pub fn place_unrolled(
    mp: &Metapipeline,
    buffer_words: &[usize],
    ag_stages: &[usize],
    par: usize,
    t: &PlasticineTarget,
    budget: LinkBudget,
) -> Result<Placement, Unplaceable> {
    assert!(par >= 1, "an unroll factor is at least one");
    let one = allocate_hosted(mp, buffer_words, t, ag_stages)?;
    let alloc = replicate(&one, mp.stages.len(), par);
    if alloc.pcus() > t.pcus() || alloc.pmus() > t.pmus() {
        return Err(Unplaceable::TooSmall {
            needed_rows: alloc.pcus(),
            needed_cols: alloc.pmus(),
            target_rows: t.pcus(),
            target_cols: t.pmus(),
            target: t.name,
        });
    }
    // Without an installed router, fall back to the xy-static one. The `router` field then reports
    // "xy-static", so the difference is visible in the Placement instead of silent: a static route can
    // fail to find a legal assignment that a negotiated one would have found.
    match mesh_router() {
        Some(router) => place_routed(&alloc, t, budget, router),
        None => place(&alloc, t, budget),
    }
}

/// Perimeter DRAM ports one instance of `mp` needs, `(into the mesh, out of the mesh)`: every buffer edge
/// written by a perimeter-hosted stage is one inbound stream, every edge read by one is one outbound stream
/// (each PMU of a spanning buffer is its own stream).
pub fn dram_streams(
    mp: &Metapipeline,
    buffer_words: &[usize],
    ag_stages: &[usize],
    t: &PlasticineTarget,
) -> Result<(usize, usize), Unplaceable> {
    let one = allocate_hosted(mp, buffer_words, t, ag_stages)?;
    let mut inbound = 0usize;
    let mut outbound = 0usize;
    for b in &one.buffers {
        if ag_stages.contains(&b.edge.0) {
            inbound += b.pmus;
        }
        if ag_stages.contains(&b.edge.1) {
            outbound += b.pmus;
        }
    }
    Ok((inbound, outbound))
}

/// The widest unroll factor whose replicated allocation fits `t` by CAPACITY (at least 1 when one instance
/// fits) — PCUs, PMUs, and the perimeter DRAM ports (`rows × 2 sides × vector_links` per direction, ISCA'17
/// §3.4–3.5) — before routing is considered.
pub fn max_par(
    mp: &Metapipeline,
    buffer_words: &[usize],
    ag_stages: &[usize],
    t: &PlasticineTarget,
) -> Result<usize, Unplaceable> {
    let one = allocate_hosted(mp, buffer_words, t, ag_stages)?;
    let by_pcu = if one.pcus() == 0 {
        usize::MAX
    } else {
        t.pcus() / one.pcus()
    };
    let by_pmu = if one.pmus() == 0 {
        usize::MAX
    } else {
        t.pmus() / one.pmus()
    };
    let (inbound, outbound) = dram_streams(mp, buffer_words, ag_stages, t)?;
    let ports = t.pcu_rows * 2 * LinkBudget::spade_default().vector_links;
    let by_in = if inbound == 0 {
        usize::MAX
    } else {
        ports / inbound
    };
    let by_out = if outbound == 0 {
        usize::MAX
    } else {
        ports / outbound
    };
    Ok(by_pcu.min(by_pmu).min(by_in).min(by_out).max(1))
}

/// The widest unroll that fits `t` AND routes statically within `budget` — the routability-aware unroll
/// (the static vector network's perimeter ports, not PCU/PMU capacity, bound a DRAM-fed unroll first on a
/// large mesh). Returns `(routable, capacity)`; `routable == 1` is reported even when a single instance
/// exceeds the budget, so a row can still say `routed_statically: false` honestly.
pub fn max_routable_par(
    mp: &Metapipeline,
    buffer_words: &[usize],
    ag_stages: &[usize],
    t: &PlasticineTarget,
    budget: LinkBudget,
) -> Result<(usize, usize), Unplaceable> {
    let capacity = max_par(mp, buffer_words, ag_stages, t)?;
    max_routable_par_within(mp, buffer_words, ag_stages, t, budget, capacity)
}

/// [`max_routable_par`] searching below `upper` (capped at capacity) — for an unroll that the work itself
/// bounds below capacity (fewer outer iterations than instances). The search is a BISECTION over
/// `[1, upper]`: routability is treated as monotone in the unroll (more instances, more perimeter streams), so
/// the result is the widest unroll the bisection finds routable — a stalled negotiation at some intermediate
/// width would hide a wider routable one, which is the honest limit of a heuristic router without the exact
/// rung on the full problem.
pub fn max_routable_par_within(
    mp: &Metapipeline,
    buffer_words: &[usize],
    ag_stages: &[usize],
    t: &PlasticineTarget,
    budget: LinkBudget,
    upper: usize,
) -> Result<(usize, usize), Unplaceable> {
    let capacity = max_par(mp, buffer_words, ag_stages, t)?;
    let hi = upper.min(capacity).max(1);
    let routes = |par: usize| -> Result<bool, Unplaceable> {
        Ok(place_unrolled(mp, buffer_words, ag_stages, par, t, budget)?.routed_statically)
    };
    if routes(hi)? {
        return Ok((hi, capacity));
    }
    // Invariant: `lo` routes (or is 1), `bad` does not.
    let (mut lo, mut bad) = (1usize, hi);
    while bad - lo > 1 {
        let mid = lo + (bad - lo) / 2;
        if routes(mid)? {
            lo = mid
        } else {
            bad = mid
        }
    }
    Ok((lo, capacity))
}

/// The capability a placement demands of the descriptor beyond depth: at least one bank per lane for a
/// conflict-free vector access (Plasticine: 16 banks for 16 lanes).
pub fn lanes_need_banks(t: &PlasticineTarget) -> Result<(), Unplaceable> {
    let cap = Capability::Banks(t.pcu_lanes);
    if t.provides(cap) {
        Ok(())
    } else {
        Err(Unplaceable::MissingCapability {
            needed: cap,
            target: t.name,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────────────
// Row-balanced placement routed by the installed mesh router (the unrolled lane's placer).
// ─────────────────────────────────────────────────────────────────────────────────────────────────────────

/// PCU cells in a ROW-BALANCED, SIDE-ALTERNATING order: one cell per row before a second in any row, and
/// within a row the outermost cells first (left-most, right-most, next-left, next-right, …) so that DRAM-fed
/// instances spread over every row and both perimeter sides instead of filling one row's corridor.
fn balanced_pcu_cells(t: &PlasticineTarget) -> Vec<(usize, usize)> {
    let (rows, cols) = (t.pcu_rows, t.pcu_cols);
    let mut per_row: Vec<Vec<(usize, usize)>> = Vec::with_capacity(rows);
    for r in 0..rows {
        let cells: Vec<(usize, usize)> = (0..cols)
            .filter(|&c| is_pcu_cell(r, c))
            .map(|c| (r, c))
            .collect();
        let mut order = Vec::with_capacity(cells.len());
        let (mut lo, mut hi) = (0usize, cells.len());
        while lo < hi {
            order.push(cells[lo]);
            lo += 1;
            if lo < hi {
                hi -= 1;
                order.push(cells[hi]);
            }
        }
        per_row.push(order);
    }
    let width = per_row.iter().map(|v| v.len()).max().unwrap_or(0);
    let mut out = Vec::with_capacity(rows * width);
    for k in 0..width {
        for row in &per_row {
            if let Some(&c) = row.get(k) {
                out.push(c);
            }
        }
    }
    out
}

/// Place `alloc` on the checkerboard with row-balanced PCU cells and each edge's PMU(s) on the free PMU
/// cells nearest its writer and reader (a perimeter-hosted end pulls the PMU toward the nearer side), then
/// route every vector net — writer tail → PMU → reader head per buffer, DRAM ↔ PMU for perimeter-hosted
/// stages, and the PCU→PCU links inside every chain (reported as a route on the self-edge
/// `(stage, stage)`) — through `router`, whose independent legality verdict is `routed_statically`.
/// `max_link_occupancy` counts the channels one directed link — or one direction of a perimeter port —
/// carries (never above the budget when legal).
pub fn place_routed(
    alloc: &Allocation,
    t: &PlasticineTarget,
    budget: LinkBudget,
    router: &dyn MeshRouter,
) -> Result<Placement, Unplaceable> {
    let (rows, cols) = (t.pcu_rows, t.pcu_cols);
    let pcu_cells = balanced_pcu_cells(t);
    let mut pmu_cells: Vec<(usize, usize)> = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            if !is_pcu_cell(r, c) {
                pmu_cells.push((r, c));
            }
        }
    }
    if alloc.pcus() > pcu_cells.len() || alloc.pmus() > pmu_cells.len() {
        return Err(Unplaceable::TooSmall {
            needed_rows: alloc.pcus(),
            needed_cols: alloc.pmus(),
            target_rows: pcu_cells.len(),
            target_cols: pmu_cells.len(),
            target: t.name,
        });
    }
    let mut placed = Vec::new();
    let mut next_pcu = 0usize;
    // Per stage: the chain's head/tail endpoint (DRAM for a perimeter-hosted stage) and its PCU cells.
    let mut head: Vec<Endpoint> = Vec::new();
    let mut tail: Vec<Endpoint> = Vec::new();
    let mut chains: Vec<Vec<(usize, usize)>> = Vec::new();
    for s in &alloc.stages {
        if s.pcus == 0 {
            head.push(Endpoint::Dram);
            tail.push(Endpoint::Dram);
            chains.push(Vec::new());
            continue;
        }
        let cells: Vec<(usize, usize)> = (0..s.pcus).map(|i| pcu_cells[next_pcu + i]).collect();
        for (i, &(r, c)) in cells.iter().enumerate() {
            placed.push(Placed {
                unit: Unit::Pcu {
                    stage: s.stage,
                    index: i,
                },
                row: r,
                col: c,
            });
        }
        next_pcu += s.pcus;
        head.push(Endpoint::Cell(cells[0].0, cells[0].1));
        let last = cells[cells.len() - 1];
        tail.push(Endpoint::Cell(last.0, last.1));
        chains.push(cells);
    }
    let mut nets: Vec<MeshNet> = Vec::new();
    let mut net_owner: Vec<(usize, usize)> = Vec::new();
    for (si, chain) in chains.iter().enumerate() {
        for w in chain.windows(2) {
            nets.push(MeshNet {
                from: Endpoint::Cell(w[0].0, w[0].1),
                to: Endpoint::Cell(w[1].0, w[1].1),
            });
            net_owner.push((alloc.stages[si].stage, alloc.stages[si].stage));
        }
    }
    let side_pull = |p: (usize, usize)| p.1.min(cols - 1 - p.1);
    let anchor = |e: Endpoint, p: (usize, usize)| -> usize {
        match e {
            Endpoint::Cell(rr, cc) => manhattan(p, (rr, cc)),
            Endpoint::Dram => side_pull(p) + 1,
        }
    };
    let mut free_pmu = pmu_cells.clone();
    for b in &alloc.buffers {
        let (w, r) = b.edge;
        for i in 0..b.pmus {
            let (idx, _) = free_pmu
                .iter()
                .enumerate()
                .min_by_key(|(_, p)| anchor(tail[w], **p) + anchor(head[r], **p))
                .expect("a free PMU cell");
            let cell = free_pmu.remove(idx);
            placed.push(Placed {
                unit: Unit::Pmu {
                    edge: b.edge,
                    index: i,
                },
                row: cell.0,
                col: cell.1,
            });
            nets.push(MeshNet {
                from: tail[w],
                to: Endpoint::Cell(cell.0, cell.1),
            });
            net_owner.push(b.edge);
            nets.push(MeshNet {
                from: Endpoint::Cell(cell.0, cell.1),
                to: head[r],
            });
            net_owner.push(b.edge);
        }
    }
    let routing = router.route(t, budget, &nets);
    let mut routes: Vec<Route> = Vec::new();
    let mut link_use: std::collections::HashMap<(Link, u8), usize> =
        std::collections::HashMap::new();
    for (i, (links, occupancy)) in routing.links.into_iter().zip(routing.occupancy).enumerate() {
        for k in occupancy {
            *link_use.entry(k).or_insert(0) += 1;
        }
        let edge = net_owner[i];
        match routes.iter_mut().find(|rt| rt.edge == edge) {
            Some(rt) => {
                rt.hops += links.len();
                rt.links.extend(links);
            }
            None => routes.push(Route {
                edge,
                hops: links.len(),
                links,
            }),
        }
    }
    let max_link_occupancy = link_use.values().copied().max().unwrap_or(0);
    Ok(Placement {
        target: t.name,
        alloc: alloc.clone(),
        placed,
        routes,
        pcu_total: pcu_cells.len(),
        pmu_total: pmu_cells.len(),
        budget,
        max_link_occupancy,
        routed_statically: routing.legal,
        router: routing.router,
    })
}
