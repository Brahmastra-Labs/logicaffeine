//! emit_psram_edge — write the off-chip memory edge for the APS6404L-3SQN QSPI PSRAM into
//! benchmarks/silicon/: the controller (psram_edge.v, at a 50 MHz system clock and a 25 MHz SCLK)
//! and the part's pin-level model (psram_model.v). Locked by psram_edge_lock.
//! Run: `cargo run -p logicaffeine-compile --example emit_psram_edge`.

use logicaffeine_compile::codegen_sva::psram::{
    emit_psram_edge_verilog, emit_psram_model_verilog, PsramEdge, APS6404L_3SQN,
};

fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    let edge =
        PsramEdge::new(APS6404L_3SQN, 20_000, 1).expect("a 25 MHz SCLK is legal for the part");
    std::fs::write(dir.join("psram_edge.v"), emit_psram_edge_verilog(&edge))
        .expect("write psram_edge.v");
    std::fs::write(
        dir.join("psram_model.v"),
        emit_psram_model_verilog(&edge.part),
    )
    .expect("write psram_model.v");
    println!("wrote psram_edge.v and psram_model.v ({} MHz SCLK, bursts of up to {} read / {} write words)",
        1_000_000 / edge.sclk_period_ps(), edge.max_words(false), edge.max_words(true));
}
