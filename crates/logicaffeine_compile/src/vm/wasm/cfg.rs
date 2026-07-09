//! Shared control-flow lowering: VM bytecode uses *absolute jumps*; WebAssembly has only
//! *structured* control flow. Both the JIT region emitter ([`func`](crate::vm::wasm::func)) and
//! the AOT whole-program emitter (`super::module`) lower a region the same way — split it into
//! basic blocks and emit the standard **dispatch loop**: a `loop` wrapping one `block` per
//! basic block, with a `br_table` on a "next block" local. Each block runs its straight-line
//! code, then sets the next-block index and re-dispatches (`br` to the loop) — or returns.
//! This translates *any* reducible (and irreducible) control flow, not just recognized shapes.
//!
//! This module owns the parts that are pure control-flow structure (leaders, block bounds,
//! the dispatch-loop assembly) and so are identical across both consumers; the per-op body
//! lowering — which differs (the JIT's integer fragment vs. the AOT's whole-program set) —
//! lives in each consumer.

use std::collections::BTreeSet;

use super::encode::{leb_u32, local_get, VOID_BLOCKTYPE};
use crate::vm::instruction::Op;

/// The basic-block partition of a region of `n` ops with region-local (0-based) jump targets:
/// the sorted leader pcs. Maps any jump target or fallthrough to its dispatch-block index.
pub(crate) struct Blocks {
    leaders: Vec<usize>,
    n: usize,
}

impl Blocks {
    /// Compute the basic-block leaders for `ops`. A leader is pc 0, every branch target, and
    /// every instruction following a branch or terminator. Returns `None` if any branch target
    /// escapes `[0, n)` — a region that is not self-contained.
    ///
    /// This accounts for *all* of the language's control-flow ops, so the partition is valid
    /// regardless of which op set the caller goes on to accept; a caller that does not support
    /// a given op rejects it in its own body lowering, not here.
    pub(crate) fn new(ops: &[Op]) -> Option<Blocks> {
        let n = ops.len();
        let mut leaders: BTreeSet<usize> = BTreeSet::new();
        leaders.insert(0);
        for (i, op) in ops.iter().enumerate() {
            match *op {
                Op::Jump { target }
                | Op::JumpIfFalse { target, .. }
                | Op::JumpIfTrue { target, .. } => {
                    if target >= n {
                        return None;
                    }
                    leaders.insert(target);
                    if i + 1 < n {
                        leaders.insert(i + 1);
                    }
                }
                // `IterNext` is a conditional branch: it falls through with the next element, or
                // jumps to `exit` (the matching `IterPop`) when the snapshot is exhausted. Both
                // its `exit` target and its fallthrough start new basic blocks.
                Op::IterNext { exit, .. } => {
                    if exit >= n {
                        return None;
                    }
                    leaders.insert(exit);
                    if i + 1 < n {
                        leaders.insert(i + 1);
                    }
                }
                Op::Return { .. } | Op::ReturnNothing | Op::Halt | Op::FailWith { .. } => {
                    if i + 1 < n {
                        leaders.insert(i + 1);
                    }
                }
                _ => {}
            }
        }
        Some(Blocks { leaders: leaders.into_iter().collect(), n })
    }

    pub(crate) fn num_blocks(&self) -> usize {
        self.leaders.len()
    }

    /// Whether `pc` begins a basic block (a branch target or post-branch instruction) — the points
    /// where a flow-sensitive analysis must conservatively join incoming states.
    pub(crate) fn is_leader(&self, pc: usize) -> bool {
        self.leaders.binary_search(&pc).is_ok()
    }

    /// The dispatch-block index of pc `pc` (which is always a leader by construction — every
    /// jump target and fallthrough is one).
    pub(crate) fn block_of(&self, pc: usize) -> usize {
        self.leaders.binary_search(&pc).expect("pc is a leader by construction")
    }

    /// The first pc of block `k`.
    pub(crate) fn start(&self, k: usize) -> usize {
        self.leaders[k]
    }

    /// The pc one past the last of block `k` (the next leader, or `n` for the last block).
    pub(crate) fn end(&self, k: usize) -> usize {
        if k + 1 < self.leaders.len() {
            self.leaders[k + 1]
        } else {
            self.n
        }
    }

    /// The `br` depth from block `k`'s code to the enclosing `$loop` (re-dispatch target).
    pub(crate) fn br_loop(&self, k: usize) -> u32 {
        (self.num_blocks() - 1 - k) as u32
    }
}

/// Assemble the dispatch-loop body from each block's already-lowered code, returning the bytes
/// from `block $exit` through its matching `end` (the caller appends the function epilogue —
/// `unreachable; end` for an i64-returning function, or just `end` for a void one). `pc_local`
/// is the i32 "next block" local; `blocks_code[k]` is block `k`'s body + terminator.
pub(crate) fn assemble_dispatch_loop(pc_local: u32, blocks_code: &[Vec<u8>]) -> Vec<u8> {
    let num_blocks = blocks_code.len();
    let mut body = Vec::new();
    body.push(0x02); // block $exit
    body.push(VOID_BLOCKTYPE);
    body.push(0x03); // loop $loop
    body.push(VOID_BLOCKTYPE);
    for _ in 0..num_blocks {
        body.push(0x02); // block $b_k
        body.push(VOID_BLOCKTYPE);
    }
    // br_table on the next-block index: target[k] = depth k (k-th dispatch block), default = $exit.
    local_get(&mut body, pc_local);
    body.push(0x0E); // br_table
    leb_u32(&mut body, num_blocks as u32); // target count
    for k in 0..num_blocks {
        leb_u32(&mut body, k as u32);
    }
    leb_u32(&mut body, (num_blocks + 1) as u32); // default → $exit (one past $loop)
    // Close each dispatch block, emitting its code after the corresponding `end`.
    for code in blocks_code {
        body.push(0x0B); // end (closes the innermost remaining dispatch block)
        body.extend_from_slice(code);
    }
    body.push(0x0B); // end $loop
    body.push(0x0B); // end $exit
    body
}
