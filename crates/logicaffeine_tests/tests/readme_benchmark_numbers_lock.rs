//! Every benchmark number the public prose quotes is a number the charts were drawn with, and the charts were
//! drawn from the results on disk.
//!
//! `benchmarks/gen-readme-charts.py` draws the README's charts from `benchmarks/results/**.json` and records in
//! `benchmarks/results/charts/headlines.json` every figure it drew, plus an FNV-1a-64 digest of every results file
//! it read. This lock recomputes those digests, so refreshed results without regenerated charts go RED; redraws the
//! charts from the committed results into a temporary directory and requires every file it writes to be the committed
//! one, byte for byte (so the generator's own label checks pass on the results it is handed); requires
//! each README chart's alt text, the caption a reader sees under it, and the headline lines of
//! `apps/logicaffeine_web/public/llms.txt`, to quote the drawn figures; requires the suite's README to count the programs `benchmarks/programs/` holds; and requires
//! results that claim the workspace version to have measured every one of those programs.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test readme_benchmark_numbers_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::path::PathBuf;

const HEADLINES: &str = "benchmarks/results/charts/headlines.json";
const README: &str = "README.md";
const LLMS: &str = "apps/logicaffeine_web/public/llms.txt";
const SUITE_README: &str = "benchmarks/README.md";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

fn json(rel: &str) -> serde_json::Value {
    serde_json::from_str(&read(rel)).unwrap_or_else(|e| panic!("{rel} is not JSON: {e}"))
}

fn fnv1a64(data: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in data {
        h = (h ^ u64::from(*b)).wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// A figure `chart` recorded, by name.
fn figure(headlines: &serde_json::Value, chart: &str, name: &str) -> f64 {
    headlines["charts"][chart][name]
        .as_f64()
        .unwrap_or_else(|| panic!("{HEADLINES} has no {chart} figure `{name}`"))
}

/// The alt text of the README image whose source is `charts/<chart>`.
fn alt_text(readme: &str, chart: &str) -> String {
    let src = format!("benchmarks/results/charts/{chart}");
    let at = readme
        .find(&src)
        .unwrap_or_else(|| panic!("{README} does not show {src}"));
    let tag = &readme[at..];
    let tag = &tag[..tag.find('>').unwrap_or(tag.len())];
    let alt = tag
        .split_once("alt=\"")
        .unwrap_or_else(|| panic!("{README}'s {chart} image has no alt text"))
        .1;
    alt[..alt.find('"').unwrap_or(alt.len())].to_string()
}

/// The phrases the alt text of `chart` must carry, each built from a drawn figure.
fn required_phrases(h: &serde_json::Value, chart: &str) -> Vec<String> {
    let f = |name: &str| figure(h, chart, name);
    match chart {
        "overall-vs-c.svg" => vec![
            format!("across {} benchmarks", f("benchmarks")),
            format!("LOGOS {:.2}x", f("logos_all")),
            format!("{:.2}x same-algorithm", f("logos_apples")),
            format!("Rust {:.2}x", f("rust")),
            format!("Zig {:.2}x", f("zig")),
            format!("C++ {:.2}x", f("cpp")),
            format!("Go {:.2}x", f("go")),
            format!("Nim {:.2}x", f("nim")),
            format!("Java {:.2}x", f("java")),
            format!("JavaScript {:.2}x", f("js")),
        ],
        "compiled-vs-c.svg" => vec![
            format!(
                "{} benchmarks collapse by {:.0}x to {:.0}x",
                f("collapses"),
                f("collapse_min"),
                f("collapse_max")
            ),
            format!("({:.2}x to {:.2}x)", f("parity_min"), f("parity_max")),
        ],
        "interp-vs-v8.svg" => vec![
            format!("{:.2}x V8", f("eager_vs_v8")),
            format!("about {:.1}x faster", f("coldstart_x")),
            format!(
                "({:.1} ms vs {:.1} ms",
                f("coldstart_logos_ms"),
                f("coldstart_node_ms")
            ),
        ],
        "codec-size.svg" => vec![
            format!("on {} of {} fair", f("fair_wins"), f("fair_total")),
            format!("geomean {:.2}x smaller", f("geomean_smaller")),
        ],
        "codec-speed.svg" => vec![
            format!(
                "encodes up to {:.1}x and decodes up to {:.1}x",
                f("enc_max"),
                f("dec_max")
            ),
            format!("takes {:.0} ns", f("ra_logos_ns")),
        ],
        "sat-arena.svg" => vec![
            format!(
                "across {} loaded SATLIB instances in {} families",
                f("instances"),
                f("families")
            ),
            format!(
                "solves {} of {} at PAR-2 {}",
                f("ours_solved"),
                f("instances"),
                f("ours_par2")
            ),
            format!("with {} machine-checked proofs", f("ours_proofs")),
            format!(
                "Kissat ({}, {}, {})",
                f("kissat_solved"),
                f("kissat_par2"),
                f("kissat_proofs")
            ),
            format!(
                "CaDiCaL ({}, {}, {})",
                f("cadical_solved"),
                f("cadical_par2"),
                f("cadical_proofs")
            ),
        ],
        _ => Vec::new(),
    }
}

#[test]
fn the_digest_is_fnv1a64() {
    assert_eq!(fnv1a64(b""), "cbf29ce484222325");
    assert_eq!(fnv1a64(b"a"), "af63dc4c8601ec8c");
}

#[test]
fn the_charts_are_current_with_the_results() {
    let h = json(HEADLINES);
    assert_eq!(
        h["input_digest"], "fnv1a64",
        "{HEADLINES} records digests of an unknown kind"
    );
    let inputs = h["inputs"]
        .as_object()
        .unwrap_or_else(|| panic!("{HEADLINES} records no inputs"));
    assert!(
        inputs.contains_key("latest.json"),
        "{HEADLINES} does not record latest.json as an input"
    );
    let stale: Vec<String> = inputs
        .iter()
        .filter_map(|(name, recorded)| {
            let bytes = std::fs::read(repo_root().join("benchmarks/results").join(name))
                .unwrap_or_default();
            (fnv1a64(&bytes) != recorded.as_str().unwrap_or_default()).then(|| name.clone())
        })
        .collect();
    assert!(
        stale.is_empty(),
        "benchmarks/results/{stale:?} changed since the charts were drawn — run `python3 benchmarks/gen-readme-charts.py`"
    );
}

#[test]
fn the_readme_quotes_the_charted_numbers() {
    let h = json(HEADLINES);
    let readme = read(README);
    let mut wrong = Vec::new();
    for chart in h["charts"].as_object().unwrap().keys() {
        let phrases = required_phrases(&h, chart);
        if phrases.is_empty() {
            continue;
        }
        let alt = alt_text(&readme, chart);
        wrong.extend(
            phrases
                .into_iter()
                .filter(|p| !alt.contains(p.as_str()))
                .map(|p| format!("{chart}: `{p}`")),
        );
    }
    assert!(
        wrong.is_empty(),
        "{README}'s chart alt text does not say what the charts drew:\n  {}",
        wrong.join("\n  ")
    );
}

#[test]
fn llms_txt_quotes_the_charted_numbers() {
    let h = json(HEADLINES);
    let llms = read(LLMS);
    let chart = "overall-vs-c.svg";
    let want = [
        format!(
            "{:.2}x geometric-mean speedup vs C",
            figure(&h, chart, "logos_all")
        ),
        format!("Rust ({:.2}x)", figure(&h, chart, "rust")),
        format!("Zig ({:.2}x)", figure(&h, chart, "zig")),
        format!("C++ ({:.2}x)", figure(&h, chart, "cpp")),
        format!("{} programs", figure(&h, chart, "benchmarks")),
    ];
    let missing: Vec<&String> = want.iter().filter(|p| !llms.contains(p.as_str())).collect();
    assert!(
        missing.is_empty(),
        "{LLMS} does not quote the charted numbers: {missing:?}"
    );
}

fn programs() -> BTreeSet<String> {
    std::fs::read_dir(repo_root().join("benchmarks/programs"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect()
}

#[test]
fn the_suite_readme_counts_the_programs_the_suite_holds() {
    let n = programs().len();
    let text = read(SUITE_README);
    let counts: Vec<&str> = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|w| {
            w[1].trim_end_matches([',', '.', ';', ':']) == "programs"
                && w[0].chars().all(|c| c.is_ascii_digit())
        })
        .map(|w| w[0])
        .collect();
    assert!(
        !counts.is_empty(),
        "{SUITE_README} does not say how many programs the suite holds"
    );
    let wrong: Vec<&&str> = counts
        .iter()
        .filter(|c| c.parse::<usize>().ok() != Some(n))
        .collect();
    assert!(
        wrong.is_empty(),
        "{SUITE_README} counts {wrong:?} programs; benchmarks/programs/ holds {n}"
    );
}

fn workspace_version() -> String {
    let manifest: toml::Value = read("Cargo.toml").parse().unwrap();
    manifest["workspace"]["package"]["version"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn results_for_this_version_measured_every_program() {
    let latest = json("benchmarks/results/latest.json");
    if latest["metadata"]["logos_version"].as_str() != Some(workspace_version().as_str()) {
        return;
    }
    let measured: BTreeSet<String> = latest["benchmarks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["id"].as_str().map(str::to_string))
        .collect();
    let unmeasured: Vec<String> = programs()
        .into_iter()
        .filter(|p| !measured.contains(p))
        .collect();
    assert!(
        unmeasured.is_empty(),
        "benchmarks/results/latest.json claims this version but never measured {unmeasured:?}"
    );
}

/// The caption a reader sees under the README image whose source is `charts/<chart>`: the prose between the
/// image's closing `</p>` and the next image, heading or rule, with its line breaks read as spaces.
fn caption(readme: &str, chart: &str) -> String {
    let src = format!("benchmarks/results/charts/{chart}");
    let at = readme
        .find(&src)
        .unwrap_or_else(|| panic!("{README} does not show {src}"));
    let after = &readme[at..];
    let after = &after[after.find("</p>").map_or(after.len(), |i| i + 4)..];
    let end = ["<p align", "\n#", "\n---"]
        .iter()
        .filter_map(|m| after.find(m))
        .min()
        .unwrap_or(after.len());
    after[..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The figures the visible caption under `chart` quotes: each chart's headline, in the words the caption uses.
fn caption_phrases(h: &serde_json::Value, chart: &str) -> Vec<String> {
    let f = |name: &str| figure(h, chart, name);
    match chart {
        "overall-vs-c.svg" => vec![
            format!("across {} benchmarks", f("benchmarks")),
            format!("LOGOS {:.2}x", f("logos_all")),
            format!("{:.2}x same-algorithm", f("logos_apples")),
        ],
        "compiled-vs-c.svg" => {
            vec![format!(
                "{} benchmarks collapse by {:.0}x to {:.0}x",
                f("collapses"),
                f("collapse_min"),
                f("collapse_max")
            )]
        }
        "interp-vs-v8.svg" => vec![
            format!("{:.2}x V8", f("eager_vs_v8")),
            format!("about {:.1}x faster", f("coldstart_x")),
        ],
        "codec-size.svg" => vec![format!("geomean {:.2}x smaller", f("geomean_smaller"))],
        "codec-speed.svg" => vec![format!(
            "encodes up to {:.1}x and decodes up to {:.1}x",
            f("enc_max"),
            f("dec_max")
        )],
        "sat-arena.svg" => vec![
            format!(
                "across {} loaded SATLIB instances in {} families",
                f("instances"),
                f("families")
            ),
            format!(
                "solves {} of {} at PAR-2 {}",
                f("ours_solved"),
                f("instances"),
                f("ours_par2")
            ),
            format!("with {} machine-checked proofs", f("ours_proofs")),
        ],
        _ => Vec::new(),
    }
}

#[test]
fn the_readme_captions_quote_the_charted_numbers() {
    let h = json(HEADLINES);
    let readme = read(README);
    let mut wrong = Vec::new();
    let mut checked = 0;
    for chart in h["charts"].as_object().unwrap().keys() {
        let phrases = caption_phrases(&h, chart);
        if phrases.is_empty() {
            continue;
        }
        let text = caption(&readme, chart);
        assert!(
            !text.is_empty(),
            "{README} shows {chart} with no caption under it"
        );
        checked += 1;
        wrong.extend(
            phrases
                .into_iter()
                .filter(|p| !text.contains(p.as_str()))
                .map(|p| format!("{chart}: `{p}` (caption: {text})")),
        );
    }
    assert!(
        checked >= 6,
        "the README captions the headline charts, {checked} checked"
    );
    assert!(
        wrong.is_empty(),
        "{README}'s chart captions do not say what the charts drew:\n  {}",
        wrong.join("\n  ")
    );
}

/// The committed charts are what `benchmarks/gen-readme-charts.py --check` draws from the committed results: the
/// generator runs, its label checks included, into a temporary directory, and every file it writes is the committed
/// one, byte for byte.
#[test]
fn the_charts_are_what_the_generator_draws_from_the_results() {
    let drawn = tempfile::TempDir::new().expect("temporary directory");
    let out = std::process::Command::new("python3")
        .arg(repo_root().join("benchmarks/gen-readme-charts.py"))
        .arg("--check")
        .env("BENCH_CHARTS_DIR", drawn.path())
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("python3 runs the chart generator");
    assert!(
        out.status.success(),
        "the chart generator fails on the committed results:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let mut files: Vec<String> = std::fs::read_dir(drawn.path())
        .expect("read the drawn charts")
        .map(|e| e.expect("a drawn chart").file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert!(
        files.iter().any(|f| f == "headlines.json") && files.iter().filter(|f| f.ends_with(".svg")).count() >= 8,
        "the generator draws every README chart and its headlines: {files:?}"
    );
    let stale: Vec<&String> = files
        .iter()
        .filter(|f| {
            let committed = std::fs::read(repo_root().join("benchmarks/results/charts").join(f.as_str()));
            committed.ok().as_deref() != Some(&std::fs::read(drawn.path().join(f.as_str())).expect("read a drawn chart")[..])
        })
        .collect();
    assert!(
        stale.is_empty(),
        "these committed charts are not what the generator draws from the committed results: {stale:?} — run \
         `python3 benchmarks/gen-readme-charts.py --check` after the results change"
    );
}
