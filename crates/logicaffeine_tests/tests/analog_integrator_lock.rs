//! ★ ANALOG INTEGRATOR LOCK ★ — counted Euler loops run as ZOH OTA-C integration: the state
//! vector lives in sample-and-holds, each frame integrates the held values for the settling
//! window, one ADC reads the result after the declared frames. Because the hold makes every
//! frame exact arithmetic, the certified semantics is the VM's Euler trajectory — bit-exact on
//! the ideal tier — with physics entering only through the budgeted layers.
//!
//! What this file locks:
//!   1. Lowering shapes — decay/driven-leaky/2-state-oscillator all extract; the oscillator's
//!      cross-coupled pair is a genuine 2-state tick with sequential-update semantics faithful
//!      to the VM (the second state sees the FIRST's new value).
//!   2. Ideal trajectories are BIT-EXACT with the VM.
//!   3. The frame-iterated certificate passes the independent multi-state checker.
//!   4. The frame-iterated budget is sound over deterministic parameter batteries.
//!   5. Physical gates refuse typed: an undeclared parameter, a runtime trip count (the frame
//!      schedule is static), and an OTA rate outside `integrator_k_range` (the gm/C identity).
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

use logicaffeine_compile::codegen_ana::{
    analog_integrator_from_source, eval_analog_integrated, eval_analog_integrated_quantized,
    integrator_error_budget, integrator_scale_pass, to_range_claim, ASrc, AnaLowerError,
    AnaUnplaceable, AnalogTarget, InitSrc,
};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_proof::interval_replay::{check_integrator_claim, RSrc};

const DECAY: &str = "## To decay (x0: Float) -> Float:\n    Assume x0 is between 0.5 and 1.0.\n    Let mutable x be x0.\n    Repeat for i from 1 to 50:\n        Set x to x + x * -0.125.\n    Return x.\n\n## Main\n    Show decay(0.75).\n";

const LEAKY: &str = "## To leaky (x0: Float) -> Float:\n    Assume x0 is between -0.5 and 0.5.\n    Let mutable x be x0.\n    Repeat for i from 1 to 50:\n        Set x to x + (0.25 - x) * 0.125.\n    Return x.\n\n## Main\n    Show leaky(0.125).\n";

const OSC: &str = "## To osc (p0: Float, v0: Float) -> Float:\n    Assume p0 is between -0.5 and 0.5.\n    Assume v0 is between -0.5 and 0.5.\n    Let mutable p be p0.\n    Let mutable v be v0.\n    Repeat for i from 1 to 50:\n        Set p to p + v * 0.125.\n        Set v to v + p * -0.125.\n    Return p.\n\n## Main\n    Show osc(0.25, 0.0).\n";

const CORPUS: &[(&str, &str, &str, &[f64])] = &[
    ("decay", DECAY, "decay", &[0.75]),
    ("leaky", LEAKY, "leaky", &[0.125]),
    ("osc", OSC, "osc", &[0.25, 0.0]),
];

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

fn vm_float(src: &str) -> f64 {
    let out = vm_outcome(src);
    assert_eq!(out.error, None, "corpus program must run on the VM:\n{src}");
    out.output
        .trim()
        .parse::<f64>()
        .unwrap_or_else(|_| panic!("VM output is not a float: {:?}", out.output))
}

#[test]
fn lowering_shapes_are_pinned() {
    let target = AnalogTarget::ideal();
    let decay = analog_integrator_from_source(DECAY, "decay", &target).expect("decay lowers");
    assert_eq!(decay.tick.n_in, 1);
    assert_eq!(decay.frames, 50);
    assert_eq!(decay.init, vec![InitSrc::Param(0)]);
    assert_eq!(decay.control.frames, 50);

    let osc = analog_integrator_from_source(OSC, "osc", &target).expect("osc lowers");
    assert_eq!(osc.tick.n_in, 2, "two held states");
    assert_eq!(osc.outs.len(), 2);
    assert_eq!(osc.init, vec![InitSrc::Param(0), InitSrc::Param(1)]);
}

#[test]
fn ideal_trajectories_are_bit_exact_with_the_vm() {
    let target = AnalogTarget::ideal();
    for (label, src, fn_name, params) in CORPUS {
        let r = analog_integrator_from_source(src, fn_name, &target).expect("lowers");
        let got = eval_analog_integrated(&r, params);
        let want = vm_float(src);
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "`{label}`: the ZOH ideal trajectory mirrors the VM's Euler loop — bit-exact"
        );
    }
}

#[test]
fn integrator_certificates_pass_the_independent_checker() {
    let target = AnalogTarget::ideal();
    for (label, src, fn_name, _) in CORPUS {
        let r = analog_integrator_from_source(src, fn_name, &target).expect("lowers");
        let (schedule, cert) = integrator_scale_pass(&r, &target).expect("schedules");
        let (cells, claim) = to_range_claim(&schedule, &cert);
        let outs: Vec<RSrc> = r
            .outs
            .iter()
            .map(|o| match o {
                ASrc::In(k) => RSrc::In(*k),
                ASrc::Val(v) => RSrc::Val(*v),
                ASrc::Const(c) => RSrc::Const(*c),
            })
            .collect();
        let (init_lo, init_hi): (Vec<f64>, Vec<f64>) = r
            .init
            .iter()
            .map(|i| match i {
                InitSrc::Param(k) => (r.param_ranges[*k].lo, r.param_ranges[*k].hi),
                InitSrc::Const(c) => (*c, *c),
            })
            .unzip();
        assert!(
            check_integrator_claim(&cells, &outs, &init_lo, &init_hi, r.frames, &claim),
            "`{label}`: the independent multi-state checker must accept the claim"
        );
        // Tamper: shrink the claimed trajectory hull — the checker must catch the escape.
        let mut lying = claim.clone();
        for h in lying.input_hi.iter_mut() {
            *h = init_hi.iter().cloned().fold(f64::NEG_INFINITY, f64::max) * 1.0e-3;
        }
        assert!(
            !check_integrator_claim(&cells, &outs, &init_lo, &init_hi, r.frames, &lying),
            "`{label}`: a hull that hides the trajectory must be rejected"
        );
    }
}

#[test]
fn quantized_trajectories_never_exceed_the_compilers_own_epsilon() {
    let target = AnalogTarget::ideal();
    for (i, (label, src, fn_name, _)) in CORPUS.iter().enumerate() {
        let r = analog_integrator_from_source(src, fn_name, &target).expect("lowers");
        let (schedule, cert) = integrator_scale_pass(&r, &target).expect("schedules");
        let budget = integrator_error_budget(&r, &cert, &schedule, &target);
        assert!(
            budget.recheck(),
            "`{label}`: budget must replay from its terms"
        );

        let mut seed = BASE_SEED ^ ((i as u64) << 8);
        let mut worst = 0.0_f64;
        for _ in 0..64 {
            let params: Vec<f64> = r
                .param_ranges
                .iter()
                .map(|p| point_in(&mut seed, p.lo, p.hi))
                .collect();
            let ideal = eval_analog_integrated(&r, &params);
            let physical = eval_analog_integrated_quantized(&r, &schedule, &target, &params);
            let dev = (physical - ideal).abs();
            worst = worst.max(dev);
            assert!(
                dev <= budget.eps_math,
                "`{label}`: |physical − ideal| = {dev:e} exceeds ε = {:e} at {params:?}",
                budget.eps_math
            );
        }
        assert!(
            worst > 0.0,
            "`{label}`: battery observed no deviation — vacuous"
        );
    }
}

#[test]
fn physical_gates_refuse_typed() {
    let target = AnalogTarget::ideal();

    let no_range = DECAY.replace("    Assume x0 is between 0.5 and 1.0.\n", "");
    match analog_integrator_from_source(&no_range, "decay", &target) {
        Err(AnaLowerError::UnboundedInput(msg)) => assert!(msg.contains("x0"), "{msg}"),
        other => panic!("undeclared parameter must refuse, got {other:?}"),
    }

    let dynamic = "## To decay (x0: Float, n: Float) -> Float:\n    Assume x0 is between 0.5 and 1.0.\n    Assume n is between 1.0 and 10.0.\n    Let mutable x be x0.\n    Repeat for i from 1 to n:\n        Set x to x + x * -0.125.\n    Return x.\n\n## Main\n    Show decay(0.75, 5.0).\n";
    match analog_integrator_from_source(dynamic, "decay", &target) {
        Err(AnaLowerError::UnloweredOp(msg)) => {
            assert!(msg.contains("compile-time constant"), "{msg}")
        }
        other => panic!("a runtime trip count must refuse (static frame schedule), got {other:?}"),
    }

    // The OTA rate identity: a target whose gm/C window excludes |k|/T refuses the schedule.
    let mut slow = AnalogTarget::ideal();
    slow.integrator_k_range = (
        logicaffeine_base::si::Hertz(1.0e12),
        logicaffeine_base::si::Hertz(1.0e13),
    ); // nothing settling in 100 ns realizes 0.125/T here
    match analog_integrator_from_source(DECAY, "decay", &slow) {
        Err(AnaLowerError::Unplaceable(AnaUnplaceable::IntegratorRateUnrealizable { .. })) => {}
        other => panic!("an unrealizable gm/C must refuse typed, got {other:?}"),
    }
}
