//! ★ ANALOG INTEGER LOCK ★ — the certified-EXACT integer lane: small-range integer dataflow on
//! the analog plane with the ADC as the restorer, and the compiler PROVING exactness.
//!
//! Integers live on a grid and the ADC rounds; if every input is a declared-range integer, the
//! region is GRID-PRESERVING (add/sub/integer-weight multiply — integer intermediates stay
//! integers), and the compiler's own error budget satisfies **ε < ½ grid step**, then the
//! rounded readout is EXACTLY the VM's integer. Not approximately — exactly, on the noisy
//! substrate, with the budget as the proof. Where ε ≥ ½ the lane refuses TYPED
//! (`ExactnessUnprovable`, carrying the itemized budget), never emitting a maybe-integer.
//!
//! What this file locks:
//!   1. Int DAG lowering (constants decoded as VALUES, not float bit patterns) + the
//!      grid-preservation guarantee; the ½-grid gate passes on the ideal target at ±100 and
//!      var×var at ±8, and every quantized evaluation restores the EXACT VM integer (==, no
//!      approx anywhere in this file).
//!   2. The gate is honest physics: sky130's 8-bit converters cannot prove ±100 (refused
//!      `ExactnessUnprovable`), CAN prove ±8 on converters alone — and with the full
//!      non-ideality model the 6σ envelope breaks exactness while CALIBRATION (half-trim-step
//!      residuals) restores it: 128 calibrated dies all read back exact integers. Calibration
//!      is what makes integer analog a product.
//!   3. The streamed integer MAC (`Seq of Int`) restores exact integers under the same gate.
//!   4. Refusals: integer division (no exact analog divider), bitwise (no voltage semantics),
//!      undeclared ranges.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

use logicaffeine_compile::codegen_ana::{
    analog_int_netlist_from_source, analog_int_reduction_from_source, apply_calibration,
    assumed_ranges, calibrate, error_budget, error_budget_calibrated, error_budget_nonideal,
    eval_analog_instance, eval_analog_quantized, eval_analog_sampled_quantized, exactness_gate,
    place_analog, reduction_error_budget, reduction_scale_pass, sample_instance, scale_pass,
    sky130_nonideal, v1_trim, AnaLowerError, AnalogCell, AnalogTarget,
};
use logicaffeine_compile::compile::vm_outcome;

const AFFINE_I: &str = "## To f (n: Int) -> Int:\n    Assume n is between -100 and 100.\n    Return n * 2 + 7.\n\n## Main\n    Show f(37).\n";

const DOT_I: &str = "## To dot (a: Int, b: Int, c: Int) -> Int:\n    Assume a is between -8 and 8.\n    Assume b is between -8 and 8.\n    Assume c is between -8 and 8.\n    Return a * 3 + b * 2 + c.\n\n## Main\n    Show dot(5, -3, 7).\n";

const VARMUL_I: &str = "## To vm2 (a: Int, b: Int) -> Int:\n    Assume a is between -8 and 8.\n    Assume b is between -8 and 8.\n    Return a * b + 3.\n\n## Main\n    Show vm2(5, -6).\n";

const ADD2_I: &str = "## To add2 (a: Int, b: Int) -> Int:\n    Assume a is between -8 and 8.\n    Assume b is between -8 and 8.\n    Return a + b.\n\n## Main\n    Show add2(5, -3).\n";

const SUM_SEQ_I: &str = "## To total (xs: Seq of Int) -> Int:\n    Assume length of xs is between 1 and 8.\n    Let mutable acc be 0.\n    Repeat for each x in xs:\n        Assume x is between -8 and 8.\n        Set acc to acc + x.\n    Return acc.\n\n## Main\n    Let mutable xs be a new Seq of Int.\n    Push 5 to xs.\n    Push -3 to xs.\n    Push 7 to xs.\n    Show total(xs).\n";

const BASE_SEED: u64 = 0xA11A_00C0_FFEE_5EED;

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn int_in(state: &mut u64, lo: i64, hi: i64) -> i64 {
    lo + (splitmix(state) % (hi - lo + 1) as u64) as i64
}

fn vm_int(src: &str) -> i64 {
    let out = vm_outcome(src);
    assert_eq!(out.error, None, "corpus program must run on the VM:\n{src}");
    out.output
        .trim()
        .parse::<i64>()
        .unwrap_or_else(|_| panic!("VM output is not an int: {:?}", out.output))
}

/// Substitute a call's literal arguments into a corpus template's Main.
fn with_args(src: &str, fn_name: &str, old: &str, args: &[i64]) -> String {
    let call: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    src.replace(old, &format!("{fn_name}({})", call.join(", ")))
}

// ============================================================
//  1. Int DAG: lowering, grid preservation, exact restoration
// ============================================================

#[test]
fn int_dag_lowers_with_value_constants_and_integer_weights() {
    let target = AnalogTarget::ideal();
    let n = analog_int_netlist_from_source(AFFINE_I, "f", &target).expect("int affine lowers");
    // Constants are integer VALUES (7.0), never float bit patterns; weights are integers.
    for cell in &n.cells {
        match cell {
            AnalogCell::Sum { inputs, weights } => {
                for w in weights {
                    assert_eq!(
                        w.fract(),
                        0.0,
                        "integer program ⇒ integer weights: {weights:?}"
                    );
                }
                for x in inputs {
                    if let logicaffeine_compile::codegen_ana::ASrc::Const(c) = x {
                        assert_eq!(c.fract(), 0.0, "integer constants decode as VALUES: {c}");
                    }
                }
            }
            AnalogCell::GainConst { k, .. } => assert_eq!(k.fract(), 0.0, "integer weight: {k}"),
            AnalogCell::Mul { .. } => {}
            // A switch passes grid values through; its own grid-preservation is enforced by
            // `assert_grid_preserving` at lowering time. Translinear tiles can never appear
            // in an int netlist (grid preservation refuses them).
            AnalogCell::Select { .. } => {}
            AnalogCell::Exp { .. }
            | AnalogCell::Log { .. }
            | AnalogCell::Tanh { .. }
            | AnalogCell::Sqrt { .. }
            | AnalogCell::Div { .. } => {
                panic!(
                    "translinear tiles are not grid-preserving — an int netlist cannot carry one"
                )
            }
        }
    }
}

#[test]
fn every_quantized_readout_restores_the_exact_vm_integer_on_ideal() {
    let target = AnalogTarget::ideal();
    for (label, src, fn_name, old_call, lo, hi, arity) in [
        ("affine", AFFINE_I, "f", "f(37)", -100i64, 100i64, 1usize),
        ("dot", DOT_I, "dot", "dot(5, -3, 7)", -8, 8, 3),
        ("varmul", VARMUL_I, "vm2", "vm2(5, -6)", -8, 8, 2),
    ] {
        let netlist = analog_int_netlist_from_source(src, fn_name, &target).expect("lowers");
        let ranges = assumed_ranges(src);
        let (schedule, cert) = scale_pass(&netlist, &ranges, &target).expect("schedules");
        let budget = error_budget(&netlist, &cert, &schedule, &target);
        exactness_gate(&budget)
            .unwrap_or_else(|e| panic!("`{label}` must prove exactness on ideal: {e}"));

        let mut seed = BASE_SEED ^ (arity as u64);
        for _ in 0..64 {
            let args: Vec<i64> = (0..arity).map(|_| int_in(&mut seed, lo, hi)).collect();
            let want = vm_int(&with_args(src, fn_name, old_call, &args));
            let xs: Vec<f64> = args.iter().map(|a| *a as f64).collect();
            let got = eval_analog_quantized(&schedule, &target, &xs).round() as i64;
            assert_eq!(
                got, want,
                "`{label}` at {args:?}: the restored integer must be EXACT"
            );
        }
    }
}

// ============================================================
//  2. The gate is honest physics; calibration makes the product
// ============================================================

#[test]
fn eight_bit_converters_refuse_plus_minus_100_and_prove_plus_minus_8() {
    let target = AnalogTarget::sky130_fpaa();

    // A unit-weight wide-range probe: `n + 7` at ±100 FITS the rails on 8 bits but each
    // converter half-LSB costs ~0.7 grid steps — provably inexact. (The ×2 affine would trip
    // the weight gate first on this chip; the exactness gate needs a shape that reaches it.)
    let wide_src = "## To f (n: Int) -> Int:\n    Assume n is between -100 and 100.\n    Return n + 7.\n\n## Main\n    Show f(37).\n";
    let wide = analog_int_netlist_from_source(wide_src, "f", &target).expect("lowers");
    let ranges = assumed_ranges(wide_src);
    let (schedule, cert) = scale_pass(&wide, &ranges, &target).expect("schedules");
    let budget = error_budget(&wide, &cert, &schedule, &target);
    match exactness_gate(&budget) {
        Err(AnaLowerError::ExactnessUnprovable { eps_math }) => {
            assert!(
                eps_math >= 0.5,
                "the refusal carries the budget that broke: {eps_math}"
            )
        }
        other => panic!("±100 through 8-bit converters cannot be proven exact, got {other:?}"),
    }

    let narrow = analog_int_netlist_from_source(ADD2_I, "add2", &target).expect("lowers");
    let ranges = assumed_ranges(ADD2_I);
    let (schedule, cert) = scale_pass(&narrow, &ranges, &target).expect("schedules");
    let budget = error_budget(&narrow, &cert, &schedule, &target);
    exactness_gate(&budget).expect("a ±8 unit-weight sum on 8-bit converters proves exact");
    // A unit-weight 4-way sum at ±8 honestly refuses on 8 bits: the block scale is set by the
    // ±32 output, so the four DAC half-LSBs alone cost ~0.9 grid steps. (Weighted dots refuse
    // even earlier on this chip — the ×3 leg exceeds the unit-element range itself.)
    let sum4_src = "## To sum4 (a: Int, b: Int, c: Int, d: Int) -> Int:\n    Assume a is between -8 and 8.\n    Assume b is between -8 and 8.\n    Assume c is between -8 and 8.\n    Assume d is between -8 and 8.\n    Return a + b + c + d.\n\n## Main\n    Show sum4(5, -3, 7, 1).\n";
    let sum4 = analog_int_netlist_from_source(sum4_src, "sum4", &target).expect("lowers");
    let ranges = assumed_ranges(sum4_src);
    let (schedule, cert) = scale_pass(&sum4, &ranges, &target).expect("schedules");
    let budget = error_budget(&sum4, &cert, &schedule, &target);
    assert!(
        matches!(
            exactness_gate(&budget),
            Err(AnaLowerError::ExactnessUnprovable { .. })
        ),
        "a 4-way ±8 sum must NOT be provable on 8-bit converters (ε = {:e})",
        budget.eps_math
    );
}

#[test]
fn calibration_restores_exactness_the_manufacturing_envelope_breaks() {
    let target = AnalogTarget::sky130_fpaa();
    let model = sky130_nonideal();
    let spec = v1_trim(&target);
    let netlist = analog_int_netlist_from_source(ADD2_I, "add2", &target).expect("lowers");
    let ranges = assumed_ranges(ADD2_I);
    let (schedule, cert) = scale_pass(&netlist, &ranges, &target).expect("schedules");
    let placement = place_analog(&schedule.quantized, &target).expect("places");

    // The 6σ manufacturing envelope breaks the ½-grid proof …
    let raw = error_budget_nonideal(&netlist, &cert, &schedule, &target, &model, &placement);
    assert!(
        matches!(
            exactness_gate(&raw),
            Err(AnaLowerError::ExactnessUnprovable { .. })
        ),
        "uncalibrated 6σ physics must NOT prove integer exactness (ε = {:e})",
        raw.eps_math
    );
    // … and half-trim-step residuals restore it.
    let cal = error_budget_calibrated(
        &netlist, &cert, &schedule, &target, &model, &placement, &spec,
    );
    exactness_gate(&cal)
        .unwrap_or_else(|e| panic!("calibrated residuals must prove exactness: {e}"));

    // The proof is live: 128 calibrated dies ALL read back the exact integer.
    let mut seed = BASE_SEED;
    for die in 0..128u64 {
        let inst = sample_instance(
            &schedule.quantized.cells,
            &placement,
            &model,
            BASE_SEED ^ (die << 16),
        );
        let table = calibrate(&inst, &spec);
        let cal_inst = apply_calibration(&inst, &table);
        for _ in 0..2 {
            let args: Vec<i64> = (0..2).map(|_| int_in(&mut seed, -8, 8)).collect();
            let want = vm_int(&with_args(ADD2_I, "add2", "add2(5, -3)", &args));
            let xs: Vec<f64> = args.iter().map(|a| *a as f64).collect();
            let got = eval_analog_instance(&schedule, &target, &cal_inst, &xs).round() as i64;
            assert_eq!(
                got, want,
                "die {die} at {args:?}: a calibrated die restores the EXACT integer"
            );
        }
    }
}

// ============================================================
//  3. The streamed integer MAC
// ============================================================

#[test]
fn the_streamed_integer_mac_restores_exact_integers() {
    let target = AnalogTarget::ideal();
    let r = analog_int_reduction_from_source(SUM_SEQ_I, "total", &target).expect("lowers");
    assert_eq!((r.min_len, r.max_len), (1, 8));
    let (schedule, cert) = reduction_scale_pass(&r, &target).expect("schedules");
    let budget = reduction_error_budget(&r, &cert, &schedule, &target);
    exactness_gate(&budget).expect("an 8-frame ±8 integer sum proves exact on ideal");

    let mut seed = BASE_SEED ^ 0xFEED;
    for _ in 0..32 {
        let len = 1 + (splitmix(&mut seed) % 8) as usize;
        let xs: Vec<i64> = (0..len).map(|_| int_in(&mut seed, -8, 8)).collect();
        let want: i64 = xs.iter().sum();
        let fxs: Vec<f64> = xs.iter().map(|a| *a as f64).collect();
        let got = eval_analog_sampled_quantized(&r, &schedule, &target, &fxs).round() as i64;
        assert_eq!(got, want, "streamed sum of {xs:?} must restore exactly");
    }
}

// ============================================================
//  4. Refusals
// ============================================================

#[test]
fn division_bitwise_and_missing_ranges_refuse_typed() {
    let target = AnalogTarget::ideal();

    let div = "## To f (n: Int) -> Int:\n    Assume n is between 1 and 100.\n    Return n / 2.\n\n## Main\n    Show f(10).\n";
    match analog_int_netlist_from_source(div, "f", &target) {
        Err(AnaLowerError::UnloweredOp(msg)) => {
            assert!(msg.contains("div") || msg.contains("Div"), "{msg}")
        }
        other => panic!("integer division has no exact analog tile, got {other:?}"),
    }

    let bitwise = "## To f (n: Int, m: Int) -> Int:\n    Assume n is between 0 and 8.\n    Assume m is between 0 and 8.\n    Return n & m.\n\n## Main\n    Show f(5, 3).\n";
    match analog_int_netlist_from_source(bitwise, "f", &target) {
        Err(AnaLowerError::UnloweredOp(msg)) => {
            assert!(msg.contains("And") || msg.contains("bitwise"), "{msg}")
        }
        other => panic!("bit patterns are not voltages, got {other:?}"),
    }

    let no_range = "## To f (n: Int) -> Int:\n    Return n * 2.\n\n## Main\n    Show f(3).\n";
    match analog_int_netlist_from_source(no_range, "f", &target) {
        Err(AnaLowerError::UnboundedInput(msg)) => assert!(msg.contains('n'), "{msg}"),
        other => panic!("an undeclared integer range must refuse, got {other:?}"),
    }
}

// ============================================================
//  5. The product surface: `compile_to_analog` serves int regions
// ============================================================

#[test]
fn the_pipeline_ships_certified_exact_integer_artifacts() {
    use logicaffeine_compile::compile::compile_to_analog;
    // ADD2 at ±8 proves exact on the CALIBRATED V1 chip — the artifact exists and its budget
    // IS the exactness proof (ε < ½ by the gate that admitted it).
    let art = compile_to_analog(ADD2_I).expect("an int region ships a certified-exact artifact");
    assert_eq!(art.fn_name, "add2");
    assert!(art.budget.recheck());
    assert!(
        art.budget.eps_math < 0.5,
        "the shipped budget carries the exactness proof"
    );

    // The ±100 ×2 affine refuses TYPED on this chip (the ×2 leg exceeds the unit elements).
    let err = compile_to_analog(AFFINE_I).expect_err("an unrealizable int region refuses");
    let msg = format!("{err:?}");
    assert!(msg.contains("analog backend:"), "{msg}");
}
