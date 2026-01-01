//! LOGOS Runtime Library

pub mod io;
pub mod types;
// Phase 53: Virtual File System (cross-platform)
pub mod fs;
// Phase 49: CRDT primitives (cross-platform)
pub mod crdt;
// Phase 55: Persistent storage (cross-platform, uses async-lock)
pub mod storage;
// Phase 56: Distributed<T> - unified persistence + network (cross-platform)
pub mod distributed;
// Phase 57: Polymorphic indexing (Vec + HashMap)
pub mod indexing;

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
// Phase 54: Go-like concurrency primitives (native only)
#[cfg(not(target_arch = "wasm32"))]
pub mod concurrency;

// Phase 51: Re-export tokio for async main support (native only)
#[cfg(not(target_arch = "wasm32"))]
pub use tokio;

pub fn panic_with(reason: &str) -> ! {
    panic!("{}", reason);
}

pub mod fmt {
    pub fn format<T: std::fmt::Display>(x: T) -> String {
        format!("{}", x)
    }
}

pub mod prelude {
    pub use crate::io::{show, read_line, println, eprintln, print, Showable};
    pub use crate::types::{Nat, Int, Real, Text, Bool, Unit, Char, Byte, Seq, Map, Set, LogosContains};
    pub use crate::panic_with;
    pub use crate::fmt::format;
    // Phase 57: Polymorphic indexing traits
    pub use crate::indexing::{LogosIndex, LogosIndexMut};
    // Phase 49: CRDT primitives
    pub use crate::crdt::{GCounter, LWWRegister, Merge};
    // Phase 56: Distributed<T>
    pub use crate::distributed::Distributed;

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
    // Phase 54: Go-like concurrency primitives
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::concurrency::{
        spawn, TaskHandle,
        Pipe, PipeSender, PipeReceiver,
        check_preemption, reset_preemption_timer,
    };
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
