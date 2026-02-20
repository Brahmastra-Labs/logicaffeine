#![cfg_attr(docsrs, feature(doc_cfg))]

//! WASM-safe data structures and CRDTs
//!
//! This crate provides pure data structures with NO IO dependencies.
//! It compiles for both native and wasm32-unknown-unknown targets.
//!
//! ## LAMPORT INVARIANT
//!
//! This crate has NO path to system IO. No tokio, no libp2p, no SystemTime.
//! Timestamps are injected by callers (typically from logicaffeine_system).

pub mod crdt;
pub mod indexing;
pub mod types;

// Re-export commonly used types
pub use crdt::{
    generate_replica_id, AddWins, DeltaBuffer, DeltaCrdt, Dot, DotContext, GCounter, LWWRegister,
    MVRegister, Merge, ORMap, ORSet, PNCounter, RemoveWins, ReplicaId, SetBias, VClock, RGA, YATA,
};
pub use types::{
    Bool, Byte, Char, Int, LogosContains, Map, Nat, Real, Seq, Set, Text, Tuple, Unit, Value,
};
pub use rustc_hash::{FxHashMap, FxHashSet};
pub use indexing::{LogosIndex, LogosIndexMut};
