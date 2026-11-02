//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! EVERY NORM FORM THE FRONT END LIFTS, COMPUTED ON THE GPU AS THE VM COMPUTES IT.
//!
//! The front end lifts four row-reduce forms: RMSNorm (weightless, weighted, and weighted with a fused residual
//! `rmsnorm of X plus R with weight W`), the row softmax, and LayerNorm (`layernorm of X with weight W and bias B`).
//! The CUDA forward's row kernel used to read only `X` and the weight: a fused residual was dropped, and a
//! LayerNorm reached a `todo!()`. The `add → rmsnorm` fusion likewise fired on a norm carrying its own residual and
//! dropped it. Each form here runs on the GPU and is compared, element by element, against the VM running the
//! same program (f32 on the device, f64 in the VM, hence the tolerance).
//!
//! Also held: a `silu` stage's launch covers exactly its elements — the lane's own walk read the `<=` loop bound
//! as `<` and launched one element past the end.
//!
//! The GPU legs need `nvcc` and a device; `LOGOS_REQUIRE_TOOLS=nvcc,gpu` makes their absence a failure.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::process::Command;

use logicaffeine_compile::compile::{compile_to_cuda_forward, vm_outcome};

fn gpu_ready() -> bool {
    common::require_tool("nvcc") && common::require_gpu()
}

/// Compile the emitted program with nvcc for this box's GPU and run it, returning its printed numbers.
fn nvcc_run(cu: &str, tag: &str) -> Vec<f64> {
    let dir = std::env::temp_dir().join(format!("logos_norm_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
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
        "the emitted forward must compile:\n{}\n--- cu ---\n{cu}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = Command::new(&bin).output().expect("run forward");
    assert!(
        r.status.success(),
        "the emitted forward failed:\n{}",
        String::from_utf8_lossy(&r.stderr)
    );
    let out = String::from_utf8_lossy(&r.stdout)
        .split_whitespace()
        .map(|s| s.parse().unwrap_or_else(|_| panic!("non-numeric output `{s}`")))
        .collect();
    std::fs::remove_dir_all(&dir).unwrap();
    out
}

fn vm_numbers(src: &str) -> Vec<f64> {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
    vm.output
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect()
}

/// Every element of the device result within f32 rounding of the VM's f64 result.
fn assert_matches_vm(got: &[f64], want: &[f64], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: element count");
    for (k, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(
            (g - w).abs() <= 2e-4 * w.abs().max(1.0),
            "{what}: element {k} is {g} on the GPU and {w} in the VM"
        );
    }
}

/// A declared `rows × cols` Real tensor filled with a fixed spread of values (no two rows alike).
fn tensor(name: &str, rows: usize, cols: usize, salt: usize) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for p in 0..rows * cols {
        let v = (((p * 37 + salt * 11) % 101) as f64 - 50.0) / 25.0;
        s.push_str(&format!("Set item {} of {name} to {v:?}.\n", p + 1));
    }
    s
}

fn show_all(name: &str, n: usize) -> String {
    (1..=n).map(|p| format!("Show item {p} of {name}.\n")).collect()
}

#[test]
fn a_residual_rmsnorm_lowers_and_matches_the_vm_on_the_gpu() {
    let (rows, cols) = (2, 64);
    let src = format!(
        "## Main\n{}{}{}Let Y be rmsnorm of X plus R with weight W.\n{}",
        tensor("X", rows, cols, 1),
        tensor("R", rows, cols, 2),
        tensor("W", 1, cols, 3),
        show_all("Y", rows * cols)
    );
    let cu = compile_to_cuda_forward(&src).unwrap_or_else(|e| panic!("must lower: {e}"));
    assert!(
        cu.contains("rr[i]") && cu.contains("buf_R"),
        "the row kernel must read the residual:\n{cu}"
    );
    if !gpu_ready() {
        return;
    }
    assert_matches_vm(
        &nvcc_run(&cu, "resrms"),
        &vm_numbers(&src),
        "rmsnorm of X plus R with weight W",
    );
}

#[test]
fn a_layernorm_lowers_and_matches_the_vm_on_the_gpu() {
    let (rows, cols) = (3, 48);
    let src = format!(
        "## Main\n{}{}{}Let Y be layernorm of X with weight W and bias B.\n{}",
        tensor("X", rows, cols, 4),
        tensor("W", 1, cols, 5),
        tensor("B", 1, cols, 6),
        show_all("Y", rows * cols)
    );
    let cu = compile_to_cuda_forward(&src).unwrap_or_else(|e| panic!("must lower: {e}"));
    assert!(
        cu.contains("mean") && cu.contains("bias[i]"),
        "the row kernel must center by the mean and shift by the bias:\n{cu}"
    );
    if !gpu_ready() {
        return;
    }
    assert_matches_vm(
        &nvcc_run(&cu, "layernorm"),
        &vm_numbers(&src),
        "layernorm of X with weight W and bias B",
    );
}

/// `S = A + B` followed by a norm of `S` that carries its OWN residual: the `add → rmsnorm` fusion computes
/// `rmsnorm(A + B)` and has no operand for the residual, so it must not fire.
#[test]
fn a_sum_feeding_a_residual_rmsnorm_is_not_fused_and_matches_the_vm() {
    let (rows, cols) = (2, 32);
    let src = format!(
        "## Main\n{}{}{}{}Let S be A plus B.\nLet Y be rmsnorm of S plus R with weight W.\n{}",
        tensor("A", rows, cols, 7),
        tensor("B", rows, cols, 8),
        tensor("R", rows, cols, 9),
        tensor("W", 1, cols, 10),
        show_all("Y", rows * cols)
    );
    let cu = compile_to_cuda_forward(&src).unwrap_or_else(|e| panic!("must lower: {e}"));
    assert!(
        !cu.contains("_add_row"),
        "the add must not fuse into a norm that carries its own residual:\n{cu}"
    );
    if !gpu_ready() {
        return;
    }
    assert_matches_vm(
        &nvcc_run(&cu, "addres"),
        &vm_numbers(&src),
        "S = A + B; rmsnorm of S plus R with weight W",
    );
}

/// The control for the guard above: a sum feeding a plain weighted norm still fuses.
#[test]
fn a_sum_feeding_a_plain_rmsnorm_still_fuses() {
    let (rows, cols) = (2, 32);
    let src = format!(
        "## Main\n{}{}{}Let S be A plus B.\nLet Y be rmsnorm of S with weight W.\n{}",
        tensor("A", rows, cols, 7),
        tensor("B", rows, cols, 8),
        tensor("W", 1, cols, 10),
        show_all("Y", rows * cols)
    );
    let cu = compile_to_cuda_forward(&src).unwrap_or_else(|e| panic!("must lower: {e}"));
    assert!(
        cu.contains("k0_add_row"),
        "a sum feeding a plain norm fuses into one add_row kernel:\n{cu}"
    );
}

/// A `silu` over 5 elements launches over 5 elements — not 6.
#[test]
fn a_silu_launch_covers_exactly_its_elements_and_matches_the_vm() {
    let src = format!(
        "## Main\n{}Let Y be silu of X.\n{}",
        tensor("X", 1, 5, 11),
        show_all("Y", 5)
    );
    let cu = compile_to_cuda_forward(&src).unwrap_or_else(|e| panic!("must lower: {e}"));
    assert!(
        cu.contains("(buf_X,buf_Y,5)"),
        "the silu launch must cover exactly the 5 elements:\n{cu}"
    );
    if !gpu_ready() {
        return;
    }
    assert_matches_vm(&nvcc_run(&cu, "silu"), &vm_numbers(&src), "silu of X");
}
