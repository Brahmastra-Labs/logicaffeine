//! The EDA section of the SAT Solver tab: model checking, bounded model checking, logic rewriting, model counting and cryptanalysis, our certified engines against the
//! public ones on the public corpora. Produced by `benchmarks/eda/run-eda.sh` (`eda_bench`, one JSON per arena under
//! `benchmarks/results/eda/`) and `benchmarks/arena/gen-crypto.sh` with `arena_bench` (`results/arena/crypto.json`).

use super::*;

use std::collections::BTreeMap;

/// What an arena's run records of itself.
#[derive(Deserialize, Default, Clone)]
pub(super) struct EdaRun {
    #[serde(default)]
    pub(super) date: String,
    #[serde(default)]
    pub(super) cpu: String,
    #[serde(default)]
    pub(super) timeout_s: f64,
    #[serde(default)]
    pub(super) abc_version: String,
    /// The CPUs the runs were pinned to, one worker on each; none when the scheduler placed them.
    #[serde(default)]
    pub(super) cores: Vec<u32>,
}

/// Model checking on HWMCC'24: our PDR and ABC's, each verdict scored against the competition's consensus, and the
/// competition's own solvers within the same limit; bounded model checking, our BMC and ABC's bmc3, in the same shape
/// over the designs the competition did not find safe. `abc_read` is how ABC read each design before its engine ran.
#[derive(Deserialize)]
pub(super) struct McData {
    #[serde(flatten)]
    pub(super) run: EdaRun,
    pub(super) abc_read: String,
    #[serde(default)]
    pub(super) field: Vec<FieldEntry>,
    pub(super) summary: BTreeMap<String, McSummary>,
    pub(super) designs: Vec<McDesign>,
}

/// A solver of the competition: its correct verdicts within the limit in real seconds, and within it in CPU seconds over
/// all its threads.
#[derive(Deserialize)]
pub(super) struct FieldEntry {
    pub(super) solver: String,
    pub(super) solved: u32,
    pub(super) solved_cpu: u32,
}

#[derive(Deserialize)]
pub(super) struct McSummary {
    pub(super) solved: u32,
    pub(super) safe: u32,
    #[serde(rename = "unsafe")]
    pub(super) unsafe_: u32,
    pub(super) wrong: u32,
    pub(super) par2: f64,
}

#[derive(Deserialize)]
pub(super) struct McDesign {
    pub(super) name: String,
    pub(super) consensus: String,
    pub(super) ours: McCell,
    pub(super) abc: McCell,
}

#[derive(Deserialize)]
pub(super) struct McCell {
    pub(super) verdict: Option<String>,
    pub(super) seconds: f64,
}

/// Logic rewriting on the EPFL suite: And nodes after ours, ABC's rewrite run four times and its resyn2.
#[derive(Deserialize)]
pub(super) struct RewriteData {
    #[serde(flatten)]
    pub(super) run: EdaRun,
    pub(super) designs: Vec<RewriteDesign>,
}

#[derive(Deserialize)]
pub(super) struct RewriteDesign {
    pub(super) name: String,
    pub(super) input: Option<u64>,
    pub(super) ours: RewriteOurs,
    pub(super) abc_rewrite_x4: RewriteCell,
    pub(super) abc_resyn2: RewriteCell,
}

#[derive(Deserialize)]
pub(super) struct RewriteOurs {
    pub(super) ands: Option<u64>,
    pub(super) seconds: f64,
    pub(super) cec: String,
}

#[derive(Deserialize)]
pub(super) struct RewriteCell {
    pub(super) ands: Option<u64>,
    pub(super) seconds: f64,
}

/// Exact model counting on the 2024 model counting competition's first track.
#[derive(Deserialize)]
pub(super) struct CountData {
    #[serde(flatten)]
    pub(super) run: EdaRun,
    pub(super) formulas: Vec<CountFormula>,
}

#[derive(Deserialize)]
pub(super) struct CountFormula {
    pub(super) name: String,
    pub(super) ours: CountCell,
}

#[derive(Deserialize)]
pub(super) struct CountCell {
    pub(super) models: Option<String>,
    pub(super) seconds: f64,
}

/// The cryptanalysis arena, in `arena_bench`'s format: per engine its summary, per category the instances each decided.
#[derive(Deserialize)]
pub(super) struct CryptoData {
    #[serde(default)]
    pub(super) timeout_s: f64,
    pub(super) solvers: Vec<String>,
    pub(super) summary: BTreeMap<String, CryptoSummary>,
    pub(super) categories: BTreeMap<String, BTreeMap<String, u32>>,
}

#[derive(Deserialize)]
pub(super) struct CryptoSummary {
    pub(super) solved: u32,
    pub(super) par2: f64,
    #[serde(default)]
    pub(super) verified: u32,
}

/// The five feeds, together.
pub(super) struct EdaData {
    pub(super) mc: McData,
    pub(super) bmc: McData,
    pub(super) rewrite: RewriteData,
    pub(super) count: CountData,
    pub(super) crypto: CryptoData,
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) static EDA_DATA: LazyLock<EdaData> = LazyLock::new(|| EdaData {
    mc: serde_json::from_str(include_str!("../../../../../../benchmarks/results/eda/mc.json")).unwrap(),
    bmc: serde_json::from_str(include_str!("../../../../../../benchmarks/results/eda/bmc.json")).unwrap(),
    rewrite: serde_json::from_str(include_str!("../../../../../../benchmarks/results/eda/rewrite.json")).unwrap(),
    count: serde_json::from_str(include_str!("../../../../../../benchmarks/results/eda/count.json")).unwrap(),
    crypto: serde_json::from_str(include_str!("../../../../../../benchmarks/results/arena/crypto.json")).unwrap(),
});
#[cfg(target_arch = "wasm32")]
pub(super) static EDA_DATA: OnceLock<EdaData> = OnceLock::new();

/// The EDA feeds — same gating contract as [`bench_data`].
pub(super) fn eda_data() -> Option<&'static EdaData> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(&EDA_DATA)
    }
    #[cfg(target_arch = "wasm32")]
    {
        EDA_DATA.get()
    }
}

/// An engine's display name.
pub(super) fn eda_label(id: &str) -> String {
    match id {
        "ours" => "LOGOS".to_string(),
        "abc" => "ABC pdr".to_string(),
        "abc_rewrite_x4" => "ABC rewrite ×4".to_string(),
        "abc_resyn2" => "ABC resyn2".to_string(),
        "ric3" => "rIC3".to_string(),
        other => solver_label(other),
    }
}

/// The competition's five best solvers on one clock, best first, each with its correct verdicts: within the limit in CPU
/// seconds over all its threads (`cpu`), or in real seconds.
pub(super) fn field_line(field: &[FieldEntry], cpu: bool) -> String {
    let count = |f: &FieldEntry| if cpu { f.solved_cpu } else { f.solved };
    let mut best: Vec<&FieldEntry> = field.iter().filter(|f| count(f) > 0).collect();
    best.sort_by(|a, b| count(b).cmp(&count(a)).then(a.solver.cmp(&b.solver)));
    best.iter().take(5).map(|f| format!("{} {}", eda_label(&f.solver), count(f))).collect::<Vec<_>>().join(", ")
}

/// Per design, how our And-node count compares with another's: fewer, equal, more.
pub(super) fn rewrite_tally(designs: &[RewriteDesign], other: impl Fn(&RewriteDesign) -> Option<u64>) -> (u32, u32, u32) {
    let mut tally = (0, 0, 0);
    for d in designs {
        let (Some(ours), Some(theirs)) = (d.ours.ands, other(d)) else { continue };
        match ours.cmp(&theirs) {
            std::cmp::Ordering::Less => tally.0 += 1,
            std::cmp::Ordering::Equal => tally.1 += 1,
            std::cmp::Ordering::Greater => tally.2 += 1,
        }
    }
    tally
}

#[component]
pub fn EdaSection() -> Element {
    let data = eda_data().expect("rendered only inside the loaded Benchmarks page");
    let (mc, bmc, rw, count, crypto) = (&data.mc, &data.bmc, &data.rewrite, &data.count, &data.crypto);
    let mc_rows: Vec<(String, &McSummary)> = ["ours", "abc"]
        .iter()
        .filter_map(|k| mc.summary.get(*k).map(|s| (eda_label(k), s)))
        .collect();
    let bmc_rows: Vec<(String, &McSummary)> = [("ours", eda_label("ours")), ("abc", "ABC bmc3".to_string())]
        .into_iter()
        .filter_map(|(k, name)| bmc.summary.get(k).map(|s| (name, s)))
        .collect();
    let bmc_designs = bmc.designs.len();
    let bmc_unsafe = bmc.designs.iter().filter(|d| d.consensus == "unsafe").count();
    let (bmc_field_cpu, bmc_field_real) = (field_line(&bmc.field, true), field_line(&bmc.field, false));
    let designs = mc.designs.len();
    let decided_by_consensus = mc.designs.iter().filter(|d| d.consensus != "unknown").count();
    let (field_cpu, field_real) = (field_line(&mc.field, true), field_line(&mc.field, false));
    let pinned = mc.run.cores.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
    let finished: Vec<&RewriteDesign> = rw
        .designs
        .iter()
        .filter(|d| d.input.is_some() && d.ours.ands.is_some() && d.abc_rewrite_x4.ands.is_some() && d.abc_resyn2.ands.is_some())
        .collect();
    let rw_totals: Vec<(String, u64, f64)> = {
        let sum = |pick: &dyn Fn(&RewriteDesign) -> (Option<u64>, f64)| {
            finished.iter().fold((0u64, 0.0f64), |(a, s), d| {
                let (n, t) = pick(d);
                (a + n.unwrap_or(0), s + t)
            })
        };
        vec![
            ("Input".to_string(), finished.iter().filter_map(|d| d.input).sum(), 0.0),
            {
                let (a, s) = sum(&|d| (d.ours.ands, d.ours.seconds));
                (eda_label("ours"), a, s)
            },
            {
                let (a, s) = sum(&|d| (d.abc_rewrite_x4.ands, d.abc_rewrite_x4.seconds));
                (eda_label("abc_rewrite_x4"), a, s)
            },
            {
                let (a, s) = sum(&|d| (d.abc_resyn2.ands, d.abc_resyn2.seconds));
                (eda_label("abc_resyn2"), a, s)
            },
        ]
    };
    let vs_four = rewrite_tally(&rw.designs, |d| d.abc_rewrite_x4.ands);
    let vs_resyn2 = rewrite_tally(&rw.designs, |d| d.abc_resyn2.ands);
    let cec_equal = rw.designs.iter().filter(|d| d.ours.cec == "equal").count();
    let counted = count.formulas.iter().filter(|f| f.ours.models.is_some()).count();
    let count_cpu: f64 = count.formulas.iter().filter(|f| f.ours.models.is_some()).map(|f| f.ours.seconds).sum();
    let crypto_rows: Vec<(String, &CryptoSummary)> =
        crypto.solvers.iter().filter_map(|k| crypto.summary.get(k).map(|s| (eda_label(k), s))).collect();
    let crypto_instances: u32 = crypto.categories.values().map(|c| c.get("count").copied().unwrap_or(0)).sum();

    rsx! {
        div { class: "bench-section", id: "eda",
            div { class: "bench-section-title", "Hardware verification and beyond" }
            div { class: "bench-section-desc",
                "The same certified core drives the engines EDA runs on: model checking, logic rewriting, exact model counting and cryptanalysis. Each is raced against the public tool of its kind on the public corpus, every run limited and timed in CPU seconds, and every answer of ours checked: an invariant refuted three ways or a run replayed, a rewritten circuit proven equal to its input, a model recomputed through the cipher."
            }

            div { class: "bench-codec-scenario", "Model checking — HWMCC'24, {designs} designs" }
            table { class: "bench-stats-table",
                thead { tr { th { "Engine" } th { "Solved" } th { "Safe" } th { "Unsafe" } th { "Wrong" } th { "PAR-2" } } }
                tbody {
                    for (name, s) in mc_rows.iter() {
                        tr { class: if name == "LOGOS" { "highlight" } else { "" },
                            td { "{name}" } td { "{s.solved}" } td { "{s.safe}" } td { "{s.unsafe_}" } td { "{s.wrong}" } td { "{s.par2:.0}" }
                        }
                    }
                }
            }
            if !mc.field.is_empty() {
                div { class: "bench-solver-note",
                    "The competition's own solvers on the same designs, from its results table and on its hardware, each by its correct verdicts — within {mc.run.timeout_s:.0} s of CPU time over all its threads, the clock our engines run under: {field_cpu}; within {mc.run.timeout_s:.0} s of real time: {field_real}. ABC reads each design as AIGER 1.9 describes it, a latch free to start anywhere taking its first value from an input of its own ({mc.abc_read}). Wrong counts a verdict that contradicts the consensus of the {decided_by_consensus} designs the competition decided."
                }
            }

            div { class: "bench-codec-scenario", style: "margin-top: 26px;", "Bounded model checking — HWMCC'24, the {bmc_designs} designs the competition did not find safe" }
            table { class: "bench-stats-table",
                thead { tr { th { "Engine" } th { "Runs found" } th { "Wrong" } th { "PAR-2" } } }
                tbody {
                    for (name, s) in bmc_rows.iter() {
                        tr { class: if name == "LOGOS" { "highlight" } else { "" },
                            td { "{name}" } td { "{s.unsafe_}" } td { "{s.wrong}" } td { "{s.par2:.0}" }
                        }
                    }
                }
            }
            div { class: "bench-solver-note",
                "A run to a bad state, found by unrolling the circuit only as far as its bad states and constraints read it, each clock encoded by cuts of up to six leaves, and asked a clock at a time and, in a cone small enough, sixty-four clocks a query by turns, so a deep run needs no refutation of each clock before it; every run replays. {bmc_unsafe} of these designs the competition found unsafe, the rest it left open. ABC reads each design as for model checking ({bmc.abc_read})."
                if !bmc.field.is_empty() {
                    span { " The competition's own solvers, each by the runs it found within {bmc.run.timeout_s:.0} s of CPU time over all its threads, the clock our engines run under: {bmc_field_cpu}; within {bmc.run.timeout_s:.0} s of real time: {bmc_field_real}." }
                }
            }

            div { class: "bench-codec-scenario", style: "margin-top: 26px;", "Logic rewriting — EPFL, the {finished.len()} of {rw.designs.len()} circuits every engine finished" }
            table { class: "bench-stats-table",
                thead { tr { th { "" } th { "And nodes" } th { "CPU seconds" } } }
                tbody {
                    for (name, ands, secs) in rw_totals.iter() {
                        tr { class: if name == "LOGOS" { "highlight" } else { "" },
                            td { "{name}" } td { "{ands}" } td { if *secs > 0.0 { "{secs:.1}" } else { "—" } }
                        }
                    }
                }
            }
            div { class: "bench-solver-note",
                "Circuit by circuit, fewer And nodes than ABC rewrite ×4 on {vs_four.0}, as many on {vs_four.1}, more on {vs_four.2}; against resyn2 {vs_resyn2.0}, {vs_resyn2.1} and {vs_resyn2.2}. Every result is proven equal to its input by our fraig, and ABC's cec agrees on {cec_equal} within the limit."
            }

            div { class: "bench-codec-scenario", style: "margin-top: 26px;", "Exact model counting — MCC 2024 track 1" }
            div { class: "bench-codec-panel",
                div { class: "bench-codec-panel-text",
                    "{counted} of {count.formulas.len()} formulas counted exactly within {count.run.timeout_s:.0} s of CPU each ({count_cpu:.0} s in all), by DPLL with components, a cache and clause learning after preprocessing that keeps the count. No other counter is raced here yet."
                }
            }

            div { class: "bench-codec-scenario", style: "margin-top: 26px;", "Cryptanalysis — reduced-round SHA-1 and Trivium, {crypto_instances} planted instances" }
            table { class: "bench-stats-table",
                thead { tr { th { "Engine" } th { "Solved" } th { "PAR-2" } } }
                tbody {
                    for (name, s) in crypto_rows.iter() {
                        tr { class: if name == "LOGOS" { "highlight" } else { "" },
                            td { "{name}" } td { "{s.solved}" } td { "{s.par2:.0}" }
                        }
                    }
                }
            }

            div { class: "bench-codec-method",
                "Method: every engine is a subprocess on the byte-identical input, limited by prlimit and timed by its own user and system CPU, so a loaded machine ranks them alike; unsolved runs count twice the limit in PAR-2. ABC is yosys-abc: pdr and bmc3 after {mc.abc_read}, which gives each latch free to start anywhere its first value from an input of its own and folds the invariant constraints into the property, rewrite four times, and resyn2 spelled out. The cryptanalysis instances are planted, so each is satisfiable, and Kissat, CaDiCaL and CryptoMiniSat race on them."
                if !mc.run.cpu.is_empty() {
                    span { " · Measured on {mc.run.cpu}" }
                }
                if !mc.run.cores.is_empty() {
                    span { ", each model-checking run pinned to one of CPUs {pinned}" }
                }
                if !mc.run.abc_version.is_empty() {
                    span { " · {mc.run.abc_version}" }
                }
                if !mc.run.date.is_empty() {
                    span { " · {mc.run.date}" }
                }
            }
        }
    }
}
