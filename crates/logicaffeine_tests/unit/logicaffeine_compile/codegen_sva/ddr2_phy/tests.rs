use super::super::dram::{
    check_legal, emit_computedram_stream, ComputeDramOp, ComputeDramTiming, DramCmd, DramTimings,
};
use super::*;

/// The JEDEC DDR2 command truth table, asserted by its DEFINING structural properties (not a
/// tautological echo of the impl): each command's RAS#/CAS#/WE# pattern, and the pit-of-success
/// invariant that NO two issued commands alias to the same pin pattern.
#[test]
fn ddr2_command_truth_table_matches_jedec() {
    use Ddr2Cmd::*;
    // Deselect is the only command with CS# high; every issued command drives CS# low.
    assert_eq!(ddr2_pins(Deselect).0, 1, "Deselect raises CS#");
    for c in [Nop, Active, Read, Write, Precharge, Refresh, LoadMode] {
        assert_eq!(ddr2_pins(c).0, 0, "issued command {c:?} drives CS# low");
    }
    // The RAS#/CAS#/WE# triples that DEFINE each command.
    assert_eq!(ddr2_pins(Nop), (0, 1, 1, 1), "NOP");
    assert_eq!(ddr2_pins(Active), (0, 0, 1, 1), "ACTIVE = RAS#");
    assert_eq!(ddr2_pins(Read), (0, 1, 0, 1), "READ = CAS#");
    assert_eq!(ddr2_pins(Write), (0, 1, 0, 0), "WRITE = CAS# + WE#");
    assert_eq!(ddr2_pins(Precharge), (0, 0, 1, 0), "PRECHARGE = RAS# + WE#");
    assert_eq!(
        ddr2_pins(Refresh),
        (0, 0, 0, 1),
        "AUTO-REFRESH = RAS# + CAS#"
    );
    assert_eq!(
        ddr2_pins(LoadMode),
        (0, 0, 0, 0),
        "LOAD-MODE = RAS# + CAS# + WE#"
    );
    // READ and WRITE differ ONLY in WE# (the JEDEC relationship).
    let (r, w) = (ddr2_pins(Read), ddr2_pins(Write));
    assert_eq!(
        (r.0, r.1, r.2),
        (w.0, w.1, w.2),
        "READ/WRITE share CS#/RAS#/CAS#"
    );
    assert_ne!(r.3, w.3, "READ/WRITE differ in WE#");
}

/// PIT OF SUCCESS: every distinct issued command has a distinct pin pattern — no command can
/// silently alias another (a decode collision would be a catastrophic, subtle DRAM bug).
#[test]
fn no_two_ddr2_commands_alias_to_the_same_pins() {
    use Ddr2Cmd::*;
    let issued = [Nop, Active, Read, Write, Precharge, Refresh, LoadMode];
    for (i, a) in issued.iter().enumerate() {
        for b in &issued[i + 1..] {
            assert_ne!(
                ddr2_pins(*a),
                ddr2_pins(*b),
                "{a:?} and {b:?} must not alias"
            );
        }
    }
}

/// The executor op field maps to exactly the right DDR2 commands (and rejects out-of-range).
#[test]
fn executor_ops_map_to_the_right_ddr2_commands() {
    assert_eq!(executor_op_to_cmd(0), Some(Ddr2Cmd::Active));
    assert_eq!(executor_op_to_cmd(1), Some(Ddr2Cmd::Read));
    assert_eq!(executor_op_to_cmd(2), Some(Ddr2Cmd::Write));
    assert_eq!(executor_op_to_cmd(3), Some(Ddr2Cmd::Precharge));
    assert_eq!(executor_op_to_cmd(4), Some(Ddr2Cmd::Refresh));
    assert_eq!(
        executor_op_to_cmd(5),
        None,
        "op 5 is not an executor command"
    );
    assert_eq!(executor_op_to_cmd(7), None);
}

/// The emitted encoder is a well-formed combinational module whose ACTIVE/READ/WRITE/PRE/REF
/// arms carry EXACTLY the truth-table pins — the netlist is generated from `ddr2_pins`, so this
/// pins the single-source-of-truth relationship (a table change ripples into the RTL).
#[test]
fn emitted_encoder_is_generated_from_the_truth_table() {
    let v = emit_ddr2_command_encoder_verilog();
    assert!(v.contains("module ddr2_cmd_encoder"), "names the module");
    for op in 0u8..5 {
        let cmd = executor_op_to_cmd(op).unwrap();
        let (cs, ras, cas, we) = ddr2_pins(cmd);
        let arm = format!(
            "3'd{op}: begin cs_n=1'b{cs}; ras_n=1'b{ras}; cas_n=1'b{cas}; we_n=1'b{we}; end"
        );
        assert!(
            v.contains(&arm),
            "encoder must carry the generated arm for {cmd:?}: {arm}"
        );
    }
    // The `issue`-low path holds NOP (bus idle), never an accidental ACTIVE/PRECHARGE.
    let (cs, ras, cas, we) = ddr2_pins(Ddr2Cmd::Nop);
    assert!(
        v.contains(&format!(
            "cs_n=1'b{cs}; ras_n=1'b{ras}; cas_n=1'b{cas}; we_n=1'b{we}; // NOP"
        )),
        "issue-low must hold NOP"
    );
}

// ── Phase 1b: DDR2 init / mode-register sequence (JEDEC invariant locks) ──────────

fn seq() -> Vec<InitStep> {
    ddr2_init_sequence(&DramTimings::ddr2_micron(), 200_000)
}

fn mode_loads(s: &[InitStep]) -> Vec<(ModeReg, u16)> {
    s.iter()
        .filter_map(|st| match st {
            InitStep::LoadMode { reg, a } => Some((*reg, *a)),
            _ => None,
        })
        .collect()
}

/// Init begins with the CKE-low power-up wait, and PRECHARGE ALL precedes any mode-register load.
#[test]
fn ddr2_init_starts_low_and_precharges_before_mode_registers() {
    let s = seq();
    assert!(
        matches!(s.first(), Some(InitStep::CkeLowWait { .. })),
        "init must begin with the CKE-low power-up wait"
    );
    let first_pre = s.iter().position(|x| matches!(x, InitStep::PrechargeAll));
    let first_mrs = s
        .iter()
        .position(|x| matches!(x, InitStep::LoadMode { .. }));
    let (first_pre, first_mrs) = (
        first_pre.expect("has PRECHARGE ALL"),
        first_mrs.expect("has a mode-register load"),
    );
    assert!(
        first_pre < first_mrs,
        "PRECHARGE ALL must precede any mode-register load"
    );
}

/// The mode registers are programmed in the exact JEDEC order, with the correct BA selectors.
#[test]
fn ddr2_init_programs_mode_registers_in_jedec_order() {
    let regs: Vec<ModeReg> = mode_loads(&seq()).into_iter().map(|(r, _)| r).collect();
    assert_eq!(
        regs,
        vec![
            ModeReg::Emr2,
            ModeReg::Emr3,
            ModeReg::Emr1, // enable DLL
            ModeReg::Mr,   // DLL reset
            ModeReg::Mr,   // normal operation
            ModeReg::Emr1, // OCD default
            ModeReg::Emr1, // OCD exit
        ],
        "JEDEC: EMR2 → EMR3 → EMR1(DLL en) → MR(DLL reset) → MR(normal) → EMR1(OCD default) → EMR1(OCD exit)"
    );
    assert_eq!(
        (
            ModeReg::Mr.ba(),
            ModeReg::Emr1.ba(),
            ModeReg::Emr2.ba(),
            ModeReg::Emr3.ba()
        ),
        (0, 1, 2, 3)
    );
}

/// DLL is reset by the first MR (A8 = 1) and cleared by the second (A8 = 0) — reset before clear.
#[test]
fn ddr2_init_resets_dll_once_then_runs_with_it_cleared() {
    let mrs: Vec<u16> = mode_loads(&seq())
        .into_iter()
        .filter_map(|(r, a)| (r == ModeReg::Mr).then_some(a))
        .collect();
    assert_eq!(mrs.len(), 2, "exactly two MR loads: DLL-reset then normal");
    assert_eq!((mrs[0] >> 8) & 1, 1, "first MR asserts DLL reset (A8)");
    assert_eq!((mrs[1] >> 8) & 1, 0, "second MR clears DLL reset");
}

/// JEDEC requires at least two auto-refreshes during init.
#[test]
fn ddr2_init_issues_at_least_two_auto_refreshes() {
    let n = seq()
        .iter()
        .filter(|x| matches!(x, InitStep::Refresh))
        .count();
    assert!(n >= 2, "init needs ≥2 AUTO-REFRESH, got {n}");
}

/// OCD calibration default (A9:A7 = 111) is followed later by OCD exit (A9:A7 = 000).
#[test]
fn ddr2_init_does_ocd_default_then_exit() {
    let emr1: Vec<u16> = mode_loads(&seq())
        .into_iter()
        .filter_map(|(r, a)| (r == ModeReg::Emr1).then_some(a))
        .collect();
    let default_idx = emr1
        .iter()
        .position(|a| (a >> 7) & 0b111 == 0b111)
        .expect("an EMR1 with OCD default (A9:A7 = 111)");
    assert!(
        emr1[default_idx + 1..]
            .iter()
            .any(|a| (a >> 7) & 0b111 == 0),
        "OCD default must be followed by OCD exit (A9:A7 = 000)"
    );
}

/// The MR opcode encodes burst length (BL8), sequential burst type, CAS latency, and write recovery.
#[test]
fn ddr2_mr_opcode_encodes_bl_cl_and_write_recovery() {
    let t = DramTimings::ddr2_micron();
    let mr = mr_opcode(&t, false);
    assert_eq!(mr & 0b111, 0b011, "BL8 in A2:A0");
    assert_eq!((mr >> 3) & 1, 0, "sequential burst type (A3 = 0)");
    assert_eq!(
        (mr >> 4) & 0b111,
        (t.trcd as u16) & 0b111,
        "CAS latency in A6:A4 (== tRCD)"
    );
    assert_eq!(
        (mr >> 9) & 0b111,
        (t.twr as u16 - 1) & 0b111,
        "write recovery WR-1 in A11:A9"
    );
    assert_eq!(
        (mr_opcode(&t, true) >> 8) & 1,
        1,
        "DLL-reset variant sets A8"
    );
}

/// The emitted `ddr2_init_fsm` ROM is GENERATED from the JEDEC sequence — every entry matches
/// `init_rom_word`, so a spec change ripples into the netlist (single source of truth).
#[test]
fn emitted_init_fsm_is_generated_from_the_sequence() {
    let t = DramTimings::ddr2_micron();
    let s = ddr2_init_sequence(&t, 200_000);
    let v = emit_ddr2_init_fsm_verilog(&t, 200_000);
    assert!(v.contains("module ddr2_init_fsm"), "names the module");
    assert!(
        v.contains(&format!("localparam NSTEPS = {}", s.len())),
        "NSTEPS matches the spec"
    );
    for (i, step) in s.iter().enumerate() {
        let w = init_rom_word(step);
        assert!(
            v.contains(&format!("rom[{i}] = 44'h{w:011x};")),
            "ROM entry {i} must be generated from the spec ({step:?})"
        );
    }
}

/// The command-encoder testbench is self-checking and exercises every op with issue = 0 and 1.
#[test]
fn cmd_encoder_testbench_is_self_checking() {
    let tb = emit_ddr2_cmd_encoder_testbench();
    assert!(
        tb.contains("module tb_ddr2_cmd_encoder"),
        "names the testbench"
    );
    assert!(tb.contains("ddr2_cmd_encoder dut"), "instantiates the DUT");
    assert!(tb.contains("PASS:"), "reports PASS");
    assert_eq!(
        tb.matches("chk(3'd").count(),
        16,
        "8 ops × {{issue 0, issue 1}}"
    );
}

/// The init testbench embeds the expected JEDEC command log (generated from the sequence).
#[test]
fn init_testbench_embeds_the_expected_sequence() {
    let t = DramTimings::ddr2_micron();
    let tb = emit_ddr2_init_testbench(&t, 8);
    assert!(
        tb.contains("module tb_ddr2_init_fsm"),
        "names the testbench"
    );
    assert!(tb.contains("ddr2_init_fsm dut"), "instantiates the DUT");
    assert!(
        tb.contains("localparam EXP_N = 11"),
        "11 real commands in the JEDEC sequence"
    );
    assert!(tb.contains("PASS:"), "reports PASS");
}

// ── Phase 1d: read/write transaction command sequencing ──────────────────────────

/// A read transaction is ACT(row) → RD → PRE, JEDEC-legal, with the CAS at tRCD.
#[test]
fn read_transaction_is_legal_and_well_formed() {
    let t = DramTimings::ddr2_micron();
    let s = emit_transaction(&t, 3, 42, false);
    check_legal(&t, &s).expect("a read transaction is timing-legal");
    assert!(matches!(
        s.first().map(|c| c.cmd),
        Some(DramCmd::Act { bank: 3, row: 42 })
    ));
    assert!(
        s.iter().any(|c| matches!(c.cmd, DramCmd::Rd { bank: 3 })),
        "issues a READ"
    );
    assert!(
        matches!(s.last().map(|c| c.cmd), Some(DramCmd::Pre { bank: 3 })),
        "closes the page"
    );
    let cas = s
        .iter()
        .find(|c| matches!(c.cmd, DramCmd::Rd { .. }))
        .unwrap();
    assert_eq!(cas.at, t.trcd, "CAS lands tRCD after ACT");
}

/// A write transaction closes the page only after write recovery (tWR) — proven by check_legal.
#[test]
fn write_transaction_respects_write_recovery() {
    let t = DramTimings::ddr2_micron();
    let s = emit_transaction(&t, 1, 100, true);
    check_legal(&t, &s).expect("a write transaction is timing-legal");
    let wr = s
        .iter()
        .find(|c| matches!(c.cmd, DramCmd::Wr { .. }))
        .unwrap();
    let pre = s
        .iter()
        .find(|c| matches!(c.cmd, DramCmd::Pre { .. }))
        .unwrap();
    assert!(
        pre.at >= wr.at + t.data_cycles + t.twr,
        "PRE waits tWR after the write burst"
    );
}

/// THE UNIFYING INVARIANT: the controller's transaction stream and the ComputeDRAM stream use the
/// SAME ACT/CAS/PRE vocabulary — but a memory transaction is check_legal-LEGAL while the compute
/// stream is check_legal-ILLEGAL. One command bus, two planes: normal memory vs in-DRAM compute.
#[test]
fn memory_transactions_are_legal_while_computedram_streams_are_not() {
    let t = DramTimings::ddr2_micron();
    for write in [false, true] {
        check_legal(&t, &emit_transaction(&t, 2, 7, write)).expect("memory transaction is legal");
    }
    let compute = emit_computedram_stream(
        &ComputeDramOp::RowCopy {
            bank: 2,
            src: 7,
            dst: 8,
        },
        &ComputeDramTiming::nominal(),
    );
    assert!(
        check_legal(&t, &compute).is_err(),
        "the compute stream is deliberately illegal"
    );
}

/// The controller / model / testbench emitters produce well-formed, timing-baked modules.
#[test]
fn controller_and_model_emitters_are_well_formed() {
    let t = DramTimings::ddr2_micron();
    let c = emit_ddr2_controller_verilog(&t);
    let m = emit_ddr2_bank_model_verilog(&t);
    let tb = emit_ddr2_controller_testbench(&t, 8);
    assert!(c.contains("module ddr2_ctrl"), "controller module");
    assert!(
        c.contains(&format!("TRCD={}", t.trcd)),
        "timings baked from the table"
    );
    assert!(
        !c.contains("%TRCD%") && !c.contains("%TRAS%"),
        "controller tokens substituted"
    );
    assert!(
        m.contains("module ddr2_bank_model") && m.contains("model_error"),
        "checker model"
    );
    assert!(
        !m.contains("%TRCD%") && !m.contains("%BURST%"),
        "model tokens substituted"
    );
    assert!(
        tb.contains("module tb_ddr2_ctrl") && tb.contains("PASS:"),
        "self-checking testbench"
    );
    assert!(
        tb.contains("localparam N=8") && tb.contains("addr[7]="),
        "8-word access pattern"
    );
    assert!(
        !tb.contains("%N%") && !tb.contains("%INIT%"),
        "testbench tokens substituted"
    );
}
