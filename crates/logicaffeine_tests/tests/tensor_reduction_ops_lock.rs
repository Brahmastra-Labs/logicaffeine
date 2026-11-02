//! tensor_reduction_ops_lock — the float reduction tensor ops `rmsnorm` and `softmax`.
//!
//! These are the transformer's normalization primitives, and the first FLOAT tensor ops:
//!   • `Let Y be rmsnorm of X.` — `Y[i] = X[i] / sqrt(mean(X²) + eps)` (unit-RMS rescale);
//!   • `Let Y be softmax of X.` — `Y[i] = e^(X[i]-max) / Σ e^(X[j]-max)` (a probability distribution).
//! `X` is a `tensor of Real`. They desugar (in the parser) to ordinary float loops, so every tier runs them.
//!
//! Cross-tier determinism: `sqrt` is IEEE-correctly-rounded (bit-identical on every backend), so `rmsnorm`
//! is exact everywhere; `softmax`'s `e^x` goes through `powf`, which libm implementations round differently,
//! so the test programs `Show round(y·1e6)` — the rounding absorbs sub-ULP divergence and keeps the exact
//! cross-tier string comparison honest.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(tensor_reduction_ops_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_riscv::RISCV_RUNTIME_C;
use logicaffeine_compile::compile::{
    compile_to_c, compile_to_cuda_tensor_op, compile_to_riscv, compile_to_tenstorrent_tensor_op,
    compile_to_wasm, run_logos_source, tw_outcome, vm_outcome,
};

/// `rmsnorm` of the 4-vector [2,4,6,8], each entry shown as `round(y·1e6)`.
const RMSNORM: &str = r#"## Main
Let X be a 1 by 4 tensor of Real.
Set item 1 of X to 2.0.
Set item 2 of X to 4.0.
Set item 3 of X to 6.0.
Set item 4 of X to 8.0.
Let Y be rmsnorm of X.
Show round(item 1 of Y * 1000000).
Show round(item 2 of Y * 1000000).
Show round(item 3 of Y * 1000000).
Show round(item 4 of Y * 1000000).
"#;

/// `softmax` of the 4-vector [1,2,3,4], each entry shown as `round(y·1e6)`.
const SOFTMAX: &str = r#"## Main
Let X be a 1 by 4 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Let Y be softmax of X.
Show round(item 1 of Y * 1000000).
Show round(item 2 of Y * 1000000).
Show round(item 3 of Y * 1000000).
Show round(item 4 of Y * 1000000).
"#;

fn nums(s: &str) -> Vec<f64> {
    s.split_whitespace()
        .map(|t| t.parse::<f64>().expect("integer-ish output"))
        .collect()
}
fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn rmsnorm_and_softmax_run_on_the_tree_walker_and_vm() {
    for src in [RMSNORM, SOFTMAX] {
        assert_eq!(
            tw_outcome(src).output.trim(),
            vm_outcome(src).output.trim(),
            "tw ≡ VM"
        );
    }
}

#[test]
fn rmsnorm_produces_a_unit_rms_vector() {
    // The defining property: after rmsnorm, mean(Y²) ≈ 1. (Outputs are Y·1e6.)
    let ys: Vec<f64> = nums(&vm_outcome(RMSNORM).output)
        .iter()
        .map(|v| v / 1e6)
        .collect();
    assert_eq!(ys.len(), 4);
    let mean_sq = ys.iter().map(|y| y * y).sum::<f64>() / ys.len() as f64;
    assert!(
        (mean_sq - 1.0).abs() < 1e-3,
        "rmsnorm must give unit RMS, got mean(Y²)={mean_sq}"
    );
}

#[test]
fn softmax_is_a_probability_distribution() {
    // Σ Y ≈ 1 and every Y ≥ 0 and monotone in X (X was ascending). (Outputs are Y·1e6.)
    let ys = nums(&vm_outcome(SOFTMAX).output);
    assert_eq!(ys.len(), 4);
    let sum: f64 = ys.iter().sum();
    assert!(
        (sum - 1e6).abs() < 5.0,
        "softmax must sum to 1, got Σ={}",
        sum / 1e6
    );
    assert!(
        ys.windows(2).all(|w| w[0] <= w[1]),
        "softmax must be monotone in ascending X: {ys:?}"
    );
    assert!(
        ys.iter().all(|&y| y >= 0.0),
        "softmax weights are non-negative"
    );
}

#[test]
fn rmsnorm_and_softmax_match_the_vm_on_the_c_backend() {
    if !have_gcc() {
        eprintln!("SKIP C: no gcc.");
        return;
    }
    for src in [RMSNORM, SOFTMAX] {
        let c = compile_to_c(src).expect("must lower to C");
        let dir =
            std::env::temp_dir().join(format!("logos_tred_{}_{:x}", std::process::id(), c.len()));
        let _ = std::fs::create_dir_all(&dir);
        let (s, b) = (dir.join("o.c"), dir.join("o"));
        std::fs::write(&s, &c).unwrap();
        let compiled = std::process::Command::new("gcc")
            .args(["-O2", "-o"])
            .arg(&b)
            .arg(&s)
            .arg("-lm")
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "emitted C must compile:\n{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let got = String::from_utf8_lossy(&std::process::Command::new(&b).output().unwrap().stdout)
            .trim()
            .to_string();
        assert_eq!(
            got,
            vm_outcome(src).output.trim(),
            "AOT-C ≡ VM on the reduction op"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn rmsnorm_and_softmax_run_on_riscv_and_wasm_and_aot_rust() {
    // Every remaining backend: RISC-V (qemu), WASM (lowers), and AOT-Rust (real cargo build) — the float ops
    // reach ALL of them, ≡ the VM. Answers "why can't we make those work on ALL the backends?" — we can.
    for src in [RMSNORM, SOFTMAX] {
        let want = vm_outcome(src).output.trim().to_string();

        // AOT-Rust — a genuine native build + run.
        match run_logos_source(src) {
            Ok(out) => assert_eq!(out.trim(), want, "AOT-Rust ≡ VM on the reduction op"),
            Err(e) => panic!("AOT-Rust must build+run the reduction op: {e}"),
        }

        // WASM — the direct backend lowers the float reduction (Seq + sqrt/pow + Show are all supported).
        assert!(
            compile_to_wasm(src).is_ok(),
            "WASM must lower the reduction op"
        );

        // RISC-V — assemble + run under qemu when the cross toolchain is present.
        let have_rv = std::process::Command::new("riscv64-linux-gnu-gcc")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            && std::process::Command::new("qemu-riscv64")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
        if have_rv {
            let asm = compile_to_riscv(src).expect("reduction must lower to RISC-V");
            let dir = std::env::temp_dir().join(format!(
                "logos_tred_rv_{}_{:x}",
                std::process::id(),
                asm.len()
            ));
            let _ = std::fs::create_dir_all(&dir);
            let (a, rt, b) = (dir.join("m.s"), dir.join("rt.c"), dir.join("m"));
            std::fs::write(&a, &asm).unwrap();
            std::fs::write(&rt, RISCV_RUNTIME_C).unwrap();
            let c = std::process::Command::new("riscv64-linux-gnu-gcc")
                .args(["-O2", "-static", "-o"])
                .arg(&b)
                .arg(&a)
                .arg(&rt)
                .arg("-lm")
                .output()
                .unwrap();
            assert!(
                c.status.success(),
                "reduction RISC-V must assemble:\n{}",
                String::from_utf8_lossy(&c.stderr)
            );
            let out = String::from_utf8_lossy(
                &std::process::Command::new("qemu-riscv64")
                    .arg(&b)
                    .output()
                    .unwrap()
                    .stdout,
            )
            .trim()
            .to_string();
            assert_eq!(out, want, "RISC-V (qemu) ≡ VM on the reduction op");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[test]
fn rmsnorm_and_softmax_are_accelerator_recognized() {
    // No missing cells: the accelerators recognize the reduction ops and emit their kernels.
    for (name, src) in [("rmsnorm", RMSNORM), ("softmax", SOFTMAX)] {
        assert!(
            compile_to_cuda_tensor_op(src).is_ok(),
            "CUDA must recognize {name}"
        );
        assert!(
            compile_to_tenstorrent_tensor_op(src).is_ok(),
            "Tenstorrent must recognize {name}"
        );
    }
}

fn have_nvcc() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}

#[test]
fn the_reduction_cuda_kernels_compile_and_run_on_the_gpu() {
    // Recognized is not enough — the emitted float kernels must actually COMPILE with nvcc and RUN on the
    // RTX 3060 (else the "accelerator" cell is a hollow gap). softmax's Σ Y must be ≈ 1 (a distribution).
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP reduction CUDA GPU: needs nvcc + /dev/nvidia0.");
        return;
    }
    for (name, src, softmax) in [("rmsnorm", RMSNORM, false), ("softmax", SOFTMAX, true)] {
        let cu = compile_to_cuda_tensor_op(src).unwrap();
        let dir =
            std::env::temp_dir().join(format!("logos_tred_gpu_{}_{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let (s, b) = (dir.join("k.cu"), dir.join("k"));
        std::fs::write(&s, &cu).unwrap();
        let c = std::process::Command::new("nvcc")
            .args(["-O2", "-arch=sm_86", "-o"])
            .arg(&b)
            .arg(&s)
            .output()
            .unwrap();
        assert!(
            c.status.success(),
            "{name} CUDA must compile:\n{}\n\n{cu}",
            String::from_utf8_lossy(&c.stderr)
        );
        let r = std::process::Command::new(&b).output().unwrap();
        let out = String::from_utf8_lossy(&r.stdout);
        let sum: f64 = out
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("{name} kernel produced no number: {out:?}"));
        if softmax {
            assert!(
                (sum - 1.0).abs() < 1e-4,
                "softmax kernel Σ Y must be ≈ 1 on the GPU, got {sum}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
