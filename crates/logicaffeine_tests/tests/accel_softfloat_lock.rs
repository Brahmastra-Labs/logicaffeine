//! IEEE-754 binary64 on integer lanes: every float cell routine, at every lane shape, gives the bits the host's
//! binary64 unit gives (through `FuncUnit::eval_checked`, which `fabric_checked_semantics_lock` holds to the
//! VM's kernel), with the VM's divergence (a zero divisor) in the fault word.
//!
//! A NaN result is held to one documented rule instead of to the host's bits: the first NaN operand with its
//! quiet bit set, or the x86 default NaN `0xFFF8_0000_0000_0000` for an invalid operation. The host's own NaN
//! for a commutative op depends on which operand its compiler placed first, and no LOGOS operation reads a
//! NaN's sign or payload (`fmt_f64` prints every NaN as `NaN`; no builtin exposes a float's bits), so the
//! host is asked only that its result is a NaN too.

use logicaffeine_compile::accel::cells::{routine, CellOp};
use logicaffeine_compile::accel::lane::{join_u64, run, split_u64, ExactLanes, LaneShape};
use logicaffeine_compile::codegen_sva::fabric_rtl::{Divergence, FuncUnit};

const DEFAULT_NAN: u64 = 0xFFF8_0000_0000_0000;
const QUIET: u64 = 1 << 51;

fn shapes() -> Vec<(LaneShape, usize)> {
    vec![
        (LaneShape::new(32, 16), 56),
        (LaneShape::new(32, 8), 40),
        (LaneShape::new(16, 8), 24),
        (LaneShape::new(8, 4), 14),
        (LaneShape::new(4, 2), 9),
        (LaneShape::new(2, 1), 7),
    ]
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The first `count` of: the special classes, the rounding and range edges, then random values.
fn floats(count: usize) -> Vec<u64> {
    let mut v: Vec<u64> = vec![
        1.0f64.to_bits(),
        0.0f64.to_bits(),
        (-0.0f64).to_bits(),
        f64::NAN.to_bits(),
        f64::INFINITY.to_bits(),
        f64::NEG_INFINITY.to_bits(),
        (-1.5f64).to_bits(),
        5e-324f64.to_bits(),
        f64::MIN_POSITIVE.to_bits(),
        f64::MAX.to_bits(),
        0.1f64.to_bits(),
        (-3.0f64).to_bits(),
        (1.0f64 + f64::EPSILON).to_bits(),
        0x7FF0_0000_0000_0001,
        0xFFF8_0000_0000_0001,
        (f64::MIN_POSITIVE - 5e-324).to_bits(),
        9_007_199_254_740_992.0f64.to_bits(),
        9_223_372_036_854_775_808.0f64.to_bits(),
        (-9_223_372_036_854_775_808.0f64).to_bits(),
        (1.0f64 / 3.0).to_bits(),
        2.5f64.to_bits(),
        (-2.5f64).to_bits(),
        1e308f64.to_bits(),
        (-1e-310f64).to_bits(),
        4503599627370496.5f64.to_bits(),
        (0.5f64 - f64::EPSILON / 4.0).to_bits(),
    ];
    let mut s = 0x0F10_A7F1_0A70_0001;
    while v.len() < count {
        match splitmix(&mut s) % 4 {
            0 => v.push(splitmix(&mut s)),
            1 => v.push(((splitmix(&mut s) as i64 >> 40) as f64).to_bits()),
            2 => v.push(
                (f64::from_bits(splitmix(&mut s) >> 12 | 0x3FF0_0000_0000_0000) - 1.0).to_bits(),
            ),
            _ => v.push(splitmix(&mut s) & 0x800F_FFFF_FFFF_FFFF),
        }
    }
    v.truncate(count);
    v
}

fn ints(count: usize) -> Vec<i64> {
    let mut v: Vec<i64> = vec![
        0,
        1,
        -1,
        i64::MAX,
        i64::MIN,
        (1 << 53) + 1,
        -(1 << 53) - 1,
        3,
        1 << 62,
        -(1 << 62) - 1,
        123_456_789,
        1 << 60,
    ];
    let mut s = 0xAB_CDEF;
    while v.len() < count {
        v.push(splitmix(&mut s) as i64);
        let k = splitmix(&mut s) % 62;
        v.push((splitmix(&mut s) as i64) >> k);
    }
    v.truncate(count);
    v
}

fn is_nan(bits: u64) -> bool {
    f64::from_bits(bits).is_nan()
}

/// The NaN the documented rule produces for a two-operand op, when one operand is NaN.
fn propagated(a: u64, b: u64) -> u64 {
    if is_nan(a) {
        a | QUIET
    } else {
        b | QUIET
    }
}

fn device(shape: LaneShape, unit: FuncUnit, a: u64, b: u64) -> (u64, u32) {
    let r = routine(shape, CellOp::Unit(unit));
    let n = shape.words_for(64);
    let mut args = split_u64(shape, a, n);
    args.extend(split_u64(shape, b, n));
    let mut lanes = ExactLanes { shape };
    let (out, _) =
        run(&r, &mut lanes, &args).unwrap_or_else(|e| panic!("{shape:?} {}: {e}", r.name));
    (
        join_u64(shape, &out[..n]),
        join_u64(shape, &out[n..]) as u32,
    )
}

/// Whether the routine's `(bits, fault)` is the checked semantics' result for `unit(a, b)`; a NaN result is
/// held to `nan_rule` (the documented NaN) and to the host's result being a NaN.
fn agrees(unit: FuncUnit, a: u64, b: u64, got: (u64, u32), nan_rule: Option<u64>) -> bool {
    let want = unit.eval_checked(a as i64, b as i64);
    match want {
        Err(d) => got.1 == d.code(),
        Ok(w) => {
            let w = w as u64;
            if got.1 != 0 {
                return false;
            }
            match nan_rule {
                Some(nan) if is_nan(w) => got.0 == nan,
                _ => got.0 == w,
            }
        }
    }
}

/// The documented NaN of a result, or `None` when the result is not a NaN.
fn nan_of(unit: FuncUnit, a: u64, b: u64) -> Option<u64> {
    let (fa, fb) = (f64::from_bits(a), f64::from_bits(b));
    match unit {
        FuncUnit::FAddF | FuncUnit::FSubF | FuncUnit::FMulF | FuncUnit::FDivF => {
            if fa.is_nan() || fb.is_nan() {
                Some(propagated(a, b))
            } else {
                Some(DEFAULT_NAN)
            }
        }
        FuncUnit::FSqrtF => Some(if fa.is_nan() { a | QUIET } else { DEFAULT_NAN }),
        _ => None,
    }
}

fn sweep(units: &[FuncUnit], int_operands: bool) -> Vec<String> {
    let mut bad = Vec::new();
    for (shape, count) in shapes() {
        let fs = floats(count);
        let is = ints(count);
        for &unit in units {
            let xs: Vec<u64> = if int_operands {
                is.iter().map(|x| *x as u64).collect()
            } else {
                fs.clone()
            };
            for &a in &xs {
                for &b in &xs {
                    let got = device(shape, unit, a, b);
                    let nan = nan_of(unit, a, b);
                    if !agrees(unit, a, b, got, nan) {
                        let want = unit.eval_checked(a as i64, b as i64);
                        bad.push(format!(
                            "{shape:?} {unit:?}({a:#018x} = {}, {b:#018x} = {}): lanes ({:#018x}, fault {}), checked {:?}",
                            f64::from_bits(a),
                            f64::from_bits(b),
                            got.0,
                            got.1,
                            want.map(|w| format!("{:#018x}", w as u64)),
                        ));
                    }
                }
            }
        }
    }
    bad
}

fn report(bad: &[String], what: &str) {
    assert!(
        bad.is_empty(),
        "{} {what} differ:\n{}",
        bad.len(),
        bad.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn addition_and_subtraction_are_ieee_at_every_shape() {
    report(
        &sweep(&[FuncUnit::FAddF, FuncUnit::FSubF], false),
        "sums/differences",
    );
}

#[test]
fn multiplication_is_ieee_at_every_shape() {
    report(&sweep(&[FuncUnit::FMulF], false), "products");
}

#[test]
fn division_is_ieee_and_a_zero_divisor_is_the_vms_failure() {
    report(&sweep(&[FuncUnit::FDivF], false), "quotients");
    let shape = LaneShape::new(32, 16);
    for zero in [0.0f64.to_bits(), (-0.0f64).to_bits()] {
        let (_, fault) = device(shape, FuncUnit::FDivF, 1.0f64.to_bits(), zero);
        assert_eq!(
            fault,
            Divergence::FloatDivideByZero.code(),
            "a {} divisor is the VM's `Division by zero`",
            f64::from_bits(zero)
        );
    }
}

#[test]
fn square_root_floor_and_comparisons_are_ieee_at_every_shape() {
    report(
        &sweep(
            &[
                FuncUnit::FSqrtF,
                FuncUnit::FFloorF,
                FuncUnit::FLtF,
                FuncUnit::FGtF,
                FuncUnit::FLeF,
                FuncUnit::FGeF,
                FuncUnit::FEqF,
                FuncUnit::FNeF,
            ],
            false,
        ),
        "roots/floors/comparisons",
    );
}

#[test]
fn integer_conversion_and_approximate_equality_are_the_vms() {
    report(
        &sweep(&[FuncUnit::IntToF, FuncUnit::ApproxEqF], true),
        "conversions/approximate comparisons",
    );
}

/// The rounding boundaries by construction: exact ties go to even, just past a tie goes away from zero, the
/// gradual-underflow ties, and the overflow to infinity.
#[test]
fn rounding_ties_underflow_and_overflow_land_where_ieee_puts_them() {
    let one = 1.0f64;
    let e = f64::EPSILON;
    let cases: Vec<(FuncUnit, f64, f64)> = vec![
        (FuncUnit::FAddF, one, e / 2.0),
        (FuncUnit::FAddF, one + e, e / 2.0),
        (FuncUnit::FAddF, one, e / 2.0 + e / 4.0),
        (FuncUnit::FSubF, one, e / 4.0),
        (FuncUnit::FMulF, one + e, one + e),
        (FuncUnit::FMulF, f64::MIN_POSITIVE, 0.5),
        (FuncUnit::FMulF, 3.0 * 5e-324, 0.5),
        (FuncUnit::FMulF, 5e-324, 0.5),
        (FuncUnit::FMulF, f64::MAX, 2.0),
        (FuncUnit::FMulF, f64::MAX, 1.0 + e),
        (FuncUnit::FAddF, f64::MAX, f64::MAX / 2.0f64.powi(53)),
        (FuncUnit::FDivF, one, 3.0),
        (FuncUnit::FDivF, f64::MIN_POSITIVE, 3.0),
        (FuncUnit::FDivF, f64::MAX, 0.5),
        (FuncUnit::FSqrtF, 2.0, 0.0),
        (FuncUnit::FSqrtF, 5e-324, 0.0),
        (FuncUnit::FSubF, 1e16, 1e16 - 2.0),
        (FuncUnit::FAddF, -0.0, -0.0),
        (FuncUnit::FSubF, -0.0, 0.0),
        (FuncUnit::FAddF, 1.0, -1.0),
    ];
    let mut bad = Vec::new();
    for (shape, _) in shapes() {
        for (unit, a, b) in &cases {
            let (a, b) = (a.to_bits(), b.to_bits());
            let got = device(shape, *unit, a, b);
            if !agrees(*unit, a, b, got, nan_of(*unit, a, b)) {
                bad.push(format!(
                    "{shape:?} {unit:?}({}, {}): lanes {:#018x}, host {:#018x}",
                    f64::from_bits(a),
                    f64::from_bits(b),
                    got.0,
                    unit.eval(a as i64, b as i64) as u64
                ));
            }
        }
    }
    report(&bad, "boundary cases");
}

/// The NaN rule itself, on the device alone: the first NaN operand quieted, else the default NaN.
#[test]
fn nan_results_follow_the_documented_rule() {
    let shape = LaneShape::new(32, 16);
    let snan = 0x7FF0_0000_0000_0001u64;
    let qnan_neg = 0xFFF8_0000_0000_0005u64;
    let cases: [(FuncUnit, u64, u64, u64); 7] = [
        (FuncUnit::FAddF, snan, qnan_neg, snan | QUIET),
        (FuncUnit::FAddF, 1.0f64.to_bits(), qnan_neg, qnan_neg),
        (FuncUnit::FMulF, qnan_neg, snan, qnan_neg),
        (
            FuncUnit::FSubF,
            f64::INFINITY.to_bits(),
            f64::INFINITY.to_bits(),
            DEFAULT_NAN,
        ),
        (
            FuncUnit::FMulF,
            0.0f64.to_bits(),
            f64::NEG_INFINITY.to_bits(),
            DEFAULT_NAN,
        ),
        (
            FuncUnit::FDivF,
            f64::INFINITY.to_bits(),
            f64::INFINITY.to_bits(),
            DEFAULT_NAN,
        ),
        (FuncUnit::FSqrtF, (-4.0f64).to_bits(), 0, DEFAULT_NAN),
    ];
    for (unit, a, b, want) in cases {
        let (got, fault) = device(shape, unit, a, b);
        assert_eq!(fault, 0, "{unit:?} does not fault");
        assert_eq!(got, want, "{unit:?}({a:#x}, {b:#x}) is {want:#x}");
    }
}

/// `is approximately` at its tolerance: integer pairs whose difference sits just inside, at, and just
/// outside `1e-9 · max(|a|, |b|)`, where the bound's `max` (not `min`, not the `1e-12` floor) decides.
#[test]
fn approximate_equality_holds_exactly_at_its_tolerance() {
    let mut bad = Vec::new();
    for (shape, _) in shapes().into_iter().take(4) {
        for base in [
            1_000_000_000i64,
            1_000_000_000_000,
            1_000_000_000_000_000,
            999_999_999_999_999_999,
            -1_000_000_000_000,
            4_000_000_000,
        ] {
            let edge = (base.unsigned_abs() as f64 * 1e-9) as i64;
            for d in [edge - 1, edge, edge + 1, edge + 2, 1, 0] {
                let (a, b) = (base, base.saturating_add(d));
                for (x, y) in [(a, b), (b, a)] {
                    let got = device(shape, FuncUnit::ApproxEqF, x as u64, y as u64);
                    if !agrees(FuncUnit::ApproxEqF, x as u64, y as u64, got, None) {
                        bad.push(format!(
                            "{shape:?} ApproxEqF({x}, {y}): lanes {}, checked {:?}",
                            got.0,
                            FuncUnit::ApproxEqF.eval_checked(x, y)
                        ));
                    }
                }
            }
        }
    }
    report(&bad, "tolerance-boundary comparisons");
}

/// The heavy sweep: `LOGOS_ACCEL_SOFTFLOAT_CASES` random operand pairs (default 50 000) per op at the RDU's and
/// Plasticine's shapes, drawn to reach every class: random bit patterns, near-equal magnitudes (cancellation),
/// neighbouring binades, subnormals and the overflow edge.
#[test]
#[ignore = "heavy: a wide random sweep of every float routine; run on purpose"]
fn heavy_random_sweep_of_every_float_routine() {
    let cases: u64 = std::env::var("LOGOS_ACCEL_SOFTFLOAT_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50_000);
    let units = [
        FuncUnit::FAddF,
        FuncUnit::FSubF,
        FuncUnit::FMulF,
        FuncUnit::FDivF,
        FuncUnit::FSqrtF,
        FuncUnit::FFloorF,
        FuncUnit::FLtF,
        FuncUnit::FEqF,
    ];
    let mut s = 0xC0FF_EE00_D15C_0DE5u64;
    let mut bad = Vec::new();
    for shape in [LaneShape::new(32, 16), LaneShape::new(32, 8)] {
        for _ in 0..cases {
            let a = splitmix(&mut s);
            let b = match splitmix(&mut s) % 5 {
                0 => splitmix(&mut s),
                1 => a ^ (splitmix(&mut s) & 0xFFFF),
                2 => {
                    (a & 0x800F_FFFF_FFFF_FFFF)
                        | ((a >> 52 & 0x7FF)
                            .wrapping_add(splitmix(&mut s) % 3)
                            .wrapping_sub(1)
                            & 0x7FF)
                            << 52
                }
                3 => splitmix(&mut s) & 0x800F_FFFF_FFFF_FFFF,
                _ => (a & 0x800F_FFFF_FFFF_FFFF) | 0x7FE0_0000_0000_0000,
            };
            for unit in units {
                let got = device(shape, unit, a, b);
                if !agrees(unit, a, b, got, nan_of(unit, a, b)) {
                    bad.push(format!(
                        "{shape:?} {unit:?}({a:#018x}, {b:#018x}): lanes {:#018x} fault {}",
                        got.0, got.1
                    ));
                }
            }
        }
    }
    report(&bad, "random float routine runs");
}
