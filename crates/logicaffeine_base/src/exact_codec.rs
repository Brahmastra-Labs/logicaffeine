//! Canonical byte encodings of the exact numbers — the one form they cross a task boundary, the
//! wire and storage in.
//!
//! Every exact type writes its canonical name: an integer its sign and magnitude, a rational its
//! reduced numerator and denominator, a real algebraic number its minimal polynomial and root
//! index, a hyperreal its reduced numerator and denominator in `dt`, an infinite natural its
//! residue-indexed polynomials, an ordinal its Cantor normal form, a computable real its exact form
//! or the expression that builds it. Lengths are LEB128 varints.
//!
//! A decoder trusts nothing it reads. [`ExactCodec::from_bytes`] accepts an encoding only when it
//! is complete, has no trailing bytes, and is the canonical encoding of the value it decodes to —
//! so a forged non-canonical name (an unreduced fraction, a reducible "minimal polynomial", a
//! non-real exponential sum) is refused rather than breaking the equality every other part of the
//! system relies on. Sizes are bounded before any work is done: a minimal polynomial's degree by
//! [`MAX_WIRE_DEGREE`], collection lengths by [`MAX_WIRE_LEN`], magnitudes by
//! [`MAX_WIRE_MAGNITUDE`] bytes, and nesting by [`MAX_WIRE_DEPTH`].

use crate::describe::{read_uvarint, write_uvarint};
use crate::numeric::{BigInt, Rational};

/// The largest minimal-polynomial degree a decoder accepts: an irreducibility check is a
/// factorization, and this bound keeps a hostile encoding from demanding an expensive one.
pub const MAX_WIRE_DEGREE: usize = 32;
/// The most elements a decoded collection may claim.
pub const MAX_WIRE_LEN: usize = 1 << 16;
/// The most magnitude bytes a decoded integer may claim.
pub const MAX_WIRE_MAGNITUDE: usize = 1 << 20;
/// The deepest nesting a decoded ordinal or expression may have.
pub const MAX_WIRE_DEPTH: usize = 256;

/// A canonical, self-delimiting byte encoding. See the module documentation.
pub trait ExactCodec: Sized {
    fn write(&self, out: &mut Vec<u8>);
    fn read(buf: &[u8], pos: &mut usize) -> Option<Self>;

    fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write(&mut out);
        out
    }

    /// The value `buf` canonically encodes, with nothing left over.
    fn from_bytes(buf: &[u8]) -> Option<Self> {
        let mut pos = 0;
        let v = Self::read(buf, &mut pos)?;
        (pos == buf.len() && v.to_bytes() == buf).then_some(v)
    }
}

/// One byte.
pub fn read_byte(buf: &[u8], pos: &mut usize) -> Option<u8> {
    let b = *buf.get(*pos)?;
    *pos += 1;
    Some(b)
}

/// A length, at most `bound`.
pub fn read_len(buf: &[u8], pos: &mut usize, bound: usize) -> Option<usize> {
    let n = read_uvarint(buf, pos)?;
    (n as usize <= bound).then_some(n as usize)
}

pub fn write_len(n: usize, out: &mut Vec<u8>) {
    write_uvarint(n as u64, out);
}

/// A length-prefixed list.
pub fn write_list<T: ExactCodec>(items: &[T], out: &mut Vec<u8>) {
    write_len(items.len(), out);
    for item in items {
        item.write(out);
    }
}

pub fn read_list<T: ExactCodec>(buf: &[u8], pos: &mut usize) -> Option<Vec<T>> {
    let n = read_len(buf, pos, MAX_WIRE_LEN)?;
    let mut out = Vec::with_capacity(n.min(64));
    for _ in 0..n {
        out.push(T::read(buf, pos)?);
    }
    Some(out)
}

impl ExactCodec for u64 {
    fn write(&self, out: &mut Vec<u8>) {
        write_uvarint(*self, out);
    }

    fn read(buf: &[u8], pos: &mut usize) -> Option<Self> {
        read_uvarint(buf, pos)
    }
}

impl ExactCodec for i64 {
    fn write(&self, out: &mut Vec<u8>) {
        write_uvarint(crate::describe::zigzag(*self), out);
    }

    fn read(buf: &[u8], pos: &mut usize) -> Option<Self> {
        read_uvarint(buf, pos).map(crate::describe::unzigzag)
    }
}

impl ExactCodec for BigInt {
    /// A sign byte (`0` nonnegative, `1` negative), the magnitude's length, its little-endian bytes.
    fn write(&self, out: &mut Vec<u8>) {
        let (negative, magnitude) = self.to_le_bytes();
        out.push(u8::from(negative));
        write_len(magnitude.len(), out);
        out.extend_from_slice(&magnitude);
    }

    fn read(buf: &[u8], pos: &mut usize) -> Option<Self> {
        let negative = match read_byte(buf, pos)? {
            0 => false,
            1 => true,
            _ => return None,
        };
        let n = read_len(buf, pos, MAX_WIRE_MAGNITUDE)?;
        let bytes = buf.get(*pos..*pos + n)?;
        *pos += n;
        Some(BigInt::from_le_bytes(negative, bytes))
    }
}

impl ExactCodec for Rational {
    /// The reduced numerator, then the positive denominator.
    fn write(&self, out: &mut Vec<u8>) {
        self.numerator().write(out);
        self.denominator().write(out);
    }

    fn read(buf: &[u8], pos: &mut usize) -> Option<Self> {
        let num = BigInt::read(buf, pos)?;
        let den = BigInt::read(buf, pos)?;
        if !den.is_positive() {
            return None;
        }
        Rational::new(num, den)
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/exact_codec/tests.rs"]
mod tests;
