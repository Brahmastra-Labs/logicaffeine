//! Write the decode-chip report — what SRAM-resident LOGOS decode chips must achieve to beat a GPU floor per user —
//! to `benchmarks/silicon/decode/decode_chip.json`, and print its requirements. Usage:
//!   cargo run -p logicaffeine-compile --example decode_chip_report

use logicaffeine_compile::spatial::decode_chip::report;

fn main() {
    let decode = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/decode");
    let r = report(&decode).unwrap_or_else(|e| panic!("the report: {e}"));
    let ns = |s: Option<f64>| s.map_or("unreachable".to_string(), |s| format!("{:.1} ns", s * 1e9));
    for m in &r.models {
        println!("{}  ({:.2} GB per token, {} dependent stages)", m.model, m.weight_bytes_per_token / 1e9, m.stages);
        for f in &m.gpu_floors {
            println!("  floor  {} x {:<10} {:>9.1} tok/s  ({:?})", f.chips, f.machine, f.floor_tok_s, f.binds);
        }
        for d in &m.demonstrated {
            println!("  shown  {:<60} {:>7.1} ns per stage", d.label, d.stage_latency_s * 1e9);
        }
        for row in &m.requirements {
            let q = &row.requirement;
            println!(
                "  {:>4}x {:<11} → {:>9.1} tok/s on {:>6} {} {:?} chips: stage ≤ {}, hop ≤ {} (design stage {:.1} ns)",
                row.speedup,
                row.baseline,
                q.target_tok_s,
                q.chips,
                q.node,
                q.network,
                ns(q.stage_budget_s),
                ns(q.hop_budget_s),
                q.design_stage_s * 1e9,
            );
        }
    }
    let out = decode.join("decode_chip.json");
    let text = serde_json::to_string_pretty(&r).expect("the report serializes") + "\n";
    std::fs::write(&out, text).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    println!("wrote {}", out.display());
}
