//! ML-DSA-65 (FIPS-204 / Dilithium) runtime kernels — the post-quantum SIGNATURE scheme that
//! complements ML-KEM-768 in the channel. This module builds bottom-up: the length-256 NTT over
//! the Dilithium prime `q = 8380417` (a COMPLETE transform, unlike Kyber's incomplete one — `q ≡ 1
//! (mod 512)`, so `X²⁵⁶+1` splits fully), with signed Montgomery reduction. Validated against the
//! schoolbook negacyclic convolution.

use crate::kernel_args as args;

pub(crate) const Q: i32 = 8_380_417;
const QINV: i32 = 58_728_449; // q⁻¹ mod 2³² (signed)
const ZETA: i64 = 1753; // a primitive 512th root of unity mod q
/// 2³² mod q — the Montgomery factor R.
const MONT: i64 = 4_193_792;
const N: usize = 256;

/// Signed Montgomery reduction: for `|a| ≤ q·2³¹`, returns `a·2⁻³² mod q` in `(−q, q)`.
#[inline]
pub(crate) fn montgomery_reduce(a: i64) -> i32 {
    let t = (a as i32).wrapping_mul(QINV) as i64;
    ((a - t * Q as i64) >> 32) as i32
}

/// The Dilithium twiddle table: `zetas[i] = ζ^brv₈(i) · R mod q`, reduced into `(−q/2, q/2]`.
pub(crate) const ZETAS: [i32; 256] = zeta_table();

const fn zeta_table() -> [i32; 256] {
    let mut pow = [1i64; 256];
    let mut i = 1;
    while i < 256 {
        pow[i] = pow[i - 1] * ZETA % Q as i64;
        i += 1;
    }
    let mut z = [0i32; 256];
    let mut i = 0;
    while i < 256 {
        let mut v = pow[(i as u8).reverse_bits() as usize] * MONT % Q as i64;
        if v > Q as i64 / 2 {
            v -= Q as i64;
        }
        z[i] = v as i32;
        i += 1;
    }
    z
}

/// Forward NTT in place (Dilithium reference): `a` normal domain → NTT domain. Kept scalar/
/// register-resident — measured A/B shows 8-wide butterflies lose here (small-group per-zeta setup
/// overhead + the scalar is already register-tight); the win is in the full-poly ops (inverse
/// F-scaling, pointwise), not the butterflies.
pub(crate) fn ntt(a: &mut [i32; N]) {
    let zetas = &ZETAS;
    let mut k = 0usize;
    let mut len = 128usize;
    while len > 0 {
        let mut start = 0usize;
        while start < N {
            k += 1;
            let zeta = zetas[k] as i64;
            for j in start..start + len {
                let t = montgomery_reduce(zeta * a[j + len] as i64);
                a[j + len] = a[j] - t;
                a[j] = a[j] + t;
            }
            start += 2 * len;
        }
        len >>= 1;
    }
}

const INVNTT_F: i64 = 41_978; // mont²/256 mod q

/// Inverse NTT with the `tomont` scaling (Dilithium `invntt_tomont`): NTT domain → normal domain,
/// folding in the final `f = mont²/256` factor. Scalar — LLVM auto-vectorizes the butterfly and the
/// final scaling in release; hand-written AVX2 measured slower.
pub(crate) fn invntt_tomont(a: &mut [i32; N]) {
    let zetas = &ZETAS;
    let mut k = 256usize;
    let mut len = 1usize;
    while len < N {
        let mut start = 0usize;
        while start < N {
            k -= 1;
            let zeta = -(zetas[k] as i64);
            for j in start..start + len {
                let t = a[j];
                a[j] = t + a[j + len];
                a[j + len] = t - a[j + len];
                a[j + len] = montgomery_reduce(zeta * a[j + len] as i64);
            }
            start += 2 * len;
        }
        len <<= 1;
    }
    for x in a.iter_mut() {
        *x = montgomery_reduce(INVNTT_F * *x as i64);
    }
}

/// `a mod q` as a representative in [−6283009, 6283008], by a multiply-free rounding shift (the
/// Dilithium reference `reduce32`). Exact for `a ≤ 2³¹ − 2²² − 1`.
#[inline]
pub(crate) fn reduce32(a: i32) -> i32 {
    let t = (a + (1 << 22)) >> 23;
    a - t * Q
}

/// Return a sum of pointwise products to the normal domain: the matrix–vector rows of KeyGen
/// (Â∘ŝ₁), Sign (Â∘ŷ) and Verify (Â∘ẑ − ĉ∘t̂₁·2ᵈ) each end here. A row of six products reaches
/// 6(q − 1), past `invntt_tomont`'s input bound of q, so every coefficient is reduced first.
pub(crate) fn to_normal_domain(acc: &mut [i32; N]) {
    for x in acc.iter_mut() {
        *x = reduce32(*x);
    }
    invntt_tomont(acc);
}

/// Pointwise product in the NTT domain, with Montgomery reduction (`poly_pointwise_montgomery`).
pub(crate) fn pointwise_montgomery(a: &[i32; N], b: &[i32; N]) -> [i32; N] {
    // Scalar — LLVM auto-vectorizes this element-wise `montgomery_reduce` loop in release; measured A/B
    // showed hand-written AVX2 intrinsics ~3× SLOWER than the auto-vectorized scalar. Trust the compiler
    // for simple full-poly loops; hand-vectorize only irreducible data-parallel restructuring (expand_a).
    std::array::from_fn(|i| montgomery_reduce(a[i] as i64 * b[i] as i64))
}

/// Reduce a coefficient into the canonical `[0, q)` representative, for `a ≤ 2³¹ − 2²² − 1`.
#[inline]
pub(crate) fn freeze(a: i32) -> i32 {
    caddq(reduce32(a))
}

/// Add q to a negative representative: `(−q, q)` → `[0, q)`, by the sign mask.
#[inline]
pub(crate) fn caddq(a: i32) -> i32 {
    a + ((a >> 31) & Q)
}

// ── ML-DSA-65 (Dilithium3) parameters ──────────────────────────────────────────────────────────
pub(crate) const D: i32 = 13; // Power2Round split
pub(crate) const GAMMA1: i32 = 1 << 19; // 524288 — ExpandMask range
pub(crate) const GAMMA2: i32 = (Q - 1) / 32; // 261888 — Decompose interval (α/2)

// ── Rounding & hints (FIPS-204 §7.4) ───────────────────────────────────────────────────────────

/// Power2Round: split `r ∈ [0,q)` as `r = r1·2ᵈ + r0` with `r0 ∈ (−2ᵈ⁻¹, 2ᵈ⁻¹]`. Returns `(r1, r0)`.
pub(crate) fn power2round(r: i32) -> (i32, i32) {
    let r = freeze(r);
    let r1 = (r + (1 << (D - 1)) - 1) >> D;
    let r0 = r - (r1 << D);
    (r1, r0)
}

/// Decompose `r ∈ [0,q)` as `r = r1·2γ2 + r0` with `r0 ∈ (−γ2, γ2]` (Dilithium's γ2 = (q−1)/32 path).
/// Returns `(r1, r0)`; `r1 ∈ [0, 16)`.
pub(crate) fn decompose(r: i32) -> (i32, i32) {
    let r = freeze(r);
    let mut a1 = (r + 127) >> 7;
    a1 = (a1 * 1025 + (1 << 21)) >> 22;
    a1 &= 15;
    let mut a0 = r - a1 * 2 * GAMMA2;
    a0 -= (((Q - 1) / 2 - a0) >> 31) & Q;
    (a1, a0)
}

/// HighBits / LowBits — the two halves of [`decompose`].
pub(crate) fn highbits(r: i32) -> i32 {
    decompose(r).0
}
pub(crate) fn lowbits(r: i32) -> i32 {
    decompose(r).1
}

/// MakeHint(z, r): 1 iff adding `z` carries `r` across a Decompose boundary (so HighBits changes).
pub(crate) fn make_hint(z: i32, r: i32) -> u8 {
    (highbits(r) != highbits(r + z)) as u8
}

/// UseHint(h, r): recover `HighBits(r + z)` from `r` and the hint bit `h` (FIPS-204 §7.4).
pub(crate) fn use_hint(h: u8, r: i32) -> i32 {
    const M: i32 = 16; // (q-1)/(2·γ2)
    let (r1, r0) = decompose(r);
    if h == 0 {
        r1
    } else if r0 > 0 {
        (r1 + 1) % M
    } else {
        (r1 - 1 + M) % M
    }
}

// ── ML-DSA-65 dimensions ───────────────────────────────────────────────────────────────────────
pub(crate) const MK: usize = 6; // rows (k)
pub(crate) const ML: usize = 5; // cols (l)
pub(crate) const ETA: i32 = 4;
pub(crate) const TAU: usize = 49;

type Poly = [i32; N];

// ── Sampling (FIPS-204 §7.3) ───────────────────────────────────────────────────────────────────

/// RejNTTPoly / ExpandA element: uniform NTT-domain polynomial from SHAKE128(seed), rejection on
/// 23-bit reads `< q`. `seed = ρ ‖ col ‖ row` (34 bytes).
fn rej_ntt_poly(seed: &[u8]) -> Poly {
    let mut st = crate::keccak::shake_absorb(seed, 168);
    let mut buf = [0u8; 168];
    crate::keccak::shake_squeeze_block(&st, &mut buf, 168);
    let mut pos = 0;
    let mut a = [0i32; N];
    let mut ctr = 0;
    while ctr < N {
        if pos + 3 > 168 {
            crate::keccak::keccak_permute(&mut st);
            crate::keccak::shake_squeeze_block(&st, &mut buf, 168);
            pos = 0;
        }
        let d = (buf[pos] as u32)
            | ((buf[pos + 1] as u32) << 8)
            | (((buf[pos + 2] as u32) & 0x7f) << 16);
        pos += 3;
        if d < Q as u32 {
            a[ctr] = d as i32;
            ctr += 1;
        }
    }
    a
}

/// RejBoundedPoly / ExpandS element (η=4): coefficients in `[−η, η]` from SHAKE256(seed), each
/// nibble `< 9` mapped to `η − nibble`. `seed = ρ' ‖ nonce(2)` (66 bytes).
fn rej_bounded_poly(seed: &[u8]) -> Poly {
    let mut st = crate::keccak::shake_absorb(seed, 136);
    let mut buf = [0u8; 136];
    crate::keccak::shake_squeeze_block(&st, &mut buf, 136);
    let mut pos = 0;
    let mut a = [0i32; N];
    let mut ctr = 0;
    while ctr < N {
        if pos >= 136 {
            crate::keccak::keccak_permute(&mut st);
            crate::keccak::shake_squeeze_block(&st, &mut buf, 136);
            pos = 0;
        }
        let b = buf[pos];
        pos += 1;
        let t0 = (b & 15) as i32;
        let t1 = (b >> 4) as i32;
        if t0 < 9 {
            a[ctr] = ETA - t0;
            ctr += 1;
        }
        if t1 < 9 && ctr < N {
            a[ctr] = ETA - t1;
            ctr += 1;
        }
    }
    a
}

/// ExpandA → the k×l matrix Â, each element a uniform NTT-domain polynomial keyed by (row, col).
/// ExpandA — the 6×5 public matrix Â in NTT domain. Each entry Â[r][c] = RejNTTPoly(ρ‖c‖r) via
/// SHAKE128 rejection. AVX2 batches four entries per `keccak_f1600_x4` permutation (bit-identical to
/// the scalar path — `keccak_f1600_x4` is a verified 4× lane replica); scalar fallback otherwise.
/// This is ~68% of `verify` (30 SHAKE128 streams), so vectorizing it is the dominant verify win.
pub(crate) fn expand_a(rho: &[u8]) -> Vec<Vec<Poly>> {
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            const ENTRIES: usize = MK * ML;
            let coords = &ENTRY_COORDS;
            let mut flat = vec![[0i32; N]; ENTRIES];
            let mut e = 0;
            while e + 4 <= ENTRIES {
                let blocks: [[u8; 168]; 4] = std::array::from_fn(|l| {
                    let (r, c) = coords[e + l];
                    mldsa_expand_xof_block(rho, c as u8, r as u8)
                });
                let mut accs: [Vec<i32>; 4] = std::array::from_fn(|_| Vec::with_capacity(256));
                let mut st = unsafe { crate::keccak::shake128_x4_absorb_once(&blocks) };
                loop {
                    let outb = unsafe { crate::keccak::shake128_x4_squeeze_block(&st) };
                    let mut done = true;
                    for (l, acc) in accs.iter_mut().enumerate() {
                        if acc.len() < N {
                            mldsa_reject_ntt_block(&outb[l], acc);
                        }
                        if acc.len() < N {
                            done = false;
                        }
                    }
                    if done {
                        break;
                    }
                    unsafe { crate::keccak::keccak_f1600_x4(&mut st) };
                }
                for (l, acc) in accs.iter().enumerate() {
                    let (r, c) = coords[e + l];
                    flat[r * ML + c].copy_from_slice(&acc[..N]);
                }
                e += 4;
            }
            while e < ENTRIES {
                let (r, c) = coords[e];
                let mut seed = rho[..32].to_vec();
                seed.push(c as u8);
                seed.push(r as u8);
                flat[r * ML + c] = rej_ntt_poly(&seed);
                e += 1;
            }
            return (0..MK)
                .map(|r| (0..ML).map(|c| flat[r * ML + c]).collect())
                .collect();
        }
    }
    expand_a_scalar(rho)
}

/// `(row, column)` of each entry of Â in row-major order, the order ExpandA batches them in.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
pub(crate) const ENTRY_COORDS: [(usize, usize); MK * ML] = entry_coords();

const fn entry_coords() -> [(usize, usize); MK * ML] {
    let mut coords = [(0, 0); MK * ML];
    let mut e = 0;
    while e < MK * ML {
        coords[e] = (e / ML, e % ML);
        e += 1;
    }
    coords
}

fn expand_a_scalar(rho: &[u8]) -> Vec<Vec<Poly>> {
    (0..MK)
        .map(|r| {
            (0..ML)
                .map(|s| {
                    let mut seed = rho.to_vec();
                    seed.push(s as u8);
                    seed.push(r as u8);
                    rej_ntt_poly(&seed)
                })
                .collect()
        })
        .collect()
}

/// The padded 168-byte SHAKE128 absorb block for Â[r][c]: `ρ‖c‖r`, the `0x1f` domain delimiter, and
/// the `0x80` final bit — matching what `shake_absorb(ρ‖c‖r, 168)` produces on the scalar path.
#[cfg(target_arch = "x86_64")]
#[inline]
fn mldsa_expand_xof_block(rho: &[u8], s: u8, r: u8) -> [u8; 168] {
    let mut blk = [0u8; 168];
    blk[..32].copy_from_slice(&rho[..32]);
    blk[32] = s;
    blk[33] = r;
    blk[34] = 0x1f;
    blk[167] |= 0x80;
    blk
}

/// RejNTTPoly over one 168-byte SHAKE128 block: 56 non-straddling 3-byte groups (168 = 56·3) parsed
/// as 23-bit integers, keeping those `< q`, appended to `out` (capped at 256).
#[cfg(target_arch = "x86_64")]
#[inline]
fn mldsa_reject_ntt_block(buf: &[u8; 168], out: &mut Vec<i32>) {
    let q = Q as u32;
    let mut k = 0;
    while k + 3 <= 168 && out.len() < N {
        let d = (buf[k] as u32) | ((buf[k + 1] as u32) << 8) | (((buf[k + 2] as u32) & 0x7f) << 16);
        k += 3;
        if d < q {
            out.push(d as i32);
        }
    }
}

// ── Bit-packing (FIPS-204 §7.1, Dilithium layouts) ─────────────────────────────────────────────

/// Pack `t1` (10-bit coefficients): 4 coeffs → 5 bytes. 320 bytes/poly.
fn pack_t1(t1: &Poly) -> Vec<u8> {
    let mut r = vec![0u8; N / 4 * 5];
    for i in 0..N / 4 {
        let a = [t1[4 * i], t1[4 * i + 1], t1[4 * i + 2], t1[4 * i + 3]];
        r[5 * i] = a[0] as u8;
        r[5 * i + 1] = ((a[0] >> 8) | (a[1] << 2)) as u8;
        r[5 * i + 2] = ((a[1] >> 6) | (a[2] << 4)) as u8;
        r[5 * i + 3] = ((a[2] >> 4) | (a[3] << 6)) as u8;
        r[5 * i + 4] = (a[3] >> 2) as u8;
    }
    r
}

/// Pack η=4 coefficients (`η − coeff` in `[0,8]`, 4 bits): 2 coeffs → 1 byte. 128 bytes/poly.
fn pack_eta(p: &Poly) -> Vec<u8> {
    let mut r = vec![0u8; N / 2];
    for i in 0..N / 2 {
        let t0 = (ETA - p[2 * i]) as u8;
        let t1 = (ETA - p[2 * i + 1]) as u8;
        r[i] = t0 | (t1 << 4);
    }
    r
}

/// Pack `t0` (13-bit signed, `2¹² − coeff`): 8 coeffs → 13 bytes. 416 bytes/poly.
fn pack_t0(p: &Poly) -> Vec<u8> {
    let mut r = vec![0u8; N / 8 * 13];
    for i in 0..N / 8 {
        let mut t = [0u32; 8];
        for j in 0..8 {
            t[j] = ((1 << (D - 1)) - p[8 * i + j]) as u32;
        }
        let o = 13 * i;
        r[o] = t[0] as u8;
        r[o + 1] = ((t[0] >> 8) | (t[1] << 5)) as u8;
        r[o + 2] = (t[1] >> 3) as u8;
        r[o + 3] = ((t[1] >> 11) | (t[2] << 2)) as u8;
        r[o + 4] = ((t[2] >> 6) | (t[3] << 7)) as u8;
        r[o + 5] = (t[3] >> 1) as u8;
        r[o + 6] = ((t[3] >> 9) | (t[4] << 4)) as u8;
        r[o + 7] = (t[4] >> 4) as u8;
        r[o + 8] = ((t[4] >> 12) | (t[5] << 1)) as u8;
        r[o + 9] = ((t[5] >> 7) | (t[6] << 6)) as u8;
        r[o + 10] = (t[6] >> 2) as u8;
        r[o + 11] = ((t[6] >> 10) | (t[7] << 3)) as u8;
        r[o + 12] = (t[7] >> 5) as u8;
    }
    r
}

// ── ML-DSA-65 KeyGen (FIPS-204 §6.1) ───────────────────────────────────────────────────────────

/// `pk` = ρ ‖ pack(t1).
pub const PK_BYTES: usize = 32 + MK * 320;
/// `sk` = ρ ‖ K ‖ tr ‖ pack(s1) ‖ pack(s2) ‖ pack(t0).
pub const SK_BYTES: usize = 32 + 32 + 64 + ML * 128 + MK * 128 + MK * 416;

/// ML-DSA.KeyGen_internal(ξ) → (pk, sk), deterministic in the 32-byte seed ξ.
pub fn keygen(seed: &[u8; 32]) -> (Vec<u8>, Vec<u8>) {
    let mut h_in = seed.to_vec();
    h_in.push(MK as u8);
    h_in.push(ML as u8);
    let hash = crate::keccak::shake256_bytes(&h_in, 128);
    let rho = &hash[0..32];
    let rho_prime = &hash[32..96];
    let k_key = &hash[96..128];

    let a_hat = expand_a(rho);
    let s1: Vec<Poly> = (0..ML)
        .map(|i| {
            let mut seed = rho_prime.to_vec();
            seed.extend_from_slice(&(i as u16).to_le_bytes());
            rej_bounded_poly(&seed)
        })
        .collect();
    let s2: Vec<Poly> = (0..MK)
        .map(|i| {
            let mut seed = rho_prime.to_vec();
            seed.extend_from_slice(&((ML + i) as u16).to_le_bytes());
            rej_bounded_poly(&seed)
        })
        .collect();

    let s1_hat: Vec<Poly> = s1
        .iter()
        .map(|p| {
            let mut q = *p;
            ntt(&mut q);
            q
        })
        .collect();
    let mut t1 = vec![[0i32; N]; MK];
    let mut t0 = vec![[0i32; N]; MK];
    for i in 0..MK {
        let mut acc = [0i32; N];
        for j in 0..ML {
            let prod = pointwise_montgomery(&a_hat[i][j], &s1_hat[j]);
            for c in 0..N {
                acc[c] += prod[c];
            }
        }
        to_normal_domain(&mut acc);
        for c in 0..N {
            let t = freeze(acc[c] + s2[i][c]);
            let (r1, r0) = power2round(t);
            t1[i][c] = r1;
            t0[i][c] = r0;
        }
    }

    let mut pk = rho.to_vec();
    for poly in &t1 {
        pk.extend(pack_t1(poly));
    }
    let tr = crate::keccak::shake256_bytes(&pk, 64);

    let mut sk = rho.to_vec();
    sk.extend_from_slice(k_key);
    sk.extend_from_slice(&tr);
    for poly in &s1 {
        sk.extend(pack_eta(poly));
    }
    for poly in &s2 {
        sk.extend(pack_eta(poly));
    }
    for poly in &t0 {
        sk.extend(pack_t0(poly));
    }
    (pk, sk)
}

// ── ML-DSA-65 sign/verify parameters ───────────────────────────────────────────────────────────
pub(crate) const BETA: i32 = (TAU as i32) * ETA; // 196
pub(crate) const OMEGA: usize = 55;
const CTILDE: usize = 48; // λ/4
/// ML-DSA-65 signature length.
pub const SIG_BYTES: usize = CTILDE + ML * 640 + OMEGA + MK;

/// Map `a` into the signed representative `(−q/2, q/2]` and return its magnitude basis for ‖·‖∞.
/// Subtracts q exactly when `Q/2 − r` is negative, by its sign mask.
#[inline]
pub(crate) fn to_signed(a: i32) -> i32 {
    let r = freeze(a);
    r - (((Q / 2 - r) >> 31) & Q)
}
fn inf_norm(p: &Poly) -> i32 {
    p.iter().map(|&c| to_signed(c).abs()).max().unwrap()
}

#[inline]
fn nttp(p: &Poly) -> Poly {
    let mut q = *p;
    ntt(&mut q);
    q
}

// ── Unpacking (sk/pk/sig decode) ───────────────────────────────────────────────────────────────

fn unpack_eta(b: &[u8]) -> Poly {
    std::array::from_fn(|i| {
        let byte = b[i / 2];
        let nib = if i % 2 == 0 { byte & 15 } else { byte >> 4 };
        ETA - nib as i32
    })
}

fn unpack_t0(b: &[u8]) -> Poly {
    let mut r = [0i32; N];
    for i in 0..N / 8 {
        let a: Vec<u32> = b[13 * i..13 * i + 13].iter().map(|&x| x as u32).collect();
        let mut t = [0u32; 8];
        t[0] = a[0] | (a[1] << 8);
        t[1] = (a[1] >> 5) | (a[2] << 3) | (a[3] << 11);
        t[2] = (a[3] >> 2) | (a[4] << 6);
        t[3] = (a[4] >> 7) | (a[5] << 1) | (a[6] << 9);
        t[4] = (a[6] >> 4) | (a[7] << 4) | (a[8] << 12);
        t[5] = (a[8] >> 1) | (a[9] << 7);
        t[6] = (a[9] >> 6) | (a[10] << 2) | (a[11] << 10);
        t[7] = (a[11] >> 3) | (a[12] << 5);
        for j in 0..8 {
            r[8 * i + j] = (1 << (D - 1)) - (t[j] & 0x1fff) as i32;
        }
    }
    r
}

fn unpack_t1(b: &[u8]) -> Poly {
    let mut r = [0i32; N];
    for i in 0..N / 4 {
        let a: Vec<u32> = b[5 * i..5 * i + 5].iter().map(|&x| x as u32).collect();
        r[4 * i] = ((a[0] | (a[1] << 8)) & 0x3ff) as i32;
        r[4 * i + 1] = (((a[1] >> 2) | (a[2] << 6)) & 0x3ff) as i32;
        r[4 * i + 2] = (((a[2] >> 4) | (a[3] << 4)) & 0x3ff) as i32;
        r[4 * i + 3] = (((a[3] >> 6) | (a[4] << 2)) & 0x3ff) as i32;
    }
    r
}

fn unpack_z(b: &[u8]) -> Poly {
    let mut r = [0i32; N];
    for i in 0..N / 2 {
        let a: Vec<u32> = b[5 * i..5 * i + 5].iter().map(|&x| x as u32).collect();
        let z0 = (a[0] | (a[1] << 8) | (a[2] << 16)) & 0xfffff;
        let z1 = ((a[2] >> 4) | (a[3] << 4) | (a[4] << 12)) & 0xfffff;
        r[2 * i] = GAMMA1 - z0 as i32;
        r[2 * i + 1] = GAMMA1 - z1 as i32;
    }
    r
}

fn unpack_hint(b: &[u8]) -> Option<Vec<Poly>> {
    let mut h = vec![[0i32; N]; MK];
    let mut k = 0usize;
    for i in 0..MK {
        let cnt = b[OMEGA + i] as usize;
        if cnt < k || cnt > OMEGA {
            return None;
        }
        for j in k..cnt {
            if j > k && b[j] <= b[j - 1] {
                return None; // indices must be strictly increasing within a poly
            }
            h[i][b[j] as usize] = 1;
        }
        k = cnt;
    }
    if b[k..OMEGA].iter().any(|&x| x != 0) {
        return None; // padding must be zero
    }
    Some(h)
}

// ── sig packing ────────────────────────────────────────────────────────────────────────────────

fn pack_z(p: &Poly) -> Vec<u8> {
    let mut r = vec![0u8; N / 2 * 5];
    for i in 0..N / 2 {
        let t0 = (GAMMA1 - to_signed(p[2 * i])) as u32;
        let t1 = (GAMMA1 - to_signed(p[2 * i + 1])) as u32;
        r[5 * i] = t0 as u8;
        r[5 * i + 1] = (t0 >> 8) as u8;
        r[5 * i + 2] = ((t0 >> 16) | (t1 << 4)) as u8;
        r[5 * i + 3] = (t1 >> 4) as u8;
        r[5 * i + 4] = (t1 >> 12) as u8;
    }
    r
}

fn pack_w1(p: &Poly) -> Vec<u8> {
    (0..N / 2)
        .map(|i| (p[2 * i] | (p[2 * i + 1] << 4)) as u8)
        .collect()
}

fn pack_hint(h: &[Poly]) -> Vec<u8> {
    let mut r = vec![0u8; OMEGA + MK];
    let mut k = 0usize;
    for (i, poly) in h.iter().enumerate() {
        for (j, &v) in poly.iter().enumerate() {
            if v != 0 {
                r[k] = j as u8;
                k += 1;
            }
        }
        r[OMEGA + i] = k as u8;
    }
    r
}

// ── ExpandMask + SampleInBall ──────────────────────────────────────────────────────────────────

fn expand_mask(rho: &[u8], kappa: u16) -> Poly {
    let mut seed = rho.to_vec();
    seed.extend_from_slice(&kappa.to_le_bytes());
    let buf = crate::keccak::shake256_bytes(&seed, N / 2 * 5);
    let mut a = [0i32; N];
    for i in 0..N / 2 {
        let o = 5 * i;
        let z0 = (buf[o] as u32) | ((buf[o + 1] as u32) << 8) | (((buf[o + 2] as u32) & 0xf) << 16);
        let z1 =
            ((buf[o + 2] as u32) >> 4) | ((buf[o + 3] as u32) << 4) | ((buf[o + 4] as u32) << 12);
        a[2 * i] = GAMMA1 - z0 as i32;
        a[2 * i + 1] = GAMMA1 - z1 as i32;
    }
    a
}

fn sample_in_ball(seed: &[u8]) -> Poly {
    let mut st = crate::keccak::shake_absorb(seed, 136);
    let mut buf = [0u8; 136];
    crate::keccak::shake_squeeze_block(&st, &mut buf, 136);
    let signs = u64::from_le_bytes(buf[0..8].try_into().unwrap());
    let mut pos = 8;
    let mut c = [0i32; N];
    let mut sign_idx = 0;
    for i in (N - TAU)..N {
        let j = loop {
            if pos >= 136 {
                crate::keccak::keccak_permute(&mut st);
                crate::keccak::shake_squeeze_block(&st, &mut buf, 136);
                pos = 0;
            }
            let candidate = buf[pos] as usize;
            pos += 1;
            if candidate <= i {
                break candidate;
            }
        };
        c[i] = c[j];
        c[j] = 1 - 2 * (((signs >> sign_idx) & 1) as i32);
        sign_idx += 1;
    }
    c
}

/// The message representative of the pure (no pre-hash) interface, FIPS 204 Algorithms 2–3:
/// `M' = 0x00 ‖ |ctx| ‖ ctx ‖ M`. `None` for a context longer than its one-byte length can name.
fn message_representative(m: &[u8], ctx: &[u8]) -> Option<Vec<u8>> {
    let len = u8::try_from(ctx.len()).ok()?;
    let mut mp = Vec::with_capacity(2 + ctx.len() + m.len());
    mp.push(0);
    mp.push(len);
    mp.extend_from_slice(ctx);
    mp.extend_from_slice(m);
    Some(mp)
}

/// `μ = H(tr ‖ M', 64)`.
fn compute_mu(tr: &[u8], mp: &[u8]) -> Vec<u8> {
    let mut mu_in = tr.to_vec();
    mu_in.extend_from_slice(mp);
    crate::keccak::shake256_bytes(&mu_in, 64)
}

// ── ML-DSA.Sign / Verify (FIPS-204 §6.2, §6.3) ─────────────────────────────────────────────────

/// Why [`sign`] refuses its inputs: FIPS 204 returns ⊥ for these instead of a signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignError {
    /// The context string is longer than the 255 bytes its one-byte length prefix can name.
    ContextTooLong { len: usize },
    /// The secret key is not the [`SK_BYTES`] bytes of an ML-DSA-65 secret key.
    SecretKeyLength { len: usize },
    /// A coefficient of s₁ or s₂ decodes outside [−η, η], so no KeyGen produced this key.
    SecretKeyOutOfRange,
}

impl std::fmt::Display for SignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignError::ContextTooLong { len } => {
                write!(f, "the context is {len} bytes; FIPS 204 allows at most 255")
            }
            SignError::SecretKeyLength { len } => write!(
                f,
                "the secret key is {len} bytes; an ML-DSA-65 secret key is {SK_BYTES}"
            ),
            SignError::SecretKeyOutOfRange => write!(
                f,
                "the secret key holds an s1 or s2 coefficient outside [-{ETA}, {ETA}]"
            ),
        }
    }
}

impl std::error::Error for SignError {}

/// A secret key through FIPS 204 skDecode: `ρ ‖ K ‖ tr ‖ s₁ ‖ s₂ ‖ t₀`, refused unless it is exactly
/// [`SK_BYTES`] long and every s₁/s₂ coefficient lies in [−η, η].
struct SecretKey<'a> {
    rho: &'a [u8],
    k_key: &'a [u8],
    tr: &'a [u8],
    s1: Vec<Poly>,
    s2: Vec<Poly>,
    t0: Vec<Poly>,
}

impl<'a> SecretKey<'a> {
    fn decode(sk: &'a [u8]) -> Result<Self, SignError> {
        if sk.len() != SK_BYTES {
            return Err(SignError::SecretKeyLength { len: sk.len() });
        }
        let s1_at = 128;
        let s2_at = s1_at + ML * 128;
        let t0_at = s2_at + MK * 128;
        let eta = |at: usize, count: usize| -> Vec<Poly> {
            (0..count)
                .map(|i| unpack_eta(&sk[at + 128 * i..at + 128 * (i + 1)]))
                .collect()
        };
        if !eta_nibbles_in_range(&sk[s1_at..t0_at]) {
            return Err(SignError::SecretKeyOutOfRange);
        }
        Ok(SecretKey {
            rho: &sk[0..32],
            k_key: &sk[32..64],
            tr: &sk[64..128],
            s1: eta(s1_at, ML),
            s2: eta(s2_at, MK),
            t0: (0..MK)
                .map(|i| unpack_t0(&sk[t0_at + 416 * i..t0_at + 416 * (i + 1)]))
                .collect(),
        })
    }
}

/// Whether every 4-bit field of the packed s₁ ‖ s₂ is at most 2η, so `η − nibble` lies in [−η, η]. The
/// verdict is accumulated over every nibble, never decided early on a secret one.
fn eta_nibbles_in_range(packed: &[u8]) -> bool {
    let mut over = 0u8;
    for &b in packed {
        over |= ((b & 15) > 2 * ETA as u8) as u8;
        over |= ((b >> 4) > 2 * ETA as u8) as u8;
    }
    over == 0
}

/// Deterministic ML-DSA-65 signature over `m` with context `ctx` (the `rnd = 0` variant).
pub fn sign(sk: &[u8], m: &[u8], ctx: &[u8]) -> Result<Vec<u8>, SignError> {
    sign_randomized(sk, m, ctx, &[0u8; 32])
}

/// Hedged ML-DSA-65 signature, FIPS 204's default variant: `rnd` is 32 fresh random bytes mixed into
/// the signature's mask seed, so signing one message twice, or under an injected fault, never
/// repeats the mask.
pub fn sign_randomized(
    sk: &[u8],
    m: &[u8],
    ctx: &[u8],
    rnd: &[u8; 32],
) -> Result<Vec<u8>, SignError> {
    let mp = message_representative(m, ctx).ok_or(SignError::ContextTooLong { len: ctx.len() })?;
    let key = SecretKey::decode(sk)?;
    Ok(sign_internal(&key, &mp, rnd))
}

/// ML-DSA.Sign_internal (FIPS 204 Algorithm 7) over the message representative `mp` with the
/// 32-byte randomness `rnd` (all zero for the deterministic variant).
fn sign_internal(key: &SecretKey<'_>, mp: &[u8], rnd: &[u8; 32]) -> Vec<u8> {
    let a_hat = expand_a(key.rho);
    let s1_hat: Vec<Poly> = key.s1.iter().map(nttp).collect();
    let s2_hat: Vec<Poly> = key.s2.iter().map(nttp).collect();
    let t0_hat: Vec<Poly> = key.t0.iter().map(nttp).collect();

    let mu = compute_mu(key.tr, mp);
    let mut rp_in = key.k_key.to_vec();
    rp_in.extend_from_slice(rnd);
    rp_in.extend_from_slice(&mu);
    let rho_pp = crate::keccak::shake256_bytes(&rp_in, 64);

    let mut kappa = 0u16;
    loop {
        let y: Vec<Poly> = (0..ML)
            .map(|i| expand_mask(&rho_pp, kappa + i as u16))
            .collect();
        let y_hat: Vec<Poly> = y.iter().map(nttp).collect();
        let mut w = vec![[0i32; N]; MK];
        for i in 0..MK {
            let mut acc = [0i32; N];
            for j in 0..ML {
                let prod = pointwise_montgomery(&a_hat[i][j], &y_hat[j]);
                for c in 0..N {
                    acc[c] += prod[c];
                }
            }
            to_normal_domain(&mut acc);
            w[i] = acc;
        }
        let w1: Vec<Poly> = w
            .iter()
            .map(|p| std::array::from_fn(|c| highbits(p[c])))
            .collect();

        let mut ct_in = mu.clone();
        for p in &w1 {
            ct_in.extend(pack_w1(p));
        }
        let c_tilde = crate::keccak::shake256_bytes(&ct_in, CTILDE);
        let c_hat = nttp(&sample_in_ball(&c_tilde));

        let cs1: Vec<Poly> = s1_hat
            .iter()
            .map(|sh| {
                let mut a = pointwise_montgomery(&c_hat, sh);
                invntt_tomont(&mut a);
                a
            })
            .collect();
        let cs2: Vec<Poly> = s2_hat
            .iter()
            .map(|sh| {
                let mut a = pointwise_montgomery(&c_hat, sh);
                invntt_tomont(&mut a);
                a
            })
            .collect();

        let z: Vec<Poly> = (0..ML)
            .map(|j| std::array::from_fn(|c| y[j][c] + cs1[j][c]))
            .collect();
        let r0: Vec<Poly> = (0..MK)
            .map(|i| std::array::from_fn(|c| lowbits(freeze(w[i][c] - cs2[i][c]))))
            .collect();

        if z.iter().any(|p| inf_norm(p) >= GAMMA1 - BETA)
            || r0.iter().any(|p| inf_norm(p) >= GAMMA2 - BETA)
        {
            kappa += ML as u16;
            continue;
        }

        let ct0: Vec<Poly> = t0_hat
            .iter()
            .map(|th| {
                let mut a = pointwise_montgomery(&c_hat, th);
                invntt_tomont(&mut a);
                a
            })
            .collect();
        let mut h = vec![[0i32; N]; MK];
        let mut weight = 0i32;
        for i in 0..MK {
            for c in 0..N {
                let r = w[i][c] - cs2[i][c] + ct0[i][c];
                let bit = make_hint(-ct0[i][c], r) as i32;
                h[i][c] = bit;
                weight += bit;
            }
        }
        if ct0.iter().any(|p| inf_norm(p) >= GAMMA2) || weight > OMEGA as i32 {
            kappa += ML as u16;
            continue;
        }

        let mut sig = c_tilde.clone();
        for p in &z {
            sig.extend(pack_z(p));
        }
        sig.extend(pack_hint(&h));
        return sig;
    }
}

/// Verify an ML-DSA-65 signature; `true` iff valid for `(pk, m, ctx)`.
pub fn verify(pk: &[u8], m: &[u8], ctx: &[u8], sig: &[u8]) -> bool {
    let Some(mp) = message_representative(m, ctx) else {
        return false;
    };
    if sig.len() != SIG_BYTES || pk.len() != PK_BYTES {
        return false;
    }
    let rho = &pk[0..32];
    let t1: Vec<Poly> = (0..MK)
        .map(|i| unpack_t1(&pk[32 + i * 320..32 + (i + 1) * 320]))
        .collect();

    let c_tilde = &sig[0..CTILDE];
    let mut off = CTILDE;
    let z: Vec<Poly> = (0..ML)
        .map(|_| {
            let p = unpack_z(&sig[off..off + 640]);
            off += 640;
            p
        })
        .collect();
    let h = match unpack_hint(&sig[off..off + OMEGA + MK]) {
        Some(h) => h,
        None => return false,
    };
    if z.iter().any(|p| inf_norm(p) >= GAMMA1 - BETA) {
        return false;
    }

    let a_hat = expand_a(rho);
    let tr = crate::keccak::shake256_bytes(pk, 64);
    let mu = compute_mu(&tr, &mp);
    let c_hat = nttp(&sample_in_ball(c_tilde));
    let z_hat: Vec<Poly> = z.iter().map(nttp).collect();

    let mut w1 = vec![[0i32; N]; MK];
    for i in 0..MK {
        let mut acc = [0i32; N];
        for j in 0..ML {
            let prod = pointwise_montgomery(&a_hat[i][j], &z_hat[j]);
            for c in 0..N {
                acc[c] += prod[c];
            }
        }
        let t1d_hat = nttp(&std::array::from_fn(|c| t1[i][c] << D));
        let ct1 = pointwise_montgomery(&c_hat, &t1d_hat);
        for c in 0..N {
            acc[c] -= ct1[c];
        }
        to_normal_domain(&mut acc);
        for c in 0..N {
            w1[i][c] = use_hint(h[i][c] as u8, acc[c]);
        }
    }

    let mut ct_in = mu;
    for p in &w1 {
        ct_in.extend(pack_w1(p));
    }
    c_tilde == crate::keccak::shake256_bytes(&ct_in, CTILDE).as_slice()
}

/// Dev-only profiler: break `verify`'s cost into components to locate the hot spot. Not shipped logic.
pub fn bench_verify_breakdown(pk: &[u8], sig: &[u8]) {
    use std::time::Instant;
    let rho = &pk[0..32];
    let c_tilde = &sig[0..CTILDE];
    let iters = 2000u32;
    macro_rules! t {
        ($l:expr, $b:expr) => {{
            for _ in 0..50 {
                std::hint::black_box($b);
            }
            let s = Instant::now();
            for _ in 0..iters {
                std::hint::black_box($b);
            }
            eprintln!(
                "  {:<22} {:>9.1} ns/op",
                $l,
                s.elapsed().as_nanos() as f64 / iters as f64
            );
        }};
    }
    eprintln!("--- ML-DSA verify component breakdown ---");
    t!("expand_a (30 poly)", expand_a(rho));
    t!("sample_in_ball", sample_in_ball(c_tilde));
    let c = sample_in_ball(c_tilde);
    t!("nttp (1 poly)", nttp(&c));
    t!("shake256(pk,64)", crate::keccak::shake256_bytes(pk, 64));
}

// ── Logos-facing wrappers (Seq of Int bytes 0..255) — the natives crypto.lg's handshake calls ────

/// `mldsaKeypair(seed)` → pk(1952) ‖ sk(4032).
pub fn mldsa_keypair_seq(seed: &[i64]) -> Result<logicaffeine_data::LogosSeq<i64>, String> {
    let (pk, sk) = keygen(&args::byte_array::<32>("seed", seed)?);
    let mut out = pk;
    out.extend(sk);
    Ok(args::seq_of_bytes(&out))
}
/// `mldsaSign(sk, msg, ctx)` → signature(3309).
pub fn mldsa_sign_seq(
    sk: &[i64],
    msg: &[i64],
    ctx: &[i64],
) -> Result<logicaffeine_data::LogosSeq<i64>, String> {
    let sk = args::bytes("secret key", sk)?;
    let msg = args::bytes("message", msg)?;
    let ctx = args::bytes("context", ctx)?;
    sign(&sk, &msg, &ctx)
        .map(|sig| args::seq_of_bytes(&sig))
        .map_err(|e| e.to_string())
}
/// `mldsaVerify(pk, msg, ctx, sig)` → 1 if valid, else 0. Whatever is not a byte string of the right
/// shape verifies nothing, so it answers 0 rather than failing the program.
pub fn mldsa_verify_seq(pk: &[i64], msg: &[i64], ctx: &[i64], sig: &[i64]) -> Result<i64, String> {
    let as_bytes = |s: &[i64]| args::bytes("", s).ok();
    Ok(
        match (as_bytes(pk), as_bytes(msg), as_bytes(ctx), as_bytes(sig)) {
            (Some(pk), Some(msg), Some(ctx), Some(sig)) => verify(&pk, &msg, &ctx, &sig) as i64,
            _ => 0,
        },
    )
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_system/mldsa/tests.rs"]
mod tests;
