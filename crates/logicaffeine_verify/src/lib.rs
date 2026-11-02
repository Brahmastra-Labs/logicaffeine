#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod abstraction;
pub mod automata;
pub mod certificate;
pub mod compositional;
pub mod consistency;
pub mod equivalence;
pub mod error;
pub mod ic3;
pub mod incremental;
pub mod interpolation;
pub mod ir;
pub mod kinduction;
pub mod liveness;
pub mod multiclock;
pub mod parameterized;
pub mod security;
pub mod smtlib;
pub mod solver;
pub mod strategy;
pub mod synthesis;
pub mod type_infer;

pub use consistency::{
    check_consistency, check_spec_consistency, ConsistencyConfig, ConsistencyReport,
    ConsistencyResult, LabeledFormula, PairwiseConflict, RedundancyFinding, SatisfiabilityResult,
    VacuityFinding,
};
pub use equivalence::{check_equivalence, CycleState, EquivalenceResult, SignalValue, Trace};
pub use error::{VerificationError, VerificationErrorKind, VerificationResult};
pub use ir::{BitVecOp, VerifyExpr, VerifyOp, VerifyType};
pub use solver::{rename_var_in_expr, VerificationSession, Verifier};
