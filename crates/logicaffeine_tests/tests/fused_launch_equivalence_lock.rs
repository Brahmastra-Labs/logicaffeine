//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ★ THE FUSION-EQUIVALENCE LOCK ★ — every launch-saving fused kernel must be OBSERVATIONALLY IDENTICAL
//! to the sequence of kernels it replaces, on the same inputs, on a real GPU.
//!
//! Batch-1 decode is per-launch-latency bound, so the way to go faster is to fuse: a residual add folded into
//! the norm that follows it, an L2 normalize folded into the conv that feeds it, an activation quantization
//! folded into the gate-norm that produces its input. Every one of those is a chance to silently change the
//! numbers — and at 27 B a changed number is a flipped argmax, twelve tokens later, in a sentence that still
//! reads fluently. This test is what makes the fusions safe to keep making: it runs BOTH paths in one program
//! and demands they agree to the last bit that float determinism allows.
//!
//! It is a *differential* test, not an oracle test. The unfused path is already pinned to a clean-room f64
//! reference by `gated_delta_mechanism_lock`; this pins the fast path to the slow one. Together they chain the
//! fused kernels all the way back to the mathematics.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::path::Path;
use std::process::Command;

use logicaffeine_compile::codegen_cuda::gdn::{GDN_KERNELS, QWEN38_KERNELS};
use logicaffeine_compile::codegen_cuda::qwen38::QWEN38_EXTRA_KERNELS;
use logicaffeine_compile::codegen_cuda::qwen_emit::GEN_DEVICE_KERNELS;

fn have_nvcc() -> bool {
    common::require_tool("nvcc")
}
fn have_gpu() -> bool {
    common::require_gpu()
}

/// Deterministic pseudo-weights — the same generator the mechanism lock uses, so a failure here can be
/// reproduced there by hand.
fn wv(salt: usize, i: usize) -> f64 {
    let x = ((salt * 7919 + i * 104729) % 20011) as f64 / 20011.0;
    (x - 0.5) * 2.0
}

fn arr(name: &str, v: &[f64]) -> String {
    let body: Vec<String> = v.iter().map(|x| format!("{:.9}f", x)).collect();
    format!(
        "__device__ __managed__ float {name}[{}] = {{{}}};\n",
        v.len().max(1),
        body.join(",")
    )
}

/// Compile and run a device program built on the SHIPPED kernel sources — no re-derivation, no copy. If the
/// kernels change, this test compiles the changed ones.
fn run(body: &str, decls: &str, tag: &str) -> Vec<f64> {
    let src = format!(
        "#include <cstdio>\n#include <cmath>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {gdn}\n{q38}\n{gen}\n{extra}\n{decls}\n\
         int main(){{\n{body}\n  cudaDeviceSynchronize();\n  return 0;\n}}\n",
        gdn = GDN_KERNELS,
        q38 = QWEN38_KERNELS,
        gen = GEN_DEVICE_KERNELS,
        extra = QWEN38_EXTRA_KERNELS,
    );
    let dir = std::env::temp_dir().join(format!("logos_fuse_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let (src_p, bin_p) = (dir.join("f.cu"), dir.join("f"));
    std::fs::write(&src_p, &src).expect("write .cu");
    let sm = std::env::var("LOGOS_SM").unwrap_or_else(|_| "sm_86".to_string());
    let comp = Command::new("nvcc")
        .args(["-O3", &format!("-arch={sm}")])
        .arg(&src_p)
        .arg("-o")
        .arg(&bin_p)
        .output()
        .expect("run nvcc");
    assert!(
        comp.status.success(),
        "[{tag}] nvcc failed:\n{}",
        String::from_utf8_lossy(&comp.stderr)
    );
    let out = Command::new(&bin_p).output().expect("run binary");
    assert!(
        out.status.success(),
        "[{tag}] run failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Vec<f64> = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(|t| t.parse().expect("float"))
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    v
}

/// The two paths run in ONE program on the SAME inputs, so anything that differs is the fusion, not the
/// environment. Float reassociation is permitted (a warp shuffle sums in a different order than a block
/// reduction), so the bar is 1e-6 relative — orders of magnitude tighter than the int8 activation grid that
/// separates any two honest implementations of this model.
fn assert_paths_agree(tag: &str, got: &[f64], n: usize) {
    assert_eq!(
        got.len(),
        2 * n,
        "[{tag}] expected the fused and unfused halves"
    );
    let (fused, unfused) = got.split_at(n);
    let mut worst = 0.0f64;
    let mut worst_i = 0usize;
    for (i, (a, b)) in fused.iter().zip(unfused.iter()).enumerate() {
        let d = (a - b).abs() / b.abs().max(1e-3);
        if d > worst {
            worst = d;
            worst_i = i;
        }
    }
    assert!(worst < 1e-6,
        "[{tag}] the fused kernel is NOT the sequence it replaces: worst relative difference {worst:.3e} at \
         element {worst_i} (fused {}, unfused {}). A fusion that changes the numbers changes the tokens.",
        fused[worst_i], unfused[worst_i]);
    println!("[{tag}] fused == unfused over {n} outputs (worst rel {worst:.2e})");
}

/// `gk_conv_silu_l2` must equal `gk_conv_silu` then `gk_l2norm_heads` — including the fact that the value
/// region past the first `nl2` head-blocks is NOT normalized. Normalizing v as well would be the easy bug,
/// and it would still produce a fluent model.
#[test]
fn the_conv_that_carries_the_l2_normalize_equals_the_two_kernels_it_replaces() {
    if !have_nvcc() || !have_gpu() {
        return;
    }
    for &(d, nk, nv, taps) in &[
        (32usize, 2usize, 4usize, 4usize),
        (128, 16, 48, 4),
        (64, 4, 4, 3),
    ] {
        let c = 2 * nk * d + nv * d;
        let x: Vec<f64> = (0..c).map(|i| wv(201, i)).collect();
        let wc: Vec<f64> = (0..taps * c).map(|i| wv(202, i)).collect();
        let cs: Vec<f64> = (0..taps * c).map(|i| wv(203, i)).collect();
        let decls = format!(
            "{}{}{}{}",
            arr("X", &x),
            arr("WC", &wc),
            arr("CS0", &cs),
            format!(
                "__device__ __managed__ float CSa[{}], CSb[{}], YA[{c}], YB[{c}];\n",
                taps * c,
                taps * c
            )
        );
        let body = format!(
            "  for(int i=0;i<{tc};++i){{ CSa[i]=CS0[i]; CSb[i]=CS0[i]; }}
  gk_conv_silu_l2<<<{c}/{d},{d}>>>(X,WC,CSa,YA,{c},{taps},{nl2},1e-6f);
  cudaDeviceSynchronize();
  gk_conv_silu<<<({c}+255)/256,256>>>(X,WC,CSb,YB,{c},{taps});
  cudaDeviceSynchronize();
  gk_l2norm_heads<<<{nl2},{d}>>>(YB,{d},1e-6f);
  cudaDeviceSynchronize();
  for(int i=0;i<{c};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{c};++i) printf(\"%.9g \", YB[i]);
  // the carried ring must advance identically too — it is this layer's state for the NEXT token
  for(int i=0;i<{tc};++i) printf(\"%.9g \", CSa[i]);
  for(int i=0;i<{tc};++i) printf(\"%.9g \", CSb[i]);\n",
            tc = taps * c,
            nl2 = 2 * nk
        );
        let got = run(&body, &decls, &format!("conv{d}_{nk}_{nv}"));
        let n = c + taps * c;
        let mut inter = Vec::with_capacity(2 * n);
        inter.extend_from_slice(&got[0..c]);
        inter.extend_from_slice(&got[2 * c..2 * c + taps * c]);
        inter.extend_from_slice(&got[c..2 * c]);
        inter.extend_from_slice(&got[2 * c + taps * c..]);
        assert_paths_agree(&format!("conv+l2 D={d} NK={nk} NV={nv}"), &inter, n);
    }
}

/// `gdn_gate_norm_q` must equal `gdn_gate_norm` then `gk_quant_act` — both the f32 output AND the int8 codes
/// and their per-32 scales. A group boundary off by one here is invisible in the float output and corrupts
/// every subsequent projection.
#[test]
fn the_gate_norm_that_carries_the_quantization_equals_the_two_kernels_it_replaces() {
    if !have_nvcc() || !have_gpu() {
        return;
    }
    for &(d, nv) in &[(32usize, 4usize), (128, 48), (64, 8)] {
        let n = nv * d;
        let z: Vec<f64> = (0..n).map(|i| wv(301, i)).collect();
        let o: Vec<f64> = (0..n).map(|i| wv(302, i)).collect();
        let gn: Vec<f64> = (0..d).map(|i| 1.0 + wv(303, i)).collect();
        let decls = format!(
            "{}{}{}{}",
            arr("Z", &z),
            arr("O", &o),
            arr("GN", &gn),
            format!(
                "__device__ __managed__ float OA[{n}], OB[{n}], SA[{}], SB[{}];\n\
                     __device__ __managed__ signed char QA[{n}], QB[{n}];\n",
                n / 32,
                n / 32
            )
        );
        let body = format!(
            "  gdn_gate_norm_q<<<{nv},{d}>>>(Z,GN,O,OA,QA,SA,{d},1e-6f);
  cudaDeviceSynchronize();
  gdn_gate_norm<<<{nv},{d}>>>(Z,GN,O,OB,{d},1e-6f);
  cudaDeviceSynchronize();
  gk_quant_act<<<({n}/32+7)/8,256>>>(OB,QB,SB,{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{n};++i) printf(\"%.9g \", OA[i]);
  for(int i=0;i<{ng};++i) printf(\"%.9g \", SA[i]);
  for(int i=0;i<{n};++i) printf(\"%d \", (int)QA[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", OB[i]);
  for(int i=0;i<{ng};++i) printf(\"%.9g \", SB[i]);
  for(int i=0;i<{n};++i) printf(\"%d \", (int)QB[i]);\n",
            ng = n / 32
        );
        let got = run(&body, &decls, &format!("gate{d}_{nv}"));
        assert_paths_agree(
            &format!("gate_norm+quant D={d} NV={nv}"),
            &got,
            n + n / 32 + n,
        );
    }
}

/// `gk_add_rmsnorm_q8f` must equal `gk_add` then `gk_rmsnorm_q8f`, and must leave the residual `x` updated in
/// place — the next layer reads it. Forgetting the in-place store is the fusion bug that silently drops every
/// residual connection in the model.
#[test]
fn the_entry_norm_that_carries_the_residual_add_equals_the_two_kernels_it_replaces() {
    if !have_nvcc() || !have_gpu() {
        return;
    }
    for &n in &[512usize, 5120, 1024] {
        let x: Vec<f64> = (0..n).map(|i| wv(401, i)).collect();
        let y: Vec<f64> = (0..n).map(|i| wv(402, i)).collect();
        let w: Vec<f64> = (0..n).map(|i| 1.0 + wv(403, i)).collect();
        let decls = format!("{}{}{}{}", arr("X0", &x), arr("Y", &y), arr("W", &w),
            format!("__device__ __managed__ float XA[{n}], XB[{n}], FA[{n}], FB[{n}], SA[{}], SB[{}];\n\
                     __device__ __managed__ signed char QA[{n}], QB[{n}];\n", n / 32, n / 32));
        let body = format!(
            "  for(int i=0;i<{n};++i){{ XA[i]=X0[i]; XB[i]=X0[i]; }}
  gk_add_rmsnorm_q8f<<<1,256>>>(XA,Y,QA,SA,FA,W,{n},1e-6f);
  cudaDeviceSynchronize();
  gk_add<<<({n}+255)/256,256>>>(XB,Y,{n});
  cudaDeviceSynchronize();
  gk_rmsnorm_q8f<<<1,256>>>(XB,QB,SB,FB,W,{n},1e-6f);
  cudaDeviceSynchronize();
  for(int i=0;i<{n};++i) printf(\"%.9g \", FA[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", XA[i]);
  for(int i=0;i<{ng};++i) printf(\"%.9g \", SA[i]);
  for(int i=0;i<{n};++i) printf(\"%d \", (int)QA[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", FB[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", XB[i]);
  for(int i=0;i<{ng};++i) printf(\"%.9g \", SB[i]);
  for(int i=0;i<{n};++i) printf(\"%d \", (int)QB[i]);\n",
            ng = n / 32
        );
        let got = run(&body, &decls, &format!("addnorm{n}"));
        assert_paths_agree(&format!("add+rmsnorm_q8f n={n}"), &got, 3 * n + n / 32);
    }
}

/// Two output rows per warp must be BIT-IDENTICAL to one row per warp: the widening changes how many rows a
/// warp is responsible for, not the order in which any single row's products are summed. Anything else means
/// the widening quietly changed the arithmetic, and on a 27 B model a changed sum is a changed token.
#[test]
fn two_rows_per_warp_is_bit_identical_to_one_row_per_warp() {
    if !have_nvcc() || !have_gpu() {
        return;
    }
    for &(k, n) in &[(512usize, 64usize), (5120, 128), (1024, 33)] {
        let wq: Vec<f64> = (0..k * n).map(|i| (wv(501, i) * 100.0).round()).collect();
        let ws: Vec<f64> = (0..(k / 32) * n)
            .map(|i| 0.01 + wv(502, i).abs() * 0.01)
            .collect();
        let xq: Vec<f64> = (0..k).map(|i| (wv(503, i) * 100.0).round()).collect();
        let xs: Vec<f64> = (0..k / 32)
            .map(|i| 0.01 + wv(504, i).abs() * 0.01)
            .collect();
        let decls = format!(
            "{}{}{}{}{}",
            arr("WQF", &wq),
            arr("WSF", &ws),
            arr("XQF", &xq),
            arr("XS", &xs),
            format!(
                "__device__ __managed__ signed char WQ[{}], XQ[{k}];\n\
                     __device__ __managed__ __half WS[{}];\n\
                     __device__ __managed__ float YA[{n}], YB[{n}];\n",
                k * n,
                (k / 32) * n
            )
        );
        let body = format!(
            "  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{k};++i) XQ[i]=(signed char)XQF[i];
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  cudaDeviceSynchronize();
  gk_gemv_h<<<Gw_({n}),256>>>(YA,WQ,WS,XQ,XS,{k},{n});
  cudaDeviceSynchronize();
  gk_gemv_h2<<<Gw2_({n}),256>>>(YB,WQ,WS,XQ,XS,{k},{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{n};++i) printf(\"%.9g \", YB[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", YA[i]);\n",
            kn = k * n,
            nb = (k / 32) * n
        );
        let decls = format!(
            "{decls}static inline int Gw_(int m){{ return (m*32+255)/256; }}\n\
                            static inline int Gw2_(int m){{ return (((m+1)/2)*32+255)/256; }}\n"
        );
        let got = run(&body, &decls, &format!("wide{k}_{n}"));
        assert_eq!(got.len(), 2 * n, "expected both halves");
        for (i, (a, b)) in got[..n].iter().zip(got[n..].iter()).enumerate() {
            assert_eq!(a, b, "row {i} of the widened GEMV differs from the one-row form ({a} vs {b}) at K={k} N={n}");
        }
        println!("[wide K={k} N={n}] two rows per warp == one row per warp, exactly");
    }
}

/// The two-pass activation split must reconstruct the f32 activation to within its design bound: the whole
/// point of the second int8 word is that `a ~= s*(qh + ql/127)` is 254x tighter than one word, so the GEMV
/// that consumes it must land within ~1e-4 of the one that reads the f32 vector directly. Being merely "close"
/// is not enough — a lane that measured no better than single-word is exactly how a silent wiring mistake
/// (a stale low word) presents itself.
#[test]
fn the_two_pass_activation_split_recovers_the_f32_gemv() {
    if !have_nvcc() || !have_gpu() {
        return;
    }
    for &(k, n) in &[(512usize, 64usize), (5120, 96)] {
        let wq: Vec<f64> = (0..k * n).map(|i| (wv(601, i) * 100.0).round()).collect();
        let ws: Vec<f64> = (0..(k / 32) * n)
            .map(|i| 0.01 + wv(602, i).abs() * 0.01)
            .collect();
        let x: Vec<f64> = (0..k).map(|i| wv(603, i) * 3.0).collect();
        let decls = format!(
            "{}{}{}{}",
            arr("WQF", &wq),
            arr("WSF", &ws),
            arr("X", &x),
            format!(
                "__device__ __managed__ signed char WQ[{}], XH[{k}], XL[{k}];\n\
                     __device__ __managed__ __half WS[{}];\n\
                     __device__ __managed__ float XS[{}], YA[{n}], YB[{n}];\n",
                k * n,
                (k / 32) * n,
                k / 32
            )
        );
        let body = format!(
            "  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  cudaDeviceSynchronize();
  gk_quant_act_hl<<<({k}/32+7)/8,256>>>(X,XH,XL,XS,{k});
  cudaDeviceSynchronize();
  gk_gemv_hl<<<Gw_({n}),256>>>(YA,WQ,WS,XH,XL,XS,{k},{n});
  cudaDeviceSynchronize();
  gk_gemv_deq_h<<<Gw_({n}),256>>>(YB,WQ,WS,X,{k},{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{n};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", YB[i]);\n",
            kn = k * n,
            nb = (k / 32) * n
        );
        let decls = format!("{decls}static inline int Gw_(int m){{ return (m*32+255)/256; }}\n");
        let got = run(&body, &decls, &format!("hl{k}_{n}"));
        assert_eq!(got.len(), 2 * n);
        let (hl, exact) = got.split_at(n);
        let mut worst = 0.0f64;
        for (a, b) in hl.iter().zip(exact.iter()) {
            let d = (a - b).abs() / b.abs().max(1e-3);
            if d > worst {
                worst = d;
            }
        }
        assert!(worst < 1e-4,
            "the two-pass split should recover the f32 GEMV to ~1e-4; worst relative difference was {worst:.3e} \
             at K={k} N={n}. A stale or mismatched low word looks exactly like this.");
        println!("[two-pass K={k} N={n}] recovers the f32 GEMV (worst rel {worst:.2e})");
    }
}

/// Split-K must agree with the rolled GEMV to float-reassociation tolerance, and — separately and more
/// importantly — it must be DETERMINISTIC: the same inputs must give the same bits every run.
///
/// Determinism is the reason the combine is a second kernel summing partials in fixed ascending order rather
/// than an `atomicAdd`. Float addition is not associative, so an atomic makes the result depend on which
/// block happens to land first, and the same prompt could then produce different tokens on different runs of
/// the same binary. A model whose output depends on scheduler order is not a model you can test.
#[test]
fn split_k_agrees_with_the_rolled_gemv_and_is_run_to_run_deterministic() {
    if !have_nvcc() || !have_gpu() {
        return;
    }
    for &(k, n, splits) in &[
        (6144usize, 512usize, 4usize),
        (17408, 256, 4),
        (5120, 128, 2),
    ] {
        let wq: Vec<f64> = (0..k * n).map(|i| (wv(1301, i) * 100.0).round()).collect();
        let ws: Vec<f64> = (0..(k / 32) * n)
            .map(|i| 0.01 + wv(1302, i).abs() * 0.01)
            .collect();
        let x: Vec<f64> = (0..k).map(|i| wv(1303, i) * 3.0).collect();
        let decls = format!(
            "{}{}{}{}",
            arr("WQF", &wq),
            arr("WSF", &ws),
            arr("X", &x),
            format!(
                "__device__ __managed__ signed char WQ[{}], XH[{k}], XL[{k}];\n\
                     __device__ __managed__ __half WS[{}];\n\
                     __device__ __managed__ float XS[{}], PART[{}], YA[{n}], YB[{n}], YC[{n}];\n\
                     static inline int Gw_(int m){{ return (m*32+255)/256; }}\n\
                     static inline int Gs_(int m){{ return (m+255)/256; }}\n",
                k * n,
                (k / 32) * n,
                k / 32,
                splits * n
            )
        );
        let body = format!(
            "  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  cudaDeviceSynchronize();
  gk_quant_act_hl<<<({k}/32+7)/8,256>>>(X,XH,XL,XS,{k});
  cudaDeviceSynchronize();
  gk_gemv_hl<<<Gw_({n}),256>>>(YA,WQ,WS,XH,XL,XS,{k},{n});
  cudaDeviceSynchronize();
  gk_gemv_hl_sk<<<dim3(Gw_({n}),{splits}),256>>>(PART,WQ,WS,XH,XL,XS,{k},{n},{splits});
  cudaDeviceSynchronize();
  gk_sk_combine<<<Gs_({n}),256>>>(YB,PART,{n},{splits});
  cudaDeviceSynchronize();
  // run it a second time: determinism, not just agreement
  gk_gemv_hl_sk<<<dim3(Gw_({n}),{splits}),256>>>(PART,WQ,WS,XH,XL,XS,{k},{n},{splits});
  cudaDeviceSynchronize();
  gk_sk_combine<<<Gs_({n}),256>>>(YC,PART,{n},{splits});
  cudaDeviceSynchronize();
  for(int i=0;i<{n};++i) printf(\"%.9g \", YB[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", YC[i]);\n",
            kn = k * n,
            nb = (k / 32) * n
        );
        let got = run(&body, &decls, &format!("sk{k}_{n}_{splits}"));
        assert_eq!(
            got.len(),
            3 * n,
            "expected split-K, rolled, and the split-K repeat"
        );
        let (sk, rest) = got.split_at(n);
        let (rolled, sk2) = rest.split_at(n);

        for (i, (a, b)) in sk.iter().zip(sk2.iter()).enumerate() {
            assert_eq!(
                a, b,
                "split-K is NOT deterministic at element {i} (K={k} N={n} S={splits}): \
                {a} then {b}. The combine must sum partials in a fixed order."
            );
        }
        let mut worst = 0.0f64;
        for (a, b) in sk.iter().zip(rolled.iter()) {
            let d = (a - b).abs() / b.abs().max(1e-3);
            if d > worst {
                worst = d;
            }
        }
        assert!(worst < 1e-5,
            "split-K should differ from the rolled GEMV only by float re-association; worst relative \
             difference {worst:.3e} at K={k} N={n} S={splits} is too large to be association alone");
        println!("[split-K K={k} N={n} S={splits}] deterministic, and within {worst:.2e} of the rolled GEMV");
    }
}
