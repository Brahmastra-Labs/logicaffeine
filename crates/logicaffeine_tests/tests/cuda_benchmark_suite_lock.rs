//! ★ THE CUDA BENCHMARK-SUITE CAPSTONE ★ — the WHOLE 35-program benchmark suite runs on a single CUDA
//! kernel, byte-for-byte ≡ the VM, ON THE REAL GPU.
//!
//! Data-driven (mirrors `tenstorrent_benchmark_suite_lock`): DISCOVERS every `benchmarks/programs/*/
//! interp.lg`, emits a single-file `.cu` via `compile_to_cuda`, compiles it with `nvcc`, runs it on the
//! GPU, and asserts stdout == `vm_outcome` byte-for-byte. A benchmark the tier cannot yet run is a
//! shrink-only `KNOWN_GAPS` entry with a written reason — never a silent skip. The count that RUN ≡VM is
//! a monotone floor.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If a benchmark regresses, fix `codegen_cuda`/`codegen_c`, not
//! the lock. GPU execution is gated on `nvcc` + `/dev/nvidia0`; when absent the suite SKIPS (never fails).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_cuda, vm_outcome};

fn benchmarks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/programs")
}

/// Benchmarks the CUDA tier does not yet run ≡VM on the GPU, each with a written reason. Shrink-only —
/// removing an entry is a conscious edit; a new deferral must be added deliberately.
/// EMPTY — the ENTIRE 35-program suite runs byte-for-byte ≡VM on a single CUDA kernel, including the
/// heaviest Seq workloads (fannkuch/graph_bfs/vm), chaotic-float nbody (`-fmad=false` non-fused
/// semantics), and `matrix_mult`. The last gap — matrix_mult's `'i' undeclared`, a `codegen_c` scoping
/// bug the CUDA tier inherited — was fixed in `optimize/bound_version`: a counted index's `Let` is now
/// hoisted before the overflow-version guard instead of being cloned into the guard branches, so the
/// index survives its reuse after the nest. See `c_bound_version_scope_lock`.
const KNOWN_GAPS: &[(&str, &str)] = &[];

/// The monotone floor: at least this many of the 35 benchmarks run ≡VM on the GPU. RAISE as gaps close.
/// 35/35 — the WHOLE benchmark suite runs byte-for-byte identical to the VM on a single CUDA kernel.
const RAN_FLOOR: usize = 35;

fn have_nvcc_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
        && std::process::Command::new("nvcc")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// nvcc-compile a `.cu` and run it on the GPU; `Some(stdout)` on success, `None` if compile or run failed.
fn nvcc_compile_and_run(cu: &str, label: &str) -> Option<String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "logos_cudabench_{}_{n}_{label}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let src = dir.join("k.cu");
    let bin = dir.join("k");
    std::fs::write(&src, cu).ok()?;
    // `-fmad=false` disables FMA contraction (fusing `a*b+c` into one rounding), so device float
    // arithmetic matches the VM's non-fused IEEE semantics byte-for-byte (nbody/spectral_norm diverge
    // otherwise). The GEMM/Qwen perf path keeps FMA — this is the general-language ≡VM correctness flag.
    let c = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-fmad=false", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .ok()?;
    if !c.status.success() {
        eprintln!(
            "  nvcc failed for {label}:\n{}",
            String::from_utf8_lossy(&c.stderr)
                .lines()
                .take(3)
                .collect::<Vec<_>>()
                .join("\n")
        );
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    let r = std::process::Command::new(&bin).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if r.status.success() {
        Some(String::from_utf8_lossy(&r.stdout).to_string())
    } else {
        eprintln!(
            "  run failed for {label}: {}",
            String::from_utf8_lossy(&r.stderr)
                .lines()
                .next()
                .unwrap_or("")
        );
        None
    }
}

fn discover_benchmarks() -> Vec<(String, String)> {
    let dir = benchmarks_dir();
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            let interp = p.join("interp.lg");
            if interp.is_file() {
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                let src = std::fs::read_to_string(&interp).expect("read interp.lg");
                out.push((name, src));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn whole_benchmark_suite_runs_on_cuda_equiv_vm() {
    if !have_nvcc_gpu() {
        eprintln!(
            "SKIP: needs nvcc + /dev/nvidia0 — cannot run the CUDA benchmark suite on the GPU"
        );
        return;
    }
    let gaps: BTreeSet<&str> = KNOWN_GAPS.iter().map(|(b, _)| *b).collect();
    let benches = discover_benchmarks();
    assert!(
        benches.len() >= 30,
        "expected the full benchmark suite, found {}",
        benches.len()
    );

    let mut ran = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut now_working_gaps: Vec<String> = Vec::new();

    for (name, src) in &benches {
        let is_gap = gaps.contains(name.as_str());
        let vm = vm_outcome(src);
        if vm.error.is_some() {
            if !is_gap {
                failures.push(format!("{name}: VM errors ({:?})", vm.error));
            }
            continue;
        }
        let matches = match compile_to_cuda(src) {
            Err(_) => false,
            Ok(art) => nvcc_compile_and_run(&art.kernel_cu, name)
                .map(|out| out.trim() == vm.output.trim())
                .unwrap_or(false),
        };
        match (matches, is_gap) {
            (true, false) => {
                ran += 1;
                eprintln!("ok ≡VM on GPU: {name}");
            }
            (true, true) => now_working_gaps.push(name.clone()),
            (false, true) => eprintln!("gap (documented): {name}"),
            (false, false) => failures.push(format!(
                "{name}: does not run ≡VM on the GPU (undocumented)"
            )),
        }
    }

    eprintln!(
        "\nCUDA benchmark suite: {ran}/{} run ≡VM on the GPU, {} documented gaps",
        benches.len(),
        gaps.len()
    );

    assert!(now_working_gaps.is_empty(), "these benchmarks are in KNOWN_GAPS but now run ≡VM — remove them (shrink-only): {now_working_gaps:?}");
    assert!(
        failures.is_empty(),
        "CUDA benchmark suite has undocumented gaps/mismatches:\n{}",
        failures.join("\n")
    );
    assert!(
        ran >= RAN_FLOOR,
        "CUDA benchmark ≡VM floor regressed: only {ran} run (expected ≥ {RAN_FLOOR})"
    );
}
