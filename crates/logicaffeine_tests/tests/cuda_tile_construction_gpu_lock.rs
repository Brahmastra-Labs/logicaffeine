//! cuda_tile_construction_gpu_lock — the empirical last mile: the GPU crowns the winner among the CONSTRUCTED
//! Pareto frontier, and we validate that dominance pruning was SAFE (it never discarded the true winner). This
//! is "let the machine construct the answer": the solver hands over a small weight-free frontier, the silicon
//! measures it, and the fastest tile falls out — we never told it which.
//!
//! We also validate the pruning against ground truth: we measure a sample of DOMINATED tiles too and confirm
//! none of them beats the frontier's best. If a dominated tile won, the monotonicity assumption behind
//! dominance would be false — and we'd report that honestly rather than trust the pruning.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_tile_construction_gpu_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::{
    emit_tiled_dp4a_bench, feasible_tiles, pareto_frontier, CudaTile, GpuLimits,
};

const NAIVE_DP4A_GOPS: f64 = 784.0;

fn have() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && std::path::Path::new("/dev/nvidia0").exists()
}

/// Only tiles that evenly divide the 2048³ problem can drive the simple kernel launch.
fn divides_2048(t: &CudaTile) -> bool {
    2048 % t.bm == 0 && 2048 % t.bn == 0 && 2048 % t.bk == 0
}

fn run_gops(t: &CudaTile) -> Option<f64> {
    let (m, k, n) = (2048usize, 2048, 2048);
    let dir = std::env::temp_dir().join(format!(
        "logos_front_{}_{}x{}_{}x{}",
        std::process::id(),
        t.bm,
        t.bn,
        t.tm,
        t.tn
    ));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("k.cu");
    let bin = dir.join("k");
    std::fs::write(&src, emit_tiled_dp4a_bench(t, m, k, n)).ok()?;
    let c = std::process::Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .ok()?;
    if !c.status.success() {
        return None;
    }
    let r = std::process::Command::new(&bin).output().ok()?;
    let out = String::from_utf8_lossy(&r.stdout);
    let g = out.lines().find_map(|l| {
        l.strip_prefix("GOPS ")
            .and_then(|v| v.trim().parse::<f64>().ok())
    });
    let _ = std::fs::remove_dir_all(&dir);
    g
}

#[test]
fn the_gpu_crowns_the_winner_among_the_constructed_frontier_and_pruning_was_safe() {
    if !have() {
        eprintln!("SKIP: needs nvcc + a GPU driver (/dev/nvidia0)");
        return;
    }
    let gpu = GpuLimits::ampere();
    // Restrict to tiles that tile 2048³ so the simple kernel is valid for every candidate we measure.
    let front: Vec<CudaTile> = pareto_frontier(&gpu)
        .into_iter()
        .filter(divides_2048)
        .collect();
    let dominated: Vec<CudaTile> = feasible_tiles(&gpu)
        .into_iter()
        .filter(|t| divides_2048(t) && !front.contains(t))
        .collect();
    assert!(
        !front.is_empty(),
        "frontier (restricted to 2048-divisors) must be non-empty"
    );

    // Measure the whole frontier — the machine crowns the winner.
    let mut best: Option<(CudaTile, f64)> = None;
    for t in &front {
        if let Some(g) = run_gops(t) {
            eprintln!(
                "  frontier {}x{} bk{} {}x{} (occ {:.2}) -> {:.1} GOPS",
                t.bm,
                t.bn,
                t.bk,
                t.tm,
                t.tn,
                t.occupancy(),
                g
            );
            if best.map_or(true, |(_, bg)| g > bg) {
                best = Some((*t, g));
            }
        }
    }
    let (wt, wg) = best.expect("at least one frontier tile must run");
    eprintln!(
        "WINNER (measured, from the frontier): {}x{} bk{} {}x{} = {:.1} GOPS",
        wt.bm, wt.bn, wt.bk, wt.tm, wt.tn, wg
    );

    // Validate dominance pruning was SAFE: sample dominated tiles; none may beat the frontier winner. If one
    // did, dominance discarded the optimum and the monotonicity assumption is false — we'd surface that.
    let mut worst_violation = 0.0f64;
    for t in dominated.iter().step_by(dominated.len().max(1) / 12 + 1) {
        if let Some(g) = run_gops(t) {
            if g > wg {
                worst_violation = worst_violation.max(g - wg);
                eprintln!(
                    "  !! dominated {}x{} {}x{} = {:.1} GOPS BEAT the frontier winner {:.1}",
                    t.bm, t.bn, t.tm, t.tn, g, wg
                );
            }
        }
    }
    assert!(
        worst_violation == 0.0,
        "a dominated tile beat the frontier winner by {worst_violation:.1} GOPS — dominance pruning is UNSAFE for this workload"
    );

    // And the winner is a genuinely fast kernel (far above naive), so the construction produced real value.
    assert!(
        wg >= 3.0 * NAIVE_DP4A_GOPS,
        "frontier winner ({wg:.1} GOPS) must climb well above naive ({NAIVE_DP4A_GOPS})"
    );
}
