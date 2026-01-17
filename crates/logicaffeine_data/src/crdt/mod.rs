//! CRDT (Conflict-free Replicated Data Types)
//!
//! Pure data structures for eventually consistent distributed state.
//! CRDTs provide automatic conflict resolution - any two replicas can be
//! merged to produce the same result regardless of order.
//!
//! This crate is WASM-safe: NO IO, NO SystemTime, NO network dependencies.
//! The `Synced<T>` wrapper lives in `logicaffeine_system` (requires tokio/libp2p).

mod gcounter;
mod lww;
mod merge;
mod replica;

// Causal infrastructure
pub mod causal;

// Delta CRDT support
mod delta;
mod delta_buffer;

// Additional CRDTs
mod pncounter;
mod mvregister;

// Complex CRDTs
mod orset;
mod ormap;
pub mod sequence;

// NOTE: sync.rs (Synced<T>) is NOT in this crate - it's in logicaffeine_system
// because it requires tokio and networking.

pub use gcounter::GCounter;
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
pub use pncounter::PNCounter;
pub use mvregister::MVRegister;

// Complex CRDTs
pub use orset::{AddWins, ORSet, RemoveWins, SetBias};
pub use ormap::ORMap;
pub use sequence::{RGA, YATA};
