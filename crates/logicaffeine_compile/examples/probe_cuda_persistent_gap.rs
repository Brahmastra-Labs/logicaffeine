//! Where the 2.78x goes: the spatial persistent kernel's configuration at 2048^3 bf16 on an RTX 3060,
//! beside the CUDA knob lattice's chosen rung for the same shape.
//!
//! `cuda_spatial_persistent_lock::persistent_throughput_ratchets_against_the_lattice_best` measures
//! 5,639 GOPS against the lattice's 15,667 and demands 0.9x. Both kernels compute the same GEMM on the
//! same GPU, so the difference is entirely in how each is shaped.
//!
//!   cargo run -q --release -p logicaffeine-compile --features codegen --example probe_cuda_persistent_gap

use logicaffeine_compile::codegen_cuda::chip::CudaChip;
use logicaffeine_compile::codegen_cuda::gemm_tile::{lattice_best_bf16_bench, GpuLimits};
use logicaffeine_compile::spatial::ir::NumericsPolicy;
use logicaffeine_compile::spatial::lower::cuda::CudaPersistentBackend;
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::passes::from_tensor_op::MatmulNames;
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

fn main() {
    let (m, n, k) = (2048usize, 2048usize, 2048usize);
    let gpu = SpatialTarget::rtx3060();
    let chip = CudaChip::rtx3060();

    // ★ `CudaChip` carries NO SM COUNT — `sm_arch` is the architecture number (86), not a count of SMs.
    // The SM count is read from the device here rather than guessed, which is the only honest source we
    // have for it today.
    let sms = device_sms();
    println!("=== THE MACHINE ===");
    println!(
        "  smem/block {} KiB  sm_arch {}  sites modelled {}  SMs (device query) {}",
        chip.smem_kib_per_block,
        chip.sm_arch,
        gpu.sites.sites(),
        sms.map(|n| n.to_string())
            .unwrap_or_else(|| "unknown".into())
    );

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
    let p = run_passes(t, &gpu).unwrap();

    println!("\n=== OURS: the spatial persistent kernel ===");
    if let Some(pl) = p.work.plan {
        println!("  plan {pl:?}");
        println!(
            "  per-site output tile: {}x{} tiles = {}x{} elems",
            pl.per_site_m,
            pl.per_site_n,
            pl.per_site_m * 16,
            pl.per_site_n * 16
        );
        println!(
            "  K per block: {} tiles = {} elems, num_blocks {}",
            pl.inner.block_k_tiles,
            pl.inner.block_k_tiles * 16,
            pl.inner.num_blocks
        );
    }
    let sites = p.sites().len();
    let items = p.work.items.len();
    println!(
        "  sites used {sites}  items {items}  items/site {:.1}",
        items as f64 / sites.max(1) as f64
    );
    match sms {
        Some(n) => println!("  blocks per SM: {:.1}", sites as f64 / n as f64),
        None => println!("  blocks per SM: unknown (no device query)"),
    }

    let art = match CudaPersistentBackend::new(gpu.clone()).lower(&p).unwrap() {
        SpatialArtifact::Cuda(a) => a,
        other => panic!("{other:?}"),
    };
    println!("  budget {:?}", art.budget);
    let cu = &art.program_cu;
    for needle in [
        "wmma::",
        "cp.async",
        "cuda::pipeline",
        "__syncthreads",
        "mma_sync",
        "load_matrix_sync",
    ] {
        println!("    {:>18} x{}", needle, cu.matches(needle).count());
    }
    println!(
        "  threads/block: {}",
        grep_int(cu, "#define THREADS").unwrap_or(-1)
    );

    println!("\n=== THEIRS: the knob lattice's best rung ===");
    let base = lattice_best_bf16_bench(m, k, n, 86, &GpuLimits::ampere());
    for key in [
        "#define BM",
        "#define BN",
        "#define BK",
        "#define WM",
        "#define WN",
        "#define WK",
        "#define STAGES",
        "#define THREADS",
        "#define WARPS",
    ] {
        if let Some(v) = grep_int(&base, key) {
            println!("  {key} {v}");
        }
    }
    for needle in [
        "wmma::",
        "cp.async",
        "__pipeline_commit",
        "__syncthreads",
        "mma_sync",
        "load_matrix_sync",
    ] {
        println!("    {:>18} x{}", needle, base.matches(needle).count());
    }

    // ── WHY ibw=1? The candidate lattice, ranked, with the objective's key components. ──────────
    {
        use logicaffeine_compile::spatial::passes::tile;
        use logicaffeine_compile::spatial::work::Objective;
        let tmpl = template_from_spec(
            &spec,
            &names(),
            (&Fill::Unknown, &Fill::Unknown),
            (None, None),
            NumericsPolicy::Exact,
            &gpu,
        )
        .unwrap();
        let mut cands = tile::candidates(&tmpl, &gpu);
        let obj = Objective::default();
        cands.sort_by_key(|c| obj.key(&c.work));
        println!(
            "\n=== THE CANDIDATE LATTICE ({} legal plans), ranked by the objective ===",
            cands.len()
        );
        println!(
            "  {:>4} {:>4} {:>5} {:>6} {:>13} {:>9} {:>12} {:>9} {:>8}",
            "psm", "psn", "ibw", "nblk", "dram_bytes", "site_mm", "engine_noc", "calls", "smem"
        );
        for c in cands.iter().take(6) {
            println!(
                "  {:>4} {:>4} {:>5} {:>6} {:>13} {:>9} {:>12} {:>9} {:>8}",
                c.plan.per_site_m,
                c.plan.per_site_n,
                c.plan.inner.block_k_tiles,
                c.plan.inner.num_blocks,
                c.work.dram_bytes,
                c.work.site_matmul_retired,
                c.work.engine_noc_bytes,
                c.work.compute_calls,
                c.footprint_bytes
            );
        }
        // The 128x128 family at every K depth: is a deeper K step legal, and where does it rank?
        println!("\n  the 8x8 output-tile family at every K depth:");
        for (rank, c) in cands.iter().enumerate() {
            if c.plan.per_site_m == 8 && c.plan.per_site_n == 8 {
                println!(
                    "    rank {:>4}: ibw {:>4} nblk {:>4} dram {:>12} calls {:>8} smem {:>7}",
                    rank,
                    c.plan.inner.block_k_tiles,
                    c.plan.inner.num_blocks,
                    c.work.dram_bytes,
                    c.work.compute_calls,
                    c.footprint_bytes
                );
            }
        }
        println!(
            "\n  budget: max_local_mem_bytes {}  acc_tiles {}",
            gpu.max_local_mem_bytes(),
            gpu.site.acc_tiles
        );
    }

    // ── PRICED vs LAID OUT, across targets: does the planner charge for what the allocator creates? ──
    {
        use logicaffeine_compile::spatial::passes::size_buffers::{
            footprint_bytes, footprint_bytes_for_plan,
        };
        println!("\n=== THE PRICE THE PLANNER PAID vs THE BYTES THE ALLOCATOR LAID OUT ===");
        println!(
            "  {:>12} {:>16} {:>10} {:>10} {:>10}  {}",
            "target", "shape", "priced", "laid out", "delta", ""
        );
        for t in [
            SpatialTarget::rtx3060(),
            SpatialTarget::wormhole(),
            SpatialTarget::blackhole(),
        ] {
            for (m, n, k) in [
                (2048usize, 2048usize, 2048usize),
                (640, 640, 640),
                (512, 512, 512),
                (1024, 1024, 4096),
            ] {
                let spec = MatmulSpec {
                    m,
                    n,
                    k,
                    a: Default::default(),
                    b: Default::default(),
                    c: Default::default(),
                };
                let Ok(tm) = template_from_spec(
                    &spec,
                    &names(),
                    (&Fill::Unknown, &Fill::Unknown),
                    (None, None),
                    NumericsPolicy::Exact,
                    &t,
                ) else {
                    continue;
                };
                let Ok(pp) = run_passes(tm, &t) else { continue };
                let Some(pl) = pp.work.plan else { continue };
                let priced = footprint_bytes_for_plan(&pp, &pl, &t);
                let laid = footprint_bytes(&pp, &t);
                println!(
                    "  {:>12} {:>16} {:>10} {:>10} {:>10}  {}",
                    t.name,
                    format!("{m}x{n}x{k}"),
                    priced,
                    laid,
                    priced as i64 - laid as i64,
                    if priced == laid { "" } else { "★ MISPRICED" }
                );
            }
        }
    }

    // Arithmetic intensity per block tile: the number that decides whether a GEMM is bandwidth-bound.
    if let Some(pl) = p.work.plan {
        let (bm, bn) = (pl.per_site_m * 16, pl.per_site_n * 16);
        let ai = (bm * bn) as f64 / (bm + bn) as f64;
        println!("\n=== ARITHMETIC INTENSITY (flops per operand element loaded, per K step) ===");
        println!("  ours   {bm}x{bn} tile -> {:.1}", ai);
        for (tm, tn) in [(128usize, 128usize), (128, 64), (64, 64)] {
            println!(
                "  a {tm}x{tn} tile would be -> {:.1}",
                (tm * tn) as f64 / (tm + tn) as f64
            );
        }
    }
}

/// The SM count, from the device. Nothing in the chip descriptor knows it.
fn device_sms() -> Option<usize> {
    let out = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=multiprocessor_count", "--format=csv,noheader"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

fn grep_int(s: &str, key: &str) -> Option<i64> {
    let at = s.find(key)?;
    s[at + key.len()..]
        .split_whitespace()
        .next()?
        .trim()
        .parse()
        .ok()
}
