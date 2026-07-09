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

impl WireEncode for f64 {
    fn wire_encode(&self, out: &mut Vec<u8>) {
        out.push(T_FLOAT);
        out.extend_from_slice(&self.to_le_bytes());
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
mod tests {
    use super::*;

    // A mirror of what codegen emits for an algebraic type: an enum with scalar, string,
    // bool, float, boxed-recursive, and list fields. If THIS round-trips, the emitted
    // impls round-trip.
    #[derive(Debug, Clone, PartialEq)]
    enum Tree {
        Leaf(i64),
        Str(String),
        Node { tag: String, flag: bool, ratio: f64, kids: Vec<Tree>, boxed: Box<Tree> },
        Empty,
    }

    impl WireEncode for Tree {
        fn wire_encode(&self, out: &mut Vec<u8>) {
            match self {
                Tree::Leaf(v) => {
                    write_inductive_header(out, "Tree", "Leaf", 1);
                    v.wire_encode(out);
                }
                Tree::Str(s) => {
                    write_inductive_header(out, "Tree", "Str", 1);
                    s.wire_encode(out);
                }
                Tree::Node { tag, flag, ratio, kids, boxed } => {
                    write_inductive_header(out, "Tree", "Node", 5);
                    tag.wire_encode(out);
                    flag.wire_encode(out);
                    ratio.wire_encode(out);
                    kids.wire_encode(out);
                    boxed.wire_encode(out);
                }
                Tree::Empty => write_inductive_header(out, "Tree", "Empty", 0),
            }
        }
    }
    impl WireDecode for Tree {
        fn wire_decode(buf: &[u8], pos: &mut usize) -> Option<Self> {
            let (ty, ctor, _n) = read_inductive_header(buf, pos)?;
            assert_eq!(ty, "Tree");
            Some(match ctor.as_str() {
                "Leaf" => Tree::Leaf(i64::wire_decode(buf, pos)?),
                "Str" => Tree::Str(String::wire_decode(buf, pos)?),
                "Node" => Tree::Node {
                    tag: String::wire_decode(buf, pos)?,
                    flag: bool::wire_decode(buf, pos)?,
                    ratio: f64::wire_decode(buf, pos)?,
                    kids: Vec::<Tree>::wire_decode(buf, pos)?,
                    boxed: Box::<Tree>::wire_decode(buf, pos)?,
                },
                "Empty" => Tree::Empty,
                _ => return None,
            })
        }
    }

    fn roundtrip(t: &Tree) -> Tree {
        let mut out = Vec::new();
        t.wire_encode(&mut out);
        let mut pos = 0usize;
        let back = Tree::wire_decode(&out, &mut pos).expect("decode");
        assert_eq!(pos, out.len(), "decoder must consume every byte");
        back
    }

    #[test]
    fn varint_roundtrips_across_the_range() {
        for x in [0u64, 1, 127, 128, 16383, 16384, u32::MAX as u64, u64::MAX] {
            let mut b = Vec::new();
            write_uvarint(x, &mut b);
            let mut p = 0;
            assert_eq!(read_uvarint(&b, &mut p), Some(x));
            assert_eq!(p, b.len());
        }
    }

    #[test]
    fn zigzag_roundtrips_including_extremes() {
        for x in [0i64, 1, -1, 42, -42, i64::MAX, i64::MIN] {
            assert_eq!(unzigzag(zigzag(x)), x);
        }
    }

    #[test]
    fn scalar_leaves_roundtrip() {
        for v in [0i64, 1, -1, 42, -99999, i64::MAX, i64::MIN] {
            assert_eq!(roundtrip(&Tree::Leaf(v)), Tree::Leaf(v));
        }
    }

    #[test]
    fn strings_roundtrip_including_empty_and_unicode() {
        for s in ["", "x", "hello world", "héllo, 世界! + x_1", "\n\t\"quoted\""] {
            assert_eq!(roundtrip(&Tree::Str(s.to_string())), Tree::Str(s.to_string()));
        }
    }

    #[test]
    fn nested_node_with_every_field_kind_roundtrips() {
        let t = Tree::Node {
            tag: "root".into(),
            flag: true,
            ratio: 3.5,
            kids: vec![
                Tree::Leaf(1),
                Tree::Str("two".into()),
                Tree::Node {
                    tag: "inner".into(),
                    flag: false,
                    ratio: -0.25,
                    kids: vec![],
                    boxed: Box::new(Tree::Empty),
                },
            ],
            boxed: Box::new(Tree::Leaf(-7)),
        };
        assert_eq!(roundtrip(&t), t);
    }

    #[test]
    fn empty_list_and_nullary_variant_roundtrip() {
        assert_eq!(roundtrip(&Tree::Empty), Tree::Empty);
        let t = Tree::Node { tag: "".into(), flag: false, ratio: 0.0, kids: vec![], boxed: Box::new(Tree::Empty) };
        assert_eq!(roundtrip(&t), t);
    }

    #[test]
    fn deep_recursion_roundtrips() {
        let mut t = Tree::Leaf(0);
        for i in 1..500 {
            t = Tree::Node { tag: format!("n{i}"), flag: i % 2 == 0, ratio: i as f64, kids: vec![Tree::Leaf(i)], boxed: Box::new(t) };
        }
        assert_eq!(roundtrip(&t), t);
    }

    #[test]
    fn logos_seq_roundtrips_and_matches_vec_bytes() {
        use crate::types::LogosSeq;
        let elems = vec![1i64, -2, 3, 0, i64::MAX];
        let seq = LogosSeq::from_vec(elems.clone());

        let mut seq_bytes = Vec::new();
        seq.wire_encode(&mut seq_bytes);
        let mut vec_bytes = Vec::new();
        elems.wire_encode(&mut vec_bytes);
        assert_eq!(seq_bytes, vec_bytes, "LogosSeq and Vec must share the T_LIST byte format");

        let mut pos = 0usize;
        let back = LogosSeq::<i64>::wire_decode(&seq_bytes, &mut pos).expect("decode");
        assert_eq!(pos, seq_bytes.len());
        assert_eq!(back.to_vec(), elems);

        // empty
        let empty = LogosSeq::<i64>::from_vec(vec![]);
        let mut b = Vec::new();
        empty.wire_encode(&mut b);
        let mut p = 0usize;
        assert_eq!(LogosSeq::<i64>::wire_decode(&b, &mut p).unwrap().to_vec(), Vec::<i64>::new());
    }

    #[test]
    fn truncated_input_returns_none_never_panics() {
        let mut out = Vec::new();
        Tree::Leaf(123456).wire_encode(&mut out);
        for cut in 0..out.len() {
            let mut pos = 0usize;
            let _ = Tree::wire_decode(&out[..cut], &mut pos); // must not panic
        }
    }
}
