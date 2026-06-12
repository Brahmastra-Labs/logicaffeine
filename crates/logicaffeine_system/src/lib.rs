#![cfg_attr(docsrs, feature(doc_cfg))]

//! Platform IO and System Services for LOGOS
//!
//! The effectful counterpart to `logicaffeine_data`. Where that crate holds the
//! pure, WASM-safe value types, this crate owns every interaction with the
//! outside world — the clock, the console, the filesystem, the network — and
//! gates the heavy dependencies behind features so a lean build (or a `wasm32`
//! build) pays only for what it uses.
//!
//! ## Modules
//!
//! Always available:
//! - `io` — console and stream IO, plus the `Showable` rendering used to print
//!   runtime values.
//! - `temporal` — clock-agnostic time arithmetic over caller-injected timestamps.
//!
//! Native only (`#[cfg(not(target_arch = "wasm32"))]`):
//! - `time`, `env`, `random`, `text` — wall-clock access, environment, RNG, and
//!   host text services that have no portable `wasm32` equivalent.
//!
//! Feature-gated (see below): `file`, `fs`, `storage` (persistence); `network`,
//! `crdt` (networking); `concurrency`, `memory` (concurrency); and `distributed`,
//! which needs both networking and persistence.
//!
//! ## Feature Flags (Cerf/Drasner Amendment)
//!
//! - (default): Lean build with basic IO only
//! - `networking`: P2P networking via libp2p (large dependency)
//! - `persistence`: File persistence with memmap2 (small dependency)
//! - `concurrency`: Parallel computation with rayon (moderate dependency)
//! - `full`: All features enabled
//! - `distributed`: networking + persistence (for `Distributed<T>`)

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
