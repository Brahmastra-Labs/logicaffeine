//! # Integer-sequence description-length codec (the MDL primitive)
//!
//! A pure, self-contained engine that describes an `&[i64]` by the **shortest** program in a fixed
//! menu of closed-form generators — affine (`base + i·stride`), geometric (`base · ratioⁱ`), a
//! degree-≤4 polynomial (finite-difference seeds), a periodic block, a sparse dominant-value form, a
//! sandboxed [`GenExpr`] generator, and the columnar fallbacks (delta, delta-of-delta,
//! frame-of-reference bit-pack, run-length, dictionary, raw byte column, plain zig-zag varint), a
//! common-multiple factoring that describes the quotients instead, and an arithmetic-coded form whose
//! context-mixing model pays `−log₂ p` bits for what the closed forms can only reject. Each candidate
//! is a complete self-delimiting byte string; [`consider`] keeps the smallest, so the result is
//! **never larger than plain varint**. A float column whose every value is a decimal `k / 10^e`
//! describes as that integer column ([`emit_decimal_float_column`]).
//!
//! [`describe_int_seq`] is a *computable upper bound on the Kolmogorov complexity* of the sequence
//! over this description language, and [`decode_int_seq`] is its exact inverse — decode reproduces
//! the sequence bit-for-bit, so the encoded bytes are a **re-checkable witness** for that bound.
//!
//! This is the same codec the wire layer (`logicaffeine_compile`'s `marshal`) uses for its
//! `WireStructure::Auto` int columns; it lives here in the leaf crate so both the wire codec and the
//! proof layer share one implementation of the format. The crate has no I/O and no receiver policy,
//! so the DoS bound (`max_elements`) is a **parameter**, supplied by each caller.

use crate::entropy::{
    cost, hash2, hash3, stretch, AdaptiveProb, Apm, BitIo, Decoder, Encoder, HashedProbs, Mixer, ProbBuckets,
};

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
pub const T_INTS_MULT: u8 = 90; // count + base + common factor g + the quotient column (v = base + g·q)
pub const T_INTS_CM: u8 = 91; // count + predictor + arithmetic-coded residuals under a context-mixing model
pub const T_FLOATS_DECIMAL: u8 = 92; // count + exponent e + raw exceptions + the integer column k (v = k / 10^e)
pub const T_FLOATS_BITS: u8 = 93; // the IEEE-754 bit patterns as an integer column (v = from_bits(k))
pub const T_INTS_CXOR: u8 = 94; // count + split + high column + low column XOR'd against the last value sharing its high bits

/// Every column tag this module writes, by name: the `describe` half of the shared wire tag namespace, so
/// a collision check covers each tag without a hand-kept list.
pub const COLUMN_TAGS: &[(&str, u8)] = &[
    ("T_INTS", T_INTS),
    ("T_INTS_AFFINE", T_INTS_AFFINE),
    ("T_INTS_DELTA", T_INTS_DELTA),
    ("T_INTS_DOD", T_INTS_DOD),
    ("T_INTS_FOR", T_INTS_FOR),
    ("T_INTS_RLE", T_INTS_RLE),
    ("T_INTS_DICT", T_INTS_DICT),
    ("T_INTS_POLY", T_INTS_POLY),
    ("T_GEN", T_GEN),
    ("T_BYTES", T_BYTES),
    ("T_INTS_GEOMETRIC", T_INTS_GEOMETRIC),
    ("T_INTS_PERIODIC", T_INTS_PERIODIC),
    ("T_INTS_SPARSE", T_INTS_SPARSE),
    ("T_INTS_LRECUR", T_INTS_LRECUR),
    ("T_INTS_LFSR", T_INTS_LFSR),
    ("T_INTS_FCSR", T_INTS_FCSR),
    ("T_INTS_MULT", T_INTS_MULT),
    ("T_INTS_CM", T_INTS_CM),
    ("T_FLOATS_DECIMAL", T_FLOATS_DECIMAL),
    ("T_FLOATS_BITS", T_FLOATS_BITS),
    ("T_INTS_CXOR", T_INTS_CXOR),
];

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
    Select {
        op: GenCmp,
        lhs: Box<GenExpr>,
        rhs: Box<GenExpr>,
        then: Box<GenExpr>,
        els: Box<GenExpr>,
    },
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
            if d == 0 {
                0
            } else {
                gen_eval(a, i).wrapping_div(d)
            }
        }
        GenExpr::Mod(a, b) => {
            let d = gen_eval(b, i);
            if d == 0 {
                0
            } else {
                gen_eval(a, i).wrapping_rem(d)
            }
        }
        GenExpr::Select {
            op,
            lhs,
            rhs,
            then,
            els,
        } => {
            let (l, r) = (gen_eval(lhs, i), gen_eval(rhs, i));
            let c = match op {
                GenCmp::Eq => l == r,
                GenCmp::Ne => l != r,
                GenCmp::Lt => l < r,
                GenCmp::Le => l <= r,
                GenCmp::Gt => l > r,
                GenCmp::Ge => l >= r,
            };
            if c {
                gen_eval(then, i)
            } else {
                gen_eval(els, i)
            }
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
        GenExpr::Add(a, b) => {
            out.push(2);
            serialize_gen(a, out);
            serialize_gen(b, out);
        }
        GenExpr::Sub(a, b) => {
            out.push(3);
            serialize_gen(a, out);
            serialize_gen(b, out);
        }
        GenExpr::Mul(a, b) => {
            out.push(4);
            serialize_gen(a, out);
            serialize_gen(b, out);
        }
        GenExpr::Div(a, b) => {
            out.push(5);
            serialize_gen(a, out);
            serialize_gen(b, out);
        }
        GenExpr::Mod(a, b) => {
            out.push(6);
            serialize_gen(a, out);
            serialize_gen(b, out);
        }
        GenExpr::Select {
            op,
            lhs,
            rhs,
            then,
            els,
        } => {
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
pub fn deserialize_gen(
    buf: &[u8],
    pos: &mut usize,
    budget: &mut u32,
    depth: u32,
) -> Option<GenExpr> {
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
            GenExpr::Select {
                op,
                lhs,
                rhs,
                then,
                els,
            }
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
    let exceptions: Vec<(usize, i64)> = v
        .iter()
        .enumerate()
        .filter(|(_, &x)| x != cand)
        .map(|(i, &x)| (i, x))
        .collect();
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
        2 => m[0][0]
            .checked_mul(m[1][1])?
            .checked_sub(m[0][1].checked_mul(m[1][0])?),
        _ => {
            let mut acc: i128 = 0;
            for j in 0..k {
                let minor: Vec<Vec<i128>> = (1..k)
                    .map(|r| (0..k).filter(|&c| c != j).map(|c| m[r][c]).collect())
                    .collect();
                let cof = m[0][j].checked_mul(det_i128(&minor)?)?;
                acc = if j % 2 == 0 {
                    acc.checked_add(cof)?
                } else {
                    acc.checked_sub(cof)?
                };
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
    fn zero() -> Self {
        Gf2(false)
    }
    fn one() -> Self {
        Gf2(true)
    }
    fn add(self, o: Self) -> Self {
        Gf2(self.0 ^ o.0)
    }
    fn sub(self, o: Self) -> Self {
        Gf2(self.0 ^ o.0)
    }
    fn mul(self, o: Self) -> Self {
        Gf2(self.0 && o.0)
    }
    fn inv(self) -> Self {
        self
    } // 1⁻¹ = 1 (the only nonzero element)
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
    fn zero() -> Self {
        Gf256(0)
    }
    fn one() -> Self {
        Gf256(1)
    }
    fn add(self, o: Self) -> Self {
        Gf256(self.0 ^ o.0)
    }
    fn sub(self, o: Self) -> Self {
        Gf256(self.0 ^ o.0)
    }
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
    let taps = if l == 0 {
        Vec::new()
    } else {
        c[1..=l].to_vec()
    };
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
    lfsr_generate_field(&t, &sd, total)
        .into_iter()
        .map(|g| g.0)
        .collect()
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
fn bigint_gcd_local(
    a: &crate::numeric::BigInt,
    b: &crate::numeric::BigInt,
) -> crate::numeric::BigInt {
    let (mut a, mut b) = (a.abs(), b.abs());
    while !b.is_zero() {
        let r = a
            .div_rem(&b)
            .map(|(_, r)| r)
            .unwrap_or_else(crate::numeric::BigInt::zero);
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
pub fn two_adic_reconstruct(
    bits: &[bool],
) -> Option<(crate::numeric::BigInt, crate::numeric::BigInt)> {
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
pub fn fcsr_generate(
    p: &crate::numeric::BigInt,
    q: &crate::numeric::BigInt,
    n: usize,
) -> Vec<bool> {
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
        r = r
            .div_rem(&two)
            .map(|(quot, _)| quot)
            .unwrap_or_else(BigInt::zero);
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
    if mat
        .iter()
        .any(|(coeff, rhs)| *rhs && coeff.iter().all(|&w| w == 0))
    {
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
/// coefficient vector over `monomials(l, d)` such that replaying `s[i] = ⊕ₘ cₘ · monomialₘ(window)`
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
/// `monomials(l, d)` and an `l`-bit seed: `s[i] = ⊕ₘ coeffsₘ · monomialₘ(s[i-1],…,s[i-l])`. The
/// inverse of [`detect_algebraic_recurrence`].
pub fn algebraic_generate(
    l: usize,
    d: usize,
    coeffs: &[bool],
    seed: &[bool],
    total: usize,
) -> Vec<bool> {
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
    debug_assert!(
        n.is_power_of_two(),
        "Walsh–Hadamard length must be a power of two"
    );
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
    Some(
        a.iter()
            .enumerate()
            .filter(|(_, &c)| c)
            .map(|(m, _)| (m as u64).count_ones() as usize)
            .max()
            .unwrap_or(0),
    )
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

/// A re-checkable annihilator witness: `coeffs` is the ANF (over `monomials(n_vars, degree)`) of a
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
            return Some((
                d,
                AnnihilatorWitness {
                    n_vars: n,
                    degree: d,
                    coeffs: g,
                    annihilates_complement: false,
                },
            ));
        }
        if let Some(g) = annihilator(n, d, &zeros) {
            return Some((
                d,
                AnnihilatorWitness {
                    n_vars: n,
                    degree: d,
                    coeffs: g,
                    annihilates_complement: true,
                },
            ));
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
        let must_vanish = if w.annihilates_complement {
            !truth[x]
        } else {
            truth[x]
        };
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
pub fn algebraic_filter_attack(
    keystream: &[bool],
    taps: &[bool],
    filter_truth: &[bool],
) -> Option<Vec<bool>> {
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
    let support: Vec<usize> = (0..filter_truth.len())
        .filter(|&x| filter_truth[x])
        .collect();
    let zeros: Vec<usize> = (0..filter_truth.len())
        .filter(|&x| !filter_truth[x])
        .collect();
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
        let anns = if keystream[t] {
            &ann_when_one
        } else {
            &ann_when_zero
        };
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
pub fn fast_correlation_attack(
    keystream: &[bool],
    taps: &[bool],
    max_iters: usize,
) -> Option<Vec<bool>> {
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
    monomials(n, deg)
        .iter()
        .map(|mono| mono.iter().fold(0u64, |b, &v| b | (1u64 << v)))
        .collect()
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
    eqs.iter()
        .all(|eq| !eq.iter().fold(false, |a, &m| a ^ eval_mask(m, x)))
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
pub fn solve_polynomial_system_gf2(
    eqs: &[Vec<u64>],
    n_vars: usize,
    max_degree: usize,
) -> PolySolveResult {
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
            let deg_eq = eq
                .iter()
                .map(|m| m.count_ones() as usize)
                .max()
                .unwrap_or(0);
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
                    Box::new(GenExpr::Mod(
                        Box::new(GenExpr::Index),
                        Box::new(GenExpr::Const(p as i64)),
                    )),
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
    let width = if range == 0 {
        0
    } else {
        (64 - range.leading_zeros()) as u8
    };
    out.push(width);
    if width > 0 {
        let residuals: Vec<u64> = v
            .iter()
            .map(|&x| (x as u64).wrapping_sub(min as u64))
            .collect();
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
    let iw = if dict.len() <= 1 {
        0
    } else {
        (64 - ((dict.len() - 1) as u64).leading_zeros()) as u8
    };
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

// ---- How much of the menu a description searches ------------------------------------------

/// How much of the menu a description searches: the closed forms, packings and factorings, or those
/// plus the entropy-coded form. The entropy-coded form is the smallest on most real data and costs about
/// thirty model decisions an element to encode and to decode, so a caller on a latency-bound path keeps
/// to the structural menu and a caller spending CPU for size asks for the full one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    Structural,
    Full,
}

thread_local! {
    static MENU: std::cell::Cell<Menu> = const { std::cell::Cell::new(Menu::Structural) };
}

/// The menu descriptions on this thread search (structural unless a caller asked for more).
pub fn menu() -> Menu {
    MENU.with(std::cell::Cell::get)
}

/// Run `f` with descriptions searching `menu`, restoring the previous menu afterwards — on unwind too.
pub fn with_menu<T>(menu: Menu, f: impl FnOnce() -> T) -> T {
    struct Restore(Menu);
    impl Drop for Restore {
        fn drop(&mut self) {
            MENU.with(|m| m.set(self.0));
        }
    }
    let _restore = Restore(MENU.with(|m| m.replace(menu)));
    f()
}

// ---- Conditional XOR ----------------------------------------------------------------------
//
// A column of 64-bit patterns splits at `split` high bits. The high column is described as itself; each
// value's low bits are XOR'd against the last value whose high bits were the same, or kept as they are
// when no earlier value shared them. A value recurring under its prefix leaves a zero low part, a nearby
// value only the bits where it differs — full-precision coordinates, identifiers and hashes with shared
// prefixes become two easy columns. The decoder reads the high column first, so it knows each value's
// reference before it needs it.

/// Below this many values the two child columns' headers outweigh what the split can find.
const CXOR_MIN_ELEMENTS: usize = 256;
/// The split is offered only when the cost estimate predicts at least this fraction saved (percent).
const CXOR_MIN_GAIN: u64 = 5;
/// …and only for wide patterns: splitting at 20–44 high bits means nothing for values that never reach
/// them (an average bit length below this).
const CXOR_MIN_WIDTH: u64 = 40;
/// The splits scored exactly, and the prefix they are scored on: the estimate gates the transform, but
/// it prices the high column at its order-0 entropy while the entropy-coded form does far better on a
/// recurring prefix, so the split itself is chosen by real encoded size (a contiguous prefix keeps the
/// locality the coder lives on, and picks the same split as the whole column).
const CXOR_SPLITS: [u32; 7] = [20, 24, 28, 32, 36, 40, 44];
const CXOR_SAMPLE: usize = 8192;

thread_local! {
    /// Whether the column being described is a conditional-XOR child, which does not split again.
    static IN_CXOR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A fast, deterministic hasher for the prefix tables (keys are already well mixed by the split).
#[derive(Default)]
struct PrefixHasher(u64);

impl std::hash::Hasher for PrefixHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(b)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }
    fn write_u64(&mut self, x: u64) {
        self.0 = (self.0 ^ x).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29);
    }
}

type PrefixMap<V> = std::collections::HashMap<u64, V, std::hash::BuildHasherDefault<PrefixHasher>>;

fn cxor_mask(split: u32) -> u64 {
    (1u64 << (64 - split)) - 1
}

/// The high and low columns of the split at `split` high bits (`1..=63`).
fn cxor_split_columns(bits: &[u64], split: u32) -> (Vec<i64>, Vec<i64>) {
    let (shift, mask) = (64 - split, cxor_mask(split));
    let mut last: PrefixMap<u64> = PrefixMap::default();
    let mut hi = Vec::with_capacity(bits.len());
    let mut lo = Vec::with_capacity(bits.len());
    for &x in bits {
        let h = x >> shift;
        let low = match last.insert(h, x) {
            Some(y) => (x ^ y) & mask,
            None => x & mask,
        };
        hi.push(h as i64);
        lo.push(low as i64);
    }
    (hi, lo)
}

/// The cheapest estimated split description in bits, next to the unsplit column's: the high column at
/// its order-0 entropy plus every low part at its bit length, minimized over splits `8..=56` in steps of
/// four. An encoder-side gate only — the decoder reads the split.
fn cxor_estimate(bits: &[u64]) -> (u64, u64) {
    let n = bits.len() as f64;
    let plain: u64 = bits.iter().map(|&x| u64::from(65 - x.leading_zeros())).sum();
    let mut best = u64::MAX;
    for split in (8..=56).step_by(4) {
        let (shift, mask) = (64 - split, cxor_mask(split));
        let mut last: PrefixMap<u64> = PrefixMap::default();
        let mut counts: PrefixMap<u32> = PrefixMap::default();
        let mut low_bits = 0u64;
        for &x in bits {
            let h = x >> shift;
            *counts.entry(h).or_insert(0) += 1;
            let low = last.insert(h, x).map_or(x & mask, |y| (x ^ y) & mask);
            low_bits += u64::from(65 - low.leading_zeros());
        }
        let entropy: f64 = counts.values().map(|&c| f64::from(c) * (n / f64::from(c)).log2()).sum();
        best = best.min(entropy as u64 + low_bits);
    }
    (best, plain)
}

/// The conditional-XOR form of a column of bit patterns at `split` high bits (tag included); each child
/// column goes through the full menu, without splitting again.
pub fn conditional_xor_encode(bits: &[u64], split: u32) -> Vec<u8> {
    let (hi, lo) = cxor_split_columns(bits, split);
    let mut out = vec![T_INTS_CXOR];
    write_uvarint(bits.len() as u64, &mut out);
    out.push(split as u8);
    let prev = IN_CXOR.with(|c| c.replace(true));
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            IN_CXOR.with(|c| c.set(self.0));
        }
    }
    let _restore = Restore(prev);
    emit_best_int_column(&hi, &mut out);
    emit_best_int_column(&lo, &mut out);
    out
}

fn cxor_join(hi: &[i64], lo: &[i64], split: u32) -> Option<Vec<i64>> {
    let (shift, mask) = (64 - split, cxor_mask(split));
    let mut last: PrefixMap<u64> = PrefixMap::default();
    let mut out = Vec::with_capacity(hi.len().min(PREALLOC_CAP));
    for (&h, &l) in hi.iter().zip(lo) {
        let (h, l) = (h as u64, l as u64);
        if h >> split != 0 || l & !mask != 0 {
            return None;
        }
        let x = (h << shift) | last.get(&h).map_or(l, |&y| (l ^ y) & mask);
        last.insert(h, x);
        out.push(x as i64);
    }
    Some(out)
}

// ---- Common-multiple factoring ------------------------------------------------------------

/// The column minimum and the gcd of every value's distance from it, when that gcd exceeds one: then
/// `v = base + g·q` with every quotient `q` exact. Distances are taken in `u64` (wrapping), so a column
/// spanning the whole `i64` range factors exactly.
pub fn detect_common_multiple(v: &[i64]) -> Option<(i64, u64)> {
    if v.len() < 2 {
        return None;
    }
    let base = *v.iter().min()?;
    let mut g = 0u64;
    for &x in v {
        g = gcd_u64(g, (x as u64).wrapping_sub(base as u64));
        if g == 1 {
            return None;
        }
    }
    (g > 1).then_some((base, g))
}

fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

// ---- The entropy-coded column ----------------------------------------------------------------
//
// Each value is first offered to three whole-value predictors — what followed the last two values the
// last time they occurred, what followed the last value, and the last value itself — and one learned
// decision per distinct guess says whether it is right. A value none of them guessed becomes a residual
// against a predictor (the value itself, the previous value, or the line through the previous two),
// zig-zagged to `u`, and `u` is coded as its bit length `k` (seven binary decisions down a tree) and then
// its bits below the leading one, most significant first. Every decision is predicted by adaptive cells
// under several contexts — the tree node, the last two lengths, the last value, the last two values, the
// mantissa prefix, and the whole prefix so far (a trie of every value already seen, so a recurring value
// costs what its frequency says) — mixed by a gated logistic mixer and recalibrated by an APM.

/// Work units an entropy-coded element charges against the decode budget: each costs about thirty
/// model decisions, so a small payload cannot demand a long decode.
pub const CM_WORK_PER_ELEMENT: u64 = 8;
/// The default receiver's element budget: the entropy-coded form is offered only for columns that a
/// default receiver will accept.
const CM_OFFER_BUDGET: u64 = 1 << 24;
/// Below this many values the model has nothing to learn from and the plain forms win.
const CM_MIN_ELEMENTS: usize = 32;
/// Residual predictors: 0 = the value itself, 1 = the previous value, 2 = the line through the previous
/// two. Modes `CM_LEAN + p` code the same residual under the lean model: one adaptive cell per decision
/// (the bit-length tree node, the whole mantissa prefix), no mixing — the adaptive order-0 distribution of
/// the residual, which is what an independent, identically distributed column needs and nothing more.
const CM_LEAN: u8 = 4;
const CM_MODES: [u8; 6] = [0, 1, 2, CM_LEAN, CM_LEAN + 1, CM_LEAN + 2];
/// The lean model's cells settle at this rate: the stationary estimate an i.i.d. column wants.
const CM_LIMIT_LEAN: u32 = 1023;
/// Mantissa bits this far below the leading one keep two predictions (their position, and the whole
/// prefix); above it they get the full context set.
const CM_DEEP: u32 = 20;
/// How many leading mantissa bits the short-prefix contexts distinguish.
const CM_PREFIX_DEPTH: u32 = 12;
const CM_INPUTS: usize = 7;
/// Mixer weight sets: one per bit-length tree node; per mantissa position band × length band ×
/// whole-prefix confidence; per deep length band × confidence; per guess predictor × record.
const CM_K_SETS: usize = 128;
const CM_DEEP_SETS: usize = CM_K_SETS + 16 * 5 * 4;
const CM_FLAG_SETS: usize = CM_DEEP_SETS + 5 * 4;
const CM_RECENT_SETS: usize = CM_FLAG_SETS + CM_PREDICTORS * 16;
const CM_RANK_SETS: usize = CM_RECENT_SETS + 16;
const CM_SIGN_SETS: usize = CM_RANK_SETS + CM_RECENT;
const CM_MIX_SETS: usize = CM_SIGN_SETS + 65;
const CM_K_APM: usize = 128 * 17;
const CM_DEEP_APM: usize = CM_K_APM + 65 * 16;
const CM_FLAG_APM: usize = CM_DEEP_APM + 65 * 4;
const CM_RECENT_APM: usize = CM_FLAG_APM + CM_PREDICTORS * 16 * 2;
const CM_RANK_APM: usize = CM_RECENT_APM + 16;
const CM_SIGN_APM: usize = CM_RANK_APM + CM_RECENT * 9;
const CM_APM_CONTEXTS: usize = CM_SIGN_APM + 65 * 2;
const CM_LIMIT_DIRECT: u32 = 255;
const CM_LIMIT_HASHED: u32 = 127;
/// Whole-value predictors: order-2 follower, order-1 follower, previous value.
const CM_PREDICTORS: usize = 3;
/// How many distinct recent values the move-to-front cache holds (a rank is one byte).
const CM_RECENT: usize = 256;
/// The encoder takes the recency path only when the residual is this many bits longer than the rank.
const CM_RECENT_MARGIN: u32 = 10;
/// A whole-value predictor (or the recency cache) spends a decision only while its running hit rate is
/// above this (12-bit). A rejected guess keeps its probability inside the residual coding — the coder
/// cannot exclude it — so on data where a predictor is rarely right every decision it spends is overhead.
const CM_GATE: u16 = 600;

/// Offsets of the two directly indexed bit-length tables inside [`IntCm::direct`]: by tree node, and by
/// node and the last length.
const CM_K0: usize = 0;
const CM_K1: usize = CM_K0 + 128;
const CM_SIGN: usize = CM_K1 + 65 * 128;
const CM_DIRECT: usize = CM_SIGN + 65 * 2;

/// Where one mixer input's prediction comes from: a directly indexed cell, a cell of a bucket (the
/// bit-length tree and the mantissa, whose contexts are fixed for a group of decisions), a cell the
/// misc table hashes per decision (the few decisions a value makes outside those walks), or nothing.
#[derive(Clone, Copy)]
enum Cell {
    Direct(usize),
    Bucketed(usize),
    Hashed(u64),
    Off,
}

struct IntCm {
    direct: Vec<AdaptiveProb>,
    buckets: ProbBuckets,
    misc: HashedProbs,
    order2: Vec<(u32, i64)>,
    order1: Vec<(u32, i64)>,
    match_mask: usize,
    runs: [u32; CM_PREDICTORS],
    hit_rate: [AdaptiveProb; CM_PREDICTORS + 1],
    recent: Vec<i64>,
    last_rank: u32,
    mixer: Mixer,
    apm: Apm,
    v1: i64,
    v2: i64,
    kh1: u32,
    kh2: u32,
    /// Whether the last nonzero residual was negative.
    negative: bool,
    lean: bool,
    /// Per (bit length, mantissa position): what the lean model's adaptive cells have cost there, and
    /// what coding those bits flat (one bit each) would have cost. Where the cells have not paid for
    /// themselves — deep positions whose prefixes recur too rarely to learn — the bits go flat.
    spent: Vec<(u32, u32)>,
}

impl IntCm {
    fn new(n: usize, lean: bool) -> Self {
        let lg = usize::BITS - n.max(1).leading_zeros();
        let bucket_bits = (lg + 2).clamp(12, 18);
        let misc_bits = (lg + 2).clamp(12, 18);
        let match_bits = (lg + 1).clamp(10, 20);
        Self {
            direct: vec![AdaptiveProb::new(); CM_DIRECT],
            buckets: ProbBuckets::new(bucket_bits),
            misc: HashedProbs::new(misc_bits),
            order2: vec![(0, 0); 1 << match_bits],
            order1: vec![(0, 0); 1 << match_bits],
            match_mask: (1 << match_bits) - 1,
            runs: [0; CM_PREDICTORS],
            hit_rate: [AdaptiveProb::new(); CM_PREDICTORS + 1],
            recent: Vec::with_capacity(CM_RECENT),
            last_rank: CM_RECENT as u32,
            mixer: Mixer::new(CM_INPUTS, CM_MIX_SETS),
            apm: Apm::new(CM_APM_CONTEXTS),
            v1: 0,
            v2: 0,
            kh1: 0,
            kh2: 0,
            negative: false,
            lean,
            spent: if lean { vec![(0, 0); 65 * 64] } else { Vec::new() },
        }
    }

    /// A lean decision: the one cell's prediction, coded and trained with no mixing.
    #[inline]
    fn decide_lean<I: BitIo>(&mut self, io: &mut I, bit: bool, cell: Cell) -> bool {
        let p = self.cell_p(cell).unwrap_or(2048);
        let bit = io.code(bit, p);
        self.train(cell, bit, CM_LIMIT_LEAN);
        bit
    }

    #[inline]
    fn cell_p(&mut self, c: Cell) -> Option<u16> {
        match c {
            Cell::Direct(i) => Some(self.direct[i].p()),
            Cell::Bucketed(i) => Some(self.buckets.cell(i).p()),
            Cell::Hashed(h) => Some(self.misc.cell(h).p()),
            Cell::Off => None,
        }
    }

    #[inline]
    fn train(&mut self, c: Cell, bit: bool, limit: u32) {
        match c {
            Cell::Direct(i) => self.direct[i].update_with_limit(bit, limit),
            Cell::Bucketed(i) => self.buckets.cell(i).update_with_limit(bit, limit),
            Cell::Hashed(h) => self.misc.cell(h).update_with_limit(bit, limit),
            Cell::Off => {}
        }
    }

    /// Mix six context predictions (plus a bias), refine through the APM, code one decision, and train
    /// everything that predicted it.
    #[inline]
    fn decide<I: BitIo>(&mut self, io: &mut I, bit: bool, cells: [Cell; 6], set: usize, apm_ctx: usize) -> bool {
        for (i, &c) in cells.iter().enumerate() {
            let logit = self.cell_p(c).map_or(0, stretch);
            self.mixer.set(i, logit);
        }
        self.mixer.set(6, 256);
        let pm = self.mixer.mix(set);
        let pa = self.apm.refine(pm, apm_ctx);
        let p = ((u32::from(pm) + 3 * u32::from(pa)) / 4) as u16;
        let bit = io.code(bit, p);
        for c in cells {
            let limit = if matches!(c, Cell::Direct(_)) { CM_LIMIT_DIRECT } else { CM_LIMIT_HASHED };
            self.train(c, bit, limit);
        }
        self.mixer.update(bit);
        self.apm.update(bit);
        bit
    }

    /// Code the bit length `k ∈ 0..=64` down a seven-level tree. A decoder may read a corrupt `k > 64`;
    /// the caller refuses it. Each hashed context holds the walk in two buckets: the first four levels in
    /// one, the last three in a second chosen by the first four's outcome.
    fn code_k<I: BitIo>(&mut self, io: &mut I, k: u32) -> u32 {
        let (kh1, kh2) = (self.kh1 as usize, self.kh2 as usize);
        let contexts = [
            hash2(0xA3, (kh1 * 65 + kh2) as u64),
            hash2(0xA1, self.v1 as u64),
            hash3(0xA2, self.v1 as u64, self.v2 as u64),
        ];
        let mut bases = contexts.map(|c| self.buckets.bucket(c));
        let mut node = 1usize;
        let mut slot = 1usize;
        for b in (0..7).rev() {
            let bit = (k >> b) & 1 == 1;
            if b == 2 {
                bases = contexts.map(|c| self.buckets.bucket(hash2(c, node as u64)));
                slot = 1;
            }
            if self.lean {
                let bit = self.decide_lean(io, bit, Cell::Direct(CM_K0 + node));
                node = node * 2 + usize::from(bit);
                continue;
            }
            let cells = [
                Cell::Direct(CM_K0 + node),
                Cell::Direct(CM_K1 + kh1 * 128 + node),
                Cell::Bucketed(bases[0] + slot),
                Cell::Bucketed(bases[1] + slot),
                Cell::Bucketed(bases[2] + slot),
                Cell::Off,
            ];
            let bit = self.decide(io, bit, cells, node, node * 17 + (kh1 >> 2));
            node = node * 2 + usize::from(bit);
            slot = slot * 2 + usize::from(bit);
        }
        (node - 128) as u32
    }

    /// Code whether a nonzero residual of bit length `k` is negative. Coded after the magnitude, so an
    /// all-positive column learns its sign once instead of paying for it in the magnitude's lowest bit.
    fn code_sign<I: BitIo>(&mut self, io: &mut I, k: u32, negative: bool) -> bool {
        let at = CM_SIGN + k as usize * 2 + usize::from(self.negative);
        if self.lean {
            return self.decide_lean(io, negative, Cell::Direct(at));
        }
        let last = u64::from(self.negative);
        let cells = [
            Cell::Direct(at),
            Cell::Hashed(hash3(0xD1, last, u64::from(self.kh1) * 65 + u64::from(k))),
            Cell::Hashed(hash3(0xD2, self.v1 as u64, u64::from(k))),
            Cell::Hashed(hash3(0xD3, hash2(self.v1 as u64, self.v2 as u64), u64::from(k))),
            Cell::Off,
            Cell::Off,
        ];
        self.decide(io, negative, cells, CM_SIGN_SETS + k as usize, CM_SIGN_APM + k as usize * 2 + usize::from(self.negative))
    }

    /// Code the `k − 1` bits of `u` below its leading one, most significant first. Every context holds a
    /// nibble of the walk in one bucket, chosen at the nibble's first bit and indexed by the nibble's bits
    /// so far.
    fn code_mantissa<I: BitIo>(&mut self, io: &mut I, k: u32, u: u64) -> u64 {
        let hv1 = self.v1 as u64;
        let hv2 = hash2(self.v1 as u64, self.v2 as u64);
        let kb = k as usize / 16;
        let mut val = 1u64;
        // Bucket bases, per context: deep position, whole prefix, short prefix, with one length, with two
        // lengths, with the last value, with the last two values.
        let mut bases = [0usize; 7];
        let mut slot = 1usize;
        for j in (0..k - 1).rev() {
            let pos = k - 2 - j;
            let bit = (u >> j) & 1 == 1;
            if pos % 4 == 0 {
                let at = u64::from(k) << 8 | u64::from(pos);
                let prefix = if pos <= CM_PREFIX_DEPTH { val } else { val >> (pos - CM_PREFIX_DEPTH) };
                let short = hash3(0xB0, at, prefix);
                let with_k1 = hash2(short, u64::from(self.kh1));
                let keys = [
                    hash2(0xB6, at),
                    hash3(0xB5, at, val),
                    short,
                    with_k1,
                    hash2(with_k1, u64::from(self.kh2)),
                    hash2(short, hv1),
                    hash2(short, hv2),
                ];
                bases = keys.map(|h| self.buckets.bucket(h));
                slot = 1;
            }
            let whole = bases[1] + slot;
            let bit = if self.lean {
                let at = k as usize * 64 + pos.min(63) as usize;
                let p = self.buckets.cell(whole).p();
                let (adaptive, flat) = self.spent[at];
                let bit = io.code(bit, if adaptive < flat { p } else { 2048 });
                self.buckets.cell(whole).update_with_limit(bit, CM_LIMIT_LEAN);
                self.spent[at] = (adaptive.saturating_add(cost(bit, p)), flat.saturating_add(256));
                bit
            } else {
                // How often this exact prefix has been walked: zero means a value never seen before, so
                // the whole-prefix prediction is noise; any more and it is a recurring value's own path.
                let seen = self.buckets.cell(whole).count().min(3) as usize;
                if pos >= CM_DEEP {
                    let cells = [
                        Cell::Bucketed(bases[0] + slot),
                        Cell::Bucketed(whole),
                        Cell::Off,
                        Cell::Off,
                        Cell::Off,
                        Cell::Off,
                    ];
                    let set = CM_DEEP_SETS + kb.min(4) * 4 + seen;
                    self.decide(io, bit, cells, set, CM_DEEP_APM + k as usize * 4 + seen)
                } else {
                    let shallow = pos < CM_PREFIX_DEPTH;
                    let cells = [
                        Cell::Bucketed(bases[2] + slot),
                        Cell::Bucketed(bases[3] + slot),
                        Cell::Bucketed(bases[4] + slot),
                        if shallow { Cell::Off } else { Cell::Bucketed(whole) },
                        Cell::Bucketed(bases[5] + slot),
                        Cell::Bucketed(bases[6] + slot),
                    ];
                    let seen = if shallow { 0 } else { seen };
                    let set = CM_K_SETS + (pos.min(15) as usize * 5 + kb.min(4)) * 4 + seen;
                    self.decide(io, bit, cells, set, CM_K_APM + k as usize * 16 + pos.min(15) as usize)
                }
            };
            val = (val << 1) | u64::from(bit);
            slot = slot * 2 + usize::from(bit);
        }
        val
    }

    /// Code whether whole-value predictor `j`'s guess `g` is right, under the same breadth of context
    /// the residual bits get: the predictor's recent record, the last lengths, the last values, and the
    /// guessed value itself (some values recur in place, others never do).
    fn code_guess<I: BitIo>(&mut self, io: &mut I, hit: bool, j: usize, g: i64, agree: bool) -> bool {
        let run = self.runs[j].min(15) as usize;
        let jt = j as u64;
        let record = self.runs.iter().fold(0u64, |acc, &r| acc * 4 + u64::from(r.min(3)));
        let cells = [
            Cell::Hashed(hash3(0xE0, jt, (run * 2 + usize::from(agree)) as u64)),
            Cell::Hashed(hash3(0xE1, jt, u64::from(self.kh1) * 65 + u64::from(self.kh2))),
            Cell::Hashed(hash3(0xE2, jt, self.v1 as u64)),
            Cell::Hashed(hash3(0xE3, jt, g as u64)),
            Cell::Hashed(hash3(0xE4, jt, hash2(self.v1 as u64, self.v2 as u64))),
            Cell::Hashed(hash3(0xE5, jt, record)),
        ];
        let apm_ctx = CM_FLAG_APM + (j * 16 + run) * 2 + usize::from(agree);
        self.decide(io, hit, cells, CM_FLAG_SETS + j * 16 + run, apm_ctx)
    }

    /// Code whether the value is one of the last [`CM_RECENT`] distinct values, and if so its
    /// move-to-front rank (8 decisions down a tree): the value-level form of an LZ repeat distance, so a
    /// value that recurs a few distinct values later costs about the logarithm of that gap.
    fn code_recent<I: BitIo>(&mut self, io: &mut I, rank: Option<usize>) -> Option<Option<usize>> {
        if self.recent.is_empty() {
            return Some(None);
        }
        let last = (self.last_rank.min(CM_RECENT as u32) as usize).min(255);
        let last_band = (usize::BITS - last.leading_zeros()) as usize;
        let rec = self.runs.iter().fold(0u64, |acc, &r| acc * 4 + u64::from(r.min(3)));
        let cells = [
            Cell::Hashed(hash2(0xF0, last_band as u64)),
            Cell::Hashed(hash2(0xF1, u64::from(self.kh1) * 65 + u64::from(self.kh2))),
            Cell::Hashed(hash2(0xF2, self.v1 as u64)),
            Cell::Hashed(hash2(0xF3, rec)),
            Cell::Hashed(hash3(0xF4, self.v1 as u64, self.v2 as u64)),
            Cell::Hashed(hash2(0xF5, self.recent.len().min(CM_RECENT) as u64 / 16)),
        ];
        let hit = self.decide(io, rank.is_some(), cells, CM_RECENT_SETS + last_band, CM_RECENT_APM + last_band);
        if !hit {
            return Some(None);
        }
        let r = rank.unwrap_or(0);
        let mut node = 1usize;
        for b in (0..8).rev() {
            let bit = (r >> b) & 1 == 1;
            let n = node as u64;
            let cells = [
                Cell::Hashed(hash2(0xF6, n)),
                Cell::Hashed(hash3(0xF7, n, last_band as u64)),
                Cell::Hashed(hash3(0xF8, n, u64::from(self.kh1))),
                Cell::Hashed(hash3(0xF9, n, self.v1 as u64)),
                Cell::Hashed(hash3(0xFA, n, rec)),
                Cell::Off,
            ];
            let bit = self.decide(io, bit, cells, CM_RANK_SETS + node, CM_RANK_APM + node * 9 + last_band.min(8));
            node = node * 2 + usize::from(bit);
        }
        let r = node - 256;
        (r < self.recent.len()).then_some(Some(r))
    }

    /// Code one value (on a decoder `v` is ignored and the decoded value returned). `None` only for a
    /// corrupt stream that decodes a bit length past 64.
    fn code_value<I: BitIo>(&mut self, io: &mut I, mode: u8, v: i64) -> Option<i64> {
        let (v1, v2) = (self.v1, self.v2);
        let h2 = hash3(0xC0, v1 as u64, v2 as u64);
        let h1 = hash2(0xC1, v1 as u64);
        let (s2, c2) = (h2 as usize & self.match_mask, (h2 >> 32) as u32 | 1);
        let (s1, c1) = (h1 as usize & self.match_mask, (h1 >> 32) as u32 | 1);
        let guesses = [
            (self.order2[s2].0 == c2).then_some(self.order2[s2].1),
            (self.order1[s1].0 == c1).then_some(self.order1[s1].1),
            Some(v1),
        ];
        let mut value = None;
        for j in 0..if self.lean { 0 } else { CM_PREDICTORS } {
            let Some(g) = guesses[j] else { continue };
            if guesses[..j].contains(&Some(g)) || self.hit_rate[j].p() < CM_GATE {
                continue;
            }
            let hit = self.code_guess(io, v == g, j, g, guesses[j + 1..].contains(&Some(g)));
            if hit {
                value = Some(g);
                break;
            }
        }
        let pred = match mode % CM_LEAN {
            0 => 0,
            1 => v1,
            _ => v1.wrapping_mul(2).wrapping_sub(v2),
        };
        let mut rank = None;
        if !self.lean && value.is_none() && self.hit_rate[CM_PREDICTORS].p() >= CM_GATE {
            // The recency path is the encoder's choice, not a fact about the value: a recent value whose
            // residual is short anyway (a small step) is cheaper sent as that residual than as a rank.
            let residual_bits = 64 - zigzag(v.wrapping_sub(pred)).leading_zeros();
            let r = self
                .recent
                .iter()
                .position(|&x| x == v)
                .filter(|&r| residual_bits > 64 - (r as u64 + 1).leading_zeros() + CM_RECENT_MARGIN);
            if let Some(r) = self.code_recent(io, r)? {
                rank = Some(r);
                value = Some(self.recent[r]);
            }
        }
        let value = match value {
            Some(x) => x,
            None => {
                // Sign and magnitude, not zig-zag: a zig-zagged column's sign sits in the lowest
                // magnitude bit, where the deep, rarely shared prefixes never learn it.
                let r = v.wrapping_sub(pred);
                let m = r.unsigned_abs();
                let k = self.code_k(io, 64 - m.leading_zeros());
                let m = match k {
                    0 => 0,
                    1 => 1,
                    2..=64 => self.code_mantissa(io, k, m),
                    _ => return None,
                };
                if m == 0 {
                    pred
                } else {
                    let negative = self.code_sign(io, k, r < 0);
                    self.negative = negative;
                    let r = m as i64;
                    pred.wrapping_add(if negative { r.wrapping_neg() } else { r })
                }
            }
        };
        for ((run, rate), g) in self.runs.iter_mut().zip(&mut self.hit_rate).zip(guesses) {
            *run = if g == Some(value) { *run + 1 } else { 0 };
            rate.update_with_limit(g == Some(value), CM_LIMIT_DIRECT);
        }
        let in_recent = rank.is_some() || self.recent.contains(&value);
        self.hit_rate[CM_PREDICTORS].update_with_limit(in_recent, CM_LIMIT_DIRECT);
        self.kh2 = self.kh1;
        self.kh1 = 64 - value.wrapping_sub(pred).unsigned_abs().leading_zeros();
        self.order2[s2] = (c2, value);
        self.order1[s1] = (c1, value);
        let at = rank.or_else(|| self.recent.iter().position(|&x| x == value));
        self.last_rank = at.map_or(CM_RECENT as u32, |r| r as u32);
        match at {
            Some(r) => {
                self.recent[..=r].rotate_right(1);
            }
            None => {
                if self.recent.len() == CM_RECENT {
                    self.recent.pop();
                }
                self.recent.insert(0, value);
            }
        }
        self.v2 = v1;
        self.v1 = value;
        Some(value)
    }
}

fn cm_payload(v: &[i64], mode: u8) -> Vec<u8> {
    let mut model = IntCm::new(v.len(), mode >= CM_LEAN);
    let mut enc = Encoder::new();
    for &x in v {
        model.code_value(&mut enc, mode, x);
    }
    enc.finish()
}

/// Above twice this many values the modes are ranked on a prefix of this length, and only the best two
/// run over the whole column — the ranking a prefix gives is the whole column's, at a fraction of the work.
const CM_MODE_SAMPLE: usize = 8192;
const CM_MODES_KEPT: usize = 2;

/// The entropy-coded form of `v` (tag included): the smallest over the predictor modes (all of them for a
/// short column; the two a prefix ranks best for a long one).
pub fn cm_encode_int_column(v: &[i64]) -> Vec<u8> {
    let mut modes = CM_MODES.to_vec();
    if v.len() > 2 * CM_MODE_SAMPLE {
        let sample = &v[..CM_MODE_SAMPLE];
        modes.sort_by_cached_key(|&m| cm_payload(sample, m).len());
        modes.truncate(CM_MODES_KEPT);
    }
    let mut best: Option<Vec<u8>> = None;
    for mode in modes {
        let payload = cm_payload(v, mode);
        let mut c = vec![T_INTS_CM];
        write_uvarint(v.len() as u64, &mut c);
        c.push(mode);
        write_uvarint(payload.len() as u64, &mut c);
        c.extend_from_slice(&payload);
        if best.as_ref().map_or(true, |b| c.len() < b.len()) {
            best = Some(c);
        }
    }
    best.expect("CM_MODES > 0")
}

fn cm_decode_body(buf: &[u8], pos: &mut usize, max_elements: usize) -> Option<Vec<i64>> {
    let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
    charge_work((n as u64).saturating_mul(CM_WORK_PER_ELEMENT), max_elements)?;
    let mode = *buf.get(*pos)?;
    *pos += 1;
    if !CM_MODES.contains(&mode) {
        return None;
    }
    let len = read_uvarint(buf, pos)? as usize;
    let payload = buf.get(*pos..pos.checked_add(len)?)?;
    *pos += len;
    let mut model = IntCm::new(n, mode >= CM_LEAN);
    let mut dec = Decoder::new(payload);
    let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
    for _ in 0..n {
        v.push(model.code_value(&mut dec, mode, 0)?);
    }
    Some(v)
}

// ---- Decimal float columns -------------------------------------------------------------------

/// `10^e` for `e ≤ 22` — every one exactly representable, so `k / 10^e` with an exact `k` is the
/// correctly rounded decimal and a decoder reproduces it bit for bit.
const POW10: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19,
    1e20, 1e21, 1e22,
];
/// The largest decimal exponent the encoder searches.
const MAX_DECIMAL_EXP: usize = 18;
/// `|k|` must stay below 2^53 so it is an exact double.
const EXACT_INT_BOUND: f64 = 9_007_199_254_740_992.0;

/// The integer `k` with `bits(k / 10^e) == bits(x)`, if one exists.
#[inline]
fn decimal_digits(x: f64, e: usize) -> Option<i64> {
    let k = (x * POW10[e]).round();
    if !(k.abs() < EXACT_INT_BOUND) {
        return None;
    }
    let k = k as i64;
    ((k as f64 / POW10[e]).to_bits() == x.to_bits()).then_some(k)
}

/// The decimal form of a float column: the exponent `e` under which the most values are exactly
/// `k / 10^e`, the rest kept verbatim as exceptions, and the `k` column described by the full integer
/// menu. `None` when fewer than half the values are decimal at any exponent (the form cannot win).
pub fn emit_decimal_float_column(v: &[f64]) -> Option<Vec<u8>> {
    if v.is_empty() {
        return None;
    }
    // Each value's smallest exact exponent; a value exact at `e` stays exact at any larger exponent
    // whose `k` is still exact, so coverage is (almost) cumulative and the histogram picks `e`.
    let mut first_exact = [0usize; MAX_DECIMAL_EXP + 2];
    for &x in v {
        let e = (0..=MAX_DECIMAL_EXP).find(|&e| decimal_digits(x, e).is_some()).unwrap_or(MAX_DECIMAL_EXP + 1);
        first_exact[e] += 1;
    }
    let mut covered = 0usize;
    let mut best_e = 0usize;
    let mut best_cover = 0usize;
    for (e, &c) in first_exact.iter().enumerate().take(MAX_DECIMAL_EXP + 1) {
        covered += c;
        if covered > best_cover {
            best_cover = covered;
            best_e = e;
        }
    }
    if best_cover * 2 < v.len() {
        return None;
    }
    let mut ks = Vec::with_capacity(v.len());
    let mut exceptions: Vec<(usize, u64)> = Vec::new();
    let mut last = 0i64;
    for (i, &x) in v.iter().enumerate() {
        match decimal_digits(x, best_e) {
            Some(k) => {
                ks.push(k);
                last = k;
            }
            None => {
                ks.push(last);
                exceptions.push((i, x.to_bits()));
            }
        }
    }
    let mut out = vec![T_FLOATS_DECIMAL];
    write_uvarint(v.len() as u64, &mut out);
    out.push(best_e as u8);
    write_uvarint(exceptions.len() as u64, &mut out);
    let mut prev = 0usize;
    for &(i, bits) in &exceptions {
        write_uvarint((i - prev) as u64, &mut out);
        prev = i;
        out.extend_from_slice(&bits.to_le_bytes());
    }
    emit_best_int_column(&ks, &mut out);
    Some(out)
}

/// The raw-bits form of a float column: each value's IEEE-754 bit pattern, described by the full integer
/// menu. For full-precision floats no decimal exponent fits, but the patterns still carry structure — a
/// shared sign and exponent, recurring values, a sorted order — that the integer forms see.
pub fn emit_float_bits_column(v: &[f64]) -> Option<Vec<u8>> {
    if v.is_empty() {
        return None;
    }
    let bits: Vec<i64> = v.iter().map(|x| x.to_bits() as i64).collect();
    let mut out = vec![T_FLOATS_BITS];
    emit_best_int_column(&bits, &mut out);
    Some(out)
}

/// Decode one float column whose tag byte has already been read as `tag`; `None` for a non-float tag
/// or a corrupt/hostile body. `max_elements` bounds the count and the inner integer column's work.
pub fn decode_float_column_body(
    tag: u8,
    buf: &[u8],
    pos: &mut usize,
    max_elements: usize,
    depth: u32,
) -> Option<Vec<f64>> {
    if depth == 0 {
        DECODE_WORK.with(|c| c.set(0));
    }
    if depth >= DECODE_MAX_DEPTH {
        return None;
    }
    if tag == T_FLOATS_BITS {
        let inner_tag = *buf.get(*pos)?;
        *pos += 1;
        let bits = decode_int_column_body(inner_tag, buf, pos, max_elements, depth + 1)?;
        return Some(bits.into_iter().map(|b| f64::from_bits(b as u64)).collect());
    }
    if tag != T_FLOATS_DECIMAL {
        return None;
    }
    let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
    let e = usize::from(*buf.get(*pos)?);
    *pos += 1;
    let scale = *POW10.get(e)?;
    let num_exc = bounded(read_uvarint(buf, pos)?, n)?;
    let mut exceptions = Vec::with_capacity(num_exc.min(PREALLOC_CAP));
    let mut idx = 0usize;
    for _ in 0..num_exc {
        idx = idx.checked_add(read_uvarint(buf, pos)? as usize)?;
        if idx >= n {
            return None;
        }
        let bits = u64::from_le_bytes(buf.get(*pos..pos.checked_add(8)?)?.try_into().ok()?);
        *pos += 8;
        exceptions.push((idx, bits));
    }
    let inner_tag = *buf.get(*pos)?;
    *pos += 1;
    let ks = decode_int_column_body(inner_tag, buf, pos, max_elements, depth + 1)?;
    if ks.len() != n {
        return None;
    }
    let mut v: Vec<f64> = ks.iter().map(|&k| k as f64 / scale).collect();
    for (i, bits) in exceptions {
        v[i] = f64::from_bits(bits);
    }
    Some(v)
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

    // Number theory before statistics: a column whose every value shares a factor with its minimum
    // (whole seconds stored in microseconds, prices on a cent grid) describes the quotients instead.
    let factored = detect_common_multiple(v);
    if let Some((base, g)) = factored {
        let q: Vec<i64> = v.iter().map(|&x| ((x as u64).wrapping_sub(base as u64) / g) as i64).collect();
        let mut c = vec![T_INTS_MULT];
        write_uvarint(v.len() as u64, &mut c);
        write_uvarint(zigzag(base), &mut c);
        write_uvarint(g, &mut c);
        emit_best_int_column(&q, &mut c);
        consider(&mut best, c);
    }
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

    // Representation search: split the patterns at their high bits and XOR each value's low bits against
    // the last value that shared them — offered when the estimate predicts the split pays.
    if menu() == Menu::Full && v.len() >= CXOR_MIN_ELEMENTS && !IN_CXOR.with(std::cell::Cell::get) {
        let bits: Vec<u64> = v.iter().map(|&x| x as u64).collect();
        let (est, plain) = cxor_estimate(&bits);
        if plain >= CXOR_MIN_WIDTH * bits.len() as u64 && est * 100 <= plain * (100 - CXOR_MIN_GAIN) {
            let sample = &bits[..bits.len().min(CXOR_SAMPLE)];
            let split = CXOR_SPLITS
                .into_iter()
                .min_by_key(|&s| conditional_xor_encode(sample, s).len())
                .expect("CXOR_SPLITS is not empty");
            consider(&mut best, conditional_xor_encode(&bits, split));
        }
    }

    // Statistics last: the entropy-coded form, after every structural form (the last-resort attacks
    // above fire only when nothing structural compressed the column, and a statistical coder shaving a
    // few bytes must not hide a register). Offered only for a column a default receiver accepts; a
    // factored column's quotients were already offered it, so the unfactored column skips it.
    if menu() == Menu::Full
        && factored.is_none()
        && v.len() >= CM_MIN_ELEMENTS
        && (v.len() as u64).saturating_mul(CM_WORK_PER_ELEMENT) <= CM_OFFER_BUDGET
    {
        consider(&mut best, cm_encode_int_column(v));
    }

    out.extend_from_slice(&best);
}

/// The **computable upper bound on Kolmogorov complexity** of `v` over this description language:
/// the shortest self-delimiting program (from the full menu) that reproduces `v`. The returned bytes are
/// a re-checkable witness — [`decode_int_seq`] reproduces `v` exactly.
pub fn describe_int_seq(v: &[i64]) -> Vec<u8> {
    let mut out = Vec::new();
    with_menu(Menu::Full, || emit_best_int_column(v, &mut out));
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

thread_local! {
    /// A shared, per-column-tree WORK budget for the generator columns. `max_elements` bounds the
    /// element COUNT, but the generators differ in per-element cost by orders of magnitude — an
    /// affine `wrapping_add` is one op, an FCSR bit is a 2-adic **bignum** step, an LFSR bit is an
    /// `O(L)` tap fold, a `T_GEN` element re-evaluates the whole expression tree — so a count-bound
    /// cannot bound wall-time. Each expensive generator charges its TRUE work (`produced ·
    /// per-element-cost`) here; once one column tree's running total exceeds `max_elements`
    /// work-units the decode is refused (clean `None`) BEFORE the (multi-minute) expansion runs.
    /// This is the small-descriptor / huge-work DoS gate the byte and count budgets cannot express.
    static DECODE_WORK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Charge `units` of generation work against [`DECODE_WORK`]; `None` once the budget
/// (`max_elements` work-units) is spent. Saturating, so a crafted huge `units` cannot wrap.
#[inline]
fn charge_work(units: u64, max_elements: usize) -> Option<()> {
    DECODE_WORK.with(|c| {
        let used = c.get().saturating_add(units);
        (used <= max_elements as u64).then(|| c.set(used))
    })
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
    // A fresh column tree starts with a full work budget; nested blocks (periodic) share it.
    if depth == 0 {
        DECODE_WORK.with(|c| c.set(0));
    }
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
            // Clocking the register is O(total_bits · L) — charge it so a huge keystream from a
            // tiny header is refused before the O(n²)-in-the-worst-case replay ever starts.
            charge_work(
                (total_bits as u64).saturating_mul(l.max(1) as u64),
                max_elements,
            )?;
            let nbytes = l.div_ceil(8);
            let tap_bytes = buf.get(*pos..pos.checked_add(nbytes)?)?;
            *pos += nbytes;
            let taps: Vec<bool> = bitunpack(tap_bytes, l, 1)?
                .into_iter()
                .map(|x| x == 1)
                .collect();
            let seed_bytes = buf.get(*pos..pos.checked_add(nbytes)?)?;
            *pos += nbytes;
            let seed: Vec<bool> = bitunpack(seed_bytes, l, 1)?
                .into_iter()
                .map(|x| x == 1)
                .collect();
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
            // Each 2-adic step is a bignum sub+halve on operands of ~max(|p|,|q|) limbs — charge
            // `total_bits · limbs` so a 10^8-bit keystream from a ~10-byte descriptor is refused
            // before the multi-minute expansion runs.
            let limbs = (bigint_bitlen(&p).max(bigint_bitlen(&q)) / 64 + 1) as u64;
            charge_work((total_bits as u64).saturating_mul(limbs), max_elements)?;
            bits_to_bytes(&fcsr_generate(&p, &q, total_bits))
        }
        // General generator: parse the bounded `GenExpr`, evaluate it at 0..n in the sandbox.
        T_GEN => {
            let mut budget = MAX_GEN_NODES;
            let expr = deserialize_gen(buf, pos, &mut budget, 0)?;
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
            // Every element re-evaluates the whole expression tree — charge `n · nodes` so a wide
            // column over a big expression is refused before the eval loop.
            let nodes = (MAX_GEN_NODES - budget).max(1) as u64;
            charge_work((n as u64).saturating_mul(nodes), max_elements)?;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            for i in 0..n {
                v.push(gen_eval(&expr, i as i64));
            }
            v
        }
        // Byte column: read the count, widen each raw byte to an Int.
        T_BYTES => {
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
            let raw = buf.get(*pos..pos.checked_add(n)?)?;
            *pos += n;
            raw.iter().map(|&b| b as i64).collect()
        }
        // Delta column: cumulative-sum the zig-zag differences.
        T_INTS_DELTA => {
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
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
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
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
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
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
                if v.len().checked_add(len)? > max_elements.min(RLE_MAX_TOTAL) {
                    return None;
                }
                v.resize(v.len() + len, val);
            }
            v
        }
        // Dictionary column: read the distinct values, map the bit-packed indices back through them.
        T_INTS_DICT => {
            let d = bounded(read_uvarint(buf, pos)?, max_elements)?;
            let mut dict = Vec::with_capacity(d.min(PREALLOC_CAP));
            for _ in 0..d {
                dict.push(unzigzag(read_uvarint(buf, pos)?));
            }
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
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
        // Common-multiple column: decode the quotient column, scale it back onto the base.
        T_INTS_MULT => {
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
            let base = unzigzag(read_uvarint(buf, pos)?);
            let g = read_uvarint(buf, pos)?;
            if g < 2 || depth >= DECODE_MAX_DEPTH {
                return None;
            }
            let inner_tag = *buf.get(*pos)?;
            *pos += 1;
            let q = decode_int_column_body(inner_tag, buf, pos, max_elements, depth + 1)?;
            if q.len() != n {
                return None;
            }
            q.iter().map(|&x| base.wrapping_add((x as u64).wrapping_mul(g) as i64)).collect()
        }
        // Entropy-coded column: replay the same model the encoder ran, decision by decision.
        T_INTS_CM => cm_decode_body(buf, pos, max_elements)?,
        // Conditional-XOR column: the high column, the low column, then each value rebuilt against the
        // last value that shared its high bits.
        T_INTS_CXOR => {
            let n = bounded(read_uvarint(buf, pos)?, max_elements)?;
            let split = u32::from(*buf.get(*pos)?);
            *pos += 1;
            if !(1..=63).contains(&split) || depth >= DECODE_MAX_DEPTH {
                return None;
            }
            let hi_tag = *buf.get(*pos)?;
            *pos += 1;
            let hi = decode_int_column_body(hi_tag, buf, pos, max_elements, depth + 1)?;
            let lo_tag = *buf.get(*pos)?;
            *pos += 1;
            let lo = decode_int_column_body(lo_tag, buf, pos, max_elements, depth + 1)?;
            if hi.len() != n || lo.len() != n {
                return None;
            }
            cxor_join(&hi, &lo, split)?
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
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/describe/tests.rs"]
mod tests;
