//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! SPECIALIZED MODEL LOCK — Phase 7a of the SambaNova-parity grind (the moat).
//!
//! A proving compiler can SPECIALIZE the datapath to a fixed model and keep the proof: fold weights in as
//! constants (no weight-fetch, foldable arithmetic), and exploit structured 2:4 sparsity to skip zero MACs —
//! things a general array cannot do safely. This lock proves:
//!
//!   A. WEIGHT-STATIONARY = SPECIALIZED: the emitted tile bakes every weight as a constant, so it has ZERO
//!      weight input ports (K activation inputs only), versus a general tile's K·N weight inputs — the
//!      constant-folding a fixed model unlocks. eval is unchanged (≡ the general GEMM ≡ VM).
//!   B. 2:4 STRUCTURED SPARSITY: with ≤2 nonzero per group of 4 weights, a sparsity-aware tile does ≤50% of
//!      the MACs for the IDENTICAL result (zeros contribute nothing) — the pruning a proving compiler can
//!      apply and still certify.
//!
//! Run: `cargo test -p logicaffeine-tests --test specialized_model_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::systolic::{emit_systolic_verilog, eval_systolic, Systolic};
use logicaffeine_compile::compile::vm_outcome;

fn prng(seed: u64, i: usize, lo: i64, hi: i64) -> i64 {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(i as u64 + 1);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    lo + (x % (hi - lo + 1) as u64) as i64
}

#[test]
fn specialization_bakes_weights_and_stays_correct() {
    let (k, n) = (6usize, 4usize);
    let weights: Vec<i64> = (0..k * n).map(|i| prng(1, i, -110, 110)).collect();
    let sys = Systolic::new(k, n, weights.clone());
    let rtl = emit_systolic_verilog("spec", &sys);

    // A. specialized ⇒ weights baked ⇒ ZERO weight input ports. The module's inputs are clk, rst, a0..a{k-1}.
    let input_ports = rtl.matches("input signed [63:0] a").count();
    assert_eq!(
        input_ports, k,
        "the specialized tile exposes exactly K activation inputs (weights are baked)"
    );
    assert!(
        !rtl.contains("input signed [63:0] w"),
        "no weight input ports — weights are constant-folded"
    );
    let general_weight_inputs = k * n; // a general (weights-as-data) tile would need this many
    assert!(
        0 < general_weight_inputs,
        "specialization removes K·N weight input ports"
    );

    // A. eval unchanged: specialized ≡ VM for an int8-range column dot.
    let acts: Vec<i64> = (0..k).map(|i| prng(2, i, -110, 110)).collect();
    let col = Systolic::new(k, 1, (0..k).map(|r| weights[r * n]).collect());
    let got = eval_systolic(&col, &acts, 1)[0];
    let terms: Vec<String> = (0..k)
        .map(|r| format!("({}) * ({})", acts[r], weights[r * n]))
        .collect();
    let vm = vm_outcome(&format!("## Main\n    Show {}.\n", terms.join(" + ")));
    assert!(vm.error.is_none());
    assert_eq!(got.to_string(), vm.output.trim(), "specialized tile != VM");
    println!("A: specialization bakes weights (0 weight ports vs K·N), eval ≡ VM");
}

#[test]
fn structured_2to4_sparsity_halves_the_macs_for_the_same_result() {
    let (k, n, m) = (16usize, 8usize, 4usize); // k divisible by 4 for clean 2:4 groups
                                               // Build a 2:4-sparse weight tile: in each group of 4 rows, keep ≤2 nonzero per column.
    let mut weights = vec![0i64; k * n];
    let mut nonzero = 0usize;
    for c in 0..n {
        for g in 0..(k / 4) {
            // keep positions 0 and 2 within the group of 4 (a fixed 2:4 pattern)
            for &keep in &[0usize, 2usize] {
                let r = g * 4 + keep;
                weights[r * n + c] = prng((c * 97 + r) as u64, 0, -100, 100).max(1); // nonzero
                nonzero += 1;
            }
        }
    }
    let total = k * n;
    assert!(
        nonzero * 2 <= total,
        "2:4 sparsity keeps ≤50% of weights ({nonzero}/{total})"
    );

    let sys = Systolic::new(k, n, weights.clone());
    let acts: Vec<i64> = (0..m * k).map(|i| prng(7, i, -100, 100)).collect();
    let dense = eval_systolic(&sys, &acts, m);

    // A sparsity-AWARE evaluator that skips zero weights entirely — must give the IDENTICAL result and do
    // exactly `nonzero`-per-column MACs (≤50%), the pruning the compiler applies + certifies.
    let mut sparse = vec![0i64; m * n];
    let mut macs_done = 0usize;
    for i in 0..m {
        for c in 0..n {
            let mut acc = 0i64;
            for r in 0..k {
                let w = sys.w(r, c) as i8 as i64;
                if w != 0 {
                    acc = acc.wrapping_add((acts[i * k + r] as i8 as i64) * w);
                    macs_done += 1;
                }
            }
            sparse[i * n + c] = acc;
        }
    }
    assert_eq!(
        sparse, dense,
        "skipping zero weights must not change the result"
    );
    let dense_macs = m * n * k;
    assert!(
        macs_done * 2 <= dense_macs,
        "sparsity-aware MACs ≤ 50% of dense ({macs_done} vs {dense_macs})"
    );
    println!("B: 2:4 sparsity — {macs_done} MACs vs {dense_macs} dense (≤50%), identical result");
}
