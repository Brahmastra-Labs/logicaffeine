//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! FORWARD ORCHESTRATOR — a multi-stage LOGOS tensor-op program composed into ONE CUDA program of chained
//! PARALLEL kernels, run on the GPU. The seam the "beat llama.cpp" path needs.
//!
//! `compile_to_cuda_tensor_op` lowers ONE op; a transformer forward is a SEQUENCE. `compile_to_cuda_forward`
//! ([`codegen_cuda::qwen_emit`]) recognizes each ordered tensor-op stage of the program and emits one `.cu`
//! with a uniquely-named parallel kernel per stage plus a host driver that allocates a device buffer per
//! tensor variable and launches the stages in dataflow order, chaining by variable name.
//!
//! This lock authors a 2-stage forward in LOGOS (`C = A matmul B; D = C plus A` — a projection + residual
//! motif) and a float reduction chain (`Y = rmsnorm X; Z = softmax Y`), emits each as a composed `.cu`, runs
//! it on the GPU, and checks the result against a clean-room Rust reference of the SAME graph. The graph
//! (which ops, order, shapes) is what LOGOS authors; the orchestrator fills inputs deterministically
//! (`v[p] = (p%7)-3`) — wiring the program's own `Set item` values in is the next increment, tracked in the
//! plan. Gated on `nvcc` + a live GPU; SKIPS cleanly otherwise.
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

/// Compile the emitted `.cu` with nvcc and run it, returning stdout lines.
fn nvcc_run(cu: &str, tag: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("logos_orch_{tag}_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (src, bin) = (dir.join("f.cu"), dir.join("f"));
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
        "emitted forward must compile:\n{}\n--- cu ---\n{cu}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = Command::new(&bin).output().expect("run forward");
    assert!(
        r.status.success(),
        "emitted forward run failed:\n{}",
        String::from_utf8_lossy(&r.stderr)
    );
    let out = String::from_utf8_lossy(&r.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// A 2-stage integer forward — `C = A·B` (matmul) then `D = C + A` (elementwise residual) — authored in the
/// LOGOS tensor DSL with the program's OWN element values, emitted as one CUDA program of two chained parallel
/// kernels whose output equals the **VM's** exactly. (The orchestrator wires each input tensor's `Set item`
/// values into its device buffer, so it computes the program's actual result — not a deterministic fill.)
#[test]
fn orchestrator_chains_matmul_then_residual_equals_the_vm_on_the_gpu() {
    let src = "## Main\n\
        Let A be a 2 by 2 tensor of Int.\n\
        Set item 1 of A to 1.\nSet item 2 of A to 2.\nSet item 3 of A to 3.\nSet item 4 of A to 4.\n\
        Let B be a 2 by 2 tensor of Int.\n\
        Set item 1 of B to 5.\nSet item 2 of B to 6.\nSet item 3 of B to 7.\nSet item 4 of B to 8.\n\
        Let C be A matmul B.\n\
        Let D be C plus A.\n\
        Show item 1 of D.\nShow item 2 of D.\nShow item 3 of D.\nShow item 4 of D.\n";
    let cu = compile_to_cuda_forward(src).expect("multi-stage forward must lower to composed CUDA");
    assert!(
        cu.contains("k0_matmul") && cu.contains("k1_elem"),
        "two chained parallel kernels expected:\n{cu}"
    );
    // The program's own input values must reach the emitted host init (not a deterministic fill).
    assert!(
        cu.contains("buf_A[0]=1;") && cu.contains("buf_B[3]=8;"),
        "input Set-values must be wired in:\n{cu}"
    );

    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP orchestrator GPU run: needs nvcc + a live GPU. (Emission asserted above.)");
        return;
    }

    // The ground truth: the VM running the SAME LOGOS program (A·B then +A over the actual Set values).
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
    let expect: Vec<i64> = vm
        .output
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();

    let got: Vec<i64> = nvcc_run(&cu, "int")
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(
        got, expect,
        "orchestrated GPU forward (matmul→residual) must equal the VM"
    );
    println!("Orchestrator: matmul→residual authored in LOGOS ran as chained parallel kernels on the GPU ≡ VM {expect:?}.");
}

/// A 2-stage float reduction chain — `Y = rmsnorm X` then `Z = softmax Y` — over the program's OWN values,
/// composes two parallel fused-row kernels; the GPU output matches the VM within f32 tolerance and is a valid
/// probability distribution. (The float `Set item` values are wired into the input buffer, so the GPU
/// computes the program's actual result; the VM is f64, the GPU f32, hence the tolerance.)
#[test]
fn orchestrator_chains_rmsnorm_then_softmax_equals_the_vm_on_the_gpu() {
    let src = "## Main\n\
        Let X be a 1 by 4 tensor of Real.\n\
        Set item 1 of X to 2.0.\nSet item 2 of X to 4.0.\nSet item 3 of X to 6.0.\nSet item 4 of X to 8.0.\n\
        Let Y be rmsnorm of X.\n\
        Let Z be softmax of Y.\n\
        Show item 1 of Z.\nShow item 2 of Z.\nShow item 3 of Z.\nShow item 4 of Z.\n";
    let cu = compile_to_cuda_forward(src).expect("rmsnorm→softmax must lower to composed CUDA");
    assert!(
        cu.contains("k0_row") && cu.contains("k1_row"),
        "two chained fused-row kernels expected:\n{cu}"
    );
    assert!(
        cu.contains("buf_X[0]=(float)2.0;"),
        "float Set-values must be wired into the input buffer:\n{cu}"
    );

    if !(have_nvcc() && have_gpu()) {
        eprintln!(
            "SKIP orchestrator float GPU run: needs nvcc + a live GPU. (Emission asserted above.)"
        );
        return;
    }

    // Ground truth: the VM running the SAME program (f64 rmsnorm→softmax over [2,4,6,8]).
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
    let expect: Vec<f64> = vm
        .output
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();

    let got: Vec<f32> = nvcc_run(&cu, "float")
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(got.len(), 4, "expected 4 outputs");
    let sum: f32 = got.iter().sum();
    assert!(
        (sum - 1.0).abs() < 1e-4,
        "softmax output must sum to 1 (got {sum})"
    );
    for (g, r) in got.iter().zip(&expect) {
        assert!(
            (*g as f64 - r).abs() < 1e-3,
            "orchestrated GPU rmsnorm→softmax {g} vs VM {r}"
        );
    }
    println!("Orchestrator: rmsnorm→softmax authored in LOGOS ran as chained parallel kernels on the GPU ≡ VM (f32 tol).");
}

/// A real TRANSFORMER SUB-BLOCK — `Xn = rmsnorm(X, Wn); Q = Xn · Wq; R = Q + X` (norm → projection →
/// residual), all float — authored in LOGOS, composed into THREE chained parallel kernels (weighted fused-row
/// + float matmul + float elementwise-add) and run on the GPU ≡ VM within f32 tolerance. This is the exact
/// shape of a Qwen attention/FFN sub-block; the orchestrator now handles a heterogeneous float pipeline.
#[test]
fn orchestrator_runs_a_float_transformer_subblock_equals_the_vm_on_the_gpu() {
    let src = "## Main\n\
        Let X be a 1 by 4 tensor of Real.\n\
        Set item 1 of X to 1.0.\nSet item 2 of X to 2.0.\nSet item 3 of X to 3.0.\nSet item 4 of X to 4.0.\n\
        Let Wn be a 1 by 4 tensor of Real.\n\
        Set item 1 of Wn to 1.0.\nSet item 2 of Wn to 1.0.\nSet item 3 of Wn to 1.0.\nSet item 4 of Wn to 1.0.\n\
        Let Wq be a 4 by 4 tensor of Real.\n\
        Set item 1 of Wq to 1.0.\nSet item 6 of Wq to 1.0.\nSet item 11 of Wq to 1.0.\nSet item 16 of Wq to 1.0.\n\
        Let Xn be rmsnorm of X with weight Wn.\n\
        Let Q be Xn matmul Wq.\n\
        Let R be Q plus X.\n\
        Show item 1 of R.\nShow item 2 of R.\nShow item 3 of R.\nShow item 4 of R.\n";
    let cu =
        compile_to_cuda_forward(src).expect("transformer sub-block must lower to composed CUDA");
    assert!(
        cu.contains("k0_row") && cu.contains("k1_matmul") && cu.contains("k2_elem"),
        "three chained parallel kernels expected (rmsnorm + matmul + add):\n{cu}"
    );

    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP transformer sub-block GPU run: needs nvcc + a live GPU. (Emission asserted above.)");
        return;
    }

    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
    let expect: Vec<f64> = vm
        .output
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(expect.len(), 4, "VM must produce 4 outputs, got {expect:?}");

    let got: Vec<f32> = nvcc_run(&cu, "subblock")
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(got.len(), 4, "expected 4 outputs");
    for (g, r) in got.iter().zip(&expect) {
        assert!(
            (*g as f64 - r).abs() < 1e-3,
            "orchestrated GPU sub-block {g} vs VM {r}"
        );
    }
    println!("Orchestrator: rmsnorm→matmul→residual (a transformer sub-block) authored in LOGOS ran as 3 chained parallel kernels on the GPU ≡ VM.");
}
