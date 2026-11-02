//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! 30B BATCHED DECODE — OCCUPANCY CAP (the delta that beats llama.cpp at B=32) — STRUCTURAL LOCK (no GPU).
//!
//! The batched (B=32) Qwen3-30B-A3B decode is dominated by three MoE GEMV kernels — `bmoe_gate`,
//! `bmoe_up_silu`, `bmoe_down_combine` — which are **65% of the whole decode** (nsys, H100 PCIe). Each of them
//! compiles to **80 registers/thread**, which caps them at ~40% occupancy on the H100 (65536 regs/SM) and leaves
//! them DRAM-latency-bound at ~27% of peak bandwidth. Forcing ptxas to keep **5 resident 256-thread blocks/SM**
//! (`__launch_bounds__(256, 5)` ⇒ ≤51 regs, measured 48 with a few bytes of stack, no meaningful spill) lifts
//! occupancy to ~62% and is the MEASURED throughput peak:
//!
//!     MoE launch-bounds min-blocks sweep, same H100 PCIe pod, tg128, llama-batched-bench S_TG metric, golden:
//!         baseline (80 regs, no cap) ....... 1183.9 tok/s   (85% of llama)
//!         __launch_bounds__(256, 4) ........ 1370.7 tok/s   (98.5%)
//!       ★ __launch_bounds__(256, 5) ........ 1407.9 tok/s   (101.1% — BEATS llama)   ← this lock
//!         __launch_bounds__(256, 6) ........ 1314.9 tok/s   (94.5%, starts to spill)
//!     Extending the cap to the already-low-occupancy kernels (bgk_gemv_qkv 48 regs / bgk_gemv 40 regs) REGRESSES
//!     it (1367 / 1300) — so the cap belongs on the MoE trio ONLY.
//!
//! Measured head-to-head (same H100 PCIe, same Q8_0 weights, -fa on, tg128):
//!     COMPILER-EMITTED (this emitter's own .cu) .. 1407.3 / 1406.3 tok/s, PER_SEQ_AGREE (golden)
//!     llama.cpp llama-batched-bench B=32 S_TG ..... 1391.95 tok/s
//! ⇒ the LOGOS-lowered 30B batched decode BEATS llama.cpp — from OUR compiler.
//!
//! This is a STRUCTURAL lock on the emission (runs anywhere, no GPU): it pins `__launch_bounds__(256, 5)` onto
//! EXACTLY the three MoE kernels and OFF the others, so a silent drop (throughput collapse to 85% of llama) or a
//! mis-tuned min-blocks (4/6 both regress) goes RED. The on-GPU golden-token + throughput proof runs on the pod.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_qwen30b_batched_occupancy_lock)'
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen_batched::emit_qwen30b_generate_batched;
use logicaffeine_compile::codegen_cuda::qwen_emit::Qwen30bCfg;

/// The batched decode emitted exactly as the beat-llama lane runs it (B=32) — the same code path as
/// `examples/emit_qwen30b_batched.rs`. Pure string generation, no GPU, no weight files touched.
fn batched_cu() -> String {
    let cfg = Qwen30bCfg::real();
    emit_qwen30b_generate_batched(&cfg, "/root/w", &[785, 6722, 315, 9625, 374], 24, 32)
}

#[test]
fn the_occupancy_cap_macro_is_the_measured_h100_optimum() {
    let cu = batched_cu();
    // The cap is defined once, as the measured optimum (256 threads, 5 resident blocks/SM ⇒ ≤51 regs). 4 and 6
    // both regress in the sweep above, so this exact value is the pit of success — change it deliberately, never
    // silently.
    assert!(
        cu.contains("#define MOE_OCC __launch_bounds__(256, 5)"),
        "the batched decode must define the H100-measured occupancy cap MOE_OCC = __launch_bounds__(256, 5):\n{cu}"
    );
}

#[test]
fn the_three_dominant_moe_kernels_carry_the_occupancy_cap() {
    let cu = batched_cu();
    // The FFN's 65% — capping these from 80 regs (~40% occ) to ≤51 regs (~62% occ) is the whole win.
    for sig in [
        "__global__ void MOE_OCC bmoe_gate(",
        "__global__ void MOE_OCC bmoe_up_silu(",
        "__global__ void MOE_OCC bmoe_down_combine(",
    ] {
        assert!(
            cu.contains(sig),
            "the register-heavy MoE kernel must carry the occupancy cap — expected `{sig}`:\n{cu}"
        );
    }
}

#[test]
fn the_cap_is_not_applied_to_the_kernels_it_would_regress() {
    let cu = batched_cu();
    // Teeth in the other direction: the sweep proved that capping the already-good-occupancy kernels REGRESSES
    // throughput (qkv 48 regs → 1367, +gemv → 1300). So the fused (unused) gate_up_silu and the attention/O-proj
    // GEMVs must stay UNCAPPED. If someone blankets MOE_OCC over everything, we'd silently drop below llama again.
    for uncapped in [
        "__global__ void bmoe_gate_up_silu(", // the fused variant (112 regs, unused by the decode) stays plain
        "__global__ void bgk_gemv_qkv(",
        "__global__ void bgk_gemv(",
    ] {
        assert!(
            cu.contains(uncapped),
            "this kernel must remain UNCAPPED (capping it regresses throughput) — expected plain `{uncapped}`:\n{cu}"
        );
    }
    // And there must be no launch_bounds glued onto them.
    assert!(
        !cu.contains("MOE_OCC bmoe_gate_up_silu(") && !cu.contains("MOE_OCC bgk_gemv"),
        "the occupancy cap must be scoped to the MoE trio only, never the qkv/O-proj/fused kernels:\n{cu}"
    );
}

#[test]
fn exactly_three_kernels_are_capped() {
    let cu = batched_cu();
    // Belt-and-suspenders on scope: the cap is APPLIED (as a kernel-signature prefix `__global__ void MOE_OCC`)
    // to exactly 3 kernels — the MoE trio, nothing more. A 4th means the cap leaked onto a kernel the sweep says
    // it hurts; a 2nd-or-fewer means one of the trio lost it. (The `#ifndef`/`#define MOE_OCC` guard mentions the
    // token twice more, but those are the definition, not an application.)
    let applied = cu.matches("__global__ void MOE_OCC").count();
    assert_eq!(
        applied, 3,
        "the occupancy cap must be applied to exactly 3 kernels (the MoE trio); found {applied} — leaked or dropped:\n{cu}"
    );
}
