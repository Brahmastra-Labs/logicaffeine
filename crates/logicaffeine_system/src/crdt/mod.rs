//! CRDT Synchronization Wrappers
//!
//! This module provides the `Synced<T>` wrapper which adds automatic
//! network synchronization to CRDT types from logicaffeine_data.
//!
//! Requires the `networking` feature to be enabled.

mod sync;

pub use sync::Synced;
