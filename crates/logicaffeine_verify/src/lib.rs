#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod equivalence;
pub mod consistency;
pub mod error;
pub mod ir;
pub mod license;
pub mod solver;
pub mod type_infer;
pub mod kinduction;
pub mod interpolation;
pub mod liveness;
pub mod ic3;
pub mod compositional;
pub mod strategy;
pub mod security;
pub mod multiclock;
pub mod parameterized;
pub mod smtlib;
pub mod certificate;
pub mod incremental;
pub mod abstraction;
pub mod automata;
pub mod synthesis;

pub use equivalence::{check_equivalence, EquivalenceResult, Trace, CycleState, SignalValue};
pub use consistency::{
    check_consistency, check_spec_consistency,
    ConsistencyResult, ConsistencyReport, ConsistencyConfig,
    LabeledFormula, SatisfiabilityResult,
    VacuityFinding, RedundancyFinding, PairwiseConflict,
};
pub use error::{VerificationError, VerificationErrorKind, VerificationResult};
pub use ir::{BitVecOp, VerifyExpr, VerifyOp, VerifyType};
pub use license::{LicensePlan, LicenseValidator};
pub use solver::{rename_var_in_expr, Verifier, VerificationSession};
