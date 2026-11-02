use super::*;

#[test]
fn word32_add_wraps_not_promotes() {
    // The whole point vs BigInt: MAX + 1 wraps to 0, it never grows.
    assert_eq!(Word32::MAX.add(Word32::ONE), Word32::ZERO);
    assert_eq!(Word32(0xFFFF_FFFF).add(Word32(2)), Word32(1));
}

#[test]
fn word64_add_wraps_not_promotes() {
    assert_eq!(Word64::MAX.add(Word64::ONE), Word64::ZERO);
    assert_eq!(Word64(u64::MAX).add(Word64(5)), Word64(4));
}

#[test]
fn div_rem_shl_shr_operators_are_unsigned_primitive() {
    use core::ops::{Div, Rem, Shl, Shr};
    // `% 2^k` is a mask, `/ 2^k` a shift — the crypto reduction primitives, unsigned.
    assert_eq!(
        Word32(0x1234_5678).rem(Word32(65536)),
        Word32(0x5678),
        "% 2^16 masks the low bits"
    );
    assert_eq!(
        Word32(0x1234_5678).div(Word32(65536)),
        Word32(0x1234),
        "/ 2^16 shifts right 16"
    );
    assert_eq!(
        Word32(7).div(Word32(2)),
        Word32(3),
        "unsigned truncating division"
    );
    assert_eq!(Word32(7).rem(Word32(3)), Word32(1), "unsigned remainder");
    assert_eq!(
        Word32(1).shl(31),
        Word32(0x8000_0000),
        "<< 31 sets the top bit"
    );
    assert_eq!(
        Word32(0x8000_0000).shr(31),
        Word32(1),
        ">> 31 is a logical (unsigned) shift"
    );
    assert_eq!(
        Word64(1).shl(63).shr(63),
        Word64(1),
        "Word64 logical shift round-trips the top bit"
    );
    assert_eq!(
        Word8(200).div(Word8(8)),
        Word8(25),
        "Word8 unsigned division (200 ÷ 8)"
    );
}

#[test]
fn rotl_is_cyclic_and_inverse_of_rotr() {
    let x = Word32(0x1234_5678);
    assert_eq!(
        x.rotl(8),
        Word32(0x3456_7812),
        "rotl by 8 is the documented permutation"
    );
    assert_eq!(x.rotl(0), x, "rotate by 0 is identity");
    assert_eq!(x.rotl(32), x, "a full rotation is identity");
    for n in 0..Word32::BITS {
        assert_eq!(x.rotl(n).rotr(n), x, "rotr undoes rotl by {n}");
    }
}

#[test]
fn agrees_with_native_wrapping_ops_over_fuzz() {
    // Cross-check every op against the primitive's own wrapping arithmetic — the oracle.
    let mut s = 0xDEAD_BEEF_CAFE_F00Du64;
    let mut next = || {
        s = s
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        s
    };
    for _ in 0..5000 {
        let a = next() as u32;
        let b = next() as u32;
        let n = (next() % 32) as u32;
        assert_eq!(Word32(a).add(Word32(b)).get(), a.wrapping_add(b), "add");
        assert_eq!(Word32(a).sub(Word32(b)).get(), a.wrapping_sub(b), "sub");
        assert_eq!(Word32(a).mul(Word32(b)).get(), a.wrapping_mul(b), "mul");
        assert_eq!(Word32(a).bitxor(Word32(b)).get(), a ^ b, "xor");
        assert_eq!(Word32(a).bitand(Word32(b)).get(), a & b, "and");
        assert_eq!(Word32(a).bitor(Word32(b)).get(), a | b, "or");
        assert_eq!(Word32(a).not().get(), !a, "not");
        assert_eq!(Word32(a).shl(n).get(), a.wrapping_shl(n), "shl");
        assert_eq!(Word32(a).shr(n).get(), a.wrapping_shr(n), "shr");
        assert_eq!(Word32(a).rotl(n).get(), a.rotate_left(n), "rotl");
        assert_eq!(Word32(a).rotr(n).get(), a.rotate_right(n), "rotr");
    }
}

#[test]
fn lanes8word32_avx2_xor_equals_scalar_lanes() {
    // The SIMD-correctness proof at the base level: the AVX2 lowering of lane-wise XOR must be
    // byte-identical to the scalar lanes (the spec). Fuzz the dispatching `bitxor` AND the raw
    // scalar path so both are exercised regardless of the host's feature set.
    let mut s = 0x0BAD_F00D_1234_5678u64;
    let mut next = || {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        s
    };
    for _ in 0..2000 {
        let a = Lanes8Word32(core::array::from_fn(|_| next() as u32));
        let b = Lanes8Word32(core::array::from_fn(|_| next() as u32));
        let n = (next() % 64) as u32; // exercises the n ≥ 32 reduction too
                                      // XOR.
        let xor_exp = Lanes8Word32(core::array::from_fn(|i| a.0[i] ^ b.0[i]));
        assert_eq!(a.bitxor_scalar(b), xor_exp, "scalar lane xor is the spec");
        assert_eq!(a.bitxor(b), xor_exp, "dispatched (AVX2) xor == spec");
        // Wrapping add.
        let add_exp = Lanes8Word32(core::array::from_fn(|i| a.0[i].wrapping_add(b.0[i])));
        assert_eq!(a.add_scalar(b), add_exp, "scalar lane add is the spec");
        assert_eq!(a.add(b), add_exp, "dispatched (AVX2) add == spec");
        // Left rotation (the ChaCha diffusion op).
        let rot_exp = Lanes8Word32(core::array::from_fn(|i| a.0[i].rotate_left(n % 32)));
        assert_eq!(
            a.rotl_scalar(n % 32),
            rot_exp,
            "scalar lane rotl is the spec"
        );
        assert_eq!(
            a.rotl(n),
            rot_exp,
            "dispatched (AVX2) rotl == spec (n mod 32)"
        );
        // Wrapping subtract (the i32 NTT butterfly difference).
        let sub_exp = Lanes8Word32(core::array::from_fn(|i| a.0[i].wrapping_sub(b.0[i])));
        assert_eq!(a.sub_scalar(b), sub_exp, "scalar lane sub32 is the spec");
        assert_eq!(a.sub(b), sub_exp, "dispatched (AVX2) sub32 == spec");
        // The signed i32 Montgomery multiply (ML-DSA q=8380417): AVX2 vpmuldq path == scalar.
        const Q: i32 = 8_380_417;
        const QINV: i32 = 58_728_449;
        let qv = Lanes8Word32::splat(Q as u32);
        let qiv = Lanes8Word32::splat(QINV as u32);
        // Inputs in [−q, q] (the NTT's working range) so the i64 product stays well within range.
        let ar = Lanes8Word32(core::array::from_fn(|i| (a.0[i] as i32 % Q) as u32));
        let br = Lanes8Word32(core::array::from_fn(|i| (b.0[i] as i32 % Q) as u32));
        let mont_exp = Lanes8Word32(core::array::from_fn(|i| {
            let p = (ar.0[i] as i32 as i64) * (br.0[i] as i32 as i64);
            let t = (p as i32).wrapping_mul(QINV) as i64;
            (((p - t * Q as i64) >> 32) as i32) as u32
        }));
        assert_eq!(
            ar.montmul32_scalar(br, qv, qiv),
            mont_exp,
            "scalar montmul32 is the spec"
        );
        assert_eq!(
            ar.montmul32(br, qv, qiv),
            mont_exp,
            "dispatched (AVX2) montmul32 == spec"
        );
        // The i32 within-vector-NTT permutes at every stride (h=4 vperm2i128, h=2/1 vpshufd/vpblendd).
        for &h in &[4usize, 2, 1] {
            let bl = Lanes8Word32(core::array::from_fn(|i| {
                a.0[(i / (2 * h)) * (2 * h) + (i % (2 * h)) % h]
            }));
            let bh = Lanes8Word32(core::array::from_fn(|i| {
                a.0[(i / (2 * h)) * (2 * h) + h + (i % (2 * h)) % h]
            }));
            let bd = Lanes8Word32(core::array::from_fn(|i| {
                if (i % (2 * h)) < h {
                    a.0[i]
                } else {
                    b.0[i]
                }
            }));
            assert_eq!(
                a.ntt_bcast_lo(h),
                bl,
                "i32 ntt_bcast_lo({h}) (AVX2) == scalar gather"
            );
            assert_eq!(
                a.ntt_bcast_hi(h),
                bh,
                "i32 ntt_bcast_hi({h}) (AVX2) == scalar gather"
            );
            assert_eq!(
                a.ntt_blend(b, h),
                bd,
                "i32 ntt_blend({h}) (AVX2) == scalar gather"
            );
        }
    }
    // splat / lane / pack / unpack round-trip.
    let v = Lanes8Word32::splat(0xABCD_1234);
    assert_eq!(v.lane(3), Word32(0xABCD_1234));
    assert_eq!(v.to_words()[7], Word32(0xABCD_1234));
    let packed = Lanes8Word32::from_words(&[Word32(1), Word32(2), Word32(3)]);
    assert_eq!(packed.0, [1, 2, 3, 0, 0, 0, 0, 0], "short slice zero-fills");
}

#[test]
fn lanes4word64_avx2_add_and_widemul_equal_scalar_lanes() {
    // The SIMD-correctness proof for the Poly1305 lane config: AVX2 add (`vpaddq`) and the
    // widening low-32 multiply (`vpmuludq`) must be byte-identical to the scalar lanes.
    let mut s = 0x1357_9bdf_2468_ace0u64;
    let mut next = || {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        s
    };
    for _ in 0..2000 {
        let a = Lanes4Word64(core::array::from_fn(|_| next()));
        let b = Lanes4Word64(core::array::from_fn(|_| next()));
        let add_exp = Lanes4Word64(core::array::from_fn(|i| a.0[i].wrapping_add(b.0[i])));
        assert_eq!(a.add_scalar(b), add_exp, "scalar lane add64 is the spec");
        assert_eq!(a.add(b), add_exp, "dispatched (AVX2) add64 == spec");
        let mul_exp = Lanes4Word64(core::array::from_fn(|i| {
            (a.0[i] & 0xffff_ffff) * (b.0[i] & 0xffff_ffff)
        }));
        assert_eq!(
            a.mul_lo32_wide_scalar(b),
            mul_exp,
            "scalar widening mul is the spec"
        );
        assert_eq!(
            a.mul_lo32_wide(b),
            mul_exp,
            "dispatched (AVX2) vpmuludq == spec"
        );
        // Horizontal sum.
        let hs = a.0[0]
            .wrapping_add(a.0[1])
            .wrapping_add(a.0[2])
            .wrapping_add(a.0[3]);
        assert_eq!(a.hsum(), hs, "horizontal sum");
    }
    let v = Lanes4Word64::from_words(&[Word64(5), Word64(9)]);
    assert_eq!(v.0, [5, 9, 0, 0], "short slice zero-fills");
    assert_eq!(v.lane(1), Word64(9));
    assert_eq!(LanesVal::L4W64(v).hsum(), 14, "5 + 9 + 0 + 0");
    assert_eq!(LanesVal::L4W64(v).type_name(), "Lanes4Word64");
}

#[test]
fn lanes16word16_avx2_ntt_ops_equal_scalar_lanes() {
    // The SIMD-correctness proof for the NTT lane config: AVX2 add/sub (`vpaddw`/`vpsubw`),
    // low-16 multiply (`vpmullw`), and SIGNED high-16 multiply (`vpmulhw`) must be byte-identical
    // to the scalar lanes (the latter is the Montgomery butterfly's `mulhi`).
    let mut s = 0x2468_ace0_1357_9bdfu64;
    let mut next = || {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (s >> 40) as u16
    };
    for _ in 0..3000 {
        let a = Lanes16Word16(core::array::from_fn(|_| next()));
        let b = Lanes16Word16(core::array::from_fn(|_| next()));
        let add_exp = Lanes16Word16(core::array::from_fn(|i| a.0[i].wrapping_add(b.0[i])));
        let sub_exp = Lanes16Word16(core::array::from_fn(|i| a.0[i].wrapping_sub(b.0[i])));
        let lo_exp = Lanes16Word16(core::array::from_fn(|i| a.0[i].wrapping_mul(b.0[i])));
        let hi_exp = Lanes16Word16(core::array::from_fn(|i| {
            (((a.0[i] as i16 as i32) * (b.0[i] as i16 as i32)) >> 16) as u16
        }));
        assert_eq!(a.add(b), add_exp, "lane add16 == spec");
        assert_eq!(a.sub(b), sub_exp, "lane sub16 == spec");
        assert_eq!(a.mullo(b), lo_exp, "lane mullo16 == spec");
        assert_eq!(a.mulhi(b), hi_exp, "lane (signed) mulhi16 == spec");
        // The within-vector-NTT lane permutes at every stride (h=8 `vperm2i128`, h=4/2
        // `vpshufd`/`vpblendd`): the dispatched (AVX2) path must equal the scalar gather.
        for &h in &[8usize, 4, 2] {
            let bl = Lanes16Word16(core::array::from_fn(|i| {
                a.0[(i / (2 * h)) * (2 * h) + (i % (2 * h)) % h]
            }));
            let bh = Lanes16Word16(core::array::from_fn(|i| {
                a.0[(i / (2 * h)) * (2 * h) + h + (i % (2 * h)) % h]
            }));
            let bd = Lanes16Word16(core::array::from_fn(|i| {
                if (i % (2 * h)) < h {
                    a.0[i]
                } else {
                    b.0[i]
                }
            }));
            assert_eq!(
                a.ntt_bcast_lo(h),
                bl,
                "ntt_bcast_lo({h}) (AVX2) == scalar gather"
            );
            assert_eq!(
                a.ntt_bcast_hi(h),
                bh,
                "ntt_bcast_hi({h}) (AVX2) == scalar gather"
            );
            assert_eq!(
                a.ntt_blend(b, h),
                bd,
                "ntt_blend({h}) (AVX2) == scalar gather"
            );
        }
    }
    assert_eq!(Lanes16Word16::splat(7).lane(9), Word16(7));
    let p = Lanes16Word16::from_words(&[Word16(3), Word16(5)]);
    assert_eq!(p.0[0..3], [3, 5, 0], "short slice zero-fills");
    assert_eq!(LanesVal::L16W16(p).type_name(), "Lanes16Word16");
    assert_eq!(LanesVal::L16W16(p).lanes(), 16);
}

#[test]
fn wordval_dispatches_by_width_and_rejects_mismatch() {
    let a = WordVal::W32(Word32(0xFFFF_FFFF));
    let b = WordVal::W32(Word32(1));
    assert_eq!(
        a.add(b),
        Some(WordVal::W32(Word32(0))),
        "same-width add wraps"
    );
    assert_eq!(a.width(), 32);
    assert_eq!(a.to_u64(), 0xFFFF_FFFF);

    let c = WordVal::W64(Word64(1));
    assert_eq!(a.add(c), None, "mixed-width add is a type error");
    assert_eq!(a.bitxor(c), None, "mixed-width xor is a type error");

    // Unary ops preserve width.
    assert_eq!(
        WordVal::W64(Word64(0)).not(),
        WordVal::W64(Word64(u64::MAX))
    );
    assert_eq!(
        WordVal::W32(Word32(0x1234_5678)).rotl(8),
        WordVal::W32(Word32(0x3456_7812))
    );

    // from_u64 / width round-trip.
    assert_eq!(
        WordVal::from_u64(32, 0x1_0000_0005),
        Some(WordVal::W32(Word32(5))),
        "32 truncates"
    );
    assert_eq!(WordVal::from_u64(64, 5), Some(WordVal::W64(Word64(5))));
    assert_eq!(
        WordVal::from_u64(16, 0x1_0005),
        Some(WordVal::W16(Word16(5))),
        "16 truncates"
    );
    assert_eq!(
        WordVal::from_u64(8, 0x105),
        Some(WordVal::W8(Word8(5))),
        "8 truncates"
    );
    assert_eq!(
        WordVal::from_u64(24, 5),
        None,
        "only 8/16/32/64 are valid widths"
    );

    // Display is the unsigned decimal value.
    assert_eq!(WordVal::W32(Word32(42)).to_string(), "42");
}
