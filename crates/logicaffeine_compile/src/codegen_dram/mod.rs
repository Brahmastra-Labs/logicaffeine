//! codegen_dram — the DRAM compute-in-memory target (ComputeDRAM bulk bitwise + RowClone).
//!
//! DRAM-via-ComputeDRAM is a CO-PROCESSOR target, like [`crate::codegen_ana`]: it realizes a
//! restricted op subset (the charge-sharing sense amps do bulk bitwise AND/OR and RowClone), and
//! everything else runs on the host/fabric control side. So it uses the same FOUR-way split as the
//! analog tier:
//!
//! - **OnPlane** — bitwise/copy the DRAM sense amps compute in place, each with a witnessed
//!   [`ComputeDramOp`](crate::codegen_sva::dram::ComputeDramOp) realization proven ≡VM by the
//!   [`RowStore`](crate::codegen_sva::dram::RowStore) oracle and the
//!   [`unified_placement`](crate::codegen_sva::unified_placement) equivalence locks;
//! - **Boundary** — data staging: operand rows written into the DRAM, result rows read back out;
//! - **ControlPlane** — digital-exact ops the host runs VM-identically (arithmetic, comparisons,
//!   control flow — the DRAM has no ALU and no sequencer), refusing nothing;
//! - **OffPlane(reason)** — refuses the whole DRAM offload (concurrency / networking — there is no
//!   scheduler or host-IO story inside a memory device).
//!
//! [`fragment`] is an EXHAUSTIVE match on `Op` (no `_` arm): a new VM instruction fails to compile
//! this crate until it is deliberately classified — the anti-drift pit of success. [`dram_prim`]
//! ties every OnPlane op to the exact ComputeDRAM primitive that realizes it, and the op-coverage
//! ratchet (`tests/dram_op_coverage_lock.rs`) proves that primitive is DRAM-capable and ≡VM. You
//! cannot mark an op OnPlane without a proven in-DRAM realization.
//!
//! Beyond the single-op layer, [`dataflow`] is the SPATIAL-DATAFLOW layer: the hardware basis
//! `{Copy, And, Or}` (ComputeDRAM) + `Not` (Ambit) is functionally complete, so chaining primitives
//! through intermediate rows realizes any boolean function and, bit-serially, integer arithmetic —
//! the memory computes a dataflow graph in place.

pub mod dataflow;
pub mod kernels;
pub mod set_algebra;

use crate::codegen_sva::unified_placement::PrimKind;
use crate::vm::{CompiledProgram, Op};

/// Why the DRAM tier declines a program — a SOUND refusal, never wrong memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DramLowerError {
    /// The program uses an op with no home in a memory device (concurrency / networking).
    Unsupported(&'static str),
    /// The program has no bulk-bitwise/copy work to offload into the DRAM plane.
    NoBitwiseRegion,
    /// An OnPlane op has no witnessed ComputeDRAM primitive — a named lowering gap, never a
    /// silent drop.
    UnloweredOp(String),
}

impl std::fmt::Display for DramLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DramLowerError::Unsupported(what) => write!(f, "unsupported {what}"),
            DramLowerError::NoBitwiseRegion => write!(
                f,
                "no DRAM region: the program has no bulk-bitwise/copy work to offload into the DRAM plane"
            ),
            DramLowerError::UnloweredOp(what) => write!(f, "unlowered op: {what}"),
        }
    }
}

impl std::error::Error for DramLowerError {}

/// Where an op lives in the DRAM-co-processor model.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DramFragment {
    OnPlane,
    Boundary,
    ControlPlane,
    OffPlane(&'static str),
}

/// ★ THE SINGLE SOURCE OF TRUTH ★ for how the DRAM tier classifies every VM op.
/// EXHAUSTIVE on `Op` (no `_` arm) — a new instruction cannot escape classification.
pub fn fragment(op: &Op) -> DramFragment {
    use DramFragment::{Boundary as Bd, ControlPlane as Cp, OffPlane as Off, OnPlane as On};
    match op {
        // ── OnPlane: the sense amps compute this in place (bulk bitwise + RowClone). ──
        // BitAnd/BitOr are the sense-amp majority MAJ(a,b,0)/MAJ(a,b,1); the set ops are the same
        // reduction over a bitset representation (intersection = AND, union = OR); DeepClone is
        // RowClone — all UNMODIFIED ComputeDRAM. BitXor needs the AMBIT tier (dual-contact NOT +
        // MAJ) — a minor-DRAM-modification hardware assumption flagged by [`dram_tier`]. Each has a
        // proven in-DRAM primitive — see [`dram_prim`].
        Op::BitAnd { .. } | Op::BitOr { .. } | Op::BitXor { .. } => On,
        Op::IntersectOp { .. } | Op::UnionOp { .. } => On,
        Op::DeepClone { .. } => On,

        // ── Boundary: staging operand rows into the plane and reading results back out. ──
        Op::LoadConst { .. } | Op::Move { .. } | Op::Show { .. } => Bd,
        Op::NewList { .. }
        | Op::NewEmptyList { .. }
        | Op::NewEmptyListWithCapacity { .. }
        | Op::NewEmptyListFixed { .. }
        | Op::NewEmptyListI32 { .. }
        | Op::NewEmptySet { .. }
        | Op::NewEmptyMap { .. }
        | Op::NewEmptyMapWithCapacity { .. }
        | Op::NewRange { .. } => Bd,
        Op::ListPush { .. } | Op::SetAdd { .. } => Bd,
        Op::Index { .. } | Op::IndexUnchecked { .. } | Op::Length { .. } => Bd,
        Op::IterPrepare { .. } | Op::IterNext { .. } | Op::IterPop => Bd,

        // ── ControlPlane: digital-exact — the host runs these VM-identically (no ALU/sequencer in
        // DRAM). BitXor has no plain-ComputeDRAM primitive (multi-step De Morgan); Not needs
        // Ambit-style dual-contact cells — honest gaps, kept on the host until a witnessed tile.
        Op::Add { .. } | Op::AddAssign { .. } | Op::Sub { .. } | Op::Mul { .. } => Cp,
        Op::Div { .. } | Op::ExactDiv { .. } | Op::FloorDiv { .. } | Op::Mod { .. } => Cp,
        Op::DivPow2 { .. } | Op::MagicDivU { .. } | Op::Pow { .. } => Cp,
        Op::Lt { .. } | Op::Gt { .. } | Op::LtEq { .. } | Op::GtEq { .. } => Cp,
        Op::Eq { .. } | Op::NotEq { .. } | Op::ApproxEq { .. } | Op::Not { .. } => Cp,
        Op::Shl { .. } | Op::Shr { .. } => Cp,
        Op::Concat { .. } | Op::SeqConcat { .. } | Op::FormatValue { .. } => Cp,
        Op::SliceOp { .. } | Op::Contains { .. } | Op::RemoveFrom { .. } => Cp,
        Op::SetIndex { .. } | Op::SetIndexUnchecked { .. } | Op::RegionBoundsGuard { .. } => Cp,
        Op::Jump { .. } | Op::JumpIfFalse { .. } | Op::JumpIfTrue { .. } => Cp,
        Op::Call { .. }
        | Op::CallBuiltin { .. }
        | Op::CallValue { .. }
        | Op::MakeClosure { .. } => Cp,
        Op::Return { .. } | Op::ReturnNothing | Op::Halt => Cp,
        Op::GlobalGet { .. } | Op::GlobalSet { .. } => Cp,
        Op::EnsureOwned { .. } | Op::CheckPolicy { .. } | Op::ListPushField { .. } => Cp,
        Op::LoadToday { .. } | Op::LoadNow { .. } => Cp,
        Op::NewTuple { .. } | Op::DestructureTuple { .. } | Op::ListPop { .. } => Cp,
        Op::NewStruct { .. } | Op::StructInsert { .. } | Op::GetField { .. } => Cp,
        Op::NewInductive { .. } | Op::TestArm { .. } | Op::BindArm { .. } => Cp,
        Op::NewCrdt { .. }
        | Op::CrdtBump { .. }
        | Op::CrdtMerge { .. }
        | Op::CrdtAppend { .. }
        | Op::CrdtResolve { .. }
        | Op::CrdtJoin { .. } => Cp,
        Op::BigIntWide { .. } | Op::Obs { .. } | Op::FailWith { .. } | Op::Args { .. } => Cp,
        Op::RevSwap { .. }
        | Op::RevXor { .. }
        | Op::RevNeg { .. }
        | Op::RevAdd { .. }
        | Op::RevSub { .. } => Cp,

        // ── OffPlane: refuses the whole offload — no scheduler / host IO inside a memory device. ──
        Op::ChanNew { .. }
        | Op::ChanSend { .. }
        | Op::ChanRecv { .. }
        | Op::ChanTrySend { .. }
        | Op::ChanTryRecv { .. }
        | Op::ChanClose { .. } => Off("channel"),
        Op::Spawn { .. } | Op::SpawnHandle { .. } | Op::TaskAwait { .. } | Op::TaskAbort { .. } => {
            Off("task")
        }
        Op::Sleep { .. }
        | Op::SelectArmRecv { .. }
        | Op::SelectArmTimeout { .. }
        | Op::SelectWait { .. } => Off("virtual-time/select"),
        Op::NetConnect { .. }
        | Op::NetListen { .. }
        | Op::NetSend { .. }
        | Op::NetStream { .. }
        | Op::NetAwait { .. }
        | Op::NetMakePeer { .. }
        | Op::NetSync { .. }
        | Op::Tcp { .. }
        | Op::Serve { .. } => Off("networking"),
    }
}

/// For an OnPlane op, the ComputeDRAM primitive that realizes it (proven ≡VM in
/// [`unified_placement`](crate::codegen_sva::unified_placement)). `None` for every non-OnPlane op —
/// the op-coverage ratchet asserts this biconditional (OnPlane ⟺ `dram_prim` is `Some`).
pub fn dram_prim(op: &Op) -> Option<PrimKind> {
    match op {
        Op::BitAnd { .. } | Op::IntersectOp { .. } => Some(PrimKind::And),
        Op::BitOr { .. } | Op::UnionOp { .. } => Some(PrimKind::Or),
        Op::BitXor { .. } => Some(PrimKind::Xor),
        Op::DeepClone { .. } => Some(PrimKind::Copy),
        _ => None,
    }
}

/// The in-DRAM hardware tier an OnPlane op needs — the HONEST hardware assumption. Copy/AND/OR run on
/// UNMODIFIED off-the-shelf DRAM (ComputeDRAM, Gao et al. MICRO 2019); XOR needs the AMBIT tier
/// (dual-contact cells, a minor DRAM modification; Seshadri et al. MICRO 2017). `None` iff not OnPlane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DramTier {
    ComputeDram,
    Ambit,
}

pub fn dram_tier(op: &Op) -> Option<DramTier> {
    match op {
        Op::BitAnd { .. }
        | Op::BitOr { .. }
        | Op::IntersectOp { .. }
        | Op::UnionOp { .. }
        | Op::DeepClone { .. } => Some(DramTier::ComputeDram),
        Op::BitXor { .. } => Some(DramTier::Ambit),
        _ => None,
    }
}

/// Returns `Some(reason)` if `program` uses an op with no home in a memory device — concurrency
/// (a DRAM has no thread scheduler) or networking (host-only IO). Shares the offload classifier
/// with the other single-device tiers so they cannot drift.
pub fn unsupported_reason(program: &CompiledProgram) -> Option<&'static str> {
    use crate::aot_unsupported::Offload;
    crate::aot_unsupported::first(program).map(|o| match o {
        Offload::Channel => "channel op (a DRAM device has no channels)",
        Offload::Task => "task spawn/await (a DRAM device has no thread scheduler)",
        Offload::VirtualTime => "virtual-time / select (needs a cooperative scheduler)",
        Offload::Net => "networking op (host-only IO)",
        Offload::Dom => "DOM / browser op (the DRAM fabric has no document to build)",
    })
}

/// Whether the program carries any OnPlane bulk-bitwise/copy work — the DRAM-region existence probe.
pub fn has_bitwise_region(program: &CompiledProgram) -> bool {
    program
        .code
        .iter()
        .any(|op| matches!(fragment(op), DramFragment::OnPlane))
}

/// The fail-closed refusal gate every DRAM offload runs FIRST. `None` means the program clears the
/// gate (a bulk-bitwise/copy region exists, inside the memory-device model).
pub fn dram_refusal(program: &CompiledProgram) -> Option<DramLowerError> {
    if let Some(reason) = unsupported_reason(program) {
        return Some(DramLowerError::Unsupported(reason));
    }
    if !has_bitwise_region(program) {
        return Some(DramLowerError::NoBitwiseRegion);
    }
    None
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_dram/tests.rs"]
mod tests;
