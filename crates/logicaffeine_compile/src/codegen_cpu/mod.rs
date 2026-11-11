//! The CPU tier for the one IR (PARITY-CPU-SHIM): each CPU instruction set's answer to the shared optimizer
//! ([`shim::CpuShim`]), and the link that emits a model's program for the host's cores.

pub mod kernels;
pub mod link;
pub mod model;
pub mod shim;
