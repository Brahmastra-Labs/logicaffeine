use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::LazyLock;
#[cfg(target_arch = "wasm32")]
use std::sync::OnceLock;
use dioxus::prelude::*;
#[cfg(all(feature = "split", target_arch = "wasm32"))]
use dioxus::wasm_split;
use serde::Deserialize;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::footer::Footer;
use crate::ui::seo::{JsonLdMultiple, PageHead, organization_schema, breadcrumb_schema, webpage_schema, BreadcrumbItem, pages as seo_pages};

/// The toggle vector (one bool per registry optimization, in discriminant order)
/// as an [`OptimizationConfig`]. Index `i` ↔ `REGISTRY[i].opt` ↔ bit `i`.
fn config_from_toggles(toggles: &[bool]) -> logicaffeine_compile::optimization::OptimizationConfig {
    let mut cfg = logicaffeine_compile::optimization::OptimizationConfig::all_off();
    for (i, m) in logicaffeine_compile::optimization::REGISTRY.iter().enumerate() {
        if toggles.get(i).copied().unwrap_or(false) {
            cfg.set(m.opt, true);
        }
    }
    cfg
}

/// The inverse of [`config_from_toggles`].
fn toggles_from_config(cfg: &logicaffeine_compile::optimization::OptimizationConfig) -> Vec<bool> {
    logicaffeine_compile::optimization::REGISTRY
        .iter()
        .map(|m| cfg.is_on(m.opt))
        .collect()
}

#[derive(Deserialize)]
struct BenchmarkData {
    metadata: Metadata,
    languages: Vec<Language>,
    benchmarks: Vec<Benchmark>,
    summary: SummaryData,
}

#[derive(Deserialize)]
struct Metadata {
    date: String,
    commit: String,
    logos_version: String,
    cpu: String,
    os: String,
    #[serde(default)]
    warmup: Option<u32>,
    #[serde(default)]
    runs: Option<u32>,
    versions: HashMap<String, String>,
}

#[derive(Deserialize)]
struct Language {
    id: String,
    label: String,
    color: String,
    tier: String,
}

#[derive(Deserialize)]
struct Benchmark {
    id: String,
    name: String,
    description: String,
    reference_size: String,
    sizes: Vec<String>,
    logos_source: String,
    generated_rust: String,
    scaling: HashMap<String, HashMap<String, TimingResult>>,
    compilation: HashMap<String, CompilationResult>,
    #[serde(default)]
    timeouts: HashMap<String, f64>,
    /// Peak RSS per language per size (kB). Absent in older result files.
    #[serde(default)]
    memory: Option<MemoryData>,
    /// Compiled-artifact size per language (bytes), as-built and stripped. Does not
    /// vary with problem size, so it is a flat `by_language` map. Absent in older files.
    #[serde(default)]
    binary_sizes: Option<BinarySizeData>,
    /// Declared time/space complexity. Absent in older result files.
    #[serde(default)]
    complexity: Option<Complexity>,
    /// The optimizations that fired for this program (all-on), baked by the
    /// benchmark run so the toggle tree pops in instantly. Absent in older files.
    #[serde(default)]
    fired: Vec<String>,
    /// `(winner, loser)` blocker preemptions that occurred (all-on).
    #[serde(default)]
    blockers: Vec<(String, String)>,
    /// `(dependent, dep)` emergent per-program dependencies (one fired only because
    /// another was on).
    #[serde(default)]
    dependencies: Vec<(String, String)>,
}

#[derive(Deserialize, Clone)]
struct MemoryData {
    #[allow(dead_code)]
    method: String,
    /// size → language id → peak RSS in kB (`null` when a language was not measured).
    by_size: HashMap<String, HashMap<String, Option<f64>>>,
}

#[derive(Deserialize, Clone)]
struct BinarySizeData {
    #[allow(dead_code)]
    method: String,
    /// language id → on-disk size of the compiled artifact in bytes.
    by_language: HashMap<String, BinSizes>,
}

/// On-disk footprint of one artifact: the real shipped size and the code-only size
/// after symbol stripping. Shared by per-program binaries and the engine binaries.
#[derive(Deserialize, Clone, Copy)]
struct BinSizes {
    /// Size exactly as the toolchain emits it (the real shipped artifact).
    as_built: f64,
    /// Size after `strip --strip-all` on a throwaway copy. `null` when strip is
    /// unavailable or the artifact carries no symbols to remove.
    #[serde(default)]
    stripped: Option<f64>,
}

#[derive(Deserialize, Clone)]
struct Complexity {
    time: String,
    space: String,
}

#[derive(Deserialize, Clone)]
struct TimingResult {
    mean_ms: f64,
    median_ms: f64,
    stddev_ms: f64,
    min_ms: f64,
    max_ms: f64,
    cv: f64,
    runs: u32,
    #[serde(default)]
    user_ms: Option<f64>,
    #[serde(default)]
    system_ms: Option<f64>,
}

#[derive(Deserialize, Clone)]
struct CompilationResult {
    mean_ms: f64,
    stddev_ms: f64,
}

#[derive(Deserialize)]
struct SummaryData {
    geometric_mean_speedup_vs_c: HashMap<String, f64>,
}

/// Fit a power law `t ≈ a·n^b` to `(n, t)` points by ordinary least squares on the
/// log-log transform; returns the exponent `b` — the EMPIRICAL big-O growth rate
/// the page shows next to the declared complexity. `None` for fewer than two
/// distinct positive points (no slope to fit).
fn empirical_exponent(points: &[(f64, f64)]) -> Option<f64> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empirical_exponent_fits_linear() {
        let pts: Vec<(f64, f64)> = [10.0, 100.0, 1000.0, 10000.0].iter().map(|&n| (n, 3.0 * n)).collect();
        let b = empirical_exponent(&pts).expect("two+ points");
        assert!((b - 1.0).abs() < 0.01, "linear data → exponent ~1.0, got {b}");
    }

    #[test]
    fn empirical_exponent_fits_quadratic() {
        let pts: Vec<(f64, f64)> = [10.0, 100.0, 1000.0].iter().map(|&n| (n, 2.0 * n * n)).collect();
        let b = empirical_exponent(&pts).expect("two+ points");
        assert!((b - 2.0).abs() < 0.01, "quadratic data → exponent ~2.0, got {b}");
    }

    #[test]
    fn empirical_exponent_degenerate_is_none() {
        assert!(empirical_exponent(&[(10.0, 5.0)]).is_none(), "one point → None");
        assert!(empirical_exponent(&[]).is_none(), "no points → None");
        assert!(empirical_exponent(&[(5.0, 1.0), (5.0, 9.0)]).is_none(), "all-equal n → None");
    }

    #[test]
    fn linear_bar_pct_is_honest_and_proportional() {
        // The largest value fills the track.
        assert!((linear_bar_pct(200.0, 200.0) - 100.0).abs() < 1e-9);
        // A bar's length is exactly its fraction of the max — no log distortion.
        assert!((linear_bar_pct(50.0, 200.0) - 25.0).abs() < 1e-9);
        assert!((linear_bar_pct(100.0, 200.0) - 50.0).abs() < 1e-9);
        // The ratio between two bars equals the ratio between their values.
        let a = linear_bar_pct(50.0, 200.0);
        let b = linear_bar_pct(100.0, 200.0);
        assert!((b / a - 2.0).abs() < 1e-9, "2x value → 2x bar, got {a} {b}");
        // The user's grievance, made a test: ~70 µs against a 75 ms field is a sliver
        // (< 0.2 %), NOT a third of the competitor's bar the way a log scale drew it.
        assert!(
            linear_bar_pct(0.070, 75.0) < 0.2,
            "a ~1000x speed win must read as a sliver, not a third"
        );
        // 2.5x vs 1.21x geomean speed reads ~100 % vs ~48 % — plainly ~2x apart, not a smidge.
        assert!((linear_bar_pct(2.544, 2.544) - 100.0).abs() < 1e-9);
        let rust = linear_bar_pct(1.210, 2.544);
        assert!((rust - 47.56).abs() < 0.1, "1.21/2.544 → ~47.6 %, got {rust}");
        // Never overflows the track; a degenerate (zero) max is safe.
        assert!(linear_bar_pct(500.0, 200.0) <= 100.0);
        assert!(linear_bar_pct(5.0, 0.0) == 0.0);
    }

    #[test]
    fn solver_bars_track_true_time_ratios() {
        // On every family we present as a win, the faster solver must draw the shorter
        // bar, and by the TRUE ratio: on a shared per-row scale, ours vs a competitor is
        // ours_ms / competitor_ms — the honest comparative the log scale was hiding.
        let d = &*SOLVER_DATA;
        for fam in d.families.iter() {
            for r in &fam.rows {
                let scale = r.others.iter().map(|o| o.ms).fold(r.ours_ms, f64::max);
                let ours = linear_bar_pct(r.ours_ms, scale);
                for o in r.others.iter().filter(|o| o.status == "unsat" || o.status == "sat") {
                    let theirs = linear_bar_pct(o.ms, scale);
                    // Faster ⇒ shorter bar.
                    assert!(
                        ours <= theirs,
                        "[{}] n={}: ours {}ms bar {ours} must not exceed {} {}ms bar {theirs}",
                        fam.id, r.n, r.ours_ms, o.solver, o.ms
                    );
                    // And proportional (both above the sliver floor): bar ratio == time ratio.
                    if ours > 0.5 && theirs > 0.5 {
                        let bar_ratio = theirs / ours;
                        let time_ratio = o.ms / r.ours_ms;
                        assert!(
                            (bar_ratio - time_ratio).abs() / time_ratio < 1e-6,
                            "[{}] n={}: bar ratio {bar_ratio} must equal time ratio {time_ratio}",
                            fam.id, r.n
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn codec_bars_are_proportional_to_bytes() {
        // The winning (smallest) LOGOS wire must draw a strictly shorter bar than a codec
        // it beats, by the exact byte ratio — never a log-flattened near-tie.
        let d = &*CODEC_DATA;
        for s in d.scenarios.iter().filter(|s| s.kind == "fair") {
            let vmax = s.rows.iter().map(|r| r.size as f64).fold(0.0, f64::max);
            let best = s.rows.iter().filter(|r| is_logos_codec(&r.codec)).map(|r| r.size).min();
            let json = s.rows.iter().find(|r| r.codec == "json").map(|r| r.size);
            if let (Some(best), Some(json)) = (best, json) {
                if best < json {
                    let pb = linear_bar_pct(best as f64, vmax);
                    let pj = linear_bar_pct(json as f64, vmax);
                    assert!(pb < pj, "[{}] logos {best}B bar must be shorter than json {json}B", s.id);
                    assert!(
                        (pj / pb - json as f64 / best as f64).abs() < 1e-6,
                        "[{}] codec bar ratio must equal byte ratio", s.id
                    );
                }
            }
        }
    }

    #[test]
    fn solver_data_loads_and_we_win_where_claimed() {
        let d = &*SOLVER_DATA; // panics if the baked solvers.json is malformed
        assert!(d.families.len() >= 3, "solver section must ship multiple families");
        // The pigeonhole headline must carry measured Z3 AND Kissat timeouts — the resolution wall.
        let php = d.families.iter().find(|f| f.id == "php").expect("php family present");
        let walls = php.rows.iter().any(|r| {
            r.others.iter().any(|o| o.solver == "z3" && o.status == "timeout")
                && r.others.iter().any(|o| o.solver == "kissat" && o.status == "timeout")
        });
        assert!(walls, "PHP must show Z3 AND Kissat hitting the resolution wall");
        // On every family we present as a WIN, ours must never be slower than a competitor that
        // COMPLETED (a regression like the dropped clique family — where a solver beat us — trips
        // this immediately, so the page can never silently claim a loss as a win).
        for fam in d.families.iter().filter(|f| {
            f.id == "php"
                || f.id == "mutilated_chessboard"
                || f.id == "tseitin"
                || f.id.starts_with("mod_")
                || f.id == "clique_coloring"
                || f.id == "parity_cardinality"
                || f.id == "count_q_mod3"
                || f.id == "ordering_gt"
                || f.id == "grid_tseitin"
        }) {
            for r in &fam.rows {
                for o in &r.others {
                    if o.status == "unsat" || o.status == "sat" {
                        assert!(
                            r.ours_ms < o.ms,
                            "[{}] n={}: claimed a win but ours {}ms >= {} {}ms",
                            fam.id,
                            r.n,
                            r.ours_ms,
                            o.solver,
                            o.ms
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn fmt_solver_ms_is_readable_and_never_zero() {
        // No measured time — however tiny — may render as a zero magnitude or lose its unit.
        let zeros = ["0", "0 µs", "0.0 µs", "0.00 µs", "0.0 ms", "0.00 ms", "0.000 ms"];
        let mut ms = 5_000.0_f64;
        while ms > 1e-7 {
            let s = fmt_solver_ms(ms);
            assert!(s.contains("µs") || s.contains("ms") || s.contains(" s"), "no unit for {ms}: {s}");
            assert!(!zeros.contains(&s.as_str()), "ms={ms} rendered as zero: {s}");
            ms /= 1.7; // sweep every decade down to sub-nanosecond
        }
        // Spot checks across the unit boundaries.
        assert_eq!(fmt_solver_ms(0.002), "2.0 µs");
        assert_eq!(fmt_solver_ms(0.17), "170 µs");
        assert_eq!(fmt_solver_ms(0.999), "999 µs");
        assert_eq!(fmt_solver_ms(9.79), "9.8 ms");
        assert_eq!(fmt_solver_ms(504.0), "504 ms");
        assert_eq!(fmt_solver_ms(10_009.8), "10.0 s");
        assert_eq!(fmt_solver_ms(0.0000004), "<0.01 µs");
        // Every "ours" time in the shipped data renders cleanly.
        for fam in &SOLVER_DATA.families {
            for r in &fam.rows {
                let s = fmt_solver_ms(r.ours_ms);
                assert!(!zeros.contains(&s.as_str()), "[{}] n={} ours renders as zero: {s}", fam.id, r.n);
            }
        }
    }

    #[test]
    fn benchmark_data_deserializes_without_memory_or_complexity() {
        // Old result files lack `memory`/`complexity`; the page must still load
        // (BENCH_DATA is `.unwrap()`ed, so a missing field would panic the page).
        let json = r#"{"id":"x","name":"X","description":"d","reference_size":"1",
            "sizes":["1"],"logos_source":"","generated_rust":"","scaling":{},
            "compilation":{}}"#;
        let b: Benchmark = serde_json::from_str(json).expect("deserialize without memory/complexity");
        assert!(b.memory.is_none() && b.complexity.is_none() && b.binary_sizes.is_none());
    }

    #[test]
    fn interp_schema_backward_and_forward_compatible() {
        // A complete TimingResult (mean..runs are required; user/system default).
        let t = |ms: f64| {
            format!(
                r#"{{"mean_ms":{ms},"median_ms":{ms},"stddev_ms":0.0,"min_ms":{ms},"max_ms":{ms},"cv":0.0,"runs":10}}"#
            )
        };
        // OLD JSON (pre-tiering): only `logos_interp` + `node`, no tiered geomean.
        // Must still load (INTERP_DATA is `.unwrap()`ed → a schema break panics the page).
        let old = format!(
            r#"{{"metadata":{{"node":"v22","date":"x"}},
            "benchmarks":[{{"id":"fib","name":"Fib","reference_size":"30",
                "scaling":{{"30":{{"logos_interp":{li},"node":{nd}}}}}}}],
            "summary":{{"geometric_mean_logos_interp_over_node":1.09}},"startup":{{}}}}"#,
            li = t(2.0),
            nd = t(1.0)
        );
        let d: InterpData = serde_json::from_str(&old).expect("old interp JSON must still load");
        assert_eq!(d.summary.geometric_mean_logos_tiered_over_node, 0.0, "missing tiered geo defaults to 0");
        assert!(d.interpreter_sizes.is_none(), "old interp JSON has no interpreter_sizes");

        // NEW JSON: adds the `logos_tiered` engine row + the tiered geomean — both the
        // new summary field AND the new engine in `scaling` deserialize.
        let new = format!(
            r#"{{"metadata":{{"node":"v22","date":"x"}},
            "benchmarks":[{{"id":"fib","name":"Fib","reference_size":"30",
                "scaling":{{"30":{{"logos_interp":{li},"logos_tiered":{lt},"node":{nd}}}}}}}],
            "summary":{{"geometric_mean_logos_interp_over_node":1.09,"geometric_mean_logos_tiered_over_node":0.98}},
            "startup":{{}}}}"#,
            li = t(2.0),
            lt = t(1.5),
            nd = t(1.0)
        );
        let d: InterpData = serde_json::from_str(&new).expect("new interp JSON must load");
        assert!((d.summary.geometric_mean_logos_tiered_over_node - 0.98).abs() < 1e-9);
        assert_eq!(d.summary.geometric_mean_logos_aot_over_node, 0.0, "missing AOT geo defaults to 0");
        let scaling = &d.benchmarks[0].scaling["30"];
        assert!(scaling.contains_key("logos_tiered"), "tiered engine row present");
        assert!((scaling["logos_tiered"].mean_ms - 1.5).abs() < 1e-9);

        // BUNDLED JSON: adds the `logos_aot` engine row + the AOT geomean (HOTSWAP
        // §Axis-3) — present only when a native bundle was built for the run.
        let bundled = format!(
            r#"{{"metadata":{{"node":"v22","date":"x"}},
            "benchmarks":[{{"id":"fib","name":"Fib","reference_size":"30",
                "scaling":{{"30":{{"logos_interp":{li},"logos_tiered":{lt},"logos_aot":{la},"node":{nd}}}}}}}],
            "summary":{{"geometric_mean_logos_interp_over_node":1.09,"geometric_mean_logos_tiered_over_node":0.98,"geometric_mean_logos_aot_over_node":0.42}},
            "startup":{{}}}}"#,
            li = t(2.0),
            lt = t(1.5),
            la = t(0.6),
            nd = t(1.0)
        );
        let d: InterpData = serde_json::from_str(&bundled).expect("bundled interp JSON must load");
        assert!((d.summary.geometric_mean_logos_aot_over_node - 0.42).abs() < 1e-9);
        let scaling = &d.benchmarks[0].scaling["30"];
        assert!(scaling.contains_key("logos_aot"), "AOT engine row present when bundled");
        assert!((scaling["logos_aot"].mean_ms - 0.6).abs() < 1e-9);
    }

    // ── Footprint metrics: the size data the benchmarks page now surfaces. These
    // assert against the REAL baked-in result files, so they go RED until a
    // `benchmarks/measure-sizes.sh --merge` backfills the JSON, then stay GREEN and
    // guard the contract on every future run.

    #[test]
    fn every_benchmark_carries_binary_sizes() {
        for b in &BENCH_DATA.benchmarks {
            let sizes = b.binary_sizes.as_ref()
                .unwrap_or_else(|| panic!("benchmark {} is missing binary_sizes", b.id));
            assert!(!sizes.by_language.is_empty(), "benchmark {} has empty binary_sizes", b.id);
            assert!(
                sizes.by_language.contains_key("logos_release"),
                "benchmark {} binary_sizes is missing the logos_release artifact", b.id
            );
            for (lang, s) in &sizes.by_language {
                assert!(s.as_built > 0.0, "{}/{} as_built must be > 0, got {}", b.id, lang, s.as_built);
                if let Some(st) = s.stripped {
                    assert!(
                        st > 0.0 && st <= s.as_built,
                        "{}/{} stripped ({st}) must be in (0, as_built={}]", b.id, lang, s.as_built
                    );
                }
            }
        }
    }

    #[test]
    fn interpreter_sizes_cover_logos_and_node() {
        let sizes = INTERP_DATA.interpreter_sizes.as_ref()
            .expect("latest-interp.json is missing interpreter_sizes");
        for id in ["logos", "node"] {
            let e = sizes.engines.get(id)
                .unwrap_or_else(|| panic!("interpreter_sizes is missing the {id} engine"));
            assert!(e.as_built > 0.0, "engine {id} as_built must be > 0, got {}", e.as_built);
            if let Some(st) = e.stripped {
                assert!(st > 0.0 && st <= e.as_built, "engine {id} stripped ({st}) out of range");
            }
        }
        if let Some(w) = sizes.wasm_bundle_bytes {
            assert!(w > 0.0, "wasm_bundle_bytes must be > 0 when present, got {w}");
        }
    }

    #[test]
    fn codec_data_deserializes_and_has_fair_scenarios() {
        let d = &*CODEC_DATA;
        assert_eq!(d.schema_version, 2, "schema_version is the page's compatibility gate");
        let fair: Vec<_> = d.scenarios.iter().filter(|s| s.kind == "fair").collect();
        assert!(!fair.is_empty(), "latest-codec.json must contain fair scenarios");
        for s in &fair {
            assert!(
                s.rows.iter().any(|r| r.codec == "logos (BEST: all knobs)"),
                "fair scenario '{}' is missing the BEST row",
                s.id
            );
        }
        assert!(
            d.scenarios.iter().any(|s| s.kind == "random_access"),
            "latest-codec.json must include the random-access scenario"
        );
    }

    #[test]
    fn codec_headline_is_provable() {
        // The displayed headline ("smallest wire on N/N fair workloads") must be backed by the
        // committed data: BEST ≤ every competitor on every fair workload.
        let (wins, total) = codec_size_wins(&CODEC_DATA);
        assert!(total > 0, "expected fair workloads in latest-codec.json");
        assert_eq!(wins, total, "the baked data must back the smallest-wire-on-all-workloads headline");
    }

    #[test]
    fn codec_scorecard_fair_axes_all_win() {
        // The scorecard claims "fastest encode N/N" and "fastest decode N/N" — the baked data
        // must back both (our fastest dial ≤ every competitor on every fair workload).
        let (enc, total) = codec_axis_wins(&CODEC_DATA, CodecAxis::Encode);
        let (dec, _) = codec_axis_wins(&CODEC_DATA, CodecAxis::Decode);
        assert!(total > 0);
        assert_eq!(enc, total, "scorecard claims fastest encode on all fair workloads");
        assert_eq!(dec, total, "scorecard claims fastest decode on all fair workloads");
        // The geo-mean size advantage and top-showcase ratio are real wins (> 1×).
        assert!(codec_geomean_size(&CODEC_DATA) > 1.0, "geo-mean size advantage must exceed 1×");
        assert!(codec_top_showcase(&CODEC_DATA).is_some_and(|r| r > 10.0), "a showcase must crush by >10×");
    }

    #[test]
    fn format_bytes_scales_units() {
        assert_eq!(format_bytes(16_120.0), "16 KB");
        assert_eq!(format_bytes(2.0 * 1024.0 * 1024.0), "2.0 MB");
        assert_eq!(format_bytes(3.0 * 1024.0 * 1024.0 * 1024.0), "3.0 GB");
    }

    #[test]
    fn footprint_label_only_tags_a_distinct_stripped_size() {
        // Clearly different (Rust ~2 MB → ~300 KB stripped): the tag is shown.
        assert_eq!(footprint_label(2_033_384.0, Some(307_280.0)), "1.9 MB (300 KB stripped)");
        // Already-minimal binary (334592 vs 334584 both render "327 KB"): no redundant tag.
        assert_eq!(footprint_label(334_592.0, Some(334_584.0)), "327 KB");
        // No stripped figure (java bytecode): just the as-built size.
        assert_eq!(footprint_label(583.0, None), "1 KB");
        // Defensive: a stripped value not smaller than as-built is ignored.
        assert_eq!(footprint_label(1000.0, Some(2000.0)), format_bytes(1000.0));
    }

    #[test]
    fn shipped_footprint_prefers_the_deployable_stripped_size() {
        // Rust ships stripped (300 KB) though it builds to ~2 MB with a symbol table — the
        // bar must use the 300 KB you actually deploy, with the as-built size alongside.
        assert_eq!(shipped_bytes(2_033_384.0, Some(307_280.0)), 307_280.0);
        assert_eq!(shipped_footprint_label(2_033_384.0, Some(307_280.0)), "300 KB (1.9 MB as-built)");
        // LOGOS already ships stripped (as-built ≈ stripped): the as-built figure is ALWAYS
        // shown for parity with every other row — the two numbers being equal is the story.
        assert_eq!(shipped_bytes(334_592.0, Some(334_584.0)), 334_584.0);
        assert_eq!(shipped_footprint_label(334_592.0, Some(334_584.0)), "327 KB (327 KB as-built)");
        // No stripped figure (a JVM class directory): shipped == as-built, shown consistently.
        assert_eq!(shipped_bytes(583.0, None), 583.0);
        assert_eq!(shipped_footprint_label(583.0, None), "1 KB (1 KB as-built)");
        // Defensive: a bogus stripped ≥ as-built is ignored (never inflate the shipped size).
        assert_eq!(shipped_bytes(1000.0, Some(2000.0)), 1000.0);
    }

    #[test]
    fn every_language_bar_uses_a_stripped_or_fallback_shipped_size() {
        // Across every benchmark, the value that drives each bar must be the shipped size:
        // the stripped binary where we have one, else the as-built artifact — never a mix
        // that would pit an already-stripped LOGOS against unstripped competitors.
        for b in BENCH_DATA.benchmarks.iter() {
            let Some(sizes) = b.binary_sizes.as_ref() else { continue };
            for (lang, s) in sizes.by_language.iter() {
                let shipped = shipped_bytes(s.as_built, s.stripped);
                assert!(shipped > 0.0 && shipped <= s.as_built,
                    "{}/{lang}: shipped {shipped} must be in (0, as_built={}]", b.id, s.as_built);
                if let Some(st) = s.stripped {
                    if st > 0.0 && st <= s.as_built {
                        assert_eq!(shipped, st, "{}/{lang}: must ship the stripped size", b.id);
                    }
                }
            }
        }
    }

    #[test]
    fn every_codec_scenario_has_a_good_for_line() {
        // Every example the harness emits must read as a use case — codec_good_for covers every
        // scenario id, so a new scenario can't ship without its "what it's good for" copy.
        for s in CODEC_DATA.scenarios.iter() {
            assert!(codec_good_for(&s.id).is_some(), "scenario '{}' has no codec_good_for copy", s.id);
        }
    }

    #[test]
    fn every_fair_best_row_names_the_dials_that_won() {
        // The winner card is driven by the BEST row's `chosen`; every fair example must carry it
        // with a non-empty summary + columns, or the card would fall back to opaque "all knobs".
        for s in CODEC_DATA.scenarios.iter().filter(|s| s.kind == "fair") {
            let best = codec_best_row(s).unwrap_or_else(|| panic!("fair '{}' missing the BEST row", s.id));
            let chosen = best.chosen.as_ref().unwrap_or_else(|| panic!("fair '{}' BEST row missing chosen", s.id));
            assert!(!chosen.summary.trim().is_empty(), "fair '{}' chosen.summary is empty", s.id);
            assert!(!chosen.columns.is_empty(), "fair '{}' chosen.columns is empty", s.id);
        }
    }

    #[test]
    fn every_fair_scenario_has_a_fair_fight() {
        // The honest compressed-vs-compressed comparison must be computable on every fair workload —
        // i.e. the competitors carry a `fair_size`, so the scorecard's "fair fight" count is real.
        for s in CODEC_DATA.scenarios.iter().filter(|s| s.kind == "fair") {
            let (lo, co, _) = codec_fair_fight(s)
                .unwrap_or_else(|| panic!("fair '{}' has no fair-fight (competitors missing fair_size)", s.id));
            assert!(lo > 0 && co > 0, "fair '{}' fair-fight sizes must be positive", s.id);
        }
        // And the scorecard's fair-fight count is honest: LOGOS does not have to win all of them.
        let (wins, total) = codec_fair_size_wins(&CODEC_DATA);
        assert!(total > 0 && wins <= total, "fair-fight count must be a valid n/{total}");
    }

    #[test]
    fn sources_cover_every_benchmark() {
        // Every benchmark in latest.json must resolve its 11-language sources BY ID — the
        // lookup the page renders — so a regenerated latest.json can never silently
        // desynchronize from the program sources the way positional indexing could.
        for b in BENCH_DATA.benchmarks.iter() {
            let s = bench_sources_for(&b.id)
                .unwrap_or_else(|| panic!("benchmark '{}' has no sources entry", b.id));
            for (lang, src) in [
                ("c", s.c), ("cpp", s.cpp), ("rust", s.rust), ("zig", s.zig), ("go", s.go),
                ("java", s.java), ("js", s.js), ("python", s.python), ("ruby", s.ruby),
                ("nim", s.nim), ("logos", s.logos),
            ] {
                assert!(!src.trim().is_empty(), "benchmark '{}' source '{lang}' is empty", b.id);
            }
        }
    }
}

// The page's data feeds. Native builds (tests, SSG prerender) compile the repo's
// benchmark results straight in; the wasm build fetches the identical bytes from
// `/data/*` (staged by scripts/stage-web-data.sh) so a megabyte of JSON never rides
// inside the shipped binary. Both sides answer through the same accessors below.
#[cfg(not(target_arch = "wasm32"))]
static BENCH_DATA: LazyLock<BenchmarkData> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../../benchmarks/results/latest.json")).unwrap()
});
#[cfg(target_arch = "wasm32")]
static BENCH_DATA: OnceLock<BenchmarkData> = OnceLock::new();

/// The language benchmark feed. `None` only on wasm before [`load_bench_bundle`]
/// resolves — the `Benchmarks` gate renders the loaded page strictly after that.
fn bench_data() -> Option<&'static BenchmarkData> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(&BENCH_DATA)
    }
    #[cfg(target_arch = "wasm32")]
    {
        BENCH_DATA.get()
    }
}

// Our certified prover vs the field — Z3 (SMT), Kissat (the CDCL world champion) and SaDiCaL (the
// reference PR/SDCL solver) — on structured-UNSAT families. Produced by benchmarks/run-solver-vs-z3.sh
// (native; external solvers run as subprocesses on the byte-identical DIMACS, emitting a clausal
// proof). Our verdicts are pure-Rust and browser-identical; the whole crate is Z3-free in WASM.
#[derive(Deserialize)]
struct SolverData {
    #[serde(default)]
    metadata: SolverMeta,
    families: Vec<SolverFamily>,
}

#[derive(Deserialize, Default)]
struct SolverMeta {
    #[serde(default)]
    date: String,
    #[serde(default)]
    cpu: String,
    #[serde(default)]
    kissat: String,
    #[serde(default)]
    sadical: String,
    #[serde(default)]
    cadical: String,
    #[serde(default)]
    cryptominisat: String,
}

#[derive(Deserialize)]
struct SolverFamily {
    id: String,
    name: String,
    /// How our prover wins (SR proof / matching / GF(2) Gaussian / plain CDCL).
    mechanism: String,
    /// The honest one-line separation framing.
    separation: String,
    /// Longer caption (e.g. the SDCL auto-discovery callout for PHP).
    note: String,
    rows: Vec<SolverRow>,
}

#[derive(Deserialize)]
struct SolverRow {
    n: u32,
    ours_ms: f64,
    #[serde(default)]
    ours_min_ms: f64,
    ours_detail: String,
    /// Size in bytes of OUR certified proof where one exists (SR proof / compact GF-ring certificate),
    /// so the "ours" bar shows a proof size just like the competitors — the answer to "why doesn't
    /// Logos show its proof size?". `None` for a structural witness (chessboard, pigeonhole variants).
    #[serde(default)]
    ours_proof_bytes: Option<u64>,
    /// Short format label for that proof (`SR`, `GF(2) cert`, `ℤ/6 cert`, …), shown beside the size.
    #[serde(default)]
    ours_proof_fmt: Option<String>,
    /// Competing solvers on the SAME instance (z3 / kissat / sadical / cadical / cryptominisat), in
    /// display order.
    #[serde(default)]
    others: Vec<OtherResult>,
}

/// One competitor's result on an instance. `status`: "unsat" (solved it), "sat", "timeout" (hit the
/// wall), or "error"/"absent". `proof_bytes` is the emitted clausal-proof size where it completed.
#[derive(Deserialize)]
struct OtherResult {
    solver: String,
    status: String,
    ms: f64,
    #[serde(default)]
    proof_bytes: Option<u64>,
}

#[cfg(not(target_arch = "wasm32"))]
static SOLVER_DATA: LazyLock<SolverData> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../../benchmarks/results/solvers.json")).unwrap()
});
#[cfg(target_arch = "wasm32")]
static SOLVER_DATA: OnceLock<SolverData> = OnceLock::new();

/// The solver head-to-head feed — same gating contract as [`bench_data`].
fn solver_data() -> Option<&'static SolverData> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(&SOLVER_DATA)
    }
    #[cfg(target_arch = "wasm32")]
    {
        SOLVER_DATA.get()
    }
}

/// A duration label that stays readable across the whole range and NEVER collapses to a misleading
/// "0 ms", no matter how tiny: sub-millisecond times render in microseconds with enough precision to
/// always show a non-zero figure (and `<0.01 µs` for the physically-impossible deep-sub-µs case),
/// the 1 ms…10 s band in milliseconds, and anything above in seconds.
fn fmt_solver_ms(ms: f64) -> String {
    if !(ms > 0.0) {
        return "0".to_string();
    }
    if ms < 1.0 {
        let us = ms * 1000.0;
        if us < 0.01 {
            "<0.01 µs".to_string()
        } else if us < 1.0 {
            format!("{us:.2} µs")
        } else if us < 10.0 {
            format!("{us:.1} µs")
        } else {
            format!("{us:.0} µs")
        }
    } else if ms < 100.0 {
        format!("{ms:.1} ms")
    } else if ms < 10_000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
}

/// Honest bar width: linear and proportional, so a bar's length is its value as a
/// fraction of the chart's largest value. A value 1000× smaller than the max is a
/// 1000× shorter bar — the true sliver (kept a hair visible by the track's min-width),
/// never a log-compressed stub that flatters the loser or shrinks the winner's lead.
/// The exact number always rides alongside the bar, so a sliver never hides its size.
fn linear_bar_pct(value: f64, max: f64) -> f64 {
    if max <= 0.0 {
        return 0.0;
    }
    (value / max * 100.0).clamp(0.0, 100.0)
}

/// Bar colour per competitor (ours is cyan; each solver its own hue; timeout is red, handled inline).
fn solver_color(name: &str) -> &'static str {
    match name {
        "z3" => "#a78bfa",
        "kissat" => "#fb923c",
        "sadical" => "#9ca3af",
        "cadical" => "#f472b6",
        "cryptominisat" => "#34d399",
        _ => "#6b7280",
    }
}

/// Display name for a competitor id.
fn solver_label(name: &str) -> String {
    match name {
        "z3" => "Z3".to_string(),
        "kissat" => "Kissat".to_string(),
        "sadical" => "SaDiCaL".to_string(),
        "cadical" => "CaDiCaL".to_string(),
        "cryptominisat" => "CryptoMiniSat".to_string(),
        other => other.to_string(),
    }
}

/// One labelled bar (ours or a competitor), reusing the page's `bench-bar-*` styling with the
/// inside/outside label rule the runtime charts use.
fn solver_bar(label: &str, pct: f64, color: &str, time: &str, detail: &str) -> Element {
    let show_inside = pct > 34.0;
    rsx! {
        div { class: "bench-bar-row",
            div { class: "bench-bar-label", "{label}" }
            div { class: "bench-bar-track",
                div {
                    class: "bench-bar-fill",
                    style: "width: {pct:.1}%; background: {color};",
                    title: "{detail}",
                    if show_inside {
                        span { class: "bench-bar-time", "{time}" }
                    }
                }
            }
            if !show_inside {
                span { class: "bench-bar-time-outside", "{time}" }
            }
        }
    }
}

/// The "Our solver vs the field" section: per-family bar charts (ours vs Z3, Kissat, SaDiCaL)
/// over the families where structure beats brute force, led by the pigeonhole wall. Bars are
/// linear per row — the slowest fills the track, ours is its true fraction — so a microsecond
/// refutation reads as the sliver it is. All numbers are measured by
/// `benchmarks/run-solver-vs-z3.sh`; the framing is the honest one the data supports — SaDiCaL
/// completes (we beat it), Z3 and Kissat hit the resolution wall.
#[component]
pub fn SolverSection() -> Element {
    let data = solver_data().expect("rendered only inside the loaded Benchmarks page");
    let meta = &data.metadata;
    // Headline figures, derived from the measured PHP data so the prose can never drift from it.
    let php = data.families.iter().find(|f| f.id == "php");
    let head = php.and_then(|f| {
        let r0 = f.rows.first()?;
        let walled: Vec<String> = r0
            .others
            .iter()
            .filter(|o| o.status == "timeout")
            .map(|o| solver_label(&o.solver))
            .collect();
        let sad_max = f
            .rows
            .iter()
            .filter(|r| r.ours_ms > 0.0)
            .filter_map(|r| {
                r.others
                    .iter()
                    .find(|o| o.solver == "sadical" && o.status == "unsat")
                    .map(|s| s.ms / r.ours_ms)
            })
            .fold(0.0_f64, f64::max);
        Some((r0.n, fmt_solver_ms(r0.ours_ms), walled, sad_max))
    });

    rsx! {
        div { class: "bench-section", id: "solver",
            div { class: "bench-section-title", "Our solver vs the field" }
            div { class: "bench-section-desc",
                "Raw speed is one axis; reasoning is another. On the classic structured-UNSAT families, our pure-Rust certified prover decides in milliseconds what brute-force search cannot — the same engine that runs in the Studio, in your browser, with no Z3. We measure it on a byte-identical formula against Z3 (SMT), Kissat (the CDCL world champion) and SaDiCaL (the reference PR/SDCL solver). Every \u{201c}ours\u{201d} verdict is a re-checked certificate, not a trusted answer."
            }

            if let Some((n, ours, walled, sad_max)) = head {
                div {
                    style: "background: linear-gradient(135deg, rgba(34,211,238,0.10), rgba(167,139,250,0.10)); border: 1px solid rgba(34,211,238,0.25); border-radius: 12px; padding: 16px 18px; margin-bottom: 22px;",
                    div { style: "font-size: 15px; font-weight: 700; color: #e5e7eb; margin-bottom: 4px;",
                        "A proof-system gap, not a tuning trick"
                    }
                    div { style: "font-size: 13px; color: rgba(229,231,235,0.7); line-height: 1.5;",
                        {
                            let walled_str = if walled.is_empty() { "Z3 and Kissat".to_string() } else { walled.join(" and ") };
                            rsx! {
                                "{walled_str} can't even finish PHP({n}) — they hit the resolution wall every CDCL solver inherits (Haken 1985, 2^\u{03a9}(n)). We certify it in {ours}"
                                if sad_max >= 1.5 {
                                    ", and beat SaDiCaL — the reference PR/SDCL solver, in our own proof class — by up to {sad_max:.0}\u{00d7}, with a kilobyte-scale certificate against its megabytes"
                                }
                                ". Our SDCL even "
                                em { "discovers" }
                                " that proof from the raw clauses with zero hints."
                            }
                        }
                    }
                }
            }

            for fam in data.families.iter() {
                div {
                    style: "margin-bottom: 26px;",
                    div { style: "font-size: 15px; font-weight: 700; color: #fff;", "{fam.name}" }
                    div { style: "font-size: 12px; color: rgba(34,211,238,0.85); margin: 2px 0 2px;", "ours: {fam.mechanism}" }
                    div { style: "font-size: 12px; color: rgba(229,231,235,0.55); margin-bottom: 12px;", "{fam.separation}" }
                    div { class: "bench-chart",
                        for row in fam.rows.iter() {
                            div { style: "margin-bottom: 12px;",
                                div { style: "font-size: 11px; color: rgba(229,231,235,0.5); font-family: ui-monospace, monospace; margin-bottom: 2px;", "n = {row.n}" }
                                {
                                    // Per-row honest scale: the slowest bar — a finite solve, or a
                                    // timeout's ceiling — fills the track, and every other bar is its
                                    // true fraction of it, so a microsecond refutation reads as the
                                    // sliver it is next to a multi-second competitor.
                                    let scale_max = row.others.iter().map(|o| o.ms).fold(row.ours_ms, f64::max);
                                    rsx! {
                                        div { class: "bench-chart",
                                            {
                                                // Our own bar now carries our proof size too — the SR proof or the
                                                // compact GF/ring certificate — beside the time, exactly the way the
                                                // competitors show theirs, with its format labelled so a linear-algebra
                                                // certificate is never mistaken for a clausal DRAT.
                                                let mut ours_lbl = fmt_solver_ms(row.ours_ms);
                                                if let Some(pb) = row.ours_proof_bytes {
                                                    let fmt = row.ours_proof_fmt.as_deref().unwrap_or("proof");
                                                    ours_lbl = format!("{ours_lbl} · {} {fmt}", format_bytes(pb as f64));
                                                }
                                                solver_bar("ours", linear_bar_pct(row.ours_ms, scale_max), "#22d3ee", &ours_lbl, &row.ours_detail)
                                            }
                                            for o in row.others.iter() {
                                                {
                                                    let (pct, color, lbl) = match o.status.as_str() {
                                                        "timeout" => (100.0_f64, "#ef4444", format!("timeout ≥{}", fmt_solver_ms(o.ms))),
                                                        "unsat" | "sat" => {
                                                            let mut l = fmt_solver_ms(o.ms);
                                                            if let Some(pb) = o.proof_bytes {
                                                                l = format!("{l} · {} proof", format_bytes(pb as f64));
                                                            }
                                                            (linear_bar_pct(o.ms, scale_max), solver_color(&o.solver), l)
                                                        }
                                                        other => (1.0_f64, "#475569", other.to_string()),
                                                    };
                                                    solver_bar(&solver_label(&o.solver), pct, color, &lbl, &format!("{} — solve + proof", solver_label(&o.solver)))
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { style: "font-size: 11.5px; color: rgba(229,231,235,0.5); line-height: 1.5; margin-top: 8px;", "{fam.note}" }
                }
            }

            div { style: "font-size: 11px; color: rgba(229,231,235,0.4); line-height: 1.5; border-top: 1px solid rgba(255,255,255,0.08); padding-top: 12px;",
                "Method: Kissat, SaDiCaL, CaDiCaL and CryptoMiniSat run as subprocesses on the byte-identical DIMACS, each emitting a clausal proof to disk whose size is shown (solve + certify, like ours); Z3 is the in-process oracle (no proof). Our own proof size is the size of our certified artifact — the SR proof for the pigeonhole class, the compact GF/ℤ-ring linear certificate for the algebraic families — measured by streaming it through a byte counter in memory, never written to disk and capped so it can never itself exhaust RAM. CryptoMiniSat runs in its default DRAT-emitting config; with its GF(2) Gaussian enabled it would decide parity fast but then could not emit a standard clausal proof — the certified gap. Ours is the median of 5 release runs (identical to the WASM build). Timeouts: Z3 10 s, Kissat/CaDiCaL/CryptoMiniSat 15 s, SaDiCaL 45 s (generous, so the PR solver runs to completion — no inflation). Sub-millisecond times are dominated by external process startup, so the headline claims rest on the cases where a competitor takes seconds or hits the wall. Random 3-SAT is the honesty control."
                if !meta.cpu.is_empty() {
                    span { " · Measured on {meta.cpu}" }
                }
                if !meta.kissat.is_empty() {
                    span { " · Kissat {meta.kissat}" }
                }
                if !meta.cadical.is_empty() {
                    span { " · CaDiCaL {meta.cadical}" }
                }
                if !meta.cryptominisat.is_empty() {
                    span { " · CryptoMiniSat {meta.cryptominisat}" }
                }
                if !meta.date.is_empty() {
                    span { " · {meta.date}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod solver_data_guard {
    use super::*;

    #[test]
    fn solver_data_parses_and_carries_our_proof_size() {
        // Guards the `include_str!` contract the page renders — and, specifically, that a regenerated
        // solvers.json keeps OUR proof size, the answer to "why doesn't Logos show its proof size?".
        let data = &*SOLVER_DATA;
        assert!(!data.families.is_empty(), "solvers.json must have families");

        // The pigeonhole family reports our SR proof size on every row.
        let php = data.families.iter().find(|f| f.id == "php").expect("php family present");
        assert!(
            php.rows.iter().all(|r| r.ours_proof_bytes.is_some() && r.ours_proof_fmt.is_some()),
            "every PHP row must carry our own SR proof size"
        );

        // Every structural family carries a proof, per its own certificate kind: GF/ring linear
        // certificate (parity/mod), the O(1) counting certificate (pigeonhole variants), the Hall
        // witness (chessboard). "Proofs for all", the answer to "shouldn't every family have one?".
        for id in ["tseitin", "mod_3_tseitin", "mod_6_tseitin", "php_variants", "mutilated_chessboard"] {
            let fam = data.families.iter().find(|f| f.id == id).unwrap_or_else(|| panic!("{id} present"));
            assert!(
                fam.rows.iter().all(|r| r.ours_proof_bytes.is_some()),
                "the {id} family must carry a proof size on every row"
            );
        }

        // The random control carries a plain-CDCL DRAT wherever it refutes (its UNSAT row); a SAT row
        // has a model, not a refutation, so no proof size there — and that is the only allowed gap.
        let random = data.families.iter().find(|f| f.id == "random_3sat").expect("random present");
        assert!(
            random.rows.iter().any(|r| r.ours_detail.contains("unsat") && r.ours_proof_bytes.is_some()),
            "the random control's UNSAT row must carry a DRAT proof size"
        );

        // CaDiCaL joined the field (CryptoMiniSat too when its binary is present at bench time).
        let solvers: std::collections::HashSet<&str> = data
            .families
            .iter()
            .flat_map(|f| f.rows.iter())
            .flat_map(|r| r.others.iter())
            .map(|o| o.solver.as_str())
            .collect();
        assert!(solvers.contains("cadical"), "CaDiCaL must appear in the field");
    }
}

// The LOGOS wire codec vs the industry serializers (Cap'n Proto, MessagePack, protobuf,
// bincode, postcard, CBOR, Arrow, JSON) — same logical data, same machine. Produced by
// the wirebench harness (crates/logicaffeine_wirebench) via benchmarks/run.sh Phase 7.
#[derive(Deserialize)]
struct CodecData {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    metadata: CodecMetadata,
    #[serde(default)]
    #[allow(dead_code)]
    iters: u32,
    #[serde(default)]
    scenarios: Vec<CodecScenario>,
}

#[derive(Deserialize, Default)]
struct CodecMetadata {
    #[serde(default)]
    date: String,
    #[serde(default)]
    #[allow(dead_code)]
    commit: String,
    #[serde(default)]
    #[allow(dead_code)]
    logos_version: String,
    #[serde(default)]
    cpu: String,
    #[serde(default)]
    os: String,
    #[serde(default)]
    versions: HashMap<String, String>,
    #[serde(default)]
    features: Vec<String>,
}

#[derive(Deserialize, Clone)]
struct CodecScenario {
    id: String,
    title: String,
    #[serde(default)]
    n: usize,
    kind: String,
    rows: Vec<CodecRow>,
}

#[derive(Deserialize, Clone)]
struct CodecRow {
    codec: String,
    size: usize,
    #[serde(default)]
    enc_ns: f64,
    #[serde(default)]
    dec_ns: f64,
    #[serde(default)]
    read_one_ns: Option<f64>,
    #[serde(default)]
    chosen: Option<ChosenConfig>,
    /// This codec's smallest size once granted the same compression LOGOS bakes in — set on the
    /// competitors, so the page can show a fair compressed-vs-compressed size fight. `None` on the
    /// LOGOS rows (the all-knobs winner's `size` is already its fair, compression-shopped size).
    #[serde(default)]
    fair_size: Option<usize>,
}

/// Which dials the all-knobs winner actually selected — surfaced so a card can say *what* won,
/// not just "all knobs". `columns` names the real per-column encoding the codec shipped (one bare
/// name for a single-column message, or one `"field: encoding"` per field for a record list);
/// `summary` is the line the card shows verbatim. Present only on the `logos (BEST: all knobs)` row.
#[derive(Deserialize, Clone)]
struct ChosenConfig {
    #[serde(default)]
    #[allow(dead_code)]
    numerics: String,
    #[serde(default)]
    #[allow(dead_code)]
    floats: String,
    #[serde(default)]
    #[allow(dead_code)]
    compression: String,
    #[serde(default)]
    #[allow(dead_code)]
    columns: Vec<String>,
    #[serde(default)]
    summary: String,
}

#[cfg(not(target_arch = "wasm32"))]
static CODEC_DATA: LazyLock<CodecData> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../../benchmarks/results/latest-codec.json")).unwrap()
});
#[cfg(target_arch = "wasm32")]
static CODEC_DATA: OnceLock<CodecData> = OnceLock::new();

/// The wire-codec head-to-head feed — same gating contract as [`bench_data`].
fn codec_data() -> Option<&'static CodecData> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(&CODEC_DATA)
    }
    #[cfg(target_arch = "wasm32")]
    {
        CODEC_DATA.get()
    }
}

/// Every LOGOS dial (varint / fixed / group-varint / affine / struct-view / all-knobs) is
/// rendered in the cyan family and highlighted; everything else is a competitor.
fn is_logos_codec(c: &str) -> bool {
    c.starts_with("logos")
}

fn codec_label(c: &str) -> &str {
    match c {
        "logos (BEST: all knobs)" => "LOGOS \u{2014} all knobs",
        "logos (varint)" => "LOGOS \u{2014} varint",
        "logos (fixed)" => "LOGOS \u{2014} fixed (memcpy)",
        "logos (gv/simd)" => "LOGOS \u{2014} group-varint",
        "logos (affine)" => "LOGOS \u{2014} affine",
        "logos (struct-view)" => "LOGOS \u{2014} struct-view",
        "logos (struct-view fixed)" => "LOGOS \u{2014} struct-view (fixed)",
        "logos (zero-copy)" => "LOGOS \u{2014} zero-copy",
        "bincode" => "bincode",
        "postcard" => "postcard",
        "messagepack" => "MessagePack",
        "cbor" => "CBOR",
        "json" => "JSON",
        "arrow (ipc)" => "Apache Arrow",
        "protobuf/grpc" => "Protobuf / gRPC",
        "capnproto" => "Cap'n Proto",
        other => other,
    }
}

fn codec_color(c: &str) -> &'static str {
    match c {
        "logos (BEST: all knobs)" => "#00d4ff",
        "logos (struct-view)" => "#00d4ff",
        "logos (struct-view fixed)" => "#26c6da",
        "logos (zero-copy)" => "#22d3ee",
        "logos (varint)" => "#0098c4",
        "logos (fixed)" => "#4dd0e1",
        "logos (gv/simd)" => "#26c6da",
        "logos (affine)" => "#80deea",
        "bincode" => "#dea584",
        "postcard" => "#f7a41d",
        "messagepack" => "#c0392b",
        "cbor" => "#8e44ad",
        "json" => "#f7df1e",
        "arrow (ipc)" => "#00ADD8",
        "protobuf/grpc" => "#4285f4",
        "capnproto" => "#e67e22",
        _ => "#94a3b8",
    }
}

/// A nanoseconds-per-op label that stays readable from a single-field read (~40 ns) to a
/// whole-message JSON decode (hundreds of µs).
fn format_ns(ns: f64) -> String {
    if ns < 1000.0 {
        format!("{ns:.0} ns")
    } else if ns < 1_000_000.0 {
        format!("{:.2} \u{00b5}s", ns / 1000.0)
    } else {
        format!("{:.2} ms", ns / 1_000_000.0)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum CodecAxis {
    Size,
    Encode,
    Decode,
    ReadOne,
}

/// One axis of a codec scenario as a sorted (winner-on-top) horizontal bar chart, reusing
/// the page's `bench-bar-*` styling. Rows missing the axis (e.g. no `read_one_ns`) are
/// dropped, so the same helper serves the size/encode/decode charts and the random-access one.
fn codec_axis_chart(s: &CodecScenario, axis: CodecAxis) -> Element {
    let val = |r: &CodecRow| -> Option<f64> {
        match axis {
            CodecAxis::Size => Some(r.size as f64),
            CodecAxis::Encode => Some(r.enc_ns),
            CodecAxis::Decode => Some(r.dec_ns),
            CodecAxis::ReadOne => r.read_one_ns,
        }
    };
    let fmt = |r: &CodecRow| -> String {
        match axis {
            CodecAxis::Size => format_bytes(r.size as f64),
            _ => format_ns(val(r).unwrap_or(0.0)),
        }
    };
    let mut rows: Vec<CodecRow> = s.rows.iter().filter(|r| val(r).is_some()).cloned().collect();
    rows.sort_by(|a, b| val(a).unwrap().partial_cmp(&val(b).unwrap()).unwrap_or(std::cmp::Ordering::Equal));
    if rows.is_empty() {
        return rsx! {};
    }
    // Sorted ascending, so the last row is the worst (largest) — it fills the track and
    // every other bar is its honest fraction of it.
    let vmax = val(rows.last().unwrap()).unwrap();
    rsx! {
        div { class: "bench-chart",
            for r in rows.iter() {
                {
                    let v = val(r).unwrap();
                    let pct = linear_bar_pct(v, vmax);
                    let s_str = fmt(r);
                    let fill_class = if is_logos_codec(&r.codec) { "bench-codec-fill logos-highlight" } else { "bench-codec-fill" };
                    let val_class = if is_logos_codec(&r.codec) { "bench-codec-val logos-val" } else { "bench-codec-val" };
                    let color = codec_color(&r.codec);
                    let label = codec_label(&r.codec);
                    rsx! {
                        div { class: "bench-bar-row",
                            div { class: "bench-bar-label", "{label}" }
                            div { class: "bench-codec-track",
                                div { class: "{fill_class}", style: "width: {pct:.1}%; background: {color};" }
                            }
                            div { class: "{val_class}", "{s_str}" }
                        }
                    }
                }
            }
        }
    }
}

/// The provable headline: the count of fair workloads where `logos (BEST: all knobs)` is no
/// larger than EVERY competitor (ties count), out of all fair workloads. True by construction
/// — the all-knobs row tries every dial and reports the minimum — and asserted against the
/// committed data by `codec_headline_is_provable`, so the page can never overclaim.
fn codec_size_wins(d: &CodecData) -> (usize, usize) {
    let fair: Vec<&CodecScenario> = d.scenarios.iter().filter(|s| s.kind == "fair").collect();
    let wins = fair
        .iter()
        .filter(|s| match s.rows.iter().find(|r| r.codec == "logos (BEST: all knobs)") {
            Some(best) => s.rows.iter().filter(|r| !is_logos_codec(&r.codec)).all(|r| best.size <= r.size),
            None => false,
        })
        .count();
    (wins, fair.len())
}

/// (smallest LOGOS bytes, smallest-competitor label, its bytes) for a scenario — the data
/// behind each "N× smaller than the best of the rest" caption.
fn codec_best_vs_competitor(s: &CodecScenario) -> Option<(usize, String, usize)> {
    let ours = s.rows.iter().filter(|r| is_logos_codec(&r.codec)).map(|r| r.size).min()?;
    let rival = s.rows.iter().filter(|r| !is_logos_codec(&r.codec)).min_by_key(|r| r.size)?;
    Some((ours, codec_label(&rival.codec).to_string(), rival.size))
}

/// (fastest LOGOS read-one ns, Cap'n Proto's read-one ns) for the random-access scenario.
fn codec_random_access_vs_capnp(s: &CodecScenario) -> Option<(f64, f64)> {
    let ours = s
        .rows
        .iter()
        .filter(|r| is_logos_codec(&r.codec))
        .filter_map(|r| r.read_one_ns)
        .fold(f64::INFINITY, f64::min);
    let capnp = s.rows.iter().find(|r| r.codec == "capnproto").and_then(|r| r.read_one_ns)?;
    if ours.is_finite() {
        Some((ours, capnp))
    } else {
        None
    }
}

/// The size caption for a scenario ("LOGOS …B vs best of the rest (…) …B — N× smaller").
fn codec_size_caption(s: &CodecScenario) -> Option<String> {
    codec_best_vs_competitor(s).map(|(ours, rival, rsize)| {
        format!(
            "LOGOS {} vs best of the rest ({rival}) {} — {:.2}× smaller",
            format_bytes(ours as f64),
            format_bytes(rsize as f64),
            rsize as f64 / ours.max(1) as f64
        )
    })
}

/// The random-access caption ("LOGOS …ns vs Cap'n Proto …ns — N× faster open + read").
fn codec_ra_caption(s: &CodecScenario) -> Option<String> {
    codec_random_access_vs_capnp(s).map(|(ours, capnp)| {
        format!(
            "LOGOS {} vs Cap'n Proto {} — {:.2}× faster open + read",
            format_ns(ours),
            format_ns(capnp),
            capnp / ours.max(1.0)
        )
    })
}

/// (#fair workloads where the fastest LOGOS dial is ≤ every competitor on `axis`, total fair) —
/// the scorecard's "fastest encode / decode on N/N" stats, computed from the baked data.
fn codec_axis_wins(d: &CodecData, axis: CodecAxis) -> (usize, usize) {
    let v = |r: &CodecRow| -> Option<f64> {
        match axis {
            CodecAxis::Size => Some(r.size as f64),
            CodecAxis::Encode => Some(r.enc_ns),
            CodecAxis::Decode => Some(r.dec_ns),
            CodecAxis::ReadOne => r.read_one_ns,
        }
    };
    let fair: Vec<&CodecScenario> = d.scenarios.iter().filter(|s| s.kind == "fair").collect();
    let wins = fair
        .iter()
        .filter(|s| {
            let lo = s.rows.iter().filter(|r| is_logos_codec(&r.codec)).filter_map(&v).fold(f64::INFINITY, f64::min);
            let co = s.rows.iter().filter(|r| !is_logos_codec(&r.codec)).filter_map(&v).fold(f64::INFINITY, f64::min);
            lo.is_finite() && co.is_finite() && lo <= co
        })
        .count();
    (wins, fair.len())
}

/// The biggest structural showcase win (best competitor ÷ smallest LOGOS, by size).
fn codec_top_showcase(d: &CodecData) -> Option<f64> {
    d.scenarios
        .iter()
        .filter(|s| s.kind == "showcase")
        .filter_map(|s| codec_best_vs_competitor(s).map(|(ours, _, rival)| rival as f64 / ours.max(1) as f64))
        .fold(None, |acc, r| Some(acc.map_or(r, |m: f64| m.max(r))))
}

/// Geometric-mean size advantage over the best competitor across the fair workloads (×).
fn codec_geomean_size(d: &CodecData) -> f64 {
    let ratios: Vec<f64> = d
        .scenarios
        .iter()
        .filter(|s| s.kind == "fair")
        .filter_map(|s| codec_best_vs_competitor(s).map(|(ours, _, rival)| rival as f64 / ours.max(1) as f64))
        .collect();
    if ratios.is_empty() {
        return 1.0;
    }
    (ratios.iter().map(|r| r.ln()).sum::<f64>() / ratios.len() as f64).exp()
}

/// The all-knobs winner row for a scenario (the size-optimal config, whose `chosen` names the
/// dials that won). `None` for a scenario that has no all-knobs row (e.g. random-access).
fn codec_best_row(s: &CodecScenario) -> Option<&CodecRow> {
    s.rows.iter().find(|r| r.codec == "logos (BEST: all knobs)")
}

/// A one-line, real-world "what this data shape is for" — so every example reads as a use case,
/// not just a bar chart. Covers every scenario id the harness emits (asserted by a test), so a
/// new scenario can't ship without its copy.
fn codec_good_for(id: &str) -> Option<&'static str> {
    Some(match id {
        "ints" => "ID columns, counters, quantities, foreign keys",
        "floats" => "measurements, prices, scientific readings",
        "timeseries" => "sensor / financial streams that vary slowly — xor-delta shines",
        "points" => "coordinates, vectors, spatial data",
        "records" => "row / struct lists — API payloads, database rows, events",
        "strings" => "text-heavy payloads — names, labels, log lines",
        "bools" => "flag columns, feature toggles, bitsets",
        "adv_negative" => "signed data that trips formats without zig-zag",
        "adv_repetitive" => "columns with long runs — status codes, repeated defaults",
        "adv_huge" => "wide 64-bit values — hashes, ids, nanosecond timestamps",
        "random_access" => "reading ONE field of a big message without decoding the rest",
        "affine_showcase" => "arithmetic progressions — row ids, ramps, fixed offsets",
        "poly_showcase" => "polynomial columns — accumulators, quadratic ramps",
        "categorical" => "low-cardinality labels — enums, tags, country codes",
        "int_set" => "membership sets — id sets, allow-lists",
        "int_map" => "int-keyed maps — lookup tables, adjacency lists",
        _ => return None,
    })
}

/// The other end of the size↔speed dial: the fastest-to-decode LOGOS dial that ISN'T the size
/// winner, when it decodes meaningfully faster (a real speed-for-size trade). This is what the
/// card surfaces as "prioritize speed?", answering "if I don't take the smallest, what do I get?".
fn codec_speed_pick(s: &CodecScenario) -> Option<(String, usize, f64)> {
    let best = codec_best_row(s)?;
    let pick = s
        .rows
        .iter()
        .filter(|r| is_logos_codec(&r.codec) && r.codec != "logos (BEST: all knobs)" && r.dec_ns > 0.0)
        .min_by(|a, b| a.dec_ns.partial_cmp(&b.dec_ns).unwrap_or(std::cmp::Ordering::Equal))?;
    (pick.dec_ns < best.dec_ns * 0.9).then(|| (codec_label(&pick.codec).to_string(), pick.size, pick.dec_ns))
}

/// The FAIR-FIGHT size comparison for a scenario: (LOGOS best-compressed size, best competitor's
/// fair-compressed size, its label). LOGOS bakes compression into the codec; here every competitor
/// is granted the SAME compression, so this is an honest compressed-vs-compressed comparison — not
/// LOGOS-compressed-vs-competitor-raw. `None` if no all-knobs winner or no competitor `fair_size`.
fn codec_fair_fight(s: &CodecScenario) -> Option<(usize, usize, String)> {
    let logos = codec_best_row(s)?.size; // the BEST row already shops compression
    let rival = s
        .rows
        .iter()
        .filter(|r| !is_logos_codec(&r.codec))
        .filter_map(|r| r.fair_size.map(|f| (f, r.codec.clone())))
        .min_by_key(|(f, _)| *f)?;
    Some((logos, rival.0, codec_label(&rival.1).to_string()))
}

/// (#fair workloads where LOGOS is no larger than every competitor IN THE FAIR FIGHT, total fair) —
/// the honest "even when everyone compresses" size count for the scorecard.
fn codec_fair_size_wins(d: &CodecData) -> (usize, usize) {
    let fair: Vec<&CodecScenario> = d.scenarios.iter().filter(|s| s.kind == "fair").collect();
    let wins = fair.iter().filter(|s| matches!(codec_fair_fight(s), Some((lo, co, _)) if lo <= co)).count();
    (wins, fair.len())
}

/// The winner card shown above each example's bar charts: what config won, its size AND encode
/// AND decode together (the trio the three separate charts never connect), what the shape is good
/// for, and the speed-pick alternative. `rsx!{}` (nothing) for a scenario with no all-knobs winner.
fn codec_winner_card(s: &CodecScenario) -> Element {
    let Some(best) = codec_best_row(s) else { return rsx! {} };
    let Some(chosen) = best.chosen.as_ref() else { return rsx! {} };
    let headline = match codec_best_vs_competitor(s) {
        Some((ours, rival, rsize)) => format!(
            "\u{2714} LOGOS ships {} \u{2014} {:.2}\u{00d7} smaller than {}",
            format_bytes(ours as f64),
            rsize as f64 / ours.max(1) as f64,
            rival
        ),
        None => format!("\u{2714} LOGOS ships {}", format_bytes(best.size as f64)),
    };
    let config_line = format!("won with: {}", chosen.summary);
    let speed_line = format!("encode {} \u{00b7} decode {}", format_ns(best.enc_ns), format_ns(best.dec_ns));
    let good = codec_good_for(&s.id);
    // The honest compressed-vs-compressed line: when the headline win used compression, this shows
    // whether LOGOS still wins (or ties) once every rival is given the same compression.
    let fair = codec_fair_fight(s).map(|(lo, co, rlabel)| {
        let verdict = if lo < co { "still smaller" } else if lo == co { "tie" } else { "rival smaller" };
        format!(
            "Fair fight (all compressed): LOGOS {} vs {} {} \u{2014} {}",
            format_bytes(lo as f64),
            rlabel,
            format_bytes(co as f64),
            verdict
        )
    });
    let speed_pick = codec_speed_pick(s).map(|(label, size, dec)| {
        format!("Prioritize speed? {} \u{2014} {} \u{00b7} decode {}", label, format_bytes(size as f64), format_ns(dec))
    });
    rsx! {
        div {
            style: "background: linear-gradient(135deg, rgba(34,211,238,0.10), rgba(167,139,250,0.08)); border: 1px solid rgba(34,211,238,0.28); border-radius: 12px; padding: 12px 14px; margin: 6px 0 12px;",
            div { style: "font-size: 13px; font-weight: 700; color: #67e8f9;", "{headline}" }
            div { style: "font-size: 12px; color: rgba(229,231,235,0.85); margin-top: 4px;", "{config_line}" }
            div { style: "font-size: 12px; color: rgba(229,231,235,0.6); margin-top: 2px;", "{speed_line}" }
            if let Some(g) = good {
                div { style: "font-size: 12px; color: rgba(167,139,250,0.9); margin-top: 4px;", "Good for: {g}" }
            }
            if let Some(f) = fair {
                div { style: "font-size: 11px; color: rgba(229,231,235,0.55); margin-top: 4px;", "{f}" }
            }
            if let Some(sp) = speed_pick {
                div { style: "font-size: 11px; color: rgba(229,231,235,0.5); margin-top: 5px; border-top: 1px solid rgba(255,255,255,0.06); padding-top: 5px;", "{sp}" }
            }
        }
    }
}

/// The dial glossary: every knob the codec exposes, in plain words, with what it's *for* — so the
/// section reads as a menu of trade-offs, not an unexplained pile of bars. The top group is the
/// dials these benchmarks actually exercise; the second is the rest of the codec's capability
/// surface, so the reader sees the whole instrument. Copy tracks the marshal.rs doc comments.
fn codec_dial_glossary() -> Element {
    let benched: &[(&str, &str)] = &[
        ("varint", "LEB128 — smallest bytes; the default for a bandwidth-bound network link"),
        ("fixed (memcpy)", "raw 8-byte i64 — fastest decode on a datacenter / RDMA link, ~4\u{00d7} the size"),
        ("group-varint", "varint-class size, SIMD-decoded several ints at once — small AND fast"),
        ("auto structure", "per-column bake-off (delta \u{00b7} delta-of-delta \u{00b7} FOR bit-pack \u{00b7} run-length \u{00b7} dictionary \u{00b7} affine \u{00b7} polynomial) — ships the smallest, never larger than varint"),
        ("xor-delta floats", "Gorilla-style — slowly-varying float streams shrink; high-entropy data falls back to memcpy"),
        ("struct-view", "an offset table per field so any ONE field reads in O(1) — Cap'n Proto's game; larger on the wire"),
        ("compression", "deflate / lz4 / zstd over the whole frame, kept only when smaller — for bulky or redundant payloads"),
    ];
    let also: &[(&str, &str)] = &[
        ("checksum", "an FNV tag so the receiver rejects a corrupted frame (Raw \u{2194} Checked)"),
        ("type-id", "elide struct / enum names on a Logos\u{2194}Logos link once both peers know the schema"),
        ("FEC", "Reed-Solomon shards — reconstruct the message from any K of N (lossy links, multicast)"),
        ("dedup", "a subtree reached by the same reference ships once + backrefs; the decoder rebuilds the sharing"),
    ];
    let item = |name: &'static str, desc: &'static str| {
        rsx! {
            div { style: "margin: 3px 0;",
                span { style: "color: #67e8f9; font-weight: 600;", "{name}" }
                span { style: "color: rgba(229,231,235,0.6);", " \u{2014} {desc}" }
            }
        }
    };
    rsx! {
        div {
            style: "background: rgba(255,255,255,0.02); border: 1px solid rgba(255,255,255,0.08); border-radius: 12px; padding: 14px 16px; margin: 6px 0 18px; font-size: 12px; line-height: 1.5;",
            div { style: "font-size: 13px; font-weight: 700; color: #fff; margin-bottom: 4px;", "The dials \u{2014} every form is self-describing, so the decoder needs no hint" }
            div { style: "color: rgba(229,231,235,0.5); margin-bottom: 6px;", "In these benchmarks:" }
            for (name, desc) in benched.iter().copied() { {item(name, desc)} }
            div { style: "color: rgba(229,231,235,0.5); margin: 10px 0 6px;", "Also available (not exercised by the charts below):" }
            for (name, desc) in also.iter().copied() { {item(name, desc)} }
        }
    }
}

/// The "Serialization — the wire codec" section: the LOGOS wire codec head-to-head against
/// the industry serializers across wire size, encode, decode and single-field random access.
/// Every figure is measured by the wirebench harness on the same logical data; the headline
/// ("smallest wire on N/N workloads") is provable, and the structured/affine showcase is
/// fenced off from the fair results so the math-hack is never conflated with general data.
#[component]
pub fn CodecSection() -> Element {
    let d = codec_data().expect("rendered only inside the loaded Benchmarks page");
    let (wins, total) = codec_size_wins(d);
    let fair: Vec<&CodecScenario> = d.scenarios.iter().filter(|s| s.kind == "fair").collect();
    let adversarial: Vec<&CodecScenario> = d.scenarios.iter().filter(|s| s.kind == "adversarial").collect();
    let random: Vec<&CodecScenario> = d.scenarios.iter().filter(|s| s.kind == "random_access").collect();
    let structural: Vec<&CodecScenario> = d.scenarios.iter().filter(|s| s.kind == "structural").collect();
    let showcase: Vec<&CodecScenario> = d.scenarios.iter().filter(|s| s.kind == "showcase").collect();

    let mut comp_names = vec!["bincode", "postcard", "MessagePack", "CBOR", "JSON"];
    if d.metadata.features.iter().any(|f| f == "arrow-bench") {
        comp_names.push("Apache Arrow");
    }
    if d.metadata.features.iter().any(|f| f == "protobuf") {
        comp_names.push("Protobuf/gRPC");
    }
    if d.metadata.features.iter().any(|f| f == "capnproto") {
        comp_names.push("Cap'n Proto");
    }
    let comp_list = comp_names.join(", ");

    let hint = "font-size: 12px; color: rgba(229,231,235,0.55); margin: 10px 0 4px;";
    let title = "font-size: 15px; font-weight: 700; color: #fff;";
    let cap = "font-size: 12px; color: rgba(34,211,238,0.85); margin: 2px 0 6px;";

    rsx! {
        div { class: "bench-section", id: "serialization",
            div { class: "bench-section-title", "Serialization \u{2014} the wire codec" }
            div { class: "bench-section-desc",
                "Speed of code is one axis; bytes on the wire are another. The LOGOS wire codec is benched head-to-head against {comp_list} \u{2014} the same logical data through every codec, on the same machine, with seeded-random payloads (never a tidy 0..n sequence that would hand a structural codec a free win). LOGOS wins encode AND decode on every fair workload and is competitive-to-smaller on size; the honest details are on each card."
            }

            // ---- Dial glossary: what each knob does and what it's for ----
            { codec_dial_glossary() }

            // ---- Scorecard: the whole head-to-head at a glance, computed from the baked data ----
            {
                let (enc_wins, _) = codec_axis_wins(d, CodecAxis::Encode);
                let (dec_wins, _) = codec_axis_wins(d, CodecAxis::Decode);
                let (fair_wins, fair_total) = codec_fair_size_wins(d);
                let geo = format!("{:.1}\u{00d7}", codec_geomean_size(d));
                let ra = random.first().and_then(|s| codec_random_access_vs_capnp(s));
                let ra_str = ra.map(|(o, c)| format!("{:.1}\u{00d7}", c / o.max(1.0)));
                let top_str = codec_top_showcase(d).map(|r| format!("{:.0}\u{00d7}", r));
                rsx! {
                    div { class: "bench-summary",
                        div { class: "bench-summary-card",
                            div { class: "bench-summary-value green", "{enc_wins}/{total}" }
                            div { class: "bench-summary-label", "fastest encode (fair workloads)" }
                        }
                        div { class: "bench-summary-card",
                            div { class: "bench-summary-value green", "{dec_wins}/{total}" }
                            div { class: "bench-summary-label", "fastest decode (fair workloads)" }
                        }
                        div { class: "bench-summary-card",
                            div { class: "bench-summary-value cyan", "{wins}/{total}" }
                            div { class: "bench-summary-label", "smallest out-of-the-box (we auto-compress; rivals raw)" }
                        }
                        div { class: "bench-summary-card",
                            div { class: "bench-summary-value cyan", "{fair_wins}/{fair_total}" }
                            div { class: "bench-summary-label", "smallest in a fair fight (everyone compressed)" }
                        }
                        if let Some(ra_str) = ra_str {
                            div { class: "bench-summary-card",
                                div { class: "bench-summary-value cyan", "{ra_str}" }
                                div { class: "bench-summary-label", "faster random-access read vs Cap'n Proto" }
                            }
                        }
                        if let Some(top_str) = top_str {
                            div { class: "bench-summary-card",
                                div { class: "bench-summary-value purple", "up to {top_str}" }
                                div { class: "bench-summary-label", "structural showcases (smaller)" }
                            }
                        }
                        div { class: "bench-summary-card",
                            div { class: "bench-summary-value cyan", "{geo}" }
                            div { class: "bench-summary-label", "geo-mean smaller, out of the box" }
                        }
                    }
                    div {
                        style: "background: linear-gradient(135deg, rgba(34,211,238,0.10), rgba(167,139,250,0.10)); border: 1px solid rgba(34,211,238,0.25); border-radius: 12px; padding: 16px 18px; margin: 4px 0 20px;",
                        div { style: "font-size: 13px; color: rgba(229,231,235,0.7); line-height: 1.5;",
                            "Every figure is measured on the same logical data, same machine, seeded-random. LOGOS wins encode AND decode on every fair workload \u{2014} the default dial is a pure memcpy that never searches at send time (you pick the shape ahead of time), so it beats even bincode's raw copy; decode is faster because the format is columnar and typed. On SIZE we\u{2019}re honest two ways: \u{201c}out of the box\u{201d} LOGOS auto-compresses and rivals ship raw (a real transport advantage), and the \u{201c}fair fight\u{201d} gives every rival the SAME compression \u{2014} there LOGOS wins on structured data and ties on random primitive arrays (a memcpy is a memcpy). The all-knobs winner\u{2019}s exact dials, and both size comparisons, are on each card. Decode is timed \u{201c}to usable\u{201d} (every value touched). The contrived generator showcases (a pure 0,1,2\u{2026} ramp) are fenced off from the fair results."
                        }
                    }
                }
            }

            // ---- Fair workloads: size / encode / decode ----
            for s in fair.iter() {
                div { style: "margin-bottom: 26px;",
                    div { style: "{title}", "{s.title}  \u{00b7}  n = {s.n}" }
                    { codec_winner_card(s) }
                    if let Some(c) = codec_size_caption(s) {
                        div { style: "{cap}", "{c}" }
                    }
                    div { style: "{hint}", "Wire size \u{2014} smaller is better, bars proportional" }
                    { codec_axis_chart(s, CodecAxis::Size) }
                    div { style: "{hint}", "Encode (ns/op) \u{2014} smaller is better, bars proportional" }
                    { codec_axis_chart(s, CodecAxis::Encode) }
                    div { style: "{hint}", "Decode to usable (ns/op) \u{2014} smaller is better, bars proportional" }
                    { codec_axis_chart(s, CodecAxis::Decode) }
                }
            }

            // ---- Random access: Cap'n Proto's home turf ----
            for s in random.iter() {
                div { style: "margin-bottom: 26px;",
                    div { style: "{title}", "{s.title}  \u{00b7}  n = {s.n}" }
                    div { style: "font-size: 12px; color: rgba(229,231,235,0.6); margin: 4px 0 6px; line-height: 1.5;",
                        "A message arrives; you open it and read ONE field. This is exactly what Cap'n Proto is built for. LOGOS opens its record-list view (row + field offset tables) in O(1); the self-describing codecs must decode the whole message to index it."
                    }
                    if let Some(c) = codec_ra_caption(s) {
                        div { style: "{cap}", "{c}" }
                    }
                    div { style: "{hint}", "Open + read one field (ns/op) \u{2014} smaller is better, bars proportional" }
                    { codec_axis_chart(s, CodecAxis::ReadOne) }
                }
            }

            // ---- Adversarial shapes ----
            if !adversarial.is_empty() {
                div { style: "font-size: 15px; font-weight: 700; color: #fff; margin-top: 28px;", "Adversarial shapes" }
                div { style: "font-size: 12px; color: rgba(229,231,235,0.55); margin: 4px 0 12px; line-height: 1.5;",
                    "The data shapes that wreck fixed-format codecs \u{2014} all dials on."
                }
                for s in adversarial.iter() {
                    div { style: "margin-bottom: 20px;",
                        div { style: "{title}", "{s.title}  \u{00b7}  n = {s.n}" }
                        { codec_winner_card(s) }
                        if let Some(c) = codec_size_caption(s) {
                            div { style: "{cap}", "{c}" }
                        }
                        div { style: "{hint}", "Wire size \u{2014} smaller is better, bars proportional" }
                        { codec_axis_chart(s, CodecAxis::Size) }
                    }
                }
            }

            // ---- Structural wins on REAL data: general columnar techniques, not contrived ----
            if !structural.is_empty() {
                div { style: "font-size: 15px; font-weight: 700; color: #fff; margin-top: 28px;", "Structural wins on real data" }
                div { style: "font-size: 12px; color: rgba(229,231,235,0.6); margin: 4px 0 12px; line-height: 1.5;",
                    "Not math tricks \u{2014} the standard columnar techniques (dictionary encoding for low-cardinality labels, bit-packing for booleans) that Parquet and Arrow are built on, applied automatically. Common shapes in real payloads, and LOGOS ships them by default when they win."
                }
                for s in structural.iter() {
                    div { style: "margin-bottom: 22px;",
                        div { style: "{title}", "{s.title}  \u{00b7}  n = {s.n}" }
                        { codec_winner_card(s) }
                        if let Some(c) = codec_size_caption(s) {
                            div { style: "{cap}", "{c}" }
                        }
                        div { style: "{hint}", "Wire size \u{2014} smaller is better, bars proportional" }
                        { codec_axis_chart(s, CodecAxis::Size) }
                    }
                }
            }

            // ---- Showcases, explicitly fenced off from the fair results ----
            if !showcase.is_empty() {
                div {
                    style: "background: linear-gradient(135deg, rgba(167,139,250,0.10), rgba(34,211,238,0.06)); border: 1px solid rgba(167,139,250,0.25); border-radius: 12px; padding: 14px 16px; margin: 18px 0 12px;",
                    div { style: "font-size: 13px; color: rgba(229,231,235,0.8); line-height: 1.5;",
                        "\u{26a1} Showcases, kept separate from the fair results: contrived columns where the codec ships the GENERATING RULE instead of the data \u{2014} an affine progression as (base, stride, n), a polynomial as its finite-difference seeds, a consecutive set as base+stride+count \u{2014} winning by orders of magnitude. Not general-data results; they show the ceiling when structure is perfect."
                    }
                }
                for s in showcase.iter() {
                    div { style: "margin-bottom: 22px;",
                        div { style: "{title}", "{s.title}  \u{00b7}  n = {s.n}" }
                        { codec_winner_card(s) }
                        if let Some(c) = codec_size_caption(s) {
                            div { style: "{cap}", "{c}" }
                        }
                        div { style: "{hint}", "Wire size \u{2014} smaller is better, bars proportional" }
                        { codec_axis_chart(s, CodecAxis::Size) }
                    }
                }
            }

            div { style: "font-size: 11px; color: rgba(229,231,235,0.4); line-height: 1.5; border-top: 1px solid rgba(255,255,255,0.08); padding-top: 12px;",
                "Method: each value space (int/float arrays, point & record lists, strings) is encoded by every codec from the same seeded-random data; size is exact bytes (no envelope), encode/decode are ns per whole-message op, taken with a warm-up. Cap'n Proto is read zero-copy (its best). Bars are linear within each chart — every length is its exact fraction of the slowest, so the gap you see is the true ratio. Measured on {d.metadata.cpu}, {d.metadata.os}."
            }
        }
    }
}

// The LOGOS interpreter (bytecode VM + JIT) vs Node/V8 — a separate peer-to-peer
// comparison at interpreter-calibrated sizes (so neither engine sits on V8's
// startup floor). Produced by benchmarks/run-interp-vs-js.sh.
#[derive(Deserialize)]
struct InterpData {
    #[serde(default)]
    metadata: InterpMetadata,
    #[serde(default)]
    benchmarks: Vec<InterpBenchmark>,
    #[serde(default)]
    summary: InterpSummary,
    #[serde(default)]
    startup: InterpStartup,
    /// Engine footprint (bytes): the largo VM+JIT binary vs the host language runtimes,
    /// plus the browser WASM bundle. Absent in older result files.
    #[serde(default)]
    interpreter_sizes: Option<InterpreterSizes>,
}

#[derive(Deserialize, Clone)]
struct InterpreterSizes {
    #[allow(dead_code)]
    method: String,
    /// engine id (logos, node, python, ruby, …) → on-disk size in bytes.
    engines: HashMap<String, BinSizes>,
    /// Largest `.wasm` in the release web bundle — the in-browser interpreter footprint.
    /// `null` when no `dx` release build was present at measurement time.
    #[serde(default)]
    wasm_bundle_bytes: Option<f64>,
}

#[derive(Deserialize, Default)]
struct InterpStartup {
    #[serde(default)]
    runs: u32,
    #[serde(default)]
    engines: HashMap<String, StartupTiming>,
}

#[derive(Deserialize, Default, Clone)]
struct StartupTiming {
    #[serde(default)]
    mean_ms: f64,
    #[serde(default)]
    min_ms: f64,
    #[serde(default)]
    median_ms: f64,
}

#[derive(Deserialize, Default)]
struct InterpMetadata {
    #[serde(default)]
    node: String,
    #[serde(default)]
    date: String,
}

#[derive(Deserialize, Default)]
struct InterpSummary {
    #[serde(default)]
    geometric_mean_logos_interp_over_node: f64,
    /// The TIERED engine's geomean (HOTSWAP §12) — the A/B counterpart to the eager
    /// `_interp_` figure. `#[serde(default)]` so the pre-tiered JSON still loads.
    #[serde(default)]
    geometric_mean_logos_tiered_over_node: f64,
    /// The AOT-NATIVE tier's geomean (HOTSWAP §Axis-3) — present only when a native
    /// bundle was built for the run. `#[serde(default)]` so non-bundled JSON still loads.
    #[serde(default)]
    geometric_mean_logos_aot_over_node: f64,
}

#[derive(Deserialize)]
struct InterpBenchmark {
    id: String,
    name: String,
    #[serde(default)]
    reference_size: String,
    #[serde(default)]
    interpreter_engine: String,
    #[serde(default)]
    scaling: HashMap<String, HashMap<String, TimingResult>>,
}

#[cfg(not(target_arch = "wasm32"))]
static INTERP_DATA: LazyLock<InterpData> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../../benchmarks/results/latest-interp.json")).unwrap()
});
#[cfg(target_arch = "wasm32")]
static INTERP_DATA: OnceLock<InterpData> = OnceLock::new();

/// The interpreter-tier feed — same gating contract as [`bench_data`].
fn interp_data() -> Option<&'static InterpData> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(&INTERP_DATA)
    }
    #[cfg(target_arch = "wasm32")]
    {
        INTERP_DATA.get()
    }
}

struct BenchSources {
    c: &'static str,
    cpp: &'static str,
    rust: &'static str,
    zig: &'static str,
    go: &'static str,
    java: &'static str,
    js: &'static str,
    python: &'static str,
    ruby: &'static str,
    nim: &'static str,
    logos: &'static str,
}

#[cfg(not(target_arch = "wasm32"))]
macro_rules! bench_sources {
    ($name:literal) => {
        ($name, BenchSources {
            c: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.c")),
            cpp: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.cpp")),
            rust: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.rs")),
            zig: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.zig")),
            go: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.go")),
            java: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/Main.java")),
            js: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.js")),
            python: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.py")),
            ruby: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.rb")),
            nim: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.nim")),
            logos: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.lg")),
        })
    };
}

#[cfg(not(target_arch = "wasm32"))]
static SOURCES: LazyLock<[(&'static str, BenchSources); 32]> = LazyLock::new(|| [
    // Recursion & Function Calls
    bench_sources!("fib"),
    bench_sources!("ackermann"),
    bench_sources!("nqueens"),
    // Sorting
    bench_sources!("bubble_sort"),
    bench_sources!("mergesort"),
    bench_sources!("quicksort"),
    bench_sources!("counting_sort"),
    bench_sources!("heap_sort"),
    // Floating Point
    bench_sources!("nbody"),
    bench_sources!("mandelbrot"),
    bench_sources!("spectral_norm"),
    bench_sources!("pi_leibniz"),
    // Integer Mathematics
    bench_sources!("gcd"),
    bench_sources!("collatz"),
    bench_sources!("primes"),
    // Array Patterns
    bench_sources!("sieve"),
    bench_sources!("matrix_mult"),
    bench_sources!("prefix_sum"),
    bench_sources!("array_reverse"),
    bench_sources!("array_fill"),
    // Hash Maps & Lookup
    bench_sources!("collect"),
    bench_sources!("two_sum"),
    bench_sources!("histogram"),
    // Dynamic Programming
    bench_sources!("knapsack"),
    bench_sources!("coins"),
    // Combinatorial
    bench_sources!("fannkuch"),
    // Memory & Allocation
    bench_sources!("strings"),
    bench_sources!("binary_trees"),
    // Loop Overhead & Control Flow
    bench_sources!("loop_sum"),
    bench_sources!("fib_iterative"),
    bench_sources!("graph_bfs"),
    bench_sources!("string_search"),
]);
#[cfg(target_arch = "wasm32")]
static SOURCES: OnceLock<HashMap<String, BenchSources>> = OnceLock::new();

/// The 11-language sources for one benchmark, looked up by its `latest.json` id —
/// same gating contract as [`bench_data`].
fn bench_sources_for(id: &str) -> Option<&'static BenchSources> {
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
struct BenchSourcesOwned {
    c: String,
    cpp: String,
    rust: String,
    zig: String,
    go: String,
    java: String,
    js: String,
    python: String,
    ruby: String,
    nim: String,
    logos: String,
}

#[cfg(target_arch = "wasm32")]
impl BenchSourcesOwned {
    fn pin(self) -> BenchSources {
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
async fn load_bench_bundle() -> Result<(), String> {
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

const GITHUB_REPO: &str = "https://github.com/Brahmastra-Labs/logicaffeine";

fn format_time(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{:.2}s", ms / 1000.0)
    } else if ms >= 1.0 {
        format!("{:.1}ms", ms)
    } else {
        format!("{:.0}\u{00b5}s", ms * 1000.0)
    }
}

fn format_timeout(timeout_ms: f64) -> String {
    let mins = timeout_ms / 60_000.0;
    if mins >= 1.0 {
        format!(">{:.0}min", mins)
    } else {
        format!(">{:.0}s", timeout_ms / 1000.0)
    }
}

fn tier_label(tier: &str) -> &'static str {
    match tier {
        "systems" => "Systems",
        "managed" => "Managed",
        "interpreted" => "Interpreted",
        "transpiled" => "Transpiled",
        _ => "Other",
    }
}

fn lang_color(lang_id: &str) -> &'static str {
    match lang_id {
        "c" => "#555555",
        "cpp" => "#f34b7d",
        "rust" => "#dea584",
        "zig" => "#f7a41d",
        "logos_release" => "#00d4ff",
        "go" => "#3fb950",
        "java" => "#b07219",
        "js" => "#f7df1e",
        "logos_interp" => "#ff8c00",
        "logos_tiered" => "#ffb84d",
        "logos_aot" => "#00d4ff",
        "python" => "#3776ab",
        "ruby" => "#cc342d",
        "nim" => "#ffe953",
        _ => "#94a3b8",
    }
}

fn lang_label(lang_id: &str) -> &'static str {
    match lang_id {
        "c" => "C",
        "cpp" => "C++",
        "rust" => "Rust",
        "zig" => "Zig",
        "logos_release" => "LOGOS",
        "go" => "Go",
        "java" => "Java",
        "js" => "JavaScript",
        "logos_interp" => "LOGOS (eager)",
        "logos_tiered" => "LOGOS (tiered)",
        "logos_aot" => "LOGOS (AOT-native)",
        "python" => "Python",
        "ruby" => "Ruby",
        "nim" => "Nim",
        _ => "Other",
    }
}

fn lang_ext(lang_id: &str) -> &'static str {
    match lang_id {
        "c" => "main.c",
        "cpp" => "main.cpp",
        "rust" => "main.rs",
        "zig" => "main.zig",
        "go" => "main.go",
        "java" => "Main.java",
        "js" => "main.js",
        "python" => "main.py",
        "ruby" => "main.rb",
        "nim" => "main.nim",
        _ => "main",
    }
}

fn compiler_label(key: &str) -> &'static str {
    match key {
        "gcc_-o3" => "gcc -O3 -march=native -flto",
        "g++_-o3" => "g++ -O3 -march=native -flto",
        "rustc_-o3" => "rustc -O3 -C lto=fat -C target-cpu=native",
        // legacy keys from older result files
        "gcc_-o2" => "gcc -O2",
        "g++_-o2" => "g++ -O2",
        "rustc_-o" => "rustc -O",
        "go_build" => "go build (release)",
        "javac" => "javac",
        "nim_c" => "nim c -d:release -march=native",
        "zig_build-exe" => "zig build-exe -O ReleaseFast -mcpu native",
        "largo_build" => "largo build (debug)",
        "largo_build_--release" => "largo build --release \u{2192} rustc -O3 -C lto=fat -C codegen-units=1 -C target-cpu=native",
        _ => "unknown",
    }
}

fn get_source(sources: &BenchSources, lang_id: &str) -> &'static str {
    match lang_id {
        "c" => sources.c,
        "cpp" => sources.cpp,
        "rust" => sources.rust,
        "zig" => sources.zig,
        "go" => sources.go,
        "java" => sources.java,
        "js" => sources.js,
        "python" => sources.python,
        "ruby" => sources.ruby,
        "nim" => sources.nim,
        _ => "",
    }
}

// Benchmarks where the LOGOS optimizer collapses the kernel — it does
// asymptotically less work than the naive algorithm the other languages run
// (tail-call / closed-form / loop folding), so the speedup reflects a compiler
// transform, not like-for-like codegen. These are excluded from the
// apples-to-apples geomean and carry a per-benchmark note. The set is curated
// from the measured results; the generated Rust shown on each benchmark makes
// every claim auditable.
fn collapse_note(id: &str) -> Option<&'static str> {
    match id {
        "fib" => Some("This one collapsed. The LOGOS optimizer folds the naive recursion to a closed form, so it does far less work than the runtime recursion the other languages execute — a compiler transform, not like-for-like codegen. See the generated Rust below."),
        "ackermann" => Some("This one collapsed. Deep recursion is folded by the optimizer instead of being executed call-by-call — a compiler transform, not like-for-like codegen. See the generated Rust below."),
        "binary_trees" => Some("This one collapsed. The allocate-and-checksum tree is reduced by the optimizer rather than built at runtime — a compiler transform, not like-for-like codegen. See the generated Rust below."),
        "loop_sum" => Some("This one collapsed. The accumulation loop is replaced with its closed-form sum (O(n) becomes O(1)) — a compiler transform, not like-for-like codegen. See the generated Rust below."),
        "collect" => Some("This one collapsed. The LOGOS optimizer folds the collection-building loop, doing far less work than inserting each element into a hash map at runtime — a compiler transform, not like-for-like codegen. See the generated Rust below."),
        _ => None,
    }
}

// Timings at a benchmark's effective reference size (reference_size if it has
// data, else the largest benchmarked size).
fn effective_ref(b: &Benchmark) -> Option<&HashMap<String, TimingResult>> {
    if let Some(t) = b.scaling.get(b.reference_size.as_str()) {
        return Some(t);
    }
    b.sizes.iter().rev().find_map(|s| b.scaling.get(s))
}

// LOGOS apples-to-apples geomean speedup vs C (C time / LOGOS time), over the
// benchmarks that did NOT collapse. Higher = faster.
fn logos_apples_geomean(data: &BenchmarkData) -> f64 {
    let mut log_sum = 0.0_f64;
    let mut n = 0u32;
    for b in &data.benchmarks {
        if collapse_note(&b.id).is_some() {
            continue;
        }
        if let Some(t) = effective_ref(b) {
            if let (Some(c), Some(l)) = (t.get("c"), t.get("logos_release")) {
                if c.mean_ms > 0.0 && l.mean_ms > 0.0 {
                    log_sum += (c.mean_ms / l.mean_ms).ln();
                    n += 1;
                }
            }
        }
    }
    if n > 0 { (log_sum / n as f64).exp() } else { 0.0 }
}

// Node sits near its ~30ms V8 startup floor when its time is dominated by
// process startup rather than compute; such interpreter benchmarks are flagged
// and kept out of the headline so they don't flatter the interpreter.
fn node_floored(t: &TimingResult) -> bool {
    t.mean_ms < 60.0
}

fn interp_ref<'a>(b: &'a InterpBenchmark) -> Option<&'a HashMap<String, TimingResult>> {
    b.scaling.get(b.reference_size.as_str()).or_else(|| b.scaling.values().next())
}

// LOGOS interpreter speed vs V8 (Node time / interpreter time), geomean over
// interpreter benchmarks where Node was off its startup floor. Higher = faster.
fn interp_speed_vs_v8(data: &InterpData) -> f64 {
    let mut log_sum = 0.0_f64;
    let mut n = 0u32;
    for b in &data.benchmarks {
        if let Some(t) = interp_ref(b) {
            if let (Some(js), Some(lg)) = (t.get("js"), t.get("logos_interp")) {
                if js.mean_ms > 0.0 && lg.mean_ms > 0.0 && !node_floored(js) {
                    log_sum += (js.mean_ms / lg.mean_ms).ln();
                    n += 1;
                }
            }
        }
    }
    if n > 0 { (log_sum / n as f64).exp() } else { 0.0 }
}

// LOGOS AOT-native (warm) speed vs V8 (Node time / AOT-native time), geomean over
// the benchmarks that carry a `logos_aot` row. Higher = faster. Returns 0 when no
// run has emitted the AOT-native tier yet, so the headline card stays hidden until
// a real AOT-native run regenerates the data.
fn aot_speed_vs_v8(data: &InterpData) -> f64 {
    let mut log_sum = 0.0_f64;
    let mut n = 0u32;
    for b in &data.benchmarks {
        if let Some(t) = interp_ref(b) {
            if let (Some(js), Some(lg)) = (t.get("js"), t.get("logos_aot")) {
                if js.mean_ms > 0.0 && lg.mean_ms > 0.0 && !node_floored(js) {
                    log_sum += (js.mean_ms / lg.mean_ms).ln();
                    n += 1;
                }
            }
        }
    }
    if n > 0 { (log_sum / n as f64).exp() } else { 0.0 }
}

fn fmt_n(n: f64) -> String {
    if n >= 1.0e9 { format!("{:.0}B", n / 1.0e9) }
    else if n >= 1.0e6 { format!("{:.0}M", n / 1.0e6) }
    else if n >= 1.0e3 { format!("{:.0}k", n / 1.0e3) }
    else { format!("{:.0}", n) }
}

// Per-benchmark scaling curve: wall-clock time vs problem size N, log-log, one
// line per language. Shows each language's scaling behaviour across the sizes
// already in the data (and where the LOGOS curve flattens out on a collapse).
fn scaling_chart(bench: &Benchmark, languages: &[Language]) -> Element {
    let mut sizes: Vec<(String, f64)> = bench.sizes.iter()
        .filter(|s| bench.scaling.contains_key(s.as_str()))
        .filter_map(|s| s.parse::<f64>().ok().map(|n| (s.clone(), n)))
        .collect();
    sizes.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    if sizes.len() < 2 {
        return rsx! {};
    }

    let mut t_min = f64::INFINITY;
    let mut t_max = 0.0_f64;
    for (s, _) in &sizes {
        if let Some(m) = bench.scaling.get(s) {
            for l in languages {
                if let Some(t) = m.get(&l.id) {
                    if t.median_ms > 0.0 {
                        t_min = t_min.min(t.median_ms);
                        t_max = t_max.max(t.median_ms);
                    }
                }
            }
        }
    }
    if !t_min.is_finite() || t_max <= 0.0 {
        return rsx! {};
    }

    let nx_min = sizes.first().unwrap().1.ln();
    let nx_max = sizes.last().unwrap().1.ln();
    let x_span = (nx_max - nx_min).max(1e-9);
    let y_span = (t_max.ln() - t_min.ln()).max(1e-9);

    let (x0, y0, pw, ph) = (54.0_f64, 14.0_f64, 572.0_f64, 220.0_f64);
    let px = |n: f64| x0 + (n.ln() - nx_min) / x_span * pw;
    let py = |t: f64| y0 + (1.0 - (t.ln() - t_min.ln()) / y_span) * ph;

    // (color, label, vertex coords)
    let mut series: Vec<(String, String, Vec<(f64, f64)>)> = Vec::new();
    for l in languages {
        let mut coords: Vec<(f64, f64)> = Vec::new();
        for (s, n) in &sizes {
            if let Some(t) = bench.scaling.get(s).and_then(|m| m.get(&l.id)) {
                if t.median_ms > 0.0 {
                    coords.push((px(*n), py(t.median_ms)));
                }
            }
        }
        if !coords.is_empty() {
            series.push((l.color.clone(), l.label.clone(), coords));
        }
    }
    if series.is_empty() {
        return rsx! {};
    }

    let y_bottom = y0 + ph;
    let x_right = x0 + pw;

    // Build the SVG inner markup as a string and set it via dangerous_inner_html
    // (the pattern the app's icons already use) so the chart never depends on
    // which individual SVG child attributes the rsx macro happens to expose.
    let mut svg_inner = String::new();
    svg_inner.push_str(&format!(
        "<line x1='{x0:.1}' y1='{y0:.1}' x2='{x0:.1}' y2='{y_bottom:.1}' stroke='rgba(255,255,255,0.15)' stroke-width='1'/>"
    ));
    svg_inner.push_str(&format!(
        "<line x1='{x0:.1}' y1='{y_bottom:.1}' x2='{x_right:.1}' y2='{y_bottom:.1}' stroke='rgba(255,255,255,0.15)' stroke-width='1'/>"
    ));
    svg_inner.push_str(&format!(
        "<text x='{:.1}' y='{:.1}' text-anchor='end' fill='rgba(229,231,235,0.45)' font-size='10'>{}</text>",
        x0 - 6.0, y0 + 4.0, format_time(t_max)
    ));
    svg_inner.push_str(&format!(
        "<text x='{:.1}' y='{:.1}' text-anchor='end' fill='rgba(229,231,235,0.45)' font-size='10'>{}</text>",
        x0 - 6.0, y_bottom, format_time(t_min)
    ));
    for (_, n) in &sizes {
        svg_inner.push_str(&format!(
            "<text x='{:.1}' y='{:.1}' text-anchor='middle' fill='rgba(229,231,235,0.45)' font-size='10'>n={}</text>",
            px(*n), y_bottom + 16.0, fmt_n(*n)
        ));
    }
    for (color, _label, coords) in &series {
        let pts = coords.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect::<Vec<_>>().join(" ");
        svg_inner.push_str(&format!(
            "<polyline points='{pts}' fill='none' stroke='{color}' stroke-width='2' stroke-linejoin='round' stroke-linecap='round'/>"
        ));
        for (x, y) in coords {
            svg_inner.push_str(&format!("<circle cx='{x:.1}' cy='{y:.1}' r='2.5' fill='{color}'/>"));
        }
    }

    rsx! {
        div { class: "bench-scaling",
            svg {
                view_box: "0 0 640 266",
                width: "100%",
                dangerous_inner_html: "{svg_inner}",
            }
            div { class: "bench-scaling-legend",
                for (color, label, _coords) in series.iter() {
                    div { class: "bench-scaling-legend-item",
                        span { class: "bench-scaling-legend-dot", style: "background: {color};" }
                        "{label}"
                    }
                }
            }
        }
    }
}

/// kB → a human byte size for the memory bars.
fn format_mem(kb: f64) -> String {
    if kb >= 1024.0 * 1024.0 {
        format!("{:.1} GB", kb / 1024.0 / 1024.0)
    } else if kb >= 1024.0 {
        format!("{:.1} MB", kb / 1024.0)
    } else {
        format!("{kb:.0} KB")
    }
}

/// bytes → a human size for the footprint bars (the byte-input sibling of `format_mem`).
fn format_bytes(bytes: f64) -> String {
    if bytes >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GB", bytes / 1024.0 / 1024.0 / 1024.0)
    } else if bytes >= 1024.0 * 1024.0 {
        format!("{:.1} MB", bytes / 1024.0 / 1024.0)
    } else {
        format!("{:.0} KB", bytes / 1024.0)
    }
}

/// Bar label for a footprint: the as-built size, plus a `(… stripped)` tag only when the
/// stripped size is smaller AND renders differently at display resolution (so an
/// already-minimal binary never reads as the redundant "327 KB (327 KB stripped)").
fn footprint_label(bytes: f64, stripped: Option<f64>) -> String {
    let main = format_bytes(bytes);
    match stripped {
        Some(st) if st > 0.0 && st < bytes => {
            let s = format_bytes(st);
            if s != main { format!("{main} ({s} stripped)") } else { main }
        }
        _ => main,
    }
}

/// The shipped footprint: the stripped, code-only binary you actually deploy — or the
/// as-built size when no stripped figure exists (e.g. a JVM class directory). This is the
/// honest, apples-to-apples number: some toolchains strip by default (`largo build
/// --release`) while others leave a symbol table on the as-built artifact, so comparing
/// as-built sizes would pit an already-stripped binary against unstripped ones.
fn shipped_bytes(as_built: f64, stripped: Option<f64>) -> f64 {
    match stripped {
        Some(s) if s > 0.0 && s <= as_built => s,
        _ => as_built,
    }
}

/// Bar label for the binary-size chart: the shipped (stripped) size is the headline, and
/// the as-built size is ALWAYS shown in parentheses so every row reads the same way. When a
/// binary already ships stripped (`largo build --release` strips by default) the two numbers
/// are equal — that identity is the honest story, not a reason to hide the second figure.
fn shipped_footprint_label(as_built: f64, stripped: Option<f64>) -> String {
    let shipped = format_bytes(shipped_bytes(as_built, stripped));
    let built = format_bytes(as_built);
    format!("{shipped} ({built} as-built)")
}

/// Compiled-artifact footprint — the size analogue of the memory bars, same visual style.
/// Each bar is the SHIPPED (stripped, code-only) binary you actually deploy, so every
/// toolchain is compared on the same basis; the as-built size (with symbols) rides along
/// in the label. Renders nothing until a size run populates `binary_sizes` (older files
/// have `binary_sizes: None`).
fn binary_size_bar_chart(bench: &Benchmark, languages: &[Language]) -> Element {
    let sizes = match &bench.binary_sizes {
        Some(s) => s,
        None => return rsx! {},
    };
    // (label, color, shipped, as_built, stripped, is_logos)
    let mut bars: Vec<(String, String, f64, f64, Option<f64>, bool)> = languages.iter()
        .filter_map(|l| sizes.by_language.get(&l.id)
            .filter(|s| s.as_built > 0.0)
            .map(|s| (l.label.clone(), l.color.clone(), shipped_bytes(s.as_built, s.stripped), s.as_built, s.stripped, l.id == "logos_release")))
        .collect();
    if bars.is_empty() {
        return rsx! {};
    }
    bars.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let max = bars.iter().map(|b| b.2).fold(0.0_f64, f64::max).max(1.0);

    rsx! {
        div { class: "bench-mem",
            div { class: "bench-chart-hint", "Shipped size of the compiled program \u{2014} the stripped, code-only binary you actually deploy, so every language is compared on the same basis. Shorter bar ships less; the as-built size (with symbols) is in parentheses." }
            div { class: "bench-chart",
                for (label, color, shipped, as_built, stripped, is_logos) in bars.iter() {
                    {
                        let pct = linear_bar_pct(*shipped, max);
                        let s = shipped_footprint_label(*as_built, *stripped);
                        let show_inside = pct > 32.0;
                        let bar_class = if *is_logos { "bench-bar-fill logos-highlight" } else { "bench-bar-fill" };
                        rsx! {
                            div { class: "bench-bar-row",
                                div { class: "bench-bar-label", "{label}" }
                                div { class: "bench-bar-track",
                                    div { class: "{bar_class}", style: "width: {pct:.1}%; background: {color};",
                                        if show_inside {
                                            span { class: "bench-bar-time", "{s}" }
                                        }
                                    }
                                }
                                if !show_inside {
                                    span { class: "bench-bar-time-outside", "{s}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Peak-RSS bar chart at the reference size — the memory analogue of the runtime
/// bars, in the same visual style. Renders nothing until a `MEASURE_MEM=1` run
/// populates `memory` in the result file (older files have `memory: None`).
fn memory_bar_chart(bench: &Benchmark, languages: &[Language]) -> Element {
    let mem = match &bench.memory {
        Some(m) => m,
        None => return rsx! {},
    };
    let size = if mem.by_size.contains_key(bench.reference_size.as_str()) {
        bench.reference_size.clone()
    } else {
        let mut ks: Vec<(String, f64)> = mem.by_size.keys()
            .filter_map(|s| s.parse::<f64>().ok().map(|n| (s.clone(), n)))
            .collect();
        ks.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        match ks.last() {
            Some((s, _)) => s.clone(),
            None => return rsx! {},
        }
    };
    let row = match mem.by_size.get(&size) {
        Some(r) => r,
        None => return rsx! {},
    };
    let mut bars: Vec<(String, String, f64)> = languages.iter()
        .filter_map(|l| row.get(&l.id).and_then(|v| *v)
            .filter(|kb| *kb > 0.0)
            .map(|kb| (l.label.clone(), l.color.clone(), kb)))
        .collect();
    if bars.is_empty() {
        return rsx! {};
    }
    bars.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let max = bars.iter().map(|b| b.2).fold(0.0_f64, f64::max);

    rsx! {
        div { class: "bench-mem",
            div { class: "bench-chart-hint", "Peak resident memory at n = {size} \u{2014} shorter bar uses less." }
            div { class: "bench-chart",
                for (label, color, kb) in bars.iter() {
                    {
                        let pct = (kb / max * 100.0).min(100.0);
                        let s = format_mem(*kb);
                        let show_inside = pct > 18.0;
                        rsx! {
                            div { class: "bench-bar-row",
                                div { class: "bench-bar-label", "{label}" }
                                div { class: "bench-bar-track",
                                    div { class: "bench-bar-fill", style: "width: {pct:.1}%; background: {color};",
                                        if show_inside {
                                            span { class: "bench-bar-time", "{s}" }
                                        }
                                    }
                                }
                                if !show_inside {
                                    span { class: "bench-bar-time-outside", "{s}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Declared time/space complexity plus the EMPIRICAL growth exponent fit from the
/// measured points: time from the scaling timings (present today), space from the
/// multi-size memory data (present after a `MEASURE_MEM=1` run). Renders nothing
/// when there is neither a declared complexity nor a fittable series.
fn complexity_panel(bench: &Benchmark, languages: &[Language]) -> Element {
    let mut sizes: Vec<(String, f64)> = bench.sizes.iter()
        .filter(|s| bench.scaling.contains_key(s.as_str()))
        .filter_map(|s| s.parse::<f64>().ok().map(|n| (s.clone(), n)))
        .collect();
    sizes.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let time_rows: Vec<(String, String, f64)> = languages.iter().filter_map(|l| {
        let pts: Vec<(f64, f64)> = sizes.iter().filter_map(|(s, n)| {
            bench.scaling.get(s).and_then(|m| m.get(&l.id))
                .filter(|t| t.median_ms > 0.0)
                .map(|t| (*n, t.median_ms))
        }).collect();
        empirical_exponent(&pts).map(|e| (l.color.clone(), l.label.clone(), e))
    }).collect();

    let space_rows: Vec<(String, String, f64)> = match &bench.memory {
        Some(mem) => {
            let mut ms: Vec<(String, f64)> = mem.by_size.keys()
                .filter_map(|s| s.parse::<f64>().ok().map(|n| (s.clone(), n)))
                .collect();
            ms.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            languages.iter().filter_map(|l| {
                let pts: Vec<(f64, f64)> = ms.iter().filter_map(|(s, n)| {
                    mem.by_size.get(s).and_then(|m| m.get(&l.id)).and_then(|v| *v)
                        .filter(|kb| *kb > 0.0).map(|kb| (*n, kb))
                }).collect();
                empirical_exponent(&pts).map(|e| (l.color.clone(), l.label.clone(), e))
            }).collect()
        }
        None => Vec::new(),
    };

    if bench.complexity.is_none() && time_rows.is_empty() && space_rows.is_empty() {
        return rsx! {};
    }

    rsx! {
        div { class: "bench-complexity",
            div { class: "bench-complexity-title", "Complexity" }
            if let Some(c) = &bench.complexity {
                div { class: "bench-complexity-declared",
                    span { class: "bench-complexity-chip",
                        "time " strong { "{c.time}" }
                    }
                    span { class: "bench-complexity-chip",
                        "space " strong { "{c.space}" }
                    }
                }
            }
            if !time_rows.is_empty() {
                div { class: "bench-complexity-grid",
                    div { class: "bench-complexity-col-title", "Measured time growth" }
                    for (color, label, e) in time_rows.iter() {
                        div { class: "bench-complexity-row",
                            span { class: "bench-scaling-legend-dot", style: "background: {color};" }
                            span { class: "bench-complexity-lang", "{label}" }
                            span { class: "bench-complexity-exp", "t \u{2248} n^{e:.2}" }
                        }
                    }
                }
            }
            if !space_rows.is_empty() {
                div { class: "bench-complexity-grid",
                    div { class: "bench-complexity-col-title", "Measured space growth" }
                    for (color, label, e) in space_rows.iter() {
                        div { class: "bench-complexity-row",
                            span { class: "bench-scaling-legend-dot", style: "background: {color};" }
                            span { class: "bench-complexity-lang", "{label}" }
                            span { class: "bench-complexity-exp", "rss \u{2248} n^{e:.2}" }
                        }
                    }
                }
            }
        }
    }
}

const BENCHMARKS_STYLE: &str = r#"
.bench-opt-toggles {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    margin: 16px 0 4px;
}
.bench-opt-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border: 1px solid rgba(255,255,255,0.12);
    border-radius: 6px;
    font-size: 12px;
    cursor: pointer;
    user-select: none;
    background: rgba(255,255,255,0.04);
    color: #cdd3e0;
    transition: border-color .15s, color .15s;
}
.bench-opt-toggle.off {
    border-color: rgba(255,120,120,0.55);
    color: #ff9a9a;
    background: rgba(255,90,90,0.06);
}
.bench-opt-toggle input { cursor: pointer; margin: 0; }
.bench-opt-toggle.on.firing {
    border-color: rgba(0,212,255,0.6);
    color: #00d4ff;
    background: rgba(0,212,255,0.08);
    box-shadow: 0 0 10px rgba(0,212,255,0.18);
}
.bench-opt-toggle.on.enabling {
    border-style: dashed;
    border-color: rgba(167,139,250,0.45);
    color: rgba(205,211,224,0.75);
}
.bench-opt-toggle.on.preempted {
    border-style: dashed;
    border-color: rgba(247,164,29,0.35);
    color: rgba(205,211,224,0.55);
    background: rgba(247,164,29,0.04);
}
.bench-tree-row {
    display: flex;
    align-items: center;
    gap: 4px;
}
.bench-tree-chevron {
    display: inline-block;
    width: 12px;
    flex: 0 0 12px;
    font-size: 9px;
    color: rgba(229,231,235,0.5);
    cursor: pointer;
    transition: transform 0.2s ease, color 0.15s;
    text-align: center;
}
.bench-tree-chevron:hover { color: #e5e7eb; }
.bench-tree-chevron.open { transform: rotate(90deg); }
.bench-tree-spacer {
    display: inline-block;
    width: 12px;
    flex: 0 0 12px;
}
.bench-opt-master {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    margin: 16px 0 6px;
}
.bench-opt-hint {
    font-size: 12px;
    color: rgba(229,231,235,0.5);
}
.bench-opt-rel {
    font-size: 10px;
    margin-left: 5px;
    padding: 1px 5px;
    border-radius: 4px;
    background: rgba(255,255,255,0.05);
    white-space: nowrap;
}
.bench-opt-rel.needs { color: #a78bfa; }
.bench-opt-rel.beats { color: #f7a41d; }
.bench-opt-rel.enables { color: #a78bfa; font-style: italic; }
.bench-opt-rel.depends { color: #6ee7b7; }
.bench-opt-rel.beaten { color: #f7a41d; font-style: italic; }
.bench-opt-rel.beats-now {
    color: #1a1410;
    background: rgba(247,164,29,0.85);
    font-weight: 700;
}
.bench-compiling {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    color: #00d4ff;
    font-size: 12px;
    font-weight: 500;
}
.bench-compiling::before {
    content: "";
    width: 11px;
    height: 11px;
    border: 2px solid rgba(0,212,255,0.25);
    border-top-color: #00d4ff;
    border-radius: 50%;
    animation: bench-spin 0.6s linear infinite;
}
@keyframes bench-spin {
    to { transform: rotate(360deg); }
}
.bench-container {
    min-height: 100vh;
    background: linear-gradient(135deg, #070a12 0%, #0b1022 50%, #070a12 100%);
    color: #e5e7eb;
    font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif;
}

.bench-hero {
    text-align: center;
    padding: 60px 20px 20px;
    max-width: 800px;
    margin: 0 auto;
}

.bench-hero h1 {
    font-size: 42px;
    font-weight: 800;
    letter-spacing: -1px;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin-bottom: 12px;
}

.bench-hero p {
    color: rgba(229,231,235,0.72);
    font-size: 18px;
    line-height: 1.6;
    margin-bottom: 20px;
}

.bench-gate-status {
    color: rgba(229,231,235,0.72);
    font-size: 18px;
    padding: 40px 0 12px;
}

.bench-gate-status.error {
    color: #fca5a5;
}

.bench-gate-retry {
    padding: 10px 28px;
    border-radius: 10px;
    border: 1px solid rgba(255,255,255,0.2);
    background: rgba(255,255,255,0.08);
    color: #e8e8e8;
    font-size: 15px;
    cursor: pointer;
}

.bench-gate-retry:hover {
    background: rgba(255,255,255,0.16);
}

.bench-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    justify-content: center;
    margin-bottom: 16px;
}

.bench-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    padding: 5px 12px;
    border-radius: 16px;
    background: rgba(255,255,255,0.05);
    border: 1px solid rgba(255,255,255,0.10);
    color: rgba(229,231,235,0.72);
    text-decoration: none;
    transition: all 0.2s ease;
}

.bench-pill:hover {
    background: rgba(255,255,255,0.08);
    border-color: rgba(255,255,255,0.15);
}

.bench-pill strong {
    color: #e5e7eb;
}

.bench-pill.link {
    cursor: pointer;
    color: #00d4ff;
}

.bench-section-nav {
    display: flex;
    justify-content: center;
    gap: 6px;
    padding: 12px 20px;
    margin-bottom: 24px;
    flex-wrap: wrap;
}

.bench-section-nav a {
    font-size: 12px;
    font-weight: 500;
    padding: 5px 14px;
    border-radius: 16px;
    background: rgba(255,255,255,0.04);
    border: 1px solid rgba(255,255,255,0.08);
    color: rgba(229,231,235,0.6);
    text-decoration: none;
    transition: all 0.2s ease;
    backdrop-filter: blur(12px);
}

.bench-section-nav a:hover {
    background: rgba(255,255,255,0.08);
    color: #e5e7eb;
    border-color: rgba(255,255,255,0.15);
}

.bench-tabbar {
    display: flex;
    justify-content: center;
    gap: 4px;
    max-width: 1000px;
    margin: 0 auto 28px;
    padding: 4px;
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 14px;
    width: fit-content;
    flex-wrap: wrap;
}

.bench-tabbar-btn {
    padding: 10px 22px;
    border: none;
    border-radius: 10px;
    background: transparent;
    color: rgba(229,231,235,0.55);
    font-size: 14px;
    font-weight: 600;
    letter-spacing: 0.2px;
    cursor: pointer;
    transition: all 0.2s ease;
}

.bench-tabbar-btn:hover {
    color: #e5e7eb;
    background: rgba(255,255,255,0.05);
}

.bench-tabbar-btn.active {
    background: linear-gradient(135deg, rgba(0,212,255,0.18), rgba(167,139,250,0.18));
    color: #e5e7eb;
    box-shadow: 0 2px 10px rgba(0,212,255,0.12);
}

.bench-content {
    max-width: 1000px;
    margin: 0 auto;
    padding: 0 20px 80px;
}

.bench-summary {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 16px;
    margin-bottom: 40px;
}

.bench-summary-card {
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 16px;
    padding: 24px;
    text-align: center;
    transition: all 0.2s ease;
}

.bench-summary-card:hover {
    background: rgba(255,255,255,0.05);
    border-color: rgba(255,255,255,0.12);
}

.bench-summary-value {
    font-size: 36px;
    font-weight: 800;
    letter-spacing: -1px;
    margin-bottom: 4px;
}

.bench-summary-eyebrow {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(229,231,235,0.5);
    margin-bottom: 8px;
}

.bench-summary-value.cyan { color: #00d4ff; }
.bench-summary-value.green { color: #22c55e; }
.bench-summary-value.purple { color: #a78bfa; }

.bench-summary-label {
    font-size: 13px;
    color: rgba(229,231,235,0.56);
}

.bench-tabs {
    display: flex;
    gap: 6px;
    margin-bottom: 24px;
    flex-wrap: wrap;
}

.bench-tab {
    padding: 8px 16px;
    border-radius: 8px;
    border: 1px solid rgba(255,255,255,0.1);
    background: rgba(255,255,255,0.03);
    color: #94a3b8;
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
    transition: all 0.2s ease;
}

.bench-tab:hover {
    background: rgba(255,255,255,0.08);
    color: #e8e8e8;
}

.bench-tab.active {
    background: linear-gradient(135deg, rgba(0,212,255,0.25), rgba(129,140,248,0.25));
    color: #00d4ff;
    border-color: rgba(0,212,255,0.4);
}

.bench-section {
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 16px;
    padding: 24px;
    margin-bottom: 24px;
}

.bench-section-title {
    font-size: 18px;
    font-weight: 700;
    color: #fff;
    margin-bottom: 6px;
}

.bench-section-desc {
    font-size: 13px;
    color: rgba(229,231,235,0.56);
    margin-bottom: 20px;
}

.bench-chart {
    display: flex;
    flex-direction: column;
    gap: 6px;
}

.bench-tier-label {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(229,231,235,0.4);
    margin-top: 10px;
    margin-bottom: 2px;
}

.bench-bar-row {
    display: flex;
    align-items: center;
    gap: 12px;
}

.bench-bar-label {
    width: 130px;
    text-align: right;
    font-size: 13px;
    font-weight: 500;
    color: rgba(229,231,235,0.8);
    flex-shrink: 0;
}

.bench-bar-track {
    flex: 1;
    min-width: 0;
    height: 28px;
    background: rgba(255,255,255,0.02);
    border-radius: 6px;
    overflow: hidden;
    position: relative;
}

.bench-bar-fill {
    height: 100%;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    padding-right: 8px;
    min-width: 2px;
    transition: width 0.4s ease;
}

.bench-bar-fill.logos-highlight {
    box-shadow: 0 0 16px rgba(0,212,255,0.35);
}

.bench-bar-time {
    font-size: 11px;
    font-weight: 600;
    color: rgba(0,0,0,0.8);
    white-space: nowrap;
}

.bench-bar-time-outside {
    font-size: 11px;
    font-weight: 600;
    color: rgba(229,231,235,0.6);
    white-space: nowrap;
    margin-left: 8px;
    flex-shrink: 0;
}

/* Codec charts: the bar shows magnitude only; the exact value lives in its own
   always-visible right column, so even the smallest (winning) bar never hides its number. */
.bench-codec-track {
    flex: 1;
    height: 22px;
    background: rgba(255,255,255,0.04);
    border-radius: 6px;
    position: relative;
}

.bench-codec-fill {
    height: 100%;
    border-radius: 6px;
    min-width: 3px;
    transition: width 0.4s ease;
}

.bench-codec-fill.logos-highlight {
    box-shadow: 0 0 14px rgba(0,212,255,0.5);
}

.bench-codec-val {
    width: 92px;
    text-align: right;
    font-size: 12px;
    font-weight: 600;
    color: rgba(229,231,235,0.75);
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
}

.bench-codec-val.logos-val {
    color: #38e1ff;
}

.bench-source {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
}

.bench-source-panel {
    display: flex;
    flex-direction: column;
}

.bench-source-header {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    padding: 8px 12px;
    border-radius: 8px 8px 0 0;
    border: 1px solid rgba(255,255,255,0.08);
    border-bottom: none;
}

.bench-source-header.logos {
    background: rgba(0,212,255,0.1);
    color: #00d4ff;
}

.bench-source-header.rust {
    background: rgba(222,165,132,0.1);
    color: #dea584;
}

.bench-source-code {
    background: rgba(0,0,0,0.3);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 0 0 8px 8px;
    padding: 16px;
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 12px;
    line-height: 1.5;
    color: #e8e8e8;
    white-space: pre-wrap;
    overflow-x: auto;
    flex: 1;
}

/* Collapsible pattern */
.bench-collapsible-btn {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 10px 0;
    background: none;
    border: none;
    border-top: 1px solid rgba(255,255,255,0.06);
    color: rgba(229,231,235,0.6);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: color 0.2s ease;
    margin-top: 16px;
    text-align: left;
}

.bench-collapsible-btn:hover {
    color: #e5e7eb;
}

.bench-collapsible-chevron {
    display: inline-block;
    font-size: 10px;
    transition: transform 0.2s ease;
}

.bench-collapsible-chevron.open {
    transform: rotate(90deg);
}

.bench-collapsible-body {
    overflow: hidden;
    max-height: 0;
    transition: max-height 0.3s ease;
}

.bench-collapsible-body.open {
    max-height: 8000px;
}

/* Stats table */
.bench-stats-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
    margin-top: 12px;
}

.bench-stats-table th {
    text-align: left;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(229,231,235,0.5);
    padding: 6px 8px;
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.bench-stats-table td {
    padding: 6px 8px;
    color: rgba(229,231,235,0.7);
    border-bottom: 1px solid rgba(255,255,255,0.04);
    font-family: 'SF Mono', 'Fira Code', monospace;
    font-size: 11px;
}

.bench-stats-table tr.highlight td {
    color: #00d4ff;
    background: rgba(0,212,255,0.05);
}

/* Language source collapsible */
.bench-lang-collapsible {
    border: 1px solid rgba(255,255,255,0.06);
    border-radius: 8px;
    margin-bottom: 8px;
    overflow: hidden;
}

.bench-lang-header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    background: rgba(255,255,255,0.02);
    cursor: pointer;
    transition: background 0.2s ease;
}

.bench-lang-header:hover {
    background: rgba(255,255,255,0.05);
}

.bench-lang-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
}

.bench-lang-name {
    font-size: 13px;
    font-weight: 500;
    color: #e5e7eb;
    flex: 1;
}

.bench-lang-version {
    font-size: 11px;
    color: rgba(229,231,235,0.4);
}

.bench-lang-link {
    font-size: 11px;
    color: #00d4ff;
    text-decoration: none;
    opacity: 0.7;
    transition: opacity 0.2s ease;
}

.bench-lang-link:hover {
    opacity: 1;
    text-decoration: underline;
}

.bench-lang-code {
    background: rgba(0,0,0,0.3);
    padding: 16px;
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 12px;
    line-height: 1.5;
    color: #e8e8e8;
    white-space: pre-wrap;
    overflow-x: auto;
    max-height: 0;
    transition: max-height 0.3s ease, padding 0.3s ease;
    padding-top: 0;
    padding-bottom: 0;
}

.bench-lang-code.open {
    max-height: 4000px;
    padding-top: 16px;
    padding-bottom: 16px;
}

/* Compile chart */
.bench-compile-table {
    width: 100%;
    border-collapse: collapse;
}

.bench-compile-table th {
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(229,231,235,0.5);
    padding: 8px 12px;
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.bench-compile-table td {
    padding: 8px 12px;
    font-size: 13px;
    color: rgba(229,231,235,0.8);
    border-bottom: 1px solid rgba(255,255,255,0.04);
}

.bench-compile-table tr:last-child td {
    border-bottom: none;
}

.bench-compile-table .compiler-name {
    font-weight: 500;
    color: #e5e7eb;
}

.bench-compile-table tr.highlight td {
    color: #00d4ff;
}

/* Methodology */
.bench-methodology {
    color: rgba(229,231,235,0.56);
    font-size: 13px;
    line-height: 1.7;
}

.bench-methodology ul {
    padding-left: 20px;
    margin: 8px 0;
}

.bench-methodology li {
    margin-bottom: 4px;
}

.bench-methodology a {
    color: #00d4ff;
    text-decoration: none;
}

.bench-methodology a:hover {
    text-decoration: underline;
}

.bench-methodology h3 {
    font-size: 14px;
    font-weight: 600;
    color: #e5e7eb;
    margin: 16px 0 8px;
}

.bench-version-table {
    width: 100%;
    border-collapse: collapse;
    margin: 8px 0 16px;
}

.bench-version-table th {
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(229,231,235,0.5);
    padding: 6px 10px;
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.bench-version-table td {
    padding: 6px 10px;
    font-size: 12px;
    color: rgba(229,231,235,0.7);
    border-bottom: 1px solid rgba(255,255,255,0.04);
}

/* Algorithmic-collapse note + badges */
.bench-summary-value sup {
    font-size: 18px;
    color: rgba(0,212,255,0.7);
    font-weight: 700;
}

.bench-note {
    background: rgba(0,212,255,0.04);
    border: 1px solid rgba(0,212,255,0.15);
    border-radius: 12px;
    padding: 14px 18px;
    margin-bottom: 32px;
    font-size: 13px;
    line-height: 1.6;
    color: rgba(229,231,235,0.7);
}

.bench-note strong { color: #00d4ff; }

.bench-tab-badge {
    display: inline-block;
    margin-left: 6px;
    font-size: 10px;
}

.bench-callout {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    background: rgba(0,212,255,0.05);
    border: 1px solid rgba(0,212,255,0.18);
    border-radius: 10px;
    padding: 12px 14px;
    margin-bottom: 18px;
    font-size: 13px;
    line-height: 1.55;
    color: rgba(229,231,235,0.8);
}

.bench-callout-icon {
    flex-shrink: 0;
    font-size: 15px;
    line-height: 1.4;
}

.bench-chart-hint {
    font-size: 11px;
    color: rgba(229,231,235,0.4);
    margin-bottom: 10px;
}

/* Scaling curve (inline SVG) */
.bench-scaling {
    margin-top: 8px;
}

.bench-scaling svg {
    width: 100%;
    height: auto;
    display: block;
}

.bench-scaling-legend {
    display: flex;
    flex-wrap: wrap;
    gap: 10px 16px;
    margin-top: 10px;
}

.bench-scaling-legend-item {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: rgba(229,231,235,0.6);
}

.bench-scaling-legend-dot {
    width: 10px;
    height: 3px;
    border-radius: 2px;
}

/* Complexity panel + memory bars */
.bench-complexity {
    margin: 20px 0 4px;
    padding: 14px 16px;
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 8px;
    background: rgba(255,255,255,0.02);
}
.bench-complexity-title {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(229,231,235,0.45);
    margin-bottom: 10px;
}
.bench-complexity-declared {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
    margin-bottom: 12px;
}
.bench-complexity-chip {
    font-size: 12px;
    color: rgba(229,231,235,0.6);
    padding: 4px 10px;
    border-radius: 6px;
    background: rgba(255,255,255,0.04);
    border: 1px solid rgba(255,255,255,0.08);
}
.bench-complexity-chip strong { color: #93c5fd; font-weight: 700; margin-left: 4px; }
.bench-complexity-grid {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 8px;
}
.bench-complexity-col-title {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    color: rgba(229,231,235,0.35);
    margin-bottom: 2px;
}
.bench-complexity-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
}
.bench-complexity-lang { color: rgba(229,231,235,0.7); min-width: 90px; }
.bench-complexity-exp { color: rgba(229,231,235,0.5); font-family: ui-monospace, monospace; }
.bench-mem { margin-top: 8px; }

/* Interpreter-vs-V8 section bits */
.bench-engine-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    padding: 3px 10px;
    border-radius: 12px;
    background: rgba(255,140,0,0.1);
    border: 1px solid rgba(255,140,0,0.25);
    color: #ff8c00;
    margin-left: 8px;
}

.bench-floor-badge {
    font-size: 10px;
    color: #fbbf24;
    margin-left: 8px;
    white-space: nowrap;
}

.bench-bar-n {
    font-size: 10px;
    color: rgba(229,231,235,0.4);
    margin-left: 6px;
}

@media (max-width: 768px) {
    .bench-hero h1 { font-size: 32px; }
    .bench-summary { grid-template-columns: 1fr; }
    .bench-source { grid-template-columns: 1fr; }
    .bench-bar-label { width: 80px; font-size: 11px; }
    .bench-summary-value { font-size: 28px; }
    .bench-section-nav { gap: 4px; }
    .bench-section-nav a { font-size: 11px; padding: 4px 10px; }
    .bench-tabbar { width: auto; }
    .bench-tabbar-btn { padding: 9px 14px; font-size: 12.5px; }
}

@media (max-width: 480px) {
    .bench-hero h1 { font-size: 26px; }
    .bench-hero p { font-size: 15px; }
    .bench-content { padding: 0 12px 60px; }
}
"#;

/// The three top-level benchmark stories, shown as tabs: the programming
/// language itself, the SAT solver, and the transport wire codec.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BenchTab {
    Language,
    Solver,
    Codec,
}

impl BenchTab {
    fn label(self) -> &'static str {
        match self {
            BenchTab::Language => "Programming Language",
            BenchTab::Solver => "SAT Solver",
            BenchTab::Codec => "Transport Codec",
        }
    }

    fn all() -> [BenchTab; 3] {
        [BenchTab::Language, BenchTab::Solver, BenchTab::Codec]
    }
}

/// Gate component: guarantees the data feeds are pinned before the page proper
/// renders, so every accessor inside `BenchmarksLoaded` can `expect` its feed.
/// Native builds (tests, SSG prerender) compile the data in and render directly;
/// wasm fetches the staged `/data` bundle first, behind a lightweight shell.
#[component(lazy)]
pub fn Benchmarks() -> Element {
    #[cfg(not(target_arch = "wasm32"))]
    return rsx! { BenchmarksLoaded {} };

    #[cfg(target_arch = "wasm32")]
    {
        let mut bundle = use_resource(load_bench_bundle);
        let state = bundle.read_unchecked();
        match &*state {
            Some(Ok(())) => rsx! { BenchmarksLoaded {} },
            Some(Err(e)) => rsx! {
                BenchShell {
                    p { class: "bench-gate-status error", "Benchmark data failed to load: {e}" }
                    button {
                        class: "bench-gate-retry",
                        onclick: move |_| bundle.restart(),
                        "Retry"
                    }
                }
            },
            None => rsx! {
                BenchShell {
                    p { class: "bench-gate-status", "Loading benchmark data\u{2026}" }
                }
            },
        }
    }
}

/// The nav/meta shell shown while the wasm data bundle is in flight (or failed):
/// full PageHead + nav + footer, so the page keeps its meta and chrome either way.
#[cfg(target_arch = "wasm32")]
#[component]
fn BenchShell(children: Element) -> Element {
    rsx! {
        PageHead {
            title: seo_pages::BENCHMARKS.title,
            description: seo_pages::BENCHMARKS.description,
            canonical_path: seo_pages::BENCHMARKS.canonical_path,
        }
        style { "{BENCHMARKS_STYLE}" }
        div { class: "bench-container",
            MainNav { active: ActivePage::Benchmarks, subtitle: Some("Performance") }
            section { class: "bench-hero",
                h1 { "Benchmarks" }
                {children}
            }
            Footer {}
        }
    }
}

#[component]
fn BenchmarksLoaded() -> Element {
    let data = bench_data().expect("gated: the Benchmarks shell loads the bundle first");
    let mut active_bench = use_signal(|| 0usize);
    let mut bench_tab = use_signal(|| BenchTab::Language);
    // Optimization-toggle showcase state: one on/off per registry optimization
    // (all on by default = the normal optimized build). Flipping one inserts a
    // `## No <X>` decorator and recompiles the Rust live, in the browser.
    let mut opt_toggles = use_signal(|| vec![true; logicaffeine_compile::optimization::REGISTRY.len()]);
    // Live-compile state for the toggle showcase. `live_rust` holds the last
    // in-browser compile (None = show the pre-baked cached Rust), `compiling`
    // drives the spinner, and `compile_gen` lets a newer toggle supersede an
    // in-flight compile.
    let mut live_rust = use_signal(|| Option::<String>::None);
    let mut compiling = use_signal(|| false);
    let mut compile_gen = use_signal(|| 0u64);
    // The all-on optimization graph for the current benchmark — the STABLE tree
    // structure. `base_fired` (what fires), `base_preempted` (blockers), and
    // `base_dependencies` (emergent per-program dependencies) are seeded instantly
    // from the baked benchmark data so switching benchmarks pops the tree in with no
    // compile; `fired_now`/`preempted_now` reflect the CURRENT toggle state (what is
    // firing right now), updated by the live re-trace when an optimization is off.
    let mut base_fired = use_signal(Vec::<&'static str>::new);
    let mut fired_now = use_signal(Vec::<&'static str>::new);
    let mut preempted_now = use_signal(Vec::<(&'static str, &'static str)>::new);
    let mut base_preempted = use_signal(Vec::<(&'static str, &'static str)>::new);
    let mut base_dependencies = use_signal(Vec::<(&'static str, &'static str)>::new);
    // Tree expand/collapse state, one bool per registry optimization (default
    // expanded). A collapsed parent hides its requires-descendants.
    let mut expanded = use_signal(|| vec![true; logicaffeine_compile::optimization::REGISTRY.len()]);
    let mut stats_open = use_signal(|| false);
    let mut compile_detail_open = use_signal(|| false);
    let mut methodology_open = use_signal(|| false);
    let mut source_open: Signal<[bool; 10]> = use_signal(|| [false; 10]);

    // Seed the stable all-on graph from the BAKED benchmark data and re-trace the
    // showcase Rust on toggle changes. The DEFAULT all-on view never compiles in the
    // browser — the optimization graph is embedded in `latest.json` (baked by the
    // benchmark run / `scripts/bake-opt-graph.sh`, exactly like the timing results)
    // and the Rust is the pre-baked `generated_rust`, so switching benchmarks is
    // instant. ONLY turning an optimization OFF triggers a browser compile, and only
    // to show that toggled state's Rust + which opts then fire — the one thing that
    // cannot be pre-computed (it is combinatorial). Tracks `active_bench` +
    // `opt_toggles`; `compile_gen` is read via peek so the effect never re-triggers
    // itself.
    use_effect(move || {
        let idx = active_bench();
        let toggles = opt_toggles();
        let b = &data.benchmarks[idx];
        let kw = |s: &str| logicaffeine_compile::optimization::by_keyword(s).map(|o| o.meta().keyword);
        // The baked all-on graph (keywords → interned &'static str). Always seeds the
        // stable tree structure — no analysis in the browser.
        let baked_fired: Vec<&'static str> = b.fired.iter().filter_map(|s| kw(s)).collect();
        let baked_blockers: Vec<(&'static str, &'static str)> =
            b.blockers.iter().filter_map(|(w, l)| Some((kw(w)?, kw(l)?))).collect();
        let baked_deps: Vec<(&'static str, &'static str)> =
            b.dependencies.iter().filter_map(|(d, x)| Some((kw(d)?, kw(x)?))).collect();
        base_fired.set(baked_fired.clone());
        base_preempted.set(baked_blockers.clone());
        base_dependencies.set(baked_deps);

        let disabled: Vec<&'static str> = logicaffeine_compile::optimization::REGISTRY
            .iter()
            .enumerate()
            .filter(|(i, _)| !toggles[*i])
            .map(|(_, m)| m.keyword)
            .collect();
        let cache_present = !b.generated_rust.trim().is_empty();

        // Default view (every optimization on): fully served from baked data — the
        // baked fired set, the baked blockers, and the cached Rust. No compile, ever.
        if disabled.is_empty() {
            fired_now.set(baked_fired);
            preempted_now.set(baked_blockers);
            live_rust.set(if cache_present { None } else { Some(b.generated_rust.clone()) });
            compiling.set(false);
            return;
        }

        // An optimization is OFF — compile just this toggled state to show its Rust
        // and which optimizations now fire.
        let decorated =
            logicaffeine_compile::optimization::decorate_source(&b.logos_source, &disabled);
        let gen = *compile_gen.peek() + 1;
        compile_gen.set(gen);
        compiling.set(true);
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(30).await;
            if *compile_gen.peek() != gen {
                return;
            }
            let (rust, fired, preempted) =
                logicaffeine_compile::compile::compile_to_rust_traced(&decorated)
                    .unwrap_or_else(|e| (format!("// compile error: {e:?}"), Vec::new(), Vec::new()));
            if *compile_gen.peek() != gen {
                return;
            }
            fired_now.set(fired);
            preempted_now.set(preempted);
            live_rust.set(Some(rust));
            compiling.set(false);
        });
    });

    let breadcrumbs = vec![
        BreadcrumbItem { name: "Home", path: "/" },
        BreadcrumbItem { name: "Benchmarks", path: "/benchmarks" },
    ];
    let schemas = vec![
        organization_schema(),
        webpage_schema("LOGOS Benchmarks", seo_pages::BENCHMARKS.description, "/benchmarks"),
        breadcrumb_schema(&breadcrumbs),
    ];

    let logos_vs_c = data.summary.geometric_mean_speedup_vs_c
        .get("logos_release").copied().unwrap_or(0.0);

    let interp = interp_data().expect("gated: the Benchmarks shell loads the bundle first");
    // Headline numbers, all framed as "x the speed of <baseline>" (higher = faster).
    let logos_apples = logos_apples_geomean(data);
    let interp_speed = interp_speed_vs_v8(interp);
    let aot_speed = aot_speed_vs_v8(interp);
    let collapse_count = data.benchmarks.iter().filter(|b| collapse_note(&b.id).is_some()).count();
    let apples_count = data.benchmarks.len().saturating_sub(collapse_count);

    let bench = &data.benchmarks[active_bench()];
    let bench_sources = bench_sources_for(&bench.id)
        .expect("sources_cover_every_benchmark locks every benchmark id to a sources entry");

    // Toggle showcase — cheap, render-safe derived state only. The decorated
    // LOGOS source is a string op; the expensive Rust compile is NOT run here
    // (doing it on every render froze the page). With every optimization on we
    // show the pre-baked cached Rust (`bench.generated_rust`) for an instant,
    // no-compile view; any optimization off shows the live-compiled `live_rust`
    // produced asynchronously by the use_effect above, with a spinner in between.
    let opt_tog = opt_toggles();
    let opt_disabled: Vec<&'static str> = logicaffeine_compile::optimization::REGISTRY
        .iter()
        .enumerate()
        .filter(|(i, _)| !opt_tog[*i])
        .map(|(_, m)| m.keyword)
        .collect();
    let opt_decorated =
        logicaffeine_compile::optimization::decorate_source(&bench.logos_source, &opt_disabled);
    let all_opts_on = opt_disabled.is_empty();

    // The program's optimization chain as a tree, derived by the crate's
    // `relationship_tree` from the baked all-on graph: the fired opts and the
    // `requires` enablers they hang under (so `coins`' fired `unchecked`/
    // `oraclehints`/`elemtype` show under `oracle`), the BLOCKERS they skipped, and
    // the per-program DEPENDENCIES that nest one opt under another it only fired
    // because of (dead-code under scalarization, symmetry under partial eval). Map
    // the keyword signals back to `Opt` and hand them to the deterministic O(n²)
    // derivation — the single source of truth for the menu-tree.
    let opt_tree: Vec<logicaffeine_compile::optimization::OptNode> = {
        use logicaffeine_compile::optimization::by_keyword;
        let fired: Vec<_> = base_fired().iter().filter_map(|kw| by_keyword(kw)).collect();
        let preempted: Vec<_> = base_preempted()
            .iter()
            .filter_map(|(w, l)| Some((by_keyword(w)?, by_keyword(l)?)))
            .collect();
        let dependencies: Vec<_> = base_dependencies()
            .iter()
            .filter_map(|(d, x)| Some((by_keyword(d)?, by_keyword(x)?)))
            .collect();
        logicaffeine_compile::optimization::relationship_tree(&fired, &preempted, &dependencies)
    };

    // Collapse visibility, parallel to `opt_tree`: walking the pre-order DFS, once
    // a collapsed parent is seen every deeper node is hidden until depth returns to
    // the parent's level. A single threshold suffices because the order is DFS.
    let tree_visible: Vec<bool> = {
        let exp = expanded();
        let mut vis = Vec::with_capacity(opt_tree.len());
        let mut hide_from: Option<usize> = None;
        for node in &opt_tree {
            if let Some(d) = hide_from {
                if node.depth <= d {
                    hide_from = None;
                }
            }
            let visible = hide_from.is_none();
            if visible && node.has_children && !exp.get(node.opt as usize).copied().unwrap_or(true) {
                hide_from = Some(node.depth);
            }
            vis.push(visible);
        }
        vis
    };

    let cache_ok = !bench.generated_rust.trim().is_empty();
    let rust_loading = (!all_opts_on || !cache_ok) && (compiling() || live_rust().is_none());
    let rust_text: String = if all_opts_on && cache_ok {
        bench.generated_rust.clone()
    } else {
        live_rust().unwrap_or_default()
    };
    // Use reference_size if it has data, otherwise fall back to the largest benchmarked size
    let ref_size = if bench.scaling.contains_key(bench.reference_size.as_str()) {
        bench.reference_size.clone()
    } else {
        bench.sizes.last().cloned().unwrap_or_else(|| bench.reference_size.clone())
    };
    let ref_timings = bench.scaling.get(ref_size.as_str());

    // (label, color, median_ms, tier, is_logos, is_timeout)
    let mut chart_entries: Vec<(&str, &str, f64, &str, bool, bool)> = Vec::new();
    let ref_timeout = bench.timeouts.get(ref_size.as_str());
    if let Some(timings) = ref_timings {
        for lang in &data.languages {
            // Node/V8 lives in the dedicated interpreter section below, not in the
            // compiled-vs-systems-languages charts at the top.
            if lang.id == "js" { continue; }
            if let Some(t) = timings.get(&lang.id) {
                chart_entries.push((
                    &lang.label,
                    &lang.color,
                    t.median_ms,
                    &lang.tier,
                    lang.id == "logos_release",
                    false,
                ));
            } else if ref_timeout.is_some() {
                // Language missing from results at a size that had a timeout — it timed out
                chart_entries.push((
                    &lang.label,
                    &lang.color,
                    ref_timeout.unwrap() + 1.0, // sort after everything else
                    &lang.tier,
                    lang.id == "logos_release",
                    true,
                ));
            }
        }
    }

    chart_entries.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    // Split into compiled and interpreted with independent scales (exclude timeouts from max calculation)
    let compiled_max = chart_entries.iter()
        .filter(|e| e.3 != "interpreted" && !e.5)
        .map(|e| e.2)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(1.0);

    let interpreted_max = chart_entries.iter()
        .filter(|e| e.3 == "interpreted" && !e.5)
        .map(|e| e.2)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(1.0);

    let compiled_tier_order = ["systems", "managed", "transpiled"];
    let mut compiled_grouped: Vec<(&str, Vec<(&str, &str, f64, bool, bool)>)> = Vec::new();
    for &tier in &compiled_tier_order {
        let entries: Vec<_> = chart_entries.iter()
            .filter(|e| e.3 == tier)
            .map(|e| (e.0, e.1, e.2, e.4, e.5))
            .collect();
        if !entries.is_empty() {
            compiled_grouped.push((tier, entries));
        }
    }

    let interpreted_flat: Vec<(&str, &str, f64, bool, bool)> = chart_entries.iter()
        .filter(|e| e.3 == "interpreted")
        .map(|e| (e.0, e.1, e.2, e.4, e.5))
        .collect();

    // Stats table entries (all fields, sorted by median; timed-out langs appended at end)
    let mut stats_entries: Vec<(&str, &str, Option<&TimingResult>)> = Vec::new();
    if let Some(timings) = ref_timings {
        for lang in &data.languages {
            if lang.id == "js" { continue; }
            if let Some(t) = timings.get(&lang.id) {
                stats_entries.push((&lang.label, &lang.id, Some(t)));
            } else if ref_timeout.is_some() {
                stats_entries.push((&lang.label, &lang.id, None));
            }
        }
    }
    stats_entries.sort_by(|a, b| {
        match (a.2, b.2) {
            (Some(ta), Some(tb)) => ta.median_ms.partial_cmp(&tb.median_ms).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });

    // Compilation entries sorted by mean_ms
    let mut compile_entries: Vec<(&str, f64, f64, bool)> = bench.compilation.iter()
        .map(|(name, r)| (name.as_str(), r.mean_ms, r.stddev_ms, name.starts_with("largo")))
        .collect();
    compile_entries.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let compile_max = compile_entries.last().map(|e| e.1).unwrap_or(1.0);

    // Summary chart entries (geometric mean) sorted by value descending
    let mut summary_entries: Vec<(&str, f64, &str, bool)> = Vec::new();
    for lang in &data.languages {
        if lang.id == "js" { continue; }
        if let Some(&val) = data.summary.geometric_mean_speedup_vs_c.get(&lang.id) {
            summary_entries.push((&lang.label, val, &lang.color, lang.id == "logos_release"));
        }
    }
    summary_entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let summary_max = summary_entries.first().map(|e| e.1).unwrap_or(1.0);

    // Interpreter vs V8, for the active benchmark (at its own calibrated N)
    let interp_bench = interp.benchmarks.iter().find(|ib| ib.id == bench.id);
    // (label, color, median_ms, is_logos, node_floored)
    let mut interp_bars: Vec<(&'static str, &'static str, f64, bool, bool)> = Vec::new();
    let mut interp_n = String::new();
    let mut interp_engine = String::new();
    let mut interp_active_speed: Option<f64> = None;
    if let Some(ib) = interp_bench {
        interp_n = ib.reference_size.clone();
        interp_engine = ib.interpreter_engine.clone();
        if let Some(t) = interp_ref(ib) {
            // The LOGOS tier ladder (eager VM → tiered VM+JIT → AOT-native, HOTSWAP
            // §12/§Axis-3) then the peer runtimes. Data-driven: a tier renders only
            // when its row is present, so tiered/AOT appear once the run emits them.
            for id in ["logos_interp", "logos_tiered", "logos_aot", "js"] {
                if let Some(tr) = t.get(id) {
                    let lbl = if id == "js" { "Node / V8" } else { lang_label(id) };
                    let col = lang_color(id);
                    let is_logos = id.starts_with("logos");
                    interp_bars.push((lbl, col, tr.median_ms, is_logos, id == "js" && node_floored(tr)));
                }
            }
            if let (Some(j), Some(l)) = (t.get("js"), t.get("logos_interp")) {
                if l.median_ms > 0.0 {
                    interp_active_speed = Some(j.median_ms / l.median_ms);
                }
            }
        }
    }
    interp_bars.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let interp_max = interp_bars.iter().map(|e| e.2).fold(0.0_f64, f64::max).max(1e-9);
    if interp_engine.is_empty() { interp_engine = "\u{2014}".to_string(); }
    let interp_offfloor = interp.benchmarks.iter().filter(|ib| {
        interp_ref(ib).and_then(|t| {
            let j = t.get("js")?;
            t.get("logos_interp")?;
            Some(!node_floored(j))
        }).unwrap_or(false)
    }).count();
    let interp_node_ver = if interp.metadata.node.is_empty() { "Node".to_string() } else { format!("Node {}", interp.metadata.node) };

    // Cold-start floor (serverless / CLI): time to launch the engine and run a
    // trivial program. Smaller is faster. (label, color, mean_ms, is_logos)
    let mut startup_bars: Vec<(&'static str, &'static str, f64, bool)> = Vec::new();
    for id in ["logos_interp", "js"] {
        if let Some(t) = interp.startup.engines.get(id) {
            if t.mean_ms > 0.0 {
                let lbl = match id { "logos_interp" => "LOGOS interp", "js" => "Node / V8", _ => id };
                let col = match id { "logos_interp" => "#ff8c00", "js" => "#f7df1e", _ => "#94a3b8" };
                startup_bars.push((lbl, col, t.mean_ms, id == "logos_interp"));
            }
        }
    }
    startup_bars.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let startup_max = startup_bars.iter().map(|e| e.2).fold(0.0_f64, f64::max).max(1e-9);
    let startup_logos = interp.startup.engines.get("logos_interp").map(|t| t.mean_ms).unwrap_or(0.0);
    let startup_node = interp.startup.engines.get("js").map(|t| t.mean_ms).unwrap_or(0.0);
    let startup_vs_v8 = if startup_logos > 0.0 { startup_node / startup_logos } else { 0.0 };

    // Engine footprint — what you ship to run a program. (label, color, as_built, stripped, is_logos)
    let mut engine_size_bars: Vec<(&'static str, &'static str, f64, Option<f64>, bool)> = Vec::new();
    let mut wasm_bundle_bytes = 0.0_f64;
    if let Some(es) = &interp.interpreter_sizes {
        for id in ["logos", "node", "deno", "bun"] {
            if let Some(s) = es.engines.get(id) {
                if s.as_built > 0.0 {
                    let lbl = match id {
                        "logos" => "largo (LOGOS VM+JIT)",
                        "node" => "Node / V8",
                        "deno" => "Deno",
                        "bun" => "Bun",
                        _ => id,
                    };
                    let col = match id { "logos" => "#ff8c00", "node" => "#f7df1e", _ => "#94a3b8" };
                    engine_size_bars.push((lbl, col, s.as_built, s.stripped, id == "logos"));
                }
            }
        }
        wasm_bundle_bytes = es.wasm_bundle_bytes.unwrap_or(0.0);
    }
    engine_size_bars.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let engine_size_max = engine_size_bars.iter().map(|e| e.2).fold(0.0_f64, f64::max).max(1.0);
    let wasm_bundle_str = if wasm_bundle_bytes > 0.0 { format_bytes(wasm_bundle_bytes) } else { String::new() };

    // Source code languages to show (not LOGOS — that's always visible)
    let source_langs = ["c", "cpp", "rust", "zig", "go", "java", "js", "python", "ruby", "nim"];

    let commit_url = format!("{}/commit/{}", GITHUB_REPO, data.metadata.commit);
    let release_url = format!("{}/releases/tag/v{}", GITHUB_REPO, data.metadata.logos_version);
    let raw_json_url = format!("{}/blob/main/benchmarks/results/latest.json", GITHUB_REPO);
    // Per-tab hero metadata: each section has its own baked measurement provenance.
    let solver_meta = &solver_data().expect("gated: the Benchmarks shell loads the bundle first").metadata;
    let codec_meta = &codec_data().expect("gated: the Benchmarks shell loads the bundle first").metadata;
    let solver_json_url = format!("{}/blob/main/benchmarks/results/solvers.json", GITHUB_REPO);
    let codec_json_url = format!("{}/blob/main/benchmarks/results/latest-codec.json", GITHUB_REPO);
    let solver_date = solver_meta.date.get(..10).unwrap_or(solver_meta.date.as_str());
    let codec_date = codec_meta.date.get(..10).unwrap_or(codec_meta.date.as_str());
    let history_url = format!("{}/tree/main/benchmarks/results/history", GITHUB_REPO);
    let runsh_url = format!("{}/blob/main/benchmarks/run.sh", GITHUB_REPO);
    let bench_dir_url = format!("{}/tree/main/benchmarks", GITHUB_REPO);

    rsx! {
        PageHead {
            title: seo_pages::BENCHMARKS.title,
            description: seo_pages::BENCHMARKS.description,
            canonical_path: seo_pages::BENCHMARKS.canonical_path,
        }
        style { "{BENCHMARKS_STYLE}" }
        JsonLdMultiple { schemas }

        div { class: "bench-container",
            MainNav { active: ActivePage::Benchmarks, subtitle: Some("Performance") }

            // Hero + Overview
            section { class: "bench-hero", id: "overview",
                h1 { "Benchmarks" }
                if bench_tab() == BenchTab::Language {
                    p { "LOGOS compiles to Rust. Rust-level performance, English-level readability." }
                    div { class: "bench-pills",
                        a { class: "bench-pill", href: "{release_url}", target: "_blank",
                            strong { "v{data.metadata.logos_version}" }
                        }
                        a { class: "bench-pill", href: "{commit_url}", target: "_blank",
                            "{data.metadata.commit}"
                        }
                        span { class: "bench-pill", "{data.metadata.cpu}" }
                        span { class: "bench-pill", "{data.metadata.os}" }
                        span { class: "bench-pill", "{&data.metadata.date[..10]}" }
                        a { class: "bench-pill link", href: "{raw_json_url}", target: "_blank",
                            "Raw JSON"
                        }
                    }
                }
                if bench_tab() == BenchTab::Solver {
                    p { "Certified proofs in milliseconds \u{2014} the same pure-Rust prover that runs in your browser, with no Z3." }
                    div { class: "bench-pills",
                        if !solver_meta.cpu.is_empty() {
                            span { class: "bench-pill", "{solver_meta.cpu}" }
                        }
                        if !solver_meta.kissat.is_empty() {
                            span { class: "bench-pill", "Kissat {solver_meta.kissat}" }
                        }
                        if !solver_meta.date.is_empty() {
                            span { class: "bench-pill", "{solver_date}" }
                        }
                        a { class: "bench-pill link", href: "{solver_json_url}", target: "_blank",
                            "Raw JSON"
                        }
                    }
                }
                if bench_tab() == BenchTab::Codec {
                    p { "Smallest bytes on the wire \u{2014} the LOGOS codec, head-to-head against the industry serializers." }
                    div { class: "bench-pills",
                        if !codec_meta.cpu.is_empty() {
                            span { class: "bench-pill", "{codec_meta.cpu}" }
                        }
                        if !codec_meta.os.is_empty() {
                            span { class: "bench-pill", "{codec_meta.os}" }
                        }
                        if !codec_meta.date.is_empty() {
                            span { class: "bench-pill", "{codec_date}" }
                        }
                        a { class: "bench-pill link", href: "{codec_json_url}", target: "_blank",
                            "Raw JSON"
                        }
                    }
                }
            }

            // Top-level tabs: the three benchmark stories.
            nav { class: "bench-tabbar",
                for t in BenchTab::all() {
                    button {
                        key: "{t.label()}",
                        class: if bench_tab() == t { "bench-tabbar-btn active" } else { "bench-tabbar-btn" },
                        onclick: move |_| bench_tab.set(t),
                        "{t.label()}"
                    }
                }
            }

            // In-page anchor nav for the programming-language sub-sections.
            if bench_tab() == BenchTab::Language {
                nav { class: "bench-section-nav",
                    a { href: "#overview", "Overview" }
                    a { href: "#performance", "Performance" }
                    a { href: "#interpreter", "Interpreter" }
                    a { href: "#source", "Source Code" }
                    a { href: "#compilation", "Compilation" }
                    a { href: "#summary", "Summary" }
                    a { href: "#methodology", "Methodology" }
                }
            }

            if bench_tab() == BenchTab::Language {
                div { class: "bench-content",
                    // Summary cards — each eyebrow states what is being compared
                    div { class: "bench-summary",
                        div { class: "bench-summary-card",
                            div { class: "bench-summary-eyebrow", "LOGOS compiled vs C" }
                            div { class: "bench-summary-value cyan",
                                "{logos_vs_c:.2}x"
                                sup { "*" }
                            }
                            div { class: "bench-summary-label", "the speed of C (geomean)" }
                        }
                        div { class: "bench-summary-card",
                            div { class: "bench-summary-eyebrow", "Same algorithm as C" }
                            div { class: "bench-summary-value green", "{logos_apples:.2}x" }
                            div { class: "bench-summary-label", "the speed of C (geomean)" }
                        }
                        div { class: "bench-summary-card",
                            div { class: "bench-summary-eyebrow", "Interpreted LOGOS vs V8" }
                            div { class: "bench-summary-value purple", "{interp_speed:.2}x" }
                            div { class: "bench-summary-label", "the speed of V8 (geomean)" }
                        }
                        if aot_speed > 0.0 {
                            div { class: "bench-summary-card",
                                div { class: "bench-summary-eyebrow", "AOT-native vs V8" }
                                div { class: "bench-summary-value", style: "color:#00d4ff;", "{aot_speed:.2}x" }
                                div { class: "bench-summary-label", "the speed of V8, warm (geomean)" }
                            }
                        }
                    }

                    div { class: "bench-note",
                        "The headline covers all {data.benchmarks.len()} benchmarks. On {collapse_count} of them the LOGOS "
                        "compiler reduces the work itself, for example by folding a recursive function into a closed form, so "
                        "it runs a faster algorithm than the C version rather than just faster machine code. Those wins are "
                        "real, and the generated Rust for each is shown in the Source Code section below. The second number is "
                        "the geometric mean over the remaining {apples_count} benchmarks, where LOGOS and C compile the same "
                        "algorithm. Both numbers are \u{201c}x the speed of C\u{201d}, so higher is faster."
                    }

                    // Benchmark tabs (shared across performance, source, compilation)
                    div { class: "bench-tabs",
                        for (i, b) in data.benchmarks.iter().enumerate() {
                            button {
                                key: "{i}",
                                class: if active_bench() == i { "bench-tab active" } else { "bench-tab" },
                                onclick: move |_| {
                                    active_bench.set(i);
                                    stats_open.set(false);
                                    compile_detail_open.set(false);
                                    source_open.set([false; 10]);
                                    // Reset optimizations to all-on so the new benchmark
                                    // shows its cached Rust instantly (no compile on switch).
                                    opt_toggles.set(vec![true; logicaffeine_compile::optimization::REGISTRY.len()]);
                                    live_rust.set(None);
                                    compiling.set(false);
                                    base_fired.set(Vec::new());
                                    fired_now.set(Vec::new());
                                    preempted_now.set(Vec::new());
                                    base_preempted.set(Vec::new());
                                    base_dependencies.set(Vec::new());
                                    expanded.set(vec![true; logicaffeine_compile::optimization::REGISTRY.len()]);
                                },
                                "{b.name}"
                                if collapse_note(&b.id).is_some() {
                                    span { class: "bench-tab-badge", title: "Algorithm collapsed by the LOGOS compiler", "\u{26a1}" }
                                }
                            }
                        }
                    }

                    // =============== PERFORMANCE ===============
                    div { class: "bench-section", id: "performance",
                        div { class: "bench-section-title", "{bench.name}" }
                        div { class: "bench-section-desc",
                            "{bench.description} (n = {ref_size})"
                        }

                        if let Some(note) = collapse_note(&bench.id) {
                            div { class: "bench-callout",
                                span { class: "bench-callout-icon", "\u{26a1}" }
                                span { "{note}" }
                            }
                        }

                        div { class: "bench-chart-hint", "Wall-clock time at n = {ref_size} \u{2014} shorter bar is faster." }

                        div { class: "bench-chart",
                            for (tier, entries) in compiled_grouped.iter() {
                                div { class: "bench-tier-label", "{tier_label(tier)}" }
                                for (label, color, median, is_logos, is_timeout) in entries.iter() {
                                    {
                                        if *is_timeout {
                                            let timeout_str = ref_timeout.map(|t| format_timeout(*t)).unwrap_or_else(|| ">timeout".to_string());
                                            rsx! {
                                                div { class: "bench-bar-row",
                                                    div { class: "bench-bar-label", "{label}" }
                                                    div { class: "bench-bar-track",
                                                        div {
                                                            class: "bench-bar-fill",
                                                            style: "width: 100%; background: repeating-linear-gradient(45deg, {color}33, {color}33 10px, {color}11 10px, {color}11 20px);",
                                                        }
                                                    }
                                                    span { class: "bench-bar-time-outside", "{timeout_str}" }
                                                }
                                            }
                                        } else {
                                            let pct = (*median / compiled_max * 100.0).min(100.0);
                                            let time_str = format_time(*median);
                                            let show_inside = pct > 15.0;
                                            let bar_class = if *is_logos { "bench-bar-fill logos-highlight" } else { "bench-bar-fill" };
                                            rsx! {
                                                div { class: "bench-bar-row",
                                                    div { class: "bench-bar-label", "{label}" }
                                                    div { class: "bench-bar-track",
                                                        div {
                                                            class: "{bar_class}",
                                                            style: "width: {pct:.1}%; background: {color};",
                                                            if show_inside {
                                                                span { class: "bench-bar-time", "{time_str}" }
                                                            }
                                                        }
                                                    }
                                                    if !show_inside {
                                                        span { class: "bench-bar-time-outside", "{time_str}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            if !interpreted_flat.is_empty() {
                                div {
                                    style: "border-top: 1px solid rgba(255,255,255,0.08); margin: 16px 0 8px; padding-top: 8px; font-size: 10px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.5px; color: rgba(229,231,235,0.3);",
                                    "Interpreted"
                                }
                                for (label, color, median, is_logos, is_timeout) in interpreted_flat.iter() {
                                    {
                                        if *is_timeout {
                                            let timeout_str = ref_timeout.map(|t| format_timeout(*t)).unwrap_or_else(|| ">timeout".to_string());
                                            rsx! {
                                                div { class: "bench-bar-row",
                                                    div { class: "bench-bar-label", "{label}" }
                                                    div { class: "bench-bar-track",
                                                        div {
                                                            class: "bench-bar-fill",
                                                            style: "width: 100%; background: repeating-linear-gradient(45deg, {color}33, {color}33 10px, {color}11 10px, {color}11 20px);",
                                                        }
                                                    }
                                                    span { class: "bench-bar-time-outside", "{timeout_str}" }
                                                }
                                            }
                                        } else {
                                            let pct = (*median / interpreted_max * 100.0).min(100.0);
                                            let time_str = format_time(*median);
                                            let show_inside = pct > 15.0;
                                            let bar_class = if *is_logos { "bench-bar-fill logos-highlight" } else { "bench-bar-fill" };
                                            rsx! {
                                                div { class: "bench-bar-row",
                                                    div { class: "bench-bar-label", "{label}" }
                                                    div { class: "bench-bar-track",
                                                        div {
                                                            class: "{bar_class}",
                                                            style: "width: {pct:.1}%; background: {color};",
                                                            if show_inside {
                                                                span { class: "bench-bar-time", "{time_str}" }
                                                            }
                                                        }
                                                    }
                                                    if !show_inside {
                                                        span { class: "bench-bar-time-outside", "{time_str}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Scaling curve — time vs problem size across the benchmarked sizes
                        div {
                            class: "bench-section-desc",
                            style: "margin: 24px 0 6px; color: rgba(229,231,235,0.72); font-weight: 600;",
                            "Scaling \u{2014} time vs problem size (log\u{2013}log)"
                        }
                        {scaling_chart(bench, &data.languages)}

                        {complexity_panel(bench, &data.languages)}

                        // Peak memory — same style as the runtime bars (lights up after a MEASURE_MEM run)
                        if bench.memory.is_some() {
                            div {
                                class: "bench-section-desc",
                                style: "margin: 24px 0 6px; color: rgba(229,231,235,0.72); font-weight: 600;",
                                "Memory \u{2014} peak resident set size"
                            }
                            {memory_bar_chart(bench, &data.languages)}
                        }

                        // Binary size — compiled-artifact footprint (lights up after a size run)
                        if bench.binary_sizes.is_some() {
                            div {
                                class: "bench-section-desc",
                                style: "margin: 24px 0 6px; color: rgba(229,231,235,0.72); font-weight: 600;",
                                "Binary size \u{2014} compiled-artifact footprint"
                            }
                            {binary_size_bar_chart(bench, &data.languages)}
                            div { class: "bench-note", style: "margin-top:12px;",
                                "The size of the program you actually ship. C and C++ stay tiny because the runtime lives in the system libc; Rust and Go statically link their runtimes; LOGOS compiles to a compact self-contained binary in between. "
                                "Java\u{2019}s figure is its bytecode alone \u{2014} it still needs the JVM to run \u{2014} and JavaScript has no compiled artifact at all (its footprint is the V8 engine, in the Interpreter section). "
                                "As-built is the real shipped file; stripped removes debug symbols for a code-only comparison."
                            }
                        }

                        // Collapsible: Detailed Statistics
                        button {
                            class: "bench-collapsible-btn",
                            onclick: move |_| stats_open.set(!stats_open()),
                            span {
                                class: if stats_open() { "bench-collapsible-chevron open" } else { "bench-collapsible-chevron" },
                                "\u{25b6}"
                            }
                            "Detailed Statistics"
                        }
                        div {
                            class: if stats_open() { "bench-collapsible-body open" } else { "bench-collapsible-body" },
                            table { class: "bench-stats-table",
                                thead {
                                    tr {
                                        th { "Language" }
                                        th { "Mean" }
                                        th { "Median" }
                                        th { "StdDev" }
                                        th { "Min" }
                                        th { "Max" }
                                        th { "User" }
                                        th { "System" }
                                        th { "CV" }
                                        th { "Runs" }
                                    }
                                }
                                tbody {
                                    for (label, lid, maybe_t) in stats_entries.iter() {
                                        if let Some(t) = maybe_t {
                                            tr {
                                                class: if *lid == "logos_release" { "highlight" } else { "" },
                                                td { "{label}" }
                                                td { "{format_time(t.mean_ms)}" }
                                                td { "{format_time(t.median_ms)}" }
                                                td { "\u{00b1}{format_time(t.stddev_ms)}" }
                                                td { "{format_time(t.min_ms)}" }
                                                td { "{format_time(t.max_ms)}" }
                                                td { "{t.user_ms.map(|v| format_time(v)).unwrap_or_else(|| \"\u{2014}\".to_string())}" }
                                                td { "{t.system_ms.map(|v| format_time(v)).unwrap_or_else(|| \"\u{2014}\".to_string())}" }
                                                td { "{t.cv:.3}" }
                                                td { "{t.runs}" }
                                            }
                                        } else {
                                            {
                                                let timeout_str = ref_timeout.map(|t| format_timeout(*t)).unwrap_or_else(|| ">timeout".to_string());
                                                rsx! {
                                                    tr {
                                                        class: if *lid == "logos_release" { "highlight" } else { "" },
                                                        td { "{label}" }
                                                        td { "{timeout_str}" }
                                                        td { "{timeout_str}" }
                                                        td { "\u{2014}" }
                                                        td { "\u{2014}" }
                                                        td { "\u{2014}" }
                                                        td { "\u{2014}" }
                                                        td { "\u{2014}" }
                                                        td { "\u{2014}" }
                                                        td { "\u{2014}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // =============== INTERPRETER vs V8 ===============
                    div { class: "bench-section", id: "interpreter",
                        div { class: "bench-section-title", "LOGOS vs JavaScript / V8" }
                        div { class: "bench-section-desc",
                            "The LOGOS engine ladder \u{2014} bytecode VM, copy-and-patch JIT, and the warm AOT-native tier \u{2014} against {interp_node_ver} / V8."
                        }

                        div { class: "bench-summary", style: "margin-bottom: 20px;",
                            div { class: "bench-summary-card",
                                div { class: "bench-summary-value cyan", "{startup_vs_v8:.1}x" }
                                div { class: "bench-summary-label", "faster cold start than V8" }
                            }
                            div { class: "bench-summary-card",
                                div { class: "bench-summary-value", style: "color:#ff8c00;", "{startup_logos:.1}ms" }
                                div { class: "bench-summary-label", "LOGOS interpreter cold start" }
                            }
                            div { class: "bench-summary-card",
                                div { class: "bench-summary-value purple", "{interp_speed:.2}x" }
                                div { class: "bench-summary-label", "the speed of V8 on compute (geomean)" }
                            }
                            if aot_speed > 0.0 {
                                div { class: "bench-summary-card",
                                    div { class: "bench-summary-value", style: "color:#00d4ff;", "{aot_speed:.2}x" }
                                    div { class: "bench-summary-label", "the speed of V8, AOT-native warm (geomean)" }
                                }
                            }
                        }

                        // Cold-start chart (serverless / CLI win)
                        if !startup_bars.is_empty() {
                            div { class: "bench-chart-hint", style: "font-weight:600;color:rgba(229,231,235,0.72);margin-top:4px;",
                                "Cold start \u{2014} launch the engine and run a trivial program ({interp.startup.runs} runs, shorter is faster)"
                            }
                            div { class: "bench-chart",
                                for (label, color, mean, is_logos) in startup_bars.iter() {
                                    {
                                        let pct = (*mean / startup_max * 100.0).min(100.0);
                                        let time_str = format_time(*mean);
                                        let show_inside = pct > 18.0;
                                        let bar_class = if *is_logos { "bench-bar-fill logos-highlight" } else { "bench-bar-fill" };
                                        rsx! {
                                            div { class: "bench-bar-row",
                                                div { class: "bench-bar-label", "{label}" }
                                                div { class: "bench-bar-track",
                                                    div { class: "{bar_class}", style: "width: {pct:.1}%; background: {color};",
                                                        if show_inside { span { class: "bench-bar-time", "{time_str}" } }
                                                    }
                                                }
                                                if !show_inside { span { class: "bench-bar-time-outside", "{time_str}" } }
                                            }
                                        }
                                    }
                                }
                            }
                            div { class: "bench-note", style: "margin-top:14px;margin-bottom:0;",
                                "A native binary has no VM to warm up, so the LOGOS interpreter reaches first output in "
                                strong { "{startup_logos:.1}ms" }
                                " versus V8\u{2019}s {startup_node:.0}ms, about "
                                strong { "{startup_vs_v8:.1}x quicker" }
                                ", which is what matters for short-lived work like cloud functions, CLI tools, and scripts. "
                                "On long-running loops V8\u{2019}s optimizing JIT pulls ahead: the interpreter is competitive on "
                                "memory-bound work and behind on heavy compute, a geometric mean of {interp_speed:.2}x the "
                                "speed of V8 across {interp_offfloor} benchmarks."
                            }
                        }

                        // Engine size — what you ship to run a program (benchmark-independent, like cold start)
                        if !engine_size_bars.is_empty() {
                            div { class: "bench-chart-hint", style: "font-weight:600;color:rgba(229,231,235,0.72);margin-top:22px;",
                                "Engine size \u{2014} the runtime you ship to execute a program (shorter ships less; stripped, code-only size in parentheses)"
                            }
                            div { class: "bench-chart",
                                for (label, color, bytes, stripped, is_logos) in engine_size_bars.iter() {
                                    {
                                        let pct = (*bytes / engine_size_max * 100.0).min(100.0);
                                        let s = footprint_label(*bytes, *stripped);
                                        let show_inside = pct > 32.0;
                                        let bar_class = if *is_logos { "bench-bar-fill logos-highlight" } else { "bench-bar-fill" };
                                        rsx! {
                                            div { class: "bench-bar-row",
                                                div { class: "bench-bar-label", "{label}" }
                                                div { class: "bench-bar-track",
                                                    div { class: "{bar_class}", style: "width: {pct:.1}%; background: {color};",
                                                        if show_inside { span { class: "bench-bar-time", "{s}" } }
                                                    }
                                                }
                                                if !show_inside { span { class: "bench-bar-time-outside", "{s}" } }
                                            }
                                        }
                                    }
                                }
                            }
                            div { class: "bench-note", style: "margin-top:14px;",
                                if !wasm_bundle_str.is_empty() {
                                    "In the browser the whole LOGOS engine ships as a "
                                    strong { "{wasm_bundle_str}" }
                                    " WebAssembly bundle \u{2014} the same VM+JIT, no native install. "
                                }
                                "Node\u{2019}s binary bundles V8, libuv, and ICU; largo bundles the transpiler, bytecode VM, and copy-and-patch JIT \u{2014} each is the whole engine you ship to run a program. As-built is the real file; stripped removes debug symbols."
                            }
                        }

                        div { class: "bench-chart-hint", style: "font-weight:600;color:rgba(229,231,235,0.72);margin-top:22px;",
                            "{bench.name} (engine: {interp_engine})"
                        }

                        if bench.id == "ackermann" {
                            div { class: "bench-callout",
                                span { class: "bench-callout-icon", "\u{26a1}" }
                                span { "Interpreted recursion is capped at the shared MAX_CALL_DEPTH (2,500 frames); ackermann\u{2019}s deep self-recursion blows past it, so the interpreter runs it at a reduced m. Deep recursion only completes in compiled mode, where the optimizer collapses it." }
                            }
                        }

                        if interp_bars.is_empty() {
                            div { class: "bench-chart-hint", "No interpreter result for {bench.name} (skipped, or not yet supported by the interpreter)." }
                        } else {
                            div { class: "bench-chart-hint",
                                "Wall-clock time at n = {interp_n} \u{2014} shorter is faster."
                                if let Some(s) = interp_active_speed {
                                    " The interpreter runs at {s:.2}x the speed of V8 here."
                                }
                            }
                            div { class: "bench-chart",
                                for (label, color, median, is_logos, _floored) in interp_bars.iter() {
                                    {
                                        let pct = (*median / interp_max * 100.0).min(100.0);
                                        let time_str = format_time(*median);
                                        let show_inside = pct > 15.0;
                                        let bar_class = if *is_logos { "bench-bar-fill logos-highlight" } else { "bench-bar-fill" };
                                        rsx! {
                                            div { class: "bench-bar-row",
                                                div { class: "bench-bar-label", "{label}" }
                                                div { class: "bench-bar-track",
                                                    div {
                                                        class: "{bar_class}",
                                                        style: "width: {pct:.1}%; background: {color};",
                                                        if show_inside {
                                                            span { class: "bench-bar-time", "{time_str}" }
                                                        }
                                                    }
                                                }
                                                if !show_inside {
                                                    span { class: "bench-bar-time-outside", "{time_str}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // =============== SOURCE CODE ===============
                    div { class: "bench-section", id: "source",
                        div { class: "bench-section-title", "Source Code" }
                        div { class: "bench-section-desc",
                            "The LOGOS source and the Rust it compiles to. Switch an optimization off and watch its \u{201c}## No <X>\u{201d} decorator appear on the LOGOS source — and the generated Rust recompile live in your browser. With every optimization on you see the cached release build; disabling them all yields plain, boring Rust."
                        }

                        // Master switch: all optimizations on (cached release Rust) ↔ all
                        // off (plain, un-optimized Rust). Both are instant.
                        div { class: "bench-opt-master",
                            label {
                                class: if all_opts_on { "bench-opt-toggle on" } else { "bench-opt-toggle off" },
                                input {
                                    r#type: "checkbox",
                                    checked: all_opts_on,
                                    onchange: move |_| {
                                        let n = logicaffeine_compile::optimization::REGISTRY.len();
                                        let v = if opt_toggles().iter().all(|&b| b) { vec![false; n] } else { vec![true; n] };
                                        opt_toggles.set(v);
                                        live_rust.set(None);
                                    },
                                }
                                span { "All Optimizations" }
                            }
                            span { class: "bench-opt-hint",
                                if base_fired().is_empty() {
                                    "analyzing which optimizations this program uses\u{2026}"
                                } else {
                                    "{base_fired().len()} of {logicaffeine_compile::optimization::REGISTRY.len()} optimizations fire for this program"
                                }
                            }
                        }

                        // The optimization graph for this program, as a collapsible tree:
                        // every opt that fires, the `requires`-parents they depend on
                        // (an "enabler" that does not itself fire, dashed), and the opts
                        // that were SKIPPED because a higher-precedence one claimed them
                        // (a greyed "preempted" node — it fires if its winner is turned
                        // off). Nested by `requires` depth; turning a parent off cascades
                        // its children off, turning a child on pulls its parents on (the
                        // registry's own rule, via the compiler's `normalize`). Cyan =
                        // firing right now.
                        div { class: "bench-opt-toggles",
                            for (node, visible) in opt_tree.iter().zip(tree_visible.iter().copied()) {
                                if visible {
                                    {
                                        use logicaffeine_compile::optimization::OptRole;
                                        let opt = node.opt;
                                        let ri = opt as usize;
                                        let m = &logicaffeine_compile::optimization::REGISTRY[ri];
                                        let firing = fired_now().contains(&m.keyword);
                                        let needs = node.requires.iter().map(|o| o.meta().keyword).collect::<Vec<_>>().join(", ");
                                        let depends = node.depends_on.iter().map(|o| o.meta().keyword).collect::<Vec<_>>().join(", ");
                                        let blocks = node.preempts.iter().map(|o| o.meta().keyword).collect::<Vec<_>>().join(", ");
                                        let blocked_by = node.preempted_by.iter().map(|o| o.meta().keyword).collect::<Vec<_>>().join(", ");
                                        let blocks_now = preempted_now().iter()
                                            .filter(|(w, _)| *w == m.keyword)
                                            .map(|(_, l)| *l)
                                            .collect::<Vec<_>>()
                                            .join(", ");
                                        let cls = if !opt_tog[ri] { "bench-opt-toggle off" }
                                                  else if firing { "bench-opt-toggle on firing" }
                                                  else { match node.role {
                                                      OptRole::Preempted => "bench-opt-toggle on preempted",
                                                      OptRole::Enabler => "bench-opt-toggle on enabling",
                                                      OptRole::Fired => "bench-opt-toggle on",
                                                  } };
                                        let row_style = format!("padding-left: {}px;", node.depth * 22);
                                        let has_children = node.has_children;
                                        let is_expanded = expanded().get(ri).copied().unwrap_or(true);
                                        let chevron_cls = if is_expanded { "bench-tree-chevron open" } else { "bench-tree-chevron" };
                                        rsx! {
                                            div { class: "bench-tree-row", style: "{row_style}",
                                                if has_children {
                                                    span {
                                                        class: "{chevron_cls}",
                                                        onclick: move |_| {
                                                            let mut e = expanded();
                                                            if ri < e.len() { e[ri] = !e[ri]; }
                                                            expanded.set(e);
                                                        },
                                                        "\u{25b6}"
                                                    }
                                                } else {
                                                    span { class: "bench-tree-spacer" }
                                                }
                                                label { class: "{cls}", title: "{m.group}",
                                                    input { r#type: "checkbox", checked: opt_tog[ri],
                                                        onchange: move |_| {
                                                            let toggles = opt_toggles();
                                                            let turning_on = !toggles.get(ri).copied().unwrap_or(false);
                                                            let mut cfg = config_from_toggles(&toggles);
                                                            if turning_on {
                                                                cfg.enable_with_requires(opt);
                                                            } else {
                                                                cfg.set(opt, false);
                                                            }
                                                            cfg.normalize();
                                                            opt_toggles.set(toggles_from_config(&cfg));
                                                            live_rust.set(None);
                                                        },
                                                    }
                                                    span { "{m.label}" }
                                                    if node.role == OptRole::Enabler {
                                                        span { class: "bench-opt-rel enables", "enabler" }
                                                    }
                                                    if node.role == OptRole::Preempted && !blocked_by.is_empty() {
                                                        span { class: "bench-opt-rel beaten", "blocked by {blocked_by} \u{00b7} fires if off" }
                                                    }
                                                    if !needs.is_empty() {
                                                        span { class: "bench-opt-rel needs", "needs {needs}" }
                                                    }
                                                    if !depends.is_empty() {
                                                        span { class: "bench-opt-rel depends", "depends on {depends}" }
                                                    }
                                                    if !blocks.is_empty() {
                                                        span { class: "bench-opt-rel beats", "blocks {blocks}" }
                                                    }
                                                    if !blocks_now.is_empty() {
                                                        span { class: "bench-opt-rel beats-now", "blocks {blocks_now} now" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // LOGOS (with the toggles applied) + Generated Rust side-by-side.
                        // All-on shows the cached release Rust instantly; a toggle off
                        // shows a spinner while the browser recompiles.
                        div { class: "bench-source",
                            div { class: "bench-source-panel",
                                div { class: "bench-source-header logos", "LOGOS" }
                                div { class: "bench-source-code", "{opt_decorated}" }
                            }
                            div { class: "bench-source-panel",
                                div { class: "bench-source-header rust", "Generated Rust" }
                                if rust_loading {
                                    div { class: "bench-source-code",
                                        span { class: "bench-compiling", "Compiling\u{2026}" }
                                    }
                                } else {
                                    div { class: "bench-source-code", "{rust_text}" }
                                }
                            }
                        }

                        // Collapsible language source sections
                        for (idx, lang_id) in source_langs.iter().enumerate() {
                            {
                                let src = get_source(bench_sources, lang_id);
                                let color = lang_color(lang_id);
                                let label = lang_label(lang_id);
                                let version = data.metadata.versions.get(*lang_id).map(|s| s.as_str()).unwrap_or("");
                                let ext = lang_ext(lang_id);
                                let file_url = format!("{}/blob/main/benchmarks/programs/{}/{}", GITHUB_REPO, bench.id, ext);
                                let is_open = source_open()[idx];
                                rsx! {
                                    div { class: "bench-lang-collapsible",
                                        div {
                                            class: "bench-lang-header",
                                            onclick: move |_| {
                                                let mut arr = source_open();
                                                arr[idx] = !arr[idx];
                                                source_open.set(arr);
                                            },
                                            span {
                                                class: "bench-lang-dot",
                                                style: "background: {color};",
                                            }
                                            span { class: "bench-lang-name", "{label}" }
                                            span { class: "bench-lang-version", "{version}" }
                                            a {
                                                class: "bench-lang-link",
                                                href: "{file_url}",
                                                target: "_blank",
                                                onclick: move |e: Event<MouseData>| e.stop_propagation(),
                                                "View on GitHub \u{2192}"
                                            }
                                            span {
                                                class: if is_open { "bench-collapsible-chevron open" } else { "bench-collapsible-chevron" },
                                                "\u{25b6}"
                                            }
                                        }
                                        div {
                                            class: if is_open { "bench-lang-code open" } else { "bench-lang-code" },
                                            "{src}"
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // =============== COMPILATION ===============
                    div { class: "bench-section", id: "compilation",
                        div { class: "bench-section-title", "Compilation Times" }
                        div { class: "bench-section-desc",
                            "Time to compile each benchmark from source, at the same flags used for the runtime numbers. "
                            "LOGOS compiles English to Rust, then invokes rustc at full optimization (-O3, fat LTO, "
                            "target-cpu=native) \u{2014} so its bar includes the Rust compile."
                        }

                        div { class: "bench-chart",
                            for (name, mean, _stddev, is_largo) in compile_entries.iter() {
                                {
                                    let pct = (*mean / compile_max * 100.0).min(100.0);
                                    let time_str = format_time(*mean);
                                    let show_inside = pct > 15.0;
                                    let bar_class = if *is_largo { "bench-bar-fill logos-highlight" } else { "bench-bar-fill" };
                                    let color = if *is_largo { "#00d4ff" } else { "#6b7280" };
                                    let display_name = compiler_label(name);
                                    rsx! {
                                        div { class: "bench-bar-row",
                                            div { class: "bench-bar-label", "{display_name}" }
                                            div { class: "bench-bar-track",
                                                div {
                                                    class: "{bar_class}",
                                                    style: "width: {pct:.1}%; background: {color};",
                                                    if show_inside {
                                                        span { class: "bench-bar-time", "{time_str}" }
                                                    }
                                                }
                                            }
                                            if !show_inside {
                                                span { class: "bench-bar-time-outside", "{time_str}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Collapsible: Detailed Compilation Data
                        button {
                            class: "bench-collapsible-btn",
                            onclick: move |_| compile_detail_open.set(!compile_detail_open()),
                            span {
                                class: if compile_detail_open() { "bench-collapsible-chevron open" } else { "bench-collapsible-chevron" },
                                "\u{25b6}"
                            }
                            "Detailed Compilation Data"
                        }
                        div {
                            class: if compile_detail_open() { "bench-collapsible-body open" } else { "bench-collapsible-body" },
                            table { class: "bench-compile-table",
                                thead {
                                    tr {
                                        th { "Compiler" }
                                        th { "Mean" }
                                        th { "StdDev" }
                                    }
                                }
                                tbody {
                                    for (name, mean, stddev, is_largo) in compile_entries.iter() {
                                        tr {
                                            class: if *is_largo { "highlight" } else { "" },
                                            td { class: "compiler-name", "{compiler_label(name)}" }
                                            td { "{format_time(*mean)}" }
                                            td { "\u{00b1}{format_time(*stddev)}" }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // =============== SUMMARY ===============
                    div { class: "bench-section", id: "summary",
                        div { class: "bench-section-title", "Cross-Benchmark Summary" }
                        div { class: "bench-section-desc", "Geometric-mean speed vs C across all {data.benchmarks.len()} benchmarks (bars proportional to speed; higher is faster)." }

                        div { class: "bench-chart",
                            for (label, val, color, is_logos) in summary_entries.iter() {
                                {
                                    // Linear and proportional: the fastest fills the track and every
                                    // other bar is its true fraction — 2.5x reads as ~2x the 1.21x bar.
                                    let pct = linear_bar_pct(*val, summary_max);

                                    let display = if *val >= 0.01 {
                                        format!("{:.2}x", val)
                                    } else {
                                        format!("{:.4}x", val)
                                    };
                                    let show_inside = pct > 20.0;
                                    let bar_class = if *is_logos { "bench-bar-fill logos-highlight" } else { "bench-bar-fill" };
                                    rsx! {
                                        div { class: "bench-bar-row",
                                            div { class: "bench-bar-label", "{label}" }
                                            div { class: "bench-bar-track",
                                                div {
                                                    class: "{bar_class}",
                                                    style: "width: {pct:.1}%; background: {color};",
                                                    if show_inside {
                                                        span { class: "bench-bar-time", "{display}" }
                                                    }
                                                }
                                            }
                                            if !show_inside {
                                                span { class: "bench-bar-time-outside", "{display}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // =============== METHODOLOGY ===============
                    div { class: "bench-section", id: "methodology",
                        button {
                            class: "bench-collapsible-btn",
                            style: "margin-top: 0; border-top: none; padding-top: 0;",
                            onclick: move |_| methodology_open.set(!methodology_open()),
                            span {
                                class: if methodology_open() { "bench-collapsible-chevron open" } else { "bench-collapsible-chevron" },
                                "\u{25b6}"
                            }
                            span { style: "font-size: 18px; font-weight: 700; color: #fff;",
                                "Methodology"
                            }
                        }
                        div {
                            class: if methodology_open() { "bench-collapsible-body open" } else { "bench-collapsible-body" },
                            div { class: "bench-methodology",
                                ul {
                                    li { "Each benchmark measured with hyperfine ({data.metadata.runs.unwrap_or(20)} runs, {data.metadata.warmup.unwrap_or(5)} warmup); the bars show the median." }
                                    li { "CPU: {data.metadata.cpu}." }
                                    li { "OS: {data.metadata.os}." }
                                    li { "Every implementation runs the same algorithm and produces identical, verified output." }
                                    li { "All compiled languages are built at full, matched optimization (see flags below) \u{2014} no language is handicapped relative to LOGOS." }
                                    li { "Two geometric means are reported vs C: the headline keeps all {data.benchmarks.len()} benchmarks; the apples-to-apples figure removes the {collapse_count} where the LOGOS compiler collapses the algorithm (\u{26a1}). Every collapse is auditable in the generated Rust per benchmark." }
                                    li { "The Interpreter-vs-V8 section is a separate peer comparison (LOGOS bytecode VM + JIT against Node/V8) at interpreter-calibrated sizes, so its n differs from the compiled section." }
                                }

                                h3 { "Compiler Versions" }
                                table { class: "bench-version-table",
                                    thead {
                                        tr {
                                            th { "Language" }
                                            th { "Version" }
                                        }
                                    }
                                    tbody {
                                        for (lang_id, version) in data.metadata.versions.iter() {
                                            tr {
                                                td { "{lang_label(lang_id)}" }
                                                td { "{version}" }
                                            }
                                        }
                                    }
                                }

                                h3 { "Compiler Flags" }
                                table { class: "bench-version-table",
                                    thead {
                                        tr {
                                            th { "Language" }
                                            th { "Flags" }
                                        }
                                    }
                                    tbody {
                                        tr { td { "C" } td { "gcc -O3 -march=native -flto -lm" } }
                                        tr { td { "C++" } td { "g++ -O3 -march=native -flto -std=c++17" } }
                                        tr { td { "Rust" } td { "rustc --edition 2021 -C opt-level=3 -C lto=fat -C codegen-units=1 -C target-cpu=native" } }
                                        tr { td { "Zig" } td { "zig build-exe -O ReleaseFast -mcpu native" } }
                                        tr { td { "Go" } td { "go build (Go has no -O levels; this is its optimizing release build)" } }
                                        tr { td { "Java" } td { "javac, run on the HotSpot JIT" } }
                                        tr { td { "Nim" } td { "nim c -d:release --passC:\"-O3 -march=native\"" } }
                                        tr { td { "JavaScript" } td { "node (V8 JIT)" } }
                                        tr { td { "LOGOS" } td { "largo build --release \u{2192} generated Rust \u{2192} rustc -C opt-level=3 -C lto=fat -C codegen-units=1 -C target-cpu=native" } }
                                        tr { td { "LOGOS (interpreted)" } td { "largo run --interpret (bytecode VM + copy-and-patch JIT)" } }
                                    }
                                }

                                h3 { "Links" }
                                ul {
                                    li {
                                        a { href: "{runsh_url}", target: "_blank", "benchmarks/run.sh" }
                                        " \u{2014} benchmark runner script"
                                    }
                                    li {
                                        a { href: "{raw_json_url}", target: "_blank", "results/latest.json" }
                                        " \u{2014} raw benchmark data"
                                    }
                                    li {
                                        a { href: "{history_url}", target: "_blank", "results/history/" }
                                        " \u{2014} historical results by version"
                                    }
                                    li {
                                        a { href: "{bench_dir_url}", target: "_blank", "benchmarks/" }
                                        " \u{2014} all benchmark source code"
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if bench_tab() == BenchTab::Solver {
                div { class: "bench-content",
                    SolverSection {}
                }
            }
            if bench_tab() == BenchTab::Codec {
                div { class: "bench-content",
                    CodecSection {}
                }
            }

            Footer {}
        }
    }
}
