//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! WEIGHT-STATIONARY GEMM LOCK — Phase 1 of the SambaNova-parity grind.
//!
//! The reduction lane (gemm_fabric_lock) is a 1-D MAC: one output per operand pass, weights re-fetched. A
//! real accelerator array keeps weights STATIONARY and produces a whole output row in parallel, reusing each
//! weight across the activation stream. This lock proves the weight-stationary tile
//! (`codegen_sva::systolic`) is correct at every hop, the same multi-tier way gemm_fabric_lock proves the
//! reduction MAC:
//!
//!   A. eval_systolic ≡ an INDEPENDENT Rust GEMM oracle (int8), over a battery of shapes + weights.
//!   B. eval_systolic ≡ the VM — a single tile column's dot equals a LOGOS program's dot on the VM (int8
//!      range, so int8 truncation is a no-op and the tile is byte-identical to the integer GEMM ≡ VM).
//!   C. PLACED PE ≡ DATAPATH by our own SAT: the PE `psum + a·w` placed-and-routed onto the tile array is
//!      proven EQUAL to the abstract datapath for ALL int8 inputs (bit-blasted, refuted) — one symbolic-
//!      weight proof covers every PE. Plus a not-vacuous guard.
//!   D. RTL ≡ eval under iverilog: the emitted array, driven a row per cycle, reads back byte-identical to
//!      eval_systolic (iverilog-gated; skips loud where absent).
//!   E. REUSE DOMINANCE over the reduction lane: N outputs/cycle (not 1) and each weight read once and
//!      reused across all M rows (not re-fetched M·N·K times) — the operand-reuse that is the array's point.
//!
//! Run: `cargo test -p logicaffeine-tests --test weight_stationary_gemm_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use std::process::Command;

use logicaffeine_compile::codegen_sva::bitblast::lower_bool;
use logicaffeine_compile::codegen_sva::fabric_grid::{grid_output_to_bounded, route_loop_body};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    fabric_to_bounded, AluOp, FabricOp, LoopCell, Src,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_compile::codegen_sva::systolic::{emit_systolic_verilog, eval_systolic, Systolic};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

// ── helpers ──────────────────────────────────────────────────────────────────────────────────────

/// Independent Rust GEMM oracle (int8 operands, wrapping i64) — deliberately written differently from
/// `eval_systolic` (accumulate order + explicit i128 guard) so agreement is not a shared-bug artifact.
fn gemm_oracle(k: usize, n: usize, weights: &[i64], acts: &[i64], m: usize) -> Vec<i64> {
    let mut out = vec![0i64; m * n];
    for i in 0..m {
        for c in 0..n {
            let mut acc: i128 = 0;
            for r in 0..k {
                let a = (acts[i * k + r] as i8) as i128;
                let w = (weights[r * n + c] as i8) as i128;
                acc += a * w;
            }
            out[i * n + c] = acc as i64; // truncate mod 2^64, matching wrapping i64
        }
    }
    out
}

/// A tiny deterministic pseudo-random int stream in a chosen range (no rand dep; varies by seed+index).
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

// ── A. eval_systolic ≡ independent Rust oracle (concrete correctness, many shapes) ─────────────────

#[test]
fn weight_stationary_matches_reference_gemm() {
    let mut checked = 0usize;
    for &(k, n, m) in &[
        (1usize, 1usize, 1usize),
        (2, 3, 4),
        (4, 4, 5),
        (8, 2, 6),
        (3, 8, 3),
        (16, 4, 2),
    ] {
        for seed in 0..8u64 {
            let weights: Vec<i64> = (0..k * n).map(|i| prng(seed, i, -128, 127)).collect();
            let acts: Vec<i64> = (0..m * k)
                .map(|i| prng(seed ^ 0x9e37, i, -128, 127))
                .collect();
            let sys = Systolic::new(k, n, weights.clone());
            let got = eval_systolic(&sys, &acts, m);
            let want = gemm_oracle(k, n, &weights, &acts, m);
            assert_eq!(
                got, want,
                "weight-stationary {k}x{n} (m={m}, seed={seed}) diverged from the oracle"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 40,
        "the correctness battery must be genuinely large (got {checked})"
    );
    println!("A: eval_systolic ≡ independent int8 GEMM oracle over {checked} shape×seed cases");
}

// ── B. eval_systolic ≡ the VM (int8-range operands → byte-identical to the integer GEMM ≡ VM) ───────

#[test]
fn weight_stationary_dot_matches_the_vm() {
    // A single tile column (N=1) is a K-deep dot. Compute it on the VM from a LOGOS program with the SAME
    // int8-range operands baked in, and require the tile to match — tying the array to the reference VM.
    let k = 6usize;
    for seed in 0..12u64 {
        let acts: Vec<i64> = (0..k).map(|i| prng(seed, i, -120, 120)).collect();
        let weights: Vec<i64> = (0..k).map(|i| prng(seed ^ 0x1234, i, -120, 120)).collect();
        let sys = Systolic::new(k, 1, weights.clone());
        let tile = eval_systolic(&sys, &acts, 1)[0];

        // LOGOS dot with the operands as literals → Show the sum. (int8 range ⇒ no truncation, ≡ i64 GEMM.)
        let terms: Vec<String> = (0..k)
            .map(|r| format!("({}) * ({})", acts[r], weights[r]))
            .collect();
        let src = format!("## Main\n    Show {}.\n", terms.join(" + "));
        let out = vm_outcome(&src);
        assert!(
            out.error.is_none(),
            "VM error on the dot program: {:?}",
            out.error
        );
        let vm = out.output.trim().to_string();
        assert_eq!(
            tile.to_string(),
            vm,
            "systolic dot {tile} != VM {vm} (seed={seed}, acts={acts:?})"
        );
    }
    println!("B: eval_systolic column ≡ the VM's dot over 12 int8-range seeds");
}

// ── C. PLACED PE ≡ abstract datapath, by our own SAT (all int8 inputs) ─────────────────────────────

/// `LoopCell::Arith` and `FabricOp` share `{op, a, b}` — direct transcription for the placer.
fn to_cells(body: &[FabricOp]) -> Vec<LoopCell> {
    body.iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect()
}

/// The SYMBOLIC PE: inputs In0=activation, In1=incoming psum, In2=weight. Cells: Mul(In0,In2)→product,
/// Add(In1,product)→psum'. One placement proof over all int8 (a,w,psum) covers EVERY baked-weight PE.
fn pe_body() -> Vec<FabricOp> {
    vec![
        FabricOp {
            op: AluOp::Mul,
            a: Src::In(0),
            b: Src::In(2),
        }, // Val 0 = a·w
        FabricOp {
            op: AluOp::Add,
            a: Src::In(1),
            b: Src::Val(0),
        }, // Val 1 = psum + a·w
    ]
}

fn pe_placement_certified(body: &[FabricOp], result: usize, w: u32) -> bool {
    let cells = to_cells(body);
    let g = route_loop_body(&cells, 3, &[Src::Val(result)]);
    let grid = grid_output_to_bounded(&g, 0, w, &|k| format!("in{k}"));
    let datapath = fabric_to_bounded(body, result, w);
    let claim = BoundedExpr::Eq(Box::new(grid), Box::new(datapath));
    let p =
        lower_bool(&claim).expect("the PE placement-equivalence claim lowers to a bounded formula");
    prove_unsat(&ProofExpr::Not(Box::new(p))) == UnsatOutcome::Refuted
}

#[test]
fn weight_stationary_pe_placement_is_certified() {
    let body = pe_body();
    assert!(
        pe_placement_certified(&body, 1, 8),
        "the placed weight-stationary PE (psum + a·w) is NOT certified ≡ its abstract datapath for all int8"
    );
    // Not vacuous: the PE grid must NOT certify as a DIFFERENT datapath (psum - a·w).
    let wrong = vec![
        FabricOp {
            op: AluOp::Mul,
            a: Src::In(0),
            b: Src::In(2),
        },
        FabricOp {
            op: AluOp::Sub,
            a: Src::In(1),
            b: Src::Val(0),
        },
    ];
    let cells = to_cells(&pe_body());
    let g = route_loop_body(&cells, 3, &[Src::Val(1)]);
    let grid = grid_output_to_bounded(&g, 0, 8, &|k| format!("in{k}"));
    let wrong_dp = fabric_to_bounded(&wrong, 1, 8);
    let claim = BoundedExpr::Eq(Box::new(grid), Box::new(wrong_dp));
    let p = lower_bool(&claim).expect("claim lowers");
    assert_ne!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "a PE placed for `psum + a·w` must NOT certify as `psum - a·w` — the SAT check would be vacuous"
    );
    println!("C: the weight-stationary PE placement is certified ≡ datapath by SAT (all int8), non-vacuously");
}

// ── D. emitted RTL ≡ eval_systolic under iverilog ──────────────────────────────────────────────────

#[test]
fn weight_stationary_rtl_matches_eval_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (k, n, m) = (4usize, 3usize, 5usize);
    let weights: Vec<i64> = (0..k * n).map(|i| prng(7, i, -128, 127)).collect();
    let acts: Vec<i64> = (0..m * k).map(|i| prng(11, i, -128, 127)).collect();
    let sys = Systolic::new(k, n, weights);
    let want = eval_systolic(&sys, &acts, m);
    let rtl = emit_systolic_verilog("wsa", &sys);

    let dir = std::env::temp_dir().join(format!("wsa_iverilog_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Testbench: drive one activation row per cycle; capture each registered output row (1-cycle latency).
    let mut tb = String::new();
    tb.push_str("`timescale 1ns/1ps\nmodule tb;\n  reg clk=0, rst=1;\n");
    for r in 0..k {
        tb.push_str(&format!("  reg signed [63:0] a{r};\n"));
    }
    for c in 0..n {
        tb.push_str(&format!("  wire signed [63:0] o{c};\n"));
    }
    let aconn: Vec<String> = (0..k).map(|r| format!(".a{r}(a{r})")).collect();
    let oconn: Vec<String> = (0..n).map(|c| format!(".o{c}(o{c})")).collect();
    tb.push_str(&format!(
        "  wsa dut(.clk(clk), .rst(rst), {}, {});\n",
        aconn.join(", "),
        oconn.join(", ")
    ));
    tb.push_str("  always #5 clk=~clk;\n  integer i;\n  initial begin\n");
    tb.push_str("    rst=1; @(posedge clk); #1 rst=0;\n");
    for i in 0..m {
        for r in 0..k {
            // two's-complement hex so negative operands are legal Verilog (`64'sd-42` is malformed).
            tb.push_str(&format!(
                "    a{r} = 64'sh{:016X};\n",
                acts[i * k + r] as u64
            ));
        }
        tb.push_str("    @(posedge clk); #1;\n"); // apply row i on this edge; outputs valid next edge
                                                  // after the edge the registered outputs hold row i's result
        let disp: Vec<String> = (0..n).map(|c| format!("o{c}")).collect();
        let fmt: Vec<&str> = (0..n).map(|_| "%0d").collect();
        tb.push_str(&format!(
            "    $display(\"ROW {}\", {});\n",
            fmt.join(" "),
            disp.join(", ")
        ));
        let _ = i;
    }
    tb.push_str("    $finish;\n  end\nendmodule\n");

    let vpath = dir.join("wsa.v");
    let tbpath = dir.join("tb.v");
    std::fs::write(&vpath, format!("{rtl}\n{tb}")).unwrap();
    std::fs::write(&tbpath, "").unwrap();
    let vvp = dir.join("sim.vvp");
    let build = Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&vvp)
        .arg(&vpath)
        .output()
        .expect("iverilog runs");
    assert!(
        build.status.success(),
        "iverilog build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new("vvp").arg(&vvp).output().expect("vvp runs");
    let stdout = String::from_utf8_lossy(&run.stdout);
    // Parse "ROW o0 o1 ..." lines; the i-th ROW line is row i's registered output.
    let rows: Vec<Vec<i64>> = stdout
        .lines()
        .filter(|l| l.starts_with("ROW "))
        .map(|l| {
            l[4..]
                .split_whitespace()
                .map(|t| t.parse::<i64>().unwrap())
                .collect()
        })
        .collect();
    assert_eq!(
        rows.len(),
        m,
        "expected {m} output rows from iverilog, got {}: {stdout}",
        rows.len()
    );
    for i in 0..m {
        for c in 0..n {
            assert_eq!(
                rows[i][c],
                want[i * n + c],
                "RTL o{c} row {i} = {} != eval {} — the emitted array diverged from the reference",
                rows[i][c],
                want[i * n + c]
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    println!("D: emitted weight-stationary RTL ≡ eval_systolic under iverilog ({k}x{n}, {m} rows)");
}

// ── E. reuse dominance over the reduction lane ─────────────────────────────────────────────────────

#[test]
fn weight_stationary_reuse_dominates_the_reduction_lane() {
    // For an M×K×N GEMM: the weight-stationary tile reads each of K·N weights ONCE and reuses it across all M
    // rows, and produces N outputs per cycle → M cycles. A 1-cycle-per-MAC scalar reduction re-fetches
    // operands per MAC and produces one output per K cycles → M·N·K cycles and M·N·K weight-reads.
    for &(k, n, m) in &[(4usize, 4usize, 8usize), (16, 8, 32), (8, 2, 100)] {
        let sys = Systolic::new(k, n, vec![1i64; k * n]);
        let reduction_cycles = m * n * k;
        let reduction_weight_reads = m * n * k;
        assert!(
            sys.cycles(m) < reduction_cycles,
            "weight-stationary must take fewer cycles than the reduction lane ({} vs {})",
            sys.cycles(m),
            reduction_cycles
        );
        assert_eq!(
            sys.weight_reads(),
            k * n,
            "each weight is read exactly once (stationary), independent of M={m}"
        );
        // The reuse factor = how many times each stationary weight is reused across the stream = M.
        let reuse_factor = reduction_weight_reads / sys.weight_reads();
        assert_eq!(
            reuse_factor, m,
            "weight reuse factor must equal the stream length M={m}"
        );
    }
    println!("E: weight-stationary reuses each weight M× and finishes in M cycles (not M·N·K) — reuse-dominant");
}
