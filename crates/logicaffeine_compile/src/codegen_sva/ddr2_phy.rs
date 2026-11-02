//! ddr2_phy — a full custom hand-rolled DDR2 controller + PHY for the Nexys A7-100T's on-board
//! Micron MT47H64M16 (1 Gb, x16, 8 banks, DDR2-800). This is Phase 1 of the DRAM-into-Compute
//! campaign: total ownership in the openXC7 flow (no vendor MIG), tied directly to the
//! `dram_executor` command engine (`super::dram`), so the SAME command stream that drives the
//! JEDEC-legal schedule also drives the deliberately-illegal ComputeDRAM cadences.
//!
//! Built in gated, iverilog-verified sub-steps (each proven against a behavioral DDR2 model
//! before the board):
//!   • 1a — command layer (THIS brick first): op field → DDR2 pins, single source of truth;
//!          clocking + I/O primitives (MMCM/PLL, SSTL18, ODDR/IDDR, IDELAY);
//!   • 1b — init / mode-register sequence;
//!   • 1c — closed-loop per-byte-lane read calibration;
//!   • 1d — full controller (ODT, refresh, power-down, bank-interleave, status);
//!   • 1e — memtest over the full 128 MiB (the Phase-1 gate).
//!
//! The command layer maps the `dram_executor` op field (0=ACT 1=RD 2=WR 3=PRE 4=REFpb — see
//! `super::dram::delta_encode`) plus the init commands (NOP / DESELECT / LOAD-MODE) to the JEDEC
//! DDR2 command truth table on {CS#, RAS#, CAS#, WE#}. The Verilog `ddr2_cmd_encoder` is GENERATED
//! from `ddr2_pins`, so the netlist and the model can never drift.

use super::dram::{DramCmd, DramTimings, TimedCmd};

/// A DDR2 command (JEDEC truth table over {CS#, RAS#, CAS#, WE#}, with CKE held high).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ddr2Cmd {
    /// CS# high — the chip ignores the bus this cycle.
    Deselect,
    /// CS# low, RAS#/CAS#/WE# high — an idle active cycle.
    Nop,
    /// Open a row (RAS#).
    Active,
    /// Column read (CAS#).
    Read,
    /// Column write (CAS# + WE#).
    Write,
    /// Close a row (RAS# + WE#); A10 selects single-bank vs all-banks.
    Precharge,
    /// Auto-refresh (RAS# + CAS#), all banks precharged.
    Refresh,
    /// Mode/extended-mode register set (RAS# + CAS# + WE#); BA selects the register.
    LoadMode,
}

/// The `(cs_n, ras_n, cas_n, we_n)` pin levels for a DDR2 command — the JEDEC DDR2 command truth
/// table. This is the SINGLE SOURCE OF TRUTH the Verilog encoder is generated from.
pub fn ddr2_pins(cmd: Ddr2Cmd) -> (u8, u8, u8, u8) {
    match cmd {
        // CS# high: bus ignored (RAS#/CAS#/WE# held high, don't-care).
        Ddr2Cmd::Deselect => (1, 1, 1, 1),
        Ddr2Cmd::Nop => (0, 1, 1, 1),
        Ddr2Cmd::Active => (0, 0, 1, 1),
        Ddr2Cmd::Read => (0, 1, 0, 1),
        Ddr2Cmd::Write => (0, 1, 0, 0),
        Ddr2Cmd::Precharge => (0, 0, 1, 0),
        Ddr2Cmd::Refresh => (0, 0, 0, 1),
        Ddr2Cmd::LoadMode => (0, 0, 0, 0),
    }
}

/// Map a `dram_executor` op field (0=ACT 1=RD 2=WR 3=PRE 4=REFpb) to its DDR2 command.
/// Returns `None` for op codes outside the executor's issued set.
pub fn executor_op_to_cmd(op: u8) -> Option<Ddr2Cmd> {
    match op {
        0 => Some(Ddr2Cmd::Active),
        1 => Some(Ddr2Cmd::Read),
        2 => Some(Ddr2Cmd::Write),
        3 => Some(Ddr2Cmd::Precharge),
        4 => Some(Ddr2Cmd::Refresh),
        _ => None,
    }
}

/// Emit the combinational `ddr2_cmd_encoder` — op field + `issue` → DDR2 command pins. When
/// `issue` is low the bus holds NOP (CS# low, RAS#/CAS#/WE# high). The `case` arms are GENERATED
/// from [`ddr2_pins`], so the netlist is by-construction consistent with the model/oracle.
pub fn emit_ddr2_command_encoder_verilog() -> String {
    let (ncs, nras, ncas, nwe) = ddr2_pins(Ddr2Cmd::Nop);
    let mut arms = String::new();
    for op in 0u8..5 {
        let cmd = executor_op_to_cmd(op).expect("0..5 are executor ops");
        let (cs, ras, cas, we) = ddr2_pins(cmd);
        arms.push_str(&format!(
            "        3'd{op}: begin cs_n=1'b{cs}; ras_n=1'b{ras}; cas_n=1'b{cas}; we_n=1'b{we}; end // {cmd:?}\n"
        ));
    }
    format!(
        r#"// ddr2_cmd_encoder — op field (0=ACT 1=RD 2=WR 3=PRE 4=REFpb) → DDR2 command pins.
// GENERATED from codegen_sva::ddr2_phy::ddr2_pins (the JEDEC DDR2 command truth table); the
// netlist and the Rust model share one source of truth. `issue` low holds NOP.
module ddr2_cmd_encoder(
  input      [2:0] op,
  input            issue,
  output reg       cs_n,
  output reg       ras_n,
  output reg       cas_n,
  output reg       we_n
);
  always @(*) begin
    if (!issue) begin
      cs_n=1'b{ncs}; ras_n=1'b{nras}; cas_n=1'b{ncas}; we_n=1'b{nwe}; // NOP
    end else begin
      case (op)
{arms}        default: begin cs_n=1'b{ncs}; ras_n=1'b{nras}; cas_n=1'b{ncas}; we_n=1'b{nwe}; end // NOP
      endcase
    end
  end
endmodule
"#
    )
}

// ── Phase 1b: the DDR2 power-up / mode-register init sequence ─────────────────────────
//
// Before any read/write, DDR2 requires a fixed JEDEC power-up sequence (JESD79-2 / the Micron
// MT47H64M16 datasheet): hold CKE low ≥200 µs, PRECHARGE ALL, then program the mode registers in
// order — EMR2, EMR3, EMR1 (enable DLL), MR (with DLL reset) — PRECHARGE ALL, ≥2 AUTO-REFRESH, MR
// (DLL not reset, normal operation), wait for DLL lock, then EMR1 OCD default → OCD exit. The
// sequence is modeled as DATA (`ddr2_init_sequence`) with the JEDEC ordering/content pinned by
// invariant locks, and the `ddr2_init_fsm` ROM is GENERATED from it (single source of truth).

/// A DDR2 mode register, identified by its `BA[1:0]` selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeReg {
    /// Mode Register (BA = 00): burst length, CAS latency, burst type, DLL reset, write recovery.
    Mr,
    /// Extended Mode Register 1 (BA = 01): DLL enable, drive strength, ODT, AL, OCD.
    Emr1,
    /// Extended Mode Register 2 (BA = 10): high-temperature self-refresh, partial-array SR.
    Emr2,
    /// Extended Mode Register 3 (BA = 11): reserved.
    Emr3,
}

impl ModeReg {
    /// The `BA[1:0]` selector that addresses this register.
    pub fn ba(self) -> u8 {
        match self {
            ModeReg::Mr => 0,
            ModeReg::Emr1 => 1,
            ModeReg::Emr2 => 2,
            ModeReg::Emr3 => 3,
        }
    }
}

/// One step of the DDR2 power-up init sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitStep {
    /// Hold CKE low (DESELECT) for `cycles` — the ≥200 µs power-up wait — then raise CKE.
    CkeLowWait { cycles: u64 },
    /// A NOP idle wait of `cycles` (satisfies tRP / tMRD / tRFC / tDLLK between commands).
    Wait { cycles: u64 },
    /// PRECHARGE ALL banks (A10 = 1).
    PrechargeAll,
    /// AUTO-REFRESH.
    Refresh,
    /// Load a mode register with the 13-bit address payload (opcode).
    LoadMode { reg: ModeReg, a: u16 },
}

/// EMR1 with the DLL ENABLED (A0 = 0), full drive, ODT off, AL = 0, OCD exit — the first EMR1.
pub const EMR1_DLL_ENABLE: u16 = 0x0000;
/// EMR1 OCD calibration DEFAULT (A9:A7 = 111).
pub const EMR1_OCD_DEFAULT: u16 = 0b111 << 7;
/// EMR1 OCD calibration EXIT (A9:A7 = 000), DLL still enabled.
pub const EMR1_OCD_EXIT: u16 = 0x0000;

/// Encode the Mode Register (BA = 00) opcode from the timing table: burst length (A2:A0),
/// sequential burst type (A3 = 0), CAS latency (A6:A4, == tRCD on the 5-5-5 grade), DLL reset
/// (A8), and write recovery WR-1 (A11:A9).
pub fn mr_opcode(t: &DramTimings, dll_reset: bool) -> u16 {
    let bl: u16 = if t.burst_bytes / 2 == 4 { 0b010 } else { 0b011 }; // x16: BL8 default
    let cl: u16 = ((t.trcd as u16) & 0b111) << 4; // CAS latency A6:A4 (CL == tRCD, 5-5-5 grade)
    let wr: u16 = ((t.twr as u16).saturating_sub(1) & 0b111) << 9; // write recovery WR-1 in A11:A9
    let dll: u16 = if dll_reset { 1 << 8 } else { 0 };
    bl | cl | wr | dll
}

/// The JEDEC DDR2 power-up init sequence for this part, as data. `cke_low_cycles` is the ≥200 µs
/// power-up hold expressed in controller cycles (a deployment parameter of the chosen clock).
pub fn ddr2_init_sequence(t: &DramTimings, cke_low_cycles: u64) -> Vec<InitStep> {
    use InitStep::*;
    use ModeReg::*;
    let tmrd = 2; // MRS → next command
    let tdllk = 200; // DLL lock after the normal-operation MR
    vec![
        CkeLowWait {
            cycles: cke_low_cycles,
        }, // ≥200 µs, CKE low, then raise CKE
        Wait { cycles: 400 }, // settle after CKE high
        PrechargeAll,
        Wait { cycles: t.trp },
        LoadMode { reg: Emr2, a: 0 },
        Wait { cycles: tmrd },
        LoadMode { reg: Emr3, a: 0 },
        Wait { cycles: tmrd },
        LoadMode {
            reg: Emr1,
            a: EMR1_DLL_ENABLE,
        }, // enable DLL
        Wait { cycles: tmrd },
        LoadMode {
            reg: Mr,
            a: mr_opcode(t, true),
        }, // program MR + reset DLL
        Wait { cycles: tmrd },
        PrechargeAll,
        Wait { cycles: t.trp },
        Refresh,
        Wait { cycles: t.trfc_pb },
        Refresh,
        Wait { cycles: t.trfc_pb },
        LoadMode {
            reg: Mr,
            a: mr_opcode(t, false),
        }, // MR, DLL not reset — normal operation
        Wait { cycles: tdllk }, // wait for DLL lock
        LoadMode {
            reg: Emr1,
            a: EMR1_OCD_DEFAULT,
        }, // OCD calibration default
        Wait { cycles: tmrd },
        LoadMode {
            reg: Emr1,
            a: EMR1_OCD_EXIT,
        }, // OCD calibration exit — device ready
        Wait { cycles: tmrd },
    ]
}

/// The pin/field tuple `(cke, cmd, ba, a, wait)` an init step drives on the DDR2 bus.
fn init_step_fields(step: &InitStep) -> (u8, Ddr2Cmd, u8, u16, u64) {
    match *step {
        InitStep::CkeLowWait { cycles } => (0, Ddr2Cmd::Deselect, 0, 0, cycles),
        InitStep::Wait { cycles } => (1, Ddr2Cmd::Nop, 0, 0, cycles),
        InitStep::PrechargeAll => (1, Ddr2Cmd::Precharge, 0, 1 << 10, 0), // A10 = 1 (all banks)
        InitStep::Refresh => (1, Ddr2Cmd::Refresh, 0, 0, 0),
        InitStep::LoadMode { reg, a } => (1, Ddr2Cmd::LoadMode, reg.ba(), a, 0),
    }
}

/// Pack an init step into the 44-bit `ddr2_init_fsm` ROM word
/// `{cke, cs_n, ras_n, cas_n, we_n, ba[1:0], a[12:0], wait[23:0]}` — the SINGLE encoding shared by
/// the emitter and the drift lock, with the command pins taken from [`ddr2_pins`].
pub fn init_rom_word(step: &InitStep) -> u64 {
    let (cke, cmd, ba, a, wait) = init_step_fields(step);
    let (cs, ras, cas, we) = ddr2_pins(cmd);
    ((cke as u64) << 43)
        | ((cs as u64) << 42)
        | ((ras as u64) << 41)
        | ((cas as u64) << 40)
        | ((we as u64) << 39)
        | ((ba as u64) << 37)
        | ((a as u64) << 24)
        | (wait & 0xff_ffff)
}

/// Emit `ddr2_init_fsm` — a ROM-driven power-up sequencer. The ROM is GENERATED from
/// [`ddr2_init_sequence`]; the FSM drives `{cke, cs_n, ras_n, cas_n, we_n, ba, a}` per entry, waits
/// its cycle count, advances, and latches `init_done`. (Behavioral iverilog validation lands with
/// the controller-integration brick; this brick pins the ROM to the JEDEC spec.)
pub fn emit_ddr2_init_fsm_verilog(t: &DramTimings, cke_low_cycles: u64) -> String {
    let seq = ddr2_init_sequence(t, cke_low_cycles);
    let n = seq.len().max(1);
    let mut rom = String::new();
    for (i, step) in seq.iter().enumerate() {
        let word = init_rom_word(step);
        let (_, cmd, ..) = init_step_fields(step);
        rom.push_str(&format!("    rom[{i}] = 44'h{word:011x}; // {cmd:?}\n"));
    }
    format!(
        r#"// ddr2_init_fsm — DDR2 power-up / mode-register sequencer. The step ROM is GENERATED from
// codegen_sva::ddr2_phy::ddr2_init_sequence (the JEDEC init spec); each entry drives
// {{cke, cs_n, ras_n, cas_n, we_n, ba, a}} for `wait` cycles, then advances. init_done latches high.
module ddr2_init_fsm(
  input             clk,
  input             rst,
  output reg        cke,
  output reg        cs_n,
  output reg        ras_n,
  output reg        cas_n,
  output reg        we_n,
  output reg [1:0]  ba,
  output reg [12:0] a,
  output reg        init_done
);
  localparam NSTEPS = {n};
  reg [43:0] rom [0:NSTEPS-1];
  initial begin
{rom}  end
  integer step;
  reg [43:0] cur;
  reg [23:0] wait_left;
  always @(posedge clk) begin
    if (rst) begin
      step <= 0; cur <= rom[0]; wait_left <= rom[0][23:0]; init_done <= 1'b0;
    end else if (!init_done) begin
      if (wait_left == 24'd0) begin
        if (step == NSTEPS-1) init_done <= 1'b1;
        else begin step <= step + 1; cur <= rom[step+1]; wait_left <= rom[step+1][23:0]; end
      end else begin
        wait_left <= wait_left - 24'd1;
      end
    end
  end
  always @(*) begin
    if (init_done) begin
      cke=1'b1; cs_n=1'b0; ras_n=1'b1; cas_n=1'b1; we_n=1'b1; ba=2'b0; a=13'b0; // NOP idle
    end else begin
      cke=cur[43]; cs_n=cur[42]; ras_n=cur[41]; cas_n=cur[40]; we_n=cur[39];
      ba=cur[38:37]; a=cur[36:24];
    end
  end
endmodule
"#
    )
}

/// A self-checking testbench for `ddr2_cmd_encoder`: drives every op with issue = 0 and issue = 1 and
/// asserts the pins equal [`ddr2_pins`] (expectations GENERATED from the same truth table). PASS/FAIL.
pub fn emit_ddr2_cmd_encoder_testbench() -> String {
    let (ncs, nras, ncas, nwe) = ddr2_pins(Ddr2Cmd::Nop);
    let mut checks = String::new();
    for op in 0u8..8 {
        checks.push_str(&format!(
            "    chk(3'd{op}, 1'b0, 1'b{ncs}, 1'b{nras}, 1'b{ncas}, 1'b{nwe});\n"
        ));
    }
    for op in 0u8..8 {
        let (cs, ras, cas, we) = match executor_op_to_cmd(op) {
            Some(cmd) => ddr2_pins(cmd),
            None => (ncs, nras, ncas, nwe), // out-of-range ⇒ default arm holds NOP
        };
        checks.push_str(&format!(
            "    chk(3'd{op}, 1'b1, 1'b{cs}, 1'b{ras}, 1'b{cas}, 1'b{we});\n"
        ));
    }
    format!(
        r#"`timescale 1ns/1ps
module tb_ddr2_cmd_encoder;
  reg [2:0] op; reg issue;
  wire cs_n, ras_n, cas_n, we_n;
  integer errors = 0;
  ddr2_cmd_encoder dut(.op(op), .issue(issue), .cs_n(cs_n), .ras_n(ras_n), .cas_n(cas_n), .we_n(we_n));
  task chk(input [2:0] o, input i, input ecs, input eras, input ecas, input ewe);
    begin
      op=o; issue=i; #1;
      if (cs_n!==ecs || ras_n!==eras || cas_n!==ecas || we_n!==ewe) begin
        errors=errors+1;
        $display("FAIL op=%0d issue=%0d got=%b%b%b%b exp=%b%b%b%b", o, i, cs_n, ras_n, cas_n, we_n, ecs, eras, ecas, ewe);
      end
    end
  endtask
  initial begin
{checks}    if (errors==0) $display("PASS: ddr2_cmd_encoder matches the JEDEC truth table for all ops");
    $finish;
  end
endmodule
"#
    )
}

/// A behavioral testbench for `ddr2_init_fsm`: a bus monitor records each isolated real command
/// (NOP/DESELECT skipped) the FSM issues during power-up, and asserts the recorded log equals the
/// JEDEC command sequence (generated from [`ddr2_init_sequence`]) and that `init_done` asserts.
pub fn emit_ddr2_init_testbench(t: &DramTimings, cke_low_cycles: u64) -> String {
    let seq = ddr2_init_sequence(t, cke_low_cycles);
    let mut exp: Vec<(u8, u8)> = Vec::new();
    for s in &seq {
        match s {
            InitStep::PrechargeAll => exp.push((4, 0)),
            InitStep::Refresh => exp.push((5, 0)),
            InitStep::LoadMode { reg, .. } => exp.push((6, reg.ba())),
            _ => {}
        }
    }
    let expn = exp.len();
    let mut expinit = String::new();
    for (i, (c, b)) in exp.iter().enumerate() {
        expinit.push_str(&format!("    exp_code[{i}]=4'd{c}; exp_ba[{i}]=2'd{b};\n"));
    }
    format!(
        r#"`timescale 1ns/1ps
module tb_ddr2_init_fsm;
  reg clk=0; always #5 clk=~clk;
  reg rst=1;
  wire cke, cs_n, ras_n, cas_n, we_n; wire [1:0] ba; wire [12:0] a; wire init_done;
  ddr2_init_fsm dut(.clk(clk), .rst(rst), .cke(cke), .cs_n(cs_n), .ras_n(ras_n),
                    .cas_n(cas_n), .we_n(we_n), .ba(ba), .a(a), .init_done(init_done));
  localparam EXP_N = {expn};
  reg [3:0] exp_code [0:EXP_N-1];
  reg [1:0] exp_ba [0:EXP_N-1];
  reg [3:0] got_code [0:255];
  reg [1:0] got_ba [0:255];
  integer ngot; reg was_real; reg [3:0] cc; integer i; integer errors;
  // Record each isolated real command (real commands are separated by NOPs in the sequence).
  always @(posedge clk) begin
    if (rst) begin ngot=0; was_real=0; end
    else if (!init_done) begin
      if (cs_n===1'b0 && {{ras_n,cas_n,we_n}} !== 3'b111) begin
        case ({{ras_n,cas_n,we_n}})
          3'b011: cc=4'd1; 3'b101: cc=4'd2; 3'b100: cc=4'd3;
          3'b010: cc=4'd4; 3'b001: cc=4'd5; 3'b000: cc=4'd6; default: cc=4'd0;
        endcase
        if (!was_real) begin got_code[ngot]=cc; got_ba[ngot]=ba; ngot=ngot+1; end
        was_real=1;
      end else was_real=0;
    end
  end
  initial begin
{expinit}    errors=0;
    repeat (3) @(posedge clk); rst=0;
    i=0;
    while (i<20000 && !init_done) begin @(posedge clk); i=i+1; end
    if (!init_done) begin errors=errors+1; $display("FAIL: init_done never asserted"); end
    if (ngot !== EXP_N) begin errors=errors+1; $display("FAIL: recorded %0d cmds, expected %0d", ngot, EXP_N); end
    for (i=0; i<EXP_N; i=i+1) begin
      if (i<ngot) begin
        if (got_code[i]!==exp_code[i] || (exp_code[i]==4'd6 && got_ba[i]!==exp_ba[i])) begin
          errors=errors+1;
          $display("FAIL: cmd %0d got code=%0d ba=%0d exp code=%0d ba=%0d", i, got_code[i], got_ba[i], exp_code[i], exp_ba[i]);
        end
      end
    end
    if (errors==0) $display("PASS: ddr2_init_fsm walks the JEDEC init sequence (%0d cmds) and asserts init_done", ngot);
    $finish;
  end
endmodule
"#
    )
}

// ── Phase 1d: read/write transaction command sequencing ───────────────────────────────
//
// The controller turns each read/write request into a single-transaction, closed-page command
// sequence — ACT(row) → RD/WR → PRE — honoring tRCD (ACT→CAS), tRAS (ACT→PRE), the burst duration,
// and (writes) tWR (write recovery before PRE). This is the LEGAL counterpart to the ComputeDRAM
// streams: the SAME ACT/CAS/PRE vocabulary, proven `super::dram::check_legal`-LEGAL here and
// `check_legal`-ILLEGAL there. One command bus, two planes — normal memory vs in-DRAM compute.

/// Emit the controller's per-request command sequence for one closed-page DDR2 access.
pub fn emit_transaction(t: &DramTimings, bank: u32, row: u64, write: bool) -> Vec<TimedCmd> {
    let cas = t.trcd;
    let cas_end = cas + t.data_cycles;
    let cas_cmd = if write {
        DramCmd::Wr { bank }
    } else {
        DramCmd::Rd { bank }
    };
    let mut pre = t.tras.max(cas_end); // tRAS from ACT, and after the burst completes
    if write {
        pre = pre.max(cas_end + t.twr); // write recovery before the page closes
    }
    vec![
        TimedCmd {
            at: 0,
            cmd: DramCmd::Act { bank, row },
        },
        TimedCmd {
            at: cas,
            cmd: cas_cmd,
        },
        TimedCmd {
            at: pre,
            cmd: DramCmd::Pre { bank },
        },
    ]
}

// ── Phase 1d: the controller FSM + behavioral DDR2 model ───────────────────────────────
//
// The controller turns a read/write request into an ACT → RD/WR → PRE transaction with a
// counter-driven schedule that mirrors `emit_transaction` (CAS at tRCD, PRE at max(tRAS, burst,
// +tWR for writes), tRP before the next ACT), plus periodic all-bank auto-refresh. It is proven
// under iverilog against `ddr2_bank_model` — a behavioral DDR2 array that stores/returns data AND
// enforces tRCD/tRAS/tRP/tWR (a buggy controller trips `model_error`, so the round-trip proof can
// never pass on illegal timing). The analog DQ/DQS SERDES + SSTL18 I/O + IDELAY calibration are the
// board-gated layer BELOW this; here the request/response and DDR command bus are digital.

fn ddr2_bake(tpl: &str, t: &DramTimings) -> String {
    tpl.replace("%TRCD%", &t.trcd.to_string())
        .replace("%TRAS%", &t.tras.to_string())
        .replace("%TRP%", &t.trp.to_string())
        .replace("%TWR%", &t.twr.to_string())
        .replace("%CL%", &t.trcd.to_string())
        .replace("%BURST%", &t.data_cycles.to_string())
        .replace("%TREFI%", &t.trefi_pb.to_string())
        .replace("%TRFC%", &t.trfc_pb.to_string())
}

/// Emit `ddr2_ctrl` — the request-driven DDR2 controller FSM (timings baked from the table).
pub fn emit_ddr2_controller_verilog(t: &DramTimings) -> String {
    ddr2_bake(
        r#"module ddr2_ctrl(
  input clk, input rst,
  input req_valid, input req_we, input [25:0] req_addr, input [15:0] req_wdata,
  output req_ready,
  output reg resp_valid, output reg [15:0] resp_rdata,
  output reg cke,
  output cs_n, output ras_n, output cas_n, output we_n,
  output [2:0] ba, output [12:0] a, output odt,
  output [15:0] wr_data, output wr_en,
  input [15:0] rd_data, input rd_valid
);
  localparam TRCD=%TRCD%, TRAS=%TRAS%, TRP=%TRP%, TWR=%TWR%, BURST=%BURST%;
  localparam PRE_RD = (TRAS > (TRCD+BURST)) ? TRAS : (TRCD+BURST);
  localparam PRE_WR = (TRAS > (TRCD+BURST+TWR)) ? TRAS : (TRCD+BURST+TWR);
  localparam integer TREFI=%TREFI%;
  localparam TRFC=%TRFC%;
  localparam S_IDLE=2'd0, S_BUSY=2'd1, S_PREW=2'd2, S_REF=2'd3;

  reg [1:0] state;
  reg cur_we; reg [2:0] cur_bank; reg [12:0] cur_row; reg [9:0] cur_col; reg [15:0] cur_wd;
  reg [15:0] t; reg [31:0] refi; reg refresh_pending; reg [15:0] reft;
  wire [15:0] pre_t = cur_we ? PRE_WR : PRE_RD;

  reg r_cs, r_ras, r_cas, r_we, r_wren, r_odt;
  reg [2:0] r_ba; reg [12:0] r_a;
  assign cs_n=r_cs; assign ras_n=r_ras; assign cas_n=r_cas; assign we_n=r_we;
  assign ba=r_ba; assign a=r_a; assign odt=r_odt; assign wr_en=r_wren; assign wr_data=cur_wd;
  assign req_ready = (state==S_IDLE) && !refresh_pending;

  // Commands are a pure function of the REGISTERED state + counter (no req_valid dependence),
  // so a missed accept can never leave a stray ACT on the bus.
  always @(*) begin
    r_cs=1'b0; r_ras=1'b1; r_cas=1'b1; r_we=1'b1; r_wren=1'b0; r_odt=1'b0; r_ba=cur_bank; r_a=13'b0;
    case (state)
      S_BUSY: begin
        if (t==0) begin r_ras=1'b0; r_cas=1'b1; r_we=1'b1; r_a=cur_row; end // ACT
        else if (t==TRCD) begin
          r_ras=1'b1; r_cas=1'b0; r_we=cur_we?1'b0:1'b1; r_a={3'b000, cur_col};
          if (cur_we) begin r_wren=1'b1; r_odt=1'b1; end
        end else if (t==pre_t) begin
          r_ras=1'b0; r_cas=1'b1; r_we=1'b0; // PRE
        end
      end
      S_REF: if (reft==0) begin r_ras=1'b0; r_cas=1'b0; r_we=1'b1; end // REF (banks idle)
      default: ;
    endcase
  end

  always @(posedge clk) begin
    if (rst) begin
      state<=S_IDLE; t<=0; refi<=0; refresh_pending<=1'b0; reft<=0; cke<=1'b1;
      resp_valid<=1'b0; resp_rdata<=0; cur_we<=0; cur_bank<=0; cur_row<=0; cur_col<=0; cur_wd<=0;
    end else begin
      cke<=1'b1; resp_valid<=1'b0;
      if (refi >= TREFI-1) begin refresh_pending<=1'b1; refi<=0; end else refi<=refi+1;
      case (state)
        S_IDLE: begin
          if (refresh_pending) begin reft<=0; refresh_pending<=1'b0; state<=S_REF; end
          else if (req_valid) begin
            cur_we<=req_we; cur_bank<=req_addr[12:10]; cur_row<=req_addr[25:13];
            cur_col<=req_addr[9:0]; cur_wd<=req_wdata; t<=0; state<=S_BUSY;
          end
        end
        S_BUSY: begin
          if (!cur_we && rd_valid) begin resp_valid<=1'b1; resp_rdata<=rd_data; end
          if (t==pre_t) state<=S_PREW;
          t<=t+1;
        end
        S_PREW: begin
          if (t >= pre_t + TRP) begin state<=S_IDLE; t<=0; end else t<=t+1;
        end
        S_REF: begin
          if (reft >= TRFC-1) state<=S_IDLE; else reft<=reft+1;
        end
      endcase
    end
  end
endmodule
"#,
        t,
    )
}

/// Emit `ddr2_bank_model` — a behavioral DDR2 array that stores/returns data (CL read latency) AND
/// enforces tRCD/tRAS/tRP/tWR, raising `model_error` on any violation (the round-trip checker).
pub fn emit_ddr2_bank_model_verilog(t: &DramTimings) -> String {
    ddr2_bake(
        r#"module ddr2_bank_model(
  input clk, input rst,
  input cke, input cs_n, input ras_n, input cas_n, input we_n,
  input [2:0] ba, input [12:0] a, input odt,
  input [15:0] wr_data, input wr_en,
  output reg [15:0] rd_data, output reg rd_valid,
  output reg model_error
);
  localparam TRCD=%TRCD%, TRAS=%TRAS%, TRP=%TRP%, TWR=%TWR%, CL=%CL%, BURST=%BURST%;
  reg openb [0:7];
  reg [12:0] open_row [0:7];
  reg [31:0] last_act [0:7];
  reg [31:0] last_pre [0:7];
  reg had_pre [0:7];
  reg [31:0] wr_end [0:7];
  reg had_wr [0:7];
  reg [31:0] mc;
  reg rd_pending; reg [15:0] rd_cnt; reg [15:0] rd_val;
  reg [15:0] mem [0:2047];
  integer bk;

  always @(posedge clk) begin
    if (rst) begin
      mc<=0; model_error<=1'b0; rd_valid<=1'b0; rd_data<=0; rd_pending<=1'b0; rd_cnt<=0; rd_val<=0;
      for (bk=0;bk<8;bk=bk+1) begin
        openb[bk]<=1'b0; had_pre[bk]<=1'b0; had_wr[bk]<=1'b0;
        last_act[bk]<=0; last_pre[bk]<=0; wr_end[bk]<=0; open_row[bk]<=0;
      end
    end else begin
      mc<=mc+1;
      rd_valid<=1'b0;
      if (rd_pending) begin
        if (rd_cnt<=1) begin rd_valid<=1'b1; rd_data<=rd_val; rd_pending<=1'b0; end
        else rd_cnt<=rd_cnt-1;
      end
      if (cke && !cs_n) begin
        case ({ras_n,cas_n,we_n})
          3'b011: begin // ACT
            if (openb[ba]) begin model_error<=1'b1; $display("FAIL: ACT on open bank %0d @%0d", ba, mc); end
            if (had_pre[ba] && (mc - last_pre[ba] < TRP)) begin model_error<=1'b1; $display("FAIL: tRP bank %0d @%0d", ba, mc); end
            openb[ba]<=1'b1; open_row[ba]<=a; last_act[ba]<=mc;
          end
          3'b101: begin // READ
            if (!openb[ba]) begin model_error<=1'b1; $display("FAIL: READ closed bank %0d @%0d", ba, mc); end
            else if (mc - last_act[ba] < TRCD) begin model_error<=1'b1; $display("FAIL: tRCD read bank %0d @%0d", ba, mc); end
            rd_pending<=1'b1; rd_cnt<=CL; rd_val<=mem[{ba, open_row[ba][3:0], a[3:0]}];
          end
          3'b100: begin // WRITE
            if (!openb[ba]) begin model_error<=1'b1; $display("FAIL: WRITE closed bank %0d @%0d", ba, mc); end
            else if (mc - last_act[ba] < TRCD) begin model_error<=1'b1; $display("FAIL: tRCD write bank %0d @%0d", ba, mc); end
            mem[{ba, open_row[ba][3:0], a[3:0]}] <= wr_data;
            wr_end[ba]<=mc+BURST; had_wr[ba]<=1'b1;
          end
          3'b010: begin // PRECHARGE
            if (!openb[ba]) begin model_error<=1'b1; $display("FAIL: PRE closed bank %0d @%0d", ba, mc); end
            else if (mc - last_act[ba] < TRAS) begin model_error<=1'b1; $display("FAIL: tRAS bank %0d @%0d", ba, mc); end
            if (had_wr[ba] && (mc < wr_end[ba] + TWR)) begin model_error<=1'b1; $display("FAIL: tWR bank %0d @%0d", ba, mc); end
            openb[ba]<=1'b0; last_pre[ba]<=mc; had_pre[ba]<=1'b1; had_wr[ba]<=1'b0;
          end
          3'b001: begin // REFRESH
            for (bk=0;bk<8;bk=bk+1) if (openb[bk]) begin model_error<=1'b1; $display("FAIL: REF open bank %0d @%0d", bk, mc); end
          end
          default: ;
        endcase
      end
    end
  end
endmodule
"#,
        t,
    )
}

/// Emit `tb_ddr2_ctrl` — writes `n_words` (distinct bank/col addresses), reads them back, and asserts
/// each read equals the written data with no `model_error`. The access pattern is generated here.
pub fn emit_ddr2_controller_testbench(t: &DramTimings, n_words: u32) -> String {
    let mut init = String::new();
    for i in 0..n_words {
        let addr = (i << 10) | i; // bank = i, col = i, row = 0 → distinct model index
        let data = 0xA000u32 + i;
        init.push_str(&format!(
            "    addr[{i}]=26'h{addr:x}; data[{i}]=16'h{data:x};\n"
        ));
    }
    let tpl = r#"`timescale 1ns/1ps
module tb_ddr2_ctrl;
  reg clk=0; always #5 clk=~clk;
  reg rst=1;
  reg req_valid; reg req_we; reg [25:0] req_addr; reg [15:0] req_wdata;
  wire req_ready; wire resp_valid; wire [15:0] resp_rdata;
  wire cke,cs_n,ras_n,cas_n,we_n,odt,wr_en,rd_valid;
  wire [2:0] ba; wire [12:0] a; wire [15:0] wr_data; wire [15:0] rd_data; wire model_error;
  ddr2_ctrl ctrl(.clk(clk),.rst(rst),.req_valid(req_valid),.req_we(req_we),.req_addr(req_addr),
    .req_wdata(req_wdata),.req_ready(req_ready),.resp_valid(resp_valid),.resp_rdata(resp_rdata),
    .cke(cke),.cs_n(cs_n),.ras_n(ras_n),.cas_n(cas_n),.we_n(we_n),.ba(ba),.a(a),.odt(odt),
    .wr_data(wr_data),.wr_en(wr_en),.rd_data(rd_data),.rd_valid(rd_valid));
  ddr2_bank_model memmodel(.clk(clk),.rst(rst),.cke(cke),.cs_n(cs_n),.ras_n(ras_n),.cas_n(cas_n),
    .we_n(we_n),.ba(ba),.a(a),.odt(odt),.wr_data(wr_data),.wr_en(wr_en),.rd_data(rd_data),
    .rd_valid(rd_valid),.model_error(model_error));
  localparam N=%N%;
  reg [25:0] addr [0:N-1];
  reg [15:0] data [0:N-1];
  reg [15:0] rdout;
  integer i; integer errors; integer timeout;
  task do_write(input [25:0] ad, input [15:0] d); begin
    @(posedge clk);
    req_valid <= 1'b1; req_we <= 1'b1; req_addr <= ad; req_wdata <= d;
    @(posedge clk);
    while (!req_ready) @(posedge clk);
    req_valid <= 1'b0;
  end endtask
  task do_read(input [25:0] ad); begin
    @(posedge clk);
    req_valid <= 1'b1; req_we <= 1'b0; req_addr <= ad; req_wdata <= 0;
    @(posedge clk);
    while (!req_ready) @(posedge clk);
    req_valid <= 1'b0;
    timeout=0;
    while (!resp_valid && timeout<500) begin @(posedge clk); timeout=timeout+1; end
    rdout=resp_rdata;
  end endtask
  initial begin
%INIT%    req_valid=0; req_we=0; req_addr=0; req_wdata=0; errors=0;
    repeat (4) @(posedge clk); rst=0;
    for (i=0;i<N;i=i+1) do_write(addr[i], data[i]);
    for (i=0;i<N;i=i+1) begin
      do_read(addr[i]);
      if (rdout !== data[i]) begin errors=errors+1; $display("FAIL: word %0d addr %h got %h exp %h", i, addr[i], rdout, data[i]); end
    end
    if (model_error) begin errors=errors+1; $display("FAIL: behavioral model flagged a timing violation"); end
    if (errors==0) $display("PASS: ddr2_ctrl round-tripped %0d words through the behavioral DDR2 model, no timing violations", N);
    $finish;
  end
endmodule
"#;
    ddr2_bake(tpl, t)
        .replace("%N%", &n_words.to_string())
        .replace("%INIT%", &init)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/ddr2_phy/tests.rs"]
mod tests;
