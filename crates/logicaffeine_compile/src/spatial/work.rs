//! THE WORK MODEL — what a program makes the machine do, counted from the program itself.
//!
//! ★ THIS IS NOT A FITTED MODEL, AND THAT IS THE POINT. Every number here is a COUNT of things the
//! choreography does — tiles read, transactions initiated, matmuls retired — walked off the same
//! `Schedule` the backends print from. There is no regression, no free parameter, and no way for the
//! model to describe a program the compiler does not emit, because it reads the emitted structure.
//!
//! It exists because the alternative was measured and found to be unusable. `ttsim_instrument_audit.json`
//! sweeps the reduction depth 8x and watches every clock on the simulator stand still: the bracket counter
//! moves 0.08%, the device's own per-core cycle register 0.3%. A cost model fitted to either is fitted to
//! noise, and the consequence was not theoretical — at 512^3 it ranked the plan issuing 2.9x the
//! instructions and 4.1x the NOC transactions FIRST, because that plan happened to score a lower clock.
//!
//! What the simulator can report is what the machine did, and the instrumented build does: Tensix
//! instructions issued, NOC transactions, NOC bytes, and matmuls RETIRED. Over that same 8x sweep those
//! move 8.11x / 7.40x / 7.59x / 8.000x. So this module PREDICTS them and
//! `spatial_work_model_lock` checks the prediction against the machine. A disagreement is a bug in the
//! compiler's model of itself — never a constant to be re-fitted away.
//!
//! ONE law is closed-form and exact, and it anchors everything else:
//!
//!   matmul_retired = mt * nt * kt * MVMUL_PER_TILE_MATMUL
//!
//! Two programs computing the same GEMM retire the same count whoever emitted them, so with it holding a
//! difference in instructions or bytes is a difference in HOW the same computation was carried out. That
//! is what makes an efficiency claim checkable rather than rhetorical.

use serde::{Deserialize, Serialize};

use super::ir::{
    walk_tagged_counted, BufferId, BufferKind, ChoreoOp, ComputeOp, SpatialProgram, StageKind,
};
use super::target::SpatialTarget;
use crate::codegen_sva::fabric_target::Precision;

/// MVMUL issues one 32x32 tile-matmul retires PER FIDELITY PASS. The matrix unit walks the tile in
/// 16 steps, and math fidelity is literally how many times it walks it: LoFi takes the operands' top
/// mantissa bits in one pass, HiFi4 makes four. So the fidelity knob is not a fitted scale factor on
/// the compute term — it is an exact integer multiplier on the work, and the machine says so.
///
/// MEASURED at 640^3, same program, three fidelities: LoFi 128,000 / HiFi2 256,000 / HiFi4 512,000
/// retired, against 8,000 tile-matmuls. 16, 32 and 64 per tile-matmul exactly.
pub const MVMUL_PER_TILE_MATMUL_PER_PASS: u64 = 16;

/// Passes the matrix unit makes over a tile at each math fidelity.
pub fn fidelity_passes(f: super::target::Fidelity) -> u64 {
    match f {
        super::target::Fidelity::LoFi => 1,
        super::target::Fidelity::HiFi2 => 2,
        super::target::Fidelity::HiFi3 => 3,
        super::target::Fidelity::HiFi4 => 4,
        // A target whose tile matmul is exact does not decompose its operands at all: one pass, full
        // precision. That is the integer/fabric engines, not a Tensix fidelity setting.
        super::target::Fidelity::Exact => 1,
    }
}

/// MVMULs one tile-matmul retires at a fidelity.
pub fn mvmul_per_tile_matmul(f: super::target::Fidelity) -> u64 {
    MVMUL_PER_TILE_MATMUL_PER_PASS * fidelity_passes(f)
}

/// What a program costs the machine, in the units the machine can actually report.
///
/// Two views, because they are two different questions and a compiler that owns a Pareto frontier has
/// to keep them apart:
///
///   * the DEVICE TOTALS are what the whole machine spends — the energy question, and the only view the
///     work meter measures, so the only one that can be validated here;
///   * the BUSIEST SITE is what one site spends — the balance question, and what a latency model on
///     silicon would be built from. It is PREDICTED STRUCTURE and is deliberately not called a time:
///     ttsim has no clock that measures the program, so nothing here may be quoted as speed.
///
/// Minimizing the totals alone would collapse a small problem onto ONE site — that genuinely does move
/// the fewest bytes — which is why the balance term exists and why the objective is stated rather than
/// assumed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    /// MVMULs that retire. Closed-form and exact; the conservation law.
    pub matmul_retired: u64,
    /// NOC transactions initiated. One per tile on an interleaved buffer, one per multicast.
    pub noc_txns: u64,
    /// Bytes moved over the NOC, DRAM traffic, multicast delivery and four-byte handshakes alike. This
    /// is what the work meter counts, so this is the one that gets validated against the machine.
    pub noc_bytes: u64,
    /// The DATA subset of `noc_bytes`: tiles, with the four-byte control writes left out.
    ///
    /// ★ THE OBJECTIVE RANKS ON THIS, NOT ON `noc_bytes`, and the reason is a pathology it caused. A
    /// semaphore atomic moves four bytes and costs a transaction SLOT; putting those four bytes into a
    /// bandwidth term let a plan win on bandwidth by eight bytes while losing on transactions by 2,160.
    /// Bandwidth and request-rate are different resources and the model keeps them apart.
    pub payload_bytes: u64,
    /// Tensix instructions issued across the device.
    pub tensix_inst: u64,
    /// Matrix-engine API invocations the choreography issues across the device.
    ///
    /// ★ THE ISSUE-COST TERM, AND IT IS A COUNT RATHER THAN A FIT. Two plans can move exactly the same
    /// bytes in exactly the same transactions and still differ enormously in what the compute engine is
    /// asked to do, because the SUB-BLOCK decides how much arithmetic one call covers: at
    /// 4096x4096x256 a 1x8 sub-block issues 16,384 `matmul_block` calls and a 1x1 sub-block issues
    /// 131,072 for identical movement — measured 1,781,904 Tensix instructions against 6,383,760, a
    /// 3.58x difference the movement axes cannot see at all.
    ///
    /// Predicting the instruction count itself would need per-call constants fitted to something; the
    /// CALL COUNT needs nothing — it is read off the same choreography as everything else, and at equal
    /// movement and equal retired arithmetic, fewer calls is unambiguously less work.
    pub compute_calls: u64,
    /// Operand tiles the matrix engines load across the device, each engine by its own rule
    /// ([`SpatialTarget::operand_loads`]). At equal calls the sub-block's SHAPE decides it: the fabric's array
    /// is fed a square sub-block's operands in fewer elements than a long one of the same area, and Tensix's
    /// unpacker, which holds one operand and re-loads the other for every held tile, the reverse.
    pub engine_operand_tiles: u64,
    /// Sites the program actually placed work on.
    pub sites_used: u64,
    /// Data bytes crossing the BUSIEST site's own network interface.
    pub site_noc_bytes: u64,
    /// Data bytes crossing the BUSIEST MOVEMENT ENGINE of any site.
    ///
    /// ★ A TENSIX SITE HAS TWO MOVEMENT ENGINES AND THEY RUN CONCURRENTLY, so what paces a site is not what
    /// it moves but what its busier engine moves. This is the term that tells apart two plans which fetch
    /// exactly the same tiles and differ only in which engine carries which stream — issuing both
    /// operand reads together leaves the store an engine to itself and serialises the reads; pairing the
    /// second operand with the store overlaps a block's write with the next block's reads. Without it
    /// the two are indistinguishable, and the compiler picked between them by coin. A site with one
    /// engine (`SiteSpec::movement_engines`) carries every stream on it.
    pub engine_noc_bytes: u64,
    /// NOC transactions the busiest site initiates.
    pub site_noc_txns: u64,
    /// MVMULs the busiest site retires.
    pub site_matmul_retired: u64,
    /// The DRAM subset of `noc_bytes`: operand reads and output writes of the tensors off the chip's SRAM tier,
    /// excluding L1-to-L1 multicast — what leaves the chip. A tensor the program holds in SRAM
    /// ([`super::residency`]) moves on chip, and is counted in `onchip_bytes` instead.
    ///
    /// ★ THIS IS THE SHARED RESOURCE, and on a GEMM it is the one that binds. Wormhole has SIX DRAM
    /// channels against sixty-four sites, so a plan whose sites each fetch their own operands puts an
    /// order of magnitude more pressure on DRAM than one that fetches each tile once and multicasts it —
    /// even though both deliver the same bytes to the same sites. A model that looked only at per-site
    /// traffic would call those two plans equal, and they are not.
    pub dram_bytes: u64,
    /// The tiles behind `dram_bytes`, whatever each tensor's width: what a lane that carries one element per
    /// cycle is paced by, where a bus is paced by the bytes.
    pub dram_tiles: u64,
    /// Tile bytes of the tensors the program holds in its SRAM tier — intermediates and carried state it keeps on
    /// chip, read and written across the crossbar without leaving it.
    pub onchip_bytes: u64,
}

impl Work {
    /// The per-axis ratio of `self` against `other`, for reporting an efficiency comparison. Only
    /// meaningful where `matmul_retired` agrees — otherwise the two did not compute the same thing.
    pub fn comparable_with(&self, other: &Work) -> bool {
        self.matmul_retired == other.matmul_retired
    }
}

/// Tiles one page of a buffer holds.
fn page_tiles(p: &SpatialProgram, b: BufferId) -> u64 {
    p.buffers
        .iter()
        .find(|x| x.id == b)
        .map(|x| match x.kind {
            BufferKind::Fifo { page_tiles, .. } | BufferKind::Interm { page_tiles, .. } => {
                page_tiles as u64
            }
            BufferKind::Accumulator { tiles } => tiles as u64,
        })
        .unwrap_or(1)
}

fn tile_bytes_of(p: &SpatialProgram, t: &SpatialTarget, b: BufferId) -> u64 {
    p.buffers
        .iter()
        .find(|x| x.id == b)
        .map(|x| t.tile_bytes(x.dtype) as u64)
        .unwrap_or(0)
}

/// What the program makes the machine do. Walks the scheduled choreography — the same structure the
/// backends print — so the model cannot drift from the code that runs.
///
/// A program without a schedule has not been through `synthesize_handshakes` and has no movement to
/// count; the compute law still holds, so that is what comes back.
///
/// A loop run to a trip counts its trip's greatest count: the work is the program's bound. [`work_at`] is the work
/// of one run.
pub fn work_of(p: &SpatialProgram, t: &SpatialTarget) -> Work {
    work_counted(p, t, &|c| c.bound())
}

/// The work of the run in which each trip `t` counts `counts[t]` — a decode step's at its position.
pub fn work_at(p: &SpatialProgram, t: &SpatialTarget, counts: &[usize]) -> Work {
    work_counted(p, t, &|c| c.at(counts))
}

fn work_counted(
    p: &SpatialProgram,
    t: &SpatialTarget,
    count: &dyn Fn(super::ir::LoopCount) -> usize,
) -> Work {
    let mut w = Work::default();
    let per_tile = mvmul_per_tile_matmul(
        t.matmul_fidelity()
            .unwrap_or(super::target::Fidelity::HiFi4),
    );
    let Some(sched) = p.schedule.as_ref() else {
        return w;
    };
    let mut per_site: std::collections::BTreeMap<super::ir::Site, Work> = Default::default();
    let mut per_engine: std::collections::BTreeMap<(super::ir::Site, super::ir::Engine), u64> =
        Default::default();
    for prog in &sched.programs {
        let before = per_site
            .get(&prog.site)
            .map(|w: &Work| w.payload_bytes)
            .unwrap_or(0);
        let sw = per_site.entry(prog.site).or_default();
        walk_tagged_counted(&prog.ops, 1, count, &mut |tag, mult| match &tag.op {
            // A tile read from an interleaved DRAM buffer is ONE transaction per tile: the pages a
            // block spans are strided across banks, so there is nothing contiguous to coalesce. That
            // is why `noc_txns` tracks tiles moved so exactly, and it is the term a sharded layout
            // would attack.
            // A transfer moves its tensor's stored bytes; the reader converts them on arrival to its buffer's
            // format, and the writer's packer converts before they leave.
            ChoreoOp::ReadTiles {
                tensor,
                block_h,
                block_w,
                ..
            } => {
                let tiles = mult * (*block_h as u64) * (*block_w as u64);
                let bytes = tiles * t.tile_bytes(p.tensor(*tensor).dtype) as u64;
                sw.noc_txns += tiles;
                sw.noc_bytes += bytes;
                sw.payload_bytes += bytes;
                if p.tensor(*tensor).space == super::ir::TensorSpace::Sram {
                    sw.onchip_bytes += bytes;
                } else {
                    sw.dram_bytes += bytes;
                    sw.dram_tiles += tiles;
                }
            }
            ChoreoOp::WriteTiles {
                tensor,
                block_h,
                block_w,
                ..
            } => {
                let tiles = mult * (*block_h as u64) * (*block_w as u64);
                let bytes = tiles * t.tile_bytes(p.tensor(*tensor).dtype) as u64;
                sw.noc_txns += tiles;
                sw.noc_bytes += bytes;
                sw.payload_bytes += bytes;
                if p.tensor(*tensor).space == super::ir::TensorSpace::Sram {
                    sw.onchip_bytes += bytes;
                } else {
                    sw.dram_bytes += bytes;
                    sw.dram_tiles += tiles;
                }
            }
            // ★ A MULTICAST IS ONE COMMAND AND IT IS COUNTED ONCE. The payload is issued once at the
            // sender's network interface and the fabric replicates it down the reserved path, so what
            // crosses the NOC is the payload — not the payload times the rectangle. That is exactly what
            // the hardware buys, and exactly why a multicast plan moves less than a blocked one: a
            // blocked plan makes every site fetch its own copy from DRAM.
            //
            // Predicting this and being wrong is what found the SIMULATOR's atomic over-count. The
            // residual was one semaphore atomic per receiver per K block, at 4,222 bytes each, because
            // the instrument was reading an atomic's instruction word as a transfer length.
            ChoreoOp::McastSend { buf, pages, .. } => {
                let per_send = (*pages as u64) * page_tiles(p, *buf) * tile_bytes_of(p, t, *buf);
                sw.noc_txns += mult * commands_for(per_send);
                sw.noc_bytes += mult * per_send;
                sw.payload_bytes += mult * per_send;
            }
            // A semaphore increment is a 32-bit read-modify-write: one command, four bytes.
            ChoreoOp::SemInc { by, .. } => {
                sw.noc_txns += mult * (*by as u64).max(1);
                sw.noc_bytes += mult * 4 * (*by as u64).max(1);
            }
            // The valid flag rides the same reserved path as the data it follows: one word, once.
            ChoreoOp::McastSetValid { .. } => {
                sw.noc_txns += mult;
                sw.noc_bytes += mult * 4;
            }
            ChoreoOp::Compute { op, .. } => {
                sw.matmul_retired += mult * retired_of(op, per_tile);
                sw.compute_calls += mult * calls_of(op, t);
                if let super::ir::ComputeOp::MatmulTiles {
                    block, granularity, ..
                } = op
                {
                    sw.engine_operand_tiles += mult * t.operand_loads(*block, *granularity);
                }
            }
            _ => {}
        });
        *per_engine.entry((prog.site, prog.engine)).or_default() +=
            per_site[&prog.site].payload_bytes - before;
    }
    // The device totals are the sum; the balance terms are the worst site, because a plan is only as
    // well spread as its busiest participant.
    for (_, s) in &per_site {
        w.matmul_retired += s.matmul_retired;
        w.noc_txns += s.noc_txns;
        w.noc_bytes += s.noc_bytes;
        w.dram_bytes += s.dram_bytes;
        w.dram_tiles += s.dram_tiles;
        w.onchip_bytes += s.onchip_bytes;
        w.payload_bytes += s.payload_bytes;
        w.compute_calls += s.compute_calls;
        w.engine_operand_tiles += s.engine_operand_tiles;
        w.site_noc_bytes = w.site_noc_bytes.max(s.payload_bytes);
        w.site_noc_txns = w.site_noc_txns.max(s.noc_txns);
        w.site_matmul_retired = w.site_matmul_retired.max(s.matmul_retired);
    }
    w.sites_used = per_site.len() as u64;
    w.engine_noc_bytes = per_engine.values().copied().max().unwrap_or(0);
    w.noc_txns += HOST_TXNS_PER_SITE * w.sites_used;
    w
}

/// NOC transactions the HOST issues per participating site, outside the kernel choreography: the program
/// dispatch handshake and the read-back of that site's cycle buffer. The choreography cannot contain them
/// — they are the runtime's, not the kernel's — but the meter counts them, so a model that predicts what
/// the meter will say has to.
///
/// It is TWO, measured exactly and at every mesh the ledger holds: 16 sites 144 -> 176, 25 sites
/// 4,400 -> 4,450, 64 sites 4,352 -> 4,480. Not a fitted constant — a count of two host operations.
pub const HOST_TXNS_PER_SITE: u64 = 2;

/// The largest payload one NOC command carries. A transfer longer than this is split by
/// `ncrisc_noc_fast_write_any_len` into burst-sized commands, each of which the network interface
/// issues — and counts — separately.
///
/// ★ THIS IS WHY A MULTICAST IS NOT ONE TRANSACTION. It is one COMMAND per burst, and a multicast page
/// is usually far larger than a burst: a 4x10 tile page is 81,920 bytes, which is ten of them. Modelling
/// a multicast as a single transaction under-counted the busiest plans by 10%, and the term was found by
/// reading `ncrisc_noc_fast_write_any_len` rather than by fitting anything.
///
/// Wormhole: `NOC_MAX_BURST_WORDS (256) * NOC_WORD_BYTES (32)`, from tt-metal's
/// `wormhole/noc/noc_parameters.h`. It belongs on the target descriptor the day a chip disagrees.
pub const NOC_MAX_BURST_BYTES: u64 = 8192;

/// NOC commands a transfer of `bytes` costs.
pub fn commands_for(bytes: u64) -> u64 {
    bytes.div_ceil(NOC_MAX_BURST_BYTES).max(1)
}

/// Matrix-engine invocations one compute op issues. The engine's block API decides how much arithmetic a single
/// call covers ([`SpatialTarget::matmul_calls`]): `matmul_block` is handed a whole `osh x osw` sub-block per
/// inner step, the fabric's array a sub-block over its whole inner block, `matmul_tiles` one tile at a time —
/// same arithmetic, `osh * osw` times the calls.
fn calls_of(op: &super::ir::ComputeOp, t: &SpatialTarget) -> u64 {
    match op {
        super::ir::ComputeOp::MatmulTiles {
            block, granularity, ..
        } => t.matmul_calls(*block, *granularity),
        _ => 0,
    }
}

/// MVMULs one compute op retires at a fidelity. Only the matrix engine retires them; the vector path
/// does not touch the matrix unit at all, so it contributes nothing to this law.
fn retired_of(op: &super::ir::ComputeOp, per_tile: u64) -> u64 {
    match op {
        // One invocation covers ONE sub-block over ONE inner block: `osh * osw` output tiles accumulating
        // over `ibw` inner steps, whether the engine is handed it a sub-block at a time (`matmul_block`)
        // or a tile at a time (`matmul_tiles`) — the granularity changes the call count, never the
        // arithmetic. The surrounding choreography loops over the sub-blocks and the K blocks, and
        // `walk_ops` carries those multiplicities in.
        super::ir::ComputeOp::MatmulTiles { block, .. } => {
            (block.osh as u64) * (block.osw as u64) * (block.ibw as u64) * per_tile
        }
        _ => 0,
    }
}

/// ★ THE FLOOR THE WHOLE MODEL IS ANCHORED ON: the fewest MVMULs a GEMM of this shape CAN retire at
/// this fidelity — the tile-matmuls the mathematics demands, times the passes the fidelity costs.
///
/// It is a floor and not an identity, and the difference is the point. A program that retires MORE is
/// doing arithmetic the problem does not require, and that is a real and measurable inefficiency rather
/// than a different way of being right. Measured at 640^3: every program this compiler emits, and every
/// tt-metal example, retires exactly this floor — 8,000 tile-matmuls. The vendor's own default heuristic
/// retires 8,820, because it rounds 20 tiles up to 21 in two of the three dimensions: **10.25% more
/// arithmetic than the problem contains**, at every fidelity.
///
/// Tile padding IS part of the floor: a 33-row operand occupies two tile rows and the machine pays for
/// both. What is not part of it is padding a program adds on top of that.
pub fn gemm_retired_floor(
    m: usize,
    n: usize,
    k: usize,
    tile: super::target::TileShape,
    f: super::target::Fidelity,
) -> u64 {
    (m.div_ceil(tile.rows) as u64)
        * (n.div_ceil(tile.cols) as u64)
        * (k.div_ceil(tile.cols) as u64)
        * mvmul_per_tile_matmul(f)
}

/// The tile widths a GEMM's movement is priced at. A transfer moves its tensor's stored bytes, so each operand
/// is read at its own tensor's width and the result written at the output's — an exact accumulator writes a
/// wider result than the operands it reads (fp32 under bf16, int32 or int64 under int8) — and a multicast
/// moves the sender's buffer, at that buffer's width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GemmWidths {
    pub a: u64,
    pub b: u64,
    pub a_buffer: u64,
    pub b_buffer: u64,
    pub out: u64,
}

impl GemmWidths {
    /// The widths of `p`'s GEMM on `t`: the tensors its matrix stage's operand buffers are loaded from (an
    /// operand no stage loads moves at its buffer's width), those buffers, and the output. `None` for a
    /// program with no two-operand tile matmul or no output.
    pub fn of(p: &SpatialProgram, t: &SpatialTarget) -> Option<GemmWidths> {
        let bytes = |d: Precision| t.tile_bytes(d) as u64;
        let buffer = |b: BufferId| p.buffers.iter().find(|x| x.id == b).map(|x| x.dtype);
        let loaded = |b: BufferId| {
            p.stages.iter().find_map(|s| match &s.kind {
                StageKind::Load { tensor, .. } if s.outs.contains(&b) => {
                    Some(p.tensor(*tensor).dtype)
                }
                _ => None,
            })
        };
        let out = bytes(p.output_tensor()?.dtype);
        p.stages.iter().find_map(|s| match &s.kind {
            StageKind::Compute(ComputeOp::MatmulTiles { .. }) if s.ins.len() == 2 => {
                let (ab, bb) = (buffer(s.ins[0])?, buffer(s.ins[1])?);
                Some(GemmWidths {
                    a: bytes(loaded(s.ins[0]).unwrap_or(ab)),
                    b: bytes(loaded(s.ins[1]).unwrap_or(bb)),
                    a_buffer: bytes(ab),
                    b_buffer: bytes(bb),
                    out,
                })
            }
            _ => None,
        })
    }
}

/// ★ THE SAME WORK, PRICED FROM A PLAN INSTEAD OF A BUILT PROGRAM.
///
/// [`work_of`] walks a scheduled choreography, which is exact but needs the whole pipeline to have run.
/// A plan search has thousands of candidates — 4,209 at 512^3 — and cannot build them all, so it needs
/// the same quantity as a closed form over the plan.
///
/// These are not two models. `spatial_work_model_lock::the_closed_form_is_the_built_choreography_on_*`
/// requires this to equal [`work_of`] of the program [`build_candidate`](super::pipeline::build_candidate)
/// emits, count for count, on every target, and [`work_of`] is in turn checked against the machine. The chain
/// is closed form == choreography == simulator, each link tested, so the planner ranks by a number the
/// hardware has already agreed to.
///
/// Everything it prices with is read off the GEMM template `p` and its target `t` — the tile extents, the
/// widths ([`GemmWidths`]), the matrix engine's fidelity, the sites and the movement engines per site — so no
/// caller can hand it a width or an engine count the emitter does not use. `None` when `p` has no GEMM.
///
/// The movement law, read off the choreography and confirmed at every shape in the ledger:
///
/// * A blocked plan makes every site fetch its own operands: `nx*mt*kt + ny*nt*kt` tiles read.
/// * A multicast plan reads each shared operand tile from DRAM ONCE and multicasts it ONCE, so it pays
///   for that tile TWICE on the NOC and only twice. That is the whole win: a blocked plan pays `nx` and
///   `ny` times instead of two.
/// * Every plan writes its output once: `mt*nt`.
/// * A streaming plan re-reads both operands for every output tile: `2*mt*nt*kt`.
pub fn work_of_plan(
    p: &SpatialProgram,
    plan: &super::ir::BlockPlan,
    fused: bool,
    t: &SpatialTarget,
) -> Option<Work> {
    let widths = GemmWidths::of(p, t)?;
    let out = p.output_tensor()?;
    let (mt, nt, kt) = (out.tiles.rows_t, out.tiles.cols_t, p.work.k_tiles);
    let fidelity = match t.matmul_cap() {
        Some(super::target::ComputeCap::MatmulTile { fidelity, .. }) => *fidelity,
        _ => super::target::Fidelity::Exact,
    };
    let sites = t.sites.sites();
    let two_engines = t.site.movement_engines >= 2;
    let (mt64, nt64, kt64) = (mt as u64, nt as u64, kt as u64);
    let per_tile = mvmul_per_tile_matmul(fidelity);
    let retired = mt64 * nt64 * kt64 * per_tile;
    if super::cost::is_streaming(plan) {
        let items = (mt64 * nt64).div_ceil((sites as u64).max(1));
        let tiles = 2 * mt64 * nt64 * kt64 + mt64 * nt64;
        let bytes = mt64 * nt64 * (kt64 * (widths.a + widths.b) + widths.out);
        let reads = items * kt64 * (widths.a + widths.b);
        let store = items * widths.out;
        return Some(Work {
            matmul_retired: retired,
            compute_calls: (mt64 * nt64 * kt64).max(1) / (plan.block.ibw as u64).max(1)
                * t.matmul_calls(plan.block, plan.granularity),
            engine_operand_tiles: (mt64 * nt64 * kt64).max(1) / (plan.block.ibw as u64).max(1)
                * t.operand_loads(plan.block, plan.granularity),
            noc_txns: tiles + HOST_TXNS_PER_SITE * (mt64 * nt64).min(sites as u64).max(1),
            noc_bytes: bytes,
            payload_bytes: bytes,
            dram_bytes: bytes,
            dram_tiles: tiles,
            onchip_bytes: 0,
            tensix_inst: 0,
            sites_used: (mt64 * nt64).min(sites as u64).max(1),
            site_noc_bytes: reads + store,
            // A streaming site reads its two operands on one engine and stores on the other, so the
            // reads pace it; a site with one engine carries both.
            engine_noc_bytes: if two_engines {
                reads.max(store)
            } else {
                reads + store
            },
            site_noc_txns: items * (2 * kt64 + 1),
            site_matmul_retired: items * kt64 * per_tile,
        });
    }
    let (ny, nx) = plan.grid;
    let (ny64, nx64) = (ny as u64, nx as u64);
    let (pm, pn) = (plan.per_site_m as u64, plan.per_site_n as u64);
    let ibw = plan.block.ibw as u64;
    let blocks = (ny64 * nx64).max(1);
    let items = blocks.div_ceil((sites as u64).max(1));
    // Which operand is shared is a property of the placement the passes chose, never of a named
    // strategy: A is multicast across a grid ROW when there is more than one column to share it with.
    let a_shared = fused && nx64 > 1;
    let b_shared = fused && ny64 > 1;
    let a_dram = if a_shared {
        mt64 * kt64
    } else {
        nx64 * mt64 * kt64
    };
    let b_dram = if b_shared {
        nt64 * kt64
    } else {
        ny64 * nt64 * kt64
    };
    let a_mcast = if a_shared { mt64 * kt64 } else { 0 };
    let b_mcast = if b_shared { nt64 * kt64 } else { 0 };
    let dram_tiles = a_dram + b_dram + mt64 * nt64;
    let dram_bytes = a_dram * widths.a + b_dram * widths.b + mt64 * nt64 * widths.out;
    let payload_bytes = dram_bytes + a_mcast * widths.a_buffer + b_mcast * widths.b_buffer;
    // A multicast rectangle costs ONE transaction whatever its payload, which is why a multicast plan's
    // transaction count falls further than its byte count does. Three things cross the NOC per block:
    // the payload, the valid flag that follows it down the same reserved path, and — from every RECEIVER
    // back to its sender — the ready atomic that says the last block has been consumed. That last term
    // is the one a byte-only reading of the plan misses, and it dominates: sixteen groups of eight sites
    // at 512^3 is 112 atomics per block against 32 sends.
    let nb = (kt / plan.block.ibw.max(1)).max(1) as u64;
    // The two operands have DIFFERENT panel sizes whenever the per-site block is not square — A is
    // `per_site_m x ibw`, B is `ibw x per_site_n` — so they split into different numbers of bursts and
    // charging both at the larger over-counts. (Measured: 4,352 predicted against 4,224 at 2048x1024x512,
    // where the block is 8x4.)
    let handshakes = if fused {
        (a_shared as u64) * ny64 * (nx64 - 1) + (b_shared as u64) * nx64 * (ny64 - 1)
    } else {
        0
    };
    // Each payload is split into burst-sized commands; the valid flag that follows it down the same
    // reserved path is one word, so one command.
    // ★ PRICED PER GRID ROW AND COLUMN, because a boundary block's panel is SMALLER and splits into
    // fewer bursts. `BlockPlan::site_block` is the one definition of what each site covers, and `place`
    // emits its patterns from the same function — a second copy of that rule would drift, and a planner
    // that ranks by a closed form the emitter does not honour is exactly the defect already removed from
    // `size_buffers`. On a grid whose extents divide, every row is the nominal block and these sums are
    // `ny * (...)` and `nx * (...)` exactly as before.
    let a_bursts: u64 = if a_shared {
        (0..ny)
            .map(|i| commands_for(plan.site_block(mt, nt, i, 0).0 as u64 * ibw * widths.a_buffer))
            .sum()
    } else {
        0
    };
    let b_bursts: u64 = if b_shared {
        (0..nx)
            .map(|j| commands_for(ibw * plan.site_block(mt, nt, 0, j).1 as u64 * widths.b_buffer))
            .sum()
    } else {
        0
    };
    let flags = (a_shared as u64) * ny64 + (b_shared as u64) * nx64;
    // The payload's bytes are its tiles'; only the flags and the ready atomics are four-byte words.
    let control = (flags + handshakes) * nb;
    let mcast_commands = (a_bursts + b_bursts) * nb + control;
    // ★ THE BUSIEST SITE. Every site writes its own output block; what it READS depends on whether it
    // fetches its operands or is handed them. In a multicast plan the counter sits at the ISSUER, so a
    // receiver initiates almost nothing and the pace is set by a SENDER — and the busiest sender is the
    // one at the CORNER, which sends A down its row AND B down its column, reading each from DRAM and
    // multicasting each. Assuming one operand there under-counted it by half, which the choreography
    // walk caught: 68 tiles predicted against 132 actually issued at 512^3.
    //
    // Without this term at all the search would collapse a small problem onto one site, which genuinely
    // does move the fewest bytes in total and is obviously wrong.
    // An operand the corner site SENDS costs it twice — once to read from DRAM, once to multicast — and
    // an operand it merely fetches for itself costs it once. Sharing is per-axis, so this is too: a plan
    // that multicasts A down its rows but has only one grid row to share B along pays double for A and
    // single for B, and assuming both would over-charge it.
    let a_part = pm * kt64 * (widths.a + if a_shared { widths.a_buffer } else { 0 }) * items;
    let b_part = pn * kt64 * (widths.b + if b_shared { widths.b_buffer } else { 0 }) * items;
    let store = pm * pn * widths.out * items;
    // Which engine carries which stream. Loading both operands together leaves the store an engine to
    // itself; pairing the second operand with the store overlaps this block's write with the next
    // block's reads. The busier engine paces the site, and a site with one engine carries every stream.
    let engine_noc_bytes = match (two_engines, plan.engines) {
        (false, _) => a_part + b_part + store,
        (true, super::ir::EnginePacking::LoadsTogether) => (a_part + b_part).max(store),
        (true, super::ir::EnginePacking::OperandWithStore) => a_part.max(b_part + store),
    };
    // The corner's transactions: a tile read or written is one, a multicast is one per burst and one for
    // its flag. A multicast plan's grid fits the mesh, so the corner carries one block.
    let corner_sends = nb
        * ((a_shared as u64) * (commands_for(pm * ibw * widths.a_buffer) + 1)
            + (b_shared as u64) * (commands_for(ibw * pn * widths.b_buffer) + 1));
    let site_noc_txns = (pm * kt64 + pn * kt64 + pm * pn) * items + corner_sends;
    // Invocations of the matrix engine and the operand tiles it loads, per sub-block per K block, by its block
    // API (`SpatialTarget::matmul_calls`, `SpatialTarget::operand_loads`), summed over the sites that actually
    // carry work: a boundary site walks fewer sub-blocks, and on a dividing grid every site is the nominal block.
    let (compute_calls, engine_operand_tiles) = plan
        .site_blocks(mt, nt)
        .iter()
        .map(|(_, (h, w, osh, osw))| {
            let sub = (*h as u64 / (*osh as u64).max(1)) * (*w as u64 / (*osw as u64).max(1));
            let block = super::ir::BlockShape {
                osh: *osh,
                osw: *osw,
                ibw: plan.block.ibw,
            };
            (
                sub * nb * t.matmul_calls(block, plan.granularity),
                sub * nb * t.operand_loads(block, plan.granularity),
            )
        })
        .fold((0u64, 0u64), |(c, l), (dc, dl)| (c + dc, l + dl));
    let (compute_calls, engine_operand_tiles) =
        (compute_calls * items, engine_operand_tiles * items);
    Some(Work {
        matmul_retired: retired,
        compute_calls,
        engine_operand_tiles,
        noc_txns: dram_tiles
            + mcast_commands
            + HOST_TXNS_PER_SITE * blocks.min((sites as u64).max(1)),
        noc_bytes: payload_bytes + 4 * control,
        payload_bytes,
        dram_bytes,
        dram_tiles,
        onchip_bytes: 0,
        tensix_inst: 0,
        sites_used: blocks.min((sites as u64).max(1)),
        site_noc_bytes: a_part + b_part + store,
        engine_noc_bytes,
        site_noc_txns,
        site_matmul_retired: pm * pn * kt64 * items * per_tile,
    })
}

/// ★ WHAT THE COMPILER IS MINIMIZING, SAID OUT LOUD — and the knob for changing it.
///
/// This is a POLICY, not a fitted weight, and the distinction is the whole point. The lattice of legal
/// plans is enormous (4,209 at 512^3) and every one of them computes the same answer; the conservation
/// law says they even retire the same arithmetic. What separates them is how much of the machine they
/// spend carrying operands to that arithmetic, and WHICH of those costs matters is a property of the
/// deployment, not of the mathematics. So it is named, defaulted, and overridable — never inferred, and
/// never a constant fitted to a simulator.
///
/// It is not a strategy enum. It names no plan, tiling or arrangement; it names an axis to minimize, and
/// which plan wins is still whatever the search finds. Two objectives over the same lattice pick two
/// different plans and a human wrote down neither of them.
///
/// ★ THE FRONTIER IS REAL AND IT IS THE POINT. `Balanced` and `Energy` genuinely disagree: the plan that
/// spends the least machine in total is not always the one that spreads best across it, and a compiler
/// that pretends otherwise is hiding a trade-off rather than owning it.
/// A plan's rank under an [`Objective`]: exact counts compared in order, most significant first.
pub type RankKey = [u128; 7];

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Objective {
    /// The shared resource, then the arithmetic spread, then the movement spread, then transactions.
    ///
    /// The default, and here is the whole argument, in the order the terms bind.
    ///
    ///   1. **DRAM is SHARED.** Wormhole has six channels against sixty-four sites, so a plan whose
    ///      sites each fetch their own operands leans on the one resource nobody can add more of. It
    ///      leads.
    ///   2. **The busiest site's ARITHMETIC.** A plan that reads DRAM once and then does all the
    ///      multiplying on a handful of cores ties on the first term and is obviously wrong — it is not
    ///      using the machine. This is why the fabric has sixty-four sites at all, so it comes second,
    ///      and it is what refuses the single-site plan that minimizing traffic alone walks straight
    ///      into.
    ///   3. **The busiest MOVEMENT ENGINE.** Two sites can move the same tiles and still differ: issuing
    ///      both operand reads on one engine serialises them, pairing the second with the store overlaps
    ///      a block's write with the next block's reads. The busier engine paces the site.
    ///   4. **The matrix engine's calls, then the operand tiles it loads.** Same bytes, same arithmetic: a
    ///      sub-block that covers eight tiles per call asks for an eighth of the calls a one-tile sub-block
    ///      does, and at equal calls the sub-block's shape decides what the engine loads, by its own rule.
    ///   5. **Transactions.** Same tiles, same engines, different block depth — this is what is left, and
    ///      it is last because it is the axis the model knows least about.
    ///
    /// Nothing here is tuned. It is lexicographic over exact integer counts and invents no rate, no
    /// bandwidth and no energy constant; the only machine facts it leans on are that DRAM is shared,
    /// that sites are not, and how many movement engines a site has.
    ///
    /// It ranks only admissible plans: a ragged grid is one only where no evenly divided plan beats it on
    /// every predicted total ([`admissible`](crate::spatial::passes::tile::admissible)).
    #[default]
    Balanced,
    /// The whole device's movement, whatever the spread.
    ///
    /// ★ AND IT COLLAPSES ONTO ONE SITE, which is worth stating rather than hiding. A single site reads
    /// each operand tile exactly once — that IS the traffic minimum — so asking only for less traffic
    /// asks for no parallelism. Measured at 512^3: this picks a 16x16 per-site block on ONE core,
    /// 1,572,864 bytes against `Balanced`'s 2,621,952. It is a real corner of the frontier and it is
    /// occasionally what someone wants (a batch of tiny problems, one per core); it is not a default,
    /// and calling it "energy" would have been a lie, because energy is power times TIME.
    Traffic,
    /// Transactions first, for a NOC limited by requests in flight rather than by bytes.
    Transactions,
    /// Issue slots first, for a fabric whose instruction bandwidth binds before its NOC does.
    Issue,
    /// An explicit per-event weighting — it takes its numbers from the caller rather than from a fit.
    Weighted {
        per_byte: f64,
        per_txn: f64,
        per_inst: f64,
    },
    /// The roofline: the plan whose slowest resource finishes first, each count of the work over the cited
    /// roof of the resource it loads ([`super::roofs::Rates`]), with `Balanced`'s key as the exact tiebreak.
    ///
    /// It prices what `Balanced` only orders. `Balanced` puts DRAM bytes ahead of the busiest site's
    /// arithmetic whatever the two cost; the roofline asks which of them the chip will wait on, at rates read
    /// from the vendor's own documents, so a plan that moves a few more bytes to spread the arithmetic wins
    /// exactly when the arithmetic is the bound. A resource with no roof is not priced. The cycles are rounded
    /// up to a whole cycle, so the rates decide which plans tie and the counts order the ties exactly.
    Roofline(super::roofs::Rates),
}

impl Objective {
    /// The roofline objective for programs whose traffic and operands are `element`s on `target`.
    pub fn roofline(
        target: &SpatialTarget,
        element: crate::codegen_sva::fabric_target::Precision,
    ) -> Objective {
        Objective::Roofline(super::roofs::Rates::of(target, element))
    }

    /// Rank a plan's work; lower is better: exact counts compared in order, most significant first, each in a
    /// place of its own and the unused places zero. The integer variants compare EXACTLY — no float rounding
    /// ever decides a plan — the weighted one is scaled and rounded once, at the end, and the roofline's
    /// bound is rounded up to a whole cycle before the exact counts break its ties.
    pub fn key(self, w: &Work) -> RankKey {
        let n = |x: u64| x as u128;
        match self {
            // Movement settled, the engine's own call count decides: same bytes, same transactions, same
            // arithmetic — but a sub-block that covers eight tiles per call asks the machine for an eighth of
            // the work a one-tile sub-block does. At equal calls, what the engine loads for them.
            Objective::Balanced => [
                n(w.dram_bytes),
                n(w.site_matmul_retired),
                n(w.engine_noc_bytes),
                n(w.compute_calls),
                n(w.engine_operand_tiles),
                n(w.noc_txns),
                0,
            ],
            Objective::Traffic => [
                n(w.payload_bytes),
                n(w.dram_bytes),
                n(w.noc_txns),
                n(w.site_matmul_retired),
                0,
                0,
                0,
            ],
            Objective::Transactions => [
                n(w.noc_txns),
                n(w.payload_bytes),
                n(w.site_matmul_retired),
                n(w.engine_noc_bytes),
                0,
                0,
                0,
            ],
            Objective::Issue => [
                n(w.compute_calls),
                n(w.payload_bytes),
                n(w.site_matmul_retired),
                n(w.engine_noc_bytes),
                0,
                0,
                0,
            ],
            Objective::Weighted {
                per_byte,
                per_txn,
                per_inst,
            } => {
                let s = per_byte * w.noc_bytes as f64
                    + per_txn * w.noc_txns as f64
                    + per_inst * w.tensix_inst as f64;
                [
                    (s * 1e6).round().max(0.0) as u128,
                    n(w.site_matmul_retired),
                    0,
                    0,
                    0,
                    0,
                    0,
                ]
            }
            Objective::Roofline(rates) => {
                let cycles = rates.roofline(w).cycles().ceil().max(0.0) as u128;
                let b = Objective::Balanced.key(w);
                [cycles, b[0], b[1], b[2], b[3], b[4], b[5]]
            }
        }
    }

    /// Every objective, for reporting the frontier rather than a single winner.
    pub const ALL: [Objective; 4] = [
        Objective::Balanced,
        Objective::Traffic,
        Objective::Transactions,
        Objective::Issue,
    ];
}
