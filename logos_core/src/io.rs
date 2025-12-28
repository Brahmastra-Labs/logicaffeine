//! IO Operations (Spec 10.5)

use std::fmt::{self, Display};

/// Custom trait for LOGOS Show verb - provides clean, natural output.
/// Primitives display without quotes, collections display with brackets.
pub trait Showable {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result;
}

// Primitives: use Display formatting
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

/// The Show verb - prints value with natural formatting
pub fn show<T: Showable>(value: T) {
    struct Wrapper<'a, T>(&'a T);
    impl<T: Showable> Display for Wrapper<'_, T> {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            self.0.format_show(f)
        }
    }
    println!("{}", Wrapper(&value));
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
