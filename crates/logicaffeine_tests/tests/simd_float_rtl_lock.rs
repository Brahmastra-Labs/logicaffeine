//! simd_float_rtl_lock — the synthesizable f32 FPU is bit-exact to Rust `f32` under iverilog.
//!
//! `simd_tile_lock` proves the packed-float F32×2 *model* isolation-clean. This builds the SILICON: a
//! combinational `fp32_add` (ported from the proven f64 `fp_addsub`), driven by a real gate-level simulator
//! against Rust's own `f32` add over IEEE-special pairs + a large random battery (normals, subnormals, ties,
//! overflow, NaN/Inf). Finite results must match bit-for-bit; NaN results agree as canonical quiet-NaNs (IEEE
//! leaves the payload unspecified — the RTL emits 0x7FC00000, the oracle canonicalizes any NaN to it, so a real
//! finite/NaN divergence is still caught). Skipped gracefully where iverilog is absent.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(simd_float_rtl_lock)'`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::simd::{
    emit_fp32_scalar_add_testbench, emit_fp32_scalar_mul_testbench, emit_simd_add_testbench,
    emit_simd_fadd_f16x4_verilog, emit_simd_fadd_f32x2_verilog, emit_simd_fmul_f16x4_verilog,
    emit_simd_fmul_f32x2_verilog, eval_lanewise_fadd_f16x4, eval_lanewise_fmul_f16x4, f16_add,
    f16_mul, FP16_ADD_V, FP16_MUL_V, FP32_ADD_V, FP32_MUL_V,
};

const CANON_QNAN: u32 = 0x7FC0_0000;

/// The oracle: Rust `f32` add, with any NaN result canonicalized (IEEE payload is unspecified).
fn oracle_add(a: u32, b: u32) -> u32 {
    let r = f32::from_bits(a) + f32::from_bits(b);
    if r.is_nan() {
        CANON_QNAN
    } else {
        r.to_bits()
    }
}

/// The oracle: Rust `f32` multiply, NaN canonicalized.
fn oracle_mul(a: u32, b: u32) -> u32 {
    let r = f32::from_bits(a) * f32::from_bits(b);
    if r.is_nan() {
        CANON_QNAN
    } else {
        r.to_bits()
    }
}

/// IEEE-special f32 bit patterns: signed zeros/infinities, NaN, the normal/subnormal boundaries, ties.
fn specials() -> Vec<u32> {
    let mut v: Vec<u32> = [
        0.0f32,
        -0.0,
        1.0,
        -1.0,
        2.0,
        0.5,
        3.14159,
        -2.71828,
        123456.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        f32::MIN_POSITIVE, // smallest normal
        f32::MAX,
        f32::MIN,
    ]
    .iter()
    .map(|x| x.to_bits())
    .collect();
    v.extend([
        1u32,        // smallest subnormal
        0x007F_FFFF, // largest subnormal
        0x0000_0010,
        0x0040_0000, // mid subnormal
        0x3F80_0001, // 1 + ulp
        0x3F7F_FFFF, // 1 - ulp
        0x4000_0001, // 2 + ulp
        0x0080_0000, // smallest normal (bits)
        0x807F_FFFF, // negative largest subnormal
    ]);
    v
}

/// A deterministic LCG over full 32-bit patterns — arbitrary floats incl. subnormals/NaN/Inf by construction.
fn random_pairs(n: usize) -> Vec<(u32, u32)> {
    let mut s: u64 = 0xF32_ADD_5EED;
    let mut next = || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 32) as u32
    };
    (0..n).map(|_| (next(), next())).collect()
}

#[test]
fn fp32_add_rtl_is_bit_exact_to_rust_f32_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join("logos_fp32_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");

    // Build the vector set: every special pair (both orders) + a large random battery.
    let sp = specials();
    let mut vectors: Vec<(u32, u32)> = Vec::new();
    for &a in &sp {
        for &b in &sp {
            vectors.push((a, b));
        }
    }
    vectors.extend(random_pairs(50_000));

    let (mut ah, mut bh, mut oh) = (String::new(), String::new(), String::new());
    for &(a, b) in &vectors {
        ah.push_str(&format!("{a:08x}\n"));
        bh.push_str(&format!("{b:08x}\n"));
        oh.push_str(&format!("{:08x}\n", oracle_add(a, b)));
    }

    let tb = emit_fp32_scalar_add_testbench(vectors.len());
    let srcv = dir.join("fp32_add.v");
    let tbv = dir.join("tb_fp32_add.v");
    let sim = dir.join("sim_fp32_add.vvp");
    std::fs::write(&srcv, FP32_ADD_V).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    std::fs::write(dir.join("fp32_add_a.hex"), &ah).unwrap();
    std::fs::write(dir.join("fp32_add_b.hex"), &bh).unwrap();
    std::fs::write(dir.join("fp32_add_o.hex"), &oh).unwrap();

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
        "iverilog failed to compile fp32_add: {}",
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
        "fp32_add RTL != Rust f32 under iverilog:\n{stdout}"
    );
    println!(
        "f32 FPU: fp32_add bit-exact to Rust f32 over {} vectors under iverilog",
        vectors.len()
    );
}

#[test]
fn fp32_mul_rtl_is_bit_exact_to_rust_f32_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join("logos_fp32_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let sp = specials();
    let mut vectors: Vec<(u32, u32)> = Vec::new();
    for &a in &sp {
        for &b in &sp {
            vectors.push((a, b));
        }
    }
    vectors.extend(random_pairs(50_000));

    let (mut ah, mut bh, mut oh) = (String::new(), String::new(), String::new());
    for &(a, b) in &vectors {
        ah.push_str(&format!("{a:08x}\n"));
        bh.push_str(&format!("{b:08x}\n"));
        oh.push_str(&format!("{:08x}\n", oracle_mul(a, b)));
    }

    let tb = emit_fp32_scalar_mul_testbench(vectors.len());
    let srcv = dir.join("fp32_mul.v");
    let tbv = dir.join("tb_fp32_mul.v");
    let sim = dir.join("sim_fp32_mul.vvp");
    std::fs::write(&srcv, FP32_MUL_V).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    std::fs::write(dir.join("fp32_mul_a.hex"), &ah).unwrap();
    std::fs::write(dir.join("fp32_mul_b.hex"), &bh).unwrap();
    std::fs::write(dir.join("fp32_mul_o.hex"), &oh).unwrap();

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
        "iverilog failed to compile fp32_mul: {}",
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
        "fp32_mul RTL != Rust f32 under iverilog:\n{stdout}"
    );
    println!(
        "f32 FPU: fp32_mul bit-exact to Rust f32 over {} vectors under iverilog",
        vectors.len()
    );
}

/// Pack two f32-bit lanes into a 64-bit word (lane 0 low).
fn pack(lo: u32, hi: u32) -> u64 {
    ((hi as u64) << 32) | lo as u64
}

/// Deterministic random 64-bit operand pairs (each two arbitrary f32 lanes).
fn random_pairs64(n: usize) -> Vec<(u64, u64)> {
    let mut s: u64 = 0xF322_1A5E_5EED_2C0D;
    let mut next = || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        s
    };
    (0..n).map(|_| (next(), next())).collect()
}

/// A small special set for the 4-fold lane product (kept modest so 10⁴ special packings stay fast).
fn small_specials() -> Vec<u32> {
    [
        0.0f32,
        -0.0,
        1.0,
        -1.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        f32::MIN_POSITIVE,
        3.14159,
        123456.0,
    ]
    .iter()
    .map(|x| x.to_bits())
    .collect()
}

/// Drive a packed F32×2 tile (`simd_{name}`, compiled with its `unit` FPU) against a per-lane oracle.
fn run_packed_f32x2(name: &str, unit_v: &str, wrapper_v: &str, oracle: impl Fn(u64, u64) -> u64) {
    let dir = std::env::temp_dir().join("logos_fp32_iverilog");
    std::fs::create_dir_all(&dir).unwrap();

    let sp = small_specials();
    let mut vectors: Vec<(u64, u64)> = Vec::new();
    for &x0 in &sp {
        for &y0 in &sp {
            for &x1 in &sp {
                for &y1 in &sp {
                    vectors.push((pack(x0, x1), pack(y0, y1)));
                }
            }
        }
    }
    vectors.extend(random_pairs64(50_000));

    let (mut ah, mut bh, mut oh) = (String::new(), String::new(), String::new());
    for &(a, b) in &vectors {
        ah.push_str(&format!("{a:016x}\n"));
        bh.push_str(&format!("{b:016x}\n"));
        oh.push_str(&format!("{:016x}\n", oracle(a, b)));
    }

    let tb = emit_simd_add_testbench(name, vectors.len());
    let unitv = dir.join(format!("unit_{name}.v"));
    let srcv = dir.join(format!("simd_{name}.v"));
    let tbv = dir.join(format!("tb_simd_{name}.v"));
    let sim = dir.join(format!("sim_simd_{name}.vvp"));
    std::fs::write(&unitv, unit_v).unwrap();
    std::fs::write(&srcv, wrapper_v).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    std::fs::write(dir.join(format!("simd_{name}_a.hex")), &ah).unwrap();
    std::fs::write(dir.join(format!("simd_{name}_b.hex")), &bh).unwrap();
    std::fs::write(dir.join(format!("simd_{name}_o.hex")), &oh).unwrap();

    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(&unitv)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed to compile simd_{name}: {}",
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
        "simd_{name} RTL != lane-wise f32 under iverilog:\n{stdout}"
    );
    println!(
        "float SIMD RTL: packed {name} ≡ two independent f32 lanes over {} vectors under iverilog",
        vectors.len()
    );
}

/// Per-lane NaN-canonicalizing packed oracles (the RTL emits a canonical qNaN per lane).
fn oracle_fadd_f32x2(a: u64, b: u64) -> u64 {
    pack(
        oracle_add(a as u32, b as u32),
        oracle_add((a >> 32) as u32, (b >> 32) as u32),
    )
}
fn oracle_fmul_f32x2(a: u64, b: u64) -> u64 {
    pack(
        oracle_mul(a as u32, b as u32),
        oracle_mul((a >> 32) as u32, (b >> 32) as u32),
    )
}

#[test]
fn packed_f32x2_add_rtl_matches_two_independent_f32_lanes_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    run_packed_f32x2(
        "fadd_f32x2",
        FP32_ADD_V,
        &emit_simd_fadd_f32x2_verilog(),
        oracle_fadd_f32x2,
    );
}

#[test]
fn packed_f32x2_mul_rtl_matches_two_independent_f32_lanes_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    run_packed_f32x2(
        "fmul_f32x2",
        FP32_MUL_V,
        &emit_simd_fmul_f32x2_verilog(),
        oracle_fmul_f32x2,
    );
}

// ── f16 ─────────────────────────────────────────────────────────────────────────────────────────────

/// Canonicalize an f16 NaN to `0x7E00` (the Verilog emits a positive qNaN; inf−inf can yield a signed NaN).
fn canon_f16(bits: u16) -> u16 {
    if (bits >> 10) & 0x1F == 0x1F && (bits & 0x3FF) != 0 {
        0x7E00
    } else {
        bits
    }
}
fn oracle_f16_add(a: u16, b: u16) -> u16 {
    canon_f16(f16_add(a, b))
}
fn oracle_f16_mul(a: u16, b: u16) -> u16 {
    canon_f16(f16_mul(a, b))
}

fn fp16_scalar_tb(name: &str, inst: &str, n: usize) -> String {
    format!(
        r#"`timescale 1ns/1ps
module tb_fp16_{name};
  reg [15:0] a, b; wire [15:0] y;
  reg [15:0] ia [0:{last}]; reg [15:0] ib [0:{last}]; reg [15:0] oo [0:{last}];
  integer i, errors = 0;
  {inst}
  initial begin
    $readmemh("fp16_{name}_a.hex", ia);
    $readmemh("fp16_{name}_b.hex", ib);
    $readmemh("fp16_{name}_o.hex", oo);
    for (i = 0; i < {n}; i = i + 1) begin
      a = ia[i]; b = ib[i]; #1;
      if (y !== oo[i]) begin errors = errors + 1; if (errors <= 20) $display("FAIL %0d: a=%h b=%h got=%h exp=%h", i, a, b, y, oo[i]); end
    end
    if (errors == 0) $display("PASS: {n} fp16 {name} bit-exact to the f16 reference");
    else $display("FAIL: %0d fp16 {name} mismatches", errors);
    $finish;
  end
endmodule
"#,
        last = n - 1,
    )
}

/// Sweep EVERY 16-bit `a` against a fixed `b` set (so all 65536 f16 values are exercised) + random pairs.
fn f16_scalar_vectors() -> Vec<(u16, u16)> {
    let bset: [u16; 4] = [0x3C00, 0x0001, 0x7BFF, 0xBC00];
    let mut v: Vec<(u16, u16)> = Vec::with_capacity(65536 * bset.len() + 30_000);
    for a in 0u32..=0xFFFF {
        for &b in &bset {
            v.push((a as u16, b));
        }
    }
    let mut s: u64 = 0xF16_5EED_C0DE_1234;
    let mut next = || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 40) as u16
    };
    for _ in 0..30_000 {
        v.push((next(), next()));
    }
    v
}

fn run_scalar_f16(name: &str, unit_v: &str, inst: &str, oracle: impl Fn(u16, u16) -> u16) {
    let dir = std::env::temp_dir().join("logos_fp16_iverilog");
    std::fs::create_dir_all(&dir).unwrap();
    let vectors = f16_scalar_vectors();
    let (mut ah, mut bh, mut oh) = (String::new(), String::new(), String::new());
    for &(a, b) in &vectors {
        ah.push_str(&format!("{a:04x}\n"));
        bh.push_str(&format!("{b:04x}\n"));
        oh.push_str(&format!("{:04x}\n", oracle(a, b)));
    }
    let tb = fp16_scalar_tb(name, inst, vectors.len());
    let unitv = dir.join(format!("fp16_{name}_unit.v"));
    let tbv = dir.join(format!("tb_fp16_{name}.v"));
    let sim = dir.join(format!("sim_fp16_{name}.vvp"));
    std::fs::write(&unitv, unit_v).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    std::fs::write(dir.join(format!("fp16_{name}_a.hex")), &ah).unwrap();
    std::fs::write(dir.join(format!("fp16_{name}_b.hex")), &bh).unwrap();
    std::fs::write(dir.join(format!("fp16_{name}_o.hex")), &oh).unwrap();
    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(&unitv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed fp16_{name}: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "fp16_{name} RTL != f16 reference: {stdout}"
    );
    println!(
        "f16 FPU: fp16_{name} bit-exact to the f16 reference over {} vectors under iverilog",
        vectors.len()
    );
}

#[test]
fn fp16_add_rtl_is_bit_exact_to_the_f16_reference_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    run_scalar_f16(
        "add",
        FP16_ADD_V,
        "fp16_add dut(.a(a), .b_in(b), .sub(1'b0), .y(y));",
        oracle_f16_add,
    );
}

#[test]
fn fp16_mul_rtl_is_bit_exact_to_the_f16_reference_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    run_scalar_f16(
        "mul",
        FP16_MUL_V,
        "fp16_mul dut(.a(a), .b(b), .y(y));",
        oracle_f16_mul,
    );
}

fn small_specials16() -> Vec<u16> {
    vec![
        0x0000, 0x8000, 0x3C00, 0xBC00, 0x7C00, 0xFC00, 0x7E00, 0x0400, 0x0001, 0x4248,
    ]
}

fn oracle_fadd_f16x4(a: u64, b: u64) -> u64 {
    let l = |sh: u32| oracle_f16_add((a >> sh) as u16, (b >> sh) as u16) as u64;
    l(0) | (l(16) << 16) | (l(32) << 32) | (l(48) << 48)
}
fn oracle_fmul_f16x4(a: u64, b: u64) -> u64 {
    let l = |sh: u32| oracle_f16_mul((a >> sh) as u16, (b >> sh) as u16) as u64;
    l(0) | (l(16) << 16) | (l(32) << 32) | (l(48) << 48)
}

fn run_packed_f16x4(name: &str, unit_v: &str, wrapper_v: &str, oracle: impl Fn(u64, u64) -> u64) {
    let dir = std::env::temp_dir().join("logos_fp16_iverilog");
    std::fs::create_dir_all(&dir).unwrap();
    let sp = small_specials16();
    let mut vectors: Vec<(u64, u64)> = Vec::new();
    for (i, &x) in sp.iter().enumerate() {
        for &y in &sp {
            let a = (x as u64)
                | ((y as u64) << 16)
                | ((sp[i % sp.len()] as u64) << 32)
                | ((y as u64) << 48);
            let b = (y as u64)
                | ((x as u64) << 16)
                | ((x as u64) << 32)
                | ((sp[(i + 3) % sp.len()] as u64) << 48);
            vectors.push((a, b));
        }
    }
    vectors.extend(random_pairs64(50_000));

    let (mut ah, mut bh, mut oh) = (String::new(), String::new(), String::new());
    for &(a, b) in &vectors {
        ah.push_str(&format!("{a:016x}\n"));
        bh.push_str(&format!("{b:016x}\n"));
        oh.push_str(&format!("{:016x}\n", oracle(a, b)));
    }
    let tb = emit_simd_add_testbench(name, vectors.len());
    let unitv = dir.join(format!("unit_{name}.v"));
    let srcv = dir.join(format!("simd_{name}.v"));
    let tbv = dir.join(format!("tb_simd_{name}.v"));
    let sim = dir.join(format!("sim_simd_{name}.vvp"));
    std::fs::write(&unitv, unit_v).unwrap();
    std::fs::write(&srcv, wrapper_v).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    std::fs::write(dir.join(format!("simd_{name}_a.hex")), &ah).unwrap();
    std::fs::write(dir.join(format!("simd_{name}_b.hex")), &bh).unwrap();
    std::fs::write(dir.join(format!("simd_{name}_o.hex")), &oh).unwrap();
    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(&unitv)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed simd_{name}: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "simd_{name} RTL != lane-wise f16 under iverilog:\n{stdout}"
    );
    println!(
        "float SIMD RTL: packed {name} ≡ four independent f16 lanes over {} vectors under iverilog",
        vectors.len()
    );
}

#[test]
fn packed_f16x4_add_rtl_matches_four_independent_f16_lanes_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    // sanity: the model spec agrees with the canonicalizing oracle on a non-NaN case
    assert_eq!(
        eval_lanewise_fadd_f16x4(0x3C00_3C00_3C00_3C00, 0x3C00_3C00_3C00_3C00),
        0x4000_4000_4000_4000
    );
    run_packed_f16x4(
        "fadd_f16x4",
        FP16_ADD_V,
        &emit_simd_fadd_f16x4_verilog(),
        oracle_fadd_f16x4,
    );
}

#[test]
fn packed_f16x4_mul_rtl_matches_four_independent_f16_lanes_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    assert_eq!(
        eval_lanewise_fmul_f16x4(0x4000_4000_4000_4000, 0x3C00_3C00_3C00_3C00),
        0x4000_4000_4000_4000
    );
    run_packed_f16x4(
        "fmul_f16x4",
        FP16_MUL_V,
        &emit_simd_fmul_f16x4_verilog(),
        oracle_fmul_f16x4,
    );
}
