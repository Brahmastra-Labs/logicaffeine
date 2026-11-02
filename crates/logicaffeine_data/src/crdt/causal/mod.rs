//! Causal infrastructure for delta-state CRDTs.
//!
//! Foundation for tracking causality in distributed systems.

mod context;
mod dot;
mod vclock;

pub use context::DotContext;
pub use dot::Dot;
pub use vclock::VClock;
