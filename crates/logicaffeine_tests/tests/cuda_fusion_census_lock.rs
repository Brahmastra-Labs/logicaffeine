//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE FUSIONS THE COMPILED CUDA FORWARD CLAIMS, PROVEN ONE BY ONE.
//!
//! `benchmarks/fusion_parity.json` lists every fusion the leading inference stacks apply and marks a class
//! `compiled` only when the compiled forward (`compile_to_cuda_forward*`, from a program's tensor statements)
//! applies it on its own. This lock is the census half of that claim: for each such class, a small program in the
//! language is compiled and the emitted program is read — the fused kernel is there, and the launches it replaces
//! are not. Where no other lock already runs the fused kernel and compares its numbers, the values half is here
//! too, on the GPU.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;

use std::collections::HashMap;
use std::process::Command;

use logicaffeine_compile::codegen_cuda::qwen_emit::{MoeWeights, Q8Ref};
use logicaffeine_compile::compile::{
    compile_to_cuda_forward, compile_to_cuda_forward_q8, compile_to_cuda_forward_q8_dp4a,
    compile_to_cuda_forward_q8_moe_dp4a, vm_outcome,
};

/// A declared Real tensor with a fixed spread of values.
fn tensor(name: &str, rows: usize, cols: usize, salt: usize) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for p in 0..rows * cols {
        s.push_str(&format!("Set item {} of {name} to {:?}.\n", p + 1, value(p, salt)));
    }
    s
}

fn value(p: usize, salt: usize) -> f64 {
    (((p * 37 + salt * 11) % 101) as f64 - 50.0) / 25.0
}

/// A weight the program declares and the manifest makes int8-resident.
fn declared(name: &str, rows: usize, cols: usize) -> String {
    format!("Let {name} be a {rows} by {cols} tensor of Real.\n")
}

fn q8_ref(dir: &str, tag: &str, in_dim: usize, out_dim: usize) -> Q8Ref {
    Q8Ref {
        quants: format!("{dir}/{tag}.q"),
        scales: format!("{dir}/{tag}.s"),
        in_dim,
        out_dim,
    }
}

fn kernels(cu: &str) -> usize {
    cu.matches("__global__").count()
}

#[test]
fn a_weighted_norm_is_one_row_kernel() {
    let src = format!(
        "## Main\n{}{}Let Y be rmsnorm of X with weight W.\nShow item 1 of Y.\n",
        tensor("X", 2, 64, 1),
        tensor("W", 1, 64, 2)
    );
    let cu = compile_to_cuda_forward(&src).unwrap_or_else(|e| panic!("must lower: {e}"));
    assert!(
        cu.contains("k0_row(const float* x, const float* w"),
        "the weight is applied inside the row kernel:\n{cu}"
    );
    assert_eq!(kernels(&cu), 1, "the norm and its weight are one kernel");
}

#[test]
fn an_add_then_norm_runs_as_one_kernel_and_matches_the_vm_on_the_gpu() {
    let (rows, cols) = (2, 64);
    let mut src = format!(
        "## Main\n{}{}{}Let S be A plus B.\nLet Y be rmsnorm of S with weight W.\n",
        tensor("A", rows, cols, 3),
        tensor("B", rows, cols, 4),
        tensor("W", 1, cols, 5)
    );
    for p in 1..=rows * cols {
        src.push_str(&format!("Show item {p} of Y.\n"));
    }
    let cu = compile_to_cuda_forward(&src).unwrap_or_else(|e| panic!("must lower: {e}"));
    assert!(
        cu.contains("k0_add_row<<<"),
        "the add and the norm are one launch:\n{cu}"
    );
    assert_eq!(kernels(&cu), 1, "no separate add kernel");
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let got = nvcc_run(&cu, "addnorm");
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
    let want: Vec<f64> = vm.output.split_whitespace().map(|s| s.parse().unwrap()).collect();
    assert_eq!(got.len(), want.len());
    for (k, (g, w)) in got.iter().zip(&want).enumerate() {
        assert!(
            (g - w).abs() <= 2e-4 * w.abs().max(1.0),
            "element {k}: {g} on the GPU, {w} in the VM"
        );
    }
}

/// The forward adds into a norm only the norm that is the add's next stage: a sum whose norm comes after another stage
/// launches its own add kernel, and the norm its own row kernel.
#[test]
fn an_add_whose_norm_is_not_the_next_stage_launches_apart() {
    let src = format!(
        "## Main\n{}{}{}{}Let S be A plus B.\nLet T be X hadamard X.\nLet Y be rmsnorm of S with weight W.\nShow item 1 of Y.\nShow item 1 of T.\n",
        tensor("A", 2, 64, 3),
        tensor("B", 2, 64, 4),
        tensor("X", 2, 64, 6),
        tensor("W", 1, 64, 5)
    );
    let cu = compile_to_cuda_forward(&src).unwrap_or_else(|e| panic!("must lower: {e}"));
    assert!(!cu.contains("_add_row"), "the norm is not the add's next stage, so the add is not inside it:\n{cu}");
    assert!(cu.contains("k0_elem<<<") && cu.contains("k2_row<<<"), "the add and the norm launch apart:\n{cu}");
}

/// `Xn = rmsnorm(X)·W` feeding an int8 projection on the int8-activation lane: the norm kernel writes the
/// quantized activation, and the projection launches no quantization kernel of its own.
#[test]
fn a_norm_feeding_an_int8_projection_quantizes_in_its_epilogue() {
    let src = format!(
        "## Main\n{}{}Let Xn be rmsnorm of X with weight W.\n{}Let Y be Xn matmul Wq.\nShow item 1 of Y.\n",
        tensor("X", 1, 64, 6),
        tensor("W", 1, 64, 7),
        declared("Wq", 64, 32)
    );
    let mut q8 = HashMap::new();
    q8.insert("Wq".to_string(), q8_ref("/w", "wq", 64, 32));
    let cu = compile_to_cuda_forward_q8_dp4a(&src, &HashMap::new(), &q8)
        .unwrap_or_else(|e| panic!("must lower: {e}"));
    assert!(
        cu.contains("k0_row(const float* x, const float* w, float* y, signed char* yq, float* ys)"),
        "the norm writes the int8 activation:\n{cu}"
    );
    assert!(
        !cu.contains("k1_qact<<<"),
        "the projection must not quantize its activation again:\n{cu}"
    );
    assert!(cu.contains("k1_q8dp4a<<<"), "the projection is the dp4a GEMV:\n{cu}");
}

#[test]
fn an_add_then_norm_feeding_an_int8_projection_quantizes_in_its_epilogue() {
    let src = format!(
        "## Main\n{}{}{}Let S be A plus B.\nLet Xn be rmsnorm of S with weight W.\n{}Let Y be Xn matmul Wq.\nShow item 1 of Y.\n",
        tensor("A", 1, 64, 8),
        tensor("B", 1, 64, 9),
        tensor("W", 1, 64, 10),
        declared("Wq", 64, 32)
    );
    let mut q8 = HashMap::new();
    q8.insert("Wq".to_string(), q8_ref("/w", "wq", 64, 32));
    let cu = compile_to_cuda_forward_q8_dp4a(&src, &HashMap::new(), &q8)
        .unwrap_or_else(|e| panic!("must lower: {e}"));
    assert!(
        cu.contains("k0_add_row(const float* a, const float* b, const float* w, float* c, float* y, signed char* yq, float* ys)"),
        "the add, the norm and the quantization are one kernel:\n{cu}"
    );
    assert!(
        !cu.contains("_qact<<<"),
        "no standalone activation quantization:\n{cu}"
    );
}

/// An int8 weight stays int8 in device memory and is dequantized inside the GEMV — the program never holds a
/// float copy of it.
#[test]
fn an_int8_weight_projection_dequantizes_inside_its_gemv() {
    let src = format!(
        "## Main\n{}{}Let Y be X matmul Wq.\nShow item 1 of Y.\n",
        tensor("X", 1, 64, 11),
        declared("Wq", 64, 32)
    );
    let mut q8 = HashMap::new();
    q8.insert("Wq".to_string(), q8_ref("/w", "wq", 64, 32));
    let cu = compile_to_cuda_forward_q8(&src, &HashMap::new(), &q8)
        .unwrap_or_else(|e| panic!("must lower: {e}"));
    assert!(
        cu.contains("signed char* buf_Wq_q;") && cu.contains("k0_q8gemv<<<"),
        "the weight is int8-resident and the GEMV reads it:\n{cu}"
    );
    assert!(
        !cu.contains("buf_Wq;"),
        "no float copy of the weight is allocated:\n{cu}"
    );
}

/// A sparse MoE on the int8-activation lane: the router GEMV, ONE kernel that does softmax + top-k + renormalize,
/// one kernel for the selected experts' gate·up·SiLU, one for their down projection and the weighted sum — and no
/// per-expert loop.
#[test]
fn a_sparse_moe_routes_with_one_topk_kernel_and_two_fused_expert_kernels() {
    let (d, dff, ne, k) = (64usize, 64usize, 4usize, 2usize);
    let src = format!(
        "## Main\n{}{}{}{}{}Let H be moe of X with router Wr gate Wg up Wu down Wd choosing {k} of {ne}.\nShow item 1 of H.\n",
        tensor("X", 1, d, 12),
        declared("Wr", d, ne),
        declared("Wg", ne * dff, d),
        declared("Wu", ne * dff, d),
        declared("Wd", ne * d, dff)
    );
    let mut moe = HashMap::new();
    moe.insert(
        "H".to_string(),
        MoeWeights {
            router: q8_ref("/w", "wr", d, ne),
            gate: q8_ref("/w", "wg", d, ne * dff),
            up: q8_ref("/w", "wu", d, ne * dff),
            down: q8_ref("/w", "wd", dff, ne * d),
            d,
            dff,
            ne,
            k,
        },
    );
    let cu = compile_to_cuda_forward_q8_moe_dp4a(&src, &HashMap::new(), &HashMap::new(), &moe)
        .unwrap_or_else(|e| panic!("must lower: {e}"));
    let step = captured_step(&cu);
    for launch in [
        "moe_router_fast<<<",
        "moe_topk_idx<<<",
        "moe_gate_up_silu<<<",
        "moe_down_combine<<<",
    ] {
        assert_eq!(
            step.matches(launch).count(),
            1,
            "`{launch}` is launched exactly once in the captured step:\n{step}"
        );
    }
    assert!(
        !step.contains("for(int _e=0;"),
        "no loop over every expert:\n{step}"
    );
}

/// The launches the CUDA graph captures — the step every replay runs. (The program also runs the same launches once
/// as a warm-up before the capture, and once more under `LOGOS_CUDA_PINS` to time them.)
fn captured_step(cu: &str) -> &str {
    let begin = cu.find("cudaStreamBeginCapture").expect("the step is captured");
    let end = cu[begin..].find("cudaStreamEndCapture").expect("the capture ends") + begin;
    &cu[begin..end]
}

/// The attention context feeding an int8 output projection: the attention kernel writes the quantized context.
fn attention_into_projection() -> (String, usize, usize, usize, usize) {
    let (h, kv, hd, l) = (2usize, 1usize, 32usize, 4usize);
    let src = format!(
        "## Main\n{}{}{}Let C be attention of Q over K and V with {h} heads {kv} groups.\n{}Let Y be C matmul Wo.\nShow item 1 of Y.\n",
        tensor("Q", 1, h * hd, 13),
        tensor("K", l, kv * hd, 14),
        tensor("V", l, kv * hd, 15),
        declared("Wo", h * hd, 32)
    );
    (src, h, kv, hd, l)
}

#[test]
fn an_attention_feeding_an_int8_projection_quantizes_in_its_epilogue() {
    let (src, ..) = attention_into_projection();
    let mut q8 = HashMap::new();
    q8.insert("Wo".to_string(), q8_ref("/w", "wo", 64, 32));
    let cu = compile_to_cuda_forward_q8_dp4a(&src, &HashMap::new(), &q8)
        .unwrap_or_else(|e| panic!("must lower: {e}"));
    assert!(
        cu.contains("k0_attn(") && cu.contains("float scale, signed char* yq, float* ys)"),
        "the attention kernel writes the int8 context:\n{cu}"
    );
    assert!(
        !cu.contains("_qact<<<"),
        "the output projection does not quantize the context again:\n{cu}"
    );
}

/// The values half: the fused attention → int8 projection, on the GPU, against a reference that computes the
/// attention in f64, quantizes the context per 32-block exactly as the int8-activation lane does, and contracts it
/// with the same int8 weights. Weight scales are powers of two so their f16 storage is exact.
#[test]
fn an_attention_feeding_an_int8_projection_matches_its_reference_on_the_gpu() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let (src, h, kv, hd, l) = attention_into_projection();
    let (kin, nout) = (h * hd, 32usize);
    let dir = std::env::temp_dir().join(format!("logos_attnq8_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wq: Vec<i8> = (0..kin * nout)
        .map(|i| (((i * 29 + 7) % 255) as i32 - 127) as i8)
        .collect();
    let ws: Vec<f32> = (0..nout * kin / 32)
        .map(|i| 2f32.powi(-7 - (i % 3) as i32))
        .collect();
    let (qf, sf) = (dir.join("wo.q"), dir.join("wo.s"));
    std::fs::write(&qf, wq.iter().map(|&v| v as u8).collect::<Vec<u8>>()).unwrap();
    std::fs::write(&sf, ws.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let mut q8 = HashMap::new();
    q8.insert(
        "Wo".to_string(),
        Q8Ref {
            quants: qf.to_string_lossy().into(),
            scales: sf.to_string_lossy().into(),
            in_dim: kin,
            out_dim: nout,
        },
    );
    let cu = compile_to_cuda_forward_q8_dp4a(&src, &HashMap::new(), &q8)
        .unwrap_or_else(|e| panic!("must lower: {e}"));
    let got = nvcc_run(&cu, "attnq8");
    std::fs::remove_dir_all(&dir).unwrap();

    let q: Vec<f64> = (0..h * hd).map(|p| value(p, 13)).collect();
    let k: Vec<f64> = (0..l * kv * hd).map(|p| value(p, 14)).collect();
    let v: Vec<f64> = (0..l * kv * hd).map(|p| value(p, 15)).collect();
    let mut ctx = vec![0f64; h * hd];
    for head in 0..h {
        let g = head / (h / kv);
        let scores: Vec<f64> = (0..l)
            .map(|j| {
                (0..hd).map(|e| q[head * hd + e] * k[j * kv * hd + g * hd + e]).sum::<f64>()
                    / (hd as f64).sqrt()
            })
            .collect();
        let m = scores.iter().cloned().fold(f64::MIN, f64::max);
        let ex: Vec<f64> = scores.iter().map(|s| (s - m).exp()).collect();
        let den: f64 = ex.iter().sum();
        for e in 0..hd {
            ctx[head * hd + e] =
                (0..l).map(|j| ex[j] * v[j * kv * hd + g * hd + e]).sum::<f64>() / den;
        }
    }
    let nb = kin / 32;
    let (mut cq, mut cs) = (vec![0i32; kin], vec![0f64; nb]);
    for b in 0..nb {
        let amax = ctx[b * 32..b * 32 + 32].iter().fold(0f64, |a, x| a.max(x.abs())).max(1e-30);
        cs[b] = amax / 127.0;
        for j in 0..32 {
            cq[b * 32 + j] = ((ctx[b * 32 + j] / cs[b]).round() as i32).clamp(-128, 127);
        }
    }
    let want: Vec<f64> = (0..nout)
        .map(|n| {
            (0..nb)
                .map(|b| {
                    let dot: i32 = (0..32).map(|j| cq[b * 32 + j] * wq[n * kin + b * 32 + j] as i32).sum();
                    dot as f64 * ws[n * nb + b] as f64 * cs[b]
                })
                .sum()
        })
        .collect();
    assert_eq!(got.len(), want.len(), "one output per output channel");
    let scale = want.iter().fold(0f64, |a, x| a.max(x.abs()));
    for (o, (g, w)) in got.iter().zip(&want).enumerate() {
        assert!(
            (g - w).abs() <= 1e-2 * scale,
            "channel {o}: {g} on the GPU, {w} in the reference (scale {scale})"
        );
    }
}

#[test]
fn the_compiled_forward_is_captured_as_one_cuda_graph() {
    let src = format!(
        "## Main\n{}{}Let Y be rmsnorm of X with weight W.\nShow item 1 of Y.\n",
        tensor("X", 1, 64, 16),
        tensor("W", 1, 64, 17)
    );
    let cu = compile_to_cuda_forward(&src).unwrap_or_else(|e| panic!("must lower: {e}"));
    for call in ["cudaStreamBeginCapture(", "cudaGraphInstantiate(", "cudaGraphLaunch("] {
        assert!(cu.contains(call), "`{call}` is part of the program:\n{cu}");
    }
}

fn nvcc_run(cu: &str, tag: &str) -> Vec<f64> {
    let dir = std::env::temp_dir().join(format!("logos_census_{tag}_{}", std::process::id()));
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
        "the emitted program must compile:\n{}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = Command::new(&bin).output().expect("run");
    assert!(
        r.status.success(),
        "the emitted program failed:\n{}",
        String::from_utf8_lossy(&r.stderr)
    );
    let out = String::from_utf8_lossy(&r.stdout)
        .split_whitespace()
        .map(|s| s.parse().unwrap_or_else(|_| panic!("non-numeric output `{s}`")))
        .collect();
    std::fs::remove_dir_all(&dir).unwrap();
    out
}

#[path = "common/forward_corpus.rs"]
mod forward_corpus;

/// `cu` printing every shown real to seventeen significant digits, which give back its value exactly.
fn exact(cu: &str) -> String {
    cu.replace("printf(\"%.6f\\n\", (double)buf_", "printf(\"%.17g\\n\", (double)buf_")
}

/// `src` showing every element of each tensor it shows an element of.
fn showing_all(src: &str) -> String {
    use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
    use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
    let rp = recognize_stages(src).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
    let g = TensorGraph::lift(&rp, None, None, &|_| Storage::Values).unwrap_or_else(|e| panic!("{e}\n{src}"));
    let mut out = String::new();
    for line in src.lines() {
        match line.strip_prefix("Show item 1 of ").and_then(|t| t.strip_suffix('.')) {
            Some(t) => {
                let n = g.tensors.get(t).unwrap_or_else(|| panic!("`{t}` is a tensor of the program")).len();
                (1..=n).for_each(|p| out.push_str(&format!("Show item {p} of {t}.\n")));
            }
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

/// `r` with its int8 weights written into `dir`: codes of both signs, block scales of a few sizes.
fn written(r: &Q8Ref, dir: &std::path::Path, tag: &str, salt: usize) -> Q8Ref {
    let n = r.in_dim * r.out_dim;
    let codes: Vec<u8> = (0..n).map(|i| ((((i * 37 + salt * 11) % 255) as i64 - 127) as i8) as u8).collect();
    let scales: Vec<u8> = (0..n / 32).flat_map(|b| (0.004f32 + ((b + salt) % 7) as f32 * 0.0015).to_le_bytes()).collect();
    let (q, s) = (dir.join(format!("{tag}.q")), dir.join(format!("{tag}.s")));
    std::fs::write(&q, codes).unwrap();
    std::fs::write(&s, scales).unwrap();
    Q8Ref { quants: q.display().to_string(), scales: s.display().to_string(), in_dim: r.in_dim, out_dim: r.out_dim }
}

/// Every fusion the compiled forward makes computes what its kernels compute apart, bit for bit (OPT-6, the forward's
/// `Contract::fused`): each program of the forward's census — the quantized programs at every numerics, their int8
/// weights written for their manifests, and the float programs that chain, add into a norm, keep a value a later
/// stage revives and join a pointwise diamond into one region, an arm of it written out for a later reader — compiled as the forward fuses it and with every step the optimizer can hold held back
/// (`fuse::steps`, the numerics fold among them), shows every element of its result alike on the device.
#[test]
fn every_forward_fusion_computes_what_its_kernels_compute_apart() {
    use logicaffeine_compile::codegen_cuda::forward_shim::ForwardShim;
    use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
    use logicaffeine_compile::compile::compile_to_cuda_forward_holding;
    use logicaffeine_compile::tensor_graph::fuse::steps;
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let dir = std::env::temp_dir().join(format!("logos_forward_apart_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (d, rows) = (64usize, 2usize);
    let mut programs: Vec<(String, String, HashMap<String, Q8Ref>, HashMap<String, MoeWeights>)> = forward_corpus::quantized_programs();
    let float = |body: &str, shown: &str| {
        format!("## Main\n{}{}{}{}{}{body}Show item 1 of {shown}.\n", tensor("A", rows, d, 31), tensor("B", rows, d, 32), tensor("C", rows, d, 33), tensor("D", rows, d, 34), tensor("W", 1, d, 35))
    };
    for (name, body, shown) in [
        ("chain, add into a norm, a value revived", "Let E be A plus B.\nLet F be E hadamard C.\nLet G be F plus D.\nLet P be rmsnorm of G with weight W.\nLet Q be P plus E.\n", "Q"),
        ("add into a norm", "Let S be A plus B.\nLet Y be rmsnorm of S with weight W.\n", "Y"),
        ("chain into a norm", "Let S be A plus B.\nLet T be S hadamard C.\nLet U be T plus D.\nLet Y be rmsnorm of U with weight W.\n", "Y"),
        ("chain through a SiLU", "Let S be A hadamard B.\nLet T be silu of S.\nLet U be T hadamard C.\n", "U"),
        ("a region: a product and a sum joined", "Let S be A plus B.\nLet P be A hadamard B.\nLet Y be S plus P.\n", "Y"),
        (
            "a region writing out an arm a later op reads",
            "Let S be A plus B.\nLet T be silu of S.\nLet P be A hadamard B.\nLet Y be T hadamard P.\nLet Z be P plus C.\n",
            "Z",
        ),
    ] {
        programs.push((name.to_string(), float(body, shown), HashMap::new(), HashMap::new()));
    }
    let (mut compared, mut taken_apart) = (0, 0);
    for (p, (name, src, q8, moe)) in programs.iter().enumerate() {
        let src = showing_all(src);
        let q8: HashMap<String, Q8Ref> = q8.iter().enumerate().map(|(k, (v, r))| (v.clone(), written(r, &dir, &format!("p{p}q{k}"), p + k))).collect();
        let moe: HashMap<String, MoeWeights> = moe
            .iter()
            .map(|(v, m)| {
                let w = |r: &Q8Ref, part: &str, salt: usize| written(r, &dir, &format!("p{p}{v}{part}"), salt);
                (v.clone(), MoeWeights { router: w(&m.router, "r", 1), gate: w(&m.gate, "g", 2), up: w(&m.up, "u", 3), down: w(&m.down, "d", 4), ..m.clone() })
            })
            .collect();
        for kind in [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
            let what = format!("{name} at {kind:?}");
            let Ok(fused) = compile_to_cuda_forward_holding(&src, &HashMap::new(), &q8, &moe, kind, &[]) else { continue };
            let held = steps(&ForwardShim { kind });
            let apart = compile_to_cuda_forward_holding(&src, &HashMap::new(), &q8, &moe, kind, &held).unwrap_or_else(|e| panic!("{what}: apart: {e}"));
            assert!(kernels(&apart) >= kernels(&fused), "{what}: the program apart launches no fewer kernels");
            if apart == fused {
                continue;
            }
            taken_apart += 1;
            let (f, a) = (nvcc_run(&exact(&fused), &format!("fused{p}{kind:?}")), nvcc_run(&exact(&apart), &format!("apart{p}{kind:?}")));
            assert!(!f.is_empty() && f.len() == a.len(), "{what}: {} shown fused, {} apart", f.len(), a.len());
            for (k, (x, y)) in f.iter().zip(&a).enumerate() {
                assert_eq!(x.to_bits(), y.to_bits(), "{what}: element {k} is {x:e} fused and {y:e} apart");
            }
            compared += f.len();
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(taken_apart >= 10, "only {taken_apart} programs of the census fuse anything, so the claim is barely checked");
    assert!(compared > 0);
}
