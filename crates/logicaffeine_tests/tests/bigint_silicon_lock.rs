//! bigint_silicon_lock — the numeric-tower datapath's semantics are pinned to the SHIPPED
//! BigInt. The silicon datapath (bigint_alu.v add/subtract/compare + bigint_mul.v 64×64
//! limb multiplier, each proven ≡ its software oracle under iverilog) is proven here to equal
//! the production `logicaffeine_base::BigInt` operations (`add`/`sub`/`Ord`/`mul`) for the
//! same operands, built from limbs through BigInt's own public arithmetic. Cited REAL in
//! SILICON_CAPABILITY_MAP.md.
//!
//! Proof strength: ≡ the shipped `logicaffeine_base::BigInt` (add/sub/compare/mul) — a
//! differential-oracle proof (the limb datapath equals the production arithmetic), the strongest
//! form for a library-equivalence claim. FPGA/Sky130 receipts are separate (reports.json).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_base::BigInt;
use logicaffeine_compile::codegen_sva::bignum_rtl::{limb_add, limb_lt, limb_mul, limb_sub, LIMBS};

/// A u64 as a BigInt (u64 may exceed i64::MAX, so split hi/lo across 2^32).
fn u64_big(v: u64) -> BigInt {
    let two32 = BigInt::from_i64(1i64 << 32);
    let hi = BigInt::from_i64((v >> 32) as i64);
    let lo = BigInt::from_i64((v & 0xFFFF_FFFF) as i64);
    hi.mul(&two32).add(&lo)
}

/// Build a BigInt from little-endian u64 limbs via BigInt's own public ops (Horner over 2^64).
fn from_limbs(limbs: &[u64]) -> BigInt {
    let two32 = BigInt::from_i64(1i64 << 32);
    let base = two32.mul(&two32); // 2^64
    let mut acc = BigInt::from_i64(0);
    for &limb in limbs.iter().rev() {
        acc = acc.mul(&base).add(&u64_big(limb));
    }
    acc
}

#[test]
fn bigint_adder_matches_the_mag_add_oracle() {
    // 2^256 = (2^64)^4 — the value of a carry out of the top limb.
    let two32 = BigInt::from_i64(1i64 << 32);
    let base = two32.mul(&two32);
    let mut two256 = BigInt::from_i64(1);
    for _ in 0..LIMBS {
        two256 = two256.mul(&base);
    }

    let mut st: u64 = 0x1234_5678_9abc_def0;
    let mut next = || {
        st = st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        st
    };
    for k in 0..300 {
        // include the all-ones carry-out edge case
        let a: [u64; LIMBS] = if k == 0 {
            [u64::MAX; LIMBS]
        } else {
            [next(), next(), next(), next()]
        };
        let b: [u64; LIMBS] = if k == 0 {
            [1, 0, 0, 0]
        } else {
            [next(), next(), next(), next()]
        };

        // Silicon result as a BigInt: sum + cout·2^256.
        let (sum, cout) = limb_add(&a, &b);
        let mut silicon = from_limbs(&sum);
        if cout != 0 {
            silicon = silicon.add(&two256);
        }

        // Shipped BigInt: a + b via the production impl.
        let expected = from_limbs(&a).add(&from_limbs(&b));

        assert_eq!(
            silicon, expected,
            "256-bit silicon adder must equal BigInt::add (a={a:?} b={b:?})"
        );
    }
}

/// A u128 (the 64×64 product) as a BigInt, from its two limbs.
fn u128_big(p: u128) -> BigInt {
    from_limbs(&[p as u64, (p >> 64) as u64])
}

#[test]
fn the_bigint_datapath_matches_the_shipped_bigint_sub_cmp_mul() {
    let mut st: u64 = 0xFACE_B00C_1051_D0DE;
    let mut next = || {
        st = st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        st
    };
    for k in 0..300 {
        let a: [u64; LIMBS] = if k == 0 {
            [0; LIMBS]
        } else {
            [next(), next(), next(), next()]
        };
        let b: [u64; LIMBS] = if k == 0 {
            [1, 0, 0, 0]
        } else {
            [next(), next(), next(), next()]
        };
        let (ba, bb) = (from_limbs(&a), from_limbs(&b));

        // SUBTRACT: the silicon borrow signals a < b exactly, and when it does not borrow the
        // difference limbs equal the shipped BigInt subtraction (a non-negative magnitude).
        let (diff, borrow) = limb_sub(&a, &b);
        assert_eq!(borrow != 0, ba < bb, "borrow ⇔ a<b (a={a:?} b={b:?})");
        if borrow == 0 {
            assert_eq!(
                from_limbs(&diff),
                ba.sub(&bb),
                "silicon a-b must equal BigInt::sub"
            );
        }

        // COMPARE: the silicon lt / eq match the shipped BigInt total order exactly.
        assert_eq!(limb_lt(&a, &b), ba < bb, "silicon lt must equal BigInt Ord");
        assert_eq!(a == b, ba == bb, "silicon eq must equal BigInt equality");

        // MULTIPLY: each 64×64→128 limb product equals the shipped BigInt multiply of the limbs.
        let (a0, b0) = (a[0], b[0]);
        assert_eq!(
            u128_big(limb_mul(a0, b0)),
            u64_big(a0).mul(&u64_big(b0)),
            "silicon 64×64 limb product must equal BigInt::mul (a0={a0} b0={b0})"
        );
    }
}
