//! Bytecode instruction set and the compiled-program container.
//!
//! Registers are per-frame `u16` indices (the compiler assigns locals to
//! registers at compile time; frames are capped at `MAX_REGISTERS_PER_FRAME`).
//! Jump targets are absolute instruction indices.

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
/// The declared element type of a `Pipe of T`, carried on [`Op::ChanNew`] so a consumer that
/// models a channel as a typed FIFO queue (the direct-WASM AOT) can type a `Receive`/`select`
/// arm's bound variable even when the pipe is never sent to (an empty `Pipe of Text` in a
/// timeout-only `select`). A `Copy` tag — the scalar element types cover every pipe the corpus
/// declares; a struct/enum element resolves to [`ChanElem::Unknown`] (falls back to the untyped
/// queue, exactly as before).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChanElem {
    Unknown,
    Int,
    Float,
    Bool,
    Text,
}

impl ChanElem {
    /// Resolve a pipe's declared element type name (`Pipe of <name>`) to its tag.
    pub fn from_type_name(name: &str) -> ChanElem {
        match name {
            "Int" | "Nat" => ChanElem::Int,
            "Real" | "Float" => ChanElem::Float,
            "Bool" => ChanElem::Bool,
            "Text" => ChanElem::Text,
            _ => ChanElem::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Op {
    /// `R[dst] = constants[idx]`
    LoadConst {
        dst: Reg,
        idx: ConstIdx,
    },
    /// `R[dst] = R[src]` (shallow clone)
    Move {
        dst: Reg,
        src: Reg,
    },
    /// Exchange `R[a]` and `R[b]` in place — the canonical ZERO-GARBAGE reversible primitive (its
    /// own inverse: swapping twice is the identity). Reverse execution re-applies it with no history
    /// tape entry, so it reverses with zero erased bits (the adiabatic charge-recovery case).
    RevSwap {
        a: Reg,
        b: Reg,
    },
    /// In-place XOR: `R[place] ^= R[delta]` over the ℤ/2ⁿ ring — SELF-INVERSE (`(x ⊕ e) ⊕ e = x`), so
    /// reverse re-applies it with no history-tape entry (zero garbage). Requires `place != delta`
    /// (else `x ⊕ x = 0` erases `x`) — a well-formedness invariant the emitter guarantees, like a
    /// valid jump target. `R[delta]` is read-only, and the reverse-order invariant restores it to its
    /// forward value before this op is re-applied, so the inverse is exact.
    RevXor {
        place: Reg,
        delta: Reg,
    },
    /// In-place modular negation: `R[place] = -R[place]` (wrapping two's-complement) — SELF-INVERSE
    /// (`-(-x) = x` for every `i64`, `i64::MIN` included), so reverse re-applies it with no tape entry.
    /// The additive inverse of the (ℤ/2ⁿ, +) group.
    RevNeg {
        place: Reg,
    },
    /// In-place modular add: `R[place] = R[place] + R[delta]` (wrapping, non-promoting — the reversible
    /// additive translation, DISTINCT from the BigInt-promoting [`Op::Add`]). Its inverse is
    /// [`Op::RevSub`] with the same operands; reverse applies that, no tape entry. Requires
    /// `place != delta` (else `x + x = 2x` is not invertible by subtraction). `R[delta]` is read-only.
    RevAdd {
        place: Reg,
        delta: Reg,
    },
    /// In-place modular subtract: `R[place] = R[place] - R[delta]` (wrapping). Mutual inverse of
    /// [`Op::RevAdd`]; reverse applies `RevAdd`, no tape entry. Requires `place != delta`.
    RevSub {
        place: Reg,
        delta: Reg,
    },
    /// Copy-on-write barrier: if `R[reg]`'s collection is shared (`Rc` strong > 1),
    /// deep-clone it in place so `R[reg]` becomes the sole owner. Emitted before an
    /// argument is passed to a function whose corresponding parameter is an inferred
    /// MUTABLE BORROW (mutated in place and returned): the callee's element writes
    /// then land on a buffer no OTHER live handle can observe, so an aliasing caller
    /// (`Let y be arr; Set arr to f(arr)`) still sees value semantics. A no-op when
    /// already uniquely owned (the common `Set x to f(x)` consume-reassign), so the
    /// hot path pays only a strong-count check. Mirrors the AOT's call-site `.cow()`.
    EnsureOwned {
        reg: Reg,
    },

    Add {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    /// `R[dst] = R[dst] + R[src]` — the `Set x to x + …` shape. Semantically
    /// identical to `Add { dst, lhs: dst, rhs: src }`; the dedicated form lets
    /// the VM append in place when `R[dst]` is a sole-owner Text (turning the
    /// O(n²) build-a-string-by-concatenation loop into amortized O(n)).
    AddAssign {
        dst: Reg,
        src: Reg,
    },
    Sub {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    Mul {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    Div {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    /// EXACT division (`7 / 2 → 7/2`, a Rational), the type-directed sibling of
    /// [`Op::Div`] — emitted for `BinaryOpKind::ExactDivide`.
    ExactDiv {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    /// FLOOR division (`-7 // 2 → -4`, toward negative infinity) — emitted for
    /// `BinaryOpKind::FloorDivide`, distinct from the truncating [`Op::Div`].
    FloorDiv {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    Mod {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    /// `dst = lhs / 2^k` (signed, round toward zero) — emitted only when the
    /// divisor is a literal power of two AND the Oracle proved `lhs` is `Int`.
    /// A single op (the JIT lowers it to the side-exit-free `divpow2` shift
    /// stencil) so it fires for loop-invariant divisors the in-region JIT
    /// detector misses, without the scratch-register pressure of an expansion.
    DivPow2 {
        dst: Reg,
        lhs: Reg,
        k: u8,
    },
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
    /// non-negative `lhs`. `divisor` carries the original constant `c` so the VM handler can fall
    /// back to an exact `Div`/`Mod` when `lhs` is a `BigInt` the u64 magic reciprocal cannot hold
    /// (the optimistic non-negative gate does not prove the dividend fits i64).
    /// `wrap`: the Oracle also proved `lhs < 2 * divisor` (`Opt::ModWrap`), so the remainder is `lhs`,
    /// less `divisor` once when `lhs >= divisor` — a compare and a subtract a backend may compute in
    /// place of the magic sequence. A backend that reads the magic constants stays exact.
    MagicDivU {
        dst: Reg,
        lhs: Reg,
        magic: u64,
        more: u8,
        mul_back: i64,
        divisor: i64,
        wrap: bool,
    },

    Lt {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    Gt {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    LtEq {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    GtEq {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    Eq {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    /// Tolerant float comparison (`a is approximately b`) — the shared
    /// isclose semantics (`logicaffeine_data::ops::logos_approx_eq`);
    /// `==`/`Eq` stays IEEE bit-exact.
    ApproxEq {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    NotEq {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },

    Not {
        dst: Reg,
        src: Reg,
    },

    Concat {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    /// `a followed by b` — merge two sequences into one.
    SeqConcat {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    /// `a ** b` — exponentiation. Integer power is exact (promotes to BigInt on
    /// overflow); a Float operand uses `powf`; a negative Int exponent errors.
    Pow {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    BitXor {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    /// `a & b` — bitwise AND on Int, logical on Bool, intersection on Sets.
    BitAnd {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    /// `a | b` (see `BitAnd`) — union on Sets.
    BitOr {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    Shl {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    Shr {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },

    /// Unconditional jump to absolute instruction index.
    Jump {
        target: usize,
    },
    /// Jump if `R[cond]` is falsey.
    JumpIfFalse {
        cond: Reg,
        target: usize,
    },
    /// Jump if `R[cond]` is truthy.
    JumpIfTrue {
        cond: Reg,
        target: usize,
    },

    /// Call a user function. The caller has placed `arg_count` arguments in
    /// consecutive registers starting at `args_start` (relative to the caller's
    /// frame base). The result is written to `R[dst]`. Uses register windowing.
    Call {
        dst: Reg,
        func: FuncIdx,
        args_start: Reg,
        arg_count: u16,
    },
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
    CallValue {
        dst: Reg,
        callee: Reg,
        args_start: Reg,
        arg_count: u16,
        name_for_err: ConstIdx,
    },
    /// Build a closure over `program.functions[func]`: its `captures` list is
    /// snapshotted — local captures deep-cloned from the register window at
    /// `locals_start`, global captures from the globals table (skipped when
    /// still undefined; the body then falls through to the live global).
    MakeClosure {
        dst: Reg,
        func: FuncIdx,
        locals_start: Reg,
    },

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
    ListPushField {
        obj: Reg,
        field: ConstIdx,
        src: Reg,
    },

    // ---- Globals (Main top-level bindings visible inside functions) ----
    /// `R[dst] = globals[idx]`, error "Undefined variable: {name}" when unset.
    GlobalGet {
        dst: Reg,
        idx: u16,
    },
    /// `globals[idx] = R[src]` (defines or overwrites).
    GlobalSet {
        idx: u16,
        src: Reg,
    },
    /// Return `R[src]` from the current function.
    Return {
        src: Reg,
    },
    /// Return `nothing` from the current function.
    ReturnNothing,

    // ---- Collections ----
    /// `R[dst] = [R[start], …, R[start+count-1]]` (a new list).
    NewList {
        dst: Reg,
        start: Reg,
        count: u16,
    },
    NewEmptyList {
        dst: Reg,
    },
    /// `R[dst] = a new empty list reserving max(R[capacity], 0) elements`.
    /// Unlike `NewEmptyListFixed`, this carries a source/runtime capacity hint
    /// and therefore reads a register rather than a compile-time proof.
    NewEmptyListWithCapacity {
        dst: Reg,
        capacity: Reg,
    },
    /// `R[dst] = a new empty list whose exact maximum length is capacity`.
    /// The shared fixed-sequence proof emits this only for a non-escaping local.
    /// It is observably identical to `NewEmptyList`; storage-aware backends may
    /// reserve once or use frame-local backing instead of a growable heap list.
    NewEmptyListFixed {
        dst: Reg,
        capacity: u16,
    },
    /// `R[dst] = a new half-width (`Vec<i32>`) Int sequence` — emitted (behind
    /// `LOGOS_NARROW_VM`) for a `new Seq of Int` declaration the narrowing
    /// proof certified fits `i32`. Observably identical to `NewEmptyList`; only
    /// the storage width differs (see [`crate::interpreter::ListRepr::IntsI32`]).
    NewEmptyListI32 {
        dst: Reg,
    },
    NewEmptySet {
        dst: Reg,
    },
    NewEmptyMap {
        dst: Reg,
    },
    /// `R[dst] = a new empty map reserving max(R[capacity], 0) entries`.
    NewEmptyMapWithCapacity {
        dst: Reg,
        capacity: Reg,
    },
    /// `R[dst] = [R[start]..=R[end]]` (inclusive integer range as a list).
    NewRange {
        dst: Reg,
        start: Reg,
        end: Reg,
    },
    /// Append `R[value]` to the list in `R[list]` (mutates in place).
    ListPush {
        list: Reg,
        value: Reg,
    },
    /// Add `R[value]` to the set in `R[set]` (no-op if already present).
    SetAdd {
        set: Reg,
        value: Reg,
    },
    /// Remove `R[value]` from the set/map in `R[collection]`.
    RemoveFrom {
        collection: Reg,
        value: Reg,
    },
    /// `R[collection][R[index]] = R[value]` (1-based list set, or map insert).
    SetIndex {
        collection: Reg,
        index: Reg,
        value: Reg,
    },
    /// Like `SetIndex` but the Oracle PROVED the index in `[1, length]`
    /// (range analysis, M9) — bounds-check elimination for the STORE. The
    /// interpreter still checks (free defense-in-depth); the JIT lowers it to
    /// an UNCHECKED array store (no bounds branch). Only listy collections
    /// with a stable length earn this, via `index_provably_in_bounds`.
    SetIndexUnchecked {
        collection: Reg,
        index: Reg,
        value: Reg,
    },
    /// `R[dst] = R[collection][R[index]]` (1-based for ordered collections).
    Index {
        dst: Reg,
        collection: Reg,
        index: Reg,
    },
    /// Like `Index` but the Oracle (range analysis, M9) PROVED the index
    /// in `[1, length]` at this point — bounds-check elimination, the V8/LLVM
    /// way. The bytecode interpreter still checks (a sound proof makes the
    /// check never fire; keeping it is free defense-in-depth), but the JIT
    /// lowers it to an UNCHECKED array load (no bounds branch, no deopt).
    /// Only listy collections with a stable length earn this; the compiler
    /// emits it solely behind `index_provably_in_bounds`.
    IndexUnchecked {
        dst: Reg,
        collection: Reg,
        index: Reg,
    },
    /// REGION-ENTRY bounds-check hoist (V8 TurboFan loop bound-check
    /// elimination). For a loop `while iv </<= bound` reading/writing
    /// `R[array]` at affine indices, this asserts ONCE — at native region
    /// entry — that the array is long enough for the whole loop:
    /// `length(R[array]) >= R[bound] + add_max` and `R[iv] + add_min >= 1`.
    /// If it holds, the region runs every covered access UNCHECKED; if not,
    /// the VM declines the region and replays on bytecode (where the accesses
    /// are checked). A pure no-op in the interpreter and the function tier —
    /// speculation is region-only, made safe solely by this entry guard.
    RegionBoundsGuard {
        array: Reg,
        bound: Reg,
        iv: Reg,
        add_max: i32,
        add_min: i32,
    },
    /// `R[dst] = length of R[collection]`.
    Length {
        dst: Reg,
        collection: Reg,
    },
    /// `R[dst] = R[collection] contains R[value]`.
    Contains {
        dst: Reg,
        collection: Reg,
        value: Reg,
    },

    // ---- Strings / slices / tuples / sets / temporal ----
    /// `R[dst] = Text(debug_prefix? + format(R[src], spec?))` — one
    /// interpolated-string segment. `u32::MAX` = no spec / no prefix.
    FormatValue {
        dst: Reg,
        src: Reg,
        spec: ConstIdx,
        debug_prefix: ConstIdx,
    },
    /// `R[dst] = R[collection][R[start]..=R[end]]` (1-indexed inclusive).
    SliceOp {
        dst: Reg,
        collection: Reg,
        start: Reg,
        end: Reg,
    },
    /// `R[dst] = deep clone of R[src]`.
    DeepClone {
        dst: Reg,
        src: Reg,
    },
    /// `R[dst] = (R[start], …, R[start+count-1])` (immutable tuple).
    NewTuple {
        dst: Reg,
        start: Reg,
        count: u16,
    },
    UnionOp {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    IntersectOp {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    /// `R[dst] = today` (Date; honors the test fixed-clock).
    LoadToday {
        dst: Reg,
    },
    /// `R[dst] = now` (Moment; honors the test fixed-clock).
    LoadNow {
        dst: Reg,
    },

    // ---- Structs / enums / Inspect / CRDT ----
    /// `R[dst] = Struct { type_name: constants[type_name], fields: {} }`.
    NewStruct {
        dst: Reg,
        type_name: ConstIdx,
    },
    /// Insert `R[value]` under field `constants[field]` into the struct in
    /// `R[obj]` (construction and SetField — structs have VALUE semantics, so
    /// in-place mutation of the register equals the tree-walker's
    /// clone-mutate-reassign).
    StructInsert {
        obj: Reg,
        field: ConstIdx,
        value: Reg,
    },
    /// `R[dst] = R[obj].field` — "Field '{f}' not found" / "Cannot access
    /// field on {type}".
    GetField {
        dst: Reg,
        obj: Reg,
        field: ConstIdx,
    },
    /// `R[dst] = Inductive { type, constructor, args: R[args_start..+count] }`.
    NewInductive {
        dst: Reg,
        type_name: ConstIdx,
        ctor: ConstIdx,
        args_start: Reg,
        count: u16,
    },
    /// `R[dst] = Bool(struct type-name or inductive constructor == constants[variant])`;
    /// false for any other value.
    TestArm {
        dst: Reg,
        target: Reg,
        variant: ConstIdx,
    },
    /// Inspect-arm binding: a Struct target binds `fields[constants[field]]`,
    /// an Inductive target binds `args[index]`. A missing field/index leaves
    /// `R[dst]` unwritten (the tree-walker skips the bind — unreachable for
    /// parsed programs, whose fields always exist after default-fill).
    BindArm {
        dst: Reg,
        target: Reg,
        field: ConstIdx,
        index: u16,
    },
    /// GCounter/PNCounter bump of `R[obj].field` by `R[amount]` (negated for
    /// Decrease — also selects the tree-walker's increment/decrement wording).
    CrdtBump {
        obj: Reg,
        field: ConstIdx,
        amount: Reg,
        negate: bool,
    },
    /// GCounter merge: fold every field of `R[source]` into `R[target]`.
    CrdtMerge {
        target: Reg,
        source: Reg,
    },
    /// GCounter LATTICE JOIN: per-replica MAX of two 4-lane G-Counters (`R[target]` ⊔= `R[source]`),
    /// the state-based, idempotent join. Unlike `CrdtMerge`'s polymorphic field-wise merge (which is
    /// field-wise ADD for bare-int counters), this is the fixed 4-lane per-replica MAX that maps 1:1
    /// to the `crdt_merge` hardware block — the coprocessor the real soft-core issues on its port.
    CrdtJoin {
        target: Reg,
        source: Reg,
    },
    /// 256-BIT BIGINT LANE — one opcode for the whole wide-arithmetic family, routed on the soft-core
    /// to the proven `bigint_alu`/`bigint_div`/`bigint_mul` blocks over an 8×256-bit wide register bank.
    /// `kind` selects the operation: 0 = FromInt (`W[dst] = zext(R[a])`), 1 = Add (`W[dst]=W[a]+W[b]`),
    /// 2 = Sub, 3 = Cmp (`R[dst] = W[a] < W[b]`), 4 = Low (`R[dst] = W[a][31:0]`), 5 = Div, 6 = Mul.
    /// On the chip word the kind rides the high nibble of the dst byte; the VM carries it as a field.
    BigIntWide {
        kind: u8,
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    /// `R[dst]` = a fresh, empty rich CRDT — `kind` 0 = SharedSet (OR-Set),
    /// 1 = SharedSequence (RGA), 2 = Divergent (MV-register). Used to default-fill a
    /// `Shared` struct's CRDT fields, mirroring the tree-walker's `new`-struct init.
    NewCrdt {
        dst: Reg,
        kind: u8,
    },
    /// RGA append: push `R[value]` onto the replicated sequence in `R[seq]` (mutates the
    /// shared CRDT in place, so a field access propagates).
    CrdtAppend {
        seq: Reg,
        value: Reg,
    },
    /// Resolve `R[obj].field` to `R[value]`: a real MV-register resolves in place, a plain
    /// field is overwritten — the same fallback the tree-walker's `Resolve` takes.
    CrdtResolve {
        obj: Reg,
        field: ConstIdx,
        value: Reg,
    },

    // ---- Repeat (snapshot iteration) ----
    /// Snapshot `R[iterable]` (List/Set items, Text chars, Map (k,v) tuples)
    /// and push it onto the iterator stack.
    IterPrepare {
        iterable: Reg,
    },
    /// Load the next snapshot element into `R[dst]` and advance; jump to
    /// `exit` when exhausted (the iterator stays pushed — `IterPop` at the
    /// exit point drops it).
    IterNext {
        dst: Reg,
        exit: usize,
    },
    /// Drop the top iterator.
    IterPop,
    /// `R[dst] = R[list].pop()` — Nothing when empty (not an error).
    ListPop {
        list: Reg,
        dst: Reg,
    },
    /// Sleep for `R[nanos]` (Duration nanos, or Int milliseconds).
    Sleep {
        duration: Reg,
    },
    /// Tuple-pattern binding: `R[start..start+count] = tuple elements` with
    /// zip semantics (stops at the shorter side, like the tree-walker).
    /// Errors when `R[src]` is not a Tuple.
    DestructureTuple {
        src: Reg,
        start: Reg,
        count: u16,
    },

    /// Emit `R[src].to_display_string()` to the output stream.
    Show {
        src: Reg,
    },
    /// Emit an observation event (Wave 13 — the OBSERVED profile only). `site`
    /// indexes [`CompiledProgram::observation_sites`] for the category / message
    /// / payload names / span kind (all resolved at compile time); a `Message`
    /// site reads its payload VALUES from the register range
    /// `R[args_start .. args_start+args_count]`. Never emitted in the elided
    /// (compiled/release) profile — observation is erased before those tiers — so
    /// this op is `Unreachable` for AOT/WASM (see `wasm_aot_lock`).
    Obs {
        site: u32,
        args_start: Reg,
        args_count: u16,
    },
    /// `R[dst] = the program argument vector` as a `Seq of Text` (the
    /// interpreter's `args()` system native, mirroring the compiled binary's
    /// `env::args()`: index 0 is the program name). Outside the JIT integer
    /// subset, so the adapters bail on it and it always runs in the VM.
    Args {
        dst: Reg,
    },
    // ─── Go-like concurrency (Phase 54 / T10) ───────────────────────────────
    // Args travel through register ranges (`Op` is `Copy`). Channel/task handles
    // are ordinary `Value`s (`RuntimeValue::Chan` / `::TaskHandle`). Every op that
    // can block suspends the resumable VM (`run_until_block`) and is serviced by
    // the deterministic scheduler, exactly as the tree-walker's `yield_request`.
    /// `R[dst] = a new channel`; `cap < 0` ⇒ the scheduler's default capacity. `elem` is the
    /// declared `Pipe of T` element type (a hint for a typed-queue consumer; the scheduler ignores it).
    ChanNew {
        dst: Reg,
        cap: i32,
        elem: ChanElem,
    },
    /// Send `R[val]` into channel `R[chan]` (blocks if the channel is full).
    ChanSend {
        chan: Reg,
        val: Reg,
    },
    /// `R[dst] = receive from channel R[chan]` (blocks if the channel is empty).
    ChanRecv {
        dst: Reg,
        chan: Reg,
    },
    /// `R[dst] = bool` — non-blocking send of `R[val]` into `R[chan]`.
    ChanTrySend {
        dst: Reg,
        chan: Reg,
        val: Reg,
    },
    /// `R[dst] = received value, or Nothing` — non-blocking receive from `R[chan]`.
    ChanTryRecv {
        dst: Reg,
        chan: Reg,
    },
    /// Close channel `R[chan]`.
    ChanClose {
        chan: Reg,
    },
    /// Spawn `functions[func]` with args in `R[args_start..+arg_count]` (fire-and-forget).
    Spawn {
        func: FuncIdx,
        args_start: Reg,
        arg_count: u16,
    },
    /// `R[dst] = task handle` of a spawned `functions[func]` (same arg convention).
    SpawnHandle {
        dst: Reg,
        func: FuncIdx,
        args_start: Reg,
        arg_count: u16,
    },
    /// `R[dst] = result of awaiting task R[handle]` (Nothing if it was aborted).
    TaskAwait {
        dst: Reg,
        handle: Reg,
    },
    /// Abort task `R[handle]`.
    TaskAbort {
        handle: Reg,
    },
    /// Register a `Receive var from chan` arm for the next `SelectWait`.
    SelectArmRecv {
        chan: Reg,
        var: Reg,
    },
    /// Register an `After ticks` timeout arm for the next `SelectWait`.
    SelectArmTimeout {
        ticks: Reg,
    },
    /// Block on the registered select arms; `R[dst_arm] = the winning arm index`
    /// (a recv arm's received value is already in its `var` register).
    SelectWait {
        dst_arm: Reg,
    },

    // ─── peer networking over the relay (async-tier). Each suspends the resumable VM via a
    // `VmBlock::Net*` request the async VM driver services with the shared `NetInbox` — the same
    // inbox the tree-walker uses, so a `Send`/`Await` runs byte-identically on both tiers. ───
    /// Dial the relay at `R[url]` (async); resume when connected.
    NetConnect {
        url: Reg,
    },
    /// Subscribe this node's inbox to `R[topic]` (async); resume when subscribed.
    NetListen {
        topic: Reg,
    },
    /// Encode `R[msg]` and publish it to peer `R[to]`.
    NetSend {
        to: Reg,
        msg: Reg,
    },
    /// Batch-stream the list `R[values]` to peer `R[to]`.
    NetStream {
        to: Reg,
        values: Reg,
    },
    /// `R[dst] = await a message (or a batch stream, if `stream`) from peer `R\[from\]`` (blocks).
    NetAwait {
        dst: Reg,
        from: Reg,
        stream: bool,
    },
    /// `R[dst] = a PeerAgent handle for address `R\[addr\]`` (its canonical relay topic). Pure.
    NetMakePeer {
        dst: Reg,
        addr: Reg,
    },
    /// CRDT sync point on topic `R[topic]`: publish `R[dst]`'s counter, merge what has arrived,
    /// and write the merged value back to `R[dst]`.
    NetSync {
        dst: Reg,
        topic: Reg,
    },

    /// A tcp.lg client native (async-tier): `kind` is the `TcpOp` code
    /// (connect/send/recv/recvExact/close — the `BigIntWide` one-op-many-kinds
    /// pattern), operands in `R[args_start..args_start+arg_count]`. Suspends via
    /// `VmBlock::Tcp`; the async io driver services it through the SAME
    /// `TcpRegistry::exec_values` the tree-walker calls, and resumes `R[dst]`
    /// with the LOGOS `Result` value.
    Tcp {
        dst: Reg,
        kind: u8,
        args_start: Reg,
        arg_count: u16,
    },

    /// Serve HTTP on `R[port]` with the handler function `handler` (a resolved
    /// `FuncIdx`) — the `Serve` keystone on the bytecode tier. Suspends via
    /// `VmBlock::Serve`; the async io driver owns the SAME concurrent accept
    /// loop the tree-walker's `serve_http_concurrent` uses, and per connection
    /// RE-ENTERS `handler` AS VM BYTECODE (a fresh child `Vm` at the handler's
    /// entry) with the marshaled `Request` value, reading its `Response`
    /// through the shared request/response marshaling — so both tiers serve
    /// byte-identically. The accept loop never returns; this op never resumes.
    Serve {
        port: Reg,
        handler: FuncIdx,
    },

    /// Fail with the Text constant at `msg` — used for constructs whose
    /// tree-walker semantics are "error WHEN EXECUTED" (an unbound `Set`, an
    /// unsupported statement). Never fails at compile time: dead branches must
    /// stay free.
    FailWith {
        msg: ConstIdx,
    },
    /// Stop execution.
    Halt,
}

impl Op {
    /// A stable name for this op variant — EXHAUSTIVE (no wildcard), so a new VM instruction fails
    /// to compile until it is named here. The single source of truth for op names shared by every
    /// catalog lock (WASM-AOT support, reversibility, …) — see `tests/reverse_op_lock.rs`.
    pub fn name(&self) -> &'static str {
        match self {
            Op::LoadConst { .. } => "LoadConst",
            Op::Move { .. } => "Move",
            Op::RevSwap { .. } => "RevSwap",
            Op::RevXor { .. } => "RevXor",
            Op::RevNeg { .. } => "RevNeg",
            Op::RevAdd { .. } => "RevAdd",
            Op::RevSub { .. } => "RevSub",
            Op::EnsureOwned { .. } => "EnsureOwned",
            Op::Add { .. } => "Add",
            Op::AddAssign { .. } => "AddAssign",
            Op::Sub { .. } => "Sub",
            Op::Mul { .. } => "Mul",
            Op::Div { .. } => "Div",
            Op::ExactDiv { .. } => "ExactDiv",
            Op::FloorDiv { .. } => "FloorDiv",
            Op::Mod { .. } => "Mod",
            Op::DivPow2 { .. } => "DivPow2",
            Op::MagicDivU { .. } => "MagicDivU",
            Op::Lt { .. } => "Lt",
            Op::Gt { .. } => "Gt",
            Op::LtEq { .. } => "LtEq",
            Op::GtEq { .. } => "GtEq",
            Op::Eq { .. } => "Eq",
            Op::ApproxEq { .. } => "ApproxEq",
            Op::NotEq { .. } => "NotEq",
            Op::Not { .. } => "Not",
            Op::Concat { .. } => "Concat",
            Op::SeqConcat { .. } => "SeqConcat",
            Op::Pow { .. } => "Pow",
            Op::BitXor { .. } => "BitXor",
            Op::BitAnd { .. } => "BitAnd",
            Op::BitOr { .. } => "BitOr",
            Op::Shl { .. } => "Shl",
            Op::Shr { .. } => "Shr",
            Op::Jump { .. } => "Jump",
            Op::JumpIfFalse { .. } => "JumpIfFalse",
            Op::JumpIfTrue { .. } => "JumpIfTrue",
            Op::Call { .. } => "Call",
            Op::CallBuiltin { .. } => "CallBuiltin",
            Op::CallValue { .. } => "CallValue",
            Op::MakeClosure { .. } => "MakeClosure",
            Op::CheckPolicy { .. } => "CheckPolicy",
            Op::ListPushField { .. } => "ListPushField",
            Op::GlobalGet { .. } => "GlobalGet",
            Op::GlobalSet { .. } => "GlobalSet",
            Op::Return { .. } => "Return",
            Op::ReturnNothing => "ReturnNothing",
            Op::NewList { .. } => "NewList",
            Op::NewEmptyList { .. } => "NewEmptyList",
            Op::NewEmptyListWithCapacity { .. } => "NewEmptyListWithCapacity",
            Op::NewEmptyListFixed { .. } => "NewEmptyListFixed",
            Op::NewEmptyListI32 { .. } => "NewEmptyListI32",
            Op::NewEmptySet { .. } => "NewEmptySet",
            Op::NewEmptyMap { .. } => "NewEmptyMap",
            Op::NewEmptyMapWithCapacity { .. } => "NewEmptyMapWithCapacity",
            Op::NewRange { .. } => "NewRange",
            Op::ListPush { .. } => "ListPush",
            Op::SetAdd { .. } => "SetAdd",
            Op::RemoveFrom { .. } => "RemoveFrom",
            Op::SetIndex { .. } => "SetIndex",
            Op::SetIndexUnchecked { .. } => "SetIndexUnchecked",
            Op::Index { .. } => "Index",
            Op::IndexUnchecked { .. } => "IndexUnchecked",
            Op::RegionBoundsGuard { .. } => "RegionBoundsGuard",
            Op::Length { .. } => "Length",
            Op::Contains { .. } => "Contains",
            Op::FormatValue { .. } => "FormatValue",
            Op::SliceOp { .. } => "SliceOp",
            Op::DeepClone { .. } => "DeepClone",
            Op::NewTuple { .. } => "NewTuple",
            Op::UnionOp { .. } => "UnionOp",
            Op::IntersectOp { .. } => "IntersectOp",
            Op::LoadToday { .. } => "LoadToday",
            Op::LoadNow { .. } => "LoadNow",
            Op::NewStruct { .. } => "NewStruct",
            Op::StructInsert { .. } => "StructInsert",
            Op::GetField { .. } => "GetField",
            Op::NewInductive { .. } => "NewInductive",
            Op::TestArm { .. } => "TestArm",
            Op::BindArm { .. } => "BindArm",
            Op::CrdtBump { .. } => "CrdtBump",
            Op::CrdtMerge { .. } => "CrdtMerge",
            Op::CrdtJoin { .. } => "CrdtJoin",
            Op::BigIntWide { .. } => "BigIntWide",
            Op::NewCrdt { .. } => "NewCrdt",
            Op::CrdtAppend { .. } => "CrdtAppend",
            Op::CrdtResolve { .. } => "CrdtResolve",
            Op::IterPrepare { .. } => "IterPrepare",
            Op::IterNext { .. } => "IterNext",
            Op::IterPop => "IterPop",
            Op::ListPop { .. } => "ListPop",
            Op::Sleep { .. } => "Sleep",
            Op::DestructureTuple { .. } => "DestructureTuple",
            Op::Show { .. } => "Show",
            Op::Args { .. } => "Args",
            Op::ChanNew { .. } => "ChanNew",
            Op::ChanSend { .. } => "ChanSend",
            Op::ChanRecv { .. } => "ChanRecv",
            Op::ChanTrySend { .. } => "ChanTrySend",
            Op::ChanTryRecv { .. } => "ChanTryRecv",
            Op::ChanClose { .. } => "ChanClose",
            Op::Spawn { .. } => "Spawn",
            Op::SpawnHandle { .. } => "SpawnHandle",
            Op::TaskAwait { .. } => "TaskAwait",
            Op::TaskAbort { .. } => "TaskAbort",
            Op::SelectArmRecv { .. } => "SelectArmRecv",
            Op::SelectArmTimeout { .. } => "SelectArmTimeout",
            Op::SelectWait { .. } => "SelectWait",
            Op::NetConnect { .. } => "NetConnect",
            Op::NetListen { .. } => "NetListen",
            Op::NetSend { .. } => "NetSend",
            Op::NetStream { .. } => "NetStream",
            Op::NetAwait { .. } => "NetAwait",
            Op::NetMakePeer { .. } => "NetMakePeer",
            Op::NetSync { .. } => "NetSync",
            Op::Tcp { .. } => "Tcp",
            Op::Serve { .. } => "Serve",
            Op::FailWith { .. } => "FailWith",
            Op::Obs { .. } => "Obs",
            Op::Halt => "Halt",
        }
    }

    /// Every register this op WRITES (rebinds). EXHAUSTIVE (no wildcard) and over-approximate — a
    /// conditional write (`IterNext`'s element) and a copy-on-write rebind (`EnsureOwned`) count. An
    /// analysis that types or classifies registers takes a written register's facts from the writing
    /// op alone, never from what it held before; a new op that writes a register cannot compile until
    /// it is listed here.
    pub fn for_each_written_reg(&self, mut f: impl FnMut(Reg)) {
        match self {
            Op::LoadConst { dst, .. }
            | Op::Move { dst, .. }
            | Op::Add { dst, .. }
            | Op::BigIntWide { dst, .. }
            | Op::AddAssign { dst, .. }
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
            | Op::ApproxEq { dst, .. }
            | Op::NotEq { dst, .. }
            | Op::Not { dst, .. }
            | Op::Concat { dst, .. }
            | Op::SeqConcat { dst, .. }
            | Op::Pow { dst, .. }
            | Op::BitXor { dst, .. }
            | Op::BitAnd { dst, .. }
            | Op::BitOr { dst, .. }
            | Op::Shl { dst, .. }
            | Op::Shr { dst, .. }
            | Op::Call { dst, .. }
            | Op::CallBuiltin { dst, .. }
            | Op::CallValue { dst, .. }
            | Op::MakeClosure { dst, .. }
            | Op::GlobalGet { dst, .. }
            | Op::NewList { dst, .. }
            | Op::NewEmptyList { dst }
            | Op::NewEmptyListWithCapacity { dst, .. }
            | Op::NewEmptyListFixed { dst, .. }
            | Op::NewEmptyListI32 { dst }
            | Op::NewEmptySet { dst }
            | Op::NewEmptyMap { dst }
            | Op::NewEmptyMapWithCapacity { dst, .. }
            | Op::NewRange { dst, .. }
            | Op::Index { dst, .. }
            | Op::IndexUnchecked { dst, .. }
            | Op::Length { dst, .. }
            | Op::Contains { dst, .. }
            | Op::FormatValue { dst, .. }
            | Op::SliceOp { dst, .. }
            | Op::DeepClone { dst, .. }
            | Op::NewTuple { dst, .. }
            | Op::UnionOp { dst, .. }
            | Op::IntersectOp { dst, .. }
            | Op::LoadToday { dst }
            | Op::LoadNow { dst }
            | Op::NewStruct { dst, .. }
            | Op::GetField { dst, .. }
            | Op::NewInductive { dst, .. }
            | Op::TestArm { dst, .. }
            | Op::BindArm { dst, .. }
            | Op::NewCrdt { dst, .. }
            | Op::IterNext { dst, .. }
            | Op::ListPop { dst, .. }
            | Op::Args { dst }
            | Op::ChanNew { dst, .. }
            | Op::ChanRecv { dst, .. }
            | Op::ChanTrySend { dst, .. }
            | Op::ChanTryRecv { dst, .. }
            | Op::SpawnHandle { dst, .. }
            | Op::TaskAwait { dst, .. }
            | Op::NetAwait { dst, .. }
            | Op::NetMakePeer { dst, .. }
            | Op::NetSync { dst, .. }
            | Op::Tcp { dst, .. }
            | Op::SelectWait { dst_arm: dst } => f(*dst),

            Op::RevSwap { a, b } => {
                f(*a);
                f(*b);
            }
            Op::RevXor { place, .. }
            | Op::RevNeg { place }
            | Op::RevAdd { place, .. }
            | Op::RevSub { place, .. } => f(*place),
            Op::EnsureOwned { reg } => f(*reg),
            Op::SelectArmRecv { var, .. } => f(*var),
            Op::DestructureTuple { start, count, .. } => {
                for i in 0..*count {
                    f(start + i);
                }
            }

            // No register writes: control, stores, effects.
            Op::Jump { .. }
            | Op::JumpIfFalse { .. }
            | Op::JumpIfTrue { .. }
            | Op::CheckPolicy { .. }
            | Op::ListPushField { .. }
            | Op::GlobalSet { .. }
            | Op::Return { .. }
            | Op::ReturnNothing
            | Op::ListPush { .. }
            | Op::SetAdd { .. }
            | Op::RemoveFrom { .. }
            | Op::SetIndex { .. }
            | Op::SetIndexUnchecked { .. }
            | Op::RegionBoundsGuard { .. }
            | Op::StructInsert { .. }
            | Op::CrdtBump { .. }
            | Op::CrdtMerge { .. }
            | Op::CrdtJoin { .. }
            | Op::CrdtAppend { .. }
            | Op::CrdtResolve { .. }
            | Op::IterPrepare { .. }
            | Op::IterPop
            | Op::Sleep { .. }
            | Op::Show { .. }
            | Op::Obs { .. }
            | Op::ChanSend { .. }
            | Op::ChanClose { .. }
            | Op::Spawn { .. }
            | Op::TaskAbort { .. }
            | Op::SelectArmTimeout { .. }
            | Op::NetConnect { .. }
            | Op::NetListen { .. }
            | Op::NetSend { .. }
            | Op::NetStream { .. }
            | Op::Serve { .. }
            | Op::FailWith { .. }
            | Op::Halt => {}
        }
    }

    /// Every register this op READS — EXHAUSTIVE and over-approximate: every range operand is expanded, and
    /// a `Reg::MAX` "no object" sentinel is skipped. A `MakeClosure` snapshots `captures(func)` locals from
    /// `locals_start`; a caller without the program passes an upper bound, which only reads more.
    pub fn for_each_read_reg(&self, captures: impl Fn(FuncIdx) -> usize, mut f: impl FnMut(Reg)) {
        fn range(start: Reg, count: u16, f: &mut impl FnMut(Reg)) {
            for i in 0..count {
                f(start.wrapping_add(i));
            }
        }
        match self {
            // dst = f(lhs, rhs)
            Op::Add { lhs, rhs, .. }
            | Op::Sub { lhs, rhs, .. }
            | Op::Mul { lhs, rhs, .. }
            | Op::Div { lhs, rhs, .. }
            | Op::ExactDiv { lhs, rhs, .. }
            | Op::FloorDiv { lhs, rhs, .. }
            | Op::Mod { lhs, rhs, .. }
            | Op::Lt { lhs, rhs, .. }
            | Op::Gt { lhs, rhs, .. }
            | Op::LtEq { lhs, rhs, .. }
            | Op::GtEq { lhs, rhs, .. }
            | Op::Eq { lhs, rhs, .. }
            | Op::ApproxEq { lhs, rhs, .. }
            | Op::NotEq { lhs, rhs, .. }
            | Op::Concat { lhs, rhs, .. }
            | Op::SeqConcat { lhs, rhs, .. }
            | Op::Pow { lhs, rhs, .. }
            | Op::BitXor { lhs, rhs, .. }
            | Op::BitAnd { lhs, rhs, .. }
            | Op::BitOr { lhs, rhs, .. }
            | Op::Shl { lhs, rhs, .. }
            | Op::Shr { lhs, rhs, .. }
            | Op::UnionOp { lhs, rhs, .. }
            | Op::IntersectOp { lhs, rhs, .. } => {
                f(*lhs);
                f(*rhs);
            }
            Op::DivPow2 { lhs, .. } | Op::MagicDivU { lhs, .. } => f(*lhs),
            // dst = f(src)
            Op::Move { src, .. }
            | Op::Not { src, .. }
            | Op::DeepClone { src, .. }
            | Op::FormatValue { src, .. } => f(*src),
            // dst = R[dst] op R[src] (dst read + written)
            Op::AddAssign { dst, src } => {
                f(*dst);
                f(*src);
            }
            // in-place reversible core (place read + written; delta read)
            Op::RevAdd { place, delta }
            | Op::RevSub { place, delta }
            | Op::RevXor { place, delta } => {
                f(*place);
                f(*delta);
            }
            Op::RevNeg { place } => f(*place),
            Op::RevSwap { a, b } => {
                f(*a);
                f(*b);
            }
            Op::EnsureOwned { reg } => f(*reg),
            // pure reads → fresh dst
            Op::Index {
                collection, index, ..
            }
            | Op::IndexUnchecked {
                collection, index, ..
            } => {
                f(*collection);
                f(*index);
            }
            Op::Length { collection, .. } => f(*collection),
            Op::Contains {
                collection, value, ..
            } => {
                f(*collection);
                f(*value);
            }
            Op::SliceOp {
                collection,
                start,
                end,
                ..
            } => {
                f(*collection);
                f(*start);
                f(*end);
            }
            Op::GetField { obj, .. } => f(*obj),
            Op::TestArm { target, .. } | Op::BindArm { target, .. } => f(*target),
            Op::NewRange { start, end, .. } => {
                f(*start);
                f(*end);
            }
            // range constructors
            Op::NewList { start, count, .. } | Op::NewTuple { start, count, .. } => {
                range(*start, *count, &mut f)
            }
            Op::NewInductive {
                args_start, count, ..
            } => range(*args_start, *count, &mut f),
            // calls: consecutive arg window
            Op::Call {
                args_start,
                arg_count,
                ..
            }
            | Op::CallBuiltin {
                args_start,
                arg_count,
                ..
            } => range(*args_start, *arg_count, &mut f),
            Op::CallValue {
                callee,
                args_start,
                arg_count,
                ..
            } => {
                f(*callee);
                range(*args_start, *arg_count, &mut f);
            }
            // a closure snapshots its local captures from `locals_start` (over-approximate the
            // window with the whole capture count; global captures within it are read harmlessly)
            Op::MakeClosure {
                func, locals_start, ..
            } => {
                range(
                    *locals_start,
                    captures(*func).min(Reg::MAX as usize) as u16,
                    &mut f,
                );
            }
            Op::Obs {
                args_start,
                args_count,
                ..
            } => range(*args_start, *args_count, &mut f),
            Op::Spawn {
                args_start,
                arg_count,
                ..
            }
            | Op::SpawnHandle {
                args_start,
                arg_count,
                ..
            } => range(*args_start, *arg_count, &mut f),
            Op::Tcp {
                args_start,
                arg_count,
                ..
            } => range(*args_start, *arg_count, &mut f),
            // in-place mutations: collection/struct read (and mutated) + value read
            Op::ListPush { list, value } => {
                f(*list);
                f(*value);
            }
            Op::ListPushField { obj, src, .. } => {
                f(*obj);
                f(*src);
            }
            Op::SetAdd { set, value } => {
                f(*set);
                f(*value);
            }
            Op::RemoveFrom { collection, value } => {
                f(*collection);
                f(*value);
            }
            Op::SetIndex {
                collection,
                index,
                value,
            }
            | Op::SetIndexUnchecked {
                collection,
                index,
                value,
            } => {
                f(*collection);
                f(*index);
                f(*value);
            }
            Op::StructInsert { obj, value, .. } => {
                f(*obj);
                f(*value);
            }
            Op::CrdtBump { obj, amount, .. } => {
                f(*obj);
                f(*amount);
            }
            Op::CrdtMerge { target, source } | Op::CrdtJoin { target, source } => {
                f(*target);
                f(*source);
            }
            Op::CrdtAppend { seq, value } => {
                f(*seq);
                f(*value);
            }
            Op::CrdtResolve { obj, value, .. } => {
                f(*obj);
                f(*value);
            }
            Op::ListPop { list, .. } => f(*list),
            Op::BigIntWide { a, b, .. } => {
                f(*a);
                f(*b);
            }
            Op::DestructureTuple { src, .. } => f(*src),
            // control / effects reading a scalar
            Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. } => f(*cond),
            Op::GlobalSet { src, .. } => f(*src),
            Op::Return { src } => f(*src),
            Op::Show { src } => f(*src),
            Op::Sleep { duration } => f(*duration),
            Op::IterPrepare { iterable } => f(*iterable),
            Op::RegionBoundsGuard {
                array, bound, iv, ..
            } => {
                f(*array);
                f(*bound);
                f(*iv);
            }
            Op::CheckPolicy {
                subject, object, ..
            } => {
                f(*subject);
                if *object != Reg::MAX {
                    f(*object);
                }
            }
            // concurrency / net handle + value reads
            Op::ChanSend { chan, val } => {
                f(*chan);
                f(*val);
            }
            Op::ChanRecv { chan, .. } | Op::ChanTryRecv { chan, .. } | Op::ChanClose { chan } => {
                f(*chan)
            }
            Op::ChanTrySend { chan, val, .. } => {
                f(*chan);
                f(*val);
            }
            Op::TaskAwait { handle, .. } | Op::TaskAbort { handle } => f(*handle),
            Op::SelectArmRecv { chan, .. } => f(*chan),
            Op::SelectArmTimeout { ticks } => f(*ticks),
            Op::NetConnect { url } => f(*url),
            Op::NetListen { topic } => f(*topic),
            Op::NetSend { to, msg } => {
                f(*to);
                f(*msg);
            }
            Op::NetStream { to, values } => {
                f(*to);
                f(*values);
            }
            Op::NetAwait { from, .. } => f(*from),
            Op::NetMakePeer { addr, .. } => f(*addr),
            Op::NetSync { dst, topic } => {
                f(*dst);
                f(*topic);
            }
            Op::Serve { port, .. } => f(*port),
            Op::NewEmptyListWithCapacity { capacity, .. }
            | Op::NewEmptyMapWithCapacity { capacity, .. } => f(*capacity),
            // read nothing
            Op::LoadConst { .. }
            | Op::LoadToday { .. }
            | Op::LoadNow { .. }
            | Op::Args { .. }
            | Op::NewEmptyList { .. }
            | Op::NewEmptyListFixed { .. }
            | Op::NewEmptyListI32 { .. }
            | Op::NewEmptySet { .. }
            | Op::NewEmptyMap { .. }
            | Op::NewStruct { .. }
            | Op::NewCrdt { .. }
            | Op::GlobalGet { .. }
            | Op::ChanNew { .. }
            | Op::SelectWait { .. }
            | Op::Jump { .. }
            | Op::IterNext { .. }
            | Op::IterPop
            | Op::ReturnNothing
            | Op::Halt
            | Op::FailWith { .. } => {}
        }
    }

    /// One instance of EVERY VM bytecode op — the full instruction catalog of the language. Catalog
    /// locks iterate this to COUNT coverage/classification. (`name`, WASM-AOT `op_support`, and
    /// `reversibility` are exhaustive matches, so the compiler already forbids a new op from
    /// escaping classification; this enumeration is what lets a lock *count* the ISA.)
    pub fn all_variants() -> Vec<Op> {
        let s = Symbol::from_index(0);
        use crate::semantics::builtins::BuiltinId;
        vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::Move { dst: 0, src: 0 },
            Op::RevSwap { a: 0, b: 0 },
            Op::RevXor { place: 0, delta: 1 },
            Op::RevNeg { place: 0 },
            Op::RevAdd { place: 0, delta: 1 },
            Op::RevSub { place: 0, delta: 1 },
            Op::EnsureOwned { reg: 0 },
            Op::Add {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::AddAssign { dst: 0, src: 0 },
            Op::Sub {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::Mul {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::Div {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::ExactDiv {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::FloorDiv {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::Mod {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::DivPow2 {
                dst: 0,
                lhs: 0,
                k: 0,
            },
            Op::MagicDivU {
                dst: 0,
                lhs: 0,
                magic: 0,
                more: 0,
                mul_back: 0,
                divisor: 1,
                wrap: false,
            },
            Op::Lt {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::Gt {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::LtEq {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::GtEq {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::Eq {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::ApproxEq {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::NotEq {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::Not { dst: 0, src: 0 },
            Op::Concat {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::SeqConcat {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::Pow {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::BitXor {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::BitAnd {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::BitOr {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::Shl {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::Shr {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::Jump { target: 0 },
            Op::JumpIfFalse { cond: 0, target: 0 },
            Op::JumpIfTrue { cond: 0, target: 0 },
            Op::Call {
                dst: 0,
                func: 0,
                args_start: 0,
                arg_count: 0,
            },
            Op::CallBuiltin {
                dst: 0,
                builtin: BuiltinId::Sqrt,
                args_start: 0,
                arg_count: 1,
            },
            Op::CallValue {
                dst: 0,
                callee: 0,
                args_start: 0,
                arg_count: 0,
                name_for_err: 0,
            },
            Op::MakeClosure {
                dst: 0,
                func: 0,
                locals_start: 0,
            },
            Op::CheckPolicy {
                subject: 0,
                predicate: s,
                is_capability: false,
                object: 0,
                source_text: 0,
            },
            Op::ListPushField {
                obj: 0,
                field: 0,
                src: 0,
            },
            Op::GlobalGet { dst: 0, idx: 0 },
            Op::GlobalSet { idx: 0, src: 0 },
            Op::Return { src: 0 },
            Op::ReturnNothing,
            Op::NewList {
                dst: 0,
                start: 0,
                count: 0,
            },
            Op::NewEmptyList { dst: 0 },
            Op::NewEmptyListWithCapacity {
                dst: 0,
                capacity: 1,
            },
            Op::NewEmptyListFixed {
                dst: 0,
                capacity: 8,
            },
            Op::NewEmptyListI32 { dst: 0 },
            Op::NewEmptySet { dst: 0 },
            Op::NewEmptyMap { dst: 0 },
            Op::NewEmptyMapWithCapacity {
                dst: 0,
                capacity: 1,
            },
            Op::NewRange {
                dst: 0,
                start: 0,
                end: 0,
            },
            Op::ListPush { list: 0, value: 0 },
            Op::SetAdd { set: 0, value: 0 },
            Op::RemoveFrom {
                collection: 0,
                value: 0,
            },
            Op::SetIndex {
                collection: 0,
                index: 0,
                value: 0,
            },
            Op::SetIndexUnchecked {
                collection: 0,
                index: 0,
                value: 0,
            },
            Op::Index {
                dst: 0,
                collection: 0,
                index: 0,
            },
            Op::IndexUnchecked {
                dst: 0,
                collection: 0,
                index: 0,
            },
            Op::RegionBoundsGuard {
                array: 0,
                bound: 0,
                iv: 0,
                add_max: 0,
                add_min: 0,
            },
            Op::Length {
                dst: 0,
                collection: 0,
            },
            Op::Contains {
                dst: 0,
                collection: 0,
                value: 0,
            },
            Op::FormatValue {
                dst: 0,
                src: 0,
                spec: 0,
                debug_prefix: 0,
            },
            Op::SliceOp {
                dst: 0,
                collection: 0,
                start: 0,
                end: 0,
            },
            Op::DeepClone { dst: 0, src: 0 },
            Op::NewTuple {
                dst: 0,
                start: 0,
                count: 0,
            },
            Op::UnionOp {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::IntersectOp {
                dst: 0,
                lhs: 0,
                rhs: 0,
            },
            Op::LoadToday { dst: 0 },
            Op::LoadNow { dst: 0 },
            Op::NewStruct {
                dst: 0,
                type_name: 0,
            },
            Op::StructInsert {
                obj: 0,
                field: 0,
                value: 0,
            },
            Op::GetField {
                dst: 0,
                obj: 0,
                field: 0,
            },
            Op::NewInductive {
                dst: 0,
                type_name: 0,
                ctor: 0,
                args_start: 0,
                count: 0,
            },
            Op::TestArm {
                dst: 0,
                target: 0,
                variant: 0,
            },
            Op::BindArm {
                dst: 0,
                target: 0,
                field: 0,
                index: 0,
            },
            Op::CrdtBump {
                obj: 0,
                field: 0,
                amount: 0,
                negate: false,
            },
            Op::CrdtMerge {
                target: 0,
                source: 0,
            },
            Op::CrdtJoin {
                target: 0,
                source: 0,
            },
            Op::BigIntWide {
                kind: 0,
                dst: 0,
                a: 0,
                b: 0,
            },
            Op::NewCrdt { dst: 0, kind: 0 },
            Op::CrdtAppend { seq: 0, value: 0 },
            Op::CrdtResolve {
                obj: 0,
                field: 0,
                value: 0,
            },
            Op::IterPrepare { iterable: 0 },
            Op::IterNext { dst: 0, exit: 0 },
            Op::IterPop,
            Op::ListPop { list: 0, dst: 0 },
            Op::Sleep { duration: 0 },
            Op::DestructureTuple {
                src: 0,
                start: 0,
                count: 0,
            },
            Op::Show { src: 0 },
            Op::Args { dst: 0 },
            Op::ChanNew {
                dst: 0,
                cap: 0,
                elem: ChanElem::Unknown,
            },
            Op::ChanSend { chan: 0, val: 0 },
            Op::ChanRecv { dst: 0, chan: 0 },
            Op::ChanTrySend {
                dst: 0,
                chan: 0,
                val: 0,
            },
            Op::ChanTryRecv { dst: 0, chan: 0 },
            Op::ChanClose { chan: 0 },
            Op::Spawn {
                func: 0,
                args_start: 0,
                arg_count: 0,
            },
            Op::SpawnHandle {
                dst: 0,
                func: 0,
                args_start: 0,
                arg_count: 0,
            },
            Op::TaskAwait { dst: 0, handle: 0 },
            Op::TaskAbort { handle: 0 },
            Op::SelectArmRecv { chan: 0, var: 0 },
            Op::SelectArmTimeout { ticks: 0 },
            Op::SelectWait { dst_arm: 0 },
            Op::NetConnect { url: 0 },
            Op::NetListen { topic: 0 },
            Op::NetSend { to: 0, msg: 0 },
            Op::NetStream { to: 0, values: 0 },
            Op::NetAwait {
                dst: 0,
                from: 0,
                stream: false,
            },
            Op::NetMakePeer { dst: 0, addr: 0 },
            Op::NetSync { dst: 0, topic: 0 },
            Op::Tcp {
                dst: 0,
                kind: 0,
                args_start: 0,
                arg_count: 0,
            },
            Op::Serve {
                port: 0,
                handler: 0,
            },
            Op::FailWith { msg: 0 },
            Op::Obs {
                site: 0,
                args_start: 0,
                args_count: 0,
            },
            Op::Halt,
        ]
    }
}

/// A parameter's (or struct field's) declared type, RESOLVED at compile time into a self-contained
/// form a STATICALLY-typed consumer can use without the AST or the interner. The dynamically-typed
/// tree-walker/VM and the scalar-only native JIT never needed static types, so they were not put in
/// the bytecode before; they are now a first-class part of the compiled program (a struct is
/// referenced by name, its layout living in [`CompiledProgram::struct_types`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BoundaryType {
    Int,
    Float,
    Bool,
    Text,
    Date,
    Moment,
    /// A `Word32`/`Word64` wrapping-integer (ℤ/2ⁿ) — a native `i32`/`i64` scalar, so a `Seq of Word32`
    /// parameter (a crypto state array) resolves its element kind cross-region.
    Word32,
    Word64,
    /// `Seq of <elem>`.
    Seq(Box<BoundaryType>),
    /// A user struct, by name — its field layout is in [`CompiledProgram::struct_types`].
    Struct(String),
    /// A user enum (sum type), by name. Carried as one i32 handle whose word 0 is the constructor
    /// tag; `Inspect`/`TestArm` read the tag directly, so no separate layout is needed.
    Enum(String),
    /// `Result of <ok> and <err>` — an enum with no NAME, carrying its two payload types.
    ///
    /// ★ STRUCTURAL BECAUSE IT IS ANONYMOUS. Every other enum here is a name to look up in
    /// [`CompiledProgram::enum_types`], because a `## An X is one of:` declared it. A `Result`
    /// is written at its use site and never declared, and two instantiations in one program
    /// (`Result of Int and E` beside `Result of Text and F`) are different types that would
    /// collide under any single name — so the payload types are carried here rather than
    /// resolved through a table. Laid out exactly as [`BoundaryType::Enum`]: one i32 handle
    /// whose word 0 is the `Ok`/`Err` tag.
    Result(Box<BoundaryType>, Box<BoundaryType>),
    /// `Map of <key> to <value>` — one i32 handle. The key kind is recovered at each access from the
    /// key expression's own kind (no map metadata needed); the VALUE kind is carried here so a
    /// parameter map's `item k of m` resolves its result kind cross-region.
    Map(Box<BoundaryType>, Box<BoundaryType>),
    /// A HETEROGENEOUS tuple (`Pair`/`Triple` of mixed element types) — one i32 handle to a buffer of
    /// 8-byte slots, each holding its element at its own kind. The per-position element types are
    /// carried so a `item N of t` (constant `N`) resolves its result kind cross-region. (A homogeneous
    /// tuple resolves to [`BoundaryType::Seq`] instead — it shares the list buffer layout.)
    Tuple(Vec<BoundaryType>),
    /// A first-class BUILTIN value type by name (`Uuid`, `Duration`, `Time`, `Span`, `Money`,
    /// `Quantity`, `Rational`, `Complex`, `Decimal`, `Modular`) — a scalar/handle type whose wasm
    /// register kind is fixed by its name, not by a user-defined layout. Carried by name (like
    /// [`BoundaryType::Struct`]/[`BoundaryType::Enum`]) so a cross-region parameter/return of one of
    /// these types (`uuidV5(namespace: Uuid) -> Uuid`) seeds the correct handle kind; a name with no
    /// wasm kind resolves to `None` and is left soundly unmodeled.
    Builtin(String),
    /// `Set of <elem>` — one i32 handle to an insertion-ordered dedup'd buffer, exactly like the VM's
    /// hashset. The element type distinguishes the two self-describing set kinds: a `Set of Int` rides
    /// `Kind::Set` (i64 slots, dedup by value) and a `Set of Text` rides `Kind::SetText` (Text-handle
    /// slots, dedup by byte equality); other element types have no self-contained set kind and resolve
    /// to `None`. (Placed LAST so the pre-existing bincode variant indices stay stable.)
    Set(Box<BoundaryType>),
    /// `Option of <inner>` — an anonymous two-constructor enum (`Some` carrying one payload,
    /// `None` carrying nothing), carrying its payload type.
    ///
    /// ★ STRUCTURAL FOR THE SAME REASON AS `Result`, and absent for a long time because an Option
    /// used not to be a value at all: `some x` compiled to `x` and `none` to the `Nothing`
    /// sentinel, so there was no type to carry and `boundary_of_type_expr` fell to `_ => None`.
    /// Now that an Option is a real tagged inductive, the payload type has to reach the backends
    /// or they cannot emit a printer for a `When Some (v):` arm — which is a real refusal on wasm
    /// ("Show of an unknown-kind value") for an Option that happens to be absent, since that arm
    /// is still lowered even though it never runs.
    ///
    /// (Placed LAST so the pre-existing bincode variant indices stay stable — the same rule `Set`
    /// records above.)
    Option(Box<BoundaryType>),
}

/// A struct type's resolved field layout (fields in declaration / slot order), carried in the
/// bytecode so any consumer can address a struct's fields without re-deriving them from the AST.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructTypeDef {
    pub name: String,
    pub fields: Vec<(String, BoundaryType)>,
}

/// One variant of an enum: its constructor name and the resolved types of its payload fields, in
/// position order (empty for a nullary variant). A `BindArm` extracts payload position `index`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnumVariantDef {
    pub name: String,
    pub field_types: Vec<BoundaryType>,
}

/// An enum type's resolved per-variant payload layout, carried in the bytecode so a consumer can
/// type a `When V (binds)` payload extraction on a value whose construction isn't visible (an enum
/// PARAMETER) — the sum-type analog of [`StructTypeDef`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnumTypeDef {
    pub name: String,
    pub variants: Vec<EnumVariantDef>,
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
    /// Each parameter's FULL declared type, resolved at compile time (`None` for a closure body —
    /// no declarations — or a type the resolver does not model). Unlike `param_kinds` (a scalar/list
    /// performance hint for the native tier), this carries struct/Text/etc. so a statically-typed
    /// consumer can type a `f(p: Point)` / `f(s: Text)` parameter from the bytecode alone.
    pub param_types: Vec<Option<BoundaryType>>,
    /// The function's FULL declared RETURN type, resolved at compile time (`None` if undeclared or
    /// unmodeled). Carries struct/map/enum/etc. so a caller can type the inline use of a returned
    /// composite (`item k of f()`, `Inspect f()`) the same way a parameter of that type is typed.
    pub return_type: Option<BoundaryType>,
    /// Argument registers retained on return so an in-out call can copy the
    /// final values back to the caller. Includes inferred mutable-borrow slots.
    pub mutable_param_regs: Vec<Reg>,
    /// Registers proven to be consumed-and-returned borrows. Only these may
    /// bypass copy-on-write; an explicit mutable parameter may make local copies.
    pub cow_exempt_regs: Vec<Reg>,
}

/// What an [`Op::Obs`] emits, resolved at COMPILE time (Wave 13). A `Message`
/// carries the speech-act category, the that-clause text, and the payload NAMES
/// (values live in the op's register range); `SpanEnter`/`SpanExit` carry the
/// span label. Kept off the hot loop — the machine only reads it.
#[derive(Clone, Debug)]
pub enum ObsSiteKind {
    Message(crate::observe::ObsCat),
    SpanEnter,
    SpanExit,
}

/// One compiled observation site (indexed by `Op::Obs { site }`).
#[derive(Clone, Debug)]
pub struct ObsSiteDesc {
    pub kind: ObsSiteKind,
    pub message: String,
    pub payload_names: Vec<String>,
    pub sample: Option<u64>,
}

/// DEBUG-ONLY names for one compiled function, for the Studio debug drawer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FnDebugInfo {
    /// The function's source name, for the call stack.
    pub name: String,
    /// Register index → source variable name (parameters and locals) in its frame.
    pub reg_names: Vec<(u16, String)>,
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
    /// DEBUG-ONLY: each function's source name and register → variable names, indexed like
    /// `functions`. Populated only by the debugger's compile path, like `reg_names`; empty on
    /// every production build.
    pub fn_debug: Vec<FnDebugInfo>,
    /// The program's struct type definitions (name → field layout), resolved at compile time. The
    /// bytecode's static type registry — the dynamically-typed tree-walker/VM never needed it, but a
    /// statically-typed consumer (the AOT backend) addresses a struct's fields through it.
    pub struct_types: Vec<StructTypeDef>,
    /// The program's enum type definitions (name → per-variant payload layout), resolved at compile
    /// time — the sum-type companion to `struct_types`. Lets the AOT backend type a `When V (binds)`
    /// payload extraction on an enum whose construction isn't in scope (an enum PARAMETER).
    pub enum_types: Vec<EnumTypeDef>,
    /// Each promoted GLOBAL's resolved composite type (by global index; `None` for scalar /
    /// self-describing / unmodeled). Lets the AOT type a closure that CAPTURES a composite global with
    /// the value's shape — the capture analog of a parameter's declared type.
    pub global_types: Vec<Option<BoundaryType>>,
    /// ★ WHAT THE PROGRAM DECLARES A VALUE TO BE, by the absolute pc of the op that defines it,
    /// sorted by pc. `a new Seq of Todo`, `a new Map of Text to Int` and `a new Set of Text` —
    /// wherever they are written, a constructor argument included — name their element types;
    /// `Let x: Seq of Text be …` and a `Let` of a call whose function declares `-> Seq of Todo` name
    /// the type of the value the binding's `Move` lands. A collection built EMPTY has no element
    /// to trace, so a statically-typed backend takes its kind from here, and infers only where
    /// nothing is declared.
    ///
    /// A side table rather than a field on the ops: the ops are matched exhaustively by every
    /// backend and catalog, and only the typed backends read this. Keyed by op position rather than
    /// register, because a backend renumbers registers before it reads it; every pass that moves
    /// ops (`peephole`'s `remove_ops`) remaps it the way it remaps jump targets, and a backend that
    /// lowers one region reads it through that region's own op mapping.
    pub declared_types: Vec<(u32, BoundaryType)>,
    /// Observation sites (Wave 13), indexed by `Op::Obs { site }`. EMPTY in the
    /// elided (compiled/release) profile — populated only when the VM compiles in
    /// the OBSERVED profile (`Compiler::compile_observed`), so ordinary programs
    /// pay nothing and `Op::Obs` never reaches the AOT/WASM backends.
    pub observation_sites: Vec<ObsSiteDesc>,
    /// `(pc, site_id)` pairs, sorted by `pc`, one entry per compiled function (Main included) at
    /// its `entry_pc` plus one per `Stmt::Site` marker the parser inserted — Wave 3: "runtime
    /// failures carry a location". A runtime error at pc `p` looks up the LAST entry with
    /// `pc <= p` (`partition_point`) for the site in force; every function opens with
    /// `NO_SITE` so a trap in the few ops before that function's own first author sentence
    /// (parameter binding, capture setup) does not inherit whatever site was active in the
    /// PREVIOUS function's own code, since `code` is one array (Main, then every function body).
    /// `site_id` indexes the parser's own `failure_sites()` table — this program does not carry
    /// that table itself (it lives beside the parsed AST, not the compiled one); a caller wanting
    /// a rendered location combines the two.
    pub site_at: Vec<(u32, u32)>,
}

/// The `site_id` a pc range with no active `Stmt::Site` marker carries — append-only, like every
/// other wire-visible sentinel in this file.
pub const NO_SITE: u32 = u32::MAX;

// ───────────────────────── register operands and control successors ─────────────────────────
//
// The ONE description of what each op does to registers and to control flow. The wasm backend's
// live-range splitter renames through it, the wasm place-mutation and heap analyses read op
// footprints through it, and the VM's copy-on-write asks it which registers are dead. A new `Op`
// variant fails to build until `visit_regs` classifies it.

/// How an op touches a register operand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Role {
    /// Written (the prior value is dead).
    Def,
    /// Read.
    Use,
    /// Read AND written through the SAME single field (`x += …`) — must rename to one local.
    DefUse,
}

/// A visitor over an op's register operands. `scalar` is a single register field; `range` is a
/// `count`-wide block of consecutive registers starting at `*start` (the members past `*start` are
/// implicit — not fields — so only `*start` is handed over, but the whole block is pinned).
pub(crate) trait RegVisitor {
    fn scalar(&mut self, r: &mut Reg, role: Role);
    fn range(&mut self, start: &mut Reg, count: u16, role: Role);
    /// A register block that must stay CONTIGUOUS only when it has ≥2 members — a range of ONE has
    /// no positional constraint, so its lone operand is a plain renamable scalar (the op reads it
    /// through `args_start`, which the rename updates). Visiting it as a scalar lets a single-arg
    /// call/builtin operand split off a register that a wider call reuses (spectral_norm's `Sqrt`
    /// arg sharing a slot with a 4-arg call's `Seq of Float` argument).
    fn members(&mut self, start: &mut Reg, count: u16, role: Role) {
        if count == 1 {
            self.scalar(start, role);
        } else {
            self.range(start, count, role);
        }
    }
}

/// THE exhaustive enumeration of every op's register operands (compiler-enforced total match: a new
/// `Op` variant fails to build until classified here). `functions` resolves a `MakeClosure`'s
/// capture count, whose register range is not stored in the op. Takes `&mut Op` so the same routine
/// drives both read-only analysis and in-place renaming.
pub(crate) fn visit_regs(op: &mut Op, functions: &[CompiledFunction], v: &mut dyn RegVisitor) {
    use Role::{Def, DefUse, Use};
    match op {
        Op::LoadConst { dst, .. } => v.scalar(dst, Def),
        Op::Move { dst, src } => {
            v.scalar(src, Use);
            v.scalar(dst, Def);
        }
        // Register swap: both operands are read AND written in place.
        Op::RevSwap { a, b } => {
            v.scalar(a, DefUse);
            v.scalar(b, DefUse);
        }
        // In-place XOR: `place` is read+written, `delta` is read-only.
        Op::RevXor { place, delta } => {
            v.scalar(delta, Use);
            v.scalar(place, DefUse);
        }
        // In-place modular negation: single operand, read+written.
        Op::RevNeg { place } => v.scalar(place, DefUse),
        // In-place modular add/subtract: `place` read+written, `delta` read-only.
        Op::RevAdd { place, delta } | Op::RevSub { place, delta } => {
            v.scalar(delta, Use);
            v.scalar(place, DefUse);
        }
        Op::EnsureOwned { reg } => v.scalar(reg, Role::DefUse),
        Op::Add { dst, lhs, rhs }
        | Op::Sub { dst, lhs, rhs }
        | Op::Mul { dst, lhs, rhs }
        | Op::Pow { dst, lhs, rhs }
        | Op::Div { dst, lhs, rhs }
        | Op::ExactDiv { dst, lhs, rhs }
        | Op::FloorDiv { dst, lhs, rhs }
        | Op::Mod { dst, lhs, rhs }
        | Op::Lt { dst, lhs, rhs }
        | Op::Gt { dst, lhs, rhs }
        | Op::LtEq { dst, lhs, rhs }
        | Op::GtEq { dst, lhs, rhs }
        | Op::Eq { dst, lhs, rhs }
        | Op::NotEq { dst, lhs, rhs }
        | Op::ApproxEq { dst, lhs, rhs }
        | Op::Concat { dst, lhs, rhs }
        | Op::SeqConcat { dst, lhs, rhs }
        | Op::BitXor { dst, lhs, rhs }
        | Op::BitAnd { dst, lhs, rhs }
        | Op::BitOr { dst, lhs, rhs }
        | Op::Shl { dst, lhs, rhs }
        | Op::Shr { dst, lhs, rhs }
        | Op::UnionOp { dst, lhs, rhs }
        | Op::IntersectOp { dst, lhs, rhs } => {
            v.scalar(lhs, Use);
            v.scalar(rhs, Use);
            v.scalar(dst, Def);
        }
        Op::AddAssign { dst, src } => {
            v.scalar(src, Use);
            v.scalar(dst, DefUse);
        }
        Op::DivPow2 { dst, lhs, .. } | Op::MagicDivU { dst, lhs, .. } => {
            v.scalar(lhs, Use);
            v.scalar(dst, Def);
        }
        Op::Not { dst, src } | Op::DeepClone { dst, src } => {
            v.scalar(src, Use);
            v.scalar(dst, Def);
        }
        Op::Jump { .. } => {}
        Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. } => v.scalar(cond, Use),
        Op::Call {
            dst,
            func,
            args_start,
            arg_count,
        } => {
            v.members(args_start, *arg_count, Use);
            // A `mutable` parameter's final value comes back in its argument register, which the caller copies
            // into the place it lent (`CompiledFunction::mutable_param_regs`): the call reads and writes it.
            if let Some(f) = functions.get(*func as usize) {
                for &p in f.mutable_param_regs.iter().filter(|p| **p < *arg_count) {
                    let mut member = args_start.wrapping_add(p);
                    v.scalar(&mut member, DefUse);
                }
            }
            v.scalar(dst, Def);
        }
        Op::CallBuiltin {
            dst,
            args_start,
            arg_count,
            ..
        } => {
            v.members(args_start, *arg_count, Use);
            v.scalar(dst, Def);
        }
        Op::CallValue {
            dst,
            callee,
            args_start,
            arg_count,
            ..
        } => {
            v.scalar(callee, Use);
            v.members(args_start, *arg_count, Use);
            v.scalar(dst, Def);
        }
        Op::MakeClosure {
            dst,
            func,
            locals_start,
        } => {
            let cap_n = functions
                .get(*func as usize)
                .map_or(0, |f| f.captures.len() as u16);
            v.members(locals_start, cap_n, Use);
            v.scalar(dst, Def);
        }
        Op::CheckPolicy {
            subject, object, ..
        } => {
            v.scalar(subject, Use);
            if *object != Reg::MAX {
                v.scalar(object, Use);
            }
        }
        Op::ListPushField { obj, src, .. } => {
            v.scalar(obj, Use);
            v.scalar(src, Use);
        }
        Op::GlobalGet { dst, .. } => v.scalar(dst, Def),
        Op::GlobalSet { src, .. } => v.scalar(src, Use),
        Op::Return { src } => v.scalar(src, Use),
        Op::ReturnNothing => {}
        Op::NewList { dst, start, count } | Op::NewTuple { dst, start, count } => {
            v.members(start, *count, Use);
            v.scalar(dst, Def);
        }
        Op::NewEmptyList { dst }
        | Op::NewEmptyListFixed { dst, .. }
        | Op::NewEmptyListI32 { dst }
        | Op::NewEmptySet { dst }
        | Op::NewEmptyMap { dst }
        | Op::LoadToday { dst }
        | Op::LoadNow { dst }
        | Op::NewStruct { dst, .. }
        | Op::NewCrdt { dst, .. }
        | Op::Args { dst }
        | Op::ChanNew { dst, .. }
        | Op::SelectWait { dst_arm: dst } => v.scalar(dst, Def),
        Op::NewEmptyListWithCapacity { dst, capacity }
        | Op::NewEmptyMapWithCapacity { dst, capacity } => {
            v.scalar(capacity, Use);
            v.scalar(dst, Def);
        }
        Op::NewRange { dst, start, end } => {
            v.scalar(start, Use);
            v.scalar(end, Use);
            v.scalar(dst, Def);
        }
        Op::ListPush { list, value } => {
            v.scalar(list, Use);
            v.scalar(value, Use);
        }
        Op::SetAdd { set, value } => {
            v.scalar(set, Use);
            v.scalar(value, Use);
        }
        Op::RemoveFrom { collection, value } => {
            v.scalar(collection, Use);
            v.scalar(value, Use);
        }
        Op::Contains {
            dst,
            collection,
            value,
        } => {
            v.scalar(collection, Use);
            v.scalar(value, Use);
            v.scalar(dst, Def);
        }
        Op::SetIndex {
            collection,
            index,
            value,
        }
        | Op::SetIndexUnchecked {
            collection,
            index,
            value,
        } => {
            v.scalar(collection, Use);
            v.scalar(index, Use);
            v.scalar(value, Use);
        }
        Op::Index {
            dst,
            collection,
            index,
        }
        | Op::IndexUnchecked {
            dst,
            collection,
            index,
        } => {
            v.scalar(collection, Use);
            v.scalar(index, Use);
            v.scalar(dst, Def);
        }
        Op::RegionBoundsGuard {
            array, bound, iv, ..
        } => {
            v.scalar(array, Use);
            v.scalar(bound, Use);
            v.scalar(iv, Use);
        }
        Op::Length { dst, collection } => {
            v.scalar(collection, Use);
            v.scalar(dst, Def);
        }
        Op::FormatValue { dst, src, .. } => {
            v.scalar(src, Use);
            v.scalar(dst, Def);
        }
        Op::SliceOp {
            dst,
            collection,
            start,
            end,
        } => {
            v.scalar(collection, Use);
            v.scalar(start, Use);
            v.scalar(end, Use);
            v.scalar(dst, Def);
        }
        Op::StructInsert { obj, value, .. } => {
            v.scalar(obj, Use);
            v.scalar(value, Use);
        }
        Op::GetField { dst, obj, .. } => {
            v.scalar(obj, Use);
            v.scalar(dst, Def);
        }
        Op::NewInductive {
            dst,
            args_start,
            count,
            ..
        } => {
            v.members(args_start, *count, Use);
            v.scalar(dst, Def);
        }
        Op::TestArm { dst, target, .. } | Op::BindArm { dst, target, .. } => {
            v.scalar(target, Use);
            v.scalar(dst, Def);
        }
        Op::CrdtBump { obj, amount, .. } => {
            v.scalar(obj, Use);
            v.scalar(amount, Use);
        }
        Op::CrdtMerge { target, source } | Op::CrdtJoin { target, source } => {
            v.scalar(target, Use);
            v.scalar(source, Use);
        }
        Op::BigIntWide { dst, a, b, .. } => {
            v.scalar(a, Use);
            v.scalar(b, Use);
            v.scalar(dst, Def);
        }
        Op::CrdtAppend { seq, value } => {
            v.scalar(seq, Use);
            v.scalar(value, Use);
        }
        Op::CrdtResolve { obj, value, .. } => {
            v.scalar(obj, Use);
            v.scalar(value, Use);
        }
        Op::IterPrepare { iterable } => v.scalar(iterable, Use),
        Op::IterNext { dst, .. } => v.scalar(dst, Def),
        Op::IterPop => {}
        Op::ListPop { list, dst } => {
            v.scalar(list, Use);
            v.scalar(dst, Def);
        }
        Op::Sleep { duration } => v.scalar(duration, Use),
        Op::DestructureTuple { src, start, count } => {
            v.scalar(src, Use);
            v.members(start, *count, Def);
        }
        Op::Show { src } => v.scalar(src, Use),
        Op::ChanSend { chan, val } => {
            v.scalar(chan, Use);
            v.scalar(val, Use);
        }
        Op::ChanRecv { dst, chan } | Op::ChanTryRecv { dst, chan } => {
            v.scalar(chan, Use);
            v.scalar(dst, Def);
        }
        Op::ChanTrySend { dst, chan, val } => {
            v.scalar(chan, Use);
            v.scalar(val, Use);
            v.scalar(dst, Def);
        }
        Op::ChanClose { chan } => v.scalar(chan, Use),
        Op::Spawn {
            args_start,
            arg_count,
            ..
        } => v.members(args_start, *arg_count, Use),
        Op::SpawnHandle {
            dst,
            args_start,
            arg_count,
            ..
        } => {
            v.members(args_start, *arg_count, Use);
            v.scalar(dst, Def);
        }
        Op::TaskAwait { dst, handle } => {
            v.scalar(handle, Use);
            v.scalar(dst, Def);
        }
        Op::TaskAbort { handle } => v.scalar(handle, Use),
        Op::SelectArmRecv { chan, var } => {
            v.scalar(chan, Use);
            v.scalar(var, Def);
        }
        Op::SelectArmTimeout { ticks } => v.scalar(ticks, Use),
        Op::NetConnect { url } => v.scalar(url, Use),
        Op::NetListen { topic } => v.scalar(topic, Use),
        Op::NetSend { to, msg } => {
            v.scalar(to, Use);
            v.scalar(msg, Use);
        }
        Op::NetStream { to, values } => {
            v.scalar(to, Use);
            v.scalar(values, Use);
        }
        Op::NetAwait { dst, from, .. } => {
            v.scalar(from, Use);
            v.scalar(dst, Def);
        }
        Op::NetMakePeer { dst, addr } => {
            v.scalar(addr, Use);
            v.scalar(dst, Def);
        }
        Op::NetSync { dst, topic } => {
            v.scalar(topic, Use);
            v.scalar(dst, DefUse);
        }
        Op::Tcp {
            dst,
            args_start,
            arg_count,
            ..
        } => {
            v.members(args_start, *arg_count, Use);
            v.scalar(dst, Def);
        }
        // `Serve` reads the port register; the handler rides a `FuncIdx`, not a register.
        Op::Serve { port, .. } => v.scalar(port, Use),
        // An observation reads its payload registers (the elided profile never emits one).
        Op::Obs {
            args_start,
            args_count,
            ..
        } => v.members(args_start, *args_count, Use),
        Op::FailWith { .. } | Op::Halt => {}
    }
}

/// Every register an op DEFINES and every register it USES (register ranges expanded to members),
/// for any analysis that needs an op's full data-flow footprint and must not miss
/// an operand — it reuses the same exhaustive [`visit_regs`] enumeration, so a new op cannot slip
/// past. A `DefUse` operand appears in both lists.
pub(crate) fn op_def_uses(op: &Op, functions: &[CompiledFunction]) -> (Vec<Reg>, Vec<Reg>) {
    struct Collect {
        defs: Vec<Reg>,
        uses: Vec<Reg>,
    }
    impl RegVisitor for Collect {
        fn scalar(&mut self, r: &mut Reg, role: Role) {
            match role {
                Role::Def => self.defs.push(*r),
                Role::Use => self.uses.push(*r),
                Role::DefUse => {
                    self.defs.push(*r);
                    self.uses.push(*r);
                }
            }
        }
        fn range(&mut self, start: &mut Reg, count: u16, role: Role) {
            for i in 0..count {
                self.scalar(&mut start.wrapping_add(i), role);
            }
        }
    }
    let mut tmp = op.clone();
    let mut c = Collect {
        defs: Vec::new(),
        uses: Vec::new(),
    };
    visit_regs(&mut tmp, functions, &mut c);
    (c.defs, c.uses)
}

/// The pcs control can reach from the op at `pc`, in the coordinate system of the op's own jump
/// targets (absolute over `program.code`, or region-relative after a backend rebases a region), with
/// fallthrough only to pcs below `end`. Every non-control-flow op falls through, so the explicit arms
/// are exactly the four ops that carry a target and the ops that never continue.
pub(crate) fn op_successors(op: &Op, pc: usize, end: usize) -> Vec<usize> {
    let fallthrough = |out: &mut Vec<usize>| {
        if pc + 1 < end {
            out.push(pc + 1);
        }
    };
    let mut out = Vec::new();
    match *op {
        Op::Jump { target } => out.push(target),
        Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
            out.push(target);
            fallthrough(&mut out);
        }
        Op::IterNext { exit, .. } => {
            out.push(exit);
            fallthrough(&mut out);
        }
        Op::Return { .. } | Op::ReturnNothing | Op::Halt | Op::FailWith { .. } => {}
        // `Fail with …` / a broken contract's reason never returns.
        Op::CallBuiltin {
            builtin:
                crate::semantics::builtins::BuiltinId::Fail
                | crate::semantics::builtins::BuiltinId::FailReason,
            ..
        } => {}
        _ => fallthrough(&mut out),
    }
    out
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/instruction/concurrency_op_tests.rs"]
mod concurrency_op_tests;
