//! Semantic transformations for logical expressions.
//!
//! This module provides post-parse semantic transformations:
//!
//! - **[`apply_axioms`]**: Expands predicates with entailments and hypernymy
//! - **[`apply_kripke_lowering`]**: Converts modals to explicit world quantification
//!
//! These transformations enrich the logical representation with inferred content.

mod axioms;
mod kripke;

pub use axioms::apply_axioms;
pub use kripke::apply_kripke_lowering;

include!(concat!(env!("OUT_DIR"), "/axiom_data.rs"));
