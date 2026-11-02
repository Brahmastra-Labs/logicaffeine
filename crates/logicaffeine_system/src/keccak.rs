//! Keccak-f\[1600\] + SHA-3 / SHAKE runtime kernels — the symmetric/hash layer ML-KEM is built on
//! (SHA3-256/512 for hashing, SHAKE128 for matrix expansion, SHAKE256 for the PRF). Reached from
//! compiled LOGOS via the `sha3_256` / `sha3_512` / `shake128` / `shake256` stdlib functions.
//! Verified against the NIST FIPS-202 KATs (see the tests). Input/output are LOGOS `Int` bytes
//! (0..255) carried in a `Seq of Int`.

use crate::kernel_args as args;

const RC: [u64; 24] = [
    0x0000000000000001,
    0x0000000000008082,
    0x800000000000808a,
    0x8000000080008000,
    0x000000000000808b,
    0x0000000080000001,
    0x8000000080008081,
    0x8000000000008009,
    0x000000000000008a,
    0x0000000000000088,
    0x0000000080008009,
    0x000000008000000a,
    0x000000008000808b,
    0x800000000000008b,
    0x8000000000008089,
    0x8000000000008003,
    0x8000000000008002,
    0x8000000000000080,
    0x000000000000800a,
    0x800000008000000a,
    0x8000000080008081,
    0x8000000000008080,
    0x0000000080000001,
    0x8000000080008008,
];
/// The Keccak-f\[1600\] permutation: 24 rounds of θ, ρ·π, χ, ι over the 5×5×64 state.
pub fn keccak_f1600(st: &mut [u64; 25]) {
    // Fully unrolled into 25 register-resident lanes — no array indexing, bounds checks, or PILN/
    // ROTC lookups in the hot loop (the proven-fast scalar form). θ, ρ·π (combined into the `b`
    // lanes), χ, ι. Bit-identical to the table-driven reference (NIST FIPS-202 KAT-verified).
    let [mut a0, mut a1, mut a2, mut a3, mut a4, mut a5, mut a6, mut a7, mut a8, mut a9, mut a10, mut a11, mut a12, mut a13, mut a14, mut a15, mut a16, mut a17, mut a18, mut a19, mut a20, mut a21, mut a22, mut a23, mut a24] =
        *st;
    for &rc in RC.iter() {
        // θ
        let c0 = a0 ^ a5 ^ a10 ^ a15 ^ a20;
        let c1 = a1 ^ a6 ^ a11 ^ a16 ^ a21;
        let c2 = a2 ^ a7 ^ a12 ^ a17 ^ a22;
        let c3 = a3 ^ a8 ^ a13 ^ a18 ^ a23;
        let c4 = a4 ^ a9 ^ a14 ^ a19 ^ a24;
        let d0 = c4 ^ c1.rotate_left(1);
        let d1 = c0 ^ c2.rotate_left(1);
        let d2 = c1 ^ c3.rotate_left(1);
        let d3 = c2 ^ c4.rotate_left(1);
        let d4 = c3 ^ c0.rotate_left(1);
        a0 ^= d0;
        a5 ^= d0;
        a10 ^= d0;
        a15 ^= d0;
        a20 ^= d0;
        a1 ^= d1;
        a6 ^= d1;
        a11 ^= d1;
        a16 ^= d1;
        a21 ^= d1;
        a2 ^= d2;
        a7 ^= d2;
        a12 ^= d2;
        a17 ^= d2;
        a22 ^= d2;
        a3 ^= d3;
        a8 ^= d3;
        a13 ^= d3;
        a18 ^= d3;
        a23 ^= d3;
        a4 ^= d4;
        a9 ^= d4;
        a14 ^= d4;
        a19 ^= d4;
        a24 ^= d4;
        // ρ + π: b[π(i)] = rotl(a[i], ρ[i])
        let b0 = a0;
        let b1 = a6.rotate_left(44);
        let b2 = a12.rotate_left(43);
        let b3 = a18.rotate_left(21);
        let b4 = a24.rotate_left(14);
        let b5 = a3.rotate_left(28);
        let b6 = a9.rotate_left(20);
        let b7 = a10.rotate_left(3);
        let b8 = a16.rotate_left(45);
        let b9 = a22.rotate_left(61);
        let b10 = a1.rotate_left(1);
        let b11 = a7.rotate_left(6);
        let b12 = a13.rotate_left(25);
        let b13 = a19.rotate_left(8);
        let b14 = a20.rotate_left(18);
        let b15 = a4.rotate_left(27);
        let b16 = a5.rotate_left(36);
        let b17 = a11.rotate_left(10);
        let b18 = a17.rotate_left(15);
        let b19 = a23.rotate_left(56);
        let b20 = a2.rotate_left(62);
        let b21 = a8.rotate_left(55);
        let b22 = a14.rotate_left(39);
        let b23 = a15.rotate_left(41);
        let b24 = a21.rotate_left(2);
        // χ (row-wise: a[i] = b[i] ^ (¬b[i+1] ∧ b[i+2]))
        a0 = b0 ^ (!b1 & b2);
        a1 = b1 ^ (!b2 & b3);
        a2 = b2 ^ (!b3 & b4);
        a3 = b3 ^ (!b4 & b0);
        a4 = b4 ^ (!b0 & b1);
        a5 = b5 ^ (!b6 & b7);
        a6 = b6 ^ (!b7 & b8);
        a7 = b7 ^ (!b8 & b9);
        a8 = b8 ^ (!b9 & b5);
        a9 = b9 ^ (!b5 & b6);
        a10 = b10 ^ (!b11 & b12);
        a11 = b11 ^ (!b12 & b13);
        a12 = b12 ^ (!b13 & b14);
        a13 = b13 ^ (!b14 & b10);
        a14 = b14 ^ (!b10 & b11);
        a15 = b15 ^ (!b16 & b17);
        a16 = b16 ^ (!b17 & b18);
        a17 = b17 ^ (!b18 & b19);
        a18 = b18 ^ (!b19 & b15);
        a19 = b19 ^ (!b15 & b16);
        a20 = b20 ^ (!b21 & b22);
        a21 = b21 ^ (!b22 & b23);
        a22 = b22 ^ (!b23 & b24);
        a23 = b23 ^ (!b24 & b20);
        a24 = b24 ^ (!b20 & b21);
        // ι
        a0 ^= rc;
    }
    *st = [
        a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, a16, a17, a18, a19,
        a20, a21, a22, a23, a24,
    ];
}

/// Rotate-left 4 lanes (one per packed Keccak state) by `n ∈ [1, 63]`. AVX2 has no 64-bit rotate
/// before AVX-512, so it is the `(x << n) | (x >> (64−n))` pair with variable-count shifts.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn rotl64_x4(x: std::arch::x86_64::__m256i, n: i32) -> std::arch::x86_64::__m256i {
    use std::arch::x86_64::*;
    _mm256_or_si256(
        _mm256_sll_epi64(x, _mm_cvtsi32_si128(n)),
        _mm256_srl_epi64(x, _mm_cvtsi32_si128(64 - n)),
    )
}

/// Keccak-f[1600] over **four independent states at once** — lane `i` of each `__m256i` is lane `i`
/// of state `i`. The same θ / ρ·π / χ / ι as the scalar `keccak_f1600`, with AVX2 lane-parallel XOR,
/// rotate, and `andnot` (`χ`'s `¬b ∧ c`). Bit-identical, per lane, to four scalar permutations.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn keccak_f1600_x4(st: &mut [std::arch::x86_64::__m256i; 25]) {
    use std::arch::x86_64::*;
    macro_rules! xor {
        ($a:expr, $b:expr) => {
            _mm256_xor_si256($a, $b)
        };
    }
    let mut a = *st;
    for &rc in RC.iter() {
        // θ
        let c0 = xor!(xor!(xor!(xor!(a[0], a[5]), a[10]), a[15]), a[20]);
        let c1 = xor!(xor!(xor!(xor!(a[1], a[6]), a[11]), a[16]), a[21]);
        let c2 = xor!(xor!(xor!(xor!(a[2], a[7]), a[12]), a[17]), a[22]);
        let c3 = xor!(xor!(xor!(xor!(a[3], a[8]), a[13]), a[18]), a[23]);
        let c4 = xor!(xor!(xor!(xor!(a[4], a[9]), a[14]), a[19]), a[24]);
        let d0 = xor!(c4, rotl64_x4(c1, 1));
        let d1 = xor!(c0, rotl64_x4(c2, 1));
        let d2 = xor!(c1, rotl64_x4(c3, 1));
        let d3 = xor!(c2, rotl64_x4(c4, 1));
        let d4 = xor!(c3, rotl64_x4(c0, 1));
        a[0] = xor!(a[0], d0);
        a[5] = xor!(a[5], d0);
        a[10] = xor!(a[10], d0);
        a[15] = xor!(a[15], d0);
        a[20] = xor!(a[20], d0);
        a[1] = xor!(a[1], d1);
        a[6] = xor!(a[6], d1);
        a[11] = xor!(a[11], d1);
        a[16] = xor!(a[16], d1);
        a[21] = xor!(a[21], d1);
        a[2] = xor!(a[2], d2);
        a[7] = xor!(a[7], d2);
        a[12] = xor!(a[12], d2);
        a[17] = xor!(a[17], d2);
        a[22] = xor!(a[22], d2);
        a[3] = xor!(a[3], d3);
        a[8] = xor!(a[8], d3);
        a[13] = xor!(a[13], d3);
        a[18] = xor!(a[18], d3);
        a[23] = xor!(a[23], d3);
        a[4] = xor!(a[4], d4);
        a[9] = xor!(a[9], d4);
        a[14] = xor!(a[14], d4);
        a[19] = xor!(a[19], d4);
        a[24] = xor!(a[24], d4);
        // ρ + π
        let b0 = a[0];
        let b1 = rotl64_x4(a[6], 44);
        let b2 = rotl64_x4(a[12], 43);
        let b3 = rotl64_x4(a[18], 21);
        let b4 = rotl64_x4(a[24], 14);
        let b5 = rotl64_x4(a[3], 28);
        let b6 = rotl64_x4(a[9], 20);
        let b7 = rotl64_x4(a[10], 3);
        let b8 = rotl64_x4(a[16], 45);
        let b9 = rotl64_x4(a[22], 61);
        let b10 = rotl64_x4(a[1], 1);
        let b11 = rotl64_x4(a[7], 6);
        let b12 = rotl64_x4(a[13], 25);
        let b13 = rotl64_x4(a[19], 8);
        let b14 = rotl64_x4(a[20], 18);
        let b15 = rotl64_x4(a[4], 27);
        let b16 = rotl64_x4(a[5], 36);
        let b17 = rotl64_x4(a[11], 10);
        let b18 = rotl64_x4(a[17], 15);
        let b19 = rotl64_x4(a[23], 56);
        let b20 = rotl64_x4(a[2], 62);
        let b21 = rotl64_x4(a[8], 55);
        let b22 = rotl64_x4(a[14], 39);
        let b23 = rotl64_x4(a[15], 41);
        let b24 = rotl64_x4(a[21], 2);
        // χ (¬b[i+1] ∧ b[i+2] is exactly _mm256_andnot_si256)
        a[0] = xor!(b0, _mm256_andnot_si256(b1, b2));
        a[1] = xor!(b1, _mm256_andnot_si256(b2, b3));
        a[2] = xor!(b2, _mm256_andnot_si256(b3, b4));
        a[3] = xor!(b3, _mm256_andnot_si256(b4, b0));
        a[4] = xor!(b4, _mm256_andnot_si256(b0, b1));
        a[5] = xor!(b5, _mm256_andnot_si256(b6, b7));
        a[6] = xor!(b6, _mm256_andnot_si256(b7, b8));
        a[7] = xor!(b7, _mm256_andnot_si256(b8, b9));
        a[8] = xor!(b8, _mm256_andnot_si256(b9, b5));
        a[9] = xor!(b9, _mm256_andnot_si256(b5, b6));
        a[10] = xor!(b10, _mm256_andnot_si256(b11, b12));
        a[11] = xor!(b11, _mm256_andnot_si256(b12, b13));
        a[12] = xor!(b12, _mm256_andnot_si256(b13, b14));
        a[13] = xor!(b13, _mm256_andnot_si256(b14, b10));
        a[14] = xor!(b14, _mm256_andnot_si256(b10, b11));
        a[15] = xor!(b15, _mm256_andnot_si256(b16, b17));
        a[16] = xor!(b16, _mm256_andnot_si256(b17, b18));
        a[17] = xor!(b17, _mm256_andnot_si256(b18, b19));
        a[18] = xor!(b18, _mm256_andnot_si256(b19, b15));
        a[19] = xor!(b19, _mm256_andnot_si256(b15, b16));
        a[20] = xor!(b20, _mm256_andnot_si256(b21, b22));
        a[21] = xor!(b21, _mm256_andnot_si256(b22, b23));
        a[22] = xor!(b22, _mm256_andnot_si256(b23, b24));
        a[23] = xor!(b23, _mm256_andnot_si256(b24, b20));
        a[24] = xor!(b24, _mm256_andnot_si256(b20, b21));
        // ι
        a[0] = xor!(a[0], _mm256_set1_epi64x(rc as i64));
    }
    *st = a;
}

/// 4-way SHAKE128 absorb of four already-padded single rate-blocks (168 bytes each: the caller
/// writes the message, the `0x1f` delimiter, and the `0x80` final-bit). Returns the packed state
/// after the permutation, so its first 168 bytes (per lane) are squeeze block 0 — the lane-parallel
/// twin of `shake128_absorb` for the 34-byte matrix-expansion XOF seed.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn shake128_x4_absorb_once(
    blocks: &[[u8; 168]; 4],
) -> [std::arch::x86_64::__m256i; 25] {
    use std::arch::x86_64::*;
    let mut st = [_mm256_setzero_si256(); 25];
    for (lane, slot) in st.iter_mut().enumerate().take(21) {
        let l = |s: usize| -> i64 {
            i64::from_le_bytes(blocks[s][lane * 8..lane * 8 + 8].try_into().unwrap())
        };
        *slot = _mm256_set_epi64x(l(3), l(2), l(1), l(0));
    }
    keccak_f1600_x4(&mut st);
    st
}

/// Extract the current 168-byte rate block of each of the four packed SHAKE128 states. Call
/// `keccak_f1600_x4` between blocks to advance the squeeze.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn shake128_x4_squeeze_block(
    st: &[std::arch::x86_64::__m256i; 25],
) -> [[u8; 168]; 4] {
    use std::arch::x86_64::*;
    let mut out = [[0u8; 168]; 4];
    let mut lane_bytes = [0u64; 4];
    for (lane, slot) in st.iter().enumerate().take(21) {
        _mm256_storeu_si256(lane_bytes.as_mut_ptr() as *mut __m256i, *slot);
        for (state, &v) in lane_bytes.iter().enumerate() {
            out[state][lane * 8..lane * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// 4-way SHAKE256 PRF for ML-KEM's CBD noise: absorb four independent inputs (each `seed‖nonce`,
/// ≤135 bytes → a single rate block) and squeeze 128 bytes from each — the lane-parallel twin of a
/// scalar `SHAKE256(·, 128)`. rate = 136 (17 lanes); 128 output bytes are the first 16 state lanes,
/// so one `keccak_f1600_x4` suffices (no second squeeze block). Batches the six/seven independent
/// noise streams of K-PKE keygen/encrypt four-per-permutation, matching the hand-tuned reference libs.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn shake256_x4_128(inputs: &[&[u8]; 4]) -> [[u8; 128]; 4] {
    use std::arch::x86_64::*;
    const RATE: usize = 136;
    let mut blocks = [[0u8; RATE]; 4];
    for (b, inp) in blocks.iter_mut().zip(inputs.iter()) {
        debug_assert!(
            inp.len() < RATE,
            "shake256_x4_128 expects single-block inputs"
        );
        b[..inp.len()].copy_from_slice(inp);
        b[inp.len()] = 0x1f;
        b[RATE - 1] |= 0x80;
    }
    let mut st = [_mm256_setzero_si256(); 25];
    for (lane, slot) in st.iter_mut().enumerate().take(RATE / 8) {
        let l = |s: usize| -> i64 {
            i64::from_le_bytes(blocks[s][lane * 8..lane * 8 + 8].try_into().unwrap())
        };
        *slot = _mm256_set_epi64x(l(3), l(2), l(1), l(0));
    }
    keccak_f1600_x4(&mut st);
    let mut out = [[0u8; 128]; 4];
    let mut lane_bytes = [0u64; 4];
    for (lane, slot) in st.iter().enumerate().take(16) {
        _mm256_storeu_si256(lane_bytes.as_mut_ptr() as *mut __m256i, *slot);
        for (state, &v) in lane_bytes.iter().enumerate() {
            out[state][lane * 8..lane * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// SHAKE128 incremental absorb: XOR `input` into a fresh state at rate 168 with the SHAKE
/// delimiter and run the final permutation, so the state's first 168 bytes are squeeze block 0.
/// The caller reads that block, then `keccak_f1600` advances to the next — streaming, no heap.
pub(crate) fn shake128_absorb(input: &[u8]) -> [u64; 25] {
    const RATE: usize = 168;
    const LANES: usize = RATE / 8;
    let mut st = [0u64; 25];
    let mut chunks = input.chunks_exact(RATE);
    for block in &mut chunks {
        for i in 0..LANES {
            st[i] ^= u64::from_le_bytes(block[i * 8..i * 8 + 8].try_into().unwrap());
        }
        keccak_f1600(&mut st);
    }
    let rem = chunks.remainder();
    let mut block = [0u8; 200];
    block[..rem.len()].copy_from_slice(rem);
    block[rem.len()] = 0x1f;
    block[RATE - 1] |= 0x80;
    for i in 0..LANES {
        st[i] ^= u64::from_le_bytes(block[i * 8..i * 8 + 8].try_into().unwrap());
    }
    keccak_f1600(&mut st);
    st
}

/// SHAKE streaming absorb at an arbitrary `rate` (168 for SHAKE128, 136 for SHAKE256), returning the
/// state after the final permutation: its first `rate` bytes are squeeze block 0, and `keccak_f1600`
/// advances to the next block. The caller drives the squeeze (the rejection samplers in `mldsa`).
pub(crate) fn shake_absorb(input: &[u8], rate: usize) -> [u64; 25] {
    let lanes = rate / 8;
    let mut st = [0u64; 25];
    let mut chunks = input.chunks_exact(rate);
    for block in &mut chunks {
        for (i, lane) in st.iter_mut().enumerate().take(lanes) {
            *lane ^= u64::from_le_bytes(block[i * 8..i * 8 + 8].try_into().unwrap());
        }
        keccak_f1600(&mut st);
    }
    let rem = chunks.remainder();
    let mut block = [0u8; 200];
    block[..rem.len()].copy_from_slice(rem);
    block[rem.len()] = 0x1f;
    block[rate - 1] |= 0x80;
    for (i, lane) in st.iter_mut().enumerate().take(lanes) {
        *lane ^= u64::from_le_bytes(block[i * 8..i * 8 + 8].try_into().unwrap());
    }
    keccak_f1600(&mut st);
    st
}

/// Squeeze the current `rate`-byte block from a SHAKE state into `out`.
pub(crate) fn shake_squeeze_block(st: &[u64; 25], out: &mut [u8], rate: usize) {
    for i in 0..rate / 8 {
        out[i * 8..i * 8 + 8].copy_from_slice(&st[i].to_le_bytes());
    }
}

/// Advance a SHAKE squeeze to the next block.
pub(crate) fn keccak_permute(st: &mut [u64; 25]) {
    keccak_f1600(st);
}

/// The Keccak sponge: absorb `input` at the given `rate` (bytes) with domain `delim`, then squeeze
/// `out.len()` bytes. Covers SHA-3 (`delim=0x06`) and SHAKE (`delim=0x1f`).
fn keccak(rate: usize, delim: u8, input: &[u8], out: &mut [u8]) {
    let mut st = [0u64; 25];
    let lanes = rate / 8;

    // Absorb full blocks.
    let mut chunks = input.chunks_exact(rate);
    for block in &mut chunks {
        for i in 0..lanes {
            st[i] ^= u64::from_le_bytes(block[i * 8..i * 8 + 8].try_into().unwrap());
        }
        keccak_f1600(&mut st);
    }
    // Pad + absorb the final (partial) block.
    let rem = chunks.remainder();
    let mut block = [0u8; 200];
    block[..rem.len()].copy_from_slice(rem);
    block[rem.len()] = delim;
    block[rate - 1] |= 0x80;
    for i in 0..lanes {
        st[i] ^= u64::from_le_bytes(block[i * 8..i * 8 + 8].try_into().unwrap());
    }
    keccak_f1600(&mut st);

    // Squeeze.
    let mut off = 0;
    while off < out.len() {
        let n = (out.len() - off).min(rate);
        let mut produced = [0u8; 200];
        for i in 0..lanes {
            produced[i * 8..i * 8 + 8].copy_from_slice(&st[i].to_le_bytes());
        }
        out[off..off + n].copy_from_slice(&produced[..n]);
        off += n;
        if off < out.len() {
            keccak_f1600(&mut st);
        }
    }
}

pub fn sha3_256_bytes(input: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    keccak(136, 0x06, input, &mut out);
    out
}
pub fn sha3_512_bytes(input: &[u8]) -> [u8; 64] {
    let mut out = [0u8; 64];
    keccak(72, 0x06, input, &mut out);
    out
}
pub fn shake128_bytes(input: &[u8], outlen: usize) -> Vec<u8> {
    let mut out = vec![0u8; outlen];
    keccak(168, 0x1f, input, &mut out);
    out
}
pub fn shake256_bytes(input: &[u8], outlen: usize) -> Vec<u8> {
    let mut out = vec![0u8; outlen];
    keccak(136, 0x1f, input, &mut out);
    out
}

// ── LOGOS-facing wrappers (Seq of Int bytes 0..255) ──────────────────────────────────────────

/// SHA3-256 — the compiled form of LOGOS `sha3_256(a)`. 32 output bytes.
pub fn sha3_256(input: &[i64]) -> Result<logicaffeine_data::LogosSeq<i64>, String> {
    Ok(args::seq_of_bytes(&sha3_256_bytes(&args::bytes(
        "message", input,
    )?)))
}
/// SHA3-512 — 64 output bytes.
pub fn sha3_512(input: &[i64]) -> Result<logicaffeine_data::LogosSeq<i64>, String> {
    Ok(args::seq_of_bytes(&sha3_512_bytes(&args::bytes(
        "message", input,
    )?)))
}
/// SHAKE128 XOF — `outlen` output bytes.
pub fn shake128(input: &[i64], outlen: i64) -> Result<logicaffeine_data::LogosSeq<i64>, String> {
    let message = args::bytes("message", input)?;
    let outlen = args::non_negative("output length", outlen)?;
    Ok(args::seq_of_bytes(&shake128_bytes(&message, outlen)))
}
/// SHAKE256 XOF — `outlen` output bytes.
pub fn shake256(input: &[i64], outlen: i64) -> Result<logicaffeine_data::LogosSeq<i64>, String> {
    let message = args::bytes("message", input)?;
    let outlen = args::non_negative("output length", outlen)?;
    Ok(args::seq_of_bytes(&shake256_bytes(&message, outlen)))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_system/keccak/tests.rs"]
mod tests;

#[cfg(all(test, target_arch = "x86_64"))]
#[path = "../../logicaffeine_tests/unit/logicaffeine_system/keccak/x4_lanes_ab.rs"]
mod x4_lanes_ab;
