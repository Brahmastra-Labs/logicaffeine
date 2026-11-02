//! ★ ANALOG PVT MONTE-CARLO LOCK ★ — with the SOURCED non-ideality physics ON, a fixed-seed
//! battery of manufactured dies NEVER leaves the compiler's own physical ε.
//!
//! Every die is a deterministic draw (fixed BASE_SEED, SplitMix64 + clamped Box–Muller): per-leg
//! gain errors at the programming-accuracy + placed-distance sigma, per-tile offsets, a frozen
//! noise realization. The budget carries each of those at the k = 6σ envelope (`KSIGMA`), so
//! the fixed seed set is deterministically green forever — a red here means the model, the
//! budget composition, or the instance evaluator drifted, never the dice. The margin guard
//! (worst ≤ 0.8·ε) gives early warning when a model change erodes headroom.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

use logicaffeine_compile::codegen_ana::{
    analog_netlist_from_source, assumed_ranges, error_budget_nonideal, eval_analog_dc,
    eval_analog_instance, place_analog, sample_instance, scale_pass, sky130_nonideal, AnalogTarget,
    ErrorSource,
};

const AFFINE: &str = "## To f (x: Float) -> Float:\n    Assume x is between -1.0 and 1.0.\n    Return x * 0.5 + 0.25.\n\n## Main\n    Show f(0.5).\n";
const DOT3: &str = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";
const VARMUL: &str = "## To vm2 (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Return a * b + 0.25.\n\n## Main\n    Show vm2(0.5, 0.5).\n";

const CORPUS: &[(&str, &str)] = &[("affine", AFFINE), ("dot3", DOT3), ("varmul", VARMUL)];
const FN_NAMES: &[&str] = &["f", "dot3", "vm2"];

const BASE_SEED: u64 = 0xA11A_00C0_FFEE_5EED;
const N_INSTANCES: u64 = 256;

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
fn all_manufactured_dies_stay_inside_the_physical_epsilon() {
    let target = AnalogTarget::sky130_fpaa();
    let model = sky130_nonideal();
    for (p, (label, src)) in CORPUS.iter().enumerate() {
        let netlist = analog_netlist_from_source(src, FN_NAMES[p], &target).expect("lowers");
        let ranges = assumed_ranges(src);
        let (schedule, cert) = scale_pass(&netlist, &ranges, &target).expect("schedules");
        let placement = place_analog(&schedule.quantized, &target).expect("places");
        let budget = error_budget_nonideal(&netlist, &cert, &schedule, &target, &model, &placement);
        assert!(
            budget.recheck(),
            "`{label}`: the physical budget must replay from its terms"
        );
        assert!(
            budget
                .terms
                .iter()
                .any(|t| matches!(t.source, ErrorSource::TileGain))
                && budget
                    .terms
                    .iter()
                    .any(|t| matches!(t.source, ErrorSource::TileOffset))
                && budget
                    .terms
                    .iter()
                    .any(|t| matches!(t.source, ErrorSource::Noise)),
            "`{label}`: the physical terms must be itemized: {:?}",
            budget.terms
        );

        let mut worst = 0.0_f64;
        for die in 0..N_INSTANCES {
            let inst = sample_instance(
                &schedule.quantized.cells,
                &placement,
                &model,
                BASE_SEED ^ (die << 16) ^ (p as u64),
            );
            let mut seed = BASE_SEED ^ die ^ ((p as u64) << 48);
            for _ in 0..4 {
                let xs: Vec<f64> = ranges
                    .iter()
                    .map(|r| point_in(&mut seed, r.lo, r.hi))
                    .collect();
                let ideal = eval_analog_dc(&netlist, &xs);
                let physical = eval_analog_instance(&schedule, &target, &inst, &xs);
                let dev = (physical - ideal).abs();
                worst = worst.max(dev);
                assert!(
                    dev <= budget.eps_math,
                    "`{label}` die {die}: |physical − ideal| = {dev:e} exceeds the physical ε = {:e} at {xs:?}",
                    budget.eps_math
                );
            }
        }
        assert!(
            worst > 0.0,
            "`{label}`: the battery observed no deviation — vacuous"
        );
        assert!(
            worst <= 0.8 * budget.eps_math,
            "`{label}`: margin guard — worst {worst:e} within 0.8·ε {:e} (headroom eroding?)",
            budget.eps_math
        );
    }
}
