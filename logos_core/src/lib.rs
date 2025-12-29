//! LOGOS Runtime Library

pub mod io;
pub mod types;
pub mod file;
pub mod time;
pub mod random;
pub mod env;
pub mod memory;
// Phase 48: Network primitives
pub mod network;

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
    pub use crate::file::{read, write};
    pub use crate::time::{now, sleep};
    pub use crate::random::{randomInt, randomFloat};
    pub use crate::env::{get, args};
    // Phase 43D: Collection indexing helpers
    pub use crate::logos_index;
    pub use crate::logos_index_mut;
    // Phase 8.5: Zone-based memory management
    pub use crate::memory::Zone;
    // Phase 48: Sipping protocol
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
