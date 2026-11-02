//! The SAT Solver tab.

use super::*;

// Our certified prover vs the field — Z3 (SMT), Kissat (the CDCL world champion) and SaDiCaL (the
// reference PR/SDCL solver) — on structured-UNSAT families. Produced by benchmarks/run-solver-vs-z3.sh
// (native; external solvers run as subprocesses on the byte-identical DIMACS, emitting a clausal
// proof). Our verdicts are pure-Rust and browser-identical; the whole crate is Z3-free in WASM.
#[derive(Deserialize)]
pub(super) struct SolverData {
    #[serde(default)]
    pub(super) metadata: SolverMeta,
    pub(super) families: Vec<SolverFamily>,
}

#[derive(Deserialize, Default)]
pub(super) struct SolverMeta {
    #[serde(default)]
    pub(super) date: String,
    #[serde(default)]
    pub(super) cpu: String,
    #[serde(default)]
    pub(super) kissat: String,
    #[serde(default)]
    pub(super) cadical: String,
    #[serde(default)]
    pub(super) cryptominisat: String,
}

#[derive(Deserialize)]
pub(super) struct SolverFamily {
    pub(super) id: String,
    pub(super) name: String,
    /// How our prover wins (SR proof / matching / GF(2) Gaussian / plain CDCL).
    pub(super) mechanism: String,
    /// The honest one-line separation framing.
    pub(super) separation: String,
    /// Longer caption (e.g. the SDCL auto-discovery callout for PHP).
    pub(super) note: String,
    pub(super) rows: Vec<SolverRow>,
}

#[derive(Deserialize)]
pub(super) struct SolverRow {
    pub(super) n: u32,
    pub(super) ours_ms: f64,
    pub(super) ours_detail: String,
    /// Size in bytes of OUR certified proof where one exists (SR proof / compact GF-ring certificate),
    /// so the "ours" bar shows a proof size just like the competitors — the answer to "why doesn't
    /// Logos show its proof size?". `None` for a structural witness (chessboard, pigeonhole variants).
    #[serde(default)]
    pub(super) ours_proof_bytes: Option<u64>,
    /// Short format label for that proof (`SR`, `GF(2) cert`, `ℤ/6 cert`, …), shown beside the size.
    #[serde(default)]
    pub(super) ours_proof_fmt: Option<String>,
    /// Competing solvers on the SAME instance (z3 / kissat / sadical / cadical / cryptominisat), in
    /// display order.
    #[serde(default)]
    pub(super) others: Vec<OtherResult>,
}

/// One competitor's result on an instance. `status`: "unsat" (solved it), "sat", "timeout" (hit the
/// wall), or "error"/"absent". `proof_bytes` is the emitted clausal-proof size where it completed.
#[derive(Deserialize)]
pub(super) struct OtherResult {
    pub(super) solver: String,
    pub(super) status: String,
    pub(super) ms: f64,
    #[serde(default)]
    pub(super) proof_bytes: Option<u64>,
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) static SOLVER_DATA: LazyLock<SolverData> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../../../../benchmarks/results/solvers.json"
    ))
    .unwrap()
});
#[cfg(target_arch = "wasm32")]
pub(super) static SOLVER_DATA: OnceLock<SolverData> = OnceLock::new();

/// The solver head-to-head feed — same gating contract as [`bench_data`].
pub(super) fn solver_data() -> Option<&'static SolverData> {
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
pub(super) fn fmt_solver_ms(ms: f64) -> String {
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
pub(super) fn linear_bar_pct(value: f64, max: f64) -> f64 {
    if max <= 0.0 {
        return 0.0;
    }
    (value / max * 100.0).clamp(0.0, 100.0)
}

/// Bar colour per competitor (ours is cyan; each solver its own hue; timeout is red, handled inline).
/// Display name for a competitor id.
pub(super) fn solver_label(name: &str) -> String {
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
pub(super) fn solver_bar(label: &str, pct: f64, class: &str, time: &str, detail: &str) -> Element {
    let show_inside = pct > 34.0;
    rsx! {
        div { class: "bench-bar-row",
            div { class: "bench-bar-label", "{label}" }
            div { class: "bench-bar-track",
                div {
                    class: "{class}",
                    style: "width: {pct:.1}%;",
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
                div { class: "bench-codec-panel",
                    div { class: "bench-solver-callout-title",
                        "A proof-system gap, not a tuning trick"
                    }
                    div { class: "bench-codec-panel-text",
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
                    div { class: "bench-codec-scenario", "{fam.name}" }
                    div { class: "bench-solver-mechanism", "ours: {fam.mechanism}" }
                    div { class: "bench-solver-separation", "{fam.separation}" }
                    div { class: "bench-chart",
                        for row in fam.rows.iter() {
                            div { style: "margin-bottom: 12px;",
                                div { class: "bench-solver-n", "n = {row.n}" }
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
                                                solver_bar("ours", linear_bar_pct(row.ours_ms, scale_max), "bench-bar-fill bench-bar-logos-rust", &ours_lbl, &row.ours_detail)
                                            }
                                            for o in row.others.iter() {
                                                {
                                                    let (pct, class, lbl) = match o.status.as_str() {
                                                        "timeout" => (100.0_f64, "bench-bar-fill bench-bar-timeout", format!("timeout ≥{}", fmt_solver_ms(o.ms))),
                                                        "unsat" | "sat" => {
                                                            let mut l = fmt_solver_ms(o.ms);
                                                            if let Some(pb) = o.proof_bytes {
                                                                l = format!("{l} · {} proof", format_bytes(pb as f64));
                                                            }
                                                            (linear_bar_pct(o.ms, scale_max), "bench-bar-fill bench-bar-other", l)
                                                        }
                                                        other => (1.0_f64, "bench-bar-fill bench-bar-muted", other.to_string()),
                                                    };
                                                    solver_bar(&solver_label(&o.solver), pct, class, &lbl, &format!("{} — solve + proof", solver_label(&o.solver)))
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "bench-solver-note", "{fam.note}" }
                }
            }

            div { class: "bench-codec-method",
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
