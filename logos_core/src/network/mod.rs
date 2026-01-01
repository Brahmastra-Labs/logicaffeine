//! Phase 48 & 51 & 52: Network primitives for LOGOS distributed system.
//!
//! This module provides:
//! - Zero-copy file chunking and resumable transfer protocols (Phase 48)
//! - P2P networking primitives for agent communication (Phase 51)
//! - GossipSub pub/sub for automatic CRDT replication (Phase 52)

mod sipping;
pub mod wire;
mod protocol;
mod behaviour;
mod mesh;
pub mod gossip;
#[cfg(test)]
mod e2e_tests;

pub use sipping::{FileSipper, FileManifest, FileChunk, DEFAULT_CHUNK_SIZE};
pub use mesh::{listen, connect, send, local_peer_id, PeerAgent, MeshNode, NetworkError};
pub use mesh::{gossip_publish, gossip_subscribe};
