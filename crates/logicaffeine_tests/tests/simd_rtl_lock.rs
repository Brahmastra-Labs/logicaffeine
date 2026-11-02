//! simd_rtl_lock — the packed SIMD tile's EMITTED Verilog runs ≡ the lane-wise spec under iverilog.
//!
//! `simd_tile_lock` proves the SWAR add carry-isolated at the model level (SAT). This closes model→silicon:
//! `emit_simd_add_verilog` emits that SWAR form as a combinational module, and a real gate-level simulator
//! (iverilog) confirms it computes N independent scalar adds for every driven vector — for I8×8, I16×4, I32×2.
//! Skipped gracefully where iverilog is absent (the SAT proof already pins correctness).
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(simd_rtl_lock)'`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::simd::{
    emit_simd_add_testbench, emit_simd_add_verilog, eval_lanewise_add, LaneTy,
};

/// Bit patterns that stress lane-boundary carries and each lane's sign bit.
const SAMPLES: &[u64] = &[
    0,
    u64::MAX,
    0x0101_0101_0101_0101,
    0x8080_8080_8080_8080,
    0x7F7F_7F7F_7F7F_7F7F,
    0xFFFF_0000_FFFF_0000,
    0x1234_5678_9ABC_DEF0,
    0xDEAD_BEEF_CAFE_F00D,
];

#[test]
fn the_emitted_simd_add_matches_the_lanewise_spec_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join("logos_simd_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");

    for &ty in LaneTy::ALL {
        let name = format!("{ty:?}").to_lowercase(); // i8x8 / i16x4 / i32x2
        let n = SAMPLES.len() * SAMPLES.len();
        let (mut ah, mut bh, mut oh) = (String::new(), String::new(), String::new());
        for &a in SAMPLES {
            for &b in SAMPLES {
                ah.push_str(&format!("{a:016x}\n"));
                bh.push_str(&format!("{b:016x}\n"));
                oh.push_str(&format!("{:016x}\n", eval_lanewise_add(ty, a, b)));
            }
        }

        let rtl = emit_simd_add_verilog(&name, ty);
        let tb = emit_simd_add_testbench(&name, n);
        let srcv = dir.join(format!("simd_{name}.v"));
        let tbv = dir.join(format!("tb_simd_{name}.v"));
        let sim = dir.join(format!("sim_simd_{name}.vvp"));
        std::fs::write(&srcv, &rtl).unwrap();
        std::fs::write(&tbv, &tb).unwrap();
        std::fs::write(dir.join(format!("simd_{name}_a.hex")), &ah).unwrap();
        std::fs::write(dir.join(format!("simd_{name}_b.hex")), &bh).unwrap();
        std::fs::write(dir.join(format!("simd_{name}_o.hex")), &oh).unwrap();

        let build = std::process::Command::new("iverilog")
            .arg("-g2012")
            .arg("-o")
            .arg(&sim)
            .arg(&srcv)
            .arg(&tbv)
            .current_dir(&dir)
            .output()
            .expect("run iverilog");
        assert!(
            build.status.success(),
            "iverilog failed to compile simd_{name}: {}\n--- RTL ---\n{rtl}",
            String::from_utf8_lossy(&build.stderr)
        );

        let run = std::process::Command::new("vvp")
            .arg(&sim)
            .current_dir(&dir)
            .output()
            .expect("run vvp");
        let stdout = String::from_utf8_lossy(&run.stdout);
        assert!(
            stdout.contains("PASS:"),
            "simd_{name} RTL != lane-wise spec under iverilog: {stdout}\n--- RTL ---\n{rtl}"
        );
    }

    println!("SIMD RTL: the emitted SWAR add ≡ N lane-wise scalar adds under iverilog for I8×8, I16×4, I32×2");
}
