//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ★ THE BATCHED-EQUIVALENCE LOCK ★ — at batch 1, every batched kernel must compute exactly what the
//! single-sequence kernel it was derived from computes.
//!
//! This exists because of a specific mistake worth not repeating. The single-sequence 27B lane is GOLDEN —
//! byte-exact against llama.cpp for 13 tokens — which makes its kernels an oracle. Writing the batched
//! versions by retyping them from memory threw that away: `gk_gate_mul` is a SIGMOID gate
//! (`ctx *= 1/(1+e^-g)`) and the retyped version was a SiLU (`g/(1+e^-g)`), a different function, corrupting
//! all 16 attention layers. It cost a rented H100 session to find something a tiny local test decides in
//! seconds on a 12 GB card.
//!
//! The batched program's own `BATCH_AGREEMENT` self-check cannot catch this: a systematic error is identical
//! across sequences, so it happily reports "32/32 sequences identical" while all 32 are wrong. That check
//! tests sequence INDEPENDENCE; this one tests CORRECTNESS. Both are needed, and they are not the same
//! property.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::path::Path;
use std::process::Command;

use logicaffeine_compile::codegen_cuda::gdn::{GDN_KERNELS, QWEN38_KERNELS};
use logicaffeine_compile::codegen_cuda::qwen38::QWEN38_EXTRA_KERNELS;
use logicaffeine_compile::codegen_cuda::qwen38_batched::QWEN38_BATCHED_KERNELS;
use logicaffeine_compile::codegen_cuda::qwen_emit::GEN_DEVICE_KERNELS;

fn have_nvcc() -> bool {
    Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn have_gpu() -> bool {
    Path::new("/dev/nvidia0").exists()
}

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

/// Compile against the SHIPPED kernel text — both the single-sequence set and the batched set — so the test
/// exercises the same source the emitter emits, not a copy of it.
fn run(body: &str, decls: &str, defs: &str, tag: &str) -> Vec<f64> {
    let src = format!(
        "#include <cstdio>\n#include <cmath>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {defs}\n{gdn}\n{q38}\n{gen}\n{extra}\n{bat}\n{decls}\n\
         int main(){{\n{body}\n  cudaDeviceSynchronize();\n\
         \x20 cudaError_t e=cudaGetLastError();\n\
         \x20 if(e!=cudaSuccess){{ fprintf(stderr,\"CUDA: %s\\n\",cudaGetErrorString(e)); return 1; }}\n\
         \x20 return 0;\n}}\n",
        gdn = GDN_KERNELS, q38 = QWEN38_KERNELS, gen = GEN_DEVICE_KERNELS,
        extra = QWEN38_EXTRA_KERNELS, bat = QWEN38_BATCHED_KERNELS);
    let dir = std::env::temp_dir().join(format!("logos_bat_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let (src_p, bin_p) = (dir.join("b.cu"), dir.join("b"));
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

/// The batched half and the single-sequence half must agree BIT FOR BIT at batch 1: nothing about widening a
/// kernel over sequences may change the arithmetic of any one of them.
fn assert_identical(tag: &str, got: &[f64], n: usize) {
    assert_eq!(got.len(), 2 * n, "[{tag}] expected both halves");
    let (batched, single) = got.split_at(n);
    for (i, (a, b)) in batched.iter().zip(single.iter()).enumerate() {
        assert_eq!(a, b,
            "[{tag}] element {i}: batched {a} != single-sequence {b}. The batched kernel must be the \
             single-sequence one with a sequence index threaded through — nothing else.");
    }
    println!("[{tag}] batched == single-sequence over {n} outputs, exactly");
}

/// The Q8_0 GEMV — the kernel batching exists for.
#[test]
fn the_batched_gemv_at_batch_one_equals_the_single_sequence_gemv() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    for &(k, n) in &[(512usize, 64usize), (5120, 96)] {
        let wq: Vec<f64> = (0..k * n).map(|i| (wv(701, i) * 100.0).round()).collect();
        let ws: Vec<f64> = (0..(k / 32) * n)
            .map(|i| 0.01 + wv(702, i).abs() * 0.01)
            .collect();
        let x: Vec<f64> = (0..k).map(|i| wv(703, i) * 3.0).collect();
        let decls = format!(
            "{}{}{}{}",
            arr("WQF", &wq),
            arr("WSF", &ws),
            arr("X", &x),
            format!(
                "__device__ __managed__ signed char WQ[{}], XH[{k}], XL[{k}];\n\
                     __device__ __managed__ __half WS[{}];\n\
                     __device__ __managed__ float XS[{}], YA[{n}], YB[{n}];\n\
                     static inline int Gw_(int m){{ return (m*32+255)/256; }}\n",
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
  bq_gemv_hl<<<dim3(Gw_({n}),1),256>>>(YA,WQ,WS,XH,XL,XS,{k},{n},1,{n});
  cudaDeviceSynchronize();
  gk_gemv_hl<<<Gw_({n}),256>>>(YB,WQ,WS,XH,XL,XS,{k},{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{n};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", YB[i]);\n",
            kn = k * n,
            nb = (k / 32) * n
        );
        let got = run(
            &body,
            &decls,
            "#define BTG 8\n#define MAXSEQ 64",
            &format!("bgemv{k}_{n}"),
        );
        assert_identical(&format!("batched GEMV K={k} N={n}"), &got, n);
    }
}

/// The pointwise and reduction kernels. `gate_mul` is first on purpose: it is the one that was wrong.
#[test]
fn the_batched_pointwise_kernels_equal_their_single_sequence_originals() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    let n = 512usize;
    let a: Vec<f64> = (0..n).map(|i| wv(801, i) * 2.0).collect();
    let g: Vec<f64> = (0..n).map(|i| wv(802, i) * 4.0).collect();
    let decls = format!(
        "{}{}{}",
        arr("A0", &a),
        arr("G", &g),
        format!("__device__ __managed__ float AA[{n}], AB[{n}];\n")
    );
    // The attention output gate: sigmoid, not SiLU.
    let body = format!(
        "  for(int i=0;i<{n};++i){{ AA[i]=A0[i]; AB[i]=A0[i]; }}
  cudaDeviceSynchronize();
  bq_gate_mul<<<dim3(({n}+255)/256,1),256>>>(AA,{n},G,{n},{n});
  cudaDeviceSynchronize();
  gk_gate_mul<<<({n}+255)/256,256>>>(AB,G,{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{n};++i) printf(\"%.9g \", AA[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", AB[i]);\n"
    );
    let got = run(&body, &decls, "#define BTG 8\n#define MAXSEQ 64", "bgate");
    assert_identical("batched gate_mul (SIGMOID, not SiLU)", &got, n);
}

/// The recurrence: the same memory update, the same output, with a sequence index added.
#[test]
fn the_batched_recurrence_at_batch_one_equals_the_single_sequence_recurrence() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    for &(hv, hk, d) in &[(4usize, 2usize, 32usize), (48, 16, 128)] {
        let (qn, vn) = (hk * d, hv * d);
        let cw = 2 * qn + vn; // the conv buffer the batched form strides by
        let cbuf: Vec<f64> = (0..cw).map(|i| wv(901, i)).collect();
        let bet: Vec<f64> = (0..hv).map(|i| 0.5 + wv(902, i) * 0.2).collect();
        let alp: Vec<f64> = (0..hv).map(|i| 0.5 + wv(903, i) * 0.2).collect();
        let s0: Vec<f64> = (0..vn * d).map(|i| wv(904, i)).collect();
        let decls = format!(
            "{}{}{}{}{}",
            arr("CB", &cbuf),
            arr("BET", &bet),
            arr("ALP", &alp),
            arr("S0", &s0),
            format!(
                "__device__ __managed__ float SA[{}], SB[{}], OA[{vn}], OB[{vn}];\n",
                vn * d,
                vn * d
            )
        );
        let body = format!(
"  for(int i=0;i<{sn};++i){{ SA[i]=S0[i]; SB[i]=S0[i]; }}
  cudaDeviceSynchronize();
  bq_gdn_step<<<dim3({hv},({d}+3)/4,1),128>>>(CB,CB+{qn},CB+{two_qn},BET,ALP,SA,OA,{d},{hk},{hv},{cw});
  cudaDeviceSynchronize();
  gdn_step<<<dim3({hv},({d}+3)/4),128>>>(CB,CB+{qn},CB+{two_qn},BET,ALP,SB,OB,{d},{hk});
  cudaDeviceSynchronize();
  for(int i=0;i<{vn};++i) printf(\"%.9g \", OA[i]);
  for(int i=0;i<{sn};++i) printf(\"%.9g \", SA[i]);
  for(int i=0;i<{vn};++i) printf(\"%.9g \", OB[i]);
  for(int i=0;i<{sn};++i) printf(\"%.9g \", SB[i]);\n",
            sn = vn * d, two_qn = 2 * qn);
        let got = run(
            &body,
            &decls,
            "#define BTG 8\n#define MAXSEQ 64",
            &format!("brec{hv}_{hk}_{d}"),
        );
        assert_identical(
            &format!("batched recurrence HV={hv} HK={hk} D={d}"),
            &got,
            vn + vn * d,
        );
    }
}

/// The conv-with-fused-L2 and the gate norm, both of which carry per-sequence state or a foreign row pitch.
#[test]
fn the_batched_conv_and_gate_norm_equal_their_single_sequence_originals() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    let (d, nk, nv, taps) = (128usize, 16usize, 48usize, 4usize);
    let c = 2 * nk * d + nv * d;
    let x: Vec<f64> = (0..c).map(|i| wv(1001, i)).collect();
    let wc: Vec<f64> = (0..taps * c).map(|i| wv(1002, i)).collect();
    let cs: Vec<f64> = (0..taps * c).map(|i| wv(1003, i)).collect();
    let decls = format!(
        "{}{}{}{}",
        arr("X", &x),
        arr("WC", &wc),
        arr("CS0", &cs),
        format!(
            "__device__ __managed__ float CSa[{tc}], CSb[{tc}], YA[{c}], YB[{c}];\n",
            tc = taps * c
        )
    );
    let body = format!(
        "  for(int i=0;i<{tc};++i){{ CSa[i]=CS0[i]; CSb[i]=CS0[i]; }}
  cudaDeviceSynchronize();
  bq_conv_silu_l2<<<dim3({c}/{d},1),{d}>>>(X,{c},WC,CSa,YA,{c},{taps},{nl2},1e-6f);
  cudaDeviceSynchronize();
  gk_conv_silu_l2<<<{c}/{d},{d}>>>(X,WC,CSb,YB,{c},{taps},{nl2},1e-6f);
  cudaDeviceSynchronize();
  for(int i=0;i<{c};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{tc};++i) printf(\"%.9g \", CSa[i]);
  for(int i=0;i<{c};++i) printf(\"%.9g \", YB[i]);
  for(int i=0;i<{tc};++i) printf(\"%.9g \", CSb[i]);\n",
        tc = taps * c,
        nl2 = 2 * nk
    );
    let got = run(&body, &decls, "#define BTG 8\n#define MAXSEQ 64", "bconv");
    assert_identical("batched conv+l2", &got, c + taps * c);
}

/// Attention, the RoPE, the KV store and the argmax — the four kernels whose batched forms carry a foreign
/// row pitch or a per-sequence cache, and the ones most likely to be retyped rather than derived.
#[test]
fn the_batched_attention_stack_equals_its_single_sequence_originals() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    let (h, kv, hd, pos) = (24usize, 4usize, 256usize, 7usize);
    let (aq, akv) = (h * hd, kv * hd);
    let pmax = 16384usize; // pbuf is wider than the attention projection: the pitch that bit us
    let q: Vec<f64> = (0..pmax).map(|i| wv(1101, i)).collect();
    let kc: Vec<f64> = (0..64 * akv).map(|i| wv(1102, i)).collect();
    let vc: Vec<f64> = (0..64 * akv).map(|i| wv(1103, i)).collect();
    let nw: Vec<f64> = (0..hd).map(|i| 1.0 + wv(1104, i) * 0.1).collect();
    let decls = format!(
        "{}{}{}{}{}",
        arr("QSRC", &q),
        arr("KC0", &kc),
        arr("VC0", &vc),
        arr("NW", &nw),
        format!(
            "__device__ __managed__ float QA[{pmax}], QB[{aq}], KA[{n}], KB2[{n}], VA[{n}];\n\
                 __device__ __managed__ float OA[{aq}], OB[{aq}];\n\
                 __device__ __managed__ int POS[1], ZERO[1];\n",
            n = 64 * akv
        )
    );
    let body = format!(
        "  POS[0]={pos}; ZERO[0]=0;
  for(int i=0;i<{pmax};++i) QA[i]=QSRC[i];
  for(int i=0;i<{aq};++i) QB[i]=QSRC[i];
  for(int i=0;i<{n};++i){{ KA[i]=KC0[i]; KB2[i]=KC0[i]; VA[i]=VC0[i]; }}
  cudaDeviceSynchronize();
  // per-head norm + partial rope, on the WIDE pitch and on the narrow one
  bq_qk_norm_rope_part<<<dim3({h},1),128>>>(QA,ZERO,0,{pmax},NW,{h},{hd},64,1e7f,1e-6f);
  cudaDeviceSynchronize();
  gk_qk_norm_rope_part<<<{h},128>>>(QB,ZERO,0,NW,{h},{hd},64,1e7f,1e-6f);
  cudaDeviceSynchronize();
  bq_attn<<<dim3({h},1),256,64*4>>>(OA,QA,{pmax},KA,VA,POS,{h},{kv},{hd},0.0625f);
  cudaDeviceSynchronize();
  gk_attn<<<{h},256,64*4>>>(OB,QB,KB2,VA,POS,{h},{kv},{hd},0.0625f);
  cudaDeviceSynchronize();
  for(int i=0;i<{aq};++i) printf(\"%.9g \", QA[i]);
  for(int i=0;i<{aq};++i) printf(\"%.9g \", OA[i]);
  for(int i=0;i<{aq};++i) printf(\"%.9g \", QB[i]);
  for(int i=0;i<{aq};++i) printf(\"%.9g \", OB[i]);\n",
        n = 64 * akv
    );
    let got = run(&body, &decls, "#define BTG 8\n#define MAXSEQ 64", "battn");
    assert_identical("batched qk-norm-rope + attention", &got, 2 * aq);
}

/// The entry norm, the activation quantizer and the SwiGLU epilogue — every place an activation is produced.
///
/// The SwiGLU launch geometry here is the EMITTER'S — `(n+255)/256` blocks of 256 threads, which is
/// `n/32` warps, one per quantisation block. It is not `(n/32+255)/256`: that was the old geometry
/// from when the kernel gave one THREAD to each 32-block, and against the current one-warp-per-block
/// form it would compute a thirty-second of the output and compare the rest against stale memory.
#[test]
fn the_batched_norms_and_quantizers_equal_their_single_sequence_originals() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    let n = 5120usize;
    let x: Vec<f64> = (0..n).map(|i| wv(1201, i) * 2.0).collect();
    let y: Vec<f64> = (0..n).map(|i| wv(1202, i) * 0.5).collect();
    let w: Vec<f64> = (0..n).map(|i| 1.0 + wv(1203, i) * 0.1).collect();
    let ff: Vec<f64> = (0..2 * n).map(|i| wv(1204, i) * 2.0).collect();
    let decls = format!("{}{}{}{}{}", arr("X0", &x), arr("Y", &y), arr("W", &w), arr("FF", &ff),
        format!("__device__ __managed__ float XA[{n}], XB[{n}], FA[{n}], FB[{n}], SA[{nb}], SB[{nb}];\n\
                 __device__ __managed__ signed char QA[{n}], QAL[{n}], QB[{n}], QBL[{n}];\n\
                 __device__ __managed__ float HSA[{hb}], HSB[{hb}];\n\
                 __device__ __managed__ signed char HQA[{n}], HQAL[{n}], HQB[{n}], HQBL[{n}];\n",
                nb = n / 32, hb = n / 32));
    let body = format!(
        "  for(int i=0;i<{n};++i){{ XA[i]=X0[i]; XB[i]=X0[i]; }}
  cudaDeviceSynchronize();
  bq_add_rmsnorm_q8f_hl<<<1,RMSN_NT>>>(XA,Y,QA,QAL,SA,FA,W,{n},1e-6f);
  cudaDeviceSynchronize();
  gk_add_rmsnorm_q8f_hl<<<1,256>>>(XB,Y,QB,QBL,SB,FB,W,{n},1e-6f);
  cudaDeviceSynchronize();
  bq_silu_had_q8_hl<<<dim3(({n}+255)/256,1),256>>>(FF,HQA,HQAL,HSA,{n});
  cudaDeviceSynchronize();
  gk_silu_had_q8_hl<<<({n}/32+255)/256,256>>>(FF,FF+{n},HQB,HQBL,HSB,{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{n};++i) printf(\"%.9g \", FA[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", XA[i]);
  for(int i=0;i<{nb};++i) printf(\"%.9g \", SA[i]);
  for(int i=0;i<{n};++i) printf(\"%d \", (int)QA[i]);
  for(int i=0;i<{n};++i) printf(\"%d \", (int)QAL[i]);
  for(int i=0;i<{nb};++i) printf(\"%.9g \", HSA[i]);
  for(int i=0;i<{n};++i) printf(\"%d \", (int)HQA[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", FB[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", XB[i]);
  for(int i=0;i<{nb};++i) printf(\"%.9g \", SB[i]);
  for(int i=0;i<{n};++i) printf(\"%d \", (int)QB[i]);
  for(int i=0;i<{n};++i) printf(\"%d \", (int)QBL[i]);
  for(int i=0;i<{nb};++i) printf(\"%.9g \", HSB[i]);
  for(int i=0;i<{n};++i) printf(\"%d \", (int)HQB[i]);\n",
        nb = n / 32
    );
    // RMSN_NT is the width the EMITTER launches this kernel at. Pinning it here rather than letting
    // the kernel's own guard supply a default is the point: the previous version hardcoded 256, so it
    // would have gone on passing while the emitter shipped 1024 against a 256-wide oracle.
    let got = run(
        &body,
        &decls,
        "#define BTG 8\n#define MAXSEQ 64\n#define RMSN_NT 256",
        "bnorm",
    );
    assert_identical(
        "batched entry norm + SwiGLU quantizer",
        &got,
        5 * n + 2 * (n / 32),
    );
}

/// ★ THE LOWERING-CHOICE LOCK ★ — the tensor-core GEMM and the register-tiled GEMV are two lowerings of the
/// SAME `matmul`, so they must agree.
///
/// This is what makes the choice safe to make in the compiler. The emitter picks by shape (GEMV at M=1, GEMM
/// once the batch fills the M tile) exactly as llama.cpp picks between mmvq and mmq — but a shape-driven
/// switch is only legitimate if both paths compute the same thing. The int8 dot accumulates in INT32, which
/// is exact, and both paths apply the per-32-block scales in the same float grouping and k order, so the
/// remaining difference is float re-association only.
#[test]
fn the_tensor_core_gemm_lowering_agrees_with_the_gemv_lowering() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    for &(k, n, m) in &[(512usize, 128usize, 8usize), (5120, 320, 16), (1024, 64, 5)] {
        let wq: Vec<f64> = (0..k * n).map(|i| (wv(1401, i) * 100.0).round()).collect();
        let ws: Vec<f64> = (0..(k / 32) * n)
            .map(|i| 0.01 + wv(1402, i).abs() * 0.01)
            .collect();
        let xq: Vec<f64> = (0..k * m).map(|i| (wv(1403, i) * 100.0).round()).collect();
        let xs: Vec<f64> = (0..(k / 32) * m)
            .map(|i| 0.01 + wv(1404, i).abs() * 0.01)
            .collect();
        let decls = format!(
            "{}{}{}{}{}",
            arr("WQF", &wq),
            arr("WSF", &ws),
            arr("XQF", &xq),
            arr("XS", &xs),
            format!(
                "__device__ __managed__ signed char WQ[{}], XQ[{}];\n\
                     __device__ __managed__ __half WS[{}];\n\
                     __device__ __managed__ float YA[{}], YB[{}];\n\
                     static inline int Gw_(int q){{ return (q*32+255)/256; }}\n",
                k * n,
                k * m,
                (k / 32) * n,
                m * n,
                m * n
            )
        );
        let body = format!(
"  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{km};++i) XQ[i]=(signed char)XQF[i];
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  cudaDeviceSynchronize();
  bq_gemm_mma<<<dim3(({n}+TBN-1)/TBN,({m}+TBM-1)/TBM),256,(size_t)(TBM+TBN)*(KSTEP+4)>>>(YA,WQ,WS,XQ,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  bq_gemv_h<<<dim3(Gw_({n}),({m}+7)/8),256>>>(YB,WQ,WS,XQ,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YB[i]);\n",
            kn = k * n, km = k * m, nb = (k / 32) * n, mn = m * n);
        let got = run(
            &body,
            &decls,
            "#define BTG 8\n#define MAXSEQ 64",
            &format!("mma{k}_{n}_{m}"),
        );
        assert_eq!(got.len(), 2 * m * n, "expected both lowerings");
        let (mma, gemv) = got.split_at(m * n);
        let mut worst = 0.0f64;
        let mut wi = 0usize;
        for (i, (a, b)) in mma.iter().zip(gemv.iter()).enumerate() {
            let d = (a - b).abs() / b.abs().max(1e-3);
            if d > worst {
                worst = d;
                wi = i;
            }
        }
        assert!(worst < 1e-5,
            "the GEMM lowering disagrees with the GEMV lowering by {worst:.3e} at element {wi} \
             (K={k} N={n} M={m}): {} vs {}. Both accumulate the int8 dot in int32 (exact) and apply the same \
             per-block scales in the same k order, so anything above float re-association is a real defect.",
            mma[wi], gemv[wi]);
        println!("[MMA lowering K={k} N={n} M={m}] tensor-core GEMM == register-tiled GEMV (worst rel {worst:.2e})");
    }
}

/// ★ THE KERNEL SET MUST COMPILE WITH NO DEFINES SUPPLIED.
///
/// This exists because the same defect landed twice in one session. `BMAX` was referenced by the
/// kernel text without being defined there, so the set compiled only when a caller happened to supply
/// it; seven tests went red the moment a second kernel used it. It was guarded — and an hour later
/// five MMQ shape constants were added the same way, and nine tests went red for the identical
/// reason.
///
/// A guard fixes one instance. This fixes the class: the shipped kernel text is compiled here with an
/// EMPTY define list, so any constant a future kernel introduces must carry its own default or this
/// goes red immediately, at the source, instead of in whichever unrelated test happens to include the
/// set next.
#[test]
fn the_kernel_set_compiles_with_no_defines_supplied() {
    if !require_tool("nvcc", have_nvcc()) {
        return;
    }
    let src =
        format!(
        "#include <cstdio>\n#include <cmath>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
         {gdn}\n{q38}\n{gen}\n{extra}\n{bat}\nint main(){{ return 0; }}\n",
        gdn = GDN_KERNELS, q38 = QWEN38_KERNELS, gen = GEN_DEVICE_KERNELS,
        extra = QWEN38_EXTRA_KERNELS, bat = QWEN38_BATCHED_KERNELS);
    let dir = std::env::temp_dir().join(format!("logos_selfcontained_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let src_p = dir.join("k.cu");
    std::fs::write(&src_p, &src).expect("write");
    let sm = std::env::var("LOGOS_SM").unwrap_or_else(|_| "sm_86".to_string());
    let out = Command::new("nvcc")
        .args(["-O3", &format!("-arch={sm}"), "-c"])
        .arg(&src_p)
        .arg("-o")
        .arg(dir.join("k.o"))
        .output()
        .expect("run nvcc");
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "the shipped kernel text does not compile on its own. Every constant it references must have \
         an `#ifndef` default in the kernel string — a caller supplying it by luck is what broke \
         BMAX and then the MMQ constants. nvcc said:\n{err}"
    );
}

/// The MMQ-shaped tile must agree with the register-tiled GEMV to a tolerance.
///
/// Unlike the staged GEMVs, this one RE-ASSOCIATES the K reduction: the register-tiled form splits K
/// across 32 lanes and tree-reduces, while a thread here owns the whole reduction for its outputs. So
/// bit-identity is not available and would be the wrong bar — the same position the tensor-core
/// lowering is in. What IS checkable: the per-32-block int8 dot is exact in int32, so the only
/// difference is the order of the K/32 float additions, and accumulating those in double makes this
/// kernel strictly MORE accurate than the reference rather than merely close.
///
/// The acceptance test for the model is the golden token stream, not this tolerance; this test exists
/// so a structural mistake (a wrong tile, a bad bijection, a dropped tail) is caught here in seconds
/// instead of on a rented chip.
#[test]
fn the_mmq_tile_agrees_with_the_register_tiled_gemv() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    // K must be a multiple of the staged chunk; every K in the model is a multiple of 512.
    for &(k, n, m) in &[
        (1024usize, 256usize, 32usize),
        (2048, 128, 32),
        (5120, 64, 32),
    ] {
        let wq: Vec<f64> = (0..k * n).map(|i| (wv(1701, i) * 100.0).round()).collect();
        let ws: Vec<f64> = (0..(k / 32) * n)
            .map(|i| 0.01 + wv(1702, i).abs() * 0.01)
            .collect();
        let xh: Vec<f64> = (0..k * m).map(|i| (wv(1703, i) * 100.0).round()).collect();
        let xl: Vec<f64> = (0..k * m).map(|i| (wv(1704, i) * 100.0).round()).collect();
        let xs: Vec<f64> = (0..(k / 32) * m)
            .map(|i| 0.01 + wv(1705, i).abs() * 0.01)
            .collect();
        let decls = format!(
            "{}{}{}{}{}{}",
            arr("WQF", &wq),
            arr("WSF", &ws),
            arr("XHF", &xh),
            arr("XLF", &xl),
            arr("XS", &xs),
            format!(
                "__device__ __managed__ signed char WQ[{}], XH[{}], XL[{}];\n\
                     __device__ __managed__ __half WS[{}];\n\
                     __device__ __managed__ float YA[{}], YB[{}];\n\
                     static inline int Gw_(int q){{ return (q*32+255)/256; }}\n",
                k * n,
                k * m,
                k * m,
                (k / 32) * n,
                m * n,
                m * n
            )
        );
        let tile = "((256/(MMQ_TJ/MMQ_RJ))*MMQ_RI)";
        let body = format!(
            "  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{km};++i) {{ XH[i]=(signed char)XHF[i]; XL[i]=(signed char)XLF[i]; }}
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  cudaDeviceSynchronize();
  bq_gemm_mmq<<<(({n})+{tile}-1)/{tile},256>>>(YA,WQ,WS,XH,XL,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  bq_gemv_hl<<<dim3(Gw_({n}),({m}+7)/8),256>>>(YB,WQ,WS,XH,XL,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YB[i]);\n",
            kn = k * n,
            km = k * m,
            nb = (k / 32) * n,
            mn = m * n,
            tile = tile
        );
        let defs = format!(
            "#define BTG 8\n#define MAXSEQ 64\n#define BMAX {m}\n\
                            #define MMQ_TJ {m}\n#define MMQ_RI 2\n#define MMQ_RJ 2\n\
                            #define MMQ_KC 256\n#define MMQ_ACC double"
        );
        let got = run(&body, &decls, &defs, &format!("mmq{k}_{n}_{m}"));
        let half = got.len() / 2;
        let (a, b) = (&got[..half], &got[half..]);
        let mut worst = 0.0f64;
        for (x, y) in a.iter().zip(b.iter()) {
            let d = (x - y).abs();
            let m = y.abs().max(1e-30);
            if m > 1e-6 {
                worst = worst.max(d / m);
            }
        }
        assert!(
            worst < 1e-3,
            "MMQ tile K={k} N={n} M={m}: worst relative difference {worst:.3e} against the \
             register-tiled GEMV. The int8 dot is exact per 32-block, so a difference this large \
             means the tile or its reduction is wrong, not that floats were re-associated."
        );
        eprintln!("MMQ tile K={k} N={n} M={m}: worst relative difference {worst:.3e}");
    }
}

/// The staged TWO-PASS GEMV must be bit-identical to its register-tiled original.
///
/// This is the lane the accuracy result rests on, so it is the one where a perturbation would be most
/// expensive: a moved bit here changes the logit vector that the whole oracle comparison is built on. The
/// staging touches only where the two activation words are read from — L2 versus shared memory — and leaves
/// the ascending 512-wide K chunking, the `blk = k0/32 + lane/2` scale index, and the
/// `((float)dh + (float)dl/127) * (wsb * xs)` grouping exactly as `bq_gemv_hl` has them. So, as with the
/// single-word staged form, the bar is zero difference rather than a tolerance.
#[test]
fn the_staged_two_pass_gemv_is_bit_identical_to_the_register_tiled_gemv() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    // Every K the model uses is a multiple of 512, which is what makes the zero-padded staging cover the
    // whole reduction; the shapes here keep that property so the test exercises the real configuration.
    for &(k, n, m) in &[
        (1024usize, 128usize, 8usize),
        (5120, 256, 8),
        (6144, 64, 4),
        (2048, 96, 2),
    ] {
        let wq: Vec<f64> = (0..k * n).map(|i| (wv(1601, i) * 100.0).round()).collect();
        let ws: Vec<f64> = (0..(k / 32) * n)
            .map(|i| 0.01 + wv(1602, i).abs() * 0.01)
            .collect();
        let xh: Vec<f64> = (0..k * m).map(|i| (wv(1603, i) * 100.0).round()).collect();
        let xl: Vec<f64> = (0..k * m).map(|i| (wv(1604, i) * 100.0).round()).collect();
        let xs: Vec<f64> = (0..(k / 32) * m)
            .map(|i| 0.01 + wv(1605, i).abs() * 0.01)
            .collect();
        let decls = format!(
            "{}{}{}{}{}{}",
            arr("WQF", &wq),
            arr("WSF", &ws),
            arr("XHF", &xh),
            arr("XLF", &xl),
            arr("XS", &xs),
            format!(
                "__device__ __managed__ signed char WQ[{}], XH[{}], XL[{}];\n\
                     __device__ __managed__ __half WS[{}];\n\
                     __device__ __managed__ float YA[{}], YB[{}];\n\
                     static inline int Gw_(int q){{ return (q*32+255)/256; }}\n",
                k * n,
                k * m,
                k * m,
                (k / 32) * n,
                m * n,
                m * n
            )
        );
        let body = format!(
"  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{km};++i) {{ XH[i]=(signed char)XHF[i]; XL[i]=(signed char)XLF[i]; }}
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  cudaDeviceSynchronize();
  bq_gemv_hl_smem<<<({n}+7)/8,256,(size_t)(2*{m}*512+{m}*16*4)>>>(YA,WQ,WS,XH,XL,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  bq_gemv_hl<<<dim3(Gw_({n}),({m}+7)/8),256>>>(YB,WQ,WS,XH,XL,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YB[i]);\n",
            kn = k * n, km = k * m, nb = (k / 32) * n, mn = m * n);
        let got = run(
            &body,
            &decls,
            &format!("#define BTG 8\n#define MAXSEQ 64\n#define BMAX {m}"),
            &format!("hlsmem{k}_{n}_{m}"),
        );
        assert_identical(
            &format!("staged two-pass GEMV K={k} N={n} M={m}"),
            &got,
            m * n,
        );
    }
}

/// The shared-memory staged GEMV must be BIT-IDENTICAL to the register-tiled one.
///
/// Staging changes only where the activation bytes are read FROM — L2 versus shared memory — not the order
/// in which products are summed: the same ascending 512-wide K blocks, the same per-block scale product. So
/// unlike the GEMM lowering (which re-associates and is held to 1e-5), this one has no excuse for any
/// difference at all. Anything nonzero here means the staging perturbed the arithmetic, which would show up
/// downstream as a flipped token rather than as a wrong number.
#[test]
fn the_staged_gemv_is_bit_identical_to_the_register_tiled_gemv() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    for &(k, n, m) in &[(1024usize, 128usize, 8usize), (5120, 256, 8), (6144, 64, 4)] {
        let wq: Vec<f64> = (0..k * n).map(|i| (wv(1501, i) * 100.0).round()).collect();
        let ws: Vec<f64> = (0..(k / 32) * n)
            .map(|i| 0.01 + wv(1502, i).abs() * 0.01)
            .collect();
        let xq: Vec<f64> = (0..k * m).map(|i| (wv(1503, i) * 100.0).round()).collect();
        let xs: Vec<f64> = (0..(k / 32) * m)
            .map(|i| 0.01 + wv(1504, i).abs() * 0.01)
            .collect();
        let decls = format!(
            "{}{}{}{}{}",
            arr("WQF", &wq),
            arr("WSF", &ws),
            arr("XQF", &xq),
            arr("XS", &xs),
            format!(
                "__device__ __managed__ signed char WQ[{}], XQ[{}];\n\
                     __device__ __managed__ __half WS[{}];\n\
                     __device__ __managed__ float YA[{}], YB[{}];\n\
                     static inline int Gw_(int q){{ return (q*32+255)/256; }}\n",
                k * n,
                k * m,
                (k / 32) * n,
                m * n,
                m * n
            )
        );
        let body = format!(
            "  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{km};++i) XQ[i]=(signed char)XQF[i];
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  cudaDeviceSynchronize();
  bq_gemv_h_smem<<<({n}+7)/8,256,(size_t)({m}*512+{m}*16*4)>>>(YA,WQ,WS,XQ,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  bq_gemv_h<<<dim3(Gw_({n}),({m}+7)/8),256>>>(YB,WQ,WS,XQ,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YB[i]);\n",
            kn = k * n,
            km = k * m,
            nb = (k / 32) * n,
            mn = m * n
        );
        let got = run(
            &body,
            &decls,
            &format!("#define BTG 8\n#define MAXSEQ 64\n#define BMAX {m}"),
            &format!("smem{k}_{n}_{m}"),
        );
        assert_identical(&format!("staged GEMV K={k} N={n} M={m}"), &got, m * n);
    }
}

/// The TENSOR-CORE tile must agree with the register-tiled GEMV, split or unsplit.
///
/// `bq_gemm_tc` moves the inner product from the integer pipe to `mma.sync.m16n8k32.s8`, which is
/// 4x the dp4a rate on an H100. The instruction consumes exactly 32 K-elements, which is exactly one
/// Q8_0 block, so the quantisation scale still lands on an instruction boundary and the accumulator
/// drains to float at the same cadence `bq_gemm_mmq` already uses. The int8 products themselves are
/// exact, so any real difference here is a wrong fragment map — and the m16n8k32 fragment layout is
/// the part of this kernel that cannot be checked by reading it. A transposed A operand or a B/C lane
/// map confusion (they differ: B is indexed by `groupID`, C by `tid_in_group*2`) still produces
/// plausible-looking numbers, which is exactly the failure this test exists to catch.
///
/// Split-K is checked in the same test because it re-associates the reduction: each K slice keeps its
/// own float accumulator and `bq_sk_reduce` sums them in a fixed ascending order. That is a different
/// grouping from the single-slice form, so it gets the same tolerance rather than bit-equality — but
/// it must not be a DIFFERENT ANSWER, which is what is asserted.
#[test]
fn the_tensor_core_tile_agrees_with_the_register_tiled_gemv() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    const SPLIT: usize = 4;
    // N=200 is deliberately not a multiple of the 128-row tile: the last block runs half-masked.
    for &(k, n, m) in &[
        (1024usize, 256usize, 32usize),
        (2048, 128, 32),
        (5120, 200, 32),
    ] {
        let wq: Vec<f64> = (0..k * n).map(|i| (wv(1801, i) * 100.0).round()).collect();
        let ws: Vec<f64> = (0..(k / 32) * n)
            .map(|i| 0.01 + wv(1802, i).abs() * 0.01)
            .collect();
        let xh: Vec<f64> = (0..k * m).map(|i| (wv(1803, i) * 100.0).round()).collect();
        let xl: Vec<f64> = (0..k * m).map(|i| (wv(1804, i) * 100.0).round()).collect();
        let xs: Vec<f64> = (0..(k / 32) * m)
            .map(|i| 0.01 + wv(1805, i).abs() * 0.01)
            .collect();
        let decls = format!(
            "{}{}{}{}{}{}",
            arr("WQF", &wq),
            arr("WSF", &ws),
            arr("XHF", &xh),
            arr("XLF", &xl),
            arr("XS", &xs),
            format!(
                "__device__ __managed__ signed char WQ[{}], XH[{}], XL[{}];\n\
                     __device__ __managed__ __half WS[{}];\n\
                     __device__ __managed__ float YA[{}], YB[{}], YC[{}], PBUF[{}];\n\
                     static inline int Gw_(int q){{ return (q*32+255)/256; }}\n",
                k * n,
                k * m,
                k * m,
                (k / 32) * n,
                m * n,
                m * n,
                m * n,
                SPLIT * m * n
            )
        );
        let ti = "(16*(8/(TC_TJ/8))*TC_WM)";
        let body = format!(
"  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{km};++i) {{ XH[i]=(signed char)XHF[i]; XL[i]=(signed char)XLF[i]; }}
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  cudaDeviceSynchronize();
  bq_gemm_tc<<<dim3((({n})+{ti}-1)/{ti},1),256>>>(YA,WQ,WS,XH,XL,XS,{k},{n},{m},{n},(float*)0,1);
  cudaDeviceSynchronize();
  bq_gemm_tc<<<dim3((({n})+{ti}-1)/{ti},{split}),256>>>(YC,WQ,WS,XH,XL,XS,{k},{n},{m},{n},PBUF,{split});
  bq_sk_reduce<<<(({mn})+255)/256,256>>>(YC,PBUF,{n},{m},{n},{split});
  cudaDeviceSynchronize();
  bq_gemv_hl<<<dim3(Gw_({n}),({m}+7)/8),256>>>(YB,WQ,WS,XH,XL,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YC[i]);
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YB[i]);\n",
            kn = k * n, km = k * m, nb = (k / 32) * n, mn = m * n, ti = ti, split = SPLIT);
        let defs = format!(
            "#define BTG 8\n#define MAXSEQ 64\n#define BMAX {m}\n\
                            #define TC_TJ {m}\n#define TC_WM 4\n#define TC_KC 128"
        );
        let got = run(&body, &decls, &defs, &format!("tc{k}_{n}_{m}"));
        let third = got.len() / 3;
        let (a, c, b) = (&got[..third], &got[third..2 * third], &got[2 * third..]);
        for (label, lane) in [("unsplit", a), ("split-K", c)] {
            let mut worst = 0.0f64;
            for (x, y) in lane.iter().zip(b.iter()) {
                let d = (x - y).abs();
                let mag = y.abs().max(1e-30);
                if mag > 1e-6 {
                    worst = worst.max(d / mag);
                }
            }
            assert!(
                worst < 1e-3,
                "tensor-core tile ({label}) K={k} N={n} M={m}: worst relative difference {worst:.3e} \
                 against the register-tiled GEMV. The int8 products are exact per 32-block, so a \
                 difference this large is a wrong m16n8k32 fragment map or a wrong split reduction, \
                 not float re-association."
            );
            eprintln!("tensor-core tile ({label}) K={k} N={n} M={m}: worst relative difference {worst:.3e}");
        }
    }
}

/// The ROW-PARALLEL GEMV must agree with the register-tiled GEMV it replaces.
///
/// `bq_gemv_wide` changes who reduces a row. The shipped `bq_gemv_hl` gives one output row to one
/// warp; this gives it to MV_NW warps and gives MV_RB rows to a block, so one activation load feeds
/// several rows. That reshuffles two things a test has to pin separately: the K reduction is now
/// split across warps and rejoined through shared memory, and the per-thread K stride is MV_NW*32
/// rather than 32. Either can be wrong in a way that still produces plausible magnitudes.
///
/// At MV_NW=1 the K order is unchanged from the original, so the bar there is BIT-EQUALITY rather
/// than a tolerance — that configuration is a pure traffic saving and must not move a single bit.
/// Above one warp the partial sums rejoin in a fixed warp order, which re-associates, so those get
/// the same 1e-3 tolerance the other tiled lowerings carry.
///
/// The N=200 shape is deliberately not a multiple of MV_RB, so the last block runs half-masked.
#[test]
fn the_row_parallel_gemv_agrees_with_the_register_tiled_gemv() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    for &(k, n, m) in &[
        (1024usize, 256usize, 2usize),
        (2048, 200, 4),
        (5120, 128, 8),
        (6144, 64, 1),
    ] {
        let wq: Vec<f64> = (0..k * n).map(|i| (wv(1901, i) * 100.0).round()).collect();
        let ws: Vec<f64> = (0..(k / 32) * n)
            .map(|i| 0.01 + wv(1902, i).abs() * 0.01)
            .collect();
        let xh: Vec<f64> = (0..k * m).map(|i| (wv(1903, i) * 100.0).round()).collect();
        let xl: Vec<f64> = (0..k * m).map(|i| (wv(1904, i) * 100.0).round()).collect();
        let xs: Vec<f64> = (0..(k / 32) * m)
            .map(|i| 0.01 + wv(1905, i).abs() * 0.01)
            .collect();
        // Every (warps, rows-per-block) the shape rule can emit, plus the bit-exact corner.
        for &(nw, rb) in &[(1usize, 2usize), (2, 2), (4, 2), (1, 1)] {
            let tj = m.next_power_of_two().max(1);
            let decls = format!(
                "{}{}{}{}{}{}",
                arr("WQF", &wq),
                arr("WSF", &ws),
                arr("XHF", &xh),
                arr("XLF", &xl),
                arr("XS", &xs),
                format!(
                    "__device__ __managed__ signed char WQ[{}], XH[{}], XL[{}];\n\
                         __device__ __managed__ __half WS[{}];\n\
                         __device__ __managed__ float YA[{}], YB[{}];\n\
                         static inline int Gw_(int q){{ return (q*32+255)/256; }}\n",
                    k * n,
                    k * m,
                    k * m,
                    (k / 32) * n,
                    m * n,
                    m * n
                )
            );
            let body = format!(
"  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{km};++i) {{ XH[i]=(signed char)XHF[i]; XL[i]=(signed char)XLF[i]; }}
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  cudaDeviceSynchronize();
  bq_gemv_wide<<<dim3(({n}+MV_RB-1)/MV_RB,1),MV_NW*32>>>(YA,WQ,WS,XH,XL,XS,{k},{n},{m},{n},(float*)0,1);
  cudaDeviceSynchronize();
  bq_gemv_hl<<<dim3(Gw_({n}),({m}+7)/8),256>>>(YB,WQ,WS,XH,XL,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YB[i]);\n",
                kn = k * n, km = k * m, nb = (k / 32) * n, mn = m * n);
            let defs = format!(
                "#define BTG 8\n#define MAXSEQ 64\n#define BMAX {m}\n\
                                #define MV_TJ {tj}\n#define MV_RB {rb}\n#define MV_NW {nw}"
            );
            let got = run(&body, &decls, &defs, &format!("wide{k}_{n}_{m}_{nw}_{rb}"));
            let half = got.len() / 2;
            let (a, b) = (&got[..half], &got[half..]);
            if nw == 1 {
                for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
                    assert_eq!(x, y,
                        "row-parallel GEMV NW=1 RB={rb} K={k} N={n} M={m} element {i}: {x} != {y}. \
                         One warp per row-group keeps the original ascending K order, so this \
                         configuration must be BIT-identical — it only stops re-reading the \
                         activation slab once per output row.");
                }
                eprintln!("row-parallel GEMV NW=1 RB={rb} K={k} N={n} M={m}: bit-identical over {} outputs", a.len());
            } else {
                let mut worst = 0.0f64;
                for (x, y) in a.iter().zip(b.iter()) {
                    let d = (x - y).abs();
                    let mag = y.abs().max(1e-30);
                    if mag > 1e-6 {
                        worst = worst.max(d / mag);
                    }
                }
                assert!(worst < 1e-3,
                    "row-parallel GEMV NW={nw} RB={rb} K={k} N={n} M={m}: worst relative difference \
                     {worst:.3e}. The int8 products are exact per 32-block, so a difference this \
                     large means the cross-warp K split or its shared-memory rejoin is wrong, not \
                     that floats were re-associated.");
                eprintln!("row-parallel GEMV NW={nw} RB={rb} K={k} N={n} M={m}: worst {worst:.3e}");
            }
        }
    }
}

/// A register tile narrower than the batch must compute NOTHING rather than a partial answer.
///
/// `MV_TJ` sizes the per-thread sequence tile. If the emitter ever picks one below the batch, the
/// natural failure is silent: the kernel fills the first MV_TJ sequences and leaves the rest holding
/// whatever the output buffer had, which reads as a plausible tensor of the right shape. That is
/// exactly the class `BATCH_AGREEMENT` was built to catch and exactly the class worth making
/// impossible instead. The kernel returns early; this pins that it does.
#[test]
fn the_row_parallel_gemv_refuses_a_tile_narrower_than_the_batch() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    let (k, n, m) = (1024usize, 64usize, 8usize);
    let wq: Vec<f64> = (0..k * n).map(|i| (wv(1911, i) * 100.0).round()).collect();
    let ws: Vec<f64> = (0..(k / 32) * n)
        .map(|i| 0.01 + wv(1912, i).abs() * 0.01)
        .collect();
    let xh: Vec<f64> = (0..k * m).map(|i| (wv(1913, i) * 100.0).round()).collect();
    let xl: Vec<f64> = (0..k * m).map(|i| (wv(1914, i) * 100.0).round()).collect();
    let xs: Vec<f64> = (0..(k / 32) * m)
        .map(|i| 0.01 + wv(1915, i).abs() * 0.01)
        .collect();
    let decls = format!(
        "{}{}{}{}{}{}",
        arr("WQF", &wq),
        arr("WSF", &ws),
        arr("XHF", &xh),
        arr("XLF", &xl),
        arr("XS", &xs),
        format!(
            "__device__ __managed__ signed char WQ[{}], XH[{}], XL[{}];\n\
                 __device__ __managed__ __half WS[{}];\n__device__ __managed__ float YA[{}];\n",
            k * n,
            k * m,
            k * m,
            (k / 32) * n,
            m * n
        )
    );
    // MV_TJ = 2 against a batch of 8: the guard must fire and leave the sentinel untouched.
    let body = format!(
"  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{km};++i) {{ XH[i]=(signed char)XHF[i]; XL[i]=(signed char)XLF[i]; }}
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  for(int i=0;i<{mn};++i) YA[i]=-12345.f;
  cudaDeviceSynchronize();
  bq_gemv_wide<<<dim3(({n}+MV_RB-1)/MV_RB,1),MV_NW*32>>>(YA,WQ,WS,XH,XL,XS,{k},{n},{m},{n},(float*)0,1);
  cudaDeviceSynchronize();
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YA[i]);\n",
        kn = k * n, km = k * m, nb = (k / 32) * n, mn = m * n);
    let defs = format!(
        "#define BTG 8\n#define MAXSEQ 64\n#define BMAX {m}\n\
                        #define MV_TJ 2\n#define MV_RB 2\n#define MV_NW 4"
    );
    let got = run(&body, &decls, &defs, "widenarrow");
    assert!(got.iter().all(|v| *v == -12345.0),
        "a sequence tile narrower than the batch must compute nothing; instead {} of {} outputs were \
         written, which is the silent-partial-answer failure this guard exists to prevent",
        got.iter().filter(|v| **v != -12345.0).count(), got.len());
    eprintln!(
        "row-parallel GEMV refused MV_TJ=2 against batch {m}, leaving all {} outputs untouched",
        got.len()
    );
}

/// Widening the residual-norm block must not break the batched == single-sequence invariant.
///
/// `bq_add_rmsnorm_q8f_hl` takes one block per sequence and reduces over the whole model dimension,
/// so at small batch its block width is the only parallelism available — widening 256 -> 1024
/// measured +5% on the whole step (A100, B=2, 31.05 -> 29.51 ms).
///
/// I expected that to be a numerics change and wrote a test asserting the two widths DIFFER. It
/// failed: they are bit-identical. The reduction genuinely does re-associate — 1024 partials of five
/// elements instead of 256 partials of twenty — but the block sum only reaches the output through
/// `rsqrtf`, which is a hardware approximation whose result granularity is coarser than the one- or
/// two-ulp difference the re-association introduces. So the divergence exists and is then rounded
/// away.
///
/// That makes "the two widths differ" the wrong thing to assert: it is a property of `rsqrtf`'s
/// precision on one input, not a contract. The contract is the one this file exists for — the
/// batched kernel must compute exactly what the single-sequence kernel computes — and it is asserted
/// here at EVERY width the emitter may pick, so the invariant is what gates the speed rather than a
/// guess about float association.
#[test]
fn the_norm_holds_the_single_sequence_invariant_at_every_block_width() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    let n = 5120usize;
    let x: Vec<f64> = (0..n).map(|i| wv(1301, i) * 2.0).collect();
    let y: Vec<f64> = (0..n).map(|i| wv(1302, i) * 0.5).collect();
    let w: Vec<f64> = (0..n).map(|i| 1.0 + wv(1303, i) * 0.1).collect();
    for width in [256usize, 1024] {
        let decls = format!("{}{}{}{}", arr("X0", &x), arr("Y", &y), arr("W", &w),
            format!("__device__ __managed__ float XA[{n}], XB[{n}], FA[{n}], FB[{n}], SA[{nb}], SB[{nb}];\n\
                     __device__ __managed__ signed char QA[{n}], QAL[{n}], QB[{n}], QBL[{n}];\n", nb = n / 32));
        let body = format!(
            "  for(int i=0;i<{n};++i){{ XA[i]=X0[i]; XB[i]=X0[i]; }}
  cudaDeviceSynchronize();
  bq_add_rmsnorm_q8f_hl<<<1,RMSN_NT>>>(XA,Y,QA,QAL,SA,FA,W,{n},1e-6f);
  cudaDeviceSynchronize();
  gk_add_rmsnorm_q8f_hl<<<1,256>>>(XB,Y,QB,QBL,SB,FB,W,{n},1e-6f);
  cudaDeviceSynchronize();
  for(int i=0;i<{n};++i) printf(\"%.9g \", FA[i]);
  for(int i=0;i<{nb};++i) printf(\"%.9g \", SA[i]);
  for(int i=0;i<{n};++i) printf(\"%d \", (int)QA[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", FB[i]);
  for(int i=0;i<{nb};++i) printf(\"%.9g \", SB[i]);
  for(int i=0;i<{n};++i) printf(\"%d \", (int)QB[i]);\n",
            nb = n / 32
        );
        let got = run(
            &body,
            &decls,
            &format!("#define BTG 8\n#define MAXSEQ 64\n#define RMSN_NT {width}"),
            &format!("norminv{width}"),
        );
        assert_identical(
            &format!("batched norm at block width {width} vs single-sequence"),
            &got,
            2 * n + n / 32,
        );
    }
}

/// The SPLIT residual norm must agree with the fused one it replaces — closely, but not exactly.
///
/// `bq_add_rmsnorm_part` + `bq_add_rmsnorm_apply` cut the sum of squares into NORM_G chunk-sums
/// instead of RMSN_NT thread-sums, which is a different association and therefore a different number.
/// This test states HOW different, in the units that decide whether it matters: our two-pass lane
/// measures rel_L2 1.428e-2 against the f64 CPU oracle and llama.cpp's `-fa 1` measures 1.499e-2,
/// with llama's own `-fa 0`/`-fa 1` differing from each other by more than either differs from the
/// oracle. So 1.5e-2 is the intrinsic spread; anything far below it cannot move the standing.
///
/// The bar here is therefore two-sided. Too large a difference means a wrong kernel, not a rounding
/// choice — the residual add, the weight multiply and the per-32-block quantiser are all untouched,
/// so only the scale may move. Too small (bit-identical) would mean the split is not actually being
/// taken and the test is measuring nothing.
#[test]
fn the_split_residual_norm_agrees_with_the_fused_one_within_a_stated_bound() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    for &(n, g) in &[(5120usize, 20usize), (5120, 40), (2048, 8)] {
        let x: Vec<f64> = (0..n).map(|i| wv(1401, i) * 2.0).collect();
        let y: Vec<f64> = (0..n).map(|i| wv(1402, i) * 0.5).collect();
        let w: Vec<f64> = (0..n).map(|i| 1.0 + wv(1403, i) * 0.1).collect();
        let decls = format!("{}{}{}{}", arr("X0", &x), arr("Y", &y), arr("W", &w),
            format!("__device__ __managed__ float XA[{n}], XB[{n}], FA[{n}], FB[{n}], SA[{nb}], SB[{nb}], PT[{g}];\n\
                     __device__ __managed__ signed char QA[{n}], QAL[{n}], QB[{n}], QBL[{n}];\n", nb = n / 32));
        let body = format!(
            "  for(int i=0;i<{n};++i){{ XA[i]=X0[i]; XB[i]=X0[i]; }}
  cudaDeviceSynchronize();
  bq_add_rmsnorm_q8f_hl<<<1,RMSN_NT>>>(XA,Y,QA,QAL,SA,FA,W,{n},1e-6f);
  cudaDeviceSynchronize();
  bq_add_rmsnorm_part<<<dim3({g},1),RMSN_NT>>>(XB,Y,PT,{n},{g});
  bq_add_rmsnorm_apply<<<dim3({g},1),RMSN_NT>>>(XB,PT,QB,QBL,SB,FB,W,{n},1e-6f,{g});
  cudaDeviceSynchronize();
  for(int i=0;i<{n};++i) printf(\"%.9g \", FA[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", XA[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", FB[i]);
  for(int i=0;i<{n};++i) printf(\"%.9g \", XB[i]);\n"
        );
        let got = run(
            &body,
            &decls,
            "#define BTG 8\n#define MAXSEQ 64\n#define RMSN_NT 256",
            &format!("splitn{n}_{g}"),
        );
        let q = got.len() / 4;
        let (fa, xa, fb, xb) = (&got[..q], &got[q..2 * q], &got[2 * q..3 * q], &got[3 * q..]);

        // The residual add itself has no reduction in it, so x must come back EXACTLY equal.
        for (i, (a, b)) in xa.iter().zip(xb.iter()).enumerate() {
            assert_eq!(a, b,
                "split norm n={n} G={g}: the residual add differs at element {i} ({a} vs {b}). \
                 Only the sum of squares re-associates; x = x + y is elementwise and must be exact.");
        }
        let mut worst = 0.0f64;
        for (a, b) in fa.iter().zip(fb.iter()) {
            let m = a.abs().max(1e-30);
            if m > 1e-6 {
                worst = worst.max((a - b).abs() / m);
            }
        }
        assert!(worst < 1e-5,
            "split norm n={n} G={g}: worst relative difference {worst:.3e} against the fused form. \
             Only the scale may move, by a reduction re-association — a difference this large means \
             the chunking or the partial sum is wrong.");
        assert!(worst > 0.0 || n != 5120,
            "split norm n={n} G={g} is bit-identical to the fused form, which means the split is not \
             being taken; the test is then measuring nothing and the speed claim is unsupported.");
        eprintln!(
            "split norm n={n} G={g}: residual add exact, scale re-association {worst:.3e} \
                   (llama's own -fa0/-fa1 spread is 1.5e-2, five orders larger)"
        );
    }
}

/// ★ PER-SEQUENCE POSITIONS — the invariant continuous batching stands on.
///
/// Today `d_pos` is a SINGLE int shared by the whole batch (`bq_attn: int L=*d_pos+1`,
/// `bq_kv_store: long p=*d_pos`, `bq_qk_norm_rope_part: (*d_pos)`), so every sequence is forced to the
/// same position. That is fine for the lockstep benchmark the batched lane was built for — all B
/// sequences carry the same prompt and advance together — and it is fatal for serving, where requests
/// arrive and finish at different times and therefore sit at different lengths.
///
/// The oracle is the golden single-sequence `gk_attn`, run once per sequence at that sequence's own
/// position. A batch of two at DIFFERENT positions must reproduce both, exactly.
///
/// This is a generalisation, not a change of behaviour: when every entry of `d_pos` is equal the
/// indexed form reads the same value the dereferenced form did, so the golden lane's arithmetic is
/// untouched — which is what makes it safe to adopt.
#[test]
fn batched_attention_honours_a_different_position_per_sequence() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    let (h, kv, hd) = (2usize, 1usize, 8usize);
    let (aq, kd, maxseq) = (h * hd, kv * hd, 64usize);
    let (p0, p1) = (1usize, 5usize); // seq 0 attends over 2 keys, seq 1 over 6
    let slab = maxseq * kd;
    let q: Vec<f64> = (0..2 * aq).map(|i| wv(2201, i)).collect();
    let k: Vec<f64> = (0..2 * slab).map(|i| wv(2202, i)).collect();
    let v: Vec<f64> = (0..2 * slab).map(|i| wv(2203, i)).collect();
    let decls = format!("{}{}{}{}",
        arr("QSRC", &q), arr("KSRC", &k), arr("VSRC", &v),
        format!("__device__ __managed__ float QA[{qa}], KA[{ka}], VA[{ka}], OA[{qa}];\n\
                 __device__ __managed__ float Q0[{aq}], Q1[{aq}], K0[{slab}], K1[{slab}], V0[{slab}], V1[{slab}];\n\
                 __device__ __managed__ float OB0[{aq}], OB1[{aq}];\n\
                 __device__ __managed__ int POS2[2], P0[1], P1[1];\n",
                qa = 2 * aq, ka = 2 * slab));
    let body = format!(
"  POS2[0]={p0}; POS2[1]={p1}; P0[0]={p0}; P1[0]={p1};
  for(int i=0;i<{qa};++i) QA[i]=QSRC[i];
  for(int i=0;i<{aq};++i){{ Q0[i]=QSRC[i]; Q1[i]=QSRC[{aq}+i]; }}
  for(int i=0;i<{ka};++i){{ KA[i]=KSRC[i]; VA[i]=VSRC[i]; }}
  for(int i=0;i<{slab};++i){{ K0[i]=KSRC[i]; V0[i]=VSRC[i]; K1[i]=KSRC[{slab}+i]; V1[i]=VSRC[{slab}+i]; }}
  cudaDeviceSynchronize();
  bq_attn<<<dim3({h},2),256,{maxseq}*4>>>(OA,QA,{aq},KA,VA,POS2,{h},{kv},{hd},0.35355339f);
  cudaDeviceSynchronize();
  gk_attn<<<{h},256,{maxseq}*4>>>(OB0,Q0,K0,V0,P0,{h},{kv},{hd},0.35355339f);
  cudaDeviceSynchronize();
  gk_attn<<<{h},256,{maxseq}*4>>>(OB1,Q1,K1,V1,P1,{h},{kv},{hd},0.35355339f);
  cudaDeviceSynchronize();
  for(int i=0;i<{qa};++i) printf(\"%.9g \", OA[i]);
  for(int i=0;i<{aq};++i) printf(\"%.9g \", OB0[i]);
  for(int i=0;i<{aq};++i) printf(\"%.9g \", OB1[i]);\n",
        qa = 2 * aq, ka = 2 * slab);
    let got = run(
        &body,
        &decls,
        &format!("#define BTG 8\n#define MAXSEQ {maxseq}"),
        "posperseq",
    );
    assert_eq!(
        got.len(),
        4 * aq,
        "expected {} printed values, got {}",
        4 * aq,
        got.len()
    );
    for i in 0..aq {
        assert_eq!(
            got[i],
            got[2 * aq + i],
            "sequence 0 at position {p0}: batched output[{i}] {} != single-sequence oracle {}",
            got[i],
            got[2 * aq + i]
        );
        assert_eq!(got[aq + i], got[3 * aq + i],
            "sequence 1 at position {p1}: batched output[{i}] {} != single-sequence oracle {} — the batch \
             forced it to sequence 0's length, which is the shared-`d_pos` defect",
            got[aq + i], got[3 * aq + i]);
    }
}

/// ★ THE ROW DIMENSION MUST EXCEED THE SEQUENCE TILE.
///
/// The tile required B <= TC_TJ <= 64, which caps one GEMM at 64 rows. That is exactly the wall in
/// front of chunked prefill: processing T prompt tokens at once is worth doing because the 24.33 GB
/// weight stream is then read ONCE for B*T rows instead of once per token, and B*T passes 64
/// immediately. Measured cost of not having it: TTFT 22.96 s at B=32, against vLLM's 1.89 s, which is
/// most of a 5.12x end-to-end serving deficit that no amount of GEMM tuning can touch.
///
/// Here B=96 against TC_TJ=32 — three M-tiles — checked against `bq_gemv_h`, which has no such cap.
#[test]
fn the_tile_handles_more_rows_than_its_sequence_tile() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    let (k, n, m, tj) = (1024usize, 256usize, 96usize, 32usize);
    let wq: Vec<f64> = (0..k * n).map(|i| (wv(2401, i) * 100.0).round()).collect();
    let ws: Vec<f64> = (0..(k / 32) * n)
        .map(|i| 0.01 + wv(2402, i).abs() * 0.01)
        .collect();
    let xh: Vec<f64> = (0..k * m).map(|i| (wv(2403, i) * 100.0).round()).collect();
    let xs: Vec<f64> = (0..(k / 32) * m)
        .map(|i| 0.01 + wv(2404, i).abs() * 0.01)
        .collect();
    let decls = format!(
        "{}{}{}{}{}",
        arr("WQF", &wq),
        arr("WSF", &ws),
        arr("XHF", &xh),
        arr("XS", &xs),
        format!(
            "__device__ __managed__ signed char WQ[{}], XH[{}];\n\
                 __device__ __managed__ __half WS[{}];\n\
                 __device__ __managed__ float YA[{}], YB[{}];\n\
                 static inline int Gw_(int q){{ return (q*32+255)/256; }}\n",
            k * n,
            k * m,
            (k / 32) * n,
            m * n,
            m * n
        )
    );
    let ti = "(16*(8/(TC_TJ/8))*TC_WM)";
    let body = format!(
"  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{km};++i) XH[i]=(signed char)XHF[i];
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  cudaDeviceSynchronize();
  bq_gemm_tc1<<<dim3((({n})+{ti}-1)/{ti},1,({m}+{tj}-1)/{tj}),256>>>(YA,WQ,WS,XH,XS,{k},{n},{m},{n},(float*)0,1);
  cudaDeviceSynchronize();
  bq_gemv_h<<<dim3(Gw_({n}),({m}+7)/8),256>>>(YB,WQ,WS,XH,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YB[i]);\n",
        kn = k * n, km = k * m, nb = (k / 32) * n, mn = m * n, ti = ti, tj = tj);
    let defs = format!(
        "#define BTG 8\n#define MAXSEQ 64\n#define BMAX {m}\n\
                        #define TC_TJ {tj}\n#define TC_WM 4\n#define TC_KC 64\n#define TC_STAGES 2"
    );
    let got = run(&body, &decls, &defs, "mtile");
    assert_eq!(got.len(), 2 * m * n, "expected 2x{} values", m * n);
    let half = got.len() / 2;
    let mut worst = 0.0f64;
    for (x, y) in got[..half].iter().zip(got[half..].iter()) {
        let mag = y.abs().max(1e-30);
        if mag > 1e-6 {
            worst = worst.max((x - y).abs() / mag);
        }
    }
    assert!(
        worst < 1e-3,
        "the tile disagrees with bq_gemv_h when the rows exceed TC_TJ: worst relative difference \
         {worst:.3e} — the M-tile origin is not being applied to both the staging and the epilogue"
    );
}

/// ★ The ASYNC PRODUCER must not move a single bit.
///
/// The tile is bound by neither math (9.9% of tensor-core peak) nor bandwidth (728 GB/s where
/// `bq_gdn_step` gets 2.0 TB/s on the same chip), and forcing occupancy measured monotonically WORSE
/// — so the lever is memory-level parallelism per thread, i.e. a deeper pipeline that does not live
/// in registers. `cp.async` is that producer. It stages the SAME bytes into the SAME padded layout,
/// four at a time, so the fragment map and the arithmetic are untouched by construction.
///
/// A staged pipeline that computes something slightly else, quickly, is the entire risk of this
/// approach — and this campaign has already met that failure once, where TC_KC=48 measured 760.29
/// tok/s, beat vLLM, and was running a different model. So the async path is held to the same oracle
/// as the synchronous one before any of its speed is believed.
#[test]
fn the_cp_async_staged_tile_agrees_with_the_synchronous_one() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    // The async producer must not change a single bit. It stages the SAME bytes into the SAME
    // XOR-swizzled layout, so the fragment map and the arithmetic are untouched by construction. This
    // proves that claim rather than asserting it, against the same oracle the synchronous tile uses.
    //
    // EVERY stage depth is swept, because depth is exactly where this tile has gone wrong before: at
    // TC_STAGES=3 the pipeline must prime S-1 chunks AND fetch S-1 ahead, and the original code was
    // correct at S=2 only by coincidence. K=128 gives FEWER chunks than stages, which is the case that
    // over-issues during priming and then reads a group it never waited on.
    for &(k, n, m) in &[
        (1024usize, 256usize, 32usize),
        (5120, 200, 32),
        (128, 256, 32),
    ] {
        for stages in 1..=3usize {
            let wq: Vec<f64> = (0..k * n).map(|i| (wv(2301, i) * 100.0).round()).collect();
            let ws: Vec<f64> = (0..(k / 32) * n)
                .map(|i| 0.01 + wv(2302, i).abs() * 0.01)
                .collect();
            let xh: Vec<f64> = (0..k * m).map(|i| (wv(2303, i) * 100.0).round()).collect();
            let xs: Vec<f64> = (0..(k / 32) * m)
                .map(|i| 0.01 + wv(2304, i).abs() * 0.01)
                .collect();
            let decls = format!(
                "{}{}{}{}{}",
                arr("WQF", &wq),
                arr("WSF", &ws),
                arr("XHF", &xh),
                arr("XS", &xs),
                format!(
                    "__device__ __managed__ signed char WQ[{}], XH[{}];\n\
                     __device__ __managed__ __half WS[{}];\n\
                     __device__ __managed__ float YA[{}], YB[{}];\n\
                     static inline int Gw_(int q){{ return (q*32+255)/256; }}\n",
                    k * n,
                    k * m,
                    (k / 32) * n,
                    m * n,
                    m * n
                )
            );
            let ti = "(16*(8/(TC_TJ/8))*TC_WM)";
            let body = format!(
                "  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{km};++i) XH[i]=(signed char)XHF[i];
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  cudaDeviceSynchronize();
  bq_gemm_tc1<<<dim3((({n})+{ti}-1)/{ti},1),256>>>(YA,WQ,WS,XH,XS,{k},{n},{m},{n},(float*)0,1);
  cudaDeviceSynchronize();
  bq_gemv_h<<<dim3(Gw_({n}),({m}+7)/8),256>>>(YB,WQ,WS,XH,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YB[i]);\n",
                kn = k * n,
                km = k * m,
                nb = (k / 32) * n,
                mn = m * n,
                ti = ti
            );
            // The ONLY difference from the synchronous lock is the staging depth.
            let defs = format!("#define BTG 8\n#define MAXSEQ 64\n#define BMAX {m}\n\
                            #define TC_TJ {m}\n#define TC_WM 4\n#define TC_KC 64\n#define TC_STAGES {stages}");
            let got = run(&body, &decls, &defs, &format!("cpa{k}_{n}_{m}_s{stages}"));
            assert_eq!(
                got.len(),
                2 * m * n,
                "expected 2x{} values at k={k} n={n}",
                m * n
            );
            let half = got.len() / 2;
            let mut worst = 0.0f64;
            for (x, y) in got[..half].iter().zip(got[half..].iter()) {
                let mag = y.abs().max(1e-30);
                if mag > 1e-6 {
                    worst = worst.max((x - y).abs() / mag);
                }
            }
            assert!(worst < 1e-3,
            "the cp.async-staged tile at TC_STAGES={stages} disagrees with bq_gemv_h at k={k} n={n} \
             m={m}: worst relative difference {worst:.3e} — a staged pipeline that computes something \
             else is the one failure mode this whole approach has, and it has measured FASTER than \
             the correct one every time it has happened");
        }
    }
}

#[test]
fn the_single_pass_tensor_core_tile_agrees_with_the_single_pass_gemv() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    const SPLIT: usize = 4;
    for &(k, n, m) in &[
        (1024usize, 256usize, 32usize),
        (2048, 128, 32),
        (5120, 200, 32),
    ] {
        let wq: Vec<f64> = (0..k * n).map(|i| (wv(1901, i) * 100.0).round()).collect();
        let ws: Vec<f64> = (0..(k / 32) * n)
            .map(|i| 0.01 + wv(1902, i).abs() * 0.01)
            .collect();
        let xh: Vec<f64> = (0..k * m).map(|i| (wv(1903, i) * 100.0).round()).collect();
        let xs: Vec<f64> = (0..(k / 32) * m)
            .map(|i| 0.01 + wv(1904, i).abs() * 0.01)
            .collect();
        let decls = format!(
            "{}{}{}{}{}",
            arr("WQF", &wq),
            arr("WSF", &ws),
            arr("XHF", &xh),
            arr("XS", &xs),
            format!(
                "__device__ __managed__ signed char WQ[{}], XH[{}];\n\
                     __device__ __managed__ __half WS[{}];\n\
                     __device__ __managed__ float YA[{}], YB[{}], YC[{}], PBUF[{}];\n\
                     static inline int Gw_(int q){{ return (q*32+255)/256; }}\n",
                k * n,
                k * m,
                (k / 32) * n,
                m * n,
                m * n,
                m * n,
                SPLIT * m * n
            )
        );
        let ti = "(16*(8/(TC_TJ/8))*TC_WM)";
        let body = format!(
"  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{km};++i) XH[i]=(signed char)XHF[i];
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  cudaDeviceSynchronize();
  bq_gemm_tc1<<<dim3((({n})+{ti}-1)/{ti},1),256>>>(YA,WQ,WS,XH,XS,{k},{n},{m},{n},(float*)0,1);
  cudaDeviceSynchronize();
  bq_gemm_tc1<<<dim3((({n})+{ti}-1)/{ti},{split}),256>>>(YC,WQ,WS,XH,XS,{k},{n},{m},{n},PBUF,{split});
  bq_sk_reduce<<<(({mn})+255)/256,256>>>(YC,PBUF,{n},{m},{n},{split});
  cudaDeviceSynchronize();
  bq_gemv_h<<<dim3(Gw_({n}),({m}+7)/8),256>>>(YB,WQ,WS,XH,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YC[i]);
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YB[i]);\n",
            kn = k * n, km = k * m, nb = (k / 32) * n, mn = m * n, ti = ti, split = SPLIT);
        let defs = format!(
            "#define BTG 8\n#define MAXSEQ 64\n#define BMAX {m}\n\
                            #define TC_TJ {m}\n#define TC_WM 4\n#define TC_KC 128"
        );
        let got = run(&body, &decls, &defs, &format!("tc1{k}_{n}_{m}"));
        assert_eq!(
            got.len(),
            3 * m * n,
            "expected 3x{} values at k={k} n={n}",
            m * n
        );
        let third = got.len() / 3;
        let (a, c, b) = (&got[..third], &got[third..2 * third], &got[2 * third..]);
        for (label, lane) in [("unsplit", a), ("split-K", c)] {
            let mut worst = 0.0f64;
            for (x, y) in lane.iter().zip(b.iter()) {
                let mag = y.abs().max(1e-30);
                if mag > 1e-6 {
                    worst = worst.max((x - y).abs() / mag);
                }
            }
            // 1e-3, the same bound the three sibling tile-vs-GEMV tests use, and for the reason the
            // two-pass one documents: the int8 products are exact per 32-block, so a difference this
            // large means a wrong m16n8k32 fragment map or a wrong split reduction -- not the float
            // re-association across K-chunks that the tile's structure necessarily introduces
            // (measured here at 2.5e-6, 400x below the bound).
            assert!(worst < 1e-3,
                "single-pass tensor-core tile ({label}) K={k} N={n} M={m}: worst relative difference \
                 {worst:.3e} against bq_gemv_h — at this size it is a wrong fragment map or split \
                 reduction, not float re-association");
        }
    }
}

/// ★ THE OPERAND SWAP: activations in m16, weights in n8.
///
/// `bq_gemm_tc*` puts weights in the m16 operand and sequences in n8, so a block covers 8 sequences
/// per column group and at most TC_TJ=64 rows across 8 warps. Rows per weight pass are capped at 64
/// **by the instruction mapping**. Invisible at decode (B<=64 needs one pass anyway); it is the entire
/// prefill deficit — 512 prompt tokens cost 512 passes over 24.33 GB, measured as TTFT 22.96 s against
/// vLLM's 1.89 s.
///
/// `bq_gemm_tcm` is the mapping CUTLASS uses, expressed with the Ampere instruction we have: rows in
/// the wide dimension. A block covers MI=256 sequence rows against MT_NT=16 channels, so the weight
/// tile is read once per 256 rows. Same arithmetic, different assignment of operands — which is
/// exactly the kind of change that can be plausibly wrong, so it is held to `bq_gemv_h`.
#[test]
fn the_rows_in_m_tile_agrees_with_the_gemv() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    // m=96 exercises a partial 256-row tile; n=48 a partial 16-channel tile.
    // m=300 > MI=256 is the case that matters and the one this test did NOT cover: it is the first
    // shape with grid.y > 1, i.e. more than one M-tile. Prefill at PT=16 (BT=512 rows) is exactly
    // that, and it produced a DIFFERENT token stream on hardware while PT=8 (BT=256, one tile) was
    // byte-identical. A test whose largest m is below MI can never see it.
    // m=2 and m=16 are the widths a SPECULATIVE VERIFY uses (B sequences x 2 tokens). The lock only
    // ever checked wide chunks, and a narrow one is exactly where the two lowerings were found to
    // disagree in situ -- the prefill lane had always taken the tile, so nothing compared them here.
    for &(k, n, m) in &[
        (1024usize, 256usize, 96usize),
        (2048, 48, 96),
        (1024, 128, 300),
        (1024, 256, 2),
        (1024, 256, 16),
        (2048, 48, 2),
    ] {
        let wq: Vec<f64> = (0..k * n).map(|i| (wv(2501, i) * 100.0).round()).collect();
        let ws: Vec<f64> = (0..(k / 32) * n)
            .map(|i| 0.01 + wv(2502, i).abs() * 0.01)
            .collect();
        let xh: Vec<f64> = (0..k * m).map(|i| (wv(2503, i) * 100.0).round()).collect();
        let xs: Vec<f64> = (0..(k / 32) * m)
            .map(|i| 0.01 + wv(2504, i).abs() * 0.01)
            .collect();
        let decls = format!(
            "{}{}{}{}{}",
            arr("WQF", &wq),
            arr("WSF", &ws),
            arr("XHF", &xh),
            arr("XS", &xs),
            format!(
                "__device__ __managed__ signed char WQ[{}], XH[{}];\n\
                     __device__ __managed__ __half WS[{}];\n\
                     __device__ __managed__ float YA[{}], YB[{}];\n\
                     static inline int Gw_(int q){{ return (q*32+255)/256; }}\n",
                k * n,
                k * m,
                (k / 32) * n,
                m * n,
                m * n
            )
        );
        // Must match the kernel: NCT = MT_NT/(8*MT_WN), not MT_NT/8. They drifted when MT_WN
        // landed, and a grid computed from the stale formula over-provisions blocks silently.
        let mi = "(16*(8/(MT_NT/(8*MT_WN)))*MT_WM)";
        let body = format!(
"  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{km};++i) XH[i]=(signed char)XHF[i];
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  for(int i=0;i<{mn};++i){{ YA[i]=0.f; YB[i]=0.f; }}
  cudaDeviceSynchronize();
  bq_gemm_tcm<<<dim3((({n})+MT_NT-1)/MT_NT,(({m})+{mi}-1)/{mi}),256>>>(YA,WQ,WS,XH,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  bq_gemv_h<<<dim3(Gw_({n}),({m}+7)/8),256>>>(YB,WQ,WS,XH,XS,{k},{n},{m},{n});
  cudaDeviceSynchronize();
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YB[i]);\n",
            kn = k * n, km = k * m, nb = (k / 32) * n, mn = m * n, mi = mi);
        let defs = format!("#define BTG 8\n#define MAXSEQ 64\n#define BMAX {m}\n\
                            #define TC_TJ 32\n#define TC_WM 4\n#define TC_KC 64\n#define TC_STAGES 1\n\
                            #define MT_WM 4\n#define MT_NT 64\n#define MT_WN 2\n#define MT_KC 64");
        let got = run(&body, &decls, &defs, &format!("tcm{k}_{n}_{m}"));
        assert_eq!(
            got.len(),
            2 * m * n,
            "expected 2x{} values at k={k} n={n}",
            m * n
        );
        let half = got.len() / 2;
        let mut worst = 0.0f64;
        for (x, y) in got[..half].iter().zip(got[half..].iter()) {
            let mag = y.abs().max(1e-30);
            if mag > 1e-6 {
                worst = worst.max((x - y).abs() / mag);
            }
        }
        assert!(
            worst < 1e-3,
            "the rows-in-M tile disagrees with bq_gemv_h at k={k} n={n} m={m}: worst relative \
             difference {worst:.3e} — the swapped fragment map or the scale pairing is wrong"
        );
    }
}

/// The chunked GDN recurrence — the prefill lever the microbenchmark picked out.
///
/// `bq_gdn_step` measures 0.794 ms per layer per token at B=32, moving 201 MB of recurrent state
/// read+write for 200 MFLOP of arithmetic. That is an intensity of 1 FLOP/byte, and it puts a
/// 512-token prefill at **4.95 TB of state traffic** — the state is re-streamed 24,576 times. The
/// kernel is not the problem (254 GB/s of the 3060's ~360 is 71% of bandwidth); the round trip is.
///
/// `bq_gdn_chunk` is that kernel with its middle wrapped in a T-loop so `sreg` stays live across the
/// chunk: the state is read once, advanced T times in registers, and written once. The arithmetic is
/// untouched — same operations, same strided lane distribution, same `__shfl_down` reduction tree —
/// so this lock demands BIT-IDENTITY, not a tolerance. Anything less would mean the recurrence drifted,
/// and a 512-token prefill compounds drift 512 times.
#[test]
fn the_chunked_recurrence_is_bit_identical_to_stepping_one_token_at_a_time() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    for &(hv, hk, d, t) in &[
        (4usize, 2usize, 32usize, 6usize),
        (48, 16, 128, 5),
        (4, 2, 32, 1),
    ] {
        let (qn, vn) = (hk * d, hv * d);
        let cw = 2 * qn + vn;
        let cbuf: Vec<f64> = (0..cw * t).map(|i| wv(911, i)).collect();
        let bet: Vec<f64> = (0..hv * t).map(|i| 0.5 + wv(912, i) * 0.2).collect();
        let alp: Vec<f64> = (0..hv * t).map(|i| 0.5 + wv(913, i) * 0.2).collect();
        let s0: Vec<f64> = (0..vn * d).map(|i| wv(914, i)).collect();
        let decls = format!(
            "{}{}{}{}{}",
            arr("CB", &cbuf),
            arr("BET", &bet),
            arr("ALP", &alp),
            arr("S0", &s0),
            format!(
                "__device__ __managed__ float SA[{}], SB[{}], OA[{}], OB[{}];\n",
                vn * d,
                vn * d,
                vn * t,
                vn * t
            )
        );
        let body = format!(
"  for(int i=0;i<{sn};++i){{ SA[i]=S0[i]; SB[i]=S0[i]; }}
  cudaDeviceSynchronize();
  for(int t=0;t<{t};++t)
    bq_gdn_step<<<dim3({hv},({d}+3)/4,1),128>>>(CB+t*{cw},CB+t*{cw}+{qn},CB+t*{cw}+{two_qn},BET+t*{hv},ALP+t*{hv},SA,OA+t*{vn},{d},{hk},{hv},{cw});
  cudaDeviceSynchronize();
  bq_gdn_chunk<<<dim3({hv},BQ_GDN_CHUNK_GY({d}),1),128>>>(CB,CB+{qn},CB+{two_qn},BET,ALP,SB,OB,{d},{hk},{hv},{cw},{t},1);
  cudaDeviceSynchronize();
  for(int i=0;i<{on};++i) printf(\"%.9g \", OA[i]);
  for(int i=0;i<{sn};++i) printf(\"%.9g \", SA[i]);
  for(int i=0;i<{on};++i) printf(\"%.9g \", OB[i]);
  for(int i=0;i<{sn};++i) printf(\"%.9g \", SB[i]);\n",
            sn = vn * d, on = vn * t, two_qn = 2 * qn);
        let got = run(
            &body,
            &decls,
            "#define BTG 8\n#define MAXSEQ 64",
            &format!("bchunk{hv}_{d}_{t}"),
        );
        assert_identical(
            &format!("chunked recurrence HV={hv} D={d} T={t}"),
            &got,
            vn * t + vn * d,
        );
    }
}

/// The mid-chunk state checkpoint must be EXACTLY the state a shorter chunk would have left.
///
/// Speculative decode advances the recurrence through k+1 tokens and then learns how many of them
/// were real. The state is destructive and 4.8 GB at 32 sequences, so it can neither be replayed nor
/// cheaply snapshotted first; `bq_gdn_chunk_mid` writes it on the way past instead, turning rollback
/// into a pointer choice. That is only sound if the checkpoint is what stopping early would have
/// produced -- BIT for bit, because a recurrence compounds every subsequent token on top of it -- and
/// if taking it does not perturb the full-length result.
#[test]
fn the_mid_chunk_checkpoint_is_exactly_the_state_a_shorter_chunk_leaves() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    for &(hv, hk, d, t, mid) in &[
        (4usize, 2usize, 32usize, 5usize, 2usize),
        (48, 16, 128, 3, 1),
        (4, 2, 32, 4, 3),
        (4, 2, 32, 2, 1),
    ] {
        let (qn, vn) = (hk * d, hv * d);
        let cw = 2 * qn + vn;
        let cbuf: Vec<f64> = (0..cw * t).map(|i| wv(921, i)).collect();
        let bet: Vec<f64> = (0..hv * t).map(|i| 0.5 + wv(922, i) * 0.2).collect();
        let alp: Vec<f64> = (0..hv * t).map(|i| 0.5 + wv(923, i) * 0.2).collect();
        let s0: Vec<f64> = (0..vn * d).map(|i| wv(924, i)).collect();
        let decls = format!("{}{}{}{}{}", arr("CB", &cbuf), arr("BET", &bet), arr("ALP", &alp), arr("S0", &s0),
            format!("__device__ __managed__ float SA[{n}], SM[{n}], SB[{n}], SC[{n}], OA[{o}], OB[{o}];\n",
                    n = vn * d, o = vn * t));
        let body = format!(
"  for(int i=0;i<{sn};++i){{ SA[i]=S0[i]; SM[i]=-1.f; SB[i]=S0[i]; SC[i]=S0[i]; }}
  cudaDeviceSynchronize();
  bq_gdn_chunk_mid<<<dim3({hv},BQ_GDN_CHUNK_GY({d}),1),128>>>(CB,CB+{qn},CB+{two_qn},BET,ALP,SA,SM,OA,{d},{hk},{hv},{cw},{t},1,{mid});
  cudaDeviceSynchronize();
  bq_gdn_chunk<<<dim3({hv},BQ_GDN_CHUNK_GY({d}),1),128>>>(CB,CB+{qn},CB+{two_qn},BET,ALP,SB,OB,{d},{hk},{hv},{cw},{t},1);
  cudaDeviceSynchronize();
  bq_gdn_chunk<<<dim3({hv},BQ_GDN_CHUNK_GY({d}),1),128>>>(CB,CB+{qn},CB+{two_qn},BET,ALP,SC,OB,{d},{hk},{hv},{cw},{mid},1);
  cudaDeviceSynchronize();
  for(int i=0;i<{sn};++i) printf(\"%.9g \", SA[i]);
  for(int i=0;i<{sn};++i) printf(\"%.9g \", SM[i]);
  for(int i=0;i<{sn};++i) printf(\"%.9g \", SB[i]);
  for(int i=0;i<{sn};++i) printf(\"%.9g \", SC[i]);\n",
            sn = vn * d, two_qn = 2 * qn);
        let got = run(
            &body,
            &decls,
            "#define BTG 8\n#define MAXSEQ 64",
            &format!("bmid{hv}_{d}_{t}_{mid}"),
        );
        let n = vn * d;
        assert_eq!(got.len(), 4 * n, "expected four state buffers");
        for (i, (a, b)) in got[..n].iter().zip(got[2 * n..3 * n].iter()).enumerate() {
            assert_eq!(
                a, b,
                "[HV={hv} D={d} T={t} MID={mid}] element {i}: taking the checkpoint changed \
                the FULL-length result ({a} != {b}) — the checkpoint must be a pure observation"
            );
        }
        for (i, (a, b)) in got[n..2 * n].iter().zip(got[3 * n..].iter()).enumerate() {
            assert_eq!(a, b, "[HV={hv} D={d} T={t} MID={mid}] element {i}: checkpoint {a} != the state a \
                {mid}-token chunk leaves ({b}) — speculative rollback would resume from a state the \
                model never actually passed through");
        }
        println!("[mid HV={hv} D={d} T={t} MID={mid}] checkpoint == {mid}-token chunk, and the {t}-token result is untouched");
    }
}

/// The MTP draft head's input concat, batched, must be the single-sequence one with an index added.
///
/// The 83.3% accept rate on record was measured against `gk_mtp_cat`'s arithmetic. If the batched
/// form normalises differently the draft is a different draft, and that number stops describing it.
#[test]
fn the_batched_mtp_concat_equals_its_single_sequence_original() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    for &n in &[128usize, 5120] {
        let e: Vec<f64> = (0..n).map(|i| wv(931, i)).collect();
        let h: Vec<f64> = (0..n).map(|i| wv(932, i)).collect();
        let we: Vec<f64> = (0..n).map(|i| 0.5 + wv(933, i)).collect();
        let wh: Vec<f64> = (0..n).map(|i| 0.5 + wv(934, i)).collect();
        let decls = format!(
            "{}{}{}{}{}",
            arr("E_", &e),
            arr("H_", &h),
            arr("WE", &we),
            arr("WH", &wh),
            format!(
                "__device__ __managed__ float OA[{}], OB[{}];\n",
                2 * n,
                2 * n
            )
        );
        let body = format!(
            "  bq_mtp_cat<<<dim3(2,1),256>>>(OA,E_,H_,WE,WH,{n},1e-6f);
  cudaDeviceSynchronize();
  gk_mtp_cat<<<2,256>>>(OB,E_,H_,WE,WH,{n},1e-6f);
  cudaDeviceSynchronize();
  for(int i=0;i<{two_n};++i) printf(\"%.9g \", OA[i]);
  for(int i=0;i<{two_n};++i) printf(\"%.9g \", OB[i]);\n",
            two_n = 2 * n
        );
        let got = run(
            &body,
            &decls,
            "#define BTG 8\n#define MAXSEQ 64",
            &format!("mcat{n}"),
        );
        assert_identical(&format!("mtp concat n={n}"), &got, 2 * n);
    }
}

/// The speculative argmax must choose exactly what decode's argmax chooses.
///
/// Decode folds the position bump into its pick because a step consumes exactly one token. A
/// speculative step cannot: it learns how many it consumed only by checking drafts against these very
/// rows. Splitting the pick out is only safe if it is the SAME pick -- same reduction tree, same
/// first-index-wins tie-break -- or the verified token stream silently stops being the greedy one.
#[test]
fn the_speculative_argmax_picks_what_decode_would_pick() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    for &(v, rows) in &[(1024usize, 4usize), (248320, 3)] {
        // exact ties included on purpose: the tie-break is part of the contract, not an accident
        let lg: Vec<f64> = (0..v * rows)
            .map(|i| if i % 977 == 0 { 0.5 } else { wv(941, i) })
            .collect();
        let decls = format!(
            "{}{}",
            arr("LG", &lg),
            format!("__device__ __managed__ int TA[{rows}], TB[{rows}], PB[{rows}];\n")
        );
        let body = format!(
            "  for(int i=0;i<{rows};++i){{ TA[i]=-1; TB[i]=-1; PB[i]=7; }}
  cudaDeviceSynchronize();
  bq_argmax_rows<<<{rows},256>>>(LG,{v},TA);
  cudaDeviceSynchronize();
  bq_argmax_adv<<<{rows},256>>>(LG,{v},TB,PB);
  cudaDeviceSynchronize();
  for(int i=0;i<{rows};++i) printf(\"%d \", TA[i]);
  for(int i=0;i<{rows};++i) printf(\"%d \", TB[i]);\n"
        );
        let got = run(
            &body,
            &decls,
            "#define BTG 8\n#define MAXSEQ 64",
            &format!("amax{v}_{rows}"),
        );
        assert_eq!(got.len(), 2 * rows, "expected both halves at V={v}");
        for i in 0..rows {
            assert_eq!(got[i], got[rows + i],
                "row {i} at V={v}: speculative argmax picked {} but decode picked {} — the verified \
                 stream would stop being the greedy one", got[i], got[rows + i]);
        }
        println!("[argmax V={v} rows={rows}] speculative pick == decode pick");
    }
}

/// Evaluate the `(size_t)A*B/C` element counts the emitted loader uses, so the fixture never has to
/// restate the emitter's own shape arithmetic.
fn eval_count(expr: &str) -> usize {
    let e = expr.replace("(size_t)", "").replace(['(', ')'], "");
    let mut it = e.split_inclusive(['*', '/']);
    let mut acc: usize = 0;
    let mut op = '*';
    let mut first = true;
    let mut cur = it.next();
    while let Some(tok) = cur {
        let next_op = tok.chars().last().filter(|c| *c == '*' || *c == '/');
        let num: usize = tok
            .trim_end_matches(['*', '/'])
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("count term {tok:?} in {expr:?}"));
        if first {
            acc = num;
            first = false;
        } else if op == '*' {
            acc *= num;
        } else {
            acc /= num;
        }
        if let Some(o) = next_op {
            op = o;
        }
        cur = it.next();
    }
    acc
}

/// ★ THE GATE THE PLAN NAMED AND NOBODY BUILT: a chunked forward must equal sequential decode steps.
///
/// "Verify on the 3060 at tiny scale: one `prefill_chunk(T)` must equal T sequential `decode_step`s —
/// same logits, same final recurrent state. DO NOT MEASURE TTFT BEFORE THAT PASSES." Every TTFT and
/// speculative number on record was taken without it, on the strength of the chunked path *looking*
/// right, and the campaign has already met five fast-and-wrong kernels of which three were FASTER
/// than the correct one.
///
/// It matters twice over now, because a speculative verify IS a chunked forward: if T>1 is not the
/// same model as T=1, then speculation cannot be lossless no matter how correct the accept logic is,
/// and the token divergence seen at 30 tokens on the golden prompt is a DEFECT rather than a tie.
///
/// `Qwen38Cfg::tiny()` cannot be used: d=8 gives `d/32 == 0` Q8_0 scales and the loader reads nothing.
/// This is the smallest structurally faithful model the quantised lane actually admits.
#[test]
fn a_chunked_forward_produces_the_same_tokens_as_stepping_one_at_a_time() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    use logicaffeine_compile::codegen_cuda::qwen38::Qwen38Cfg;
    let cfg = Qwen38Cfg {
        d: 64,
        layers: 4,
        full_attn_every: 4,
        dff: 128,
        vocab: 64,
        h: 2,
        kv: 1,
        hd: 32,
        rope_dim: 16,
        rope_theta: 1.0e7,
        rms_eps: 1e-6,
        gdn_k_heads: 1,
        gdn_v_heads: 2,
        gdn_head_dim: 32,
        conv_taps: 4,
        ctx_len: 64,
    };
    let dir = std::env::temp_dir().join(format!("logos_chunkgate_{}", std::process::id()));
    let wdir = dir.join("w");
    std::fs::create_dir_all(&wdir).expect("tmpdir");
    let prompt: Vec<usize> = vec![3, 11, 5, 29, 17, 2, 40, 8];

    // Emit once just to learn which weight files the loader wants and how big each is.
    let probe = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_pt(
        &cfg,
        wdir.to_str().unwrap(),
        &prompt,
        12,
        1,
        false,
        1,
    );
    let mut made = 0usize;
    for line in probe.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("LDQ(") {
            let parts: Vec<&str> = rest.trim_end_matches(");").split(',').collect();
            let (qn, sn) = (eval_count(parts[1]), eval_count(parts[2]));
            assert!(
                sn > 0,
                "a weight with zero Q8_0 scales means the config is too narrow: {l}"
            );
            let qp = parts[3].trim().trim_matches('"');
            let sp = parts[4].trim().trim_matches('"');
            let q: Vec<u8> = (0..qn).map(|i| (wv(7001, i) * 100.0) as i8 as u8).collect();
            let s: Vec<u8> = (0..sn)
                .flat_map(|i| ((0.02 + wv(7002, i).abs() * 0.02) as f32).to_le_bytes())
                .collect();
            std::fs::write(qp, &q).expect("write .q");
            std::fs::write(sp, &s).expect("write .s");
            made += 2;
        } else if let Some(rest) = l.strip_prefix("LDF(") {
            let parts: Vec<&str> = rest.trim_end_matches(");").split(',').collect();
            let n = eval_count(parts[1]);
            let p = parts[2].trim().trim_matches('"');
            let f: Vec<u8> = (0..n)
                .flat_map(|i| ((0.5 + wv(7003, i) * 0.5) as f32).to_le_bytes())
                .collect();
            std::fs::write(p, &f).expect("write .f");
            made += 1;
        }
    }
    assert!(
        made > 20,
        "expected the loader to name a real weight set; saw {made} files"
    );

    // Same model, same prompt, different chunk widths. T=1 is the form every recorded number used.
    let mut streams: Vec<(usize, String)> = Vec::new();
    for pt in [1usize, 2, 4] {
        let cu =
            logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_pt(
                &cfg,
                wdir.to_str().unwrap(),
                &prompt,
                12,
                1,
                false,
                pt,
            );
        let src = dir.join(format!("pt{pt}.cu"));
        let bin = dir.join(format!("pt{pt}"));
        std::fs::write(&src, &cu).expect("write .cu");
        let sm = std::env::var("LOGOS_SM").unwrap_or_else(|_| "sm_86".to_string());
        let c = Command::new("nvcc")
            .args(["-O3", &format!("-arch={sm}")])
            .arg(&src)
            .arg("-o")
            .arg(&bin)
            .output()
            .expect("nvcc");
        assert!(
            c.status.success(),
            "PT={pt} does not compile:\n{}",
            String::from_utf8_lossy(&c.stderr)
                .lines()
                .filter(|l| l.contains("error"))
                .take(8)
                .collect::<Vec<_>>()
                .join("\n")
        );
        let out = Command::new(&bin).output().expect("run");
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let toks = text
            .lines()
            .find(|l| l.starts_with("SEQ0_TOKENS:"))
            .unwrap_or_else(|| {
                panic!(
                    "PT={pt} printed no token stream:\n{}",
                    String::from_utf8_lossy(&out.stderr)
                        .lines()
                        .take(6)
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            })
            .trim_start_matches("SEQ0_TOKENS:")
            .trim()
            .to_string();
        streams.push((pt, toks));
    }
    // ── AND THE SPECULATIVE LANE ─────────────────────────────────────────────────────────────────
    // Speculative decode is only worth anything if it is LOSSLESS, and losslessness is a measurement.
    // The draft is checked against the model's own pick, so a correct implementation must reproduce
    // the greedy stream exactly; anything else means the accept logic, the rollback, or the hidden
    // state the draft conditions on is wrong. This is the check that found both of those bugs.
    let mut spec_stream: Option<String> = None;
    {
        let cu =
            logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_mtp(
                &cfg,
                wdir.to_str().unwrap(),
                &prompt,
                12,
                1,
                false,
                2,
                true,
            );
        // the draft head lives in blk.{layers}; generate the weights the speculative loader adds
        for line in cu.lines() {
            let l = line.trim();
            if let Some(rest) = l.strip_prefix("LDQ(") {
                let parts: Vec<&str> = rest.trim_end_matches(");").split(',').collect();
                let (qn, sn) = (eval_count(parts[1]), eval_count(parts[2]));
                let (qp, sp) = (
                    parts[3].trim().trim_matches('"'),
                    parts[4].trim().trim_matches('"'),
                );
                if std::path::Path::new(qp).exists() {
                    continue;
                }
                let q: Vec<u8> = (0..qn).map(|i| (wv(7001, i) * 100.0) as i8 as u8).collect();
                let sc: Vec<u8> = (0..sn)
                    .flat_map(|i| ((0.02 + wv(7002, i).abs() * 0.02) as f32).to_le_bytes())
                    .collect();
                std::fs::write(qp, &q).expect("write .q");
                std::fs::write(sp, &sc).expect("write .s");
            } else if let Some(rest) = l.strip_prefix("LDF(") {
                let parts: Vec<&str> = rest.trim_end_matches(");").split(',').collect();
                let (n, p2) = (eval_count(parts[1]), parts[2].trim().trim_matches('"'));
                if std::path::Path::new(p2).exists() {
                    continue;
                }
                let f: Vec<u8> = (0..n)
                    .flat_map(|i| ((0.5 + wv(7003, i) * 0.5) as f32).to_le_bytes())
                    .collect();
                std::fs::write(p2, &f).expect("write .f");
            }
        }
        let (src, bin) = (dir.join("spec.cu"), dir.join("spec"));
        std::fs::write(&src, &cu).expect("write .cu");
        let sm = std::env::var("LOGOS_SM").unwrap_or_else(|_| "sm_86".to_string());
        let c = Command::new("nvcc")
            .args(["-O3", &format!("-arch={sm}")])
            .arg(&src)
            .arg("-o")
            .arg(&bin)
            .output()
            .expect("nvcc");
        assert!(
            c.status.success(),
            "the speculative lane does not compile:\n{}",
            String::from_utf8_lossy(&c.stderr)
                .lines()
                .take(8)
                .collect::<Vec<_>>()
                .join("\n")
        );
        let out = Command::new(&bin)
            .env("LOGOS_MTP_TOKENS", "1")
            .output()
            .expect("run");
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        spec_stream = text
            .lines()
            .find(|l| l.starts_with("MTP_TOKENS:"))
            .map(|l| l.trim_start_matches("MTP_TOKENS:").trim().to_string());
        assert!(
            spec_stream.is_some(),
            "the speculative lane printed no token stream:\n{}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .take(8)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    let (_, base) = &streams[0];
    for (pt, s) in &streams[1..] {
        assert_eq!(s, base,
            "a chunk width of {pt} produces a DIFFERENT token stream than stepping one token at a \
             time.\n  T=1  {base}\n  T={pt}  {s}\nA chunked forward must be the same model as the \
             unchunked one: TTFT gains measured against it would be gains on a different model, and \
             a speculative verify -- which IS a chunked forward -- could never be lossless.");
    }
    let spec = spec_stream.expect("speculative stream");
    let (sb, ss): (Vec<&str>, Vec<&str>) = (
        base.split_whitespace().collect(),
        spec.split_whitespace().collect(),
    );
    let n = sb.len().min(ss.len());
    let first_bad = (0..n).find(|&i| sb[i] != ss[i]);
    assert!(first_bad.is_none(),
        "SPECULATIVE DECODE IS NOT LOSSLESS: it diverges from greedy at token {}.\n  greedy {base}\n  spec   {spec}\n\
         The draft is checked against the model's own pick, so a correct implementation reproduces the \
         greedy stream exactly. A divergence means the accept test, the state rollback, or the hidden \
         state the draft conditions on is wrong -- and NONE of those show up in a throughput number.",
        first_bad.unwrap());
    println!("chunked forward == sequential decode at T=2 and T=4, and speculation == greedy ({n} tokens)");
}

/// Pre-tiled weights are a PERMUTATION, so the tile must compute the identical thing from them.
///
/// The decode GEMM waits 67% of its time for data (measured in-kernel with `clock64()`, because `ncu`
/// is permission-blocked in RunPod containers) at 42% of a 3180 GB/s pure-read roofline, and nothing
/// that leaves the ACCESS PATTERN alone moves it: row-major weights make a block read TI separate
/// KC-byte streams where a contiguous reader gets 95% of the roofline. Storing each (n-tile, k-chunk)
/// contiguously makes the staging one streaming read.
///
/// Reordering the bytes a GEMM consumes is exactly the kind of change that yields a fast wrong kernel,
/// and this campaign has met eight of those with three FASTER than correct. So the bar is bit-identity
/// against the row-major path, not a tolerance.
#[test]
fn the_pre_tiled_weight_layout_computes_exactly_what_row_major_does() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    for &(k, n, m) in &[
        (1024usize, 256usize, 32usize),
        (2048, 200, 32),
        (512, 129, 32),
    ] {
        let wq: Vec<f64> = (0..k * n).map(|i| (wv(2601, i) * 100.0).round()).collect();
        let ws: Vec<f64> = (0..(k / 32) * n)
            .map(|i| 0.01 + wv(2602, i).abs() * 0.01)
            .collect();
        let xh: Vec<f64> = (0..k * m).map(|i| (wv(2603, i) * 100.0).round()).collect();
        let xs: Vec<f64> = (0..(k / 32) * m)
            .map(|i| 0.01 + wv(2604, i).abs() * 0.01)
            .collect();
        let ti = "(16*(8/(TC_TJ/8))*TC_WM)";
        // the tiled buffers are padded up to whole tiles, so size them from the tile geometry
        let decls = format!(
            "{}{}{}{}{}",
            arr("WQF", &wq),
            arr("WSF", &ws),
            arr("XHF", &xh),
            arr("XS", &xs),
            format!(
                "__device__ __managed__ signed char WQ[{}], XH[{}], WQT[{}];\n\
                     __device__ __managed__ __half WS[{}], WST[{}];\n\
                     __device__ __managed__ float YA[{}], YB[{}];\n",
                k * n,
                k * m,
                k * (n + 512),
                (k / 32) * n,
                (k / 32) * (n + 512),
                m * n,
                m * n
            )
        );
        let body = format!(
"  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{km};++i) XH[i]=(signed char)XHF[i];
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  cudaDeviceSynchronize();
  const int TIv={ti};
  bq_tile_w<<<256,256>>>(WQT,WQ,{n},{k},TIv,TC_KC);
  bq_tile_ws<<<256,256>>>(WST,WS,{n},{k},TIv,TC_KC);
  cudaDeviceSynchronize();
  bq_gemm_tc1<<<dim3(({n}+TIv-1)/TIv,1,1),256>>>(YA,WQ,WS,XH,XS,{k},{n},{m},{n},(float*)0,1);
  cudaDeviceSynchronize();
  bq_gemm_tc1_tiled<<<dim3(({n}+TIv-1)/TIv,1,1),256>>>(YB,WQT,WST,XH,XS,{k},{n},{m},{n},(float*)0,1);
  cudaDeviceSynchronize();
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YB[i]);\n",
            kn = k * n, km = k * m, nb = (k / 32) * n, mn = m * n, ti = ti);
        let defs = format!("#define BTG 8\n#define MAXSEQ 64\n#define BMAX {m}\n\
                            #define TC_TJ {m}\n#define TC_WM 4\n#define TC_KC 64\n#define TC_STAGES 2");
        let got = run(&body, &decls, &defs, &format!("tiled{k}_{n}_{m}"));
        assert_identical(&format!("tiled weights k={k} n={n} m={m}"), &got, m * n);
    }
}

/// Where Q4 is LOSSLESS it must equal the int8 tile bit for bit.
///
/// The decode GEMM is memory-bound end to end — 42% of a measured 3180 GB/s pure-read roofline, with
/// every instruction-side lever priced at 5% or less. The only thing that moves a memory-bound kernel
/// is moving less, and Q4 carries 0.5625 bytes per weight against Q8_0's 1.0625.
///
/// The requantiser scales a block by its own maximum: with `m == 7` the nibble IS the int8 value and
/// the Q4 scale IS the Q8_0 scale, so the two lanes must agree EXACTLY. That turns a lossy format
/// into an exactly-checkable one over a chosen input, which is worth far more than a tolerance —
/// a tolerance would have passed the fp8 tile that measured rel_L2 1.001 against the oracle.
#[test]
fn where_q4_is_lossless_it_equals_the_int8_tile_exactly() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    for &(k, n, m) in &[
        (1024usize, 256usize, 32usize),
        (2048, 200, 32),
        (512, 129, 32),
    ] {
        // every 32-block spans [-7,7] AND attains 7, so the 4-bit rescale is the identity
        let wq: Vec<f64> = (0..k * n)
            .map(|i| {
                if i % 32 == 0 {
                    7.0
                } else {
                    (wv(2701, i) * 7.0).round().clamp(-7.0, 7.0)
                }
            })
            .collect();
        let ws: Vec<f64> = (0..(k / 32) * n)
            .map(|i| 0.01 + wv(2702, i).abs() * 0.01)
            .collect();
        let xh: Vec<f64> = (0..k * m).map(|i| (wv(2703, i) * 100.0).round()).collect();
        let xs: Vec<f64> = (0..(k / 32) * m)
            .map(|i| 0.01 + wv(2704, i).abs() * 0.01)
            .collect();
        let ti = "(16*(8/(TC_TJ/8))*TC_WM)";
        let decls = format!(
            "{}{}{}{}{}",
            arr("WQF", &wq),
            arr("WSF", &ws),
            arr("XHF", &xh),
            arr("XS", &xs),
            format!(
                "__device__ __managed__ signed char WQ[{}], XH[{}], Q4[{}];\n\
                     __device__ __managed__ __half WS[{}], Q4S[{}];\n\
                     __device__ __managed__ float YA[{}], YB[{}];\n",
                k * n,
                k * m,
                k * n / 2,
                (k / 32) * n,
                (k / 32) * n,
                m * n,
                m * n
            )
        );
        let body = format!(
            "  for(int i=0;i<{kn};++i) WQ[i]=(signed char)WQF[i];
  for(int i=0;i<{km};++i) XH[i]=(signed char)XHF[i];
  for(int i=0;i<{nb};++i) WS[i]=__float2half(WSF[i]);
  cudaDeviceSynchronize();
  bq_q8_to_q4<<<256,256>>>((unsigned char*)Q4,Q4S,WQ,WS,{n},{k});
  cudaDeviceSynchronize();
  bq_gemm_tc1<<<dim3(({n}+{ti}-1)/{ti},1,1),256>>>(YA,WQ,WS,XH,XS,{k},{n},{m},{n},(float*)0,1);
  cudaDeviceSynchronize();
  bq_gemm_tc1_q4<<<dim3(({n}+{ti}-1)/{ti},1,1),256>>>(YB,Q4,Q4S,XH,XS,{k},{n},{m},{n},(float*)0,1);
  cudaDeviceSynchronize();
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YA[i]);
  for(int i=0;i<{mn};++i) printf(\"%.9g \", YB[i]);\n",
            kn = k * n,
            km = k * m,
            nb = (k / 32) * n,
            mn = m * n,
            ti = ti
        );
        let defs = format!("#define BQ_ENABLE_Q4 1\n#define BTG 8\n#define MAXSEQ 64\n#define BMAX {m}\n\
                            #define TC_TJ {m}\n#define TC_WM 4\n#define TC_KC 64\n#define TC_STAGES 2");
        let got = run(&body, &decls, &defs, &format!("q4_{k}_{n}_{m}"));
        assert_identical(&format!("Q4 lossless k={k} n={n} m={m}"), &got, m * n);
    }
}

/// The gated recurrence, measured against an f64 CPU oracle — the accuracy gate any FASTER recurrence
/// must clear.
///
/// The prefill bottleneck is this scan: `emit_gdn_microbench` puts it at 0.1811 ms/layer/token, i.e.
/// ~4.5 s across 48 GDN layers and a 512-token prompt, and it is SEQUENTIAL in T. The replacement is
/// the chunked delta rule, which closes the same recurrence in GEMMs that are parallel across the
/// chunk — and which **re-associates the arithmetic**, so it cannot be gated on bit-identity the way
/// `the_chunked_recurrence_is_bit_identical_to_stepping_one_token_at_a_time` gates the current form.
///
/// This campaign already paid for the lesson: `TC_KC=48` ran 10% FASTER by computing a different model
/// and reported BATCH_AGREEMENT 32/32 while every sequence was equally wrong. So the rule is accuracy
/// against an f64 oracle, never token identity alone — and the oracle has to exist BEFORE the kernel
/// that needs it. This test establishes the baseline the current scan achieves, so a delta-rule
/// implementation can be held to it rather than to a number invented after the fact.
#[test]
fn the_gated_recurrence_matches_an_f64_oracle() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    for &(hv, hk, d, t) in &[(4usize, 2usize, 32usize, 8usize), (48, 16, 128, 4)] {
        let (qn, vn) = (hk * d, hv * d);
        let cw = 2 * qn + vn;
        let cbuf: Vec<f64> = (0..cw * t).map(|i| wv(4711, i)).collect();
        let bet: Vec<f64> = (0..hv * t).map(|i| 0.5 + wv(4712, i) * 0.2).collect();
        let alp: Vec<f64> = (0..hv * t).map(|i| 0.5 + wv(4713, i) * 0.2).collect();
        let s0: Vec<f64> = (0..vn * d).map(|i| wv(4714, i)).collect();

        // ── the oracle: the same recurrence in f64, straight from the kernel's own lines ──
        //   u = S·k ; w = S·q ; kq = k·q ; coef = β(v − αu) ; O = (αw + coef·kq)/√D ; S ← αS + coef·kᵀ
        let mut s = s0.clone();
        let mut oracle = vec![0.0f64; vn * t];
        let inv_sqrt_d = 1.0 / (d as f64).sqrt();
        for step in 0..t {
            let base = step * cw;
            for h in 0..hv {
                let g = h % hk; // k/q heads are shared across v-heads
                let (alpha, beta) = (alp[step * hv + h], bet[step * hv + h]);
                // the launch is (Q, Kk, V) = (CB, CB+qn, CB+2qn) -- Q first, K second
                let q = &cbuf[base + g * d..base + g * d + d];
                let k = &cbuf[base + qn + g * d..base + qn + g * d + d];
                let v = &cbuf[base + 2 * qn + h * d..base + 2 * qn + h * d + d];
                let kq: f64 = (0..d).map(|b| k[b] * q[b]).sum();
                for a in 0..d {
                    let row = (h * d + a) * d;
                    let u: f64 = (0..d).map(|b| s[row + b] * k[b]).sum();
                    let w: f64 = (0..d).map(|b| s[row + b] * q[b]).sum();
                    let coef = beta * (v[a] - alpha * u);
                    oracle[step * vn + h * d + a] = (alpha * w + coef * kq) * inv_sqrt_d;
                    for b in 0..d {
                        s[row + b] = alpha * s[row + b] + coef * k[b];
                    }
                }
            }
        }

        let decls = format!(
            "{}{}{}{}{}",
            arr("CB", &cbuf),
            arr("BET", &bet),
            arr("ALP", &alp),
            arr("S0", &s0),
            format!(
                "__device__ __managed__ float SB[{}], OB[{}];\n",
                vn * d,
                vn * t
            )
        );
        let body = format!(
"  for(int i=0;i<{sn};++i) SB[i]=S0[i];
  cudaDeviceSynchronize();
  bq_gdn_chunk<<<dim3({hv},BQ_GDN_CHUNK_GY({d}),1),128>>>(CB,CB+{qn},CB+{two_qn},BET,ALP,SB,OB,{d},{hk},{hv},{cw},{t},1);
  cudaDeviceSynchronize();
  for(int i=0;i<{on};++i) printf(\"%.9g \", OB[i]);
  for(int i=0;i<{sn};++i) printf(\"%.9g \", SB[i]);\n",
            sn = vn * d, on = vn * t, two_qn = 2 * qn);
        let got = run(
            &body,
            &decls,
            "#define BTG 8\n#define MAXSEQ 64",
            &format!("gdnoracle{hv}_{d}_{t}"),
        );
        assert_eq!(
            got.len(),
            vn * t + vn * d,
            "expected outputs then final state"
        );

        let (num, den) = got[..vn * t]
            .iter()
            .zip(oracle.iter())
            .fold((0.0f64, 0.0f64), |(n, dd), (g, o)| {
                (n + (g - o) * (g - o), dd + o * o)
            });
        let rel = (num / den.max(1e-300)).sqrt();
        // fp32 accumulation over D=128 products; the scan is the shipped arithmetic, so this is the
        // BASELINE a replacement must match or beat, not an aspiration.
        assert!(rel < 1e-5,
            "[gdn HV={hv} D={d} T={t}] rel_L2 {rel:.3e} against the f64 oracle — the shipped scan should \
             sit near fp32 rounding; anything larger means the kernel, not the format, is losing bits");
        println!("[gdn HV={hv} D={d} T={t}] rel_L2 vs f64 oracle = {rel:.3e}");
    }
}

/// The CHUNKED DELTA RULE must equal the sequential scan, proved in f64 before any CUDA is written.
///
/// The scan is prefill's dominant cost (0.1811 ms/layer/token => ~4.5 s over 48 layers x 512 tokens)
/// and it is sequential in T. The delta rule closes the same recurrence in a unit-triangular solve plus
/// GEMMs, all parallel across the chunk — the form vLLM's GDN path uses.
///
/// Deriving it: with A_t = prod α, S_t = A_t S₀ + Σ_{j<=t} (A_t/A_j) c_j k_jᵀ, and substituting into
/// c_t = β_t(v_t − α_t S_{t-1} k_t) gives, using α_t A_{t-1} = A_t,
///
///     c_t + Σ_{j<t} β_t (A_t/A_j)(k_j·k_t) c_j  =  β_t (v_t − A_t S₀k_t)
///
/// i.e. (I + M) C = B with M STRICTLY LOWER triangular — a forward substitution, not a general solve.
/// The outputs and the final state then fall out as two more contractions.
///
/// This test is the algebra gate. It runs on the CPU in f64 with no GPU, so it isolates "is the
/// mathematics right" from "is the kernel right" — the two failure modes this campaign keeps conflating
/// (a fast wrong kernel passed every correctness gate once already).
#[test]
fn the_chunked_delta_rule_equals_the_sequential_gated_recurrence() {
    for &(d, t) in &[(32usize, 8usize), (128, 16), (64, 1), (32, 64)] {
        let k: Vec<f64> = (0..d * t).map(|i| wv(5101, i) * 0.3).collect();
        let q: Vec<f64> = (0..d * t).map(|i| wv(5102, i) * 0.3).collect();
        let v: Vec<f64> = (0..d * t).map(|i| wv(5103, i)).collect();
        let bet: Vec<f64> = (0..t).map(|i| 0.5 + wv(5104, i) * 0.2).collect();
        // decays in (0,1): the product A_t must not blow up across the chunk
        let alp: Vec<f64> = (0..t).map(|i| 0.85 + wv(5105, i) * 0.1).collect();
        let s0: Vec<f64> = (0..d * d).map(|i| wv(5106, i) * 0.1).collect();
        let inv_sqrt_d = 1.0 / (d as f64).sqrt();

        // ── reference: the sequential scan, exactly as bq_gdn_chunk does it ──
        let mut s = s0.clone();
        let mut o_ref = vec![0.0f64; d * t];
        for step in 0..t {
            let (al, be) = (alp[step], bet[step]);
            let (kt, qt, vt) = (
                &k[step * d..][..d],
                &q[step * d..][..d],
                &v[step * d..][..d],
            );
            let kq: f64 = (0..d).map(|b| kt[b] * qt[b]).sum();
            let mut coef = vec![0.0f64; d];
            for a in 0..d {
                let u: f64 = (0..d).map(|b| s[a * d + b] * kt[b]).sum();
                let w: f64 = (0..d).map(|b| s[a * d + b] * qt[b]).sum();
                coef[a] = be * (vt[a] - al * u);
                o_ref[step * d + a] = (al * w + coef[a] * kq) * inv_sqrt_d;
            }
            for a in 0..d {
                for b in 0..d {
                    s[a * d + b] = al * s[a * d + b] + coef[a] * kt[b];
                }
            }
        }
        let s_ref = s;

        // ── the delta rule ──
        let mut acc = vec![0.0f64; t]; // A_t = prod_{i<=t} alpha_i
        let mut run = 1.0;
        for i in 0..t {
            run *= alp[i];
            acc[i] = run;
        }
        let gate = |ti: usize, j: usize| acc[ti] / acc[j]; // A_t / A_j, j <= t

        // B_t = beta_t (v_t - A_t S0 k_t)
        let mut bmat = vec![0.0f64; t * d];
        for ti in 0..t {
            let kt = &k[ti * d..][..d];
            for a in 0..d {
                let s0k: f64 = (0..d).map(|b| s0[a * d + b] * kt[b]).sum();
                bmat[ti * d + a] = bet[ti] * (v[ti * d + a] - acc[ti] * s0k);
            }
        }
        // C = (I + M)^{-1} B by forward substitution; M[t][j] = beta_t (A_t/A_j)(k_j . k_t), j < t
        let mut c = vec![0.0f64; t * d];
        for ti in 0..t {
            let kt = &k[ti * d..][..d];
            let mut row = bmat[ti * d..][..d].to_vec();
            for j in 0..ti {
                let kk: f64 = (0..d).map(|b| k[j * d + b] * kt[b]).sum();
                let m = bet[ti] * gate(ti, j) * kk;
                for a in 0..d {
                    row[a] -= m * c[j * d + a];
                }
            }
            c[ti * d..][..d].copy_from_slice(&row);
        }
        // O_t = [A_t (S0 q_t) + sum_{j<=t} (A_t/A_j)(k_j . q_t) C_j] / sqrt(D)
        let mut o_new = vec![0.0f64; d * t];
        for ti in 0..t {
            let qt = &q[ti * d..][..d];
            for a in 0..d {
                let s0q: f64 = (0..d).map(|b| s0[a * d + b] * qt[b]).sum();
                let mut acc_o = acc[ti] * s0q;
                for j in 0..=ti {
                    let kqj: f64 = (0..d).map(|b| k[j * d + b] * qt[b]).sum();
                    acc_o += gate(ti, j) * kqj * c[j * d + a];
                }
                o_new[ti * d + a] = acc_o * inv_sqrt_d;
            }
        }
        // S_C = A_C S0 + sum_j (A_C/A_j) C_j k_j^T
        let mut s_new = vec![0.0f64; d * d];
        for a in 0..d {
            for b in 0..d {
                let mut acc_s = acc[t - 1] * s0[a * d + b];
                for j in 0..t {
                    acc_s += gate(t - 1, j) * c[j * d + a] * k[j * d + b];
                }
                s_new[a * d + b] = acc_s;
            }
        }

        let rel = |x: &[f64], y: &[f64]| -> f64 {
            let (n, dd) = x.iter().zip(y).fold((0.0f64, 0.0f64), |(n, dd), (a, b)| {
                (n + (a - b) * (a - b), dd + b * b)
            });
            (n / dd.max(1e-300)).sqrt()
        };
        let (ro, rs) = (rel(&o_new, &o_ref), rel(&s_new, &s_ref));
        assert!(ro < 1e-11 && rs < 1e-11,
            "[delta rule D={d} T={t}] outputs rel_L2 {ro:.3e}, state rel_L2 {rs:.3e} against the \
             sequential scan — in f64 these are the SAME recurrence, so anything above rounding means \
             the algebra is wrong, not the precision");
        println!(
            "[delta rule D={d} T={t}] outputs {ro:.3e}, state {rs:.3e} vs the sequential scan"
        );
    }
}

/// The delta-rule KERNEL against the same f64 oracle the scan is held to.
///
/// The algebra is already proved exactly (`the_chunked_delta_rule_equals_the_sequential_gated_recurrence`,
/// machine epsilon in f64), so anything this finds is an implementation bug — indexing, the shared Gram
/// matrices, or the state write-back — and never the mathematics. That separation is the whole reason
/// the CPU proof came first.
///
/// The bar is the scan's own measured accuracy, ~1.6e-7 (see `the_gated_recurrence_matches_an_f64_oracle`).
/// The delta rule re-associates the recurrence, so it will not be bit-identical and must not be gated as
/// if it were; it is held to being no worse in `rel_L2` than the form it replaces.
#[test]
fn the_delta_rule_kernel_matches_the_f64_oracle() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    for &(hv, hk, d, t) in &[
        (4usize, 2usize, 32usize, 8usize),
        (4, 2, 32, 16),
        (48, 16, 128, 4),
    ] {
        let (qn, vn) = (hk * d, hv * d);
        let cw = 2 * qn + vn;
        let cbuf: Vec<f64> = (0..cw * t).map(|i| wv(6201, i) * 0.3).collect();
        let bet: Vec<f64> = (0..hv * t).map(|i| 0.5 + wv(6202, i) * 0.2).collect();
        let alp: Vec<f64> = (0..hv * t).map(|i| 0.88 + wv(6203, i) * 0.06).collect();
        let s0: Vec<f64> = (0..vn * d).map(|i| wv(6204, i) * 0.1).collect();

        // the f64 scan oracle, the shipped arithmetic
        let mut s = s0.clone();
        let mut oracle = vec![0.0f64; vn * t];
        let inv = 1.0 / (d as f64).sqrt();
        for step in 0..t {
            let base = step * cw;
            for h in 0..hv {
                let g = h % hk;
                let (al, be) = (alp[step * hv + h], bet[step * hv + h]);
                let q = &cbuf[base + g * d..][..d];
                let k = &cbuf[base + qn + g * d..][..d];
                let v = &cbuf[base + 2 * qn + h * d..][..d];
                let kq: f64 = (0..d).map(|b| k[b] * q[b]).sum();
                for a in 0..d {
                    let row = (h * d + a) * d;
                    let u: f64 = (0..d).map(|b| s[row + b] * k[b]).sum();
                    let w: f64 = (0..d).map(|b| s[row + b] * q[b]).sum();
                    let coef = be * (v[a] - al * u);
                    oracle[step * vn + h * d + a] = (al * w + coef * kq) * inv;
                    for b in 0..d {
                        s[row + b] = al * s[row + b] + coef * k[b];
                    }
                }
            }
        }

        let shmem = (2 * t * t + t + 2 * t * d) * 4;
        let decls = format!(
            "{}{}{}{}{}",
            arr("CB", &cbuf),
            arr("BET", &bet),
            arr("ALP", &alp),
            arr("S0", &s0),
            format!(
                "__device__ __managed__ float SD[{}], OD[{}];\n",
                vn * d,
                vn * t
            )
        );
        let body = format!(
"  for(int i=0;i<{sn};++i) SD[i]=S0[i];
  cudaDeviceSynchronize();
  bq_gdn_delta<<<dim3({hv},1,1),128,{shmem}>>>(CB,CB+{qn},CB+{two_qn},BET,ALP,SD,OD,{d},{hk},{hv},{cw},{t},1);
  cudaDeviceSynchronize();
  for(int i=0;i<{on};++i) printf(\"%.9g \", OD[i]);
  for(int i=0;i<{sn};++i) printf(\"%.9g \", SD[i]);\n",
            sn = vn * d, on = vn * t, two_qn = 2 * qn);
        let got = run(
            &body,
            &decls,
            "#define BTG 8\n#define MAXSEQ 64",
            &format!("gdndelta{hv}_{d}_{t}"),
        );
        assert_eq!(
            got.len(),
            vn * t + vn * d,
            "expected outputs then final state"
        );

        let (num, den) = got[..vn * t]
            .iter()
            .zip(oracle.iter())
            .fold((0.0f64, 0.0f64), |(n, dd), (g, o)| {
                (n + (g - o) * (g - o), dd + o * o)
            });
        let rel = (num / den.max(1e-300)).sqrt();
        assert!(rel < 1e-5,
            "[delta kernel HV={hv} D={d} T={t}] rel_L2 {rel:.3e} vs the f64 oracle. The algebra is \
             already proved exact in f64, so this is an implementation bug — indexing, the shared Gram \
             matrices, or the state write-back.");
        println!("[delta kernel HV={hv} D={d} T={t}] rel_L2 vs f64 oracle = {rel:.3e}");
    }
}

/// e4m3 round-to-nearest: 3 mantissa bits, subnormals to 2^-9, finite max 448.
fn e4m3(v: f64) -> f64 {
    if v == 0.0 {
        return 0.0;
    }
    let (s, a) = (v.signum(), v.abs());
    let e = a.log2().floor().max(-6.0);
    let step = (e - 3.0).exp2();
    s * ((a / step).round() * step).min(448.0)
}

/// Quantise in blocks of `block` along the contiguous axis and dequantise: `amax/peak`, round, scale back.
fn blocked(x: &[f64], block: usize, peak: f64, e4: bool) -> Vec<f64> {
    let mut out = vec![0.0; x.len()];
    for c in x
        .chunks(block)
        .enumerate()
        .map(|(i, c)| (i * block, c))
        .collect::<Vec<_>>()
    {
        let (off, chunk) = c;
        let amax = chunk.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let sc = if amax > 0.0 { amax / peak } else { 1.0 };
        for (i, v) in chunk.iter().enumerate() {
            out[off + i] = if e4 {
                e4m3(v / sc) * sc
            } else {
                (v / sc).round() * sc
            };
        }
    }
    out
}

/// ★ THE PRICE OF THE DeepGEMM LANE, PINNED.
///
/// This lock exists because I quoted a number that did not measure what I said it did. The standalone
/// DeepGEMM bench reported `rel_L2 = 1.07e-04` and I carried it into the emitter as evidence that the
/// routed lane was accurate. Its own comment says otherwise -- "reference in double, from the SAME fp8
/// bytes and scales the kernel saw". It measured the kernel's ACCUMULATION against a recomputation of
/// the already-quantised inputs. The int8 -> fp8 quantisation loss, which is the entire accuracy
/// question, was never in that number at all.
///
/// The real relationship, computed here from f64 truth with no GPU: e4m3 carries 3 mantissa bits
/// against int8's 8, and per-128 blocking is coarser than Q8_0's per-32, so the DeepGEMM lane is
/// roughly TWICE as lossy as the lane it replaces. That is the honest cost of its 1.43x prefill, it is
/// the reason its token stream diverges from ours on a 64-layer gated recurrence, and it is the number
/// that must be compared against vLLM's fp8 -- which pays the same e4m3 price -- rather than against
/// our own int8, which does not.
///
/// If this ever reads as 1e-4, the reference has been recomputed from quantised inputs again.
#[test]
fn the_deepgemm_fp8_lane_costs_about_twice_the_quantisation_error_of_the_int8_lane() {
    let (m, n, k) = (64usize, 512usize, 2048usize);
    let mut st = 0x2545F4914F6CDD1Du64;
    let mut rnd = || {
        st ^= st << 13;
        st ^= st >> 7;
        st ^= st << 17;
        ((st >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
    };

    // Activations carry the heavy outlier channels every transformer has; weights do not.
    let mut x = vec![0.0f64; m * k];
    for v in x.iter_mut() {
        *v = rnd();
    }
    for c in (0..k).step_by(k / 16) {
        for r in 0..m {
            x[r * k + c] *= 40.0;
        }
    }
    let mut w = vec![0.0f64; n * k];
    for v in w.iter_mut() {
        *v = rnd() * 0.02;
    }

    let gemm = |a: &[f64], b: &[f64]| -> Vec<f64> {
        let mut y = vec![0.0f64; m * n];
        for r in 0..m {
            for col in 0..n {
                let mut acc = 0.0;
                for i in 0..k {
                    acc += a[r * k + i] * b[col * k + i];
                }
                y[r * n + col] = acc;
            }
        }
        y
    };
    let rel = |y: &[f64], r: &[f64]| -> f64 {
        let (mut num, mut den) = (0.0, 0.0);
        for i in 0..y.len() {
            let d = y[i] - r[i];
            num += d * d;
            den += r[i] * r[i];
        }
        (num / den).sqrt()
    };

    let truth = gemm(&x, &w);
    let int8 = rel(
        &gemm(
            &blocked(&x, 32, 127.0, false),
            &blocked(&w, 32, 127.0, false),
        ),
        &truth,
    );
    let fp8 = rel(
        &gemm(
            &blocked(&x, 128, 448.0, true),
            &blocked(&w, 128, 448.0, true),
        ),
        &truth,
    );

    assert!(
        int8 > 1e-3 && int8 < 5e-2,
        "the int8 lane's own quantisation error moved: {int8:.3e} (expected ~1e-2)"
    );
    assert!(fp8 > int8,
        "fp8 e4m3 carries 3 mantissa bits against int8's 8 and blocks 4x coarser — it cannot be more \
         accurate. int8 {int8:.3e}, fp8 {fp8:.3e}: the reference is measuring the wrong thing");
    let ratio = fp8 / int8;
    assert!((1.3..=4.0).contains(&ratio),
        "the DeepGEMM lane is {ratio:.2}x the int8 lane's error (int8 {int8:.3e}, fp8 {fp8:.3e}); the \
         measured relationship is ~2x, so this is either a changed blocking or a broken reference");
    assert!(fp8 > 1e-3,
        "fp8 end-to-end error came out at {fp8:.3e}. A number near 1e-4 is the signature of a reference \
         recomputed FROM the quantised bytes, which measures accumulation and not quantisation at all");
}

/// ★ THE RECURRENT STATE IS THE LARGEST NON-GEMM TERM IN A DECODE STEP, AND IT CAN BE HALVED.
///
/// Gated DeltaNet carries 48 v-heads x 128 x 128 floats per layer — 151 MB per SEQUENCE, 9.66 GB of
/// read-modify-write per step at B=32, and far too large to sit in L2 the way the split-K partials do.
/// Storing it at half width halves both, and doubles the concurrency ceiling, which is itself a serving
/// metric: vLLM carries the identical state and its own baseline died on "max_num_seqs exceeds
/// available Mamba cache blocks".
///
/// The obvious objection is that error compounds through a recurrence. It does not, and the mechanism
/// is the decay: an error injected at step t is multiplied by alpha^(T-t) before it reaches the output,
/// so old error dies rather than accumulating. This test is that claim, measured — the drift at T=640
/// is within a small factor of the drift at T=128, not 5x it.
///
/// It also settles WHICH half width. bf16's 8 mantissa bits are not enough; f16's 10 are. That is a
/// 40-100x difference in output error and it is the whole design decision.
#[test]
fn the_gated_recurrence_tolerates_a_half_width_state_but_only_with_ten_mantissa_bits() {
    const D: usize = 128;
    // xorshift so the sequence is identical across the four runs being compared
    fn seq(seed: u64, n: usize) -> Vec<f64> {
        let mut s = seed | 1;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                ((s >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
            })
            .collect()
    }
    fn f16_round(v: f64) -> f64 {
        half_bits(v as f32) as f64
    }
    fn half_bits(x: f32) -> f32 {
        // round-to-nearest-even into 10 mantissa bits, with f16's range
        if x == 0.0 || !x.is_finite() {
            return x;
        }
        let a = x.abs();
        if a >= 65504.0 {
            return x.signum() * 65504.0;
        }
        let e = a.log2().floor().max(-14.0);
        let step = (e - 10.0).exp2();
        x.signum() * ((a / step).round() * step)
    }
    fn bf16_round(v: f64) -> f64 {
        f32::from_bits((v as f32).to_bits() & 0xFFFF_0000) as f64
    }

    let run = |t: usize, alpha: f64, mode: u8| -> Vec<f64> {
        let mut s = vec![0.0f64; D * D];
        let mut out = Vec::with_capacity(t * D);
        let r = seq(0x9E3779B97F4A7C15, t * (3 * D + 1));
        for step in 0..t {
            let o = step * (3 * D + 1);
            let mut k: Vec<f64> = r[o..o + D].to_vec();
            let nk = k.iter().map(|v| v * v).sum::<f64>().sqrt();
            for v in k.iter_mut() {
                *v /= nk;
            }
            let mut q: Vec<f64> = r[o + D..o + 2 * D].to_vec();
            let nq = q.iter().map(|v| v * v).sum::<f64>().sqrt();
            for v in q.iter_mut() {
                *v /= nq;
            }
            let vv = &r[o + 2 * D..o + 3 * D];
            let beta = 0.5 + 0.4 * (r[o + 3 * D] * 0.5 + 0.5);

            let mut u = vec![0.0f64; D];
            let mut w = vec![0.0f64; D];
            for i in 0..D {
                let row = &s[i * D..(i + 1) * D];
                u[i] = row.iter().zip(&k).map(|(a, b)| a * b).sum();
                w[i] = row.iter().zip(&q).map(|(a, b)| a * b).sum();
            }
            let kq: f64 = k.iter().zip(&q).map(|(a, b)| a * b).sum();
            let coef: Vec<f64> = (0..D).map(|i| beta * (vv[i] - alpha * u[i])).collect();
            for i in 0..D {
                out.push((alpha * w[i] + coef[i] * kq) / (D as f64).sqrt());
            }
            for i in 0..D {
                for j in 0..D {
                    let nv = alpha * s[i * D + j] + coef[i] * k[j];
                    s[i * D + j] = match mode {
                        1 => nv as f32 as f64,
                        2 => f16_round(nv),
                        3 => bf16_round(nv),
                        _ => nv,
                    };
                }
            }
        }
        out
    };
    let rel = |a: &[f64], b: &[f64]| -> f64 {
        let (mut n, mut d) = (0.0, 0.0);
        for i in 0..a.len() {
            let e = a[i] - b[i];
            n += e * e;
            d += b[i] * b[i];
        }
        (n / d).sqrt()
    };

    for &alpha in &[0.90f64, 0.99, 0.999] {
        let short = run(128, alpha, 0);
        let long = run(512, alpha, 0);
        let (f16s, f16l) = (
            rel(&run(128, alpha, 2), &short),
            rel(&run(512, alpha, 2), &long),
        );
        let bf16l = rel(&run(512, alpha, 3), &long);
        let f32l = rel(&run(512, alpha, 1), &long);

        assert!(
            f32l < 1e-5,
            "an f32 state drifted {f32l:.3e} at decay {alpha} — the harness is wrong"
        );
        assert!(f16l < 1e-2,
            "an f16 state drifts {f16l:.3e} at decay {alpha}, past the 1e-2 the int8 weights already \
             cost. Halving the state would then be the dominant error in the model, not a free byte");
        assert!(f16l < f16s * 5.0,
            "f16 state drift grew from {f16s:.3e} at T=128 to {f16l:.3e} at T=512 — faster than the \
             decay should allow. The claim that alpha^(T-t) kills old error is what licenses a half \
             width state at all, and this says it does not hold at decay {alpha}");
        assert!(bf16l > f16l * 5.0,
            "bf16 ({bf16l:.3e}) is no longer far worse than f16 ({f16l:.3e}) at decay {alpha}. The \
             choice of ten mantissa bits over eight is the whole design decision here, and if the two \
             have converged then this test has stopped measuring it");
    }
}

/// The KV cache at half width — the other half of the concurrency lever, and the one that makes it 2x.
///
/// Halving the GDN state alone moves the per-sequence footprint at 2K context from 427 MB to 352 MB
/// (1.21x more sequences). It is only 1.21x because the KV cache is the LARGER term there and is kept
/// as `float`: 268 MB against the state's 151. Halving both gives 218 MB, 1.97x. That correction cost
/// this campaign a retraction, so the arithmetic is asserted here rather than recalled.
///
/// The accuracy is measured, not assumed: ~2e-4, FLAT in context length, and almost entirely from V —
/// K in half width costs essentially nothing because softmax is shift-invariant and damps score error.
/// bf16 is again the trap: 8 mantissa bits are ~17x worse than 10.
#[test]
fn the_kv_cache_tolerates_half_width_and_the_concurrency_arithmetic_holds() {
    const HD: usize = 256;
    let mut st = 0xD1B54A32D192ED03u64;
    let mut rnd = || {
        st ^= st << 13;
        st ^= st >> 7;
        st ^= st << 17;
        ((st >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
    };
    fn h16(v: f64) -> f64 {
        if v == 0.0 {
            return 0.0;
        }
        let a = v.abs();
        if a >= 65504.0 {
            return v.signum() * 65504.0;
        }
        let e = a.log2().floor().max(-14.0);
        let step = (e - 10.0).exp2();
        v.signum() * ((a / step).round() * step)
    }
    fn b16(v: f64) -> f64 {
        f32::from_bits((v as f32).to_bits() & 0xFFFF_0000) as f64
    }

    for &l in &[512usize, 2048] {
        let q: Vec<f64> = (0..HD).map(|_| rnd()).collect();
        let k: Vec<f64> = (0..l * HD)
            .map(|_| rnd() * 3.0 / (HD as f64).sqrt())
            .collect();
        let v: Vec<f64> = (0..l * HD).map(|_| rnd()).collect();
        let scale = 1.0 / (HD as f64).sqrt();

        let attend = |kf: &dyn Fn(f64) -> f64, vf: &dyn Fn(f64) -> f64| -> Vec<f64> {
            let mut s: Vec<f64> = (0..l)
                .map(|j| (0..HD).map(|e| q[e] * kf(k[j * HD + e])).sum::<f64>() * scale)
                .collect();
            let m = s.iter().cloned().fold(f64::MIN, f64::max);
            let mut tot = 0.0;
            for x in s.iter_mut() {
                *x = (*x - m).exp();
                tot += *x;
            }
            (0..HD)
                .map(|e| (0..l).map(|j| s[j] * vf(v[j * HD + e])).sum::<f64>() / tot)
                .collect()
        };
        let id = |x: f64| x;
        let refo = attend(&id, &id);
        let rel = |y: &[f64]| -> f64 {
            let (mut n, mut d) = (0.0, 0.0);
            for i in 0..y.len() {
                let e = y[i] - refo[i];
                n += e * e;
                d += refo[i] * refo[i];
            }
            (n / d).sqrt()
        };
        let half = rel(&attend(&h16, &h16));
        let bflt = rel(&attend(&b16, &b16));
        assert!(half < 2e-3,
            "half-width KV drifts {half:.3e} at ctx={l}; the int8 weights already cost 1.27e-2 and this \
             should sit well under that, not approach it");
        assert!(bflt > half * 5.0,
            "bf16 KV ({bflt:.3e}) is no longer clearly worse than f16 ({half:.3e}) at ctx={l} — ten \
             mantissa bits over eight is the design choice and this is what measures it");
    }

    // the concurrency arithmetic the lever is actually sold on, at 2K context
    let (state32, state16, conv) = (151.0f64, 75.5, 7.9);
    let kv32 = 16.0 * 2048.0 * 4.0 * 256.0 * 2.0 * 4.0 / 1e6; // 16 attn layers, 4 kv heads, K and V, f32
    let seats = |s: f64, k: f64| (51_000.0 / (s + k + conv)).floor();
    let (base, both) = (seats(state32, kv32), seats(state16, kv32 / 2.0));
    assert!(
        (base - 119.0).abs() < 2.0,
        "the shipped per-sequence footprint moved: {base} seats"
    );
    assert!(both / base > 1.9,
        "halving BOTH the state and the KV cache is supposed to be ~1.97x the sequences ({base} -> \
         {both}). If this drops, the 2x claim is wrong again — it was already retracted once for \
         counting the state alone, which is only 1.21x");
}

/// The honest tool-skip seam, mirroring `logicaffeine_tests::common::require_tool`. A test that prints
/// `SKIP` and returns reports PASS to cargo, so an absent tool becomes a green result nobody reads —
/// which is why `skip_budget_lock` ratchets those sites down and tells you not to raise its ceiling.
/// This emits the machine-readable marker a CI budget can grep, and turns an absent tool into a LOUD
/// failure when `LOGOS_REQUIRE_TOOLS` demands it.
fn require_tool(tool: &str, available: bool) -> bool {
    if available {
        return true;
    }
    let spec = std::env::var("LOGOS_REQUIRE_TOOLS").unwrap_or_default();
    let required = spec == "1"
        || spec.eq_ignore_ascii_case("all")
        || spec
            .split([',', ' ', '\t'])
            .map(str::trim)
            .any(|t| t == tool);
    assert!(
        !required,
        "required external tool '{tool}' is not available, but LOGOS_REQUIRE_TOOLS demands it — \
         the selected gate fails loud (never a silent SKIP-as-PASS)"
    );
    eprintln!("LOGOS_TOOL_SKIP: {tool}");
    false
}

/// ★ GQA-SHARED ATTENTION IS THE SAME ARITHMETIC, NOT AN APPROXIMATION OF IT.
///
/// Qwen3.8 has 24 query heads over 4 KV heads, and `bq_attn` gives every QUERY head its own block — so
/// six blocks read each KV head's cache. Measured on a 3060 at the decode shape that kernel moves its
/// KV at 43% of the memory ceiling, the one non-GEMM kernel in the step with real headroom.
/// `bq_attn_gqa<G>` serves G of the query heads that share a KV head from one block, loading each K and
/// V row once for all of them. It keeps EVERY per-head accumulation in bq_attn's order — per-lane
/// e-ascending dot partials, the same shuffle tree, the same max and sum reductions, j-ascending V —
/// so the output is bit-identical, which is what licenses it as the default rather than a lane.
///
/// Checked bit for bit at every G that divides the head ratio, through the host launcher that picks G
/// for the machine, and with RAGGED per-sequence lengths — the case a lockstep batch never exercises.
#[test]
fn gqa_shared_attention_is_bit_identical_to_the_per_head_kernel() {
    if !require_tool("nvcc", have_nvcc()) || !require_tool("gpu", have_gpu()) {
        return;
    }
    let (h, kv, hd, nb, maxseq) = (12usize, 2usize, 64usize, 5usize, 64usize);
    let (qs, kd) = (h * hd, kv * hd);
    let pos: Vec<usize> = vec![0, 7, 31, 62, 18]; // ragged: 1..63 keys per sequence
    let q: Vec<f64> = (0..nb * qs).map(|i| wv(3301, i)).collect();
    let k: Vec<f64> = (0..nb * maxseq * kd).map(|i| wv(3302, i)).collect();
    let v: Vec<f64> = (0..nb * maxseq * kd).map(|i| wv(3303, i)).collect();
    let n = nb * qs;
    let decls = format!(
        "{}{}{}__device__ __managed__ float QA[{n}], KA[{kn}], VA[{kn}], OREF[{n}], OG[5][{n}];\n\
                         __device__ __managed__ int POS[{nb}];\n",
        arr("QSRC", &q),
        arr("KSRC", &k),
        arr("VSRC", &v),
        kn = nb * maxseq * kd
    );
    let launch = |g: usize, slot: usize| {
        format!(
        "  {{ const size_t sm=(size_t)({g}*MAXSEQ+{g}*{hd})*4;\n\
         \x20   cudaFuncSetAttribute(bq_attn_gqa<{g}>,cudaFuncAttributeMaxDynamicSharedMemorySize,(int)sm);\n\
         \x20   bq_attn_gqa<{g}><<<dim3({kv}*(({h}/{kv})/{g}),{nb}),256,sm>>>(OG[{slot}],QA,{qs},KA,VA,POS,{h},{kv},{hd},0.125f);\n\
         \x20   cudaDeviceSynchronize(); }}\n")
    };
    let body = format!(
        "  for(int i=0;i<{nb};++i) POS[i]=(int[]){{{p}}}[i];
  for(int i=0;i<{n};++i) QA[i]=QSRC[i];
  for(int i=0;i<{kn};++i){{ KA[i]=KSRC[i]; VA[i]=VSRC[i]; }}
  cudaDeviceSynchronize();
  bq_attn<<<dim3({h},{nb}),256,MAXSEQ*4>>>(OREF,QA,{qs},KA,VA,POS,{h},{kv},{hd},0.125f);
  cudaDeviceSynchronize();
{g1}{g2}{g3}{g6}  bq_attn_launch(0,OG[4],QA,{qs},KA,VA,POS,{h},{kv},{hd},0.125f,{nb});
  cudaDeviceSynchronize();
  for(int i=0;i<{n};++i) printf(\"%.9g \", OREF[i]);
  for(int s=0;s<5;++s) for(int i=0;i<{n};++i) printf(\"%.9g \", OG[s][i]);\n",
        p = pos
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(","),
        kn = nb * maxseq * kd,
        g1 = launch(1, 0),
        g2 = launch(2, 1),
        g3 = launch(3, 2),
        g6 = launch(6, 3)
    );
    let got = run(
        &body,
        &decls,
        &format!("#define BTG 8\n#define MAXSEQ {maxseq}"),
        "gqa_attn",
    );
    assert_eq!(
        got.len(),
        6 * n,
        "expected {} printed values, got {}",
        6 * n,
        got.len()
    );
    for (slot, label) in ["G=1", "G=2", "G=3", "G=6", "bq_attn_launch"]
        .iter()
        .enumerate()
    {
        let off = (slot + 1) * n;
        let bad: Vec<usize> = (0..n)
            .filter(|&i| got[i].to_bits() != got[off + i].to_bits())
            .collect();
        assert!(bad.is_empty(),
            "{label}: {} of {n} outputs differ from bq_attn (first at {}: {} vs {}). GQA sharing must \
             change which block computes a head, never the order its sums are taken in",
            bad.len(), bad[0], got[off + bad[0]], got[bad[0]]);
    }
}
