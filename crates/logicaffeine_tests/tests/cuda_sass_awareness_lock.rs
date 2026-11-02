//! cuda_sass_awareness_lock — the COMPILER knows its own machine code. It emits its champion GEMM kernel,
//! compiles it, and reads back the SASS report (registers, occupancy, tensor-core MACs, reuse-cache usage) —
//! so the backend can SEE the ptxas ceiling it is up against (here: register-bound, ~33% occupancy). This is
//! the compiler being SASS-aware, not a human eyeballing nvdisasm.
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::emit_mma_tn_k32swz_kernel;
use logicaffeine_compile::codegen_sass::{analyze_source, SassReport};
use std::process::Command;

fn have() -> bool {
    Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && Command::new("nvdisasm")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

#[test]
fn the_compiler_reads_back_the_sass_of_its_own_gemm_kernel() {
    if !have() {
        eprintln!("SKIP: needs nvcc + nvdisasm");
        return;
    }
    // The compiler emits its 79.5%-of-cuBLAS champion kernel and analyzes the machine code ptxas made of it.
    let src = emit_mma_tn_k32swz_kernel(128, 128, 2, 4, 4);
    let rep: SassReport = analyze_source(&src, "gemm_mma_tn_k32swz", "sm_86")
        .expect("compiler analyzes its own SASS");

    eprintln!(
        "COMPILER SASS-AWARENESS of gemm_mma_tn_k32swz: {} insns | {} IMMA (tensor-core MACs) | {} LDSM (ldmatrix) | {} regs/thread | {} reuse-cache insns | avg stall {:.2} | {} blocks/SM @256thr ({}% occupancy)",
        rep.instructions, rep.imma, rep.ldsm, rep.registers, rep.reuse_insns,
        rep.avg_stall_x100 as f64 / 100.0, rep.blocks_per_sm(256), rep.blocks_per_sm(256) * 256 * 100 / 1536
    );

    // The backend must correctly SEE the real machine code of its own output:
    assert!(
        rep.imma >= 8,
        "must see the tensor-core MACs (IMMA) it emitted (got {})",
        rep.imma
    );
    assert!(
        rep.ldsm >= 4,
        "must see the ldmatrix cooperative loads (LDSM) it emitted (got {})",
        rep.ldsm
    );
    assert!(
        rep.registers >= 64,
        "must read the real register/thread count that drives occupancy (got {})",
        rep.registers
    );
    // And it must correctly DIAGNOSE the bind: at 256 threads, >64 regs/thread => <=3 blocks/SM => register-bound.
    let occ = rep.blocks_per_sm(256) * 256 * 100 / 1536;
    assert!(
        occ <= 50,
        "the compiler must SEE it is register-bound (occupancy {occ}% should be low — that is the ptxas ceiling vs cuBLAS)"
    );
}
