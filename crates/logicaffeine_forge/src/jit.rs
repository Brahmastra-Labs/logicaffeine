//! J1: the straight-line micro-op compiler — bytecode in, native code out.
//!
//! [`MicroOp`] is the JIT's input IR: the integer subset of the bytecode VM's
//! register operations. [`compile_straightline`] lowers each op to a fixed
//! stencil micro-sequence over the frame/operand-stack machine model and glues
//! the result into one executable chain:
//!
//! ```text
//! LoadConst{dst,v}  →  const(v); slot_set(dst)
//! Move{dst,src}     →  slot_get(src); slot_set(dst)
//! Add{dst,l,r}      →  slot_get(l); slot_get(r); addi; slot_set(dst)
//! …                    (Sub/Mul/Lt/Eq identical shape)
//! Gt{dst,l,r}       →  slot_get(r); slot_get(l); lti; slot_set(dst)   (swap)
//! Return{src}       →  slot_get(src); return
//! ```
//!
//! Anything outside this subset is the caller's tier-up bail (`None` from the
//! adapter) — the VM keeps running it.

use crate::buffer::{HoleValue, JitBuffer, JitChain};
use crate::{
    ST_ADDI, ST_BRANCH_IF, ST_CONST, ST_EQI, ST_JUMP, ST_LTI, ST_MULI, ST_RETURN, ST_SLOT_GET,
    ST_SLOT_SET, ST_SUBI,
};

/// Frame slot index (a VM register number).
pub type Slot = u16;

/// The integer straight-line subset the J1 compiler accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicroOp {
    /// `frame[dst] = value`.
    LoadConst {
        /// Destination slot.
        dst: Slot,
        /// Immediate value.
        value: i64,
    },
    /// `frame[dst] = frame[src]`.
    Move {
        /// Destination slot.
        dst: Slot,
        /// Source slot.
        src: Slot,
    },
    /// `frame[dst] = frame[lhs] + frame[rhs]` (wrapping).
    Add {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = frame[lhs] - frame[rhs]` (wrapping).
    Sub {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = frame[lhs] * frame[rhs]` (wrapping).
    Mul {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = (frame[lhs] < frame[rhs]) as i64`.
    Lt {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = (frame[lhs] > frame[rhs]) as i64`.
    Gt {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = (frame[lhs] == frame[rhs]) as i64`.
    Eq {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// Unconditional transfer to the micro-op at `target` (an index into the
    /// program; forward or backward).
    Jump {
        /// Micro-op index to transfer to.
        target: usize,
    },
    /// Transfer to `target` when frame[cond] is ZERO; fall through otherwise.
    JumpIfFalse {
        /// Condition slot (zero = jump).
        cond: Slot,
        /// Micro-op index to transfer to.
        target: usize,
    },
    /// Terminate the chain, returning frame[src].
    Return {
        /// Slot whose value is returned.
        src: Slot,
    },
}

/// Compile errors — structural, found before any code is emitted.
#[derive(Debug, PartialEq, Eq)]
pub enum JitCompileError {
    /// The program is empty.
    Empty,
    /// Execution can run off the end: the final op must be Return or Jump.
    FallsOffTheEnd,
    /// A jump target is outside the program.
    BadJumpTarget {
        /// Index of the offending op.
        op_index: usize,
        /// The out-of-range target.
        target: usize,
    },
    /// Assembly failed (missing hole/patch/map errors).
    Assembly(String),
}

impl std::fmt::Display for JitCompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JitCompileError::Empty => write!(f, "jit: empty program"),
            JitCompileError::FallsOffTheEnd => {
                write!(f, "jit: the final op must be Return or Jump")
            }
            JitCompileError::BadJumpTarget { op_index, target } => {
                write!(f, "jit: op {op_index} jumps to {target}, outside the program")
            }
            JitCompileError::Assembly(e) => write!(f, "jit: assembly failed: {e}"),
        }
    }
}

impl std::error::Error for JitCompileError {}

/// Lower a straight-line micro-op program into one executable stencil chain.
///
/// The returned chain is run with [`JitChain::run_with_frame`]; the frame's
/// slots are the program's registers (inputs pre-loaded by the caller,
/// outputs visible after the run).
pub fn compile_straightline(ops: &[MicroOp]) -> Result<JitChain, JitCompileError> {
    if ops.is_empty() {
        return Err(JitCompileError::Empty);
    }
    if !matches!(ops.last(), Some(MicroOp::Return { .. }) | Some(MicroOp::Jump { .. })) {
        return Err(JitCompileError::FallsOffTheEnd);
    }
    for (i, op) in ops.iter().enumerate() {
        match *op {
            MicroOp::Jump { target } | MicroOp::JumpIfFalse { target, .. } => {
                if target >= ops.len() {
                    return Err(JitCompileError::BadJumpTarget { op_index: i, target });
                }
            }
            _ => {}
        }
    }

    /// Pieces each micro-op expands to (must mirror the emission below).
    fn piece_count(op: &MicroOp) -> usize {
        match op {
            MicroOp::LoadConst { .. } | MicroOp::Move { .. } | MicroOp::Return { .. } => 2,
            MicroOp::Jump { .. } => 1,
            MicroOp::JumpIfFalse { .. } => 2,
            _ => 4,
        }
    }

    // Pass 1: the first-piece index of every micro-op (jump label targets).
    let mut op_piece: Vec<usize> = Vec::with_capacity(ops.len());
    let mut total = 0usize;
    for op in ops {
        op_piece.push(total);
        total += piece_count(op);
    }

    // Pass 2: emit. Sequential continuations point at "the next piece";
    // jump continuations at the target op's first piece.
    fn push_seq(
        buf: &mut JitBuffer,
        count: &mut usize,
        stencil: &'static crate::Stencil,
        konst: Option<i64>,
    ) {
        *count += 1;
        let next = buf.label(*count);
        let mut holes = vec![HoleValue::Cont(0, next)];
        if let Some(v) = konst {
            holes.push(HoleValue::Const(0, v));
        }
        buf.push_stencil(stencil, &holes);
    }
    let mut buf = JitBuffer::new();
    let mut count = 0usize;

    for op in ops {
        match *op {
            MicroOp::LoadConst { dst, value } => {
                push_seq(&mut buf, &mut count, &ST_CONST, Some(value));
                push_seq(&mut buf, &mut count, &ST_SLOT_SET, Some(dst as i64));
            }
            MicroOp::Move { dst, src } => {
                push_seq(&mut buf, &mut count, &ST_SLOT_GET, Some(src as i64));
                push_seq(&mut buf, &mut count, &ST_SLOT_SET, Some(dst as i64));
            }
            MicroOp::Add { dst, lhs, rhs }
            | MicroOp::Sub { dst, lhs, rhs }
            | MicroOp::Mul { dst, lhs, rhs }
            | MicroOp::Lt { dst, lhs, rhs }
            | MicroOp::Eq { dst, lhs, rhs } => {
                let stencil = match op {
                    MicroOp::Add { .. } => &ST_ADDI,
                    MicroOp::Sub { .. } => &ST_SUBI,
                    MicroOp::Mul { .. } => &ST_MULI,
                    MicroOp::Lt { .. } => &ST_LTI,
                    MicroOp::Eq { .. } => &ST_EQI,
                    _ => unreachable!(),
                };
                push_seq(&mut buf, &mut count, &ST_SLOT_GET, Some(lhs as i64));
                push_seq(&mut buf, &mut count, &ST_SLOT_GET, Some(rhs as i64));
                push_seq(&mut buf, &mut count, stencil, None);
                push_seq(&mut buf, &mut count, &ST_SLOT_SET, Some(dst as i64));
            }
            MicroOp::Gt { dst, lhs, rhs } => {
                // a > b  ⇔  b < a: swap the operand pushes, reuse lti.
                push_seq(&mut buf, &mut count, &ST_SLOT_GET, Some(rhs as i64));
                push_seq(&mut buf, &mut count, &ST_SLOT_GET, Some(lhs as i64));
                push_seq(&mut buf, &mut count, &ST_LTI, None);
                push_seq(&mut buf, &mut count, &ST_SLOT_SET, Some(dst as i64));
            }
            MicroOp::Jump { target } => {
                count += 1;
                let t = buf.label(op_piece[target]);
                buf.push_stencil(&ST_JUMP, &[HoleValue::Cont(0, t)]);
            }
            MicroOp::JumpIfFalse { cond, target } => {
                push_seq(&mut buf, &mut count, &ST_SLOT_GET, Some(cond as i64));
                count += 1;
                let fallthrough = buf.label(count);
                let t = buf.label(op_piece[target]);
                // branch_if: nonzero → cont 0 (fall through), zero → cont 1.
                buf.push_stencil(
                    &ST_BRANCH_IF,
                    &[HoleValue::Cont(0, fallthrough), HoleValue::Cont(1, t)],
                );
            }
            MicroOp::Return { src } => {
                push_seq(&mut buf, &mut count, &ST_SLOT_GET, Some(src as i64));
                count += 1;
                buf.push_stencil(&ST_RETURN, &[]);
            }
        }
    }
    debug_assert_eq!(count, total, "piece_count out of sync with emission");

    buf.finish().map_err(|e| JitCompileError::Assembly(e.to_string()))
}

/// The reference evaluator — the independent model every differential runs
/// against. Deliberately the dumbest possible implementation: a pc loop.
/// `fuel` bounds the step count (None when caught in a loop that long).
pub fn reference_eval(ops: &[MicroOp], frame: &mut [i64], mut fuel: u64) -> Option<i64> {
    let mut pc = 0usize;
    while pc < ops.len() {
        if fuel == 0 {
            return None;
        }
        fuel -= 1;
        match ops[pc] {
            MicroOp::LoadConst { dst, value } => frame[dst as usize] = value,
            MicroOp::Move { dst, src } => frame[dst as usize] = frame[src as usize],
            MicroOp::Add { dst, lhs, rhs } => {
                frame[dst as usize] = frame[lhs as usize].wrapping_add(frame[rhs as usize])
            }
            MicroOp::Sub { dst, lhs, rhs } => {
                frame[dst as usize] = frame[lhs as usize].wrapping_sub(frame[rhs as usize])
            }
            MicroOp::Mul { dst, lhs, rhs } => {
                frame[dst as usize] = frame[lhs as usize].wrapping_mul(frame[rhs as usize])
            }
            MicroOp::Lt { dst, lhs, rhs } => {
                frame[dst as usize] = (frame[lhs as usize] < frame[rhs as usize]) as i64
            }
            MicroOp::Gt { dst, lhs, rhs } => {
                frame[dst as usize] = (frame[lhs as usize] > frame[rhs as usize]) as i64
            }
            MicroOp::Eq { dst, lhs, rhs } => {
                frame[dst as usize] = (frame[lhs as usize] == frame[rhs as usize]) as i64
            }
            MicroOp::Jump { target } => {
                pc = target;
                continue;
            }
            MicroOp::JumpIfFalse { cond, target } => {
                if frame[cond as usize] == 0 {
                    pc = target;
                    continue;
                }
            }
            MicroOp::Return { src } => return Some(frame[src as usize]),
        }
        pc += 1;
    }
    unreachable!("validated programs cannot fall off the end")
}
