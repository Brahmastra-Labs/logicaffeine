#![cfg_attr(docsrs, feature(doc_cfg))]

//! WASM-safe data structures and CRDTs
//!
//! The value layer shared by the rest of the workspace: the runtime [`Value`]
//! types LOGOS programs manipulate, the conflict-free replicated data types
//! (CRDTs) that let those values converge across replicas, and the indexing
//! traits that make them subscriptable. Everything here is pure — it compiles
//! identically for native and `wasm32-unknown-unknown` because it never touches
//! the clock, the network, or the filesystem.
//!
//! # Modules
//!
//! - [`types`] — the runtime value universe ([`Value`], [`Int`], [`Nat`],
//!   [`Real`], [`Text`], [`Seq`], [`Map`], [`Set`], [`Tuple`], [`Unit`], …)
//!   plus the [`LogosContains`] membership trait.
//! - [`crdt`] — replicated state: counters ([`GCounter`], [`PNCounter`]),
//!   registers ([`LWWRegister`], [`MVRegister`]), sets and maps ([`ORSet`],
//!   [`ORMap`]), sequences ([`RGA`], [`YATA`]), and the causal machinery
//!   ([`Dot`], [`DotContext`], [`VClock`]) they share. All converge through the
//!   [`Merge`] trait.
//! - [`indexing`] — the [`LogosIndex`], [`LogosIndexMut`], and [`LogosGetChar`]
//!   traits backing indexed access into sequences, maps, and text.
//!
//! # Example
//!
//! Two replicas increment a grow-only counter independently, then converge by
//! merging. [`Merge`] is commutative, associative, and idempotent, so the order
//! and number of merges never changes the result:
//!
//! ```
//! use logicaffeine_data::{GCounter, Merge};
//!
//! let mut replica_a = GCounter::with_replica_id(1);
//! let mut replica_b = GCounter::with_replica_id(2);
//!
//! replica_a.increment(3);
//! replica_b.increment(5);
//!
//! replica_a.merge(&replica_b);
//! assert_eq!(replica_a.value(), 8);
//! ```
//!
//! # LAMPORT INVARIANT
//!
//! This crate has NO path to system IO. No tokio, no libp2p, no SystemTime.
//! Timestamps are injected by callers (typically from `logicaffeine_system`).

pub mod crdt;
pub mod indexing;
pub mod types;

// Re-export commonly used types
pub use crdt::{
    generate_replica_id, AddWins, DeltaBuffer, DeltaCrdt, Dot, DotContext, GCounter, LWWRegister,
    MVRegister, Merge, ORMap, ORSet, PNCounter, RemoveWins, ReplicaId, SetBias, VClock, RGA, YATA,
};
pub use types::{
    Bool, Byte, Char, Int, LogosContains, LogosDenseI64Map, LogosDenseI64MapNoPresence,
    LogosDenseI64Set, LogosDivU64, LogosI32Map, LogosI32Set, LogosI64Map, LogosI64Set, LogosMap,
    LogosRational, Map, Nat, Real, Seq, Set, Text, Tuple, Unit, Value, LogosSeq,
};
pub use rustc_hash::{FxHashMap, FxHashSet};
pub use indexing::{LogosGetChar, LogosIndex, LogosIndexMut};
