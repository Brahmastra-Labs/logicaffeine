//! Write the D5005 roofline — Qwen3.8-27B priced on the Intel PAC D5005 at every knob point of the decode engine,
//! one trick at a time — to `benchmarks/silicon/d5005/roofline.json`, and print it. Usage:
//!   cargo run -p logicaffeine-compile --example d5005_roofline

use logicaffeine_compile::spatial::d5005::roofline;

fn main() {
    let silicon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    let r = roofline(&silicon.join("decode")).unwrap_or_else(|e| panic!("the roofline: {e}"));
    println!(
        "{}: array {:.2} T multiply-adds/s, lanes {:.1} G cycles/s, vector {:.0} G f32 multiply-adds/s, memory {:.1} GB/s \
         sustained {:.2}% on a read, {:.2}% rewritten in place, {:.2}% read and appended",
        r.card.name,
        r.card.array.value / 1e12,
        r.card.scale.value / 1e9,
        r.card.vector.value / 1e9,
        r.card.memory.bandwidth.value / 1e9,
        r.card.streams.read.value * 100.0,
        r.card.streams.rewrite.value * 100.0,
        r.card.streams.append.value * 100.0
    );
    for row in r.rows.iter().chain(&r.full_beat.rows) {
        let c = &row.cost;
        println!(
            "{:<64} B {:>4} ({:>3} host)  {:>7.1} tok/s  pass {:>6.1} ms {:<6} mem {:>6.1} host {:>6.1} array {:>6.1} lanes {:>6.1} vector {:>6.1}  held {:>4.1}+{:>4.1} GB",
            row.label,
            c.batch,
            c.host_sequences,
            row.tokens_per_s,
            c.pass_s * 1e3,
            format!("{:?}", c.binds),
            c.memory_s * 1e3,
            c.host_s * 1e3,
            c.array_s * 1e3,
            c.scale_s * 1e3,
            c.vector_s * 1e3,
            c.held_bytes / 1e9,
            c.host_held_bytes / 1e9
        );
    }
    let dir = silicon.join("d5005");
    std::fs::create_dir_all(&dir).expect("benchmarks/silicon/d5005");
    let path = dir.join("roofline.json");
    std::fs::write(&path, serde_json::to_string_pretty(&r).expect("the roofline serializes") + "\n")
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("wrote {}", path.display());
}
