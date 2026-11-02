//! Set algebra on the DRAM plane — the COMPLETE finite set algebra over BITSET-encoded sets, lowered
//! to DRAM dataflow kernels. A set over a bounded universe is a bit vector (a row): membership of
//! element `i` is bit `i`. Over that encoding the set algebra IS the bitwise algebra —
//!
//!   intersection ∩ = AND        union ∪ = OR              difference \ = AND-NOT
//!   symmetric difference △ = XOR complement (¬) = NOT (within the universe)
//!
//! — so each set op lowers to a [`DramDataflow`] kernel built from the DRAM basis `{And, Or, Not}`,
//! and is proven ≡ its set-theoretic reference. The VM's first-class set ops — `IntersectOp`/`UnionOp`
//! — are [`recognize`]d directly; difference / symmetric_difference / complement are provided as
//! proven kernels (the VM currently represents those via the AOT set-method path, not a VM ISA op).

use super::dataflow::DramDataflow;
use crate::codegen_sva::unified_placement::Row;
use crate::vm::Op;

/// A finite set operation over bitset-encoded sets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SetOp {
    Intersection,
    Union,
    Difference,
    SymmetricDifference,
    Complement,
}

impl SetOp {
    /// The number of set operands (Complement is unary; the rest are binary).
    pub fn arity(self) -> usize {
        if self == SetOp::Complement {
            1
        } else {
            2
        }
    }

    /// Every set operation this backend realizes in-DRAM — the coverage of the set-algebra lowering.
    pub const ALL: &'static [SetOp] = &[
        SetOp::Intersection,
        SetOp::Union,
        SetOp::Difference,
        SetOp::SymmetricDifference,
        SetOp::Complement,
    ];
}

/// Recognize a VM ISA op as a set operation, if it is one. `IntersectOp`/`UnionOp` are the VM's
/// first-class set ops; difference / symmetric_difference / complement have no VM ISA op yet (the AOT
/// tier emits them as IndexSet methods), so they are not recognized from a VM program — but their
/// kernels below are proven and ready to dispatch once those ops become first-class.
pub fn recognize(op: &Op) -> Option<SetOp> {
    match op {
        Op::IntersectOp { .. } => Some(SetOp::Intersection),
        Op::UnionOp { .. } => Some(SetOp::Union),
        _ => None,
    }
}

/// Lower a set op to a DRAM dataflow kernel over operand rows `a` (and `b` for binary ops; `b` is
/// ignored for Complement). Returns the result row index. Built entirely from the DRAM basis.
pub fn lower(op: SetOp, df: &mut DramDataflow, a: usize, b: usize) -> usize {
    match op {
        SetOp::Intersection => df.and(a, b),
        SetOp::Union => df.or(a, b),
        SetOp::Difference => df.difference(a, b), // a AND NOT b
        SetOp::SymmetricDifference => df.xor(a, b),
        SetOp::Complement => df.not(a), // within the universe = the full row width
    }
}

/// The set-theoretic reference over bitset rows — the universe is the full row width (`!0` per word).
pub fn reference(op: SetOp, a: &Row, b: &Row) -> Row {
    match op {
        SetOp::Intersection => a.iter().zip(b).map(|(x, y)| x & y).collect(),
        SetOp::Union => a.iter().zip(b).map(|(x, y)| x | y).collect(),
        SetOp::Difference => a.iter().zip(b).map(|(x, y)| x & !y).collect(),
        SetOp::SymmetricDifference => a.iter().zip(b).map(|(x, y)| x ^ y).collect(),
        SetOp::Complement => a.iter().map(|x| !x).collect(),
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_dram/set_algebra/tests.rs"]
mod tests;
