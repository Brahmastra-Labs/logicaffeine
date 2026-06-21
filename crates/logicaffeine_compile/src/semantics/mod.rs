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
/// past this is a catchable runtime error, not a host crash. Set to 2500 (up
/// from the old 1000) so naive deep recursion like ackermann(3,8) — depth ~2045
/// — runs in the interpreter, closer to Node/V8's recursion headroom. The VM is
/// heap-stacked (depth-safe); the native-recursion engines stay within their
/// stacks at this depth (JIT ~2500 frames is a couple MB; the tree-walker's
/// depth tests run on a big-stack thread).
pub const MAX_CALL_DEPTH: usize = 2_500;
/// The canonical depth-exceeded error.
pub const CALL_DEPTH_ERR: &str = "Stack overflow: maximum call depth exceeded";
