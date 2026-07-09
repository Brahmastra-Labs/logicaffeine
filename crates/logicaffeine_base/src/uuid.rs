//! UUID — a 128-bit universally-unique identifier (RFC 9562, superseding RFC 4122).
//!
//! Rolled in-house rather than wrapping a crate: the campaign builds first-class types, and owning the
//! implementation is what lets us be byte-exact across every tier and benchmark it head-to-head. The
//! value is a fixed `[u8; 16]` in network (big-endian) byte order, so it is `Copy`, `Ord` by bytes
//! (which makes v6/v7 time-ordered ids sort chronologically), and hashes cheaply.
//!
//! Every standard version is here:
//! - **nil** (all-zero) and **max** (all-one) — the two special ids.
//! - **v1** (gregorian time + node) and **v6** (the same fields reordered to sort by time).
//! - **v3** (MD5 of namespace ‖ name) and **v5** (SHA-1 of namespace ‖ name) — name-based, stable.
//! - **v4** (random) and **v7** (Unix-millis time-ordered + random) — the two you reach for today.
//! - **v8** (free-form / vendor-defined) — arbitrary bytes with the version+variant bits stamped.
//!
//! Generation takes the entropy/time as parameters (no ambient clock or RNG here) so it is pure and
//! deterministic — the higher tiers seed it for byte-identical cross-tier output. Validated against
//! the `uuid` crate (the name-based and random/time builders, parse, and display) in the tests.

use core::fmt;

use crate::hash::{md5, sha1};

/// A 128-bit UUID, stored big-endian (RFC 9562 byte order).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uuid([u8; 16]);

/// Build `namespace ‖ name` and hand the slice to `f`, WITHOUT a heap allocation for any real name.
/// A 240-byte inline stack buffer covers every practical v3/v5 input (DNS names, URLs, OIDs are far
/// shorter); only a pathologically long name falls back to a `Vec`. The 16 namespace bytes always fit.
#[inline]
fn with_namespaced_input<R>(namespace: Uuid, name: &[u8], f: impl FnOnce(&[u8]) -> R) -> R {
    const INLINE: usize = 256;
    let total = 16 + name.len();
    if total <= INLINE {
        let mut buf = [0u8; INLINE];
        buf[..16].copy_from_slice(&namespace.0);
        buf[16..total].copy_from_slice(name);
        f(&buf[..total])
    } else {
        let mut input = Vec::with_capacity(total);
        input.extend_from_slice(&namespace.0);
        input.extend_from_slice(name);
        f(&input)
    }
}

/// The RFC-defined variant of a UUID — which layout the variant/version bits follow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variant {
    /// Reserved, NCS backward compatibility (`0xx`).
    Ncs,
    /// The standard RFC 9562 / RFC 4122 layout (`10x`) — every version we generate.
    Rfc4122,
    /// Reserved, Microsoft GUID (`110`).
    Microsoft,
    /// Reserved for future definition (`111`).
    Future,
}

impl Uuid {
    /// The nil UUID — all 128 bits zero (`00000000-0000-0000-0000-000000000000`).
    pub const NIL: Uuid = Uuid([0u8; 16]);
    /// The max UUID — all 128 bits one (`ffffffff-ffff-ffff-ffff-ffffffffffff`), RFC 9562 §5.10.
    pub const MAX: Uuid = Uuid([0xFFu8; 16]);

    /// Namespace ID for fully-qualified domain names (RFC 9562 Appendix A).
    pub const NAMESPACE_DNS: Uuid = Uuid([
        0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
    ]);
    /// Namespace ID for URLs.
    pub const NAMESPACE_URL: Uuid = Uuid([
        0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
    ]);
    /// Namespace ID for ISO OIDs.
    pub const NAMESPACE_OID: Uuid = Uuid([
        0x6b, 0xa7, 0xb8, 0x12, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
    ]);
    /// Namespace ID for X.500 DNs.
    pub const NAMESPACE_X500: Uuid = Uuid([
        0x6b, 0xa7, 0xb8, 0x14, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
    ]);

    /// Wrap raw big-endian bytes verbatim (no version/variant stamping).
    pub const fn from_bytes(bytes: [u8; 16]) -> Uuid {
        Uuid(bytes)
    }

    /// The raw 16 big-endian bytes.
    pub const fn to_bytes(self) -> [u8; 16] {
        self.0
    }

    /// Borrow the raw 16 big-endian bytes.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// True for the all-zero nil UUID.
    pub fn is_nil(&self) -> bool {
        self.0 == [0u8; 16]
    }

    /// True for the all-one max UUID.
    pub fn is_max(&self) -> bool {
        self.0 == [0xFFu8; 16]
    }

    /// The version number (the high nibble of byte 6): 1–8 for the defined versions, 0 for nil,
    /// 0xF for max. A pure read of the field — it does not assert the id was generated correctly.
    pub fn version(&self) -> u8 {
        self.0[6] >> 4
    }

    /// The variant (the high bits of byte 8). Everything we generate is [`Variant::Rfc4122`].
    pub fn variant(&self) -> Variant {
        let b = self.0[8];
        if b & 0x80 == 0x00 {
            Variant::Ncs
        } else if b & 0xC0 == 0x80 {
            Variant::Rfc4122
        } else if b & 0xE0 == 0xC0 {
            Variant::Microsoft
        } else {
            Variant::Future
        }
    }

    /// Stamp the version nibble (byte 6) and the RFC-4122 variant bits (byte 8). Done as one `u128`
    /// (register-resident) rather than two byte-indexed writes, so the value never spills to the stack:
    /// byte 6's high nibble is bits `[76..80)` big-endian, byte 8's top two bits are `[62..64)`.
    #[inline]
    fn stamp(bytes: [u8; 16], version: u8) -> Uuid {
        let mut v = u128::from_be_bytes(bytes);
        v = (v & !(0xFu128 << 76)) | ((version as u128) << 76);
        v = (v & !(0b11u128 << 62)) | (0b10u128 << 62);
        Uuid(v.to_be_bytes())
    }

    /// Version 1: a 60-bit gregorian timestamp (100-ns ticks since 1582-10-15), a 14-bit clock
    /// sequence, and a 48-bit node id. Layout is little-end-first on time (not time-sortable; that is
    /// what v6 fixes).
    pub fn new_v1(timestamp_100ns: u64, clock_seq: u16, node: [u8; 6]) -> Uuid {
        let ts = timestamp_100ns & 0x0FFF_FFFF_FFFF_FFFF;
        let time_low = (ts & 0xFFFF_FFFF) as u32;
        let time_mid = ((ts >> 32) & 0xFFFF) as u16;
        let time_hi = ((ts >> 48) & 0x0FFF) as u16;
        let mut b = [0u8; 16];
        b[0..4].copy_from_slice(&time_low.to_be_bytes());
        b[4..6].copy_from_slice(&time_mid.to_be_bytes());
        b[6..8].copy_from_slice(&time_hi.to_be_bytes());
        b[8] = (clock_seq >> 8) as u8;
        b[9] = (clock_seq & 0xFF) as u8;
        b[10..16].copy_from_slice(&node);
        Uuid::stamp(b, 1)
    }

    /// Version 6: the v1 fields with the timestamp reordered most-significant-first, so byte order
    /// sorts chronologically (RFC 9562 §5.6) — a drop-in, sortable replacement for v1.
    pub fn new_v6(timestamp_100ns: u64, clock_seq: u16, node: [u8; 6]) -> Uuid {
        let ts = timestamp_100ns & 0x0FFF_FFFF_FFFF_FFFF;
        let time_high = ((ts >> 28) & 0xFFFF_FFFF) as u32;
        let time_mid = ((ts >> 12) & 0xFFFF) as u16;
        let time_low = (ts & 0x0FFF) as u16;
        let mut b = [0u8; 16];
        b[0..4].copy_from_slice(&time_high.to_be_bytes());
        b[4..6].copy_from_slice(&time_mid.to_be_bytes());
        b[6..8].copy_from_slice(&time_low.to_be_bytes());
        b[8] = (clock_seq >> 8) as u8;
        b[9] = (clock_seq & 0xFF) as u8;
        b[10..16].copy_from_slice(&node);
        Uuid::stamp(b, 6)
    }

    /// Version 3: MD5 of `namespace` bytes followed by `name` (RFC 9562 §5.3). Name-based and stable —
    /// the same namespace+name always yields the same id. This is the native REFERENCE ORACLE: the
    /// language's `uuid_v3` is written in Logos (`assets/std/uuid.lg`, over the Logos `md5Digest`) and is
    /// proven byte-exact against this and the `uuid`/`md-5` crates — it is not on any language path.
    pub fn new_v3(namespace: Uuid, name: &[u8]) -> Uuid {
        with_namespaced_input(namespace, name, |input| Uuid::stamp(md5(input), 3))
    }

    /// Version 5: SHA-1 of `namespace` bytes followed by `name`, truncated to 16 bytes (RFC 9562
    /// §5.5). Name-based and stable; preferred over v3. The native REFERENCE ORACLE for the Logos
    /// `uuid_v5` (uuid.lg, over `sha1Digest`) — see [`Uuid::new_v3`].
    pub fn new_v5(namespace: Uuid, name: &[u8]) -> Uuid {
        with_namespaced_input(namespace, name, |input| {
            let digest = sha1(input);
            let mut b = [0u8; 16];
            b.copy_from_slice(&digest[..16]);
            Uuid::stamp(b, 5)
        })
    }

    /// Version 4: 122 bits of supplied randomness (RFC 9562 §5.4). The 6 version/variant bits are
    /// overwritten, so all 16 bytes of entropy may be passed.
    #[inline]
    pub fn new_v4(random: [u8; 16]) -> Uuid {
        Uuid::stamp(random, 4)
    }

    /// Version 7: a 48-bit big-endian Unix-millisecond timestamp followed by 74 bits of randomness
    /// (RFC 9562 §5.7). Time-ordered (byte order sorts by creation time) — the modern default.
    #[inline]
    pub fn new_v7(unix_ms: u64, random: [u8; 10]) -> Uuid {
        let mut b = [0u8; 16];
        let ms = unix_ms & 0xFFFF_FFFF_FFFF;
        b[0..6].copy_from_slice(&ms.to_be_bytes()[2..8]);
        b[6..16].copy_from_slice(&random);
        Uuid::stamp(b, 7)
    }

    /// Version 8: vendor/experimental — the 16 bytes are taken as given, with only the version and
    /// variant bits stamped (RFC 9562 §5.8).
    pub fn new_v8(bytes: [u8; 16]) -> Uuid {
        Uuid::stamp(bytes, 8)
    }

    /// Parse a UUID from text. Accepts the canonical hyphenated form, the 32-hex simple form, the
    /// braced `{…}` form, and the `urn:uuid:…` form; case-insensitive. Returns `None` on any other
    /// shape, a bad length, or a non-hex digit — never panics.
    pub fn parse(input: &str) -> Option<Uuid> {
        let s = input.trim();
        // Fast path: the canonical 36-char hyphenated form — the overwhelmingly common case. A
        // byte-table hex decode with no per-char branching; any invalid digit poisons `bad` to ≥16.
        if s.len() == 36 {
            // Fast path: the canonical 36-char hyphenated form — the overwhelmingly common case.
            return Self::parse_canonical(s.as_bytes().try_into().unwrap());
        }
        // Strip an optional `urn:uuid:` prefix (case-insensitive) and `{…}` braces.
        let s = s.strip_prefix("urn:uuid:").or_else(|| s.strip_prefix("URN:UUID:")).unwrap_or(s);
        let s = s.strip_prefix('{').and_then(|t| t.strip_suffix('}')).unwrap_or(s);

        // Collect exactly the hex digits, rejecting any non-hyphen, non-hex character, and require the
        // hyphens (when present) to sit at the canonical 8-4-4-4-12 positions.
        let bytes = s.as_bytes();
        let mut hex = [0u8; 32];
        let mut n = 0usize;
        match bytes.len() {
            36 => {
                // Canonical: hyphens at indices 8, 13, 18, 23.
                for (i, &c) in bytes.iter().enumerate() {
                    if matches!(i, 8 | 13 | 18 | 23) {
                        if c != b'-' {
                            return None;
                        }
                    } else {
                        if n >= 32 {
                            return None;
                        }
                        hex[n] = c;
                        n += 1;
                    }
                }
            }
            32 => hex.copy_from_slice(bytes),
            _ => return None,
        }
        if n != 0 && n != 32 {
            return None;
        }

        let mut out = [0u8; 16];
        for i in 0..16 {
            let hi = hex_val(hex[2 * i])?;
            let lo = hex_val(hex[2 * i + 1])?;
            out[i] = (hi << 4) | lo;
        }
        Some(Uuid(out))
    }

    /// Decode one canonical 36-byte hyphenated record — the shared fast path behind [`Uuid::parse`] and
    /// [`Uuid::parse_many`]. Checks the four hyphens, then the SIMD `pshufb` decode (statically dispatched
    /// under `target-cpu=native`, runtime-detected otherwise, scalar-table fallback). `None` if malformed.
    #[inline]
    fn parse_canonical(arr: &[u8; 36]) -> Option<Uuid> {
        if arr[8] != b'-' || arr[13] != b'-' || arr[18] != b'-' || arr[23] != b'-' {
            return None;
        }
        #[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))]
        {
            return unsafe { x86_hex::parse_hyphenated(arr) }.map(Uuid);
        }
        #[cfg(all(target_arch = "x86_64", not(target_feature = "ssse3")))]
        {
            if x86_hex::available() {
                return unsafe { x86_hex::parse_hyphenated(arr) }.map(Uuid);
            }
        }
        #[cfg(not(all(target_arch = "x86_64", target_feature = "ssse3")))]
        {
            let mut out = [0u8; 16];
            let mut bad = 0u8;
            // The 16 byte positions: pairs of hex indices, skipping the four hyphens.
            const PAIRS: [usize; 16] = [0, 2, 4, 6, 9, 11, 14, 16, 19, 21, 24, 26, 28, 30, 32, 34];
            for (i, &p) in PAIRS.iter().enumerate() {
                let hi = HEX_DECODE[arr[p] as usize];
                let lo = HEX_DECODE[arr[p + 1] as usize];
                bad |= hi | lo;
                out[i] = (hi << 4) | (lo & 0x0f);
            }
            if bad < 16 {
                Some(Uuid(out))
            } else {
                None
            }
        }
    }

    /// Parse many canonical 36-char UUIDs packed back-to-back (`36·n` bytes) into a `Vec<Uuid>` — the
    /// bulk read path (a DB column / log-ingest stream). Loops the SIMD decode core directly with no
    /// per-id `trim`/dispatch overhead, into a single pre-sized allocation. `None` if the length isn't a
    /// multiple of 36 or any record is malformed. Pairs with [`encode_many`](encode_many) (the bulk write path).
    pub fn parse_many(packed: &[u8]) -> Option<Vec<Uuid>> {
        if packed.is_empty() || packed.len() % 36 != 0 {
            return None;
        }
        #[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))]
        {
            return unsafe { x86_hex::parse_batch(packed) };
        }
        #[cfg(all(target_arch = "x86_64", not(target_feature = "ssse3")))]
        {
            if x86_hex::available() {
                return unsafe { x86_hex::parse_batch(packed) };
            }
        }
        #[cfg(not(all(target_arch = "x86_64", target_feature = "ssse3")))]
        {
            let n = packed.len() / 36;
            let mut out = Vec::with_capacity(n);
            for chunk in packed.chunks_exact(36) {
                let rec: &[u8; 36] = chunk.try_into().unwrap();
                out.push(Self::parse_canonical(rec)?);
            }
            Some(out)
        }
    }
}

/// `HEX_DECODE[c]` is the nibble value (0–15) of ASCII hex char `c`, or `0xFF` (≥16) for any
/// non-hex byte — so a single `|`-accumulation over a run detects an invalid digit branch-free.
const HEX_DECODE: [u8; 256] = {
    let mut t = [0xFFu8; 256];
    let mut i = 0u8;
    while i < 10 {
        t[(b'0' + i) as usize] = i;
        i += 1;
    }
    let mut i = 0u8;
    while i < 6 {
        t[(b'a' + i) as usize] = 10 + i;
        t[(b'A' + i) as usize] = 10 + i;
        i += 1;
    }
    t
};

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// `HEX_PAIRS[b]` is the two lowercase ASCII hex chars of byte `b`, packed — one table lookup per
/// byte instead of two nibble lookups.
const HEX_PAIRS: [[u8; 2]; 256] = {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut t = [[0u8; 2]; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = [HEX[i >> 4], HEX[i & 0xf]];
        i += 1;
    }
    t
};

/// Write a UUID's canonical 36-byte text into a caller buffer (no allocation) — the alloc-free hot
/// path for `Display` and for BULK formatting (fill one big buffer, zero per-id allocations). Uses an
/// SSSE3 `pshufb` nibble→hex encode when available, else the byte-pair table.
#[inline]
pub fn encode_canonical(bytes: &[u8; 16], buf: &mut [u8; 36]) {
    // Under `target-cpu=native` the SSSE3 detect is compiled away and this is a direct SIMD call.
    #[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))]
    {
        // SAFETY: ssse3 is statically enabled for this build.
        return unsafe { x86_hex::encode_canonical(bytes, buf) };
    }
    #[cfg(all(target_arch = "x86_64", not(target_feature = "ssse3")))]
    {
        if x86_hex::available() {
            // SAFETY: guarded by the ssse3 feature detection.
            unsafe { x86_hex::encode_canonical(bytes, buf) };
            return;
        }
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "ssse3")))]
    encode_canonical_scalar(bytes, buf);
}

/// Portable byte-pair-table encode (the fallback / non-x86 path).
#[inline]
fn encode_canonical_scalar(bytes: &[u8; 16], buf: &mut [u8; 36]) {
    let mut j = 0;
    let mut write = |buf: &mut [u8; 36], range: std::ops::Range<usize>, j: &mut usize| {
        for &byte in &bytes[range] {
            let pair = HEX_PAIRS[byte as usize];
            buf[*j] = pair[0];
            buf[*j + 1] = pair[1];
            *j += 2;
        }
    };
    write(buf, 0..4, &mut j);
    buf[j] = b'-';
    j += 1;
    write(buf, 4..6, &mut j);
    buf[j] = b'-';
    j += 1;
    write(buf, 6..8, &mut j);
    buf[j] = b'-';
    j += 1;
    write(buf, 8..10, &mut j);
    buf[j] = b'-';
    j += 1;
    write(buf, 10..16, &mut j);
}

/// SSSE3 hex encode for a UUID: one `pshufb` turns 16 nibbles into 16 hex chars, done twice
/// (high/low), interleaved with `punpck`, then scattered into the `8-4-4-4-12` layout.
#[cfg(target_arch = "x86_64")]
mod x86_hex {
    /// The SSSE3 (`pshufb`) instruction this encode needs; cached by std after the first call.
    #[inline]
    pub fn available() -> bool {
        std::is_x86_feature_detected!("ssse3")
    }

    #[target_feature(enable = "ssse3,sse2")]
    pub unsafe fn encode_canonical(bytes: &[u8; 16], buf: &mut [u8; 36]) {
        use core::arch::x86_64::*;
        let v = _mm_loadu_si128(bytes.as_ptr() as *const __m128i);
        let mask0f = _mm_set1_epi8(0x0f);
        // Per byte: high nibble = (byte >> 4) & 0x0f, low nibble = byte & 0x0f.
        let hi = _mm_and_si128(_mm_srli_epi16(v, 4), mask0f);
        let lo = _mm_and_si128(v, mask0f);
        let lut = _mm_setr_epi8(
            b'0' as i8, b'1' as i8, b'2' as i8, b'3' as i8, b'4' as i8, b'5' as i8, b'6' as i8,
            b'7' as i8, b'8' as i8, b'9' as i8, b'a' as i8, b'b' as i8, b'c' as i8, b'd' as i8,
            b'e' as i8, b'f' as i8,
        );
        let hi_hex = _mm_shuffle_epi8(lut, hi);
        let lo_hex = _mm_shuffle_epi8(lut, lo);
        // Interleave to (hi0,lo0,hi1,lo1,…): chars 0–15 in `h0`, 16–31 in `h1`.
        let h0 = _mm_unpacklo_epi8(hi_hex, lo_hex);
        let h1 = _mm_unpackhi_epi8(hi_hex, lo_hex);
        // SIMD scatter into the 8-4-4-4-12 layout — `pshufb` places the hex + zeros the dash slots,
        // `palignr` bridges the c14/c15 that straddle h0/h1 so out[16..32] is one shuffle, dashes OR'd
        // in. Two 16-byte stores + a 4-byte tail; measured ~1.23× the per-range `copy_from_slice`.
        let mask_lo = _mm_setr_epi8(0, 1, 2, 3, 4, 5, 6, 7, -128, 8, 9, 10, 11, -128, 12, 13);
        let dash_lo = _mm_setr_epi8(0, 0, 0, 0, 0, 0, 0, 0, 0x2d, 0, 0, 0, 0, 0x2d, 0, 0);
        let out_lo = _mm_or_si128(_mm_shuffle_epi8(h0, mask_lo), dash_lo);
        let mid = _mm_alignr_epi8(h1, h0, 14); // [c14, c15, c16..c29]
        let mask_mid = _mm_setr_epi8(0, 1, -128, 2, 3, 4, 5, -128, 6, 7, 8, 9, 10, 11, 12, 13);
        let dash_mid = _mm_setr_epi8(0, 0, 0x2d, 0, 0, 0, 0, 0x2d, 0, 0, 0, 0, 0, 0, 0, 0);
        let out_mid = _mm_or_si128(_mm_shuffle_epi8(mid, mask_mid), dash_mid);
        _mm_storeu_si128(buf.as_mut_ptr() as *mut __m128i, out_lo);
        _mm_storeu_si128(buf.as_mut_ptr().add(16) as *mut __m128i, out_mid);
        let mut t = [0u8; 16];
        _mm_storeu_si128(t.as_mut_ptr() as *mut __m128i, h1);
        buf[32..36].copy_from_slice(&t[12..16]); // c28-31
    }

    /// ASCII hex chars → nibble values (0–15), one lane each: `(c & 0x0f) + 9·((c & 0x40) >> 6)`.
    #[inline]
    #[target_feature(enable = "ssse3,sse2")]
    unsafe fn nibbles(chars: core::arch::x86_64::__m128i) -> core::arch::x86_64::__m128i {
        use core::arch::x86_64::*;
        let lo = _mm_and_si128(chars, _mm_set1_epi8(0x0f));
        let hi = _mm_srli_epi16(_mm_and_si128(chars, _mm_set1_epi8(0x40)), 6); // per byte {0,1}
        let hi8 = _mm_slli_epi16(hi, 3); // per byte {0,8}
        _mm_add_epi8(_mm_add_epi8(lo, hi8), hi) // lo + 9·hi
    }

    /// Per-lane hex-validity MASK (`0xFF` where the lane is `0-9A-Fa-f`, else `0x00`), case-folded — the
    /// value form of a boolean range check, so a batch can `&`-accumulate one mask across many records
    /// and pay the single `movemask`+branch once per column instead of once per record.
    #[inline]
    #[target_feature(enable = "ssse3,sse2")]
    unsafe fn hex_ok(chars: core::arch::x86_64::__m128i) -> core::arch::x86_64::__m128i {
        use core::arch::x86_64::*;
        let cf = _mm_or_si128(chars, _mm_set1_epi8(0x20)); // fold 'A'-'F' → 'a'-'f'
        let clamp = |lo: i8, hi: i8| {
            _mm_cmpeq_epi8(_mm_min_epu8(_mm_max_epu8(cf, _mm_set1_epi8(lo)), _mm_set1_epi8(hi)), cf)
        };
        _mm_or_si128(clamp(0x30, 0x39), clamp(0x61, 0x66))
    }

    /// The branch-free decode core shared by the single and bulk parsers. Two `pshufb` gathers strip the
    /// four hyphens; `pmaddubsw` with weights `[16, 1, …]` fuses each nibble pair and `packuswb` narrows
    /// to the first 14 bytes; the last two bytes (`string[32..36]`, hyphen-free) decode with the same
    /// branchless `(c & 0xf) + 9·(c >> 6)` nibble. Writes all 16 bytes UNCONDITIONALLY (garbage for
    /// non-hex input, which the caller rejects) and RETURNS the per-lane hex-validity mask of the gather.
    #[inline]
    #[target_feature(enable = "ssse3,sse2")]
    unsafe fn decode_core(b: &[u8; 36], out: &mut [u8; 16]) -> core::arch::x86_64::__m128i {
        use core::arch::x86_64::*;
        let l0 = _mm_loadu_si128(b.as_ptr() as *const __m128i); // string[0..16]
        let l1 = _mm_loadu_si128(b.as_ptr().add(16) as *const __m128i); // string[16..32]
        // Gather each half's hex chars, dropping the hyphens. The two padding lanes reuse index 0 (a
        // real hex char), so validity never trips on them and their (discarded) output byte is safe.
        let mask_a = _mm_setr_epi8(0, 1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 14, 15, 0, 0);
        let mask_b = _mm_setr_epi8(0, 1, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15, 0, 0);
        let ra = _mm_shuffle_epi8(l0, mask_a);
        let rb = _mm_shuffle_epi8(l1, mask_b);
        // Bytes `[16, 1, 16, 1, …]` as little-endian 16-bit lanes `0x0110`.
        let weights = _mm_set1_epi16(0x0110);
        let ba = _mm_packus_epi16(_mm_maddubs_epi16(nibbles(ra), weights), _mm_setzero_si128());
        let bb = _mm_packus_epi16(_mm_maddubs_epi16(nibbles(rb), weights), _mm_setzero_si128());
        _mm_storel_epi64(out.as_mut_ptr() as *mut __m128i, ba); // out[0..8] (byte 7 discarded)
        _mm_storel_epi64(out.as_mut_ptr().add(7) as *mut __m128i, bb); // out[7..15] (byte 14 discarded)
        // Tail — string[32..36], no hyphen. The `& 0x0f` keeps the nibble in range for garbage input so
        // the unconditional `<< 4` cannot debug-overflow (valid nibbles are unaffected); rejection is
        // the caller's job via the returned mask + the scalar-tail table check.
        let nib = |c: u8| ((c & 0x0f) + 9 * (c >> 6)) & 0x0f;
        out[14] = (nib(b[32]) << 4) | nib(b[33]);
        out[15] = (nib(b[34]) << 4) | nib(b[35]);
        _mm_and_si128(hex_ok(ra), hex_ok(rb))
    }

    /// SSSE3 hex DECODE for one canonical 36-char UUID. The caller guarantees `'-'` at 8/13/18/23.
    /// Returns `None` on any non-hex digit — one `movemask` over the [`decode_core`] mask plus a table
    /// check of the two scalar-tail bytes.
    #[target_feature(enable = "ssse3,sse2")]
    pub unsafe fn parse_hyphenated(b: &[u8; 36]) -> Option<[u8; 16]> {
        use core::arch::x86_64::*;
        let mut out = [0u8; 16];
        let mask = decode_core(b, &mut out);
        let tail = super::HEX_DECODE[b[32] as usize]
            | super::HEX_DECODE[b[33] as usize]
            | super::HEX_DECODE[b[34] as usize]
            | super::HEX_DECODE[b[35] as usize];
        if _mm_movemask_epi8(mask) == 0xffff && tail < 16 {
            Some(out)
        } else {
            None
        }
    }

    /// Bulk parse of packed 36-char records — the batch-validated fast path. Every record decodes through
    /// the branch-free [`decode_core`], and the three validity channels accumulate WITHOUT branching (`&`
    /// the hex-lane masks, `|` the scalar-tail nibbles, `|` the XORed hyphen slots), so the single
    /// `movemask`+branch is paid once for the whole column instead of once per record — that per-record
    /// `movemask`+branch is exactly what serialized the loop (measured ~3× the decode itself). A valid
    /// column (the overwhelming common case) returns at trusted-decode speed; a single bad byte anywhere
    /// sinks the batch to `None`, byte-for-byte matching the per-record parser. `packed.len()` is a
    /// nonzero multiple of 36 (the caller checks).
    #[target_feature(enable = "ssse3,sse2")]
    pub unsafe fn parse_batch(packed: &[u8]) -> Option<Vec<super::Uuid>> {
        use core::arch::x86_64::*;
        let n = packed.len() / 36;
        let mut out: Vec<super::Uuid> = Vec::with_capacity(n);
        let mut acc = _mm_set1_epi8(-1i8); // all lanes 0xFF
        let mut tail_bad = 0u8;
        let mut dash_bad = 0u8;
        for chunk in packed.chunks_exact(36) {
            let b: &[u8; 36] = chunk.try_into().unwrap();
            dash_bad |= (b[8] ^ b'-') | (b[13] ^ b'-') | (b[18] ^ b'-') | (b[23] ^ b'-');
            tail_bad |= super::HEX_DECODE[b[32] as usize]
                | super::HEX_DECODE[b[33] as usize]
                | super::HEX_DECODE[b[34] as usize]
                | super::HEX_DECODE[b[35] as usize];
            let mut rec = [0u8; 16];
            acc = _mm_and_si128(acc, decode_core(b, &mut rec));
            out.push(super::Uuid(rec));
        }
        if dash_bad == 0 && tail_bad < 16 && _mm_movemask_epi8(acc) == 0xffff {
            Some(out)
        } else {
            None
        }
    }
}

/// Format many UUIDs into ONE contiguous buffer (each 36 bytes, `\n`-free), zero per-id allocation —
/// the bulk path a database column / log stream wants. Returns the packed `36·n`-byte buffer.
pub fn encode_many(ids: &[Uuid]) -> Vec<u8> {
    // Per-id SSSE3 encode, whose scatter is now SIMD (`pshufb`/`palignr`) — that's where bulk format
    // spends its time. Controlled interleaved A/B (contention-matched): the SIMD scatter beat the
    // per-range `copy_from_slice` by median 1.23× (whole distribution above 1.0). An AVX2 2-wide *encode*
    // was separately a wash — the encode was never the bottleneck; the scatter was.
    let mut out = vec![0u8; ids.len() * 36];
    for (i, id) in ids.iter().enumerate() {
        let slot: &mut [u8; 36] = (&mut out[i * 36..i * 36 + 36]).try_into().unwrap();
        encode_canonical(&id.0, slot);
    }
    out
}

impl fmt::Display for Uuid {
    /// Canonical lowercase hyphenated form, `8-4-4-4-12` (RFC 9562 §4).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = [0u8; 36];
        encode_canonical(&self.0, &mut buf);
        // SAFETY: every byte written is ASCII (`0-9a-f` or `-`), so the buffer is valid UTF-8.
        f.write_str(unsafe { core::str::from_utf8_unchecked(&buf) })
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Uuid({self})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nil_and_max_render_canonically() {
        assert_eq!(Uuid::NIL.to_string(), "00000000-0000-0000-0000-000000000000");
        assert_eq!(Uuid::MAX.to_string(), "ffffffff-ffff-ffff-ffff-ffffffffffff");
        assert!(Uuid::NIL.is_nil());
        assert!(Uuid::MAX.is_max());
        assert_eq!(Uuid::NIL.version(), 0);
        assert_eq!(Uuid::MAX.version(), 0xF);
    }

    #[test]
    fn parse_round_trips_every_accepted_form() {
        let canonical = "550e8400-e29b-41d4-a716-446655440000";
        let u = Uuid::parse(canonical).unwrap();
        assert_eq!(u.to_string(), canonical);
        // Simple, braced, urn, uppercase — all parse to the same value.
        assert_eq!(Uuid::parse("550e8400e29b41d4a716446655440000").unwrap(), u);
        assert_eq!(Uuid::parse("{550e8400-e29b-41d4-a716-446655440000}").unwrap(), u);
        assert_eq!(Uuid::parse("urn:uuid:550e8400-e29b-41d4-a716-446655440000").unwrap(), u);
        assert_eq!(Uuid::parse("550E8400-E29B-41D4-A716-446655440000").unwrap(), u);
        assert_eq!(Uuid::parse("  550e8400-e29b-41d4-a716-446655440000  ").unwrap(), u);
    }

    #[test]
    fn parse_rejects_malformed_input() {
        assert_eq!(Uuid::parse(""), None);
        assert_eq!(Uuid::parse("not-a-uuid"), None);
        assert_eq!(Uuid::parse("550e8400-e29b-41d4-a716-44665544000"), None); // too short
        assert_eq!(Uuid::parse("550e8400-e29b-41d4-a716-4466554400000"), None); // too long
        assert_eq!(Uuid::parse("550e8400xe29b-41d4-a716-446655440000"), None); // hyphen slot wrong
        assert_eq!(Uuid::parse("ZZZe8400-e29b-41d4-a716-446655440000"), None); // non-hex
    }

    #[test]
    fn parse_validates_every_hex_position_and_case() {
        // The SIMD path must accept both cases and the extremes, byte-identical to the scalar table.
        assert_eq!(Uuid::parse("ffffffff-ffff-ffff-ffff-ffffffffffff").unwrap(), Uuid::MAX);
        assert_eq!(Uuid::parse("FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF").unwrap(), Uuid::MAX);
        assert_eq!(Uuid::parse("00000000-0000-0000-0000-000000000000").unwrap(), Uuid::NIL);
        let base = "550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(
            Uuid::parse("550E8400-e29b-41D4-A716-446655440000").unwrap(),
            Uuid::parse(base).unwrap(),
            "mixed-case canonical must equal lowercase",
        );
        // Corrupt EVERY hex slot in turn with each boundary char adjacent to a valid range — the
        // parallel range check must reject all of them at all 32 positions (catches an off-by-one in
        // the SIMD validation, and the maddubs/pshufb lane layout via the value cases below).
        for i in 0..36 {
            if matches!(i, 8 | 13 | 18 | 23) {
                continue; // hyphen slots handled separately
            }
            for c in [b'g', b'G', b':', b'/', b'@', b'`', b' ', 0x00u8, 0xff] {
                let mut bad = base.as_bytes().to_vec();
                bad[i] = c;
                if let Ok(s) = std::str::from_utf8(&bad) {
                    assert_eq!(Uuid::parse(s), None, "must reject byte {c:#x} at position {i}");
                }
            }
        }
    }

    #[test]
    fn parse_many_round_trips_a_packed_column() {
        // Build a packed column of ids, format them, and bulk-parse the buffer back — must equal.
        let mut ids = Vec::new();
        let mut st = 0x1234_5678_9abc_def0u64;
        for _ in 0..257 {
            let mut b = [0u8; 16];
            for x in b.iter_mut() {
                st = st.wrapping_mul(6364136223846793005).wrapping_add(1);
                *x = (st >> 56) as u8;
            }
            ids.push(Uuid::from_bytes(b));
        }
        let packed = encode_many(&ids);
        assert_eq!(Uuid::parse_many(&packed).unwrap(), ids);
        // Malformed inputs are all rejected: empty and a non-multiple length.
        assert_eq!(Uuid::parse_many(&[]), None);
        assert_eq!(Uuid::parse_many(&packed[..packed.len() - 1]), None);
        // A single bad byte anywhere in ANY record must sink the whole batch — the batch-validated
        // fast path accumulates three independent validity channels (the SIMD hex-lane mask, the
        // scalar tail digits, and the four hyphens), so corruption is probed in each channel, at the
        // first record, an interior record, and the very last record. Positions 8/13/18/23 are the
        // hyphens; 32–35 are the scalar tail; everything else is a SIMD-lane hex digit.
        let n = ids.len();
        for &rec in &[0usize, 3, n - 1] {
            for &pos in &[0usize, 5, 8, 12, 18, 23, 30, 32, 35] {
                let mut bad = packed.clone();
                let byte = 36 * rec + pos;
                bad[byte] = if pos == 8 || pos == 13 || pos == 18 || pos == 23 {
                    b'f' // a hyphen slot filled with a hex digit — dash channel must still reject
                } else {
                    b'z' // a hex slot filled with a non-hex byte — hex/tail channel must reject
                };
                assert_eq!(
                    Uuid::parse_many(&bad),
                    None,
                    "record {rec} byte {pos} corruption not rejected"
                );
            }
        }
    }

    #[test]
    fn name_based_versions_match_the_rfc_worked_examples() {
        // RFC-published name-based ids under the DNS namespace.
        let v3 = Uuid::new_v3(Uuid::NAMESPACE_DNS, b"www.example.com");
        assert_eq!(v3.version(), 3);
        assert_eq!(v3.variant(), Variant::Rfc4122);
        assert_eq!(v3.to_string(), "5df41881-3aed-3515-88a7-2f4a814cf09e");

        let v5 = Uuid::new_v5(Uuid::NAMESPACE_DNS, b"www.example.com");
        assert_eq!(v5.version(), 5);
        assert_eq!(v5.variant(), Variant::Rfc4122);
        assert_eq!(v5.to_string(), "2ed6657d-e927-568b-95e1-2665a8aea6a2");
    }

    #[test]
    fn version_and_variant_bits_are_correct_for_every_generated_version() {
        let node = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
        let cases: [(Uuid, u8); 7] = [
            (Uuid::new_v1(0x1234_5678_9ABC, 0x0102, node), 1),
            (Uuid::new_v3(Uuid::NAMESPACE_URL, b"a"), 3),
            (Uuid::new_v4([0xAB; 16]), 4),
            (Uuid::new_v5(Uuid::NAMESPACE_URL, b"a"), 5),
            (Uuid::new_v6(0x1234_5678_9ABC, 0x0102, node), 6),
            (Uuid::new_v7(0x0190_0000_0000, [0x11; 10]), 7),
            (Uuid::new_v8([0x77; 16]), 8),
        ];
        for (u, want) in cases {
            assert_eq!(u.version(), want, "version nibble for v{want}");
            assert_eq!(u.variant(), Variant::Rfc4122, "variant for v{want}");
            // Round-trips through canonical text.
            assert_eq!(Uuid::parse(&u.to_string()).unwrap(), u, "round trip v{want}");
        }
    }

    #[test]
    fn v7_is_time_ordered_by_bytes() {
        // Two v7 ids one millisecond apart sort in time order regardless of the random tail.
        let earlier = Uuid::new_v7(1000, [0xFF; 10]);
        let later = Uuid::new_v7(1001, [0x00; 10]);
        assert!(earlier < later, "v7 must sort by timestamp: {earlier} !< {later}");
    }

    #[test]
    fn differential_against_the_uuid_crate() {
        // Name-based (deterministic) versions, byte-for-byte against the reference crate.
        for name in ["", "a", "www.example.com", "the quick brown fox", "🦀"] {
            let ours = Uuid::new_v3(Uuid::NAMESPACE_DNS, name.as_bytes());
            let theirs = uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_DNS, name.as_bytes());
            assert_eq!(ours.to_bytes(), *theirs.as_bytes(), "v3 differs for {name:?}");

            let ours = Uuid::new_v5(Uuid::NAMESPACE_URL, name.as_bytes());
            let theirs = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, name.as_bytes());
            assert_eq!(ours.to_bytes(), *theirs.as_bytes(), "v5 differs for {name:?}");
        }

        // v4 / v7 from identical entropy/time must equal the crate's builder output.
        let rand16 = [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
        ];
        let ours = Uuid::new_v4(rand16);
        let theirs = uuid::Builder::from_random_bytes(rand16).into_uuid();
        assert_eq!(ours.to_bytes(), *theirs.as_bytes(), "v4 differs");

        let rand10 = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc];
        let ms: u64 = 0x0190_2233_4455;
        let ours = Uuid::new_v7(ms, rand10);
        let theirs = uuid::Builder::from_unix_timestamp_millis(ms, &rand10).into_uuid();
        assert_eq!(ours.to_bytes(), *theirs.as_bytes(), "v7 differs");

        // Parse + display agree with the crate over random ids.
        let mut state: u64 = 0xDEAD_BEEF_CAFE_F00D;
        for _ in 0..2000 {
            let mut bytes = [0u8; 16];
            for byte in bytes.iter_mut() {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                *byte = (state >> 56) as u8;
            }
            let ours = Uuid::from_bytes(bytes);
            let theirs = uuid::Uuid::from_bytes(bytes);
            assert_eq!(ours.to_string(), theirs.to_string(), "display differs");
            assert_eq!(Uuid::parse(&theirs.to_string()).unwrap(), ours, "parse differs");
            assert_eq!(ours.version(), theirs.get_version_num() as u8, "version differs");
        }
    }
}
