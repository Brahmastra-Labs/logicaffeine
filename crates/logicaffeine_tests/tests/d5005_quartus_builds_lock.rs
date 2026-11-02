//! The D5005 Quartus build ledger, `benchmarks/silicon/d5005/quartus_builds.json`: every compile on the card's device
//! recorded by `quartus_record` from its own reports (`codegen_sva::quartus`), the failed ones with the rest, so the
//! next build is priced from what the last ones cost. The file is the ledger's own serialization, never edited by
//! hand.

use std::collections::HashSet;
use std::path::PathBuf;

use logicaffeine_compile::codegen_sva::quartus::{Ledger, Ratio};

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005/quartus_builds.json")
}

fn ledger() -> Ledger {
    Ledger::load(&path()).expect("the ledger reads as a Ledger")
}

#[test]
fn the_file_is_the_ledgers_own_serialization() {
    let text = std::fs::read_to_string(path()).unwrap();
    let mut canonical = serde_json::to_string_pretty(&ledger()).unwrap();
    canonical.push('\n');
    assert!(text == canonical, "quartus_builds.json is written by quartus_record, not by hand");
}

#[test]
fn every_compile_is_named_once() {
    let l = ledger();
    assert!(!l.rows.is_empty());
    let mut seen = HashSet::new();
    for r in &l.rows {
        assert!(seen.insert(r.name.as_str()), "{} is recorded twice", r.name);
        assert!(!r.name.is_empty() && !r.name.contains(char::is_whitespace), "{:?}", r.name);
    }
}

#[test]
fn every_recording_is_stamped_and_its_inputs_hashed() {
    for r in &ledger().rows {
        let t = r.recorded.as_bytes();
        assert!(
            t.len() == 20 && t[4] == b'-' && t[7] == b'-' && t[10] == b'T' && t[13] == b':' && t[16] == b':' && t[19] == b'Z',
            "{}: {:?} is an RFC 3339 UTC stamp",
            r.name,
            r.recorded
        );
        let mut paths = HashSet::new();
        for i in &r.inputs {
            assert!(
                i.sha256.len() == 64 && i.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "{}: {} has a SHA-256",
                r.name,
                i.path
            );
            assert!(paths.insert(i.path.as_str()), "{}: {} named twice", r.name, i.path);
        }
    }
}

#[test]
fn a_compiles_total_time_is_its_phases_summed() {
    for r in &ledger().rows {
        let phases = &r.build.flow.phases;
        let total = phases.iter().find(|p| p.name == "Total").unwrap_or_else(|| panic!("{}: no total", r.name));
        let sum: u64 = phases.iter().filter(|p| p.name != "Total").map(|p| p.seconds).sum();
        assert_eq!(total.seconds, sum, "{}", r.name);
        assert_eq!(total.peak_mb, None, "{}: the total carries no memory peak", r.name);
        assert!(phases.iter().filter(|p| p.name != "Total").all(|p| p.peak_mb.is_some()), "{}", r.name);
    }
}

#[test]
fn nothing_is_used_past_what_the_device_holds() {
    let within = |name: &str, what: &str, r: Option<Ratio>| {
        if let Some(r) = r {
            assert!(r.used <= r.total && r.total > 0, "{name}: {what} {}/{}", r.used, r.total);
        }
    };
    for r in &ledger().rows {
        if let Some(f) = &r.build.fit {
            within(&r.name, "ALMs", f.alms);
            within(&r.name, "memory bits", f.block_memory_bits);
            within(&r.name, "RAM blocks", f.ram_blocks);
            within(&r.name, "DSP blocks", f.dsp_blocks);
        }
        for p in &r.build.synthesis {
            let e = &p.estimate;
            if let (Some(all), Some(fixed), Some(float)) = (e.dsp_blocks, e.fixed_point_dsp, e.floating_point_dsp) {
                assert_eq!(fixed + float, all, "{} {}: the DSP split adds up", r.name, p.partition);
            }
        }
    }
}

#[test]
fn every_synthesis_estimate_says_how_many_floating_point_dsps_it_used() {
    // A synthesis that failed writes no estimate: it is the one kind of row without one.
    let failed = |r: &&logicaffeine_compile::codegen_sva::quartus::Row| r.build.flow.status.as_deref().is_some_and(|s| s.starts_with("Synthesis Failed"));
    let ledger = ledger();
    for r in ledger.rows.iter().filter(failed) {
        assert!(r.build.synthesis.is_empty(), "{}: a failed synthesis estimated nothing", r.name);
    }
    for r in ledger.rows.iter().filter(|r| !failed(r)) {
        assert!(!r.build.synthesis.is_empty(), "{}: a synthesis estimate", r.name);
        for p in &r.build.synthesis {
            assert!(p.estimate.floating_point_dsp.is_some(), "{} {}", r.name, p.partition);
        }
    }
}

#[test]
fn the_entity_rollups_are_largest_first() {
    for r in &ledger().rows {
        let e = &r.build.entities;
        assert!(e.windows(2).all(|w| w[0].aluts >= w[1].aluts), "{}", r.name);
        assert!(e.len() <= logicaffeine_compile::codegen_sva::quartus::ENTITY_ROWS, "{}", r.name);
    }
}

#[test]
fn failed_compiles_stay_on_record() {
    let l = ledger();
    let failed: Vec<&str> = l.rows.iter().filter(|r| !r.build.flow.succeeded()).map(|r| r.name.as_str()).collect();
    assert!(failed.contains(&"engine1-pr"), "the engine build the PIM refused is recorded: {failed:?}");
    assert!(l.rows.iter().filter(|r| !r.build.flow.succeeded()).all(|r| r.build.flow.status.is_some()));
}

#[test]
fn every_row_is_judged_and_its_verdict_holds_what_its_reports_say() {
    use logicaffeine_compile::codegen_sva::quartus::{judge, Evidence};
    for r in &ledger().rows {
        let v = &r.verdict;
        assert_eq!(v.passed, v.failures.is_empty(), "{}: a verdict passes exactly when nothing failed", r.name);
        let from_reports = judge(&r.build, &Evidence { exit: v.exit, expected: v.expected.clone(), ..Evidence::default() });
        for f in &from_reports.failures {
            assert!(v.failures.contains(f), "{}: the reports and the recorded evidence say `{f}`, the verdict does not", r.name);
        }
        assert!(v.expected.iter().all(|m| !m.is_empty() && m != "Total"), "{}: {:?}", r.name, v.expected);
    }
}

#[test]
fn a_build_whose_fitter_never_ran_is_on_record_as_failed() {
    let l = ledger();
    for name in ["hybrid2b-pr", "hybrid2b-fit"] {
        let r = l.row(name).unwrap_or_else(|| panic!("{name} is recorded"));
        assert!(r.build.flow.succeeded(), "{name}: its flow report, written after synthesis, says Successful");
        assert!(!r.verdict.passed, "{name}: but the fitter it runs never ran");
        assert!(r.verdict.failures.iter().any(|f| f.contains("Fitter")), "{name}: {:?}", r.verdict.failures);
        assert!(r.verdict.failures.iter().any(|f| f.starts_with("the log: ")), "{name}: its log's failure line is kept");
    }
}

#[test]
fn no_router_estimate_is_counted_twice() {
    for r in &ledger().rows {
        let w = &r.build.wire_demand;
        for (i, a) in w.iter().enumerate() {
            assert!(!w[i + 1..].contains(a), "{}: {a:?} twice", r.name);
        }
    }
}

#[test]
fn every_persona_and_fit_is_judged_against_the_modules_it_runs() {
    // Judged on what its job ran: a job that went on from another compile's fit is judged without the fitter, and the
    // compile it names as resumed, recorded before it from the same tree, ran that fitter.
    let ledger = ledger();
    for (k, r) in ledger.rows.iter().enumerate() {
        let ran_a_fitter = r.build.flow.phase("Fitter").is_some() || r.name.ends_with("-pr") || r.name.ends_with("-fit");
        if !ran_a_fitter || r.verdict.expected.iter().any(|m| m == "Fitter") {
            continue;
        }
        let from = r.resumed.as_deref().unwrap_or_else(|| panic!("{}: judged without the fitter it runs, and resumed nothing", r.name));
        // A job that went on from its own earlier attempt: its own flow ran the fitter.
        if from == r.name {
            assert!(r.build.flow.phase("Fitter").is_some(), "{}: resumed from itself, its own flow ran the fitter", r.name);
            continue;
        }
        let at = ledger.rows.iter().position(|e| e.name == from).unwrap_or_else(|| panic!("{}: resumed {from}, which is not on record", r.name));
        let earlier = &ledger.rows[at];
        assert!(at < k, "{}: resumed {from}, recorded after it", r.name);
        assert!(earlier.verdict.expected.iter().any(|m| m == "Fitter"), "{}: resumed {from}, which did not run the fitter either", r.name);
        let paths = |row: &logicaffeine_compile::codegen_sva::quartus::Row| row.inputs.iter().map(|i| i.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(earlier), paths(r), "{}: resumed {from}, from another tree", r.name);
    }
}

#[test]
fn every_fit_records_the_hold_delay_it_cost_and_the_clocks_it_left_off_the_globals() {
    // A fit that placed counts its ALMs; one that failed before placing wrote no control-signal table.
    for r in ledger().rows.iter().filter(|r| r.build.fit.as_ref().is_some_and(|f| f.alms.is_some())) {
        let Some(nets) = &r.build.clock_nets else { panic!("{}: a placed fit reads its clock nets", r.name) };
        assert!(!nets.is_empty(), "{}: a fit clocks something", r.name);
        assert!(
            nets.windows(2).all(|w| (w[0].global, std::cmp::Reverse(w[0].fanout)) <= (w[1].global, std::cmp::Reverse(w[1].fanout))),
            "{}: the clocks off every global first, then by fan-out",
            r.name
        );
        if r.build.flow.phase("Fitter").is_some() && r.build.clocks.iter().any(|c| c.analysis == "Hold") {
            let hold = r.build.hold.as_ref().unwrap_or_else(|| panic!("{}: a routed fit states the delay it added for hold", r.name));
            assert!(hold.iter().all(|h| h.ns >= 0.0 && !h.source.is_empty() && !h.destination.is_empty()), "{}: {hold:?}", r.name);
        }
    }
    let engine2 = ledger().row("engine2-pr").cloned().expect("engine2 is on record");
    let off: Vec<&str> = engine2.build.clock_nets.iter().flatten().filter(|c| !c.global).map(|c| c.name.as_str()).collect();
    assert!(off.iter().any(|n| n.ends_with("compute_clock|cclk")), "engine2's divided clock rode general routing: {off:?}");
}

#[test]
fn every_persona_fit_reread_since_regions_records_the_one_pr_region_it_was_placed_in() {
    // The FIM's PR region is the persona's capacity, the one preflight holds a build to: six rectangles, the same in
    // every persona, and a persona that fit needed no more than it holds. A flat image's fit (the FIM's own revision)
    // records its floorplan's Logic Lock regions instead: each a `logos_*` region, one rectangle, nothing over it.
    let ledger = ledger();
    let personas: Vec<_> =
        ledger.rows.iter().filter(|r| r.build.flow.revision.as_deref() == Some("iofs_pr_afu") && !r.build.regions.is_empty()).collect();
    assert!(personas.len() >= 4, "the persona fits read since regions were: {}", personas.len());
    let region = &personas[0].build.regions[0];
    assert_eq!(region.name, "afu_top|port_gasket|pr_slot|afu_main");
    assert_eq!(region.placement.len(), 6, "{:?}", region.placement);
    for r in &personas {
        assert_eq!(r.build.flow.revision.as_deref(), Some("iofs_pr_afu"), "{}: a persona's fit", r.name);
        let [g] = r.build.regions.as_slice() else { panic!("{}: one region", r.name) };
        assert_eq!((&g.name, g.alms, &g.placement), (&region.name, region.alms, &region.placement), "{}: the same region", r.name);
        for ratio in [g.registers, g.m20k, g.dsp].into_iter().flatten() {
            assert!(ratio.used <= ratio.total, "{}: {ratio:?}", r.name);
        }
        if r.verdict.passed {
            assert!(g.alms_needed <= g.alms as f64, "{}: {} ALMs needed of {}", r.name, g.alms_needed, g.alms);
        }
    }
    for r in ledger.rows.iter().filter(|r| r.build.flow.revision.as_deref() == Some("d5005")) {
        for g in &r.build.regions {
            assert!(g.name.starts_with("logos_"), "{}: a floorplan's region, not {}", r.name, g.name);
            assert_eq!(g.placement.len(), 1, "{} {}: one rectangle", r.name, g.name);
            for ratio in [g.registers, g.m20k, g.dsp].into_iter().flatten() {
                assert!(ratio.used <= ratio.total, "{} {}: {ratio:?}", r.name, g.name);
            }
        }
    }
}
