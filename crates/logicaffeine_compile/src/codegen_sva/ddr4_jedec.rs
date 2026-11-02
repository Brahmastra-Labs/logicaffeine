//! ddr4_jedec — a DDR4 channel's timing at its clock, the JESD79-4 command rules a trace is held to, and the
//! in-order controller that turns line streams into commands on it.
//!
//! [`Ddr4Grade`] is a channel as its controller is configured: its geometry (bank groups, banks, rows, columns, bus
//! width), its latencies (CL, CWL, the command-address parity latency), its datasheet minimums in picoseconds or in
//! clocks as the configuration states them, and how many memory clocks one controller clock spans. [`Ddr4Grade::at`]
//! clocks it: every picosecond minimum rounds up to whole clocks at the memory clock, tRTP to no fewer than four, tRC
//! is tRAS + tRP where the configuration names none, and tREFI rounds down.
//!
//! [`BusChecker`] replays a command trace against the clocked rules, one command at a time, and stops at the first
//! violation; [`check_bus`] runs it over a whole trace. [`Scheduler`] is the controller: one column command a line in
//! the order the lines come, each at the earliest clock its rules and the data bus allow; a line's row opened (and the
//! bank's other row closed) at the earliest clock its rules allow once the line is within [`LOOKAHEAD`] lines of the
//! column commands, so row commands run ahead of the data and hide behind it; one command a clock; an all-bank
//! refresh, after precharging every open bank, as each refresh interval comes due. It releases its commands in clock
//! order once no later decision can come before them, so a stream of any length runs through the checker in bounded
//! memory ([`measure`]).
//!
//! The rules are JESD79-4's for one rank, a fixed burst of eight, additive latency 0 and a one-clock write preamble
//! (the D5005 controller's MEM_DDR4_WRITE_PREAMBLE 1), and agree with Ramulator 2's DDR4 constraint table: read
//! latency CL + PL, write latency CWL + PL; column to column tCCD_L within a bank group and tCCD_S across; WRITE to
//! READ WL + BL/2 + tWTR_L within a bank group and tWTR_S across; READ to WRITE RL + BL/2 − WL + 2; READ to PRECHARGE
//! tRTP; WRITE to PRECHARGE WL + BL/2 + tWR; ACT to column tRCD; ACT to PRECHARGE tRAS; PRECHARGE to ACT or REFRESH
//! tRP; ACT to ACT tRC in one bank, tRRD_L within a bank group, tRRD_S across, and no fifth ACT within tFAW of the
//! fourth before it; REFRESH with every bank precharged, and no command for tRFC after it; no two bursts on the data
//! bus at once. A PRECHARGE to a bank with no open row is legal and changes nothing but its tRP. The refresh account
//! starts at cycle 0 with the device freshly refreshed: each tREFI that passes owes one REFRESH, each REFRESH pays
//! one, at most eight may be owed at any clock and at most eight paid ahead (one past that pays nothing). That bound
//! keeps every row open less than 9 × tREFI, which is tRAS's maximum, so tRAS's maximum needs no rule of its own.

use super::ddr2_jedec::Violation;
use super::dram::StreamDesc;
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};

/// The REFRESH commands JESD79-4 lets a controller owe, and the most it may pay ahead, in fixed 1x mode.
pub const REFRESH_POSTPONE_MAX: i64 = 8;
pub const REFRESH_PULL_IN_MAX: i64 = 8;
/// Beats in a burst: the burst length is fixed at eight (MEM_DDR4_BL_ENUM DDR4_BL_BL8).
pub const BL: u64 = 8;
/// Lines the controller model holds ahead of its column commands, whose rows it may open early. Modeled: the depth
/// of the hard controller's command queue is not in its configuration.
pub const LOOKAHEAD: usize = 16;
/// Where [`Ddr4Grade::D5005_EMIF`] is read from.
pub const D5005_EMIF_IP: &str = "OFS ofs-d5005 2024.1-1 ipss/mem/ip/emif_ddr4_no_ecc.ip";

/// A DDR4 channel as its controller is configured: geometry, latencies and timing minimums (picoseconds unless named
/// in clocks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Ddr4Grade {
    pub part: &'static str,
    pub bank_groups: u32,
    pub banks_per_group: u32,
    pub row_bits: u32,
    pub col_bits: u32,
    /// Data bits the channel carries a transfer.
    pub dq_bits: u32,
    /// Memory clocks a controller clock spans: a controller-side word is one burst.
    pub rate: u64,
    pub mem_mhz: u64,
    pub cl: u64,
    pub cwl: u64,
    /// Command-address parity latency, added to both latencies.
    pub pl: u64,
    pub trcd_ps: u64,
    pub trp_ps: u64,
    pub tras_ps: u64,
    /// ACT to ACT in one bank; `None` for tRAS + tRP.
    pub trc_ps: Option<u64>,
    pub trrd_s_ck: u64,
    pub trrd_l_ck: u64,
    pub tfaw_ps: u64,
    pub tccd_s_ck: u64,
    pub tccd_l_ck: u64,
    pub twr_ps: u64,
    pub twtr_s_ck: u64,
    pub twtr_l_ck: u64,
    pub trtp_ps: u64,
    pub trfc_ps: u64,
    pub trefi_ps: u64,
}

impl Ddr4Grade {
    /// One memory channel of the Intel PAC D5005 as its OFS shell configures it ([`D5005_EMIF_IP`]): a single-rank
    /// RDIMM of x8 devices (MEM_DDR4_FORMAT_ENUM MEM_FORMAT_RDIMM, MEM_DDR4_RANKS_PER_DIMM 1, MEM_DDR4_DQ_PER_DQS 8),
    /// 72 bits wide (MEM_DDR4_DQ_WIDTH 72); 4 bank groups of 4 banks (MEM_DDR4_BANK_GROUP_WIDTH 2,
    /// MEM_DDR4_BANK_ADDR_WIDTH 2), 2^16 rows (MEM_DDR4_ROW_ADDR_WIDTH 16) of 2^10 columns (MEM_DDR4_COL_ADDR_WIDTH 10);
    /// DDR4-2400 (MEM_DDR4_SPEEDBIN_ENUM DDR4_SPEEDBIN_2400) at 1200 MHz (PHY_DDR4_MEM_CLK_FREQ_MHZ 1200.0) under a
    /// quarter-rate controller (PHY_DDR4_RATE_ENUM RATE_QUARTER); CL 21 (MEM_DDR4_TCL), CWL 16 (MEM_DDR4_WTCL),
    /// parity latency 4 (MEM_DDR4_AC_PARITY_LATENCY DDR4_AC_PARITY_LATENCY_4); tRCD and tRP 14.16 ns, tRAS 32 ns,
    /// tFAW 21 ns, tWR 15 ns, tRFC 350 ns, tREFI 7.8 µs (MEM_DDR4_TRCD_NS, _TRP_NS, _TRAS_NS, _TFAW_NS, _TWR_NS,
    /// _TRFC_NS, _TREFI_US); tRRD_S 4, tRRD_L 6, tCCD_S 4, tCCD_L 6, tWTR_S 3, tWTR_L 9 clocks (MEM_DDR4_TRRD_S_CYC,
    /// _TRRD_L_CYC, _TCCD_S_CYC, _TCCD_L_CYC, _TWTR_S_CYC, _TWTR_L_CYC). The configuration names no tRTP: it is
    /// JESD79-4's minimum, the larger of 4 clocks and 7.5 ns.
    pub const D5005_EMIF: Ddr4Grade = Ddr4Grade {
        part: "d5005-emif-ddr4-2400",
        bank_groups: 4,
        banks_per_group: 4,
        row_bits: 16,
        col_bits: 10,
        dq_bits: 72,
        rate: 4,
        mem_mhz: 1200,
        cl: 21,
        cwl: 16,
        pl: 4,
        trcd_ps: 14_160,
        trp_ps: 14_160,
        tras_ps: 32_000,
        trc_ps: None,
        trrd_s_ck: 4,
        trrd_l_ck: 6,
        tfaw_ps: 21_000,
        tccd_s_ck: 4,
        tccd_l_ck: 6,
        twr_ps: 15_000,
        twtr_s_ck: 3,
        twtr_l_ck: 9,
        trtp_ps: 7_500,
        trfc_ps: 350_000,
        trefi_ps: 7_800_000,
    };

    /// The configuration clocked: every picosecond minimum in whole memory clocks, rounded up.
    pub fn at(&self) -> Ddr4Timing {
        let up = |ps: u64| (ps * self.mem_mhz).div_ceil(1_000_000);
        Ddr4Timing {
            grade: *self,
            rl: self.cl + self.pl,
            wl: self.cwl + self.pl,
            trcd: up(self.trcd_ps),
            trp: up(self.trp_ps),
            tras: up(self.tras_ps),
            trc: up(self.trc_ps.unwrap_or(self.tras_ps + self.trp_ps)),
            trrd_s: self.trrd_s_ck,
            trrd_l: self.trrd_l_ck,
            tfaw: up(self.tfaw_ps),
            tccd_s: self.tccd_s_ck.max(BL / 2),
            tccd_l: self.tccd_l_ck.max(BL / 2),
            twr: up(self.twr_ps),
            twtr_s: self.twtr_s_ck,
            twtr_l: self.twtr_l_ck,
            trtp: up(self.trtp_ps).max(4),
            trfc: up(self.trfc_ps),
            trefi: self.trefi_ps * self.mem_mhz / 1_000_000,
        }
    }
}

/// A channel clocked: every rule in whole memory clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Ddr4Timing {
    pub grade: Ddr4Grade,
    pub rl: u64,
    pub wl: u64,
    pub trcd: u64,
    pub trp: u64,
    pub tras: u64,
    pub trc: u64,
    pub trrd_s: u64,
    pub trrd_l: u64,
    pub tfaw: u64,
    pub tccd_s: u64,
    pub tccd_l: u64,
    pub twr: u64,
    pub twtr_s: u64,
    pub twtr_l: u64,
    pub trtp: u64,
    pub trfc: u64,
    pub trefi: u64,
}

impl Ddr4Timing {
    pub fn banks(&self) -> u32 {
        self.grade.bank_groups * self.grade.banks_per_group
    }

    /// Lines (bursts) the channel addresses: every bank's rows' columns in bursts of eight.
    pub fn lines(&self) -> u64 {
        u64::from(self.banks()) << (self.grade.row_bits + self.grade.col_bits - BL.trailing_zeros())
    }

    /// Bits a burst moves: eight transfers of the channel's width.
    pub fn burst_bits(&self) -> u64 {
        BL * u64::from(self.grade.dq_bits)
    }

    /// Clocks a burst holds the data bus.
    pub fn burst_ck(&self) -> u64 {
        BL / 2
    }

    /// WRITE to READ: WL + BL/2 + tWTR_L within a bank group, tWTR_S across.
    pub fn write_to_read(&self, same_group: bool) -> u64 {
        self.wl + BL / 2 + if same_group { self.twtr_l } else { self.twtr_s }
    }

    /// READ to WRITE, any bank: RL + BL/2 − WL + 2.
    pub fn read_to_write(&self) -> u64 {
        self.rl + BL / 2 + 2 - self.wl
    }

    /// WRITE to PRECHARGE, one bank: WL + BL/2 + tWR.
    pub fn write_to_precharge(&self) -> u64 {
        self.wl + BL / 2 + self.twr
    }

    /// Controller clocks `mem_ck` memory clocks take, rounded up.
    pub fn controller_ck(&self, mem_ck: u64) -> u64 {
        mem_ck.div_ceil(self.grade.rate)
    }

    /// Bytes per second the data bus carries at full use, `line_bytes` of every burst being data.
    pub fn peak_bytes_per_s(&self, line_bytes: u64) -> f64 {
        line_bytes as f64 * self.grade.mem_mhz as f64 * 1e6 / self.burst_ck() as f64
    }
}

/// Where a line lives: bank group, bank within it, row, and the burst's first column (a multiple of eight).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    pub group: u32,
    pub bank: u32,
    pub row: u64,
    pub col: u64,
}

/// A line (one burst) by the controller's address order, chip select, row, bank, column, bank group from the most
/// significant end (CTRL_DDR4_ADDR_ORDER_ENUM DDR4_CTRL_ADDR_ORDER_CS_R_B_C_BG): consecutive lines take the bank
/// groups in turn, then the bursts of a row, then the banks, then the rows. Lines past the channel's last wrap.
pub fn locate(t: &Ddr4Timing, line: u64) -> Location {
    let g = t.grade;
    let gbits = g.bank_groups.trailing_zeros();
    let cbits = g.col_bits - BL.trailing_zeros();
    let bbits = g.banks_per_group.trailing_zeros();
    let field = |shift: u32, bits: u32| (line >> shift) & ((1u64 << bits) - 1);
    Location {
        group: field(0, gbits) as u32,
        col: field(gbits, cbits) << BL.trailing_zeros(),
        bank: field(gbits + cbits, bbits) as u32,
        row: field(gbits + cbits + bbits, g.row_bits),
    }
}

/// One command on the channel. `PreAll` precharges every bank; `Ref` is an all-bank refresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ddr4Cmd {
    Act { group: u32, bank: u32, row: u64 },
    Rd { group: u32, bank: u32, col: u64 },
    Wr { group: u32, bank: u32, col: u64 },
    Pre { group: u32, bank: u32 },
    PreAll,
    Ref,
}

/// A command and the memory clock it issues on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ddr4Timed {
    pub at: u64,
    pub cmd: Ddr4Cmd,
}

/// What a legal trace put on the channel: its commands by kind, the bursts that turned the data bus around, the clocks
/// its bursts held the data bus, and the clock after its last data or command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Ddr4Facts {
    pub activates: u64,
    pub reads: u64,
    pub writes: u64,
    pub precharges: u64,
    pub refreshes: u64,
    pub turnarounds: u64,
    pub data_ck: u64,
    pub end: u64,
}

impl Ddr4Facts {
    /// The share of the trace's clocks the data bus carried data.
    pub fn efficiency(&self) -> f64 {
        if self.end == 0 {
            0.0
        } else {
            self.data_ck as f64 / self.end as f64
        }
    }

    /// Bytes per second the trace moved, `line_bytes` of every burst being data.
    pub fn bytes_per_s(&self, t: &Ddr4Timing, line_bytes: u64) -> f64 {
        self.efficiency() * t.peak_bytes_per_s(line_bytes)
    }
}

#[derive(Clone, Copy)]
struct CheckedBank {
    open: Option<u64>,
    act_at: u64,
    last_act: Option<u64>,
    ready_at: u64,
    last_rd: Option<u64>,
    last_wr: Option<u64>,
}

/// The JESD79-4 rules of one channel, held over a trace one command at a time.
pub struct BusChecker {
    t: Ddr4Timing,
    banks: Vec<CheckedBank>,
    last_act: Option<u64>,
    last_act_group: Vec<Option<u64>>,
    faw: VecDeque<u64>,
    last_cas: Option<u64>,
    last_cas_group: Vec<Option<u64>>,
    last_rd: Option<u64>,
    last_wr_group: Vec<Option<u64>>,
    last_dir: Option<bool>,
    data_free: u64,
    refresh_until: u64,
    /// REFRESH paid minus REFRESH owed, as of `account_at`.
    balance: i64,
    account_at: u64,
    prev: Option<u64>,
    index: usize,
    facts: Ddr4Facts,
}

impl BusChecker {
    pub fn new(t: &Ddr4Timing) -> BusChecker {
        let groups = t.grade.bank_groups as usize;
        let bank = CheckedBank { open: None, act_at: 0, last_act: None, ready_at: 0, last_rd: None, last_wr: None };
        BusChecker {
            t: *t,
            banks: vec![bank; t.banks() as usize],
            last_act: None,
            last_act_group: vec![None; groups],
            faw: VecDeque::with_capacity(4),
            last_cas: None,
            last_cas_group: vec![None; groups],
            last_rd: None,
            last_wr_group: vec![None; groups],
            last_dir: None,
            data_free: 0,
            refresh_until: 0,
            balance: 0,
            account_at: 0,
            prev: None,
            index: 0,
            facts: Ddr4Facts::default(),
        }
    }

    fn violation(&self, cycle: u64, rule: &'static str, detail: String) -> Violation {
        Violation { index: self.index, cycle, rule, detail }
    }

    /// Bring the refresh account to clock `at`: one REFRESH owed for every tREFI boundary passed.
    fn owe_until(&mut self, at: u64) {
        self.balance -= (at / self.t.trefi - self.account_at / self.t.trefi) as i64;
        self.account_at = at;
    }

    fn bank(&self, group: u32, bank: u32, at: u64) -> Result<usize, Violation> {
        let g = self.t.grade;
        if group >= g.bank_groups || bank >= g.banks_per_group {
            return Err(self.violation(at, "address range", format!("bank group {group} bank {bank}")));
        }
        Ok((group * g.banks_per_group + bank) as usize)
    }

    /// Hold the trace's next command to every rule.
    pub fn push(&mut self, c: Ddr4Timed) -> Result<(), Violation> {
        let t = self.t;
        let g = t.grade;
        let at = c.at;
        if let Some(p) = self.prev {
            if at <= p {
                return Err(self.violation(at, "one command per clock", format!("the command before it was at clock {p}")));
            }
        }
        if at < self.refresh_until {
            return Err(self.violation(at, "tRFC", format!("a REFRESH holds the channel until clock {}", self.refresh_until)));
        }
        self.owe_until(at);
        let paid = self.balance + i64::from(c.cmd == Ddr4Cmd::Ref);
        if paid < -REFRESH_POSTPONE_MAX {
            return Err(self.violation(at, "refresh postponed", format!("{} REFRESH owed at clock {at}", -paid)));
        }
        match c.cmd {
            Ddr4Cmd::Act { group, bank, row } => {
                let b = self.bank(group, bank, at)?;
                if row >= 1 << g.row_bits {
                    return Err(self.violation(at, "address range", format!("row {row} of {}", 1u64 << g.row_bits)));
                }
                let s = self.banks[b];
                if let Some(open) = s.open {
                    return Err(self.violation(at, "ACT to an open bank", format!("bank {b} has row {open} open since clock {}", s.act_at)));
                }
                if at < s.ready_at {
                    return Err(self.violation(at, "tRP", format!("bank {b} was precharged at clock {}", s.ready_at - t.trp)));
                }
                if let Some(la) = s.last_act {
                    if at < la + t.trc {
                        return Err(self.violation(at, "tRC", format!("bank {b} was activated at clock {la}")));
                    }
                }
                if let Some(la) = self.last_act_group[group as usize] {
                    if at < la + t.trrd_l {
                        return Err(self.violation(at, "tRRD_L", format!("an ACT to bank group {group} at clock {la}")));
                    }
                }
                if let Some(la) = self.last_act {
                    if at < la + t.trrd_s {
                        return Err(self.violation(at, "tRRD_S", format!("an ACT at clock {la}")));
                    }
                }
                if self.faw.len() == 4 {
                    if at < self.faw[0] + t.tfaw {
                        return Err(self.violation(at, "tFAW", format!("four ACTs since clock {}", self.faw[0])));
                    }
                    self.faw.pop_front();
                }
                self.faw.push_back(at);
                self.last_act = Some(at);
                self.last_act_group[group as usize] = Some(at);
                self.banks[b] = CheckedBank { open: Some(row), act_at: at, last_act: Some(at), last_rd: None, last_wr: None, ..s };
                self.facts.activates += 1;
            }
            Ddr4Cmd::Rd { group, bank, col } | Ddr4Cmd::Wr { group, bank, col } => {
                let write = matches!(c.cmd, Ddr4Cmd::Wr { .. });
                let b = self.bank(group, bank, at)?;
                if col >= 1 << g.col_bits || col % BL != 0 {
                    return Err(self.violation(at, "address range", format!("column {col} of {} in bursts of {BL}", 1u64 << g.col_bits)));
                }
                let s = self.banks[b];
                if s.open.is_none() {
                    return Err(self.violation(at, "column command to an idle bank", format!("bank {b} has no open row")));
                }
                if at < s.act_at + t.trcd {
                    return Err(self.violation(at, "tRCD", format!("bank {b} was activated at clock {}", s.act_at)));
                }
                if let Some(l) = self.last_cas_group[group as usize] {
                    if at < l + t.tccd_l {
                        return Err(self.violation(at, "tCCD_L", format!("a column command to bank group {group} at clock {l}")));
                    }
                }
                if let Some(l) = self.last_cas {
                    if at < l + t.tccd_s {
                        return Err(self.violation(at, "tCCD_S", format!("a column command at clock {l}")));
                    }
                }
                if write {
                    if let Some(r) = self.last_rd {
                        if at < r + t.read_to_write() {
                            return Err(self.violation(at, "READ to WRITE", format!("a READ at clock {r}")));
                        }
                    }
                } else {
                    for (grp, w) in self.last_wr_group.iter().enumerate() {
                        if let Some(w) = *w {
                            let same = grp as u32 == group;
                            if at < w + t.write_to_read(same) {
                                let rule = if same { "tWTR_L" } else { "tWTR_S" };
                                return Err(self.violation(at, rule, format!("a WRITE to bank group {grp} at clock {w}")));
                            }
                        }
                    }
                }
                let data_at = at + if write { t.wl } else { t.rl };
                if data_at < self.data_free {
                    return Err(self.violation(at, "data bus", format!("a burst holds the data bus until clock {}", self.data_free)));
                }
                self.data_free = data_at + t.burst_ck();
                if self.last_dir.is_some_and(|d| d != write) {
                    self.facts.turnarounds += 1;
                }
                self.last_dir = Some(write);
                self.last_cas = Some(at);
                self.last_cas_group[group as usize] = Some(at);
                if write {
                    self.last_wr_group[group as usize] = Some(at);
                    self.banks[b].last_wr = Some(at);
                    self.facts.writes += 1;
                } else {
                    self.last_rd = Some(at);
                    self.banks[b].last_rd = Some(at);
                    self.facts.reads += 1;
                }
                self.facts.data_ck += t.burst_ck();
                self.facts.end = self.facts.end.max(self.data_free);
            }
            Ddr4Cmd::Pre { .. } | Ddr4Cmd::PreAll => {
                let targets = match c.cmd {
                    Ddr4Cmd::Pre { group, bank } => {
                        let b = self.bank(group, bank, at)?;
                        b..b + 1
                    }
                    _ => 0..self.banks.len(),
                };
                for b in targets {
                    let s = self.banks[b];
                    if s.open.is_some() {
                        if at < s.act_at + t.tras {
                            return Err(self.violation(at, "tRAS", format!("bank {b} was activated at clock {}", s.act_at)));
                        }
                        if let Some(r) = s.last_rd {
                            if at < r + t.trtp {
                                return Err(self.violation(at, "tRTP", format!("bank {b} was read at clock {r}")));
                            }
                        }
                        if let Some(w) = s.last_wr {
                            if at < w + t.write_to_precharge() {
                                return Err(self.violation(at, "tWR", format!("bank {b} was written at clock {w}")));
                            }
                        }
                    }
                    self.banks[b].open = None;
                    self.banks[b].ready_at = at + t.trp;
                }
                self.facts.precharges += 1;
            }
            Ddr4Cmd::Ref => {
                for (b, s) in self.banks.iter().enumerate() {
                    if let Some(row) = s.open {
                        return Err(self.violation(at, "REFRESH with an open bank", format!("bank {b} has row {row} open")));
                    }
                    if at < s.ready_at {
                        return Err(self.violation(at, "tRP", format!("bank {b} was precharged at clock {}", s.ready_at - t.trp)));
                    }
                }
                self.refresh_until = at + t.trfc;
                self.balance = (self.balance + 1).min(REFRESH_PULL_IN_MAX);
                self.facts.refreshes += 1;
            }
        }
        self.prev = Some(at);
        self.facts.end = self.facts.end.max(at + 1);
        self.index += 1;
        Ok(())
    }

    /// The trace ends: the refresh account must hold at its last clock too.
    pub fn finish(mut self) -> Result<Ddr4Facts, Violation> {
        let end = self.facts.end;
        self.owe_until(end);
        if self.balance < -REFRESH_POSTPONE_MAX {
            return Err(self.violation(end, "refresh postponed", format!("{} REFRESH owed at the trace's end, clock {end}", -self.balance)));
        }
        Ok(self.facts)
    }
}

/// Hold `trace` to the JESD79-4 rules of `t`: the first violation, or what the trace put on the channel.
pub fn check_bus(t: &Ddr4Timing, trace: &[Ddr4Timed]) -> Result<Ddr4Facts, Violation> {
    let mut checker = BusChecker::new(t);
    for &c in trace {
        checker.push(c)?;
    }
    checker.finish()
}

/// The lines a stream touches, in its order, with its direction: every line any of its elements covers, a line that
/// consecutive elements share once.
pub fn stream_lines(s: &StreamDesc, line_bytes: u64) -> impl Iterator<Item = (u64, bool)> + '_ {
    let mut last = None;
    (0..s.count)
        .flat_map(move |i| {
            let start = s.base + i * s.stride_bytes;
            start / line_bytes..(start + s.elem_bytes.max(1)).div_ceil(line_bytes)
        })
        .filter(move |&line| last.replace(line) != Some(line))
        .map(move |line| (line, s.write))
}

#[derive(Clone, Copy, Default)]
struct OpenBank {
    row: Option<u64>,
    act_at: u64,
    last_act: Option<u64>,
    ready_at: u64,
    last_rd: Option<u64>,
    last_wr: Option<u64>,
    /// The bank's last command: no later command of its own comes before it.
    last_use: u64,
}

/// The controller model: lines in, commands out in clock order.
pub struct Scheduler {
    t: Ddr4Timing,
    lookahead: usize,
    /// Placed commands not yet released, by clock: a clock holds one command.
    pending: BTreeMap<u64, Ddr4Cmd>,
    /// Every clock below this is released and no command is placed below it. After each line it stands at the next
    /// line's place in the queue (the clock after the column command `lookahead` lines back) or past it, so it holds
    /// a line's row commands until the line enters the queue.
    released: u64,
    /// The column clocks of the last `lookahead` lines.
    recent: VecDeque<u64>,
    banks: Vec<OpenBank>,
    last_act: Option<u64>,
    last_act_group: Vec<Option<u64>>,
    faw: VecDeque<u64>,
    last_cas: Option<u64>,
    last_cas_group: Vec<Option<u64>>,
    last_rd: Option<u64>,
    last_wr_group: Vec<Option<u64>>,
    data_free: u64,
    refresh_until: u64,
    next_refresh: u64,
}

impl Scheduler {
    pub fn new(t: &Ddr4Timing, lookahead: usize) -> Scheduler {
        let groups = t.grade.bank_groups as usize;
        let lookahead = lookahead.max(1);
        Scheduler {
            t: *t,
            lookahead,
            pending: BTreeMap::new(),
            released: 0,
            recent: VecDeque::with_capacity(lookahead),
            banks: vec![OpenBank::default(); t.banks() as usize],
            last_act: None,
            last_act_group: vec![None; groups],
            faw: VecDeque::with_capacity(4),
            last_cas: None,
            last_cas_group: vec![None; groups],
            last_rd: None,
            last_wr_group: vec![None; groups],
            data_free: 0,
            refresh_until: 0,
            next_refresh: t.trefi,
        }
    }

    /// The clock after the last column command: every later column command and refresh comes at or after it.
    fn frontier(&self) -> u64 {
        self.last_cas.map_or(0, |c| c + 1)
    }

    /// The clock after the column command of the line `lookahead` lines back: the next line's row commands wait for
    /// it, the line entering the queue as that one leaves.
    fn queued(&self) -> u64 {
        if self.recent.len() == self.lookahead {
            self.recent[0] + 1
        } else {
            0
        }
    }

    fn place(&mut self, earliest: u64, cmd: Ddr4Cmd) -> u64 {
        let mut at = earliest.max(self.released).max(self.refresh_until);
        while self.pending.contains_key(&at) {
            at += 1;
        }
        self.pending.insert(at, cmd);
        at
    }

    /// The earliest clock bank `b`'s open row may close.
    fn close_from(&self, b: usize) -> u64 {
        let (t, s) = (&self.t, &self.banks[b]);
        let mut at = (s.act_at + t.tras).max(s.last_use + 1);
        if let Some(r) = s.last_rd {
            at = at.max(r + t.trtp);
        }
        if let Some(w) = s.last_wr {
            at = at.max(w + t.write_to_precharge());
        }
        at
    }

    /// The refresh now due: every open bank precharged, then the REFRESH, neither before its interval comes due.
    fn refresh(&mut self) {
        let t = self.t;
        let due = self.frontier().max(self.next_refresh);
        let open: Vec<usize> = (0..self.banks.len()).filter(|&b| self.banks[b].row.is_some()).collect();
        if !open.is_empty() {
            let pre_at = open.iter().map(|&b| self.close_from(b)).fold(due, u64::max);
            let p = self.place(pre_at, Ddr4Cmd::PreAll);
            for s in &mut self.banks {
                s.row = None;
                s.ready_at = p + t.trp;
                s.last_use = p;
            }
        }
        let ref_at = self.banks.iter().map(|s| s.ready_at.max(s.last_use + 1)).fold(due, u64::max);
        let r = self.place(ref_at, Ddr4Cmd::Ref);
        for s in &mut self.banks {
            s.last_use = r;
        }
        self.refresh_until = r + t.trfc;
        self.next_refresh += t.trefi;
    }

    /// Issue one line: a READ, or a WRITE when `write`, opening its row first when it is not the bank's open one. The
    /// clock of its column command.
    pub fn line(&mut self, line: u64, write: bool, sink: &mut dyn FnMut(Ddr4Timed)) -> u64 {
        self.line_at(line, write, 0, sink)
    }

    /// Issue one line the controller takes at clock `arrive`: none of its commands comes before it, and a refresh
    /// that came due while the channel waited for it is issued first. The clock of its column command.
    pub fn line_at(&mut self, line: u64, write: bool, arrive: u64, sink: &mut dyn FnMut(Ddr4Timed)) -> u64 {
        let t = self.t;
        let g = t.grade;
        while self.frontier().max(arrive) >= self.next_refresh {
            self.refresh();
        }
        let loc = locate(&t, line);
        let b = (loc.group * g.banks_per_group + loc.bank) as usize;
        if self.banks[b].row != Some(loc.row) {
            if self.banks[b].row.is_some() {
                let pre_at = self.close_from(b).max(arrive);
                let p = self.place(pre_at, Ddr4Cmd::Pre { group: loc.group, bank: loc.bank });
                let s = &mut self.banks[b];
                s.row = None;
                s.ready_at = p + t.trp;
                s.last_use = p;
            }
            let s = self.banks[b];
            let mut act_at = s.ready_at.max(s.last_use + 1).max(arrive);
            if let Some(la) = s.last_act {
                act_at = act_at.max(la + t.trc);
            }
            if let Some(la) = self.last_act_group[loc.group as usize] {
                act_at = act_at.max(la + t.trrd_l);
            }
            if let Some(la) = self.last_act {
                act_at = act_at.max(la + t.trrd_s);
            }
            if self.faw.len() == 4 {
                act_at = act_at.max(self.faw[0] + t.tfaw);
            }
            let a = self.place(act_at, Ddr4Cmd::Act { group: loc.group, bank: loc.bank, row: loc.row });
            if self.faw.len() == 4 {
                self.faw.pop_front();
            }
            self.faw.push_back(a);
            self.last_act = Some(a);
            self.last_act_group[loc.group as usize] = Some(a);
            self.banks[b] = OpenBank { row: Some(loc.row), act_at: a, last_act: Some(a), ready_at: s.ready_at, last_rd: None, last_wr: None, last_use: a };
        }
        let latency = if write { t.wl } else { t.rl };
        let mut cas_at = (self.banks[b].act_at + t.trcd).max(self.frontier()).max(arrive);
        if let Some(l) = self.last_cas_group[loc.group as usize] {
            cas_at = cas_at.max(l + t.tccd_l);
        }
        if let Some(l) = self.last_cas {
            cas_at = cas_at.max(l + t.tccd_s);
        }
        if write {
            if let Some(r) = self.last_rd {
                cas_at = cas_at.max(r + t.read_to_write());
            }
        } else {
            for (grp, w) in self.last_wr_group.iter().enumerate() {
                if let Some(w) = *w {
                    cas_at = cas_at.max(w + t.write_to_read(grp as u32 == loc.group));
                }
            }
        }
        cas_at = cas_at.max(self.data_free.saturating_sub(latency));
        let cmd = if write {
            Ddr4Cmd::Wr { group: loc.group, bank: loc.bank, col: loc.col }
        } else {
            Ddr4Cmd::Rd { group: loc.group, bank: loc.bank, col: loc.col }
        };
        let c = self.place(cas_at, cmd);
        self.data_free = c + latency + t.burst_ck();
        self.last_cas = Some(c);
        self.last_cas_group[loc.group as usize] = Some(c);
        if write {
            self.last_wr_group[loc.group as usize] = Some(c);
            self.banks[b].last_wr = Some(c);
        } else {
            self.last_rd = Some(c);
            self.banks[b].last_rd = Some(c);
        }
        self.banks[b].last_use = c;
        if self.recent.len() == self.lookahead {
            self.recent.pop_front();
        }
        self.recent.push_back(c);
        self.release(sink);
        c
    }

    /// Hand `sink`, in clock order, every placed command below the earliest clock a later one may take: a later column
    /// command or refresh comes after the last column command; a later ACT after the last ACT and a PRECHARGE after its
    /// bank's last command, each no earlier than the next line's place in the queue. Every clock before that place is
    /// so released, and no command placed after it: that is the floor under the next line's row commands.
    fn release(&mut self, sink: &mut dyn FnMut(Ddr4Timed)) {
        let queued = self.queued();
        let mut low = self.frontier().min(self.last_act.map_or(0, |a| a + 1).max(queued));
        for s in &self.banks {
            if s.row.is_some() {
                low = low.min((s.last_use + 1).max(queued));
            }
        }
        if low <= self.released {
            return;
        }
        let later = self.pending.split_off(&low);
        for (at, cmd) in std::mem::replace(&mut self.pending, later) {
            sink(Ddr4Timed { at, cmd });
        }
        self.released = low;
    }

    /// Every command still held, in clock order.
    pub fn finish(mut self, sink: &mut dyn FnMut(Ddr4Timed)) {
        for (at, cmd) in std::mem::take(&mut self.pending) {
            sink(Ddr4Timed { at, cmd });
        }
    }
}

/// The controller's trace for `streams` (byte addresses in one channel, `line_bytes` to a burst) at [`LOOKAHEAD`].
pub fn schedule(t: &Ddr4Timing, line_bytes: u64, streams: &[StreamDesc]) -> Vec<Ddr4Timed> {
    let mut out = Vec::new();
    let mut sched = Scheduler::new(t, LOOKAHEAD);
    let mut sink = |c: Ddr4Timed| out.push(c);
    for s in streams {
        for (line, write) in stream_lines(s, line_bytes) {
            sched.line(line, write, &mut sink);
        }
    }
    sched.finish(&mut sink);
    out
}

/// The controller's trace for `streams` held to the rules as it is issued, never held whole: what it put on the
/// channel, or the first rule it broke.
pub fn measure(t: &Ddr4Timing, line_bytes: u64, streams: &[StreamDesc]) -> Result<Ddr4Facts, Violation> {
    let mut checker = BusChecker::new(t);
    let mut broke: Option<Violation> = None;
    {
        let mut sched = Scheduler::new(t, LOOKAHEAD);
        let mut sink = |c: Ddr4Timed| {
            if broke.is_none() {
                broke = checker.push(c).err();
            }
        };
        for s in streams {
            for (line, write) in stream_lines(s, line_bytes) {
                sched.line(line, write, &mut sink);
            }
        }
        sched.finish(&mut sink);
    }
    match broke {
        Some(v) => Err(v),
        None => checker.finish(),
    }
}

/// Clocks a command kind is logged as by [`verilog_model`]: ACT, READ, WRITE, PRECHARGE, PRECHARGE ALL, REFRESH.
pub const LOGGED_KINDS: [&str; 6] = ["ACT", "RD", "WR", "PRE", "PREA", "REF"];

/// A DDR4 channel behind an Avalon-MM slave for simulation, scheduled by this module's controller model: every line
/// a burst brings is issued as [`Scheduler::line_at`] issues it at the memory clock the slave takes it (four to a
/// controller clock), a write's data is stored as it is taken, and a read's data, as the memory held it when the read
/// was taken, returns in order `PIPE` controller clocks after the controller clock its burst leaves the data bus in.
/// With `LOG` set it writes, to `<LOGFILE>_<ID>.log`, every line it schedules (`L line write arrive column`) and every
/// command it places (`C clock kind group bank row-or-column`, the kind an index of [`LOGGED_KINDS`]).
pub fn verilog_model(t: &Ddr4Timing, lookahead: usize) -> String {
    let g = t.grade;
    let gbits = g.bank_groups.trailing_zeros();
    let cbits = g.col_bits - BL.trailing_zeros();
    let bbits = g.banks_per_group.trailing_zeros();
    let banks = t.banks();
    let groups = g.bank_groups;
    format!(
        r#"// A DDR4 channel ({part}) behind an Avalon-MM slave, for simulation: the in-order controller model of
// codegen_sva::ddr4_jedec schedules every line at the memory clock the slave takes it, {rate} memory clocks a controller
// clock; a read's data returns in order PIPE controller clocks after its burst leaves the data bus.
module logos_ddr4_channel_model #(parameter AW = 27, parameter DW = 512, parameter BW = 7, parameter LINES_LOG2 = 12,
                                  parameter PIPE = 0, parameter QUEUE = 1024, parameter LOG = 0,
                                  parameter LOGFILE = "ddr4_channel", parameter ID = 0) (
  input               clk,
  input               rst_n,
  input  [AW-1:0]     address,
  input               read,
  input               write,
  input  [BW-1:0]     burstcount,
  input  [DW-1:0]     writedata,
  input  [DW/8-1:0]   byteenable,
  output reg          waitrequest,
  output reg [DW-1:0] readdata,
  output reg          readdatavalid
);
  localparam RATE = {rate}, RL = {rl}, WL = {wl}, TRCD = {trcd}, TRP = {trp}, TRAS = {tras}, TRC = {trc};
  localparam TRRD_S = {trrd_s}, TRRD_L = {trrd_l}, TFAW = {tfaw}, TCCD_S = {tccd_s}, TCCD_L = {tccd_l};
  localparam TRTP = {trtp}, WTP = {wtp}, RTW = {rtw}, WTR_S = {wtr_s}, WTR_L = {wtr_l}, TRFC = {trfc}, TREFI = {trefi};
  localparam BURST = {burst}, NB = {banks}, NG = {groups}, LOOK = {lookahead}, RING = 65536;
  reg [DW-1:0] mem [0:(1<<LINES_LOG2)-1];
  integer i, logf;
  reg [8*256-1:0] log_name;
  initial begin
    for (i = 0; i < (1 << LINES_LOG2); i = i + 1) mem[i] = {{DW{{1'bx}}}};
    if (LOG) begin
      $sformat(log_name, "%0s_%0d.log", LOGFILE, ID);
      logf = $fopen(log_name, "w");
    end
  end

  // The controller model's state, as the scheduler keeps it.
  reg        b_open [0:NB-1];
  reg [63:0] b_row [0:NB-1], b_act_at [0:NB-1], b_last_act [0:NB-1], b_ready_at [0:NB-1];
  reg [63:0] b_last_rd [0:NB-1], b_last_wr [0:NB-1], b_last_use [0:NB-1];
  reg        b_has_act [0:NB-1], b_has_rd [0:NB-1], b_has_wr [0:NB-1];
  reg        has_act, has_cas, has_rd;
  reg [63:0] last_act, last_cas, last_rd, data_free, refresh_until, next_refresh, released;
  reg        g_has_act [0:NG-1], g_has_cas [0:NG-1], g_has_wr [0:NG-1];
  reg [63:0] g_last_act [0:NG-1], g_last_cas [0:NG-1], g_last_wr [0:NG-1];
  reg [63:0] faw [0:3];
  integer    faw_n, rec_n, rec_h;
  reg [63:0] recent [0:LOOK-1];
  reg        slot_v [0:RING-1];
  reg [63:0] slot_t [0:RING-1];
  initial begin
    for (i = 0; i < NB; i = i + 1) begin
      b_open[i] = 0; b_has_act[i] = 0; b_has_rd[i] = 0; b_has_wr[i] = 0;
      b_row[i] = 0; b_act_at[i] = 0; b_last_act[i] = 0; b_ready_at[i] = 0; b_last_rd[i] = 0; b_last_wr[i] = 0;
      b_last_use[i] = 0;
    end
    for (i = 0; i < NG; i = i + 1) begin
      g_has_act[i] = 0; g_has_cas[i] = 0; g_has_wr[i] = 0; g_last_act[i] = 0; g_last_cas[i] = 0; g_last_wr[i] = 0;
    end
    for (i = 0; i < RING; i = i + 1) slot_v[i] = 0;
    has_act = 0; has_cas = 0; has_rd = 0; last_act = 0; last_cas = 0; last_rd = 0; data_free = 0;
    refresh_until = 0; next_refresh = TREFI; released = 0; faw_n = 0; rec_n = 0; rec_h = 0;
  end

  function [63:0] max2(input [63:0] a, input [63:0] b);
    max2 = a > b ? a : b;
  endfunction
  function [63:0] frontier(input dummy);
    frontier = has_cas ? last_cas + 1 : 0;
  endfunction
  function [63:0] queued(input dummy);
    queued = rec_n == LOOK ? recent[rec_h] + 1 : 0;
  endfunction
  function [63:0] close_from(input integer b);
    begin
      close_from = max2(b_act_at[b] + TRAS, b_last_use[b] + 1);
      if (b_has_rd[b]) close_from = max2(close_from, b_last_rd[b] + TRTP);
      if (b_has_wr[b]) close_from = max2(close_from, b_last_wr[b] + WTP);
    end
  endfunction

  task place(input [63:0] earliest, input integer kind, input integer grp, input integer bnk, input [63:0] arg,
             output [63:0] at);
    begin
      at = max2(max2(earliest, released), refresh_until);
      while (slot_v[at % RING] && slot_t[at % RING] == at) at = at + 1;
      slot_v[at % RING] = 1;
      slot_t[at % RING] = at;
      if (LOG) $fwrite(logf, "C %0d %0d %0d %0d %0d\n", at, kind, grp, bnk, arg);
    end
  endtask

  task refresh;
    reg [63:0] due, pre_at, ref_at, p, r;
    reg any_open;
    integer b;
    begin
      due = max2(frontier(0), next_refresh);
      any_open = 0;
      pre_at = due;
      for (b = 0; b < NB; b = b + 1) if (b_open[b]) begin
        any_open = 1;
        pre_at = max2(pre_at, close_from(b));
      end
      if (any_open) begin
        place(pre_at, 4, 0, 0, 0, p);
        for (b = 0; b < NB; b = b + 1) begin
          b_open[b] = 0;
          b_ready_at[b] = p + TRP;
          b_last_use[b] = p;
        end
      end
      ref_at = due;
      for (b = 0; b < NB; b = b + 1) ref_at = max2(ref_at, max2(b_ready_at[b], b_last_use[b] + 1));
      place(ref_at, 5, 0, 0, 0, r);
      for (b = 0; b < NB; b = b + 1) b_last_use[b] = r;
      refresh_until = r + TRFC;
      next_refresh = next_refresh + TREFI;
    end
  endtask

  task release_floor;
    reg [63:0] q, low;
    integer b;
    begin
      q = queued(0);
      low = frontier(0);
      low = low < max2(has_act ? last_act + 1 : 0, q) ? low : max2(has_act ? last_act + 1 : 0, q);
      for (b = 0; b < NB; b = b + 1) if (b_open[b] && max2(b_last_use[b] + 1, q) < low) low = max2(b_last_use[b] + 1, q);
      if (low > released) released = low;
    end
  endtask

  task line_at(input [63:0] line, input wr, input [63:0] arrive, output [63:0] c);
    reg [63:0] row, col, pre_at, act_at, cas_at, p, a, latency;
    integer grp, bnk, b, k;
    begin
      while (max2(frontier(0), arrive) >= next_refresh) refresh;
      grp = line[{gb1}:0];
      col = line[{c_hi}:{gbits}] << 3;
      bnk = line[{b_hi}:{b_lo}];
      row = line[{r_hi}:{r_lo}];
      b = grp * {bpg} + bnk;
      if (!(b_open[b] && b_row[b] == row)) begin
        if (b_open[b]) begin
          pre_at = max2(close_from(b), arrive);
          place(pre_at, 3, grp, bnk, 0, p);
          b_open[b] = 0;
          b_ready_at[b] = p + TRP;
          b_last_use[b] = p;
        end
        act_at = max2(max2(b_ready_at[b], b_last_use[b] + 1), arrive);
        if (b_has_act[b]) act_at = max2(act_at, b_last_act[b] + TRC);
        if (g_has_act[grp]) act_at = max2(act_at, g_last_act[grp] + TRRD_L);
        if (has_act) act_at = max2(act_at, last_act + TRRD_S);
        if (faw_n == 4) act_at = max2(act_at, faw[0] + TFAW);
        place(act_at, 0, grp, bnk, row, a);
        if (faw_n == 4) begin
          faw[0] = faw[1]; faw[1] = faw[2]; faw[2] = faw[3]; faw[3] = a;
        end else begin
          faw[faw_n] = a;
          faw_n = faw_n + 1;
        end
        has_act = 1; last_act = a;
        g_has_act[grp] = 1; g_last_act[grp] = a;
        b_open[b] = 1; b_row[b] = row; b_act_at[b] = a; b_has_act[b] = 1; b_last_act[b] = a;
        b_has_rd[b] = 0; b_has_wr[b] = 0; b_last_use[b] = a;
      end
      latency = wr ? WL : RL;
      cas_at = max2(max2(b_act_at[b] + TRCD, frontier(0)), arrive);
      if (g_has_cas[grp]) cas_at = max2(cas_at, g_last_cas[grp] + TCCD_L);
      if (has_cas) cas_at = max2(cas_at, last_cas + TCCD_S);
      if (wr) begin
        if (has_rd) cas_at = max2(cas_at, last_rd + RTW);
      end else begin
        for (k = 0; k < NG; k = k + 1) if (g_has_wr[k]) cas_at = max2(cas_at, g_last_wr[k] + (k == grp ? WTR_L : WTR_S));
      end
      cas_at = max2(cas_at, data_free >= latency ? data_free - latency : 0);
      place(cas_at, wr ? 2 : 1, grp, bnk, col, c);
      data_free = c + latency + BURST;
      has_cas = 1; last_cas = c;
      g_has_cas[grp] = 1; g_last_cas[grp] = c;
      if (wr) begin
        g_has_wr[grp] = 1; g_last_wr[grp] = c;
        b_has_wr[b] = 1; b_last_wr[b] = c;
      end else begin
        has_rd = 1; last_rd = c;
        b_has_rd[b] = 1; b_last_rd[b] = c;
      end
      b_last_use[b] = c;
      if (rec_n == LOOK) begin
        recent[rec_h] = c;
        rec_h = (rec_h + 1) % LOOK;
      end else begin
        recent[(rec_h + rec_n) % LOOK] = c;
        rec_n = rec_n + 1;
      end
      release_floor;
      if (LOG) $fwrite(logf, "L %0d %0d %0d %0d\n", line, wr, arrive, c);
    end
  endtask

  // Reads taken and not yet returned, in order: the line's data and the controller clock it is due.
  reg [DW-1:0] q_data [0:QUEUE-1];
  reg [63:0]   q_due [0:QUEUE-1];
  integer qh = 0, qn = 0;
  reg [63:0] cycle = 0, cas;
  reg [AW-1:0] w_addr;
  reg [BW-1:0] w_left = 0;
  reg [AW-1:0] at_line;
  integer j;
  always @(posedge clk) begin
    cycle = cycle + 1;
    readdatavalid <= 1'b0;
    if (!rst_n) begin
      waitrequest <= 1'b1;
    end else begin
      if (read && !waitrequest) begin
        for (j = 0; j < burstcount; j = j + 1) begin
          at_line = address + j;
          line_at(at_line, 1'b0, cycle * RATE, cas);
          q_data[(qh + qn) % QUEUE] = mem[at_line[LINES_LOG2-1:0]];
          q_due[(qh + qn) % QUEUE] = (cas + RL + BURST + RATE - 1) / RATE + PIPE;
          qn = qn + 1;
        end
      end
      if (write && !waitrequest) begin
        at_line = w_left == 0 ? address : w_addr;
        for (j = 0; j < DW / 8; j = j + 1)
          if (byteenable[j]) mem[at_line[LINES_LOG2-1:0]][8*j +: 8] = writedata[8*j +: 8];
        line_at(at_line, 1'b1, cycle * RATE, cas);
        w_left = w_left == 0 ? burstcount - 1 : w_left - 1;
        w_addr = at_line + 1;
      end
      if (qn != 0 && q_due[qh] <= cycle) begin
        readdata <= q_data[qh];
        readdatavalid <= 1'b1;
        qh = (qh + 1) % QUEUE;
        qn = qn - 1;
      end
      waitrequest <= qn + (1 << (BW - 1)) > QUEUE;
    end
  end
endmodule
"#,
        part = g.part,
        rate = g.rate,
        rl = t.rl,
        wl = t.wl,
        trcd = t.trcd,
        trp = t.trp,
        tras = t.tras,
        trc = t.trc,
        trrd_s = t.trrd_s,
        trrd_l = t.trrd_l,
        tfaw = t.tfaw,
        tccd_s = t.tccd_s,
        tccd_l = t.tccd_l,
        trtp = t.trtp,
        wtp = t.write_to_precharge(),
        rtw = t.read_to_write(),
        wtr_s = t.write_to_read(false),
        wtr_l = t.write_to_read(true),
        trfc = t.trfc,
        trefi = t.trefi,
        burst = t.burst_ck(),
        banks = banks,
        groups = groups,
        lookahead = lookahead.max(1),
        gb1 = gbits - 1,
        gbits = gbits,
        c_hi = gbits + cbits - 1,
        b_lo = gbits + cbits,
        b_hi = gbits + cbits + bbits - 1,
        r_lo = gbits + cbits + bbits,
        r_hi = gbits + cbits + bbits + g.row_bits - 1,
        bpg = g.banks_per_group,
    )
}
