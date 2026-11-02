//! CRDT (Conflict-free Replicated Data Types)
//!
//! Pure data structures for eventually consistent distributed state.
//! CRDTs provide automatic conflict resolution - any two replicas can be
//! merged to produce the same result regardless of order.
//!
//! This crate is WASM-safe: no IO and no network dependencies. Merging never reads a clock;
//! only native `generate_replica_id` does, to seed a random replica id.
//! The `Synced<T>` wrapper lives in `logicaffeine_system` (requires tokio/libp2p).

mod gcounter;
mod latest;
mod lww;
mod merge;
mod replica;

// Causal infrastructure
pub mod causal;

// Delta CRDT support
mod delta;
mod delta_buffer;

// Additional CRDTs
mod mvregister;
mod pncounter;

// Complex CRDTs
mod ormap;
mod orset;
pub mod sequence;

// NOTE: sync.rs (Synced<T>) is NOT in this crate - it's in logicaffeine_system
// because it requires tokio and networking.

pub use gcounter::GCounter;
pub use latest::LatestWrite;
pub use lww::LWWRegister;
pub use merge::Merge;

// Replica utilities
pub use replica::{generate_replica_id, ReplicaId};

// Causal types
pub use causal::{Dot, DotContext, VClock};

// Delta types
pub use delta::DeltaCrdt;
pub use delta_buffer::DeltaBuffer;

// Additional CRDTs
pub use mvregister::MVRegister;
pub use pncounter::PNCounter;

// Complex CRDTs
pub use ormap::ORMap;
pub use orset::{AddWins, ORSet, RemoveWins, SetBias};
pub use sequence::{RGA, YATA};
