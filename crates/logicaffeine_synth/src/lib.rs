#![doc = include_str!("../README.md")]

pub mod reversible;
pub mod spec;
pub mod witness;

pub use reversible::{all_rev_specs, prove_bijective, prove_swap_involution, RevSpec};
pub use spec::{all_specs, OpSpec, SpecKind};
pub use witness::{check_spec_with_witnesses, WitnessReport};
