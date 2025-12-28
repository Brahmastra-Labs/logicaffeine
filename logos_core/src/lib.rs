//! LOGOS Runtime Library

pub mod io;
pub mod types;

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
    pub use crate::types::{Nat, Int, Real, Text, Bool, Unit, Seq};
    pub use crate::panic_with;
    pub use crate::fmt::format;
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
