//! Fixed-width wrapping integers — the ring ℤ/2ᵏℤ.
//!
//! Unlike [`crate::numeric::BigInt`], whose arithmetic is exact and unbounded, a `Word`
//! is **total and wrapping**: `Word32::MAX.add(Word32::ONE) == Word32::ZERO`. This is the
//! natural home for the bit-twiddling primitives — ChaCha20 lives over `Word32`, Keccak
//! over `Word64` — where each operation is a single native instruction and the modular
//! semantics are exact rather than an overflow into arbitrary precision. Rotation
//! (`rotl`/`rotr`) is width-defined and lives only here.

macro_rules! impl_word {
    ($name:ident, $prim:ty, $bits:literal) => {
        #[doc = concat!("A ", $bits, "-bit wrapping integer: the ring ℤ/2^", $bits, "ℤ, where every operation is total and wraps modulo 2^", $bits, ".")]
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
        #[repr(transparent)]
        pub struct $name(pub $prim);

        impl $name {
            /// Bit width of the ring.
            pub const BITS: u32 = <$prim>::BITS;
            /// Additive identity.
            pub const ZERO: Self = Self(0);
            /// Multiplicative identity.
            pub const ONE: Self = Self(1);
            /// The all-ones value (`2^BITS − 1`).
            pub const MAX: Self = Self(<$prim>::MAX);

            /// The underlying primitive value.
            #[inline]
            pub const fn get(self) -> $prim {
                self.0
            }

            /// Wrapping addition in ℤ/2ᵏ.
            #[inline]
            pub const fn add(self, o: Self) -> Self {
                Self(self.0.wrapping_add(o.0))
            }

            /// Wrapping subtraction in ℤ/2ᵏ.
            #[inline]
            pub const fn sub(self, o: Self) -> Self {
                Self(self.0.wrapping_sub(o.0))
            }

            /// Wrapping multiplication in ℤ/2ᵏ.
            #[inline]
            pub const fn mul(self, o: Self) -> Self {
                Self(self.0.wrapping_mul(o.0))
            }

            /// Bitwise AND.
            #[inline]
            pub const fn bitand(self, o: Self) -> Self {
                Self(self.0 & o.0)
            }

            /// Bitwise OR.
            #[inline]
            pub const fn bitor(self, o: Self) -> Self {
                Self(self.0 | o.0)
            }

            /// Bitwise XOR.
            #[inline]
            pub const fn bitxor(self, o: Self) -> Self {
                Self(self.0 ^ o.0)
            }

            /// Bitwise complement.
            #[inline]
            pub const fn not(self) -> Self {
                Self(!self.0)
            }

            /// Logical left shift by `n` (the count is taken modulo `BITS`, matching the
            /// hardware shift and the language's `shifted left by`).
            #[inline]
            pub const fn shl(self, n: u32) -> Self {
                Self(self.0.wrapping_shl(n))
            }

            /// Logical right shift by `n` (count modulo `BITS`).
            #[inline]
            pub const fn shr(self, n: u32) -> Self {
                Self(self.0.wrapping_shr(n))
            }

            /// Left rotation by `n` — a width-defined bit-permutation (the crypto primitive).
            #[inline]
            pub const fn rotl(self, n: u32) -> Self {
                Self(self.0.rotate_left(n))
            }

            /// Right rotation by `n`.
            #[inline]
            pub const fn rotr(self, n: u32) -> Self {
                Self(self.0.rotate_right(n))
            }
        }

        // Operator traits delegate to the wrapping primitives, so generated Rust can write the
        // natural `a + b` / `a ^ b` and get ring semantics — no per-site `wrapping_*` in codegen.
        impl ::core::ops::Add for $name {
            type Output = Self;
            #[inline]
            fn add(self, o: Self) -> Self { Self(self.0.wrapping_add(o.0)) }
        }
        impl ::core::ops::Sub for $name {
            type Output = Self;
            #[inline]
            fn sub(self, o: Self) -> Self { Self(self.0.wrapping_sub(o.0)) }
        }
        impl ::core::ops::Mul for $name {
            type Output = Self;
            #[inline]
            fn mul(self, o: Self) -> Self { Self(self.0.wrapping_mul(o.0)) }
        }
        impl ::core::ops::BitAnd for $name {
            type Output = Self;
            #[inline]
            fn bitand(self, o: Self) -> Self { Self(self.0 & o.0) }
        }
        impl ::core::ops::BitOr for $name {
            type Output = Self;
            #[inline]
            fn bitor(self, o: Self) -> Self { Self(self.0 | o.0) }
        }
        impl ::core::ops::BitXor for $name {
            type Output = Self;
            #[inline]
            fn bitxor(self, o: Self) -> Self { Self(self.0 ^ o.0) }
        }
        impl ::core::ops::Not for $name {
            type Output = Self;
            #[inline]
            fn not(self) -> Self { Self(!self.0) }
        }
        // `/` and `%` are the underlying UNSIGNED integer division / remainder — so `x % 2^k` is a
        // mask and `x / 2^k` a shift (the crypto reduction primitives), with no sign-correction.
        impl ::core::ops::Div for $name {
            type Output = Self;
            #[inline]
            fn div(self, o: Self) -> Self { Self(self.0 / o.0) }
        }
        impl ::core::ops::Rem for $name {
            type Output = Self;
            #[inline]
            fn rem(self, o: Self) -> Self { Self(self.0 % o.0) }
        }
        impl ::core::ops::Shl<u32> for $name {
            type Output = Self;
            #[inline]
            fn shl(self, n: u32) -> Self { Self(self.0.wrapping_shl(n)) }
        }
        impl ::core::ops::Shr<u32> for $name {
            type Output = Self;
            #[inline]
            fn shr(self, n: u32) -> Self { Self(self.0.wrapping_shr(n)) }
        }
        // Displays as the underlying unsigned value (decimal) — the canonical scalar form the
        // tree-walker prints, so tw / VM / AOT render a word identically.
        impl ::core::fmt::Display for $name {
            #[inline]
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::write!(f, "{}", self.0)
            }
        }
    };
}

impl_word!(Word8, u8, "8");
impl_word!(Word16, u16, "16");
impl_word!(Word32, u32, "32");
impl_word!(Word64, u64, "64");

/// A fixed-width wrapping integer of either supported width — the single runtime carrier for
/// `Word32`/`Word64` across the interpreter, VM, and wire, so the rest of the system matches on
/// one `Word` value rather than a variant per width. Binary ops require matching widths; a
/// width mismatch is a type error the caller reports (the ops return `None`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WordVal {
    /// A 32-bit wrapping value.
    W32(Word32),
    /// A 64-bit wrapping value.
    W64(Word64),
}

impl WordVal {
    /// Bit width, 32 or 64.
    #[inline]
    pub const fn width(self) -> u32 {
        match self {
            WordVal::W32(_) => 32,
            WordVal::W64(_) => 64,
        }
    }

    /// The value zero-extended to `u64` — the canonical scalar form for display, hashing into
    /// other contexts, and the wire.
    #[inline]
    pub const fn to_u64(self) -> u64 {
        match self {
            WordVal::W32(w) => w.0 as u64,
            WordVal::W64(w) => w.0,
        }
    }

    /// Build a word of the given `width` from the low bits of `bits` (32 → truncates to `u32`).
    #[inline]
    pub const fn from_u64(width: u32, bits: u64) -> Option<Self> {
        match width {
            32 => Some(WordVal::W32(Word32(bits as u32))),
            64 => Some(WordVal::W64(Word64(bits))),
            _ => None,
        }
    }

    #[inline]
    fn zip(
        self,
        o: Self,
        f32: impl FnOnce(Word32, Word32) -> Word32,
        f64: impl FnOnce(Word64, Word64) -> Word64,
    ) -> Option<Self> {
        match (self, o) {
            (WordVal::W32(a), WordVal::W32(b)) => Some(WordVal::W32(f32(a, b))),
            (WordVal::W64(a), WordVal::W64(b)) => Some(WordVal::W64(f64(a, b))),
            _ => None,
        }
    }

    /// Wrapping addition; `None` on a width mismatch.
    #[inline]
    pub fn add(self, o: Self) -> Option<Self> {
        self.zip(o, Word32::add, Word64::add)
    }
    /// Wrapping subtraction; `None` on a width mismatch.
    #[inline]
    pub fn sub(self, o: Self) -> Option<Self> {
        self.zip(o, Word32::sub, Word64::sub)
    }
    /// Wrapping multiplication; `None` on a width mismatch.
    #[inline]
    pub fn mul(self, o: Self) -> Option<Self> {
        self.zip(o, Word32::mul, Word64::mul)
    }
    /// Bitwise AND; `None` on a width mismatch.
    #[inline]
    pub fn bitand(self, o: Self) -> Option<Self> {
        self.zip(o, Word32::bitand, Word64::bitand)
    }
    /// Bitwise OR; `None` on a width mismatch.
    #[inline]
    pub fn bitor(self, o: Self) -> Option<Self> {
        self.zip(o, Word32::bitor, Word64::bitor)
    }
    /// Bitwise XOR; `None` on a width mismatch.
    #[inline]
    pub fn bitxor(self, o: Self) -> Option<Self> {
        self.zip(o, Word32::bitxor, Word64::bitxor)
    }

    /// Bitwise complement (width-preserving).
    #[inline]
    pub const fn not(self) -> Self {
        match self {
            WordVal::W32(w) => WordVal::W32(w.not()),
            WordVal::W64(w) => WordVal::W64(w.not()),
        }
    }
    /// Logical left shift by `n` (width-preserving).
    #[inline]
    pub const fn shl(self, n: u32) -> Self {
        match self {
            WordVal::W32(w) => WordVal::W32(w.shl(n)),
            WordVal::W64(w) => WordVal::W64(w.shl(n)),
        }
    }
    /// Logical right shift by `n` (width-preserving).
    #[inline]
    pub const fn shr(self, n: u32) -> Self {
        match self {
            WordVal::W32(w) => WordVal::W32(w.shr(n)),
            WordVal::W64(w) => WordVal::W64(w.shr(n)),
        }
    }
    /// Left rotation by `n` (width-preserving) — the bit-permutation crypto needs.
    #[inline]
    pub const fn rotl(self, n: u32) -> Self {
        match self {
            WordVal::W32(w) => WordVal::W32(w.rotl(n)),
            WordVal::W64(w) => WordVal::W64(w.rotl(n)),
        }
    }
    /// Right rotation by `n` (width-preserving).
    #[inline]
    pub const fn rotr(self, n: u32) -> Self {
        match self {
            WordVal::W32(w) => WordVal::W32(w.rotr(n)),
            WordVal::W64(w) => WordVal::W64(w.rotr(n)),
        }
    }
}

impl std::fmt::Display for WordVal {
    /// A word renders as its unsigned decimal value (no width suffix), so a `Word32` holding 5
    /// and an `Int` 5 print identically — the value is what `Show` reports.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_u64())
    }
}

// ── SIMD lane vectors — fixed-width vectors over the Word ring ─────────────────────────────────
//
// `Lanes8Word32` is 8 lanes of `Word32` = exactly one AVX2 `__m256i`. The whole point of the type
// is that a vectorized algorithm WRITTEN in Logos over these lanes compiles to the same instructions
// as hand-written AVX2 (each lane op IS an intrinsic). The runtime carries the portable scalar
// representation `[u32; 8]`; every lane op has an AVX2 fast path AND a scalar fallback, and the two
// are proven byte-identical by a fuzz test (the SIMD-correctness proof at the base level). The
// language's tier differential (tree-walker == VM == AOT) then proves the pipeline lowers it right.

/// Eight lanes of `Word32` (one 256-bit SIMD register). Operations are lane-wise over the ℤ/2³² ring.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(C, align(32))]
pub struct Lanes8Word32(pub [u32; 8]);

impl Lanes8Word32 {
    /// The number of lanes.
    pub const LANES: usize = 8;

    /// Broadcast one value into all eight lanes.
    #[inline]
    pub const fn splat(x: u32) -> Self {
        Self([x; 8])
    }

    /// Pack the first eight `Word32`s of a slice into a lane vector (shorter slices zero-fill).
    #[inline]
    pub fn from_words(s: &[Word32]) -> Self {
        let mut a = [0u32; 8];
        for (i, w) in s.iter().take(8).enumerate() {
            a[i] = w.0;
        }
        Self(a)
    }

    /// The lanes as eight `Word32`s.
    #[inline]
    pub fn to_words(self) -> [Word32; 8] {
        self.0.map(Word32)
    }

    /// Lane `i` (0-based) as a `Word32`.
    #[inline]
    pub fn lane(self, i: usize) -> Word32 {
        Word32(self.0[i])
    }

    /// Lane-wise XOR (`vpxor`). `#[inline(always)]` + compile-time `cfg(target_feature="avx2")`
    /// intrinsics so a hot Logos lane kernel (ChaCha/NTT) inlines register-resident under `+avx2`
    /// (no per-op `#[target_feature]` call boundary — that pessimizes ~20× on Keccak-scale kernels).
    #[inline(always)]
    pub fn bitxor(self, o: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            use std::arch::x86_64::*;
            let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
            let b = _mm256_loadu_si256(o.0.as_ptr() as *const __m256i);
            let mut r = [0u32; 8];
            _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, _mm256_xor_si256(a, b));
            return Self(r);
        }
        #[allow(unreachable_code)]
        {
            let mut r = [0u32; 8];
            for i in 0..8 {
                r[i] = self.0[i] ^ o.0[i];
            }
            Self(r)
        }
    }

    /// Scalar lane-wise reference implementations — the *spec* the AVX2 lowerings are fuzz-checked
    /// against. Test-only: each dispatched op inlines its own `#[allow(unreachable_code)]` scalar
    /// fallback, so these exist purely as the independent differential oracle.
    #[cfg(test)]
    #[inline]
    fn bitxor_scalar(self, o: Self) -> Self {
        Self(core::array::from_fn(|i| self.0[i] ^ o.0[i]))
    }
    #[cfg(test)]
    #[inline]
    fn add_scalar(self, o: Self) -> Self {
        Self(core::array::from_fn(|i| self.0[i].wrapping_add(o.0[i])))
    }
    #[cfg(test)]
    #[inline]
    fn sub_scalar(self, o: Self) -> Self {
        Self(core::array::from_fn(|i| self.0[i].wrapping_sub(o.0[i])))
    }
    #[cfg(test)]
    #[inline]
    fn rotl_scalar(self, n: u32) -> Self {
        Self(core::array::from_fn(|i| self.0[i].rotate_left(n)))
    }
    #[cfg(test)]
    #[inline]
    fn montmul32_scalar(self, b: Self, q: Self, qinv: Self) -> Self {
        Self(core::array::from_fn(|i| {
            let p = (self.0[i] as i32 as i64) * (b.0[i] as i32 as i64);
            let t = (p as i32).wrapping_mul(qinv.0[i] as i32) as i64;
            (((p - t * (q.0[i] as i32 as i64)) >> 32) as i32) as u32
        }))
    }

    /// Lane-wise AND (`_mm256_and_si256`) — the MD5 F/G-function bit mixing; LLVM lowers the loop to one
    /// `vpand`. AND/OR/NOT have no cross-lane dependency, so the scalar form auto-vectorizes cleanly.
    #[inline]
    pub fn bitand(self, o: Self) -> Self {
        let mut r = [0u32; 8];
        for i in 0..8 {
            r[i] = self.0[i] & o.0[i];
        }
        Self(r)
    }

    /// Lane-wise OR (`_mm256_or_si256`).
    #[inline]
    pub fn bitor(self, o: Self) -> Self {
        let mut r = [0u32; 8];
        for i in 0..8 {
            r[i] = self.0[i] | o.0[i];
        }
        Self(r)
    }

    /// Lane-wise complement (`vpxor` with all-ones) — MD5's `~b`/`~d` terms.
    #[inline]
    pub fn not(self) -> Self {
        let mut r = [0u32; 8];
        for i in 0..8 {
            r[i] = !self.0[i];
        }
        Self(r)
    }

    /// Lane-wise wrapping add in ℤ/2³² (`vpaddd`) — cfg-inline so it fuses into hot Logos lane kernels.
    #[inline(always)]
    pub fn add(self, o: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            use std::arch::x86_64::*;
            let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
            let b = _mm256_loadu_si256(o.0.as_ptr() as *const __m256i);
            let mut r = [0u32; 8];
            _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, _mm256_add_epi32(a, b));
            return Self(r);
        }
        #[allow(unreachable_code)]
        {
            let mut r = [0u32; 8];
            for i in 0..8 {
                r[i] = self.0[i].wrapping_add(o.0[i]);
            }
            Self(r)
        }
    }

    /// Lane-wise left rotation by `n` (ChaCha diffusion) — `(x<<n)|(x>>(32−n))` via `vpslld`/`vpsrld`.
    /// cfg-inline; `n` is taken mod 32 (n = 0 → the `32−n = 32` shift zeroes → identity).
    #[inline(always)]
    pub fn rotl(self, n: u32) -> Self {
        let n = n % 32;
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            use std::arch::x86_64::*;
            let x = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
            let l = _mm256_sll_epi32(x, _mm_cvtsi32_si128(n as i32));
            let r_sh = _mm256_srl_epi32(x, _mm_cvtsi32_si128((32 - n) as i32));
            let mut r = [0u32; 8];
            _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, _mm256_or_si256(l, r_sh));
            return Self(r);
        }
        #[allow(unreachable_code)]
        {
            let mut r = [0u32; 8];
            for i in 0..8 {
                r[i] = self.0[i].rotate_left(n);
            }
            Self(r)
        }
    }

    /// Lane-wise wrapping subtract in ℤ/2³² (`vpsubd`) — the i32 NTT butterfly's difference. cfg-inline.
    #[inline(always)]
    pub fn sub(self, o: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            use std::arch::x86_64::*;
            let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
            let b = _mm256_loadu_si256(o.0.as_ptr() as *const __m256i);
            let mut r = [0u32; 8];
            _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, _mm256_sub_epi32(a, b));
            return Self(r);
        }
        #[allow(unreachable_code)]
        {
            let mut r = [0u32; 8];
            for i in 0..8 {
                r[i] = self.0[i].wrapping_sub(o.0[i]);
            }
            Self(r)
        }
    }

    /// The signed i32 Montgomery multiply — per lane `montgomery_reduce(aᵢ·bᵢ) = (aᵢbᵢ − t·q)≫32`,
    /// `t = (aᵢbᵢ mod 2³²)·qinv`, the ML-DSA (Dilithium) NTT butterfly's multiply (q = 8380417,
    /// q,qinv broadcast). AVX2: `vpmuldq` the even and the (≫32) odd 32-bit lanes to eight i64
    /// products, reduce each (the result lands in the high 32 bits), recombine with `vpblendd`.
    #[inline(always)]
    pub fn montmul32(self, b: Self, q: Self, qinv: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            use std::arch::x86_64::*;
            let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
            let bb = _mm256_loadu_si256(b.0.as_ptr() as *const __m256i);
            let qv = _mm256_loadu_si256(q.0.as_ptr() as *const __m256i);
            let qiv = _mm256_loadu_si256(qinv.0.as_ptr() as *const __m256i);
            // Eight i64 products: even 32-bit lanes directly, odd lanes shifted into even position.
            let pe = _mm256_mul_epi32(a, bb);
            let po = _mm256_mul_epi32(_mm256_srli_epi64(a, 32), _mm256_srli_epi64(bb, 32));
            // Reduce: t = (p mod 2³²)·qinv; re = p − t·q lives in the high 32.
            let te = _mm256_mul_epi32(pe, qiv);
            let re = _mm256_sub_epi64(pe, _mm256_mul_epi32(te, qv));
            let to = _mm256_mul_epi32(po, qiv);
            let ro = _mm256_sub_epi64(po, _mm256_mul_epi32(to, qv));
            let res = _mm256_blend_epi32(_mm256_srli_epi64(re, 32), ro, 0xAA);
            let mut r = [0u32; 8];
            _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, res);
            return Self(r);
        }
        #[allow(unreachable_code)]
        {
            let qq = q.0[0] as i32 as i64;
            let qi = qinv.0[0] as i32;
            let mut r = [0u32; 8];
            for i in 0..8 {
                let p = (self.0[i] as i32 as i64) * (b.0[i] as i32 as i64);
                let t = (p as i32).wrapping_mul(qi) as i64;
                r[i] = (((p - t * qq) >> 32) as i32) as u32;
            }
            Self(r)
        }
    }

    /// Broadcast each `2h`-block's low `h` lanes into both halves — the within-vector NTT source-low
    /// duplication for 8 i32 lanes, stride `h ∈ {4,2,1}`. `h=4`→`vperm2i128(0x00)` (128-bit halves);
    /// `h=2`→`vpshufd(0x44)`; `h=1`→`vpshufd(0xA0)`. (The byte op is the i16 stride-`2h` shuffle.)
    #[inline(always)]
    pub fn ntt_bcast_lo(self, h: usize) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            use std::arch::x86_64::*;
            let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
            let v = match h {
                4 => _mm256_permute2x128_si256::<0x00>(a, a),
                2 => _mm256_shuffle_epi32::<0x44>(a),
                1 => _mm256_shuffle_epi32::<0xA0>(a),
                _ => unreachable!("i32 within-vector NTT stride is 4/2/1"),
            };
            let mut r = [0u32; 8];
            _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, v);
            return Self(r);
        }
        #[allow(unreachable_code)]
        Self(core::array::from_fn(|i| self.0[(i / (2 * h)) * (2 * h) + (i % (2 * h)) % h]))
    }

    /// Broadcast each `2h`-block's high `h` lanes into both halves. `h=4`→`vperm2i128(0x11)`;
    /// `h=2`→`vpshufd(0xEE)`; `h=1`→`vpshufd(0xF5)`.
    #[inline(always)]
    pub fn ntt_bcast_hi(self, h: usize) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            use std::arch::x86_64::*;
            let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
            let v = match h {
                4 => _mm256_permute2x128_si256::<0x11>(a, a),
                2 => _mm256_shuffle_epi32::<0xEE>(a),
                1 => _mm256_shuffle_epi32::<0xF5>(a),
                _ => unreachable!("i32 within-vector NTT stride is 4/2/1"),
            };
            let mut r = [0u32; 8];
            _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, v);
            return Self(r);
        }
        #[allow(unreachable_code)]
        Self(core::array::from_fn(|i| self.0[(i / (2 * h)) * (2 * h) + h + (i % (2 * h)) % h]))
    }

    /// Recombine the `+`/`−` halves: each `2h`-block's low `h` from `self`, high `h` from `o`.
    /// `h=4`→`vperm2i128(0x30)`; `h=2`→`vpblendd(0xCC)`; `h=1`→`vpblendd(0xAA)`.
    #[inline(always)]
    pub fn ntt_blend(self, o: Self, h: usize) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            use std::arch::x86_64::*;
            let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
            let b = _mm256_loadu_si256(o.0.as_ptr() as *const __m256i);
            let v = match h {
                4 => _mm256_permute2x128_si256::<0x30>(a, b),
                2 => _mm256_blend_epi32::<0xCC>(a, b),
                1 => _mm256_blend_epi32::<0xAA>(a, b),
                _ => unreachable!("i32 within-vector NTT stride is 4/2/1"),
            };
            let mut r = [0u32; 8];
            _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, v);
            return Self(r);
        }
        #[allow(unreachable_code)]
        Self(core::array::from_fn(|i| if (i % (2 * h)) < h { self.0[i] } else { o.0[i] }))
    }
}

// Operator traits delegate to the lane ops, so generated Rust writes the natural `v ^ w` / `v + w`
// and gets the AVX2 lowering — the same free-operator path Word uses.
impl ::core::ops::BitXor for Lanes8Word32 {
    type Output = Self;
    #[inline]
    fn bitxor(self, o: Self) -> Self {
        Lanes8Word32::bitxor(self, o)
    }
}
impl ::core::ops::Add for Lanes8Word32 {
    type Output = Self;
    #[inline]
    fn add(self, o: Self) -> Self {
        Lanes8Word32::add(self, o)
    }
}
impl ::core::ops::Sub for Lanes8Word32 {
    type Output = Self;
    #[inline]
    fn sub(self, o: Self) -> Self {
        Lanes8Word32::sub(self, o)
    }
}
impl ::core::ops::BitAnd for Lanes8Word32 {
    type Output = Self;
    #[inline]
    fn bitand(self, o: Self) -> Self {
        Lanes8Word32::bitand(self, o)
    }
}
impl ::core::ops::BitOr for Lanes8Word32 {
    type Output = Self;
    #[inline]
    fn bitor(self, o: Self) -> Self {
        Lanes8Word32::bitor(self, o)
    }
}
impl ::core::ops::Not for Lanes8Word32 {
    type Output = Self;
    #[inline]
    fn not(self) -> Self {
        Lanes8Word32::not(self)
    }
}

/// Four lanes of `Word32` = one 128-bit register (`__m128i`) — the SHA-1 state/message carrier. Unlike
/// the arithmetic lane types, its vocabulary is the four Intel SHA operations (`sha1rnds4`/`sha1msg1`/
/// `sha1msg2`/`sha1nexte`), so SHA-1 WRITTEN in Logos over these compiles to the `sha1rnds4` hardware
/// sequence (AOT) and runs the byte-identical software spec [`crate::sha_ops`] on the interpreter.
/// Lane `i` is bits `[32i+31 : 32i]` — index 0 low, index 3 high — matching `_mm_loadu_si128`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(C, align(16))]
pub struct Lanes4Word32(pub [u32; 4]);

impl Lanes4Word32 {
    /// The number of lanes.
    pub const LANES: usize = 4;

    /// Broadcast one value into all four lanes.
    #[inline]
    pub const fn splat(x: u32) -> Self {
        Self([x; 4])
    }

    /// Pack the first four `Word32`s of a slice (shorter slices zero-fill), lane 0 = element 0.
    #[inline]
    pub fn from_words(s: &[Word32]) -> Self {
        let mut a = [0u32; 4];
        for (i, w) in s.iter().take(4).enumerate() {
            a[i] = w.0;
        }
        Self(a)
    }

    /// The lanes as four `Word32`s.
    #[inline]
    pub fn to_words(self) -> [Word32; 4] {
        self.0.map(Word32)
    }

    /// Lane `i` (0-based) as a `Word32`.
    #[inline]
    pub fn lane(self, i: usize) -> Word32 {
        Word32(self.0[i])
    }

    /// Four SHA-1 rounds (`sha1rnds4`), `func` ∈ 0..=3 — the Intel SHA-NI instruction when the CPU has
    /// it, else the byte-identical software spec ([`crate::sha_ops`], proven equal by fuzz). `self` is
    /// the ABCD state, `msg` the four message dwords with the round's E folded in.
    #[inline(always)]
    pub fn sha1rnds4(self, msg: Self, func: u32) -> Self {
        // When the crate is built with SHA statically enabled (`target-cpu=native` or
        // `-C target-feature=+sha`), the intrinsic is callable WITHOUT a `#[target_feature]` boundary,
        // so this inlines and — chained across a compress — LLVM keeps the lanes in XMM registers
        // (no per-op detect branch, no memory round-trip). The `not` arm is the portable runtime-detect.
        #[cfg(all(target_arch = "x86_64", target_feature = "sha"))]
        unsafe {
            use core::arch::x86_64::*;
            let a: __m128i = core::mem::transmute(self.0);
            let b: __m128i = core::mem::transmute(msg.0);
            let r = match func & 3 {
                0 => _mm_sha1rnds4_epu32(a, b, 0),
                1 => _mm_sha1rnds4_epu32(a, b, 1),
                2 => _mm_sha1rnds4_epu32(a, b, 2),
                _ => _mm_sha1rnds4_epu32(a, b, 3),
            };
            Self(core::mem::transmute(r))
        }
        #[cfg(not(all(target_arch = "x86_64", target_feature = "sha")))]
        {
            #[cfg(target_arch = "x86_64")]
            {
                if shani_available() {
                    return unsafe { self.sha1rnds4_hw(msg, func) };
                }
            }
            Self(crate::sha_ops::sha1rnds4(self.0, msg.0, func))
        }
    }
    /// Message-schedule step 1 (`sha1msg1`).
    #[inline(always)]
    pub fn sha1msg1(self, o: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "sha"))]
        unsafe {
            use core::arch::x86_64::*;
            let a: __m128i = core::mem::transmute(self.0);
            let b: __m128i = core::mem::transmute(o.0);
            Self(core::mem::transmute(_mm_sha1msg1_epu32(a, b)))
        }
        #[cfg(not(all(target_arch = "x86_64", target_feature = "sha")))]
        {
            #[cfg(target_arch = "x86_64")]
            {
                if shani_available() {
                    return unsafe { self.sha1msg1_hw(o) };
                }
            }
            Self(crate::sha_ops::sha1msg1(self.0, o.0))
        }
    }
    /// Message-schedule step 2 (`sha1msg2`).
    #[inline(always)]
    pub fn sha1msg2(self, o: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "sha"))]
        unsafe {
            use core::arch::x86_64::*;
            let a: __m128i = core::mem::transmute(self.0);
            let b: __m128i = core::mem::transmute(o.0);
            Self(core::mem::transmute(_mm_sha1msg2_epu32(a, b)))
        }
        #[cfg(not(all(target_arch = "x86_64", target_feature = "sha")))]
        {
            #[cfg(target_arch = "x86_64")]
            {
                if shani_available() {
                    return unsafe { self.sha1msg2_hw(o) };
                }
            }
            Self(crate::sha_ops::sha1msg2(self.0, o.0))
        }
    }
    /// Fold the next round constant E (`sha1nexte`).
    #[inline(always)]
    pub fn sha1nexte(self, o: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "sha"))]
        unsafe {
            use core::arch::x86_64::*;
            let a: __m128i = core::mem::transmute(self.0);
            let b: __m128i = core::mem::transmute(o.0);
            Self(core::mem::transmute(_mm_sha1nexte_epu32(a, b)))
        }
        #[cfg(not(all(target_arch = "x86_64", target_feature = "sha")))]
        {
            #[cfg(target_arch = "x86_64")]
            {
                if shani_available() {
                    return unsafe { self.sha1nexte_hw(o) };
                }
            }
            Self(crate::sha_ops::sha1nexte(self.0, o.0))
        }
    }

    #[cfg(all(target_arch = "x86_64", not(target_feature = "sha")))]
    #[target_feature(enable = "sha,sse2,sse4.1")]
    unsafe fn sha1rnds4_hw(self, msg: Self, func: u32) -> Self {
        use std::arch::x86_64::*;
        let a = _mm_loadu_si128(self.0.as_ptr() as *const __m128i);
        let b = _mm_loadu_si128(msg.0.as_ptr() as *const __m128i);
        // The round-function selector is a compile-time immediate, so branch to the four forms.
        let r = match func & 3 {
            0 => _mm_sha1rnds4_epu32(a, b, 0),
            1 => _mm_sha1rnds4_epu32(a, b, 1),
            2 => _mm_sha1rnds4_epu32(a, b, 2),
            _ => _mm_sha1rnds4_epu32(a, b, 3),
        };
        let mut out = [0u32; 4];
        _mm_storeu_si128(out.as_mut_ptr() as *mut __m128i, r);
        Self(out)
    }

    #[cfg(all(target_arch = "x86_64", not(target_feature = "sha")))]
    #[target_feature(enable = "sha,sse2,ssse3,sse4.1")]
    unsafe fn sha1msg1_hw(self, o: Self) -> Self {
        use std::arch::x86_64::*;
        let a = _mm_loadu_si128(self.0.as_ptr() as *const __m128i);
        let b = _mm_loadu_si128(o.0.as_ptr() as *const __m128i);
        let mut out = [0u32; 4];
        _mm_storeu_si128(out.as_mut_ptr() as *mut __m128i, _mm_sha1msg1_epu32(a, b));
        Self(out)
    }

    #[cfg(all(target_arch = "x86_64", not(target_feature = "sha")))]
    #[target_feature(enable = "sha,sse2,ssse3,sse4.1")]
    unsafe fn sha1msg2_hw(self, o: Self) -> Self {
        use std::arch::x86_64::*;
        let a = _mm_loadu_si128(self.0.as_ptr() as *const __m128i);
        let b = _mm_loadu_si128(o.0.as_ptr() as *const __m128i);
        let mut out = [0u32; 4];
        _mm_storeu_si128(out.as_mut_ptr() as *mut __m128i, _mm_sha1msg2_epu32(a, b));
        Self(out)
    }

    #[cfg(all(target_arch = "x86_64", not(target_feature = "sha")))]
    #[target_feature(enable = "sha,sse2,sse4.1")]
    unsafe fn sha1nexte_hw(self, o: Self) -> Self {
        use std::arch::x86_64::*;
        let a = _mm_loadu_si128(self.0.as_ptr() as *const __m128i);
        let b = _mm_loadu_si128(o.0.as_ptr() as *const __m128i);
        let mut out = [0u32; 4];
        _mm_storeu_si128(out.as_mut_ptr() as *mut __m128i, _mm_sha1nexte_epu32(a, b));
        Self(out)
    }

    /// Lane-wise wrapping add (`_mm_add_epi32`) — folds the round E into the message dwords and the
    /// per-block state back into the running hash. LLVM lowers the four-lane loop to one `paddd`.
    #[inline]
    pub fn add(self, o: Self) -> Self {
        let mut out = [0u32; 4];
        for i in 0..4 {
            out[i] = self.0[i].wrapping_add(o.0[i]);
        }
        Self(out)
    }

    /// Lane-wise XOR (`_mm_xor_si128`) — the message-schedule W_t ⊕ W_{t-2} coupling; lowers to `pxor`.
    #[inline]
    pub fn bitxor(self, o: Self) -> Self {
        let mut out = [0u32; 4];
        for i in 0..4 {
            out[i] = self.0[i] ^ o.0[i];
        }
        Self(out)
    }
}

impl ::core::ops::Add for Lanes4Word32 {
    type Output = Self;
    #[inline]
    fn add(self, o: Self) -> Self {
        Lanes4Word32::add(self, o)
    }
}
impl ::core::ops::BitXor for Lanes4Word32 {
    type Output = Self;
    #[inline]
    fn bitxor(self, o: Self) -> Self {
        Lanes4Word32::bitxor(self, o)
    }
}

/// Sixteen `Word8` lanes = one 128-bit register (`__m128i`) — the BYTE-SHUFFLE carrier for SIMD text
/// codecs (hex encode/decode). Its vocabulary is `shuffle` (`pshufb`, a 16-entry byte LUT), byte AND,
/// per-byte shift, and the two byte interleaves — so a hex codec WRITTEN in Logos over it compiles to
/// the `pshufb` sequence (AOT, when SSSE3 is statically enabled) and runs the byte-identical scalar
/// spec on the interpreter. Lane `i` is byte `i` (index 0 low), matching `_mm_loadu_si128`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(C, align(16))]
pub struct Lanes16Word8(pub [u8; 16]);

impl Lanes16Word8 {
    /// The number of lanes.
    pub const LANES: usize = 16;

    /// Broadcast one byte into all sixteen lanes.
    #[inline]
    pub const fn splat(x: u8) -> Self {
        Self([x; 16])
    }

    /// Pack the first sixteen bytes of a slice (shorter slices zero-fill).
    #[inline]
    pub fn from_bytes(s: &[u8]) -> Self {
        let mut a = [0u8; 16];
        for (i, b) in s.iter().take(16).enumerate() {
            a[i] = *b;
        }
        Self(a)
    }

    /// The sixteen lane bytes.
    #[inline]
    pub fn to_bytes(self) -> [u8; 16] {
        self.0
    }

    /// Lane `i` (0-based).
    #[inline]
    pub fn lane(self, i: usize) -> u8 {
        self.0[i]
    }

    /// Byte shuffle (`pshufb`): `out[i] = if idx[i] & 0x80 { 0 } else { self[idx[i] & 0x0f] }` — a
    /// 16-entry byte lookup, the core of SIMD hex codecs (nibble → hex char; hyphen strip).
    #[inline]
    pub fn shuffle(self, idx: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))]
        unsafe {
            use core::arch::x86_64::*;
            let a: __m128i = core::mem::transmute(self.0);
            let b: __m128i = core::mem::transmute(idx.0);
            Self(core::mem::transmute(_mm_shuffle_epi8(a, b)))
        }
        #[cfg(not(all(target_arch = "x86_64", target_feature = "ssse3")))]
        {
            let mut r = [0u8; 16];
            for i in 0..16 {
                let x = idx.0[i];
                r[i] = if x & 0x80 != 0 { 0 } else { self.0[(x & 0x0f) as usize] };
            }
            Self(r)
        }
    }

    /// Lane-wise AND (`_mm_and_si128`) — the low-nibble mask; auto-vectorizes to `pand`.
    #[inline]
    pub fn bitand(self, o: Self) -> Self {
        let mut r = [0u8; 16];
        for i in 0..16 {
            r[i] = self.0[i] & o.0[i];
        }
        Self(r)
    }

    /// Per-byte logical shift right by `n` — the high-nibble extract (`v >> 4`).
    #[inline]
    pub fn shr_bytes(self, n: u32) -> Self {
        let mut r = [0u8; 16];
        for i in 0..16 {
            r[i] = self.0[i] >> n;
        }
        Self(r)
    }

    /// Interleave the low eight bytes (`_mm_unpacklo_epi8`): `[a0,b0,a1,b1,…,a7,b7]`.
    #[inline]
    pub fn interleave_lo(self, o: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))]
        unsafe {
            use core::arch::x86_64::*;
            let a: __m128i = core::mem::transmute(self.0);
            let b: __m128i = core::mem::transmute(o.0);
            Self(core::mem::transmute(_mm_unpacklo_epi8(a, b)))
        }
        #[cfg(not(all(target_arch = "x86_64", target_feature = "ssse3")))]
        {
            let mut r = [0u8; 16];
            for i in 0..8 {
                r[2 * i] = self.0[i];
                r[2 * i + 1] = o.0[i];
            }
            Self(r)
        }
    }

    /// Interleave the high eight bytes (`_mm_unpackhi_epi8`): `[a8,b8,…,a15,b15]`.
    #[inline]
    pub fn interleave_hi(self, o: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))]
        unsafe {
            use core::arch::x86_64::*;
            let a: __m128i = core::mem::transmute(self.0);
            let b: __m128i = core::mem::transmute(o.0);
            Self(core::mem::transmute(_mm_unpackhi_epi8(a, b)))
        }
        #[cfg(not(all(target_arch = "x86_64", target_feature = "ssse3")))]
        {
            let mut r = [0u8; 16];
            for i in 0..8 {
                r[2 * i] = self.0[8 + i];
                r[2 * i + 1] = o.0[8 + i];
            }
            Self(r)
        }
    }

    /// Per-byte wrapping add (`_mm_add_epi8`) — the ASCII→nibble decode (`lo + 9·hibit`).
    #[inline]
    pub fn byte_add(self, o: Self) -> Self {
        let mut r = [0u8; 16];
        for i in 0..16 {
            r[i] = self.0[i].wrapping_add(o.0[i]);
        }
        Self(r)
    }

    /// Multiply-add adjacent byte pairs (`pmaddubsw`): `self` unsigned × `o` signed, summing pairs into
    /// eight saturating `i16` lanes stored little-endian. Weights `[16,1,…]` fuse each nibble pair into
    /// the decoded byte (hex parse). Result carries 8 `u16` in its 16 bytes; narrow with `packus`.
    #[inline]
    pub fn maddubs(self, o: Self) -> Self {
        let mut r = [0u8; 16];
        for i in 0..8 {
            let a0 = self.0[2 * i] as i32;
            let a1 = self.0[2 * i + 1] as i32;
            let b0 = o.0[2 * i] as i8 as i32;
            let b1 = o.0[2 * i + 1] as i8 as i32;
            let s = (a0 * b0 + a1 * b1).clamp(-32768, 32767) as i16 as u16;
            r[2 * i] = (s & 0xff) as u8;
            r[2 * i + 1] = (s >> 8) as u8;
        }
        Self(r)
    }

    /// Pack two `8×i16` vectors to `16×u8` with unsigned saturation (`packuswb`): `self`'s eight 16-bit
    /// lanes → bytes 0–7, `o`'s → bytes 8–15. Narrows the `maddubs` output back to bytes.
    #[inline]
    pub fn packus(self, o: Self) -> Self {
        let mut r = [0u8; 16];
        for i in 0..8 {
            let a = (self.0[2 * i] as u16 | (self.0[2 * i + 1] as u16) << 8) as i16;
            r[i] = a.clamp(0, 255) as u8;
            let b = (o.0[2 * i] as u16 | (o.0[2 * i + 1] as u16) << 8) as i16;
            r[8 + i] = b.clamp(0, 255) as u8;
        }
        Self(r)
    }
}

impl ::core::ops::BitAnd for Lanes16Word8 {
    type Output = Self;
    #[inline]
    fn bitand(self, o: Self) -> Self {
        Lanes16Word8::bitand(self, o)
    }
}

/// True when the CPU has the SHA + SSE4.1 (+ SSSE3) instructions the SHA-NI lane ops use; `std` caches
/// the detection so this is a cheap load after the first call. Only the portable build path (SHA not
/// statically enabled) runtime-detects; a `target-feature=+sha` build skips it entirely.
#[cfg(all(target_arch = "x86_64", not(target_feature = "sha")))]
#[inline]
fn shani_available() -> bool {
    std::is_x86_feature_detected!("sha")
        && std::is_x86_feature_detected!("ssse3")
        && std::is_x86_feature_detected!("sse4.1")
}

/// Four lanes of `Word64` (one 256-bit SIMD register) — the Poly1305 accumulator lane vector.
/// Carries `[u64; 4]`; ops have an AVX2 fast path proven byte-identical to the scalar lanes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(C, align(32))]
pub struct Lanes4Word64(pub [u64; 4]);

impl Lanes4Word64 {
    /// The number of lanes.
    pub const LANES: usize = 4;

    /// Pack the first four `Word64`s of a slice into a lane vector (shorter slices zero-fill).
    #[inline]
    pub fn from_words(s: &[Word64]) -> Self {
        let mut a = [0u64; 4];
        for (i, w) in s.iter().take(4).enumerate() {
            a[i] = w.0;
        }
        Self(a)
    }

    /// The lanes as four `Word64`s.
    #[inline]
    pub fn to_words(self) -> [Word64; 4] {
        self.0.map(Word64)
    }

    /// Lane `i` (0-based) as a `Word64`.
    #[inline]
    pub fn lane(self, i: usize) -> Word64 {
        Word64(self.0[i])
    }

    /// The horizontal sum of the four lanes (wrapping in ℤ/2⁶⁴) — combines the per-lane partial
    /// products in a 4-way Poly1305 multiply. Reads the scalar array; no SIMD needed.
    #[inline]
    pub fn hsum(self) -> u64 {
        self.0[0]
            .wrapping_add(self.0[1])
            .wrapping_add(self.0[2])
            .wrapping_add(self.0[3])
    }

    /// Lane-wise wrapping add in ℤ/2⁶⁴ (`vpaddq`). `#[inline(always)]` + compile-time
    /// `cfg(target_feature="avx2")` (no runtime `is_x86_feature_detected` branch, no `#[target_feature]`
    /// call boundary) so the 4-way Poly1305 accumulator stays register-resident under `+avx2`.
    #[inline(always)]
    pub fn add(self, o: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            use std::arch::x86_64::*;
            let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
            let b = _mm256_loadu_si256(o.0.as_ptr() as *const __m256i);
            let mut r = [0u64; 4];
            _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, _mm256_add_epi64(a, b));
            return Self(r);
        }
        #[allow(unreachable_code)]
        {
            let mut r = [0u64; 4];
            for i in 0..4 {
                r[i] = self.0[i].wrapping_add(o.0[i]);
            }
            Self(r)
        }
    }

    #[cfg(test)]
    #[inline]
    fn add_scalar(self, o: Self) -> Self {
        let mut r = [0u64; 4];
        for i in 0..4 {
            r[i] = self.0[i].wrapping_add(o.0[i]);
        }
        Self(r)
    }

    /// Lane-wise widening multiply of the low 32 bits: `(aₗₒ·bₗₒ)` per lane → a 64-bit product
    /// (`vpmuludq`). `#[inline(always)]` + compile-time `cfg(target_feature="avx2")` so the Poly1305
    /// 4-way limb multiply inlines register-resident under `+avx2`.
    #[inline(always)]
    pub fn mul_lo32_wide(self, o: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            use std::arch::x86_64::*;
            let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
            let b = _mm256_loadu_si256(o.0.as_ptr() as *const __m256i);
            let mut r = [0u64; 4];
            _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, _mm256_mul_epu32(a, b));
            return Self(r);
        }
        #[allow(unreachable_code)]
        {
            let mut r = [0u64; 4];
            for i in 0..4 {
                r[i] = (self.0[i] & 0xffff_ffff) * (o.0[i] & 0xffff_ffff);
            }
            Self(r)
        }
    }

    #[cfg(test)]
    #[inline]
    fn mul_lo32_wide_scalar(self, o: Self) -> Self {
        let mut r = [0u64; 4];
        for i in 0..4 {
            r[i] = (self.0[i] & 0xffff_ffff) * (o.0[i] & 0xffff_ffff);
        }
        Self(r)
    }

    // ── Bitwise ops — the 4-way Keccak-f[1600] substrate (θ/ρ/π/χ/ι over 4 independent states) ─────
    // `#[inline(always)]` with COMPILE-TIME `cfg(target_feature="avx2")` intrinsics (NOT a separate
    // `#[target_feature]` fn) so they inline straight into a hot kernel (Keccak's ~600 ops) with the
    // vectors register-resident — no per-op call boundary, no per-op runtime detect. This is what
    // closes the ~20× gap of a `#[target_feature]`-per-op lane Keccak (measured). A build with
    // `-C target-feature=+avx2` / `target-cpu=native` takes the fast path; otherwise portable scalar.

    /// Broadcast one `u64` into all four lanes (Keccak's ι round-constant XOR is a splat).
    #[inline(always)]
    pub fn splat(x: u64) -> Self {
        Self([x; 4])
    }

    /// Lane-wise XOR (`vpxor`) — θ column parity, χ, and ι.
    #[inline(always)]
    pub fn bitxor(self, o: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            use std::arch::x86_64::*;
            let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
            let b = _mm256_loadu_si256(o.0.as_ptr() as *const __m256i);
            let mut r = [0u64; 4];
            _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, _mm256_xor_si256(a, b));
            return Self(r);
        }
        #[allow(unreachable_code)]
        Self([self.0[0] ^ o.0[0], self.0[1] ^ o.0[1], self.0[2] ^ o.0[2], self.0[3] ^ o.0[3]])
    }

    /// Lane-wise AND (`vpand`).
    #[inline(always)]
    pub fn and(self, o: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            use std::arch::x86_64::*;
            let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
            let b = _mm256_loadu_si256(o.0.as_ptr() as *const __m256i);
            let mut r = [0u64; 4];
            _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, _mm256_and_si256(a, b));
            return Self(r);
        }
        #[allow(unreachable_code)]
        Self([self.0[0] & o.0[0], self.0[1] & o.0[1], self.0[2] & o.0[2], self.0[3] & o.0[3]])
    }

    /// Lane-wise AND-NOT `(¬self) & o` (`vpandn`) — Keccak's χ nonlinearity `¬bᵢ₊₁ ∧ bᵢ₊₂` in one op.
    #[inline(always)]
    pub fn andnot(self, o: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            use std::arch::x86_64::*;
            let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
            let b = _mm256_loadu_si256(o.0.as_ptr() as *const __m256i);
            let mut r = [0u64; 4];
            _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, _mm256_andnot_si256(a, b));
            return Self(r);
        }
        #[allow(unreachable_code)]
        Self([!self.0[0] & o.0[0], !self.0[1] & o.0[1], !self.0[2] & o.0[2], !self.0[3] & o.0[3]])
    }

    /// Lane-wise left rotation by `n` (mod 64) — Keccak's ρ offsets and θ's D term. `(x<<n)|(x>>(64−n))`
    /// via `vpsllq`/`vpsrlq` (n = 0 is the identity — the `64−n = 64` shift zeroes).
    #[inline(always)]
    pub fn rotl(self, n: u32) -> Self {
        let n = n % 64;
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            use std::arch::x86_64::*;
            if n == 0 {
                return self;
            }
            let x = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
            let l = _mm256_sll_epi64(x, _mm_cvtsi32_si128(n as i32));
            let r_sh = _mm256_srl_epi64(x, _mm_cvtsi32_si128((64 - n) as i32));
            let mut r = [0u64; 4];
            _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, _mm256_or_si256(l, r_sh));
            return Self(r);
        }
        #[allow(unreachable_code)]
        {
            let mut r = [0u64; 4];
            for i in 0..4 {
                r[i] = self.0[i].rotate_left(n);
            }
            Self(r)
        }
    }
}

impl ::core::ops::Add for Lanes4Word64 {
    type Output = Self;
    #[inline]
    fn add(self, o: Self) -> Self {
        Lanes4Word64::add(self, o)
    }
}

impl ::core::ops::BitXor for Lanes4Word64 {
    type Output = Self;
    #[inline]
    fn bitxor(self, o: Self) -> Self {
        Lanes4Word64::bitxor(self, o)
    }
}

impl ::core::ops::BitAnd for Lanes4Word64 {
    type Output = Self;
    #[inline]
    fn bitand(self, o: Self) -> Self {
        Lanes4Word64::and(self, o)
    }
}

/// Sixteen lanes of `Word16` (one 256-bit SIMD register) — the NTT coefficient lane vector. Carries
/// `[u16; 16]`; the multiply-high is the SIGNED `_mm256_mulhi_epi16` (the Montgomery butterfly's
/// `mulhi`). Ops have an AVX2 fast path proven byte-identical to the scalar lanes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(C, align(32))]
pub struct Lanes16Word16(pub [u16; 16]);

/// The AVX2 256-bit lane vector — the real `__m256i` intrinsic on x86_64, a placeholder elsewhere.
/// The SIMD fast path is `#[cfg(target_arch = "x86_64")]`-compiled out on other targets (e.g. the
/// `wasm32` web build), so the placeholder is never constructed; this keeps the lane ops' signatures
/// and closures arch-independent while the scalar fallback runs everywhere.
#[cfg(target_arch = "x86_64")]
type Avx256 = std::arch::x86_64::__m256i;
#[cfg(not(target_arch = "x86_64"))]
type Avx256 = [u16; 16];

// The four AVX2 lane butterflies, isolated behind safe wrappers so the lane methods' closures stay
// arch-independent. On x86_64 each is the intrinsic (the caller `binop` runtime-detects avx2 before
// invoking it); on other targets the SIMD path is `#[cfg]`-compiled out, so these are never called.
#[cfg(target_arch = "x86_64")]
#[inline]
fn simd_add(a: Avx256, b: Avx256) -> Avx256 {
    unsafe { std::arch::x86_64::_mm256_add_epi16(a, b) }
}
#[cfg(target_arch = "x86_64")]
#[inline]
fn simd_sub(a: Avx256, b: Avx256) -> Avx256 {
    unsafe { std::arch::x86_64::_mm256_sub_epi16(a, b) }
}
#[cfg(target_arch = "x86_64")]
#[inline]
fn simd_mullo(a: Avx256, b: Avx256) -> Avx256 {
    unsafe { std::arch::x86_64::_mm256_mullo_epi16(a, b) }
}
#[cfg(target_arch = "x86_64")]
#[inline]
fn simd_mulhi(a: Avx256, b: Avx256) -> Avx256 {
    unsafe { std::arch::x86_64::_mm256_mulhi_epi16(a, b) }
}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
fn simd_add(_a: Avx256, _b: Avx256) -> Avx256 {
    unreachable!("SIMD lane path is x86_64-only")
}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
fn simd_sub(_a: Avx256, _b: Avx256) -> Avx256 {
    unreachable!("SIMD lane path is x86_64-only")
}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
fn simd_mullo(_a: Avx256, _b: Avx256) -> Avx256 {
    unreachable!("SIMD lane path is x86_64-only")
}
#[cfg(not(target_arch = "x86_64"))]
#[inline]
fn simd_mulhi(_a: Avx256, _b: Avx256) -> Avx256 {
    unreachable!("SIMD lane path is x86_64-only")
}

impl Lanes16Word16 {
    /// The number of lanes.
    pub const LANES: usize = 16;

    /// Broadcast one value into all sixteen lanes.
    #[inline]
    pub const fn splat(x: u16) -> Self {
        Self([x; 16])
    }

    /// Pack the first sixteen `Word16`s of a slice into a lane vector (shorter slices zero-fill).
    #[inline]
    pub fn from_words(s: &[Word16]) -> Self {
        let mut a = [0u16; 16];
        for (i, w) in s.iter().take(16).enumerate() {
            a[i] = w.0;
        }
        Self(a)
    }

    /// The lanes as sixteen `Word16`s.
    #[inline]
    pub fn to_words(self) -> [Word16; 16] {
        self.0.map(Word16)
    }

    /// Lane `i` (0-based) as a `Word16`.
    #[inline]
    pub fn lane(self, i: usize) -> Word16 {
        Word16(self.0[i])
    }

    /// Lane-wise wrapping add in ℤ/2¹⁶ (`vpaddw`).
    #[inline(always)]
    pub fn add(self, o: Self) -> Self {
        self.binop(o, |a, b| a.wrapping_add(b), |a, b| simd_add(a, b))
    }

    /// Lane-wise wrapping subtract in ℤ/2¹⁶ (`vpsubw`).
    #[inline(always)]
    pub fn sub(self, o: Self) -> Self {
        self.binop(o, |a, b| a.wrapping_sub(b), |a, b| simd_sub(a, b))
    }

    /// Lane-wise low 16 bits of the product (`vpmullw`) — interpretation-independent.
    #[inline(always)]
    pub fn mullo(self, o: Self) -> Self {
        self.binop(o, |a, b| a.wrapping_mul(b), |a, b| simd_mullo(a, b))
    }

    /// Lane-wise high 16 bits of the SIGNED product (`vpmulhw`) — the Montgomery butterfly's `mulhi`.
    #[inline(always)]
    pub fn mulhi(self, o: Self) -> Self {
        self.binop(
            o,
            |a, b| (((a as i16 as i32) * (b as i16 as i32)) >> 16) as u16,
            |a, b| simd_mulhi(a, b),
        )
    }

    /// Broadcast each `2h`-block's low `h` lanes into both of its halves — the within-vector NTT
    /// butterfly's source-low duplication, at stride `h ∈ {8,4,2}`. `h=8`→`vperm2i128(v,v,0x00)`;
    /// `h=4`→`vpshufd(v,0x44)`; `h=2`→`vpshufd(v,0xA0)` (all per-128-bit-lane, so the multiple
    /// blocks packed in one register are handled at once).
    #[inline(always)]
    pub fn ntt_bcast_lo(self, h: usize) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        {
            use std::arch::x86_64::*;
            let mut r = [0u16; 16];
            unsafe {
                let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
                let v = match h {
                    8 => _mm256_permute2x128_si256::<0x00>(a, a),
                    4 => _mm256_shuffle_epi32::<0x44>(a),
                    2 => _mm256_shuffle_epi32::<0xA0>(a),
                    _ => unreachable!("within-vector NTT stride is 8/4/2"),
                };
                _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, v);
            }
            return Self(r);
        }
        #[allow(unreachable_code)]
        Self(core::array::from_fn(|i| self.0[(i / (2 * h)) * (2 * h) + (i % (2 * h)) % h]))
    }

    /// Broadcast each `2h`-block's high `h` lanes into both of its halves — the source-high
    /// duplication. `h=8`→`vperm2i128(v,v,0x11)`; `h=4`→`vpshufd(v,0xEE)`; `h=2`→`vpshufd(v,0xF5)`.
    #[inline(always)]
    pub fn ntt_bcast_hi(self, h: usize) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        {
            use std::arch::x86_64::*;
            let mut r = [0u16; 16];
            unsafe {
                let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
                let v = match h {
                    8 => _mm256_permute2x128_si256::<0x11>(a, a),
                    4 => _mm256_shuffle_epi32::<0xEE>(a),
                    2 => _mm256_shuffle_epi32::<0xF5>(a),
                    _ => unreachable!("within-vector NTT stride is 8/4/2"),
                };
                _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, v);
            }
            return Self(r);
        }
        #[allow(unreachable_code)]
        Self(core::array::from_fn(|i| self.0[(i / (2 * h)) * (2 * h) + h + (i % (2 * h)) % h]))
    }

    /// Recombine the `+`/`−` halves of the within-vector butterfly: each `2h`-block's low `h` lanes
    /// from `self`, high `h` from `o`. `h=8`→`vperm2i128(a,b,0x30)`; `h=4`→`vpblendd(a,b,0xCC)`;
    /// `h=2`→`vpblendd(a,b,0xAA)`.
    #[inline(always)]
    pub fn ntt_blend(self, o: Self, h: usize) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        {
            use std::arch::x86_64::*;
            let mut r = [0u16; 16];
            unsafe {
                let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
                let b = _mm256_loadu_si256(o.0.as_ptr() as *const __m256i);
                let v = match h {
                    8 => _mm256_permute2x128_si256::<0x30>(a, b),
                    4 => _mm256_blend_epi32::<0xCC>(a, b),
                    2 => _mm256_blend_epi32::<0xAA>(a, b),
                    _ => unreachable!("within-vector NTT stride is 8/4/2"),
                };
                _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, v);
            }
            return Self(r);
        }
        #[allow(unreachable_code)]
        Self(core::array::from_fn(|i| if (i % (2 * h)) < h { self.0[i] } else { o.0[i] }))
    }

    /// Shared add/sub/mullo/mulhi dispatcher. `#[inline(always)]` + compile-time
    /// `cfg(target_feature="avx2")` (NOT a runtime `is_x86_feature_detected` branch) so a chain of
    /// NTT butterflies inlines register-resident under `+avx2`: LLVM forwards each op's store-to-`r`
    /// into the next op's load and keeps the coefficients in one YMM register. A runtime branch is
    /// opaque to that forwarding — it forces a load/store round-trip per op (the ~20× lane pessimization).
    #[inline(always)]
    fn binop(
        self,
        o: Self,
        scalar: impl Fn(u16, u16) -> u16,
        #[allow(unused_variables)] vector: impl Fn(Avx256, Avx256) -> Avx256,
    ) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        {
            use std::arch::x86_64::*;
            let mut r = [0u16; 16];
            unsafe {
                let a = _mm256_loadu_si256(self.0.as_ptr() as *const __m256i);
                let b = _mm256_loadu_si256(o.0.as_ptr() as *const __m256i);
                _mm256_storeu_si256(r.as_mut_ptr() as *mut __m256i, vector(a, b));
            }
            return Self(r);
        }
        #[allow(unreachable_code)]
        Self(core::array::from_fn(|i| scalar(self.0[i], o.0[i])))
    }
}

macro_rules! lanes16_op {
    ($trait:ident, $tm:ident, $m:ident) => {
        impl ::core::ops::$trait for Lanes16Word16 {
            type Output = Self;
            #[inline]
            fn $tm(self, o: Self) -> Self {
                Lanes16Word16::$m(self, o)
            }
        }
    };
}
lanes16_op!(Add, add, add);
lanes16_op!(Sub, sub, sub);
lanes16_op!(Mul, mul, mullo);

/// A SIMD lane vector of any supported lane config — the single runtime carrier the interpreter/VM
/// match on, mirroring [`WordVal`]. Binary ops require the same config; a mismatch returns `None`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LanesVal {
    /// 8 lanes of `Word32` (one `__m256i`).
    L8W32(Lanes8Word32),
    /// 4 lanes of `Word64` (one `__m256i`) — the Poly1305 accumulator.
    L4W64(Lanes4Word64),
    /// 16 lanes of `Word16` (one `__m256i`) — NTT coefficients.
    L16W16(Lanes16Word16),
    /// 4 lanes of `Word32` (one `__m128i`) — the SHA-1 state/message register (SHA-NI ops).
    L4W32(Lanes4Word32),
    /// 16 lanes of `Word8` (one `__m128i`) — the byte-shuffle register for SIMD text codecs.
    L16W8(Lanes16Word8),
}

impl LanesVal {
    /// The lane count.
    #[inline]
    pub const fn lanes(self) -> usize {
        match self {
            LanesVal::L8W32(_) => 8,
            LanesVal::L4W64(_) => 4,
            LanesVal::L16W16(_) => 16,
            LanesVal::L4W32(_) => 4,
            LanesVal::L16W8(_) => 16,
        }
    }

    /// The lane-element bit width.
    #[inline]
    pub const fn elem_bits(self) -> u32 {
        match self {
            LanesVal::L8W32(_) => 32,
            LanesVal::L4W64(_) => 64,
            LanesVal::L16W16(_) => 16,
            LanesVal::L4W32(_) => 32,
            LanesVal::L16W8(_) => 8,
        }
    }

    /// The Logos type name.
    #[inline]
    pub const fn type_name(self) -> &'static str {
        match self {
            LanesVal::L8W32(_) => "Lanes8Word32",
            LanesVal::L4W64(_) => "Lanes4Word64",
            LanesVal::L16W16(_) => "Lanes16Word16",
            LanesVal::L4W32(_) => "Lanes4Word32",
            LanesVal::L16W8(_) => "Lanes16Word8",
        }
    }

    /// Lane `i`'s value, zero-extended to `u64` (for unpacking back into a Seq and for display).
    #[inline]
    pub fn lane(self, i: usize) -> u64 {
        match self {
            LanesVal::L8W32(v) => v.lane(i).0 as u64,
            LanesVal::L4W64(v) => v.lane(i).0,
            LanesVal::L16W16(v) => v.lane(i).0 as u64,
            LanesVal::L4W32(v) => v.lane(i).0 as u64,
            LanesVal::L16W8(v) => v.lane(i) as u64,
        }
    }

    /// Byte-shuffle vocabulary, defined only on the `L16W8` config (SIMD hex codec); `None` otherwise.
    #[inline]
    pub fn shuffle(self, idx: Self) -> Option<Self> {
        match (self, idx) {
            (LanesVal::L16W8(a), LanesVal::L16W8(b)) => Some(LanesVal::L16W8(a.shuffle(b))),
            _ => None,
        }
    }
    #[inline]
    pub fn byte_and(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L16W8(a), LanesVal::L16W8(b)) => Some(LanesVal::L16W8(a.bitand(b))),
            _ => None,
        }
    }
    #[inline]
    pub fn shr_bytes(self, n: u32) -> Option<Self> {
        match self {
            LanesVal::L16W8(a) => Some(LanesVal::L16W8(a.shr_bytes(n))),
            _ => None,
        }
    }
    #[inline]
    pub fn interleave_lo(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L16W8(a), LanesVal::L16W8(b)) => Some(LanesVal::L16W8(a.interleave_lo(b))),
            _ => None,
        }
    }
    #[inline]
    pub fn interleave_hi(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L16W8(a), LanesVal::L16W8(b)) => Some(LanesVal::L16W8(a.interleave_hi(b))),
            _ => None,
        }
    }
    #[inline]
    pub fn byte_add(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L16W8(a), LanesVal::L16W8(b)) => Some(LanesVal::L16W8(a.byte_add(b))),
            _ => None,
        }
    }
    #[inline]
    pub fn maddubs_bytes(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L16W8(a), LanesVal::L16W8(b)) => Some(LanesVal::L16W8(a.maddubs(b))),
            _ => None,
        }
    }
    #[inline]
    pub fn packus_bytes(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L16W8(a), LanesVal::L16W8(b)) => Some(LanesVal::L16W8(a.packus(b))),
            _ => None,
        }
    }

    /// The four SHA-1 (SHA-NI) operations, defined only on the `L4W32` config; `None` otherwise.
    #[inline]
    pub fn sha1rnds4(self, msg: Self, func: u32) -> Option<Self> {
        match (self, msg) {
            (LanesVal::L4W32(a), LanesVal::L4W32(b)) => Some(LanesVal::L4W32(a.sha1rnds4(b, func))),
            _ => None,
        }
    }
    #[inline]
    pub fn sha1msg1(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L4W32(a), LanesVal::L4W32(b)) => Some(LanesVal::L4W32(a.sha1msg1(b))),
            _ => None,
        }
    }
    #[inline]
    pub fn sha1msg2(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L4W32(a), LanesVal::L4W32(b)) => Some(LanesVal::L4W32(a.sha1msg2(b))),
            _ => None,
        }
    }
    #[inline]
    pub fn sha1nexte(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L4W32(a), LanesVal::L4W32(b)) => Some(LanesVal::L4W32(a.sha1nexte(b))),
            _ => None,
        }
    }

    /// Lane-wise XOR; `None` on a config mismatch (or a config that does not define XOR).
    #[inline]
    pub fn bitxor(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L8W32(a), LanesVal::L8W32(b)) => Some(LanesVal::L8W32(a.bitxor(b))),
            (LanesVal::L4W32(a), LanesVal::L4W32(b)) => Some(LanesVal::L4W32(a.bitxor(b))),
            _ => None,
        }
    }

    /// Lane-wise AND (the MD5 F/G mixing) — `None` on a config mismatch or a config without AND.
    #[inline]
    pub fn bitand(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L8W32(a), LanesVal::L8W32(b)) => Some(LanesVal::L8W32(a.bitand(b))),
            (LanesVal::L16W8(a), LanesVal::L16W8(b)) => Some(LanesVal::L16W8(a.bitand(b))),
            _ => None,
        }
    }

    /// Lane-wise OR — `None` on a config mismatch or a config without OR.
    #[inline]
    pub fn bitor(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L8W32(a), LanesVal::L8W32(b)) => Some(LanesVal::L8W32(a.bitor(b))),
            _ => None,
        }
    }

    /// Lane-wise complement — `None` for a config without NOT.
    #[inline]
    pub fn lane_not(self) -> Option<Self> {
        match self {
            LanesVal::L8W32(a) => Some(LanesVal::L8W32(a.not())),
            _ => None,
        }
    }

    /// Lane-wise wrapping add; `None` on a config mismatch.
    #[inline]
    pub fn add(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L8W32(a), LanesVal::L8W32(b)) => Some(LanesVal::L8W32(a.add(b))),
            (LanesVal::L4W64(a), LanesVal::L4W64(b)) => Some(LanesVal::L4W64(a.add(b))),
            (LanesVal::L16W16(a), LanesVal::L16W16(b)) => Some(LanesVal::L16W16(a.add(b))),
            (LanesVal::L4W32(a), LanesVal::L4W32(b)) => Some(LanesVal::L4W32(a.add(b))),
            _ => None,
        }
    }

    /// Lane-wise wrapping subtract; `None` on a config mismatch (the i16 NTT and the i32 ML-DSA NTT).
    #[inline]
    pub fn sub(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L16W16(a), LanesVal::L16W16(b)) => Some(LanesVal::L16W16(a.sub(b))),
            (LanesVal::L8W32(a), LanesVal::L8W32(b)) => Some(LanesVal::L8W32(a.sub(b))),
            _ => None,
        }
    }

    /// Lane-wise low-16 multiply (`vpmullw`); `None` unless `Word16`.
    #[inline]
    pub fn mullo(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L16W16(a), LanesVal::L16W16(b)) => Some(LanesVal::L16W16(a.mullo(b))),
            _ => None,
        }
    }

    /// Lane-wise SIGNED high-16 multiply (`vpmulhw`, the Montgomery `mulhi`); `None` unless `Word16`.
    #[inline]
    pub fn mulhi(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L16W16(a), LanesVal::L16W16(b)) => Some(LanesVal::L16W16(a.mulhi(b))),
            _ => None,
        }
    }

    /// Within-vector NTT source-low duplication at stride `h` (`vperm2i128`/`vpshufd`); `None`
    /// unless `Word16`.
    #[inline]
    pub fn ntt_bcast_lo(self, h: usize) -> Option<Self> {
        match self {
            LanesVal::L16W16(a) => Some(LanesVal::L16W16(a.ntt_bcast_lo(h))),
            LanesVal::L8W32(a) => Some(LanesVal::L8W32(a.ntt_bcast_lo(h))),
            _ => None,
        }
    }

    /// Within-vector NTT source-high duplication at stride `h`; `None` unless `Word16`/`Word32`.
    #[inline]
    pub fn ntt_bcast_hi(self, h: usize) -> Option<Self> {
        match self {
            LanesVal::L16W16(a) => Some(LanesVal::L16W16(a.ntt_bcast_hi(h))),
            LanesVal::L8W32(a) => Some(LanesVal::L8W32(a.ntt_bcast_hi(h))),
            _ => None,
        }
    }

    /// Within-vector NTT half-recombine at stride `h` (`vperm2i128`/`vpblendd`); `None` unless both
    /// are `Word16`/`Word32`.
    #[inline]
    pub fn ntt_blend(self, o: Self, h: usize) -> Option<Self> {
        match (self, o) {
            (LanesVal::L16W16(a), LanesVal::L16W16(b)) => Some(LanesVal::L16W16(a.ntt_blend(b, h))),
            (LanesVal::L8W32(a), LanesVal::L8W32(b)) => Some(LanesVal::L8W32(a.ntt_blend(b, h))),
            _ => None,
        }
    }

    /// Lane-wise left rotation by `n`; `None` for a config that does not define rotation.
    #[inline]
    pub fn rotl(self, n: u32) -> Option<Self> {
        match self {
            LanesVal::L8W32(a) => Some(LanesVal::L8W32(a.rotl(n))),
            _ => None,
        }
    }

    /// Lane-wise widening low-32 multiply (the Poly1305 4-way limb product); `None` unless `Word64`.
    #[inline]
    pub fn mul_lo32_wide(self, o: Self) -> Option<Self> {
        match (self, o) {
            (LanesVal::L4W64(a), LanesVal::L4W64(b)) => Some(LanesVal::L4W64(a.mul_lo32_wide(b))),
            _ => None,
        }
    }

    /// The signed i32 Montgomery multiply (ML-DSA NTT butterfly), `q`/`qinv` broadcast; `None` unless
    /// all four are `Word32`.
    #[inline]
    pub fn montmul32(self, b: Self, q: Self, qinv: Self) -> Option<Self> {
        match (self, b, q, qinv) {
            (LanesVal::L8W32(a), LanesVal::L8W32(b), LanesVal::L8W32(q), LanesVal::L8W32(qi)) => {
                Some(LanesVal::L8W32(a.montmul32(b, q, qi)))
            }
            _ => None,
        }
    }

    /// Horizontal sum of the lanes as an `Int`.
    #[inline]
    pub fn hsum(self) -> i64 {
        match self {
            LanesVal::L8W32(a) => (0..8).map(|i| a.lane(i).0 as i64).sum(),
            LanesVal::L4W64(a) => a.hsum() as i64,
            LanesVal::L16W16(a) => (0..16).map(|i| a.lane(i).0 as i64).sum(),
            LanesVal::L4W32(a) => (0..4).map(|i| a.lane(i).0 as i64).sum(),
            LanesVal::L16W8(a) => (0..16).map(|i| a.lane(i) as i64).sum(),
        }
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(Word32(0x1234_5678).rem(Word32(65536)), Word32(0x5678), "% 2^16 masks the low bits");
        assert_eq!(Word32(0x1234_5678).div(Word32(65536)), Word32(0x1234), "/ 2^16 shifts right 16");
        assert_eq!(Word32(7).div(Word32(2)), Word32(3), "unsigned truncating division");
        assert_eq!(Word32(7).rem(Word32(3)), Word32(1), "unsigned remainder");
        assert_eq!(Word32(1).shl(31), Word32(0x8000_0000), "<< 31 sets the top bit");
        assert_eq!(Word32(0x8000_0000).shr(31), Word32(1), ">> 31 is a logical (unsigned) shift");
        assert_eq!(Word64(1).shl(63).shr(63), Word64(1), "Word64 logical shift round-trips the top bit");
        assert_eq!(Word8(200).div(Word8(8)), Word8(25), "Word8 unsigned division (200 ÷ 8)");
    }

    #[test]
    fn rotl_is_cyclic_and_inverse_of_rotr() {
        let x = Word32(0x1234_5678);
        assert_eq!(x.rotl(8), Word32(0x3456_7812), "rotl by 8 is the documented permutation");
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
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
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
            assert_eq!(a.rotl_scalar(n % 32), rot_exp, "scalar lane rotl is the spec");
            assert_eq!(a.rotl(n), rot_exp, "dispatched (AVX2) rotl == spec (n mod 32)");
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
            assert_eq!(ar.montmul32_scalar(br, qv, qiv), mont_exp, "scalar montmul32 is the spec");
            assert_eq!(ar.montmul32(br, qv, qiv), mont_exp, "dispatched (AVX2) montmul32 == spec");
            // The i32 within-vector-NTT permutes at every stride (h=4 vperm2i128, h=2/1 vpshufd/vpblendd).
            for &h in &[4usize, 2, 1] {
                let bl = Lanes8Word32(core::array::from_fn(|i| a.0[(i / (2 * h)) * (2 * h) + (i % (2 * h)) % h]));
                let bh = Lanes8Word32(core::array::from_fn(|i| a.0[(i / (2 * h)) * (2 * h) + h + (i % (2 * h)) % h]));
                let bd = Lanes8Word32(core::array::from_fn(|i| if (i % (2 * h)) < h { a.0[i] } else { b.0[i] }));
                assert_eq!(a.ntt_bcast_lo(h), bl, "i32 ntt_bcast_lo({h}) (AVX2) == scalar gather");
                assert_eq!(a.ntt_bcast_hi(h), bh, "i32 ntt_bcast_hi({h}) (AVX2) == scalar gather");
                assert_eq!(a.ntt_blend(b, h), bd, "i32 ntt_blend({h}) (AVX2) == scalar gather");
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
            let mul_exp =
                Lanes4Word64(core::array::from_fn(|i| (a.0[i] & 0xffff_ffff) * (b.0[i] & 0xffff_ffff)));
            assert_eq!(a.mul_lo32_wide_scalar(b), mul_exp, "scalar widening mul is the spec");
            assert_eq!(a.mul_lo32_wide(b), mul_exp, "dispatched (AVX2) vpmuludq == spec");
            // Horizontal sum.
            let hs = a.0[0].wrapping_add(a.0[1]).wrapping_add(a.0[2]).wrapping_add(a.0[3]);
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
                let bl = Lanes16Word16(core::array::from_fn(|i| a.0[(i / (2 * h)) * (2 * h) + (i % (2 * h)) % h]));
                let bh = Lanes16Word16(core::array::from_fn(|i| a.0[(i / (2 * h)) * (2 * h) + h + (i % (2 * h)) % h]));
                let bd = Lanes16Word16(core::array::from_fn(|i| if (i % (2 * h)) < h { a.0[i] } else { b.0[i] }));
                assert_eq!(a.ntt_bcast_lo(h), bl, "ntt_bcast_lo({h}) (AVX2) == scalar gather");
                assert_eq!(a.ntt_bcast_hi(h), bh, "ntt_bcast_hi({h}) (AVX2) == scalar gather");
                assert_eq!(a.ntt_blend(b, h), bd, "ntt_blend({h}) (AVX2) == scalar gather");
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
        assert_eq!(a.add(b), Some(WordVal::W32(Word32(0))), "same-width add wraps");
        assert_eq!(a.width(), 32);
        assert_eq!(a.to_u64(), 0xFFFF_FFFF);

        let c = WordVal::W64(Word64(1));
        assert_eq!(a.add(c), None, "mixed-width add is a type error");
        assert_eq!(a.bitxor(c), None, "mixed-width xor is a type error");

        // Unary ops preserve width.
        assert_eq!(WordVal::W64(Word64(0)).not(), WordVal::W64(Word64(u64::MAX)));
        assert_eq!(WordVal::W32(Word32(0x1234_5678)).rotl(8), WordVal::W32(Word32(0x3456_7812)));

        // from_u64 / width round-trip.
        assert_eq!(WordVal::from_u64(32, 0x1_0000_0005), Some(WordVal::W32(Word32(5))), "32 truncates");
        assert_eq!(WordVal::from_u64(64, 5), Some(WordVal::W64(Word64(5))));
        assert_eq!(WordVal::from_u64(16, 5), None, "only 32/64 are valid widths");

        // Display is the unsigned decimal value.
        assert_eq!(WordVal::W32(Word32(42)).to_string(), "42");
    }
}
