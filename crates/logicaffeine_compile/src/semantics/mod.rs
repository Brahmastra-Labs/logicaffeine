//! The shared semantics kernel.
//!
//! ONE implementation of LOGOS value semantics — arithmetic, comparison,
//! equality, collections, builtins — used by BOTH the tree-walker interpreter
//! and the bytecode VM (and later the JIT's slow paths). Routing every engine
//! through this module makes behavioral divergence structurally impossible,
//! and every runtime error string for these operations lives here.

pub mod acceptance;
pub mod arith;
pub mod builtin_overloads;
pub mod builtins;
pub mod collections;
pub mod compare;
pub mod crdt;
pub mod dom;
pub mod drive;
pub mod escape;
pub(crate) mod defined;
pub mod exact;
pub mod failcode;
pub mod format;
pub mod native_kernels;
pub mod operator_overloads;
pub mod patch;
pub mod policy;
pub mod storage;
pub mod temporal;
pub mod world;

/// Maximum LOGOS call depth, enforced identically by every engine: recursion
/// past this is a catchable runtime error, not a host crash. Set to 2500 (up
/// from the old 1000) so naive deep recursion like ackermann(3,8) — depth ~2045
/// — runs in the interpreter, closer to Node/V8's recursion headroom. The VM is
/// heap-stacked (depth-safe); the native-recursion engines stay within their
/// stacks at this depth (JIT ~2500 frames is a couple MB; the tree-walker's
/// depth tests run on a big-stack thread).
pub const MAX_CALL_DEPTH: usize = 2_500;
/// The canonical depth-exceeded error.
pub const CALL_DEPTH_ERR: &str = logicaffeine_data::failure::CALL_DEPTH_EXCEEDED;

/// The program name of a run that has no argv of its own — an in-process engine, a chip, a circuit.
pub const PROGRAM_NAME: &str = "logos";

/// The argument vector `args()` answers: `argv` as C, Python and Go define it — the program name
/// first, then the real arguments. A run given no vector at all (the in-process engines) still
/// has a program name, [`PROGRAM_NAME`], so `item 2 of args()` is always the first real argument
/// on every tier and `length of args()` is never 0.
pub fn argv_with_program_name(args: Vec<String>) -> Vec<String> {
    if args.is_empty() {
        vec![PROGRAM_NAME.to_string()]
    } else {
        args
    }
}
