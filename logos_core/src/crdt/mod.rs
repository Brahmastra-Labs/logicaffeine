//! CRDT (Conflict-free Replicated Data Types) for LOGOS
//!
//! Phase 49: Native support for eventually consistent distributed state.
//!
//! CRDTs provide automatic conflict resolution for distributed state synchronization.
//! Any two replicas can be merged to produce the same result regardless of order.
//!
//! Phase 52: Added `Synced<T>` wrapper for automatic GossipSub replication.
//!
//! Wave 1: Added causal infrastructure (VClock, Dot, DotContext) and delta support.

mod gcounter;
mod lww;
mod merge;
mod replica;

// Wave 1: Causal infrastructure
pub mod causal;

// Wave 1: Delta CRDT support
mod delta;
mod delta_buffer;

// Wave 2: Additional CRDTs
mod pncounter;
mod mvregister;

// Wave 3: Complex CRDTs
mod orset;
mod ormap;
pub mod sequence;

// Phase 52: Synced wrapper uses tokio and network - native only
#[cfg(not(target_arch = "wasm32"))]
mod sync;

pub use gcounter::GCounter;
pub use lww::LWWRegister;
pub use merge::Merge;

// Wave 1: Export replica utilities
pub use replica::{generate_replica_id, ReplicaId};

// Wave 1: Export causal types
pub use causal::{Dot, DotContext, VClock};

// Wave 1: Export delta types
pub use delta::DeltaCrdt;
pub use delta_buffer::DeltaBuffer;

// Wave 2: Export additional CRDTs
pub use pncounter::PNCounter;
pub use mvregister::MVRegister;

// Wave 3: Export complex CRDTs
pub use orset::{AddWins, ORSet, RemoveWins, SetBias};
pub use ormap::ORMap;
pub use sequence::{RGA, YATA};

#[cfg(not(target_arch = "wasm32"))]
pub use sync::Synced;
