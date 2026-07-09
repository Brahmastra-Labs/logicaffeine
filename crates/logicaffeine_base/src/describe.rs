//! # Integer-sequence description-length codec (the MDL primitive)
//!
//! A pure, self-contained engine that describes an `&[i64]` by the **shortest** program in a fixed
//! menu of closed-form generators — affine (`base + i·stride`), geometric (`base · ratioⁱ`), a
//! degree-≤4 polynomial (finite-difference seeds), a periodic block, a sparse dominant-value form, a
//! sandboxed [`GenExpr`] generator, and the columnar fallbacks (delta, delta-of-delta,
//! frame-of-reference bit-pack, run-length, dictionary, raw byte column, plain zig-zag varint). Each
//! candidate is a complete self-delimiting byte string; [`consider`] keeps the smallest, so the
//! result is **never larger than plain varint**.
//!
//! [`describe_int_seq`] is a *computable upper bound on the Kolmogorov complexity* of the sequence
//! over this description language, and [`decode_int_seq`] is its exact inverse — decode reproduces
//! the sequence bit-for-bit, so the encoded bytes are a **re-checkable witness** for that bound.
//!
//! This is the same codec the wire layer (`logicaffeine_compile`'s `marshal`) uses for its
//! `WireStructure::Auto` int columns; it lives here in the leaf crate so both the wire codec and the
//! proof layer share one implementation of the format. The crate has no I/O and no receiver policy,
//! so the DoS bound (`max_elements`) is a **parameter**, supplied by each caller.

// ---- Tuning + format constants -------------------------------------------------------------

/// A cap on a length prefix's pre-allocation, so a corrupt huge count can't ask for gigabytes up
/// front; the actual reads still bound-check every element.
const PREALLOC_CAP: usize = 4096;

/// The longest repeating block the periodic detector will consider.
const PERIOD_CAP: usize = 512;

/// The highest polynomial degree the generator detector will fit (degree 1 is the affine case).
pub const MAX_POLY_DEGREE: usize = 4;

/// The highest constant-coefficient linear-recurrence order the detector will fit. Order 2 already
/// covers Fibonacci / Lucas / Pell and any LFSR of that length (this is Berlekamp–Massey over ℤ).
pub const MAX_RECUR_ORDER: usize = 4;

/// The largest byte column on which the GF(2) LFSR detector runs. Berlekamp–Massey is `O(bits²)`, so
/// this bounds the deep-attack cost; it runs only as a LAST RESORT on columns nothing else compressed.
const LFSR_MAX_BYTES: usize = 512;

/// The largest byte column on which the 2-adic FCSR detector runs (bignum rational reconstruction).
const FCSR_MAX_BYTES: usize = 256;

/// A decode cap so a corrupt run-length can't ask for terabytes of output.
const RLE_MAX_TOTAL: usize = 1 << 28;

/// A hostile/garbage [`GenExpr`] tree is rejected past this many nodes.
pub const MAX_GEN_NODES: u32 = 256;

/// …or this deep — bounds both decode recursion and eval.
pub const MAX_GEN_DEPTH: u32 = 32;

/// The nesting cap for a periodic column whose block is itself a column (bounds decode recursion).
const DECODE_MAX_DEPTH: u32 = 32;

/// A generous element cap for the standalone [`decode_int_seq`] (which has no receiver policy): a
/// tampered generator descriptor still cannot materialize more than this many elements.
const DEFAULT_MAX_ELEMENTS: usize = 1 << 28;

// The `T_INTS_*` tags for the Auto column menu. These bytes are the wire format; the wider tag space
// (structs, strings, floats, …) lives in the wire codec that also speaks these.
pub const T_INTS: u8 = 19; // adaptive-sign zig-zag varint per element (the plain baseline)
pub const T_INTS_AFFINE: u8 = 32; // closed-form: base + stride·i (3 numbers, no data)
pub const T_INTS_DELTA: u8 = 39; // first + zig-zag successive differences — monotone columns
pub const T_INTS_DOD: u8 = 40; // first + first delta + zig-zag second differences — near-linear
pub const T_INTS_FOR: u8 = 41; // min + bit-width + bit-packed (v-min) residuals — clustered
pub const T_INTS_RLE: u8 = 42; // (value, run-length) pairs — runs of repeats
pub const T_INTS_DICT: u8 = 43; // distinct values + bit-packed indices — low cardinality
pub const T_INTS_POLY: u8 = 50; // degree + (degree+1) finite-difference seeds — a polynomial column
pub const T_GEN: u8 = 51; // a serialized GenExpr over the index `i` (the general generator form)
pub const T_BYTES: u8 = 53; // raw 1-byte-per-element blob — a byte column
pub const T_INTS_GEOMETRIC: u8 = 61; // closed-form: base · ratioⁱ (3 numbers, no data)
pub const T_INTS_PERIODIC: u8 = 62; // cyclic: period p + count + one block → pattern[i % p]
pub const T_INTS_SPARSE: u8 = 67; // dominant value + (delta-index, value) exceptions — sparse
pub const T_INTS_LRECUR: u8 = 82; // constant-coefficient linear recurrence: order + coeffs + seeds
pub const T_INTS_LFSR: u8 = 83; // GF(2) LFSR: byte count + linear complexity L + L feedback taps + L seed bits
pub const T_INTS_FCSR: u8 = 84; // 2-adic FCSR: byte count + rational p/q (p then q, each sign+len+LE bytes)

// ---- Varint / zig-zag primitives -----------------------------------------------------------

/// LEB128 unsigned varint: 7 bits per byte, high bit = continuation.
#[inline]
pub fn write_uvarint(mut x: u64, out: &mut Vec<u8>) {
    while x >= 0x80 {
        out.push((x as u8) | 0x80);
        x >>= 7;
    }
    out.push(x as u8);
}

/// Read a LEB128 unsigned varint. `None` on an overlong/overflowing encoding or a short buffer.
#[inline]
pub fn read_uvarint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let b = *buf.get(*pos)?;
        *pos += 1;
        if shift >= 64 {
            return None; // overlong / overflow
        }
        result |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
}

/// Map a signed integer to an unsigned one with small magnitudes near zero (for varint).
#[inline]
pub fn zigzag(x: i64) -> u64 {
    ((x << 1) ^ (x >> 63)) as u64
}

/// The inverse of [`zigzag`].
#[inline]
pub fn unzigzag(x: u64) -> i64 {
    ((x >> 1) as i64) ^ -((x & 1) as i64)
}

/// The number of bytes [`write_uvarint`] would emit for `x`.
pub fn uvarint_byte_len(x: u64) -> usize {
    (((64 - x.leading_zeros()).max(1) + 6) / 7) as usize
}

/// The plain baseline column (`T_INTS`): an adaptive-sign header (`(n << 1) | signed`) then a
/// varint per element — zig-zag when any value is negative, plain LEB128 otherwise.
pub fn leb128_encode<I: Iterator<Item = i64> + Clone>(out: &mut Vec<u8>, vals: I, n: usize) {
    let signed = vals.clone().any(|x| x < 0);
    write_uvarint(((n as u64) << 1) | signed as u64, out);
    out.reserve(n * 2);
    if signed {
        for x in vals {
            write_uvarint(zigzag(x), out);
        }
    } else {
        for x in vals {
            write_uvarint(x as u64, out);
        }
    }
}

// ---- Bit packing ---------------------------------------------------------------------------

/// Pack `vals` LSB-first at `width` bits each (1..=64). The inverse of [`bitunpack`].
pub fn bitpack(vals: &[u64], width: u8) -> Vec<u8> {
    if width == 0 {
        return Vec::new();
    }
    let total_bits = vals.len().saturating_mul(width as usize);
    let mut out = vec![0u8; total_bits.div_ceil(8)];
    let mut bitpos = 0usize;
    for &val in vals {
        let mut bits = val;
        let mut remaining = width as usize;
        while remaining > 0 {
            let byte = bitpos / 8;
            let off = bitpos % 8;
            let take = remaining.min(8 - off);
            let mask = (1u64 << take) - 1;
            out[byte] |= ((bits & mask) as u8) << off;
            bits >>= take;
            bitpos += take;
            remaining -= take;
        }
    }
    out
}

/// Read `count` LSB-first `width`-bit values from `bytes`. `None` if `bytes` is too short. The
/// inverse of [`bitpack`].
pub fn bitunpack(bytes: &[u8], count: usize, width: u8) -> Option<Vec<u64>> {
    if width == 0 || width > 64 {
        return None;
    }
    let total_bits = count.checked_mul(width as usize)?;
    if bytes.len() < total_bits.div_ceil(8) {
        return None;
    }
    let mut out = Vec::with_capacity(count.min(PREALLOC_CAP));
    let mut bitpos = 0usize;
    for _ in 0..count {
        let mut val = 0u64;
        let mut got = 0usize;
        while got < width as usize {
            let byte = bitpos / 8;
            let off = bitpos % 8;
            let take = (width as usize - got).min(8 - off);
            let mask = (1u64 << take) - 1;
            val |= (((bytes[byte] >> off) as u64) & mask) << got;
            got += take;
            bitpos += take;
        }
        out.push(val);
    }
    Some(out)
}

// ---- The sandboxed generator IR (ship the computation, not the data) -----------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenCmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A restricted, pure, TOTAL expression over the element index `i`. Every op is total (div/mod by
/// zero is 0, wrapping i64 arithmetic), and a malformed/hostile tree is bounded at decode by a node
/// budget + depth cap, so evaluation can never panic, diverge, overflow, or escape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenExpr {
    Index,
    Const(i64),
    Add(Box<GenExpr>, Box<GenExpr>),
    Sub(Box<GenExpr>, Box<GenExpr>),
    Mul(Box<GenExpr>, Box<GenExpr>),
    Div(Box<GenExpr>, Box<GenExpr>),
    Mod(Box<GenExpr>, Box<GenExpr>),
    Select { op: GenCmp, lhs: Box<GenExpr>, rhs: Box<GenExpr>, then: Box<GenExpr>, els: Box<GenExpr> },
}

/// Evaluate a generator at index `i`. TOTAL: div/mod by zero is 0; all arithmetic wraps.
pub fn gen_eval(e: &GenExpr, i: i64) -> i64 {
    match e {
        GenExpr::Index => i,
        GenExpr::Const(c) => *c,
        GenExpr::Add(a, b) => gen_eval(a, i).wrapping_add(gen_eval(b, i)),
        GenExpr::Sub(a, b) => gen_eval(a, i).wrapping_sub(gen_eval(b, i)),
        GenExpr::Mul(a, b) => gen_eval(a, i).wrapping_mul(gen_eval(b, i)),
        GenExpr::Div(a, b) => {
            let d = gen_eval(b, i);
            if d == 0 { 0 } else { gen_eval(a, i).wrapping_div(d) }
        }
        GenExpr::Mod(a, b) => {
            let d = gen_eval(b, i);
            if d == 0 { 0 } else { gen_eval(a, i).wrapping_rem(d) }
        }
        GenExpr::Select { op, lhs, rhs, then, els } => {
            let (l, r) = (gen_eval(lhs, i), gen_eval(rhs, i));
            let c = match op {
                GenCmp::Eq => l == r,
                GenCmp::Ne => l != r,
                GenCmp::Lt => l < r,
                GenCmp::Le => l <= r,
                GenCmp::Gt => l > r,
                GenCmp::Ge => l >= r,
            };
            if c { gen_eval(then, i) } else { gen_eval(els, i) }
        }
    }
}

/// Serialize a generator pre-order: a 1-byte node tag, then children. Self-delimiting.
pub fn serialize_gen(e: &GenExpr, out: &mut Vec<u8>) {
    match e {
        GenExpr::Index => out.push(0),
        GenExpr::Const(c) => {
            out.push(1);
            write_uvarint(zigzag(*c), out);
        }
        GenExpr::Add(a, b) => { out.push(2); serialize_gen(a, out); serialize_gen(b, out); }
        GenExpr::Sub(a, b) => { out.push(3); serialize_gen(a, out); serialize_gen(b, out); }
        GenExpr::Mul(a, b) => { out.push(4); serialize_gen(a, out); serialize_gen(b, out); }
        GenExpr::Div(a, b) => { out.push(5); serialize_gen(a, out); serialize_gen(b, out); }
        GenExpr::Mod(a, b) => { out.push(6); serialize_gen(a, out); serialize_gen(b, out); }
        GenExpr::Select { op, lhs, rhs, then, els } => {
            out.push(7);
            out.push(*op as u8);
            serialize_gen(lhs, out);
            serialize_gen(rhs, out);
            serialize_gen(then, out);
            serialize_gen(els, out);
        }
    }
}

/// Parse a generator under a node `budget` and `depth` cap — a garbage/hostile tree returns `None`.
pub fn deserialize_gen(buf: &[u8], pos: &mut usize, budget: &mut u32, depth: u32) -> Option<GenExpr> {
    if depth > MAX_GEN_DEPTH || *budget == 0 {
        return None;
    }
    *budget -= 1;
    let tag = *buf.get(*pos)?;
    *pos += 1;
    Some(match tag {
        0 => GenExpr::Index,
        1 => GenExpr::Const(unzigzag(read_uvarint(buf, pos)?)),
        2..=6 => {
            let a = Box::new(deserialize_gen(buf, pos, budget, depth + 1)?);
            let b = Box::new(deserialize_gen(buf, pos, budget, depth + 1)?);
            match tag {
                2 => GenExpr::Add(a, b),
                3 => GenExpr::Sub(a, b),
                4 => GenExpr::Mul(a, b),
                5 => GenExpr::Div(a, b),
                _ => GenExpr::Mod(a, b),
            }
        }
        7 => {
            let op = match *buf.get(*pos)? {
                0 => GenCmp::Eq,
                1 => GenCmp::Ne,
                2 => GenCmp::Lt,
                3 => GenCmp::Le,
                4 => GenCmp::Gt,
                5 => GenCmp::Ge,
                _ => return None,
            };
            *pos += 1;
            let lhs = Box::new(deserialize_gen(buf, pos, budget, depth + 1)?);
            let rhs = Box::new(deserialize_gen(buf, pos, budget, depth + 1)?);
            let then = Box::new(deserialize_gen(buf, pos, budget, depth + 1)?);
            let els = Box::new(deserialize_gen(buf, pos, budget, depth + 1)?);
            GenExpr::Select { op, lhs, rhs, then, els }
        }
        _ => return None,
    })
}

// ---- Column-shape detectors ----------------------------------------------------------------

/// If `v` is an exact affine progression `base + stride·i`, return `(base, stride)`.
pub fn detect_affine(v: &[i64]) -> Option<(i64, i64)> {
    if v.len() < 2 {
        return None;
    }
    let base = v[0];
    let stride = v[1].wrapping_sub(v[0]);
    for (i, &x) in v.iter().enumerate() {
        if base.wrapping_add((i as i64).wrapping_mul(stride)) != x {
            return None;
        }
    }
    Some((base, stride))
}

/// If `v` is an exact geometric progression `base · ratioⁱ` (≥3 elements, confirmed by replaying the
/// decoder's wrapping arithmetic), return `(base, ratio)`.
pub fn detect_geometric(v: &[i64]) -> Option<(i64, i64)> {
    if v.len() < 3 {
        return None;
    }
    let base = v[0];
    if base == 0 || v[1].checked_rem(base)? != 0 {
        return None;
    }
    let ratio = v[1].checked_div(base)?;
    if ratio == 0 || ratio == 1 {
        return None;
    }
    let mut cur = base;
    for &x in v {
        if cur != x {
            return None;
        }
        cur = cur.wrapping_mul(ratio);
    }
    Some((base, ratio))
}

/// If `v` is an exact cyclic repetition of a minimal block of period `2 ≤ p ≤ min(len/2, cap)`,
/// return `p`.
pub fn detect_period(v: &[i64]) -> Option<usize> {
    let n = v.len();
    if n < 4 {
        return None;
    }
    let cap = (n / 2).min(PERIOD_CAP);
    'p: for p in 2..=cap {
        for i in p..n {
            if v[i] != v[i - p] {
                continue 'p;
            }
        }
        return Some(p);
    }
    None
}

/// If one value dominates the column (covers ≥ ¾), return it and the sorted `(index, value)`
/// exceptions.
pub fn detect_sparse(v: &[i64]) -> Option<(i64, Vec<(usize, i64)>)> {
    if v.len() < 8 {
        return None;
    }
    let mut cand = v[0];
    let mut count: i64 = 0;
    for &x in v {
        if count == 0 {
            cand = x;
            count = 1;
        } else if x == cand {
            count += 1;
        } else {
            count -= 1;
        }
    }
    let occ = v.iter().filter(|&&x| x == cand).count();
    if v.len() - occ > v.len() / 4 {
        return None; // not dominant enough — the exception list wouldn't beat the menu
    }
    let exceptions: Vec<(usize, i64)> =
        v.iter().enumerate().filter(|(_, &x)| x != cand).map(|(i, &x)| (i, x)).collect();
    Some((cand, exceptions))
}

/// Recognize `v` as a polynomial column (degree ≤ [`MAX_POLY_DEGREE`]) via finite differences and
/// return `(degree, seeds)` — the k+1 leading-edge seeds, confirmed by exact reconstruction.
pub fn detect_poly_generator(v: &[i64]) -> Option<(u8, Vec<i64>)> {
    if v.len() < 3 {
        return None;
    }
    let mut levels: Vec<Vec<i64>> = Vec::with_capacity(MAX_POLY_DEGREE + 1);
    levels.push(v.to_vec());
    for d in 0..MAX_POLY_DEGREE {
        let prev = &levels[d];
        if prev.len() < 2 {
            break;
        }
        let mut next = Vec::with_capacity(prev.len() - 1);
        for w in prev.windows(2) {
            next.push(w[1].checked_sub(w[0])?); // a difference overflow → fall back to the menu
        }
        if next.len() >= 2 && next.iter().all(|&x| x == next[0]) {
            let degree = (d + 1) as u8;
            let mut seeds: Vec<i64> = levels.iter().map(|lvl| lvl[0]).collect();
            seeds.push(next[0]);
            if reconstruct_poly(&seeds, v.len()) == v {
                return Some((degree, seeds));
            }
            return None;
        }
        levels.push(next);
    }
    None
}

/// Replay a polynomial column from its finite-difference seeds via a difference engine.
pub fn reconstruct_poly(seeds: &[i64], n: usize) -> Vec<i64> {
    let mut diffs = seeds.to_vec();
    let mut out = Vec::with_capacity(n.min(PREALLOC_CAP));
    for _ in 0..n {
        out.push(diffs[0]);
        for j in 0..diffs.len().saturating_sub(1) {
            diffs[j] = diffs[j].wrapping_add(diffs[j + 1]);
        }
    }
    out
}

/// Recognize `v` as a constant-coefficient linear recurrence `v[i] = Σⱼ cⱼ·v[i-1-j]` of minimal order
/// `k ≤ [`MAX_RECUR_ORDER`]` with **integer** coefficients, and return `(coeffs, seeds)` — `k`
/// coefficients + the first `k` values. This is exactly Berlekamp–Massey over ℤ: it captures Fibonacci,
/// Lucas, Pell, and any linear-feedback sequence — none of which the polynomial detector can reach
/// (their finite differences never settle). The coefficients are solved exactly (Cramer over `i128`)
/// from the smallest `2k` terms and then CONFIRMED by replaying the recurrence with the SAME wrapping
/// arithmetic the decoder uses, so a match certifies a bit-exact reconstruction.
pub fn detect_linear_recurrence(v: &[i64]) -> Option<(Vec<i64>, Vec<i64>)> {
    let n = v.len();
    for k in 1..=MAX_RECUR_ORDER.min(n / 2) {
        if let Some(coeffs) = solve_recurrence(v, k) {
            if reconstruct_recurrence(&coeffs, &v[..k], n) == v {
                return Some((coeffs, v[..k].to_vec()));
            }
        }
    }
    None
}

/// Solve the `k×k` linear system for integer recurrence coefficients from the smallest `2k` terms via
/// Cramer's rule over `i128`. `None` if the system is singular, a coefficient is non-integer or
/// out-of-`i64`, or a determinant overflows.
fn solve_recurrence(v: &[i64], k: usize) -> Option<Vec<i64>> {
    let mut m = vec![vec![0i128; k]; k];
    let mut b = vec![0i128; k];
    for r in 0..k {
        for j in 0..k {
            m[r][j] = v[k + r - 1 - j] as i128; // coefficient of c_{j+1} in the equation for v[k+r]
        }
        b[r] = v[k + r] as i128;
    }
    let det_m = det_i128(&m)?;
    if det_m == 0 {
        return None;
    }
    let mut coeffs = Vec::with_capacity(k);
    for j in 0..k {
        let mut mj = m.clone();
        for (r, br) in b.iter().enumerate() {
            mj[r][j] = *br;
        }
        let det_j = det_i128(&mj)?;
        if det_j % det_m != 0 {
            return None; // non-integer coefficient — not an exact integer recurrence
        }
        let c = det_j / det_m;
        if c < i64::MIN as i128 || c > i64::MAX as i128 {
            return None;
        }
        coeffs.push(c as i64);
    }
    Some(coeffs)
}

/// A checked `i128` determinant by Laplace expansion (used only for `k ≤ [`MAX_RECUR_ORDER`]`, so at
/// most `4! = 24` terms). `None` on overflow.
fn det_i128(m: &[Vec<i128>]) -> Option<i128> {
    let k = m.len();
    match k {
        1 => Some(m[0][0]),
        2 => m[0][0].checked_mul(m[1][1])?.checked_sub(m[0][1].checked_mul(m[1][0])?),
        _ => {
            let mut acc: i128 = 0;
            for j in 0..k {
                let minor: Vec<Vec<i128>> =
                    (1..k).map(|r| (0..k).filter(|&c| c != j).map(|c| m[r][c]).collect()).collect();
                let cof = m[0][j].checked_mul(det_i128(&minor)?)?;
                acc = if j % 2 == 0 { acc.checked_add(cof)? } else { acc.checked_sub(cof)? };
            }
            Some(acc)
        }
    }
}

/// Replay a linear recurrence from `coeffs` and the first `seeds.len()` values, wrapping (matches the
/// decoder, so a confirmed fit is exact across all of `i64`). `n` values out.
fn reconstruct_recurrence(coeffs: &[i64], seeds: &[i64], n: usize) -> Vec<i64> {
    let k = coeffs.len();
    // Guard: without `k` seeds the recurrence cannot start — return what seeds we have (a malformed
    // decode then simply fails the equality/round-trip check rather than indexing out of bounds).
    if seeds.len() < k {
        return seeds[..seeds.len().min(n)].to_vec();
    }
    let mut out = Vec::with_capacity(n.min(PREALLOC_CAP));
    out.extend_from_slice(&seeds[..k.min(n)]);
    for i in k..n {
        let mut acc = 0i64;
        for j in 0..k {
            acc = acc.wrapping_add(coeffs[j].wrapping_mul(out[i - 1 - j]));
        }
        out.push(acc);
    }
    out.truncate(n);
    out
}

// ---- Berlekamp–Massey over an arbitrary field: the shortest LFSR (the LFSR attack, as a compressor) --
//
// The Berlekamp–Massey algorithm finds the shortest linear-feedback shift register generating a
// sequence over ANY field. Over GF(2) it is the classic bit-LFSR attack; over GF(2⁸) it is the
// *word*-oriented LFSR attack (a byte stream whose bytes are a GF(256)-linear recurrence). A word-LFSR
// of order `L` is also a bit-LFSR of order `≤ 8L`, so GF(2) already *catches* it — but GF(2⁸) reports
// the natural word complexity and runs in `O(n²)` byte-ops instead of `O((8n)²)` bit-ops.

/// The minimal field interface Berlekamp–Massey needs.
pub trait FieldElem: Copy + PartialEq {
    fn zero() -> Self;
    fn one() -> Self;
    fn add(self, o: Self) -> Self;
    fn sub(self, o: Self) -> Self;
    fn mul(self, o: Self) -> Self;
    /// Multiplicative inverse; only called on nonzero elements.
    fn inv(self) -> Self;
}

/// GF(2) — a single bit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Gf2(pub bool);
impl FieldElem for Gf2 {
    fn zero() -> Self { Gf2(false) }
    fn one() -> Self { Gf2(true) }
    fn add(self, o: Self) -> Self { Gf2(self.0 ^ o.0) }
    fn sub(self, o: Self) -> Self { Gf2(self.0 ^ o.0) }
    fn mul(self, o: Self) -> Self { Gf2(self.0 && o.0) }
    fn inv(self) -> Self { self } // 1⁻¹ = 1 (the only nonzero element)
}

/// GF(2⁸) — the Rijndael/AES field (reduction polynomial `x⁸+x⁴+x³+x²+1`), one byte per element.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Gf256(pub u8);

fn gf256_xtime(a: u8) -> u8 {
    (a << 1) ^ (if a & 0x80 != 0 { 0x1B } else { 0 })
}
/// `(log, exp)` tables for GF(2⁸) with generator 3: `exp[i] = 3ⁱ`, `log` its inverse. `exp` is doubled
/// to length 512 so a product's log index never needs a modular reduction.
fn gf256_tables() -> &'static (Vec<u8>, Vec<u8>) {
    static T: std::sync::OnceLock<(Vec<u8>, Vec<u8>)> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut exp = vec![0u8; 512];
        let mut log = vec![0u8; 256];
        let mut x = 1u8;
        for i in 0..255usize {
            exp[i] = x;
            log[x as usize] = i as u8;
            x ^= gf256_xtime(x); // x ← x·3 = x·2 ⊕ x
        }
        for i in 255..512 {
            exp[i] = exp[i - 255];
        }
        (log, exp)
    })
}
impl FieldElem for Gf256 {
    fn zero() -> Self { Gf256(0) }
    fn one() -> Self { Gf256(1) }
    fn add(self, o: Self) -> Self { Gf256(self.0 ^ o.0) }
    fn sub(self, o: Self) -> Self { Gf256(self.0 ^ o.0) }
    fn mul(self, o: Self) -> Self {
        if self.0 == 0 || o.0 == 0 {
            return Gf256(0);
        }
        let (log, exp) = gf256_tables();
        Gf256(exp[log[self.0 as usize] as usize + log[o.0 as usize] as usize])
    }
    fn inv(self) -> Self {
        let (log, exp) = gf256_tables();
        Gf256(exp[255 - log[self.0 as usize] as usize])
    }
}

/// **Berlekamp–Massey over a field.** Returns `(L, taps)` where `L` is the linear complexity and `taps`
/// are the connection-polynomial coefficients `c₁..c_L`; the recurrence is `s[i] = -Σⱼ c_{j+1}·s[i-1-j]`.
pub fn berlekamp_massey_field<F: FieldElem>(s: &[F]) -> (usize, Vec<F>) {
    let n = s.len();
    let mut c = vec![F::zero(); n + 1];
    let mut b = vec![F::zero(); n + 1];
    c[0] = F::one();
    b[0] = F::one();
    let mut l = 0usize;
    let mut m = 1usize;
    let mut b_disc = F::one(); // the discrepancy at the last length change
    for nn in 0..n {
        let mut d = s[nn];
        for i in 1..=l {
            d = d.add(c[i].mul(s[nn - i]));
        }
        if d == F::zero() {
            m += 1;
        } else {
            let coef = d.mul(b_disc.inv());
            let t = c.clone();
            for i in 0..=n {
                if i + m <= n && b[i] != F::zero() {
                    c[i + m] = c[i + m].sub(coef.mul(b[i]));
                }
            }
            if 2 * l <= nn {
                l = nn + 1 - l;
                b = t;
                b_disc = d;
                m = 1;
            } else {
                m += 1;
            }
        }
    }
    let taps = if l == 0 { Vec::new() } else { c[1..=l].to_vec() };
    (l, taps)
}

/// Replay a field LFSR: emit `seed`, then `s[i] = -Σⱼ tapsⱼ·s[i-1-j]` (the connection recurrence).
pub fn lfsr_generate_field<F: FieldElem>(taps: &[F], seed: &[F], total: usize) -> Vec<F> {
    let l = taps.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&seed[..l.min(seed.len()).min(total)]);
    for i in l..total {
        let mut acc = F::zero();
        for j in 0..l {
            acc = acc.add(taps[j].mul(out[i - 1 - j]));
        }
        out.push(F::zero().sub(acc));
    }
    out.truncate(total);
    out
}

/// **Berlekamp–Massey over GF(2)** — the shortest bit-LFSR. Thin wrapper over [`berlekamp_massey_field`]
/// (see it for the full semantics). For an LFSR keystream `L` is the register length; for random ≈ n/2.
pub fn berlekamp_massey_gf2(s: &[bool]) -> (usize, Vec<bool>) {
    let elems: Vec<Gf2> = s.iter().map(|&b| Gf2(b)).collect();
    let (l, taps) = berlekamp_massey_field(&elems);
    (l, taps.into_iter().map(|g| g.0).collect())
}

/// Replay a GF(2) LFSR: `s[i] = ⊕ⱼ tapsⱼ · s[i-1-j]`. The inverse of [`berlekamp_massey_gf2`].
pub fn lfsr_generate(taps: &[bool], seed: &[bool], total: usize) -> Vec<bool> {
    let t: Vec<Gf2> = taps.iter().map(|&b| Gf2(b)).collect();
    let sd: Vec<Gf2> = seed.iter().map(|&b| Gf2(b)).collect();
    lfsr_generate_field(&t, &sd, total).into_iter().map(|g| g.0).collect()
}

/// If a byte column is a GF(2) LFSR keystream — its `8·n`-bit expansion has linear complexity `L` and
/// the length-`L` LFSR regenerates every bit — return `(L, taps, seed_bits)`. Only worthwhile when `L`
/// is well below the bit count (else the raw byte column wins and `consider` keeps it).
fn detect_lfsr_bytes(v: &[i64]) -> Option<(usize, Vec<bool>, Vec<bool>)> {
    let bits = bytes_to_bits(v);
    let (l, taps) = berlekamp_massey_gf2(&bits);
    if l == 0 || l >= bits.len() {
        return None;
    }
    let seed = bits[..l].to_vec();
    if lfsr_generate(&taps, &seed, bits.len()) != bits {
        return None; // the LFSR does not regenerate the whole column — not a clean keystream
    }
    Some((l, taps, seed))
}

/// Expand a byte column to its LSB-first bit sequence (bit `j` of byte `i` is `(byte_i >> j) & 1`).
pub fn bytes_to_bits(v: &[i64]) -> Vec<bool> {
    let mut bits = Vec::with_capacity(v.len().saturating_mul(8));
    for &x in v {
        let byte = x as u8;
        for j in 0..8 {
            bits.push((byte >> j) & 1 == 1);
        }
    }
    bits
}

/// Pack an LSB-first bit sequence back into bytes (the inverse of [`bytes_to_bits`]); a trailing
/// partial group is zero-padded.
pub fn bits_to_bytes(bits: &[bool]) -> Vec<i64> {
    bits.chunks(8)
        .map(|chunk| {
            let mut byte = 0u8;
            for (j, &bit) in chunk.iter().enumerate() {
                if bit {
                    byte |= 1 << j;
                }
            }
            byte as i64
        })
        .collect()
}

// ---- 2-adic complexity / FCSR: the carry-based attack (what every LINEAR tool misses) -----------
//
// A feedback-with-carry shift register (FCSR) generates a bit sequence whose 2-adic value `Σ sᵢ2ⁱ`
// equals a rational `p/q` with `q` ODD; the register is defined by the connection integer `q`. Because
// the carry makes the map NON-linear over GF(2), an FCSR keystream (mod-2ⁿ LCGs, summation combiners,
// the Marsaglia add-with-carry family) is invisible to Berlekamp–Massey over any field. The 2-adic
// analogue — the **Rational Approximation Algorithm** (Klapper–Goresky), here via rational
// reconstruction over the integers — recovers `p/q` and its *2-adic complexity* `≈ log₂ max(|p|,|q|)`.

/// gcd of `|a|`, `|b|` by the Euclidean algorithm over `BigInt`.
fn bigint_gcd_local(a: &crate::numeric::BigInt, b: &crate::numeric::BigInt) -> crate::numeric::BigInt {
    let (mut a, mut b) = (a.abs(), b.abs());
    while !b.is_zero() {
        let r = a.div_rem(&b).map(|(_, r)| r).unwrap_or_else(crate::numeric::BigInt::zero);
        a = b;
        b = r;
    }
    a
}

/// The bit length of `|x|` (`0` for zero).
fn bigint_bitlen(x: &crate::numeric::BigInt) -> usize {
    let (_, bytes) = x.to_le_bytes();
    for (i, &b) in bytes.iter().enumerate().rev() {
        if b != 0 {
            return i * 8 + (8 - b.leading_zeros() as usize);
        }
    }
    0
}

/// **Rational reconstruction of a bit sequence as a 2-adic number.** Returns `(p, q)` with `q` odd and
/// positive and `p/q ≡ Σ sᵢ2ⁱ (mod 2ⁿ)` — the shortest FCSR generating the sequence. `None` when no
/// odd-denominator rational fits (an algorithmically-random sequence has no small FCSR).
pub fn two_adic_reconstruct(bits: &[bool]) -> Option<(crate::numeric::BigInt, crate::numeric::BigInt)> {
    use crate::numeric::BigInt;
    let n = bits.len();
    if n == 0 {
        return None;
    }
    let two = BigInt::from_i64(2);
    // α = Σ bits[i]·2ⁱ, and N = 2ⁿ.
    let mut alpha = BigInt::zero();
    let mut pow2 = BigInt::from_i64(1);
    for &bit in bits {
        if bit {
            alpha = alpha.add(&pow2);
        }
        pow2 = pow2.mul(&two);
    }
    let n_big = pow2; // 2ⁿ
    // Extended Euclid keeping rᵢ ≡ tᵢ·α (mod N); stop at the first rᵢ with rᵢ² ≤ N.
    let (mut r0, mut t0) = (n_big.clone(), BigInt::zero());
    let (mut r1, mut t1) = (alpha, BigInt::from_i64(1));
    while !r1.is_zero() && r1.mul(&r1) > n_big {
        let (quot, rem) = r0.div_rem(&r1)?;
        let t2 = t0.sub(&quot.mul(&t1));
        r0 = r1;
        t0 = t1;
        r1 = rem;
        t1 = t2;
    }
    let (mut p, mut q) = (r1, t1);
    if q.is_zero() {
        return None;
    }
    if q.is_negative() {
        p = BigInt::zero().sub(&p);
        q = BigInt::zero().sub(&q);
    }
    // Reduce to lowest terms — the rational reconstruction can return a non-coprime `p/q` (a common
    // factor `g` gives the same 2-adic value `pg/qg`); reducing yields the minimal FCSR and the true
    // 2-adic complexity.
    let g = bigint_gcd_local(&p, &q);
    if !g.is_zero() && g != BigInt::from_i64(1) {
        p = p.div_rem(&g).map(|(quot, _)| quot)?;
        q = q.div_rem(&g).map(|(quot, _)| quot)?;
    }
    if !q.is_odd() {
        return None; // no odd-denominator (FCSR) rational — treat as incompressible
    }
    Some((p, q))
}

/// Generate `n` bits of the FCSR keystream for `p/q` (`q` odd) — the 2-adic expansion, by repeated
/// 2-adic division. The inverse of [`two_adic_reconstruct`].
pub fn fcsr_generate(p: &crate::numeric::BigInt, q: &crate::numeric::BigInt, n: usize) -> Vec<bool> {
    use crate::numeric::BigInt;
    let two = BigInt::from_i64(2);
    let mut r = p.clone();
    let mut bits = Vec::with_capacity(n);
    for _ in 0..n {
        let bit = r.is_odd();
        bits.push(bit);
        if bit {
            r = r.sub(q);
        }
        // r ← r/2 (exact: r − bit·q is even because q is odd).
        r = r.div_rem(&two).map(|(quot, _)| quot).unwrap_or_else(BigInt::zero);
    }
    bits
}

/// The **2-adic complexity** of a bit sequence: `≈ log₂ max(|p|,|q|)` for its FCSR rational `p/q`. Low ⇒
/// a carry-based keystream (an FCSR / add-with-carry generator); `≈ n/2` for a random sequence.
pub fn two_adic_complexity(bits: &[bool]) -> usize {
    match two_adic_reconstruct(bits) {
        Some((p, q)) => bigint_bitlen(&p).max(bigint_bitlen(&q)),
        None => bits.len(),
    }
}

/// The **maximal order complexity** (Jansen) of a bit sequence: the length of the shortest feedback
/// shift register — **linear OR nonlinear** — that generates it, i.e. the smallest `L` such that every
/// length-`L` window has a unique successor (`s[i] = f(s[i-1],…,s[i-L])` for some feedback function
/// `f`). It is the TOP of the FSR complexity hierarchy — `MOC ≤ 2-adic complexity` and `MOC ≤ linear
/// complexity` — so it catches nonlinear generators (NFSRs, algebraic combiners with memory) that fool
/// both Berlekamp–Massey and the 2-adic Rational Approximation. Low relative to `n` ⇒ a short-register
/// generator; `≈ n/2` for a random sequence. Consistency is monotone in `L`, so we binary-search it.
pub fn maximal_order_complexity(bits: &[bool]) -> usize {
    let n = bits.len();
    if n == 0 {
        return 0;
    }
    // Is every length-`l` window followed by a unique bit?
    let consistent = |l: usize| -> bool {
        if l >= n {
            return true;
        }
        let mut succ: std::collections::HashMap<&[bool], bool> = std::collections::HashMap::new();
        for i in 0..(n - l) {
            match succ.insert(&bits[i..i + l], bits[i + l]) {
                Some(prev) if prev != bits[i + l] => return false,
                _ => {}
            }
        }
        true
    };
    // The smallest `L` in `[0, n]` with `consistent(L)`.
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if consistent(mid) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

// ---- Algebraic recurrence: the degree-`d` generalization of Berlekamp–Massey (the OPEN rung) ------
//
// Maximal order complexity certifies that a sequence is generated by a length-`L` feedback register,
// but the register's feedback function `f` is a full `2^L` truth table — as large as the data — so a
// low MOC is a structural *weakness* without a compressor. The algebraic recovery closes that gap when
// `f` has *low degree*: model `s[i] = f(s[i-1],…,s[i-L])` as a GF(2) polynomial and recover its sparse
// **algebraic normal form** by linearization. Each monomial of degree `≤ d` over the `L` window bits is
// treated as an independent unknown; each output position gives one linear equation in those unknowns;
// a GF(2) Gaussian solve returns the ANF coefficients. For a degree-`d` feedback this is `M = Σₖ₌₀ᵈ
// C(L,k) = O(Lᵈ)` coefficients — vastly smaller than the `2^L` truth table — so a low-degree nonlinear
// generator (a quadratic NFSR, an algebraic filter with memory) is *compressed*, not merely diagnosed.
// This is the algebraic attack (Courtois–Meier linearization) recast as an MDL codec. It stays cheap
// only while `d` is small; a high-degree `f` sends `M → 2^L` and the sequence back to the ceiling.

/// The monomials of degree `≤ d` over `l` window variables, each as a sorted variable-index list
/// (`[]` = the constant `1`). The ANF basis for the algebraic recurrence solve.
fn monomials(l: usize, d: usize) -> Vec<Vec<usize>> {
    fn combos(start: usize, l: usize, k: usize, cur: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if cur.len() == k {
            out.push(cur.clone());
            return;
        }
        for v in start..l {
            cur.push(v);
            combos(v + 1, l, k, cur, out);
            cur.pop();
        }
    }
    let mut out = vec![Vec::new()];
    for k in 1..=d.min(l) {
        combos(0, l, k, &mut Vec::new(), &mut out);
    }
    out
}

/// Evaluate a monomial (an AND of window bits; the empty product is the constant `1`) on a window.
fn eval_monomial(mono: &[usize], window: &[bool]) -> bool {
    mono.iter().all(|&v| window[v])
}

/// Solve a GF(2) linear system `A·c = b` for a particular solution `c` (free variables set to `0`), with
/// rows given as `(coefficient bitmask over `ncols` columns, rhs bit)`. Full Gaussian elimination to
/// reduced row-echelon form over any number of columns (bitset rows, so `> 64` unknowns are fine).
/// Returns `None` if the system is inconsistent.
fn solve_gf2_system(rows: &[(Vec<u64>, bool)], ncols: usize) -> Option<Vec<bool>> {
    let words = ncols.div_ceil(64).max(1);
    let mut mat: Vec<(Vec<u64>, bool)> = rows.to_vec();
    let mut pivot_row_of_col = vec![usize::MAX; ncols];
    let mut r = 0usize;
    for c in 0..ncols {
        let (w, bit) = (c / 64, 1u64 << (c % 64));
        let Some(pr) = (r..mat.len()).find(|&i| mat[i].0[w] & bit != 0) else {
            continue;
        };
        mat.swap(r, pr);
        for i in 0..mat.len() {
            if i != r && mat[i].0[w] & bit != 0 {
                for k in 0..words {
                    mat[i].0[k] ^= mat[r].0[k];
                }
                mat[i].1 ^= mat[r].1;
            }
        }
        pivot_row_of_col[c] = r;
        r += 1;
    }
    // A row that reduced to `0 = 1` proves the system inconsistent.
    if mat.iter().any(|(coeff, rhs)| *rhs && coeff.iter().all(|&w| w == 0)) {
        return None;
    }
    // Free variables are `0`; each pivot variable equals its row's rhs (that row now has only its own
    // pivot among the pivot columns, so the free-variables-zero assignment reads straight off the rhs).
    let mut c = vec![false; ncols];
    for col in 0..ncols {
        if pivot_row_of_col[col] != usize::MAX {
            c[col] = mat[pivot_row_of_col[col]].1;
        }
    }
    Some(c)
}

/// Recover the **algebraic normal form** of a degree-`≤ d`, order-`l` nonlinear feedback: the ANF
/// coefficient vector over [`monomials`]`(l, d)` such that replaying `s[i] = ⊕ₘ cₘ · monomialₘ(window)`
/// reproduces `bits` exactly. Returns `None` if no such low-degree feedback fits (the GF(2) system is
/// inconsistent, or the recovered coefficients fail to regenerate the sequence). This is the degree-`d`
/// generalization of [`berlekamp_massey_gf2`] (which is the `d = 1` case): it compresses low-degree
/// nonlinear generators that fool the linear, 2-adic, and maximal-order tests.
pub fn detect_algebraic_recurrence(bits: &[bool], l: usize, d: usize) -> Option<Vec<bool>> {
    if l == 0 || bits.len() <= l {
        return None;
    }
    let monos = monomials(l, d);
    let m = monos.len();
    let words = m.div_ceil(64).max(1);
    let mut rows: Vec<(Vec<u64>, bool)> = Vec::with_capacity(bits.len() - l);
    for i in l..bits.len() {
        // Window variable `v` is `s[i-1-v]`, matching [`algebraic_generate`].
        let window: Vec<bool> = (0..l).map(|v| bits[i - 1 - v]).collect();
        let mut coeff = vec![0u64; words];
        for (mi, mono) in monos.iter().enumerate() {
            if eval_monomial(mono, &window) {
                coeff[mi / 64] |= 1u64 << (mi % 64);
            }
        }
        rows.push((coeff, bits[i]));
    }
    let coeffs = solve_gf2_system(&rows, m)?;
    if algebraic_generate(l, d, &coeffs, &bits[..l], bits.len()) != bits {
        return None; // the recovered feedback does not regenerate the whole sequence
    }
    Some(coeffs)
}

/// Replay a degree-`d`, order-`l` algebraic feedback register from ANF coefficients over
/// [`monomials`]`(l, d)` and an `l`-bit seed: `s[i] = ⊕ₘ coeffsₘ · monomialₘ(s[i-1],…,s[i-l])`. The
/// inverse of [`detect_algebraic_recurrence`].
pub fn algebraic_generate(l: usize, d: usize, coeffs: &[bool], seed: &[bool], total: usize) -> Vec<bool> {
    let monos = monomials(l, d);
    let mut out: Vec<bool> = seed.iter().take(l).copied().collect();
    for i in l..total {
        let window: Vec<bool> = (0..l).map(|v| out[i - 1 - v]).collect();
        let mut bit = false;
        for (mi, mono) in monos.iter().enumerate() {
            if coeffs.get(mi).copied().unwrap_or(false) && eval_monomial(mono, &window) {
                bit ^= true;
            }
        }
        out.push(bit);
    }
    out.truncate(total);
    out
}

/// The **algebraic complexity** of a bit sequence at maximal degree `max_degree`: the smallest order
/// `l` for which [`detect_algebraic_recurrence`] recovers a degree-`≤ max_degree` feedback, together
/// with the ANF coefficient count `M` (the true description size — `O(lᵈ)`, not the `2^l` truth table).
/// `None` if no order `≤ bits.len()/2` admits such a feedback at that degree.
pub fn algebraic_complexity(bits: &[bool], max_degree: usize) -> Option<(usize, usize)> {
    for l in 1..=(bits.len() / 2) {
        if let Some(coeffs) = detect_algebraic_recurrence(bits, l, max_degree) {
            let used = coeffs.iter().filter(|&&c| c).count();
            return Some((l, used.max(1)));
        }
    }
    None
}

// ---- Correlation attack: divide-and-conquer on nonlinear COMBINER generators (the hidden-state rung) --
//
// A nonlinear combiner runs several LFSRs in parallel and emits `z[i] = C(a₁[i],…,aₖ[i])` for a Boolean
// combining function `C`. Every rung so far is blind to it: `z` is a function of the *hidden* register
// outputs, not of its own past, so no feedback recovery (linear, word, carry, maximal-order, or
// algebraic) applies. But if `C` is correlated with one input — `Pr[z = aⱼ] = p ≠ ½` — then `z` is a
// noisy copy of that single LFSR through a binary symmetric channel, and Siegenthaler's attack recovers
// `aⱼ`'s initial state ALONE: try each of the `2^Lⱼ − 1` nonzero states, keep the one whose output best
// agrees with `z`. This is divide-and-conquer — cost `Σⱼ 2^Lⱼ` instead of `2^(Σ Lⱼ)` — and it isolates
// each constituent register independently. It is the classical break the earlier rungs cannot express.

/// The certified result of a correlation attack on one target LFSR: the recovered initial state and the
/// statistical edge it carries. The `init_state` is a re-checkable witness — regenerate the LFSR from it
/// and it reproduces the register the keystream leaks.
#[derive(Clone, Debug, PartialEq)]
pub struct CorrelationAttack {
    /// The register length `L` (`= taps.len()`).
    pub register_len: usize,
    /// The recovered initial state — the `L`-bit LFSR seed that best correlates with the keystream.
    pub init_state: Vec<bool>,
    /// `Pr[z = a]` over the sample — the correlation `p` (or `1 − p` if the leak is anti-correlated).
    pub agreement: f64,
    /// `|agreement − ½|` — the statistical edge; compare against [`spurious_bias_floor`].
    pub bias: f64,
    /// The number of keystream bits scored, `n`.
    pub samples: usize,
}

/// Siegenthaler's basic correlation attack: recover the initial state of the length-`L` LFSR with
/// feedback `taps` that best correlates with `keystream`, by exhaustive search over the `2^L − 1`
/// nonzero initial states (each scored by agreement with the keystream). Returns the best state with its
/// bias — significant only when `bias` clears [`spurious_bias_floor`]`(L, n)`. Cost `O(2^L·n)`; returns
/// `None` for an empty register, a register longer than the keystream, or `L > 20` (past the exhaustive
/// cap — the domain of the fast-correlation attack, its scaling successor).
pub fn correlation_attack(keystream: &[bool], taps: &[bool]) -> Option<CorrelationAttack> {
    let l = taps.len();
    let n = keystream.len();
    if l == 0 || l > 20 || n <= l {
        return None;
    }
    let mut best: Option<CorrelationAttack> = None;
    for code in 1u64..(1u64 << l) {
        let seed: Vec<bool> = (0..l).map(|k| (code >> k) & 1 == 1).collect();
        let stream = lfsr_generate(taps, &seed, n);
        let agree = stream.iter().zip(keystream).filter(|(a, b)| a == b).count();
        let bias = (agree as f64 / n as f64 - 0.5).abs();
        if best.as_ref().is_none_or(|b| bias > b.bias) {
            best = Some(CorrelationAttack {
                register_len: l,
                init_state: seed,
                agreement: agree as f64 / n as f64,
                bias,
                samples: n,
            });
        }
    }
    best
}

/// The spurious-bias floor: the expected maximum `|agreement − ½|` when the best of `2^register_len`
/// *uncorrelated* states is chosen on `n` samples — `≈ √(L·ln2 / 2n)`, from the tail of the maximum of
/// `2^L` centered binomials. A recovered [`CorrelationAttack::bias`] well above this floor is a genuine
/// correlation (a certified combiner leak); at or below it, the register does not measurably leak.
pub fn spurious_bias_floor(register_len: usize, samples: usize) -> f64 {
    if samples == 0 {
        return 1.0;
    }
    ((register_len as f64) * std::f64::consts::LN_2 / (2.0 * samples as f64)).sqrt()
}

// ---- Walsh spectrum & linear cryptanalysis: EVERY linear approximation at once (generalize Rung E) ---
//
// The correlation attack read ONE correlation, `Pr[z = aⱼ]`. Linear cryptanalysis reads them all. For a
// Boolean combining/filter function `C` on `n` inputs, the Walsh–Hadamard transform of its ±1 form
// `F(x) = (−1)^C(x)` is `Ŵ(w) = Σₓ (−1)^{C(x) ⊕ ⟨w,x⟩}` — the signed correlation of `C` with EVERY linear
// function `⟨w,x⟩` simultaneously, with `Pr[C(x)=⟨w,x⟩] = ½ + Ŵ(w)/2^{n+1}`. Rung E is exactly the slice
// at weight-1 masks (`w = eⱼ`); the full spectrum exposes the multi-register approximations E is blind
// to — a function can be first-order correlation-immune (every weight-1 `Ŵ` vanishes, so E finds
// nothing) yet leak a weight-2 approximation with large bias. The largest `|Ŵ(w)|` is the best linear
// approximation (the distinguisher); `nonlinearity = 2^{n-1} − ½·max|Ŵ|` measures distance to the
// nearest affine function. The symmetry break: one FWHT diagonalizes the whole correlation structure.
// Ceiling: a BENT function has a FLAT spectrum (all `|Ŵ| = 2^{n/2}`) — maximal nonlinearity, no
// exploitable linear approximation, the linear-incompressible residue.

/// The Walsh–Hadamard transform in place (the `±1` butterfly), on a slice whose length is a power of 2.
/// `a[w]` becomes `Σₓ a[x]·(−1)^{⟨w,x⟩}`.
pub fn fast_walsh_hadamard(a: &mut [i64]) {
    let n = a.len();
    debug_assert!(n.is_power_of_two(), "Walsh–Hadamard length must be a power of two");
    let mut len = 1;
    while len < n {
        let mut i = 0;
        while i < n {
            for j in i..i + len {
                let (x, y) = (a[j], a[j + len]);
                a[j] = x + y;
                a[j + len] = x - y;
            }
            i += 2 * len;
        }
        len <<= 1;
    }
}

/// The Walsh spectrum of a Boolean function given as its `2ⁿ`-entry truth table: `Ŵ(w) = Σₓ (−1)^{C(x) ⊕
/// ⟨w,x⟩}`, the signed correlation of `C` with every linear function `⟨w,x⟩`. Index `w` is the linear
/// mask (bit `i` selects variable `i`). `None` if the length is not a positive power of two.
pub fn walsh_spectrum(truth: &[bool]) -> Option<Vec<i64>> {
    if truth.is_empty() || !truth.len().is_power_of_two() {
        return None;
    }
    let mut a: Vec<i64> = truth.iter().map(|&b| if b { -1 } else { 1 }).collect();
    fast_walsh_hadamard(&mut a);
    Some(a)
}

/// The best linear approximation of a Boolean function: the mask `w` maximizing `|Ŵ(w)|`, with its bias
/// `|Pr[C(x)=⟨w,x⟩] − ½| = |Ŵ(w)| / 2^{n+1}`. When `skip_zero`, the trivial constant mask `w = 0` (which
/// measures `C`'s own imbalance) is excluded. Returns `(mask, bias)`, or `None` for a malformed table.
pub fn best_linear_approximation(truth: &[bool], skip_zero: bool) -> Option<(usize, f64)> {
    let spec = walsh_spectrum(truth)?;
    let denom = 2.0 * truth.len() as f64; // 2^{n+1}
    spec.iter()
        .enumerate()
        .skip(usize::from(skip_zero))
        .max_by_key(|(_, &c)| c.unsigned_abs())
        .map(|(w, &c)| (w, c.unsigned_abs() as f64 / denom))
}

/// The nonlinearity of a Boolean function: `2^{n-1} − ½·maxₘ|Ŵ(w)|` — its Hamming distance to the nearest
/// affine function. High ⇒ resistant to linear approximation; maximal (bent, `n` even) ⇒ `2^{n-1} −
/// 2^{n/2−1}`. `None` for a malformed table.
pub fn nonlinearity(truth: &[bool]) -> Option<u64> {
    let spec = walsh_spectrum(truth)?;
    let max_abs = spec.iter().map(|&c| c.unsigned_abs()).max()?;
    Some((truth.len() as u64) / 2 - max_abs / 2)
}

/// The correlation-immunity order of a Boolean function (Xiao–Massey): the largest `m` such that every
/// Walsh coefficient at a mask of Hamming weight `1..=m` vanishes. Order `≥ 1` ⇒ IMMUNE to the
/// first-order correlation attack (Rung E finds nothing) — yet a higher-weight coefficient may still
/// leak, which [`best_linear_approximation`] surfaces. `None` for a malformed table.
pub fn correlation_immunity_order(truth: &[bool]) -> Option<usize> {
    let spec = walsh_spectrum(truth)?;
    let n = truth.len().trailing_zeros() as usize;
    for m in 1..=n {
        let vanishes = spec
            .iter()
            .enumerate()
            .filter(|(w, _)| w.count_ones() as usize == m)
            .all(|(_, &c)| c == 0);
        if !vanishes {
            return Some(m - 1);
        }
    }
    Some(n)
}

/// The algebraic normal form (ANF) of a Boolean function: coefficients over the `2ⁿ` monomials `∏_{i∈S}
/// xᵢ` (S a variable bitmask), via the binary Möbius transform — the GF(2) dual of the Walsh–Hadamard
/// butterfly, and its own inverse over GF(2) (applying it to the ANF returns the truth table). `anf[S]` is
/// `true` iff the monomial `∏_{i∈S} xᵢ` is present. `None` for a malformed table.
pub fn anf(truth: &[bool]) -> Option<Vec<bool>> {
    if truth.is_empty() || !truth.len().is_power_of_two() {
        return None;
    }
    let n = truth.len().trailing_zeros();
    let mut a = truth.to_vec();
    for i in 0..n {
        let step = 1usize << i;
        let mut j = 0;
        while j < a.len() {
            for k in j..j + step {
                let lo = a[k];
                a[k + step] ^= lo;
            }
            j += step << 1;
        }
    }
    Some(a)
}

/// The algebraic degree of a Boolean function: the largest number of variables in any monomial present in
/// its ANF (0 for a constant, 1 for affine, up to `n`). `None` for a malformed table.
pub fn algebraic_degree(truth: &[bool]) -> Option<usize> {
    let a = anf(truth)?;
    Some(a.iter().enumerate().filter(|(_, &c)| c).map(|(m, _)| (m as u64).count_ones() as usize).max().unwrap_or(0))
}

/// The autocorrelation spectrum of a Boolean function: `r_f(a) = Σ_x (−1)^{f(x) ⊕ f(x⊕a)}`, the correlation
/// of `f` with its shift by `a`. Computed by Wiener–Khinchin — the Walsh transform of the squared Walsh
/// spectrum, scaled by `2ⁿ` — so it costs one extra butterfly over [`walsh_spectrum`]. `r_f(0) = 2ⁿ`
/// always; `|r_f(a)| = 2ⁿ` exactly when the derivative `f(x⊕a) ⊕ f(x)` is CONSTANT — i.e. `a` is a linear
/// structure. `None` for a malformed table.
pub fn autocorrelation(truth: &[bool]) -> Option<Vec<i64>> {
    let mut w = walsh_spectrum(truth)?;
    for c in w.iter_mut() {
        *c *= *c;
    }
    fast_walsh_hadamard(&mut w);
    let scale = truth.len() as i64;
    for c in w.iter_mut() {
        *c /= scale;
    }
    Some(w)
}

// ---- Algebraic immunity & annihilators: break the FILTER generator (the algebraic rung, Courtois–Meier)
//
// The correlation/Walsh rungs attack a combiner through its *statistical* leaks. The algebraic attack is
// exact: for a filter generator `z[t] = C(state_t)` (a single LFSR whose state is filtered by a nonlinear
// `C`), a low-degree ANNIHILATOR of `C` — a nonzero `g` with `g·C ≡ 0` — turns every keystream bit into a
// low-degree equation in the *initial state*. Where `z[t] = 1`, `C(state_t) = 1`, so `g(state_t) = 0`; and
// `state_t` is a LINEAR image of the initial state, so `g(state_t)` is a degree-`AI(C)` polynomial in the
// initial bits. Collect enough, linearize over monomials of degree `≤ AI(C)`, and solve for the whole
// state at once — polynomial for fixed `AI`, versus `2^L` brute force. The algebraic immunity `AI(C)` is
// the min degree of a nonzero annihilator of `C` or of `C⊕1`. The symmetry break: a low-degree
// annihilator is a hidden algebraic relation that compresses the `2^L` state search to a linear solve.
// Ceiling: a maximal-AI filter (`AI ≈ n/2`) leaves no low-degree relation — the algebraic-incompressible
// residue.

/// A re-checkable annihilator witness: `coeffs` is the ANF (over [`monomials`]`(n_vars, degree)`) of a
/// nonzero `g` of degree `degree` that vanishes on `C`'s support (`annihilates_complement = false`) or on
/// `C`'s zero-set (`= true`, i.e. `g` annihilates `C ⊕ 1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnihilatorWitness {
    pub n_vars: usize,
    pub degree: usize,
    pub coeffs: Vec<bool>,
    pub annihilates_complement: bool,
}

/// A basis for the kernel `{c : A·c = 0}` over GF(2), `rows` being the coefficient bitmasks over `ncols`
/// columns: one basis vector per free column (that free variable set to 1, the pivots back-substituted).
/// Empty if the columns are independent. Bitset RREF, so `ncols > 64` is fine.
fn gf2_kernel_basis(rows: &[Vec<u64>], ncols: usize) -> Vec<Vec<bool>> {
    let words = ncols.div_ceil(64).max(1);
    let mut mat: Vec<Vec<u64>> = rows.to_vec();
    let mut pivot_row_of_col = vec![usize::MAX; ncols];
    let mut pivot_cols: Vec<usize> = Vec::new();
    let mut r = 0usize;
    for c in 0..ncols {
        let (w, bit) = (c / 64, 1u64 << (c % 64));
        let Some(pr) = (r..mat.len()).find(|&i| mat[i][w] & bit != 0) else {
            continue;
        };
        mat.swap(r, pr);
        for i in 0..mat.len() {
            if i != r && mat[i][w] & bit != 0 {
                for k in 0..words {
                    mat[i][k] ^= mat[r][k];
                }
            }
        }
        pivot_row_of_col[c] = r;
        pivot_cols.push(c);
        r += 1;
    }
    // Each free column (no pivot) yields one basis vector: set it to 1, read pivots off their RREF rows
    // (a pivot row is `pivot_col ⊕ (free columns)`, so with only this free var at 1, `c[pivot] = row[free]`).
    (0..ncols)
        .filter(|&c| pivot_row_of_col[c] == usize::MAX)
        .map(|free| {
            let mut c = vec![false; ncols];
            c[free] = true;
            for &pc in &pivot_cols {
                let row = &mat[pivot_row_of_col[pc]];
                if (row[free / 64] >> (free % 64)) & 1 == 1 {
                    c[pc] = true;
                }
            }
            c
        })
        .collect()
}

/// A basis for the space of Boolean functions of degree `≤ d` (as ANF coefficients over [`monomials`]`(n,
/// d)`) that vanish on every point of `zero_set` (indices into `0..2ⁿ`) — every annihilator of that
/// support at that degree. Empty if only the zero function qualifies.
fn annihilator_basis(n: usize, d: usize, zero_set: &[usize]) -> Vec<Vec<bool>> {
    let monos = monomials(n, d);
    let m = monos.len();
    let words = m.div_ceil(64).max(1);
    let rows: Vec<Vec<u64>> = zero_set
        .iter()
        .map(|&x| {
            let window: Vec<bool> = (0..n).map(|i| (x >> i) & 1 == 1).collect();
            let mut coeff = vec![0u64; words];
            for (mi, mono) in monos.iter().enumerate() {
                if eval_monomial(mono, &window) {
                    coeff[mi / 64] |= 1u64 << (mi % 64);
                }
            }
            coeff
        })
        .collect();
    gf2_kernel_basis(&rows, m)
}

/// A single nonzero annihilator of `zero_set` at degree `≤ d`, or `None` if none exists.
fn annihilator(n: usize, d: usize, zero_set: &[usize]) -> Option<Vec<bool>> {
    annihilator_basis(n, d, zero_set).into_iter().next()
}

/// The **algebraic immunity** of a Boolean function given as its `2ⁿ` truth table: the minimum degree of
/// a nonzero annihilator of `C` or of `C ⊕ 1`, together with a re-checkable witness. Searching degrees
/// ascending, the first hit is the minimum (no lower-degree annihilator exists, or an earlier degree
/// would have found it). `None` for a malformed table. `AI ≤ ⌈n/2⌉` always.
pub fn algebraic_immunity(truth: &[bool]) -> Option<(usize, AnnihilatorWitness)> {
    if truth.is_empty() || !truth.len().is_power_of_two() {
        return None;
    }
    let n = truth.len().trailing_zeros() as usize;
    let support: Vec<usize> = (0..truth.len()).filter(|&x| truth[x]).collect();
    let zeros: Vec<usize> = (0..truth.len()).filter(|&x| !truth[x]).collect();
    for d in 0..=n {
        if let Some(g) = annihilator(n, d, &support) {
            return Some((d, AnnihilatorWitness { n_vars: n, degree: d, coeffs: g, annihilates_complement: false }));
        }
        if let Some(g) = annihilator(n, d, &zeros) {
            return Some((d, AnnihilatorWitness { n_vars: n, degree: d, coeffs: g, annihilates_complement: true }));
        }
    }
    None
}

/// Evaluate an ANF (coefficients over [`monomials`]`(n_vars, degree)`) at a point `x ∈ 0..2ⁿ`.
fn eval_anf(coeffs: &[bool], n_vars: usize, degree: usize, x: usize) -> bool {
    let window: Vec<bool> = (0..n_vars).map(|i| (x >> i) & 1 == 1).collect();
    monomials(n_vars, degree)
        .iter()
        .enumerate()
        .filter(|(mi, _)| coeffs.get(*mi).copied().unwrap_or(false))
        .fold(false, |acc, (_, mono)| acc ^ eval_monomial(mono, &window))
}

/// Re-check an [`AnnihilatorWitness`] against a truth table with zero trust in the producer: the witness
/// is nonzero and vanishes on the required set (support of `C`, or of `C ⊕ 1`). Confirms the
/// algebraic-immunity claim independently.
pub fn verify_annihilator(truth: &[bool], w: &AnnihilatorWitness) -> bool {
    if w.coeffs.iter().all(|&c| !c) || monomials(w.n_vars, w.degree).len() != w.coeffs.len() {
        return false;
    }
    (0..truth.len()).all(|x| {
        let must_vanish = if w.annihilates_complement { !truth[x] } else { truth[x] };
        !must_vanish || !eval_anf(&w.coeffs, w.n_vars, w.degree, x)
    })
}

/// Recover the initial state of a **filter generator** by the algebraic attack: a length-`l` LFSR with
/// feedback `taps` drives a filter `C` (truth table `filter_truth` over its `m = log₂` inputs, read as
/// `m` CONSECUTIVE state bits), emitting `keystream[t] = C(seq[t], …, seq[t+m−1])`. Using a min-degree
/// annihilator `g` of `C`, each applicable keystream bit becomes `g(⟨r_t,s₀⟩, …) = 0`, a degree-`AI`
/// equation in the initial state `s₀`; expanding over `s₀`-monomials of degree `≤ AI` and solving the
/// GF(2) system recovers `s₀`. Returns the `l`-bit initial state (verified by regeneration), or `None` if
/// there is no low-AI annihilator, the system is underdetermined, or `l > 64`.
pub fn algebraic_filter_attack(keystream: &[bool], taps: &[bool], filter_truth: &[bool]) -> Option<Vec<bool>> {
    let l = taps.len();
    let n = keystream.len();
    if l == 0 || l > 64 || !filter_truth.len().is_power_of_two() {
        return None;
    }
    let m = filter_truth.len().trailing_zeros() as usize;
    if m == 0 || n < l + m {
        return None;
    }
    let (ai, _) = algebraic_immunity(filter_truth)?;
    if ai == 0 {
        return None; // a constant filter carries no state
    }
    let g_monos = monomials(m, ai);
    // The whole annihilator space at degree AI, from BOTH sides: functions vanishing on the filter's
    // support (applied where the keystream is 1) and on its zero-set (applied where it is 0). A single
    // annihilator underdetermines the linearized system; the full basis makes it full rank.
    let support: Vec<usize> = (0..filter_truth.len()).filter(|&x| filter_truth[x]).collect();
    let zeros: Vec<usize> = (0..filter_truth.len()).filter(|&x| !filter_truth[x]).collect();
    let ann_when_one = annihilator_basis(m, ai, &support);
    let ann_when_zero = annihilator_basis(m, ai, &zeros);

    // Linear forms r_k (bitmask over the l initial-state bits) for each sequence position: seq[k]=⟨r_k,s₀⟩.
    let need = n + m;
    let mut r: Vec<u64> = Vec::with_capacity(need);
    for k in 0..need {
        if k < l {
            r.push(1u64 << k);
        } else {
            let mut acc = 0u64;
            for (j, &t) in taps.iter().enumerate() {
                if t {
                    acc ^= r[k - 1 - j];
                }
            }
            r.push(acc);
        }
    }

    // Column layout: monomials(l, ai), column 0 = the constant (pinned to 1 via a seed equation).
    let s_monos = monomials(l, ai);
    let ncols = s_monos.len();
    let col_of: std::collections::HashMap<u64, usize> = s_monos
        .iter()
        .enumerate()
        .map(|(i, mono)| (mono.iter().fold(0u64, |b, &v| b | (1u64 << v)), i))
        .collect();
    let words = ncols.div_ceil(64).max(1);

    // Expand an annihilator `g(y₀,…,y_{m-1})` with `yᵢ = ⟨r_{t+i}, s₀⟩` into an equation row over the
    // s₀-monomials (the constant monomial, mask 0, moves to the right-hand side).
    let expand = |g: &[bool], t: usize| -> Option<(Vec<u64>, bool)> {
        let mut acc: std::collections::HashSet<u64> = std::collections::HashSet::new();
        for (gi, mono) in g_monos.iter().enumerate() {
            if !g[gi] {
                continue;
            }
            let mut poly: std::collections::HashSet<u64> = std::collections::HashSet::from([0u64]);
            for &i in mono {
                let form = r[t + i];
                let mut next: std::collections::HashSet<u64> = std::collections::HashSet::new();
                for &pm in &poly {
                    let mut bits = form;
                    while bits != 0 {
                        let v = bits.trailing_zeros();
                        bits &= bits - 1;
                        let term = pm | (1u64 << v);
                        if !next.insert(term) {
                            next.remove(&term); // XOR: cancel duplicates
                        }
                    }
                }
                poly = next;
            }
            for pm in poly {
                if !acc.insert(pm) {
                    acc.remove(&pm);
                }
            }
        }
        let mut coeff = vec![0u64; words];
        let mut rhs = false;
        for mask in acc {
            if mask == 0 {
                rhs = true;
            } else {
                coeff[*col_of.get(&mask)? / 64] |= 1u64 << (col_of[&mask] % 64);
            }
        }
        Some((coeff, rhs))
    };

    let mut rows: Vec<(Vec<u64>, bool)> = Vec::new();
    let mut pin = vec![0u64; words];
    pin[0] = 1;
    rows.push((pin, true)); // pin the constant monomial to 1

    for t in 0..n.saturating_sub(m - 1) {
        let anns = if keystream[t] { &ann_when_one } else { &ann_when_zero };
        for g in anns {
            rows.push(expand(g, t)?);
        }
    }

    let sol = solve_gf2_system(&rows, ncols)?;
    // The degree-1 monomial columns hold the recovered state bits.
    let state: Vec<bool> = (0..l)
        .map(|i| col_of.get(&(1u64 << i)).map(|&ci| sol[ci]).unwrap_or(false))
        .collect();

    // Verify by regeneration: the recovered state must reproduce the keystream exactly.
    let seq = lfsr_generate(taps, &state, need);
    let regen: Vec<bool> = (0..n)
        .map(|t| {
            let idx = (0..m).fold(0usize, |a, i| a | (usize::from(seq[t + i]) << i));
            filter_truth[idx]
        })
        .collect();
    (regen == keystream).then_some(state)
}

// ---- Fast correlation attack: decode the LFSR-as-linear-code, no 2^L search (Meier–Staffelbach) ------
//
// The correlation attack (Rung E) recovered a leaking LFSR's initial state by exhaustive search over its
// 2^L states. The fast correlation attack removes the exponential: the LFSR's output is a codeword of a
// linear code (it satisfies its feedback recurrence at every position), so a noisy keystream `z = a ⊕
// noise` is a corrupted codeword, and recovering `a` is DECODING. The recurrence gives a low-weight
// parity check — `a[i] ⊕ ⊕ⱼ tapsⱼ·a[i−1−j] = 0` — and over GF(2) each squaring of the feedback
// polynomial (`g(x)^{2^k}`, the Frobenius map) gives another check of the SAME low weight, stretched.
// Instantiated at every anchor, these give many low-weight checks per bit; a Gallager hard-decision
// bit-flip decoder then flips the bits sitting in a majority of unsatisfied checks until the sequence
// satisfies its recurrence — a valid LFSR output — recovered in time polynomial in `L`, not `2^L`. The
// symmetry broken is the code's own automorphism: its dual carries low-weight words that pin the errors.
// Ceiling: too much noise (past the Meier–Staffelbach threshold) or a dense feedback polynomial (no
// low-weight checks) and the decoder cannot converge — the register resists.

/// Meier–Staffelbach fast correlation attack: recover the initial state of the length-`L` LFSR with
/// feedback `taps` from a noisy keystream `keystream` (a `z = a ⊕ noise` binary symmetric channel), by
/// iterative bit-flip decoding against the low-weight parity checks the feedback recurrence and its
/// Frobenius squares generate. Returns the recovered `L`-bit initial state when decoding converges to a
/// valid LFSR output that agrees with the keystream noticeably better than chance, else `None` (too much
/// noise or too few low-weight checks — the register resists). Polynomial in `L` — no `2^L` search.
pub fn fast_correlation_attack(keystream: &[bool], taps: &[bool], max_iters: usize) -> Option<Vec<bool>> {
    let (n, l) = (keystream.len(), taps.len());
    if l == 0 || n <= 2 * l {
        return None;
    }
    // Base parity-check offsets (relative to an anchor `i`): position `i` and each `i − (1+j)` with a tap.
    let mut base: Vec<usize> = vec![0];
    for (j, &t) in taps.iter().enumerate() {
        if t {
            base.push(1 + j);
        }
    }
    // Frobenius: squaring the feedback polynomial doubles the offsets and preserves the weight, so each
    // level adds independent low-weight checks that reach further across the sequence.
    let mut checks: Vec<Vec<usize>> = Vec::new();
    let mut scale = 1usize;
    for _ in 0..4 {
        let span = base.iter().map(|&o| o * scale).max().unwrap_or(0);
        if span >= n {
            break;
        }
        for i in span..n {
            checks.push(base.iter().map(|&o| i - o * scale).collect());
        }
        scale *= 2;
    }
    if checks.is_empty() {
        return None;
    }
    let mut per_bit = vec![0usize; n];
    for c in &checks {
        for &p in c {
            per_bit[p] += 1;
        }
    }

    let mut y = keystream.to_vec();
    for _ in 0..max_iters {
        let mut votes = vec![0usize; n];
        let mut all_satisfied = true;
        for c in &checks {
            if c.iter().fold(false, |a, &p| a ^ y[p]) {
                all_satisfied = false;
                for &p in c {
                    votes[p] += 1;
                }
            }
        }
        if all_satisfied {
            break;
        }
        let mut flipped = false;
        for i in 0..n {
            if per_bit[i] > 0 && votes[i] * 2 > per_bit[i] {
                y[i] ^= true;
                flipped = true;
            }
        }
        if !flipped {
            break; // stuck below the flip threshold — decoding cannot progress
        }
    }

    // Accept only a genuine codeword (the recurrence holds everywhere) that correlates with the keystream.
    let state = y[..l].to_vec();
    let regen = lfsr_generate(taps, &state, n);
    if regen != y {
        return None;
    }
    let agree = regen.iter().zip(keystream).filter(|(a, b)| a == b).count() as f64 / n as f64;
    (agree > 0.6).then_some(state)
}

// ---- Clock control / decimation: break the shrinking generator (the structural rung) -----------------
//
// Every rung so far assumes the keystream is a fixed function of register state read at a fixed cadence.
// A clock-controlled generator breaks that: in the SHRINKING GENERATOR a clock register `A` gates a data
// register `S`, emitting `S[i]` only when `A[i] = 1` and dropping it otherwise. The output is therefore a
// DATA-DEPENDENT decimation of `S` — it is not a fixed function of either register's own past, so no
// feedback, correlation, algebraic, or linear rung applies, and its linear complexity is enormous
// (≈ `2^{L_S}`). But the decimation is the exploitable symmetry: guess the clock register's `2^{L_A}`
// states (divide-and-conquer on the CLOCK alone), and a correct guess fixes the emit positions
// `{i : A[i] = 1}`. Each output bit is then `S` at a known position — `output[k] = ⟨r_{iₖ}, s₀⟩`, a
// LINEAR equation in `S`'s initial state — so a GF(2) solve recovers `S` and the whole generator falls.
// Ceiling: a large or securely-clocked control register makes the clock guess itself exponential.

/// The shrinking generator: clock register `A` (feedback `a_taps`, seed `a_seed`) gates data register `S`
/// (feedback `s_taps`, seed `s_seed`), emitting `S`'s bit exactly when `A`'s bit is `1`, until `out_len`
/// bits are produced. Its output is a data-dependent decimation of `S` with very high linear complexity.
pub fn shrinking_generator(
    a_taps: &[bool],
    a_seed: &[bool],
    s_taps: &[bool],
    s_seed: &[bool],
    out_len: usize,
) -> Vec<bool> {
    let clocks = out_len.saturating_mul(4) + 64; // A = 1 about half the time; 4× gives ample headroom
    let a = lfsr_generate(a_taps, a_seed, clocks);
    let s = lfsr_generate(s_taps, s_seed, clocks);
    let mut out = Vec::with_capacity(out_len);
    for i in 0..clocks {
        if a[i] {
            out.push(s[i]);
            if out.len() == out_len {
                break;
            }
        }
    }
    out
}

/// Attack the shrinking generator: recover the initial states of both the clock register `A` (feedback
/// `a_taps`) and the data register `S` (feedback `s_taps`) from `output` alone, by divide-and-conquer on
/// the clock. Each of the `2^{L_A}` clock states is tried; a guess fixes the emit positions, turning every
/// output bit into a linear equation `⟨r_{iₖ}, s₀⟩ = output[k]` in `S`'s initial state, solved over GF(2)
/// and verified by regeneration. Returns `(a_state, s_state)` for the first guess that reproduces the
/// output, or `None` (clock register too large — `L_A > 22` — or no consistent state). Exponential in
/// `L_A` only, not `L_A + L_S`.
pub fn attack_shrinking_generator(
    output: &[bool],
    a_taps: &[bool],
    s_taps: &[bool],
) -> Option<(Vec<bool>, Vec<bool>)> {
    let (la, ls) = (a_taps.len(), s_taps.len());
    let m = output.len();
    if la == 0 || la > 22 || ls == 0 || ls > 64 || m < ls {
        return None;
    }
    let clocks = m.saturating_mul(4) + 64;
    // S's linear forms r_k (bitmask over its ls initial-state bits): S[k] = ⟨r_k, s₀⟩.
    let mut r: Vec<u64> = Vec::with_capacity(clocks);
    for k in 0..clocks {
        if k < ls {
            r.push(1u64 << k);
        } else {
            let mut acc = 0u64;
            for (j, &t) in s_taps.iter().enumerate() {
                if t {
                    acc ^= r[k - 1 - j];
                }
            }
            r.push(acc);
        }
    }
    let words = ls.div_ceil(64).max(1);
    for code in 1u64..(1u64 << la) {
        let a_seed: Vec<bool> = (0..la).map(|k| (code >> k) & 1 == 1).collect();
        let a = lfsr_generate(a_taps, &a_seed, clocks);
        let mut positions = Vec::with_capacity(m);
        for (i, &bit) in a.iter().enumerate() {
            if bit {
                positions.push(i);
                if positions.len() == m {
                    break;
                }
            }
        }
        if positions.len() < m {
            continue; // this clock did not emit enough bits
        }
        let rows: Vec<(Vec<u64>, bool)> = positions
            .iter()
            .enumerate()
            .map(|(k, &pos)| {
                let mut coeff = vec![0u64; words];
                coeff[0] = r[pos];
                (coeff, output[k])
            })
            .collect();
        if let Some(sol) = solve_gf2_system(&rows, ls) {
            let s_seed: Vec<bool> = sol[..ls].to_vec();
            if shrinking_generator(a_taps, &a_seed, s_taps, &s_seed, m) == output {
                return Some((a_seed, s_seed));
            }
        }
    }
    None
}

// ---- XL / Gröbner escalation: solve where linearization underdetermines (the algebraic capstone) -----
//
// The algebraic-immunity rung linearizes a degree-`AI` system over its monomials and solves. When the
// number of monomials outruns the equations, that linear solve is underdetermined and stalls. The XL
// (eXtended Linearization) algorithm escalates: MULTIPLY each equation by monomials to manufacture more
// equations of bounded degree, then linearize the enlarged system. Multiplication surfaces implicit
// lower-degree consequences — the "degree fall" — that the original degree could not express, so a system
// unsolvable by linearization becomes solvable at a higher operating degree. And an inconsistency that
// only appears after multiplication is a REFUTATION: proof the system has no solution (the same certified
// UNSAT the algebraic-DRAT bridge consumes). The symmetry broken is the polynomial ideal's own structure
// — its Gröbner basis of implicit relations. Ceiling: the regularity degree — a truly random high-degree
// system forces the operating degree (and the monomial count) up to the wall.

/// The outcome of an XL solve over GF(2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolySolveResult {
    /// A satisfying assignment, verified against the original equations.
    Solved(Vec<bool>),
    /// The system is unsatisfiable — a certified refutation (the linearized system became inconsistent).
    Refuted,
    /// No verdict at the operating degrees tried (raise `max_degree`).
    Undetermined,
}

/// All monomials of degree `≤ deg` over `n` variables, as bitmasks (`≤ 64` variables).
fn monomial_masks(n: usize, deg: usize) -> Vec<u64> {
    monomials(n, deg).iter().map(|mono| mono.iter().fold(0u64, |b, &v| b | (1u64 << v))).collect()
}

/// Evaluate a monomial (bitmask; `0` = the constant `1`) at an assignment.
fn eval_mask(mask: u64, x: &[bool]) -> bool {
    let mut m = mask;
    let mut v = true;
    while m != 0 {
        v &= x[m.trailing_zeros() as usize];
        m &= m - 1;
    }
    v
}

/// Does `x` satisfy every equation (each an XOR of monomials that must be `0`)?
fn verify_poly_system(eqs: &[Vec<u64>], x: &[bool]) -> bool {
    eqs.iter().all(|eq| !eq.iter().fold(false, |a, &m| a ^ eval_mask(m, x)))
}

/// Solve a Boolean polynomial system over GF(2) by **XL** (eXtended Linearization). Each equation is an
/// XOR of monomials (bitmasks over the `n_vars` variables, `x²=x`; mask `0` is the constant) that must be
/// `0`. For each operating degree `d = 1..=max_degree`, every equation is multiplied by all monomials of
/// degree `≤ d − deg(eq)`, the enlarged system is linearized over the degree-`≤ d` monomials (with the
/// constant pinned to `1`) and solved: an inconsistency proves the system unsatisfiable ([`Refuted`]), a
/// solution whose degree-1 part satisfies the original equations is returned ([`Solved`]), otherwise the
/// degree is raised. This solves systems the plain degree-`d` linearization cannot, and refutes ones it
/// cannot see contradictory.
///
/// [`Refuted`]: PolySolveResult::Refuted
/// [`Solved`]: PolySolveResult::Solved
pub fn solve_polynomial_system_gf2(eqs: &[Vec<u64>], n_vars: usize, max_degree: usize) -> PolySolveResult {
    for d in 1..=max_degree {
        let cols = monomial_masks(n_vars, d);
        let col_of: std::collections::HashMap<u64, usize> =
            cols.iter().enumerate().map(|(i, &m)| (m, i)).collect();
        let ncols = cols.len();
        let words = ncols.div_ceil(64).max(1);

        let mut rows: Vec<(Vec<u64>, bool)> = Vec::new();
        // Pin the constant monomial to 1.
        let mut pin = vec![0u64; words];
        let c0 = col_of[&0];
        pin[c0 / 64] |= 1u64 << (c0 % 64);
        rows.push((pin, true));

        for eq in eqs {
            let deg_eq = eq.iter().map(|m| m.count_ones() as usize).max().unwrap_or(0);
            if deg_eq > d {
                continue; // cannot be represented at this operating degree yet
            }
            for mu in monomial_masks(n_vars, d - deg_eq) {
                // Multiply the equation by `mu` (monomial product = OR of masks over GF(2)).
                let mut acc: std::collections::HashSet<u64> = std::collections::HashSet::new();
                for &m in eq {
                    let prod = m | mu;
                    if !acc.insert(prod) {
                        acc.remove(&prod); // XOR cancellation
                    }
                }
                if acc.is_empty() {
                    continue;
                }
                let mut coeff = vec![0u64; words];
                let mut rhs = false;
                for mask in acc {
                    if mask == 0 {
                        rhs = true;
                    } else if let Some(&ci) = col_of.get(&mask) {
                        coeff[ci / 64] |= 1u64 << (ci % 64);
                    }
                }
                rows.push((coeff, rhs));
            }
        }

        match solve_gf2_system(&rows, ncols) {
            None => return PolySolveResult::Refuted,
            Some(sol) => {
                let x: Vec<bool> = (0..n_vars)
                    .map(|i| col_of.get(&(1u64 << i)).map(|&ci| sol[ci]).unwrap_or(false))
                    .collect();
                if verify_poly_system(eqs, &x) {
                    return PolySolveResult::Solved(x);
                }
            }
        }
    }
    PolySolveResult::Undetermined
}

/// Serialize a `BigInt` as sign byte + varint length + little-endian magnitude, trimmed to minimal
/// length (`to_le_bytes` is limb-aligned, so a small value would otherwise carry trailing zero bytes).
fn write_bigint(x: &crate::numeric::BigInt, out: &mut Vec<u8>) {
    let (neg, mut bytes) = x.to_le_bytes();
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    out.push(neg as u8);
    write_uvarint(bytes.len() as u64, out);
    out.extend_from_slice(&bytes);
}

/// The inverse of [`write_bigint`].
fn read_bigint(buf: &[u8], pos: &mut usize) -> Option<crate::numeric::BigInt> {
    let neg = *buf.get(*pos)? != 0;
    *pos += 1;
    let len = read_uvarint(buf, pos)? as usize;
    let bytes = buf.get(*pos..pos.checked_add(len)?)?;
    *pos += len;
    Some(crate::numeric::BigInt::from_le_bytes(neg, bytes))
}

/// If a byte column is an FCSR (carry-based) keystream — its bit expansion is the 2-adic rational
/// `p/q` and that rational regenerates every bit — return `(p, q)`. Catches carry generators (add-with-
/// carry, mod-2ⁿ LCGs) that fool every linear generator; `consider` keeps it only when `p/q` beats raw.
fn detect_fcsr_bytes(v: &[i64]) -> Option<(crate::numeric::BigInt, crate::numeric::BigInt)> {
    let bits = bytes_to_bits(v);
    let (p, q) = two_adic_reconstruct(&bits)?;
    if fcsr_generate(&p, &q, bits.len()) != bits {
        return None;
    }
    Some((p, q))
}

/// Recognize `v[i] = a + b·(i mod p)` for a small period `p` and synthesize the generator.
pub fn detect_modular_affine(v: &[i64]) -> Option<GenExpr> {
    const MAX_PERIOD: usize = 16;
    if v.len() < 4 {
        return None;
    }
    for p in 2..=MAX_PERIOD.min(v.len() / 2) {
        let a = v[0];
        let b = v[1].wrapping_sub(v[0]);
        if b != 0 && (0..v.len()).all(|i| v[i] == a.wrapping_add(b.wrapping_mul((i % p) as i64))) {
            return Some(GenExpr::Add(
                Box::new(GenExpr::Const(a)),
                Box::new(GenExpr::Mul(
                    Box::new(GenExpr::Const(b)),
                    Box::new(GenExpr::Mod(Box::new(GenExpr::Index), Box::new(GenExpr::Const(p as i64)))),
                )),
            ));
        }
    }
    None
}

// ---- Columnar fallback encoders ------------------------------------------------------------

/// Delta: first value then zig-zag successive differences. Wins on monotone columns.
pub fn delta_encode(out: &mut Vec<u8>, v: &[i64]) {
    out.push(T_INTS_DELTA);
    write_uvarint(v.len() as u64, out);
    if let Some(&first) = v.first() {
        write_uvarint(zigzag(first), out);
        let mut prev = first;
        for &x in &v[1..] {
            write_uvarint(zigzag(x.wrapping_sub(prev)), out);
            prev = x;
        }
    }
}

/// Delta-of-delta: first value, first delta, then zig-zag second differences. Wins on near-linear
/// progressions (timestamps with jitter).
pub fn dod_encode(out: &mut Vec<u8>, v: &[i64]) {
    out.push(T_INTS_DOD);
    write_uvarint(v.len() as u64, out);
    if v.is_empty() {
        return;
    }
    write_uvarint(zigzag(v[0]), out);
    if v.len() == 1 {
        return;
    }
    let mut prev_delta = v[1].wrapping_sub(v[0]);
    write_uvarint(zigzag(prev_delta), out);
    let mut prev = v[1];
    for &x in &v[2..] {
        let d = x.wrapping_sub(prev);
        write_uvarint(zigzag(d.wrapping_sub(prev_delta)), out);
        prev_delta = d;
        prev = x;
    }
}

/// Frame-of-reference: subtract the column minimum, bit-pack the residuals. Wins on clustered
/// columns (a small range around any base).
pub fn for_encode(out: &mut Vec<u8>, v: &[i64]) {
    out.push(T_INTS_FOR);
    write_uvarint(v.len() as u64, out);
    let min = v.iter().copied().min().unwrap_or(0);
    write_uvarint(zigzag(min), out);
    if v.is_empty() {
        out.push(0);
        return;
    }
    let max = v.iter().copied().max().unwrap();
    let range = (max as u64).wrapping_sub(min as u64);
    let width = if range == 0 { 0 } else { (64 - range.leading_zeros()) as u8 };
    out.push(width);
    if width > 0 {
        let residuals: Vec<u64> = v.iter().map(|&x| (x as u64).wrapping_sub(min as u64)).collect();
        out.extend_from_slice(&bitpack(&residuals, width));
    }
}

/// Run-length: (value, run-length) pairs. Wins on columns of repeated runs.
pub fn rle_encode(out: &mut Vec<u8>, v: &[i64]) {
    let mut runs: Vec<(i64, u64)> = Vec::new();
    for &x in v {
        match runs.last_mut() {
            Some(last) if last.0 == x => last.1 += 1,
            _ => runs.push((x, 1)),
        }
    }
    out.push(T_INTS_RLE);
    write_uvarint(runs.len() as u64, out);
    for (val, len) in runs {
        write_uvarint(zigzag(val), out);
        write_uvarint(len, out);
    }
}

/// Dictionary: distinct values (first-seen order) then a bit-packed index column. Wins on
/// low-cardinality columns.
pub fn dict_encode(v: &[i64]) -> Vec<u8> {
    let mut dict: Vec<i64> = Vec::new();
    let mut index_of: std::collections::HashMap<i64, u64> = std::collections::HashMap::new();
    let mut indices: Vec<u64> = Vec::with_capacity(v.len());
    for &x in v {
        let idx = *index_of.entry(x).or_insert_with(|| {
            dict.push(x);
            (dict.len() - 1) as u64
        });
        indices.push(idx);
    }
    let mut out = vec![T_INTS_DICT];
    write_uvarint(dict.len() as u64, &mut out);
    for &d in &dict {
        write_uvarint(zigzag(d), &mut out);
    }
    write_uvarint(v.len() as u64, &mut out);
    let iw = if dict.len() <= 1 { 0 } else { (64 - ((dict.len() - 1) as u64).leading_zeros()) as u8 };
    out.push(iw);
    if iw > 0 {
        out.extend_from_slice(&bitpack(&indices, iw));
    }
    out
}

/// Keep `cand` if it is smaller than the current `best` — the MDL argmin over candidate encodings.
pub fn consider(best: &mut Vec<u8>, cand: Vec<u8>) {
    if cand.len() < best.len() {
        *best = cand;
    }
}

// ---- The description engine (encode) -------------------------------------------------------

/// Build every applicable column encoding and append the smallest to `out`. The plain-varint
/// baseline is always a candidate, so the result is never larger than the varint form.
pub fn emit_best_int_column(v: &[i64], out: &mut Vec<u8>) {
    let mut best = Vec::new();
    best.push(T_INTS);
    leb128_encode(&mut best, v.iter().copied(), v.len());

    if let Some((base, stride)) = detect_affine(v) {
        let mut c = vec![T_INTS_AFFINE];
        write_uvarint(zigzag(base), &mut c);
        write_uvarint(zigzag(stride), &mut c);
        write_uvarint(v.len() as u64, &mut c);
        consider(&mut best, c);
    }
    if let Some((base, ratio)) = detect_geometric(v) {
        let mut c = vec![T_INTS_GEOMETRIC];
        write_uvarint(zigzag(base), &mut c);
        write_uvarint(zigzag(ratio), &mut c);
        write_uvarint(v.len() as u64, &mut c);
        consider(&mut best, c);
    }
    if let Some(p) = detect_period(v) {
        let mut c = vec![T_INTS_PERIODIC];
        write_uvarint(v.len() as u64, &mut c);
        emit_best_int_column(&v[..p], &mut c);
        consider(&mut best, c);
    }
    if let Some((dom, exc)) = detect_sparse(v) {
        let mut c = vec![T_INTS_SPARSE];
        write_uvarint(zigzag(dom), &mut c);
        write_uvarint(v.len() as u64, &mut c);
        write_uvarint(exc.len() as u64, &mut c);
        let mut prev = 0usize;
        for (i, x) in &exc {
            write_uvarint((i - prev) as u64, &mut c);
            prev = *i;
            write_uvarint(zigzag(*x), &mut c);
        }
        consider(&mut best, c);
    }
    if let Some((degree, seeds)) = detect_poly_generator(v) {
        let mut c = vec![T_INTS_POLY, degree];
        write_uvarint(v.len() as u64, &mut c);
        for &s in &seeds {
            write_uvarint(zigzag(s), &mut c);
        }
        consider(&mut best, c);
    }
    // Ship the GENERATOR, not the data: a linear-recurrence column (Fibonacci-class) becomes its order,
    // coefficients, and seeds — a handful of numbers regardless of n. Reaches sequences the polynomial
    // detector cannot (finite differences that never settle).
    if let Some((coeffs, seeds)) = detect_linear_recurrence(v) {
        let mut c = vec![T_INTS_LRECUR, coeffs.len() as u8];
        write_uvarint(v.len() as u64, &mut c);
        for &x in &coeffs {
            write_uvarint(zigzag(x), &mut c);
        }
        for &s in &seeds {
            write_uvarint(zigzag(s), &mut c);
        }
        consider(&mut best, c);
    }
    if let Some(expr) = detect_modular_affine(v) {
        let mut c = vec![T_GEN];
        serialize_gen(&expr, &mut c);
        write_uvarint(v.len() as u64, &mut c);
        consider(&mut best, c);
    }
    let mut delta = Vec::new();
    delta_encode(&mut delta, v);
    consider(&mut best, delta);

    let mut dod = Vec::new();
    dod_encode(&mut dod, v);
    consider(&mut best, dod);

    if !v.is_empty() && v.iter().all(|&x| (0..256).contains(&x)) {
        let mut b = vec![T_BYTES];
        write_uvarint(v.len() as u64, &mut b);
        b.extend(v.iter().map(|&x| x as u8));
        consider(&mut best, b);
    }

    let mut for_c = Vec::new();
    for_encode(&mut for_c, v);
    consider(&mut best, for_c);

    let mut rle = Vec::new();
    rle_encode(&mut rle, v);
    consider(&mut best, rle);

    consider(&mut best, dict_encode(v));

    // LAST RESORT — the LFSR attack: if this is a small byte column that NOTHING above compressed (it
    // looks random), run Berlekamp–Massey. A keystream from a short LFSR collapses to `O(L)` bits even
    // though every other generator sees noise. Gated by size and by "nothing else worked", so the
    // `O(bits²)` cost is paid only where it can pay off.
    if !v.is_empty()
        && v.len() <= LFSR_MAX_BYTES
        && best.len() >= v.len()
        && v.iter().all(|&x| (0..256).contains(&x))
    {
        if let Some((l, taps, seed)) = detect_lfsr_bytes(v) {
            let mut c = vec![T_INTS_LFSR];
            write_uvarint(v.len() as u64, &mut c);
            write_uvarint(l as u64, &mut c);
            let tap_vals: Vec<u64> = taps.iter().map(|&b| b as u64).collect();
            c.extend_from_slice(&bitpack(&tap_vals, 1));
            let seed_vals: Vec<u64> = seed.iter().map(|&b| b as u64).collect();
            c.extend_from_slice(&bitpack(&seed_vals, 1));
            consider(&mut best, c);
        }
    }

    // LAST RESORT — the CARRY attack: an FCSR keystream (add-with-carry / mod-2ⁿ LCG) fools every LINEAR
    // generator (its linear complexity is high), so it survives even the LFSR pass above. The 2-adic
    // Rational Approximation collapses it to a small rational `p/q`. Same gating as the LFSR pass, plus
    // its own size bound (the reconstruction is bignum).
    if !v.is_empty()
        && v.len() <= FCSR_MAX_BYTES
        && best.len() >= v.len()
        && v.iter().all(|&x| (0..256).contains(&x))
    {
        if let Some((p, q)) = detect_fcsr_bytes(v) {
            let mut c = vec![T_INTS_FCSR];
            write_uvarint(v.len() as u64, &mut c);
            write_bigint(&p, &mut c);
            write_bigint(&q, &mut c);
            consider(&mut best, c);
        }
    }

    out.extend_from_slice(&best);
}

/// The **computable upper bound on Kolmogorov complexity** of `v` over this description language:
/// the shortest self-delimiting program (from the generator menu) that reproduces `v`. The returned
/// bytes are a re-checkable witness — [`decode_int_seq`] reproduces `v` exactly.
pub fn describe_int_seq(v: &[i64]) -> Vec<u8> {
    let mut out = Vec::new();
    emit_best_int_column(v, &mut out);
    out
}

// ---- The description engine (decode) -------------------------------------------------------

/// Reject a decoded element count that exceeds `max_elements`, before any `count`-sized
/// materialization (the small-message-huge-output guard for generator columns).
#[inline]
fn bounded(n: u64, max_elements: usize) -> Option<usize> {
    let n = n as usize;
    (n <= max_elements).then_some(n)
}

/// Decode one int column whose tag byte has already been read as `tag`. The inverse of
/// [`emit_best_int_column`]; returns `None` for a non-int tag or a corrupt/hostile body.
/// `max_elements` caps generator expansion; `depth` bounds the periodic-block recursion.
pub fn decode_int_column_body(
    tag: u8,
    buf: &[u8],
    pos: &mut usize,
    max_elements: usize,
    depth: u32,
) -> Option<Vec<i64>> {
    Some(match tag {
        // Adaptive sign: the count's low bit says whether the column was zig-zag encoded.
        T_INTS => {
            let header = read_uvarint(buf, pos)?;
            let signed = header & 1 == 1;
            let n = bounded(header >> 1, max_elements)?;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            for _ in 0..n {
                let u = read_uvarint(buf, pos)?;
                v.push(if signed { unzigzag(u) } else { u as i64 });
            }
            v
        }
        // Closed form `base + stride·i`, replayed with the SAME wrapping arithmetic the encoder verified.
        T_INTS_AFFINE => {
            let base = unzigzag(read_uvarint(buf, pos)?);
            let stride = unzigzag(read_uvarint(buf, pos)?);
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            for i in 0..n {
                v.push(base.wrapping_add((i as i64).wrapping_mul(stride)));
            }
            v
        }
        // Geometric generator: replay `wrapping_mul` — exact even across overflow.
        T_INTS_GEOMETRIC => {
            let base = unzigzag(read_uvarint(buf, pos)?);
            let ratio = unzigzag(read_uvarint(buf, pos)?);
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            let mut cur = base;
            for _ in 0..n {
                v.push(cur);
                cur = cur.wrapping_mul(ratio);
            }
            v
        }
        // Cyclic generator: decode the period BLOCK (itself a best-encoded column), emit `block[i % p]`.
        T_INTS_PERIODIC => {
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
            if depth >= DECODE_MAX_DEPTH {
                return None;
            }
            let block_tag = *buf.get(*pos)?;
            *pos += 1;
            let block = decode_int_column_body(block_tag, buf, pos, max_elements, depth + 1)?;
            let p = block.len();
            if p == 0 {
                return None; // an empty block has no period — malformed
            }
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            for i in 0..n {
                v.push(block[i % p]);
            }
            v
        }
        // Sparse column: fill with the dominant value, patch the delta-indexed exceptions.
        T_INTS_SPARSE => {
            let dom = unzigzag(read_uvarint(buf, pos)?);
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
            let num_exc = bounded(read_uvarint(buf, pos)?, max_elements)?;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            v.resize(n, dom);
            let mut idx = 0usize;
            for _ in 0..num_exc {
                idx = idx.checked_add(read_uvarint(buf, pos)? as usize)?;
                let val = unzigzag(read_uvarint(buf, pos)?);
                *v.get_mut(idx)? = val; // out-of-range exception index → clean None
            }
            v
        }
        // Polynomial generator: read the degree-bounded seeds, replay the difference engine.
        T_INTS_POLY => {
            let degree = *buf.get(*pos)? as usize;
            *pos += 1;
            if degree > MAX_POLY_DEGREE {
                return None;
            }
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
            let mut seeds = Vec::with_capacity(degree + 1);
            for _ in 0..=degree {
                seeds.push(unzigzag(read_uvarint(buf, pos)?));
            }
            reconstruct_poly(&seeds, n)
        }
        // Linear-recurrence generator: read the order-bounded coefficients + seeds, replay the recurrence.
        T_INTS_LRECUR => {
            let k = *buf.get(*pos)? as usize;
            *pos += 1;
            if k == 0 || k > MAX_RECUR_ORDER {
                return None; // malformed / untrusted order
            }
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
            let mut coeffs = Vec::with_capacity(k);
            for _ in 0..k {
                coeffs.push(unzigzag(read_uvarint(buf, pos)?));
            }
            let mut seeds = Vec::with_capacity(k);
            for _ in 0..k {
                seeds.push(unzigzag(read_uvarint(buf, pos)?));
            }
            reconstruct_recurrence(&coeffs, &seeds, n)
        }
        // LFSR keystream: read the linear complexity L, the L feedback taps and L seed bits, then
        // replay the register for 8·n bits and pack back to n bytes.
        T_INTS_LFSR => {
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?; // byte count
            let l = read_uvarint(buf, pos)? as usize;
            let total_bits = n.checked_mul(8)?;
            if l > total_bits {
                return None;
            }
            let nbytes = l.div_ceil(8);
            let tap_bytes = buf.get(*pos..pos.checked_add(nbytes)?)?;
            *pos += nbytes;
            let taps: Vec<bool> = bitunpack(tap_bytes, l, 1)?.into_iter().map(|x| x == 1).collect();
            let seed_bytes = buf.get(*pos..pos.checked_add(nbytes)?)?;
            *pos += nbytes;
            let seed: Vec<bool> = bitunpack(seed_bytes, l, 1)?.into_iter().map(|x| x == 1).collect();
            bits_to_bytes(&lfsr_generate(&taps, &seed, total_bits))
        }
        // FCSR keystream: read the byte count and the rational p/q, then replay the 2-adic expansion.
        T_INTS_FCSR => {
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?; // byte count
            let p = read_bigint(buf, pos)?;
            let q = read_bigint(buf, pos)?;
            if !q.is_odd() {
                return None; // an FCSR connection integer must be odd
            }
            let total_bits = n.checked_mul(8)?;
            bits_to_bytes(&fcsr_generate(&p, &q, total_bits))
        }
        // General generator: parse the bounded `GenExpr`, evaluate it at 0..n in the sandbox.
        T_GEN => {
            let mut budget = MAX_GEN_NODES;
            let expr = deserialize_gen(buf, pos, &mut budget, 0)?;
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            for i in 0..n {
                v.push(gen_eval(&expr, i as i64));
            }
            v
        }
        // Byte column: read the count, widen each raw byte to an Int.
        T_BYTES => {
            let n = read_uvarint(buf, pos)? as usize;
            let raw = buf.get(*pos..pos.checked_add(n)?)?;
            *pos += n;
            raw.iter().map(|&b| b as i64).collect()
        }
        // Delta column: cumulative-sum the zig-zag differences.
        T_INTS_DELTA => {
            let n = read_uvarint(buf, pos)? as usize;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            if n > 0 {
                let mut cur = unzigzag(read_uvarint(buf, pos)?);
                v.push(cur);
                for _ in 1..n {
                    cur = cur.wrapping_add(unzigzag(read_uvarint(buf, pos)?));
                    v.push(cur);
                }
            }
            v
        }
        // Delta-of-delta column: double cumulative sum (the inverse of `dod_encode`).
        T_INTS_DOD => {
            let n = read_uvarint(buf, pos)? as usize;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            if n > 0 {
                let first = unzigzag(read_uvarint(buf, pos)?);
                v.push(first);
                if n > 1 {
                    let mut prev_delta = unzigzag(read_uvarint(buf, pos)?);
                    let mut prev = first.wrapping_add(prev_delta);
                    v.push(prev);
                    for _ in 2..n {
                        prev_delta = prev_delta.wrapping_add(unzigzag(read_uvarint(buf, pos)?));
                        prev = prev.wrapping_add(prev_delta);
                        v.push(prev);
                    }
                }
            }
            v
        }
        // Frame-of-reference column: unpack the bit-packed residuals, add the minimum.
        T_INTS_FOR => {
            let n = read_uvarint(buf, pos)? as usize;
            let min = unzigzag(read_uvarint(buf, pos)?);
            let width = *buf.get(*pos)?;
            *pos += 1;
            if width > 64 {
                return None;
            }
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            if width == 0 {
                for _ in 0..n {
                    v.push(min);
                }
            } else {
                let nbytes = n.checked_mul(width as usize)?.div_ceil(8);
                let bytes = buf.get(*pos..pos.checked_add(nbytes)?)?;
                *pos += nbytes;
                for r in bitunpack(bytes, n, width)? {
                    v.push(r.wrapping_add(min as u64) as i64);
                }
            }
            v
        }
        // Run-length column: expand each (value, run-length) pair, capped against a corrupt length.
        T_INTS_RLE => {
            let runs = read_uvarint(buf, pos)? as usize;
            let mut v: Vec<i64> = Vec::new();
            for _ in 0..runs {
                let val = unzigzag(read_uvarint(buf, pos)?);
                let len = read_uvarint(buf, pos)? as usize;
                if v.len().checked_add(len)? > RLE_MAX_TOTAL {
                    return None;
                }
                v.resize(v.len() + len, val);
            }
            v
        }
        // Dictionary column: read the distinct values, map the bit-packed indices back through them.
        T_INTS_DICT => {
            let d = read_uvarint(buf, pos)? as usize;
            let mut dict = Vec::with_capacity(d.min(PREALLOC_CAP));
            for _ in 0..d {
                dict.push(unzigzag(read_uvarint(buf, pos)?));
            }
            let n = read_uvarint(buf, pos)? as usize;
            let iw = *buf.get(*pos)?;
            *pos += 1;
            if iw > 64 {
                return None;
            }
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            if iw == 0 {
                if n > 0 {
                    let val = *dict.first()?;
                    v.resize(n, val);
                }
            } else {
                let nbytes = n.checked_mul(iw as usize)?.div_ceil(8);
                let bytes = buf.get(*pos..pos.checked_add(nbytes)?)?;
                *pos += nbytes;
                for ix in bitunpack(bytes, n, iw)? {
                    v.push(*dict.get(ix as usize)?);
                }
            }
            v
        }
        _ => return None,
    })
}

/// Decode a full [`describe_int_seq`] byte string back to the exact sequence. Requires the whole
/// buffer to be consumed (a clean round-trip witness). Uses a default element cap.
pub fn decode_int_seq(bytes: &[u8]) -> Option<Vec<i64>> {
    let mut pos = 0usize;
    let tag = *bytes.get(pos)?;
    pos += 1;
    let v = decode_int_column_body(tag, bytes, &mut pos, DEFAULT_MAX_ELEMENTS, 0)?;
    (pos == bytes.len()).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrips(v: &[i64]) {
        let enc = describe_int_seq(v);
        let dec = decode_int_seq(&enc);
        assert_eq!(dec.as_deref(), Some(v), "round-trip failed for {v:?} (enc {enc:?})");
    }

    #[test]
    fn affine_round_trips_and_beats_varint() {
        let v: Vec<i64> = (0..1000).map(|i| 10 + 7 * i).collect();
        roundtrips(&v);
        // The generator (3 numbers) must be far smaller than 1000 varints.
        assert!(describe_int_seq(&v).len() < 20, "affine must ship as a generator, not data");
    }

    #[test]
    fn geometric_round_trips() {
        let v: Vec<i64> = (0..40).map(|i| 3i64.wrapping_mul(2i64.wrapping_pow(i))).collect();
        roundtrips(&v);
    }

    #[test]
    fn polynomial_round_trips() {
        let v: Vec<i64> = (0..500).map(|i| i * i - 3 * i + 5).collect();
        roundtrips(&v);
        assert!(describe_int_seq(&v).len() < 30, "poly must ship as finite-difference seeds");
    }

    #[test]
    fn linear_recurrence_ships_the_generator() {
        // Fibonacci — order-2 recurrence, NOT polynomial (its finite differences never settle), so
        // ONLY the recurrence detector catches it: 60 terms collapse to a handful of numbers.
        let mut fib = vec![0i64, 1];
        while fib.len() < 60 {
            let n = fib.len();
            fib.push(fib[n - 1].wrapping_add(fib[n - 2]));
        }
        roundtrips(&fib);
        assert!(describe_int_seq(&fib).len() < 15, "Fibonacci ships as (order, coeffs, seeds), not data");

        // Lucas (same recurrence, different seeds).
        let mut lucas = vec![2i64, 1];
        while lucas.len() < 60 {
            let n = lucas.len();
            lucas.push(lucas[n - 1].wrapping_add(lucas[n - 2]));
        }
        roundtrips(&lucas);
        assert!(describe_int_seq(&lucas).len() < 15);

        // Pell: c = [2, 1].
        let mut pell = vec![0i64, 1];
        while pell.len() < 50 {
            let n = pell.len();
            pell.push(2i64.wrapping_mul(pell[n - 1]).wrapping_add(pell[n - 2]));
        }
        roundtrips(&pell);
        assert!(describe_int_seq(&pell).len() < 15);

        // A general order-3 recurrence v[i] = v[i-1] + v[i-2] − v[i-3].
        let mut r3 = vec![1i64, 2, 3];
        while r3.len() < 40 {
            let n = r3.len();
            r3.push(r3[n - 1].wrapping_add(r3[n - 2]).wrapping_sub(r3[n - 3]));
        }
        roundtrips(&r3);
        assert!(describe_int_seq(&r3).len() < 18);
    }

    #[test]
    fn berlekamp_massey_recovers_the_shortest_lfsr() {
        // Connection polynomial 1 + x + x³ → taps [1,0,1] (s[i] = s[i-1] ⊕ s[i-3]); linear complexity 3.
        let taps = vec![true, false, true];
        let seed = vec![true, false, false];
        let seq = lfsr_generate(&taps, &seed, 40);
        let (l, recovered) = berlekamp_massey_gf2(&seq);
        assert_eq!(l, 3, "a length-3 LFSR has linear complexity 3");
        // The recovered LFSR regenerates the whole sequence from its first L bits (the attack).
        assert_eq!(lfsr_generate(&recovered, &seq[..l], seq.len()), seq);
        // A well-mixed random bit sequence has linear complexity ≈ n/2 — incompressible as an LFSR.
        // (splitmix64's nonlinear finalizer; a plain xorshift low bit is deliberately NOT used — BM
        // correctly reports its ~64 linear complexity, exposing that generator's linear weakness.)
        let mut st = 0x1234_5678u64;
        let rnd: Vec<bool> = (0..200)
            .map(|_| {
                st = st.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut z = st;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                z ^= z >> 31;
                z & 1 == 1
            })
            .collect();
        let (lr, _) = berlekamp_massey_gf2(&rnd);
        assert!((80..=120).contains(&lr), "high-quality random complexity ≈ n/2, got {lr}");
        // Bit ↔ byte conversions round-trip.
        let bytes = vec![0x41i64, 0x00, 0xFF, 0x7E];
        assert_eq!(bits_to_bytes(&bytes_to_bits(&bytes)), bytes);
    }

    #[test]
    fn lfsr_keystream_bytes_compress_to_the_register() {
        // A maximal length-7 LFSR (x⁷+x³+1, bit-period 127) — its 200-byte keystream has byte-period
        // 127, beyond the periodic detector's n/2 reach, and looks random to every arithmetic generator.
        // Berlekamp–Massey recovers the 7-bit register and the whole column collapses to a handful of
        // bytes — the classic stream-cipher attack, here as a compressor.
        let taps = vec![false, false, true, false, false, false, true]; // c₃ = c₇ = 1
        let seed = vec![true, false, true, true, false, false, true];
        let bits = lfsr_generate(&taps, &seed, 200 * 8);
        let bytes = bits_to_bytes(&bits);
        roundtrips(&bytes);
        let enc = describe_int_seq(&bytes);
        assert_eq!(enc[0], T_INTS_LFSR, "the keystream ships as its LFSR register (tag), got {}", enc[0]);
        assert!(enc.len() < 20, "200 bytes collapse to the 7-bit register, got {} bytes", enc.len());
    }

    #[test]
    fn berlekamp_massey_over_gf256_recovers_a_word_lfsr() {
        // GF(2⁸) field sanity: 3⁵¹ ≠ 1 but 3²⁵⁵ = 1 (generator), and inv is a true inverse.
        assert_eq!(Gf256(0x53).mul(Gf256(0xCA)), Gf256(0x53).mul(Gf256(0xCA)));
        assert_eq!(Gf256(0x53).mul(Gf256(0x53).inv()), Gf256::one());
        assert_eq!(Gf256(0xFF).mul(Gf256(0xFF).inv()), Gf256::one());
        // An order-3 word-LFSR over GF(256): s[i] = c₁·s[i-1] + c₂·s[i-2] + c₃·s[i-3] with field
        // coefficients. Berlekamp–Massey over GF(256) recovers order 3 directly — exercising the
        // b_disc bookkeeping the GF(2) path never touches.
        let taps = vec![Gf256(0x02), Gf256(0x8d), Gf256(0x1f)];
        let seed = vec![Gf256(0x41), Gf256(0x9c), Gf256(0x07)];
        let seq = lfsr_generate_field(&taps, &seed, 60);
        let (l, recovered) = berlekamp_massey_field(&seq);
        assert_eq!(l, 3, "order-3 word-LFSR has GF(256) linear complexity 3, got {l}");
        assert_eq!(lfsr_generate_field(&recovered, &seq[..l], seq.len()), seq, "recovered register regenerates it");
    }

    #[test]
    fn two_adic_complexity_detects_fcsr_keystreams() {
        use crate::numeric::BigInt;
        // FCSR: connection integer q = 19 (odd), numerator p = 3. Its 2-adic expansion is an FCSR
        // keystream — the carry makes it nonlinear over GF(2), so no field-BM sees its structure.
        let bits = fcsr_generate(&BigInt::from_i64(3), &BigInt::from_i64(19), 120);
        let (rp, rq) = two_adic_reconstruct(&bits).expect("a clean 2-adic rational");
        assert_eq!(fcsr_generate(&rp, &rq, bits.len()), bits, "the recovered FCSR regenerates the keystream");
        let tac = two_adic_complexity(&bits);
        assert!(tac < 12, "a small FCSR has low 2-adic complexity, got {tac}");
        // The carry makes it MORE complex to a LINEAR view than to a 2-adic one — BM sees more.
        assert!(berlekamp_massey_gf2(&bits).0 > tac, "the FCSR is simpler 2-adically than linearly");
        // A well-mixed random sequence has 2-adic complexity ≈ n/2.
        let mut st = 0xABCD_1234u64;
        let rnd: Vec<bool> = (0..200)
            .map(|_| {
                st = st.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut z = st;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                z ^= z >> 31;
                z & 1 == 1
            })
            .collect();
        assert!(two_adic_complexity(&rnd) > 60, "random ≈ n/2 2-adic complexity, got {}", two_adic_complexity(&rnd));
    }

    #[test]
    fn fcsr_keystream_bytes_compress_to_the_rational() {
        use crate::numeric::BigInt;
        // A large connection integer q = 1000003: long period (not periodic), HIGH linear complexity
        // (fools the LFSR compressor), tiny 2-adic complexity. Only the FCSR generator collapses it.
        let bits = fcsr_generate(&BigInt::from_i64(7), &BigInt::from_i64(1_000_003), 100 * 8);
        let bytes = bits_to_bytes(&bits);
        roundtrips(&bytes);
        let enc = describe_int_seq(&bytes);
        assert_eq!(enc[0], T_INTS_FCSR, "the FCSR keystream ships as its rational p/q (tag), got {}", enc[0]);
        assert!(enc.len() < 20, "100 bytes collapse to a small rational, got {} bytes", enc.len());
    }

    #[test]
    fn maximal_order_complexity_catches_nonlinear_feedback() {
        // A De Bruijn sequence of order L=6: generated by a NONLINEAR order-6 feedback (prefer-one),
        // full period 2⁶=64, every 6-window appearing exactly once. Its linear complexity is ~2⁵ and its
        // 2-adic complexity is high (both tools see it as complex), but its maximal order complexity is
        // exactly the register order 6 — the nonlinear rung sees the short register the linear rungs cannot.
        let order = 6;
        let period = 1usize << order;
        let mut db: Vec<bool> = vec![false; order];
        let mut seen: std::collections::HashSet<Vec<bool>> = std::collections::HashSet::new();
        seen.insert(db[..order].to_vec());
        while db.len() < period {
            let mut w: Vec<bool> = db[db.len() - (order - 1)..].to_vec();
            w.push(true);
            if seen.contains(&w) {
                w.pop();
                w.push(false);
            }
            seen.insert(w.clone());
            db.push(*w.last().unwrap());
        }
        let bits: Vec<bool> = [db.as_slice(), db.as_slice(), db.as_slice()].concat(); // 3 cyclic copies
        let moc = maximal_order_complexity(&bits);
        let lin = berlekamp_massey_gf2(&bits).0;
        assert!((5..=7).contains(&moc), "the order-6 De Bruijn register has MOC ≈ 6, got {moc}");
        // The SAME sequence fools the LINEAR tools — they see far higher complexity than its true order.
        assert!(lin > moc, "nonlinear feedback fools linear complexity (BM {lin} > MOC {moc})");
        assert!(two_adic_complexity(&bits) > moc, "nonlinear feedback fools 2-adic complexity too");
        // MOC ≤ linear complexity for EVERYTHING (it is the shorter, more general register).
        let mut st = 0x2468_ace0_1357_9bdfu64;
        let rnd: Vec<bool> = (0..300)
            .map(|_| {
                st = st.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut z = st;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                z ^= z >> 31;
                z & 1 == 1
            })
            .collect();
        assert!(maximal_order_complexity(&rnd) <= berlekamp_massey_gf2(&rnd).0, "MOC ≤ linear complexity");
    }

    #[test]
    fn algebraic_recurrence_recovers_low_degree_nonlinear_feedback() {
        // A QUADRATIC (degree-2) order-8 nonlinear feedback register:
        //   s[i] = s[i-1] ⊕ s[i-5] ⊕ s[i-6] ⊕ s[i-8] ⊕ (s[i-1] AND s[i-6]).
        // Its linear complexity is 246 (the linear rung is fooled — Berlekamp–Massey needs a 246-stage
        // LFSR), its 2-adic complexity is high, and maximal order complexity would diagnose "order 8"
        // only as a 2⁸=256-entry truth table. The ALGEBRAIC recovery returns the sparse degree-2 ANF
        // that regenerates it — 37 coefficients — compressing a nonlinear generator every earlier rung
        // can only measure.
        let seed: Vec<bool> = (0..8).map(|k| (0x9E37u64 >> k) & 1 == 1).collect();
        let mut bits = seed.clone();
        while bits.len() < 600 {
            let i = bits.len();
            let s = |k: usize| bits[i - k];
            bits.push(s(1) ^ s(5) ^ s(6) ^ s(8) ^ (s(1) & s(6)));
        }

        let coeffs = detect_algebraic_recurrence(&bits, 8, 2).expect("recovers the degree-2 feedback");
        assert_eq!(
            algebraic_generate(8, 2, &coeffs, &bits[..8], bits.len()),
            bits,
            "the recovered ANF regenerates the whole nonlinear sequence"
        );

        // The description is the sparse ANF: M = 1 + C(8,1) + C(8,2) = 37 monomials, versus the 2⁸ = 256
        // truth-table entries maximal order complexity would need for the same register.
        assert_eq!(coeffs.len(), 37, "degree-2 ANF over 8 vars has 37 monomials");
        assert!(coeffs.len() < (1usize << 8), "the ANF is far sparser than the full truth table");

        // The quadratic term genuinely fools the LINEAR rung — Berlekamp–Massey reports complexity far
        // above the true register order — so nothing below this rung could have compressed it.
        let lin = berlekamp_massey_gf2(&bits).0;
        assert!(lin > 200, "the nonlinear term drives linear complexity to 246, got {lin}");
        assert!(two_adic_complexity(&bits) > 8, "and 2-adic complexity is high too — carry tools miss it");

        // A degree-1 (purely affine) fit CANNOT reproduce it — the compression is genuinely nonlinear.
        assert!(detect_algebraic_recurrence(&bits, 8, 1).is_none(), "no affine order-8 recurrence fits");

        // `algebraic_complexity` finds the same short register at degree 2 with its true coefficient count.
        let (l, m) = algebraic_complexity(&bits, 2).expect("degree-2 algebraic complexity exists");
        assert!(l <= 8, "the algebraic recovery finds an order-≤8 register, got {l}");
        assert!(m <= 37, "and describes it in at most its 37 ANF coefficients, got {m}");
    }

    #[test]
    fn algebraic_recovery_is_a_strict_generalization_of_berlekamp_massey() {
        // At degree 1 the algebraic recovery IS Berlekamp–Massey: a pure GF(2) LFSR keystream must be
        // recovered by the degree-1 solve and regenerate identically. (The affine ANF is not unique when
        // the constant column is dependent on the taps over the sample, so we assert regeneration — the
        // real correctness witness — not a specific coefficient vector.)
        let taps = vec![true, false, false, true, false, true]; // an order-6 LFSR
        let seed = vec![true, false, true, true, false, false];
        let bits = lfsr_generate(&taps, &seed, 200);
        let coeffs = detect_algebraic_recurrence(&bits, 6, 1).expect("degree-1 recovery = Berlekamp–Massey");
        assert_eq!(
            algebraic_generate(6, 1, &coeffs, &bits[..6], bits.len()),
            bits,
            "the degree-1 ANF regenerates the linear keystream"
        );
        // The linear rung already handles this one — Berlekamp–Massey finds the same short register.
        assert!(berlekamp_massey_gf2(&bits).0 <= 6, "an LFSR keystream has linear complexity ≤ its order");
    }

    #[test]
    fn correlation_attack_breaks_the_geffe_combiner_register_by_register() {
        // The Geffe generator: three LFSRs feed a multiplexer z[i] = x2[i] ? x1[i] : x3[i]. Its output
        // is a function of the HIDDEN register outputs, not of its own past — so every feedback-recovery
        // rung (linear, word, carry, maximal-order, algebraic) is blind to it. But Geffe leaks: z agrees
        // with x1 and with x3 three-quarters of the time (Pr[z=x1]=Pr[z=x3]=¾), while x2 is invisible
        // (Pr[z=x2]=½). The correlation attack recovers x1 and x3 ALONE — divide-and-conquer.
        let n = 2000;
        let taps1 = [false, false, true, false, false, false, true]; // L=7, period 127
        let taps2 = [false, false, true, false, true]; // L=5, period 31 (the protected middle)
        let taps3 = [false, false, false, false, true, false, false, false, true]; // L=9, period 511
        let seed1 = [true, false, true, true, false, false, true];
        let seed2 = [true, true, false, false, true];
        let seed3 = [true, false, false, true, false, true, true, false, true];
        let x1 = lfsr_generate(&taps1, &seed1, n);
        let x2 = lfsr_generate(&taps2, &seed2, n);
        let x3 = lfsr_generate(&taps3, &seed3, n);
        let z: Vec<bool> = (0..n).map(|i| if x2[i] { x1[i] } else { x3[i] }).collect();

        // Attack x1's register alone (2⁷ = 128 states): the correct initial state pops out at ¾ agreement.
        let a1 = correlation_attack(&z, &taps1).expect("x1 register attackable");
        assert_eq!(a1.init_state, seed1.to_vec(), "recovers x1's exact initial state");
        assert!((a1.agreement - 0.75).abs() < 0.05, "x1 leaks at ~¾ agreement, got {}", a1.agreement);
        assert!(a1.bias > 3.0 * spurious_bias_floor(7, n), "x1's correlation clears the spurious floor");
        // The recovered state is a re-checkable witness — regenerate it and it IS x1.
        assert_eq!(lfsr_generate(&taps1, &a1.init_state, n), x1, "the witness regenerates the x1 register");

        // Attack x3's register alone (2⁹ = 512 states) — independently of x1 and x2.
        let a3 = correlation_attack(&z, &taps3).expect("x3 register attackable");
        assert_eq!(a3.init_state, seed3.to_vec(), "recovers x3's exact initial state");
        assert!((a3.agreement - 0.75).abs() < 0.05, "x3 leaks at ~¾ agreement, got {}", a3.agreement);
        assert!(a3.bias > 3.0 * spurious_bias_floor(9, n), "x3's correlation clears the spurious floor");
        assert_eq!(lfsr_generate(&taps3, &a3.init_state, n), x3, "the witness regenerates the x3 register");

        // x2 is the CORRELATION-IMMUNE middle: its best state sits at the spurious floor, no real leak.
        let a2 = correlation_attack(&z, &taps2).expect("x2 register scanned");
        assert!(a2.bias < 3.0 * spurious_bias_floor(5, n), "x2 does not measurably leak, bias {}", a2.bias);
        assert!(a1.bias > 3.0 * a2.bias, "the leaking registers are starkly separated from the protected one");
    }

    #[test]
    fn walsh_spectrum_sees_the_linear_approximation_correlation_is_blind_to() {
        // f(x1,x2,x3,x4) = x1 ⊕ x2 ⊕ (x3 ∧ x4). It is FIRST-ORDER correlation-immune: every weight-1
        // Walsh coefficient vanishes, so Rung E's single-input attack sees Pr[z=xⱼ]=½ and finds nothing.
        // But the full Walsh spectrum reveals a WEIGHT-2 approximation z ≈ x1⊕x2 with bias ¼ — the
        // multi-register leak E structurally cannot express (it only reads weight-1 masks).
        let f: Vec<bool> = (0..16)
            .map(|x| {
                let b = |i: usize| (x >> i) & 1 == 1;
                b(0) ^ b(1) ^ (b(2) & b(3))
            })
            .collect();
        let spec = walsh_spectrum(&f).expect("well-formed truth table");
        for m in [1usize, 2, 4, 8] {
            assert_eq!(spec[m], 0, "weight-1 mask {m} vanishes — Rung E is blind here");
        }
        assert_eq!(correlation_immunity_order(&f), Some(1), "f is first-order correlation-immune");
        let (mask, bias) = best_linear_approximation(&f, true).expect("a best approximation exists");
        assert_eq!(bias, 0.25, "the exploitable approximation has bias ¼");
        assert!(mask.count_ones() >= 2, "and it lives at weight ≥ 2 — beyond E's single-register view");
        assert_eq!(nonlinearity(&f), Some(4), "nonlinearity 2³ − 8/2 = 4");

        // THE CEILING — a bent function g = x1x2 ⊕ x3x4 has a FLAT spectrum: every |Ŵ| = 2^{n/2} = 4, so
        // no linear approximation beats bias 1/8. Maximal nonlinearity, the linear-incompressible residue.
        let g: Vec<bool> = (0..16)
            .map(|x| {
                let b = |i: usize| (x >> i) & 1 == 1;
                (b(0) & b(1)) ^ (b(2) & b(3))
            })
            .collect();
        let spec_g = walsh_spectrum(&g).expect("well-formed");
        assert!(spec_g.iter().all(|&c| c.unsigned_abs() == 4), "bent ⇒ perfectly flat spectrum");
        assert_eq!(nonlinearity(&g), Some(6), "bent nonlinearity 2³ − 2¹ = 6 (maximal for n=4)");
        assert_eq!(best_linear_approximation(&g, true).unwrap().1, 0.125, "no approximation beats 1/8");
    }

    #[test]
    fn anf_is_its_own_inverse_and_reads_the_degree() {
        // The Möbius transform is an involution over GF(2): anf(anf(f)) == f, for every function.
        for n in 1..=5usize {
            for seed in 0..8u64 {
                let f: Vec<bool> = (0..1u64 << n)
                    .map(|i| {
                        let mut z = 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i.wrapping_add(seed * 131 + 1));
                        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                        (z ^ (z >> 27)) & 1 == 1
                    })
                    .collect();
                let a = anf(&f).expect("well-formed");
                assert_eq!(anf(&a).as_deref(), Some(&f[..]), "anf∘anf = identity (n={n}, seed={seed})");
            }
        }
        // Known ANFs: constant (degree 0), parity (degree 1), the inner-product bent (degree 2).
        let n = 4;
        let parity: Vec<bool> = (0..1usize << n).map(|x| (x as u32).count_ones() % 2 == 1).collect();
        assert_eq!(algebraic_degree(&parity), Some(1), "parity is affine");
        let bent: Vec<bool> = (0..1usize << n)
            .map(|x| {
                let b = |i: usize| x & (1 << i) != 0;
                (b(0) && b(1)) ^ (b(2) && b(3))
            })
            .collect();
        assert_eq!(algebraic_degree(&bent), Some(2), "x0x1 ⊕ x2x3 is quadratic");
        assert_eq!(algebraic_degree(&vec![true; 8]), Some(0), "a constant has degree 0");
    }

    #[test]
    fn autocorrelation_reads_the_derivative_symmetry() {
        let n = 4;
        // A linear function has EVERY direction as a linear structure: |r(a)| = 2ⁿ for all a.
        let parity: Vec<bool> = (0..1usize << n).map(|x| (x as u32).count_ones() % 2 == 1).collect();
        let rp = autocorrelation(&parity).expect("well-formed");
        assert_eq!(rp[0], 16, "r(0) = 2ⁿ always");
        assert!(rp.iter().all(|&c| c.unsigned_abs() == 16), "a linear function is flat: |r(a)| = 2ⁿ ∀a");

        // A bent function is PERFECTLY nonlinear: r(a) = 0 for every a ≠ 0 — no linear structure at all.
        let bent: Vec<bool> = (0..1usize << n)
            .map(|x| {
                let b = |i: usize| x & (1 << i) != 0;
                (b(0) && b(1)) ^ (b(2) && b(3))
            })
            .collect();
        let rb = autocorrelation(&bent).expect("well-formed");
        assert_eq!(rb[0], 16);
        assert!(rb[1..].iter().all(|&c| c == 0), "bent ⇒ no nonzero linear structure");

        // f(x) = g(x0,x1,x2) ⊕ x3 has the complement structure a = e3: flipping x3 always flips f.
        let f: Vec<bool> = (0..1usize << n)
            .map(|x| {
                let b = |i: usize| x & (1 << i) != 0;
                ((b(0) && b(1)) ^ b(2)) ^ b(3)
            })
            .collect();
        let rf = autocorrelation(&f).expect("well-formed");
        assert_eq!(rf[1 << 3], -16, "flipping x3 always flips f ⇒ r(e3) = −2ⁿ (a linear structure)");
    }

    #[test]
    fn walsh_found_approximation_breaks_a_correlation_immune_combiner() {
        // Four LFSRs feed the CI(1) combiner z = a1 ⊕ a2 ⊕ (a3 ∧ a4). Because it is first-order
        // correlation-immune, Rung E sees each single register at ½ — invisible. The Walsh-found weight-2
        // approximation z ≈ a1⊕a2 is the break: the COMBINED register leaks at ¾.
        let n = 4000;
        let taps1 = [false, false, true, false, false, false, true]; // L=7
        let taps2 = [false, false, true, false, true]; // L=5
        let taps3 = [false, false, false, false, true, false, false, false, true]; // L=9
        let taps4 = [false, false, false, false, false, false, false, false, true, false, true]; // L=11
        let a1 = lfsr_generate(&taps1, &[true, false, true, true, false, false, true], n);
        let a2 = lfsr_generate(&taps2, &[true, true, false, false, true], n);
        let a3 = lfsr_generate(&taps3, &[true, false, false, true, false, true, true, false, true], n);
        let a4 = lfsr_generate(&taps4, &[true, false, true, false, false, true, true, false, false, true, false], n);
        let z: Vec<bool> = (0..n).map(|i| a1[i] ^ a2[i] ^ (a3[i] & a4[i])).collect();

        // E's view — the keystream against each single register: flat, no leak (correlation-immune).
        let agree1 = z.iter().zip(&a1).filter(|(x, y)| x == y).count() as f64 / n as f64;
        assert!((agree1 - 0.5).abs() < 0.04, "single register a1 is invisible to first-order correlation, {agree1}");

        // F's view — the keystream against the COMBINED register a1⊕a2 (the weight-2 mask Walsh found).
        let combined: Vec<bool> = (0..n).map(|i| a1[i] ^ a2[i]).collect();
        let agree_r = z.iter().zip(&combined).filter(|(x, y)| x == y).count() as f64 / n as f64;
        assert!((agree_r - 0.75).abs() < 0.04, "the weight-2 approximation leaks at ¾ — E could not see it, {agree_r}");
    }

    #[test]
    fn fast_correlation_attack_decodes_a_noisy_lfsr_without_exhaustive_search() {
        // Primitive trinomial x¹⁷+x³+1: a[i] = a[i-14] ⊕ a[i-17], weight-3 parity checks — ideal for fast
        // correlation. Rung E would search 2¹⁷ states; the decoder recovers the state in polynomial time.
        let mut taps = vec![false; 17];
        taps[13] = true; // a[i-14]
        taps[16] = true; // a[i-17]
        let seed: Vec<bool> = (0..17).map(|k| (0xACE1u64 >> k) & 1 == 1).collect();
        let n = 4000;
        let a = lfsr_generate(&taps, &seed, n);

        // Transmit through a binary symmetric channel: flip ~12% of the bits.
        let mut st = 0x1234_5678_9abc_def0u64;
        let z: Vec<bool> = a
            .iter()
            .map(|&bit| {
                st = st.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                bit ^ ((st >> 40) % 100 < 12)
            })
            .collect();
        let agree = a.iter().zip(&z).filter(|(x, y)| x == y).count() as f64 / n as f64;
        assert!((agree - 0.88).abs() < 0.04, "the channel is genuinely ~12% noisy, got {agree}");

        let state = fast_correlation_attack(&z, &taps, 400).expect("the decoder recovers the register");
        assert_eq!(state, seed, "fast correlation recovers the exact initial state — no 2¹⁷ search");

        // Ceiling: pure noise (no correlation) decodes to nothing significant.
        let mut st2 = 0xdead_beef_0bad_c0deu64;
        let pure: Vec<bool> = (0..n)
            .map(|_| {
                st2 = st2.wrapping_mul(6364136223846793005).wrapping_add(1);
                (st2 >> 40) & 1 == 1
            })
            .collect();
        assert!(fast_correlation_attack(&pure, &taps, 400).is_none(), "pure noise leaks no register — the ceiling");
    }

    #[test]
    fn shrinking_generator_falls_to_a_clock_guess_the_linear_rungs_cannot_touch() {
        // Clock register A (L=7) gates data register S (L=9). The output is a data-dependent decimation
        // of S — not a fixed function of any register's own past — so its linear complexity is enormous
        // and every feedback/correlation/algebraic/linear rung is blind. The attack guesses A's 2⁷ states
        // (divide-and-conquer on the CLOCK alone) and linear-solves S.
        let a_taps = [false, false, true, false, false, false, true]; // L=7
        let s_taps = [false, false, false, false, true, false, false, false, true]; // L=9
        let a_seed = [true, false, true, true, false, false, true];
        let s_seed = [true, true, false, true, false, true, true, false, true];
        let m = 300;
        let output = shrinking_generator(&a_taps, &a_seed, &s_taps, &s_seed, m);

        // The linear rung is blind: the output's linear complexity is far above the data register order.
        assert!(
            berlekamp_massey_gf2(&output).0 > 40,
            "the shrinking generator's output has high linear complexity, got {}",
            berlekamp_massey_gf2(&output).0
        );

        let (a_rec, s_rec) = attack_shrinking_generator(&output, &a_taps, &s_taps).expect("the generator falls");
        // The shrinking generator has key-equivalences (distinct register states yielding the same
        // keystream), so the break is keystream-equivalence, not literal state equality: the recovered
        // pair reproduces the output — and keeps reproducing it far past the 300 bits the attack used
        // (1000 bits), proving a full recovery of the generator, not a short coincidence.
        assert_eq!(
            shrinking_generator(&a_taps, &a_rec, &s_taps, &s_rec, m),
            output,
            "the recovered pair regenerates the keystream — the certified break"
        );
        assert_eq!(
            shrinking_generator(&a_taps, &a_rec, &s_taps, &s_rec, 1000),
            shrinking_generator(&a_taps, &a_seed, &s_taps, &s_seed, 1000),
            "and stays identical to the true generator well past the attacked length — a full break"
        );
    }

    #[test]
    fn xl_solves_a_quadratic_system_by_degree_escalation() {
        // A quadratic system over GF(2) in x0..x3 with the unique solution [1,0,1,0]. Each equation is an
        // XOR of monomial bitmasks that must be 0 (bit i = variable i; mask 0 = the constant 1).
        let eqs: Vec<Vec<u64>> = vec![
            vec![0b0011],           // x0·x1 = 0
            vec![0b0101, 0b0000],   // x0·x2 = 1
            vec![0b0110],           // x1·x2 = 0
            vec![0b1100],           // x2·x3 = 0
            vec![0b0001, 0b0100],   // x0 ⊕ x2 = 0
            vec![0b0010, 0b1000],   // x1 ⊕ x3 = 0
        ];
        // At operating degree 1 the quadratic constraints cannot even be represented — only the two
        // linear relations survive, and the system is underdetermined.
        assert_eq!(
            solve_polynomial_system_gf2(&eqs, 4, 1),
            PolySolveResult::Undetermined,
            "degree 1 cannot represent the quadratic constraints"
        );
        // XL at degree 2 multiplies the equations, linking the products back to the variables, and the
        // unique root falls out — verified against the original nonlinear system.
        assert_eq!(
            solve_polynomial_system_gf2(&eqs, 4, 2),
            PolySolveResult::Solved(vec![true, false, true, false]),
            "XL recovers the unique root"
        );
    }

    #[test]
    fn xl_refutes_an_unsatisfiable_system_linearization_misses() {
        // x0·x1 = 1 and x0 = 0 is contradictory (x0=0 forces x0·x1=0≠1), but a degree-1 linearization
        // cannot even represent x0·x1. XL multiplies x0=0 by x1 to derive x0·x1=0, colliding with
        // x0·x1=1 → 0=1, a certified refutation.
        // Over 2 variables (x0 = bit 0, x1 = bit 1), x0·x1 has mask 0b11.
        let eqs: Vec<Vec<u64>> = vec![
            vec![0b11, 0b00], // x0·x1 ⊕ 1 = 0
            vec![0b01],       // x0 = 0
        ];
        assert_eq!(
            solve_polynomial_system_gf2(&eqs, 2, 1),
            PolySolveResult::Undetermined,
            "at degree 1 the quadratic contradiction is invisible"
        );
        assert_eq!(
            solve_polynomial_system_gf2(&eqs, 2, 2),
            PolySolveResult::Refuted,
            "XL manufactures x0·x1 = 0, refuting the system"
        );
    }

    #[test]
    fn algebraic_immunity_computes_and_verifies_known_values() {
        let b = |x: usize, i: usize| (x >> i) & 1 == 1;

        // An affine function has algebraic immunity 1: g = C ⊕ 1 annihilates it (degree 1).
        let affine: Vec<bool> = (0..8).map(|x| b(x, 0) ^ b(x, 1) ^ b(x, 2)).collect();
        let (ai, w) = algebraic_immunity(&affine).expect("well-formed");
        assert_eq!(ai, 1, "affine functions have AI 1");
        assert!(verify_annihilator(&affine, &w), "the affine annihilator re-checks");

        // Majority-of-3 has algebraic immunity 2 — no affine annihilator exists on either side.
        let maj3: Vec<bool> = (0..8).map(|x| (x as u32).count_ones() >= 2).collect();
        let (ai, w) = algebraic_immunity(&maj3).expect("well-formed");
        assert_eq!(ai, 2, "majority-3 has AI 2 (the maximum for n=3)");
        assert!(verify_annihilator(&maj3, &w), "the degree-2 annihilator re-checks");

        // A product term x1∧x2∧x3 has AI 1 (x1 ⊕ 1 vanishes on its lone support point 111).
        let and3: Vec<bool> = (0..8).map(|x| b(x, 0) & b(x, 1) & b(x, 2)).collect();
        assert_eq!(algebraic_immunity(&and3).unwrap().0, 1, "a single product term has AI 1");

        // Tamper cases: an all-zero witness is not an annihilator, and flipping the side breaks vanishing.
        let (_, w) = algebraic_immunity(&maj3).unwrap();
        let mut zeroed = w.clone();
        zeroed.coeffs = vec![false; zeroed.coeffs.len()];
        assert!(!verify_annihilator(&maj3, &zeroed), "the zero function is not a valid annihilator");
        let mut flipped = w.clone();
        flipped.annihilates_complement = !flipped.annihilates_complement;
        assert!(!verify_annihilator(&maj3, &flipped), "a nonzero g cannot vanish on both sides");
    }

    #[test]
    fn algebraic_immunity_never_exceeds_half_n() {
        // AI(C) ≤ ⌈n/2⌉ for every Boolean function — a theorem; re-checked here on pseudo-random tables.
        let mut st = 0x00C0_FFEE_1234_5678u64;
        for _ in 0..24 {
            let truth: Vec<bool> = (0..16)
                .map(|_| {
                    st = st.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    (st >> 33) & 1 == 1
                })
                .collect();
            let (ai, w) = algebraic_immunity(&truth).expect("well-formed");
            assert!(ai <= 2, "AI ≤ ⌈4/2⌉ = 2, got {ai}");
            assert!(verify_annihilator(&truth, &w), "every witness re-checks");
        }
    }

    #[test]
    fn algebraic_filter_attack_recovers_the_filter_generator_state() {
        // A filter generator: a length-10 maximal LFSR (x¹⁰+x³+1, period 1023) filtered by majority-of-3
        // over three consecutive state bits. Correlation/Walsh see only statistical leaks; the algebraic
        // attack is exact — maj3 has AI 2, so each keystream bit is a degree-2 equation in the initial
        // state, and linearizing over the 56 monomials recovers all 10 secret bits at once.
        let taps = [false, false, false, false, false, false, true, false, false, true]; // s[i]=s[i-7]⊕s[i-10]
        let s0 = [true, false, true, true, false, false, true, false, true, true];
        let filter_truth: Vec<bool> = (0..8).map(|x| (x as u32).count_ones() >= 2).collect(); // maj3
        let m = 3;
        let n = 400;
        let seq = lfsr_generate(&taps, &s0, n + m);
        let keystream: Vec<bool> = (0..n)
            .map(|t| {
                let idx = (0..m).fold(0usize, |a, i| a | (usize::from(seq[t + i]) << i));
                filter_truth[idx]
            })
            .collect();

        let recovered = algebraic_filter_attack(&keystream, &taps, &filter_truth)
            .expect("the algebraic attack recovers the state");
        assert_eq!(recovered, s0.to_vec(), "the recovered initial state IS the secret key");
    }

    #[test]
    fn periodic_round_trips() {
        let block = [4i64, 1, 1, 5, 9, 2, 6];
        let v: Vec<i64> = (0..300).map(|i| block[i % block.len()]).collect();
        roundtrips(&v);
    }

    #[test]
    fn modular_affine_round_trips() {
        let v: Vec<i64> = (0..200).map(|i| 100 + 3 * ((i % 8) as i64)).collect();
        roundtrips(&v);
    }

    #[test]
    fn sparse_round_trips() {
        let mut v = vec![42i64; 500];
        v[17] = -1;
        v[300] = 999;
        v[499] = 7;
        roundtrips(&v);
    }

    #[test]
    fn delta_and_dod_shapes_round_trip() {
        let monotone: Vec<i64> = (0..200).map(|i| i * i / 7 + i).collect();
        roundtrips(&monotone);
        let jittered: Vec<i64> = (0..200).map(|i| 1_000_000 + 60 * i + (i % 3)).collect();
        roundtrips(&jittered);
    }

    #[test]
    fn runs_and_low_cardinality_round_trip() {
        let mut runs = vec![5i64; 100];
        runs.extend(std::iter::repeat(9).take(80));
        runs.extend(std::iter::repeat(-2).take(120));
        roundtrips(&runs);
        let categorical: Vec<i64> = (0..600).map(|i| [10, 20, 30][i % 3]).collect();
        roundtrips(&categorical);
    }

    #[test]
    fn byte_column_round_trips() {
        let v: Vec<i64> = (0..500).map(|i| ((i * 37) % 256) as i64).collect();
        roundtrips(&v);
    }

    #[test]
    fn edge_cases_round_trip() {
        roundtrips(&[]);
        roundtrips(&[42]);
        roundtrips(&[-1]);
        roundtrips(&[i64::MIN, i64::MAX, 0, -1, 1]);
        roundtrips(&[7, 7, 7, 7]);
    }

    #[test]
    fn pseudorandom_round_trips_and_is_never_larger_than_varint() {
        // A pseeded pseudo-random column has no generator structure; the varint baseline must win
        // and the round-trip must still be exact.
        let mut state = 0x1234_5678_9abc_def0u64;
        let v: Vec<i64> = (0..400)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 1) as i64
            })
            .collect();
        roundtrips(&v);
        let mut baseline = vec![T_INTS];
        leb128_encode(&mut baseline, v.iter().copied(), v.len());
        assert!(describe_int_seq(&v).len() <= baseline.len(), "never worse than the varint baseline");
    }
}
