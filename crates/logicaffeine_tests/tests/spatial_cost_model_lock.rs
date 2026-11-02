//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! COST-MODEL LOCK — what survives after the plan decision moved to a model that can be FALSIFIED.
//!
//! ★ THIS FILE USED TO ASSERT THAT A FITTED CYCLE MODEL DECIDES THE PLAN. It no longer does, and the
//! change is approved (2026-09-11). The reason is a
//! measurement, not a preference: `CycleModel`'s eighteen constants were fitted to the `cycles` column
//! of the ttsim ledger, and `ttsim_instrument_audit.json` shows that column is not a function of the
//! program. Eight times the reduction depth moves it 0.08%; the device's own per-core cycle register
//! moves 0.3%. Regressed against every plan quantity the model carries, 27 of 58 calibration points
//! stay more than 15% out with seven regressors — so `every_fully_specified_point_is_predicted_within_
//! fifteen_percent` was not failing because the fit was bad, it was failing because no function of the
//! plan predicts that column at all.
//!
//! The cost was not theoretical. At 512^3 the fitted model ranked FIRST a plan that moves 8,912,896 NOC
//! bytes and issues 4,480 transactions over one that moves 2,621,952 and issues 1,040 — a 3.40x and
//! 4.31x loss, bought with a lower score on a counter that measures neither.
//!
//! RETIRED HERE, MOVED TO `spatial_work_model_lock` (each superseded by a PREDICTION that the machine
//! checks, rather than a fit that cannot be wrong):
//!   * `fit_reproduces_the_pinned_wormhole_constants_within_one_percent` — there is no fit to reproduce;
//!     the work model has no free parameters. `the_closed_form_agrees_with_the_choreography` replaces it.
//!   * `every_fully_specified_point_is_predicted_within_fifteen_percent` — replaced by
//!     `the_model_predicts_the_work_the_machine_actually_did`, which holds to 5% against the WORK meter.
//!   * `leave_one_out_keeps_the_decisions` — a jackknife over a fit that no longer exists.
//!   * `the_synchronization_terms_are_load_bearing` — the synchronization constants decided nothing even
//!     when they were load-bearing on paper; the work model prices a multicast handshake by COUNTING it.
//!   * `the_model_reproduces_the_measured_decisions` — replaced by
//!     `spatial_work_model_lock::the_pipeline_never_chooses_a_plan_the_ledger_measured_to_do_more_work`,
//!     which is the same obligation against a meter that measures the program.
//!
//! WHAT STAYS, because it was always about the compiler rather than about the fit:
//!   A. the ledger holds enough fully-specified Wormhole rows, each with its provenance and reproduce
//!      command, for any of this to be checkable at all;
//!   E. an uncalibrated target locks STRUCTURE only — the cycle model still sizes buffers, where a rate
//!      model belongs, and its monotonicity in work is still an obligation;
//!   F. NO STRATEGY INPUT EXISTS: the pass table is typed `fn(&Program, &Target)` and the spatial module
//!      never mentions the retired names. This is the one that matters most and it is untouched — the
//!      objective is not a strategy enum, it names an axis to minimize and names no plan.
//!   G. every spatial row in the ledger is the plan the pipeline emits TODAY.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(spatial_cost_model_lock)'`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::cost::{quantities_for_point, CalibrationPoint, CycleModel};
use logicaffeine_compile::spatial::ir::NumericsPolicy;
use logicaffeine_compile::spatial::pipeline::{
    calibration_candidates_matching, CandidateKind, PlanFilter, PASS_ORDER,
};
use logicaffeine_compile::spatial::target::SpatialTarget;
use spatial_support::*;

fn points() -> Vec<CalibrationPoint> {
    ledger_points()
}

/// The Wormhole rows whose printed plan the compiler can still QUANTIFY — our emitted programs and the
/// tt-metal streaming example, whose choreography is ours. It used to be called the fit set; nothing is
/// fitted to it any more, but a row the compiler cannot describe is a row no model can be checked
/// against, so how many there are is still worth asserting.
fn quantifiable() -> Vec<logicaffeine_compile::spatial::cost::PlanQuantities> {
    let wh = SpatialTarget::wormhole_ttsim();
    points()
        .iter()
        .filter(|p| p.chip == "wormhole_b0" && p.stack != "ttnn_production")
        .filter_map(|p| quantities_for_point(p, &wh))
        .collect()
}

#[test]
fn calibration_points_are_recorded_with_provenance() {
    let pts = points();
    assert!(
        pts.len() >= 8,
        "at least eight ledger rows (the measurement campaign has run): {}",
        pts.len()
    );
    for p in &pts {
        assert_eq!(p.provenance, "sim_functional_not_timed", "{p:?}");
        assert!(p.cycles > 0);
        assert!(
            p.reproduce.starts_with("scripts/tt-matmul-measure.sh"),
            "every point carries its reproduce command: {p:?}"
        );
        assert!(p.pcc >= p.pcc_floor, "{p:?}");
    }
    let fully = quantifiable().len();
    assert!(
        fully >= 6,
        "at least six fully-specified points the compiler can describe: {fully}"
    );
    let shapes: std::collections::BTreeSet<_> = pts.iter().map(|p| p.shape).collect();
    assert!(
        shapes.contains(&(640, 640, 640))
            && shapes.contains(&(2048, 1024, 512))
            && shapes.contains(&(2048, 2048, 1024))
    );
    let spatial_rows = pts.iter().filter(|p| p.program == "logos_spatial").count();
    assert!(
        spatial_rows >= 4,
        "the IR-lowered program has been measured at several shapes: {spatial_rows}"
    );
}

#[test]
fn uncalibrated_targets_lock_structure_only() {
    for t in [
        SpatialTarget::blackhole(),
        SpatialTarget::sky_fabric(64, 64),
        SpatialTarget::sn40l_rdu(),
        SpatialTarget::rtx3060(),
    ] {
        assert!(
            !t.cycle.calibrated,
            "{}: no ttsim-class measurement exists for this target",
            t.name
        );
    }
    let m = CycleModel::uncalibrated();
    let wh = SpatialTarget::wormhole_ttsim();
    let base = quantities_for_point(
        &points()
            .into_iter()
            .find(|p| p.shape == (2048, 1024, 512))
            .unwrap(),
        &wh,
    )
    .unwrap();
    let mut more = base;
    more.tile_macs *= 2;
    assert!(
        m.cycles(&more) >= m.cycles(&base),
        "more work never predicts fewer cycles"
    );
    let mut more_dram = base;
    more_dram.dram_tiles_site *= 2;
    assert!(m.cycles(&more_dram) >= m.cycles(&base));
}

#[test]
fn no_strategy_enum_input_exists() {
    // Type-level: every pass is a pure function of (program, target).
    let names: Vec<&str> = PASS_ORDER.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        names,
        [
            // `fuse_generic` runs FIRST, on the template: legalization splits a workload into stages so
            // the split can be checked against its specification, and this puts back together whatever
            // the destination registers can carry — before anything is tiled, placed or scheduled.
            "fuse_generic",
            "tile",
            "place",
            "fuse_readers",
            "size_buffers",
            "synthesize_handshakes",
            "persist",
            "verify",
        ]
    );
    let _typed: &[(
        &str,
        fn(
            &logicaffeine_compile::spatial::ir::SpatialProgram,
            &SpatialTarget,
        ) -> Result<
            logicaffeine_compile::spatial::ir::SpatialProgram,
            logicaffeine_compile::spatial::passes::SpatialReject,
        >,
    )] = PASS_ORDER;
    // Text-level: the spatial module never mentions the retired bespoke names.
    let dir = repo_root().join("crates/logicaffeine_compile/src/spatial");
    let mut files = Vec::new();
    fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    walk(&dir, &mut files);
    assert!(
        files.len() >= 12,
        "the spatial module is real: {} files",
        files.len()
    );
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for tok in [
            "MatmulStrategy",
            "choose_strategy",
            "tune_tiling",
            "ChipKnobs",
            "dram_read_tiles",
            "ReuseMcast",
            "BlockReuse",
        ] {
            assert!(
                !text.contains(tok),
                "{}: mentions retired `{tok}`",
                f.display()
            );
        }
    }
}

#[test]
fn every_spatial_row_in_the_ledger_is_the_plan_the_pipeline_emits_today() {
    // A ledger row of the IR-lowered program records the plan it printed; the compiler must still produce
    // exactly that plan for the same (shape, knobs) — a silent drift between the recorded number and the
    // compiler's choice is impossible.
    //
    // Restricted to the rows the ledger's own `frontier[].compiler_chosen` names. The other rows in each
    // group are the ALTERNATIVES the sweep measured under a forced `--candidate`, and they exist so the
    // cost model's ranking can be checked against the simulator's; asserting that the pipeline emits each
    // of them is unsatisfiable whatever the compiler does. `the_model_reproduces_the_measured_decisions`
    // is what holds the alternatives to account.
    // ★ EVERY CHIP THE LEDGER HOLDS ROWS FOR, not the one it held rows for when this was written.
    //
    // This filtered to `wormhole_b0`, which was every row in the ledger at the time. It stopped being
    // every row the day Blackhole was measured, and nothing said so — the chip's rows were simply held
    // to nothing while the lock stayed green. The simulated twin differs per part: Wormhole's preset is
    // the production n150 (8×8) and the simulator exposes 8×9, so it needs `wormhole_ttsim()`;
    // Blackhole's preset already IS the grid the simulator reports.
    let target_for = |chip: &str| match chip {
        "wormhole_b0" => Some(SpatialTarget::wormhole_ttsim()),
        "blackhole" => Some(SpatialTarget::blackhole()),
        _ => None,
    };
    let chosen = compiler_chosen_rows();
    assert!(
        chosen.len() >= 4,
        "the ledger must name the compiler's choice per shape: {} named",
        chosen.len()
    );
    let mut checked = 0;
    for p in points().iter().filter(|p| {
        target_for(&p.chip).is_some() && p.program == "logos_spatial" && chosen.contains(&p.id)
    }) {
        let target = target_for(&p.chip).expect("filtered above");
        let numerics = if p.exact {
            NumericsPolicy::Exact
        } else {
            NumericsPolicy::AllowSpillRounding
        };
        // The packer's in-place accumulate is a PLAN knob the harness pins with `--l1acc`: a row records the plan
        // the pipeline emits under the knob it was measured at, so it is compared with the best plan of that
        // arrangement, exactly as the harness asks for it.
        let pinned = PlanFilter {
            pack_accumulate: Some(p.l1acc),
            ..PlanFilter::default()
        };
        let prog = calibration_candidates_matching(
            &gemm_program(p.shape.0, p.shape.1, p.shape.2),
            &target,
            numerics,
            pinned,
        )
        .unwrap()
        .into_iter()
        .find(|(kind, _)| *kind == CandidateKind::Best)
        .map(|(_, prog)| prog)
        .unwrap_or_else(|| panic!("{p:?}: the pipeline has a plan for the row's knobs"));
        let s = prog.summary();
        assert_eq!(s.streaming, p.plan.streaming, "{p:?}: streaming");
        assert_eq!(s.multicast_groups > 0, p.plan.fused, "{p:?}: fused");
        if !p.plan.streaming {
            assert_eq!(s.grid, p.plan.grid, "{p:?}: grid");
            assert_eq!(s.per_site_block, p.plan.per_site, "{p:?}: per-site block");
            if let Some(b) = p.plan.block {
                assert_eq!(s.block.unwrap(), b, "{p:?}: block");
            }
        }
        checked += 1;
    }
    assert!(
        checked >= 4,
        "the IR-lowered program has ledger rows at several shapes: {checked}"
    );
    // And the sweep must actually span the chips the ledger holds rows for, or a per-chip filter has
    // quietly narrowed back to one.
    let pts = points();
    let chips: std::collections::BTreeSet<&str> = pts
        .iter()
        .filter(|p| {
            target_for(&p.chip).is_some()
                && p.program == "logos_spatial"
                && !p.l1acc
                && chosen.contains(&p.id)
        })
        .map(|p| p.chip.as_str())
        .collect();
    assert!(
        chips.len() >= 2,
        "the ledger holds compiler-chosen spatial rows on more than one chip: {chips:?}"
    );
}
