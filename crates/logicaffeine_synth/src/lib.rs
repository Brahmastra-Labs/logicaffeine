#![doc = include_str!("../README.md")]

pub mod spec;
pub mod witness;

pub use spec::{all_specs, OpSpec, SpecKind};
pub use witness::{check_spec_with_witnesses, WitnessReport};
