//! signoff_period_lock — every hardened block's clock period lives in the compiler, is derived from
//! its measured SLOW corner, and is the only period a run may use.
//!
//! WHY. Until 2026-09-23 a block's period existed only as an argument on whichever command launched
//! it, and the recorder read only the typical corner. The two together let 16 of 18 blocks carry a
//! frequency they do not meet at ss_100C_1v60: `fabric_soc` was built at 300 ns and its slowest
//! corner needs 421.4 ns. Ruling (Tristen, 2026-09-23): signoff means EVERY corner closes, at a period
//! set to what the slow corner actually meets.
//!
//! So the period is data with a derivation, held in `codegen_sva::signoff_blocks`, and the flow
//! emitter refuses any other value for a registered block.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;
#[path = "common/sky130_ledgers.rs"]
mod sky130_ledgers;

use logicaffeine_compile::codegen_sva::signoff_blocks::{
    honest_period_ns, registered_period_ns, SIGNOFF_BLOCKS,
};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn a_block_that_meets_its_slow_corner_keeps_its_period() {
    assert_eq!(honest_period_ns(15.0, 0.14), 15.0);
    assert_eq!(honest_period_ns(15.0, 9.69), 15.0);
    assert_eq!(honest_period_ns(10.0, 0.0), 10.0);
}

/// A report printing `-0.00` missed by less than its resolution. That is a miss.
#[test]
fn a_negative_zero_slack_is_a_miss_not_a_pass() {
    assert_eq!(
        honest_period_ns(10.0, -0.0),
        10.5,
        "dram_executor's slow corner read -0.00"
    );
}

#[test]
fn a_failing_slow_corner_lengthens_the_period_by_the_miss_plus_margin() {
    // required = period - slack; +5% margin; rounded UP to the 0.5 ns grid.
    assert_eq!(
        honest_period_ns(300.0, -121.40),
        442.5,
        "fabric_soc: 421.40 * 1.05 = 442.47"
    );
    assert_eq!(
        honest_period_ns(10.0, -8.22),
        19.5,
        "pipe16: 18.22 * 1.05 = 19.13"
    );
    assert_eq!(
        honest_period_ns(15.0, -10.46),
        27.0,
        "gemm: 25.46 * 1.05 = 26.73"
    );
    assert_eq!(
        honest_period_ns(10.0, -0.44),
        11.0,
        "dram_executor_wb: 10.44 * 1.05 = 10.96"
    );
    for (p, s) in [(10.0, -0.01), (60.0, -3.58), (12.0, -7.27)] {
        let h = honest_period_ns(p, s);
        assert!(
            h >= (p - s) * 1.05 - 1e-9 && (h * 2.0).fract() == 0.0,
            "{p} ns at {s} ns slack -> {h}: must cover the miss plus margin, on the 0.5 ns grid"
        );
    }
}

/// Every block in every per-block signoff ledger has a registered period, and no registered
/// period is shorter than what that block's recorded slow corner requires.
#[test]
fn the_registry_covers_every_ledger_block_and_never_undercuts_its_slow_corner() {
    let mut seen = 0;
    for path in sky130_ledgers::ledgers(&root(), Some("signoff")) {
        let n = path.file_name().unwrap().to_string_lossy().into_owned();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let Some(blocks) = doc["blocks"].as_object() else {
            continue;
        };
        for (name, b) in blocks {
            seen += 1;
            let reg = registered_period_ns(name).unwrap_or_else(|| {
                panic!("{n}/{name} has no registered signoff period in codegen_sva::signoff_blocks")
            });
            if let (Some(p), Some(s)) = (
                b["clock_period_ns"].as_f64(),
                b["setup_worst_slack_ns"].as_f64(),
            ) {
                assert!(reg >= honest_period_ns(p, s) || (reg == p && s >= 0.0),
                    "{n}/{name}: registry says {reg} ns but the recorded run at {p} ns has slow-corner \
                     setup {s} ns, which needs {} ns", honest_period_ns(p, s));
            }
        }
    }
    assert!(
        seen >= 19,
        "found only {seen} ledger blocks — discovery is broken"
    );
    for b in SIGNOFF_BLOCKS.iter() {
        assert!(
            !b.basis.is_empty() && b.period_ns > 0.0,
            "{}: a period needs its measured basis",
            b.name
        );
    }
}

/// A period's basis is the run whose GDS the evidence ships — the block ledger's `gds_bytes`, or for the
/// Caravel user macro the wrapper ledger's `macro_gds_bytes` — at the period and worst slack the ledger
/// records. A basis left on a superseded run derives the period from a netlist nobody ships, and the
/// period then outlives the measurement that justified it.
#[test]
fn every_period_basis_is_the_run_its_evidence_ships() {
    if !common::require_tool("eda-tools") {
        return;
    }
    let designs = root().join("tools/OpenLane/designs");
    let json = |p: PathBuf| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap()
    };
    let mut shipped = std::collections::HashMap::new();
    for path in sky130_ledgers::ledgers(&root(), Some("signoff")) {
        if let Some(blocks) = json(path)["blocks"].as_object() {
            for (name, b) in blocks {
                shipped.insert(name.clone(), b.clone());
            }
        }
    }
    let upw = json(root().join("benchmarks/silicon/caravel_signoff.json"))["user_project_wrapper"]
        .clone();
    let gds_of = |run: &str, block: &str| -> Option<u64> {
        designs.read_dir().ok()?.flatten().find_map(|d| {
            std::fs::metadata(
                d.path()
                    .join("runs")
                    .join(run)
                    .join(format!("results/final/gds/{block}.gds")),
            )
            .ok()
            .map(|m| m.len())
        })
    };
    let mut wrong = Vec::new();
    let mut checked = 0;
    for b in SIGNOFF_BLOCKS.iter() {
        let field = |from: &str, to: &str| -> f64 {
            let at = b
                .basis
                .find(from)
                .map(|i| i + from.len())
                .unwrap_or_else(|| panic!("{}: basis `{}`", b.name, b.basis));
            b.basis[at..at + b.basis[at..].find(to).unwrap()]
                .parse()
                .unwrap()
        };
        let (slack, period) = (field("every corner ", " ns at"), field(" ns at ", " ns,"));
        let run = b.basis.rsplit(", ").next().unwrap();
        // A seeded block is not built yet: its evidence is the sibling's shipped run, and it has none of
        // its own — once a run of its own is recorded, the basis must move to it.
        let (evidence, seeded) = match b
            .basis
            .strip_prefix("seeded from ")
            .and_then(|r| r.split_once(':'))
        {
            Some((sibling, _)) => (sibling, true),
            None => (b.name, false),
        };
        if seeded && shipped.contains_key(b.name) {
            wrong.push(format!(
                "  {}: seeded from {evidence}, but its own run is recorded — move the basis to it",
                b.name
            ));
            continue;
        }
        let Some(bytes) = gds_of(run, evidence) else {
            wrong.push(format!("  {}: basis {run} holds no {evidence}.gds", b.name));
            continue;
        };
        checked += 1;
        if let Some(led) = shipped.get(evidence) {
            let want = (
                led["gds_bytes"].as_u64(),
                led["clock_period_ns"].as_f64(),
                led["setup_worst_slack_ns"].as_f64(),
            );
            if want.0 != Some(bytes)
                || want.1 != Some(period)
                || want.2.map_or(true, |w| (w - slack).abs() > 0.005)
            {
                wrong.push(format!("  {}: basis {run} ({bytes} bytes, {period} ns, slack {slack}) but the ledger ships \
                                    {:?} bytes at {:?} ns, slack {:?}", b.name, want.0, want.1, want.2));
            }
        } else if seeded {
            wrong.push(format!(
                "  {}: seeded from {evidence}, which no ledger ships",
                b.name
            ));
        } else if upw["macro"] == b.name {
            if upw["macro_gds_bytes"].as_u64() != Some(bytes) {
                wrong.push(format!(
                    "  {}: basis {run} ({bytes} bytes) but the wrapper ships {}",
                    b.name, upw["macro_gds_bytes"]
                ));
            }
        } else {
            wrong.push(format!("  {}: registered, but no ledger ships it", b.name));
        }
    }
    assert!(
        checked > 0,
        "tools/OpenLane/designs/ is populated but no basis run was found — the lookup is broken"
    );
    assert!(
        wrong.is_empty(),
        "a signoff period derived from a run the evidence does not ship:\n{}",
        wrong.join("\n")
    );
}

/// The emitter and the recorder are both wired to the registry.
#[test]
fn a_run_cannot_use_a_period_the_compiler_does_not_hold() {
    let emit = std::fs::read_to_string(
        root().join("crates/logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    assert!(
        emit.contains("registered_period_ns("),
        "emit_flow_config must check the requested period against the registry"
    );
    assert!(
        emit.contains("is not the registered signoff period"),
        "a mismatching period must be REFUSED with a message, not silently used"
    );
    let rec = std::fs::read_to_string(root().join("scripts/record-sky130-signoff.py")).unwrap();
    assert!(rec.contains("\"clock_period_ns\""),
        "the recorder must store the period the run was built at, or no lock can relate slack to it");
}
