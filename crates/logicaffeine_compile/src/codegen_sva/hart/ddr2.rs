//! The hart's main memory: the Nexys A7's DDR2, a Micron MT47H64M16 (1 Gb, x16, eight banks of 8192 rows of
//! 1024 columns), run by `hart_ddr2.v` at CK = 125 MHz — the slowest clock the part's DLL is specified for, and twice
//! the hart's. One hart clock is a *frame*: two CK cycles, four data beats, one 64-bit word.
//!
//! The controller moves whole 64-byte lines, closed-page: ACTIVATE, four BL8 bursts two frames apart (the last with
//! auto-precharge), and the bank idle again before the next line. Every command is in a frame's first CK cycle.
//! This module is the one definition of the part's timing, the frame counts the controller schedules by, the mode
//! registers, the power-up sequence, the address map and the calibration pattern; `hart_ddr2.v` takes them through
//! [`verilog_params`], and the simulation's device model checks the controller against the part's own numbers.

use super::super::ddr2_phy::{ddr2_pins, Ddr2Cmd, InitStep, ModeReg};

/// One CK cycle, and one frame (a hart clock), in picoseconds.
pub const CK_PS: u64 = 8_000;
pub const FRAME_PS: u64 = 2 * CK_PS;

/// The part's timing, from the MT47H64M16 data sheet (-25E grade), in picoseconds unless named in CK cycles.
#[derive(Clone, Copy, Debug)]
pub struct Part {
    /// ACTIVATE to READ or WRITE.
    pub trcd: u64,
    /// PRECHARGE to ACTIVATE.
    pub trp: u64,
    /// ACTIVATE to PRECHARGE, least.
    pub tras: u64,
    /// ACTIVATE to ACTIVATE in one bank.
    pub trc: u64,
    /// ACTIVATE to ACTIVATE in different banks.
    pub trrd: u64,
    /// The last write beat to PRECHARGE.
    pub twr: u64,
    /// The last write beat to READ.
    pub twtr: u64,
    /// READ to PRECHARGE.
    pub trtp: u64,
    /// REFRESH to anything.
    pub trfc: u64,
    /// The longest one refresh may follow another.
    pub trefi: u64,
    /// Power-up with CKE low.
    pub power_up: u64,
    /// CKE high to the first command.
    pub cke_settle: u64,
    /// A mode register load to the next command, in CK.
    pub tmrd_ck: u64,
    /// DLL reset to the first READ, in CK.
    pub tdllk_ck: u64,
    /// CAS latency, in CK; write latency is one less.
    pub cl: u64,
}

pub const MT47H64M16: Part = Part {
    trcd: 12_500,
    trp: 12_500,
    tras: 45_000,
    trc: 57_500,
    trrd: 10_000,
    twr: 15_000,
    twtr: 7_500,
    trtp: 7_500,
    trfc: 127_500,
    trefi: 7_800_000,
    power_up: 200_000_000,
    cke_settle: 400_000,
    tmrd_ck: 2,
    tdllk_ck: 200,
    cl: 5,
};

/// Burst length, in beats (BL8): sixteen bytes, two frames.
pub const BURST_BEATS: u64 = 8;
pub const BURSTS_PER_LINE: u64 = 4;

/// Frames covering at least `ps`.
pub const fn frames(ps: u64) -> u64 {
    (ps + FRAME_PS - 1) / FRAME_PS
}

/// CK cycles covering at least `ps`.
pub const fn cks(ps: u64) -> u64 {
    (ps + CK_PS - 1) / CK_PS
}

/// The controller's schedule, in frames: what [`verilog_params`] hands `hart_ddr2.v`. A line transaction issues
/// ACTIVATE at frame 0 and its four column commands at `cas(k)`; the write data's DQS preamble is the frame after a
/// WRITE and its eight beats the two frames after that (WL = CL − 1 = four CK cycles); the bank is idle and every
/// constraint met by `read_done` / `write_done`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub rcd: u64,
    pub ccd: u64,
    pub read_done: u64,
    pub write_done: u64,
    pub rfc: u64,
    pub refi: u64,
    pub power_up: u64,
    pub cke_settle: u64,
    pub mrd: u64,
    pub dllk: u64,
}

impl Schedule {
    pub fn cas(&self, k: u64) -> u64 {
        self.rcd + k * self.ccd
    }
}

pub fn schedule(p: &Part) -> Schedule {
    let rcd = frames(p.trcd);
    let ccd = BURST_BEATS / 4;
    let last = rcd + (BURSTS_PER_LINE - 1) * ccd;
    // A READ with auto-precharge precharges BL/2 − 2 + max(tRTP, 2 CK) CK after it, not before tRAS from ACTIVATE;
    // the bank takes tRP more, and the next ACTIVATE also waits tRC.
    let rd_pre_ps = (last * FRAME_PS + (BURST_BEATS / 2 - 2) * CK_PS + p.trtp.max(2 * CK_PS)).max(p.tras);
    let read_done = frames(rd_pre_ps + p.trp).max(frames(p.trc));
    // A WRITE with auto-precharge precharges WL + BL/2 + WR CK after it (WR = tWR in whole CK cycles).
    let wl = p.cl - 1;
    let wr_pre_ps = (last * FRAME_PS + (wl + BURST_BEATS / 2 + cks(p.twr)) * CK_PS).max(p.tras);
    let write_done = frames(wr_pre_ps + p.trp).max(frames(p.trc));
    Schedule {
        rcd,
        ccd,
        read_done,
        write_done,
        rfc: frames(p.trfc),
        // Refresh a little before the deadline: a line transaction may be under way when it falls due.
        refi: p.trefi / FRAME_PS - 2 * write_done,
        power_up: frames(p.power_up),
        cke_settle: frames(p.cke_settle),
        mrd: (p.tmrd_ck + 1) / 2,
        dllk: (p.tdllk_ck + 1) / 2,
    }
}

/// The mode register: BL8, sequential, CAS latency, write recovery, and whether the DLL resets.
pub fn mr(p: &Part, dll_reset: bool) -> u16 {
    let bl8 = 0b011;
    let cl = (p.cl as u16 & 0b111) << 4;
    let wr = ((cks(p.twr) as u16 - 1) & 0b111) << 9;
    bl8 | cl | wr | if dll_reset { 1 << 8 } else { 0 }
}

/// Extended mode register 1: DLL on, full drive, no on-die termination, no additive latency, differential DQS; the
/// off-chip driver calibration as given (0b111 default, 0 exit).
pub fn emr1(ocd: u16) -> u16 {
    (ocd & 0b111) << 7
}

/// The JEDEC power-up sequence in frames.
pub fn init_sequence(p: &Part) -> Vec<InitStep> {
    use InitStep::*;
    let s = schedule(p);
    let rp = frames(p.trp);
    vec![
        CkeLowWait { cycles: s.power_up },
        Wait { cycles: s.cke_settle },
        PrechargeAll,
        Wait { cycles: rp },
        LoadMode { reg: ModeReg::Emr2, a: 0 },
        Wait { cycles: s.mrd },
        LoadMode { reg: ModeReg::Emr3, a: 0 },
        Wait { cycles: s.mrd },
        LoadMode { reg: ModeReg::Emr1, a: emr1(0) },
        Wait { cycles: s.mrd },
        LoadMode { reg: ModeReg::Mr, a: mr(p, true) },
        Wait { cycles: s.mrd },
        PrechargeAll,
        Wait { cycles: rp },
        Refresh,
        Wait { cycles: s.rfc },
        Refresh,
        Wait { cycles: s.rfc },
        LoadMode { reg: ModeReg::Mr, a: mr(p, false) },
        Wait { cycles: s.dllk },
        LoadMode { reg: ModeReg::Emr1, a: emr1(0b111) },
        Wait { cycles: s.mrd },
        LoadMode { reg: ModeReg::Emr1, a: emr1(0) },
        Wait { cycles: s.mrd },
    ]
}

/// An init step as the controller's ROM word: `{cke, cs_n, ras_n, cas_n, we_n, ba[2:0], a[12:0], frames[23:0]}`
/// (45 bits), the command's pins from the JEDEC truth table [`ddr2_pins`].
pub fn init_word(step: &InitStep) -> u64 {
    let (cke, cmd, ba, a, wait) = match *step {
        InitStep::CkeLowWait { cycles } => (0u64, Ddr2Cmd::Deselect, 0u64, 0u64, cycles),
        InitStep::Wait { cycles } => (1, Ddr2Cmd::Nop, 0, 0, cycles),
        InitStep::PrechargeAll => (1, Ddr2Cmd::Precharge, 0, 1 << 10, 0),
        InitStep::Refresh => (1, Ddr2Cmd::Refresh, 0, 0, 0),
        InitStep::LoadMode { reg, a } => (1, Ddr2Cmd::LoadMode, reg.ba() as u64, a as u64, 0),
    };
    assert!(wait < 1 << 24, "an init wait of {wait} frames does not fit the ROM word");
    let (cs, ras, cas, we) = ddr2_pins(cmd);
    (cke << 44)
        | ((cs as u64) << 43)
        | ((ras as u64) << 42)
        | ((cas as u64) << 41)
        | ((we as u64) << 40)
        | (ba << 37)
        | (a << 24)
        | wait
}

/// Where a byte of the RAM lives: (row, bank, column). The bank bits sit just above a row's 2 KB, so consecutive
/// rows of the address space rotate through the banks.
pub fn locate(offset: u64) -> (u64, u64, u64) {
    ((offset >> 14) & 0x1fff, (offset >> 11) & 0b111, (offset >> 1) & 0x3ff)
}

/// The line the controller writes and reads back to calibrate its read capture: thirty-two beats per byte lane, each
/// byte distinct within its lane and from every other's complement, so the first beat (or its complement) is found
/// at exactly one place in the captured stream and the rest confirm it.
pub fn calibration_line() -> [u64; 8] {
    let mut words = [0u64; 8];
    let mut x: u32 = 0x9e37_79b9;
    let mut used = [[false; 256]; 2];
    for beat in 0..32 {
        for lane in 0..2 {
            let byte = loop {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                let b = (x >> 11) as u8;
                if !used[lane][b as usize] && !used[lane][!b as usize] {
                    used[lane][b as usize] = true;
                    break b;
                }
            };
            words[beat / 4] |= (byte as u64) << (16 * (beat % 4) + 8 * lane);
        }
    }
    words
}

/// The parameters `hart_ddr2.v` includes (`hart_ddr2_params.vh`): the schedule in frames, the power-up ROM, the
/// calibration line.
pub fn verilog_params(p: &Part) -> String {
    let s = schedule(p);
    let seq = init_sequence(p);
    let mut v = String::from("// Generated from codegen_sva::hart::ddr2. Do not edit.\n`ifndef HART_DDR2_PARAMS_VH\n`define HART_DDR2_PARAMS_VH\n");
    for (name, value) in [
        ("RCD", s.rcd),
        ("CCD", s.ccd),
        ("READ_DONE", s.read_done),
        ("WRITE_DONE", s.write_done),
        ("RFC", s.rfc),
        ("REFI", s.refi),
        ("INIT_STEPS", seq.len() as u64),
    ] {
        v.push_str(&format!("`define DDR2_{name} {value}\n"));
    }
    v.push_str("`define DDR2_INIT_ROM(rom) \\\n");
    for (i, step) in seq.iter().enumerate() {
        v.push_str(&format!("  rom[{i}] = 45'h{:012x}; \\\n", init_word(step)));
    }
    v.push_str("  ;\n`define DDR2_CAL_LINE(line) \\\n");
    for (i, w) in calibration_line().iter().enumerate() {
        v.push_str(&format!("  line[{i}] = 64'h{w:016x}; \\\n"));
    }
    v.push_str("  ;\n`endif\n");
    v
}
