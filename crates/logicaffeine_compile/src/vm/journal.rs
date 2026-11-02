//! The history tape — the Bennett/Landauer "garbage" of reverse execution.
//!
//! A forward run of the VM produces `(final state, tape)`; [`Vm::run_reverse`](crate::vm::Vm) walks
//! the instruction stream backward and consumes the tape to reconstruct the EXACT initial state.
//! Each information-losing op appends the minimal destroyed bits (an undo-log delta, NOT a full
//! snapshot); a [`RevClass::Bijection`](crate::vm::reverse::RevClass) op appends NOTHING — it
//! self-inverts algebraically. So the tape's size is the erased-bit (Landauer energy) cost, and it
//! is ZERO for an all-abelian run — the zero-erasure ideal the adiabatic chip is built on.
//! Bennett uncompute (R4) shrinks it further toward that floor.

use crate::vm::value::Value;

/// One recorded delta on the [`HistoryTape`]. Records only what a forward op DESTROYED, so reverse
/// execution can restore it. Zero-cost/control entries (a shown line, a branch direction) carry no
/// erased data — they are re-derivable — and so contribute nothing to the erased-bit measure.
#[derive(Debug)]
pub enum JournalEntry {
    /// A register slot (`base + r`) was overwritten; restore `old` on reverse.
    RegOverwrite { slot: usize, old: Value },
    /// A promoted global was overwritten (or first-defined, `old == None`); restore it on reverse.
    GlobalOverwrite { idx: usize, old: Option<Value> },
    /// Several register slots were overwritten at once (a tuple destructure). Reverse restores each.
    MultiRegOverwrite { slots: Vec<(usize, Value)> },
    /// A `Show` appended one output line; reverse truncates the output buffer. The content is
    /// re-derivable on a re-forward, so no data is erased.
    ShowLine,
    /// A NONDETERMINISTIC read (clock / random) drew `drawn` into a register. Reading does not mutate
    /// the world, so reverse simply restores the register's prior value (`dst_old`). Critically,
    /// `drawn` is RECORDED — the entropy is captured on the tape (a deterministic replay could feed it
    /// back), rather than silently discarded, which is what makes reversing a nondeterministic program
    /// honest rather than a lie.
    NondetRead {
        dst_slot: usize,
        dst_old: Value,
        drawn: Value,
    },
    /// A conditional branch was taken (`true`) or fell through (`false`); reverse follows it back.
    /// One bit of control flow, not erased data.
    Branch { taken: bool },
    /// An in-place collection mutation (push / index-set / add / remove). `old` is the register's
    /// pre-mutation value — an `Rc` bump for a shared List/Map/Set, or a deep copy for a by-value
    /// container. Because that bump forces the op's copy-on-write, the original stays pristine, so
    /// reverse restores `old` exactly (bit-identical, including aliasing). If instead the register
    /// still holds the SAME `Rc` (an in-place mutation — only when value-semantics is off), reverse
    /// declines rather than mis-restore.
    CollectionMutate { slot: usize, old: Value },
    /// A `ListPop`: it both shrinks a list AND overwrites a destination register. Reverse restores
    /// the pristine pre-pop list (`list_old`, which — COW-forced — still holds the popped element)
    /// and the destination's prior value (`dst_old`), so no push-back or empty-flag is needed.
    ListPop {
        list_slot: usize,
        list_old: Value,
        dst_slot: usize,
        dst_old: Value,
    },
    /// An `IterNext`. `advanced` = it yielded an element (idx < len): reverse restores the
    /// destination's prior value and rewinds the top iterator's index by one. `!advanced` = it was
    /// exhausted (a pure jump to the loop exit): reverse does nothing.
    IterNextUndo {
        advanced: bool,
        dst_slot: usize,
        dst_old: Value,
    },
    /// An `IterPop` dropped the top iterator (a snapshot + cursor). Reverse re-pushes it so the loop
    /// body's `IterNext`s can be rewound.
    IterPopUndo { items: Vec<Value>, idx: usize },
    /// A CRDT interior mutation (`CrdtAppend`/`CrdtResolve` on an `Rc<RefCell<CrdtValue>>`). Unlike a
    /// by-value container, the mutation happens IN PLACE through the shared handle, so an `Rc`-bump
    /// snapshot would share the mutated interior, not preserve it. Instead `snapshot` is a DEEP clone
    /// of the pre-mutation `CrdtValue` (wrapped in a fresh handle); reverse copies it back THROUGH the
    /// register's live handle (`*rc.borrow_mut() = …`), so every alias observes the restore.
    CrdtInterior { slot: usize, snapshot: Value },
    /// A `CrdtResolve` onto a nested CRDT-register FIELD (`registers[obj_slot]`'s `field`) — the
    /// field's `Rc<RefCell<CrdtValue>>` interior is resolved in place. Like
    /// [`JournalEntry::CrdtInterior`] but the handle lives inside a struct field, so `snapshot` (a
    /// deep pre-resolve clone) is copied back through that field's live handle. (A resolve onto a
    /// plain field is a `CollectionMutate` instead.)
    CrdtFieldInterior {
        obj_slot: usize,
        field: String,
        snapshot: Value,
    },
    /// An IN-PLACE integer division `a = a / b` — the quotient is kept in `dst_slot`, the REMAINDER is
    /// the only bit truly discarded. Reverse reconstructs `a_old = q·b + r` from the kept quotient and
    /// the divisor, so the Landauer cost is the remainder's few bits (`< log₂ b`), not the whole
    /// dividend word. The divisor is a recomputable input (stored here for a self-contained reverse,
    /// but it is not erased information, so it costs nothing).
    DivReconstruct {
        dst_slot: usize,
        divisor: Value,
        remainder: Value,
    },
    /// An IN-PLACE bit shift (`a = a << k` / `a = a >> k`; `Shl`/`Shr`/`DivPow2` with `dst == lhs`).
    /// Only the shifted-OUT bits are destroyed — the rest of `a_old` survives in the kept result — so
    /// the tape records just those `k` bits (`dropped`: the low `k` bits for a right shift, the top `k`
    /// bits right-aligned for a left shift). Reverse recombines them with the kept result, so the
    /// Landauer cost is `k` bits, not the whole 64-bit word.
    ShiftReconstruct {
        dst_slot: usize,
        dropped: i64,
        k: u32,
        left: bool,
    },
    /// An IN-PLACE bitwise mask (`a = a & b` / `a = a | b`; `BitAnd`/`BitOr` with `dst == lhs`). The
    /// bits the live mask `b` cannot recover are `saved`: for AND the bits where `b = 0`, for OR the
    /// bits where `b = 1`. Reverse re-merges them under the complementary mask. The Landauer cost is
    /// exactly the count of unrecoverable positions (`64 − popcount b` for AND, `popcount b` for OR).
    MaskReconstruct {
        dst_slot: usize,
        saved: i64,
        mask: i64,
        and: bool,
    },
    /// An IN-PLACE integer modulo (`a = a % b`; `Mod` with `dst == lhs`). The remainder is kept in the
    /// destination; the QUOTIENT is the discarded information. Reverse rebuilds `a_old = q·b + r` from
    /// the saved quotient and the live divisor. The Landauer cost is the quotient's magnitude bits.
    ModReconstruct {
        dst_slot: usize,
        quotient: i64,
        divisor: Value,
    },
    /// A `Call` grew the register file (a callee window). Reverse truncates it back to `restore_len`
    /// after the callee's body has been reversed.
    CallUndo { restore_len: usize },
    /// A `CallValue` (closure invocation): like `Call` it grew the register file, and it also bound
    /// captures into the callee frame. `captures` are the pre-binding `(slot, old_value)` pairs (an
    /// overlap capture slot survives the truncate, so its prior caller value must be restored).
    /// Reverse restores the capture slots, then truncates back to `restore_len`.
    CallValueUndo {
        restore_len: usize,
        captures: Vec<(usize, Value)>,
    },
    /// A `Return`/`ReturnNothing` tore down the callee frame. Reverse rebuilds it: regrow the
    /// registers to `frame_from + frame_vals.len()` restoring the callee's saved window, restore the
    /// iterator stack, and undo the return-value write (`rv_slot`←`rv_old`) and the argument-window
    /// clear (`arg_lo`..←`arg_old`) — leaving the machine exactly in the callee's pre-return state so
    /// the callee body can be walked backward.
    ReturnUndo {
        frame_from: usize,
        frame_vals: Vec<Value>,
        iter_from: usize,
        iter_vals: Vec<(Vec<Value>, usize)>,
        rv_slot: usize,
        rv_old: Value,
        arg_lo: usize,
        arg_old: Vec<Value>,
    },
}

impl JournalEntry {
    /// The erased-DATA cost of this entry in bytes — the Landauer energy proxy. Control/derivable
    /// entries cost 0; a value-overwrite costs the machine word it displaced.
    fn erased_bytes(&self) -> usize {
        use std::mem::size_of;
        match self {
            JournalEntry::RegOverwrite { .. }
            | JournalEntry::CollectionMutate { .. }
            | JournalEntry::CrdtInterior { .. }
            | JournalEntry::CrdtFieldInterior { .. } => size_of::<Value>(),
            // Divisor + remainder are held for a self-contained reverse (peak-tape cost).
            JournalEntry::DivReconstruct { .. } => 2 * size_of::<Value>(),
            // A shift/mask entry holds one saved machine word; a modulo entry holds quotient + divisor.
            JournalEntry::ShiftReconstruct { .. } | JournalEntry::MaskReconstruct { .. } => {
                size_of::<Value>()
            }
            JournalEntry::ModReconstruct { .. } => 2 * size_of::<Value>(),
            JournalEntry::ListPop { .. } => 2 * size_of::<Value>(),
            JournalEntry::IterNextUndo { advanced, .. } => {
                usize::from(*advanced) * size_of::<Value>()
            }
            JournalEntry::IterPopUndo { items, .. } => items.len() * size_of::<Value>(),
            JournalEntry::CallUndo { .. } => 0,
            JournalEntry::CallValueUndo { captures, .. } => captures.len() * size_of::<Value>(),
            JournalEntry::ReturnUndo {
                frame_vals,
                arg_old,
                ..
            } => (frame_vals.len() + arg_old.len() + 1) * size_of::<Value>(),
            JournalEntry::GlobalOverwrite { old, .. } => {
                old.as_ref().map_or(0, |_| size_of::<Value>())
            }
            JournalEntry::MultiRegOverwrite { slots } => slots.len() * size_of::<Value>(),
            // Only the overwritten register is erased data; `drawn` is captured INPUT entropy.
            JournalEntry::NondetRead { .. } => size_of::<Value>(),
            JournalEntry::ShowLine | JournalEntry::Branch { .. } => 0,
        }
    }

    /// The GENUINELY-irreversible information this entry records — the true Landauer cost. Refines
    /// [`JournalEntry::erased_bytes`]: a write that overwrote the DEFAULT (`Nothing`) destroyed no
    /// information (the pristine initial state is known, so reverse re-derives it — the same reason
    /// `GlobalOverwrite { old: None }` is free), and so costs 0. What remains is the information the
    /// program truly erased — an overwrite of a live value, a lattice merge, a captured entropy read.
    fn landauer_bytes(&self) -> usize {
        use std::mem::size_of;
        let w = size_of::<Value>();
        let cost = |v: &Value| if v.is_nothing() { 0 } else { w };
        match self {
            JournalEntry::RegOverwrite { old, .. } | JournalEntry::CollectionMutate { old, .. } => {
                cost(old)
            }
            // A CRDT interior snapshot records a join-semilattice state the merge/resolve destroyed —
            // genuinely irreducible information.
            JournalEntry::CrdtInterior { .. } | JournalEntry::CrdtFieldInterior { .. } => w,
            // Only the remainder is erased (the divisor is a recomputable input, kept quotient aside).
            // A remainder is `< |b|`, so it fits in `⌈log₂|b|⌉` bits — the true Landauer cost.
            JournalEntry::DivReconstruct { divisor, .. } => {
                let d = divisor.as_int().unwrap_or(0).unsigned_abs();
                let bits = if d <= 1 {
                    0
                } else {
                    (u64::BITS - (d - 1).leading_zeros()) as usize
                };
                bits.div_ceil(8)
            }
            // A shift destroys exactly the `k` shifted-out bits — the reconstruction recovers the rest
            // from the kept result, so that is the whole erased-information cost.
            JournalEntry::ShiftReconstruct { k, .. } => (*k as usize).div_ceil(8),
            // A mask destroys the positions the live mask can't recover — `64 − popcount` for AND (the
            // cleared bits), `popcount` for OR (the forced bits).
            JournalEntry::MaskReconstruct { mask, and, .. } => {
                let m = *mask as u64;
                let bits = if *and {
                    (u64::BITS - m.count_ones()) as usize
                } else {
                    m.count_ones() as usize
                };
                bits.div_ceil(8)
            }
            // Only the quotient is erased (the divisor is a live recomputable input; the remainder is
            // kept in the destination). It fits in `⌈log₂|q|⌉` bits — the true Landauer cost.
            JournalEntry::ModReconstruct { quotient, .. } => {
                let q = quotient.unsigned_abs();
                let bits = if q == 0 {
                    0
                } else {
                    (u64::BITS - q.leading_zeros()) as usize
                };
                bits.div_ceil(8)
            }
            JournalEntry::ListPop {
                list_old, dst_old, ..
            } => cost(list_old) + cost(dst_old),
            JournalEntry::IterNextUndo {
                advanced, dst_old, ..
            } => {
                if *advanced {
                    cost(dst_old)
                } else {
                    0
                }
            }
            JournalEntry::IterPopUndo { items, .. } => items.iter().map(cost).sum(),
            JournalEntry::CallUndo { .. } => 0,
            JournalEntry::CallValueUndo { captures, .. } => {
                captures.iter().map(|(_, v)| cost(v)).sum()
            }
            JournalEntry::ReturnUndo {
                frame_vals,
                arg_old,
                rv_old,
                ..
            } => {
                frame_vals.iter().map(cost).sum::<usize>()
                    + arg_old.iter().map(cost).sum::<usize>()
                    + cost(rv_old)
            }
            JournalEntry::GlobalOverwrite { old, .. } => old.as_ref().map_or(0, cost),
            JournalEntry::MultiRegOverwrite { slots } => slots.iter().map(|(_, v)| cost(v)).sum(),
            // The overwritten dst is the erased part; `drawn` is captured INPUT entropy, not erased.
            JournalEntry::NondetRead { dst_old, .. } => cost(dst_old),
            JournalEntry::ShowLine | JournalEntry::Branch { .. } => 0,
        }
    }
}

/// The append-forward / pop-backward garbage tape of a recording forward run. TWO stacks:
///
///   • `entries` — the erased-DATA deltas (register/global overwrites, output lines). These are the
///     Landauer garbage; [`HistoryTape::erased_bytes`]/[`HistoryTape::len`] count them, and they are
///     EMPTY for an all-abelian run.
///   • `steps` — the executed program-counter trace (one entry per executed op), the control
///     skeleton. Reverse execution walks it backward to replay the exact executed path — branches
///     and loops fall out for free (a loop is just the trace with repeated pcs). This is control
///     overhead (re-derivable by re-running forward, so zero erased DATA and Bennett-uncomputable),
///     NOT part of the erased-bit count.
#[derive(Debug, Default)]
pub struct HistoryTape {
    entries: Vec<JournalEntry>,
    /// The executed `(pc, base)` trace. `base` is the register-window base the op ran in — carried
    /// so reverse execution restores the correct frame when walking back through a function call.
    steps: Vec<(usize, usize)>,
}

impl HistoryTape {
    /// A fresh, empty tape.
    pub fn new() -> Self {
        HistoryTape {
            entries: Vec::new(),
            steps: Vec::new(),
        }
    }

    /// Append an erased-data delta (forward execution).
    pub fn push(&mut self, entry: JournalEntry) {
        self.entries.push(entry);
    }

    /// Pop the most recent erased-data delta (reverse execution).
    pub fn pop(&mut self) -> Option<JournalEntry> {
        self.entries.pop()
    }

    /// Record that the op at `pc` (running in window `base`) executed (forward) — the control trace.
    pub fn push_step(&mut self, pc: usize, base: usize) {
        self.steps.push((pc, base));
    }

    /// Pop the most recently executed `(pc, base)` (reverse execution walks the path backward).
    pub fn pop_step(&mut self) -> Option<(usize, usize)> {
        self.steps.pop()
    }

    /// The number of recorded erased-data deltas.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the DATA tape is empty. `true` after an all-abelian (bijection-only) run — the
    /// zero-erasure signature (a program may still have a non-empty control-flow `steps` trace).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The number of executed ops on the control-flow trace.
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// The number of recorded nondeterministic reads (clock / random) — the captured entropy count.
    pub fn nondet_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e, JournalEntry::NondetRead { .. }))
            .count()
    }

    /// Total erased DATA on the tape, in bytes — the Landauer energy proxy. Exactly `0` for an
    /// all-abelian run (every op self-inverts, nothing is journaled), regardless of how many times a
    /// reversible loop body ran.
    pub fn erased_bytes(&self) -> usize {
        self.entries.iter().map(JournalEntry::erased_bytes).sum()
    }

    /// The GENUINELY-irreversible information on the tape — the true Landauer (energy) number. Below
    /// [`HistoryTape::erased_bytes`] because a write that overwrote the DEFAULT (`Nothing`) destroyed
    /// no information: pebbling elides the recomputable scratch, and this excludes the zero-information
    /// first-writes, so a pure computation into an output measures **0**.
    pub fn landauer_bytes(&self) -> usize {
        self.entries.iter().map(JournalEntry::landauer_bytes).sum()
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/journal/tests.rs"]
mod tests;
