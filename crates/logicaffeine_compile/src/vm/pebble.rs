//! Pebbling — the reverse-liveness pass that decides which journal entries a recording run actually
//! NEEDS, driving the Bennett history tape toward the Landauer floor.
//!
//! **The idea.** The tape is not fundamental. Most journaled bits are *recomputable, not destroyed*:
//! a scratch temporary `t = f(a,b)` with its inputs still live is a pure function of live state, and
//! reverse execution only reads live registers for `Bijection` ops (`RevAdd`'s `place`/`delta`, …) and
//! collection reverses — every `Journaled` overwrite restores its destination straight from the tape
//! without reading its operands. So a scratch write that only feeds `Journaled` results has an old
//! value **no reverse step ever reads**; recording it was a naive-forward artifact. Eliding it is
//! sound and shrinks the tape.
//!
//! **P1 rule (sound set-membership).** A Journaled single-destination overwrite is `Elide`d iff its
//! register is (a) not program-observable — not named in its frame ([`CompiledProgram::named_regs`] /
//! [`CompiledFunction::named_regs`](crate::vm::CompiledFunction::named_regs)) — AND (b) never a reverse-read operand
//! ([`crate::vm::reverse::reverse_reads`]). Both the recorder ([`crate::vm::Vm`]'s `record_op`) and the
//! reverse executor consult the plan, so an elided entry is neither pushed nor popped.
//!
//! Empty `named_regs` (hand-built bytecode / no name info) ⇒ everything is treated observable ⇒
//! nothing is elided ⇒ the reverse executor's full-fidelity behaviour is preserved byte-for-byte.
//!
//! [`CompiledFunction::named_regs`]: crate::vm::CompiledFunction::named_regs

use std::collections::HashSet;

use crate::vm::instruction::{CompiledProgram, Op, Reg};
use crate::vm::reverse::{self, ReverseMode};

/// What the recorder should do with a Journaled write's tape entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JournalDecision {
    /// Journal the destroyed value — the reverse executor will read it back.
    Keep,
    /// Skip the entry: the destroyed value is program-invisible scratch that no reverse step reads.
    Elide,
    /// (`Adiabatic`, P3) Skip the entry; recompute the value on reverse from its pure defining segment.
    Recompute,
}

/// Per-pc journaling decisions for a recording run in `mode`. `FullState` keeps everything (today's
/// behaviour, made explicit); the pebbling modes elide scratch whose destroyed value no reverse step
/// reads. Indexed by absolute pc in [`CompiledProgram::code`].
pub fn plan(prog: &CompiledProgram, mode: ReverseMode) -> Vec<JournalDecision> {
    plan_budgeted(prog, mode, 0)
}

/// [`plan`] with an explicit pebble CHECKPOINT budget (corner 2). Under `Adiabatic`, `budget` recomputable
/// chain intermediates are KEPT on the tape (journaled once — `landauer = 0`, since a recomputable
/// scratch is first-def so its displaced value is `Nothing`) instead of upgraded to `Recompute`. A
/// reverse step that reads a node above a checkpoint then re-walks only down to the nearest checkpoint,
/// not to the chain root — the Bennett space-for-time trade. `budget = 0` is the recompute-everything
/// corner (`O(1)` tape, chain·reads time); higher budgets bound the re-walk length.
pub fn plan_budgeted(
    prog: &CompiledProgram,
    mode: ReverseMode,
    budget: usize,
) -> Vec<JournalDecision> {
    let n = prog.code.len();
    if mode == ReverseMode::FullState {
        return vec![JournalDecision::Keep; n];
    }
    // FRAME-OVERLAP SOUNDNESS (Piece 1). A callee runs at `base = args_start`, so its registers ALIAS
    // the caller's slots `[args_start..]` — a register that reads as "scratch" in a callee frame can be
    // the SAME absolute slot as a NAMED variable in the caller. We resolve this exactly: compute each
    // function's possible absolute BASE(s) from the call graph, then elide a write only if NONE of the
    // absolute slots it can occupy is a Main-observable slot. Recursion / dynamic dispatch give
    // unbounded / unknown bases → elision is disabled (sound).
    let bases = match frame_bases(prog) {
        Some(b) => b,
        None => return vec![JournalDecision::Keep; n],
    };
    // The absolute slots compared at reverse end = Main's named registers. Empty `named_regs` (hand-
    // built bytecode) ⇒ EVERY Main slot is observable, so nothing touching Main's frame is elided —
    // preserving the reverse executor's full-fidelity tests.
    let main_size = prog.register_count;
    let main_all = prog.named_regs.is_empty();
    let main_observable = |slot: usize| -> bool {
        slot < main_size && (main_all || prog.named_regs.get(slot).copied().unwrap_or(false))
    };
    // Registers any reverse step reads live (Bijection operands + collection handles) — pinned Keep.
    let mut reverse_read: HashSet<Reg> = HashSet::new();
    for op in &prog.code {
        reverse_read.extend(reverse::reverse_reads(op));
    }
    let mut plan = (0..n)
        .map(|pc| {
            let op = &prog.code[pc];
            let Some(r) = reverse::overwritten_reg(op) else {
                return JournalDecision::Keep;
            };
            if !reverse::records(op) || reverse_read.contains(&r) {
                return JournalDecision::Keep;
            }
            let frame = frame_of(prog, pc);
            // Named in its own frame? (Empty `named_regs` ⇒ observable ⇒ keep.)
            let f_named = match frame {
                None => main_all || prog.named_regs.get(r as usize).copied().unwrap_or(true),
                Some(f) => {
                    let nm = &prog.functions[f].named_regs;
                    nm.is_empty() || nm.get(r as usize).copied().unwrap_or(true)
                }
            };
            if f_named {
                return JournalDecision::Keep;
            }
            // Elide only if NONE of the absolute slots this write can occupy (over every possible base)
            // is a Main-observable slot.
            let touches_observable = match frame {
                None => main_observable(r as usize),
                Some(f) => bases[f].iter().any(|&b| main_observable(b + r as usize)),
            };
            if touches_observable {
                JournalDecision::Keep
            } else {
                JournalDecision::Elide
            }
        })
        .collect::<Vec<_>>();
    // ADIABATIC (Piece 2): upgrade each recomputable reverse-live scratch DEF from `Keep` to
    // `Recompute` — re-run on reverse from stable live inputs instead of journaled, pushing the tape
    // BELOW Observable while keeping full fidelity. Corner 2: a `budget` of chain intermediates stays
    // `Keep` (a CHECKPOINT) so downstream recomputes stop there instead of re-walking to the root.
    if mode == ReverseMode::Adiabatic {
        let checkpoints = checkpoint_defs(prog, budget);
        for (_frame, _r, def_pc, _op) in recomputable(prog) {
            if !checkpoints.contains(&def_pc) {
                plan[def_pc] = JournalDecision::Recompute;
            }
        }
    }
    plan
}

/// The recomputable-chain intermediates to KEEP as checkpoints (corner 2) — the `budget` nodes that
/// most shorten the reverse re-walk. An INTERMEDIATE is a recomputable node whose register is an input
/// to another recomputable def in the same frame (so it sits *inside* a chain, not at its top). We
/// checkpoint the intermediates CLOSEST to the chain tops (highest `def_pc`) first: keeping a node near
/// the top caps every above-it read at a re-walk of length 1, the largest marginal saving per pebble
/// (the Bennett "cache near the read" heuristic). `budget = 0` ⇒ none (recompute everything).
pub fn checkpoint_defs(prog: &CompiledProgram, budget: usize) -> HashSet<usize> {
    if budget == 0 {
        return HashSet::new();
    }
    let rec = recomputable(prog);
    // (frame, input-reg) pairs consumed by some recomputable def — the interior edges of the chains.
    let mut consumed: HashSet<(Option<usize>, Reg)> = HashSet::new();
    for (frame, _r, _pc, op) in &rec {
        for input in def_inputs(op) {
            consumed.insert((*frame, input));
        }
    }
    let mut intermediates: Vec<usize> = rec
        .iter()
        .filter(|(frame, r, _pc, _op)| consumed.contains(&(*frame, *r)))
        .map(|(_f, _r, pc, _op)| *pc)
        .collect();
    // Highest def_pc first (closest to the chain tops), then take `budget` of them.
    intermediates.sort_unstable_by(|a, b| b.cmp(a));
    intermediates.into_iter().take(budget).collect()
}

/// The two input registers of a pure two-operand recomputable def (kept in lockstep with
/// `recomputable`'s accepted-op set). Empty for anything else.
fn def_inputs(op: &Op) -> Vec<Reg> {
    match *op {
        Op::Add { lhs, rhs, .. }
        | Op::Sub { lhs, rhs, .. }
        | Op::Mul { lhs, rhs, .. }
        | Op::BitXor { lhs, rhs, .. }
        | Op::Div { lhs, rhs, .. }
        | Op::ExactDiv { lhs, rhs, .. }
        | Op::FloorDiv { lhs, rhs, .. }
        | Op::Mod { lhs, rhs, .. }
        | Op::Pow { lhs, rhs, .. }
        | Op::BitAnd { lhs, rhs, .. }
        | Op::BitOr { lhs, rhs, .. }
        | Op::Shl { lhs, rhs, .. }
        | Op::Shr { lhs, rhs, .. } => vec![lhs, rhs],
        _ => vec![],
    }
}

/// The pc range `[lo, hi)` of a frame — Main is `[0, first-function-entry)`, function `i` is
/// `[entry_pc_i, next-entry)` (bodies are contiguous in `entry_pc` order; see `vm/compiler.rs`).
fn frame_range(prog: &CompiledProgram, frame: Option<usize>) -> (usize, usize) {
    match frame {
        None => (
            0,
            prog.functions
                .iter()
                .map(|f| f.entry_pc)
                .min()
                .unwrap_or(prog.code.len()),
        ),
        Some(i) => {
            let lo = prog.functions[i].entry_pc;
            let hi = prog
                .functions
                .iter()
                .map(|f| f.entry_pc)
                .filter(|&e| e > lo)
                .min()
                .unwrap_or(prog.code.len());
            (lo, hi)
        }
    }
}

/// Reverse-live scratch registers that can be RECOMPUTED on reverse instead of journaled — Bennett
/// uncompute — keyed by the FRAME they live in (register numbers are frame-relative). Each is a
/// SCRATCH register that some reverse step in its frame reads live (so `Observable`/`FullState` must
/// journal it), with a SINGLE pure-arithmetic def whose inputs are STABLE within the frame, and that
/// (via the frame-overlap bases) aliases NO Main-observable slot. Re-running the def during reverse
/// with `self.base` set to the frame reconstructs it exactly. Returns `(frame, register, def_pc,
/// def_op)`. Dynamic dispatch (`CallValue`, unknown callee) yields nothing; recursion is handled via
/// the bounded frame bases (corner 4) — each activation recomputes at its own live base.
pub fn recomputable(prog: &CompiledProgram) -> Vec<(Option<usize>, Reg, usize, Op)> {
    if prog
        .code
        .iter()
        .any(|op| matches!(op, Op::CallValue { .. }))
    {
        return Vec::new();
    }
    let bases = match frame_bases(prog) {
        Some(b) => b,
        None => return Vec::new(),
    };
    let main_size = prog.register_count;
    let main_all = prog.named_regs.is_empty();
    let main_observable = |slot: usize| {
        slot < main_size && (main_all || prog.named_regs.get(slot).copied().unwrap_or(false))
    };
    // The registers an op WRITES (mutates) — a fresh overwrite, a collection mutation, or a Bijection's
    // in-place `place`/`a`/`b`. NOT a Bijection's `delta` (read-only, surfaced by `reverse_reads`).
    let writes = |op: &Op| -> Vec<Reg> {
        match *op {
            Op::RevAdd { place, .. } | Op::RevSub { place, .. } | Op::RevXor { place, .. } => {
                vec![place]
            }
            Op::RevNeg { place } => vec![place],
            Op::RevSwap { a, b } => vec![a, b],
            _ => reverse::overwritten_reg(op)
                .into_iter()
                .chain(reverse::mutated_collection_reg(op))
                .collect(),
        }
    };
    let frames = std::iter::once(None).chain((0..prog.functions.len()).map(Some));
    let mut out = Vec::new();
    for frame in frames {
        let (lo, hi) = frame_range(prog, frame);
        let named: &[bool] = match frame {
            None => &prog.named_regs,
            Some(i) => &prog.functions[i].named_regs,
        };
        if named.is_empty() {
            continue; // no name info for this frame ⇒ do not recompute (safe fallback)
        }
        let mut reverse_read: Vec<Reg> = Vec::new();
        for pc in lo..hi {
            for r in reverse::reverse_reads(&prog.code[pc]) {
                if !reverse_read.contains(&r) {
                    reverse_read.push(r);
                }
            }
        }
        for r in reverse_read {
            if named.get(r as usize).copied().unwrap_or(true) {
                continue; // named — must be journaled to restore its value
            }
            // Frame-overlap: skip if any absolute slot this register can occupy is Main-observable.
            let aliases_observable = match frame {
                None => main_observable(r as usize),
                Some(i) => bases[i].iter().any(|&b| main_observable(b + r as usize)),
            };
            if aliases_observable {
                continue;
            }
            let defs: Vec<usize> = (lo..hi)
                .filter(|&pc| reverse::overwritten_reg(&prog.code[pc]) == Some(r))
                .collect();
            if defs.len() != 1 {
                continue;
            }
            let def_pc = defs[0];
            let def_op = prog.code[def_pc];
            // The whole pure two-operand family is recomputable (corner 3): re-running any of these from
            // the same live inputs reproduces the exact value. Keep in lockstep with `recompute_into`.
            let (lhs, rhs) = match def_op {
                Op::Add { lhs, rhs, .. }
                | Op::Sub { lhs, rhs, .. }
                | Op::Mul { lhs, rhs, .. }
                | Op::BitXor { lhs, rhs, .. }
                | Op::Div { lhs, rhs, .. }
                | Op::ExactDiv { lhs, rhs, .. }
                | Op::FloorDiv { lhs, rhs, .. }
                | Op::Mod { lhs, rhs, .. }
                | Op::Pow { lhs, rhs, .. }
                | Op::BitAnd { lhs, rhs, .. }
                | Op::BitOr { lhs, rhs, .. }
                | Op::Shl { lhs, rhs, .. }
                | Op::Shr { lhs, rhs, .. } => (lhs, rhs),
                _ => continue,
            };
            // Stability WITHIN the frame: neither the register nor its inputs are re-written after the
            // def, so the reverse-time recompute reproduces the exact forward value.
            let modified =
                |reg: Reg| ((def_pc + 1)..hi).any(|pc| writes(&prog.code[pc]).contains(&reg));
            if modified(r) || modified(lhs) || modified(rhs) {
                continue;
            }
            out.push((frame, r, def_pc, def_op));
        }
    }
    out
}

/// A static summary of a mode's plan: how many Journaled entries survive vs. are elided.
#[derive(Debug, Clone)]
pub struct PebbleReport {
    pub mode: ReverseMode,
    pub ops: usize,
    pub journaled: usize,
    pub elided: usize,
}

/// Summarise the plan for `mode` — the count of Journaled writes kept vs. elided (the static
/// space budget; the dynamic erased-byte floor comes from a recording run's tape).
pub fn report(prog: &CompiledProgram, mode: ReverseMode) -> PebbleReport {
    let plan = plan(prog, mode);
    let (mut journaled, mut elided) = (0usize, 0usize);
    for (pc, op) in prog.code.iter().enumerate() {
        if reverse::records(op) {
            match plan[pc] {
                JournalDecision::Keep => journaled += 1,
                JournalDecision::Elide | JournalDecision::Recompute => elided += 1,
            }
        }
    }
    PebbleReport {
        mode,
        ops: prog.code.len(),
        journaled,
        elided,
    }
}

/// The function that owns `pc` — the one with the greatest `entry_pc ≤ pc` (Main is compiled first,
/// then each function body appended contiguously; see `vm/compiler.rs` Pass 2a/2b). `None` = Main.
pub fn frame_of(prog: &CompiledProgram, pc: usize) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (i, f) in prog.functions.iter().enumerate() {
        if f.entry_pc <= pc && best.map_or(true, |b| f.entry_pc > prog.functions[b].entry_pc) {
            best = Some(i);
        }
    }
    best
}

/// Each function's set of possible absolute BASE offsets — the register-window origins it can run at,
/// propagated from Main (base 0) through the call graph (`callee_base = caller_base + args_start`).
///
/// RECURSION-SAFE (corner 4). The elision decision only ever asks `main_observable(base + r)`, which
/// is `false` whenever `base + r ≥ main_size`. So a base `≥ main_size` is USELESS — it can never make
/// any slot observable — and we drop it. Dropping useless bases truncates a recursion's
/// arithmetic-progression bases (`b₀, b₀+A, b₀+2A, …`) to a FINITE set inside `[0, main_size)`, so the
/// monotone-insertion fixpoint TERMINATES even when the call graph has a cycle — no bail on recursion.
/// This loses nothing: every base that could make a slot observable satisfies `base < main_size` and is
/// kept; the multi-hop overlap (a grandchild aliasing a grandparent-Main slot) is captured because the
/// propagated `base` already sums every hop's `args_start`. Only dynamic dispatch (`CallValue` — the
/// callee, hence its base, is not statically known) still returns `None` and disables elision. An
/// uncalled (or only-beyond-Main) function keeps an empty set — sound, its scratch aliases no Main slot.
fn frame_bases(prog: &CompiledProgram) -> Option<Vec<Vec<usize>>> {
    if prog
        .code
        .iter()
        .any(|op| matches!(op, Op::CallValue { .. }))
    {
        return None;
    }
    let n = prog.functions.len();
    let main_size = prog.register_count;
    // Static call sites: (caller frame, args_start, callee index).
    let sites: Vec<(Option<usize>, usize, usize)> = prog
        .code
        .iter()
        .enumerate()
        .filter_map(|(pc, op)| match op {
            Op::Call {
                func, args_start, ..
            } => Some((frame_of(prog, pc), *args_start as usize, *func as usize)),
            _ => None,
        })
        .collect();
    let mut bases: Vec<std::collections::BTreeSet<usize>> = vec![Default::default(); n];
    // Monotone insertion into the finite domain `frames × [0, main_size)` converges in at most that
    // many steps; the cap is a backstop that the bounded domain guarantees is never hit.
    let cap = n * (main_size + 1) + n + 2;
    for _ in 0..cap {
        let mut changed = false;
        for &(caller, args_start, callee) in &sites {
            let caller_bases: Vec<usize> = match caller {
                None => vec![0],
                Some(c) => bases[c].iter().copied().collect(),
            };
            for cb in caller_bases {
                let b = cb + args_start;
                // Drop bases ≥ main_size (they can never alias a Main-observable slot) — this is what
                // makes the recursion fixpoint finite.
                if b < main_size && bases[callee].insert(b) {
                    changed = true;
                }
            }
        }
        if !changed {
            return Some(bases.into_iter().map(|s| s.into_iter().collect()).collect());
        }
    }
    None
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/pebble/tests.rs"]
mod tests;
