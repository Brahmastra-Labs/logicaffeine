//! computedram_sim_lock — Phase 4: the in-DRAM compute path proven end-to-end under iverilog.
//!
//! A `ComputeDramOp::RowCopy` is lowered to its raw, deliberately-timing-illegal ACT-PRE-ACT stream
//! (`emit_computedram_stream`), delta-encoded, and replayed through the PROVEN `dram_executor` (Phase 2:
//! raw sub-JEDEC command control) into the behavioral `computedram_model` (which implements the RowClone
//! charge-sharing effect). The harness — playing the fabric's orchestration role — preloads the source
//! row, triggers the stream, and reads the destination back, asserting it equals the source (the
//! RowStore/VM oracle). This is "compute over RAM" proven in simulation.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(computedram_sim_lock)'`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::computedram_rtl::{
    emit_computedram_harness_testbench, emit_computedram_model_verilog,
    emit_computedram_reduce_testbench,
};
use logicaffeine_compile::codegen_sva::dram::{
    delta_encode, emit_computedram_stream, emit_dram_executor_verilog, ComputeDramOp,
    ComputeDramTiming,
};

/// A ComputeDRAM RowClone, executed as raw sub-JEDEC commands through the executor into the behavioral
/// device, copies the source row to the destination — byte-identical to the oracle — under iverilog.
#[test]
fn computedram_rowclone_runs_as_raw_commands_and_equals_the_oracle_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (src, dst) = (10usize, 20usize);
    let pattern = 0xDEAD_BEEF_CAFE_F00Du64;

    let stream = emit_computedram_stream(
        &ComputeDramOp::RowCopy {
            bank: 0,
            src: src as u64,
            dst: dst as u64,
        },
        &ComputeDramTiming::nominal(),
    );
    let words = delta_encode(&stream);

    let dir = std::env::temp_dir().join("logos_computedram");
    std::fs::create_dir_all(&dir).expect("create dir");
    let hex: String = words.iter().map(|w| format!("{w:016x}\n")).collect();
    std::fs::write(dir.join("cdram_plan.hex"), hex).expect("write hex");
    let rtl = format!(
        "{}\n{}",
        emit_dram_executor_verilog(),
        emit_computedram_model_verilog()
    );
    let tb = emit_computedram_harness_testbench(words.len(), src, dst, pattern);
    let srcv = dir.join("cdram.v");
    let tbv = dir.join("tb_cdram.v");
    let sim = dir.join("sim_cdram.vvp");
    std::fs::write(&srcv, &rtl).expect("write rtl");
    std::fs::write(&tbv, &tb).expect("write tb");

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
        "iverilog failed: {}\n--- RTL ---\n{rtl}\n--- TB ---\n{tb}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("run vvp");
    let out = String::from_utf8_lossy(&run.stdout);
    assert!(
        out.contains("PASS:"),
        "ComputeDRAM RowClone did not pass:\n{out}"
    );
}

/// In-DRAM bitwise AND and OR via triple-row activation (the sense-amp majority: AND = MAJ(a,b,0),
/// OR = MAJ(a,b,1)) executed as raw sub-JEDEC command streams through the executor into the device,
/// byte-identical to the oracle — under iverilog. Actual COMPUTATION inside the memory, not just copy.
#[test]
fn computedram_bitwise_and_or_via_triple_activation_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (a, b, z, a2, b2, ones) = (1usize, 2, 3, 4, 5, 6);
    let (av, bv) = (0xF0F0_F0F0_F0F0_F0F0u64, 0xFF00_FF00_FF00_FF00u64);
    let nom = ComputeDramTiming::nominal();
    let s_and = emit_computedram_stream(
        &ComputeDramOp::Maj {
            bank: 0,
            a: a as u64,
            b: b as u64,
            c: z as u64,
            dst: a as u64,
        },
        &nom,
    );
    let s_or = emit_computedram_stream(
        &ComputeDramOp::Maj {
            bank: 0,
            a: a2 as u64,
            b: b2 as u64,
            c: ones as u64,
            dst: a2 as u64,
        },
        &nom,
    );
    let mut rw = delta_encode(&s_and);
    rw.extend(delta_encode(&s_or));

    let dir = std::env::temp_dir().join("logos_computedram_reduce");
    std::fs::create_dir_all(&dir).expect("create dir");
    std::fs::write(
        dir.join("cdram_reduce.hex"),
        rw.iter().map(|w| format!("{w:016x}\n")).collect::<String>(),
    )
    .expect("write hex");
    let rtl = format!(
        "{}\n{}",
        emit_dram_executor_verilog(),
        emit_computedram_model_verilog()
    );
    let tb = emit_computedram_reduce_testbench(rw.len(), a, b, z, a2, b2, ones, av, bv);
    let srcv = dir.join("cdram.v");
    let tbv = dir.join("tb_cdram_reduce.v");
    let sim = dir.join("sim_reduce.vvp");
    std::fs::write(&srcv, &rtl).expect("write rtl");
    std::fs::write(&tbv, &tb).expect("write tb");

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
        "iverilog failed: {}\n--- TB ---\n{tb}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("run vvp");
    let out = String::from_utf8_lossy(&run.stdout);
    assert!(out.contains("PASS:"), "in-DRAM AND/OR did not pass:\n{out}");
}
