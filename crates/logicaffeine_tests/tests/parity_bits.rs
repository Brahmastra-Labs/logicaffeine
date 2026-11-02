//! Stdlib-parity north-star corpus — domain: bits
//!
//! Bit-manipulation capabilities of the C/Python/Rust/Go (+Java) superset, realized in pure
//! LOGOS (`bits.lg`) over the native bitwise operators and proven byte-identical on every
//! execution tier. The width-parametric forms subsume Go's sized `*8/*16/*32/*64` variants.
//! To LAND a capability: implement it, give it a real `assert_all_tiers` body, DELETE the
//! `#[ignore]` line, flip the catalog entry to `status: green`, bump GREEN_FLOOR.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Population count — number of set bits. Subsumes Go OnesCount/8/16/32/64.
/// provenance — go: math/bits.OnesCount · python: int.bit_count · rust: u64.count_ones · java: java.lang.Long.bitCount
/// LOGOS surface: `popcount(x)`
#[test]
fn parity_bits_popcount() {
    assert_all_tiers(
        "## Main\nShow popcount(7).\nShow popcount(255).\nShow popcount(0).\nShow popcount(1024).\n",
        "3\n8\n0\n1",
    );
}

/// The guarded fast path (native `count_ones`/`leading_zeros`/`trailing_zeros` built-ins) covers the
/// whole word range `0..=i64::MAX`: the boundary value 9223372036854775807 has 63 set bits and bit-length
/// 63; 0 is the low end (popcount 0, bitLength 0). trailingZeros(8)=3 via the built-in, and x=0 still
/// returns `width`. Locks that the built-in fast path agrees with the arbitrary-precision loop it replaced.
#[test]
fn parity_bits_fastpath_word_range() {
    assert_all_tiers(
        "## Main\nShow popcount(9223372036854775807).\nShow bitLength(9223372036854775807).\nShow popcount(0).\nShow bitLength(0).\nShow trailingZeros(8, 32).\nShow trailingZeros(0, 32).\n",
        "63\n63\n0\n0\n3\n32",
    );
}

/// Minimum number of bits to represent x (0 for x=0). Subsumes Go Len/8/16/32/64.
/// provenance — go: math/bits.Len · python: int.bit_length · rust: u64.ilog2 · java: java.lang.Long.numberOfLeadingZeros
/// LOGOS surface: `bitLength(x)`
#[test]
fn parity_bits_bit_length() {
    assert_all_tiers(
        "## Main\nShow bitLength(255).\nShow bitLength(256).\nShow bitLength(0).\nShow bitLength(1).\n",
        "8\n9\n0\n1",
    );
}

/// Leading zero count within a bit width. Subsumes Go LeadingZeros/8/16/32/64.
/// provenance — go: math/bits.LeadingZeros · rust: u32.leading_zeros · java: java.lang.Integer.numberOfLeadingZeros
/// LOGOS surface: `leadingZeros(x, width)`
#[test]
fn parity_bits_leading_zeros() {
    assert_all_tiers(
        "## Main\nShow leadingZeros(1, 8).\nShow leadingZeros(255, 8).\nShow leadingZeros(0, 8).\n",
        "7\n0\n8",
    );
}

/// Trailing zero count (returns width for 0). Subsumes Go TrailingZeros/8/16/32/64.
/// provenance — go: math/bits.TrailingZeros · rust: u32.trailing_zeros · java: java.lang.Integer.numberOfTrailingZeros
/// LOGOS surface: `trailingZeros(x, width)`
#[test]
fn parity_bits_trailing_zeros() {
    assert_all_tiers(
        "## Main\nShow trailingZeros(8, 8).\nShow trailingZeros(1, 8).\nShow trailingZeros(0, 8).\n",
        "3\n0\n8",
    );
}

/// Reverse the low `width` bits. Subsumes Go Reverse/8/16/32/64.
/// provenance — go: math/bits.Reverse · rust: u8.reverse_bits · java: java.lang.Integer.reverse
/// LOGOS surface: `reverseBits(x, width)`
#[test]
fn parity_bits_reverse_bits() {
    assert_all_tiers(
        "## Main\nShow reverseBits(1, 8).\nShow reverseBits(128, 8).\nShow reverseBits(3, 8).\n",
        "128\n1\n192",
    );
}

/// Reverse the bytes of a width-bit value. Subsumes Go ReverseBytes/16/32/64.
/// provenance — go: math/bits.ReverseBytes · rust: u32.swap_bytes · java: java.lang.Integer.reverseBytes
/// LOGOS surface: `reverseBytes(x, width)`
#[test]
fn parity_bits_reverse_bytes() {
    assert_all_tiers(
        "## Main\nShow reverseBytes(258, 16).\nShow reverseBytes(1, 16).\n",
        "513\n256",
    );
}

/// Rotate the low `width` bits left by n. Subsumes Go RotateLeft/8/16/32/64.
/// provenance — go: math/bits.RotateLeft · rust: u8.rotate_left · java: java.lang.Integer.rotateLeft
/// LOGOS surface: `rotateLeft(x, n, width)`
#[test]
fn parity_bits_rotate_left() {
    assert_all_tiers(
        "## Main\nShow rotateLeft(1, 1, 8).\nShow rotateLeft(128, 1, 8).\nShow rotateLeft(1, 0, 8).\n",
        "2\n1\n1",
    );
}

/// Rotate the low `width` bits right by n (RotateLeft with negative count).
/// provenance — rust: u8.rotate_right · java: java.lang.Integer.rotateRight
/// LOGOS surface: `rotateRight(x, n, width)`
#[test]
fn parity_bits_rotate_right() {
    assert_all_tiers(
        "## Main\nShow rotateRight(1, 1, 8).\nShow rotateRight(2, 1, 8).\n",
        "128\n1",
    );
}
