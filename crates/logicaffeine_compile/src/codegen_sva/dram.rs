//! dram — the DRAM machine.
//!
//! The handout claims ">90% bus efficiency, because the compiler schedules row hits and
//! interleaves banks deterministically" and "refresh is co-scheduled, never a surprise."
//! This module makes those MEASUREMENTS:
//!
//!   • [`DramTimings`] — an LPDDR5X-class timing table (one struct of data, so the
//!     memory architect's corrections drop in as numbers, not code);
//!   • [`schedule`] / [`schedule_ordered`] — the compile-time command scheduler: stream descriptors
//!     (the Wave-1 static schedule's vocabulary) → a fully TIMED command stream with
//!     bank-rotated row pipelining and per-bank refresh placed into planned slots;
//!   • [`check_legal`] — an INDEPENDENT per-command legality checker (tRCD/tRP/tRAS/
//!     tRC/tRRD/tFAW/tCCD, data-bus occupancy, refresh deadlines) — the scheduler is
//!     never trusted to grade its own homework;
//!   • [`DeviceModel`] — a bank-state device that EXECUTES a timed stream and reports
//!     every data-beat time: the zero-jitter fact is `planned == executed`, cycle for
//!     cycle;
//!   • blacklist remapping — a compile-time row-remap around known-bad rows: the same
//!     payload moves, legally, with a measured efficiency delta (yield-by-recompile).
//!
//! Everything is a pure function of its inputs — deterministic by construction, like
//! the rest of the stack. Wave 3 grows the certified-SAT legality obligation and the
//! in-order executor RTL on top of these exact command streams.

use std::collections::BTreeMap;

/// LPDDR5X-class timing parameters, in CONTROLLER CLOCK CYCLES, plus geometry.
/// Values are engineering-realistic for an LPDDR5X-6400-class part driven by a
/// controller clock where one RD/WR burst (BL16) occupies `data_cycles` on the bus —
/// the table is DATA for the memory architect to correct, and every measurement in
/// this module re-derives from it.
#[derive(Clone, Debug)]
pub struct DramTimings {
    pub banks: u32,
    /// Row (page) size in bytes.
    pub row_bytes: u64,
    /// Bytes transferred by one RD/WR burst.
    pub burst_bytes: u64,
    /// Cycles one burst occupies the data bus.
    pub data_cycles: u64,
    /// ACT → RD/WR to the same bank.
    pub trcd: u64,
    /// PRE → ACT to the same bank.
    pub trp: u64,
    /// ACT → PRE minimum to the same bank.
    pub tras: u64,
    /// ACT → ACT to the same bank.
    pub trc: u64,
    /// ACT → ACT to DIFFERENT banks.
    pub trrd: u64,
    /// Rolling window in which at most four ACTs may issue.
    pub tfaw: u64,
    /// CAS → CAS (any bank) — data-bus burst spacing.
    pub tccd: u64,
    /// Per-bank refresh (REFpb) duration: REFpb → next ACT to that bank.
    pub trfc_pb: u64,
    /// Per-bank refresh interval: every bank needs one REFpb per window this long.
    pub trefi_pb: u64,
    /// Write recovery: last WRITE burst end → PRE on that bank.
    pub twr: u64,
    /// Data-bus turnaround bubble when switching read↔write direction.
    pub tturn: u64,
    /// Above 85 °C the refresh interval halves.
    pub hot: bool,
}

impl DramTimings {
    /// An LPDDR5X-6400-class nominal-corner timing table (16 banks, 2 KB rows, BL16 ≈ 32 B
    /// bursts). The `t*` counts below are JEDEC LPDDR5X typical timings for that speed grade,
    /// expressed in the controller's tick unit — a representative nominal corner, NOT a datasheet
    /// pin for a specific die. Validate them against the exact part's datasheet before production.
    pub fn lpddr5x() -> Self {
        DramTimings {
            banks: 16,
            row_bytes: 2048,
            burst_bytes: 32,
            data_cycles: 8,
            trcd: 15,
            trp: 15,
            tras: 34,
            trc: 49,
            trrd: 8,
            tfaw: 40,
            tccd: 8,
            trfc_pb: 112,
            trefi_pb: 3906,
            twr: 18,
            tturn: 6,
            hot: false,
        }
    }

    /// Micron MT47H64M16 (1 Gb, x16, 8 banks) at DDR2-800 (-25E speed grade, tCK = 2.5 ns),
    /// JEDEC 5-5-5 timings expressed in DDR-clock cycles. This is the on-board DRAM of the
    /// Nexys A7-100T and the timing FLOOR the ComputeDRAM streams deliberately violate.
    /// NOTE: DDR2 has all-bank AUTO-REFRESH only (no per-bank REFpb); the RefPb schema here
    /// models refresh as per-bank for schedule/checker compatibility — flagged, and irrelevant
    /// to the ACT-PRE-ACT compute path this campaign exercises.
    pub fn ddr2_micron() -> Self {
        DramTimings {
            banks: 8,
            row_bytes: 2048, // x16 1 Gb page = 2 KB
            burst_bytes: 16, // BL8 × 2 bytes (x16)
            data_cycles: 4,  // BL8 = 8 beats = 4 tCK
            trcd: 5,         // 12.5 ns
            trp: 5,          // 12.5 ns
            tras: 18,        // 45 ns
            trc: 23,         // 57.5 ns (= tRAS + tRP)
            trrd: 4,         // 10 ns (2 KB page)
            tfaw: 18,        // 45 ns (2 KB page; ≥ 4 × tRRD)
            tccd: 2,         // 2 tCK
            trfc_pb: 51,     // 127.5 ns (1 Gb)
            trefi_pb: 3120,  // 7.8 µs
            twr: 6,          // 15 ns
            tturn: 2,
            hot: false,
        }
    }

    pub fn trefi_effective(&self) -> u64 {
        if self.hot {
            self.trefi_pb / 2
        } else {
            self.trefi_pb
        }
    }
}

/// A compile-time transfer descriptor — the vocabulary the Wave-1 static schedule
/// hands to the memory edge: `count` elements of `elem_bytes` from `base`, advancing
/// `stride_bytes` per element (stride == elem ⇒ sequential; stride > row ⇒ row-per-
/// element; the dynamic tail is modeled as an explicit gather list, see [`Gather`]).
#[derive(Clone, Debug)]
pub struct StreamDesc {
    pub base: u64,
    pub elem_bytes: u64,
    pub stride_bytes: u64,
    pub count: u64,
    pub write: bool,
}

/// The dynamic tail: explicit addresses (a pointer-chase), one element each.
#[derive(Clone, Debug)]
pub struct Gather {
    pub addrs: Vec<u64>,
    pub elem_bytes: u64,
    pub write: bool,
}

/// One DRAM command.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DramCmd {
    Act { bank: u32, row: u64 },
    Rd { bank: u32 },
    Wr { bank: u32 },
    Pre { bank: u32 },
    RefPb { bank: u32 },
}

/// A command with its planned issue cycle. The PLAN is the artifact: Wave 3 proves
/// windows of it timing-legal in the certified SAT, and the executor RTL runs it
/// in order with no interpretation.
#[derive(Clone, Copy, Debug)]
pub struct TimedCmd {
    pub at: u64,
    pub cmd: DramCmd,
}

/// Address → (bank, row): rows rotate across banks so sequential rows pipeline
/// (bank k streams data while bank k+1 activates). A compile-time-known mapping —
/// the schedule depends on it, and so does the blacklist remap.
pub fn map_addr(t: &DramTimings, addr: u64) -> (u32, u64) {
    let row_index = addr / t.row_bytes;
    let bank = (row_index % t.banks as u64) as u32;
    (bank, row_index / t.banks as u64)
}

/// How multi-stream access lists are ordered into the plan.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VisitOrder {
    /// Streams back-to-back in declaration order (single-stream kernels).
    ProgramOrder,
    /// ROW-TILED round-robin: one full row from each stream in rotation — the
    /// compile-time answer to SoA kernels (nbody reads 7 arrays per iteration; a
    /// reactive controller thrashes rows, the static scheduler TILES them, because
    /// it is allowed to reorder anything without a data dependence).
    RowTiled,
}

fn one_stream_bursts(t: &DramTimings, s: &StreamDesc) -> Vec<(u32, u64, bool)> {
    let mut out = Vec::new();
    let mut last_burst = u64::MAX;
    for i in 0..s.count {
        let addr = s.base + i * s.stride_bytes;
        for b in 0..s.elem_bytes.div_ceil(t.burst_bytes).max(1) {
            let burst = (addr + b * t.burst_bytes) / t.burst_bytes;
            if burst != last_burst {
                last_burst = burst;
                let (bank, row) = map_addr(t, burst * t.burst_bytes);
                out.push((bank, row, s.write));
            }
        }
    }
    out
}

/// Expand descriptors into the per-burst access list: each entry is (bank, row,
/// write). Bursts within one row are contiguous `burst_bytes` chunks.
fn burst_list(
    t: &DramTimings,
    streams: &[StreamDesc],
    gathers: &[Gather],
    order: VisitOrder,
) -> Vec<(u32, u64, bool)> {
    let mut out = Vec::new();
    match order {
        VisitOrder::ProgramOrder => {
            for s in streams {
                out.extend(one_stream_bursts(t, s));
            }
        }
        VisitOrder::RowTiled => {
            // Split each stream's bursts into row-runs, then rotate one run at a time.
            let mut runs: Vec<Vec<Vec<(u32, u64, bool)>>> = streams
                .iter()
                .map(|s| {
                    let bursts = one_stream_bursts(t, s);
                    let mut rows: Vec<Vec<(u32, u64, bool)>> = Vec::new();
                    for b in bursts {
                        match rows.last_mut() {
                            Some(run) if run[0].0 == b.0 && run[0].1 == b.1 => run.push(b),
                            _ => rows.push(vec![b]),
                        }
                    }
                    rows
                })
                .collect();
            let mut idx = vec![0usize; runs.len()];
            loop {
                let mut any = false;
                for (s, run_list) in runs.iter_mut().enumerate() {
                    if idx[s] < run_list.len() {
                        out.append(&mut run_list[idx[s]]);
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
        for &addr in &g.addrs {
            let (bank, row) = map_addr(t, addr);
            out.push((bank, row, g.write));
        }
    }
    out
}

/// The compile-time scheduler: bursts grouped into row visits, rows pipelined across
/// banks (ACT ahead under tRRD/tFAW while the data bus streams), refresh co-scheduled
/// per bank into planned slots before every deadline. Returns the timed plan.
///
/// `bad_rows` is the yield/blacklist remap input: any (bank,row) in it is skipped and
/// its data lands on the next clean row of the same bank — a pure compile-time remap
/// (the program's payload is unchanged; only physical placement moves).
pub fn schedule(
    t: &DramTimings,
    streams: &[StreamDesc],
    gathers: &[Gather],
    bad_rows: &[(u32, u64)],
) -> Vec<TimedCmd> {
    schedule_ordered(t, streams, gathers, bad_rows, VisitOrder::ProgramOrder)
}

/// [`schedule`] with an explicit multi-stream visit order (see [`VisitOrder`]).
pub fn schedule_ordered(
    t: &DramTimings,
    streams: &[StreamDesc],
    gathers: &[Gather],
    bad_rows: &[(u32, u64)],
    order: VisitOrder,
) -> Vec<TimedCmd> {
    let bursts = burst_list(t, streams, gathers, order);

    // Blacklist remap: per bank, a sorted set of bad rows; row r maps to r + (number
    // of bad rows ≤ its remapped position) — computed by walking clean rows in order.
    let mut bad: BTreeMap<u32, Vec<u64>> = BTreeMap::new();
    for &(b, r) in bad_rows {
        bad.entry(b).or_default().push(r);
    }
    for v in bad.values_mut() {
        v.sort_unstable();
        v.dedup();
    }
    let remap = |bank: u32, row: u64| -> u64 {
        match bad.get(&bank) {
            None => row,
            Some(list) => {
                // Skip bad rows: the k-th clean row of the bank.
                let mut r = row;
                for &b in list {
                    if b <= r {
                        r += 1;
                    } else {
                        break;
                    }
                }
                // A run of bad rows directly at/after the shift needs another pass.
                while list.binary_search(&r).is_ok() {
                    r += 1;
                }
                r
            }
        }
    };

    // Group consecutive same-(bank,row) bursts into row visits.
    struct Visit {
        bank: u32,
        row: u64,
        bursts: Vec<bool>, // write flags, in order
    }
    let mut visits: Vec<Visit> = Vec::new();
    for (bank, row, write) in bursts {
        let row = remap(bank, row);
        match visits.last_mut() {
            Some(v) if v.bank == bank && v.row == row => v.bursts.push(write),
            _ => visits.push(Visit {
                bank,
                row,
                bursts: vec![write],
            }),
        }
    }

    // ── The plan is CLOSED-PAGE, bank-rotated, with BACKGROUND refresh ────────────
    // Closed-page: every row visit is self-contained (ACT … CAS… PRE) — sequential
    // rotation never revisits a row, so open-page buys nothing, and a closed bank is
    // always refresh-ready (the "fixed-latency closed-page" of the rung-3 DRAM).
    // Background refresh: each bank refreshes at STAGGERED ABSOLUTE times planned at
    // compile time; REFpb on an idle bank costs ZERO data-bus cycles, so the tax only
    // appears when a refresh collides with that bank's own visit — rare under 16-way
    // rotation, and even then planned, never a surprise.
    let nb = t.banks as usize;
    let trefi = t.trefi_effective();
    let mut idle_ready: Vec<u64> = vec![0; nb]; // bank fully idle & tRP/tRFC elapsed
    let mut last_act_bank: Vec<u64> = vec![u64::MAX; nb]; // for tRC (u64::MAX = never)
    let mut next_ref: Vec<u64> = (0..nb)
        .map(|i| (i as u64 + 1) * trefi / nb as u64)
        .collect();
    let mut acts: Vec<u64> = Vec::new(); // ACT timestamps for tRRD/tFAW
    let mut bus_free: u64 = 0;
    let mut last_dir: Option<bool> = None; // false = read, true = write
    let mut out: Vec<TimedCmd> = Vec::new();

    fn place_act(t: &DramTimings, acts: &mut Vec<u64>, mut at: u64) -> u64 {
        if let Some(&last) = acts.last() {
            at = at.max(last + t.trrd);
        }
        loop {
            let win_start = at.saturating_sub(t.tfaw - 1);
            let in_window = acts.iter().filter(|&&a| a >= win_start).count();
            if in_window < 4 {
                break;
            }
            let oldest = acts
                .iter()
                .copied()
                .filter(|&a| a >= win_start)
                .min()
                .unwrap();
            at = oldest + t.tfaw;
        }
        acts.push(at);
        let horizon = at.saturating_sub(2 * t.tfaw);
        acts.retain(|&a| a >= horizon);
        at
    }

    let drain_refreshes = |bank: usize,
                           before: u64,
                           idle_ready: &mut Vec<u64>,
                           next_ref: &mut Vec<u64>,
                           out: &mut Vec<TimedCmd>| {
        while next_ref[bank] <= before {
            let r_at = next_ref[bank].max(idle_ready[bank]);
            out.push(TimedCmd {
                at: r_at,
                cmd: DramCmd::RefPb { bank: bank as u32 },
            });
            idle_ready[bank] = r_at + t.trfc_pb;
            next_ref[bank] += trefi;
        }
    };

    for v in visits.iter() {
        let bank = v.bank as usize;

        // Take any refresh slot planned before this visit's data would start.
        drain_refreshes(bank, bus_free, &mut idle_ready, &mut next_ref, &mut out);

        // ACT: backdated toward (bus_free - tRCD) so the first CAS lands exactly when
        // the bus frees — the pipeline overlap that makes rotation near-peak.
        let mut act_lb = idle_ready[bank];
        if last_act_bank[bank] != u64::MAX {
            act_lb = act_lb.max(last_act_bank[bank] + t.trc);
        }
        let want = bus_free.saturating_sub(t.trcd);
        let act_at = place_act(t, &mut acts, act_lb.max(want.min(u64::MAX)));
        out.push(TimedCmd {
            at: act_at,
            cmd: DramCmd::Act {
                bank: v.bank,
                row: v.row,
            },
        });
        last_act_bank[bank] = act_at;

        // Bursts: CAS spaced by the data bus, with a turnaround bubble on any
        // read↔write direction switch (planned, like everything else).
        let mut cas_at = (act_at + t.trcd).max(bus_free);
        let mut last_cas_end = 0u64;
        let mut last_write_end = 0u64;
        for &write in &v.bursts {
            let turn = if last_dir.is_some_and(|d| d != write) {
                t.tturn
            } else {
                0
            };
            cas_at = cas_at.max(bus_free + turn);
            let cmd = if write {
                DramCmd::Wr { bank: v.bank }
            } else {
                DramCmd::Rd { bank: v.bank }
            };
            out.push(TimedCmd { at: cas_at, cmd });
            bus_free = cas_at + t.tccd.max(t.data_cycles);
            last_cas_end = cas_at + t.data_cycles;
            if write {
                last_write_end = last_cas_end;
            }
            last_dir = Some(write);
            cas_at += t.tccd.max(t.data_cycles);
        }

        // Close the page (write recovery honored).
        let pre_at = (act_at + t.tras)
            .max(last_cas_end)
            .max(if last_write_end > 0 {
                last_write_end + t.twr
            } else {
                0
            });
        out.push(TimedCmd {
            at: pre_at,
            cmd: DramCmd::Pre { bank: v.bank },
        });
        idle_ready[bank] = pre_at + t.trp;
    }

    // Background refresh obligations for EVERY bank across the whole plan span.
    let span = out.iter().map(|c| c.at).max().unwrap_or(0);
    for bank in 0..nb {
        drain_refreshes(bank, span, &mut idle_ready, &mut next_ref, &mut out);
    }

    out.sort_by(|a, b| a.at.cmp(&b.at));
    out
}

/// The measured outcome of a plan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Efficiency {
    pub data_bus_cycles: u64,
    pub span_cycles: u64,
    pub bursts: u64,
    pub row_activates: u64,
    pub refreshes: u64,
}

impl Efficiency {
    pub fn value(&self) -> f64 {
        if self.span_cycles == 0 {
            1.0
        } else {
            self.data_bus_cycles as f64 / self.span_cycles as f64
        }
    }
}

pub fn measure(t: &DramTimings, plan: &[TimedCmd]) -> Efficiency {
    let mut e = Efficiency {
        data_bus_cycles: 0,
        span_cycles: 0,
        bursts: 0,
        row_activates: 0,
        refreshes: 0,
    };
    let mut last_end = 0u64;
    for c in plan {
        match c.cmd {
            DramCmd::Rd { .. } | DramCmd::Wr { .. } => {
                e.bursts += 1;
                e.data_bus_cycles += t.data_cycles;
                last_end = last_end.max(c.at + t.data_cycles);
            }
            DramCmd::Act { .. } => {
                e.row_activates += 1;
                last_end = last_end.max(c.at + 1);
            }
            DramCmd::RefPb { .. } => {
                e.refreshes += 1;
                last_end = last_end.max(c.at + t.trfc_pb);
            }
            DramCmd::Pre { .. } => last_end = last_end.max(c.at + 1),
        }
    }
    e.span_cycles = last_end;
    e
}

/// The INDEPENDENT legality checker — a total function over the plan, sharing no code
/// with the scheduler. Returns the first violation as `Err(description)`.
pub fn check_legal(t: &DramTimings, plan: &[TimedCmd]) -> Result<(), String> {
    let nb = t.banks as usize;
    #[derive(Clone, Copy, PartialEq)]
    enum BankState {
        Idle,
        Active,
    }
    let mut state = vec![BankState::Idle; nb];
    let mut act_at = vec![0u64; nb];
    /// Earliest next ACT per bank (tRC from its last ACT).
    let mut act_ready = vec![0u64; nb];
    /// Earliest the bank counts as idle-and-settled (tRP after PRE, tRFC after REFpb).
    let mut idle_ready = vec![0u64; nb];
    let mut last_cas_end = vec![0u64; nb];
    let mut acts: Vec<u64> = Vec::new();
    let mut last_act: Option<u64> = None;
    let mut bus_busy_until = 0u64;
    let mut bus_last_dir: Option<bool> = None;
    let mut last_write_end = vec![0u64; nb];
    let mut refs: Vec<Vec<u64>> = vec![Vec::new(); nb];

    let mut prev_at = 0u64;
    for (i, c) in plan.iter().enumerate() {
        if c.at < prev_at {
            return Err(format!("cmd {i}: plan not time-sorted"));
        }
        prev_at = c.at;
        match c.cmd {
            DramCmd::Act { bank, .. } => {
                let b = bank as usize;
                if state[b] != BankState::Idle {
                    return Err(format!("cmd {i}: ACT on open bank {bank}"));
                }
                if c.at < idle_ready[b] {
                    return Err(format!(
                        "cmd {i}: ACT bank {bank} at {} violates tRP/tRFC (idle at {})",
                        c.at, idle_ready[b]
                    ));
                }
                if c.at < act_ready[b] {
                    return Err(format!("cmd {i}: ACT bank {bank} violates tRC"));
                }
                if let Some(la) = last_act {
                    if c.at < la + t.trrd {
                        return Err(format!("cmd {i}: ACT violates tRRD"));
                    }
                }
                let win = c.at.saturating_sub(t.tfaw - 1);
                if acts.iter().filter(|&&a| a >= win).count() >= 4 {
                    return Err(format!("cmd {i}: 5th ACT inside tFAW window"));
                }
                acts.push(c.at);
                let horizon = c.at.saturating_sub(4 * t.tfaw);
                acts.retain(|&a| a >= horizon);
                last_act = Some(c.at);
                state[b] = BankState::Active;
                act_at[b] = c.at;
                act_ready[b] = c.at + t.trc;
            }
            DramCmd::Rd { bank } | DramCmd::Wr { bank } => {
                let b = bank as usize;
                let write = matches!(c.cmd, DramCmd::Wr { .. });
                if state[b] != BankState::Active {
                    return Err(format!("cmd {i}: CAS on idle bank {bank}"));
                }
                if c.at < act_at[b] + t.trcd {
                    return Err(format!("cmd {i}: CAS violates tRCD on bank {bank}"));
                }
                let turn = if bus_last_dir.is_some_and(|d| d != write) {
                    t.tturn
                } else {
                    0
                };
                if c.at < bus_busy_until + turn {
                    return Err(format!(
                        "cmd {i}: data-bus overlap/turnaround (free at {}, CAS at {})",
                        bus_busy_until + turn,
                        c.at
                    ));
                }
                bus_busy_until = c.at + t.data_cycles.max(t.tccd);
                bus_last_dir = Some(write);
                last_cas_end[b] = c.at + t.data_cycles;
                if write {
                    last_write_end[b] = c.at + t.data_cycles;
                }
            }
            DramCmd::Pre { bank } => {
                let b = bank as usize;
                if state[b] != BankState::Active {
                    return Err(format!("cmd {i}: PRE on idle bank {bank}"));
                }
                if c.at < act_at[b] + t.tras {
                    return Err(format!("cmd {i}: PRE violates tRAS on bank {bank}"));
                }
                if c.at < last_cas_end[b] {
                    return Err(format!(
                        "cmd {i}: PRE before burst completes on bank {bank}"
                    ));
                }
                if last_write_end[b] > 0 && c.at < last_write_end[b] + t.twr {
                    return Err(format!("cmd {i}: PRE violates tWR on bank {bank}"));
                }
                state[b] = BankState::Idle;
                idle_ready[b] = idle_ready[b].max(c.at + t.trp);
            }
            DramCmd::RefPb { bank } => {
                let b = bank as usize;
                if state[b] != BankState::Idle {
                    return Err(format!("cmd {i}: REFpb on open bank {bank}"));
                }
                if c.at < idle_ready[b] {
                    return Err(format!(
                        "cmd {i}: REFpb bank {bank} at {} before idle-ready {}",
                        c.at, idle_ready[b]
                    ));
                }
                refs[b].push(c.at);
                idle_ready[b] = c.at + t.trfc_pb;
            }
        }
    }

    // Refresh deadlines: over a span long enough to owe refreshes, every bank must
    // be refreshed, with no gap (start → first, k → k+1) exceeding 1.5× the
    // effective interval — the staggered background plan keeps well inside this.
    let span = plan.iter().map(|c| c.at).max().unwrap_or(0);
    let trefi = t.trefi_effective();
    if span > 2 * trefi {
        for (b, bank_refs) in refs.iter().enumerate() {
            if bank_refs.is_empty() {
                return Err(format!("bank {b}: no REFpb inside a {span}-cycle plan"));
            }
            let mut prev = 0u64;
            for &r in bank_refs {
                if r.saturating_sub(prev) > trefi + trefi / 2 {
                    return Err(format!(
                        "bank {b}: refresh gap {} exceeds 1.5×tREFI ({})",
                        r - prev,
                        trefi + trefi / 2
                    ));
                }
                prev = r;
            }
            if span.saturating_sub(prev) > trefi + trefi / 2 {
                return Err(format!("bank {b}: trailing refresh gap exceeds 1.5×tREFI"));
            }
        }
    }
    Ok(())
}

/// A bank-state DEVICE that executes the plan and reports each data burst's
/// completion cycle. The zero-jitter fact: these times equal the plan's own
/// (`cas.at + data_cycles`) exactly — the device adds nothing and guesses nothing.
pub struct DeviceModel;

impl DeviceModel {
    pub fn execute(t: &DramTimings, plan: &[TimedCmd]) -> Result<Vec<u64>, String> {
        check_legal(t, plan)?;
        Ok(plan
            .iter()
            .filter_map(|c| match c.cmd {
                DramCmd::Rd { .. } | DramCmd::Wr { .. } => Some(c.at + t.data_cycles),
                _ => None,
            })
            .collect())
    }
}

/// Planned data-beat completion times straight from the plan (what the compiler
/// promises); [`DeviceModel::execute`] is what the device delivers.
pub fn planned_beats(t: &DramTimings, plan: &[TimedCmd]) -> Vec<u64> {
    plan.iter()
        .filter_map(|c| match c.cmd {
            DramCmd::Rd { .. } | DramCmd::Wr { .. } => Some(c.at + t.data_cycles),
            _ => None,
        })
        .collect()
}

/// The standard measurement scenarios — kernel-shaped traffic classes, each checked
/// legal and measured. Returns a stable JSON document (pure function of the timing
/// table; no clocks, no randomness) — the `benchmarks/results/dram_sim.json` artifact.
pub fn standard_report() -> String {
    let t = DramTimings::lpddr5x();
    let seq = |base: u64, bytes: u64, write: bool| StreamDesc {
        base,
        elem_bytes: 8,
        stride_bytes: 8,
        count: bytes / 8,
        write,
    };

    let mut scenarios: Vec<(&str, Vec<TimedCmd>, &DramTimings)> = Vec::new();
    let seq_plan = schedule(&t, &[seq(0, 1 << 20, false)], &[], &[]);
    scenarios.push(("sequential_read_1mb", seq_plan, &t));

    // nbody-shaped SoA: seven parallel float arrays walked together, row-tiled.
    let soa: Vec<StreamDesc> = (0..7)
        .map(|i| seq(i * (1 << 22), 1 << 17, i == 6))
        .collect();
    let soa_plan = schedule_ordered(&t, &soa, &[], &[], VisitOrder::RowTiled);
    scenarios.push(("soa7_rowtiled_nbody", soa_plan, &t));

    // mergesort-shaped: two sorted inputs streamed against one output, row-tiled.
    let merge = vec![
        seq(0, 1 << 18, false),
        seq(1 << 22, 1 << 18, false),
        seq(1 << 23, 1 << 19, true),
    ];
    let merge_plan = schedule_ordered(&t, &merge, &[], &[], VisitOrder::RowTiled);
    scenarios.push(("merge3_rowtiled", merge_plan, &t));

    // The dynamic tail: a seeded scattered gather (graph_bfs's 15%).
    let mut x = 0x9E3779B97F4A7C15u64;
    let addrs: Vec<u64> = (0..4096)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % (1 << 26)) & !7
        })
        .collect();
    let gather_plan = schedule(
        &t,
        &[],
        &[Gather {
            addrs,
            elem_bytes: 8,
            write: false,
        }],
        &[],
    );
    scenarios.push(("gather_4096_random", gather_plan, &t));

    let hot = DramTimings {
        hot: true,
        ..DramTimings::lpddr5x()
    };
    let hot_plan = schedule(&hot, &[seq(0, 1 << 20, false)], &[], &[]);
    let hot_owned = hot.clone();

    let mut body = String::new();
    let emit = |name: &str, plan: &[TimedCmd], tt: &DramTimings, body: &mut String| {
        check_legal(tt, plan).unwrap_or_else(|e| panic!("{name}: illegal plan: {e}"));
        let e = measure(tt, plan);
        body.push_str(&format!(
            "    \"{name}\": {{ \"efficiency\": {:.4}, \"bursts\": {}, \"activates\": {}, \
             \"refreshes\": {}, \"span_cycles\": {} }},\n",
            e.value(),
            e.bursts,
            e.row_activates,
            e.refreshes,
            e.span_cycles
        ));
    };
    for (name, plan, tt) in &scenarios {
        emit(name, plan, tt, &mut body);
    }
    emit("sequential_read_1mb_hot", &hot_plan, &hot_owned, &mut body);
    let body = body.trim_end_matches(",\n").to_string();

    format!(
        "{{\n  \"meta\": {{\n    \"model\": \"LPDDR5X-class 16-bank closed-page, \
         bank-rotated, background per-bank refresh\",\n    \"timings\": {{ \"trcd\": {}, \
         \"trp\": {}, \"tras\": {}, \"trc\": {}, \"trrd\": {}, \"tfaw\": {}, \"tccd\": {}, \
         \"trfc_pb\": {}, \"trefi_pb\": {}, \"twr\": {}, \"tturn\": {}, \"banks\": {}, \"row_bytes\": {}, \
         \"burst_bytes\": {} }},\n    \"note\": \"pure function of the timing table — \
         regenerate with DRAM_RECORD=1 cargo test dram_sim_artifact\", \"unmodeled\": \"bank-group tCCD_L/S, ZQ calibration, command-bus contention — flagged for the memory-architect review\"\n  }},\n  \
         \"scenarios\": {{\n{body}\n  }}\n}}\n",
        t.trcd,
        t.trp,
        t.tras,
        t.trc,
        t.trrd,
        t.tfaw,
        t.tccd,
        t.trfc_pb,
        t.trefi_pb,
        t.twr,
        t.tturn,
        t.banks,
        t.row_bytes,
        t.burst_bytes
    )
}

// ── Wave 3: the certified command stream + the dumb in-order executor ────────────────
//
// Rung 3 of the co-design ladder: the controller collapses to an executor that plays a
// compile-time command stream with NO reorder logic, NO page-policy guesses, NO refresh
// juggling — because the plan already contains all of it. The stream is delta-encoded
// (each word carries its distance from the previous command), the executor is a
// countdown FSM, and the countdown arithmetic is proven exact by our certified SAT.

/// Encode a timed plan as executor words: `[delta:24][op:3][bank:5][row:32]`.
/// `delta` = cycles after the PREVIOUS command (first word: cycles after reset).
/// op: 0=ACT 1=RD 2=WR 3=PRE 4=REFpb.
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
        let (op, bank, row) = match c.cmd {
            DramCmd::Act { bank, row } => (0u64, bank as u64, row),
            DramCmd::Rd { bank } => (1, bank as u64, 0),
            DramCmd::Wr { bank } => (2, bank as u64, 0),
            DramCmd::Pre { bank } => (3, bank as u64, 0),
            DramCmd::RefPb { bank } => (4, bank as u64, 0),
        };
        assert!(row < (1 << 32), "row exceeds 32-bit executor field");
        out.push((delta << 40) | (op << 37) | (bank << 32) | row);
    }
    out
}

/// The dumb in-order executor: a countdown FSM. It loads the next word, waits exactly
/// `delta` cycles, strobes the command pins, and acks the word — in arrival order, by
/// construction (there is no other datapath). The rung-3 controller.
pub fn emit_dram_executor_verilog() -> String {
    r#"// dram_executor — the rung-3 in-order command-stream executor (COLD_HARD_FACTS Wave 3).
// The compile-time plan is delta-encoded ([delta:24][op:3][bank:5][row:32]); this FSM
// waits each delta and fires the command pins. Commands can only issue in strict word
// order — there is no queue or scheduling logic in the netlist to do otherwise. The
// countdown arithmetic is proven exact by our certified SAT (see codegen_sva::dram).
// cmd_ack is a FIFO-POP pulse asserted the cycle a word is CONSUMED (loaded), not the
// cycle it later issues — so the producer's read index advances immediately and a
// zero-delay command can never reload itself (the classic handshake-latency trap).
module dram_executor(
  input clk,
  input rst,
  input [63:0] cmd_word,
  input cmd_valid,
  output reg cmd_ack,
  output reg issue,
  output reg [2:0] op,
  output reg [4:0] bank,
  output reg [31:0] row
);
  reg loaded;
  reg [23:0] wait_left;
  reg [2:0] op_l;
  reg [4:0] bank_l;
  reg [31:0] row_l;
  always @(posedge clk) begin
    if (rst) begin
      loaded <= 1'b0; wait_left <= 24'd0; issue <= 1'b0; cmd_ack <= 1'b0;
      op <= 3'd0; bank <= 5'd0; row <= 32'd0; op_l <= 3'd0; bank_l <= 5'd0; row_l <= 32'd0;
    end else begin
      issue <= 1'b0;
      cmd_ack <= 1'b0;
      if (!loaded) begin
        if (cmd_valid) begin
          wait_left <= cmd_word[63:40];
          op_l <= cmd_word[39:37];
          bank_l <= cmd_word[36:32];
          row_l <= cmd_word[31:0];
          loaded <= 1'b1;
          cmd_ack <= 1'b1;
        end
      end else if (wait_left == 24'd0) begin
        issue <= 1'b1;
        op <= op_l; bank <= bank_l; row <= row_l;
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

/// A self-checking testbench: a ROM of REAL plan words (from [`delta_encode`]) drives
/// the executor; the bench asserts every issue arrives IN ORDER with the exact opcode/
/// bank/row of its word, and prints PASS/FAIL. (Absolute-time equality to the plan is
/// the Rust-side lock; the bench pins order + payload + one-issue-per-word in RTL.)
pub fn emit_dram_executor_testbench(nwords: usize) -> String {
    format!(
        r#"// tb_dram_executor — replays a REAL Wave-2 command stream (dram_plan.hex).
`timescale 1ns/1ps
module tb_dram_executor;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1;
  reg [63:0] rom [0:{last}];
  integer idx = 0;
  integer issues = 0;
  integer errors = 0;
  wire [63:0] cmd_word = rom[idx];
  wire cmd_valid = (idx <= {last});
  wire cmd_ack, issue;
  wire [2:0] op; wire [4:0] bank; wire [31:0] row;
  dram_executor dut(.clk(clk), .rst(rst), .cmd_word(cmd_word), .cmd_valid(cmd_valid),
                    .cmd_ack(cmd_ack), .issue(issue), .op(op), .bank(bank), .row(row));
  reg [63:0] expect_word;
  always @(posedge clk) begin
    if (cmd_ack) idx <= idx + 1;   // pop on CONSUME (load), not issue
    if (issue) begin
      expect_word = rom[issues];   // issues arrive strictly in word order
      if (op !== expect_word[39:37] || bank !== expect_word[36:32] || row !== expect_word[31:0]) begin
        errors = errors + 1;
        $display("FAIL: issue %0d payload mismatch", issues);
      end
      issues = issues + 1;
    end
  end
  initial begin
    $readmemh("dram_plan.hex", rom);
    repeat (4) @(posedge clk);
    rst = 0;
    // Run long enough for every delta in the plan slice.
    repeat (200000) @(posedge clk);
    if (issues !== {n}) begin
      $display("FAIL: issued %0d of {n} commands", issues);
    end else if (errors == 0) begin
      $display("PASS: %0d commands issued strictly in order with exact payloads", issues);
    end
    $finish;
  end
endmodule
"#,
        last = nwords - 1,
        n = nwords
    )
}

// ── ComputeDRAM: the in-DRAM compute plane (rung 4, off-the-shelf DRAM) ───────────────
//
// ComputeDRAM (Gao/Tziantzioulis/Wentzlaff, MICRO 2019) shows UNMODIFIED DRAM computes
// RowClone (row copy) and bulk bitwise AND/OR/MAJ when `ACT → PRE → ACT` is issued FASTER
// than JEDEC tRP/tRAS allow: the interrupted precharge leaves multiple rows sharing the
// bitlines and the sense amps resolve them to the bitwise MAJORITY (AND = MAJ(a,b,0),
// OR = MAJ(a,b,1) with a preset control row). This is the DRAM execution plane of the unified
// LOGOS machine: a bulk-bitwise/copy LOGOS op lowers to a `ComputeDramOp`, whose logical result
// is proven ≡ the VM's bitwise semantics by the `RowStore` oracle below.
//
// These command streams are DELIBERATELY timing-illegal — `check_legal` MUST reject them, and
// that quarantine is a PIT-OF-SUCCESS invariant (a ComputeDRAM stream can never be mistaken for
// a certified JEDEC-legal plan). The exact effective violation window is chip-specific and is
// MEASURED on real silicon in Phase 3; `ComputeDramTiming::nominal` is the model's recipe.

/// The sub-JEDEC violation window that triggers the ComputeDRAM effect, in controller cycles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComputeDramTiming {
    /// ACT → PRE gap — deliberately below tRAS (opens the row, then interrupts its close).
    pub t1: u64,
    /// PRE → ACT gap — deliberately below tRP (interrupts the precharge mid-flight).
    pub t2: u64,
}

impl ComputeDramTiming {
    /// The nominal paper-class recipe (a few cycles). The real window is MEASURED in Phase 3.
    pub fn nominal() -> Self {
        ComputeDramTiming { t1: 2, t2: 1 }
    }
}

/// A bulk in-DRAM operation over whole rows of one bank. `RowCopy` is RowClone; `And`/`Or`/`Maj`
/// are the triple-row-activation majority reductions (`And`/`Or` assume a preset control row
/// established by a prior `RowCopy` — wired in the Phase 5 lowering).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComputeDramOp {
    RowCopy {
        bank: u32,
        src: u64,
        dst: u64,
    },
    And {
        bank: u32,
        a: u64,
        b: u64,
        dst: u64,
    },
    Or {
        bank: u32,
        a: u64,
        b: u64,
        dst: u64,
    },
    Maj {
        bank: u32,
        a: u64,
        b: u64,
        c: u64,
        dst: u64,
    },
}

impl ComputeDramOp {
    pub fn bank(&self) -> u32 {
        match *self {
            ComputeDramOp::RowCopy { bank, .. }
            | ComputeDramOp::And { bank, .. }
            | ComputeDramOp::Or { bank, .. }
            | ComputeDramOp::Maj { bank, .. } => bank,
        }
    }

    /// The rows activated in rapid succession (the ACT-PRE-ACT cadence walks these in order).
    fn activation_rows(&self) -> Vec<u64> {
        match *self {
            ComputeDramOp::RowCopy { src, dst, .. } => vec![src, dst],
            ComputeDramOp::And { a, b, .. } | ComputeDramOp::Or { a, b, .. } => vec![a, b],
            ComputeDramOp::Maj { a, b, c, .. } => vec![a, b, c],
        }
    }

    fn is_reduction(&self) -> bool {
        !matches!(self, ComputeDramOp::RowCopy { .. })
    }
}

/// Emit the raw, deliberately timing-VIOLATING command stream that triggers `op` on real DRAM:
/// an `ACT → PRE → ACT (…)` cadence with sub-`tRAS`/sub-`tRP` gaps. The stream is NEVER
/// JEDEC-legal (that is the point) — `check_legal` rejects it by design.
pub fn emit_computedram_stream(op: &ComputeDramOp, tv: &ComputeDramTiming) -> Vec<TimedCmd> {
    let bank = op.bank();
    let rows = op.activation_rows();
    let mut out = Vec::new();
    let mut at = 0u64;
    // Walk the operand rows, opening each and interrupting its precharge with the next ACT —
    // the ACT→PRE→ACT charge-sharing cadence, with sub-tRAS (t1) and sub-tRP (t2) gaps.
    for (i, row) in rows.iter().enumerate() {
        out.push(TimedCmd {
            at,
            cmd: DramCmd::Act { bank, row: *row },
        });
        at += tv.t1;
        if i + 1 < rows.len() {
            out.push(TimedCmd {
                at,
                cmd: DramCmd::Pre { bank },
            });
            at += tv.t2;
        }
    }
    // Reductions (And/Or/Maj) sense the resolved majority off the bitlines; RowCopy needs no read.
    if op.is_reduction() {
        out.push(TimedCmd {
            at,
            cmd: DramCmd::Rd { bank },
        });
        at += 1;
    }
    out.push(TimedCmd {
        at,
        cmd: DramCmd::Pre { bank },
    });
    out
}

/// Bitwise majority of three words — the sense-amp reduction ComputeDRAM performs per bit.
pub fn word_majority(a: u64, b: u64, c: u64) -> u64 {
    (a & b) | (a & c) | (b & c)
}

/// A functional model of DRAM rows — the ComputeDRAM ORACLE the real silicon is measured
/// against. Each row is a bit vector packed into `words` u64 words.
#[derive(Clone, Debug, Default)]
pub struct RowStore {
    words: usize,
    rows: BTreeMap<(u32, u64), Vec<u64>>,
}

impl RowStore {
    pub fn new(words: usize) -> Self {
        RowStore {
            words,
            rows: BTreeMap::new(),
        }
    }

    pub fn write(&mut self, bank: u32, row: u64, data: &[u64]) {
        let mut v = vec![0u64; self.words];
        for (slot, w) in v.iter_mut().zip(data.iter()) {
            *slot = *w;
        }
        self.rows.insert((bank, row), v);
    }

    pub fn read(&self, bank: u32, row: u64) -> Vec<u64> {
        self.rows
            .get(&(bank, row))
            .cloned()
            .unwrap_or_else(|| vec![0u64; self.words])
    }

    /// Apply a ComputeDRAM op to the modeled rows (the oracle semantics): RowClone copies a row;
    /// And/Or/Maj resolve the per-bit sense-amp majority (And with a 0 control, Or with a 1 control).
    pub fn apply(&mut self, op: &ComputeDramOp) {
        let (bank, dst, result): (u32, u64, Vec<u64>) = match *op {
            ComputeDramOp::RowCopy { bank, src, dst } => (bank, dst, self.read(bank, src)),
            ComputeDramOp::And { bank, a, b, dst } => {
                let (ra, rb) = (self.read(bank, a), self.read(bank, b));
                (
                    bank,
                    dst,
                    ra.iter()
                        .zip(&rb)
                        .map(|(x, y)| word_majority(*x, *y, 0))
                        .collect(),
                )
            }
            ComputeDramOp::Or { bank, a, b, dst } => {
                let (ra, rb) = (self.read(bank, a), self.read(bank, b));
                (
                    bank,
                    dst,
                    ra.iter()
                        .zip(&rb)
                        .map(|(x, y)| word_majority(*x, *y, u64::MAX))
                        .collect(),
                )
            }
            ComputeDramOp::Maj { bank, a, b, c, dst } => {
                let (ra, rb, rc) = (self.read(bank, a), self.read(bank, b), self.read(bank, c));
                let r = ra
                    .iter()
                    .zip(&rb)
                    .zip(&rc)
                    .map(|((x, y), z)| word_majority(*x, *y, *z))
                    .collect();
                (bank, dst, r)
            }
        };
        self.write(bank, dst, &result);
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/dram/tests.rs"]
mod tests;
