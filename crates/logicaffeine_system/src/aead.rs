//! ChaCha20-Poly1305 AEAD (RFC 8439) — the symmetric seal that closes the post-quantum channel:
//! an ML-KEM-768 handshake establishes the shared secret, HKDF derives this 32-byte key, and every
//! wire message is sealed here. ChaCha20 is a pure `Word32` ARX cipher (the same primitive the
//! `assets/std/crypto.lg` `chacha20Block` ships in Logos); Poly1305 is the one-time authenticator in
//! constant-time 5×26-bit limbs (no bignum). Verified against the RFC 8439 §2.4.2 / §2.5.2 / §2.8.2
//! known-answer vectors. The tag compare is constant-time.

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
        0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574, key[0], key[1], key[2], key[3], key[4],
        key[5], key[6], key[7], counter, nonce[0], nonce[1], nonce[2],
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
unsafe fn chacha20_xor_avx2(key: &[u32; 8], nonce: &[u32; 3], counter: u32, data: &[u8]) -> Vec<u8> {
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
    let (a0, a1, a2, a3, a4) = (a[0] as u64, a[1] as u64, a[2] as u64, a[3] as u64, a[4] as u64);
    let (b0, b1, b2, b3, b4) = (b[0] as u64, b[1] as u64, b[2] as u64, b[3] as u64, b[4] as u64);
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
    let sum = [h[0] + m[0], h[1] + m[1], h[2] + m[2], h[3] + m[3], h[4] + m[4]];
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
) -> ([std::arch::x86_64::__m256i; 5], [std::arch::x86_64::__m256i; 5]) {
    use std::arch::x86_64::*;
    let r2 = poly1305_mul(r, r);
    let r3 = poly1305_mul(&r2, r);
    let r4 = poly1305_mul(&r2, &r2);
    let mut vb = [_mm256_setzero_si256(); 5];
    let mut vs = [_mm256_setzero_si256(); 5];
    for l in 0..5 {
        vb[l] = _mm256_set_epi32(0, r[l] as i32, 0, r2[l] as i32, 0, r3[l] as i32, 0, r4[l] as i32);
        vs[l] = _mm256_set_epi32(
            0, (r[l] * 5) as i32, 0, (r2[l] * 5) as i32, 0, (r3[l] * 5) as i32, 0, (r4[l] * 5) as i32,
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
                0, t[3][l] as i32, 0, t[2][l] as i32, 0, t[1][l] as i32, 0, t[0][l] as i32,
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
            add5!(mul!(va[0], vb[0]), mul!(va[1], vs[4]), mul!(va[2], vs[3]), mul!(va[3], vs[2]), mul!(va[4], vs[1])),
            add5!(mul!(va[0], vb[1]), mul!(va[1], vb[0]), mul!(va[2], vs[4]), mul!(va[3], vs[3]), mul!(va[4], vs[2])),
            add5!(mul!(va[0], vb[2]), mul!(va[1], vb[1]), mul!(va[2], vb[0]), mul!(va[3], vs[4]), mul!(va[4], vs[3])),
            add5!(mul!(va[0], vb[3]), mul!(va[1], vb[2]), mul!(va[2], vb[1]), mul!(va[3], vb[0]), mul!(va[4], vs[4])),
            add5!(mul!(va[0], vb[4]), mul!(va[1], vb[3]), mul!(va[2], vb[2]), mul!(va[3], vb[1]), mul!(va[4], vb[0])),
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
    let sum = [h[0] + m[0], h[1] + m[1], h[2] + m[2], h[3] + m[3], h[4] + m[4]];
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

/// AEAD seal: returns `ciphertext ‖ tag[16]`.
pub fn chacha20poly1305_seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
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
    if sealed.len() < 16 {
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

// ── Logos-facing wrappers (Seq of Int bytes 0..255) — the natives crypto.lg's aeadSeal/Open call ──

fn bytes(s: &[i64]) -> Vec<u8> {
    s.iter().map(|&x| x.rem_euclid(256) as u8).collect()
}
fn seq(v: &[u8]) -> logicaffeine_data::LogosSeq<i64> {
    logicaffeine_data::LogosSeq::from_vec(v.iter().map(|&b| b as i64).collect())
}

/// `chacha20Encrypt(key, nonce, counter, data)` → `data ⊕ keystream`. The cipher; the AEAD flow in
/// Logos calls it for both the Poly1305 one-time key (counter 0 over 32 zeros) and the payload
/// (counter 1).
pub fn chacha20_encrypt_seq(
    key: &[i64],
    nonce: &[i64],
    counter: i64,
    data: &[i64],
) -> logicaffeine_data::LogosSeq<i64> {
    let k: [u8; 32] = bytes(key)[..32].try_into().unwrap();
    let n: [u8; 12] = bytes(nonce)[..12].try_into().unwrap();
    seq(&chacha20_xor(&k, &n, counter as u32, &bytes(data)))
}

/// `poly1305Mac(key, msg)` → 16-byte tag. The MAC primitive (constant-time 5×26-bit limbs).
pub fn poly1305_seq(key: &[i64], msg: &[i64]) -> logicaffeine_data::LogosSeq<i64> {
    let k: [u8; 32] = bytes(key)[..32].try_into().unwrap();
    seq(&poly1305(&k, &bytes(msg)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn chacha20_avx2_matches_scalar_over_lengths() {
        // The 8-block AVX2 path must be byte-identical to the scalar block across chunk boundaries,
        // partial tails, and counter wrap-around.
        let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(31).wrapping_add(7));
        let nonce: [u8; 12] = std::array::from_fn(|i| (i as u8).wrapping_mul(17));
        let mut s = 0x1234_5678u64;
        let mut data = vec![0u8; 2000];
        for b in data.iter_mut() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (s >> 33) as u8;
        }
        for &len in &[0usize, 1, 63, 64, 65, 511, 512, 513, 1024, 1100, 2000] {
            for &ctr in &[0u32, 1, 7, u32::MAX - 3] {
                let want = chacha20_xor_scalar(&key, &nonce, ctr, &data[..len]);
                let got = chacha20_xor(&key, &nonce, ctr, &data[..len]);
                assert_eq!(got, want, "AVX2 == scalar, len {len} ctr {ctr}");
            }
        }
    }

    #[test]
    fn poly1305_avx2_matches_scalar_over_lengths() {
        // The 4-block AVX2 path must be byte-identical to the scalar block across group boundaries
        // and tails (the AVX2 path engages at ≥ 256 bytes).
        let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(29).wrapping_add(3));
        let mut s = 0xdead_beefu64;
        let mut msg = vec![0u8; 4096];
        for b in msg.iter_mut() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (s >> 33) as u8;
        }
        for &len in &[
            0usize, 1, 16, 17, 63, 64, 255, 256, 257, 271, 512, 1000, 1024, 1025, 4000, 4096,
        ] {
            let want = poly1305_scalar(&key, &msg[..len]);
            let got = poly1305(&key, &msg[..len]);
            assert_eq!(got, want, "AVX2 == scalar Poly1305, len {len}");
        }
    }

    #[test]
    fn poly1305_aead_streamed_matches_mac_data() {
        // The streamed AEAD MAC must equal the materialized `poly1305(mac_data(aad, ct))` across all
        // alignments of aad and ct (full groups, partial tails, AVX2 vs scalar ct).
        let mut s = 0x0123_4567_89ab_cdefu64;
        let mut next = |buf: &mut [u8]| {
            for b in buf.iter_mut() {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
                *b = (s >> 33) as u8;
            }
        };
        let mut otk = [0u8; 32];
        next(&mut otk);
        let mut ct = vec![0u8; 4100];
        next(&mut ct);
        let mut aad = vec![0u8; 64];
        next(&mut aad);
        for &al in &[0usize, 1, 11, 16, 17, 32, 48] {
            for &cl in &[0usize, 1, 15, 16, 63, 64, 255, 256, 257, 271, 1024, 1025, 4100] {
                let streamed = poly1305_aead(&otk, &aad[..al], &ct[..cl]);
                let reference = poly1305(&otk, &mac_data(&aad[..al], &ct[..cl]));
                assert_eq!(streamed, reference, "streamed AEAD MAC, aad {al} ct {cl}");
            }
        }
    }

    #[test]
    fn chacha20_xor_matches_rfc8439_2_4_2() {
        // RFC 8439 §2.4.2: the famous "Ladies and Gentlemen..." sunscreen vector.
        let key: [u8; 32] = std::array::from_fn(|i| i as u8);
        let nonce: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0x4a, 0, 0, 0, 0];
        let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let ct = chacha20_xor(&key, &nonce, 1, plaintext);
        assert_eq!(
            hex(&ct),
            "6e2e359a2568f98041ba0728dd0d6981e97e7aec1d4360c20a27afccfd9fae0bf91b65c5524733ab8f593dabcd62b3571639d624e65152ab8f530c359f0861d807ca0dbf500d6a6156a38e088a22b65e52bc514d16ccf806818ce91ab77937365af90bbf74a35be6b40b8eedf2785e42874d",
            "ChaCha20 keystream XOR must match RFC 8439 §2.4.2"
        );
    }

    #[test]
    fn poly1305_matches_rfc8439_2_5_2() {
        let key: [u8; 32] =
            unhex("85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b")[..32]
                .try_into()
                .unwrap();
        let msg = b"Cryptographic Forum Research Group";
        let tag = poly1305(&key, msg);
        assert_eq!(hex(&tag), "a8061dc1305136c6c22b8baf0c0127a9", "Poly1305 must match RFC 8439 §2.5.2");
    }

    #[test]
    fn aead_seals_and_opens_round_trip_and_rejects_tamper() {
        let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(3));
        let nonce: [u8; 12] = std::array::from_fn(|i| i as u8 + 0x40);
        let aad = b"channel-suite-id-v1";
        let msg = b"post-quantum sealed wire payload";
        let sealed = chacha20poly1305_seal(&key, &nonce, aad, msg);
        assert_eq!(sealed.len(), msg.len() + 16, "ciphertext + 16-byte tag");
        assert_eq!(chacha20poly1305_open(&key, &nonce, aad, &sealed).as_deref(), Some(&msg[..]));
        // Tamper the ciphertext, the tag, and the AAD — each must reject.
        let mut t = sealed.clone();
        t[0] ^= 1;
        assert_eq!(chacha20poly1305_open(&key, &nonce, aad, &t), None, "ciphertext tamper rejects");
        let mut t = sealed.clone();
        let last = t.len() - 1;
        t[last] ^= 1;
        assert_eq!(chacha20poly1305_open(&key, &nonce, aad, &t), None, "tag tamper rejects");
        assert_eq!(chacha20poly1305_open(&key, &nonce, b"wrong-aad", &sealed), None, "AAD mismatch rejects");
    }

    #[test]
    #[ignore = "benchmark — run with --ignored --nocapture"]
    fn bench_aead_vs_rustcrypto() {
        use chacha20poly1305::aead::{Aead, KeyInit, Payload};
        use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
        use std::time::Instant;

        let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(1));
        let nonce: [u8; 12] = std::array::from_fn(|i| i as u8 + 0x10);
        let aad = b"channel-aad";
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
        let n = Nonce::from_slice(&nonce);

        for &len in &[64usize, 1024, 16384] {
            let pt = vec![0x5au8; len];
            let sealed = chacha20poly1305_seal(&key, &nonce, aad, &pt);
            let sealed_o = cipher.encrypt(n, Payload { msg: &pt, aad }).unwrap();
            assert_eq!(sealed, sealed_o, "our AEAD must match RustCrypto for {len}B");

            const ITERS: u32 = 20000;
            // Peak throughput = min time over repeats, so a transient scheduling/thermal stall on a
            // shared box can't masquerade as a slow primitive (both sides measured the same way).
            macro_rules! mbps {
                ($body:expr) => {{
                    for _ in 0..50 { std::hint::black_box($body); }
                    let mut best = f64::INFINITY;
                    for _ in 0..15 {
                        let t = Instant::now();
                        for _ in 0..ITERS { std::hint::black_box($body); }
                        best = best.min(t.elapsed().as_secs_f64());
                    }
                    (len as f64 * ITERS as f64) / best / 1e6
                }};
            }
            let ours = mbps!(chacha20poly1305_seal(&key, &nonce, aad, &pt));
            let theirs = mbps!(cipher.encrypt(n, Payload { msg: &pt, aad }).unwrap());
            eprintln!(
                "seal {:>6}B:  ours {:>8.0} MB/s   RustCrypto {:>8.0} MB/s   ({:.2}x)",
                len, ours, theirs, theirs / ours
            );
        }
    }

    #[test]
    fn aead_matches_rfc8439_2_8_2() {
        let key: [u8; 32] =
            unhex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f")[..32]
                .try_into()
                .unwrap();
        let nonce: [u8; 12] = unhex("070000004041424344454647")[..12].try_into().unwrap();
        let aad = unhex("50515253c0c1c2c3c4c5c6c7");
        let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let sealed = chacha20poly1305_seal(&key, &nonce, &aad, plaintext);
        // RFC 8439 §2.8.2 ciphertext ‖ tag.
        assert_eq!(
            hex(&sealed),
            "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc3ff4def08e4b7a9de576d26586cec64b61161ae10b594f09e26a7e902ecbd0600691",
            "AEAD seal must match RFC 8439 §2.8.2"
        );
        assert_eq!(
            chacha20poly1305_open(&key, &nonce, &aad, &sealed).as_deref(),
            Some(&plaintext[..]),
            "AEAD open round-trips the RFC vector"
        );
    }
}
