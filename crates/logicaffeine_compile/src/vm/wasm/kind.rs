//! Static per-register kind inference for the AOT backend.
//!
//! The bytecode's arithmetic and `Show` ops are runtime-polymorphic: one `Op::Add` adds two
//! Ints, two Floats, or promotes — the VM dispatches on the live `RuntimeValue`. The browser
//! JIT tier sidesteps this with a runtime gate (it fires only when every argument is an `Int`);
//! a standalone AOT module has no such fallback, so it must know each register's *static* kind
//! to choose `i64` vs `f64` ops and the right `print_*` sink.
//!
//! This is a small dataflow fixpoint over a function's region: constants and declared
//! parameter/return kinds seed it, and each op propagates a kind to its destination. A register
//! that would carry two incompatible kinds (`Int` and `Float`, or `Int` and `Bool` — they need
//! different wasm locals, or different display) makes the whole function `Unsupported` (a sound
//! refusal, never a miscompile). Pure-integer kernels infer everything `Int` and never conflict.

use super::encode::{F64, I32, I64};
use super::WasmLowerError;
use crate::semantics::builtins::BuiltinId;
use crate::vm::instruction::{BoundaryType, ChanElem, CompiledFunction, Constant, EnumTypeDef, Op, Reg, StructTypeDef};
use crate::vm::native_tier::{ParamKind, PinElem, SlotKind};

/// The static kind of a scalar register. Each maps to exactly one wasm local type, so a
/// register's kind determines both its `local` declaration and how its ops lower.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// A signed 64-bit integer (`i64` local). Also covers the truthy-Int boolean a comparison
    /// would feed a jump — but a value *displayed* as a boolean is [`Kind::Bool`].
    Int,
    /// A boolean (`i64` local holding 0/1, like the VM's truthy-Int booleans) — distinguished
    /// from [`Kind::Int`] only so `Show` picks `print_bool` over `print_i64`.
    Bool,
    /// A Unicode scalar (`i64` local holding the code point, `char as u32`) — `RuntimeValue::Char`.
    /// Distinguished from [`Kind::Int`] only so `Show` picks `print_char` (which emits the UTF-8
    /// character) over `print_i64` (which would emit the numeric code point).
    Char,
    /// A double-precision float (`f64` local).
    Float,
    /// A calendar date as days-since-epoch (`i32` local) — `RuntimeValue::Date`.
    Date,
    /// A moment as nanos-since-epoch (`i64` local) — `RuntimeValue::Moment`.
    Moment,
    /// A duration as a nanosecond tick count (`i64` local) — `RuntimeValue::Duration`. Rides i64 like an
    /// Int (arithmetic/compare are i64), but `Show` renders the magnitude-bucketed form (`5s`/`3h`/…).
    Duration,
    /// A wall-clock time-of-day as a nanosecond tick count (`i64` local) — `RuntimeValue::Time`. Rides
    /// i64 like an Int, but `Show` renders `HH:MM:SS[.frac]`.
    Time,
    /// A calendar span (`months` + `days`, both i32) packed into one `i64` local (`months << 32 | days`)
    /// — `RuntimeValue::Span`. `Moment`/`Date` `+`/`-` a Span is civil calendar arithmetic (linked
    /// `logos_rt_moment_add_span`/`date_add_span`); `Show` renders `1 year and 2 months and 3 days`.
    Span,
    /// A `Lanes4Word32` SIMD lane vector (128-bit = 4×`Word32`) — an i32 handle to a 16-byte `[u32; 4]`
    /// block. LINKER MODE ONLY: `lanes4Of`/`lanes4Word32` build it inline, `seqOfLanes4W32` unpacks it,
    /// and the four SHA-1 SHA-NI ops (`sha1rnds4`/`sha1msg1`/`sha1msg2`/`sha1nexte`) call the linked
    /// `base::sha_ops` spec — the substrate a SHA-1 (and `uuid_v3`/`uuid_v5`) written in Logos compiles to.
    Lanes,
    /// A growable sequence of `i64` scalars (Int) — an `i32` handle to a stable header
    /// `[len:i32][cap:i32][data_ptr:i32]` in linear memory (so a realloc-on-push never
    /// invalidates the register holding it). The element kind (here Int) rides the variant.
    SeqInt,
    /// A growable sequence of `Bool` scalars — the SAME i64-0/1 storage as [`SeqInt`] (a Bool is an
    /// i64 `0`/`1`), but its element reads as a `Bool` and its whole-sequence `Show` renders
    /// `[true, false, …]` (not `[1, 0, …]`), matching the VM's `ListRepr::Bools`.
    SeqBool,
    /// A growable sequence of `f64` scalars (Float).
    SeqFloat,
    /// A growable sequence of `Text` handles — an `i32` handle to the same header, over a buffer of
    /// 8-byte slots each holding a Text handle (`i32`) in its low word. The first handle-element
    /// sequence; element access / `length` / iteration work, whole-sequence `Show` is deferred.
    SeqText,
    /// A growable sequence of `Struct` handles (a list of records). Like [`SeqText`] but each slot's
    /// handle is a struct; an `Index` flows the element struct's field layout to its destination (so
    /// `(item N of people)'s field` works). Whole-sequence `Show` is deferred (struct field order).
    SeqStruct,
    /// A growable sequence of `Enum` handles (a list of variant values). Like [`SeqStruct`] but the
    /// elements are enums; `Inspect`/`TestArm` reads each element's tag (no layout flow needed for
    /// nullary matching). Whole-sequence `Show` is deferred.
    SeqEnum,
    /// A growable sequence of `SeqInt` handles — a NESTED Int sequence (a matrix `[[1,2],[3,4]]`).
    /// `item N of m` yields a `SeqInt` (the row); indexing that yields an `Int`. The element-kind
    /// chain (`SeqSeqInt` → `SeqInt` → `Int`) flows through `seq_elem` with no extra tracking.
    SeqSeqInt,
    /// A new empty sequence whose element kind is not yet known — refined to `SeqInt`/`SeqFloat`/
    /// `SeqText` by the first `Push` (or stays `SeqAny` for a never-pushed sequence; `length` works).
    SeqAny,
    /// A UTF-8 text string — an `i32` handle to the same stable header
    /// `[len:i32][cap:i32][data_ptr:i32]`, where `len`/`cap` are BYTE counts and `data_ptr` points
    /// at the UTF-8 bytes (`length` is a byte count, matching the tree-walker).
    Text,
    /// A struct/record value — an `i32` handle to a header `[num_fields:i32][cap:i32][data_ptr:i32]`
    /// whose `data_ptr` points at an array of 8-byte field slots in declaration/insertion order.
    /// Field names are compile-time only (the [`struct_layout`] analysis maps each field to its
    /// slot and value kind); the runtime object is a flat slot array.
    Struct,
    /// A `Map of Int to Int` — an `i32` handle to a header `[num_entries:i32][cap:i32][data_ptr:i32]`
    /// whose `data_ptr` points at an array of 16-byte `[key:i64][value:i64]` entries. Lookup is a
    /// linear scan (order-independent, so byte-identical to the VM's hashmap for get/insert/contains
    /// /length; iteration/`Show`, which expose hashmap order, stay deferred).
    Map,
    /// A `Set of Int` — an `i32` handle to a header `[num:i32][cap:i32][data_ptr:i32]` over an array
    /// of unique `i64` values (`add` dedups by linear scan). Like [`Map`], add/contains/length are
    /// order-independent so they're byte-identical to the VM's hashset; iteration/`Show` deferred.
    Set,
    /// A `Set of Text` — the same `[num][cap][data_ptr]` header + 8-byte slots as [`Set`], but each
    /// slot's low word is a `Text` handle and `add`/`remove`/`contains` dedup by BYTE equality (the
    /// VM compares string values, not handle identity). Insertion-ordered, so its `{s0, s1, …}`
    /// display is deterministic (`print_set_text`). Distinct from [`Set`] only in the element
    /// comparison + display; the storage/scan/swap-remove are identical.
    SetText,
    /// A `SharedSet of Text` (OR-Set CRDT) FIELD of a `Shared` struct — the same byte layout + ops as
    /// [`SetText`], but MUTABLE-SHARED (reference-semantic, not value-semantic): a CRDT field is held
    /// behind an `Rc`, so `Add X to <obj>'s <set-field>` mutates the shared field IN PLACE. Modeled as
    /// a distinct kind purely so it is NON-cow-clonable — `GetField` does not retain it and `SetAdd`/
    /// `RemoveFrom` do not copy-on-write it, so the add reaches the field (a value-semantic `SetText`
    /// would clone). Single-replica ops (add/remove/contains/show) are byte-identical to the VM.
    CrdtSetText,
    /// An enum/inductive value — an `i32` handle whose first word is the constructor TAG (the
    /// constructor name's constant index; constant dedup makes this equal across `NewInductive` and
    /// `TestArm` for the same name, so an `i32.eq` on tags == the VM's string compare). A payload-
    /// carrying constructor lays its arguments after the tag in 8-byte slots (offset `8*(1+index)`),
    /// extracted by `BindArm`.
    Enum,
    /// A closure value — an `i32` handle to a heap object whose first word is the body's function
    /// index (a no-capture closure is just that index; captures would follow). Built by
    /// `MakeClosure`, invoked by `CallValue` via `call_indirect` through the module's function table.
    Closure,
    /// A HETEROGENEOUS tuple `(a, b, …)` — an `i32` handle to the same `[len][cap][data_ptr]` header
    /// over a flat array of 8-byte slots, each holding its element at that element's kind/width
    /// (Int/Float/Text…). A `tuple_layout` analysis maps each position to its defining register's
    /// kind; `item N of t` with a constant N reads slot N-1. (A homogeneous tuple uses a `SeqX`
    /// kind instead, sharing the sequence machinery.)
    Tuple,
    /// An exact `Rational` (`7 / 2 → 7/2`) — an `i32` handle to a 16-byte value `[num: i64][den: i64]`,
    /// gcd-reduced with `den > 0`. Produced by `ExactDiv` in a `Rational` context. `Show` renders
    /// `num/den`, or just `num` when `den == 1` (matching the VM, which downsizes a whole quotient to
    /// an `Int`). Immutable, so the handle is freely shared.
    Rational,
    /// An `Optional` value (`Try to receive` result, a bare `Nothing`) — an `i32` handle where `0`
    /// means `Nothing` and any non-zero handle points to an 8-byte heap box holding the present inner
    /// scalar (the box `[value: i64]`, or the f64 bits for a `Float` inner). The inner kind is carried
    /// out-of-band in [`StructLayout::opt_inner`] (from the producing channel's element kind) so `Show`
    /// prints "nothing" for the null handle or the inner value otherwise, and `x is equal to nothing`
    /// is a plain `i32` handle comparison against `0`.
    Optional,
    /// A `Word32` — the ℤ/2³² wrapping-integer ring (the MD5/SHA-1 crypto substrate). Rides a native
    /// wasm `i32` local: every op is the cheapest possible lowering because wasm `i32` arithmetic
    /// wraps by definition (no overflow check, unlike `Int`'s checked-i64). `Show` prints the UNSIGNED
    /// value (`i64.extend_i32_u` then `print_word`).
    Word32,
    /// A `Word64` — the ℤ/2⁶⁴ wrapping-integer ring (the Keccak/SHA-3 substrate). Rides a native wasm
    /// `i64` local with wrapping ops; `Show` prints the unsigned `u64` value via `print_word`.
    Word64,
    /// A growable sequence of `Word32` — the crypto message-schedule / state array (SHA-256's `W[0..64]`,
    /// MD5's block words). Rides the `SeqText` representation (an `i32` element per 8-byte slot); `item
    /// N of xs` yields a `Word32`, refined from a `Word32` `Push` like any list.
    SeqWord32,
    /// A growable sequence of `Word64` — the Keccak/SHA-512 state lane array. Rides the `SeqInt`
    /// representation (an `i64` element per slot); `item N of xs` yields a `Word64`.
    SeqWord64,
    /// An arbitrary-precision `BigInt` — an `i32` handle to a leaked `Box<logicaffeine_base::BigInt>`
    /// managed by the linked runtime. LINKER MODE ONLY (produced by an integer `Op::Pow` and the BigInt
    /// arithmetic ops); the intermediate stays a HANDLE (not yet a decimal), so a chain like
    /// `(2^100) * (3^50)` keeps computing on real BigInts and only `to_text`s at the final `Show`. There
    /// is no self-contained representation — a standalone module never produces this kind.
    BigInt,
    /// An EXACT `Complex` number — an `i32` handle to a leaked `Box<logicaffeine_base::Complex>` (Rational
    /// components) managed by the linked runtime. LINKER MODE ONLY (produced by the `complex(re, im)`
    /// builtin + `+`/`-`/`*` on a Complex operand); the intermediate stays a HANDLE, `to_text`'d at `Show`
    /// — so `i * i = -1` etc. are exact. No self-contained representation (standalone never produces it).
    Complex,
    /// An exact `Modular` (ℤ/nℤ) — an i32 handle to a leaked `Box<logicaffeine_base::Modular>` (value +
    /// modulus) in the linked runtime. LINKER MODE ONLY (`modular(v, n)` builtin + `+`/`-`/`*`); reduces
    /// on construction, wraps in the ring, `to_text`s (`v (mod n)`) at `Show`. No self-contained form.
    Modular,
    /// An exact base-10 `Decimal` — an i32 handle to a leaked `Box<logicaffeine_base::Decimal>` in the
    /// linked runtime. LINKER MODE ONLY (`decimal("…")` parses a Text; `+`/`-`/`*` keep exact scale;
    /// `to_text`s at `Show`). No self-contained form. (Division/comparison carry a scale — deferred.)
    Decimal,
    /// An exact `Money` (Decimal amount + currency) — an i32 handle to a leaked `Box<base::Money>` in
    /// the linked runtime. LINKER MODE ONLY (`money(amount, "USD")`; `+`/`-` require matching currency;
    /// `to_text`s at `Show`). No self-contained form. (Division/cross-currency deferred.)
    Money,
    /// An exact physical `Quantity` (rational magnitude in the SI base + a display unit) — an i32 handle
    /// to a leaked `Box<{Quantity, Unit}>` in the linked runtime. LINKER MODE ONLY (`5 meters` /
    /// `quantity(v, "unit")` construct; `X in <unit>` re-expresses; `+`/`-` keep the left unit,
    /// dimension-checked; `×`/`÷` combine dimensions; `to_text`s at `Show`). No self-contained form.
    Quantity,
    /// An RFC 9562 `Uuid` (a 16-byte value) — an i32 handle to a leaked `Box<base::Uuid>` in the linked
    /// runtime. LINKER MODE ONLY (`uuid("…")` parses; `uuid_nil`/`uuid_max`/`uuid_dns`/… are constants;
    /// `uuid_version` reads the version nibble; equality compares the 16 bytes; `to_text`s the canonical
    /// lowercase form at `Show`). No self-contained form (parse/hash live in `base::Uuid`).
    Uuid,
    /// A general SIMD lane vector (`RuntimeValue::Lanes` = `base::LanesVal`, any width — `Lanes16Word8`
    /// / `Lanes16Word16` / `Lanes8Word32` / `Lanes4Word64`) — an i32 handle to a leaked `Box<LanesVal>`
    /// in the linked runtime. LINKER MODE ONLY: the SSE byte-lane vocabulary (`lanes16Word8`/`shuffle16`/
    /// `maddubs16`/`packus16`/`interleave*`/`byteAdd16`/`shrBytes16`/`splat*`/`lanes8Word32`/`lanes4Word64`)
    /// delegates to the SAME pure-Rust `base::word` spec the VM uses (SSSE3 lowers to `pshufb` etc. on x86,
    /// a scalar fallback elsewhere — including wasm32), so each op is bit-identical to the interpreter.
    /// Distinct from [`Kind::Lanes`] (the INLINE 16-byte `Lanes4Word32` block the SHA-1 SHA-NI ops use).
    LanesV,
    /// A wire-decoded DYNAMIC value — an i32 handle to a leaked `Box<RuntimeValue>` in the linked runtime,
    /// whose concrete type is only known at RUNTIME (the result of `readWireProgram`). LINKER MODE ONLY:
    /// it can be `Show`n (`logos_rt_dynamic_to_text` = `RuntimeValue::to_display_string`) or passed to
    /// `run_accepted` (`AcceptanceContract::apply` over the boxed `RuntimeValue`). The one boxed value in
    /// the otherwise-statically-kinded AOT — it exists precisely because a wire program's type is dynamic.
    Dynamic,
}

impl Kind {
    /// The wasm value-type byte for a register of this kind.
    pub(crate) fn wasm_valtype(self) -> u8 {
        match self {
            Kind::Int | Kind::Bool | Kind::Char | Kind::Moment | Kind::Duration | Kind::Time | Kind::Span | Kind::Word64 => I64,
            Kind::Float => F64,
            Kind::Date | Kind::SeqInt | Kind::SeqBool | Kind::SeqFloat | Kind::SeqText | Kind::SeqStruct | Kind::SeqEnum | Kind::SeqSeqInt | Kind::SeqAny | Kind::Text | Kind::Struct | Kind::Map | Kind::Set | Kind::SetText | Kind::CrdtSetText | Kind::Enum | Kind::Closure | Kind::Tuple | Kind::Rational | Kind::Optional | Kind::Word32 | Kind::SeqWord32 | Kind::SeqWord64 | Kind::BigInt | Kind::Complex | Kind::Modular | Kind::Decimal | Kind::Money | Kind::Quantity | Kind::Uuid | Kind::Lanes | Kind::LanesV | Kind::Dynamic => I32,
        }
    }

    /// The element kind of a sequence kind (the kind an `Index` yields), or `None` if not a Seq
    /// or the element kind is still unknown (`SeqAny`).
    pub(crate) fn seq_elem(self) -> Option<Kind> {
        match self {
            Kind::SeqInt => Some(Kind::Int),
            Kind::SeqBool => Some(Kind::Bool),
            Kind::SeqFloat => Some(Kind::Float),
            Kind::SeqText => Some(Kind::Text),
            Kind::SeqStruct => Some(Kind::Struct),
            Kind::SeqEnum => Some(Kind::Enum),
            Kind::SeqSeqInt => Some(Kind::SeqInt),
            Kind::SeqWord32 => Some(Kind::Word32),
            Kind::SeqWord64 => Some(Kind::Word64),
            _ => None,
        }
    }

    /// Whether this is any sequence kind.
    pub(crate) fn is_seq(self) -> bool {
        matches!(self, Kind::SeqInt | Kind::SeqBool | Kind::SeqFloat | Kind::SeqText | Kind::SeqStruct | Kind::SeqEnum | Kind::SeqSeqInt | Kind::SeqAny | Kind::SeqWord32 | Kind::SeqWord64)
    }

    /// Whether two kinds can share one wasm local (same value type). `Int`/`Bool` share `i64`;
    /// `Float` is alone on `f64`.
    fn same_valtype(self, other: Kind) -> bool {
        self.wasm_valtype() == other.wasm_valtype()
    }

    pub(crate) fn from_slot(slot: SlotKind) -> Kind {
        match slot {
            SlotKind::Int => Kind::Int,
            SlotKind::Bool => Kind::Bool,
            SlotKind::Float => Kind::Float,
        }
    }
}

/// The static kind of a constant-pool entry, when it is a P0 scalar; `None` for constants the
/// scalar backend does not represent (Text, Char, temporal, …) — a register loaded from one
/// stays unknown and any use of it is rejected at lowering.
fn const_kind(c: &Constant) -> Option<Kind> {
    match c {
        Constant::Int(_) => Some(Kind::Int),
        Constant::Bool(_) => Some(Kind::Bool),
        Constant::Char(_) => Some(Kind::Char),
        // A bare `Constant::Nothing` reads as the Int `0`: it is the read-as-zero default of a `Shared`
        // CRDT counter field (`crdt_counter_bump` = `field.wrapping_add(±n)`, a `Nothing` field = 0) and
        // the write-only dead `Zone`-name binding — never Shown as a bare const. A GENUINE `Optional`
        // (Shown as "nothing", compared with `is nothing`) arises only from `ChanTryRecv`, which types
        // its result `Kind::Optional` directly; the `nothing` literal in `x is equal to nothing` stays
        // this Int `0` and the compare special-cases the Optional side against it.
        Constant::Nothing => Some(Kind::Int),
        Constant::Float(_) => Some(Kind::Float),
        Constant::Text(_) => Some(Kind::Text),
        Constant::Date(_) => Some(Kind::Date),
        Constant::Moment(_) => Some(Kind::Moment),
        // A `Duration`/`Time` rides a single i64 tick count. The only one the AOT loads is a
        // `select`'s `After N …` ticks register, which is dead (the timeout fires deterministically),
        // so typing it `Int` just lets its i64 local declare.
        Constant::Duration(_) => Some(Kind::Duration),
        Constant::Time(_) => Some(Kind::Time),
        Constant::Span { .. } => Some(Kind::Span),
        _ => None,
    }
}

/// The result kind of an arithmetic op given its operand kinds: `Int ∘ Int = Int`,
/// `Float` anywhere ⇒ `Float` (the VM promotes `Int ∘ Float → Float`). A `Bool` operand is a
/// type error in a well-typed program, so it is rejected. `None` (an operand not yet inferred)
/// leaves the result unknown for this pass — the fixpoint revisits it.
/// The result kind of `+` (`Op::Add`/`Op::AddAssign`). A Text operand makes it string concatenation
/// (the result is a fresh Text — `lower_concat` stringifies the other operand); otherwise it is the
/// numeric join. A Text/non-Text mix where the non-Text side is still unknown stays Text — `""` pins
/// the chain even before the interior values resolve.
fn add_join(a: Option<Kind>, b: Option<Kind>) -> Result<Option<Kind>, WasmLowerError> {
    match (a, b) {
        (Some(Kind::Text), _) | (_, Some(Kind::Text)) => Ok(Some(Kind::Text)),
        (a, b) => numeric_join(a, b),
    }
}

fn numeric_join(a: Option<Kind>, b: Option<Kind>) -> Result<Option<Kind>, WasmLowerError> {
    match (a, b) {
        (Some(Kind::Bool), _) | (_, Some(Kind::Bool)) => {
            Err(WasmLowerError::Unsupported("boolean operand to arithmetic"))
        }
        (Some(Kind::Float), _) | (_, Some(Kind::Float)) => Ok(Some(Kind::Float)),
        (Some(Kind::Int), Some(Kind::Int)) => Ok(Some(Kind::Int)),
        // Word arithmetic stays in the same ℤ/2ⁿ ring (both operands the same width) — `Word ± Word`,
        // `Word * Word` wrap; a Word/Int mix is a width error the VM also rejects, so it defers.
        (Some(Kind::Word32), Some(Kind::Word32)) => Ok(Some(Kind::Word32)),
        (Some(Kind::Word64), Some(Kind::Word64)) => Ok(Some(Kind::Word64)),
        // One side still unknown and the other not Float: defer (revisited next pass).
        _ => Ok(None),
    }
}

/// The inferred kind of every register `0..num_regs` in a function region.
pub(crate) struct KindTable {
    kinds: Vec<Option<Kind>>,
}

impl KindTable {
    /// The inferred kind of register `r`, or `None` if it was never given one (a register the
    /// function writes only via unsupported ops — using it elsewhere is rejected at lowering).
    pub(crate) fn get(&self, r: usize) -> Option<Kind> {
        self.kinds.get(r).copied().flatten()
    }

    /// The wasm local type for register `r` (defaults to `i64` for never-kinded registers — a
    /// harmless declaration, since such a register is never read on a lowerable path).
    pub(crate) fn valtype(&self, r: usize) -> u8 {
        self.get(r).map(Kind::wasm_valtype).unwrap_or(I64)
    }

    /// An empty kind table — for a stub function body (an unreachable import the AOT drops).
    pub(crate) fn empty() -> KindTable {
        KindTable { kinds: Vec::new() }
    }
}

/// Pair each `IterNext` with the iterable register of its governing `IterPrepare`. The compiler
/// emits `IterPrepare(it); loop: IterNext …; body; Jump loop; IterPop` and nests loops fully, so
/// in code order the k-th `IterPrepare` is the header of the k-th `IterNext` (a loop's header
/// `IterNext` always precedes any `IterPrepare` nested in its body). Returns a vec indexed by op
/// position: `Some(iterable_reg)` at each `IterNext`, `None` elsewhere. (Break-emitted extra
/// `IterPop`s do not perturb this — only `IterPrepare`/`IterNext` counts matter.)
/// Registers whose Int-vs-Bool *label* (not just value) changes how an op lowers: a `Show` sink
/// (`print_i64` vs `print_bool`) or a `Not` sink. Every other op treats Int and Bool identically
/// (both are `i64`), so a register the VM reuses across both kinds can safely collapse to one
/// `i64` local UNLESS it feeds one of these — see [`unify_strict`]'s Int/Bool merge.
fn label_sensitive_regs(ops: &[Op], num_regs: usize) -> Vec<bool> {
    let mut s = vec![false; num_regs];
    let mut mark = |r: u16| {
        if (r as usize) < num_regs {
            s[r as usize] = true;
        }
    };
    for op in ops {
        match *op {
            Op::Show { src } | Op::Not { src, .. } => mark(src),
            _ => {}
        }
    }
    s
}

/// The message register of the first `NetSend` + the values register of the first `NetStream` in a
/// region. Offline networking is ONE local inbox FIFO, so an `Await`'s bound variable takes the sent
/// message's kind (a `NetSend`-to-`NetAwait` pairing the queue leaves implicit).
fn net_registers(ops: &[Op]) -> (Option<u16>, Option<u16>) {
    let mut msg = None;
    let mut stream = None;
    for op in ops {
        match *op {
            Op::NetSend { msg: m, .. } if msg.is_none() => msg = Some(m),
            Op::NetStream { values, .. } if stream.is_none() => stream = Some(values),
            _ => {}
        }
    }
    (msg, stream)
}

fn iter_pairing(ops: &[Op]) -> Vec<Option<usize>> {
    let prepares: Vec<usize> = ops
        .iter()
        .filter_map(|op| match *op {
            Op::IterPrepare { iterable } => Some(iterable as usize),
            _ => None,
        })
        .collect();
    let mut out = vec![None; ops.len()];
    let mut k = 0;
    for (pc, op) in ops.iter().enumerate() {
        if matches!(op, Op::IterNext { .. }) {
            out[pc] = prepares.get(k).copied();
            k += 1;
        }
    }
    out
}

/// Per-op struct layout from a forward pass over `ops` following `NewStruct`/`StructInsert`/
/// `GetField` and aliasing `Move`s. The runtime struct object is a flat array of 8-byte slots in
/// field-insertion order; this maps each field op to its slot (and a `GetField` to the register
/// whose value defines the field's kind), and each `NewStruct` to its field count (slot buffer
/// size). All indexed by op position.
#[derive(Default)]
pub(crate) struct StructLayout {
    pub(crate) slot: Vec<Option<u16>>,
    pub(crate) field_value: Vec<Option<u16>>,
    pub(crate) count: Vec<Option<u16>>,
    /// Per-`BindArm` op position → the register whose value defines the bound payload's kind (the
    /// inductive constructor's `index`-th argument register, traced from the matched `target`).
    /// Mirrors `field_value` for the enum-payload case. `None` where the target's construction is
    /// not statically known in this region.
    pub(crate) arg_value: Vec<Option<u16>>,
    /// Per-`CallValue` op position → the body function index of the closure being called (traced
    /// from the `callee` register's `MakeClosure`). `None` if the closure's construction is not
    /// statically known here; its result kind (hence the call's kind) is then unknown.
    pub(crate) callee_func: Vec<Option<u16>>,
    /// Per-`Index` op position → a value register stored into that map (the most recent `SetIndex`),
    /// whose kind is the map's value kind. Lets `item k of m` yield Float/Bool values, not only Int.
    /// (Meaningless for a sequence `Index`, which uses its element kind instead.)
    pub(crate) map_value: Vec<Option<u16>>,
    /// Per-`Index` op position → the register defining the accessed heterogeneous-tuple slot's
    /// element (its kind is the access's result kind), when the collection is a tuple and the index
    /// is a constant — the heterogeneous-tuple analog of `map_value`/`field_value`.
    pub(crate) tuple_value: Vec<Option<u16>>,
    /// Per-`GetField` op position → the RESOLVED kind of a field accessed on a CROSS-REGION struct
    /// (one returned by a `Call`/`CallValue`, whose field layout was threaded in resolved). Takes
    /// precedence over `field_value` (which is value-register-based, only valid intra-region).
    pub(crate) field_kind: Vec<Option<Kind>>,
    /// Per-`Index` op position → the resolved VALUE kind for a parameter-map access (`item k of m`
    /// where `m` is a `Map of K to V` parameter). The Kind-direct analog of `map_value` (which is
    /// register-based, only valid for a map constructed in this region); takes precedence over it.
    pub(crate) index_value_kind: Vec<Option<Kind>>,
    /// Per-`BindArm` op position → the resolved bound payload kind for an enum PARAMETER's
    /// `When V (binds)` extraction. The Kind-direct analog of `arg_value` (register-based, only valid
    /// for an enum constructed in this region); takes precedence over it.
    pub(crate) arg_kind: Vec<Option<Kind>>,
    /// Per struct-handle register, its inferred field layout `(field-name const idx, defining value
    /// register)` — the internal layout map, exposed so a callee's RETURN struct layout can be
    /// resolved (its field kinds) and threaded to callers as cross-region `field_kind`.
    pub(crate) reg_layout: std::collections::HashMap<u16, Vec<(u32, u16)>>,
    /// Per struct-handle register, its declared TYPE NAME (from the `NewStruct`'s `type_name`
    /// constant), aliased by `Move`/`DeepClone`. Lets a RETURN layout name each struct-typed field's
    /// type, so a caller can re-seed it and resolve `f(…)'s inner's v` cross-region — the call-result
    /// analog of the parameter path's `struct_types`-sourced names.
    pub(crate) struct_name_of: std::collections::HashMap<u16, String>,
    /// Per composite-handle register, its resolved access SHAPE — the unified view a consumer reads to
    /// type ANY register holding a struct/map/enum/het-tuple (a parameter's, a cross-region call
    /// result's, or a locally-built struct's), `Move`/`DeepClone`-aliased. Built from the per-shape
    /// tracks; lets a closure capturing a function-local/param composite re-use the captured value's
    /// resolution exactly as a parameter of that type would. (A locally-built map/enum/tuple is absent
    /// — its shape needs register kinds this pass doesn't have.)
    pub(crate) reg_shape: std::collections::HashMap<u16, ParamShape>,
    /// Per map-handle register, the value register last `SetIndex`'d into it (its kind = the map's
    /// value kind) — for a LOCALLY-BUILT map, so a post-inference pass can complete `reg_shape`.
    pub(crate) map_set_value: std::collections::HashMap<u16, u16>,
    /// Per map-handle register, the KEY register last `SetIndex`'d into it (its kind = the map's key
    /// kind, Int or Text) — the key analog of `map_set_value`, so a whole-map `Show` of a locally-built
    /// map can render each key by its kind.
    pub(crate) map_set_key: std::collections::HashMap<u16, u16>,
    /// Per tuple-handle register, its element registers by position (from `NewTuple`) — for a
    /// locally-built heterogeneous tuple's positional kinds, resolved post-inference.
    pub(crate) tuple_layouts: std::collections::HashMap<u16, Vec<u16>>,
    /// Per inductive-handle register, its enum TYPE name (from `NewInductive`) — for a locally-built
    /// enum's variant layout, resolved post-inference via `resolve_enum_variants`.
    pub(crate) ind_type_of: std::collections::HashMap<u16, String>,
    /// Per `Seq of Enum` handle register, its ELEMENT enum type name — from the first element at
    /// `NewList`/`ListPush` (a homogeneous enum list), so a whole `Seq of Enum` `Show` can resolve the
    /// element variant set for the per-element tag→name dispatch.
    pub(crate) seq_elem_ind_type: std::collections::HashMap<u16, String>,
    /// Per `Seq of Struct` handle register, its ELEMENT struct type name — the struct analog of
    /// `seq_elem_ind_type`, so a whole `Seq of Struct` `Show` can resolve the declared field layout.
    pub(crate) seq_elem_struct_name: std::collections::HashMap<u16, String>,
    /// Per closure-handle register → its body function index, `Move`/`DeepClone`-aliased, AND seeded
    /// from a `Call` whose callee returns a known closure (`fn_return_closure`). Exposed so a function
    /// that `Return`s a closure can publish WHICH closure it returns — letting a caller `f(args)` on
    /// the returned handle resolve its callee like a locally-built one (closures as return values).
    pub(crate) closure_of: std::collections::HashMap<u16, u16>,
    /// Per `Optional`-handle register, the present-value inner scalar kind (Int/Float/Bool/Char/Text)
    /// — from the producing channel's element kind at `ChanTryRecv`, `Move`-aliased. Lets `Show` of an
    /// `Optional` load + print the boxed inner value in the `Some` arm (the null-handle arm prints
    /// "nothing"). Absent ⇒ default `Int` (the common `Pipe of Int` case).
    pub(crate) opt_inner: std::collections::HashMap<u16, Kind>,
}

/// How a `GetField` on a composite-typed field re-seeds its RESULT register's own resolution, so a
/// chained access (`p's inner's v`, `item k of p's m`, `Inspect p's e`) resolves cross-region. The
/// struct case is by NAME (re-resolved lazily, so a recursive struct type terminates); the map/enum
/// cases carry what the access needs directly.
#[derive(Clone)]
pub(crate) enum FieldNested {
    None,
    /// A struct field: re-seed the result's field layout via [`resolve_named_layout`] of this name.
    Struct(String),
    /// A `Map of K to V` field: re-seed the result's map value kind.
    Map(Kind),
    /// An enum field: re-seed the result's per-variant payload layout via [`resolve_enum_variants`].
    Enum(String),
    /// A heterogeneous-tuple field: re-seed the result's per-position element kinds.
    Tuple(Vec<Kind>),
}

/// A struct's field layout resolved cross-region: per field, `(field-name const idx, kind, how a
/// `GetField` on it re-seeds the result)`, in slot order. See [`FieldNested`].
pub(crate) type FieldLayout = Vec<(u32, Kind, FieldNested)>;

/// Resolve an enum type's per-variant payload layout (variant name → payload field kinds by
/// position) from the bytecode's `enum_types`. `None` if any variant field has no single AOT kind.
pub(crate) fn resolve_enum_variants(
    name: &str,
    enum_types: &[EnumTypeDef],
) -> Option<std::collections::HashMap<String, Vec<Kind>>> {
    let def = enum_types.iter().find(|e| e.name == name)?;
    let mut variants = std::collections::HashMap::new();
    for v in &def.variants {
        let kinds: Option<Vec<Kind>> = v.field_types.iter().map(boundary_to_kind).collect();
        variants.insert(v.name.clone(), kinds?);
    }
    Some(variants)
}

/// What a non-scalar PARAMETER seeds into the layout pass so its accesses resolve cross-region: a
/// struct's field layout (for `p's field`), or a map's VALUE kind (for `item k of m`). Tuple
/// parameters need no shape — a homogeneous tuple resolves through the seq element-kind path.
#[derive(Clone)]
pub(crate) enum ParamShape {
    Struct(FieldLayout),
    Map(Kind),
    /// An enum parameter's per-variant payload layout: variant name → the kinds of its payload
    /// fields by position. Lets a `When V (binds)` `BindArm` resolve the bound payload's kind.
    Enum(std::collections::HashMap<String, Vec<Kind>>),
    /// A HETEROGENEOUS tuple parameter's per-position element kinds, so a constant `item N of t`
    /// resolves its result kind.
    Tuple(Vec<Kind>),
}

/// Resolve a struct type's [`FieldLayout`] from the bytecode's `struct_types`: each field's NAME
/// constant index (the same one the body's `GetField` carries) paired with its [`Kind`] and, for a
/// struct-typed field, the nested type's name (for lazy deeper resolution). `None` if any field has
/// no single AOT kind (a nested seq-of-struct, …), leaving such a struct parameter/return rejected.
/// Resolution is ONE level — deeper layouts are re-resolved on demand at each `GetField`, so a
/// recursive struct type terminates.
pub(crate) fn resolve_named_layout(
    name: &str,
    struct_types: &[StructTypeDef],
    constants: &[Constant],
) -> Option<FieldLayout> {
    let def = struct_types.iter().find(|s| s.name == name)?;
    let mut layout = Vec::with_capacity(def.fields.len());
    for (fname, ftype) in &def.fields {
        let kind = boundary_to_kind(ftype)?;
        // How a `GetField` on this field re-seeds the result, by the field's composite type: a struct
        // re-resolves by name (lazy/recursion-safe), a map carries its value kind, an enum its name.
        let nested = match ftype {
            BoundaryType::Struct(n) => FieldNested::Struct(n.clone()),
            BoundaryType::Map(_, v) => match boundary_to_kind(v) {
                Some(vk) => FieldNested::Map(vk),
                None => FieldNested::None,
            },
            BoundaryType::Enum(n) => FieldNested::Enum(n.clone()),
            BoundaryType::Tuple(elems) => match elems.iter().map(boundary_to_kind).collect::<Option<Vec<_>>>() {
                Some(ks) => FieldNested::Tuple(ks),
                None => FieldNested::None,
            },
            _ => FieldNested::None,
        };
        let const_idx = constants
            .iter()
            .position(|c| matches!(c, Constant::Text(t) if t == fname))
            .map_or(u32::MAX, |p| p as u32);
        layout.push((const_idx, kind, nested));
    }
    Some(layout)
}

pub(crate) fn struct_layout(
    ops: &[Op],
    constants: &[Constant],
    struct_types: &[StructTypeDef],
    enum_types: &[EnumTypeDef],
    fn_return_types: &[Option<BoundaryType>],
    ret_layout: &dyn Fn(u16) -> Option<FieldLayout>,
    fn_return_closure: &dyn Fn(u16) -> Option<u16>,
    param_layouts: &[(u16, ParamShape)],
    param_closures: &[(u16, u16)],
) -> StructLayout {
    let n = ops.len();
    let mut slot = vec![None; n];
    let mut field_value = vec![None; n];
    let mut count = vec![None; n];
    let mut arg_value = vec![None; n];
    let mut callee_func = vec![None; n];
    let mut map_value = vec![None; n];
    let mut tuple_value = vec![None; n];
    let mut field_kind = vec![None; n];
    // Per register, the RESOLVED field layout of a struct returned by a call (its kinds known from
    // the callee). Move-aliased; read at `GetField` to set `field_kind` for cross-region access.
    let mut call_result: std::collections::HashMap<u16, FieldLayout> = std::collections::HashMap::new();
    // Per map-handle register, its VALUE kind — for a map PARAMETER (seeded below) so `item k of m`
    // resolves cross-region; Move-aliased like the other layouts.
    let mut map_param_value: std::collections::HashMap<u16, Kind> = std::collections::HashMap::new();
    // Per enum-handle register, its per-variant payload layout — for an enum PARAMETER (seeded below)
    // so a `When V (binds)` `BindArm` resolves the bound payload kind; Move-aliased.
    let mut enum_param_variants: std::collections::HashMap<u16, std::collections::HashMap<String, Vec<Kind>>> =
        std::collections::HashMap::new();
    // Per tuple-handle register, its per-position element kinds — for a heterogeneous tuple PARAMETER
    // or call return (seeded below) so a constant `item N of t` resolves its result kind; Move-aliased.
    let mut tuple_param_kinds: std::collections::HashMap<u16, Vec<Kind>> = std::collections::HashMap::new();
    // A non-scalar PARAMETER seeds its access-resolution shape: a struct's field layout (seeded into
    // `call_result` exactly like a call result, so `param's field` resolves), a map's value kind, or
    // an enum's per-variant payload layout.
    for (reg, shape) in param_layouts {
        match shape {
            ParamShape::Struct(layout) => {
                call_result.insert(*reg, layout.clone());
            }
            ParamShape::Map(value_kind) => {
                map_param_value.insert(*reg, *value_kind);
            }
            ParamShape::Enum(variants) => {
                enum_param_variants.insert(*reg, variants.clone());
            }
            ParamShape::Tuple(kinds) => {
                tuple_param_kinds.insert(*reg, kinds.clone());
            }
        }
    }
    // Per inspected enum register → the variant name of the most recent `TestArm` on it; in the
    // structured `Inspect` emission a `BindArm` always sits in the block its `TestArm` guards, so
    // this names the bind's variant (last-write-wins, the sum-type analog of the construct-then-
    // inspect model the local `ind_layouts` path uses).
    let mut arm_variant: std::collections::HashMap<u16, String> = std::collections::HashMap::new();
    // Per `Index` op position → the resolved VALUE kind for a parameter-map / cross-region access
    // (the Kind-direct analog of `map_value`'s register-based resolution). Takes precedence.
    let mut index_value_kind = vec![None; n];
    // Per `BindArm` op position → the resolved bound payload kind for an enum PARAMETER's `When V
    // (binds)` extraction (the Kind-direct analog of `arg_value`'s register-based resolution).
    let mut arg_kind = vec![None; n];
    // Per map-handle register, a value register stored into it (last `SetIndex` wins). Read at each
    // `Index` to resolve the map's value kind. Aliased by `Move` like the other layouts.
    let mut map_set_value: std::collections::HashMap<u16, u16> = std::collections::HashMap::new();
    let mut map_set_key: std::collections::HashMap<u16, u16> = std::collections::HashMap::new();
    // Per tuple-handle register, its element registers by position (from `NewTuple`). Plus the
    // constant value held by each Int-`LoadConst` register, to resolve a constant tuple index.
    let mut tuple_layouts: std::collections::HashMap<u16, Vec<u16>> = std::collections::HashMap::new();
    let mut const_int: std::collections::HashMap<u16, i64> = std::collections::HashMap::new();
    // Per Seq-of-Struct handle register, the field layout of its (homogeneous) struct elements — so
    // an `Index` can flow that layout to the extracted struct (making `(item N of xs)'s field` work).
    let mut seq_elem_layout: std::collections::HashMap<u16, Vec<(u32, u16)>> = std::collections::HashMap::new();
    let mut layouts: std::collections::HashMap<u16, Vec<(u32, u16)>> = std::collections::HashMap::new();
    let mut origin: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
    // Per inductive-handle register, the constructor argument registers by position (`index`). Like
    // `layouts`, last-write-wins as the forward pass reaches each `BindArm` — sound for the
    // structured Inspect pattern (construct-then-inspect), the same model the struct path uses.
    let mut ind_layouts: std::collections::HashMap<u16, Vec<u16>> = std::collections::HashMap::new();
    // Per closure-handle register, the body function index (from its `MakeClosure`). Same
    // last-write-wins/Move-aliasing model — sound for the bind-then-call pattern. Pre-seeded with the
    // closure PARAMETERS whose single statically-known origin a whole-program pass resolved, so a
    // `CallValue` on a closure ARGUMENT traces its callee exactly like a local one (closures as args).
    let mut closure_of: std::collections::HashMap<u16, u16> = std::collections::HashMap::new();
    for &(reg, func) in param_closures {
        closure_of.insert(reg, func);
    }
    // Per channel-handle register, its element scalar kind (from `ChanNew`'s `ChanElem`), Move-aliased.
    // Seeds `opt_inner` at a `ChanTryRecv`: the Optional's present inner kind is the channel's element.
    let mut chan_elem: std::collections::HashMap<u16, Kind> = std::collections::HashMap::new();
    let mut opt_inner: std::collections::HashMap<u16, Kind> = std::collections::HashMap::new();
    // Per struct-handle register, its declared type name (from `NewStruct`'s `type_name` constant).
    // Move/DeepClone-aliased like the layouts; read by `fn_return_struct_layout` to name a returned
    // struct's struct-typed fields (so a caller resolves `f(…)'s inner's v`).
    let mut struct_name_of: std::collections::HashMap<u16, String> = std::collections::HashMap::new();
    // Per inductive-handle register, its declared ENUM type name (from `NewInductive`'s `type_name`
    // constant), Move/DeepClone-aliased. Lets a captured locally-built enum resolve its variant layout.
    let mut ind_type_of: std::collections::HashMap<u16, String> = std::collections::HashMap::new();
    let mut seq_elem_ind_type: std::collections::HashMap<u16, String> = std::collections::HashMap::new();
    let mut seq_elem_struct_name: std::collections::HashMap<u16, String> = std::collections::HashMap::new();
    // `IterNext` pc → its iterable register, so iterating a Seq-of-Struct can flow the element
    // layout to the loop variable (the `Index` flow's analog for `Repeat for p in …`).
    let iter_iterable = iter_pairing(ops);
    for (pc, op) in ops.iter().enumerate() {
        match *op {
            Op::NewStruct { dst, type_name } => {
                layouts.insert(dst, Vec::new());
                origin.insert(dst, pc);
                count[pc] = Some(0);
                if let Some(Constant::Text(t)) = constants.get(type_name as usize) {
                    struct_name_of.insert(dst, t.clone());
                }
            }
            Op::StructInsert { obj, field, value } => {
                let layout = layouts.entry(obj).or_default();
                let s = match layout.iter().position(|(f, _)| *f == field) {
                    Some(i) => {
                        layout[i].1 = value;
                        i
                    }
                    None => {
                        layout.push((field, value));
                        layout.len() - 1
                    }
                };
                slot[pc] = Some(s as u16);
                let len = layout.len() as u16;
                if let Some(&op_pc) = origin.get(&obj) {
                    count[op_pc] = Some(len);
                }
            }
            Op::NewInductive { dst, type_name, args_start, count: c, .. } => {
                ind_layouts.insert(dst, (0..c).map(|k| args_start + k).collect());
                if let Some(Constant::Text(t)) = constants.get(type_name as usize) {
                    ind_type_of.insert(dst, t.clone());
                }
            }
            Op::NewTuple { dst, start, count: c } => {
                tuple_layouts.insert(dst, (0..c).map(|k| start + k).collect());
                if c > 0 {
                    if let Some(l) = layouts.get(&start).cloned() {
                        seq_elem_layout.insert(dst, l);
                    }
                }
            }
            // A list literal `[s0, s1, …]` of structs is a Seq-of-Struct; its element layout is the
            // first element's struct layout (the elements are a homogeneous struct type).
            Op::NewList { dst, start, count: c } => {
                if c > 0 {
                    if let Some(l) = layouts.get(&start).cloned() {
                        seq_elem_layout.insert(dst, l);
                    }
                    // A homogeneous `Seq of Enum` literal — the element enum type is the first element.s.
                    if let Some(t) = ind_type_of.get(&start).cloned() {
                        seq_elem_ind_type.insert(dst, t);
                    }
                    if let Some(t) = struct_name_of.get(&start).cloned() {
                        seq_elem_struct_name.insert(dst, t);
                    }
                }
            }
            Op::ListPush { list, value } => {
                if let Some(l) = layouts.get(&value).cloned() {
                    seq_elem_layout.insert(list, l);
                }
                if let Some(t) = ind_type_of.get(&value).cloned() {
                    seq_elem_ind_type.insert(list, t);
                }
                if let Some(t) = struct_name_of.get(&value).cloned() {
                    seq_elem_struct_name.insert(list, t);
                }
            }
            // A popped element inherits the sequence's element layout (so `(popped)'s field` works
            // for a `Seq of Struct` pop).
            Op::ListPop { list, dst } => {
                if let Some(l) = seq_elem_layout.get(&list).cloned() {
                    layouts.insert(dst, l);
                }
            }
            Op::LoadConst { dst, idx } => {
                if let Some(Constant::Int(v)) = constants.get(idx as usize) {
                    const_int.insert(dst, *v);
                }
            }
            Op::MakeClosure { dst, func, .. } => {
                closure_of.insert(dst, func);
            }
            // A struct/inductive/closure handle copied to another register aliases the same layout.
            Op::Move { dst, src } => {
                if let Some(l) = layouts.get(&src).cloned() {
                    layouts.insert(dst, l);
                    if let Some(&o) = origin.get(&src) {
                        origin.insert(dst, o);
                    }
                }
                if let Some(l) = ind_layouts.get(&src).cloned() {
                    ind_layouts.insert(dst, l);
                }
                if let Some(&c) = closure_of.get(&src) {
                    closure_of.insert(dst, c);
                }
                if let Some(&v) = map_set_value.get(&src) {
                    map_set_value.insert(dst, v);
                }
                if let Some(&k) = map_set_key.get(&src) {
                    map_set_key.insert(dst, k);
                }
                if let Some(l) = tuple_layouts.get(&src).cloned() {
                    tuple_layouts.insert(dst, l);
                }
                if let Some(l) = seq_elem_layout.get(&src).cloned() {
                    seq_elem_layout.insert(dst, l);
                }
                if let Some(t) = seq_elem_ind_type.get(&src).cloned() {
                    seq_elem_ind_type.insert(dst, t);
                }
                if let Some(t) = seq_elem_struct_name.get(&src).cloned() {
                    seq_elem_struct_name.insert(dst, t);
                }
                if let Some(l) = call_result.get(&src).cloned() {
                    call_result.insert(dst, l);
                }
                if let Some(n) = struct_name_of.get(&src).cloned() {
                    struct_name_of.insert(dst, n);
                }
                if let Some(n) = ind_type_of.get(&src).cloned() {
                    ind_type_of.insert(dst, n);
                }
                if let Some(&vk) = map_param_value.get(&src) {
                    map_param_value.insert(dst, vk);
                }
                if let Some(v) = enum_param_variants.get(&src).cloned() {
                    enum_param_variants.insert(dst, v);
                }
                if let Some(v) = tuple_param_kinds.get(&src).cloned() {
                    tuple_param_kinds.insert(dst, v);
                }
                if let Some(&e) = chan_elem.get(&src) {
                    chan_elem.insert(dst, e);
                }
                if let Some(&i) = opt_inner.get(&src) {
                    opt_inner.insert(dst, i);
                }
            }
            // A channel's element scalar kind is statically evident from `ChanNew`'s `ChanElem`; record
            // it so a later `ChanTryRecv` on this channel can type its `Optional` result's inner value.
            Op::ChanNew { dst, elem, .. } => {
                let k = match elem {
                    ChanElem::Int => Kind::Int,
                    ChanElem::Bool => Kind::Bool,
                    ChanElem::Float => Kind::Float,
                    ChanElem::Text => Kind::Text,
                    ChanElem::Unknown => Kind::Int,
                };
                chan_elem.insert(dst, k);
            }
            // `Try to receive`'s `Optional` result carries the channel's element kind as its present-value
            // inner kind (defaulting to `Int` for an unseen/untyped channel).
            Op::ChanTryRecv { dst, chan } => {
                opt_inner.insert(dst, chan_elem.get(&chan).copied().unwrap_or(Kind::Int));
            }
            Op::SetIndex { collection, index, value } | Op::SetIndexUnchecked { collection, index, value } => {
                map_set_value.insert(collection, value);
                map_set_key.insert(collection, index);
            }
            Op::Index { dst, collection, index } | Op::IndexUnchecked { dst, collection, index } => {
                map_value[pc] = map_set_value.get(&collection).copied();
                // A parameter map's value kind resolves the access directly (no `SetIndex` register).
                if let Some(&vk) = map_param_value.get(&collection) {
                    index_value_kind[pc] = Some(vk);
                }
                // A parameter / call-return heterogeneous tuple resolves the CONSTANT position's kind
                // directly (no `NewTuple` register), the Kind-direct analog of `tuple_value`.
                if let (Some(ks), Some(&iv)) = (tuple_param_kinds.get(&collection), const_int.get(&index)) {
                    if iv >= 1 {
                        index_value_kind[pc] = ks.get((iv - 1) as usize).copied();
                    }
                }
                // Heterogeneous-tuple access: a constant index into a known tuple resolves to a
                // static slot + the register defining that slot's element kind.
                if let (Some(elems), Some(&iv)) = (tuple_layouts.get(&collection), const_int.get(&index)) {
                    if iv >= 1 && (iv as usize) <= elems.len() {
                        tuple_value[pc] = Some(elems[(iv - 1) as usize]);
                    }
                }
                // Seq-of-Struct access: the extracted struct inherits the sequence's element layout,
                // so a subsequent `GetField` resolves its slot/kind.
                if let Some(l) = seq_elem_layout.get(&collection).cloned() {
                    layouts.insert(dst, l);
                }
            }
            Op::GetField { dst, obj, field } => {
                let resolved = layouts
                    .get(&obj)
                    .and_then(|layout| layout.iter().position(|(f, _)| *f == field).map(|i| (i, layout[i].1)));
                if let Some((i, value_reg)) = resolved {
                    slot[pc] = Some(i as u16);
                    field_value[pc] = Some(value_reg);
                    // Nested struct: a field that is itself a struct flows its layout to the result,
                    // so `b's corner's x` resolves. Likewise a field holding a Seq-of-Struct.
                    if let Some(inner) = layouts.get(&value_reg).cloned() {
                        layouts.insert(dst, inner);
                    }
                    if let Some(inner) = seq_elem_layout.get(&value_reg).cloned() {
                        seq_elem_layout.insert(dst, inner);
                    }
                    // A Map/enum/tuple field of a LOCALLY-BUILT struct re-seeds the result's value-kind /
                    // variant / tuple-positions from the struct's DECLARED type (the `value_reg` path above
                    // only carries struct + seq-of-struct layouts), so `item k of (s's mapfield)` /
                    // `Inspect s's enumfield` resolves — symmetric with the call-result/parameter path below.
                    if let Some(name) = struct_name_of.get(&obj) {
                        if let Some(named) = resolve_named_layout(name, struct_types, constants) {
                            if let Some((_, _, nested)) = named.iter().find(|(f, _, _)| *f == field) {
                                match nested {
                                    FieldNested::Map(vk) => {
                                        // A locally-built struct's Map field (incl. a `SharedMap` CRDT
                                        // field) is default-filled with a placeholder constant whose kind
                                        // is NOT `Map`, so the value-reg path (`struct_field_value`) types
                                        // this `GetField` wrong. The DECLARED field type is authoritative:
                                        // seed the result kind to `Map` so `Set s's m[k]` lowers to a map
                                        // insert and `s's m[k]` to a map get, not the sequence path.
                                        field_kind[pc] = Some(Kind::Map);
                                        map_param_value.insert(dst, *vk);
                                    }
                                    FieldNested::Enum(ename) => {
                                        if let Some(v) = resolve_enum_variants(ename, enum_types) {
                                            enum_param_variants.insert(dst, v);
                                        }
                                    }
                                    FieldNested::Tuple(ks) => {
                                        tuple_param_kinds.insert(dst, ks.clone());
                                    }
                                    FieldNested::Struct(_) | FieldNested::None => {}
                                }
                            }
                        }
                    }
                } else if let Some(layout) = call_result.get(&obj).cloned() {
                    // Cross-region: a struct returned by a call OR a struct parameter — resolve the
                    // field's slot + KIND (the defining register is in the callee's/caller's region,
                    // so a value_reg won't do).
                    if let Some(i) = layout.iter().position(|(f, _, _)| *f == field) {
                        slot[pc] = Some(i as u16);
                        field_kind[pc] = Some(layout[i].1);
                        // A composite-typed field re-seeds the result's OWN resolution, so the NEXT
                        // access on it resolves cross-region: a struct field re-resolves its layout
                        // lazily by name (recursion-safe); a map field its value kind; an enum field
                        // its per-variant payload layout.
                        match &layout[i].2 {
                            FieldNested::Struct(nested_name) => {
                                if let Some(nested) = resolve_named_layout(nested_name, struct_types, constants) {
                                    call_result.insert(dst, nested);
                                }
                            }
                            FieldNested::Map(vk) => {
                                map_param_value.insert(dst, *vk);
                            }
                            FieldNested::Enum(ename) => {
                                if let Some(variants) = resolve_enum_variants(ename, enum_types) {
                                    enum_param_variants.insert(dst, variants);
                                }
                            }
                            FieldNested::Tuple(ks) => {
                                tuple_param_kinds.insert(dst, ks.clone());
                            }
                            FieldNested::None => {}
                        }
                    }
                }
            }
            // Record the variant this arm matched, so a following `BindArm` on the same target can
            // name its variant for a parameter enum (whose construction isn't visible).
            Op::TestArm { target, variant, .. } => {
                if let Some(Constant::Text(v)) = constants.get(variant as usize) {
                    arm_variant.insert(target, v.clone());
                }
            }
            Op::BindArm { target, index, .. } => {
                if let Some(args) = ind_layouts.get(&target) {
                    arg_value[pc] = args.get(index as usize).copied();
                }
                // Parameter enum: resolve the bound payload kind from the seeded variant layout +
                // the arm's variant (the Kind-direct analog of `arg_value`'s register resolution).
                if let (Some(variants), Some(vname)) =
                    (enum_param_variants.get(&target), arm_variant.get(&target))
                {
                    if let Some(field_kinds) = variants.get(vname) {
                        arg_kind[pc] = field_kinds.get(index as usize).copied();
                    }
                }
            }
            Op::CallValue { dst, callee, .. } => {
                let func = closure_of.get(&callee).copied();
                callee_func[pc] = func;
                // The call result is a struct → record its (cross-region) resolved field layout.
                if let Some(l) = func.and_then(|f| ret_layout(f)) {
                    call_result.insert(dst, l);
                }
            }
            Op::Call { dst, func, .. } => {
                if let Some(l) = ret_layout(func) {
                    call_result.insert(dst, l);
                }
                // A call to a function that RETURNS a known closure makes its result that closure — so a
                // later `dst(args)` `CallValue` traces its callee through here, exactly as if `dst` were a
                // local `MakeClosure`. This is how closures-as-return-values resolve their callee/captures.
                if let Some(c) = fn_return_closure(func) {
                    closure_of.insert(dst, c);
                }
                // A call returning a map/enum seeds the result's value kind / variant layout from the
                // callee's declared return type, so `item k of f()` / `Inspect f()` resolve like the
                // matching parameter shape (the return-side analog of `param_seeds`).
                match fn_return_types.get(func as usize).and_then(|t| t.as_ref()) {
                    Some(BoundaryType::Map(_, v)) => {
                        if let Some(vk) = boundary_to_kind(v) {
                            map_param_value.insert(dst, vk);
                        }
                    }
                    Some(BoundaryType::Enum(name)) => {
                        if let Some(variants) = resolve_enum_variants(name, enum_types) {
                            enum_param_variants.insert(dst, variants);
                        }
                    }
                    Some(BoundaryType::Tuple(elems)) => {
                        let ks: Option<Vec<Kind>> = elems.iter().map(boundary_to_kind).collect();
                        if let Some(ks) = ks {
                            tuple_param_kinds.insert(dst, ks);
                        }
                    }
                    _ => {}
                }
            }
            // `a copy of p` (DeepClone): the clone has the same field layout as its source, so a
            // subsequent `clone's field` resolves.
            Op::DeepClone { dst, src } => {
                if let Some(l) = layouts.get(&src).cloned() {
                    layouts.insert(dst, l);
                }
                if let Some(l) = seq_elem_layout.get(&src).cloned() {
                    seq_elem_layout.insert(dst, l);
                }
                if let Some(n) = struct_name_of.get(&src).cloned() {
                    struct_name_of.insert(dst, n);
                }
                if let Some(n) = ind_type_of.get(&src).cloned() {
                    ind_type_of.insert(dst, n);
                }
            }
            // `Repeat for p in xs` over a Seq-of-Struct: the loop variable inherits the sequence's
            // element layout, so a `GetField` on it inside the body resolves (the `Index` analog).
            Op::IterNext { dst, .. } => {
                if let Some(iterable) = iter_iterable.get(pc).copied().flatten() {
                    if let Some(l) = seq_elem_layout.get(&(iterable as u16)).cloned() {
                        layouts.insert(dst, l);
                    }
                }
            }
            _ => {}
        }
    }
    // The unified per-register SHAPE — every register whose composite resolution is already known
    // here: a locally-built struct (by `NewStruct` type name), a parameter / cross-region struct (its
    // resolved field layout), and a parameter map/enum/het-tuple (their seeded value kind / variant
    // layout / positional kinds). All `Move`-aliased, so a closure capturing any of these reads the
    // captured register's shape directly. (A locally-built map/enum/tuple is omitted — its shape needs
    // register kinds unavailable in this pass.)
    let mut reg_shape: std::collections::HashMap<u16, ParamShape> = std::collections::HashMap::new();
    for (reg, name) in &struct_name_of {
        if let Some(layout) = resolve_named_layout(name, struct_types, constants) {
            reg_shape.insert(*reg, ParamShape::Struct(layout));
        }
    }
    for (reg, layout) in &call_result {
        reg_shape.insert(*reg, ParamShape::Struct(layout.clone()));
    }
    for (reg, vk) in &map_param_value {
        reg_shape.insert(*reg, ParamShape::Map(*vk));
    }
    for (reg, variants) in &enum_param_variants {
        reg_shape.insert(*reg, ParamShape::Enum(variants.clone()));
    }
    for (reg, ks) in &tuple_param_kinds {
        reg_shape.insert(*reg, ParamShape::Tuple(ks.clone()));
    }
    StructLayout { slot, field_value, count, arg_value, callee_func, map_value, tuple_value, field_kind, index_value_kind, arg_kind, reg_layout: layouts, struct_name_of, reg_shape, map_set_value, map_set_key, tuple_layouts, ind_type_of, seq_elem_ind_type, seq_elem_struct_name, closure_of, opt_inner }
}

/// Infer the kind of every register in `ops` (a region with region-local jump targets).
/// `seeds[r]` is register `r`'s declared kind (function parameters; empty/`None` for Main and
/// for non-parameter registers). `ret_of(func)` gives a callee's declared return kind, for
/// `Op::Call`. Errors with [`WasmLowerError::Unsupported`] on an irreconcilable kind conflict.
/// The registers a LINKED program should type as `Kind::BigInt` because their value is only ever
/// OBSERVED (`Show`n) or fed into further big-integer arithmetic — never read where an `i64` width is
/// required (a comparison, an index, a loop test, an `i64` call argument). Promoting exactly these makes
/// an OVERFLOWING pure-integer expression (`Show 99999999999 * 99999999999`) compute on real BigInts and
/// print the exact value — matching the VM's promote-on-overflow — while a bounded loop counter (read by
/// a compare) stays a fast `i64`. SOUND by construction: a register is promoted only when EVERY reader is
/// a `Show` or a still-promoted arithmetic op (the reader footprint comes from the exhaustive
/// [`super::regsplit::op_def_uses`], so no reader is ever missed), so a register any `i64`-typed op reads
/// is never promoted — at worst it keeps the trapping i64 path, never a miscompile. Empty unless `linked`.
pub(crate) fn bigint_demanded_regs(ops: &[Op], functions: &[CompiledFunction], linked: bool) -> std::collections::HashSet<Reg> {
    use std::collections::{HashMap, HashSet};
    let mut ok: HashSet<Reg> = HashSet::new();
    if !linked {
        return ok;
    }
    let is_arith = |op: &Op| matches!(op, Op::Add { .. } | Op::Sub { .. } | Op::Mul { .. } | Op::Div { .. } | Op::Mod { .. });
    let mut defs: Vec<Vec<Reg>> = Vec::with_capacity(ops.len());
    let mut readers: HashMap<Reg, Vec<usize>> = HashMap::new();
    let mut all_regs: HashSet<Reg> = HashSet::new();
    for (i, op) in ops.iter().enumerate() {
        let (d, u) = super::regsplit::op_def_uses(op, functions);
        for &r in &d {
            all_regs.insert(r);
        }
        for r in u {
            all_regs.insert(r);
            readers.entry(r).or_default().push(i);
        }
        defs.push(d);
    }
    // A "bigint-SAFE" greatest fixpoint over EVERY register: a register can hold a BigInt iff every reader
    // accepts one — a `Show`, an arith op whose own result stays safe (the BigInt flows through the
    // operation), or a `Move` whose destination stays safe (the BigInt flows through the copy). A register
    // any `i64`-typed op reads (a comparison, index, loop test, `i64` call argument) is UNSAFE. Starts
    // all-safe and removes violators until stable. Crossing `Move` is exactly what lets
    // `Let x be a*b. Show x.` — compiled `Mul T; Move x=T; Show x` — promote the product `T` (whose only
    // reader is the `Move`). SOUND: the reader footprint is the exhaustive [`super::regsplit::op_def_uses`],
    // so no reader is missed; an unsafe register is never promoted (at worst it keeps the trapping i64
    // path, never a miscompile).
    let mut safe: HashSet<Reg> = all_regs;
    loop {
        let mut changed = false;
        for r in safe.iter().copied().collect::<Vec<_>>() {
            let good = match readers.get(&r) {
                None => false, // a value nothing reads is dead — not worth promoting
                Some(rs) => rs.iter().all(|&i| match &ops[i] {
                    Op::Show { .. } => true,
                    Op::Move { dst, .. } => safe.contains(dst),
                    op if is_arith(op) => defs[i].iter().all(|d| safe.contains(d)),
                    _ => false,
                }),
            };
            if !good {
                safe.remove(&r);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // DEMAND = an arith RESULT that is safe AND whose EVERY writer is an arith op. A register ALSO written
    // by a non-arith op — a `LoadConst 0` loop-accumulator init, an `AddAssign`/`IterNext` — must keep its
    // declared kind (a single local can't be both an i64 and an i32 handle). A `Move` DESTINATION need not
    // be a demand candidate: it gets its BigInt via kind PROPAGATION from the safe source (`op_kind_effect`
    // `Op::Move` copies the source kind), so promoting the source suffices.
    let mut nonarith_written: HashSet<Reg> = HashSet::new();
    for (i, op) in ops.iter().enumerate() {
        if !is_arith(op) {
            for &d in &defs[i] {
                nonarith_written.insert(d);
            }
        }
    }
    for (i, op) in ops.iter().enumerate() {
        if is_arith(op) {
            for &d in &defs[i] {
                if safe.contains(&d) && !nonarith_written.contains(&d) {
                    ok.insert(d);
                }
            }
        }
    }
    ok
}

pub(crate) fn infer(
    ops: &[Op],
    constants: &[Constant],
    struct_types: &[StructTypeDef],
    enum_types: &[EnumTypeDef],
    fn_return_types: &[Option<BoundaryType>],
    num_regs: usize,
    seeds: &[Option<Kind>],
    ret_of: &dyn Fn(usize) -> Option<Kind>,
    global_of: &dyn Fn(u16) -> Option<Kind>,
    closure_ret: &dyn Fn(usize) -> Option<Kind>,
    ret_layout: &dyn Fn(u16) -> Option<FieldLayout>,
    fn_return_closure: &dyn Fn(u16) -> Option<u16>,
    param_layouts: &[(u16, ParamShape)],
    param_closures: &[(u16, u16)],
    reachable: &[bool],
    linked: bool,
    bigint_demand: &std::collections::HashSet<Reg>,
) -> Result<KindTable, WasmLowerError> {
    let mut kinds = vec![None; num_regs];
    for (r, &seed) in seeds.iter().enumerate() {
        kinds[r] = seed;
    }
    let iter_iterable = iter_pairing(ops);
    let label_sensitive = label_sensitive_regs(ops, num_regs);
    let sl = struct_layout(ops, constants, struct_types, enum_types, fn_return_types, ret_layout, fn_return_closure, param_layouts, param_closures);
    let env = Env { constants, ret_of, global_of, iter_iterable: &iter_iterable, struct_field_value: &sl.field_value, bind_arm_value: &sl.arg_value, callee_func: &sl.callee_func, closure_ret, map_value: &sl.map_value, tuple_value: &sl.tuple_value, field_kind: &sl.field_kind, index_value_kind: &sl.index_value_kind, arg_kind: &sl.arg_kind, net_msg: net_registers(ops).0, net_stream: net_registers(ops).1, linked, bigint_demand };
    loop {
        let mut changed = false;
        for (pc, op) in ops.iter().enumerate() {
            // Skip statically-unreachable ops: their writes (e.g. the dead logical branch of a
            // monomorphized `and`/`or`) must not poison a register's kind.
            if !reachable[pc] {
                continue;
            }
            if let Some((r, k)) = op_kind_effect(op, pc, &kinds, &env)? {
                unify_strict(&mut kinds, &mut changed, r, k, &label_sensitive)?;
            }
            // `Let (a, b) be t` — a MULTI-target op: each destructured register takes the source
            // tuple's positional element kind. A heterogeneous tuple's positions come from
            // `tuple_layouts` (the element registers that filled it); a homogeneous one rides a `SeqX`
            // whose `seq_elem` is every position's kind.
            if let Op::DestructureTuple { src, start, count } = *op {
                for i in 0..count {
                    let k = sl
                        .tuple_layouts
                        .get(&src)
                        .and_then(|es| es.get(i as usize))
                        .and_then(|&e| kinds[e as usize])
                        .or_else(|| kinds[src as usize].and_then(Kind::seq_elem));
                    if let Some(k) = k {
                        unify_strict(&mut kinds, &mut changed, (start + i) as usize, Some(k), &label_sensitive)?;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    Ok(KindTable { kinds })
}

/// The cross-function inputs `op_kind_effect` needs.
struct Env<'a> {
    constants: &'a [Constant],
    ret_of: &'a dyn Fn(usize) -> Option<Kind>,
    global_of: &'a dyn Fn(u16) -> Option<Kind>,
    /// `IterNext`→iterable-register pairing (see [`iter_pairing`]), indexed by op position.
    iter_iterable: &'a [Option<usize>],
    /// `GetField`→defining-value-register map (see [`struct_layout`]), indexed by op position; the
    /// field's kind is that register's kind.
    struct_field_value: &'a [Option<u16>],
    /// `BindArm`→defining-value-register map (the enum-payload analog of `struct_field_value`).
    bind_arm_value: &'a [Option<u16>],
    /// `CallValue`→callee body-function-index map (see [`struct_layout`]); the call's result kind
    /// is that function's result kind, via [`Env::closure_ret`].
    callee_func: &'a [Option<u16>],
    /// Inferred result kind of a (closure-body) function, by index — for `CallValue`.
    closure_ret: &'a dyn Fn(usize) -> Option<Kind>,
    /// `Index`→inserted-value-register map (see [`struct_layout`]); a map `Index`'s result kind is
    /// that register's kind (the map's value kind).
    map_value: &'a [Option<u16>],
    /// `Index`→tuple-slot-defining-register map; a heterogeneous-tuple `Index`'s result kind is
    /// that register's kind (the element at the constant position).
    tuple_value: &'a [Option<u16>],
    /// `GetField`→resolved field kind for a cross-region (call-result) struct access (see
    /// [`struct_layout`]); takes precedence over `struct_field_value`.
    field_kind: &'a [Option<Kind>],
    /// `Index`→resolved value kind for a parameter-map access (see [`struct_layout`]); takes
    /// precedence over `map_value`'s register-based resolution.
    index_value_kind: &'a [Option<Kind>],
    /// `BindArm`→resolved bound payload kind for a parameter-enum extraction (see [`struct_layout`]);
    /// takes precedence over `bind_arm_value`'s register-based resolution.
    arg_kind: &'a [Option<Kind>],
    /// The message register of a `NetSend` / values register of a `NetStream` in this region — a
    /// single offline inbox FIFO, so an `Await`'s bound variable takes the message's / stream-list's
    /// kind (`NetAwait` is not connected to its `NetSend` by a register; the queue is implicit).
    net_msg: Option<u16>,
    net_stream: Option<u16>,
    /// Linker mode: an `Op::Pow` of two Ints resolves to a `Text` handle (the exact big-integer
    /// decimal from the linked `logicaffeine_base::BigInt` runtime) rather than a trapping i64 power,
    /// so an overflowing power computes the same value the VM's BigInt promotion does. Only ever set by
    /// [`super::module::assemble_program_linked`]; the standalone path leaves it `false` (unchanged).
    linked: bool,
    /// Linker mode: registers to type `Kind::BigInt` because their value is only observed or fed into
    /// more BigInt arithmetic (see [`bigint_demanded_regs`]) — general Int-overflow→BigInt. Empty unless
    /// `linked`.
    bigint_demand: &'a std::collections::HashSet<Reg>,
}

/// The kind one op assigns to one register, given the current kinds — the single source of truth
/// both the strict and the lenient pass share. `None` for an op with no scalar kind effect
/// (control flow, a non-scalar write, a store). The inner `Option<Kind>` is the (possibly still
/// unknown) kind to unify into the destination.
fn op_kind_effect(
    op: &Op,
    pc: usize,
    kinds: &[Option<Kind>],
    env: &Env,
) -> Result<Option<(usize, Option<Kind>)>, WasmLowerError> {
    Ok(match *op {
        Op::LoadConst { dst, idx } => Some((dst as usize, env.constants.get(idx as usize).and_then(const_kind))),
        Op::Move { dst, src } => Some((dst as usize, kinds[src as usize])),
        // `Await <task>` for its value passes the (synchronously-completed) task's handle register through
        // to `dst` — the handle's kind IS the result kind. (No compiler emit site produces this today.)
        Op::TaskAwait { dst, handle } => Some((dst as usize, kinds[handle as usize])),
        // `+` is runtime-polymorphic: a Text operand makes it string CONCATENATION (result Text),
        // anything else is numeric addition. `-`/`*`/`/`/`%` are numeric-only.
        Op::Add { dst, lhs, rhs } => {
            let (bk, ek) = (kinds[lhs as usize], kinds[rhs as usize]);
            // `+` on a `BigInt` operand (linked) is exact big-integer addition; an `Int` operand
            // promotes. BUT a `Text` operand makes `+` string CONCATENATION even with a BigInt on the
            // other side (`"x = " + (2^200)` → the decimal appended to the text, matching the VM), so a
            // Text operand vetoes the BigInt path and falls to `add_join`. Otherwise the
            // runtime-polymorphic `add_join` (numeric add or Text concatenation).
            let text_operand = bk == Some(Kind::Text) || ek == Some(Kind::Text);
            let out = if env.linked && (bk == Some(Kind::Complex) || ek == Some(Kind::Complex)) {
                Some(Kind::Complex)
            } else if env.linked && (bk == Some(Kind::Modular) || ek == Some(Kind::Modular)) {
                Some(Kind::Modular)
            } else if env.linked && (bk == Some(Kind::Decimal) || ek == Some(Kind::Decimal)) {
                Some(Kind::Decimal) // `+` on a Decimal operand is exact base-10 addition (linked runtime)
            } else if env.linked && (bk == Some(Kind::Money) || ek == Some(Kind::Money)) {
                Some(Kind::Money) // `+` on a Money operand (matching currency)
            } else if env.linked && (bk == Some(Kind::Quantity) || ek == Some(Kind::Quantity)) {
                Some(Kind::Quantity) // `+` on a Quantity operand (matching dimension)
            } else if env.linked && !text_operand && (bk == Some(Kind::Rational) || ek == Some(Kind::Rational)) {
                Some(Kind::Rational) // `+` on a Rational operand (an Int/BigInt operand widens) stays exact
            } else if env.linked && bk == Some(Kind::Lanes) && ek == Some(Kind::Lanes) {
                Some(Kind::Lanes) // lane-wise `Lanes + Lanes` (the SHA-1 block fold)
            } else if bk == Some(Kind::Duration) && ek == Some(Kind::Duration) {
                Some(Kind::Duration) // `Duration + Duration = Duration` (i64 nanos add, shown formatted)
            } else if (bk == Some(Kind::Moment) && ek == Some(Kind::Duration))
                || (bk == Some(Kind::Duration) && ek == Some(Kind::Moment))
            {
                Some(Kind::Moment) // `Moment + Duration = Moment` (a later moment; commutes)
            } else if env.linked
                && ((bk == Some(Kind::Moment) && ek == Some(Kind::Span))
                    || (bk == Some(Kind::Span) && ek == Some(Kind::Moment)))
            {
                Some(Kind::Moment) // `Moment + Span` civil calendar arithmetic (linked; commutes)
            } else if env.linked
                && ((bk == Some(Kind::Date) && ek == Some(Kind::Span))
                    || (bk == Some(Kind::Span) && ek == Some(Kind::Date)))
            {
                Some(Kind::Date) // `Date + Span` civil calendar arithmetic (linked; commutes)
            } else if env.linked
                && !text_operand
                && (bk == Some(Kind::BigInt) || ek == Some(Kind::BigInt) || env.bigint_demand.contains(&dst))
            {
                Some(Kind::BigInt) // temporal clauses above win over the bigint_demand heuristic
            } else {
                add_join(bk, ek)?
            };
            Some((dst as usize, out))
        }
        // `/` and `%` on a `BigInt` operand (linked) are exact big-integer quotient / remainder (the same
        // `div_rem` the VM uses); an `Int` operand promotes. Otherwise the numeric i64/f64 result.
        Op::Div { dst, lhs, rhs } | Op::Mod { dst, lhs, rhs } => {
            let (bk, ek) = (kinds[lhs as usize], kinds[rhs as usize]);
            let out = if env.linked && (bk == Some(Kind::Quantity) || ek == Some(Kind::Quantity)) {
                Some(Kind::Quantity) // `÷` combines dimensions (or scales a Quantity by a scalar)
            } else if env.linked && (bk == Some(Kind::Rational) || ek == Some(Kind::Rational)) {
                // `÷` on a Rational operand is exact rational division (a bare `a / b` on two Rationals is a
                // plain `Div` the VM dispatches to the rational path — the type annotation only forces the
                // literal `7 / 2` to `ExactDiv`); it wins over the Int→BigInt overflow demand.
                Some(Kind::Rational)
            } else if env.linked && (bk == Some(Kind::BigInt) || ek == Some(Kind::BigInt) || env.bigint_demand.contains(&dst)) {
                Some(Kind::BigInt)
            } else {
                numeric_join(bk, ek)?
            };
            Some((dst as usize, out))
        }
        // `//` floor division: the numeric i64/f64 (or Word) result of the operands. The
        // standalone i64 path traps on the `i64::MIN // -1` overflow like `Div`; no linker
        // BigInt handle (floor division stays in-module).
        Op::FloorDiv { dst, lhs, rhs } => {
            Some((dst as usize, numeric_join(kinds[lhs as usize], kinds[rhs as usize])?))
        }
        // `-` on a `BigInt` operand (linked) is exact big-integer subtraction (a `BigInt` handle, which
        // may be negative — `to_text` renders the sign); an `Int` operand promotes. Else numeric i64/f64.
        Op::Sub { dst, lhs, rhs } => {
            let (bk, ek) = (kinds[lhs as usize], kinds[rhs as usize]);
            let out = if env.linked && (bk == Some(Kind::Complex) || ek == Some(Kind::Complex)) {
                Some(Kind::Complex)
            } else if env.linked && (bk == Some(Kind::Modular) || ek == Some(Kind::Modular)) {
                Some(Kind::Modular)
            } else if env.linked && (bk == Some(Kind::Decimal) || ek == Some(Kind::Decimal)) {
                Some(Kind::Decimal)
            } else if env.linked && (bk == Some(Kind::Money) || ek == Some(Kind::Money)) {
                Some(Kind::Money) // `-` on a Money operand (matching currency)
            } else if env.linked && (bk == Some(Kind::Quantity) || ek == Some(Kind::Quantity)) {
                Some(Kind::Quantity) // `-` on a Quantity operand (matching dimension)
            } else if env.linked && (bk == Some(Kind::Rational) || ek == Some(Kind::Rational)) {
                Some(Kind::Rational) // `-` on a Rational operand (an Int/BigInt operand widens) stays exact
            } else if bk == Some(Kind::Duration) && ek == Some(Kind::Duration) {
                Some(Kind::Duration) // `Duration - Duration = Duration`
            } else if bk == Some(Kind::Moment) && ek == Some(Kind::Duration) {
                Some(Kind::Moment) // `Moment - Duration = Moment` (an earlier moment; Moment - Moment stays deferred, matching the VM)
            } else if env.linked && bk == Some(Kind::Moment) && ek == Some(Kind::Span) {
                Some(Kind::Moment) // `Moment - Span` steps the calendar backward (linked)
            } else if env.linked && bk == Some(Kind::Date) && ek == Some(Kind::Span) {
                Some(Kind::Date) // `Date - Span` steps the calendar backward (linked)
            } else if env.linked && (bk == Some(Kind::BigInt) || ek == Some(Kind::BigInt) || env.bigint_demand.contains(&dst)) {
                Some(Kind::BigInt) // temporal clauses above win over the bigint_demand heuristic
            } else {
                numeric_join(bk, ek)?
            };
            Some((dst as usize, out))
        }
        // In LINKER mode an integer power resolves to a `BigInt` HANDLE — the exact big integer the real
        // `logicaffeine_base::BigInt` runtime computes (`logos_rt_bigint_from_i64`→`_pow`), so an
        // overflowing `x to the power of y` yields the same value the VM's BigInt promotion prints
        // rather than trapping. The handle stays a handle (rendered to a decimal `Text` only at `Show`),
        // so a chain `(2^100) * (3^50)` keeps multiplying real BigInts. Standalone keeps the i64 power.
        Op::Pow { dst, lhs, rhs } => {
            let (bk, ek) = (kinds[lhs as usize], kinds[rhs as usize]);
            let out = if env.linked && bk == Some(Kind::Int) && ek == Some(Kind::Int) {
                Some(Kind::BigInt)
            } else {
                numeric_join(bk, ek)?
            };
            Some((dst as usize, out))
        }
        // `*` on a `BigInt` operand (linked) is exact big-integer multiplication (a `BigInt` handle);
        // an `Int` operand promotes to a BigInt at lowering. Otherwise the numeric i64/f64 product.
        Op::Mul { dst, lhs, rhs } => {
            let (bk, ek) = (kinds[lhs as usize], kinds[rhs as usize]);
            let out = if env.linked && (bk == Some(Kind::Complex) || ek == Some(Kind::Complex)) {
                Some(Kind::Complex)
            } else if env.linked && (bk == Some(Kind::Modular) || ek == Some(Kind::Modular)) {
                Some(Kind::Modular)
            } else if env.linked && (bk == Some(Kind::Decimal) || ek == Some(Kind::Decimal)) {
                Some(Kind::Decimal)
            } else if env.linked && (bk == Some(Kind::Quantity) || ek == Some(Kind::Quantity)) {
                Some(Kind::Quantity) // `×` combines dimensions (or scales a Quantity by a scalar)
            } else if env.linked && (bk == Some(Kind::Rational) || ek == Some(Kind::Rational)) {
                Some(Kind::Rational) // `×` on a Rational operand (an Int/BigInt operand widens) stays exact
            } else if env.linked && (bk == Some(Kind::BigInt) || ek == Some(Kind::BigInt) || env.bigint_demand.contains(&dst)) {
                Some(Kind::BigInt)
            } else {
                numeric_join(bk, ek)?
            };
            Some((dst as usize, out))
        }
        Op::AddAssign { dst, src } => Some((dst as usize, add_join(kinds[dst as usize], kinds[src as usize])?)),
        Op::DivPow2 { dst, .. } | Op::MagicDivU { dst, .. } => Some((dst as usize, Some(Kind::Int))),
        // `a / b` in a `Rational` context (`ExactDiv`) yields an exact `Rational` handle.
        Op::ExactDiv { dst, .. } => Some((dst as usize, Some(Kind::Rational))),
        Op::Shl { dst, .. } | Op::Shr { dst, .. } => Some((dst as usize, Some(Kind::Int))),
        // `^ & |` follow their operands: bitwise on Int, logical on Bool, set algebra on Set.
        Op::BitXor { dst, lhs, .. } | Op::BitAnd { dst, lhs, .. } | Op::BitOr { dst, lhs, .. } => {
            Some((dst as usize, kinds[lhs as usize]))
        }
        Op::Lt { dst, .. }
        | Op::Gt { dst, .. }
        | Op::LtEq { dst, .. }
        | Op::GtEq { dst, .. }
        | Op::Eq { dst, .. }
        | Op::NotEq { dst, .. }
        | Op::ApproxEq { dst, .. } => Some((dst as usize, Some(Kind::Bool))),
        // `not` is logical — truthiness in, Bool out (`~` lowers to `x ^ -1`).
        Op::Not { dst, .. } => Some((dst as usize, Some(Kind::Bool))),
        Op::Call { dst, func, .. } => Some((dst as usize, (env.ret_of)(func as usize))),
        Op::GlobalGet { dst, idx } => Some((dst as usize, (env.global_of)(idx))),
        Op::LoadToday { dst } => Some((dst as usize, Some(Kind::Date))),
        Op::LoadNow { dst } => Some((dst as usize, Some(Kind::Moment))),
        // A new empty sequence defaults to Int elements (the element kind is i32-handle-agnostic;
        // Float sequences are refined where pushes reveal them, in a later phase).
        // `NewEmptyListI32` is the optimizer's i32-narrowed empty list — observably identical to
        // `NewEmptyList` (the i32-fit is a memory-compaction hint, not a semantic change), so it takes
        // the same self-describing i64 seq path; the elements are Int, refined by `Push` like any list.
        Op::NewEmptyList { dst } | Op::NewEmptyListI32 { dst } => Some((dst as usize, Some(Kind::SeqAny))),
        // `Push value to seq` refines the sequence's element kind from the pushed value.
        Op::ListPush { list, value } => {
            let seq = match kinds.get(value as usize).copied().flatten() {
                Some(Kind::Int) => Some(Kind::SeqInt),
                // A `Seq of Bool` uses the SAME i64-0/1 storage as `SeqInt`, but keeps its Bool element
                // kind so element reads and the whole-sequence `Show` render `true`/`false`, not `1`/`0`.
                Some(Kind::Bool) => Some(Kind::SeqBool),
                Some(Kind::Float) => Some(Kind::SeqFloat),
                Some(Kind::Text) => Some(Kind::SeqText),
                Some(Kind::Struct) => Some(Kind::SeqStruct),
                Some(Kind::Enum) => Some(Kind::SeqEnum),
                Some(Kind::SeqInt) => Some(Kind::SeqSeqInt),
                Some(Kind::Word32) => Some(Kind::SeqWord32),
                Some(Kind::Word64) => Some(Kind::SeqWord64),
                _ => None,
            };
            Some((list as usize, seq))
        }
        // `Pop from seq into dst` yields the sequence's element (the last one removed).
        Op::ListPop { list, dst } => Some((dst as usize, kinds.get(list as usize).copied().flatten().and_then(Kind::seq_elem))),
        // A `Pipe`/channel used single-threaded is a FIFO queue: `ChanNew` is an empty seq, `Send`
        // refines its element kind (like `Push`), `Receive`/a `select` recv arm yields an element
        // (pop-front). The `Pipe of T` DECLARED element type seeds the queue's element kind up front,
        // so a channel that is never sent to (a timeout-only `select` over an empty `Pipe of Text`)
        // still types its recv-arm variable; a `Send` then only refines a consistent kind.
        Op::ChanNew { dst, elem, .. } => {
            let seq = match elem {
                ChanElem::Int | ChanElem::Bool => Some(Kind::SeqInt),
                ChanElem::Float => Some(Kind::SeqFloat),
                ChanElem::Text => Some(Kind::SeqText),
                ChanElem::Unknown => Some(Kind::SeqAny),
            };
            Some((dst as usize, seq))
        }
        Op::ChanSend { chan, val } => {
            let seq = match kinds.get(val as usize).copied().flatten() {
                Some(Kind::Int) | Some(Kind::Bool) => Some(Kind::SeqInt),
                Some(Kind::Float) => Some(Kind::SeqFloat),
                Some(Kind::Text) => Some(Kind::SeqText),
                _ => None,
            };
            Some((chan as usize, seq))
        }
        Op::ChanRecv { dst, chan } => Some((dst as usize, kinds.get(chan as usize).copied().flatten().and_then(Kind::seq_elem))),
        // Non-blocking `Try to receive` yields an `Optional`: the popped value (present) or `Nothing`
        // (empty). The present inner kind (for `Show`) is the channel's element kind, carried via
        // `StructLayout::opt_inner`; here we only fix the result register's kind to `Optional`.
        Op::ChanTryRecv { dst, .. } => Some((dst as usize, Some(Kind::Optional))),
        // `Try to send`'s result is the delivered/queued success flag — a `Bool`.
        Op::ChanTrySend { dst, .. } => Some((dst as usize, Some(Kind::Bool))),
        // A `select` recv arm (`Receive var from chan`) binds `var` to the channel's element kind (the
        // value delivered when this arm wins) — the pop-front is emitted by the winning `SelectWait`.
        Op::SelectArmRecv { chan, var } => Some((var as usize, kinds.get(chan as usize).copied().flatten().and_then(Kind::seq_elem))),
        // `SelectWait` writes the winning arm INDEX (an `Int` the following per-arm `Eq` dispatch reads).
        Op::SelectWait { dst_arm } => Some((dst_arm as usize, Some(Kind::Int))),
        // `Let job be Launch a task …` — the deterministic single-thread model runs the task
        // synchronously, so the handle is a dead `Int` dummy (only `Stop`/`Await` read it, as no-ops).
        Op::SpawnHandle { dst, .. } => Some((dst as usize, Some(Kind::Int))),
        // `a PeerAgent at <addr>` — locally the handle IS the address (its canonical topic Text).
        Op::NetMakePeer { dst, addr } => Some((dst as usize, kinds.get(addr as usize).copied().flatten())),
        // `Await [stream] from <peer> into <dst>` — the offline inbox loopback delivers the sent value,
        // so `dst` takes the `NetStream` values kind (stream) or the `NetSend` message kind.
        Op::NetAwait { dst, stream, .. } => {
            let src = if stream { env.net_stream } else { env.net_msg };
            Some((dst as usize, src.and_then(|r| kinds.get(r as usize).copied().flatten())))
        }
        Op::Length { dst, .. } => Some((dst as usize, Some(Kind::Int))),
        // `seq contains value` is a membership test → Bool.
        Op::Contains { dst, .. } => Some((dst as usize, Some(Kind::Bool))),
        // `items i through j of seq` is a subsequence of the same element kind.
        Op::SliceOp { dst, collection, .. } => Some((dst as usize, kinds.get(collection as usize).copied().flatten())),
        // `lhs followed by rhs` concatenates two sequences → the same element kind as lhs.
        Op::SeqConcat { dst, lhs, .. } => Some((dst as usize, kinds.get(lhs as usize).copied().flatten())),
        // `lhs + rhs` / interpolation concatenation always yields a Text.
        Op::Concat { dst, .. } => Some((dst as usize, Some(Kind::Text))),
        // A formatted interpolation piece (`"{x:.9}"`) is a fresh Text.
        Op::FormatValue { dst, .. } => Some((dst as usize, Some(Kind::Text))),
        // `a copy of x` clones to the same kind.
        Op::DeepClone { dst, src } => Some((dst as usize, kinds.get(src as usize).copied().flatten())),
        // A fresh struct handle.
        Op::NewStruct { dst, .. } => Some((dst as usize, Some(Kind::Struct))),
        // A fresh enum/inductive handle; `it is Variant` tests yield Bool.
        Op::NewInductive { dst, .. } => Some((dst as usize, Some(Kind::Enum))),
        Op::TestArm { dst, .. } => Some((dst as usize, Some(Kind::Bool))),
        // A closure value (`(x) -> …`). `f(x)` yields the closure body's result kind, traced from
        // the callee's construction site (the closure-result analog of `GetField`'s field kind).
        Op::MakeClosure { dst, .. } => Some((dst as usize, Some(Kind::Closure))),
        Op::CallValue { dst, .. } => {
            let k = env
                .callee_func
                .get(pc)
                .copied()
                .flatten()
                .and_then(|fi| (env.closure_ret)(fi as usize));
            Some((dst as usize, k))
        }
        // `obj's field` yields the field's kind — the resolved cross-region kind for a call-result
        // struct, else the kind of the value that was inserted into that field intra-region.
        Op::GetField { dst, .. } => {
            let k = env.field_kind.get(pc).copied().flatten().or_else(|| {
                env.struct_field_value
                    .get(pc)
                    .copied()
                    .flatten()
                    .and_then(|r| kinds.get(r as usize).copied().flatten())
            });
            Some((dst as usize, k))
        }
        // `If it is a Circle (radius: r)` binds the constructor's `index`-th argument; its kind is
        // that argument register's kind (traced from the matched value's construction site).
        Op::BindArm { dst, .. } => {
            let k = env.arg_kind.get(pc).copied().flatten().or_else(|| {
                env.bind_arm_value
                    .get(pc)
                    .copied()
                    .flatten()
                    .and_then(|r| kinds.get(r as usize).copied().flatten())
            });
            Some((dst as usize, k))
        }
        // A new empty map (`Map of Int to Int`).
        Op::NewEmptyMap { dst } => Some((dst as usize, Some(Kind::Map))),
        // A new empty set (`Set of Int` by default; a Text `Add` refines it to `SetText`).
        Op::NewEmptySet { dst } => Some((dst as usize, Some(Kind::Set))),
        // A fresh CRDT collection (a `Shared` struct field default). SINGLE-replica, it is just its
        // underlying collection: an OR-Set (`kind` 0/3) → `CrdtSetText` (byte-dedup set, but MUTABLE-
        // SHARED so `Add`/`Remove` on the field mutate in place), an RGA/sequence (1) → `SeqText`, a
        // divergent register (else) → `Text`. Per-replica merge metadata isn't represented (merge defers).
        Op::NewCrdt { dst, kind } => Some((dst as usize, Some(match kind {
            0 | 3 => Kind::CrdtSetText,
            1 => Kind::SeqText,
            _ => Kind::Text,
        }))),
        // `Add value to s` refines a set's element kind: a `Text` value makes it a `Set of Text`
        // (byte-equality dedup). An Int value keeps `Set`. `unify_strict` lets `Set` (the empty-set
        // default) absorb this `SetText` refinement, exactly as `SeqAny` refines to a concrete seq.
        Op::SetAdd { set, value } => {
            let k = match kinds.get(value as usize).copied().flatten() {
                Some(Kind::Text) => Some(Kind::SetText),
                _ => Some(Kind::Set),
            };
            Some((set as usize, k))
        }
        // `a union b` / `a intersection b` build a fresh combined Set.
        Op::UnionOp { dst, .. } | Op::IntersectOp { dst, .. } => Some((dst as usize, Some(Kind::Set))),
        // `item key of m` yields the map's VALUE kind (the kind of a value stored into it — Int,
        // Float, or Bool, resolved via [`struct_layout`]'s `map_value` track) for a Map, or the
        // element kind for a sequence.
        Op::Index { dst, collection, .. } | Op::IndexUnchecked { dst, collection, .. } => {
            let elem = match kinds.get(collection as usize).copied().flatten() {
                Some(Kind::Map) => env
                    .index_value_kind
                    .get(pc)
                    .copied()
                    .flatten()
                    .or_else(|| env.map_value.get(pc).copied().flatten().and_then(|r| kinds.get(r as usize).copied().flatten())),
                Some(Kind::Tuple) => env
                    .index_value_kind
                    .get(pc)
                    .copied()
                    .flatten()
                    .or_else(|| env.tuple_value.get(pc).copied().flatten().and_then(|r| kinds.get(r as usize).copied().flatten())),
                // Indexing a `Text` yields a one-character `Text` (the VM's `item i of "abc"`).
                Some(Kind::Text) => Some(Kind::Text),
                other => other.and_then(Kind::seq_elem),
            };
            Some((dst as usize, elem))
        }
        // The loop variable of `Repeat for x in seq` gets the iterable's element kind. The
        // iterable is found via the `IterPrepare`↔`IterNext` pairing; a `SeqAny` (only ever an
        // empty sequence) yields `None` here, so the never-executed load defaults harmlessly.
        Op::IterNext { dst, .. } => {
            let elem = env
                .iter_iterable
                .get(pc)
                .copied()
                .flatten()
                .and_then(|r| kinds.get(r).copied().flatten())
                .and_then(Kind::seq_elem);
            Some((dst as usize, elem))
        }
        // `start to end` is always an Int range.
        Op::NewRange { dst, .. } => Some((dst as usize, Some(Kind::SeqInt))),
        // A list literal `[a, b, …]` is a sequence of its (first) element's kind. A homogeneous
        // tuple `(a, b, …)` lays out identically (a buffer of the shared element kind), so for the
        // ops the AOT backend supports — `item N of t`, `t[N]`, `length of t` — it IS that sequence
        // kind. (A heterogeneous tuple is rejected at lowering.)
        // A list literal `[a, b, …]` is a homogeneous sequence of its (first) element's kind.
        Op::NewList { dst, start, count } => {
            let elem = if count > 0 { kinds.get(start as usize).copied().flatten() } else { Some(Kind::Int) };
            let seq = match elem {
                Some(Kind::Bool) => Kind::SeqBool,
                Some(Kind::Float) => Kind::SeqFloat,
                Some(Kind::Text) => Kind::SeqText,
                Some(Kind::Struct) => Kind::SeqStruct,
                Some(Kind::Enum) => Kind::SeqEnum,
                Some(Kind::SeqInt) => Kind::SeqSeqInt,
                Some(Kind::Word32) => Kind::SeqWord32,
                Some(Kind::Word64) => Kind::SeqWord64,
                _ => Kind::SeqInt,
            };
            Some((dst as usize, Some(seq)))
        }
        // A tuple `(a, b, …)`: a homogeneous one shares the matching `SeqX` representation; a
        // heterogeneous one is a `Kind::Tuple` (flat slots at per-element widths). Deferred until
        // every element kind is resolved (the fixpoint revisits).
        Op::NewTuple { dst, start, count } => {
            if count == 0 {
                return Ok(Some((dst as usize, Some(Kind::SeqInt))));
            }
            let elems: Vec<Option<Kind>> = (0..count).map(|j| kinds.get((start + j) as usize).copied().flatten()).collect();
            let k = if elems.iter().any(Option::is_none) {
                None
            } else if elems.iter().all(|e| *e == elems[0]) {
                Some(match elems[0] {
                    Some(Kind::Float) => Kind::SeqFloat,
                    Some(Kind::Text) => Kind::SeqText,
                    Some(Kind::Struct) => Kind::SeqStruct,
                    Some(Kind::Enum) => Kind::SeqEnum,
                    Some(Kind::SeqInt) => Kind::SeqSeqInt,
                    _ => Kind::SeqInt,
                })
            } else {
                Some(Kind::Tuple)
            };
            Some((dst as usize, k))
        }
        // Numeric builtins. `sqrt` always returns a Float; `floor`/`ceil`/`round` always return
        // an Int; `abs` keeps the operand kind; `min`/`max`/`pow` return Float if either operand
        // is a Float, else Int. Other builtins are rejected at lowering (dst left unknown).
        Op::CallBuiltin { dst, builtin, args_start, arg_count } => {
            let a = kinds.get(args_start as usize).copied().flatten();
            let b = if arg_count >= 2 { kinds.get((args_start + 1) as usize).copied().flatten() } else { None };
            let k = match builtin {
                BuiltinId::Sqrt => Some(Kind::Float),
                // `floor`/`ceil`/`round` of a Float/Int is an `Int`; of a LINKED `Rational` it is the exact
                // BigInt the fraction rounds to (`floor(7/2) = 3`, arbitrary precision) — a BigInt handle.
                BuiltinId::Floor | BuiltinId::Ceil | BuiltinId::Round => {
                    if env.linked && a == Some(Kind::Rational) { Some(Kind::BigInt) } else { Some(Kind::Int) }
                }
                BuiltinId::Abs => a,
                BuiltinId::Min | BuiltinId::Max | BuiltinId::Pow => match (a, b) {
                    (Some(Kind::Float), _) | (_, Some(Kind::Float)) => Some(Kind::Float),
                    (Some(Kind::Int), Some(Kind::Int)) => Some(Kind::Int),
                    _ => None,
                },
                // `complex(re, im)` builds an EXACT `Complex` via the linked runtime (LINKER MODE only;
                // standalone can't reach the exact Rational-backed runtime, so it stays refused there).
                BuiltinId::Complex if env.linked => Some(Kind::Complex),
                BuiltinId::Modular if env.linked => Some(Kind::Modular),
                BuiltinId::Decimal if env.linked => Some(Kind::Decimal),
                BuiltinId::Money if env.linked => Some(Kind::Money),
                // `quantity(v, "unit")` builds an EXACT `Quantity`; `convert(q, "unit")` (and the
                // surface `X in <unit>`) re-expresses one in a new display unit — both LINKER MODE only.
                BuiltinId::Quantity if env.linked => Some(Kind::Quantity),
                BuiltinId::Convert if env.linked => Some(Kind::Quantity),
                // The `Uuid`-producing builtins (parse + the well-known namespace/nil/max constants) →
                // a linked `Uuid` handle; `uuid_version` reads the version nibble as an `Int`.
                BuiltinId::Uuid
                | BuiltinId::UuidNil
                | BuiltinId::UuidMax
                | BuiltinId::UuidDns
                | BuiltinId::UuidUrl
                | BuiltinId::UuidOid
                | BuiltinId::UuidX500 if env.linked => Some(Kind::Uuid),
                BuiltinId::UuidVersion if env.linked => Some(Kind::Int),
                // `repeatSeq(x, n) -> Seq` — a fresh n-element sequence of the scalar `x`.
                BuiltinId::RepeatSeq => match a {
                    Some(Kind::Float) => Some(Kind::SeqFloat),
                    Some(Kind::Int) => Some(Kind::SeqInt),
                    Some(Kind::Bool) => Some(Kind::SeqBool),
                    _ => Some(Kind::SeqAny),
                },
                // `parseInt(text) -> Int` (a host call to `parse_int`).
                BuiltinId::ParseInt => Some(Kind::Int),
                // `parseFloat(text) -> Float` (a host call to `parse_float`).
                BuiltinId::ParseFloat => Some(Kind::Float),
                // `count_ones(n) -> Int` — the Int's population count (`i64.popcnt`).
                BuiltinId::CountOnes => Some(Kind::Int),
                // `writeWireResidual(text) -> Int` — the framed byte count written to the host wire sink.
                BuiltinId::WriteWireResidual => Some(Kind::Int),
                // `chr(code) -> Text` — a one-character Text built inline (`lower_chr`).
                BuiltinId::Chr => Some(Kind::Text),
                // Word ring construct/extract: `word32`/`word64` build a Word; `intOfWord*` yield an Int.
                BuiltinId::Word32 => Some(Kind::Word32),
                BuiltinId::Word64 => Some(Kind::Word64),
                BuiltinId::IntOfWord32 | BuiltinId::IntOfWord64 => Some(Kind::Int),
                // `rotl`/`rotr`/`word_and`/`word_or`/`word_not` PRESERVE the (first) word operand's kind
                // (`a` = the `args_start` register's kind — `Word32` or `Word64`).
                BuiltinId::Rotl | BuiltinId::Rotr | BuiltinId::Wand | BuiltinId::Wor | BuiltinId::Wnot => a,
                // `copy(x)` deep-clones its argument, PRESERVING the kind (a scalar clone is the value; a
                // heap clone is an independent copy — `lower_deep_clone` handles both).
                BuiltinId::Copy => a,
                // The `word64*` ops always yield a `Word64`.
                BuiltinId::Word64Shl | BuiltinId::Word64Shr | BuiltinId::Word64And => Some(Kind::Word64),
                // `word32Shr` (logical right-shift) always yields a `Word32`.
                BuiltinId::Word32Shr => Some(Kind::Word32),
                // `parse_timestamp(text) -> Moment` (a host call to `parse_timestamp`).
                BuiltinId::ParseTimestamp => Some(Kind::Moment),
                // Calendar/clock component extractors on a `Moment` each yield an `Int`.
                BuiltinId::YearOf
                | BuiltinId::MonthOf
                | BuiltinId::DayOf
                | BuiltinId::WeekdayOf
                | BuiltinId::HourOf
                | BuiltinId::MinuteOf
                | BuiltinId::SecondOf
                | BuiltinId::WeekOf
                | BuiltinId::QuarterOf => Some(Kind::Int),
                // Moment arithmetic + calendar/clock extraction (self-contained): `seconds_between`
                // yields an `Int`, `add_seconds` a `Moment`, `date_of` a `Date`, `time_of` a `Time`.
                BuiltinId::SecondsBetween => Some(Kind::Int),
                BuiltinId::AddSeconds => Some(Kind::Moment),
                BuiltinId::DateOf => Some(Kind::Date),
                BuiltinId::TimeOf => Some(Kind::Time),
                // LINKER-mode extended temporal (calendar logic in `base::temporal`): `format_timestamp`
                // yields a `Text` handle, `months_between`/`years_between` an `Int`.
                BuiltinId::FormatTimestamp if env.linked => Some(Kind::Text),
                BuiltinId::MonthsBetween | BuiltinId::YearsBetween if env.linked => Some(Kind::Int),
                // Zoned temporal (linker): `in_zone`→a `Text` (local wall-clock), `local_instant`→a `Moment`.
                BuiltinId::InZone if env.linked => Some(Kind::Text),
                BuiltinId::LocalInstant if env.linked => Some(Kind::Moment),
                // The general SIMD lane vocabulary (LINKER MODE): constructors + byte/word-lane ops yield a
                // `LanesV` handle; the extractors unpack it to a `Seq of Int` (bytes) / `Seq of Word32`.
                BuiltinId::Lanes16Word8Make
                | BuiltinId::Lanes8Word32
                | BuiltinId::Lanes4Word64
                | BuiltinId::Splat16Word8
                | BuiltinId::Splat8Word32
                | BuiltinId::Shuffle16
                | BuiltinId::InterleaveLo16
                | BuiltinId::InterleaveHi16
                | BuiltinId::ByteAdd16
                | BuiltinId::Maddubs16
                | BuiltinId::Packus16
                | BuiltinId::ShrBytes16 if env.linked => Some(Kind::LanesV),
                BuiltinId::SeqOfLanes16W8 if env.linked => Some(Kind::SeqInt),
                BuiltinId::SeqOfLanes8 if env.linked => Some(Kind::SeqWord32),
                // Money FX (linker): `to_currency`→a `Money`; `set_rate`/`set_rates` are effectful and
                // yield `Nothing` (an `Optional`, handle 0).
                BuiltinId::ToCurrency if env.linked => Some(Kind::Money),
                BuiltinId::SetRate | BuiltinId::SetRates if env.linked => Some(Kind::Optional),
                // `wireBytes(value) -> Seq of Int` (linker) — the value's wire encoding as a byte sequence.
                BuiltinId::WireBytes if env.linked => Some(Kind::SeqInt),
                // `readWireProgram() -> a DYNAMIC value` (linker); `run_accepted(...) -> Int` (linker).
                BuiltinId::ReadWireProgram if env.linked => Some(Kind::Dynamic),
                BuiltinId::RunAccepted if env.linked => Some(Kind::Int),
                // `format(x) -> Text` — the value's `to_display_string` as a Text (`emit_stringify`).
                BuiltinId::Format => Some(Kind::Text),
                // Byte interop: `text_bytes`/`uuid_bytes` → a `Seq of Int` (the UTF-8 / 16 raw bytes);
                // `text_from_bytes` → `Text`; `uuid_from_bytes` → a linked `Uuid` handle.
                BuiltinId::TextBytes => Some(Kind::SeqInt),
                BuiltinId::UuidBytes if env.linked => Some(Kind::SeqInt),
                BuiltinId::TextFromBytes => Some(Kind::Text),
                BuiltinId::UuidFromBytes if env.linked => Some(Kind::Uuid),
                // The SHA-1 SHA-NI lane vocabulary (LINKER MODE): `lanes4Of`/`lanes4Word32` + the four
                // intrinsics produce a `Lanes` handle; `seqOfLanes4W32` unpacks it back to a `Seq of Word32`.
                BuiltinId::Lanes4Of
                | BuiltinId::Lanes4Word32Make
                | BuiltinId::Sha1Rnds4
                | BuiltinId::Sha1Msg1
                | BuiltinId::Sha1Msg2
                | BuiltinId::Sha1Nexte if env.linked => Some(Kind::Lanes),
                BuiltinId::SeqOfLanes4W32 if env.linked => Some(Kind::SeqWord32),
                _ => None,
            };
            Some((dst as usize, k))
        }
        // `args()` yields the command-line arguments as a `Seq of Text` handle.
        Op::Args { dst } => Some((dst as usize, Some(Kind::SeqText))),
        // Every other op writes no register, writes a non-scalar (rejected when lowered), or is
        // control flow — no scalar kind effect.
        _ => None,
    })
}

/// Unify register `r` with `kind`. A `None` kind (not yet known) is a no-op. A clash between two
/// value-type-incompatible kinds is unrepresentable in a single wasm local, so it makes the
/// function `Unsupported`. `Int` vs `Bool` (both `i64`) is also rejected: they display
/// differently, and proving a register monotyped keeps `Show` sound. Sets `changed`.
fn unify_strict(
    kinds: &mut [Option<Kind>],
    changed: &mut bool,
    r: usize,
    kind: Option<Kind>,
    label_sensitive: &[bool],
) -> Result<(), WasmLowerError> {
    let Some(k) = kind else { return Ok(()) };
    match kinds[r] {
        None => {
            kinds[r] = Some(k);
            *changed = true;
        }
        Some(existing) if existing == k => {}
        // Sequence element refinement: `SeqAny` (a never-yet-pushed empty seq) refines to the
        // concrete element seq the first push reveals; a concrete seq absorbs a `SeqAny` no-op.
        Some(Kind::SeqAny) if k == Kind::SeqInt || k == Kind::SeqBool || k == Kind::SeqFloat || k == Kind::SeqText || k == Kind::SeqStruct || k == Kind::SeqEnum || k == Kind::SeqSeqInt || k == Kind::SeqWord32 || k == Kind::SeqWord64 => {
            kinds[r] = Some(k);
            *changed = true;
        }
        Some(Kind::SeqInt | Kind::SeqBool | Kind::SeqFloat | Kind::SeqText | Kind::SeqStruct | Kind::SeqEnum | Kind::SeqSeqInt | Kind::SeqWord32 | Kind::SeqWord64) if k == Kind::SeqAny => {}
        // Set element refinement: the `Set` default (an empty `new Set of …`) refines to `SetText`
        // the first time a Text is added; a concrete `SetText` absorbs a later `Set` (Int-default) no-op.
        Some(Kind::Set) if k == Kind::SetText => {
            kinds[r] = Some(Kind::SetText);
            *changed = true;
        }
        Some(Kind::SetText) if k == Kind::Set => {}
        // A CRDT set field's kind is fixed by `NewCrdt`; a `SetAdd`/`RemoveFrom` on it re-derives a
        // `Set`/`SetText` kind, which `CrdtSetText` ABSORBS (it stays the mutable-shared CRDT kind).
        Some(Kind::CrdtSetText) if k == Kind::Set || k == Kind::SetText => {}
        // Two different concrete sequences (Int vs Float elements) cannot be the same register.
        Some(s1) if s1.is_seq() && k.is_seq() => {
            return Err(WasmLowerError::Unsupported("sequence reused across Int/Float elements"))
        }
        // Int vs Bool — both are `i64`, so the VM legitimately reuses one register slot across
        // them (e.g. an Int accumulator and a Bool membership/compare result with disjoint live
        // ranges). The label only changes lowering for Show/Not; if the register feeds
        // none of those it collapses to one `i64` local (a Bool's 0/1 is a valid Int and every
        // other op lowers identically). If it does feed one, the display/dispatch is genuinely
        // ambiguous and the function is (soundly) refused. This merge resolves to `Int` (an i64
        // local), so it is sound ONLY when the shared valtype is i64 — distinct i32-handle kinds
        // (Date/Seq/Text) cannot collapse to Int without corrupting the local type, so they error.
        Some(existing) if existing.same_valtype(k) && existing.wasm_valtype() == I64 => {
            if label_sensitive.get(r).copied().unwrap_or(true) {
                return Err(WasmLowerError::Unsupported("register reused across Int/Bool kinds"));
            }
            if kinds[r] != Some(Kind::Int) {
                kinds[r] = Some(Kind::Int);
                *changed = true;
            }
        }
        // Int vs Float — a VM accumulator legitimately starts Int (`Let sum be 0`) and becomes
        // Float on its first float op (`sum + 1.5`); the single def-use web (one variable) is
        // genuinely both, so live-range splitting cannot separate them. Promote the whole register
        // to Float (f64): its Int constant defs lower as `f64.const`, its Int arithmetic flows
        // through the float path (operands promoted by `push_as_f64`). Display-sound because a
        // whole-number float prints identically to the int (`5.0` → "5"), and the f64 the VM holds
        // after promotion is the same value, so every later `Show`/compare agrees.
        Some(existing) if matches!((existing, k), (Kind::Int, Kind::Float) | (Kind::Float, Kind::Int)) => {
            if kinds[r] != Some(Kind::Float) {
                kinds[r] = Some(Kind::Float);
                *changed = true;
            }
        }
        Some(_) => return Err(WasmLowerError::Unsupported("register reused across incompatible value types")),
    }
    Ok(())
}

/// Map declared parameter kinds to seed register kinds — the parameters occupy registers
/// `0..param_kinds.len()`. A non-scalar parameter (`Seq of …`, Map, Text) makes the function
/// unsupported in the scalar P0 backend.
pub(crate) fn param_seeds(param_kinds: &[Option<ParamKind>]) -> Result<Vec<Option<Kind>>, WasmLowerError> {
    param_kinds
        .iter()
        .map(|pk| match pk {
            Some(ParamKind::Scalar(slot)) => Ok(Some(Kind::from_slot(*slot))),
            // A `Seq of <scalar>` parameter arrives as one stable i32 handle (same as any heap value
            // in the bytecode — the native tier's pinned ptr+len is its own ABI, not this one). The
            // element kind is in the declaration, so the seq kind is self-describing; the body's
            // Index/Length/iteration just work. Element kinds without a self-describing seq kind
            // (Bool would mis-display as Int, Map nesting, …) stay unsupported.
            Some(ParamKind::List(elem)) => match elem {
                PinElem::Int | PinElem::IntI32 => Ok(Some(Kind::SeqInt)),
                PinElem::Float => Ok(Some(Kind::SeqFloat)),
                _ => Err(WasmLowerError::Unsupported("non-scalar (list) parameter element")),
            },
            None => Ok(None),
        })
        .collect()
}

/// Map a resolved [`BoundaryType`] to this backend's [`Kind`] — the bridge from the bytecode's
/// static type registry to the AOT value model. `None` for a type with no self-describing single
/// `Kind` (a `Seq` of struct/enum, etc.).
pub(crate) fn boundary_to_kind(bt: &BoundaryType) -> Option<Kind> {
    Some(match bt {
        BoundaryType::Int => Kind::Int,
        BoundaryType::Bool => Kind::Bool,
        BoundaryType::Float => Kind::Float,
        BoundaryType::Text => Kind::Text,
        BoundaryType::Date => Kind::Date,
        BoundaryType::Moment => Kind::Moment,
        BoundaryType::Word32 => Kind::Word32,
        BoundaryType::Word64 => Kind::Word64,
        BoundaryType::Seq(elem) => match boundary_to_kind(elem)? {
            Kind::Int => Kind::SeqInt,
            Kind::Bool => Kind::SeqBool,
            Kind::Float => Kind::SeqFloat,
            Kind::Text => Kind::SeqText,
            Kind::Word32 => Kind::SeqWord32,
            Kind::Word64 => Kind::SeqWord64,
            _ => return None,
        },
        BoundaryType::Struct(_) => Kind::Struct,
        BoundaryType::Enum(_) => Kind::Enum,
        // One i32 handle. The value kind is carried by the param seed (for `item k of m`), not here.
        BoundaryType::Map(_, _) => Kind::Map,
        // One i32 handle to a mixed-kind slot buffer. The per-position kinds are carried by the param
        // seed (for `item N of t`), not here.
        BoundaryType::Tuple(_) => Kind::Tuple,
        // A first-class builtin value type — its handle/scalar kind is fixed by name. A name with no
        // wasm kind returns `None` (early), leaving the parameter soundly unmodeled.
        BoundaryType::Builtin(name) => return builtin_name_to_kind(name),
    })
}

/// The wasm register [`Kind`] for a first-class builtin value type name (the linked numeric-tower and
/// temporal/uuid handle types), or `None` for a name the wasm backend does not model.
fn builtin_name_to_kind(name: &str) -> Option<Kind> {
    Some(match name {
        "Uuid" => Kind::Uuid,
        "Duration" => Kind::Duration,
        "Time" => Kind::Time,
        "Span" => Kind::Span,
        "Rational" => Kind::Rational,
        "Complex" => Kind::Complex,
        "Modular" => Kind::Modular,
        "Decimal" => Kind::Decimal,
        "Money" => Kind::Money,
        "Quantity" => Kind::Quantity,
        _ => return None,
    })
}

/// A function's parameter seed kinds for the AOT — preferring each parameter's FULL declared type
/// (`param_types`, which models struct/Text/etc.) and falling back to the native `param_kinds`
/// hint. Unlike [`param_seeds`] this never errors: an unmodeled parameter seeds `None` and is left
/// to fail (soundly) at the first op that needs its kind.
pub(crate) fn function_param_seeds(f: &CompiledFunction) -> Vec<Option<Kind>> {
    (0..f.param_count as usize)
        .map(|i| {
            if let Some(Some(bt)) = f.param_types.get(i) {
                if let Some(k) = boundary_to_kind(bt) {
                    return Some(k);
                }
            }
            match f.param_kinds.get(i) {
                Some(Some(ParamKind::Scalar(slot))) => Some(Kind::from_slot(*slot)),
                Some(Some(ParamKind::List(PinElem::Int | PinElem::IntI32))) => Some(Kind::SeqInt),
                Some(Some(ParamKind::List(PinElem::Float))) => Some(Kind::SeqFloat),
                _ => None,
            }
        })
        .collect()
}
