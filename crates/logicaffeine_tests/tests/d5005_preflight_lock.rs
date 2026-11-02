//! Preflight against what the D5005 build ledger and the timing calibration have measured (`codegen_sva::preflight`):
//! the PR region every persona was placed in is the capacity a build is held to, Quartus's own syntheses give the ALMs
//! an ALUT packs into, and the personas' fits give the fitter's time. A design over the region is refused before
//! Quartus runs, one past the fullest persona that fit is tight, and one well inside it goes, with its fit's time.

mod common;

use std::path::PathBuf;

use logicaffeine_compile::codegen_sva::preflight::{alms_per_alut, capacity, fitter_seconds, fullest, packing, preflight, Design, LoopBound, Target};
use logicaffeine_compile::codegen_sva::quartus::Ledger;
use logicaffeine_compile::codegen_sva::timing_graph::{Area, Calibration};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ledger() -> Ledger {
    Ledger::load(&root().join("benchmarks/silicon/d5005/quartus_builds.json")).unwrap()
}

fn calibration() -> Calibration {
    serde_json::from_str(&std::fs::read_to_string(root().join("benchmarks/silicon/d5005/timing_calibration.json")).unwrap()).unwrap()
}

/// fit27's measure of the model's delay on the card: 0.55 ns a LUT level.
const TARGET: Target = Target { period_ns: 10.0, ns_per_unit: 0.0055 };

/// A design the model maps to `luts` LUTs and nothing else, a deepest path of `depth` units, no loop.
fn design(luts: u64, depth: i64) -> Design {
    Design { area: Area { luts, ..Area::default() }, depth, loop_bound: LoopBound::Acyclic }
}

#[test]
fn the_capacity_is_the_fims_pr_region() {
    let c = capacity(&ledger()).expect("a persona recorded with its region");
    assert_eq!((c.alms, c.registers, c.m20k, c.dsp), (601_050, 2_404_200, 7_583, 3_971));
}

#[test]
fn quartus_packs_an_alut_into_between_a_half_and_three_quarters_of_an_alm() {
    let r = alms_per_alut(&ledger()).expect("syntheses to measure it from");
    assert!((0.5..=0.75).contains(&r), "{r}");
}

/// Quartus packs a design by whichever of its ALUTs and its registers sets its ALMs: the logic-bound syntheses the
/// ledger records pack between a half and three quarters of an ALM an ALUT, the register-bound ones (the memory probes
/// built in flip-flops) between a third and nine tenths of an ALM a register, and every synthesis's ALMs are the larger
/// of the two packings within a quarter.
#[test]
fn quartus_packs_a_register_bound_design_by_its_registers_and_every_synthesis_by_the_larger_term() {
    let ledger = ledger();
    let p = packing(&ledger).expect("syntheses to measure it from");
    assert_eq!(alms_per_alut(&ledger), Some(p.per_alut));
    let per_register = p.per_register.expect("a register-bound synthesis");
    assert!((0.33..=0.9).contains(&per_register), "{per_register}");
    let mut seen = 0;
    for r in ledger.rows.iter().filter(|r| r.synthesis_sources().is_some()) {
        let (Some(alms), Some(top)) = (r.build.synthesis.first().and_then(|e| e.estimate.alms), r.build.top.iter().find(|n| n.path == "|")) else { continue };
        let predicted = p.alms(top.aluts as f64, top.registers as f64);
        let ratio = predicted / alms as f64;
        assert!((0.75..=1.0 / 0.75).contains(&ratio), "{}: {predicted:.0} predicted, {alms} synthesized", r.name);
        seen += 1;
    }
    assert!(seen >= 10, "{seen} syntheses");
}

#[test]
fn the_fitters_time_is_predicted_within_a_factor_of_two_of_every_persona_it_was_fitted_to() {
    let ledger = ledger();
    let mut seen = 0;
    for r in ledger.rows.iter().filter(|r| r.build.flow.revision.as_deref() == Some("iofs_pr_afu")) {
        let persona = r.build.regions.iter().find(|g| g.name == "afu_top|port_gasket|pr_slot|afu_main");
        let (Some(region), Some(fitter)) = (persona, r.build.flow.phase("Fitter")) else { continue };
        let predicted = fitter_seconds(&ledger, region.alms_needed).unwrap();
        let ratio = predicted / fitter.seconds as f64;
        assert!((0.5..=2.0).contains(&ratio), "{}: {predicted:.0} s predicted, {} s fitted", r.name, fitter.seconds);
        seen += 1;
    }
    assert!(seen >= 4, "{seen}");
}

#[test]
fn a_design_over_the_region_is_refused_past_the_fullest_fit_is_tight_and_well_inside_goes() {
    let (ledger, calibration) = (ledger(), calibration());
    let (fit, ratio) = (calibration.area.clone().unwrap(), alms_per_alut(&ledger).unwrap());
    let luts_for = |alms: f64| (alms / ratio / fit.scale) as u64;
    let (fullest_row, fullest_alms) = fullest(&ledger).expect("a persona that fit");
    assert_eq!(fullest_row, "hybrid3b-signoff");
    let over = preflight(&design(luts_for(700_000.0), 1_000), &ledger, &calibration, &TARGET);
    assert_eq!(over.refusals.iter().map(|r| r.what()).collect::<Vec<_>>(), ["ALMs"], "{:?}", over.refusals);
    let tight = preflight(&design(luts_for((fullest_alms + 601_050.0) / 2.0), 1_000), &ledger, &calibration, &TARGET);
    assert!(tight.refusals.is_empty() && tight.tight.len() == 1, "{:?} {:?}", tight.refusals, tight.tight);
    let go = preflight(&design(luts_for(200_000.0), 1_500), &ledger, &calibration, &TARGET);
    assert!(go.refusals.is_empty() && go.tight.is_empty(), "{:?} {:?}", go.refusals, go.tight);
    assert!((go.predicted.alms - 200_000.0).abs() / 200_000.0 < 0.01, "{:?}", go.predicted);
    let hours = go.predicted.fitter_seconds.unwrap() / 3_600.0;
    assert!(hours > 0.5 && hours < 8.0, "a 200k-ALM persona's fit: {hours:.1} h");
    let slow = preflight(&design(luts_for(200_000.0), 2_000), &ledger, &calibration, &TARGET);
    assert_eq!(slow.refusals.iter().map(|r| r.what()).collect::<Vec<_>>(), ["the deepest path"], "11 ns is past a 10 ns clock");
}

/// A design whose adds are the card's floating-point DSP blocks preflights with each block one DSP block and its
/// paths ending and starting at the block, the device's primitive given by the library as a box.
#[test]
fn a_design_on_the_cards_floating_point_blocks_counts_a_dsp_block_each() {
    use logicaffeine_compile::codegen_sva::fp_hard::{hard_verilog, Block, Op};
    use logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics;
    use logicaffeine_compile::codegen_sva::preflight::device_library;
    use logicaffeine_compile::codegen_sva::timing_graph::{synthesize, Delays};
    if !common::require_tool("yosys") {
        return;
    }
    let wrapper = hard_verilog(Op::Add, "logos_fadd_p", &Block { semantics: Semantics::ENGINE, full: 3, least: 1 }, 4).unwrap();
    let pair = format!("{wrapper}module two_adds(input clk, input [31:0] a, b, c, output [31:0] y);\n  wire [31:0] s;\n  logos_fadd_p u0(.clk(clk), .a(a), .b(b), .y(s));\n  logos_fadd_p u1(.clk(clk), .a(s), .b(c), .y(y));\nendmodule\n");
    let g = synthesize(&format!("{pair}{}", device_library(&pair)), "two_adds", &[], &[], &Delays::LEVELS).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(g.boxes.iter().filter(|b| b.module == "fourteennm_fp_mac").count(), 2, "{:?}", g.boxes);
    assert_eq!(g.area_under("").half_dsps, 4, "a whole DSP block each");
    assert_eq!(Design::of(&g).unwrap().area.half_dsps.div_ceil(2), 2);
}

