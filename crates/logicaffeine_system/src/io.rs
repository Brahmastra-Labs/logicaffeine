//! I/O Operations for LOGOS Programs
//!
//! Provides input/output primitives for LOGOS programs including:
//!
//! - [`show`]: Natural formatting output (primitives without quotes, collections with brackets)
//! - [`print`], [`println`], [`eprintln`]: Standard output functions
//! - [`read_line`]: Read a line from stdin
//!
//! The [`Showable`] trait enables custom types to integrate with the `show` verb.
//!
//! # Example
//!
//! ```no_run
//! use logicaffeine_system::io::{show, println, read_line};
//!
//! // Natural formatting with show
//! show(&42);           // Prints: 42
//! show(&"hello");      // Prints: hello (no quotes)
//! show(&vec![1, 2, 3]); // Prints: [1, 2, 3]
//!
//! // Standard output
//! println("Enter your name:");
//! let name = read_line();
//! println(format!("Hello, {}!", name));
//! ```

use std::fmt::{self, Display};

/// Custom trait for LOGOS Show verb - provides clean, natural output.
/// Primitives display without quotes, collections display with brackets.
pub trait Showable {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result;
}

// Primitives: use Display formatting
impl Showable for i32 {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for i64 {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for u64 {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for usize {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for f64 {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for bool {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for u8 {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for char {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for String {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for &str {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

// Sequences: bracket notation with recursive formatting
impl<T: Showable> Showable for Vec<T> {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[")?;
        for (i, item) in self.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            item.format_show(f)?;
        }
        write!(f, "]")
    }
}

// Slices: same as Vec
impl<T: Showable> Showable for [T] {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[")?;
        for (i, item) in self.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            item.format_show(f)?;
        }
        write!(f, "]")
    }
}

// Reference to slice
impl<T: Showable> Showable for &[T] {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        (*self).format_show(f)
    }
}

// Option type: shows "nothing" or the value
impl<T: Showable> Showable for Option<T> {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Some(v) => v.format_show(f),
            None => write!(f, "nothing"),
        }
    }
}

// CRDT types: show the value
impl Showable for logicaffeine_data::crdt::GCounter {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.value())
    }
}

impl Showable for logicaffeine_data::crdt::PNCounter {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.value())
    }
}

// LWWRegister: show the current value
impl<T: Showable> Showable for logicaffeine_data::crdt::LWWRegister<T> {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.get().format_show(f)
    }
}

// MVRegister: show single value or conflict notation
impl<T: Showable + Clone + PartialEq> Showable for logicaffeine_data::crdt::MVRegister<T> {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let values = self.values();
        if values.len() == 1 {
            values[0].format_show(f)
        } else if values.is_empty() {
            write!(f, "nothing")
        } else {
            // Multiple concurrent values - show as conflict
            write!(f, "conflict[")?;
            for (i, val) in values.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                val.format_show(f)?;
            }
            write!(f, "]")
        }
    }
}

// Dynamic Value type for tuples
impl Showable for logicaffeine_data::Value {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

// Temporal types: Duration with human-readable formatting
impl Showable for std::time::Duration {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let nanos = self.as_nanos();
        if nanos >= 3_600_000_000_000 {
            // Hours
            write!(f, "{}h", nanos / 3_600_000_000_000)
        } else if nanos >= 60_000_000_000 {
            // Minutes
            write!(f, "{}min", nanos / 60_000_000_000)
        } else if nanos >= 1_000_000_000 {
            // Seconds
            write!(f, "{}s", nanos / 1_000_000_000)
        } else if nanos >= 1_000_000 {
            // Milliseconds
            write!(f, "{}ms", nanos / 1_000_000)
        } else if nanos >= 1_000 {
            // Microseconds
            write!(f, "{}μs", nanos / 1_000)
        } else {
            // Nanoseconds
            write!(f, "{}ns", nanos)
        }
    }
}

/// The Show verb - prints value with natural formatting
/// Takes a reference to avoid moving the value.
pub fn show<T: Showable>(value: &T) {
    struct Wrapper<'a, T>(&'a T);
    impl<T: Showable> Display for Wrapper<'_, T> {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            self.0.format_show(f)
        }
    }
    println!("{}", Wrapper(value));
}

pub fn read_line() -> String {
    let mut buffer = String::new();
    std::io::stdin().read_line(&mut buffer).unwrap_or(0);
    buffer.trim().to_string()
}

pub fn print<T: Display>(x: T) {
    print!("{}", x);
}

pub fn eprintln<T: Display>(x: T) {
    eprintln!("{}", x);
}

pub fn println<T: Display>(x: T) {
    println!("{}", x);
}
