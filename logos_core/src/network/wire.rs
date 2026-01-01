//! LogosWire: Bincode-based wire serialization for P2P messaging.
//!
//! Provides a simple abstraction over bincode for encoding/decoding
//! messages on the wire. Designed for easy future migration to rkyv
//! if zero-copy performance becomes necessary.

use serde::{de::DeserializeOwned, Serialize};
use std::fmt;

/// Error type for wire serialization/deserialization.
#[derive(Debug, Clone)]
pub enum WireError {
    /// Failed to encode message to bytes
    Encode(String),
    /// Failed to decode bytes to message
    Decode(String),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(msg) => write!(f, "Wire encode error: {}", msg),
            Self::Decode(msg) => write!(f, "Wire decode error: {}", msg),
        }
    }
}

impl std::error::Error for WireError {}

/// Encode a serializable message to bytes.
///
/// # Example
/// ```
/// use serde::{Serialize, Deserialize};
/// use logos_core::network::wire;
///
/// #[derive(Serialize, Deserialize, PartialEq, Debug)]
/// struct Ping { id: u32 }
///
/// let msg = Ping { id: 42 };
/// let bytes = wire::encode(&msg).unwrap();
/// let decoded: Ping = wire::decode(&bytes).unwrap();
/// assert_eq!(msg, decoded);
/// ```
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, WireError> {
    bincode::serialize(msg).map_err(|e| WireError::Encode(e.to_string()))
}

/// Decode bytes to a deserializable message.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, WireError> {
    bincode::deserialize(bytes).map_err(|e| WireError::Decode(e.to_string()))
}

#[cfg(test)]
mod tests {
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
}
