//! Reversibility classification of the bytecode ISA — the static substrate for running the whole
//! language BACKWARD (`forward ; reverse ≡ identity`).
//!
//! Because the register-VM opcode set is the chip ISA, a reversibility
//! strategy for every op is the software prerequisite for adiabatic / sub-Landauer hardware that
//! recycles charge instead of erasing it. Every
//! instruction lands in exactly one of four working strategies (or the empty `NonReversible`
//! census), and [`reversibility`] is EXHAUSTIVE on `Op` with NO wildcard arm — a new instruction
//! cannot compile until it is classified, so reversibility can never be silently broken. The
//! `tests/reverse_op_lock.rs` catalog lock enforces the census, the monotone zero-garbage floor,
//! and inverse well-formedness.
//!
//! This module is the STATIC half. The behavioural half — a reverse executor proving
//! `forward ; reverse` restores the exact initial state, with the history tape at zero bytes for
//! the abelian core — lands in R1+ (`vm::journal`, `Vm::run_reverse`).

use crate::vm::instruction::{Op, Reg};

/// How an op participates in reversible execution. Four working strategies plus a census bucket
/// that MUST stay empty. `Copy` so a lock can classify every variant without allocation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RevClass {
    /// Injective on machine state with a named inverse — reverses with ZERO garbage (the abelian /
    /// involution core; the adiabatic charge-recovery path). The zero-erasure ideal.
    Bijection(InverseSpec),
    /// Information-losing — reversed by a minimal Bennett delta appended to the history tape
    /// ([`JournalKind`] names WHAT the delta records). Correct, but pays garbage until uncomputed.
    Journaled(JournalKind),
    /// An observable side effect — reversed via the effect-log and, where the world cannot be
    /// recalled, a NAMED irreversibility barrier ([`EffectKind`] names the effect family).
    Effectful(EffectKind),
    /// Control flow or a value-preserving instruction — reversed on the recorded execution trace
    /// (branch/loop/call decisions), with no data inverse and no journal entry.
    Control,
    /// GENUINE census of ops with no reversibility story yet. This MUST stay ∅ — see the lock.
    NonReversible(&'static str),
}

/// How a [`RevClass::Bijection`] op inverts — the zero-garbage inverse recipe.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InverseSpec {
    /// The op is its own inverse: applying it twice is the identity (`Not`-into-self, `Swap`,
    /// additive negation, XOR-by-a-value). `f ∘ f = id`.
    Involution,
    /// The inverse is the SAME op variant with a flag flipped (e.g. `CrdtBump { negate }` inverts
    /// by flipping `negate`). The variant carries its own inverse direction.
    SelfFamily,
    /// The inverse is a DISTINCT op variant, named by its [`Op::name`] (e.g. `RevAdd`⁻¹ = `RevSub`).
    /// The lock checks the name resolves to a real ISA op.
    NamedOp(&'static str),
}

/// What a [`RevClass::Journaled`] op must record on the history tape to be reversible — the minimal
/// destroyed information (Bennett/Landauer embedding), not a full state snapshot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JournalKind {
    /// Writes a destination register/global that is NOT one of its operands: the operands survive,
    /// so reversal only needs the destination's PRIOR value (one entry). Covers the bulk of the
    /// three-address ISA — arithmetic, comparisons, constructors, loads, field reads.
    Overwrite,
    /// Writes a destination that IS one of its operands (`x := x ⊕ e`): the prior value is lost.
    /// Reversible algebraically when `⊕` is an abelian group op (the R1 promotion to `Bijection`),
    /// or by journaling the prior value.
    InPlace,
    /// Mutates a heap collection in place (push / pop / remove / index-store / struct-field write):
    /// reversal records the structural delta (the pushed slot to pop, the popped element + index to
    /// re-insert, the overwritten slot's old value).
    Collection,
    /// A CRDT join-semilattice step (merge / resolve) that collapses lattice points and cannot be
    /// inverted algebraically: reversal snapshots the target replica's pre-state (bounded to that
    /// one value, not the whole machine).
    CrdtSnapshot,
}

/// The family of observable effect a [`RevClass::Effectful`] op performs — determines how far
/// reversal can go before it hits a named barrier.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EffectKind {
    /// Buffered program output (`Show`) — fully reversible by truncating the output buffer.
    Output,
    /// Reads the environment clock (`today` / `now`) — reversed via the value recorded on the
    /// determinism trace, so a reverse-then-re-forward replays the identical instant.
    Clock,
    /// Advances virtual time (`Sleep`) — a no-op on observable state; reverses as a no-op.
    Timer,
    /// Deterministic in-process concurrency (channels / tasks / select) — internal queue and
    /// scheduler mutations, reversed via the recorded schedule trace.
    Concurrency,
    /// Peer networking over the relay — the genuinely external frontier; internal state reverses
    /// fully, but an already-observed send raises a NAMED irreversibility barrier.
    Network,
}

/// The reversibility fidelity ⇄ space ⇄ time point a recording run is asked to hit — the pebbling
/// knob (`vm/pebble.rs`). The tape is only as large as the mode demands.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ReverseMode {
    /// Restore the program-OBSERVABLE state (named variables, globals, output, iterators); compiler
    /// scratch is program-invisible, so its journal is elided. Smallest tape, fastest. The default.
    #[default]
    Observable,
    /// Restore EVERY register bit-exactly by journaling all destroyed values. Largest tape, simplest.
    FullState,
    /// Restore every register bit-exactly at the LANDAUER FLOOR: recompute pure scratch on reverse
    /// (Bennett uncompute) instead of journaling it. Full fidelity, minimal tape, +recompute time —
    /// the adiabatic chip contract.
    Adiabatic,
}

/// The registers a REVERSE step reads from the live machine (not from the tape) when undoing `op` —
/// so their values must be intact at reverse time. Journaled overwrites restore their destination
/// straight from the tape and read NOTHING live, so they contribute none; only the `Bijection` ops
/// (which recompute in place) and the collection/CRDT reverses (which read the live handle) do.
/// Eliding a journal entry for a register in this set could corrupt a later reverse-read, so the
/// pebbler treats membership here as "must keep". Keep in lockstep with the reverse executor.
pub fn reverse_reads(op: &Op) -> Vec<Reg> {
    match op {
        Op::RevAdd { place, delta } | Op::RevSub { place, delta } | Op::RevXor { place, delta } => {
            vec![*place, *delta]
        }
        Op::RevNeg { place } => vec![*place],
        Op::RevSwap { a, b } => vec![*a, *b],
        // An in-place division reverses by reconstructing `a_old = q·b + r`, reading the kept quotient
        // straight from `dst` at reverse time — so `dst` must not be elided out from under it.
        Op::Div { dst, lhs, .. } if dst == lhs => vec![*dst],
        // In-place shifts / masks / modulo likewise reconstruct `a_old` from the KEPT result in `dst`
        // (plus bits/quotient held self-contained on the tape), so `dst` must survive elision.
        Op::Shr { dst, lhs, .. } | Op::Shl { dst, lhs, .. } if dst == lhs => vec![*dst],
        Op::DivPow2 { dst, lhs, .. } if dst == lhs => vec![*dst],
        Op::BitAnd { dst, lhs, .. } | Op::BitOr { dst, lhs, .. } if dst == lhs => vec![*dst],
        Op::Mod { dst, lhs, .. } if dst == lhs => vec![*dst],
        // `ListPop`/`CrdtAppend`/`CrdtResolve` and every `mutated_collection` op reverse THROUGH the
        // live handle (`reverse_collection_mutate` reads it for the aliasing check), so that register
        // must survive.
        Op::ListPop { list, .. } => vec![*list],
        Op::CrdtAppend { seq, .. } => vec![*seq],
        Op::CrdtResolve { obj, .. } => vec![*obj],
        _ => mutated_collection_reg(op).into_iter().collect(),
    }
}

/// ★ THE STATIC REVERSIBILITY CATALOG ★ — the single source of truth for how every VM bytecode op
/// is run backward. EXHAUSTIVE on `Op` (no `_` arm): adding an instruction fails to compile until it
/// is classified, so no op can silently escape a reversibility strategy. Keep this in lockstep with
/// the reverse executor (`vm::journal` / `Vm::run_reverse`, R1+) — the behavioural roundtrip lock
/// proves they agree.
///
/// R0 ships the classification only; the abelian promotions (`Journaled(InPlace)` → `Bijection`,
/// e.g. `CrdtBump`) land in R1 alongside the reverse executor that proves each roundtrip.
pub fn reversibility(op: &Op) -> RevClass {
    use EffectKind as E;
    use InverseSpec::{Involution, NamedOp};
    use JournalKind::{Collection, CrdtSnapshot, InPlace, Overwrite};
    use RevClass::{Bijection, Control, Effectful, Journaled};

    match op {
        // ── Abelian / involution core — ZERO garbage: self-inverts / algebraic inverse, no tape ──
        // Swapping two registers is its own inverse; reverse execution re-applies it.
        Op::RevSwap { .. } => Bijection(Involution),
        // In-place XOR over ℤ/2ⁿ is self-inverse (`x ⊕ e ⊕ e = x`); reverse re-applies it.
        Op::RevXor { .. } => Bijection(Involution),
        // Modular negation is self-inverse (`-(-x) = x`).
        Op::RevNeg { .. } => Bijection(Involution),
        // The (ℤ/2ⁿ, +) additive translations: RevAdd and RevSub are MUTUAL inverses.
        Op::RevAdd { .. } => Bijection(NamedOp("RevSub")),
        Op::RevSub { .. } => Bijection(NamedOp("RevAdd")),

        // ── Value-preserving / control flow — reversed on the recorded trace, no data inverse ──
        // A copy-on-write ownership barrier leaves the observed value unchanged.
        Op::EnsureOwned { .. } => Control,
        // A native-region bounds hoist is a pure no-op outside the JIT.
        Op::RegionBoundsGuard { .. } => Control,
        // Branches carry no data mutation; the taken direction is recorded on the trace.
        Op::Jump { .. } | Op::JumpIfFalse { .. } | Op::JumpIfTrue { .. } => Control,
        // Calls / returns reverse via the frame trace (CallEnter/Leave, R2) + callee uncompute.
        Op::Call { .. } | Op::CallValue { .. } | Op::Return { .. } | Op::ReturnNothing => Control,
        // `Check …` traps on failure and is otherwise a no-op on state.
        Op::CheckPolicy { .. } => Control,
        // Snapshot-iterator bookkeeping (the iterator stack) is control state, reversed via the
        // Iter trace in R2.
        Op::IterPrepare { .. } | Op::IterNext { .. } | Op::IterPop => Control,
        // A terminal error / stop: nothing to reverse past it.
        Op::FailWith { .. } | Op::Halt => Control,

        // ── Overwrite: writes a fresh destination; operands survive → journal the old dst ──
        Op::LoadConst { .. } | Op::Move { .. } => Journaled(Overwrite),
        Op::Add { .. } | Op::Sub { .. } | Op::Mul { .. } | Op::Pow { .. } => Journaled(Overwrite),
        Op::BigIntWide { .. } => Journaled(Overwrite),
        Op::Div { .. } | Op::ExactDiv { .. } | Op::FloorDiv { .. } | Op::Mod { .. } => {
            Journaled(Overwrite)
        }
        Op::DivPow2 { .. } | Op::MagicDivU { .. } => Journaled(Overwrite),
        Op::Lt { .. } | Op::Gt { .. } | Op::LtEq { .. } | Op::GtEq { .. } => Journaled(Overwrite),
        Op::Eq { .. } | Op::NotEq { .. } | Op::ApproxEq { .. } => Journaled(Overwrite),
        Op::Not { .. } => Journaled(Overwrite),
        Op::BitXor { .. } | Op::BitAnd { .. } | Op::BitOr { .. } => Journaled(Overwrite),
        Op::Shl { .. } | Op::Shr { .. } => Journaled(Overwrite),
        Op::Concat { .. } | Op::SeqConcat { .. } | Op::FormatValue { .. } => Journaled(Overwrite),
        Op::SliceOp { .. } | Op::DeepClone { .. } => Journaled(Overwrite),
        Op::UnionOp { .. } | Op::IntersectOp { .. } => Journaled(Overwrite),
        Op::Length { .. } | Op::Contains { .. } => Journaled(Overwrite),
        Op::Index { .. } | Op::IndexUnchecked { .. } | Op::GetField { .. } => Journaled(Overwrite),
        Op::NewList { .. }
        | Op::NewEmptyList { .. }
        | Op::NewEmptyListWithCapacity { .. }
        | Op::NewEmptyListFixed { .. }
        | Op::NewEmptyListI32 { .. } => Journaled(Overwrite),
        Op::NewEmptySet { .. }
        | Op::NewEmptyMap { .. }
        | Op::NewEmptyMapWithCapacity { .. }
        | Op::NewRange { .. } => Journaled(Overwrite),
        Op::NewTuple { .. } | Op::DestructureTuple { .. } => Journaled(Overwrite),
        Op::NewStruct { .. } | Op::NewInductive { .. } | Op::NewCrdt { .. } => Journaled(Overwrite),
        Op::TestArm { .. } | Op::BindArm { .. } => Journaled(Overwrite),
        Op::GlobalGet { .. } | Op::GlobalSet { .. } => Journaled(Overwrite),
        Op::CallBuiltin { .. } => Journaled(Overwrite),
        Op::MakeClosure { .. } => Journaled(Overwrite),
        // `args()` reads the run's argv (fixed per run) into a fresh dst; reversal restores old dst.
        Op::Args { .. } => Journaled(Overwrite),
        // A `PeerAgent` handle is the address itself — pure, into a fresh dst.
        Op::NetMakePeer { .. } => Journaled(Overwrite),

        // ── In-place scalar (dst is an operand) — abelian, promoted to Bijection in R1 ──
        // `Set x to x + …` (amortized append form); `Increase/Decrease` counter bump.
        Op::AddAssign { .. } => Journaled(InPlace),
        Op::CrdtBump { .. } => Journaled(InPlace),

        // ── Collection: in-place heap mutation — journal the structural delta ──
        Op::ListPush { .. } | Op::ListPushField { .. } | Op::SetAdd { .. } => Journaled(Collection),
        Op::CrdtAppend { .. } => Journaled(Collection),
        Op::ListPop { .. } | Op::RemoveFrom { .. } => Journaled(Collection),
        Op::SetIndex { .. } | Op::SetIndexUnchecked { .. } => Journaled(Collection),
        Op::StructInsert { .. } => Journaled(Collection),

        // ── CRDT join-semilattice — lossy merge/resolve, snapshot the pre-state ──
        Op::CrdtMerge { .. } | Op::CrdtJoin { .. } | Op::CrdtResolve { .. } => {
            Journaled(CrdtSnapshot)
        }

        // ── Effectful — reversed via the effect-log / determinism trace / a named barrier ──
        Op::Show { .. } => Effectful(E::Output),
        Op::LoadToday { .. } | Op::LoadNow { .. } => Effectful(E::Clock),
        Op::Sleep { .. } => Effectful(E::Timer),
        Op::ChanNew { .. }
        | Op::ChanSend { .. }
        | Op::ChanRecv { .. }
        | Op::ChanTrySend { .. }
        | Op::ChanTryRecv { .. }
        | Op::ChanClose { .. } => Effectful(E::Concurrency),
        Op::Spawn { .. } | Op::SpawnHandle { .. } | Op::TaskAwait { .. } | Op::TaskAbort { .. } => {
            Effectful(E::Concurrency)
        }
        Op::SelectArmRecv { .. } | Op::SelectArmTimeout { .. } | Op::SelectWait { .. } => {
            Effectful(E::Concurrency)
        }
        Op::NetConnect { .. }
        | Op::NetListen { .. }
        | Op::NetSend { .. }
        | Op::NetStream { .. }
        | Op::NetAwait { .. }
        | Op::NetSync { .. }
        | Op::Tcp { .. }
        | Op::Serve { .. } => Effectful(E::Network),

        // Observation (Wave 13) writes only to the observation channel — an
        // output-class effect, reversed via the effect log exactly like `Show`.
        Op::Obs { .. } => Effectful(E::Output),
    }
}

/// The single register an op OVERWRITES with a fresh value — its data destination — for the ops
/// whose only state effect is that write and whose operands survive. Reverse execution restores this
/// register's prior value from the tape. `None` for ops that write a global / heap object / output
/// line instead (handled separately) or write nothing.
///
/// This is CORRECTNESS-GRADE and deliberately distinct from [`crate::vm::disasm::op_io`] (a debug
/// display helper whose lossy `_ => default` wildcard would silently report "no write" for an
/// unenumerated op — a reverse-execution landmine). Every op here is one the reverse executor is
/// proven to restore.
pub fn overwritten_reg(op: &Op) -> Option<Reg> {
    match op {
        Op::LoadConst { dst, .. }
        | Op::Move { dst, .. }
        | Op::GlobalGet { dst, .. }
        | Op::Add { dst, .. }
        | Op::Sub { dst, .. }
        | Op::Mul { dst, .. }
        | Op::Div { dst, .. }
        | Op::ExactDiv { dst, .. }
        | Op::FloorDiv { dst, .. }
        | Op::Mod { dst, .. }
        | Op::DivPow2 { dst, .. }
        | Op::MagicDivU { dst, .. }
        | Op::Lt { dst, .. }
        | Op::Gt { dst, .. }
        | Op::LtEq { dst, .. }
        | Op::GtEq { dst, .. }
        | Op::Eq { dst, .. }
        | Op::NotEq { dst, .. }
        | Op::ApproxEq { dst, .. }
        | Op::Not { dst, .. }
        | Op::Concat { dst, .. }
        | Op::SeqConcat { dst, .. }
        | Op::Pow { dst, .. }
        | Op::BitXor { dst, .. }
        | Op::BitAnd { dst, .. }
        | Op::BitOr { dst, .. }
        | Op::Shl { dst, .. }
        | Op::Shr { dst, .. }
        | Op::AddAssign { dst, .. }
        // Pure READS into a fresh dst — the source collection/struct is not mutated, so restoring
        // dst's prior value is an exact reversal (any freshly-aliased Rc drops back to its old count).
        | Op::Index { dst, .. }
        | Op::IndexUnchecked { dst, .. }
        | Op::Length { dst, .. }
        | Op::Contains { dst, .. }
        | Op::SliceOp { dst, .. }
        | Op::DeepClone { dst, .. }
        | Op::FormatValue { dst, .. }
        | Op::UnionOp { dst, .. }
        | Op::IntersectOp { dst, .. }
        | Op::GetField { dst, .. }
        | Op::TestArm { dst, .. }
        | Op::BindArm { dst, .. }
        // Pure CONSTRUCTION into a fresh dst — a brand-new heap object with no prior aliases, so
        // restoring dst (and dropping the new object) is exact. `NewCrdt` is EXCLUDED (it advances a
        // global replica-id counter — a side effect the store snapshot can't reverse).
        | Op::NewStruct { dst, .. }
        | Op::NewInductive { dst, .. }
        | Op::NewList { dst, .. }
        | Op::NewEmptyList { dst, .. }
        | Op::NewEmptyListWithCapacity { dst, .. }
        | Op::NewEmptyListFixed { dst, .. }
        | Op::NewEmptyListI32 { dst, .. }
        | Op::NewEmptySet { dst, .. }
        | Op::NewEmptyMap { dst, .. }
        | Op::NewEmptyMapWithCapacity { dst, .. }
        | Op::NewRange { dst, .. }
        | Op::NewTuple { dst, .. }
        // A closure snapshots its captures (deep-cloned) into a fresh dst — a pure constructor.
        | Op::MakeClosure { dst, .. }
        // `args()` reads the run's argv into a fresh dst; a `PeerAgent` handle is the address itself.
        | Op::Args { dst, .. }
        | Op::NetMakePeer { dst, .. } => Some(*dst),
        // A copy-on-write barrier rewrites its register (or is a no-op); restoring the prior handle
        // is exact (journaling it forces the clone, leaving the original pristine).
        Op::EnsureOwned { reg } => Some(*reg),
        _ => None,
    }
}

/// Whether forward execution RECORDS a history-tape delta for `op` — i.e. it destroys data a
/// reverse pass must restore. Exactly the ops that write a global/output line or a scalar register.
/// The DUAL of the reverse pass's pop condition: `record_op` pushes iff this is true, and
/// `run_reverse` pops iff this is true, so the tape stays aligned. A [`RevClass::Bijection`] op is
/// never here — it self-inverts with no tape entry (the zero-garbage core).
/// The single collection register an op mutates IN PLACE (push / index-set / add / remove) — for the
/// ops reversed by restoring the pristine pre-mutation snapshot ([`super::journal::JournalEntry::CollectionMutate`]).
/// `None` for non-mutating ops and for `ListPop` (which ALSO writes a dst — it carries its own
/// combined entry). Journaling this register's value forces the op's copy-on-write, so the original
/// stays pristine for the reverse.
pub fn mutated_collection_reg(op: &Op) -> Option<Reg> {
    match op {
        Op::ListPush { list, .. } => Some(*list),
        Op::SetAdd { set, .. } => Some(*set),
        Op::RemoveFrom { collection, .. } => Some(*collection),
        Op::SetIndex { collection, .. } | Op::SetIndexUnchecked { collection, .. } => {
            Some(*collection)
        }
        // A struct is a by-value `Box<StructValue>`: the pre-insert snapshot is a deep clone (the
        // replaced field's old value is captured), so restoring it is exact. (`ListPushField` is
        // EXCLUDED — it mutates a nested Rc list, which a struct clone would share, not snapshot.)
        Op::StructInsert { obj, .. } => Some(*obj),
        // A single-replica counter bump mutates a by-value struct's Int field — same deep-clone
        // snapshot as `StructInsert`.
        Op::CrdtBump { obj, .. } => Some(*obj),
        // A CRDT merge folds the source's fields into the target struct by REPLACING field values
        // with freshly-computed joins (`crdt_merge_field` is pure — it never mutates a nested `Rc`
        // in place), so a deep-clone snapshot of the target is exact, like `StructInsert`.
        Op::CrdtMerge { target, .. } | Op::CrdtJoin { target, .. } => Some(*target),
        Op::BigIntWide { dst, .. } => Some(*dst),
        _ => None,
    }
}

/// Whether a builtin is PURE — a deterministic function of its arguments, reversible by restoring the
/// destination and the cleared argument registers. Excludes the wire-subsystem builtins, which touch
/// external sinks or run shipped code (effects the store snapshot cannot reverse).
pub fn reversible_builtin(b: crate::semantics::builtins::BuiltinId) -> bool {
    use crate::semantics::builtins::BuiltinId;
    !matches!(
        b,
        BuiltinId::WireBytes
            | BuiltinId::ReadWireProgram
            | BuiltinId::WriteWireResidual
            | BuiltinId::RunAccepted
    )
}

pub fn records(op: &Op) -> bool {
    matches!(op, Op::GlobalSet { .. } | Op::Show { .. } | Op::ListPop { .. } | Op::DestructureTuple { .. })
        // CRDT interior mutations journal a deep pre-mutation snapshot of the shared handle.
        || matches!(op, Op::CrdtAppend { .. } | Op::CrdtResolve { .. })
        || matches!(op, Op::CallBuiltin { builtin, .. } if reversible_builtin(*builtin))
        || mutated_collection_reg(op).is_some()
        || overwritten_reg(op).is_some()
}

/// Whether the reverse executor supports `op` yet — the GROWING reversible subset. R1 covers the
/// abelian core (`RevSwap`, zero garbage), straight-line scalar compute ([`overwritten_reg`]), the
/// global write/read, output, and program stop. A program containing any other op declines recording
/// with a clear error rather than silently mis-reversing; the subset expands (control flow,
/// collections, effects) in R2/R3.
/// If `op` is a truly-EXTERNAL effect that cannot be un-done — the honest Landauer wall, an
/// already-sent network message can't be un-sent — name the irreversibility barrier. `None` for
/// reversible ops and for internally-modeled effects (buffered output, in-process channels, the
/// clock) that DO reverse. This turns an irreversibility into a NAMED, enumerable boundary instead of
/// a generic "unsupported", so the reverse executor never silently pretends the world can be rewound.
pub fn external_effect_barrier(op: &Op) -> Option<&'static str> {
    matches!(reversibility(op), RevClass::Effectful(EffectKind::Network)).then_some("network")
}

pub fn is_reversible_supported(op: &Op) -> bool {
    matches!(
        op,
        Op::Halt
            | Op::GlobalSet { .. }
            | Op::Show { .. }
            // Control flow — mutates only `pc`, reconstructed from the recorded step trace.
            | Op::Jump { .. }
            | Op::JumpIfFalse { .. }
            | Op::JumpIfTrue { .. }
            // Interpreter function calls — reversed by the CallUndo / ReturnUndo frame journal.
            | Op::Call { .. }
            | Op::CallValue { .. }
            | Op::Return { .. }
            | Op::ReturnNothing
            // `Repeat` iterator machinery — reversed by reconstructing the iterator stack.
            | Op::IterPrepare { .. }
            | Op::IterNext { .. }
            | Op::IterPop
            // Nondeterministic clock reads — reversed by dst-restore; the drawn instant is captured.
            | Op::LoadNow { .. }
            | Op::LoadToday { .. }
            // Abelian zero-garbage core.
            | Op::RevSwap { .. }
            | Op::RevXor { .. }
            | Op::RevNeg { .. }
            | Op::RevAdd { .. }
            | Op::RevSub { .. }
            // CRDT create + interior mutation. `NewCrdt` draws a replica id from a global counter —
            // captured nondeterminism (the clock pattern): reversed by dst-restore, the counter left
            // advanced. `CrdtAppend` mutates the shared `Rc<RefCell<CrdtValue>>` interior in place;
            // reversed by a deep interior snapshot restored through the live handle.
            | Op::NewCrdt { .. }
            | Op::CrdtAppend { .. }
            // A resolve onto a nested CRDT register field is a deep field-interior snapshot; onto a
            // plain field it is a Box snapshot (both from the `records`/reverse `CrdtResolve` path).
            | Op::CrdtResolve { .. }
            // In-place collection mutation that also writes a dst.
            | Op::ListPop { .. }
            // Tuple destructure (multi-register write) + a pure interpreter no-op guard.
            | Op::DestructureTuple { .. }
            | Op::RegionBoundsGuard { .. }
            // A Sleep advances virtual time only — no observable state change, reverses to nothing.
            | Op::Sleep { .. }
            // Fire-and-forget spawn — the child is a separate task VM the reversible scheduler
            // reverses on its own tape; the spawn op itself writes nothing, reversing to nothing.
            | Op::Spawn { .. }
            // Spawn-with-handle delivers a TaskHandle; Await joins a child, taking its result. Both
            // are reversed by the scheduler (handle / result restored from the recorded schedule).
            | Op::SpawnHandle { .. }
            | Op::TaskAwait { .. }
            // Channel create / send / receive — the op itself only suspends (writes nothing on the
            // task tape); the reversible scheduler undoes the channel delta + the delivered resume
            // value from the recorded schedule.
            | Op::ChanNew { .. }
            | Op::ChanSend { .. }
            | Op::ChanRecv { .. }
            // Non-blocking channel ops: TrySend (always succeeds on an unbounded channel — Bool(true)),
            // TryRecv (value-or-Nothing), Close (a reversible closed flag). All reversed by the
            // scheduler from the recorded resume + channel delta.
            | Op::ChanTrySend { .. }
            | Op::ChanTryRecv { .. }
            | Op::ChanClose { .. }
            // Select — the arm-registration ops accumulate `select_pending` (writing nothing on the
            // tape); `SelectWait` suspends, and the scheduler undoes the winning arm's value + index
            // deliveries and any channel dequeue from the recorded schedule.
            | Op::SelectArmRecv { .. }
            | Op::SelectArmTimeout { .. }
            | Op::SelectWait { .. }
    ) || matches!(op, Op::CallBuiltin { builtin, .. } if reversible_builtin(*builtin))
        || mutated_collection_reg(op).is_some()
        || overwritten_reg(op).is_some()
}
