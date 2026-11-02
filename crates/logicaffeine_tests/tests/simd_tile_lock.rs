//! simd_tile_lock — configurable-precision integer SIMD, proven carry-isolated.
//!
//! The packed SWAR add the SIMD tile emits must be BIT-IDENTICAL to N independent scalar `width`-bit adds, for
//! ALL 64-bit inputs — no carry may ever cross a lane boundary (the "silent SIMD integer miscompile" is the
//! catastrophic bug this lock exists to make impossible). Certified three ways:
//!  * SAT — `swar_add_body ≡ lanewise_add_body` at the full 64-bit width, for every lane type.
//!  * NON-VACUITY — a naive single 64-bit add (carry bleeds) is NOT equal to the lane-wise spec, so the SAT is
//!    genuinely discriminating.
//!  * CONCRETE — the SWAR residual evaluated over a large input grid matches the lane-wise spec exactly.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(simd_tile_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::bitblast::lower_bool;
use logicaffeine_compile::codegen_sva::fabric_rtl::{eval_fabric, fabric_to_bounded, FabricOp};
use logicaffeine_compile::codegen_sva::simd::{
    eval_lanewise_add, eval_lanewise_fadd_f16x4, eval_lanewise_fadd_f32x2,
    eval_lanewise_fmul_f16x4, eval_lanewise_fmul_f32x2, f16_add, f16_mul, f16_to_f32, f32_to_f16,
    lanewise_add_body, naive_add_body, pack_f16x4, pack_f32x2, swar_add_body, LaneTy,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

/// Prove `a[ar] == b[br]` for ALL inputs at `width` via our certified SAT (refute the negation).
fn bodies_equal_sat(a: &[FabricOp], ar: usize, b: &[FabricOp], br: usize, width: u32) -> bool {
    let ea = fabric_to_bounded(a, ar, width);
    let eb = fabric_to_bounded(b, br, width);
    let claim = BoundedExpr::Eq(Box::new(ea), Box::new(eb));
    let p = lower_bool(&claim).expect("the lane-equivalence claim lowers to a bounded formula");
    prove_unsat(&ProofExpr::Not(Box::new(p))) == UnsatOutcome::Refuted
}

#[test]
fn the_packed_swar_add_is_carry_isolated_bit_identical_to_lanewise_scalar_adds() {
    for &ty in LaneTy::ALL {
        let (sw, swr) = swar_add_body(ty);
        let (lw, lwr) = lanewise_add_body(ty);
        assert!(
            bodies_equal_sat(&sw, swr, &lw, lwr, 64),
            "{ty:?}: the packed SWAR add is NOT bit-identical to {} lane-wise {}-bit adds — carry crosses a lane",
            ty.lanes(),
            ty.width()
        );
    }
    println!("SIMD: SWAR add certified carry-isolated ≡ lane-wise for I8x8, I16x4, I32x2 at 64-bit by our SAT");
}

#[test]
fn the_carry_isolation_proof_is_not_vacuous() {
    // A single 64-bit add lets carry bleed from lane 0 into lane 1, so it must NOT certify as lane-wise.
    let (nv, nvr) = naive_add_body();
    let (lw, lwr) = lanewise_add_body(LaneTy::I32x2);
    assert!(
        !bodies_equal_sat(&nv, nvr, &lw, lwr, 64),
        "a naive 64-bit add MUST differ from two lane-wise 32-bit adds — else the isolation SAT is vacuous"
    );
}

#[test]
fn the_swar_add_matches_the_lanewise_spec_over_a_large_input_grid() {
    // The lane-wise spec itself is right: hand-check one packed case (two 32-bit lanes that each wrap).
    // a = [0xFFFFFFFF, 1], b = [1, 2] ⇒ lane0 wraps to 0, lane1 = 3.
    let a = (1u64 << 32) | 0xFFFF_FFFF;
    let b = (2u64 << 32) | 1;
    assert_eq!(
        eval_lanewise_add(LaneTy::I32x2, a, b),
        3u64 << 32,
        "lane0 must wrap without touching lane1"
    );

    let mut checked = 0usize;
    for &ty in LaneTy::ALL {
        let (sw, swr) = swar_add_body(ty);
        // A deterministic spread of bit patterns incl. lane-boundary carries and the sign bits of each lane.
        let samples: [u64; 8] = [
            0,
            u64::MAX,
            0x0101_0101_0101_0101,
            0x8080_8080_8080_8080,
            0x7F7F_7F7F_7F7F_7F7F,
            0xFFFF_0000_FFFF_0000,
            0x1234_5678_9ABC_DEF0,
            0xDEAD_BEEF_CAFE_F00D,
        ];
        for &a in &samples {
            for &b in &samples {
                let got = eval_fabric(&sw, &[a as i64, b as i64], swr) as u64;
                let want = eval_lanewise_add(ty, a, b);
                assert_eq!(
                    got, want,
                    "{ty:?}: SWAR add {a:#018x}+{b:#018x} = {got:#018x} != spec {want:#018x}"
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 128,
        "the concrete grid must be non-trivial (got {checked})"
    );
    println!("SIMD: SWAR add matches the lane-wise spec over {checked} packed cases");
}

/// The IEEE-special f32 values that break naive lane packing (signed zeros, infinities, NaN, subnormals).
const F32_SPECIALS: &[f32] = &[
    0.0,
    -0.0,
    1.0,
    -1.0,
    f32::INFINITY,
    f32::NEG_INFINITY,
    f32::NAN,
    f32::MIN_POSITIVE,
    3.14159,
    1e30,
];

#[test]
fn packed_f32x2_ops_are_bit_exact_per_lane() {
    // Each lane of a packed F32×2 op must equal an INDEPENDENT scalar f32 op, bit-for-bit (NaN payloads incl.).
    let mut checked = 0usize;
    for &x0 in F32_SPECIALS {
        for &y0 in F32_SPECIALS {
            for &x1 in F32_SPECIALS {
                for &y1 in F32_SPECIALS {
                    let (a, b) = (pack_f32x2(x0, x1), pack_f32x2(y0, y1));

                    let add = eval_lanewise_fadd_f32x2(a, b);
                    assert_eq!(add as u32, (x0 + y0).to_bits(), "add lane0 mismatch");
                    assert_eq!(
                        (add >> 32) as u32,
                        (x1 + y1).to_bits(),
                        "add lane1 mismatch"
                    );

                    let mul = eval_lanewise_fmul_f32x2(a, b);
                    assert_eq!(mul as u32, (x0 * y0).to_bits(), "mul lane0 mismatch");
                    assert_eq!(
                        (mul >> 32) as u32,
                        (x1 * y1).to_bits(),
                        "mul lane1 mismatch"
                    );
                    checked += 1;
                }
            }
        }
    }
    println!(
        "SIMD-float: packed F32×2 add/mul bit-exact per lane over {checked} IEEE-special cases"
    );
}

#[test]
fn the_f16_reference_rounds_known_values_correctly() {
    // Independently anchor the f16 reference (so the RTL differential isn't checked against a buggy oracle).
    let cases: &[(f32, u16)] = &[
        (0.0, 0x0000),
        (-0.0, 0x8000),
        (1.0, 0x3C00),
        (-1.0, 0xBC00),
        (0.5, 0x3800),
        (2.0, 0x4000),
        (65504.0, 0x7BFF), // largest finite f16
        (65536.0, 0x7C00), // overflow → +Inf
        (f32::INFINITY, 0x7C00),
        (f32::NEG_INFINITY, 0xFC00),
        (2f32.powi(-14), 0x0400),             // smallest normal
        (2f32.powi(-24), 0x0001),             // smallest subnormal
        (2f32.powi(-25), 0x0000),             // half a subnormal ulp → ties to even (0)
        (1.0 + 2f32.powi(-11), 0x3C00),       // tie at 1.0 → to even (down)
        (1.0 + 3.0 * 2f32.powi(-11), 0x3C02), // tie → to even (up)
    ];
    for &(x, want) in cases {
        assert_eq!(f32_to_f16(x), want, "f32_to_f16({x}) should be {want:#06x}");
    }
    // Round-trips for exact f16 values.
    for bits in [0x3C00u16, 0x4000, 0x7BFF, 0x0400, 0x0001, 0xBC00] {
        assert_eq!(f32_to_f16(f16_to_f32(bits)), bits, "round-trip {bits:#06x}");
    }
    // NaN maps to a quiet NaN (exp all ones, nonzero mantissa).
    assert!((f32_to_f16(f32::NAN) & 0x7C00) == 0x7C00 && (f32_to_f16(f32::NAN) & 0x03FF) != 0);
}

/// IEEE-special f16 bit patterns.
const F16_SPECIALS: &[u16] = &[
    0x0000, 0x8000, 0x3C00, 0xBC00, 0x4000, 0x7C00, 0xFC00, 0x7E00, 0x0400, 0x0001, 0x03FF, 0x7BFF,
    0x4248,
];

#[test]
fn packed_f16x4_ops_are_bit_exact_per_lane() {
    let mut checked = 0usize;
    for &l0 in F16_SPECIALS {
        for &l1 in F16_SPECIALS {
            // Fix lanes 2,3 to a spread; sweep lanes 0,1 fully (full 4-fold product is 13^4 ≈ 28k — keep modest).
            for &l2 in &[0x3C00u16, 0x7C00, 0x0001, 0x7E00] {
                for &l3 in &[0xBC00u16, 0x0400, 0x7BFF, 0x8000] {
                    let a = pack_f16x4(l0, l1, l2, l3);
                    let b = pack_f16x4(l1, l2, l3, l0);
                    let add = eval_lanewise_fadd_f16x4(a, b);
                    let mul = eval_lanewise_fmul_f16x4(a, b);
                    let want_add = pack_f16x4(
                        f16_add(l0, l1),
                        f16_add(l1, l2),
                        f16_add(l2, l3),
                        f16_add(l3, l0),
                    );
                    let want_mul = pack_f16x4(
                        f16_mul(l0, l1),
                        f16_mul(l1, l2),
                        f16_mul(l2, l3),
                        f16_mul(l3, l0),
                    );
                    assert_eq!(add, want_add, "F16×4 add lane mismatch");
                    assert_eq!(mul, want_mul, "F16×4 mul lane mismatch");
                    checked += 1;
                }
            }
        }
    }
    println!("SIMD-float: packed F16×4 add/mul bit-exact per lane over {checked} cases");
}

#[test]
fn a_special_value_in_one_f16_lane_never_contaminates_the_others() {
    // Fix lane 0 to 1.0 + 1.0 = 2.0 (0x4000); whatever chaos lanes 1..3 hold, lane 0's result is unchanged.
    let base_add =
        eval_lanewise_fadd_f16x4(pack_f16x4(0x3C00, 0, 0, 0), pack_f16x4(0x3C00, 0, 0, 0)) as u16;
    let base_mul =
        eval_lanewise_fmul_f16x4(pack_f16x4(0x4000, 0, 0, 0), pack_f16x4(0x3C00, 0, 0, 0)) as u16;
    assert_eq!(base_add, 0x4000, "lane0 add sanity (1+1=2)");
    for &s in F16_SPECIALS {
        for &t in F16_SPECIALS {
            let a = pack_f16x4(0x3C00, s, t, s);
            let b = pack_f16x4(0x3C00, t, s, t);
            assert_eq!(
                eval_lanewise_fadd_f16x4(a, b) as u16,
                base_add,
                "lane1..3 add contaminated lane0"
            );
            let am = pack_f16x4(0x4000, s, t, s);
            let bm = pack_f16x4(0x3C00, t, s, t);
            assert_eq!(
                eval_lanewise_fmul_f16x4(am, bm) as u16,
                base_mul,
                "lane1..3 mul contaminated lane0"
            );
        }
    }
    println!("SIMD-float: F16×4 lanes are contamination-free");
}

#[test]
fn a_special_value_in_one_f32_lane_never_contaminates_the_other() {
    // Fix lane 0 to a plain 1.0 + 2.0 = 3.0; whatever chaos (Inf/NaN/subnormal) lane 1 holds, lane 0's result
    // bits must be IDENTICAL — no cross-lane contamination.
    let (x0, y0) = (1.0f32, 2.0f32);
    let base_lo = eval_lanewise_fadd_f32x2(pack_f32x2(x0, 0.0), pack_f32x2(y0, 0.0)) as u32;
    let base_mul = eval_lanewise_fmul_f32x2(pack_f32x2(x0, 0.0), pack_f32x2(y0, 0.0)) as u32;
    assert_eq!(base_lo, 3.0f32.to_bits(), "lane0 add sanity");
    for &s1 in F32_SPECIALS {
        for &t1 in F32_SPECIALS {
            let lo_add = eval_lanewise_fadd_f32x2(pack_f32x2(x0, s1), pack_f32x2(y0, t1)) as u32;
            let lo_mul = eval_lanewise_fmul_f32x2(pack_f32x2(x0, s1), pack_f32x2(y0, t1)) as u32;
            assert_eq!(lo_add, base_lo, "lane1 add {s1}+{t1} contaminated lane0");
            assert_eq!(lo_mul, base_mul, "lane1 mul {s1}*{t1} contaminated lane0");
        }
    }
    println!("SIMD-float: F32×2 lanes are contamination-free — a NaN/Inf lane cannot perturb its neighbor");
}
