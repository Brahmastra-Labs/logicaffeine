//! ChaCha20-Poly1305 AEAD (RFC 8439) — the symmetric seal that closes the post-quantum channel:
//! an ML-KEM-768 handshake establishes the shared secret, HKDF derives this 32-byte key, and every
//! wire message is sealed here. ChaCha20 is a pure `Word32` ARX cipher (the same primitive the
//! `assets/std/crypto.lg` `chacha20Block` ships in Logos); Poly1305 is the one-time authenticator in
//! constant-time 5×26-bit limbs (no bignum). Verified against the RFC 8439 §2.4.2 / §2.5.2 / §2.8.2
//! known-answer vectors. The tag compare is constant-time.

use crate::kernel_args as args;

// ── ChaCha20 (RFC 8439 §2.3–2.4) ───────────────────────────────────────────────────────────────

#[inline]
fn qr(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(7);
}

fn chacha20_block(key: &[u32; 8], counter: u32, nonce: &[u32; 3]) -> [u32; 16] {
    let mut state = [
        0x6170_7865,
        0x3320_646e,
        0x7962_2d32,
        0x6b20_6574,
        key[0],
        key[1],
        key[2],
        key[3],
        key[4],
        key[5],
        key[6],
        key[7],
        counter,
        nonce[0],
        nonce[1],
        nonce[2],
    ];
    let mut w = state;
    for _ in 0..10 {
        qr(&mut w, 0, 4, 8, 12);
        qr(&mut w, 1, 5, 9, 13);
        qr(&mut w, 2, 6, 10, 14);
        qr(&mut w, 3, 7, 11, 15);
        qr(&mut w, 0, 5, 10, 15);
        qr(&mut w, 1, 6, 11, 12);
        qr(&mut w, 2, 7, 8, 13);
        qr(&mut w, 3, 4, 9, 14);
    }
    for i in 0..16 {
        state[i] = w[i].wrapping_add(state[i]);
    }
    state
}

fn le_u32x8(b: &[u8; 32]) -> [u32; 8] {
    std::array::from_fn(|i| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()))
}
fn le_u32x3(b: &[u8; 12]) -> [u32; 3] {
    std::array::from_fn(|i| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()))
}

/// ChaCha20 keystream XOR (RFC 8439 §2.4): `data ⊕ ChaCha20(key, counter‖nonce)`. AVX2 (8 blocks at
/// a time) when available, else scalar — bit-identical.
pub fn chacha20_xor(key: &[u8; 32], nonce: &[u8; 12], counter: u32, data: &[u8]) -> Vec<u8> {
    #[cfg(target_arch = "x86_64")]
    {
        // The 8-block AVX2 path computes 512 B of keystream at once, so it only pays off past a few
        // blocks; below that the scalar single-block path avoids the wasted work.
        if data.len() >= 256 && std::is_x86_feature_detected!("avx2") {
            return unsafe { chacha20_xor_avx2(&le_u32x8(key), &le_u32x3(nonce), counter, data) };
        }
    }
    chacha20_xor_scalar(key, nonce, counter, data)
}

fn chacha20_xor_scalar(key: &[u8; 32], nonce: &[u8; 12], counter: u32, data: &[u8]) -> Vec<u8> {
    let kw = le_u32x8(key);
    let nw = le_u32x3(nonce);
    let mut out = Vec::with_capacity(data.len());
    for (bi, chunk) in data.chunks(64).enumerate() {
        let block = chacha20_block(&kw, counter.wrapping_add(bi as u32), &nw);
        let mut ks = [0u8; 64];
        for i in 0..16 {
            ks[i * 4..i * 4 + 4].copy_from_slice(&block[i].to_le_bytes());
        }
        for (j, &b) in chunk.iter().enumerate() {
            out.push(b ^ ks[j]);
        }
    }
    out
}

/// AVX2 ChaCha20: 8 blocks computed in parallel (each `__m256i` lane is one block's word at a fixed
/// index), the keystream serialized and XORed. The `<512`-byte tail falls back to the scalar block.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn chacha20_xor_avx2(
    key: &[u32; 8],
    nonce: &[u32; 3],
    counter: u32,
    data: &[u8],
) -> Vec<u8> {
    use std::arch::x86_64::*;
    let rot16 = _mm256_setr_epi8(
        2, 3, 0, 1, 6, 7, 4, 5, 10, 11, 8, 9, 14, 15, 12, 13, 2, 3, 0, 1, 6, 7, 4, 5, 10, 11, 8, 9,
        14, 15, 12, 13,
    );
    let rot8 = _mm256_setr_epi8(
        3, 0, 1, 2, 7, 4, 5, 6, 11, 8, 9, 10, 15, 12, 13, 14, 3, 0, 1, 2, 7, 4, 5, 6, 11, 8, 9, 10,
        15, 12, 13, 14,
    );
    #[inline(always)]
    unsafe fn rotl12(x: std::arch::x86_64::__m256i) -> std::arch::x86_64::__m256i {
        use std::arch::x86_64::*;
        _mm256_or_si256(_mm256_slli_epi32::<12>(x), _mm256_srli_epi32::<20>(x))
    }
    #[inline(always)]
    unsafe fn rotl7(x: std::arch::x86_64::__m256i) -> std::arch::x86_64::__m256i {
        use std::arch::x86_64::*;
        _mm256_or_si256(_mm256_slli_epi32::<7>(x), _mm256_srli_epi32::<25>(x))
    }

    let mut out = Vec::with_capacity(data.len());
    let lane_ctr = _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7);
    let mut off = 0usize;
    let mut base = counter;
    while off < data.len() {
        let init: [__m256i; 16] = [
            _mm256_set1_epi32(0x6170_7865u32 as i32),
            _mm256_set1_epi32(0x3320_646eu32 as i32),
            _mm256_set1_epi32(0x7962_2d32u32 as i32),
            _mm256_set1_epi32(0x6b20_6574u32 as i32),
            _mm256_set1_epi32(key[0] as i32),
            _mm256_set1_epi32(key[1] as i32),
            _mm256_set1_epi32(key[2] as i32),
            _mm256_set1_epi32(key[3] as i32),
            _mm256_set1_epi32(key[4] as i32),
            _mm256_set1_epi32(key[5] as i32),
            _mm256_set1_epi32(key[6] as i32),
            _mm256_set1_epi32(key[7] as i32),
            _mm256_add_epi32(_mm256_set1_epi32(base as i32), lane_ctr),
            _mm256_set1_epi32(nonce[0] as i32),
            _mm256_set1_epi32(nonce[1] as i32),
            _mm256_set1_epi32(nonce[2] as i32),
        ];
        let mut v = init;
        macro_rules! qr {
            ($a:tt, $b:tt, $c:tt, $d:tt) => {
                v[$a] = _mm256_add_epi32(v[$a], v[$b]);
                v[$d] = _mm256_shuffle_epi8(_mm256_xor_si256(v[$d], v[$a]), rot16);
                v[$c] = _mm256_add_epi32(v[$c], v[$d]);
                v[$b] = rotl12(_mm256_xor_si256(v[$b], v[$c]));
                v[$a] = _mm256_add_epi32(v[$a], v[$b]);
                v[$d] = _mm256_shuffle_epi8(_mm256_xor_si256(v[$d], v[$a]), rot8);
                v[$c] = _mm256_add_epi32(v[$c], v[$d]);
                v[$b] = rotl7(_mm256_xor_si256(v[$b], v[$c]));
            };
        }
        for _ in 0..10 {
            qr!(0, 4, 8, 12);
            qr!(1, 5, 9, 13);
            qr!(2, 6, 10, 14);
            qr!(3, 7, 11, 15);
            qr!(0, 5, 10, 15);
            qr!(1, 6, 11, 12);
            qr!(2, 7, 8, 13);
            qr!(3, 4, 9, 14);
        }
        let mut words = [[0u32; 8]; 16];
        for j in 0..16 {
            let sum = _mm256_add_epi32(v[j], init[j]);
            _mm256_storeu_si256(words[j].as_mut_ptr() as *mut __m256i, sum);
        }
        let mut ks = [0u8; 512];
        for blk in 0..8 {
            for j in 0..16 {
                ks[blk * 64 + j * 4..blk * 64 + j * 4 + 4]
                    .copy_from_slice(&words[j][blk].to_le_bytes());
            }
        }
        let n = (data.len() - off).min(512);
        let dst_start = out.len();
        out.resize(dst_start + n, 0);
        let dst = out.as_mut_ptr().add(dst_start);
        let src = data.as_ptr().add(off);
        let mut i = 0;
        while i + 32 <= n {
            let a = _mm256_loadu_si256(src.add(i) as *const __m256i);
            let b = _mm256_loadu_si256(ks.as_ptr().add(i) as *const __m256i);
            _mm256_storeu_si256(dst.add(i) as *mut __m256i, _mm256_xor_si256(a, b));
            i += 32;
        }
        while i < n {
            *dst.add(i) = *src.add(i) ^ ks[i];
            i += 1;
        }
        off += 512;
        base = base.wrapping_add(8);
    }
    out
}

// ── Poly1305 (RFC 8439 §2.5), 5×26-bit limbs, constant-time ────────────────────────────────────

const MASK26: u32 = 0x3ff_ffff;

/// Clamp `r` into 5×26-bit limbs.
fn poly1305_clamp(key: &[u8; 32]) -> [u32; 5] {
    let t0 = u32::from_le_bytes([key[0], key[1], key[2], key[3]]);
    let t1 = u32::from_le_bytes([key[4], key[5], key[6], key[7]]);
    let t2 = u32::from_le_bytes([key[8], key[9], key[10], key[11]]);
    let t3 = u32::from_le_bytes([key[12], key[13], key[14], key[15]]);
    [
        t0 & 0x3ff_ffff,
        ((t0 >> 26) | (t1 << 6)) & 0x3ff_ff03,
        ((t1 >> 20) | (t2 << 12)) & 0x3ff_c0ff,
        ((t2 >> 14) | (t3 << 18)) & 0x3f0_3fff,
        (t3 >> 8) & 0x00f_ffff,
    ]
}

/// A 16-byte block's 5×26-bit limbs; `hibit = 1` adds the 2¹²⁸ marker (a full block — for a partial
/// block the caller writes the appended `1` byte inside `b` and passes `hibit = 0`).
fn poly1305_block(b: &[u8; 16], hibit: u32) -> [u32; 5] {
    let m0 = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let m1 = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    let m2 = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
    let m3 = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
    [
        m0 & MASK26,
        ((m0 >> 26) | (m1 << 6)) & MASK26,
        ((m1 >> 20) | (m2 << 12)) & MASK26,
        ((m2 >> 14) | (m3 << 18)) & MASK26,
        (m3 >> 8) | (hibit << 24),
    ]
}

/// Carry-reduce 5 wide limbs (each ≤ ~2⁶⁰, e.g. the sum of four 130-bit products) into 5×26-bit
/// limbs mod (2¹³⁰−5).
fn poly1305_reduce(d: [u64; 5]) -> [u32; 5] {
    let m = MASK26 as u64;
    let mut c = d[0] >> 26;
    let h0 = (d[0] & m) as u32;
    let d1 = d[1] + c;
    c = d1 >> 26;
    let h1 = (d1 & m) as u32;
    let d2 = d[2] + c;
    c = d2 >> 26;
    let h2 = (d2 & m) as u32;
    let d3 = d[3] + c;
    c = d3 >> 26;
    let h3 = (d3 & m) as u32;
    let d4 = d[4] + c;
    c = d4 >> 26;
    let h4 = (d4 & m) as u32;
    // Top carry `c` (up to ~2³⁴) wraps via ·5 into limb 0 — it can span more than one limb.
    let h0w = h0 as u64 + c * 5;
    let mut h = [(h0w & m) as u32, h1 + (h0w >> 26) as u32, h2, h3, h4];
    let c1 = h[1] >> 26;
    h[1] &= MASK26;
    h[2] += c1;
    h
}

/// `a · b mod (2¹³⁰−5)` over 5×26-bit limbs (inputs ≤ ~2²⁷), fully reduced.
fn poly1305_mul(a: &[u32; 5], b: &[u32; 5]) -> [u32; 5] {
    let (a0, a1, a2, a3, a4) = (
        a[0] as u64,
        a[1] as u64,
        a[2] as u64,
        a[3] as u64,
        a[4] as u64,
    );
    let (b0, b1, b2, b3, b4) = (
        b[0] as u64,
        b[1] as u64,
        b[2] as u64,
        b[3] as u64,
        b[4] as u64,
    );
    let (s1, s2, s3, s4) = (b1 * 5, b2 * 5, b3 * 5, b4 * 5);
    poly1305_reduce([
        a0 * b0 + a1 * s4 + a2 * s3 + a3 * s2 + a4 * s1,
        a0 * b1 + a1 * b0 + a2 * s4 + a3 * s3 + a4 * s2,
        a0 * b2 + a1 * b1 + a2 * b0 + a3 * s4 + a4 * s3,
        a0 * b3 + a1 * b2 + a2 * b1 + a3 * b0 + a4 * s4,
        a0 * b4 + a1 * b3 + a2 * b2 + a3 * b1 + a4 * b0,
    ])
}

/// Finalize: full reduction mod p, then `+ s mod 2¹²⁸` → the 16-byte tag.
fn poly1305_finalize(h: [u32; 5], key: &[u8; 32]) -> [u8; 16] {
    let (mut h0, mut h1, mut h2, mut h3, mut h4) = (h[0], h[1], h[2], h[3], h[4]);
    let mut c;
    c = h1 >> 26;
    h1 &= 0x3ff_ffff;
    h2 += c;
    c = h2 >> 26;
    h2 &= 0x3ff_ffff;
    h3 += c;
    c = h3 >> 26;
    h3 &= 0x3ff_ffff;
    h4 += c;
    c = h4 >> 26;
    h4 &= 0x3ff_ffff;
    h0 += c * 5;
    c = h0 >> 26;
    h0 &= 0x3ff_ffff;
    h1 += c;
    let mut g0 = h0.wrapping_add(5);
    c = g0 >> 26;
    g0 &= 0x3ff_ffff;
    let mut g1 = h1.wrapping_add(c);
    c = g1 >> 26;
    g1 &= 0x3ff_ffff;
    let mut g2 = h2.wrapping_add(c);
    c = g2 >> 26;
    g2 &= 0x3ff_ffff;
    let mut g3 = h3.wrapping_add(c);
    c = g3 >> 26;
    g3 &= 0x3ff_ffff;
    let g4 = h4.wrapping_add(c).wrapping_sub(1 << 26);
    let mask = (g4 >> 31).wrapping_sub(1);
    let nmask = !mask;
    h0 = (h0 & nmask) | (g0 & mask);
    h1 = (h1 & nmask) | (g1 & mask);
    h2 = (h2 & nmask) | (g2 & mask);
    h3 = (h3 & nmask) | (g3 & mask);
    h4 = (h4 & nmask) | (g4 & mask);
    let f0 = (h0 | (h1 << 26)) as u64;
    let f1 = ((h1 >> 6) | (h2 << 20)) as u64;
    let f2 = ((h2 >> 12) | (h3 << 14)) as u64;
    let f3 = ((h3 >> 18) | (h4 << 8)) as u64;
    let s0 = u32::from_le_bytes([key[16], key[17], key[18], key[19]]) as u64;
    let s1 = u32::from_le_bytes([key[20], key[21], key[22], key[23]]) as u64;
    let s2 = u32::from_le_bytes([key[24], key[25], key[26], key[27]]) as u64;
    let s3 = u32::from_le_bytes([key[28], key[29], key[30], key[31]]) as u64;
    let mut tag = [0u8; 16];
    let mut f = f0 + s0;
    tag[0..4].copy_from_slice(&(f as u32).to_le_bytes());
    f = f1 + s1 + (f >> 32);
    tag[4..8].copy_from_slice(&(f as u32).to_le_bytes());
    f = f2 + s2 + (f >> 32);
    tag[8..12].copy_from_slice(&(f as u32).to_le_bytes());
    f = f3 + s3 + (f >> 32);
    tag[12..16].copy_from_slice(&(f as u32).to_le_bytes());
    tag
}

/// Process one block scalar: `h ← (h + block) · r`.
#[inline]
fn poly1305_absorb(h: &mut [u32; 5], msg: &[u8], i: usize, r: &[u32; 5]) {
    let n = (msg.len() - i).min(16);
    let mut b = [0u8; 16];
    b[..n].copy_from_slice(&msg[i..i + n]);
    let hibit = if n == 16 {
        1
    } else {
        b[n] = 1;
        0
    };
    let m = poly1305_block(&b, hibit);
    let sum = [
        h[0] + m[0],
        h[1] + m[1],
        h[2] + m[2],
        h[3] + m[3],
        h[4] + m[4],
    ];
    *h = poly1305_mul(&sum, r);
}

/// Poly1305 one-time authenticator: `tag = ((Σ blockᵢ·r^(n−i)) mod (2¹³⁰−5)) + s mod 2¹²⁸`. AVX2
/// (4 blocks/group via precomputed r¹..r⁴) for long messages, else scalar — bit-identical.
pub fn poly1305(key: &[u8; 32], msg: &[u8]) -> [u8; 16] {
    #[cfg(target_arch = "x86_64")]
    {
        if msg.len() >= 256 && std::is_x86_feature_detected!("avx2") {
            return unsafe { poly1305_avx2(key, msg) };
        }
    }
    poly1305_scalar(key, msg)
}

fn poly1305_scalar(key: &[u8; 32], msg: &[u8]) -> [u8; 16] {
    let r = poly1305_clamp(key);
    let mut h = [0u32; 5];
    let mut i = 0;
    while i < msg.len() {
        poly1305_absorb(&mut h, msg, i, &r);
        i += 16;
    }
    poly1305_finalize(h, key)
}

/// Horizontal sum of the four u64 lanes.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn poly1305_hsum(v: std::arch::x86_64::__m256i) -> u64 {
    use std::arch::x86_64::*;
    let lo = _mm256_castsi256_si128(v);
    let hi = _mm256_extracti128_si256::<1>(v);
    let s = _mm_add_epi64(lo, hi);
    let s2 = _mm_add_epi64(s, _mm_unpackhi_epi64(s, s));
    _mm_cvtsi128_si64(s2) as u64
}

/// Per-limb multiplier vectors `[r⁴, r³, r², r¹]` (and `5×` for the 2¹³⁰ wrap) at even 32-bit lanes.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn poly1305_vectors(
    r: &[u32; 5],
) -> (
    [std::arch::x86_64::__m256i; 5],
    [std::arch::x86_64::__m256i; 5],
) {
    use std::arch::x86_64::*;
    let r2 = poly1305_mul(r, r);
    let r3 = poly1305_mul(&r2, r);
    let r4 = poly1305_mul(&r2, &r2);
    let mut vb = [_mm256_setzero_si256(); 5];
    let mut vs = [_mm256_setzero_si256(); 5];
    for l in 0..5 {
        vb[l] = _mm256_set_epi32(
            0,
            r[l] as i32,
            0,
            r2[l] as i32,
            0,
            r3[l] as i32,
            0,
            r4[l] as i32,
        );
        vs[l] = _mm256_set_epi32(
            0,
            (r[l] * 5) as i32,
            0,
            (r2[l] * 5) as i32,
            0,
            (r3[l] * 5) as i32,
            0,
            (r4[l] * 5) as i32,
        );
    }
    (vb, vs)
}

/// Absorb the full 4-block groups of `data` into `h` (four products/group via `_mm256_mul_epu32`,
/// one per lane, summed), each block carrying the 2¹²⁸ marker. Returns bytes consumed (`groups·64`).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn poly1305_groups(
    h: &mut [u32; 5],
    data: &[u8],
    vb: &[std::arch::x86_64::__m256i; 5],
    vs: &[std::arch::x86_64::__m256i; 5],
) -> usize {
    use std::arch::x86_64::*;
    let groups = (data.len() / 16) / 4;
    for g in 0..groups {
        let base = g * 64;
        let mut t = [[0u32; 5]; 4];
        for k in 0..4 {
            let b: &[u8; 16] = data[base + k * 16..base + k * 16 + 16].try_into().unwrap();
            t[k] = poly1305_block(b, 1);
        }
        for l in 0..5 {
            t[0][l] += h[l];
        }
        let mut va = [_mm256_setzero_si256(); 5];
        for l in 0..5 {
            va[l] = _mm256_set_epi32(
                0,
                t[3][l] as i32,
                0,
                t[2][l] as i32,
                0,
                t[1][l] as i32,
                0,
                t[0][l] as i32,
            );
        }
        macro_rules! mul {
            ($x:expr, $y:expr) => {
                _mm256_mul_epu32($x, $y)
            };
        }
        macro_rules! add5 {
            ($a:expr, $b:expr, $c:expr, $d:expr, $e:expr) => {
                _mm256_add_epi64(
                    _mm256_add_epi64(_mm256_add_epi64($a, $b), _mm256_add_epi64($c, $d)),
                    $e,
                )
            };
        }
        let d = [
            add5!(
                mul!(va[0], vb[0]),
                mul!(va[1], vs[4]),
                mul!(va[2], vs[3]),
                mul!(va[3], vs[2]),
                mul!(va[4], vs[1])
            ),
            add5!(
                mul!(va[0], vb[1]),
                mul!(va[1], vb[0]),
                mul!(va[2], vs[4]),
                mul!(va[3], vs[3]),
                mul!(va[4], vs[2])
            ),
            add5!(
                mul!(va[0], vb[2]),
                mul!(va[1], vb[1]),
                mul!(va[2], vb[0]),
                mul!(va[3], vs[4]),
                mul!(va[4], vs[3])
            ),
            add5!(
                mul!(va[0], vb[3]),
                mul!(va[1], vb[2]),
                mul!(va[2], vb[1]),
                mul!(va[3], vb[0]),
                mul!(va[4], vs[4])
            ),
            add5!(
                mul!(va[0], vb[4]),
                mul!(va[1], vb[3]),
                mul!(va[2], vb[2]),
                mul!(va[3], vb[1]),
                mul!(va[4], vb[0])
            ),
        ];
        *h = poly1305_reduce([
            poly1305_hsum(d[0]),
            poly1305_hsum(d[1]),
            poly1305_hsum(d[2]),
            poly1305_hsum(d[3]),
            poly1305_hsum(d[4]),
        ]);
    }
    groups * 64
}

/// AVX2 Poly1305: 4 message blocks per group in parallel, then a scalar tail.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn poly1305_avx2(key: &[u8; 32], msg: &[u8]) -> [u8; 16] {
    let r = poly1305_clamp(key);
    let (vb, vs) = poly1305_vectors(&r);
    let mut h = [0u32; 5];
    let mut i = poly1305_groups(&mut h, msg, &vb, &vs);
    while i < msg.len() {
        poly1305_absorb(&mut h, msg, i, &r);
        i += 16;
    }
    poly1305_finalize(h, key)
}

/// Absorb one AEAD block: up to 16 bytes, zero-padded to 16, ALWAYS with the 2¹²⁸ marker — the AEAD
/// MAC zero-pads `aad`/`ct` to 16-byte boundaries, so every block is "full" (unlike generic
/// Poly1305, which appends a `1` to a short final block).
#[inline]
fn poly1305_absorb_padded(h: &mut [u32; 5], data: &[u8], r: &[u32; 5]) {
    let n = data.len().min(16);
    let mut b = [0u8; 16];
    b[..n].copy_from_slice(&data[..n]);
    let m = poly1305_block(&b, 1);
    let sum = [
        h[0] + m[0],
        h[1] + m[1],
        h[2] + m[2],
        h[3] + m[3],
        h[4] + m[4],
    ];
    *h = poly1305_mul(&sum, r);
}

/// The AEAD MAC over `aad ‖ pad ‖ ct ‖ pad ‖ len(aad) ‖ len(ct)`, STREAMED — no `mac_data` buffer.
/// `ct` runs through the AVX2 4-block path in place (the dominant cost on bulk); `aad`, the padded
/// tails, and the length block are scalar. Bit-identical to `poly1305(otk, &mac_data(aad, ct))`.
fn poly1305_aead(otk: &[u8; 32], aad: &[u8], ct: &[u8]) -> [u8; 16] {
    let r = poly1305_clamp(otk);
    let mut h = [0u32; 5];
    let mut a = 0;
    while a < aad.len() {
        poly1305_absorb_padded(&mut h, &aad[a..(a + 16).min(aad.len())], &r);
        a += 16;
    }
    let mut i = 0usize;
    #[cfg(target_arch = "x86_64")]
    {
        if ct.len() >= 256 && std::is_x86_feature_detected!("avx2") {
            unsafe {
                let (vb, vs) = poly1305_vectors(&r);
                i = poly1305_groups(&mut h, ct, &vb, &vs);
            }
        }
    }
    while i < ct.len() {
        poly1305_absorb_padded(&mut h, &ct[i..(i + 16).min(ct.len())], &r);
        i += 16;
    }
    let mut lb = [0u8; 16];
    lb[0..8].copy_from_slice(&(aad.len() as u64).to_le_bytes());
    lb[8..16].copy_from_slice(&(ct.len() as u64).to_le_bytes());
    poly1305_absorb_padded(&mut h, &lb, &r);
    poly1305_finalize(h, otk)
}

// ── ChaCha20-Poly1305 AEAD (RFC 8439 §2.8) ─────────────────────────────────────────────────────

fn poly1305_key_gen(key: &[u8; 32], nonce: &[u8; 12]) -> [u8; 32] {
    let block = chacha20_block(&le_u32x8(key), 0, &le_u32x3(nonce));
    let mut otk = [0u8; 32];
    for i in 0..8 {
        otk[i * 4..i * 4 + 4].copy_from_slice(&block[i].to_le_bytes());
    }
    otk
}

/// The MAC input `aad ‖ pad ‖ ct ‖ pad ‖ len(aad) ‖ len(ct)`, materialized. Kept only as the
/// reference the streaming `poly1305_aead` is fuzzed against — the shipped seal/open stream instead.
#[cfg(test)]
fn mac_data(aad: &[u8], ciphertext: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(aad.len() + ciphertext.len() + 32);
    m.extend_from_slice(aad);
    m.resize(m.len() + ((16 - aad.len() % 16) % 16), 0);
    m.extend_from_slice(ciphertext);
    m.resize(m.len() + ((16 - ciphertext.len() % 16) % 16), 0);
    m.extend_from_slice(&(aad.len() as u64).to_le_bytes());
    m.extend_from_slice(&(ciphertext.len() as u64).to_le_bytes());
    m
}

/// The longest plaintext one (key, nonce) seals (RFC 8439 §2.8): the payload's blocks start at
/// counter 1 and must not wrap the 32-bit counter back onto the one-time key's keystream.
pub const MAX_PLAINTEXT: u64 = (1 << 38) - 64;

/// AEAD seal: returns `ciphertext ‖ tag[16]`.
pub fn chacha20poly1305_seal(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    plaintext: &[u8],
) -> Vec<u8> {
    assert!(
        plaintext.len() as u64 <= MAX_PLAINTEXT,
        "ChaCha20-Poly1305 seals at most 2^38 - 64 bytes under one nonce"
    );
    let otk = poly1305_key_gen(key, nonce);
    let mut out = chacha20_xor(key, nonce, 1, plaintext);
    let tag = poly1305_aead(&otk, aad, &out);
    out.extend_from_slice(&tag);
    out
}

/// AEAD open: verifies the tag (constant-time) and decrypts, or `None` on tamper/truncation.
pub fn chacha20poly1305_open(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    sealed: &[u8],
) -> Option<Vec<u8>> {
    if sealed.len() < 16 || (sealed.len() - 16) as u64 > MAX_PLAINTEXT {
        return None;
    }
    let (ciphertext, tag) = sealed.split_at(sealed.len() - 16);
    let otk = poly1305_key_gen(key, nonce);
    let expected = poly1305_aead(&otk, aad, ciphertext);
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= expected[i] ^ tag[i];
    }
    if diff != 0 {
        return None;
    }
    Some(chacha20_xor(key, nonce, 1, ciphertext))
}

// ── XChaCha20-Poly1305 (draft-irtf-cfrg-xchacha-03) ─────────────────────────────────────────────

/// HChaCha20 (draft-irtf-cfrg-xchacha §2.2): the ChaCha20 rounds over `key` and a 128-bit `nonce` in
/// the counter-and-nonce words, without the final addition, returning the first and last rows — a
/// subkey as uniform as the key, one per nonce.
pub fn hchacha20(key: &[u8; 32], nonce: &[u8; 16]) -> [u8; 32] {
    let k = le_u32x8(key);
    let n: [u32; 4] =
        std::array::from_fn(|i| u32::from_le_bytes(nonce[i * 4..i * 4 + 4].try_into().unwrap()));
    let mut w = [
        0x6170_7865,
        0x3320_646e,
        0x7962_2d32,
        0x6b20_6574,
        k[0],
        k[1],
        k[2],
        k[3],
        k[4],
        k[5],
        k[6],
        k[7],
        n[0],
        n[1],
        n[2],
        n[3],
    ];
    for _ in 0..10 {
        qr(&mut w, 0, 4, 8, 12);
        qr(&mut w, 1, 5, 9, 13);
        qr(&mut w, 2, 6, 10, 14);
        qr(&mut w, 3, 7, 11, 15);
        qr(&mut w, 0, 5, 10, 15);
        qr(&mut w, 1, 6, 11, 12);
        qr(&mut w, 2, 7, 8, 13);
        qr(&mut w, 3, 4, 9, 14);
    }
    let mut out = [0u8; 32];
    for (i, word) in w[..4].iter().chain(&w[12..]).enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

/// The ChaCha20-Poly1305 key and nonce an XChaCha20-Poly1305 nonce selects: the HChaCha20 subkey of
/// its first 16 bytes, and its last 8 behind four zero bytes.
fn xchacha_subkey(key: &[u8; 32], nonce: &[u8; 24]) -> ([u8; 32], [u8; 12]) {
    let subkey = hchacha20(key, nonce[..16].try_into().unwrap());
    let mut inner = [0u8; 12];
    inner[4..].copy_from_slice(&nonce[16..]);
    (subkey, inner)
}

/// XChaCha20-Poly1305 seal: `ciphertext ‖ tag[16]`. Its 192-bit nonce is wide enough to draw at
/// random for every message under one key, which a 96-bit nonce is not.
pub fn xchacha20poly1305_seal(
    key: &[u8; 32],
    nonce: &[u8; 24],
    aad: &[u8],
    plaintext: &[u8],
) -> Vec<u8> {
    let (subkey, inner) = xchacha_subkey(key, nonce);
    chacha20poly1305_seal(&subkey, &inner, aad, plaintext)
}

/// XChaCha20-Poly1305 open: verifies the tag (constant-time) and decrypts, or `None` on
/// tamper/truncation.
pub fn xchacha20poly1305_open(
    key: &[u8; 32],
    nonce: &[u8; 24],
    aad: &[u8],
    sealed: &[u8],
) -> Option<Vec<u8>> {
    let (subkey, inner) = xchacha_subkey(key, nonce);
    chacha20poly1305_open(&subkey, &inner, aad, sealed)
}

// ── Boxes: XChaCha20-Poly1305 under a nonce drawn for each seal ────────────────────────────────

/// The associated data a box of bytes is sealed under.
pub const BOX_BYTES_LABEL: &[u8] = b"logos box: bytes";

/// The associated data a box of a wire-encoded value is sealed under: a box of one kind never opens
/// as the other.
pub const BOX_VALUE_LABEL: &[u8] = b"logos box: value";

/// What a box adds to what it seals: its 24-byte nonce in front and its 16-byte tag behind.
pub const BOX_OVERHEAD: usize = 24 + 16;

/// The box of `plaintext` under `key` and `nonce`: `nonce ‖ XChaCha20-Poly1305(key, nonce, label,
/// plaintext)`.
pub fn seal_box_under(key: &[u8; 32], nonce: &[u8; 24], label: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(BOX_OVERHEAD + plaintext.len());
    out.extend_from_slice(nonce);
    out.extend(xchacha20poly1305_seal(key, nonce, label, plaintext));
    out
}

/// The box of `plaintext` under `key`, its nonce drawn from the system's entropy: 192 random bits, so
/// one key seals any number of boxes without two sharing a nonce.
pub fn seal_box(key: &[u8; 32], label: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let mut nonce = [0u8; 24];
    crate::random::fill_from_entropy(&mut nonce)?;
    Ok(seal_box_under(key, &nonce, label, plaintext))
}

/// The plaintext of a box sealed under `key` and `label`, or `None` when it is shorter than a box or
/// does not authenticate.
pub fn open_box(key: &[u8; 32], label: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < BOX_OVERHEAD {
        return None;
    }
    let (nonce, body) = sealed.split_at(24);
    xchacha20poly1305_open(key, nonce.try_into().ok()?, label, body)
}

// ── Logos-facing wrappers (Seq of Int bytes 0..255) ────────────────────────────────────────────

/// `chacha20Encrypt(key, nonce, counter, data)` → `data ⊕ keystream` (RFC 8439 §2.4): a 32-byte key,
/// a 12-byte nonce, and a block counter the data's blocks stay inside of.
pub fn chacha20_encrypt_seq(
    key: &[i64],
    nonce: &[i64],
    counter: i64,
    data: &[i64],
) -> Result<logicaffeine_data::LogosSeq<i64>, String> {
    let k = args::byte_array::<32>("key", key)?;
    let n = args::byte_array::<12>("nonce", nonce)?;
    let counter = args::int_in("block counter", counter, 0, u32::MAX as i64)?;
    let data = args::bytes("data", data)?;
    let last = counter + data.len().div_ceil(64) as i64 - 1;
    if last > u32::MAX as i64 {
        return Err(format!(
            "the data runs the block counter from {counter} to {last}, past its 32 bits"
        ));
    }
    Ok(args::seq_of_bytes(&chacha20_xor(
        &k,
        &n,
        counter as u32,
        &data,
    )))
}

/// `poly1305Mac(key, msg)` → 16-byte tag. The MAC primitive (constant-time 5×26-bit limbs).
pub fn poly1305_seq(key: &[i64], msg: &[i64]) -> Result<logicaffeine_data::LogosSeq<i64>, String> {
    let k = args::byte_array::<32>("one-time key", key)?;
    Ok(args::seq_of_bytes(&poly1305(
        &k,
        &args::bytes("message", msg)?,
    )))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_system/aead/tests.rs"]
mod tests;
