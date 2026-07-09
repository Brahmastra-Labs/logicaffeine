//! Standalone prototype of the ML-KEM (Kyber) negacyclic NTT — scalar reference + AVX2 i16×16,
//! to prove the SIMD win and nail correctness before wiring an `ntt_kernel` into the Logos
//! codegen (mirroring `codegen/strsearch_kernel.rs`). NO logicaffeine deps — build with:
//!   rustc -O -C target-feature=+avx2 -C target-cpu=native scripts/ntt_simd_proto.rs -o /tmp/nttp && /tmp/nttp
//!
//! Faithful to the Kyber reference (signed i16 centered coeffs, Montgomery domain, R = 2^16,
//! q = 3329, the incomplete 7-level NTT over ℤ_q[X]/(X^256+1)). The zetas are Kyber's published
//! table (Montgomery form, bit-reversed). Correctness gate: invntt(ntt(f)) ≡ f (mod q).

#![allow(clippy::needless_range_loop)]

const Q: i32 = 3329;
const QINV: i32 = -3327; // q^-1 mod 2^16 (signed i16)

/// Kyber zetas[128] — ζ^bitrev(i) · R mod q, signed, Montgomery form.
const ZETAS: [i16; 128] = [
    -1044, -758, -359, -1517, 1493, 1422, 287, 202, -171, 622, 1577, 182, 962, -1202, -1474, 1468,
    573, -1325, 264, 383, -829, 1458, -1602, -130, -681, 1017, 732, 608, -1542, 411, -205, -1571,
    1223, 652, -552, 1015, -1293, 1491, -282, -1544, 516, -8, -320, -666, -1618, -1162, 126, 1469,
    -853, -90, -271, 830, 107, -1421, -247, -951, -398, 961, -1508, -725, 448, -1065, 677, -1275,
    -1103, 430, 555, 843, -1251, 871, 1550, 105, 422, 587, 177, -235, -291, -460, 1574, 1653, -246,
    778, 1159, -147, -777, 1483, -602, 1119, -1590, 644, -872, 349, 418, 329, -156, -75, 817, 1097,
    603, 610, 1322, -1285, -1465, 384, -1215, -136, 1218, -1335, -874, 220, -1187, -1659, -1185,
    -1530, -1278, 794, -1510, -854, -870, 478, -108, -308, 996, 991, 958, -1460, 1522, 1628,
];

#[inline]
fn montgomery_reduce(a: i32) -> i16 {
    let t = (a as i16).wrapping_mul(QINV as i16);
    ((a - (t as i32) * Q) >> 16) as i16
}
#[inline]
fn fqmul(a: i16, b: i16) -> i16 {
    montgomery_reduce(a as i32 * b as i32)
}
#[inline]
fn barrett_reduce(a: i16) -> i16 {
    const V: i32 = ((1 << 26) + Q / 2) / Q;
    let t = (((V * a as i32) + (1 << 25)) >> 26) as i16;
    a.wrapping_sub(t.wrapping_mul(Q as i16))
}

/// Forward negacyclic NTT (Kyber reference), in place. 7 levels, one zeta per block.
fn ntt(r: &mut [i16; 256]) {
    let mut k = 1usize;
    let mut len = 128usize;
    while len >= 2 {
        let mut start = 0usize;
        while start < 256 {
            let zeta = ZETAS[k];
            k += 1;
            for j in start..start + len {
                let t = fqmul(zeta, r[j + len]);
                r[j + len] = r[j].wrapping_sub(t);
                r[j] = r[j].wrapping_add(t);
            }
            start += 2 * len;
        }
        len >>= 1;
    }
}

/// Inverse NTT (Kyber reference), in place — includes the 1/128 · mont² final scaling.
fn invntt(r: &mut [i16; 256]) {
    const F: i16 = 1441; // mont² / 128
    let mut k = 127usize;
    let mut len = 2usize;
    while len <= 128 {
        let mut start = 0usize;
        while start < 256 {
            let zeta = ZETAS[k];
            k = k.wrapping_sub(1);
            for j in start..start + len {
                let t = r[j];
                r[j] = barrett_reduce(t.wrapping_add(r[j + len]));
                r[j + len] = r[j + len].wrapping_sub(t);
                r[j + len] = fqmul(zeta, r[j + len]);
            }
            start += 2 * len;
        }
        // k is decremented once per block; restore for the next (larger) level grouping
        k = k.wrapping_add(0);
        len <<= 1;
    }
    for j in 0..256 {
        r[j] = fqmul(r[j], F);
    }
}

/// AVX2 forward NTT: the `len ≥ 16` levels run 16-wide (contiguous loads, broadcast zeta, the
/// i16 Montgomery butterfly via mullo/mulhi); the `len < 8` tail stays scalar (those levels need
/// lane shuffles — the next increment). Bit-identical to the scalar `ntt`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn ntt_avx2(r: &mut [i16; 256]) {
    use std::arch::x86_64::*;
    let qv = _mm256_set1_epi16(Q as i16);
    let qinvv = _mm256_set1_epi16(QINV as i16);
    let mut k = 1usize;
    let mut len = 128usize;
    while len >= 16 {
        let mut start = 0usize;
        while start < 256 {
            let zeta = _mm256_set1_epi16(ZETAS[k]);
            k += 1;
            let mut j = start;
            while j < start + len {
                let aj = _mm256_loadu_si256(r.as_ptr().add(j) as *const __m256i);
                let ajl = _mm256_loadu_si256(r.as_ptr().add(j + len) as *const __m256i);
                // t = fqmul(zeta, ajl)
                let rlo = _mm256_mullo_epi16(zeta, ajl);
                let rhi = _mm256_mulhi_epi16(zeta, ajl);
                let tt = _mm256_mullo_epi16(rlo, qinvv);
                let tt = _mm256_mulhi_epi16(tt, qv);
                let t = _mm256_sub_epi16(rhi, tt);
                _mm256_storeu_si256(r.as_mut_ptr().add(j) as *mut __m256i, _mm256_add_epi16(aj, t));
                _mm256_storeu_si256(r.as_mut_ptr().add(j + len) as *mut __m256i, _mm256_sub_epi16(aj, t));
                j += 16;
            }
            start += 2 * len;
        }
        len >>= 1;
    }
    // len = 8: each 16-coeff block butterflies lanes [0..8) with [8..16). Process one block per
    // vector: swap the 128-bit halves to align the partners, fqmul the high half, then a
    // blend selects (lo+t) into the low half and (lo-t) into the high half. Bit-identical.
    debug_assert_eq!(len, 8);
    {
        // mask: low 128 bits = all-ones (keep add), high = zero — used with blendv to pick add/sub.
        let lo_mask = _mm256_set_epi64x(0, 0, -1, -1);
        let mut start = 0usize;
        while start < 256 {
            let zeta = _mm256_set1_epi16(ZETAS[k]);
            k += 1;
            let v = _mm256_loadu_si256(r.as_ptr().add(start) as *const __m256i);
            let hi = _mm256_permute2x128_si256(v, v, 0x01); // [v.hi128, v.lo128]
            // t = fqmul(zeta, v.hi128) in BOTH halves (we only use the low half)
            let rlo = _mm256_mullo_epi16(zeta, hi);
            let rhi = _mm256_mulhi_epi16(zeta, hi);
            let tt = _mm256_mullo_epi16(rlo, qinvv);
            let tt = _mm256_mulhi_epi16(tt, qv);
            let t = _mm256_sub_epi16(rhi, tt); // low half = fqmul(zeta, v[8..16))
            let add = _mm256_add_epi16(v, t); // low half = v[0..8)+t  (high half garbage)
            let sub = _mm256_sub_epi16(v, t); // low half = v[0..8)-t  (high half garbage)
            // We need low result in low half, high result in high half. add's low half is correct;
            // sub's low half is the high result — move it to the high half via a half-swap, then
            // blend: low 128 from `add`, high 128 from swapped `sub`.
            let sub_hi = _mm256_permute2x128_si256(sub, sub, 0x01); // sub.low → high half
            let out = _mm256_blendv_epi8(sub_hi, add, lo_mask);
            _mm256_storeu_si256(r.as_mut_ptr().add(start) as *mut __m256i, out);
            start += 16;
        }
        len >>= 1;
    }
    // len = 4: a 16-coeff vector holds TWO blocks (8 each), each with its own zeta. Partners are
    // the two 64-bit halves within each 128-bit lane → align via shuffle_epi32(_,0x4E).
    debug_assert_eq!(len, 4);
    {
        let mask4 = _mm256_set_epi64x(0, -1, 0, -1); // low 64 of each 128-bit lane ← add, high ← sub
        let mut start = 0usize;
        while start < 256 {
            let z0 = ZETAS[k];
            let z1 = ZETAS[k + 1];
            k += 2;
            let zv = _mm256_set_epi16(z1, z1, z1, z1, z1, z1, z1, z1, z0, z0, z0, z0, z0, z0, z0, z0);
            let v = _mm256_loadu_si256(r.as_ptr().add(start) as *const __m256i);
            let vhi = _mm256_shuffle_epi32(v, 0x4E); // each block's G_hi → its G_lo position
            let rlo = _mm256_mullo_epi16(zv, vhi);
            let rhi = _mm256_mulhi_epi16(zv, vhi);
            let tt = _mm256_mullo_epi16(rlo, qinvv);
            let tt = _mm256_mulhi_epi16(tt, qv);
            let t = _mm256_sub_epi16(rhi, tt);
            let add = _mm256_add_epi16(v, t);
            let sub = _mm256_sub_epi16(v, t);
            let sub_s = _mm256_shuffle_epi32(sub, 0x4E); // new-G_hi (in G_lo position) → G_hi position
            let out = _mm256_blendv_epi8(sub_s, add, mask4);
            _mm256_storeu_si256(r.as_mut_ptr().add(start) as *mut __m256i, out);
            start += 16;
        }
        len >>= 1;
    }
    // len = 2: a 16-coeff vector holds FOUR blocks (4 each). Partners are adjacent 32-bit lanes →
    // align via shuffle_epi32(_,0xB1).
    debug_assert_eq!(len, 2);
    {
        let mask2 = _mm256_set_epi32(0, -1, 0, -1, 0, -1, 0, -1);
        let mut start = 0usize;
        while start < 256 {
            let z0 = ZETAS[k];
            let z1 = ZETAS[k + 1];
            let z2 = ZETAS[k + 2];
            let z3 = ZETAS[k + 3];
            k += 4;
            let zv = _mm256_set_epi16(z3, z3, z3, z3, z2, z2, z2, z2, z1, z1, z1, z1, z0, z0, z0, z0);
            let v = _mm256_loadu_si256(r.as_ptr().add(start) as *const __m256i);
            let vhi = _mm256_shuffle_epi32(v, 0xB1); // each block's G_hi → its G_lo position
            let rlo = _mm256_mullo_epi16(zv, vhi);
            let rhi = _mm256_mulhi_epi16(zv, vhi);
            let tt = _mm256_mullo_epi16(rlo, qinvv);
            let tt = _mm256_mulhi_epi16(tt, qv);
            let t = _mm256_sub_epi16(rhi, tt);
            let add = _mm256_add_epi16(v, t);
            let sub = _mm256_sub_epi16(v, t);
            let sub_s = _mm256_shuffle_epi32(sub, 0xB1);
            let out = _mm256_blendv_epi8(sub_s, add, mask2);
            _mm256_storeu_si256(r.as_mut_ptr().add(start) as *mut __m256i, out);
            start += 16;
        }
    }
}

fn norm(x: i16) -> i32 {
    ((x as i32) % Q + Q) % Q
}

fn main() {
    // Round-trip: invntt(ntt(f)) ≡ f (mod q).
    let mut s = 0x1234_5678u64;
    let f0: [i16; 256] = std::array::from_fn(|_| {
        s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        ((s >> 16) % Q as u64) as i16
    });
    const MONT: i32 = 2285; // R mod q — invntt is `tomont`, so invntt(ntt(p)) = p·MONT mod q
    let mut r = f0;
    ntt(&mut r);
    invntt(&mut r);
    let want = |i: usize| (f0[i] as i32 * MONT).rem_euclid(Q);
    let ok = (0..256).all(|i| norm(r[i]) == want(i));
    println!(
        "scalar Kyber NTT round-trip invntt(ntt(f)) == f·MONT (mod q): {}",
        if ok { "OK" } else { "MISMATCH" }
    );
    if !ok {
        let bad: Vec<usize> = (0..256).filter(|&i| norm(r[i]) != want(i)).take(8).collect();
        for i in bad {
            println!("  [{i}] got {} want {}", norm(r[i]), want(i));
        }
    }

    // AVX2 forward NTT must be bit-identical to the scalar forward NTT.
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            let mut a = f0;
            let mut b = f0;
            ntt(&mut a);
            unsafe { ntt_avx2(&mut b) };
            let same = a == b;
            println!("AVX2 NTT == scalar NTT (bit-identical): {}", if same { "OK" } else { "MISMATCH" });

            // Benchmark: forward NTT, scalar vs AVX2 (median per-NTT over many iters).
            let iters = 200_000u64;
            let bench = |f: &dyn Fn(&mut [i16; 256])| -> f64 {
                let mut best = u128::MAX;
                for _ in 0..5 {
                    let mut x = f0;
                    let t0 = std::time::Instant::now();
                    for _ in 0..iters {
                        f(&mut x);
                        // re-seed cheaply so the optimizer can't hoist the whole loop
                        x[0] = x[0].wrapping_add(1);
                    }
                    let dt = t0.elapsed().as_nanos();
                    std::hint::black_box(&x);
                    if dt < best { best = dt; }
                }
                best as f64 / iters as f64
            };
            let s_ns = bench(&|x| ntt(x));
            let v_ns = bench(&|x| unsafe { ntt_avx2(x) });
            println!("\n=== n=256 forward NTT (i16 Kyber) — scalar vs AVX2 (this machine) ===");
            println!("  scalar : {s_ns:>7.1} ns / NTT");
            println!("  AVX2   : {v_ns:>7.1} ns / NTT   ({:.2}× faster)", s_ns / v_ns.max(0.001));
        } else {
            println!("(AVX2 not detected on this CPU — skipping SIMD bench)");
        }
    }
}
