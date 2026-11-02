//! Write the ZCU104 sizing — every local checkpoint priced on the board at each PL operating point — to
//! `benchmarks/silicon/decode/zcu104_sizing.json`, and print it. Usage:
//!   cargo run -p logicaffeine-compile --example zcu104_sizing

use logicaffeine_compile::spatial::fpga_decode::{zcu104_sizing, Fit};

fn main() {
    let decode = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/decode");
    let s = zcu104_sizing(&decode).unwrap_or_else(|e| panic!("the sizing: {e}"));
    println!(
        "{}: {:.2} MB on chip, {:.2} GB DDR4, KV cache at {} tokens",
        s.board,
        s.on_chip_bytes / 1e6,
        s.ddr_bytes / 1e9,
        s.ctx
    );
    for p in &s.points {
        println!(
            "  point {:<16} PL {:>5.0} MHz ({:?}), {} HP ports: DDR read {:>5.1} GB/s ({:?}), on-chip read {:>7.1} GB/s, int8 {:.2} TOPS",
            p.name, p.pl_mhz, p.pl_provenance, p.hp_ports, p.ddr_read_gb_s, p.ddr_read_provenance, p.sram_read_gb_s, p.int8_tops
        );
    }
    for m in &s.models {
        println!(
            "{}  ({:.3} GB held, {:.3} GB a token{})",
            m.model,
            m.held_bytes / 1e9,
            (m.weight_bytes_per_token + m.state_bytes_per_token) / 1e9,
            if m.tied_head { ", tied head" } else { "" }
        );
        match &m.fit {
            Fit::Resident { rows } => {
                for r in rows {
                    println!(
                        "  floor {:>7.2} tok/s at {:<16} ({:?}, {:.4}% of its bytes on chip, {:?})",
                        r.floor_tok_s,
                        r.point,
                        r.binds,
                        r.on_chip_share * 100.0,
                        r.provenance
                    );
                }
            }
            Fit::NotResident { needed, capacity } => {
                println!("  does not fit: {:.2} GB held against the board's {:.2} GB", needed / 1e9, capacity / 1e9)
            }
        }
    }
    let out = decode.join("zcu104_sizing.json");
    let text = serde_json::to_string_pretty(&s).expect("the sizing serializes") + "\n";
    std::fs::write(&out, text).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    println!("wrote {}", out.display());
}
