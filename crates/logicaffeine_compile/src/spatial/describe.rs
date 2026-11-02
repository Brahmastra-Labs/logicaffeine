//! `describe` — what a lowered program actually DOES, as facts derived from the IR.
//!
//! ★ WHY THIS EXISTS: A KERNEL MUST NOT BE ABLE TO ASSERT ITS OWN CORRECTNESS IN PROSE.
//!
//! The tests this replaces asked the emitted C++ whether it did the right thing, by substring. That is
//! a question generated text can always answer favourably. The concrete case:
//! `fused_row_family_lock` asserted `contains("per-row rmsnorm (weighted)")` and passed for as long as
//! the retired template PRINTED THAT PHRASE IN A COMMENT — while its body declared `cb_w` and never
//! read it. The kernel claimed to apply a weight, in English, and did not apply one.
//!
//! Swapping the needle for an instruction name (`contains("mul_tiles_bcast_rows")`) does not fix that;
//! it moves the same hole one level down, because a comment can contain an instruction name just as
//! easily as a sentence. The only durable answer is to stop asking the TEXT.
//!
//! So these facts are computed from the [`SpatialProgram`] — from iterator kinds and access maps, the
//! things the compiler actually decided — and never from a string. A reduction is counted because an
//! axis is marked [`IterKind::Reduce`]. A per-column weight is counted because an operand's access map
//! OMITS the row axis, which is what makes it constant down the rows. Nothing a printer emits, and no
//! comment anyone writes, can change either number.

use std::collections::BTreeSet;

use super::ir::{ComputeOp, IterKind, SfpuKind, SpatialProgram, StageKind, TensorBody};

/// What a program's compute stages do, in the vocabulary of the algebra rather than of a backend.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComputeFacts {
    /// Compute stages in the program.
    pub stages: usize,
    /// Stages that fold an axis away — the row reductions.
    pub reductions: usize,
    /// Stages applying a value that is constant DOWN THE ROWS: a per-COLUMN vector, which is what a
    /// normalization's weight and bias are. The access map omits the row axis.
    pub per_column_operands: usize,
    /// Stages applying a value that is constant ACROSS A ROW: a per-ROW scalar, which is what a
    /// reduction's result is when it is broadcast back. The access map omits the column axis.
    pub per_row_operands: usize,
    /// Every SFPU transform the program applies, anywhere in a body (including nested inside
    /// arithmetic, which is where fusion puts them).
    pub sfpu: BTreeSet<SfpuKind>,
}

impl ComputeFacts {
    /// Whether the program applies a per-column vector — a weight or a bias.
    pub fn applies_a_per_column_operand(&self) -> bool {
        self.per_column_operands > 0
    }
    /// Whether the program broadcasts a per-row scalar back across its row — a rescale.
    pub fn rescales_by_a_per_row_scalar(&self) -> bool {
        self.per_row_operands > 0
    }
    /// Whether the program applies a given SFPU transform anywhere.
    pub fn uses(&self, kind: SfpuKind) -> bool {
        self.sfpu.contains(&kind)
    }
}

fn collect_sfpu(b: &TensorBody, acc: &mut BTreeSet<SfpuKind>) {
    match b {
        TensorBody::Unary { kind, arg } => {
            acc.insert(*kind);
            collect_sfpu(arg, acc);
        }
        TensorBody::Add(x, y)
        | TensorBody::Sub(x, y)
        | TensorBody::Mul(x, y)
        | TensorBody::Min(x, y)
        | TensorBody::Max(x, y)
        | TensorBody::FloorDiv(x, y) => {
            collect_sfpu(x, acc);
            collect_sfpu(y, acc);
        }
        TensorBody::Shr { arg, .. } => collect_sfpu(arg, acc),
        TensorBody::Select {
            then, otherwise, ..
        } => {
            collect_sfpu(then, acc);
            collect_sfpu(otherwise, acc);
        }
        TensorBody::Operand(_) | TensorBody::Const(_) => {}
    }
}

/// The facts of a program, computed from its stages. Cheap and total — it never fails, because every
/// question it answers is structural.
pub fn compute_facts(p: &SpatialProgram) -> ComputeFacts {
    let mut f = ComputeFacts::default();
    for s in &p.stages {
        let StageKind::Compute(op) = &s.kind else {
            continue;
        };
        f.stages += 1;
        let ComputeOp::Generic {
            iters, ins, body, ..
        } = op
        else {
            continue;
        };
        let (row_ax, col_ax) = (iters.first(), iters.get(1));
        if iters
            .iter()
            .any(|a| matches!(a.kind, IterKind::Reduce { .. }))
        {
            f.reductions += 1;
        }
        collect_sfpu(body, &mut f.sfpu);
        // An operand CONSTANT along an axis is broadcast along it. The scaler tile a reduction takes is
        // constant along BOTH axes and is not a broadcast of data, so it is excluded from both counts.
        for m in ins {
            let varies_row = row_ax.map(|a| m.varies_along(a.var)).unwrap_or(true);
            let varies_col = col_ax.map(|a| m.varies_along(a.var)).unwrap_or(true);
            match (varies_row, varies_col) {
                (false, true) => f.per_column_operands += 1,
                (true, false) => f.per_row_operands += 1,
                _ => {}
            }
        }
    }
    f
}
