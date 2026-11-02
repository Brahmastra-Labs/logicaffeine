//! ★ TENSTORRENT MATRIX-ENGINE LOCK (Phase 3) ★ — the RDU-class workload (weight-stationary int8
//! GEMM) lowers onto the Tensix matrix engine, gated by the SAME certified capability descriptor the
//! fabric lowering uses (`FabricTarget::rdu_shaped`), and its numerics are byte-for-byte ≡ the VM.
//!
//! Three halves, all runnable without a card:
//! 1. **Certified capability gate.** `rdu_shaped()` advertises int8 weight-stationary GEMM + the
//!    three memory tiers + deep fusion; a target that lacks int8 WS GEMM is REFUSED (typed).
//! 2. **GEMM numerics ≡ VM.** The scalar int8 GEMM oracle compiles with gcc and runs byte-for-byte
//!    identical to the VM — certifying the matmul the kernel computes.
//! 3. **Well-formed matrix-engine artifact.** The compute kernel carries the weight-stationary int8
//!    matmul API surface (`mm_init` / `matmul_tiles` / `tile_regs_*` / `pack_tile`); the host wires
//!    int8 circular buffers + a compute kernel; the reader/writer stream the tiles.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. On-silicon numeric equality (tiled matmul == oracle) is the
//! Phase-5 gate; here we prove the anchor ≡VM and the artifact well-formed.

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::codegen_sva::fabric_target::{
    Capability, Dataflow, FabricTarget, MemTier, Precision,
};
use logicaffeine_compile::compile::{compile_to_c, vm_outcome};
use logicaffeine_compile::spatial::programs::GEMM_ORACLE_LOGOS;

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── 1. Certified capability gate ────────────────────────────────────────────

#[test]
fn rdu_shaped_advertises_int8_weight_stationary_gemm() {
    let rdu = FabricTarget::rdu_shaped();
    // The dials a fused int8 transformer block needs (the RDU's inner workload).
    rdu.require_gemm(Dataflow::WeightStationary, Precision::Int8)
        .expect("int8 WS GEMM on rdu");
    assert!(rdu.provides(Capability::Dataflow(Dataflow::WeightStationary)));
    assert!(rdu.provides(Capability::Precision(Precision::Int8)));
    assert!(
        rdu.provides(Capability::MemTier(MemTier::Sram)),
        "PMU on-chip SRAM"
    );
    assert!(
        rdu.provides(Capability::MemTier(MemTier::Hbm)),
        "on-package HBM"
    );
    assert!(
        rdu.provides(Capability::MemTier(MemTier::Ddr)),
        "off-package DDR"
    );
}

#[test]
fn a_target_without_int8_ws_gemm_is_refused() {
    // A scalar int64-only ALU array (no weight-stationary dataflow, no int8) must refuse the RDU GEMM.
    let scalar = FabricTarget::with_alu_ops("scalar", 8, 8, 64, &[]);
    assert!(scalar
        .require_gemm(Dataflow::WeightStationary, Precision::Int8)
        .is_err());
}

// ── 2. GEMM numerics ≡ VM ────────────────────────────────────────────────────

#[test]
fn scalar_int8_gemm_oracle_matches_the_vm() {
    if !have_gcc() {
        eprintln!("SKIP: gcc not present");
        return;
    }
    let vm = vm_outcome(GEMM_ORACLE_LOGOS);
    assert!(
        vm.error.is_none(),
        "GEMM oracle must run cleanly on the VM: {:?}",
        vm.error
    );

    let c = compile_to_c(GEMM_ORACLE_LOGOS).expect("GEMM oracle compiles to C");
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("logos_ttgemm_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cpath = dir.join("gemm.c");
    let bin = dir.join("gemm");
    std::fs::write(&cpath, &c).unwrap();
    let out = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cpath)
        .arg("-lm")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "gcc: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = std::process::Command::new(&bin).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(run.status.success(), "GEMM oracle binary ran nonzero");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        vm.output.trim(),
        "int8 GEMM oracle diverged from the VM"
    );
}

// ── 3. Well-formed matrix-engine artifact ────────────────────────────────────

// ★ `emitted_gemm_kernel_is_a_wellformed_weight_stationary_int8_matmul` WAS DELETED WITH `matrix.rs`,
// and it is worth recording WHY, because the test passed for as long as it existed while the kernel it
// certified computed garbage.
//
// It asserted that the emitted kernel CONTAINED `matmul_tiles`, `pack_tile`, `tile_regs_acquire` and so
// on. It never asserted a number. Measured under both simulators: the native int8 `matmul_tiles` path
// produced 0/4096 correct elements (196465 against a golden of 131) on Wormhole AND on Blackhole. The
// root cause is not ours to fix — the LLK matmul is MATH_FIDELITY(float)-based and there is no
// integer-accumulate path in the public API, so the matrix engine is float-primary on both parts; TTNN's
// own matmul asserts `is_floating_point` for both inputs. "Rebuild for Blackhole" was tried and
// DISPROVEN by measurement.
//
// So a test spelled in kernel substrings certified a kernel that cannot work. The validated int8 path is
// bf16 matmul (parity-proven) plus int8 quant/dequant on the SFPU, which the spatial IR re-locks as
// stages under P4b. Until that lands there is no int8 GEMM artifact — which is the honest state, and
// strictly better than shipping one whose only evidence was that it contained the right words.
//
// The three tests below are the ones that asserted SEMANTICS, and they are untouched: the capability
// gate, the typed refusal, and the oracle's ≡VM numerics.
