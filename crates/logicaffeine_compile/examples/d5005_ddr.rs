//! Write the D5005 DDR4 ledger — every kind of stream a decode pass issues, priced on one of the card's channels by
//! the JESD79-4 controller model — to `benchmarks/silicon/d5005/ddr4_efficiency.json`, and print it. Usage:
//!   cargo run -p logicaffeine-compile --example d5005_ddr

use logicaffeine_compile::spatial::decode_engine::ddr::ddr_ledger;

fn main() {
    let silicon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    let l = ddr_ledger(&silicon.join("decode")).unwrap_or_else(|e| panic!("the DDR4 ledger: {e}"));
    println!(
        "{}: RL {} WL {} tRCD {} tRP {} tRAS {} tFAW {} tRFC {} tREFI {} clocks at {} MHz; state {} bytes, a cache position {} bytes",
        l.timing.grade.part,
        l.timing.rl,
        l.timing.wl,
        l.timing.trcd,
        l.timing.trp,
        l.timing.tras,
        l.timing.tfaw,
        l.timing.trfc,
        l.timing.trefi,
        l.timing.grade.mem_mhz,
        l.state_bytes,
        l.position_bytes
    );
    for r in &l.rows {
        println!(
            "{:<96} {:?}  {:>6.2}%  {:>6.2} GB/s a channel  {:>6.2} GB/s the card  {:>5} turns {:>4} refreshes",
            r.pattern,
            r.beat,
            r.efficiency * 100.0,
            r.channel_bytes_per_s / 1e9,
            r.card_bytes_per_s / 1e9,
            r.facts.turnarounds,
            r.facts.refreshes
        );
    }
    let path = silicon.join("d5005/ddr4_efficiency.json");
    std::fs::write(&path, serde_json::to_string_pretty(&l).expect("the ledger serializes") + "\n")
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("wrote {}", path.display());
}
