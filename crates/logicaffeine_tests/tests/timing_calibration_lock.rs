//! The timing model held against Quartus: `benchmarks/silicon/d5005/timing_calibration.json` is what `timing_calibrate
//! --fit` records from the D5005 build ledger (`quartus_builds.json`), never edited by hand. Each synthesis the ledger
//! records from one Verilog file is mapped again from that file through `codegen_sva::timing_graph`, at the delays fitted
//! to every build's top-level Max Depth, beside the Max Depth Quartus reported for each instance. The model's depth is
//! what preflight and the auto-pipeliner price a design by, so the fit's spread is held here: every build's top within a
//! factor of two of Quartus, and the root mean square of the log ratios within a quarter. Each instance's area is held
//! beside Quartus's count of it, and the ALUTs the model predicts from its LUTs and carry bits (`fit_area`) within a
//! factor of two of Quartus's for every instance of the size a fit is drawn from.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use logicaffeine_compile::codegen_sva::quartus::Ledger;
use logicaffeine_compile::codegen_sva::timing_graph::{fit_area, Area, Calibration, CalibrationRow, InstanceArea, AREA_CARRIES};

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005/timing_calibration.json")
}

fn calibration() -> Calibration {
    let text = std::fs::read_to_string(path()).expect("the calibration is checked in");
    serde_json::from_str(&text).expect("the calibration reads as a Calibration")
}

fn ledger() -> Ledger {
    Ledger::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005/quartus_builds.json"))
        .expect("the ledger reads as a Ledger")
}

/// Each row's top: Quartus's Max Depth and the model's, in Quartus's unit.
fn tops(c: &Calibration) -> Vec<(&CalibrationRow, f64, f64)> {
    c.rows
        .iter()
        .map(|r| {
            let top = r.top().unwrap_or_else(|| panic!("{}: the top is calibrated", r.build));
            (r, top.quartus, top.predicted as f64 / 100.0)
        })
        .collect()
}

#[test]
fn the_file_is_the_calibrations_own_serialization() {
    let text = std::fs::read_to_string(path()).unwrap();
    let mut again = serde_json::to_string_pretty(&calibration()).unwrap();
    again.push('\n');
    assert!(text == again, "timing_calibration.json is written by timing_calibrate, never by hand");
}

#[test]
fn every_synthesis_the_ledger_records_is_calibrated_or_refused_once_from_the_verilog_it_compiled() {
    let ledger = ledger();
    let sources: BTreeMap<&str, &str> = ledger
        .rows
        .iter()
        .filter_map(|r| r.synthesis_sources().map(|(v, _)| (r.name.as_str(), v.sha256.as_str())))
        .collect();
    assert!(!sources.is_empty(), "the ledger records syntheses to calibrate against");
    let c = calibration();
    let named: Vec<(&str, &str)> = c
        .rows
        .iter()
        .map(|r| (r.build.as_str(), r.verilog_sha256.as_str()))
        .chain(c.refused.iter().map(|r| (r.build.as_str(), r.verilog_sha256.as_str())))
        .collect();
    let once: BTreeSet<&str> = named.iter().map(|(b, _)| *b).collect();
    assert_eq!(once.len(), named.len(), "each build calibrated or refused once: {named:?}");
    assert_eq!(named.iter().copied().collect::<BTreeMap<_, _>>(), sources, "each build by the Verilog the ledger hashed");
}

#[test]
fn every_row_names_the_top_its_build_synthesized() {
    let ledger = ledger();
    for r in &calibration().rows {
        let build = &ledger.row(&r.build).unwrap().build;
        let top = build.top.iter().find(|n| n.path == "|").unwrap_or_else(|| panic!("{}: the ledger holds its top", r.build));
        assert_eq!(top.entity, r.top, "{}", r.build);
    }
}

#[test]
fn every_build_is_timed_at_the_one_set_of_delays_fitted() {
    let c = calibration();
    let first = c.rows.first().expect("a calibrated build").delays;
    assert!(first.lut > 0, "a LUT costs depth: {first:?}");
    assert!([first.carry, first.dsp, first.memory].iter().all(|&d| d >= 0), "{first:?}");
    for r in &c.rows {
        assert_eq!(r.delays, first, "{}: one fit for every build", r.build);
    }
}

#[test]
fn every_instance_the_ledger_records_has_the_max_depth_it_reported() {
    let ledger = ledger();
    for r in &calibration().rows {
        let build = &ledger.row(&r.build).unwrap().build;
        let calibrated: BTreeMap<&str, f64> = r.instances.iter().map(|i| (i.path.as_str(), i.quartus)).collect();
        let mut matched = 0;
        for n in &build.top {
            let path = if n.path == "|" { "" } else { n.path.as_str() };
            if let (Some(&q), Some(d)) = (calibrated.get(path), n.max_depth) {
                assert_eq!(q, d, "{} {path:?}: the Max Depth the ledger read", r.build);
                matched += 1;
            }
        }
        assert!(matched > 0, "{}: the top at least", r.build);
        let paths: BTreeSet<&str> = r.instances.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths.len(), r.instances.len(), "{}: each instance once", r.build);
        // An instance Quartus maps wholly into DSP blocks (`logos_dsp_sum2`) has no logic level of its own.
        assert!(r.instances.iter().all(|i| i.quartus >= 0.0 && i.predicted > 0), "{}: no depth negative", r.build);
        assert!(r.top().is_some_and(|t| t.quartus > 0.0), "{}: the top has depth", r.build);
    }
}

#[test]
fn every_top_is_predicted_within_a_factor_of_two_of_quartus() {
    for (r, quartus, model) in tops(&calibration()) {
        let ratio = model / quartus;
        assert!((0.5..=2.0).contains(&ratio), "{}: the model says {model:.1}, Quartus {quartus:.1}", r.build);
    }
}

#[test]
fn the_fit_is_centred_and_its_spread_within_a_quarter() {
    let logs: Vec<f64> = tops(&calibration()).iter().map(|(_, q, m)| (m / q).ln()).collect();
    let mean = logs.iter().sum::<f64>() / logs.len() as f64;
    let rms = (logs.iter().map(|l| l * l).sum::<f64>() / logs.len() as f64).sqrt();
    assert!(mean.abs() <= 0.05, "the fitted scale leaves the model unbiased: mean log ratio {mean:.4}");
    assert!(rms <= 0.25, "root mean square log ratio {rms:.4}");
}

#[test]
fn every_refusal_says_what_refused_it() {
    for r in &calibration().refused {
        assert!(!r.why.trim().is_empty(), "{}", r.build);
        assert_eq!(r.verilog_sha256.len(), 64, "{}", r.build);
    }
}

#[test]
fn every_instance_holds_its_area_both_ways_and_quartus_counts_the_ledgers() {
    let ledger = ledger();
    for r in &calibration().rows {
        let build = &ledger.row(&r.build).unwrap().build;
        let calibrated: BTreeMap<&str, &InstanceArea> =
            r.instances.iter().map(|i| (i.path.as_str(), i.area.as_ref().unwrap_or_else(|| panic!("{} {:?}: an area", r.build, i.path)))).collect();
        for n in &build.top {
            let path = if n.path == "|" { "" } else { n.path.as_str() };
            if let Some(area) = calibrated.get(path) {
                let q = &area.quartus;
                assert_eq!((q.aluts, q.registers, q.dsp, q.memory_bits), (n.aluts, n.registers, n.dsp, n.memory_bits), "{} {path:?}", r.build);
            }
        }
    }
}

#[test]
fn the_area_fit_recorded_is_the_fit_of_the_recorded_areas() {
    let c = calibration();
    let fit = c.area.as_ref().expect("the calibration records its area fit");
    let samples: Vec<(Area, u64)> =
        c.rows.iter().flat_map(|r| r.instances.iter()).filter_map(|i| i.area.map(|a| (a.model, a.quartus.aluts))).collect();
    let again = fit_area(&samples, fit.floor, &AREA_CARRIES);
    assert_eq!(&again, fit, "the fit is recomputed from the rows");
}

#[test]
fn every_large_instance_is_predicted_within_a_factor_of_two_and_the_spread_within_a_seventh() {
    let c = calibration();
    let fit = c.area.as_ref().unwrap();
    assert!(fit.instances >= 30, "{fit:?}");
    assert!(fit.error <= 0.15, "root mean square log ratio {:.4}", fit.error);
    for r in &c.rows {
        for i in &r.instances {
            let a = i.area.unwrap();
            if a.quartus.aluts >= fit.floor {
                let ratio = fit.aluts(&a.model) / a.quartus.aluts as f64;
                assert!((0.5..=2.0).contains(&ratio), "{} {:?}: {:.0} predicted, {} by Quartus", r.build, i.path, fit.aluts(&a.model), a.quartus.aluts);
            }
        }
    }
}
