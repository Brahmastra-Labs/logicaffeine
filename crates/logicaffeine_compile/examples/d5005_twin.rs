//! Write the D5005 cycle twin's prediction — Qwen3.8-27B's step program scheduled on the Intel PAC D5005's units at
//! every knob point the real checkpoints are held to, beside the roofline's price — to
//! `benchmarks/silicon/d5005/predicted.json`, and print it. Usage:
//!   cargo run -p logicaffeine-compile --example d5005_twin

use logicaffeine_compile::spatial::decode_engine::twin::predicted;

fn main() {
    let silicon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    let p = predicted(&silicon.join("decode")).unwrap_or_else(|e| panic!("the prediction: {e}"));
    for row in &p.rows {
        println!(
            "{:<120} B {:>4}  {:>7.1} tok/s (roofline {:>7.1})  step {:>7.2} ms  mem {:>6.2} array {:>6.2} lanes {:>6.2} vector {:>6.2} ms  window {:>5.1} MB",
            row.numerics,
            row.batch,
            row.tokens_per_s,
            row.roofline_tokens_per_s,
            row.step_s * 1e3,
            row.memory_s * 1e3,
            row.array_s * 1e3,
            row.lanes_s * 1e3,
            row.vector_s * 1e3,
            row.window_bytes / 1e6
        );
    }
    let path = silicon.join("d5005/predicted.json");
    std::fs::write(&path, serde_json::to_string_pretty(&p).expect("the prediction serializes") + "\n")
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("wrote {}", path.display());
}
