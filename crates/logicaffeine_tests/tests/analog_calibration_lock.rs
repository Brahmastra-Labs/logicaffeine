//! ★ ANALOG CALIBRATION LOCK ★ — calibration is NON-VACUOUS and its residual budget is sound.
//!
//! Production test measures each die and burns trims on the trim grid; the calibrated budget
//! then carries HALF A TRIM STEP for gain and offset (deterministic residual bound) instead of
//! the 6σ manufacturing envelope — the product-grade ε. This file proves the whole story on a
//! deep chain with the scatter placement (worst honest case):
//!   1. The calibrated budget is strictly tighter than the uncalibrated one.
//!   2. Every calibrated die stays inside the CALIBRATED budget (128 fixed-seed dies).
//!   3. NON-VACUITY: at least one uncalibrated die EXCEEDS the calibrated budget — calibration
//!      is load-bearing, not bookkeeping.
//!   4. Every trim value sits exactly on the trim grid (what the trim DAC can actually burn).
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

use logicaffeine_compile::codegen_ana::{
    analog_netlist_from_source, apply_calibration, assumed_ranges, calibrate,
    error_budget_calibrated, error_budget_nonideal, eval_analog_dc, eval_analog_instance,
    place_blind, sample_instance, scale_pass, sky130_nonideal, v1_trim, AnalogTarget,
};

const CHAIN: &str = "## To chain (x: Float) -> Float:\n    Assume x is between -1.0 and 1.0.\n    Return ((((((x * 0.5 + 0.25) * 0.5 + 0.125) * 0.5 + 0.0625) * 0.5 + 0.03125) * 0.5 + 0.015625) * 0.5 + 0.0078125) * 0.5.\n\n## Main\n    Show chain(0.5).\n";

const BASE_SEED: u64 = 0xA11A_00C0_FFEE_5EED;

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn point_in(state: &mut u64, lo: f64, hi: f64) -> f64 {
    let u = (splitmix(state) >> 11) as f64 / (1u64 << 53) as f64;
    lo + u * (hi - lo)
}

#[test]
fn calibration_is_load_bearing_and_its_residual_budget_is_sound() {
    let target = AnalogTarget::sky130_fpaa();
    let model = sky130_nonideal();
    let spec = v1_trim(&target);
    let netlist = analog_netlist_from_source(CHAIN, "chain", &target).expect("lowers");
    let ranges = assumed_ranges(CHAIN);
    let (schedule, cert) = scale_pass(&netlist, &ranges, &target).expect("schedules");
    let q = &schedule.quantized;
    let placement = place_blind(q, &target).expect("places");

    let uncal = error_budget_nonideal(&netlist, &cert, &schedule, &target, &model, &placement);
    let cal = error_budget_calibrated(
        &netlist, &cert, &schedule, &target, &model, &placement, &spec,
    );
    assert!(cal.recheck() && uncal.recheck());
    // 1. Calibration tightens ε.
    assert!(
        cal.eps_math < uncal.eps_math,
        "the calibrated ε {:e} must be tighter than the manufacturing ε {:e}",
        cal.eps_math,
        uncal.eps_math
    );

    let mut any_uncal_exceeds_cal_budget = false;
    for die in 0..128u64 {
        let inst = sample_instance(&q.cells, &placement, &model, BASE_SEED ^ (die << 16));
        let table = calibrate(&inst, &spec);
        // 4. Every trim on the grid.
        for trims in &table.gain_trim {
            for t in trims {
                assert!(
                    (t / spec.gain_step).fract() == 0.0,
                    "gain trim {t} must sit on the {} grid",
                    spec.gain_step
                );
            }
        }
        for t in &table.offset_trim_v {
            assert!(
                (t / spec.offset_step_v).fract() == 0.0,
                "offset trim {t} must sit on the grid"
            );
        }
        let cal_inst = apply_calibration(&inst, &table);

        let mut seed = BASE_SEED ^ die;
        for _ in 0..2 {
            let xs: Vec<f64> = ranges
                .iter()
                .map(|r| point_in(&mut seed, r.lo, r.hi))
                .collect();
            let ideal = eval_analog_dc(&netlist, &xs);
            // 2. Calibrated dies inside the calibrated budget.
            let dev_cal = (eval_analog_instance(&schedule, &target, &cal_inst, &xs) - ideal).abs();
            assert!(
                dev_cal <= cal.eps_math,
                "die {die}: calibrated deviation {dev_cal:e} exceeds the calibrated ε {:e}",
                cal.eps_math
            );
            // 3. Watch for an uncalibrated die the calibrated budget cannot hold.
            let dev_raw = (eval_analog_instance(&schedule, &target, &inst, &xs) - ideal).abs();
            if dev_raw > cal.eps_math {
                any_uncal_exceeds_cal_budget = true;
            }
        }
    }
    assert!(
        any_uncal_exceeds_cal_budget,
        "NON-VACUITY: no uncalibrated die exceeded the calibrated ε — calibration would be bookkeeping"
    );
}
