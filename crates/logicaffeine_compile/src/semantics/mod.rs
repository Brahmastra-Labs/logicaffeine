//! The shared semantics kernel.
//!
//! ONE implementation of LOGOS value semantics — arithmetic, comparison,
//! equality, collections, builtins — used by BOTH the tree-walker interpreter
//! and the bytecode VM (and later the JIT's slow paths). Routing every engine
//! through this module makes behavioral divergence structurally impossible,
//! and every runtime error string for these operations lives here.

pub mod arith;
pub mod builtins;
pub mod collections;
pub mod compare;
pub mod format;
pub mod policy;
pub mod temporal;

/// Maximum LOGOS call depth, enforced identically by every engine: recursion
/// past this is a catchable runtime error, not a host crash.
pub const MAX_CALL_DEPTH: usize = 1_000;
/// The canonical depth-exceeded error.
pub const CALL_DEPTH_ERR: &str = "Stack overflow: maximum call depth exceeded";
