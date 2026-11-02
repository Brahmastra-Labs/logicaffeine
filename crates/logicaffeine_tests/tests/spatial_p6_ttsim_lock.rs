//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! P6 LOCK — the purely compiler-generated Tensix program BEATS the vendor's production `ttnn.matmul` at
//! matched numerics, measured on Tenstorrent's simulator and recorded in the ttsim ledger.
//!
//! The IR levers that close the gap (sub-block compute granularity, packer-L1 accumulate, reader/writer role
//! fusion, per-site NOC balancing, operand multicast direction, two-level blocking, in-kernel padding) are
//! passes over the IR chosen by the cost model; this lock never names one. It reads the ledger:
//!   A. at 640³ on Wormhole, for every knob set at which a `ttnn_production` row exists, the `logos_spatial`
//!      row of the compiler's OWN choice spends no more of the MACHINE than every ttnn row at that knob set
//!      — NOC bytes, NOC transactions and Tensix instructions, and only where both retired the same
//!      arithmetic (ttnn's default pads 640³ by 10.25%, which is its own finding, not a ratio);
//!   B. the same holds at the 2048 class wherever ttnn rows exist;
//!   C. every such spatial row passes at ≥ its PCC floor and was generated end-to-end.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(spatial_p6_ttsim_lock)'`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/tt_ledger.rs"]
mod tt_ledger;

fn spatial_vs_ttnn(shape: &str) {
    let d = tt_ledger::ledger();
    let rows = tt_ledger::current_rows(&d);
    let chosen: std::collections::BTreeSet<String> = d["frontier"]
        .as_array()
        .map(|f| {
            f.iter()
                .filter_map(|e| e["compiler_chosen"]["row"].as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mut checked = 0;
    let knob_sets: std::collections::BTreeSet<String> = rows
        .iter()
        .filter(|r| {
            r["chip"] == "wormhole_b0"
                && tt_ledger::shape_str(r) == shape
                && r["stack"] == "ttnn_production"
        })
        .map(|r| tt_ledger::knobs_str(&r["knobs"]))
        .collect();
    assert!(!knob_sets.is_empty(), "{shape}: the ledger must hold a ttnn production row (the vendor bar) before P6 can be judged");
    for knobs in knob_sets {
        let ttnn: Vec<_> = rows
            .iter()
            .filter(|r| {
                r["chip"] == "wormhole_b0"
                    && tt_ledger::shape_str(r) == shape
                    && r["stack"] == "ttnn_production"
                    && tt_ledger::knobs_str(&r["knobs"]) == knobs
            })
            .collect();
        let spatial: Vec<_> = rows
            .iter()
            .filter(|r| {
                r["chip"] == "wormhole_b0"
                    && tt_ledger::shape_str(r) == shape
                    && r["program"] == "logos_spatial"
                    && r["strategy"] != "streamed-forced"
                    && tt_ledger::knobs_str(&r["knobs"]) == knobs
            })
            .collect();
        assert!(
            !spatial.is_empty(),
            "{shape} {knobs}: no IR-lowered row at the knobs the vendor was measured at"
        );
        // ★ ON THE WORK METER. This used to pick our lowest-CYCLE row and require it under ttnn's — and
        // `ttsim_instrument_audit.json` shows that counter is not a function of the program at all (8x the
        // reduction depth moves it 0.08%), so "beats" meant "won a coin toss". What the simulator can
        // report is what the machine DID, and the comparison is only licensed where both sides retired the
        // same arithmetic: ttnn's default heuristic pads 640^3 from 8,000 tile-matmuls to 8,820, and a
        // ratio against that row would compare two different computations.
        let work = |r: &serde_json::Value, f: &str| r["work"][f].as_u64();
        // ★ THE COMPILER'S OWN CHOICE, named by the ledger — not the best row in the sweep. Most rows at a
        // shape are arrangements pinned on the command line precisely BECAUSE the compiler did not pick
        // them; taking the minimum over all of them asks whether the MACHINE can beat the vendor, which is
        // a different and much weaker question than whether this compiler does it unaided.
        let ours = spatial
            .iter()
            .find(|r| chosen.contains(r["id"].as_str().unwrap_or_default()))
            .unwrap_or_else(|| panic!("{shape} {knobs}: the ledger names no compiler-chosen row at these knobs — every row is a pinned calibration point"));
        assert!(work(ours, "noc_bytes").is_some(), "{shape} {knobs}: the compiler-chosen row carries no work meter — re-measure on the instrumented simulator");
        assert_eq!(ours["pass"], true);
        assert!(ours["pcc"].as_f64().unwrap() >= ours["pcc_floor"].as_f64().unwrap());
        assert!(ours["program_origin"]
            .as_str()
            .unwrap()
            .contains("all kernels generated"));
        let (ob, ot, oi, oret) = (
            work(ours, "noc_bytes").unwrap(),
            work(ours, "noc_txns").unwrap(),
            work(ours, "tensix_inst").unwrap(),
            work(ours, "matmul_retired").unwrap(),
        );
        for t in &ttnn {
            let (Some(vb), Some(vt), Some(vi), Some(vret)) = (
                work(t, "noc_bytes"),
                work(t, "noc_txns"),
                work(t, "tensix_inst"),
                work(t, "matmul_retired"),
            ) else {
                continue;
            };
            if vret != oret {
                // The vendor computed a different amount — its own padding, recorded by
                // `tt_work_meter_lock::the_vendors_default_heuristic_pads_by_an_amount_that_is_the_chips_not_the_vendors`.
                // Not a comparison, so not judged here.
                continue;
            }
            assert!(
                ob <= vb && ot <= vt && oi <= vi,
                "{shape} {knobs}: at identical retired work ({oret}) the generated program must spend no more of the machine than ttnn ({}) — NOC bytes {ob} vs {vb}, transactions {ot} vs {vt}, instructions {oi} vs {vi}",
                t["program_origin"]
            );
            checked += 1;
        }
    }
    assert!(checked >= 1);
}

#[test]
fn the_generated_program_beats_ttnn_production_at_640_cubed_at_matched_knobs() {
    spatial_vs_ttnn("640x640x640");
}

#[test]
fn the_generated_program_beats_ttnn_production_at_the_2048_class_where_measured() {
    let d = tt_ledger::ledger();
    let shapes: std::collections::BTreeSet<String> = tt_ledger::current_rows(&d)
        .iter()
        .filter(|r| r["stack"] == "ttnn_production" && tt_ledger::shape_str(r).starts_with("2048x"))
        .map(tt_ledger::shape_str)
        .collect();
    for s in shapes {
        spatial_vs_ttnn(&s);
    }
}
