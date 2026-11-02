//! emit_systolic_pipelined_tile — emit PIPELINED, register-staged weight-stationary systolic tiles (the
//! TPU-MXU microarchitecture: one mul+add between flip-flops → K-INDEPENDENT critical path) at 8×8 AND 16×16,
//! each with a self-contained BIST harness (LFSR stimulus → MISR signature, only clk/rst/sig cross the
//! boundary). Taping BOTH shows the critical path stays ~one-MAC as the array grows — vs the combinational
//! tile's K-deep tree (11 ns at 8×8). Feeds:
//!   OUT_JSON=benchmarks/silicon/systolic_pipelined_signoff.json scripts/openlane-run.sh systolic_pipe8_realbist clk <period> systolic_pipe8_realbist.v systolic_pipe8.v
//!   scripts/measure-power-vcd.sh systolic_pipe8_realbist
//!   cargo run -q -p logicaffeine-compile --example emit_systolic_pipelined_tile
use std::fs;

use logicaffeine_compile::codegen_sva::systolic::{
    bist_tile_weights, emit_systolic_bist_wrapper, emit_systolic_os_bist_wrapper,
    emit_systolic_os_pipelined_verilog_bits_acc, emit_systolic_pipelined_verilog_bits_acc, Systolic, SystolicOs,
    ACC_BITS_MXU,
};

fn main() {
    let sil = "benchmarks/silicon";
    // weight-stationary pipelined tiles at 8×8 AND 16×16 (Fmax flatness demonstrator)
    for &d in &[8usize, 16usize] {
        let sys = Systolic::new(d, d, bist_tile_weights(d, d));
        let core = format!("systolic_pipe{d}");
        let array = emit_systolic_pipelined_verilog_bits_acc(&core, &sys, 8, ACC_BITS_MXU);
        fs::write(format!("{sil}/{core}.v"), &array).unwrap();
        fs::write(format!("{sil}/{core}_realbist.v"), emit_systolic_bist_wrapper(&core, d, d)).unwrap();
        println!(
            "emitted {sil}/{core}.v ({d}×{d} WS pipelined tile, int{ACC_BITS_MXU} acc, latency L={}) + {core}_realbist.v",
            d + d - 1
        );
    }
    // output-stationary pipelined tile at 8×8 (the SN40L PCU's other dataflow)
    {
        let os = SystolicOs::new(8, 8, 8);
        let core = "systolic_os_pipe8";
        let array = emit_systolic_os_pipelined_verilog_bits_acc(core, &os, 8, ACC_BITS_MXU);
        fs::write(format!("{sil}/{core}.v"), &array).unwrap();
        fs::write(format!("{sil}/{core}_realbist.v"), emit_systolic_os_bist_wrapper(core, 8, 8)).unwrap();
        println!(
            "emitted {sil}/{core}.v (8×8×8 OS pipelined tile, int{ACC_BITS_MXU} acc, latency L={}) + {core}_realbist.v",
            8 + 8 + 8 - 2
        );
    }
    println!("tape:  OUT_JSON={sil}/systolic_pipelined_signoff.json scripts/openlane-run.sh systolic_pipe8_realbist clk 5 systolic_pipe8_realbist.v systolic_pipe8.v");
    println!("       (also systolic_pipe16_realbist, systolic_os_pipe8_realbist); measure: scripts/measure-power-vcd.sh <block>");
}
