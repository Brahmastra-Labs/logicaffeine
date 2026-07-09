//! Runtime support for the `Word8`/`Word16`/`Word32`/`Word64` ring types in COMPILED LOGOS.
//!
//! Generated Rust constructs words with `word32(x)`, rotates with `rotl(x, n)`, and shows them.
//! Arithmetic and bit operators (`+`, `^`, `&`, …) already live as trait impls on the `Word`
//! newtypes in [`logicaffeine_base`] — each delegating to the wrapping primitive — so the
//! emitted `a + b` is ring-correct with no per-site `wrapping_*`. This module supplies the
//! remaining glue the codegen names: the `word*` constructors, the width-generic `rotl`/`rotr`,
//! and the [`Showable`] impls so `Show` renders a word as its decimal value (matching the
//! tree-walker and VM byte-for-byte).

use crate::io::Showable;
use core::fmt;
use logicaffeine_data::LogosSeq;

pub use logicaffeine_base::{
    Lanes16Word16, Lanes16Word8, Lanes4Word32, Lanes4Word64, Lanes8Word32, Word16, Word32, Word64,
    Word8,
};

/// Construct a word from the low bits of an integer — the compiled form of `word32(x)` etc.
#[inline]
pub fn word8(x: i64) -> Word8 {
    Word8(x as u8)
}
#[inline]
pub fn word16(x: i64) -> Word16 {
    Word16(x as u16)
}
#[inline]
pub fn word32(x: i64) -> Word32 {
    Word32(x as u32)
}
#[inline]
pub fn word64(x: i64) -> Word64 {
    Word64(x as u64)
}

/// Width-defined bit rotation, available on every word width so `rotl(x, n)` is one name.
pub trait WordRotate: Copy {
    fn rotl(self, n: u32) -> Self;
    fn rotr(self, n: u32) -> Self;
}

macro_rules! impl_rotate {
    ($($w:ident => $prim:ty),* $(,)?) => { $(
        impl WordRotate for logicaffeine_base::$w {
            #[inline]
            fn rotl(self, n: u32) -> Self { logicaffeine_base::$w(self.0.rotate_left(n)) }
            #[inline]
            fn rotr(self, n: u32) -> Self { logicaffeine_base::$w(self.0.rotate_right(n)) }
        }
    )* };
}
impl_rotate!(Word8 => u8, Word16 => u16, Word32 => u32, Word64 => u64);

// A SIMD lane vector rotates lane-wise — so generated `rotl(v, 16)` over `Lanes8Word32` lowers to
// the AVX2 shift-or rotation. `rotr` is the inverse rotation (the lane vocabulary only ships `rotl`).
impl WordRotate for Lanes8Word32 {
    #[inline]
    fn rotl(self, n: u32) -> Self {
        Lanes8Word32::rotl(self, n)
    }
    #[inline]
    fn rotr(self, n: u32) -> Self {
        Lanes8Word32::rotl(self, (32 - n % 32) % 32)
    }
}

// The 4-way Keccak lane vector rotates lane-wise (64-bit) — generated `rotl(v, n)` over
// `Lanes4Word64` lowers to the AVX2 `vpsllq`/`vpsrlq` shift-or (ρ offsets, θ's D term).
impl WordRotate for Lanes4Word64 {
    #[inline]
    fn rotl(self, n: u32) -> Self {
        Lanes4Word64::rotl(self, n)
    }
    #[inline]
    fn rotr(self, n: u32) -> Self {
        Lanes4Word64::rotl(self, (64 - n % 64) % 64)
    }
}

/// Left rotation — the compiled form of `rotl(x, n)`. The amount is taken as `i64` (the type of
/// a LOGOS `Int`) and reduced to the rotation width.
#[inline]
pub fn rotl<W: WordRotate>(x: W, n: i64) -> W {
    x.rotl(n as u32)
}
/// Bitwise AND/OR/NOT — the compiled form of `word_and`/`word_or`/`word_not`. Distinct from the
/// `and`/`or` keywords (logical short-circuit) so word crypto written in LOGOS (the MD5/SHA-1 round
/// functions) is bit-exact on every tier. Word8/16/32/64 impl these operators, so they lower to a
/// single machine `and`/`or`/`not`; a lane vector lowers to the AVX2 vector form.
#[inline]
pub fn word_and<W: ::core::ops::BitAnd<Output = W>>(a: W, b: W) -> W {
    a & b
}
#[inline]
pub fn word_or<W: ::core::ops::BitOr<Output = W>>(a: W, b: W) -> W {
    a | b
}
#[inline]
pub fn word_not<W: ::core::ops::Not<Output = W>>(a: W) -> W {
    !a
}

/// Right rotation — the compiled form of `rotr(x, n)`.
#[inline]
pub fn rotr<W: WordRotate>(x: W, n: i64) -> W {
    x.rotr(n as u32)
}

macro_rules! impl_showable_word {
    ($($w:ty),* $(,)?) => { $(
        impl Showable for $w {
            #[inline(always)]
            fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }
    )* };
}
impl_showable_word!(Word8, Word16, Word32, Word64);

// ── SIMD lane vectors — the compiled forms of the lane constructors/accessors ─────────────────

/// Pack the first 8 `Word32`s of a Seq into a lane vector — the compiled form of `lanes8Word32(s)`.
#[inline]
pub fn lanes8_word32(s: &LogosSeq<Word32>) -> Lanes8Word32 {
    // Pack straight from the backing slice — `from_words` already takes `&[Word32]` and zero-fills a
    // short slice, so there's no reason to clone the Seq into a fresh Vec on this crypto hot path.
    Lanes8Word32::from_words(s.0.borrow().as_slice())
}

/// Broadcast one `Word32` into all 8 lanes — the compiled form of `splat8Word32(x)` (a crypto kernel
/// loads a shared constant/key word into every block's lane this way).
#[inline]
pub fn splat8_word32(x: Word32) -> Lanes8Word32 {
    Lanes8Word32::splat(x.0)
}

// ── Byte-shuffle lane (`Lanes16Word8` = one `__m128i`) — the compiled forms of the byte ops a SIMD
//    hex codec WRITTEN in Logos lowers to: `pshufb`, per-byte shift, and the two byte interleaves. ──

/// Pack the first 16 bytes of a `Seq of Int` into a byte-shuffle register — compiled `lanes16Word8(s)`.
#[inline]
pub fn lanes16_word8(s: &LogosSeq<i64>) -> Lanes16Word8 {
    let mut a = [0u8; 16];
    for (i, v) in s.0.borrow().iter().take(16).enumerate() {
        a[i] = *v as u8;
    }
    Lanes16Word8(a)
}
/// The 16 bytes back as a `Seq of Int` — compiled `seqOfLanes16W8(v)`.
#[inline]
pub fn seq_of_lanes16w8(v: Lanes16Word8) -> LogosSeq<i64> {
    LogosSeq::from_vec(v.0.iter().map(|&b| b as i64).collect())
}
/// Broadcast one byte into all 16 lanes — compiled `splat16Word8(x)`.
#[inline]
pub fn splat16_word8(x: i64) -> Lanes16Word8 {
    Lanes16Word8::splat(x as u8)
}
/// Byte shuffle (`pshufb`) — compiled `shuffle16(table, idx)`.
#[inline]
pub fn shuffle16(table: Lanes16Word8, idx: Lanes16Word8) -> Lanes16Word8 {
    table.shuffle(idx)
}
/// Per-byte logical shift right — compiled `shrBytes16(v, n)`.
#[inline]
pub fn shr_bytes16(v: Lanes16Word8, n: i64) -> Lanes16Word8 {
    v.shr_bytes(n as u32)
}
/// Low-eight byte interleave (`_mm_unpacklo_epi8`) — compiled `interleaveLo16(a, b)`.
#[inline]
pub fn interleave_lo16(a: Lanes16Word8, b: Lanes16Word8) -> Lanes16Word8 {
    a.interleave_lo(b)
}
/// High-eight byte interleave (`_mm_unpackhi_epi8`) — compiled `interleaveHi16(a, b)`.
#[inline]
pub fn interleave_hi16(a: Lanes16Word8, b: Lanes16Word8) -> Lanes16Word8 {
    a.interleave_hi(b)
}
/// Per-byte wrapping add — compiled `byteAdd16(a, b)` (the ASCII→nibble decode).
#[inline]
pub fn byte_add16(a: Lanes16Word8, b: Lanes16Word8) -> Lanes16Word8 {
    a.byte_add(b)
}
/// Multiply-add adjacent byte pairs (`pmaddubsw`) — compiled `maddubs16(a, w)` (nibble-pair → byte).
#[inline]
pub fn maddubs16(a: Lanes16Word8, w: Lanes16Word8) -> Lanes16Word8 {
    a.maddubs(w)
}
/// Pack two `8×i16` vectors to `16×u8` with unsigned saturation (`packuswb`) — compiled `packus16(a, b)`.
#[inline]
pub fn packus16(a: Lanes16Word8, b: Lanes16Word8) -> Lanes16Word8 {
    a.packus(b)
}

// ── SHA-1 SHA-NI lane (`Lanes4Word32` = one `__m128i`) — the compiled forms of the SHA-1 ops that a
//    SHA-1 WRITTEN in Logos over `Lanes4Word32` lowers to (the base type carries the SHA-NI fast
//    path + software fallback). Pack/unpack move a `Seq of Word32` (4 words) to/from the register. ──

/// Pack the first 4 `Word32`s of a Seq into a SHA-1 lane register — compiled `lanes4Word32(s)`.
#[inline]
pub fn lanes4_word32(s: &LogosSeq<Word32>) -> Lanes4Word32 {
    // Pack straight from the backing slice — no intermediate Vec clone (SHA-1 packs 4 words per lane,
    // several times per block, so this clone sat on the hottest part of the compiled hash).
    Lanes4Word32::from_words(s.0.borrow().as_slice())
}
/// Pack four `Word32`s straight into a SHA-1 lane register — compiled `lanes4Of(a, b, c, d)`. The
/// alloc-free constructor (no `Seq`, no heap): the Logos SHA-1 packs a lane this way every round.
#[inline]
pub fn lanes4_of(a: Word32, b: Word32, c: Word32, d: Word32) -> Lanes4Word32 {
    Lanes4Word32([a.0, b.0, c.0, d.0])
}
/// The 4 lanes back as a `Seq of Word32` — compiled `seqOfLanes4W32(v)`.
#[inline]
pub fn seq_of_lanes4w32(v: Lanes4Word32) -> LogosSeq<Word32> {
    LogosSeq::from_vec(v.to_words().to_vec())
}
/// `sha1rnds4(abcd, msg, func)` — four SHA-1 rounds. Lowers to the `sha1rnds4` instruction.
#[inline]
pub fn sha1rnds4(abcd: Lanes4Word32, msg: Lanes4Word32, func: i64) -> Lanes4Word32 {
    abcd.sha1rnds4(msg, func as u32)
}
/// `sha1msg1(a, b)` — message-schedule step 1. Lowers to `sha1msg1`.
#[inline]
pub fn sha1msg1(a: Lanes4Word32, b: Lanes4Word32) -> Lanes4Word32 {
    a.sha1msg1(b)
}
/// `sha1msg2(a, b)` — message-schedule step 2. Lowers to `sha1msg2`.
#[inline]
pub fn sha1msg2(a: Lanes4Word32, b: Lanes4Word32) -> Lanes4Word32 {
    a.sha1msg2(b)
}
/// `sha1nexte(a, b)` — fold the next round E. Lowers to `sha1nexte`.
#[inline]
pub fn sha1nexte(a: Lanes4Word32, b: Lanes4Word32) -> Lanes4Word32 {
    a.sha1nexte(b)
}

/// The unsigned value of a `Word32` as an `Int` — the compiled form of `intOfWord32(w)`, used to
/// serialize a keystream word into bytes (`Int` mod/div) for the XOR against a `Seq of Int` payload.
#[inline]
pub fn int_of_word32(w: Word32) -> i64 {
    w.0 as i64
}

/// The value of a `Word64` as an `Int` — `intOfWord64(w)`. For values ≥ 2⁶³ this is a negative
/// `i64` (two's-complement bits); used on byte-masked lanes (`< 256`) in Keccak's squeeze.
#[inline]
pub fn int_of_word64(w: Word64) -> i64 {
    w.0 as i64
}
/// `word64Shl(w, n)` — logical shift-left of a `Word64` by `n` bits (Keccak lane byte-packing).
#[inline]
pub fn word64_shl(w: Word64, n: i64) -> Word64 {
    Word64(w.0.wrapping_shl(n as u32))
}
/// `word64Shr(w, n)` — logical shift-right of a `Word64` by `n` bits (Keccak squeeze byte-extract).
#[inline]
pub fn word64_shr(w: Word64, n: i64) -> Word64 {
    Word64(w.0.wrapping_shr(n as u32))
}
/// `word64And(a, b)` — bitwise AND of two `Word64`s (Keccak χ's `¬b ∧ c`, and byte masking).
#[inline]
pub fn word64_and(a: Word64, b: Word64) -> Word64 {
    Word64(a.0 & b.0)
}
/// `word32Shr(w, n)` — logical shift-right of a `Word32` by `n` bits (SHA-256's `σ0`/`σ1` message
/// schedule, where the shift is NOT a rotate — the vacated high bits are zero).
#[inline]
pub fn word32_shr(w: Word32, n: i64) -> Word32 {
    Word32(w.0.wrapping_shr(n as u32))
}

/// The unsigned value of a `Word16` as an `Int` (0..2¹⁶−1) — the compiled form of `intOfWord16(w)`,
/// the ML-KEM NTT's Word16-coefficient → Int boundary.
#[inline]
pub fn int_of_word16(w: Word16) -> i64 {
    w.0 as i64
}

// ── Lanes4Word64 — the Poly1305 accumulator lane config ───────────────────────────────────────

/// Pack the first 4 `Int`s of a Seq into a `Lanes4Word64` — the compiled form of `lanes4Word64(s)`.
#[inline]
pub fn lanes4_word64(s: &LogosSeq<i64>) -> Lanes4Word64 {
    let mut a = [0u64; 4];
    for (i, v) in s.iter().take(4).enumerate() {
        a[i] = v as u64;
    }
    Lanes4Word64(a)
}

/// Unpack a `Lanes4Word64` into a Seq of 4 `Int` lanes — the compiled form of `seqOfLanes4(v)`.
#[inline]
pub fn seq_of_lanes4(v: Lanes4Word64) -> LogosSeq<i64> {
    LogosSeq::from_vec(v.0.iter().map(|&x| x as i64).collect())
}

/// Lane-wise widening low-32 multiply (`vpmuludq`) — the compiled form of `mul32x32to64(a, b)`.
#[inline]
pub fn mul32x32to64(a: Lanes4Word64, b: Lanes4Word64) -> Lanes4Word64 {
    a.mul_lo32_wide(b)
}

/// `splat4Word64(x)` — broadcast one `Word64` into all four Keccak lanes (the ι round-constant XOR,
/// and the all-ones vector for χ's complement). The compiled form of the Logos builtin.
#[inline]
pub fn splat4_word64(x: Word64) -> Lanes4Word64 {
    Lanes4Word64::splat(x.0)
}

/// `andNot4(a, b)` — Keccak χ's `(¬a) ∧ b` in one lane op (`vpandn`). The compiled form of the builtin.
#[inline]
pub fn and_not4(a: Lanes4Word64, b: Lanes4Word64) -> Lanes4Word64 {
    a.andnot(b)
}

/// The horizontal sum of a `Lanes4Word64`'s lanes as an `Int` — the compiled form of `hsumLanes4(v)`.
#[inline]
pub fn hsum_lanes4(v: Lanes4Word64) -> i64 {
    v.hsum() as i64
}

// ── Lanes16Word16 — the NTT coefficient lane config ───────────────────────────────────────────

/// Pack the first 16 `Int`s of a Seq into a `Lanes16Word16` — the compiled form of `lanes16Word16(s)`.
#[inline]
pub fn lanes16_word16(s: &LogosSeq<i64>) -> Lanes16Word16 {
    let mut a = [0u16; 16];
    for (i, v) in s.iter().take(16).enumerate() {
        a[i] = v as u16;
    }
    Lanes16Word16(a)
}

/// Unpack a `Lanes16Word16` into a Seq of 16 `Int` lanes (u16 bits) — `seqOfLanes16(v)`.
#[inline]
pub fn seq_of_lanes16(v: Lanes16Word16) -> LogosSeq<i64> {
    LogosSeq::from_vec(v.0.iter().map(|&x| x as i64).collect())
}

/// Broadcast a `Word16`/`Int` into all 16 lanes — the compiled form of `splat16Word16(x)`.
#[inline]
pub fn splat16_word16(x: i64) -> Lanes16Word16 {
    Lanes16Word16::splat(x as u16)
}

/// Lane-wise SIGNED high-16 multiply (`vpmulhw`) — the compiled form of `mulhi16(a, b)`.
#[inline]
pub fn mulhi16(a: Lanes16Word16, b: Lanes16Word16) -> Lanes16Word16 {
    a.mulhi(b)
}

/// The signed i32 Montgomery multiply (`vpmuldq`) — the compiled form of `montmul32(a, b, q, qinv)`,
/// the ML-DSA NTT butterfly multiply.
#[inline]
pub fn montmul32(
    a: Lanes8Word32,
    b: Lanes8Word32,
    q: Lanes8Word32,
    qinv: Lanes8Word32,
) -> Lanes8Word32 {
    a.montmul32(b, q, qinv)
}

// The within-vector NTT permutes (`nttBcastLo`/`nttBcastHi`/`nttBlend`) compile to inherent-method
// calls (`v.ntt_bcast_lo(h)`), so Rust resolves the right lane impl by type (Lanes16Word16 i16 /
// Lanes8Word32 i32) — no per-config free-function wrapper needed.

/// Unpack a lane vector into a Seq of 8 `Word32` — the compiled form of `seqOfLanes8(v)`.
#[inline]
pub fn seq_of_lanes8(v: Lanes8Word32) -> LogosSeq<Word32> {
    LogosSeq::from_vec(v.to_words().to_vec())
}

// A lane vector renders like a Seq of its lanes (`[l0, l1, …]`), byte-identical to the tree-walker.
impl Showable for Lanes8Word32 {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[")?;
        for (i, w) in self.to_words().iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            fmt::Display::fmt(w, f)?;
        }
        write!(f, "]")
    }
}
