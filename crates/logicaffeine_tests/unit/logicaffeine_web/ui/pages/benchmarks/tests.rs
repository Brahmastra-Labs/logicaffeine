use super::*;

#[test]
fn empirical_exponent_fits_linear() {
    let pts: Vec<(f64, f64)> = [10.0, 100.0, 1000.0, 10000.0]
        .iter()
        .map(|&n| (n, 3.0 * n))
        .collect();
    let b = empirical_exponent(&pts).expect("two+ points");
    assert!(
        (b - 1.0).abs() < 0.01,
        "linear data → exponent ~1.0, got {b}"
    );
}

#[test]
fn empirical_exponent_fits_quadratic() {
    let pts: Vec<(f64, f64)> = [10.0, 100.0, 1000.0]
        .iter()
        .map(|&n| (n, 2.0 * n * n))
        .collect();
    let b = empirical_exponent(&pts).expect("two+ points");
    assert!(
        (b - 2.0).abs() < 0.01,
        "quadratic data → exponent ~2.0, got {b}"
    );
}

#[test]
fn empirical_exponent_degenerate_is_none() {
    assert!(
        empirical_exponent(&[(10.0, 5.0)]).is_none(),
        "one point → None"
    );
    assert!(empirical_exponent(&[]).is_none(), "no points → None");
    assert!(
        empirical_exponent(&[(5.0, 1.0), (5.0, 9.0)]).is_none(),
        "all-equal n → None"
    );
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
    assert!(
        (rust - 47.56).abs() < 0.1,
        "1.21/2.544 → ~47.6 %, got {rust}"
    );
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
            for o in r
                .others
                .iter()
                .filter(|o| o.status == "unsat" || o.status == "sat")
            {
                let theirs = linear_bar_pct(o.ms, scale);
                // Faster ⇒ shorter bar.
                assert!(
                    ours <= theirs,
                    "[{}] n={}: ours {}ms bar {ours} must not exceed {} {}ms bar {theirs}",
                    fam.id,
                    r.n,
                    r.ours_ms,
                    o.solver,
                    o.ms
                );
                // And proportional (both above the sliver floor): bar ratio == time ratio.
                if ours > 0.5 && theirs > 0.5 {
                    let bar_ratio = theirs / ours;
                    let time_ratio = o.ms / r.ours_ms;
                    assert!(
                        (bar_ratio - time_ratio).abs() / time_ratio < 1e-6,
                        "[{}] n={}: bar ratio {bar_ratio} must equal time ratio {time_ratio}",
                        fam.id,
                        r.n
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
        let best = s
            .rows
            .iter()
            .filter(|r| is_logos_codec(&r.codec))
            .map(|r| r.size)
            .min();
        let json = s.rows.iter().find(|r| r.codec == "json").map(|r| r.size);
        if let (Some(best), Some(json)) = (best, json) {
            if best < json {
                let pb = linear_bar_pct(best as f64, vmax);
                let pj = linear_bar_pct(json as f64, vmax);
                assert!(
                    pb < pj,
                    "[{}] logos {best}B bar must be shorter than json {json}B",
                    s.id
                );
                assert!(
                    (pj / pb - json as f64 / best as f64).abs() < 1e-6,
                    "[{}] codec bar ratio must equal byte ratio",
                    s.id
                );
            }
        }
    }
}

#[test]
fn solver_data_loads_and_we_win_where_claimed() {
    let d = &*SOLVER_DATA; // panics if the baked solvers.json is malformed
    assert!(
        d.families.len() >= 3,
        "solver section must ship multiple families"
    );
    // The pigeonhole headline must carry measured Z3 AND Kissat timeouts — the resolution wall.
    let php = d
        .families
        .iter()
        .find(|f| f.id == "php")
        .expect("php family present");
    let walls = php.rows.iter().any(|r| {
        r.others
            .iter()
            .any(|o| o.solver == "z3" && o.status == "timeout")
            && r.others
                .iter()
                .any(|o| o.solver == "kissat" && o.status == "timeout")
    });
    assert!(
        walls,
        "PHP must show Z3 AND Kissat hitting the resolution wall"
    );
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
    let zeros = [
        "0", "0 µs", "0.0 µs", "0.00 µs", "0.0 ms", "0.00 ms", "0.000 ms",
    ];
    let mut ms = 5_000.0_f64;
    while ms > 1e-7 {
        let s = fmt_solver_ms(ms);
        assert!(
            s.contains("µs") || s.contains("ms") || s.contains(" s"),
            "no unit for {ms}: {s}"
        );
        assert!(
            !zeros.contains(&s.as_str()),
            "ms={ms} rendered as zero: {s}"
        );
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
            assert!(
                !zeros.contains(&s.as_str()),
                "[{}] n={} ours renders as zero: {s}",
                fam.id,
                r.n
            );
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
    assert_eq!(
        d.summary.geometric_mean_logos_tiered_over_node, 0.0,
        "missing tiered geo defaults to 0"
    );
    assert!(
        d.interpreter_sizes.is_none(),
        "old interp JSON has no interpreter_sizes"
    );

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
    assert_eq!(
        d.summary.geometric_mean_logos_aot_over_node, 0.0,
        "missing AOT geo defaults to 0"
    );
    let scaling = &d.benchmarks[0].scaling["30"];
    assert!(
        scaling.contains_key("logos_tiered"),
        "tiered engine row present"
    );
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
    assert!(
        scaling.contains_key("logos_aot"),
        "AOT engine row present when bundled"
    );
    assert!((scaling["logos_aot"].mean_ms - 0.6).abs() < 1e-9);
}

// ── Footprint metrics: the size data the benchmarks page now surfaces. These
// assert against the REAL baked-in result files, so they go RED until a
// `benchmarks/measure-sizes.sh --merge` backfills the JSON, then stay GREEN and
// guard the contract on every future run.

#[test]
fn every_benchmark_carries_binary_sizes() {
    for b in &BENCH_DATA.benchmarks {
        let sizes = b
            .binary_sizes
            .as_ref()
            .unwrap_or_else(|| panic!("benchmark {} is missing binary_sizes", b.id));
        assert!(
            !sizes.by_language.is_empty(),
            "benchmark {} has empty binary_sizes",
            b.id
        );
        assert!(
            sizes.by_language.contains_key("logos_release"),
            "benchmark {} binary_sizes is missing the logos_release artifact",
            b.id
        );
        for (lang, s) in &sizes.by_language {
            assert!(
                s.as_built > 0.0,
                "{}/{} as_built must be > 0, got {}",
                b.id,
                lang,
                s.as_built
            );
            if let Some(st) = s.stripped {
                assert!(
                    st > 0.0 && st <= s.as_built,
                    "{}/{} stripped ({st}) must be in (0, as_built={}]",
                    b.id,
                    lang,
                    s.as_built
                );
            }
        }
    }
}

#[test]
fn interpreter_sizes_cover_logos_and_node() {
    let sizes = INTERP_DATA
        .interpreter_sizes
        .as_ref()
        .expect("latest-interp.json is missing interpreter_sizes");
    for id in ["logos", "node"] {
        let e = sizes
            .engines
            .get(id)
            .unwrap_or_else(|| panic!("interpreter_sizes is missing the {id} engine"));
        assert!(
            e.as_built > 0.0,
            "engine {id} as_built must be > 0, got {}",
            e.as_built
        );
        if let Some(st) = e.stripped {
            assert!(
                st > 0.0 && st <= e.as_built,
                "engine {id} stripped ({st}) out of range"
            );
        }
    }
    if let Some(w) = sizes.wasm_bundle_bytes {
        assert!(
            w > 0.0,
            "wasm_bundle_bytes must be > 0 when present, got {w}"
        );
    }
}

#[test]
fn codec_data_deserializes_and_has_fair_scenarios() {
    let d = &*CODEC_DATA;
    assert_eq!(
        d.schema_version, 2,
        "schema_version is the page's compatibility gate"
    );
    let fair: Vec<_> = d.scenarios.iter().filter(|s| s.kind == "fair").collect();
    assert!(
        !fair.is_empty(),
        "latest-codec.json must contain fair scenarios"
    );
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
    assert_eq!(
        wins, total,
        "the baked data must back the smallest-wire-on-all-workloads headline"
    );
}

#[test]
fn codec_scorecard_fair_axes_all_win() {
    // The scorecard claims "fastest encode N/N" and "fastest decode N/N" — the baked data
    // must back both (our fastest dial ≤ every competitor on every fair workload).
    let (enc, total) = codec_axis_wins(&CODEC_DATA, CodecAxis::Encode);
    let (dec, _) = codec_axis_wins(&CODEC_DATA, CodecAxis::Decode);
    assert!(total > 0);
    assert_eq!(
        enc, total,
        "scorecard claims fastest encode on all fair workloads"
    );
    assert_eq!(
        dec, total,
        "scorecard claims fastest decode on all fair workloads"
    );
    // The geo-mean size advantage and top-showcase ratio are real wins (> 1×).
    assert!(
        codec_geomean_size(&CODEC_DATA) > 1.0,
        "geo-mean size advantage must exceed 1×"
    );
    assert!(
        codec_top_showcase(&CODEC_DATA).is_some_and(|r| r > 10.0),
        "a showcase must crush by >10×"
    );
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
    assert_eq!(
        footprint_label(2_033_384.0, Some(307_280.0)),
        "1.9 MB (300 KB stripped)"
    );
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
    assert_eq!(
        shipped_footprint_label(2_033_384.0, Some(307_280.0)),
        "300 KB (1.9 MB as-built)"
    );
    // LOGOS already ships stripped (as-built ≈ stripped): the as-built figure is ALWAYS
    // shown for parity with every other row — the two numbers being equal is the story.
    assert_eq!(shipped_bytes(334_592.0, Some(334_584.0)), 334_584.0);
    assert_eq!(
        shipped_footprint_label(334_592.0, Some(334_584.0)),
        "327 KB (327 KB as-built)"
    );
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
        let Some(sizes) = b.binary_sizes.as_ref() else {
            continue;
        };
        for (lang, s) in sizes.by_language.iter() {
            let shipped = shipped_bytes(s.as_built, s.stripped);
            assert!(
                shipped > 0.0 && shipped <= s.as_built,
                "{}/{lang}: shipped {shipped} must be in (0, as_built={}]",
                b.id,
                s.as_built
            );
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
        assert!(
            codec_good_for(&s.id).is_some(),
            "scenario '{}' has no codec_good_for copy",
            s.id
        );
    }
}

#[test]
fn every_fair_best_row_names_the_dials_that_won() {
    // The winner card is driven by the BEST row's `chosen`; every fair example must carry it
    // with a non-empty summary + columns, or the card would fall back to opaque "all knobs".
    for s in CODEC_DATA.scenarios.iter().filter(|s| s.kind == "fair") {
        let best =
            codec_best_row(s).unwrap_or_else(|| panic!("fair '{}' missing the BEST row", s.id));
        let chosen = best
            .chosen
            .as_ref()
            .unwrap_or_else(|| panic!("fair '{}' BEST row missing chosen", s.id));
        assert!(
            !chosen.summary.trim().is_empty(),
            "fair '{}' chosen.summary is empty",
            s.id
        );
        assert!(
            !chosen.columns.is_empty(),
            "fair '{}' chosen.columns is empty",
            s.id
        );
    }
}

#[test]
fn every_fair_scenario_has_a_fair_fight() {
    // The honest compressed-vs-compressed comparison must be computable on every fair workload —
    // i.e. the competitors carry a `fair_size`, so the scorecard's "fair fight" count is real.
    for s in CODEC_DATA.scenarios.iter().filter(|s| s.kind == "fair") {
        let (lo, co, _) = codec_fair_fight(s).unwrap_or_else(|| {
            panic!(
                "fair '{}' has no fair-fight (competitors missing fair_size)",
                s.id
            )
        });
        assert!(
            lo > 0 && co > 0,
            "fair '{}' fair-fight sizes must be positive",
            s.id
        );
    }
    // And the scorecard's fair-fight count is honest: LOGOS does not have to win all of them.
    let (wins, total) = codec_fair_size_wins(&CODEC_DATA);
    assert!(
        total > 0 && wins <= total,
        "fair-fight count must be a valid n/{total}"
    );
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
            ("c", s.c),
            ("cpp", s.cpp),
            ("rust", s.rust),
            ("zig", s.zig),
            ("go", s.go),
            ("java", s.java),
            ("js", s.js),
            ("python", s.python),
            ("ruby", s.ruby),
            ("nim", s.nim),
            ("logos", s.logos),
        ] {
            assert!(
                !src.trim().is_empty(),
                "benchmark '{}' source '{lang}' is empty",
                b.id
            );
        }
    }
}

fn lang(id: &str, tier: &str) -> Language {
    Language {
        id: id.to_string(),
        label: id.to_uppercase(),
        color: "#000000".to_string(),
        tier: tier.to_string(),
    }
}

fn timed(ms: f64) -> TimingResult {
    TimingResult {
        mean_ms: ms,
        median_ms: ms,
        stddev_ms: 0.0,
        min_ms: ms,
        max_ms: ms,
        cv: 0.0,
        runs: 10,
        user_ms: None,
        system_ms: None,
        times_ms: vec![ms; 10],
    }
}

#[test]
fn the_logos_family_is_the_logos_tier() {
    assert!(lang("logos_c", "logos").is_logos());
    assert!(!lang("rust", "systems").is_logos());
}

/// The LOGOS family leads in registry order however fast each tier ran, the competition follows by kind fastest first,
/// JavaScript is on the chart beside the JIT, and a tier the page has never seen is drawn under `other`, not dropped.
#[test]
fn chart_groups_lead_with_the_logos_family_in_registry_order() {
    let langs = vec![
        lang("c", "systems"),
        lang("cpp", "systems"),
        lang("rust", "systems"),
        lang("logos_release", "logos"),
        lang("logos_c", "logos"),
        lang("logos_jit", "logos"),
        lang("go", "managed"),
        lang("js", "managed"),
        lang("nim", "transpiled"),
        lang("mystery", "quantum"),
    ];
    let timings: HashMap<String, TimingResult> = [
        ("c", 10.0),
        ("cpp", 9.0),
        ("rust", 12.0),
        ("logos_release", 8.0),
        ("logos_c", 7.0),
        ("logos_jit", 30.0),
        ("go", 20.0),
        ("js", 15.0),
        ("nim", 11.0),
        ("mystery", 1.0),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), timed(v)))
    .collect();
    let groups = chart_groups(&langs, Some(&timings), None, None);
    let shape: Vec<(String, Vec<String>)> = groups
        .iter()
        .map(|g| {
            (
                g.tier.clone(),
                g.bars.iter().map(|b| b.id.clone()).collect(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            (
                "logos".to_string(),
                vec!["logos_release".into(), "logos_c".into(), "logos_jit".into()]
            ),
            (
                "systems".to_string(),
                vec!["cpp".into(), "c".into(), "rust".into()]
            ),
            ("managed".to_string(), vec!["js".into(), "go".into()]),
            ("transpiled".to_string(), vec!["nim".into()]),
            ("other".to_string(), vec!["mystery".into()]),
        ]
    );
}

/// A language that timed out, or cannot run at this size, is drawn last in its group and says why; one with no time, no
/// timeout and no limit (it did not run at this size for another reason) is not drawn at all.
#[test]
fn a_missing_bar_is_drawn_last_with_its_reason() {
    let langs = vec![
        lang("c", "systems"),
        lang("zig", "systems"),
        lang("js", "managed"),
        lang("go", "managed"),
        lang("java", "managed"),
    ];
    let timings: HashMap<String, TimingResult> = [("c", 5.0), ("go", 9.0)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), timed(v)))
        .collect();
    let timed_out: HashMap<String, f64> = [("zig".to_string(), 120_000.0)].into_iter().collect();
    let limits: HashMap<String, String> = [(
        "js".to_string(),
        "V8 caps a Map at 2^24 entries".to_string(),
    )]
    .into_iter()
    .collect();
    let groups = chart_groups(&langs, Some(&timings), Some(&timed_out), Some(&limits));
    let systems = &groups.iter().find(|g| g.tier == "systems").unwrap().bars;
    assert_eq!(
        systems.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
        ["c", "zig"]
    );
    assert_eq!(systems[1].missing, Some(Missing::TimedOut(120_000.0)));
    let managed = &groups.iter().find(|g| g.tier == "managed").unwrap().bars;
    assert_eq!(
        managed.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
        ["go", "js"],
        "java has no record and is not drawn"
    );
    assert_eq!(
        managed[1].missing,
        Some(Missing::NotRun("V8 caps a Map at 2^24 entries".to_string()))
    );
}

/// One headline per LOGOS tier, in registry order, each with its own geometric means and coverage — never one number
/// picked across tiers.
#[test]
fn tier_headlines_are_one_card_per_logos_tier() {
    let data: BenchmarkData = serde_json::from_str(
        r##"{
          "metadata": {"date": "2026-10-01T00:00:00Z", "commit": "abc", "logos_version": "0.11.0", "cpu": "x", "os": "y", "versions": {}},
          "languages": [
            {"id": "c", "label": "C", "color": "#555555", "tier": "systems"},
            {"id": "logos_release", "label": "LOGOS (Rust)", "color": "#00d4ff", "tier": "logos"},
            {"id": "logos_c", "label": "LOGOS (C)", "color": "#67e8f9", "tier": "logos"},
            {"id": "logos_jit", "label": "LOGOS (JIT)", "color": "#0891b2", "tier": "logos"}
          ],
          "benchmarks": [],
          "summary": {
            "geometric_mean_speedup_vs_c": {"c": 1.0, "logos_release": 2.5, "logos_c": 2.1, "logos_jit": 0.4},
            "same_algorithm_speedup_vs_c": {"logos_release": 1.2, "logos_c": 1.05, "logos_jit": 0.3},
            "coverage": {"logos_release": {"benchmarks": 35, "of": 35, "at_reference_size": 35},
                         "logos_c": {"benchmarks": 34, "of": 35, "at_reference_size": 34},
                         "logos_jit": {"benchmarks": 35, "of": 35, "at_reference_size": 31}}
          }
        }"##,
    )
    .expect("the new summary shape parses");
    let cards = tier_headlines(&data);
    assert_eq!(
        cards.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ["logos_release", "logos_c", "logos_jit"]
    );
    assert_eq!(
        (cards[1].vs_c, cards[1].same_algorithm, cards[1].coverage),
        (Some(2.1), Some(1.05), Some((34, 35)))
    );
}

/// A colour written out in source — `#1a2b3c` (3, 4, 6 or 8 hex digits, then a non-alphanumeric), or `rgb(`/`rgba(`/
/// `hsl(` with a literal first argument — if `code` holds one.
fn colour_literal(code: &str) -> Option<String> {
    for (j, c) in code.char_indices() {
        if c != '#' {
            continue;
        }
        let digits: String = code[j + 1..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        let next = code[j + 1 + digits.len()..].chars().next();
        if matches!(digits.len(), 3 | 4 | 6 | 8)
            && !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Some(format!("#{digits}"));
        }
    }
    for f in ["rgb(", "rgba(", "hsl(", "hsla("] {
        for (j, _) in code.match_indices(f) {
            if code[j + f.len()..]
                .trim_start()
                .starts_with(|c: char| c.is_ascii_digit())
            {
                return Some(code[j..].chars().take(24).collect());
            }
        }
    }
    None
}

/// Colour on this page is a theme decision, made in one place. Every colour comes from the `--bench-*` tokens defined
/// once on `.bench-container` in style.css (each resolving to the active theme's variables), so the page works in every
/// theme: the page's Rust names no colour at all — not in a `style` attribute, not in SVG markup — a style carries
/// layout only (width, position, height, margins, an indent), and no CSS rule outside the token block names a colour.
#[test]
fn every_colour_on_the_page_is_a_theme_token() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/ui/pages/benchmarks");
    let layout = [
        "width",
        "left",
        "right",
        "bottom",
        "top",
        "height",
        "min-height",
        "margin",
        "margin-top",
        "margin-bottom",
        "padding-left",
    ];
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("the page's module directory") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for (i, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if let Some(literal) = colour_literal(code) {
                offenders.push(format!(
                    "{}:{}: colour literal `{literal}`",
                    path.file_name().unwrap().to_string_lossy(),
                    i + 1
                ));
            }
            let Some(at) = line.find("style: \"") else {
                continue;
            };
            let style = &line[at + 8..];
            let style = &style[..style.find('"').unwrap_or(style.len())];
            for decl in style.split(';').map(str::trim).filter(|d| !d.is_empty()) {
                let property = decl.split(':').next().unwrap_or("").trim();
                if !layout.contains(&property) {
                    offenders.push(format!(
                        "{}:{}: `{decl}`",
                        path.file_name().unwrap().to_string_lossy(),
                        i + 1
                    ));
                }
            }
        }
    }
    let css = std::fs::read_to_string(dir.join("style.css")).unwrap();
    let block_start = css
        .find(".bench-container {")
        .expect("style.css defines the token block");
    let preamble = css[..block_start].trim();
    assert!(
        preamble.is_empty() || (preamble.starts_with("/*") && preamble.ends_with("*/")),
        "the token block is the first rule in style.css (only a comment may precede it)"
    );
    let block_end = block_start
        + css[block_start..]
            .find("\n}\n")
            .expect("the token block closes")
        + 3;
    for (i, line) in css[block_end..].lines().enumerate() {
        let code = line.split("/*").next().unwrap_or("");
        let hex = code.char_indices().any(|(j, c)| {
            c == '#'
                && code[j + 1..]
                    .chars()
                    .take_while(|c| c.is_ascii_hexdigit())
                    .count()
                    >= 3
        });
        let literal_function = ["rgb(", "rgba(", "hsl(", "hsla("].iter().any(|f| {
            code.match_indices(f)
                .any(|(j, _)| !code[j + f.len()..].trim_start().starts_with("var("))
        });
        if hex || literal_function {
            offenders.push(format!(
                "style.css:{}: `{}`",
                css[..block_end].lines().count() + i + 1,
                line.trim()
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "colours outside the theme tokens:\n  {}",
        offenders.join("\n  ")
    );
}

/// A run that skipped the quiet-machine check is marked noisy on the page; a run that passed it is not, and a run that
/// recorded no gate (an older results file) is treated as unchecked, so it is marked too.
#[test]
fn a_run_that_skipped_the_quiet_check_is_noisy() {
    let env = |gate: &str| -> data::Environment {
        serde_json::from_str(&format!(
            r#"{{"quiet_box": {{"gate": "{gate}", "waited_seconds": 0}}}}"#
        ))
        .unwrap()
    };
    assert!(
        !env("passed").noisy(),
        "a run that passed the gate is not noisy"
    );
    assert!(
        env("overridden").noisy(),
        "a run with BENCH_QUIET=off is noisy"
    );
    assert!(env("").noisy(), "a run that recorded no gate is unchecked");
}

#[test]
fn the_page_offers_exactly_the_language_solver_and_codec_tabs() {
    let labels: Vec<&str> = BenchTab::all().iter().map(|t| t.label()).collect();
    assert_eq!(
        labels,
        ["Programming Language", "SAT Solver", "Transport Codec"],
        "the benchmarks page's top-level tabs"
    );
    for label in &labels {
        assert!(
            !label.to_lowercase().contains("tenstorrent"),
            "the benchmarks page offers a Tenstorrent tab: {label:?}"
        );
    }
}

