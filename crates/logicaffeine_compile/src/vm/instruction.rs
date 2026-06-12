//! Bytecode instruction set and the compiled-program container.
//!
//! Registers are per-frame `u16` indices (the compiler assigns locals to
//! registers at compile time; frames are capped at `MAX_REGISTERS_PER_FRAME`).
//! Jump targets are absolute instruction indices. This is the growing core of
//! VM_PLAN.md's 87-opcode set.

use std::collections::HashMap;

use crate::intern::Symbol;

pub type Reg = u16;
pub type ConstIdx = u32;
pub type FuncIdx = u16;

/// A constant-pool entry.
#[derive(Clone, Debug)]
pub enum Constant {
    Int(i64),
    Float(f64),
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

/// A bytecode instruction.
#[derive(Clone, Debug)]
pub enum Op {
    /// `R[dst] = constants[idx]`
    LoadConst { dst: Reg, idx: ConstIdx },
    /// `R[dst] = R[src]` (shallow clone)
    Move { dst: Reg, src: Reg },

    Add { dst: Reg, lhs: Reg, rhs: Reg },
    Sub { dst: Reg, lhs: Reg, rhs: Reg },
    Mul { dst: Reg, lhs: Reg, rhs: Reg },
    Div { dst: Reg, lhs: Reg, rhs: Reg },
    Mod { dst: Reg, lhs: Reg, rhs: Reg },

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
    /// `R[dst] = R[collection][R[index]]` (1-based for ordered collections).
    Index { dst: Reg, collection: Reg, index: Reg },
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
}
