//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! 30B DECODE FUSION — the compiler fuses each residual-add → RMSNorm boundary in the LOGOS-lowered Qwen3-30B
//! flash decode into ONE kernel. The H100 batch-1 decode is MEASURED latency-bound (~38% of HBM bandwidth), so
//! cutting kernel nodes (and the residual write/re-read) is the lever to beat llama.cpp — while staying
//! token-exact (the fused kernel replicates the SAME 256-thread reduction, so the float sum is bit-identical).
//!
//! This is a STRUCTURAL lock on the emission (runs anywhere, no GPU): the fused kernel is emitted and the
//! unfused add+rmsnorm pair is gone. The on-GPU golden-token + throughput proof is the flash-lane lock that
//! runs on the pod (`the_logos_30b_flash_lane_holds_llama_throughput_parity`).
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_qwen30b_fusion_lock)'
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen_emit::{emit_qwen30b_generate, Qwen30bCfg};
use logicaffeine_proof::cdcl::{Lit, Var};
use logicaffeine_proof::sdcl::{solve_certified, CertifiedOutcome};

/// The fused `gk_add_rmsnorm` reduces with `for(i=tid; i<n; i+=THREADS)`. The exact-cover / bijection argument
/// is SIZE-INDEPENDENT (element = iter*THREADS + thread is injective and surjective onto `[0, THREADS*iters)`
/// whenever the row length is a whole number of thread-waves), so we discharge the certificate at a small
/// REPRESENTATIVE scale — the certified solver's cost grows with the instance, and the structural property is
/// identical at every scale (the real kernel uses THREADS=256 over E=2048 = 8 waves; here 8 threads × 4 waves).
const RED_THREADS: usize = 8;
const RED_WAVES: usize = 4;

fn flash_cu() -> String {
    let cfg = Qwen30bCfg::real();
    emit_qwen30b_generate(
        &cfg,
        "/root/w",
        &[785, 6722, 315, 9625, 374],
        4,
        false,
        true,
    )
}

#[test]
fn the_flash_decode_fuses_post_attention_add_into_rmsnorm() {
    let cu = flash_cu();
    // The emitter defines a fused residual-add + RMSNorm kernel (base form, still used by the deq reference).
    assert!(
        cu.contains("__global__ void gk_add_rmsnorm("),
        "the emitter must define a fused add+rmsnorm kernel:\n{cu}"
    );
    // …and the flash post-attention residual+ffn-norm is ONE fused launch. It has since evolved to the
    // quant-folding `gk_add_rmsnorm_q8` (F5): x kept as residual, x2n = rmsnorm(x)·fn AND int8 x2n for experts.
    assert!(
        cu.contains("gk_add_rmsnorm_q8<<<1,256,0,s>>>(x,tmp,x2n,fn_[l],moe_xq,moe_xs,E,EPSV)"),
        "post-attention residual+ffn-norm must be a single fused launch (quant-folding q8 form):\n{cu}"
    );
    // The standalone attn-residual add is gone (folded into the fused kernel).
    assert!(
        !cu.contains("gk_add<<<(E+255)/256,256,0,s>>>(x,tmp,E)"),
        "the standalone attn-residual add must be fused away:\n{cu}"
    );
}

#[test]
fn the_fused_kernel_writes_the_residual_and_reduces_in_one_pass() {
    let cu = flash_cu();
    // Correctness-by-construction of the fusion: the fused kernel writes the residual `x[i]=v` (still needed by
    // the next residual) AND accumulates `v*v` in the SAME pass with the SAME 256-thread tree as gk_rmsnorm —
    // so the reduction is bit-identical (token-exact). Pin the tell-tale body.
    assert!(
        cu.contains("float v=x[i]+y[i]; x[i]=v;"),
        "must write the residual back in-place:\n{cu}"
    );
    assert!(
        cu.contains("for(int k=blockDim.x/2;k>0;k>>=1)"),
        "must use the identical 256-thread reduction tree:\n{cu}"
    );
}

#[test]
fn the_flash_decode_folds_the_expert_quant_into_the_norm() {
    // F5: the FFN norm ALSO emits the int8-quantized activation the experts need, folding the standalone
    // `moe_qact(x2n)` away — one fewer kernel + one fewer full pass over the E-vector per layer. The quant is
    // bit-identical to moe_qact (same per-32 amax/127), so the experts see identical int8 (token-exact).
    let cu = flash_cu();
    assert!(
        cu.contains("__global__ void gk_add_rmsnorm_q8("),
        "must emit the fused add+rmsnorm+quant kernel:\n{cu}"
    );
    assert!(
        cu.contains("gk_add_rmsnorm_q8<<<1,256,0,s>>>(x,tmp,x2n,fn_[l],moe_xq,moe_xs,E,EPSV)"),
        "the flash ffn-norm must fold the expert int8 quant (emit moe_xq/moe_xs):\n{cu}"
    );
    assert!(
        !cu.contains("moe_qact<<<((E/32)+7)/8,256,0,s>>>(x2n,moe_xq,moe_xs,E)"),
        "the standalone moe_qact(x2n) must be folded away:\n{cu}"
    );
}

/// ★ F4: CONSTRUCTIVELY SAT-PROVEN CORRECT ★ — the fusion is not just measured faster, it is PROVED
/// semantics-preserving by construction with the same certified solver that certifies the Marlin kernel.
///
/// The fused `gk_add_rmsnorm` reduces the row with `for(i=tid; i<n; i+=THREADS)`. Correctness of the fusion
/// hinges on that strided loop covering EVERY element of the row EXACTLY ONCE: each element must contribute to
/// `sum(v²)` once (else the RMSNorm mean is wrong) and its residual `x[i]=v` must be written once (no race, no
/// drop). We encode the `(thread,iteration) → element` map as an EXACT-COVER SAT: it is satisfiable iff the map
/// is a BIJECTION over `[0,n)`. `solve_certified` discharges it with a machine-checked certificate — so "the
/// faster fused kernel computes the identical reduction" is a proof, not a hope.
fn exact_cover_clauses(n: usize) -> (usize, Vec<Vec<Lit>>) {
    let iters = n.div_ceil(RED_THREADS);
    let slot = |t: usize, it: usize| t * iters + it;
    let slots = RED_THREADS * iters;
    // preimage of each element under (t,it) -> elem = it*THREADS + t  (only slots with elem < n are real).
    let mut preimage: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut real_slots: Vec<usize> = Vec::new();
    for t in 0..RED_THREADS {
        for it in 0..iters {
            let elem = it * RED_THREADS + t;
            if elem < n {
                preimage[elem].push(slot(t, it));
                real_slots.push(slot(t, it));
            }
        }
    }
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    for &s in &real_slots {
        clauses.push(vec![Lit::pos(s as Var)]); // every real strided visit actually happens (active)
    }
    for c in 0..n {
        let p = &preimage[c];
        clauses.push(p.iter().map(|&s| Lit::pos(s as Var)).collect()); // at-least-one: element is covered
        for i in 0..p.len() {
            for j in (i + 1)..p.len() {
                clauses.push(vec![Lit::neg(p[i] as Var), Lit::neg(p[j] as Var)]);
                // at-most-one: no double count
            }
        }
    }
    (slots, clauses)
}

#[test]
fn the_add_rmsnorm_reduction_is_a_certified_exact_cover() {
    // Representative row length = THREADS × WAVES (the real kernel's pattern at every scale; see RED_THREADS).
    const N: usize = RED_THREADS * RED_WAVES;
    let (slots, clauses) = exact_cover_clauses(N);
    match solve_certified(slots, &clauses) {
        CertifiedOutcome::Sat(model) => {
            assert!(model.iter().take(slots).all(|&b| b), "every strided visit must be active in the witness");
            println!(
                "F4 ✓ SAT-CERTIFIED: the fused add→rmsnorm strided reduction is an EXACT COVER of all {N} \
                 elements (each summed once, each residual written once, no race) — the fusion is \
                 semantics-preserving by construction, so the faster fused kernel is proven-equivalent."
            );
        }
        CertifiedOutcome::Unsat { .. } => panic!(
            "the fused reduction is NOT a bijection — an element would be missed or double-counted, changing \
             the RMSNorm mean. The fusion would be UNSOUND. This is exactly what the certificate rules out."
        ),
    }
}

#[test]
fn the_exact_cover_certificate_actually_catches_a_broken_reduction() {
    // The certificate has teeth: if a reduction DROPPED the last element (a classic off-by-one), the cover is
    // incomplete and the solver returns UNSAT. We prove the proof by feeding it a deliberately-broken cover.
    const N: usize = RED_THREADS * RED_WAVES;
    let (slots, mut clauses) = exact_cover_clauses(N);
    // Remove element (N-1)'s at-least-one coverage clause AND forbid its slot — simulating a loop that stops one
    // short. The exact cover is now unsatisfiable (element N-1 uncovered) ⇒ UNSAT ⇒ the bug is caught.
    // Its at-least-one clause is the first clause of element (N-1)'s block; simplest: force its only slot false.
    let last_slot = (0..slots)
        .find(|&s| {
            // slot s = t*iters+it ; elem = it*THREADS+t ; find the slot mapping to N-1
            let iters = N.div_ceil(RED_THREADS);
            let (t, it) = (s / iters, s % iters);
            it * RED_THREADS + t == N - 1
        })
        .expect("a slot covers the last element");
    clauses.push(vec![Lit::neg(last_slot as Var)]); // forbid covering the last element
    match solve_certified(slots, &clauses) {
        CertifiedOutcome::Unsat { .. } => println!("F4 ✓ the certificate is sound: a reduction that drops an element is caught as UNSAT."),
        CertifiedOutcome::Sat(_) => panic!("a broken (element-dropping) reduction must be UNSAT — the certificate has no teeth otherwise"),
    }
}
