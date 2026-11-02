//! Every cp.async-pipelined GEMM kernel the CUDA emitter produces commits exactly ONE pipeline
//! group per main-loop iteration — an EMPTY group when no stage is left to load.
//!
//! The pipelines wait with `__pipeline_wait_prior(STAGES-2)` at the top of iteration `s`: "at most
//! `STAGES-2` groups still in flight, so the group that loaded stage `s` has landed". That bound is
//! only meaningful if every iteration committed a group. The prologue already does this
//! (`if (i < nstages) LOAD_STAGE(i, i); else __pipeline_commit();`), but the steady-state tail
//! `int nxt = s + STAGES - 1; if (nxt < nstages) LOAD_STAGE(nxt % STAGES, nxt);` committed nothing
//! once the loads ran out — so on the last `STAGES-2` iterations the wait no longer covered the
//! stage being read, and the kernel occasionally read a buffer whose async copy had not landed.
//! Measured: the flagship `emit_mma_tn_k64_kernel(128,128,2,2,3)` produced INCORRECT results in
//! 2 of 40 runs under the varied-fill oracle (a sampled oracle passes ~95% of the time, which is
//! exactly how this hid). This lock is structural and CPU-only: it reads the emitted CUDA text.
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::*;

/// Every pipelined kernel emitter, at a representative configuration.
fn pipelined_kernels() -> Vec<(&'static str, String)> {
    vec![
        (
            "mma_tn_k64_kernel(128,128,2,2,3)",
            emit_mma_tn_k64_kernel(128, 128, 2, 2, 3),
        ),
        (
            "mma_tn_k64_program(128,128,2,2,3)",
            emit_mma_tn_k64_program(128, 128, 2, 2, 3, 256, 256, 256),
        ),
        (
            "mma_tn_k32swz_kernel(128,128,2,2,3)",
            emit_mma_tn_k32swz_kernel(128, 128, 2, 2, 3),
        ),
        (
            "mma_tn_k32_program(128,128,2,2,3)",
            emit_mma_tn_k32_program(128, 128, 2, 2, 3, 256, 256, 256),
        ),
        (
            "mma_tn_k32db_program(128,128,2,2,3)",
            emit_mma_tn_k32db_program(128, 128, 2, 2, 3, 256, 256, 256),
        ),
        (
            "mma_tn_ldm_swizzle_program(128,128,64,2,2,3,16)",
            emit_mma_tn_ldm_swizzle_program(128, 128, 64, 2, 2, 3, 16, 256, 256, 256),
        ),
        (
            "mma_tn_ldm_cfg_bench(128,128,64,2,2,3)",
            emit_mma_tn_ldm_cfg_bench(128, 128, 64, 2, 2, 3, 256, 256, 256),
        ),
        (
            "wmma_cpasync_i8_program_staged(3)",
            emit_wmma_cpasync_i8_program_staged(&default_wmma_tile(), 3, 256, 256, 256),
        ),
    ]
}

/// Whitespace-insensitive view of the kernel source, so the check reads the structure, not the layout.
fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn every_steady_state_iteration_commits_a_pipeline_group() {
    for (label, cu) in pipelined_kernels() {
        let flat = squash(&cu);
        // Every steady-state prefetch site: `int nxt = s + STAGES - 1; if (nxt < nstages) <LOAD>(...);`
        // (the register-double-buffer variant names it `fetch`).
        let sites: Vec<usize> = flat
            .match_indices("int nxt")
            .chain(flat.match_indices("int fetch"))
            .map(|(i, _)| i)
            .collect();
        assert!(
            !sites.is_empty(),
            "{label}: no steady-state prefetch site found — the pattern moved; update this lock"
        );
        for at in sites {
            let window = &flat[at..(at + 220).min(flat.len())];
            let window = window.replace(' ', "");
            assert!(
                window.contains("else__pipeline_commit();") || window.contains("else{__pipeline_commit();}"),
                "{label}: the steady-state prefetch `if(nxt<nstages) LOAD(...)` (or `fetch`) has no `else __pipeline_commit();` — \
                 the tail iterations then wait on a group count that no longer covers the stage being read \
                 (a measured 5% INCORRECT rate). Site:\n{}",
                &flat[at..(at + 220).min(flat.len())]
            );
        }
    }
}
