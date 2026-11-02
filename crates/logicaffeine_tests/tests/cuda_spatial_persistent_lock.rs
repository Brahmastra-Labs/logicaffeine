//! ★ SPATIAL → CUDA PERSISTENT-KERNEL LOCK ★ — the SAME spatial IR lowers to a warp-specialized persistent
//! kernel on the local RTX 3060, and it is proven against the IR's semantics.
//!
//! A site is a resident thread block looping over ITS static work list (the IR's placement, the same static
//! assignment every other target runs); FIFO buffers are shared-memory rings whose pages are `cp.async`
//! commit groups (a `Push` commits, a `Wait` waits on the groups and crosses a block barrier); semaphore edges
//! are split-arrive barriers; barrier-role semaphores are named barriers; loads are every thread's 16-byte
//! `cp.async` share of a page; tile matmuls are WMMA fragments. This lock proves:
//!   A. the program is ONE `__global__` and its census is the IR's (rings == FIFO templates, barrier arrays ==
//!      semaphore edges, named barriers == barrier-role semaphores, one persistent item loop per site);
//!   B. it compiles with nvcc for bf16 and int8 operands and for a dynamic-shared-memory depth;
//!   C. on the GPU its output equals `eval_choreography` (and the systolic reference) for several shapes
//!      including ragged ones;
//!   D. its measured throughput at 2048³ bf16 is at least 0.9× the CUDA knob lattice's best rung and at
//!      least 0.9× the recorded floor (ratchet-only-up, recorded under `SPATIAL_RECORD=1`);
//!   E. a multicast group that would span sites is refused typed (sm_86 has no clusters).
//!
//! ⚠️ If a lock goes RED, fix the lowering, never the assertion.

use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::{Handshake, NumericsPolicy, TensorId};
use logicaffeine_compile::spatial::lower::cuda::{CudaArtifact, CudaPersistentBackend};
use logicaffeine_compile::spatial::lower::{
    Census, SpatialArtifact, SpatialBackend, SpatialLowerError,
};
use logicaffeine_compile::spatial::passes::from_tensor_op::MatmulNames;
use logicaffeine_compile::spatial::pipeline::{
    calibration_candidates, run_passes, template_from_spec, CandidateKind,
};
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_recognize::{Fill, MatmulSpec};

fn have_nvcc() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}

fn prng(seed: u64, i: usize, lo: i64, hi: i64) -> i64 {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(i as u64 + 1);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    lo + (x % (hi - lo + 1) as u64) as i64
}

/// The GEMM source the candidate search recognizes (the same program every spatial lock compiles).
fn gemm_source(m: usize, n: usize, k: usize) -> String {
    format!(
        "## Main\nLet rows be {m}.\nLet cols be {n}.\nLet mid be {k}.\nLet mutable a be a new Seq of Int.\nLet mutable b be a new Seq of Int.\nLet mutable c be a new Seq of Int.\nLet mutable p be 0.\nWhile p is less than rows * mid:\n    Push (p % 7) - 3 to a.\n    Set p to p + 1.\nSet p to 0.\nWhile p is less than mid * cols:\n    Push (p % 11) - 5 to b.\n    Set p to p + 1.\nSet p to 0.\nWhile p is less than rows * cols:\n    Push 0 to c.\n    Set p to p + 1.\nLet mutable i be 0.\nWhile i is less than rows:\n    Let mutable k be 0.\n    While k is less than mid:\n        Let mutable j be 0.\n        While j is less than cols:\n            Let idx be i * cols + j + 1.\n            Set item idx of c to item idx of c + item (i * mid + k + 1) of a * item (k * cols + j + 1) of b.\n            Set j to j + 1.\n        Set k to k + 1.\n    Set i to i + 1.\nLet mutable checksum be 0.\nLet mutable q be 1.\nWhile q is at most rows * cols:\n    Set checksum to checksum + item q of c.\n    Set q to q + 1.\nShow checksum.\n"
    )
}

fn names() -> MatmulNames {
    MatmulNames {
        a: "a".into(),
        b: "b".into(),
        c: "c".into(),
    }
}

/// An int8 program (the fills' range fits int8 → int8 WMMA with i32 accumulate) or a bf16 program (unknown
/// range → bf16 operands, fp32 accumulate).
fn program(
    m: usize,
    n: usize,
    k: usize,
    int8: bool,
) -> logicaffeine_compile::spatial::ir::SpatialProgram {
    let gpu = SpatialTarget::rtx3060();
    let spec = MatmulSpec {
        m,
        n,
        k,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let ranges = if int8 {
        (Some((-128, 127)), Some((-128, 127)))
    } else {
        (None, None)
    };
    let t = template_from_spec(
        &spec,
        &names(),
        (&Fill::Unknown, &Fill::Unknown),
        ranges,
        NumericsPolicy::Exact,
        &gpu,
    )
    .unwrap();
    run_passes(t, &gpu).unwrap()
}

fn lower(p: &logicaffeine_compile::spatial::ir::SpatialProgram) -> CudaArtifact {
    match CudaPersistentBackend::new(SpatialTarget::rtx3060())
        .lower(p)
        .unwrap()
    {
        SpatialArtifact::Cuda(a) => a,
        other => panic!("{other:?}"),
    }
}

fn nvcc(cu: &str, label: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("logos_spatial_cuda_{}_{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("persistent.cu");
    let bin = dir.join("persistent");
    std::fs::write(&src, cu).unwrap();
    let out = std::process::Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-std=c++17", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("nvcc");
    assert!(
        out.status.success(),
        "{label}: nvcc failed:\n{}\n\n{cu}",
        String::from_utf8_lossy(&out.stderr)
    );
    bin
}

#[test]
fn persistent_program_has_one_global_and_the_ir_census() {
    for (int8, label) in [(true, "int8"), (false, "bf16")] {
        let p = program(256, 256, 256, int8);
        let art = lower(&p);
        let cu = &art.program_cu;
        assert_eq!(
            cu.matches("__global__").count(),
            1,
            "{label}: ONE persistent kernel"
        );
        // Each site walks ITS OWN slice of the static work list. Two spellings, one property: a
        // loop when some site holds several items, and the single-item form (with a guard for the
        // sites holding none) when no site holds more than one — which is what a big block tile on
        // a small problem produces. Pinning only the loop made a BETTER plan look like a
        // regression, so the assertion is the property rather than one of its spellings.
        let walks_its_slice = cu.contains("for (int item = 0; item < n_items; ++item)")
            || (cu.contains("const int item_id = ITEM_LIST[item_off];")
                && cu.contains("if (n_items == 0) { return; }"));
        assert!(
            walks_its_slice,
            "{label}: the site walks its slice of the static work list"
        );
        assert!(
            cu.contains("ITEM_LIST["),
            "{label}: the work list IS the placement, item by item"
        );
        assert_eq!(
            cu.matches("atomicAdd(").count(),
            0,
            "{label}: static work lists need no atomic counter"
        );
        assert_eq!(
            art.census,
            Census::of(&p),
            "{label}: the artifact reports the IR census"
        );
        let fifo_templates = p
            .buffers
            .iter()
            .filter(|b| {
                matches!(
                    b.kind,
                    logicaffeine_compile::spatial::ir::BufferKind::Fifo { .. }
                )
            })
            .map(|b| b.origin.unwrap_or(b.id))
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let rings = cu
            .lines()
            .filter(|l| {
                l.contains("ring_") && (l.contains("[DEPTH_") || l.contains(" = reinterpret_cast<"))
            })
            .count();
        assert_eq!(
            rings, fifo_templates,
            "{label}: one shared ring per FIFO template"
        );
        let sem_edges = p
            .edges
            .iter()
            .filter(|e| matches!(e.handshake, Handshake::Semaphore { .. }))
            .count();
        assert_eq!(
            cu.matches("cuda::barrier<cuda::thread_scope_block>")
                .count(),
            sem_edges,
            "{label}: one barrier array per semaphore edge"
        );
        let barrier_sems = p
            .semaphores
            .iter()
            .filter(|s| {
                matches!(
                    s.role,
                    logicaffeine_compile::spatial::ir::SemRole::Barrier { .. }
                )
            })
            .count();
        assert_eq!(
            cu.matches("bar.sync").count(),
            barrier_sems,
            "{label}: named barriers are the images of barrier-role semaphores"
        );
        assert!(cu.contains("__launch_bounds__("));
        assert!(
            art.budget.smem_bytes > 0
                && art.budget.blocks_per_sm >= 1
                && art.budget.threads % 32 == 0
        );
        assert!(art.bench_cu.contains("GOPS"));
    }
}

#[test]
fn persistent_program_compiles_with_nvcc() {
    if !have_nvcc() {
        eprintln!("SKIP: nvcc not installed");
        return;
    }
    for (int8, label) in [(true, "int8"), (false, "bf16")] {
        let art = lower(&program(256, 256, 256, int8));
        nvcc(&art.program_cu, label);
    }
    // A depth that exceeds 48 KiB static shared memory takes the dynamic opt-in path.
    let mut deep = SpatialTarget::rtx3060();
    deep.site.local_mem_bytes = 96 * 1024;
    let spec = MatmulSpec {
        m: 512,
        n: 512,
        k: 512,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let p = run_passes(
        template_from_spec(
            &spec,
            &names(),
            (&Fill::Unknown, &Fill::Unknown),
            (None, None),
            NumericsPolicy::Exact,
            &deep,
        )
        .unwrap(),
        &deep,
    )
    .unwrap();
    let art = match CudaPersistentBackend::new(deep).lower(&p).unwrap() {
        SpatialArtifact::Cuda(a) => a,
        other => panic!("{other:?}"),
    };
    if art.budget.dynamic_opt_in {
        assert!(
            art.program_cu.contains("cudaFuncSetAttribute")
                && art.program_cu.contains("MaxDynamicSharedMemorySize")
        );
    }
    nvcc(&art.program_cu, "dynamic");
}

#[test]
fn persistent_kernel_equals_eval_choreography_on_the_gpu() {
    if !have_nvcc() || !have_gpu() {
        eprintln!("SKIP: needs nvcc + /dev/nvidia0");
        return;
    }
    for (i, &(m, n, k)) in [
        (128usize, 32usize, 128usize),
        (256, 256, 256),
        (384, 128, 512),
        (200, 96, 136),
        (130, 40, 70),
    ]
    .iter()
    .enumerate()
    {
        let p = program(m, n, k, true);
        let acts: Vec<i64> = (0..m * k)
            .map(|j| prng(100 + i as u64, j, -128, 127))
            .collect();
        let weights: Vec<i64> = (0..k * n)
            .map(|j| prng(200 + i as u64, j, -128, 127))
            .collect();
        let mut inputs = Inputs::default();
        inputs.tensors.insert(TensorId(0), Value::Int(acts.clone()));
        inputs
            .tensors
            .insert(TensorId(1), Value::Int(weights.clone()));
        let expected = match &eval_choreography(&p, &inputs).unwrap().tensors[&TensorId(2)] {
            Value::Int(v) => v.clone(),
            Value::Float(v) => v.iter().map(|x| *x as i64).collect(),
        };
        let art = lower(&p);
        let bin = nvcc(&art.program_cu, &format!("gpu{i}"));
        let dir = bin.parent().unwrap();
        std::fs::write(
            dir.join("a.txt"),
            acts.iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join("b.txt"),
            weights
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let run = std::process::Command::new(&bin)
            .current_dir(dir)
            .output()
            .expect("run");
        assert!(
            run.status.success(),
            "{m}x{n}x{k}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        let got: Vec<i64> = String::from_utf8_lossy(&run.stdout)
            .split_whitespace()
            .filter_map(|t| t.parse().ok())
            .collect();
        assert_eq!(
            got.len(),
            m * n,
            "{m}x{n}x{k}: the program prints every output element"
        );
        assert_eq!(got, expected, "{m}x{n}x{k}: GPU != eval_choreography");
        let sys = logicaffeine_compile::codegen_sva::systolic::Systolic::new(k, n, weights);
        assert_eq!(
            got,
            logicaffeine_compile::codegen_sva::systolic::eval_systolic(&sys, &acts, m)
        );
    }
}

fn gops_of(bin: &std::path::Path) -> f64 {
    let mut best = 0f64;
    for _ in 0..5 {
        let out = std::process::Command::new(bin).output().expect("bench");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(rest) = line.strip_prefix("GOPS ") {
                best = best.max(rest.trim().parse::<f64>().unwrap());
            }
        }
    }
    assert!(best > 0.0, "no GOPS line");
    best
}

#[test]
fn persistent_throughput_ratchets_against_the_lattice_best() {
    if !have_nvcc() || !have_gpu() {
        eprintln!("SKIP: needs nvcc + /dev/nvidia0");
        return;
    }
    use logicaffeine_compile::codegen_cuda::gemm_tile::{lattice_best_bf16_bench, GpuLimits};
    let (m, k, n) = (2048usize, 2048usize, 2048usize);
    let p = program(m, n, k, false);
    let art = lower(&p);
    let ours = nvcc(&art.bench_cu, "bench_persistent");
    let baseline_cu = lattice_best_bf16_bench(m, k, n, 86, &GpuLimits::ampere());
    let theirs = nvcc(&baseline_cu, "bench_lattice");
    let (mut g_ours, mut g_lattice) = (0f64, 0f64);
    for _ in 0..3 {
        g_ours = g_ours.max(gops_of(&ours));
        g_lattice = g_lattice.max(gops_of(&theirs));
    }
    let ratio = g_ours / g_lattice;
    eprintln!("persistent {g_ours:.0} GOPS vs lattice best {g_lattice:.0} GOPS (ratio {ratio:.3})");
    assert!(
        ratio >= 0.9,
        "the persistent kernel must reach 0.9× the knob lattice's best rung: {ratio:.3}"
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let json_path = root.join("benchmarks/results/cuda_spatial_persistent.json");
    let recorded: Option<serde_json::Value> = std::fs::read_to_string(&json_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    if let Some(r) = &recorded {
        let floor = r["persistent"]["gops"].as_f64().unwrap();
        assert!(
            g_ours >= 0.9 * floor,
            "regression below the recorded floor {floor:.0} (got {g_ours:.0})"
        );
    }
    if std::env::var("SPATIAL_RECORD").is_ok() {
        let better = recorded
            .as_ref()
            .map(|r| g_ours > r["persistent"]["gops"].as_f64().unwrap())
            .unwrap_or(true);
        if better {
            let v = serde_json::json!({
                "schema_version": 1,
                "gpu": "rtx3060",
                "shape": [m, n, k],
                "precision": "bf16",
                "persistent": {"gops": g_ours, "config": art.budget, "census": art.census},
                "lattice_best": {"gops": g_lattice},
                "ratio": ratio,
                "floor_ratio": 0.9,
                "provenance": "measured_rtx3060",
                "reproduce": "SPATIAL_RECORD=1 cargo nextest run -p logicaffeine-tests --test cuda_spatial_persistent_lock -E 'test(persistent_throughput_ratchets_against_the_lattice_best)'",
            });
            std::fs::write(&json_path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        }
    } else {
        assert!(
            recorded.is_some(),
            "record the first measurement with SPATIAL_RECORD=1"
        );
    }
}

#[test]
fn a_group_spanning_sites_is_refused_on_sm86() {
    // Hand a program placed for a multicast mesh to the GPU backend: rectangle multicast has no sm_86 image.
    let wh = SpatialTarget::wormhole();
    let spec = MatmulSpec {
        m: 2048,
        n: 1024,
        k: 512,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    // The fused arrangement explicitly: whether the cost model PREFERS multicast at this shape is a
    // separate question, and this test is about what the GPU backend does when handed one.
    let t = template_from_spec(
        &spec,
        &names(),
        (&Fill::Unknown, &Fill::Unknown),
        (Some((-3, 3)), Some((-2, 2))),
        NumericsPolicy::Exact,
        &wh,
    )
    .unwrap();
    let p = calibration_candidates(&gemm_source(2048, 1024, 512), &wh, NumericsPolicy::Exact)
        .ok()
        .and_then(|c| {
            c.into_iter()
                .find(|(k, p)| *k == CandidateKind::Fused && p.summary().multicast_groups > 0)
                .map(|(_, p)| p)
        })
        .unwrap_or_else(|| run_passes(t, &wh).unwrap());
    assert!(
        p.summary().multicast_groups > 0,
        "the fused candidate multicasts"
    );
    match CudaPersistentBackend::new(SpatialTarget::rtx3060()).lower(&p) {
        Err(SpatialLowerError::GroupSpansSites { .. }) | Err(SpatialLowerError::Unsupported(_)) => {
        }
        other => panic!("{other:?}"),
    }
}
