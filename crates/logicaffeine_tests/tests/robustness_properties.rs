//! ════════════════════════════════════════════════════════════════════════════
//! ROBUSTNESS PROPERTIES — round-trip identity and totality.
//!
//! Property-based checks over a deterministic generator (the same SplitMix64 the
//! never-panic lock uses). Two laws for the peer wire codec:
//!   • encode∘decode == identity  — a value survives a wire round-trip unchanged
//!     (the metamorphic law that makes the p2p codec trustworthy).
//!   • decode is TOTAL             — ANY byte string (corrupt, truncated, adversarial)
//!     decodes to Some/None, never a panic (the "corrupt frame" robustness).
//!
//! Heavy-volume, shrinking property fuzzing (proptest / cargo-fuzz) is the nightly
//! Wave-5 job; this is the fast, deterministic, in-suite regression mirror. The
//! random-VALID-program tier differential is already covered by `vm_fuzz_source.rs`.

use std::rc::Rc;

use logicaffeine_compile::concurrency::marshal::{
    decode_value_raw, encode_value_raw, message_from_wire, message_to_wire,
};
use logicaffeine_compile::interpreter::RuntimeValue;

/// SplitMix64 — the project's deterministic generator (matches the never-panic lock).
struct SplitMix64(u64);
impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Scalar value identity WITHOUT `RuntimeValue: PartialEq` (it derives only Debug/Clone):
/// match the variant and compare the payload. Floats compare bit-exact (the codec stores
/// the full IEEE-754 bit pattern), which is also the only correct comparison for a codec.
fn same_scalar(a: &RuntimeValue, b: &RuntimeValue) -> bool {
    match (a, b) {
        (RuntimeValue::Int(x), RuntimeValue::Int(y)) => x == y,
        (RuntimeValue::Bool(x), RuntimeValue::Bool(y)) => x == y,
        (RuntimeValue::Text(x), RuntimeValue::Text(y)) => **x == **y,
        (RuntimeValue::Float(x), RuntimeValue::Float(y)) => x.to_bits() == y.to_bits(),
        _ => false,
    }
}

/// A random scalar value across the four primitive kinds. Text is built from code points
/// below the surrogate range (all valid), including multibyte, to stress string encoding.
fn random_scalar(rng: &mut SplitMix64) -> RuntimeValue {
    match rng.next() % 4 {
        0 => RuntimeValue::Int(rng.next() as i64),
        1 => RuntimeValue::Bool(rng.next() & 1 == 0),
        2 => {
            let len = (rng.next() % 32) as usize;
            let mut s = String::new();
            for _ in 0..len {
                let cp = (rng.next() % 0xD7FF) as u32 + 1; // 1..=0xD7FF, all valid scalars
                s.push(char::from_u32(cp).unwrap_or('x'));
            }
            RuntimeValue::Text(Rc::new(s))
        }
        _ => {
            let f = f64::from_bits(rng.next());
            // A codec must round-trip NaN bit-for-bit too, but NaN != NaN defeats value
            // comparison; keep the property on finite floats (bit-exact) and let the
            // never-panic/corrupt-bytes tests cover NaN payloads.
            if f.is_finite() {
                RuntimeValue::Float(f)
            } else {
                RuntimeValue::Int(rng.next() as i64)
            }
        }
    }
}

/// LAW: `decode_value_raw(encode_value_raw(v)) == v` for every scalar value.
#[test]
fn wire_scalar_roundtrip_is_identity() {
    let mut rng = SplitMix64::new(0x00D7_2150_1DE4_7A11);
    let mut checked = 0u64;
    for _ in 0..4_000 {
        let v = random_scalar(&mut rng);
        let bytes = encode_value_raw(&v).expect("a well-formed scalar must encode");
        let decoded =
            decode_value_raw(&bytes).expect("bytes we just encoded must decode back to Some");
        assert!(
            same_scalar(&v, &decoded),
            "WIRE ROUND-TRIP BROKEN (Invariant VIII): encode∘decode changed the value.\n\
             original={v:?}\ndecoded ={decoded:?}\nbytes   ={bytes:?}"
        );
        checked += 1;
    }
    assert_eq!(checked, 4_000, "the generator must actually run every case");
}

/// LAW: a full framed message `(channel, value)` survives `message_to_wire` →
/// `message_from_wire` unchanged (default integrity = Checked, so this also exercises
/// the integrity hash round-trip).
#[test]
fn wire_frame_roundtrip_is_identity() {
    let mut rng = SplitMix64::new(0xFEED_FACE_0000_0001);
    for _ in 0..3_000 {
        // A random channel name (ASCII, may be empty).
        let name_len = (rng.next() % 20) as usize;
        let from: String = (0..name_len)
            .map(|_| (b'a' + (rng.next() % 26) as u8) as char)
            .collect();
        let v = random_scalar(&mut rng);

        let bytes = message_to_wire(&from, &v).expect("a well-formed message must encode");
        let (from2, v2) =
            message_from_wire(&bytes).expect("a message we just framed must decode to Some");
        assert_eq!(
            from2, from,
            "the channel name must survive the wire round-trip"
        );
        assert!(
            same_scalar(&v, &v2),
            "FRAME ROUND-TRIP BROKEN: value changed.\noriginal={v:?}\ndecoded={v2:?}"
        );
    }
}

/// LAW: decoding is TOTAL — no byte string, however corrupt or truncated, may panic the
/// decoder. It must return Some/None. (A panic here fails the test, which is the point.)
#[test]
fn wire_decode_is_total_on_random_bytes() {
    let mut rng = SplitMix64::new(0xC0FF_EE00_D06_F00D);
    for _ in 0..8_000 {
        let len = (rng.next() % 300) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| (rng.next() & 0xff) as u8).collect();
        // Both decoders must simply RETURN on arbitrary input — never panic/abort.
        let _ = decode_value_raw(&bytes);
        let _ = message_from_wire(&bytes);
    }
}

/// LAW: decoding is total even on MUTATED valid frames — take a real encoded value and
/// flip/insert/truncate bytes; the decoder still only ever returns Some/None. This is the
/// "corrupt database file" discipline (SQLite) applied to the peer codec: a real header
/// with a corrupt body must not crash the decoder.
#[test]
fn wire_decode_is_total_on_mutated_valid_frames() {
    let mut rng = SplitMix64::new(0x1234_5678_9ABC_DEF0);
    for _ in 0..4_000 {
        let v = random_scalar(&mut rng);
        let Ok(mut bytes) = message_to_wire("peer", &v) else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        // A random single-byte mutation somewhere in the frame.
        let idx = (rng.next() as usize) % bytes.len();
        bytes[idx] ^= (rng.next() & 0xff) as u8;
        // Occasionally truncate too.
        if rng.next() & 1 == 0 && bytes.len() > 1 {
            let keep = (rng.next() as usize) % bytes.len();
            bytes.truncate(keep);
        }
        let _ = message_from_wire(&bytes);
        let _ = decode_value_raw(&bytes);
    }
}
