//! Backs every performance claim with a committed measurement.
//!
//! The parity catalog may only mark a capability `perf.status: faster` if it is genuinely faster
//! than its counterparts. This lock proves that number is not hand-typed: every `faster` capability
//! must cite a committed benchmark result under benchmarks/parity/bench/results/<id>.json whose
//! recorded speedup is > 1, beats every `perf.beat` language, and matches the catalog's
//! `perf.speedup`. Regenerate results with scripts/bench/bench-parity.sh (on a quiet box).
#![cfg(not(target_arch = "wasm32"))]

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const CATALOG_SRC: &str = include_str!("../../../benchmarks/parity/catalog/superset.json");

#[derive(Deserialize)]
struct Catalog {
    capabilities: Vec<Cap>,
}
#[derive(Deserialize)]
struct Cap {
    id: String,
    #[serde(default)]
    perf: Option<Perf>,
}
#[derive(Deserialize)]
struct Perf {
    status: String,
    #[serde(default)]
    speedup: Option<f64>,
    #[serde(default)]
    beat: Vec<String>,
}
#[derive(Deserialize)]
struct BenchResult {
    logos_strictly_fastest: bool,
    #[serde(default)]
    min_speedup: Option<f64>,
    #[serde(default)]
    speedups_vs_logos: BTreeMap<String, f64>,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

#[test]
fn every_faster_capability_cites_a_winning_benchmark() {
    let cat: Catalog = serde_json::from_str(CATALOG_SRC).expect("superset.json parses");
    let results = repo_root().join("benchmarks/parity/bench/results");

    let mut checked = 0usize;
    for c in &cat.capabilities {
        let Some(p) = &c.perf else { continue };
        if p.status != "faster" {
            continue;
        }
        let path = results.join(format!("{}.json", c.id));
        let src = fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!(
                "{}: perf.status=faster but no committed benchmark at {} — run \
                 scripts/bench/bench-parity.sh and commit the result",
                c.id,
                path.display()
            )
        });
        let r: BenchResult = serde_json::from_str(&src)
            .unwrap_or_else(|e| panic!("{}: bench result malformed: {e}", c.id));

        assert!(
            r.logos_strictly_fastest,
            "{}: bench result says LOGOS is NOT strictly fastest — cannot be perf.status=faster",
            c.id
        );
        // Must beat every parity `beat` language with a recorded speedup > 1.
        for lang in &p.beat {
            let s = r.speedups_vs_logos.get(lang).copied().unwrap_or_else(|| {
                panic!(
                    "{}: bench result has no speedup vs beat-language '{lang}'",
                    c.id
                )
            });
            assert!(
                s > 1.0,
                "{}: only {s}x vs '{lang}' — perf.status=faster requires strictly faster (>1x)",
                c.id
            );
        }
        // The catalog's advertised speedup must match the committed measurement (within noise).
        let catalog_s = p
            .speedup
            .unwrap_or_else(|| panic!("{}: faster needs a speedup", c.id));
        let measured = r.min_speedup.unwrap_or(0.0);
        assert!(
            (catalog_s - measured).abs() <= 0.15 * measured.max(1.0),
            "{}: catalog perf.speedup={catalog_s} disagrees with measured min_speedup={measured} \
             — update the catalog from the committed bench result",
            c.id
        );
        checked += 1;
    }
    // No assertion on `checked` count: it rises naturally as capabilities are proven faster.
    // The green ratchet (parity_catalog_lock::faster_count_ratchets_upward) guards the count.
    let _ = checked;
}
