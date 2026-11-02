//! fused_row_family_lock — the per-row **weighted** RMSNorm sugar (the transformer normalization shape).
//!
//! `Let Y be rmsnorm of X with weight W.` — for an `R by C` tensor `X` and a length-`C` weight `W`:
//!   `Y[r,i] = X[r,i] / sqrt(mean_i(X[r,·]²) + 1e-6) * W[i]`  — normalized PER ROW, then scaled per column.
//!
//! This is the exact HF RMSNorm (and the reduce+scale core of FIB `001_fused_add_rmsnorm`). It desugars to
//! ordinary per-row float loops, so every executing tier runs it ≡ the VM. (Accelerator recognition + the
//! cross-backend coverage ratchet land in the following steps; this lock pins executing-tier correctness.)
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(fused_row_family_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::compile::{
    compile_to_c, compile_to_cuda_tensor_op, compile_to_tenstorrent_tensor_op, run_logos_source,
    tw_outcome, vm_outcome,
};
use logicaffeine_compile::spatial::describe::compute_facts;
use logicaffeine_compile::spatial::ir::SfpuKind;
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;

/// A 2×3 `X` and a length-3 weight `W`, each output shown as `round(y·1e6)` (cross-tier-deterministic).
const WEIGHTED: &str = r#"## Main
Let X be a 2 by 3 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Set item 5 of X to 5.0.
Set item 6 of X to 6.0.
Let W be a 1 by 3 tensor of Real.
Set item 1 of W to 0.5.
Set item 2 of W to 1.0.
Set item 3 of W to 2.0.
Let Y be rmsnorm of X with weight W.
Show round(item 1 of Y * 1000000).
Show round(item 2 of Y * 1000000).
Show round(item 3 of Y * 1000000).
Show round(item 4 of Y * 1000000).
Show round(item 5 of Y * 1000000).
Show round(item 6 of Y * 1000000).
"#;

/// The reference: per-row weighted RMSNorm over the 2×3 `X` and length-3 `W`, each `round(y·1e6)`.
fn reference() -> Vec<f64> {
    let x = [[1.0f64, 2.0, 3.0], [4.0, 5.0, 6.0]];
    let w = [0.5f64, 1.0, 2.0];
    let mut out = Vec::new();
    for row in x {
        let ss: f64 = row.iter().map(|v| v * v).sum();
        let denom = (ss / 3.0 + 1e-6).sqrt();
        for i in 0..3 {
            out.push((row[i] / denom * w[i] * 1e6).round());
        }
    }
    out
}

/// The full FIB-001 shape: `Y = rmsnorm(X + R) * W`, per row, over a 2×3 `X`/`R` and length-3 `W`.
const FUSED_ADD: &str = r#"## Main
Let X be a 2 by 3 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Set item 5 of X to 5.0.
Set item 6 of X to 6.0.
Let R be a 2 by 3 tensor of Real.
Set item 1 of R to 0.5.
Set item 2 of R to 0.5.
Set item 3 of R to 0.5.
Set item 4 of R to 1.0.
Set item 5 of R to 1.0.
Set item 6 of R to 1.0.
Let W be a 1 by 3 tensor of Real.
Set item 1 of W to 0.5.
Set item 2 of W to 1.0.
Set item 3 of W to 2.0.
Let Y be rmsnorm of X plus R with weight W.
Show round(item 1 of Y * 1000000).
Show round(item 2 of Y * 1000000).
Show round(item 3 of Y * 1000000).
Show round(item 4 of Y * 1000000).
Show round(item 5 of Y * 1000000).
Show round(item 6 of Y * 1000000).
"#;

/// Reference for the fused-add: per-row RMSNorm of `X + R`, scaled by `W`.
fn fused_add_reference() -> Vec<f64> {
    let x = [[1.0f64, 2.0, 3.0], [4.0, 5.0, 6.0]];
    let r = [[0.5f64, 0.5, 0.5], [1.0, 1.0, 1.0]];
    let w = [0.5f64, 1.0, 2.0];
    let mut out = Vec::new();
    for row in 0..2 {
        let e: Vec<f64> = (0..3).map(|i| x[row][i] + r[row][i]).collect();
        let ss: f64 = e.iter().map(|v| v * v).sum();
        let denom = (ss / 3.0 + 1e-6).sqrt();
        for i in 0..3 {
            out.push((e[i] / denom * w[i] * 1e6).round());
        }
    }
    out
}

/// Per-row LayerNorm: `Y = (X - mean)/sqrt(var + eps) * W + B`, over a 2×3 `X`, length-3 `W`/`B`.
const LAYERNORM: &str = r#"## Main
Let X be a 2 by 3 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Set item 5 of X to 5.0.
Set item 6 of X to 6.0.
Let W be a 1 by 3 tensor of Real.
Set item 1 of W to 0.5.
Set item 2 of W to 1.0.
Set item 3 of W to 2.0.
Let B be a 1 by 3 tensor of Real.
Set item 1 of B to 0.1.
Set item 2 of B to 0.2.
Set item 3 of B to 0.3.
Let Y be layernorm of X with weight W and bias B.
Show round(item 1 of Y * 1000000).
Show round(item 2 of Y * 1000000).
Show round(item 3 of Y * 1000000).
Show round(item 4 of Y * 1000000).
Show round(item 5 of Y * 1000000).
Show round(item 6 of Y * 1000000).
"#;

/// Reference for per-row LayerNorm over the 2×3 `X`, length-3 `W`/`B`.
fn layernorm_reference() -> Vec<f64> {
    let x = [[1.0f64, 2.0, 3.0], [4.0, 5.0, 6.0]];
    let w = [0.5f64, 1.0, 2.0];
    let b = [0.1f64, 0.2, 0.3];
    let mut out = Vec::new();
    for row in x {
        let mean = row.iter().sum::<f64>() / 3.0;
        let var = row.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / 3.0;
        let denom = (var + 1e-6).sqrt();
        for i in 0..3 {
            out.push((((row[i] - mean) / denom * w[i] + b[i]) * 1e6).round());
        }
    }
    out
}

/// SwiGLU: `Y[i] = silu(G[i]) * U[i]`, `silu(g) = g/(1+e^-g)`, over a 1×4 `G`/`U`.
const SWIGLU: &str = r#"## Main
Let G be a 1 by 4 tensor of Real.
Set item 1 of G to -1.0.
Set item 2 of G to 0.0.
Set item 3 of G to 1.0.
Set item 4 of G to 2.0.
Let U be a 1 by 4 tensor of Real.
Set item 1 of U to 1.0.
Set item 2 of U to 2.0.
Set item 3 of U to 3.0.
Set item 4 of U to 4.0.
Let Y be swiglu of G and U.
Show round(item 1 of Y * 1000000).
Show round(item 2 of Y * 1000000).
Show round(item 3 of Y * 1000000).
Show round(item 4 of Y * 1000000).
"#;

/// Reference for SwiGLU over the 1×4 `G`/`U`.
fn swiglu_reference() -> Vec<f64> {
    let g = [-1.0f64, 0.0, 1.0, 2.0];
    let u = [1.0f64, 2.0, 3.0, 4.0];
    (0..4)
        .map(|i| {
            let silu = g[i] / (1.0 + (-g[i]).exp());
            (silu * u[i] * 1e6).round()
        })
        .collect()
}

fn nums(s: &str) -> Vec<f64> {
    s.split_whitespace()
        .map(|t| t.parse::<f64>().expect("integer-ish output"))
        .collect()
}

#[test]
fn weighted_rmsnorm_is_per_row_and_matches_the_reference_on_the_vm() {
    let got = nums(&vm_outcome(WEIGHTED).output);
    assert_eq!(
        got,
        reference(),
        "weighted per-row RMSNorm must match the hand-computed reference"
    );
}

#[test]
fn weighted_rmsnorm_agrees_across_the_tree_walker_and_vm() {
    assert_eq!(
        tw_outcome(WEIGHTED).output.trim(),
        vm_outcome(WEIGHTED).output.trim(),
        "tw ≡ VM"
    );
}

#[test]
fn swiglu_matches_the_reference_on_the_vm() {
    let got = nums(&vm_outcome(SWIGLU).output);
    assert_eq!(
        got,
        swiglu_reference(),
        "swiglu must be silu(G)·U elementwise"
    );
}

#[test]
fn swiglu_agrees_across_the_tree_walker_and_vm() {
    assert_eq!(
        tw_outcome(SWIGLU).output.trim(),
        vm_outcome(SWIGLU).output.trim(),
        "tw ≡ VM"
    );
}

#[test]
fn swiglu_matches_the_vm_on_aot_rust() {
    match run_logos_source(SWIGLU) {
        Ok(out) => assert_eq!(
            out.trim(),
            vm_outcome(SWIGLU).output.trim(),
            "AOT-Rust ≡ VM"
        ),
        Err(e) => panic!("AOT-Rust must build+run swiglu: {e}"),
    }
}

#[test]
fn swiglu_is_recognized_and_lowered_by_both_accelerators() {
    assert!(
        compile_to_cuda_tensor_op(SWIGLU).is_ok(),
        "CUDA must recognize+lower swiglu"
    );
    assert!(
        compile_to_tenstorrent_tensor_op(SWIGLU).is_ok(),
        "Tenstorrent must recognize+lower swiglu"
    );
    let cu = compile_to_cuda_tensor_op(SWIGLU).unwrap();
    assert!(
        cu.contains("__global__ void swiglu"),
        "emits a swiglu kernel: {cu}"
    );
    assert!(
        cu.contains("g/(1.0f+expf(-g))") && cu.contains("#define N 4"),
        "silu(gate)·up over N=4"
    );
}

#[test]
fn layernorm_matches_the_reference_on_the_vm() {
    let got = nums(&vm_outcome(LAYERNORM).output);
    assert_eq!(
        got,
        layernorm_reference(),
        "per-row LayerNorm must be (X-mean)/sqrt(var+eps)*W+B"
    );
}

#[test]
fn layernorm_agrees_across_the_tree_walker_and_vm() {
    assert_eq!(
        tw_outcome(LAYERNORM).output.trim(),
        vm_outcome(LAYERNORM).output.trim(),
        "tw ≡ VM"
    );
}

#[test]
fn layernorm_matches_the_vm_on_aot_rust() {
    match run_logos_source(LAYERNORM) {
        Ok(out) => assert_eq!(
            out.trim(),
            vm_outcome(LAYERNORM).output.trim(),
            "AOT-Rust ≡ VM"
        ),
        Err(e) => panic!("AOT-Rust must build+run layernorm: {e}"),
    }
}

#[test]
fn layernorm_is_recognized_and_lowered_by_both_accelerators() {
    assert!(
        compile_to_cuda_tensor_op(LAYERNORM).is_ok(),
        "CUDA must recognize+lower layernorm"
    );
    assert!(
        compile_to_tenstorrent_tensor_op(LAYERNORM).is_ok(),
        "Tenstorrent must recognize+lower layernorm"
    );
    let cu = compile_to_cuda_tensor_op(LAYERNORM).unwrap();
    assert!(
        cu.contains("#define ROWS 2") && cu.contains("#define COLS 3"),
        "recognized the 2x3 shape: {cu}"
    );
    assert!(
        cu.contains("mean") && cu.contains("*scale*w[i]+b[i]"),
        "two-pass mean/var, then scale + shift"
    );
}

#[test]
fn fused_add_rmsnorm_matches_the_reference_on_the_vm() {
    let got = nums(&vm_outcome(FUSED_ADD).output);
    assert_eq!(
        got,
        fused_add_reference(),
        "fused-add RMSNorm must be rmsnorm(X+R)*W per row"
    );
}

#[test]
fn fused_add_rmsnorm_agrees_across_the_tree_walker_and_vm() {
    assert_eq!(
        tw_outcome(FUSED_ADD).output.trim(),
        vm_outcome(FUSED_ADD).output.trim(),
        "tw ≡ VM"
    );
}

#[test]
fn fused_add_rmsnorm_is_recognized_and_lowered_by_both_accelerators() {
    assert!(
        compile_to_cuda_tensor_op(FUSED_ADD).is_ok(),
        "CUDA must recognize+lower fused-add rmsnorm"
    );
    assert!(
        compile_to_tenstorrent_tensor_op(FUSED_ADD).is_ok(),
        "Tenstorrent must recognize+lower fused-add rmsnorm"
    );
    // The CUDA kernel genuinely fuses the residual add (a `res` input, `xr[i]+rr[i]` reduced) AND the weight.
    let cu = compile_to_cuda_tensor_op(FUSED_ADD).unwrap();
    assert!(
        cu.contains("const float* res"),
        "fused-add kernel takes a residual input: {cu}"
    );
    assert!(cu.contains("(xr[i]+rr[i])"), "the reduced value is X+R");
    assert!(cu.contains("*scale*w[i]"), "…then per-column weighted");
    assert!(
        cu.contains("#define ROWS 2") && cu.contains("#define COLS 3"),
        "recognized the 2x3 shape"
    );
}

#[test]
fn weighted_rmsnorm_is_recognized_and_lowered_by_both_accelerators() {
    // The cross-backend ratchet made concrete: both accelerators recognize the per-row WEIGHTED shape and
    // emit a kernel — the whole reason a new tensor word is allowed into tensor_op_coverage_lock.
    assert!(
        compile_to_cuda_tensor_op(WEIGHTED).is_ok(),
        "CUDA must recognize+lower weighted rmsnorm"
    );
    assert!(
        compile_to_tenstorrent_tensor_op(WEIGHTED).is_ok(),
        "Tenstorrent must recognize+lower weighted rmsnorm"
    );
    // …and the CUDA kernel is genuinely PER-ROW (rows=2, not flattened) and genuinely WEIGHTED.
    let cu = compile_to_cuda_tensor_op(WEIGHTED).unwrap();
    assert!(
        cu.contains("#define ROWS 2") && cu.contains("#define COLS 3"),
        "recognized the 2x3 shape: {cu}"
    );
    assert!(
        cu.contains("const float* w") && cu.contains("*scale*w[i]"),
        "weight applied per column in the rescale"
    );
    // ★ ASK THE IR, NOT THE TEXT. This used to be `tt.contains("per-row rmsnorm (weighted)")` — a
    // COMMENT in the retired template's header. That template declared `cb_w` and never read it, so the
    // assertion "the Tensix kernel is the per-row weighted reducer" was satisfied by the kernel SAYING
    // SO, on a kernel that did not do it. Grepping for an instruction name instead would move the same
    // hole one level down: a comment can contain `mul_tiles_bcast_rows` just as easily as a sentence.
    //
    // These are facts computed from the lowered program's iterator kinds and access maps
    // (`spatial::describe`) — the decisions the compiler actually made. A per-column weight IS an
    // access map that omits the row axis. No printed text can produce or suppress one.
    let p = compile_to_spatial(WEIGHTED, &SpatialTarget::wormhole())
        .expect("Tenstorrent must recognize+lower weighted rmsnorm");
    let facts = compute_facts(&p);
    assert!(
        facts.reductions >= 1,
        "weighted rmsnorm must FOLD a row axis (facts: {facts:?})"
    );
    assert!(
        facts.rescales_by_a_per_row_scalar(),
        "the reduced scalar must be broadcast back across its row — the rescale (facts: {facts:?})"
    );
    assert!(
        facts.applies_a_per_column_operand(),
        "★ the WEIGHT must actually be applied: a per-column operand broadcast down the rows. This is \
         the assertion the retired template passed while ignoring `cb_w` entirely (facts: {facts:?})"
    );
    assert!(
        facts.uses(SfpuKind::Rsqrt {
            eps_nano: 1_000,
            over: 3
        }),
        "the RMS rescale (facts: {facts:?})"
    );
    // The emitted kernel still has to exist and be the compute one; the facts above say what it does.
    let tt = compile_to_tenstorrent_tensor_op(WEIGHTED).unwrap();
    assert!(
        tt.contains("kernel_main"),
        "a compute kernel is emitted: {tt}"
    );
}

#[test]
fn weighted_rmsnorm_matches_the_vm_on_aot_rust() {
    // A genuine native build+run — the desugared per-row loops reach the AOT-Rust tier ≡ the VM.
    match run_logos_source(WEIGHTED) {
        Ok(out) => assert_eq!(
            out.trim(),
            vm_outcome(WEIGHTED).output.trim(),
            "AOT-Rust ≡ VM"
        ),
        Err(e) => panic!("AOT-Rust must build+run weighted rmsnorm: {e}"),
    }
}

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn weighted_rmsnorm_matches_the_vm_on_the_c_backend() {
    if !have_gcc() {
        eprintln!("SKIP C: no gcc.");
        return;
    }
    let c = compile_to_c(WEIGHTED).expect("must lower to C");
    let dir = std::env::temp_dir().join(format!("logos_frf_{}", std::process::id()));
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
        vm_outcome(WEIGHTED).output.trim(),
        "AOT-C ≡ VM on weighted rmsnorm"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
