//! cuda_fused_row_lock — the real one-block-per-row fused-row CUDA emitter (RMSNorm / softmax).
//!
//! This is the Phase-1 foundation of the SOL-ExecBench normalization family. The old reduction templates
//! reduced on a single thread over one flat vector; [`emit_fused_row`] generates the transformer shape — one
//! block per row, a shared-memory tree reduction over `cols`, generalizing `qwen_forward.cu::rmsnorm`. The
//! flat sugar is the `rows == 1` case, so this emitter is the durable base the per-row / weighted / fused-add
//! variants build on.
//!
//! The lock pins:
//!   • the emitted kernel IS one-block-per-row with a shared tree reduction (structural, always runs);
//!   • the recognized LOGOS sugar lowers to THIS kernel (LOGOS → real kernel, not a toy);
//!   • the launch bounds are a function of the recognized shape (genuineness — change shape ⇒ change kernel);
//!   • the target chip stamps the kernel (the chip knob flows into emission);
//!   • (nvcc + GPU) PER-ROW RMSNorm over rows > 1 is numerically ≡ a CPU reference on the RTX 3060 — proving
//!     the parallel per-row reduction is correct beyond the flat `rows == 1` case.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_fused_row_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::{emit_fused_row, emit_swiglu, CudaChip, CudaKnobs};
use logicaffeine_compile::compile::compile_to_cuda_tensor_op;
use logicaffeine_compile::intern::Symbol;
use logicaffeine_compile::tensor_recognize::{Epsilon, FusedRowSpec, RowReduceKind, SwiGluSpec};

fn rms(rows: usize, cols: usize) -> FusedRowSpec {
    // x/y/residual/weight/bias symbols are not consumed by the emitter (the kernel is self-contained).
    FusedRowSpec {
        rows,
        cols,
        reduce: RowReduceKind::RmsNorm,
        x: Symbol::EMPTY,
        y: Symbol::EMPTY,
        weight: None,
        weight_rows: 1,
        residual: None,
        bias: None,
        eps: Epsilon::DEFAULT,
    }
}

fn rms_weighted(rows: usize, cols: usize) -> FusedRowSpec {
    FusedRowSpec {
        weight: Some(Symbol::EMPTY),
        ..rms(rows, cols)
    }
}

fn rms_fused_add(rows: usize, cols: usize) -> FusedRowSpec {
    FusedRowSpec {
        weight: Some(Symbol::EMPTY),
        residual: Some(Symbol::EMPTY),
        ..rms(rows, cols)
    }
}

fn layernorm(rows: usize, cols: usize) -> FusedRowSpec {
    FusedRowSpec {
        reduce: RowReduceKind::LayerNorm,
        weight: Some(Symbol::EMPTY),
        bias: Some(Symbol::EMPTY),
        ..rms(rows, cols)
    }
}

/// The weightless `rmsnorm` sugar (1×4) — the same program the reduction lock uses.
const RMSNORM_SUGAR: &str = r#"## Main
Let X be a 1 by 4 tensor of Real.
Set item 1 of X to 2.0.
Set item 2 of X to 4.0.
Set item 3 of X to 6.0.
Set item 4 of X to 8.0.
Let Y be rmsnorm of X.
Show round(item 1 of Y * 1000000).
"#;

#[test]
fn the_emitted_kernel_is_one_block_per_row_with_a_shared_tree_reduction() {
    let cu = emit_fused_row(&rms(4, 8), &CudaKnobs::default());
    assert!(
        cu.contains("__global__ void fused_row"),
        "must emit a fused_row kernel"
    );
    assert!(cu.contains("int row = blockIdx.x"), "one block per ROW");
    assert!(
        cu.contains("__shared__ float sh[THREADS]"),
        "shared-memory scratch for the reduction"
    );
    assert!(
        cu.contains("for(int k = blockDim.x/2; k > 0; k >>= 1)"),
        "a shared-memory TREE reduction"
    );
    assert!(cu.contains("rsqrtf"), "RMSNorm uses rsqrt");
    assert!(
        cu.contains("fused_row<<<ROWS,THREADS>>>"),
        "launch is one block per row"
    );
    assert!(
        cu.contains("#define ROWS 4") && cu.contains("#define COLS 8"),
        "shape is baked from the spec"
    );
}

#[test]
fn the_recognized_logos_sugar_lowers_to_the_real_kernel() {
    // The whole point: a LOGOS program → the REAL one-block-per-row kernel (not the retired serial template).
    let cu = compile_to_cuda_tensor_op(RMSNORM_SUGAR)
        .expect("CUDA must recognize+lower the rmsnorm sugar");
    assert!(
        cu.contains("__global__ void fused_row"),
        "the sugar must lower to the fused_row kernel"
    );
    assert!(
        cu.contains("int row = blockIdx.x"),
        "…which is genuinely one-block-per-row"
    );
    assert!(
        cu.contains("codegen_cuda::fused_row"),
        "provenance: emitted by the fused_row emitter"
    );
}

#[test]
fn the_launch_bounds_are_a_function_of_the_recognized_shape() {
    // Genuineness: different shapes ⇒ different kernels (a template could not do this).
    assert_ne!(
        emit_fused_row(&rms(4, 8), &CudaKnobs::default()),
        emit_fused_row(&rms(8, 8), &CudaKnobs::default())
    );
    assert_ne!(
        emit_fused_row(&rms(4, 8), &CudaKnobs::default()),
        emit_fused_row(&rms(4, 16), &CudaKnobs::default())
    );
}

#[test]
fn softmax_emits_a_max_then_sum_two_pass_reduction() {
    let sm = FusedRowSpec {
        reduce: RowReduceKind::Softmax,
        ..rms(4, 8)
    };
    let cu = emit_fused_row(&sm, &CudaKnobs::default());
    assert!(cu.contains("fmaxf"), "softmax needs the row max");
    assert!(cu.contains("expf"), "softmax exponentiates");
    assert_eq!(
        cu.matches("for(int k = blockDim.x/2; k > 0; k >>= 1)")
            .count(),
        2,
        "max reduction THEN sum reduction"
    );
}

#[test]
fn the_target_chip_stamps_the_kernel() {
    // The chip knob flows into emission — the same op stamped for the local 3060 vs the cloud B200.
    let local = emit_fused_row(&rms(4, 8), &CudaKnobs::default());
    let cloud = emit_fused_row(
        &rms(4, 8),
        &CudaKnobs {
            chip: CudaChip::b200(),
            ..Default::default()
        },
    );
    assert!(
        local.contains("Target: rtx3060 (sm_86)"),
        "local kernel is stamped for the 3060"
    );
    assert!(
        cloud.contains("Target: b200 (sm_100)"),
        "cloud kernel is stamped for the B200"
    );
    assert_ne!(
        local, cloud,
        "toggling the chip knob changes the emitted kernel"
    );
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

/// CPU per-row RMSNorm over the emitter's deterministic fill `x[i] = (i % 7) - 3`, summed over all outputs.
fn cpu_rmsnorm_sum(rows: usize, cols: usize) -> f64 {
    let mut total = 0.0f64;
    for r in 0..rows {
        let mut ss = 0.0f64;
        for i in 0..cols {
            let x = ((r * cols + i) % 7) as f64 - 3.0;
            ss += x * x;
        }
        let denom = (ss / cols as f64 + 1e-6).sqrt();
        for i in 0..cols {
            let x = ((r * cols + i) % 7) as f64 - 3.0;
            total += x / denom;
        }
    }
    total
}

/// CPU per-row WEIGHTED RMSNorm over the emitter's fills `x[i]=(i%7)-3`, `w[c]=1+0.5·(c%3)`, summed.
fn cpu_weighted_rmsnorm_sum(rows: usize, cols: usize) -> f64 {
    let mut total = 0.0f64;
    for r in 0..rows {
        let mut ss = 0.0f64;
        for i in 0..cols {
            let x = ((r * cols + i) % 7) as f64 - 3.0;
            ss += x * x;
        }
        let scale = 1.0 / (ss / cols as f64 + 1e-6).sqrt();
        for i in 0..cols {
            let x = ((r * cols + i) % 7) as f64 - 3.0;
            let w = 1.0 + 0.5 * (i % 3) as f64;
            total += x * scale * w;
        }
    }
    total
}

/// CPU per-row FUSED-ADD weighted RMSNorm: reduced value is `x[i]+res[i]` with `res[i]=((i+1)%5)-2`.
fn cpu_fused_add_rmsnorm_sum(rows: usize, cols: usize) -> f64 {
    let elem = |idx: usize| -> f64 {
        let x = (idx % 7) as f64 - 3.0;
        let res = ((idx + 1) % 5) as f64 - 2.0;
        x + res
    };
    let mut total = 0.0f64;
    for r in 0..rows {
        let mut ss = 0.0f64;
        for i in 0..cols {
            let e = elem(r * cols + i);
            ss += e * e;
        }
        let scale = 1.0 / (ss / cols as f64 + 1e-6).sqrt();
        for i in 0..cols {
            let w = 1.0 + 0.5 * (i % 3) as f64;
            total += elem(r * cols + i) * scale * w;
        }
    }
    total
}

#[test]
fn per_row_rmsnorm_matches_the_cpu_reference_on_the_gpu() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP fused-row GPU: needs nvcc + /dev/nvidia0.");
        return;
    }
    // rows > 1 is the real test: each of the 3 rows must be normalized INDEPENDENTLY over its 8 columns. A
    // flat (whole-tensor) reduction would give a different sum — this pins genuine per-row semantics on-device.
    let knobs = CudaKnobs::default();
    let (rows, cols) = (3usize, 8usize);
    let cu = emit_fused_row(&rms(rows, cols), &knobs);
    let dir = std::env::temp_dir().join(format!("logos_fused_row_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (s, b) = (dir.join("k.cu"), dir.join("k"));
    std::fs::write(&s, &cu).unwrap();
    // Compile for the CHIP the knobs target (the chip knob drives the build).
    let c = std::process::Command::new("nvcc")
        .args(["-O2", &knobs.chip.nvcc_arch_flag(), "-o"])
        .arg(&b)
        .arg(&s)
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "fused_row must compile:\n{}\n\n{cu}",
        String::from_utf8_lossy(&c.stderr)
    );
    let out = String::from_utf8_lossy(&std::process::Command::new(&b).output().unwrap().stdout)
        .trim()
        .to_string();
    let got: f64 = out
        .parse()
        .unwrap_or_else(|_| panic!("kernel produced no number: {out:?}"));
    let want = cpu_rmsnorm_sum(rows, cols);
    assert!(
        (got - want).abs() < 1e-2,
        "per-row RMSNorm Σ Y on GPU={got} must match CPU reference={want}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn per_row_weighted_rmsnorm_matches_the_cpu_reference_on_the_gpu() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP weighted fused-row GPU: needs nvcc + /dev/nvidia0.");
        return;
    }
    // The full HF/FIB shape on-device: per-row normalize (3 rows) THEN per-column weight — numerically ≡ CPU.
    let knobs = CudaKnobs::default();
    let (rows, cols) = (3usize, 8usize);
    let cu = emit_fused_row(&rms_weighted(rows, cols), &knobs);
    assert!(cu.contains("*scale*w[i]"), "must emit the weighted kernel");
    let dir = std::env::temp_dir().join(format!("logos_fused_row_w_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (s, b) = (dir.join("k.cu"), dir.join("k"));
    std::fs::write(&s, &cu).unwrap();
    let c = std::process::Command::new("nvcc")
        .args(["-O2", &knobs.chip.nvcc_arch_flag(), "-o"])
        .arg(&b)
        .arg(&s)
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "weighted fused_row must compile:\n{}\n\n{cu}",
        String::from_utf8_lossy(&c.stderr)
    );
    let out = String::from_utf8_lossy(&std::process::Command::new(&b).output().unwrap().stdout)
        .trim()
        .to_string();
    let got: f64 = out
        .parse()
        .unwrap_or_else(|_| panic!("kernel produced no number: {out:?}"));
    let want = cpu_weighted_rmsnorm_sum(rows, cols);
    assert!(
        (got - want).abs() < 1e-2,
        "weighted per-row RMSNorm Σ Y on GPU={got} must match CPU reference={want}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fused_add_rmsnorm_matches_the_cpu_reference_on_the_gpu() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP fused-add fused-row GPU: needs nvcc + /dev/nvidia0.");
        return;
    }
    // The complete FIB-001 kernel on-device: per-row rmsnorm of (X + residual), then per-column weight.
    let knobs = CudaKnobs::default();
    let (rows, cols) = (3usize, 8usize);
    let cu = emit_fused_row(&rms_fused_add(rows, cols), &knobs);
    assert!(
        cu.contains("(xr[i]+rr[i])") && cu.contains("*scale*w[i]"),
        "must emit the fused-add weighted kernel"
    );
    let dir = std::env::temp_dir().join(format!("logos_fused_row_fa_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (s, b) = (dir.join("k.cu"), dir.join("k"));
    std::fs::write(&s, &cu).unwrap();
    let c = std::process::Command::new("nvcc")
        .args(["-O2", &knobs.chip.nvcc_arch_flag(), "-o"])
        .arg(&b)
        .arg(&s)
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "fused-add fused_row must compile:\n{}\n\n{cu}",
        String::from_utf8_lossy(&c.stderr)
    );
    let out = String::from_utf8_lossy(&std::process::Command::new(&b).output().unwrap().stdout)
        .trim()
        .to_string();
    let got: f64 = out
        .parse()
        .unwrap_or_else(|_| panic!("kernel produced no number: {out:?}"));
    let want = cpu_fused_add_rmsnorm_sum(rows, cols);
    assert!(
        (got - want).abs() < 1e-2,
        "fused-add RMSNorm Σ Y on GPU={got} must match CPU reference={want}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// CPU per-row LayerNorm over the emitter fills `x[i]=(i%7)-3`, `w[c]=1+0.5·(c%3)`, `b[c]=0.1·(c+1)`, summed.
fn cpu_layernorm_sum(rows: usize, cols: usize) -> f64 {
    let x = |idx: usize| (idx % 7) as f64 - 3.0;
    let mut total = 0.0f64;
    for r in 0..rows {
        let mean = (0..cols).map(|i| x(r * cols + i)).sum::<f64>() / cols as f64;
        let var = (0..cols)
            .map(|i| (x(r * cols + i) - mean).powi(2))
            .sum::<f64>()
            / cols as f64;
        let scale = 1.0 / (var + 1e-6).sqrt();
        for i in 0..cols {
            let w = 1.0 + 0.5 * (i % 3) as f64;
            let b = 0.1 * (i + 1) as f64;
            total += (x(r * cols + i) - mean) * scale * w + b;
        }
    }
    total
}

#[test]
fn swiglu_matches_the_cpu_reference_on_the_gpu() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP swiglu GPU: needs nvcc + /dev/nvidia0.");
        return;
    }
    // SwiGLU on-device: y[i] = silu(gate[i])·up[i] over the emitter's fills gate=(i%7)-3, up=((i+1)%5)-2.
    let knobs = CudaKnobs::default();
    let n = 40usize;
    let cu = emit_swiglu(
        &SwiGluSpec {
            total: n,
            gate: Symbol::EMPTY,
            up: Symbol::EMPTY,
            y: Symbol::EMPTY,
        },
        &knobs,
    );
    let dir = std::env::temp_dir().join(format!("logos_swiglu_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (s, b) = (dir.join("k.cu"), dir.join("k"));
    std::fs::write(&s, &cu).unwrap();
    let c = std::process::Command::new("nvcc")
        .args(["-O2", &knobs.chip.nvcc_arch_flag(), "-o"])
        .arg(&b)
        .arg(&s)
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "swiglu must compile:\n{}\n\n{cu}",
        String::from_utf8_lossy(&c.stderr)
    );
    let out = String::from_utf8_lossy(&std::process::Command::new(&b).output().unwrap().stdout)
        .trim()
        .to_string();
    let got: f64 = out
        .parse()
        .unwrap_or_else(|_| panic!("kernel produced no number: {out:?}"));
    let want: f64 = (0..n)
        .map(|i| {
            let g = (i % 7) as f64 - 3.0;
            let up = ((i + 1) % 5) as f64 - 2.0;
            (g / (1.0 + (-g).exp())) * up
        })
        .sum();
    assert!(
        (got - want).abs() < 1e-2,
        "SwiGLU Σ Y on GPU={got} must match CPU reference={want}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn layernorm_matches_the_cpu_reference_on_the_gpu() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP layernorm fused-row GPU: needs nvcc + /dev/nvidia0.");
        return;
    }
    // Per-row LayerNorm on-device: two-pass mean/variance, then scale + shift — numerically ≡ CPU.
    let knobs = CudaKnobs::default();
    let (rows, cols) = (3usize, 8usize);
    let cu = emit_fused_row(&layernorm(rows, cols), &knobs);
    assert!(
        cu.contains("*scale*w[i]+b[i]"),
        "must emit the layernorm scale+shift kernel"
    );
    let dir = std::env::temp_dir().join(format!("logos_fused_row_ln_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (s, b) = (dir.join("k.cu"), dir.join("k"));
    std::fs::write(&s, &cu).unwrap();
    let c = std::process::Command::new("nvcc")
        .args(["-O2", &knobs.chip.nvcc_arch_flag(), "-o"])
        .arg(&b)
        .arg(&s)
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "layernorm fused_row must compile:\n{}\n\n{cu}",
        String::from_utf8_lossy(&c.stderr)
    );
    let out = String::from_utf8_lossy(&std::process::Command::new(&b).output().unwrap().stdout)
        .trim()
        .to_string();
    let got: f64 = out
        .parse()
        .unwrap_or_else(|_| panic!("kernel produced no number: {out:?}"));
    let want = cpu_layernorm_sum(rows, cols);
    assert!(
        (got - want).abs() < 1e-2,
        "LayerNorm Σ Y on GPU={got} must match CPU reference={want}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
