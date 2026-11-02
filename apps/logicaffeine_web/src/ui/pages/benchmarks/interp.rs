//! The interpreter-vs-V8 data and the benchmark source bundle.

use super::*;

// The LOGOS interpreter (bytecode VM + JIT) vs Node/V8 — a separate peer-to-peer
// comparison at interpreter-calibrated sizes (so neither engine sits on V8's
// startup floor). Produced by benchmarks/run-interp-vs-js.sh.
#[derive(Deserialize)]
pub(super) struct InterpData {
    #[serde(default)]
    pub(super) metadata: InterpMetadata,
    #[serde(default)]
    pub(super) benchmarks: Vec<InterpBenchmark>,
    #[serde(default)]
    pub(super) summary: InterpSummary,
    #[serde(default)]
    pub(super) startup: InterpStartup,
    /// Engine footprint (bytes): the largo VM+JIT binary vs the host language runtimes,
    /// plus the browser WASM bundle. Absent in older result files.
    #[serde(default)]
    pub(super) interpreter_sizes: Option<InterpreterSizes>,
}

#[derive(Deserialize, Clone)]
pub(super) struct InterpreterSizes {
    #[allow(dead_code)]
    pub(super) method: String,
    /// engine id (logos, node, python, ruby, …) → on-disk size in bytes.
    pub(super) engines: HashMap<String, BinSizes>,
    /// Largest `.wasm` in the release web bundle — the in-browser interpreter footprint.
    /// `null` when no `dx` release build was present at measurement time.
    #[serde(default)]
    pub(super) wasm_bundle_bytes: Option<f64>,
}

#[derive(Deserialize, Default)]
pub(super) struct InterpStartup {
    #[serde(default)]
    pub(super) runs: u32,
    #[serde(default)]
    pub(super) engines: HashMap<String, StartupTiming>,
}

#[derive(Deserialize, Default, Clone)]
pub(super) struct StartupTiming {
    #[serde(default)]
    pub(super) mean_ms: f64,
}

#[derive(Deserialize, Default)]
pub(super) struct InterpMetadata {
    #[serde(default)]
    pub(super) node: String,
}

#[derive(Deserialize, Default)]
pub(super) struct InterpSummary {
    #[serde(default)]
    pub(super) geometric_mean_logos_interp_over_node: f64,
    /// The TIERED engine's geomean (HOTSWAP §12) — the A/B counterpart to the eager
    /// `_interp_` figure. `#[serde(default)]` so the pre-tiered JSON still loads.
    #[serde(default)]
    pub(super) geometric_mean_logos_tiered_over_node: f64,
    /// The AOT-NATIVE tier's geomean (HOTSWAP §Axis-3) — present only when a native
    /// bundle was built for the run. `#[serde(default)]` so non-bundled JSON still loads.
    #[serde(default)]
    pub(super) geometric_mean_logos_aot_over_node: f64,
}

#[derive(Deserialize)]
pub(super) struct InterpBenchmark {
    pub(super) id: String,
    #[serde(default)]
    pub(super) reference_size: String,
    #[serde(default)]
    pub(super) interpreter_engine: String,
    #[serde(default)]
    pub(super) scaling: HashMap<String, HashMap<String, TimingResult>>,
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) static INTERP_DATA: LazyLock<InterpData> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../../../../benchmarks/results/latest-interp.json"
    ))
    .unwrap()
});
#[cfg(target_arch = "wasm32")]
pub(super) static INTERP_DATA: OnceLock<InterpData> = OnceLock::new();

/// The interpreter-tier feed — same gating contract as [`bench_data`].
pub(super) fn interp_data() -> Option<&'static InterpData> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(&INTERP_DATA)
    }
    #[cfg(target_arch = "wasm32")]
    {
        INTERP_DATA.get()
    }
}

pub(super) struct BenchSources {
    pub(super) c: &'static str,
    pub(super) cpp: &'static str,
    pub(super) rust: &'static str,
    pub(super) zig: &'static str,
    pub(super) go: &'static str,
    pub(super) java: &'static str,
    pub(super) js: &'static str,
    pub(super) python: &'static str,
    pub(super) ruby: &'static str,
    pub(super) nim: &'static str,
    pub(super) logos: &'static str,
}

/// Every benchmark's eleven sources, one entry per row of `benchmarks/lib/benchmarks.tsv` (written by build.rs).
#[cfg(not(target_arch = "wasm32"))]
pub(super) static SOURCES: LazyLock<Vec<(&'static str, BenchSources)>> =
    LazyLock::new(|| include!(concat!(env!("OUT_DIR"), "/bench_sources.rs")));
#[cfg(target_arch = "wasm32")]
pub(super) static SOURCES: OnceLock<HashMap<String, BenchSources>> = OnceLock::new();

/// The 11-language sources for one benchmark, looked up by its `latest.json` id —
/// same gating contract as [`bench_data`].
pub(super) fn bench_sources_for(id: &str) -> Option<&'static BenchSources> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        SOURCES.iter().find(|(name, _)| *name == id).map(|(_, s)| s)
    }
    #[cfg(target_arch = "wasm32")]
    {
        SOURCES.get()?.get(id)
    }
}

/// Owned mirror of [`BenchSources`] for deserializing the staged `bench-sources.json`;
/// pinned via `Box::leak` so the page keeps its `&'static str` shape on both targets.
#[cfg(target_arch = "wasm32")]
#[derive(Deserialize)]
pub(super) struct BenchSourcesOwned {
    pub(super) c: String,
    pub(super) cpp: String,
    pub(super) rust: String,
    pub(super) zig: String,
    pub(super) go: String,
    pub(super) java: String,
    pub(super) js: String,
    pub(super) python: String,
    pub(super) ruby: String,
    pub(super) nim: String,
    pub(super) logos: String,
}

#[cfg(target_arch = "wasm32")]
impl BenchSourcesOwned {
    pub(super) fn pin(self) -> BenchSources {
        fn pin(s: String) -> &'static str {
            Box::leak(s.into_boxed_str())
        }
        BenchSources {
            c: pin(self.c),
            cpp: pin(self.cpp),
            rust: pin(self.rust),
            zig: pin(self.zig),
            go: pin(self.go),
            java: pin(self.java),
            js: pin(self.js),
            python: pin(self.python),
            ruby: pin(self.ruby),
            nim: pin(self.nim),
            logos: pin(self.logos),
        }
    }
}

/// Fetches the staged `/data` bundle once and pins it for the app's lifetime; repeat
/// visits to the page re-run the (now instant) loads and keep the first pinned value.
#[cfg(target_arch = "wasm32")]
pub(super) async fn load_bench_bundle() -> Result<(), String> {
    use crate::ui::data_fetch::fetch_static_json;
    let bench: BenchmarkData = fetch_static_json("/data/latest.json").await?;
    let solver: SolverData = fetch_static_json("/data/solvers.json").await?;
    let codec: CodecData = fetch_static_json("/data/latest-codec.json").await?;
    let interp: InterpData = fetch_static_json("/data/latest-interp.json").await?;
    let sources: HashMap<String, BenchSourcesOwned> =
        fetch_static_json("/data/bench-sources.json").await?;
    let _ = BENCH_DATA.set(bench);
    let _ = SOLVER_DATA.set(solver);
    let _ = CODEC_DATA.set(codec);
    let _ = INTERP_DATA.set(interp);
    let _ = SOURCES.set(sources.into_iter().map(|(id, s)| (id, s.pin())).collect());
    Ok(())
}
