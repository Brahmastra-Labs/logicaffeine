//! Cryptographic hash primitives — MD5 (RFC 1321) and SHA-1 (RFC 3174), rolled in pure Rust.
//!
//! The REFERENCE ORACLE. MD5 and SHA-1 (the hashes RFC 9562 defines UUID v3/v5 on) are written in the
//! LOGOS language (`crates/logicaffeine_compile/assets/std/uuid.lg`: `md5Digest`/`sha1Digest`, which the
//! `md5`/`sha1`/`uuid_v3`/`uuid_v5` stdlib functions call) and compile to native through the Futamura
//! pipeline — that is the language's implementation. These pure-Rust versions are the independent oracle
//! the Logos ones are proven byte-exact against (here and cross-tier), NOT on any language path.
//! (SHA-3/Keccak is the modern hash and lives in `logicaffeine_system`.)
//!
//! Both are block functions over 64-byte chunks with the standard Merkle–Damgård padding. They are NOT
//! for security (MD5 and SHA-1 are both broken against collision attacks). Validated bit-exact against
//! the `md-5`/`sha1` reference crates (known RFC vectors + differential fuzz) in the tests.

/// MD5 digest of `data` (RFC 1321) — 16 bytes.
pub fn md5(data: &[u8]) -> [u8; 16] {
    // Per-round left-rotation amounts (RFC 1321 §3.4).
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
        14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
        21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    // K[i] = floor(2^32 · |sin(i+1)|) — the additive constants (RFC 1321 §3.4).
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
        0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
        0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
        0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
        0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
        0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
        0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
    ];

    // The rotation amounts are baked into the unrolled steps below as literals; the linear `S` table
    // is unused by this form.
    let _ = S;

    let mut state: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];

    #[inline(always)]
    fn compress(state: &mut [u32; 4], block: &[u8; 64]) {
        let mut m = [0u32; 16];
        for (i, w) in m.iter_mut().enumerate() {
            *w = u32::from_le_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]);
        }
        let (mut a, mut b, mut c, mut d) = (state[0], state[1], state[2], state[3]);
        // Fully-unrolled rotating-variable form (RFC 1321): instead of moving a=d;d=c;c=b each step,
        // the OUTPUT variable cycles a→d→c→b, so the 192 register-shuffle moves vanish and every
        // g/s/K is a compile-time constant. `op(a,b,c,d,x,s,t): a = b + rotl(a + f(b,c,d) + x + t, s)`.
        macro_rules! ff { ($a:ident,$b:ident,$c:ident,$d:ident,$g:expr,$s:expr,$t:expr) => {
            $a = $b.wrapping_add(($a.wrapping_add(($b & $c) | (!$b & $d)).wrapping_add(m[$g]).wrapping_add($t)).rotate_left($s));
        }}
        macro_rules! gg { ($a:ident,$b:ident,$c:ident,$d:ident,$g:expr,$s:expr,$t:expr) => {
            $a = $b.wrapping_add(($a.wrapping_add(($b & $d) | ($c & !$d)).wrapping_add(m[$g]).wrapping_add($t)).rotate_left($s));
        }}
        macro_rules! hh { ($a:ident,$b:ident,$c:ident,$d:ident,$g:expr,$s:expr,$t:expr) => {
            $a = $b.wrapping_add(($a.wrapping_add($b ^ $c ^ $d).wrapping_add(m[$g]).wrapping_add($t)).rotate_left($s));
        }}
        macro_rules! ii { ($a:ident,$b:ident,$c:ident,$d:ident,$g:expr,$s:expr,$t:expr) => {
            $a = $b.wrapping_add(($a.wrapping_add($c ^ ($b | !$d)).wrapping_add(m[$g]).wrapping_add($t)).rotate_left($s));
        }}
        // Round 1 — g = i.
        ff!(a, b, c, d, 0, 7, K[0]); ff!(d, a, b, c, 1, 12, K[1]); ff!(c, d, a, b, 2, 17, K[2]); ff!(b, c, d, a, 3, 22, K[3]);
        ff!(a, b, c, d, 4, 7, K[4]); ff!(d, a, b, c, 5, 12, K[5]); ff!(c, d, a, b, 6, 17, K[6]); ff!(b, c, d, a, 7, 22, K[7]);
        ff!(a, b, c, d, 8, 7, K[8]); ff!(d, a, b, c, 9, 12, K[9]); ff!(c, d, a, b, 10, 17, K[10]); ff!(b, c, d, a, 11, 22, K[11]);
        ff!(a, b, c, d, 12, 7, K[12]); ff!(d, a, b, c, 13, 12, K[13]); ff!(c, d, a, b, 14, 17, K[14]); ff!(b, c, d, a, 15, 22, K[15]);
        // Round 2 — g = (5i+1) mod 16.
        gg!(a, b, c, d, 1, 5, K[16]); gg!(d, a, b, c, 6, 9, K[17]); gg!(c, d, a, b, 11, 14, K[18]); gg!(b, c, d, a, 0, 20, K[19]);
        gg!(a, b, c, d, 5, 5, K[20]); gg!(d, a, b, c, 10, 9, K[21]); gg!(c, d, a, b, 15, 14, K[22]); gg!(b, c, d, a, 4, 20, K[23]);
        gg!(a, b, c, d, 9, 5, K[24]); gg!(d, a, b, c, 14, 9, K[25]); gg!(c, d, a, b, 3, 14, K[26]); gg!(b, c, d, a, 8, 20, K[27]);
        gg!(a, b, c, d, 13, 5, K[28]); gg!(d, a, b, c, 2, 9, K[29]); gg!(c, d, a, b, 7, 14, K[30]); gg!(b, c, d, a, 12, 20, K[31]);
        // Round 3 — g = (3i+5) mod 16.
        hh!(a, b, c, d, 5, 4, K[32]); hh!(d, a, b, c, 8, 11, K[33]); hh!(c, d, a, b, 11, 16, K[34]); hh!(b, c, d, a, 14, 23, K[35]);
        hh!(a, b, c, d, 1, 4, K[36]); hh!(d, a, b, c, 4, 11, K[37]); hh!(c, d, a, b, 7, 16, K[38]); hh!(b, c, d, a, 10, 23, K[39]);
        hh!(a, b, c, d, 13, 4, K[40]); hh!(d, a, b, c, 0, 11, K[41]); hh!(c, d, a, b, 3, 16, K[42]); hh!(b, c, d, a, 6, 23, K[43]);
        hh!(a, b, c, d, 9, 4, K[44]); hh!(d, a, b, c, 12, 11, K[45]); hh!(c, d, a, b, 15, 16, K[46]); hh!(b, c, d, a, 2, 23, K[47]);
        // Round 4 — g = 7i mod 16.
        ii!(a, b, c, d, 0, 6, K[48]); ii!(d, a, b, c, 7, 10, K[49]); ii!(c, d, a, b, 14, 15, K[50]); ii!(b, c, d, a, 5, 21, K[51]);
        ii!(a, b, c, d, 12, 6, K[52]); ii!(d, a, b, c, 3, 10, K[53]); ii!(c, d, a, b, 10, 15, K[54]); ii!(b, c, d, a, 1, 21, K[55]);
        ii!(a, b, c, d, 8, 6, K[56]); ii!(d, a, b, c, 15, 10, K[57]); ii!(c, d, a, b, 6, 15, K[58]); ii!(b, c, d, a, 13, 21, K[59]);
        ii!(a, b, c, d, 4, 6, K[60]); ii!(d, a, b, c, 11, 10, K[61]); ii!(c, d, a, b, 2, 15, K[62]); ii!(b, c, d, a, 9, 21, K[63]);
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }

    each_padded_block(data, Endian::Little, |block| compress(&mut state, block));

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&state[0].to_le_bytes());
    out[4..8].copy_from_slice(&state[1].to_le_bytes());
    out[8..12].copy_from_slice(&state[2].to_le_bytes());
    out[12..16].copy_from_slice(&state[3].to_le_bytes());
    out
}

/// The 64 MD5 additive constants `floor(2^32·|sin(i+1)|)` and per-round rotation amounts (RFC 1321).
const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];
const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14,
    20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6,
    10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// MD5 of FOUR equal-length messages at once — 4-way SSE2 multi-buffer, the high-throughput path for
/// hashing many same-size records (bulk name-based ids, dedup keys, content addresses). All four ABCD
/// states advance in one `__m128i` (lane j = message j), so ~4× a scalar MD5 per message. The four
/// inputs MUST share a length; returns the four 16-byte digests, byte-identical to [`md5`] per lane.
#[cfg(target_arch = "x86_64")]
pub fn md5_x4(msgs: [&[u8]; 4]) -> [[u8; 16]; 4] {
    let len = msgs[0].len();
    assert!(msgs.iter().all(|m| m.len() == len), "md5_x4 requires equal-length inputs");
    if std::is_x86_feature_detected!("sse2") {
        // SAFETY: guarded by the sse2 feature detection just above.
        unsafe { md5_x4_sse2(msgs, len) }
    } else {
        [md5(msgs[0]), md5(msgs[1]), md5(msgs[2]), md5(msgs[3])]
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn md5_x4_sse2(msgs: [&[u8]; 4], len: usize) -> [[u8; 16]; 4] {
    // Identical length ⇒ identical Merkle–Damgård padding (0x80, zero fill, 64-bit LE bit length) and
    // block count for all four lanes. A 256-byte inline buffer per lane keeps the common case (records
    // up to 247 bytes) entirely on the stack — no per-call heap; longer inputs fall back to a Vec.
    let total = (len + 8) / 64 * 64 + 64;
    let nb = total / 64;
    let bitlen = (len as u64).wrapping_mul(8);
    const CAP: usize = 256;
    if total <= CAP {
        let mut pad = [[0u8; CAP]; 4];
        for (j, p) in pad.iter_mut().enumerate() {
            p[..len].copy_from_slice(msgs[j]);
            p[len] = 0x80;
            p[total - 8..total].copy_from_slice(&bitlen.to_le_bytes());
        }
        md5_x4_run([&pad[0][..], &pad[1][..], &pad[2][..], &pad[3][..]], nb)
    } else {
        let mut pad = [vec![0u8; total], vec![0u8; total], vec![0u8; total], vec![0u8; total]];
        for (j, p) in pad.iter_mut().enumerate() {
            p[..len].copy_from_slice(msgs[j]);
            p[len] = 0x80;
            p[total - 8..].copy_from_slice(&bitlen.to_le_bytes());
        }
        md5_x4_run([&pad[0][..], &pad[1][..], &pad[2][..], &pad[3][..]], nb)
    }
}

/// The 4-way SSE2 MD5 compress + transpose over the four (already padded) lane buffers — the alloc-free
/// core shared by both the stack and heap staging paths of [`md5_x4_sse2`].
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn md5_x4_run(pad: [&[u8]; 4], nb: usize) -> [[u8; 16]; 4] {
    use core::arch::x86_64::*;

    let rotl = |x: __m128i, s: u32| -> __m128i {
        _mm_or_si128(
            _mm_sll_epi32(x, _mm_cvtsi32_si128(s as i32)),
            _mm_srl_epi32(x, _mm_cvtsi32_si128((32 - s) as i32)),
        )
    };
    let ones = _mm_set1_epi32(-1);

    let mut sa = _mm_set1_epi32(0x67452301u32 as i32);
    let mut sb = _mm_set1_epi32(0xefcdab89u32 as i32);
    let mut sc = _mm_set1_epi32(0x98badcfeu32 as i32);
    let mut sd = _mm_set1_epi32(0x10325476u32 as i32);

    for bi in 0..nb {
        // Transpose the four lanes' 16 message words: mw[g] = (msg0[g], msg1[g], msg2[g], msg3[g]).
        let mut mw = [_mm_setzero_si128(); 16];
        for (g, slot) in mw.iter_mut().enumerate() {
            let o = bi * 64 + g * 4;
            let w = |p: &[u8]| u32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]) as i32;
            *slot = _mm_setr_epi32(w(&pad[0]), w(&pad[1]), w(&pad[2]), w(&pad[3]));
        }
        let (mut a, mut b, mut c, mut d) = (sa, sb, sc, sd);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => (_mm_or_si128(_mm_and_si128(b, c), _mm_andnot_si128(b, d)), i),
                1 => (_mm_or_si128(_mm_and_si128(b, d), _mm_andnot_si128(d, c)), (5 * i + 1) % 16),
                2 => (_mm_xor_si128(_mm_xor_si128(b, c), d), (3 * i + 5) % 16),
                _ => (_mm_xor_si128(c, _mm_or_si128(b, _mm_xor_si128(d, ones))), (7 * i) % 16),
            };
            let t = _mm_add_epi32(
                _mm_add_epi32(a, f),
                _mm_add_epi32(mw[g], _mm_set1_epi32(MD5_K[i] as i32)),
            );
            a = d;
            d = c;
            c = b;
            b = _mm_add_epi32(b, rotl(t, MD5_S[i]));
        }
        sa = _mm_add_epi32(sa, a);
        sb = _mm_add_epi32(sb, b);
        sc = _mm_add_epi32(sc, c);
        sd = _mm_add_epi32(sd, d);
    }

    let mut a4 = [0u32; 4];
    let mut b4 = [0u32; 4];
    let mut c4 = [0u32; 4];
    let mut d4 = [0u32; 4];
    _mm_storeu_si128(a4.as_mut_ptr() as *mut __m128i, sa);
    _mm_storeu_si128(b4.as_mut_ptr() as *mut __m128i, sb);
    _mm_storeu_si128(c4.as_mut_ptr() as *mut __m128i, sc);
    _mm_storeu_si128(d4.as_mut_ptr() as *mut __m128i, sd);
    let mut out = [[0u8; 16]; 4];
    for (j, o) in out.iter_mut().enumerate() {
        o[0..4].copy_from_slice(&a4[j].to_le_bytes());
        o[4..8].copy_from_slice(&b4[j].to_le_bytes());
        o[8..12].copy_from_slice(&c4[j].to_le_bytes());
        o[12..16].copy_from_slice(&d4[j].to_le_bytes());
    }
    out
}

/// MD5 of EIGHT equal-length messages at once — AVX2 8-way multi-buffer, twice the width of [`md5_x4`].
/// All eight ABCD states advance in one `__m256i` (lane j = message j). Falls back to two `md5_x4`
/// passes without AVX2. Byte-identical to [`md5`] per lane; the high-throughput path for hashing many
/// same-size records (bulk name-based ids, dedup/content keys).
#[cfg(target_arch = "x86_64")]
pub fn md5_x8(msgs: [&[u8]; 8]) -> [[u8; 16]; 8] {
    let len = msgs[0].len();
    assert!(msgs.iter().all(|m| m.len() == len), "md5_x8 requires equal-length inputs");
    if std::is_x86_feature_detected!("avx2") {
        // SAFETY: guarded by the avx2 feature detection just above.
        unsafe { md5_x8_avx2(msgs, len) }
    } else {
        let a = md5_x4([msgs[0], msgs[1], msgs[2], msgs[3]]);
        let b = md5_x4([msgs[4], msgs[5], msgs[6], msgs[7]]);
        [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]]
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn md5_x8_avx2(msgs: [&[u8]; 8], len: usize) -> [[u8; 16]; 8] {
    let total = (len + 8) / 64 * 64 + 64;
    let nb = total / 64;
    let bitlen = (len as u64).wrapping_mul(8);
    const CAP: usize = 256;
    if total <= CAP {
        let mut pad = [[0u8; CAP]; 8];
        for (j, p) in pad.iter_mut().enumerate() {
            p[..len].copy_from_slice(msgs[j]);
            p[len] = 0x80;
            p[total - 8..total].copy_from_slice(&bitlen.to_le_bytes());
        }
        let s: [&[u8]; 8] =
            [&pad[0], &pad[1], &pad[2], &pad[3], &pad[4], &pad[5], &pad[6], &pad[7]];
        md5_x8_run(s, nb)
    } else {
        let mut pad: Vec<Vec<u8>> = (0..8).map(|_| vec![0u8; total]).collect();
        for (j, p) in pad.iter_mut().enumerate() {
            p[..len].copy_from_slice(msgs[j]);
            p[len] = 0x80;
            p[total - 8..].copy_from_slice(&bitlen.to_le_bytes());
        }
        let s: [&[u8]; 8] = [
            &pad[0], &pad[1], &pad[2], &pad[3], &pad[4], &pad[5], &pad[6], &pad[7],
        ];
        md5_x8_run(s, nb)
    }
}

/// Transpose eight `__m256i` rows (8 lanes each) — the textbook AVX2 8×8 32-bit transpose (unpack
/// `epi32` → unpack `epi64` → `permute2x128`). `out[k]` is column `k` = `(r0[k], r1[k], …, r7[k])`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn transpose8x8(r: [core::arch::x86_64::__m256i; 8]) -> [core::arch::x86_64::__m256i; 8] {
    use core::arch::x86_64::*;
    let t0 = _mm256_unpacklo_epi32(r[0], r[1]);
    let t1 = _mm256_unpackhi_epi32(r[0], r[1]);
    let t2 = _mm256_unpacklo_epi32(r[2], r[3]);
    let t3 = _mm256_unpackhi_epi32(r[2], r[3]);
    let t4 = _mm256_unpacklo_epi32(r[4], r[5]);
    let t5 = _mm256_unpackhi_epi32(r[4], r[5]);
    let t6 = _mm256_unpacklo_epi32(r[6], r[7]);
    let t7 = _mm256_unpackhi_epi32(r[6], r[7]);
    let s0 = _mm256_unpacklo_epi64(t0, t2);
    let s1 = _mm256_unpackhi_epi64(t0, t2);
    let s2 = _mm256_unpacklo_epi64(t1, t3);
    let s3 = _mm256_unpackhi_epi64(t1, t3);
    let s4 = _mm256_unpacklo_epi64(t4, t6);
    let s5 = _mm256_unpackhi_epi64(t4, t6);
    let s6 = _mm256_unpacklo_epi64(t5, t7);
    let s7 = _mm256_unpackhi_epi64(t5, t7);
    [
        _mm256_permute2x128_si256(s0, s4, 0x20),
        _mm256_permute2x128_si256(s1, s5, 0x20),
        _mm256_permute2x128_si256(s2, s6, 0x20),
        _mm256_permute2x128_si256(s3, s7, 0x20),
        _mm256_permute2x128_si256(s0, s4, 0x31),
        _mm256_permute2x128_si256(s1, s5, 0x31),
        _mm256_permute2x128_si256(s2, s6, 0x31),
        _mm256_permute2x128_si256(s3, s7, 0x31),
    ]
}

/// The 8-way AVX2 MD5 compress + transpose over the eight (already padded) lane buffers.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn md5_x8_run(pad: [&[u8]; 8], nb: usize) -> [[u8; 16]; 8] {
    use core::arch::x86_64::*;

    let rotl = |x: __m256i, s: u32| -> __m256i {
        _mm256_or_si256(
            _mm256_sll_epi32(x, _mm_cvtsi32_si128(s as i32)),
            _mm256_srl_epi32(x, _mm_cvtsi32_si128((32 - s) as i32)),
        )
    };
    let ones = _mm256_set1_epi32(-1);

    let mut sa = _mm256_set1_epi32(0x67452301u32 as i32);
    let mut sb = _mm256_set1_epi32(0xefcdab89u32 as i32);
    let mut sc = _mm256_set1_epi32(0x98badcfeu32 as i32);
    let mut sd = _mm256_set1_epi32(0x10325476u32 as i32);

    for bi in 0..nb {
        // Register transpose: load each lane's block as two rows (words 0–7 and 8–15) and transpose
        // the 8×8 so `mw[g]` = word `g` of all eight messages — SIMD shuffles, not 128 scalar loads.
        let base = bi * 64;
        let ld = |p: &[u8], off: usize| _mm256_loadu_si256(p.as_ptr().add(base + off) as *const __m256i);
        let lo = transpose8x8([
            ld(pad[0], 0), ld(pad[1], 0), ld(pad[2], 0), ld(pad[3], 0),
            ld(pad[4], 0), ld(pad[5], 0), ld(pad[6], 0), ld(pad[7], 0),
        ]);
        let hi = transpose8x8([
            ld(pad[0], 32), ld(pad[1], 32), ld(pad[2], 32), ld(pad[3], 32),
            ld(pad[4], 32), ld(pad[5], 32), ld(pad[6], 32), ld(pad[7], 32),
        ]);
        let mut mw = [_mm256_setzero_si256(); 16];
        mw[..8].copy_from_slice(&lo);
        mw[8..].copy_from_slice(&hi);
        let (mut a, mut b, mut c, mut d) = (sa, sb, sc, sd);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => (_mm256_or_si256(_mm256_and_si256(b, c), _mm256_andnot_si256(b, d)), i),
                1 => (_mm256_or_si256(_mm256_and_si256(b, d), _mm256_andnot_si256(d, c)), (5 * i + 1) % 16),
                2 => (_mm256_xor_si256(_mm256_xor_si256(b, c), d), (3 * i + 5) % 16),
                _ => (_mm256_xor_si256(c, _mm256_or_si256(b, _mm256_xor_si256(d, ones))), (7 * i) % 16),
            };
            let t = _mm256_add_epi32(
                _mm256_add_epi32(a, f),
                _mm256_add_epi32(mw[g], _mm256_set1_epi32(MD5_K[i] as i32)),
            );
            a = d;
            d = c;
            c = b;
            b = _mm256_add_epi32(b, rotl(t, MD5_S[i]));
        }
        sa = _mm256_add_epi32(sa, a);
        sb = _mm256_add_epi32(sb, b);
        sc = _mm256_add_epi32(sc, c);
        sd = _mm256_add_epi32(sd, d);
    }

    let mut a8 = [0u32; 8];
    let mut b8 = [0u32; 8];
    let mut c8 = [0u32; 8];
    let mut d8 = [0u32; 8];
    _mm256_storeu_si256(a8.as_mut_ptr() as *mut __m256i, sa);
    _mm256_storeu_si256(b8.as_mut_ptr() as *mut __m256i, sb);
    _mm256_storeu_si256(c8.as_mut_ptr() as *mut __m256i, sc);
    _mm256_storeu_si256(d8.as_mut_ptr() as *mut __m256i, sd);
    let mut out = [[0u8; 16]; 8];
    for (j, o) in out.iter_mut().enumerate() {
        o[0..4].copy_from_slice(&a8[j].to_le_bytes());
        o[4..8].copy_from_slice(&b8[j].to_le_bytes());
        o[8..12].copy_from_slice(&c8[j].to_le_bytes());
        o[12..16].copy_from_slice(&d8[j].to_le_bytes());
    }
    out
}

/// SHA-1 digest of `data` (RFC 3174) — 20 bytes.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];

    #[inline(always)]
    fn compress(h: &mut [u32; 5], block: &[u8; 64]) {
        // Decode and extend the message schedule to 80 big-endian words.
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    // Fast path: SHA-NI hardware instructions (≈ an order of magnitude faster than scalar). Gated on
    // a one-shot runtime CPU-feature check; falls back to the scalar `compress` everywhere else.
    #[cfg(target_arch = "x86_64")]
    {
        if x86_sha::available() {
            // SAFETY: guarded by the `sha`/`ssse3`/`sse4.1` feature detection just checked.
            unsafe { x86_sha::sha1_blocks(data, &mut h) };
            let mut out = [0u8; 20];
            for (i, hi) in h.iter().enumerate() {
                out[4 * i..4 * i + 4].copy_from_slice(&hi.to_be_bytes());
            }
            return out;
        }
    }

    each_padded_block(data, Endian::Big, |block| compress(&mut h, block));

    let mut out = [0u8; 20];
    for (i, hi) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&hi.to_be_bytes());
    }
    out
}

#[derive(Clone, Copy)]
enum Endian {
    Little,
    Big,
}

/// Drive `compress` over the Merkle–Damgård padded message — ALLOC-FREE. Full 64-byte blocks are
/// fed directly from `data` (no copy); only the final partial block plus the `0x80`/zero/length
/// padding is staged in a 128-byte stack buffer (the padding spills to a second block when the tail
/// is ≥ 56 bytes). The bit length is little-endian for MD5, big-endian for SHA-1. This keeps the hot
/// path — small UUID inputs — entirely on the stack.
#[inline(always)]
fn each_padded_block(data: &[u8], endian: Endian, mut compress: impl FnMut(&[u8; 64])) {
    let mut chunks = data.chunks_exact(64);
    for chunk in &mut chunks {
        let mut block = [0u8; 64];
        block.copy_from_slice(chunk);
        compress(&block);
    }
    let rem = chunks.remainder();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let len_bytes = match endian {
        Endian::Little => bit_len.to_le_bytes(),
        Endian::Big => bit_len.to_be_bytes(),
    };

    let mut tail = [0u8; 128];
    tail[..rem.len()].copy_from_slice(rem);
    tail[rem.len()] = 0x80;
    if rem.len() < 56 {
        // Padding + length fit in one final block.
        tail[56..64].copy_from_slice(&len_bytes);
        let mut b = [0u8; 64];
        b.copy_from_slice(&tail[..64]);
        compress(&b);
    } else {
        // Spills to a second block; the length goes at the very end.
        tail[120..128].copy_from_slice(&len_bytes);
        let mut b0 = [0u8; 64];
        b0.copy_from_slice(&tail[..64]);
        compress(&b0);
        let mut b1 = [0u8; 64];
        b1.copy_from_slice(&tail[64..128]);
        compress(&b1);
    }
}

/// Lowercase hex of a digest — handy for tests and debugging.
pub fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

/// SHA-1 over Intel SHA extensions (SHA-NI). Beats the scalar core by ~10× by running the round
/// function in dedicated silicon (`_mm_sha1rnds4` etc.). Selected at runtime; the scalar core is the
/// fallback. The instruction sequence is the canonical Intel reference (validated bit-exact against
/// the `sha1` crate + RFC vectors in the tests).
#[cfg(target_arch = "x86_64")]
mod x86_sha {
    use super::{each_padded_block, Endian};

    /// True when the CPU has the SHA + SSSE3 + SSE4.1 instructions this path needs. `std`'s detection
    /// caches the result, so this is a cheap load after the first call.
    #[inline]
    pub fn available() -> bool {
        std::is_x86_feature_detected!("sha")
            && std::is_x86_feature_detected!("ssse3")
            && std::is_x86_feature_detected!("sse4.1")
    }

    /// Hash every (padded) block of `data` into `h`. Caller must ensure [`available`].
    #[target_feature(enable = "sha,sse2,ssse3,sse4.1")]
    pub unsafe fn sha1_blocks(data: &[u8], h: &mut [u32; 5]) {
        each_padded_block(data, Endian::Big, |block| compress(h, block));
    }

    /// One 64-byte block through the SHA-NI round pipeline (Intel's `sha1_process_x86`).
    #[target_feature(enable = "sha,sse2,ssse3,sse4.1")]
    unsafe fn compress(state: &mut [u32; 5], block: &[u8; 64]) {
        use core::arch::x86_64::*;

        // Full 16-byte reversal: puts big-endian W0 in lane 3, aligned with A (lane 3) and the carried
        // E (lane 3). E is NOT shuffled — `set_epi32(e,0,0,0)` already places it in lane 3.
        let mask = _mm_set_epi64x(0x0001_0203_0405_0607, 0x0809_0a0b_0c0d_0e0fu64 as i64);
        let mut abcd = _mm_shuffle_epi32(_mm_loadu_si128(state.as_ptr() as *const __m128i), 0x1B);
        let mut e0 = _mm_set_epi32(state[4] as i32, 0, 0, 0);
        let abcd_save = abcd;
        let e0_save = e0;
        let p = block.as_ptr();
        let mut msg0 = _mm_shuffle_epi8(_mm_loadu_si128(p as *const __m128i), mask);
        let mut msg1 = _mm_shuffle_epi8(_mm_loadu_si128(p.add(16) as *const __m128i), mask);
        let mut msg2 = _mm_shuffle_epi8(_mm_loadu_si128(p.add(32) as *const __m128i), mask);
        let mut msg3 = _mm_shuffle_epi8(_mm_loadu_si128(p.add(48) as *const __m128i), mask);
        let mut e1;

        // Rounds 0–3
        e0 = _mm_add_epi32(e0, msg0);
        e1 = abcd;
        abcd = _mm_sha1rnds4_epu32(abcd, e0, 0);
        // 4–7
        e1 = _mm_sha1nexte_epu32(e1, msg1);
        e0 = abcd;
        abcd = _mm_sha1rnds4_epu32(abcd, e1, 0);
        msg0 = _mm_sha1msg1_epu32(msg0, msg1);
        // 8–11
        e0 = _mm_sha1nexte_epu32(e0, msg2);
        e1 = abcd;
        abcd = _mm_sha1rnds4_epu32(abcd, e0, 0);
        msg1 = _mm_sha1msg1_epu32(msg1, msg2);
        msg0 = _mm_xor_si128(msg0, msg2);
        // 12–15
        e1 = _mm_sha1nexte_epu32(e1, msg3);
        e0 = abcd;
        msg0 = _mm_sha1msg2_epu32(msg0, msg3);
        abcd = _mm_sha1rnds4_epu32(abcd, e1, 0);
        msg2 = _mm_sha1msg1_epu32(msg2, msg3);
        msg1 = _mm_xor_si128(msg1, msg3);
        // 16–19
        e0 = _mm_sha1nexte_epu32(e0, msg0);
        e1 = abcd;
        msg1 = _mm_sha1msg2_epu32(msg1, msg0);
        abcd = _mm_sha1rnds4_epu32(abcd, e0, 0);
        msg3 = _mm_sha1msg1_epu32(msg3, msg0);
        msg2 = _mm_xor_si128(msg2, msg0);
        // 20–23
        e1 = _mm_sha1nexte_epu32(e1, msg1);
        e0 = abcd;
        msg2 = _mm_sha1msg2_epu32(msg2, msg1);
        abcd = _mm_sha1rnds4_epu32(abcd, e1, 1);
        msg0 = _mm_sha1msg1_epu32(msg0, msg1);
        msg3 = _mm_xor_si128(msg3, msg1);
        // 24–27
        e0 = _mm_sha1nexte_epu32(e0, msg2);
        e1 = abcd;
        msg3 = _mm_sha1msg2_epu32(msg3, msg2);
        abcd = _mm_sha1rnds4_epu32(abcd, e0, 1);
        msg1 = _mm_sha1msg1_epu32(msg1, msg2);
        msg0 = _mm_xor_si128(msg0, msg2);
        // 28–31
        e1 = _mm_sha1nexte_epu32(e1, msg3);
        e0 = abcd;
        msg0 = _mm_sha1msg2_epu32(msg0, msg3);
        abcd = _mm_sha1rnds4_epu32(abcd, e1, 1);
        msg2 = _mm_sha1msg1_epu32(msg2, msg3);
        msg1 = _mm_xor_si128(msg1, msg3);
        // 32–35
        e0 = _mm_sha1nexte_epu32(e0, msg0);
        e1 = abcd;
        msg1 = _mm_sha1msg2_epu32(msg1, msg0);
        abcd = _mm_sha1rnds4_epu32(abcd, e0, 1);
        msg3 = _mm_sha1msg1_epu32(msg3, msg0);
        msg2 = _mm_xor_si128(msg2, msg0);
        // 36–39
        e1 = _mm_sha1nexte_epu32(e1, msg1);
        e0 = abcd;
        msg2 = _mm_sha1msg2_epu32(msg2, msg1);
        abcd = _mm_sha1rnds4_epu32(abcd, e1, 1);
        msg0 = _mm_sha1msg1_epu32(msg0, msg1);
        msg3 = _mm_xor_si128(msg3, msg1);
        // 40–43
        e0 = _mm_sha1nexte_epu32(e0, msg2);
        e1 = abcd;
        msg3 = _mm_sha1msg2_epu32(msg3, msg2);
        abcd = _mm_sha1rnds4_epu32(abcd, e0, 2);
        msg1 = _mm_sha1msg1_epu32(msg1, msg2);
        msg0 = _mm_xor_si128(msg0, msg2);
        // 44–47
        e1 = _mm_sha1nexte_epu32(e1, msg3);
        e0 = abcd;
        msg0 = _mm_sha1msg2_epu32(msg0, msg3);
        abcd = _mm_sha1rnds4_epu32(abcd, e1, 2);
        msg2 = _mm_sha1msg1_epu32(msg2, msg3);
        msg1 = _mm_xor_si128(msg1, msg3);
        // 48–51
        e0 = _mm_sha1nexte_epu32(e0, msg0);
        e1 = abcd;
        msg1 = _mm_sha1msg2_epu32(msg1, msg0);
        abcd = _mm_sha1rnds4_epu32(abcd, e0, 2);
        msg3 = _mm_sha1msg1_epu32(msg3, msg0);
        msg2 = _mm_xor_si128(msg2, msg0);
        // 52–55
        e1 = _mm_sha1nexte_epu32(e1, msg1);
        e0 = abcd;
        msg2 = _mm_sha1msg2_epu32(msg2, msg1);
        abcd = _mm_sha1rnds4_epu32(abcd, e1, 2);
        msg0 = _mm_sha1msg1_epu32(msg0, msg1);
        msg3 = _mm_xor_si128(msg3, msg1);
        // 56–59
        e0 = _mm_sha1nexte_epu32(e0, msg2);
        e1 = abcd;
        msg3 = _mm_sha1msg2_epu32(msg3, msg2);
        abcd = _mm_sha1rnds4_epu32(abcd, e0, 2);
        msg1 = _mm_sha1msg1_epu32(msg1, msg2);
        msg0 = _mm_xor_si128(msg0, msg2);
        // 60–63
        e1 = _mm_sha1nexte_epu32(e1, msg3);
        e0 = abcd;
        msg0 = _mm_sha1msg2_epu32(msg0, msg3);
        abcd = _mm_sha1rnds4_epu32(abcd, e1, 3);
        msg2 = _mm_sha1msg1_epu32(msg2, msg3);
        msg1 = _mm_xor_si128(msg1, msg3);
        // 64–67
        e0 = _mm_sha1nexte_epu32(e0, msg0);
        e1 = abcd;
        msg1 = _mm_sha1msg2_epu32(msg1, msg0);
        abcd = _mm_sha1rnds4_epu32(abcd, e0, 3);
        msg3 = _mm_sha1msg1_epu32(msg3, msg0);
        msg2 = _mm_xor_si128(msg2, msg0);
        // 68–71
        e1 = _mm_sha1nexte_epu32(e1, msg1);
        e0 = abcd;
        msg2 = _mm_sha1msg2_epu32(msg2, msg1);
        abcd = _mm_sha1rnds4_epu32(abcd, e1, 3);
        msg3 = _mm_xor_si128(msg3, msg1);
        // 72–75
        e0 = _mm_sha1nexte_epu32(e0, msg2);
        e1 = abcd;
        msg3 = _mm_sha1msg2_epu32(msg3, msg2);
        abcd = _mm_sha1rnds4_epu32(abcd, e0, 3);
        // 76–79
        e1 = _mm_sha1nexte_epu32(e1, msg3);
        e0 = abcd;
        abcd = _mm_sha1rnds4_epu32(abcd, e1, 3);

        // Fold this block's result back into the running state.
        e0 = _mm_sha1nexte_epu32(e0, e0_save);
        abcd = _mm_add_epi32(abcd, abcd_save);

        abcd = _mm_shuffle_epi32(abcd, 0x1B);
        _mm_storeu_si128(state.as_mut_ptr() as *mut __m128i, abcd);
        state[4] = _mm_extract_epi32(e0, 3) as u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_known_rfc_vectors() {
        assert_eq!(to_hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(to_hex(&md5(b"a")), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(to_hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(to_hex(&md5(b"message digest")), "f96b697d7cb7938d525a2f31aaf161d0");
        assert_eq!(
            to_hex(&md5(b"abcdefghijklmnopqrstuvwxyz")),
            "c3fcd3d76192e4007dfb496cca67e13b"
        );
        assert_eq!(
            to_hex(&md5(b"The quick brown fox jumps over the lazy dog")),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
    }

    #[test]
    fn sha1_known_rfc_vectors() {
        assert_eq!(to_hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(to_hex(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            to_hex(&sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(
            to_hex(&sha1(b"The quick brown fox jumps over the lazy dog")),
            "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"
        );
    }

    #[test]
    fn md5_block_boundaries_are_exact() {
        // Lengths straddling the 55/56/64/119/120-byte padding edges — the cases padding bugs hide in.
        for n in [0usize, 1, 54, 55, 56, 57, 63, 64, 65, 119, 120, 127, 128, 256, 1000] {
            let data = vec![0xABu8; n];
            let mut oracle = <md5::Md5 as md5::Digest>::new();
            md5::Digest::update(&mut oracle, &data);
            let want: [u8; 16] = md5::Digest::finalize(oracle).into();
            assert_eq!(md5(&data), want, "md5 mismatch at len {n}");
        }
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn md5_x4_matches_scalar_over_lengths() {
        // The 4-way SIMD multi-buffer MD5 must equal the scalar MD5 on every lane, across lengths that
        // straddle the block and one/two-block padding edges (55/56/57, 63/64/65, 119/120).
        for len in [0usize, 1, 3, 16, 32, 55, 56, 57, 63, 64, 65, 119, 120, 128, 200] {
            let mut msgs = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
            let mut st = 0x1234_5678u64.wrapping_add(len as u64);
            for m in msgs.iter_mut() {
                for _ in 0..len {
                    st = st.wrapping_mul(6364136223846793005).wrapping_add(1);
                    m.push((st >> 56) as u8);
                }
            }
            let refs: [&[u8]; 4] = [&msgs[0], &msgs[1], &msgs[2], &msgs[3]];
            let x4 = md5_x4(refs);
            for (j, m) in msgs.iter().enumerate() {
                assert_eq!(x4[j], md5(m), "md5_x4 lane {j} differs at len {len}");
            }
        }
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn md5_x8_matches_scalar_over_lengths() {
        // The 8-way AVX2 multi-buffer MD5 must equal the scalar MD5 on every lane, across the same
        // block/padding edges the 4-way is checked at.
        for len in [0usize, 1, 3, 16, 32, 55, 56, 57, 63, 64, 65, 119, 120, 128, 200] {
            let mut msgs: Vec<Vec<u8>> = (0..8).map(|_| Vec::new()).collect();
            let mut st = 0x9e37_79b9u64.wrapping_add(len as u64);
            for m in msgs.iter_mut() {
                for _ in 0..len {
                    st = st.wrapping_mul(6364136223846793005).wrapping_add(1);
                    m.push((st >> 56) as u8);
                }
            }
            let refs: [&[u8]; 8] = [
                &msgs[0], &msgs[1], &msgs[2], &msgs[3], &msgs[4], &msgs[5], &msgs[6], &msgs[7],
            ];
            let x8 = md5_x8(refs);
            for (j, m) in msgs.iter().enumerate() {
                assert_eq!(x8[j], md5(m), "md5_x8 lane {j} differs at len {len}");
            }
        }
    }

    #[test]
    fn sha1_block_boundaries_are_exact() {
        for n in [0usize, 1, 54, 55, 56, 57, 63, 64, 65, 119, 120, 127, 128, 256, 1000] {
            let data = vec![0x5Au8; n];
            let mut oracle = <sha1::Sha1 as sha1::Digest>::new();
            sha1::Digest::update(&mut oracle, &data);
            let want: [u8; 20] = sha1::Digest::finalize(oracle).into();
            assert_eq!(sha1(&data), want, "sha1 mismatch at len {n}");
        }
    }

    #[test]
    fn differential_fuzz_against_the_reference_crates() {
        // A cheap deterministic LCG drives 5000 random-length, random-content inputs; both digests
        // must agree with the reference crates bit-for-bit. (No external RNG — reproducible.)
        let mut state: u64 = 0x1234_5678_9abc_def0;
        let mut next = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as u32
        };
        for _ in 0..5000 {
            let len = (next() % 600) as usize;
            let data: Vec<u8> = (0..len).map(|_| (next() & 0xff) as u8).collect();

            let mut m = <md5::Md5 as md5::Digest>::new();
            md5::Digest::update(&mut m, &data);
            let m_want: [u8; 16] = md5::Digest::finalize(m).into();
            assert_eq!(md5(&data), m_want, "md5 differs at len {len}");

            let mut s = <sha1::Sha1 as sha1::Digest>::new();
            sha1::Digest::update(&mut s, &data);
            let s_want: [u8; 20] = sha1::Digest::finalize(s).into();
            assert_eq!(sha1(&data), s_want, "sha1 differs at len {len}");
        }
    }
}
