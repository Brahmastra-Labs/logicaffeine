//! The kernel core: a type theory over a hash-consed expression DAG, in two modes
//! ([`tc::Theory`]): Lean 4's theory exactly, and LOGOS's Calculus of Inductive Constructions.
//!
//! This is the representation the kernel needs to be as fast as the fastest Lean kernels:
//! de Bruijn indices, one id per structurally distinct term, loose-bound-variable ranges and
//! free-variable flags cached per node, and every cache keyed on ids. In Lean's theory it checks
//! `lean4export` streams directly ([`export::check_export`]). In LOGOS's it checks the declarations
//! of a [`Context`](crate::Context) ([`logos_env::replay`], [`logos_env::admit`]) with LOGOS's own
//! rules ([`logos_tc`]); LOGOS's proof surfaces are still decided by the [`Term`](crate::Term)
//! kernel and [`recheck`](mod@crate::recheck), against which the core's verdicts are locked.
//!
//! Layout: [`name`], [`level`] and [`expr`] are the interned stores; [`env`](mod@env) the admitted
//! declarations; [`tc`] inference, reduction and definitional equality by substitution, and
//! [`sem`] the same judgment by evaluation into closures (Lean's theory); [`check`], [`inductive`]
//! and [`primitive`] the admission rules; [`json`] and [`export`] the streaming reader; [`logos`]
//! the lowering of LOGOS terms into the store, [`logos_tc`] LOGOS's rules and [`logos_env`] its
//! declarations; [`mirror`] keeps a core in step with a context through the context's admission
//! log.

pub mod check;
pub mod env;
pub mod error;
pub mod export;
pub mod expr;
pub mod inductive;
pub mod json;
pub mod level;
pub mod logos;
pub mod logos_env;
pub(crate) mod logos_obs;
pub mod logos_tc;
pub mod mirror;
pub mod name;
pub mod nat;
/// The closure-based conversion engine, compiled only with the `nbe` feature while it is built.
#[cfg(feature = "nbe")]
pub mod nbe;
pub mod pretty;
pub mod primitive;
pub mod primitive_wf;
pub mod relevance;
pub mod sem;
pub(crate) mod slot;
pub mod tc;
pub mod wellknown;

pub use error::{FailKind, KResult, KernelFailure};
pub use export::{
    check_export, check_export_in, check_export_observed, check_export_sized, check_export_with,
    Verdict,
};
pub use tc::Kernel;
