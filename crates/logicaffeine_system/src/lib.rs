#![cfg_attr(docsrs, feature(doc_cfg))]

//! Platform IO and System Services for LOGOS
//!
//! This crate provides platform-specific IO operations with feature-gated heavy dependencies.
//!
//! ## Feature Flags (Cerf/Drasner Amendment)
//!
//! - (default): Lean build with basic IO only
//! - `networking`: P2P networking via libp2p (large dependency)
//! - `persistence`: File persistence with memmap2 (small dependency)
//! - `concurrency`: Parallel computation with rayon (moderate dependency)
//! - `full`: All features enabled
//! - `distributed`: networking + persistence (for Distributed<T>)

// === Always Available (Core IO) ===
pub mod io;
pub mod temporal;

// Native-only core modules
#[cfg(not(target_arch = "wasm32"))]
pub mod time;
#[cfg(not(target_arch = "wasm32"))]
pub mod env;
#[cfg(not(target_arch = "wasm32"))]
pub mod random;
#[cfg(not(target_arch = "wasm32"))]
pub mod text;

// === Feature-Gated Modules ===

// Persistence feature: file operations, storage, VFS
#[cfg(feature = "persistence")]
pub mod file;
#[cfg(feature = "persistence")]
pub mod fs;
#[cfg(feature = "persistence")]
pub mod storage;

// Networking feature: P2P networking
#[cfg(feature = "networking")]
pub mod network;

// Concurrency feature: parallel computation
#[cfg(feature = "concurrency")]
pub mod concurrency;
#[cfg(feature = "concurrency")]
pub mod memory;

// Distributed<T> requires both networking AND persistence
#[cfg(all(feature = "networking", feature = "persistence"))]
pub mod distributed;

// CRDT sync wrapper requires networking (uses tokio + libp2p)
#[cfg(feature = "networking")]
pub mod crdt;

// Re-export tokio for async main support (native only)
#[cfg(not(target_arch = "wasm32"))]
pub use tokio;

// Re-export commonly used items
pub use io::{show, read_line, println, eprintln, print, Showable};
pub use temporal::{LogosDate, LogosMoment, LogosSpan};

/// Panic with a custom message (used by generated LOGOS code)
pub fn panic_with(reason: &str) -> ! {
    panic!("{}", reason);
}

/// Formatting utilities
pub mod fmt {
    pub fn format<T: std::fmt::Display>(x: T) -> String {
        format!("{}", x)
    }
}
