//! The page's data: the results JSON types and the feeds that load them.

use super::*;

#[derive(Deserialize)]
pub(super) struct BenchmarkData {
    pub(super) metadata: Metadata,
    pub(super) languages: Vec<Language>,
    pub(super) benchmarks: Vec<Benchmark>,
    pub(super) summary: SummaryData,
}

#[derive(Deserialize)]
pub(super) struct Metadata {
    pub(super) date: String,
    pub(super) commit: String,
    pub(super) logos_version: String,
    pub(super) cpu: String,
    pub(super) os: String,
    #[serde(default)]
    pub(super) warmup: Option<u32>,
    #[serde(default)]
    pub(super) runs: Option<u32>,
    pub(super) versions: HashMap<String, String>,
    #[serde(default)]
    pub(super) kernel: Option<String>,
    /// language id → the command that built it, as a reader would type it.
    #[serde(default)]
    pub(super) build_commands: HashMap<String, String>,
    #[serde(default)]
    pub(super) environment: Option<Environment>,
    /// The C runtime every LOGOS (C) program carries, measured once (the per-benchmark generated C omits it).
    #[serde(default)]
    pub(super) c_runtime: Option<CRuntime>,
    /// compile-timing row id → how the page names it (`benchmarks/lib/compilers.tsv`).
    #[serde(default)]
    pub(super) compilers: HashMap<String, CompilerMeta>,
}

#[derive(Deserialize, Default)]
pub(super) struct CompilerMeta {
    pub(super) label: String,
}

impl Metadata {
    /// How the page names a compile-timing row: its registry label, or its id when the run recorded none.
    pub(super) fn compiler_label<'a>(&'a self, id: &'a str) -> &'a str {
        self.compilers
            .get(id)
            .map(|c| c.label.as_str())
            .unwrap_or(id)
    }

    /// The run's date as `YYYY-MM-DD` — or the whole recorded string when it is shorter than that.
    pub(super) fn day(&self) -> &str {
        self.date.get(..10).unwrap_or(&self.date)
    }
}

/// The machine a run was measured on: the CPUs timed commands were pinned to, the frequency governor, the quiet-box
/// gate, the memory cap and the load average at either end.
#[derive(Deserialize, Default)]
pub(super) struct Environment {
    #[serde(default)]
    pub(super) pinning: Pinning,
    #[serde(default)]
    pub(super) governor: String,
    #[serde(default)]
    pub(super) no_turbo: String,
    #[serde(default)]
    pub(super) quiet_box: QuietBox,
    #[serde(default)]
    pub(super) memory_cap: String,
    #[serde(default)]
    pub(super) load: LoadAverage,
}

impl Environment {
    /// The run skipped the quiet-machine check, so its numbers may carry another workload's noise.
    pub(super) fn noisy(&self) -> bool {
        self.quiet_box.gate != "passed"
    }
}

#[derive(Deserialize, Default)]
pub(super) struct Pinning {
    #[serde(default)]
    pub(super) cpus: String,
    #[serde(default)]
    pub(super) source: String,
}

#[derive(Deserialize, Default)]
pub(super) struct QuietBox {
    /// `passed` (the machine was quiet), or `overridden` (the run was told to skip the check: a noisy run).
    #[serde(default)]
    pub(super) gate: String,
    #[serde(default)]
    pub(super) waited_seconds: u64,
}

#[derive(Deserialize, Default)]
pub(super) struct LoadAverage {
    #[serde(default)]
    pub(super) start: f64,
    #[serde(default)]
    pub(super) end: f64,
}

#[derive(Deserialize, Default)]
pub(super) struct CRuntime {
    #[serde(default)]
    pub(super) bytes: u64,
    #[serde(default)]
    pub(super) lines: u64,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub(super) struct Language {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) color: String,
    pub(super) tier: String,
}

impl Language {
    /// One of the three LOGOS tiers (compiled through Rust, compiled through C, the JIT).
    pub(super) fn is_logos(&self) -> bool {
        self.tier == "logos"
    }
}

#[derive(Deserialize)]
pub(super) struct Benchmark {
    pub(super) id: String,
    pub(super) name: String,
    #[serde(default)]
    pub(super) short: String,
    #[serde(default)]
    pub(super) category: String,
    pub(super) description: String,
    /// Why this benchmark is a collapse (the LOGOS optimizer ran a different algorithm), or `None`.
    #[serde(default)]
    pub(super) collapse: Option<String>,
    /// What the benchmark prints and what that proves.
    #[serde(default)]
    pub(super) output: Option<OutputProof>,
    pub(super) reference_size: String,
    pub(super) sizes: Vec<String>,
    pub(super) logos_source: String,
    pub(super) generated_rust: String,
    /// The program part of LOGOS (C)'s generated source (the shared runtime is `Metadata::c_runtime`).
    #[serde(default)]
    pub(super) generated_c: String,
    /// The program part of LOGOS (C, bytecode)'s generated source: the C lowered from the program's bytecode.
    #[serde(default)]
    pub(super) generated_c_bytecode: String,
    /// What the JIT did on this program: the engine that ran and what it compiled.
    #[serde(default)]
    pub(super) jit: Option<JitRecord>,
    pub(super) scaling: HashMap<String, HashMap<String, TimingResult>>,
    pub(super) compilation: HashMap<String, CompilationResult>,
    #[serde(default)]
    pub(super) timeouts: HashMap<String, f64>,
    /// size (or `compile`) → language id → the timeout (ms) it ran into.
    #[serde(default)]
    pub(super) timed_out: HashMap<String, HashMap<String, f64>>,
    /// size → language id → why that implementation cannot run at that size (a runtime limit).
    #[serde(default)]
    pub(super) limits: HashMap<String, HashMap<String, String>>,
    /// Peak RSS per language per size (kB). Absent in older result files.
    #[serde(default)]
    pub(super) memory: Option<MemoryData>,
    /// Compiled-artifact size per language (bytes), as-built and stripped. Does not
    /// vary with problem size, so it is a flat `by_language` map. Absent in older files.
    #[serde(default)]
    pub(super) binary_sizes: Option<BinarySizeData>,
    /// Declared time/space complexity. Absent in older result files.
    #[serde(default)]
    pub(super) complexity: Option<Complexity>,
    /// The optimizations that fired for this program (all-on) on any path — compiled, run, VM — baked by the
    /// benchmark run so the toggle tree pops in instantly. Absent in older files.
    #[serde(default)]
    pub(super) fired: Vec<String>,
    /// The same, per concrete path. JIT is distinct from VM so a native-tier
    /// lowering or fallback can never be represented by the bytecode trace.
    #[serde(default)]
    pub(super) fired_paths: FiredPaths,
    /// The ratchet view: shared optimizer transformations vs choices the
    /// concrete backend lowering actually made, plus any explicit fallback.
    #[serde(default)]
    pub(super) optimization_stages: OptimizationStages,
    /// The fired optimizations that replaced the program's algorithm rather than speeding it up — the reason for
    /// the collapse note.
    #[serde(default)]
    pub(super) changes_algorithm: Vec<String>,
    /// `(winner, loser)` blocker preemptions that occurred (all-on).
    #[serde(default)]
    pub(super) blockers: Vec<(String, String)>,
    /// `(dependent, dep)` emergent per-program dependencies (one fired only because
    /// another was on).
    #[serde(default)]
    pub(super) dependencies: Vec<(String, String)>,
}

/// Which optimizations fired on each path a program can take (`largo opts --json`'s `paths`).
#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
pub(super) struct FiredPaths {
    #[serde(default)]
    pub(super) compiled: Vec<String>,
    #[serde(default)]
    pub(super) c: Vec<String>,
    #[serde(default)]
    pub(super) c_bytecode: Vec<String>,
    #[serde(default)]
    pub(super) run: Vec<String>,
    #[serde(default)]
    pub(super) vm: Vec<String>,
    #[serde(default)]
    pub(super) jit: Vec<String>,
}

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
pub(super) struct OptimizationStageTrace {
    #[serde(default)]
    pub(super) optimizer: Vec<String>,
    #[serde(default)]
    pub(super) lowering: Vec<String>,
    #[serde(default)]
    pub(super) declines: Vec<String>,
    #[serde(default)]
    pub(super) fallbacks: Vec<String>,
}

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
pub(super) struct OptimizationStages {
    #[serde(default)]
    pub(super) compiled: OptimizationStageTrace,
    #[serde(default)]
    pub(super) c: OptimizationStageTrace,
    #[serde(default)]
    pub(super) c_bytecode: OptimizationStageTrace,
    #[serde(default)]
    pub(super) run: OptimizationStageTrace,
    #[serde(default)]
    pub(super) vm: OptimizationStageTrace,
    #[serde(default)]
    pub(super) jit: OptimizationStageTrace,
}

impl Benchmark {
    /// The note saying how the LOGOS optimizer changed this benchmark's algorithm, when it did — `None` when every
    /// language runs the same one.
    pub(super) fn collapse_note(&self) -> Option<&str> {
        self.collapse.as_deref().filter(|n| !n.trim().is_empty())
    }
}

#[derive(Deserialize, Clone, Default)]
pub(super) struct OutputProof {
    /// `fold` (an order-sensitive checksum over the whole result) or `answer` (the result is one value or tuple).
    #[serde(default)]
    pub(super) proof: String,
    #[serde(default)]
    pub(super) prints: String,
}

#[derive(Deserialize, Clone, Default)]
pub(super) struct JitRecord {
    #[serde(default)]
    pub(super) engine: Option<String>,
    #[serde(default)]
    pub(super) compiled: Option<String>,
    /// Stable reasons for native compile attempts that declined to bytecode.
    #[serde(default)]
    pub(super) declines: String,
    #[serde(default)]
    pub(super) optimizations: Vec<String>,
    /// Actual native entries/completions/deopts, not merely code that compiled.
    #[serde(default)]
    pub(super) execution: String,
}

#[derive(Deserialize, Clone)]
pub(super) struct MemoryData {
    #[allow(dead_code)]
    pub(super) method: String,
    /// size → language id → peak RSS in kB (`null` when a language was not measured).
    pub(super) by_size: HashMap<String, HashMap<String, Option<f64>>>,
}

#[derive(Deserialize, Clone)]
pub(super) struct BinarySizeData {
    #[allow(dead_code)]
    pub(super) method: String,
    /// language id → on-disk size of the compiled artifact in bytes.
    pub(super) by_language: HashMap<String, BinSizes>,
}

/// On-disk footprint of one artifact: the real shipped size and the code-only size
/// after symbol stripping. Shared by per-program binaries and the engine binaries.
#[derive(Deserialize, Clone, Copy)]
pub(super) struct BinSizes {
    /// Size exactly as the toolchain emits it (the real shipped artifact).
    pub(super) as_built: f64,
    /// Size after `strip --strip-all` on a throwaway copy. `null` when strip is
    /// unavailable or the artifact carries no symbols to remove.
    #[serde(default)]
    pub(super) stripped: Option<f64>,
}

#[derive(Deserialize, Clone)]
pub(super) struct Complexity {
    pub(super) time: String,
    pub(super) space: String,
}

#[derive(Deserialize, Clone)]
pub(super) struct TimingResult {
    pub(super) mean_ms: f64,
    pub(super) median_ms: f64,
    pub(super) stddev_ms: f64,
    pub(super) min_ms: f64,
    pub(super) max_ms: f64,
    pub(super) cv: f64,
    pub(super) runs: u32,
    #[serde(default)]
    pub(super) user_ms: Option<f64>,
    #[serde(default)]
    pub(super) system_ms: Option<f64>,
    /// Every timed run, in the order hyperfine took them.
    #[serde(default)]
    pub(super) times_ms: Vec<f64>,
}

#[derive(Deserialize, Clone)]
pub(super) struct CompilationResult {
    pub(super) mean_ms: f64,
    pub(super) stddev_ms: f64,
}

#[derive(Deserialize)]
pub(super) struct SummaryData {
    pub(super) geometric_mean_speedup_vs_c: HashMap<String, f64>,
    /// The same geometric mean without the collapses: where every language runs the same algorithm.
    #[serde(default)]
    pub(super) same_algorithm_speedup_vs_c: HashMap<String, f64>,
    #[serde(default)]
    pub(super) coverage: HashMap<String, Coverage>,
    /// language id → benchmark id → the size its ratio was taken at, where that is not the reference size.
    #[serde(default)]
    pub(super) ratio_sizes: HashMap<String, HashMap<String, String>>,
}

#[derive(Deserialize, Clone, Copy, Default)]
pub(super) struct Coverage {
    #[serde(default)]
    pub(super) benchmarks: u32,
    #[serde(default)]
    pub(super) of: u32,
    #[serde(default)]
    pub(super) at_reference_size: u32,
}

/// Fit a power law `t ≈ a·n^b` to `(n, t)` points by ordinary least squares on the
/// log-log transform; returns the exponent `b` — the EMPIRICAL big-O growth rate
/// the page shows next to the declared complexity. `None` for fewer than two
/// distinct positive points (no slope to fit).
pub(super) fn empirical_exponent(points: &[(f64, f64)]) -> Option<f64> {
    let pts: Vec<(f64, f64)> = points
        .iter()
        .filter(|&&(n, t)| n > 0.0 && t > 0.0)
        .map(|&(n, t)| (n.ln(), t.ln()))
        .collect();
    if pts.len() < 2 {
        return None;
    }
    let m = pts.len() as f64;
    let sx: f64 = pts.iter().map(|p| p.0).sum();
    let sy: f64 = pts.iter().map(|p| p.1).sum();
    let sxx: f64 = pts.iter().map(|p| p.0 * p.0).sum();
    let sxy: f64 = pts.iter().map(|p| p.0 * p.1).sum();
    let denom = m * sxx - sx * sx;
    if denom.abs() < 1e-12 {
        return None; // all x equal → undefined slope
    }
    Some((m * sxy - sx * sy) / denom)
}

// The page's data feeds. Native builds (tests, SSG prerender) compile the repo's
// benchmark results straight in; the wasm build fetches the identical bytes from
// `/data/*` (staged by scripts/web/stage-web-data.sh) so a megabyte of JSON never rides
// inside the shipped binary. Both sides answer through the same accessors below.
#[cfg(not(target_arch = "wasm32"))]
pub(super) static BENCH_DATA: LazyLock<BenchmarkData> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../../../../benchmarks/results/latest.json"
    ))
    .unwrap()
});
#[cfg(target_arch = "wasm32")]
pub(super) static BENCH_DATA: OnceLock<BenchmarkData> = OnceLock::new();

/// The language benchmark feed. `None` only on wasm before [`load_bench_bundle`]
/// resolves — the `Benchmarks` gate renders the loaded page strictly after that.
pub(super) fn bench_data() -> Option<&'static BenchmarkData> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(&BENCH_DATA)
    }
    #[cfg(target_arch = "wasm32")]
    {
        BENCH_DATA.get()
    }
}
