//! storage_controller — the deterministic NAND storage controller (the storage analog
//! of [`super::dram`]).
//!
//! The memory controller proves that timing can be *guaranteed*: DRAM timing is
//! deterministic by spec, so the compiler schedules a command stream, an independent
//! checker proves it legal, and a device model delivers exactly the promised beat cycles
//! — zero jitter, refresh co-scheduled and never a surprise. Storage looks impossible at
//! first because a commodity SSD's FTL runs garbage-collection / wear-leveling / erase
//! *reactively and hidden* (that is SSD tail latency). The fix is the SAME move as DRAM
//! refresh: make every reactive background event a SCHEDULED event the compiler owns.
//! That requires HOST-MANAGED flash (NVMe ZNS / open-channel): append-only zones (no
//! device-side GC surprise) and an exposed channel×die geometry (schedulable
//! parallelism), so a host-scheduled block erase is the flash analog of REFpb.
//!
//! This module makes that a checked, executable fact:
//!
//!   • [`FlashTimings`] — a ZNS-TLC-class timing table (one struct of DATA, so the flash
//!     architect's corrections drop in as numbers, not code);
//!   • [`schedule_zoned`] — the compile-time command scheduler: zone/gather/reset
//!     descriptors → a fully TIMED NAND command stream, die-interleaved and channel-
//!     striped, with block erases placed into idle die windows (never displacing a read);
//!   • [`check_legal_flash`] — an INDEPENDENT per-command legality checker (per-die busy
//!     serialization, per-channel data-bus occupancy, tR / tPROG / tBERS) — the scheduler
//!     never grades its own homework;
//!   • [`FlashDeviceModel`] — a die/channel-state device that EXECUTES a timed stream and
//!     reports every read's completion cycle: the zero-jitter fact is `planned == executed`;
//!   • [`execute_with_suspend`] / [`black_box_execute`] — the Tier-B (bounded, erase-
//!     suspend) and Tier-C (black-box reactive FTL) executors that prove the determinism
//!     ladder: bounded jitter under a budgeted suspend, and the IMPOSSIBILITY of any static
//!     guarantee once reclamation timing is not host-owned.
//!
//! Everything is a pure function of its inputs — deterministic by construction, like the
//! rest of the stack, and the certified-SAT tR / erase-collision obligations live in the
//! test battery beside the executor RTL.

/// One NAND operation on a die.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlashOp {
    /// Page sense (tR) then data-out over the channel.
    Read,
    /// Data-in over the channel then page program (tPROG).
    Program,
    /// Block erase (tBERS) — the host-scheduled reclamation event (the REFpb analog).
    Erase,
}

/// One timed NAND command. `chan` is the shared data-bus this die hangs off; `die` is the
/// global die index (its own serializing resource).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FlashCmd {
    pub op: FlashOp,
    pub chan: u32,
    pub die: u32,
}

/// A command with its planned issue cycle. The PLAN is the artifact: the certified SAT
/// proves windows of it timing-legal, and the executor RTL runs it in order.
#[derive(Clone, Copy, Debug)]
pub struct TimedCmd {
    pub at: u64,
    pub cmd: FlashCmd,
}

/// ZNS-TLC-class NAND timing + geometry, in CONTROLLER CLOCK CYCLES. Engineering-realistic
/// ratios (read : program : erase ≈ 1 : 10 : 100); the table is DATA for the flash
/// architect to correct, and every measurement re-derives from it.
#[derive(Clone, Debug)]
pub struct FlashTimings {
    /// Independent data-bus channels.
    pub channels: u32,
    /// Dies per channel — the parallelism that lets one die erase while others read.
    pub dies_per_channel: u32,
    /// Planes per die (multi-plane program/erase; reads serialize on the die here).
    pub planes_per_die: u32,
    /// Pages per erase block.
    pub pages_per_block: u32,
    /// Pages per zone (append-only unit; a zone reset erases its blocks).
    pub zone_pages: u64,
    /// Page read (sense) latency.
    pub t_r: u64,
    /// Page program latency.
    pub t_prog: u64,
    /// Block erase latency (~10³× a read — the tail source we schedule away).
    pub t_bers: u64,
    /// Channel cycles to move one page of data in/out.
    pub data_cycles: u64,
    /// Max cycles a program/erase may be SUSPENDED to let a read through (Tier-B budget).
    pub suspend_window: u64,
    /// Bounded worst-case extra latency of a soft-LDPC read-retry (Tier-B envelope).
    pub ldpc_retry_worst: u64,
}

impl FlashTimings {
    /// A host-managed ZNS-TLC-class part: 8 channels × 8 dies (enough die depth to hide the
    /// sense latency behind the channel data-out), 100× erase.
    pub fn zns_tlc() -> Self {
        FlashTimings {
            channels: 8,
            dies_per_channel: 8,
            planes_per_die: 2,
            pages_per_block: 384,
            zone_pages: 12288,
            t_r: 64,
            t_prog: 640,
            t_bers: 6400,
            data_cycles: 8,
            suspend_window: 96,
            ldpc_retry_worst: 32,
        }
    }

    /// A pSLC fast-path part: faster sense/program/erase, tighter suspend — the crown-tier
    /// medium where erases fit in per-die idle gaps with room to spare.
    pub fn pslc_fast() -> Self {
        FlashTimings {
            channels: 8,
            dies_per_channel: 8,
            planes_per_die: 2,
            pages_per_block: 256,
            zone_pages: 8192,
            t_r: 16,
            t_prog: 160,
            t_bers: 1600,
            data_cycles: 8,
            suspend_window: 32,
            ldpc_retry_worst: 8,
        }
    }

    /// Total independent dies.
    pub fn n_dies(&self) -> u32 {
        self.channels * self.dies_per_channel
    }

    /// The worst-case read latency the tier promises: sense + data-out, plus (Tier B) a
    /// budgeted suspend and a bounded LDPC retry. A read never exceeds this.
    pub fn read_latency_bound(&self) -> u64 {
        self.t_r + self.data_cycles + self.suspend_window + self.ldpc_retry_worst
    }
}

/// Global page → (channel, die): consecutive pages rotate across ALL dies so a sequential
/// stream interleaves — one die senses while the next streams data, and idle dies are free
/// to erase. A compile-time-known mapping the schedule depends on.
pub fn map_page(t: &FlashTimings, page: u64) -> (u32, u32) {
    let g = (page % t.n_dies() as u64) as u32;
    let chan = g % t.channels;
    (chan, g)
}

/// A compile-time transfer descriptor — the vocabulary the static schedule hands to the
/// storage edge: `count` consecutive pages from global page `base_page`, read or programmed.
#[derive(Clone, Debug)]
pub struct ZoneDesc {
    pub zone: u64,
    pub base_page: u64,
    pub count: u64,
    pub write: bool,
}

/// The random tail: explicit page reads (a pointer-chase / index lookup).
#[derive(Clone, Debug)]
pub struct Gather {
    pub pages: Vec<u64>,
}

/// A host-scheduled zone reset: erase every die's block backing the zone — the reclamation
/// event, placed into idle windows by the scheduler (the REFpb analog).
#[derive(Clone, Copy, Debug)]
pub struct ZoneReset {
    pub zone: u64,
}

/// How the access list is ordered into the plan.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VisitOrder {
    /// Descriptors back-to-back in declaration order (single-stream kernels).
    ProgramOrder,
    /// Round-robin one page from each stream — the multi-stream interleave.
    Interleaved,
}

fn access_list(zones: &[ZoneDesc], gathers: &[Gather], order: VisitOrder) -> Vec<(FlashOp, u64)> {
    let mut out: Vec<(FlashOp, u64)> = Vec::new();
    let per_zone: Vec<Vec<(FlashOp, u64)>> = zones
        .iter()
        .map(|z| {
            let op = if z.write {
                FlashOp::Program
            } else {
                FlashOp::Read
            };
            (0..z.count).map(|i| (op, z.base_page + i)).collect()
        })
        .collect();
    match order {
        VisitOrder::ProgramOrder => {
            for z in &per_zone {
                out.extend(z.iter().copied());
            }
        }
        VisitOrder::Interleaved => {
            let mut idx = vec![0usize; per_zone.len()];
            loop {
                let mut any = false;
                for (s, z) in per_zone.iter().enumerate() {
                    if idx[s] < z.len() {
                        out.push(z[idx[s]]);
                        idx[s] += 1;
                        any = true;
                    }
                }
                if !any {
                    break;
                }
            }
        }
    }
    for g in gathers {
        for &p in &g.pages {
            out.push((FlashOp::Read, p));
        }
    }
    out
}

/// A zone's DIE WINDOW. Real ZNS zones occupy a subset of the dies, so reads on one zone and a
/// reset on a DISJOINT zone can run CONCURRENTLY without ever colliding — the system-level form
/// of "GC is free". [`Placement::full`] is the classic all-dies stripe.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub die_base: u32,
    pub die_count: u32,
}

impl Placement {
    pub fn full(t: &FlashTimings) -> Self {
        Placement {
            die_base: 0,
            die_count: t.n_dies(),
        }
    }
    /// Map a zone-local page to (channel, global die) within this window.
    fn map(&self, t: &FlashTimings, local: u64) -> (u32, u32) {
        let die = self.die_base + (local % self.die_count.max(1) as u64) as u32;
        (die % t.channels, die)
    }
    fn dies(&self) -> std::ops::Range<u32> {
        self.die_base..self.die_base + self.die_count
    }
}

/// The shared scheduling core: lay a stream of pre-mapped `(op, chan, die)` commands out die-
/// interleaved + channel-striped (a die senses while its channel-mates stream data), then place
/// one trailing block erase per entry in `erase_dies` at that die's idle point. Time-sorted.
fn schedule_ops(
    t: &FlashTimings,
    ops: &[(FlashOp, u32, u32)],
    erase_dies: &[u32],
) -> Vec<TimedCmd> {
    let nd = t.n_dies() as usize;
    let nc = t.channels as usize;
    let mut die_free = vec![0u64; nd];
    let mut chan_free = vec![0u64; nc];
    let mut out: Vec<TimedCmd> = Vec::new();

    for &(op, chan, die) in ops {
        let (ci, gi) = (chan as usize, die as usize);
        match op {
            FlashOp::Read => {
                // Align sense so the data-out lands exactly when the channel frees.
                let mut at = die_free[gi];
                if chan_free[ci] > at + t.t_r {
                    at = chan_free[ci] - t.t_r;
                }
                at = at.max(die_free[gi]);
                out.push(TimedCmd {
                    at,
                    cmd: FlashCmd { op, chan, die },
                });
                die_free[gi] = at + t.t_r;
                chan_free[ci] = at + t.t_r + t.data_cycles;
            }
            FlashOp::Program => {
                // Data-in over the channel first, then program the die.
                let at = die_free[gi].max(chan_free[ci]);
                out.push(TimedCmd {
                    at,
                    cmd: FlashCmd { op, chan, die },
                });
                chan_free[ci] = at + t.data_cycles;
                die_free[gi] = at + t.data_cycles + t.t_prog;
            }
            FlashOp::Erase => unreachable!("erases come from erase_dies"),
        }
    }

    // Reclamation: an erase per `erase_dies` entry, placed at that die's idle point AFTER its
    // own ops — in the plan at a known time, never a surprise. A die is serial, so an erase on a
    // die can never overlap that die's reads; an erase on a die with NO reads is free from t=0.
    for &die in erase_dies {
        let gi = die as usize;
        let chan = die % t.channels;
        let at = die_free[gi];
        out.push(TimedCmd {
            at,
            cmd: FlashCmd {
                op: FlashOp::Erase,
                chan,
                die,
            },
        });
        die_free[gi] = at + t.t_bers;
    }

    out.sort_by(|a, b| a.at.cmp(&b.at));
    out
}

/// The compile-time scheduler (full-stripe placement): read/program streams laid down die-
/// interleaved and channel-striped, then reset erases placed into each die's trailing idle
/// window. Returns the timed plan, time-sorted.
pub fn schedule_zoned(
    t: &FlashTimings,
    zones: &[ZoneDesc],
    gathers: &[Gather],
    resets: &[ZoneReset],
    order: VisitOrder,
) -> Vec<TimedCmd> {
    let nd = t.n_dies();
    let ops: Vec<(FlashOp, u32, u32)> = access_list(zones, gathers, order)
        .into_iter()
        .map(|(op, page)| {
            let (chan, die) = map_page(t, page);
            (op, chan, die)
        })
        .collect();
    let mut erase_dies: Vec<u32> = Vec::new();
    for _ in resets {
        erase_dies.extend(0..nd);
    }
    schedule_ops(t, &ops, &erase_dies)
}

/// Placement-aware scheduler: each read/program stream and each reset targets a die WINDOW
/// ([`Placement`]). A reset on a window DISJOINT from the read window runs concurrently with the
/// reads and provably cannot collide with them — concurrent, zero-jitter reclamation.
pub fn schedule_placed(
    t: &FlashTimings,
    reads: &[(ZoneDesc, Placement)],
    resets: &[(ZoneReset, Placement)],
    order: VisitOrder,
) -> Vec<TimedCmd> {
    let per_stream: Vec<Vec<(FlashOp, u32, u32)>> = reads
        .iter()
        .map(|(z, p)| {
            let op = if z.write {
                FlashOp::Program
            } else {
                FlashOp::Read
            };
            (0..z.count)
                .map(|i| {
                    let (chan, die) = p.map(t, z.base_page + i);
                    (op, chan, die)
                })
                .collect()
        })
        .collect();
    let mut ops: Vec<(FlashOp, u32, u32)> = Vec::new();
    match order {
        VisitOrder::ProgramOrder => {
            for s in &per_stream {
                ops.extend(s.iter().copied());
            }
        }
        VisitOrder::Interleaved => {
            let mut idx = vec![0usize; per_stream.len()];
            loop {
                let mut any = false;
                for (s, stream) in per_stream.iter().enumerate() {
                    if idx[s] < stream.len() {
                        ops.push(stream[idx[s]]);
                        idx[s] += 1;
                        any = true;
                    }
                }
                if !any {
                    break;
                }
            }
        }
    }
    let mut erase_dies: Vec<u32> = Vec::new();
    for (_r, p) in resets {
        erase_dies.extend(p.dies());
    }
    schedule_ops(t, &ops, &erase_dies)
}

/// The INDEPENDENT legality checker — a total function over the plan, sharing no code with
/// the scheduler. Verifies per-die serialization (a die runs one op at a time, so a read is
/// never issued while its die is mid-erase — the collision the whole design avoids) and
/// per-channel data-bus disjointness. A die's ops arrive in issue order, so a running
/// watermark is exact; a read's data-out is DELAYED by tR (sense first), so its bus window
/// need not align with issue order — the channel is checked by interval disjointness, not a
/// watermark. First violation as `Err(description)`.
pub fn check_legal_flash(t: &FlashTimings, plan: &[TimedCmd]) -> Result<(), String> {
    let nd = t.n_dies() as usize;
    let nc = t.channels as usize;
    let mut die_busy = vec![0u64; nd]; // die free at/after this cycle (issue-ordered per die)
    let mut chan_bus: Vec<Vec<(u64, u64, usize)>> = vec![Vec::new(); nc]; // (start, end, cmd)
    let mut prev_at = 0u64;

    for (i, c) in plan.iter().enumerate() {
        if c.at < prev_at {
            return Err(format!("cmd {i}: plan not time-sorted"));
        }
        prev_at = c.at;
        let (di, ci) = (c.cmd.die as usize, c.cmd.chan as usize);
        if di >= nd {
            return Err(format!("cmd {i}: die {} out of range", c.cmd.die));
        }
        if ci >= nc {
            return Err(format!("cmd {i}: chan {} out of range", c.cmd.chan));
        }
        if c.at < die_busy[di] {
            return Err(format!(
                "cmd {i}: {:?} on die {} at {} while die busy until {} (op collision)",
                c.cmd.op, c.cmd.die, c.at, die_busy[di]
            ));
        }
        match c.cmd.op {
            FlashOp::Read => {
                // Sense holds the die for [at, at+tR); data-out holds the channel for
                // [at+tR, at+tR+data_cycles) — the die is free to pipeline the next op.
                let bus_start = c.at + t.t_r;
                chan_bus[ci].push((bus_start, bus_start + t.data_cycles, i));
                die_busy[di] = c.at + t.t_r;
            }
            FlashOp::Program => {
                // Data-in holds the channel for [at, at+data); program holds the die after.
                chan_bus[ci].push((c.at, c.at + t.data_cycles, i));
                die_busy[di] = c.at + t.data_cycles + t.t_prog;
            }
            FlashOp::Erase => {
                die_busy[di] = c.at + t.t_bers;
            }
        }
    }

    // Channel data-bus disjointness — order-independent (a read's window is delayed by tR).
    for (ci, windows) in chan_bus.iter().enumerate() {
        let mut sorted = windows.clone();
        sorted.sort_by_key(|w| w.0);
        for pair in sorted.windows(2) {
            if pair[1].0 < pair[0].1 {
                return Err(format!(
                    "cmd {}: data bus on chan {ci} at {} overlaps a window busy until {}",
                    pair[1].2, pair[1].0, pair[0].1
                ));
            }
        }
    }
    Ok(())
}

/// For a pure read gather, the order in which the requested pages' data arrives — indices
/// into `pages`, sorted by delivery beat (ties broken by issue order). Near-storage compute
/// consumes beats in THIS order, so a reduction fed at the storage edge (merge-on-ingest,
/// scan, hash) must be SOUND under it: the die-interleave that saturates the bus reorders the
/// stream, so only order-invariant reductions (a CRDT join, a commutative fold) are safe here.
pub fn gather_delivery_order(t: &FlashTimings, pages: &[u64]) -> Vec<usize> {
    let nd = t.n_dies() as usize;
    let nc = t.channels as usize;
    let mut die_free = vec![0u64; nd];
    let mut chan_free = vec![0u64; nc];
    let mut beats: Vec<(u64, usize)> = Vec::with_capacity(pages.len());
    for (i, &page) in pages.iter().enumerate() {
        let (chan, g) = map_page(t, page);
        let (ci, gi) = (chan as usize, g as usize);
        let mut at = die_free[gi];
        if chan_free[ci] > at + t.t_r {
            at = chan_free[ci] - t.t_r;
        }
        at = at.max(die_free[gi]);
        beats.push((at + t.t_r + t.data_cycles, i));
        die_free[gi] = at + t.t_r;
        chan_free[ci] = at + t.t_r + t.data_cycles;
    }
    beats.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    beats.into_iter().map(|(_, i)| i).collect()
}

/// Planned read-completion cycles straight from the plan (what the compiler promises).
pub fn planned_read_beats(t: &FlashTimings, plan: &[TimedCmd]) -> Vec<u64> {
    plan.iter()
        .filter(|c| c.cmd.op == FlashOp::Read)
        .map(|c| c.at + t.t_r + t.data_cycles)
        .collect()
}

/// The measured outcome of a plan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Utilization {
    /// Read data-out cycles on the channels.
    pub read_data_cycles: u64,
    /// Program data-in cycles on the channels (writes use the bus too, then bind the die on tPROG).
    pub program_data_cycles: u64,
    pub span_cycles: u64,
    pub reads: u64,
    pub programs: u64,
    pub erases: u64,
    /// Worst-case observed read latency (beat − issue) across all reads.
    pub worst_read_latency: u64,
    /// Channel count — the aggregate bus can move `channels` data cycles per cycle.
    pub channels: u32,
}

impl Utilization {
    /// Aggregate data-bus utilization ∈ `[0,1]`: read data-out + program data-in cycles ÷ the
    /// total bus capacity over the span (span × channels), since the channels run in parallel.
    /// A pure-write workload is tPROG-bound (die-bound), so its channel utilization is honestly
    /// low — the bus idles while the dies program.
    pub fn value(&self) -> f64 {
        let cap = self.span_cycles * self.channels as u64;
        if cap == 0 {
            1.0
        } else {
            (self.read_data_cycles + self.program_data_cycles) as f64 / cap as f64
        }
    }
}

pub fn measure(t: &FlashTimings, plan: &[TimedCmd]) -> Utilization {
    let mut u = Utilization {
        read_data_cycles: 0,
        program_data_cycles: 0,
        span_cycles: 0,
        reads: 0,
        programs: 0,
        erases: 0,
        worst_read_latency: 0,
        channels: t.channels,
    };
    let mut last_end = 0u64;
    for c in plan {
        match c.cmd.op {
            FlashOp::Read => {
                u.reads += 1;
                u.read_data_cycles += t.data_cycles;
                let end = c.at + t.t_r + t.data_cycles;
                last_end = last_end.max(end);
                u.worst_read_latency = u.worst_read_latency.max(t.t_r + t.data_cycles);
            }
            FlashOp::Program => {
                u.programs += 1;
                u.program_data_cycles += t.data_cycles;
                last_end = last_end.max(c.at + t.data_cycles + t.t_prog);
            }
            FlashOp::Erase => {
                u.erases += 1;
                last_end = last_end.max(c.at + t.t_bers);
            }
        }
    }
    u.span_cycles = last_end;
    u
}

/// Tier A — a die-state DEVICE that SIMULATES the plan and reports each read's actual
/// completion cycle. It does not trust the plan's promise: it replays each command against a
/// per-die busy timeline and STALLS a command whose die is not yet free (a read issued into a
/// mid-erase die waits out the erase). So on a legal plan the simulated beats equal
/// [`planned_read_beats`] EXACTLY (zero jitter, cross-checked against the promise), and on an
/// illegal plan they DIVERGE — the device is an independent collision detector, not an echo.
pub struct FlashDeviceModel;

impl FlashDeviceModel {
    pub fn execute(t: &FlashTimings, plan: &[TimedCmd]) -> Vec<u64> {
        let nd = t.n_dies() as usize;
        let mut die_free = vec![0u64; nd];
        let mut beats = Vec::new();
        for c in plan {
            let di = c.cmd.die as usize;
            if di >= nd {
                continue;
            }
            let issue = c.at.max(die_free[di]); // stall until the die is genuinely free
            match c.cmd.op {
                FlashOp::Read => {
                    beats.push(issue + t.t_r + t.data_cycles);
                    die_free[di] = issue + t.t_r;
                }
                FlashOp::Program => die_free[di] = issue + t.data_cycles + t.t_prog,
                FlashOp::Erase => die_free[di] = issue + t.t_bers,
            }
        }
        beats
    }
}

/// Tier B — an executor that lets a read PREEMPT an in-progress erase on its die, paying a
/// bounded `suspend_window` instead of the full tBERS, then resuming the erase. Returns each
/// read's completion cycle; every read stays within [`FlashTimings::read_latency_bound`].
pub fn execute_with_suspend(t: &FlashTimings, plan: &[TimedCmd]) -> Vec<u64> {
    let nd = t.n_dies() as usize;
    // (busy_until, is_erase) per die.
    let mut die_until = vec![0u64; nd];
    let mut die_erasing = vec![false; nd];
    let mut beats = Vec::new();
    for c in plan {
        let di = c.cmd.die as usize;
        match c.cmd.op {
            FlashOp::Read => {
                let mut at = c.at;
                if at < die_until[di] {
                    // Die busy: if it's an erase, suspend it (bounded); else wait it out.
                    at = if die_erasing[di] {
                        c.at + t.suspend_window
                    } else {
                        die_until[di]
                    };
                }
                beats.push(at + t.t_r + t.data_cycles);
                die_until[di] = at + t.t_r;
                die_erasing[di] = false;
            }
            FlashOp::Program => {
                let at = c.at.max(die_until[di]);
                die_until[di] = at + t.data_cycles + t.t_prog;
                die_erasing[di] = false;
            }
            FlashOp::Erase => {
                let at = c.at.max(die_until[di]);
                die_until[di] = at + t.t_bers;
                die_erasing[di] = true;
            }
        }
    }
    beats
}

/// Tier C — the black-box SSD: a HIDDEN FTL fires an un-suspendable erase on `victim_die`
/// just before the `victim_read`-th read (0-based, over reads only), so that read stalls
/// behind the full tBERS. Returns each read's completion cycle. This models what a commodity
/// drive does; the point is that a read now BLOWS its latency bound — proving no static
/// guarantee survives once reclamation timing is not host-owned.
pub fn black_box_execute(
    t: &FlashTimings,
    plan: &[TimedCmd],
    victim_die: u32,
    victim_read: usize,
) -> Vec<u64> {
    let nd = t.n_dies() as usize;
    let mut die_until = vec![0u64; nd];
    let mut beats = Vec::new();
    let mut read_ix = 0usize;
    for c in plan {
        let di = c.cmd.die as usize;
        match c.cmd.op {
            FlashOp::Read => {
                // The hidden FTL slams a full erase onto the victim die right before it.
                if read_ix == victim_read && c.cmd.die == victim_die {
                    die_until[di] = die_until[di].max(c.at) + t.t_bers;
                }
                let at = c.at.max(die_until[di]);
                beats.push(at + t.t_r + t.data_cycles);
                die_until[di] = at + t.t_r;
                read_ix += 1;
            }
            FlashOp::Program => {
                let at = c.at.max(die_until[di]);
                die_until[di] = at + t.data_cycles + t.t_prog;
            }
            FlashOp::Erase => {
                let at = c.at.max(die_until[di]);
                die_until[di] = at + t.t_bers;
            }
        }
    }
    beats
}

// ── The certified command stream + the dumb in-order executor ────────────────────────────
//
// The controller collapses to an executor that plays a compile-time NAND command stream
// with NO reorder logic and NO reclamation guesses — because the plan already contains all
// of it. The stream is delta-encoded, the executor is a countdown FSM, and the countdown
// arithmetic is proven exact by our certified SAT (see the test battery).

/// Encode a timed plan as executor words: `[delta:24][op:3][chan:5][die:8][pad:24]`.
/// `delta` = cycles after the PREVIOUS command (first word: cycles after reset).
/// op: 0=Read 1=Program 2=Erase.
pub fn delta_encode(plan: &[TimedCmd]) -> Vec<u64> {
    let mut out = Vec::with_capacity(plan.len());
    let mut prev = 0u64;
    for c in plan {
        let delta = c.at - prev;
        assert!(
            delta < (1 << 24),
            "command delta {delta} exceeds 24-bit executor field"
        );
        prev = c.at;
        let op = match c.cmd.op {
            FlashOp::Read => 0u64,
            FlashOp::Program => 1,
            FlashOp::Erase => 2,
        };
        out.push(
            (delta << 40) | (op << 37) | ((c.cmd.chan as u64) << 32) | ((c.cmd.die as u64) << 24),
        );
    }
    out
}

/// The dumb in-order executor: a countdown FSM. It loads the next word, waits exactly
/// `delta` cycles, strobes the command pins, and acks the word — in arrival order, by
/// construction (there is no other datapath). `cmd_ack` pulses on CONSUME (load), so the
/// producer's read index advances immediately (the classic handshake-latency trap avoided,
/// exactly as in `dram_executor`).
pub fn emit_storage_controller_verilog() -> String {
    r#"// storage_controller — the in-order NAND command-stream executor (storage analog of
// dram_executor). The compile-time plan is delta-encoded ([delta:24][op:3][chan:5][die:8]);
// this FSM waits each delta and fires the command pins. Commands can only issue in strict
// word order — there is no queue or scheduling logic in the netlist to do otherwise. The
// countdown arithmetic is proven exact by our certified SAT (see codegen_sva::storage_controller_rtl).
module storage_controller(
  input clk,
  input rst,
  input [63:0] cmd_word,
  input cmd_valid,
  output reg cmd_ack,
  output reg issue,
  output reg [2:0] op,
  output reg [4:0] chan,
  output reg [7:0] die
);
  reg loaded;
  reg [23:0] wait_left;
  reg [2:0] op_l;
  reg [4:0] chan_l;
  reg [7:0] die_l;
  always @(posedge clk) begin
    if (rst) begin
      loaded <= 1'b0; wait_left <= 24'd0; issue <= 1'b0; cmd_ack <= 1'b0;
      op <= 3'd0; chan <= 5'd0; die <= 8'd0; op_l <= 3'd0; chan_l <= 5'd0; die_l <= 8'd0;
    end else begin
      issue <= 1'b0;
      cmd_ack <= 1'b0;
      if (!loaded) begin
        if (cmd_valid) begin
          wait_left <= cmd_word[63:40];
          op_l <= cmd_word[39:37];
          chan_l <= cmd_word[36:32];
          die_l <= cmd_word[31:24];
          loaded <= 1'b1;
          cmd_ack <= 1'b1;
        end
      end else if (wait_left == 24'd0) begin
        issue <= 1'b1;
        op <= op_l; chan <= chan_l; die <= die_l;
        loaded <= 1'b0;
      end else begin
        wait_left <= wait_left - 24'd1;
      end
    end
  end
endmodule
"#
    .to_string()
}

/// A self-checking testbench: a ROM of REAL plan words (from [`delta_encode`]) drives the
/// executor; the bench asserts every issue arrives IN ORDER with the exact op/chan/die of its
/// word, AND that the hardware honors the compiled TIMING — the clk-cycle gap between two
/// consecutive issues exceeds the command's own delta by a FIXED pipeline overhead (so every
/// compiled inter-command interval is reproduced exactly, not just the order). Prints PASS/FAIL.
pub fn emit_storage_controller_testbench(nwords: usize) -> String {
    format!(
        r#"// tb_storage_controller — replays a REAL compiled NAND stream (storage_plan.hex) and
// checks order + payload + TIMING FAITHFULNESS (gap - delta is a fixed pipeline overhead).
`timescale 1ns/1ps
module tb_storage_controller;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1;
  reg [63:0] rom [0:{last}];
  integer idx = 0;
  integer issues = 0;
  integer errors = 0;
  integer cyc = 0;
  integer prev_issue = -1;
  integer overhead = -1;
  integer dlt = 0;
  wire [63:0] cmd_word = rom[idx];
  wire cmd_valid = (idx <= {last});
  wire cmd_ack, issue;
  wire [2:0] op; wire [4:0] chan; wire [7:0] die;
  storage_controller dut(.clk(clk), .rst(rst), .cmd_word(cmd_word), .cmd_valid(cmd_valid),
                    .cmd_ack(cmd_ack), .issue(issue), .op(op), .chan(chan), .die(die));
  reg [63:0] expect_word;
  always @(posedge clk) begin
    if (!rst) cyc = cyc + 1;         // global cycle counter (origin irrelevant; gaps are what matter)
    if (cmd_ack) idx <= idx + 1;     // pop on CONSUME (load), not issue
    if (issue) begin
      expect_word = rom[issues];     // issues arrive strictly in word order
      dlt = expect_word[63:40];      // this command's compiled delta
      if (op !== expect_word[39:37] || chan !== expect_word[36:32] || die !== expect_word[31:24]) begin
        errors = errors + 1;
        $display("FAIL: issue %0d payload mismatch", issues);
      end
      // TIMING FAITHFULNESS: gap since the previous issue = delta + a constant pipeline overhead.
      if (prev_issue >= 0) begin
        if (overhead == -1) overhead = (cyc - prev_issue) - dlt;
        else if (((cyc - prev_issue) - dlt) != overhead) begin
          errors = errors + 1;
          $display("FAIL: issue %0d timing skew (gap %0d, delta %0d, overhead %0d != %0d)",
                   issues, cyc - prev_issue, dlt, (cyc - prev_issue) - dlt, overhead);
        end
      end
      prev_issue = cyc;
      issues = issues + 1;
    end
  end
  initial begin
    $readmemh("storage_plan.hex", rom);
    repeat (4) @(posedge clk);
    rst = 0;
    repeat (2000000) @(posedge clk);
    if (issues !== {n}) begin
      $display("FAIL: issued %0d of {n} commands", issues);
    end else if (errors == 0) begin
      $display("PASS: %0d commands issued in order, exact payloads, timing faithful (overhead %0d cycles)", issues, overhead);
    end
    $finish;
  end
endmodule
"#,
        last = nwords - 1,
        n = nwords
    )
}

/// The standard measurement scenarios — kernel-shaped storage traffic, each checked legal
/// and measured. Returns a stable JSON document (pure function of the timing table) — the
/// `benchmarks/results/storage_sim.json` artifact the handout cites.
pub fn standard_report() -> String {
    let t = FlashTimings::zns_tlc();
    let read_zone = |base: u64, count: u64| ZoneDesc {
        zone: 0,
        base_page: base,
        count,
        write: false,
    };

    let mut scenarios: Vec<(&str, Vec<TimedCmd>)> = Vec::new();
    scenarios.push((
        "sequential_read_8k_pages",
        schedule_zoned(
            &t,
            &[read_zone(0, 8192)],
            &[],
            &[],
            VisitOrder::ProgramOrder,
        ),
    ));

    // ZNS append stream (program-heavy).
    let append = ZoneDesc {
        zone: 1,
        base_page: 0,
        count: 4096,
        write: true,
    };
    scenarios.push((
        "zns_append_4k_pages",
        schedule_zoned(&t, &[append], &[], &[], VisitOrder::ProgramOrder),
    ));

    // CONCURRENT reclamation: a read stream on 56 dies while a full zone erases on the other
    // 8 dies — the reads keep streaming, the GC runs alongside, neither touches the other.
    let read_win = Placement {
        die_base: 0,
        die_count: 56,
    };
    let gc_win = Placement {
        die_base: 56,
        die_count: 8,
    };
    scenarios.push((
        "reads_with_concurrent_gc",
        schedule_placed(
            &t,
            &[(
                ZoneDesc {
                    zone: 0,
                    base_page: 0,
                    count: 6144,
                    write: false,
                },
                read_win,
            )],
            &[(ZoneReset { zone: 1 }, gc_win)],
            VisitOrder::ProgramOrder,
        ),
    ));

    // The random tail (index lookup / pointer chase).
    let mut x = 0x9E3779B97F4A7C15u64;
    let pages: Vec<u64> = (0..4096)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % (1 << 24)
        })
        .collect();
    scenarios.push((
        "random_gather_4096",
        schedule_zoned(&t, &[], &[Gather { pages }], &[], VisitOrder::ProgramOrder),
    ));

    let mut body = String::new();
    for (name, plan) in &scenarios {
        check_legal_flash(&t, plan).unwrap_or_else(|e| panic!("{name}: illegal plan: {e}"));
        let u = measure(&t, plan);
        body.push_str(&format!(
            "    \"{name}\": {{ \"utilization\": {:.4}, \"reads\": {}, \"programs\": {}, \
             \"erases\": {}, \"worst_read_latency\": {}, \"span_cycles\": {} }},\n",
            u.value(),
            u.reads,
            u.programs,
            u.erases,
            u.worst_read_latency,
            u.span_cycles
        ));
    }
    let body = body.trim_end_matches(",\n").to_string();

    format!(
        "{{\n  \"meta\": {{\n    \"model\": \"host-managed ZNS-TLC, {ch}ch × {dc} dies, \
         die-interleaved / channel-striped, erases scheduled into idle windows\",\n    \
         \"timings\": {{ \"t_r\": {}, \"t_prog\": {}, \"t_bers\": {}, \"data_cycles\": {}, \
         \"suspend_window\": {}, \"ldpc_retry_worst\": {}, \"channels\": {}, \"dies_per_channel\": {} }},\n    \
         \"read_latency_bound\": {},\n    \"note\": \"pure function of the timing table — \
         regenerate with STORAGE_RECORD=1\"\n  }},\n  \"scenarios\": {{\n{body}\n  }}\n}}\n",
        t.t_r,
        t.t_prog,
        t.t_bers,
        t.data_cycles,
        t.suspend_window,
        t.ldpc_retry_worst,
        t.channels,
        t.dies_per_channel,
        t.read_latency_bound(),
        ch = t.channels,
        dc = t.dies_per_channel,
    )
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/storage_controller_rtl/tests.rs"]
mod tests;
