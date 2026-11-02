//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! INT8-ACTIVATION DP4A MMQ — the llama.cpp-beating kernel. A LOGOS matmul against a real Qwen3-8B int8
//! weight, emitted with the DP4A path: the f32 activation is quantized to int8 per 32-block and `__dp4a`'d
//! against the int8 weight (4 int8 MACs/instr, int32 accumulate). This is the fast path `cuda/qwen_forward.cu`
//! uses to edge past llama.cpp; here we prove the orchestrator EMITS it (from a LOGOS matmul) and it computes
//! the int8-activation result correctly on the GPU.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8Ref;
use logicaffeine_compile::compile::compile_to_cuda_forward_q8_dp4a;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn model_path() -> PathBuf {
    manifest().join("../../models/qwen3-8b/Qwen_Qwen3-8B-Q8_0.gguf")
}
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

#[test]
fn logos_matmul_emits_the_int8_activation_dp4a_kernel_on_a_real_qwen_weight() {
    if !(model_path().exists() && have_nvcc() && have_gpu()) {
        eprintln!("SKIP int8-activation DP4A: needs the 8B checkpoint + nvcc + a live GPU.");
        return;
    }
    let mut g = Gguf::open(&model_path()).expect("open gguf");
    let name = "blk.0.attn_q.weight";
    let (in_f, _) = g.dims2(name).expect("dims");
    let (quants, scales) = g.q8_0_matrix(name).expect("Q8_0");
    let nb_full = in_f / 32;
    let (k, n) = (64usize, 8usize);
    let nb = k / 32;
    let mut q_tile = vec![0i8; n * k];
    let mut s_tile = vec![0f32; n * nb];
    for out in 0..n {
        for t in 0..k {
            q_tile[out * k + t] = quants[out * in_f + t];
        }
        for b in 0..nb {
            s_tile[out * nb + b] = scales[out * nb_full + b];
        }
    }

    let dir = std::env::temp_dir().join(format!("logos_dp4a_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (qf, sf) = (dir.join("wq.i8"), dir.join("wq.s"));
    std::fs::write(&qf, q_tile.iter().map(|&q| q as u8).collect::<Vec<u8>>()).unwrap();
    let mut sb = Vec::new();
    for &s in &s_tile {
        sb.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(&sf, &sb).unwrap();

    let a: Vec<f32> = (0..k).map(|t| ((t % 9) as f32 - 4.0) * 0.05).collect();
    let mut src = String::from("## Main\nLet A be a 1 by 64 tensor of Real.\n");
    for (t, &av) in a.iter().enumerate() {
        src.push_str(&format!("Set item {} of A to {:?}.\n", t + 1, av));
    }
    src.push_str("Let Wq be a 64 by 8 tensor of Real.\nLet Y be A matmul Wq.\n");
    for o in 0..n {
        src.push_str(&format!("Show item {} of Y.\n", o + 1));
    }

    let mut q8 = HashMap::new();
    q8.insert(
        "Wq".to_string(),
        Q8Ref {
            quants: qf.to_string_lossy().into(),
            scales: sf.to_string_lossy().into(),
            in_dim: k,
            out_dim: n,
        },
    );
    let cu = compile_to_cuda_forward_q8_dp4a(&src, &HashMap::new(), &q8)
        .expect("dp4a forward must lower");
    assert!(
        cu.contains("k0_q8dp4a") && cu.contains("__dp4a"),
        "the matmul must emit the int8-activation DP4A kernel:\n{cu}"
    );

    let (cu_path, bin) = (dir.join("d.cu"), dir.join("d"));
    std::fs::write(&cu_path, &cu).unwrap();
    let comp = Command::new("nvcc")
        .args(["-O2", "-arch=sm_86"])
        .arg(&cu_path)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("nvcc");
    assert!(
        comp.status.success(),
        "emitted DP4A kernel must compile:\n{}",
        String::from_utf8_lossy(&comp.stderr)
    );
    let run = Command::new(&bin).output().expect("run");
    assert!(
        run.status.success(),
        "run failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let got: Vec<f32> = String::from_utf8_lossy(&run.stdout)
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();

    // Clean-room INT8-ACTIVATION reference (mirrors the kernel's per-block quantization + int dot).
    let mut refy = vec![0f32; n];
    for out in 0..n {
        let mut acc = 0f32;
        for bl in 0..nb {
            let amax = (0..32)
                .map(|j| a[bl * 32 + j].abs())
                .fold(1e-30f32, f32::max);
            let (asc, inv) = (amax / 127.0, 127.0 / amax);
            let mut dot = 0i32;
            for j in 0..32 {
                let q = (a[bl * 32 + j] * inv).round().clamp(-128.0, 127.0) as i32;
                dot += q * q_tile[out * k + bl * 32 + j] as i32;
            }
            acc += dot as f32 * s_tile[out * nb + bl] * asc;
        }
        refy[out] = acc;
    }
    // Also the exact (dequant) result — the int8-activation answer must be CLOSE to it (a good approximation).
    let mut deq = vec![0f32; n];
    for out in 0..n {
        deq[out] = (0..k)
            .map(|t| a[t] * q_tile[out * k + t] as f32 * s_tile[out * nb + t / 32])
            .sum();
    }

    assert_eq!(got.len(), n);
    for (o, g) in got.iter().enumerate() {
        assert!(
            (g - refy[o]).abs() < 5e-3,
            "DP4A out {o}: GPU {g} vs int8-act reference {}",
            refy[o]
        );
        // int8 activation is a good approximation of the exact dequant result.
        assert!(
            (g - deq[o]).abs() < 5e-2 * (1.0 + deq[o].abs()),
            "DP4A out {o}: GPU {g} vs exact {}",
            deq[o]
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    println!("INT8-activation DP4A MMQ: a LOGOS matmul on a real Qwen3-8B int8 weight emitted __dp4a + ran on the GPU ≡ int8-act reference {refy:?}.");
}
