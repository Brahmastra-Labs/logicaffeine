//! CRDT (Conflict-free Replicated Data Types) for LOGOS
//!
//! Phase 49: Native support for eventually consistent distributed state.
//!
//! CRDTs provide automatic conflict resolution for distributed state synchronization.
//! Any two replicas can be merged to produce the same result regardless of order.
//!
//! Phase 52: Added `Synced<T>` wrapper for automatic GossipSub replication.

mod gcounter;
mod lww;
mod merge;

// Phase 52: Synced wrapper uses tokio and network - native only
#[cfg(not(target_arch = "wasm32"))]
mod sync;

pub use gcounter::GCounter;
pub use lww::LWWRegister;
pub use merge::Merge;

#[cfg(not(target_arch = "wasm32"))]
pub use sync::Synced;
