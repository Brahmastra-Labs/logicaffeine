//! Vernacular interface for the Kernel.
//!
//! Provides a text-based command interface for interacting with the kernel:
//! - `Definition name : type := term.` - Add a definition
//! - `Check term.` - Print the type of a term
//! - `Eval term.` - Normalize and print a term
//! - `Inductive Name := C1 : T1 | C2 : T2.` - Define an inductive type
//!
//! # Literate Syntax (English-like alternative)
//!
//! - `A Bool is either Yes or No.` - Define an inductive type
//! - `## To add (n: Nat) and (m: Nat) -> Nat:` - Define a function
//! - `Consider x: When Zero: Yield m.` - Pattern matching

mod command;
mod command_parser;
mod error;
pub mod literate_parser;
mod repl;
mod term_parser;

pub use command::Command;
pub use command_parser::parse_command;
pub use error::{InterfaceError, ParseError};
pub use repl::Repl;
pub use term_parser::TermParser;
