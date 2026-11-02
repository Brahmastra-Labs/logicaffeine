//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! PIPELINED SYSTOLIC LOCK — the SOTA register-staged systolic array (TPU-MXU microarchitecture).
//!
//! The combinational weight-stationary tile (`weight_stationary_gemm_lock`) accumulates each column with a
//! K-deep adder tree → critical path grows with K (11 ns at 8×8). This lock proves the PIPELINED array:
//! each PE registers its activation (EAST) and partial sum (SOUTH), so the only combinational path between
//! flip-flops is ONE mul + ONE add — the critical path is K-INDEPENDENT (the property that lets a real MXU
//! stay at GHz at 128×128). Proof legs (added incrementally):
//!
//!   P0 MODEL — the cycle-accurate golden model `eval_systolic_pipelined` computes the SAME O as the
//!      combinational `eval_systolic` (math unchanged; only timing differs), over the reuse battery, with a
//!      K=2,N=2 hand-trace anchor and the latency formula L = K+N−1.
//!   P1 PE-SAT — the pipelined PE's combinational core is the SAME `psum+a·w` already SAT-certified
//!      (pipelining adds only registers).
//!   P2 RTL — the emitted register-staged RTL ≡ the golden model cycle-for-cycle under iverilog (incl. the
//!      fill latency), on asymmetric K≠N shapes + full drain — this proves the timed skew/handoff wiring.
//!   P4 OUTPUT-STATIONARY — the SN40L PCU's other dataflow (R×C stationary accumulators, activations stream
//!      east, weights stream south, K contraction beats). Same three legs: model ≡ GEMM reference ≡ the WS
//!      ≡VM reference, the PE core is the same certified `acc+a·w`, and the emitted OS RTL ≡ the OS model
//!      cycle-for-cycle under iverilog. Latency L = K+R+C−2.
//!   P6 BIST FOLD — the output-stationary tile's self-test wrapper (`systolic_os_pipe8_realbist`) folds its R×C
//!      accumulators row by row: no register it writes XORs more than one row of them, so no fold gathers every
//!      accumulator at one point of the die (the single fold of all 64 was the block's routing hotspot).
//!   P5 DIAL + DOMINANCE — a `SystolicSchedule{Combinational,Pipelined}` selector (both ≡VM, distinct RTL) and
//!      the honest characterization: pipelined clocks faster (K-independent one-MAC path) but pays fill latency
//!      + register area ⇒ per the change policy it is a live TOGGLE, not a strict dominator.
//!
//! Run: `cargo test -p logicaffeine-tests --test pipelined_systolic_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::process::Command;

use logicaffeine_compile::codegen_sva::bitblast::lower_bool;
use logicaffeine_compile::codegen_sva::fabric_grid::{grid_output_to_bounded, route_loop_body};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    fabric_to_bounded, AluOp, FabricOp, LoopCell, Src,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_compile::codegen_sva::systolic::{
    bist_tile_weights, emit_systolic_bist_wrapper, emit_systolic_os_bist_wrapper,
    emit_systolic_os_pipelined_verilog_bits_acc, emit_systolic_pipelined_verilog,
    emit_systolic_pipelined_verilog_bits_acc, emit_systolic_scheduled, emit_systolic_verilog,
    eval_systolic, eval_systolic_os, eval_systolic_os_pipelined, eval_systolic_os_pipelined_acc,
    eval_systolic_os_pipelined_o, eval_systolic_pipelined, eval_systolic_pipelined_acc,
    eval_systolic_pipelined_o, pipelined_latency, pipelined_os_latency, Systolic, SystolicOs,
    SystolicSchedule, ACC_BITS_FULL, ACC_BITS_MXU,
};
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

fn prng(seed: u64, i: usize, lo: i64, hi: i64) -> i64 {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(i as u64 + 1);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    lo + (x % (hi - lo + 1) as u64) as i64
}

// ── P0a: the pipelined model computes the SAME result as the combinational reference ───────────────

#[test]
fn pipelined_ws_matches_the_combinational_reference() {
    let mut checked = 0usize;
    // include ASYMMETRIC K≠N shapes so a K↔N transposition in skew/de-skew cannot hide
    for &(k, n, m) in &[
        (1usize, 1usize, 1usize),
        (2, 3, 4),
        (4, 4, 5),
        (8, 2, 6),
        (3, 8, 3),
        (16, 4, 2),
        (5, 2, 7),
    ] {
        for seed in 0..8u64 {
            let weights: Vec<i64> = (0..k * n).map(|i| prng(seed, i, -128, 127)).collect();
            let acts: Vec<i64> = (0..m * k)
                .map(|i| prng(seed ^ 0x9e37, i, -128, 127))
                .collect();
            let sys = Systolic::new(k, n, weights);
            let (piped, l) = eval_systolic_pipelined(&sys, &acts, m, 8);
            assert_eq!(
                piped,
                eval_systolic(&sys, &acts, m),
                "pipelined {k}x{n} (m={m}, seed={seed}) != combinational"
            );
            assert_eq!(
                l,
                pipelined_latency(k, n),
                "reported latency must equal the shared formula"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 50,
        "the battery must be genuinely large (got {checked})"
    );
    println!(
        "P0a: eval_systolic_pipelined ≡ eval_systolic over {checked} shape×seed cases (incl. K≠N)"
    );
}

// ── P0b: the fill latency is exactly K+N−1 ─────────────────────────────────────────────────────────

#[test]
fn pipelined_ws_latency_is_k_plus_n_minus_1() {
    for &(k, n) in &[
        (2usize, 2usize),
        (8, 8),
        (4, 3),
        (5, 2),
        (16, 16),
        (1, 1),
        (128, 128),
    ] {
        assert_eq!(
            pipelined_latency(k, n),
            k + n - 1,
            "L must be K+N-1 for {k}x{n}"
        );
    }
    // and the model's discovered latency agrees with the formula on a concrete run
    let sys = Systolic::new(4, 3, (0..12).map(|i| prng(1, i, -100, 100)).collect());
    let acts: Vec<i64> = (0..3 * 4).map(|i| prng(2, i, -100, 100)).collect();
    let (_, l) = eval_systolic_pipelined(&sys, &acts, 3, 8);
    assert_eq!(l, 4 + 3 - 1, "model latency must match the formula");
    println!("P0b: fill latency L = K+N−1, agreed by model + formula");
}

// ── P0c: K=2,N=2 hand-trace anchor (guards against a self-consistently-wrong model) ────────────────

#[test]
fn pipelined_ws_2x2_hand_trace() {
    // W = [[w00,w01],[w10,w11]], one activation row A=[a0,a1]. By hand:
    //   O[0][0] = a0*w00 + a1*w10,  O[0][1] = a0*w01 + a1*w11  (emerges de-skewed at cycle L = 2+2-1 = 3).
    let (a0, a1) = (7i64, -5i64);
    let (w00, w01, w10, w11) = (3i64, -2i64, 4i64, 6i64);
    let sys = Systolic::new(2, 2, vec![w00, w01, w10, w11]);
    let (o, l) = eval_systolic_pipelined(&sys, &[a0, a1], 1, 8);
    assert_eq!(l, 3, "L for 2×2 is 3");
    assert_eq!(o[0], a0 * w00 + a1 * w10, "O[0][0] hand-trace");
    assert_eq!(o[1], a0 * w01 + a1 * w11, "O[0][1] hand-trace");
    assert_eq!(
        o,
        eval_systolic(&sys, &[a0, a1], 1),
        "hand-trace also equals eval_systolic"
    );
    let _ = eval_systolic_pipelined_o(&sys, &[a0, a1], 1, 8);
    println!("P0c: 2×2 hand-trace anchors the model (O = A·W, de-skewed at L=3)");
}

// ── P1: the pipelined PE's combinational core is the SAME `psum+a·w` already SAT-certified ──────────
// Pipelining inserts only flip-flops around the PE; the Boolean function between registers is unchanged.
// We re-invoke the exact placement certificate the combinational lock uses, so the pipelined array inherits
// an all-int8 proof of every PE's arithmetic — only the timed WIRING is left to the iverilog leg (P2).

fn to_cells(body: &[FabricOp]) -> Vec<LoopCell> {
    body.iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect()
}

/// The symbolic PE: In0=activation, In1=incoming psum, In2=weight → `psum + a·w`.
fn pe_body() -> Vec<FabricOp> {
    vec![
        FabricOp {
            op: AluOp::Mul,
            a: Src::In(0),
            b: Src::In(2),
        },
        FabricOp {
            op: AluOp::Add,
            a: Src::In(1),
            b: Src::Val(0),
        },
    ]
}

fn pe_certifies_as(claim: &[FabricOp], result: usize) -> bool {
    let g = route_loop_body(&to_cells(&pe_body()), 3, &[Src::Val(1)]);
    let grid = grid_output_to_bounded(&g, 0, 8, &|k| format!("in{k}"));
    let dp = fabric_to_bounded(claim, result, 8);
    let eq = BoundedExpr::Eq(Box::new(grid), Box::new(dp));
    let p = lower_bool(&eq).expect("claim lowers");
    prove_unsat(&ProofExpr::Not(Box::new(p))) == UnsatOutcome::Refuted
}

#[test]
fn pipelined_pe_core_is_the_certified_psum_plus_aw() {
    assert!(
        pe_certifies_as(&pe_body(), 1),
        "the pipelined PE core must be SAT-certified ≡ psum + a·w (all int8)"
    );
    // non-vacuous: it must NOT certify as psum - a·w
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
    assert!(
        !pe_certifies_as(&wrong, 1),
        "the PE must NOT certify as psum − a·w — the SAT would be vacuous"
    );
    println!("P1: the pipelined PE's combinational core is the certified `psum + a·w` (pipelining adds only registers)");
}

// ── P2: emitted register-staged RTL ≡ the golden model cycle-for-cycle under iverilog ──────────────
// This proves the TIMED wiring (skew depths, EAST/SOUTH handoff, de-skew) the per-PE SAT does not: sample
// o{c} after every clock edge; the sample after edge e is model cycle e+1, so output row i = sample[i+L-1].

fn pipelined_rtl_matches_model(k: usize, n: usize, m: usize, seed: u64, acc_bits: u32) {
    let weights: Vec<i64> = (0..k * n).map(|i| prng(seed, i, -128, 127)).collect();
    let acts: Vec<i64> = (0..m * k)
        .map(|i| prng(seed ^ 0xabc, i, -128, 127))
        .collect();
    let sys = Systolic::new(k, n, weights);
    let (want, l) = eval_systolic_pipelined_acc(&sys, &acts, m, 8, acc_bits);
    // pit-of-success guard: at the taped narrow accumulator, the result must equal the full-width (i64) model —
    // i.e. the int8×K accumulation is genuinely lossless at this width, so the tape isn't silently truncating.
    if acc_bits < ACC_BITS_FULL {
        assert_eq!(want, eval_systolic_pipelined(&sys, &acts, m, 8).0,
            "int{acc_bits} accumulator must be lossless for this int8 shape {k}x{n} m={m} (else widen ACC_BITS)");
    }
    let rtl = emit_systolic_pipelined_verilog_bits_acc("psys", &sys, 8, acc_bits);

    let dir = std::env::temp_dir().join(format!("psys_{k}x{n}_{}_{}", m, std::process::id()));
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
        "  psys dut(.clk(clk), .rst(rst), {}, {});\n",
        ac.join(", "),
        oc.join(", ")
    ));
    tb.push_str("  always #5 clk=~clk;\n  initial begin\n");
    for r in 0..k {
        tb.push_str(&format!("    a{r} = 64'sd0;\n"));
    }
    tb.push_str("    rst=1; @(posedge clk); @(posedge clk); #1 rst=0;\n");
    let disp: Vec<String> = (0..n).map(|c| format!("o{c}")).collect();
    let fmt: Vec<&str> = (0..n).map(|_| "%0d").collect();
    for e in 0..(m + l) {
        for r in 0..k {
            let v = if e < m { acts[e * k + r] } else { 0 };
            tb.push_str(&format!("    a{r} = 64'sh{:016X};\n", v as u64));
        }
        tb.push_str("    @(posedge clk); #1;\n");
        tb.push_str(&format!(
            "    $display(\"S {e} {}\", {});\n",
            fmt.join(" "),
            disp.join(", ")
        ));
    }
    tb.push_str("    $finish;\n  end\nendmodule\n");

    let vpath = dir.join("psys.v");
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
        "iverilog build ({k}x{n}): {}",
        String::from_utf8_lossy(&b.stderr)
    );
    let run = Command::new("vvp").arg(&vvp).output().unwrap();
    let so = String::from_utf8_lossy(&run.stdout);
    let samples: HashMap<usize, Vec<i64>> = so
        .lines()
        .filter(|l| l.starts_with("S "))
        .map(|l| {
            let mut it = l[2..].split_whitespace();
            let e: usize = it.next().unwrap().parse().unwrap();
            (e, it.map(|t| t.parse::<i64>().unwrap()).collect())
        })
        .collect();
    // output row i = the sample after edge i+L-1
    for i in 0..m {
        let e = i + l - 1;
        let sample = samples
            .get(&e)
            .unwrap_or_else(|| panic!("no sample at edge {e} ({k}x{n})"));
        for c in 0..n {
            assert_eq!(
                sample[c],
                want[i * n + c],
                "pipelined RTL o{c} row {i} (edge {e}, {k}x{n}) = {} != model {}",
                sample[c],
                want[i * n + c]
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pipelined_rtl_matches_model_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    // asymmetric K≠N shapes + a square, several seeds — a transposed skew/de-skew would fail on K≠N.
    // Both the full (int64) accumulator AND the taped int32 MXU accumulator are proven cycle-exact.
    for &acc_bits in &[ACC_BITS_FULL, ACC_BITS_MXU] {
        for &(k, n, m) in &[
            (8usize, 8usize, 6usize),
            (4, 3, 5),
            (5, 2, 7),
            (3, 8, 4),
            (2, 2, 3),
        ] {
            for seed in 0..3u64 {
                pipelined_rtl_matches_model(k, n, m, seed, acc_bits);
            }
        }
    }
    println!("P2: emitted register-staged RTL ≡ the golden model cycle-for-cycle under iverilog (int64 + taped int32 acc, K≠N + latency)");
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// P4 — OUTPUT-STATIONARY pipelined variant (the SN40L PCU's other dataflow). Accumulators stationary in an
// R×C grid, activations stream EAST, weights stream SOUTH, K contraction beats. Same three proof legs.
// ══════════════════════════════════════════════════════════════════════════════════════════════════

// ── P4a: the OS model ≡ its GEMM reference ≡ the weight-stationary ≡VM reference (same O = A·W) ─────

#[test]
fn os_pipelined_matches_reference_and_ws() {
    let mut checked = 0usize;
    for &(r, c, k) in &[
        (1usize, 1usize, 1usize),
        (2, 3, 4),
        (4, 4, 5),
        (8, 2, 6),
        (3, 8, 3),
        (5, 2, 7),
        (8, 8, 8),
    ] {
        for seed in 0..8u64 {
            let a: Vec<i64> = (0..r * k).map(|i| prng(seed, i, -128, 127)).collect();
            let w: Vec<i64> = (0..k * c)
                .map(|i| prng(seed ^ 0x51ed, i, -128, 127))
                .collect();
            let os = SystolicOs::new(r, c, k);
            let want = eval_systolic_os(&os, &a, &w, 8);
            let (piped, l) = eval_systolic_os_pipelined(&os, &a, &w, 8);
            assert_eq!(
                piped, want,
                "OS pipelined {r}x{c}x{k} (seed={seed}) != OS reference"
            );
            assert_eq!(
                l,
                pipelined_os_latency(r, c, k),
                "OS latency must equal the shared formula"
            );
            // ≡VM: the OS reference is the SAME O = A·W the weight-stationary tile computes (W baked k×c, A streams).
            let ws = Systolic::new(k, c, w.clone());
            assert_eq!(
                want,
                eval_systolic(&ws, &a, r),
                "OS reference must equal the WS ≡VM reference (same A·W)"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 50,
        "the OS battery must be genuinely large (got {checked})"
    );
    println!("P4a: eval_systolic_os_pipelined ≡ eval_systolic_os ≡ eval_systolic (same A·W) over {checked} cases");
}

// ── P4b: OS latency is exactly K+R+C−2, with a 1×1×2 hand-trace anchor ─────────────────────────────

#[test]
fn os_pipelined_latency_and_hand_trace() {
    for &(r, c, k) in &[
        (1usize, 1usize, 1usize),
        (8, 8, 8),
        (4, 3, 5),
        (16, 16, 16),
        (2, 2, 2),
    ] {
        assert_eq!(
            pipelined_os_latency(r, c, k),
            k + r + c - 2,
            "OS L must be K+R+C-2 for {r}x{c}x{k}"
        );
    }
    // 1×1×2 by hand: single accumulator O = a0*w0 + a1*w1, final at cycle K+R+C-2 = 2.
    let (a0, a1, w0, w1) = (7i64, -5i64, 3i64, 4i64);
    let os = SystolicOs::new(1, 1, 2);
    let (o, l) = eval_systolic_os_pipelined(&os, &[a0, a1], &[w0, w1], 8);
    assert_eq!(l, 2, "OS L for 1×1×2 is 2");
    assert_eq!(
        o[0],
        a0 * w0 + a1 * w1,
        "OS 1×1×2 hand-trace: acc = a0·w0 + a1·w1"
    );
    let _ = eval_systolic_os_pipelined_o(&os, &[a0, a1], &[w0, w1], 8);
    println!("P4b: OS fill latency L = K+R+C−2 (formula + model + hand-trace agree)");
}

// ── P4c: the OS PE core is the SAME certified `acc + a·w` (pipelining adds only registers) ──────────

#[test]
fn os_pipelined_pe_core_is_the_certified_acc_plus_aw() {
    // acc + a·w has the identical Boolean structure as psum + a·w (In1 + In0·In2) — the P1 certificate carries.
    assert!(
        pe_certifies_as(&pe_body(), 1),
        "the OS PE core (acc + a·w) reuses the SAT-certified psum + a·w"
    );
    println!("P4c: the output-stationary PE core is the same SAT-certified `acc + a·w`");
}

// ── P4d: emitted OS RTL ≡ the OS golden model cycle-for-cycle under iverilog ────────────────────────

fn os_pipelined_rtl_matches_model(r: usize, c: usize, k: usize, seed: u64, acc_bits: u32) {
    let a: Vec<i64> = (0..r * k).map(|i| prng(seed, i, -128, 127)).collect();
    let w: Vec<i64> = (0..k * c)
        .map(|i| prng(seed ^ 0x51ed, i, -128, 127))
        .collect();
    let os = SystolicOs::new(r, c, k);
    let (want, l) = eval_systolic_os_pipelined_acc(&os, &a, &w, 8, acc_bits);
    if acc_bits < ACC_BITS_FULL {
        assert_eq!(want, eval_systolic_os_pipelined(&os, &a, &w, 8).0,
            "int{acc_bits} OS accumulator must be lossless for int8 {r}x{c}x{k} (else widen ACC_BITS)");
    }
    let rtl = emit_systolic_os_pipelined_verilog_bits_acc("posys", &os, 8, acc_bits);

    let dir = std::env::temp_dir().join(format!("posys_{r}x{c}x{k}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut tb = String::from("`timescale 1ns/1ps\nmodule tb;\n  reg clk=0, rst=1;\n");
    for i in 0..r {
        tb.push_str(&format!("  reg signed [63:0] aw{i};\n"));
    }
    for j in 0..c {
        tb.push_str(&format!("  reg signed [63:0] wn{j};\n"));
    }
    for i in 0..r {
        for j in 0..c {
            tb.push_str(&format!("  wire signed [63:0] acc{i}_{j};\n"));
        }
    }
    let mut conns: Vec<String> = Vec::new();
    for i in 0..r {
        conns.push(format!(".aw{i}(aw{i})"));
    }
    for j in 0..c {
        conns.push(format!(".wn{j}(wn{j})"));
    }
    for i in 0..r {
        for j in 0..c {
            conns.push(format!(".acc{i}_{j}(acc{i}_{j})"));
        }
    }
    tb.push_str(&format!(
        "  posys dut(.clk(clk), .rst(rst), {});\n",
        conns.join(", ")
    ));
    tb.push_str("  always #5 clk=~clk;\n  initial begin\n");
    for i in 0..r {
        tb.push_str(&format!("    aw{i} = 64'sd0;\n"));
    }
    for j in 0..c {
        tb.push_str(&format!("    wn{j} = 64'sd0;\n"));
    }
    tb.push_str("    rst=1; @(posedge clk); @(posedge clk); #1 rst=0;\n");
    let mut disp: Vec<String> = Vec::new();
    let mut fmt: Vec<&str> = Vec::new();
    for i in 0..r {
        for j in 0..c {
            disp.push(format!("acc{i}_{j}"));
            fmt.push("%0d");
        }
    }
    for e in 0..=l {
        for i in 0..r {
            let v = if e < k { a[i * k + e] } else { 0 };
            tb.push_str(&format!("    aw{i} = 64'sh{:016X};\n", v as u64));
        }
        for j in 0..c {
            let v = if e < k { w[e * c + j] } else { 0 };
            tb.push_str(&format!("    wn{j} = 64'sh{:016X};\n", v as u64));
        }
        tb.push_str("    @(posedge clk); #1;\n");
        tb.push_str(&format!(
            "    $display(\"S {e} {}\", {});\n",
            fmt.join(" "),
            disp.join(", ")
        ));
    }
    tb.push_str("    $finish;\n  end\nendmodule\n");

    let vpath = dir.join("posys.v");
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
        "iverilog build OS ({r}x{c}x{k}): {}",
        String::from_utf8_lossy(&b.stderr)
    );
    let run = Command::new("vvp").arg(&vvp).output().unwrap();
    let so = String::from_utf8_lossy(&run.stdout);
    let samples: HashMap<usize, Vec<i64>> = so
        .lines()
        .filter(|l| l.starts_with("S "))
        .map(|l| {
            let mut it = l[2..].split_whitespace();
            let e: usize = it.next().unwrap().parse().unwrap();
            (e, it.map(|t| t.parse::<i64>().unwrap()).collect())
        })
        .collect();
    // accumulators are final after the last edge (e = L)
    let sample = samples
        .get(&l)
        .unwrap_or_else(|| panic!("no OS sample at edge {l} ({r}x{c}x{k})"));
    for idx in 0..r * c {
        assert_eq!(
            sample[idx], want[idx],
            "OS RTL acc[{idx}] ({r}x{c}x{k}) = {} != model {}",
            sample[idx], want[idx]
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn os_pipelined_rtl_matches_model_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    for &acc_bits in &[ACC_BITS_FULL, ACC_BITS_MXU] {
        for &(r, c, k) in &[
            (8usize, 8usize, 8usize),
            (4, 3, 5),
            (5, 2, 4),
            (3, 5, 6),
            (2, 2, 3),
        ] {
            for seed in 0..3u64 {
                os_pipelined_rtl_matches_model(r, c, k, seed, acc_bits);
            }
        }
    }
    println!("P4d: emitted output-stationary RTL ≡ the OS golden model cycle-for-cycle under iverilog (int64 + taped int32 acc, K≠…, latency)");
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// P5 — the microarchitecture DIAL + dominance characterization. Both schedules compute the identical GEMM;
// they TRADE (pipelined buys K-independent Fmax with fill latency + register area) → both stay live toggles.
// ══════════════════════════════════════════════════════════════════════════════════════════════════

// ── P5a: the SystolicSchedule dial dispatches to the two microarchitectures, both ≡VM ──────────────

#[test]
fn systolic_schedule_dial_selects_the_microarchitecture() {
    let sys = Systolic::new(4, 3, (0..12).map(|i| prng(7, i, -128, 127)).collect());
    let comb = emit_systolic_scheduled("d", &sys, SystolicSchedule::Combinational, 8);
    let pipe = emit_systolic_scheduled("d", &sys, SystolicSchedule::Pipelined, 8);
    // the dial must produce EXACTLY the two dedicated emitters
    assert_eq!(
        comb,
        emit_systolic_verilog("d", &sys),
        "Combinational schedule must equal the combinational emitter"
    );
    assert_eq!(
        pipe,
        emit_systolic_pipelined_verilog("d", &sys),
        "Pipelined schedule must equal the pipelined emitter"
    );
    // and they must be genuinely different microarchitectures: only the pipelined one has per-PE ps registers
    assert!(
        pipe.contains("ps_reg_"),
        "pipelined RTL must register partial sums per PE"
    );
    assert!(
        !comb.contains("ps_reg_"),
        "combinational RTL must NOT have per-PE ps registers (it's an adder tree)"
    );
    assert_eq!(
        SystolicSchedule::default(),
        SystolicSchedule::Combinational,
        "default schedule is the smallest tile"
    );
    // both are observationally the SAME GEMM (proven cycle-accurately in P0a; restated here for the dial)
    let acts: Vec<i64> = (0..3 * 4).map(|i| prng(8, i, -128, 127)).collect();
    assert_eq!(
        eval_systolic_pipelined_o(&sys, &acts, 3, 8),
        eval_systolic(&sys, &acts, 3),
        "both schedules ≡VM"
    );
    println!("P5a: the SystolicSchedule dial selects {{Combinational, Pipelined}} — same GEMM, different microarchitecture");
}

// ── P5b: dominance characterization on the MEASURED Sky130 signoff — pipelined is a TOGGLE, not a dominator ─
// Numbers are the recorded VCD/STA signoff from the tapes (benchmarks/silicon/*.json). The combinational 8×8
// closes at 11.0 ns (~90 MHz); the pipelined tile's critical path is ~one MAC and stays FLAT 8→16 (K-independent).
// It pays fill latency + register area, so per the change policy it is a CHARACTERIZED TOGGLE, not a strict win.

#[derive(Clone, Copy)]
struct Sig {
    correct: bool,
    energy_per_mac_pj: f64,
    area_mm2: f64,
    fill_latency_cycles: f64,
    fmax_mhz: f64,
}

#[derive(PartialEq, Debug)]
enum Choice {
    DefaultNew,
    KeepBothToggle,
}

fn policy(new: Sig, old: Sig) -> Choice {
    if !new.correct {
        return Choice::KeepBothToggle;
    }
    let no_worse = new.energy_per_mac_pj <= old.energy_per_mac_pj
        && new.area_mm2 <= old.area_mm2
        && new.fill_latency_cycles <= old.fill_latency_cycles
        && new.fmax_mhz >= old.fmax_mhz;
    let strictly_better = new.energy_per_mac_pj < old.energy_per_mac_pj
        || new.area_mm2 < old.area_mm2
        || new.fill_latency_cycles < old.fill_latency_cycles
        || new.fmax_mhz > old.fmax_mhz;
    if no_worse && strictly_better {
        Choice::DefaultNew
    } else {
        Choice::KeepBothToggle
    }
}

#[test]
fn pipelined_is_a_characterized_toggle_not_a_dominator() {
    // Both tiles as MEASURED on the netlists their signoff ledgers ship (scripts/measure-silicon-power.py
    // -> scripts/record-silicon-power.py): energy/MAC from VCD-annotated power at each tile's own signoff
    // clock, area from the ledger, Fmax the frequency every timing corner meets.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let load = |rel: &str| -> serde_json::Value {
        serde_json::from_str(
            &std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}")),
        )
        .unwrap_or_else(|e| panic!("bad JSON in {rel}: {e}"))
    };
    let tile = load("benchmarks/silicon/systolic_power.json");
    let pipe_p = load("benchmarks/silicon/systolic_pipelined_power.json");
    let num =
        |v: &serde_json::Value, what: &str| v.as_f64().unwrap_or_else(|| panic!("{what} missing"));
    let comb = Sig {
        correct: true,
        energy_per_mac_pj: num(
            &tile["energy_per_mac_pJ"]["vcd_measured"],
            "tile energy/MAC",
        ),
        area_mm2: num(&tile["die_mm2"], "tile area"),
        fill_latency_cycles: 1.0,
        fmax_mhz: num(&tile["fmax_mhz"], "tile fmax"),
    };
    let pipe = Sig {
        correct: true,
        energy_per_mac_pj: num(
            &pipe_p["energy_per_mac_pJ"]["vcd_measured"],
            "pipelined energy/MAC",
        ),
        area_mm2: num(&pipe_p["die_mm2"], "pipelined area"),
        fill_latency_cycles: (8 + 8 - 1) as f64,
        fmax_mhz: num(&pipe_p["fmax_mhz"], "pipelined fmax"),
    };
    // it clocks FASTER (higher Fmax, K-independent) but pays fill latency + register area ⇒ neither dominates.
    assert!(
        pipe.fmax_mhz > comb.fmax_mhz,
        "pipelined must clock faster than the combinational tile: {:.0} vs {:.0} MHz",
        pipe.fmax_mhz,
        comb.fmax_mhz
    );
    assert!(
        pipe.fill_latency_cycles > comb.fill_latency_cycles,
        "pipelined pays fill latency (K+N−1 vs 1)"
    );
    assert_eq!(
        policy(pipe, comb),
        Choice::KeepBothToggle,
        "pipelined is a characterized TOGGLE, not a strict dominator"
    );
    assert_eq!(
        policy(comb, pipe),
        Choice::KeepBothToggle,
        "combinational is likewise not dominated (smaller, zero-latency)"
    );
    println!("P5b: pipelined Fmax≈{:.0} MHz vs combinational≈{:.0} MHz — faster + K-independent but a characterized toggle (fill-latency + area cost)", pipe.fmax_mhz, comb.fmax_mhz);
}


fn silicon(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon").join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The accumulator coordinates one register assignment reads, `acc{i}_{j}` → `(i, j)`.
fn accumulators_read(line: &str) -> Vec<(usize, usize)> {
    let read = &line[line.find("<=").unwrap() + 2..];
    read.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter_map(|w| w.strip_prefix("acc"))
        .filter_map(|rc| rc.split_once('_'))
        .filter_map(|(r, c)| Some((r.parse().ok()?, c.parse().ok()?)))
        .collect()
}

/// An OS self-test wrapper over an `rows × cols` array folds row by row: every register assignment that reads an
/// accumulator reads exactly one row's, all of it, each row has such a fold, and the two half-folds read row words
/// only, every row word once between them (a register's last assignment is its clock step, after its reset).
fn assert_folds_row_by_row(rtl: &str, rows: usize, cols: usize) {
    let assigns: Vec<&str> = rtl.lines().map(str::trim).filter(|l| l.contains("<=")).collect();
    let mut folded = vec![false; rows];
    for line in &assigns {
        let accs = accumulators_read(line);
        if accs.is_empty() {
            continue;
        }
        let row = accs[0].0;
        assert!(accs.iter().all(|(r, _)| *r == row), "a fold reads one row's accumulators: {line}");
        let mut cols_read: Vec<usize> = accs.iter().map(|(_, c)| *c).collect();
        cols_read.sort();
        assert_eq!(cols_read, (0..cols).collect::<Vec<_>>(), "a row's fold reads each accumulator of its row once: {line}");
        assert!(!std::mem::replace(&mut folded[row], true), "row {row} folds into one register");
    }
    assert!(folded.iter().all(|f| *f), "every row folds into a register of its own: {folded:?}");
    let mut words = Vec::new();
    for half in ["foldA", "foldB"] {
        let line = assigns
            .iter()
            .rev()
            .find(|l| l.starts_with(&format!("{half} <= ")))
            .unwrap_or_else(|| panic!("the wrapper writes {half}"));
        let read = &line[line.find("<=").unwrap() + 2..];
        assert!(!read.contains("acc"), "{half} folds row words, never an accumulator: {line}");
        words.extend(
            read.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .filter_map(|w| w.strip_prefix("rowfold"))
                .map(|i| i.parse::<usize>().expect("a row word's index")),
        );
    }
    words.sort();
    assert_eq!(words, (0..rows).collect::<Vec<_>>(), "the half-folds read every row word once");
}

/// P6: the shipped OS tile's self-test wrapper folds its accumulators a row at a time, and so does the emitter at
/// shapes the shipped tile does not have (odd rows, a row wider than the array is tall, one row).
#[test]
fn the_os_tile_s_self_test_folds_its_accumulators_row_by_row() {
    assert_folds_row_by_row(&silicon("systolic_os_pipe8_realbist.v"), 8, 8);
    for (r, c) in [(3, 5), (5, 2), (1, 4), (2, 1)] {
        assert_folds_row_by_row(&emit_systolic_os_bist_wrapper("t", r, c), r, c);
    }
}

/// P6: every pipelined tile `benchmarks/silicon` ships, array and self-test wrapper, is exactly what the library
/// prints for it, so the signed RTL and the emitter that names its structure never drift apart.
#[test]
fn the_shipped_pipelined_tiles_are_what_the_emitter_prints() {
    for d in [8usize, 16] {
        let core = format!("systolic_pipe{d}");
        let sys = Systolic::new(d, d, bist_tile_weights(d, d));
        assert_eq!(silicon(&format!("{core}.v")), emit_systolic_pipelined_verilog_bits_acc(&core, &sys, 8, ACC_BITS_MXU), "{core}.v");
        assert_eq!(silicon(&format!("{core}_realbist.v")), emit_systolic_bist_wrapper(&core, d, d), "{core}_realbist.v");
    }
    let os = SystolicOs::new(8, 8, 8);
    assert_eq!(
        silicon("systolic_os_pipe8.v"),
        emit_systolic_os_pipelined_verilog_bits_acc("systolic_os_pipe8", &os, 8, ACC_BITS_MXU),
        "systolic_os_pipe8.v"
    );
    assert_eq!(silicon("systolic_os_pipe8_realbist.v"), emit_systolic_os_bist_wrapper("systolic_os_pipe8", 8, 8), "systolic_os_pipe8_realbist.v");
}

/// Runs an OS self-test wrapper `{core}_realbist` over its array `rtl` for `cycles` clocks out of reset and returns,
/// per clock, the wrapper's MISR, its `sig` and the `rows × cols` accumulators the array hands it.
fn bist_trace(rtl: &str, core: &str, rows: usize, cols: usize, cycles: usize) -> Vec<(u64, u8, Vec<u64>)> {
    let dir = std::env::temp_dir().join(format!("osbist_{core}_{rows}x{cols}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let accs: Vec<String> = (0..rows).flat_map(|i| (0..cols).map(move |j| format!("dut.acc{i}_{j}"))).collect();
    let fmt = vec!["%h"; accs.len()].join(" ");
    let tb = format!(
        "module tb;\n  reg clk = 0, rst = 1;\n  wire sig;\n  {core}_realbist dut(.clk(clk), .rst(rst), .sig(sig));\n\
         \x20 always #5 clk = ~clk;\n  integer e;\n  initial begin\n\
         \x20   @(posedge clk); @(posedge clk); #1 rst = 0;\n\
         \x20   for (e = 0; e < {cycles}; e = e + 1) begin\n\
         \x20     @(posedge clk); #1;\n\
         \x20     $display(\"S %h %b {fmt}\", dut.misr, sig, {});\n\
         \x20   end\n    $finish;\n  end\nendmodule\n",
        accs.join(", ")
    );
    let vpath = dir.join("osbist.v");
    std::fs::write(&vpath, format!("{rtl}\n{tb}")).unwrap();
    let vvp = dir.join("osbist.vvp");
    let b = Command::new("iverilog").args(["-g2012", "-o"]).arg(&vvp).arg(&vpath).output().unwrap();
    assert!(b.status.success(), "iverilog build of {core}_realbist: {}", String::from_utf8_lossy(&b.stderr));
    let run = Command::new("vvp").arg(&vvp).output().unwrap();
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);
    let trace: Vec<(u64, u8, Vec<u64>)> = out
        .lines()
        .filter_map(|l| l.strip_prefix("S "))
        .map(|l| {
            let mut it = l.split_whitespace();
            let hex = |t: &str| u64::from_str_radix(t, 16).unwrap_or_else(|_| panic!("{core}_realbist drives an unknown value: {l}"));
            let misr = hex(it.next().unwrap());
            let sig = it.next().unwrap().parse::<u8>().unwrap_or_else(|_| panic!("{core}_realbist's sig is unknown: {l}"));
            (misr, sig, it.map(hex).collect())
        })
        .collect();
    assert_eq!(trace.len(), cycles, "{core}_realbist ran {cycles} clocks:\n{out}");
    trace
}

/// The MISR absorbs every accumulator, two clocks late: the row words register the accumulators, the half-folds
/// register the row words, and the MISR takes the half-folds, so on every clock `misr' = rotl(misr) ^ ⊕acc` of the
/// clock two before. Checked on the shipped 8×8 wrapper over its signed array and on emitted shapes the shipped tile
/// does not have; `sig` is the MISR's parity on every clock, and the stimulus reaches the accumulators.
#[test]
fn the_os_tile_s_self_test_signs_every_accumulator_two_clocks_late() {
    if !common::require_tool("iverilog") {
        return;
    }
    let shipped = format!("{}\n{}", silicon("systolic_os_pipe8.v"), silicon("systolic_os_pipe8_realbist.v"));
    let emitted: Vec<(String, String, usize, usize)> = [(3usize, 5usize, 4usize), (5, 2, 3), (1, 4, 2)]
        .into_iter()
        .map(|(r, c, k)| {
            let core = format!("os{r}x{c}");
            let rtl = format!(
                "{}\n{}",
                emit_systolic_os_pipelined_verilog_bits_acc(&core, &SystolicOs::new(r, c, k), 8, ACC_BITS_MXU),
                emit_systolic_os_bist_wrapper(&core, r, c)
            );
            (rtl, core, r, c)
        })
        .collect();
    let cases = std::iter::once((shipped, "systolic_os_pipe8".to_string(), 8, 8)).chain(emitted);
    for (rtl, core, rows, cols) in cases {
        let trace = bist_trace(&rtl, &core, rows, cols, 48);
        let fold = |t: usize| trace[t].2.iter().fold(0u64, |a, x| a ^ x);
        assert!((0..trace.len()).any(|t| fold(t) != 0), "{core}: the LFSR reaches the accumulators");
        for (t, (misr, sig, _)) in trace.iter().enumerate() {
            assert_eq!(*sig as u32, misr.count_ones() % 2, "{core}: sig is the MISR's parity at clock {t}");
        }
        for t in 2..trace.len() - 1 {
            assert_eq!(
                trace[t + 1].0,
                trace[t].0.rotate_left(1) ^ fold(t - 2),
                "{core}: the MISR at clock {} absorbs the accumulators of clock {}",
                t + 1,
                t - 2
            );
        }
    }
}
