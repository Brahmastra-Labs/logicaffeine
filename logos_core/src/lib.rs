//! LOGOS Runtime Library
//!
//! This crate provides the runtime support for compiled LOGOS programs.
//! It wraps Rust's standard library macros into callable functions,
//! since LOGOS FFI cannot invoke macros directly.

pub mod io {
    /// Print a value to stdout with a newline.
    /// LOGOS: `Show x to the console.`
    pub fn println<T: std::fmt::Display>(x: T) {
        println!("{}", x);
    }

    /// Print a value to stderr with a newline.
    /// LOGOS: `Log message.`
    pub fn eprintln<T: std::fmt::Display>(x: T) {
        eprintln!("{}", x);
    }

    /// Print a value to stdout without a newline.
    pub fn print<T: std::fmt::Display>(x: T) {
        print!("{}", x);
    }
}

/// Panic with a reason message.
/// LOGOS: `Panic with reason.`
pub fn panic_with(reason: &str) -> ! {
    panic!("{}", reason);
}

pub mod fmt {
    /// Format a value to a String.
    /// LOGOS: `Format "template [x]".`
    pub fn format<T: std::fmt::Display>(x: T) -> String {
        format!("{}", x)
    }
}

pub mod prelude {
    pub use super::io::{println, eprintln, print};
    pub use super::panic_with;
    pub use super::fmt::format;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format() {
        assert_eq!(fmt::format(42), "42");
        assert_eq!(fmt::format("hello"), "hello");
    }
}
