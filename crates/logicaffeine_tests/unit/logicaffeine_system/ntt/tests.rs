use super::*;

#[test]
#[cfg(target_arch = "x86_64")]
fn reject_sample_avx2_bit_exact_vs_scalar() {
    if !std::is_x86_feature_detected!("avx2") {
        return;
    }
    // Fuzz many 168-byte blocks (incl. all-accept / all-reject boundaries) — the vectorized
    // rejection sampler MUST append exactly the coefficients the scalar reference does, in order.
    let mut s = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for iter in 0..2000 {
        let mut buf = [0u8; 168];
        match iter % 4 {
            0 => buf.iter_mut().for_each(|b| *b = (next() & 0xff) as u8), // random
            1 => buf.fill(0),                                             // all d=0 (accept)
            2 => buf.fill(0xff),                                          // all ≥ q (reject)
            _ => buf.iter_mut().enumerate().for_each(|(i, b)| {
                *b = if i % 3 == 1 {
                    0x0d
                } else {
                    (next() & 0xff) as u8
                }
            }),
        }
        let mut want: Vec<Word16> = Vec::with_capacity(256);
        reject_sample_block(&buf, &mut want);
        let mut got: Vec<Word16> = Vec::with_capacity(256);
        unsafe { reject_sample_block_avx2(&buf, &mut got) };
        assert_eq!(
            got, want,
            "AVX2 rejection sampler must be bit-identical to scalar (iter {iter})"
        );

        // Also validate the partial-fill path (out pre-loaded near the 256 cap).
        for pre in [0usize, 100, 250, 255] {
            let mut w = vec![Word16(1); pre];
            let mut g = vec![Word16(1); pre];
            reject_sample_block(&buf, &mut w);
            unsafe { reject_sample_block_avx2(&buf, &mut g) };
            assert_eq!(
                g, w,
                "AVX2 sampler must match scalar with {pre} pre-filled (iter {iter})"
            );
        }
    }
}

#[test]
fn word16_native_layer_is_correct() {
    let q = Q as u32;
    let a: Vec<Word16> = (0..256).map(|i| Word16((i * 13 % 3329) as u16)).collect();
    let b: Vec<Word16> = (0..256).map(|i| Word16((i * 29 % 3329) as u16)).collect();

    // modular arithmetic
    let add = mlkem_add_mod_q_w16(&a, &b).expect("in-contract arguments");
    let sub = mlkem_sub_mod_q_w16(&a, &b).expect("in-contract arguments");
    for i in 0..256 {
        assert_eq!(
            add.borrow()[i].0 as u32,
            (a[i].0 as u32 + b[i].0 as u32) % q
        );
        assert_eq!(
            sub.borrow()[i].0 as i32,
            (a[i].0 as i32 - b[i].0 as i32).rem_euclid(Q)
        );
    }
    assert_eq!(
        mlkem_zeros_w16(256)
            .expect("in-contract arguments")
            .borrow()
            .len(),
        256
    );
    assert!(mlkem_zeros_w16(256)
        .expect("in-contract arguments")
        .borrow()
        .iter()
        .all(|w| w.0 == 0));

    // ByteEncode∘ByteDecode round-trip (d=12) through the Word16/Word8 entries
    let enc = mlkem_byte_encode_w16_seq(&a, 12).expect("in-contract arguments");
    let dec = mlkem_byte_decode_w16_seq(&enc.borrow(), 12).expect("in-contract arguments");
    assert_eq!(
        dec.borrow().as_slice(),
        a.as_slice(),
        "byteDecode∘byteEncode = id (Word16)"
    );

    // cbd / sampleA Word16 entries match the i64 kernels (reduced into [0,q))
    let buf: Vec<Word8> = (0..128).map(|i| Word8((i * 7) as u8)).collect();
    let cbd_i64: Vec<i64> = mlkem_cbd2(&buf.iter().map(|w| w.0 as i64).collect::<Vec<_>>())
        .expect("in-contract arguments")
        .to_vec();
    let cbd_w16: Vec<i64> = mlkem_cbd2_w16_seq(&buf)
        .expect("in-contract arguments")
        .borrow()
        .iter()
        .map(|w| (w.0 as i64).rem_euclid(Q as i64))
        .collect();
    let cbd_i64_red: Vec<i64> = cbd_i64.iter().map(|&c| c.rem_euclid(Q as i64)).collect();
    assert_eq!(cbd_w16, cbd_i64_red, "cbd2 Word16 == reduced i64 cbd2");

    let seed: Vec<Word8> = (0..32).map(|i| Word8(i as u8)).collect();
    let sa_i64: Vec<i64> =
        mlkem_sample_a(&seed.iter().map(|w| w.0 as i64).collect::<Vec<_>>(), 1, 2)
            .expect("in-contract arguments")
            .to_vec();
    let sa_w16: Vec<i64> = mlkem_sample_a_w16_seq(&seed, 1, 2)
        .expect("in-contract arguments")
        .borrow()
        .iter()
        .map(|w| w.0 as i64)
        .collect();
    assert_eq!(sa_w16, sa_i64, "sampleA Word16 == i64 sampleA");
}

#[test]
fn scalar_round_trip_and_avx2_matches_scalar() {
    let mut s = 0x1234_5678u64;
    let f: [i16; 256] = std::array::from_fn(|_| {
        s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        ((s >> 16) % Q as u64) as i16
    });
    // Round-trip: invntt(ntt(f)) == f·MONT mod q (MONT = R mod q = 2285; invntt is `tomont`).
    const MONT: i32 = 2285;
    let mut r = f;
    ntt_scalar(&mut r);
    invntt_scalar(&mut r);
    let norm = |x: i16| ((x as i32) % Q + Q) % Q;
    assert!(
        (0..256).all(|i| norm(r[i]) == (f[i] as i32 * MONT).rem_euclid(Q)),
        "scalar Kyber NTT round-trip must hold"
    );

    #[cfg(target_arch = "x86_64")]
    if std::is_x86_feature_detected!("avx2") {
        let mut a = f;
        let mut b = f;
        ntt_scalar(&mut a);
        unsafe { ntt_avx2(&mut b) };
        assert_eq!(a, b, "AVX2 NTT must be bit-identical to scalar");
    }
}

#[test]
#[cfg(target_arch = "x86_64")]
fn invntt_avx2_matches_scalar() {
    if !std::is_x86_feature_detected!("avx2") {
        return;
    }
    // Fuzz a wide spread of NTT-domain inputs; the inverse must be bit-identical to the scalar
    // reference at every level (Gentleman-Sande + rounded Barrett + the final f-scaling).
    let mut s = 0xC0FFEE_1234_5678u64;
    for _ in 0..2000 {
        let r0: [i16; 256] = std::array::from_fn(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 33) % Q as u64) as i16
        });
        let mut a = r0;
        let mut b = r0;
        invntt_scalar(&mut a);
        unsafe { invntt_avx2(&mut b) };
        assert_eq!(a, b, "AVX2 inverse NTT must be bit-identical to scalar");
    }
}

fn negacyclic_conv(a: &[i64], b: &[i64]) -> Vec<i64> {
    let q = Q as i64;
    let n = 256;
    let mut c = vec![0i64; n];
    for i in 0..n {
        for j in 0..n {
            let prod = (a[i] * b[j]).rem_euclid(q);
            let k = i + j;
            if k < n {
                c[k] = (c[k] + prod).rem_euclid(q);
            } else {
                c[k - n] = (c[k - n] - prod).rem_euclid(q);
            }
        }
    }
    c
}

fn inv_mod_q(x: i64) -> i64 {
    let q = Q as i64;
    let (mut r, mut base, mut e) = (1i64, x.rem_euclid(q), q - 2);
    while e > 0 {
        if e & 1 == 1 {
            r = r * base % q;
        }
        base = base * base % q;
        e >>= 1;
    }
    r
}

/// The CBD spec, bit by bit: coefficient k = Σ(η bits) − Σ(η bits) from bit offset 2·η·k.
fn cbd_ref(buf: &[u8], eta: usize) -> [i16; 256] {
    let bit = |idx: usize| -> i16 { ((buf[idx / 8] >> (idx % 8)) & 1) as i16 };
    let mut r = [0i16; 256];
    for k in 0..256 {
        let base = 2 * eta * k;
        let (mut a, mut b) = (0i16, 0i16);
        for i in 0..eta {
            a += bit(base + i);
            b += bit(base + eta + i);
        }
        r[k] = a - b;
    }
    r
}

#[test]
fn sample_matrix_x4_matches_nine_scalar_entries() {
    // The 4-way batched matrix expander must be bit-identical to nine single-lane samples, for
    // several seeds (rejection consumes a data-dependent number of blocks per stream).
    for seed_byte in [0u8, 0x11, 0x5a, 0xff] {
        let seed: Vec<u8> = (0..32).map(|i| seed_byte ^ (i as u8 * 7)).collect();
        let matrix = mlkem_sample_matrix_w16(&seed);
        assert_eq!(matrix.len(), 9 * 256);
        for r in 0..3u8 {
            for c in 0..3u8 {
                let want = mlkem_sample_a_w16(&seed, r as i64, c as i64);
                let e = (r as usize * 3 + c as usize) * 256;
                assert_eq!(
                    &matrix[e..e + 256],
                    &want[..],
                    "matrix entry ({r},{c}) must match the scalar sampleA"
                );
            }
        }
    }
}

#[test]
fn cbd_bit_trick_matches_bit_by_bit_reference() {
    let mut s = 0xDEAD_BEEFu64;
    let mut rb = || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 56) as u8
    };
    let buf2: Vec<u8> = (0..128).map(|_| rb()).collect();
    assert_eq!(
        cbd_eta2(&buf2),
        cbd_ref(&buf2, 2),
        "fast CBD_2 must equal the bit-by-bit definition"
    );
    assert!(
        cbd_eta2(&buf2).iter().all(|&c| (-2..=2).contains(&c)),
        "CBD_2 ∈ [−2, 2]"
    );

    let buf3: Vec<u8> = (0..192).map(|_| rb()).collect();
    assert_eq!(
        cbd_eta3(&buf3),
        cbd_ref(&buf3, 3),
        "fast CBD_3 must equal the bit-by-bit definition"
    );
    assert!(
        cbd_eta3(&buf3).iter().all(|&c| (-3..=3).contains(&c)),
        "CBD_3 ∈ [−3, 3]"
    );
}

/// The SampleNTT spec, written with a different bit layout (`d1 = b0 | (b1&0xF)<<8`) than the
/// kernel's arithmetic form — they must agree.
fn sample_ntt_reference(stream: &[u8]) -> Vec<i64> {
    let q = Q as u32;
    let mut a = Vec::new();
    let mut i = 0;
    while a.len() < 256 {
        let (b0, b1, b2) = (stream[i] as u32, stream[i + 1] as u32, stream[i + 2] as u32);
        let d1 = b0 | ((b1 & 0x0F) << 8);
        let d2 = (b1 >> 4) | (b2 << 4);
        if d1 < q && a.len() < 256 {
            a.push(d1 as i64);
        }
        if d2 < q && a.len() < 256 {
            a.push(d2 as i64);
        }
        i += 3;
    }
    a
}

#[test]
fn sample_ntt_matches_reference_and_stays_in_field() {
    let mut s = 0xCAFE_F00Du64;
    let mut rb = || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 56) as u8
    };
    let stream: Vec<u8> = (0..2048).map(|_| rb()).collect();
    let stream_i: Vec<i64> = stream.iter().map(|&b| b as i64).collect();

    let got = mlkem_sample_ntt(&stream_i)
        .expect("in-contract arguments")
        .to_vec();
    assert_eq!(got.len(), 256, "SampleNTT yields exactly 256 coefficients");
    assert!(
        got.iter().all(|&c| (0..Q as i64).contains(&c)),
        "every coefficient is in [0, q)"
    );
    assert_eq!(
        got,
        sample_ntt_reference(&stream),
        "kernel SampleNTT must match the reference"
    );

    // Hand check: bytes 0x01,0x20,0x03 ⇒ d1 = 1 + 256·0 = 1, d2 = 2 + 16·3 = 50.
    let hand = [0x01i64, 0x20, 0x03]
        .iter()
        .chain(std::iter::repeat(&0))
        .take(2048)
        .copied()
        .collect::<Vec<_>>();
    let hg = mlkem_sample_ntt(&hand)
        .expect("in-contract arguments")
        .to_vec();
    assert_eq!(hg[0], 1, "first sampled coefficient");
    assert_eq!(hg[1], 50, "second sampled coefficient");
}

#[test]
fn sample_a_is_deterministic_and_consumes_its_own_xof() {
    let seed: Vec<u8> = (0..32u8).collect();
    let a1 = mlkem_sample_a_raw(&seed, 1, 2);
    let a2 = mlkem_sample_a_raw(&seed, 1, 2);
    assert_eq!(a1, a2, "SampleA is a deterministic function of (ρ, i, j)");
    assert_ne!(
        a1,
        mlkem_sample_a_raw(&seed, 2, 1),
        "index order matters (Â is not symmetric)"
    );
    assert_eq!(a1.len(), 256);
    assert!(a1.iter().all(|&c| (0..Q as i64).contains(&c)));

    // SampleA must equal SampleNTT over its own SHAKE128(ρ‖i‖j) stream.
    let mut xof_in = seed.clone();
    xof_in.push(1);
    xof_in.push(2);
    let stream = crate::keccak::shake128_bytes(&xof_in, 168 * 6);
    assert_eq!(
        a1,
        try_sample_ntt(&stream, 256).unwrap(),
        "SampleA = SampleNTT∘XOF"
    );
}

#[test]
fn byte_encode_decode_round_trips_losslessly() {
    // For every d ∈ {1,4,5,10,11,12} a polynomial of d-bit coefficients survives the
    // pack/unpack cycle exactly, and the packed length is 32·d bytes.
    let mut s = 0x1234_5678u64;
    let mut rc = |bound: i64| -> i64 {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 40) as i64 % bound
    };
    for &d in &[1usize, 4, 5, 10, 11, 12] {
        let modulus = if d == 12 { Q as i64 } else { 1i64 << d };
        let coeffs: Vec<i64> = (0..256).map(|_| rc(modulus)).collect();
        let bytes = mlkem_byte_encode_raw(&coeffs, d);
        assert_eq!(bytes.len(), 32 * d, "ByteEncode_{d} length must be 32·d");
        assert!(
            bytes.iter().all(|&b| (0..256).contains(&b)),
            "encoded values are bytes"
        );
        let back = mlkem_byte_decode_raw(&bytes, d);
        assert_eq!(
            back, coeffs,
            "ByteDecode_{d}∘ByteEncode_{d} must be the identity"
        );
    }
}

#[test]
fn compress_decompress_meets_fips203_error_bound() {
    // FIPS 203 guarantees |Decompress_d(Compress_d(x)) − x mod ±q| ≤ round(q / 2^(d+1)).
    let q = Q as i64;
    let mod_pm_abs = |a: i64| -> i64 {
        let mut r = a.rem_euclid(q);
        if r > q / 2 {
            r -= q;
        }
        r.abs()
    };
    for &d in &[1usize, 4, 5, 10, 11] {
        let bound = (q + (1i64 << d)) / (1i64 << (d + 1)); // round(q / 2^(d+1))
        for x in 0..q {
            let c = mlkem_compress_raw(&[x], d)[0];
            assert!(
                (0..(1i64 << d)).contains(&c),
                "Compress_{d}({x}) must be a d-bit value"
            );
            let back = mlkem_decompress_raw(&[c], d)[0];
            assert!(
                mod_pm_abs(back - x) <= bound,
                "d={d} x={x}: error {} exceeds bound {bound}",
                mod_pm_abs(back - x)
            );
        }
    }
}

#[test]
fn poly_multiply_via_ntt_matches_schoolbook_convolution() {
    let mut s = 0xCAFE_1234u64;
    let mut rand = || {
        s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        ((s >> 16) % Q as u64) as i64
    };
    let a: Vec<i64> = (0..256).map(|_| rand()).collect();
    let b: Vec<i64> = (0..256).map(|_| rand()).collect();
    // The full ML-KEM poly multiply: invntt(basemul(ntt(a), ntt(b))).
    let got = mlkem_inv_ntt_raw(&mlkem_base_mul_raw(&mlkem_ntt_raw(&a), &mlkem_ntt_raw(&b)));
    let conv = negacyclic_conv(&a, &b);
    // Determine the uniform Montgomery domain factor from the first non-zero coefficient.
    let k0 = (0..256)
        .find(|&k| conv[k] != 0)
        .expect("nonzero convolution");
    let factor = (got[k0] * inv_mod_q(conv[k0])).rem_euclid(Q as i64);
    assert!(
        (0..256).all(|k| got[k] == (conv[k] * factor).rem_euclid(Q as i64)),
        "invntt(basemul(ntt(a),ntt(b))) must equal the negacyclic convolution × a uniform factor (got factor {factor})"
    );
}

#[test]
fn mlkem_ntt_reduces_and_matches_scalar() {
    let input: Vec<i64> = (0..256).map(|i| (i * 37 % 5000) as i64 - 1000).collect();
    let got = mlkem_ntt_raw(&input);
    assert_eq!(got.len(), 256);
    assert!(
        got.iter().all(|&x| (0..Q as i64).contains(&x)),
        "output reduced into [0, q)"
    );
    // Independent scalar reference through the same reduction boundary.
    let mut r = [0i16; 256];
    for (slot, &x) in r.iter_mut().zip(&input) {
        *slot = x.rem_euclid(Q as i64) as i16;
    }
    ntt_scalar(&mut r);
    let want: Vec<i64> = r.iter().map(|&x| (x as i32).rem_euclid(Q) as i64).collect();
    assert_eq!(
        got, want,
        "mlkem_ntt must equal the scalar reference (mod q)"
    );
}

/// Same-run A/B: the vectorized Kyber Montgomery butterfly (`fqmul` ×16) built from the
/// `Lanes16Word16` lane API — the exact 5-op chain (`mullo·mulhi·mullo·mulhi·sub`) the compiled
/// `crypto.lg` NTT lowers to — vs. the identical raw-`__m256i` intrinsic chain. The lane methods
/// are `#[inline(always)]` + compile-time `cfg(target_feature="avx2")` (no runtime detect, no
/// `#[target_feature]` call boundary), so under `+avx2` a dependent chain stays register-resident:
/// LLVM forwards each op's store-to-`r` into the next op's load. This asserts (a) the lane fqmul is
/// bit-identical to the scalar `fqmul` oracle lane-by-lane, and (b) a 200k-deep lane chain equals
/// the raw-intrinsic chain bit-for-bit. It prints lane-vs-raw ns so register-residency is *measured*
/// (the box is shared; timing is informational, not asserted) — proving the "hand-assembled NTT in
/// Logos" lane ops hit the same roofline as raw intrinsics.
#[test]
#[cfg(target_arch = "x86_64")]
fn lanes16_montgomery_butterfly_eq_scalar_and_ab() {
    use logicaffeine_base::{Lanes16Word16, Word16};
    if !std::is_x86_feature_detected!("avx2") {
        return; // scalar-only box: the lane path is the portable fallback (covered by base tests)
    }
    let coeffs: [i16; 16] = std::array::from_fn(|i| ((i as i32 * 211 - 1500) % Q) as i16);
    let tw: [i16; 16] = std::array::from_fn(|i| ZETAS[i]);

    let qinv_v = Lanes16Word16::splat((QINV as i16) as u16);
    let q_v = Lanes16Word16::splat(Q as u16);
    let lane_fqmul = |a: Lanes16Word16, b: Lanes16Word16| -> Lanes16Word16 {
        let lo = a.mullo(b);
        let hi = a.mulhi(b);
        let m = lo.mullo(qinv_v);
        let th = m.mulhi(q_v);
        hi.sub(th)
    };
    let a = Lanes16Word16::from_words(&coeffs.map(|c| Word16(c as u16)));
    let b = Lanes16Word16::from_words(&tw.map(|c| Word16(c as u16)));
    let res = lane_fqmul(a, b);

    for i in 0..16 {
        let want = fqmul(coeffs[i], tw[i]);
        assert_eq!(
            res.lane(i).0 as i16,
            want,
            "lane fqmul lane {i} must equal scalar fqmul"
        );
    }

    const K: usize = 200_000;
    let mut v = a;
    let t0 = std::time::Instant::now();
    for _ in 0..K {
        v = lane_fqmul(v, b);
    }
    let lane_ns = t0.elapsed().as_nanos() as f64 / K as f64;
    let lane_sink = v.lane(0).0;

    let t1 = std::time::Instant::now();
    let raw_sink = unsafe { raw_fqmul_chain(coeffs, tw, K) };
    let raw_ns = t1.elapsed().as_nanos() as f64 / K as f64;

    assert_eq!(
        lane_sink, raw_sink,
        "lane and raw fqmul chains must be bit-identical after {K} iters"
    );

    println!(
        "\n=== Lanes16Word16 Montgomery butterfly (fqmul ×16) — 200k dependent chain, +avx2 ==="
    );
    println!("  raw __m256i intrinsics : {raw_ns:>7.3} ns/fqmul");
    println!(
        "  Lanes16 lane API       : {lane_ns:>7.3} ns/fqmul   ({:.2}× of raw)",
        lane_ns / raw_ns.max(0.001)
    );
    println!("  (sink {lane_sink}; shared box — timing informational, correctness asserted)");
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn raw_fqmul_chain(coeffs: [i16; 16], tw: [i16; 16], k: usize) -> u16 {
    use std::arch::x86_64::*;
    let mut v = _mm256_loadu_si256(coeffs.as_ptr() as *const __m256i);
    let b = _mm256_loadu_si256(tw.as_ptr() as *const __m256i);
    let qinv = _mm256_set1_epi16(QINV as i16);
    let q = _mm256_set1_epi16(Q as i16);
    for _ in 0..k {
        let lo = _mm256_mullo_epi16(v, b);
        let hi = _mm256_mulhi_epi16(v, b);
        let m = _mm256_mullo_epi16(lo, qinv);
        let th = _mm256_mulhi_epi16(m, q);
        v = _mm256_sub_epi16(hi, th);
    }
    let mut out = [0i16; 16];
    _mm256_storeu_si256(out.as_mut_ptr() as *mut __m256i, v);
    out[0] as u16
}

#[test]
fn barrett_reduce_centers_every_i16() {
    for x in i16::MIN..=i16::MAX {
        let r = barrett_reduce(x);
        assert!(
            (-(Q - 1) / 2..=(Q - 1) / 2).contains(&(r as i32)),
            "barrett_reduce({x}) = {r} is not centered"
        );
        assert_eq!(
            (r as i32).rem_euclid(Q),
            (x as i32).rem_euclid(Q),
            "barrett_reduce({x}) ≡ {x}"
        );
    }
}

#[test]
fn cadd_q_after_barrett_is_the_canonical_representative_of_every_i16() {
    for x in i16::MIN..=i16::MAX {
        assert_eq!(
            cadd_q(barrett_reduce(x)) as i32,
            (x as i32).rem_euclid(Q),
            "cadd_q(barrett_reduce({x}))"
        );
    }
}

#[test]
fn csub_q_reduces_every_value_below_twice_q() {
    for x in 0..2 * Q as u16 {
        assert_eq!(csub_q(x) as i32, x as i32 % Q, "csub_q({x})");
    }
}

#[test]
fn reduce_wide_is_the_canonical_representative_of_every_i64() {
    let mut cases: Vec<i64> = vec![i64::MIN, i64::MIN + 1, i64::MAX, i64::MAX - 1, 0, 1, -1];
    for k in [1i64, 2, 3, 1 << 20, 1 << 40, (1 << 50) + 7] {
        for m in [-k * Q as i64, k * Q as i64] {
            cases.extend([m - 1, m, m + 1]);
        }
    }
    for shift in 0..63 {
        let p = 1i64 << shift;
        cases.extend([p - 1, p, p + 1, -p - 1, -p, -p + 1]);
    }
    cases.extend(-200_000..=200_000);
    let mut s = 0x7E57_1A2Bu64;
    for _ in 0..1_000_000 {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        cases.push(s as i64);
    }
    for x in cases {
        assert_eq!(
            reduce_wide(x) as i64,
            x.rem_euclid(Q as i64),
            "reduce_wide({x})"
        );
    }
}

#[test]
fn compress_coefficient_is_the_fips203_rounding_for_every_coefficient_and_width() {
    for d in 1..=11usize {
        for x in 0..Q as u16 {
            let want = ((((x as u64) << d) + (Q as u64) / 2) / Q as u64) & ((1u64 << d) - 1);
            assert_eq!(compress_coefficient(x, d) as u64, want, "Compress_{d}({x})");
        }
    }
}
