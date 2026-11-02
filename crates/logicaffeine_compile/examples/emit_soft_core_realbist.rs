//! emit_soft_core_realbist — write the CPU twin of `decision_hft_realbist` into benchmarks/silicon/:
//!   * soft_core_realbist.v   — the block top (the scalar core, its boot ROM the program below)
//!   * soft_core_realbist.hex — that boot ROM: the HFT decision over the realbist's input ranges,
//!                              compiled to the scalar core's words (codegen_sva::decision_workload)
//! The flow copies the .hex beside the core as the `program.hex` its `$readmemh` reads. Locked by
//! soft_core_realbist_lock. Run: `cargo run -p logicaffeine-compile --example emit_soft_core_realbist`.

use logicaffeine_compile::codegen_sva::decision_workload::{
    soft_core_realbist_program, soft_core_realbist_verilog,
};
use logicaffeine_compile::codegen_sva::scalar_asm::to_hex;

fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    let words = soft_core_realbist_program();
    std::fs::write(
        dir.join("soft_core_realbist.v"),
        soft_core_realbist_verilog(),
    )
    .expect("write soft_core_realbist.v");
    std::fs::write(dir.join("soft_core_realbist.hex"), to_hex(&words))
        .expect("write soft_core_realbist.hex");
    println!(
        "wrote soft_core_realbist.v and soft_core_realbist.hex ({} words of 256)",
        words.len()
    );
}
