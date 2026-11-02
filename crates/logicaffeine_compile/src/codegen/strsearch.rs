//! Naive-substring-search idiom: recognizer + emitted SIMD runtime kernel.
//!
//! A doubly-nested "for each start position, does the fixed window equal the
//! needle? if so, count it" loop *is* an overlapping occurrence count. When the
//! AOT path proves that shape is what a loop nest computes — the needle is
//! loop-invariant, the counter only ever takes `+1` under a full-window match,
//! the body has no other effect, and the scan covers exactly the valid start
//! positions — it replaces the whole nest with a call to a SIMD kernel that
//! scans for the needle's first byte with a vector compare and verifies each
//! candidate window.
//!
//! The kernel lives in [`strsearch_kernel.rs`](./strsearch_kernel.rs), the
//! single source of truth: [`RUNTIME_SRC`] embeds its text to emit into the
//! generated program (which cannot link the compiler), and the test module
//! `include!`s the same text so the differential fuzz suite exercises exactly
//! what ships.

/// The kernel source, emitted verbatim into a generated program's prelude when
/// [`try_emit_naive_search`] fires. Defines `__logos_count_window_matches`.
pub(crate) const RUNTIME_SRC: &str = include_str!("strsearch_kernel.rs");

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen/strsearch/tests.rs"]
mod tests;
