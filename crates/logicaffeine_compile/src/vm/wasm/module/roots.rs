//! Function-local roots for collection inside a batch program.
//!
//! A shadow local owns the value last pinned for that register. Before a call or loop safepoint,
//! shadows are synchronized with the registers that can still be read. Suspended callers keep
//! their shadows until they resume. Ordinary register reads remain borrows.
//!
//! A synchronization is code only where a shadow may not hold what it is about to store: what each
//! shadow holds is known before every op, on every path that reaches it, so a shadow already holding
//! its register's value, or already null for a register no longer read, costs nothing at that point.
use super::*;

pub(super) struct Frame {
    registers: Vec<u16>,
    words: usize,
    live: Vec<u64>,
    base: u32,
    /// Per op, per register: what its shadow holds before the op runs; `None` for an op no path
    /// reaches.
    held: Vec<Option<Vec<Shadow>>>,
    /// Per op, per register: whether the op touches the register — writes it, or reads it, as a write
    /// in place may store a fresh handle in it (a copy-on-write `Push`, `SetIndex` or field insert reports
    /// its collection as read).
    touched: Vec<u64>,
    /// Per op: whether the shadows are synchronized before it — a call that can reach a safepoint, or a
    /// loop's back edge, which is one.
    syncs: Vec<bool>,
}

/// What a shadow local holds at a point of the function, on every path that reaches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shadow {
    /// The null handle: what every local starts as, and what a synchronization stores for a register
    /// no longer read.
    Null,
    /// The value its register holds: stored by a synchronization, and the register not touched since.
    Register,
    /// Anything else: the paths that reach the point disagree, or the register was touched since.
    Unknown,
}

impl Frame {
    pub(super) fn new(plan: &Plan, functions: &[CompiledFunction], may_safepoint: &[bool]) -> Self {
        let registers: Vec<_> = (0..plan.num_regs)
            .filter(|r| counted_kind(plan.kinds.get(*r as usize)))
            .map(|r| r as u16)
            .collect();
        let index: std::collections::HashMap<_, _> =
            registers.iter().enumerate().map(|(i, r)| (*r, i)).collect();
        let words = registers.len().div_ceil(64);
        // Mutable parameters are implicit function results: a caller reads
        // their final values even when no explicit Return operand names them.
        let mut exit = vec![0_u64; words];
        for r in &plan.inout {
            if let Some(&i) = index.get(r) {
                exit[i / 64] |= 1_u64 << (i % 64);
            }
        }
        let n = plan.ops.len();
        let mut defs = vec![0_u64; n * words];
        let mut uses = vec![0_u64; n * words];
        let mut successors = Vec::with_capacity(n);
        for (pc, op) in plan.ops.iter().enumerate() {
            let (d, u) = crate::vm::instruction::op_def_uses(op, functions);
            for (bits, regs) in [(&mut defs, d), (&mut uses, u)] {
                for r in regs {
                    if let Some(&i) = index.get(&r) {
                        bits[pc * words + i / 64] |= 1_u64 << (i % 64);
                    }
                }
            }
            successors.push(crate::vm::instruction::op_successors(op, pc, n));
        }
        let mut live = vec![0_u64; n * words];
        let mut changed = true;
        while changed {
            changed = false;
            for pc in (0..n).rev() {
                for w in 0..words {
                    let at_exit = if successors[pc].is_empty() || pc + 1 == n {
                        exit[w]
                    } else {
                        0
                    };
                    let out = successors[pc].iter().fold(at_exit, |bits, &next| {
                        bits | if next < n {
                            live[next * words + w]
                        } else {
                            u64::MAX
                        }
                    });
                    let value = uses[pc * words + w] | (out & !defs[pc * words + w]);
                    if value != live[pc * words + w] {
                        live[pc * words + w] = value;
                        changed = true;
                    }
                }
            }
        }
        let touched = defs.iter().zip(&uses).map(|(d, u)| d | u).collect();
        let mut frame = Self {
            registers,
            words,
            live,
            base: plan.num_regs
                + 25
                + exact::extra_locals(plan) as u32
                + absence::extra_locals(plan)
                + tower::extra_locals(plan),
            held: vec![None; n],
            touched,
            syncs: plan
                .ops
                .iter()
                .enumerate()
                .map(|(pc, op)| Self::reaches_a_safepoint(plan, op, pc, may_safepoint))
                .collect(),
        };
        // Forward over the same edges: every shadow starts null, a synchronization leaves each shadow
        // holding what it stored, and an op that touches a register may leave it a value its shadow does
        // not hold.
        if n > 0 {
            frame.held[0] = Some(vec![Shadow::Null; frame.registers.len()]);
        }
        let mut pending: Vec<usize> = if n > 0 { vec![0] } else { Vec::new() };
        while let Some(pc) = pending.pop() {
            let out = frame.after(pc);
            for &next in &successors[pc] {
                if next >= n {
                    continue;
                }
                let joined = match &frame.held[next] {
                    None => out.clone(),
                    Some(before) => before
                        .iter()
                        .zip(&out)
                        .map(|(a, b)| if a == b { *a } else { Shadow::Unknown })
                        .collect(),
                };
                if frame.held[next].as_ref() != Some(&joined) {
                    frame.held[next] = Some(joined);
                    pending.push(next);
                }
            }
        }
        frame
    }

    /// Whether the op at `pc` can reach a safepoint before the next op runs: a loop's back edge is one,
    /// and a call reaches one when what it runs can ([`super::may_safepoint`]).
    fn reaches_a_safepoint(plan: &Plan, op: &Op, pc: usize, may_safepoint: &[bool]) -> bool {
        match *op {
            Op::Jump { target }
            | Op::JumpIfFalse { target, .. }
            | Op::JumpIfTrue { target, .. } => target <= pc,
            Op::Call { func, .. } | Op::Spawn { func, .. } | Op::SpawnHandle { func, .. } => {
                may_safepoint.get(func as usize).copied().unwrap_or(true)
            }
            Op::CallValue { .. } => true,
            Op::CallBuiltin {
                builtin: BuiltinId::TextFromBytes,
                ..
            } => true,
            Op::CallBuiltin {
                args_start,
                arg_count,
                ..
            } => (0..arg_count)
                .any(|a| plan.kinds.get((args_start + a) as usize) == Some(Kind::Closure)),
            _ => false,
        }
    }

    fn is_live(&self, pc: usize, i: usize) -> bool {
        self.live[pc * self.words + i / 64] & (1_u64 << (i % 64)) != 0
    }

    /// What register `i`'s shadow holds before the op at `pc` — `Unknown` where no path is known.
    fn held_before(&self, pc: usize, i: usize) -> Shadow {
        self.held
            .get(pc)
            .and_then(|h| h.as_ref())
            .map_or(Shadow::Unknown, |h| h[i])
    }

    /// What a synchronization before the op at `pc` stores in register `i`'s shadow.
    fn stored(&self, pc: usize, i: usize) -> Shadow {
        if self.is_live(pc, i) {
            Shadow::Register
        } else {
            Shadow::Null
        }
    }

    /// What every shadow holds once the op at `pc` has run.
    fn after(&self, pc: usize) -> Vec<Shadow> {
        (0..self.registers.len())
            .map(|i| {
                let mut held = self.held_before(pc, i);
                if self.syncs[pc] {
                    held = self.stored(pc, i);
                }
                let touched = self.touched[pc * self.words + i / 64] & (1_u64 << (i % 64)) != 0;
                if touched && held == Shadow::Register {
                    held = Shadow::Unknown;
                }
                held
            })
            .collect()
    }

    pub(super) fn len(&self) -> usize {
        self.registers.len()
    }

    fn sync(&self, c: &mut Vec<u8>, ctx: &Ctx, pc: usize) {
        for (i, &register) in self.registers.iter().enumerate() {
            if self.held_before(pc, i) == self.stored(pc, i) {
                continue;
            }
            let shadow = self.base + i as u32;
            let live = self.is_live(pc, i);
            let value = |c: &mut Vec<u8>| {
                if live {
                    local_get(c, u32::from(register));
                } else {
                    i32_const(c, 0);
                }
            };
            value(c);
            local_get(c, shadow);
            c.push(0x47);
            c.push(0x04);
            c.push(0x40);
            // Acquire the new root before relinquishing the old one; they may be one object.
            emit_true_retain_of(c, ctx, value);
            emit_true_release(c, ctx, |c| local_get(c, shadow));
            value(c);
            local_set(c, shadow);
            c.push(0x0B);
        }
    }

    pub(super) fn before(&self, c: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, pc: usize) {
        let op = &plan.ops[pc];
        let backedge = matches!(op, Op::Jump { target } | Op::JumpIfFalse { target, .. }
            | Op::JumpIfTrue { target, .. } if *target <= pc);
        if self.syncs[pc] {
            self.sync(c, ctx, pc);
        }
        if backedge {
            c.push(0x10);
            leb_u32(
                c,
                ctx.runtime
                    .expect("counted batch runtime")
                    .index(RuntimeFn::Safepoint)
                    .expect("safepoint helper"),
            );
        }
        if matches!(op, Op::Return { .. } | Op::ReturnNothing | Op::Halt) {
            let held: Vec<Shadow> = (0..self.registers.len())
                .map(|i| self.held_before(pc, i))
                .collect();
            self.release(c, ctx, &held);
        }
    }

    /// Leave the function from the end of its last op, at `pc`: every shadow that may hold a value
    /// gives it up.
    pub(super) fn leave(&self, c: &mut Vec<u8>, ctx: &Ctx, pc: usize) {
        let held = if self.held.get(pc).is_some_and(Option::is_some) {
            self.after(pc)
        } else {
            vec![Shadow::Unknown; self.registers.len()]
        };
        self.release(c, ctx, &held);
    }

    fn release(&self, c: &mut Vec<u8>, ctx: &Ctx, held: &[Shadow]) {
        for (i, held) in held.iter().enumerate() {
            if *held == Shadow::Null {
                continue;
            }
            let shadow = self.base + i as u32;
            emit_true_release(c, ctx, |c| local_get(c, shadow));
        }
    }
}
