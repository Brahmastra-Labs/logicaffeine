//! ddr2_jedec — a DDR2 part's datasheet timing at one clock, and the JESD79-2 command rules a bus trace is held to.
//!
//! [`Ddr2Grade`] is a speed grade as its datasheet prints it: nanosecond minimums and the CAS latency each clock
//! allows. [`Ddr2Grade::at`] clocks it: every minimum rounds up to whole clocks (tRRD, tWR, tWTR and tRTP to no fewer
//! than two), every maximum rounds down, and tRPA is tRP + tCK as an eight-bank part's is. [`check_bus`] replays a
//! pin-level command trace against the clocked rules and returns the first violation, or the trace's facts: what
//! was issued, and every column access with the cycle its data reaches the bus.
//!
//! Every DDR2 controller the comparative bench drives is judged by this one checker over the commands it put on the
//! bus, so the controllers are compared under the same rules rather than each under its own model. The rules are
//! JESD79-2's with additive latency 0, write latency CL − 1, and bursts never cut short. A READ or WRITE with A10 high
//! precharges its bank itself: the precharge starts at the later of the clock an explicit PRECHARGE could have come
//! and tRAS after the ACT, and the bank is ready tRP after it. The refresh debt counts from the trace's cycle 0, when
//! the device is taken as freshly refreshed: by any cycle t it has had at least floor(t / tREFI) − 8 REFRESH commands,
//! eight being the most JESD79-2 lets a controller postpone. A trace that writes a mode register is refused, not
//! judged; that belongs to initialisation, not to the traffic the checker is for.

use super::ddr2_phy::{ddr2_pins, Ddr2Cmd};
use super::dram::DramTimings;
use std::fmt;

/// The REFRESH commands JESD79-2 lets a controller postpone: by any clock t it owes at least floor(t / tREFI) − this.
pub const REFRESH_POSTPONE_MAX: u64 = 8;

/// A DDR2 speed grade: its geometry, the CAS latency each clock period allows, and its datasheet minimums (and the
/// tRAS and tREFI maximums), in picoseconds unless named in clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ddr2Grade {
    pub part: &'static str,
    pub banks: u32,
    pub row_bits: u32,
    pub col_bits: u32,
    pub dq_bits: u32,
    /// `(tCK floor, CL)` by rising CL: a CAS latency runs at any clock period from its floor up to `tck_max_ps`.
    pub cl_floors: &'static [(u64, u64)],
    pub tck_max_ps: u64,
    pub trcd_ps: u64,
    pub trp_ps: u64,
    pub tras_ps: u64,
    pub tras_max_ps: u64,
    pub trc_ps: u64,
    pub trrd_ps: u64,
    pub tfaw_ps: u64,
    pub twr_ps: u64,
    pub twtr_ps: u64,
    pub trtp_ps: u64,
    pub trfc_ps: u64,
    pub trefi_ps: u64,
    pub tccd_ck: u64,
}

impl Ddr2Grade {
    /// Micron MT47H64M16 (1 Gb, x16, 8 banks), speed grade -25: DDR2-800 at CL 6. The Nexys A7-100T carries this
    /// part, MT47H64M16HR-25:H. Micron 1Gb DDR2 datasheet, Rev. V 6/10: Table 1 and Table 12's x16 rows.
    pub const MT47H64M16_25: Ddr2Grade = Ddr2Grade {
        part: "MT47H64M16-25",
        cl_floors: &[(5000, 3), (3750, 4), (3000, 5), (2500, 6)],
        trcd_ps: 15_000,
        trp_ps: 15_000,
        ..Self::MT47H64M16_25E
    };

    /// The same part's -25E grade: DDR2-800 at CL 5.
    pub const MT47H64M16_25E: Ddr2Grade = Ddr2Grade {
        part: "MT47H64M16-25E",
        banks: 8,
        row_bits: 13,
        col_bits: 10,
        dq_bits: 16,
        cl_floors: &[(5000, 3), (3750, 4), (2500, 5)],
        tck_max_ps: 8_000,
        trcd_ps: 12_500,
        trp_ps: 12_500,
        tras_ps: 40_000,
        tras_max_ps: 70_000_000,
        trc_ps: 55_000,
        trrd_ps: 10_000,
        tfaw_ps: 45_000,
        twr_ps: 15_000,
        twtr_ps: 7_500,
        trtp_ps: 7_500,
        trfc_ps: 127_500,
        trefi_ps: 7_800_000,
        tccd_ck: 2,
    };

    /// The grade clocked at `tck_ps` with burst length `bl`, or why the part cannot run that way.
    pub fn at(&self, tck_ps: u64, bl: u64) -> Result<Ddr2Timing, String> {
        if bl != 4 && bl != 8 {
            return Err(format!("{}: a DDR2 burst is 4 or 8 beats, not {bl}", self.part));
        }
        if tck_ps == 0 || tck_ps > self.tck_max_ps {
            return Err(format!("{}: tCK {tck_ps} ps is outside the part's range (at most {} ps)", self.part, self.tck_max_ps));
        }
        let cl = self
            .cl_floors
            .iter()
            .find(|&&(floor, _)| floor <= tck_ps)
            .map(|&(_, cl)| cl)
            .ok_or_else(|| format!("{}: tCK {tck_ps} ps is faster than any CAS latency it supports", self.part))?;
        let up = |ps: u64| ps.div_ceil(tck_ps);
        let up2 = |ps: u64| up(ps).max(2);
        Ok(Ddr2Timing {
            grade: *self,
            tck_ps,
            bl,
            cl,
            wl: cl - 1,
            trcd: up(self.trcd_ps),
            trp: up(self.trp_ps),
            trpa: up(self.trp_ps + tck_ps),
            tras: up(self.tras_ps),
            tras_max: self.tras_max_ps / tck_ps,
            trc: up(self.trc_ps),
            trrd: up2(self.trrd_ps),
            tfaw: up(self.tfaw_ps),
            twr: up2(self.twr_ps),
            twtr: up2(self.twtr_ps),
            trtp: up2(self.trtp_ps),
            trfc: up(self.trfc_ps),
            trefi: self.trefi_ps / tck_ps,
            tccd: self.tccd_ck,
        })
    }
}

/// A grade clocked: every rule in whole clocks. `wl` is `cl − 1`, `trpa` the precharge-all period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ddr2Timing {
    pub grade: Ddr2Grade,
    pub tck_ps: u64,
    pub bl: u64,
    pub cl: u64,
    pub wl: u64,
    pub trcd: u64,
    pub trp: u64,
    pub trpa: u64,
    pub tras: u64,
    pub tras_max: u64,
    pub trc: u64,
    pub trrd: u64,
    pub tfaw: u64,
    pub twr: u64,
    pub twtr: u64,
    pub trtp: u64,
    pub trfc: u64,
    pub trefi: u64,
    pub tccd: u64,
}

impl Ddr2Timing {
    /// READ → PRECHARGE, same bank: AL + BL/2 − 2 + max(tRTP, 2).
    pub fn read_to_precharge(&self) -> u64 {
        self.bl / 2 - 2 + self.trtp.max(2)
    }

    /// WRITE → PRECHARGE, same bank: WL + BL/2 + tWR.
    pub fn write_to_precharge(&self) -> u64 {
        self.wl + self.bl / 2 + self.twr
    }

    /// WRITE → READ, any bank: WL + BL/2 + tWTR.
    pub fn write_to_read(&self) -> u64 {
        self.wl + self.bl / 2 + self.twtr
    }

    /// READ → WRITE, any bank: BL/2 + 2, one idle clock between the read data and the write data.
    pub fn read_to_write(&self) -> u64 {
        self.bl / 2 + 2
    }

    /// Column command → column command in one direction: max(tCCD, BL/2), so no burst is cut short.
    pub fn cas_to_cas(&self) -> u64 {
        self.tccd.max(self.bl / 2)
    }

    /// This clocked part as the DRAM table `ddr2_ctrl` and the planner read. The table counts write recovery from
    /// the end of a burst issued with its command, so DDR2's write latency rides in `twr`; it has one turnaround for
    /// both directions, so `tturn` takes the longer, WRITE→READ. WRITE→PRECHARGE and WRITE→READ are then exact,
    /// READ→WRITE is never shorter, and READ→PRECHARGE is the burst, exact while tRTP is two clocks: the table has
    /// no term for a longer one.
    pub fn dram_timings(&self) -> DramTimings {
        let bytes = u64::from(self.grade.dq_bits / 8);
        DramTimings {
            banks: self.grade.banks,
            row_bytes: (1u64 << self.grade.col_bits) * bytes,
            burst_bytes: self.bl * bytes,
            data_cycles: self.bl / 2,
            trcd: self.trcd,
            trp: self.trp,
            tras: self.tras,
            trc: self.trc,
            trrd: self.trrd,
            tfaw: self.tfaw,
            tccd: self.tccd,
            trfc_pb: self.trfc,
            trefi_pb: self.trefi,
            twr: self.wl + self.twr,
            tturn: (self.wl + self.twtr).max(2),
            hot: false,
        }
    }
}

/// The command on the bus for `(CS#, RAS#, CAS#, WE#)` by the JEDEC truth table, `None` for the one reserved
/// pattern (RAS# and CAS# high, WE# low) or a level that is not 0 or 1.
pub fn command_of_pins(cs_n: u8, ras_n: u8, cas_n: u8, we_n: u8) -> Option<Ddr2Cmd> {
    use Ddr2Cmd::*;
    if [cs_n, ras_n, cas_n, we_n].iter().any(|&p| p > 1) {
        return None;
    }
    if cs_n == 1 {
        return Some(Deselect);
    }
    [Nop, Active, Read, Write, Precharge, Refresh, LoadMode].into_iter().find(|&c| ddr2_pins(c) == (cs_n, ras_n, cas_n, we_n))
}

/// One command as the bus carried it: the clock it was sampled on, the command its pins encode, the bank address
/// and the address bus (the row for ACTIVATE, the column and A10 for READ and WRITE, A10 alone for PRECHARGE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusCmd {
    pub cycle: u64,
    pub cmd: Ddr2Cmd,
    pub ba: u8,
    pub a: u16,
}

/// A READ or WRITE, with the row its bank had open and the clock its first data beat occupies the bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CasEvent {
    pub cycle: u64,
    pub write: bool,
    pub bank: u8,
    pub row: u16,
    pub col: u16,
    pub data_at: u64,
}

/// What a legal trace put on the bus. `end` is the clock after its last command or its last data beat, whichever
/// is later; `data_cycles` the clocks its bursts held the data bus.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BusFacts {
    pub activates: u64,
    pub reads: u64,
    pub writes: u64,
    pub precharges: u64,
    pub refreshes: u64,
    pub first_cycle: u64,
    pub last_cycle: u64,
    pub end: u64,
    pub data_cycles: u64,
    pub cas: Vec<CasEvent>,
}

/// The first broken rule: the index of the command that broke it (the trace's length when the trace's end did), the
/// clock, the rule's name, and the earlier command or limit it was measured against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub index: usize,
    pub cycle: u64,
    pub rule: &'static str,
    pub detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "command {} at cycle {} breaks {}: {}", self.index, self.cycle, self.rule, self.detail)
    }
}

#[derive(Clone, Copy)]
struct Bank {
    open: Option<u16>,
    act_at: u64,
    last_act: Option<u64>,
    ready_at: u64,
    ready_rule: &'static str,
    last_rd: Option<u64>,
    last_wr: Option<u64>,
}

/// Hold `trace` to the JESD79-2 command rules of `t`. NOP and DESELECT entries carry no command and are passed
/// over; every other command must come on a later clock than the one before it.
pub fn check_bus(t: &Ddr2Timing, trace: &[BusCmd]) -> Result<BusFacts, Violation> {
    let nb = t.grade.banks as usize;
    let mut banks = vec![
        Bank { open: None, act_at: 0, last_act: None, ready_at: 0, ready_rule: "tRP", last_rd: None, last_wr: None };
        nb
    ];
    let mut acts: Vec<u64> = Vec::new();
    let (mut last_cas, mut last_rd, mut last_wr): (Option<u64>, Option<u64>, Option<u64>) = (None, None, None);
    let mut refreshes = 0u64;
    let mut prev: Option<u64> = None;
    let mut facts = BusFacts::default();
    let mut busy = 0u64;
    let owed = |at: u64| (at / t.trefi).saturating_sub(REFRESH_POSTPONE_MAX);

    for (index, c) in trace.iter().enumerate() {
        if matches!(c.cmd, Ddr2Cmd::Nop | Ddr2Cmd::Deselect) {
            continue;
        }
        let fail = |rule: &'static str, detail: String| Err(Violation { index, cycle: c.cycle, rule, detail });
        if let Some(p) = prev {
            if c.cycle <= p {
                return fail("one command per cycle", format!("the command before it was at cycle {p}"));
            }
        } else {
            facts.first_cycle = c.cycle;
        }
        prev = Some(c.cycle);
        let paid = refreshes + u64::from(c.cmd == Ddr2Cmd::Refresh);
        if owed(c.cycle) > paid {
            return fail("refresh interval", format!("{paid} REFRESH by cycle {}, {} owed", c.cycle, owed(c.cycle)));
        }
        for (b, bank) in banks.iter().enumerate() {
            if bank.open.is_some() && c.cycle - bank.act_at > t.tras_max {
                return fail("tRAS max", format!("bank {b} open since cycle {} (at most {})", bank.act_at, t.tras_max));
            }
        }
        let ba = usize::from(c.ba);
        let banked = matches!(c.cmd, Ddr2Cmd::Active | Ddr2Cmd::Read | Ddr2Cmd::Write)
            || (c.cmd == Ddr2Cmd::Precharge && c.a & (1 << 10) == 0);
        if banked && ba >= nb {
            return fail("address range", format!("bank {ba} of {nb}"));
        }
        match c.cmd {
            Ddr2Cmd::Nop | Ddr2Cmd::Deselect => unreachable!("passed over above"),
            Ddr2Cmd::LoadMode => return fail("mode register", "a mode-register write inside traffic".into()),
            Ddr2Cmd::Active => {
                if u32::from(c.a) >= 1 << t.grade.row_bits {
                    return fail("address range", format!("row {} of {}", c.a, 1u32 << t.grade.row_bits));
                }
                let bank = banks[ba];
                if let Some(row) = bank.open {
                    return fail("ACT to an open bank", format!("bank {ba} has row {row} open since cycle {}", bank.act_at));
                }
                if c.cycle < bank.ready_at {
                    return fail(bank.ready_rule, format!("bank {ba} is ready at cycle {}", bank.ready_at));
                }
                if let Some(la) = bank.last_act {
                    if c.cycle < la + t.trc {
                        return fail("tRC", format!("bank {ba} was activated at cycle {la}"));
                    }
                }
                if let Some(&la) = acts.last() {
                    if c.cycle < la + t.trrd {
                        return fail("tRRD", format!("an ACT at cycle {la}"));
                    }
                }
                if acts.len() >= 4 {
                    let fourth = acts[acts.len() - 4];
                    if c.cycle < fourth + t.tfaw {
                        return fail("tFAW", format!("four ACTs since cycle {fourth}"));
                    }
                }
                acts.push(c.cycle);
                if acts.len() > 4 {
                    acts.remove(0);
                }
                banks[ba] = Bank { open: Some(c.a), act_at: c.cycle, last_act: Some(c.cycle), last_rd: None, last_wr: None, ..bank };
                facts.activates += 1;
            }
            Ddr2Cmd::Read | Ddr2Cmd::Write => {
                let write = c.cmd == Ddr2Cmd::Write;
                let auto = c.a & (1 << 10) != 0;
                let col_mask = (1u16 << t.grade.col_bits) - 1;
                if c.a & !(col_mask | (1 << 10)) != 0 {
                    return fail("address range", format!("column address {:#x} past {} column bits", c.a, t.grade.col_bits));
                }
                let bank = banks[ba];
                let Some(row) = bank.open else {
                    return fail("CAS to an idle bank", format!("bank {ba} has no open row"));
                };
                if c.cycle < bank.act_at + t.trcd {
                    return fail("tRCD", format!("bank {ba} was activated at cycle {}", bank.act_at));
                }
                if let Some(l) = last_cas {
                    if c.cycle < l + t.cas_to_cas() {
                        return fail("CAS to CAS", format!("a column command at cycle {l}"));
                    }
                }
                if !write {
                    if let Some(w) = last_wr {
                        if c.cycle < w + t.write_to_read() {
                            return fail("WRITE to READ", format!("a WRITE at cycle {w}"));
                        }
                    }
                } else if let Some(r) = last_rd {
                    if c.cycle < r + t.read_to_write() {
                        return fail("READ to WRITE", format!("a READ at cycle {r}"));
                    }
                }
                last_cas = Some(c.cycle);
                let data_at = c.cycle + if write { t.wl } else { t.cl };
                if write {
                    last_wr = Some(c.cycle);
                    banks[ba].last_wr = Some(c.cycle);
                    facts.writes += 1;
                } else {
                    last_rd = Some(c.cycle);
                    banks[ba].last_rd = Some(c.cycle);
                    facts.reads += 1;
                }
                facts.data_cycles += t.bl / 2;
                busy = busy.max(data_at + t.bl / 2 - 1);
                facts.cas.push(CasEvent { cycle: c.cycle, write, bank: c.ba, row, col: c.a & col_mask, data_at });
                if auto {
                    let spacing = if write { t.write_to_precharge() } else { t.read_to_precharge() };
                    let start = (c.cycle + spacing).max(bank.act_at + t.tras);
                    let b = &mut banks[ba];
                    b.open = None;
                    if start + t.trp > b.ready_at {
                        b.ready_at = start + t.trp;
                        b.ready_rule = "tRP";
                    }
                }
            }
            Ddr2Cmd::Precharge => {
                let all = c.a & (1 << 10) != 0;
                let (period, rule) = if all { (t.trpa, "tRPA") } else { (t.trp, "tRP") };
                for b in 0..nb {
                    if !all && b != ba {
                        continue;
                    }
                    let bank = banks[b];
                    if bank.open.is_some() {
                        if c.cycle < bank.act_at + t.tras {
                            return fail("tRAS", format!("bank {b} was activated at cycle {}", bank.act_at));
                        }
                        if let Some(r) = bank.last_rd {
                            if c.cycle < r + t.read_to_precharge() {
                                return fail("READ to PRECHARGE", format!("bank {b} read at cycle {r}"));
                            }
                        }
                        if let Some(w) = bank.last_wr {
                            if c.cycle < w + t.write_to_precharge() {
                                return fail("WRITE to PRECHARGE", format!("bank {b} written at cycle {w}"));
                            }
                        }
                    } else if !all {
                        continue;
                    }
                    let bank = &mut banks[b];
                    bank.open = None;
                    if c.cycle + period > bank.ready_at {
                        bank.ready_at = c.cycle + period;
                        bank.ready_rule = rule;
                    }
                }
                facts.precharges += 1;
            }
            Ddr2Cmd::Refresh => {
                for (b, bank) in banks.iter().enumerate() {
                    if let Some(row) = bank.open {
                        return fail("REFRESH with an open bank", format!("bank {b} has row {row} open"));
                    }
                    if c.cycle < bank.ready_at {
                        return fail(bank.ready_rule, format!("bank {b} is ready at cycle {}", bank.ready_at));
                    }
                }
                for bank in banks.iter_mut() {
                    bank.ready_at = c.cycle + t.trfc;
                    bank.ready_rule = "tRFC";
                }
                refreshes += 1;
                facts.refreshes += 1;
            }
        }
        facts.last_cycle = c.cycle;
        busy = busy.max(c.cycle);
    }
    if prev.is_none() {
        return Ok(facts);
    }
    let at_end = |rule: &'static str, detail: String| Err(Violation { index: trace.len(), cycle: busy, rule, detail });
    if owed(busy) > refreshes {
        return at_end("refresh interval", format!("{refreshes} REFRESH by the trace's last busy cycle {busy}, {} owed", owed(busy)));
    }
    for (b, bank) in banks.iter().enumerate() {
        if bank.open.is_some() && busy - bank.act_at > t.tras_max {
            return at_end("tRAS max", format!("bank {b} open since cycle {} at the trace's end", bank.act_at));
        }
    }
    facts.end = busy + 1;
    Ok(facts)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/ddr2_jedec/tests.rs"]
mod tests;
