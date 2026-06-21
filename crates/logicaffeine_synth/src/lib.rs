//! EXODIA Phase 2 — the FORGE's offline proof tooling (D11 layers a + b).
//!
//! Everything here runs at development/CI time, never in the production
//! path: Z3 specifications for the JIT's micro-operations over 64-bit
//! bitvectors, satisfiability and algebraic-property gates, and a witness
//! harness that runs Z3-chosen inputs through the REAL compiled stencil
//! chain, the forge's `reference_eval`, and the spec itself — three
//! independent evaluators that must agree input by input.
//!
//! The spec language is SMT bitvector arithmetic, which matches the
//! kernel's locked Int semantics exactly: `bvsdiv`/`bvsrem` wrap
//! `i64::MIN / -1` and take the dividend's sign just like `wrapping_div`/
//! `wrapping_rem`; shifts mask their amount to the low six bits just like
//! `wrapping_shl(b as u32)`.

pub mod spec;
pub mod witness;

pub use spec::{all_specs, OpSpec, SpecKind};
pub use witness::{check_spec_with_witnesses, WitnessReport};
