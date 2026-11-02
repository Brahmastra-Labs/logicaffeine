//! The target-parametric passes: every one is `fn(&SpatialProgram, &SpatialTarget) -> Result<SpatialProgram, SpatialReject>`,
//! reads only the program and the descriptor, and leaves `validate()` holding. The order is pinned in
//! [`super::pipeline::PASS_ORDER`]. There is no strategy input anywhere: `tile` chooses the plan by the cost
//! model, `fuse_readers` re-derives the same comparison, and every later pass is a function of the structure.

pub mod compose;
pub mod from_q8;
pub mod from_qwen_ops;
pub mod from_tensor_op;
pub mod forward;
pub mod fuse_generic;
pub mod fuse_readers;
pub mod pack;
pub mod persist;
pub mod place;
pub use crate::tensor_graph::front as recognize_stages;
pub mod section_shape;
pub mod shard;
pub mod size_buffers;
pub mod synthesize_handshakes;
pub mod tile;
pub mod verify;

use super::eval::ChoreoFault;
use super::ir::{AccPolicy, GroupId, IrViolation, Site};
use super::target::{HandshakeKind, Interconnect};
use crate::codegen_sva::fabric_target::Precision;
use crate::tensor_recognize::{ElementwiseReject, MatmulReject, ReductionReject};

/// Why a program cannot become (or stay) a spatial program on a target — typed, never a silent fallback.
#[derive(Clone, Debug, PartialEq)]
pub enum SpatialReject {
    Parse(String),
    Unrecognized {
        matmul: MatmulReject,
        elementwise: ElementwiseReject,
        reduction: ReductionReject,
    },
    NotLowered(&'static str),
    MissingCompute {
        needed: &'static str,
        target: &'static str,
    },
    MissingPrecision {
        needed: Precision,
        target: &'static str,
    },
    MissingHandshake {
        needed: HandshakeKind,
        target: &'static str,
    },
    MissingInterconnect {
        needed: Interconnect,
        target: &'static str,
    },
    NoLegalTiling {
        rows_t: usize,
        cols_t: usize,
        k_tiles: usize,
        acc_tiles: usize,
        sites: usize,
    },
    LocalMemExceeded {
        site: Site,
        needed_bytes: usize,
        budget_bytes: usize,
    },
    AccTilesExceeded {
        needed: usize,
        available: usize,
        acc: AccPolicy,
    },
    TooManyEngines {
        site: Site,
        needed: usize,
        available: u8,
    },
    GroupCountMismatch {
        group: GroupId,
        counts: Vec<(Site, usize)>,
    },
    WrongPhase {
        pass: &'static str,
        expected: &'static str,
        found: super::ir::Phase,
    },
    Unverified(ChoreoFault),
    Invalid(IrViolation),
    /// The scalar (decision-fabric) front end refused the body in its own words — an op outside the
    /// compare/arith/branch family, a runtime-bound slice, a value kind it does not carry.
    ScalarFrontEnd(String),
    /// A weight the deployment stores Q8_0 whose declared values are not an int8 multiple of a scale per 32 rows.
    NotQ8_0 {
        weight: String,
    },
}

impl From<IrViolation> for SpatialReject {
    fn from(v: IrViolation) -> Self {
        SpatialReject::Invalid(v)
    }
}

/// The loop variables the choreography addresses tiles with.
pub const LV_ITEM_Y: super::ir::LoopVar = super::ir::LoopVar(0);
pub const LV_ITEM_X: super::ir::LoopVar = super::ir::LoopVar(1);
pub const LV_KBLOCK: super::ir::LoopVar = super::ir::LoopVar(2);
pub const LV_SUB_H: super::ir::LoopVar = super::ir::LoopVar(3);
pub const LV_SUB_W: super::ir::LoopVar = super::ir::LoopVar(4);
pub const LV_ITEM: super::ir::LoopVar = super::ir::LoopVar(5);

/// A [`ComputeOp::Generic`](crate::spatial::ir::ComputeOp::Generic)'s own iteration space: the rows inside a tile, and the columns across the
/// operand's tiles. Distinct from the choreography's loop variables above, which index WORK; these
/// index the ELEMENTS one compute stage walks, and are the axes an access map may omit to broadcast.
///
/// [`ComputeOp::Generic`]: super::ir::ComputeOp::Generic
pub const LV_GEN_ROW: super::ir::LoopVar = super::ir::LoopVar(6);
pub const LV_GEN_COL: super::ir::LoopVar = super::ir::LoopVar(7);
/// A [`ComputeOp::Generic`]'s CONTRACTION axis: the third iteration variable, summed at every `(row, col)` point —
/// `C[r, c] = Σ_k A[r, k]·B[k, c]` — so a small matrix product is one stage. Maps step along it; the output never does.
///
/// [`ComputeOp::Generic`]: super::ir::ComputeOp::Generic
pub const LV_GEN_K: super::ir::LoopVar = super::ir::LoopVar(8);
