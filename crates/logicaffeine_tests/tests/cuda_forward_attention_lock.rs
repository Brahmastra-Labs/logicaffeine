//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ATTENTION — a transpose op + a full scaled-dot-product attention computation authored in LOGOS, composed
//! into chained parallel CUDA kernels and run on the GPU ≡ VM.
//!
//! Attention needs `Q · Kᵀ`, so this lock adds the first-class `transpose` op (`Let Kt be transpose of K`)
//! and proves it lowers to a parallel transpose kernel. Then it authors SINGLE-QUERY attention — the decode
//! step, one query attending to N keys: `Kt = Kᵀ; S = Q·Kt; P = softmax S; C = P·V` — which is REAL
//! attention semantics (the flat softmax over the 1×N score row IS the query's distribution over keys). The
//! orchestrator composes four chained parallel kernels (transpose + matmul + fused-row softmax + matmul), run
//! on the GPU ≡ the VM.
//!
//! Gated on `nvcc` + a live GPU; SKIPS cleanly otherwise (emission is still asserted).
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::process::Command;

use logicaffeine_compile::compile::{compile_to_cuda_forward, vm_outcome};

fn have_nvcc() -> bool {
    Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}

fn nvcc_run(cu: &str, tag: &str) -> Vec<f32> {
    let dir = std::env::temp_dir().join(format!("logos_attn_{tag}_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (src, bin) = (dir.join("a.cu"), dir.join("a"));
    std::fs::write(&src, cu).unwrap();
    let c = Command::new("nvcc")
        .args(["-O2", "-arch=sm_86"])
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("nvcc");
    assert!(
        c.status.success(),
        "emitted program must compile:\n{}\n--- cu ---\n{cu}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = Command::new(&bin).output().expect("run");
    assert!(
        r.status.success(),
        "run failed:\n{}",
        String::from_utf8_lossy(&r.stderr)
    );
    let out = String::from_utf8_lossy(&r.stdout)
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn vm_floats(src: &str) -> Vec<f64> {
    let out = vm_outcome(src);
    assert!(out.error.is_none(), "VM error: {:?}", out.error);
    out.output
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect()
}

/// `Let Kt be transpose of K` lowers to a parallel transpose kernel and matches the VM exactly.
#[test]
fn transpose_lowers_to_a_parallel_kernel_and_matches_the_vm() {
    let src = "## Main\n\
        Let K be a 2 by 3 tensor of Real.\n\
        Set item 1 of K to 1.0.\nSet item 2 of K to 2.0.\nSet item 3 of K to 3.0.\n\
        Set item 4 of K to 4.0.\nSet item 5 of K to 5.0.\nSet item 6 of K to 6.0.\n\
        Let Kt be transpose of K.\n\
        Show item 1 of Kt.\nShow item 2 of Kt.\nShow item 3 of Kt.\n\
        Show item 4 of Kt.\nShow item 5 of Kt.\nShow item 6 of Kt.\n";
    let cu = compile_to_cuda_forward(src).expect("transpose must lower to composed CUDA");
    assert!(
        cu.contains("k0_transpose"),
        "expected a parallel transpose kernel:\n{cu}"
    );

    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP transpose GPU run: needs nvcc + a live GPU. (Emission asserted above.)");
        return;
    }
    let expect = vm_floats(src);
    let got = nvcc_run(&cu, "t");
    assert_eq!(got.len(), 6, "expected 6 outputs");
    // Kᵀ of [[1,2,3],[4,5,6]] = [[1,4],[2,5],[3,6]] → row-major [1,4,2,5,3,6].
    assert_eq!(
        got.iter().map(|&g| g as f64).collect::<Vec<_>>(),
        vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0],
        "transpose layout"
    );
    for (g, r) in got.iter().zip(&expect) {
        assert!((*g as f64 - r).abs() < 1e-4, "GPU transpose {g} vs VM {r}");
    }
    println!("Transpose authored in LOGOS → parallel CUDA kernel ≡ VM.");
}

/// SINGLE-QUERY (decode-step) attention — `Kt = Kᵀ; S = Q·Kt; P = softmax S; C = P·V` — authored in LOGOS,
/// composed into FOUR chained parallel kernels (transpose + matmul + softmax + matmul), runs on the GPU ≡ VM.
#[test]
fn single_query_attention_runs_on_the_gpu_equals_the_vm() {
    let src = "## Main\n\
        Let Q be a 1 by 3 tensor of Real.\n\
        Set item 1 of Q to 0.5.\nSet item 2 of Q to -0.5.\nSet item 3 of Q to 1.0.\n\
        Let K be a 2 by 3 tensor of Real.\n\
        Set item 1 of K to 1.0.\nSet item 2 of K to 0.0.\nSet item 3 of K to -1.0.\n\
        Set item 4 of K to 0.5.\nSet item 5 of K to 0.5.\nSet item 6 of K to 0.5.\n\
        Let V be a 2 by 3 tensor of Real.\n\
        Set item 1 of V to 1.0.\nSet item 2 of V to 2.0.\nSet item 3 of V to 3.0.\n\
        Set item 4 of V to 4.0.\nSet item 5 of V to 5.0.\nSet item 6 of V to 6.0.\n\
        Let Kt be transpose of K.\n\
        Let S be Q matmul Kt.\n\
        Let P be softmax of S.\n\
        Let C be P matmul V.\n\
        Show item 1 of C.\nShow item 2 of C.\nShow item 3 of C.\n";
    let cu = compile_to_cuda_forward(src).expect("attention must lower to composed CUDA");
    for kern in ["k0_transpose", "k1_matmul", "k2_row", "k3_matmul"] {
        assert!(cu.contains(kern), "attention must compose {kern}:\n{cu}");
    }

    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP attention GPU run: needs nvcc + a live GPU. (Emission asserted above.)");
        return;
    }
    let expect = vm_floats(src);
    assert_eq!(
        expect.len(),
        3,
        "VM must produce 3 attention outputs, got {expect:?}"
    );
    let got = nvcc_run(&cu, "a");
    assert_eq!(got.len(), 3, "expected 3 outputs");
    for (g, r) in got.iter().zip(&expect) {
        assert!((*g as f64 - r).abs() < 1e-3, "GPU attention {g} vs VM {r}");
    }
    println!("Single-query attention authored in LOGOS → 4 chained parallel kernels (transpose+matmul+softmax+matmul) on the GPU ≡ VM {expect:?}.");
}
