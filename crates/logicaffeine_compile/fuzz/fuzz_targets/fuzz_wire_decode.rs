#![no_main]
//! Fuzz the peer wire DECODER on untrusted bytes (Invariants I & X): a corrupt, truncated,
//! or hostile frame must return Some/None — never panic, abort, or OOM. This is the
//! "corrupt database file" discipline (SQLite) applied to the p2p codec; it is the exact
//! surface where Wave 1's 476 GB OOM-DoS lived (a lying length prefix → giant allocation,
//! now capped at remaining input). libFuzzer catches the abort/OOM class directly.

use libfuzzer_sys::fuzz_target;
use logicaffeine_compile::concurrency::marshal::{
    decode_value_raw, message_from_wire, parse_handshake_frame,
};

fuzz_target!(|data: &[u8]| {
    let _ = message_from_wire(data);
    let _ = decode_value_raw(data);
    let _ = parse_handshake_frame(data);
});
