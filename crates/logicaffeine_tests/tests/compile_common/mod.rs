//! Shared test infrastructure for the compile crate.
//!
//! Kept deliberately small: a helper belongs here only when several tests need the SAME answer to
//! the same question, and a wrong answer would be invisible in any one of them.
#![allow(dead_code)]

#[cfg(feature = "wasm-jit")]
use logicaffeine_compile;

/// The wasm host imports whose behaviour the ABI fixes. Gated because it needs the `wasmi` engine,
/// which only the `wasm-jit` feature brings in — a test file may declare `mod common;`
/// unconditionally and simply find nothing inside it in a pass that has no wasm.
#[cfg(feature = "wasm-jit")]
pub mod wasm_host;
