//! cuda_wmma_multistage_gpu_lock — the ratchet's rung 5: a DEEPER cp.async pipeline (3–4 shared buffers vs 2).
//! More in-flight stages keep the tensor cores fed further ahead, hiding more global-load latency. Every
//! measured config carries the all-ones in-harness guard (A=B=1 ⇒ every C==K), so a pipeline race can never
//! print a GOPS number. The best depth must climb ABOVE the 2-stage rung-4 kernel (31,027 GOPS).
//!
//! ⚠️ If a lock goes RED, fix/extend the kernel + sweep (`codegen_cuda::gemm_tile`), never the assertion.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_wmma_multistage_gpu_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{
    cpasync_stages_fit, emit_wmma_cpasync_i8_bench_staged, emit_wmma_cpasync_i8_program_staged,
    GpuLimits, WmmaTile,
};

const RUNG4_GOPS: f64 = 31027.0; // best 2-stage cp.async WMMA, measured in cuda_wmma_cpasync_gpu_lock
const CUBLAS_INT8_GOPS: f64 = 88490.0;

fn oracle(m: usize, k: usize, n: usize) -> Vec<i64> {
    let a: Vec<i64> = (0..m * k)
        .map(|i| ((i as i64 * 7 + 3) % 251) - 125)
        .collect();
    let b: Vec<i64> = (0..k * n)
        .map(|i| ((i as i64 * 5 + 1) % 251) - 125)
        .collect();
    let mut c = vec![0i64; m * n];
    for i in 0..m {
        for j in 0..n {
            let mut acc = 0i64;
            for kk in 0..k {
                acc += a[i * k + kk] * b[kk * n + j];
            }
            c[i * n + j] = acc;
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

fn nvcc_out(cu: &str, label: &str) -> Option<String> {
    let dir = std::env::temp_dir().join(format!("logos_ms_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("k.cu");
    let bin = dir.join("k");
    std::fs::write(&src, cu).ok()?;
    let c = std::process::Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .ok()?;
    if !c.status.success() {
        eprintln!(
            "nvcc failed [{label}]:\n{}",
            String::from_utf8_lossy(&c.stderr)
        );
        return None;
    }
    let r = std::process::Command::new(&bin).output().ok()?;
    let out = String::from_utf8_lossy(&r.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    Some(out)
}

#[test]
fn a_deep_pipeline_still_equals_the_scalar_oracle() {
    if !have() {
        eprintln!("SKIP: needs nvcc + a GPU driver (/dev/nvidia0)");
        return;
    }
    let t = WmmaTile {
        bm: 128,
        bn: 64,
        bk: 32,
        warps_m: 2,
        warps_n: 2,
    };
    for stages in [3usize, 4] {
        for &(m, k, n) in &[
            (128usize, 128usize, 128usize),
            (256, 256, 256),
            (384, 128, 512),
        ] {
            let out = nvcc_out(
                &emit_wmma_cpasync_i8_program_staged(&t, stages, m, k, n),
                &format!("s{stages}_{m}x{k}x{n}"),
            )
            .expect("runs");
            let got: Vec<i64> = out
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim().parse().expect("int"))
                .collect();
            let want = oracle(m, k, n);
            assert_eq!(
                got, want,
                "stages={stages} ({m},{k},{n}): deep-pipeline WMMA must equal the oracle"
            );
        }
    }
}

#[test]
fn the_deeper_pipeline_climbs_above_the_two_stage_kernel() {
    if !have() {
        eprintln!("SKIP: needs nvcc + a GPU driver");
        return;
    }
    let (m, k, n) = (2048usize, 2048usize, 2048usize);
    let gpu = GpuLimits::ampere();
    let tiles = [
        WmmaTile {
            bm: 128,
            bn: 64,
            bk: 32,
            warps_m: 2,
            warps_n: 2,
        },
        WmmaTile {
            bm: 128,
            bn: 128,
            bk: 32,
            warps_m: 2,
            warps_n: 4,
        },
        WmmaTile {
            bm: 128,
            bn: 64,
            bk: 64,
            warps_m: 2,
            warps_n: 2,
        },
        WmmaTile {
            bm: 64,
            bn: 64,
            bk: 32,
            warps_m: 2,
            warps_n: 2,
        },
    ];
    let mut best: Option<(WmmaTile, usize, f64)> = None;
    for t in tiles {
        for stages in [2usize, 3, 4] {
            if !cpasync_stages_fit(&t, stages, &gpu) {
                continue;
            }
            if let Some(out) = nvcc_out(
                &emit_wmma_cpasync_i8_bench_staged(&t, stages, m, k, n),
                &format!("{}x{}bk{}s{}", t.bm, t.bn, t.bk, stages),
            ) {
                if let Some(g) = out.lines().find_map(|l| {
                    l.strip_prefix("GOPS ")
                        .and_then(|v| v.trim().parse::<f64>().ok())
                }) {
                    eprintln!(
                        "  {}x{} bk{} warps {}x{} stages={} -> {:.1} GOPS",
                        t.bm, t.bn, t.bk, t.warps_m, t.warps_n, stages, g
                    );
                    if best.map_or(true, |(_, _, bg)| g > bg) {
                        best = Some((t, stages, g));
                    }
                }
            }
        }
    }
    let (bt, bs, bg) = best.expect("at least one deep-pipeline config must run");
    eprintln!(
        "BEST deep cp.async {}x{} bk{} warps {}x{} stages={} = {:.1} GOPS  |  {:.2}× over rung-4 {RUNG4_GOPS:.0}  |  {:.1}% of cuBLAS {CUBLAS_INT8_GOPS:.0}",
        bt.bm, bt.bn, bt.bk, bt.warps_m, bt.warps_n, bs, bg, bg / RUNG4_GOPS, 100.0 * bg / CUBLAS_INT8_GOPS
    );
    assert!(
        bg > RUNG4_GOPS,
        "best deep-pipeline cp.async ({bg:.1} GOPS) must beat the 2-stage rung-4 kernel ({RUNG4_GOPS} GOPS)"
    );
}
