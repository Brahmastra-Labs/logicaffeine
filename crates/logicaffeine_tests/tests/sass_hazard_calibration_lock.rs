//! sass_hazard_calibration_lock — the empirically-calibrated sm_86 hazard model. Every constant here was
//! MEASURED on this GPU (clock64 microbenchmarks) or DECODED from ptxas's own control codes — not guessed. This
//! locks the reverse-engineered timing model so it can't silently drift, and documents the two phases done.
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_sass::SassClass;

#[test]
fn the_hazard_model_matches_the_measured_silicon() {
    // PHASE 1 — instruction latency & throughput (clock64 dependent/independent chains, inline-PTX-forced):
    //   IMAD lat 4.48 / thr 1.21 ; FFMA lat 4.21 / thr 2.65 ; SHF/LOP3 lat 4.21 ; IMMA lat 24.03 / thr 16.04.
    assert_eq!(
        SassClass::Alu.fixed_latency(),
        5,
        "ALU ~4.2-4.5 measured => 5"
    );
    assert_eq!(SassClass::Fp.fixed_latency(), 5, "FFMA 4.21 measured");
    assert_eq!(
        SassClass::Tensor.fixed_latency(),
        24,
        "IMMA dependent-accumulate 24.03 measured (was guessed 8 => hung)"
    );
    assert_eq!(
        SassClass::Tensor.throughput(),
        16,
        "single-warp IMMA throughput 16.04 measured"
    );
    assert_eq!(
        SassClass::Fp.throughput(),
        3,
        "FFMA throughput 2.65 measured"
    );
    assert!(
        SassClass::Alu.throughput() <= 2,
        "IMAD throughput 1.21 measured => high (1)"
    );

    // PHASE 2 — variable-latency memory & the scoreboard model (pointer-chasing + ptxas decode):
    //   LDG (L1 hit) 45.1 cyc ; LDSM (ldmatrix) 45.1 cyc — both barrier-tracked, NOT stall-tracked.
    //   ptxas: LDSM -> write-barrier scoreboards {1,2,4}; LDGSTS(cp.async) -> {0,1}; IMMA waits on its
    //   ldmatrix's exact scoreboard (0b10=sb1, 0b100=sb2, ...).
    assert_eq!(
        SassClass::Memory.variable_result_latency(),
        45,
        "LDG/LDSM ~45 cyc measured (L1/shared hit)"
    );
    assert_eq!(SassClass::AsyncCopy.variable_result_latency(), 45);
    assert!(
        SassClass::Memory.is_variable_latency(),
        "45-cyc loads MUST be scoreboard-tracked, never a fixed stall"
    );
    assert_eq!(
        SassClass::Alu.variable_result_latency(),
        0,
        "fixed-latency ops don't set a variable barrier"
    );

    // the calibrated model must be internally consistent: variable memory latency (45) >> the fixed IMMA
    // latency (24) >> ALU (5). This ordering is WHY loads use barriers and ALU/IMMA use stalls.
    assert!(SassClass::Memory.variable_result_latency() > SassClass::Tensor.fixed_latency() as u32);
    assert!((SassClass::Tensor.fixed_latency() as u32) > SassClass::Alu.fixed_latency() as u32);
}
