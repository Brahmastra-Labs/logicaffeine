use super::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct TestMessage {
    id: u32,
    content: String,
}

#[test]
fn test_roundtrip() {
    let msg = TestMessage {
        id: 42,
        content: "hello mesh".to_string(),
    };
    let bytes = encode(&msg).unwrap();
    let decoded: TestMessage = decode(&bytes).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_decode_invalid_bytes() {
    let bytes = vec![0xFF, 0xFF, 0xFF];
    let result: Result<TestMessage, _> = decode(&bytes);
    assert!(result.is_err());
}

/// `encode` must produce the message's real, content-dependent bytes — not a constant. Kills
/// the `encode -> Ok(vec![])` / `vec![0]` / `vec![1]` mutants (a gutted encoder returns a fixed
/// short vec that neither round-trips nor distinguishes distinct messages).
#[test]
fn encode_is_content_dependent_and_round_trips() {
    let msg = TestMessage {
        id: 0xDEAD_BEEF,
        content: "the quick brown fox".to_string(),
    };
    let bytes = encode(&msg).unwrap();
    assert!(
        bytes.len() > 3,
        "encoding too short to be a real serialization: {bytes:?}"
    );
    let back: TestMessage = decode(&bytes).unwrap();
    assert_eq!(msg, back);
    // Distinct messages must encode to distinct bytes (a constant encoder fails this).
    let other = TestMessage {
        id: 1,
        content: "x".to_string(),
    };
    assert_ne!(
        encode(&other).unwrap(),
        bytes,
        "distinct messages encoded identically"
    );
}

/// The `Display` for `WireError` must name its kind and carry its cause — kills the
/// `fmt -> Ok(Default::default())` mutant (which would write nothing).
#[test]
fn wire_error_display_names_kind_and_cause() {
    let s = format!("{}", WireError::Encode("boom".to_string()));
    assert!(
        s.contains("encode") && s.contains("boom"),
        "encode error display: {s:?}"
    );
    let d = format!("{}", WireError::Decode("bad-frame".to_string()));
    assert!(
        d.contains("decode") && d.contains("bad-frame"),
        "decode error display: {d:?}"
    );
}
