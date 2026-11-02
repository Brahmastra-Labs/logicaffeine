//! cuda_mma_k64_gpu_lock — the BK=64 champion (CUTLASS's Ampere int8 config): 64×64 warp tile, two m16n8k32
//! steps per stage, XOR-swizzled ldmatrix, cp.async. Broke past the false 79.5% "ceiling" to ~83% of cuBLAS-TN.
//! Correct vs the col-major oracle; measured vs cuBLAS-TN (72,511).
#![cfg(feature = "codegen")]
use logicaffeine_compile::codegen_cuda::gemm_tile::{
    emit_mma_tn_k64_bench, emit_mma_tn_k64_program,
};
const CUBLAS_TN: f64 = 72511.0;
const PREV_CEILING: f64 = 57638.0; // the BK=32 "ceiling" we broke

fn oracle(m: usize, k: usize, n: usize) -> Vec<i64> {
    let a: Vec<i64> = (0..m * k)
        .map(|i| ((i as i64 * 7 + 3) % 251) - 125)
        .collect();
    let mut c = vec![0i64; m * n];
    for i in 0..m {
        for j in 0..n {
            let mut s = 0i64;
            for kk in 0..k {
                let bkn = kk as i64 * n as i64 + j as i64;
                let bv = ((bkn * 5 + 1) % 251) - 125;
                s += a[i * k + kk] * bv;
            }
            c[i * n + j] = s;
        }
    }
    c
}
fn have() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && std::path::Path::new("/dev/nvidia0").exists()
}
fn runp(cu: &str, l: &str) -> Option<String> {
    let d = std::env::temp_dir().join(format!("logos_k64_{}_{}", std::process::id(), l));
    let _ = std::fs::create_dir_all(&d);
    let s = d.join("k.cu");
    let b = d.join("k");
    std::fs::write(&s, cu).ok()?;
    let c = std::process::Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-o"])
        .arg(&b)
        .arg(&s)
        .output()
        .ok()?;
    if !c.status.success() {
        eprintln!(
            "nvcc[{l}]:{}",
            String::from_utf8_lossy(&c.stderr)
                .lines()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ")
        );
        return None;
    }
    let r = std::process::Command::new(&b).output().ok()?;
    let o = String::from_utf8_lossy(&r.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&d);
    Some(o)
}
fn gops(cu: &str, l: &str) -> Option<f64> {
    runp(cu, l)?.lines().find_map(|x| {
        x.strip_prefix("GOPS ")
            .and_then(|v| v.trim().parse::<f64>().ok())
    })
}

#[test]
fn bk64_champion_is_correct_and_breaks_the_bk32_ceiling() {
    if !have() {
        eprintln!("SKIP: needs nvcc + a GPU driver (/dev/nvidia0)");
        return;
    }
    for &(m, k, n) in &[
        (128usize, 64usize, 128usize),
        (256, 256, 256),
        (384, 128, 512),
    ] {
        let out = runp(
            &emit_mma_tn_k64_program(128, 128, 2, 2, 3, m, k, n),
            &format!("c{m}x{k}x{n}"),
        )
        .expect("runs");
        let got: Vec<i64> = out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.trim().parse().expect("int"))
            .collect();
        assert_eq!(
            got,
            oracle(m, k, n),
            "({m},{k},{n}) BK=64 champion must equal oracle"
        );
    }
    let (m, k, n) = (2048usize, 2048, 2048);
    let cfgs = [
        (128usize, 128usize, 2usize, 2usize, 3usize),
        (128, 128, 2, 2, 2),
        (128, 64, 2, 2, 2),
        (128, 128, 2, 4, 3),
    ];
    let mut best = (0usize, 0usize, 0usize, 0.0f64);
    for (bm, bn, wm, wn, st) in cfgs {
        if let Some(g) = gops(
            &emit_mma_tn_k64_bench(bm, bn, wm, wn, st, m, k, n),
            &format!("{bm}x{bn}w{wm}{wn}s{st}"),
        ) {
            eprintln!(
                "  k64 {bm}x{bn} w{wm}x{wn} s{st} -> {g:.1} ({:.1}% cuBLAS-TN)",
                100.0 * g / CUBLAS_TN
            );
            if g > best.3 {
                best = (bm, bn, st, g);
            }
        }
    }
    eprintln!(
        "BEST BK=64 = {:.1} GOPS = {:.1}% of cuBLAS-TN {CUBLAS_TN:.0}  (broke the BK=32 ceiling of {:.1}%)",
        best.3, 100.0 * best.3 / CUBLAS_TN, 100.0 * PREV_CEILING / CUBLAS_TN
    );
    assert!(
        best.3 > PREV_CEILING,
        "BK=64 champion ({:.1}) must beat the BK=32 ceiling ({PREV_CEILING})",
        best.3
    );
}
