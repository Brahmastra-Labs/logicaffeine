//! energy_efficiency_lock — the Vdd/energy characterization is DERIVED from measured silicon, never fabricated.
//!
//! HiVec reports energy efficiency with a Vdd sweep + normalized TOPS/W + a SOTA table. This lock holds our
//! equivalent (`benchmarks/silicon/energy_efficiency.json`) to that bar WITHOUT trusting any hand-typed number:
//!  * the typical-corner efficiency must equal what its own measured power + fmax + op count imply (E = P/f,
//!    TOPS/W = ops·f/P) — a fabricated figure fails the derivation;
//!  * the Vdd sweep must be monotone and track the V² switching law the measured corners themselves state;
//!  * every LOGOS efficiency claim in the SOTA table must trace back to the measured corner (no unbacked
//!    number), and every external reference row must cite a source;
//!  * the area breakdown must partition the tile count.
//!
//! The pure JSON validation runs everywhere; the underlying OpenSTA Vdd sweep runs on the bench box
//! (`scripts/characterize-vdd-sweep.sh`). Run: `cargo nextest run -p logicaffeine-tests -E 'binary(energy_efficiency_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn data() -> serde_json::Value {
    let p = repo_root().join("benchmarks/silicon/energy_efficiency.json");
    let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"));
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("parse energy_efficiency.json: {e}"))
}

fn f(v: &serde_json::Value) -> f64 {
    v.as_f64()
        .unwrap_or_else(|| panic!("expected a number, got {v}"))
}

/// Relative closeness (the JSON is allowed light rounding).
fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() / b.abs() < tol
}

#[test]
fn the_typical_corner_efficiency_is_derived_from_measured_power_and_frequency() {
    let d = data();
    let ops = f(&d["ops_per_eval"]);
    let tc = &d["typical_corner"];
    let fmax_hz = f(&tc["fmax_mhz"]) * 1e6;
    let power_w = f(&tc["power_mw"]) * 1e-3;

    let energy_pj = power_w / fmax_hz * 1e12; // E = P / f, per single-cycle eval
    let tops_per_w = ops * fmax_hz / power_w / 1e12; // ops·f / P

    assert!(
        close(f(&tc["energy_per_eval_pj"]), energy_pj, 0.02),
        "energy/eval not derived from P/f: json {} vs computed {energy_pj:.3} pJ",
        f(&tc["energy_per_eval_pj"])
    );
    assert!(
        close(f(&tc["tops_per_w"]), tops_per_w, 0.02),
        "tops/w not derived from ops·f/P: json {} vs computed {tops_per_w:.4}",
        f(&tc["tops_per_w"])
    );
}

#[test]
fn the_vdd_sweep_is_monotone_and_tracks_the_v_squared_switching_law() {
    let d = data();
    let sweep = d["vdd_sweep"].as_array().expect("vdd_sweep array");
    assert!(
        sweep.len() >= 3,
        "a Vdd sweep needs at least three corners (got {})",
        sweep.len()
    );

    for w in sweep.windows(2) {
        assert!(
            f(&w[1]["vdd"]) > f(&w[0]["vdd"]),
            "vdd_sweep must be sorted ascending in vdd"
        );
        assert!(
            f(&w[1]["power_mw"]) > f(&w[0]["power_mw"]),
            "power must increase with Vdd across the sweep"
        );
    }

    // Dynamic power scales ~ C·V²; the measured end-to-end power ratio must track (Vhi/Vlo)² within the slack
    // that C/temperature variation across the corners allows.
    let (lo, hi) = (&sweep[0], sweep.last().unwrap());
    let v2 = (f(&hi["vdd"]) / f(&lo["vdd"])).powi(2);
    let pr = f(&hi["power_mw"]) / f(&lo["power_mw"]);
    assert!(
        (pr / v2) > 0.85 && (pr / v2) < 1.15,
        "power ratio {pr:.3} must track the V² prediction {v2:.3} (ratio {:.3})",
        pr / v2
    );
}

#[test]
fn every_logos_efficiency_claim_traces_to_a_measured_corner_and_externals_are_cited() {
    let d = data();
    let tc_tops = f(&d["typical_corner"]["tops_per_w"]);
    let die = f(&d["die_mm2"]);
    let sota = d["sota_comparison"]
        .as_array()
        .expect("sota_comparison array");

    let (mut saw_logos, mut saw_external) = (false, false);
    for row in sota {
        let source = row["source"].as_str().unwrap_or("");
        assert!(
            !source.is_empty(),
            "every SOTA row must cite a source: {row}"
        );
        let platform = row["platform"].as_str().unwrap_or("");
        if platform.contains("LOGOS") {
            saw_logos = true;
            assert!(
                close(f(&row["tops_per_w"]), tc_tops, 0.02),
                "a LOGOS SOTA figure must trace to the measured corner: {} vs {tc_tops}",
                f(&row["tops_per_w"])
            );
            assert!(
                (f(&row["area_mm2"]) - die).abs() < 1e-6,
                "the LOGOS area must be the signoff die area"
            );
        } else {
            saw_external = true;
        }
    }
    assert!(
        saw_logos,
        "the SOTA table must include the measured LOGOS point"
    );
    assert!(
        saw_external,
        "the SOTA table must include at least one cited external reference"
    );
}

#[test]
fn the_area_breakdown_partitions_the_tile_count() {
    let d = data();
    let ab = &d["area_breakdown"];
    let total = f(&ab["tiles_total"]) as u64;
    let parts: f64 = ab["tiles"]
        .as_object()
        .expect("tiles object")
        .values()
        .map(f)
        .sum();
    assert_eq!(
        total, parts as u64,
        "the tile breakdown must sum to tiles_total"
    );
    assert!(f(&d["die_mm2"]) > 0.0, "die area must be positive");
}
