//! LOGOS Runtime Library

pub mod io;
pub mod types;
// Phase 53: Virtual File System (cross-platform)
pub mod fs;
// Phase 49: CRDT primitives (cross-platform)
pub mod crdt;
// Phase 55: Persistent storage (cross-platform, uses async-lock)
pub mod storage;

// Native-only modules
#[cfg(not(target_arch = "wasm32"))]
pub mod file;
#[cfg(not(target_arch = "wasm32"))]
pub mod time;
#[cfg(not(target_arch = "wasm32"))]
pub mod random;
#[cfg(not(target_arch = "wasm32"))]
pub mod env;
#[cfg(not(target_arch = "wasm32"))]
pub mod memory;
#[cfg(not(target_arch = "wasm32"))]
pub mod network;

// Phase 51: Re-export tokio for async main support (native only)
#[cfg(not(target_arch = "wasm32"))]
pub use tokio;

pub fn panic_with(reason: &str) -> ! {
    panic!("{}", reason);
}

/// Phase 43D: 1-based indexing with clear error messages
///
/// LOGOS uses 1-based indexing to match natural language ("the first item").
/// This function converts 1-based indices to 0-based and provides helpful
/// error messages for out-of-bounds access.
#[inline]
pub fn logos_index<T: Copy>(slice: &[T], index: i64) -> T {
    if index < 1 {
        panic!("Index {} is invalid: LOGOS uses 1-based indexing (minimum index is 1)", index);
    }
    let idx = (index - 1) as usize;
    if idx >= slice.len() {
        panic!("Index {} is out of bounds for seq of length {}", index, slice.len());
    }
    slice[idx]
}

/// Phase 43D: 1-based mutable indexing with clear error messages
#[inline]
pub fn logos_index_mut<T>(slice: &mut [T], index: i64) -> &mut T {
    if index < 1 {
        panic!("Index {} is invalid: LOGOS uses 1-based indexing (minimum index is 1)", index);
    }
    let idx = (index - 1) as usize;
    if idx >= slice.len() {
        panic!("Index {} is out of bounds for seq of length {}", index, slice.len());
    }
    &mut slice[idx]
}

pub mod fmt {
    pub fn format<T: std::fmt::Display>(x: T) -> String {
        format!("{}", x)
    }
}

pub mod prelude {
    pub use crate::io::{show, read_line, println, eprintln, print, Showable};
    pub use crate::types::{Nat, Int, Real, Text, Bool, Unit, Seq};
    pub use crate::panic_with;
    pub use crate::fmt::format;
    // Phase 43D: Collection indexing helpers
    pub use crate::logos_index;
    pub use crate::logos_index_mut;
    // Phase 49: CRDT primitives
    pub use crate::crdt::{GCounter, LWWRegister, Merge};

    // Native-only prelude exports
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::file::{read, write};
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::time::{now, sleep};
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::random::{randomInt, randomFloat};
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::env::{get, args};
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::memory::Zone;
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::network::{FileSipper, FileManifest, FileChunk};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format() {
        assert_eq!(fmt::format(42), "42");
        assert_eq!(fmt::format("hello"), "hello");
    }

    #[test]
    fn test_type_aliases() {
        let _n: types::Nat = 42;
        let _i: types::Int = -42;
        let _r: types::Real = 3.14;
        let _t: types::Text = String::from("hello");
        let _b: types::Bool = true;
        let _u: types::Unit = ();
    }
}
