use super::*;
use crate::codegen_sva::quartus::{Build, Flow, Input, Node, PartitionEstimate, Estimate, Phase, Ratio, RegionUsage, Row, Verdict};
use crate::codegen_sva::timing_graph::{AreaFit, Calibration};

fn region(alms_needed: f64) -> RegionUsage {
    RegionUsage {
        name: "afu_top|port_gasket|pr_slot|afu_main".into(),
        alms_needed,
        alms: 600_000,
        registers: Some(Ratio { used: 100, total: 2_400_000 }),
        m20k: Some(Ratio { used: 10, total: 7_500 }),
        dsp: Some(Ratio { used: 1, total: 4_000 }),
        placement: vec![((0, 0), (10, 10))],
    }
}

/// A persona fit of `alms` ALMs that took `seconds` to fit.
fn persona(name: &str, alms: f64, seconds: u64) -> Row {
    let flow = Flow {
        revision: Some("iofs_pr_afu".into()),
        phases: vec![Phase { name: "Fitter".into(), seconds, peak_mb: Some(30_000) }],
        ..Flow::default()
    };
    Row {
        name: name.into(),
        recorded: "2026-10-23T00:00:00Z".into(),
        inputs: vec![Input { path: "a.v".into(), sha256: "00".repeat(32) }],
        build: Build { flow, regions: vec![region(alms)], ..Build::default() },
        verdict: Verdict { passed: true, ..Verdict::default() },
        resumed: None,
    }
}

/// A unit synthesis Quartus estimated at `alms` ALMs over `aluts` ALUTs.
fn unit(name: &str, alms: u64, aluts: u64) -> Row {
    let estimate = Estimate { alms: Some(alms), ..Estimate::default() };
    let top = Node { partition: "root".into(), path: "|".into(), entity: "u".into(), depth: 0, aluts, registers: 0, memory_bits: 0, dsp: 0, max_depth: None };
    Row {
        name: name.into(),
        recorded: "2026-10-23T00:00:00Z".into(),
        inputs: vec![Input { path: "u.v".into(), sha256: "00".repeat(32) }, Input { path: "u.qsf".into(), sha256: "11".repeat(32) }],
        build: Build { synthesis: vec![PartitionEstimate { partition: "root".into(), estimate }], top: vec![top], ..Build::default() },
        verdict: Verdict { passed: true, expected: vec!["Synthesis".into()], ..Verdict::default() },
        resumed: None,
    }
}

fn ledger() -> Ledger {
    Ledger { rows: vec![unit("u1", 600, 1_000), unit("u2", 600, 1_000), persona("small", 30_000.0, 3_000), persona("large", 480_000.0, 24_000)] }
}

fn calibration() -> Calibration {
    Calibration { area: Some(AreaFit { scale: 0.5, carry: 0.5, floor: 10_000, instances: 40, error: 0.1 }), ..Calibration::default() }
}

/// A design whose model maps to `luts` LUTs, `half_dsps` DSP halves, a deepest path of `depth` and a loop bound of
/// `cycle`, in the model's unit.
fn design(luts: u64, half_dsps: u64, depth: i64, cycle: Option<f64>) -> Design {
    Design { area: Area { luts, half_dsps, ..Area::default() }, depth, loop_bound: cycle.map_or(LoopBound::Acyclic, LoopBound::Ratio) }
}

const TARGET: Target = Target { period_ns: 10.0, ns_per_unit: 0.0055 };

#[test]
fn the_capacity_is_the_region_the_last_persona_was_placed_in() {
    let c = capacity(&ledger()).expect("a persona with a region");
    assert_eq!((c.alms, c.registers, c.m20k, c.dsp), (600_000, 2_400_000, 7_500, 4_000));
    assert_eq!(c.row, "large");
    let mut moved = ledger();
    moved.rows[2].build.regions[0].alms = 500_000;
    assert_eq!(capacity(&moved).unwrap().alms, 600_000, "a FIM's region is the last persona's, not an earlier one's");
    assert!(capacity(&Ledger::default()).is_none());
}

#[test]
fn quartus_packs_its_aluts_into_alms_at_the_ratio_its_own_syntheses_show() {
    assert!((alms_per_alut(&ledger()).unwrap() - 0.6).abs() < 1e-9);
    assert_eq!(alms_per_alut(&Ledger::default()), None);
}

#[test]
fn a_design_that_fits_and_closes_is_a_go_with_what_it_will_take() {
    // 600k LUTs → 300k ALUTs → 180k ALMs of 600k; a path of 1500 units = 8.25 ns under 10.
    let p = preflight(&design(600_000, 40, 1_500, Some(1_200.0)), &ledger(), &calibration(), &TARGET);
    assert!(p.refusals.is_empty(), "{:?}", p.refusals);
    assert!((p.predicted.aluts - 300_000.0).abs() < 1e-6 && (p.predicted.alms - 180_000.0).abs() < 1e-6, "{:?}", p.predicted);
    assert_eq!(p.predicted.dsp, 20);
    assert!((p.predicted.timing[0].depth_ns - 8.25).abs() < 1e-9);
    let fit = p.predicted.fitter_seconds.expect("a fitter time from the personas");
    assert!(fit > 3_000.0 && fit < 24_000.0, "between the two personas' fits: {fit}");
}

#[test]
fn each_thing_that_will_not_fit_or_close_is_refused_by_name() {
    let over = preflight(&design(2_400_000, 9_000, 2_000, Some(1_900.0)), &ledger(), &calibration(), &TARGET);
    let names: Vec<&str> = over.refusals.iter().map(Refusal::what).collect();
    assert_eq!(names, ["ALMs", "DSP blocks", "the deepest path", "a loop"], "{:?}", over.refusals);
    let looped = preflight(&design(1_000, 0, 500, None).with_loop(LoopBound::Combinational), &ledger(), &calibration(), &TARGET);
    assert_eq!(looped.refusals.iter().map(Refusal::what).collect::<Vec<_>>(), ["a loop"]);
    let unmeasured = preflight(&design(1_000, 0, 500, None), &Ledger::default(), &Calibration::default(), &TARGET);
    let names: Vec<&str> = unmeasured.refusals.iter().map(Refusal::what).collect();
    assert_eq!(names, ["the area fit", "the PR region"], "no data is no verdict");
    for r in &over.refusals {
        assert!(!r.to_string().is_empty());
    }
}

#[test]
fn a_design_past_the_fullest_persona_that_fit_but_inside_the_region_is_tight() {
    // 1.7M LUTs → 850k ALUTs → 510k ALMs: past 480k (the fullest that fit), inside 600k.
    let p = preflight(&design(1_700_000, 0, 1_000, None), &ledger(), &calibration(), &TARGET);
    assert!(p.refusals.is_empty(), "{:?}", p.refusals);
    assert_eq!(p.tight.len(), 1, "{:?}", p.tight);
    assert!(p.tight[0].contains("480000"), "{}", p.tight[0]);
}

#[test]
fn each_part_is_held_to_the_cycles_its_paths_have_and_the_area_to_the_whole() {
    // A core stepping once in 63 clocks with a 300 ns path, beside a fast side stepping every clock: the core goes,
    // the fast side's 11 ns path is refused, and the area is the whole design's, not the parts'.
    let core = design(10, 0, 300 * 182, Some(250.0 * 182.0));
    let fast = design(10, 0, 2_000, None);
    let whole = Area { luts: 600_000, ..Area::default() };
    let parts = [Timed { name: "core", design: &core, cycles: 63 }, Timed { name: "fast", design: &fast, cycles: 1 }];
    let p = preflight_domains(&whole, &parts, &ledger(), &calibration(), &TARGET);
    assert_eq!(p.refusals.len(), 1, "{:?}", p.refusals);
    assert!(matches!(&p.refusals[0], Refusal::Depth { part, .. } if part == "fast"), "{:?}", p.refusals);
    assert!(p.refusals[0].to_string().starts_with("fast: "), "{}", p.refusals[0]);
    assert!((p.predicted.aluts - 300_000.0).abs() < 1e-6, "the whole design's area: {:?}", p.predicted);
    assert_eq!(p.predicted.timing.iter().map(|t| (t.name.as_str(), t.cycles)).collect::<Vec<_>>(), [("core", 63), ("fast", 1)]);
    let slow = [Timed { name: "core", design: &core, cycles: 20 }];
    let q = preflight_domains(&whole, &slow, &ledger(), &calibration(), &TARGET);
    let names: Vec<&str> = q.refusals.iter().map(Refusal::what).collect();
    assert_eq!(names, ["the deepest path", "a loop"], "300 ns and a 250 ns loop past 20 periods: {:?}", q.refusals);
}

/// A design on the card's floating-point DSP blocks names the device's primitive without defining it: the library
/// gives it the block's ports as a box one DSP block large, and gives nothing to a design that defines it or uses none.
#[test]
fn a_design_on_the_cards_floating_point_blocks_is_given_the_block_as_a_box() {
    use crate::codegen_sva::fp_hard::{hard_verilog, Block, Op};
    use crate::codegen_sva::fp_hypotheses::Semantics;
    let wrapper = hard_verilog(Op::Add, "logos_fadd_p", &Block { semantics: Semantics::ENGINE, full: 3, least: 1 }, 4).unwrap();
    let library = device_library(&wrapper);
    assert!(library.contains("module fourteennm_fp_mac") && library.contains("(* blackbox, logos_box_area = \"0,0,2,0,0\" *)"), "{library}");
    assert!(library.contains("output [31:0] resulta") && library.contains("parameter accumulate_clock"), "the block's own ports and parameters");
    assert_eq!(device_library(&format!("{wrapper}{library}")), "", "a design that defines the block");
    assert_eq!(device_library("module m(input a, output y); assign y = a; endmodule\n"), "", "a design that uses none");
}

/// A flat image's fit (the FIM's own revision, `d5005`) is placed over the whole device, and the regions it records are
/// its floorplan's Logic Lock regions, not the PR region: recorded after the personas, passed or not, it changes neither
/// the capacity, nor the fullest persona, nor the fitter's time a persona is predicted to take.
#[test]
fn a_flat_fit_and_its_floorplans_regions_are_no_persona() {
    let flat = |passed: bool| {
        let mut r = persona("flat", 6_000.0, 90_000);
        r.build.flow.revision = Some("d5005".into());
        r.build.regions = (0..4)
            .map(|k| RegionUsage { name: format!("logos_ch{k}"), alms_needed: 60.0, alms: 6_160, placement: vec![((219, 346), (246, 373))], ..region(60.0) })
            .collect();
        r.verdict.passed = passed;
        r
    };
    for passed in [false, true] {
        let mut with = ledger();
        with.rows.push(flat(passed));
        let c = capacity(&with).expect("the personas' region");
        assert_eq!((c.row.as_str(), c.alms), ("large", 600_000), "passed {passed}: the PR region, not a channel's");
        assert_eq!(fullest(&with), fullest(&ledger()), "passed {passed}");
        assert_eq!(fitter_seconds(&with, 100_000.0), fitter_seconds(&ledger(), 100_000.0), "passed {passed}");
    }
    let mut renamed = ledger();
    renamed.rows[3].build.regions[0].name = "logos_ch0".into();
    assert_eq!(capacity(&renamed).unwrap().row, "small", "a persona's region is the PR slot's");
}

/// A flat fit is no persona even when a region of its floorplan is named for the PR slot's instance: the revision says
/// which compile placed a persona.
#[test]
fn a_flat_fit_with_a_region_named_for_the_pr_slot_is_no_persona() {
    let mut flat = persona("flat", 6_000.0, 90_000);
    flat.build.flow.revision = Some("d5005".into());
    flat.verdict.passed = true;
    let mut with = ledger();
    with.rows.push(flat);
    assert_eq!(capacity(&with).map(|c| c.row), Some("large".to_string()), "the personas' region, not the flat fit's");
    assert_eq!(fullest(&with), fullest(&ledger()));
}

/// A synthesis whose registers set its ALMs: `alms` ALMs over `aluts` ALUTs and `registers` registers.
fn registered(name: &str, alms: u64, aluts: u64, registers: u64) -> Row {
    let mut r = unit(name, alms, aluts);
    r.build.top[0].registers = registers;
    r
}

/// A synthesis holding more ALUTs than registers is still set by its registers when its registers' packing gives the more
/// ALMs: judged first by which it holds more of, every synthesis is judged again by the term the packings give it until
/// none changes side, so the packings are the logic units' and the register-bound syntheses' alone.
#[test]
fn a_synthesis_is_judged_by_the_term_its_packings_set_not_by_which_it_holds_more_of() {
    let mut l = ledger();
    l.rows.push(registered("probe-a", 1_400, 700, 2_000));
    // 1,000 ALUTs and 950 registers in 665 ALMs: 0.7 a register, past the 600 that 0.6 an ALUT gives.
    l.rows.push(registered("near", 665, 1_000, 950));
    let p = packing(&l).unwrap();
    assert!((p.per_alut - 0.6).abs() < 1e-9, "the logic units alone: {}", p.per_alut);
    assert!((p.per_register.unwrap() - 0.7).abs() < 1e-9, "the register-bound pair: {:?}", p.per_register);
}

/// The logic-bound syntheses give the ALMs an ALUT packs into, the register-bound ones the ALMs a register does, each
/// synthesis judged by the term that sets it; a design whose registers set its ALMs is held to them, refused past the
/// region though its ALUTs alone would fit.
#[test]
fn a_register_bound_design_is_held_to_its_registers_packing() {
    let mut l = ledger();
    l.rows.push(registered("probe-a", 1_400, 700, 2_000));
    l.rows.push(registered("probe-b", 14_000, 7_100, 20_000));
    let p = packing(&l).unwrap();
    assert!((p.per_alut - 0.6).abs() < 1e-9, "the logic units alone: {}", p.per_alut);
    let per_register = p.per_register.unwrap();
    assert!((per_register - 0.7).abs() < 1e-9, "the probes alone: {per_register}");
    assert_eq!(p.alms(1_000.0, 100.0), 600.0, "the ALUTs set it");
    assert_eq!(p.alms(1_000.0, 2_000.0), 1_400.0, "the registers set it");
    assert_eq!(packing(&ledger()).unwrap().per_register, None, "no register-bound synthesis, no register packing");
    let design = Design { area: Area { luts: 20_000, flops: 3_000_000, ..Area::default() }, depth: 10, loop_bound: LoopBound::Acyclic };
    let held = preflight(&design, &l, &calibration(), &TARGET);
    assert!(held.refusals.iter().any(|r| matches!(r, Refusal::Over { what: "ALMs", .. })), "{:?}", held.refusals);
    assert!(held.predicted.alms >= 0.7 * 3_000_000.0 - 1.0, "{}", held.predicted.alms);
}

/// A synthesis report with two partitions: their estimates, and a memory each.
fn two_partitions(alms: [u64; 2], dsp: [u64; 2], store: (u64, u64)) -> String {
    let usage = |partition: &str, alms: u64, dsp: u64| {
        format!(
            "+----+\n; Synthesis Resource Usage Summary for Partition \"{partition}\" ;\n+----+\n; Resource ; Usage ;\n+----+\n\
             ; Estimate of Logic utilization (ALMs needed) ; {alms} ;\n; Total block memory bits ; 0 ;\n; Total DSP Blocks ; {dsp} ;\n+----+\n\n\n"
        )
    };
    let rams = |partition: &str, row: String| {
        format!(
            "+----+\n; Synthesis RAM Summary for Partition \"{partition}\" ;\n+----+\n\
             ; Name ; Type ; Mode ; Port A Depth ; Port A Width ; Port B Depth ; Port B Width ; Size ; MIF ;\n+----+\n{row}\n+----+\n\n\n"
        )
    };
    let (depth, width) = store;
    usage("root_partition", alms[0], dsp[0])
        + &usage("auto_fab_0", alms[1], dsp[1])
        + &rams("root_partition", format!("; top|store|ram ; M20K block ; Simple Dual Port ; {depth} ; {width} ; {depth} ; {width} ; 0 ; None ;"))
        + &rams("auto_fab_0", "; fab|fifo|ram ; M20K block ; Simple Dual Port ; 512 ; 40 ; 512 ; 40 ; 20480 ; None ;".to_string())
}

/// A flat image is held to the device by what its synthesis says it needs: the ALMs and DSP blocks every partition's
/// estimate sums to, and the M20K blocks the fitter gives its memories. A store of 57,344 words of 512 bits holds 29.4
/// Mbit, 1,434 blocks' worth of bits, but stacks to 65,536 words: 1,664 blocks.
#[test]
fn a_flat_image_needs_the_m20k_blocks_the_fitter_gives_its_memories() {
    let syn = two_partitions([800_000, 40_000], [5_000, 10], (57_344, 512));
    assert_eq!(synthesized_holds(&syn), Some(Holds { alms: 840_000, dsp: 5_010, m20k: 1_664 + 1 }));
    assert_eq!(over(synthesized_holds(&syn).unwrap(), DEVICE), Vec::new(), "the device holds it");
    assert_eq!(synthesized_holds("no report"), None, "no estimate to hold");
}

#[test]
fn each_resource_past_the_devices_is_refused_by_name() {
    let syn = two_partitions([900_000, 40_000], [5_760, 1], (65_536 * 7, 512));
    let needs = synthesized_holds(&syn).unwrap();
    assert_eq!(needs.m20k, 13 * 1_024 + 1, "458,752 words: a stack of 524,288, 13 blocks of 40 bits wide and 1,024 deep");
    let refused = over(needs, DEVICE);
    assert_eq!(
        refused,
        vec![
            Refusal::Device { what: "ALMs", needs: 940_000, holds: 933_120 },
            Refusal::Device { what: "DSP blocks", needs: 5_761, holds: 5_760 },
            Refusal::Device { what: "M20K blocks", needs: 13 * 1_024 + 1, holds: 11_721 },
        ]
    );
    assert_eq!(refused[2].to_string(), "13313 M20K blocks, where the device holds 11721");
    assert_eq!(over(DEVICE, DEVICE), Vec::new(), "exactly what the device holds");
}

/// The M20K blocks a flat image needs are the fitter's, not its memories' shapes alone: an `AUTO` memory small enough for
/// MLABs takes none, a byte-enabled one 32 bits a block.
#[test]
fn a_flat_images_m20k_blocks_are_the_fitters_not_its_shapes_alone() {
    let syn = "\
+----+
; Synthesis Resource Usage Summary for Partition \"root_partition\" ;
+----+
; Resource ; Usage ;
+----+
; Estimate of Logic utilization (ALMs needed) ; 1000 ;
; Total DSP Blocks ; 0 ;
+----+


+----+
; Synthesis RAM Summary for Partition \"root_partition\" ;
+----+
; Name ; Type ; Mode ; Port A Depth ; Port A Width ; Port B Depth ; Port B Width ; Size ; MIF ;
+----+
; top|small|mem_rtl_0|auto_generated|ALTERA_SYNCRAM ; AUTO ; Simple Dual Port ; 16 ; 32 ; 16 ; 32 ; 512 ; None ;
; top|store|mem_rtl_0|auto_generated|ALTERA_SYNCRAM ; AUTO ; Simple Dual Port ; 512 ; 512 ; 512 ; 512 ; 262144 ; None ;
+----+


+----+
; Parameter Settings for Inferred Entity Instance: top|store|mem_rtl_0 ;
+----+
; Parameter Name ; Value ; Type ;
+----+
; BYTE_SIZE ; 8 ; Untyped ;
; READ_DURING_WRITE_MODE_MIXED_PORTS ; OLD_DATA ; Untyped ;
; WIDTH_BYTEENA_A ; 64 ; Signed Integer ;
+----+
";
    assert_eq!(synthesized_holds(syn).map(|h| h.m20k), Some(16), "the small memory in MLABs, the store 16 blocks of 32 bits");
}
