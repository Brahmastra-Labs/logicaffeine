//! Emit the DDR2 PHY simulation artifacts (command encoder, init FSM, controller + behavioral
//! model, and their self-checking testbenches) into a directory for iverilog. A dev tool mirroring
//! the other `emit_*` examples; the checked-in proof is `ddr2_phy_sim_lock` in logicaffeine-tests.
//!
//!   cargo run -p logicaffeine-compile --example emit_ddr2_sim -- /tmp/logos_ddr2_port

use logicaffeine_compile::codegen_sva::computedram_rtl::{
    emit_computedram_harness_testbench, emit_computedram_model_verilog,
    emit_computedram_reduce_testbench,
};
use logicaffeine_compile::codegen_sva::ddr2_phy::{
    emit_ddr2_bank_model_verilog, emit_ddr2_cmd_encoder_testbench,
    emit_ddr2_command_encoder_verilog, emit_ddr2_controller_testbench,
    emit_ddr2_controller_verilog, emit_ddr2_init_fsm_verilog, emit_ddr2_init_testbench,
};
use logicaffeine_compile::codegen_sva::dram::{
    delta_encode, emit_computedram_stream, emit_dram_executor_verilog, ComputeDramOp,
    ComputeDramTiming, DramTimings,
};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/logos_ddr2_port".to_string());
    let dir = std::path::Path::new(&out);
    std::fs::create_dir_all(dir).expect("create dir");
    let t = DramTimings::ddr2_micron();

    let write = |name: &str, body: String| std::fs::write(dir.join(name), body).expect("write");
    write("enc.v", emit_ddr2_command_encoder_verilog());
    write("enc_tb.v", emit_ddr2_cmd_encoder_testbench());
    write("init.v", emit_ddr2_init_fsm_verilog(&t, 8));
    write("init_tb.v", emit_ddr2_init_testbench(&t, 8));
    // Hardware init FSM: 200 µs CKE-low at 125 MHz = 25000 cycles.
    write("init_hw.v", emit_ddr2_init_fsm_verilog(&t, 25000));
    write(
        "ctrl.v",
        format!(
            "{}\n{}",
            emit_ddr2_controller_verilog(&t),
            emit_ddr2_bank_model_verilog(&t)
        ),
    );
    write("ctrl_tb.v", emit_ddr2_controller_testbench(&t, 8));

    // Phase 4: the ComputeDRAM in-DRAM compute path (RowClone via raw sub-JEDEC commands).
    let (src, dst, pattern) = (10usize, 20usize, 0xDEAD_BEEF_CAFE_F00Du64);
    let stream = emit_computedram_stream(
        &ComputeDramOp::RowCopy {
            bank: 0,
            src: src as u64,
            dst: dst as u64,
        },
        &ComputeDramTiming::nominal(),
    );
    let words = delta_encode(&stream);
    std::fs::write(
        dir.join("cdram_plan.hex"),
        words
            .iter()
            .map(|w| format!("{w:016x}\n"))
            .collect::<String>(),
    )
    .expect("write hex");
    write(
        "cdram.v",
        format!(
            "{}\n{}",
            emit_dram_executor_verilog(),
            emit_computedram_model_verilog()
        ),
    );
    write(
        "cdram_tb.v",
        emit_computedram_harness_testbench(words.len(), src, dst, pattern),
    );

    // Phase 4 reductions: in-DRAM bitwise AND/OR via triple-row activation.
    let (a, b, z, a2, b2, ones) = (1usize, 2, 3, 4, 5, 6);
    let (av, bv) = (0xF0F0_F0F0_F0F0_F0F0u64, 0xFF00_FF00_FF00_FF00u64);
    let s_and = emit_computedram_stream(
        &ComputeDramOp::Maj {
            bank: 0,
            a: a as u64,
            b: b as u64,
            c: z as u64,
            dst: a as u64,
        },
        &ComputeDramTiming::nominal(),
    );
    let s_or = emit_computedram_stream(
        &ComputeDramOp::Maj {
            bank: 0,
            a: a2 as u64,
            b: b2 as u64,
            c: ones as u64,
            dst: a2 as u64,
        },
        &ComputeDramTiming::nominal(),
    );
    let mut rw = delta_encode(&s_and);
    rw.extend(delta_encode(&s_or));
    std::fs::write(
        dir.join("cdram_reduce.hex"),
        rw.iter().map(|w| format!("{w:016x}\n")).collect::<String>(),
    )
    .expect("write reduce hex");
    write(
        "cdram_reduce_tb.v",
        emit_computedram_reduce_testbench(rw.len(), a, b, z, a2, b2, ones, av, bv),
    );

    // AND-only stream (for the on-silicon SoC): Maj{a,b,zero} → mem[a] := a & b.
    let and_words = delta_encode(&s_and);
    std::fs::write(
        dir.join("computedram_and.hex"),
        and_words
            .iter()
            .map(|w| format!("{w:016x}\n"))
            .collect::<String>(),
    )
    .expect("write and hex");
    println!(
        "AND stream: {} executor words (rows a={a} b={b} zero={z})",
        and_words.len()
    );

    println!(
        "wrote DDR2 + ComputeDRAM sim artifacts to {}",
        dir.display()
    );
}
