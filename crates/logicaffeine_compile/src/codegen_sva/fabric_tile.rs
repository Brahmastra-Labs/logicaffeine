//! fabric_tile — the SINGLE SOURCE OF TRUTH for "which ISA ops the spatial fabric realizes".
//!
//! The soft-core interpreter runs *every* op (it is the universal fallback). The spatial fabric — the
//! ASIC-perf datapath — realizes a growing SUBSET: the campaign lowers the ISA onto tiles category by
//! category ("the interpreter itself becomes the fabric"). [`fabric_tile`] classifies each [`Op`] as the
//! tile it lowers to, or `None` when it still falls back to the interpreter. This is the fabric analog of
//! `isa::silicon_realized`: the coverage ratchet (`fabric_coverage_lock.rs`) keys off it, and — crucially —
//! every op it marks realized must ship an `eval == VM` equivalence witness, so a coverage claim can never
//! outrun its proof.
//!
//! Tile kinds grow as coverage grows (bitwise/shift, the shared state plane for collections, floats, CRDT,
//! concurrency, calls). Today the realized set is the arithmetic/comparison datapath plus the plumbing
//! (Move/LoadConst) and control (Return) the fabric framework wires around it.

use super::fabric_rtl::{AluOp, CmpKind, FuncUnit};
use super::fabric_target::{Capability, FabricTarget};
use crate::vm::Op;

/// The fabric tile an [`Op`] lowers to. A *compute* tile (`Alu`/`Cmp`/`Func`) has independently-fuzzable
/// datapath semantics and MUST carry an `eval == VM` witness in the coverage lock; a *structural* tile
/// (`Wire`/`Control`) is realized by the fabric framework itself (a route, a const, a controller edge) and
/// is exercised by every fabric, so it carries no standalone fuzz witness.
#[derive(Clone, Copy, Debug)]
pub enum FabricTile {
    /// Combinational arithmetic — an `AluOp` tile (`alu.v`).
    Alu(AluOp),
    /// Combinational comparison — folds into a `Select` predicate tile.
    Cmp(CmpKind),
    /// Sequential functional unit — the `shared_divider` (division/remainder), start/done handshake.
    Func(FuncUnit),
    /// Memory read — a scratchpad/heap read tile (the induction counter drives the address bus). Today:
    /// `IterNext`'s element read `xs[iv]` in a sequence loop (proven by `fabric_mem_lock`).
    Mem,
    /// Plumbing — `Move`/`LoadConst`: realized as a wire / a baked constant, no compute tile.
    Wire,
    /// Control — `Return`/`ReturnNothing`: realized by the CFG controller (the fabric's result edge).
    Control,
}

impl FabricTile {
    /// A compute tile has independently-fuzzable datapath semantics (so it needs a witness); structural
    /// tiles (`Wire`/`Control`) are framework plumbing (exercised by every fabric, no standalone witness).
    pub fn is_compute(&self) -> bool {
        matches!(
            self,
            FabricTile::Alu(_) | FabricTile::Cmp(_) | FabricTile::Func(_) | FabricTile::Mem
        )
    }

    /// The capability a described target must provide to HOST this tile: the ALU op, the functional unit,
    /// the predicated mux a comparison folds into, the state plane for a memory tile. A structural tile
    /// (`Wire`/`Control`) is the fabric framework's own and needs nothing from the descriptor.
    pub fn capability(&self) -> Option<Capability> {
        match self {
            FabricTile::Alu(op) => Some(Capability::Alu(*op)),
            FabricTile::Func(unit) => Some(Capability::Func(*unit)),
            FabricTile::Cmp(_) => Some(Capability::Select),
            FabricTile::Mem => Some(Capability::Memory),
            FabricTile::Wire | FabricTile::Control => None,
        }
    }
}

/// THE authority: the fabric tile this op lowers to, or `None` when it still falls back to the interpreter.
/// As the campaign lowers each op category, its arm moves from `None` into a `Some(tile)` here — and the
/// coverage lock then demands its equivalence witness before the floor can rise.
pub fn fabric_tile(op: &Op) -> Option<FabricTile> {
    Some(match op {
        // ── combinational arithmetic (alu.v) ──
        Op::Add { .. } => FabricTile::Alu(AluOp::Add),
        Op::AddAssign { .. } => FabricTile::Alu(AluOp::Add), // `x += y` — the same adder, in place
        // Reversible ALU (`rev_alu_core` ISA — no source emits them, so hand-bytecode witnessed against the
        // VM oracle): the same combinational tiles as their irreversible twins, updating `place` in place.
        Op::RevXor { .. } => FabricTile::Alu(AluOp::Xor),
        Op::RevAdd { .. } => FabricTile::Alu(AluOp::Add),
        Op::RevSub { .. } => FabricTile::Alu(AluOp::Sub),
        Op::RevNeg { .. } => FabricTile::Alu(AluOp::Sub), // `p = -p` = `0 - p`
        Op::Sub { .. } => FabricTile::Alu(AluOp::Sub),
        Op::Mul { .. } => FabricTile::Alu(AluOp::Mul),
        // ── bitwise / shift (combinational, alu.v) ──
        Op::BitAnd { .. } => FabricTile::Alu(AluOp::And),
        Op::BitOr { .. } => FabricTile::Alu(AluOp::Or),
        Op::BitXor { .. } => FabricTile::Alu(AluOp::Xor),
        Op::Shl { .. } => FabricTile::Alu(AluOp::Shl),
        Op::Shr { .. } => FabricTile::Alu(AluOp::Shr),
        // ── sequential division (shared_divider), b==0 → 0 ──
        Op::Div { .. } => FabricTile::Func(FuncUnit::DivS),
        Op::Mod { .. } => FabricTile::Func(FuncUnit::ModS),
        Op::FloorDiv { .. } => FabricTile::Func(FuncUnit::FloorDivS),
        // `ExactDiv` (exact-operand contract) and `DivPow2` (`lhs / 2^k`) both reduce to the same signed
        // truncating divider — the Oracle only emits them where that equals the intended result. No source
        // emits them on the no-oracle fabric path, so they are hand-bytecode witnessed.
        Op::ExactDiv { .. } | Op::DivPow2 { .. } => FabricTile::Func(FuncUnit::DivS),
        // `MagicDivU` — division/modulo by a constant via the magic reciprocal; the characteristic unit is
        // the unsigned high-half multiply (`MulHiU`), reduced by shifts/add. Hand-bytecode witnessed.
        Op::MagicDivU { .. } => FabricTile::Func(FuncUnit::MulHiU),
        // ── exponentiation: the sequential power unit (a `b`-step multiply FSM) ──
        Op::Pow { .. } => FabricTile::Func(FuncUnit::Pow),
        // `is approximately` — the numeric approximate-equality functional unit (f64 tolerance). Float
        // operands are refused at the Float load, so only the numeric-int case is realized (witnessed).
        Op::ApproxEq { .. } => FabricTile::Func(FuncUnit::ApproxEqF),
        // `BigIntWide` — the 256-bit BigInt lane. FromInt/Add/Mul/Low are realized with an overflow-tracked
        // `big` flag (characteristic unit = the signed-overflow test); Sub/Cmp/Div are refused. Witnessed.
        Op::BigIntWide { .. } => FabricTile::Func(FuncUnit::MulOvfS),
        // ── combinational comparison → predicate/select ──
        Op::Lt { .. } => FabricTile::Cmp(CmpKind::Lt),
        Op::Gt { .. } => FabricTile::Cmp(CmpKind::Gt),
        Op::LtEq { .. } => FabricTile::Cmp(CmpKind::Le),
        Op::GtEq { .. } => FabricTile::Cmp(CmpKind::Ge),
        Op::Eq { .. } => FabricTile::Cmp(CmpKind::Eq),
        Op::NotEq { .. } => FabricTile::Cmp(CmpKind::Ne),
        // logical not `(src == 0) ? 1 : 0` — a Select driven by an equality-to-zero comparison
        Op::Not { .. } => FabricTile::Cmp(CmpKind::Eq),
        // ── sequence iteration (loop fabric): the element read + the loop-setup scaffolding ──
        Op::IterNext { .. } => FabricTile::Mem,
        // `IterPop` — pop the iterator at a loop's end; the explicit-loop-index model makes it a vacuous
        // structural no-op (the loop-fabric extractor already treats it so).
        Op::NewRange { .. } | Op::IterPrepare { .. } | Op::IterPop => FabricTile::Control,
        // ── state-plane READS (the shared heap): collection length + 1-based element index ──
        // `IndexUnchecked` (the Oracle-proven-in-bounds form) shares the read tile — no source emits it on
        // the no-oracle fabric compile path, so it is hand-bytecode witnessed (identical to `Index` in range).
        Op::Length { .. } | Op::Index { .. } | Op::IndexUnchecked { .. } => FabricTile::Mem,
        // ── state-plane ALLOC + WRITE: build a fresh list, set an element in place, grow via push ──
        // `SetIndexUnchecked` shares the in-place write tile (hand-bytecode witnessed, like `IndexUnchecked`).
        Op::NewList { .. }
        | Op::SetIndex { .. }
        | Op::SetIndexUnchecked { .. }
        | Op::ListPush { .. } => FabricTile::Mem,
        // `NewEmptyListI32` (the narrowing-proven half-width empty `Seq of Int`) shares the length-0 heap
        // region — no source emits it on the no-oracle fabric path, so it is hand-bytecode witnessed.
        Op::NewEmptyList { .. }
        | Op::NewEmptyListWithCapacity { .. }
        | Op::NewEmptyListFixed { .. }
        | Op::NewEmptyListI32 { .. }
        | Op::ListPop { .. } => FabricTile::Mem,
        // an empty set / map — the same length-0 heap region (structure only matters once elements are added)
        Op::NewEmptySet { .. } | Op::NewEmptyMap { .. } | Op::NewEmptyMapWithCapacity { .. } => {
            FabricTile::Mem
        }
        // `Add v to s` — dedup append: a runtime-count-guarded membership scan then a conditional write
        Op::SetAdd { .. } => FabricTile::Mem,
        // `Remove v from s` — compacting removal: a running `past` flag shifts every later slot down by one
        Op::RemoveFrom { .. } => FabricTile::Mem,
        // `s union t` — a fresh set: copy lhs, then dedup-append each rhs element not already present
        Op::UnionOp { .. } => FabricTile::Mem,
        // `s intersection t` — a fresh set: lhs elements filtered by membership in rhs (nested scan)
        Op::IntersectOp { .. } => FabricTile::Mem,
        // `Concat` (string interpolation) — render each operand to a char-list (int→decimal) and concatenate
        Op::Concat { .. } => FabricTile::Mem,
        // `FormatValue` — render a value to TEXT (default case: int→decimal char-list, or a Text identity).
        // Format-specs are a follow-on; hand-bytecode witnessed on the default rendering.
        Op::FormatValue { .. } => FabricTile::Mem,
        // channel DATA ops — a heap FIFO buffer (create / enqueue / dequeue) for sequential (spawn-free) use;
        // ChanTrySend always succeeds on the depth-bounded FIFO (enqueue + true)
        Op::ChanNew { .. }
        | Op::ChanSend { .. }
        | Op::ChanRecv { .. }
        | Op::ChanTrySend { .. }
        | Op::ChanTryRecv { .. } => FabricTile::Mem,
        // `ChanClose` — output-transparent on the deterministic scheduler (buffered values stay receivable),
        // so a structural no-op on the combinational fabric. Hand-bytecode differential vs the RevCoop oracle.
        Op::ChanClose { .. } => FabricTile::Control,
        // `select` — the arm-registration ops (`SelectArmRecv`/`SelectArmTimeout`) record pending state (no
        // cell → Control); `SelectWait` is the PRIORITY resolver — a bounded fan of channel-readiness reads +
        // a guarded dequeue of the winner (a memory tile), proven by a `select` corpus program ≡ the VM.
        Op::SelectArmRecv { .. } | Op::SelectArmTimeout { .. } => FabricTile::Control,
        Op::SelectWait { .. } => FabricTile::Mem,
        // `contains` — a bounded, unrolled OR-scan over the collection's static length (heap reads + equality)
        Op::Contains { .. } => FabricTile::Mem,
        // `copy of xs` — a deep clone: unrolled element copy into a fresh heap region (value semantics)
        Op::DeepClone { .. } => FabricTile::Mem,
        // `EnsureOwned` — the call-site COW barrier: clone a possibly-aliased collection before a mutable-
        // borrow callee writes it (always-clone = observably identical to the VM's clone-iff-shared). Mem.
        Op::EnsureOwned { .. } => FabricTile::Mem,
        // `xs followed by ys` — sequence concatenation: two unrolled copies into a fresh region
        Op::SeqConcat { .. } => FabricTile::Mem,
        // `items s through e of xs` — a slice: unrolled subrange copy (const bounds) into a fresh region
        Op::SliceOp { .. } => FabricTile::Mem,
        // `(a, b, c)` — a tuple shares the list heap shape `[count, e0..]`; indexed by `item i of t`
        Op::NewTuple { .. } => FabricTile::Mem,
        // `Let (x, y, ..) be t` — read each tuple component `heap[ptr+1+j]` into a register (a fan of reads).
        // Source emits it only in a `Repeat for (x,y) in ..` loop, so it is hand-bytecode witnessed.
        Op::DestructureTuple { .. } => FabricTile::Mem,
        // `Args` — the command-line list, EMPTY on a spatial circuit (no CLI); a length-0 heap region.
        Op::Args { .. } => FabricTile::Mem,
        // ── structs on the shared state plane `[n, f0..f_{n-1}]` — alloc + field read/write. `GetField`/
        // `StructInsert`'s field NAME resolves to a slot via register TYPE-TRACKING (REG_TYPE set at
        // NewStruct → STRUCT_FIELD_POS), the sound mechanism that also unlocks floats.
        Op::NewStruct { .. } | Op::GetField { .. } | Op::StructInsert { .. } => FabricTile::Mem,
        // `Increase/Decrease s's field by n` — a CRDT counter bump: a struct-field read-add-write
        Op::CrdtBump { .. } => FabricTile::Mem,
        // `Resolve s's field to v` — resolve a plain field: a struct-field overwrite (value semantics)
        Op::CrdtResolve { .. } => FabricTile::Mem,
        // `Append v to xs` — for a list seq, exactly a heap push (grow the FIFO/list in place)
        Op::CrdtAppend { .. } => FabricTile::Mem,
        // `Merge source into target` — per-field add over a plain-Int `Shared` struct's fields
        Op::CrdtMerge { .. } => FabricTile::Mem,
        // `CrdtJoin` — per-field MAX (the idempotent G-Counter lattice join). No source emits it, so it is
        // hand-bytecode witnessed: the same struct heap walk as CrdtMerge, with a `max` Select per field.
        Op::CrdtJoin { .. } => FabricTile::Mem,
        // `new counter` / `new growing set` — create a fresh CRDT value. The fabric models the deterministic
        // initial state (a counter is 0; the drawn replica id is unobservable single-replica). Witnessed by a
        // hand-bytecode counter/PN-counter Show ≡ VM; the collection kinds (set/seq/register) are a follow-on.
        Op::NewCrdt { .. } => FabricTile::Mem,
        // `Push v to s's field` — read the field's list pointer, then push (nested list in a struct)
        Op::ListPushField { .. } => FabricTile::Mem,
        // ── enums / pattern matching on the same heap plane `[tag, arg0..]`. NewInductive/BindArm are heap
        // alloc + payload read; TestArm is a tag comparison that folds into the match's `Select` predicate.
        Op::NewInductive { .. } | Op::BindArm { .. } => FabricTile::Mem,
        Op::TestArm { .. } => FabricTile::Cmp(CmpKind::Eq),
        // A promoted GLOBAL (a module-level `Let` used inside a function) is a pure dataflow WIRE in the
        // straight-line fabric: `GlobalSet` remembers the definition's wire, `GlobalGet` re-wires it into the
        // reader. No cell — set-before-use makes it plumbing, exactly like `Move` (proven by a whole-program
        // differential that reads a global from an inlined callee ≡ the VM).
        Op::GlobalGet { .. } | Op::GlobalSet { .. } => FabricTile::Wire,
        // ── plumbing / control the fabric framework realizes ──
        Op::Move { .. } | Op::LoadConst { .. } => FabricTile::Wire,
        // `now` / `today` — a clock value-load: the fabric reads the injected fixed clock and bakes the
        // instant as a Const (structurally a value-load, like LoadConst). Proven by a differential where a
        // date-component extractor (`the hour of now`, `the weekday of today`) renders a plain Int == the VM.
        Op::LoadNow { .. } | Op::LoadToday { .. } => FabricTile::Wire,
        // `Swap a and b` — a reversible register exchange, realized as pure rewiring (no cell)
        Op::RevSwap { .. } => FabricTile::Wire,
        Op::Return { .. } | Op::ReturnNothing => FabricTile::Control,
        // `Show v` — the whole-program output edge: the fabric framework appends the value to the output
        // tape (proven by the whole-program differential, `eval_whole_program == VM stdout`).
        Op::Show { .. } => FabricTile::Control,
        // `Sleep n` — a temporal delay with no data effect. The combinational fabric is clockless, so the
        // deterministic execution model makes it a vacuous no-op (proven by a differential vs the concurrent
        // VM, whose output is identical with or without the sleep). A structural control edge.
        Op::Sleep { .. } => FabricTile::Control,
        // `FailWith` — raise an error; the VM shows nothing after, so the fabric HALTS its output tape at
        // that point (a control edge). Proven by a differential vs the VM's output up to the failure.
        Op::FailWith { .. } => FabricTile::Control,
        // `CheckPolicy` — a SECURITY guard; a granted check is transparent, a denied one makes the VM return
        // `Err` (no output after). The fabric compiles the condition to a per-`Check` GRANT predicate and
        // CONDITIONALLY halts the output tape (a control edge, not a data tile). A no-op here would be a
        // security hole, so it is proven by a grant-AND-deny differential vs the VM's output-up-to-the-error.
        Op::CheckPolicy { .. } => FabricTile::Control,
        // `Obs` — an observability event feeding a separate trace sink (off by default), never stdout; it has
        // no effect on the program's output, so a structural no-op. Differential vs the VM (output unchanged).
        Op::Obs { .. } => FabricTile::Control,
        // `RegionBoundsGuard` — an Oracle bounds-check assertion with no data effect (VM: `pc += 1`), so a
        // structural control edge. No source emits it on the fabric path; exercised by a hand-bytecode test.
        Op::RegionBoundsGuard { .. } => FabricTile::Control,
        // OFFLINE networking (single node, no relay) — the deterministic model the AOT proves (tw==VM==AOT):
        // Connect/Listen/Send/Stream have no observable effect and `NetSync` is offline-identity, so all are
        // structural control no-ops; a `vm_outcome_net` differential exercises them (never "realized by
        // ignoring"). `NetMakePeer` wires an opaque handle from its address — structural plumbing.
        // `NetListen`/`NetSend`/`NetAwait` form the offline loopback (Listen establishes a FIFO inbox, Send
        // publishes, Await drains) — the deterministic single-node round-trip; Connect/Stream/Sync are pure
        // no-ops/offline-identity. All structural, proven together by the `vm_outcome_net` differential.
        Op::NetConnect { .. }
        | Op::NetListen { .. }
        | Op::NetSend { .. }
        | Op::NetStream { .. }
        | Op::NetSync { .. }
        | Op::NetAwait { .. } => FabricTile::Control,
        Op::NetMakePeer { .. } => FabricTile::Wire,
        // `Halt` — the program-end edge: every `Main` terminates on it, and the whole-program builder ends
        // the fabric there (exercised by every whole-program differential).
        Op::Halt => FabricTile::Control,
        // A direct call is realized by INLINING the callee's decision fabric into the caller (Futamura) —
        // the call site is a structural composition edge; the callee's own ops are the compute tiles.
        Op::Call { .. } => FabricTile::Control,
        // `CallBuiltin` — the numeric builtins: the float lane (`Sqrt`/`Floor` on `f64` bit patterns) plus
        // integer `Abs`/`Min`/`Max` selects. A compute tile (characteristic unit = the float floor bridge);
        // its witness exercises both the float path and the int selects. IO/DOM/format builtins are refused.
        Op::CallBuiltin { .. } => FabricTile::Func(FuncUnit::FFloorF),
        // CLOSURES: `MakeClosure` captures its free variables into a tracked closure value (dataflow, no
        // cell → Wire); `CallValue` resolves the closure's STATIC body and inlines it (a composition edge
        // like `Call` → Control). Dynamic dispatch realized as a Futamura projection; hand-bytecode witnessed.
        Op::MakeClosure { .. } => FabricTile::Wire,
        Op::CallValue { .. } => FabricTile::Control,
        // The SCHEDULER (spatial coroutine): `Spawn`/`SpawnHandle` inline a producer child's side effects at
        // the spawn point (the child's ops are the compute tiles); `TaskAwait` joins instantly (child already
        // ran) and `TaskAbort` is a no-op on a complete child — all structural composition edges, witnessed
        // through the cooperative scheduler oracle (the full scheduler for `TaskAbort`, RevCoop for the rest).
        Op::Spawn { .. } | Op::SpawnHandle { .. } | Op::TaskAwait { .. } | Op::TaskAbort { .. } => {
            FabricTile::Control
        }
        // Branch control: the decision fabric if-CONVERTS `JumpIfFalse` (its predicate → a Select) and
        // consumes the merge `Jump`; both are exercised by every comparison witness's `If`. `JumpIfTrue` is
        // the short-circuit `or`/`and` mirror — it materialises a Bool temp the same way, if-converted too.
        Op::Jump { .. } | Op::JumpIfFalse { .. } | Op::JumpIfTrue { .. } => FabricTile::Control,
        // ── the interpreter-fallback frontier (grows smaller each campaign phase) ──
        _ => return None,
    })
}

/// The mnemonics of every op the fabric realizes today — the ground-truth "covered" set for the ratchet.
pub fn fabric_realized_ops() -> Vec<&'static str> {
    Op::all_variants()
        .iter()
        .filter(|o| fabric_tile(o).is_some())
        .map(|o| o.name())
        .collect()
}

/// The realized ops a described target's envelope HOSTS: every op whose tile's capability the descriptor
/// provides (a structural tile is hosted everywhere). One descriptor's column of the coverage ratchet —
/// `rdu_op_coverage_lock` pins it for the SN50 exactly, so a capability lost on the descriptor is as red
/// as a tile lost on the fabric.
pub fn hosted_ops(target: &FabricTarget) -> Vec<&'static str> {
    Op::all_variants()
        .iter()
        .filter(|o| {
            fabric_tile(o).is_some_and(|t| t.capability().map_or(true, |cap| target.provides(cap)))
        })
        .map(|o| o.name())
        .collect()
}

/// Why the fabric has no realization of an op on the interpreter-fallback frontier — `None` for every
/// realized op, and `None` for a frontier op nobody has explained, which the coverage locks refuse: an op
/// is tiled or it states its reason, there is no third state. A lane that meets one of these ops refuses
/// the program with this sentence, never a silently dropped op.
pub fn frontier_reason(op: &Op) -> Option<&'static str> {
    if fabric_tile(op).is_some() {
        return None;
    }
    match op {
        Op::Tcp { .. } => Some(
            "Tcp: real host sockets (the tcp.lg client natives) have no spatial-fabric realization and no offline model yet — graduating it needs a sealed-world tcp model, never a silent no-op tile",
        ),
        Op::Serve { .. } => Some("Serve: the HTTP accept loop is host IO (bind/accept/read/write) with no spatial-fabric realization"),
        _ => None,
    }
}

/// The fraction of the ISA the fabric datapath realizes (the rest runs on the interpreter fallback).
/// Ratchets toward 1.0 as the campaign lowers each op category.
pub fn fabric_coverage() -> f64 {
    let total = Op::all_variants().len();
    fabric_realized_ops().len() as f64 / total as f64
}
