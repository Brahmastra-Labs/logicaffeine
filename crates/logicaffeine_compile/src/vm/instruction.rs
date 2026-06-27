//! Bytecode instruction set and the compiled-program container.
//!
//! Registers are per-frame `u16` indices (the compiler assigns locals to
//! registers at compile time; frames are capped at `MAX_REGISTERS_PER_FRAME`).
//! Jump targets are absolute instruction indices. This is the growing core of
//! VM_PLAN.md's 87-opcode set.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::intern::Symbol;

/// Serialize an interned [`Symbol`] as its `u32` index (HOTSWAP §P12). Sound for the
/// tier cache because the cache key pins the exact source — re-parsing it reproduces
/// the same interning order, so the index round-trips to the same symbol.
pub(crate) mod symbol_serde {
    use crate::intern::Symbol;
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(s: &Symbol, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_u32(s.index() as u32)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Symbol, D::Error> {
        Ok(Symbol::from_index(u32::deserialize(de)? as usize))
    }
}

/// Serialize an `f64` by its raw bits so the cache round-trips bit-exactly (a text
/// format would otherwise mangle NaN / ±∞ and risk precision drift).
pub(crate) mod f64_bits {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &f64, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_u64(v.to_bits())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<f64, D::Error> {
        Ok(f64::from_bits(u64::deserialize(de)?))
    }
}

pub type Reg = u16;
pub type ConstIdx = u32;
pub type FuncIdx = u16;

/// A constant-pool entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Constant {
    Int(i64),
    Float(#[serde(with = "f64_bits")] f64),
    Bool(bool),
    Text(String),
    Char(char),
    Nothing,
    Duration(i64),
    Date(i32),
    Moment(i64),
    Span { months: i32, days: i32 },
    Time(i64),
}

/// A bytecode instruction. Every field is a small `Copy` scalar (registers,
/// constant-pool indices, interned symbols), so the dispatch loop reads each
/// op by value instead of `clone()`-ing through the `Clone` machinery.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Op {
    /// `R[dst] = constants[idx]`
    LoadConst { dst: Reg, idx: ConstIdx },
    /// `R[dst] = R[src]` (shallow clone)
    Move { dst: Reg, src: Reg },

    Add { dst: Reg, lhs: Reg, rhs: Reg },
    /// `R[dst] = R[dst] + R[src]` — the `Set x to x + …` shape. Semantically
    /// identical to `Add { dst, lhs: dst, rhs: src }`; the dedicated form lets
    /// the VM append in place when `R[dst]` is a sole-owner Text (turning the
    /// O(n²) build-a-string-by-concatenation loop into amortized O(n)).
    AddAssign { dst: Reg, src: Reg },
    Sub { dst: Reg, lhs: Reg, rhs: Reg },
    Mul { dst: Reg, lhs: Reg, rhs: Reg },
    Div { dst: Reg, lhs: Reg, rhs: Reg },
    /// EXACT division (`7 / 2 → 7/2`, a Rational), the type-directed sibling of
    /// [`Op::Div`] — emitted for `BinaryOpKind::ExactDivide`.
    ExactDiv { dst: Reg, lhs: Reg, rhs: Reg },
    Mod { dst: Reg, lhs: Reg, rhs: Reg },
    /// `dst = lhs / 2^k` (signed, round toward zero) — emitted only when the
    /// divisor is a literal power of two AND the Oracle proved `lhs` is `Int`.
    /// A single op (the JIT lowers it to the side-exit-free `divpow2` shift
    /// stencil) so it fires for loop-invariant divisors the in-region JIT
    /// detector misses, without the scratch-register pressure of an expansion.
    DivPow2 { dst: Reg, lhs: Reg, k: u8 },
    /// `dst = lhs / c` (`mul_back == 0`) or `dst = lhs % c` (`mul_back == c`),
    /// where `c` is a compile-time-constant divisor that is NOT a power of two
    /// (W5/`DivPow2` handles powers of two), computed by the Granlund–Montgomery
    /// / libdivide UNSIGNED magic-reciprocal sequence (a `mul`-high + shift,
    /// ~3 cycles) instead of `idiv` (~25 cycles). `magic`/`more` are the
    /// precomputed constants (the exact [`logicaffeine_data::LogosDivU64`]
    /// encoding — low 6 bits of `more` are the shift, `0x40` is the 65-bit
    /// add-marker path). Emitted ONLY when the Oracle proves `lhs` is `Int` and
    /// NON-NEGATIVE: the unsigned magic equals the signed truncating `/`/`%`
    /// only for a non-negative dividend (for `x < 0` the signs disagree, exactly
    /// as for the `% 2^k → &` rewrite). The remainder is derived as
    /// `lhs - q*c` (wrapping), bit-exact with the kernel's `wrapping_rem` for
    /// non-negative `lhs`.
    MagicDivU { dst: Reg, lhs: Reg, magic: u64, more: u8, mul_back: i64 },

    Lt { dst: Reg, lhs: Reg, rhs: Reg },
    Gt { dst: Reg, lhs: Reg, rhs: Reg },
    LtEq { dst: Reg, lhs: Reg, rhs: Reg },
    GtEq { dst: Reg, lhs: Reg, rhs: Reg },
    Eq { dst: Reg, lhs: Reg, rhs: Reg },
    NotEq { dst: Reg, lhs: Reg, rhs: Reg },

    Not { dst: Reg, src: Reg },

    /// Eager `and`: bitwise for Int×Int, truthiness otherwise. The compiler
    /// arranges short-circuit evaluation with jumps; this op only fires when
    /// the right operand must be evaluated.
    AndEager { dst: Reg, lhs: Reg, rhs: Reg },
    /// Eager `or` (see `AndEager`).
    OrEager { dst: Reg, lhs: Reg, rhs: Reg },
    Concat { dst: Reg, lhs: Reg, rhs: Reg },
    BitXor { dst: Reg, lhs: Reg, rhs: Reg },
    Shl { dst: Reg, lhs: Reg, rhs: Reg },
    Shr { dst: Reg, lhs: Reg, rhs: Reg },

    /// Unconditional jump to absolute instruction index.
    Jump { target: usize },
    /// Jump if `R[cond]` is falsey.
    JumpIfFalse { cond: Reg, target: usize },
    /// Jump if `R[cond]` is truthy.
    JumpIfTrue { cond: Reg, target: usize },
    /// Jump if `R[cond]` is an Int (drives the `and`/`or` eager-vs-short-circuit
    /// split: Int operands always evaluate both sides, bitwise).
    JumpIfInt { cond: Reg, target: usize },

    /// Call a user function. The caller has placed `arg_count` arguments in
    /// consecutive registers starting at `args_start` (relative to the caller's
    /// frame base). The result is written to `R[dst]`. Uses register windowing.
    Call { dst: Reg, func: FuncIdx, args_start: Reg, arg_count: u16 },
    /// Call a kernel builtin with already-evaluated arguments (arity was
    /// validated at compile time, mirroring the tree-walker's
    /// arity-before-evaluation rule).
    CallBuiltin {
        dst: Reg,
        builtin: crate::semantics::builtins::BuiltinId,
        args_start: Reg,
        arg_count: u16,
    },
    /// Call the closure value in `R[callee]`. `name_for_err` is the function
    /// name when this came from a by-name call (`Unknown function: f` when the
    /// value is not callable) or `u32::MAX` for a call-by-expression
    /// (`Cannot call value of type T`).
    CallValue { dst: Reg, callee: Reg, args_start: Reg, arg_count: u16, name_for_err: ConstIdx },
    /// Build a closure over `program.functions[func]`: its `captures` list is
    /// snapshotted — local captures deep-cloned from the register window at
    /// `locals_start`, global captures from the globals table (skipped when
    /// still undefined; the body then falls through to the live global).
    MakeClosure { dst: Reg, func: FuncIdx, locals_start: Reg },

    /// `Check subject can/is predicate (of object)` — kernel policy check.
    /// `object == Reg::MAX` means no object.
    CheckPolicy {
        subject: Reg,
        #[serde(with = "symbol_serde")]
        predicate: Symbol,
        is_capability: bool,
        object: Reg,
        source_text: ConstIdx,
    },

    /// `Push value to obj's field` — kernel push into a struct's List field.
    /// `field` is a Text constant (the resolved field name).
    ListPushField { obj: Reg, field: ConstIdx, src: Reg },

    // ---- Globals (Main top-level bindings visible inside functions) ----
    /// `R[dst] = globals[idx]`, error "Undefined variable: {name}" when unset.
    GlobalGet { dst: Reg, idx: u16 },
    /// `globals[idx] = R[src]` (defines or overwrites).
    GlobalSet { idx: u16, src: Reg },
    /// Return `R[src]` from the current function.
    Return { src: Reg },
    /// Return `nothing` from the current function.
    ReturnNothing,

    // ---- Collections ----
    /// `R[dst] = [R[start], …, R[start+count-1]]` (a new list).
    NewList { dst: Reg, start: Reg, count: u16 },
    NewEmptyList { dst: Reg },
    /// `R[dst] = a new half-width (`Vec<i32>`) Int sequence` — emitted (behind
    /// `LOGOS_NARROW_VM`) for a `new Seq of Int` declaration the narrowing
    /// proof certified fits `i32`. Observably identical to `NewEmptyList`; only
    /// the storage width differs (see [`crate::interpreter::ListRepr::IntsI32`]).
    NewEmptyListI32 { dst: Reg },
    NewEmptySet { dst: Reg },
    NewEmptyMap { dst: Reg },
    /// `R[dst] = [R[start]..=R[end]]` (inclusive integer range as a list).
    NewRange { dst: Reg, start: Reg, end: Reg },
    /// Append `R[value]` to the list in `R[list]` (mutates in place).
    ListPush { list: Reg, value: Reg },
    /// Add `R[value]` to the set in `R[set]` (no-op if already present).
    SetAdd { set: Reg, value: Reg },
    /// Remove `R[value]` from the set/map in `R[collection]`.
    RemoveFrom { collection: Reg, value: Reg },
    /// `R[collection][R[index]] = R[value]` (1-based list set, or map insert).
    SetIndex { collection: Reg, index: Reg, value: Reg },
    /// Like [`SetIndex`] but the Oracle PROVED the index in `[1, length]`
    /// (range analysis, M9) — bounds-check elimination for the STORE. The
    /// interpreter still checks (free defense-in-depth); the JIT lowers it to
    /// an UNCHECKED array store (no bounds branch). Only listy collections
    /// with a stable length earn this, via `index_provably_in_bounds`.
    SetIndexUnchecked { collection: Reg, index: Reg, value: Reg },
    /// `R[dst] = R[collection][R[index]]` (1-based for ordered collections).
    Index { dst: Reg, collection: Reg, index: Reg },
    /// Like [`Index`] but the Oracle (range analysis, M9) PROVED the index
    /// in `[1, length]` at this point — bounds-check elimination, the V8/LLVM
    /// way. The bytecode interpreter still checks (a sound proof makes the
    /// check never fire; keeping it is free defense-in-depth), but the JIT
    /// lowers it to an UNCHECKED array load (no bounds branch, no deopt).
    /// Only listy collections with a stable length earn this; the compiler
    /// emits it solely behind `index_provably_in_bounds`.
    IndexUnchecked { dst: Reg, collection: Reg, index: Reg },
    /// REGION-ENTRY bounds-check hoist (V8 TurboFan loop bound-check
    /// elimination). For a loop `while iv </<= bound` reading/writing
    /// `R[array]` at affine indices, this asserts ONCE — at native region
    /// entry — that the array is long enough for the whole loop:
    /// `length(R[array]) >= R[bound] + add_max` and `R[iv] + add_min >= 1`.
    /// If it holds, the region runs every covered access UNCHECKED; if not,
    /// the VM declines the region and replays on bytecode (where the accesses
    /// are checked). A pure no-op in the interpreter and the function tier —
    /// speculation is region-only, made safe solely by this entry guard.
    RegionBoundsGuard { array: Reg, bound: Reg, iv: Reg, add_max: i32, add_min: i32 },
    /// `R[dst] = length of R[collection]`.
    Length { dst: Reg, collection: Reg },
    /// `R[dst] = R[collection] contains R[value]`.
    Contains { dst: Reg, collection: Reg, value: Reg },

    // ---- Strings / slices / tuples / sets / temporal ----
    /// `R[dst] = Text(debug_prefix? + format(R[src], spec?))` — one
    /// interpolated-string segment. `u32::MAX` = no spec / no prefix.
    FormatValue { dst: Reg, src: Reg, spec: ConstIdx, debug_prefix: ConstIdx },
    /// `R[dst] = R[collection][R[start]..=R[end]]` (1-indexed inclusive).
    SliceOp { dst: Reg, collection: Reg, start: Reg, end: Reg },
    /// `R[dst] = deep clone of R[src]`.
    DeepClone { dst: Reg, src: Reg },
    /// `R[dst] = (R[start], …, R[start+count-1])` (immutable tuple).
    NewTuple { dst: Reg, start: Reg, count: u16 },
    UnionOp { dst: Reg, lhs: Reg, rhs: Reg },
    IntersectOp { dst: Reg, lhs: Reg, rhs: Reg },
    /// `R[dst] = today` (Date; honors the test fixed-clock).
    LoadToday { dst: Reg },
    /// `R[dst] = now` (Moment; honors the test fixed-clock).
    LoadNow { dst: Reg },

    // ---- Structs / enums / Inspect / CRDT ----
    /// `R[dst] = Struct { type_name: constants[type_name], fields: {} }`.
    NewStruct { dst: Reg, type_name: ConstIdx },
    /// Insert `R[value]` under field `constants[field]` into the struct in
    /// `R[obj]` (construction and SetField — structs have VALUE semantics, so
    /// in-place mutation of the register equals the tree-walker's
    /// clone-mutate-reassign).
    StructInsert { obj: Reg, field: ConstIdx, value: Reg },
    /// `R[dst] = R[obj].field` — "Field '{f}' not found" / "Cannot access
    /// field on {type}".
    GetField { dst: Reg, obj: Reg, field: ConstIdx },
    /// `R[dst] = Inductive { type, constructor, args: R[args_start..+count] }`.
    NewInductive { dst: Reg, type_name: ConstIdx, ctor: ConstIdx, args_start: Reg, count: u16 },
    /// `R[dst] = Bool(struct type-name or inductive constructor == constants[variant])`;
    /// false for any other value.
    TestArm { dst: Reg, target: Reg, variant: ConstIdx },
    /// Inspect-arm binding: a Struct target binds `fields[constants[field]]`,
    /// an Inductive target binds `args[index]`. A missing field/index leaves
    /// `R[dst]` unwritten (the tree-walker skips the bind — unreachable for
    /// parsed programs, whose fields always exist after default-fill).
    BindArm { dst: Reg, target: Reg, field: ConstIdx, index: u16 },
    /// GCounter/PNCounter bump of `R[obj].field` by `R[amount]` (negated for
    /// Decrease — also selects the tree-walker's increment/decrement wording).
    CrdtBump { obj: Reg, field: ConstIdx, amount: Reg, negate: bool },
    /// GCounter merge: fold every field of `R[source]` into `R[target]`.
    CrdtMerge { target: Reg, source: Reg },
    /// `R[dst]` = a fresh, empty rich CRDT — `kind` 0 = SharedSet (OR-Set),
    /// 1 = SharedSequence (RGA), 2 = Divergent (MV-register). Used to default-fill a
    /// `Shared` struct's CRDT fields, mirroring the tree-walker's `new`-struct init.
    NewCrdt { dst: Reg, kind: u8 },
    /// RGA append: push `R[value]` onto the replicated sequence in `R[seq]` (mutates the
    /// shared CRDT in place, so a field access propagates).
    CrdtAppend { seq: Reg, value: Reg },
    /// Resolve `R[obj].field` to `R[value]`: a real MV-register resolves in place, a plain
    /// field is overwritten — the same fallback the tree-walker's `Resolve` takes.
    CrdtResolve { obj: Reg, field: ConstIdx, value: Reg },

    // ---- Repeat (snapshot iteration) ----
    /// Snapshot `R[iterable]` (List/Set items, Text chars, Map (k,v) tuples)
    /// and push it onto the iterator stack.
    IterPrepare { iterable: Reg },
    /// Load the next snapshot element into `R[dst]` and advance; jump to
    /// `exit` when exhausted (the iterator stays pushed — `IterPop` at the
    /// exit point drops it).
    IterNext { dst: Reg, exit: usize },
    /// Drop the top iterator.
    IterPop,
    /// `R[dst] = R[list].pop()` — Nothing when empty (not an error).
    ListPop { list: Reg, dst: Reg },
    /// Sleep for `R[nanos]` (Duration nanos, or Int milliseconds).
    Sleep { duration: Reg },
    /// Tuple-pattern binding: `R[start..start+count] = tuple elements` with
    /// zip semantics (stops at the shorter side, like the tree-walker).
    /// Errors when `R[src]` is not a Tuple.
    DestructureTuple { src: Reg, start: Reg, count: u16 },

    /// Emit `R[src].to_display_string()` to the output stream.
    Show { src: Reg },
    /// `R[dst] = the program argument vector` as a `Seq of Text` (the
    /// interpreter's `args()` system native, mirroring the compiled binary's
    /// `env::args()`: index 0 is the program name). Outside the JIT integer
    /// subset, so the adapters bail on it and it always runs in the VM.
    Args { dst: Reg },
    // ─── Go-like concurrency (Phase 54 / T10) ───────────────────────────────
    // Args travel through register ranges (`Op` is `Copy`). Channel/task handles
    // are ordinary `Value`s (`RuntimeValue::Chan` / `::TaskHandle`). Every op that
    // can block suspends the resumable VM (`run_until_block`) and is serviced by
    // the deterministic scheduler, exactly as the tree-walker's `yield_request`.

    /// `R[dst] = a new channel`; `cap < 0` ⇒ the scheduler's default capacity.
    ChanNew { dst: Reg, cap: i32 },
    /// Send `R[val]` into channel `R[chan]` (blocks if the channel is full).
    ChanSend { chan: Reg, val: Reg },
    /// `R[dst] = receive from channel R[chan]` (blocks if the channel is empty).
    ChanRecv { dst: Reg, chan: Reg },
    /// `R[dst] = bool` — non-blocking send of `R[val]` into `R[chan]`.
    ChanTrySend { dst: Reg, chan: Reg, val: Reg },
    /// `R[dst] = received value, or Nothing` — non-blocking receive from `R[chan]`.
    ChanTryRecv { dst: Reg, chan: Reg },
    /// Close channel `R[chan]`.
    ChanClose { chan: Reg },
    /// Spawn `functions[func]` with args in `R[args_start..+arg_count]` (fire-and-forget).
    Spawn { func: FuncIdx, args_start: Reg, arg_count: u16 },
    /// `R[dst] = task handle` of a spawned `functions[func]` (same arg convention).
    SpawnHandle { dst: Reg, func: FuncIdx, args_start: Reg, arg_count: u16 },
    /// `R[dst] = result of awaiting task R[handle]` (Nothing if it was aborted).
    TaskAwait { dst: Reg, handle: Reg },
    /// Abort task `R[handle]`.
    TaskAbort { handle: Reg },
    /// Register a `Receive var from chan` arm for the next `SelectWait`.
    SelectArmRecv { chan: Reg, var: Reg },
    /// Register an `After ticks` timeout arm for the next `SelectWait`.
    SelectArmTimeout { ticks: Reg },
    /// Block on the registered select arms; `R[dst_arm] = the winning arm index`
    /// (a recv arm's received value is already in its `var` register).
    SelectWait { dst_arm: Reg },

    /// Fail with the Text constant at `msg` — used for constructs whose
    /// tree-walker semantics are "error WHEN EXECUTED" (an unbound `Set`, an
    /// unsupported statement). Never fails at compile time: dead branches must
    /// stay free.
    FailWith { msg: ConstIdx },
    /// Stop execution.
    Halt,
}

/// A compiled user function (or closure body). All bodies share the program's
/// single `code` vector; `entry_pc` is where this one begins. A closure body's
/// frame layout is `[params… , capture values… , capture-present flags…]`.
#[derive(Clone, Debug)]
pub struct CompiledFunction {
    pub name: Symbol,
    pub entry_pc: usize,
    pub param_count: u16,
    pub register_count: usize,
    /// Capture list for a closure body (empty for plain functions), in frame
    /// order. Each entry: the captured name and, when that name is a promoted
    /// global, its global index (the live-fallback source).
    pub captures: Vec<(Symbol, Option<u16>)>,
    /// Which of THIS frame's registers carry a user-visible name (params,
    /// captures, Let targets, loop variables) — the region JIT's
    /// observability map for loops tiering up inside this function.
    pub named_regs: Vec<bool>,
    /// DECLARED parameter kinds: scalars (`x: Float`) ride i64 slots,
    /// `Seq of <scalar>` pins at the boundary; `None` for types native
    /// code cannot represent (Map, Text, nested Seq, …). Closures (no
    /// declarations) default to all-Int — exactly the old entry-guard
    /// contract.
    pub param_kinds: Vec<Option<super::native_tier::ParamKind>>,
    /// Declared return kind; `None` falls back to the adapter's Int/Bool
    /// return inference.
    pub ret_kind: Option<super::native_tier::SlotKind>,
}

/// A compiled program: the constant pool, the linear bytecode (Main first, then
/// every function body), the size of Main's register frame, and the function
/// table (indexed by `FuncIdx`, with a name → index map for call resolution).
#[derive(Clone, Debug, Default)]
pub struct CompiledProgram {
    pub constants: Vec<Constant>,
    pub code: Vec<Op>,
    pub register_count: usize,
    pub functions: Vec<CompiledFunction>,
    pub fn_index: HashMap<Symbol, FuncIdx>,
    /// Names of the promoted globals (Main top-level bindings referenced from
    /// function/closure bodies), for "Undefined variable" errors.
    pub globals: Vec<String>,
    /// Which Main-frame registers carry a user-visible NAME (Let targets,
    /// loop variables). Everything else is a statement-local scratch — dead
    /// at every statement boundary by the allocator's recycling discipline —
    /// so the region JIT neither writes it back nor preserves its pre-state.
    pub named_regs: Vec<bool>,
    /// `loop_locals[head]` = the registers bound INSIDE the loop whose region
    /// head (back-edge target) is the absolute pc `head` — names lexically dead
    /// the moment that loop exits. The region JIT subtracts these from the
    /// write-back set, so copy-prop/CSE/fusion can treat them as true scratch.
    /// Keyed by absolute pc (one code array); valued by the head's OWNING frame's
    /// register indices (Main or the enclosing function).
    pub loop_locals: HashMap<usize, Vec<bool>>,
    /// DEBUG-ONLY: Main-frame register index → source variable name, populated only
    /// by the debugger's compile path ([`crate::vm::Compiler::compile_for_debug`]).
    /// Empty on every production build, so the runtime pays nothing; it just lets the
    /// Studio debug drawer show `x` instead of `R0`.
    pub reg_names: Vec<(u16, String)>,
}

#[cfg(test)]
mod concurrency_op_tests {
    use super::*;

    /// Every concurrency op must survive the tier cache's `serde_json` roundtrip
    /// (`CompiledProgram`/`FnBytecode` are cached as JSON), or a cached program
    /// containing one would fail to reload. `Op` is `Copy` but not `PartialEq`,
    /// so we compare its `Debug` form.
    #[test]
    fn concurrency_ops_serde_roundtrip() {
        let ops = [
            Op::ChanNew { dst: 1, cap: -1 },
            Op::ChanNew { dst: 2, cap: 8 },
            Op::ChanSend { chan: 3, val: 4 },
            Op::ChanRecv { dst: 5, chan: 6 },
            Op::ChanTrySend { dst: 7, chan: 8, val: 9 },
            Op::ChanTryRecv { dst: 10, chan: 11 },
            Op::ChanClose { chan: 12 },
            Op::Spawn { func: 1, args_start: 13, arg_count: 2 },
            Op::SpawnHandle { dst: 14, func: 2, args_start: 15, arg_count: 0 },
            Op::TaskAwait { dst: 16, handle: 17 },
            Op::TaskAbort { handle: 18 },
            Op::SelectArmRecv { chan: 19, var: 20 },
            Op::SelectArmTimeout { ticks: 21 },
            Op::SelectWait { dst_arm: 22 },
        ];
        for op in ops {
            let json = serde_json::to_string(&op).expect("serialize");
            let back: Op = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(format!("{op:?}"), format!("{back:?}"), "roundtrip {op:?} via {json}");
        }
    }
}
