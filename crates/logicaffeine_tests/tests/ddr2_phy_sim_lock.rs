//! ddr2_phy_sim_lock — Phase 1 behavioral proof of the hand-rolled DDR2 controller under iverilog.
//!
//! Two RTL bricks of the custom DDR2 PHY (`codegen_sva::ddr2_phy`) are proven behaviorally, not just
//! structurally:
//!   • `ddr2_cmd_encoder` — the op field → DDR2 pins mapping matches the JEDEC command truth table for
//!     every op and both `issue` values (a self-checking testbench, expectations from `ddr2_pins`);
//!   • `ddr2_init_fsm` — the power-up sequencer actually WALKS the JEDEC mode-register sequence: a bus
//!     monitor records each real command it issues and asserts the log equals `ddr2_init_sequence`,
//!     and that `init_done` asserts.
//!
//! This turns Phase 1a/1b from structurally-locked (ROM generated-from-spec) into behavior-proven, and
//! establishes the iverilog harness every later PHY brick reuses. Skips cleanly if iverilog is absent.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(ddr2_phy_sim_lock)'`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::ddr2_phy::{
    emit_ddr2_bank_model_verilog, emit_ddr2_cmd_encoder_testbench,
    emit_ddr2_command_encoder_verilog, emit_ddr2_controller_testbench,
    emit_ddr2_controller_verilog, emit_ddr2_init_fsm_verilog, emit_ddr2_init_testbench,
};
use logicaffeine_compile::codegen_sva::dram::DramTimings;

/// Compile `rtl` + `tb` with iverilog into a temp dir and return the vvp stdout.
fn run_sim(name: &str, rtl: &str, tb: &str) -> String {
    let dir = std::env::temp_dir().join(format!("logos_ddr2_{name}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let srcv = dir.join(format!("{name}.v"));
    let tbv = dir.join(format!("tb_{name}.v"));
    let sim = dir.join(format!("sim_{name}.vvp"));
    std::fs::write(&srcv, rtl).expect("write rtl");
    std::fs::write(&tbv, tb).expect("write tb");
    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed to compile {name}: {}\n--- RTL ---\n{rtl}\n--- TB ---\n{tb}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("run vvp");
    String::from_utf8_lossy(&run.stdout).to_string()
}

/// The DDR2 command encoder maps op → pins per the JEDEC truth table, proven under iverilog.
#[test]
fn ddr2_cmd_encoder_matches_jedec_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let out = run_sim(
        "ddr2_cmd_encoder",
        &emit_ddr2_command_encoder_verilog(),
        &emit_ddr2_cmd_encoder_testbench(),
    );
    assert!(
        out.contains("PASS:"),
        "ddr2_cmd_encoder sim did not pass:\n{out}"
    );
}

/// The DDR2 init FSM walks the full JEDEC power-up / mode-register sequence and asserts init_done,
/// proven under iverilog (a small CKE-low count keeps the sim fast; the sequence structure is identical).
#[test]
fn ddr2_init_fsm_walks_the_jedec_sequence_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let t = DramTimings::ddr2_micron();
    let out = run_sim(
        "ddr2_init_fsm",
        &emit_ddr2_init_fsm_verilog(&t, 8),
        &emit_ddr2_init_testbench(&t, 8),
    );
    assert!(
        out.contains("PASS:"),
        "ddr2_init_fsm sim did not pass:\n{out}"
    );
}

/// The DDR2 controller FSM round-trips write→read through the behavioral DDR2 model with no timing
/// violations, proven under iverilog — the last fully-simulatable controller layer before analog I/O.
#[test]
fn ddr2_controller_round_trips_write_then_read_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let t = DramTimings::ddr2_micron();
    let rtl = format!(
        "{}\n{}",
        emit_ddr2_controller_verilog(&t),
        emit_ddr2_bank_model_verilog(&t)
    );
    let tb = emit_ddr2_controller_testbench(&t, 8);
    let out = run_sim("ddr2_ctrl", &rtl, &tb);
    assert!(
        out.contains("PASS:"),
        "ddr2_ctrl round-trip did not pass:\n{out}"
    );
}
