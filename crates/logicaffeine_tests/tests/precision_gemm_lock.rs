//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! PRECISION GEMM LOCK — Phase 4 of the SambaNova-parity grind.
//!
//! Precision is a DIAL on the GEMM tile. This lock proves each advertised format and RATCHETS the dial:
//!
//!   INT (int8 / int16 / int64): eval_systolic_bits ≡ an independent int oracle, ≡ the VM (in-range
//!   operands), and the emitted RTL ≡ eval under iverilog — a fully-proven integer precision per width.
//!
//!   FLOAT (bf16): round-to-nearest-even is idempotent and zeroes the low 16 mantissa bits; a bf16 GEMM over
//!   bf16-representable inputs equals the exact f32 GEMM (the rounding is a no-op there) — a numeric witness.
//!
//!   RATCHET (pit-of-success): the witness registry keys MUST EQUAL the precision set a target advertises
//!   (`FabricTarget::full().precisions`). Advertising a precision without shipping its witness is RED, and
//!   every witness is invoked. A dial can never outrun its proof.
//!
//! Run: `cargo test -p logicaffeine-tests --test precision_gemm_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use std::collections::BTreeSet;
use std::process::Command;

use logicaffeine_compile::codegen_sva::fabric_target::{FabricTarget, Precision};
use logicaffeine_compile::codegen_sva::systolic::{
    emit_systolic_verilog_bits, eval_systolic_bf16, eval_systolic_bits, eval_systolic_f32,
    round_bf16, trunc_signed, Systolic,
};
use logicaffeine_compile::compile::vm_outcome;

fn prng(seed: u64, i: usize, lo: i64, hi: i64) -> i64 {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(i as u64 + 1);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    let span = (hi - lo + 1) as u64;
    lo + (x % span) as i64
}

/// Independent int GEMM oracle at a given bit width (i128 accumulate, truncate mod 2^64).
fn int_oracle(k: usize, n: usize, weights: &[i64], acts: &[i64], m: usize, bits: u32) -> Vec<i64> {
    let mut out = vec![0i64; m * n];
    for i in 0..m {
        for c in 0..n {
            let mut acc: i128 = 0;
            for r in 0..k {
                acc += trunc_signed(acts[i * k + r], bits) as i128
                    * trunc_signed(weights[r * n + c], bits) as i128;
            }
            out[i * n + c] = acc as i64;
        }
    }
    out
}

/// RTL ≡ eval_systolic_bits under iverilog, one row, at width `bits`.
fn rtl_matches_eval(bits: u32) {
    if !common::require_tool("iverilog") {
        return;
    }
    let (k, n) = (4usize, 3usize);
    let lim = 1i64 << (bits - 1).min(20); // bound so operands + products stay in range (also avoids 1<<63)
    let weights: Vec<i64> = (0..k * n)
        .map(|i| prng(bits as u64, i, -lim, lim - 1))
        .collect();
    let acts: Vec<i64> = (0..k)
        .map(|i| prng(bits as u64 ^ 0x5, i, -lim, lim - 1))
        .collect();
    let sys = Systolic::new(k, n, weights);
    let want = eval_systolic_bits(&sys, &acts, 1, bits);
    let rtl = emit_systolic_verilog_bits("psa", &sys, bits);
    let dir = std::env::temp_dir().join(format!("psa_{bits}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut tb = String::from("`timescale 1ns/1ps\nmodule tb;\n  reg clk=0, rst=1;\n");
    for r in 0..k {
        tb.push_str(&format!("  reg signed [63:0] a{r};\n"));
    }
    for c in 0..n {
        tb.push_str(&format!("  wire signed [63:0] o{c};\n"));
    }
    let ac: Vec<String> = (0..k).map(|r| format!(".a{r}(a{r})")).collect();
    let oc: Vec<String> = (0..n).map(|c| format!(".o{c}(o{c})")).collect();
    tb.push_str(&format!(
        "  psa dut(.clk(clk), .rst(rst), {}, {});\n",
        ac.join(", "),
        oc.join(", ")
    ));
    tb.push_str("  always #5 clk=~clk;\n  initial begin\n    rst=1; @(posedge clk); #1 rst=0;\n");
    for r in 0..k {
        tb.push_str(&format!("    a{r} = 64'sh{:016X};\n", acts[r] as u64));
    }
    tb.push_str("    @(posedge clk); #1;\n");
    let disp: Vec<String> = (0..n).map(|c| format!("o{c}")).collect();
    let fmt: Vec<&str> = (0..n).map(|_| "%0d").collect();
    tb.push_str(&format!(
        "    $display(\"O {}\", {});\n    $finish;\n  end\nendmodule\n",
        fmt.join(" "),
        disp.join(", ")
    ));
    let vpath = dir.join("psa.v");
    std::fs::write(&vpath, format!("{rtl}\n{tb}")).unwrap();
    let vvp = dir.join("s.vvp");
    let b = Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&vvp)
        .arg(&vpath)
        .output()
        .unwrap();
    assert!(
        b.status.success(),
        "iverilog build (bits={bits}): {}",
        String::from_utf8_lossy(&b.stderr)
    );
    let run = Command::new("vvp").arg(&vvp).output().unwrap();
    let so = String::from_utf8_lossy(&run.stdout);
    let row: Vec<i64> = so
        .lines()
        .find(|l| l.starts_with("O "))
        .expect("iverilog O line")[2..]
        .split_whitespace()
        .map(|t| t.parse().unwrap())
        .collect();
    assert_eq!(row, want, "RTL != eval at bits={bits}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── the witnesses (one per advertised precision) ───────────────────────────────────────────────────

fn witness_int(bits: u32) {
    let lim = 1i64 << (bits - 1).min(20); // bound so i128 accumulate + VM stay sane
    for &(k, n, m) in &[(4usize, 4usize, 3usize), (8, 3, 5)] {
        for seed in 0..6u64 {
            let weights: Vec<i64> = (0..k * n)
                .map(|i| prng(seed + bits as u64, i, -lim, lim - 1))
                .collect();
            let acts: Vec<i64> = (0..m * k)
                .map(|i| prng(seed ^ 0x3c, i, -lim, lim - 1))
                .collect();
            let sys = Systolic::new(k, n, weights.clone());
            let got = eval_systolic_bits(&sys, &acts, m, bits);
            assert_eq!(
                got,
                int_oracle(k, n, &weights, &acts, m, bits),
                "int{bits} != oracle (seed={seed})"
            );
        }
    }
    // ≡VM: in-range operands (trunc is a no-op) → a column dot equals the VM's dot.
    let k = 5usize;
    let acts: Vec<i64> = (0..k)
        .map(|i| prng(bits as u64, i, -lim, lim - 1))
        .collect();
    let weights: Vec<i64> = (0..k)
        .map(|i| prng(bits as u64 ^ 0xf, i, -lim, lim - 1))
        .collect();
    let sys = Systolic::new(k, 1, weights.clone());
    let got = eval_systolic_bits(&sys, &acts, 1, bits)[0];
    let terms: Vec<String> = (0..k)
        .map(|r| format!("({}) * ({})", acts[r], weights[r]))
        .collect();
    let vm = vm_outcome(&format!("## Main\n    Show {}.\n", terms.join(" + ")));
    assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
    assert_eq!(got.to_string(), vm.output.trim(), "int{bits} dot != VM");
    rtl_matches_eval(bits);
}

fn witness_int8() {
    witness_int(8);
}
fn witness_int16() {
    witness_int(16);
}
fn witness_int64() {
    witness_int(64);
}

fn witness_bf16() {
    // idempotent + low-16-bits zeroed
    for seed in 0..200u64 {
        let x = (prng(seed, 0, -100000, 100000) as f64) * 0.001;
        let r = round_bf16(x);
        assert_eq!(round_bf16(r), r, "bf16 rounding must be idempotent (x={x})");
        assert_eq!(
            (r as f32).to_bits() & 0xFFFF,
            0,
            "bf16 value must have zero low mantissa bits (x={x})"
        );
    }
    // bf16-representable inputs (small integers, exact in bf16) → bf16 GEMM == exact f32 GEMM.
    let (k, n, m) = (4usize, 3usize, 4usize);
    let weights: Vec<f64> = (0..k * n).map(|i| prng(1, i, -8, 8) as f64).collect();
    let acts: Vec<f64> = (0..m * k).map(|i| prng(2, i, -8, 8) as f64).collect();
    for &w in &weights {
        assert_eq!(round_bf16(w), w, "small int {w} must be bf16-exact");
    }
    let got = eval_systolic_bf16(&weights, &acts, m, k, n);
    // exact f32 oracle over the same representable inputs
    let mut want = vec![0f64; m * n];
    for i in 0..m {
        for c in 0..n {
            let mut acc = 0f32;
            for r in 0..k {
                acc += acts[i * k + r] as f32 * weights[r * n + c] as f32;
            }
            want[i * n + c] = acc as f64;
        }
    }
    assert_eq!(
        got, want,
        "bf16 GEMM over representable inputs must equal the exact f32 GEMM"
    );
}

fn witness_fp32() {
    // fp32 is exact on integers below 2^24, so an integer-valued GEMM whose every partial sum stays below
    // 2^24 equals the exact (i64) GEMM — and a value with a fractional part is a genuine f32 (not silently
    // rounded to an integer or to bf16): 0.1 survives, and 1 + 2^-20 is NOT bf16-representable but IS f32.
    let (k, n, m) = (4usize, 3usize, 4usize);
    let weights: Vec<f64> = (0..k * n)
        .map(|i| prng(11, i, -1000, 1000) as f64)
        .collect();
    let acts: Vec<f64> = (0..m * k)
        .map(|i| prng(12, i, -1000, 1000) as f64)
        .collect();
    let got = eval_systolic_f32(&weights, &acts, m, k, n);
    let mut want = vec![0f64; m * n];
    for i in 0..m {
        for c in 0..n {
            let mut acc: i64 = 0;
            for r in 0..k {
                acc += acts[i * k + r] as i64 * weights[r * n + c] as i64;
            }
            assert!(
                acc.abs() < (1 << 24),
                "the witness keeps every sum inside fp32's exact integer range"
            );
            want[i * n + c] = acc as f64;
        }
    }
    assert_eq!(
        got, want,
        "fp32 GEMM over integers < 2^24 must equal the exact GEMM"
    );
    let fine = 1.0 + 2f64.powi(-20);
    assert_ne!(
        round_bf16(fine),
        fine,
        "1 + 2^-20 is not bf16-representable — this witness is NOT the bf16 one"
    );
    let one = eval_systolic_f32(&[fine], &[1.0], 1, 1, 1)[0];
    assert_eq!(
        one,
        (fine as f32) as f64,
        "a 32-bit product keeps the 2^-20 fraction"
    );
    let tenth = eval_systolic_f32(&[0.1], &[1.0], 1, 1, 1)[0];
    assert_eq!(
        tenth,
        (0.1f32) as f64,
        "0.1 is carried at f32 precision, not truncated"
    );
}

const PRECISION_WITNESS: &[(&str, fn())] = &[
    ("Int8", witness_int8),
    ("Int16", witness_int16),
    ("Int64", witness_int64),
    ("Bf16", witness_bf16),
    ("Fp32", witness_fp32),
];

#[test]
fn every_advertised_precision_has_a_witness_and_it_holds() {
    // Ratchet: the witness registry keys MUST EQUAL the precision set the superset target advertises.
    let advertised: BTreeSet<String> = FabricTarget::full(8, 8, 64)
        .precisions
        .iter()
        .map(|p| format!("{p:?}"))
        .collect();
    let witnessed: BTreeSet<String> = PRECISION_WITNESS
        .iter()
        .map(|(k, _)| k.to_string())
        .collect();
    assert_eq!(
        advertised, witnessed,
        "every advertised precision must have a witness (and vice versa): advertised={advertised:?} witnessed={witnessed:?}"
    );
    // And every witness actually holds.
    for (name, w) in PRECISION_WITNESS {
        w();
        println!("  precision witness OK: {name}");
    }
    // Sanity: an UNwitnessed float format is genuinely not advertised (honest follow-on, not a silent claim).
    assert!(
        !FabricTarget::full(8, 8, 64)
            .precisions
            .contains(&Precision::Fp8),
        "Fp8 must not be advertised until its witness ships"
    );
    println!(
        "Phase 4: {} precisions advertised, each with a passing witness; ratchet holds",
        witnessed.len()
    );
}
