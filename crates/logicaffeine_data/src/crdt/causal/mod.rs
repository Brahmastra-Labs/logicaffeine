//! Causal infrastructure for delta-state CRDTs.
//!
//! Foundation for tracking causality in distributed systems.

mod dot;
mod vclock;
mod context;

pub use dot::Dot;
pub use vclock::VClock;
pub use context::DotContext;
