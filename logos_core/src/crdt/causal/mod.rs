//! Causal infrastructure for delta-state CRDTs
//!
//! Wave 1: Foundation for tracking causality in distributed systems.

mod dot;
mod vclock;
mod context;

pub use dot::Dot;
pub use vclock::VClock;
pub use context::DotContext;
