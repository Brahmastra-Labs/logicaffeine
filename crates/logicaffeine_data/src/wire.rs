//! Shared wire-codec core — the exact byte format of the peer/transport codec
//! (`logicaffeine_compile::concurrency::marshal`), factored out so that TWO value models
//! encode and decode through ONE definition:
//!
//!   * the interpreter's `RuntimeValue` (host side), and
//!   * AOT-generated Rust `enum`/`struct` types (native side).
//!
//! Because both go through this module, the wire bytes are byte-identical by construction —
//! not a parallel re-implementation that can drift. This is what lets a compile-once native
//! partial evaluator receive a program as data over the *real* fast codec, at native speed.
//!
//! The tag bytes and field layout MUST stay in lockstep with `marshal.rs`; the
//! `wire_bytes_match_peer_codec` test in `logicaffeine_compile` proves it.

// ── Tag bytes (subset used by algebraic ASTs; identical values to marshal.rs) ──────────────
/// `Nothing` / unit.
pub const T_NOTHING: u8 = 0;
/// Boolean `false`.
pub const T_FALSE: u8 = 1;
/// Boolean `true`.
pub const T_TRUE: u8 = 2;
/// A signed integer, zig-zag + LEB128 varint.
pub const T_INT: u8 = 3;
/// An IEEE-754 double, 8 little-endian bytes.
pub const T_FLOAT: u8 = 4;
/// A UTF-8 string: varint byte-length, then the bytes.
pub const T_TEXT: u8 = 6;
/// A heterogeneous list: varint element count, then each element (tagged).
pub const T_LIST: u8 = 13;
/// An inductive/enum value: `type_name` string, `constructor` string, varint arg count, args.
pub const T_INDUCTIVE: u8 = 18;

// ── Primitive stream helpers (byte-for-byte identical to marshal.rs) ───────────────────────

/// LEB128 unsigned varint.
#[inline]
pub fn write_uvarint(mut x: u64, out: &mut Vec<u8>) {
    while x >= 0x80 {
        out.push((x as u8) | 0x80);
        x >>= 7;
    }
    out.push(x as u8);
}

/// Inverse of [`write_uvarint`]; `None` on truncation or overlong (>64-bit) input.
#[inline]
pub fn read_uvarint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let b = *buf.get(*pos)?;
        *pos += 1;
        if shift >= 64 {
            return None;
        }
        result |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
}

/// Map a signed integer onto an unsigned one so small magnitudes stay short.
#[inline]
pub fn zigzag(x: i64) -> u64 {
    ((x << 1) ^ (x >> 63)) as u64
}

/// Inverse of [`zigzag`].
#[inline]
pub fn unzigzag(x: u64) -> i64 {
    ((x >> 1) as i64) ^ -((x & 1) as i64)
}

/// A length-prefixed UTF-8 string, UNtagged (used for inductive `type_name`/`constructor`).
#[inline]
pub fn write_str(s: &str, out: &mut Vec<u8>) {
    write_uvarint(s.len() as u64, out);
    out.extend_from_slice(s.as_bytes());
}

/// Inverse of [`write_str`]; `None` on truncation or invalid UTF-8.
#[inline]
pub fn read_str(buf: &[u8], pos: &mut usize) -> Option<String> {
    let n = read_uvarint(buf, pos)? as usize;
    let bytes = buf.get(*pos..pos.checked_add(n)?)?;
    *pos += n;
    String::from_utf8(bytes.to_vec()).ok()
}

/// Read the tag byte at `pos` and advance; `None` if it is not `expected`.
#[inline]
pub fn expect_tag(buf: &[u8], pos: &mut usize, expected: u8) -> Option<()> {
    let t = *buf.get(*pos)?;
    if t != expected {
        return None;
    }
    *pos += 1;
    Some(())
}

// ── Inductive header (the seam codegen emits into) ─────────────────────────────────────────

/// Write the header of an inductive value: `T_INDUCTIVE`, its `type_name`, its `constructor`,
/// and the argument count. The args themselves follow, each written via [`WireEncode`].
#[inline]
pub fn write_inductive_header(out: &mut Vec<u8>, type_name: &str, constructor: &str, nargs: u64) {
    out.push(T_INDUCTIVE);
    write_str(type_name, out);
    write_str(constructor, out);
    write_uvarint(nargs, out);
}

/// Read an inductive header, returning `(type_name, constructor, nargs)`. The caller dispatches
/// on `constructor` and reads `nargs` arguments via [`WireDecode`].
#[inline]
pub fn read_inductive_header(buf: &[u8], pos: &mut usize) -> Option<(String, String, u64)> {
    expect_tag(buf, pos, T_INDUCTIVE)?;
    let type_name = read_str(buf, pos)?;
    let constructor = read_str(buf, pos)?;
    let nargs = read_uvarint(buf, pos)?;
    Some((type_name, constructor, nargs))
}

// ── The two traits (codegen emits impls for generated types; RuntimeValue impls them host-side) ─

/// A value that serializes into the shared wire format.
pub trait WireEncode {
    /// Append `self`'s tagged wire bytes to `out`.
    fn wire_encode(&self, out: &mut Vec<u8>);
}

/// A value that reconstructs from the shared wire format.
pub trait WireDecode: Sized {
    /// Read one value starting at `*pos`, advancing `pos`. `None` on any malformed input.
    fn wire_decode(buf: &[u8], pos: &mut usize) -> Option<Self>;
}

impl WireEncode for i64 {
    fn wire_encode(&self, out: &mut Vec<u8>) {
        out.push(T_INT);
        write_uvarint(zigzag(*self), out);
    }
}
impl WireDecode for i64 {
    fn wire_decode(buf: &[u8], pos: &mut usize) -> Option<Self> {
        expect_tag(buf, pos, T_INT)?;
        Some(unzigzag(read_uvarint(buf, pos)?))
    }
}

impl WireEncode for bool {
    fn wire_encode(&self, out: &mut Vec<u8>) {
        out.push(if *self { T_TRUE } else { T_FALSE });
    }
}
impl WireDecode for bool {
    fn wire_decode(buf: &[u8], pos: &mut usize) -> Option<Self> {
        let t = *buf.get(*pos)?;
        *pos += 1;
        match t {
            T_TRUE => Some(true),
            T_FALSE => Some(false),
            _ => None,
        }
    }
}

/// The bits every NaN is written as: the canonical quiet NaN. A NaN's sign and payload differ by host
/// (x86 arithmetic makes `0xFFF8…`, other hosts and WASM differ), and the wire is the one place a
/// program can read a float's bits, so every encoder writes a NaN as this and nothing else.
pub const CANONICAL_NAN_BITS: u64 = 0x7FF8_0000_0000_0000;

/// The bits the wire carries for `x`: its own, or the canonical NaN's.
#[inline]
pub fn canonical_f64_bits(x: f64) -> u64 {
    if x.is_nan() {
        CANONICAL_NAN_BITS
    } else {
        x.to_bits()
    }
}

/// `x`, or the canonical NaN when `x` is a NaN.
#[inline]
pub fn canonical_f64(x: f64) -> f64 {
    f64::from_bits(canonical_f64_bits(x))
}

/// Append `x`'s wire bytes (little-endian, NaN canonical).
#[inline]
pub fn put_f64(out: &mut Vec<u8>, x: f64) {
    out.extend_from_slice(&canonical_f64_bits(x).to_le_bytes());
}

/// Append a float column's wire bytes (little-endian, every NaN canonical) in one pass: each element is
/// loaded, tested and stored once, the memory traffic of a copy. On an AVX2 host the pass runs four
/// elements per instruction (an unordered compare and a blend).
pub fn put_f64s(out: &mut Vec<u8>, v: &[f64]) {
    let start = out.len();
    out.reserve(v.len() * 8);
    let dst = out.spare_capacity_mut().as_mut_ptr().cast::<u64>();
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            // SAFETY: AVX2 is present; `reserve` made room for `v.len()` words past `start`.
            unsafe { put_f64s_avx2(dst, v) };
        } else {
            // SAFETY: `reserve` made room for `v.len()` words past `start`.
            unsafe { put_f64s_into(dst, v) };
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    // SAFETY: `reserve` made room for `v.len()` words past `start`.
    unsafe {
        put_f64s_into(dst, v)
    };
    // SAFETY: every byte in `start..start + 8·v.len()` was written above.
    unsafe { out.set_len(start + v.len() * 8) };
}

/// Write `v`'s wire words to `dst`, each once and unaligned.
///
/// # Safety
/// `dst` must be valid for `v.len()` unaligned 8-byte writes.
#[inline(always)]
unsafe fn put_f64s_into(dst: *mut u64, v: &[f64]) {
    for (i, &x) in v.iter().enumerate() {
        // SAFETY: the caller's contract covers word `i < v.len()`.
        unsafe { dst.add(i).write_unaligned(canonical_f64_bits(x).to_le()) };
    }
}

/// [`put_f64s_into`] compiled for AVX2.
///
/// # Safety
/// The host has AVX2, and `dst` is valid for `v.len()` unaligned 8-byte writes.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn put_f64s_avx2(dst: *mut u64, v: &[f64]) {
    // SAFETY: forwarded from the caller.
    unsafe { put_f64s_into(dst, v) }
}

impl WireEncode for f64 {
    fn wire_encode(&self, out: &mut Vec<u8>) {
        out.push(T_FLOAT);
        put_f64(out, *self);
    }
}
impl WireDecode for f64 {
    fn wire_decode(buf: &[u8], pos: &mut usize) -> Option<Self> {
        expect_tag(buf, pos, T_FLOAT)?;
        let b: [u8; 8] = buf.get(*pos..pos.checked_add(8)?)?.try_into().ok()?;
        *pos += 8;
        Some(f64::from_le_bytes(b))
    }
}

impl WireEncode for String {
    fn wire_encode(&self, out: &mut Vec<u8>) {
        out.push(T_TEXT);
        write_str(self, out);
    }
}
impl WireDecode for String {
    fn wire_decode(buf: &[u8], pos: &mut usize) -> Option<Self> {
        expect_tag(buf, pos, T_TEXT)?;
        read_str(buf, pos)
    }
}

impl<T: WireEncode + ?Sized> WireEncode for Box<T> {
    fn wire_encode(&self, out: &mut Vec<u8>) {
        (**self).wire_encode(out);
    }
}
impl<T: WireDecode> WireDecode for Box<T> {
    fn wire_decode(buf: &[u8], pos: &mut usize) -> Option<Self> {
        Some(Box::new(T::wire_decode(buf, pos)?))
    }
}

impl<T: WireEncode> WireEncode for Vec<T> {
    fn wire_encode(&self, out: &mut Vec<u8>) {
        out.push(T_LIST);
        write_uvarint(self.len() as u64, out);
        for e in self {
            e.wire_encode(out);
        }
    }
}
impl<T: WireDecode> WireDecode for Vec<T> {
    fn wire_decode(buf: &[u8], pos: &mut usize) -> Option<Self> {
        expect_tag(buf, pos, T_LIST)?;
        let n = read_uvarint(buf, pos)? as usize;
        let mut xs = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            xs.push(T::wire_decode(buf, pos)?);
        }
        Some(xs)
    }
}

// A `Seq of T` field codegens as `LogosSeq<T>` (the interpreter/AOT sequence type), so the
// generated `wire_encode`/`wire_decode` need it too — same `T_LIST` bytes as `Vec<T>`.
impl<T: WireEncode> WireEncode for crate::types::LogosSeq<T> {
    fn wire_encode(&self, out: &mut Vec<u8>) {
        let inner = self.0.borrow();
        out.push(T_LIST);
        write_uvarint(inner.len() as u64, out);
        for e in inner.iter() {
            e.wire_encode(out);
        }
    }
}
impl<T: WireDecode> WireDecode for crate::types::LogosSeq<T> {
    fn wire_decode(buf: &[u8], pos: &mut usize) -> Option<Self> {
        expect_tag(buf, pos, T_LIST)?;
        let n = read_uvarint(buf, pos)? as usize;
        let mut v = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            v.push(T::wire_decode(buf, pos)?);
        }
        Some(crate::types::LogosSeq::from_vec(v))
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/wire/tests.rs"]
mod tests;
