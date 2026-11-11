//! The RV64GC emitter. Walks the [`CompiledProgram`] region by region (Main first, then the body of each
//! function the program can enter) and lowers every op to assembly, using the stack-slot register model
//! documented in the module header. Anything outside the current phase returns
//! [`RiscVLowerError::Unsupported`].

use crate::analysis::{PolicyCondition, PolicyRegistry};
use crate::intern::Interner;
use crate::semantics::builtins::BuiltinId;
use crate::vm::{BoundaryType, ChanElem, CompiledProgram, Constant, Op};

use super::rodata::RoData;
use super::RiscVLowerError;

mod tower;

type Emit = Result<(), RiscVLowerError>;

/// The static kind of the value in a register at a program point — enough to pick Show formatting and
/// to refuse (soundly) an operation on a value family a phase does not yet lower (e.g. float arithmetic
/// in Phase 1). Inferred by a small forward fixpoint over each region's ops.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Int,
    Bool,
    Float,
    /// An exact rational: a handle to an `LDec` of the shim's exact numbers, which follow the VM's rules — a whole
    /// rational answers `/`, `%`, `//`, `**` and the bitwise operators as the integer it equals. Produced by `ExactDiv`
    /// in a `Rational` context; arithmetic with an Int promotes the Int. `Show` renders `num/den`, or the integer a
    /// whole one equals.
    Rational,
    /// A heap Text handle (a `char*` in a slot).
    Text,
    /// A `Date` (days since epoch, in an i64 slot) — `today`. `Show` renders `YYYY-MM-DD`.
    Date,
    /// A `Moment` (nanoseconds since epoch, in an i64 slot) — `now`. `Show` renders `YYYY-MM-DD HH:MM`.
    Moment,
    /// A `Duration` (nanoseconds, in an i64 slot) — `2 milliseconds`, the difference of two instants. Not shown.
    Duration,
    /// A `Span` (`2 weeks`, `3 months`): its calendar months in the high and its days in the low 32 bits of an i64
    /// slot.
    Span,
    /// A `PeerAgent` — the canonical relay topic it names (a `char*`). `Show` renders `<peer topic>`.
    Peer,
    /// A counter CRDT handle (a `logos_crdt*` = per-replica count map). Lives in a struct field whose
    /// DECLARED type is `Int`; `GetField` on such a field SUMS the map to an `Int`, and `CrdtMerge`/
    /// `CrdtJoin` take the per-replica MAX (idempotent). Produced by `NewCrdt`.
    CrdtCounter,
    /// A heap sequence handle (a `logos_seq*` in a slot), by scalar element kind.
    SeqInt,
    SeqFloat,
    SeqText,
    /// A `Seq of Bool` — storage-identical to `SeqInt` (each cell an i64 0/1); only `Show` differs
    /// (elements render `true`/`false`, matching the VM's `to_display_string`).
    SeqBool,
    /// A `Word32` — a wrapping 32-bit unsigned integer (ℤ/2³²), held zero-extended in an i64 slot
    /// (0..2³²−1). Arithmetic masks to 32 bits after each op (the VM's `WordVal::W32` ring).
    Word32,
    /// A `Seq of Word32` — storage-identical to `SeqInt` (each cell a zero-extended 32-bit value).
    SeqWord32,
    /// A heap set handle (insertion-ordered, dedup'd — shares the `logos_seq` storage), by element kind.
    SetInt,
    SetText,
    /// A composite kind (Map key+value, or heterogeneous Tuple per-position) — the `u32` indexes the
    /// emitter's composite side-table, so `Kind` stays `Copy` (essential for the per-pc kind snapshots).
    Composite(u32),
    /// A sequence/set/map handle whose element kind isn't yet known (freshly created, not yet added-to)
    /// or is a kind this phase does not lower (nested elements) — Show/element ops refuse it.
    SeqUnknown,
    SetUnknown,
    MapUnknown,
    /// A value of the exact extension (an infinitesimal, a hyperreal, an algebraic number, an ordinal, a
    /// residue-dependent or provisional answer, and every list of them): its `LTower` cell, a collected cell around a
    /// handle into the linked value runtime, in the slot (`tower.c`).
    Tower,
    /// Not a scalar this phase reasons about (a heap handle, or a merge of disagreeing kinds).
    Unknown,
}

/// Whether `prog` makes or declares a rational: an exact quotient, or a parameter, result, field or global declared
/// one.
fn holds_rationals(prog: &CompiledProgram) -> bool {
    fn names(t: &BoundaryType) -> bool {
        match t {
            BoundaryType::Builtin(name) => name == "Rational",
            BoundaryType::Seq(e) | BoundaryType::Set(e) | BoundaryType::Option(e) => names(e),
            BoundaryType::Map(k, v) | BoundaryType::Result(k, v) => names(k) || names(v),
            BoundaryType::Tuple(ts) => ts.iter().any(names),
            _ => false,
        }
    }
    let declared = |t: &Option<BoundaryType>| t.as_ref().is_some_and(names);
    prog.code.iter().any(|op| matches!(op, Op::ExactDiv { .. }))
        || prog.functions.iter().any(|f| declared(&f.return_type) || f.param_types.iter().any(declared))
        || prog.struct_types.iter().any(|s| s.fields.iter().any(|(_, t)| names(t)))
        || prog.global_types.iter().any(declared)
}

/// The rounds a region's exact webs and number joins take to hold still before the region is refused: each round only
/// widens what a variable holds, and no chain of widenings is this long.
const SETTLE_ROUNDS: usize = 64;

/// What one round of [`Emitter::infer_kinds_forced`] settles: the kinds before each op, the definitions that land as
/// the extension's value and the kind each such one computes, the kind each definition writes, and the kind each
/// definition a number join lands computes before it lands.
struct Inferred {
    per_pc: std::collections::HashMap<usize, Vec<Kind>>,
    lands: std::collections::BTreeSet<usize>,
    natural: std::collections::HashMap<usize, Kind>,
    written: std::collections::HashMap<usize, Kind>,
    number_natural: std::collections::HashMap<usize, Kind>,
}

/// How a region's number variables are held ([`crate::vm::numbers`]): the kind each definition lands as, the kind
/// each register taken in is converted to, and one definition of each variable no machine form holds.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct NumberLandings {
    defs: std::collections::BTreeMap<usize, Kind>,
    entries: std::collections::BTreeMap<u16, Kind>,
    refused: Vec<crate::vm::variables::Site>,
}

/// A composite kind stored in the emitter's side-table (referenced by [`Kind::Composite`]).
#[derive(Clone, PartialEq, Eq)]
enum Composite {
    /// A map handle: key kind, value kind.
    Map(Kind, Kind),
    /// A tuple handle: the per-position kinds (heterogeneous).
    Tuple(Vec<Kind>),
    /// A struct handle, by type name — the field layout (order + kinds) lives in `prog.struct_types`.
    Struct(String),
    /// An enum (inductive) handle, by type name — variant payload types live in `prog.enum_types`.
    /// Stored as `[constructor tag, payload…]` cells ([`Emitter::ctor_tag`]).
    Enum(String),
    /// `Option of T` — the anonymous `Some`/`None` enum, laid out as any enum, carrying `Some`'s payload
    /// kind (it is never declared, so no `enum_types` entry types it). A missing kind means that
    /// constructor is absent; a present `Kind::Unknown` still needs a modeled runtime layout.
    Option(Option<Kind>),
    /// `Result of T and E` — each possible constructor carries its own payload kind.
    Result(Option<Kind>, Option<Kind>),
    /// A closure over function body `func`: a call through it answers that body's return kind.
    Closure(u16),
    /// A channel handle (`Pipe of T`) carrying its element kind, so `Receive`'s bound variable gets the
    /// right kind. The handle is a `logos_chan*`.
    Chan(Kind),
    /// A sequence whose elements are themselves a composite (a `Seq of (Int, Int)` → `Seq(Tuple[..])`,
    /// or a nested `Seq of Seq of Int`). Rides the same `logos_seq` of 8-byte cells as any sequence —
    /// each cell is the element handle — so iteration/index share the scalar-seq machinery; the inner
    /// kind is only needed to give the bound/extracted element its composite kind (e.g. so a tuple
    /// for-each can `DestructureTuple` it).
    Seq(Kind),
}

/// The element kind of a sequence/set kind (Unknown for a non-collection or element-unknown collection).
fn elem_of(coll: Kind) -> Kind {
    match coll {
        Kind::SeqInt | Kind::SetInt => Kind::Int,
        Kind::SeqFloat => Kind::Float,
        Kind::SeqText | Kind::SetText => Kind::Text,
        Kind::SeqBool => Kind::Bool,
        Kind::SeqWord32 => Kind::Word32,
        _ => Kind::Unknown,
    }
}

/// The sequence kind whose elements are `elem` (SeqUnknown for a non-scalar/unsupported element kind).
fn seq_of(elem: Kind) -> Kind {
    match elem {
        Kind::Int => Kind::SeqInt,
        Kind::Float => Kind::SeqFloat,
        Kind::Text => Kind::SeqText,
        Kind::Bool => Kind::SeqBool,
        Kind::Word32 => Kind::SeqWord32,
        _ => Kind::SeqUnknown,
    }
}

/// The set kind whose elements are `elem` (SetUnknown for an unsupported element kind — Float sets later).
fn set_of(elem: Kind) -> Kind {
    match elem {
        Kind::Int => Kind::SetInt,
        Kind::Text => Kind::SetText,
        _ => Kind::SetUnknown,
    }
}

fn is_concrete_seq(k: Kind) -> bool {
    matches!(
        k,
        Kind::SeqInt | Kind::SeqFloat | Kind::SeqText | Kind::SeqBool | Kind::SeqWord32
    )
}

fn is_concrete_set(k: Kind) -> bool {
    matches!(k, Kind::SetInt | Kind::SetText)
}

/// A concrete sequence OR set — both back `Contains` and for-each iteration (shared storage).
fn is_collection(k: Kind) -> bool {
    is_concrete_seq(k) || is_concrete_set(k)
}

/// The `logos_val_to_str` tag of a value-or-nothing register holding `nothing`.
const NOTHING_TAG: u8 = 7;
/// The `logos_val_to_str` tag of a `PeerAgent`.
const PEER_TAG: u8 = 8;
/// The `logos_val_to_str` tag of a cell of the exact extension.
const TOWER_TAG: u8 = 9;

/// The runtime type tag for `logos_val_to_str` / `logos_concat_vals` (0=Int, 1=Float, 2=Bool, 3=Text, 4=Rational,
/// 5=Date, 6=Moment, 8=PeerAgent).
fn kind_tag(kind: Kind) -> Result<u8, RiscVLowerError> {
    match kind {
        Kind::Int | Kind::Word32 => Ok(0),
        Kind::Float => Ok(1),
        Kind::Bool => Ok(2),
        Kind::Text => Ok(3),
        Kind::Rational => Ok(4),
        Kind::Date => Ok(5),
        Kind::Moment => Ok(6),
        Kind::Peer => Ok(PEER_TAG),
        Kind::Tower => Ok(TOWER_TAG),
        Kind::Span
        | Kind::Duration
        | Kind::SeqInt
        | Kind::SeqFloat
        | Kind::SeqText
        | Kind::SeqBool
        | Kind::SeqWord32
        | Kind::SetInt
        | Kind::SetText
        | Kind::Composite(_)
        | Kind::CrdtCounter
        | Kind::SeqUnknown
        | Kind::SetUnknown
        | Kind::MapUnknown
        | Kind::Unknown => Err(RiscVLowerError::Unsupported(
            "text conversion of a non-scalar value",
        )),
    }
}

/// The tag a map compares a key of kind `kind` by: its `logos_val_to_str` tag. A value of the exact extension has
/// no machine comparison, so a map keyed by one is refused.
fn map_key_tag(kind: Kind) -> Result<u8, RiscVLowerError> {
    if kind == Kind::Tower {
        return Err(RiscVLowerError::Unsupported("a map keyed by a value of the exact extension"));
    }
    kind_tag(kind)
}

/// One straight-line region of the shared `code` vector: either `Main` (the C `main`) or a function
/// body. `start..end` are absolute pc indices; `frame_regs`/`params` size the stack frame; `param_kinds`
/// seeds kind inference for the incoming parameter slots.
struct Region {
    label: String,
    /// The pc intervals this region OWNS, in execution order. Usually one `[entry, end)`, but a region
    /// that defines an inline closure has a HOLE where the closure body sits (the closure is its own
    /// region), so it owns several disjoint intervals. The prologue/label go at `intervals[0].0`.
    intervals: Vec<(usize, usize)>,
    frame_regs: usize,
    params: u16,
    param_kinds: Vec<Kind>,
    is_main: bool,
    /// For a closure-body function: the kind of each captured value (empty for a non-closure). The body
    /// receives the closure object pointer as a hidden arg after its params and, in its prologue, loads
    /// the captures into slots `params..params+captures.len()` (values) and the present flags into
    /// `params+captures.len()..params+2*captures.len()`. Filled by [`Emitter::derive_capture_kinds`].
    captures: Vec<Kind>,
    /// What never runs, and why ([`Emitter::never_runs`]).
    never: NeverRuns,
}

/// How a region keeps its records' fields their own ([`Mutation::Field`]). Each `(register, cell)` a
/// field-mutating op changes has a FIELD flag in the frame: 1 while the register owns its record and nothing
/// but the record holds the value in that cell, so the op changes it in place; 0 otherwise, so the op copies
/// it first and sets the flag. A register that reads a cell's value out of the record (`GetField`, an arm's
/// binding, `item "f" of r`) aliases it: while an alias is live at a field op, the op copies (`forced`), and
/// where an alias can keep the value (anything but a read that only inspects it) the flag is cleared
/// (`escapes`).
#[derive(Default)]
struct FieldPlan {
    cells: std::collections::BTreeSet<(u16, usize)>,
    forced: std::collections::HashSet<usize>,
    escapes: std::collections::HashMap<usize, std::collections::BTreeSet<(u16, usize)>>,
}

/// The code of a region that never runs, and why: the loops over a collection nothing was put into (by
/// their `IterNext` pc), whose bodies never run; the item reads from one (by pc), which always fail; and
/// every pc control cannot reach once those loops never enter their bodies and those reads never complete
/// ([`Emitter::never_runs`]).
#[derive(Default)]
struct NeverRuns {
    empty_loops: std::collections::BTreeSet<usize>,
    failing_reads: std::collections::BTreeSet<usize>,
    dead: std::collections::BTreeSet<usize>,
}

impl Region {
    /// The region's owned pcs in execution order (flattened intervals).
    fn pcs(&self) -> Vec<usize> {
        self.intervals.iter().flat_map(|&(s, e)| s..e).collect()
    }

    /// The region's pcs that can run, in execution order.
    fn live_pcs(&self) -> Vec<usize> {
        self.pcs()
            .into_iter()
            .filter(|pc| !self.never.dead.contains(pc))
            .collect()
    }

    /// The function whose body the region is; `None` for Main.
    fn function(&self) -> Option<usize> {
        self.label
            .strip_prefix("logos_fn_")
            .and_then(|s| s.parse().ok())
    }
}

/// A region of the emitter as the exact-web analysis reads it ([`crate::vm::tower::webs`]): the pcs that run, the
/// control that reaches them, and every register an op writes and reads.
struct RegionFlow<'e, 'a> {
    emitter: &'e Emitter<'a>,
    region: &'e Region,
}

impl crate::vm::variables::Flow for RegionFlow<'_, '_> {
    fn pcs(&self) -> Vec<usize> {
        self.region.live_pcs()
    }

    fn successors(&self, pc: usize) -> Vec<usize> {
        self.emitter.successors_in(pc, &self.region.never)
    }

    fn written(&self, op: &Op, f: &mut dyn FnMut(u16)) {
        self.emitter.for_each_written(op, f);
    }

    fn read(&self, op: &Op, f: &mut dyn FnMut(u16)) {
        op.for_each_read_reg(|func| self.emitter.prog.functions[func as usize].captures.len(), f);
    }
}

/// The element kind a channel carries (its `Receive` result kind), from the declared `ChanElem`.
fn chan_elem_to_kind(e: ChanElem) -> Kind {
    match e {
        ChanElem::Int => Kind::Int,
        ChanElem::Float => Kind::Float,
        ChanElem::Bool => Kind::Bool,
        ChanElem::Text => Kind::Text,
        ChanElem::Unknown => Kind::Unknown,
    }
}

/// True for an op that needs the cooperative scheduler runtime (so `main` must run as a coroutine task).
fn is_concurrency_op(op: &Op) -> bool {
    matches!(
        op,
        Op::ChanNew { .. }
            | Op::ChanSend { .. }
            | Op::ChanRecv { .. }
            | Op::ChanTrySend { .. }
            | Op::ChanTryRecv { .. }
            | Op::ChanClose { .. }
            | Op::Spawn { .. }
            | Op::SpawnHandle { .. }
            | Op::TaskAwait { .. }
            | Op::TaskAbort { .. }
            | Op::SelectArmRecv { .. }
            | Op::SelectArmTimeout { .. }
            | Op::SelectWait { .. }
            | Op::Sleep { .. }
    )
}

/// How an op that changes a register's collection in place treats the other holders of that collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mutation {
    /// Value semantics: the register's own collection changes, never another holder's. Lowered through an
    /// `_own` runtime helper, which copies the collection first unless the register's OWNED flag says the
    /// register is its only holder.
    Owned(u16),
    /// Value semantics through a record's field: the register's own record, and the record's own value in the
    /// field, change, never another holder's of either. Lowered through a `logos_field_*_own` helper, which
    /// copies the record unless the register's owned flag says the register is its only holder, and the
    /// field's value unless the register's FIELD flag for that field says the record is its only holder
    /// ([`Emitter::field_plan`]).
    Field(u16),
    /// Shared semantics, the VM's: the change reaches every holder (a `SharedSequence` field's one log), or
    /// nothing changes (`EnsureOwned` is a no-op on this tier).
    Shared,
}

/// The class of every op that changes a register's collection in place; `None` for every other op. The
/// `every_in_place_mutation_is_classified` census holds this to the ISA's own list of mutating ops, and
/// `every_mutation_kind_is_stated` to the kind each one has on the VM.
pub(super) fn mutation(op: &Op) -> Option<Mutation> {
    match *op {
        Op::ListPush { list, .. } | Op::ListPop { list, .. } => Some(Mutation::Owned(list)),
        Op::SetIndex { collection, .. }
        | Op::SetIndexUnchecked { collection, .. }
        | Op::RemoveFrom { collection, .. } => Some(Mutation::Owned(collection)),
        Op::SetAdd { set, .. } => Some(Mutation::Owned(set)),
        Op::StructInsert { obj, .. } | Op::CrdtResolve { obj, .. } => Some(Mutation::Owned(obj)),
        Op::CrdtMerge { target, .. } | Op::CrdtJoin { target, .. } => Some(Mutation::Owned(target)),
        Op::ListPushField { obj, .. } | Op::CrdtBump { obj, .. } => Some(Mutation::Field(obj)),
        Op::CrdtAppend { .. } | Op::EnsureOwned { .. } => Some(Mutation::Shared),
        _ => None,
    }
}

/// `t = t + v` — the op that appends to the Text register `t` (its register and `v`'s), which this tier
/// lowers into the register's own Text: in place while the register's owned flag says it is the Text's only
/// holder, after a copy otherwise.
pub(super) fn text_append(op: &Op) -> Option<(u16, u16)> {
    match *op {
        Op::AddAssign { dst, src } => Some((dst, src)),
        _ => None,
    }
}

/// What one op does to the ownership of the collections in its registers, declared by the op's lowering.
/// The undeclared default is the safe one: every register the op reads may have left its handle somewhere
/// else, and every register it writes holds a handle something else may hold too. A missed declaration
/// costs a copy, never a wrong answer.
#[derive(Default)]
struct OwnershipEffect {
    /// Registers the op writes with a collection it just made, which nothing else holds.
    fresh: Vec<u16>,
    /// Registers the op reads without keeping their handle anywhere.
    inspected: Vec<u16>,
    /// Registers whose owned flag the op leaves as it found it: an `_own` helper's target (the helper keeps
    /// the flag itself) and a register an op leaves holding the same handle.
    keeps: Vec<u16>,
}

/// Subtract each hole in `holes` from the span `[s, e)`, returning the remaining pc intervals in order.
fn subtract_holes(s: usize, e: usize, holes: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut cuts: Vec<(usize, usize)> = holes
        .iter()
        .copied()
        .filter(|&(a, b)| s <= a && b <= e && (a, b) != (s, e))
        .collect();
    cuts.sort();
    let mut out = Vec::new();
    let mut cur = s;
    for (a, b) in cuts {
        if a < cur {
            // Nested inside an already-subtracted hole — skip (its pcs are already removed).
            continue;
        }
        if a > cur {
            out.push((cur, a));
        }
        cur = b;
    }
    if cur < e {
        out.push((cur, e));
    }
    out
}

/// Scratch address register for slot/frame offsets that overflow a signed 12-bit immediate.
const ADDR_TMP: &str = "t6";

/// The scratch slots of a frame in a program the exact extension reaches, one per argument register a call takes:
/// where the operands of one op are presented in the kinds it reads them as ([`Emitter::present_operands`]).
const SCRATCH_SLOTS: usize = 8;

pub fn emit(
    prog: &CompiledProgram,
    policies: &PolicyRegistry,
    interner: &Interner,
) -> Result<String, RiscVLowerError> {
    let mut e = Emitter {
        prog,
        policies,
        interner,
        out: String::new(),
        ro: RoData::default(),
        fail_stubs: Vec::new(),
        need_dispatch: false,
        need_scheduler: false,
        closable: false,
        pipe_elem: Kind::Unknown,
        select_boxes: Vec::new(),
        may_be_nothing: std::collections::BTreeSet::new(),
        nothing_literals: std::collections::HashMap::new(),
        flag_base: 0,
        need_nothing_fail: false,
        nothing_label: 0,
        poly_label: 0,
        global_kinds: Vec::new(),
        crdt_fields: std::collections::HashMap::new(),
        composites: std::cell::RefCell::new(Vec::new()),
        owned: std::collections::BTreeSet::new(),
        owned_base: 0,
        fx: OwnershipEffect::default(),
        running: Vec::new(),
        closure_bodies: Vec::new(),
        answers_nothing: Vec::new(),
        reports_nothing: false,
        fails_fast: std::collections::HashMap::new(),
        field_plan: FieldPlan::default(),
        field_flags: std::collections::BTreeMap::new(),
        fail_sites: Vec::new(),
        failing_reads: std::collections::BTreeSet::new(),
        returns: std::cell::RefCell::new(Vec::new()),
        net_elem: std::cell::Cell::new(Kind::Unknown),
        net_stream: std::cell::Cell::new(Kind::Unknown),
        shapes: Vec::new(),
        declared: std::collections::HashMap::new(),
        pc: 0,
        region_pcs: Vec::new(),
        jump_targets: std::collections::HashSet::new(),
        inout: Vec::new(),
        tower: crate::vm::tower::reaches(prog),
        rationals: holds_rationals(prog),
        param_kinds: std::cell::RefCell::new(Vec::new()),
        tower_lands: std::cell::RefCell::new(std::collections::BTreeSet::new()),
        tower_from: std::cell::RefCell::new(std::collections::HashMap::new()),
        tower_entries: std::cell::RefCell::new(std::collections::HashMap::new()),
        number_from: std::cell::RefCell::new(std::collections::HashMap::new()),
        number_entries: std::cell::RefCell::new(std::collections::HashMap::new()),
        scratch_base: 0,
        presented: std::collections::HashMap::new(),
        current_fn: None,
        nothing_shape: None,
        defined_answers: None,
    };
    e.run()?;
    Ok(e.out)
}

struct Emitter<'a> {
    prog: &'a CompiledProgram,
    policies: &'a PolicyRegistry,
    interner: &'a Interner,
    out: String,
    ro: RoData,
    /// The reasons inline checks stop the program with, each with a stub that stops it: `.Lfail_<index>`.
    fail_stubs: Vec<String>,
    /// Set when a `CallValue` is lowered → emit the indirect-call dispatch table.
    need_dispatch: bool,
    /// Set when the program uses any concurrency op → run `main` as a coroutine task under the scheduler
    /// and emit the dispatch table (Spawn resolves the task body through it).
    need_scheduler: bool,
    /// Whether the program closes a pipe anywhere. Only then can a `Receive` read `nothing`.
    closable: bool,
    /// The element kind every pipe of the program carries, when all its `Pipe of …`s agree. A pipe reaches a
    /// task as an `Int` parameter (a task's parameters carry no pipe type), and every pipe is made by one
    /// of those, so a receive from a pipe whose own kind is lost still binds this kind.
    pipe_elem: Kind,
    /// The receive arms of the select being staged: each arm variable's slot holds the box its arm resolves
    /// into, read back at the wait.
    select_boxes: Vec<u16>,
    /// The current region's registers that may hold `nothing` at run time (`VALUE-OR-NOTHING`): what a
    /// receive binds in a program that closes a pipe, what `Try to receive`, `Await … into` and `Pop` bind,
    /// and every copy of one. Each has a flag slot beside the frame's value slots — 1 while it holds
    /// `nothing`, written with every write to it — so an Int read of one is guarded (it fails, as
    /// arithmetic on `nothing` fails on the VM), while `Show`, text rendering, a copy and `is` read the flag.
    may_be_nothing: std::collections::BTreeSet<u16>,
    /// The current region's registers holding the `nothing` literal before each pc (the latest write in
    /// program order), which `is` compares by presence alone.
    nothing_literals: std::collections::HashMap<usize, std::collections::BTreeSet<u16>>,
    /// The frame offset of register 0's flag slot.
    flag_base: i64,
    /// Set when an Int read of a value-or-nothing register is guarded → emit the shared `.Lnothing` stub.
    need_nothing_fail: bool,
    /// Monotone counter for unique local labels around a value-or-nothing branch (`.Lnth_<n>`).
    nothing_label: usize,
    /// Monotone counter for unique local labels inside a compiled policy condition (`.Lpol_<n>`).
    poly_label: usize,
    /// Program-wide kind of each promoted global (from the declared type, else the writing `GlobalSet`).
    global_kinds: Vec<Kind>,
    /// `(struct name, cell index)` of every field that holds a CRDT, with the `NewCrdt` kind code it was made
    /// by ([`Emitter::derive_crdt_fields`]). A field's declared type is its CRDT's projection (a counter's is
    /// `Int`), so this side-table is how a field read, a merge and a copy know the field holds a CRDT.
    crdt_fields: std::collections::HashMap<(String, usize), u8>,
    /// The composite-kind side-table (content-deduped so indices are stable across inference passes).
    composites: std::cell::RefCell<Vec<Composite>>,
    /// The current region's registers that some op mutates as the register's own collection
    /// ([`Mutation::Owned`]). Each has an OWNED flag slot beside the frame's value slots: 1 while the
    /// register is the only holder of its collection's handle, so the mutation happens in place; 0 when the
    /// handle may be held elsewhere too, so the mutation copies first. Every region entry clears them.
    owned: std::collections::BTreeSet<u16>,
    /// The frame offset of register 0's owned flag.
    owned_base: i64,
    /// The ownership effect the lowering of the current op declared.
    fx: OwnershipEffect,
    /// `running[fi]` — whether function `fi` can be entered ([`crate::vm::reach`]); only those are lowered.
    running: Vec<bool>,
    /// `closure_bodies[fi]` — whether function `fi` is a closure body (a `MakeClosure` target).
    closure_bodies: Vec<bool>,
    /// `answers_nothing[fi]` — whether a call of function `fi` can answer `nothing`
    /// ([`Emitter::derive_answers_nothing`]). Every function hands back, beside its value in `a0`, the
    /// value's `nothing` flag in `a1`, which a call of one that can answer `nothing` keeps in its
    /// destination's flag slot.
    answers_nothing: Vec<bool>,
    /// Whether the current region hands back its value's `nothing` flag in `a1`: a function some caller reads
    /// it from — one that can answer `nothing`, or a closure body a call through a closure of its arity
    /// reads it from. Every other return leaves `a1` alone.
    reports_nothing: bool,
    /// The current region's field-ownership plan ([`Emitter::field_plan`]) and the frame offset of each field
    /// flag it keeps, by `(record register, cell)`.
    field_plan: FieldPlan,
    field_flags: std::collections::BTreeMap<(u16, usize), i64>,
    /// The current region's definitions that stop the program where they would yield `nothing`, with the
    /// sentence the op after them stops it with ([`Emitter::fail_fast`]), by pc.
    fails_fast: std::collections::HashMap<usize, String>,
    /// Each call site whose callee stops the program itself on the way to a value it cannot give, with the
    /// sentence it stops with: `(label of the return address, label of the sentence)`, emitted as the
    /// `logos_fail_sites` table the runtime searches by its return address.
    fail_sites: Vec<(String, String)>,
    /// The current region's item reads from a collection with no item ([`NeverRuns`]).
    failing_reads: std::collections::BTreeSet<usize>,
    /// The kind each function answers: what its body's `Return`s hand back where that is known, else its
    /// declared return type ([`Emitter::derive_return_kinds`]).
    returns: std::cell::RefCell<Vec<Kind>>,
    /// The kind of an awaited message and of an awaited batch ([`Emitter::derive_message_kinds`]).
    net_elem: std::cell::Cell<Kind>,
    net_stream: std::cell::Cell<Kind>,
    /// The shape table label of each kind a deep copy has been emitted for ([`Emitter::shape`]).
    shapes: Vec<(Kind, String)>,
    /// The kind each op's value is declared to have ([`CompiledProgram::declared_types`]), by the op's pc.
    declared: std::collections::HashMap<usize, Kind>,
    /// The pc of the op being lowered, the current region's pcs in order, and every pc a jump lands on.
    pc: usize,
    region_pcs: Vec<usize>,
    jump_targets: std::collections::HashSet<usize>,
    /// The current region's `mutable` parameters ([`Emitter::inout_params`]), handed back in `a2`, `a3`, … at
    /// every return.
    inout: Vec<u16>,
    /// Whether the program computes with the exact extension ([`crate::vm::tower::reaches`]): it links the value
    /// runtime, and its registers' kinds are settled by exact webs ([`Emitter::infer_kinds`]).
    tower: bool,
    /// Whether the program makes or declares a rational, so a frame keeps scratch slots to present an integer an op
    /// hands a rational place as the rational it equals ([`Emitter::present_numbers`]).
    rationals: bool,
    /// The pcs whose write lands as a value of the extension, as of the last inference of their region.
    tower_lands: std::cell::RefCell<std::collections::BTreeSet<usize>>,
    /// The definitions of an exact web that compute a value of another kind, with that kind: each lowers as what it
    /// computes, and its register is made the extension's value after it.
    tower_from: std::cell::RefCell<std::collections::HashMap<usize, Kind>>,
    /// The registers each region (by its entry pc) takes in as some other kind and holds as the extension's value,
    /// converted on entry.
    tower_entries: std::cell::RefCell<std::collections::HashMap<usize, Vec<(u16, Kind)>>>,
    /// Each definition a number join lands as another kind ([`crate::vm::numbers`]), by pc: the kind it computes and
    /// the kind it lands as.
    number_from: std::cell::RefCell<std::collections::HashMap<usize, (Kind, Kind)>>,
    /// Per region (by its first pc): the registers it takes in that a number join converts (the kind each arrives as
    /// and the kind it is converted to), and one definition of each variable no machine form holds.
    number_entries: std::cell::RefCell<
        std::collections::HashMap<usize, (Vec<(u16, Kind, Kind)>, Vec<crate::vm::variables::Site>)>,
    >,
    /// The kind each function's parameters arrive as, by function: declared, and the extension's value where a call
    /// hands one ([`Emitter::derive_exact_params`]).
    param_kinds: std::cell::RefCell<Vec<Vec<Kind>>>,
    /// The frame offset of the scratch slots an op's operands are presented in ([`Emitter::present`]).
    scratch_base: i64,
    /// The registers whose reads the op being lowered takes from a scratch slot, by its offset: an operand presented
    /// in the kind the op reads it as.
    presented: std::collections::HashMap<u16, i64>,
    /// The function whose body is being lowered; `None` for Main.
    current_fn: Option<usize>,
    /// The label of the shape of `nothing` (form 23), once an operand of the exact extension has needed it.
    nothing_shape: Option<String>,
    /// The label of the table of the shapes each function's answers are read by when a real number is defined by it,
    /// once a `definedReal` has needed it.
    defined_answers: Option<String>,
}

impl<'a> Emitter<'a> {
    fn run(&mut self) -> Emit {
        self.out
            .push_str("# ============================================================\n");
        self.out
            .push_str("#  LOGOS → RISC-V (RV64GC, lp64d) — AOT codegen tier\n");
        self.out
            .push_str("#  Lowered from the register bytecode; ≡ VM (byte-for-byte).\n");
        self.out
            .push_str("# ============================================================\n");
        self.out.push_str("  .text\n");
        self.out.push_str("  .globl main\n");

        // A concurrency op anywhere means `main` must run as a coroutine task under the scheduler (it can
        // block on `Receive`), so decide this BEFORE emitting the main region (it changes main's label).
        self.need_scheduler = self.prog.code.iter().any(is_concurrency_op);
        self.closable = self
            .prog
            .code
            .iter()
            .any(|op| matches!(op, Op::ChanClose { .. }));
        let mut elems: Vec<Kind> = Vec::new();
        for op in &self.prog.code {
            if let Op::ChanNew { elem, .. } = op {
                let k = chan_elem_to_kind(*elem);
                if !elems.contains(&k) {
                    elems.push(k);
                }
            }
        }
        self.pipe_elem = if let [k] = elems[..] {
            k
        } else {
            Kind::Unknown
        };

        // Only the functions the program can enter are lowered: the loader splices whole standard-library
        // modules, and most of a spliced module is never called. The same walk decides what the
        // unlinkable-code gate judges, so what is lowered and what is judged agree. A layout the walk
        // cannot resolve is lowered whole.
        self.running = match crate::vm::reach::code_layout(self.prog) {
            Ok(layout) => crate::vm::reach::reach(self.prog, &layout, self.interner).functions,
            Err(_) => vec![true; self.prog.functions.len()],
        };
        self.jump_targets = self
            .prog
            .code
            .iter()
            .flat_map(crate::vm::disasm::op_targets)
            .collect();
        self.closure_bodies = vec![false; self.prog.functions.len()];
        for op in &self.prog.code {
            if let Op::MakeClosure { func, .. } = *op {
                self.closure_bodies[func as usize] = true;
            }
        }

        self.declared = self
            .prog
            .declared_types
            .iter()
            .map(|(pc, t)| (*pc as usize, self.boundary_kind(&Some(t.clone()))))
            .collect();
        *self.param_kinds.borrow_mut() = self
            .prog
            .functions
            .iter()
            .map(|f| f.param_types.iter().map(|t| self.boundary_kind(t)).collect())
            .collect();
        let mut regions = self.regions();
        for r in &mut regions {
            r.never = self.never_runs(r);
        }
        *self.returns.borrow_mut() = self
            .prog
            .functions
            .iter()
            .map(|f| self.boundary_kind(&f.return_type))
            .collect();
        for _ in 0..8 {
            let globals = self.derive_global_kinds(&regions);
            let captures: Vec<Vec<Kind>> = regions.iter().map(|r| r.captures.clone()).collect();
            self.derive_capture_kinds(&mut regions);
            let returns_changed = self.derive_return_kinds(&regions);
            let messages_changed = self.derive_message_kinds(&regions);
            let params_changed = self.derive_exact_params(&mut regions);
            let settled = globals == self.global_kinds
                && regions.iter().zip(&captures).all(|(r, c)| r.captures == *c)
                && !returns_changed
                && !messages_changed
                && !params_changed;
            self.global_kinds = globals;
            if settled {
                break;
            }
        }
        self.crdt_fields = self.derive_crdt_fields(&regions);
        self.derive_answers_nothing(&regions);
        for r in &regions {
            self.emit_region(r)?;
        }

        // Under the scheduler, the C entry `main` bootstraps the runtime with the LOGOS main body (emitted
        // as `logos_main_body`) as the first task, then drives the scheduler to quiescence.
        if self.need_scheduler {
            self.out.push_str("\nmain:\n");
            self.out.push_str("  addi sp, sp, -16\n  sd ra, 8(sp)\n");
            // C `main` receives argc (a0) / argv (a1); stash them for `args()` before overwriting a0.
            self.out.push_str("  call logos_set_args\n");
            self.out.push_str("  la a0, logos_main_body\n");
            self.out.push_str("  call logos_sched_bootstrap\n");
            self.out
                .push_str("  li a0, 0\n  ld ra, 8(sp)\n  addi sp, sp, 16\n  ret\n");
        }

        // One stub per reason an inline check stops the program with (a zero divisor, a NaN divisor…): it
        // fails through `logos_fatal`, so the binary's exit status mirrors the VM's `error.is_some()` and a
        // failing task fails its run rather than the process.
        for (i, reason) in std::mem::take(&mut self.fail_stubs).into_iter().enumerate() {
            let reason = self.ro.intern_text(&reason);
            self.out.push_str(&format!("\n.Lfail_{i}:\n"));
            self.out.push_str(&format!("  la a0, {reason}\n"));
            self.out.push_str("  call logos_fatal\n");
        }
        if self.need_nothing_fail {
            // An Int read of a value-or-nothing register holding `nothing`.
            let reason = self
                .ro
                .intern_text("expected an Int, but the value is nothing");
            self.out.push_str("\n.Lnothing:\n");
            self.out.push_str(&format!("  la a0, {reason}\n"));
            self.out.push_str("  call logos_fatal\n");
        }

        // The sentences the runtime stops a program with, the VM's own (`refusals`), as globals it reads only
        // on the way to stopping: the calls that can fail pass nothing for them.
        let r = refusals();
        let (utf8_invalid, utf8_incomplete) = text_from_bytes_refusals();
        self.out.push_str("\n  .section .rodata\n");
        for (symbol, text) in [
            ("logos_index_refusal", &r.index),
            ("logos_missing_key_refusal", &r.missing_key),
            ("logos_exact_division_refusal", &r.exact_division),
            ("logos_float_division_refusal", &r.float_division),
            ("logos_negative_exponent_refusal", &r.negative_exponent),
            ("logos_exponent_too_large_refusal", &r.exponent_too_large),
            ("logos_parse_int_refusal", &r.parse_int),
            ("logos_parse_float_refusal", &r.parse_float),
            ("logos_random_bounds_refusal", &r.random_bounds),
            ("logos_utf8_invalid_refusal", utf8_invalid),
            ("logos_utf8_incomplete_refusal", utf8_incomplete),
            (
                "logos_await_without_listen_refusal",
                &crate::concurrency::net_inbox::AWAIT_WITHOUT_LISTEN.to_string(),
            ),
        ] {
            self.out.push_str(&format!(
                "  .globl {symbol}\n{symbol}: .string \"{}\"\n",
                super::rodata::escape_asm_string(text)
            ));
        }
        // The sentences the shim's exact numbers stop a program with, the VM's own.
        for (symbol, text) in crate::codegen_c::words::exact_number_sentences() {
            self.out.push_str(&format!(
                "  .globl {symbol}\n{symbol}: .string \"{}\"\n",
                super::rodata::escape_asm_string(&text)
            ));
        }
        // The fail-fast call sites (`fail_sites`), ended by a zero pair; always emitted, since the runtime reads it.
        self.out.push_str(
            "\n  .section .rodata\n  .align 3\n  .globl logos_fail_sites\nlogos_fail_sites:\n",
        );
        for (site, reason) in std::mem::take(&mut self.fail_sites) {
            self.out.push_str(&format!("  .quad {site}, {reason}\n"));
        }
        self.out.push_str("  .quad 0, 0\n");
        // Indirect-call dispatch table (`CallValue` reads a closure's body index; `Spawn` resolves a task
        // body through it from the C runtime): one `.quad` per function, in index order, 0 for a function
        // nothing can enter (it has no body). ALWAYS emitted as a GLOBAL symbol — the concurrency runtime
        // `.o` references it unconditionally (harmless + unused for a program that never spawns), so it
        // must always resolve.
        self.out.push_str(
            "\n  .section .rodata\n  .align 3\n  .globl logos_fn_dispatch\nlogos_fn_dispatch:\n",
        );
        for fi in 0..self.prog.functions.len() {
            if self.running[fi] {
                self.out.push_str(&format!("  .quad logos_fn_{fi}\n"));
            } else {
                self.out.push_str("  .quad 0\n");
            }
        }
        let _ = self.need_dispatch;

        // The promoted-globals table (Main top-level bindings read inside functions), zero-initialized.
        if !self.prog.globals.is_empty() {
            self.out.push_str("\n  .section .bss\n  .align 3\n");
            self.out.push_str("logos_globals:\n");
            self.out
                .push_str(&format!("  .zero {}\n", 8 * self.prog.globals.len()));
        }

        // A program the exact extension reaches declares the value runtime it links (`codegen_riscv::runtime`).
        if self.tower {
            self.out.push_str(&format!(
                "\n  .section .rodata\n  .globl {0}\n{0}:\n  .byte 1\n",
                super::VALUE_RUNTIME_SYMBOL
            ));
        }

        let mut ro = String::new();
        self.ro.emit(&mut ro);
        self.out.push_str(&ro);
        Ok(())
    }

    /// Partition the shared `code` vector into Main + one region per function. Inline CLOSURE bodies are
    /// embedded inside their defining region (bracketed by a jump-over), so a region owns a set of pc
    /// INTERVALS with holes where its inline closures sit; each closure is emitted as its own region.
    fn regions(&self) -> Vec<Region> {
        let code = &self.prog.code;
        let code_len = code.len();

        // An inline closure is entered only via `CallValue` and physically embedded mid-region: the op
        // just before its entry is a forward `Jump` over the (Return-terminated) body — the compiler's
        // jump-over. Its extent is `[entry, jover_target)`.
        let is_inline = |e: usize| -> Option<usize> {
            if e >= 1 {
                if let Op::Jump { target } = code[e - 1] {
                    if target > e
                        && target <= code_len
                        && matches!(code[target - 1], Op::Return { .. } | Op::ReturnNothing)
                    {
                        return Some(target);
                    }
                }
            }
            None
        };

        let mut inline_extents: Vec<(usize, usize)> = Vec::new();
        for f in &self.prog.functions {
            if let Some(end) = is_inline(f.entry_pc) {
                inline_extents.push((f.entry_pc, end));
            }
        }

        // Top-level block boundaries: Main (pc 0) + every non-inline function, sorted by entry.
        let mut top_starts: Vec<usize> = vec![0];
        for f in &self.prog.functions {
            if is_inline(f.entry_pc).is_none() {
                top_starts.push(f.entry_pc);
            }
        }
        top_starts.sort_unstable();
        top_starts.dedup();

        let block_end = |s: usize| -> usize {
            top_starts
                .iter()
                .copied()
                .find(|&x| x > s)
                .unwrap_or(code_len)
        };

        // Main owns its block [0, next top-level) minus the inline closures embedded in it. Under the
        // scheduler it becomes the `logos_main_body` task (the real `main` entry bootstraps the runtime).
        let mut regions = vec![Region {
            label: if self.need_scheduler {
                "logos_main_body".to_string()
            } else {
                "main".to_string()
            },
            intervals: subtract_holes(0, block_end(0), &inline_extents),
            frame_regs: self.prog.register_count,
            params: 0,
            param_kinds: Vec::new(),
            is_main: true,
            captures: Vec::new(),
            never: NeverRuns::default(),
        }];

        for (fi, f) in self.prog.functions.iter().enumerate() {
            if !self.running[fi] {
                continue;
            }
            let param_kinds = f
                .param_types
                .iter()
                .map(|t| self.boundary_kind(t))
                .collect();
            let (span_s, span_e) = match is_inline(f.entry_pc) {
                Some(end) => (f.entry_pc, end), // an inline closure: its bracketed body
                None => (f.entry_pc, block_end(f.entry_pc)), // a top-level function block
            };
            regions.push(Region {
                label: format!("logos_fn_{fi}"),
                intervals: subtract_holes(span_s, span_e, &inline_extents),
                frame_regs: f.register_count,
                params: f.param_count,
                param_kinds,
                is_main: false,
                // Capture count now; kinds are filled by `derive_capture_kinds` from the MakeClosure site.
                captures: vec![Kind::Unknown; f.captures.len()],
                never: NeverRuns::default(),
            });
        }
        regions
    }

    /// Discover which struct fields hold a CRDT, and which: a `NewCrdt` value stored into a field by the
    /// `StructInsert` that takes it (a `Shared` record's constructor makes each CRDT field so).
    fn derive_crdt_fields(
        &self,
        regions: &[Region],
    ) -> std::collections::HashMap<(String, usize), u8> {
        let mut out = std::collections::HashMap::new();
        for r in regions {
            let per_pc = self.infer_kinds(r);
            let pcs = r.live_pcs();
            for (i, &pc) in pcs.iter().enumerate() {
                let Op::NewCrdt { dst, kind } = self.prog.code[pc] else {
                    continue;
                };
                for &later in &pcs[i + 1..] {
                    let op = self.prog.code[later];
                    if let Op::StructInsert { obj, field, value } = op {
                        if value == dst {
                            let ok = kind_at(&per_pc[&later], obj);
                            if let (Some(Composite::Struct(name)), Some((cell, _))) =
                                (self.as_composite(ok), self.resolve_field(ok, field))
                            {
                                out.insert((name, cell), kind);
                            }
                            break;
                        }
                    }
                    let mut rewritten = false;
                    self.for_each_written(&op, |w| rewritten |= w == dst);
                    if rewritten {
                        break;
                    }
                }
            }
        }
        out
    }

    /// The `NewCrdt` kind code of the CRDT that field `cell` of struct `name` holds, if it holds one.
    fn crdt_field(&self, name: &str, cell: usize) -> Option<u8> {
        self.crdt_fields.get(&(name.to_string(), cell)).copied()
    }

    /// Fill each closure-body region's capture kinds by inspecting the `MakeClosure` that builds it: a
    /// local capture takes the kind of the captured slot at the build site; a global capture takes the
    /// program-wide global kind. (`derive_global_kinds` must have run first.)
    fn derive_capture_kinds(&self, regions: &mut [Region]) {
        // func index → capture kinds, discovered at the build site.
        let mut found: std::collections::HashMap<usize, Vec<Kind>> =
            std::collections::HashMap::new();
        for r in regions.iter() {
            let per_pc = self.infer_kinds(r);
            for pc in r.live_pcs() {
                if let Op::MakeClosure {
                    func, locals_start, ..
                } = self.prog.code[pc]
                {
                    let caps = &self.prog.functions[func as usize].captures;
                    let cur = &per_pc[&pc];
                    let mut local_k: u16 = 0;
                    let kinds: Vec<Kind> = caps
                        .iter()
                        .map(|(_, global)| match global {
                            Some(gidx) => self
                                .global_kinds
                                .get(*gidx as usize)
                                .copied()
                                .unwrap_or(Kind::Unknown),
                            None => {
                                let k = kind_at(cur, locals_start + local_k);
                                local_k += 1;
                                k
                            }
                        })
                        .collect();
                    found.insert(func as usize, kinds);
                }
            }
        }
        for r in regions.iter_mut() {
            if let Some(fi) = r
                .label
                .strip_prefix("logos_fn_")
                .and_then(|s| s.parse::<usize>().ok())
            {
                if let Some(kinds) = found.get(&fi) {
                    r.captures = kinds.clone();
                }
            }
        }
    }

    /// The kind each function answers: what its `Return`s hand back, unified, where that is known — the value
    /// a function returns is what its body builds, which its declaration may describe less exactly (a named
    /// tuple is a record, a `-> Real` may return an Int) or not at all (a closure body, a function the front
    /// end synthesized) — and its declared type where it is not. Returns whether any function's kind changed
    /// (the caller iterates to a fixpoint, since a body's kinds depend on what the functions it calls answer).
    fn derive_return_kinds(&self, regions: &[Region]) -> bool {
        let mut changed = false;
        for r in regions {
            let Some(fi) = r.function() else {
                continue;
            };
            let declared = self.boundary_kind(&self.prog.functions[fi].return_type);
            let per_pc = self.infer_kinds(r);
            let mut answered: Option<Kind> = None;
            let mut disagree = false;
            for pc in r.live_pcs() {
                if let Op::Return { src } = self.prog.code[pc] {
                    let k = kind_at(&per_pc[&pc], src);
                    answered = Some(match answered {
                        None => k,
                        Some(a) => {
                            let u = self.unify(a, k);
                            disagree |=
                                u == Kind::Unknown && a != Kind::Unknown && k != Kind::Unknown;
                            u
                        }
                    });
                }
            }
            let answered = if disagree {
                Kind::Unknown
            } else {
                answered.unwrap_or(Kind::Unknown)
            };
            let k = match (answered, self.unify(declared, answered)) {
                (Kind::Unknown, _) => declared,
                (_, Kind::Unknown) => answered,
                (_, both) => both,
            };
            let mut returns = self.returns.borrow_mut();
            if returns[fi] != k {
                returns[fi] = k;
                changed = true;
            }
        }
        changed
    }

    /// The parameters the program hands a value of the exact extension: each holds one, whatever it declares, and a
    /// call handing it another kind converts that value on the way in. Returns whether any parameter's kind changed
    /// (the caller iterates to a fixpoint with the rest of the signatures).
    fn derive_exact_params(&self, regions: &mut [Region]) -> bool {
        if !self.tower {
            return false;
        }
        let mut handed: std::collections::BTreeSet<(usize, u16)> = std::collections::BTreeSet::new();
        for r in regions.iter() {
            let per_pc = self.infer_kinds(r);
            for pc in r.live_pcs() {
                let (func, args_start, arg_count) = match self.prog.code[pc] {
                    Op::Call {
                        func,
                        args_start,
                        arg_count,
                        ..
                    } => (func as usize, args_start, arg_count),
                    Op::CallValue {
                        callee,
                        args_start,
                        arg_count,
                        ..
                    } => match self.as_composite(kind_at(&per_pc[&pc], callee)) {
                        Some(Composite::Closure(f)) => (f as usize, args_start, arg_count),
                        _ => continue,
                    },
                    _ => continue,
                };
                for i in 0..arg_count {
                    if kind_at(&per_pc[&pc], args_start + i) == Kind::Tower {
                        handed.insert((func, i));
                    }
                }
            }
        }
        let mut changed = false;
        for r in regions.iter_mut() {
            let Some(fi) = r.function() else {
                continue;
            };
            for &(_, i) in handed.range((fi, 0)..=(fi, u16::MAX)) {
                if let Some(k) = r.param_kinds.get_mut(i as usize) {
                    if *k != Kind::Tower {
                        *k = Kind::Tower;
                        changed = true;
                    }
                }
            }
            self.param_kinds.borrow_mut()[fi] = r.param_kinds.clone();
        }
        changed
    }

    /// The kind of what an `Await` binds: every message the program sends travels one inbox, so a message
    /// awaited has the kind every `Send` agrees on, and a batch awaited the list kind every `Stream` agrees
    /// on — `Unknown` when two disagree or one is not modeled. Returns whether either changed.
    fn derive_message_kinds(&self, regions: &[Region]) -> bool {
        let agree = |seen: &mut Option<Option<Kind>>, k: Kind| {
            *seen = Some(match *seen {
                None if k != Kind::Unknown => Some(k),
                Some(Some(a)) if a == k => Some(a),
                _ => None,
            });
        };
        let (mut messages, mut batches) = (None, None);
        for r in regions {
            let per_pc = self.infer_kinds(r);
            for pc in r.live_pcs() {
                match self.prog.code[pc] {
                    Op::NetSend { msg, .. } => agree(&mut messages, kind_at(&per_pc[&pc], msg)),
                    Op::NetStream { values, .. } => {
                        let vk = kind_at(&per_pc[&pc], values);
                        let listed = self.is_seq_like(vk);
                        agree(&mut batches, if listed { vk } else { self.seq_of_kind(vk) });
                    }
                    _ => {}
                }
            }
        }
        let message = messages.flatten().unwrap_or(Kind::Unknown);
        let batch = batches.flatten().unwrap_or(Kind::Unknown);
        let changed = message != self.net_elem.get() || batch != self.net_stream.get();
        self.net_elem.set(message);
        self.net_stream.set(batch);
        changed
    }

    /// The kind a call through a closure of unknown body answers: the kind every closure body of the
    /// program taking `arity` arguments answers, when they all agree.
    fn closure_return(&self, arity: u16) -> Kind {
        let returns = self.returns.borrow();
        let mut agreed: Option<Kind> = None;
        for (fi, f) in self.prog.functions.iter().enumerate() {
            if !self.closure_bodies[fi] || !self.running[fi] || f.param_count != arity {
                continue;
            }
            match agreed {
                None => agreed = Some(returns[fi]),
                Some(k) if k == returns[fi] => {}
                Some(_) => return Kind::Unknown,
            }
        }
        agreed.unwrap_or(Kind::Unknown)
    }

    /// Program-wide kind of each promoted global: the declared type when modeled, else the kind of the
    /// value the first `GlobalSet` stores into it. Two-level so a `GlobalGet` in a function body knows
    /// the kind a `GlobalSet` in Main produced (cross-region), which per-region inference cannot see.
    fn derive_global_kinds(&self, regions: &[Region]) -> Vec<Kind> {
        let mut gk = vec![Kind::Unknown; self.prog.globals.len()];
        for (i, gt) in self.prog.global_types.iter().enumerate() {
            if i < gk.len() {
                gk[i] = self.boundary_kind(gt);
            }
        }
        for r in regions {
            let per_pc = self.infer_kinds(r);
            for pc in r.live_pcs() {
                if let Op::GlobalSet { idx, src } = self.prog.code[pc] {
                    let i = idx as usize;
                    let k = kind_at(&per_pc[&pc], src);
                    // A global any value of the exact extension is stored into holds one.
                    if i < gk.len() && (gk[i] == Kind::Unknown || k == Kind::Tower) {
                        gk[i] = k;
                    }
                }
            }
        }
        gk
    }

    /// Intern a composite kind (content-deduped so its index is stable across inference passes), and
    /// return the referencing [`Kind::Composite`].
    fn intern_composite(&self, c: Composite) -> Kind {
        let mut t = self.composites.borrow_mut();
        let idx = t.iter().position(|x| *x == c).unwrap_or_else(|| {
            t.push(c);
            t.len() - 1
        });
        Kind::Composite(idx as u32)
    }

    fn as_composite(&self, k: Kind) -> Option<Composite> {
        match k {
            Kind::Composite(i) => self.composites.borrow().get(i as usize).cloned(),
            _ => None,
        }
    }

    fn map_of(&self, key: Kind, val: Kind) -> Kind {
        self.intern_composite(Composite::Map(key, val))
    }

    fn tuple_of(&self, positions: Vec<Kind>) -> Kind {
        self.intern_composite(Composite::Tuple(positions))
    }

    /// The sequence kind whose elements are `elem`, tracking a composite element via the side-table
    /// (`Seq of (Int,Int)`, nested `Seq of Seq`); scalar elements use the flat `SeqInt/Float/Text`.
    fn seq_of_kind(&self, elem: Kind) -> Kind {
        match elem {
            // A list holding a value of the extension is one: the runtime's list.
            Kind::Tower => Kind::Tower,
            Kind::Int | Kind::Float | Kind::Text | Kind::Bool | Kind::Word32 => seq_of(elem),
            Kind::Rational | Kind::Date | Kind::Moment | Kind::Peer | Kind::Composite(_) => {
                self.intern_composite(Composite::Seq(elem))
            }
            k if is_collection(k) => self.intern_composite(Composite::Seq(k)),
            _ => Kind::SeqUnknown,
        }
    }

    /// The kind of a list literal whose items are of kinds `items`: its first item's list, or a list of rationals where
    /// integers sit beside rationals, each integer held as the rational it equals.
    fn list_kind(&self, items: &[Kind]) -> Kind {
        if items.contains(&Kind::Rational) && items.iter().all(|k| matches!(k, Kind::Int | Kind::Rational)) {
            return self.seq_of_kind(Kind::Rational);
        }
        items.first().map_or(Kind::SeqUnknown, |first| self.seq_of_kind(*first))
    }

    /// Whether `k` is a list of rationals.
    fn rational_list(&self, k: Kind) -> bool {
        matches!(self.as_composite(k), Some(Composite::Seq(Kind::Rational)))
    }

    /// Whether `k` is a sequence this tier models: of scalars, or of composites or collections.
    fn is_seq_like(&self, k: Kind) -> bool {
        is_concrete_seq(k) || matches!(self.as_composite(k), Some(Composite::Seq(_)))
    }

    /// The element kind of any sequence — the composite element for a `Composite::Seq`, else the scalar
    /// element of a flat sequence/set (Unknown for a non-sequence).
    fn elem_of_kind(&self, coll: Kind) -> Kind {
        match self.as_composite(coll) {
            Some(Composite::Seq(e)) => e,
            // An exact list's items are exact values.
            _ if coll == Kind::Tower => Kind::Tower,
            _ => elem_of(coll),
        }
    }

    /// True if `k` can be iterated by `IterPrepare` — a concrete scalar sequence/set OR a sequence of
    /// composites (both ride the `logos_seq` cell array; iteration yields one 8-byte cell at a time).
    fn is_iterable(&self, k: Kind) -> bool {
        is_collection(k) || matches!(self.as_composite(k), Some(Composite::Seq(_)))
    }

    /// True if `k` is a map handle (a `Composite::Map`).
    fn is_map(&self, k: Kind) -> bool {
        matches!(self.as_composite(k), Some(Composite::Map(..)))
    }

    /// The struct type name a `type_name` constant refers to (a `Constant::Text`).
    fn struct_type_name(&self, type_name: u32) -> Option<String> {
        match self.prog.constants.get(type_name as usize) {
            Some(Constant::Text(s)) => Some(s.clone()),
            _ => None,
        }
    }

    /// Number of fields in the named struct type.
    fn struct_field_count(&self, name: &str) -> Option<usize> {
        self.prog
            .struct_types
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.fields.len())
    }

    /// Resolve a struct field (by the object's `Composite::Struct` kind + a field-name constant) to its
    /// declaration-order cell index and its `Kind`.
    fn resolve_field(&self, obj_kind: Kind, field: u32) -> Option<(usize, Kind)> {
        let sname = match self.as_composite(obj_kind) {
            Some(Composite::Struct(n)) => n,
            _ => return None,
        };
        let fname = match self.prog.constants.get(field as usize) {
            Some(Constant::Text(s)) => s.clone(),
            _ => return None,
        };
        let sd = self.prog.struct_types.iter().find(|s| s.name == sname)?;
        let idx = sd.fields.iter().position(|(n, _)| *n == fname)?;
        Some((idx, self.boundary_kind(&Some(sd.fields[idx].1.clone()))))
    }

    /// The (cell index, kind) of a struct field by struct + field NAME (policy conditions carry names,
    /// not const indices). Mirrors [`Self::resolve_field`] but keyed by name.
    fn field_cell_by_name(&self, struct_name: &str, field_name: &str) -> Option<(usize, Kind)> {
        let sd = self
            .prog
            .struct_types
            .iter()
            .find(|s| s.name == struct_name)?;
        let idx = sd.fields.iter().position(|(n, _)| *n == field_name)?;
        Some((idx, self.boundary_kind(&Some(sd.fields[idx].1.clone()))))
    }

    /// Emit code leaving the boolean result (0/1 in `a0`) of a compiled policy `cond` evaluated against
    /// the `subject` struct (and `object` for capability conditions). Mirrors
    /// `semantics::policy::evaluate_policy_condition` exactly.
    fn emit_policy_condition(
        &mut self,
        cond: &PolicyCondition,
        subject: u16,
        subject_type: &str,
        object: u16,
        object_type: Option<&str>,
    ) -> Emit {
        match cond {
            PolicyCondition::FieldEquals {
                field,
                value,
                is_string_literal,
            } => {
                let fname = self.interner.resolve(*field).to_string();
                let vstr = self.interner.resolve(*value).to_string();
                let (cell, fk) = self.field_cell_by_name(subject_type, &fname).ok_or(
                    RiscVLowerError::Unsupported("policy references an unknown field"),
                )?;
                self.load_slot(subject, "a0");
                self.line(&format!("li a1, {}", cell + 1));
                self.line("call logos_seq_index"); // a0 = field value
                match fk {
                    Kind::Text => {
                        let lbl = self.ro.intern_text(&vstr);
                        self.line(&format!("la a1, {lbl}"));
                        self.line("call strcmp");
                        self.line("seqz a0, a0");
                    }
                    Kind::Int if !is_string_literal => match vstr.parse::<i64>() {
                        Ok(n) => {
                            self.line(&format!("li t0, {n}"));
                            self.line("xor a0, a0, t0");
                            self.line("seqz a0, a0");
                        }
                        Err(_) => self.line("li a0, 0"),
                    },
                    Kind::Bool if !is_string_literal => match vstr.parse::<bool>() {
                        Ok(b) => {
                            self.line(&format!("li t0, {}", b as i64));
                            self.line("xor a0, a0, t0");
                            self.line("seqz a0, a0");
                        }
                        Err(_) => self.line("li a0, 0"),
                    },
                    _ => self.line("li a0, 0"), // string-literal vs Int/Bool field, or unmodeled kind
                }
            }
            PolicyCondition::FieldBool { field, value } => {
                let fname = self.interner.resolve(*field).to_string();
                let (cell, _) = self.field_cell_by_name(subject_type, &fname).ok_or(
                    RiscVLowerError::Unsupported("policy references an unknown field"),
                )?;
                self.load_slot(subject, "a0");
                self.line(&format!("li a1, {}", cell + 1));
                self.line("call logos_seq_index");
                self.line(&format!("li t0, {}", *value as i64));
                self.line("xor a0, a0, t0");
                self.line("seqz a0, a0");
            }
            PolicyCondition::Predicate { predicate, .. } => {
                // Recurse into the named predicate's condition on the same subject.
                let sub = self.interner.lookup(subject_type).and_then(|ts| {
                    self.policies.get_predicates(ts).and_then(|ps| {
                        ps.iter()
                            .find(|p| p.predicate_name == *predicate)
                            .map(|p| p.condition.clone())
                    })
                });
                match sub {
                    Some(c) => {
                        self.emit_policy_condition(&c, subject, subject_type, object, object_type)?
                    }
                    None => self.line("li a0, 0"),
                }
            }
            PolicyCondition::ObjectFieldEquals {
                subject: subj_field,
                field,
                ..
            } => {
                self.emit_field_equality(
                    subject,
                    subject_type,
                    *subj_field,
                    object,
                    object_type,
                    *field,
                )?;
            }
            PolicyCondition::SubjectFieldEqualsObjectField {
                subject_field,
                object_field,
                ..
            } => {
                self.emit_field_equality(
                    subject,
                    subject_type,
                    *subject_field,
                    object,
                    object_type,
                    *object_field,
                )?;
            }
            PolicyCondition::Or(l, r) => {
                let n = self.poly_label;
                self.poly_label += 1;
                self.emit_policy_condition(l, subject, subject_type, object, object_type)?;
                self.line(&format!("bnez a0, .Lpol_{n}_t")); // short-circuit: left true ⇒ true
                self.emit_policy_condition(r, subject, subject_type, object, object_type)?;
                self.out.push_str(&format!(".Lpol_{n}_t:\n"));
            }
            PolicyCondition::And(l, r) => {
                let n = self.poly_label;
                self.poly_label += 1;
                self.emit_policy_condition(l, subject, subject_type, object, object_type)?;
                self.line(&format!("beqz a0, .Lpol_{n}_f")); // short-circuit: left false ⇒ false
                self.emit_policy_condition(r, subject, subject_type, object, object_type)?;
                self.out.push_str(&format!(".Lpol_{n}_f:\n"));
            }
        }
        Ok(())
    }

    /// The sentence a `Check` stops the program with when it does not pass, from the function the VM checks
    /// with ([`crate::semantics::policy::check_policy`]): asked about a value of the subject's type with no
    /// fields, which satisfies no condition, it answers exactly the failure the VM reports for this check —
    /// the unmet predicate, a predicate the type does not define, or a subject that is not a struct.
    fn check_refusal(
        &self,
        subject: Kind,
        object_type: Option<&str>,
        predicate: crate::intern::Symbol,
        is_capability: bool,
        source: &str,
    ) -> Result<String, RiscVLowerError> {
        let subject = self.exemplar(subject).ok_or(RiscVLowerError::Unsupported(
            "a Check of a subject whose kind is not modeled",
        ))?;
        let object = object_type.map(bare_struct);
        match crate::semantics::policy::check_policy(
            self.policies,
            self.interner,
            &subject,
            predicate,
            is_capability,
            object.as_ref(),
            source,
        ) {
            Err(reason) => Ok(reason),
            Ok(()) => Err(RiscVLowerError::Unsupported(
                "a policy condition that holds of a value with no fields",
            )),
        }
    }

    /// A value of kind `k`, to ask a function of the VM what it makes of a value of that kind
    /// (`check_policy`, `peer_topic`): a struct with no fields, a scalar's zero, or `None` for a kind this
    /// tier does not model.
    fn exemplar(&self, k: Kind) -> Option<crate::interpreter::RuntimeValue> {
        use crate::interpreter::RuntimeValue;
        Some(match (k, self.as_composite(k)) {
            (_, Some(Composite::Struct(name))) => bare_struct(&name),
            (Kind::Int, _) => RuntimeValue::Int(0),
            (Kind::Float, _) => RuntimeValue::Float(0.0),
            (Kind::Bool, _) => RuntimeValue::Bool(false),
            (Kind::Text, _) => RuntimeValue::Text(std::rc::Rc::new(String::new())),
            _ => return None,
        })
    }

    /// `subject.<subj_field> == object.<obj_field>` (the VM's `values_equal`) → 0/1 in `a0`. A missing
    /// object (a predicate context, not a capability) is false.
    fn emit_field_equality(
        &mut self,
        subject: u16,
        subject_type: &str,
        subj_field: crate::intern::Symbol,
        object: u16,
        object_type: Option<&str>,
        obj_field: crate::intern::Symbol,
    ) -> Emit {
        let obj_type = match (object_type, object != u16::MAX) {
            (Some(t), true) => t.to_string(),
            _ => {
                self.line("li a0, 0");
                return Ok(());
            }
        };
        let sf = self.interner.resolve(subj_field).to_string();
        let of = self.interner.resolve(obj_field).to_string();
        let (sc, sk) =
            self.field_cell_by_name(subject_type, &sf)
                .ok_or(RiscVLowerError::Unsupported(
                    "policy references an unknown subject field",
                ))?;
        let (oc, _) =
            self.field_cell_by_name(&obj_type, &of)
                .ok_or(RiscVLowerError::Unsupported(
                    "policy references an unknown object field",
                ))?;
        let tag = kind_tag(sk)?;
        self.load_slot(subject, "a0");
        self.line(&format!("li a1, {}", sc + 1));
        self.load_slot(object, "a2");
        self.line(&format!("li a3, {}", oc + 1));
        self.line(&format!("li a4, {tag}"));
        self.line("call logos_struct_field_eq");
        Ok(())
    }

    /// A constructor's runtime tag: the index of the first constant spelling its name. `NewInductive`
    /// writes it into cell 0 and `TestArm` compares against it, so a value matches an arm by constructor
    /// NAME, as the VM matches it, whether or not its enum is declared (`Option` and `Result` are not).
    fn ctor_tag(&self, name: &str) -> Option<usize> {
        self.prog
            .constants
            .iter()
            .position(|c| matches!(c, Constant::Text(s) if s == name))
    }

    /// The name a `Constant::Text` index spells.
    fn text_const(&self, idx: u32) -> Option<&str> {
        match self.prog.constants.get(idx as usize) {
            Some(Constant::Text(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Whether `k` is laid out as an enum value — `[constructor tag, payload…]`.
    fn is_enum_like(&self, k: Kind) -> bool {
        matches!(
            self.as_composite(k),
            Some(Composite::Enum(_) | Composite::Option(_) | Composite::Result(..))
        )
    }

    /// The kind of payload `index` of constructor `variant` in a value of enum kind `enum_kind`.
    fn variant_payload_kind(&self, enum_kind: Kind, variant: &str, index: usize) -> Kind {
        match self.as_composite(enum_kind) {
            Some(Composite::Enum(name)) => self
                .prog
                .enum_types
                .iter()
                .find(|e| e.name == name)
                .and_then(|e| e.variants.iter().find(|v| v.name == variant))
                .and_then(|v| v.field_types.get(index))
                .map(|t| self.boundary_kind(&Some(t.clone())))
                .unwrap_or(Kind::Unknown),
            Some(Composite::Option(some)) if variant == "Some" && index == 0 => {
                some.unwrap_or(Kind::Unknown)
            }
            Some(Composite::Result(ok, _)) if variant == "Ok" && index == 0 => {
                ok.unwrap_or(Kind::Unknown)
            }
            Some(Composite::Result(_, err)) if variant == "Err" && index == 0 => {
                err.unwrap_or(Kind::Unknown)
            }
            _ => Kind::Unknown,
        }
    }

    /// The kind `NewInductive` gives a value of enum `type_name` built by constructor `ctor`, whose first
    /// payload has kind `first`: a declared enum by name; `Option`/`Result`, which are never declared,
    /// carrying the payload kind this constructor fixes.
    fn inductive_kind(&self, type_name: &str, ctor: &str, first: Kind) -> Kind {
        if self.prog.enum_types.iter().any(|e| e.name == type_name) {
            return self.intern_composite(Composite::Enum(type_name.to_string()));
        }
        match (type_name, ctor) {
            ("Option", "Some") => self.intern_composite(Composite::Option(Some(first))),
            ("Option", _) => self.intern_composite(Composite::Option(None)),
            ("Result", "Ok") => self.intern_composite(Composite::Result(Some(first), None)),
            ("Result", "Err") => self.intern_composite(Composite::Result(None, Some(first))),
            _ => self.intern_composite(Composite::Enum(type_name.to_string())),
        }
    }

    /// The most informative kind two sightings of one value agree on: an unknown part takes the other's,
    /// an `Option`/`Result` unifies payload by payload; a real disagreement is `Unknown`.
    fn unify(&self, a: Kind, b: Kind) -> Kind {
        if a == b {
            return a;
        }
        match (a, b) {
            (Kind::Unknown, x) | (x, Kind::Unknown) => x,
            (Kind::SeqUnknown, x) | (x, Kind::SeqUnknown) if self.is_seq_like(x) => x,
            (Kind::SetUnknown, x) | (x, Kind::SetUnknown) if is_concrete_set(x) => x,
            (Kind::MapUnknown, x) | (x, Kind::MapUnknown) if self.is_map(x) => x,
            // A value of the extension and any other are both held as the extension's value.
            (Kind::Tower, _) | (_, Kind::Tower) => Kind::Tower,
            _ => match (self.as_composite(a), self.as_composite(b)) {
                (Some(Composite::Option(x)), Some(Composite::Option(y))) => {
                    let k = self.unify_payload(x, y);
                    self.intern_composite(Composite::Option(k))
                }
                (Some(Composite::Result(o1, e1)), Some(Composite::Result(o2, e2))) => {
                    let (o, e) = (self.unify_payload(o1, o2), self.unify_payload(e1, e2));
                    self.intern_composite(Composite::Result(o, e))
                }
                _ => Kind::Unknown,
            },
        }
    }

    /// Combine constructor possibilities without treating an absent payload as an unknown value.
    fn unify_payload(&self, a: Option<Kind>, b: Option<Kind>) -> Option<Kind> {
        match (a, b) {
            (Some(x), Some(y)) => Some(self.unify(x, y)),
            (Some(k), None) | (None, Some(k)) => Some(k),
            (None, None) => None,
        }
    }

    /// A struct field by name when the object's struct type is not known statically: its cell and kind,
    /// provided every struct type with a field of that name holds it in the same cell at the same kind —
    /// the VM finds a field by name, and then so does this tier. `None` when they disagree.
    fn field_by_name_anywhere(&self, field: u32) -> Option<(usize, Kind)> {
        let fname = self.text_const(field)?;
        let mut found: Option<(usize, Kind)> = None;
        for sd in &self.prog.struct_types {
            if let Some(idx) = sd.fields.iter().position(|(n, _)| n == fname) {
                let here = (idx, self.boundary_kind(&Some(sd.fields[idx].1.clone())));
                match found {
                    None => found = Some(here),
                    Some(seen) if seen == here => {}
                    Some(_) => return None,
                }
            }
        }
        found
    }

    /// A struct field of an object of kind `obj_kind`: through its struct type when known, else by name.
    fn field_of(&self, obj_kind: Kind, field: u32) -> Option<(usize, Kind)> {
        match self.as_composite(obj_kind) {
            Some(Composite::Struct(_)) => self.resolve_field(obj_kind, field),
            _ if obj_kind == Kind::Unknown => self.field_by_name_anywhere(field),
            _ => None,
        }
    }

    /// A declared boundary type → [`Kind`], interning Map/Tuple composites (delegates scalars/seqs/sets
    /// to the free [`boundary_to_kind`]).
    fn boundary_kind(&self, ty: &Option<BoundaryType>) -> Kind {
        if ty.as_ref().is_some_and(crate::vm::tower::declared) {
            return Kind::Tower;
        }
        match ty {
            Some(BoundaryType::Map(k, v)) => {
                let kk = self.boundary_kind(&Some((**k).clone()));
                let vk = self.boundary_kind(&Some((**v).clone()));
                self.map_of(kk, vk)
            }
            Some(BoundaryType::Tuple(positions)) => {
                let ks: Vec<Kind> = positions
                    .iter()
                    .map(|p| self.boundary_kind(&Some(p.clone())))
                    .collect();
                self.tuple_of(ks)
            }
            Some(BoundaryType::Struct(name)) => {
                self.intern_composite(Composite::Struct(name.clone()))
            }
            Some(BoundaryType::Enum(name)) => self.intern_composite(Composite::Enum(name.clone())),
            Some(BoundaryType::Seq(elem)) => {
                let k = self.boundary_kind(&Some((**elem).clone()));
                self.seq_of_kind(k)
            }
            Some(BoundaryType::Option(inner)) => {
                let k = self.boundary_kind(&Some((**inner).clone()));
                self.intern_composite(Composite::Option(Some(k)))
            }
            Some(BoundaryType::Result(ok, err)) => {
                let (ok, err) = (
                    self.boundary_kind(&Some((**ok).clone())),
                    self.boundary_kind(&Some((**err).clone())),
                );
                self.intern_composite(Composite::Result(Some(ok), Some(err)))
            }
            other => boundary_to_kind(other),
        }
    }

    fn emit_region(&mut self, r: &Region) -> Emit {
        let per_pc = self.infer_kinds(r);
        (self.may_be_nothing, self.fails_fast) = self.may_hold_nothing(r);
        self.nothing_literals = self.nothing_literals_of(r);
        self.flag_base = 8 * r.frame_regs as i64;
        let flags = if self.may_be_nothing.is_empty() {
            0
        } else {
            r.frame_regs
        };
        self.owned = r
            .pcs()
            .into_iter()
            .filter_map(|pc| match mutation(&self.prog.code[pc]) {
                Some(Mutation::Owned(reg) | Mutation::Field(reg)) => Some(reg),
                _ => text_append(&self.prog.code[pc])
                    .map(|(reg, _)| reg)
                    .filter(|&reg| kind_at(&per_pc[&pc], reg) == Kind::Text),
            })
            .collect();
        self.field_plan = self.field_plan(r, &per_pc);
        self.reports_nothing = r.function().is_some_and(|fi| {
            self.answers_nothing[fi]
                || (self.closure_bodies[fi]
                    && self.a_closure_answers_nothing(self.prog.functions[fi].param_count))
        });
        self.owned_base = 8 * (r.frame_regs + flags) as i64;
        let owned_flags = if self.owned.is_empty() {
            0
        } else {
            r.frame_regs
        };
        let field_base = 8 * (r.frame_regs + flags + owned_flags) as i64;
        self.field_flags = self
            .field_plan
            .cells
            .iter()
            .enumerate()
            .map(|(i, &cell)| (cell, field_base + 8 * i as i64))
            .collect();
        self.scratch_base = field_base + 8 * self.field_flags.len() as i64;
        let scratch = if self.tower || self.rationals { SCRATCH_SLOTS } else { 0 };
        self.current_fn = r.function();
        let frame = frame_size(r.frame_regs + flags + owned_flags + self.field_flags.len() + scratch);

        self.out.push_str(&format!("\n{}:\n", r.label));
        // Prologue: allocate the frame, save ra, spill incoming params to their slots.
        self.adjust_sp(-frame);
        self.save_ra(frame);
        // When `main` IS the C entry (no scheduler), a0/a1 hold argc/argv — stash them for `args()` before
        // any op clobbers them. Under the scheduler this region is a task, and the bootstrap `main:` does it.
        if r.is_main && !self.need_scheduler {
            self.line("call logos_set_args");
        }
        self.inout = r.function().map(|fi| self.inout_params(fi)).unwrap_or_default();
        if self.inout.len() > 6 {
            return Err(RiscVLowerError::Unsupported(
                "function with more than six mutable parameters (each is handed back in a2..a7)",
            ));
        }
        let max_params = if r.captures.is_empty() { 8 } else { 7 };
        if r.params > max_params {
            return Err(RiscVLowerError::Unsupported(
                "function with too many parameters (closure needs a spare arg reg)",
            ));
        }
        for k in 0..r.params {
            self.store_slot(k, &format!("a{k}"));
        }
        for reg in self.may_be_nothing.clone() {
            self.store_flag(reg, "zero");
        }
        for reg in self.owned.clone() {
            self.store_owned(reg, "zero");
        }
        for off in self.field_flags.values().copied().collect::<Vec<_>>() {
            self.store_at_sp(off, "zero");
        }
        // Closure body: the object pointer arrives in a{params} (the hidden arg after the params). Load
        // its captured values into slots params..params+cap and the present flags into the next cap slots
        // — the layout the VM's `CallValue` binding establishes.
        if !r.captures.is_empty() {
            let ptr = format!("a{}", r.params);
            let cap = r.captures.len() as u16;
            for k in 0..cap {
                self.line(&format!("ld t0, {}({ptr})", 8 * (1 + k)));
                self.store_slot(r.params + k, "t0");
                self.line(&format!("ld t0, {}({ptr})", 8 * (1 + cap + k)));
                self.store_slot(r.params + cap + k, "t0");
            }
        }
        // A register the region takes in as another kind and holds as the extension's value becomes one on entry.
        let entries = r
            .pcs()
            .first()
            .and_then(|entry| self.tower_entries.borrow().get(entry).cloned())
            .unwrap_or_default();
        for (reg, k) in entries {
            let shape = self.tower_shape(k)?;
            self.load_slot(reg, "a0");
            self.line(&format!("la a1, {shape}"));
            self.line("call logos_tower_of");
            self.store_slot(reg, "a0");
        }
        // A register a number join widens is converted as the region begins; a variable no machine form holds
        // refuses the region by name.
        let (widened, refused) = r
            .pcs()
            .first()
            .and_then(|entry| self.number_entries.borrow().get(entry).cloned())
            .unwrap_or_default();
        if !refused.is_empty() {
            return Err(RiscVLowerError::Unsupported(
                "number variable holding an exact number and a real where control merges",
            ));
        }
        for (reg, from, to) in widened {
            self.land_number(reg, from, to)?;
        }

        self.region_pcs = r.pcs();
        self.failing_reads = r.never.failing_reads.clone();
        let mut after_dead = false;
        for pc in r.pcs() {
            self.pc = pc;
            self.out.push_str(&format!(".Lpc_{pc}:\n"));
            // Code that never runs is not lowered; control that reached it would be a fault of the analysis,
            // so the first op of each such stretch stops the program instead of running into what follows.
            if r.never.dead.contains(&pc) {
                if !after_dead {
                    let stop = self.fail_stub(NEVER_RUNS);
                    self.jump_to_stop(None, &stop);
                }
                after_dead = true;
                continue;
            }
            after_dead = false;
            let op = self.prog.code[pc];
            self.fx = OwnershipEffect::default();
            if self.tower {
                self.lower_over_tower(pc, &op, &per_pc[&pc], frame)?;
            } else {
                let seen = self.present_numbers(&op, &per_pc[&pc])?;
                let lowered = self.lower_op_over_nothing(pc, &op, &seen, frame);
                self.presented.clear();
                lowered?;
            }
            // A definition a number join lands lands converted; a collection changed in place took its item
            // converted as it went in.
            let landing = self.number_from.borrow().get(&pc).copied();
            if let (Some((from, to)), Some(w)) = (landing, crate::vm::variables::defined(&op)) {
                if !crate::vm::variables::changes_in_place(&op) {
                    self.land_number(w, from, to)?;
                }
            }
            self.write_owned_flags(&op);
            self.write_field_flags(pc, &op, &per_pc[&pc]);
        }

        // Safety epilogue if the region falls through (well-formed regions end in Return/Halt).
        self.emit_return_nothing(frame);
        Ok(())
    }

    // ---- value-or-nothing registers ------------------------------------------

    /// Which functions a call of can answer `nothing`: one whose entry reaches a `ReturnNothing` or the
    /// region's end (a body that can finish without returning a value, or one the front end gives no body),
    /// or a `Return` of a register that may hold `nothing` — among them the value of a call of a function
    /// that can. A fixpoint over the call graph.
    fn derive_answers_nothing(&mut self, regions: &[Region]) {
        self.answers_nothing = vec![false; self.prog.functions.len()];
        loop {
            let mut changed = false;
            for r in regions {
                let Some(fi) = r.function() else {
                    continue;
                };
                if self.answers_nothing[fi] {
                    continue;
                }
                let (may, _) = self.may_hold_nothing(r);
                let literals = self.nothing_literals_of(r);
                let (reached, falls_off) = self.reached_pcs(r);
                let answers = falls_off
                    || reached.into_iter().any(|pc| match self.prog.code[pc] {
                        Op::ReturnNothing => true,
                        Op::Return { src } => may.contains(&src) || literals[&pc].contains(&src),
                        _ => false,
                    });
                if answers {
                    self.answers_nothing[fi] = true;
                    changed = true;
                }
            }
            if !changed {
                return;
            }
        }
    }

    /// Whether a call through a closure taking `arity` arguments can answer `nothing`: whether a closure body
    /// the program can enter that takes `arity` arguments can.
    fn a_closure_answers_nothing(&self, arity: u16) -> bool {
        self.prog.functions.iter().enumerate().any(|(fi, f)| {
            self.closure_bodies[fi]
                && self.running[fi]
                && f.param_count == arity
                && self.answers_nothing[fi]
        })
    }

    /// The pcs of region `r` control reaches from its entry ([`Emitter::successors_in`]), and whether
    /// control can run past the region's last op.
    fn reached_pcs(&self, r: &Region) -> (Vec<usize>, bool) {
        let (seen, falls_off) = self.walk(r, &r.never);
        (seen.into_iter().collect(), falls_off)
    }

    /// The pcs control can reach from the op at `pc`, given what `never` knows: a loop over a collection with
    /// no item goes straight to its exit, and an item read from one never completes.
    fn successors_in(&self, pc: usize, never: &NeverRuns) -> Vec<usize> {
        match self.prog.code[pc] {
            Op::IterNext { exit, .. } if never.empty_loops.contains(&pc) => vec![exit],
            _ if never.failing_reads.contains(&pc) => Vec::new(),
            ref op => successors(op, pc),
        }
    }

    /// The pcs of region `r` control reaches from its entry given what `never` knows, and whether control can
    /// run past the region's last op.
    fn walk(&self, r: &Region, never: &NeverRuns) -> (std::collections::BTreeSet<usize>, bool) {
        let owned: std::collections::HashSet<usize> = r.pcs().into_iter().collect();
        let mut seen = std::collections::BTreeSet::new();
        let mut falls_off = false;
        let mut stack: Vec<usize> = r
            .intervals
            .first()
            .map(|&(entry, _)| entry)
            .into_iter()
            .collect();
        while let Some(pc) = stack.pop() {
            if !owned.contains(&pc) {
                falls_off = true;
                continue;
            }
            if seen.insert(pc) {
                stack.extend(self.successors_in(pc, never));
            }
        }
        (seen, falls_off)
    }

    /// What of region `r` never runs. Each round finds, over the control flow the round before left
    /// ([`Emitter::item_less_before`]), the loops and item reads whose collection has no item on every path
    /// that reaches them, and the code control no longer reaches once those loops never enter their bodies and
    /// those reads never complete. A round only removes paths, so each finds at least what the one before
    /// found; the rounds stop where two agree, and no later than one round per pc.
    fn never_runs(&self, r: &Region) -> NeverRuns {
        let mut never = NeverRuns::default();
        let prepares = self.loop_prepares(r);
        for _ in 0..=r.pcs().len() {
            let item_less = self.item_less_before(r, &never);
            let has_no_item =
                |pc: usize, reg: u16| item_less.get(&pc).is_some_and(|regs| regs.contains(&reg));
            let mut next = NeverRuns::default();
            for (&next_pc, &prepare_pc) in &prepares {
                if let Op::IterPrepare { iterable } = self.prog.code[prepare_pc] {
                    if has_no_item(prepare_pc, iterable) {
                        next.empty_loops.insert(next_pc);
                    }
                }
            }
            for pc in r.pcs() {
                if let Op::Index { collection, .. } | Op::IndexUnchecked { collection, .. } =
                    self.prog.code[pc]
                {
                    if has_no_item(pc, collection) {
                        next.failing_reads.insert(pc);
                    }
                }
            }
            let (reached, _) = self.walk(r, &next);
            next.dead = r
                .pcs()
                .into_iter()
                .filter(|pc| !reached.contains(pc))
                .collect();
            let settled =
                next.empty_loops == never.empty_loops && next.failing_reads == never.failing_reads;
            never = next;
            if settled {
                break;
            }
        }
        never
    }

    /// Each `IterNext` pc of region `r` with the pc of its loop's `IterPrepare` — the innermost one open
    /// before it.
    fn loop_prepares(&self, r: &Region) -> std::collections::BTreeMap<usize, usize> {
        let mut open: Vec<usize> = Vec::new();
        let mut pairs = std::collections::BTreeMap::new();
        for pc in r.pcs() {
            match self.prog.code[pc] {
                Op::IterPrepare { .. } => open.push(pc),
                Op::IterNext { .. } => {
                    if let Some(&prepare) = open.last() {
                        pairs.insert(pc, prepare);
                    }
                }
                Op::IterPop => {
                    open.pop();
                }
                _ => {}
            }
        }
        pairs
    }

    /// The registers that hold a collection with no item before each pc of region `r` that control reaches
    /// (given what `never` knows), on every path that reaches it: an untyped empty literal (`[]`), or a copy
    /// of a register holding one, not since written or changed and not since read by an op that could keep a
    /// hold of it — any op but an iteration, an item read, a length, a search, a `Show`, a comparison or a
    /// copy. Only the registers some empty literal can reach are followed.
    fn item_less_before(
        &self,
        r: &Region,
        never: &NeverRuns,
    ) -> std::collections::HashMap<usize, std::collections::BTreeSet<u16>> {
        let pcs = r.pcs();
        let mut followed: std::collections::BTreeSet<u16> = pcs
            .iter()
            .filter_map(|&pc| match self.prog.code[pc] {
                Op::NewEmptyList { dst }
                | Op::NewEmptyListWithCapacity { dst, .. }
                | Op::NewEmptyListFixed { dst, .. }
                | Op::NewList { dst, count: 0, .. } => Some(dst),
                _ => None,
            })
            .collect();
        if followed.is_empty() {
            return std::collections::HashMap::new();
        }
        loop {
            let before = followed.len();
            for &pc in &pcs {
                if let Op::Move { dst, src } = self.prog.code[pc] {
                    if followed.contains(&src) {
                        followed.insert(dst);
                    }
                }
            }
            if followed.len() == before {
                break;
            }
        }
        let owned: std::collections::HashSet<usize> = pcs.iter().copied().collect();
        let mut before: std::collections::HashMap<usize, std::collections::BTreeSet<u16>> =
            std::collections::HashMap::new();
        let Some(&(entry, _)) = r.intervals.first() else {
            return before;
        };
        before.insert(entry, std::collections::BTreeSet::new());
        let mut work = vec![entry];
        while let Some(pc) = work.pop() {
            let after = self.item_less_after(pc, &before[&pc], &followed);
            for next in self.successors_in(pc, never) {
                if !owned.contains(&next) {
                    continue;
                }
                let merged = match before.get(&next) {
                    None => after.clone(),
                    Some(known) => known.intersection(&after).copied().collect(),
                };
                if before.get(&next) != Some(&merged) {
                    before.insert(next, merged);
                    work.push(next);
                }
            }
        }
        before
    }

    /// The item-less registers after the op at `pc`, given those before it ([`Emitter::item_less_before`]).
    fn item_less_after(
        &self,
        pc: usize,
        before: &std::collections::BTreeSet<u16>,
        followed: &std::collections::BTreeSet<u16>,
    ) -> std::collections::BTreeSet<u16> {
        let op = self.prog.code[pc];
        let mut after = before.clone();
        match op {
            Op::NewEmptyList { dst }
            | Op::NewEmptyListWithCapacity { dst, .. }
            | Op::NewEmptyListFixed { dst, .. }
            | Op::NewList { dst, count: 0, .. } => {
                after.insert(dst);
            }
            Op::Move { dst, src } => {
                if before.contains(&src) && followed.contains(&dst) {
                    after.insert(dst);
                } else {
                    after.remove(&dst);
                }
            }
            _ => {
                self.for_each_written(&op, |w| {
                    after.remove(&w);
                });
                if let Some(Mutation::Owned(reg)) = mutation(&op) {
                    after.remove(&reg);
                }
                let keeps_no_hold = matches!(
                    op,
                    Op::IterPrepare { .. }
                        | Op::Index { .. }
                        | Op::IndexUnchecked { .. }
                        | Op::Length { .. }
                        | Op::Contains { .. }
                        | Op::Show { .. }
                        | Op::Eq { .. }
                        | Op::NotEq { .. }
                );
                if !keeps_no_hold {
                    op.for_each_read_reg(
                        |f| self.prog.functions[f as usize].captures.len(),
                        |reg| {
                            after.remove(&reg);
                        },
                    );
                }
            }
        }
        after
    }

    /// The region's field-ownership plan ([`FieldPlan`]): the `(register, cell)` pairs its field-mutating ops
    /// change, and, from a forward walk of which registers alias which of those cells and a backward walk of
    /// which registers are live, the ops that must copy and the reads that let an alias keep the value.
    fn field_plan(
        &self,
        r: &Region,
        per_pc: &std::collections::HashMap<usize, Vec<Kind>>,
    ) -> FieldPlan {
        let mut plan = FieldPlan::default();
        for pc in r.live_pcs() {
            if let Op::ListPushField { obj, field, .. } | Op::CrdtBump { obj, field, .. } =
                self.prog.code[pc]
            {
                if let Some((cell, _)) = self.resolve_field(kind_at(&per_pc[&pc], obj), field) {
                    plan.cells.insert((obj, cell));
                }
            }
        }
        if plan.cells.is_empty() {
            return plan;
        }
        let live = self.live_in(r);
        let owned: std::collections::HashSet<usize> = r.pcs().into_iter().collect();
        let Some(&(entry, _)) = r.intervals.first() else {
            return plan;
        };
        let mut state_in: std::collections::HashMap<usize, Aliases> =
            std::collections::HashMap::new();
        state_in.insert(entry, Aliases::new());
        let mut work = vec![entry];
        while let Some(pc) = work.pop() {
            let out = self.alias_transfer(pc, &state_in[&pc], &plan.cells, per_pc);
            for next in self.successors_in(pc, &r.never) {
                if !owned.contains(&next) {
                    continue;
                }
                let merged = match state_in.get(&next) {
                    None => out.clone(),
                    Some(known) => {
                        let mut m = known.clone();
                        for (reg, cells) in &out {
                            m.entry(*reg).or_default().extend(cells.iter().copied());
                        }
                        m
                    }
                };
                if state_in.get(&next) != Some(&merged) {
                    state_in.insert(next, merged);
                    work.push(next);
                }
            }
        }
        for (&pc, aliases) in &state_in {
            let op = self.prog.code[pc];
            if let Op::ListPushField { obj, field, .. } | Op::CrdtBump { obj, field, .. } = op {
                if let Some((cell, _)) = self.resolve_field(kind_at(&per_pc[&pc], obj), field) {
                    if live[&pc]
                        .iter()
                        .any(|a| aliases.get(a).is_some_and(|c| c.contains(&(obj, cell))))
                    {
                        plan.forced.insert(pc);
                    }
                }
            }
            if only_inspects_an_alias(&op) {
                continue;
            }
            let own_copy = match mutation(&op) {
                Some(Mutation::Owned(a)) => Some(a),
                _ => None,
            };
            let mut kept = std::collections::BTreeSet::new();
            op.for_each_read_reg(
                |f| self.prog.functions[f as usize].captures.len(),
                |a| {
                    if Some(a) != own_copy {
                        if let Some(cells) = aliases.get(&a) {
                            kept.extend(cells.iter().copied());
                        }
                    }
                },
            );
            if !kept.is_empty() {
                plan.escapes.insert(pc, kept);
            }
        }
        plan
    }

    /// Which registers alias which field cells after the op at `pc`, given the aliases before it: a read of a
    /// cell out of its record aliases it; a copy aliases what its source does; a register written anew, or
    /// made its own by its own mutation, aliases nothing; and a cell a field op, an insert or a merge replaces
    /// is aliased by nothing.
    fn alias_transfer(
        &self,
        pc: usize,
        before: &Aliases,
        cells: &std::collections::BTreeSet<(u16, usize)>,
        per_pc: &std::collections::HashMap<usize, Vec<Kind>>,
    ) -> Aliases {
        let op = self.prog.code[pc];
        let kinds = &per_pc[&pc];
        let of_record = |obj: u16| cells.iter().copied().filter(move |&(r, _)| r == obj);
        let cell_of = |obj: u16, field: u32| {
            self.resolve_field(kind_at(kinds, obj), field)
                .map(|(c, _)| c)
        };
        let is_record = |reg: u16| {
            matches!(
                self.as_composite(kind_at(kinds, reg)),
                Some(Composite::Struct(_))
            )
        };
        let read_out: Option<(u16, std::collections::BTreeSet<(u16, usize)>)> = match op {
            Op::GetField { dst, obj, field } => Some((
                dst,
                of_record(obj)
                    .filter(|&(_, c)| {
                        self.field_of(kind_at(kinds, obj), field).map(|f| f.0) == Some(c)
                    })
                    .collect(),
            )),
            Op::BindArm {
                dst, target, field, ..
            } if is_record(target)
                && self.text_const(field) != Some(crate::vm::SOME_BINDING_MARKER) =>
            {
                Some((
                    dst,
                    of_record(target)
                        .filter(|&(_, c)| cell_of(target, field) == Some(c))
                        .collect(),
                ))
            }
            Op::Index {
                dst, collection, ..
            }
            | Op::IndexUnchecked {
                dst, collection, ..
            } if is_record(collection) => Some((dst, of_record(collection).collect())),
            Op::Move { dst, src } => Some((dst, before.get(&src).cloned().unwrap_or_default())),
            _ => None,
        };
        let mut after = before.clone();
        let mut cleared: Vec<u16> = Vec::new();
        self.for_each_written(&op, |w| cleared.push(w));
        if let Some(Mutation::Owned(a)) = mutation(&op) {
            after.remove(&a);
        }
        let replaced: Vec<(u16, usize)> = match op {
            Op::ListPushField { obj, field, .. }
            | Op::CrdtBump { obj, field, .. }
            | Op::StructInsert { obj, field, .. } => {
                cell_of(obj, field).map(|c| (obj, c)).into_iter().collect()
            }
            Op::CrdtMerge { target, .. } | Op::CrdtJoin { target, .. } => {
                of_record(target).collect()
            }
            _ => Vec::new(),
        };
        for w in &cleared {
            after.remove(w);
        }
        for cells in after.values_mut() {
            cells.retain(|cell| !replaced.contains(cell) && !cleared.contains(&cell.0));
        }
        if let Some((dst, read)) = read_out {
            if !read.is_empty() {
                after.insert(dst, read);
            }
        }
        after.retain(|_, cells| !cells.is_empty());
        after
    }

    /// The registers live before each pc of region `r` that control reaches: read there, or read later
    /// before they are written.
    fn live_in(
        &self,
        r: &Region,
    ) -> std::collections::HashMap<usize, std::collections::BTreeSet<u16>> {
        let pcs = r.pcs();
        let owned: std::collections::HashSet<usize> = pcs.iter().copied().collect();
        let mut live: std::collections::HashMap<usize, std::collections::BTreeSet<u16>> = pcs
            .iter()
            .map(|&pc| (pc, std::collections::BTreeSet::new()))
            .collect();
        loop {
            let mut changed = false;
            for &pc in pcs.iter().rev() {
                let op = self.prog.code[pc];
                let mut now: std::collections::BTreeSet<u16> = self
                    .successors_in(pc, &r.never)
                    .into_iter()
                    .filter(|next| owned.contains(next))
                    .flat_map(|next| live[&next].clone())
                    .collect();
                self.for_each_written(&op, |w| {
                    now.remove(&w);
                });
                op.for_each_read_reg(
                    |f| self.prog.functions[f as usize].captures.len(),
                    |reg| {
                        now.insert(reg);
                    },
                );
                if live[&pc] != now {
                    live.insert(pc, now);
                    changed = true;
                }
            }
            if !changed {
                return live;
            }
        }
    }

    /// After the op at `pc`: clear the field flags of the cells an alias may now keep, the cell an insert
    /// replaces, and every cell of both records of a merge (the target takes the source's values).
    fn write_field_flags(&mut self, pc: usize, op: &Op, kinds: &[Kind]) {
        if self.field_flags.is_empty() {
            return;
        }
        let mut clear: std::collections::BTreeSet<(u16, usize)> = self
            .field_plan
            .escapes
            .get(&pc)
            .cloned()
            .unwrap_or_default();
        match *op {
            Op::StructInsert { obj, field, .. } => {
                if let Some((cell, _)) = self.resolve_field(kind_at(kinds, obj), field) {
                    clear.insert((obj, cell));
                }
            }
            Op::CrdtMerge { target, source } | Op::CrdtJoin { target, source } => {
                clear.extend(
                    self.field_flags
                        .keys()
                        .copied()
                        .filter(|&(r, _)| r == target || r == source),
                );
            }
            _ => {}
        }
        for cell in clear {
            if let Some(&off) = self.field_flags.get(&cell) {
                self.store_at_sp(off, "zero");
            }
        }
    }

    /// Clear every field flag of register `reg`: its owned flag was just written, so it may no longer hold the
    /// record whose fields the flags describe, or hold it alone.
    fn clear_field_flags(&mut self, reg: u16) {
        let offs: Vec<i64> = self
            .field_flags
            .iter()
            .filter(|((r, _), _)| *r == reg)
            .map(|(_, &off)| off)
            .collect();
        for off in offs {
            self.store_at_sp(off, "zero");
        }
    }

    /// The region's registers that may hold `nothing` at run time (see `Emitter::may_be_nothing`), and the
    /// definitions that never leave `nothing` behind ([`Emitter::fail_fast`]).
    fn may_hold_nothing(
        &self,
        r: &Region,
    ) -> (
        std::collections::BTreeSet<u16>,
        std::collections::HashMap<usize, String>,
    ) {
        let pcs = r.pcs();
        let defs: Vec<(usize, u16)> = pcs
            .iter()
            .filter_map(|&pc| match self.prog.code[pc] {
                Op::ChanRecv { dst, .. } if self.closable => Some((pc, dst)),
                Op::SelectArmRecv { var, .. } if self.closable => Some((pc, var)),
                Op::ChanTryRecv { dst, .. }
                | Op::TaskAwait { dst, .. }
                | Op::ListPop { dst, .. } => Some((pc, dst)),
                Op::Call { dst, func, .. } if self.answers_nothing[func as usize] => {
                    Some((pc, dst))
                }
                Op::CallValue { dst, arg_count, .. }
                    if self.a_closure_answers_nothing(arg_count) =>
                {
                    Some((pc, dst))
                }
                _ => None,
            })
            .collect();
        let closure = |seeds: &mut dyn Iterator<Item = u16>| {
            let mut may: std::collections::BTreeSet<u16> = seeds.collect();
            loop {
                let before = may.len();
                for &pc in &pcs {
                    if let Op::Move { dst, src } = self.prog.code[pc] {
                        if may.contains(&src) {
                            may.insert(dst);
                        }
                    }
                }
                if may.len() == before {
                    return may;
                }
            }
        };
        let everything = closure(&mut defs.iter().map(|&(_, reg)| reg));
        let fail_fast = self.fail_fast(r, &pcs, &defs, &everything);
        let may = closure(
            &mut defs
                .iter()
                .filter(|(pc, _)| !fail_fast.contains_key(pc))
                .map(|&(_, reg)| reg),
        );
        (may, fail_fast)
    }

    /// The pops and calls whose value the very next op reads in a way that fails on `nothing`, before anything
    /// else can happen, with that op's sentence: such a definition stops the program itself when it yields
    /// `nothing` (a cold branch), so its register holds a value whenever the program goes on and needs no
    /// flag. A pop or a call always goes on to the op after it, so that op must only be in the region, take the
    /// value as one operand and, as the other, a register that holds a value (`may` is every register that might
    /// hold `nothing` otherwise). Another path into that op does not matter: it does not run the pop.
    fn fail_fast(
        &self,
        r: &Region,
        pcs: &[usize],
        defs: &[(usize, u16)],
        may: &std::collections::BTreeSet<u16>,
    ) -> std::collections::HashMap<usize, String> {
        use crate::vm::Value;
        let mut out = std::collections::HashMap::new();
        let kinds = self.infer_kinds(r);
        let literals = self.nothing_literals_of(r);
        for &(pc, dst) in defs {
            if !matches!(
                self.prog.code[pc],
                Op::ListPop { .. } | Op::Call { .. } | Op::CallValue { .. }
            ) {
                continue;
            }
            let next = pc + 1;
            if !pcs.contains(&next) {
                continue;
            }
            let Some((lhs, rhs, vm_op)) = vm_binary_op(&self.prog.code[next]) else {
                continue;
            };
            let other = match (lhs == dst, rhs == dst) {
                (true, false) => rhs,
                (false, true) => lhs,
                _ => continue,
            };
            if may.contains(&other) || literals[&next].contains(&other) {
                continue;
            }
            let Some(value) = self
                .exemplar(kind_at(&kinds[&next], other))
                .map(Value::from_runtime)
            else {
                continue;
            };
            let refusal = if lhs == dst {
                vm_op(&Value::nothing(), &value)
            } else {
                vm_op(&value, &Value::nothing())
            };
            if let Err(sentence) = refusal {
                out.insert(pc, sentence);
            }
        }
        out
    }

    /// The registers holding the `nothing` literal before each pc of the region.
    fn nothing_literals_of(
        &self,
        r: &Region,
    ) -> std::collections::HashMap<usize, std::collections::BTreeSet<u16>> {
        let mut cur = std::collections::BTreeSet::new();
        let mut per_pc = std::collections::HashMap::new();
        for pc in r.pcs() {
            per_pc.insert(pc, cur.clone());
            let op = self.prog.code[pc];
            let copies_literal = matches!(op, Op::Move { src, .. } if cur.contains(&src));
            self.for_each_written(&op, |w| {
                cur.remove(&w);
            });
            match op {
                Op::LoadConst { dst, idx }
                    if matches!(self.prog.constants[idx as usize], Constant::Nothing) =>
                {
                    cur.insert(dst);
                }
                Op::Move { dst, .. } if copies_literal => {
                    cur.insert(dst);
                }
                _ => {}
            }
        }
        per_pc
    }

    /// After a call into `dst`: when the callee reports its value's `nothing` flag in `a1` (`reported`), a
    /// fail-fast call stops the program on it and any other keeps it in `dst`'s flag slot.
    fn nothing_after_call(&mut self, dst: u16, reported: bool) {
        if let Some(reason) = self.fails_fast.get(&self.pc).cloned() {
            let fail = self.fail_stub(&reason);
            self.jump_to_stop(Some(("bnez", "a1")), &fail);
            if self.may_be_nothing.contains(&dst) {
                self.store_flag(dst, "zero");
            }
        } else if self.may_be_nothing.contains(&dst) {
            self.store_flag(dst, if reported { "a1" } else { "zero" });
        }
    }

    fn store_flag(&mut self, reg: u16, from: &str) {
        self.store_at_sp(self.flag_base + 8 * reg as i64, from);
    }

    // ---- owned collections -------------------------------------------------

    fn store_owned(&mut self, reg: u16, from: &str) {
        self.store_at_sp(self.owned_base + 8 * reg as i64, from);
    }

    /// The address of `reg`'s owned flag, into `into`.
    fn owned_addr(&mut self, reg: u16, into: &str) {
        let off = self.owned_base + 8 * reg as i64;
        if fits_imm12(off) {
            self.line(&format!("addi {into}, sp, {off}"));
        } else {
            self.line(&format!("li {into}, {off}"));
            self.line(&format!("add {into}, sp, {into}"));
        }
    }

    /// Declare that the op writes `regs` with collections it just made.
    fn fresh(&mut self, regs: &[u16]) {
        self.fx.fresh.extend_from_slice(regs);
    }

    /// Declare that the op reads `regs` without keeping their handles anywhere.
    fn inspects(&mut self, regs: &[u16]) {
        self.fx.inspected.extend_from_slice(regs);
    }

    /// Declare that the op leaves `reg` holding the handle it held, with its ownership unchanged.
    fn keeps(&mut self, reg: u16) {
        self.fx.keeps.push(reg);
    }

    /// Set up `a0` = the address of `target`'s slot and `a1` = the address of its owned flag, the first two
    /// arguments of every `_own` runtime helper; the op's other operands go in `a2`.. and are read by value
    /// before the helper runs. A target that is also one of those operands is being stored into itself, so
    /// its flag is cleared first: the helper then copies, and what it stores is the collection as it was.
    fn own_target(&mut self, target: u16, operands: &[u16]) {
        if operands.contains(&target) {
            self.store_owned(target, "zero");
            self.clear_field_flags(target);
        }
        self.slot_addr(target, "a0");
        self.owned_addr(target, "a1");
        self.keeps(target);
    }

    /// Set up the first four arguments of every `logos_field_*_own` helper for field `cell` of the record in
    /// `obj`: `a0`/`a1` its slot and owned flag ([`Emitter::own_target`]), `a2` the field flag, `a3` the 1-based
    /// cell. A field op an alias of the cell is live across ([`FieldPlan::forced`]) clears the flag first.
    fn field_target(&mut self, obj: u16, cell: usize, operands: &[u16]) {
        let flag = *self
            .field_flags
            .get(&(obj, cell))
            .expect("every field op's cell has a flag");
        if self.field_plan.forced.contains(&self.pc) || operands.contains(&obj) {
            self.store_at_sp(flag, "zero");
        }
        self.own_target(obj, operands);
        if fits_imm12(flag) {
            self.line(&format!("addi a2, sp, {flag}"));
        } else {
            self.line(&format!("li a2, {flag}"));
            self.line("add a2, sp, a2");
        }
        self.line(&format!("li a3, {}", cell + 1));
    }

    /// Build a fresh seq in `dst`: the optional `head` cell (an enum's variant tag), then the `count`
    /// registers from `start` in order. The seq is new, so its cells are pushed in place.
    fn new_seq_of(&mut self, dst: u16, head: Option<i64>, start: u16, count: u16) {
        self.line("call logos_seq_new");
        self.store_slot(dst, "a0");
        if let Some(tag) = head {
            self.load_slot(dst, "a0");
            self.line(&format!("li a1, {tag}"));
            self.line("call logos_seq_push_inplace");
        }
        for k in 0..count {
            self.load_slot(dst, "a0");
            self.load_slot(start + k, "a1");
            self.line("call logos_seq_push_inplace");
        }
        self.fresh(&[dst]);
    }

    /// Keep the owned flags true after `op`: a register `op` wrote holds a fresh collection (flag 1), its
    /// handle as before (flag unchanged), or a handle that may be held elsewhere (flag 0); a register `op`
    /// read and may have left its handle in (another register, a global, a field, an element, an
    /// iterator, a closure, a channel) is no longer the only holder (flag 0). A terminator's frame is gone.
    fn write_owned_flags(&mut self, op: &Op) {
        let fx = std::mem::take(&mut self.fx);
        if self.owned.is_empty()
            || matches!(
                op,
                Op::Return { .. } | Op::ReturnNothing | Op::Halt | Op::FailWith { .. }
            )
        {
            return;
        }
        let mut written = Vec::new();
        self.for_each_written(&op, |w| written.push(w));
        let mut read = Vec::new();
        op.for_each_read_reg(
            |f| self.prog.functions[f as usize].captures.len(),
            |r| read.push(r),
        );
        let mut escaped: Vec<u16> = read
            .into_iter()
            .filter(|r| self.owned.contains(r) && !written.contains(r))
            .filter(|r| !fx.inspected.contains(r) && !fx.keeps.contains(r))
            .collect();
        escaped.sort_unstable();
        escaped.dedup();
        for r in escaped {
            self.store_owned(r, "zero");
            self.clear_field_flags(r);
        }
        written.sort_unstable();
        written.dedup();
        for w in written {
            if !self.owned.contains(&w) || fx.keeps.contains(&w) {
                continue;
            }
            if fx.fresh.contains(&w) {
                self.line("li t0, 1");
                self.store_owned(w, "t0");
            } else {
                self.store_owned(w, "zero");
            }
            self.clear_field_flags(w);
        }
    }

    fn load_flag(&mut self, reg: u16, into: &str) {
        self.load_at_sp(self.flag_base + 8 * reg as i64, into);
    }

    /// `into` = 1 when `reg` holds `nothing` before `pc`, else 0.
    fn nothing_flag(&mut self, pc: usize, reg: u16, into: &str) {
        if self.may_be_nothing.contains(&reg) {
            self.load_flag(reg, into);
        } else if self.is_nothing_literal(pc, reg) {
            self.line(&format!("li {into}, 1"));
        } else {
            self.line(&format!("li {into}, 0"));
        }
    }

    fn is_nothing_literal(&self, pc: usize, reg: u16) -> bool {
        self.nothing_literals
            .get(&pc)
            .is_some_and(|lits| lits.contains(&reg))
    }

    fn fresh_nothing_label(&mut self) -> usize {
        self.nothing_label += 1;
        self.nothing_label
    }

    /// Lower one op of a region whose registers may hold `nothing`. The operations total over `nothing` —
    /// `Show`, text rendering, a copy, `is`/`is not` — read a register's flag; every other read of a
    /// value-or-nothing register is an Int read, guarded to fail on `nothing`. Every write to one sets its
    /// flag.
    fn lower_op_over_nothing(&mut self, pc: usize, op: &Op, kinds: &[Kind], frame: i64) -> Emit {
        let over = |e: &Self, r: u16| e.may_be_nothing.contains(&r) || e.is_nothing_literal(pc, r);
        match *op {
            Op::Show { src } if over(self, src) => {
                self.show_over_nothing(pc, op, src, kinds, frame)?
            }
            Op::Eq { dst, lhs, rhs } | Op::NotEq { dst, lhs, rhs }
                if over(self, lhs) || over(self, rhs) =>
            {
                self.is_over_nothing(pc, op, dst, lhs, rhs, kinds, frame)?
            }
            Op::Concat { dst, lhs, rhs } if over(self, lhs) || over(self, rhs) => {
                self.text_tag_over_nothing(pc, lhs, kinds, "a0", "a1")?;
                self.text_tag_over_nothing(pc, rhs, kinds, "a2", "a3")?;
                self.line("call logos_concat_vals");
                self.store_slot(dst, "a0");
            }
            Op::FormatValue {
                dst,
                src,
                spec,
                debug_prefix,
            } if spec == u32::MAX && debug_prefix == u32::MAX && over(self, src) => {
                self.text_tag_over_nothing(pc, src, kinds, "a0", "a1")?;
                self.line("call logos_val_to_str");
                self.store_slot(dst, "a0");
            }
            Op::RevSwap { a, b }
                if self.may_be_nothing.contains(&a) || self.may_be_nothing.contains(&b) =>
            {
                return Err(RiscVLowerError::Unsupported(
                    "a reversible swap of a value-or-nothing register",
                ));
            }
            // `nothing` matches the `None` arm and no other.
            Op::TestArm {
                dst,
                target,
                variant,
            } if over(self, target) => {
                let none = self.text_const(variant) == Some("None");
                self.over_nothing(target, op, kinds, frame, |e| {
                    e.line(&format!("li t2, {}", none as i64));
                    e.store_slot(dst, "t2");
                })?
            }
            // `nothing` is false: `not nothing` is true, and a condition on it is not met.
            Op::Not { dst, src } if over(self, src) => {
                self.over_nothing(src, op, kinds, frame, |e| {
                    e.line("li t1, 1");
                    e.store_slot(dst, "t1");
                })?
            }
            Op::JumpIfFalse { cond, target } if over(self, cond) => {
                self.over_nothing(cond, op, kinds, frame, |e| {
                    e.line(&format!("j .Lpc_{target}"))
                })?
            }
            Op::JumpIfTrue { cond, .. } if over(self, cond) => {
                self.over_nothing(cond, op, kinds, frame, |_| {})?
            }
            // `+` with a Text operand concatenates, and `nothing` takes part as its text.
            Op::Add { dst, lhs, rhs } if self.text_add_over(pc, lhs, rhs, kinds) => {
                self.concat_over_nothing(pc, dst, lhs, rhs, kinds)?
            }
            Op::AddAssign { dst, src } if self.text_add_over(pc, dst, src, kinds) => {
                self.concat_over_nothing(pc, dst, dst, src, kinds)?
            }
            Op::Move { .. } | Op::Return { .. } => self.lower_op(op, kinds, frame)?,
            _ => {
                self.guard_nothing_reads(op, kinds);
                self.lower_op(op, kinds, frame)?;
            }
        }
        self.write_nothing_flags(pc, op);
        Ok(())
    }

    /// Set the flag of every value-or-nothing register `op` wrote. A receive, `Try to receive`, `Await`, a
    /// select's wait and a call set their own from what they read.
    fn write_nothing_flags(&mut self, pc: usize, op: &Op) {
        if self.may_be_nothing.is_empty() {
            return;
        }
        if matches!(
            op,
            Op::ChanRecv { .. }
                | Op::ChanTryRecv { .. }
                | Op::TaskAwait { .. }
                | Op::SelectArmRecv { .. }
                | Op::ListPop { .. }
                | Op::Call { .. }
                | Op::CallValue { .. }
        ) {
            return;
        }
        let mut written = Vec::new();
        self.for_each_written(&op, |w| written.push(w));
        written.retain(|w| self.may_be_nothing.contains(w));
        for w in written {
            match *op {
                Op::Move { src, .. } => {
                    self.nothing_flag(pc, src, "t0");
                    self.store_flag(w, "t0");
                }
                Op::LoadConst { idx, .. }
                    if matches!(self.prog.constants[idx as usize], Constant::Nothing) =>
                {
                    self.line("li t0, 1");
                    self.store_flag(w, "t0");
                }
                _ => self.store_flag(w, "zero"),
            }
        }
    }

    /// `Show` of a register that may hold `nothing`: `nothing`, else the value by its kind.
    fn show_over_nothing(
        &mut self,
        pc: usize,
        op: &Op,
        src: u16,
        kinds: &[Kind],
        frame: i64,
    ) -> Emit {
        let nothing = self.ro.intern_text("nothing");
        if !self.may_be_nothing.contains(&src) {
            self.line(&format!("la a0, {nothing}"));
            self.line("call logos_show_text");
            return Ok(());
        }
        let n = self.fresh_nothing_label();
        self.load_flag(src, "t0");
        self.line(&format!("beqz t0, .Lnth_{n}_value"));
        self.line(&format!("la a0, {nothing}"));
        self.line("call logos_show_text");
        self.line(&format!("j .Lnth_{n}_done"));
        self.out.push_str(&format!(".Lnth_{n}_value:\n"));
        self.lower_op(op, kinds, frame)?;
        self.out.push_str(&format!(".Lnth_{n}_done:\n"));
        let _ = pc;
        Ok(())
    }

    /// `a is b` / `a is not b` where either may hold `nothing`: equal when both hold `nothing`, or both hold
    /// values that are equal — the values are compared only when both are present.
    fn is_over_nothing(
        &mut self,
        pc: usize,
        op: &Op,
        dst: u16,
        lhs: u16,
        rhs: u16,
        kinds: &[Kind],
        frame: i64,
    ) -> Emit {
        let eq = matches!(op, Op::Eq { .. });
        let literal = self.is_nothing_literal(pc, lhs) || self.is_nothing_literal(pc, rhs);
        // `nothing` equals an `Option` exactly when the Option is `none` — absence is one thing, however it
        // was spelled (the VM's `values_equal`).
        let other = if self.is_nothing_literal(pc, lhs) {
            rhs
        } else {
            lhs
        };
        if literal
            && matches!(
                self.as_composite(self.kind_of(kinds, other)),
                Some(Composite::Option(_))
            )
        {
            self.inspects(&[other]);
            match self.ctor_tag("None") {
                Some(tag) => {
                    self.load_slot(other, "a0");
                    self.line("li a1, 1");
                    self.line("call logos_seq_index");
                    self.line(&format!("li t0, {tag}"));
                    self.line("sub t1, a0, t0");
                    self.line(if eq { "seqz t1, t1" } else { "snez t1, t1" });
                }
                None => self.line(&format!("li t1, {}", (!eq) as i64)),
            }
            self.store_slot(dst, "t1");
            return Ok(());
        }
        let n = self.fresh_nothing_label();
        self.nothing_flag(pc, lhs, "t3");
        self.nothing_flag(pc, rhs, "t4");
        if !literal {
            self.line("or t5, t3, t4");
            self.line(&format!("bnez t5, .Lnth_{n}_absent"));
            self.lower_op(op, kinds, frame)?;
            self.line(&format!("j .Lnth_{n}_done"));
            self.out.push_str(&format!(".Lnth_{n}_absent:\n"));
        }
        self.line("xor t5, t3, t4");
        self.line(if eq { "seqz t5, t5" } else { "snez t5, t5" });
        self.store_slot(dst, "t5");
        if !literal {
            self.out.push_str(&format!(".Lnth_{n}_done:\n"));
        }
        Ok(())
    }

    /// Whether `lhs + rhs` is a concatenation (a Text operand) that reads a register which may hold `nothing`.
    fn text_add_over(&self, pc: usize, lhs: u16, rhs: u16, kinds: &[Kind]) -> bool {
        let over = |r: u16| self.may_be_nothing.contains(&r) || self.is_nothing_literal(pc, r);
        (over(lhs) || over(rhs))
            && (self.kind_of(kinds, lhs) == Kind::Text || self.kind_of(kinds, rhs) == Kind::Text)
    }

    /// `dst = lhs + rhs` as text, either of which may hold `nothing` — rendered `nothing`, as the VM's `add`
    /// renders it when it concatenates.
    fn concat_over_nothing(
        &mut self,
        pc: usize,
        dst: u16,
        lhs: u16,
        rhs: u16,
        kinds: &[Kind],
    ) -> Emit {
        self.text_tag_over_nothing(pc, lhs, kinds, "a0", "a1")?;
        self.text_tag_over_nothing(pc, rhs, kinds, "a2", "a3")?;
        self.line("call logos_concat_vals");
        self.store_slot(dst, "a0");
        Ok(())
    }

    /// Stop the program where `op` reads a register holding `nothing`, as the VM fails it. An arithmetic or
    /// ordering op fails with the sentence the VM's own `Value` operation gives it for the operands that hold
    /// `nothing` and the kinds of the others; any other op with `.Lnothing`'s.
    fn guard_nothing_reads(&mut self, op: &Op, kinds: &[Kind]) {
        if self.may_be_nothing.is_empty() {
            return;
        }
        let mut reads = Vec::new();
        op.for_each_read_reg(
            |f| self.prog.functions[f as usize].captures.len(),
            |r| reads.push(r),
        );
        reads.sort_unstable();
        reads.dedup();
        reads.retain(|r| self.may_be_nothing.contains(r));
        if reads.is_empty() {
            return;
        }
        if let Some((lhs, rhs, vm_op)) = vm_binary_op(op) {
            use crate::vm::Value;
            let (l, r) = (
                self.may_be_nothing.contains(&lhs),
                self.may_be_nothing.contains(&rhs),
            );
            let value = |k: Kind| self.exemplar(k).map(Value::from_runtime);
            let (lv, rv) = (
                value(self.kind_of(kinds, lhs)),
                value(self.kind_of(kinds, rhs)),
            );
            let refusal = |a: Option<Value>, b: Option<Value>| {
                a.zip(b).and_then(|(a, b)| vm_op(&a, &b).err())
            };
            let both = refusal(Some(Value::nothing()), Some(Value::nothing()));
            let left = if lhs == rhs {
                both.clone()
            } else {
                refusal(Some(Value::nothing()), rv)
            };
            let right = if lhs == rhs {
                both.clone()
            } else {
                refusal(lv, Some(Value::nothing()))
            };
            if (!(l && r) || both.is_some()) && (!l || left.is_some()) && (!r || right.is_some()) {
                if l {
                    self.load_flag(lhs, "t0");
                }
                if r {
                    self.load_flag(rhs, "t1");
                }
                if l && r {
                    let fail = self.fail_stub(&both.expect("checked above"));
                    self.line("and t2, t0, t1");
                    self.jump_to_stop(Some(("bnez", "t2")), &fail);
                }
                if l {
                    let fail = self.fail_stub(&left.expect("checked above"));
                    self.jump_to_stop(Some(("bnez", "t0")), &fail);
                }
                if r {
                    let fail = self.fail_stub(&right.expect("checked above"));
                    self.jump_to_stop(Some(("bnez", "t1")), &fail);
                }
                return;
            }
        }
        for r in reads {
            self.need_nothing_fail = true;
            self.load_flag(r, "t0");
            self.jump_to_stop(Some(("bnez", "t0")), ".Lnothing");
        }
    }

    /// `op` over a register that may hold `nothing`: `on_nothing` while it holds `nothing`, `op` as any value
    /// takes it otherwise.
    fn over_nothing(
        &mut self,
        reg: u16,
        op: &Op,
        kinds: &[Kind],
        frame: i64,
        on_nothing: impl FnOnce(&mut Self),
    ) -> Emit {
        if !self.may_be_nothing.contains(&reg) {
            on_nothing(self);
            return Ok(());
        }
        let n = self.fresh_nothing_label();
        self.load_flag(reg, "t2");
        self.line(&format!("beqz t2, .Lnth_{n}_value"));
        on_nothing(self);
        self.line(&format!("j .Lnth_{n}_done"));
        self.out.push_str(&format!(".Lnth_{n}_value:\n"));
        self.lower_op(op, kinds, frame)?;
        self.out.push_str(&format!(".Lnth_{n}_done:\n"));
        Ok(())
    }

    /// Load `reg` into `val` and its `logos_val_to_str` tag into `tag`: its kind's, or `NOTHING_TAG` while it
    /// holds `nothing`.
    fn text_tag_over_nothing(
        &mut self,
        pc: usize,
        reg: u16,
        kinds: &[Kind],
        val: &str,
        tag: &str,
    ) -> Emit {
        self.load_slot(reg, val);
        if self.is_nothing_literal(pc, reg) && !self.may_be_nothing.contains(&reg) {
            self.line(&format!("li {tag}, {NOTHING_TAG}"));
            return Ok(());
        }
        let t = kind_tag(self.kind_of(kinds, reg))?;
        self.line(&format!("li {tag}, {t}"));
        if self.may_be_nothing.contains(&reg) {
            let n = self.fresh_nothing_label();
            self.load_flag(reg, "t0");
            self.line(&format!("beqz t0, .Lnth_{n}_done"));
            self.line(&format!("li {tag}, {NOTHING_TAG}"));
            self.out.push_str(&format!(".Lnth_{n}_done:\n"));
        }
        Ok(())
    }

    // ---- per-op lowering ---------------------------------------------------

    fn lower_op(&mut self, op: &Op, kinds: &[Kind], frame: i64) -> Emit {
        match op {
            Op::LoadConst { dst, idx } => {
                match &self.prog.constants[*idx as usize] {
                    Constant::Int(v) => {
                        self.line(&format!("li t0, {v}"));
                        self.store_slot(*dst, "t0");
                    }
                    Constant::Bool(b) => {
                        self.line(&format!("li t0, {}", if *b { 1 } else { 0 }));
                        self.store_slot(*dst, "t0");
                    }
                    Constant::Float(f) => {
                        let label = self.ro.intern_float(f.to_bits());
                        self.line(&format!("fld ft0, {label}, t0"));
                        self.fstore_slot(*dst, "ft0");
                    }
                    Constant::Text(s) => {
                        let label = self.ro.intern_text(s);
                        self.line(&format!("la t0, {label}"));
                        self.store_slot(*dst, "t0");
                    }
                    // `Nothing` — a null sentinel (the default-fill of a `Divergent`/register field before
                    // its first `Resolve`, and a bare `nothing` literal). Rides an i64 slot as 0.
                    Constant::Nothing => {
                        self.line("li t0, 0");
                        self.store_slot(*dst, "t0");
                    }
                    // A `Duration` literal (`1 seconds`) rides its i64 nanos — used as a `Sleep`/`After`
                    // tick count (never Shown directly), so an integer slot suffices.
                    Constant::Duration(n) => {
                        self.line(&format!("li t0, {n}"));
                        self.store_slot(*dst, "t0");
                    }
                    // A date literal rides its days since the epoch, a moment its nanoseconds, a span its months and
                    // days packed into one word ([`Kind::Span`]).
                    Constant::Date(days) => {
                        self.line(&format!("li t0, {days}"));
                        self.store_slot(*dst, "t0");
                    }
                    Constant::Moment(nanos) => {
                        self.line(&format!("li t0, {nanos}"));
                        self.store_slot(*dst, "t0");
                    }
                    Constant::Span { months, days } => {
                        let packed = ((*months as i64) << 32) | (*days as u32 as i64);
                        self.line(&format!("li t0, {packed}"));
                        self.store_slot(*dst, "t0");
                    }
                    _ => {
                        return Err(RiscVLowerError::Unsupported(
                            "LoadConst of non-scalar constant",
                        ))
                    }
                }
            }
            Op::Move { dst, src } => {
                self.load_slot(*src, "t0");
                self.store_slot(*dst, "t0");
            }

            Op::Add { dst, lhs, rhs } => self.arith(*dst, *lhs, *rhs, kinds, "add", "fadd.d")?,
            Op::Sub { dst, lhs, rhs } => self.arith(*dst, *lhs, *rhs, kinds, "sub", "fsub.d")?,
            Op::Mul { dst, lhs, rhs } => self.arith(*dst, *lhs, *rhs, kinds, "mul", "fmul.d")?,
            // `t = t + v` on a Text register: `v`'s text appended to the register's own Text.
            Op::AddAssign { dst, src } if self.kind_of(kinds, *dst) == Kind::Text => {
                let tag = kind_tag(self.kind_of(kinds, *src))?;
                self.inspects(&[*src]);
                self.own_target(*dst, &[*src]);
                self.load_slot(*src, "a2");
                self.line(&format!("li a3, {tag}"));
                self.line("call logos_text_append_own");
            }
            Op::AddAssign { dst, src } => self.arith(*dst, *dst, *src, kinds, "add", "fadd.d")?,

            Op::BitXor { dst, lhs, rhs }
            | Op::BitAnd { dst, lhs, rhs }
            | Op::BitOr { dst, lhs, rhs }
            | Op::Shl { dst, lhs, rhs }
            | Op::Shr { dst, lhs, rhs }
                if self.over_rational(kinds, &[*lhs, *rhs]) =>
            {
                let symbol = match op {
                    Op::BitXor { .. } => '^',
                    Op::BitAnd { .. } => '&',
                    Op::BitOr { .. } => '|',
                    Op::Shl { .. } => '<',
                    _ => '>',
                };
                self.rat_bitwise(*dst, *lhs, *rhs, kinds, symbol)?
            }
            Op::BitXor { dst, lhs, rhs } => self.int_bitop("xor", *dst, *lhs, *rhs, kinds)?,
            Op::BitAnd { dst, lhs, rhs } => self.int_bitop("and", *dst, *lhs, *rhs, kinds)?,
            Op::BitOr { dst, lhs, rhs } => self.int_bitop("or", *dst, *lhs, *rhs, kinds)?,
            // Shifts: RV64 `sll`/`sra` mask the count to 6 bits — exactly the VM's wrapping_shl/shr
            // (`(b as u32) & 63`); `sra` is the arithmetic (sign-preserving) right shift for signed Int.
            Op::Shl { dst, lhs, rhs } => self.int_shift("sll", *dst, *lhs, *rhs, kinds)?,
            Op::Shr { dst, lhs, rhs } => self.int_shift("sra", *dst, *lhs, *rhs, kinds)?,
            Op::Pow { dst, lhs, rhs } if self.over_rational(kinds, &[*lhs, *rhs]) => {
                if self.over_real(kinds, &[*lhs, *rhs]) {
                    self.rat_real_op(*dst, *lhs, *rhs, kinds, 4)?;
                } else {
                    self.rat_binop(*dst, *lhs, *rhs, kinds, 7)?;
                }
            }
            Op::Pow { dst, lhs, rhs } => self.pow(*dst, *lhs, *rhs, kinds)?,

            // ── Reversible-computing primitives (forward direction) — no SOURCE emits these into an AOT
            // program, but their forward semantics are well-defined and proven ≡ VM via bytecode-level
            // differentials (the reversible subsystem injects them; the AOT still lowers them faithfully).
            Op::RevSwap { a, b } => {
                self.load_slot(*a, "t0");
                self.load_slot(*b, "t1");
                self.store_slot(*b, "t0");
                self.store_slot(*a, "t1");
            }
            Op::RevXor { place, delta } => {
                self.load_slot(*place, "t0");
                self.load_slot(*delta, "t1");
                self.line("xor t0, t0, t1");
                self.store_slot(*place, "t0");
            }
            Op::RevNeg { place } => {
                self.load_slot(*place, "t0");
                self.line("neg t0, t0");
                self.store_slot(*place, "t0");
            }
            Op::RevAdd { place, delta } => {
                self.load_slot(*place, "t0");
                self.load_slot(*delta, "t1");
                self.line("add t0, t0, t1");
                self.store_slot(*place, "t0");
            }
            Op::RevSub { place, delta } => {
                self.load_slot(*place, "t0");
                self.load_slot(*delta, "t1");
                self.line("sub t0, t0, t1");
                self.store_slot(*place, "t0");
            }
            // The 256-bit BigInt lane (in-i64 here, like the rest of integer arithmetic): kind selects the op.
            Op::BigIntWide { kind, dst, a, b } => match kind {
                0 => {
                    self.load_slot(*a, "t0");
                    self.store_slot(*dst, "t0");
                } // FromInt: dst = a
                1 => {
                    self.load_slot(*a, "t0");
                    self.load_slot(*b, "t1");
                    self.line("add t0, t0, t1");
                    self.store_slot(*dst, "t0");
                }
                2 => {
                    self.load_slot(*a, "t0");
                    self.load_slot(*b, "t1");
                    self.line("sub t0, t0, t1");
                    self.store_slot(*dst, "t0");
                }
                3 => {
                    self.load_slot(*a, "t0");
                    self.load_slot(*b, "t1");
                    self.line("slt t0, t0, t1");
                    self.store_slot(*dst, "t0");
                } // Cmp: a < b
                4 => {
                    self.load_slot(*a, "t0");
                    self.line("slli t0, t0, 32");
                    self.line("srli t0, t0, 32");
                    self.store_slot(*dst, "t0");
                } // Low: a mod 2^32 (zero-extend)
                5 => {
                    self.load_slot(*a, "t0");
                    self.load_slot(*b, "t1");
                    self.line("div t0, t0, t1");
                    self.store_slot(*dst, "t0");
                }
                _ => {
                    self.load_slot(*a, "t0");
                    self.load_slot(*b, "t1");
                    self.line("mul t0, t0, t1");
                    self.store_slot(*dst, "t0");
                } // Mul (kind 6)
            },
            // A native-region-JIT bounds hint + an OBSERVED-profile observation site — both no-ops in the
            // elided AOT profile the differential compares (no observable effect).
            Op::RegionBoundsGuard { .. } | Op::Obs { .. } => {
                let mut read = Vec::new();
                op.for_each_read_reg(
                    |f| self.prog.functions[f as usize].captures.len(),
                    |r| read.push(r),
                );
                self.inspects(&read);
            }

            Op::Div { dst, lhs, rhs } if self.over_rational(kinds, &[*lhs, *rhs]) => {
                if self.over_real(kinds, &[*lhs, *rhs]) {
                    self.rat_real_op(*dst, *lhs, *rhs, kinds, 3)?;
                } else {
                    self.rat_binop(*dst, *lhs, *rhs, kinds, 3)?;
                }
            }
            Op::Div { dst, lhs, rhs } => match self.num_kind(kinds, *lhs, *rhs)? {
                Kind::Float => self.float_div(*dst, *lhs, *rhs, kinds)?,
                _ => self.int_divmod("div", *dst, *lhs, *rhs, kinds)?,
            },
            Op::ExactDiv { dst, lhs, rhs } => {
                // `exact_divide`: a Float operand → float division; otherwise an exact Rational (`a/b`).
                let (lk, rk) = (self.kind_of(kinds, *lhs), self.kind_of(kinds, *rhs));
                if lk == Kind::Float || rk == Kind::Float {
                    self.float_div(*dst, *lhs, *rhs, kinds)?;
                } else {
                    self.rat_binop(*dst, *lhs, *rhs, kinds, 4)?;
                }
            }
            Op::Mod { dst, lhs, rhs }
                if self.over_rational(kinds, &[*lhs, *rhs]) && !self.over_real(kinds, &[*lhs, *rhs]) =>
            {
                self.rat_binop(*dst, *lhs, *rhs, kinds, 6)?
            }
            Op::FloorDiv { dst, lhs, rhs }
                if self.over_rational(kinds, &[*lhs, *rhs]) && !self.over_real(kinds, &[*lhs, *rhs]) =>
            {
                self.rat_binop(*dst, *lhs, *rhs, kinds, 5)?
            }
            Op::Mod { dst, lhs, rhs } => self.int_divmod("rem", *dst, *lhs, *rhs, kinds)?,
            Op::FloorDiv { dst, lhs, rhs } => self.floor_div(*dst, *lhs, *rhs, kinds)?,
            // An integer a number join holds as a rational divides as the exact number it is.
            Op::DivPow2 { .. } | Op::MagicDivU { .. }
                if crate::vm::tower::by_constant(op)
                    .is_some_and(|(_, lhs, _, _)| self.kind_of(kinds, lhs) == Kind::Rational) =>
            {
                let (dst, lhs, divisor, f) = crate::vm::tower::by_constant(op).expect("a division by a constant");
                self.load_slot(lhs, "a0");
                self.line("li a1, 1");
                self.line(&format!("li a2, {divisor}"));
                self.line("li a3, 0");
                self.line(&format!("li a4, {}", if f == "div" { 3 } else { 6 }));
                self.line("call logos_ldec_op");
                self.store_slot(dst, "a0");
            }
            Op::DivPow2 { dst, lhs, k } => self.div_pow2(*dst, *lhs, *k, kinds)?,
            Op::MagicDivU {
                dst,
                lhs,
                magic,
                more,
                mul_back,
                divisor: _,
                wrap: _,
            } => {
                self.require(kinds, &[*lhs], Kind::Int, "MagicDivU on non-Int")?;
                self.load_slot(*lhs, "a0");
                self.line(&format!("li a1, {magic:#x}"));
                self.line(&format!("li a2, {more}"));
                self.line(&format!("li a3, {mul_back}"));
                self.line("call logos_magic_div");
                self.store_slot(*dst, "a0");
            }

            // Copy-on-write barrier — a NO-OP here: a callee's parameter starts with a clear owned flag, so
            // its first mutation copies and the caller's collection never changes (the property EnsureOwned
            // exists to guarantee).
            Op::EnsureOwned { reg } => self.keeps(*reg),

            Op::Lt { dst, lhs, rhs } => self.cmp("lt", *dst, *lhs, *rhs, kinds)?,
            Op::Gt { dst, lhs, rhs } => self.cmp("gt", *dst, *lhs, *rhs, kinds)?,
            Op::LtEq { dst, lhs, rhs } => self.cmp("le", *dst, *lhs, *rhs, kinds)?,
            Op::GtEq { dst, lhs, rhs } => self.cmp("ge", *dst, *lhs, *rhs, kinds)?,
            Op::Eq { dst, lhs, rhs } => self.cmp("eq", *dst, *lhs, *rhs, kinds)?,
            Op::NotEq { dst, lhs, rhs } => self.cmp("ne", *dst, *lhs, *rhs, kinds)?,
            Op::ApproxEq { dst, lhs, rhs } => self.approx_eq(*dst, *lhs, *rhs, kinds)?,

            Op::Concat { dst, lhs, rhs } => {
                // Stringify both operands (by kind) and join — matches the VM's `arith::concat`.
                self.inspects(&[*lhs, *rhs]);
                let (lk, rk) = (self.kind_of(kinds, *lhs), self.kind_of(kinds, *rhs));
                let (lt, rt) = (kind_tag(lk)?, kind_tag(rk)?);
                self.load_slot(*lhs, "a0");
                self.line(&format!("li a1, {lt}"));
                self.load_slot(*rhs, "a2");
                self.line(&format!("li a3, {rt}"));
                self.line("call logos_concat_vals");
                self.store_slot(*dst, "a0");
            }
            Op::FormatValue {
                dst,
                src,
                spec,
                debug_prefix,
            } => {
                // Interpolation with a format spec / debug prefix. Both consts are known at codegen, so the
                // spec is parsed here and specialized (mirrors `semantics::format::apply_format_spec`).
                self.inspects(&[*src]);
                let sk = self.kind_of(kinds, *src);
                if *spec == u32::MAX {
                    // Debug prefix without a spec → the plain display string.
                    let tag = kind_tag(sk)?;
                    self.load_slot(*src, "a0");
                    self.line(&format!("li a1, {tag}"));
                    self.line("call logos_val_to_str");
                } else {
                    let spec_s = match &self.prog.constants[*spec as usize] {
                        Constant::Text(s) => s.clone(),
                        _ => {
                            return Err(RiscVLowerError::Unsupported(
                                "format spec is not a text constant",
                            ))
                        }
                    };
                    self.emit_format_spec(&spec_s, *src, sk)?; // → char* in a0
                }
                if *debug_prefix != u32::MAX {
                    // Prepend `name=` (the VM's debug form) by concatenating the two Text values.
                    let prefix = match &self.prog.constants[*debug_prefix as usize] {
                        Constant::Text(s) => format!("{s}="),
                        _ => {
                            return Err(RiscVLowerError::Unsupported(
                                "debug prefix is not a text constant",
                            ))
                        }
                    };
                    let label = self.ro.intern_text(&prefix);
                    self.line("mv a2, a0"); // b = formatted
                    self.line("li a3, 3");
                    self.line(&format!("la a0, {label}")); // a = "name="
                    self.line("li a1, 3");
                    self.line("call logos_concat_vals");
                }
                self.store_slot(*dst, "a0");
            }

            // `not x` — the VM's `!is_truthy(x)`.
            Op::Not { dst, src } => {
                self.inspects(&[*src]);
                self.truth(*src, kinds, "t0", false)?;
                self.line("seqz t1, t0");
                self.store_slot(*dst, "t1");
            }

            Op::Jump { target } => self.line(&format!("j .Lpc_{target}")),
            // A condition is the VM's truthiness of its value.
            Op::JumpIfFalse { cond, target } => {
                self.inspects(&[*cond]);
                self.truth(*cond, kinds, "t0", true)?;
                self.line(&format!("beqz t0, .Lpc_{target}"));
            }
            Op::JumpIfTrue { cond, target } => {
                self.inspects(&[*cond]);
                self.truth(*cond, kinds, "t0", true)?;
                self.line(&format!("bnez t0, .Lpc_{target}"));
            }

            Op::Call {
                dst,
                func,
                args_start,
                arg_count,
            } => {
                if *arg_count > 8 {
                    return Err(RiscVLowerError::Unsupported(
                        "call with more than 8 arguments",
                    ));
                }
                for k in 0..*arg_count {
                    self.load_slot(*args_start + k, &format!("a{k}"));
                }
                self.line(&format!("call logos_fn_{func}"));
                // Each `mutable` parameter's final value comes back in a2.. into the argument register it was lent
                // from, for the copy-out after the call.
                for (j, p) in self.inout_params(*func as usize).into_iter().enumerate() {
                    self.store_slot(*args_start + p, &format!("a{}", 2 + j));
                    if self.may_be_nothing.contains(&(*args_start + p)) {
                        self.store_flag(*args_start + p, "zero");
                    }
                }
                self.store_slot(*dst, "a0");
                self.nothing_after_call(*dst, self.answers_nothing[*func as usize]);
            }
            Op::MakeClosure {
                dst,
                func,
                locals_start,
            } => {
                // Build the closure object `[body_index, captured values…, present flags…]`; a local
                // capture snapshots the slot at the build site, a global capture the global (present=1).
                let caps = self.prog.functions[*func as usize].captures.clone();
                let cap = caps.len() as u16;
                self.line(&format!("li a0, {func}"));
                self.line(&format!("li a1, {cap}"));
                self.line("call logos_closure_new");
                self.store_slot(*dst, "a0");
                let mut local_k: u16 = 0;
                for (k, (_, global)) in caps.iter().enumerate() {
                    let k = k as u16;
                    match global {
                        Some(gidx) => {
                            self.line(&format!("la {ADDR_TMP}, logos_globals"));
                            self.line(&format!("ld a2, {}({ADDR_TMP})", 8 * (*gidx as i64)));
                        }
                        None => {
                            self.load_slot(*locals_start + local_k, "a2");
                            local_k += 1;
                        }
                    }
                    self.load_slot(*dst, "a0");
                    self.line(&format!("li a1, {}", 1 + k));
                    self.line("call logos_closure_set");
                    // present flag = 1 (a local is always present; a captured global is assumed defined).
                    self.load_slot(*dst, "a0");
                    self.line(&format!("li a1, {}", 1 + cap + k));
                    self.line("li a2, 1");
                    self.line("call logos_closure_set");
                }
            }
            Op::CallValue {
                dst,
                callee,
                args_start,
                arg_count,
                ..
            } => {
                // Indirect call through a closure: args in a0.., the closure pointer as the hidden arg
                // a{arg_count}, then jump via the dispatch table indexed by the closure's body index.
                self.need_dispatch = true;
                if *arg_count > 7 {
                    return Err(RiscVLowerError::Unsupported(
                        "indirect call with more than 7 arguments",
                    ));
                }
                for k in 0..*arg_count {
                    self.load_slot(*args_start + k, &format!("a{k}"));
                }
                self.load_slot(*callee, "t0"); // closure pointer
                self.line(&format!("mv a{arg_count}, t0"));
                self.line("ld t1, 0(t0)"); // body index
                self.line("la t2, logos_fn_dispatch");
                self.line("slli t1, t1, 3");
                self.line("add t2, t2, t1");
                self.line("ld t2, 0(t2)");
                self.line("jalr t2");
                self.store_slot(*dst, "a0");
                self.nothing_after_call(*dst, self.a_closure_answers_nothing(*arg_count));
            }
            Op::Return { src } => {
                self.load_slot(*src, "a0");
                if self.reports_nothing {
                    self.nothing_flag(self.pc, *src, "a1");
                }
                self.emit_epilogue(frame);
            }
            Op::LoadToday { dst } => {
                self.line("call logos_today");
                self.store_slot(*dst, "a0");
            }
            Op::LoadNow { dst } => {
                self.line("call logos_now");
                self.store_slot(*dst, "a0");
            }
            Op::ReturnNothing => self.emit_return_nothing(frame),
            Op::Halt => {
                self.line("li a0, 0");
                self.emit_epilogue(frame);
            }

            // ── Seq (Int elements) ──
            // `NewEmptyListI32` is the half-width (LOGOS_NARROW_VM) empty `Seq of Int`; its i32 storage is
            // an internal optimization — the observable is a Seq of Int, so it lowers like `NewEmptyList`.
            Op::NewEmptyList { dst } | Op::NewEmptyListI32 { dst } => {
                self.line("call logos_seq_new");
                self.store_slot(*dst, "a0");
                self.fresh(&[*dst]);
            }
            Op::NewEmptyListFixed { dst, capacity } => {
                self.line(&format!("li a0, {capacity}"));
                self.line("call logos_seq_new_cap");
                self.store_slot(*dst, "a0");
                self.fresh(&[*dst]);
            }
            Op::NewEmptyListWithCapacity { dst, capacity } => {
                self.load_slot(*capacity, "a0");
                self.line("call logos_seq_new_cap");
                self.store_slot(*dst, "a0");
                self.fresh(&[*dst]);
            }
            Op::ListPush { list, value } => {
                let among_rationals =
                    self.rational_list(self.kind_of(kinds, *list)) && self.kind_of(kinds, *value) == Kind::Int;
                self.own_target(*list, &[*value]);
                self.load_slot(*value, "a2");
                self.line(if among_rationals {
                    "call logos_seq_push_own_ldec"
                } else {
                    "call logos_seq_push_own"
                });
            }
            Op::Length { dst, collection } => {
                self.inspects(&[*collection]);
                let ck = self.kind_of(kinds, *collection);
                self.load_slot(*collection, "a0");
                if ck == Kind::Text {
                    self.line("call logos_text_len");
                } else if self.is_map(ck) {
                    self.line("call logos_map_len");
                } else {
                    self.line("call logos_seq_len");
                }
                self.store_slot(*dst, "a0");
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
                self.inspects(&[*collection, *index]);
                let ck = self.kind_of(kinds, *collection);
                if self.is_seq_like(ck)
                    || ck == Kind::SeqUnknown
                    || self.failing_reads.contains(&self.pc)
                {
                    // 1-based sequence read (uniform 8-byte cells; the element kind rides on `dst`).
                    // IndexUnchecked keeps the bounds check (a safe superset — the VM checks it too), and a
                    // sequence with no item fails it whatever it would have held. A sequence whose element kind
                    // no path has settled (`[]` on one path) reads its cells the same way, as iterating it does.
                    self.int_operand(kinds, *index, crate::codegen_c::words::IntegerPlace::ListRead, "a1")?;
                    self.load_slot(*collection, "a0");
                    self.line("call logos_seq_index");
                    self.store_slot(*dst, "a0");
                } else if self.is_map(ck) {
                    // Map GET by key (key compared by its kind); a missing key errors like the VM.
                    let key_tag = map_key_tag(self.kind_of(kinds, *index))?;
                    self.load_slot(*collection, "a0");
                    self.load_slot(*index, "a1");
                    self.line(&format!("li a2, {key_tag}"));
                    self.line("call logos_map_get");
                    self.store_slot(*dst, "a0");
                } else if ck == Kind::Text {
                    // `item i of text` — 1-based character access → a single-character Text.
                    self.int_operand(kinds, *index, crate::codegen_c::words::IntegerPlace::Text, "a1")?;
                    self.load_slot(*collection, "a0");
                    self.line("call logos_text_index");
                    self.store_slot(*dst, "a0");
                } else if matches!(self.as_composite(ck), Some(Composite::Tuple(_))) {
                    // `item i of t` — a tuple's cells, 1-based or from the back.
                    self.load_slot(*collection, "a0");
                    self.load_slot(*index, "a1");
                    self.line("call logos_seq_index");
                    self.store_slot(*dst, "a0");
                } else if let Some(Composite::Struct(record)) = self.as_composite(ck) {
                    // `item "f" of r` — the record's field `f`.
                    let cell = self.record_field_named(&record, *index)?;
                    self.load_slot(*collection, "a0");
                    self.line(&format!("li a1, {}", cell + 1));
                    self.line("call logos_seq_index");
                    self.store_slot(*dst, "a0");
                } else {
                    return Err(RiscVLowerError::Unsupported(
                        "Index on a non-sequence / non-map",
                    ));
                }
            }
            // `Repeat for x in v` — the VM's `iteration_snapshot`: a sequence or set by its items, a Text by
            // its characters, a map by its `(key, value)` pairs.
            Op::IterPrepare { iterable } => {
                let k = self.kind_of(kinds, *iterable);
                self.load_slot(*iterable, "a0");
                if k == Kind::Text {
                    self.line("call logos_text_chars");
                } else if self.is_map(k) {
                    self.line("call logos_map_pairs");
                } else if !(self.is_iterable(k) || k == Kind::SeqUnknown || k == Kind::SetUnknown) {
                    return Err(RiscVLowerError::Unsupported(
                        "iteration over a non-collection",
                    ));
                }
                self.line("call logos_iter_begin");
            }
            Op::IterNext { dst, exit } => {
                self.line("call logos_iter_has");
                self.line(&format!("beqz a0, .Lpc_{exit}"));
                self.line("call logos_iter_take");
                self.store_slot(*dst, "a0");
            }
            Op::IterPop => self.line("call logos_iter_end"),

            Op::DestructureTuple { src, start, count } => {
                // `for (a, b) in …` — split a tuple (a `logos_seq` of cells) into `count` consecutive
                // slots. Each destructured slot inherits its position kind via `infer_kinds`.
                self.inspects(&[*src]);
                for k in 0..*count {
                    self.load_slot(*src, "a0");
                    self.line(&format!("li a1, {}", k + 1));
                    self.line("call logos_seq_index");
                    self.store_slot(*start + k, "a0");
                }
            }
            Op::ListPop { list, dst } => {
                // `Pop from xs into v` — value semantics (the VM `ensure_reg_owned`s then pops): the list's
                // own copy loses its last element, so aliases keep the full sequence. An empty list pops
                // `nothing` (the helper answers present/value in a0/a1).
                self.own_target(*list, &[]);
                if let Some(reason) = self.fails_fast.get(&self.pc).cloned() {
                    // The pop answers the value itself; from an empty list it stops the program with the
                    // sentence this call site registers (`logos_fail_sites`), so the value path checks nothing.
                    self.line("call logos_seq_pop_value");
                    let site = format!(".Lsite_{}", self.fail_sites.len());
                    self.out.push_str(&format!("{site}:\n"));
                    let reason = self.ro.intern_text(&reason);
                    self.fail_sites.push((site, reason));
                    self.store_slot(*dst, "a0");
                    if self.may_be_nothing.contains(dst) {
                        self.store_flag(*dst, "zero");
                    }
                } else {
                    self.line("call logos_seq_pop_own");
                    self.store_slot(*dst, "a1");
                    if self.may_be_nothing.contains(dst) {
                        self.line("seqz t0, a0");
                        self.store_flag(*dst, "t0");
                    }
                }
            }

            Op::NewList { dst, start, count } => {
                let items: Vec<Kind> = (0..*count).map(|i| self.kind_of(kinds, start + i)).collect();
                if self.rational_list(self.list_kind(&items)) {
                    // Integers beside rationals go in as the rationals they equal.
                    self.line("call logos_seq_new");
                    self.store_slot(*dst, "a0");
                    for (k, item) in items.iter().enumerate() {
                        self.load_slot(*dst, "a0");
                        self.load_slot(start + k as u16, "a1");
                        self.line(if *item == Kind::Int {
                            "call logos_seq_push_inplace_ldec"
                        } else {
                            "call logos_seq_push_inplace"
                        });
                    }
                    self.fresh(&[*dst]);
                } else {
                    self.new_seq_of(*dst, None, *start, *count);
                }
            }
            Op::NewRange { dst, start, end } if self.over_rational(kinds, &[*start, *end]) => {
                // A bound that is a rational is the integer a whole one equals, refused in the VM's words else.
                let (past_word, fraction) = self.integer_place(crate::codegen_c::words::IntegerPlace::Range);
                self.load_slot(*start, "a0");
                self.line(&format!("li a1, {}", (self.kind_of(kinds, *start) == Kind::Rational) as u8));
                self.load_slot(*end, "a2");
                self.line(&format!("li a3, {}", (self.kind_of(kinds, *end) == Kind::Rational) as u8));
                self.line(&format!("la a4, {past_word}"));
                self.line(&format!("la a5, {fraction}"));
                self.line("call logos_seq_range_ldec");
                self.store_slot(*dst, "a0");
                self.fresh(&[*dst]);
            }
            Op::NewRange { dst, start, end } => {
                self.load_slot(*start, "a0");
                self.load_slot(*end, "a1");
                self.line("call logos_seq_range");
                self.store_slot(*dst, "a0");
                self.fresh(&[*dst]);
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
                let ck = self.kind_of(kinds, *collection);
                if self.is_seq_like(ck) {
                    // 1-based sequence write into the register's own list; an integer stored among rationals goes in
                    // as the rational it equals, and a whole rational index is the integer it equals.
                    let among_rationals =
                        self.rational_list(ck) && self.kind_of(kinds, *value) == Kind::Int;
                    self.int_operand(kinds, *index, crate::codegen_c::words::IntegerPlace::ListStore, "a2")?;
                    self.own_target(*collection, &[*index, *value]);
                    self.load_slot(*value, "a3");
                    self.line(if among_rationals {
                        "call logos_seq_set_own_ldec"
                    } else {
                        "call logos_seq_set_own"
                    });
                } else if self.is_map(ck) || ck == Kind::MapUnknown {
                    // Map INSERT/UPDATE by key into the register's own map. The first insert sees a
                    // still-`MapUnknown` handle.
                    let key_tag = map_key_tag(self.kind_of(kinds, *index))?;
                    self.own_target(*collection, &[*index, *value]);
                    self.load_slot(*index, "a2");
                    self.load_slot(*value, "a3");
                    self.line(&format!("li a4, {key_tag}"));
                    self.line("call logos_map_set_own");
                } else if let Some(Composite::Struct(record)) = self.as_composite(ck) {
                    // `item "f" of r` written — the record's field `f`, named by a Text the code before it
                    // loads (the VM's take-out/put-back around a mutation of a field's collection).
                    let cell = self.record_field_named(&record, *index)?;
                    self.own_target(*collection, &[*value]);
                    self.line(&format!("li a2, {}", cell + 1));
                    self.load_slot(*value, "a3");
                    self.line("call logos_seq_set_own");
                } else {
                    return Err(RiscVLowerError::Unsupported(
                        "indexed write on a non-sequence / non-map",
                    ));
                }
            }
            Op::NewEmptyMap { dst } => {
                self.line("call logos_map_new");
                self.store_slot(*dst, "a0");
                self.fresh(&[*dst]);
            }
            Op::NewEmptyMapWithCapacity { dst, capacity } => {
                self.load_slot(*capacity, "a0");
                self.line("call logos_map_new_cap");
                self.store_slot(*dst, "a0");
                self.fresh(&[*dst]);
            }
            Op::NewTuple { dst, start, count } => {
                // A tuple is a fixed array of uniform 8-byte cells (per-position kinds tracked statically).
                self.new_seq_of(*dst, None, *start, *count);
            }

            // ── Structs (fixed field-cell array in declaration order; field name → index via struct_types) ──
            Op::NewStruct { dst, type_name } => {
                let name =
                    self.struct_type_name(*type_name)
                        .ok_or(RiscVLowerError::Unsupported(
                            "struct type name is not a text constant",
                        ))?;
                let n = self
                    .struct_field_count(&name)
                    .ok_or(RiscVLowerError::Unsupported("unknown struct type"))?;
                self.line(&format!("li a0, {n}"));
                self.line("call logos_struct_new");
                self.store_slot(*dst, "a0");
                self.fresh(&[*dst]);
            }
            Op::StructInsert { obj, field, value } => {
                let (idx, _) = self
                    .resolve_field(self.kind_of(kinds, *obj), *field)
                    .ok_or(RiscVLowerError::Unsupported(
                        "field set on a non-struct / unknown field",
                    ))?;
                self.own_target(*obj, &[*value]);
                self.line(&format!("li a2, {}", idx + 1)); // 1-based cell
                self.load_slot(*value, "a3");
                self.line("call logos_seq_set_own");
            }
            Op::GetField { dst, obj, field } => {
                self.inspects(&[*obj]);
                let (idx, _) = self.field_of(self.kind_of(kinds, *obj), *field).ok_or(
                    RiscVLowerError::Unsupported("field get on a non-struct / unknown field"),
                )?;
                // A counter-CRDT field holds a per-replica map; reading it yields the SUM (an Int), just
                // as the VM's `Show c's points` renders the counter's total.
                let is_counter = matches!(self.as_composite(self.kind_of(kinds, *obj)), Some(Composite::Struct(n)) if self.crdt_field(&n, idx).is_some_and(is_counter_crdt));
                self.load_slot(*obj, "a0");
                self.line(&format!("li a1, {}", idx + 1));
                self.line("call logos_seq_index");
                if is_counter {
                    self.line("call logos_crdt_sum");
                }
                self.store_slot(*dst, "a0");
            }

            Op::Args { dst } => {
                // `args()` → a `Seq of Text` of the process argv (item 1 is the program name, item 2 the
                // first real argument), exactly like the VM's `program_args`.
                self.line("call logos_args");
                self.store_slot(*dst, "a0");
                self.fresh(&[*dst]);
            }
            // `Push v to s's items` — the register's own record's own list takes `v` ([`Mutation::Field`]): in
            // place while the register owns the record and its field flag says the record alone holds the list,
            // after the copies the flags call for otherwise.
            Op::ListPushField { obj, field, src } => {
                let (cell, fk) = self
                    .resolve_field(self.kind_of(kinds, *obj), *field)
                    .ok_or(RiscVLowerError::Unsupported("push to unknown struct field"))?;
                if !self.is_seq_like(fk) {
                    return Err(RiscVLowerError::Unsupported("push to a non-sequence field"));
                }
                self.field_target(*obj, cell, &[*src]);
                self.load_slot(*src, "a4");
                self.line("call logos_field_push_own");
            }
            Op::NewCrdt { dst, kind } => {
                // A counter CRDT (ConvergentCount/GCounter=4, Tally/PNCounter=5): a per-replica count map
                // (idempotent lattice). A sequence CRDT (SharedSequence/RGA=1): on the single replica the
                // converged log IS a plain `Seq` behind a stable handle — the field lays out as its
                // underlying `Seq of T` (see the VM compiler's `boundary_of_field_type`), so seed the cell
                // with an empty seq (`logos_seq_new`) and let `CrdtAppend` push in place, exactly like a
                // plain shared-`Seq` field. Set/register CRDTs (0/2/3) need their own lattices → deferred.
                match kind {
                    4 | 5 => self.line("call logos_crdt_new"),
                    1 => self.line("call logos_seq_new"),
                    _ => {
                        return Err(RiscVLowerError::Unsupported(
                            "non-counter/sequence CRDT (set/register lattice)",
                        ))
                    }
                }
                self.store_slot(*dst, "a0");
            }
            // `Increase/Decrease s's counter by n` — the register's own record's own counter changes
            // ([`Mutation::Field`]), as the struct's field does on the VM.
            Op::CrdtBump {
                obj,
                field,
                amount,
                negate,
            } => {
                let (cell, _) = self
                    .resolve_field(self.kind_of(kinds, *obj), *field)
                    .ok_or(RiscVLowerError::Unsupported("bump of unknown struct field"))?;
                // A ConvergentCount only grows: a negative change stops the program with the VM's sentence.
                if let Some(Composite::Struct(record)) =
                    self.as_composite(self.kind_of(kinds, *obj))
                {
                    if self.crdt_field(&record, cell)
                        == Some(crate::semantics::crdt::CrdtKind::Counter.new_crdt_code())
                    {
                        let reason = crate::semantics::crdt::CrdtValue::new_counter(0)
                            .bump(-1)
                            .expect_err("a ConvergentCount refuses a decrease");
                        let fail = self.fail_stub(&reason);
                        self.load_slot(*amount, "t0");
                        if *negate {
                            self.line("neg t0, t0");
                        }
                        self.jump_to_stop(Some(("bltz", "t0")), &fail);
                    }
                }
                self.field_target(*obj, cell, &[*amount]);
                self.load_slot(*amount, "a4");
                self.line(&format!("li a5, {}", if *negate { 1 } else { 0 }));
                self.line("call logos_field_bump_own");
            }
            // `Merge b into a` / `Converge b into a` — the target's own record takes, field by field, what the
            // VM's `crdt_merge_field` (`crdt_join_field` for a join) makes of its value and the source's: a
            // fresh merged counter, a union of sets, a key-wise merge of maps, a sum (a join: the larger) of
            // Ints, and the source's value for any other field.
            Op::CrdtMerge { target, source } | Op::CrdtJoin { target, source } => {
                let is_join = matches!(op, Op::CrdtJoin { .. });
                self.inspects(&[*source]);
                let tname = match (
                    self.as_composite(self.kind_of(kinds, *target)),
                    self.as_composite(self.kind_of(kinds, *source)),
                ) {
                    (Some(Composite::Struct(t)), Some(Composite::Struct(s))) if t == s => t,
                    _ => {
                        return Err(RiscVLowerError::Unsupported(
                            "a merge of values that are not records of one type",
                        ))
                    }
                };
                let fields: Vec<(usize, Kind)> = self
                    .prog
                    .struct_types
                    .iter()
                    .find(|sd| sd.name == tname)
                    .ok_or(RiscVLowerError::Unsupported(
                        "a merge of an unknown record type",
                    ))?
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(cell, (_, t))| (cell, self.boundary_kind(&Some(t.clone()))))
                    .collect();
                // The target's own record, copied first only when its register may not be its only holder
                // (a cold call), then changed cell by cell in place: a plain field inline, a CRDT or collection
                // field through the runtime. A cell is read before it is written, so a record merged into
                // itself merges each field with its own value.
                self.keeps(*target);
                let n = self.fresh_nothing_label();
                self.load_at_sp(self.owned_base + 8 * *target as i64, "t0");
                self.line(&format!("bnez t0, .Lmrg_{n}"));
                self.slot_addr(*target, "a0");
                self.owned_addr(*target, "a1");
                self.line("call logos_seq_make_own");
                self.out.push_str(&format!(".Lmrg_{n}:\n"));
                let cells = |e: &mut Self| {
                    e.load_slot(*target, "t1");
                    e.line("ld t1, 16(t1)");
                    e.load_slot(*source, "t2");
                    e.line("ld t2, 16(t2)");
                };
                cells(self);
                for (cell, fk) in fields {
                    let off = 8 * cell as i64;
                    match self.merge_rule(&tname, cell, fk, is_join)? {
                        (0, _, _) => {
                            self.cell_access("ld", "t3", "t2", off);
                            self.cell_access("sd", "t3", "t1", off);
                        }
                        (1, _, _) => {
                            self.cell_access("ld", "t3", "t1", off);
                            self.cell_access("ld", "t4", "t2", off);
                            self.line("add t3, t3, t4");
                            self.cell_access("sd", "t3", "t1", off);
                        }
                        (2, _, _) => {
                            self.cell_access("ld", "t3", "t1", off);
                            self.cell_access("ld", "t4", "t2", off);
                            self.line("bge t3, t4, 1f");
                            self.line("mv t3, t4");
                            self.line("1:");
                            self.cell_access("sd", "t3", "t1", off);
                        }
                        (how, a, b) => {
                            self.load_slot(*target, "a0");
                            self.load_slot(*source, "a1");
                            self.line(&format!("li a2, {}", cell + 1));
                            self.line(&format!("li a3, {how}"));
                            self.line(&format!("li a4, {a}"));
                            self.line(&format!("li a5, {b}"));
                            self.line("call logos_merge_cell");
                            cells(self);
                            self.cell_access("sd", "a0", "t1", off);
                        }
                    }
                }
            }
            // `Resolve s's reg to v` — a divergent register resolved on the single replica: the last value written
            // wins. The field lays out as its value, written into the register's own record, as a field write.
            Op::CrdtResolve { obj, field, value } => {
                let (cell, _) = self
                    .resolve_field(self.kind_of(kinds, *obj), *field)
                    .ok_or(RiscVLowerError::Unsupported(
                        "resolve of unknown struct field",
                    ))?;
                self.own_target(*obj, &[*value]);
                self.line(&format!("li a2, {}", cell + 1));
                self.load_slot(*value, "a3");
                self.line("call logos_seq_set_own");
            }
            Op::CrdtAppend { seq, value } => {
                // `Append v to <seq field>` — the sequence is shared (behind a stable seq handle in the
                // struct field), so append IN PLACE; the mutation propagates to every alias (no write-back).
                let sk = self.kind_of(kinds, *seq);
                if !self.is_seq_like(sk) && sk != Kind::SeqUnknown {
                    return Err(RiscVLowerError::Unsupported("append to a non-sequence"));
                }
                self.keeps(*seq);
                self.load_slot(*seq, "a0");
                self.load_slot(*value, "a1");
                self.line("call logos_seq_push_inplace");
            }
            // ── Concurrency (the cooperative-scheduler runtime) ──
            Op::ChanNew { dst, cap, .. } => {
                self.line(&format!("li a0, {cap}"));
                self.line("call logos_chan_new");
                self.store_slot(*dst, "a0");
            }
            Op::ChanSend { chan, val } => {
                self.load_slot(*chan, "a0");
                self.load_slot(*val, "a1");
                self.line("call logos_chan_send");
            }
            // `Receive` answers value-or-nothing in a0 (present) / a1 (value).
            Op::ChanRecv { dst, chan } => {
                self.load_slot(*chan, "a0");
                self.line("call logos_chan_recv");
                self.store_slot(*dst, "a1");
                if self.may_be_nothing.contains(dst) {
                    self.line("seqz t0, a0");
                    self.store_flag(*dst, "t0");
                }
            }
            Op::ChanClose { chan } => {
                self.load_slot(*chan, "a0");
                self.line("call logos_chan_close");
            }
            Op::ChanTrySend { dst, chan, val } => {
                self.load_slot(*chan, "a0");
                self.load_slot(*val, "a1");
                self.line("call logos_chan_trysend");
                self.store_slot(*dst, "a0"); // success bool
            }
            // `Try to receive x from ch` answers value-or-nothing in a0 (present) / a1 (value).
            Op::ChanTryRecv { dst, chan } => {
                self.load_slot(*chan, "a0");
                self.line("call logos_chan_tryrecv");
                self.store_slot(*dst, "a1");
                self.line("seqz t0, a0");
                self.store_flag(*dst, "t0");
            }
            Op::Spawn {
                func,
                args_start,
                arg_count,
            } => {
                self.emit_spawn(*func, *args_start, *arg_count)?;
            }
            Op::SpawnHandle {
                dst,
                func,
                args_start,
                arg_count,
            } => {
                self.emit_spawn(*func, *args_start, *arg_count)?;
                self.store_slot(*dst, "a0"); // task handle
            }
            // `Await h`: resumes once `h` has finished; a task answers `nothing`.
            Op::TaskAwait { dst, handle } => {
                self.load_slot(*handle, "a0");
                self.line("call logos_task_await");
                self.store_slot(*dst, "zero");
                self.line("li t0, 1");
                self.store_flag(*dst, "t0");
            }
            Op::TaskAbort { handle } => {
                self.load_slot(*handle, "a0");
                self.line("call logos_task_abort");
            }
            // ── Networking — the VM's offline inbox: one node, no relay (the runtime's `logos_net_*`) ──
            // `Connect` dials nothing offline, and a `Sync` of a variable has no peer to merge with, so its
            // register keeps the value it holds.
            Op::NetConnect { .. } | Op::NetSync { .. } => {}
            // `Listen on <topic>` — the topic the value's text names becomes this node's inbox.
            Op::NetListen { topic } => {
                self.inspects(&[*topic]);
                self.net_address(*topic, kinds)?;
                self.line("call logos_net_listen");
            }
            // `a PeerAgent at <addr>` — the topic the address's text names.
            Op::NetMakePeer { dst, addr } => {
                self.inspects(&[*addr]);
                self.net_address(*addr, kinds)?;
                self.store_slot(*dst, "a0");
            }
            Op::NetSend { to, msg } => {
                self.inspects(&[*to]);
                self.net_peer(*to, kinds)?;
                self.load_slot(*msg, "a1");
                self.line("li a2, 0");
                self.line("call logos_net_send");
            }
            // `Stream <values> to <peer>` — the batch travels as one list: the sequence itself, or a list of
            // the one value when it is not a sequence.
            Op::NetStream { to, values } => {
                self.inspects(&[*to]);
                let vk = self.kind_of(kinds, *values);
                let listed = self.is_seq_like(vk) || vk == Kind::SeqUnknown;
                self.net_peer(*to, kinds)?;
                self.load_slot(*values, "a1");
                self.line(&format!("li a2, {}", if listed { 1 } else { 2 }));
                self.line("call logos_net_send");
            }
            // `Await [stream] from <peer> into <dst>` — the first message (or batch) that peer sent this
            // node's inbox; with no inbox, the program stops with the reason every tier gives.
            Op::NetAwait { dst, from, stream } => {
                self.inspects(&[*from]);
                self.net_peer(*from, kinds)?;
                self.line(&format!("li a1, {}", *stream as i64));
                self.line("call logos_net_await");
                self.store_slot(*dst, "a0");
            }
            Op::Sleep { duration } => {
                self.load_slot(*duration, "a0");
                self.line("call logos_sleep");
            }
            Op::SelectArmRecv { chan, var } => {
                // Register a `Receive var from chan` arm: the var's slot holds a fresh box the winning
                // delivery writes what it read — a value, or `nothing` from a closed pipe — into.
                self.line("call logos_opt_nothing");
                self.store_slot(*var, "a0");
                self.line("mv a1, a0");
                self.load_slot(*chan, "a0");
                self.line("call logos_select_arm_recv");
                self.select_boxes.push(*var);
            }
            Op::SelectArmTimeout { ticks } => {
                self.load_slot(*ticks, "a0");
                self.line("call logos_select_arm_timeout");
            }
            Op::SelectWait { dst_arm } => {
                self.line("call logos_select_wait");
                self.store_slot(*dst_arm, "a0"); // the winning arm index
                                                 // Each arm variable reads its box back (a losing arm's is never read).
                for var in std::mem::take(&mut self.select_boxes) {
                    self.load_slot(var, "t0");
                    self.line("ld t1, 8(t0)");
                    self.store_slot(var, "t1");
                    if self.may_be_nothing.contains(&var) {
                        self.line("ld t1, 0(t0)");
                        self.line("seqz t1, t1");
                        self.store_flag(var, "t1");
                    }
                }
            }
            // `Check that <subject> is <predicate>` / a capability — the policy condition (from the registry)
            // evaluated inline against the subject struct's fields. A check that does not pass stops the
            // program with the VM's own sentence for it (`check_refusal`).
            Op::CheckPolicy {
                subject,
                predicate,
                is_capability,
                object,
                source_text,
            } => {
                self.inspects(&[*subject, *object]);
                let source = self
                    .text_const(*source_text)
                    .ok_or(RiscVLowerError::Unsupported(
                        "a Check whose source is not a text constant",
                    ))?
                    .to_string();
                let subject_kind = self.kind_of(kinds, *subject);
                let obj_type = if *object != u16::MAX {
                    match self.as_composite(self.kind_of(kinds, *object)) {
                        Some(Composite::Struct(n)) => Some(n),
                        _ => None,
                    }
                } else {
                    None
                };
                let reason = self.check_refusal(
                    subject_kind,
                    obj_type.as_deref(),
                    *predicate,
                    *is_capability,
                    &source,
                )?;
                let reason = self.ro.intern_text(&reason);
                let subj_type = match self.as_composite(subject_kind) {
                    Some(Composite::Struct(name)) => Some(name),
                    _ => None,
                };
                let cond = subj_type
                    .as_deref()
                    .and_then(|t| self.interner.lookup(t))
                    .and_then(|type_sym| {
                        if *is_capability {
                            self.policies.get_capabilities(type_sym).and_then(|caps| {
                                caps.iter()
                                    .find(|c| c.action == *predicate)
                                    .map(|c| c.condition.clone())
                            })
                        } else {
                            self.policies.get_predicates(type_sym).and_then(|preds| {
                                preds
                                    .iter()
                                    .find(|p| p.predicate_name == *predicate)
                                    .map(|p| p.condition.clone())
                            })
                        }
                    });
                match (cond, subj_type) {
                    (Some(c), Some(subj_type)) => {
                        let n = self.poly_label;
                        self.poly_label += 1;
                        self.emit_policy_condition(
                            &c,
                            *subject,
                            &subj_type,
                            *object,
                            obj_type.as_deref(),
                        )?;
                        self.line(&format!("bnez a0, .Lpol_{n}_pass"));
                        self.line(&format!("la a0, {reason}"));
                        self.line("call logos_fatal");
                        self.out.push_str(&format!(".Lpol_{n}_pass:\n"));
                    }
                    _ => {
                        self.line(&format!("la a0, {reason}"));
                        self.line("call logos_fatal");
                    }
                }
            }

            // ── Enums (inductive): value = [constructor tag, payload…] (`ctor_tag`) ──
            Op::NewInductive {
                ctor,
                dst,
                args_start,
                count,
                ..
            } => {
                let tag = self
                    .text_const(*ctor)
                    .and_then(|c| self.ctor_tag(c))
                    .ok_or(RiscVLowerError::Unsupported(
                        "constructor name is not a text constant",
                    ))?;
                self.new_seq_of(*dst, Some(tag as i64), *args_start, *count);
            }
            // `When V …:` — the VM's `TestArm`: an enum value matches the arm naming its constructor, a
            // struct the arm naming its type, `nothing` the `None` arm, and any other value no arm.
            Op::TestArm {
                dst,
                target,
                variant,
            } => {
                let vname = self
                    .text_const(*variant)
                    .ok_or(RiscVLowerError::Unsupported(
                        "variant name is not a text constant",
                    ))?
                    .to_string();
                let tk = self.kind_of(kinds, *target);
                self.inspects(&[*target]);
                if self.is_enum_like(tk) {
                    let tag = self
                        .ctor_tag(&vname)
                        .expect("a variant constant names itself");
                    self.load_slot(*target, "a0");
                    self.line("li a1, 1");
                    self.line("call logos_seq_index");
                    self.line(&format!("li t0, {tag}"));
                    self.line("sub t2, a0, t0");
                    self.line("seqz t2, t2");
                    self.store_slot(*dst, "t2");
                } else if let Some(Composite::Struct(name)) = self.as_composite(tk) {
                    self.line(&format!("li t2, {}", (name == vname) as i64));
                    self.store_slot(*dst, "t2");
                } else if tk == Kind::Unknown {
                    return Err(RiscVLowerError::Unsupported(
                        "Inspect arm on a value of unknown kind",
                    ));
                } else {
                    self.line("li t2, 0");
                    self.store_slot(*dst, "t2");
                }
            }
            // The matched arm's bindings: an enum value's payload `index` (cell 0 is the tag); a struct's
            // field by name, or the whole struct for a `Some` arm; any other value is its own `Some` payload.
            Op::BindArm {
                dst,
                target,
                field,
                index,
            } => {
                let tk = self.kind_of(kinds, *target);
                let cell = if self.is_enum_like(tk) {
                    Some(*index as usize + 2)
                } else if let Some(Composite::Struct(_)) = self.as_composite(tk) {
                    if self.text_const(*field) == Some(crate::vm::SOME_BINDING_MARKER) {
                        None
                    } else {
                        Some(
                            self.resolve_field(tk, *field)
                                .ok_or(RiscVLowerError::Unsupported(
                                    "bind of unknown struct field",
                                ))?
                                .0
                                + 1,
                        )
                    }
                } else if tk == Kind::Unknown || *index != 0 {
                    return Err(RiscVLowerError::Unsupported(
                        "bind on a value of unknown kind",
                    ));
                } else {
                    None
                };
                match cell {
                    Some(cell) => {
                        self.inspects(&[*target]);
                        self.load_slot(*target, "a0");
                        self.line(&format!("li a1, {cell}"));
                        self.line("call logos_seq_index");
                        self.store_slot(*dst, "a0");
                    }
                    None => {
                        self.load_slot(*target, "t0");
                        self.store_slot(*dst, "t0");
                    }
                }
            }
            Op::SeqConcat { dst, lhs, rhs } => {
                if !(self.is_seq_like(self.kind_of(kinds, *lhs))
                    || self.kind_of(kinds, *lhs) == Kind::SeqUnknown)
                {
                    return Err(RiscVLowerError::Unsupported("SeqConcat on a non-sequence"));
                }
                self.load_slot(*lhs, "a0");
                self.load_slot(*rhs, "a1");
                self.line("call logos_seq_concat");
                self.store_slot(*dst, "a0");
                self.inspects(&[*lhs, *rhs]);
                self.fresh(&[*dst]);
            }
            Op::SliceOp {
                dst,
                collection,
                start,
                end,
            } => {
                if !self.is_seq_like(self.kind_of(kinds, *collection)) {
                    return Err(RiscVLowerError::Unsupported("slice of a non-sequence"));
                }
                if self.over_rational(kinds, &[*start, *end]) {
                    // A bound that is a rational is the integer a whole one equals, refused in the VM's words else.
                    let (past_word, fraction) = self.integer_place(crate::codegen_c::words::IntegerPlace::Slice);
                    self.load_slot(*collection, "a0");
                    self.load_slot(*start, "a1");
                    self.line(&format!("li a2, {}", (self.kind_of(kinds, *start) == Kind::Rational) as u8));
                    self.load_slot(*end, "a3");
                    self.line(&format!("li a4, {}", (self.kind_of(kinds, *end) == Kind::Rational) as u8));
                    self.line(&format!("la a5, {past_word}"));
                    self.line(&format!("la a6, {fraction}"));
                    self.line("call logos_seq_slice_ldec");
                } else {
                    self.load_slot(*collection, "a0");
                    self.load_slot(*start, "a1");
                    self.load_slot(*end, "a2");
                    self.line("call logos_seq_slice");
                }
                self.store_slot(*dst, "a0");
                self.inspects(&[*collection]);
                self.fresh(&[*dst]);
            }
            Op::Contains {
                dst,
                collection,
                value,
            } => {
                self.inspects(&[*collection, *value]);
                let coll = self.kind_of(kinds, *collection);
                if let Some(Composite::Map(key, _)) = self.as_composite(coll) {
                    // `map contains key` — key membership (the VM's `Contains` on a Map).
                    let tag = map_key_tag(key)?;
                    self.load_slot(*collection, "a0");
                    self.load_slot(*value, "a1");
                    self.line(&format!("li a2, {tag}"));
                    self.line("call logos_map_has");
                    self.store_slot(*dst, "a0");
                } else if (self.rational_list(coll) || coll == Kind::SeqInt)
                    && (self.rational_list(coll) || self.kind_of(kinds, *value) == Kind::Rational)
                    && matches!(self.kind_of(kinds, *value), Kind::Int | Kind::Rational)
                {
                    // A rational is found by its value, an integer among rationals by the rational it equals.
                    self.load_slot(*collection, "a0");
                    self.line(&format!("li a1, {}", self.rational_list(coll) as u8));
                    self.load_slot(*value, "a2");
                    self.line(&format!("li a3, {}", (self.kind_of(kinds, *value) == Kind::Rational) as u8));
                    self.line("call logos_seq_contains_exact");
                    self.store_slot(*dst, "a0");
                } else if is_collection(coll) {
                    let tag = kind_tag(elem_of(coll))?;
                    self.load_slot(*collection, "a0");
                    self.load_slot(*value, "a1");
                    self.line(&format!("li a2, {tag}"));
                    self.line("call logos_seq_contains");
                    self.store_slot(*dst, "a0");
                } else {
                    return Err(RiscVLowerError::Unsupported(
                        "membership on a non-collection",
                    ));
                }
            }
            Op::NewEmptySet { dst } => {
                self.line("call logos_seq_new");
                self.store_slot(*dst, "a0");
                self.fresh(&[*dst]);
            }
            Op::SetAdd { set, value } => {
                let tag = kind_tag(self.kind_of(kinds, *value))?;
                self.own_target(*set, &[*value]);
                self.load_slot(*value, "a2");
                self.line(&format!("li a3, {tag}"));
                self.line("call logos_set_add_own");
            }
            Op::RemoveFrom { collection, value } => {
                if !is_concrete_set(self.kind_of(kinds, *collection)) {
                    return Err(RiscVLowerError::Unsupported("Remove from a non-set"));
                }
                let tag = kind_tag(self.kind_of(kinds, *value))?;
                self.own_target(*collection, &[*value]);
                self.load_slot(*value, "a2");
                self.line(&format!("li a3, {tag}"));
                self.line("call logos_set_remove_own");
            }
            Op::UnionOp { dst, lhs, rhs } | Op::IntersectOp { dst, lhs, rhs } => {
                let lk = self.kind_of(kinds, *lhs);
                if !is_concrete_set(lk) {
                    return Err(RiscVLowerError::Unsupported("set algebra on a non-set"));
                }
                let tag = kind_tag(elem_of(lk))?;
                self.load_slot(*lhs, "a0");
                self.load_slot(*rhs, "a1");
                self.line(&format!("li a2, {tag}"));
                let f = if matches!(op, Op::UnionOp { .. }) {
                    "logos_set_union"
                } else {
                    "logos_set_intersect"
                };
                self.line(&format!("call {f}"));
                self.store_slot(*dst, "a0");
                self.inspects(&[*lhs, *rhs]);
                self.fresh(&[*dst]);
            }

            // `copy of x` — the VM's `deep_clone`.
            Op::DeepClone { dst, src } => self.deep_copy(*dst, *src, kinds)?,

            Op::GlobalGet { dst, idx } => {
                self.line(&format!("la {ADDR_TMP}, logos_globals"));
                self.line(&format!("ld t0, {}({ADDR_TMP})", 8 * (*idx as i64)));
                self.store_slot(*dst, "t0");
            }
            Op::GlobalSet { idx, src } => {
                self.load_slot(*src, "t0");
                self.line(&format!("la {ADDR_TMP}, logos_globals"));
                self.line(&format!("sd t0, {}({ADDR_TMP})", 8 * (*idx as i64)));
            }

            Op::Show { src } => {
                self.inspects(&[*src]);
                self.show(*src, kinds)?;
            }

            Op::CallBuiltin {
                dst,
                builtin,
                args_start,
                ..
            } => self.call_builtin(*dst, *builtin, *args_start, kinds)?,

            // "error WHEN EXECUTED" (a failed contract, a non-exhaustive Inspect fallthrough, an unbound
            // Set, …): the VM's exact reason on stderr, exit 1 — through the same `logos_fatal` every
            // failure path reports with.
            Op::FailWith { msg } => match self.prog.constants.get(*msg as usize) {
                Some(Constant::Text(text)) => {
                    let label = self.ro.intern_text(text);
                    self.line(&format!("la a0, {label}"));
                    self.line("call logos_fatal");
                }
                _ => {
                    return Err(RiscVLowerError::Unsupported(
                        "FailWith of a constant that is not Text",
                    ))
                }
            },

            other => return Err(RiscVLowerError::Unsupported(other.name())),
        }
        Ok(())
    }

    /// `Show` of `src` by its kind.
    fn show(&mut self, src: u16, kinds: &[Kind]) -> Emit {
        match self.kind_of(kinds, src) {
            // A word shows as its unsigned value, which its zero-extended slot holds as an Int.
            Kind::Int | Kind::Word32 => {
                self.ro.need_fmt_i64 = true;
                self.load_slot(src, "a1");
                self.line("la a0, .Lfmt_i64");
                self.line("call printf");
            }
            Kind::Bool => {
                self.ro.need_bool = true;
                self.load_slot(src, "t0");
                self.line("la a0, .Lstr_true");
                self.line("bnez t0, 1f");
                self.line("la a0, .Lstr_false");
                self.line("1:");
                self.line("call puts");
            }
            Kind::Float => {
                // The f64 rides in fa0 (a named double param — no variadic marshalling); the runtime
                // shim `logos_show_f64` formats it byte-for-byte like the VM's Show.
                self.fload_slot(src, "fa0");
                self.line("call logos_show_f64");
            }
            Kind::Text => {
                self.load_slot(src, "a0");
                self.line("call logos_show_text");
            }
            Kind::Rational => {
                // Stringify (`num/den`, or `num` when den == 1) then print like any Text.
                self.load_slot(src, "a0");
                self.line("call logos_ldec_str");
                self.line("call logos_show_text");
            }
            Kind::Date => {
                self.load_slot(src, "a0");
                self.line("call logos_date_str");
                self.line("call logos_show_text");
            }
            Kind::Moment => {
                self.load_slot(src, "a0");
                self.line("call logos_moment_str");
                self.line("call logos_show_text");
            }
            Kind::Peer => {
                self.load_slot(src, "a0");
                self.line(&format!("li a1, {PEER_TAG}"));
                self.line("call logos_val_to_str");
                self.line("call logos_show_text");
            }
            Kind::SeqInt | Kind::SeqWord32 => {
                self.load_slot(src, "a0");
                self.line("call logos_seq_show_int");
            }
            Kind::SeqFloat => {
                self.load_slot(src, "a0");
                self.line("call logos_seq_show_float");
            }
            Kind::SeqText => {
                self.load_slot(src, "a0");
                self.line("call logos_seq_show_text");
            }
            Kind::SeqBool => {
                self.load_slot(src, "a0");
                self.line("call logos_seq_show_bool");
            }
            Kind::SetInt => {
                self.load_slot(src, "a0");
                self.line("call logos_set_show_int");
            }
            Kind::SetText => {
                self.load_slot(src, "a0");
                self.line("call logos_set_show_text");
            }
            Kind::Composite(_) => match self.as_composite(self.kind_of(kinds, src)) {
                Some(Composite::Map(key, val)) => {
                    let kt = kind_tag(key)?;
                    let vt = kind_tag(val)?;
                    self.load_slot(src, "a0");
                    self.line(&format!("li a1, {kt}"));
                    self.line(&format!("li a2, {vt}"));
                    self.line("call logos_map_show");
                }
                Some(Composite::Struct(name)) => self.show_struct(src, &name)?,
                _ if self.displays(self.kind_of(kinds, src)) => {
                    let shape = self.shape(self.kind_of(kinds, src))?;
                    self.load_slot(src, "a0");
                    self.line(&format!("la a1, {shape}"));
                    self.line("call logos_show_shaped");
                }
                _ => return Err(RiscVLowerError::Unsupported("Show of this composite value")),
            },
            _ => return Err(RiscVLowerError::Unsupported("Show of non-scalar value")),
        }
        Ok(())
    }

    /// Whether the runtime displays a value of kind `k` by its shape (`logos_shaped_str`): a scalar, and a sequence,
    /// set, tuple or map of what it displays.
    fn displays(&self, k: Kind) -> bool {
        match k {
            Kind::Int
            | Kind::Bool
            | Kind::Float
            | Kind::Rational
            | Kind::Text
            | Kind::Date
            | Kind::Moment
            | Kind::Peer
            | Kind::Word32
            | Kind::CrdtCounter
            | Kind::Tower => true,
            Kind::Span | Kind::Duration => false,
            Kind::SeqInt
            | Kind::SeqFloat
            | Kind::SeqText
            | Kind::SeqBool
            | Kind::SeqWord32
            | Kind::SetInt
            | Kind::SetText => true,
            Kind::Composite(_) => match self.as_composite(k) {
                Some(Composite::Seq(e)) => self.displays(e),
                Some(Composite::Tuple(ps)) => ps.iter().all(|p| self.displays(*p)),
                Some(Composite::Map(key, v)) => self.displays(key) && self.displays(v),
                _ => false,
            },
            Kind::SeqUnknown | Kind::SetUnknown | Kind::MapUnknown | Kind::Unknown => false,
        }
    }

    /// The kind of a binary arithmetic op: `Int` if BOTH operands are Int, else `Float` (any Float
    /// operand promotes the whole op — matching the VM's `(i as f64) op f`). Non-numeric operands are
    /// refused.
    fn num_kind(&self, kinds: &[Kind], lhs: u16, rhs: u16) -> Result<Kind, RiscVLowerError> {
        let numeric = |k| matches!(k, Kind::Int | Kind::Float);
        let (l, r) = (self.kind_of(kinds, lhs), self.kind_of(kinds, rhs));
        if !numeric(l) || !numeric(r) {
            return Err(RiscVLowerError::Unsupported(
                "non-numeric arithmetic operand",
            ));
        }
        Ok(if l == Kind::Int && r == Kind::Int {
            Kind::Int
        } else {
            Kind::Float
        })
    }

    /// Emit code leaving a fresh `char*` in `a0` for `FormatValue`'s `src` under the compile-time-known
    /// format `spec`. Mirrors `semantics::format::apply_format_spec`'s cascade EXACTLY (currency, then
    /// precision, then alignment, then bare width, then the plain display fallthrough) so a malformed
    /// spec degrades identically to the VM.
    fn emit_format_spec(&mut self, spec: &str, src: u16, sk: Kind) -> Emit {
        // Stringify `src` by its scalar kind into `a0` (the VM's `to_display_string`).
        let display = |s: &mut Self| -> Emit {
            let tag = kind_tag(sk)?;
            s.load_slot(src, "a0");
            s.line(&format!("li a1, {tag}"));
            s.line("call logos_val_to_str");
            Ok(())
        };
        // Currency `$` — `${:.2}` for a numeric; non-numeric degrades to the plain display.
        if spec == "$" {
            return match sk {
                Kind::Int | Kind::Float => {
                    self.load_num_as_float(src, sk, "fa0")?;
                    self.line("call logos_fmt_currency");
                    Ok(())
                }
                _ => display(self),
            };
        }
        // Precision `.N` — `{:.N}` fixed-decimal for a numeric; non-numeric degrades to the plain display.
        if let Some(rest) = spec.strip_prefix('.') {
            if let Ok(prec) = rest.parse::<usize>() {
                return match sk {
                    Kind::Int | Kind::Float => {
                        self.load_num_as_float(src, sk, "fa0")?;
                        self.line(&format!("li a0, {prec}"));
                        self.line("call logos_fmt_fixed");
                        Ok(())
                    }
                    _ => display(self),
                };
            }
        }
        // Alignment `>N`/`<N`/`^N` — pad the display string to width N.
        let b = spec.as_bytes();
        if spec.len() >= 2 && (b[0] == b'>' || b[0] == b'<' || b[0] == b'^') {
            if let Ok(width) = spec[1..].parse::<usize>() {
                display(self)?;
                self.line(&format!("li a1, {width}"));
                self.line(&format!("li a2, {}", b[0]));
                self.line("call logos_pad");
                return Ok(());
            }
        }
        // Bare width `N` — right-align (Rust's default) to width N.
        if let Ok(width) = spec.parse::<usize>() {
            display(self)?;
            self.line(&format!("li a1, {width}"));
            self.line("li a2, 62"); // '>'
            self.line("call logos_pad");
            return Ok(());
        }
        // Unrecognized spec → the plain display string.
        display(self)
    }

    /// Load `reg` into an FP register as f64, promoting an Int via `fcvt.d.l` (round-to-nearest-even —
    /// bit-identical to the VM's `i64 as f64`). Refuses non-numeric operands.
    fn load_num_as_float(&mut self, reg: u16, kind: Kind, ftreg: &str) -> Emit {
        match kind {
            Kind::Float => self.fload_slot(reg, ftreg),
            Kind::Int => {
                self.load_slot(reg, "t0");
                self.line(&format!("fcvt.d.l {ftreg}, t0"));
            }
            _ => {
                return Err(RiscVLowerError::Unsupported(
                    "numeric promotion of non-numeric operand",
                ))
            }
        }
        Ok(())
    }

    /// `dst = lhs <op> rhs` — a native RV64 integer op (`iop`) or a D-extension float op (`fop`). Mixed
    /// Int/Float promotes the Int operand to f64 (exactly as the VM does), so the whole op is float.
    /// Keep the low 32 bits of `reg`, zero-extended — the canonical Word32 storage form (`v & 0xFFFFFFFF`).
    fn mask_word32(&mut self, reg: &str) {
        self.line(&format!("slli {reg}, {reg}, 32"));
        self.line(&format!("srli {reg}, {reg}, 32"));
    }

    fn arith(
        &mut self,
        dst: u16,
        lhs: u16,
        rhs: u16,
        kinds: &[Kind],
        iop: &str,
        fop: &str,
    ) -> Emit {
        // A Rational operand routes to exact rational arithmetic (`ratop` selects the C helper's op).
        let (lk, rk) = (self.kind_of(kinds, lhs), self.kind_of(kinds, rhs));
        if temporal_kind(iop, lk, rk).is_some() {
            self.load_slot(lhs, "t0");
            self.load_slot(rhs, "t1");
            self.line(&format!("{iop} t2, t0, t1"));
            self.store_slot(dst, "t2");
            return Ok(());
        }
        // `text + x` is string concatenation, exactly like the VM's `arith::add` (it stringifies the
        // non-text operand via `to_display_string`). Only `add` concatenates — `sub`/`mul`/… on a Text
        // operand error in the VM and fall through to the numeric path's refusal below.
        if iop == "add" && (lk == Kind::Text || rk == Kind::Text) {
            let (lt, rt) = (kind_tag(lk)?, kind_tag(rk)?);
            self.load_slot(lhs, "a0");
            self.line(&format!("li a1, {lt}"));
            self.load_slot(rhs, "a2");
            self.line(&format!("li a3, {rt}"));
            self.line("call logos_concat_vals");
            self.store_slot(dst, "a0");
            return Ok(());
        }
        if lk == Kind::Rational || rk == Kind::Rational {
            let op = match iop {
                "add" => 0,
                "sub" => 1,
                "mul" => 2,
                _ => {
                    return Err(RiscVLowerError::Unsupported(
                        "this arithmetic op on a Rational",
                    ))
                }
            };
            if lk == Kind::Float || rk == Kind::Float {
                return self.rat_real_op(dst, lhs, rhs, kinds, op);
            }
            return self.rat_binop(dst, lhs, rhs, kinds, op);
        }
        // Word32 arithmetic wraps mod 2³² — do the 64-bit op then keep the low 32 bits (zero-extended),
        // exactly the VM's `WordVal::W32` ring.
        if lk == Kind::Word32 || rk == Kind::Word32 {
            if !matches!(iop, "add" | "sub" | "mul") {
                return Err(RiscVLowerError::Unsupported(
                    "this arithmetic op on a Word32",
                ));
            }
            self.load_slot(lhs, "t0");
            self.load_slot(rhs, "t1");
            self.line(&format!("{iop} t2, t0, t1"));
            self.mask_word32("t2");
            self.store_slot(dst, "t2");
            return Ok(());
        }
        match self.num_kind(kinds, lhs, rhs)? {
            Kind::Float => {
                self.load_num_as_float(lhs, lk, "ft0")?;
                self.load_num_as_float(rhs, rk, "ft1")?;
                self.line(&format!("{fop} ft2, ft0, ft1"));
                self.fstore_slot(dst, "ft2");
            }
            _ => {
                self.load_slot(lhs, "t0");
                self.load_slot(rhs, "t1");
                self.line(&format!("{iop} t2, t0, t1"));
                self.store_slot(dst, "t2");
            }
        }
        Ok(())
    }

    /// The constant `reg` holds before the op at `pcs[at]`, when the straight-line code leading to it loaded
    /// one into it: a `LoadConst`, with no other write of `reg` and no jump target between.
    fn const_before(&self, pcs: &[usize], at: usize, reg: u16) -> Option<&Constant> {
        for i in (0..at).rev() {
            let (p, next) = (pcs[i], pcs[i + 1]);
            if next != p + 1 || self.jump_targets.contains(&next) {
                return None;
            }
            let op = self.prog.code[p];
            if let Op::LoadConst { dst, idx } = op {
                if dst == reg {
                    return self.prog.constants.get(idx as usize);
                }
            }
            let mut writes = false;
            self.for_each_written(&op, |w| writes |= w == reg);
            if writes {
                return None;
            }
        }
        None
    }

    /// The Text constant `reg` holds before the op being lowered ([`Emitter::const_before`]).
    fn text_const_before(&self, reg: u16) -> Option<String> {
        let at = self.region_pcs.iter().position(|&p| p == self.pc)?;
        match self.const_before(&self.region_pcs, at, reg)? {
            Constant::Text(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// The cell of record type `record`'s field named by the Text constant in `name_reg`.
    fn record_field_named(&self, record: &str, name_reg: u16) -> Result<usize, RiscVLowerError> {
        let name = self
            .text_const_before(name_reg)
            .ok_or(RiscVLowerError::Unsupported(
                "a record's field named by a value not known when compiling",
            ))?;
        self.field_cell_by_name(record, &name)
            .map(|(cell, _)| cell)
            .ok_or(RiscVLowerError::Unsupported(
                "a record's field it does not declare",
            ))
    }

    /// The label of the shape of a value of kind `k`, what a deep copy (`logos_deep_copy`, the VM's
    /// `deep_clone`) and a structural comparison (`logos_deep_eq`, the VM's `values_equal`) do with each
    /// 8-byte cell: a table `[form, n, parts]` whose form is, for a value a copy keeps as it is, 0 when its
    /// bits are an Int, 7 for Text, 8 for Float and 9 for Rational; 1 for a sequence, 10 for a set,
    /// 2 for a map, 3 for a declared enum, 6 for a counter, 11 for a tuple, 12 for a nominal record,
    /// 13 for Option and 14 for Result. Forms 16 through 21 distinguish Bool, Date, Moment, Word32,
    /// opaque identities and Peer. Aggregates carry child shapes; enums carry tagged payload shapes.
    /// Keeping the kinds distinct lets two layouts compare their cells by value without losing
    /// nominal identity or giving tuples, records and enums the collections' identity shortcut.
    fn shape(&mut self, k: Kind) -> Result<String, RiscVLowerError> {
        if let Some((_, label)) = self.shapes.iter().find(|(seen, _)| *seen == k) {
            return Ok(label.clone());
        }
        let label = format!(".Lshape_{}", self.shapes.len());
        self.shapes.push((k, label.clone()));
        let table = |form: i64, parts: &[String]| -> Vec<String> {
            vec![
                form.to_string(),
                parts.len().to_string(),
                format!("{label}_parts"),
            ]
        };
        let (form, parts): (i64, Vec<String>) = match k {
            Kind::Int => (0, Vec::new()),
            Kind::Bool => (16, Vec::new()),
            Kind::Date => (17, Vec::new()),
            Kind::Moment => (18, Vec::new()),
            Kind::Word32 => (19, Vec::new()),
            Kind::Text => (7, Vec::new()),
            Kind::Peer => (21, Vec::new()),
            Kind::Float => (8, Vec::new()),
            Kind::Rational => (9, Vec::new()),
            Kind::Tower => (22, Vec::new()),
            Kind::Span => (24, Vec::new()),
            Kind::Duration => {
                return Err(RiscVLowerError::Unsupported("a Duration inside a value the runtime shapes"));
            }
            Kind::SeqInt | Kind::SeqFloat | Kind::SeqText | Kind::SeqBool | Kind::SeqWord32 => {
                (1, vec![self.shape(elem_of(k))?])
            }
            Kind::SetInt | Kind::SetText => (10, vec![self.shape(elem_of(k))?]),
            Kind::CrdtCounter => (6, Vec::new()),
            Kind::Composite(_) => match self.as_composite(k).expect("an interned composite") {
                Composite::Seq(e) => (1, vec![self.shape(e)?]),
                Composite::Tuple(ps) => (
                    11,
                    ps.into_iter()
                        .map(|p| self.shape(p))
                        .collect::<Result<_, _>>()?,
                ),
                Composite::Map(key, v) => (2, vec![self.shape(key)?, self.shape(v)?]),
                Composite::Struct(name) => {
                    let fields: Vec<Kind> = self
                        .prog
                        .struct_types
                        .iter()
                        .find(|sd| sd.name == name)
                        .ok_or(RiscVLowerError::Unsupported(
                            "a copy of an unknown record type",
                        ))?
                        .fields
                        .iter()
                        .map(|(_, t)| self.boundary_kind(&Some(t.clone())))
                        .collect();
                    let mut parts = Vec::with_capacity(fields.len());
                    for (cell, fk) in fields.into_iter().enumerate() {
                        let fk = match self.crdt_field(&name, cell) {
                            Some(code) if is_counter_crdt(code) => Kind::CrdtCounter,
                            Some(code) if lays_out_as_its_value(code) => fk,
                            None => fk,
                            Some(_) => {
                                return Err(RiscVLowerError::Unsupported(
                                    "a copy of a convergent field this tier does not model",
                                ))
                            }
                        };
                        parts.push(self.shape(fk)?);
                    }
                    (12, parts)
                }
                enum_like @ (Composite::Enum(_) | Composite::Option(_) | Composite::Result(..)) => {
                    let form = match &enum_like {
                        Composite::Option(_) => 13,
                        Composite::Result(..) => 14,
                        _ => 3,
                    };
                    let variants: Vec<(String, Vec<Kind>)> = match enum_like {
                        Composite::Enum(name) => self
                            .prog
                            .enum_types
                            .iter()
                            .find(|e| e.name == name)
                            .ok_or(RiscVLowerError::Unsupported(
                                "a copy of an enum whose constructors are not declared",
                            ))?
                            .variants
                            .iter()
                            .map(|v| {
                                (
                                    v.name.clone(),
                                    v.field_types
                                        .iter()
                                        .map(|t| self.boundary_kind(&Some(t.clone())))
                                        .collect(),
                                )
                            })
                            .collect(),
                        Composite::Option(some) => {
                            let mut variants = vec![("None".into(), Vec::new())];
                            if let Some(kind) = some {
                                variants.push(("Some".into(), vec![kind]));
                            }
                            variants
                        }
                        Composite::Result(ok, err) => {
                            let mut variants = Vec::new();
                            if let Some(kind) = ok {
                                variants.push(("Ok".into(), vec![kind]));
                            }
                            if let Some(kind) = err {
                                variants.push(("Err".into(), vec![kind]));
                            }
                            variants
                        }
                        _ => unreachable!("matched as enum-like"),
                    };
                    let mut records = Vec::new();
                    for (vi, (name, payloads)) in variants.into_iter().enumerate() {
                        let Some(tag) = self.ctor_tag(&name) else {
                            continue;
                        };
                        let payload_shapes = payloads
                            .into_iter()
                            .map(|p| self.shape(p))
                            .collect::<Result<Vec<_>, _>>()?;
                        let record = format!("{label}_v{vi}");
                        self.ro.table(
                            record.clone(),
                            vec![
                                tag.to_string(),
                                payload_shapes.len().to_string(),
                                format!("{record}_parts"),
                            ],
                        );
                        self.ro.table(format!("{record}_parts"), payload_shapes);
                        records.push(record);
                    }
                    (form, records)
                }
                Composite::Closure(_) | Composite::Chan(_) => (20, Vec::new()),
            },
            Kind::SeqUnknown | Kind::SetUnknown | Kind::MapUnknown | Kind::Unknown => {
                return Err(RiscVLowerError::Unsupported(
                    "a copy of a value whose kind is not modeled",
                ))
            }
        };
        self.ro.table(label.clone(), table(form, &parts));
        self.ro.table(format!("{label}_parts"), parts);
        Ok(label)
    }

    /// `dst = (lhs == rhs)` (or `!=`) by structure, the VM's `values_equal`, for values that are not scalars:
    /// equal kinds use one shape; different known kinds retain both layouts for exact cell comparisons.
    fn structural_eq(
        &mut self,
        op: &str,
        dst: u16,
        lhs: u16,
        rhs: u16,
        lk: Kind,
        rk: Kind,
    ) -> Emit {
        let k = self.unify(lk, rk);
        let unknown = |kind| {
            matches!(
                kind,
                Kind::Unknown | Kind::SeqUnknown | Kind::SetUnknown | Kind::MapUnknown
            )
        };
        let pair = lk != rk && !unknown(lk) && !unknown(rk);
        if !pair && k == Kind::Unknown {
            self.line(&format!("li t1, {}", (op == "ne") as i64));
            self.store_slot(dst, "t1");
            return Ok(());
        }
        // Equality can compare two different element kinds without converting either value.
        let shape = self.shape(if pair { lk } else { k })?;
        self.load_slot(lhs, "a0");
        self.load_slot(rhs, "a1");
        self.line(&format!("la a2, {shape}"));
        if pair {
            let right_shape = self.shape(rk)?;
            self.line(&format!("la a3, {right_shape}"));
            self.line("call logos_deep_eq_pair");
        } else {
            self.line("call logos_deep_eq");
        }
        if op == "ne" {
            self.line("xori a0, a0, 1");
        }
        self.store_slot(dst, "a0");
        Ok(())
    }

    /// Whether equality on kind `k` compares structure (a collection, record, tuple or enum value).
    fn is_structural(&self, k: Kind) -> bool {
        is_collection(k)
            || matches!(k, Kind::SeqUnknown | Kind::SetUnknown | Kind::MapUnknown)
            || matches!(
                self.as_composite(k),
                Some(
                    Composite::Seq(_)
                        | Composite::Tuple(_)
                        | Composite::Map(..)
                        | Composite::Struct(_)
                        | Composite::Enum(_)
                        | Composite::Option(_)
                        | Composite::Result(..)
                )
            )
    }

    /// `dst` = a deep copy of `src` (the VM's `deep_clone`): the value itself when it is copied as it is,
    /// else a copy by its shape — a fresh collection its register owns.
    fn deep_copy(&mut self, dst: u16, src: u16, kinds: &[Kind]) -> Emit {
        let k = self.kind_of(kinds, src);
        let shape = self.shape(k)?;
        self.inspects(&[src]);
        self.load_slot(src, "a0");
        if matches!(
            k,
            Kind::Int
                | Kind::Float
                | Kind::Bool
                | Kind::Text
                | Kind::Rational
                | Kind::Date
                | Kind::Moment
                | Kind::Peer
                | Kind::Word32
        ) {
            self.store_slot(dst, "a0");
            return Ok(());
        }
        self.line(&format!("la a1, {shape}"));
        self.line("call logos_deep_copy");
        self.store_slot(dst, "a0");
        self.fresh(&[dst]);
        Ok(())
    }

    /// How a merge treats field `cell` (of kind `kind`) of record type `record` — a `how` code and two kind
    /// codes — the VM's `crdt_merge_field` case by case (`crdt_join_field` for a join, which differs only on a
    /// plain Int). Inline: 0 the source's value, 1 the wrapping sum of Ints, 2 the larger Int. Through
    /// `logos_merge_cell`: 3 a counter's per-replica maximum into a fresh counter, 4 the union of sets of element
    /// kind `a`, 5 the key-wise merge of maps from key kind `a` to Int (`b` 0, summed) or to another scalar (`b`
    /// 1, the source's).
    fn merge_rule(
        &self,
        record: &str,
        cell: usize,
        kind: Kind,
        is_join: bool,
    ) -> Result<(u8, u8, u8), RiscVLowerError> {
        match self.crdt_field(record, cell) {
            Some(code) if is_counter_crdt(code) => return Ok((3, 0, 0)),
            Some(_) => {
                return Err(RiscVLowerError::Unsupported(
                    "a merge of a convergent field this tier does not model",
                ))
            }
            None => {}
        }
        Ok(match kind {
            Kind::Int if is_join => (2, 0, 0),
            Kind::Int => (1, 0, 0),
            Kind::SetInt | Kind::SetText => (4, kind_tag(elem_of(kind))?, 0),
            k => match self.as_composite(k) {
                Some(Composite::Map(key, Kind::Int)) => (5, kind_tag(key)?, 0),
                Some(Composite::Map(key, v))
                    if matches!(v, Kind::Float | Kind::Bool | Kind::Text) =>
                {
                    (5, kind_tag(key)?, 1)
                }
                Some(Composite::Map(..)) => {
                    return Err(RiscVLowerError::Unsupported(
                        "a merge of a map whose values merge as collections",
                    ))
                }
                _ if k == Kind::Unknown || k == Kind::SetUnknown || k == Kind::MapUnknown => {
                    return Err(RiscVLowerError::Unsupported(
                        "a merge of a field whose kind is not modeled",
                    ))
                }
                _ => (0, 0, 0),
            },
        })
    }

    /// `a0` = the relay topic the text of `reg`'s value names — `Listen`'s and a `PeerAgent`'s address,
    /// read as the VM reads it: the value displayed, then canonicalized.
    fn net_address(&mut self, reg: u16, kinds: &[Kind]) -> Emit {
        let tag = kind_tag(self.kind_of(kinds, reg))?;
        self.load_slot(reg, "a0");
        self.line(&format!("li a1, {tag}"));
        self.line("call logos_val_to_str");
        self.line("call logos_net_topic");
        Ok(())
    }

    /// `a0` = the relay topic a `Send`/`Await` peer operand names ([`crate::concurrency::net_inbox::peer_topic`]):
    /// a `PeerAgent`'s own, an address text's canonical one. Any other value stops the program with the reason
    /// that function gives it.
    fn net_peer(&mut self, reg: u16, kinds: &[Kind]) -> Emit {
        match self.kind_of(kinds, reg) {
            Kind::Peer => self.load_slot(reg, "a0"),
            Kind::Text => {
                self.load_slot(reg, "a0");
                self.line("call logos_net_topic");
            }
            k => {
                let value = self.exemplar(k).ok_or(RiscVLowerError::Unsupported(
                    "a peer operand whose kind is not modeled",
                ))?;
                let reason = crate::concurrency::net_inbox::peer_topic(&value)
                    .expect_err("only a peer or a text names a topic");
                let reason = self.ro.intern_text(&reason);
                self.line(&format!("la a0, {reason}"));
                self.line("call logos_fatal");
            }
        }
        Ok(())
    }

    /// `Spawn`/`SpawnHandle` — `logos_spawn_fn(func_idx, nargs, args)`, the arguments being the frame's
    /// contiguous slots from `args_start` (the task copies them). The task handle is left in a0 (the caller
    /// stores it for `SpawnHandle`). A task body takes its arguments in registers, as any call does, so
    /// the limit is a call's.
    fn emit_spawn(&mut self, func: u16, args_start: u16, arg_count: u16) -> Emit {
        if arg_count > 8 {
            return Err(RiscVLowerError::Unsupported(
                "spawn with more than 8 arguments",
            ));
        }
        self.slot_addr(args_start, "a2");
        self.line(&format!("li a0, {func}"));
        self.line(&format!("li a1, {arg_count}"));
        self.line("call logos_spawn_fn");
        Ok(())
    }

    /// `dst = lhs <op> rhs` on exact numbers via `logos_ldec_op` (`op`: 0 `+`, 1 `-`, 2 `*`, 3 `/`, 4 the exact `/`,
    /// 5 `//`, 6 `%`, 7 `**`). Each operand is an Int or a Rational handle; passing the kind flags lets the whole op
    /// be a single call, so no handle is spilled across it.
    fn rat_binop(&mut self, dst: u16, lhs: u16, rhs: u16, kinds: &[Kind], op: u8) -> Emit {
        let flag = |k: Kind| -> Result<u8, RiscVLowerError> {
            match k {
                Kind::Rational => Ok(1),
                Kind::Int => Ok(0),
                _ => Err(RiscVLowerError::Unsupported(
                    "rational arithmetic with a non-int/rational operand",
                )),
            }
        };
        let (lf, rf) = (
            flag(self.kind_of(kinds, lhs))?,
            flag(self.kind_of(kinds, rhs))?,
        );
        self.load_slot(lhs, "a0");
        self.line(&format!("li a1, {lf}"));
        self.load_slot(rhs, "a2");
        self.line(&format!("li a3, {rf}"));
        self.line(&format!("li a4, {op}"));
        self.line("call logos_ldec_op");
        self.store_slot(dst, "a0");
        Ok(())
    }

    /// Register `reg` into `into` as the machine integer an op at `place` takes: the register itself, or the integer a
    /// whole rational equals, a fraction or one past i64 refused in the VM's words for the place. The conversion is a
    /// call, so it goes first, before the op's other arguments are loaded.
    fn int_operand(&mut self, kinds: &[Kind], reg: u16, place: crate::codegen_c::words::IntegerPlace, into: &str) -> Emit {
        if self.kind_of(kinds, reg) != Kind::Rational {
            self.load_slot(reg, into);
            return Ok(());
        }
        let (past_word, fraction) = self.integer_place(place);
        self.load_slot(reg, "a0");
        self.line(&format!("la a1, {past_word}"));
        self.line(&format!("la a2, {fraction}"));
        self.line("call logos_ldec_as_i64");
        if into != "a0" {
            self.line(&format!("mv {into}, a0"));
        }
        Ok(())
    }

    /// The labels of the VM's sentences for a number past i64 and for a fraction where it takes a machine integer
    /// at `place`.
    fn integer_place(&mut self, place: crate::codegen_c::words::IntegerPlace) -> (String, String) {
        let (past_word, fraction) = crate::semantics::collections::integer_place_refusals(place);
        (self.ro.intern_text(&past_word), self.ro.intern_text(&fraction))
    }

    /// Present each operand `op` hands to a place declared a rational, or a list of rationals, that holds an integer or
    /// a list of integers: converted into a scratch slot the op reads it from ([`Emitter::presented`]), after any the
    /// op's exact-extension operands took.
    fn present_numbers(&mut self, op: &Op, kinds: &[Kind]) -> Result<Vec<Kind>, RiscVLowerError> {
        let mut seen = kinds.to_vec();
        let declared = |e: &Self, f: usize| e.param_kinds.borrow().get(f).cloned().unwrap_or_default();
        let mut widen: Vec<(u16, Kind)> = Vec::new();
        let mut handed = |e: &Self, params: &[Kind], args_start: u16, arg_count: u16| {
            for i in 0..arg_count {
                let reg = args_start + i;
                if let Some(&to) = params.get(i as usize) {
                    if e.widens_to(kind_at(kinds, reg), to) {
                        widen.push((reg, to));
                    }
                }
            }
        };
        match *op {
            Op::Call {
                func,
                args_start,
                arg_count,
                ..
            } => handed(self, &declared(self, func as usize), args_start, arg_count),
            Op::CallValue {
                callee,
                args_start,
                arg_count,
                ..
            } => {
                if let Some(Composite::Closure(f)) = self.as_composite(kind_at(kinds, callee)) {
                    handed(self, &declared(self, f as usize), args_start, arg_count);
                }
            }
            Op::Return { src } => {
                let answers = self.current_fn.map(|fi| self.returns.borrow()[fi]);
                if let Some(to) = answers.filter(|to| self.widens_to(kind_at(kinds, src), *to)) {
                    widen.push((src, to));
                }
            }
            // The language holds nothing in a rational field a record is built without, which a compiled rational
            // cannot hold.
            Op::StructInsert { obj, field, value } => {
                if let Some((_, to)) = self.resolve_field(self.kind_of(kinds, obj), field) {
                    if to == Kind::Rational && self.is_nothing_literal(self.pc, value) {
                        return Err(RiscVLowerError::Unsupported(
                            "a record's rational field holding nothing: a compiled rational holds no nothing",
                        ));
                    }
                    if self.widens_to(kind_at(kinds, value), to) {
                        widen.push((value, to));
                    }
                }
            }
            _ => {}
        }
        let mut at = self.scratch_base + 8 * self.presented.len() as i64;
        if self.presented.len() + widen.len() > SCRATCH_SLOTS {
            return Err(RiscVLowerError::Unsupported(
                "an op with more operands to present than a frame has scratch slots",
            ));
        }
        for (reg, to) in widen {
            self.load_slot(reg, "a0");
            self.line(if to == Kind::Rational {
                "call logos_ldec_of_int"
            } else {
                "call logos_seq_as_ldec"
            });
            self.store_at_sp(at, "a0");
            self.presented.insert(reg, at);
            if let Some(k) = seen.get_mut(reg as usize) {
                *k = to;
            }
            at += 8;
        }
        Ok(seen)
    }

    /// Whether a value of kind `from` is handed to a place declared `to` as the exact number it equals: an integer
    /// where a rational is declared, a list of integers where a list of rationals is.
    fn widens_to(&self, from: Kind, to: Kind) -> bool {
        match (from, to) {
            (Kind::Int, Kind::Rational) => true,
            (Kind::SeqInt, _) => self.rational_list(to),
            _ => false,
        }
    }

    /// Whether any of `regs` holds a rational.
    fn over_rational(&self, kinds: &[Kind], regs: &[u16]) -> bool {
        regs.iter().any(|r| self.kind_of(kinds, *r) == Kind::Rational)
    }

    /// Whether any of `regs` holds a real.
    fn over_real(&self, kinds: &[Kind], regs: &[u16]) -> bool {
        regs.iter().any(|r| self.kind_of(kinds, *r) == Kind::Float)
    }

    /// `dst = lhs <op> rhs` with a rational beside a real, computed on the rational's double as the VM does, by
    /// `logos_ldec_real_op` (`op`: 0 `+`, 1 `-`, 2 `*`, 3 `/`, 4 `**`).
    fn rat_real_op(&mut self, dst: u16, lhs: u16, rhs: u16, kinds: &[Kind], op: u8) -> Emit {
        let left = self.kind_of(kinds, lhs) == Kind::Rational;
        let (rational, real) = if left { (lhs, rhs) } else { (rhs, lhs) };
        if self.kind_of(kinds, real) != Kind::Float {
            return Err(RiscVLowerError::Unsupported("a rational beside a non-real operand"));
        }
        self.load_slot(rational, "a0");
        self.fload_slot(real, "fa0");
        self.line(&format!("li a1, {}", left as u8));
        self.line(&format!("li a2, {op}"));
        self.line("call logos_ldec_real_op");
        self.fstore_slot(dst, "fa0");
        Ok(())
    }

    /// `dst = lhs <op> rhs` bitwise with a rational operand (`symbol` '&', '|', '^', '<', '>'): two whole numbers as
    /// their integers, a fraction refused, by `logos_ldec_bitwise`.
    fn rat_bitwise(&mut self, dst: u16, lhs: u16, rhs: u16, kinds: &[Kind], symbol: char) -> Emit {
        let flag = |k: Kind| -> Result<u8, RiscVLowerError> {
            match k {
                Kind::Rational => Ok(1),
                Kind::Int | Kind::Bool => Ok(0),
                _ => Err(RiscVLowerError::Unsupported("bitwise op on a rational and a non-integer")),
            }
        };
        let (lf, rf) = (flag(self.kind_of(kinds, lhs))?, flag(self.kind_of(kinds, rhs))?);
        self.load_slot(lhs, "a0");
        self.line(&format!("li a1, {lf}"));
        self.load_slot(rhs, "a2");
        self.line(&format!("li a3, {rf}"));
        self.line(&format!("li a4, {}", symbol as u32));
        self.line("call logos_ldec_bitwise");
        self.store_slot(dst, "a0");
        Ok(())
    }

    /// `dst = rational <op> real` (or `real <op> rational` when `reversed`), by the real's exact value; a NaN orders
    /// against nothing and equals nothing.
    fn rat_real_cmp(&mut self, op: &str, dst: u16, rational: u16, real: u16, reversed: bool) -> Emit {
        self.load_slot(rational, "a0");
        self.fload_slot(real, "fa0");
        self.line("call logos_ldec_cmp_f64"); // a0 = -1/0/1, 2 for NaN
        let op = match (op, reversed) {
            ("lt", true) => "gt",
            ("gt", true) => "lt",
            ("le", true) => "ge",
            ("ge", true) => "le",
            (o, _) => o,
        };
        match op {
            "lt" => self.line("sltz a0, a0"),
            "gt" => {
                self.line("addi a0, a0, -1");
                self.line("seqz a0, a0");
            }
            "le" => self.line("slti a0, a0, 1"),
            "ge" => self.line("sltiu a0, a0, 2"), // 0 or 1; -1 and NaN's 2 are not
            "eq" => self.line("seqz a0, a0"),
            "ne" => self.line("snez a0, a0"),
            _ => return Err(RiscVLowerError::Unsupported("unknown rational comparison")),
        }
        self.store_slot(dst, "a0");
        Ok(())
    }

    /// `into` = a value nonzero exactly when `reg`'s value is truthy (the VM's `is_truthy`): a nonzero number,
    /// a non-empty Text or collection, and any other value but `false`. A condition whose kind is unknown
    /// (`raw_unknown`) is read as the bits it holds, the truth of the Bool or Int it is compiled from.
    fn truth(&mut self, reg: u16, kinds: &[Kind], into: &str, raw_unknown: bool) -> Emit {
        let k = self.kind_of(kinds, reg);
        match k {
            Kind::Bool | Kind::Int | Kind::Word32 => self.load_slot(reg, into),
            Kind::Unknown if raw_unknown => self.load_slot(reg, into),
            Kind::Unknown => {
                return Err(RiscVLowerError::Unsupported(
                    "the truth of a value of unknown kind",
                ))
            }
            Kind::Float => {
                self.fload_slot(reg, "ft0");
                self.line("fcvt.d.w ft1, zero");
                self.line(&format!("feq.d {into}, ft0, ft1"));
                self.line(&format!("xori {into}, {into}, 1"));
            }
            Kind::Text => {
                self.load_slot(reg, into);
                self.line(&format!("lbu {into}, 0({into})"));
            }
            Kind::Rational => {
                self.load_slot(reg, into);
                self.line(&format!("ld {into}, 0({into})"));
            }
            k if self.is_map(k) || k == Kind::MapUnknown => {
                self.load_slot(reg, "a0");
                self.line("call logos_map_len");
                self.line(&format!("mv {into}, a0"));
            }
            k if self.is_seq_like(k)
                || is_concrete_set(k)
                || matches!(k, Kind::SeqUnknown | Kind::SetUnknown) =>
            {
                self.load_slot(reg, "a0");
                self.line("call logos_seq_len");
                self.line(&format!("mv {into}, a0"));
            }
            _ => self.line(&format!("li {into}, 1")),
        }
        Ok(())
    }

    /// Float division with a divisor zero-check — LOGOS ERRORS on float `/0` (it never yields inf/nan),
    /// so a zero divisor branches to the fail stub exactly as the VM errors. Promotes a mixed Int operand.
    fn float_div(&mut self, dst: u16, lhs: u16, rhs: u16, kinds: &[Kind]) -> Emit {
        let fail = self.fail_stub(&refusals().float_division);
        let (lk, rk) = (self.kind_of(kinds, lhs), self.kind_of(kinds, rhs));
        self.load_num_as_float(lhs, lk, "ft0")?;
        self.load_num_as_float(rhs, rk, "ft1")?;
        self.line("fcvt.d.w ft2, zero"); // ft2 = 0.0
        self.line("feq.d t0, ft1, ft2");
        self.jump_to_stop(Some(("bnez", "t0")), &fail);
        self.line("fdiv.d ft2, ft0, ft1");
        self.fstore_slot(dst, "ft2");
        Ok(())
    }

    /// A comparison, dispatched on operand kind: float↔float uses the D-extension `feq/flt/fle`; a
    /// MIXED Int↔Float uses the exact runtime comparison (the VM's `cmp_i64_f64_exact`, not a lossy
    /// promotion); everything else goes to [`Self::int_cmp`] (Int, plus Bool for eq/ne, else refuses).
    fn cmp(&mut self, op: &str, dst: u16, lhs: u16, rhs: u16, kinds: &[Kind]) -> Emit {
        self.inspects(&[lhs, rhs]);
        let (lk, rk) = (self.kind_of(kinds, lhs), self.kind_of(kinds, rhs));
        match (lk, rk) {
            (Kind::Float, Kind::Float) => {
                self.float_cmp(op, dst, lhs, rhs);
                Ok(())
            }
            (Kind::Int, Kind::Float) => self.mixed_cmp(op, dst, lhs, rhs, false),
            (Kind::Float, Kind::Int) => self.mixed_cmp(op, dst, rhs, lhs, true),
            (Kind::Rational, Kind::Float) => self.rat_real_cmp(op, dst, lhs, rhs, false),
            (Kind::Float, Kind::Rational) => self.rat_real_cmp(op, dst, rhs, lhs, true),
            (a, b) if a == Kind::Rational || b == Kind::Rational => {
                self.rat_cmp(op, dst, lhs, rhs, kinds)
            }
            (Kind::Text, Kind::Text) => {
                self.text_cmp(op, dst, lhs, rhs);
                Ok(())
            }
            (Kind::Peer, Kind::Peer) if matches!(op, "eq" | "ne") => {
                self.text_cmp(op, dst, lhs, rhs);
                Ok(())
            }
            (l, r)
                if matches!(op, "eq" | "ne")
                    && (self.is_structural(l) || self.is_structural(r))
                    && l != Kind::Unknown
                    && r != Kind::Unknown =>
            {
                self.structural_eq(op, dst, lhs, rhs, l, r)
            }
            // A peer equals only a peer naming the same topic (the VM's `values_equal`).
            (Kind::Peer, _) | (_, Kind::Peer)
                if matches!(op, "eq" | "ne") && lk != Kind::Unknown && rk != Kind::Unknown =>
            {
                self.line(&format!("li t1, {}", (op == "ne") as i64));
                self.store_slot(dst, "t1");
                Ok(())
            }
            _ => self.int_cmp(op, dst, lhs, rhs, kinds),
        }
    }

    /// Compare two `Text` values by content (`strcmp`), matching the VM: `Eq`/`NotEq` are content
    /// equality and `Lt/Gt/…` are byte-lexicographic (Rust's `&str` Ord == byte order for UTF-8).
    fn text_cmp(&mut self, op: &str, dst: u16, lhs: u16, rhs: u16) {
        self.load_slot(lhs, "a0");
        self.load_slot(rhs, "a1");
        self.line("call strcmp"); // a0 = sign of the comparison
        match op {
            "eq" => self.line("seqz t1, a0"),
            "ne" => self.line("snez t1, a0"),
            "lt" => self.line("sltz t1, a0"),
            "gt" => self.line("sgtz t1, a0"),
            "le" => {
                self.line("sgtz t1, a0");
                self.line("xori t1, t1, 1");
            }
            "ge" => {
                self.line("sltz t1, a0");
                self.line("xori t1, t1, 1");
            }
            _ => self.line("seqz t1, a0"),
        }
        self.store_slot(dst, "t1");
    }

    /// A relational op with a Rational operand: `logos_ldec_cmp` yields sign(lhs-rhs) ∈ {-1,0,1}, mapped to the
    /// boolean result. The other operand may be an Int; a Float compares by its exact value
    /// ([`Self::rat_real_cmp`]).
    fn rat_cmp(&mut self, op: &str, dst: u16, lhs: u16, rhs: u16, kinds: &[Kind]) -> Emit {
        let flag = |k: Kind| -> Result<u8, RiscVLowerError> {
            match k {
                Kind::Rational => Ok(1),
                Kind::Int => Ok(0),
                _ => Err(RiscVLowerError::Unsupported(
                    "rational comparison with a non-int/rational operand",
                )),
            }
        };
        let (lf, rf) = (
            flag(self.kind_of(kinds, lhs))?,
            flag(self.kind_of(kinds, rhs))?,
        );
        self.load_slot(lhs, "a0");
        self.line(&format!("li a1, {lf}"));
        self.load_slot(rhs, "a2");
        self.line(&format!("li a3, {rf}"));
        self.line("call logos_ldec_cmp"); // a0 = sign(lhs - rhs)
        match op {
            "lt" => self.line("sltz a0, a0"),    // sign < 0
            "gt" => self.line("sgtz a0, a0"),    // sign > 0
            "le" => self.line("slti a0, a0, 1"), // sign <= 0
            "ge" => {
                self.line("sltz a0, a0"); // sign < 0
                self.line("xori a0, a0, 1"); // sign >= 0
            }
            "eq" => self.line("seqz a0, a0"),
            "ne" => self.line("snez a0, a0"),
            _ => return Err(RiscVLowerError::Unsupported("unknown rational comparison")),
        }
        self.store_slot(dst, "a0");
        Ok(())
    }

    /// Exact mixed Int/Float comparison via the runtime shim. `int_reg`/`float_reg` are the operands
    /// (int first); `reversed` is true when the SOURCE order was (Float, Int), so the shim compares
    /// (float vs int). Matches the VM's exact `cmp_i64_f64_exact` for every magnitude, incl. big ints.
    fn mixed_cmp(
        &mut self,
        op: &str,
        dst: u16,
        int_reg: u16,
        float_reg: u16,
        reversed: bool,
    ) -> Emit {
        let op_code = match op {
            "lt" => 0,
            "gt" => 1,
            "le" => 2,
            "ge" => 3,
            "eq" => 4,
            "ne" => 5,
            _ => return Err(RiscVLowerError::Unsupported("unknown mixed comparison")),
        };
        self.load_slot(int_reg, "a0");
        self.fload_slot(float_reg, "fa0");
        self.line(&format!("li a1, {}", if reversed { 1 } else { 0 }));
        self.line(&format!("li a2, {op_code}"));
        self.line("call logos_cmp_i64_f64_rel");
        self.store_slot(dst, "a0");
        Ok(())
    }

    /// `dst = (lhs <cmp> rhs) ? 1 : 0` for f64 operands (IEEE compare; NaN ⇒ false, per `flt.d`/`fle.d`).
    fn float_cmp(&mut self, cmp: &str, dst: u16, lhs: u16, rhs: u16) {
        self.fload_slot(lhs, "ft0");
        self.fload_slot(rhs, "ft1");
        match cmp {
            "lt" => self.line("flt.d t2, ft0, ft1"),
            "gt" => self.line("flt.d t2, ft1, ft0"),
            "le" => self.line("fle.d t2, ft0, ft1"),
            "ge" => self.line("fle.d t2, ft1, ft0"),
            "eq" => self.line("feq.d t2, ft0, ft1"),
            "ne" => {
                self.line("feq.d t2, ft0, ft1");
                self.line("xori t2, t2, 1");
            }
            _ => unreachable!("unknown float comparison {cmp}"),
        }
        self.store_slot(dst, "t2");
    }

    /// `dst = (lhs is approximately rhs) ? 1 : 0` — the shared isclose:
    /// `a == b` OR `|a-b| <= max(1e-9*max(|a|,|b|), 1e-12)`. f64 operands only.
    fn approx_eq(&mut self, dst: u16, lhs: u16, rhs: u16, kinds: &[Kind]) -> Emit {
        self.inspects(&[lhs, rhs]);
        let (lk, rk) = (self.kind_of(kinds, lhs), self.kind_of(kinds, rhs));
        let number = |k: Kind| matches!(k, Kind::Int | Kind::Float);
        if !(number(lk) && number(rk)) {
            // Anything but two numbers stops the program with the VM's own sentence for the pair.
            let (a, b) = match (self.exemplar(lk), self.exemplar(rk)) {
                (Some(a), Some(b)) => (a, b),
                _ => {
                    return Err(RiscVLowerError::Unsupported(
                        "`is approximately` of values whose kinds are not modeled",
                    ))
                }
            };
            let reason = crate::semantics::arith::approx_eq(a, b)
                .expect_err("only numbers compare approximately");
            let fail = self.fail_stub(&reason);
            self.jump_to_stop(None, &fail);
            return Ok(());
        }
        self.ro.need_approx_consts = true;
        self.load_num_as_float(lhs, lk, "ft0")?; // a
        self.load_num_as_float(rhs, rk, "ft1")?; // b
        self.line("feq.d t2, ft0, ft1"); // exact fast path
        self.line("bnez t2, 1f");
        self.line("fsub.d ft2, ft0, ft1");
        self.line("fabs.d ft2, ft2"); // |a-b|
        self.line("fabs.d ft3, ft0"); // |a|
        self.line("fabs.d ft4, ft1"); // |b|
        self.line("fmax.d ft3, ft3, ft4"); // max(|a|,|b|)
        self.line("fld ft5, .Lapprox_rel, t0"); // 1e-9
        self.line("fmul.d ft3, ft3, ft5"); // rel tol
        self.line("fld ft6, .Lapprox_abs, t0"); // 1e-12
        self.line("fmax.d ft3, ft3, ft6"); // max(rel, abs)
        self.line("fle.d t2, ft2, ft3"); // |a-b| <= tol
        self.line("j 2f");
        self.line("1:");
        self.line("li t2, 1");
        self.line("2:");
        self.store_slot(dst, "t2");
        Ok(())
    }

    /// The scalar numeric kernel builtins (Abs/Sqrt/Floor/Ceil/Round/Min/Max), matching the VM bit-exactly.
    /// Non-numeric builtins (string/uuid/money/complex/…) are refused — a later value-surface pass.
    fn call_builtin(
        &mut self,
        dst: u16,
        builtin: BuiltinId,
        args_start: u16,
        kinds: &[Kind],
    ) -> Emit {
        let a = args_start;
        let ak = self.kind_of(kinds, a);
        match builtin {
            BuiltinId::Sqrt => {
                // Int/Float → Float sqrt.
                self.load_num_as_float(a, ak, "ft0")?;
                self.line("fsqrt.d ft0, ft0");
                self.fstore_slot(dst, "ft0");
            }
            BuiltinId::Abs => match ak {
                Kind::Float => {
                    self.fload_slot(a, "ft0");
                    self.line("fabs.d ft0, ft0");
                    self.fstore_slot(dst, "ft0");
                }
                Kind::Int => {
                    // Wrapping i64 abs: (x ^ (x>>63)) - (x>>63). i64::MIN is the bigint gap.
                    self.load_slot(a, "t0");
                    self.line("srai t1, t0, 63");
                    self.line("xor t2, t0, t1");
                    self.line("sub t2, t2, t1");
                    self.store_slot(dst, "t2");
                }
                _ => return Err(RiscVLowerError::Unsupported("abs of a non-number")),
            },
            BuiltinId::Floor | BuiltinId::Ceil | BuiltinId::Round => match ak {
                Kind::Float => {
                    // `f.floor() as i64` etc. — libm rounds to an integral double; fcvt (rtz) → i64.
                    let f = match builtin {
                        BuiltinId::Floor => "floor",
                        BuiltinId::Ceil => "ceil",
                        _ => "round", // C round() = round-half-away-from-zero, matching Rust f64::round
                    };
                    self.fload_slot(a, "fa0");
                    self.line(&format!("call {f}"));
                    self.line("fcvt.l.d a0, fa0, rtz");
                    self.store_slot(dst, "a0");
                }
                Kind::Int => {
                    // An exact integer is already whole — passthrough.
                    self.load_slot(a, "t0");
                    self.store_slot(dst, "t0");
                }
                _ => {
                    return Err(RiscVLowerError::Unsupported(
                        "floor/ceil/round of a non-number",
                    ))
                }
            },
            BuiltinId::Min | BuiltinId::Max => {
                let b = args_start + 1;
                match self.num_kind(kinds, a, b)? {
                    Kind::Float => {
                        let (ak, bk) = (self.kind_of(kinds, a), self.kind_of(kinds, b));
                        self.load_num_as_float(a, ak, "ft0")?;
                        self.load_num_as_float(b, bk, "ft1")?;
                        let op = if matches!(builtin, BuiltinId::Min) {
                            "fmin.d"
                        } else {
                            "fmax.d"
                        };
                        self.line(&format!("{op} ft2, ft0, ft1"));
                        self.fstore_slot(dst, "ft2");
                    }
                    _ => {
                        self.load_slot(a, "t0");
                        self.load_slot(b, "t1");
                        // min = a<b ? a : b ; max = a>b ? a : b (RV64 base has no int min/max).
                        let cond = if matches!(builtin, BuiltinId::Min) {
                            "blt t0, t1, 1f"
                        } else {
                            "blt t1, t0, 1f"
                        };
                        self.line(cond);
                        self.line("mv t2, t1");
                        self.line("j 2f");
                        self.line("1:");
                        self.line("mv t2, t0");
                        self.line("2:");
                        self.store_slot(dst, "t2");
                    }
                }
            }
            // `parseInt(text)` / `parseFloat(text)` — read as the VM reads them; text that is not a number
            // stops the program with the VM's sentence naming it.
            BuiltinId::ParseInt => {
                self.inspects(&[a]);
                self.load_slot(a, "a0");
                self.line("call logos_parse_int");
                self.store_slot(dst, "a0");
            }
            BuiltinId::ParseFloat => {
                self.inspects(&[a]);
                self.load_slot(a, "a0");
                self.line("call logos_parse_float");
                self.fstore_slot(dst, "fa0");
            }
            BuiltinId::Chr => {
                // `chr(codepoint)` — Int → single-character UTF-8 Text handle.
                self.load_slot(a, "a0");
                self.line("call logos_chr");
                self.store_slot(dst, "a0");
            }
            // env.lg's `get(name)` as the VM expands it: whether the process environment sets the
            // variable, and its text (`""` when unset).
            BuiltinId::EnvHas => {
                self.inspects(&[a]);
                self.load_slot(a, "a0");
                self.line("call logos_env_has");
                self.store_slot(dst, "a0");
            }
            BuiltinId::EnvValue => {
                self.inspects(&[a]);
                self.load_slot(a, "a0");
                self.line("call logos_env_value");
                self.store_slot(dst, "a0");
            }
            // time.lg's `now()` — the Unix time in milliseconds.
            BuiltinId::TimeNow => {
                self.line("call logos_time_now");
                self.store_slot(dst, "a0");
            }
            // `copy(x)` — the VM's `deep_clone`, as `copy of x`.
            BuiltinId::Copy => self.deep_copy(dst, a, kinds)?,
            // `text_bytes(t)` — the Text's UTF-8 bytes, a fresh `Seq of Int`.
            BuiltinId::TextBytes => {
                self.inspects(&[a]);
                self.load_slot(a, "a0");
                self.line("call logos_text_bytes");
                self.store_slot(dst, "a0");
                self.fresh(&[dst]);
            }
            // `readReals(path)` — the file's binary64 values as a Seq of Real, each slot a value's bits.
            BuiltinId::ReadReals => {
                self.inspects(&[a]);
                self.load_slot(a, "a0");
                self.line("call logos_read_reals");
                self.store_slot(dst, "a0");
                self.fresh(&[dst]);
            }
            // `text_from_bytes(bytes)` — the bytes as a Text, validated as UTF-8 exactly as the VM
            // validates them; invalid bytes stop the program with the VM's own sentence.
            BuiltinId::TextFromBytes => {
                self.inspects(&[a]);
                self.load_slot(a, "a0");
                self.line("call logos_text_from_bytes");
                self.store_slot(dst, "a0");
            }
            // `Write <content> to file <path>` / `Read … from file <path>` / `Mount … at <path>` — the VM's
            // sandboxed in-memory VFS, which keeps files only inside a `## Test`'s sealed world. A program keeps
            // none: a write leaves nothing, a mount finds an empty journal, and a read stops the program with
            // the VM's own sentence for a missing file.
            BuiltinId::VfsWriteFile => self.store_slot(dst, "zero"),
            BuiltinId::VfsMount => {
                let empty = self.ro.intern_text("");
                self.line(&format!("la t0, {empty}"));
                self.store_slot(dst, "t0");
            }
            BuiltinId::VfsReadFile => {
                self.inspects(&[a]);
                let tag = kind_tag(ak)?;
                let missing = self.ro.intern_text(vfs_read_refusal());
                self.load_slot(a, "a0");
                self.line(&format!("li a1, {tag}"));
                self.line("call logos_val_to_str");
                self.line(&format!("la a1, {missing}"));
                self.line("call logos_vfs_read");
            }
            // `randomInt(lo, hi)` — a uniform draw from `lo..=hi` from the system's entropy, as the VM draws
            // outside a sealed test world; crossed bounds stop the program with the VM's sentence.
            BuiltinId::RandomInt => {
                self.load_slot(a, "a0");
                self.load_slot(a + 1, "a1");
                self.line("call logos_random_int");
                self.store_slot(dst, "a0");
            }
            // `format(x)` — the value's text exactly as `Show` renders it (`logos_val_to_str`).
            BuiltinId::Format => {
                self.inspects(&[a]);
                let tag = kind_tag(ak)?;
                self.load_slot(a, "a0");
                self.line(&format!("li a1, {tag}"));
                self.line("call logos_val_to_str");
                self.store_slot(dst, "a0");
            }
            // `Fail with <value>.` — its rendering on stderr, exit 1 (`logos_fail_val`).
            BuiltinId::Fail => {
                let tag = kind_tag(ak)?;
                self.load_slot(a, "a0");
                self.line(&format!("li a1, {tag}"));
                self.line("call logos_fail_val");
            }
            // A broken contract's `, otherwise <reason>`: the Text, or the refusal the VM gives a reason of
            // another kind (`contract_reason_text`).
            BuiltinId::FailReason => {
                if ak == Kind::Text {
                    self.load_slot(a, "a0");
                } else {
                    let value = self.exemplar(ak).ok_or(RiscVLowerError::Unsupported(
                        "a contract reason whose kind is not modeled",
                    ))?;
                    let refusal = crate::semantics::builtins::contract_reason_text(value)
                        .expect_err("only a Text is a reason");
                    let refusal = self.ro.intern_text(&refusal);
                    self.line(&format!("la a0, {refusal}"));
                }
                self.line("call logos_fatal");
            }
            // ── Word32 (ℤ/2³²) — values ride an i64 slot zero-extended to the low 32 bits. ──
            BuiltinId::Word32 => {
                // `word32(n)` = n as u32, held zero-extended (mask to the low 32 bits).
                self.load_slot(a, "t0");
                self.mask_word32("t0");
                self.store_slot(dst, "t0");
            }
            BuiltinId::IntOfWord32 => {
                // `intOfWord32(w)` = the unsigned value 0..2³²−1 — already how a Word32 is stored.
                self.load_slot(a, "t0");
                self.store_slot(dst, "t0");
            }
            BuiltinId::Wand | BuiltinId::Wor => {
                // `word_and`/`word_or` — bitwise on two 32-bit values (result stays in 32 bits).
                let b = args_start + 1;
                let op = if matches!(builtin, BuiltinId::Wand) {
                    "and"
                } else {
                    "or"
                };
                self.load_slot(a, "t0");
                self.load_slot(b, "t1");
                self.line(&format!("{op} t2, t0, t1"));
                self.store_slot(dst, "t2");
            }
            BuiltinId::Wnot => {
                // `word_not(x)` = ~x masked to 32 bits.
                self.load_slot(a, "t0");
                self.line("not t0, t0");
                self.mask_word32("t0");
                self.store_slot(dst, "t0");
            }
            BuiltinId::Word32Shr => {
                // `word32Shr(w, n)` = w >>u (n & 31) (logical), matching `u32::wrapping_shr`.
                let b = args_start + 1;
                self.load_slot(a, "t0");
                self.load_slot(b, "t1");
                self.line("andi t1, t1, 31");
                self.line("srl t0, t0, t1"); // w is zero-extended → logical shift is exact
                self.store_slot(dst, "t0");
            }
            BuiltinId::Rotr => {
                // `rotr(w, n)` — 32-bit rotate right by (n & 31): (w >> n) | (w << (32 - n)), masked.
                let b = args_start + 1;
                self.load_slot(a, "t0");
                self.load_slot(b, "t1");
                self.line("andi t1, t1, 31");
                self.line("srl t2, t0, t1"); // w >> n
                self.line("li t3, 32");
                self.line("sub t3, t3, t1"); // 32 - n
                self.line("sll t4, t0, t3"); // w << (32 - n)
                self.line("or t2, t2, t4");
                self.mask_word32("t2");
                self.store_slot(dst, "t2");
            }
            BuiltinId::Rotl => {
                // `rotl(w, n)` — 32-bit rotate left by (n & 31): (w << n) | (w >> (32 - n)), masked.
                let b = args_start + 1;
                self.load_slot(a, "t0");
                self.load_slot(b, "t1");
                self.line("andi t1, t1, 31");
                self.line("sll t2, t0, t1"); // w << n
                self.line("li t3, 32");
                self.line("sub t3, t3, t1"); // 32 - n
                self.line("srl t4, t0, t3"); // w >> (32 - n)
                self.line("or t2, t2, t4");
                self.mask_word32("t2");
                self.store_slot(dst, "t2");
            }
            _ => return Err(RiscVLowerError::Unsupported("non-numeric kernel builtin")),
        }
        Ok(())
    }

    /// `dst = lhs <shift> rhs` (Int) — RV64 masks the count to 6 bits (== the VM's masked wrapping shift).
    fn int_shift(&mut self, rv: &str, dst: u16, lhs: u16, rhs: u16, kinds: &[Kind]) -> Emit {
        self.require(kinds, &[lhs, rhs], Kind::Int, "shift on non-Int")?;
        self.load_slot(lhs, "t0");
        self.load_slot(rhs, "t1");
        self.line(&format!("{rv} t2, t0, t1"));
        self.store_slot(dst, "t2");
        Ok(())
    }

    /// `dst = lhs ** rhs`. Int^Int via the squaring shim (negative exp errors); a Float operand promotes
    /// to f64 and calls libm `pow`.
    fn pow(&mut self, dst: u16, lhs: u16, rhs: u16, kinds: &[Kind]) -> Emit {
        match self.num_kind(kinds, lhs, rhs)? {
            Kind::Float => {
                let (lk, rk) = (self.kind_of(kinds, lhs), self.kind_of(kinds, rhs));
                self.load_num_as_float(lhs, lk, "fa0")?;
                self.load_num_as_float(rhs, rk, "fa1")?;
                self.line("call pow");
                self.fstore_slot(dst, "fa0");
            }
            _ => {
                self.load_slot(lhs, "a0");
                self.load_slot(rhs, "a1");
                self.line("call logos_ipow");
                self.store_slot(dst, "a0");
            }
        }
        Ok(())
    }

    /// `dst = lhs <op> rhs` bitwise (Int or Bool operands — Bool is 0/1, so and/or/xor is logical).
    /// Set/collection `&`/`|` are refused here (heap, Phase 3).
    fn int_bitop(&mut self, rv: &str, dst: u16, lhs: u16, rhs: u16, kinds: &[Kind]) -> Emit {
        // Int/Bool bitwise, or Word32 `xor`/`and`/`or` (both operands zero-extended 32-bit → result
        // stays in 32 bits, so no extra masking is needed).
        if !self.all_kind(kinds, &[lhs, rhs], &[Kind::Int, Kind::Bool, Kind::Word32]) {
            return Err(RiscVLowerError::Unsupported("bitwise op on non-Int/Bool"));
        }
        self.load_slot(lhs, "t0");
        self.load_slot(rhs, "t1");
        self.line(&format!("{rv} t2, t0, t1"));
        self.store_slot(dst, "t2");
        Ok(())
    }

    /// Signed `div`/`rem` with an explicit divisor zero-check (RV64 does not trap on /0; the VM errors).
    fn int_divmod(&mut self, rv: &str, dst: u16, lhs: u16, rhs: u16, kinds: &[Kind]) -> Emit {
        self.require(kinds, &[lhs, rhs], Kind::Int, "integer division on non-Int")?;
        let fail = self.fail_stub(if rv == "rem" {
            &refusals().modulo
        } else {
            &refusals().division
        });
        self.load_slot(lhs, "t0");
        self.load_slot(rhs, "t1");
        self.jump_to_stop(Some(("beqz", "t1")), &fail);
        self.line(&format!("{rv} t2, t0, t1"));
        self.store_slot(dst, "t2");
        Ok(())
    }

    /// Floor division (toward −∞): truncating quotient, corrected by one on a sign-crossing nonzero
    /// remainder — bit-exact with the VM's `FloorDiv` for in-i64 operands.
    fn floor_div(&mut self, dst: u16, lhs: u16, rhs: u16, kinds: &[Kind]) -> Emit {
        self.require(kinds, &[lhs, rhs], Kind::Int, "floor division on non-Int")?;
        let fail = self.fail_stub(&refusals().floor_division);
        self.load_slot(lhs, "t0");
        self.load_slot(rhs, "t1");
        self.jump_to_stop(Some(("beqz", "t1")), &fail);
        self.line("div t2, t0, t1"); // trunc quotient
        self.line("rem t3, t0, t1"); // remainder, sign of dividend
        self.line("xor t4, t3, t1"); // sign(rem) != sign(divisor) ?
        self.line("slti t4, t4, 0");
        self.line("snez t5, t3"); // rem != 0 ?
        self.line("and t5, t5, t4");
        self.line("sub t2, t2, t5"); // q -= correction
        self.store_slot(dst, "t2");
        Ok(())
    }

    /// The label of the stub that stops the program with `reason`, one per distinct reason.
    fn fail_stub(&mut self, reason: &str) -> String {
        let i = match self.fail_stubs.iter().position(|r| r == reason) {
            Some(i) => i,
            None => {
                self.fail_stubs.push(reason.to_string());
                self.fail_stubs.len() - 1
            }
        };
        format!(".Lfail_{i}")
    }

    /// Jump to `stub`, a stub that stops the program (`.Lfail_<i>`, `.Lnothing`), when `when` holds — a branch and the
    /// register it tests (`("bnez", "t0")`) — or always. The stubs sit after the whole program, beyond a `jal`'s ±1 MiB
    /// from a check early in a large one, so the jump is `jump`'s `auipc`/`jalr` pair (±2 GiB) behind the inverted
    /// branch; the stub never returns, so `t6` carries its address.
    fn jump_to_stop(&mut self, when: Option<(&str, &str)>, stub: &str) {
        let Some((branch, reg)) = when else {
            self.line(&format!("jump {stub}, t6"));
            return;
        };
        let inverse = match branch {
            "bnez" => "beqz",
            "beqz" => "bnez",
            "bltz" => "bgez",
            other => unreachable!("a stop's branch is bnez, beqz or bltz, not {other}"),
        };
        self.line(&format!("{inverse} {reg}, 1f"));
        self.line(&format!("jump {stub}, t6"));
        self.out.push_str("1:\n");
    }

    /// `dst = lhs / 2^k` (signed, round toward zero) — the Oracle emits this only when `lhs` is Int.
    fn div_pow2(&mut self, dst: u16, lhs: u16, k: u8, kinds: &[Kind]) -> Emit {
        self.require(kinds, &[lhs], Kind::Int, "DivPow2 on non-Int")?;
        self.load_slot(lhs, "t0");
        self.line("srai t2, t0, 63"); // all-ones mask if negative
        self.line(&format!("li t3, {}", (1i64 << k) - 1));
        self.line("and t2, t2, t3"); // bias for negative dividends
        self.line("add t2, t0, t2");
        self.line(&format!("srai t2, t2, {k}"));
        self.store_slot(dst, "t2");
        Ok(())
    }

    /// `dst = (lhs <cmp> rhs) ? 1 : 0` for Int, for two moments or two durations (nanoseconds) and for two dates (days),
    /// each an i64 slot; Eq/NotEq also accept Bool operands.
    fn int_cmp(&mut self, cmp: &str, dst: u16, lhs: u16, rhs: u16, kinds: &[Kind]) -> Emit {
        let ok = (matches!(cmp, "eq" | "ne") && self.all_kind(kinds, &[lhs, rhs], &[Kind::Int, Kind::Bool]))
            || self.all_kind(kinds, &[lhs, rhs], &[Kind::Moment])
            || self.all_kind(kinds, &[lhs, rhs], &[Kind::Duration])
            || self.all_kind(kinds, &[lhs, rhs], &[Kind::Date]);
        if !ok {
            self.require(kinds, &[lhs, rhs], Kind::Int, "comparison on non-Int")?;
        }
        self.load_slot(lhs, "t0");
        self.load_slot(rhs, "t1");
        match cmp {
            "lt" => self.line("slt t2, t0, t1"),
            "gt" => self.line("slt t2, t1, t0"),
            "le" => {
                self.line("slt t2, t1, t0");
                self.line("xori t2, t2, 1");
            }
            "ge" => {
                self.line("slt t2, t0, t1");
                self.line("xori t2, t2, 1");
            }
            "eq" => {
                self.line("sub t2, t0, t1");
                self.line("seqz t2, t2");
            }
            "ne" => {
                self.line("sub t2, t0, t1");
                self.line("snez t2, t2");
            }
            _ => unreachable!("unknown comparison {cmp}"),
        }
        self.store_slot(dst, "t2");
        Ok(())
    }

    /// Emit `Show` of a struct: `Name { a: .., b: .. }` with fields SORTED BY NAME (matching the VM's
    /// deterministic order), each value formatted by its field kind; an empty struct shows as its name.
    fn show_struct(&mut self, src: u16, name: &str) -> Emit {
        let mut fs: Vec<(String, usize, Kind)> = {
            let sd = self
                .prog
                .struct_types
                .iter()
                .find(|s| s.name == name)
                .ok_or(RiscVLowerError::Unsupported("Show of unknown struct type"))?;
            sd.fields
                .iter()
                .enumerate()
                .map(|(i, (n, t))| (n.clone(), i, self.boundary_kind(&Some(t.clone()))))
                .collect()
        };
        fs.sort_by(|a, b| a.0.cmp(&b.0));
        if fs.is_empty() {
            let lbl = self.ro.intern_text(name);
            self.line(&format!("la a0, {lbl}"));
            self.line("call logos_show_text");
            return Ok(());
        }
        let pre = self.ro.intern_text(&format!("{name} {{ "));
        self.line(&format!("la a0, {pre}"));
        self.line("call logos_print_raw");
        for (i, (fname, decl, kind)) in fs.iter().enumerate() {
            let tag = kind_tag(*kind)?;
            if i > 0 {
                let sep = self.ro.intern_text(", ");
                self.line(&format!("la a0, {sep}"));
                self.line("call logos_print_raw");
            }
            let flbl = self.ro.intern_text(&format!("{fname}: "));
            self.line(&format!("la a0, {flbl}"));
            self.line("call logos_print_raw");
            self.load_slot(src, "a0");
            self.line(&format!("li a1, {}", decl + 1));
            self.line("call logos_seq_index"); // a0 = field value bits
            self.line(&format!("li a1, {tag}"));
            self.line("call logos_val_to_str"); // a0 = value string
            self.line("call logos_print_raw");
        }
        let suf = self.ro.intern_text(" }");
        self.line(&format!("la a0, {suf}"));
        self.line("call logos_print_raw");
        self.line("call logos_print_nl");
        Ok(())
    }

    // ---- frame / slot / prologue helpers -----------------------------------

    fn emit_epilogue(&mut self, frame: i64) {
        for (j, p) in self.inout.clone().into_iter().enumerate() {
            self.load_slot(p, &format!("a{}", 2 + j));
        }
        self.restore_ra(frame);
        self.adjust_sp(frame);
        self.line("ret");
    }

    fn emit_return_nothing(&mut self, frame: i64) {
        self.line("li a0, 0");
        if self.reports_nothing {
            self.line("li a1, 1");
        }
        self.emit_epilogue(frame);
    }

    /// `sp += delta` (delta is negative in the prologue), with a large-frame fallback.
    fn adjust_sp(&mut self, delta: i64) {
        if fits_imm12(delta) {
            self.line(&format!("addi sp, sp, {delta}"));
        } else {
            self.line(&format!("li {ADDR_TMP}, {delta}"));
            self.line(&format!("add sp, sp, {ADDR_TMP}"));
        }
    }

    fn save_ra(&mut self, frame: i64) {
        self.store_at_sp(frame - 8, "ra");
    }

    fn restore_ra(&mut self, frame: i64) {
        self.load_at_sp(frame - 8, "ra");
    }

    fn load_slot(&mut self, reg: u16, into: &str) {
        let off = self.presented.get(&reg).copied().unwrap_or(8 * reg as i64);
        self.load_at_sp(off, into);
    }

    fn store_slot(&mut self, reg: u16, from: &str) {
        self.store_at_sp(8 * reg as i64, from);
    }

    /// The address of slot `reg`, into `into`.
    /// `insn value, off(base)` — a load or store of the cell `off` bytes into a collection's data, through
    /// `t5` when the offset does not fit an immediate.
    fn cell_access(&mut self, insn: &str, value: &str, base: &str, off: i64) {
        if fits_imm12(off) {
            self.line(&format!("{insn} {value}, {off}({base})"));
        } else {
            self.line(&format!("li t5, {off}"));
            self.line(&format!("add t5, {base}, t5"));
            self.line(&format!("{insn} {value}, 0(t5)"));
        }
    }

    fn slot_addr(&mut self, reg: u16, into: &str) {
        let off = 8 * reg as i64;
        if fits_imm12(off) {
            self.line(&format!("addi {into}, sp, {off}"));
        } else {
            self.line(&format!("li {into}, {off}"));
            self.line(&format!("add {into}, sp, {into}"));
        }
    }

    fn load_at_sp(&mut self, off: i64, into: &str) {
        if fits_imm12(off) {
            self.line(&format!("ld {into}, {off}(sp)"));
        } else {
            self.line(&format!("li {ADDR_TMP}, {off}"));
            self.line(&format!("add {ADDR_TMP}, sp, {ADDR_TMP}"));
            self.line(&format!("ld {into}, 0({ADDR_TMP})"));
        }
    }

    fn store_at_sp(&mut self, off: i64, from: &str) {
        if fits_imm12(off) {
            self.line(&format!("sd {from}, {off}(sp)"));
        } else {
            self.line(&format!("li {ADDR_TMP}, {off}"));
            self.line(&format!("add {ADDR_TMP}, sp, {ADDR_TMP}"));
            self.line(&format!("sd {from}, 0({ADDR_TMP})"));
        }
    }

    /// Load a register slot into an FP register (`fld`). Slots hold raw f64 bit patterns.
    fn fload_slot(&mut self, reg: u16, into: &str) {
        let off = self.presented.get(&reg).copied().unwrap_or(8 * reg as i64);
        if fits_imm12(off) {
            self.line(&format!("fld {into}, {off}(sp)"));
        } else {
            self.line(&format!("li {ADDR_TMP}, {off}"));
            self.line(&format!("add {ADDR_TMP}, sp, {ADDR_TMP}"));
            self.line(&format!("fld {into}, 0({ADDR_TMP})"));
        }
    }

    /// Store an FP register into a register slot (`fsd`).
    fn fstore_slot(&mut self, reg: u16, from: &str) {
        let off = 8 * reg as i64;
        if fits_imm12(off) {
            self.line(&format!("fsd {from}, {off}(sp)"));
        } else {
            self.line(&format!("li {ADDR_TMP}, {off}"));
            self.line(&format!("add {ADDR_TMP}, sp, {ADDR_TMP}"));
            self.line(&format!("fsd {from}, 0({ADDR_TMP})"));
        }
    }

    fn line(&mut self, s: &str) {
        self.out.push_str("  ");
        self.out.push_str(s);
        self.out.push('\n');
    }

    // ---- kind inference ----------------------------------------------------

    /// The registers `op` writes: its own, and for a call each argument register a `mutable` parameter hands its
    /// final value back into ([`Emitter::inout_params`]).
    fn for_each_written(&self, op: &Op, mut f: impl FnMut(u16)) {
        op.for_each_written_reg(&mut f);
        if let Op::Call {
            func, args_start, ..
        } = *op
        {
            for p in self.inout_params(func as usize) {
                f(args_start + p);
            }
        }
    }

    /// The `mutable` parameters of function `fi`, in order: what each holds when the function returns is the
    /// caller's, which the VM keeps in the argument register for the caller's copy-out. The function hands them back
    /// in `a2`, `a3`, … beside its value, and the call stores them into its argument registers. A parameter the
    /// compiler only inferred a mutable borrow for (`cow_exempt_regs`) has no copy-out, so nothing hands it back.
    fn inout_params(&self, fi: usize) -> Vec<u16> {
        let f = &self.prog.functions[fi];
        let mut params: Vec<u16> = f
            .mutable_param_regs
            .iter()
            .copied()
            .filter(|p| *p < f.param_count && !f.cow_exempt_regs.contains(p))
            .collect();
        params.sort_unstable();
        params.dedup();
        params
    }

    fn kind_of(&self, kinds: &[Kind], reg: u16) -> Kind {
        kinds.get(reg as usize).copied().unwrap_or(Kind::Unknown)
    }

    fn all_kind(&self, kinds: &[Kind], regs: &[u16], allowed: &[Kind]) -> bool {
        regs.iter()
            .all(|r| allowed.contains(&self.kind_of(kinds, *r)))
    }

    fn require(&self, kinds: &[Kind], regs: &[u16], want: Kind, why: &'static str) -> Emit {
        if regs.iter().all(|r| self.kind_of(kinds, *r) == want) {
            Ok(())
        } else {
            Err(RiscVLowerError::Unsupported(why))
        }
    }

    /// FLOW-SENSITIVE kind inference: the register kinds BEFORE each op (indexed by `pc - r.start`). A
    /// single forward pass with OVERWRITE-on-definition (reaching-definition semantics). Flow sensitivity
    /// is ESSENTIAL: the register allocator reuses a slot for values of different kinds once the prior
    /// value dies (e.g. a `Seq` slot later reused for an `Int` literal), so a single kind-per-register map
    /// would conflate them and wrongly refuse a later `Show`. LOGOS is statically typed, so a given
    /// definition has one kind; at a use we take the most recent definition's kind (correct for the
    /// structured control flow the compiler emits — a var's kind is stable across a loop).
    ///
    /// In a program the exact extension reaches, a variable holds the extension's value at every point if it does at
    /// any: the region's exact webs ([`crate::vm::tower::webs`]) grow with what holds one until they hold still, each
    /// pass's kinds deciding the webs, and every definition of an exact web landing as the extension's value in the
    /// next. Each such definition that computes another kind is kept with that kind ([`Emitter::tower_from`]), and
    /// each register the region takes in as another kind and holds as the extension's value with the kind it
    /// arrives as ([`Emitter::tower_entries`]).
    fn infer_kinds(&self, r: &Region) -> std::collections::HashMap<usize, Vec<Kind>> {
        let entries = (r.params as usize + 2 * r.captures.len()).min(r.frame_regs) as u16;
        let mut forced = crate::vm::tower::Webs::default();
        let mut landings = NumberLandings::default();
        for round in 0.. {
            let inferred = self.infer_kinds_forced(r, &forced, &landings.defs, &landings.entries);
            let next = if self.tower {
                crate::vm::tower::webs(
                    &self.prog.code,
                    &RegionFlow { emitter: self, region: r },
                    entries,
                    |reg| forced.entries.contains(&reg) || self.entry_kind(r, reg) == Kind::Tower,
                    |pc, _| inferred.lands.contains(&pc),
                )
            } else {
                crate::vm::tower::Webs::default()
            };
            let mut next_landings = self.number_landings(r, entries, &inferred);
            if round == SETTLE_ROUNDS {
                next_landings = landings.clone();
                next_landings.refused.push(crate::vm::variables::Site::Op(r.pcs().first().copied().unwrap_or(0)));
            }
            if next == forced && next_landings == landings {
                let pcs = r.pcs();
                let mut from = self.tower_from.borrow_mut();
                let mut landing = self.tower_lands.borrow_mut();
                let mut numbers = self.number_from.borrow_mut();
                for pc in &pcs {
                    from.remove(pc);
                    landing.remove(pc);
                    numbers.remove(pc);
                }
                from.extend(inferred.natural);
                landing.extend(inferred.lands);
                numbers.extend(
                    inferred
                        .number_natural
                        .iter()
                        .map(|(pc, k)| (*pc, (*k, landings.defs[pc]))),
                );
                let converted = forced
                    .entries
                    .iter()
                    .map(|reg| (*reg, self.entry_kind(r, *reg)))
                    .filter(|(_, k)| *k != Kind::Tower)
                    .collect();
                let widened = landings
                    .entries
                    .iter()
                    .filter(|(reg, _)| !forced.entries.contains(reg))
                    .map(|(reg, k)| (*reg, self.entry_kind(r, *reg), *k))
                    .collect();
                if let Some(&entry) = pcs.first() {
                    self.tower_entries.borrow_mut().insert(entry, converted);
                    self.number_entries.borrow_mut().insert(entry, (widened, landings.refused.clone()));
                }
                return inferred.per_pc;
            }
            forced = next;
            landings = next_landings;
        }
        unreachable!("the rounds end when the webs and joins hold still")
    }

    /// The number form of a kind: an integer, a rational, a real, or a list of one of them.
    fn number_form(&self, k: Kind) -> Option<crate::vm::numbers::Form> {
        use crate::vm::numbers::{Form, Number};
        let number = |k: Kind| match k {
            Kind::Int => Some(Number::Int),
            Kind::Rational => Some(Number::Rational),
            Kind::Float => Some(Number::Real),
            _ => None,
        };
        match k {
            Kind::SeqInt => Some(Form::List(Number::Int)),
            Kind::SeqFloat => Some(Form::List(Number::Real)),
            other => match self.as_composite(other) {
                Some(Composite::Seq(e)) => number(e).map(Form::List),
                _ => number(other).map(Form::Number),
            },
        }
    }

    /// The kind that holds number form `f`.
    fn number_kind(&self, f: crate::vm::numbers::Form) -> Kind {
        use crate::vm::numbers::{Form, Number};
        match f {
            Form::Number(Number::Int | Number::BigInt) => Kind::Int,
            Form::Number(Number::Rational) => Kind::Rational,
            Form::Number(Number::Real) => Kind::Float,
            Form::List(Number::Int | Number::BigInt) => Kind::SeqInt,
            Form::List(Number::Real) => Kind::SeqFloat,
            Form::List(Number::Rational) => self.seq_of_kind(Kind::Rational),
        }
    }

    /// The number joins of region `r` ([`crate::vm::numbers`]) under one round's kinds: what each definition a join
    /// lands lands as, what each register taken in is converted to, and the variables no machine form holds.
    fn number_landings(&self, r: &Region, entries: u16, inferred: &Inferred) -> NumberLandings {
        let j = crate::vm::numbers::joined(
            &self.prog.code,
            &self.prog.constants,
            &RegionFlow { emitter: self, region: r },
            entries,
            |site| match site {
                crate::vm::variables::Site::Entry(reg) => self.number_form(self.entry_kind(r, reg)),
                crate::vm::variables::Site::Op(pc) => {
                    let k = inferred.number_natural.get(&pc).or_else(|| inferred.written.get(&pc))?;
                    self.number_form(*k)
                }
            },
            |pc, reg| inferred.per_pc.get(&pc).is_some_and(|kinds| kind_at(kinds, reg) == Kind::Float),
        );
        NumberLandings {
            defs: j.lands.into_iter().map(|(pc, f)| (pc, self.number_kind(f))).collect(),
            entries: j.entries.into_iter().map(|(reg, f)| (reg, self.number_kind(f))).collect(),
            refused: j.refused,
        }
    }

    /// Register `reg`, of kind `from`, converted in its slot to number kind `to` as a number join lands it.
    fn land_number(&mut self, reg: u16, from: Kind, to: Kind) -> Emit {
        match (from, to) {
            (Kind::Int | Kind::Bool, Kind::Rational) => {
                self.load_slot(reg, "a0");
                self.line("call logos_ldec_of_int");
                self.store_slot(reg, "a0");
            }
            (Kind::Int | Kind::Bool, Kind::Float) => {
                self.load_slot(reg, "t0");
                self.line("fcvt.d.l ft0, t0");
                self.fstore_slot(reg, "ft0");
            }
            (Kind::SeqInt, _) if matches!(self.as_composite(to), Some(Composite::Seq(Kind::Rational))) => {
                self.load_slot(reg, "a0");
                self.line("call logos_seq_as_ldec");
                self.store_slot(reg, "a0");
                self.fresh(&[reg]);
            }
            _ => return Err(RiscVLowerError::Unsupported("a number landing between these kinds")),
        }
        Ok(())
    }

    /// The kind register `reg` of region `r` arrives as: a parameter's, a captured value's, a captured value's
    /// present flag's.
    fn entry_kind(&self, r: &Region, reg: u16) -> Kind {
        let i = reg as usize;
        let (params, cap) = (r.params as usize, r.captures.len());
        if i < params {
            r.param_kinds.get(i).copied().unwrap_or(Kind::Unknown)
        } else if i < params + cap {
            r.captures[i - params]
        } else if i < params + 2 * cap {
            Kind::Bool
        } else {
            Kind::Unknown
        }
    }

    /// Whether a value of kind `k` stores a value of the extension under a key, keeping its own kind: a map or a
    /// record.
    fn keyed(&self, k: Kind) -> bool {
        k == Kind::MapUnknown || self.is_map(k) || matches!(self.as_composite(k), Some(Composite::Struct(_)))
    }

    /// [`Emitter::infer_kinds`] with the definitions of `forced`'s exact webs landing as the extension's value, and
    /// `forced`'s entries arriving as one. Answers the kinds before each op, the pcs whose write lands as the
    /// extension's value, and each forced definition that computes another kind, with that kind.
    #[allow(clippy::type_complexity)]
    fn infer_kinds_forced(
        &self,
        r: &Region,
        forced: &crate::vm::tower::Webs,
        number_lands: &std::collections::BTreeMap<usize, Kind>,
        number_entries: &std::collections::BTreeMap<u16, Kind>,
    ) -> Inferred {
        let nregs = r.frame_regs.max(1);
        let mut cur = vec![Kind::Unknown; nregs];
        for (i, k) in r.param_kinds.iter().enumerate() {
            if i < nregs {
                cur[i] = *k;
            }
        }
        // Closure body: the captured values (slots params..params+cap) and the present flags (Bool, the
        // next cap slots) are seeded — the body reads them like any local, so they need concrete kinds.
        let cap = r.captures.len();
        for (k, ck) in r.captures.iter().enumerate() {
            let v = r.params as usize + k;
            let p = r.params as usize + cap + k;
            if v < nregs {
                cur[v] = *ck;
            }
            if p < nregs {
                cur[p] = Kind::Bool;
            }
        }
        for (reg, k) in number_entries {
            if (*reg as usize) < nregs {
                cur[*reg as usize] = *k;
            }
        }
        for reg in &forced.entries {
            if (*reg as usize) < nregs {
                cur[*reg as usize] = Kind::Tower;
            }
        }
        let mut lands = std::collections::BTreeSet::new();
        let mut natural = std::collections::HashMap::new();
        let mut written: std::collections::HashMap<usize, Kind> = std::collections::HashMap::new();
        let mut number_natural: std::collections::HashMap<usize, Kind> = std::collections::HashMap::new();
        let mut per_pc: std::collections::HashMap<usize, Vec<Kind>> =
            std::collections::HashMap::new();
        // A monotone forward fixpoint (bounded): a collection created empty (`SeqUnknown`/`SetUnknown`/
        // `MapUnknown`) is refined only by a later `Push`/`SetAdd`/`SetIndex`. When that refinement is
        // loop-carried (a `contains`/`item … of` reads the collection BEFORE the first insert in program
        // order), a single pass would leave the read `Unknown`. Each pass seeds a slot's creation with the
        // concrete collection kind it reached in the previous pass, until the slot kinds stop changing.
        let seed0 = cur.clone();
        let mut overrides = vec![Kind::Unknown; nregs];
        for _pass in 0..8 {
            cur = seed0.clone();
            per_pc.clear();
            lands.clear();
            natural.clear();
            written.clear();
            number_natural.clear();
            // The element kind of each in-flight iterator, so `IterNext`'s bound var gets the right kind.
            let mut iter_elems: Vec<Kind> = Vec::new();
            // The most recent `TestArm` (target, constructor name) — the arm a following `BindArm` extracts
            // from, giving its payload the right kind (the Inspect true-branch follows its TestArm).
            let mut last_test: Option<(u16, String)> = None;
            let pcs = r.pcs();
            for (i, &pc) in pcs.iter().enumerate() {
                per_pc.insert(pc, cur.clone()); // the kinds BEFORE this op
                if r.never.dead.contains(&pc) {
                    continue;
                }
                // RevSwap exchanges two registers (and their kinds) — handle out of band.
                if let Op::RevSwap { a, b } = self.prog.code[pc] {
                    let (ai, bi) = (a as usize, b as usize);
                    if ai < cur.len() && bi < cur.len() {
                        cur.swap(ai, bi);
                    }
                    continue;
                }
                // DestructureTuple writes MULTIPLE registers (one per position) — handle it out of band.
                if let Op::DestructureTuple { src, start, count } = self.prog.code[pc] {
                    if let Some(Composite::Tuple(positions)) = self.as_composite(kind_at(&cur, src))
                    {
                        for i in 0..(count as usize).min(positions.len()) {
                            let idx = start as usize + i;
                            if idx < cur.len() {
                                cur[idx] = positions[i];
                            }
                        }
                    }
                    continue;
                }
                let write: Option<(u16, Kind)> = match self.prog.code[pc] {
                    Op::LoadConst { dst, idx } => {
                        let k = match self.prog.constants[idx as usize] {
                            Constant::Int(_) => Kind::Int,
                            Constant::Duration(_) => Kind::Duration,
                            Constant::Bool(_) => Kind::Bool,
                            Constant::Float(_) => Kind::Float,
                            Constant::Text(_) => Kind::Text,
                            Constant::Date(_) => Kind::Date,
                            Constant::Moment(_) => Kind::Moment,
                            Constant::Span { .. } => Kind::Span,
                            _ => Kind::Unknown,
                        };
                        Some((dst, k))
                    }
                    Op::Move { dst, src } => Some((dst, kind_at(&cur, src))),
                    Op::Add { dst, lhs, rhs } => {
                        let (l, r) = (kind_at(&cur, lhs), kind_at(&cur, rhs));
                        Some((dst, temporal_kind("add", l, r).unwrap_or_else(|| join(l, r))))
                    }
                    Op::Sub { dst, lhs, rhs } => {
                        let (l, r) = (kind_at(&cur, lhs), kind_at(&cur, rhs));
                        Some((dst, temporal_kind("sub", l, r).unwrap_or_else(|| join(l, r))))
                    }
                    Op::Mul { dst, lhs, rhs }
                    | Op::Div { dst, lhs, rhs }
                    | Op::Mod { dst, lhs, rhs }
                    | Op::FloorDiv { dst, lhs, rhs }
                    | Op::Pow { dst, lhs, rhs } => {
                        Some((dst, join(kind_at(&cur, lhs), kind_at(&cur, rhs))))
                    }
                    // A bitwise result over a rational is the integer two whole numbers make.
                    Op::BitXor { dst, lhs, rhs }
                    | Op::BitAnd { dst, lhs, rhs }
                    | Op::BitOr { dst, lhs, rhs }
                    | Op::Shl { dst, lhs, rhs }
                    | Op::Shr { dst, lhs, rhs } => Some((
                        dst,
                        match join(kind_at(&cur, lhs), kind_at(&cur, rhs)) {
                            Kind::Rational => Kind::Int,
                            joined => joined,
                        },
                    )),
                    Op::AddAssign { dst, src } => {
                        let (l, r) = (kind_at(&cur, dst), kind_at(&cur, src));
                        Some((dst, temporal_kind("add", l, r).unwrap_or_else(|| join(l, r))))
                    }
                    // `exact_divide` → an exact Rational, unless a Float operand makes it float division.
                    Op::ExactDiv { dst, lhs, rhs } => {
                        let (lk, rk) = (kind_at(&cur, lhs), kind_at(&cur, rhs));
                        let k = if lk == Kind::Float || rk == Kind::Float {
                            Kind::Float
                        } else {
                            Kind::Rational
                        };
                        Some((dst, k))
                    }
                    // A division by a constant of an integer a number join holds as a rational is that exact
                    // number's quotient or remainder.
                    Op::DivPow2 { dst, lhs, .. } | Op::MagicDivU { dst, lhs, .. }
                        if kind_at(&cur, lhs) == Kind::Rational =>
                    {
                        Some((dst, Kind::Rational))
                    }
                    Op::DivPow2 { dst, .. } | Op::MagicDivU { dst, .. } => Some((dst, Kind::Int)),
                    // Reversible primitives write an Int `place` (RevSwap's two-register swap is out-of-band).
                    Op::RevXor { place, .. }
                    | Op::RevNeg { place }
                    | Op::RevAdd { place, .. }
                    | Op::RevSub { place, .. } => Some((place, Kind::Int)),
                    // BigInt-lane: kind 3 (Cmp) yields a Bool, every other kind an Int.
                    Op::BigIntWide { dst, kind, .. } => {
                        Some((dst, if kind == 3 { Kind::Bool } else { Kind::Int }))
                    }
                    Op::NewEmptyListI32 { dst } => Some((dst, Kind::SeqInt)),
                    Op::Lt { dst, .. }
                    | Op::Gt { dst, .. }
                    | Op::LtEq { dst, .. }
                    | Op::GtEq { dst, .. }
                    | Op::Eq { dst, .. }
                    | Op::NotEq { dst, .. }
                    | Op::ApproxEq { dst, .. }
                    | Op::Not { dst, .. }
                    | Op::Contains { dst, .. } => Some((dst, Kind::Bool)),
                    Op::Concat { dst, .. } | Op::FormatValue { dst, .. } => Some((dst, Kind::Text)),
                    Op::Length { dst, .. } => Some((dst, Kind::Int)),
                    Op::NewEmptyList { dst }
                    | Op::NewEmptyListWithCapacity { dst, .. }
                    | Op::NewEmptyListFixed { dst, .. } => Some((dst, Kind::SeqUnknown)),
                    Op::NewRange { dst, .. } => Some((dst, Kind::SeqInt)),
                    Op::NewList { dst, count: 0, .. } => Some((dst, Kind::SeqUnknown)),
                    Op::NewList { dst, start, count } => {
                        let items: Vec<Kind> = (0..count).map(|i| kind_at(&cur, start + i)).collect();
                        Some((dst, self.list_kind(&items)))
                    }
                    Op::ListPush { list, value } => {
                        Some((list, self.seq_of_kind(kind_at(&cur, value))))
                    }
                    Op::ListPop { list, dst } => {
                        Some((dst, self.elem_of_kind(kind_at(&cur, list))))
                    }
                    Op::NewEmptySet { dst } => Some((dst, Kind::SetUnknown)),
                    Op::SetAdd { set, value } => Some((set, set_of(kind_at(&cur, value)))),
                    Op::UnionOp { dst, lhs, .. } | Op::IntersectOp { dst, lhs, .. } => {
                        Some((dst, kind_at(&cur, lhs)))
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
                        let ck = kind_at(&cur, collection);
                        let dk = match self.as_composite(ck) {
                            Some(Composite::Map(_, v)) => v, // map get → value kind
                            Some(Composite::Tuple(positions)) => {
                                let at = match self.const_before(&pcs, i, index) {
                                    Some(Constant::Int(n)) => Some(*n),
                                    _ => None,
                                };
                                tuple_position_kind(&positions, at)
                            }
                            // `item i of text` → a single-character Text.
                            _ if ck == Kind::Text => Kind::Text,
                            _ => self.elem_of_kind(ck), // sequence element (scalar or composite)
                        };
                        Some((dst, dk))
                    }
                    Op::Args { dst } => Some((dst, Kind::SeqText)),
                    Op::NewEmptyMap { dst } | Op::NewEmptyMapWithCapacity { dst, .. } => {
                        Some((dst, Kind::MapUnknown))
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
                        let ck = kind_at(&cur, collection);
                        if ck == Kind::MapUnknown || self.is_map(ck) {
                            // First insert refines a fresh map to Map(key, value); later inserts keep it. A map
                            // that holds a value of the exact extension holds every value as one.
                            let held = match self.as_composite(ck) {
                                Some(Composite::Map(_, Kind::Tower)) => Kind::Tower,
                                _ => kind_at(&cur, value),
                            };
                            Some((collection, self.map_of(kind_at(&cur, index), held)))
                        } else if ck == Kind::SeqInt && kind_at(&cur, value) == Kind::Rational {
                            // A rational stored among integers makes the list one of rationals; its variable lands as
                            // one ([`crate::vm::numbers`]).
                            Some((collection, self.seq_of_kind(Kind::Rational)))
                        } else {
                            // A sequence write does not change the collection's kind.
                            None
                        }
                    }
                    Op::NewTuple { dst, start, count } => {
                        let positions: Vec<Kind> =
                            (0..count).map(|i| kind_at(&cur, start + i)).collect();
                        Some((dst, self.tuple_of(positions)))
                    }
                    Op::NewStruct { dst, type_name } => {
                        let k = self
                            .struct_type_name(type_name)
                            .map(|n| self.intern_composite(Composite::Struct(n)))
                            .unwrap_or(Kind::Unknown);
                        Some((dst, k))
                    }
                    Op::GetField { dst, obj, field } => {
                        let k = self
                            .field_of(kind_at(&cur, obj), field)
                            .map(|(_, k)| k)
                            .unwrap_or(Kind::Unknown);
                        Some((dst, k))
                    }
                    Op::NewInductive {
                        dst,
                        type_name,
                        ctor,
                        args_start,
                        count,
                    } => {
                        let first = if count > 0 {
                            kind_at(&cur, args_start)
                        } else {
                            Kind::Unknown
                        };
                        let k = match (self.text_const(type_name), self.text_const(ctor)) {
                            (Some(t), Some(c)) => self.inductive_kind(t, c, first),
                            _ => Kind::Unknown,
                        };
                        Some((dst, k))
                    }
                    Op::TestArm {
                        dst,
                        target,
                        variant,
                    } => {
                        if let Some(v) = self.text_const(variant) {
                            last_test = Some((target, v.to_string()));
                        }
                        Some((dst, Kind::Bool))
                    }
                    Op::BindArm {
                        dst,
                        target,
                        field,
                        index,
                    } => {
                        let tk = kind_at(&cur, target);
                        let k = if self.is_enum_like(tk) {
                            match &last_test {
                                Some((t, v)) if *t == target => {
                                    self.variant_payload_kind(tk, v, index as usize)
                                }
                                _ => Kind::Unknown,
                            }
                        } else if let Some(Composite::Struct(_)) = self.as_composite(tk) {
                            if self.text_const(field) == Some(crate::vm::SOME_BINDING_MARKER) {
                                tk
                            } else {
                                self.resolve_field(tk, field)
                                    .map(|(_, k)| k)
                                    .unwrap_or(Kind::Unknown)
                            }
                        } else if index == 0 {
                            tk
                        } else {
                            Kind::Unknown
                        };
                        Some((dst, k))
                    }
                    Op::SeqConcat { dst, lhs, rhs } => {
                        Some((dst, self.unify(kind_at(&cur, lhs), kind_at(&cur, rhs))))
                    }
                    Op::SliceOp {
                        dst, collection, ..
                    } => Some((dst, kind_at(&cur, collection))),
                    Op::DeepClone { dst, src } => Some((dst, kind_at(&cur, src))),
                    Op::IterPrepare { iterable } => {
                        let k = kind_at(&cur, iterable);
                        iter_elems.push(match self.as_composite(k) {
                            Some(Composite::Map(key, value)) => self.tuple_of(vec![key, value]),
                            _ if k == Kind::Text => Kind::Text,
                            _ => self.elem_of_kind(k),
                        });
                        None
                    }
                    Op::IterNext { dst, .. } => {
                        Some((dst, iter_elems.last().copied().unwrap_or(Kind::Unknown)))
                    }
                    Op::IterPop => {
                        iter_elems.pop();
                        None
                    }
                    Op::CallBuiltin {
                        dst,
                        builtin,
                        args_start,
                        ..
                    } => {
                        let k = match builtin {
                            // env.lg's `get` arrives expanded: whether the variable is set and its text.
                            BuiltinId::EnvHas => Kind::Bool,
                            BuiltinId::EnvValue => Kind::Text,
                            BuiltinId::TimeNow => Kind::Int,
                            BuiltinId::Sqrt | BuiltinId::ParseFloat => Kind::Float,
                            BuiltinId::Floor
                            | BuiltinId::Ceil
                            | BuiltinId::Round
                            | BuiltinId::ParseInt => Kind::Int,
                            BuiltinId::IntOfWord32 | BuiltinId::RandomInt => Kind::Int,
                            BuiltinId::Word32
                            | BuiltinId::Rotl
                            | BuiltinId::Rotr
                            | BuiltinId::Word32Shr
                            | BuiltinId::Wand
                            | BuiltinId::Wor
                            | BuiltinId::Wnot => Kind::Word32,
                            BuiltinId::Chr | BuiltinId::Format | BuiltinId::TextFromBytes => {
                                Kind::Text
                            }
                            BuiltinId::VfsReadFile | BuiltinId::VfsMount => Kind::Text,
                            BuiltinId::TextBytes => Kind::SeqInt,
                            BuiltinId::ReadReals => Kind::SeqFloat,
                            // A Real on either side makes `min`/`max` Real, as the VM promotes it.
                            BuiltinId::Min | BuiltinId::Max => {
                                match (kind_at(&cur, args_start), kind_at(&cur, args_start + 1)) {
                                    (Kind::Float, _) | (_, Kind::Float) => Kind::Float,
                                    (first, _) => first,
                                }
                            }
                            BuiltinId::Abs | BuiltinId::Copy => kind_at(&cur, args_start),
                            _ => Kind::Unknown,
                        };
                        Some((dst, k))
                    }
                    Op::Call { dst, func, .. } => Some((
                        dst,
                        self.returns
                            .borrow()
                            .get(func as usize)
                            .copied()
                            .unwrap_or(Kind::Unknown),
                    )),
                    Op::LoadToday { dst } => Some((dst, Kind::Date)),
                    Op::LoadNow { dst } => Some((dst, Kind::Moment)),
                    // A counter CRDT handle (per-replica map, kind 4/5); `GetField` sums it to an Int, so its
                    // value is `CrdtCounter` (which marks the holding field a counter field). A sequence CRDT
                    // (SharedSequence/RGA, kind 1) IS a plain seq handle on the single replica — an empty
                    // `SeqUnknown` (never a counter field, so `GetField` reads the field's declared `Seq of T`
                    // and `CrdtAppend`/`Length` treat it as a sequence). Other kinds are refused at lowering.
                    Op::NewCrdt { dst, kind } => Some((
                        dst,
                        match kind {
                            4 | 5 => Kind::CrdtCounter,
                            1 => Kind::SeqUnknown,
                            _ => Kind::Unknown,
                        },
                    )),
                    // Channels + tasks. A channel carries its element kind so `Receive`'s var is typed; a task
                    // handle / await result is opaque (the awaited body's return kind isn't statically known).
                    Op::ChanNew { dst, elem, .. } => Some((
                        dst,
                        self.intern_composite(Composite::Chan(chan_elem_to_kind(elem))),
                    )),
                    Op::ChanRecv { dst, chan } => {
                        let k = match self.as_composite(kind_at(&cur, chan)) {
                            Some(Composite::Chan(e)) => e,
                            _ => self.pipe_elem,
                        };
                        Some((dst, k))
                    }
                    Op::ChanTrySend { dst, .. } => Some((dst, Kind::Bool)),
                    Op::ChanTryRecv { dst, chan } => {
                        let e = match self.as_composite(kind_at(&cur, chan)) {
                            Some(Composite::Chan(e)) => e,
                            _ => self.pipe_elem,
                        };
                        Some((dst, e))
                    }
                    Op::SpawnHandle { dst, .. } => Some((dst, Kind::Unknown)),
                    // An awaited task answers `nothing`: a value-or-nothing Int that always holds nothing.
                    Op::TaskAwait { dst, .. } => Some((dst, Kind::Int)),
                    // A `Receive var from chan` select arm binds `var` to the channel's element kind.
                    Op::SelectArmRecv { chan, var } => {
                        let k = match self.as_composite(kind_at(&cur, chan)) {
                            Some(Composite::Chan(e)) => e,
                            _ => self.pipe_elem,
                        };
                        Some((var, k))
                    }
                    Op::SelectWait { dst_arm } => Some((dst_arm, Kind::Int)),
                    // A peer handle is its address string; an awaited message is a scalar (default Int), a
                    // stream a Seq — the offline loopback round-trips whatever was sent.
                    Op::NetMakePeer { dst, .. } => Some((dst, Kind::Peer)),
                    Op::NetAwait { dst, stream, .. } => Some((
                        dst,
                        if stream {
                            self.net_stream.get()
                        } else {
                            self.net_elem.get()
                        },
                    )),
                    // A closure handle, knowing its body — a call through it answers that body's return kind.
                    Op::MakeClosure { dst, func, .. } => {
                        Some((dst, self.intern_composite(Composite::Closure(func))))
                    }
                    Op::CallValue {
                        dst,
                        callee,
                        arg_count,
                        ..
                    } => {
                        let k = match self.as_composite(kind_at(&cur, callee)) {
                            Some(Composite::Closure(f)) => self
                                .returns
                                .borrow()
                                .get(f as usize)
                                .copied()
                                .unwrap_or(Kind::Unknown),
                            _ => self.closure_return(arg_count),
                        };
                        Some((dst, k))
                    }
                    Op::GlobalGet { dst, idx } => Some((
                        dst,
                        self.global_kinds
                            .get(idx as usize)
                            .copied()
                            .unwrap_or(Kind::Unknown),
                    )),
                    _ => None,
                };
                if let Some((dst, k)) = write {
                    // A declaration gives the value its kind where inference has not refined one.
                    let k = match self.declared.get(&pc).copied() {
                        Some(d)
                            if matches!(
                                k,
                                Kind::Unknown
                                    | Kind::SeqUnknown
                                    | Kind::SetUnknown
                                    | Kind::MapUnknown
                            ) && d != Kind::Unknown =>
                        {
                            d
                        }
                        Some(d) if self.is_enum_like(k) && self.is_enum_like(d) => {
                            match self.unify(k, d) {
                                Kind::Unknown => k,
                                refined => refined,
                            }
                        }
                        _ => k,
                    };
                    let i = dst as usize;
                    if i < cur.len() {
                        // Upgrade a freshly-created empty collection to the concrete kind it reached in the
                        // previous pass (family-matched, monotone — a concrete kind is never downgraded).
                        let ov = overrides[i];
                        let k = match k {
                            Kind::MapUnknown if self.is_map(ov) => ov,
                            Kind::SeqUnknown if self.is_seq_like(ov) => ov,
                            Kind::SetUnknown if is_concrete_set(ov) => ov,
                            // An `Option`/`Result` built by a constructor that fixes only some payload kinds
                            // (`none`, `a new Ok …`) takes the rest from what the register holds later.
                            other if self.is_enum_like(other) && self.is_enum_like(ov) => {
                                match self.unify(other, ov) {
                                    Kind::Unknown => other,
                                    refined => refined,
                                }
                            }
                            other => other,
                        };
                        cur[i] = k; // OVERWRITE — reaching definition
                    }
                }
                if self.tower {
                    self.tower_write(pc, &per_pc[&pc], &mut cur, forced, &mut lands, &mut natural);
                    // A `mutable` parameter that holds the extension's value hands one back.
                    if let Op::Call { func, args_start, .. } = self.prog.code[pc] {
                        let params = self.param_kinds.borrow();
                        for p in self.inout_params(func as usize) {
                            let arg = (args_start + p) as usize;
                            let exact = params.get(func as usize).and_then(|ps| ps.get(p as usize)) == Some(&Kind::Tower);
                            if exact && arg < cur.len() {
                                cur[arg] = Kind::Tower;
                            }
                        }
                    }
                }
                // A definition a number join lands computes its own kind and lands as its variable's.
                if let Some(w) = crate::vm::variables::defined(&self.prog.code[pc]).filter(|w| (*w as usize) < cur.len()) {
                    if let Some(&to) = number_lands.get(&pc) {
                        let k = cur[w as usize];
                        if k != Kind::Tower && k != to {
                            number_natural.insert(pc, k);
                            cur[w as usize] = to;
                        }
                    }
                    written.insert(pc, cur[w as usize]);
                }
            }
            if cur == overrides {
                break;
            }
            overrides = cur.clone();
        }
        let _ = r.is_main;
        Inferred {
            per_pc,
            lands,
            natural,
            written,
            number_natural,
        }
    }

    /// What the op at `pc` writes in a program the exact extension reaches, over the kinds `before` it and `cur`
    /// once its own write landed: a value of the extension, by the rule every backend reads
    /// ([`crate::vm::tower::exact_write`]), or by a definition of an exact web in `forced`, kept in `natural` with
    /// the kind it computes; and in `lands` when what it defines is one.
    fn tower_write(
        &self,
        pc: usize,
        before: &[Kind],
        cur: &mut [Kind],
        forced: &crate::vm::tower::Webs,
        lands: &mut std::collections::BTreeSet<usize>,
        natural: &mut std::collections::HashMap<usize, Kind>,
    ) {
        let op = self.prog.code[pc];
        let exact = crate::vm::tower::exact_write(
            &op,
            |r| kind_at(before, r) == Kind::Tower,
            |r| kind_at(before, r) == Kind::Text,
            |r| self.keyed(kind_at(before, r)),
        );
        let defined = crate::vm::variables::defined(&op);
        let landing = exact.or_else(|| defined.filter(|_| forced.defs.contains(&pc)));
        if let Some(w) = landing.filter(|w| (*w as usize) < cur.len()) {
            let computed = cur[w as usize];
            if exact.is_none() && computed != Kind::Tower {
                natural.insert(pc, computed);
            }
            cur[w as usize] = Kind::Tower;
        }
        if defined.is_some_and(|w| kind_at(cur, w) == Kind::Tower) {
            lands.insert(pc);
        }
    }
}

/// What a read of a file stops the program with, as a `printf` format of the path (`%s`): the VM's own
/// failure for a file its VFS does not hold, taken from its `VfsReadFile` builtin at emit time.
fn vfs_read_refusal() -> &'static str {
    static FORMAT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    FORMAT.get_or_init(|| {
        let path = "\u{0}";
        let failure = crate::semantics::builtins::call_builtin(
            BuiltinId::VfsReadFile,
            vec![crate::interpreter::RuntimeValue::Text(std::rc::Rc::new(
                path.to_string(),
            ))],
        )
        .expect_err("no file is named by a NUL");
        let (before, after) = failure
            .split_once(path)
            .expect("the VM names the missing file");
        format!(
            "{}%s{}",
            before.replace('%', "%%"),
            after.replace('%', "%%")
        )
    })
}

/// The sentences a program stops with when an operation fails, taken at emit time from the functions the VM
/// fails with, so a failure on this tier reads exactly as it does on the VM: `index` is a `printf` format of
/// the index asked for (`%lld`) and `missing_key` of the missing key's text (`%s`); the rest are plain text.
struct Refusals {
    index: String,
    missing_key: String,
    division: String,
    float_division: String,
    modulo: String,
    floor_division: String,
    exact_division: String,
    negative_exponent: String,
    exponent_too_large: String,
    parse_int: String,
    parse_float: String,
    random_bounds: String,
}

fn refusals() -> &'static Refusals {
    static REFUSALS: std::sync::OnceLock<Refusals> = std::sync::OnceLock::new();
    REFUSALS.get_or_init(|| {
        use crate::interpreter::RuntimeValue as V;
        use crate::semantics::arith;
        let fails = |r: Result<V, String>| r.expect_err("the VM refuses it");
        let builtin = |id: BuiltinId, text: &str| {
            crate::semantics::builtins::call_builtin(
                id,
                vec![V::Text(std::rc::Rc::new(text.to_string()))],
            )
        };
        let with_hole = |text: String, hole: &str, spec: &str| {
            let (before, after) = text.split_once(hole).expect("the VM names the value");
            format!(
                "{}{spec}{}",
                before.replace('%', "%%"),
                after.replace('%', "%%")
            )
        };
        const AT: i64 = -7_919_000_000_000_000_007;
        let index = crate::semantics::failcode::message(
            crate::semantics::failcode::FailCode::IndexOutOfBounds.code(),
            AT,
        );
        let key = "\u{0}";
        let empty = V::Map(std::rc::Rc::new(std::cell::RefCell::new(
            crate::interpreter::MapStorage::default(),
        )));
        let missing = crate::semantics::collections::index_get(
            &empty,
            &V::Text(std::rc::Rc::new(key.to_string())),
        )
        .expect_err("an empty map holds no key");
        Refusals {
            index: with_hole(index, &AT.to_string(), "%lld"),
            missing_key: with_hole(missing, key, "%s"),
            division: fails(arith::divide(V::Int(1), V::Int(0))),
            float_division: fails(arith::divide(V::Float(1.0), V::Float(0.0))),
            modulo: fails(arith::modulo(V::Int(1), V::Int(0))),
            floor_division: fails(arith::floor_divide(V::Int(1), V::Int(0))),
            exact_division: fails(arith::exact_divide(V::Int(1), V::Int(0))),
            negative_exponent: fails(arith::power(V::Int(2), V::Int(-1))),
            exponent_too_large: fails(arith::power(V::Int(2), V::Int(1 << 40))),
            parse_int: with_hole(fails(builtin(BuiltinId::ParseInt, key)), key, "%s"),
            parse_float: with_hole(fails(builtin(BuiltinId::ParseFloat, key)), key, "%s"),
            random_bounds: {
                const LO: i64 = 7_919_000_003;
                const HI: i64 = 7_919_000_001;
                let refusal = crate::semantics::builtins::random_int(LO, HI)
                    .expect_err("crossed bounds are refused");
                let (lo, hi) = (LO.to_string(), HI.to_string());
                assert!(
                    refusal.contains(&lo) && refusal.contains(&hi),
                    "the VM names both bounds: {refusal}"
                );
                refusal
                    .replace('%', "%%")
                    .replace(&lo, "%1$lld")
                    .replace(&hi, "%2$lld")
            },
        }
    })
}

/// The VM operation an arithmetic or ordering op performs, with its two operand registers: what the VM's
/// `binop` applies (`Value::add`, `Value::lt`, …).
#[allow(clippy::type_complexity)]
fn vm_binary_op(
    op: &Op,
) -> Option<(
    u16,
    u16,
    fn(&crate::vm::Value, &crate::vm::Value) -> Result<crate::vm::Value, String>,
)> {
    use crate::vm::Value;
    let binary: fn(&Value, &Value) -> Result<Value, String> = match *op {
        Op::Add { .. } | Op::AddAssign { .. } => Value::add,
        Op::Sub { .. } => Value::sub,
        Op::Mul { .. } => Value::mul,
        Op::Div { .. } => Value::div,
        Op::ExactDiv { .. } => Value::exact_div,
        Op::FloorDiv { .. } => Value::floor_div,
        Op::Mod { .. } => Value::modulo,
        Op::Pow { .. } => Value::pow,
        Op::BitAnd { .. } => Value::bitand,
        Op::BitOr { .. } => Value::bitor,
        Op::BitXor { .. } => Value::bitxor,
        Op::Shl { .. } => Value::shl,
        Op::Shr { .. } => Value::shr,
        Op::Lt { .. } => Value::lt,
        Op::Gt { .. } => Value::gt,
        Op::LtEq { .. } => Value::lte,
        Op::GtEq { .. } => Value::gte,
        _ => return None,
    };
    match *op {
        Op::AddAssign { dst, src } => Some((dst, src, binary)),
        Op::Add { lhs, rhs, .. }
        | Op::Sub { lhs, rhs, .. }
        | Op::Mul { lhs, rhs, .. }
        | Op::Div { lhs, rhs, .. }
        | Op::ExactDiv { lhs, rhs, .. }
        | Op::FloorDiv { lhs, rhs, .. }
        | Op::Mod { lhs, rhs, .. }
        | Op::Pow { lhs, rhs, .. }
        | Op::BitAnd { lhs, rhs, .. }
        | Op::BitOr { lhs, rhs, .. }
        | Op::BitXor { lhs, rhs, .. }
        | Op::Shl { lhs, rhs, .. }
        | Op::Shr { lhs, rhs, .. }
        | Op::Lt { lhs, rhs, .. }
        | Op::Gt { lhs, rhs, .. }
        | Op::LtEq { lhs, rhs, .. }
        | Op::GtEq { lhs, rhs, .. } => Some((lhs, rhs, binary)),
        _ => None,
    }
}

/// Whether a `NewCrdt` kind code makes a counter (`ConvergentCount`/`Tally`): a per-replica count map whose
/// field reads as its sum.
fn is_counter_crdt(code: u8) -> bool {
    use crate::semantics::crdt::CrdtKind;
    code == CrdtKind::Counter.new_crdt_code() || code == CrdtKind::PnCounter.new_crdt_code()
}

/// Whether a `NewCrdt` kind code makes a CRDT this tier lays out as its value on the single replica: a
/// `SharedSequence` as its `Seq`, a `Divergent` register as the value it resolved to.
fn lays_out_as_its_value(code: u8) -> bool {
    use crate::semantics::crdt::CrdtKind;
    code == CrdtKind::Seq.new_crdt_code() || code == CrdtKind::Register.new_crdt_code()
}

/// A struct value of type `type_name` with no fields.
fn bare_struct(type_name: &str) -> crate::interpreter::RuntimeValue {
    crate::interpreter::RuntimeValue::Struct(Box::new(crate::interpreter::StructValue {
        type_name: type_name.to_string(),
        fields: Default::default(),
    }))
}

/// What `text_from_bytes` stops with on bytes that are not UTF-8, as two `printf` formats: one for an
/// invalid sequence, of its length (`%1$d`) and the index it starts at (`%2$lld`), and one for bytes that
/// end inside a sequence, of the index it starts at (`%lld`). Taken at emit time from the VM's own builtin
/// and the standard library's own descriptions of the two faults, so the binary reports a fault in exactly
/// the words the VM's `String::from_utf8` gives it.
fn text_from_bytes_refusals() -> &'static (String, String) {
    static FORMATS: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
    FORMATS.get_or_init(text_from_bytes_formats)
}

fn text_from_bytes_formats() -> (String, String) {
    const AT: usize = 70_931;
    let at = AT.to_string();
    let describe = |tail: &[u8], lead: usize| {
        let mut bytes = vec![b'a'; lead];
        bytes.extend_from_slice(tail);
        String::from_utf8(bytes)
            .expect_err("the tail is not UTF-8")
            .to_string()
    };
    let failure = crate::semantics::builtins::call_builtin(
        BuiltinId::TextFromBytes,
        vec![crate::semantics::builtins::bytes_to_seq(&[0xFF])],
    )
    .expect_err("0xFF is not UTF-8");
    let lead = failure
        .strip_suffix(&describe(&[0xFF], 0))
        .expect("the VM's failure ends with the standard library's description")
        .replace('%', "%%");
    let format = |text: String, len: Option<(&str, &str)>, index: &str| {
        let words: Vec<String> = text
            .replace('%', "%%")
            .split(' ')
            .map(|w| {
                if w == at {
                    index.to_string()
                } else {
                    match len {
                        Some((l, spec)) if w == l => spec.to_string(),
                        _ => w.to_string(),
                    }
                }
            })
            .collect();
        let out = format!("{lead}{}", words.join(" "));
        assert!(
            out.contains(index) && len.is_none_or(|(_, spec)| out.contains(spec)),
            "the standard library names every part of a UTF-8 fault: {out}"
        );
        out
    };
    let invalid = format(
        describe(&[0xF0, 0x90, 0x80, b'a'], AT),
        Some(("3", "%1$d")),
        "%2$lld",
    );
    let incomplete = format(describe(&[0xE0], AT), None, "%lld");
    (invalid, incomplete)
}

/// Which field cells, `(record register, cell)`, each register may hold the value of ([`Emitter::field_plan`]).
type Aliases = std::collections::BTreeMap<u16, std::collections::BTreeSet<(u16, usize)>>;

/// Whether `op` reads a register holding a field's value without keeping it: it measures, indexes, searches,
/// shows, compares, tests or copies out of it, or copies it to another register (which then aliases it).
fn only_inspects_an_alias(op: &Op) -> bool {
    matches!(
        op,
        Op::Move { .. }
            | Op::Length { .. }
            | Op::Index { .. }
            | Op::IndexUnchecked { .. }
            | Op::Contains { .. }
            | Op::Show { .. }
            | Op::Eq { .. }
            | Op::NotEq { .. }
            | Op::FormatValue { .. }
            | Op::Concat { .. }
            | Op::SliceOp { .. }
            | Op::SeqConcat { .. }
            | Op::DeepClone { .. }
            | Op::Not { .. }
            | Op::JumpIfFalse { .. }
            | Op::JumpIfTrue { .. }
    )
}

/// What stops a program whose control reached code the emitter proved never runs ([`NeverRuns`]).
const NEVER_RUNS: &str = "the RISC-V tier reached code it proved never runs";

/// The pcs control can reach from the op at `pc`: where it jumps ([`crate::vm::disasm::op_targets`]) and,
/// unless it never continues — a jump, a return, a halt, a `Fail with` — the next pc.
fn successors(op: &Op, pc: usize) -> Vec<usize> {
    let mut out = crate::vm::disasm::op_targets(op);
    let ends = matches!(
        op,
        Op::Jump { .. }
            | Op::Return { .. }
            | Op::ReturnNothing
            | Op::Halt
            | Op::FailWith { .. }
            | Op::CallBuiltin {
                builtin: BuiltinId::Fail | BuiltinId::FailReason,
                ..
            }
    );
    if !ends {
        out.push(pc + 1);
    }
    out
}

/// The kind of the item of a tuple of `positions` at index `at` (1-based, from the back when negative): that
/// position's, or when the index is not known, the one kind every position has. An index past either end
/// fails before anything receives an item, so its item takes the first position's kind.
fn tuple_position_kind(positions: &[Kind], at: Option<i64>) -> Kind {
    let n = positions.len() as i64;
    match at {
        Some(i) if (1..=n).contains(&i) => positions[(i - 1) as usize],
        Some(i) if (-n..=-1).contains(&i) => positions[(n + i) as usize],
        Some(_) => positions.first().copied().unwrap_or(Kind::Unknown),
        None => match positions.split_first() {
            Some((first, rest)) if rest.iter().all(|k| k == first) => *first,
            _ => Kind::Unknown,
        },
    }
}

/// The kind of `reg` in a per-pc snapshot (Unknown if out of range).
fn kind_at(cur: &[Kind], reg: u16) -> Kind {
    cur.get(reg as usize).copied().unwrap_or(Kind::Unknown)
}

/// A frame big enough for `register_count` 8-byte slots plus the saved `ra`, 16-byte aligned.
fn frame_size(register_count: usize) -> i64 {
    let n = 8 + 8 * register_count as i64;
    (n + 15) & !15
}

fn fits_imm12(v: i64) -> bool {
    (-2048..=2047).contains(&v)
}

/// Map a declared boundary type to a scalar [`Kind`]. Ints (incl. wrapping words) → Int; the rest that
/// Phase 1 does not model → Unknown (a sound refusal downstream).
fn boundary_to_kind(ty: &Option<BoundaryType>) -> Kind {
    if ty.as_ref().is_some_and(crate::vm::tower::declared) {
        return Kind::Tower;
    }
    match ty {
        // `Word64` rides an i64 slot and wraps at 64 bits exactly like the VM's `WordVal::W64` — plain
        // i64 arithmetic already wraps mod 2⁶⁴ — so it shares the Int lowering. `Word32` needs its own
        // kind (its arithmetic masks to 32 bits).
        Some(BoundaryType::Int) | Some(BoundaryType::Word64) => Kind::Int,
        Some(BoundaryType::Word32) => Kind::Word32,
        Some(BoundaryType::Bool) => Kind::Bool,
        Some(BoundaryType::Float) => Kind::Float,
        Some(BoundaryType::Text) => Kind::Text,
        Some(BoundaryType::Builtin(name)) if name == "Rational" => Kind::Rational,
        Some(BoundaryType::Seq(elem)) => match **elem {
            BoundaryType::Int | BoundaryType::Word64 => Kind::SeqInt,
            BoundaryType::Word32 => Kind::SeqWord32,
            BoundaryType::Float => Kind::SeqFloat,
            BoundaryType::Text => Kind::SeqText,
            _ => Kind::SeqUnknown,
        },
        Some(BoundaryType::Set(elem)) => match **elem {
            BoundaryType::Int | BoundaryType::Word32 | BoundaryType::Word64 => Kind::SetInt,
            BoundaryType::Text => Kind::SetText,
            _ => Kind::SetUnknown,
        },
        _ => Kind::Unknown,
    }
}

/// Merge two register kinds for an arithmetic result: Int⊔Float = Float (numeric promotion, matching
/// the VM's mixed arithmetic); an unknown operand carries the other; any other disagreement ⇒ Unknown.
/// The kind `op` (`add`, `sub`) makes of two temporal operands, each riding i64 nanoseconds: an instant moved by a
/// duration, the duration between two instants, two durations summed or differenced. `None` for any other pairing.
fn temporal_kind(op: &str, a: Kind, b: Kind) -> Option<Kind> {
    match (op, a, b) {
        ("add", Kind::Moment, Kind::Duration)
        | ("add", Kind::Duration, Kind::Moment)
        | ("sub", Kind::Moment, Kind::Duration) => Some(Kind::Moment),
        ("sub", Kind::Moment, Kind::Moment)
        | ("add", Kind::Duration, Kind::Duration)
        | ("sub", Kind::Duration, Kind::Duration) => Some(Kind::Duration),
        _ => None,
    }
}

fn join(a: Kind, b: Kind) -> Kind {
    match (a, b) {
        (Kind::Unknown, x) | (x, Kind::Unknown) => x,
        (x, y) if x == y => x,
        (Kind::Int, Kind::Float) | (Kind::Float, Kind::Int) => Kind::Float,
        // A real beside a rational makes the operation a real one, as the VM's.
        (Kind::Rational, Kind::Float) | (Kind::Float, Kind::Rational) => Kind::Float,
        // A Rational absorbs an Int operand (`r + 3` → Rational); the Int promotes to `n/1`.
        (Kind::Rational, Kind::Int) | (Kind::Int, Kind::Rational) => Kind::Rational,
        // `text + x` is string concatenation (the VM's `arith::add` stringifies the other operand), so
        // any binary op with a Text operand yields Text. Non-`add` ops with a Text operand error in the
        // VM and are refused at lowering regardless — the kind only steers `add`'s concat routing.
        (Kind::Text, _) | (_, Kind::Text) => Kind::Text,
        // A value of the exact extension takes every other operand into its own kind.
        (Kind::Tower, _) | (_, Kind::Tower) => Kind::Tower,
        // Word32 arithmetic/bitwise stays in the ℤ/2³² ring.
        (Kind::Word32, Kind::Word32) => Kind::Word32,
        _ => Kind::Unknown,
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_riscv/emit/tests.rs"]
mod tests;
