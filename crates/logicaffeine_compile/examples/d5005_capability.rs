//! The decode engine's capability matrix: the verdict on every checkpoint header ledger in
//! `benchmarks/silicon/decode/headers`, written to `benchmarks/silicon/d5005/capability.json`. A checkpoint the engine
//! computes is admitted; any other carries each gap that stands in its way, the hardware and lowering backlog a model
//! asks of the engine.
//!
//!     cargo run -p logicaffeine-compile --example d5005_capability

use std::path::PathBuf;

use logicaffeine_compile::spatial::decode_engine::capability::{matrix, Verdict};

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rows = matrix(&root.join("benchmarks/silicon/decode/headers")).unwrap_or_else(|e| {
        eprintln!("d5005_capability: {e}");
        std::process::exit(1)
    });
    for r in &rows {
        match &r.verdict {
            Verdict::Admitted => println!("{:40} admitted", r.checkpoint),
            Verdict::Refused(why) => println!("{:40} refused: {}", r.checkpoint, why.join("; ")),
            Verdict::Undescribed(why) => println!("{:40} undescribed: {why}", r.checkpoint),
        }
    }
    let out = root.join("benchmarks/silicon/d5005/capability.json");
    let mut text = serde_json::to_string_pretty(&rows).expect("rows serialize");
    text.push('\n');
    std::fs::write(&out, text).unwrap_or_else(|e| {
        eprintln!("d5005_capability: {}: {e}", out.display());
        std::process::exit(1)
    });
}
