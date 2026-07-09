//! ML-KEM (Kyber) negacyclic NTT runtime kernel — the verified scalar reference + AVX2 i16×16
//! path that compiled LOGOS reaches through the `mlkemNtt` stdlib function. Ported from the
//! validated `scripts/ntt_simd_proto.rs` (round-trip + AVX2==scalar bit-identical). q = 3329,
//! R = 2¹⁶, the incomplete 7-level transform over ℤ_q\[X\]/(X²⁵⁶+1). The Montgomery reduction it
//! uses is kernel-certified (`logicaffeine_kernel::field_algebra::montgomery_reduction_*`).
//!
//! Input/output are LOGOS `Int` (i64); coefficients are reduced into [0, q) at the boundary, the
//! transform runs in i16, and the result is returned reduced into [0, q). On x86-64 with AVX2 the
//! 16-wide kernel runs (≈49 ns/NTT on a modern core); otherwise the scalar path (≈147 ns).

const Q: i32 = 3329;
const QINV: i32 = -3327; // q⁻¹ mod 2¹⁶ (signed i16)

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

/// Reduce a value already in the Montgomery range `(−q, q)` into `[0, q)` with a single conditional
/// add — branchless (`x + (q & (x >> 15))`), auto-vectorizes 16-wide, and replaces the division-based
/// `rem_euclid(Q)` in the coefficient hot path. Sound ONLY when `|x| < q` (the reduction invariant).
#[inline]
fn cadd_q(x: i16) -> u16 {
    (x + (Q as i16 & (x >> 15))) as u16
}
#[inline]
fn fqmul(a: i16, b: i16) -> i16 {
    montgomery_reduce(a as i32 * b as i32)
}

/// Scalar forward negacyclic NTT (Kyber reference), in place.
fn ntt_scalar(r: &mut [i16; 256]) {
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

/// AVX2 forward NTT — all 7 levels vectorized 16-wide. Bit-identical to `ntt_scalar`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn ntt_avx2(r: &mut [i16; 256]) {
    use std::arch::x86_64::*;
    let qv = _mm256_set1_epi16(Q as i16);
    let qinvv = _mm256_set1_epi16(QINV as i16);
    let mut k = 1usize;
    let mut len = 128usize;
    // len >= 16: contiguous 16-wide butterflies, broadcast zeta.
    while len >= 16 {
        let mut start = 0usize;
        while start < 256 {
            let zeta = _mm256_set1_epi16(ZETAS[k]);
            k += 1;
            let mut j = start;
            while j < start + len {
                let aj = _mm256_loadu_si256(r.as_ptr().add(j) as *const __m256i);
                let ajl = _mm256_loadu_si256(r.as_ptr().add(j + len) as *const __m256i);
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
    // len = 8: one block per vector, 128-bit half-swap + blend.
    {
        let lo_mask = _mm256_set_epi64x(0, 0, -1, -1);
        let mut start = 0usize;
        while start < 256 {
            let zeta = _mm256_set1_epi16(ZETAS[k]);
            k += 1;
            let v = _mm256_loadu_si256(r.as_ptr().add(start) as *const __m256i);
            let hi = _mm256_permute2x128_si256(v, v, 0x01);
            let rlo = _mm256_mullo_epi16(zeta, hi);
            let rhi = _mm256_mulhi_epi16(zeta, hi);
            let tt = _mm256_mullo_epi16(rlo, qinvv);
            let tt = _mm256_mulhi_epi16(tt, qv);
            let t = _mm256_sub_epi16(rhi, tt);
            let add = _mm256_add_epi16(v, t);
            let sub = _mm256_sub_epi16(v, t);
            let sub_hi = _mm256_permute2x128_si256(sub, sub, 0x01);
            let out = _mm256_blendv_epi8(sub_hi, add, lo_mask);
            _mm256_storeu_si256(r.as_mut_ptr().add(start) as *mut __m256i, out);
            start += 16;
        }
        len >>= 1;
    }
    // len = 4: two blocks per vector, per-block zeta + 64-bit swap.
    {
        let mask4 = _mm256_set_epi64x(0, -1, 0, -1);
        let mut start = 0usize;
        while start < 256 {
            let z0 = ZETAS[k];
            let z1 = ZETAS[k + 1];
            k += 2;
            let zv = _mm256_set_epi16(z1, z1, z1, z1, z1, z1, z1, z1, z0, z0, z0, z0, z0, z0, z0, z0);
            let v = _mm256_loadu_si256(r.as_ptr().add(start) as *const __m256i);
            let vhi = _mm256_shuffle_epi32(v, 0x4E);
            let rlo = _mm256_mullo_epi16(zv, vhi);
            let rhi = _mm256_mulhi_epi16(zv, vhi);
            let tt = _mm256_mullo_epi16(rlo, qinvv);
            let tt = _mm256_mulhi_epi16(tt, qv);
            let t = _mm256_sub_epi16(rhi, tt);
            let add = _mm256_add_epi16(v, t);
            let sub = _mm256_sub_epi16(v, t);
            let sub_s = _mm256_shuffle_epi32(sub, 0x4E);
            let out = _mm256_blendv_epi8(sub_s, add, mask4);
            _mm256_storeu_si256(r.as_mut_ptr().add(start) as *mut __m256i, out);
            start += 16;
        }
        len >>= 1;
    }
    // len = 2: four blocks per vector, per-block zeta + 32-bit swap.
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
            let vhi = _mm256_shuffle_epi32(v, 0xB1);
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

/// Montgomery multiply 16 i16 lanes: `montgomery_reduce(a·b)` — bit-identical to the scalar `fqmul`
/// (low·qinv→t, then high − (t·q)high). The verified Kyber AVX2 reduction.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn mont_mul_x16(
    a: std::arch::x86_64::__m256i,
    b: std::arch::x86_64::__m256i,
) -> std::arch::x86_64::__m256i {
    use std::arch::x86_64::*;
    let q = _mm256_set1_epi16(Q as i16);
    let qinv = _mm256_set1_epi16(QINV as i16);
    let rlo = _mm256_mullo_epi16(b, a);
    let rhi = _mm256_mulhi_epi16(b, a);
    let tt = _mm256_mullo_epi16(rlo, qinv);
    let tt = _mm256_mulhi_epi16(tt, q);
    _mm256_sub_epi16(rhi, tt)
}

/// Barrett-reduce 16 i16 lanes, bit-identical to the scalar `barrett_reduce`. The scalar form
/// `t = (V·a + 2²⁵) >> 26` (V = ⌊2²⁶/q⌉ = 20159, rounded) equals `t = (mulhi(a,V) + 512) >> 10`
/// exactly: writing `V·a = hi·2¹⁶ + L` with `L ∈ [0,2¹⁶)`, the term `L/2²⁶ < 1/1024` can never
/// move the floor, so the low half drops out and the rounding collapses to a +512 before the >>10.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn barrett_x16(a: std::arch::x86_64::__m256i) -> std::arch::x86_64::__m256i {
    use std::arch::x86_64::*;
    const V: i16 = (((1 << 26) + Q / 2) / Q) as i16;
    let q = _mm256_set1_epi16(Q as i16);
    let hi = _mm256_mulhi_epi16(a, _mm256_set1_epi16(V));
    let t = _mm256_srai_epi16(_mm256_add_epi16(hi, _mm256_set1_epi16(512)), 10);
    _mm256_sub_epi16(a, _mm256_mullo_epi16(t, q))
}

/// AVX2 inverse NTT (`invntt_tomont`) — all 7 Gentleman-Sande levels vectorized 16-wide, then the
/// final `f`-scaling. Bit-identical to `invntt_scalar`. Mirrors `ntt_avx2`'s lane routing in reverse
/// level order: the GS butterfly writes `barrett(lo+hi)` to the low slot and `fqmul(ζ, hi−lo)` to the
/// high slot (both already in position — no post-swap, unlike the forward Cooley–Tukey blend).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn invntt_avx2(r: &mut [i16; 256]) {
    use std::arch::x86_64::*;
    const F: i16 = 1441;
    let mut k = 127usize;
    // len = 2: four blocks per vector, per-block zeta, 32-bit swap.
    {
        let mask2 = _mm256_set_epi32(0, -1, 0, -1, 0, -1, 0, -1);
        let mut start = 0usize;
        while start < 256 {
            let z0 = ZETAS[k];
            let z1 = ZETAS[k - 1];
            let z2 = ZETAS[k - 2];
            let z3 = ZETAS[k - 3];
            k -= 4;
            let zv = _mm256_set_epi16(z3, z3, z3, z3, z2, z2, z2, z2, z1, z1, z1, z1, z0, z0, z0, z0);
            let v = _mm256_loadu_si256(r.as_ptr().add(start) as *const __m256i);
            let vsh = _mm256_shuffle_epi32(v, 0xB1);
            let lopart = barrett_x16(_mm256_add_epi16(v, vsh));
            let hipart = mont_mul_x16(zv, _mm256_sub_epi16(v, vsh));
            let out = _mm256_blendv_epi8(hipart, lopart, mask2);
            _mm256_storeu_si256(r.as_mut_ptr().add(start) as *mut __m256i, out);
            start += 16;
        }
    }
    // len = 4: two blocks per vector, per-block zeta, 64-bit swap.
    {
        let mask4 = _mm256_set_epi64x(0, -1, 0, -1);
        let mut start = 0usize;
        while start < 256 {
            let z0 = ZETAS[k];
            let z1 = ZETAS[k - 1];
            k -= 2;
            let zv = _mm256_set_epi16(z1, z1, z1, z1, z1, z1, z1, z1, z0, z0, z0, z0, z0, z0, z0, z0);
            let v = _mm256_loadu_si256(r.as_ptr().add(start) as *const __m256i);
            let vsh = _mm256_shuffle_epi32(v, 0x4E);
            let lopart = barrett_x16(_mm256_add_epi16(v, vsh));
            let hipart = mont_mul_x16(zv, _mm256_sub_epi16(v, vsh));
            let out = _mm256_blendv_epi8(hipart, lopart, mask4);
            _mm256_storeu_si256(r.as_mut_ptr().add(start) as *mut __m256i, out);
            start += 16;
        }
    }
    // len = 8: one block per vector, 128-bit half-swap.
    {
        let lo_mask = _mm256_set_epi64x(0, 0, -1, -1);
        let mut start = 0usize;
        while start < 256 {
            let zeta = _mm256_set1_epi16(ZETAS[k]);
            k -= 1;
            let v = _mm256_loadu_si256(r.as_ptr().add(start) as *const __m256i);
            let vsh = _mm256_permute2x128_si256(v, v, 0x01);
            let lopart = barrett_x16(_mm256_add_epi16(v, vsh));
            let hipart = mont_mul_x16(zeta, _mm256_sub_epi16(v, vsh));
            let out = _mm256_blendv_epi8(hipart, lopart, lo_mask);
            _mm256_storeu_si256(r.as_mut_ptr().add(start) as *mut __m256i, out);
            start += 16;
        }
    }
    // len >= 16: contiguous 16-wide butterflies, broadcast zeta.
    let mut len = 16usize;
    while len <= 128 {
        let mut start = 0usize;
        while start < 256 {
            let zeta = _mm256_set1_epi16(ZETAS[k]);
            k = k.wrapping_sub(1);
            let mut j = start;
            while j < start + len {
                let lo = _mm256_loadu_si256(r.as_ptr().add(j) as *const __m256i);
                let hi = _mm256_loadu_si256(r.as_ptr().add(j + len) as *const __m256i);
                _mm256_storeu_si256(
                    r.as_mut_ptr().add(j) as *mut __m256i,
                    barrett_x16(_mm256_add_epi16(lo, hi)),
                );
                _mm256_storeu_si256(
                    r.as_mut_ptr().add(j + len) as *mut __m256i,
                    mont_mul_x16(zeta, _mm256_sub_epi16(hi, lo)),
                );
                j += 16;
            }
            start += 2 * len;
        }
        len <<= 1;
    }
    // Final f = mont²/128 scaling.
    let fv = _mm256_set1_epi16(F);
    let mut i = 0usize;
    while i < 256 {
        let v = _mm256_loadu_si256(r.as_ptr().add(i) as *const __m256i);
        _mm256_storeu_si256(r.as_mut_ptr().add(i) as *mut __m256i, mont_mul_x16(fv, v));
        i += 16;
    }
}

/// Core forward ML-KEM NTT of 256 coefficients (mod q = 3329): AVX2 when the CPU has it, else
/// scalar — same result. Coefficients are reduced into [0, q) at the boundary; output in [0, q).
fn mlkem_ntt_raw(input: &[i64]) -> Vec<i64> {
    assert_eq!(input.len(), 256, "mlkemNtt expects exactly 256 coefficients");
    // mlkem_ntt's contract is to reduce ANY i64 input (see mlkem_ntt_reduces_and_matches_scalar),
    // so the full rem_euclid stays here — unlike base_mul/inv_ntt/to_mont, which only ever see
    // already-reduced NTT-domain coefficients.
    let mut r = [0i16; 256];
    for (slot, &x) in r.iter_mut().zip(input) {
        *slot = x.rem_euclid(Q as i64) as i16;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            unsafe { ntt_avx2(&mut r) };
        } else {
            ntt_scalar(&mut r);
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    ntt_scalar(&mut r);
    r.iter().map(|&x| (x as i32).rem_euclid(Q) as i64).collect()
}

/// The compiled form of LOGOS `mlkemNtt(a)` — the runtime entry the generated Rust calls (and the
/// interpreter dispatches to). Takes/returns the LOGOS `Seq of Int` carrier.
pub fn mlkem_ntt(input: &[i64]) -> logicaffeine_data::LogosSeq<i64> {
    let out = mlkem_ntt_raw(input);
    logicaffeine_data::LogosSeq::from_vec(out)
}

/// `Word16`-native forward NTT — coefficients are carried as `Word16` (u16) in [0, q), so the
/// boundary is a plain reinterpret (`w.0 as i16`, exact since q < 2¹⁵), no rem_euclid and no
/// i64 round-trip. The experimental fast carrier for the Word16-representation crypto.
pub fn mlkem_ntt_w16(input: &[logicaffeine_base::Word16]) -> Vec<logicaffeine_base::Word16> {
    use logicaffeine_base::Word16;
    assert_eq!(input.len(), 256);
    let mut r = [0i16; 256];
    for (slot, w) in r.iter_mut().zip(input) {
        *slot = w.0 as i16;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            unsafe { ntt_avx2(&mut r) };
        } else {
            ntt_scalar(&mut r);
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    ntt_scalar(&mut r);
    r.iter().map(|&x| Word16((x as i32).rem_euclid(Q) as u16)).collect()
}

/// `Word16`-native inverse NTT, base multiply, and tomont — same zero-conversion carrier.
pub fn mlkem_inv_ntt_w16(input: &[logicaffeine_base::Word16]) -> Vec<logicaffeine_base::Word16> {
    use logicaffeine_base::Word16;
    let mut r = [0i16; 256];
    for (slot, w) in r.iter_mut().zip(input) {
        *slot = w.0 as i16;
    }
    inv_ntt_inplace(&mut r);
    r.iter().map(|&x| Word16((x as i32).rem_euclid(Q) as u16)).collect()
}

/// Inverse NTT in place: AVX2 when the CPU has it (bit-identical to the scalar reference), else scalar.
#[inline]
fn inv_ntt_inplace(r: &mut [i16; 256]) {
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            unsafe { invntt_avx2(r) };
            return;
        }
    }
    invntt_scalar(r);
}
pub fn mlkem_base_mul_w16(
    a: &[logicaffeine_base::Word16],
    b: &[logicaffeine_base::Word16],
) -> Vec<logicaffeine_base::Word16> {
    use logicaffeine_base::Word16;
    let to16 = |v: &[Word16]| -> [i16; 256] {
        let mut r = [0i16; 256];
        for (slot, w) in r.iter_mut().zip(v) {
            *slot = w.0 as i16;
        }
        r
    };
    let r = basemul_scalar(&to16(a), &to16(b));
    // basemul_scalar sums two Montgomery products (each ∈ (−q, q)) ⇒ result ∈ (−2q, 2q): Barrett into
    // (−q/2, q/2] then a conditional add into [0, q) — both branchless, no division.
    r.iter().map(|&x| Word16(cadd_q(barrett_reduce(x)))).collect()
}
pub fn mlkem_to_mont_w16(coeffs: &[logicaffeine_base::Word16]) -> Vec<logicaffeine_base::Word16> {
    use logicaffeine_base::Word16;
    const F: i32 = 1353;
    // montgomery_reduce returns (−q, q) ⇒ conditional add, no division.
    coeffs.iter().map(|w| Word16(cadd_q(montgomery_reduce(w.0 as i16 as i32 * F)))).collect()
}

/// Native entry for the LOGOS `Seq of Word16` carrier — wraps the raw Word16 NTT in a `LogosSeq`.
pub fn mlkem_ntt_w16_seq(
    input: &[logicaffeine_base::Word16],
) -> logicaffeine_data::LogosSeq<logicaffeine_base::Word16> {
    logicaffeine_data::LogosSeq::from_vec(mlkem_ntt_w16(input))
}

// ── Word16/Word8 carrier: the rest of the ML-KEM pipeline (zero i64 round-trip) ───────────────
use logicaffeine_base::{Word16, Word8};

/// CBD noise sampled to [0, q) Word16 (the reduced form the Word16 NTT consumes directly).
pub fn mlkem_cbd2_w16(buf: &[u8]) -> Vec<Word16> {
    cbd_eta2(buf).iter().map(|&c| Word16((c as i32).rem_euclid(Q) as u16)).collect()
}
pub fn mlkem_cbd3_w16(buf: &[u8]) -> Vec<Word16> {
    cbd_eta3(buf).iter().map(|&c| Word16((c as i32).rem_euclid(Q) as u16)).collect()
}
pub fn mlkem_compress_w16(coeffs: &[Word16], d: usize) -> Vec<Word16> {
    let mask = (1u64 << d) - 1;
    coeffs
        .iter()
        .map(|w| Word16(((((w.0 as u64) << d) + (Q as u64) / 2) / (Q as u64) & mask) as u16))
        .collect()
}
pub fn mlkem_decompress_w16(coeffs: &[Word16], d: usize) -> Vec<Word16> {
    let denom = 1u64 << d;
    coeffs
        .iter()
        .map(|w| Word16((((w.0 as u64) * (Q as u64) + denom / 2) >> d) as u16))
        .collect()
}
pub fn mlkem_byte_encode_w16(coeffs: &[Word16], d: usize) -> Vec<u8> {
    let mask = (1u64 << d) - 1;
    let mut out = Vec::with_capacity((coeffs.len() * d).div_ceil(8));
    let (mut acc, mut nbits) = (0u64, 0u32);
    for w in coeffs {
        acc |= (w.0 as u64 & mask) << nbits;
        nbits += d as u32;
        while nbits >= 8 {
            out.push((acc & 0xff) as u8);
            acc >>= 8;
            nbits -= 8;
        }
    }
    if nbits > 0 {
        out.push((acc & 0xff) as u8);
    }
    out
}
pub fn mlkem_byte_decode_w16(bytes: &[u8], d: usize) -> Vec<Word16> {
    let n = (bytes.len() * 8) / d;
    let mask = (1u64 << d) - 1;
    let mut out = Vec::with_capacity(n);
    let (mut acc, mut nbits, mut bi) = (0u64, 0u32, 0usize);
    for _ in 0..n {
        while nbits < d as u32 {
            acc |= (bytes[bi] as u64) << nbits;
            nbits += 8;
            bi += 1;
        }
        let val = (acc & mask) as u16;
        acc >>= d;
        nbits -= d as u32;
        out.push(Word16(if d == 12 { (val as i32 % Q) as u16 } else { val }));
    }
    out
}
/// FIPS-203 SampleNTT rejection over one 168-byte SHAKE128 block: parse 12-bit pairs, keep those
/// `< q`, append to `out` (capped at 256). 168 = 56·3 so triples never straddle a block boundary.
#[inline]
fn reject_sample_block(buf: &[u8; 168], out: &mut Vec<Word16>) {
    let q = Q as u32;
    let mut k = 0;
    while k + 3 <= 168 && out.len() < 256 {
        let (b0, b1, b2) = (buf[k] as u32, buf[k + 1] as u32, buf[k + 2] as u32);
        let d1 = b0 + 256 * (b1 % 16);
        let d2 = (b1 / 16) + 16 * b2;
        if d1 < q {
            out.push(Word16(d1 as u16));
        }
        if d2 < q && out.len() < 256 {
            out.push(Word16(d2 as u16));
        }
        k += 3;
    }
}

/// The 256-entry byte-compaction shuffle table for the vectorized rejection sampler: `table[m]` is the
/// `vpshufb` index that packs the accepted 16-bit lanes (accept bitmask `m` over 8 lanes) to the front
/// of a 128-bit register. Built once (idempotent).
#[cfg(target_arch = "x86_64")]
fn rej_compact_table() -> &'static [[u8; 16]; 256] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<[[u8; 16]; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [[0xFFu8; 16]; 256];
        for (m, entry) in t.iter_mut().enumerate() {
            let mut pos = 0usize;
            for lane in 0..8u8 {
                if (m >> lane) & 1 == 1 {
                    entry[pos * 2] = lane * 2;
                    entry[pos * 2 + 1] = lane * 2 + 1;
                    pos += 1;
                }
            }
        }
        t
    })
}

/// AVX2 vectorized ML-KEM matrix rejection sampler — **bit-identical** to [`reject_sample_block`],
/// several × faster: spread 24 bytes → 16 twelve-bit candidates per step (`vpshufb` + `vpsrlw` + blend),
/// compare all sixteen to q at once (`vpcmpgtw`), then compact the accepted lanes with a 256-entry
/// shuffle table (`vpshufb` + popcount advance). Appends accepted coefficients to `out` in the SAME
/// order as the scalar sampler and stops at 256. This is the ExpandA hot loop (~85% of sampleA cost).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn reject_sample_block_avx2(buf: &[u8; 168], out: &mut Vec<Word16>) {
    use std::arch::x86_64::*;
    let table = rej_compact_table();
    let qv = _mm256_set1_epi16(Q as i16);
    let maskv = _mm256_set1_epi16(0x0fff);
    // idx8: within each 128-bit half, 16-bit lane 2i ← bytes [3i, 3i+1], lane 2i+1 ← bytes [3i+1, 3i+2]
    // (low half over its bytes 0..11, high half over its bytes 4..15 = the group's bytes 12..23).
    let idx8 = _mm256_set_epi8(
        15, 14, 14, 13, 12, 11, 11, 10, 9, 8, 8, 7, 6, 5, 5, 4, //
        11, 10, 10, 9, 8, 7, 7, 6, 5, 4, 4, 3, 2, 1, 1, 0,
    );
    // Pad so every 32-byte load stays in bounds (168 = 7·24; the 7th load reads to byte 175).
    let mut padded = [0u8; 192];
    padded[..168].copy_from_slice(buf);

    let mut off = 0usize;
    while off + 24 <= 168 && out.len() < 256 {
        let raw = _mm256_loadu_si256(padded.as_ptr().add(off) as *const __m256i);
        let perm = _mm256_permute4x64_epi64(raw, 0x94); // lo=bytes0..15, hi=bytes8..23
        let mut f = _mm256_shuffle_epi8(perm, idx8);
        let g = _mm256_srli_epi16(f, 4);
        f = _mm256_blend_epi16(f, g, 0xAA); // odd 16-bit lanes take the >>4 form (d2)
        f = _mm256_and_si256(f, maskv);
        let good = _mm256_cmpgt_epi16(qv, f); // 0xFFFF where f < q (accepted)

        for half in 0..2 {
            if out.len() >= 256 {
                break;
            }
            let cand = if half == 0 { _mm256_castsi256_si128(f) } else { _mm256_extracti128_si256(f, 1) };
            let gd = if half == 0 { _mm256_castsi256_si128(good) } else { _mm256_extracti128_si256(good, 1) };
            let m = (_mm_movemask_epi8(_mm_packs_epi16(gd, gd)) & 0xff) as usize;
            let sh = _mm_loadu_si128(table[m].as_ptr() as *const __m128i);
            let packed = _mm_shuffle_epi8(cand, sh);
            let mut tmp = [0i16; 8];
            _mm_storeu_si128(tmp.as_mut_ptr() as *mut __m128i, packed);
            let n = (m.count_ones() as usize).min(256 - out.len());
            for &c in tmp.iter().take(n) {
                out.push(Word16(c as u16));
            }
        }
        off += 24;
    }
}

/// The padded 168-byte SHAKE128 absorb block for matrix entry `Â[r][c]`: `seed‖r‖c`, the `0x1f`
/// delimiter, and the `0x80` final bit — the single-block XOF input the rejection sampler streams.
#[inline]
fn sample_a_xof_block(seed: &[u8], r: u8, c: u8) -> [u8; 168] {
    let mut blk = [0u8; 168];
    blk[..32].copy_from_slice(&seed[..32]);
    blk[32] = r;
    blk[33] = c;
    blk[34] = 0x1f;
    blk[167] |= 0x80;
    blk
}

/// Matrix entry Â\[i\]\[j\] sampled to [0, q) Word16 — streaming SHAKE128 rejection (scalar reference).
pub fn mlkem_sample_a_w16(seed: &[u8], idx_i: i64, idx_j: i64) -> Vec<Word16> {
    let mut xof_in = [0u8; 34];
    xof_in[..32].copy_from_slice(&seed[..32]);
    xof_in[32] = idx_i.rem_euclid(256) as u8;
    xof_in[33] = idx_j.rem_euclid(256) as u8;
    let mut st = crate::keccak::shake128_absorb(&xof_in);
    let mut out: Vec<Word16> = Vec::with_capacity(256);
    let mut buf = [0u8; 168];
    loop {
        for i in 0..21 {
            buf[i * 8..i * 8 + 8].copy_from_slice(&st[i].to_le_bytes());
        }
        #[cfg(target_arch = "x86_64")]
        {
            if std::is_x86_feature_detected!("avx2") {
                unsafe { reject_sample_block_avx2(&buf, &mut out) };
            } else {
                reject_sample_block(&buf, &mut out);
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        reject_sample_block(&buf, &mut out);
        if out.len() >= 256 {
            out.truncate(256);
            return out;
        }
        crate::keccak::keccak_f1600(&mut st);
    }
}

/// Sample the full ML-KEM-768 3×3 matrix Â at once (9 × 256 Word16, entry Â\[r\]\[c\] at slot
/// `(r·3+c)·256`), batching four rejection-sampling streams per 4-way AVX2 SHAKE128 permutation.
/// Bit-identical to nine `mlkem_sample_a_w16` calls; falls back to scalar without AVX2. This is the
/// matrix-expansion keystone — sampleA is ~60% of keygen and is `×9` again in encaps.
pub fn mlkem_sample_matrix_w16(seed: &[u8]) -> Vec<Word16> {
    const ENTRIES: [(u8, u8); 9] =
        [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (2, 0), (2, 1), (2, 2)];
    let mut out = vec![Word16(0); 9 * 256];
    let mut place = |e: usize, poly: &[Word16]| {
        out[e * 256..e * 256 + 256].copy_from_slice(&poly[..256]);
    };

    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            let mut e = 0;
            while e + 4 <= ENTRIES.len() {
                let blocks: [[u8; 168]; 4] = std::array::from_fn(|l| {
                    sample_a_xof_block(seed, ENTRIES[e + l].0, ENTRIES[e + l].1)
                });
                let mut accs: [Vec<Word16>; 4] =
                    std::array::from_fn(|_| Vec::with_capacity(256));
                let mut st = unsafe { crate::keccak::shake128_x4_absorb_once(&blocks) };
                loop {
                    let outb = unsafe { crate::keccak::shake128_x4_squeeze_block(&st) };
                    let mut done = true;
                    for (l, acc) in accs.iter_mut().enumerate() {
                        if acc.len() < 256 {
                            unsafe { reject_sample_block_avx2(&outb[l], acc) };
                        }
                        if acc.len() < 256 {
                            done = false;
                        }
                    }
                    if done {
                        break;
                    }
                    unsafe { crate::keccak::keccak_f1600_x4(&mut st) };
                }
                for (l, acc) in accs.iter().enumerate() {
                    place(e + l, acc);
                }
                e += 4;
            }
            while e < ENTRIES.len() {
                let v = mlkem_sample_a_w16(seed, ENTRIES[e].0 as i64, ENTRIES[e].1 as i64);
                place(e, &v);
                e += 1;
            }
            return out;
        }
    }

    for (e, &(r, c)) in ENTRIES.iter().enumerate() {
        let v = mlkem_sample_a_w16(seed, r as i64, c as i64);
        place(e, &v);
    }
    out
}

#[inline]
fn barrett_reduce(a: i16) -> i16 {
    const V: i32 = ((1 << 26) + Q / 2) / Q;
    let t = (((V * a as i32) + (1 << 25)) >> 26) as i16;
    a.wrapping_sub(t.wrapping_mul(Q as i16))
}

/// Scalar inverse NTT (Kyber reference, `invntt_tomont`), in place — Gentleman-Sande butterflies
/// with Barrett reduction, then the `f = mont²/128` final scaling. invntt(ntt(p)) = p·MONT mod q.
fn invntt_scalar(r: &mut [i16; 256]) {
    const F: i16 = 1441;
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
        len <<= 1;
    }
    for x in r.iter_mut() {
        *x = fqmul(*x, F);
    }
}

fn mlkem_inv_ntt_raw(input: &[i64]) -> Vec<i64> {
    assert_eq!(input.len(), 256, "mlkemInvNtt expects exactly 256 coefficients");
    // Inputs are NTT-domain coefficients in [0, q) — the truncating `as i16` cast is exact.
    let mut r = [0i16; 256];
    for (slot, &x) in r.iter_mut().zip(input) {
        debug_assert!((0..Q as i64).contains(&x), "inv_ntt input out of [0,q)");
        *slot = x as i16;
    }
    inv_ntt_inplace(&mut r);
    r.iter().map(|&x| (x as i32).rem_euclid(Q) as i64).collect()
}

/// The compiled form of LOGOS `mlkemInvNtt(a)` — the inverse ML-KEM NTT (`tomont`), so
/// `mlkemInvNtt(mlkemNtt(p)) = p·2285 mod q`. Together with `mlkemNtt` and a base-multiply this is
/// the polynomial-multiplication primitive ML-KEM is built on.
pub fn mlkem_inv_ntt(input: &[i64]) -> logicaffeine_data::LogosSeq<i64> {
    let out = mlkem_inv_ntt_raw(input);
    logicaffeine_data::LogosSeq::from_vec(out)
}

/// One degree-1 base multiply mod (X² − ζ): `r0 = a0·b0 + ζ·a1·b1`, `r1 = a0·b1 + a1·b0`. This is
/// the gauss/Karatsuba bilinear form the kernel certifies (`field_algebra::gauss_three_multiply`).
#[inline]
fn bmul(a: &[i16], b: &[i16], zeta: i16) -> (i16, i16) {
    let r0 = fqmul(fqmul(a[1], b[1]), zeta).wrapping_add(fqmul(a[0], b[0]));
    let r1 = fqmul(a[0], b[1]).wrapping_add(fqmul(a[1], b[0]));
    (r0, r1)
}

/// Scalar pointwise base-multiply of two NTT-domain polynomials (Kyber `poly_basemul`). The 128
/// degree-1 pairs use ζ = ±zetas[64+i].
fn basemul_scalar(a: &[i16; 256], b: &[i16; 256]) -> [i16; 256] {
    let mut r = [0i16; 256];
    for i in 0..64 {
        let zeta = ZETAS[64 + i];
        let (c0, c1) = bmul(&a[4 * i..4 * i + 2], &b[4 * i..4 * i + 2], zeta);
        r[4 * i] = c0;
        r[4 * i + 1] = c1;
        let (d0, d1) = bmul(&a[4 * i + 2..4 * i + 4], &b[4 * i + 2..4 * i + 4], zeta.wrapping_neg());
        r[4 * i + 2] = d0;
        r[4 * i + 3] = d1;
    }
    r
}

fn mlkem_base_mul_raw(a: &[i64], b: &[i64]) -> Vec<i64> {
    assert_eq!(a.len(), 256);
    assert_eq!(b.len(), 256);
    // Inputs are NTT-domain coefficients already reduced to [0, q) (q = 3329 < 2¹⁵), so the
    // truncating `as i16` cast is exact — no per-element rem_euclid needed on the way in.
    let to16 = |v: &[i64]| -> [i16; 256] {
        let mut r = [0i16; 256];
        for (slot, &x) in r.iter_mut().zip(v) {
            debug_assert!((0..Q as i64).contains(&x), "base_mul input out of [0,q)");
            *slot = x as i16;
        }
        r
    };
    let r = basemul_scalar(&to16(a), &to16(b));
    r.iter().map(|&x| (x as i32).rem_euclid(Q) as i64).collect()
}

/// The compiled form of LOGOS `mlkemBaseMul(a, b)` — pointwise multiply of two NTT-domain
/// polynomials. `mlkemInvNtt(mlkemBaseMul(mlkemNtt(a), mlkemNtt(b)))` is the ML-KEM polynomial
/// product a·b in ℤ_q\[X\]/(X²⁵⁶+1).
pub fn mlkem_base_mul(
    a: &[i64],
    b: &[i64],
) -> logicaffeine_data::LogosSeq<i64> {
    let out = mlkem_base_mul_raw(a, b);
    logicaffeine_data::LogosSeq::from_vec(out)
}

/// poly_tomont (Kyber): bring coefficients into the Montgomery domain by multiplying by R = 2¹⁶
/// (F = 2³² mod q = 1353, one montgomery_reduce). After accumulating `Σ_j basemul(Â[i][j], ŝ[j])`
/// — which lands in the R⁻¹ domain — a single tomont returns t̂ to normal form for ByteEncode12.
fn mlkem_to_mont_raw(coeffs: &[i64]) -> Vec<i64> {
    const F: i32 = 1353; // 2³² mod q
    coeffs
        .iter()
        .map(|&x| {
            debug_assert!((0..Q as i64).contains(&x), "to_mont input out of [0,q)");
            montgomery_reduce(x as i32 * F) as i32 % Q
        })
        .map(|x| x.rem_euclid(Q) as i64)
        .collect()
}

/// The compiled form of LOGOS `toMont(coeffs)` — multiply a polynomial into the Montgomery domain.
pub fn mlkem_to_mont(coeffs: &[i64]) -> logicaffeine_data::LogosSeq<i64> {
    logicaffeine_data::LogosSeq::from_vec(mlkem_to_mont_raw(coeffs))
}

// ── CBD sampling: centered binomial noise, the Kyber bit-trick (η = 2, 3) ─────────────────────

/// CBD_2 (ML-KEM η=2): 128 bytes → 256 coefficients in [−2, 2]. Each coefficient is
/// popcount(2 bits) − popcount(2 bits); the `& 0x55..` trick sums the bit-pairs in parallel.
fn cbd_eta2(buf: &[u8]) -> [i16; 256] {
    assert_eq!(buf.len(), 128, "CBD_2 needs 64·η = 128 bytes");
    let mut r = [0i16; 256];
    for i in 0..32 {
        let t = u32::from_le_bytes(buf[4 * i..4 * i + 4].try_into().unwrap());
        let mut d = t & 0x5555_5555;
        d += (t >> 1) & 0x5555_5555;
        for j in 0..8 {
            let a = ((d >> (4 * j)) & 0x3) as i16;
            let b = ((d >> (4 * j + 2)) & 0x3) as i16;
            r[8 * i + j] = a - b;
        }
    }
    r
}

/// CBD_3 (ML-KEM η=3 / ML-KEM-512's `s`,`e`): 192 bytes → 256 coefficients in [−3, 3].
fn cbd_eta3(buf: &[u8]) -> [i16; 256] {
    assert_eq!(buf.len(), 192, "CBD_3 needs 64·η = 192 bytes");
    let mut r = [0i16; 256];
    for i in 0..64 {
        let t =
            buf[3 * i] as u32 | (buf[3 * i + 1] as u32) << 8 | (buf[3 * i + 2] as u32) << 16;
        let mut d = t & 0x0024_9249;
        d += (t >> 1) & 0x0024_9249;
        d += (t >> 2) & 0x0024_9249;
        for j in 0..4 {
            let a = ((d >> (6 * j)) & 0x7) as i16;
            let b = ((d >> (6 * j + 3)) & 0x7) as i16;
            r[4 * i + j] = a - b;
        }
    }
    r
}

fn mlkem_cbd_raw(buf: &[i64], eta: usize) -> Vec<i64> {
    let bytes: Vec<u8> = buf.iter().map(|&x| x.rem_euclid(256) as u8).collect();
    let r = match eta {
        2 => cbd_eta2(&bytes),
        3 => cbd_eta3(&bytes),
        _ => panic!("ML-KEM CBD supports η ∈ {{2, 3}}"),
    };
    r.iter().map(|&x| x as i64).collect()
}

/// The compiled form of LOGOS `cbd2(buf)` — sample a noise polynomial (η=2) from 128 bytes of
/// (SHAKE) randomness; 256 coefficients in [−2, 2].
pub fn mlkem_cbd2(buf: &[i64]) -> logicaffeine_data::LogosSeq<i64> {
    logicaffeine_data::LogosSeq::from_vec(mlkem_cbd_raw(buf, 2))
}
/// The compiled form of LOGOS `cbd3(buf)` — η=3 noise from 192 bytes; coefficients in [−3, 3].
pub fn mlkem_cbd3(buf: &[i64]) -> logicaffeine_data::LogosSeq<i64> {
    logicaffeine_data::LogosSeq::from_vec(mlkem_cbd_raw(buf, 3))
}

// ── Serialization: Compress/Decompress + ByteEncode/ByteDecode (FIPS 203 §4.2.1) ──────────────

/// Compress_d (FIPS 203): map a coefficient in [0, q) into d bits, `round(2^d·x / q) mod 2^d`,
/// rounding half up exactly as the Kyber reference (`(x·2^d + ⌊q/2⌋) / q`).
fn mlkem_compress_raw(coeffs: &[i64], d: usize) -> Vec<i64> {
    let mask = (1u64 << d) - 1;
    coeffs
        .iter()
        .map(|&x| {
            let x = x.rem_euclid(Q as i64) as u64;
            (((x << d) + (Q as u64) / 2) / (Q as u64) & mask) as i64
        })
        .collect()
}

/// Decompress_d (FIPS 203): map d bits back toward [0, q), `round(q·y / 2^d)` (round half up).
fn mlkem_decompress_raw(coeffs: &[i64], d: usize) -> Vec<i64> {
    let denom = 1u64 << d;
    coeffs
        .iter()
        .map(|&y| (((y as u64) * (Q as u64) + denom / 2) >> d) as i64)
        .collect()
}

/// ByteEncode_d (FIPS 203 Alg. 5): pack the low d bits of each coefficient, least-significant bit
/// first, into a byte string of length ⌈(len·d)/8⌉ (= 32·d for a 256-coefficient polynomial). A
/// 64-bit bit-accumulator emits whole bytes (d ≤ 12, so at most 19 staged bits) — no per-bit loop.
fn mlkem_byte_encode_raw(coeffs: &[i64], d: usize) -> Vec<i64> {
    let mask = (1u64 << d) - 1;
    let mut out = Vec::with_capacity((coeffs.len() * d).div_ceil(8));
    let mut acc: u64 = 0;
    let mut nbits = 0u32;
    for &c in coeffs {
        acc |= (c.rem_euclid(1i64 << d) as u64 & mask) << nbits;
        nbits += d as u32;
        while nbits >= 8 {
            out.push((acc & 0xff) as i64);
            acc >>= 8;
            nbits -= 8;
        }
    }
    if nbits > 0 {
        out.push((acc & 0xff) as i64);
    }
    out
}

/// ByteDecode_d (FIPS 203 Alg. 6): the inverse of ByteEncode; for d = 12 the value is reduced
/// mod q (12-bit fields can exceed q), otherwise it is a plain d-bit integer. Byte-batched: bytes
/// feed a 64-bit accumulator from which d-bit values are sliced — no per-bit loop.
fn mlkem_byte_decode_raw(bytes: &[i64], d: usize) -> Vec<i64> {
    let n = (bytes.len() * 8) / d;
    let mask = (1u64 << d) - 1;
    let mut out = Vec::with_capacity(n);
    let mut acc: u64 = 0;
    let mut nbits = 0u32;
    let mut bi = 0usize;
    for _ in 0..n {
        while nbits < d as u32 {
            acc |= (bytes[bi].rem_euclid(256) as u64) << nbits;
            nbits += 8;
            bi += 1;
        }
        let val = (acc & mask) as i64;
        acc >>= d;
        nbits -= d as u32;
        out.push(if d == 12 { val.rem_euclid(Q as i64) } else { val });
    }
    out
}

/// The compiled form of LOGOS `compress(coeffs, d)` — element-wise Compress_d.
pub fn mlkem_compress(
    coeffs: &[i64],
    d: i64,
) -> logicaffeine_data::LogosSeq<i64> {
    logicaffeine_data::LogosSeq::from_vec(mlkem_compress_raw(coeffs, d as usize))
}
/// The compiled form of LOGOS `decompress(coeffs, d)` — element-wise Decompress_d.
pub fn mlkem_decompress(
    coeffs: &[i64],
    d: i64,
) -> logicaffeine_data::LogosSeq<i64> {
    logicaffeine_data::LogosSeq::from_vec(mlkem_decompress_raw(coeffs, d as usize))
}
/// The compiled form of LOGOS `byteEncode(coeffs, d)` — pack coefficients to bytes.
pub fn mlkem_byte_encode(
    coeffs: &[i64],
    d: i64,
) -> logicaffeine_data::LogosSeq<i64> {
    logicaffeine_data::LogosSeq::from_vec(mlkem_byte_encode_raw(coeffs, d as usize))
}
/// The compiled form of LOGOS `byteDecode(bytes, d)` — unpack bytes to coefficients.
pub fn mlkem_byte_decode(
    bytes: &[i64],
    d: i64,
) -> logicaffeine_data::LogosSeq<i64> {
    logicaffeine_data::LogosSeq::from_vec(mlkem_byte_decode_raw(bytes, d as usize))
}

// ── Uniform sampling in the NTT domain: SampleNTT + matrix-entry expansion (FIPS 203 §4.2.2) ──

/// Try to rejection-sample 256 coefficients (< q) from a byte stream of XOF output, three bytes
/// yielding two 12-bit candidates. Returns `None` if the stream is exhausted first (the caller
/// then squeezes more — SHAKE output is a stream, so a longer squeeze extends this one's prefix).
fn try_sample_ntt(stream: &[u8], want: usize) -> Option<Vec<i64>> {
    let q = Q as u32;
    let mut a = Vec::with_capacity(want);
    let mut i = 0usize;
    while a.len() < want {
        if i + 3 > stream.len() {
            return None;
        }
        let (b0, b1, b2) = (stream[i] as u32, stream[i + 1] as u32, stream[i + 2] as u32);
        let d1 = b0 + 256 * (b1 % 16);
        let d2 = (b1 / 16) + 16 * b2;
        if d1 < q {
            a.push(d1 as i64);
        }
        if d2 < q && a.len() < want {
            a.push(d2 as i64);
        }
        i += 3;
    }
    Some(a)
}

fn mlkem_sample_ntt_raw(stream: &[i64]) -> Vec<i64> {
    let bytes: Vec<u8> = stream.iter().map(|&x| x.rem_euclid(256) as u8).collect();
    try_sample_ntt(&bytes, 256).expect("SampleNTT: XOF stream exhausted before 256 coefficients")
}

/// Expand one NTT-domain matrix entry Â[i][j] (FIPS 203 §5.1): XOF = SHAKE128(ρ ‖ i ‖ j), then
/// SampleNTT. Squeezes 168-byte SHAKE128 blocks until 256 coefficients are sampled — never
/// truncates (the rejection rate makes one extra block astronomically rare, but the loop is exact).
fn mlkem_sample_a_raw(seed: &[u8], idx_i: i64, idx_j: i64) -> Vec<i64> {
    // Streaming SHAKE128 rejection sampling: absorb ρ‖i‖j once, then squeeze 168-byte blocks and
    // reject directly from each block (168 = 56·3, so the 3-byte candidate triples never straddle a
    // block boundary — bit-identical to sampling the concatenated stream). No heap stream, no
    // re-squeeze-from-scratch.
    let mut xof_in = [0u8; 34];
    xof_in[..32].copy_from_slice(&seed[..32]);
    xof_in[32] = idx_i.rem_euclid(256) as u8;
    xof_in[33] = idx_j.rem_euclid(256) as u8;
    let mut st = crate::keccak::shake128_absorb(&xof_in);

    let q = Q as u32;
    let mut out = Vec::with_capacity(256);
    let mut buf = [0u8; 168];
    loop {
        for i in 0..21 {
            buf[i * 8..i * 8 + 8].copy_from_slice(&st[i].to_le_bytes());
        }
        let mut k = 0;
        while k + 3 <= 168 && out.len() < 256 {
            let (b0, b1, b2) = (buf[k] as u32, buf[k + 1] as u32, buf[k + 2] as u32);
            let d1 = b0 + 256 * (b1 % 16);
            let d2 = (b1 / 16) + 16 * b2;
            if d1 < q {
                out.push(d1 as i64);
            }
            if d2 < q && out.len() < 256 {
                out.push(d2 as i64);
            }
            k += 3;
        }
        if out.len() >= 256 {
            return out;
        }
        crate::keccak::keccak_f1600(&mut st);
    }
}

/// The compiled form of LOGOS `sampleNtt(stream)` — rejection-sample a uniform NTT-domain
/// polynomial directly from a caller-provided byte stream.
pub fn mlkem_sample_ntt(stream: &[i64]) -> logicaffeine_data::LogosSeq<i64> {
    logicaffeine_data::LogosSeq::from_vec(mlkem_sample_ntt_raw(stream))
}

/// The compiled form of LOGOS `sampleA(seed, i, j)` — expand matrix entry Â\[i\]\[j\] from the
/// 32-byte seed ρ via SHAKE128, with the XOF and rejection handled in one verified step.
pub fn mlkem_sample_a(
    seed: &[i64],
    idx_i: i64,
    idx_j: i64,
) -> logicaffeine_data::LogosSeq<i64> {
    let seed_bytes: Vec<u8> = seed.iter().map(|&x| x.rem_euclid(256) as u8).collect();
    logicaffeine_data::LogosSeq::from_vec(mlkem_sample_a_raw(&seed_bytes, idx_i, idx_j))
}

// ── Word16/Word8 LOGOS native entries (LogosSeq carrier) + native modular arithmetic ──────────
// The full Word16-representation ML-KEM rides on these: the SHIPPED `crypto.lg` calls them, the
// coefficient hot loop never touches i64, and the byte edges convert cheaply (≤1184 B).

type Seq16 = logicaffeine_data::LogosSeq<Word16>;
type Seq8 = logicaffeine_data::LogosSeq<Word8>;
#[inline]
fn w8_to_u8(s: &[Word8]) -> Vec<u8> {
    s.iter().map(|w| w.0).collect()
}
#[inline]
fn u8_to_seq8(v: Vec<u8>) -> Seq8 {
    logicaffeine_data::LogosSeq::from_vec(v.into_iter().map(Word8).collect())
}

pub fn mlkem_inv_ntt_w16_seq(input: &[Word16]) -> Seq16 {
    logicaffeine_data::LogosSeq::from_vec(mlkem_inv_ntt_w16(input))
}
pub fn mlkem_base_mul_w16_seq(a: &[Word16], b: &[Word16]) -> Seq16 {
    logicaffeine_data::LogosSeq::from_vec(mlkem_base_mul_w16(a, b))
}
pub fn mlkem_to_mont_w16_seq(c: &[Word16]) -> Seq16 {
    logicaffeine_data::LogosSeq::from_vec(mlkem_to_mont_w16(c))
}
pub fn mlkem_compress_w16_seq(c: &[Word16], d: i64) -> Seq16 {
    logicaffeine_data::LogosSeq::from_vec(mlkem_compress_w16(c, d as usize))
}
pub fn mlkem_decompress_w16_seq(c: &[Word16], d: i64) -> Seq16 {
    logicaffeine_data::LogosSeq::from_vec(mlkem_decompress_w16(c, d as usize))
}
pub fn mlkem_byte_encode_w16_seq(c: &[Word16], d: i64) -> Seq8 {
    u8_to_seq8(mlkem_byte_encode_w16(c, d as usize))
}
pub fn mlkem_byte_decode_w16_seq(b: &[Word8], d: i64) -> Seq16 {
    logicaffeine_data::LogosSeq::from_vec(mlkem_byte_decode_w16(&w8_to_u8(b), d as usize))
}
pub fn mlkem_cbd2_w16_seq(buf: &[Word8]) -> Seq16 {
    logicaffeine_data::LogosSeq::from_vec(mlkem_cbd2_w16(&w8_to_u8(buf)))
}
pub fn mlkem_cbd3_w16_seq(buf: &[Word8]) -> Seq16 {
    logicaffeine_data::LogosSeq::from_vec(mlkem_cbd3_w16(&w8_to_u8(buf)))
}
pub fn mlkem_sample_a_w16_seq(seed: &[Word8], i: i64, j: i64) -> Seq16 {
    logicaffeine_data::LogosSeq::from_vec(mlkem_sample_a_w16(&w8_to_u8(seed), i, j))
}
pub fn sha3_256_w8_seq(input: &[Word8]) -> Seq8 {
    u8_to_seq8(crate::keccak::sha3_256_bytes(&w8_to_u8(input)).to_vec())
}
pub fn sha3_512_w8_seq(input: &[Word8]) -> Seq8 {
    u8_to_seq8(crate::keccak::sha3_512_bytes(&w8_to_u8(input)).to_vec())
}
pub fn shake128_w8_seq(input: &[Word8], outlen: i64) -> Seq8 {
    u8_to_seq8(crate::keccak::shake128_bytes(&w8_to_u8(input), outlen.max(0) as usize))
}
pub fn shake256_w8_seq(input: &[Word8], outlen: i64) -> Seq8 {
    u8_to_seq8(crate::keccak::shake256_bytes(&w8_to_u8(input), outlen.max(0) as usize))
}

/// `(a + b) mod q`, element-wise on Word16 [0, q) — the native modular-add the Logos matrix-vector
/// loop calls (so the orchestration never does Word16 arithmetic, only structure).
pub fn mlkem_add_mod_q_w16(a: &[Word16], b: &[Word16]) -> Seq16 {
    // Operands ∈ [0, q) ⇒ a + b ∈ [0, 2q): one conditional subtract (auto-vectorizes 16-wide), no
    // division — this is the Logos ML-KEM matrix-vector loop's inner reduction (called ~9×/keygen).
    let q = Q as u16;
    logicaffeine_data::LogosSeq::from_vec(
        a.iter()
            .zip(b)
            .map(|(x, y)| {
                let s = x.0 + y.0;
                Word16(if s >= q { s - q } else { s })
            })
            .collect(),
    )
}
/// `(a − b) mod q`, element-wise on Word16 [0, q).
pub fn mlkem_sub_mod_q_w16(a: &[Word16], b: &[Word16]) -> Seq16 {
    // Operands ∈ [0, q) ⇒ a − b ∈ (−q, q): one conditional add, no division.
    let q = Q as i32;
    logicaffeine_data::LogosSeq::from_vec(
        a.iter()
            .zip(b)
            .map(|(x, y)| {
                let d = x.0 as i32 - y.0 as i32;
                Word16((if d < 0 { d + q } else { d }) as u16)
            })
            .collect(),
    )
}
/// A run of `n` zero coefficients (Word16).
pub fn mlkem_zeros_w16(n: i64) -> Seq16 {
    logicaffeine_data::LogosSeq::from_vec(vec![Word16(0); n.max(0) as usize])
}

// Int↔Word16 bridges at the byte edges: ML-KEM byte buffers stay LOGOS `Seq of Int` (so the AOT
// gates / hashing are unchanged), only the COEFFICIENT hot loop is Word16. Cheap (≤1184 bytes).
pub fn mlkem_cbd2_w16_from_int(buf: &[i64]) -> Seq16 {
    let bytes: Vec<u8> = buf.iter().map(|&x| x.rem_euclid(256) as u8).collect();
    logicaffeine_data::LogosSeq::from_vec(mlkem_cbd2_w16(&bytes))
}
pub fn mlkem_byte_encode_w16_to_int(c: &[Word16], d: i64) -> logicaffeine_data::LogosSeq<i64> {
    logicaffeine_data::LogosSeq::from_vec(
        mlkem_byte_encode_w16(c, d as usize).into_iter().map(|b| b as i64).collect(),
    )
}
pub fn mlkem_byte_decode_w16_from_int(b: &[i64], d: i64) -> Seq16 {
    let bytes: Vec<u8> = b.iter().map(|&x| x.rem_euclid(256) as u8).collect();
    logicaffeine_data::LogosSeq::from_vec(mlkem_byte_decode_w16(&bytes, d as usize))
}
pub fn mlkem_sample_a_w16_from_int(seed: &[i64], i: i64, j: i64) -> Seq16 {
    let bytes: Vec<u8> = seed.iter().map(|&x| x.rem_euclid(256) as u8).collect();
    logicaffeine_data::LogosSeq::from_vec(mlkem_sample_a_w16(&bytes, i, j))
}
/// Native entry for the full 3×3 matrix Â (9 × 256 Word16, entry Â\[r\]\[c\] at slot `(r·3+c)·256`),
/// 4-way AVX2 SHAKE128 batched — the shipped-Logos `mlkemSampleMatrixW16(rho)`.
pub fn mlkem_sample_matrix_w16_from_int(seed: &[i64]) -> Seq16 {
    let bytes: Vec<u8> = seed.iter().map(|&x| x.rem_euclid(256) as u8).collect();
    logicaffeine_data::LogosSeq::from_vec(mlkem_sample_matrix_w16(&bytes))
}

#[cfg(test)]
mod tests {
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
                1 => buf.fill(0),                                            // all d=0 (accept)
                2 => buf.fill(0xff),                                         // all ≥ q (reject)
                _ => buf.iter_mut().enumerate().for_each(|(i, b)| *b = if i % 3 == 1 { 0x0d } else { (next() & 0xff) as u8 }),
            }
            let mut want: Vec<Word16> = Vec::with_capacity(256);
            reject_sample_block(&buf, &mut want);
            let mut got: Vec<Word16> = Vec::with_capacity(256);
            unsafe { reject_sample_block_avx2(&buf, &mut got) };
            assert_eq!(got, want, "AVX2 rejection sampler must be bit-identical to scalar (iter {iter})");

            // Also validate the partial-fill path (out pre-loaded near the 256 cap).
            for pre in [0usize, 100, 250, 255] {
                let mut w = vec![Word16(1); pre];
                let mut g = vec![Word16(1); pre];
                reject_sample_block(&buf, &mut w);
                unsafe { reject_sample_block_avx2(&buf, &mut g) };
                assert_eq!(g, w, "AVX2 sampler must match scalar with {pre} pre-filled (iter {iter})");
            }
        }
    }

    #[test]
    fn word16_native_layer_is_correct() {
        let q = Q as u32;
        let a: Vec<Word16> = (0..256).map(|i| Word16((i * 13 % 3329) as u16)).collect();
        let b: Vec<Word16> = (0..256).map(|i| Word16((i * 29 % 3329) as u16)).collect();

        // modular arithmetic
        let add = mlkem_add_mod_q_w16(&a, &b);
        let sub = mlkem_sub_mod_q_w16(&a, &b);
        for i in 0..256 {
            assert_eq!(add.borrow()[i].0 as u32, (a[i].0 as u32 + b[i].0 as u32) % q);
            assert_eq!(sub.borrow()[i].0 as i32, (a[i].0 as i32 - b[i].0 as i32).rem_euclid(Q));
        }
        assert_eq!(mlkem_zeros_w16(256).borrow().len(), 256);
        assert!(mlkem_zeros_w16(256).borrow().iter().all(|w| w.0 == 0));

        // ByteEncode∘ByteDecode round-trip (d=12) through the Word16/Word8 entries
        let enc = mlkem_byte_encode_w16_seq(&a, 12);
        let dec = mlkem_byte_decode_w16_seq(&enc.borrow(), 12);
        assert_eq!(dec.borrow().as_slice(), a.as_slice(), "byteDecode∘byteEncode = id (Word16)");

        // cbd / sampleA Word16 entries match the i64 kernels (reduced into [0,q))
        let buf: Vec<Word8> = (0..128).map(|i| Word8((i * 7) as u8)).collect();
        let cbd_i64: Vec<i64> = mlkem_cbd2(&buf.iter().map(|w| w.0 as i64).collect::<Vec<_>>()).to_vec();
        let cbd_w16: Vec<i64> =
            mlkem_cbd2_w16_seq(&buf).borrow().iter().map(|w| (w.0 as i64).rem_euclid(Q as i64)).collect();
        let cbd_i64_red: Vec<i64> = cbd_i64.iter().map(|&c| c.rem_euclid(Q as i64)).collect();
        assert_eq!(cbd_w16, cbd_i64_red, "cbd2 Word16 == reduced i64 cbd2");

        let seed: Vec<Word8> = (0..32).map(|i| Word8(i as u8)).collect();
        let sa_i64: Vec<i64> = mlkem_sample_a(&seed.iter().map(|w| w.0 as i64).collect::<Vec<_>>(), 1, 2).to_vec();
        let sa_w16: Vec<i64> = mlkem_sample_a_w16_seq(&seed, 1, 2).borrow().iter().map(|w| w.0 as i64).collect();
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
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
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
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (s >> 56) as u8
        };
        let buf2: Vec<u8> = (0..128).map(|_| rb()).collect();
        assert_eq!(cbd_eta2(&buf2), cbd_ref(&buf2, 2), "fast CBD_2 must equal the bit-by-bit definition");
        assert!(cbd_eta2(&buf2).iter().all(|&c| (-2..=2).contains(&c)), "CBD_2 ∈ [−2, 2]");

        let buf3: Vec<u8> = (0..192).map(|_| rb()).collect();
        assert_eq!(cbd_eta3(&buf3), cbd_ref(&buf3, 3), "fast CBD_3 must equal the bit-by-bit definition");
        assert!(cbd_eta3(&buf3).iter().all(|&c| (-3..=3).contains(&c)), "CBD_3 ∈ [−3, 3]");
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
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (s >> 56) as u8
        };
        let stream: Vec<u8> = (0..2048).map(|_| rb()).collect();
        let stream_i: Vec<i64> = stream.iter().map(|&b| b as i64).collect();

        let got = mlkem_sample_ntt_raw(&stream_i);
        assert_eq!(got.len(), 256, "SampleNTT yields exactly 256 coefficients");
        assert!(got.iter().all(|&c| (0..Q as i64).contains(&c)), "every coefficient is in [0, q)");
        assert_eq!(got, sample_ntt_reference(&stream), "kernel SampleNTT must match the reference");

        // Hand check: bytes 0x01,0x20,0x03 ⇒ d1 = 1 + 256·0 = 1, d2 = 2 + 16·3 = 50.
        let hand = [0x01i64, 0x20, 0x03].iter().chain(std::iter::repeat(&0)).take(2048).copied().collect::<Vec<_>>();
        let hg = mlkem_sample_ntt_raw(&hand);
        assert_eq!(hg[0], 1, "first sampled coefficient");
        assert_eq!(hg[1], 50, "second sampled coefficient");
    }

    #[test]
    fn sample_a_is_deterministic_and_consumes_its_own_xof() {
        let seed: Vec<u8> = (0..32u8).collect();
        let a1 = mlkem_sample_a_raw(&seed, 1, 2);
        let a2 = mlkem_sample_a_raw(&seed, 1, 2);
        assert_eq!(a1, a2, "SampleA is a deterministic function of (ρ, i, j)");
        assert_ne!(a1, mlkem_sample_a_raw(&seed, 2, 1), "index order matters (Â is not symmetric)");
        assert_eq!(a1.len(), 256);
        assert!(a1.iter().all(|&c| (0..Q as i64).contains(&c)));

        // SampleA must equal SampleNTT over its own SHAKE128(ρ‖i‖j) stream.
        let mut xof_in = seed.clone();
        xof_in.push(1);
        xof_in.push(2);
        let stream = crate::keccak::shake128_bytes(&xof_in, 168 * 6);
        assert_eq!(a1, try_sample_ntt(&stream, 256).unwrap(), "SampleA = SampleNTT∘XOF");
    }

    #[test]
    fn byte_encode_decode_round_trips_losslessly() {
        // For every d ∈ {1,4,5,10,11,12} a polynomial of d-bit coefficients survives the
        // pack/unpack cycle exactly, and the packed length is 32·d bytes.
        let mut s = 0x1234_5678u64;
        let mut rc = |bound: i64| -> i64 {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (s >> 40) as i64 % bound
        };
        for &d in &[1usize, 4, 5, 10, 11, 12] {
            let modulus = if d == 12 { Q as i64 } else { 1i64 << d };
            let coeffs: Vec<i64> = (0..256).map(|_| rc(modulus)).collect();
            let bytes = mlkem_byte_encode_raw(&coeffs, d);
            assert_eq!(bytes.len(), 32 * d, "ByteEncode_{d} length must be 32·d");
            assert!(bytes.iter().all(|&b| (0..256).contains(&b)), "encoded values are bytes");
            let back = mlkem_byte_decode_raw(&bytes, d);
            assert_eq!(back, coeffs, "ByteDecode_{d}∘ByteEncode_{d} must be the identity");
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
                assert!((0..(1i64 << d)).contains(&c), "Compress_{d}({x}) must be a d-bit value");
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
        let k0 = (0..256).find(|&k| conv[k] != 0).expect("nonzero convolution");
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
        assert!(got.iter().all(|&x| (0..Q as i64).contains(&x)), "output reduced into [0, q)");
        // Independent scalar reference through the same reduction boundary.
        let mut r = [0i16; 256];
        for (slot, &x) in r.iter_mut().zip(&input) {
            *slot = x.rem_euclid(Q as i64) as i16;
        }
        ntt_scalar(&mut r);
        let want: Vec<i64> = r.iter().map(|&x| (x as i32).rem_euclid(Q) as i64).collect();
        assert_eq!(got, want, "mlkem_ntt must equal the scalar reference (mod q)");
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
            assert_eq!(res.lane(i).0 as i16, want, "lane fqmul lane {i} must equal scalar fqmul");
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

        assert_eq!(lane_sink, raw_sink, "lane and raw fqmul chains must be bit-identical after {K} iters");

        println!("\n=== Lanes16Word16 Montgomery butterfly (fqmul ×16) — 200k dependent chain, +avx2 ===");
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
}
