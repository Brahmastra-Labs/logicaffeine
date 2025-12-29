//! Phase 48: Network primitives for LOGOS distributed system.
//!
//! This module provides zero-copy file chunking and resumable transfer protocols.

mod sipping;

pub use sipping::{FileSipper, FileManifest, FileChunk, DEFAULT_CHUNK_SIZE};
