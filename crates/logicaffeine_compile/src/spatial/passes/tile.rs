//! `tile` — choose the per-site block plan from the legal lattice by the cost model.
//!
//! Candidates: the per-tile STREAMING family (one output tile per work item, single-tile FIFOs, tt-metal's
//! work split over the whole mesh) and every BLOCKED point of the divisor lattice (`per_site_m | Mt`,
//! `per_site_n | Nt` with the block grid inside the mesh, `osh | per_site_m`, `osw | per_site_n`,
//! `osh·osw ≤` the accumulator budget, `ibw | Kt`), each legal only if its buffers fit local memory. Every
//! candidate is scored for both reader arrangements (fused/multicast where the target can, unfused) with the
//! target's cycle model; the minimum wins. Deterministic tie-break: fewer bytes, then fewer sites, then the
//! lexicographically smallest plan. The winning arrangement is NOT stored — `fuse_readers` re-derives it
//! from the same cost function.

use super::size_buffers::footprint_bytes_for_plan;
use super::{SpatialReject, LV_ITEM_X, LV_ITEM_Y, LV_KBLOCK, LV_SUB_H, LV_SUB_W};
use crate::spatial::cost::{is_streaming, quantities_for_gemm, streaming_plan, PlanQuantities};
use crate::spatial::ir::*;
use crate::spatial::target::{HandshakeKind, Interconnect, SpatialTarget};
use crate::spatial::work::{work_of_plan, Objective, RankKey, Work};

/// One scored point of the lattice.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub plan: BlockPlan,
    pub fused: bool,
    pub quantities: PlanQuantities,
    pub cycles: f64,
    /// What this plan makes the machine do — the quantity the search actually ranks by, predicted in
    /// closed form and checked against both the built choreography and the simulator's work meter.
    pub work: Work,
    pub footprint_bytes: usize,
    /// The grid's extent does not divide the tile count, so the last row or column of sites carries what is
    /// left ([`BlockPlan::site_block`]).
    pub ragged: bool,
}

/// ★ O(√n), AND THE ORDER IS THE SAME ASCENDING LIST THE TRIAL SCAN PRODUCED.
///
/// This was `(1..=n).filter(|d| n % d == 0).collect()` — n trial divisions and a heap
/// allocation, called from INSIDE the five-deep candidate nest below. That is affordable
/// while `n` is a tile count on a 32×32-tile part (640 elements = 20 tiles, so n ≤ 20),
/// and it is not affordable on a 1×1-tile part, where the tile count IS the element count:
/// the same 640-element GEMM makes every bound in that nest 640 instead of 20, which is
/// 96× the lattice points and 169× the integers trial-divided. That is where this pass's
/// superlinear time and memory on scalar-tile targets came from.
///
/// Pairing `d` with `n / d` visits only up to √n, then one sort restores the exact
/// ascending order the old scan emitted — so the candidate lattice is enumerated in an
/// IDENTICAL order and `best_candidate_for` breaks ties exactly as before. Same output,
/// less work: this is a speed fix, not a planning change.
fn divisors(n: usize) -> Vec<usize> {
    let mut v = Vec::new();
    let mut d = 1usize;
    while d * d <= n {
        if n % d == 0 {
            v.push(d);
            let co = n / d;
            if co != d {
                v.push(co);
            }
        }
        d += 1;
    }
    v.sort_unstable();
    v
}

/// The accumulator of a template's matmul stage (the only tensor op the lattice applies to).
fn matmul_acc(p: &SpatialProgram) -> Option<AccPolicy> {
    p.stages.iter().find_map(|s| match &s.kind {
        StageKind::Compute(ComputeOp::MatmulTiles { acc, .. }) => Some(*acc),
        _ => None,
    })
}

/// Every legal candidate of `p` on `target`, scored.
pub fn candidates(p: &SpatialProgram, target: &SpatialTarget) -> Vec<Candidate> {
    candidates_where(p, target, &|_| true)
}

/// Every legal candidate of `p` on `target`, scored, where the plans a caller will keep decide whether the grid may
/// outgrow the mesh: items wrap onto sites only when no blocked plan the caller keeps fits the mesh. A caller that
/// restricts the arrangement (a calibration run pinning the packer's accumulate) gets the best plan of that
/// arrangement, not the streaming fallback of a round the plans it discards had filled.
pub fn candidates_where(
    p: &SpatialProgram,
    target: &SpatialTarget,
    keep: &dyn Fn(&BlockPlan) -> bool,
) -> Vec<Candidate> {
    let Some(acc) = matmul_acc(p) else {
        return Vec::new();
    };
    let out = p.output_tensor().expect("a template has an output tensor");
    let (mt, nt, kt) = (out.tiles.rows_t, out.tiles.cols_t, p.work.k_tiles);
    let exact = p.numerics == NumericsPolicy::Exact;
    let acc_budget = target.effective_acc_tiles(acc, exact);
    let can_fuse = target.provides_interconnect(Interconnect::MulticastRect)
        && target.provides_handshake(HandshakeKind::Semaphore);
    // The packer's L1 accumulate needs a packer that can add into a page, and only pays when the reduction
    // has more than two blocks (the vendor's own predicate: the drain costs a page round-trip per block).
    let pack_acc_ok = target.packer_accumulates();
    let two_engines = target.site.movement_engines >= 2;
    // Exact numerics spill at the accumulator's width; a target that cannot reload that width may not spill.
    //
    // ★ UNLESS THE ACCUMULATOR NEVER LEAVES THE REGISTERS. A GPU's `wmma` fragment is
    // register-resident and the K loop runs inside the accumulate region, so a blocked plan there
    // performs no spill at all — the reload it would be refused for is one it would never do.
    // Conflating "cannot reload a spill" with "must spill" made every blocked GPU plan illegal
    // under exact numerics, leaving only the streaming family: ONE output tile per item, measured
    // 214 GOPS against the knob lattice's 16,591 at 2048³.
    let acc_resident = target.accumulator_in_registers();
    let wide_spill_ok = target.reloads_wide_spills() || !exact || acc_resident;
    let fidelity = match target.matmul_cap() {
        Some(crate::spatial::target::ComputeCap::MatmulTile { fidelity, .. }) => *fidelity,
        _ => crate::spatial::target::Fidelity::Exact,
    };
    let mut cands = Vec::new();
    let push = |cands: &mut Vec<Candidate>, plan: BlockPlan, fused: bool| {
        let footprint = footprint_bytes_for_plan(p, &plan, target);
        // ★ THE BUDGET IS WHAT THE SITE CAN BE CONFIGURED WITH, not its default. An Ampere block
        // opts in to ~100 KiB of shared memory (`cudaFuncSetAttribute`), and the CUDA lowering
        // already emits that path — measuring against the 48 KiB default rejected every block tile
        // big enough to be worth running, so the planner could only pick tiles whose arithmetic
        // intensity was hopeless.
        if footprint > target.max_local_mem_bytes() {
            return;
        }
        let q = quantities_for_gemm(mt, nt, kt, &plan, fused, fidelity, target);
        let cycles = target.cycle.cycles(&q);
        let Some(work) = work_of_plan(p, &plan, fused, target) else {
            return;
        };
        let ragged = mt % plan.per_site_m != 0 || nt % plan.per_site_n != 0;
        cands.push(Candidate {
            plan,
            fused,
            quantities: q,
            cycles,
            work,
            footprint_bytes: footprint,
            ragged,
        });
    };
    push(&mut cands, streaming_plan(mt, nt, kt, acc), false);
    // Two rounds. The first offers only block grids that FIT the mesh, which is every plan we have measured.
    // The second, taken only when the first yielded no blocked plan at all, allows the grid to exceed the mesh
    // so items wrap onto sites — the difference between a blocked plan and streaming on a large GEMM.
    //
    // Wrapping is a fallback rather than a free candidate on purpose: a wrapped plan re-reads its operands
    // once per item, and the cost model is not yet accurate enough to arbitrate that against a mesh-fitting
    // plan (worst absolute error 55% on the current ledger). Offering both would let a model we know to be
    // wrong overturn decisions the simulator has already settled.
    // ★ THE DIVISOR LISTS ARE LOOP-INVARIANT — COMPUTE THEM ONCE.
    // `divisors(kt)` sat in the INNERMOST position, so it was rebuilt (and reheap-allocated)
    // once per (allow_wrap, pm, pn, osh, osw); `divisors(pm)`/`divisors(pn)` were rebuilt one
    // and two levels deeper than the value they depend on. Hoisting changes no value and no
    // order — it is the same lattice, enumerated the same way, without the rebuilds.
    let div_mt = divisors(mt);
    let div_nt = divisors(nt);
    let div_kt = divisors(kt);
    // ★ A GRID EXTENT NEED NOT DIVIDE THE TILE COUNT — the boundary block carries what is left.
    //
    // Deriving the grid only from divisors means a reachable extent must be a divisor of the tile count
    // that also fits the mesh. On a 10x13 mesh a 64x64-tile output caps at 8x8 = 64 of 130 cores, because
    // 64's divisors jump 8 -> 16 and straddle both bounds. Measured over 256 shapes, Blackhole averages
    // 57% of its mesh and NOT ONE shape reaches all 130 — 13 is prime, and no power of two has a divisor
    // of 13. Wormhole's 8x8 is the friendly case, which is why this went unnoticed for the whole campaign.
    //
    // `BlockPlan::site_block` defines what each site covers; `place` emits its patterns from it and
    // `work_of_plan` prices from it, so the planner cannot rank a block the emitter will not lay out.
    //
    // Offered ONLY where the grid FITS the mesh: then `items == ny*nx <= sites`, every site holds exactly
    // one block, and its extent is well defined. A wrapped plan's site holds several blocks at once and
    // could not describe two different extents with one stage — so wrap stays divisor-derived.
    // Every divisor is still enumerated (for a divisor `pm`, `div_ceil` IS `mt / pm`), so this strictly
    // ADDS candidates and the objective still chooses among them.
    let extents = |n: usize, mesh: usize| -> Vec<usize> {
        let mut v = divisors(n);
        v.extend((1..=mesh).map(|g| n.div_ceil(g)));
        v.sort_unstable();
        v.dedup();
        v
    };
    let ragged_mt = extents(mt, target.sites.y);
    let ragged_nt = extents(nt, target.sites.x);
    // ★ WHY THE GRID IS STILL DERIVED FROM DIVISORS — and exactly what must change first.
    //
    // A grid extent that does not divide the tile count would let a 64x64-tile output use 10x13 of
    // Blackhole's mesh instead of 8x8: measured over 256 shapes, that chip averages 57% of its cores and
    // NOT ONE shape reaches all 130, because 13 is prime and no power of two has a divisor of 13. The
    // prize is 1.52x the cores (`examples/probe_mesh_utilization.rs`).
    //
    // Two of the three things such a grid needs are already here. `WorkItem.out` is a `TileRect` with its
    // own `h_t`/`w_t`, so a smaller boundary block needs no new IR — and `apply_plan` below already clamps
    // items to the tensor. The Tensix backend already emits one kernel per program class.
    //
    // What is NOT here is the operand READ. `AccessPattern` has an affine ORIGIN and a CONSTANT extent
    // (`block_h`, `block_w`), and the pattern lives on the STAGE, shared by every site — so a boundary
    // site cannot describe a narrower read, and enumerating the wider grid makes the pipeline's own
    // `verify` reject the program: `ReadOutOfRange { tensor: 1, tile: 1024 }` on a B operand that holds
    // exactly 1024 tiles. (Tried, measured, reverted — the enumeration change alone is one line.)
    //
    // The fix is to give `AccessPattern` the meaning it should always have had: the rectangle INTERSECTED
    // with the tensor's tile grid, never past the end. That is `eval` skipping instead of faulting, and a
    // runtime `min` in the readers — small in each place and touching four printers plus every kernel
    // census, which is why it is named here rather than done in passing.
    // `spatial_tile_lock::the_planner_fills_a_mesh_whose_dimensions_do_not_divide_the_tile_count` is the
    // ignored test that states it.
    for allow_wrap in [false, true] {
        if allow_wrap
            && cands
                .iter()
                .any(|c| !is_streaming(&c.plan) && keep(&c.plan))
        {
            break;
        }
        for &pm in if allow_wrap { &div_mt } else { &ragged_mt } {
            let ny = mt.div_ceil(pm);
            let div_pm = divisors(pm);
            for &pn in if allow_wrap { &div_nt } else { &ragged_nt } {
                let nx = nt.div_ceil(pn);
                let div_pn = divisors(pn);
                // A block grid LARGER than the mesh is legal: `place` wraps items onto sites modulo the mesh and
                // the cost model already charges `blocks / sites` items per site. Requiring the grid to fit forces
                // a per-site block big enough to cover the whole output, whose buffers exceed local memory on any
                // large GEMM — which is why an 8192-square problem could only stream.
                //
                // Fusion is the exception, and for a structural reason: a multicast group shares one operand panel
                // down a mesh row or column, and once items wrap, a row of the mesh holds blocks from several
                // different block-rows that share no panel. So a fused candidate must still fit the mesh.
                let fits_mesh = ny <= target.sites.y && nx <= target.sites.x;
                if !fits_mesh && !allow_wrap {
                    continue;
                }
                // ★ RAGGED IN AT MOST ONE AXIS — a MEASURED build-time bound, not a taste.
                //
                // Each distinct block shape is its own kernel class, and tt-metal compiles a separate RISC-V
                // kernel per class. Ragged in one axis gives two shapes (interior, edge); ragged in BOTH gives
                // four (interior, right, bottom, corner) and multiplies the kernels to build. Measured: a 7x7
                // grid ragged both ways at 640x640x1280 blew past a 45-minute build timeout, where an 8x9 grid
                // ragged only in rows at 4096x256x256 built in about eight minutes and measured 72 sites at
                // 10,755 cycles against the recorded 64 sites at 11,175.
                //
                // The utilisation given up is small: on Blackhole's 10x13 mesh a 64x64-tile output reaches 104
                // of 130 sites with one ragged axis against 130 with two, and 64 with neither. Most of the
                // prize, a bounded number of kernels.
                if mt % pm != 0 && nt % pn != 0 {
                    continue;
                }
                for &osh in &div_pm {
                    for &osw in &div_pn {
                        if osh * osw > acc_budget {
                            continue;
                        }
                        // ★ ON A REGISTER-RESIDENT TARGET THE BLOCK IS THE SUB-BLOCK. One accumulate
                        // region spans the whole K reduction, so a second sub-block would need a second
                        // set of accumulators live at the same time — which is not what the single
                        // `AcquireAcc` the synthesis emits provides.
                        if acc_resident && (pm != osh || pn != osw) {
                            continue;
                        }
                        // …and its tiles must split evenly across the site's accumulator lanes, or the
                        // epilogue cannot write straight to global and the plan is sized for a scratch
                        // it would overrun.
                        if acc_resident && target.accumulator_lane_grid(osh, osw).is_none() {
                            continue;
                        }
                        for &ibw in &div_kt {
                            // A matrix engine that caps its PE array takes a block of at most its rows of contraction
                            // and its columns of output; the rest of the contraction runs as further blocks.
                            if let Some((rows, cols)) = target.matmul_max_array() {
                                if ibw * target.tile.cols > rows || osw * target.tile.cols > cols {
                                    continue;
                                }
                            }
                            let plan = BlockPlan {
                                granularity: if target.matmul_block_api() {
                                    Granularity::PerSubBlock
                                } else {
                                    Granularity::PerTile
                                },
                                per_site_m: pm,
                                per_site_n: pn,
                                grid: (ny, nx),
                                inner: InnerLoop {
                                    num_blocks: kt / ibw,
                                    block_k_tiles: ibw,
                                },
                                block: BlockShape { osh, osw, ibw },
                                acc,
                                pack_accumulate: false,
                                engines: EnginePacking::LoadsTogether,
                            };
                            if is_streaming(&plan) {
                                continue;
                            }
                            // A plan that spills needs the target to be able to reload a partial of the
                            // accumulator's width; under exact numerics the spill IS that width.
                            //
                            // This covers the packer's in-place accumulate too, MEASURED: its blocks 0…nb-2 do
                            // avoid the reload, but the last block still reloads once to pack the result out, and
                            // that single reload is the same illegal wide unpack. A reload-free last block needs
                            // the store to read the accumulated intermediate directly; until it does, a spilling
                            // plan under exact numerics is refused rather than emitted for the device to reject.
                            let reload_ok = !(exact && kt / ibw > 1 && !wide_spill_ok);
                            // The packer's L1 accumulate is a plan choice wherever the target offers it and the
                            // reduction has enough blocks for the in-place sum to pay for its drain. Its last block
                            // reloads the partial once, unless the partial is the result and the packer finishes in
                            // the output's own page: then it reloads nothing, and it is how an exact reduction splits
                            // where the target cannot reload a partial of the accumulator's width.
                            let l1 = BlockPlan {
                                pack_accumulate: true,
                                ..plan
                            };
                            let l1_ok = finishes_in_place(p, &l1, target)
                                || (pack_acc_ok && kt / ibw > 2 && reload_ok);
                            // The movement packing is a plan choice too: pairing the second operand's loader with
                            // the store overlaps a block's output write with the next block's operand reads.
                            let packings: &[EnginePacking] = if two_engines {
                                &[
                                    EnginePacking::LoadsTogether,
                                    EnginePacking::OperandWithStore,
                                ]
                            } else {
                                &[EnginePacking::LoadsTogether]
                            };
                            for &engines in packings {
                                for (base, ok) in [(plan, reload_ok), (l1, l1_ok)] {
                                    if !ok {
                                        continue;
                                    }
                                    let arranged = BlockPlan { engines, ..base };
                                    push(&mut cands, arranged, false);
                                    if can_fuse && fits_mesh && (nx > 1 || ny > 1) {
                                        push(&mut cands, arranged, true);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    cands
}

/// ★ THE RANKING, AND WHAT IT IS MADE OF. The leading terms come from `Objective::key` over the plan's
/// PREDICTED WORK — bytes and transactions the choreography will actually issue, checked against the
/// machine by `spatial_work_model_lock`. Everything after them is a deterministic tie-break so that two
/// plans the objective cannot separate are still ordered the same way on every run.
///
/// It used to lead with `c.cycles`, a regression against ttsim's bracket counter. That counter is not a
/// function of the program — 8x the reduction depth moves it 0.08% — so the leading term of the plan
/// search was noise, and at 512^3 it chose the plan that issues 2.94x the instructions and moves 3.40x
/// the bytes of the one beside it. `cycles` is kept on the candidate for the calibration harness and no
/// longer decides anything.
fn plan_key(
    c: &Candidate,
    objective: Objective,
) -> (
    RankKey,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    bool,
) {
    (
        objective.key(&c.work),
        c.footprint_bytes,
        c.quantities.sites_used,
        c.plan.per_site_m,
        c.plan.per_site_n,
        c.plan.block.osh,
        c.plan.block.osw,
        c.plan.block.ibw,
        c.fused,
    )
}

/// The winning candidate (cost-minimal, deterministic tie-break).
///
/// ★ ON A REGISTER-ACCUMULATOR TARGET THE RANKING IS ARITHMETIC INTENSITY, not the cycle model.
///
/// The model is a roofline fitted on TENSIX measurements — its own doc records a worst absolute
/// error of 55% on that ledger — and it has never been fitted for a GPU. Asked to order GPU plans
/// it preferred a 32-tile block over the 64-tile block that measured FASTER, which is the failure
/// mode of using a model outside the data it was fitted on.
///
/// So for that target the choice is made on the physics we are sure of. A block computing
/// `osh · osw` output tiles over `ibw` contraction tiles does `osh · osw · ibw` tile-multiplies per
/// `(osh + osw) · ibw` tiles read, so intensity rises with the block and a GEMM at this size is
/// bandwidth-bound — bigger is faster until a resource binds, and the candidate list only contains
/// plans that already fit. Ties fall back to the model, so nothing about the Tensix path changes.
pub fn best_candidate(p: &SpatialProgram, target: &SpatialTarget) -> Option<Candidate> {
    best_candidate_for(p, target, Objective::default())
}

/// ★ ON A TARGET WHOSE TIME IS EXACT, THE PLANNER TIMES ITS CANDIDATES. The fabric's cycle twin
/// ([`fabric_cycles_within`](crate::spatial::lower::fabric::fabric_cycles_within)) is cycle-exact against its RTL, so
/// the planner builds candidates in the work model's order and times each one, and the fastest is the plan. A model
/// stands in for time only where nothing can measure it; on the fabric every engine change would otherwise leave the
/// model choosing a plan the twin can show is slower.
///
/// The timing is bounded and deterministic: the candidates share [`TWIN_PLANNING_CYCLES`] cycles of simulation, each
/// one stopping once it is slower than the fastest so far — it can no longer win — so a program small enough for the
/// fabric is timed through every candidate, and a large one is timed through as many of the model's best as the
/// budget reaches. Only the fastest timed candidate is certified, and the first that certifies is the plan.
///
/// `None` on every other target — the fabric's lowering is the one that states its local memory's read latency — and
/// where no candidate builds and finishes within the budget.
pub fn fastest_on_the_twin(p: &SpatialProgram, target: &SpatialTarget) -> Option<Candidate> {
    target.site.local_mem_read_latency?;
    let mut cands = candidates(p, target);
    cands.sort_by_key(|c| plan_key(c, Objective::default()));
    let mut budget = TWIN_PLANNING_CYCLES;
    let mut timed: Vec<(u64, usize, Candidate, SpatialProgram)> = Vec::new();
    for (rank, c) in cands.into_iter().enumerate() {
        if budget == 0 {
            break;
        }
        let Ok(built) = crate::spatial::pipeline::build_candidate_uncertified(p, &c, target) else { continue };
        let fastest = timed.iter().map(|t| t.0).min().unwrap_or(u64::MAX);
        let limit = budget.min(fastest);
        match crate::spatial::lower::fabric::fabric_cycles_within(&built, target, limit) {
            Some(cycles) => {
                budget -= cycles;
                timed.push((cycles, rank, c, built));
            }
            None => budget -= limit,
        }
    }
    timed.sort_by_key(|(cycles, rank, ..)| (*cycles, *rank));
    timed
        .into_iter()
        .find(|(.., built)| super::verify::verify(built, target).is_ok())
        .map(|(_, _, c, _)| c)
}

/// The cycles of simulation the planner spends timing one program's candidates on the fabric
/// ([`fastest_on_the_twin`]): enough to time every candidate of a program the size of a decode step's section, a
/// fraction of a second of the twin.
pub const TWIN_PLANNING_CYCLES: u64 = 4_000_000;

/// The winning candidate under a stated objective.
pub fn best_candidate_for(
    p: &SpatialProgram,
    target: &SpatialTarget,
    objective: Objective,
) -> Option<Candidate> {
    let cands = candidates(p, target);
    if target.accumulator_in_registers() {
        // ★ PREDATES THE WORK MODEL. This branch exists because the CYCLE model was fitted on Tensix and
        // ordered GPU plans backwards; the work model is derived rather than fitted and has no such
        // excuse, but nothing here can measure a GPU, so the intensity ranking stays until a measurement
        // lane exists to justify replacing it. Changing it blind would be trading a checked heuristic for
        // an unchecked one.
        return cands.into_iter().max_by_key(|c| {
            (
                intensity_key(c),
                // ★ OCCUPANCY BEFORE DEPTH. Intensity cancels `ibw` exactly, so depth used to decide on
                // its own — and a deeper stage buys fewer pipeline round-trips WITH shared memory, which
                // on this target is how many sites a unit can hold. Measured at 2048³ (see
                // `SpatialTarget::resident_sites_per_unit`): K=16 → 5,429 GOPS, K=32 → 5,743, K=64 →
                // 4,102, the last one having fallen from two resident blocks to one. The rule is the
                // deepest stage that does not cost a block, not the deepest stage.
                target.resident_sites_per_unit(
                    c.footprint_bytes,
                    c.plan.block.osh,
                    c.plan.block.osw,
                ),
                stage_depth_key(c),
                std::cmp::Reverse(plan_key(c, objective)),
            )
        });
    }
    best_among(&cands, objective).cloned()
}

/// The candidate `objective` chooses among `cands`: the admissible ones ([`admissible`]), ranked by
/// `plan_key`.
pub fn best_among(cands: &[Candidate], objective: Objective) -> Option<&Candidate> {
    admissible(cands, objective)
        .into_iter()
        .min_by_key(|c| plan_key(c, objective))
}

/// The candidates `objective` may choose among.
///
/// ★ A RAGGED GRID ONLY WHERE IT COSTS NOTHING IN TOTAL WORK. `Balanced` ranks the busiest site's arithmetic
/// second, so it favours a grid that spreads the arithmetic thinner even where the spread moves more bytes in
/// more transactions than an evenly divided plan beside it — at 640³ on the Wormhole twin, a ragged 7×5 grid
/// against the 5×5 that beats it on every meter, a benefit the simulator cannot show. So under `Balanced` a
/// ragged plan is admissible only where no evenly divided plan beats it on every predicted total: bytes moved,
/// transactions and compute-engine calls. The comparison is against the even plans only: against every plan it
/// would choose the fewest sites, since a plan on fewer sites moves fewer bytes in total, and refuse the
/// parallelism the objective exists to buy. Every other objective names its own axis and admits every plan;
/// the roofline prices the spread a ragged grid buys against the bytes and transactions it costs, at the
/// target's cited rates, which is the measurement this rule stands in for.
pub fn admissible(cands: &[Candidate], objective: Objective) -> Vec<&Candidate> {
    if objective != Objective::Balanced {
        return cands.iter().collect();
    }
    let front = even_front(cands);
    cands
        .iter()
        .filter(|c| !c.ragged || !front.iter().any(|t| beats(*t, totals(c))))
        .collect()
}

/// The predicted totals a ragged plan answers for: bytes moved, transactions, compute-engine calls.
fn totals(c: &Candidate) -> (u64, u64, u64) {
    (c.work.noc_bytes, c.work.noc_txns, c.work.compute_calls)
}

/// `a` is no worse than `b` on every total, and better on one.
fn beats(a: (u64, u64, u64), b: (u64, u64, u64)) -> bool {
    a.0 <= b.0 && a.1 <= b.1 && a.2 <= b.2 && a != b
}

/// The evenly divided plans' totals that no other even plan beats — the only ones that can beat anything,
/// since beating is transitive. A few dozen points out of thousands of candidates. In ascending order a point
/// can be beaten only by one before it, so one pass against the front kept so far finds it.
fn even_front(cands: &[Candidate]) -> Vec<(u64, u64, u64)> {
    let mut points: Vec<(u64, u64, u64)> = cands.iter().filter(|c| !c.ragged).map(totals).collect();
    points.sort_unstable();
    points.dedup();
    let mut front: Vec<(u64, u64, u64)> = Vec::new();
    for p in points {
        if !front.iter().any(|f| beats(*f, p)) {
            front.push(p);
        }
    }
    front
}

/// A blocked plan's arithmetic intensity, as an integer ratio scaled to compare exactly: tile
/// multiplies per operand tile read. Streaming plans score zero — they read both operands per
/// multiply, which is the whole reason they are the fallback.
fn intensity_key(c: &Candidate) -> usize {
    if is_streaming(&c.plan) {
        return 0;
    }
    let BlockShape { osh, osw, ibw } = c.plan.block;
    let reads = (osh + osw) * ibw;
    if reads == 0 {
        return 0;
    }
    (osh * osw * ibw * 1024) / reads
}

/// How many contraction tiles a stage carries. Ties in intensity are broken toward a DEEPER stage:
/// every stage pays a pipeline round-trip (acquire, commit, wait, release) and a deeper one amortises
/// it over more multiplies, so between two plans of equal intensity the one with fewer, larger stages
/// does the same arithmetic with less synchronisation.
fn stage_depth_key(c: &Candidate) -> usize {
    if is_streaming(&c.plan) {
        0
    } else {
        c.plan.block.ibw
    }
}

/// ★ WHERE THE PARTIAL ALREADY IS THE RESULT, THE REDUCTION FINISHES IN THE OUTPUT'S PAGE. Exact numerics hold
/// the partial sums at the accumulator's own width; where the output is stored at that width and nothing
/// computes after the matmul, the last K block's sum is the result itself. A target whose packer adds into the
/// output's page ([`PackerAccumulate::IntoTheOutput`](crate::spatial::target::PackerAccumulate::IntoTheOutput))
/// then accumulates every block there — the intermediate shares the output's memory (`Buffer::alias_of`) — and
/// never reloads the partial into the register file, which Wormhole and Blackhole cannot do at that width.
///
/// ★ ONLY WHERE THE SUM IS THE SAME IN EVERY ORDER. The packer adds a block's sum, formed from zero in the
/// register file, to the partial: it re-associates the reduction at every block boundary, where exact numerics
/// promise a straight K-ascending accumulation. The two agree bit for bit exactly when every partial sum is an
/// integer the accumulator holds exactly, which the program's claim states (`NumericsClaim::Exact`: integer
/// operands whose bound times K stays inside fp32's mantissa). On data whose range is unknown the reduction
/// keeps its order.
pub fn finishes_in_place(p: &SpatialProgram, plan: &BlockPlan, target: &SpatialTarget) -> bool {
    let computes_after = p.stages.iter().any(|s| {
        matches!(
            s.kind,
            StageKind::Compute(_) | StageKind::Scalar(_) | StageKind::AllReduce { .. }
        ) && !matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. }))
    });
    target.packer_accumulates_into_the_output()
        && plan.pack_accumulate
        && plan.inner.num_blocks > 1
        && !is_streaming(plan)
        && p.numerics == NumericsPolicy::Exact
        && p.claim == NumericsClaim::Exact
        && p.output_tensor()
            .is_some_and(|o| o.dtype == plan.acc.interm_precision())
        && !computes_after
}

/// Apply a plan to a template: patterns, block shape, spill buffer, work items.
pub fn apply_plan(p: &SpatialProgram, plan: BlockPlan, target: &SpatialTarget) -> SpatialProgram {
    let mut q = p.clone();
    let out = q.output_tensor().unwrap().clone();
    let (mt, nt, kt) = (out.tiles.rows_t, out.tiles.cols_t, q.work.k_tiles);
    let (pm, pn) = (plan.per_site_m, plan.per_site_n);
    let BlockShape { osh, osw, ibw } = plan.block;
    let nb = plan.inner.num_blocks;
    let sub_blocks = (pm / osh, pn / osw);
    // Buffers: page = one block of the operand, one sub-block of the output.
    let (mut in0, mut in1, mut outb) = (None, None, None);
    for s in &q.stages {
        match &s.kind {
            StageKind::Load { tensor, .. } if tensor.idx() == 0 => in0 = s.outs.first().copied(),
            StageKind::Load { tensor, .. } if tensor.idx() == 1 => in1 = s.outs.first().copied(),
            StageKind::Compute(ComputeOp::MatmulTiles { .. }) => outb = s.outs.first().copied(),
            _ => {}
        }
    }
    let (in0, in1, outb) = (
        in0.expect("load_a"),
        in1.expect("load_b"),
        outb.expect("mm out"),
    );
    let in_place = finishes_in_place(p, &plan, target);
    // ★ A DEEPER OPERAND RING WHERE OCCUPANCY CANNOT HIDE LATENCY. A persistent GPU kernel runs
    // ONE block per site — 8 warps of the 48 an SM can hold — so there are no other warps to run
    // while a load is outstanding, and the only thing covering DRAM latency is how far ahead the
    // pipeline can read. Three stages let the loads for block i+2 be in flight while block i
    // computes; two only ever cover one block's worth.
    //
    // A Tensix site is the opposite case — its two movement engines and its own scheduler cover
    // the latency — so it keeps the double buffer it was measured with.
    // MEASURED: three stages made it SLOWER (2,397 → 2,060 GOPS). The extra page costs shared
    // memory, and shared memory is what the block tile is bought with — the deeper pipeline covered
    // less latency than the smaller tile lost in arithmetic intensity. Two stages.
    let operand_depth = 2;
    q.buffers[in0.idx()].kind = BufferKind::Fifo {
        depth_pages: operand_depth,
        page_tiles: pm * ibw,
    };
    q.buffers[in1.idx()].kind = BufferKind::Fifo {
        depth_pages: operand_depth,
        page_tiles: ibw * pn,
    };
    // ★ WHERE THE EPILOGUE IS FUSED, THE OUTPUT BUFFER IS A PER-WARP SCRATCH. A register-resident
    // accumulator writes its tiles straight to global one at a time through a slot of its own, so
    // the block's output never stages through a ring — and the ring it used to need was the
    // largest in the kernel (`osh · osw` tiles at the accumulator's width, double-buffered), the
    // one thing stopping the planner from affording a block tile worth running.
    //
    // Four tiles: one per warp, which is what the lowering declares and what the footprint model
    // charges, so the plan the planner priced is the plan the allocator lays out.
    q.buffers[outb.idx()].kind =
        if target.accumulator_in_registers() && target.accumulator_lane_grid(osh, osw).is_some() {
            // One tile per accumulator lane — the fused epilogue's whole shared-memory cost, whatever
            // the block tile is. Sized only where the epilogue REALLY fuses; a block whose tiles do not
            // split across the lanes still stages through a full ring, and sizing that as a scratch is
            // what the device reported as an illegal access.
            BufferKind::Fifo {
                depth_pages: target.accumulator_lanes(),
                page_tiles: 1,
            }
        } else if in_place {
            // The intermediate shares this buffer's memory page for page, so both hold exactly one page per
            // sub-block and their write pointers meet at the same page when the last block packs.
            BufferKind::Fifo {
                depth_pages: sub_blocks.0 * sub_blocks.1,
                page_tiles: osh * osw,
            }
        } else {
            BufferKind::Fifo {
                depth_pages: (sub_blocks.0 * sub_blocks.1).max(2),
                page_tiles: osh * osw,
            }
        };
    let operand = q.buffers[in0.idx()].dtype;
    // A spill buffer exists only where the compute engine spills: a blocked plan with more than one K block.
    // The streaming family accumulates every K step in the register file and never spills.
    // A spill buffer exists only where the accumulator has to be DRAINED between K blocks. Where it
    // is register-resident (the GPU) the partial never touches memory, and emitting a buffer for it
    // would be a construct the lowering then refuses.
    let interm = if nb > 1
        && !crate::spatial::cost::is_streaming(&plan)
        && !target.accumulator_in_registers()
    {
        let dtype = if q.numerics == NumericsPolicy::Exact {
            plan.acc.interm_precision()
        } else {
            operand
        };
        let id = BufferId::new(q.buffers.len());
        q.buffers.push(Buffer {
            id,
            name: "interm".into(),
            kind: BufferKind::Interm {
                depth_pages: sub_blocks.0 * sub_blocks.1,
                page_tiles: osh * osw,
            },
            dtype,
            space: MemSpace::Local,
            site: None,
            origin: None,
            alias_of: in_place.then_some(outb),
        });
        Some(id)
    } else {
        None
    };
    for s in q.stages.iter_mut() {
        match &mut s.kind {
            StageKind::Load {
                tensor,
                pattern,
                num_blocks,
                ..
            } if tensor.idx() == 0 => {
                *pattern = AccessPattern {
                    origin: TileAddr {
                        base: 0,
                        steps: vec![(LV_ITEM_Y, pm * kt), (LV_KBLOCK, ibw)],
                        indirect: None,
                    },
                    block_h: pm,
                    block_w: ibw,
                    stride_w: 1,
                    stride_h: kt,
                };
                *num_blocks = nb;
            }
            StageKind::Load {
                tensor,
                pattern,
                num_blocks,
                ..
            } if tensor.idx() == 1 => {
                *pattern = AccessPattern {
                    origin: TileAddr {
                        base: 0,
                        steps: vec![(LV_ITEM_X, pn), (LV_KBLOCK, ibw * nt)],
                        indirect: None,
                    },
                    block_h: ibw,
                    block_w: pn,
                    stride_w: 1,
                    stride_h: nt,
                };
                *num_blocks = nb;
            }
            StageKind::Store {
                pattern,
                sub_blocks: sb,
                ..
            } => {
                *pattern = AccessPattern {
                    origin: TileAddr {
                        base: 0,
                        steps: vec![
                            (LV_ITEM_Y, pm * nt),
                            (LV_ITEM_X, pn),
                            (LV_SUB_H, osh * nt),
                            (LV_SUB_W, osw),
                        ],
                        indirect: None,
                    },
                    block_h: osh,
                    block_w: osw,
                    stride_w: 1,
                    stride_h: nt,
                };
                *sb = sub_blocks;
            }
            StageKind::Compute(ComputeOp::MatmulTiles {
                block,
                sub_blocks: sb,
                num_blocks,
                interm: it,
                granularity,
                ..
            }) => {
                // A whole sub-block per call where the engine takes one: the unpacker, the math unit and the
                // packer then pipeline across the sub-block instead of restarting per tile.
                *granularity = plan.granularity;
                *block = plan.block;
                *sb = sub_blocks;
                *num_blocks = nb;
                *it = interm;
            }
            _ => {}
        }
    }
    // A compute chain after the matmul carries the matmul's page shape; a row-vector operand (a bias) is
    // loaded per sub-block column.
    let stage_ids: Vec<StageId> = q.stages.iter().map(|s| s.id).collect();
    for sid in stage_ids {
        let (kind, ins, outs) = {
            let s = &q.stages[sid.idx()];
            (s.kind.clone(), s.ins.clone(), s.outs.clone())
        };
        match kind {
            StageKind::Compute(ComputeOp::MatmulTiles { .. }) => {}
            StageKind::Compute(_) | StageKind::Scalar(_) => {
                if let (Some(i), Some(o)) = (ins.first(), outs.first()) {
                    let page = q.buffers[i.idx()].kind.page_tiles();
                    if let BufferKind::Fifo { depth_pages, .. } = q.buffers[o.idx()].kind {
                        q.buffers[o.idx()].kind = BufferKind::Fifo {
                            depth_pages,
                            page_tiles: page,
                        };
                    }
                }
            }
            StageKind::Load { tensor, .. }
                if tensor.idx() >= 2 && q.tensors[tensor.idx()].tiles.rows_t == 1 =>
            {
                let cols_t = q.tensors[tensor.idx()].tiles.cols_t;
                if let StageKind::Load {
                    pattern,
                    num_blocks,
                    ..
                } = &mut q.stages[sid.idx()].kind
                {
                    *pattern = AccessPattern {
                        origin: TileAddr {
                            base: 0,
                            steps: vec![(LV_ITEM_X, pn), (LV_SUB_W, osw)],
                            indirect: None,
                        },
                        block_h: 1,
                        block_w: osw,
                        stride_w: 1,
                        stride_h: cols_t,
                    };
                    *num_blocks = 1;
                }
                if let Some(o) = outs.first() {
                    if let BufferKind::Fifo { depth_pages, .. } = q.buffers[o.idx()].kind {
                        q.buffers[o.idx()].kind = BufferKind::Fifo {
                            depth_pages,
                            page_tiles: osw,
                        };
                    }
                }
            }
            _ => {}
        }
    }
    let (ny, nx) = plan.grid;
    let mut items = Vec::with_capacity(ny * nx);
    for oy in 0..ny {
        for ox in 0..nx {
            // The boundary row and column carry what is LEFT, which is what makes a non-dividing grid
            // legal: items already own their extent, so a smaller last block needs no new IR.
            let (row0, col0) = (oy * pm, ox * pn);
            let h_t = pm.min(mt.saturating_sub(row0));
            let w_t = pn.min(nt.saturating_sub(col0));
            if h_t == 0 || w_t == 0 {
                continue;
            }
            items.push(WorkItem {
                id: WorkItemId::new(items.len()),
                coord: (oy, ox),
                out: TileRect {
                    row0_t: row0,
                    col0_t: col0,
                    h_t,
                    w_t,
                },
            });
        }
    }
    q.work.items = items;
    q.work.plan = Some(plan);
    q
}

/// The `tile` pass.
pub fn tile(p: &SpatialProgram, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    if p.work.plan.is_some() {
        return Ok(p.clone());
    }
    if p.phase() != Phase::Template {
        return Err(SpatialReject::WrongPhase {
            pass: "tile",
            expected: "Template",
            found: p.phase(),
        });
    }
    let Some(acc) = matmul_acc(p) else {
        // Non-matmul templates (elementwise, row ops, scalar) carry a trivial one-block plan.
        return trivial_plan(p, target);
    };
    let out = p.output_tensor().unwrap();
    let exact = p.numerics == NumericsPolicy::Exact;
    let best = fastest_on_the_twin(p, target).or_else(|| best_candidate(p, target)).ok_or(SpatialReject::NoLegalTiling {
        rows_t: out.tiles.rows_t,
        cols_t: out.tiles.cols_t,
        k_tiles: p.work.k_tiles,
        acc_tiles: target.effective_acc_tiles(acc, exact),
        sites: target.sites.sites(),
    })?;
    let q = apply_plan(p, best.plan, target);
    q.validate()?;
    Ok(q)
}

/// Programs without a tile matmul (elementwise / row ops / scalar bodies): one block per work item with
/// unit sub-blocks — the streaming family, whose legality is the local-memory fit.
fn trivial_plan(
    p: &SpatialProgram,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let out = p.output_tensor().expect("an output tensor");
    let (mt, nt) = (out.tiles.rows_t, out.tiles.cols_t);
    // The template's own work granularity (a row op works a whole tile-row per item); per tile otherwise.
    let acc = AccPolicy::for_result(out.dtype);
    let mut q = p.clone();
    if q.work.items.is_empty() {
        let mut items = Vec::with_capacity(mt * nt);
        for oy in 0..mt {
            for ox in 0..nt {
                items.push(WorkItem {
                    id: WorkItemId::new(items.len()),
                    coord: (oy, ox),
                    out: TileRect {
                        row0_t: oy,
                        col0_t: ox,
                        h_t: 1,
                        w_t: 1,
                    },
                });
            }
        }
        q.work.items = items;
    }
    let (h, w) = q
        .work
        .items
        .first()
        .map(|it| (it.out.h_t.max(1), it.out.w_t.max(1)))
        .unwrap_or((1, 1));
    // An in-place update of carried state works only its items' blocks (see `validate_plan`).
    let grid = if out.role.is_fully_covered() {
        (mt.div_ceil(h), nt.div_ceil(w))
    } else {
        (1, q.work.items.len())
    };
    let plan = BlockPlan {
        per_site_m: h,
        per_site_n: w,
        grid,
        inner: InnerLoop {
            num_blocks: 1,
            block_k_tiles: 1,
        },
        block: BlockShape {
            osh: h,
            osw: w,
            ibw: 1,
        },
        acc,
        pack_accumulate: false,
        engines: EnginePacking::LoadsTogether,
        granularity: Granularity::PerTile,
    };
    q.work.k_tiles = q.work.k_tiles.max(1);
    q.work.plan = Some(plan);
    let footprint = q.buffers.iter().map(|b| q.buffer_bytes(b)).sum::<usize>();
    // The budget is what the site can be configured with, as the matmul plans' is.
    if footprint > target.max_local_mem_bytes() {
        return Err(SpatialReject::NoLegalTiling {
            rows_t: mt,
            cols_t: nt,
            k_tiles: q.work.k_tiles,
            acc_tiles: q.acc_tiles_used,
            sites: target.sites.sites(),
        });
    }
    q.validate()?;
    Ok(q)
}

// ── The block-lattice search, as algebra ────────────────────────────────────────────────────────────────
//
// `candidates` above searches this same lattice against a calibrated cycle model and a full `SpatialProgram`.
// What follows is the lattice ITSELF — divisors, the three legality rules, and a heuristic ranking — stated
// over nothing but three tile counts and a part's limits. The Tenstorrent facade is expressed in terms of it
// rather than carrying its own copy, and `tt_tiling_specification_lock` restates it independently and
// requires the answer to be cost-minimal, so the two can never drift into disagreeing about what is legal.

/// What a part can hold, in the units the lattice is filtered by. A named-part preset produces one of these;
/// so does a [`SpatialTarget`], which is what makes the search portable rather than Tenstorrent-shaped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileLimits {
    /// Output tiles the accumulator register file holds per acquire — the reuse sub-block ceiling.
    pub max_acc_tiles: usize,
    /// Site-grid width (max sites across the N dimension).
    pub sites_x: usize,
    /// Site-grid height (max sites across the M dimension).
    pub sites_y: usize,
    /// Per-site local SRAM (KiB) — budgets the resident operand blocks, which bounds the inner block width.
    pub local_kib: usize,
}

/// A tiling / blocking config for the blocked matmul kernel — the knobs an autotuner searches, in 32×32
/// tiles. [`best_tile_config`] fills this from a cost model; the vendor hand-tunes the equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileConfig {
    /// Site columns (each owns `per_core_n` output tile-columns).
    pub grid_x: usize,
    /// Site rows (each owns `per_core_m` output tile-rows).
    pub grid_y: usize,
    /// Output tile-rows computed per site.
    pub per_core_m: usize,
    /// Output tile-cols computed per site.
    pub per_core_n: usize,
    /// Inner block width — contraction tiles streamed per block (bigger ⇒ more reuse per DRAM read).
    pub in0_block_w: usize,
    /// Accumulator-resident output sub-block height.
    pub out_subblock_h: usize,
    /// Accumulator-resident output sub-block width.
    pub out_subblock_w: usize,
}

impl TileConfig {
    /// Output tiles held in the accumulator register file per acquire — must be ≤ [`TileLimits::max_acc_tiles`].
    pub fn dst_tiles(&self) -> usize {
        self.out_subblock_h * self.out_subblock_w
    }
    /// Sites used.
    pub fn num_cores(&self) -> usize {
        self.grid_x * self.grid_y
    }
}

/// Estimated per-site cost of a tiling (lower = faster) — a heuristic RANKING model, not an absolute cycle
/// count. It captures the three levers that move blocked-matmul time: (1) **parallelism** — the per-site
/// output-tile work `per_core_m·per_core_n`, minimized by using more sites (dominant term); (2) **reuse** —
/// the number of accumulator acquires per site `(per_core_m/osh)·(per_core_n/osw)·(kt/in0_block_w)`,
/// minimized by a bigger sub-block and wider inner block; (3) **reload** — the spill/reload traffic when the
/// contraction is blocked.
pub fn tiling_cost(kt: usize, cfg: &TileConfig) -> u64 {
    let per_core_out = (cfg.per_core_m * cfg.per_core_n) as u64;
    let num_blocks = (kt / cfg.in0_block_w) as u64;
    let acquires = (cfg.per_core_m / cfg.out_subblock_h) as u64
        * (cfg.per_core_n / cfg.out_subblock_w) as u64
        * num_blocks;
    let reload = if num_blocks > 1 { acquires } else { 0 };
    per_core_out * 1_000_000 + acquires * 100 + reload
}

/// Rough local-memory footprint (in tiles) of the resident double-buffered operand blocks for `cfg` — this
/// is what bounds `in0_block_w`.
pub fn l1_block_tiles(cfg: &TileConfig) -> usize {
    2 * (cfg.per_core_m * cfg.in0_block_w + cfg.per_core_n * cfg.in0_block_w)
        + cfg.per_core_m * cfg.per_core_n
}

/// **Smart-lattice autotune**: search the divisor lattice of the tiling space — `per_core_m | mt`,
/// `per_core_n | nt`, `out_subblock_h | per_core_m`, `out_subblock_w | per_core_n`, `in0_block_w | kt` —
/// keeping only points LEGAL on this part (grid fits, the accumulator holds the sub-block, blocks fit local
/// memory), and return the minimum-[`tiling_cost`] point. This is the compiler *choosing* the config the
/// vendor hand-tunes, and it adapts to the part's limits: a larger accumulator or grid unlocks larger,
/// faster lattice points.
pub fn best_tile_config(mt: usize, nt: usize, kt: usize, limits: &TileLimits) -> TileConfig {
    // Rough local tile budget: a bf16 tile is 2 KiB, so ~ local_kib/2 tiles, held back for output/overhead.
    let budget_tiles = (limits.local_kib / 2).saturating_sub(8);
    let mut best: Option<TileConfig> = None;
    let mut best_cost = u64::MAX;
    // Loop-invariant, same as in `candidates` above: `divisors(kt)` was being rebuilt at the
    // innermost level once per (pcm, pcn, osh, osw). Same lists, same order, computed once.
    let div_mt = divisors(mt);
    let div_nt = divisors(nt);
    let div_kt = divisors(kt);
    for &pcm in div_mt.iter().filter(|&&d| mt / d <= limits.sites_y) {
        let div_pcm = divisors(pcm);
        for &pcn in div_nt.iter().filter(|&&d| nt / d <= limits.sites_x) {
            let div_pcn = divisors(pcn);
            for &osh in &div_pcm {
                for &osw in &div_pcn {
                    if osh * osw > limits.max_acc_tiles {
                        continue;
                    }
                    for &ibw in &div_kt {
                        let cfg = TileConfig {
                            grid_x: nt / pcn,
                            grid_y: mt / pcm,
                            per_core_m: pcm,
                            per_core_n: pcn,
                            in0_block_w: ibw,
                            out_subblock_h: osh,
                            out_subblock_w: osw,
                        };
                        if l1_block_tiles(&cfg) > budget_tiles {
                            continue;
                        }
                        let cost = tiling_cost(kt, &cfg);
                        if cost < best_cost {
                            best_cost = cost;
                            best = Some(cfg);
                        }
                    }
                }
            }
        }
    }
    // The lattice always contains the trivial point (1×1 sub-block, 1-wide block), so this never fails.
    best.unwrap_or(TileConfig {
        grid_x: 1,
        grid_y: 1,
        per_core_m: mt,
        per_core_n: nt,
        in0_block_w: 1,
        out_subblock_h: 1,
        out_subblock_w: 1,
    })
}

impl SpatialTarget {
    /// The part's limits in the units [`best_tile_config`] filters by, so any target can drive the lattice
    /// search without a bespoke preset of its own.
    pub fn tile_limits(&self, acc: AccPolicy, exact: bool) -> TileLimits {
        TileLimits {
            max_acc_tiles: self.effective_acc_tiles(acc, exact),
            sites_x: self.sites.x,
            sites_y: self.sites.y,
            local_kib: self.max_local_mem_bytes() / 1024,
        }
    }
}
