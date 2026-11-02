//! The relay protocol's bytes, pinned, so every client speaks exactly the one definition.
//!
//! The relay frames (`logicaffeine_system::relay_proto::RelayFrame`) and the LOGOS peer layer on top of them
//! (`logicaffeine_compile::concurrency::marshal`: data messages, values, handshakes, streams) are defined once, in
//! Rust. This lock derives a golden vector for each of their forms from that definition and holds the committed
//! `tests/fixtures/relay/vectors.tsv` to them. A client in another language parses and produces exactly these
//! bytes; when the Rust side changes a byte, this lock reddens before any other client can drift from it.
//!
//! Each vector also decodes back to what it encodes, through the decoder every LOGOS peer runs.
//!
//! Regenerate after a deliberate change: `LOGOS_REGEN=1 cargo nextest run -p logicaffeine-tests --test
//! relay_protocol_lock`, then review the diff.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/golden.rs"]
mod golden;

use std::cell::RefCell;
use std::rc::Rc;

use logicaffeine_compile::concurrency::marshal::{self, PeerProfile, WireCodec, WireIntegrity};
use logicaffeine_compile::interpreter::{ListRepr, RuntimeValue};
use logicaffeine_system::relay_proto::RelayFrame;

const VECTORS: &str = "crates/logicaffeine_tests/tests/fixtures/relay/vectors.tsv";
const REGEN: &str =
    "LOGOS_REGEN=1 cargo nextest run -p logicaffeine-tests --test relay_protocol_lock";

/// The relay frame's name in the vector file. No `_` arm: a new frame does not compile until it has a
/// name, and [`every_relay_frame_has_a_vector`] then requires a vector for it.
fn frame_name(f: &RelayFrame) -> &'static str {
    match f {
        RelayFrame::Subscribe { .. } => "subscribe",
        RelayFrame::Unsubscribe { .. } => "unsubscribe",
        RelayFrame::Publish { .. } => "publish",
        RelayFrame::SubAck { .. } => "suback",
        RelayFrame::Event { .. } => "event",
    }
}

fn frames() -> Vec<RelayFrame> {
    vec![
        RelayFrame::Subscribe {
            topic: "room".to_string(),
        },
        RelayFrame::Unsubscribe {
            topic: "room".to_string(),
        },
        RelayFrame::Publish {
            topic: "room".to_string(),
            data: vec![1, 2, 3],
        },
        RelayFrame::SubAck {
            topic: "room".to_string(),
        },
        RelayFrame::Event {
            topic: "room".to_string(),
            data: vec![1, 2, 3],
        },
    ]
}

fn text(s: &str) -> RuntimeValue {
    RuntimeValue::Text(Rc::new(s.to_string()))
}

fn list(values: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(values))))
}

/// The values a data message's vectors cover: every scalar tag at its edges, text with and without
/// multi-byte and multi-line content, and lists in the shapes a sender chooses between.
fn values() -> Vec<(&'static str, RuntimeValue)> {
    vec![
        ("nothing", RuntimeValue::Nothing),
        ("false", RuntimeValue::Bool(false)),
        ("true", RuntimeValue::Bool(true)),
        ("int-0", RuntimeValue::Int(0)),
        ("int-7", RuntimeValue::Int(7)),
        ("int-minus-1", RuntimeValue::Int(-1)),
        ("int-max", RuntimeValue::Int(i64::MAX)),
        ("int-min", RuntimeValue::Int(i64::MIN)),
        ("real-2.5", RuntimeValue::Float(2.5)),
        ("real-minus-0", RuntimeValue::Float(-0.0)),
        ("char-e-acute", RuntimeValue::Char('é')),
        ("text-empty", text("")),
        ("text-hi", text("hi")),
        ("text-multibyte-multiline", text("héllo\nwörld")),
        ("list-empty", list(vec![])),
        (
            "list-ints",
            list(vec![
                RuntimeValue::Int(1),
                RuntimeValue::Int(2),
                RuntimeValue::Int(3),
            ]),
        ),
        ("list-texts", list(vec![text("a"), text("b")])),
        (
            "list-nested",
            list(vec![
                list(vec![RuntimeValue::Int(1)]),
                list(vec![RuntimeValue::Int(2), RuntimeValue::Int(3)]),
            ]),
        ),
        (
            "tuple-int-text",
            RuntimeValue::Tuple(Rc::new(vec![RuntimeValue::Int(1), text("a")])),
        ),
    ]
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Every vector: `(name, what it is, its bytes)`.
fn vectors() -> Vec<(String, String, Vec<u8>)> {
    let mut v = Vec::new();
    for f in frames() {
        v.push((
            format!("relay/{}", frame_name(&f)),
            format!("{f:?}"),
            f.to_bytes(),
        ));
    }
    v.push((
        "peer/handshake-full".to_string(),
        "the handshake a LOGOS node sends from inbox \"alice\" (its full profile)".to_string(),
        marshal::make_handshake_frame("alice", &PeerProfile::default()),
    ));
    v.push((
        "peer/handshake-conservative".to_string(),
        "a handshake from inbox \"alice\" advertising only the base form".to_string(),
        marshal::make_handshake_frame("alice", &PeerProfile::conservative()),
    ));
    for (name, value) in values() {
        let checked = marshal::message_to_wire_with(
            "alice",
            &value,
            WireCodec::Native,
            WireIntegrity::Checked,
        )
        .expect("the value encodes");
        v.push((
            format!("message/{name}"),
            format!("from \"alice\", checksummed: {}", value.to_display_string()),
            checked,
        ));
    }
    let raw = marshal::message_to_wire_with(
        "alice",
        &RuntimeValue::Int(7),
        WireCodec::Native,
        WireIntegrity::Raw,
    )
    .expect("the value encodes");
    v.push((
        "message/int-7-unchecked".to_string(),
        "from \"alice\", no checksum: 7".to_string(),
        raw,
    ));
    let stream = marshal::frame_stream_message(
        "alice",
        &[
            RuntimeValue::Int(1),
            RuntimeValue::Int(2),
            RuntimeValue::Int(3),
        ],
    )
    .expect("the stream frames");
    v.push((
        "stream/ints".to_string(),
        "a stream from \"alice\" of 1, 2, 3".to_string(),
        stream,
    ));
    v
}

fn table() -> String {
    let mut out = String::from("# name\thex\twhat\n");
    for (name, what, bytes) in vectors() {
        out.push_str(&format!(
            "{name}\t{}\t{}\n",
            hex(&bytes),
            what.replace(['\t', '\n'], " ")
        ));
    }
    out
}

#[test]
fn the_committed_vectors_are_the_protocol() {
    golden::committed_bytes(VECTORS, table().as_bytes(), REGEN);
}

#[test]
fn every_relay_frame_has_a_vector() {
    let names: std::collections::BTreeSet<String> =
        vectors().into_iter().map(|(n, _, _)| n).collect();
    for f in frames() {
        assert!(
            names.contains(&format!("relay/{}", frame_name(&f))),
            "no vector for {f:?}"
        );
    }
    assert_eq!(
        frames().len(),
        5,
        "a relay frame kind exists with no vector"
    );
}

#[test]
fn every_relay_frame_vector_decodes_to_its_frame() {
    for f in frames() {
        assert_eq!(
            RelayFrame::from_bytes(&f.to_bytes()).as_ref(),
            Some(&f),
            "{f:?} does not round-trip"
        );
    }
}

#[test]
fn every_message_vector_decodes_to_its_sender_and_value() {
    for (name, value) in values() {
        let bytes = marshal::message_to_wire_with(
            "alice",
            &value,
            WireCodec::Native,
            WireIntegrity::Checked,
        )
        .unwrap();
        let (from, back) =
            marshal::message_from_wire(&bytes).unwrap_or_else(|| panic!("{name} does not decode"));
        assert_eq!(from, "alice", "{name}");
        assert_eq!(
            back.to_display_string(),
            value.to_display_string(),
            "{name} decodes to another value"
        );
    }
}

#[test]
fn a_handshake_vector_decodes_to_its_profile() {
    for profile in [PeerProfile::default(), PeerProfile::conservative()] {
        let bytes = marshal::make_handshake_frame("alice", &profile);
        let (from, back) = marshal::parse_handshake_frame(&bytes).expect("the handshake parses");
        assert_eq!((from.as_str(), back), ("alice", profile));
    }
}

#[test]
fn a_stream_vector_deframes_to_its_values() {
    let bytes = marshal::frame_stream_message(
        "alice",
        &[
            RuntimeValue::Int(1),
            RuntimeValue::Int(2),
            RuntimeValue::Int(3),
        ],
    )
    .unwrap();
    assert_eq!(
        marshal::peek_stream_sender(&bytes).as_deref(),
        Some("alice")
    );
    let values: Vec<String> = marshal::deframe_stream_message(&bytes)
        .expect("the stream deframes")
        .iter()
        .map(|v| v.to_display_string())
        .collect();
    assert_eq!(values, ["1", "2", "3"]);
}
