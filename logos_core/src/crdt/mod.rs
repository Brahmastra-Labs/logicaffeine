//! CRDT (Conflict-free Replicated Data Types) for LOGOS
//!
//! Phase 49: Native support for eventually consistent distributed state.
//!
//! CRDTs provide automatic conflict resolution for distributed state synchronization.
//! Any two replicas can be merged to produce the same result regardless of order.

mod gcounter;
mod lww;
mod merge;

pub use gcounter::GCounter;
pub use lww::LWWRegister;
pub use merge::Merge;
