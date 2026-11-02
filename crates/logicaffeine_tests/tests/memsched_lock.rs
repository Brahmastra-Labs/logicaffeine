//! memsched_lock — the measured static-memory-schedule fractions of the REAL benchmark
//! kernels, pinned as ratchet floors.
//!
//! The claim that a compiled kernel's memory access schedule is static is backed here by
//! measurement: `codegen_sva::memsched::analyze` classifies every datapath memory site of each
//! compiled kernel, deliberately under-approximating (unproven → dynamic), so each pinned floor is
//! conservative and may only RISE as the analysis or the compiler sharpens. graph_bfs is the honest
//! tail — pointer-chasing — and its floor is pinned LOW on purpose.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::{Path, PathBuf};

use logicaffeine_compile::codegen_sva::memsched::{
    analyze, cache_plan, explain_dynamic_sites, MemSchedReport,
};
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::Compiler;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn report_for(kernel: &str) -> (MemSchedReport, String) {
    let path = root().join(format!("benchmarks/programs/{kernel}/main.lg"));
    let src =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    with_parsed_program(&src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.unwrap_or_else(|e| panic!("{kernel}: {e:?}"));
        let program = Compiler::compile_with_types(stmts, interner, Some(types))
            .unwrap_or_else(|e| panic!("{kernel} compiles: {e}"));
        (analyze(&program), explain_dynamic_sites(&program, 12))
    })
}

/// Run `f` with the compiled program for `kernel`.
fn with_program<R>(
    kernel: &str,
    f: impl FnOnce(&logicaffeine_compile::vm::CompiledProgram) -> R,
) -> R {
    let path = root().join(format!("benchmarks/programs/{kernel}/main.lg"));
    let src =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    with_parsed_program(&src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.unwrap_or_else(|e| panic!("{kernel}: {e:?}"));
        let program = Compiler::compile_with_types(stmts, interner, Some(types))
            .unwrap_or_else(|e| panic!("{kernel} compiles: {e}"));
        f(&program)
    })
}

/// Per-kernel floors, pinned AT the 2026-07-14 measurement (flow-sensitive
/// reaching-defs + payload-taint analysis); floors only rise.
///   loop_sum 1.0000 · nbody 1.0000 (120 static / 0 dynamic) · mergesort 0.9643 ·
///   graph_bfs 0.8478 (the honest pointer-chasing tail).
const FLOORS: &[(&str, f64)] = &[
    ("loop_sum", 1.00),
    ("nbody", 1.00),
    ("mergesort", 0.96),
    ("graph_bfs", 0.84),
];

#[test]
fn benchmark_kernels_meet_their_static_fraction_floors() {
    for &(kernel, floor) in FLOORS {
        let (r, why) = report_for(kernel);
        let f = r.static_fraction();
        println!(
            "{kernel}: static={} dynamic={} coprocessor={} nomem={} → static_fraction={f:.4}",
            r.static_sites, r.dynamic_sites, r.coprocessor_sites, r.nomem_sites
        );
        assert!(
            f >= floor,
            "{kernel}: static fraction {f:.4} fell below its pinned floor {floor:.2} — a \
             regression in the compiler or the analysis; floors only rise.\nFirst dynamic \
             sites:\n{why}"
        );
    }
}

/// The streaming kernels must have real datapath memory traffic (the fractions above
/// are not vacuous 0/0 wins).
#[test]
fn streaming_kernels_have_datapath_memory_sites() {
    for kernel in ["nbody", "mergesort", "graph_bfs"] {
        let (r, _) = report_for(kernel);
        assert!(
            r.datapath_sites() > 0,
            "{kernel}: expected datapath memory sites, got {r:?}"
        );
    }
}

/// The software-managed-cache acceptance contract (GAPS G19, `MEMORY_EDGE_SPEC.md §3`):
/// the managed-cache PROBE list — the only sites that can miss — equals the compiler's
/// `dynamic_sites` exactly, the partition is TOTAL (no datapath access dropped), and the
/// STREAMING kernels have an EMPTY probe list (they never touch the managed-cache path,
/// so their zero-jitter contract is preserved) while the honest tail (graph_bfs) does.
#[test]
fn managed_cache_probe_set_equals_the_dynamic_sites() {
    for kernel in ["loop_sum", "nbody", "mergesort", "graph_bfs"] {
        let (report, plan) = with_program(kernel, |prog| (analyze(prog), cache_plan(prog)));
        assert!(
            plan.is_total_over(&report),
            "{kernel}: managed-cache plan must partition every datapath site \
             (prefetch {} + probe {} vs static {} + dynamic {})",
            plan.prefetch.len(),
            plan.probe.len(),
            report.static_sites,
            report.dynamic_sites
        );
        assert_eq!(
            plan.probe.len(),
            report.dynamic_sites,
            "{kernel}: the managed-cache miss set must equal the compiler's dynamic_sites"
        );
    }
    // Streaming kernels: ZERO managed-cache traffic → zero-jitter preserved.
    for kernel in ["loop_sum", "nbody"] {
        let plan = with_program(kernel, cache_plan);
        assert!(
            plan.probe.is_empty(),
            "{kernel} is fully static — the managed-cache path must never engage \
             (probe list should be empty, got {})",
            plan.probe.len()
        );
    }
    // The honest tail engages the fallback on exactly its data-dependent gathers.
    let bfs = with_program("graph_bfs", cache_plan);
    assert!(
        !bfs.probe.is_empty(),
        "graph_bfs (pointer-chasing) must route its dynamic gathers through the managed cache"
    );
}
