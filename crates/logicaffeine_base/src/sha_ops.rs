//! The four Intel SHA-1 (SHA-NI) operations, in SOFTWARE — the exact bit-for-bit semantics of
//! `sha1rnds4` / `sha1msg1` / `sha1msg2` / `sha1nexte`. These are the *spec* the tree-walker runs so
//! that SHA-1 written in LOGOS over these ops produces the identical result whether it is interpreted
//! (software here) or AOT-compiled to the real hardware instruction (`core::arch::x86_64`). Same idea
//! as the scalar-lane spec behind `Lanes8Word32` → AVX2.
//!
//! A 128-bit value is a `[u32; 4]` in LANE order: index `i` is lanes bits `[32i+31 : 32i]`, so index 0
//! is the low dword (`SRC[31:0]`) and index 3 is the high dword (`SRC[127:96]`) — matching how
//! `_mm_loadu_si128`/`_mm_storeu_si128` move an array to/from an `__m128i`. The tests below load the
//! same array into an `__m128i` and assert the software op equals the hardware intrinsic on random
//! inputs, so the spec is validated against silicon, not just against itself.

#[inline]
fn ch(b: u32, c: u32, d: u32) -> u32 {
    (b & c) | (!b & d)
}
#[inline]
fn parity(b: u32, c: u32, d: u32) -> u32 {
    b ^ c ^ d
}
#[inline]
fn maj(b: u32, c: u32, d: u32) -> u32 {
    (b & c) | (b & d) | (c & d)
}

/// `sha1rnds4(abcd, msg, func)` — four rounds of SHA-1. `abcd` is the working state (A in lane 3 …
/// D in lane 0), `msg` the four message dwords with the round's E already folded into its high dword
/// (via [`sha1nexte`] / the initial add), and `func` ∈ 0..=3 selects the round function + constant
/// (0 = Ch/K0, 1 = Parity/K1, 2 = Maj/K2, 3 = Parity/K3). Returns the new state.
pub fn sha1rnds4(abcd: [u32; 4], msg: [u32; 4], func: u32) -> [u32; 4] {
    let (k, f): (u32, fn(u32, u32, u32) -> u32) = match func & 3 {
        0 => (0x5A82_7999, ch),
        1 => (0x6ED9_EBA1, parity),
        2 => (0x8F1B_BCDC, maj),
        _ => (0xCA62_C1D6, parity),
    };
    // A = SRC1[127:96] = lane 3, … D = SRC1[31:0] = lane 0. Message W0 = SRC2[127:96] = lane 3.
    let mut a = abcd[3];
    let mut b = abcd[2];
    let mut c = abcd[1];
    let mut d = abcd[0];
    let w = [msg[3], msg[2], msg[1], msg[0]];
    // Round 0's E is folded into W0 (msg high dword); subsequent rounds carry E through the state.
    let mut e = 0u32;
    for &wi in &w {
        let t = f(b, c, d)
            .wrapping_add(a.rotate_left(5))
            .wrapping_add(wi)
            .wrapping_add(k)
            .wrapping_add(e);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = t;
    }
    // DEST[127:96] = A (lane 3) … DEST[31:0] = D (lane 0).
    [d, c, b, a]
}

/// `sha1msg1(a, b)` — the first half of the message-schedule recurrence (the XOR mixing before the
/// rotate). `dest = { W2^W0, W3^W1, W4^W2, W5^W3 }` per Intel, in lane order.
pub fn sha1msg1(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    // W0=a[3],W1=a[2],W2=a[1],W3=a[0]; W4=b[3],W5=b[2].
    [
        b[2] ^ a[0], // DEST[31:0]  = W5 ^ W3
        b[3] ^ a[1], // DEST[63:32] = W4 ^ W2
        a[0] ^ a[2], // DEST[95:64] = W3 ^ W1
        a[1] ^ a[3], // DEST[127:96]= W2 ^ W0
    ]
}

/// `sha1msg2(a, b)` — completes the schedule: the final XOR with the previous words and the ROL-1,
/// including the intra-vector dependency of W19 on W16.
pub fn sha1msg2(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    // W13=b[2],W14=b[1],W15=b[0].
    let w16 = (a[3] ^ b[2]).rotate_left(1); // DEST[127:96]
    let w17 = (a[2] ^ b[1]).rotate_left(1); // DEST[95:64]
    let w18 = (a[1] ^ b[0]).rotate_left(1); // DEST[63:32]
    let w19 = (a[0] ^ w16).rotate_left(1); // DEST[31:0], depends on W16
    [w19, w18, w17, w16]
}

/// `sha1nexte(a, b)` — fold the next E (the previous block-round's A, rotated) into the high dword of
/// the next message group. `dest[127:96] = b[127:96] + (a[127:96] ROL 30)`; the low 96 bits pass `b`.
pub fn sha1nexte(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    [b[0], b[1], b[2], b[3].wrapping_add(a[3].rotate_left(30))]
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/sha_ops/tests.rs"]
mod tests;
