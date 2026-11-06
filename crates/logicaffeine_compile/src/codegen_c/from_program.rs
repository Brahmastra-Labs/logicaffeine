//! ★ Phase 1 — the `CompiledProgram → C` Op-walker (bytecode-IR-as-source-of-truth re-base) ★
//!
//! The SYSTEMS-tier C backend that hangs off the bytecode VM, mirroring `codegen_riscv`'s `Emitter`: walk the
//! `CompiledProgram`'s flat `Op` stream and lower each op to C over a register file, with `Jump`/`JumpIfFalse`/
//! `JumpIfTrue` reconstructed as `goto` on the same CFG (a relooper recovering `while`/`if` for gcc's
//! vectorizer is a later perf pass; correctness first). User functions are separate regions (delimited by
//! `entry_pc`), each emitted as a C function under the VM's register-windowing call convention. Because it
//! lowers the bytecode, it inherits the whole optimizer lattice — the perf win the plan predicts.
//!
//! **Uniform register file.** Every register is one `int64_t` slot. Scalars of every kind ride it: an `Int`
//! holds its value, a `Bool` holds 0/1, a `Float` holds its IEEE-754 *bit pattern* (recovered with `as_f64`),
//! a `Text` holds a `char*` reinterpreted as an integer, and a `Seq` holds a heap-boxed `Seq_i64*`/`Seq_f64*`/
//! `Seq_str*`/`Seq_bool*` (the proven `codegen_c` value-typed runtime, boxed so its handle fits a slot). A
//! per-register `Kind`, inferred by a forward pass mirroring `codegen_riscv::infer_kinds`, drives how an op
//! combines its operands (int vs float), how a collection op picks its typed runtime call, and how `Show`
//! renders — exactly the proven fabric/wpe model where "float rides in the i64 regs as bit patterns."
//!
//! Grown op-by-op behind `c_bytecode_lock` (`program_outcome(P) == c_bytecode_run(P)`); a not-yet-lowered op
//! (or a kind this pass does not model) is a LOUD `Err` (a typed refusal, never wrong code), exactly like
//! `codegen_riscv`'s `Unsupported`. The AST-based `codegen_c` stays the shipping path until this Op-walker
//! covers the corpus, then `compile_to_c` swaps to it and the AST entanglement is deleted.
use std::fmt::Write;

use super::runtime::{
    c_net_runtime, c_runtime, c_vfs_runtime, C_CRDT_RUNTIME, C_GC_COLLECTOR, C_TCP_RUNTIME,
};
use super::words;
use crate::analysis::{PolicyCondition, PolicyRegistry};
use crate::intern::Interner;
use crate::interpreter::RuntimeValue;
use crate::vm::{BoundaryType, CompiledProgram, Constant, Op, SlotKind};

mod exactness;
mod numbers;
mod tower;

/// The rounds a region's exact webs and number joins take to hold still before the region is refused: each round
/// only widens what a variable holds, and no chain of widenings is this long.
const SETTLE_ROUNDS: usize = 64;

/// The kind carried in an `int64_t` register slot. Grown milestone by milestone; a kind this pass does not
/// model is `Unknown` and any kind-sensitive op on it is a typed refusal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Int,
    Bool,
    Float,
    Text,
    /// An exact integer, held where the program can make an `Int` outgrow i64 ([`Emitter::exactness`]). It rides
    /// its register tagged: an even word is the value shifted left once (any integer within 63 bits, so the zero
    /// word is 0), an odd word a boxed `LInt*` with its low bit set. The `lc_x*` operations stay on the word
    /// while the value fits it and go through the runtime's `lint_*` tower when it does not — the VM's promotion.
    BigInt,
    /// An exact rational (LOGOS `Rational`/`Decimal`), the C mirror of `LogosRational` — a heap-boxed `LDec*`
    /// (`{ is_rat, num, den, scale }` over `LBig`) reinterpreted as the register's integer. `7 / 2` stays `7/2`
    /// exact (never the floored `3`); arithmetic routes through the runtime's `ldec_add`/`sub`/`mul`/`div`, and
    /// `Show`/interpolation render it via `ldec_to_string`. Boxed exactly like `Text`/`Struct` (a pointer in the
    /// uniform i64 slot), so it composes with `Move`/`Let`/`Call` for free.
    Rational,
    /// A heap-boxed sequence handle, by scalar element kind.
    SeqInt,
    SeqFloat,
    SeqText,
    SeqBool,
    /// A heap-boxed sequence of values of a kind with no scalar cell of its own — tuples, records, enums, other
    /// collections — a `Seq_i64` whose cells are the elements' register encodings (a heap pointer rides the
    /// uniform i64 slot). The `u16` interns the element kind (`Emitter::seq_elems`), so an element read out of
    /// it — by index, by iteration, by pop — is the value it was.
    SeqOf(u16),
    /// A sequence created empty whose element kind is not yet known (refined by a later push) — refused.
    SeqUnknown,
    /// A heap-boxed insertion-ordered dedup'd set handle (`Set_i64*`/`Set_str*`), by scalar element kind
    /// (Int/Text only — the runtime's two self-describing set kinds, matching the RISC-V/WASM tiers).
    SetInt,
    /// A set of machine words of the given width: a `Set_i64` of their zero-extended bits, shown unsigned.
    SetWord(u8),
    SetText,
    /// A set created empty whose element kind is not yet known (refined by a later `SetAdd`) — refused.
    SetUnknown,
    /// A counter CRDT handle (a `logos_crdt*` = per-replica count map) riding the uniform i64 slot. Produced by
    /// `NewCrdt` for a G-Counter (`ConvergentCount`, kind 4) or a PN-Counter (`Tally`, kind 5). It lives in a
    /// struct field whose DECLARED type is `Int`; a `GetField` on such a field SUMS the map to an `Int` (matching
    /// the VM's `Show c's points` = the counter total), and `CrdtMerge`/`CrdtJoin` take the per-replica MAX
    /// (idempotent — a redelivered state never double-counts). Mirrors `codegen_riscv::Kind::CrdtCounter`.
    CrdtCounter,
    /// A user-struct handle (a heap buffer of one `int64_t` cell per field, in declaration order). The `u16`
    /// indexes `prog.struct_types`, so the field layout stays reachable while `Kind` remains `Copy`.
    Struct(u16),
    /// A user-struct handle whose declared type DID NOT resolve statically (an unmodeled field type left the
    /// struct out of `prog.struct_types` — e.g. a `Tally` whose `n: a Counter` field names an undeclared type).
    /// The VM still runs such a struct dynamically (its `CrdtBump` on a `Nothing` cell yields a plain `Int`), so
    /// the walker synthesizes the field layout from the bytecode: the `u16` indexes the Emitter's `synth_structs`
    /// interning table (name + field-name-in-cell-order), collected by scanning the field ops on the struct.
    StructSynth(u16),
    /// A user-enum (inductive) handle — a heap buffer `[variant-tag, payload…]` (tag = the variant's index in
    /// the declaration). The `u16` indexes `prog.enum_types`.
    Enum(u16),
    /// An enum the program never declares — `Option`, `Result`, a library enum outside `prog.enum_types` — laid
    /// out like a declared one: a heap `[tag, payload…]` buffer. Its constructors are known only by name, which is
    /// how the VM matches them too (`ind.constructor == name`), so each type's constructors get their tags in one
    /// program-wide order (`Emitter::ctor_tags`). The `u16` interns the type with the payload kinds known for each
    /// constructor (`Emitter::inductive_types`), so a bound payload shows and computes as what it is.
    Inductive(u16),
    /// `nothing` — the unit value (a `Unit` result's payload, a field before its first write), riding the slot
    /// as `0` and shown as `nothing`.
    Nothing,
    /// An anonymous tuple handle — a heap buffer of one `int64_t` cell per position (positional, like a struct
    /// with no field names). The `u16` indexes the Emitter's `tuple_types` interning table: tuples are
    /// structural/unnamed, so — unlike `Struct`/`Enum` — there is no program-level type table to point at.
    Tuple(u16),
    /// A heap-boxed hash map handle. The `u16` indexes the Emitter's `map_types` interning table of (key kind,
    /// value kind): the KEY is a runtime-hashable scalar (Int→i64 / Text→str) and the VALUE may be ANY kind
    /// (it rides the i64/str cell as its register encoding), so the true value kind is remembered for `Index`.
    Map(u16),
    /// A map created empty whose key/value kinds are not yet known (refined by the first `SetIndex`) — an
    /// unpopulated one boxes a `Map_i64_i64` and renders `{}`.
    MapUnknown,
    /// A peer handle (`a PeerAgent at …`): its canonical relay topic, a `char*`, shown as `<peer topic>`.
    Peer,
    /// A calendar date (`today`), riding the slot as its day count since the epoch, shown `YYYY-MM-DD`.
    Date,
    /// An instant (`now`), riding the slot as its nanoseconds since the epoch, shown `YYYY-MM-DD HH:MM`.
    Moment,
    /// A time of day, riding the slot as its nanoseconds since midnight.
    Time,
    /// A calendar span, riding the slot as its months in the high 32 bits and its days in the low 32.
    Span,
    /// A wrapping machine word of the given width in bits (8, 16, 32 or 64): the ring ℤ/2ⁿ, riding the slot
    /// zero-extended, shown as its unsigned value.
    Word(u8),
    /// A value of the exact extension (an infinitesimal, a hyperreal, an algebraic number, an ordinal, … and every
    /// list of them): an `LTower` cell around a handle into the linked value runtime, riding the slot as a pointer
    /// (see `tower`).
    Tower,
    /// A value that cannot exist: an element of a sequence nothing ever puts an element in. Reading one fails
    /// (an index into an empty sequence) or never happens (a loop over one), so every op that would use it is
    /// unreachable.
    Never,
    Unknown,
}

impl Kind {
    /// The element kind of a sequence/set kind of scalar elements (`Unknown` otherwise; a `SeqOf`'s element
    /// kind is interned, see [`Emitter::elem_kind`]).
    fn elem(self) -> Kind {
        match self {
            Kind::SeqInt | Kind::SetInt => Kind::Int,
            Kind::SetWord(bits) => Kind::Word(bits),
            Kind::SeqFloat => Kind::Float,
            Kind::SeqText | Kind::SetText => Kind::Text,
            Kind::SeqBool => Kind::Bool,
            _ => Kind::Unknown,
        }
    }
    /// The set kind whose element is `self` (`SetUnknown` for an element with no self-describing set kind).
    fn set_of(self) -> Kind {
        match self {
            Kind::Int => Kind::SetInt,
            Kind::Word(bits) => Kind::SetWord(bits),
            Kind::Text => Kind::SetText,
            _ => Kind::SetUnknown,
        }
    }
    /// The C `Seq_*` struct type for a sequence kind.
    fn seq_c_type(self) -> Option<&'static str> {
        match self {
            Kind::SeqInt => Some("Seq_i64"),
            Kind::SeqFloat => Some("Seq_f64"),
            Kind::SeqText => Some("Seq_str"),
            Kind::SeqBool => Some("Seq_bool"),
            // A seq of non-scalar values is a `Seq_i64` of their register encodings.
            Kind::SeqOf(_) => Some("Seq_i64"),
            _ => None,
        }
    }
    /// The C `Set_*` struct type for a set kind.
    fn set_c_type(self) -> Option<&'static str> {
        match self {
            Kind::SetInt | Kind::SetWord(_) => Some("Set_i64"),
            Kind::SetText => Some("Set_str"),
            _ => None,
        }
    }
    /// The C function-name infix (`i64`/`f64`/`str`/`bool`) for a sequence OR set kind.
    fn seq_infix(self) -> Option<&'static str> {
        match self {
            Kind::SeqInt | Kind::SetInt | Kind::SetWord(_) | Kind::SeqOf(_) => Some("i64"),
            Kind::SeqFloat => Some("f64"),
            Kind::SeqText | Kind::SetText => Some("str"),
            Kind::SeqBool => Some("bool"),
            _ => None,
        }
    }
}

/// True for a set kind whose element type is known (so its boxed `Set_*` can be allocated).
fn is_concrete_set(k: Kind) -> bool {
    matches!(k, Kind::SetInt | Kind::SetWord(_) | Kind::SetText)
}

/// The concrete set kind to operate on: a `SetUnknown` (an empty, never-added-to set) was boxed as a
/// `Set_i64`, so it operates as `SetInt`; a non-set is `None`.
fn resolve_set(k: Kind) -> Option<Kind> {
    match k {
        Kind::SetInt | Kind::SetUnknown => Some(Kind::SetInt),
        Kind::SetWord(bits) => Some(Kind::SetWord(bits)),
        Kind::SetText => Some(Kind::SetText),
        _ => None,
    }
}

/// The sequence kind to operate on: a sequence made empty whose element kind never became known was boxed
/// as a `Seq_i64`, so it operates as `SeqInt`; every other kind is itself.
fn seq_kind_of(k: Kind) -> Kind {
    if k == Kind::SeqUnknown {
        Kind::SeqInt
    } else {
        k
    }
}

/// True for any sequence kind, its elements known or not.
fn is_sequence(k: Kind) -> bool {
    k.seq_c_type().is_some() || k == Kind::SeqUnknown
}

/// True for a sequence kind whose element type is known (so its boxed `Seq_*` can be allocated).
fn is_concrete_seq(k: Kind) -> bool {
    matches!(
        k,
        Kind::SeqInt | Kind::SeqFloat | Kind::SeqText | Kind::SeqBool | Kind::SeqOf(_)
    )
}

// ── World-class maps: any hashable key (the runtime types Int→i64 / Text→str), any VALUE. ──────────────────
// A value of ANY kind rides the map's storage cell as its uniform i64 register encoding — an integer as an exact tagged word, a
// Float as its bits, a Bool as 0/1, a Text as its `char*`, and a Seq/Set/Struct/Enum/Tuple/Map as its heap
// pointer. So the C runtime's `Map_<key>_<i64|str>` (str cell for a Text value, i64 cell for everything else)
// stores it losslessly, and `Index` recovers it with the interned logical value kind — insert/get/length/
// contains work for every value type. `Show` of a whole map renders only the runtime-renderable value kinds
// (Int, Text); a Float/Bool/composite value refuses `Show` (its i64 cell would print as an integer), never
// miscompiling. The (key,value) kinds are interned so `Kind::Map(u16)` stays `Copy`.

/// The C storage cell (`i64`/`str`) for a map KEY — only Int/Text are hashable by the runtime's typed maps.
fn map_key_storage(k: Kind) -> Option<&'static str> {
    match k {
        Kind::Int | Kind::Word(_) => Some("i64"),
        Kind::Text => Some("str"),
        _ => None,
    }
}
/// The C storage cell for a map VALUE — `str` for a Text value, `i64` for every other concrete kind (its
/// register encoding rides the i64 cell); `None` only for a not-yet-known value kind.
fn map_val_storage(k: Kind) -> Option<&'static str> {
    match k {
        Kind::Unknown => None,
        Kind::Text => Some("str"),
        _ => Some("i64"),
    }
}
/// The runtime `Map_<key>_<val>` struct type for a (key,value) kind pair, or `None` if unsupported.
fn map_type_name(key: Kind, val: Kind) -> Option<String> {
    Some(format!(
        "Map_{}_{}",
        map_key_storage(key)?,
        map_val_storage(val)?
    ))
}
/// The runtime function-name prefix `map_<key>_<val>` for a (key,value) kind pair.
fn map_fn(key: Kind, val: Kind) -> Option<String> {
    Some(format!(
        "map_{}_{}",
        map_key_storage(key)?,
        map_val_storage(val)?
    ))
}
/// A map handle register cast to its boxed `Map_<key>_<val>*`.
fn map_ptr_kv(reg: u16, key: Kind, val: Kind) -> Option<String> {
    Some(format!(
        "(({}*)(intptr_t)r[{reg}])",
        map_type_name(key, val)?
    ))
}
/// A register holding a map KEY, as the runtime function's C argument (Int rides its i64 slot, Text is its `char*`).
fn map_key_arg(reg: u16, key: Kind) -> String {
    match key {
        Kind::Text => format!("(const char*)(intptr_t)r[{reg}]"),
        _ => format!("r[{reg}]"),
    }
}
/// A register holding a map VALUE, as the runtime function's C argument — a Text value is its `char*`, any
/// other value uses its i64 cell. Machine integers are tagged to the map's stable exact encoding;
/// exact integers, Float bits, Bool values, and heap pointers are stored verbatim.
fn map_val_store(reg: u16, val: Kind) -> String {
    match val {
        Kind::Text => format!("(char*)(intptr_t)r[{reg}]"),
        Kind::Int => format!("lc_xof(r[{reg}])"),
        _ => format!("r[{reg}]"),
    }
}
/// A `map_*_get(...)` result expression turned back into the i64 register value for the value kind.
fn map_val_load(get: &str, val: Kind) -> String {
    match val {
        Kind::Text => format!("(int64_t)(intptr_t)({get})"),
        _ => get.to_string(),
    }
}
fn is_concrete_map(k: Kind) -> bool {
    matches!(k, Kind::Map(_))
}

/// Map a declared boundary type to the C register kind (scalar + sequence subset — anything else is left
/// `Unknown` and soundly refused if an op needs its kind).
/// `true` if `pc` falls inside any inline-closure hole `[s, e)` — its op belongs to an embedded closure body
/// emitted as its own region, so the enclosing region skips it (control jumps over it at runtime).
fn in_hole(pc: usize, holes: &[(usize, usize)]) -> bool {
    holes.iter().any(|&(s, e)| pc >= s && pc < e)
}

/// A region `[start, end)` of the walker, around its holes, as the variable analysis reads it.
struct WalkerRegion<'e, 'a> {
    emitter: &'e Emitter<'a>,
    start: usize,
    end: usize,
    holes: &'e [(usize, usize)],
}

impl crate::vm::variables::Flow for WalkerRegion<'_, '_> {
    fn pcs(&self) -> Vec<usize> {
        (self.start..self.end).filter(|pc| !in_hole(*pc, self.holes)).collect()
    }

    fn successors(&self, pc: usize) -> Vec<usize> {
        self.emitter.successors(pc, self.start, self.end, self.holes)
    }

    fn written(&self, op: &Op, f: &mut dyn FnMut(u16)) {
        self.emitter.for_each_written(op, f);
    }

    fn read(&self, op: &Op, f: &mut dyn FnMut(u16)) {
        op.for_each_read_reg(|func| self.emitter.prog.functions[func as usize].captures.len(), f);
    }
}

/// If `entry` is an INLINE closure body, return its exclusive end. An inline closure is physically embedded
/// mid-region: the op just before its entry is a forward `Jump` over the (Return-terminated) body — the
/// compiler's jump-over — so its extent is `[entry, jump_target)`. Mirrors `codegen_riscv`'s `is_inline`.
fn is_inline_closure(prog: &CompiledProgram, entry: usize) -> Option<usize> {
    if entry >= 1 {
        if let Op::Jump { target } = prog.code[entry - 1] {
            if target > entry
                && target <= prog.code.len()
                && matches!(
                    prog.code.get(target - 1),
                    Some(Op::Return { .. }) | Some(Op::ReturnNothing)
                )
            {
                return Some(target);
            }
        }
    }
    None
}

fn boundary_to_kind_c(ty: &Option<BoundaryType>) -> Kind {
    match ty {
        Some(BoundaryType::Int) => Kind::Int,
        Some(BoundaryType::Word32) => Kind::Word(32),
        Some(BoundaryType::Word64) => Kind::Word(64),
        Some(BoundaryType::Bool) => Kind::Bool,
        Some(BoundaryType::Float) => Kind::Float,
        Some(BoundaryType::Text) => Kind::Text,
        Some(BoundaryType::Builtin(name)) if name == "Rational" => Kind::Rational,
        Some(BoundaryType::Seq(elem)) => match **elem {
            BoundaryType::Int => Kind::SeqInt,
            BoundaryType::Float => Kind::SeqFloat,
            BoundaryType::Text => Kind::SeqText,
            BoundaryType::Bool => Kind::SeqBool,
            _ => Kind::SeqUnknown,
        },
        Some(BoundaryType::Set(elem)) => match **elem {
            BoundaryType::Int => Kind::SetInt,
            BoundaryType::Word32 => Kind::SetWord(32),
            BoundaryType::Word64 => Kind::SetWord(64),
            BoundaryType::Text => Kind::SetText,
            _ => Kind::SetUnknown,
        },
        _ => Kind::Unknown,
    }
}

/// The result kind of a binary arithmetic op: a `Float` operand promotes the other (so `Int + Float` is
/// `Float`); two equal kinds pass through; an `Unknown` yields to a known partner; anything else is `Unknown`.
fn join(a: Kind, b: Kind) -> Kind {
    if a == b {
        a
    } else if a == Kind::Tower || b == Kind::Tower {
        // A value of the extension holds every number it meets in the runtime's value model.
        Kind::Tower
    } else if a == Kind::Never {
        b
    } else if b == Kind::Never {
        a
    } else if a == Kind::Float || b == Kind::Float {
        Kind::Float
    } else if a == Kind::Rational || b == Kind::Rational {
        // Exact rational promotes its integer partner (`Rational + Int` stays exact), mirroring Float promotion.
        Kind::Rational
    } else if a == Kind::BigInt || b == Kind::BigInt {
        // A bignum operand keeps the result arbitrary-precision (`BigInt + Int` stays exact).
        Kind::BigInt
    } else if a == Kind::Unknown {
        b
    } else if b == Kind::Unknown {
        a
    } else {
        Kind::Unknown
    }
}

/// The kind of a result no larger than one of its operands (`bound`): the operands' join, except that an exact join
/// is a machine integer unless that operand is exact.
fn bounded_by(joined: Kind, bound: Kind) -> Kind {
    if joined == Kind::BigInt && bound != Kind::BigInt {
        Kind::Int
    } else {
        joined
    }
}

/// The kind of register `reg` in a pre-op kind snapshot.
fn kind_at(kinds: &[Kind], reg: u16) -> Kind {
    *kinds.get(reg as usize).unwrap_or(&Kind::Unknown)
}

/// A register read as a `double` — a `Float` slot is bit-cast, an exact integer converted, any other numeric
/// slot is a value cast.
fn as_double(reg: u16, k: Kind) -> String {
    match k {
        Kind::Float => format!("as_f64(r[{reg}])"),
        Kind::BigInt => format!("lint_to_f64(lc_xget(r[{reg}]))"),
        _ => format!("(double)r[{reg}]"),
    }
}

/// A register read as a `const char*` (a `Text` slot holds the pointer reinterpreted as an integer).
fn as_cstr(reg: u16) -> String {
    format!("(const char*)(intptr_t)r[{reg}]")
}

/// A register read as an `LDec` (by value, for the runtime's by-value `ldec_*` arithmetic): a `Rational` slot
/// dereferences its boxed `LDec*`; an `Int` promotes losslessly via `ldec_from_i64`. Any other kind is refused.
fn as_ldec(reg: u16, k: Kind) -> Result<String, String> {
    match k {
        Kind::Rational => Ok(format!("(*(LDec*)(intptr_t)r[{reg}])")),
        Kind::Int => Ok(format!("ldec_from_i64(r[{reg}])")),
        Kind::BigInt => Ok(format!("ldec_from_lint(lc_xget(r[{reg}]))")),
        other => Err(format!(
            "expected a Rational/Int operand for exact arithmetic, got {other:?}"
        )),
    }
}

/// A register read as an exact integer's tagged word (for the `lc_x*` operations): a `BigInt` slot holds one;
/// a machine integer is tagged losslessly.
fn as_exact(reg: u16, k: Kind) -> Result<String, String> {
    match k {
        Kind::BigInt => Ok(format!("r[{reg}]")),
        Kind::Int | Kind::Bool => Ok(format!("lc_xof(r[{reg}])")),
        other => Err(format!(
            "expected a BigInt/Int operand for exact-integer arithmetic, got {other:?}"
        )),
    }
}

/// A C test that register `reg` (an exact divisor of kind `k`) holds zero.
fn exact_zero_test(reg: u16, k: Kind) -> String {
    match k {
        Kind::Rational => format!("((LDec*)(intptr_t)r[{reg}])->a.len == 0"),
        _ => format!("r[{reg}] == 0"),
    }
}

/// [`sample_value`] for a divisor: a number of kind `k` that is not zero, so a division answers about the kinds.
fn sample_divisor(k: Kind) -> Option<RuntimeValue> {
    Some(match k {
        Kind::Int | Kind::BigInt => RuntimeValue::Int(1),
        Kind::Float => RuntimeValue::Float(1.0),
        Kind::Word(bits) => RuntimeValue::Word(match bits {
            8 => logicaffeine_base::WordVal::W8(logicaffeine_base::Word8(1)),
            16 => logicaffeine_base::WordVal::W16(logicaffeine_base::Word16(1)),
            32 => logicaffeine_base::WordVal::W32(logicaffeine_base::Word32(1)),
            _ => logicaffeine_base::WordVal::W64(logicaffeine_base::Word64(1)),
        }),
        other => return sample_value(other),
    })
}

/// A value of kind `k` the VM's own functions can be handed at generation time, to learn what the VM says
/// about that kind (a refusal names the operand's type). `None` for a kind with no such value.
fn sample_value(k: Kind) -> Option<RuntimeValue> {
    use std::rc::Rc;
    Some(match k {
        Kind::Int | Kind::BigInt => RuntimeValue::Int(0),
        Kind::Rational => RuntimeValue::from_rational(
            logicaffeine_base::Rational::from_ratio_i64(1, 2).expect("a nonzero denominator"),
        ),
        Kind::Float => RuntimeValue::Float(0.0),
        Kind::Bool => RuntimeValue::Bool(false),
        Kind::Text => RuntimeValue::Text(Rc::new(String::new())),
        Kind::SeqInt
        | Kind::SeqFloat
        | Kind::SeqText
        | Kind::SeqBool
        | Kind::SeqOf(_)
        | Kind::SeqUnknown => RuntimeValue::List(Rc::new(std::cell::RefCell::new(
            crate::interpreter::ListRepr::from_values(Vec::new()),
        ))),
        Kind::Tuple(_) => RuntimeValue::Tuple(Rc::new(Vec::new())),
        Kind::Map(_) | Kind::MapUnknown => RuntimeValue::Map(Rc::new(std::cell::RefCell::new(
            crate::interpreter::MapStorage::default(),
        ))),
        Kind::Peer => RuntimeValue::Peer(Rc::new(String::new())),
        Kind::Word(bits) => RuntimeValue::Word(match bits {
            8 => logicaffeine_base::WordVal::W8(logicaffeine_base::Word8(0)),
            16 => logicaffeine_base::WordVal::W16(logicaffeine_base::Word16(0)),
            32 => logicaffeine_base::WordVal::W32(logicaffeine_base::Word32(0)),
            _ => logicaffeine_base::WordVal::W64(logicaffeine_base::Word64(0)),
        }),
        _ => return None,
    })
}

/// A register coerced to a `const char*` for text concatenation — a `Text` is the pointer itself; any other
/// scalar is formatted to a fresh string exactly as `Show` renders it (the VM coerces operands of `combined
/// with` / interpolation to text this way).
fn coerce_cstr(reg: u16, k: Kind) -> String {
    match k {
        Kind::Text => as_cstr(reg),
        Kind::Bool => format!("(r[{reg}] ? \"true\" : \"false\")"),
        Kind::Float => format!("logos_fmt_f64(as_f64(r[{reg}]))"),
        Kind::Rational => format!("ldec_to_string((LDec*)(intptr_t)r[{reg}])"),
        Kind::BigInt => format!("lint_to_string(lc_xget(r[{reg}]))"),
        Kind::Tower => format!("ltower_to_string((LTower)(intptr_t)r[{reg}])"),
        _ => format!("logos_dyn_sprintf(\"%\" PRId64, r[{reg}])"),
    }
}

/// A register (holding an element of kind `elem`) read as the C value pushed into a typed `Seq_*`.
fn as_elem_push(reg: u16, elem: Kind) -> String {
    match elem {
        Kind::Float => format!("as_f64(r[{reg}])"),
        Kind::Text => format!("(char*)(intptr_t)r[{reg}]"),
        Kind::Bool => format!("(bool)r[{reg}]"),
        _ => format!("r[{reg}]"),
    }
}

/// A C expression yielding an element of kind `elem`, wrapped back into an `int64_t` register value.
fn elem_to_reg(expr: &str, elem: Kind) -> String {
    match elem {
        Kind::Float => format!("as_bits({expr})"),
        Kind::Text => format!("(int64_t)(intptr_t)({expr})"),
        _ => expr.to_string(),
    }
}

/// Escape a Rust string as a C string-literal body.
fn c_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\{:03o}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// The walker's own C helpers, emitted once after the runtime: the bit casts, the scalar-arithmetic helpers
/// (integer power, floor division rounding toward −∞, the VM's exact `is approximately` tolerance), set
/// copies, UTF-8 character walking, and every check that fails a run — each failing in the VM's own words,
/// which [`words`] takes from the VM's functions at generation time.
fn walker_preamble() -> String {
    let utf8 = words::invalid_utf8();
    WALKER_PREAMBLE
        .replace("@FLOOR_DIVISION_BY_ZERO@", &words::floor_division_by_zero())
        .replace(
            "@REAL_FLOOR_DIVISION_BY_ZERO@",
            &words::real_floor_division_by_zero(),
        )
        .replace("@EXACT_DIVISION_BY_ZERO@", &words::exact_division_by_zero())
        .replace("@NEGATIVE_EXPONENT@", &words::negative_exponent())
        .replace("@EXPONENT_TOO_LARGE@", &words::exponent_too_large())
        .replace("@BIG_EXPONENT@", &words::big_exponent_refused())
        .replace("@DIVISION_BY_ZERO@", &words::division_by_zero())
        .replace("@MODULO_BY_ZERO@", &words::modulo_by_zero())
        .replace("@UTF8_1@", &utf8[0])
        .replace("@UTF8_2@", &utf8[1])
        .replace("@UTF8_3@", &utf8[2])
        .replace("@UTF8_INCOMPLETE@", &utf8[3])
        .replace("@NOT_AN_INT@", &words::not_an_int())
        .replace("@NOT_A_CHARACTER@", &words::not_a_character())
        .replace("@CANNOT_READ_FILE@", &words::cannot_read_file())
        .replace("@RANDOM_BOUNDS@", &words::random_bounds_refused())
}

const WALKER_PREAMBLE: &str = "\
#include <errno.h>
static inline double as_f64(int64_t b){ double d; __builtin_memcpy(&d,&b,8); return d; }
static inline int64_t as_bits(double d){ int64_t b; __builtin_memcpy(&b,&d,8); return b; }
static inline int64_t ipow64(int64_t b, int64_t e){ int64_t r=1; if(e<0) logos_fatalf(@NEGATIVE_EXPONENT@); while(e){ if(e&1) r*=b; e>>=1; if(e) b*=b; } return r; }
static inline int64_t fldiv64(int64_t a, int64_t b){ if(b==0) logos_fatalf(@FLOOR_DIVISION_BY_ZERO@); if(b==-1) return (int64_t)(0-(uint64_t)a); int64_t q=a/b, r=a%b; if(r!=0 && ((r<0)!=(b<0))) q--; return q; }
static inline double lc_rfldiv(double a, double b){ if(b==0.0) logos_fatalf(@REAL_FLOOR_DIVISION_BY_ZERO@); return floor(a/b); }
static inline void lc_exact_divisor(int zero){ if(zero) logos_fatalf(@EXACT_DIVISION_BY_ZERO@); }
static inline int lc_approx_eq(double a, double b){ if(a==b) return 1; double d=(a-b)<0?(b-a):(a-b); double x=a<0?-a:a, y=b<0?-b:b, m=x>y?x:y, t=1e-9*m; if(t<1e-12) t=1e-12; return d<=t; }
/* Each enum cell buffer is a traced object; its children live exactly as long as a reachable parent. */
__attribute__((unused)) static int64_t *lc_enum_alloc(size_t n){
    if(n > SIZE_MAX / sizeof(int64_t)) logos_fatal(\"memory allocation failed\");
    return (int64_t*)lc_gc_alloc(n * sizeof(int64_t));
}
static Set_i64 lc_set_i64_copy(Set_i64 *s){ Set_i64 r=set_i64_new(); for(size_t oi=0;oi<s->ord_len;oi++){ size_t si=s->order[oi]; if(s->state[si]) set_i64_add(&r,s->keys[si]); } return r; }
static Set_str lc_set_str_copy(Set_str *s){ Set_str r=set_str_new(); for(size_t oi=0;oi<s->ord_len;oi++){ size_t si=s->order[oi]; if(s->state[si]) set_str_add(&r,s->keys[si]); } return r; }
__attribute__((unused)) static Map_i64_i64 lc_map_i64_i64_copy(Map_i64_i64 *m){ Map_i64_i64 r=map_i64_i64_new(); for(size_t oi=0;oi<m->ord_len;oi++){ size_t si=m->order[oi]; if(m->state[si]) map_i64_i64_set(&r,m->keys[si],m->vals[si]); } return r; }
__attribute__((unused)) static Map_i64_str lc_map_i64_str_copy(Map_i64_str *m){ Map_i64_str r=map_i64_str_new(); for(size_t oi=0;oi<m->ord_len;oi++){ size_t si=m->order[oi]; if(m->state[si]) map_i64_str_set(&r,m->keys[si],m->vals[si]); } return r; }
__attribute__((unused)) static Map_str_i64 lc_map_str_i64_copy(Map_str_i64 *m){ Map_str_i64 r=map_str_i64_new(); for(size_t oi=0;oi<m->ord_len;oi++){ size_t si=m->order[oi]; if(m->state[si]) map_str_i64_set(&r,m->keys[si],m->vals[si]); } return r; }
__attribute__((unused)) static Map_str_str lc_map_str_str_copy(Map_str_str *m){ Map_str_str r=map_str_str_new(); for(size_t oi=0;oi<m->ord_len;oi++){ size_t si=m->order[oi]; if(m->state[si]) map_str_str_set(&r,m->keys[si],m->vals[si]); } return r; }
/* A list of integers as the list of the rationals they equal, each item a boxed `LDec`. */
__attribute__((unused)) static Seq_i64 *lc_seq_i64_as_ldec(Seq_i64 *s){
    Seq_i64 *r=(Seq_i64*)lc_gc_alloc(sizeof(Seq_i64)); *r=seq_i64_new();
    for(size_t i=0;i<s->len;i++){ LDec *d=(LDec*)lc_gc_alloc(sizeof(LDec)); *d=ldec_from_i64(s->data[i]); seq_i64_push(r,(int64_t)(intptr_t)d); }
    return r;
}
/* The byte length of the UTF-8 character that starts with `lead`. */
static inline size_t lc_utf8_len(unsigned char lead){ return lead<0x80?1:lead<0xE0?2:lead<0xF0?3:4; }
/* Every ASCII character as its own one-character Text, so reading one out of a text allocates nothing. */
static char lc_ascii_texts[128][2];
static inline char *lc_ascii_text(unsigned char c){ lc_ascii_texts[c][0]=(char)c; lc_ascii_texts[c][1]=0; return lc_ascii_texts[c]; }
/* `item i of text`: the i-th character (not byte), as its own Text. */
static char *lc_text_item(const char *s, int64_t i){
    size_t chars=0; for(const unsigned char *p=(const unsigned char*)s; *p; p++) if((*p&0xC0)!=0x80) chars++;
    size_t k=logos_idx0(i, chars), at=0;
    const unsigned char *p=(const unsigned char*)s;
    while(at<k){ p+=lc_utf8_len(*p); at++; }
    if(*p<0x80) return lc_ascii_text(*p);
    size_t n=lc_utf8_len(*p); char *c=(char*)lc_gc_alloc_atomic(n+1); memcpy(c,p,n); c[n]=0; return c;
}
__attribute__((noreturn)) static void lc_utf8_fail(int error_len, size_t at){
    switch(error_len){
    case 1: logos_fatalf(@UTF8_1@, (int64_t)at);
    case 2: logos_fatalf(@UTF8_2@, (int64_t)at);
    case 3: logos_fatalf(@UTF8_3@, (int64_t)at);
    default: logos_fatalf(@UTF8_INCOMPLETE@, (int64_t)at);
    }
}
/* `text_from_bytes(bytes)`: each Int's low byte, validated as UTF-8 exactly as Rust's `str::from_utf8` validates
   it, so bytes that are not UTF-8 fail at the same offset with the same sentence. */
static char *lc_text_from_bytes(Seq_i64 *b){
    size_t n=b->len; unsigned char *v=(unsigned char*)lc_gc_alloc_atomic(n+1);
    for(size_t k=0;k<n;k++) v[k]=(unsigned char)(b->data[k]&0xff);
    v[n]=0;
    size_t i=0;
    while(i<n){
        size_t at=i; unsigned char c=v[i];
        if(c<0x80){ i++; continue; }
        if(c>=0xC2 && c<=0xDF){
            if(++i>=n) lc_utf8_fail(0,at);
            if((v[i]&0xC0)!=0x80) lc_utf8_fail(1,at);
        } else if(c>=0xE0 && c<=0xEF){
            if(++i>=n) lc_utf8_fail(0,at);
            unsigned char d=v[i];
            int ok=(c==0xE0)?(d>=0xA0&&d<=0xBF):(c==0xED)?(d>=0x80&&d<=0x9F):(d>=0x80&&d<=0xBF);
            if(!ok) lc_utf8_fail(1,at);
            if(++i>=n) lc_utf8_fail(0,at);
            if((v[i]&0xC0)!=0x80) lc_utf8_fail(2,at);
        } else if(c>=0xF0 && c<=0xF4){
            if(++i>=n) lc_utf8_fail(0,at);
            unsigned char d=v[i];
            int ok=(c==0xF0)?(d>=0x90&&d<=0xBF):(c==0xF4)?(d>=0x80&&d<=0x8F):(d>=0x80&&d<=0xBF);
            if(!ok) lc_utf8_fail(1,at);
            if(++i>=n) lc_utf8_fail(0,at);
            if((v[i]&0xC0)!=0x80) lc_utf8_fail(2,at);
            if(++i>=n) lc_utf8_fail(0,at);
            if((v[i]&0xC0)!=0x80) lc_utf8_fail(3,at);
        } else lc_utf8_fail(1,at);
        i++;
    }
    return (char*)v;
}
/* An exact integer rides its register as a tagged word: even, the value shifted left once (any integer within 63
   bits); odd, a boxed `LInt*` with its low bit set. Each operation stays on the word while its result fits one, and
   goes through the `lint_*` tower when it does not. */
#define LC_XLIM ((int64_t)1 << 62)
static inline LInt lc_xget(int64_t w){ return (w & 1) ? *(LInt*)(intptr_t)(w ^ 1) : lint_small(w >> 1); }
static inline int64_t lc_xput(LInt v){
    if(!v.big && v.small >= -LC_XLIM && v.small < LC_XLIM) return (int64_t)((uint64_t)v.small << 1);
    LInt *b=(LInt*)lc_gc_alloc(sizeof(LInt)); *b=v; return (int64_t)(intptr_t)b | 1;
}
static inline int64_t lc_xof(int64_t v){ return (v >= -LC_XLIM && v < LC_XLIM) ? (int64_t)((uint64_t)v << 1) : lc_xput(lint_small(v)); }
/* The exact integer as a machine one, where only a machine integer can go. */
__attribute__((unused)) static inline int64_t lc_xnarrow(int64_t w){ return (w & 1) ? lint_expect_i64(*(LInt*)(intptr_t)(w ^ 1)) : (w >> 1); }
/* The low 64 bits of an exact integer, reduced modulo 2^64 whatever its width and sign: what a word built from it holds. */
__attribute__((unused)) static inline uint64_t lc_xword_bits(int64_t w){ return (w & 1) ? lint_word_bits(*(LInt*)(intptr_t)(w ^ 1)) : (uint64_t)(w >> 1); }
/* The exact integer as an index, refused past a machine word with the VM's sentence for that index (`why`). */
__attribute__((unused)) static inline int64_t lc_xindex(int64_t w, const char *why){ return (w & 1) ? lint_index_i64(*(LInt*)(intptr_t)(w ^ 1), why) : (w >> 1); }
/* Whether the exact integer is within i64 — what the VM holds as an `Int` — and its value when it is. */
__attribute__((unused)) static int lc_xi64(int64_t w, int64_t *v){
    if(!(w & 1)){ *v=w >> 1; return 1; }
    LInt x=*(LInt*)(intptr_t)(w ^ 1);
    if(!x.big){ *v=x.small; return 1; }
    LBig lo=lbig_from_i64(INT64_MIN), hi=lbig_from_i64(INT64_MAX);
    int in=lbig_cmp(x.big, &lo) >= 0 && lbig_cmp(x.big, &hi) <= 0; lbig_free(&lo); lbig_free(&hi);
    if(in) *v=lint_expect_i64(x);
    return in;
}
__attribute__((unused)) static inline int64_t lc_xadd(int64_t a, int64_t b){ int64_t s; if(!((a|b)&1) && !__builtin_add_overflow(a,b,&s)) return s; return lc_xput(lint_add(lc_xget(a), lc_xget(b))); }
__attribute__((unused)) static inline int64_t lc_xsub(int64_t a, int64_t b){ int64_t s; if(!((a|b)&1) && !__builtin_sub_overflow(a,b,&s)) return s; return lc_xput(lint_sub(lc_xget(a), lc_xget(b))); }
__attribute__((unused)) static inline int64_t lc_xmul(int64_t a, int64_t b){ int64_t s; if(!((a|b)&1) && !__builtin_mul_overflow(a>>1,b,&s)) return s; return lc_xput(lint_mul(lc_xget(a), lc_xget(b))); }
__attribute__((unused)) static inline int lc_xcmp(int64_t a, int64_t b){ if(!((a|b)&1)) return (a>b)-(a<b); return lint_cmp(lc_xget(a), lc_xget(b)); }
__attribute__((unused)) static int64_t lc_xdiv(int64_t a, int64_t b){
    if(b==0) logos_fatalf(@DIVISION_BY_ZERO@);
    if(!((a|b)&1)) return lc_xof((a>>1)/(b>>1));
    return lc_xput(lint_div(lc_xget(a), lc_xget(b)));
}
__attribute__((unused)) static int64_t lc_xmod(int64_t a, int64_t b){
    if(b==0) logos_fatalf(@MODULO_BY_ZERO@);
    if(!((a|b)&1)) return (int64_t)((uint64_t)((a>>1)%(b>>1)) << 1);
    return lc_xput(lint_mod(lc_xget(a), lc_xget(b)));
}
__attribute__((unused)) static int64_t lc_xfloordiv(int64_t a, int64_t b){
    if(b==0) logos_fatalf(@FLOOR_DIVISION_BY_ZERO@);
    if(!((a|b)&1)){ int64_t x=a>>1, y=b>>1, q=x/y, m=x%y; if(m!=0 && ((m<0)!=(y<0))) q--; return lc_xof(q); }
    LInt x=lc_xget(a), y=lc_xget(b), q=lint_div(x, y), m=lint_mod(x, y);
    int ms=lint_cmp(m, lint_small(0)), ys=lint_cmp(y, lint_small(0));
    if(ms!=0 && ((ms<0)!=(ys<0))){
        LInt previous=q;
        q=lint_sub(q, lint_small(1));
        if(previous.big){ lbig_free(previous.big); lc_gc_free(previous.big); }
    }
    if(m.big){ lbig_free(m.big); lc_gc_free(m.big); }
    return lc_xput(q);
}
/* `b ** e` on machine words: 1 when the power outgrows the word, else 0 with the power in `*out`. An exponent the VM
   refuses fails the run in its words. */
__attribute__((unused)) static inline int lc_ipow_ovf(int64_t b, int64_t e, int64_t *out){
    if(e<0) logos_fatalf(@NEGATIVE_EXPONENT@);
    if(e>(int64_t)UINT32_MAX) logos_fatalf(@EXPONENT_TOO_LARGE@);
    int64_t p=1; uint64_t k=(uint64_t)e;
    while(k){
        if((k&1) && __builtin_mul_overflow(p, b, &p)) return 1;
        k>>=1;
        if(k && __builtin_mul_overflow(b, b, &b)) return 1;
    }
    *out=p; return 0;
}
/* `a ** e`: a machine base stays on the word while the power fits one. */
__attribute__((unused)) static int64_t lc_xpow(int64_t a, int64_t e){
    if(a & 1){
        if(e<0 || e>(int64_t)UINT32_MAX) logos_fatalf(@BIG_EXPONENT@);
    } else {
        if(e<0) logos_fatalf(@NEGATIVE_EXPONENT@);
        if(e>(int64_t)UINT32_MAX) logos_fatalf(@EXPONENT_TOO_LARGE@);
        int64_t p=1, b=a>>1; uint64_t k=(uint64_t)e; int fits=1;
        while(k){
            if((k&1) && __builtin_mul_overflow(p, b, &p)){ fits=0; break; }
            k>>=1;
            if(k && __builtin_mul_overflow(b, b, &b)){ fits=0; break; }
        }
        if(fits) return lc_xof(p);
    }
    LBig b=lint_as_big(lc_xget(a)), p=lbig_pow(&b, (uint64_t)e); lbig_free(&b); return lc_xput(lint_norm(p));
}
/* The bitwise operations over exact integers: the machine's on two words, the runtime's `lint_*` past them. */
__attribute__((unused)) static int64_t lc_xbitand(int64_t a, int64_t b){ if(!((a|b)&1)) return (a>>1) & (b>>1); return lint_bitand(lc_xget(a), lc_xget(b)); }
__attribute__((unused)) static int64_t lc_xbitor(int64_t a, int64_t b){ if(!((a|b)&1)) return (a>>1) | (b>>1); return lint_bitor(lc_xget(a), lc_xget(b)); }
__attribute__((unused)) static int64_t lc_xbitxor(int64_t a, int64_t b){ if(!((a|b)&1)) return (a>>1) ^ (b>>1); return lint_bitxor(lc_xget(a), lc_xget(b)); }
__attribute__((unused)) static int64_t lc_xshl(int64_t a, int64_t b){ if(!((a|b)&1)) return (int64_t)((uint64_t)(a>>1) << ((uint32_t)(b>>1) & 63u)); return lint_shl(lc_xget(a), lc_xget(b)); }
__attribute__((unused)) static int64_t lc_xshr(int64_t a, int64_t b){ if(!((a|b)&1)) return (a>>1) >> ((uint32_t)(b>>1) & 63u); return lint_shr(lc_xget(a), lc_xget(b)); }
/* `parseInt(text)`: the text trimmed as Rust trims it, read as Rust reads an i64 — an optional sign, then only
   digits, within range — or the VM's failure naming the text as given. */
__attribute__((unused)) static int64_t lc_parse_int(const char *s){
    char *t=logos_trim(s); const char *p=t; int neg=0;
    if(*p=='+'||*p=='-'){ neg=(*p=='-'); p++; }
    if(!*p) logos_fatalf(@NOT_AN_INT@, s);
    uint64_t v=0, limit=neg?(uint64_t)INT64_MAX+1u:(uint64_t)INT64_MAX;
    for(; *p; p++){
        if(*p<'0'||*p>'9') logos_fatalf(@NOT_AN_INT@, s);
        uint64_t d=(uint64_t)(*p-'0');
        if(v>(limit-d)/10) logos_fatalf(@NOT_AN_INT@, s);
        v=v*10+d;
    }
    return neg?(int64_t)(0-v):(int64_t)v;
}
/* A text a register grows by appending: the characters are preceded by this head, which keeps what a plain C
   string would have to rescan for — its byte length, its capacity, its character count, whether it is ASCII.
   Only a register whose owned flag is set holds one (it built it, and nothing else holds it), so only such a
   register ever reads the head or grows the text in place. */
typedef struct { size_t len, cap, chars; int ascii; } lc_text_head;
static inline lc_text_head *lc_head(int64_t text){ return ((lc_text_head*)(intptr_t)text) - 1; }
/* `text + tail`, appended in place when the register owns its text (`owned`), otherwise into a fresh growable
   text the register will own — amortized linear, however long the text grows. */
static int64_t lc_text_append(int64_t text, int owned, const char *tail){
    size_t tl=strlen(tail), tc=0; int ta=1;
    for(const unsigned char *p=(const unsigned char*)tail; *p; p++){ if((*p&0xC0)!=0x80) tc++; if(*p>=0x80) ta=0; }
    lc_text_head *h;
    if(owned){ h=lc_head(text); }
    else {
        const char *s=(const char*)(intptr_t)text; size_t sl=strlen(s), sc=0; int sa=1;
        for(const unsigned char *p=(const unsigned char*)s; *p; p++){ if((*p&0xC0)!=0x80) sc++; if(*p>=0x80) sa=0; }
        size_t cap=(sl+tl)*2+16;
        h=(lc_text_head*)lc_gc_alloc_atomic(sizeof(lc_text_head)+cap+1);
        h->len=sl; h->cap=cap; h->chars=sc; h->ascii=sa;
        memcpy((char*)(h+1), s, sl+1);
    }
    if(h->len+tl>h->cap){ size_t cap=(h->len+tl)*2; h=(lc_text_head*)lc_gc_realloc_atomic(h, sizeof(lc_text_head)+cap+1); h->cap=cap; }
    memcpy((char*)(h+1)+h->len, tail, tl+1);
    h->len+=tl; h->chars+=tc; h->ascii=h->ascii&&ta;
    return (int64_t)(intptr_t)(char*)(h+1);
}
/* Append an Int without allocating through logos_dyn_sprintf. The unsigned
   subtraction preserves INT64_MIN's magnitude without signed overflow. */
static int64_t lc_text_append_i64(int64_t text, int owned, int64_t value){
    char digits[21]; size_t at=sizeof digits;
    uint64_t magnitude=value<0 ? (uint64_t)0-(uint64_t)value : (uint64_t)value;
    do {
        digits[--at]=(char)('0'+magnitude%10);
        magnitude/=10;
    } while(magnitude);
    if(value<0) digits[--at]='-';
    lc_text_head *h;
    if(owned) h=lc_head(text);
    else {
        const char *s=(const char*)(intptr_t)text; size_t sl=strlen(s), sc=0; int sa=1;
        for(const unsigned char *p=(const unsigned char*)s; *p; p++){
            if((*p&0xC0)!=0x80) sc++;
            if(*p>=0x80) sa=0;
        }
        size_t cap=(sl+sizeof digits-at)*2+16;
        h=(lc_text_head*)lc_gc_alloc_atomic(sizeof(lc_text_head)+cap+1);
        h->len=sl; h->cap=cap; h->chars=sc; h->ascii=sa;
        memcpy((char*)(h+1), s, sl+1);
    }
    size_t len=sizeof digits-at;
    if(h->len+len>h->cap){
        size_t cap=(h->len+len)*2;
        h=(lc_text_head*)lc_gc_realloc_atomic(h, sizeof(lc_text_head)+cap+1);
        h->cap=cap;
    }
    memcpy((char*)(h+1)+h->len, digits+at, len);
    h->len+=len; h->chars+=len; ((char*)(h+1))[h->len]=0;
    return (int64_t)(intptr_t)(char*)(h+1);
}
/* `length of text` — from the head when the register owns its growing text. */
static inline int64_t lc_text_len(int64_t text, int owned){ return owned ? (int64_t)lc_head(text)->len : (int64_t)strlen((const char*)(intptr_t)text); }
/* `item i of text` — in constant time when the register owns an ASCII growing text. */
static inline char *lc_text_item_of(int64_t text, int owned, int64_t i){
    if(owned){ lc_text_head *h=lc_head(text); if(h->ascii && i>=1 && (uint64_t)i<=(uint64_t)h->len) return lc_ascii_text((unsigned char)((const char*)(intptr_t)text)[i-1]); }
    return lc_text_item((const char*)(intptr_t)text, i);
}
/* `chr(code)`: the character with that Unicode scalar value, or the VM's failure for a code that names none
   (a surrogate, or past U+10FFFF — judged on the low 32 bits, as `char::from_u32(code as u32)` judges it). */
__attribute__((unused)) static char *lc_chr(int64_t code){
    uint32_t c=(uint32_t)code;
    if(c>0x10FFFF || (c>=0xD800 && c<=0xDFFF)) logos_fatalf(@NOT_A_CHARACTER@, code);
    return logos_chr(code);
}
/* `tryReadBytes(path)`: the file's bytes from the real file system, `Ok` (tag `ok`) with a Seq of Int, or
   `Err` (tag `err`) with the facade's sentence for why it could not be read. */
__attribute__((unused)) static int64_t lc_try_read_bytes(const char *path, int64_t ok, int64_t err){
    int64_t *res=(int64_t*)lc_gc_calloc(2, sizeof(int64_t));
    FILE *f=fopen(path, \"rb\");
    Seq_i64 *bytes=(Seq_i64*)lc_gc_alloc(sizeof(Seq_i64)); *bytes=seq_i64_new();
    int fail=0;
    if(!f) fail=errno;
    else {
        unsigned char buf[4096]; size_t n;
        while((n=fread(buf, 1, sizeof buf, f))>0) for(size_t i=0;i<n;i++) seq_i64_push(bytes, (int64_t)buf[i]);
        if(ferror(f)) fail=errno;
        fclose(f);
    }
    if(fail){ res[0]=err; res[1]=(int64_t)(intptr_t)logos_dyn_sprintf(@CANNOT_READ_FILE@, path, strerror(fail), fail); }
    else { res[0]=ok; res[1]=(int64_t)(intptr_t)bytes; }
    return (int64_t)(intptr_t)res;
}
/* The text of the environment variable `name`, `\"\"` when it is unset: with whether it is set, the two
   halves the VM expands `get(name)` into. */
__attribute__((unused)) static char *lc_env_value(const char *name){
    const char *v=getenv(name);
    return lc_gc_strdup(v ? v : \"\");
}
/* The program's argument vector, as `main` received it. */
static int lc_argc; static char **lc_argv;
/* A growing string, for showing a value made of other values. */
typedef struct { char *s; size_t len, cap; } lc_sb;
__attribute__((unused)) static void lc_sb_add(lc_sb *b, const char *t){
    size_t n=strlen(t);
    if(b->len+n+1>b->cap){ size_t c=b->cap?b->cap*2:64; while(c<b->len+n+1) c*=2; b->s=(char*)lc_gc_realloc_atomic(b->s,c); b->cap=c; }
    memcpy(b->s+b->len,t,n+1); b->len+=n;
}
";

/// An in-flight `for-each` iterator frame (innermost last on the emitter's `iter_stack`). `IterNext`/`IterPop`
/// carry no iterable — they operate on the top frame, exactly as the VM's iterator stack does.
struct IterFrame {
    /// The sequence kind (`SeqInt`/…), so `IterNext` casts to the right `Seq_*` and yields the right element;
    /// `Text` for a walk over a text's characters.
    seq_kind: Kind,
    /// This frame walks a borrowed UTF-8 byte view rather than a boxed Seq.
    text_bytes: bool,
    /// The C ids of the loop's snapshot of the iterated handle (`_itv{id}`) and its position (`_it{id}`).
    id: usize,
}

/// Lower a whole `CompiledProgram` to a self-contained C translation unit. Returns a typed refusal (`Err`) for
/// any op/const/kind not yet lowered — the caller falls back to the AST emitter.
pub fn compile_program_to_c(prog: &CompiledProgram) -> Result<String, String> {
    compile_program_to_c_with_policy(prog, None)
}

/// The Op-walker with an optional policy context — the `(interner, PolicyRegistry)` a `CheckPolicy` op needs
/// to resolve its predicate/capability name into a condition (the bytecode carries only the name, exactly as
/// the VM consults its `policy_ctx`). A program with no `Check` statement passes `None` and emits byte-identical
/// C to before. This is the seam `compile_to_c_via_bytecode` uses so runtime authorization lowers natively.
pub fn compile_program_to_c_with_policy<'a>(
    prog: &'a CompiledProgram,
    policy_ctx: Option<(&'a Interner, &'a PolicyRegistry)>,
) -> Result<String, String> {
    compile_program_to_c_with_offloads(prog, policy_ctx, &[])
}

/// A function whose calls a device may answer: its index in the program, the device region that answers it, and how
/// each parameter and the result cross the boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offload {
    pub func: usize,
    pub region: u32,
    pub params: Vec<SlotKind>,
    pub ret: SlotKind,
}

/// The walker's C for a program with an offload table, and each offloaded function it keeps on the host with the
/// reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffloadedC {
    pub c: String,
    pub kept: Vec<(usize, String)>,
}

/// The Op-walker with an offload table ([`compile_program_to_c_offloaded`]); an empty table is
/// [`compile_program_to_c_with_policy`].
pub fn compile_program_to_c_with_offloads<'a>(
    prog: &'a CompiledProgram,
    policy_ctx: Option<(&'a Interner, &'a PolicyRegistry)>,
    offloads: &[Offload],
) -> Result<String, String> {
    compile_program_to_c_offloaded(prog, policy_ctx, offloads).map(|o| o.c)
}

/// The Op-walker with an offload table. Each function the table names asks its device first, at its entry, so every
/// caller's call is offered: `logos_dev_call(region, args, &out)` with the parameters in their ports, whose DONE answer
/// is the call's result and whose DECLINED answer runs the body the walker emits for it anyway. A function a port
/// cannot serve — a parameter or result whose settled kind the port does not carry — stays on the host, reported in
/// `kept`. The driver's definition is the caller's to append.
pub fn compile_program_to_c_offloaded<'a>(
    prog: &'a CompiledProgram,
    policy_ctx: Option<(&'a Interner, &'a PolicyRegistry)>,
    offloads: &[Offload],
) -> Result<OffloadedC, String> {
    // Every function a `MakeClosure` builds is a closure body (hidden closure-ptr param + capture prologue).
    let closure_bodies: std::collections::BTreeSet<usize> = prog
        .code
        .iter()
        .filter_map(|op| match op {
            Op::MakeClosure { func, .. } => Some(*func as usize),
            _ => None,
        })
        .collect();
    let mut em = Emitter {
        prog,
        fn_ret: Vec::new(),
        global_kinds: Vec::new(),
        out: String::new(),
        iter_stack: Vec::new(),
        tmp: 0,
        tuple_types: std::cell::RefCell::new(Vec::new()),
        map_types: std::cell::RefCell::new(Vec::new()),
        inductive_types: std::cell::RefCell::new(Vec::new()),
        seq_elems: std::cell::RefCell::new(Vec::new()),
        param_kinds: Vec::new(),
        capture_kinds: Vec::new(),
        current_fn: None,
        net_message: std::cell::Cell::new(Kind::Unknown),
        net_stream: std::cell::Cell::new(Kind::Unknown),
        ctor_tags: ctor_tags(prog),
        exact_int: std::cell::Cell::new(false),
        closure_bodies,
        counter_fields: std::collections::HashSet::new(),
        synth_structs: std::cell::RefCell::new(Vec::new()),
        policy_ctx,
        owned: std::collections::BTreeSet::new(),
        handle_kinds: std::collections::BTreeMap::new(),
        region_span: (0, 0),
        region_targets: std::collections::HashSet::new(),
        live_out: Vec::new(),
        exact_defs: std::collections::HashSet::new(),
        speculative_defs: std::collections::HashSet::new(),
        exact_outs: std::collections::HashSet::new(),
        entry_speculative: std::collections::HashMap::new(),
        on_words: std::cell::Cell::new(false),
        promoted: std::cell::RefCell::new([
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
        ]),
        text_byte_calls: std::collections::HashSet::new(),
        text_byte_iters: std::collections::HashSet::new(),
        label: 'L',
        deopt: std::collections::BTreeSet::new(),
        arrivals: std::collections::BTreeSet::new(),
        word_twins: std::collections::HashSet::new(),
        word_entry: false,
        twin_arrival: false,
        held_body: String::new(),
        held_owns: false,
        held_entries: Vec::new(),
        held_start: None,
        offloads: offloads.to_vec(),
        kept: Vec::new(),
        tower: false,
        tower_from: std::cell::RefCell::new([
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        ]),
        tower_entries: std::cell::RefCell::new(std::collections::HashMap::new()),
        number_from: std::cell::RefCell::new([
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        ]),
        number_entries: std::cell::RefCell::new(std::collections::HashMap::new()),
    };
    let fn_ret: Vec<Kind> = prog
        .functions
        .iter()
        .map(|f| em.boundary_kind(&f.return_type))
        .collect();
    em.fn_ret = fn_ret;
    em.compile()?;
    Ok(OffloadedC {
        c: em.out,
        kept: em.kept,
    })
}

struct Emitter<'a> {
    prog: &'a CompiledProgram,
    fn_ret: Vec<Kind>,
    /// The inferred kind of each module-level global (indexed like `prog.globals`), derived from its
    /// `GlobalSet` sites so a `GlobalGet` yields the right register kind.
    global_kinds: Vec<Kind>,
    out: String,
    /// Active `for-each` iterators (innermost last), reset per region.
    iter_stack: Vec<IterFrame>,
    /// Monotonic fresh-temp counter (unique C locals across the whole unit).
    tmp: usize,
    /// Interning table for anonymous tuple layouts: each entry is a tuple's per-position kinds, and a
    /// `Kind::Tuple(idx)` points here. Interior-mutable because it is filled during the `&self` `infer_kinds`
    /// fixpoint (as `NewTuple` ops are seen) and read back in `lower_op`.
    tuple_types: std::cell::RefCell<Vec<Vec<Kind>>>,
    /// Interning table for map layouts: each entry is a map's (key kind, value kind), and a `Kind::Map(idx)`
    /// points here — the value may be ANY kind (it rides the i64/str cell), so the pair is remembered so a
    /// later `Index`/`Show` recovers the true value kind.
    map_types: std::cell::RefCell<Vec<(Kind, Kind)>>,
    /// Interning table for the enums the program never declares (`Kind::Inductive`): the type's name and, per
    /// constructor in tag order, the payload kinds known for it — `None` for a constructor the value can never
    /// be (a `none` is never a `Some`). Interior-mutable (filled during the infer fixpoint).
    inductive_types: std::cell::RefCell<Vec<(String, Vec<(String, Option<Vec<Kind>>)>)>>,
    /// Interning table for the element kinds of `Kind::SeqOf` sequences.
    seq_elems: std::cell::RefCell<Vec<Kind>>,
    /// Every function's parameter kinds, settled over the whole program ([`Emitter::refine_signatures`]).
    param_kinds: Vec<Vec<Kind>>,
    /// Every closure body's captured values, as the `MakeClosure` that builds it stores them, settled with the
    /// parameters: a local as the machine word it holds there, a global as its own slot holds it.
    capture_kinds: Vec<Vec<Kind>>,
    /// The function whose body is being emitted (`None` in Main).
    current_fn: Option<usize>,
    /// The kind of every message the program `Send`s, and of every sequence it `Stream`s — one offline node,
    /// so an `Await` receives exactly what the program sent ([`Emitter::refine_signatures`]).
    net_message: std::cell::Cell<Kind>,
    net_stream: std::cell::Cell<Kind>,
    /// Per undeclared enum type, its constructors in tag order — one program-wide order, so every site that
    /// builds, tests or binds a constructor agrees on its tag. `Option` is `Some, None` and `Result` is `Ok, Err`;
    /// any other type lists its constructors as the program first builds them.
    ctor_tags: std::collections::HashMap<String, Vec<String>>,
    /// Set while emitting/inferring a CLOSURE BODY: its integer `Add`/`Sub`/`Mul`/`Pow` are exact
    /// unconditionally (result → `BigInt`). Bignum promotion does NOT flow through the closure boundary in
    /// the shared `bigint_promote`/`int_exceeds_i64` analyses (the Rust backend PANICS, AST-C wraps), so an
    /// unbounded closure param that overflows would wrap; being exact-by-default inside the rare, perf-insensitive
    /// closure body closes that hole with no perf cost to the hot non-closure paths. Interior-mutable Cell because
    /// `infer_kinds` runs under `&self`.
    exact_int: std::cell::Cell<bool>,
    /// Function indices that are CLOSURE BODIES (referenced by a `MakeClosure`). Each is emitted with a hidden
    /// trailing `int64_t clo` param + a prologue that loads its captured values (and present flags) from the
    /// closure object, and every `CallValue` dispatches into this set by the closure's runtime body index.
    closure_bodies: std::collections::BTreeSet<usize>,
    /// `(struct name, cell index)` of every field that holds a LIVE counter CRDT — a `NewCrdt` kind-4/5 value
    /// flowed into it via a `StructInsert`. The declared field type is `Int`, so this side-set is how
    /// `GetField`/`CrdtMerge` know to SUM / per-replica-MAX rather than treat the cell as a plain integer. Keyed
    /// by struct NAME (unique) so a synthesized-layout struct and a `prog.struct_types` struct share one key
    /// space. Mirrors `codegen_riscv`'s `counter_fields`. Filled by `derive_counter_fields` in `compile()`.
    counter_fields: std::collections::HashSet<(String, usize)>,
    /// Interning table for SYNTHESIZED struct layouts (a `Kind::StructSynth(idx)` points here): each entry is
    /// `(struct name, [(field name, field kind) in cell order])` for a struct whose declared type did not resolve
    /// into `prog.struct_types`. Filled during the `&self` `infer_kinds`/`derive` passes as `NewStruct` on an
    /// unresolved type name is seen: field names grow as field ops on the struct are encountered, and each field
    /// kind is monotonically refined from the value flowed into it (`StructInsert` / `CrdtBump` → the cell's kind),
    /// so a later `GetField`/`Show` renders it correctly (an unresolved `Tally.n` bumped from `Nothing` is `Int`).
    synth_structs: std::cell::RefCell<Vec<(String, Vec<(String, Kind)>)>>,
    /// The `(interner, PolicyRegistry)` a `CheckPolicy` op consults to resolve its predicate/capability name
    /// into a `PolicyCondition` (the bytecode carries only the interned name, so the registry — the same one
    /// the VM's `policy_ctx` uses — is required to compile the condition). `None` when the front-end supplied
    /// no policies: a `CheckPolicy` then lowers to the VM's "Security Check requires policies" trap.
    policy_ctx: Option<(&'a Interner, &'a PolicyRegistry)>,
    /// The current region's registers that some op mutates as the register's own collection
    /// ([`Mutation::Owned`]). Each has an owned flag, `own[r]`: 1 while the register is the only holder of its
    /// collection, so a change happens in place; 0 while the collection may be held elsewhere too, so a change
    /// copies it first — the VM's copy-on-write, decided without reference counts. Every region entry clears
    /// them: a parameter, a global, an element read out of a collection may all be shared.
    owned: std::collections::BTreeSet<u16>,
    /// The kind of handle each register of the current region holds, where every handle the region writes to it is
    /// of that one kind: a sequence such a register holds alone can be emptied in place for the next one.
    handle_kinds: std::collections::BTreeMap<u16, Kind>,
    /// The current region's ops, by absolute pc, and the pcs a jump can land on — what the dead-after scan
    /// ([`Emitter::dead_after`]) walks.
    region_span: (usize, usize),
    region_targets: std::collections::HashSet<usize>,
    /// Exact operation PCs selected by the conservative byte-view recognizer:
    /// TextBytes calls become pointer aliases and their IterPrepare operations
    /// walk those pointers directly.
    text_byte_calls: std::collections::HashSet<usize>,
    text_byte_iters: std::collections::HashSet<usize>,
    /// The registers live after each op of the current region (by `pc - start`): read on some path from there
    /// before being written.
    live_out: Vec<std::collections::BTreeSet<u16>>,
    /// The ops (by pc) whose integer result is held exact ([`Emitter::exactness`]), settled with the signatures, and
    /// those among them exact only because they can grow: a region runs these on machine words until one overflows.
    exact_defs: std::collections::HashSet<usize>,
    speculative_defs: std::collections::HashSet<usize>,
    /// The registers an op writing more than one leaves exact, by (pc, register): a call's in-out arguments, a
    /// swap's two sides. Held exact, a swap side that receives a machine word takes it tagged.
    exact_outs: std::collections::HashSet<(usize, u16)>,
    /// Per function, the parameters an exact variable starts from only because it grows: machine words on entry.
    entry_speculative: std::collections::HashMap<usize, std::collections::BTreeSet<u16>>,
    /// Set while the region's kinds on machine words are inferred or emitted: the speculative ops are not exact.
    on_words: std::cell::Cell<bool>,
    /// The exact ops whose own lowering makes a machine integer, in the region held exact (`[0]`) and on machine
    /// words (`[1]`): their result is tagged once it lands.
    promoted: std::cell::RefCell<[std::collections::HashSet<usize>; 2]>,
    /// The prefix of the labels of the version of the region being emitted: `L` on machine words, `X` held exact.
    label: char,
    /// The ops of the region on machine words that move into the region held exact when they overflow, and those
    /// whose exact result moves it there when it does not fit a word.
    deopt: std::collections::BTreeSet<usize>,
    arrivals: std::collections::BTreeSet<usize>,
    /// The functions whose body is their word entry ([`Emitter::takes_words`]), `fn_N_w`: a caller on machine words
    /// hands it machine words, and its exact result comes back a machine word, or the exact value it escaped to, marked
    /// so; `fn_N` is a wrapper taking tagged words ([`Emitter::emit_tagged_entry`]).
    word_twins: std::collections::HashSet<usize>,
    /// Set while a word entry's body is emitted.
    word_entry: bool,
    /// Set by a call on machine words through a word entry, whose escape test stands in for the arrival's.
    twin_arrival: bool,
    /// A word entry's region held exact, emitted apart as `fn_N_x` ([`Emitter::emit_held_function`]) so the word
    /// entry's frame holds only what its machine words need: its text, whether it keeps owned flags, and each way in
    /// (a resume id, the loops it hands over, the op it resumes at).
    held_body: String,
    held_owns: bool,
    held_entries: Vec<(usize, Vec<usize>, usize, Vec<u16>)>,
    /// How a word entry's tagged entry enters its region held exact when a parameter arrives boxed: the way in, and
    /// the registers live where the region starts.
    held_start: Option<(usize, Vec<u16>)>,
    /// The functions a device may answer ([`compile_program_to_c_offloaded`]), and those kept on the host, with why.
    offloads: Vec<Offload>,
    kept: Vec<(usize, String)>,
    /// Whether the program computes with the exact extension ([`crate::vm::tower::reaches`]): it links the value
    /// runtime and its regions' exact webs are settled as their kinds are.
    tower: bool,
    /// The natural kind each definition of an exact web makes, by version (`[0]` held exact, `[1]` on machine
    /// words) and pc, where it is not a cell: the value it writes is converted into one as it lands.
    tower_from: std::cell::RefCell<[std::collections::HashMap<usize, Kind>; 2]>,
    /// Per region (by its first pc), the parameters of an exact web that arrive as another kind, with that kind:
    /// the region converts each into a cell as it begins.
    tower_entries: std::cell::RefCell<std::collections::HashMap<usize, Vec<(u16, Kind)>>>,
    /// Each definition a number join lands as another kind ([`numbers`]), by version and pc: the kind it computes and
    /// the kind it lands as.
    number_from: std::cell::RefCell<[std::collections::HashMap<usize, (Kind, Kind)>; 2]>,
    /// Per region (by its first pc): the parameters a number join converts as the region begins (the kind each arrives
    /// as and the kind it is converted to), and one definition of each variable no machine form holds.
    number_entries: std::cell::RefCell<
        std::collections::HashMap<usize, (Vec<(u16, Kind, Kind)>, Vec<crate::vm::variables::Site>)>,
    >,
}

impl<'a> Emitter<'a> {
    /// Whether function `fi`'s integer arithmetic runs on the exact `lint_*` tower (see `exact_int`): a closure
    /// body, and any function that returns a value without declaring its type — a function value the optimizer
    /// turned into a direct call is still the lambda it was, and nothing bounds what it returns.
    fn computes_exactly(&self, fi: usize) -> bool {
        if self.closure_bodies.contains(&fi) {
            return true;
        }
        let f = &self.prog.functions[fi];
        if f.return_type.is_some() {
            return false;
        }
        let end = self
            .prog
            .functions
            .iter()
            .map(|g| g.entry_pc)
            .filter(|&e| e > f.entry_pc)
            .min()
            .unwrap_or(self.prog.code.len());
        self.prog.code[f.entry_pc..end]
            .iter()
            .any(|op| matches!(op, Op::Return { .. }))
    }

    /// Whether function `fi`'s body is a word entry, `fn_N_w`: an exact parameter or an exact result it runs on
    /// machine words until a value outgrows one. Its exact parameters take machine words with no tag to test or
    /// strip, and it returns a machine word, or the exact value it moved to marked `escaped` ([`Emitter::ret`]), so a
    /// call between two regions on machine words boxes nothing; its region held exact is a function of its own
    /// ([`Emitter::emit_held_function`]). A function value's body, a function whose
    /// `mutable` parameter is the caller's variable, and one a device may answer keep the one entry.
    fn takes_words(&self, fi: usize) -> bool {
        let exact_param = self.param_kinds[fi].contains(&Kind::BigInt);
        let tower = self.param_kinds[fi].contains(&Kind::Tower) || self.fn_ret[fi] == Kind::Tower;
        (exact_param || self.fn_ret[fi] == Kind::BigInt)
            && !tower
            && !self.computes_exactly(fi)
            && self.inout_regs(fi).is_empty()
            && !self.offloads.iter().any(|o| o.func == fi)
    }

    /// The statement returning `value` from the function being emitted: from a word entry, with whether it escaped to
    /// an exact value.
    fn ret(&self, value: &str, escaped: bool) -> String {
        if self.word_entry {
            format!("return (lc_wret){{ {value}, {} }};", u8::from(escaped))
        } else {
            format!("return {value};")
        }
    }

    /// Intern a tuple's per-position kinds, returning the index a `Kind::Tuple` carries (dedup by value, so
    /// two `(Int, Text)` tuples share one entry).
    fn intern_tuple(&self, ks: Vec<Kind>) -> u16 {
        let mut t = self.tuple_types.borrow_mut();
        if let Some(i) = t.iter().position(|e| e == &ks) {
            return i as u16;
        }
        t.push(ks);
        (t.len() - 1) as u16
    }

    /// The per-position kinds of the tuple layout `idx`.
    fn tuple_positions(&self, idx: u16) -> Vec<Kind> {
        self.tuple_types
            .borrow()
            .get(idx as usize)
            .cloned()
            .unwrap_or_default()
    }

    /// The element kind of a sequence or set kind — a scalar's, or the interned element of a `SeqOf`.
    fn elem_kind(&self, k: Kind) -> Kind {
        match k {
            Kind::SeqOf(ei) => self
                .seq_elems
                .borrow()
                .get(ei as usize)
                .copied()
                .unwrap_or(Kind::Unknown),
            // A sequence made empty that nothing ever puts an element in holds none.
            Kind::SeqUnknown | Kind::SetUnknown => Kind::Never,
            // An exact list's items are exact values.
            Kind::Tower => Kind::Tower,
            other => other.elem(),
        }
    }

    /// The sequence kind whose elements are of kind `elem`: a scalar's own, a `SeqOf` for any other known kind,
    /// `SeqUnknown` for an unknown element.
    fn seq_kind_for(&self, elem: Kind) -> Kind {
        match elem {
            Kind::Int => Kind::SeqInt,
            Kind::Float => Kind::SeqFloat,
            Kind::Text => Kind::SeqText,
            Kind::Bool => Kind::SeqBool,
            Kind::Unknown => Kind::SeqUnknown,
            other => {
                let mut t = self.seq_elems.borrow_mut();
                let i = match t.iter().position(|e| *e == other) {
                    Some(i) => i,
                    None => {
                        t.push(other);
                        t.len() - 1
                    }
                };
                Kind::SeqOf(i as u16)
            }
        }
    }

    /// The tag of constructor `ctor` of the undeclared enum `ty`, in the program-wide order; `None` for a
    /// constructor the program never builds (a test for it is always false).
    fn ctor_tag(&self, ty: &str, ctor: &str) -> Option<usize> {
        self.ctor_tags.get(ty)?.iter().position(|c| c == ctor)
    }

    /// The `Kind::Inductive` of undeclared enum `ty` that can be only the constructors `known` lists, each with
    /// the payload kinds given for it; interned by value.
    fn inductive_kind(&self, ty: &str, known: &[(&str, Vec<Kind>)]) -> Kind {
        let ctors: Vec<(String, Option<Vec<Kind>>)> = self
            .ctor_tags
            .get(ty)
            .map(|cs| cs.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(|c| {
                (
                    c.clone(),
                    known.iter().find(|(k, _)| k == c).map(|(_, ks)| ks.clone()),
                )
            })
            .collect();
        self.intern_inductive(ty.to_string(), ctors)
    }

    fn intern_inductive(&self, ty: String, ctors: Vec<(String, Option<Vec<Kind>>)>) -> Kind {
        let mut t = self.inductive_types.borrow_mut();
        let entry = (ty, ctors);
        let idx = match t.iter().position(|e| *e == entry) {
            Some(i) => i,
            None => {
                t.push(entry);
                t.len() - 1
            }
        };
        Kind::Inductive(idx as u16)
    }

    /// The undeclared enum layout `idx`: its type name and, per constructor in tag order, the payload kinds of
    /// each constructor the value can be.
    fn inductive(&self, idx: u16) -> (String, Vec<(String, Option<Vec<Kind>>)>) {
        self.inductive_types
            .borrow()
            .get(idx as usize)
            .cloned()
            .unwrap_or_default()
    }

    /// The payload kind at `index` of constructor `ctor` of the undeclared enum layout `idx`.
    fn inductive_payload(&self, idx: u16, ctor: &str, index: usize) -> Kind {
        let (_, ctors) = self.inductive(idx);
        ctors
            .iter()
            .find(|(c, _)| c == ctor)
            .and_then(|(_, ks)| ks.as_ref()?.get(index).copied())
            .unwrap_or(Kind::Unknown)
    }

    /// Two layouts of one undeclared enum as one: the value can be any constructor either can be, and a
    /// payload kind known on one side and unknown on the other is known; `None` when they disagree on a kind
    /// both know.
    fn unify_inductive(&self, a: u16, b: u16) -> Option<Kind> {
        let (ta, ca) = self.inductive(a);
        let (tb, cb) = self.inductive(b);
        if ta != tb || ca.len() != cb.len() {
            return None;
        }
        let mut ctors = Vec::with_capacity(ca.len());
        for ((name, ka), (_, kb)) in ca.iter().zip(cb.iter()) {
            let merged = match (ka, kb) {
                (None, None) => None,
                (Some(k), None) | (None, Some(k)) => Some(k.clone()),
                (Some(ka), Some(kb)) => {
                    let n = ka.len().max(kb.len());
                    let mut ks = Vec::with_capacity(n);
                    for i in 0..n {
                        let (x, y) = (
                            ka.get(i).copied().unwrap_or(Kind::Unknown),
                            kb.get(i).copied().unwrap_or(Kind::Unknown),
                        );
                        ks.push(match (x, y) {
                            (Kind::Unknown, y) => y,
                            (x, Kind::Unknown) => x,
                            (x, y) if x == y => x,
                            _ => return None,
                        });
                    }
                    Some(ks)
                }
            };
            ctors.push((name.clone(), merged));
        }
        Some(self.intern_inductive(ta, ctors))
    }

    /// Intern a map's (key, value) kinds, returning the index a `Kind::Map` carries.
    fn intern_map(&self, key: Kind, val: Kind) -> u16 {
        let mut t = self.map_types.borrow_mut();
        if let Some(i) = t.iter().position(|e| *e == (key, val)) {
            return i as u16;
        }
        t.push((key, val));
        (t.len() - 1) as u16
    }
    /// The (key, value) kinds of the map layout `idx`.
    fn map_pair(&self, idx: u16) -> (Kind, Kind) {
        self.map_types
            .borrow()
            .get(idx as usize)
            .copied()
            .unwrap_or((Kind::Int, Kind::Int))
    }
    /// Refine a (key, value) kind pair into a `Kind::Map`: `MapUnknown` while a side is still unknown (an empty
    /// map defaults to `Map_i64_i64` and renders `{}`); `Kind::Unknown` (→ a clean refusal) for a key the
    /// runtime cannot hash by value (anything but an Int, a word or a Text); otherwise an interned concrete `Map`.
    fn map_of(&self, key: Kind, val: Kind) -> Kind {
        // A map may cross function/global and word/exact region boundaries.
        // Its integer cells therefore always use the exact register encoding,
        // including values that currently fit a machine word.
        let key = if key == Kind::BigInt { Kind::Int } else { key };
        let val = if val == Kind::Int { Kind::BigInt } else { val };
        if key == Kind::Unknown || val == Kind::Unknown {
            return Kind::MapUnknown;
        }
        if map_key_storage(key).is_none() {
            return Kind::Unknown;
        }
        Kind::Map(self.intern_map(key, val))
    }
    /// The (key, value) kinds to operate on: an unrefined empty map (`MapUnknown`) acts as `Map_i64_i64`.
    fn resolve_map_kv(&self, k: Kind) -> Option<(Kind, Kind)> {
        match k {
            Kind::Map(idx) => Some(self.map_pair(idx)),
            Kind::MapUnknown => Some((Kind::Int, Kind::BigInt)),
            _ => None,
        }
    }

    /// The compile-time integer that register `reg` holds at `pc`, when the most recent write to it in the
    /// region `[start, pc)` is a `LoadConst` of an `Int`. Positional tuple access (`Let (a, b) be …` lowers to
    /// `Index t 0` / `Index t 1`) always feeds the index from a fresh constant, so this recovers the position
    /// needed to pick the heterogeneous element's kind. Returns `None` for a non-constant index.
    fn const_int_at(&self, start: usize, pc: usize, reg: u16) -> Option<i64> {
        for p in (start..pc).rev() {
            if let Op::LoadConst { dst, idx } = &self.prog.code[p] {
                if *dst == reg {
                    return match &self.prog.constants[*idx as usize] {
                        Constant::Int(v) => Some(*v),
                        _ => None,
                    };
                }
            }
        }
        None
    }

    /// The compile-time Text that register `reg` holds at `pc`, when the most recent write to it in `[start, pc)`
    /// is a `LoadConst` of a `Text` (a field name the bytecode indexes a record by).
    fn const_text_at(&self, start: usize, pc: usize, reg: u16) -> Option<String> {
        for p in (start..pc).rev() {
            let mut writes = false;
            self.for_each_written(&self.prog.code[p], |w| writes |= w == reg);
            if writes {
                return match &self.prog.code[p] {
                    Op::LoadConst { idx, .. } => match &self.prog.constants[*idx as usize] {
                        Constant::Text(t) => Some(t.clone()),
                        _ => None,
                    },
                    _ => None,
                };
            }
        }
        None
    }

    /// The cell of the field named `name` in a record of kind `k`.
    fn field_cell_of_name(&self, k: Kind, name: &str) -> Option<usize> {
        match k {
            Kind::Struct(si) => self
                .prog
                .struct_types
                .get(si as usize)?
                .fields
                .iter()
                .position(|(n, _)| n == name),
            Kind::StructSynth(idx) => self
                .synth_structs
                .borrow()
                .get(idx as usize)?
                .1
                .iter()
                .position(|(n, _)| n == name),
            _ => None,
        }
    }

    /// The result kind of an integer `Add`/`Sub`/`Mul`/`Pow`: exact inside a function value's body
    /// (`exact_int`), otherwise the numeric join of the operands — exact when either is. Which other results are
    /// exact is [`Emitter::exactness`]'s to say.
    fn arith_result_kind(&self, lk: Kind, rk: Kind) -> Kind {
        let j = join(lk, rk);
        if self.exact_int.get() && matches!(j, Kind::Int | Kind::BigInt) {
            Kind::BigInt
        } else {
            j
        }
    }

    /// `r[dst] = func(lhs, rhs)` over exact integers, each operand read as its tagged word.
    fn emit_exact_bin(
        &mut self,
        dst: u16,
        lhs: u16,
        rhs: u16,
        lk: Kind,
        rk: Kind,
        func: &str,
    ) -> Result<(), String> {
        let (l, r) = (as_exact(lhs, lk)?, as_exact(rhs, rk)?);
        self.line(&format!("r[{dst}] = {func}({l}, {r});"));
        Ok(())
    }

    /// `r[dst] = func(lhs, rhs)` over exact integers, whose result fits a machine integer unless it is exact
    /// itself (`result`): a quotient, a remainder, a bitwise combination.
    fn emit_exact_bounded(
        &mut self,
        dst: u16,
        lhs: u16,
        rhs: u16,
        kinds: &[Kind],
        result: Kind,
        func: &str,
    ) -> Result<(), String> {
        let (l, r) = (
            as_exact(lhs, kind_at(kinds, lhs))?,
            as_exact(rhs, kind_at(kinds, rhs))?,
        );
        let value = format!("{func}({l}, {r})");
        let value = match (result, func) {
            (Kind::BigInt, _) => value,
            (_, "lc_xdiv" | "lc_xmod" | "lc_xfloordiv") => format!("lc_xnarrow({value})"),
            _ => value,
        };
        self.line(&format!("r[{dst}] = {value};"));
        Ok(())
    }

    /// Whether an integer op over `operands` producing a `result` runs on exact integers: an operand or the
    /// result is exact, and nothing about them is a Real, a rational or a word.
    fn exact_integers(&self, operands: &[u16], kinds: &[Kind], result: Kind) -> bool {
        let ks: Vec<Kind> = operands.iter().map(|r| kind_at(kinds, *r)).collect();
        (result == Kind::BigInt || ks.contains(&Kind::BigInt))
            && ks
                .iter()
                .all(|k| matches!(k, Kind::Int | Kind::BigInt | Kind::Bool))
    }

    /// Write one indented C line.
    fn line(&mut self, s: &str) {
        self.out.push_str("    ");
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn compile(&mut self) -> Result<(), String> {
        self.out.push_str(C_GC_COLLECTOR);
        self.out.push_str(&c_runtime());
        self.out.push('\n');
        // The exact extension's values live in the linked value runtime, declared only for a program that uses it.
        self.tower = crate::vm::tower::reaches(self.prog);
        if self.tower {
            self.out.push_str(super::tower::RUNTIME);
            self.out.push('\n');
        }
        // The single-replica CRDT lattice (counter map) and the offline loopback networking transport are
        // emitted ONLY when the program uses them, so a non-CRDT / non-net program stays byte-identical to a
        // world without them (mirroring the AST-C tier's conditional inclusion). Scan the flat op stream —
        // Main + every function body live in `code`.
        let uses_crdt = self.prog.code.iter().any(op_is_crdt);
        let uses_net = self.prog.code.iter().any(op_is_net);
        let uses_tcp = self.prog.code.iter().any(op_is_tcp);
        if uses_crdt {
            self.out.push_str(C_CRDT_RUNTIME);
            self.out.push('\n');
        }
        if uses_net {
            self.out.push_str(&c_net_runtime());
            self.out.push('\n');
        }
        // The sandboxed file system — emitted only when the program reads, writes or mounts a file.
        if self.prog.code.iter().any(op_is_file) {
            self.out.push_str(&c_vfs_runtime());
            self.out.push('\n');
        }
        // The real-socket TCP client runtime — emitted only when the program contains a `Tcp` op, so a
        // non-TCP program stays byte-identical to a world without it (mirroring the CRDT/net gates).
        if uses_tcp {
            self.out.push_str(C_TCP_RUNTIME);
            self.out.push('\n');
        }
        self.out.push_str(&walker_preamble());
        self.out.push('\n');

        // ── Inline-closure-aware region layout: an embedded closure body is entered only via `CallValue` and
        // sits mid-region behind a jump-over, so each is emitted as its own region and the enclosing block gets a
        // HOLE there (control jumps over it). Mirrors `codegen_riscv::regions`. ──
        let inline_extents: Vec<(usize, usize)> = self
            .prog
            .functions
            .iter()
            .filter_map(|f| is_inline_closure(self.prog, f.entry_pc).map(|e| (f.entry_pc, e)))
            .collect();
        let mut top_starts: Vec<usize> = vec![0];
        for f in &self.prog.functions {
            if is_inline_closure(self.prog, f.entry_pc).is_none() {
                top_starts.push(f.entry_pc);
            }
        }
        top_starts.sort_unstable();
        top_starts.dedup();
        let code_len = self.prog.code.len();
        let block_end = |s: usize| -> usize {
            top_starts
                .iter()
                .copied()
                .find(|&x| x > s)
                .unwrap_or(code_len)
        };
        // Holes = inline closures embedded STRICTLY INSIDE span `[s, e)` (`hs > s`), so an inline-closure region
        // does not subtract its OWN extent (which begins exactly at `s`) — only nested closures are holes.
        let holes_in = |s: usize, e: usize| -> Vec<(usize, usize)> {
            inline_extents
                .iter()
                .copied()
                .filter(|&(hs, _)| hs > s && hs < e)
                .collect()
        };
        let main_end = block_end(0);
        if main_end == 0 {
            return Err("no Main region".into());
        }
        // Every region — Main and each function — as `(function, start, end, holes)`.
        let mut regions: Vec<(Option<usize>, usize, usize, Vec<(usize, usize)>)> =
            vec![(None, 0, main_end, holes_in(0, main_end))];
        for (fi, f) in self.prog.functions.iter().enumerate() {
            let end =
                is_inline_closure(self.prog, f.entry_pc).unwrap_or_else(|| block_end(f.entry_pc));
            regions.push((Some(fi), f.entry_pc, end, holes_in(f.entry_pc, end)));
        }
        // Module-level globals: a file-scope `int64_t g[N]` (zero-initialized) both Main and the functions reach.
        // Their kinds are settled before the signatures, so the exactness decision sees a global's reads as the
        // integers they are, and again after, so a global an exact integer is stored into reads back exact.
        self.param_kinds = self.declared_param_kinds();
        self.derive_global_kinds(main_end);
        self.refine_signatures(&regions);
        self.derive_global_kinds(main_end);
        // Which struct fields hold a LIVE counter CRDT (a `NewCrdt` value flowed in) — the side-set `GetField`/
        // `CrdtMerge` consult to SUM / per-replica-MAX instead of treating the `Int` cell as a plain integer.
        self.derive_counter_fields(main_end);
        self.word_twins = (0..self.prog.functions.len())
            .filter(|fi| self.takes_words(*fi))
            .collect();
        // The seam between the fixed runtime above and the program's own C below, the one the AST generator writes
        // ([`super::USER_CODE_SENTINEL`]): a reader of either C output finds the program part the same way.
        self.out.push_str(super::USER_CODE_SENTINEL);
        self.out.push('\n');
        if !self.prog.globals.is_empty() {
            writeln!(
                self.out,
                "static int64_t g[{}] = {{0}};",
                self.prog.globals.len()
            )
            .unwrap();
            // Each global's owned flag: set while the global is the only holder of its collection.
            writeln!(
                self.out,
                "static unsigned char gown[{}] = {{0}};",
                self.prog.globals.len()
            )
            .unwrap();
        }

        // The device an offloaded function asks first; the caller appends its driver.
        if !self.offloads.is_empty() {
            writeln!(
                self.out,
                "int logos_dev_call(int region, const int64_t *args, int64_t *out);"
            )
            .unwrap();
        }
        // A word entry's result: a machine word, or the exact value it escaped to, tagged, with `escaped` set — two
        // words, returned in registers.
        if !self.word_twins.is_empty() {
            writeln!(
                self.out,
                "typedef struct {{ int64_t v; int64_t escaped; }} lc_wret;"
            )
            .unwrap();
            // What a word entry hands its region held exact: every register, its owned flags, and each loop's
            // snapshot and position, per thread.
            let regs = self
                .word_twins
                .iter()
                .map(|fi| self.prog.functions[*fi].register_count.max(1))
                .max()
                .unwrap_or(1);
            let loops = self
                .word_twins
                .iter()
                .map(|fi| {
                    let f = &self.prog.functions[*fi];
                    let end = is_inline_closure(self.prog, f.entry_pc)
                        .unwrap_or_else(|| block_end(f.entry_pc));
                    self.prog.code[f.entry_pc..end]
                        .iter()
                        .filter(|op| matches!(op, Op::IterPrepare { .. }))
                        .count()
                })
                .max()
                .unwrap_or(0);
            writeln!(self.out, "static __thread int64_t lc_held_regs[{regs}];").unwrap();
            writeln!(
                self.out,
                "static __thread unsigned char lc_held_owns[{regs}];"
            )
            .unwrap();
            writeln!(
                self.out,
                "static __thread int64_t lc_held_loops[{}];",
                2 * loops + 2
            )
            .unwrap();
        }
        // Forward-declare every function; a closure body takes a hidden trailing closure-object pointer arg.
        for (fi, f) in self.prog.functions.iter().enumerate() {
            let mut params: Vec<String> =
                (0..f.param_count).map(|i| self.param_decl(fi, i)).collect();
            if self.closure_bodies.contains(&fi) {
                params.push("int64_t clo".to_string());
            }
            // Carry sole ownership beside the return value. Scalar callers
            // pass NULL; collection callers pass the result register's flag.
            params.push("unsigned char *__ret_own".to_string());
            let sig = params.join(", ");
            writeln!(self.out, "static int64_t fn_{fi}({sig});").unwrap();
            if self.word_twins.contains(&fi) {
                writeln!(self.out, "static lc_wret fn_{fi}_w({sig});").unwrap();
                // The region held exact runs only once a value outgrows a word: cold, so every path into it
                // leaves the word entry's own code.
                writeln!(
                    self.out,
                    "__attribute__((cold, noinline)) static lc_wret fn_{fi}_x(int __resume, int __owned, unsigned char *__ret_own);"
                )
                .unwrap();
            }
        }
        self.out.push('\n');
        self.emit_defining_reader()?;

        // Emit each function body over its (hole-subtracted) span.
        for fi in 0..self.prog.functions.len() {
            let f = &self.prog.functions[fi];
            let entry = f.entry_pc;
            let end = is_inline_closure(self.prog, entry).unwrap_or_else(|| block_end(entry));
            let (param_count, register_count) = (f.param_count, f.register_count);
            let captures = f.captures.clone();
            let holes = holes_in(entry, end);
            self.word_entry = self.word_twins.contains(&fi);
            let body = self.emit_function(
                fi,
                entry,
                end,
                param_count,
                register_count,
                &holes,
                &captures,
            );
            let word_entry = std::mem::take(&mut self.word_entry);
            body.map_err(|e| format!("fn_{fi}: {e}"))?;
            if word_entry {
                self.emit_held_function(fi, register_count);
                self.emit_tagged_entry(fi);
            }
        }

        // Emit Main over [0, first top-level) minus the inline-closure holes. Main is not a closure body — its
        // integer arithmetic stays fast raw i64 (exact only where a constant overflow is proven).
        self.exact_int.set(false);
        writeln!(self.out, "int main(int argc, char **argv) {{").unwrap();
        writeln!(self.out, "    lc_argc = argc; lc_argv = argv;").unwrap();
        writeln!(
            self.out,
            "    int64_t r[{}] = {{0}};",
            self.prog.register_count.max(1)
        )
        .unwrap();
        writeln!(self.out, "    (void)r;").unwrap();
        let main_holes = holes_in(0, main_end);
        self.emit_region(0, main_end, false, None, &main_holes)
            .map_err(|e| format!("Main: {e}"))?;
        writeln!(self.out, "    return 0;\n}}").unwrap();
        Ok(())
    }

    /// The register kinds function `fi`'s body starts with: its parameters' settled kinds (`param_kinds`), a closure
    /// body's captured values (`capture_kinds`) and their present flags in the slots after them, every other register
    /// unknown.
    fn seed_of(&self, fi: usize) -> Vec<Kind> {
        let f = &self.prog.functions[fi];
        let mut seed = vec![Kind::Unknown; f.register_count.max(1)];
        for (i, k) in self.param_kinds[fi].iter().enumerate() {
            if i < seed.len() {
                seed[i] = *k;
            }
        }
        if self.closure_bodies.contains(&fi) {
            let (params, caps) = (f.param_count as usize, f.captures.len());
            for (i, k) in self.capture_kinds.get(fi).into_iter().flatten().enumerate() {
                for (slot, kind) in [(params + i, *k), (params + caps + i, Kind::Bool)] {
                    if slot < seed.len() {
                        seed[slot] = kind;
                    }
                }
            }
        }
        seed
    }

    /// The register kinds a region starts with: a function's parameters' settled kinds, nothing in Main.
    fn seed_of_region(&self, func: Option<usize>) -> Vec<Kind> {
        match func {
            Some(fi) => self.seed_of(fi),
            None => vec![Kind::Unknown; self.prog.register_count.max(1)],
        }
    }

    /// The register kinds a region starts with on machine words: its entry kinds, an exact parameter a machine word
    /// (checked at entry, see [`Emitter::emit_region`]).
    fn words_seed_of_region(&self, func: Option<usize>) -> Vec<Kind> {
        self.seed_of_region(func)
            .into_iter()
            .map(|k| if k == Kind::BigInt { Kind::Int } else { k })
            .collect()
    }

    /// Whether `op` hands its destination a value exact where it comes from: a function's exact result, an exact
    /// global, or an integer map value.
    fn arrives_exact(&self, op: &Op, kinds: &[Kind]) -> bool {
        match op {
            Op::Index { collection, .. } | Op::IndexUnchecked { collection, .. } => self
                .resolve_map_kv(kind_at(kinds, *collection))
                .is_some_and(|(_, value)| value == Kind::BigInt),
            Op::GetField { obj, field, .. } => self
                .resolve_field_any(kind_at(kinds, *obj), *field)
                .is_some_and(|(cell, _)| {
                    self.field_storage_kind(kind_at(kinds, *obj), cell) == Some(Kind::BigInt)
                }),
            Op::Call { func, .. } => self.fn_ret.get(*func as usize) == Some(&Kind::BigInt),
            Op::CallValue { .. } => self
                .closure_bodies
                .iter()
                .any(|b| self.fn_ret.get(*b) == Some(&Kind::BigInt)),
            Op::GlobalGet { idx, .. } => {
                self.global_kinds.get(*idx as usize) == Some(&Kind::BigInt)
            }
            _ => false,
        }
    }

    /// The register kinds a region held exact starts with: its entry kinds, and the parameters an exact variable
    /// starts from, exact.
    fn exact_seed_of_region(&self, func: Option<usize>) -> Vec<Kind> {
        let mut seed = self.seed_of_region(func);
        if let Some(regs) = func.and_then(|fi| self.entry_speculative.get(&fi)) {
            for reg in regs {
                if let Some(k @ (Kind::Int | Kind::Bool)) = seed.get_mut(*reg as usize) {
                    *k = Kind::BigInt;
                }
            }
        }
        seed
    }

    /// Whether the op at `pc` makes its integer exact in the version being inferred or emitted: every exact op held
    /// exact, on machine words only the ones the exact tower computes.
    fn held_exact(&self, pc: usize) -> bool {
        self.exact_defs.contains(&pc)
            && !(self.on_words.get() && self.speculative_defs.contains(&pc))
    }

    /// Every function's parameter kinds as its declaration gives them.
    fn declared_param_kinds(&self) -> Vec<Vec<Kind>> {
        self.prog
            .functions
            .iter()
            .map(|f| {
                f.param_types
                    .iter()
                    .map(|t| self.boundary_kind(t))
                    .collect()
            })
            .collect()
    }

    /// Settle every function's parameter kinds and result kind over the whole program, to a fixpoint. A declared
    /// type seeds each; what the program actually passes and returns refines what the boundary under-types or
    /// leaves out — a tuple handed to a `Pair` parameter (which the boundary records as a `Seq`), an exact
    /// integer handed to an `Int` one, a lambda's undeclared result, what an `Option`'s payload turns out to be.
    fn refine_signatures(
        &mut self,
        regions: &[(Option<usize>, usize, usize, Vec<(usize, usize)>)],
    ) {
        let nf = self.prog.functions.len();
        self.param_kinds = self.declared_param_kinds();
        let declared_params = self.param_kinds.clone();
        // A function no path of which returns a value — a native the program declares and no body implements —
        // hands every caller `nothing`, whatever it declares.
        for (func, start, end, holes) in regions {
            if let Some(fi) = func {
                if !(*start..*end).any(|pc| {
                    !in_hole(pc, holes) && matches!(self.prog.code[pc], Op::Return { .. })
                }) {
                    self.fn_ret[*fi] = Kind::Nothing;
                }
            }
        }
        let declared_ret: Vec<Kind> = self.fn_ret.clone();
        self.capture_kinds = self
            .prog
            .functions
            .iter()
            .map(|f| vec![Kind::Unknown; f.captures.len()])
            .collect();
        for _ in 0..16 {
            let before = (
                self.param_kinds.clone(),
                self.capture_kinds.clone(),
                self.fn_ret.clone(),
                self.net_message.get(),
                self.net_stream.get(),
                self.exact_defs.len(),
                self.speculative_defs.len(),
                self.exact_outs.len(),
                self.entry_speculative.clone(),
            );
            let mut passed: Vec<Vec<Option<Kind>>> = (0..nf)
                .map(|fi| vec![None; self.param_kinds[fi].len()])
                .collect();
            let mut captured: Vec<Vec<Option<Kind>>> = (0..nf)
                .map(|fi| vec![None; self.capture_kinds[fi].len()])
                .collect();
            let (mut sent, mut streamed): (Option<Kind>, Option<Kind>) = (None, None);
            for (func, start, end, holes) in regions {
                self.exact_int
                    .set(func.is_some_and(|fi| self.computes_exactly(fi)));
                // The integers the region can make outgrow i64 — its own, and the parameters they start from — read
                // off its kinds on machine words, where only what the exact tower computes is exact.
                self.on_words.set(true);
                let on_words =
                    self.infer_kinds(*start, *end, self.words_seed_of_region(*func), holes);
                self.on_words.set(false);
                let exact = self.exactness(
                    *start,
                    *end,
                    holes,
                    &on_words,
                    &self.seed_of_region(*func),
                    *func,
                );
                self.speculative_defs
                    .retain(|pc| !(*start..*end).contains(pc));
                self.speculative_defs
                    .extend(exact.speculative.iter().copied());
                self.exact_defs.extend(exact.defs);
                self.exact_outs.extend(exact.outs);
                if let Some(fi) = func {
                    self.entry_speculative.insert(*fi, exact.entry_speculative);
                }
                // The signatures take the region held exact: what it passes and returns.
                let per_pc =
                    self.infer_kinds(*start, *end, self.exact_seed_of_region(*func), holes);
                self.exact_int.set(false);
                for pc in *start..*end {
                    if in_hole(pc, holes) {
                        continue;
                    }
                    match &self.prog.code[pc] {
                        Op::Call {
                            func: callee,
                            args_start,
                            arg_count,
                            ..
                        } => {
                            for i in 0..*arg_count as usize {
                                let k = kind_at(&per_pc[pc - start], args_start + i as u16);
                                if let Some(slot) = passed[*callee as usize].get_mut(i) {
                                    *slot = Some(match *slot {
                                        None => k,
                                        Some(prev) => self.join_passed(prev, k),
                                    });
                                }
                            }
                        }
                        Op::Return { src } => {
                            if let Some(fi) = func {
                                let k = kind_at(&per_pc[pc - start], *src);
                                self.refine_return(*fi, declared_ret[*fi], k);
                            }
                        }
                        // A closure stores a local it captures as the machine word it holds (an exact one narrowed,
                        // as every op that does not read exact integers sees them) and a global as its slot holds it.
                        Op::MakeClosure {
                            func: body,
                            locals_start,
                            ..
                        } => {
                            let mut local = *locals_start;
                            for (i, (_, global)) in self.prog.functions[*body as usize].captures.iter().enumerate() {
                                let k = match global {
                                    Some(g) => self.global_kinds.get(*g as usize).copied().unwrap_or(Kind::Unknown),
                                    None => {
                                        let k = kind_at(&per_pc[pc - start], local);
                                        local += 1;
                                        if k == Kind::BigInt { Kind::Int } else { k }
                                    }
                                };
                                if let Some(slot) = captured[*body as usize].get_mut(i) {
                                    *slot = Some(match *slot {
                                        None => k,
                                        Some(prev) => self.join_passed(prev, k),
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                    // A return hands each `mutable` parameter back to the caller: what it holds there is passed to
                    // the parameter as much as what a caller lends it.
                    if let (Some(fi), Op::Return { .. } | Op::ReturnNothing) =
                        (func, &self.prog.code[pc])
                    {
                        for p in self.inout_regs(*fi) {
                            let k = kind_at(&per_pc[pc - start], p);
                            if let Some(slot) = passed[*fi].get_mut(p as usize) {
                                *slot = Some(match *slot {
                                    None => k,
                                    Some(prev) => self.join_passed(prev, k),
                                });
                            }
                        }
                    }
                    match &self.prog.code[pc] {
                        Op::NetSend { msg, .. } => {
                            let k = kind_at(&per_pc[pc - start], *msg);
                            sent = Some(sent.map_or(k, |s| self.join_passed(s, k)));
                        }
                        Op::NetStream { values, .. } => {
                            let k = kind_at(&per_pc[pc - start], *values);
                            streamed = Some(streamed.map_or(k, |s| self.join_passed(s, k)));
                        }
                        _ => {}
                    }
                }
            }
            for fi in 0..nf {
                for i in 0..passed[fi].len() {
                    if let Some(got) = passed[fi][i] {
                        self.param_kinds[fi][i] =
                            self.refine_param(declared_params[fi][i], self.param_kinds[fi][i], got);
                    }
                }
                for i in 0..captured[fi].len() {
                    if let Some(got) = captured[fi][i] {
                        self.capture_kinds[fi][i] = self.refine_param(Kind::Unknown, self.capture_kinds[fi][i], got);
                    }
                }
            }
            self.net_message.set(sent.unwrap_or(Kind::Unknown));
            self.net_stream.set(streamed.unwrap_or(Kind::Unknown));
            if (
                self.param_kinds.clone(),
                self.capture_kinds.clone(),
                self.fn_ret.clone(),
                self.net_message.get(),
                self.net_stream.get(),
                self.exact_defs.len(),
                self.speculative_defs.len(),
                self.exact_outs.len(),
                self.entry_speculative.clone(),
            ) == before
            {
                break;
            }
        }
    }

    /// Two kinds a parameter is handed at different call sites, as one: equal kinds, an exact integer over an
    /// `Int`, two layouts of one undeclared enum; anything else is not one kind.
    fn join_passed(&self, a: Kind, b: Kind) -> Kind {
        match (a, b) {
            (a, b) if a == b => a,
            (Kind::Tower, _) | (_, Kind::Tower) => Kind::Tower,
            (Kind::Int, Kind::BigInt) | (Kind::BigInt, Kind::Int) => Kind::BigInt,
            (Kind::Inductive(x), Kind::Inductive(y)) => {
                self.unify_inductive(x, y).unwrap_or(Kind::Unknown)
            }
            _ => Kind::Unknown,
        }
    }

    /// A parameter's kind, given its declared kind, its current kind and the kind every call site hands it: an
    /// undeclared parameter takes what it is handed; an `Int` handed an exact integer is exact; a `Seq`
    /// handed a tuple of that element everywhere is that tuple; an `Option`/`Result` learns its payloads. What
    /// is learned is never given back.
    fn refine_param(&self, declared: Kind, current: Kind, passed: Kind) -> Kind {
        let refined = match (declared, passed) {
            // A parameter handed a value of the extension holds its cell, whatever it declares.
            (_, Kind::Tower) => Kind::Tower,
            (Kind::Unknown, p) if p != Kind::Unknown => p,
            (Kind::Int, Kind::BigInt) => Kind::BigInt,
            (d, Kind::Tuple(ti)) if d.seq_c_type().is_some() || d == Kind::SeqUnknown => {
                let elem = self.elem_kind(d);
                if self
                    .tuple_positions(ti)
                    .iter()
                    .all(|k| *k == elem || elem == Kind::Unknown)
                {
                    passed
                } else {
                    declared
                }
            }
            (Kind::Inductive(d), Kind::Inductive(p)) => {
                self.unify_inductive(d, p).unwrap_or(declared)
            }
            _ => declared,
        };
        match (current, refined) {
            (Kind::BigInt, Kind::Int) => Kind::BigInt,
            (c, r) if r == declared && c != declared => c,
            _ => refined,
        }
    }

    /// Fold one `Return` of function `fi`, returning a value of kind `k`, into the function's result kind: a
    /// boxed kind the boundary under-types (a `Tuple` declared as a `Seq`, an exact integer or rational declared
    /// `Int`), an undeclared result, an `Option`/`Result` learning its payloads.
    fn refine_return(&mut self, fi: usize, declared: Kind, k: Kind) {
        let current = self.fn_ret[fi];
        self.fn_ret[fi] = match (current, k) {
            (_, Kind::Unknown) => current,
            // A function that hands back a value of the extension answers its cell, whatever it declares.
            (Kind::Tower, _) | (_, Kind::Tower) => Kind::Tower,
            (Kind::Inductive(c), Kind::Inductive(r)) => {
                self.unify_inductive(c, r).unwrap_or(current)
            }
            (Kind::Int, Kind::BigInt) | (Kind::BigInt, Kind::Int) => Kind::BigInt,
            (c, r) if c == r => c,
            (c, r @ (Kind::Tuple(_) | Kind::BigInt | Kind::Rational))
                if c == declared =>
            {
                r
            }
            (Kind::Unknown, r) if declared == Kind::Unknown => r,
            (c, r) if declared == Kind::Unknown => join(c, r),
            (c, _) => c,
        };
    }

    /// Infer each global's kind from its `GlobalSet` sites, scanning Main + every function region (a couple of
    /// passes so a global assigned from another global settles).
    fn derive_global_kinds(&mut self, main_end: usize) {
        let n = self.prog.globals.len();
        if n == 0 {
            return;
        }
        // A global the program declares a type for is that type — an `Int` one held exact once an exact integer is
        // stored in it, as a parameter is; any other takes the kind of what is stored in it — never the `nothing` a
        // global is emptied to while its collection is changed in a register.
        self.global_kinds = (0..n)
            .map(|i| self.boundary_kind(self.prog.global_types.get(i).unwrap_or(&None)))
            .collect();
        let declared: Vec<Kind> = self.global_kinds.clone();
        // A global any region stores a value of the exact extension into holds one, whatever else it holds.
        let takes = |idx: usize, k: Kind| {
            k == Kind::Tower
                || match declared.get(idx).copied().unwrap_or(Kind::Unknown) {
                    Kind::Unknown => true,
                    Kind::Int => matches!(k, Kind::Int | Kind::BigInt),
                    _ => false,
                }
        };
        let regions = self.regions_with_seeds(main_end);
        for _ in 0..2 {
            let mut updates: Vec<(usize, Kind)> = Vec::new();
            for (start, end, seed) in &regions {
                let per_pc = self.infer_kinds(*start, *end, seed.clone(), &[]);
                for pc in *start..*end {
                    if let Op::GlobalSet { idx, src } = &self.prog.code[pc] {
                        let k = per_pc[pc - start]
                            .get(*src as usize)
                            .copied()
                            .unwrap_or(Kind::Unknown);
                        if !matches!(k, Kind::Unknown | Kind::Nothing) && takes(*idx as usize, k) {
                            updates.push((*idx as usize, k));
                        }
                    }
                }
            }
            for (idx, k) in updates {
                if let Some(slot) = self.global_kinds.get_mut(idx) {
                    *slot = match (*slot, k) {
                        (Kind::Tower, _) | (_, Kind::Tower) => Kind::Tower,
                        (Kind::BigInt, Kind::Int) | (Kind::Int, Kind::BigInt) => Kind::BigInt,
                        _ => k,
                    };
                }
            }
        }
    }

    /// The `(start, end, seed)` of Main and every function region — the same partition `derive_global_kinds` /
    /// `derive_counter_fields` scan.
    fn regions_with_seeds(&self, main_end: usize) -> Vec<(usize, usize, Vec<Kind>)> {
        let mut regions: Vec<(usize, usize, Vec<Kind>)> = vec![(
            0,
            main_end,
            vec![Kind::Unknown; self.prog.register_count.max(1)],
        )];
        let mut starts: Vec<usize> = self.prog.functions.iter().map(|f| f.entry_pc).collect();
        starts.sort_unstable();
        for (fi, f) in self.prog.functions.iter().enumerate() {
            let end = starts
                .iter()
                .copied()
                .find(|&s| s > f.entry_pc)
                .unwrap_or(self.prog.code.len());
            regions.push((f.entry_pc, end, self.seed_of(fi)));
        }
        regions
    }

    /// Discover which struct fields hold a LIVE counter CRDT: a `StructInsert` whose stored value is a
    /// `Kind::CrdtCounter` (a `NewCrdt` kind-4/5 result) marks `(struct name, cell)` as a counter field — so
    /// `GetField` SUMS the per-replica map and `CrdtMerge`/`CrdtJoin` take the per-replica MAX. A field that
    /// is NEVER assigned a `NewCrdt` value (e.g. `sync_only`'s `Tally.n`, only ever `CrdtBump`ed from a
    /// `Nothing` cell) is NOT a counter here — matching the VM's `crdt_counter_bump(Nothing, n) = Int(n)`,
    /// where the cell stays a plain integer. Mirrors `codegen_riscv::derive_counter_fields`, keyed by name so
    /// the synthesized-layout structs and the `prog.struct_types` structs share one key space.
    fn derive_counter_fields(&mut self, main_end: usize) {
        let regions = self.regions_with_seeds(main_end);
        let mut found: std::collections::HashSet<(String, usize)> =
            std::collections::HashSet::new();
        for (start, end, seed) in &regions {
            let per_pc = self.infer_kinds(*start, *end, seed.clone(), &[]);
            for pc in *start..*end {
                if let Op::StructInsert { obj, field, value } = &self.prog.code[pc] {
                    let cur = &per_pc[pc - start];
                    let vk = cur.get(*value as usize).copied().unwrap_or(Kind::Unknown);
                    if vk == Kind::CrdtCounter {
                        let ok = cur.get(*obj as usize).copied().unwrap_or(Kind::Unknown);
                        if let (Some(name), Some((cell, _))) = (
                            self.struct_name_of_kind(ok),
                            self.resolve_field_any(ok, *field),
                        ) {
                            found.insert((name, cell));
                        }
                    }
                }
            }
        }
        self.counter_fields = found;
    }

    fn emit_function(
        &mut self,
        fi: usize,
        entry: usize,
        end: usize,
        param_count: u16,
        register_count: usize,
        holes: &[(usize, usize)],
        captures: &[(crate::intern::Symbol, Option<u16>)],
    ) -> Result<(), String> {
        let is_closure = self.closure_bodies.contains(&fi);
        if is_closure && !self.inout_regs(fi).is_empty() {
            return Err("a function value's body with a `mutable` parameter".into());
        }
        // Inside a function value's body, integer arithmetic is exact (`lint_*`) — see `exact_int`.
        self.exact_int.set(self.computes_exactly(fi));
        let mut params: Vec<String> = (0..param_count).map(|i| self.param_decl(fi, i)).collect();
        if is_closure {
            params.push("int64_t clo".to_string());
        }
        params.push("unsigned char *__ret_own".to_string());
        let sig = params.join(", ");
        if self.word_entry {
            writeln!(self.out, "static lc_wret fn_{fi}_w({sig}) {{").unwrap();
        } else {
            writeln!(self.out, "static int64_t fn_{fi}({sig}) {{").unwrap();
        }
        writeln!(
            self.out,
            "    int64_t r[{}] = {{0}};",
            register_count.max(1)
        )
        .unwrap();
        writeln!(self.out, "    (void)r;").unwrap();
        writeln!(self.out, "    if (__ret_own) *__ret_own = 0;").unwrap();
        let inout = self.inout_regs(fi);
        for i in 0..param_count {
            let deref = if inout.contains(&i) { "*" } else { "" };
            writeln!(self.out, "    r[{i}] = {deref}p{i};").unwrap();
        }
        // A closure body loads its captured values into slots `params..params+cap` and the present flags into
        // `params+cap..params+2*cap` from the closure object — the layout the VM's `CallValue` binding sets up.
        if is_closure && !captures.is_empty() {
            let cap = captures.len();
            writeln!(self.out, "    int64_t *_clo = (int64_t*)(intptr_t)clo;").unwrap();
            for k in 0..cap {
                writeln!(
                    self.out,
                    "    r[{}] = _clo[{}];",
                    param_count as usize + k,
                    1 + k
                )
                .unwrap();
                writeln!(
                    self.out,
                    "    r[{}] = _clo[{}];",
                    param_count as usize + cap + k,
                    1 + cap + k
                )
                .unwrap();
            }
        } else if is_closure {
            writeln!(self.out, "    (void)clo;").unwrap();
        }
        self.dispatch_to_device(fi);
        self.current_fn = Some(fi);
        self.emit_region(entry, end, true, Some(fi), holes)?;
        self.current_fn = None;
        let fall = self.ret("0", false);
        writeln!(self.out, "    {fall}\n}}\n").unwrap();
        Ok(())
    }

    /// The region held exact of word entry `fi`, as a function of its own: its registers, owned flags and loops arrive
    /// through the spill buffers, `__resume` names the way in, and each way in tags what the region takes tagged and
    /// hands its loops over before it jumps to the op it resumes at. A word entry whose region never holds an integer
    /// exact has nothing to resume.
    fn emit_held_function(&mut self, fi: usize, register_count: usize) {
        let n = register_count.max(1);
        let body = std::mem::take(&mut self.held_body);
        let entries = std::mem::take(&mut self.held_entries);
        let owns = std::mem::take(&mut self.held_owns);
        writeln!(
            self.out,
            "static lc_wret fn_{fi}_x(int __resume, int __owned, unsigned char *__ret_own) {{"
        )
        .unwrap();
        if entries.is_empty() {
            writeln!(
                self.out,
                "    (void)__resume; (void)__owned; (void)__ret_own;"
            )
            .unwrap();
            writeln!(self.out, "    return (lc_wret){{ 0, 0 }};\n}}\n").unwrap();
            return;
        }
        writeln!(self.out, "    int64_t r[{n}];").unwrap();
        writeln!(self.out, "    memcpy(r, lc_held_regs, sizeof r);").unwrap();
        if owns {
            writeln!(self.out, "    unsigned char own[{n}]; (void)own;").unwrap();
            writeln!(
                self.out,
                "    if (__owned) memcpy(own, lc_held_owns, sizeof own); else memset(own, 0, sizeof own);"
            )
            .unwrap();
        } else {
            writeln!(self.out, "    (void)__owned;").unwrap();
        }
        writeln!(self.out, "    switch (__resume) {{").unwrap();
        for (id, ..) in &entries {
            writeln!(self.out, "    case {id}: goto R{id};").unwrap();
        }
        writeln!(self.out, "    default: abort();\n    }}").unwrap();
        self.out.push_str(&body);
        for (id, loops, resume, tags) in &entries {
            writeln!(self.out, "R{id}:;").unwrap();
            for reg in tags {
                writeln!(self.out, "    r[{reg}] = lc_xof(r[{reg}]);").unwrap();
            }
            for (k, h) in loops.iter().enumerate() {
                writeln!(
                    self.out,
                    "    _itv{h} = (__typeof__(_itv{h}))(intptr_t)lc_held_loops[{}]; _it{h} = (size_t)lc_held_loops[{}];",
                    2 * k,
                    2 * k + 1
                )
                .unwrap();
            }
            writeln!(self.out, "    goto X{resume};").unwrap();
        }
        writeln!(self.out, "}}\n").unwrap();
    }

    /// The tagged entry of a function whose body is its word entry: `fn_N` takes its exact parameters as tagged words,
    /// as every caller off machine words passes them. Small ones enter the word entry untagged; a boxed one enters the
    /// region held exact (`fn_N_x`) at its start, handed the parameters live there. A machine-word result is tagged on
    /// the way out.
    fn emit_tagged_entry(&mut self, fi: usize) {
        let params = self.param_kinds[fi].clone();
        let count = self.prog.functions[fi].param_count as usize;
        let mut decls: Vec<String> = (0..count as u16).map(|i| self.param_decl(fi, i)).collect();
        decls.push("unsigned char *__ret_own".to_string());
        let exact: Vec<usize> = (0..count)
            .filter(|i| params.get(*i) == Some(&Kind::BigInt))
            .collect();
        let mut words: Vec<String> = (0..count)
            .map(|i| {
                if exact.contains(&i) {
                    format!("p{i} >> 1")
                } else {
                    format!("p{i}")
                }
            })
            .collect();
        words.push("__ret_own".to_string());
        let start = self.held_start.take();
        writeln!(self.out, "static int64_t fn_{fi}({}) {{", decls.join(", ")).unwrap();
        writeln!(self.out, "    lc_wret _w;").unwrap();
        match (&start, exact.is_empty()) {
            (Some((id, live)), false) => {
                let tags: Vec<String> = exact.iter().map(|i| format!("p{i}")).collect();
                writeln!(self.out, "    if (({}) & 1) {{", tags.join(" | ")).unwrap();
                for reg in live {
                    let value = if (*reg as usize) < count {
                        format!("p{reg}")
                    } else {
                        "0".to_string()
                    };
                    writeln!(self.out, "        lc_held_regs[{reg}] = {value};").unwrap();
                }
                writeln!(self.out, "        _w = fn_{fi}_x({id}, 0, __ret_own);").unwrap();
                writeln!(self.out, "    }} else {{").unwrap();
                writeln!(self.out, "        _w = fn_{fi}_w({});", words.join(", ")).unwrap();
                writeln!(self.out, "    }}").unwrap();
            }
            _ => {
                writeln!(self.out, "    _w = fn_{fi}_w({});", words.join(", ")).unwrap();
            }
        }
        if self.fn_ret[fi] == Kind::BigInt {
            writeln!(self.out, "    return _w.escaped ? _w.v : lc_xof(_w.v);").unwrap();
        } else {
            writeln!(self.out, "    return _w.v;").unwrap();
        }
        writeln!(self.out, "}}\n").unwrap();
    }

    /// The registers `op` writes: its own, and for a call each argument register a `mutable` parameter leaves its
    /// final value in, which the VM keeps on return for the caller to copy back.
    fn for_each_written(&self, op: &Op, mut f: impl FnMut(u16)) {
        op.for_each_written_reg(&mut f);
        if let Op::Call {
            func, args_start, ..
        } = op
        {
            for p in self.inout_regs(*func as usize) {
                f(args_start + p);
            }
        }
    }

    /// The parameters of function `fi` that are the caller's own variables (`mutable`, declared or inferred): the
    /// argument registers the VM keeps when the call returns, for the caller to copy back.
    fn inout_regs(&self, fi: usize) -> Vec<u16> {
        let f = &self.prog.functions[fi];
        f.mutable_param_regs
            .iter()
            .copied()
            .filter(|p| *p < f.param_count)
            .collect()
    }

    /// The transient argument-window register whose collection a direct call
    /// consumes and returns, when that identity proof is unambiguous.  The VM
    /// compiler records exactly these inferred mutable borrows in
    /// `cow_exempt_regs`: an `EnsureOwned` at the call site isolates the value,
    /// the callee mutates that same allocation, and its result gives it back.
    fn consumed_borrow_arg(&self, func: u16, args_start: u16, arg_count: u16) -> Option<u16> {
        let f = self.prog.functions.get(func as usize)?;
        let mut params = f
            .cow_exempt_regs
            .iter()
            .copied()
            .filter(|p| *p < arg_count && *p < f.param_count);
        let param = params.next()?;
        // Multiple consumed collection parameters do not identify which one a
        // single return value carries.  Keep the conservative COW path.
        if params.next().is_some() {
            return None;
        }
        Some(args_start + param)
    }

    fn consumed_borrow_transfer(&self, op: &Op) -> Option<(u16, u16)> {
        let Op::Call {
            dst,
            func,
            args_start,
            arg_count,
        } = op
        else {
            return None;
        };
        self.consumed_borrow_arg(*func, *args_start, *arg_count)
            .map(|arg| (*dst, arg))
    }

    /// Whether `dst` is an untracked transient argument-window slot for the
    /// next direct call, at a parameter the callee cannot mutate in place.
    /// The VM compiler materializes arguments as `Move`s immediately before
    /// the call.  Moving a sole-owned collection into an untracked window used
    /// to clear the source's owner bit permanently, even though ordinary value
    /// parameters use COW and therefore only borrow the caller's allocation.
    fn feeds_readonly_call_arg(&self, pc: usize, dst: u16) -> bool {
        let (_, end) = self.region_span;
        for at in pc + 1..end.min(pc + 16) {
            if self.region_targets.contains(&at) {
                return false;
            }
            match &self.prog.code[at] {
                Op::Move { .. } => continue,
                Op::Call {
                    func,
                    args_start,
                    arg_count,
                    ..
                } => {
                    if dst < *args_start || dst >= args_start.saturating_add(*arg_count) {
                        return false;
                    }
                    let param = dst - *args_start;
                    let f = &self.prog.functions[*func as usize];
                    return !self.inout_regs(*func as usize).contains(&param)
                        && !f.cow_exempt_regs.contains(&param);
                }
                _ => return false,
            }
        }
        false
    }

    /// The C declaration of parameter `i` of function `fi`: a value, or for an in-out parameter a pointer to the
    /// caller's argument register, which the callee loads on entry and stores back before it returns.
    fn param_decl(&self, fi: usize, i: u16) -> String {
        if self.inout_regs(fi).contains(&i) {
            format!("int64_t *p{i}")
        } else {
            format!("int64_t p{i}")
        }
    }

    /// Before a return from the current function: each in-out parameter's final value stored into the caller's
    /// argument register, in the kind the caller settles it to after the call (the parameter's own): a machine
    /// integer an exact parameter holds is stored exact. An exact value a machine-integer parameter cannot carry is
    /// refused.
    fn store_inout_params(&mut self, kinds: &[Kind]) -> Result<(), String> {
        let Some(fi) = self.current_fn else {
            return Ok(());
        };
        for p in self.inout_regs(fi) {
            let settled = self.param_kinds[fi]
                .get(p as usize)
                .copied()
                .unwrap_or(Kind::Unknown);
            match (settled, kind_at(kinds, p)) {
                (Kind::BigInt, Kind::Int | Kind::Bool) => self.line(&format!("*p{p} = lc_xof(r[{p}]);")),
                (Kind::Int | Kind::Bool, Kind::BigInt) => {
                    return Err(format!("`mutable` parameter {p} is settled as {settled:?} and returns an exact integer"))
                }
                _ => self.line(&format!("*p{p} = r[{p}];")),
            }
        }
        Ok(())
    }

    /// The device dispatch at the entry of function `fi`, when the offload table names it: its parameters marshaled
    /// into their ports by their settled kinds, the device asked, and its DONE answer re-boxed to the settled result
    /// kind and returned. An exact integer past i64 cannot cross an Int port, so that call runs on the host. A port
    /// a settled kind does not fit keeps the whole function on the host, reported. A function with an in-out
    /// parameter stays on the host: a port carries a value in, not the caller's variable.
    fn dispatch_to_device(&mut self, fi: usize) {
        let Some(offload) = self.offloads.iter().find(|o| o.func == fi).cloned() else {
            return;
        };
        if !self.inout_regs(fi).is_empty() {
            self.kept.push((
                fi,
                "a `mutable` parameter is the caller's variable, which a port does not carry"
                    .into(),
            ));
            return;
        }
        let arity = self.prog.functions[fi].param_count as usize;
        if offload.params.len() != arity {
            self.kept.push((
                fi,
                format!(
                    "the region takes {} ports and the function {arity} parameters",
                    offload.params.len()
                ),
            ));
            return;
        }
        let mut marshal = Vec::new();
        for (i, port) in offload.params.iter().enumerate() {
            let settled = self.param_kinds[fi]
                .get(i)
                .copied()
                .unwrap_or(Kind::Unknown);
            let cross = match (port, settled) {
                (SlotKind::Int, Kind::Int)
                | (SlotKind::Float, Kind::Float)
                | (SlotKind::Word64, Kind::Word(64)) => {
                    format!("_dv[{i}] = r[{i}];")
                }
                (SlotKind::Int, Kind::BigInt) => {
                    format!("if (!lc_xi64(r[{i}], &_dv[{i}])) _dm = 0;")
                }
                (SlotKind::Bool, Kind::Bool) => format!("_dv[{i}] = r[{i}] != 0;"),
                (SlotKind::Word32, Kind::Word(32)) => {
                    format!("_dv[{i}] = (int64_t)((uint64_t)r[{i}] & 0xFFFFFFFFu);")
                }
                _ => {
                    self.kept.push((fi, format!("parameter {i} is settled as {settled:?}, which the {port:?} port does not carry")));
                    return;
                }
            };
            marshal.push(cross);
        }
        let settled = self.fn_ret[fi];
        let answer = match (offload.ret, settled) {
            (SlotKind::Int, Kind::Int)
            | (SlotKind::Float, Kind::Float)
            | (SlotKind::Word64, Kind::Word(64)) => "_do".to_string(),
            (SlotKind::Int, Kind::BigInt) => "lc_xof(_do)".to_string(),
            (SlotKind::Bool, Kind::Bool) => "(_do != 0)".to_string(),
            (SlotKind::Word32, Kind::Word(32)) => {
                "(int64_t)((uint64_t)_do & 0xFFFFFFFFu)".to_string()
            }
            (SlotKind::CharText, Kind::Text) => "(int64_t)(intptr_t)lc_chr(_do)".to_string(),
            (port, _) => {
                self.kept.push((fi, format!("the result is settled as {settled:?}, which the {port:?} port does not carry")));
                return;
            }
        };
        self.line(&format!(
            "{{ int64_t _dv[{}], _do; int _dm = 1;",
            arity.max(1)
        ));
        for m in marshal {
            self.line(&format!("  {m}"));
        }
        self.line(&format!(
            "  if (_dm && logos_dev_call({}, _dv, &_do)) return {answer}; }}",
            offload.region
        ));
    }

    /// Emit the ops of one region `[start, end)`, the body of function `func` or Main. `in_function` distinguishes a
    /// `Return` (yield a value) from a Main-level `Return`/`Halt`. Jump targets use absolute-pc labels.
    ///
    /// A region with exact integers runs twice over: on machine words (labels `L…`), and held exact (labels `X…`),
    /// entered only when a value does not fit a word. On machine words each operation that can overflow an exact
    /// integer checks for it (stub `D…`), each exact value that arrives — a function's result, a global — checks that
    /// it is a small one and takes its word (stub `A…`), and so does each exact parameter on entry (stub `N…`). A stub
    /// tags every value the region held exact holds as a machine word, hands each running loop its place, and resumes
    /// the region held exact at the op (which redoes it exactly) or after the arrival. So exactness costs a not-taken
    /// branch per such op until a value outgrows the word.
    fn emit_region(
        &mut self,
        start: usize,
        end: usize,
        in_function: bool,
        func: Option<usize>,
        holes: &[(usize, usize)],
    ) -> Result<(), String> {
        let mut is_target = std::collections::HashSet::new();
        for op in &self.prog.code[start..end] {
            if let Some(t) = jump_target(op) {
                is_target.insert(t);
            }
        }
        let entry_seed = self.seed_of_region(func);
        let register_count = entry_seed.len();
        self.on_words.set(true);
        let on_words = self.infer_kinds(start, end, self.words_seed_of_region(func), holes);
        self.on_words.set(false);
        let held = self.infer_kinds(start, end, self.exact_seed_of_region(func), holes);
        let reachable = self.reachable(start, end, holes, &on_words);
        let integer = |k: Kind| matches!(k, Kind::Int | Kind::Bool);
        let dst_of = |op: &Op| {
            let mut dst = None;
            op.for_each_written_reg(|w| dst = Some(w));
            dst
        };
        let words_to_exact = |pc: usize, d: u16| {
            integer(kind_at(&on_words[pc - start + 1], d))
                && kind_at(&held[pc - start + 1], d) == Kind::BigInt
        };
        let live_ops: Vec<usize> = (start..end)
            .filter(|pc| !in_hole(*pc, holes) && reachable.contains(pc))
            .collect();
        let deopt: std::collections::BTreeSet<usize> = live_ops
            .iter()
            .copied()
            .filter(|pc| self.speculative_defs.contains(pc) && overflows(&self.prog.code[*pc]))
            .filter(|pc| dst_of(&self.prog.code[*pc]).is_some_and(|d| words_to_exact(*pc, d)))
            .collect();
        let arrivals: std::collections::BTreeSet<usize> = live_ops
            .iter()
            .copied()
            .filter(|pc| self.arrives_exact(&self.prog.code[*pc], &on_words[*pc - start]))
            .filter(|pc| dst_of(&self.prog.code[*pc]).is_some_and(|d| words_to_exact(*pc, d)))
            .collect();
        let exact_params: Vec<u16> = (0..register_count as u16)
            .filter(|r| entry_seed[*r as usize] == Kind::BigInt)
            .collect();
        let owned_of = |per_pc: &[Vec<Kind>]| -> std::collections::BTreeSet<u16> {
            let mut owned = std::collections::BTreeSet::new();
            for pc in start..end {
                if in_hole(pc, holes) {
                    continue;
                }
                match (mutation(&self.prog.code[pc]), &self.prog.code[pc]) {
                    (Some(Mutation::Owned(target)), _) => {
                        owned.insert(target);
                    }
                    // A text a register grows by appending is the register's own, like a collection it changes.
                    (_, Op::AddAssign { dst, .. })
                        if kind_at(&per_pc[pc - start], *dst) == Kind::Text =>
                    {
                        owned.insert(*dst);
                    }
                    _ => {}
                }
                if let Some((dst, arg)) = self.consumed_borrow_transfer(&self.prog.code[pc]) {
                    owned.insert(dst);
                    owned.insert(arg);
                }
            }
            owned
        };
        let speculates = !deopt.is_empty() || !arrivals.is_empty() || !exact_params.is_empty();
        let (owned_words, owned_held) = (owned_of(&on_words), owned_of(&held));
        // An owned flag rides every handle the region writes to a register, in either version (a deopt keeps the
        // flags): the register's handles are of one kind only when every write of one, in both, is of that kind.
        let handle_kinds = {
            let mut seen: std::collections::BTreeMap<u16, Option<Kind>> =
                std::collections::BTreeMap::new();
            for per_pc in [&on_words, &held] {
                for pc in (start..end).filter(|pc| !in_hole(*pc, holes)) {
                    self.for_each_written(&self.prog.code[pc], |w| {
                        let k = kind_at(&per_pc[pc - start + 1], w);
                        if !carries_no_handle(k) {
                            let one = seen.entry(w).or_insert(Some(k));
                            if *one != Some(k) {
                                *one = None;
                            }
                        }
                    });
                }
            }
            seen.into_iter()
                .filter_map(|(r, k)| Some((r, k?)))
                .collect()
        };
        self.handle_kinds = handle_kinds;
        // A parameter of an exact web arrives as what its caller holds and becomes a cell as the region begins.
        let arriving = self.tower_entries.borrow().get(&start).cloned().unwrap_or_default();
        for (reg, k) in arriving {
            let cell = self.tower_of(&format!("r[{reg}]"), k)?;
            self.line(&format!("r[{reg}] = (int64_t)(intptr_t)({cell});"));
        }
        // A parameter whose variable a number join widens is converted as the region begins; a variable no machine
        // form holds refuses the region by name.
        let (widened, refused) = self.number_entries.borrow().get(&start).cloned().unwrap_or_default();
        if let Some(site) = refused.first() {
            let at = match site {
                crate::vm::variables::Site::Op(pc) => format!("pc {pc}"),
                crate::vm::variables::Site::Entry(reg) => format!("parameter register {reg}"),
            };
            return Err(format!(
                "{at}: a variable that holds an exact number and a real where control merges is not yet lowered"
            ));
        }
        for (reg, from, to) in widened {
            let v = self.convert_number(&format!("r[{reg}]"), from, to)?;
            self.line(&format!("r[{reg}] = {v};"));
        }
        self.region_span = (start, end);
        self.live_out = self.liveness(start, end, holes);
        // A word entry's region held exact runs in a function of its own, which declares its own flags.
        let hot_owns =
            !owned_words.is_empty() || (speculates && !self.word_entry && !owned_held.is_empty());
        if hot_owns {
            writeln!(
                self.out,
                "    unsigned char own[{}] = {{0}}; (void)own;",
                register_count.max(1)
            )
            .unwrap();
            // A `mutable` parameter is the caller's own value, isolated at the call: the function holds it alone.
            for p in self.current_fn.map(|fi| self.inout_regs(fi)).unwrap_or_default() {
                if owned_words.contains(&p) || owned_held.contains(&p) {
                    writeln!(self.out, "    own[{p}] = 1;").unwrap();
                }
            }
        }

        // On machine words: an exact parameter takes its word, or the whole call runs held exact. A word entry's
        // parameters arrive as machine words; its tagged entry takes a boxed one into the region held exact itself.
        if !exact_params.is_empty() && !self.word_entry {
            let tags: Vec<String> = exact_params.iter().map(|r| format!("r[{r}]")).collect();
            self.line(&format!("if (({}) & 1) goto N{start};", tags.join(" | ")));
            for r in &exact_params {
                self.line(&format!("r[{r}] >>= 1;"));
            }
        }
        self.on_words.set(true);
        self.owned = owned_words;
        self.region_targets = is_target.clone();
        let (text_byte_calls, text_byte_iters) =
            self.text_byte_iteration_plan(start, end, holes, &reachable);
        self.text_byte_calls = text_byte_calls;
        self.text_byte_iters = text_byte_iters;
        if !self.text_byte_calls.is_empty() {
            crate::optimize::mark_fired(crate::optimization::Opt::TextBytesView);
        }
        self.deopt = deopt.clone();
        self.arrivals = arrivals.clone();
        let words_frames = self.emit_version(start, end, in_function, holes, &on_words, &reachable);
        self.on_words.set(false);
        self.deopt.clear();
        self.arrivals.clear();
        let words_frames = words_frames?;
        if !speculates {
            return Ok(());
        }
        self.line(&self.ret("0", false));

        // Held exact: it opens with its own label, and every op a stub resumes it at has one.
        let next_of = |pc: usize| live_ops.iter().copied().find(|p| *p > pc);
        let first = live_ops.first().copied();
        let resumed: Vec<usize> = arrivals.iter().filter_map(|pc| next_of(*pc)).collect();
        let held_owns = hot_owns || !owned_held.is_empty();
        self.owned = owned_held;
        self.region_targets = is_target
            .iter()
            .chain(&deopt)
            .chain(&first)
            .chain(&resumed)
            .copied()
            .collect();
        let hot = self.word_entry.then(|| std::mem::take(&mut self.out));
        self.label = 'X';
        let held_frames = self.emit_version(start, end, in_function, holes, &held, &reachable);
        self.label = 'L';
        self.line(&self.ret("0", false));
        if let Some(hot) = hot {
            self.held_body = std::mem::replace(&mut self.out, hot);
            self.held_owns = held_owns;
        }
        let held_frames = held_frames?;
        // A word entry hands its registers, owned flags and loops to `fn_N_x` through the spill buffers, one element
        // at a time so its own registers never leave the machine's, and returns what that function returns.
        let split = func.filter(|_| self.word_entry);

        let first = first.ok_or("a region with exact parameters and no op")?;
        if !exact_params.is_empty() {
            if split.is_none() {
                writeln!(self.out, "N{start}:;").unwrap();
            }
            let starts_exact: Vec<u16> = func
                .and_then(|fi| self.entry_speculative.get(&fi))
                .into_iter()
                .flatten()
                .copied()
                .collect();
            for r in starts_exact.iter().filter(|_| split.is_none()) {
                if !exact_params.contains(r) && integer(entry_seed[*r as usize]) {
                    self.line(&format!("r[{r}] = lc_xof(r[{r}]);"));
                }
            }
            if split.is_some() {
                let id = self.held_entries.len();
                let live: Vec<u16> = self.live_in(first).into_iter().collect();
                let tags: Vec<u16> = live
                    .iter()
                    .copied()
                    .filter(|r| {
                        !exact_params.contains(r)
                            && starts_exact.contains(r)
                            && integer(entry_seed[*r as usize])
                    })
                    .collect();
                self.held_entries.push((id, Vec::new(), first, tags));
                self.held_start = Some((id, live));
            } else {
                self.line(&format!("goto X{first};"));
            }
        }
        let stubs = deopt
            .iter()
            .map(|pc| ('D', *pc, *pc, None))
            .chain(arrivals.iter().map(|pc| {
                (
                    'A',
                    *pc,
                    next_of(*pc).unwrap_or(*pc),
                    dst_of(&self.prog.code[*pc]),
                )
            }));
        for (kind, pc, resume, arrived) in stubs.collect::<Vec<_>>() {
            writeln!(self.out, "{kind}{pc}:;").unwrap();
            // The kinds where the region held exact resumes: before the op it redoes, after the value that arrived.
            let at = if kind == 'D' {
                pc - start
            } else {
                pc - start + 1
            };
            // A word entry's stub touches only the registers live where the region resumes, so a register its machine
            // words leave dead stays dead on their path.
            let live = split.map(|_| self.live_in(resume));
            for reg in 0..register_count as u16 {
                if Some(reg) != arrived
                    && live.as_ref().is_none_or(|l| l.contains(&reg))
                    && kind_at(&held[at], reg) == Kind::BigInt
                    && integer(kind_at(&on_words[at], reg))
                {
                    self.line(&format!("r[{reg}] = lc_xof(r[{reg}]);"));
                }
            }
            let (fw, fh) = (&words_frames[&pc], &held_frames[&pc]);
            if fw.len() != fh.len() {
                return Err(format!(
                    "pc {pc}: the two versions of the region iterate {} and {} loops",
                    fw.len(),
                    fh.len()
                ));
            }
            if let Some(fi) = split {
                let (fw, fh) = (fw.clone(), fh.clone());
                self.hand_over(fi, resume, &fw, &fh, hot_owns);
                continue;
            }
            for (w, h) in fw.iter().zip(fh) {
                self.line(&format!("_itv{h} = _itv{w}; _it{h} = _it{w};"));
            }
            self.line(&format!("goto X{resume};"));
        }
        Ok(())
    }

    /// The registers live where the op at `pc` of the current region begins: those it reads, and those live after it
    /// that it does not write.
    fn live_in(&self, pc: usize) -> std::collections::BTreeSet<u16> {
        let op = &self.prog.code[pc];
        let mut live = self.live_out[pc - self.region_span.0].clone();
        self.for_each_written(op, |w| {
            live.remove(&w);
        });
        op.for_each_read_reg(
            |f| self.prog.functions[f as usize].captures.len(),
            |r| {
                live.insert(r);
            },
        );
        live
    }

    /// A word entry's way into its region held exact, resuming at `resume`: the registers live there and their owned
    /// flags into the spill buffers, each loop the machine words iterate (`words`) with the one the region held exact
    /// takes it over in (`held`), and the call to `fn_N_x` that runs the rest, whose result is the word entry's.
    fn hand_over(&mut self, fi: usize, resume: usize, words: &[usize], held: &[usize], owns: bool) {
        for reg in self.live_in(resume) {
            self.line(&format!("lc_held_regs[{reg}] = r[{reg}];"));
            if owns {
                self.line(&format!("lc_held_owns[{reg}] = own[{reg}];"));
            }
        }
        for (k, w) in words.iter().enumerate() {
            self.line(&format!(
                "lc_held_loops[{}] = (int64_t)(intptr_t)_itv{w}; lc_held_loops[{}] = (int64_t)_it{w};",
                2 * k,
                2 * k + 1
            ));
        }
        let id = self.held_entries.len();
        self.held_entries
            .push((id, held.to_vec(), resume, Vec::new()));
        self.line(&format!(
            "return fn_{fi}_x({id}, {}, __ret_own);",
            u8::from(owns)
        ));
    }

    /// Emit one version of region `[start, end)` over its kinds `per_pc`, every op `reachable` reaches, under the
    /// current label prefix; returns the ids of the loops iterating at each op, innermost last.
    fn emit_version(
        &mut self,
        start: usize,
        end: usize,
        in_function: bool,
        holes: &[(usize, usize)],
        per_pc: &[Vec<Kind>],
        reachable: &std::collections::HashSet<usize>,
    ) -> Result<std::collections::HashMap<usize, Vec<usize>>, String> {
        self.iter_stack.clear();
        let mut frames = std::collections::HashMap::new();
        for pc in start..end {
            // An embedded inline-closure body is emitted as its own region — the enclosing block jumps over it —
            // and an op no path reaches (an arm for a constructor the value can never be) is emitted not at all.
            if in_hole(pc, holes) || !reachable.contains(&pc) {
                continue;
            }
            if self.region_targets.contains(&pc) {
                writeln!(self.out, "{}{pc}:;", self.label).unwrap();
            }
            // The bytecode tier's guarded AddAssign is already unchecked on its word version. On
            // measured sparse predicates, replacing its predictable branch with an add on every
            // iteration is slower; expose that profitability refusal instead of silently pretending
            // this Rust/direct-C transform was unavailable.
            if self.label == 'L'
                && crate::optimize::active_config()
                    .is_on(crate::optimization::Opt::BranchlessAccumulate)
            {
                if let (
                    Some(Op::JumpIfFalse { target, .. }),
                    Some(Op::LoadConst { dst: delta, idx }),
                    Some(Op::AddAssign { src, .. }),
                ) = (
                    self.prog.code.get(pc),
                    self.prog.code.get(pc + 1),
                    self.prog.code.get(pc + 2),
                ) {
                    if *target == pc + 3
                        && delta == src
                        && matches!(
                            self.prog.constants.get(*idx as usize),
                            Some(Constant::Int(_))
                        )
                    {
                        crate::optimize::mark_backend_decline(
                            crate::optimization::Opt::BranchlessAccumulate,
                            "branchlessaccumulate: retained the faster predictable bytecode branch",
                        );
                    }
                }
            }
            frames.insert(pc, self.iter_stack.iter().map(|f| f.id).collect());
            // `before` = operand kinds entering this op; `after` = kinds once its write lands (lets a
            // `NewEmptyList` read its own fixpoint-refined element kind).
            let before = per_pc[pc - start].clone();
            let after = per_pc[pc - start + 1].clone();
            let op = self.prog.code[pc].clone();
            self.try_emit_byte_compare_window(pc, end, &before);
            // A definition of an exact web lowers as what it computes, then lands as a cell.
            // A definition a number join lands lowers as what it computes, then lands converted; a collection changed
            // in place takes its item converted as it goes in.
            let version = self.on_words.get() as usize;
            let natural = self.tower_from.borrow()[version].get(&pc).copied();
            let number = self.number_from.borrow()[version].get(&pc).copied();
            let mut computed = after.clone();
            let mut landing = None;
            op.for_each_written_reg(|w| landing = Some(w));
            if let Some(k) = natural.or(number.map(|(from, _)| from)) {
                if let Some(slot) = landing.and_then(|w| computed.get_mut(w as usize)) {
                    *slot = k;
                }
            }
            let lowered = if self.tower { self.lower_tower(&op, &before, pc)? } else { None };
            if lowered.is_none() {
                self.lower_over_exact(&op, &before, &computed, in_function, pc)
                    .map_err(|e| format!("pc {pc}: {e}"))?;
            }
            if let (Some(k), Some(w), false) = (natural, landing, lowered == Some(true)) {
                let cell = self.tower_of(&format!("r[{w}]"), k).map_err(|e| format!("pc {pc}: {e}"))?;
                self.line(&format!("r[{w}] = (int64_t)(intptr_t)({cell});"));
            }
            if let (Some((from, to)), Some(w)) = (number, landing) {
                let v = self.convert_number(&format!("r[{w}]"), from, to).map_err(|e| format!("pc {pc}: {e}"))?;
                self.line(&format!("r[{w}] = {v};"));
            }
            // A call through a word entry tests its escape flag itself, and its machine word carries no tag.
            let tested = std::mem::take(&mut self.twin_arrival);
            if self.arrivals.contains(&pc) && !tested {
                // An exact value that arrived on machine words: a small one's word, or the region held exact.
                let mut dst = 0;
                op.for_each_written_reg(|w| dst = w);
                self.line(&format!("if (r[{dst}] & 1) goto A{pc};"));
                self.line(&format!("r[{dst}] >>= 1;"));
            }
            self.write_owned_flags(&op, &after, pc);
        }
        Ok(frames)
    }

    /// Lower `op`, an operation of the region on machine words that moves into the region held exact (`D{pc}`) when
    /// its result overflows the word. A product by a constant is the machine's own, guarded by the range of the other
    /// operand it cannot overflow on — a test beside the value's path, so a `3 * k + 1` stays one `lea`. Anything else
    /// computes with the overflow check, a constant operand written in as an immediate: the range a sum or difference
    /// with a constant cannot overflow on ends within that constant of i64's ends, a bound no x86 compare takes as an
    /// immediate, so gcc would hold it in a register for the whole function, while the flag of the `add`/`sub` that
    /// computes the value costs nothing beside it.
    fn lower_checked(
        &mut self,
        op: &Op,
        kinds: &[Kind],
        after: &[Kind],
        in_function: bool,
        pc: usize,
    ) -> Result<(), String> {
        let word = |v: i128| {
            if v == i64::MIN as i128 {
                "INT64_MIN".to_string()
            } else {
                format!("{v}LL")
            }
        };
        let known = |reg: u16| self.const_operand(pc, reg).map(i128::from);
        let operand = |reg: u16| known(reg).map_or_else(|| format!("r[{reg}]"), word);
        let checked = |f: &str, dst: u16, a: u16, b: u16| {
            format!(
                "{{ int64_t _o; if ({f}({}, {}, &_o)) goto D{pc}; r[{dst}] = _o; }}",
                operand(a),
                operand(b)
            )
        };
        let line = match *op {
            Op::Add { dst, lhs, rhs } => checked("__builtin_add_overflow", dst, lhs, rhs),
            Op::Sub { dst, lhs, rhs } => checked("__builtin_sub_overflow", dst, lhs, rhs),
            Op::AddAssign { dst, src } => checked("__builtin_add_overflow", dst, dst, src),
            Op::Pow { dst, lhs, rhs } => checked("lc_ipow_ovf", dst, lhs, rhs),
            Op::Mul { dst, lhs, rhs } => {
                let by_constant = known(rhs)
                    .map(|c| (lhs, c))
                    .or_else(|| known(lhs).map(|c| (rhs, c)));
                let Some((x, c)) = by_constant else {
                    self.line(&checked("__builtin_mul_overflow", dst, lhs, rhs));
                    return Ok(());
                };
                let (lo, hi) = product_domain(c);
                let guard = match (lo == i64::MIN as i128, hi == i64::MAX as i128) {
                    (true, true) => None,
                    (true, false) => Some(format!("r[{x}] > {}", word(hi))),
                    (false, true) => Some(format!("r[{x}] < {}", word(lo))),
                    (false, false) => Some(format!(
                        "(uint64_t)r[{x}] - (uint64_t){} > {}ULL",
                        word(lo),
                        (hi - lo) as u64
                    )),
                };
                if let Some(g) = guard {
                    self.line(&format!("if ({g}) goto D{pc};"));
                }
                format!("r[{dst}] = r[{lhs}] * r[{rhs}];")
            }
            // A quotient outgrows the word only as `INT64_MIN / -1`.
            Op::Div { lhs, rhs, .. } | Op::FloorDiv { lhs, rhs, .. } => {
                self.line(&format!(
                    "if (r[{rhs}] == -1 && r[{lhs}] == INT64_MIN) goto D{pc};"
                ));
                return self.lower_op(op, kinds, after, in_function, pc);
            }
            _ => return Err(format!("an overflow check on {}", op.name())),
        };
        self.line(&line);
        Ok(())
    }

    /// The integer register `reg` holds at `pc` when its one definition reaching `pc` is a constant: the last write to
    /// it before `pc` is a `LoadConst` of an `Int`, and no jump lands between them.
    fn const_operand(&self, pc: usize, reg: u16) -> Option<i64> {
        let (start, _) = self.region_span;
        let p = (start..pc).rev().find(|p| {
            let mut writes = false;
            self.for_each_written(&self.prog.code[*p], |w| writes |= w == reg);
            writes
        })?;
        if (p + 1..=pc).any(|q| self.region_targets.contains(&q)) {
            return None;
        }
        match &self.prog.code[p] {
            Op::LoadConst { idx, .. } => match &self.prog.constants[*idx as usize] {
                Constant::Int(v) => Some(*v),
                _ => None,
            },
            _ => None,
        }
    }

    /// Lower `op` where exact integers meet it. An op that reads an exact operand as one sees it as it is; any
    /// other sees the machine integer it holds (loudly, if it holds none), and the register gets its exact value
    /// back after. A result the region holds exact that the op makes as a machine integer is tagged once it lands.
    fn lower_over_exact(
        &mut self,
        op: &Op,
        before: &[Kind],
        after: &[Kind],
        in_function: bool,
        pc: usize,
    ) -> Result<(), String> {
        if self.deopt.contains(&pc) {
            return self.lower_checked(op, before, after, in_function, pc);
        }
        let mut read = Vec::new();
        op.for_each_read_reg(
            |f| self.prog.functions[f as usize].captures.len(),
            |r| read.push(r),
        );
        read.sort_unstable();
        read.dedup();
        let narrowed: Vec<u16> = read
            .into_iter()
            .filter(|r| kind_at(before, *r) == Kind::BigInt && !reads_exact_reg(op, before, *r))
            .collect();
        let mut written = Vec::new();
        op.for_each_written_reg(|w| written.push(w));
        let tagged: Vec<u16> = if self.promoted.borrow()[self.on_words.get() as usize].contains(&pc)
            && !makes_exact(op)
            && !self.arrives_exact(op, before)
        {
            written
                .iter()
                .copied()
                .filter(|w| kind_at(after, *w) == Kind::BigInt)
                .collect()
        } else {
            Vec::new()
        };
        if narrowed.is_empty() && tagged.is_empty() {
            return self.lower_op(op, before, after, in_function, pc);
        }
        let mut kinds = before.to_vec();
        let mut saved = Vec::new();
        for reg in &narrowed {
            kinds[*reg as usize] = Kind::Int;
            let n = self.fresh();
            let narrow = match index_too_wide(op, before, *reg) {
                Some(why) => format!("lc_xindex(_xs{n}, {})", words::index_too_wide(why)),
                None => format!("lc_xnarrow(_xs{n})"),
            };
            self.line(&format!("int64_t _xs{n} = r[{reg}]; r[{reg}] = {narrow};"));
            saved.push((*reg, n));
        }
        let mut made = after.to_vec();
        for reg in &tagged {
            made[*reg as usize] = Kind::Int;
        }
        self.lower_op(op, &kinds, &made, in_function, pc)?;
        for (reg, n) in saved {
            if !written.contains(&reg) {
                self.line(&format!("r[{reg}] = _xs{n};"));
            }
        }
        for reg in tagged {
            self.line(&format!("r[{reg}] = lc_xof(r[{reg}]);"));
        }
        Ok(())
    }

    /// The pcs of region `[start, end)` some path from its entry reaches. A branch on an arm test that cannot
    /// succeed — the value's kind rules the constructor out — follows only its failing edge.
    fn reachable(
        &self,
        start: usize,
        end: usize,
        holes: &[(usize, usize)],
        per_pc: &[Vec<Kind>],
    ) -> std::collections::HashSet<usize> {
        let mut seen = std::collections::HashSet::new();
        let mut work = vec![start];
        while let Some(pc) = work.pop() {
            if pc < start || pc >= end || !seen.insert(pc) {
                continue;
            }
            let op = &self.prog.code[pc];
            let next = {
                let mut n = pc + 1;
                while n < end && in_hole(n, holes) {
                    n += 1;
                }
                n
            };
            let never = |cond: u16| -> bool {
                (start..pc)
                    .rev()
                    .take_while(|p| !matches!(self.prog.code[*p], Op::Jump { .. }))
                    .find_map(|p| match &self.prog.code[p] {
                        Op::TestArm {
                            dst,
                            target,
                            variant,
                        } if *dst == cond => Some(
                            self.static_arm_test(kind_at(&per_pc[p - start], *target), *variant)
                                == Some(false),
                        ),
                        other => {
                            let mut writes = false;
                            self.for_each_written(other, |w| writes |= w == cond);
                            writes.then_some(false)
                        }
                    })
                    == Some(true)
            };
            let yields_nothing = |dst: u16| kind_at(&per_pc[pc - start + 1], dst) == Kind::Never;
            let mut writes_nothing = false;
            op.for_each_written_reg(|w| writes_nothing |= yields_nothing(w));
            match op {
                Op::Return { .. } | Op::ReturnNothing | Op::Halt | Op::FailWith { .. } => {}
                // A loop over a sequence that holds nothing never enters; any other op whose result cannot
                // exist — an index into such a sequence, an operation the VM refuses for its operands' kinds —
                // always fails, and nothing after it runs.
                Op::IterNext { dst, exit } if yields_nothing(*dst) => work.push(*exit),
                _ if writes_nothing => {}
                Op::Jump { target } => work.push(*target),
                Op::JumpIfFalse { cond, target } if never(*cond) => work.push(*target),
                Op::JumpIfTrue { cond, .. } if never(*cond) => work.push(next),
                _ => {
                    if let Some(t) = jump_target(op) {
                        work.push(t);
                    }
                    work.push(next);
                }
            }
        }
        seen
    }

    /// What an arm test decides before the program runs, when the tested value's kind decides it: `Some(false)`
    /// for a constructor the value can never be, `Some(true)` for a record tested against its own type's name.
    fn static_arm_test(&self, tk: Kind, variant: u32) -> Option<bool> {
        let name = self.const_text(variant)?;
        match tk {
            Kind::Enum(ei) => self.variant_index(ei, variant).is_none().then_some(false),
            Kind::Inductive(ii) => {
                let (ty, ctors) = self.inductive(ii);
                let possible = ctors.iter().any(|(c, ks)| c == name && ks.is_some());
                (self.ctor_tag(&ty, name).is_none() || !possible).then_some(false)
            }
            Kind::Struct(_) | Kind::StructSynth(_) => {
                Some(self.struct_name_of_kind(tk).as_deref() == Some(name))
            }
            Kind::Nothing => Some(name == "None"),
            Kind::Int | Kind::Bool | Kind::Float | Kind::Text | Kind::BigInt | Kind::Rational => {
                Some(false)
            }
            _ => None,
        }
    }

    /// Whether a new sequence of kind `kind` in `dst` may empty the buffer the register last held alone, in place: a
    /// register the region tracks, every handle of which is a sequence of that one kind.
    fn reuses_list_buffer(&self, dst: u16, kind: Kind) -> bool {
        self.owned.contains(&dst) && self.handle_kinds.get(&dst) == Some(&kind)
    }

    /// Keep the owned flags true after `op`: a register it wrote holds a collection it just made (flag 1), the
    /// handle its own lowering already accounted for (the target of an owned change), or a handle that may be
    /// held elsewhere (flag 0); a register it read and may have left its handle in — another register, a
    /// field, an element, an iterator, a closure, a message — is no longer the only holder (flag 0).
    fn write_owned_flags(&mut self, op: &Op, after: &[Kind], pc: usize) {
        if self.owned.is_empty()
            || matches!(
                op,
                Op::Return { .. } | Op::ReturnNothing | Op::Halt | Op::FailWith { .. }
            )
        {
            return;
        }
        // RevSwap's lowering exchanges the ownership bits together with the
        // handles. Reclassifying its destinations as ordinary writes would
        // erase that proof and force the next NewEmptyList to allocate.
        if matches!(op, Op::RevSwap { .. }) {
            return;
        }
        let transfer = self.consumed_borrow_transfer(op);
        let mut kept: Vec<u16> = match (mutation(op), op) {
            (Some(Mutation::Owned(target)), _) => vec![target],
            (_, Op::AddAssign { dst, .. }) if kind_at(after, *dst) == Kind::Text => vec![*dst],
            (_, Op::Call { dst, .. } | Op::CallValue { dst, .. })
                if crate::optimize::active_config().is_on(crate::optimization::Opt::Borrow) =>
            {
                vec![*dst]
            }
            _ => Vec::new(),
        };
        if let Some((dst, arg)) = transfer {
            kept.push(dst);
            kept.push(arg);
        }
        // A copy out of a register that is dead from here on moves the collection rather than sharing it: the
        // destination holds it alone exactly when the source did.
        if let Op::Move { dst, src } = op {
            if self.owned.contains(src)
                && !self.owned.contains(dst)
                && crate::optimize::active_config().is_on(crate::optimization::Opt::Borrow)
                && self.feeds_readonly_call_arg(pc, *dst)
            {
                // The argument window merely borrows the handle.  Keep the
                // source register as its sole owner so a loop-local NewList in
                // that register can clear and refill the allocation next time.
                self.line("/* retain ownership across read-only call argument */");
                crate::optimize::mark_fired(crate::optimization::Opt::Borrow);
                return;
            }
            if self.owned.contains(dst)
                && self.owned.contains(src)
                && dst != src
                && self.dead_after(pc, *src)
            {
                self.line(&format!("own[{dst}] = own[{src}]; own[{src}] = 0;"));
                return;
            }
        }
        let taken = match op {
            Op::GetField { dst, .. } => Some(*dst).filter(|_| self.takes_out_field(pc)),
            Op::GlobalGet { dst, .. } => Some(*dst).filter(|_| self.takes_out_global(pc)),
            _ => None,
        };
        let inspected = inspected_regs(op);
        let fresh = fresh_regs(op, after);
        let mut written = Vec::new();
        self.for_each_written(op, |w| written.push(w));
        let mut read = Vec::new();
        op.for_each_read_reg(
            |f| self.prog.functions[f as usize].captures.len(),
            |r| read.push(r),
        );
        let mut escaped: Vec<u16> = read
            .into_iter()
            .filter(|r| {
                self.owned.contains(r)
                    && !written.contains(r)
                    && !inspected.contains(r)
                    && !kept.contains(r)
            })
            .collect();
        escaped.sort_unstable();
        escaped.dedup();
        for r in escaped {
            self.line(&format!("own[{r}] = 0;"));
        }
        written.sort_unstable();
        written.dedup();
        for w in written {
            if !self.owned.contains(&w) || kept.contains(&w) || taken == Some(w) {
                continue;
            }
            self.line(&format!(
                "own[{w}] = {};",
                if fresh.contains(&w) { 1 } else { 0 }
            ));
        }
        if let Some((dst, arg)) = transfer {
            if self.owned.contains(&dst) && self.owned.contains(&arg) && dst != arg {
                self.line("/* preserve sole ownership through consumed borrow */");
                self.line(&format!("own[{dst}] = own[{arg}]; own[{arg}] = 0;"));
                crate::optimize::mark_fired(crate::optimization::Opt::Borrow);
            }
        }
    }

    /// Emit the VM's refusal of builtin `id` for arguments of kinds `ks`, taken from the builtin itself.
    fn builtin_refusal(
        &mut self,
        id: crate::semantics::builtins::BuiltinId,
        ks: &[Kind],
    ) -> Result<(), String> {
        let samples: Option<Vec<RuntimeValue>> = ks.iter().map(|k| sample_value(*k)).collect();
        let samples = samples.ok_or_else(|| format!("{id:?} over {ks:?}"))?;
        self.line(&format!(
            "logos_fatalf({});",
            words::builtin_refused(id, samples)
        ));
        Ok(())
    }

    /// The C expression of register `reg`'s owned flag: `own[reg]` for a register the region tracks, `0` for one it
    /// does not (whose value may be held anywhere).
    fn owned_flag(&self, reg: u16) -> String {
        if self.owned.contains(&reg) {
            format!("own[{reg}]")
        } else {
            "0".to_string()
        }
    }

    /// Release scalar-sequence boxes this function still owns when it returns, except for the
    /// register transferred to its caller. Rust drops the equivalent unique `Vec`s automatically;
    /// bytecode C used to retain every recursive temporary until process exit. `own[r]` is already
    /// the emitter's copy-on-write proof that no other live handle aliases this allocation, and
    /// `handle_kinds` limits this first reclamation step to registers whose every value has the
    /// same flat scalar-sequence representation (no nested element ownership to walk).
    fn release_owned_scalar_sequences(&mut self, except: Option<u16>) {
        if !crate::optimize::active_config().is_on(crate::optimization::Opt::Unbox) {
            return;
        }
        // `store_inout_params` has just handed these handles back through the
        // caller's pointer slots. They escape even though this frame's own[]
        // proof still says the callee was their sole owner.
        let inout = self
            .current_fn
            .map(|fi| self.inout_regs(fi))
            .unwrap_or_default();
        let drops: Vec<(u16, Kind)> = self
            .owned
            .iter()
            .copied()
            .filter(|reg| Some(*reg) != except)
            .filter(|reg| !inout.contains(reg))
            .filter_map(|reg| self.handle_kinds.get(&reg).copied().map(|kind| (reg, kind)))
            .filter(|(_, kind)| matches!(kind, Kind::SeqInt | Kind::SeqFloat | Kind::SeqBool))
            .collect();
        for (reg, kind) in drops {
            let ty = kind
                .seq_c_type()
                .expect("a scalar sequence has a C representation");
            let n = self.fresh();
            self.line(&format!(
                "if (own[{reg}] && r[{reg}]) {{ {ty} *_drop{n} = ({ty}*)(intptr_t)r[{reg}]; lc_gc_free(_drop{n}->data); lc_gc_free(_drop{n}); own[{reg}] = 0; }} /* unique local drop */"
            ));
            crate::optimize::mark_fired(crate::optimization::Opt::Unbox);
        }
    }

    /// Before an owned change to register `target` (of kind `k`): unless the register is the only holder of its
    /// collection, give it a copy of its own — the VM's copy-on-write. A `mutable` parameter arrives its caller's
    /// own (the call site isolates it) and changes in place until another holder shares it; its final value goes
    /// back to the caller whichever it is. A CRDT changes in place, every holder sharing it by design.
    fn own_target(&mut self, target: u16, k: Kind) -> Result<(), String> {
        if matches!(k, Kind::CrdtCounter) {
            return Ok(());
        }
        let mark = self.out.len();
        self.line(&format!("if (!own[{target}]) {{"));
        match self.clone_collection_expr(target, k)? {
            Some(copy) => {
                self.line(&format!("r[{target}] = {copy};"));
                self.line(&format!("own[{target}] = 1;"));
                self.line("}");
            }
            None => self.out.truncate(mark),
        }
        Ok(())
    }

    /// The registers live after each op of region `[start, end)`: a backward fixpoint over its control flow (a
    /// fall-through, a jump, a branch's two ways, a loop's exit), skipping the embedded closure bodies.
    fn liveness(
        &self,
        start: usize,
        end: usize,
        holes: &[(usize, usize)],
    ) -> Vec<std::collections::BTreeSet<u16>> {
        use std::collections::BTreeSet;
        let n = end - start;
        let succs = |pc: usize| self.successors(pc, start, end, holes);
        let mut live_in: Vec<BTreeSet<u16>> = vec![BTreeSet::new(); n];
        let mut live_out: Vec<BTreeSet<u16>> = vec![BTreeSet::new(); n];
        loop {
            let mut changed = false;
            for pc in (start..end).rev() {
                if in_hole(pc, holes) {
                    continue;
                }
                let mut out = BTreeSet::new();
                for s in succs(pc) {
                    out.extend(live_in[s - start].iter().copied());
                }
                let op = &self.prog.code[pc];
                let mut inn = out.clone();
                self.for_each_written(op, |w| {
                    inn.remove(&w);
                });
                op.for_each_read_reg(
                    |f| self.prog.functions[f as usize].captures.len(),
                    |r| {
                        inn.insert(r);
                    },
                );
                if out != live_out[pc - start] || inn != live_in[pc - start] {
                    live_out[pc - start] = out;
                    live_in[pc - start] = inn;
                    changed = true;
                }
            }
            if !changed {
                return live_out;
            }
        }
    }

    /// The pcs control can pass to from the op at `pc`, within region `[start, end)` and around its holes.
    fn successors(
        &self,
        pc: usize,
        start: usize,
        end: usize,
        holes: &[(usize, usize)],
    ) -> Vec<usize> {
        let op = &self.prog.code[pc];
        let mut out = Vec::new();
        match op {
            Op::Return { .. } | Op::ReturnNothing | Op::Halt | Op::FailWith { .. } => {}
            Op::Jump { target } => out.push(*target),
            _ => {
                if let Some(t) = jump_target(op) {
                    out.push(t);
                }
                let mut next = pc + 1;
                while next < end && in_hole(next, holes) {
                    next += 1;
                }
                out.push(next);
            }
        }
        out.retain(|p| *p >= start && *p < end);
        out
    }

    /// Whether register `reg` is dead after the op at `pc`: no path from there reads it before writing it.
    fn dead_after(&self, pc: usize, reg: u16) -> bool {
        let (start, _) = self.region_span;
        self.live_out
            .get(pc - start)
            .is_some_and(|live| !live.contains(&reg))
    }

    /// Whether `reg` is dead on entry to `pc`. This differs from asking about
    /// the previous instruction in a loop: that instruction's live-out set
    /// follows its back edge, while a synthesized fast path can jump directly
    /// to the loop exit.
    fn dead_before(&self, pc: usize, reg: u16) -> bool {
        let (start, _) = self.region_span;
        let Some(live_out) = self.live_out.get(pc - start) else {
            return false;
        };
        let mut live = live_out.clone();
        self.for_each_written(&self.prog.code[pc], |written| {
            live.remove(&written);
        });
        self.prog.code[pc].for_each_read_reg(
            |function| self.prog.functions[function as usize].captures.len(),
            |read| {
                live.insert(read);
            },
        );
        !live.contains(&reg)
    }

    /// Inject the bytecode-C counterpart of the Rust/direct-C fixed-window
    /// text comparison.  The original scalar bytecode remains immediately
    /// below this guard as the exact fallback; the fast path is taken only for
    /// an owned ASCII text builder, an ASCII needle window, and in-bounds word
    /// indices.  This keeps Unicode, bounds failures, and exact-integer edge
    /// cases on the ordinary lowering while removing per-character Text
    /// allocation from the benchmark's hot path.
    fn try_emit_byte_compare_window(&mut self, pc: usize, end: usize, before: &[Kind]) -> bool {
        if self.label != 'L'
            || !crate::optimize::active_config().is_on(crate::optimization::Opt::Simd)
            || pc + 17 > end
        {
            return false;
        }

        let op = |offset: usize| &self.prog.code[pc + offset];
        let int_const = |idx: u32| match self.prog.constants.get(idx as usize) {
            Some(Constant::Int(value)) => Some(*value),
            _ => None,
        };
        let indexed = |op: &Op| match op {
            Op::Index {
                dst,
                collection,
                index,
            }
            | Op::IndexUnchecked {
                dst,
                collection,
                index,
            } => Some((*dst, *collection, *index)),
            _ => None,
        };

        let (len_reg, len) = match op(0) {
            Op::LoadConst { dst, idx } => (*dst, int_const(*idx)),
            _ => return false,
        };
        let Some(len) = len.filter(|len| (1..=16).contains(len)) else {
            return false;
        };
        let (cond_reg, j_reg) = match op(1) {
            Op::Lt { dst, lhs, rhs } if *rhs == len_reg => (*dst, *lhs),
            _ => return false,
        };
        let exit = match op(2) {
            Op::JumpIfFalse { cond, target } if *cond == cond_reg && *target == pc + 17 => *target,
            _ => return false,
        };
        let (text_index, text_base) = match op(3) {
            Op::Add { dst, lhs, rhs } if *lhs == j_reg => (*dst, *rhs),
            Op::Add { dst, lhs, rhs } if *rhs == j_reg => (*dst, *lhs),
            _ => return false,
        };
        let Some((text_char, text, index)) = indexed(op(4)) else {
            return false;
        };
        if index != text_index {
            return false;
        }
        let one_reg = match op(5) {
            Op::LoadConst { dst, idx } if int_const(*idx) == Some(1) => *dst,
            _ => return false,
        };
        let needle_index = match op(6) {
            Op::Add { dst, lhs, rhs }
                if (*lhs == j_reg && *rhs == one_reg) || (*rhs == j_reg && *lhs == one_reg) =>
            {
                *dst
            }
            _ => return false,
        };
        let Some((needle_char, needle, index)) = indexed(op(7)) else {
            return false;
        };
        if index != needle_index {
            return false;
        }
        let mismatch = match op(8) {
            Op::NotEq { dst, lhs, rhs }
                if (*lhs == text_char && *rhs == needle_char)
                    || (*rhs == text_char && *lhs == needle_char) =>
            {
                *dst
            }
            _ => return false,
        };
        if !matches!(op(9), Op::JumpIfFalse { cond, target } if *cond == mismatch && *target == pc + 14)
        {
            return false;
        }
        let (flag_value_reg, flag_value) = match op(10) {
            Op::LoadConst { dst, idx } => (*dst, int_const(*idx)),
            _ => return false,
        };
        let Some(flag_value) = flag_value else {
            return false;
        };
        let flag = match op(11) {
            Op::Move { dst, src } if *src == flag_value_reg => *dst,
            _ => return false,
        };
        let break_reg = match op(12) {
            Op::LoadConst { dst, idx } if int_const(*idx) == Some(len) => *dst,
            _ => return false,
        };
        if !matches!(op(13), Op::Move { dst, src } if *dst == j_reg && *src == break_reg) {
            return false;
        }
        let inc_reg = match op(14) {
            Op::LoadConst { dst, idx } if int_const(*idx) == Some(1) => *dst,
            _ => return false,
        };
        if !matches!(op(15), Op::AddAssign { dst, src } if *dst == j_reg && *src == inc_reg)
            || !matches!(op(16), Op::Jump { target } if *target == pc)
        {
            return false;
        }

        let (region_start, region_end) = self.region_span;
        if (region_start..region_end).any(|source| {
            !(pc..exit).contains(&source)
                && jump_target(&self.prog.code[source])
                    .is_some_and(|target| (pc + 1..exit).contains(&target))
        }) {
            return false;
        }
        let mut written = std::collections::BTreeSet::new();
        let mut flag_writes = 0;
        let mut index_writes = 0;
        for inner in pc..exit {
            self.for_each_written(&self.prog.code[inner], |reg| {
                written.insert(reg);
                flag_writes += usize::from(reg == flag);
                index_writes += usize::from(reg == j_reg);
            });
        }
        if [text, text_base, needle]
            .iter()
            .any(|reg| written.contains(reg))
            || flag_writes != 1
            || index_writes != 2
            || written
                .iter()
                .any(|reg| *reg != flag && !self.dead_before(exit, *reg))
        {
            return false;
        }

        if kind_at(before, text) != Kind::Text
            || kind_at(before, needle) != Kind::Text
            || kind_at(before, text_base) != Kind::Int
            || !self.owned.contains(&text)
        {
            return false;
        }

        let text_ptr = format!("((const unsigned char *)(intptr_t)r[{text}])");
        let needle_ptr = format!("((const unsigned char *)(intptr_t)r[{needle}])");
        let ascii_needle = (0..len)
            .map(|offset| format!("{needle_ptr}[{offset}] < 0x80"))
            .collect::<Vec<_>>()
            .join(" && ");
        let equal = (0..len)
            .map(|offset| {
                format!(
                    "{text_ptr}[(size_t)(r[{text_base}] - 1LL) + {offset}] == {needle_ptr}[{offset}]"
                )
            })
            .collect::<Vec<_>>()
            .join(" && ");
        self.line("/* shared fixed-window byte compare: guarded bytecode-C fast path */");
        self.line(&format!(
            "if (own[{text}] && lc_head(r[{text}])->ascii && lc_head(r[{text}])->len >= {len}u && r[{text_base}] >= 1LL && (uint64_t)(r[{text_base}] - 1LL) <= (uint64_t)(lc_head(r[{text}])->len - {len}u) && strlen((const char *)(intptr_t)r[{needle}]) >= {len}u && {ascii_needle}) {{"
        ));
        self.line(&format!("if (!({equal})) r[{flag}] = {flag_value}LL;"));
        self.line(&format!("goto L{exit};"));
        self.line("}");
        crate::optimize::mark_fired(crate::optimization::Opt::Simd);
        true
    }

    /// `obj`'s field cell `cell` := `value`, under value semantics: the record becomes the register's own first,
    /// and the field-ownership bit records whether the record is now its field's only holder — true when
    /// `value` held the collection alone and is dead from here on.
    fn store_field(
        &mut self,
        obj: u16,
        ok: Kind,
        cell: usize,
        value: u16,
        value_kind: Kind,
        pc: usize,
    ) -> Result<(), String> {
        self.own_target(obj, ok)?;
        let stored = self.field_store_value(ok, cell, value, value_kind)?;
        self.line(&format!(
            "((int64_t*)(intptr_t)r[{obj}])[{cell}] = {stored};"
        ));
        if cell < 64 {
            let word = self
                .field_owner_word(obj, ok)
                .ok_or("a field store on a record of unknown layout")?;
            let sole = if self.owned.contains(&value) && self.dead_after(pc, value) {
                format!("((uint64_t)(own[{value}] != 0) << {cell})")
            } else {
                "0".to_string()
            };
            self.line(&format!("{word} = ({word} & ~(1ULL << {cell})) | {sole};"));
        }
        Ok(())
    }

    /// After `dst` := the collection in field `cell` of the record in `obj`. When the read takes the collection
    /// out of the record, `dst` becomes its only holder exactly when the record was. Otherwise the record keeps
    /// holding it alone only if `dst` is never more than inspected and is dead wherever the field could change
    /// in place (a push into the field, a call the record reaches).
    fn read_field_collection(&mut self, dst: u16, obj: u16, ok: Kind, cell: usize, pc: usize) {
        let Some(word) = self.field_owner_word(obj, ok) else {
            return;
        };
        if cell >= 64 {
            return;
        }
        if self.takes_out_field(pc) {
            if self.owned.contains(&dst) {
                let record = if self.owned.contains(&obj) {
                    format!("own[{obj}]")
                } else {
                    "0".to_string()
                };
                self.line(&format!(
                    "own[{dst}] = {record} && (({word} >> {cell}) & 1);"
                ));
            }
        } else if !self.stays_an_inspection(dst) {
            self.line(&format!("{word} &= ~(1ULL << {cell});"));
        }
    }

    /// Whether register `reg`'s handle, once read out of a record, can never be seen changed by a later change
    /// to that record's field: every read of it in the region is an inspection, and it is dead at every op that
    /// could change a field's collection in place.
    fn stays_an_inspection(&self, reg: u16) -> bool {
        let (start, end) = self.region_span;
        (start..end).all(|pc| {
            let op = &self.prog.code[pc];
            let mut read = false;
            op.for_each_read_reg(
                |f| self.prog.functions[f as usize].captures.len(),
                |r| read |= r == reg,
            );
            let inspected = !read || inspected_regs(op).contains(&reg);
            let changes_fields = matches!(
                op,
                Op::ListPushField { .. } | Op::Call { .. } | Op::CallValue { .. }
            );
            inspected && (!changes_fields || self.dead_after(pc, reg))
        })
    }

    /// Whether the `GlobalGet` at `pc` takes its global's collection out: the ops that follow it (after constant
    /// loads) empty that global with `nothing` before anything else happens.
    fn takes_out_global(&self, pc: usize) -> bool {
        let Op::GlobalGet { idx, .. } = &self.prog.code[pc] else {
            return false;
        };
        let (_, end) = self.region_span;
        let mut loaded: std::collections::HashMap<u16, &Constant> =
            std::collections::HashMap::new();
        for at in pc + 1..end {
            if self.region_targets.contains(&at) {
                return false;
            }
            match &self.prog.code[at] {
                Op::LoadConst { dst, idx: c } => {
                    loaded.insert(*dst, &self.prog.constants[*c as usize]);
                }
                Op::GlobalSet { idx: g, src } => {
                    return g == idx && matches!(loaded.get(src), Some(Constant::Nothing))
                }
                _ => return false,
            }
        }
        false
    }

    /// Whether the `GetField` at `pc` takes its field's collection out of the record: the ops that follow it (after
    /// constant loads) overwrite that field with `nothing` before anything else happens — the bytecode's
    /// take-out/put-back idiom for changing a collection that lives in a field, which leaves the register the
    /// field's only holder whenever the record was.
    fn takes_out_field(&self, pc: usize) -> bool {
        let Op::GetField { obj, field, .. } = &self.prog.code[pc] else {
            return false;
        };
        let Some(field_name) = self.const_text(*field) else {
            return false;
        };
        let (_, end) = self.region_span;
        let mut loaded: std::collections::HashMap<u16, &Constant> =
            std::collections::HashMap::new();
        for at in pc + 1..end {
            if self.region_targets.contains(&at) {
                return false;
            }
            match &self.prog.code[at] {
                Op::LoadConst { dst, idx } => {
                    loaded.insert(*dst, &self.prog.constants[*idx as usize]);
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
                    let names_field =
                        matches!(loaded.get(index), Some(Constant::Text(t)) if t == field_name);
                    let clears = matches!(loaded.get(value), Some(Constant::Nothing));
                    return *collection == *obj && names_field && clears;
                }
                _ => return false,
            }
        }
        false
    }
    /// Select TextBytes calls whose result flows only through register moves
    /// into one for-each iterator. Those calls can keep the source text's
    /// byte pointer instead of allocating and expanding a Seq_i64.
    fn text_byte_iteration_plan(
        &self,
        start: usize,
        end: usize,
        holes: &[(usize, usize)],
        reachable: &std::collections::HashSet<usize>,
    ) -> (
        std::collections::HashSet<usize>,
        std::collections::HashSet<usize>,
    ) {
        use crate::semantics::builtins::BuiltinId;

        if !crate::optimize::active_config().is_on(crate::optimization::Opt::TextBytesView) {
            return Default::default();
        }

        let move_root = |pc: usize, mut reg: u16| {
            let mut before = pc;
            loop {
                let Some(def) = (start..before).rev().find(|at| {
                    let mut writes = false;
                    self.prog.code[*at].for_each_written_reg(|w| writes |= w == reg);
                    writes
                }) else {
                    return reg;
                };
                if (def + 1..=before).any(|at| self.region_targets.contains(&at)) {
                    return reg;
                }
                match self.prog.code[def] {
                    Op::Move { dst, src } if dst == reg => {
                        reg = src;
                        before = def;
                    }
                    _ => return reg,
                }
            }
        };

        let mut calls = std::collections::HashSet::new();
        let mut iters = std::collections::HashSet::new();
        for pc in start..end {
            let Op::CallBuiltin {
                dst,
                builtin: BuiltinId::TextBytes,
                args_start,
                ..
            } = self.prog.code[pc]
            else {
                continue;
            };
            if !reachable.contains(&pc) || in_hole(pc, holes) {
                continue;
            }

            let mut aliases = std::collections::BTreeSet::from([dst]);
            let source_root = move_root(pc, args_start);
            let sources = std::collections::BTreeSet::from([args_start, source_root]);
            let mut selected_iter = None;
            let mut valid = true;

            for at in pc + 1..end {
                if in_hole(at, holes) || !reachable.contains(&at) {
                    continue;
                }
                let op = &self.prog.code[at];
                if selected_iter.is_none()
                    && (self.region_targets.contains(&at) || jump_target(op).is_some())
                {
                    valid = false;
                    break;
                }

                let mut reads = std::collections::BTreeSet::new();
                op.for_each_read_reg(
                    |f| self.prog.functions[f as usize].captures.len(),
                    |r| {
                        reads.insert(r);
                    },
                );
                if reads.iter().any(|r| sources.contains(r)) {
                    valid = false;
                    break;
                }
                let alias_reads: Vec<u16> = reads
                    .iter()
                    .copied()
                    .filter(|r| aliases.contains(r))
                    .collect();

                match op {
                    Op::Move { dst, src }
                        if aliases.contains(src) && alias_reads.as_slice() == [*src] =>
                    {
                        aliases.insert(*dst);
                    }
                    Op::IterPrepare { iterable }
                        if aliases.contains(iterable) && selected_iter.is_none() =>
                    {
                        selected_iter = Some(at);
                    }
                    _ if !alias_reads.is_empty() => {
                        valid = false;
                        break;
                    }
                    _ => {
                        op.for_each_written_reg(|w| {
                            aliases.remove(&w);
                        });
                    }
                }
            }

            if valid {
                if let Some(iter_pc) = selected_iter {
                    calls.insert(pc);
                    iters.insert(iter_pc);
                }
            }
        }
        (calls, iters)
    }

    /// A forward kind pass over region `[start, end)` (mirrors `codegen_riscv::infer_kinds`, scalar+seq subset).
    /// Returns `end - start + 1` register-kind vectors: entry `i` is the state BEFORE op `start+i`, and the
    /// last entry is the state AFTER the final op (so a creation site can read its post-write kind). A bounded
    /// forward FIXPOINT refines a `SeqUnknown` created empty to the concrete element kind a later `ListPush`
    /// establishes — seeding each pass's creation with the kind that slot reached in the previous pass.
    fn infer_kinds(
        &self,
        start: usize,
        end: usize,
        seed: Vec<Kind>,
        holes: &[(usize, usize)],
    ) -> Vec<Vec<Kind>> {
        // The region's exact webs grow with what holds a cell, and its number variables widen with the forms their
        // definitions make, until both hold still: each pass's kinds decide the webs and the joins, every definition of
        // an exact web makes a cell in the next, and every definition a join lands lands as its variable's kind.
        let mut forced = crate::vm::tower::Webs::default();
        let mut landings = numbers::Landings::default();
        for round in 0.. {
            let mut entry = seed.clone();
            for (r, k) in &landings.entries {
                if let Some(slot) = entry.get_mut(*r as usize) {
                    *slot = *k;
                }
            }
            for r in &forced.entries {
                if let Some(k) = entry.get_mut(*r as usize) {
                    *k = Kind::Tower;
                }
            }
            let kinds = self.infer_kinds_forced(start, end, entry.clone(), holes, &forced, &landings.defs);
            let next = if self.tower {
                self.tower_webs(start, end, holes, &kinds, &entry)
            } else {
                crate::vm::tower::Webs::default()
            };
            let mut next_landings = self.number_landings(start, end, holes, &kinds, &seed);
            if round == SETTLE_ROUNDS {
                next_landings = landings.clone();
                next_landings.refused.push(crate::vm::variables::Site::Op(start));
            }
            if next == forced && next_landings == landings {
                let converted = forced
                    .entries
                    .iter()
                    .filter_map(|r| {
                        let k = seed.get(*r as usize).copied().unwrap_or(Kind::Unknown);
                        (k != Kind::Tower).then_some((*r, k))
                    })
                    .collect();
                self.tower_entries.borrow_mut().insert(start, converted);
                let widened = landings
                    .entries
                    .iter()
                    .filter(|(r, _)| !forced.entries.contains(r))
                    .map(|(r, k)| (*r, seed.get(*r as usize).copied().unwrap_or(Kind::Unknown), *k))
                    .collect();
                self.number_entries.borrow_mut().insert(start, (widened, landings.refused));
                return kinds;
            }
            forced = next;
            landings = next_landings;
        }
        unreachable!("the rounds end when the webs and joins hold still")
    }

    /// [`Emitter::infer_kinds`] with the definitions of `forced`'s exact webs making cells, and each definition of `lands`
    /// landing as its kind there.
    fn infer_kinds_forced(
        &self,
        start: usize,
        end: usize,
        seed: Vec<Kind>,
        holes: &[(usize, usize)],
        forced: &crate::vm::tower::Webs,
        lands: &std::collections::BTreeMap<usize, Kind>,
    ) -> Vec<Vec<Kind>> {
        let n = seed.len();
        let mut overrides = vec![Kind::Unknown; n];
        let mut per_pc: Vec<Vec<Kind>> = Vec::new();
        let set = |cur: &mut Vec<Kind>, reg: u16, k: Kind| {
            let i = reg as usize;
            if i < cur.len() {
                cur[i] = k;
            }
        };
        let kind_of =
            |cur: &[Kind], reg: u16| -> Kind { *cur.get(reg as usize).unwrap_or(&Kind::Unknown) };
        for _pass in 0..8 {
            let mut cur = seed.clone();
            per_pc = Vec::with_capacity(end - start + 1);
            let mut iter_elems: Vec<Kind> = Vec::new();
            // The most recent `TestArm` (target reg, enum index, variant index) — the arm a following `BindArm`
            // extracts a payload field from, so that field gets the variant's declared kind.
            let mut last_test: Option<(u16, u16, usize)> = None;
            // The most recent `TestArm` on an undeclared enum (target reg, constructor name) — the constructor a
            // following `BindArm` on it extracts a payload of.
            let mut last_test_ctor: Option<(u16, String)> = None;
            for pc in start..end {
                per_pc.push(cur.clone());
                // An embedded closure body's ops belong to a separate region — control jumps over them here, so
                // they contribute NO kind transfer to this region's register state (passthrough the hole).
                if in_hole(pc, holes) {
                    continue;
                }
                // An op that does not read exact integers as they are sees the machine integers they hold, as its
                // lowering does ([`Emitter::lower_over_exact`]): what it builds from them holds machine integers.
                let mut narrowed: Vec<u16> = Vec::new();
                self.prog.code[pc].for_each_read_reg(
                    |f| self.prog.functions[f as usize].captures.len(),
                    |r| {
                        if kind_of(&cur, r) == Kind::BigInt
                            && !reads_exact_reg(&self.prog.code[pc], &cur, r)
                            && !narrowed.contains(&r)
                        {
                            narrowed.push(r);
                        }
                    },
                );
                for r in &narrowed {
                    set(&mut cur, *r, Kind::Int);
                }
                // The (dst, kind) this op writes — or `None` for control flow / iterator bookkeeping.
                let write: Option<(u16, Kind)> = match &self.prog.code[pc] {
                    Op::LoadConst { dst, idx } => {
                        let k = match &self.prog.constants[*idx as usize] {
                            Constant::Int(_) | Constant::Duration(_) => Kind::Int,
                            Constant::Bool(_) => Kind::Bool,
                            Constant::Float(_) => Kind::Float,
                            Constant::Text(_) => Kind::Text,
                            // `nothing` — the unit value, riding the slot as `0`; a field seeded with it takes
                            // its real kind from its later `StructInsert` / `CrdtBump` / `CrdtResolve`.
                            Constant::Nothing => Kind::Nothing,
                            Constant::Date(_) => Kind::Date,
                            Constant::Moment(_) => Kind::Moment,
                            Constant::Time(_) => Kind::Time,
                            Constant::Span { .. } => Kind::Span,
                            _ => Kind::Unknown,
                        };
                        Some((*dst, k))
                    }
                    Op::Move { dst, src } => Some((*dst, kind_of(&cur, *src))),
                    // Add/Sub/Mul/Pow: the numeric join of the operands (see `arith_result_kind`).
                    Op::Add { dst, lhs, rhs } => {
                        let (lk, rk) = (kind_of(&cur, *lhs), kind_of(&cur, *rhs));
                        // A Text operand makes `+` concatenation (see `arith`).
                        let k = if lk == Kind::Text || rk == Kind::Text {
                            Kind::Text
                        } else {
                            self.arith_result_kind(lk, rk)
                        };
                        Some((*dst, k))
                    }
                    Op::Sub { dst, lhs, rhs }
                    | Op::Mul { dst, lhs, rhs }
                    | Op::Pow { dst, lhs, rhs } => Some((
                        *dst,
                        self.arith_result_kind(kind_of(&cur, *lhs), kind_of(&cur, *rhs)),
                    )),
                    // A quotient is no larger than its dividend, a remainder smaller than its modulus: each is exact
                    // only when that operand is.
                    Op::Div { dst, lhs, rhs } | Op::FloorDiv { dst, lhs, rhs } => Some((
                        *dst,
                        bounded_by(
                            join(kind_of(&cur, *lhs), kind_of(&cur, *rhs)),
                            kind_of(&cur, *lhs),
                        ),
                    )),
                    Op::Mod { dst, lhs, rhs } => Some((
                        *dst,
                        bounded_by(
                            join(kind_of(&cur, *lhs), kind_of(&cur, *rhs)),
                            kind_of(&cur, *rhs),
                        ),
                    )),
                    // A bitwise result is a machine integer, whatever its operands (`lc_xbitand` and kin).
                    Op::BitXor { dst, lhs, rhs }
                    | Op::BitAnd { dst, lhs, rhs }
                    | Op::BitOr { dst, lhs, rhs } => Some((
                        *dst,
                        match join(kind_of(&cur, *lhs), kind_of(&cur, *rhs)) {
                            Kind::Rational => Kind::Int,
                            joined => bounded_by(joined, Kind::Int),
                        },
                    )),
                    // A word shifts by an integer count and stays a word.
                    Op::Shl { dst, lhs, rhs } | Op::Shr { dst, lhs, rhs } => {
                        let lk = kind_of(&cur, *lhs);
                        Some((
                            *dst,
                            match (lk, join(lk, kind_of(&cur, *rhs))) {
                                (Kind::Word(_), _) => lk,
                                (_, Kind::Rational) => Kind::Int,
                                (_, joined) => bounded_by(joined, Kind::Int),
                            },
                        ))
                    }
                    Op::AddAssign { dst, src } => {
                        let (dk, sk) = (kind_of(&cur, *dst), kind_of(&cur, *src));
                        Some((
                            *dst,
                            if dk == Kind::Text || sk == Kind::Text {
                                Kind::Text
                            } else {
                                join(dk, sk)
                            },
                        ))
                    }
                    Op::Concat { dst, .. } | Op::FormatValue { dst, .. } => {
                        Some((*dst, Kind::Text))
                    }
                    Op::Lt { dst, .. }
                    | Op::Gt { dst, .. }
                    | Op::LtEq { dst, .. }
                    | Op::GtEq { dst, .. }
                    | Op::Eq { dst, .. }
                    | Op::NotEq { dst, .. }
                    | Op::ApproxEq { dst, .. }
                    | Op::Not { dst, .. }
                    | Op::Contains { dst, .. } => Some((*dst, Kind::Bool)),
                    Op::Length { dst, .. } => Some((*dst, Kind::Int)),
                    // Strength-reduced division by a constant: exact when its dividend is; a remainder by a
                    // constant is a machine integer, or the exact number a rational dividend makes.
                    Op::DivPow2 { dst, lhs, .. }
                    | Op::MagicDivU {
                        dst,
                        lhs,
                        mul_back: 0,
                        ..
                    } => Some((*dst, bounded_by(kind_of(&cur, *lhs), kind_of(&cur, *lhs)))),
                    Op::MagicDivU { dst, lhs, .. } if kind_of(&cur, *lhs) == Kind::Rational => Some((*dst, Kind::Rational)),
                    Op::MagicDivU { dst, .. } => Some((*dst, Kind::Int)),
                    // Exact division yields an exact rational (`7 / 2` → `7/2`), regardless of the Int operands.
                    Op::ExactDiv { dst, .. } => Some((*dst, Kind::Rational)),
                    Op::NewRange { dst, .. } => Some((*dst, Kind::SeqInt)),
                    Op::NewEmptyListI32 { dst } => Some((*dst, Kind::SeqInt)),
                    Op::NewEmptyList { dst }
                    | Op::NewEmptyListWithCapacity { dst, .. }
                    | Op::NewEmptyListFixed { dst, .. } => Some((*dst, Kind::SeqUnknown)),
                    Op::NewList { dst, start, count } => {
                        let k = if *count == 0 {
                            Kind::SeqUnknown
                        } else {
                            let items: Vec<Kind> = (0..*count).map(|i| kind_of(&cur, *start + i)).collect();
                            self.list_kind(&items).unwrap_or_else(|_| self.seq_kind_for(items[0]))
                        };
                        Some((*dst, k))
                    }
                    Op::ListPush { list, value } => {
                        Some((*list, self.seq_kind_for(kind_of(&cur, *value))))
                    }
                    // A pop meets only a list with an item to give (`resolve_pop` checks every other one first), so
                    // what it yields is that item.
                    Op::ListPop { list, dst } => Some((*dst, self.elem_kind(kind_of(&cur, *list)))),
                    Op::NewEmptySet { dst } => Some((*dst, Kind::SetUnknown)),
                    Op::SetAdd { set, value } => Some((*set, kind_of(&cur, *value).set_of())),
                    Op::NewEmptyMap { dst } | Op::NewEmptyMapWithCapacity { dst, .. } => {
                        Some((*dst, Kind::MapUnknown))
                    }
                    // `SetIndex` on a MAP (`item k of m to v`) refines the map's (key,value) kinds in place; on
                    // a sequence it is an in-place element write that leaves the kind unchanged, save a rational
                    // stored among integers, which makes the list one of rationals (its variable lands as one).
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
                        let ck = kind_of(&cur, *collection);
                        if matches!(ck, Kind::Map(_) | Kind::MapUnknown) {
                            // A map that holds a value of the exact extension holds every value as one.
                            let held = match ck {
                                Kind::Map(mi) if self.map_pair(mi).1 == Kind::Tower => Kind::Tower,
                                _ => kind_of(&cur, *value),
                            };
                            Some((*collection, self.map_of(kind_of(&cur, *index), held)))
                        } else if kind_of(&cur, *value) == Kind::Rational
                            && matches!(self.elem_kind(ck), Kind::Int | Kind::BigInt)
                        {
                            Some((*collection, self.seq_kind_for(Kind::Rational)))
                        } else {
                            None
                        }
                    }
                    // A slice preserves the sequence kind; a seq-concat yields the (shared) element seq kind;
                    // set union/intersection yields the left set's kind.
                    Op::SliceOp {
                        dst, collection, ..
                    } => Some((*dst, kind_of(&cur, *collection))),
                    Op::SeqConcat { dst, lhs, rhs } => {
                        let lk = kind_of(&cur, *lhs);
                        Some((
                            *dst,
                            if lk == Kind::SeqUnknown {
                                kind_of(&cur, *rhs)
                            } else {
                                lk
                            },
                        ))
                    }
                    Op::UnionOp { dst, lhs, .. } | Op::IntersectOp { dst, lhs, .. } => {
                        Some((*dst, kind_of(&cur, *lhs)))
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
                        let ck = kind_of(&cur, *collection);
                        // Positional tuple access: the element kind is the interned position at the constant
                        // index. A sequence yields its homogeneous element kind as before.
                        let k = if let Kind::Tuple(ti) = ck {
                            // A constant LOGOS index (1-based, or negative from the end) names one position of
                            // the interned layout; any index into a tuple whose positions agree names that kind.
                            let poss = self.tuple_positions(ti);
                            let at = self.const_int_at(start, pc, *index).and_then(|i| match i {
                                i if i >= 1 => Some((i - 1) as usize),
                                i if i <= -1 => poss.len().checked_sub(i.unsigned_abs() as usize),
                                _ => None,
                            });
                            match at.and_then(|p| poss.get(p).copied()) {
                                Some(k) => k,
                                None if !poss.is_empty() && poss.iter().all(|k| *k == poss[0]) => {
                                    poss[0]
                                }
                                None => Kind::Unknown,
                            }
                        } else if ck == Kind::Text {
                            // `item i of text` is the i-th character, itself a Text.
                            Kind::Text
                        } else if let Some((_, v)) = self.resolve_map_kv(ck) {
                            // `item k of m` yields the map's value kind (any kind — it rode the i64/str cell).
                            v
                        } else {
                            self.elem_kind(ck)
                        };
                        Some((*dst, k))
                    }
                    Op::Call {
                        dst,
                        func,
                        args_start,
                        ..
                    } => {
                        // A `mutable` parameter is the caller's own collection, passed by reference: after the
                        // call the argument register holds what the callee made of it — its declared kind.
                        if let Some(f) = self.prog.functions.get(*func as usize) {
                            for &p in &f.mutable_param_regs {
                                let pk = self
                                    .param_kinds
                                    .get(*func as usize)
                                    .and_then(|ks| ks.get(p as usize))
                                    .copied();
                                if let Some(pk) = pk.filter(|k| *k != Kind::Unknown) {
                                    set(&mut cur, *args_start + p, pk);
                                }
                            }
                        }
                        Some((
                            *dst,
                            self.fn_ret
                                .get(*func as usize)
                                .copied()
                                .unwrap_or(Kind::Unknown),
                        ))
                    }
                    // A closure value is an opaque heap pointer riding the i64 slot (like a struct/text pointer).
                    Op::MakeClosure { dst, .. } => Some((*dst, Kind::Int)),
                    // Calling a closure yields the join of every closure body's return kind (the dispatch targets).
                    Op::CallValue { dst, .. } => {
                        let k = self
                            .closure_bodies
                            .iter()
                            .map(|&b| self.fn_ret.get(b).copied().unwrap_or(Kind::Unknown))
                            .reduce(join)
                            .unwrap_or(Kind::Unknown);
                        Some((*dst, k))
                    }
                    // Scalar math/text builtins: Sqrt/ParseFloat→Float, Floor/Ceil/Round/ParseInt→Int (they
                    // land an i64), Abs→its argument's kind, Min/Max→the numeric join of the two args, Chr/Format→Text.
                    // Any other builtin stays Unknown here and is refused in `lower_op`.
                    Op::CallBuiltin {
                        dst,
                        builtin,
                        args_start,
                        ..
                    } => {
                        use crate::semantics::builtins::BuiltinId;
                        let a = kind_of(&cur, *args_start);
                        let k = match builtin {
                            BuiltinId::Sqrt | BuiltinId::ParseFloat => Kind::Float,
                            BuiltinId::Floor
                            | BuiltinId::Ceil
                            | BuiltinId::Round
                            | BuiltinId::ParseInt => Kind::Int,
                            BuiltinId::Abs => a,
                            BuiltinId::Min | BuiltinId::Max => {
                                join(a, kind_of(&cur, *args_start + 1))
                            }
                            BuiltinId::Chr | BuiltinId::Format | BuiltinId::TextFromBytes => {
                                Kind::Text
                            }
                            BuiltinId::TextBytes => Kind::SeqInt,
                            BuiltinId::ReadReals => Kind::SeqFloat,
                            BuiltinId::Contains => Kind::Bool,
                            BuiltinId::CountOnes
                            | BuiltinId::LeadingZeros
                            | BuiltinId::TrailingZeros => Kind::Int,
                            BuiltinId::Word8Make => Kind::Word(8),
                            BuiltinId::Word16Make => Kind::Word(16),
                            BuiltinId::Word32 | BuiltinId::Word32Shr => Kind::Word(32),
                            BuiltinId::Word64
                            | BuiltinId::Word64Shl
                            | BuiltinId::Word64Shr
                            | BuiltinId::Word64And => Kind::Word(64),
                            BuiltinId::IntOfWord8
                            | BuiltinId::IntOfWord16
                            | BuiltinId::IntOfWord32
                            | BuiltinId::IntOfWord64 => Kind::Int,
                            BuiltinId::Rotl | BuiltinId::Rotr | BuiltinId::Wnot => a,
                            BuiltinId::Wand | BuiltinId::Wor => a,
                            BuiltinId::TryReadBytes => self.inductive_kind(
                                "Result",
                                &[("Ok", vec![Kind::SeqInt]), ("Err", vec![Kind::Text])],
                            ),
                            BuiltinId::VfsReadFile | BuiltinId::VfsMount => Kind::Text,
                            BuiltinId::VfsWriteFile => Kind::Nothing,
                            BuiltinId::RandomInt => Kind::Int,
                            BuiltinId::RandomFloat => Kind::Float,
                            BuiltinId::EnvHas => Kind::Bool,
                            BuiltinId::EnvValue => Kind::Text,
                            BuiltinId::TimeNow => Kind::Int,
                            _ => Kind::Unknown,
                        };
                        Some((*dst, k))
                    }
                    // A tuple's layout is the kinds of its source registers at this pc; intern them so the
                    // `Kind::Tuple(idx)` stays `Copy` while the per-position kinds remain reachable.
                    Op::NewTuple { dst, start, count } => {
                        let ks: Vec<Kind> =
                            (0..*count).map(|i| kind_of(&cur, *start + i)).collect();
                        Some((*dst, Kind::Tuple(self.intern_tuple(ks))))
                    }
                    // Destructure writes MULTIPLE registers (one per position) — set each from the tuple's
                    // interned layout directly (like `RevSwap`), returning `None` for the single-write path.
                    Op::DestructureTuple { src, start, count } => {
                        if let Kind::Tuple(ti) = kind_of(&cur, *src) {
                            let poss = self.tuple_positions(ti);
                            for i in 0..*count {
                                let k = poss.get(i as usize).copied().unwrap_or(Kind::Unknown);
                                let idx = (*start + i) as usize;
                                if idx < cur.len() {
                                    cur[idx] = k;
                                }
                            }
                        }
                        None
                    }
                    Op::NewStruct { dst, type_name } => {
                        Some((*dst, self.struct_kind_of_type_name(*type_name)))
                    }
                    // A counter CRDT (`ConvergentCount`/`Tally`, kind 4/5) is a `logos_crdt*` handle that a
                    // holding field SUMS — its value kind is `CrdtCounter` (which, flowing into a `StructInsert`,
                    // marks that field a counter field). A sequence CRDT (`SharedSequence`/RGA, kind 1) IS a plain
                    // seq handle on the single replica — an empty `SeqUnknown` refined by a later `CrdtAppend`
                    // element. Other kinds are refused at lowering. Mirrors `codegen_riscv`'s `NewCrdt` kind.
                    Op::NewCrdt { dst, kind } => Some((
                        *dst,
                        match kind {
                            4 | 5 => Kind::CrdtCounter,
                            1 => Kind::SeqUnknown,
                            _ => Kind::Unknown,
                        },
                    )),
                    // A field write on a SYNTHESIZED struct (a `Tally` whose declared type didn't resolve) has no
                    // declared field type, so refine the field's kind from the value flowed into its cell — a
                    // later `GetField`/`Show` then renders it. (A resolved `Struct` has its declared field kinds,
                    // so this is a no-op there.) The write itself is a heap-cell store — no register write here.
                    Op::StructInsert { obj, field, value } => {
                        if let Kind::StructSynth(idx) = kind_of(&cur, *obj) {
                            self.synth_refine_field_kind(idx, *field, kind_of(&cur, *value));
                        }
                        None
                    }
                    // An append to a shared sequence gives it the appended element's kind.
                    Op::CrdtAppend { seq, value } => {
                        Some((*seq, self.seq_kind_for(kind_of(&cur, *value))))
                    }
                    // `CrdtBump` on a synth field's `Nothing`/`Int` cell keeps it an `Int` (the VM's
                    // `crdt_counter_bump(Nothing|Int, n) = Int(..)`), so refine the field kind to `Int`.
                    Op::CrdtBump { obj, field, .. } => {
                        if let Kind::StructSynth(idx) = kind_of(&cur, *obj) {
                            self.synth_refine_field_kind(idx, *field, Kind::Int);
                        }
                        None
                    }
                    Op::GetField { dst, obj, field } => {
                        let ok = kind_of(&cur, *obj);
                        // A counter field's declared cell type is `Int`; reading it SUMS the per-replica map to an
                        // `Int` (matching the VM's `Show c's points`), so its register kind is `Int`. A plain (or
                        // synthesized) field takes its resolved declaration kind.
                        let is_counter = self
                            .struct_name_of_kind(ok)
                            .zip(self.resolve_field_any(ok, *field))
                            .map(|(n, (cell, _))| self.counter_fields.contains(&(n, cell)))
                            .unwrap_or(false);
                        let k = if is_counter {
                            Kind::Int
                        } else {
                            self.resolve_field_any(ok, *field)
                                .map(|(_, k)| k)
                                .unwrap_or(Kind::Unknown)
                        };
                        Some((*dst, k))
                    }
                    Op::NewInductive {
                        dst,
                        type_name,
                        ctor,
                        args_start,
                        count,
                    } => {
                        let k = match self.enum_index_of_type_name(*type_name) {
                            Some(ei) => Kind::Enum(ei),
                            None => match (self.const_text(*type_name), self.const_text(*ctor)) {
                                (Some(ty), Some(c)) => {
                                    let payload: Vec<Kind> = (0..*count)
                                        .map(|i| kind_of(&cur, *args_start + i))
                                        .collect();
                                    self.inductive_kind(ty, &[(c, payload)])
                                }
                                _ => Kind::Unknown,
                            },
                        };
                        Some((*dst, k))
                    }
                    Op::TestArm {
                        dst,
                        target,
                        variant,
                    } => {
                        match kind_of(&cur, *target) {
                            Kind::Enum(ei) => {
                                if let Some(vi) = self.variant_index(ei, *variant) {
                                    last_test = Some((*target, ei, vi));
                                }
                            }
                            Kind::Inductive(_) => {
                                if let Some(c) = self.const_text(*variant) {
                                    last_test_ctor = Some((*target, c.to_string()));
                                }
                            }
                            _ => {}
                        }
                        Some((*dst, Kind::Bool))
                    }
                    Op::BindArm {
                        dst, target, index, ..
                    } => {
                        let k = match (kind_of(&cur, *target), &last_test, &last_test_ctor) {
                            (Kind::Enum(_), Some((t, ei, vi)), _) if t == target => {
                                self.variant_field_kind(*ei, *vi, *index as usize)
                            }
                            (Kind::Inductive(ii), _, Some((t, c))) if t == target => {
                                self.inductive_payload(ii, c, *index as usize)
                            }
                            _ => Kind::Unknown,
                        };
                        Some((*dst, k))
                    }
                    // Reversible-arithmetic places are Int; a reversible swap exchanges two slots' kinds.
                    Op::RevXor { place, .. }
                    | Op::RevNeg { place }
                    | Op::RevAdd { place, .. }
                    | Op::RevSub { place, .. } => Some((*place, Kind::Int)),
                    Op::RevSwap { a, b } => {
                        let (ai, bi) = (*a as usize, *b as usize);
                        if ai < cur.len() && bi < cur.len() {
                            cur.swap(ai, bi);
                            // Held exact, a side the program can make outgrow i64 holds the word it receives tagged.
                            if !self.on_words.get() {
                                for r in [*a, *b] {
                                    if self.exact_outs.contains(&(pc, r))
                                        && matches!(cur[r as usize], Kind::Int | Kind::Bool)
                                    {
                                        cur[r as usize] = Kind::BigInt;
                                    }
                                }
                            }
                        }
                        None
                    }
                    Op::DeepClone { dst, src } => Some((*dst, kind_of(&cur, *src))),
                    Op::GlobalGet { dst, idx } => Some((
                        *dst,
                        self.global_kinds
                            .get(*idx as usize)
                            .copied()
                            .unwrap_or(Kind::Unknown),
                    )),
                    Op::IterPrepare { iterable } => {
                        let ik = kind_of(&cur, *iterable);
                        // A Text iterates its characters, each itself a Text; a map its entries, each a
                        // `(key, value)` tuple; a set and a sequence their elements.
                        iter_elems.push(match ik {
                            Kind::Text => Kind::Text,
                            Kind::Map(_) | Kind::MapUnknown => {
                                let (kk, vk) = self.resolve_map_kv(ik).unwrap();
                                Kind::Tuple(self.intern_tuple(vec![kk, vk]))
                            }
                            other => self.elem_kind(other),
                        });
                        None
                    }
                    Op::Args { dst } => Some((*dst, Kind::SeqText)),
                    Op::LoadToday { dst } => Some((*dst, Kind::Date)),
                    Op::LoadNow { dst } => Some((*dst, Kind::Moment)),
                    Op::IterNext { dst, .. } => {
                        Some((*dst, iter_elems.last().copied().unwrap_or(Kind::Unknown)))
                    }
                    Op::IterPop => {
                        iter_elems.pop();
                        None
                    }
                    // A peer handle IS its address string; `Sync` keeps `dst`'s current value (offline identity —
                    // NO write, so the prior kind stands); `Await` yields the received 64-bit message (an Int).
                    // Other CRDT/net ops mutate a struct cell / topic FIFO, not a register — no write here.
                    Op::NetMakePeer { dst, .. } => Some((*dst, Kind::Peer)),
                    Op::NetAwait { dst, stream, .. } => Some((
                        *dst,
                        if *stream {
                            self.net_stream.get()
                        } else {
                            self.net_message.get()
                        },
                    )),
                    // A `Tcp` op yields a LOGOS `Result` ([`Emitter::tcp_result_kind`]).
                    Op::Tcp { dst, kind, .. } => Some((*dst, self.tcp_result_kind(*kind))),
                    _ => None,
                };
                // An operation the VM refuses for its operands' kinds — words of two widths, a word beside another
                // number, the remainder of a Real, an approximate comparison of anything but numbers — has no
                // result: its register holds a value that cannot exist.
                let write = match (&self.prog.code[pc], write) {
                    (op, Some((dst, _))) if refused_for_kinds(op, &cur) => Some((dst, Kind::Never)),
                    (_, w) => w,
                };
                // On machine words an exact value arrives as the machine word it is checked to fit.
                let write = match write {
                    Some((dst, Kind::BigInt))
                        if self.on_words.get() && self.arrives_exact(&self.prog.code[pc], &cur) =>
                    {
                        Some((dst, Kind::Int))
                    }
                    w => w,
                };
                // An integer the program can make outgrow i64 is held exact ([`Emitter::exactness`]) — on machine
                // words, only one the exact tower computes; `promoted` records, as of this inference, whether the op
                // itself makes a machine integer.
                let write = match write {
                    Some((dst, k @ (Kind::Int | Kind::BigInt))) if self.held_exact(pc) => {
                        let version = self.on_words.get() as usize;
                        if k == Kind::Int {
                            self.promoted.borrow_mut()[version].insert(pc);
                        } else {
                            self.promoted.borrow_mut()[version].remove(&pc);
                        }
                        Some((dst, Kind::BigInt))
                    }
                    w => w,
                };
                // A value of the extension computed from the kinds read here, and a definition of an exact web,
                // which makes a cell whatever it computes (`tower_from` keeps what it does compute).
                let write = match self.tower.then(|| self.tower_write(&self.prog.code[pc], &cur)).flatten() {
                    Some(w) => Some(w),
                    None => write,
                };
                let write = match write {
                    Some((dst, k)) if forced.defs.contains(&pc) => {
                        let version = self.on_words.get() as usize;
                        if k == Kind::Tower {
                            self.tower_from.borrow_mut()[version].remove(&pc);
                        } else {
                            self.tower_from.borrow_mut()[version].insert(pc, k);
                        }
                        Some((dst, Kind::Tower))
                    }
                    w => w,
                };
                // A definition a number join lands computes its own kind and lands as its variable's.
                let write = {
                    let version = self.on_words.get() as usize;
                    match (write, lands.get(&pc)) {
                        (Some((dst, k)), Some(&to)) if k != Kind::Tower && k != to => {
                            self.number_from.borrow_mut()[version].insert(pc, (k, to));
                            Some((dst, to))
                        }
                        (w, _) => {
                            self.number_from.borrow_mut()[version].remove(&pc);
                            w
                        }
                    }
                };
                for r in &narrowed {
                    set(&mut cur, *r, Kind::BigInt);
                }
                if let Some((dst, k)) = write {
                    // What the program declares the value to be (`a new Seq of Text`, `Let x: Seq of Int be …`)
                    // types it wherever nothing it was built from does.
                    let k = match k {
                        Kind::Unknown | Kind::SeqUnknown | Kind::SetUnknown | Kind::MapUnknown => {
                            match self.declared_kind_at(pc) {
                                Some(d) if d != Kind::Unknown => d,
                                _ => k,
                            }
                        }
                        other => other,
                    };
                    // A variable holding an undeclared enum keeps what is known of its payloads across its
                    // assignments (`none` then `some x`), as long as the two agree.
                    let k = match (kind_of(&cur, dst), k) {
                        (Kind::Inductive(a), Kind::Inductive(b)) => {
                            self.unify_inductive(a, b).unwrap_or(k)
                        }
                        _ => k,
                    };
                    // Upgrade a freshly-created empty sequence to the concrete kind it reached last pass
                    // (monotone — a concrete kind is never downgraded).
                    let ov = kind_of(&overrides, dst);
                    let k = match k {
                        Kind::SeqUnknown if is_concrete_seq(ov) => ov,
                        Kind::SetUnknown if is_concrete_set(ov) => ov,
                        Kind::MapUnknown if is_concrete_map(ov) => ov,
                        other => other,
                    };
                    set(&mut cur, dst, k);
                }
            }
            per_pc.push(cur.clone()); // state AFTER the final op
            if cur == overrides {
                break;
            }
            overrides = cur;
        }
        per_pc
    }

    fn fresh(&mut self) -> usize {
        let n = self.tmp;
        self.tmp += 1;
        n
    }

    /// Resolve a declared boundary type to a register kind (a `Struct` by name → `Kind::Struct(idx)`; scalars
    /// and scalar-element sequences fall through to the free mapping).
    fn boundary_kind(&self, ty: &Option<BoundaryType>) -> Kind {
        match ty {
            Some(BoundaryType::Struct(name)) => self
                .prog
                .struct_types
                .iter()
                .position(|s| &s.name == name)
                .map(|i| Kind::Struct(i as u16))
                .unwrap_or(Kind::Unknown),
            Some(BoundaryType::Enum(name)) => self
                .prog
                .enum_types
                .iter()
                .position(|e| &e.name == name)
                .map(|i| Kind::Enum(i as u16))
                .unwrap_or(Kind::Unknown),
            // A declared tuple return/param type `(Int, Text)` → an interned `Kind::Tuple`, so a function that
            // returns a tuple gives its `Call` dst (and a following `DestructureTuple`) the right layout.
            Some(BoundaryType::Tuple(elems)) => {
                let ks: Vec<Kind> = elems
                    .iter()
                    .map(|e| self.boundary_kind(&Some(e.clone())))
                    .collect();
                Kind::Tuple(self.intern_tuple(ks))
            }
            // `Seq of T` — a sequence of scalars, or of any other declared kind as a `SeqOf`; a list of exact values
            // is one exact value, the runtime's list.
            Some(BoundaryType::Seq(elem)) => match self.boundary_kind(&Some((**elem).clone())) {
                Kind::Tower => Kind::Tower,
                k => self.seq_kind_for(k),
            },
            // `Option of T` / `Result of T and E` — the undeclared enums, with their payloads' declared kinds.
            Some(BoundaryType::Option(inner)) => {
                let inner = self.boundary_kind(&Some((**inner).clone()));
                self.inductive_kind("Option", &[("Some", vec![inner]), ("None", Vec::new())])
            }
            Some(BoundaryType::Result(ok, err)) => {
                let (ok, err) = (
                    self.boundary_kind(&Some((**ok).clone())),
                    self.boundary_kind(&Some((**err).clone())),
                );
                self.inductive_kind("Result", &[("Ok", vec![ok]), ("Err", vec![err])])
            }
            // A declared `Map from K to V` → the interned map kind (or a refusal-forcing `Unknown` for an
            // unhashable key). The value may be any declared kind.
            Some(BoundaryType::Map(k, v)) => self.map_of(
                self.boundary_kind(&Some((**k).clone())),
                self.boundary_kind(&Some((**v).clone())),
            ),
            // A declared type of the extension (`Hyperreal`, `Algebraic`, …) holds the runtime's value.
            Some(BoundaryType::Builtin(name)) if super::tower::TYPE_NAMES.contains(&name.as_str()) => Kind::Tower,
            other => boundary_to_kind_c(other),
        }
    }

    /// The kind the program declares for the value the op at `pc` defines (`declared_types`, sorted by pc).
    fn declared_kind_at(&self, pc: usize) -> Option<Kind> {
        let at = self
            .prog
            .declared_types
            .binary_search_by_key(&(pc as u32), |(p, _)| *p)
            .ok()?;
        Some(self.boundary_kind(&Some(self.prog.declared_types[at].1.clone())))
    }

    /// The `Result` a native `Tcp` op answers with (`TcpOp::code()`): `Ok` carries the handle or byte count
    /// (Int), the received bytes (Seq of Int) or `nothing`; `Err` carries the failure's Text.
    fn tcp_result_kind(&self, code: u8) -> Kind {
        let ok = match code {
            0 | 1 | 6 | 7 | 8 => Kind::Int,
            2 | 3 => Kind::SeqInt,
            4 | 5 | 9 => Kind::Nothing,
            _ => Kind::Unknown,
        };
        self.inductive_kind("Result", &[("Ok", vec![ok]), ("Err", vec![Kind::Text])])
    }

    /// The `prog.enum_types` index a `NewInductive` type-name constant (a `Text`) refers to.
    fn enum_index_of_type_name(&self, type_name: u32) -> Option<u16> {
        let ename = match self.prog.constants.get(type_name as usize) {
            Some(Constant::Text(s)) => s,
            _ => return None,
        };
        self.prog
            .enum_types
            .iter()
            .position(|e| &e.name == ename)
            .map(|i| i as u16)
    }

    /// The declaration-order index of a variant (a `Text` constant) within an enum type.
    fn variant_index(&self, enum_idx: u16, variant: u32) -> Option<usize> {
        let ed = self.prog.enum_types.get(enum_idx as usize)?;
        let vname = match self.prog.constants.get(variant as usize) {
            Some(Constant::Text(s)) => s.as_str(),
            _ => return None,
        };
        ed.variants.iter().position(|v| v.name == vname)
    }

    /// The `Kind` of payload field `index` of a specific enum variant.
    fn variant_field_kind(&self, enum_idx: u16, variant_idx: usize, index: usize) -> Kind {
        self.prog
            .enum_types
            .get(enum_idx as usize)
            .and_then(|e| e.variants.get(variant_idx))
            .and_then(|v| v.field_types.get(index))
            .map(|t| self.boundary_kind(&Some(t.clone())))
            .unwrap_or(Kind::Unknown)
    }

    /// The `prog.struct_types` index a `NewStruct` type-name constant (a `Text`) refers to.
    fn struct_index_of_type_name(&self, type_name: u32) -> Option<u16> {
        let sname = match self.prog.constants.get(type_name as usize) {
            Some(Constant::Text(s)) => s,
            _ => return None,
        };
        self.prog
            .struct_types
            .iter()
            .position(|s| &s.name == sname)
            .map(|i| i as u16)
    }

    /// The Text value of a constant slot (`None` for a non-Text constant).
    fn const_text(&self, idx: u32) -> Option<&str> {
        match self.prog.constants.get(idx as usize) {
            Some(Constant::Text(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Intern a synthesized struct layout by NAME (dedup), returning the `u16` a `Kind::StructSynth` carries.
    fn synth_index_of_name(&self, name: &str) -> u16 {
        let mut t = self.synth_structs.borrow_mut();
        if let Some(i) = t.iter().position(|(n, _)| n == name) {
            return i as u16;
        }
        t.push((name.to_string(), Vec::new()));
        (t.len() - 1) as u16
    }

    /// The struct kind a `NewStruct` type-name constant produces: a `Kind::Struct` when the type resolved into
    /// `prog.struct_types`, else a `Kind::StructSynth` over a synthesized layout (interned by name). `Unknown`
    /// only for a non-Text type-name constant.
    fn struct_kind_of_type_name(&self, type_name: u32) -> Kind {
        if let Some(si) = self.struct_index_of_type_name(type_name) {
            return Kind::Struct(si);
        }
        match self.const_text(type_name) {
            Some(name) => Kind::StructSynth(self.synth_index_of_name(name)),
            None => Kind::Unknown,
        }
    }

    /// The struct NAME behind a `Struct`/`StructSynth` kind (used to key `counter_fields`).
    fn struct_name_of_kind(&self, k: Kind) -> Option<String> {
        match k {
            Kind::Struct(si) => self
                .prog
                .struct_types
                .get(si as usize)
                .map(|s| s.name.clone()),
            Kind::StructSynth(idx) => self
                .synth_structs
                .borrow()
                .get(idx as usize)
                .map(|(n, _)| n.clone()),
            _ => None,
        }
    }

    /// The cell index of a field-name constant within a SYNTHESIZED layout, appending it (growing the layout in
    /// first-seen order) when not yet present. This is how an unresolved struct's field→cell map is built from
    /// the field ops encountered on it.
    fn synth_field_cell(&self, idx: u16, field: u32) -> Option<usize> {
        let fname = self.const_text(field)?.to_string();
        let mut t = self.synth_structs.borrow_mut();
        let (_, fields) = t.get_mut(idx as usize)?;
        Some(match fields.iter().position(|(f, _)| f == &fname) {
            Some(cell) => cell,
            None => {
                fields.push((fname, Kind::Unknown));
                fields.len() - 1
            }
        })
    }

    /// Monotonically refine the KIND of a synthesized struct's field (the value flowed into its cell): a concrete
    /// kind is never downgraded to `Unknown`, and a later concrete kind overrides an earlier one (the last write
    /// establishes the readable value — the LWW cell).
    fn synth_refine_field_kind(&self, idx: u16, field: u32, k: Kind) {
        if k == Kind::Unknown {
            return;
        }
        let cell = match self.synth_field_cell(idx, field) {
            Some(c) => c,
            None => return,
        };
        let mut t = self.synth_structs.borrow_mut();
        if let Some((_, fields)) = t.get_mut(idx as usize) {
            if let Some((_, fk)) = fields.get_mut(cell) {
                *fk = k;
            }
        }
    }

    /// The number of cells a `Struct`/`StructSynth` lays out (its field count).
    fn struct_cell_count(&self, k: Kind) -> Option<usize> {
        match k {
            Kind::Struct(si) => self
                .prog
                .struct_types
                .get(si as usize)
                .map(|s| s.fields.len()),
            Kind::StructSynth(idx) => self
                .synth_structs
                .borrow()
                .get(idx as usize)
                .map(|(_, f)| f.len()),
            _ => None,
        }
    }

    /// Heap fields have a stable representation across every store and read.
    /// Integer cells use exact tagged words; counter cells retain their CRDT handles.
    fn field_storage_kind(&self, obj: Kind, cell: usize) -> Option<Kind> {
        if self
            .struct_name_of_kind(obj)
            .is_some_and(|name| self.counter_fields.contains(&(name, cell)))
        {
            return Some(Kind::CrdtCounter);
        }
        let kind = match obj {
            Kind::Struct(si) => {
                let (_, ty) = self.prog.struct_types.get(si as usize)?.fields.get(cell)?;
                self.boundary_kind(&Some(ty.clone()))
            }
            Kind::StructSynth(si) => self.synth_structs.borrow().get(si as usize)?.1.get(cell)?.1,
            _ => return None,
        };
        Some(if kind == Kind::Int {
            Kind::BigInt
        } else {
            kind
        })
    }

    fn field_store_value(
        &mut self,
        obj: Kind,
        cell: usize,
        value: u16,
        kind: Kind,
    ) -> Result<String, String> {
        if self.field_storage_kind(obj, cell) == Some(Kind::Tower) && kind != Kind::Tower {
            let converted = self.tower_of(&format!("r[{value}]"), kind)?;
            return Ok(format!("(int64_t)(intptr_t)({converted})"));
        }
        // A rational field holds an integer as the rational it equals. The language holds nothing in one a record is
        // built without, which a compiled rational cannot hold.
        if self.field_storage_kind(obj, cell) == Some(Kind::Rational) && kind != Kind::Rational {
            if kind == Kind::Nothing {
                return Err(
                    "a record's rational field holding nothing: a compiled rational holds no nothing".to_string(),
                );
            }
            return self.convert_number(&format!("r[{value}]"), kind, Kind::Rational);
        }
        if self.field_storage_kind(obj, cell) == Some(Kind::BigInt) {
            // Move-out clears and uninitialized CRDT fields carry the empty
            // cell, which a later store or bump replaces.
            if kind == Kind::Nothing {
                return Ok("0".to_string());
            }
            as_exact(value, kind)
        } else {
            Ok(format!("r[{value}]"))
        }
    }

    /// Resolve a field on ANY struct kind (`Struct` or synthesized `StructSynth`) to its cell index and `Kind`.
    /// A `Struct`'s field kind is its declared type; a synthesized field's kind is the (refined) kind of the
    /// value flowed into its cell (`Unknown` until a `StructInsert`/`CrdtBump` establishes it). The counter
    /// side-set drives the counter-vs-plain decision separately.
    fn resolve_field_any(&self, obj_kind: Kind, field: u32) -> Option<(usize, Kind)> {
        match obj_kind {
            Kind::Struct(si) => self.resolve_field(si, field),
            Kind::StructSynth(idx) => {
                let cell = self.synth_field_cell(idx, field)?;
                let fk = self
                    .synth_structs
                    .borrow()
                    .get(idx as usize)
                    .and_then(|(_, f)| f.get(cell))
                    .map(|(_, k)| *k)
                    .unwrap_or(Kind::Unknown);
                Some((cell, self.field_storage_kind(obj_kind, cell).unwrap_or(fk)))
            }
            _ => None,
        }
    }

    /// Resolve a field (by the object's `Struct` kind + a field-name constant) to its declaration-order cell
    /// index and its `Kind`.
    fn resolve_field(&self, struct_idx: u16, field: u32) -> Option<(usize, Kind)> {
        let sd = self.prog.struct_types.get(struct_idx as usize)?;
        let fname = match self.prog.constants.get(field as usize) {
            Some(Constant::Text(s)) => s.clone(),
            _ => return None,
        };
        let idx = sd.fields.iter().position(|(n, _)| *n == fname)?;
        Some((idx, self.field_storage_kind(Kind::Struct(struct_idx), idx)?))
    }

    /// Resolve a field by struct type NAME + field NAME (policy conditions carry names, not `ConstIdx`) to its
    /// declaration-order cell index and declared `Kind`. Only real `prog.struct_types` are consulted — a policy
    /// always names a declared type.
    fn field_cell_by_name(&self, type_name: &str, field_name: &str) -> Option<(usize, Kind)> {
        let sd = self
            .prog
            .struct_types
            .iter()
            .position(|s| s.name == type_name)?;
        let struct_idx = sd as u16;
        let sd = &self.prog.struct_types[sd];
        let idx = sd.fields.iter().position(|(n, _)| n == field_name)?;
        Some((idx, self.field_storage_kind(Kind::Struct(struct_idx), idx)?))
    }

    /// A C boolean EXPRESSION (0/1) for the policy `cond` evaluated against the `subject` struct's field cells
    /// (and the `object` struct for capability conditions). Mirrors `semantics::policy::evaluate_policy_condition`
    /// and `codegen_riscv::emit_policy_condition` exactly, but as a composable C expression instead of asm.
    fn policy_condition_expr(
        &self,
        cond: &PolicyCondition,
        subject: u16,
        subject_type: &str,
        object: u16,
        object_type: Option<&str>,
        interner: &Interner,
    ) -> Result<String, String> {
        let cell_c = |reg: u16, cell: usize| format!("((int64_t*)(intptr_t)r[{reg}])[{cell}]");
        match cond {
            PolicyCondition::FieldEquals {
                field,
                value,
                is_string_literal,
            } => {
                let fname = interner.resolve(*field);
                let vstr = interner.resolve(*value);
                let (cell, fk) = self
                    .field_cell_by_name(subject_type, fname)
                    .ok_or("policy references an unknown field")?;
                let lv = cell_c(subject, cell);
                Ok(match fk {
                    Kind::Text => format!(
                        "(strcmp((const char*)(intptr_t){lv}, \"{}\")==0)",
                        c_escape(vstr)
                    ),
                    Kind::BigInt if !*is_string_literal => match vstr.parse::<i64>() {
                        Ok(n) => format!("(lc_xcmp({lv}, lc_xof({n}LL)) == 0)"),
                        Err(_) => "0".to_string(),
                    },
                    Kind::Bool if !*is_string_literal => match vstr.parse::<bool>() {
                        Ok(b) => format!("({lv} == {})", b as i64),
                        Err(_) => "0".to_string(),
                    },
                    _ => "0".to_string(), // string-literal vs Int/Bool field, or an unmodeled kind
                })
            }
            PolicyCondition::FieldBool { field, value } => {
                let fname = interner.resolve(*field);
                let (cell, _) = self
                    .field_cell_by_name(subject_type, fname)
                    .ok_or("policy references an unknown field")?;
                Ok(format!("({} == {})", cell_c(subject, cell), *value as i64))
            }
            PolicyCondition::Predicate { predicate, .. } => {
                let sub = self.policy_ctx.and_then(|(_, policies)| {
                    interner.lookup(subject_type).and_then(|ts| {
                        policies.get_predicates(ts).and_then(|ps| {
                            ps.iter()
                                .find(|p| p.predicate_name == *predicate)
                                .map(|p| p.condition.clone())
                        })
                    })
                });
                match sub {
                    Some(c) => self.policy_condition_expr(
                        &c,
                        subject,
                        subject_type,
                        object,
                        object_type,
                        interner,
                    ),
                    None => Ok("0".to_string()),
                }
            }
            PolicyCondition::ObjectFieldEquals {
                subject: subj_field,
                field,
                ..
            } => self.field_equality_expr(
                subject,
                subject_type,
                *subj_field,
                object,
                object_type,
                *field,
                interner,
            ),
            PolicyCondition::SubjectFieldEqualsObjectField {
                subject_field,
                object_field,
                ..
            } => self.field_equality_expr(
                subject,
                subject_type,
                *subject_field,
                object,
                object_type,
                *object_field,
                interner,
            ),
            PolicyCondition::Or(l, r) => {
                let le = self.policy_condition_expr(
                    l,
                    subject,
                    subject_type,
                    object,
                    object_type,
                    interner,
                )?;
                let re = self.policy_condition_expr(
                    r,
                    subject,
                    subject_type,
                    object,
                    object_type,
                    interner,
                )?;
                Ok(format!("(({le}) || ({re}))"))
            }
            PolicyCondition::And(l, r) => {
                let le = self.policy_condition_expr(
                    l,
                    subject,
                    subject_type,
                    object,
                    object_type,
                    interner,
                )?;
                let re = self.policy_condition_expr(
                    r,
                    subject,
                    subject_type,
                    object,
                    object_type,
                    interner,
                )?;
                Ok(format!("(({le}) && ({re}))"))
            }
        }
    }

    /// `subject.<subj_field> == object.<obj_field>` as a C boolean expression (the VM's `values_equal`); a
    /// missing object (a predicate context, not a capability) is `0`. Text compares with `strcmp`, else `==`.
    fn field_equality_expr(
        &self,
        subject: u16,
        subject_type: &str,
        subj_field: crate::intern::Symbol,
        object: u16,
        object_type: Option<&str>,
        obj_field: crate::intern::Symbol,
        interner: &Interner,
    ) -> Result<String, String> {
        let obj_type = match (object_type, object != u16::MAX) {
            (Some(t), true) => t,
            _ => return Ok("0".to_string()),
        };
        let sf = interner.resolve(subj_field);
        let of = interner.resolve(obj_field);
        let (sc, sk) = self
            .field_cell_by_name(subject_type, sf)
            .ok_or("policy references an unknown subject field")?;
        let (oc, _) = self
            .field_cell_by_name(obj_type, of)
            .ok_or("policy references an unknown object field")?;
        let sl = format!("((int64_t*)(intptr_t)r[{subject}])[{sc}]");
        let ol = format!("((int64_t*)(intptr_t)r[{object}])[{oc}]");
        Ok(match sk {
            Kind::Text => {
                format!("(strcmp((const char*)(intptr_t){sl}, (const char*)(intptr_t){ol})==0)")
            }
            _ => format!("({sl} == {ol})"),
        })
    }

    /// The boxed sequence in register `reg`, cast to its typed `Seq_*` pointer.
    fn seq_ptr(reg: u16, seq_kind: Kind) -> Result<String, String> {
        let ty = seq_kind
            .seq_c_type()
            .ok_or_else(|| format!("sequence op on a non-sequence kind {seq_kind:?}"))?;
        Ok(format!("(({ty}*)(intptr_t)r[{reg}])"))
    }

    /// The boxed set in register `reg`, cast to its typed `Set_*` pointer.
    fn set_ptr(reg: u16, set_kind: Kind) -> Result<String, String> {
        let ty = set_kind
            .set_c_type()
            .ok_or_else(|| format!("set op on a non-set kind {set_kind:?}"))?;
        Ok(format!("(({ty}*)(intptr_t)r[{reg}])"))
    }

    /// Emit a deep copy of the collection in register `reg` into a fresh box, returning the `int64_t`-cast
    /// handle expression; `None` for an immutable scalar/Text (the caller uses `r[reg]` directly). The single
    /// choke point for value-semantics cloning (DeepClone, EnsureOwned COW, by-value call arguments).
    fn clone_collection_expr(&mut self, reg: u16, k: Kind) -> Result<Option<String>, String> {
        if k == Kind::Tower {
            return Ok(Some(format!("(int64_t)(intptr_t)ltower_own((LTower)(intptr_t)r[{reg}])")));
        }
        let k = match k {
            Kind::SeqUnknown => Kind::SeqInt,
            Kind::SetUnknown => Kind::SetInt,
            other => other,
        };
        if let (Some(ty), Some(infix)) = (k.seq_c_type(), k.seq_infix()) {
            let ptr = Self::seq_ptr(reg, k)?;
            let t = self.fresh();
            self.line(&format!(
                "{ty} *_cl{t} = ({ty}*)lc_gc_alloc(sizeof({ty})); *_cl{t} = seq_{infix}_copy({ptr});"
            ));
            Ok(Some(format!("(int64_t)(intptr_t)_cl{t}")))
        } else if is_concrete_set(k) {
            let (ty, infix) = (k.set_c_type().unwrap(), k.seq_infix().unwrap());
            let ptr = Self::set_ptr(reg, k)?;
            let t = self.fresh();
            self.line(&format!(
                "{ty} *_cl{t} = ({ty}*)lc_gc_alloc(sizeof({ty})); *_cl{t} = lc_set_{infix}_copy({ptr});"
            ));
            Ok(Some(format!("(int64_t)(intptr_t)_cl{t}")))
        } else if let Some((key, val)) = self.resolve_map_kv(k) {
            let (ty, pre) = (map_type_name(key, val).unwrap(), map_fn(key, val).unwrap());
            let t = self.fresh();
            self.line(&format!("{ty} *_cl{t} = ({ty}*)lc_gc_alloc(sizeof({ty})); *_cl{t} = lc_{pre}_copy(({ty}*)(intptr_t)r[{reg}]);"));
            Ok(Some(format!("(int64_t)(intptr_t)_cl{t}")))
        } else if let Some(n) = self.struct_cell_count(k) {
            // A record's copy shares its fields' collections until one of them changes: its field-ownership
            // word (the cell after the fields) starts clear, so the first change through the copy copies that
            // field's collection — the VM's deep copy, made lazily.
            let t = self.fresh();
            self.line(&format!(
                "int64_t *_cl{t} = (int64_t*)lc_gc_alloc({} * sizeof(int64_t)); memcpy(_cl{t}, (void*)(intptr_t)r[{reg}], {n} * sizeof(int64_t)); _cl{t}[{n}] = 0;",
                n + 1
            ));
            Ok(Some(format!("(int64_t)(intptr_t)_cl{t}")))
        } else {
            Ok(None)
        }
    }

    /// The C lvalue of the field-ownership word of the record in register `obj` (of struct kind `k`): bit `c` is
    /// set while the record is the only holder of the collection in field cell `c`.
    fn field_owner_word(&self, obj: u16, k: Kind) -> Option<String> {
        let n = self.struct_cell_count(k)?;
        Some(format!("((uint64_t*)(intptr_t)r[{obj}])[{n}]"))
    }

    /// A C expression yielding the display text (a `char*`) of the value `v` — an `int64_t` C expression — of
    /// kind `k`: the VM's `to_display_string`, recursively. A list shows its elements, a tuple its positions, a
    /// record its fields in name order, an enum its constructor and payloads, an `Option` its payload or
    /// `nothing` — each part shown by its own kind.
    fn render(&mut self, v: &str, k: Kind) -> Result<String, String> {
        let n = self.fresh();
        Ok(match k {
            Kind::Int => format!("logos_dyn_sprintf(\"%\" PRId64, (int64_t)({v}))"),
            Kind::Bool => format!("(({v}) ? \"true\" : \"false\")"),
            Kind::Float => format!("logos_fmt_f64(as_f64({v}))"),
            Kind::Text => format!("((const char*)(intptr_t)({v}))"),
            Kind::Rational => format!("ldec_to_string((LDec*)(intptr_t)({v}))"),
            Kind::BigInt => format!("lint_to_string(lc_xget({v}))"),
            Kind::Tower => format!("ltower_to_string((LTower)(intptr_t)({v}))"),
            Kind::Nothing => "\"nothing\"".to_string(),
            Kind::Peer => format!("logos_dyn_sprintf(\"<peer %s>\", (const char*)(intptr_t)({v}))"),
            Kind::Date => format!("logos_date_to_string((int64_t)({v}))"),
            Kind::Word(_) => format!("logos_dyn_sprintf(\"%\" PRIu64, (uint64_t)({v}))"),
            Kind::Moment => format!("logos_moment_to_string((int64_t)({v}))"),
            Kind::Time => format!("logos_format_time_of_day((int64_t)({v}))"),
            Kind::Span => format!(
                "logos_span_to_string((LogosSpan){{(int32_t)((int64_t)({v}) >> 32), (int32_t)(int64_t)({v})}})"
            ),
            Kind::CrdtCounter => {
                format!("logos_dyn_sprintf(\"%lld\", logos_crdt_sum((logos_crdt*)(intptr_t)({v})))")
            }
            Kind::SeqInt | Kind::SeqFloat | Kind::SeqText | Kind::SeqBool | Kind::SeqUnknown => {
                let sk = seq_kind_of(k);
                format!(
                    "seq_{}_to_string(({}*)(intptr_t)({v}))",
                    sk.seq_infix().unwrap(),
                    sk.seq_c_type().unwrap()
                )
            }
            Kind::SetWord(_) => format!("set_i64_to_string_as((Set_i64*)(intptr_t)({v}), 1)"),
            Kind::SetInt | Kind::SetText | Kind::SetUnknown => {
                let sk = resolve_set(k).unwrap();
                format!(
                    "set_{}_to_string(({}*)(intptr_t)({v}))",
                    sk.seq_infix().unwrap(),
                    sk.set_c_type().unwrap()
                )
            }
            Kind::Map(_) | Kind::MapUnknown => {
                let (kk, vk) = self.resolve_map_kv(k).unwrap();
                let ty = map_type_name(kk, vk).unwrap();
                let key = if kk == Kind::Text {
                    format!("_m{n}->keys[_si{n}]")
                } else {
                    format!("_m{n}->keys[_si{n}]")
                };
                let key_text = self.render(&format!("(int64_t)(intptr_t){key}"), kk)?;
                let val = map_val_load(&format!("_m{n}->vals[_si{n}]"), vk);
                let val_text = self.render(&val, vk)?;
                format!(
                    "({{ {ty} *_m{n} = ({ty}*)(intptr_t)({v}); lc_sb _b{n} = {{0}}; lc_sb_add(&_b{n}, \"{{\"); int _f{n} = 1; \
                     for (size_t _oi{n} = 0; _oi{n} < _m{n}->ord_len; _oi{n}++) {{ size_t _si{n} = _m{n}->order[_oi{n}]; if (!_m{n}->state[_si{n}]) continue; \
                     if (!_f{n}) lc_sb_add(&_b{n}, \", \"); _f{n} = 0; lc_sb_add(&_b{n}, {key_text}); lc_sb_add(&_b{n}, \": \"); lc_sb_add(&_b{n}, {val_text}); }} \
                     lc_sb_add(&_b{n}, \"}}\"); _b{n}.s; }})"
                )
            }
            Kind::Tuple(ti) => {
                let poss = self.tuple_positions(ti);
                let mut parts = String::new();
                for (i, pk) in poss.iter().enumerate() {
                    if i > 0 {
                        parts.push_str(&format!("lc_sb_add(&_b{n}, \", \"); "));
                    }
                    let cell = self.render(&format!("_t{n}[{i}]"), *pk)?;
                    parts.push_str(&format!("lc_sb_add(&_b{n}, {cell}); "));
                }
                format!("({{ int64_t *_t{n} = (int64_t*)(intptr_t)({v}); lc_sb _b{n} = {{0}}; lc_sb_add(&_b{n}, \"(\"); {parts}lc_sb_add(&_b{n}, \")\"); _b{n}.s; }})")
            }
            Kind::SeqOf(ei) => {
                let item = self.render(
                    &format!("_s{n}->data[_i{n}]"),
                    self.elem_kind(Kind::SeqOf(ei)),
                )?;
                format!(
                    "({{ Seq_i64 *_s{n} = (Seq_i64*)(intptr_t)({v}); lc_sb _b{n} = {{0}}; lc_sb_add(&_b{n}, \"[\"); \
                     for (size_t _i{n} = 0; _i{n} < _s{n}->len; _i{n}++) {{ if (_i{n}) lc_sb_add(&_b{n}, \", \"); lc_sb_add(&_b{n}, {item}); }} \
                     lc_sb_add(&_b{n}, \"]\"); _b{n}.s; }})"
                )
            }
            Kind::Struct(_) | Kind::StructSynth(_) => {
                let name = self.struct_name_of_kind(k).ok_or("a record with no name")?;
                let mut fields: Vec<(String, usize, Kind)> = match k {
                    Kind::Struct(si) => {
                        let sd = self.prog.struct_types[si as usize].clone();
                        sd.fields
                            .iter()
                            .enumerate()
                            .map(|(c, (f, _))| {
                                (f.clone(), c, self.field_storage_kind(k, c).unwrap())
                            })
                            .collect()
                    }
                    Kind::StructSynth(idx) => {
                        let fs = self.synth_structs.borrow()[idx as usize].1.clone();
                        fs.into_iter()
                            .enumerate()
                            .map(|(c, (f, _))| (f, c, self.field_storage_kind(k, c).unwrap()))
                            .collect()
                    }
                    _ => unreachable!(),
                };
                if fields.is_empty() {
                    return Ok(format!("\"{}\"", c_escape(&name)));
                }
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                let mut parts = String::new();
                for (i, (f, c, fk)) in fields.iter().enumerate() {
                    let is_counter = self.counter_fields.contains(&(name.clone(), *c));
                    let fk = if is_counter { Kind::CrdtCounter } else { *fk };
                    let cell = self.render(&format!("_r{n}[{c}]"), fk)?;
                    let label = format!("{}{}: ", if i > 0 { ", " } else { "" }, f);
                    parts.push_str(&format!(
                        "lc_sb_add(&_b{n}, \"{}\"); lc_sb_add(&_b{n}, {cell}); ",
                        c_escape(&label)
                    ));
                }
                format!(
                    "({{ int64_t *_r{n} = (int64_t*)(intptr_t)({v}); lc_sb _b{n} = {{0}}; lc_sb_add(&_b{n}, \"{} {{ \"); {parts}lc_sb_add(&_b{n}, \" }}\"); _b{n}.s; }})",
                    c_escape(&name)
                )
            }
            Kind::Enum(ei) => {
                let ed = self.prog.enum_types[ei as usize].clone();
                let arms: Vec<(String, Option<Vec<Kind>>)> = ed
                    .variants
                    .iter()
                    .map(|var| {
                        (
                            var.name.clone(),
                            Some(
                                var.field_types
                                    .iter()
                                    .map(|t| self.boundary_kind(&Some(t.clone())))
                                    .collect(),
                            ),
                        )
                    })
                    .collect();
                self.render_constructors(v, &arms, n)?
            }
            Kind::Inductive(ii) => {
                let (ty, ctors) = self.inductive(ii);
                if ty == "Option" {
                    // An `Option` shows as its payload, or `nothing` — never as its wrapper.
                    let can_be_some = ctors.iter().any(|(c, ks)| c == "Some" && ks.is_some());
                    if !can_be_some {
                        "\"nothing\"".to_string()
                    } else {
                        let some = self.ctor_tag("Option", "Some").unwrap();
                        let payload = self.inductive_payload(ii, "Some", 0);
                        let inner = self.render(&format!("_o{n}[1]"), payload)?;
                        format!("({{ int64_t *_o{n} = (int64_t*)(intptr_t)({v}); (_o{n}[0] == {some}) ? {inner} : \"nothing\"; }})")
                    }
                } else {
                    self.render_constructors(v, &ctors, n)?
                }
            }
            other => return Err(format!("the display of a {other:?} value")),
        })
    }

    /// The display of an enum value `v` whose constructors, in tag order, carry the payload kinds `arms`: the
    /// constructor's name, then its payloads in parentheses when it has any.
    fn render_constructors(
        &mut self,
        v: &str,
        arms: &[(String, Option<Vec<Kind>>)],
        n: usize,
    ) -> Result<String, String> {
        let mut cases = String::new();
        for (tag, (name, payloads)) in arms.iter().enumerate() {
            // A constructor the value can never be needs no case.
            let Some(payloads) = payloads else { continue };
            let mut body = format!("lc_sb_add(&_b{n}, \"{}\"); ", c_escape(name));
            if !payloads.is_empty() {
                body.push_str(&format!("lc_sb_add(&_b{n}, \"(\"); "));
                for (i, pk) in payloads.iter().enumerate() {
                    if i > 0 {
                        body.push_str(&format!("lc_sb_add(&_b{n}, \", \"); "));
                    }
                    let part = self.render(&format!("_e{n}[{}]", i + 1), *pk)?;
                    body.push_str(&format!("lc_sb_add(&_b{n}, {part}); "));
                }
                body.push_str(&format!("lc_sb_add(&_b{n}, \")\"); "));
            }
            cases.push_str(&format!("case {tag}: {body}break; "));
        }
        Ok(format!(
            "({{ int64_t *_e{n} = (int64_t*)(intptr_t)({v}); lc_sb _b{n} = {{0}}; lc_sb_add(&_b{n}, \"\"); switch (_e{n}[0]) {{ {cases}}} _b{n}.s; }})"
        ))
    }

    /// The C address a `Send`/`Stream`/`Await` operand names — a peer's topic or an address string — or, for any
    /// other kind, the VM's refusal emitted in its place (`None`).
    fn peer_address(&mut self, reg: u16, kinds: &[Kind]) -> Result<Option<String>, String> {
        match kind_at(kinds, reg) {
            Kind::Peer | Kind::Text => Ok(Some(as_cstr(reg))),
            k => {
                let sample =
                    sample_value(k).ok_or_else(|| format!("a peer address of kind {k:?}"))?;
                self.line(&format!("logos_fatalf({});", words::not_a_peer(sample)));
                Ok(None)
            }
        }
    }

    /// A C expression (0 or 1) that the values `a` (of kind `ka`) and `b` (of kind `kb`) — `int64_t` C expressions —
    /// are equal, as the VM's `values_equal` decides it: numbers by value across their kinds, text by its bytes,
    /// a sequence or tuple element by element, a record field by field, an enum by constructor and payloads, a set
    /// or map by content in any order, absence as one thing however spelled; values of unrelated kinds never.
    fn eq_expr(&mut self, a: &str, ka: Kind, b: &str, kb: Kind) -> Result<String, String> {
        let numeric =
            |k: Kind| matches!(k, Kind::Int | Kind::Float | Kind::BigInt | Kind::Rational);
        let n = self.fresh();
        Ok(match (ka, kb) {
            (x, y) if numeric(x) && numeric(y) => {
                let (la, lb) = (self.scalar_c(a, x), self.scalar_c(b, y));
                match (x, y) {
                    (Kind::Int, Kind::Int) => format!("({a} == {b})"),
                    (Kind::BigInt | Kind::Int, Kind::BigInt | Kind::Int) => {
                        let tagged = |v: &str, k: Kind| {
                            if k == Kind::BigInt {
                                v.to_string()
                            } else {
                                format!("lc_xof({v})")
                            }
                        };
                        format!("(lc_xcmp({}, {}) == 0)", tagged(a, x), tagged(b, y))
                    }
                    (Kind::Rational, Kind::Rational) => format!("(ldec_cmp({la}, {lb}) == 0)"),
                    (Kind::Rational, Kind::Int) => format!("(ldec_cmp({la}, ldec_from_i64({b})) == 0)"),
                    (Kind::Int, Kind::Rational) => format!("(ldec_cmp(ldec_from_i64({a}), {lb}) == 0)"),
                    (Kind::Rational, Kind::BigInt) => format!("(ldec_cmp({la}, ldec_from_lint({lb})) == 0)"),
                    (Kind::BigInt, Kind::Rational) => format!("(ldec_cmp(ldec_from_lint({la}), {lb}) == 0)"),
                    (Kind::Rational, Kind::Float) => format!("(ldec_cmp_f64({la}, {lb}) == 0)"),
                    (Kind::Float, Kind::Rational) => format!("(ldec_cmp_f64({lb}, {la}) == 0)"),
                    (Kind::BigInt, Kind::Float) => format!("(lint_to_f64({la}) == {lb})"),
                    (Kind::Float, Kind::BigInt) => format!("({la} == lint_to_f64({lb}))"),
                    _ => format!("({la} == {lb})"),
                }
            }
            (Kind::Bool, Kind::Bool) | (Kind::Date, Kind::Date) | (Kind::Moment, Kind::Moment) => {
                format!("({a} == {b})")
            }
            (Kind::Word(x), Kind::Word(y)) => {
                if x == y {
                    format!("({a} == {b})")
                } else {
                    "0".to_string()
                }
            }
            (Kind::Text, Kind::Text) | (Kind::Peer, Kind::Peer) => {
                format!("str_equals((const char*)(intptr_t)({a}), (const char*)(intptr_t)({b}))")
            }
            (Kind::Nothing, Kind::Nothing) => "1".to_string(),
            (Kind::Nothing, Kind::Inductive(ii)) | (Kind::Inductive(ii), Kind::Nothing) => {
                let v = if ka == Kind::Nothing { b } else { a };
                match (
                    self.inductive(ii).0.as_str(),
                    self.ctor_tag("Option", "None"),
                ) {
                    ("Option", Some(none)) => format!("(((int64_t*)(intptr_t)({v}))[0] == {none})"),
                    _ => "0".to_string(),
                }
            }
            (x, y) if is_sequence(x) && is_sequence(y) => {
                let (sx, sy) = (seq_kind_of(x), seq_kind_of(y));
                let (tx, ty) = (sx.seq_c_type().unwrap(), sy.seq_c_type().unwrap());
                let (ex, ey) = (self.elem_kind(x), self.elem_kind(y));
                let same = if ex == Kind::Never || ey == Kind::Never {
                    "1".to_string()
                } else {
                    let (ca, cb) = (
                        elem_to_reg(&format!("_s{n}->data[_i{n}]"), ex),
                        elem_to_reg(&format!("_t{n}->data[_i{n}]"), ey),
                    );
                    self.eq_expr(&ca, ex, &cb, ey)?
                };
                format!(
                    "({{ {tx} *_s{n} = ({tx}*)(intptr_t)({a}); {ty} *_t{n} = ({ty}*)(intptr_t)({b}); int _e{n} = _s{n}->len == _t{n}->len; \
                     for (size_t _i{n} = 0; _e{n} && _i{n} < _s{n}->len; _i{n}++) _e{n} = {same}; _e{n}; }})"
                )
            }
            (Kind::Tuple(x), Kind::Tuple(y)) => {
                let (px, py) = (self.tuple_positions(x), self.tuple_positions(y));
                if px.len() != py.len() {
                    return Ok("0".to_string());
                }
                let mut parts = Vec::new();
                for (i, (kx, ky)) in px.iter().zip(py.iter()).enumerate() {
                    parts.push(self.eq_expr(
                        &format!("_s{n}[{i}]"),
                        *kx,
                        &format!("_t{n}[{i}]"),
                        *ky,
                    )?);
                }
                let all = if parts.is_empty() {
                    "1".to_string()
                } else {
                    parts.join(" && ")
                };
                format!("({{ int64_t *_s{n} = (int64_t*)(intptr_t)({a}), *_t{n} = (int64_t*)(intptr_t)({b}); {all}; }})")
            }
            (
                x @ (Kind::Struct(_) | Kind::StructSynth(_)),
                y @ (Kind::Struct(_) | Kind::StructSynth(_)),
            ) => {
                if self.struct_name_of_kind(x) != self.struct_name_of_kind(y) || x != y {
                    return Ok("0".to_string());
                }
                let name = self.struct_name_of_kind(x).unwrap_or_default();
                let fields: Vec<(usize, Kind)> = match x {
                    Kind::Struct(si) => {
                        let sd = self.prog.struct_types[si as usize].clone();
                        sd.fields
                            .iter()
                            .enumerate()
                            .map(|(c, (_, _))| (c, self.field_storage_kind(x, c).unwrap()))
                            .collect()
                    }
                    Kind::StructSynth(idx) => self.synth_structs.borrow()[idx as usize]
                        .1
                        .iter()
                        .enumerate()
                        .map(|(c, (_, _))| (c, self.field_storage_kind(x, c).unwrap()))
                        .collect(),
                    _ => unreachable!(),
                };
                let mut parts = Vec::new();
                for (c, fk) in fields {
                    let fk = if self.counter_fields.contains(&(name.clone(), c)) {
                        Kind::CrdtCounter
                    } else {
                        fk
                    };
                    let (ca, cb) = (format!("_s{n}[{c}]"), format!("_t{n}[{c}]"));
                    parts.push(if fk == Kind::CrdtCounter {
                        format!("(logos_crdt_sum((logos_crdt*)(intptr_t){ca}) == logos_crdt_sum((logos_crdt*)(intptr_t){cb}))")
                    } else {
                        self.eq_expr(&ca, fk, &cb, fk)?
                    });
                }
                let all = if parts.is_empty() {
                    "1".to_string()
                } else {
                    parts.join(" && ")
                };
                format!("({{ int64_t *_s{n} = (int64_t*)(intptr_t)({a}), *_t{n} = (int64_t*)(intptr_t)({b}); {all}; }})")
            }
            (Kind::Enum(x), Kind::Enum(y)) if x == y => {
                let ed = self.prog.enum_types[x as usize].clone();
                let arms: Vec<(String, Option<Vec<Kind>>)> = ed
                    .variants
                    .iter()
                    .map(|var| {
                        (
                            var.name.clone(),
                            Some(
                                var.field_types
                                    .iter()
                                    .map(|t| self.boundary_kind(&Some(t.clone())))
                                    .collect(),
                            ),
                        )
                    })
                    .collect();
                self.eq_constructors(a, b, &arms, n)?
            }
            (Kind::Inductive(x), Kind::Inductive(y)) => {
                let (tx, _) = self.inductive(x);
                let (ty, _) = self.inductive(y);
                if tx != ty {
                    return Ok("0".to_string());
                }
                let merged = self
                    .unify_inductive(x, y)
                    .ok_or("an equality of two differently-shaped enums")?;
                let Kind::Inductive(m) = merged else {
                    unreachable!()
                };
                let arms = self.inductive(m).1;
                self.eq_constructors(a, b, &arms, n)?
            }
            (
                Kind::SetInt | Kind::SetWord(_) | Kind::SetText | Kind::SetUnknown,
                Kind::SetInt | Kind::SetWord(_) | Kind::SetText | Kind::SetUnknown,
            ) => {
                let (sx, sy) = (resolve_set(ka).unwrap(), resolve_set(kb).unwrap());
                if sx != sy && ka != Kind::SetUnknown && kb != Kind::SetUnknown {
                    return Ok("0".to_string());
                }
                let (ty, infix) = (sx.set_c_type().unwrap(), sx.seq_infix().unwrap());
                format!(
                    "({{ {ty} *_s{n} = ({ty}*)(intptr_t)({a}), *_t{n} = ({ty}*)(intptr_t)({b}); int _e{n} = _s{n}->len == _t{n}->len; \
                     for (size_t _o{n} = 0; _e{n} && _o{n} < _s{n}->ord_len; _o{n}++) {{ size_t _k{n} = _s{n}->order[_o{n}]; if (_s{n}->state[_k{n}]) _e{n} = set_{infix}_contains(_t{n}, _s{n}->keys[_k{n}]); }} _e{n}; }})"
                )
            }
            (Kind::Map(_) | Kind::MapUnknown, Kind::Map(_) | Kind::MapUnknown) => {
                let ((kx, vx), (ky, vy)) = (
                    self.resolve_map_kv(ka).unwrap(),
                    self.resolve_map_kv(kb).unwrap(),
                );
                if kx != ky {
                    return Ok("0".to_string());
                }
                let (tx, ty) = (
                    map_type_name(kx, vx).unwrap(),
                    map_type_name(ky, vy).unwrap(),
                );
                let (fy, key) = (map_fn(ky, vy).unwrap(), format!("_s{n}->keys[_k{n}]"));
                let va = map_val_load(&format!("_s{n}->vals[_k{n}]"), vx);
                let vb = map_val_load(&format!("{fy}_get(_t{n}, {key})"), vy);
                let same = self.eq_expr(&va, vx, &vb, vy)?;
                format!(
                    "({{ {tx} *_s{n} = ({tx}*)(intptr_t)({a}); {ty} *_t{n} = ({ty}*)(intptr_t)({b}); int _e{n} = _s{n}->len == _t{n}->len; \
                     for (size_t _o{n} = 0; _e{n} && _o{n} < _s{n}->ord_len; _o{n}++) {{ size_t _k{n} = _s{n}->order[_o{n}]; if (_s{n}->state[_k{n}]) _e{n} = {fy}_contains(_t{n}, {key}) && {same}; }} _e{n}; }})"
                )
            }
            (Kind::Unknown, _) | (_, Kind::Unknown) => {
                return Err(format!("an equality over {ka:?} and {kb:?}"))
            }
            _ => "0".to_string(),
        })
    }

    /// Equality of two values of one enum whose constructors, in tag order, carry the payload kinds `arms`:
    /// the same constructor with equal payloads.
    fn eq_constructors(
        &mut self,
        a: &str,
        b: &str,
        arms: &[(String, Option<Vec<Kind>>)],
        n: usize,
    ) -> Result<String, String> {
        let mut cases = String::new();
        for (tag, (_, payloads)) in arms.iter().enumerate() {
            let Some(payloads) = payloads else { continue };
            let mut parts = Vec::new();
            for (i, pk) in payloads.iter().enumerate() {
                parts.push(self.eq_expr(
                    &format!("_s{n}[{}]", i + 1),
                    *pk,
                    &format!("_t{n}[{}]", i + 1),
                    *pk,
                )?);
            }
            let all = if parts.is_empty() {
                "1".to_string()
            } else {
                parts.join(" && ")
            };
            cases.push_str(&format!("case {tag}: _e{n} = {all}; break; "));
        }
        Ok(format!(
            "({{ int64_t *_s{n} = (int64_t*)(intptr_t)({a}), *_t{n} = (int64_t*)(intptr_t)({b}); int _e{n} = _s{n}[0] == _t{n}[0]; if (_e{n}) switch (_s{n}[0]) {{ {cases}default: break; }} _e{n}; }})"
        ))
    }

    /// A scalar value `v` (an `int64_t` C expression) of numeric kind `k` in the form the numeric runtime takes
    /// it: a `double` for a Real, an `LInt` for an exact integer, an `LDec` for a rational, the integer itself.
    /// Register `reg` (kind `k`) as the double the VM computes with beside a real: a rational's numerator over its
    /// denominator, any other number as [`as_double`] reads it.
    fn real_of(&self, reg: u16, k: Kind) -> String {
        if k == Kind::Rational {
            format!("ldec_to_f64(*(LDec*)(intptr_t)r[{reg}])")
        } else {
            as_double(reg, k)
        }
    }

    /// The C expression of the machine integer an op reads from `reg` at `place`: the register; the `Int` a cell of the
    /// extension or a whole rational is, a fraction or one past i64 refused in the VM's words for the place (any
    /// other value of the extension in the interpreter's words for it).
    fn int_operand(&self, kinds: &[Kind], reg: u16, place: words::IntegerPlace) -> String {
        match kind_at(kinds, reg) {
            Kind::Tower => format!(
                "ltower_int_at((LTower)(intptr_t)r[{reg}], {}, {})",
                words::integer_past_a_word(place),
                words::fraction_for_an_integer(place)
            ),
            Kind::Rational => format!(
                "ldec_as_i64(*(LDec*)(intptr_t)r[{reg}], {}, {})",
                words::integer_past_a_word(place),
                words::fraction_for_an_integer(place)
            ),
            _ => format!("r[{reg}]"),
        }
    }

    /// `r[dst] = lhs % rhs` with a rational operand: two exact numbers as `ldec_remainder` takes them; beside a real,
    /// the VM's refusal for the kinds the two values are, a whole rational being the integer it equals.
    fn rational_remainder(&mut self, dst: u16, lhs: u16, rhs: u16, lk: Kind, rk: Kind) -> Result<(), String> {
        if lk != Kind::Float && rk != Kind::Float {
            let (l, r) = (as_ldec(lhs, lk)?, as_ldec(rhs, rk)?);
            self.box_ldec(dst, &format!("ldec_remainder({l}, {r})"));
            return Ok(());
        }
        let fraction = RuntimeValue::from_rational(
            logicaffeine_base::Rational::from_ratio_i64(1, 2).expect("a nonzero denominator"),
        );
        let side = |k: Kind, whole: bool| {
            if k == Kind::Rational && !whole {
                Some(fraction.clone())
            } else {
                sample_value(if k == Kind::Rational { Kind::Int } else { k })
            }
        };
        let refused = |whole: bool| -> Result<String, String> {
            let (Some(l), Some(r)) = (side(lk, whole), side(rk, whole)) else {
                return Err(format!("a remainder of {lk:?} and {rk:?}"));
            };
            Ok(words::modulo_refused(l, r))
        };
        let (whole, fractional) = (refused(true)?, refused(false)?);
        let rational = if lk == Kind::Rational { lhs } else { rhs };
        self.line(&format!(
            "logos_fatalf(ldec_whole((LDec*)(intptr_t)r[{rational}]) ? {whole} : {fractional});"
        ));
        Ok(())
    }

    /// The sequence kind of a list literal whose items are of kinds `items`: one kind (the forms of one integer aside),
    /// or integers beside rationals, held as rationals; the VM keeps each item the value it is, so items of other
    /// different kinds are refused by name.
    fn list_kind(&self, items: &[Kind]) -> Result<Kind, String> {
        let integer = |k: Kind| matches!(k, Kind::Int | Kind::BigInt);
        let exact = |k: Kind| integer(k) || k == Kind::Rational;
        let known: Vec<Kind> = items.iter().copied().filter(|k| *k != Kind::Never).collect();
        let Some(&first) = known.first() else {
            return Ok(self.seq_kind_for(items.first().copied().unwrap_or(Kind::Unknown)));
        };
        if known.iter().all(|k| *k == first || (integer(*k) && integer(first))) {
            return Ok(self.seq_kind_for(first));
        }
        if known.iter().all(|k| exact(*k)) {
            return Ok(self.seq_kind_for(Kind::Rational));
        }
        Err("a list whose items differ in kind is not yet lowered".to_string())
    }

    /// Whether a value of kind `from` is handed to a place declared `to` as the exact number it equals: an integer
    /// where a rational is declared, a list of integers where a list of rationals is.
    fn widens_to(&self, from: Kind, to: Kind) -> bool {
        match (from, to) {
            (Kind::Int | Kind::BigInt, Kind::Rational) => true,
            (Kind::SeqInt, Kind::SeqOf(_)) => self.elem_kind(to) == Kind::Rational,
            _ => false,
        }
    }

    /// Register `reg` (kind `k`) as an item of a sequence whose items are of kind `elem`: an integer stored among
    /// rationals is the rational it equals.
    fn elem_value(&mut self, reg: u16, k: Kind, elem: Kind) -> Result<String, String> {
        if elem == Kind::Rational && matches!(k, Kind::Int | Kind::BigInt) {
            return self.convert_number(&format!("r[{reg}]"), k, Kind::Rational);
        }
        Ok(as_elem_push(reg, elem))
    }

    /// `r[dst]` set to a fresh box holding the `LDec` expression `value`.
    fn box_ldec(&mut self, dst: u16, value: &str) {
        let n = self.fresh();
        self.line(&format!(
            "LDec *_rat{n} = (LDec*)lc_gc_alloc(sizeof(LDec)); *_rat{n} = {value}; r[{dst}] = (int64_t)(intptr_t)_rat{n};"
        ));
    }

    fn scalar_c(&self, v: &str, k: Kind) -> String {
        match k {
            Kind::Float => format!("as_f64({v})"),
            Kind::BigInt => format!("lc_xget({v})"),
            Kind::Rational => format!("(*(LDec*)(intptr_t)({v}))"),
            _ => format!("({v})"),
        }
    }

    /// Register `reg` (of kind `k`) as text, the way `combined with`, interpolation and `format` read it: a
    /// scalar as `Show` renders it, any other value as its display.
    fn text_of(&mut self, reg: u16, k: Kind) -> Result<String, String> {
        match k {
            Kind::Text | Kind::Bool | Kind::Float | Kind::Rational | Kind::BigInt | Kind::Int => {
                Ok(coerce_cstr(reg, k))
            }
            Kind::Word(_) => Ok(format!(
                "logos_dyn_sprintf(\"%\" PRIu64, (uint64_t)r[{reg}])"
            )),
            other => self.render(&format!("r[{reg}]"), other),
        }
    }

    fn lower_op(
        &mut self,
        op: &Op,
        kinds: &[Kind],
        after: &[Kind],
        in_function: bool,
        pc: usize,
    ) -> Result<(), String> {
        match op {
            Op::LoadConst { dst, idx } => match &self.prog.constants[*idx as usize] {
                Constant::Int(n) => self.line(&format!("r[{dst}] = {n}LL;")),
                Constant::Duration(n) => self.line(&format!("r[{dst}] = {n}LL;")),
                Constant::Bool(b) => self.line(&format!("r[{dst}] = {};", if *b { 1 } else { 0 })),
                Constant::Float(f) => self.line(&format!(
                    "r[{dst}] = (int64_t)0x{:016X}ULL; /* {} */",
                    f.to_bits(),
                    f
                )),
                Constant::Text(s) => self.line(&format!(
                    "r[{dst}] = (int64_t)(intptr_t)\"{}\";",
                    c_escape(s)
                )),
                // `Nothing` — the seed value of a CRDT register/counter field (a `Divergent T` / counter before
                // its first write). It rides the slot as `0` (an empty
                // pointer / zero int); a following `StructInsert` / `CrdtBump` / `CrdtResolve` establishes the
                // real cell value, matching the VM's `RuntimeValue::Nothing` initial field state.
                Constant::Nothing => self.line(&format!("r[{dst}] = 0;")),
                Constant::Date(days) => self.line(&format!("r[{dst}] = {days}LL;")),
                Constant::Moment(nanos) | Constant::Time(nanos) => self.line(&format!("r[{dst}] = {nanos}LL;")),
                Constant::Span { months, days } => self.line(&format!(
                    "r[{dst}] = (int64_t)(((uint64_t)(uint32_t){months} << 32) | (uint32_t){days});"
                )),
                c => return Err(format!("LoadConst {c:?} not yet lowered")),
            },
            Op::Move { dst, src } => self.line(&format!("r[{dst}] = r[{src}];")),
            // An exact result (or operand) goes through the tagged `lc_x*` operations; anything else takes the
            // machine path.
            Op::Add { dst, lhs, rhs }
                if self.exact_integers(&[*lhs, *rhs], kinds, kind_at(after, *dst)) =>
            {
                self.emit_exact_bin(
                    *dst,
                    *lhs,
                    *rhs,
                    kind_at(kinds, *lhs),
                    kind_at(kinds, *rhs),
                    "lc_xadd",
                )?;
            }
            Op::Add { dst, lhs, rhs } => self.arith(*dst, *lhs, *rhs, "+", kinds)?,
            Op::Sub { dst, lhs, rhs }
                if self.exact_integers(&[*lhs, *rhs], kinds, kind_at(after, *dst)) =>
            {
                self.emit_exact_bin(
                    *dst,
                    *lhs,
                    *rhs,
                    kind_at(kinds, *lhs),
                    kind_at(kinds, *rhs),
                    "lc_xsub",
                )?;
            }
            Op::Sub { dst, lhs, rhs } => self.arith(*dst, *lhs, *rhs, "-", kinds)?,
            Op::Mul { dst, lhs, rhs }
                if self.exact_integers(&[*lhs, *rhs], kinds, kind_at(after, *dst)) =>
            {
                self.emit_exact_bin(
                    *dst,
                    *lhs,
                    *rhs,
                    kind_at(kinds, *lhs),
                    kind_at(kinds, *rhs),
                    "lc_xmul",
                )?;
            }
            Op::Mul { dst, lhs, rhs } => self.arith(*dst, *lhs, *rhs, "*", kinds)?,
            // A quotient or remainder of exact integers: exact when its result is, else the machine integer it
            // provably fits.
            Op::Div { dst, lhs, rhs }
                if self.exact_integers(&[*lhs, *rhs], kinds, kind_at(after, *dst)) =>
            {
                self.emit_exact_bounded(*dst, *lhs, *rhs, kinds, kind_at(after, *dst), "lc_xdiv")?;
            }
            Op::Div { dst, lhs, rhs } => self.arith(*dst, *lhs, *rhs, "/", kinds)?,
            // Exact rational division (`Let x: Rational be 7 / 2` → `7/2`, never floored). Operands are Int or
            // Rational; box the reduced `LDec` result and store its pointer in the register. Reuses the runtime's
            // exact `ldec_div` (num/den over `LBig`) verbatim — the same helper the AST C backend calls.
            Op::ExactDiv { dst, lhs, rhs } => {
                let (lk, rk) = (kind_at(kinds, *lhs), kind_at(kinds, *rhs));
                let (l, r) = (as_ldec(*lhs, lk)?, as_ldec(*rhs, rk)?);
                self.line(&format!("lc_exact_divisor({});", exact_zero_test(*rhs, rk)));
                let n = self.fresh();
                self.line(&format!(
                    "LDec *_rat{n} = (LDec*)lc_gc_alloc(sizeof(LDec)); *_rat{n} = ldec_div({l}, {r}); r[{dst}] = (int64_t)(intptr_t)_rat{n};"
                ));
            }
            // An integer remainder by zero fails as the VM's does; the VM computes no remainder of a Real, so
            // neither does the walker — it fails with the VM's refusal for those two operands.
            Op::Mod { dst, lhs, rhs }
                if self.exact_integers(&[*lhs, *rhs], kinds, kind_at(after, *dst)) =>
            {
                self.emit_exact_bounded(*dst, *lhs, *rhs, kinds, kind_at(after, *dst), "lc_xmod")?;
            }
            Op::Mod { dst, lhs, rhs } => {
                let (lk, rk) = (kind_at(kinds, *lhs), kind_at(kinds, *rhs));
                if lk == Kind::Rational || rk == Kind::Rational {
                    self.rational_remainder(*dst, *lhs, *rhs, lk, rk)?;
                } else if matches!(lk, Kind::Word(_)) || matches!(rk, Kind::Word(_)) {
                    // A word's remainder is unsigned, in its ring, as `/` is.
                    self.word_arith(*dst, *lhs, *rhs, "%", lk, rk)?;
                } else if lk == Kind::Float || rk == Kind::Float {
                    let (lv, rv) = (sample_value(lk), sample_value(rk));
                    let (Some(lv), Some(rv)) = (lv, rv) else {
                        return Err(format!("a remainder of {lk:?} and {rk:?}"));
                    };
                    self.line(&format!("logos_fatalf({});", words::modulo_refused(lv, rv)));
                } else {
                    self.line(&format!("r[{dst}] = logos_imod(r[{lhs}], r[{rhs}]);"));
                }
            }
            Op::AddAssign { dst, src }
                if self.exact_integers(&[*dst, *src], kinds, kind_at(after, *dst)) =>
            {
                self.emit_exact_bin(
                    *dst,
                    *dst,
                    *src,
                    kind_at(kinds, *dst),
                    kind_at(kinds, *src),
                    "lc_xadd",
                )?;
            }
            Op::AddAssign { dst, src } => {
                let (dk, sk) = (kind_at(kinds, *dst), kind_at(kinds, *src));
                if dk == Kind::Text {
                    // `Set s to s + …` on Text appends to the text the register owns, in place. A number's
                    // rendering is a fresh temporary nothing else holds, released once appended.
                    if sk == Kind::Int
                        && crate::optimize::active_config()
                            .is_on(crate::optimization::Opt::StringAccumReuse)
                    {
                        crate::optimize::mark_fired(crate::optimization::Opt::StringAccumReuse);
                        self.line(&format!("r[{dst}] = lc_text_append_i64(r[{dst}], own[{dst}], r[{src}]); own[{dst}] = 1;"));
                        return Ok(());
                    }
                    let tail = self.text_of(*src, sk)?;
                    if !matches!(sk, Kind::Int | Kind::Float | Kind::Rational | Kind::BigInt) {
                        self.line(&format!("r[{dst}] = lc_text_append(r[{dst}], own[{dst}], {tail}); own[{dst}] = 1;"));
                    } else {
                        let n = self.fresh();
                        self.line(&format!(
                            "{{ char *_tail{n} = (char*)({tail}); r[{dst}] = lc_text_append(r[{dst}], own[{dst}], _tail{n}); lc_gc_free(_tail{n}); own[{dst}] = 1; }}"
                        ));
                    }
                } else if sk == Kind::Text {
                    let (l, r) = (self.text_of(*dst, dk)?, self.text_of(*src, sk)?);
                    self.line(&format!(
                        "r[{dst}] = (int64_t)(intptr_t)str_concat({l}, {r});"
                    ));
                } else if dk == Kind::Rational || sk == Kind::Rational {
                    self.arith(*dst, *dst, *src, "+", kinds)?;
                } else if matches!(dk, Kind::Word(_)) || matches!(sk, Kind::Word(_)) {
                    // A word adds in its ring, as `+` does.
                    self.word_arith(*dst, *dst, *src, "+", dk, sk)?;
                } else if dk == Kind::Float || sk == Kind::Float {
                    self.line(&format!(
                        "r[{dst}] = as_bits({} + {});",
                        as_double(*dst, dk),
                        as_double(*src, sk)
                    ));
                } else {
                    self.line(&format!("r[{dst}] = r[{dst}] + r[{src}];"));
                }
            }
            Op::Pow { dst, lhs, rhs } => {
                let (lk, rk) = (kind_at(kinds, *lhs), kind_at(kinds, *rhs));
                if (lk == Kind::Rational || rk == Kind::Rational) && lk != Kind::Float && rk != Kind::Float {
                    let (b, e) = (as_ldec(*lhs, lk)?, as_ldec(*rhs, rk)?);
                    self.box_ldec(*dst, &format!("ldec_power({b}, {e})"));
                } else if lk == Kind::Rational || rk == Kind::Rational {
                    self.line(&format!(
                        "r[{dst}] = as_bits(pow({}, {}));",
                        self.real_of(*lhs, lk),
                        self.real_of(*rhs, rk)
                    ));
                } else if kind_at(after, *dst) == Kind::BigInt {
                    let base = as_exact(*lhs, lk)?;
                    let exponent = if rk == Kind::BigInt {
                        format!("lc_xnarrow(r[{rhs}])")
                    } else {
                        format!("r[{rhs}]")
                    };
                    self.line(&format!("r[{dst}] = lc_xpow({base}, {exponent});"));
                } else if lk == Kind::Float || rk == Kind::Float {
                    self.line(&format!(
                        "r[{dst}] = as_bits(pow({}, {}));",
                        as_double(*lhs, lk),
                        as_double(*rhs, rk)
                    ));
                } else {
                    self.line(&format!("r[{dst}] = ipow64(r[{lhs}], r[{rhs}]);"));
                }
            }
            Op::FloorDiv { dst, lhs, rhs }
                if self.exact_integers(&[*lhs, *rhs], kinds, kind_at(after, *dst)) =>
            {
                self.emit_exact_bounded(
                    *dst,
                    *lhs,
                    *rhs,
                    kinds,
                    kind_at(after, *dst),
                    "lc_xfloordiv",
                )?;
            }
            Op::FloorDiv { dst, lhs, rhs } => {
                let (lk, rk) = (kind_at(kinds, *lhs), kind_at(kinds, *rhs));
                if lk == Kind::Float || rk == Kind::Float {
                    self.line(&format!(
                        "r[{dst}] = as_bits(lc_rfldiv({}, {}));",
                        self.real_of(*lhs, lk),
                        self.real_of(*rhs, rk)
                    ));
                } else if lk == Kind::Rational || rk == Kind::Rational {
                    let (l, r) = (as_ldec(*lhs, lk)?, as_ldec(*rhs, rk)?);
                    self.box_ldec(*dst, &format!("ldec_floor_quotient({l}, {r})"));
                } else {
                    self.line(&format!("r[{dst}] = fldiv64(r[{lhs}], r[{rhs}]);"));
                }
            }
            // Scalar math builtins called as functions (`sqrt(x)`, `abs(x)`, `floor(x)`, `min(a,b)`, …). Args are
            // already evaluated into consecutive registers at `args_start`. Mirrors `codegen_riscv::call_builtin`
            // exactly: Sqrt is Int/Float→Float; Abs is fabs (Float) / wrapping i64 (Int, i64::MIN is the bignum
            // gap); Floor/Ceil/Round land an i64 (Float via libm then truncate, Int passthrough); Min/Max promote
            // to Float if either arg is Float, else integer compare-select. Non-math builtins are refused.
            Op::CallBuiltin {
                dst,
                builtin,
                args_start,
                ..
            } => {
                use crate::semantics::builtins::BuiltinId;
                let a = *args_start;
                let ak = kind_at(kinds, a);
                match builtin {
                    // `Fail with <value>.` / a broken contract's reason: the reason on stderr, exit 1 — the
                    // report every tier gives. A contract reason that is not Text is refused in the other
                    // tiers' words; a kind this backend cannot render is refused at generation, never
                    // printed as a pointer.
                    BuiltinId::Fail | BuiltinId::FailReason => {
                        let text = match (builtin, ak) {
                            (_, Kind::Text) => as_cstr(a),
                            (BuiltinId::Fail, Kind::Int | Kind::Bool | Kind::Float | Kind::Rational | Kind::BigInt) => {
                                coerce_cstr(a, ak)
                            }
                            (BuiltinId::FailReason, Kind::Int | Kind::BigInt) => {
                                "\"The reason after `otherwise` must be Text, but it was Int.\"".to_string()
                            }
                            (BuiltinId::FailReason, Kind::Float) => {
                                "\"The reason after `otherwise` must be Text, but it was Real.\"".to_string()
                            }
                            (BuiltinId::FailReason, Kind::Bool) => {
                                "\"The reason after `otherwise` must be Text, but it was Bool.\"".to_string()
                            }
                            (_, k) => return Err(format!("a failure reason of kind {k:?} is not yet rendered by the C backend")),
                        };
                        self.line(&format!(
                            "fflush(stdout); fprintf(stderr, \"%s\\n\", {text}); exit(1);"
                        ));
                    }
                    // `format(x)` — the value's text exactly as `Show` renders it.
                    BuiltinId::Format => match ak {
                        Kind::Text => self.line(&format!("r[{dst}] = r[{a}];")),
                        k => {
                            let text = self.text_of(a, k)?;
                            self.line(&format!("r[{dst}] = (int64_t)(intptr_t)({text});"));
                        }
                    },
                    BuiltinId::Sqrt => {
                        self.line(&format!("r[{dst}] = as_bits(sqrt({}));", as_double(a, ak)));
                    }
                    BuiltinId::Abs => match ak {
                        Kind::Float => {
                            self.line(&format!("r[{dst}] = as_bits(fabs(as_f64(r[{a}])));"))
                        }
                        Kind::Int => {
                            self.line(&format!("r[{dst}] = (r[{a}] < 0) ? -r[{a}] : r[{a}];"))
                        }
                        k => return Err(format!("abs of a non-number kind {k:?}")),
                    },
                    BuiltinId::Floor | BuiltinId::Ceil | BuiltinId::Round => match ak {
                        Kind::Float => {
                            let f = match builtin {
                                BuiltinId::Floor => "floor",
                                BuiltinId::Ceil => "ceil",
                                _ => "round",
                            };
                            self.line(&format!("r[{dst}] = (int64_t){f}(as_f64(r[{a}]));"));
                        }
                        Kind::Int => self.line(&format!("r[{dst}] = r[{a}];")),
                        k => return Err(format!("floor/ceil/round of a non-number kind {k:?}")),
                    },
                    BuiltinId::Min | BuiltinId::Max => {
                        let b = *args_start + 1;
                        let bk = kind_at(kinds, b);
                        if ak == Kind::Float || bk == Kind::Float {
                            let f = if matches!(builtin, BuiltinId::Min) {
                                "logos_min_f64"
                            } else {
                                "logos_max_f64"
                            };
                            self.line(&format!(
                                "r[{dst}] = as_bits({f}({}, {}));",
                                as_double(a, ak),
                                as_double(b, bk)
                            ));
                        } else {
                            let op = if matches!(builtin, BuiltinId::Min) {
                                "<"
                            } else {
                                ">"
                            };
                            self.line(&format!(
                                "r[{dst}] = (r[{a}] {op} r[{b}]) ? r[{a}] : r[{b}];"
                            ));
                        }
                    }
                    // `text_bytes(t)` — the text's UTF-8 bytes; `text_from_bytes(bs)` — back, validated.
                    BuiltinId::TextBytes => {
                        if self.text_byte_calls.contains(&pc) {
                            self.line(&format!("r[{dst}] = r[{a}];"));
                        } else {
                            let n = self.fresh();
                            self.line(&format!(
                                "Seq_i64 *_tb{n} = (Seq_i64*)lc_gc_alloc(sizeof(Seq_i64)); *_tb{n} = logos_text_bytes({}); r[{dst}] = (int64_t)(intptr_t)_tb{n};",
                                as_cstr(a)
                            ));
                        }
                    }
                    BuiltinId::TextFromBytes => {
                        let ptr = Self::seq_ptr(a, seq_kind_of(ak))?;
                        self.line(&format!(
                            "r[{dst}] = (int64_t)(intptr_t)lc_text_from_bytes({ptr});"
                        ));
                    }
                    // `contains(haystack, needle)` — a substring test over two Texts, refused as the VM refuses
                    // any other pair.
                    BuiltinId::Contains => {
                        let b = a + 1;
                        let bk = kind_at(kinds, b);
                        if ak == Kind::Text && bk == Kind::Text {
                            self.line(&format!(
                                "r[{dst}] = (strstr({}, {}) != NULL);",
                                as_cstr(a),
                                as_cstr(b)
                            ));
                        } else {
                            let (Some(x), Some(y)) = (sample_value(ak), sample_value(bk)) else {
                                return Err(format!("contains over {ak:?} and {bk:?}"));
                            };
                            self.line(&format!(
                                "logos_fatalf({});",
                                words::builtin_refused(*builtin, vec![x, y])
                            ));
                        }
                    }
                    // The file statements, against the sandboxed file system (`c_vfs_runtime`): the content and the
                    // path are displayed, as the VM displays them.
                    BuiltinId::VfsWriteFile => {
                        let content = self.text_of(a, ak)?;
                        let path = self.text_of(a + 1, kind_at(kinds, a + 1))?;
                        self.line(&format!(
                            "logos_vfs_write({path}, {content}); r[{dst}] = 0;"
                        ));
                    }
                    BuiltinId::VfsReadFile | BuiltinId::VfsMount => {
                        let path = self.text_of(a, ak)?;
                        let f = if *builtin == BuiltinId::VfsReadFile {
                            "logos_vfs_read"
                        } else {
                            "logos_vfs_mount"
                        };
                        self.line(&format!("r[{dst}] = (int64_t)(intptr_t){f}({path});"));
                    }
                    // The bit counts of an Int's 64-bit two's-complement pattern (64 zeros for 0).
                    BuiltinId::CountOnes | BuiltinId::LeadingZeros | BuiltinId::TrailingZeros => {
                        match ak {
                            Kind::Int => {
                                let count = match builtin {
                                BuiltinId::CountOnes => format!("__builtin_popcountll((unsigned long long)r[{a}])"),
                                BuiltinId::LeadingZeros => format!("(r[{a}] ? __builtin_clzll((unsigned long long)r[{a}]) : 64)"),
                                _ => format!("(r[{a}] ? __builtin_ctzll((unsigned long long)r[{a}]) : 64)"),
                            };
                                self.line(&format!("r[{dst}] = (int64_t){count};"));
                            }
                            k => {
                                let sample = sample_value(k)
                                    .ok_or_else(|| format!("a bit count of a {k:?}"))?;
                                self.line(&format!(
                                    "logos_fatalf({});",
                                    words::builtin_refused(*builtin, vec![sample])
                                ));
                            }
                        }
                    }
                    // Machine words: a word from an Int or another word (truncated to the width), a word's value as
                    // an Int, rotations and bitwise words — each refused, for any other operand, in the VM's words.
                    BuiltinId::Word8Make
                    | BuiltinId::Word16Make
                    | BuiltinId::Word32
                    | BuiltinId::Word64 => match ak {
                        Kind::Int | Kind::Word(_) | Kind::BigInt => {
                            let bits = match builtin {
                                BuiltinId::Word8Make => 8,
                                BuiltinId::Word16Make => 16,
                                BuiltinId::Word32 => 32,
                                _ => 64,
                            };
                            let mask = if bits == 64 {
                                "UINT64_MAX".to_string()
                            } else {
                                format!("((1ULL << {bits}) - 1)")
                            };
                            let low = if ak == Kind::BigInt {
                                format!("lc_xword_bits(r[{a}])")
                            } else {
                                format!("(uint64_t)r[{a}]")
                            };
                            self.line(&format!("r[{dst}] = (int64_t)({low} & {mask});"));
                        }
                        k => self.builtin_refusal(*builtin, &[k])?,
                    },
                    BuiltinId::IntOfWord8
                    | BuiltinId::IntOfWord16
                    | BuiltinId::IntOfWord32
                    | BuiltinId::IntOfWord64 => match (builtin, ak) {
                        (BuiltinId::IntOfWord8, Kind::Int) => {
                            self.line(&format!("r[{dst}] = (int64_t)(uint8_t)r[{a}];"))
                        }
                        (BuiltinId::IntOfWord16, Kind::Int) => {
                            self.line(&format!("r[{dst}] = (int64_t)(uint16_t)r[{a}];"))
                        }
                        (_, Kind::Int | Kind::Word(_)) => self.line(&format!("r[{dst}] = r[{a}];")),
                        (_, k) => self.builtin_refusal(*builtin, &[k])?,
                    },
                    BuiltinId::Rotl | BuiltinId::Rotr => {
                        let nk = kind_at(kinds, a + 1);
                        match (ak, nk) {
                            (Kind::Word(bits), Kind::Int | Kind::Word(_)) => {
                                let mask = if bits == 64 {
                                    "UINT64_MAX".to_string()
                                } else {
                                    format!("((1ULL << {bits}) - 1)")
                                };
                                let (there, back) = if *builtin == BuiltinId::Rotl {
                                    ("<<", ">>")
                                } else {
                                    (">>", "<<")
                                };
                                let n = self.fresh();
                                self.line(&format!(
                                    "{{ uint64_t _w{n} = (uint64_t)r[{a}]; uint32_t _c{n} = (uint32_t)r[{}] % {bits}; r[{dst}] = (int64_t)(_c{n} ? (((_w{n} {there} _c{n}) | (_w{n} {back} ({bits} - _c{n}))) & {mask}) : _w{n}); }}",
                                    a + 1
                                ));
                            }
                            (x, y) => self.builtin_refusal(*builtin, &[x, y])?,
                        }
                    }
                    BuiltinId::Wand | BuiltinId::Wor => {
                        let bk = kind_at(kinds, a + 1);
                        match (ak, bk) {
                            (Kind::Word(x), Kind::Word(y)) if x == y => {
                                let op = if *builtin == BuiltinId::Wand {
                                    "&"
                                } else {
                                    "|"
                                };
                                self.line(&format!("r[{dst}] = r[{a}] {op} r[{}];", a + 1));
                            }
                            (x, y) => self.builtin_refusal(*builtin, &[x, y])?,
                        }
                    }
                    BuiltinId::Wnot => match ak {
                        Kind::Word(bits) => {
                            let mask = if bits == 64 {
                                "UINT64_MAX".to_string()
                            } else {
                                format!("((1ULL << {bits}) - 1)")
                            };
                            self.line(&format!(
                                "r[{dst}] = (int64_t)(~(uint64_t)r[{a}] & {mask});"
                            ));
                        }
                        k => self.builtin_refusal(*builtin, &[k])?,
                    },
                    BuiltinId::Word32Shr => {
                        let nk = kind_at(kinds, a + 1);
                        match (ak, nk) {
                            (Kind::Int | Kind::Word(_), Kind::Int) => {
                                self.line(&format!("r[{dst}] = (int64_t)(uint32_t)((uint32_t)r[{a}] >> ((uint32_t)r[{}] & 31));", a + 1));
                            }
                            (x, y) => self.builtin_refusal(*builtin, &[x, y])?,
                        }
                    }
                    BuiltinId::Word64Shl | BuiltinId::Word64Shr => {
                        let nk = kind_at(kinds, a + 1);
                        match (ak, nk) {
                            (Kind::Int | Kind::Word(_), Kind::Int) => {
                                let dir = if *builtin == BuiltinId::Word64Shl {
                                    "<<"
                                } else {
                                    ">>"
                                };
                                self.line(&format!("r[{dst}] = (int64_t)((uint64_t)r[{a}] {dir} ((uint32_t)r[{}] & 63));", a + 1));
                            }
                            (x, y) => self.builtin_refusal(*builtin, &[x, y])?,
                        }
                    }
                    BuiltinId::Word64And => {
                        let bk = kind_at(kinds, a + 1);
                        match (ak, bk) {
                            (Kind::Int | Kind::Word(_), Kind::Int | Kind::Word(_)) => {
                                self.line(&format!(
                                    "r[{dst}] = (int64_t)((uint64_t)r[{a}] & (uint64_t)r[{}]);",
                                    a + 1
                                ));
                            }
                            (x, y) => self.builtin_refusal(*builtin, &[x, y])?,
                        }
                    }
                    BuiltinId::ReadReals => {
                        let path = self.text_of(a, ak)?;
                        let n = self.fresh();
                        self.line(&format!(
                            "Seq_f64 *_rr{n} = (Seq_f64*)lc_gc_alloc(sizeof(Seq_f64)); *_rr{n} = logos_read_reals({path}); r[{dst}] = (int64_t)(intptr_t)_rr{n};"
                        ));
                    }
                    BuiltinId::TryReadBytes => {
                        let path = self.text_of(a, ak)?;
                        let (ok, err) = (
                            self.ctor_tag("Result", "Ok").unwrap(),
                            self.ctor_tag("Result", "Err").unwrap(),
                        );
                        self.line(&format!(
                            "r[{dst}] = lc_try_read_bytes({path}, {ok}, {err});"
                        ));
                    }
                    BuiltinId::Chr => match ak {
                        Kind::Int => {
                            self.line(&format!("r[{dst}] = (int64_t)(intptr_t)lc_chr(r[{a}]);"))
                        }
                        k => {
                            let sample =
                                sample_value(k).ok_or_else(|| format!("chr of a {k:?}"))?;
                            self.line(&format!(
                                "logos_fatalf({});",
                                words::builtin_refused(*builtin, vec![sample])
                            ));
                        }
                    },
                    // `parseInt(text)` — refused as the VM refuses anything but Text.
                    BuiltinId::ParseInt => match ak {
                        Kind::Text => {
                            self.line(&format!("r[{dst}] = lc_parse_int({});", as_cstr(a)))
                        }
                        k => {
                            let sample =
                                sample_value(k).ok_or_else(|| format!("parseInt of a {k:?}"))?;
                            self.line(&format!(
                                "logos_fatalf({});",
                                words::builtin_refused(*builtin, vec![sample])
                            ));
                        }
                    },
                    // `randomInt(lo, hi)`: a uniform draw from `lo..=hi`, refused in the VM's words when the bounds
                    // are crossed or are not integers.
                    BuiltinId::RandomInt => match (ak, kind_at(kinds, a + 1)) {
                        (Kind::Int, Kind::Int) => {
                            self.line(&format!("r[{dst}] = lc_random_int(r[{a}], r[{}]);", a + 1))
                        }
                        (x, y) => self.builtin_refusal(*builtin, &[x, y])?,
                    },
                    BuiltinId::RandomFloat => {
                        self.line(&format!("r[{dst}] = as_bits(lc_random_float());"))
                    }
                    // `get(name)`'s two halves, as the VM expands it: whether the process environment sets the
                    // variable, and its text (`""` when unset) — refused in the VM's words for a name that is
                    // not Text.
                    BuiltinId::EnvHas => match ak {
                        Kind::Text => {
                            self.line(&format!("r[{dst}] = (getenv({}) != NULL);", as_cstr(a)))
                        }
                        k => self.builtin_refusal(*builtin, &[k])?,
                    },
                    BuiltinId::EnvValue => match ak {
                        Kind::Text => self.line(&format!(
                            "r[{dst}] = (int64_t)(intptr_t)lc_env_value({});",
                            as_cstr(a)
                        )),
                        k => self.builtin_refusal(*builtin, &[k])?,
                    },
                    // `now()`: time.lg's Unix time in milliseconds.
                    BuiltinId::TimeNow => self.line(&format!("r[{dst}] = logos_time_now();")),
                    other => return Err(format!("CallBuiltin {other:?} not yet lowered")),
                }
            }
            // A tuple is a heap buffer of one i64 cell per position (the register value IS the cell — the
            // uniform i64 file means no per-cell conversion, exactly like a struct). Destructure reads them back.
            Op::NewTuple { dst, start, count } => {
                let t = self.fresh();
                let n = (*count).max(1); // lc_gc_calloc(0,…) is UB-adjacent; a 0-tuple still gets one cell (unused)
                self.line(&format!(
                    "int64_t *_tup{t} = (int64_t*)lc_gc_calloc({n}, sizeof(int64_t));"
                ));
                for i in 0..*count {
                    self.line(&format!("_tup{t}[{i}] = r[{}];", *start + i));
                }
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_tup{t};"));
            }
            Op::DestructureTuple { src, start, count } => {
                for i in 0..*count {
                    self.line(&format!(
                        "r[{}] = ((int64_t*)(intptr_t)r[{src}])[{i}];",
                        *start + i
                    ));
                }
            }
            // Bitwise / shifts are integer-domain (Bool 0/1 rides the same lowering) — never float, so the
            // shared `arith` int path is exactly right.
            Op::BitXor { dst, lhs, rhs }
                if self.exact_integers(&[*lhs, *rhs], kinds, Kind::Int) =>
            {
                self.emit_exact_bounded(*dst, *lhs, *rhs, kinds, Kind::Int, "lc_xbitxor")?;
            }
            Op::BitAnd { dst, lhs, rhs }
                if self.exact_integers(&[*lhs, *rhs], kinds, Kind::Int) =>
            {
                self.emit_exact_bounded(*dst, *lhs, *rhs, kinds, Kind::Int, "lc_xbitand")?;
            }
            Op::BitOr { dst, lhs, rhs } if self.exact_integers(&[*lhs, *rhs], kinds, Kind::Int) => {
                self.emit_exact_bounded(*dst, *lhs, *rhs, kinds, Kind::Int, "lc_xbitor")?;
            }
            Op::BitXor { dst, lhs, rhs } => self.arith(*dst, *lhs, *rhs, "^", kinds)?,
            Op::BitAnd { dst, lhs, rhs } => self.arith(*dst, *lhs, *rhs, "&", kinds)?,
            Op::BitOr { dst, lhs, rhs } => self.arith(*dst, *lhs, *rhs, "|", kinds)?,
            // Shifts mask the count to 6 bits (LOGOS `WordVal` semantics); a raw C shift by ≥64 is UB.
            // A word shifts by its count modulo its width (Rust's `wrapping_shl`/`wrapping_shr`), logically.
            Op::Shl { dst, lhs, rhs } | Op::Shr { dst, lhs, rhs }
                if matches!(kind_at(kinds, *lhs), Kind::Word(_)) =>
            {
                let Kind::Word(bits) = kind_at(kinds, *lhs) else {
                    unreachable!()
                };
                let mask = if bits == 64 {
                    "UINT64_MAX".to_string()
                } else {
                    format!("((1ULL << {bits}) - 1)")
                };
                let dir = if matches!(op, Op::Shl { .. }) {
                    "<<"
                } else {
                    ">>"
                };
                self.line(&format!("r[{dst}] = (int64_t)(((uint64_t)r[{lhs}] {dir} ((uint32_t)r[{rhs}] & {})) & {mask});", bits - 1));
            }
            Op::Shl { dst, lhs, rhs } | Op::Shr { dst, lhs, rhs }
                if kind_at(kinds, *lhs) == Kind::Rational || kind_at(kinds, *rhs) == Kind::Rational =>
            {
                let (l, r) = (as_ldec(*lhs, kind_at(kinds, *lhs))?, as_ldec(*rhs, kind_at(kinds, *rhs))?);
                let dir = if matches!(op, Op::Shl { .. }) { '<' } else { '>' };
                self.line(&format!("r[{dst}] = ldec_bitwise({l}, {r}, '{dir}');"));
            }
            Op::Shl { dst, lhs, rhs } if self.exact_integers(&[*lhs, *rhs], kinds, Kind::Int) => {
                self.emit_exact_bounded(*dst, *lhs, *rhs, kinds, Kind::Int, "lc_xshl")?;
            }
            Op::Shr { dst, lhs, rhs } if self.exact_integers(&[*lhs, *rhs], kinds, Kind::Int) => {
                self.emit_exact_bounded(*dst, *lhs, *rhs, kinds, Kind::Int, "lc_xshr")?;
            }
            Op::Shl { dst, lhs, rhs } => self.line(&format!(
                "r[{dst}] = (int64_t)((uint64_t)r[{lhs}] << (r[{rhs}] & 63));"
            )),
            Op::Shr { dst, lhs, rhs } => {
                self.line(&format!("r[{dst}] = r[{lhs}] >> (r[{rhs}] & 63);"))
            }
            // `is approximately` compares numbers; any other operand fails the run with the VM's refusal.
            Op::ApproxEq { dst, lhs, rhs } => {
                let (lk, rk) = (kind_at(kinds, *lhs), kind_at(kinds, *rhs));
                let numeric = |k: Kind| matches!(k, Kind::Int | Kind::Float);
                if numeric(lk) && numeric(rk) {
                    self.line(&format!(
                        "r[{dst}] = lc_approx_eq({}, {});",
                        as_double(*lhs, lk),
                        as_double(*rhs, rk)
                    ));
                } else {
                    let (Some(lv), Some(rv)) = (sample_value(lk), sample_value(rk)) else {
                        return Err(format!("`is approximately` over {lk:?} and {rk:?}"));
                    };
                    self.line(&format!("logos_fatalf({});", words::approx_refused(lv, rv)));
                }
            }
            // Strength-reduced constant division/modulo: bit-identical to `Div`/`Mod` by the constant, and
            // gcc -O2 re-derives the same magic-reciprocal code from the constant divisor (same speed). An integer
            // a number join holds as a rational divides as the exact number it is.
            Op::DivPow2 { .. } | Op::MagicDivU { .. } if crate::vm::tower::by_constant(op).is_some_and(|(_, lhs, _, _)| kind_at(kinds, lhs) == Kind::Rational) => {
                let (dst, lhs, divisor, f) = crate::vm::tower::by_constant(op).expect("a division by a constant");
                let f = if f == "div" { "ldec_quotient" } else { "ldec_remainder" };
                let l = as_ldec(lhs, Kind::Rational)?;
                self.box_ldec(dst, &format!("{f}({l}, ldec_from_i64((int64_t){divisor}LL))"));
            }
            Op::DivPow2 { dst, lhs, k } if kind_at(kinds, *lhs) == Kind::BigInt => {
                let q = format!("lc_xdiv(r[{lhs}], lc_xof((int64_t)1 << {k}))");
                self.line(&format!(
                    "r[{dst}] = {};",
                    if kind_at(after, *dst) == Kind::BigInt {
                        q
                    } else {
                        format!("lc_xnarrow({q})")
                    }
                ));
            }
            Op::MagicDivU {
                dst,
                lhs,
                mul_back,
                divisor,
                ..
            } if kind_at(kinds, *lhs) == Kind::BigInt => {
                let f = if *mul_back == 0 { "lc_xdiv" } else { "lc_xmod" };
                let v = format!("{f}(r[{lhs}], lc_xof({divisor}LL))");
                self.line(&format!(
                    "r[{dst}] = {};",
                    if kind_at(after, *dst) == Kind::BigInt {
                        v
                    } else {
                        format!("lc_xnarrow({v})")
                    }
                ));
            }
            Op::DivPow2 { dst, lhs, k } => {
                self.line(&format!("r[{dst}] = r[{lhs}] / (1LL << {k});"))
            }
            Op::MagicDivU {
                dst,
                lhs,
                mul_back,
                divisor,
                wrap,
                ..
            } => {
                if *mul_back == 0 {
                    self.line(&format!("r[{dst}] = r[{lhs}] / {divisor}LL;"));
                } else if *wrap {
                    crate::optimize::mark_fired(crate::optimization::Opt::ModWrap);
                    self.line(&format!(
                        "r[{dst}] = r[{lhs}] >= {divisor}LL ? r[{lhs}] - {divisor}LL : r[{lhs}];"
                    ));
                } else {
                    self.line(&format!("r[{dst}] = r[{lhs}] % {divisor}LL;"));
                }
            }
            Op::Concat { dst, lhs, rhs } => {
                // `combined with` / interpolation coerces a non-text operand to its `Show` rendering first.
                let l = self.text_of(*lhs, kind_at(kinds, *lhs))?;
                let r = self.text_of(*rhs, kind_at(kinds, *rhs))?;
                self.line(&format!(
                    "r[{dst}] = (int64_t)(intptr_t)str_concat({l}, {r});"
                ));
            }
            // Interpolation `{x}` / `Show x` debug — render a value to text by its `Show` form. An explicit
            // format spec (width/precision) is refused for now; the bare + debug-prefix forms are the common ones.
            Op::FormatValue {
                dst,
                src,
                spec,
                debug_prefix,
            } => {
                let sk = kind_at(kinds, *src);
                // With an explicit spec, format via the runtime exactly as the AST backend's `c_format_spec_for_hole`
                // (width/precision/alignment/`$`money/`^`center); with none, coerce to text as the VM does.
                let body = if *spec != u32::MAX {
                    let spec_str = match &self.prog.constants[*spec as usize] {
                        Constant::Text(s) => s.clone(),
                        _ => return Err("FormatValue spec is not Text".into()),
                    };
                    self.format_spec_body(*src, sk, &spec_str)?
                } else {
                    self.text_of(*src, sk)?
                };
                if *debug_prefix != u32::MAX {
                    let prefix = match &self.prog.constants[*debug_prefix as usize] {
                        Constant::Text(p) => c_escape(p),
                        _ => return Err("FormatValue debug prefix is not Text".into()),
                    };
                    self.line(&format!(
                        "r[{dst}] = (int64_t)(intptr_t)str_concat(\"{prefix}=\", {body});"
                    ));
                } else {
                    self.line(&format!("r[{dst}] = (int64_t)(intptr_t)({body});"));
                }
            }
            Op::Lt { dst, lhs, rhs } => self.cmp(*dst, *lhs, *rhs, "<", kinds)?,
            Op::Gt { dst, lhs, rhs } => self.cmp(*dst, *lhs, *rhs, ">", kinds)?,
            Op::LtEq { dst, lhs, rhs } => self.cmp(*dst, *lhs, *rhs, "<=", kinds)?,
            Op::GtEq { dst, lhs, rhs } => self.cmp(*dst, *lhs, *rhs, ">=", kinds)?,
            // `is equal to` / `is not equal to` — the VM's structural equality ([`Emitter::eq_expr`]).
            Op::Eq { dst, lhs, rhs } | Op::NotEq { dst, lhs, rhs } => {
                let (lk, rk) = (kind_at(kinds, *lhs), kind_at(kinds, *rhs));
                let scalar = |k: Kind| {
                    matches!(
                        k,
                        Kind::Int | Kind::Bool | Kind::Float | Kind::BigInt | Kind::Rational
                    )
                };
                let not = if matches!(op, Op::NotEq { .. }) {
                    "!"
                } else {
                    ""
                };
                if scalar(lk) && scalar(rk) && !(lk == Kind::Bool) == !(rk == Kind::Bool) {
                    self.cmp(
                        *dst,
                        *lhs,
                        *rhs,
                        if not.is_empty() { "==" } else { "!=" },
                        kinds,
                    )?;
                } else {
                    let same = self.eq_expr(&format!("r[{lhs}]"), lk, &format!("r[{rhs}]"), rk)?;
                    self.line(&format!("r[{dst}] = {not}({same});"));
                }
            }
            // `not x`: on a Bool it's logical negation; on a Text it's the emptiness test (an empty string is
            // falsy in LOGOS, so `not ""` is true).
            Op::Not { dst, src } => {
                if kind_at(kinds, *src) == Kind::Text {
                    self.line(&format!("r[{dst}] = (str_len({}) == 0);", as_cstr(*src)));
                } else {
                    self.line(&format!("r[{dst}] = !r[{src}];"));
                }
            }
            Op::Jump { target } => self.line(&format!("goto {}{target};", self.label)),
            Op::JumpIfFalse { cond, target } => {
                self.line(&format!("if (!r[{cond}]) goto {}{target};", self.label))
            }
            Op::JumpIfTrue { cond, target } => {
                self.line(&format!("if (r[{cond}]) goto {}{target};", self.label))
            }
            // A collection argument passes as its handle: the callee's registers start with clear owned flags, so
            // its first change to a parameter copies it (value semantics), while a `mutable` parameter — the
            // caller's collection — changes in place. A `mutable` parameter passes its argument register itself,
            // which holds the parameter's final value when the call returns, as the VM leaves it.
            Op::Call {
                dst,
                func,
                args_start,
                arg_count,
            } => {
                let params = self
                    .param_kinds
                    .get(*func as usize)
                    .cloned()
                    .unwrap_or_default();
                let inout = self.inout_regs(*func as usize);
                // On machine words, a function with a word entry takes its exact parameters as the machine words
                // they hold here, and an exact result it escapes to resumes the region held exact at this call: an
                // exact result arrives on machine words as a word, so the call is one of the region's arrivals.
                let exact_result = self.fn_ret.get(*func as usize) == Some(&Kind::BigInt);
                let twin = self.on_words.get()
                    && self.word_twins.contains(&(*func as usize))
                    && (0..*arg_count).all(|i| {
                        params.get(i as usize) != Some(&Kind::BigInt)
                            || matches!(kind_at(kinds, args_start + i), Kind::Int | Kind::Bool)
                    });
                let mut args = Vec::with_capacity(*arg_count as usize);
                for i in 0..*arg_count {
                    let reg = args_start + i;
                    // An exact parameter handed a machine integer receives it as an exact integer.
                    let widen = !twin
                        && matches!(
                            (params.get(i as usize), kind_at(kinds, reg)),
                            (Some(Kind::BigInt), Kind::Int | Kind::Bool)
                        );
                    if inout.contains(&i) {
                        if widen {
                            self.line(&format!("r[{reg}] = lc_xof(r[{reg}]);"));
                        }
                        args.push(format!("&r[{reg}]"));
                    } else if widen {
                        args.push(format!("lc_xof(r[{reg}])"));
                    } else if params.get(i as usize) == Some(&Kind::Tower)
                        && kind_at(kinds, reg) != Kind::Tower
                    {
                        // An exact parameter takes the runtime's value of what the caller holds.
                        let cell = self.tower_of(&format!("r[{reg}]"), kind_at(kinds, reg))?;
                        args.push(format!("(int64_t)(intptr_t)({cell})"));
                    } else if let Some(&to) = params
                        .get(i as usize)
                        .filter(|to| self.widens_to(kind_at(kinds, reg), **to))
                    {
                        // A Rational parameter handed an integer takes the rational it equals.
                        args.push(self.convert_number(&format!("r[{reg}]"), kind_at(kinds, reg), to)?);
                    } else if params.get(i as usize) == Some(&Kind::Float)
                        && matches!(kind_at(kinds, reg), Kind::Int | Kind::BigInt)
                    {
                        // A Real parameter handed an integer takes it widened to the nearest double, as the VM widens it.
                        args.push(format!("as_bits({})", as_double(reg, kind_at(kinds, reg))));
                    } else {
                        args.push(format!("r[{reg}]"));
                    }
                }
                let ret_own = if self.owned.contains(dst)
                    && crate::optimize::active_config().is_on(crate::optimization::Opt::Borrow)
                {
                    crate::optimize::mark_fired(crate::optimization::Opt::Borrow);
                    format!("&own[{dst}]")
                } else {
                    "NULL".to_string()
                };
                args.push(ret_own);
                if twin && exact_result {
                    let n = self.fresh();
                    self.line(&format!(
                        "{{ lc_wret _w{n} = fn_{func}_w({}); r[{dst}] = _w{n}.v; if (_w{n}.escaped) goto A{pc}; }}",
                        args.join(", ")
                    ));
                    self.twin_arrival = true;
                } else if twin {
                    self.line(&format!("r[{dst}] = fn_{func}_w({}).v;", args.join(", ")));
                } else {
                    self.line(&format!("r[{dst}] = fn_{func}({});", args.join(", ")));
                }
            }
            // Build a closure object `[body_index, captured values…, present flags…]`: a local capture snapshots
            // the register at the build site, a global capture the module global (present=1, assumed defined).
            Op::MakeClosure {
                dst,
                func,
                locals_start,
            } => {
                let caps = self.prog.functions[*func as usize].captures.clone();
                let ncap = caps.len();
                let n = self.fresh();
                self.line(&format!(
                    "int64_t *_clo{n} = (int64_t*)lc_gc_calloc({}, sizeof(int64_t));",
                    1 + 2 * ncap
                ));
                self.line(&format!("_clo{n}[0] = {func};"));
                let mut local_k: u16 = 0;
                for (i, (_sym, gidx)) in caps.iter().enumerate() {
                    match gidx {
                        Some(g) => {
                            self.line(&format!("_clo{n}[{}] = g[{}];", 1 + i, g));
                            self.line(&format!("_clo{n}[{}] = 1;", 1 + ncap + i));
                        }
                        None => {
                            self.line(&format!(
                                "_clo{n}[{}] = r[{}];",
                                1 + i,
                                locals_start + local_k
                            ));
                            self.line(&format!("_clo{n}[{}] = 1;", 1 + ncap + i));
                            local_k += 1;
                        }
                    }
                }
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_clo{n};"));
            }
            // Call a closure value: dispatch on its runtime body index, passing the args then the closure pointer
            // as the hidden trailing arg (the layout each closure-body `fn_*` expects).
            Op::CallValue {
                dst,
                callee,
                args_start,
                arg_count,
                ..
            } => {
                let bodies: Vec<usize> = self.closure_bodies.iter().copied().collect();
                if bodies.is_empty() {
                    return Err("CallValue but the program builds no closures".into());
                }
                let arglist: String = (0..*arg_count)
                    .map(|k| format!("r[{}], ", args_start + k))
                    .collect();
                let ret_own = if self.owned.contains(dst)
                    && crate::optimize::active_config().is_on(crate::optimization::Opt::Borrow)
                {
                    crate::optimize::mark_fired(crate::optimization::Opt::Borrow);
                    format!("&own[{dst}]")
                } else {
                    "NULL".to_string()
                };
                self.line(&format!(
                    "switch ((int)((int64_t*)(intptr_t)r[{callee}])[0]) {{"
                ));
                for b in bodies {
                    self.line(&format!(
                        "case {b}: r[{dst}] = fn_{b}({arglist}r[{callee}], {ret_own}); break;"
                    ));
                }
                self.line("default: fprintf(stderr, \"CallValue: unknown closure body\\n\"); fflush(stdout); exit(1);");
                self.line("}");
            }
            // ── Collections ──────────────────────────────────────────────────────────────────────────────
            Op::NewRange { dst, start, end } => {
                let t = self.fresh();
                self.line(&format!(
                    "Seq_i64 *_s{t} = (Seq_i64*)lc_gc_alloc(sizeof(Seq_i64)); *_s{t} = seq_i64_new();"
                ));
                let (from, to) = (
                    self.int_operand(kinds, *start, words::IntegerPlace::Range),
                    self.int_operand(kinds, *end, words::IntegerPlace::Range),
                );
                self.line(&format!("for (int64_t _i{t} = {from}, _e{t} = {to}; _i{t} <= _e{t}; _i{t}++) seq_i64_push(_s{t}, _i{t});"));
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_s{t};"));
            }
            Op::NewList { dst, start, count } => {
                // An empty `[]` carries no element kind here — take the fixpoint-refined kind if a later push
                // established one, else box an empty `Seq_i64` (an empty list renders `[]` for any element type).
                let seq_kind = if *count == 0 {
                    let k = kind_at(after, *dst);
                    if is_concrete_seq(k) {
                        k
                    } else {
                        Kind::SeqInt
                    }
                } else {
                    self.list_kind(&(0..*count).map(|i| kind_at(kinds, start + i)).collect::<Vec<_>>())?
                };
                let ty = seq_kind
                    .seq_c_type()
                    .ok_or_else(|| format!("NewList of a non-scalar element kind {seq_kind:?}"))?;
                let infix = seq_kind.seq_infix().unwrap();
                let elem = self.elem_kind(seq_kind);
                let t = self.fresh();
                self.line(&format!(
                    "{ty} *_s{t} = ({ty}*)lc_gc_alloc(sizeof({ty})); *_s{t} = seq_{infix}_new();"
                ));
                for k in 0..*count {
                    let item = self.elem_value(start + k, kind_at(kinds, start + k), elem)?;
                    self.line(&format!("seq_{infix}_push(_s{t}, {item});"));
                }
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_s{t};"));
            }
            // An empty sequence: allocate the fixpoint-refined typed `Seq_*`; a never-populated one that stays
            // `SeqUnknown` allocates a `Seq_i64` (an empty list renders `[]` regardless of element type).
            Op::NewEmptyListFixed { dst, capacity } => {
                let sk = match kind_at(after, *dst) {
                    k if is_concrete_seq(k) => k,
                    _ => Kind::SeqInt,
                };
                let ty = sk
                    .seq_c_type()
                    .ok_or_else(|| format!("fixed list of non-sequence kind {sk:?}"))?;
                let elem = match sk {
                    Kind::SeqFloat => "double",
                    Kind::SeqText => "char *",
                    Kind::SeqBool => "bool",
                    _ => "int64_t",
                };
                let t = self.fresh();
                self.line(&format!(
                    "{elem} _fxd{t}[{capacity}] = {{0}}; /* shared fixed Seq */"
                ));
                self.line(&format!("{ty} _fx{t} = ({ty}){{_fxd{t}, 0, {capacity}}};"));
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)&_fx{t};"));
                if self.owned.contains(dst) {
                    self.line(&format!("own[{dst}] = 1;"));
                }
                crate::optimize::mark_fired(crate::optimization::Opt::Scalarize);
            }
            Op::NewEmptyListWithCapacity { dst, capacity } => {
                let sk = {
                    let k = kind_at(after, *dst);
                    if is_concrete_seq(k) {
                        k
                    } else {
                        Kind::SeqInt
                    }
                };
                let ty = sk.seq_c_type().unwrap();
                let infix = sk.seq_infix().unwrap();
                let t = self.fresh();
                let reuses = self.reuses_list_buffer(*dst, sk);
                if reuses {
                    self.line(&format!("if (own[{dst}] && r[{dst}] != 0) {{"));
                    self.line(&format!("{ty} *_s{t} = ({ty}*)(intptr_t)r[{dst}];"));
                    self.line(&format!("_s{t}->len = 0;"));
                    self.line(&format!(
                        "if (r[{capacity}] > 0 && (uint64_t)r[{capacity}] > _s{t}->cap) {{"
                    ));
                    self.line(&format!("_s{t}->data = lc_gc_realloc(_s{t}->data, (size_t)r[{capacity}] * sizeof(*_s{t}->data));"));
                    self.line(&format!("_s{t}->cap = (size_t)r[{capacity}];"));
                    self.line("}");
                    self.line("} else {");
                }
                self.line(&format!("{ty} *_s{t} = ({ty}*)lc_gc_alloc(sizeof({ty})); *_s{t} = seq_{infix}_with_capacity(r[{capacity}]);"));
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_s{t};"));
                if reuses {
                    self.line("}");
                }
            }
            Op::NewEmptyList { dst } | Op::NewEmptyListI32 { dst } => {
                let sk = {
                    let k = kind_at(after, *dst);
                    if is_concrete_seq(k) {
                        k
                    } else {
                        Kind::SeqInt
                    }
                };
                let ty = sk.seq_c_type().unwrap();
                let infix = sk.seq_infix().unwrap();
                let t = self.fresh();
                let reuses = self.reuses_list_buffer(*dst, sk);
                if reuses {
                    self.line(&format!("if (own[{dst}] && r[{dst}] != 0) {{"));
                    self.line("/* reuse sole-owned list buffer */");
                    self.line(&format!("(({ty}*)(intptr_t)r[{dst}])->len = 0;"));
                    self.line("} else {");
                }
                self.line(&format!(
                    "{ty} *_s{t} = ({ty}*)lc_gc_alloc(sizeof({ty})); *_s{t} = seq_{infix}_new();"
                ));
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_s{t};"));
                if reuses {
                    self.line("}");
                }
            }
            Op::ListPop { list, dst } => {
                // The list has an item to give: `resolve_pop` stopped the program with the VM's sentence before any
                // pop that could find none.
                let lk = seq_kind_of(kind_at(kinds, *list));
                let infix = lk
                    .seq_infix()
                    .ok_or_else(|| format!("Pop from a non-sequence kind {lk:?}"))?;
                self.own_target(*list, lk)?;
                let ptr = Self::seq_ptr(*list, lk)?;
                let read = elem_to_reg(&format!("seq_{infix}_pop({ptr})"), self.elem_kind(lk));
                self.line(&format!("r[{dst}] = {read};"));
            }
            // ── Sets (heap-boxed insertion-ordered dedup'd Set_i64/Set_str) ──────────────────────────────────
            Op::NewEmptySet { dst } => {
                // The fixpoint-refined element kind (an unpopulated set stays `SetUnknown` → box a `Set_i64`,
                // which renders `{}` like any empty set).
                let sk = {
                    let k = kind_at(after, *dst);
                    if is_concrete_set(k) {
                        k
                    } else {
                        Kind::SetInt
                    }
                };
                let ty = sk.set_c_type().unwrap();
                let infix = sk.seq_infix().unwrap();
                let t = self.fresh();
                self.line(&format!(
                    "{ty} *_s{t} = ({ty}*)lc_gc_alloc(sizeof({ty})); *_s{t} = set_{infix}_new();"
                ));
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_s{t};"));
            }
            Op::NewEmptyMap { dst } => {
                // Box the (fixpoint-refined) map struct; an unrefined empty map boxes a `Map_i64_i64` (renders `{}`).
                let (key, val) = match kind_at(after, *dst) {
                    Kind::Map(idx) => self.map_pair(idx),
                    _ => (Kind::Int, Kind::Int),
                };
                let (ty, pre) = (map_type_name(key, val).unwrap(), map_fn(key, val).unwrap());
                let t = self.fresh();
                self.line(&format!(
                    "{ty} *_m{t} = ({ty}*)lc_gc_alloc(sizeof({ty})); *_m{t} = {pre}_new();"
                ));
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_m{t};"));
            }
            Op::NewEmptyMapWithCapacity { dst, capacity } => {
                let (key, val) = match kind_at(after, *dst) {
                    Kind::Map(idx) => self.map_pair(idx),
                    _ => (Kind::Int, Kind::Int),
                };
                let (ty, pre) = (map_type_name(key, val).unwrap(), map_fn(key, val).unwrap());
                let t = self.fresh();
                self.line(&format!("{ty} *_m{t} = ({ty}*)lc_gc_alloc(sizeof({ty})); *_m{t} = {pre}_with_capacity(r[{capacity}]);"));
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_m{t};"));
            }
            Op::SetAdd { set, value } => {
                let sk = kind_at(kinds, *value).set_of();
                let infix = sk
                    .seq_infix()
                    .ok_or_else(|| format!("SetAdd of a non-scalar element kind {sk:?}"))?;
                self.own_target(*set, sk)?;
                let ptr = Self::set_ptr(*set, sk)?;
                self.line(&format!(
                    "set_{infix}_add({ptr}, {});",
                    as_elem_push(*value, sk.elem())
                ));
            }
            Op::RemoveFrom { collection, value } => {
                let ck = kind_at(kinds, *collection);
                let sk = resolve_set(ck)
                    .ok_or_else(|| format!("RemoveFrom on a non-set kind {ck:?}"))?;
                let infix = sk.seq_infix().unwrap();
                self.own_target(*collection, sk)?;
                let ptr = Self::set_ptr(*collection, sk)?;
                self.line(&format!(
                    "set_{infix}_remove({ptr}, {});",
                    as_elem_push(*value, sk.elem())
                ));
            }
            Op::UnionOp { dst, lhs, rhs } => {
                let sk = resolve_set(kind_at(kinds, *lhs))
                    .ok_or_else(|| format!("Union on a non-set kind {:?}", kind_at(kinds, *lhs)))?;
                let (ty, infix) = (sk.set_c_type().unwrap(), sk.seq_infix().unwrap());
                let (lp, rp) = (Self::set_ptr(*lhs, sk)?, Self::set_ptr(*rhs, sk)?);
                let t = self.fresh();
                self.line(&format!("{ty} *_su{t} = ({ty}*)lc_gc_alloc(sizeof({ty})); *_su{t} = set_{infix}_union({lp}, {rp});"));
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_su{t};"));
            }
            Op::IntersectOp { dst, lhs, rhs } => {
                let sk = resolve_set(kind_at(kinds, *lhs)).ok_or_else(|| {
                    format!("Intersect on a non-set kind {:?}", kind_at(kinds, *lhs))
                })?;
                let (ty, infix) = (sk.set_c_type().unwrap(), sk.seq_infix().unwrap());
                let (lp, rp) = (Self::set_ptr(*lhs, sk)?, Self::set_ptr(*rhs, sk)?);
                let t = self.fresh();
                self.line(&format!("{ty} *_si{t} = ({ty}*)lc_gc_alloc(sizeof({ty})); *_si{t} = set_{infix}_intersect({lp}, {rp});"));
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_si{t};"));
            }
            // ── Sequence slice / element-set / concat ────────────────────────────────────────────────────────
            Op::SliceOp {
                dst,
                collection,
                start,
                end,
            } => {
                let ck = seq_kind_of(kind_at(kinds, *collection));
                let ty = ck
                    .seq_c_type()
                    .ok_or_else(|| format!("Slice of a non-sequence kind {ck:?}"))?;
                let infix = ck.seq_infix().unwrap();
                let ptr = Self::seq_ptr(*collection, ck)?;
                let t = self.fresh();
                // `items X through Y` is 1-based inclusive; an out-of-range slice (`Y > len` or `X-1 >= Y`) is
                // EMPTY (the VM's `slice.get(start..end)` semantics), not clamped-to-available.
                let (from, to) = (
                    self.int_operand(kinds, *start, words::IntegerPlace::Slice),
                    self.int_operand(kinds, *end, words::IntegerPlace::Slice),
                );
                self.line(&format!("{ty} *_sl{t} = ({ty}*)lc_gc_alloc(sizeof({ty}));"));
                self.line(&format!("{{ int64_t _sf{t} = {from}, _st{t} = {to}; if (_sf{t} - 1 < _st{t} && _st{t} <= (int64_t){ptr}->len) *_sl{t} = seq_{infix}_slice({ptr}, _sf{t}, _st{t}); else *_sl{t} = seq_{infix}_new(); }}"));
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_sl{t};"));
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
                let unchecked = matches!(op, Op::SetIndexUnchecked { .. });
                let ck = kind_at(kinds, *collection);
                if matches!(ck, Kind::Struct(_) | Kind::StructSynth(_)) {
                    // `obj["field"] = v` — a field store by name (the take-out/put-back idiom around a change
                    // to a collection that lives in a field).
                    let name = self
                        .const_text_at(self.region_span.0, pc, *index)
                        .ok_or("a record indexed by something other than a field name")?;
                    let cell = self
                        .field_cell_of_name(ck, &name)
                        .ok_or_else(|| format!("a store to the unknown field `{name}`"))?;
                    self.store_field(*collection, ck, cell, *value, kind_at(kinds, *value), pc)?;
                } else if matches!(ck, Kind::Map(_) | Kind::MapUnknown) {
                    // `item k of m to v` — a map insert/update. Take the map type from THIS insert's actual
                    // key/value kinds (the value rides its i64/str cell — ANY kind), and REFUSE an unhashable
                    // key — never silently coerce it.
                    let (input_key, given) = (kind_at(kinds, *index), kind_at(kinds, *value));
                    let vk = match ck {
                        Kind::Map(mi) if self.map_pair(mi).1 == Kind::Tower => Kind::Tower,
                        _ => given,
                    };
                    let kk = if input_key == Kind::BigInt {
                        Kind::Int
                    } else {
                        input_key
                    };
                    if map_key_storage(kk).is_none() || map_val_storage(vk).is_none() {
                        return Err(format!(
                            "map insert with an unsupported key/value kind ({kk:?} -> {vk:?})"
                        ));
                    }
                    self.own_target(*collection, self.map_of(kk, vk))?;
                    let (pre, ptr) = (
                        map_fn(kk, vk).unwrap(),
                        map_ptr_kv(*collection, kk, vk).unwrap(),
                    );
                    // The key and value can name the same register. Narrow the
                    // key expression only; retain the exact value for its cell.
                    let key = if input_key == Kind::BigInt {
                        format!("lc_xnarrow(r[{index}])")
                    } else {
                        map_key_arg(*index, kk)
                    };
                    let val = if vk == Kind::Tower && given != Kind::Tower {
                        let cell = self.tower_of(&format!("r[{value}]"), given)?;
                        format!("(int64_t)(intptr_t)({cell})")
                    } else {
                        map_val_store(*value, vk)
                    };
                    self.line(&format!("{pre}_set({ptr}, {key}, {val});"));
                } else {
                    let sk = seq_kind_of(ck);
                    sk.seq_infix()
                        .ok_or_else(|| format!("SetIndex on a non-sequence kind {ck:?}"))?;
                    self.own_target(*collection, sk)?;
                    let ptr = Self::seq_ptr(*collection, sk)?;
                    let index_word = self.int_operand(kinds, *index, words::IntegerPlace::ListStore);
                    let at = if unchecked {
                        crate::optimize::mark_fired(crate::optimization::Opt::Unchecked);
                        format!("(size_t)({index_word} - 1LL)")
                    } else {
                        format!("logos_idx0({index_word}, {ptr}->len)")
                    };
                    let item = self.elem_value(*value, kind_at(kinds, *value), self.elem_kind(sk))?;
                    self.line(&format!("{ptr}->data[{at}] = {item};"));
                }
            }
            Op::SeqConcat { dst, lhs, rhs } => {
                let (lk, rk) = (kind_at(kinds, *lhs), kind_at(kinds, *rhs));
                // Two sequences meet at the element kind either declares; one made empty takes the other's.
                let lk = seq_kind_of(if lk == Kind::SeqUnknown { rk } else { lk });
                let ty = lk
                    .seq_c_type()
                    .ok_or_else(|| format!("SeqConcat of a non-sequence kind {lk:?}"))?;
                let infix = lk.seq_infix().unwrap();
                let (lp, rp) = (Self::seq_ptr(*lhs, lk)?, Self::seq_ptr(*rhs, lk)?);
                let t = self.fresh();
                self.line(&format!(
                    "{ty} *_sc{t} = ({ty}*)lc_gc_alloc(sizeof({ty})); *_sc{t} = seq_{infix}_new();"
                ));
                self.line(&format!("for (size_t _a{t}=0; _a{t}<{lp}->len; _a{t}++) seq_{infix}_push(_sc{t}, {lp}->data[_a{t}]);"));
                self.line(&format!("for (size_t _b{t}=0; _b{t}<{rp}->len; _b{t}++) seq_{infix}_push(_sc{t}, {rp}->data[_b{t}]);"));
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_sc{t};"));
            }
            // A copy-on-write ownership barrier: a mutable collection aliased through the register file must be
            // cloned so an in-place mutation cannot leak back to an alias (matching the VM's `ensure_reg_owned`).
            // Immutable values (scalars, Text, Enums) need no copy.
            Op::EnsureOwned { reg } => self.own_target(*reg, kind_at(kinds, *reg))?,
            // ── Structs (a heap buffer of one i64 cell per field; the uniform register value IS the cell) ────
            Op::NewStruct { dst, type_name } => {
                let sk = self.struct_kind_of_type_name(*type_name);
                // One cell per field, then the field-ownership word ([`Emitter::field_owner_word`]).
                let n = self
                    .struct_cell_count(sk)
                    .ok_or("NewStruct of an unknown struct type")?
                    + 1;
                let t = self.fresh();
                // An exact field starts at nothing, as the VM's does: a cell of the runtime's nothing, not a null one.
                let exact_cells: String = match sk {
                    Kind::Struct(si) => self.prog.struct_types[si as usize]
                        .fields
                        .iter()
                        .enumerate()
                        .filter(|(_, (_, ty))| self.boundary_kind(&Some(ty.clone())) == Kind::Tower)
                        .map(|(i, _)| format!(" _st{t}[{i}] = (int64_t)(intptr_t)ltower_nothing();"))
                        .collect(),
                    _ => String::new(),
                };
                self.line(&format!(
                    "{{ int64_t *_st{t} = (int64_t*)lc_gc_calloc({n}, sizeof(int64_t));{exact_cells} r[{dst}] = (int64_t)(intptr_t)_st{t}; }}"
                ));
            }
            Op::StructInsert { obj, field, value } => {
                let ok = kind_at(kinds, *obj);
                if !matches!(ok, Kind::Struct(_) | Kind::StructSynth(_)) {
                    return Err(format!("StructInsert on a non-struct kind {ok:?}"));
                }
                let (cell, _) = self
                    .resolve_field_any(ok, *field)
                    .ok_or("StructInsert of an unknown field")?;
                self.store_field(*obj, ok, cell, *value, kind_at(kinds, *value), pc)?;
            }
            Op::GetField { dst, obj, field } => {
                let ok = kind_at(kinds, *obj);
                if !matches!(ok, Kind::Struct(_) | Kind::StructSynth(_)) {
                    return Err(format!("GetField on a non-struct kind {ok:?}"));
                }
                let (cell, _) = self
                    .resolve_field_any(ok, *field)
                    .ok_or("GetField of an unknown field")?;
                // A counter-CRDT field holds a `logos_crdt*` (cast to the `int64_t` cell); reading it yields the
                // SUM (an Int), exactly as the VM renders the counter's total. A plain field reads the cell verbatim.
                let is_counter = self
                    .struct_name_of_kind(ok)
                    .map(|n| self.counter_fields.contains(&(n, cell)))
                    .unwrap_or(false);
                if is_counter {
                    self.line(&format!(
                        "r[{dst}] = logos_crdt_sum((logos_crdt*)(intptr_t)((int64_t*)(intptr_t)r[{obj}])[{cell}]);"
                    ));
                } else {
                    self.line(&format!(
                        "r[{dst}] = ((int64_t*)(intptr_t)r[{obj}])[{cell}];"
                    ));
                    self.read_field_collection(*dst, *obj, ok, cell, pc);
                }
            }
            // ── Enums (a heap buffer `[variant-tag, payload…]`; the uniform register value IS each cell) ──────
            // A declared enum tags a variant by its declaration index; an undeclared one (`Option`, `Result`, …)
            // by its constructor's place in the program-wide order (`ctor_tags`).
            Op::NewInductive {
                dst,
                type_name,
                ctor,
                args_start,
                count,
            } => {
                let tag = match self.enum_index_of_type_name(*type_name) {
                    Some(ei) => self
                        .variant_index(ei, *ctor)
                        .ok_or("NewInductive of an unknown variant")?,
                    None => {
                        let (ty, c) = (self.const_text(*type_name), self.const_text(*ctor));
                        let (Some(ty), Some(c)) = (ty, c) else {
                            return Err("NewInductive with a non-Text name".into());
                        };
                        self.ctor_tag(ty, c)
                            .ok_or_else(|| format!("the constructor {ty}::{c} has no tag"))?
                    }
                };
                let n = *count as usize + 1;
                let t = self.fresh();
                if crate::optimize::active_config().is_on(crate::optimization::Opt::Scalarize) {
                    self.line(&format!("int64_t *_en{t} = lc_enum_alloc({n});"));
                    crate::optimize::mark_fired(crate::optimization::Opt::Scalarize);
                } else {
                    self.line(&format!(
                        "int64_t *_en{t} = (int64_t*)lc_gc_calloc({n}, sizeof(int64_t));"
                    ));
                }
                self.line(&format!("_en{t}[0] = {tag};"));
                for k in 0..*count {
                    self.line(&format!("_en{t}[{}] = r[{}];", k + 1, args_start + k));
                }
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_en{t};"));
            }
            // `When V …` — does the value answer to constructor `V`? As the VM decides it: an enum by its
            // constructor, a record by its type's name, `nothing` only a `None` arm, anything else no arm.
            Op::TestArm {
                dst,
                target,
                variant,
            } => {
                let name = self
                    .const_text(*variant)
                    .ok_or("TestArm of a non-Text variant")?
                    .to_string();
                let tk = kind_at(kinds, *target);
                let test = match tk {
                    Kind::Enum(ei) => match self.variant_index(ei, *variant) {
                        Some(vi) => format!("(((int64_t*)(intptr_t)r[{target}])[0] == {vi})"),
                        None => "0".to_string(),
                    },
                    Kind::Inductive(ii) => match self.ctor_tag(&self.inductive(ii).0, &name) {
                        Some(tag) => format!("(((int64_t*)(intptr_t)r[{target}])[0] == {tag})"),
                        None => "0".to_string(),
                    },
                    Kind::Struct(_) | Kind::StructSynth(_) => {
                        ((self.struct_name_of_kind(tk).as_deref() == Some(name.as_str())) as i32)
                            .to_string()
                    }
                    Kind::Nothing => ((name == "None") as i32).to_string(),
                    Kind::Unknown => return Err(format!("TestArm on a non-enum kind {tk:?}")),
                    _ => "0".to_string(),
                };
                self.line(&format!("r[{dst}] = {test};"));
            }
            // The bound payload of the arm just matched: an enum's payload cell, a record's field (or, for a
            // `Some` arm over a record, the record itself); `nothing` binds nothing.
            Op::BindArm {
                dst,
                target,
                field,
                index,
            } => {
                let tk = kind_at(kinds, *target);
                match tk {
                    Kind::Enum(_) | Kind::Inductive(_) => {
                        self.line(&format!(
                            "r[{dst}] = ((int64_t*)(intptr_t)r[{target}])[{}];",
                            *index as usize + 1
                        ));
                    }
                    Kind::Struct(_) | Kind::StructSynth(_) => {
                        let fname = self
                            .const_text(*field)
                            .ok_or("BindArm of a non-Text field")?;
                        if fname == crate::vm::SOME_BINDING_MARKER {
                            self.line(&format!("r[{dst}] = r[{target}];"));
                        } else {
                            let cell = self.field_cell_of_name(tk, fname).ok_or_else(|| {
                                format!("an arm binds the field `{fname}` its record does not have")
                            })?;
                            self.line(&format!(
                                "r[{dst}] = ((int64_t*)(intptr_t)r[{target}])[{cell}];"
                            ));
                        }
                    }
                    Kind::Nothing => {}
                    Kind::Unknown => return Err(format!("BindArm on a non-enum kind {tk:?}")),
                    _ if *index == 0 => self.line(&format!("r[{dst}] = r[{target}];")),
                    _ => {}
                }
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
                let unchecked = matches!(op, Op::IndexUnchecked { .. });
                let ck = kind_at(kinds, *collection);
                if let Some((kk, vk)) = self.resolve_map_kv(ck) {
                    // `item k of m` — a map lookup; an absent key fails the run in the VM's words. The value
                    // rides its cell as any kind: a Text as the returned `char*`, everything else as its raw
                    // i64 register encoding.
                    let (pre, ptr) = (
                        map_fn(kk, vk).unwrap(),
                        map_ptr_kv(*collection, kk, vk).unwrap(),
                    );
                    let key = map_key_arg(*index, kk);
                    let shown = match kk {
                        Kind::Text => key.clone(),
                        _ => format!("logos_dyn_sprintf(\"%\" PRId64, r[{index}])"),
                    };
                    self.line(&format!(
                        "if (!{pre}_contains({ptr}, {key})) logos_fatalf({}, {shown});",
                        words::missing_key()
                    ));
                    let get = format!("{pre}_get({ptr}, {key})");
                    self.line(&format!("r[{dst}] = {};", map_val_load(&get, vk)));
                } else if let Kind::Tuple(ti) = ck {
                    // Positional tuple cell read — the cell already holds the register-encoded value (Text ptr /
                    // Float bits / Int), exactly like a struct field. A LOGOS index is 1-based or counts back from
                    // the end, and one naming no position fails as the VM's does.
                    let n = self.tuple_positions(ti).len();
                    let at = self.int_operand(kinds, *index, words::IntegerPlace::Tuple);
                    self.line(&format!("r[{dst}] = ((int64_t*)(intptr_t)r[{collection}])[logos_idx0({at}, {n})];"));
                } else if ck == Kind::Text {
                    if unchecked {
                        crate::optimize::mark_backend_decline(
                            crate::optimization::Opt::Unchecked,
                            "unchecked: a Text is indexed through its bounds-checked character accessor",
                        );
                    }
                    let owned = self.owned_flag(*collection);
                    let at = self.int_operand(kinds, *index, words::IntegerPlace::Text);
                    self.line(&format!("r[{dst}] = (int64_t)(intptr_t)lc_text_item_of(r[{collection}], {owned}, {at});"));
                } else {
                    let sk = seq_kind_of(ck);
                    sk.seq_infix()
                        .ok_or_else(|| format!("Index on a non-sequence kind {ck:?}"))?;
                    let ptr = Self::seq_ptr(*collection, sk)?;
                    let index_word = self.int_operand(kinds, *index, words::IntegerPlace::ListRead);
                    let at = if unchecked {
                        crate::optimize::mark_fired(crate::optimization::Opt::Unchecked);
                        format!("(size_t)({index_word} - 1LL)")
                    } else {
                        format!("logos_idx0({index_word}, {ptr}->len)")
                    };
                    let cell = format!("{ptr}->data[{at}]");
                    self.line(&format!(
                        "r[{dst}] = {};",
                        elem_to_reg(&cell, self.elem_kind(sk))
                    ));
                }
            }
            Op::Length { dst, collection } => {
                let ck = kind_at(kinds, *collection);
                if ck == Kind::Text {
                    let owned = self.owned_flag(*collection);
                    self.line(&format!(
                        "r[{dst}] = lc_text_len(r[{collection}], {owned});"
                    ));
                } else if let Some((kk, vk)) = self.resolve_map_kv(ck) {
                    // Distinct-entry count — the struct's `len` field (bumped once per key), matching the AST C.
                    self.line(&format!(
                        "r[{dst}] = (int64_t){}->len;",
                        map_ptr_kv(*collection, kk, vk).unwrap()
                    ));
                } else if let Some(sk) = resolve_set(ck) {
                    let infix = sk.seq_infix().unwrap();
                    let ptr = Self::set_ptr(*collection, sk)?;
                    self.line(&format!("r[{dst}] = set_{infix}_len({ptr});"));
                } else if let Kind::Tuple(ti) = ck {
                    self.line(&format!("r[{dst}] = {};", self.tuple_positions(ti).len()));
                } else {
                    let sk = seq_kind_of(ck);
                    sk.seq_infix()
                        .ok_or_else(|| format!("Length on a non-sequence kind {ck:?}"))?;
                    let ptr = Self::seq_ptr(*collection, sk)?;
                    self.line(&format!("r[{dst}] = (int64_t){ptr}->len;"));
                }
            }
            Op::Contains {
                dst,
                collection,
                value,
            } => {
                let ck = kind_at(kinds, *collection);
                if let Some((kk, vk)) = self.resolve_map_kv(ck) {
                    // `m contains k` — key membership.
                    let (pre, ptr) = (
                        map_fn(kk, vk).unwrap(),
                        map_ptr_kv(*collection, kk, vk).unwrap(),
                    );
                    self.line(&format!(
                        "r[{dst}] = {pre}_contains({ptr}, {});",
                        map_key_arg(*value, kk)
                    ));
                } else if let Some(sk) = resolve_set(ck) {
                    let infix = sk.seq_infix().unwrap();
                    let ptr = Self::set_ptr(*collection, sk)?;
                    self.line(&format!(
                        "r[{dst}] = set_{infix}_contains({ptr}, {});",
                        as_elem_push(*value, sk.elem())
                    ));
                } else if ck == Kind::Text {
                    // `text contains part` — a substring test; any other needle fails as the VM's does.
                    let vk = kind_at(kinds, *value);
                    if vk == Kind::Text {
                        self.line(&format!(
                            "r[{dst}] = (strstr({}, {}) != NULL);",
                            as_cstr(*collection),
                            as_cstr(*value)
                        ));
                    } else {
                        let sample =
                            sample_value(vk).ok_or_else(|| format!("Text contains a {vk:?}"))?;
                        self.line(&format!(
                            "logos_fatalf({});",
                            words::text_contains_refused(sample)
                        ));
                    }
                } else {
                    let sk = seq_kind_of(ck);
                    let infix = sk
                        .seq_infix()
                        .ok_or_else(|| format!("Contains on a non-sequence kind {ck:?}"))?;
                    let ptr = Self::seq_ptr(*collection, sk)?;
                    let (ek, vk) = (self.elem_kind(sk), kind_at(kinds, *value));
                    if ek == Kind::Rational || vk == Kind::Rational {
                        // A rational is found by its value, an integer among rationals by the rational it equals.
                        let n = self.fresh();
                        let item = elem_to_reg(&format!("{ptr}->data[_ci{n}]"), ek);
                        let same = self.eq_expr(&item, ek, &format!("r[{value}]"), vk)?;
                        self.line(&format!(
                            "{{ int64_t _cf{n} = 0; for (size_t _ci{n} = 0; _ci{n} < {ptr}->len && !_cf{n}; _ci{n}++) _cf{n} = {same}; r[{dst}] = _cf{n}; }}"
                        ));
                    } else {
                        self.line(&format!(
                            "r[{dst}] = seq_{infix}_contains({ptr}, {});",
                            as_elem_push(*value, ek)
                        ));
                    }
                }
            }
            Op::ListPush { list, value } => {
                // The list's kind AFTER this op is the pushed-element seq kind (its kind here, pre-op, may still
                // be a freshly-created seq), or the kind its variable holds when a number join lands the push as
                // that; an integer pushed among rationals goes in as the rational it equals.
                let vk = kind_at(kinds, *value);
                let seq_kind = match kind_at(after, *list) {
                    k @ Kind::SeqOf(_) if self.elem_kind(k) == Kind::Rational => k,
                    _ => self.seq_kind_for(vk),
                };
                let infix = seq_kind.seq_infix().ok_or_else(|| {
                    format!("Push of an element of unknown kind into {seq_kind:?}")
                })?;
                self.own_target(*list, seq_kind)?;
                let ptr = Self::seq_ptr(*list, seq_kind)?;
                let item = self.elem_value(*value, vk, self.elem_kind(seq_kind))?;
                self.line(&format!("seq_{infix}_push({ptr}, {item});"));
            }
            // `Push v to obj's field` — append into the Seq that lives in a struct field cell. Same typed push
            // helper as `ListPush`; the field's boundary kind gives the element type, and the cell holds the
            // boxed seq pointer (as every struct cell holds its value).
            Op::ListPushField { obj, field, src } => {
                let si = match kind_at(kinds, *obj) {
                    Kind::Struct(si) => si,
                    k => return Err(format!("ListPushField on a non-struct kind {k:?}")),
                };
                let (cell, fkind) = self
                    .resolve_field(si, *field)
                    .ok_or("ListPushField of an unknown field")?;
                if fkind == Kind::Tower {
                    return self.push_to_exact_field(*obj, si, cell, *src, kinds);
                }
                if kind_at(kinds, *src) == Kind::Tower {
                    return Err(format!("an exact value pushed onto a field of kind {fkind:?} is not yet lowered"));
                }
                let infix = fkind.seq_infix().ok_or_else(|| {
                    format!("ListPushField into a non-sequence field kind {fkind:?}")
                })?;
                let ty = fkind.seq_c_type().ok_or_else(|| {
                    format!("ListPushField into a non-sequence field kind {fkind:?}")
                })?;
                // The record becomes the register's own, then the field's list the record's own: a list the record
                // may share with another holder is copied into the field before the push lands.
                self.own_target(*obj, Kind::Struct(si))?;
                let cell_val = format!("((int64_t*)(intptr_t)r[{obj}])[{cell}]");
                let word = self.field_owner_word(*obj, Kind::Struct(si)).unwrap();
                let held_alone = if cell < 64 {
                    format!("(({word} >> {cell}) & 1)")
                } else {
                    "0".to_string()
                };
                let n = self.fresh();
                self.line(&format!(
                    "if (!{held_alone}) {{ {ty} *_fl{n} = ({ty}*)lc_gc_alloc(sizeof({ty})); *_fl{n} = seq_{infix}_copy(({ty}*)(intptr_t){cell_val}); {cell_val} = (int64_t)(intptr_t)_fl{n};{} }}",
                    if cell < 64 { format!(" {word} |= 1ULL << {cell};") } else { String::new() }
                ));
                self.line(&format!(
                    "seq_{infix}_push(({ty}*)(intptr_t){cell_val}, {});",
                    as_elem_push(*src, self.elem_kind(fkind))
                ));
            }
            // ── for-each iterator machine (stack of frames; IterNext/IterPop act on the innermost) ──────────
            Op::IterPrepare { iterable } => {
                // The loop walks the collection the register held when it began: a snapshot of the handle, so
                // a change the body makes to the register (its own copy, under value semantics) is not seen.
                let raw = kind_at(kinds, *iterable);
                let id = self.fresh();
                if self.text_byte_iters.contains(&pc) {
                    self.line(&format!(
                        "const unsigned char *_itv{id} = (const unsigned char*)(intptr_t)r[{iterable}]; size_t _it{id} = 0;"
                    ));
                    self.iter_stack.push(IterFrame {
                        seq_kind: Kind::SeqInt,
                        text_bytes: true,
                        id,
                    });
                } else if raw == Kind::Text {
                    self.line(&format!(
                        "const char *_itv{id} = {}; size_t _it{id} = 0;",
                        as_cstr(*iterable)
                    ));
                    self.iter_stack.push(IterFrame {
                        seq_kind: Kind::Text,
                        text_bytes: false,
                        id,
                    });
                } else if let Some((kk, vk)) = self.resolve_map_kv(raw) {
                    // A map walks a snapshot of its entries, in insertion order, each a `(key, value)` tuple.
                    let ti = self.intern_tuple(vec![kk, vk]);
                    let (ty, ptr) = (
                        map_type_name(kk, vk).unwrap(),
                        map_ptr_kv(*iterable, kk, vk).unwrap(),
                    );
                    let key = if kk == Kind::Text {
                        format!("(int64_t)(intptr_t)_m{id}->keys[_si]")
                    } else {
                        format!("_m{id}->keys[_si]")
                    };
                    let val = map_val_load(&format!("_m{id}->vals[_si]"), vk);
                    self.line(&format!(
                        "{ty} *_m{id} = {ptr}; Seq_i64 *_itv{id} = (Seq_i64*)lc_gc_alloc(sizeof(Seq_i64)); *_itv{id} = seq_i64_new(); size_t _it{id} = 0;"
                    ));
                    self.line(&format!(
                        "for (size_t _oi = 0; _oi < _m{id}->ord_len; _oi++) {{ size_t _si = _m{id}->order[_oi]; if (!_m{id}->state[_si]) continue; int64_t *_kv = (int64_t*)lc_gc_calloc(2, sizeof(int64_t)); _kv[0] = {key}; _kv[1] = {val}; seq_i64_push(_itv{id}, (int64_t)(intptr_t)_kv); }}"
                    ));
                    self.iter_stack.push(IterFrame {
                        seq_kind: self.seq_kind_for(Kind::Tuple(ti)),
                        text_bytes: false,
                        id,
                    });
                } else if let Some(sk) = resolve_set(raw) {
                    // A set walks a snapshot of its elements, in insertion order.
                    let (set_ty, infix) = (sk.set_c_type().unwrap(), sk.seq_infix().unwrap());
                    let seq_kind = self.seq_kind_for(sk.elem());
                    let seq_ty = seq_kind.seq_c_type().unwrap();
                    self.line(&format!(
                        "{set_ty} *_st{id} = ({set_ty}*)(intptr_t)r[{iterable}]; {seq_ty} *_itv{id} = ({seq_ty}*)lc_gc_alloc(sizeof({seq_ty})); *_itv{id} = seq_{infix}_new(); size_t _it{id} = 0;"
                    ));
                    self.line(&format!(
                        "for (size_t _oi = 0; _oi < _st{id}->ord_len; _oi++) {{ size_t _si = _st{id}->order[_oi]; if (_st{id}->state[_si]) seq_{infix}_push(_itv{id}, _st{id}->keys[_si]); }}"
                    ));
                    self.iter_stack.push(IterFrame {
                        seq_kind,
                        text_bytes: false,
                        id,
                    });
                } else {
                    let seq_kind = seq_kind_of(raw);
                    let ty = seq_kind
                        .seq_c_type()
                        .ok_or_else(|| format!("iteration over a non-sequence kind {raw:?}"))?;
                    self.line(&format!(
                        "{ty} *_itv{id} = ({ty}*)(intptr_t)r[{iterable}]; size_t _it{id} = 0;"
                    ));
                    self.iter_stack.push(IterFrame {
                        seq_kind,
                        text_bytes: false,
                        id,
                    });
                }
            }
            Op::IterNext { dst, exit } => {
                let frame = self
                    .iter_stack
                    .last()
                    .ok_or("IterNext with no active iterator")?;
                let (seq_kind, id) = (frame.seq_kind, frame.id);
                if frame.text_bytes {
                    self.line(&format!(
                        "if (!_itv{id}[_it{id}]) goto {}{exit};",
                        self.label
                    ));
                    self.line(&format!(
                        "r[{dst}] = (int64_t)_itv{id}[_it{id}]; _it{id}++;"
                    ));
                } else if seq_kind == Kind::Text {
                    // The next character, as its own Text.
                    self.line(&format!(
                        "if (!_itv{id}[_it{id}]) goto {}{exit};",
                        self.label
                    ));
                    self.line(&format!(
                        "{{ size_t _n = lc_utf8_len((unsigned char)_itv{id}[_it{id}]); char *_c = (char*)lc_gc_alloc_atomic(_n + 1); memcpy(_c, _itv{id} + _it{id}, _n); _c[_n] = 0; r[{dst}] = (int64_t)(intptr_t)_c; _it{id} += _n; }}"
                    ));
                } else {
                    self.line(&format!(
                        "if (_it{id} >= _itv{id}->len) goto {}{exit};",
                        self.label
                    ));
                    let read = elem_to_reg(
                        &format!("_itv{id}->data[_it{id}]"),
                        self.elem_kind(seq_kind),
                    );
                    self.line(&format!("r[{dst}] = {read}; _it{id}++;"));
                }
            }
            Op::IterPop => {
                self.iter_stack.pop();
            }
            // ── Show renders by the source register's inferred kind ────────────────────────────────────────
            Op::Show { src } => match kind_at(kinds, *src) {
                Kind::Int => self.line(&format!("show_i64(r[{src}]);")),
                Kind::Bool => self.line(&format!("show_bool(r[{src}]);")),
                Kind::Float => self.line(&format!("show_f64(as_f64(r[{src}]));")),
                Kind::Text => self.line(&format!("show_str({});", as_cstr(*src))),
                // Arbitrary-precision integer renders its exact value via `lint_to_string` (+ newline).
                Kind::BigInt => self.line(&format!("show_lint(lc_xget(r[{src}]));")),
                Kind::Word(_) => self.line(&format!(
                    "printf(\"%\" PRIu64 \"\\n\", (uint64_t)r[{src}]);"
                )),
                Kind::Unknown => {
                    return Err(
                        "Show of a register whose kind is not yet modeled (Unknown)".to_string()
                    )
                }
                k => {
                    let text = self.render(&format!("r[{src}]"), k)?;
                    self.line(&format!("show_str({text});"));
                }
            },
            // "error WHEN EXECUTED" (a non-exhaustive `Inspect` fallthrough, an unbound `Set`, …): trap exactly
            // where the VM would error. The message is stderr-only (never compared to the VM's stdout).
            Op::FailWith { msg } => {
                let m = match &self.prog.constants[*msg as usize] {
                    Constant::Text(s) => c_escape(s),
                    _ => "FailWith".to_string(),
                };
                self.line(&format!("logos_fatal(\"{m}\");"));
            }
            // Optimizer/JIT artifacts with no AOT effect: a region bound guard (the emitted AOT code IS the
            // correct slow path, so the guard always passes — a deopt only exists for the JIT tier) and an
            // observation site (a JIT profiling marker). Both lower to nothing, exactly as `codegen_riscv` does.
            Op::RegionBoundsGuard { .. } | Op::Obs { .. } => {}
            // `today` / `now` — the injected fixed clock the differential harness sets, else the real one.
            Op::LoadToday { dst } => self.line(&format!("r[{dst}] = logos_today();")),
            Op::LoadNow { dst } => self.line(&format!("r[{dst}] = logos_now();")),
            // `args()` — the program's argument vector, program name first.
            Op::Args { dst } => {
                let n = self.fresh();
                self.line(&format!(
                    "Seq_str *_av{n} = (Seq_str*)lc_gc_alloc(sizeof(Seq_str)); *_av{n} = seq_str_new();"
                ));
                self.line(&format!(
                    "for (int _a = 0; _a < lc_argc; _a++) seq_str_push(_av{n}, lc_argv[_a]);"
                ));
                self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_av{n};"));
            }
            // Reversible-block primitives (uncompute-safe integer updates) — mirror `codegen_riscv`.
            Op::RevSwap { a, b } => {
                let t = self.fresh();
                self.line("/* ping-pong buffer swap */");
                self.line(&format!(
                    "{{ int64_t _rs{t} = r[{a}]; r[{a}] = r[{b}]; r[{b}] = _rs{t}; }}"
                ));
                // The ownership bits travel with the handles; a register the region does not track holds a
                // handle that may be shared, so the tracked side that receives it is no longer its only holder.
                match (self.owned.contains(a), self.owned.contains(b)) {
                    (true, true) => self.line(&format!("{{ unsigned char _rso{t} = own[{a}]; own[{a}] = own[{b}]; own[{b}] = _rso{t}; }}")),
                    (true, false) => self.line(&format!("own[{a}] = 0;")),
                    (false, true) => self.line(&format!("own[{b}] = 0;")),
                    (false, false) => {}
                }
                // A side held exact that received a machine word holds it tagged.
                for (side, from) in [(*a, *b), (*b, *a)] {
                    if kind_at(after, side) == Kind::BigInt
                        && matches!(kind_at(kinds, from), Kind::Int | Kind::Bool)
                    {
                        self.line(&format!("r[{side}] = lc_xof(r[{side}]);"));
                    }
                }
            }
            // Module-level globals ride a file-scope `int64_t g[]`.
            // A global holds a collection like a record's field does: the bytecode takes it out (reading it,
            // then emptying the global) to change it in a register and puts it back, and the register holds it
            // alone exactly when the global did. Any other read leaves it shared.
            Op::GlobalGet { dst, idx } => {
                self.line(&format!("r[{dst}] = g[{idx}];"));
                if self.takes_out_global(pc) {
                    if self.owned.contains(dst) {
                        self.line(&format!("own[{dst}] = gown[{idx}];"));
                    }
                } else if !self.stays_an_inspection(*dst) {
                    self.line(&format!("gown[{idx}] = 0;"));
                }
            }
            Op::GlobalSet { idx, src } => {
                let value = match (
                    self.global_kinds
                        .get(*idx as usize)
                        .copied()
                        .unwrap_or(Kind::Unknown),
                    kind_at(kinds, *src),
                ) {
                    (Kind::BigInt, Kind::Int | Kind::Bool) => format!("lc_xof(r[{src}])"),
                    (Kind::Int, Kind::BigInt) => format!("lc_xnarrow(r[{src}])"),
                    (Kind::Tower, k) if k != Kind::Tower => format!(
                        "(int64_t)(intptr_t)({})",
                        self.tower_of(&format!("r[{src}]"), k)?
                    ),
                    _ => format!("r[{src}]"),
                };
                self.line(&format!("g[{idx}] = {value};"));
                let sole = if self.owned.contains(src) && self.dead_after(pc, *src) {
                    format!("own[{src}]")
                } else {
                    "0".to_string()
                };
                self.line(&format!("gown[{idx}] = {sole};"));
            }
            Op::RevXor { place, delta } => self.line(&format!("r[{place}] ^= r[{delta}];")),
            Op::RevNeg { place } => self.line(&format!("r[{place}] = -r[{place}];")),
            Op::RevAdd { place, delta } => self.line(&format!("r[{place}] += r[{delta}];")),
            Op::RevSub { place, delta } => self.line(&format!("r[{place}] -= r[{delta}];")),
            // Deep copy: box a fresh collection (its value semantics), or copy the scalar.
            Op::DeepClone { dst, src } => {
                let e = self
                    .clone_collection_expr(*src, kind_at(kinds, *src))?
                    .unwrap_or_else(|| format!("r[{src}]"));
                self.line(&format!("r[{dst}] = {e};"));
            }
            Op::Halt => {}
            Op::Return { src } => {
                if in_function {
                    self.store_inout_params(kinds)?;
                    let owner = self.owned_flag(*src);
                    self.line(&format!("if (__ret_own) *__ret_own = {owner};"));
                    self.release_owned_scalar_sequences(Some(*src));
                    // A function whose result is exact returns a machine integer as an exact one; its word entry
                    // returns a machine integer as itself and an exact one marked `escaped`.
                    let exact = self
                        .current_fn
                        .is_some_and(|fi| self.fn_ret[fi] == Kind::BigInt);
                    let word = matches!(kind_at(kinds, *src), Kind::Int | Kind::Bool);
                    let tower = self
                        .current_fn
                        .is_some_and(|fi| self.fn_ret[fi] == Kind::Tower);
                    let declared = self.current_fn.map(|fi| self.fn_ret[fi]).unwrap_or(Kind::Unknown);
                    if tower && kind_at(kinds, *src) != Kind::Tower {
                        // An exact result is the runtime's value of what the function hands back.
                        let cell = self.tower_of(&format!("r[{src}]"), kind_at(kinds, *src))?;
                        self.line(&format!("return (int64_t)(intptr_t)({cell});"));
                    } else if self.widens_to(kind_at(kinds, *src), declared) {
                        // A rational result handed back as an integer is the rational it equals.
                        let v = self.convert_number(&format!("r[{src}]"), kind_at(kinds, *src), declared)?;
                        self.line(&format!("return {v};"));
                    } else if self.word_entry {
                        self.line(&self.ret(&format!("r[{src}]"), exact && !word));
                    } else if exact && word {
                        self.line(&format!("return lc_xof(r[{src}]);"));
                    } else {
                        self.line(&format!("return r[{src}];"));
                    }
                }
            }
            Op::ReturnNothing => {
                if in_function {
                    self.store_inout_params(kinds)?;
                    self.line("if (__ret_own) *__ret_own = 0;");
                    self.release_owned_scalar_sequences(None);
                    self.line(&self.ret("0", false));
                }
            }
            // ── CRDT lattice (single-replica; mirrors `codegen_riscv`, but over the C `int64_t*` struct layout) ──
            // A counter CRDT (`ConvergentCount`/`Tally`, kind 4/5) → a fresh `logos_crdt*` per-replica map; a
            // sequence CRDT (`SharedSequence`/RGA, kind 1) IS a plain empty seq on the single replica (its field
            // lays out as `Seq of T`, so `CrdtAppend` pushes in place, exactly like a shared-`Seq` field). Set /
            // register CRDTs (0/2/3) need their own lattices → refused.
            Op::NewCrdt { dst, kind } => {
                match kind {
                    4 | 5 => {
                        self.line(&format!("r[{dst}] = (int64_t)(intptr_t)logos_crdt_new();"));
                    }
                    1 => {
                        let t = self.fresh();
                        // The converged log is a plain seq behind a stable handle; its concrete element kind is
                        // refined once the first `CrdtAppend` (or a `GetField` typed by the field) lands. Box an empty
                        // `Seq_i64` — its handle rides the slot, and the field's declared `Seq of T` retypes accesses.
                        self.line(&format!("Seq_i64 *_cr{t} = (Seq_i64*)lc_gc_alloc(sizeof(Seq_i64)); *_cr{t} = seq_i64_new();"));
                        self.line(&format!("r[{dst}] = (int64_t)(intptr_t)_cr{t};"));
                    }
                    k => {
                        return Err(format!(
                        "NewCrdt of a non-counter/sequence CRDT kind {k} (set/register lattice)"
                    ))
                    }
                }
            }
            Op::CrdtBump {
                obj,
                field,
                amount,
                negate,
            } => {
                // `Increase/Decrease s's counter by n`. The VM's `crdt_counter_bump`: a LIVE counter field bumps
                // its own replica slot (`logos_crdt_bump`); a field that is a plain Int / `Nothing` (never a
                // `NewCrdt` value flowed in) just ADDS the (signed) amount to the cell, IN PLACE (visible through
                // every struct alias, which all share the same heap buffer pointer).
                let ok = kind_at(kinds, *obj);
                let (cell, _) = self
                    .resolve_field_any(ok, *field)
                    .ok_or("CrdtBump of an unknown struct field")?;
                let is_counter = self
                    .struct_name_of_kind(ok)
                    .map(|n| self.counter_fields.contains(&(n, cell)))
                    .unwrap_or(false);
                let cell_lv = format!("((int64_t*)(intptr_t)r[{obj}])[{cell}]");
                if is_counter {
                    let amount = if kind_at(kinds, *amount) == Kind::BigInt {
                        format!("lc_xnarrow(r[{amount}])")
                    } else {
                        format!("r[{amount}]")
                    };
                    self.line(&format!(
                        "logos_crdt_bump((logos_crdt*)(intptr_t){cell_lv}, {amount}, {});",
                        if *negate { 1 } else { 0 }
                    ));
                } else {
                    let op = if *negate { "lc_xsub" } else { "lc_xadd" };
                    let amount = as_exact(*amount, kind_at(kinds, *amount))?;
                    self.line(&format!("{cell_lv} = {op}({cell_lv}, {amount});"));
                }
            }
            Op::CrdtMerge { target, source } | Op::CrdtJoin { target, source } => {
                // Field-wise lattice join of `source` into `target`, in place. A counter field takes the
                // per-replica MAX (idempotent, `logos_crdt_merge`); a plain field ADDs (`Merge`, a grow-counter)
                // or MAXes (`Join`, the state-based join) — the VM's `crdt_merge_field`. Both structs share the
                // type, so iterate its cells.
                let is_join = matches!(op, Op::CrdtJoin { .. });
                let tk = kind_at(kinds, *target);
                let tname = self
                    .struct_name_of_kind(tk)
                    .ok_or("CRDT merge target is not a struct")?;
                let n = self
                    .struct_cell_count(tk)
                    .ok_or("CRDT merge of an unknown struct type")?;
                for cell in 0..n {
                    let t_lv = format!("((int64_t*)(intptr_t)r[{target}])[{cell}]");
                    let s_lv = format!("((int64_t*)(intptr_t)r[{source}])[{cell}]");
                    if self.counter_fields.contains(&(tname.clone(), cell)) {
                        self.line(&format!(
                            "logos_crdt_merge((logos_crdt*)(intptr_t){t_lv}, (logos_crdt*)(intptr_t){s_lv});"
                        ));
                    } else if self.field_storage_kind(tk, cell) == Some(Kind::BigInt) {
                        if is_join {
                            self.line(&format!(
                                "if (lc_xcmp({s_lv}, {t_lv}) > 0) {t_lv} = {s_lv};"
                            ));
                        } else {
                            self.line(&format!("{t_lv} = lc_xadd({t_lv}, {s_lv});"));
                        }
                    } else if is_join {
                        self.line(&format!("if ({s_lv} > {t_lv}) {t_lv} = {s_lv};"));
                    } else {
                        self.line(&format!("{t_lv} += {s_lv};"));
                    }
                }
            }
            Op::CrdtResolve { obj, field, value } => {
                // `Resolve s's reg to v` — a multi-value register's LWW resolution. Single-replica = the last
                // value written wins, so set the field's value cell in place (visible through every struct alias).
                let ok = kind_at(kinds, *obj);
                let (cell, _) = self
                    .resolve_field_any(ok, *field)
                    .ok_or("CrdtResolve of an unknown struct field")?;
                let stored = self.field_store_value(ok, cell, *value, kind_at(kinds, *value))?;
                self.line(&format!(
                    "((int64_t*)(intptr_t)r[{obj}])[{cell}] = {stored};"
                ));
            }
            Op::CrdtAppend { seq, value } => {
                // `Append v to <seq field>` — the sequence is shared behind a stable seq handle in the struct
                // field, so append IN PLACE; the mutation propagates to every alias (no write-back). The pushed
                // value's kind picks the typed seq call (the field was a `NewCrdt` kind-1 empty `Seq_i64` handle).
                let seq_kind = self.seq_kind_for(kind_at(kinds, *value));
                let infix = seq_kind.seq_infix().ok_or_else(|| {
                    format!("CrdtAppend of an element of unknown kind {seq_kind:?}")
                })?;
                let ty = seq_kind.seq_c_type().unwrap();
                self.line(&format!(
                    "seq_{infix}_push(({ty}*)(intptr_t)r[{seq}], {});",
                    as_elem_push(*value, self.elem_kind(seq_kind))
                ));
            }
            // ── Peer messaging (the VM's offline `NetInbox`, see `c_net_runtime`) ──────────────────────────
            // `Connect` validates its address and dials nothing; `Listen` names this node's inbox; `Sync` of a
            // value is offline identity, so the register keeps what it holds.
            Op::NetConnect { url, .. } => {
                let text = self.text_of(*url, kind_at(kinds, *url))?;
                self.line(&format!("logos_net_connect({text});"));
            }
            Op::NetListen { topic } => {
                let text = self.text_of(*topic, kind_at(kinds, *topic))?;
                self.line(&format!("logos_net_listen({text});"));
            }
            Op::NetSync { .. } => {}
            // A peer handle is its address's canonical topic.
            Op::NetMakePeer { dst, addr } => {
                let text = self.text_of(*addr, kind_at(kinds, *addr))?;
                self.line(&format!(
                    "r[{dst}] = (int64_t)logos_net_makepeer((long long)(intptr_t)({text}));"
                ));
            }
            // A message travels as its register encoding — the value itself, or its heap handle — and arrives
            // as the kind every `Send` of the program sends (`net_message`).
            Op::NetSend { to, msg } => {
                if let Some(to) = self.peer_address(*to, kinds)? {
                    self.line(&format!("logos_net_send({to}, (long long)r[{msg}]);"));
                }
            }
            Op::NetStream { to, values } => {
                if let Some(to) = self.peer_address(*to, kinds)? {
                    self.line(&format!("logos_net_stream({to}, (long long)r[{values}]);"));
                }
            }
            Op::NetAwait { dst, from, stream } => {
                if let Some(from) = self.peer_address(*from, kinds)? {
                    let take = if *stream {
                        "logos_net_await_stream"
                    } else {
                        "logos_net_await"
                    };
                    self.line(&format!("r[{dst}] = (int64_t){take}({from});"));
                }
            }
            // ── Real-socket TCP client (`tcp.lg` natives) ──────────────────────────────────────────────────
            // Each op calls its `C_TCP_RUNTIME` helper and BOXES the raw result into the LOGOS `Result` value
            // `r[dst]` — a heap `[tag, payload]` buffer (tag 0 = `Ok`, tag 1 = `Err`), the SAME layout a two-
            // variant `Kind::Enum` uses, so `Inspect`'s `TestArm`/`BindArm` read it uniformly. The `Err` payload
            // is the shared error `char*` (byte-identical to the Rust tiers); the `Ok` payload is the handle /
            // byte-count Int, the received `Seq of Int`, or `Nothing` (for the `Unit`-returning close/timeout).
            // `kind` is the `TcpOp::code()` value (connect=0, send=1, recv=2, recvExact=3, close=4, setTimeout=5,
            // listen=6, accept=7, listenerPort=8, shutdownWrite=9); operands ride `r[args_start..+arg_count]`.
            Op::Tcp {
                dst,
                kind,
                args_start,
                arg_count,
            } => {
                let a = |i: u16| args_start + i;
                let n = self.fresh();
                let err = format!("_tcperr{n}");
                self.line(&format!("char *{err} = NULL;"));
                // Emit `r[dst] = <Ok box of `ok_expr`>` OR `r[dst] = <Err box of `err`>` guarded by `cond` (true →
                // Err). The Result box is the one `NewInductive` builds: `lc_gc_calloc(2, int64_t)`, cell[0] = the tag.
                let (ok_tag, err_tag) = (
                    self.ctor_tag("Result", "Ok").unwrap(),
                    self.ctor_tag("Result", "Err").unwrap(),
                );
                let box_result = |em: &mut Self, ok_expr: &str, cond: &str| {
                    let b = em.fresh();
                    em.line(&format!(
                        "int64_t *_res{b} = (int64_t*)lc_gc_calloc(2, sizeof(int64_t));"
                    ));
                    em.line(&format!("if ({cond}) {{ _res{b}[0] = {err_tag}; _res{b}[1] = (int64_t)(intptr_t){err}; }}"));
                    em.line(&format!(
                        "else {{ _res{b}[0] = {ok_tag}; _res{b}[1] = (int64_t){ok_expr}; }}"
                    ));
                    em.line(&format!("r[{dst}] = (int64_t)(intptr_t)_res{b};"));
                };
                match *kind {
                    // Connect(host: Text, port: Int) -> Result of Int and Text — Ok = the connection handle.
                    0 => {
                        let c = self.fresh();
                        self.line(&format!(
                            "long long _tc{c} = logos_tcp_connect({}, r[{}], &{err});",
                            as_cstr(a(0)),
                            a(1)
                        ));
                        box_result(self, &format!("_tc{c}"), &format!("_tc{c} < 0"));
                    }
                    // Send(conn: Int, data: Seq of Int) -> Result of Int and Text — Ok = the byte count sent.
                    1 => {
                        let dptr = Self::seq_ptr(a(1), Kind::SeqInt)?;
                        let c = self.fresh();
                        self.line(&format!(
                            "long long _tc{c} = logos_tcp_send(r[{}], {dptr}, &{err});",
                            a(0)
                        ));
                        box_result(self, &format!("_tc{c}"), &format!("_tc{c} < 0"));
                    }
                    // Recv(conn, maxLen) / RecvExact(conn, len) -> Result of (Seq of Int) and Text — Ok = the bytes.
                    2 | 3 => {
                        let f = if *kind == 2 {
                            "logos_tcp_recv"
                        } else {
                            "logos_tcp_recv_exact"
                        };
                        let c = self.fresh();
                        self.line(&format!(
                            "Seq_i64 *_tc{c} = {f}(r[{}], r[{}], &{err});",
                            a(0),
                            a(1)
                        ));
                        box_result(
                            self,
                            &format!("(intptr_t)_tc{c}"),
                            &format!("_tc{c} == NULL"),
                        );
                    }
                    // Close(conn) -> Result of Unit and Text — Ok = `Nothing` (0), like the VM's `RuntimeValue::Nothing`.
                    4 => {
                        let c = self.fresh();
                        self.line(&format!(
                            "long long _tc{c} = logos_tcp_close(r[{}], &{err});",
                            a(0)
                        ));
                        box_result(self, "0", &format!("_tc{c} < 0"));
                    }
                    // Listen(port) -> Result of Int and Text — Ok = the listener handle.
                    6 => {
                        let c = self.fresh();
                        self.line(&format!(
                            "long long _tc{c} = logos_tcp_listen(r[{}], &{err});",
                            a(0)
                        ));
                        box_result(self, &format!("_tc{c}"), &format!("_tc{c} < 0"));
                    }
                    // Accept(listener) -> Result of Int and Text — Ok = the new connection handle.
                    7 => {
                        let c = self.fresh();
                        self.line(&format!(
                            "long long _tc{c} = logos_tcp_accept(r[{}], &{err});",
                            a(0)
                        ));
                        box_result(self, &format!("_tc{c}"), &format!("_tc{c} < 0"));
                    }
                    // ListenerPort(listener) -> Result of Int and Text — Ok = the bound local port.
                    8 => {
                        let c = self.fresh();
                        self.line(&format!(
                            "long long _tc{c} = logos_tcp_listener_port(r[{}], &{err});",
                            a(0)
                        ));
                        box_result(self, &format!("_tc{c}"), &format!("_tc{c} < 0"));
                    }
                    // SetTimeout(conn, millis) -> Result of Unit and Text — Ok = `Nothing` (0). Real POSIX
                    // `setsockopt(SO_RCVTIMEO/SO_SNDTIMEO)` on the live fd, matching the VM's per-handle deadline.
                    5 => {
                        let c = self.fresh();
                        self.line(&format!(
                            "long long _tc{c} = logos_tcp_set_timeout(r[{}], r[{}], &{err});",
                            a(0),
                            a(1)
                        ));
                        box_result(self, "0", &format!("_tc{c} < 0"));
                    }
                    // ShutdownWrite(conn) -> Result of Unit and Text — Ok = `Nothing` (0). Real `shutdown(fd, SHUT_WR)`
                    // half-close (the peer reads EOF), matching the VM's `shutdown_write`.
                    9 => {
                        let c = self.fresh();
                        self.line(&format!(
                            "long long _tc{c} = logos_tcp_shutdown_write(r[{}], &{err});",
                            a(0)
                        ));
                        box_result(self, "0", &format!("_tc{c} < 0"));
                    }
                    other => return Err(format!("Tcp op with unknown kind {other}")),
                }
                let _ = arg_count;
            }
            // `Check that <subject> is <predicate>` / capability — resolve the predicate to its condition from
            // the policy registry, compile the condition against the subject struct's field cells, and TRAP on a
            // false result exactly where the VM's `check_policy` errors (`Security Check Failed` / undefined rule
            // / missing registry). Mirrors `codegen_riscv`'s `emit_policy_condition`, in a C boolean expression.
            Op::CheckPolicy {
                subject,
                predicate,
                is_capability,
                object,
                source_text,
            } => {
                let src_msg: String = self
                    .const_text(*source_text)
                    .unwrap_or_default()
                    .to_string();
                let msg_lit = c_escape(&src_msg);
                let trap = |em: &mut Self| {
                    em.line("fflush(stdout);");
                    em.line(&format!(
                        "fprintf(stderr, \"Security Check Failed: {msg_lit}\\n\"); exit(1);"
                    ));
                };
                let Some((interner, policies)) = self.policy_ctx else {
                    // The VM traps: "Security Check requires policies. Use compiled Rust or add ## Policy block."
                    trap(self);
                    return Ok(());
                };
                let subj_kind = kind_at(kinds, *subject);
                let Some(subj_type) = self.struct_name_of_kind(subj_kind) else {
                    trap(self); // VM: "Check subject must be a struct"
                    return Ok(());
                };
                let Some(type_sym) = interner.lookup(&subj_type) else {
                    trap(self); // VM: "Unknown type in Check statement"
                    return Ok(());
                };
                let obj_type = if *object != u16::MAX {
                    self.struct_name_of_kind(kind_at(kinds, *object))
                } else {
                    None
                };
                let cond = if *is_capability {
                    policies.get_capabilities(type_sym).and_then(|caps| {
                        caps.iter()
                            .find(|c| c.action == *predicate)
                            .map(|c| c.condition.clone())
                    })
                } else {
                    policies.get_predicates(type_sym).and_then(|preds| {
                        preds
                            .iter()
                            .find(|p| p.predicate_name == *predicate)
                            .map(|p| p.condition.clone())
                    })
                };
                match cond {
                    Some(c) => {
                        let expr = self.policy_condition_expr(
                            &c,
                            *subject,
                            &subj_type,
                            *object,
                            obj_type.as_deref(),
                            interner,
                        )?;
                        self.line(&format!("if (!({expr})) {{"));
                        trap(self);
                        self.line("}");
                    }
                    None => trap(self), // VM: "No predicate/capability defined for type"
                }
            }
            other => return Err(format!("op not yet lowered: {}", other.name())),
        }
        Ok(())
    }

    /// `r[dst] = <lhs> <cop> <rhs>` over words: both operands of one width, the result wrapped to it (a
    /// division by zero, or two widths, or a word beside another kind, fails as the VM's does).
    fn word_arith(
        &mut self,
        dst: u16,
        lhs: u16,
        rhs: u16,
        cop: &str,
        lk: Kind,
        rk: Kind,
    ) -> Result<(), String> {
        let refused = |em: &mut Self| -> Result<(), String> {
            let (Some(l), Some(r)) = (sample_value(lk), sample_value(rk)) else {
                return Err(format!("`{cop}` over {lk:?} and {rk:?}"));
            };
            let result = match cop {
                "+" => crate::semantics::arith::add(l, r),
                "-" => crate::semantics::arith::subtract(l, r),
                "*" => crate::semantics::arith::multiply(l, r),
                "/" => crate::semantics::arith::divide(l, r),
                "%" => crate::semantics::arith::modulo(l, r),
                "&" => {
                    crate::semantics::arith::binary_op(crate::ast::stmt::BinaryOpKind::BitAnd, l, r)
                }
                "|" => {
                    crate::semantics::arith::binary_op(crate::ast::stmt::BinaryOpKind::BitOr, l, r)
                }
                _ => {
                    crate::semantics::arith::binary_op(crate::ast::stmt::BinaryOpKind::BitXor, l, r)
                }
            };
            let sentence = match result {
                Err(sentence) => sentence,
                Ok(_) => return Err(format!("the VM combines {lk:?} and {rk:?} with `{cop}`")),
            };
            em.line(&format!("logos_fatalf({});", words::literal(&sentence)));
            Ok(())
        };
        let (Kind::Word(a), Kind::Word(b)) = (lk, rk) else {
            return refused(self);
        };
        if a != b {
            return refused(self);
        }
        let mask = if a == 64 {
            "UINT64_MAX".to_string()
        } else {
            format!("((1ULL << {a}) - 1)")
        };
        let (x, y) = (format!("(uint64_t)r[{lhs}]"), format!("(uint64_t)r[{rhs}]"));
        match cop {
            "/" | "%" => {
                let zero = if cop == "/" {
                    words::division_by_zero()
                } else {
                    words::modulo_by_zero()
                };
                self.line(&format!("if ({y} == 0) logos_fatalf({zero});"));
                self.line(&format!("r[{dst}] = (int64_t)({x} {cop} {y});"));
            }
            _ => self.line(&format!("r[{dst}] = (int64_t)(({x} {cop} {y}) & {mask});")),
        }
        Ok(())
    }

    /// `r[dst] = <lhs> <cop> <rhs>` at the right kind: float when either operand is float, else int.
    fn arith(
        &mut self,
        dst: u16,
        lhs: u16,
        rhs: u16,
        cop: &str,
        kinds: &[Kind],
    ) -> Result<(), String> {
        let (lk, rk) = (kind_at(kinds, lhs), kind_at(kinds, rhs));
        // Arithmetic on a time value (a date and a span, an instant and a duration) is the runtime's own; machine
        // arithmetic on the numbers a slot holds them as would answer something the VM never does.
        if [lk, rk].iter().any(|k| matches!(k, Kind::Date | Kind::Moment | Kind::Time | Kind::Span)) {
            return Err(format!("`{cop}` over {lk:?} and {rk:?} is not yet lowered"));
        }
        // A Text operand makes `+` string concatenation — the VM emits a plain `Add` whenever it cannot
        // prove both sides are Text (a `format(…)` result, a parameter) — exactly as `Concat` lowers it, a word
        // rendered unsigned as `Show` gives it. Any other arithmetic on Text is refused, never computed on the
        // pointers.
        if lk == Kind::Text || rk == Kind::Text {
            if cop != "+" {
                return Err(format!("`{cop}` on a Text operand"));
            }
            let (l, r) = (self.text_of(lhs, lk)?, self.text_of(rhs, rk)?);
            self.line(&format!(
                "r[{dst}] = (int64_t)(intptr_t)str_concat({l}, {r});"
            ));
            return Ok(());
        }
        if matches!(lk, Kind::Word(_)) || matches!(rk, Kind::Word(_)) {
            return self.word_arith(dst, lhs, rhs, cop, lk, rk);
        }
        // A rational is the VM's exact number: `+ - *` exact, `/` truncating two whole values and exact otherwise, each
        // result boxed; the bitwise operators take two whole values as their integers. A Real operand makes the
        // operation a real one, as the VM's.
        if lk == Kind::Rational || rk == Kind::Rational {
            if lk == Kind::Float || rk == Kind::Float {
                let (l, r) = (self.real_of(lhs, lk), self.real_of(rhs, rk));
                if cop == "/" {
                    self.line(&format!("r[{dst}] = as_bits(logos_rdiv({l}, {r}));"));
                } else if matches!(cop, "+" | "-" | "*") {
                    self.line(&format!("r[{dst}] = as_bits({l} {cop} {r});"));
                } else {
                    return Err(format!("`{cop}` over {lk:?} and {rk:?} is not yet lowered"));
                }
                return Ok(());
            }
            let (l, r) = (as_ldec(lhs, lk)?, as_ldec(rhs, rk)?);
            if matches!(cop, "&" | "|" | "^") {
                self.line(&format!("r[{dst}] = ldec_bitwise({l}, {r}, '{cop}');"));
                return Ok(());
            }
            let f = match cop {
                "+" => "ldec_add",
                "-" => "ldec_sub",
                "*" => "ldec_mul",
                "/" => "ldec_quotient",
                _ => return Err(format!("`{cop}` over {lk:?} and {rk:?} is not yet lowered")),
            };
            self.box_ldec(dst, &format!("{f}({l}, {r})"));
            return Ok(());
        }
        if cop == "/" && (lk == Kind::Float || rk == Kind::Float) {
            self.line(&format!(
                "r[{dst}] = as_bits(logos_rdiv({}, {}));",
                as_double(lhs, lk),
                as_double(rhs, rk)
            ));
        } else if lk == Kind::Float || rk == Kind::Float {
            self.line(&format!(
                "r[{dst}] = as_bits({} {cop} {});",
                as_double(lhs, lk),
                as_double(rhs, rk)
            ));
        } else if cop == "/" {
            self.line(&format!("r[{dst}] = logos_idiv(r[{lhs}], r[{rhs}]);"));
        } else {
            self.line(&format!("r[{dst}] = r[{lhs}] {cop} r[{rhs}];"));
        }
        Ok(())
    }

    /// A `char*` rendering register `src` (kind `sk`) under interpolation format spec `spec`, mirroring the AST
    /// backend's `c_format_spec_for_hole` byte-for-byte — `$` money (`$%.2f`), `.N` precision, `<`/`>`/`^`
    /// alignment to a width, and a bare `N` right-aligned width — reusing the runtime's `logos_dyn_sprintf` /
    /// `logos_center_*`. A spec/kind combination neither models is a clean refusal.
    fn format_spec_body(&self, src: u16, sk: Kind, spec: &str) -> Result<String, String> {
        if spec == "$" {
            return Ok(format!(
                "logos_dyn_sprintf(\"$%.2f\", {})",
                as_double(src, sk)
            ));
        }
        if let Some(rest) = spec.strip_prefix('.') {
            let prec: usize = rest
                .parse()
                .map_err(|_| format!("bad precision spec `{spec}`"))?;
            return Ok(format!(
                "logos_dyn_sprintf(\"%.{prec}f\", {})",
                as_double(src, sk)
            ));
        }
        if let Some(align) = spec.chars().next().filter(|c| matches!(c, '<' | '>' | '^')) {
            let width: usize = spec[1..]
                .parse()
                .map_err(|_| format!("bad width in spec `{spec}`"))?;
            if align == '^' {
                return match sk {
                    Kind::Int => Ok(format!("logos_center_i64(r[{src}], {width})")),
                    Kind::Text => Ok(format!("logos_center_str({}, {width})", as_cstr(src))),
                    Kind::Word(_) => Ok(format!(
                        "logos_center_str(logos_dyn_sprintf(\"%\" PRIu64, (uint64_t)r[{src}]), {width})"
                    )),
                    _ => Err(format!("center-format of kind {sk:?} unsupported")),
                };
            }
            let flag = if align == '<' { "-" } else { "" };
            return match sk {
                Kind::Int => Ok(format!(
                    "logos_dyn_sprintf(\"%{flag}{width}\" PRId64, r[{src}])"
                )),
                Kind::Text => Ok(format!(
                    "logos_dyn_sprintf(\"%{flag}{width}s\", {})",
                    as_cstr(src)
                )),
                Kind::Float => Ok(format!(
                    "logos_dyn_sprintf(\"%{flag}{width}g\", as_f64(r[{src}]))"
                )),
                Kind::Word(_) => Ok(format!(
                    "logos_dyn_sprintf(\"%{flag}{width}\" PRIu64, (uint64_t)r[{src}])"
                )),
                _ => Err(format!("aligned-format of kind {sk:?} unsupported")),
            };
        }
        if let Ok(width) = spec.parse::<usize>() {
            return match sk {
                Kind::Int => Ok(format!("logos_dyn_sprintf(\"%{width}\" PRId64, r[{src}])")),
                Kind::Text => Ok(format!(
                    "logos_dyn_sprintf(\"%{width}s\", {})",
                    as_cstr(src)
                )),
                Kind::Float => Ok(format!(
                    "logos_dyn_sprintf(\"%{width}g\", as_f64(r[{src}]))"
                )),
                Kind::Word(_) => Ok(format!(
                    "logos_dyn_sprintf(\"%{width}\" PRIu64, (uint64_t)r[{src}])"
                )),
                _ => Err(format!("width-format of kind {sk:?} unsupported")),
            };
        }
        Err(format!("unsupported interpolation format spec `{spec}`"))
    }

    /// `r[dst] = (<lhs> <cop> <rhs>)` (a Bool result): float compare when either operand is float.
    fn cmp(&mut self, dst: u16, lhs: u16, rhs: u16, cop: &str, kinds: &[Kind]) -> Result<(), String> {
        let (lk, rk) = (kind_at(kinds, lhs), kind_at(kinds, rhs));
        // Two time values of one kind order by the number their slots hold; a span has no order, and a time value
        // beside anything else is the runtime's to compare.
        let time = |k: Kind| matches!(k, Kind::Date | Kind::Moment | Kind::Time | Kind::Span);
        if (time(lk) || time(rk)) && (lk != rk || lk == Kind::Span) {
            return Err(format!("`{cop}` over {lk:?} and {rk:?} is not yet lowered"));
        }
        if let (Kind::Word(a), Kind::Word(b)) = (lk, rk) {
            if a == b {
                self.line(&format!(
                    "r[{dst}] = ((uint64_t)r[{lhs}] {cop} (uint64_t)r[{rhs}]);"
                ));
                return Ok(());
            }
        }
        // A word orders only against a word of its width: any other pairing fails in the VM's words.
        if matches!(lk, Kind::Word(_)) || matches!(rk, Kind::Word(_)) {
            if let (Some(l), Some(r)) = (sample_value(lk), sample_value(rk)) {
                use crate::ast::stmt::BinaryOpKind;
                let op = match cop {
                    "<" => BinaryOpKind::Lt,
                    ">" => BinaryOpKind::Gt,
                    "<=" => BinaryOpKind::LtEq,
                    _ => BinaryOpKind::GtEq,
                };
                if let Err(sentence) = crate::semantics::arith::binary_op(op, l, r) {
                    self.line(&format!("logos_fatalf({});", words::literal(&sentence)));
                    return Ok(());
                }
            }
        }
        // A rational orders exactly against any exact number, and against a real by the real's exact value; a NaN
        // orders against nothing and equals nothing.
        if lk == Kind::Rational || rk == Kind::Rational {
            if lk == Kind::Float || rk == Kind::Float {
                let (x, f, flip) = if lk == Kind::Rational {
                    (as_ldec(lhs, lk)?, as_double(rhs, rk), "")
                } else {
                    (as_ldec(rhs, rk)?, as_double(lhs, lk), "-")
                };
                let n = self.fresh();
                let holds = if cop == "!=" {
                    format!("_c{n} == 2 || _c{n} != 0")
                } else {
                    format!("_c{n} != 2 && ({flip}_c{n}) {cop} 0")
                };
                self.line(&format!("{{ int _c{n} = ldec_cmp_f64({x}, {f}); r[{dst}] = ({holds}); }}"));
            } else {
                let (l, r) = (as_ldec(lhs, lk)?, as_ldec(rhs, rk)?);
                self.line(&format!("r[{dst}] = (ldec_cmp({l}, {r}) {cop} 0);"));
            }
            return Ok(());
        }
        if (lk == Kind::BigInt || rk == Kind::BigInt)
            && !matches!(lk, Kind::Float | Kind::Rational)
            && !matches!(rk, Kind::Float | Kind::Rational)
        {
            // `lc_xcmp` orders two exact integers as -1/0/1, so `<cmp> 0` realizes all six relations.
            let tagged = |reg: u16, k: Kind| {
                if k == Kind::BigInt {
                    format!("r[{reg}]")
                } else {
                    format!("lc_xof(r[{reg}])")
                }
            };
            self.line(&format!(
                "r[{dst}] = (lc_xcmp({}, {}) {cop} 0);",
                tagged(lhs, lk),
                tagged(rhs, rk)
            ));
        } else if lk == Kind::Float || rk == Kind::Float {
            self.line(&format!(
                "r[{dst}] = ({} {cop} {});",
                as_double(lhs, lk),
                as_double(rhs, rk)
            ));
        } else {
            self.line(&format!("r[{dst}] = (r[{lhs}] {cop} r[{rhs}]);"));
        }
        Ok(())
    }
}

/// Every undeclared enum type's constructors, in the tag order every site agrees on: `Option` is `Some, None`,
/// `Result` is `Ok, Err` (the order the native `Tcp` results are boxed in), and any other type not in
/// `prog.enum_types` lists its constructors as the program first builds them.
fn ctor_tags(prog: &CompiledProgram) -> std::collections::HashMap<String, Vec<String>> {
    let mut tags: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    tags.insert(
        "Option".to_string(),
        vec!["Some".to_string(), "None".to_string()],
    );
    tags.insert(
        "Result".to_string(),
        vec!["Ok".to_string(), "Err".to_string()],
    );
    let text = |idx: u32| match prog.constants.get(idx as usize) {
        Some(Constant::Text(t)) => Some(t.clone()),
        _ => None,
    };
    for op in &prog.code {
        if let Op::NewInductive {
            type_name, ctor, ..
        } = op
        {
            let (Some(ty), Some(c)) = (text(*type_name), text(*ctor)) else {
                continue;
            };
            if prog.enum_types.iter().any(|e| e.name == ty) {
                continue;
            }
            let ctors = tags.entry(ty).or_default();
            if !ctors.contains(&c) {
                ctors.push(c);
            }
        }
    }
    tags
}

/// How an op changes a register's collection in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mutation {
    /// Value semantics: the register's own collection changes, never another holder's — the walker copies it
    /// first unless the register's owned flag says the register is its only holder.
    Owned(u16),
    /// Shared semantics: the change reaches every holder — a CRDT's interior, which aliases share by design.
    Shared,
}

/// Whether the VM refuses `op` for the kinds its operands hold in `kinds`, whatever their values: words of two
/// widths or a word beside a number (as the VM's arithmetic answers), the remainder of a Real, `is approximately`
/// over anything but numbers.
fn refused_for_kinds(op: &Op, kinds: &[Kind]) -> bool {
    use crate::ast::stmt::BinaryOpKind;
    // A word beside another kind is the VM's own arithmetic's to judge: words of two widths and a word beside a
    // number are refused, a word joined to a Text is the text. A divisor is handed as a nonzero value, so the
    // answer is about the kinds alone.
    let words_clash = |bop: BinaryOpKind, a: Kind, b: Kind| {
        if !matches!(a, Kind::Word(_)) && !matches!(b, Kind::Word(_)) {
            return false;
        }
        match (sample_value(a), sample_divisor(b)) {
            (Some(x), Some(y)) => crate::semantics::arith::binary_op(bop, x, y).is_err(),
            _ => false,
        }
    };
    let pair = |lhs: u16, rhs: u16| (kind_at(kinds, lhs), kind_at(kinds, rhs));
    match *op {
        Op::Add { lhs, rhs, .. } => {
            let (l, r) = pair(lhs, rhs);
            words_clash(BinaryOpKind::Add, l, r)
        }
        Op::Sub { lhs, rhs, .. } => {
            let (l, r) = pair(lhs, rhs);
            words_clash(BinaryOpKind::Subtract, l, r)
        }
        Op::Mul { lhs, rhs, .. } => {
            let (l, r) = pair(lhs, rhs);
            words_clash(BinaryOpKind::Multiply, l, r)
        }
        Op::Div { lhs, rhs, .. } => {
            let (l, r) = pair(lhs, rhs);
            words_clash(BinaryOpKind::Divide, l, r)
        }
        Op::BitAnd { lhs, rhs, .. } => {
            let (l, r) = pair(lhs, rhs);
            words_clash(BinaryOpKind::BitAnd, l, r)
        }
        Op::BitOr { lhs, rhs, .. } => {
            let (l, r) = pair(lhs, rhs);
            words_clash(BinaryOpKind::BitOr, l, r)
        }
        Op::BitXor { lhs, rhs, .. } => {
            let (l, r) = pair(lhs, rhs);
            words_clash(BinaryOpKind::BitXor, l, r)
        }
        Op::Mod { lhs, rhs, .. } => {
            let (l, r) = pair(lhs, rhs);
            words_clash(BinaryOpKind::Modulo, l, r) || l == Kind::Float || r == Kind::Float
        }
        Op::ApproxEq { lhs, rhs, .. } => {
            let numeric = |k: Kind| matches!(k, Kind::Int | Kind::Float | Kind::Unknown);
            !numeric(kind_at(kinds, lhs)) || !numeric(kind_at(kinds, rhs))
        }
        _ => false,
    }
}

/// The class of every op that changes a register's collection in place — the ISA's own list
/// (`vm::peephole::mutates`), each decided between value and shared semantics; `None` for every other op.
fn mutation(op: &Op) -> Option<Mutation> {
    match *op {
        Op::ListPush { list, .. } | Op::ListPop { list, .. } => Some(Mutation::Owned(list)),
        Op::SetIndex { collection, .. }
        | Op::SetIndexUnchecked { collection, .. }
        | Op::RemoveFrom { collection, .. } => Some(Mutation::Owned(collection)),
        Op::SetAdd { set, .. } => Some(Mutation::Owned(set)),
        Op::StructInsert { obj, .. } | Op::ListPushField { obj, .. } => Some(Mutation::Owned(obj)),
        Op::EnsureOwned { reg } => Some(Mutation::Owned(reg)),
        Op::CrdtBump { .. }
        | Op::CrdtMerge { .. }
        | Op::CrdtJoin { .. }
        | Op::CrdtAppend { .. }
        | Op::CrdtResolve { .. } => Some(Mutation::Shared),
        _ => None,
    }
}

/// The registers `op` reads without leaving their handles anywhere: a measure, a lookup, a comparison, a
/// rendering, a copy made from them.
fn inspected_regs(op: &Op) -> Vec<u16> {
    match *op {
        Op::RegionBoundsGuard {
            array, bound, iv, ..
        } => vec![array, bound, iv],
        Op::Length { collection, .. } => vec![collection],
        Op::Index {
            collection, index, ..
        }
        | Op::IndexUnchecked {
            collection, index, ..
        } => vec![collection, index],
        Op::Contains {
            collection, value, ..
        } => vec![collection, value],
        Op::Show { src }
        | Op::FormatValue { src, .. }
        | Op::Not { src, .. }
        | Op::DeepClone { src, .. } => vec![src],
        Op::Concat { lhs, rhs, .. }
        | Op::Eq { lhs, rhs, .. }
        | Op::NotEq { lhs, rhs, .. }
        | Op::Lt { lhs, rhs, .. }
        | Op::Gt { lhs, rhs, .. }
        | Op::LtEq { lhs, rhs, .. }
        | Op::GtEq { lhs, rhs, .. }
        | Op::ApproxEq { lhs, rhs, .. }
        | Op::SeqConcat { lhs, rhs, .. }
        | Op::UnionOp { lhs, rhs, .. }
        | Op::IntersectOp { lhs, rhs, .. } => vec![lhs, rhs],
        Op::SliceOp {
            collection,
            start,
            end,
            ..
        } => vec![collection, start, end],
        Op::GetField { obj, .. } => vec![obj],
        Op::TestArm { target, .. } | Op::BindArm { target, .. } => vec![target],
        Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. } => vec![cond],
        Op::AddAssign { src, .. } => vec![src],
        Op::CheckPolicy {
            subject, object, ..
        } => vec![subject, object],
        Op::NewEmptyListWithCapacity { capacity, .. }
        | Op::NewEmptyMapWithCapacity { capacity, .. } => vec![capacity],
        _ => Vec::new(),
    }
}

/// Whether a register of kind `k` holds a value with no heap handle an owned flag could ride.
fn carries_no_handle(k: Kind) -> bool {
    matches!(
        k,
        Kind::Int
            | Kind::Bool
            | Kind::Float
            | Kind::BigInt
            | Kind::Word(_)
            | Kind::Nothing
            | Kind::Never
    )
}

/// The registers `op` writes with a collection it just made, which nothing else holds.
fn fresh_regs(op: &Op, after: &[Kind]) -> Vec<u16> {
    use crate::semantics::builtins::BuiltinId;
    match *op {
        Op::NewList { dst, .. }
        | Op::NewEmptyList { dst }
        | Op::NewEmptyListWithCapacity { dst, .. }
        | Op::NewEmptyListFixed { dst, .. }
        | Op::NewEmptyListI32 { dst }
        | Op::NewEmptySet { dst }
        | Op::NewEmptyMap { dst }
        | Op::NewEmptyMapWithCapacity { dst, .. }
        | Op::NewRange { dst, .. }
        | Op::SeqConcat { dst, .. }
        | Op::SliceOp { dst, .. }
        | Op::UnionOp { dst, .. }
        | Op::IntersectOp { dst, .. }
        | Op::NewStruct { dst, .. }
        | Op::Args { dst } => vec![dst],
        Op::DeepClone { dst, .. } if !matches!(kind_at(after, dst), Kind::CrdtCounter) => vec![dst],
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::TextBytes,
            ..
        } => vec![dst],
        _ => Vec::new(),
    }
}

fn jump_target(op: &Op) -> Option<usize> {
    match op {
        Op::Jump { target } | Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
            Some(*target)
        }
        // A `for-each` loop's exhaustion edge: `IterNext` falls through to `L{exit}` when the iterator is empty.
        Op::IterNext { exit, .. } => Some(*exit),
        _ => None,
    }
}

/// The VM's refusal when `op` reads exact register `reg` as the index of a list or a Text and it is past a machine
/// word; `None` where it reads `reg` some other way, whose narrowing fails as any place holding a machine word does.
fn index_too_wide(
    op: &Op,
    kinds: &[Kind],
    reg: u16,
) -> Option<crate::semantics::failcode::FailCode> {
    use crate::semantics::failcode::FailCode;
    match *op {
        Op::Index {
            collection, index, ..
        }
        | Op::IndexUnchecked {
            collection, index, ..
        } if index == reg => match kind_at(kinds, collection) {
            Kind::Text => Some(FailCode::TextIndexTooWide),
            k if is_sequence(k) => Some(FailCode::ListIndexTooWide),
            _ => None,
        },
        Op::SetIndex {
            collection, index, ..
        }
        | Op::SetIndexUnchecked {
            collection, index, ..
        } if index == reg && is_sequence(kind_at(kinds, collection)) => {
            Some(FailCode::ListStoreIndexTooWide)
        }
        _ => None,
    }
}

/// A map's value is stored as an exact integer, independently of its key.
fn reads_exact_reg(op: &Op, kinds: &[Kind], reg: u16) -> bool {
    reads_exact(op, kinds)
        || matches!(op, Op::CrdtBump { amount, .. } if *amount == reg)
        || matches!(op, Op::StructInsert { value, .. } | Op::CrdtResolve { value, .. } if *value == reg)
        || matches!(op,
            Op::SetIndex { collection, value, .. }
            | Op::SetIndexUnchecked { collection, value, .. }
            if *value == reg && matches!(kind_at(kinds, *collection), Kind::Map(_) | Kind::MapUnknown | Kind::Struct(_) | Kind::StructSynth(_))
        )
}

/// Whether `op`'s lowering reads an exact-integer operand as one (a tagged word) — for these kinds of its operands.
fn reads_exact(op: &Op, kinds: &[Kind]) -> bool {
    match op {
        Op::Shl { lhs, .. } | Op::Shr { lhs, .. } => !matches!(kind_at(kinds, *lhs), Kind::Word(_)),
        Op::FormatValue { spec, .. } => *spec == u32::MAX,
        Op::Move { .. }
        | Op::DeepClone { .. }
        | Op::Add { .. }
        | Op::Sub { .. }
        | Op::Mul { .. }
        | Op::Pow { .. }
        | Op::Div { .. }
        | Op::FloorDiv { .. }
        | Op::Mod { .. }
        | Op::DivPow2 { .. }
        | Op::MagicDivU { .. }
        | Op::AddAssign { .. }
        | Op::BitXor { .. }
        | Op::BitAnd { .. }
        | Op::BitOr { .. }
        | Op::Lt { .. }
        | Op::Gt { .. }
        | Op::LtEq { .. }
        | Op::GtEq { .. }
        | Op::Eq { .. }
        | Op::NotEq { .. }
        | Op::Show { .. }
        | Op::Concat { .. }
        | Op::Return { .. }
        | Op::Call { .. }
        | Op::EnsureOwned { .. }
        | Op::GlobalSet { .. }
        | Op::RevSwap { .. } => true,
        // A word built from an exact integer takes its low bits, however wide it is.
        Op::CallBuiltin {
            builtin:
                crate::semantics::builtins::BuiltinId::Word8Make
                | crate::semantics::builtins::BuiltinId::Word16Make
                | crate::semantics::builtins::BuiltinId::Word32
                | crate::semantics::builtins::BuiltinId::Word64,
            ..
        } => true,
        _ => false,
    }
}

/// The operands `x` for which `c * x` is within i64, as the closed range `(lo, hi)` (the whole range when `c` is zero).
fn product_domain(c: i128) -> (i128, i128) {
    let (min, max) = (i64::MIN as i128, i64::MAX as i128);
    let floor_div = |n: i128, d: i128| {
        let q = n / d;
        if n % d != 0 && ((n < 0) != (d < 0)) {
            q - 1
        } else {
            q
        }
    };
    let ceil_div = |n: i128, d: i128| -floor_div(-n, d);
    let (lo, hi) = match c {
        0 => (min, max),
        c if c > 0 => (ceil_div(min, c), floor_div(max, c)),
        c => (ceil_div(max, c), floor_div(min, c)),
    };
    (lo.max(min), hi.min(max))
}

/// Whether `op` can make an integer past i64 out of machine words: a sum, difference, product, power or quotient.
fn overflows(op: &Op) -> bool {
    matches!(
        op,
        Op::Add { .. }
            | Op::Sub { .. }
            | Op::Mul { .. }
            | Op::AddAssign { .. }
            | Op::Pow { .. }
            | Op::Div { .. }
            | Op::FloorDiv { .. }
    )
}

/// Whether `op`'s lowering writes its result tagged when the result is exact (it reads the result's kind).
fn makes_exact(op: &Op) -> bool {
    matches!(
        op,
        Op::Add { .. }
            | Op::Sub { .. }
            | Op::Mul { .. }
            | Op::Pow { .. }
            | Op::Div { .. }
            | Op::FloorDiv { .. }
            | Op::Mod { .. }
            | Op::DivPow2 { .. }
            | Op::MagicDivU { .. }
            | Op::AddAssign { .. }
    )
}

/// A single-replica CRDT-lattice op (needs the `C_CRDT_RUNTIME` counter map + `logos_crdt_*` helpers).
fn op_is_crdt(op: &Op) -> bool {
    matches!(
        op,
        Op::NewCrdt { .. }
            | Op::CrdtBump { .. }
            | Op::CrdtMerge { .. }
            | Op::CrdtJoin { .. }
            | Op::CrdtResolve { .. }
            | Op::CrdtAppend { .. }
    )
}

/// A file statement's builtin (needs the `c_vfs_runtime` sandboxed file system + `logos_vfs_*` helpers).
fn op_is_file(op: &Op) -> bool {
    use crate::semantics::builtins::BuiltinId;
    matches!(
        op,
        Op::CallBuiltin {
            builtin: BuiltinId::VfsReadFile | BuiltinId::VfsWriteFile | BuiltinId::VfsMount,
            ..
        }
    )
}

/// An offline-networking op (needs the `c_net_runtime` loopback transport + `logos_net_*` helpers).
fn op_is_net(op: &Op) -> bool {
    matches!(
        op,
        Op::NetConnect { .. }
            | Op::NetListen { .. }
            | Op::NetMakePeer { .. }
            | Op::NetSend { .. }
            | Op::NetStream { .. }
            | Op::NetAwait { .. }
            | Op::NetSync { .. }
    )
}

/// A real-socket TCP-client op (needs the `C_TCP_RUNTIME` POSIX socket runtime + `logos_tcp_*` helpers).
fn op_is_tcp(op: &Op) -> bool {
    matches!(op, Op::Tcp { .. })
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_c/from_program/tests.rs"]
mod tests;
