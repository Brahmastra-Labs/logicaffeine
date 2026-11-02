use super::*;

/// The emitted GEMM program is well-formed: DP4A intrinsic, the kernel, baked data, and a host that
/// prints the result. (On-GPU numeric ≡ `eval_systolic` is `cuda_matrix_lock`, gated on nvcc.)
#[test]
fn emits_a_wellformed_dp4a_gemm() {
    let (w, a) = sample_gemm_data(4, 8, 4);
    let art = emit_int8_gemm(4, 8, 4, &w, &a).expect("int8 WS GEMM is a certified capability");
    let cu = &art.kernel_cu;
    assert!(cu.contains("__dp4a("), "must use the DP4A int8 intrinsic");
    assert!(cu.contains("__global__ void gemm_i8"), "kernel present");
    assert!(cu.contains("#define KP 8"), "K=8 needs no padding");
    assert!(cu.contains("cudaMemcpy"), "host reads the result back");
    assert!(art.makefile.contains("nvcc"));
}

/// K not a multiple of 4 pads up cleanly (pad lanes are 0 → result unchanged).
#[test]
fn pads_k_to_a_multiple_of_four() {
    let (w, a) = sample_gemm_data(2, 5, 2);
    let art = emit_int8_gemm(2, 5, 2, &w, &a).expect("gemm");
    assert!(art.kernel_cu.contains("#define KP 8"), "K=5 pads to KP=8");
}

/// The block-scaled int8 GEMM (what the DP4A kernel computes) is byte-for-byte the full dequant GEMM —
/// the scale-indexing is correct. Distinct per-block scales, so a wrong block index would diverge.
#[test]
fn block_scaled_int8_matches_full_dequant() {
    let (m, k, n) = (3usize, 64usize, 5usize);
    let nb = k / 32;
    let wq: Vec<i32> = (0..n * k)
        .map(|i| ((i as i32 * 7 + 3).rem_euclid(255)) - 128)
        .collect();
    let aq: Vec<i32> = (0..m * k)
        .map(|i| ((i as i32 * 5 + 1).rem_euclid(255)) - 128)
        .collect();
    let ws: Vec<f32> = (0..n * nb).map(|i| 0.01 + (i % 7) as f32 * 0.003).collect();
    let as_: Vec<f32> = (0..m * nb).map(|i| 0.02 + (i % 5) as f32 * 0.004).collect();
    let a = blockscaled_gemm_int8(m, k, n, &wq, &ws, &aq, &as_);
    let b = blockscaled_gemm_dequant(m, k, n, &wq, &ws, &aq, &as_);
    assert_eq!(a.len(), m * n);
    for (x, y) in a.iter().zip(&b) {
        assert!(
            (x - y).abs() <= 1e-9 * x.abs().max(1.0),
            "block-scaled {x} != dequant {y}"
        );
    }
}

/// With unit scales the block-scaled GEMM is exactly the integer int8 dot (ties the scaled path to the
/// certified raw-int8 semantics).
#[test]
fn unit_scales_reduce_to_the_integer_dot() {
    let (m, k, n) = (2usize, 32usize, 3usize);
    let wq: Vec<i32> = (0..n * k).map(|i| (i as i32 % 7) - 3).collect();
    let aq: Vec<i32> = (0..m * k).map(|i| (i as i32 % 5) - 2).collect();
    let ones_w = vec![1.0f32; n * (k / 32)];
    let ones_a = vec![1.0f32; m * (k / 32)];
    let scaled = blockscaled_gemm_int8(m, k, n, &wq, &ones_w, &aq, &ones_a);
    for i in 0..m {
        for o in 0..n {
            let dot: i64 = (0..k)
                .map(|t| aq[i * k + t] as i64 * wq[o * k + t] as i64)
                .sum();
            assert_eq!(scaled[i * n + o], dot as f64);
        }
    }
}
