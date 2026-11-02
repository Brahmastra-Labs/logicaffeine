//! The relay wire protocol — shared verbatim by the native server/client
//! (`super::relay`) and the browser client (`super::relay_browser`). Both
//! targets serialize the SAME [`RelayFrame`] with `bincode`, so a browser tab and
//! a native node speak an identical language across the WebSocket.
//!
//! This module is target-agnostic (no `tokio`, no `web-sys`) so it compiles into
//! both the native binary and the wasm bundle.

use serde::{Deserialize, Serialize};

/// A frame on the relay WebSocket. Client→relay: `Subscribe`/`Unsubscribe`/
/// `Publish`. Relay→client: `SubAck`/`Event`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelayFrame {
    /// Client → relay: start receiving `Event`s for `topic`.
    Subscribe { topic: String },
    /// Client → relay: stop receiving `topic`.
    Unsubscribe { topic: String },
    /// Client → relay: deliver `data` to every subscriber of `topic`.
    Publish { topic: String, data: Vec<u8> },
    /// Relay → client: acknowledges a `Subscribe` is registered (so a publish
    /// that races a subscribe cannot be missed — the client awaits this).
    SubAck { topic: String },
    /// Relay → client: a published message on a topic the client subscribes to.
    Event { topic: String, data: Vec<u8> },
}

impl RelayFrame {
    /// Serialize to the bytes that cross the WebSocket (a binary message).
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).expect("relay frame is serializable")
    }

    /// Deserialize a frame from WebSocket bytes; `None` on a malformed frame.
    pub fn from_bytes(bytes: &[u8]) -> Option<RelayFrame> {
        bincode::deserialize(bytes).ok()
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_system/relay_proto/tests.rs"]
mod tests;
