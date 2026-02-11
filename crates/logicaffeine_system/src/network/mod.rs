//! P2P Networking for LOGOS Distributed Systems
//!
//! Provides libp2p-based peer-to-peer networking with automatic peer discovery,
//! request-response messaging, and GossipSub pub/sub for CRDT synchronization.
//!
//! # Architecture
//!
//! - **Request-Response**: Point-to-point message exchange between agents
//! - **mDNS Discovery**: Automatic local network peer discovery
//! - **GossipSub**: Pub/sub broadcast for CRDT replication
//! - **File Sipping**: Zero-copy file chunking with resumable transfers
//!
//! # Features
//!
//! - `networking`: Required for all network functionality
//! - `persistence` + `concurrency`: Required for file sipping
//!
//! # Example
//!
//! ```no_run
//! use logicaffeine_system::network::{listen, connect, send, PeerAgent};
//! # use serde::{Serialize, Deserialize};
//!
//! # #[derive(Serialize, Deserialize)]
//! # struct MyMessage { data: i32 }
//! # fn main() {}
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Start listening for connections
//! listen("/ip4/0.0.0.0/tcp/8000").await?;
//!
//! // Connect to a remote peer
//! connect("/ip4/192.168.1.100/tcp/8000").await?;
//!
//! // Send a message to a specific peer
//! let peer = PeerAgent::new("/ip4/192.168.1.100/tcp/8000/p2p/12D3Koo...")?;
//! send(&peer, &MyMessage { data: 42 }).await?;
//! # Ok(())
//! # }
//! ```

// Sipping requires persistence (sha2, memmap2) AND concurrency (Zone/bumpalo)
#[cfg(all(feature = "persistence", feature = "concurrency"))]
mod sipping;
pub mod wire;
mod protocol;
mod behaviour;
mod mesh;
pub mod gossip;
#[cfg(test)]
mod e2e_tests;

#[cfg(all(feature = "persistence", feature = "concurrency"))]
pub use sipping::{FileSipper, FileManifest, FileChunk, DEFAULT_CHUNK_SIZE};
pub use mesh::{listen, connect, send, local_peer_id, PeerAgent, MeshNode, NetworkError};
pub use mesh::{gossip_publish, gossip_subscribe};
