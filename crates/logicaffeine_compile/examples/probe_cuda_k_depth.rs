//! ★ THE MEASUREMENT LANE `tile::best_candidate_for` ASKED FOR.
//!
//! On a register-accumulator target the planner ranks by arithmetic intensity, and
//! `intensity_key = osh·osw·ibw / ((osh+osw)·ibw)` — the contraction depth CANCELS. So K depth is
//! settled by `stage_depth_key`, which maximizes `ibw` on the stated grounds that "every stage pays a
//! pipeline round-trip and a deeper one amortises it over more multiplies".
//!
//! That reasoning is sound about the BENEFIT and silent about the PRICE: a deeper stage needs
//! proportionally more shared memory, and on a GPU shared memory per block IS occupancy. `best_candidate_for`
//! says the intensity ranking "stays until a measurement lane exists to justify replacing it".
//!
//! This is that lane: the same 128×128 output tile at every affordable K depth, each lowered, compiled by
//! nvcc and benchmarked on the device.
//!
//!   cargo run -q --release -p logicaffeine-compile --features codegen --example probe_cuda_k_depth

use logicaffeine_compile::spatial::ir::NumericsPolicy;
use logicaffeine_compile::spatial::lower::cuda::CudaPersistentBackend;
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::passes::from_tensor_op::MatmulNames;
use logicaffeine_compile::spatial::passes::tile;
use logicaffeine_compile::spatial::pipeline::{run_passes, template_from_spec};
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_recognize::{Fill, MatmulSpec};

fn names() -> MatmulNames {
    MatmulNames {
        a: "a".into(),
        b: "b".into(),
        c: "c".into(),
    }
}

fn nvcc(cu: &str, label: &str) -> Option<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("logos_kdepth_{}_{label}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let (src, bin) = (dir.join("k.cu"), dir.join("k"));
    std::fs::write(&src, cu).ok()?;
    let out = std::process::Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-std=c++17", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .ok()?;
    if !out.status.success() {
        eprintln!(
            "  nvcc failed for {label}:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    Some(bin)
}

fn gops(bin: &std::path::Path) -> f64 {
    let mut best = 0f64;
    for _ in 0..5 {
        let Ok(out) = std::process::Command::new(bin).output() else {
            continue;
        };
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(rest) = line.strip_prefix("GOPS ") {
                best = best.max(rest.trim().parse::<f64>().unwrap_or(0.0));
            }
        }
    }
    best
}

fn main() {
    let (m, n, k) = (2048usize, 2048usize, 2048usize);
    let gpu = SpatialTarget::rtx3060();
    let spec = MatmulSpec {
        m,
        n,
        k,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let t = template_from_spec(
        &spec,
        &names(),
        (&Fill::Unknown, &Fill::Unknown),
        (None, None),
        NumericsPolicy::Exact,
        &gpu,
    )
    .unwrap();
    let base = run_passes(t.clone(), &gpu).unwrap();
    let chosen = base.work.plan.expect("a plan");
    println!(
        "the planner chooses ibw={} (osh {} osw {})",
        chosen.block.ibw, chosen.block.osh, chosen.block.osw
    );

    // Every candidate sharing the chosen output tile, at each contraction depth.
    let mut cands: Vec<_> = tile::candidates(&t, &gpu)
        .into_iter()
        .filter(|c| {
            c.plan.per_site_m == chosen.per_site_m
                && c.plan.per_site_n == chosen.per_site_n
                && c.plan.block.osh == chosen.block.osh
                && c.plan.block.osw == chosen.block.osw
        })
        .collect();
    cands.sort_by_key(|c| c.plan.block.ibw);

    println!(
        "\n{:>5} {:>7} {:>9} {:>9} {:>8} {:>12} {:>9}",
        "ibw", "K elems", "nblk", "smem B", "dynamic", "blocks/SM", "GOPS"
    );
    for c in &cands {
        let p = run_passes(tile::apply_plan(&t, c.plan, &gpu), &gpu);
        let Ok(p) = p else {
            println!("{:>5} refused by the pipeline", c.plan.block.ibw);
            continue;
        };
        let art = match CudaPersistentBackend::new(gpu.clone()).lower(&p) {
            Ok(SpatialArtifact::Cuda(a)) => a,
            other => {
                println!("{:>5} lowering refused: {other:?}", c.plan.block.ibw);
                continue;
            }
        };
        let g = nvcc(&art.bench_cu, &format!("ibw{}", c.plan.block.ibw))
            .map(|b| gops(&b))
            .unwrap_or(0.0);
        println!(
            "{:>5} {:>7} {:>9} {:>9} {:>8} {:>12} {:>9.0}",
            c.plan.block.ibw,
            c.plan.block.ibw * 16,
            c.plan.inner.num_blocks,
            art.budget.smem_bytes,
            art.budget.dynamic_opt_in,
            art.budget.blocks_per_sm,
            g
        );
    }
    println!("\nThe deepest stage is not free: it buys fewer pipeline round-trips with shared memory, and");
    println!("shared memory per block is how many blocks an SM can hold.");
}
