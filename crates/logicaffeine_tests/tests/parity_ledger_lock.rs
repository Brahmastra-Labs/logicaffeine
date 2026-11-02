//! Keeps benchmarks/parity/PARITY_LEDGER.md honest.
//!
//! The ledger is a pure function of the catalog + inventories. This lock recomputes the numeric
//! spine (per-domain and overall green/in-scope/faster/perf-gated, plus upstream triage) and
//! asserts the committed markdown reports exactly those integers — so a stale ledger (someone
//! landed a capability without regenerating) or a hand-edited/lying ledger goes RED.
//! Regenerate with scripts/bench/gen-parity-ledger.py.
#![cfg(not(target_arch = "wasm32"))]

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const LEDGER_SRC: &str = include_str!("../../../benchmarks/parity/PARITY_LEDGER.md");
const CATALOG_SRC: &str = include_str!("../../../benchmarks/parity/catalog/superset.json");
const LANGS: [&str; 5] = ["c", "python", "go", "rust", "java"];

#[derive(Deserialize)]
struct Catalog {
    capabilities: Vec<Cap>,
}
#[derive(Deserialize)]
struct Cap {
    domain: String,
    status: String,
    #[serde(default)]
    provenance: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    perf: Option<Perf>,
}
#[derive(Deserialize)]
struct Perf {
    status: String,
}
#[derive(Deserialize)]
struct InvEntry {
    module: String,
    symbol: String,
}
#[derive(Deserialize)]
struct Inventory {
    entries: Vec<InvEntry>,
}

#[derive(Default)]
struct Tally {
    green: usize,
    inscope: usize,
    faster: usize,
    perfgated: usize,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

#[test]
fn ledger_matches_catalog_derived_numbers() {
    let cat: Catalog = serde_json::from_str(CATALOG_SRC).expect("superset.json parses");

    let mut per: BTreeMap<String, Tally> = BTreeMap::new();
    let (mut green, mut inscope, mut na, mut faster, mut perfgated) = (0, 0, 0, 0, 0);
    for c in &cat.capabilities {
        let t = per.entry(c.domain.clone()).or_default();
        if c.status == "na" {
            na += 1;
        } else {
            inscope += 1;
            t.inscope += 1;
            if c.status == "green" {
                green += 1;
                t.green += 1;
            }
        }
        if let Some(p) = &c.perf {
            if p.status == "pending" || p.status == "faster" {
                perfgated += 1;
                t.perfgated += 1;
                if p.status == "faster" {
                    faster += 1;
                    t.faster += 1;
                }
            }
        }
    }

    // Upstream triage from inventories.
    let inv_root = repo_root().join("benchmarks/parity/inventory");
    let mut universe: BTreeSet<String> = BTreeSet::new();
    for lang in LANGS {
        let src = fs::read_to_string(inv_root.join(format!("{lang}.json")))
            .unwrap_or_else(|_| panic!("inventory/{lang}.json missing"));
        let inv: Inventory = serde_json::from_str(&src).expect("inventory parses");
        for e in inv.entries {
            universe.insert(format!("{lang}:{}.{}", e.module, e.symbol));
        }
    }
    let mut accounted: BTreeSet<String> = BTreeSet::new();
    for c in &cat.capabilities {
        for (lang, syms) in &c.provenance {
            for s in syms {
                accounted.insert(format!("{lang}:{s}"));
            }
        }
    }
    let total = universe.len();
    let acc = accounted.len();

    let must_contain = |needle: &str| {
        assert!(
            LEDGER_SRC.contains(needle),
            "PARITY_LEDGER.md is stale or wrong — expected to contain:\n  {needle}\n\
             Regenerate with scripts/bench/gen-parity-ledger.py"
        );
    };

    // Overall spine (integer tuples — deterministic, no float-format dependency).
    must_contain(&format!(
        "({green} green / {inscope} in-scope; {na} n/a excluded)"
    ));
    must_contain(&format!("({faster} faster / {perfgated} perf-gated)"));
    // Triage: claimed + total are catalog/inventory-derived here; the exact in-scope backlog
    // (scope.json-partitioned) is guarded by parity_catalog_lock::every_upstream_symbol_…
    must_contain(&format!("{acc} claimed (of {total})"));

    // Per-domain rows: verify the integer counts appear in each domain's row.
    for (name, t) in &per {
        let row = LEDGER_SRC
            .lines()
            .find(|l| l.starts_with(&format!("| {name} |")))
            .unwrap_or_else(|| panic!("PARITY_LEDGER.md is missing a row for domain '{name}'"));
        let cells: Vec<&str> = row.split('|').map(|c| c.trim()).collect();
        // | domain | green | inscope | correct% | faster | perfgated | faster% |
        assert_eq!(
            cells.get(2).copied(),
            Some(t.green.to_string().as_str()),
            "domain '{name}': ledger green count drifted (expected {})",
            t.green
        );
        assert_eq!(
            cells.get(3).copied(),
            Some(t.inscope.to_string().as_str()),
            "domain '{name}': ledger in-scope count drifted (expected {})",
            t.inscope
        );
        assert_eq!(
            cells.get(5).copied(),
            Some(t.faster.to_string().as_str()),
            "domain '{name}': ledger faster count drifted (expected {})",
            t.faster
        );
        assert_eq!(
            cells.get(6).copied(),
            Some(t.perfgated.to_string().as_str()),
            "domain '{name}': ledger perf-gated count drifted (expected {})",
            t.perfgated
        );
    }

    assert!(
        LEDGER_SRC.contains("do not edit by hand"),
        "PARITY_LEDGER.md lost its generated-artifact header"
    );
}
