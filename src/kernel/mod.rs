//! The Kernel: Calculus of Constructions
//!
//! A unified type system where terms and types are the same syntactic category.
//! Based on the Calculus of Inductive Constructions (CIC).
//!
//! Core insight: Everything is a Term.
//! - Types are Terms (Nat : Type 0)
//! - Values are Terms (zero : Nat)
//! - Functions are Terms (λx:Nat. x)
//! - Proofs are Terms (refl : a = a)

mod context;
mod error;
pub mod positivity;
pub mod prelude;
mod reduction;
pub mod termination;
mod term;
mod type_checker;

pub use context::Context;
pub use error::{KernelError, KernelResult};
pub use reduction::normalize;
pub use term::{Literal, Term, Universe};
pub use type_checker::{infer_type, is_subtype};
