//! AST → bytecode compiler.
//!
//! Layout: Main's bytecode is emitted first (entry pc 0) and ends with `Halt`;
//! each function body is appended after it and reached via `Call` (an absolute
//! jump to its `entry_pc`). Function names are registered in a first pass so
//! forward references and (mutual) recursion resolve.
//!
//! Locals are assigned registers as they are first bound; expression temporaries
//! get fresh registers above them, so a call's argument block never overlaps a
//! live caller local.

use std::collections::HashMap;

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, ObsCategory, Stmt};
use crate::intern::{Interner, Symbol};

use super::instruction::{
    BoundaryType, CompiledFunction, CompiledProgram, ConstIdx, Constant, FnDebugInfo, FuncIdx,
    ObsSiteDesc, ObsSiteKind, Op, Reg, StructTypeDef,
};
use super::{MAX_EXPR_DEPTH, MAX_REGISTERS_PER_FRAME};

/// Why the bytecode compiler refused a program, and — when the refusal is about something the
/// author wrote — where they wrote it.
///
/// `at` is a span in the parsed program's coordinates, attached where the refusal is DECIDED: the
/// element, handler, route or page the message names. A limit of the compiler itself (the register
/// or constant pools, nesting depth) was caused by no single construct and carries `None`. Every
/// surface renders the location from here rather than recovering it from the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub message: String,
    pub at: Option<crate::token::Span>,
}

impl Refusal {
    /// A refusal about the construct the author wrote at `span`.
    pub fn at(span: crate::token::Span, message: impl Into<String>) -> Self {
        Refusal {
            message: message.into(),
            at: Some(span),
        }
    }

    /// The sentence alone, for a caller that reports text.
    pub fn into_message(self) -> String {
        self.message
    }
}

impl From<String> for Refusal {
    fn from(message: String) -> Self {
        Refusal { message, at: None }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// A refusal as the diagnostic every source-pointing surface already renders: the message, at the
/// construct's span. A span-less refusal reports at the start of the program, the one place a
/// program-wide limit can be said to be.
impl From<Refusal> for crate::error::ParseError {
    fn from(r: Refusal) -> Self {
        crate::error::ParseError {
            kind: crate::error::ParseErrorKind::Custom(r.message),
            span: r.at.unwrap_or_default(),
        }
    }
}

/// Map the AST speech-act category to the observation channel's category
/// (Wave 13). The channel type is interner-free so sinks need no AST dependency.
fn map_obs_cat(c: ObsCategory) -> crate::observe::ObsCat {
    match c {
        ObsCategory::Note => crate::observe::ObsCat::Note,
        ObsCategory::Warn => crate::observe::ObsCat::Warn,
        ObsCategory::Record => crate::observe::ObsCat::Record,
    }
}

/// A loop-local fresh sequence which is transferred into a loop-carried
/// sequence at the end of the iteration.  Lowering that transfer to a register
/// exchange leaves the old outer buffer in `inner`; the next `NewEmptyList`
/// can therefore clear and refill it instead of allocating again.
#[derive(Clone, Copy)]
struct LoopBufferSwap {
    inner: Symbol,
    outer: Symbol,
    set_idx: usize,
}

fn unit_increment(stmt: &Stmt) -> bool {
    let Stmt::Set { target, value } = stmt else {
        return false;
    };
    matches!(
        value,
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } if (matches!(left, Expr::Identifier(sym) if sym == target)
            && matches!(right, Expr::Literal(Literal::Number(1))))
            || (matches!(left, Expr::Literal(Literal::Number(1)))
                && matches!(right, Expr::Identifier(sym) if sym == target))
    )
}

/// Recognize the same conservative ping-pong shape as the Rust and C AST
/// emitters.  This lives at bytecode lowering so VM, C-bytecode, and every JIT
/// consuming the bytecode receive the optimization together.
fn detect_loop_buffer_swap(body: &[Stmt], interner: &Interner) -> Option<LoopBufferSwap> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Peephole) || body.len() < 2
    {
        return None;
    }

    let inner = body.iter().take(2).find_map(|stmt| {
        let Stmt::Let {
            var,
            value:
                Expr::New {
                    type_name,
                    init_fields,
                    ..
                },
            mutable: true,
            ..
        } = stmt
        else {
            return None;
        };
        (matches!(interner.resolve(*type_name), "Seq" | "List" | "Vec") && init_fields.is_empty())
            .then_some(*var)
    })?;

    let (outer, set_idx) = body.iter().enumerate().rev().find_map(|(idx, stmt)| {
        let Stmt::Set {
            target,
            value: Expr::Identifier(source),
        } = stmt
        else {
            return None;
        };
        (*source == inner && *target != inner).then_some((*target, idx))
    })?;

    if set_idx + 1 == body.len() {
        // The ownership transfer is the final statement.
    } else if set_idx + 2 == body.len() && unit_increment(&body[set_idx + 1]) {
        // The only following statement advances the loop counter.
    } else {
        return None;
    }

    Some(LoopBufferSwap {
        inner,
        outer,
        set_idx,
    })
}

/// Whether the W24 constant-divisor magic-reciprocal lowering is enabled.
/// Default ON (it is bit-identical to `Op::Div`/`Op::Mod` for the proven
/// non-negative dividend it gates on, validated by the corpus differential).
/// `LOGOS_MAGIC_DIV=0` is the kill-switch — also the A/B handle for measuring
/// the lever's effect on a quiet box. Read once.
pub fn magic_div_enabled() -> bool {
    crate::optimize::active_config().is_on(crate::optimization::Opt::FastDiv)
}

/// Narrow a `new Seq of Int` whose elements provably fit `i32` into half-width
/// (`Vec<i32>`) VM storage — the interpreter analogue of the AOT `Vec<i64>`→
/// `Vec<i32>` narrowing, halving the array footprint and cache pressure. The JIT
/// region path is i32-array aware (the hot region still tiers — 0 bails). PROMOTED
/// 2026-06-21: default ON (kill-switch LOGOS_NARROW_VM=0) — graph_bfs -27.6% and
/// array_fill -16.7% on the faithful interleaved A/B, all 33 benchmarks
/// bit-identical, full suite 10341/10341 green, no regression.
///
/// Read fresh on each body-compile (not memoized) so a process can compile both
/// regimes — the A/B differential harness and the in-process tests rely on this.
/// The cost is one env lookup per function body, never per op.
pub fn narrow_vm_enabled() -> bool {
    crate::optimize::active_config().is_on(crate::optimization::Opt::NarrowVm)
}

/// Precompute the unsigned magic-reciprocal constants `(magic, more)` for a
/// constant divisor `c` (`c != 0`), reusing the single canonical
/// [`logicaffeine_data::LogosDivU64`] generator — the same algorithm the AOT
/// `compile_to_rust` path emits, exhaustively proven against hardware `/`/`%`.
/// The `more` byte encoding (low 6 bits = shift, `0x40` = the 65-bit add-marker
/// path, `0x80` = the pure-shift power-of-two path) is exactly what
/// [`magic_eval`] consumes.
pub fn magic_u64_gen(c: u64) -> (u64, u8) {
    logicaffeine_data::LogosDivU64::new(c).parts()
}

/// Evaluate the magic reciprocal: `x / c` when `mul_back == 0`, else `x % c`
/// (`mul_back == c`). `x` is the dividend reinterpreted as `u64` — sound because
/// the op is emitted ONLY for a proven non-negative `x`, where the i64 bit
/// pattern equals the mathematical value and the unsigned quotient/remainder
/// equal the signed truncating ones. The remainder is `x - q*c` computed in
/// wrapping i64 (the low 64 bits agree with the u64 difference), bit-exact with
/// the kernel's `wrapping_rem` for non-negative `x`.
pub fn magic_eval(x: i64, magic: u64, more: u8, mul_back: i64) -> i64 {
    const SHIFT_MASK: u8 = 0x3F;
    const ADD_MARKER: u8 = 0x40;
    const SHIFT_PATH: u8 = 0x80;
    let n = x as u64;
    let q = if more & SHIFT_PATH != 0 {
        n >> (more & SHIFT_MASK)
    } else {
        let hi = (((magic as u128) * (n as u128)) >> 64) as u64;
        if more & ADD_MARKER != 0 {
            let t = (n.wrapping_sub(hi) >> 1).wrapping_add(hi);
            t >> (more & SHIFT_MASK)
        } else {
            hi >> (more & SHIFT_MASK)
        }
    };
    if mul_back == 0 {
        q as i64
    } else {
        x.wrapping_sub((q as i64).wrapping_mul(mul_back))
    }
}

/// Constant-pool dedup key. Floats are keyed by bit pattern so that distinct
/// NaN payloads and -0.0/0.0 stay distinct pool entries.
#[derive(Clone, PartialEq, Eq, Hash)]
enum ConstKey {
    Int(i64),
    FloatBits(u64),
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

fn const_key(c: &Constant) -> ConstKey {
    match c {
        Constant::Int(n) => ConstKey::Int(*n),
        Constant::Float(f) => ConstKey::FloatBits(f.to_bits()),
        Constant::Bool(b) => ConstKey::Bool(*b),
        Constant::Text(s) => ConstKey::Text(s.clone()),
        Constant::Char(c) => ConstKey::Char(*c),
        Constant::Nothing => ConstKey::Nothing,
        Constant::Duration(n) => ConstKey::Duration(*n),
        Constant::Date(d) => ConstKey::Date(*d),
        Constant::Moment(n) => ConstKey::Moment(*n),
        Constant::Span { months, days } => ConstKey::Span {
            months: *months,
            days: *days,
        },
        Constant::Time(n) => ConstKey::Time(*n),
    }
}

/// Which i64-slot kind a DECLARED scalar type compiles to, if any.
fn slot_kind_of_type(
    ty: &crate::ast::stmt::TypeExpr,
    interner: &Interner,
) -> Option<super::native_tier::SlotKind> {
    use super::native_tier::SlotKind;
    use crate::ast::stmt::TypeExpr;
    match ty {
        // Float reaches the AST as Named (the Primitive set is Int, Nat,
        // Text, Bool) — accept the scalar names through either variant.
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => match interner.resolve(*sym) {
            "Int" | "Nat" => Some(SlotKind::Int),
            // DOM handles are nominal in the checker and i64 handles at runtime.
            "Element" | "Event" | "View" => Some(SlotKind::Int),
            "Float" => Some(SlotKind::Float),
            "Bool" => Some(SlotKind::Bool),
            // A `Word32` element type — a `Seq of Word32` param pins as a `Words32` buffer.
            "Word32" => Some(SlotKind::Word32),
            // A `Word64` rides as its bit pattern; a `Seq of Word64` has no packed list (see `param_kind_of_type`).
            "Word64" => Some(SlotKind::Word64),
            _ => None,
        },
        TypeExpr::Refinement { base, .. } => slot_kind_of_type(base, interner),
        _ => None,
    }
}

/// A DECLARED parameter's native representation: scalar slot or pinned
/// `Seq of <scalar>`.
fn param_kind_of_type(
    ty: &crate::ast::stmt::TypeExpr,
    interner: &Interner,
) -> Option<super::native_tier::ParamKind> {
    use super::native_tier::{ParamKind, PinElem, SlotKind};
    use crate::ast::stmt::TypeExpr;
    if let TypeExpr::Mutable { inner } = ty {
        return param_kind_of_type(inner, interner);
    }
    if let Some(k) = slot_kind_of_type(ty, interner) {
        return Some(ParamKind::Scalar(k));
    }
    if let TypeExpr::Generic { base, params } = ty {
        if interner.resolve(*base) == "Seq" && params.len() == 1 {
            return match slot_kind_of_type(&params[0], interner) {
                Some(SlotKind::Int) => Some(ParamKind::List(PinElem::Int)),
                Some(SlotKind::Float) => Some(ParamKind::List(PinElem::Float)),
                Some(SlotKind::Bool) => Some(ParamKind::List(PinElem::Bool)),
                // A `Seq of Word32` param pins as a `Words32` buffer (the sha256 `h`/`m`/`kk` schedule +
                // constants) — reuses the IntList 8-byte pin, tags reads `Word32`. This lets a function
                // taking Word32-list params (sha256Compress) tier instead of being marked `Failed`.
                Some(SlotKind::Word32) => Some(ParamKind::List(PinElem::Word32)),
                // `CharText` is a forge-only write-back carrier, never a declared element type.
                Some(SlotKind::CharText) => None,
                // A `Seq of Word64` has no packed list repr yet (no `Words64` `ListRepr`) — declines.
                Some(SlotKind::Word64) => None,
                None => None,
            };
        }
    }
    if let TypeExpr::Refinement { base, .. } = ty {
        return param_kind_of_type(base, interner);
    }
    None
}

/// Resolve a type NAME to its full [`BoundaryType`] — a scalar/temporal primitive, or (if the name
/// is a user type) a `Struct`/`Enum` reference (`user_types[name]` = true for struct, false for
/// enum). `None` for an unmodeled type (Map, generic param…).
fn boundary_of_name(
    s: Symbol,
    user_types: &std::collections::HashMap<Symbol, bool>,
    interner: &Interner,
) -> Option<BoundaryType> {
    Some(match interner.resolve(s) {
        "Int" | "Nat" | "Element" | "Event" | "View" => BoundaryType::Int,
        "Float" | "Real" => BoundaryType::Float,
        "Bool" => BoundaryType::Bool,
        "Text" => BoundaryType::Text,
        "Date" => BoundaryType::Date,
        "Moment" => BoundaryType::Moment,
        "Word32" => BoundaryType::Word32,
        "Word64" => BoundaryType::Word64,
        name @ ("Duration" | "Time" | "Span" | "Rational" | "Complex" | "Decimal" | "Modular"
        | "Money" | "Quantity" | "Uuid" | "Algebraic" | "ExactReal" | "Hyperreal" | "Hypernatural"
        | "Ordinal") => BoundaryType::Builtin(name.to_string()),
        other => match user_types.get(&s) {
            Some(true) => BoundaryType::Struct(other.to_string()),
            Some(false) => BoundaryType::Enum(other.to_string()),
            None => return None,
        },
    })
}

/// Resolve a parameter's AST [`TypeExpr`] to its [`BoundaryType`]. `None` for an unmodeled type.
fn boundary_of_type_expr(
    ty: &crate::ast::stmt::TypeExpr,
    user_types: &std::collections::HashMap<Symbol, bool>,
    interner: &Interner,
) -> Option<BoundaryType> {
    use crate::ast::stmt::TypeExpr;
    match ty {
        TypeExpr::Primitive(s) | TypeExpr::Named(s) => boundary_of_name(*s, user_types, interner),
        TypeExpr::Generic { base, params }
            if matches!(interner.resolve(*base), "Seq" | "List") && params.len() == 1 =>
        {
            boundary_of_type_expr(&params[0], user_types, interner)
                .map(|e| BoundaryType::Seq(Box::new(e)))
        }
        // `Map of K to V` — resolve both the key and value element types (carried so a parameter
        // map's `item k of m` resolves its result kind); rejected if either is unresolvable.
        TypeExpr::Generic { base, params }
            if matches!(interner.resolve(*base), "Map" | "HashMap") && params.len() == 2 =>
        {
            let k = boundary_of_type_expr(&params[0], user_types, interner)?;
            let v = boundary_of_type_expr(&params[1], user_types, interner)?;
            Some(BoundaryType::Map(Box::new(k), Box::new(v)))
        }
        // `Set of <elem>` — one i32 handle, resolved so a `Set of Int`/`Set of Text` PARAMETER seeds
        // its self-describing kind (`Kind::Set`/`Kind::SetText`) and a `-> Set of …` RETURN threads it.
        TypeExpr::Generic { base, params }
            if matches!(interner.resolve(*base), "Set" | "HashSet") && params.len() == 1 =>
        {
            boundary_of_type_expr(&params[0], user_types, interner)
                .map(|e| BoundaryType::Set(Box::new(e)))
        }
        // A HOMOGENEOUS tuple (`Pair of Int and Int`, `Triple of Int and Int and Int`) lays out
        // identically to a `Seq` of the shared element kind, so it resolves through the same seq path
        // (`item N of t` = seq index). A heterogeneous tuple has no single AOT element kind and stays
        // rejected — exactly as a heterogeneous tuple LITERAL is rejected at lowering.
        TypeExpr::Generic { base, params }
            if matches!(interner.resolve(*base), "Pair" | "Triple") =>
        {
            let elems: Vec<BoundaryType> = params
                .iter()
                .filter_map(|p| boundary_of_type_expr(p, user_types, interner))
                .collect();
            if elems.len() != params.len() {
                None
            } else if elems.windows(2).all(|w| w[0] == w[1]) {
                Some(BoundaryType::Seq(Box::new(elems[0].clone())))
            } else {
                Some(BoundaryType::Tuple(elems))
            }
        }
        // An ANONYMOUS tuple type `(Int, Int)` / `(Int, Text)` — identical resolution to `Pair`/`Triple`:
        // a homogeneous tuple lays out as a `Seq` of the shared element (so `item N of t` = seq index),
        // a heterogeneous one is a `Tuple` of per-position kinds.
        TypeExpr::Tuple(params) => {
            let elems: Vec<BoundaryType> = params
                .iter()
                .filter_map(|p| boundary_of_type_expr(p, user_types, interner))
                .collect();
            if elems.len() != params.len() {
                None
            } else if elems.windows(2).all(|w| w[0] == w[1]) {
                Some(BoundaryType::Seq(Box::new(elems[0].clone())))
            } else {
                Some(BoundaryType::Tuple(elems))
            }
        }
        // `Result of <ok> and <err>` — an enum written at its use site, never declared, so its
        // payload types are carried structurally rather than looked up by name. This is what lets
        // a `When Err (e)` read a field of a RECORD payload: the arm resolves its layout from the
        // declared type, exactly as a named enum's arm resolves it from `enum_types`.
        // ★ AN `Option of <inner>` RESOLVES THE SAME WAY, and used to fall to `_ => None` below —
        // so a declared `-> Option of Int` told the backends nothing, and a `When Some (v):` arm
        // had no type for `v`. `Maybe` is the documented alias (`registry.rs` gives both arity 1).
        TypeExpr::Generic { base, params }
            if matches!(interner.resolve(*base), "Option" | "Maybe") && params.len() == 1 =>
        {
            let inner = boundary_of_type_expr(&params[0], user_types, interner)?;
            Some(BoundaryType::Option(Box::new(inner)))
        }
        TypeExpr::Generic { base, params }
            if interner.resolve(*base) == "Result" && params.len() == 2 =>
        {
            let ok = boundary_of_type_expr(&params[0], user_types, interner)?;
            let err = boundary_of_type_expr(&params[1], user_types, interner)?;
            Some(BoundaryType::Result(Box::new(ok), Box::new(err)))
        }
        TypeExpr::Refinement { base, .. } => boundary_of_type_expr(base, user_types, interner),
        TypeExpr::Mutable { inner } => boundary_of_type_expr(inner, user_types, interner),
        _ => None,
    }
}

/// Resolve a struct FIELD's registry [`FieldType`] to its [`BoundaryType`] (the registry, unlike the
/// lossy `struct_defs` map, preserves a `Seq of <elem>` element).
fn boundary_of_field_type(
    ft: &crate::analysis::registry::FieldType,
    user_types: &std::collections::HashMap<Symbol, bool>,
    interner: &Interner,
) -> Option<BoundaryType> {
    use crate::analysis::registry::FieldType;
    match ft {
        // A single-replica counter CRDT field (`ConvergentCount`/`GCounter`/`Tally`/`PNCounter`) lays out as
        // its underlying Int (the net count), exactly as a `SharedMap` field resolves as its `Map`. This lets a
        // `Shared` struct carrying a counter resolve its field layout (the CRDT machinery bumps/reads that int
        // slot in place), so the struct is a valid heap-plane type — the whole-program fabric resolves the field.
        FieldType::Named(s)
            if matches!(
                interner.resolve(*s),
                "ConvergentCount" | "GCounter" | "Tally" | "PNCounter"
            ) =>
        {
            Some(BoundaryType::Int)
        }
        // A single-replica set / sequence CRDT field (`SharedSet of T` / `SharedSequence of T` and their
        // variants) lays out as its underlying `Seq of T` (the live converged collection), exactly as a
        // counter field resolves as its Int and a `SharedMap` as its `Map`.
        FieldType::Generic { base, params }
            if matches!(
                interner.resolve(*base),
                "SharedSet"
                    | "ORSet"
                    | "SharedSet_AddWins"
                    | "SharedSet_RemoveWins"
                    | "SharedSequence"
                    | "RGA"
                    | "SharedSequence_YATA"
                    | "CollaborativeSequence"
            ) && params.len() == 1 =>
        {
            boundary_of_field_type(&params[0], user_types, interner)
                .map(|e| BoundaryType::Seq(Box::new(e)))
        }
        // A multi-value register CRDT field (`Divergent T`) lays out as its underlying value type T: a
        // single-replica LWW resolve keeps the last value written, so the field is that value's slot
        // (exactly as a counter field resolves as its Int). This lets a `Shared` struct with a register
        // field resolve its layout instead of being silently dropped.
        FieldType::Generic { base, params }
            if interner.resolve(*base) == "Divergent" && params.len() == 1 =>
        {
            boundary_of_field_type(&params[0], user_types, interner)
        }
        FieldType::Primitive(s) | FieldType::Named(s) => boundary_of_name(*s, user_types, interner),
        FieldType::Generic { base, params }
            if matches!(interner.resolve(*base), "Seq" | "List") && params.len() == 1 =>
        {
            boundary_of_field_type(&params[0], user_types, interner)
                .map(|e| BoundaryType::Seq(Box::new(e)))
        }
        // A `Set of T` field — the same `Set` boundary a `Set of T` parameter resolves to, so a record
        // carrying a set declares what its set holds for the record's whole life.
        FieldType::Generic { base, params }
            if matches!(interner.resolve(*base), "Set" | "HashSet") && params.len() == 1 =>
        {
            boundary_of_field_type(&params[0], user_types, interner)
                .map(|e| BoundaryType::Set(Box::new(e)))
        }
        // A `Map of K to V` field — resolve both element types, so a struct carrying a map can be a
        // parameter and `item k of s's m` resolves the value kind cross-region. A `SharedMap from K to V`
        // (single-replica ORMap CRDT) lays out as its underlying Map, so it resolves the same way.
        FieldType::Generic { base, params }
            if matches!(interner.resolve(*base), "Map" | "HashMap" | "SharedMap")
                && params.len() == 2 =>
        {
            let k = boundary_of_field_type(&params[0], user_types, interner)?;
            let v = boundary_of_field_type(&params[1], user_types, interner)?;
            Some(BoundaryType::Map(Box::new(k), Box::new(v)))
        }
        // A HOMOGENEOUS tuple field (`Pair of Int and Int`, `Triple of …`) lays out identically to a
        // `Seq` of the shared element kind, so it resolves through the seq path (`item N of s's t` =
        // seq index). A heterogeneous tuple field has no single AOT element kind and stays rejected.
        FieldType::Generic { base, params }
            if matches!(interner.resolve(*base), "Pair" | "Triple") =>
        {
            let elems: Vec<BoundaryType> = params
                .iter()
                .filter_map(|p| boundary_of_field_type(p, user_types, interner))
                .collect();
            if elems.len() != params.len() {
                None
            } else if elems.windows(2).all(|w| w[0] == w[1]) {
                Some(BoundaryType::Seq(Box::new(elems[0].clone())))
            } else {
                Some(BoundaryType::Tuple(elems))
            }
        }
        // A `Result of <ok> and <err>` FIELD — the third boundary position, resolved structurally
        // like the parameter one (see `boundary_of_type_expr`), so a record carrying an answer can
        // be a parameter and `Inspect s's answer` reads a record payload's field cross-region.
        // An `Option of <inner>` FIELD — the same third boundary position a `Result` field holds.
        FieldType::Generic { base, params }
            if matches!(interner.resolve(*base), "Option" | "Maybe") && params.len() == 1 =>
        {
            let inner = boundary_of_field_type(&params[0], user_types, interner)?;
            Some(BoundaryType::Option(Box::new(inner)))
        }
        FieldType::Generic { base, params }
            if interner.resolve(*base) == "Result" && params.len() == 2 =>
        {
            let ok = boundary_of_field_type(&params[0], user_types, interner)?;
            let err = boundary_of_field_type(&params[1], user_types, interner)?;
            Some(BoundaryType::Result(Box::new(ok), Box::new(err)))
        }
        // A tuple FIELD rides the boundary exactly as a `Pair`/`Triple` field does: a homogeneous
        // one as a `Seq` of its element, a heterogeneous one as a `Tuple`.
        FieldType::Tuple(_) | FieldType::Record(_) => {
            let items: Vec<&FieldType> = match ft {
                FieldType::Tuple(items) => items.iter().collect(),
                FieldType::Record(fields) => fields.iter().map(|(_, t)| t).collect(),
                _ => unreachable!("matched a tuple form"),
            };
            let elems: Vec<BoundaryType> = items
                .iter()
                .filter_map(|p| boundary_of_field_type(p, user_types, interner))
                .collect();
            if elems.len() != items.len() || elems.is_empty() {
                None
            } else if elems.windows(2).all(|w| w[0] == w[1]) {
                Some(BoundaryType::Seq(Box::new(elems[0].clone())))
            } else {
                Some(BoundaryType::Tuple(elems))
            }
        }
        FieldType::TypeParam(_) | FieldType::Generic { .. } => None,
    }
}

/// Resolve the static [`BoundaryType`] of a `Let`-binding's VALUE expression — for a Main top-level
/// binding promoted to a global, so a closure capturing it can be typed with the composite's shape
/// (the capture analog of a parameter's declared type). Handles the constructor forms whose type is
/// statically evident (`new Struct`, `new Map of K to V`, `a new EnumVariant`), and a call to one of
/// the program's functions by `calls`, its declared return type; `None` otherwise (the global stays
/// Kind-only — fine for self-describing values like a `Seq` and for scalars).
fn boundary_of_value_expr(
    e: &crate::ast::stmt::Expr,
    user_types: &std::collections::HashMap<Symbol, bool>,
    interner: &Interner,
    calls: &dyn Fn(Symbol) -> Option<BoundaryType>,
) -> Option<BoundaryType> {
    use crate::ast::stmt::{Expr, Literal};
    match e {
        Expr::New {
            type_name,
            type_args,
            ..
        } => {
            if matches!(interner.resolve(*type_name), "Map" | "HashMap") && type_args.len() == 2 {
                let k = boundary_of_type_expr(&type_args[0], user_types, interner)?;
                let v = boundary_of_type_expr(&type_args[1], user_types, interner)?;
                Some(BoundaryType::Map(Box::new(k), Box::new(v)))
            } else if matches!(interner.resolve(*type_name), "Set" | "HashSet")
                && type_args.len() == 1
            {
                let elem = boundary_of_type_expr(&type_args[0], user_types, interner)?;
                Some(BoundaryType::Set(Box::new(elem)))
            } else if matches!(interner.resolve(*type_name), "Seq" | "List") && type_args.len() == 1
            {
                // ★ `a new Seq of Todo` SAYS WHAT IT HOLDS, and a global that says so is how a
                // reader of it resolves a field of a row. Without this the type was `None`, and a
                // function looping the global had no element layout to resolve against — so a
                // page whose `State:` holds a list of records could not read one field of one row.
                let elem = boundary_of_type_expr(&type_args[0], user_types, interner)?;
                Some(BoundaryType::Seq(Box::new(elem)))
            } else {
                boundary_of_name(*type_name, user_types, interner)
            }
        }
        // ★ A NON-EMPTY MAP LITERAL says it through its first ENTRY — the same rule as the list
        // literal below, one collection along. `{k: v, …}` lowers to the variadic `mapOf` builtin,
        // so that call is what a global actually holds, and without this arm a program writing
        // `Let m be {"a": 1}.` gave the backend a map of unknown value kind — the exact refusal
        // `wasm_map_global_lock` exists for, arriving by the other construction route.
        Expr::Call { function, args }
            if interner.resolve(*function) == "mapOf" && args.len() >= 2 =>
        {
            let k = boundary_of_value_expr(args[0], user_types, interner, calls)?;
            let v = boundary_of_value_expr(args[1], user_types, interner, calls)?;
            Some(BoundaryType::Map(Box::new(k), Box::new(v)))
        }
        // A call says it through the function's declared return type — at any depth, so a
        // component's per-instance state seeded by a template (`mapOf("", badge("a"))`, a map of
        // the program's `__View` enum) is as typed as one seeded by a literal.
        Expr::Call { function, .. } => calls(*function),
        // A non-empty list LITERAL says it through its first element — the other way a program
        // writes a list of records down.
        Expr::List(items) => {
            let first = items.first()?;
            let elem = boundary_of_value_expr(first, user_types, interner, calls)?;
            Some(BoundaryType::Seq(Box::new(elem)))
        }
        Expr::NewVariant { enum_name, .. } => boundary_of_name(*enum_name, user_types, interner),
        // A scalar literal — also resolves a tuple-literal's elements (below).
        Expr::Literal(Literal::Number(_)) => Some(BoundaryType::Int),
        Expr::Literal(Literal::Float(_)) => Some(BoundaryType::Float),
        Expr::Literal(Literal::Boolean(_)) => Some(BoundaryType::Bool),
        Expr::Literal(Literal::Text(_)) => Some(BoundaryType::Text),
        // A tuple literal — every element's type must resolve. A homogeneous tuple lays out like a
        // `Seq` (self-describing, no shape needed); a heterogeneous one carries its per-position kinds.
        Expr::Tuple(elems) if !elems.is_empty() => {
            let bts: Vec<BoundaryType> = elems
                .iter()
                .filter_map(|el| boundary_of_value_expr(el, user_types, interner, calls))
                .collect();
            if bts.len() != elems.len() {
                None
            } else if bts.windows(2).all(|w| w[0] == w[1]) {
                Some(BoundaryType::Seq(Box::new(bts[0].clone())))
            } else {
                Some(BoundaryType::Tuple(bts))
            }
        }
        _ => None,
    }
}

pub struct Compiler<'i> {
    interner: &'i Interner,
    code: Vec<Op>,
    constants: Vec<Constant>,
    const_map: HashMap<ConstKey, ConstIdx>,
    functions: Vec<CompiledFunction>,
    fn_index: HashMap<Symbol, FuncIdx>,
    /// The refusal a `Mount` of each LGX component raises, by component — the cause `lowerable`
    /// found, worded and located, computed once from the statement list ([`mount_refusals`]).
    mount_refusals: HashMap<Symbol, Refusal>,
    /// The refusal each `## Route` this program cannot lower raises, keyed by `(method, path)`.
    ///
    /// Computed once from the WHOLE program, because a `-> View` route's lowerability depends on
    /// the components beside it; the statement arm has no program in hand.
    route_refusals: HashMap<(Symbol, Symbol), Refusal>,
    // Struct definitions: field (name, declared-type, is_public) per struct —
    // from `Stmt::StructDef` and the discovery-pass type registry. Drives
    // default-fill on construction, like the tree-walker's struct_defs.
    struct_defs: HashMap<Symbol, Vec<(Symbol, Symbol, bool)>>,
    // Every enum's own constructor list, in declaration order, from the discovery-pass type
    // registry — G5's "no arm matched" fallthrough reads this to test each UNCOVERED constructor
    // against a compile-time-known message, needing no runtime tag extraction at all (`compile_
    // inspect`). `HashMap::new()` (never populated) when no type registry was given (`Compiler::
    // compile`'s plain entry) — `compile_inspect` falls back to a dynamic message then.
    enum_variants: HashMap<Symbol, Vec<Symbol>>,
    // The reverse index: a constructor's name → the enum it belongs to. Built alongside
    // `enum_variants` from the same registry pass.
    variant_enum: HashMap<Symbol, Symbol>,
    // A constructor's DECLARED field names, in declaration order — mirrors `struct_defs` for
    // enum variants — keyed by `(enum name, constructor name)`. `Expr::NewVariant` compiles its
    // `with`-clause fields in WRITE order but must seat each value at its DECLARED position
    // (`Op::NewInductive`'s `args` are read back positionally by `execute_inspect`'s
    // binding-by-index, so declared order is the only order that keeps a bind arm correct when a
    // constructor lists its fields out of order). Also drives default-fill for a field the `with`
    // clause omits, like `struct_defs` does for structs. Keyed by the constructor alone until this
    // session: two enums declaring a same-named variant with DIFFERENT field shapes (`a new
    // Failure from Outcome`/`a new Failure from Status`, constructible together for the first time
    // by lang5's `from`-qualification) collided on one shared entry, whichever registered second in
    // the discovery pass's own `HashMap` iteration order silently winning.
    variant_fields: HashMap<(Symbol, Symbol), Vec<Symbol>>,
    // Every user type's name → is-it-a-struct (true) / enum (false), built once before statement
    // compilation. Lets a CLOSURE resolve a struct/enum PARAMETER's declared type (the closure path
    // has no other access to the type registry the top-level function path uses).
    user_types: HashMap<Symbol, bool>,
    // Lexical block scopes for the frame currently being compiled (Main or one
    // function body). scopes[0] is the frame root; If/While/Repeat bodies push
    // a child scope whose registers are recycled on exit — mirroring the
    // tree-walker's execute_block undo-scope.
    scopes: Vec<HashMap<Symbol, Reg>>,
    next_reg: Reg,
    /// DEBUG-ONLY: when set (the debugger's compile path), capture source variable
    /// names so the debug drawer can show `x` rather than `R0`. Off in production.
    debug_names: bool,
    /// When FALSE, division/modulo by a constant is NOT strength-reduced to
    /// `DivPow2`/`MagicDivU`/`BitAnd` — plain `Div`/`Mod` are emitted instead.
    /// The soft-core chip backend sets this off: it has a real sequential
    /// `shared_divider`, so magic-reciprocal division (a 64-bit magic constant
    /// that cannot ride the 32-bit chip instruction word) is neither needed nor
    /// representable. Correctness is identical — strength reduction is exact — so
    /// the VM and the chip agree on every result. Default TRUE everywhere else.
    strength_reduce: bool,
    /// DEBUG-ONLY: (register, name-symbol) bound in the CURRENT frame; cleared per
    /// frame in `begin_scope`, resolved into `CompiledProgram::reg_names` for Main.
    dbg_names: Vec<(Reg, Symbol)>,
    /// OBSERVED profile (Wave 13): when set, `Note`/`Warn`/`Record`/`During` lower
    /// to `Op::Obs` + entries in `observation_sites`. Off in the elided
    /// (compiled/release) profile, where observation is erased — so ordinary
    /// programs and every AOT/WASM compile are byte-unchanged.
    emit_observation: bool,
    /// The observation sites accumulated while compiling in the OBSERVED profile,
    /// handed to `CompiledProgram::observation_sites`.
    observation_sites: Vec<ObsSiteDesc>,
    /// `(pc, site_id)` pairs accumulated while compiling, handed to `CompiledProgram::site_at` —
    /// see that field's own doc comment (Wave 3: "runtime failures carry a location").
    site_at: Vec<(u32, u32)>,
    /// TIERS-CHECK mode (`largo test --tiers all`, Wave 13): compile a `## Test`
    /// body's test-only statements (`Expect`/`ForAll`/…) as NO-OPS so the VM can
    /// run the test's COMPUTATION and its agreement with the tree-walker can be
    /// checked (the assertions themselves stay the interpreter's authoritative
    /// verdict). Off everywhere else — a test statement outside the runner is an
    /// error, as before.
    test_noop: bool,
    /// Registers bound to a NAME (Let targets, params, loop variables,
    /// captures) in any frame compiled so far. Everything else is a
    /// statement-local scratch — dead at every statement boundary by the
    /// allocator's recycling discipline. Snapshotted for Main into
    /// `CompiledProgram::named_regs`, where the region JIT uses it to skip
    /// write-back (and pre-state preservation) for scratch slots.
    named: Vec<bool>,
    /// Loop heads (back-edge target pcs) currently being compiled, outermost
    /// first. Every `mark_named` while this is non-empty records the register as
    /// a LOOP-LOCAL of each enclosing loop — bound inside a loop body, so it is
    /// lexically dead the instant the loop exits (the scope is popped).
    loop_stack: Vec<usize>,
    /// `loop_locals[head]` = the registers bound inside the loop whose region
    /// head is `head` (absolute pc). The region JIT subtracts these from the
    /// write-back set: a slot dead at the loop exit must not be written back,
    /// which is what lets copy-prop/CSE/fusion treat it as true scratch. Keyed
    /// by absolute pc (Main and every function share one code array), valued by
    /// the OWNING frame's register indices.
    loop_locals: std::collections::HashMap<usize, Vec<bool>>,
    /// High-water mark of `next_reg` for the current frame — block scopes
    /// recycle registers, so the frame's true size is the maximum ever
    /// reached, not the final `next_reg`.
    max_reg: Reg,
    expr_depth: usize,
    // Enclosing control-flow contexts (innermost last). Loops collect Break
    // jumps; Zones (and Concurrent/Parallel tasks) SWALLOW Break/Return — the
    // tree-walker's execute path discards their ControlFlow.
    flow_stack: Vec<FlowCtx>,
    // Whether we are compiling a function body (false ⇒ Main, where a
    // top-level Break/Return halts the program).
    in_function: bool,
    /// The return type of the function currently being compiled — so a `Return (a, b)` in a
    /// NAMED-RECORD-returning function builds a self-describing struct (`RuntimeValue::Struct` with the
    /// record's field names) rather than a bare tuple, mirroring the interpreter's `coerce_record_return`.
    current_return_type: Option<&'i crate::ast::stmt::TypeExpr<'i>>,
    /// Self-tail-call optimization context for the function currently being
    /// compiled: `(name, entry_pc, param_count)`. A self-call in TAIL position
    /// (`Return f(args)`, or a `Set/Let x = f(args)` immediately followed by
    /// `Return x`) is lowered to "reassign params 0..param_count, jump to
    /// entry_pc" instead of Call+Return — turning self-tail-recursion into a
    /// loop (constant stack, no per-call frame round-trip). `None` outside any
    /// function and for non-self / non-arity-matching calls.
    tail_fn: Option<(Symbol, usize, u16)>,
    // Promoted globals: Main TOP-LEVEL Let names referenced from function or
    // closure bodies, with their slot in the runtime globals table.
    promoted: HashMap<Symbol, u16>,
    // When compiling a closure body: per promoted capture, its (value, flag)
    // frame registers — a captured-global access reads the snapshot when the
    // flag is set and the LIVE global otherwise (tree-walker fall-through).
    closure_ctx: Option<HashMap<Symbol, (Reg, Reg)>>,
    /// Oracle range-analysis facts (M9) for the program being compiled.
    /// Present only on the live run path; when set, an `Index` whose
    /// `index_provably_in_bounds` holds is emitted as `IndexUnchecked`
    /// (bounds-check elimination, V8/LLVM-style). Keyed on the AST `Expr`
    /// pointers of THESE stmts, so the facts must be analyzed on the exact
    /// snapshot being compiled.
    oracle: Option<crate::optimize::OracleFacts>,
    /// Per-enclosing-loop sets of arrays whose region-entry `RegionBoundsGuard`
    /// was successfully emitted. A speculative access elides ONLY when its
    /// array is in some enclosing frame — the compiler's half of the
    /// elision ⟺ guard ⟺ VM-check invariant.
    hoist_enabled: Vec<std::collections::HashSet<Symbol>>,
    /// `new Seq of Int` declarations in the body CURRENTLY being compiled whose
    /// every element provably fits `i32` (the `codegen::narrow` proof). Their
    /// empty-constructor lowers to [`Op::NewEmptyListI32`] (half-width storage),
    /// observably identical to the i64 form. Populated per-body only when
    /// `LOGOS_NARROW_VM=1`; empty otherwise (default OFF).
    narrowable_seqs: std::collections::HashSet<Symbol>,
    /// Exact capacities for non-escaping fixed `Seq` locals, produced by the
    /// same proof Rust and syntax-tree C consume. A proven declaration lowers
    /// to `NewEmptyListFixed`, making support explicit to every bytecode tier.
    fixed_seq_caps: HashMap<(Option<Symbol>, Symbol), usize>,
    /// What the program declares values to be, by the absolute pc of the defining op
    /// ([`CompiledProgram::declared_types`]); sorted when the program is finalized.
    declared_types: Vec<(u32, BoundaryType)>,
    /// Inferred MUTABLE-BORROW params per function: `fn → set of param indices` that
    /// are mutated in place and returned (the AOT's `&mut [T]` set, computed by the
    /// SAME eligibility). Their frame registers are copy-on-write-exempt so the
    /// callee's element writes stay in place; each call site gets an `EnsureOwned`
    /// barrier so an aliasing caller still observes value semantics. Only populated
    /// when the type registry is available (`compile_with_types`).
    mut_borrow_params: HashMap<Symbol, std::collections::HashSet<usize>>,
    /// Per function, the positions of its parameters declared `mutable`: IN-OUT. A call passing a
    /// binding there isolates the binding's value first (so the callee's writes reach no copy of
    /// it) and takes the parameter's final value back into the binding when the call returns (so
    /// every write the callee made — a field, a whole replacement — is the caller's).
    inout_params: HashMap<Symbol, Vec<usize>>,
    /// Functions declared `## To native` (EMPTY bodies), each with the Rust path it names: calls to
    /// the known system natives among them compile to the shared seams instead of the empty body,
    /// and a call no seam takes is refused by name. Membership is the declarer-wins guard — a user
    /// function with a real body is never in this map, so it compiles as an ordinary call.
    native_fns: HashMap<Symbol, Option<Symbol>>,
    /// Per function, the consume-alias LOCAL of each mutable-borrow param (`Let
    /// result be arr`): its register must ALSO be COW-exempt (the in-place writes
    /// land on the alias, not the param). Recorded during body compile.
    mut_borrow_alias_syms: HashMap<Symbol, std::collections::HashSet<Symbol>>,
    /// The alias symbols of the function CURRENTLY being compiled (from
    /// `mut_borrow_alias_syms`); a `let_reg` for one of these appends its register to
    /// `current_exempt_regs`.
    current_exempt_syms: std::collections::HashSet<Symbol>,
    /// Registers bound to `current_exempt_syms` in the current function body — merged
    /// into `CompiledFunction::mutable_param_regs` after the body compiles.
    current_exempt_regs: Vec<Reg>,
    /// Nesting depth of the enclosing `Reversibly:` / `Uncompute:` block. When positive, an
    /// IN-PLACE ABELIAN scalar update (`Set x to x + e` / `x - e` / `x xor e` / `-x`, where `x`
    /// is a local and `e` provably never observes `x`) lowers to the zero-garbage reversible ISA
    /// (`Op::RevAdd`/`RevSub`/`RevXor`/`RevNeg`) instead of the plain journaling op — the region
    /// then reverses with NO history tape. Everything else lowers identically to outside the block,
    /// so program OUTPUT is unchanged; only the emitted ops differ. A counter (not a bool) so a
    /// nested block leaves the outer mode intact on exit.
    reversible_depth: usize,
}

/// Which reversible additive/XOR op a recognized `Set x to x ⊕ e` form lowers to inside a
/// `Reversibly:` block — the delta-carrying members of the zero-garbage abelian core.
enum RevOp {
    Add,
    Sub,
    Xor,
}

/// A mutable argument's place, with every index already evaluated.
struct LentPlace {
    root: Symbol,
    steps: Vec<LentStep>,
}

enum LentStep {
    Field(u32),
    Item(Reg),
}

/// How a name resolves at a point in compilation.
enum NameRef {
    Local(Reg),
    /// A closure capture whose name is also a promoted global: snapshot when
    /// present, live global otherwise.
    CaptureOrGlobal {
        value: Reg,
        flag: Reg,
        global: u16,
    },
    Global(u16),
    Unbound,
}

enum FlowCtx {
    Loop {
        breaks: Vec<usize>,
        /// Repeat loops own a live iterator that must be IterPop'd when a
        /// Return jumps out across them into a Zone.
        is_repeat: bool,
    },
    Zone {
        exits: Vec<usize>,
    },
}

impl<'i> Compiler<'i> {
    /// Compile a statement block to a runnable program.
    pub fn compile(stmts: &[Stmt], interner: &'i Interner) -> Result<CompiledProgram, String> {
        Self::compile_with_types(stmts, interner, None)
    }

    /// Compile with the discovery-pass type registry (struct definitions that
    /// never appear as `Stmt::StructDef`).
    pub fn compile_with_types(
        stmts: &[Stmt],
        interner: &'i Interner,
        types: Option<&crate::analysis::TypeRegistry>,
    ) -> Result<CompiledProgram, String> {
        Self::compile_with_oracle(stmts, interner, types, None)
    }

    /// Like `compile_with_types`, plus the Oracle's range-analysis facts
    /// (M9) for bounds-check elimination. The live run path computes these
    /// on the exact (optimized) snapshot it then compiles, so an `Index`
    /// proven in-bounds becomes `IndexUnchecked`. `None` ⇒ every index
    /// stays checked (the codegen / test paths).
    pub fn compile_with_oracle(
        stmts: &[Stmt],
        interner: &'i Interner,
        types: Option<&crate::analysis::TypeRegistry>,
        oracle: Option<crate::optimize::OracleFacts>,
    ) -> Result<CompiledProgram, String> {
        Self::compile_with_oracle_located(stmts, interner, types, oracle)
            .map_err(Refusal::into_message)
    }

    /// [`Self::compile_with_oracle`], keeping WHERE a refusal happened — the entry a surface that
    /// points at the author's source (the browser build, its dev overlay, an editor) compiles
    /// through. The text-only entries are this, with the location dropped.
    pub fn compile_with_oracle_located(
        stmts: &[Stmt],
        interner: &'i Interner,
        types: Option<&crate::analysis::TypeRegistry>,
        oracle: Option<crate::optimize::OracleFacts>,
    ) -> Result<CompiledProgram, Refusal> {
        Self::compile_inner(stmts, interner, types, oracle, false, false, false, true)
    }

    /// Compile for the soft-core CHIP backend: identical to `compile_with_oracle` except division /
    /// modulo by a constant is NOT strength-reduced (no `DivPow2`/`MagicDivU` — the chip issues plain
    /// `Div`/`Mod` to its sequential `shared_divider`, and a 64-bit magic constant cannot ride the
    /// 32-bit chip instruction word). Results are bit-identical to `compile_with_oracle` (strength
    /// reduction is exact), so the VM and the chip agree.
    pub fn compile_for_chip(
        stmts: &[Stmt],
        interner: &'i Interner,
        types: Option<&crate::analysis::TypeRegistry>,
        oracle: Option<crate::optimize::OracleFacts>,
    ) -> Result<CompiledProgram, String> {
        Self::compile_inner(stmts, interner, types, oracle, false, false, false, false)
            .map_err(Refusal::into_message)
    }

    /// Like `compile_with_types` but also captures source variable names into
    /// `CompiledProgram::reg_names` for the Studio debugger. Used only by the
    /// debugger's compile path; production callers pay nothing for it.
    pub fn compile_for_debug(
        stmts: &[Stmt],
        interner: &'i Interner,
        types: Option<&crate::analysis::TypeRegistry>,
    ) -> Result<CompiledProgram, String> {
        Self::compile_inner(stmts, interner, types, None, true, false, false, true)
            .map_err(Refusal::into_message)
    }

    /// Compile in the OBSERVED profile (Wave 13): `Note`/`Warn`/`Record`/`During`
    /// lower to `Op::Obs`, so the VM emits the same observation event stream the
    /// tree-walker does. The dev/test tier — the compiled/release tiers keep the
    /// elided profile (this op never reaches them).
    pub fn compile_observed(
        stmts: &[Stmt],
        interner: &'i Interner,
        types: Option<&crate::analysis::TypeRegistry>,
    ) -> Result<CompiledProgram, String> {
        Self::compile_observed_located(stmts, interner, types).map_err(Refusal::into_message)
    }

    /// [`Self::compile_observed`], keeping where a refusal happened.
    pub fn compile_observed_located(
        stmts: &[Stmt],
        interner: &'i Interner,
        types: Option<&crate::analysis::TypeRegistry>,
    ) -> Result<CompiledProgram, Refusal> {
        Self::compile_inner(stmts, interner, types, None, false, true, false, true)
    }

    /// Compile a `## Test` body for the `--tiers all` differential (Wave 13):
    /// test-only statements (`Expect`/`ForAll`/…) compile as NO-OPS so the VM can
    /// run the test's computation and confirm it agrees with the tree-walker.
    pub fn compile_tiers(
        stmts: &[Stmt],
        interner: &'i Interner,
        types: Option<&crate::analysis::TypeRegistry>,
    ) -> Result<CompiledProgram, String> {
        Self::compile_inner(stmts, interner, types, None, false, false, true, true)
            .map_err(Refusal::into_message)
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_inner(
        stmts: &[Stmt],
        interner: &'i Interner,
        types: Option<&crate::analysis::TypeRegistry>,
        oracle: Option<crate::optimize::OracleFacts>,
        debug_names: bool,
        emit_observation: bool,
        test_noop: bool,
        strength_reduce: bool,
    ) -> Result<CompiledProgram, Refusal> {
        // Public callers may supply an AST without passing through the parser's depth gate.
        // Bound it iteratively before any recursive analysis visits it.
        if logicaffeine_language::ast_depth::program_depth(stmts)
            > logicaffeine_language::ast_depth::max_ast_depth()
        {
            return Err("vm: expression too deeply nested".to_string().into());
        }
        // How each `Repeat` may iterate: the shared optimizer's proofs, on exactly what this compile lowers.
        crate::optimize::iteration::prove_iteration_facts(stmts, interner);
        let mut c = Compiler {
            interner,
            debug_names,
            strength_reduce,
            dbg_names: Vec::new(),
            emit_observation,
            observation_sites: Vec::new(),
            site_at: vec![(0, crate::vm::instruction::NO_SITE)],
            test_noop,
            code: Vec::new(),
            constants: Vec::new(),
            const_map: HashMap::new(),
            functions: Vec::new(),
            fn_index: HashMap::new(),
            mount_refusals: HashMap::new(),
            route_refusals: HashMap::new(),
            struct_defs: HashMap::new(),
            enum_variants: HashMap::new(),
            variant_enum: HashMap::new(),
            variant_fields: HashMap::new(),
            user_types: HashMap::new(),
            scopes: vec![HashMap::new()],
            next_reg: 0,
            named: Vec::new(),
            loop_stack: Vec::new(),
            loop_locals: std::collections::HashMap::new(),
            max_reg: 0,
            expr_depth: 0,
            flow_stack: Vec::new(),
            tail_fn: None,
            hoist_enabled: Vec::new(),
            in_function: false,
            current_return_type: None,
            promoted: HashMap::new(),
            closure_ctx: None,
            oracle,
            narrowable_seqs: std::collections::HashSet::new(),
            fixed_seq_caps: HashMap::new(),
            declared_types: Vec::new(),
            mut_borrow_params: HashMap::new(),
            inout_params: HashMap::new(),
            native_fns: HashMap::new(),
            mut_borrow_alias_syms: HashMap::new(),
            current_exempt_syms: std::collections::HashSet::new(),
            current_exempt_regs: Vec::new(),
            reversible_depth: 0,
        };

        // Every user type's name → is-it-a-struct (true) or an enum (false) — so a field or
        // parameter referencing a user type resolves to `BoundaryType::Struct`/`Enum`. Collected
        // from both the registry and the pass-1 statements (a type can be defined either way).
        let mut user_types: std::collections::HashMap<Symbol, bool> =
            std::collections::HashMap::new();
        for s in stmts {
            if let Stmt::StructDef { name, .. } = s {
                user_types.insert(*name, true);
            }
        }
        // Enum types are not a top-level `Stmt` — they come from the type registry (below).
        // The bytecode's static struct registry (name → resolved field layout). Built from the type
        // registry, which (unlike the lossy `struct_defs` map below) preserves `Seq` field elements.
        let mut struct_types: Vec<StructTypeDef> = Vec::new();
        // The sum-type companion: name → per-variant payload layout, for typing a `When V (binds)`
        // extraction on an enum PARAMETER (whose construction isn't in scope).
        let mut enum_types: Vec<crate::vm::instruction::EnumTypeDef> = Vec::new();

        // Struct definitions from the type registry (mirrors the tree-walker's
        // with_type_registry) and from StructDef statements (pass 1 below).
        if let Some(registry) = types {
            use crate::analysis::registry::{FieldType, TypeDef};
            for (name_sym, type_def) in registry.iter_types() {
                match type_def {
                    TypeDef::Struct { .. } => {
                        user_types.insert(*name_sym, true);
                    }
                    TypeDef::Enum { .. } => {
                        user_types.insert(*name_sym, false);
                    }
                    _ => {}
                }
            }
            for (name_sym, type_def) in registry.iter_types() {
                if let TypeDef::Struct { fields, .. } = type_def {
                    let field_defs: Vec<(Symbol, Symbol, bool)> = fields
                        .iter()
                        .map(|f| {
                            let type_sym = match &f.ty {
                                FieldType::Primitive(s)
                                | FieldType::Named(s)
                                | FieldType::TypeParam(s) => *s,
                                FieldType::Generic { base, .. } => *base,
                                // A tuple names no type with a scalar default.
                                FieldType::Tuple(_) | FieldType::Record(_) => Symbol::EMPTY,
                            };
                            (f.name, type_sym, f.is_public)
                        })
                        .collect();
                    c.struct_defs.insert(*name_sym, field_defs);
                    // Resolve the FULL field layout; carry the struct only if EVERY field resolves
                    // (a partial layout would mis-place slots), so an unmodeled field type (Map, …)
                    // simply leaves a parameter of that struct type rejected — never miscompiled.
                    let resolved: Option<Vec<(String, BoundaryType)>> = fields
                        .iter()
                        .map(|f| {
                            boundary_of_field_type(&f.ty, &user_types, interner)
                                .map(|t| (interner.resolve(f.name).to_string(), t))
                        })
                        .collect();
                    if let Some(field_layout) = resolved {
                        struct_types.push(StructTypeDef {
                            name: interner.resolve(*name_sym).to_string(),
                            fields: field_layout,
                        });
                    }
                }
            }
            // Every enum's own constructor list (G5's `compile_inspect` fallthrough) — unconditional,
            // unlike `enum_types` below: naming an UNCOVERED constructor needs only its name, never
            // its payload layout, so an unmodeled payload type must not hide the enum from this table.
            for (name_sym, type_def) in registry.iter_types() {
                if let TypeDef::Enum { variants, .. } = type_def {
                    let names: Vec<Symbol> = variants.iter().map(|v| v.name).collect();
                    for v in &names {
                        c.variant_enum.insert(*v, *name_sym);
                    }
                    c.enum_variants.insert(*name_sym, names);
                    for v in variants {
                        let field_names: Vec<Symbol> = v.fields.iter().map(|f| f.name).collect();
                        c.variant_fields.insert((*name_sym, v.name), field_names);
                    }
                }
            }
            // Enum variant payload layouts. Carry the enum only if EVERY variant's EVERY field
            // resolves (a partial layout could mis-type a `BindArm`), so an unmodeled payload type
            // simply leaves a parameter of that enum's payload extraction rejected — never miscompiled.
            for (name_sym, type_def) in registry.iter_types() {
                if let TypeDef::Enum { variants, .. } = type_def {
                    let resolved: Option<Vec<crate::vm::instruction::EnumVariantDef>> = variants
                        .iter()
                        .map(|v| {
                            let field_types: Option<Vec<BoundaryType>> = v
                                .fields
                                .iter()
                                .map(|f| boundary_of_field_type(&f.ty, &user_types, interner))
                                .collect();
                            field_types.map(|ft| crate::vm::instruction::EnumVariantDef {
                                name: interner.resolve(v.name).to_string(),
                                field_types: ft,
                            })
                        })
                        .collect();
                    if let Some(variant_layouts) = resolved {
                        enum_types.push(crate::vm::instruction::EnumTypeDef {
                            name: interner.resolve(*name_sym).to_string(),
                            variants: variant_layouts,
                        });
                    }
                }
            }
        }
        // Expose the fully-built user-type map so `compile_closure` can resolve a struct/enum
        // closure PARAMETER's declared type (it runs deep in statement compilation, below).
        c.user_types = user_types.clone();

        // Inferred mutable-borrow params (value-semantics COW exemption): compute the
        // SAME set the AOT uses (`codegen/program.rs`) so the eager VM mutates a
        // consumed-and-returned Seq param in place instead of deep-cloning on every
        // `SetIndex` (the heap_sort/quicksort O(n²) collapse). Gated on the type
        // registry being available AND value semantics being on (else the exemption
        // is irrelevant). See [`Op::EnsureOwned`] for the call-site soundness barrier.
        if let Some(registry) = types {
            if crate::semantics::collections::value_semantics_enabled() {
                use crate::analysis::readonly::{detect_consume_alias, MutableBorrowParams};
                use crate::codegen::detection::is_vec_type_expr;
                use crate::codegen::program::body_contains_escape;
                use crate::codegen::tce::is_tail_recursive;
                use crate::optimization::Opt;
                use crate::optimize::active_config;
                use crate::tail_call::detect_accumulator_pattern;
                let cg = crate::analysis::callgraph::CallGraph::build(stmts, interner);
                let type_env = crate::analysis::TypeEnv::infer_program(stmts, interner, registry);
                let mb = MutableBorrowParams::analyze(stmts, &cg, &type_env);
                for s in stmts {
                    if let Stmt::FunctionDef {
                        name,
                        params,
                        body,
                        is_native,
                        is_exported,
                        opt_flags,
                        ..
                    } = s
                    {
                        if *is_native || *is_exported {
                            continue;
                        }
                        if !active_config().merged(opt_flags).is_on(Opt::Borrow) {
                            continue;
                        }
                        if is_tail_recursive(*name, body)
                            || detect_accumulator_pattern(*name, body).is_some()
                            || body_contains_escape(body)
                        {
                            continue;
                        }
                        let mut idxs = std::collections::HashSet::new();
                        let mut aliases = std::collections::HashSet::new();
                        for (i, (psym, pty)) in params.iter().enumerate() {
                            if mb.is_mutable_borrow(*name, *psym) && is_vec_type_expr(pty, interner)
                            {
                                idxs.insert(i);
                                if let Some(alias) = detect_consume_alias(body, *psym) {
                                    aliases.insert(alias);
                                }
                            }
                        }
                        if !idxs.is_empty() {
                            c.mut_borrow_params.insert(*name, idxs);
                            c.mut_borrow_alias_syms.insert(*name, aliases);
                        }
                    }
                }
            }
        }

        if let Some(registry) = types {
            if crate::optimize::active_config().is_on(crate::optimization::Opt::Scalarize) {
                use crate::analysis::readonly::ReadonlyParams;
                let callgraph = crate::analysis::callgraph::CallGraph::build(stmts, interner);
                let type_env = crate::analysis::TypeEnv::infer_program(stmts, interner, registry);
                let readonly = ReadonlyParams::analyze(stmts, &callgraph, &type_env);
                let mut borrow = HashMap::<Symbol, std::collections::HashSet<usize>>::new();
                for stmt in stmts {
                    let Stmt::FunctionDef { name, params, .. } = stmt else {
                        continue;
                    };
                    let indices = params
                        .iter()
                        .enumerate()
                        .filter_map(|(index, (param, _))| {
                            readonly.is_readonly(*name, *param).then_some(index)
                        })
                        .collect::<std::collections::HashSet<_>>();
                    if !indices.is_empty() {
                        borrow.insert(*name, indices);
                    }
                }
                c.fixed_seq_caps =
                    crate::codegen::affine_array::fixed_stack_seq_plan(stmts, &borrow, interner);
                if !c.fixed_seq_caps.is_empty() {
                    crate::optimize::mark_fired(crate::optimization::Opt::Scalarize);
                }
            }
        }

        // Pass 1: register every function name → index (entry_pc filled later)
        // and every struct definition.
        for s in stmts {
            if let Stmt::StructDef { name, fields, .. } = s {
                c.struct_defs.insert(*name, fields.clone());
            }
            if let Stmt::FunctionDef {
                name,
                is_native: true,
                native_path,
                ..
            } = s
            {
                c.native_fns.insert(*name, *native_path);
            }
            if let Stmt::FunctionDef {
                name,
                params,
                return_type,
                ..
            } = s
            {
                if c.fn_index.contains_key(name) {
                    return Err(format!(
                        "vm: function '{}' defined twice",
                        interner.resolve(*name)
                    )
                    .into());
                }
                let idx = c.functions.len() as FuncIdx;
                c.fn_index.insert(*name, idx);
                let inout: Vec<usize> = params
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, ty))| matches!(ty, crate::ast::stmt::TypeExpr::Mutable { .. }))
                    .map(|(i, _)| i)
                    .collect();
                if !inout.is_empty() {
                    c.inout_params.insert(*name, inout);
                }
                c.functions.push(CompiledFunction {
                    name: *name,
                    entry_pc: 0,
                    param_count: u16::try_from(params.len())
                        .map_err(|_| "vm: too many parameters".to_string())?,
                    register_count: 0,
                    captures: Vec::new(),
                    named_regs: Vec::new(),
                    param_kinds: params
                        .iter()
                        .map(|(_, ty)| {
                            param_kind_of_type(ty, interner).or_else(|| {
                                matches!(
                                    boundary_of_type_expr(ty, &user_types, interner),
                                    Some(BoundaryType::Enum(_))
                                )
                                .then_some(super::native_tier::ParamKind::Inductive)
                            })
                        })
                        .collect(),
                    ret_kind: return_type.and_then(|ty| slot_kind_of_type(ty, interner)),
                    param_types: params
                        .iter()
                        .map(|(_, ty)| boundary_of_type_expr(ty, &user_types, interner))
                        .collect(),
                    return_type: return_type
                        .and_then(|ty| boundary_of_type_expr(ty, &user_types, interner)),
                    mutable_param_regs: {
                        // `mutable` param at position `i` → frame register `i`; plus
                        // every INFERRED mutable-borrow param (same reg == index).
                        let inferred = c.mut_borrow_params.get(name);
                        let mut mp: Vec<Reg> = Vec::new();
                        for (i, (_psym, ty)) in params.iter().enumerate() {
                            if matches!(ty, crate::ast::stmt::TypeExpr::Mutable { .. })
                                || inferred.is_some_and(|s| s.contains(&i))
                            {
                                mp.push(i as Reg);
                            }
                        }
                        mp
                    },
                    cow_exempt_regs: params
                        .iter()
                        .enumerate()
                        .filter(|(i, (_, ty))| {
                            !matches!(ty, crate::ast::stmt::TypeExpr::Mutable { .. })
                                && c.mut_borrow_params.get(name).is_some_and(|p| p.contains(i))
                        })
                        .map(|(i, _)| i as Reg)
                        .collect(),
                });
            }
        }

        // The refusals the `Mount` and `## Route` arms raise, decided once from the whole
        // program — the same ones `lgx_refusals` hands an editor.
        c.mount_refusals = mount_refusals(stmts, interner, types);
        c.route_refusals =
            route_refusal_map(stmts, interner, &|sym| c.struct_defs.contains_key(&sym));

        // Pass 1.5: promote Main TOP-LEVEL Let names referenced from any
        // function or closure body to globals (lexically visible everywhere).
        let mut nonlocal_idents: std::collections::HashSet<Symbol> =
            std::collections::HashSet::new();
        for s in stmts {
            collect_nonlocal_idents_stmt(s, true, &mut nonlocal_idents);
        }
        let mut global_names: Vec<String> = Vec::new();
        // Parallel to `global_names`: each promoted global's resolved composite type (for typing a
        // closure capture of it); `None` for a scalar / self-describing / unmodeled value.
        let mut global_types: Vec<Option<BoundaryType>> = Vec::new();
        for s in stmts {
            if let Stmt::Let { var, ty, value, .. } = s {
                if nonlocal_idents.contains(var) && !c.promoted.contains_key(var) {
                    let idx = u16::try_from(c.promoted.len())
                        .map_err(|_| "vm: too many globals".to_string())?;
                    c.promoted.insert(*var, idx);
                    global_names.push(interner.resolve(*var).to_string());
                    // What the binding declares — its annotation, or the declared return type of the
                    // function its value calls — and only then what its value's construction shows.
                    let declared = c.declared_binding(*ty, value);
                    global_types.push(declared.or_else(|| {
                        boundary_of_value_expr(value, &user_types, interner, &|f| {
                            c.return_type_of(f)
                        })
                    }));
                }
            }
        }

        // Pass 2a: compile Main (every non-FunctionDef top-level statement).
        // Behind `LOGOS_NARROW_VM`: certify which top-level `new Seq of Int`
        // declarations fit `i32` (the AOT narrowing proof), so their empty
        // constructor lowers to the half-width `NewEmptyListI32`. The proof's
        // `walk`/`escapes` ignore `FunctionDef` statements (none of the write
        // forms it tracks), and the runtime widen-on-overflow is the ultimate
        // safety net, so feeding the full statement list is sound.
        if narrow_vm_enabled() {
            c.narrowable_seqs = crate::codegen::narrow::narrowable_seqs_for_body(stmts, interner);
            if !c.narrowable_seqs.is_empty() {
                crate::optimize::mark_fired(crate::optimization::Opt::NarrowVm);
            }
        }
        c.begin_scope();
        for s in stmts {
            if !matches!(s, Stmt::FunctionDef { .. }) {
                c.compile_stmt(s)?;
            }
        }
        c.narrowable_seqs.clear();
        c.emit(Op::Halt);
        let main_regs = c.max_reg as usize;
        let mut named_regs = c.named.clone();
        named_regs.resize(main_regs, false);
        // Snapshot Main's variable names NOW — the per-function `begin_scope` below
        // clears `dbg_names`. Empty unless the debugger enabled name capture.
        let reg_names: Vec<(u16, String)> = c
            .dbg_names
            .iter()
            .map(|(r, sym)| (*r, c.interner.resolve(*sym).to_string()))
            .collect();

        // DEBUG-ONLY: each function's name and, once its body compiles, its variable names.
        let mut fn_debug: Vec<FnDebugInfo> = if c.debug_names {
            c.functions
                .iter()
                .map(|f| FnDebugInfo {
                    name: c.interner.resolve(f.name).to_string(),
                    reg_names: Vec::new(),
                })
                .collect()
        } else {
            Vec::new()
        };

        // Pass 2b: compile each function body, recording its entry point.
        for s in stmts {
            if let Stmt::FunctionDef {
                name,
                params,
                body,
                return_type,
                ..
            } = s
            {
                let idx = c.fn_index[name];
                let entry_pc = c.code.len();
                c.site_at
                    .push((entry_pc as u32, crate::vm::instruction::NO_SITE));
                c.begin_scope();
                c.in_function = true;
                c.current_return_type = *return_type;
                c.named.clear();
                for (i, (psym, _ty)) in params.iter().enumerate() {
                    c.scopes.last_mut().unwrap().insert(*psym, i as Reg);
                    c.mark_named(i as Reg);
                    c.record_dbg_name(*psym, i as Reg);
                }
                c.next_reg = params.len() as Reg;
                // Parameters occupy registers 0..n even when the body
                // allocates no temps — register_count must cover them or
                // the native frame's limit slot aliases a parameter.
                c.max_reg = c.max_reg.max(c.next_reg);
                debug_assert!(params.len() <= MAX_REGISTERS_PER_FRAME);
                c.tail_fn = Some((*name, entry_pc, params.len() as u16));
                // COW-exempt the consume-alias locals of this function's mutable-borrow
                // params: `let_reg` appends each one's register as the body compiles.
                c.current_exempt_syms = c
                    .mut_borrow_alias_syms
                    .get(name)
                    .cloned()
                    .unwrap_or_default();
                c.current_exempt_regs.clear();
                let body_slice: &[Stmt] = body;
                let mut bk = 0;
                while bk < body_slice.len() {
                    // TAIL-CALL PAIR: `Set/Let x = self(args)` immediately
                    // followed by `Return x` is a self-tail-call (x is returned,
                    // never read after) — lower it to the loop-back instead of
                    // call+return. A direct `Return self(args)` is handled in the
                    // Return arm of compile_stmt (a Return is inherently tail).
                    if bk + 1 < body_slice.len() {
                        if let Some(args) = crate::tail_call::tail_pair_args(
                            &body_slice[bk],
                            &body_slice[bk + 1],
                            *name,
                            params.len(),
                        ) {
                            let mark = c.next_reg;
                            c.emit_tail_call(args, entry_pc)?;
                            c.reclaim_scratch(mark);
                            bk += 2;
                            continue;
                        }
                    }
                    c.compile_stmt(&body_slice[bk])?;
                    bk += 1;
                }
                c.tail_fn = None;
                // Fall off the end → return nothing.
                c.emit(Op::ReturnNothing);
                // Merge the consume-alias exempt registers discovered while compiling
                // the body into this function's COW-exempt set.
                let exempt = std::mem::take(&mut c.current_exempt_regs);
                c.current_exempt_syms.clear();
                let f = &mut c.functions[idx as usize];
                for r in exempt {
                    if !f.mutable_param_regs.contains(&r) {
                        f.mutable_param_regs.push(r);
                    }
                    if !f.cow_exempt_regs.contains(&r) {
                        f.cow_exempt_regs.push(r);
                    }
                }
                f.entry_pc = entry_pc;
                f.register_count = c.max_reg as usize;
                let mut fnamed = c.named.clone();
                fnamed.resize(f.register_count, false);
                f.named_regs = fnamed;
                if let Some(info) = fn_debug.get_mut(idx as usize) {
                    info.reg_names = c
                        .dbg_names
                        .iter()
                        .map(|(r, sym)| (*r, c.interner.resolve(*sym).to_string()))
                        .collect();
                }
            }
        }
        // Closures compiled along the way have no source name of their own.
        if c.debug_names {
            fn_debug.resize_with(c.functions.len(), FnDebugInfo::default);
        }

        Ok(CompiledProgram {
            constants: c.constants,
            code: c.code,
            register_count: main_regs,
            functions: c.functions,
            fn_index: c.fn_index,
            globals: global_names,
            named_regs,
            loop_locals: c.loop_locals,
            reg_names,
            fn_debug,
            struct_types,
            enum_types,
            global_types,
            declared_types: {
                let mut declared = std::mem::take(&mut c.declared_types);
                declared.sort_by_key(|(pc, _)| *pc);
                declared
            },
            observation_sites: c.observation_sites,
            site_at: c.site_at,
        })
    }

    fn begin_scope(&mut self) {
        self.scopes = vec![HashMap::new()];
        self.next_reg = 0;
        self.max_reg = 0;
        self.in_function = false;
        self.current_return_type = None;
        self.dbg_names.clear();
    }

    /// Enter a child block scope (If/While/Repeat body).
    fn enter_block(&mut self) -> Reg {
        self.scopes.push(HashMap::new());
        self.next_reg
    }

    /// Leave a block scope, recycling every register it allocated (its values
    /// are dead — the tree-walker's pop_scope undoes the bindings too).
    fn exit_block(&mut self, mark: Reg) {
        self.scopes.pop();
        self.next_reg = mark;
    }

    /// Resolve a name through the enclosing block scopes (innermost first).
    fn lookup_local(&self, sym: Symbol) -> Option<Reg> {
        self.scopes.iter().rev().find_map(|s| s.get(&sym).copied())
    }

    /// Is `sym`'s region-entry bounds guard active in some enclosing loop?
    fn is_hoist_enabled(&self, sym: Symbol) -> bool {
        self.hoist_enabled.iter().any(|s| s.contains(&sym))
    }

    /// Should `item index of collection` be emitted UNCHECKED? Either the
    /// Oracle proved it in range on every run, or it is speculatively in bounds
    /// AND its array's region-entry guard was emitted for an enclosing loop (so
    /// the VM verifies it before any unchecked native access runs). A proof that
    /// holds only under a `% m` precondition keeps the check.
    fn index_in_bounds(&self, collection: &Expr, index: &Expr) -> bool {
        // The shared optimizer can attach an unconditional proof directly to
        // this exact index node (for example, the TileLegal matrix clone).
        // Consume that proof before asking the Oracle for an independently
        // reconstructed interval.  Requiring both proofs made `InBounds`
        // unreachable whenever optimization changed the loop shape enough
        // that the Oracle no longer rediscovered the same relation.
        if crate::optimize::active_config().is_on(crate::optimization::Opt::Unchecked)
            && crate::optimize::facts::holds_expr(crate::optimize::facts::Fact::InBounds, index)
        {
            return true;
        }
        let Some(o) = self.oracle.as_ref() else {
            return false;
        };
        if o.index_provably_in_bounds(collection, index) {
            return o.index_always_in_bounds(collection, index);
        }
        o.index_is_speculative(index)
            && matches!(collection, Expr::Identifier(s) if self.is_hoist_enabled(*s))
    }

    /// `x / 2^k` where the divisor is a literal power of two and the Oracle
    /// proves the dividend is `Int` → the shift amount `k` (`1..=62`), else
    /// `None`. Type-safety is the crux: `Op::Div` dispatches int/float at
    /// runtime on the dividend, so lowering to a shift is sound ONLY for an
    /// integer dividend. The literal divisor is visible here pre-hoist, so this
    /// fires for loop-invariant divisors the JIT's in-region detector misses.
    fn divpow2_shift(&self, left: &Expr, right: &Expr) -> Option<u32> {
        let Expr::Literal(Literal::Number(d)) = right else {
            return None;
        };
        let d = *d;
        if d < 2 || (d & (d - 1)) != 0 {
            return None;
        }
        let k = d.trailing_zeros();
        if !(1..=62).contains(&k) {
            return None;
        }
        if self.oracle.as_ref()?.expr_scalar(left)? != crate::optimize::ScalarKind::Int {
            return None;
        }
        Some(k)
    }

    /// `x % 2^k` where the divisor is a literal power of two AND the Oracle
    /// proves the dividend is `Int` and NON-NEGATIVE → the low-bit mask
    /// `2^k - 1`, else `None`. On a two's-complement i64 the truncated
    /// remainder agrees with `x & (2^k - 1)` only when `x >= 0` (the sign of
    /// `x % m` follows the dividend, so `(-1) % 8 == -1 != 7 == (-1) & 7`); the
    /// non-negativity gate is the SOUNDNESS condition — exactly the gate the AOT
    /// e-graph's `mod-pow2-and` rule applies. The literal divisor is visible
    /// here pre-hoist, so this fires for loop-invariant moduli the JIT's
    /// in-region detector misses (histogram's `% 2^31` LCG feedback). The win:
    /// a register-form `BitAnd` (1-cycle AND, which the JIT lowers to a
    /// `BitAnd` stencil) instead of `Op::Mod`'s idiv on BOTH the VM and JIT.
    fn modpow2_mask(&self, left: &Expr, right: &Expr) -> Option<i64> {
        let Expr::Literal(Literal::Number(d)) = right else {
            return None;
        };
        let d = *d;
        if d < 2 || (d & (d - 1)) != 0 {
            return None;
        }
        let k = d.trailing_zeros();
        if !(1..=62).contains(&k) {
            return None;
        }
        let oracle = self.oracle.as_ref()?;
        if oracle.expr_scalar(left)? != crate::optimize::ScalarKind::Int {
            return None;
        }
        if !oracle.expr_proven_nonneg(left) {
            return None;
        }
        Some(d - 1)
    }

    /// `x / c` or `x % c` where the divisor is a literal constant `c > 0` that is
    /// NOT a power of two (W5's `divpow2_shift`/`modpow2_mask` own powers of two)
    /// AND the Oracle proves `x` is `Int` and NON-NEGATIVE → the precomputed
    /// `(magic, more)` for the unsigned magic-reciprocal sequence, else `None`.
    ///
    /// SOUNDNESS — the unsigned magic equals the signed truncating `/`/`%` only
    /// for a non-negative dividend: `x.wrapping_div(c)` / `x.wrapping_rem(c)`
    /// equal `(x as u64) / c` / `(x as u64) % c` exactly when `x >= 0` and
    /// `c > 0` (both operands non-negative, no overflow edge — `c` is a literal
    /// `> 0` so `MIN/-1` is impossible). For `x < 0` the signs disagree, so the
    /// non-negativity gate is the soundness condition — exactly the gate
    /// `modpow2_mask` and the AOT `fast_div` lever apply. The literal divisor is
    /// visible here pre-hoist, so this fires for loop-invariant moduli the JIT's
    /// in-region detector misses (matrix's `% 1000000007`, histogram's `% 1000`).
    fn magic_div_const(&self, left: &Expr, right: &Expr) -> Option<(u64, u8, i64)> {
        if !magic_div_enabled() {
            return None;
        }
        let Expr::Literal(Literal::Number(d)) = right else {
            return None;
        };
        let d = *d;
        // `c > 0` and NOT a power of two (pow2 is W5's AND/shift territory).
        if d < 2 || (d & (d - 1)) == 0 {
            return None;
        }
        let oracle = self.oracle.as_ref()?;
        if oracle.expr_scalar(left)? != crate::optimize::ScalarKind::Int {
            return None;
        }
        // The magic reciprocal is emitted OPTIMISTICALLY on a proven-non-negative dividend, whose
        // interval may be unbounded above (the AI cannot narrow a loop induction var like `i < 5000`
        // that stays i64 at runtime — the perf lock `magic_nonneg_div_rewrite_fires` depends on this).
        // Soundness for a value that IS a `BigInt` at runtime is restored by `Op::MagicDivU`'s carried
        // `divisor`: the VM handler falls back to exact `Div`/`Mod` when the operand does not fit i64.
        if !oracle.expr_proven_nonneg(left) {
            return None;
        }
        let (magic, more) = magic_u64_gen(d as u64);
        crate::optimize::mark_fired(crate::optimization::Opt::FastDiv);
        Some((magic, more, d))
    }

    /// `Opt::ModWrap` for `x % c`: how the remainder reduces when `c` is a literal and the Oracle proves `x` an
    /// `Int` within a finite range ([`crate::optimize::modwrap::mod_wrap`]), or `None` when it must divide. The
    /// AST emitters decide the same reduction from the same Oracle.
    fn mod_wrap_const(&self, left: &Expr, right: &Expr) -> Option<crate::optimize::modwrap::ModWrap> {
        if !crate::optimize::active_config().is_on(crate::optimization::Opt::ModWrap) {
            return None;
        }
        let Expr::Literal(Literal::Number(c)) = right else {
            return None;
        };
        let oracle = self.oracle.as_ref()?;
        if oracle.expr_scalar(left)? != crate::optimize::ScalarKind::Int {
            return None;
        }
        let (lo, hi) = oracle.expr_int_range(left)?;
        crate::optimize::modwrap::mod_wrap(*c, Some((i128::from(lo), i128::from(hi))))
    }

    /// Emit `RegionBoundsGuard` for each hoist descriptor the Oracle recorded
    /// for the loop `stmt`, returning the set of arrays whose guard was
    /// emitted (only those are then eligible for speculative elision). All
    /// three symbols must resolve to local registers, and the loop must read
    /// the array's length nowhere stale — guaranteed by the Oracle's analysis.
    fn emit_hoist_guards(&mut self, stmt: &Stmt) -> std::collections::HashSet<Symbol> {
        let mut enabled = std::collections::HashSet::new();
        let Some(oracle) = self.oracle.as_ref() else {
            return enabled;
        };
        let descs = oracle
            .hoist_descs_for(stmt as *const Stmt as usize)
            .to_vec();
        for d in descs {
            let (Some(array), Some(bound), Some(iv)) = (
                self.lookup_local(d.array),
                self.lookup_local(d.bound),
                self.lookup_local(d.iv),
            ) else {
                continue;
            };
            self.emit(Op::RegionBoundsGuard {
                array,
                bound,
                iv,
                add_max: d.add_max,
                add_min: d.add_min,
            });
            enabled.insert(d.array);
        }
        enabled
    }

    /// Emit the runtime "Undefined variable" failure — unbound names are a
    /// RUNTIME error in the tree-walker, so dead branches stay free.
    fn emit_unbound(&mut self, sym: Symbol) -> Result<(), Refusal> {
        let msg = format!("Undefined variable: {}", self.interner.resolve(sym));
        let idx = self.add_const(Constant::Text(msg))?;
        self.emit(Op::FailWith { msg: idx });
        Ok(())
    }

    /// Emit an unconditional runtime failure with a fixed message — the
    /// pattern for statements the interpreter spec rejects WHEN EXECUTED.
    fn emit_fail(&mut self, msg: &str) -> Result<(), Refusal> {
        let idx = self.add_const(Constant::Text(msg.to_string()))?;
        self.emit(Op::FailWith { msg: idx });
        Ok(())
    }

    /// Resolve a name: block locals (and closure captures) shadow promoted
    /// globals; a promoted capture in a closure body needs the
    /// snapshot-or-live-global split.
    fn resolve_name(&self, sym: Symbol) -> NameRef {
        if let Some(r) = self.lookup_local(sym) {
            if let Some(ctx) = &self.closure_ctx {
                if let Some(&(value, flag)) = ctx.get(&sym) {
                    // Only when the local resolution IS the capture slot (a
                    // body-local `Let` shadows the capture entirely).
                    if r == value {
                        if let Some(&global) = self.promoted.get(&sym) {
                            return NameRef::CaptureOrGlobal {
                                value,
                                flag,
                                global,
                            };
                        }
                    }
                }
            }
            NameRef::Local(r)
        } else if let Some(&g) = self.promoted.get(&sym) {
            NameRef::Global(g)
        } else {
            NameRef::Unbound
        }
    }

    /// Emit a read of `sym` into `dst`.
    fn emit_read(&mut self, sym: Symbol, dst: Reg) -> Result<(), Refusal> {
        match self.resolve_name(sym) {
            NameRef::Local(src) => {
                if src != dst {
                    self.emit(Op::Move { dst, src });
                }
                Ok(())
            }
            NameRef::CaptureOrGlobal {
                value,
                flag,
                global,
            } => {
                let jg = self.emit_placeholder_jump_if_false(flag);
                self.emit(Op::Move { dst, src: value });
                let jend = self.emit_placeholder_jump();
                self.patch_jump_target(jg, self.current_pc())?;
                self.emit(Op::GlobalGet { dst, idx: global });
                self.patch_jump_target(jend, self.current_pc())?;
                Ok(())
            }
            NameRef::Global(idx) => {
                self.emit(Op::GlobalGet { dst, idx });
                Ok(())
            }
            NameRef::Unbound => self.emit_unbound(sym),
        }
    }

    /// Emit a write of `R[src]` to `sym` (Set semantics: the binding must
    /// already exist somewhere).
    fn emit_write(&mut self, sym: Symbol, src: Reg) -> Result<(), Refusal> {
        match self.resolve_name(sym) {
            NameRef::Local(dst) => {
                if src != dst {
                    self.emit(Op::Move { dst, src });
                }
                Ok(())
            }
            NameRef::CaptureOrGlobal {
                value,
                flag,
                global,
            } => {
                let jg = self.emit_placeholder_jump_if_false(flag);
                self.emit(Op::Move { dst: value, src });
                let jend = self.emit_placeholder_jump();
                self.patch_jump_target(jg, self.current_pc())?;
                self.emit(Op::GlobalSet { idx: global, src });
                self.patch_jump_target(jend, self.current_pc())?;
                Ok(())
            }
            NameRef::Global(idx) => {
                self.emit(Op::GlobalSet { idx, src });
                Ok(())
            }
            NameRef::Unbound => self.emit_unbound(sym),
        }
    }

    /// `Click "#go".` / `Type … into "#draft".` on this tier: find the element, hand each event the
    /// verb raises (`semantics::drive::Verb::events`) to the page's node dispatcher
    /// (`__lgx_dispatch_node`) with what the verb carries, and fail in `semantics::drive`'s words —
    /// the ones every tier reports — when nothing matches the selector or nothing answered. The
    /// one drive semantics every tier shares: a browser, the page host and this tier all deliver
    /// an element's event through the same entry. A program with no dispatcher attaches no handler
    /// at all, and saying so is more use than a call to a function that does not exist.
    fn compile_lgx_drive(
        &mut self,
        verb: crate::semantics::drive::Verb,
        selector: &Expr,
        carried: Option<&Expr>,
        key: Option<&Expr>,
    ) -> Result<(), Refusal> {
        use crate::semantics::drive;
        let first_event = verb.events()[0];
        let dispatcher = logicaffeine_language::parser::lgx_lower::dispatch_node_fn_name();
        let func = self
            .interner
            .lookup(dispatcher)
            .and_then(|s| self.fn_index.get(&s).copied());
        let Some(func) = func else {
            return Err(format!(
                "this page answers no {first_event}: nothing in it attaches a handler, so there is \
                 nothing for the {first_event} to reach on this tier."
            )
            .into());
        };
        let kept_selector = self.keep_operand(selector)?;
        let kept_key = key.map(|k| self.keep_operand(k)).transpose()?;
        // Typing and choosing are the browser's own default action first — the control takes the
        // value whether or not the page listens — so they are answered by the control itself,
        // refused only when it cannot take them (`drive::cannot_take`), and their events need no
        // listener.
        let enters = match verb {
            drive::Verb::Type => Some("type"),
            drive::Verb::Choose => Some("choose"),
            _ => None,
        };
        let kept_value = match (enters, carried) {
            (Some(_), Some(v)) => Some(self.keep_operand(v)?),
            _ => None,
        };
        let found = self.alloc_reg()?;
        let value = self.alloc_reg()?;
        let answered = self.alloc_reg()?;
        match carried {
            Some(v) => {
                self.compile_expr_into(v, value)?;
                if let Some(k) = kept_key.as_ref().or(kept_value.as_ref()) {
                    self.keep_move(k, value);
                }
            }
            None => {
                let idx = self.add_const(Constant::Text(String::new()))?;
                self.emit(Op::LoadConst { dst: value, idx });
            }
        }
        let query = self.reserve_regs(1)?;
        self.compile_expr_into(selector, query)?;
        self.keep_move(&kept_selector, query);
        self.emit(Op::CallBuiltin {
            dst: found,
            builtin: crate::semantics::builtins::BuiltinId::DomQuerySelector,
            args_start: query,
            arg_count: 1,
        });

        let no_element = DriveText::render(|sel, _| drive::no_element(sel), &kept_selector, None);
        let nothing_answers = DriveText::render(
            |sel, k| drive::nothing_answers(verb.events(), k, sel),
            &kept_selector,
            kept_key.as_ref(),
        );

        let zero = self.alloc_reg()?;
        let zidx = self.add_const(Constant::Int(0))?;
        self.emit(Op::LoadConst {
            dst: zero,
            idx: zidx,
        });
        let absent = self.alloc_reg()?;
        self.emit(Op::Lt {
            dst: absent,
            lhs: found,
            rhs: zero,
        });
        let missing_branch = self.code.len();
        self.emit(Op::JumpIfTrue {
            cond: absent,
            target: usize::MAX,
        });
        if let Some(verb_name) = enters {
            let args = self.reserve_regs(3)?;
            let vidx = self.add_const(Constant::Text(verb_name.to_string()))?;
            self.emit(Op::LoadConst {
                dst: args,
                idx: vidx,
            });
            match &kept_selector {
                DriveOperand::Known(s) => {
                    let sidx = self.add_const(Constant::Text(s.clone()))?;
                    self.emit(Op::LoadConst {
                        dst: args + 1,
                        idx: sidx,
                    });
                }
                DriveOperand::Held(r) => self.emit(Op::Move {
                    dst: args + 1,
                    src: *r,
                }),
            }
            self.emit(Op::Move {
                dst: args + 2,
                src: value,
            });
            let took = self.alloc_reg()?;
            self.emit(Op::CallBuiltin {
                dst: took,
                builtin: crate::semantics::builtins::BuiltinId::DomEnterValue,
                args_start: args,
                arg_count: 3,
            });
            let takes = self.code.len();
            self.emit(Op::JumpIfTrue {
                cond: took,
                target: usize::MAX,
            });
            let cannot_take = DriveText::render(
                |sel, v| drive::cannot_take(verb, sel, v.unwrap_or("")),
                &kept_selector,
                kept_value.as_ref(),
            );
            self.emit_fail_text(&cannot_take)?;
            self.patch_jump_target(takes, self.current_pc())?;
        } else {
            let no = self.add_const(Constant::Bool(false))?;
            self.emit(Op::LoadConst {
                dst: answered,
                idx: no,
            });
        }
        let yes = if enters.is_none() {
            Some(self.add_const(Constant::Bool(true))?)
        } else {
            None
        };
        // The page hears each event as the HOST's entry into it, as it hears a visitor's: a turn
        // that fails there is contained to it and heard by the page's failure door, and the
        // program runs on (`BuiltinId::DomDispatchEntry`). The door is the page's own two
        // functions, when it has them.
        let page_function = |c: &Self, name: &str| -> i64 {
            c.interner
                .lookup(name)
                .and_then(|s| c.fn_index.get(&s))
                .map_or(-1, |&f| i64::from(f))
        };
        let door = [
            i64::from(func),
            page_function(
                self,
                logicaffeine_language::parser::lgx_lower::failure_context_fn_name(),
            ),
            page_function(
                self,
                logicaffeine_language::parser::lgx_lower::on_failure_fn_name(),
            ),
        ];
        for event in verb.events() {
            let args_start = self.reserve_regs(6)?;
            for (k, f) in door.into_iter().enumerate() {
                let idx = self.add_const(Constant::Int(f))?;
                self.emit(Op::LoadConst {
                    dst: args_start + k as Reg,
                    idx,
                });
            }
            self.emit(Op::Move {
                dst: args_start + 3,
                src: found,
            });
            let name = self.add_const(Constant::Text(event.to_string()))?;
            self.emit(Op::LoadConst {
                dst: args_start + 4,
                idx: name,
            });
            self.emit(Op::Move {
                dst: args_start + 5,
                src: value,
            });
            let heard = self.alloc_reg()?;
            self.emit(Op::CallBuiltin {
                dst: heard,
                builtin: crate::semantics::builtins::BuiltinId::DomDispatchEntry,
                args_start,
                arg_count: 6,
            });
            if let Some(yes) = yes {
                let unheard = self.code.len();
                self.emit(Op::JumpIfFalse {
                    cond: heard,
                    target: usize::MAX,
                });
                self.emit(Op::LoadConst {
                    dst: answered,
                    idx: yes,
                });
                self.patch_jump_target(unheard, self.current_pc())?;
            }
        }
        let ok = self.code.len();
        if enters.is_some() {
            self.emit(Op::Jump { target: usize::MAX });
        } else {
            self.emit(Op::JumpIfTrue {
                cond: answered,
                target: usize::MAX,
            });
            self.emit_fail_text(&nothing_answers)?;
        }
        self.patch_jump_target(missing_branch, self.current_pc())?;
        self.emit_fail_text(&no_element)?;
        self.patch_jump_target(ok, self.current_pc())?;
        Ok(())
    }

    /// Call a PAGE-level driver — `__lgx_raise` (a window event), `__lgx_tick` (the page clock).
    /// The page always exists and an event it ignores is not a mistake, so these drivers always
    /// answer 2 and there is no failure to raise.
    fn compile_page_drive(&mut self, func: FuncIdx, args: &[&Expr]) -> Result<(), Refusal> {
        let arg_count =
            u16::try_from(args.len()).map_err(|_| "vm: too many arguments".to_string())?;
        let args_start = self.reserve_regs(arg_count)?;
        for (i, arg) in args.iter().enumerate() {
            self.compile_expr_into(arg, args_start + i as Reg)?;
        }
        let dst = self.alloc_reg()?;
        self.emit(Op::Call {
            dst,
            func,
            args_start,
            arg_count,
        });
        Ok(())
    }

    /// Stop the program with `text`: one constant when every piece was known at compile time,
    /// otherwise the pieces concatenated at run time and handed to the failing builtin.
    fn emit_fail_text(&mut self, text: &DriveText) -> Result<(), Refusal> {
        if let [DrivePiece::Text(whole)] = text.pieces.as_slice() {
            let idx = self.add_const(Constant::Text(whole.clone()))?;
            self.emit(Op::FailWith { msg: idx });
            return Ok(());
        }
        let arg = self.reserve_regs(1)?;
        let empty = self.add_const(Constant::Text(String::new()))?;
        self.emit(Op::LoadConst {
            dst: arg,
            idx: empty,
        });
        for piece in &text.pieces {
            let part = match piece {
                DrivePiece::Text(t) => {
                    let idx = self.add_const(Constant::Text(t.clone()))?;
                    let r = self.alloc_reg()?;
                    self.emit(Op::LoadConst { dst: r, idx });
                    r
                }
                DrivePiece::Plain(r) => *r,
            };
            self.emit(Op::Concat {
                dst: arg,
                lhs: arg,
                rhs: part,
            });
        }
        let dst = self.alloc_reg()?;
        self.emit(Op::CallBuiltin {
            dst,
            builtin: crate::semantics::builtins::BuiltinId::Fail,
            args_start: arg,
            arg_count: 1,
        });
        Ok(())
    }

    /// Stop the program with a reason computed at run time: evaluate `reason` into an argument
    /// register and call the failing builtin, whose `Err` is the program's error text.
    fn emit_fail_builtin(
        &mut self,
        builtin: crate::semantics::builtins::BuiltinId,
        reason: &Expr,
    ) -> Result<(), Refusal> {
        let arg = self.reserve_regs(1)?;
        self.compile_expr_into(reason, arg)?;
        let dst = self.alloc_reg()?;
        self.emit(Op::CallBuiltin {
            dst,
            builtin,
            args_start: arg,
            arg_count: 1,
        });
        Ok(())
    }

    /// Compile one statement, then reclaim the scratch registers it allocated.
    ///
    /// ★ SCRATCH IS DEAD AT THE STATEMENT BOUNDARY — this is where that promise is kept, in every frame (Main,
    /// function, closure) and every block (`If`/`While`/`Repeat`/`Inspect` arm, zone, splice). Before it, the
    /// only recycler was `exit_block`, which fires when a whole block closes, so a straight-line run of
    /// statements never gave a register back: two or three per statement, and the 16,384-register frame ran
    /// out near 5,000–8,000 statements — below a single real-width Qwen3.8 tensor fill, and at the same length
    /// inside a function or a loop body.
    ///
    /// What survives a statement is exactly what a NAME holds. So the new floor is the higher of where the
    /// statement began and one past the highest register any live binding occupies; everything above it was
    /// the statement's own scratch. A binding created by this statement is a live name, so it is kept, and
    /// anything from before it — earlier bindings, a loop's iterator, a closure's capture slots — sits below
    /// the mark. The floor is never lowered below the mark, so reclaiming can only ever reuse registers this
    /// statement itself allocated.
    fn compile_stmt(&mut self, s: &Stmt) -> Result<(), Refusal> {
        let mark = self.next_reg;
        self.lower_stmt(s)?;
        self.reclaim_scratch(mark);
        Ok(())
    }

    fn reclaim_scratch(&mut self, mark: Reg) {
        let named_top = self
            .scopes
            .iter()
            .flat_map(|scope| scope.values())
            .map(|r| *r as usize + 1)
            .max()
            .unwrap_or(0);
        let floor = (mark as usize).max(named_top);
        if (self.next_reg as usize) > floor {
            self.next_reg = floor as Reg;
        }
    }

    fn alloc_reg(&mut self) -> Result<Reg, Refusal> {
        self.reserve_regs(1)
    }

    /// A drive failure text's operand, known at compile time when `e` is a literal; otherwise a
    /// register reserved to hold it, ahead of whatever the value's own register will be. A
    /// register at or above a call's `args_start` does not survive the call: `Op::Call`'s callee
    /// frame overlays it (`Vm::enter_call`'s `callee_base = self.base + args_start`, free to leave
    /// its own locals there on return), and `Op::CallBuiltin` clears its argument registers when
    /// it returns. Reserving the keeping register FIRST, before the value's own register, puts it
    /// below either, so [`keep_move`](Self::keep_move) can copy the value there once it is known
    /// and read it back after the call untouched.
    fn keep_operand(&mut self, e: &Expr) -> Result<DriveOperand, Refusal> {
        Ok(match e {
            Expr::Literal(Literal::Text(sym)) => {
                DriveOperand::Known(self.interner.resolve(*sym).to_string())
            }
            _ => DriveOperand::Held(self.alloc_reg()?),
        })
    }

    /// Copy `src` (the operand's own, about-to-be-unsafe register) into `operand`'s kept register,
    /// once its value is known; a no-op for a compile-time-known operand.
    fn keep_move(&mut self, operand: &DriveOperand, src: Reg) {
        if let DriveOperand::Held(dst) = *operand {
            self.emit(Op::Move { dst, src });
        }
    }

    /// Emit a self-tail-call as a LOOP-BACK (TCO): evaluate every argument into a
    /// FRESH temp first — so a parameter read by a later argument is never
    /// clobbered by an earlier parameter store — then move the temps into the
    /// parameter slots `0..param_count` and jump to the function entry. This
    /// turns self-tail-recursion into constant-stack iteration (no per-call frame
    /// round-trip, no `MAX_CALL_DEPTH` ceiling for tail calls).
    fn emit_tail_call(&mut self, args: &[&Expr], entry_pc: usize) -> Result<(), Refusal> {
        let mut temps = Vec::with_capacity(args.len());
        for a in args {
            let t = self.alloc_reg()?;
            self.compile_expr_into(a, t)?;
            temps.push(t);
        }
        for (i, t) in temps.into_iter().enumerate() {
            if t != i as Reg {
                self.emit(Op::Move {
                    dst: i as Reg,
                    src: t,
                });
            }
        }
        self.emit(Op::Jump { target: entry_pc });
        Ok(())
    }

    /// Reserve `n` consecutive registers, returning the first.
    fn reserve_regs(&mut self, n: u16) -> Result<Reg, Refusal> {
        let start = self.next_reg;
        let next = (self.next_reg as usize) + n as usize;
        if next > MAX_REGISTERS_PER_FRAME {
            return Err(format!(
                "vm: out of registers (frame limit is {})",
                MAX_REGISTERS_PER_FRAME
            )
            .into());
        }
        self.next_reg = next as Reg;
        if self.next_reg > self.max_reg {
            self.max_reg = self.next_reg;
        }
        // Scratch reserved inside a loop is a per-iteration temp, dead at the
        // loop exit — record it as a loop-local so the region JIT treats it as
        // true scratch even when it recycled a (now-dead) named variable's slot
        // (the fannkuch `perm[hi]` transfer temp that blocked swap fusion).
        if !self.loop_stack.is_empty() {
            for r in start..(next as Reg) {
                self.record_loop_local(r);
            }
        }
        Ok(start)
    }

    /// Resolve a mutation-target collection (`Push … to xs`) to its name
    /// reference. The caller materializes it (Rc-backed collections mutate
    /// through any clone) or emits the unbound failure AFTER the value's side
    /// effects, matching the tree-walker.
    fn resolve_collection(&mut self, e: &Expr) -> Result<(Symbol, NameRef), Refusal> {
        match e {
            Expr::Identifier(sym) => Ok((*sym, self.resolve_name(*sym))),
            _ => Err("vm: expected an identifier collection".to_string().into()),
        }
    }

    /// Materialize a resolved collection into a register (None = unbound).
    fn collection_reg(&mut self, sym: Symbol, nr: &NameRef) -> Result<Option<Reg>, Refusal> {
        match nr {
            NameRef::Local(r) => Ok(Some(*r)),
            NameRef::Unbound => Ok(None),
            _ => {
                let scratch = self.alloc_reg()?;
                self.emit_read(sym, scratch)?;
                Ok(Some(scratch))
            }
        }
    }

    /// The ONE lowering every mutating statement's target goes through: `Push`/`Add`/`Remove`/
    /// `SetIndex`/`SetAdd`/`RemoveFrom`'s collection may be a local, a field, an item, a field of a
    /// field, an item of a field, a field of an item — arbitrarily nested. `mutate` receives the
    /// register holding the LEAF collection and emits whatever op the statement itself performs;
    /// the mutated value is threaded back up through every level to the root binding, so the write
    /// always lands where the author's sentence says it does. Returns `Ok(false)` only when the
    /// place bottoms out at an unbound name (mirrors `collection_reg`); every other place shape is
    /// always "bound" (a non-identifier root, e.g. a call's result, is mutated as a bare value with
    /// no place to write back into — unchanged from every statement's own prior fallback).
    ///
    /// A `FieldAccess`/`Index` step reads its target the ordinary way (`GetField`/`Index`, an `Rc`
    /// clone for a List/Map/Set) and then IMMEDIATELY clears its PARENT's own reference to it
    /// (`SetIndex` to `Nothing`) before recursing further — so by the time the leaf is reached, the
    /// chain's own projection reads have left no trace, and the runtime's ordinary `ensure_reg_owned`
    /// gate (already wired into `ListPush`/`SetAdd`/`RemoveFrom`/`SetIndex`) sees a `strong_count`
    /// that reflects ONLY a real alias — never the artifact this lowering's own reads would
    /// otherwise add. A real alias still forces its own deep clone, exactly as it should; the
    /// steady-state, unaliased case never clones at all. Restoring is the same walk in reverse: the
    /// mutated (or cloned) leaf is written back into its parent, that parent's own possibly-changed
    /// value into ITS parent, and so on to the root — a `Local` root's register IS the binding, so
    /// nothing further is needed there; a `Global`/captured root gets one final `emit_write`.
    fn compile_place_mutation(
        &mut self,
        place: &Expr,
        mutate: &mut dyn FnMut(&mut Self, Reg) -> Result<(), Refusal>,
    ) -> Result<bool, Refusal> {
        match place {
            Expr::Identifier(sym) => {
                let nr = self.resolve_name(*sym);
                match self.collection_reg(*sym, &nr)? {
                    Some(reg) => {
                        // The same "release the binding's own reference before checking
                        // ownership" trick, generalized from `Stmt::SetIndex`'s original
                        // internal-global case: restricted to a COMPILER-WRITTEN name (an
                        // author's global is left on the ordinary path, where an occasional
                        // extra clone is the price of the semantics they can observe — see
                        // that case's own long-form rationale, unchanged).
                        let internal = !matches!(nr, NameRef::Local(_))
                            && logicaffeine_language::parser::lgx_lower::is_compiler_written(
                                self.interner.resolve(*sym),
                            );
                        if internal {
                            let nothing = self.add_const(Constant::Nothing)?;
                            let clear = self.alloc_reg()?;
                            self.emit(Op::LoadConst {
                                dst: clear,
                                idx: nothing,
                            });
                            self.emit_write(*sym, clear)?;
                        }
                        mutate(self, reg)?;
                        if !matches!(nr, NameRef::Local(_)) {
                            self.emit_write(*sym, reg)?;
                        }
                        Ok(true)
                    }
                    // Matches every caller's own prior `None` arm: the name is unbound, so the
                    // statement unconditionally fails at RUNTIME — `mutate`'s own value/index
                    // operands are already compiled by the time this is reached (tree-walker
                    // order: evaluate, then look up), so nothing is lost by not calling it.
                    None => {
                        self.emit_unbound(*sym)?;
                        Ok(false)
                    }
                }
            }
            _ if Self::place_key_count(place).is_some() => {
                let keys = u16::try_from(Self::place_key_count(place).unwrap())
                    .map_err(|_| "vm: too many mutation target keys".to_string())?;
                let mut next_key = self.reserve_regs(keys)?;
                let captured = self
                    .compile_lent_place(place, &mut next_key)?
                    .expect("a rooted mutation target");
                self.mutate_lent_place(&captured, mutate)
            }
            _ => {
                // Not a place at all — a call's result, a literal, any other value-producing
                // expression used directly as a mutation target. No binding to write back into;
                // mutate the value exactly as it stands, unchanged from every caller's own prior
                // catch-all.
                let reg = self.compile_expr(place)?;
                mutate(self, reg)?;
                Ok(true)
            }
        }
    }

    /// Bind `sym` for a `Let`: reuse its register when the innermost scope
    /// already has it (re-Let overwrites), otherwise allocate a fresh one in
    /// the innermost scope (shadowing any outer binding).
    /// Record that the op just emitted defines a value the program declares to be `bt`
    /// ([`CompiledProgram::declared_types`]).
    fn note_declared(&mut self, bt: BoundaryType) {
        let pc = self.code.len().saturating_sub(1) as u32;
        self.declared_types.push((pc, bt));
    }

    /// The type a collection construction names: `a new Seq of T`, `a new Set of T`,
    /// `a new Map of K to V`. `None` for a record, or for an element type the boundary does not model.
    fn declared_collection(
        &self,
        type_name: Symbol,
        type_args: &[crate::ast::stmt::TypeExpr],
    ) -> Option<BoundaryType> {
        let arg = |i: usize| boundary_of_type_expr(&type_args[i], &self.user_types, self.interner);
        match (self.interner.resolve(type_name), type_args.len()) {
            ("Seq" | "List", 1) => Some(BoundaryType::Seq(Box::new(arg(0)?))),
            ("Set" | "HashSet", 1) => Some(BoundaryType::Set(Box::new(arg(0)?))),
            ("Map" | "HashMap", 2) => Some(BoundaryType::Map(Box::new(arg(0)?), Box::new(arg(1)?))),
            _ => None,
        }
    }

    /// The type a `Let` binding declares for its value: its annotation (`Let x: Seq of Text be …`),
    /// else the declared return type of the function its value calls. A construction declares its
    /// own type where it is compiled ([`Compiler::declared_collection`]).
    fn declared_binding(
        &self,
        ty: Option<&crate::ast::stmt::TypeExpr>,
        value: &Expr,
    ) -> Option<BoundaryType> {
        if let Some(ty) = ty {
            return boundary_of_type_expr(ty, &self.user_types, self.interner);
        }
        match value {
            Expr::Call { function, .. } => self.return_type_of(*function),
            _ => None,
        }
    }

    /// The declared return type of the program's function `f`; `None` for a builtin or a function
    /// whose return type the boundary does not model.
    fn return_type_of(&self, f: Symbol) -> Option<BoundaryType> {
        self.fn_index
            .get(&f)
            .and_then(|&idx| self.functions[idx as usize].return_type.clone())
    }

    fn let_reg(&mut self, sym: Symbol) -> Result<Reg, Refusal> {
        let r = if let Some(&r) = self.scopes.last().unwrap().get(&sym) {
            self.mark_named(r);
            r
        } else {
            let r = self.alloc_reg()?;
            self.scopes.last_mut().unwrap().insert(sym, r);
            self.mark_named(r);
            r
        };
        self.record_dbg_name(sym, r);
        // A consume-alias of a mutable-borrow param (`Let result be arr`): its
        // register is COW-exempt, since the in-place writes land on it, not the param.
        if self.current_exempt_syms.contains(&sym) && !self.current_exempt_regs.contains(&r) {
            self.current_exempt_regs.push(r);
        }
        Ok(r)
    }

    /// DEBUG-ONLY: remember that register `r` holds source variable `sym` in the
    /// current frame (last name wins after block-scope register reuse). A no-op
    /// unless the debugger's compile path enabled name capture.
    fn record_dbg_name(&mut self, sym: Symbol, r: Reg) {
        if !self.debug_names {
            return;
        }
        match self.dbg_names.iter_mut().find(|(rr, _)| *rr == r) {
            Some(slot) => slot.1 = sym,
            None => self.dbg_names.push((r, sym)),
        }
    }

    /// Record that `r` carries a user-visible name in the current frame.
    fn mark_named(&mut self, r: Reg) {
        let i = r as usize;
        if self.named.len() <= i {
            self.named.resize(i + 1, false);
        }
        self.named[i] = true;
        self.record_loop_local(r);
    }

    /// Record register `r` as LOCAL to every enclosing loop — lexically dead at
    /// each loop's exit — so the region JIT drops it from the write-back set.
    /// Called for named bindings (`mark_named`) AND for anonymous scratch
    /// reserved inside a loop: a per-iteration temp is dead at the loop exit too,
    /// and may have recycled a now-dead named variable's slot (block-scope reuse).
    /// Without this, such a temp keeps that slot's stale `named` flag and the
    /// region JIT treats it as live — blocking fusions like the swap idiom
    /// (fannkuch's `perm[hi]` transfer temp).
    fn record_loop_local(&mut self, r: Reg) {
        let i = r as usize;
        for &head in &self.loop_stack {
            let m = self.loop_locals.entry(head).or_default();
            if m.len() <= i {
                m.resize(i + 1, false);
            }
            m[i] = true;
        }
    }

    fn add_const(&mut self, c: Constant) -> Result<ConstIdx, Refusal> {
        let key = const_key(&c);
        if let Some(&idx) = self.const_map.get(&key) {
            return Ok(idx);
        }
        let idx = ConstIdx::try_from(self.constants.len())
            .map_err(|_| "vm: constant pool overflow".to_string())?;
        self.constants.push(c);
        self.const_map.insert(key, idx);
        Ok(idx)
    }

    /// Register an observation site (Wave 13), returning its `Op::Obs { site }`
    /// index. Only ever called in the OBSERVED profile.
    fn add_obs_site(&mut self, desc: ObsSiteDesc) -> u32 {
        let idx = self.observation_sites.len() as u32;
        self.observation_sites.push(desc);
        idx
    }

    fn emit(&mut self, op: Op) {
        self.code.push(op);
    }

    fn lower_stmt(&mut self, s: &Stmt) -> Result<(), Refusal> {
        match s {
            // A runtime failure-site marker (Wave 3): no op, just the pc it stood at — see
            // `CompiledProgram::site_at`'s own doc comment.
            Stmt::Site(id) => {
                self.site_at.push((self.code.len() as u32, *id));
                Ok(())
            }
            // A component is a DECLARATION here: the parser has already lowered a renderable
            // view into a `__render_<Name>` function, which compiles like any other. An
            // `## Error page` is one too — the router renders it in place of the bare status —
            // unless the server-side walk cannot render its template, which is refused at the
            // node the walk refused. This guard sits BEFORE the general arm below, which would
            // otherwise take every component first.
            Stmt::ComponentDef(def)
                if matches!(
                    def.kind,
                    crate::ast::component::ComponentKind::ErrorPage { .. }
                ) =>
            {
                error_page_refusal(def, self.interner).map_or(Ok(()), Err)
            }
            Stmt::ComponentDef(_) => Ok(()),
            // `Mount App [at "#sel"].` — resolve the mount point, then call the lowered render
            // function. A component whose view needs the runtime has none, and this refuses by
            // name rather than rendering part of a page.
            Stmt::Ui(crate::ast::stmt::UiStmt::Mount {
                component,
                selector,
            }) => {
                let name = self.interner.resolve(*component).to_string();
                let fn_name =
                    self.interner
                        .lookup(&logicaffeine_language::parser::lgx_lower::mount_fn_name(
                            &name,
                        ));
                let func = fn_name.and_then(|s| self.fn_index.get(&s).copied());
                let Some(func) = func else {
                    return Err(self
                        .mount_refusals
                        .get(component)
                        .cloned()
                        .unwrap_or_else(|| {
                            format!(
                            "`Mount {name}` cannot run on this tier: this file does not declare it \
                             as a mountable component."
                        )
                            .into()
                        }));
                };
                let parent = self.alloc_reg()?;
                match selector {
                    None => self.emit(Op::CallBuiltin {
                        dst: parent,
                        builtin: crate::semantics::builtins::BuiltinId::DomBody,
                        args_start: 0,
                        arg_count: 0,
                    }),
                    Some(sel) => {
                        // `querySelector` answers -1 when nothing matches, and a page appended to
                        // -1 would parent every node to a node that is not there while reporting
                        // success. The failure is raised here — where the selector's text is
                        // known — in the reference tier's words; a non-literal selector is kept
                        // where `CallBuiltin`'s post-return argument clear cannot reach it.
                        let selector = self.keep_operand(sel)?;
                        let arg = self.reserve_regs(1)?;
                        self.compile_expr_into(sel, arg)?;
                        self.keep_move(&selector, arg);
                        self.emit(Op::CallBuiltin {
                            dst: parent,
                            builtin: crate::semantics::builtins::BuiltinId::DomQuerySelector,
                            args_start: arg,
                            arg_count: 1,
                        });
                        let missing_text = DriveText::render(
                            |s, _| crate::semantics::drive::mount_target_missing(&name, s),
                            &selector,
                            None,
                        );
                        let zero = self.alloc_reg()?;
                        let zidx = self.add_const(Constant::Int(0))?;
                        self.emit(Op::LoadConst {
                            dst: zero,
                            idx: zidx,
                        });
                        let missing = self.alloc_reg()?;
                        self.emit(Op::Lt {
                            dst: missing,
                            lhs: parent,
                            rhs: zero,
                        });
                        let ok = self.code.len();
                        self.emit(Op::JumpIfFalse {
                            cond: missing,
                            target: usize::MAX,
                        });
                        self.emit_fail_text(&missing_text)?;
                        self.patch_jump_target(ok, self.current_pc())?;
                    }
                }
                let args_start = self.reserve_regs(1)?;
                self.emit(Op::Move {
                    dst: args_start,
                    src: parent,
                });
                let dst = self.alloc_reg()?;
                self.emit(Op::Call {
                    dst,
                    func,
                    args_start,
                    arg_count: 1,
                });
                Ok(())
            }
            // The driving verbs route through the synthesized dispatcher, which the parser
            // built from the handlers the views attach. This tier cannot call a closure it
            // found at run time, so the routing is decided where the handlers are known.
            // ★ `Raise "offline".` — a WINDOW event, delivered to the page. It routes through the
            // same dispatcher every other driving verb uses; what differs is that the page always
            // exists and an unanswered raise is not a mistake, so `__lgx_raise` reports "answered"
            // whatever happened and this never raises the "nothing answers" failure.
            Stmt::Ui(crate::ast::stmt::UiStmt::RaiseWindowEvent {
                event,
                target,
                value,
            }) => {
                // ★ A PAGE THAT ANSWERS NOTHING STILL ACCEPTS A RAISE. Every other driving verb
                // fails when the program has no dispatcher, and rightly: clicking an element
                // nothing listens to is a test naming the wrong element. Going offline is not
                // something the page did, and a page is entitled to ignore it — the reference tier
                // says exactly that by delivering with `must_answer: false`. With no dispatcher
                // there is nothing to call and nothing to report, so this compiles to nothing.
                let driver = logicaffeine_language::parser::lgx_lower::raise_fn_name();
                match self
                    .interner
                    .lookup(driver)
                    .and_then(|s| self.fn_index.get(&s).copied())
                {
                    Some(func) => self.compile_page_drive(func, &[target, event, value]),
                    None => Ok(()),
                }
            }
            Stmt::Ui(crate::ast::stmt::UiStmt::Click { selector }) => {
                self.compile_lgx_drive(crate::semantics::drive::Verb::Click, selector, None, None)
            }
            // A press with no selector goes to the page; the parser fills in that selector, so
            // both forms arrive here the same shape.
            Stmt::Ui(crate::ast::stmt::UiStmt::Press {
                key,
                selector: Some(selector),
            }) => self.compile_lgx_drive(
                crate::semantics::drive::Verb::Press,
                selector,
                Some(key),
                Some(key),
            ),
            Stmt::Ui(crate::ast::stmt::UiStmt::TypeInto { text, selector }) => self
                .compile_lgx_drive(
                    crate::semantics::drive::Verb::Type,
                    selector,
                    Some(text),
                    None,
                ),
            // `Submit SEL.` had the AST, the parser and the reference tier and NO lowering here,
            // so it fell through to the generic refusal below — a verb that looked landed and
            // only ever worked on one tier.
            Stmt::Ui(crate::ast::stmt::UiStmt::Submit { selector }) => {
                self.compile_lgx_drive(crate::semantics::drive::Verb::Submit, selector, None, None)
            }
            Stmt::Ui(crate::ast::stmt::UiStmt::Choose { value, selector }) => self
                .compile_lgx_drive(
                    crate::semantics::drive::Verb::Choose,
                    selector,
                    Some(value),
                    None,
                ),
            // A directive is a declaration: nothing to emit here, and a component that uses one
            // is refused where it MOUNTS, by name — which is the message worth reading.
            Stmt::DirectiveDef { .. } => Ok(()),
            // A `## Route` whose body lowered to a function is a DECLARATION here — the function
            // carries it, and `route(method, path)` dispatches. One that could not lower is
            // refused BY NAME, saying which part is missing, rather than silently never being
            // reachable through the dispatcher (which would read as a 404 for a route the author
            // can see in their file).
            Stmt::RouteDef { method, path, .. } => match self.route_refusals.get(&(*method, *path))
            {
                None => Ok(()),
                Some(refusal) => Err(refusal.clone()),
            },
            // A `## Before each request` is a DECLARATION here — it lowered to a function and
            // `route(method, path)` calls it before matching.
            Stmt::MiddlewareDef { .. } => Ok(()),
            // A `## Redirect` is a DECLARATION — the router carries it.
            Stmt::RedirectDef { .. } => Ok(()),
            // A typed request is REFUSED BY NAME, not left to the generic message below. Its
            // answer arrives in a CLOSURE, and this tier cannot call a closure it found at run
            // time — the same constraint that made the driving verbs route through a synthesized
            // dispatcher. Saying that is worth more than "the remaining action verbs".
            Stmt::Fetch { .. } => Err(
                "a typed request (`Get … as T, then action.`) cannot run on this tier yet: its \
                 answer arrives in a closure, and this tier cannot call a closure it found at \
                 run time. Drive the page on the tree-walker, or answer the request with a \
                 `Given that …` stub in a test."
                    .to_string()
                    .into(),
            ),
            // An `Actions:` verb with no lowering on this tier is refused by its own sentence
            // (`action_verb_tier_lock` holds the catalogue of which verbs lower). The tree-walker
            // runs every one of them, so the browser build is the one place such a gap shows, and
            // the refusal has to say WHICH sentence.
            Stmt::Ui(u) => Err(format!(
                "`{}` cannot run in a browser yet: it is an `Actions:` verb this tier has no \
                 lowering for. It works on the tree-walker (`largo test` runs it), so a page using \
                 it renders in a test and fails to build for the browser — rewrite that sentence, \
                 or drive it from a `## Test`.",
                ui_verb_phrase(u)
            )
            .into()),
            Stmt::GivenFetch { .. } | Stmt::ServeFetches | Stmt::Replay { .. } => {
                // The SAME words the tree-walker uses. Two tiers refusing the same construct
                // differently is a small thing that costs real time: whoever reads one message
                // and searches for it does not find the other.
                Err(
                    "LGX routes and the remaining action verbs are not executable on this tier yet"
                        .to_string()
                        .into(),
                )
            }
            Stmt::Splice { body } => {
                // Scope-transparent desugar output: compile the statements in
                // order, no scope of their own (temporaries are gensym'd).
                for inner in body.iter() {
                    self.compile_stmt(inner)?;
                }
                Ok(())
            }
            Stmt::Reversibly { body } => {
                // Output-transparent block: the body compiles in the enclosing scope, but while
                // `reversible_depth` is positive an in-place abelian scalar update lowers to the
                // zero-garbage reversible ISA (`Op::RevAdd`/`RevSub`/`RevXor`/`RevNeg`) rather than
                // the plain journaling op — so the region reverses with no history tape. The
                // FORWARD result is identical to the same statements outside the block.
                self.reversible_depth += 1;
                let result = (|| {
                    for inner in body.iter() {
                        self.compile_stmt(inner)?;
                    }
                    Ok(())
                })();
                self.reversible_depth -= 1;
                result
            }
            Stmt::Let { var, ty, value, .. } => {
                // A promoted Main TOP-LEVEL Let defines the global; everywhere
                // else (function bodies, Main blocks — which shadow) a
                // register binding.
                if !self.in_function && self.scopes.len() == 1 && self.promoted.contains_key(var) {
                    let scratch = self.alloc_reg()?;
                    self.compile_expr_into(value, scratch)?;
                    let idx = self.promoted[var];
                    self.emit(Op::GlobalSet { idx, src: scratch });
                    return Ok(());
                }
                // `Let var be a new Seq/Set/Map` (a fresh EMPTY collection):
                // bind first and construct directly into the var's register.
                // An empty constructor reads nothing, so the
                // evaluate-in-old-environment rule below is moot — and writing
                // into the var's stable register lets the loop-carried reuse
                // path (NewEmptyList over a sole-owned buffer) fire across
                // iterations instead of allocating fresh every pass.
                if let Expr::New {
                    type_name,
                    init_fields,
                    type_args,
                } = value
                {
                    if init_fields.is_empty()
                        && matches!(
                            self.interner.resolve(*type_name),
                            "Seq" | "List" | "Set" | "HashSet" | "Map" | "HashMap"
                        )
                    {
                        let dst = self.let_reg(*var)?;
                        // A narrowing-proven `new Seq of Int` allocates the
                        // half-width (`Vec<i32>`) buffer directly into the var's
                        // stable register (so the loop-carried reuse path fires).
                        let fixed_key = (self.tail_fn.map(|(function, _, _)| function), *var);
                        if matches!(self.interner.resolve(*type_name), "Seq" | "List")
                            && self.fixed_seq_caps.contains_key(&fixed_key)
                        {
                            let capacity =
                                u16::try_from(self.fixed_seq_caps[&fixed_key]).map_err(|_| {
                                    "vm: fixed sequence capacity exceeds 65535".to_string()
                                })?;
                            self.emit(Op::NewEmptyListFixed { dst, capacity });
                            if let Some(bt) = self.declared_collection(*type_name, type_args) {
                                self.note_declared(bt);
                            }
                        } else if self.narrowable_seqs.contains(var)
                            && matches!(self.interner.resolve(*type_name), "Seq" | "List")
                        {
                            self.emit(Op::NewEmptyListI32 { dst });
                            if let Some(bt) = self.declared_collection(*type_name, type_args) {
                                self.note_declared(bt);
                            }
                        } else {
                            self.compile_expr_into(value, dst)?;
                        }
                        return Ok(());
                    }
                }
                // The value is evaluated BEFORE the new binding exists (the
                // tree-walker evaluates in the old environment): `Let x be
                // x + 1` in a block reads the OUTER x. Compile into a scratch
                // register first, then bind.
                let scratch = self.alloc_reg()?;
                self.compile_expr_into(value, scratch)?;
                let dst = self.let_reg(*var)?;
                self.emit(Op::Move { dst, src: scratch });
                if let Some(bt) = self.declared_binding(*ty, value) {
                    self.note_declared(bt);
                }
                Ok(())
            }
            Stmt::Swap { a, b } => {
                // The zero-garbage reversible register exchange. Two distinct locals emit the abelian
                // `RevSwap` directly (its own inverse, no history-tape cost); equal names are a no-op;
                // anything else (e.g. a promoted global) falls back to a correct temp-mediated swap.
                match (self.resolve_name(*a), self.resolve_name(*b)) {
                    (NameRef::Local(ra), NameRef::Local(rb)) => {
                        if ra != rb {
                            self.emit(Op::RevSwap { a: ra, b: rb });
                        }
                    }
                    _ => {
                        let ta = self.alloc_reg()?;
                        self.emit_read(*a, ta)?;
                        let tb = self.alloc_reg()?;
                        self.emit_read(*b, tb)?;
                        self.emit_write(*a, tb)?;
                        self.emit_write(*b, ta)?;
                    }
                }
                Ok(())
            }
            Stmt::Set { target, value } => {
                match self.resolve_name(*target) {
                    NameRef::Local(dst) => {
                        // Inside a `Reversibly:` block, an in-place abelian scalar update lowers to
                        // the zero-garbage reversible ISA (`RevAdd`/`RevSub`/`RevXor`/`RevNeg`). The
                        // FORWARD result is identical to the plain lowering below, so program output
                        // is unchanged; only the emitted ops (and their tape-free reversibility)
                        // differ. Declines (falls through) for any non-abelian value form.
                        if self.reversible_depth > 0
                            && self.try_emit_reversible_set(*target, dst, value)?
                        {
                            return Ok(());
                        }
                        // `Set x to x + e1 + … + ek`: lower to an AddAssign
                        // chain (in-place Text append; identical fold for
                        // every other type). Only when each later term
                        // provably never observes x — see add_assign_chain.
                        if let Some(terms) = add_assign_chain(*target, value) {
                            for term in terms {
                                let scratch = self.alloc_reg()?;
                                self.compile_expr_into(term, scratch)?;
                                self.emit(Op::AddAssign { dst, src: scratch });
                            }
                            return Ok(());
                        }
                        // `Set x to f(…)`: compile the call straight into `x`. A call
                        // writes its destination LAST (after its arguments are read into
                        // the arg window), so `x` is never clobbered before use — even
                        // in the self-consuming `Set x to f(x, …)` shape. This avoids a
                        // scratch whose lingering `Rc` clone would leave `x` spuriously
                        // shared (`strong == 2`) and force the next mutable-borrow call's
                        // `EnsureOwned` barrier to deep-clone every iteration (the
                        // heap_sort O(n²) collapse under value semantics). Skipped when
                        // `x` is a COW-exempt local (its writes never clone anyway) so a
                        // self-recursive function's tier-able body is left untouched.
                        if matches!(value, Expr::Call { .. })
                            && !self.current_exempt_syms.contains(target)
                        {
                            self.compile_expr_into(value, dst)?;
                            return Ok(());
                        }
                        // Never compile directly into the live target: a
                        // multi-op value (interpolation accumulation, branch
                        // joins) would clobber the register before reading it
                        // — `Set result to "{result}{s}"` must accumulate.
                        let scratch = self.alloc_reg()?;
                        self.compile_expr_into(value, scratch)?;
                        self.emit(Op::Move { dst, src: scratch });
                        Ok(())
                    }
                    NameRef::Unbound => {
                        // The tree-walker evaluates the value FIRST (its side
                        // effects happen), then fails the assignment — and only
                        // when the statement actually executes.
                        let scratch = self.alloc_reg()?;
                        self.compile_expr_into(value, scratch)?;
                        self.emit_unbound(*target)
                    }
                    _ => {
                        let scratch = self.alloc_reg()?;
                        self.compile_expr_into(value, scratch)?;
                        self.emit_write(*target, scratch)
                    }
                }
            }
            Stmt::Return { value } => {
                // A Zone swallows Return (the tree-walker discards the zone
                // body's ControlFlow): jump to the zone's end instead of
                // returning, popping the iterator of every Repeat crossed.
                let zone_pos = self
                    .flow_stack
                    .iter()
                    .rposition(|c| matches!(c, FlowCtx::Zone { .. }));
                if let Some(pos) = zone_pos {
                    if let Some(e) = value {
                        let scratch = self.alloc_reg()?;
                        self.compile_expr_into(e, scratch)?;
                    }
                    let crossed_repeats = self.flow_stack[pos + 1..]
                        .iter()
                        .filter(|c| {
                            matches!(
                                c,
                                FlowCtx::Loop {
                                    is_repeat: true,
                                    ..
                                }
                            )
                        })
                        .count();
                    for _ in 0..crossed_repeats {
                        self.emit(Op::IterPop);
                    }
                    let j = self.code.len();
                    self.emit(Op::Jump { target: usize::MAX });
                    if let FlowCtx::Zone { exits } = &mut self.flow_stack[pos] {
                        exits.push(j);
                    }
                } else if self.in_function {
                    match value {
                        Some(e) => {
                            // A NAMED-RECORD return of a tuple literal `(a, b)` builds a self-describing struct
                            // with the record's field names, so `r's quotient` resolves via ordinary struct
                            // field access — mirroring the interpreter's `coerce_record_return`.
                            if let Some(crate::ast::stmt::TypeExpr::Record {
                                field_names, ..
                            }) = self.current_return_type
                            {
                                if let Expr::Tuple(items) = e {
                                    if field_names.len() == items.len() {
                                        let dst = self.alloc_reg()?;
                                        let name_idx =
                                            self.add_const(Constant::Text("Record".to_string()))?;
                                        self.emit(Op::NewStruct {
                                            dst,
                                            type_name: name_idx,
                                        });
                                        for (n, item) in field_names.iter().zip(items.iter()) {
                                            let fname = self.interner.resolve(*n).to_string();
                                            let fidx = self.add_const(Constant::Text(fname))?;
                                            let v = self.compile_expr(item)?;
                                            self.emit(Op::StructInsert {
                                                obj: dst,
                                                field: fidx,
                                                value: v,
                                            });
                                        }
                                        self.emit(Op::Return { src: dst });
                                        return Ok(());
                                    }
                                }
                            }
                            // TCO: a direct self-tail-call `Return self(args)` is
                            // inherently in tail position — lower it to the
                            // loop-back (reassign params + jump to entry) instead
                            // of a real call + return.
                            if let Some((tf, entry_pc, pc)) = self.tail_fn {
                                // Do NOT TCO across a `Repeat` (for-in): the
                                // loop-back jump would skip its `IterPop`,
                                // stacking abandoned iterators. (A `While`
                                // holds no runtime state, so jumping out of it
                                // is fine; `Zone`s are handled above.)
                                let in_repeat = self.flow_stack.iter().any(|c| {
                                    matches!(
                                        c,
                                        FlowCtx::Loop {
                                            is_repeat: true,
                                            ..
                                        }
                                    )
                                });
                                if !in_repeat {
                                    if let Some(args) =
                                        crate::tail_call::direct_self_tail_args(*e, tf, pc as usize)
                                    {
                                        self.emit_tail_call(args, entry_pc)?;
                                        return Ok(());
                                    }
                                }
                            }
                            let src = self.compile_expr(e)?;
                            self.emit(Op::Return { src });
                        }
                        None => self.emit(Op::ReturnNothing),
                    }
                } else {
                    // Return at Main top level = stop the program (the value's
                    // side effects still happen first).
                    if let Some(e) = value {
                        let scratch = self.alloc_reg()?;
                        self.compile_expr_into(e, scratch)?;
                    }
                    self.emit(Op::Halt);
                }
                Ok(())
            }
            Stmt::Break => {
                // The innermost Loop or Zone catches it, whichever is nearer.
                match self.flow_stack.last_mut() {
                    Some(FlowCtx::Loop { breaks, .. }) => {
                        let j = self.code.len();
                        breaks.push(j);
                        self.emit(Op::Jump { target: usize::MAX });
                    }
                    Some(FlowCtx::Zone { .. }) => {
                        let j = self.code.len();
                        self.emit(Op::Jump { target: usize::MAX });
                        if let Some(FlowCtx::Zone { exits }) = self.flow_stack.last_mut() {
                            exits.push(j);
                        }
                    }
                    None => {
                        if self.in_function {
                            // A Break with no enclosing loop ends the function
                            // body (the tree-walker's call loop treats Break
                            // as "stop executing the body").
                            self.emit(Op::ReturnNothing);
                        } else {
                            // At Main top level it stops the program.
                            self.emit(Op::Halt);
                        }
                    }
                }
                Ok(())
            }
            Stmt::Call { function, args } => {
                // A call used for its effect; the result register is discarded.
                let dst = self.alloc_reg()?;
                self.compile_call(*function, args, dst)
            }
            Stmt::Push { value, collection } => {
                // Value evaluated before the place (matches the tree-walker's `Stmt::Push`);
                // `compile_place_mutation` handles an unbound root itself.
                let val = self.compile_expr_before(value, &[collection])?;
                // FAST PATH: `Push V to <name>'s <field>.` with `<name>` a local — the ONE shape
                // `Op::ListPushField` exists for. It pushes onto the field of the record in the
                // binding's OWN register, taking the field's list for that record first when
                // another value shares it (a copy of the record, a plain parameter's caller), so
                // the push is seen through this binding and no other. A global or a capture is
                // read into a scratch copy, where a push would be lost; those take the general
                // place mechanism, which writes the record back. Kept as its own op rather than
                // folded into the general mechanism: every other backend (RISC-V/C/wasm/SVA/DRAM)
                // still lowers this op, and letting it go unemitted silently breaks their own
                // coverage censuses.
                if let Expr::FieldAccess {
                    object: Expr::Identifier(obj_sym),
                    field,
                } = collection
                {
                    if let NameRef::Local(obj) = self.resolve_name(*obj_sym) {
                        let name = self.interner.resolve(*field).to_string();
                        let fidx = self.add_const(Constant::Text(name))?;
                        self.emit(Op::ListPushField {
                            obj,
                            field: fidx,
                            src: val,
                        });
                        return Ok(());
                    }
                }
                self.compile_place_mutation(collection, &mut |c, list| {
                    c.emit(Op::ListPush { list, value: val });
                    Ok(())
                })?;
                Ok(())
            }
            Stmt::Add { value, collection } => {
                // Value evaluated before the place (matches the tree-walker's `Stmt::Add`).
                let val = self.compile_expr_before(value, &[collection])?;
                self.compile_place_mutation(collection, &mut |c, set| {
                    c.emit(Op::SetAdd { set, value: val });
                    Ok(())
                })?;
                Ok(())
            }
            Stmt::Remove { value, collection } => {
                // Value evaluated before the place (matches the tree-walker's `Stmt::Remove`).
                let val = self.compile_expr_before(value, &[collection])?;
                self.compile_place_mutation(collection, &mut |c, coll| {
                    c.emit(Op::RemoveFrom {
                        collection: coll,
                        value: val,
                    });
                    Ok(())
                })?;
                Ok(())
            }
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => {
                // Tree-walker order: index, then value, then the place itself.
                let proven = self.index_in_bounds(collection, index);
                let idx = self.compile_expr_before(index, &[value, collection])?;
                let val = self.compile_expr_before(value, &[collection])?;
                self.compile_place_mutation(collection, &mut |c, coll| {
                    if proven {
                        c.emit(Op::SetIndexUnchecked {
                            collection: coll,
                            index: idx,
                            value: val,
                        });
                    } else {
                        c.emit(Op::SetIndex {
                            collection: coll,
                            index: idx,
                            value: val,
                        });
                    }
                    Ok(())
                })?;
                Ok(())
            }
            Stmt::Show { object, recipient } => {
                if let Expr::Identifier(sym) = recipient {
                    self.compile_recipient_call(*sym, object)
                } else {
                    // Non-identifier recipient: the tree-walker still evaluates
                    // the object (side effects), then silently moves on.
                    let scratch = self.alloc_reg()?;
                    self.compile_expr_into(object, scratch)?;
                    Ok(())
                }
            }
            Stmt::Give { object, recipient } => {
                if let Expr::Identifier(sym) = recipient {
                    self.compile_recipient_call(*sym, object)
                } else {
                    // Non-identifier recipient: the tree-walker evaluates the
                    // object (side effects) and silently moves on.
                    let scratch = self.alloc_reg()?;
                    self.compile_expr_into(object, scratch)?;
                    Ok(())
                }
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let c = self.compile_expr(cond)?;
                let jif = self.emit_placeholder_jump_if_false(c);
                let mark = self.enter_block();
                for st in *then_block {
                    self.compile_stmt(st)?;
                }
                self.exit_block(mark);
                match else_block {
                    Some(eb) => {
                        let jend = self.emit_placeholder_jump();
                        self.patch_jump_target(jif, self.current_pc())?;
                        let mark = self.enter_block();
                        for st in *eb {
                            self.compile_stmt(st)?;
                        }
                        self.exit_block(mark);
                        self.patch_jump_target(jend, self.current_pc())?;
                    }
                    None => {
                        self.patch_jump_target(jif, self.current_pc())?;
                    }
                }
                Ok(())
            }
            Stmt::While { cond, body, .. } => {
                // Region-entry bounds hoists go at the loop head (the back-edge
                // target), so they fall inside any region formed for the loop.
                let loop_start = self.current_pc();
                // Track names bound inside this loop body (the region head is
                // `loop_start`, the back-edge target) so the JIT can treat them
                // as dead-at-exit scratch. Pushed before the cond (re-evaluated
                // each iteration, so cond-bound names are loop-local too).
                self.loop_stack.push(loop_start);
                let enabled = self.emit_hoist_guards(s);
                self.hoist_enabled.push(enabled);
                let c = self.compile_expr(cond)?;
                let jexit = self.emit_placeholder_jump_if_false(c);
                self.flow_stack.push(FlowCtx::Loop {
                    breaks: Vec::new(),
                    is_repeat: false,
                });
                let mark = self.enter_block();
                let buffer_swap = detect_loop_buffer_swap(body, self.interner);
                for (body_idx, st) in body.iter().enumerate() {
                    if let Some(reuse) = buffer_swap.filter(|reuse| reuse.set_idx == body_idx) {
                        if let (NameRef::Local(inner), NameRef::Local(outer)) = (
                            self.resolve_name(reuse.inner),
                            self.resolve_name(reuse.outer),
                        ) {
                            if inner != outer {
                                self.emit(Op::RevSwap { a: outer, b: inner });
                                crate::optimize::mark_fired(crate::optimization::Opt::Peephole);
                                continue;
                            }
                        }
                    }
                    self.compile_stmt(st)?;
                }
                self.exit_block(mark);
                self.hoist_enabled.pop();
                self.loop_stack.pop();
                self.emit(Op::Jump { target: loop_start });
                let exit_pc = self.current_pc();
                self.patch_jump_target(jexit, exit_pc)?;
                if let Some(FlowCtx::Loop { breaks, .. }) = self.flow_stack.pop() {
                    for j in breaks {
                        self.patch_jump_target(j, exit_pc)?;
                    }
                }
                Ok(())
            }
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => self.compile_repeat(s, pattern, iterable, body),
            Stmt::Pop { collection, into } => {
                if !matches!(collection, Expr::Identifier(_)) {
                    // A list in a field or an item (`Pop from c's tags.`): the place mechanism takes
                    // it out, pops, and puts it back, whatever holds it.
                    let dst = match into {
                        Some(var) => self.let_reg(*var)?,
                        None => self.alloc_reg()?,
                    };
                    self.compile_place_mutation(collection, &mut |c, list| {
                        c.emit(Op::ListPop { list, dst });
                        Ok(())
                    })?;
                    return Ok(());
                }
                let (sym, nr) = self.resolve_collection(collection)?;
                match self.collection_reg(sym, &nr)? {
                    Some(list) => {
                        let dst = match into {
                            Some(var) => self.let_reg(*var)?,
                            None => self.alloc_reg()?,
                        };
                        self.emit(Op::ListPop { list, dst });
                        if !matches!(nr, NameRef::Local(_)) {
                            self.emit_write(sym, list)?;
                        }
                        Ok(())
                    }
                    None => self.emit_unbound(sym),
                }
            }
            Stmt::RuntimeAssert {
                condition, reason, ..
            } => {
                let c = self.compile_expr(condition)?;
                let jok = self.code.len();
                self.emit(Op::JumpIfTrue {
                    cond: c,
                    target: usize::MAX,
                });
                match reason {
                    // A CONSTANT reason is the message itself (`contract_reason_text` of a Text is that Text),
                    // so it needs no builtin: the core `FailWith` op carries it, and every tier that lowers
                    // from bytecode reports the same words.
                    Some(Expr::Literal(Literal::Text(sym))) => {
                        let idx = self
                            .add_const(Constant::Text(self.interner.resolve(*sym).to_string()))?;
                        self.emit(Op::FailWith { msg: idx });
                    }
                    Some(r) => self
                        .emit_fail_builtin(crate::semantics::builtins::BuiltinId::FailReason, r)?,
                    None => {
                        let idx = self.add_const(Constant::Text("Assertion failed".to_string()))?;
                        self.emit(Op::FailWith { msg: idx });
                    }
                }
                self.patch_jump_target(jok, self.current_pc())?;
                Ok(())
            }
            Stmt::Fail { reason } => {
                self.emit_fail_builtin(crate::semantics::builtins::BuiltinId::Fail, reason)
            }
            // A `## Test` block is a declaration — the VM emits nothing for it;
            // the `largo test` runner drives test bodies itself (Wave 3).
            Stmt::TestDef { .. } => Ok(()),
            // `Advance time by …` drives the page's timers through the same function a browser's
            // interval calls. Where the program has no timers there is no tick to call, and it is
            // a test-only statement that compiles away.
            Stmt::AdvanceTime { millis }
                if self
                    .interner
                    .lookup(logicaffeine_language::parser::lgx_lower::tick_fn_name())
                    .and_then(|s| self.fn_index.get(&s).copied())
                    .is_some() =>
            {
                let tick = self
                    .interner
                    .lookup(logicaffeine_language::parser::lgx_lower::tick_fn_name())
                    .and_then(|s| self.fn_index.get(&s).copied())
                    .ok_or_else(|| {
                        Refusal::from("vm: the page clock's tick is not compiled".to_string())
                    })?;
                self.compile_page_drive(tick, &[millis])
            }
            // Test-only statements are parser-gated to `## Test` bodies; the
            // main program's bytecode can never contain them.
            Stmt::Expect { .. }
            | Stmt::ExpectFail { .. }
            | Stmt::ExpectOutput { .. }
            | Stmt::AdvanceTime { .. }
            | Stmt::GivenFile { .. }
            | Stmt::ForAll { .. } => {
                if self.test_noop {
                    // `--tiers all`: the test's assertions are the interpreter's
                    // authoritative verdict; here the VM runs the computation
                    // around them, so these compile away.
                    Ok(())
                } else {
                    Err("test-only statement outside the test runner (`largo test` lands in Wave 3)"
                        .to_string().into())
                }
            }
            // Observation (Wave 13). OBSERVED profile ⇒ lower to `Op::Obs` so the
            // VM emits the tree-walker's event stream; ELIDED profile ⇒ erase
            // (`During` is scope-transparent — its body compiles inline).
            Stmt::Observe {
                category,
                message,
                payloads,
                sample,
            } => {
                if self.emit_observation {
                    let count = u16::try_from(payloads.len()).map_err(|_| {
                        "vm: observation has too many payloads (max 65535)".to_string()
                    })?;
                    let start = if count > 0 {
                        self.reserve_regs(count)?
                    } else {
                        0
                    };
                    let mut names = Vec::with_capacity(payloads.len());
                    for (i, (name, expr)) in payloads.iter().enumerate() {
                        self.compile_expr_into(expr, start + i as Reg)?;
                        names.push(self.interner.resolve(*name).to_string());
                    }
                    let site = self.add_obs_site(ObsSiteDesc {
                        kind: ObsSiteKind::Message(map_obs_cat(*category)),
                        message: self.interner.resolve(*message).to_string(),
                        payload_names: names,
                        sample: *sample,
                    });
                    self.emit(Op::Obs {
                        site,
                        args_start: start,
                        args_count: count,
                    });
                }
                Ok(())
            }
            Stmt::During { label, body } => {
                if self.emit_observation {
                    let name = self.interner.resolve(*label).to_string();
                    let enter = self.add_obs_site(ObsSiteDesc {
                        kind: ObsSiteKind::SpanEnter,
                        message: name.clone(),
                        payload_names: Vec::new(),
                        sample: None,
                    });
                    self.emit(Op::Obs {
                        site: enter,
                        args_start: 0,
                        args_count: 0,
                    });
                    for s in body.iter() {
                        self.compile_stmt(s)?;
                    }
                    let exit = self.add_obs_site(ObsSiteDesc {
                        kind: ObsSiteKind::SpanExit,
                        message: name,
                        payload_names: Vec::new(),
                        sample: None,
                    });
                    self.emit(Op::Obs {
                        site: exit,
                        args_start: 0,
                        args_count: 0,
                    });
                } else {
                    for s in body.iter() {
                        self.compile_stmt(s)?;
                    }
                }
                Ok(())
            }
            Stmt::Zone { name, body, .. } => self.compile_zone(*name, body),
            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
                // Sequential in the interpreter spec. Each task runs WITHOUT a
                // block scope (its Lets persist), and its Break/Return is
                // swallowed — the tree-walker discards each task's ControlFlow.
                for task in tasks.iter() {
                    self.flow_stack.push(FlowCtx::Zone { exits: Vec::new() });
                    self.compile_stmt(task)?;
                    let end_pc = self.current_pc();
                    if let Some(FlowCtx::Zone { exits }) = self.flow_stack.pop() {
                        for j in exits {
                            self.patch_jump_target(j, end_pc)?;
                        }
                    }
                }
                Ok(())
            }
            Stmt::Sleep { milliseconds } => {
                let duration = self.compile_expr(milliseconds)?;
                self.emit(Op::Sleep { duration });
                Ok(())
            }
            // Verification-only / declaration statements: no runtime effect.
            Stmt::Assert { .. }
            | Stmt::Trust { .. }
            | Stmt::Require { .. }
            | Stmt::Theorem(_)
            | Stmt::Definition(_)
            | Stmt::Axiom(_)
            | Stmt::Theory(_) => Ok(()),
            // Definitions were registered in pass 1.
            Stmt::StructDef { .. } => Ok(()),
            Stmt::SetField {
                object,
                field,
                value,
            } => {
                let field_name = self.interner.resolve(*field).to_string();
                let fidx = self.add_const(Constant::Text(field_name))?;
                if let Expr::Identifier(obj_sym) = object {
                    // Tree-walker order: value first, then the target lookup.
                    match self.resolve_name(*obj_sym) {
                        NameRef::Local(obj) => {
                            let v = self.compile_expr(value)?;
                            self.emit(Op::StructInsert {
                                obj,
                                field: fidx,
                                value: v,
                            });
                            Ok(())
                        }
                        NameRef::Unbound => {
                            let scratch = self.alloc_reg()?;
                            self.compile_expr_into(value, scratch)?;
                            self.emit_unbound(*obj_sym)
                        }
                        _ => {
                            // Structs are VALUES: load the global's copy,
                            // mutate it, store it back.
                            let v = self.compile_expr(value)?;
                            let obj = self.alloc_reg()?;
                            self.emit_read(*obj_sym, obj)?;
                            self.emit(Op::StructInsert {
                                obj,
                                field: fidx,
                                value: v,
                            });
                            self.emit_write(*obj_sym, obj)
                        }
                    }
                } else {
                    // A field of a field (`Set a's session's name to who`): the value first, then the
                    // place mechanism takes the inner record out, writes its field, and puts it back
                    // — through every level, whatever holds the outer one.
                    let v = self.compile_expr(value)?;
                    self.compile_place_mutation(object, &mut |c, obj| {
                        c.emit(Op::StructInsert {
                            obj,
                            field: fidx,
                            value: v,
                        });
                        Ok(())
                    })?;
                    Ok(())
                }
            }
            Stmt::Inspect { target, arms, .. } => self.compile_inspect(target, arms),
            Stmt::IncreaseCrdt {
                object,
                field,
                amount,
            }
            | Stmt::DecreaseCrdt {
                object,
                field,
                amount,
            } => {
                let negate = matches!(s, Stmt::DecreaseCrdt { .. });
                let amt = self.compile_expr(amount)?;
                let fname = self.interner.resolve(*field).to_string();
                let fidx = self.add_const(Constant::Text(fname))?;
                if let Expr::Identifier(obj_sym) = object {
                    match self.resolve_name(*obj_sym) {
                        NameRef::Local(obj) => {
                            self.emit(Op::CrdtBump {
                                obj,
                                field: fidx,
                                amount: amt,
                                negate,
                            });
                            Ok(())
                        }
                        NameRef::Unbound => self.emit_unbound(*obj_sym),
                        _ => {
                            let obj = self.alloc_reg()?;
                            self.emit_read(*obj_sym, obj)?;
                            self.emit(Op::CrdtBump {
                                obj,
                                field: fidx,
                                amount: amt,
                                negate,
                            });
                            self.emit_write(*obj_sym, obj)
                        }
                    }
                } else {
                    let msg = if negate {
                        "DecreaseCrdt target must be an identifier"
                    } else {
                        "IncreaseCrdt target must be an identifier"
                    };
                    let idx = self.add_const(Constant::Text(msg.to_string()))?;
                    self.emit(Op::FailWith { msg: idx });
                    Ok(())
                }
            }
            Stmt::MergeCrdt { source, target } => {
                let src = self.compile_expr(source)?;
                if let Expr::Identifier(target_sym) = target {
                    match self.resolve_name(*target_sym) {
                        NameRef::Local(tgt) => {
                            self.emit(Op::CrdtMerge {
                                target: tgt,
                                source: src,
                            });
                            Ok(())
                        }
                        NameRef::Unbound => self.emit_unbound(*target_sym),
                        _ => {
                            let tgt = self.alloc_reg()?;
                            self.emit_read(*target_sym, tgt)?;
                            self.emit(Op::CrdtMerge {
                                target: tgt,
                                source: src,
                            });
                            self.emit_write(*target_sym, tgt)
                        }
                    }
                } else {
                    let idx = self.add_const(Constant::Text(
                        "Merge target must be an identifier".to_string(),
                    ))?;
                    self.emit(Op::FailWith { msg: idx });
                    Ok(())
                }
            }
            // The IDEMPOTENT sibling of `MergeCrdt`: emits `Op::CrdtJoin` (per-field MAX for bare-Int, so a
            // redelivered state never double-counts). Register handling / unbound checks mirror `MergeCrdt`.
            Stmt::JoinCrdt { source, target } => {
                let src = self.compile_expr(source)?;
                if let Expr::Identifier(target_sym) = target {
                    match self.resolve_name(*target_sym) {
                        NameRef::Local(tgt) => {
                            self.emit(Op::CrdtJoin {
                                target: tgt,
                                source: src,
                            });
                            Ok(())
                        }
                        NameRef::Unbound => self.emit_unbound(*target_sym),
                        _ => {
                            let tgt = self.alloc_reg()?;
                            self.emit_read(*target_sym, tgt)?;
                            self.emit(Op::CrdtJoin {
                                target: tgt,
                                source: src,
                            });
                            self.emit_write(*target_sym, tgt)
                        }
                    }
                } else {
                    let idx = self.add_const(Constant::Text(
                        "Converge target must be an identifier".to_string(),
                    ))?;
                    self.emit(Op::FailWith { msg: idx });
                    Ok(())
                }
            }
            Stmt::ReadFrom { var, source } => {
                use crate::ast::stmt::ReadSource;
                match source {
                    ReadSource::Console => {
                        // Console reads yield empty Text in the interpreter.
                        let dst = self.let_reg(*var)?;
                        let idx = self.add_const(Constant::Text(String::new()))?;
                        self.emit(Op::LoadConst { dst, idx });
                        Ok(())
                    }
                    ReadSource::File(path_expr) => {
                        // The SAME sandboxed VFS the tree-walker's `ReadFrom` reads through
                        // (`BuiltinId::VfsReadFile`), so a file read errors identically on both
                        // tiers rather than the VM reporting an uninitialized VFS the
                        // tree-walker never has.
                        let arg = self.reserve_regs(1)?;
                        self.compile_expr_into(path_expr, arg)?;
                        let dst = self.let_reg(*var)?;
                        self.emit(Op::CallBuiltin {
                            dst,
                            builtin: crate::semantics::builtins::BuiltinId::VfsReadFile,
                            args_start: arg,
                            arg_count: 1,
                        });
                        Ok(())
                    }
                }
            }
            Stmt::WriteFile { content, path } => {
                // The SAME sandboxed VFS the tree-walker's `WriteFile` writes through
                // (`BuiltinId::VfsWriteFile`): the content, then the path, in the order the
                // tree-walker evaluates them, so a failing expression fails first on both tiers.
                let args = self.reserve_regs(2)?;
                self.compile_expr_into(content, args)?;
                self.compile_expr_into(path, args + 1)?;
                let dst = self.alloc_reg()?;
                self.emit(Op::CallBuiltin {
                    dst,
                    builtin: crate::semantics::builtins::BuiltinId::VfsWriteFile,
                    args_start: args,
                    arg_count: 2,
                });
                Ok(())
            }
            Stmt::Mount { var, path } => {
                // The journal's text from the same VFS, or empty Text when there is none yet
                // (`BuiltinId::VfsMount`) — the tree-walker's `Mount` arm, on the VM.
                let arg = self.reserve_regs(1)?;
                self.compile_expr_into(path, arg)?;
                let dst = self.let_reg(*var)?;
                self.emit(Op::CallBuiltin {
                    dst,
                    builtin: crate::semantics::builtins::BuiltinId::VfsMount,
                    args_start: arg,
                    arg_count: 1,
                });
                Ok(())
            }
            Stmt::Spawn { name, .. } => {
                // Agents do not run in the interpreter: the handle is Nothing.
                let dst = self.let_reg(*name)?;
                let idx = self.add_const(Constant::Nothing)?;
                self.emit(Op::LoadConst { dst, idx });
                Ok(())
            }
            // Peer messaging: emit the networking opcodes the async VM net driver
            // (`run_vm_net_async`) services through the shared `NetInbox` — the same inbox the
            // tree-walker uses, so a `Send`/`Await` is byte-identical on both tiers. (The dial
            // knobs — compression/cached/redundant/view — are not yet threaded here; the default
            // self-describing encode still decodes identically, so output parity holds.)
            Stmt::SendMessage {
                message,
                destination,
                ..
            } => {
                let to = self.compile_expr(destination)?;
                let msg = self.compile_expr(message)?;
                self.emit(Op::NetSend { to, msg });
                Ok(())
            }
            Stmt::StreamMessage {
                values,
                destination,
            } => {
                let to = self.compile_expr(destination)?;
                let vals = self.compile_expr(values)?;
                self.emit(Op::NetStream { to, values: vals });
                Ok(())
            }
            Stmt::AwaitMessage {
                source,
                into,
                stream,
                ..
            } => {
                let from = self.compile_expr(source)?;
                let dst = self.let_reg(*into)?;
                self.emit(Op::NetAwait {
                    dst,
                    from,
                    stream: *stream,
                });
                Ok(())
            }
            Stmt::Check {
                subject,
                predicate,
                is_capability,
                object,
                source_text,
                ..
            } => {
                let subj = self.alloc_reg()?;
                self.emit_read(*subject, subj)?;
                // The object is only looked up for capability checks.
                let obj = if *is_capability {
                    match object {
                        Some(obj_sym) => {
                            let r = self.alloc_reg()?;
                            self.emit_read(*obj_sym, r)?;
                            r
                        }
                        None => Reg::MAX,
                    }
                } else {
                    Reg::MAX
                };
                let st = self.add_const(Constant::Text(source_text.clone()))?;
                self.emit(Op::CheckPolicy {
                    subject: subj,
                    predicate: *predicate,
                    is_capability: *is_capability,
                    object: obj,
                    source_text: st,
                });
                Ok(())
            }
            Stmt::AppendToSequence { sequence, value } => {
                // The RGA is always behind a shared `Rc` (whether a bare variable or a
                // struct field), so appending in place propagates — no write-back needed.
                // Value compiled first, matching the tree-walker's evaluation order.
                let val = self.compile_expr(value)?;
                let seq = self.compile_expr(sequence)?;
                self.emit(Op::CrdtAppend { seq, value: val });
                Ok(())
            }
            Stmt::ResolveConflict {
                object,
                field,
                value,
            } => {
                let val = self.compile_expr(value)?;
                let fname = self.interner.resolve(*field).to_string();
                let fidx = self.add_const(Constant::Text(fname))?;
                // Mirrors `IncreaseCrdt`: a register field resolves in place via its shared
                // `Rc`; a plain field is overwritten, so it needs the read/write-back path
                // (structs have value semantics).
                if let Expr::Identifier(obj_sym) = object {
                    match self.resolve_name(*obj_sym) {
                        NameRef::Local(obj) => {
                            self.emit(Op::CrdtResolve {
                                obj,
                                field: fidx,
                                value: val,
                            });
                            Ok(())
                        }
                        NameRef::Unbound => self.emit_unbound(*obj_sym),
                        _ => {
                            let obj = self.alloc_reg()?;
                            self.emit_read(*obj_sym, obj)?;
                            self.emit(Op::CrdtResolve {
                                obj,
                                field: fidx,
                                value: val,
                            });
                            self.emit_write(*obj_sym, obj)
                        }
                    }
                } else {
                    let idx = self.add_const(Constant::Text(
                        "Resolve target must be a struct field".to_string(),
                    ))?;
                    self.emit(Op::FailWith { msg: idx });
                    Ok(())
                }
            }
            // Relay networking opcodes — serviced by the async VM net driver (`run_vm_net_async`)
            // through the shared `NetInbox`, byte-identically to the tree-walker. `LetPeerAgent`
            // and `Sync` are not yet driven on the VM net runner (they stay async-tier below).
            Stmt::Listen { address, secure } => {
                if secure.is_some() {
                    return Err("the PNP one-time pad (`with pad`) is only supported by the interpreter tier, not the bytecode VM".to_string().into());
                }
                let topic = self.compile_expr(address)?;
                self.emit(Op::NetListen { topic });
                Ok(())
            }
            Stmt::Serve { port, handler } => {
                let handler_sym = match handler {
                    Expr::Identifier(sym) => *sym,
                    _ => {
                        return Err("Serve: the handler must be a named function"
                            .to_string()
                            .into())
                    }
                };
                let &func = self.fn_index.get(&handler_sym).ok_or_else(|| {
                    format!(
                        "Serve: the handler '{}' is not a defined function",
                        self.interner.resolve(handler_sym)
                    )
                })?;
                let port = self.compile_expr(port)?;
                self.emit(Op::Serve {
                    port,
                    handler: func,
                });
                Ok(())
            }
            Stmt::ConnectTo { address, secure } => {
                if secure.is_some() {
                    return Err("the PNP one-time pad (`with pad`) is only supported by the interpreter tier, not the bytecode VM".to_string().into());
                }
                let url = self.compile_expr(address)?;
                self.emit(Op::NetConnect { url });
                Ok(())
            }
            Stmt::LetPeerAgent { var, address } => {
                let addr = self.compile_expr(address)?;
                let dst = self.let_reg(*var)?;
                self.emit(Op::NetMakePeer { dst, addr });
                Ok(())
            }
            Stmt::Sync { var, topic } => {
                // Read the local counter, sync+merge over the relay, write the merged value back —
                // a read-modify-write on `var` through `Op::NetSync`.
                let topic_reg = self.compile_expr(topic)?;
                let tmp = self.alloc_reg()?;
                self.emit_read(*var, tmp)?;
                self.emit(Op::NetSync {
                    dst: tmp,
                    topic: topic_reg,
                });
                self.emit_write(*var, tmp)
            }
            // Go-like concurrency (T11d). The VM emits the scheduler opcodes; a
            // concurrent program is then driven through `run_until_block` by a
            // `VmTask`. (Concurrent programs currently route to the tree-walker
            // before the VM, so these are exercised via the dedicated VM-scheduler
            // entry; the default routing is unchanged.)
            Stmt::CreatePipe {
                var,
                element_type,
                capacity,
            } => {
                let dst = self.let_reg(*var)?;
                let cap = capacity.map(|c| c as i32).unwrap_or(-1);
                let elem = crate::vm::instruction::ChanElem::from_type_name(
                    self.interner.resolve(*element_type),
                );
                self.emit(Op::ChanNew { dst, cap, elem });
                Ok(())
            }
            Stmt::SendPipe { value, pipe } => {
                let val = self.compile_expr(value)?;
                let chan = self.compile_expr(pipe)?;
                self.emit(Op::ChanSend { chan, val });
                Ok(())
            }
            Stmt::ReceivePipe { var, pipe } => {
                let chan = self.compile_expr(pipe)?;
                let dst = self.let_reg(*var)?;
                self.emit(Op::ChanRecv { dst, chan });
                Ok(())
            }
            Stmt::TrySendPipe {
                value,
                pipe,
                result,
            } => {
                let val = self.compile_expr(value)?;
                let chan = self.compile_expr(pipe)?;
                let dst = match result {
                    Some(sym) => self.let_reg(*sym)?,
                    None => self.alloc_reg()?,
                };
                self.emit(Op::ChanTrySend { dst, chan, val });
                Ok(())
            }
            Stmt::TryReceivePipe { var, pipe } => {
                let chan = self.compile_expr(pipe)?;
                let dst = self.let_reg(*var)?;
                self.emit(Op::ChanTryRecv { dst, chan });
                Ok(())
            }
            Stmt::LaunchTask { function, args } => self.compile_spawn(*function, args, None),
            Stmt::LaunchTaskWithHandle {
                handle,
                function,
                args,
            } => self.compile_spawn(*function, args, Some(*handle)),
            Stmt::StopTask { handle } => {
                let h = self.compile_expr(handle)?;
                self.emit(Op::TaskAbort { handle: h });
                Ok(())
            }
            Stmt::CloseChannel { channel } => {
                let chan = self.compile_expr(channel)?;
                self.emit(Op::ChanClose { chan });
                Ok(())
            }
            Stmt::AwaitTask { handle, result } => {
                let h = self.compile_expr(handle)?;
                // `Op::TaskAwait` always writes a result register; bind it to the named
                // variable when `… into r` is given, otherwise a scratch register the
                // result is discarded from.
                let dst = match result {
                    Some(r) => self.let_reg(*r)?,
                    None => self.alloc_reg()?,
                };
                self.emit(Op::TaskAwait { dst, handle: h });
                Ok(())
            }
            Stmt::Select { branches } => {
                use crate::ast::stmt::SelectBranch;
                // Register each arm. A recv arm allocates and binds its var
                // register up front, so the winning branch body reads the value
                // `deliver_select` writes there.
                for branch in branches.iter() {
                    match branch {
                        SelectBranch::Receive { var, pipe, .. } => {
                            let chan = self.compile_expr(pipe)?;
                            let var_reg = self.let_reg(*var)?;
                            self.emit(Op::SelectArmRecv { chan, var: var_reg });
                        }
                        SelectBranch::Timeout { milliseconds, .. } => {
                            let ticks = self.compile_expr(milliseconds)?;
                            self.emit(Op::SelectArmTimeout { ticks });
                        }
                    }
                }
                let dst_arm = self.alloc_reg()?;
                self.emit(Op::SelectWait { dst_arm });
                // Dispatch on the winning arm index: run that branch's body.
                let mut end_jumps = Vec::new();
                for (i, branch) in branches.iter().enumerate() {
                    let lit = self.alloc_reg()?;
                    let idx = self.add_const(Constant::Int(i as i64))?;
                    self.emit(Op::LoadConst { dst: lit, idx });
                    let cmp = self.alloc_reg()?;
                    self.emit(Op::Eq {
                        dst: cmp,
                        lhs: dst_arm,
                        rhs: lit,
                    });
                    let jskip = self.emit_placeholder_jump_if_false(cmp);
                    let body = match branch {
                        SelectBranch::Receive { body, .. } | SelectBranch::Timeout { body, .. } => {
                            *body
                        }
                    };
                    let mark = self.enter_block();
                    for st in body {
                        self.compile_stmt(st)?;
                    }
                    self.exit_block(mark);
                    let jend = self.emit_placeholder_jump();
                    end_jumps.push(jend);
                    self.patch_jump_target(jskip, self.current_pc())?;
                }
                for j in end_jumps {
                    self.patch_jump_target(j, self.current_pc())?;
                }
                Ok(())
            }
            Stmt::Escape { .. } => self.emit_fail(crate::semantics::escape::REFUSAL),
            Stmt::FunctionDef { .. } => {
                // Nested function definitions are unreachable from the parser
                // (## To headers are top level); statically registering one
                // here cannot model the tree-walker's conditional runtime
                // definition, so it is a compile error.
                Err("vm: nested function definitions are not supported"
                    .to_string()
                    .into())
            }
        }
    }

    /// Inside a `Reversibly:` block, try to lower a `Set x to <value>` where `x` is the local in
    /// register `dst` to the zero-garbage reversible ISA. Recognizes exactly the IN-PLACE ABELIAN
    /// forms (`x` on the left, `x` never re-observed on the right):
    /// `x + e` → `RevAdd`, `x - e` → `RevSub`, `x xor e` → `RevXor`, and `-x` (`0 - x`) → `RevNeg`.
    /// Returns `Ok(true)` if it emitted a reversible op, `Ok(false)` to fall through to the plain
    /// lowering (an unrecognized form). The FORWARD semantics are identical to the plain path, so
    /// program output is unchanged. The delta rides a FRESH scratch register, so the ISA
    /// `place != delta` well-formedness invariant holds by construction.
    fn try_emit_reversible_set(
        &mut self,
        target: Symbol,
        dst: Reg,
        value: &Expr,
    ) -> Result<bool, Refusal> {
        // `-x`, which the parser desugars to `0 - x`: `Set x to -x` → in-place `RevNeg`.
        if let Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left,
            right,
        } = value
        {
            if matches!(left, Expr::Literal(Literal::Number(0)))
                && matches!(right, Expr::Identifier(s) if *s == target)
            {
                self.emit(Op::RevNeg { place: dst });
                return Ok(true);
            }
        }
        // `x ⊕ e` where `⊕` is an abelian in-place operator, `x` leads, and `e` provably never
        // observes `x` (so the delta read is untouched by the in-place write — a genuine
        // group translation, invertible without a tape).
        if let Expr::BinaryOp { op, left, right } = value {
            let rev = match op {
                BinaryOpKind::Add => Some(RevOp::Add),
                BinaryOpKind::Subtract => Some(RevOp::Sub),
                BinaryOpKind::BitXor => Some(RevOp::Xor),
                _ => None,
            };
            if let Some(kind) = rev {
                if matches!(left, Expr::Identifier(s) if *s == target) && expr_avoids(right, target)
                {
                    let delta = self.alloc_reg()?;
                    self.compile_expr_into(right, delta)?;
                    self.emit(match kind {
                        RevOp::Add => Op::RevAdd { place: dst, delta },
                        RevOp::Sub => Op::RevSub { place: dst, delta },
                        RevOp::Xor => Op::RevXor { place: dst, delta },
                    });
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn current_pc(&self) -> usize {
        self.code.len()
    }

    fn emit_placeholder_jump(&mut self) -> usize {
        let idx = self.code.len();
        self.emit(Op::Jump { target: usize::MAX });
        idx
    }

    fn emit_placeholder_jump_if_false(&mut self, cond: Reg) -> usize {
        let idx = self.code.len();
        self.emit(Op::JumpIfFalse {
            cond,
            target: usize::MAX,
        });
        idx
    }

    fn patch_jump_target(&mut self, idx: usize, target: usize) -> Result<(), Refusal> {
        Ok(patch_jump(&mut self.code, idx, target)?)
    }

    /// Compile `e`, returning the register that holds its value. An unbound
    /// identifier becomes a runtime failure at this point in the instruction
    /// stream (never a compile error — dead branches stay free).
    fn compile_expr(&mut self, e: &Expr) -> Result<Reg, Refusal> {
        match e {
            Expr::Identifier(sym) => match self.interner.resolve(*sym) {
                "today" | "now" => {
                    let scratch = self.alloc_reg()?;
                    self.compile_expr_into(e, scratch)?;
                    Ok(scratch)
                }
                _ => match self.resolve_name(*sym) {
                    NameRef::Local(r) => Ok(r),
                    _ => {
                        let scratch = self.alloc_reg()?;
                        self.emit_read(*sym, scratch)?;
                        Ok(scratch)
                    }
                },
            },
            _ => {
                let dst = self.alloc_reg()?;
                self.compile_expr_into(e, dst)?;
                Ok(dst)
            }
        }
    }

    /// Preserve an already evaluated local value across a later function call.
    /// A direct local read normally reuses its binding's register; a mutable
    /// argument can replace that register before the surrounding operation runs.
    /// Pure expressions and values already in temporary registers need no copy.
    fn compile_expr_before(&mut self, e: &Expr, later: &[&Expr]) -> Result<Reg, Refusal> {
        let src = self.compile_expr(e)?;
        if !matches!(e, Expr::Identifier(sym)
            if matches!(self.resolve_name(*sym), NameRef::Local(reg) if reg == src))
        {
            return Ok(src);
        }
        let mut pending = later.to_vec();
        while let Some(next) = pending.pop() {
            match next {
                Expr::Call { .. } | Expr::CallExpr { .. } => {
                    let dst = self.alloc_reg()?;
                    self.emit(Op::Move { dst, src });
                    return Ok(dst);
                }
                Expr::Closure { .. } => {}
                _ => pending.extend(logicaffeine_language::ast_depth::expr_children(next)),
            }
        }
        Ok(src)
    }

    /// Compile `e`, placing its value into `dst`. Depth-guarded so adversarially
    /// nested expressions fail with an error instead of exhausting the native
    /// stack.
    fn compile_expr_into(&mut self, e: &Expr, dst: Reg) -> Result<(), Refusal> {
        self.expr_depth += 1;
        if self.expr_depth > MAX_EXPR_DEPTH {
            self.expr_depth -= 1;
            return Err("vm: expression too deeply nested".to_string().into());
        }
        let result = self.compile_expr_into_inner(e, dst);
        self.expr_depth -= 1;
        result
    }

    fn compile_expr_into_inner(&mut self, e: &Expr, dst: Reg) -> Result<(), Refusal> {
        match e {
            Expr::Literal(Literal::Text(sym)) => {
                let idx =
                    self.add_const(Constant::Text(self.interner.resolve(*sym).to_string()))?;
                self.emit(Op::LoadConst { dst, idx });
                Ok(())
            }
            Expr::Literal(lit) => {
                let idx = self.add_const(literal_const(lit)?)?;
                self.emit(Op::LoadConst { dst, idx });
                Ok(())
            }
            Expr::Identifier(sym) => {
                // Temporal builtins: the NAME wins even when shadowed
                // (tree-walker checks before the env lookup).
                match self.interner.resolve(*sym) {
                    "today" => {
                        self.emit(Op::LoadToday { dst });
                        Ok(())
                    }
                    "now" => {
                        self.emit(Op::LoadNow { dst });
                        Ok(())
                    }
                    _ => self.emit_read(*sym, dst),
                }
            }
            Expr::BinaryOp {
                op: BinaryOpKind::And,
                left,
                right,
            } => self.compile_short_circuit(true, left, right, dst),
            Expr::BinaryOp {
                op: BinaryOpKind::Or,
                left,
                right,
            } => self.compile_short_circuit(false, left, right, dst),
            Expr::BinaryOp { op, left, right } => {
                // Int `x / 2^k` → single `DivPow2` op (the JIT lowers it to the
                // 1-op shift stencil): replaces idiv, fires for loop-invariant
                // divisors, no scratch-register pressure.
                if self.strength_reduce && matches!(op, BinaryOpKind::Divide) {
                    if let Some(k) = self.divpow2_shift(left, right) {
                        let lhs = self.compile_expr(left)?;
                        self.emit(Op::DivPow2 {
                            dst,
                            lhs,
                            k: k as u8,
                        });
                        return Ok(());
                    }
                    // Proven-non-negative Int `x / c` (c a constant non-pow2) →
                    // the magic reciprocal (mul-high + shift) instead of idiv.
                    if let Some((magic, more, divisor)) = self.magic_div_const(left, right) {
                        let lhs = self.compile_expr(left)?;
                        self.emit(Op::MagicDivU {
                            dst,
                            lhs,
                            magic,
                            more,
                            mul_back: 0,
                            divisor,
                            wrap: false,
                        });
                        return Ok(());
                    }
                }
                // Proven-non-negative Int `x % 2^k` → `x & (2^k - 1)`, emitted
                // as the register-form `BitAnd`: replaces idiv with a 1-cycle
                // AND, which the JIT lowers to a `BitAnd` stencil. Gated on
                // Oracle-proven non-negativity — unsound for negative dividends.
                if self.strength_reduce && matches!(op, BinaryOpKind::Modulo) {
                    if let Some(mask) = self.modpow2_mask(left, right) {
                        let lhs = self.compile_expr(left)?;
                        let rhs = self.alloc_reg()?;
                        let idx = self.add_const(Constant::Int(mask))?;
                        self.emit(Op::LoadConst { dst: rhs, idx });
                        self.emit(Op::BitAnd { dst, lhs, rhs });
                        return Ok(());
                    }
                    // Opt::ModWrap: an `x` the Oracle bounds below `c` is its own remainder; one below
                    // `2c` keeps the magic remainder below, marked `wrap` (`crate::optimize::modwrap`).
                    let wrap = self.mod_wrap_const(left, right);
                    if wrap == Some(crate::optimize::modwrap::ModWrap::Identity) {
                        crate::optimize::mark_fired(crate::optimization::Opt::ModWrap);
                        return self.compile_expr_into(left, dst);
                    }
                    // Proven-non-negative Int `x % c` (c a constant non-pow2) →
                    // the magic reciprocal: `x - (x/c)*c` via mul-high + shift,
                    // instead of idiv. `mul_back == c` selects the remainder.
                    if let Expr::Literal(Literal::Number(c)) = right {
                        if let Some((magic, more, divisor)) = self.magic_div_const(left, right) {
                            let wrap = wrap == Some(crate::optimize::modwrap::ModWrap::Once);
                            if wrap {
                                crate::optimize::mark_fired(crate::optimization::Opt::ModWrap);
                            }
                            let lhs = self.compile_expr(left)?;
                            self.emit(Op::MagicDivU {
                                dst,
                                lhs,
                                magic,
                                more,
                                mul_back: *c,
                                divisor,
                                wrap,
                            });
                            return Ok(());
                        }
                    }
                }
                // ★ ABSENCE IS ONE THING (`semantics::compare`, R2): `x is none` and `x is nothing`
                // are equal on every tier, so an equality against `none` loads the one absence
                // constant every backend tests a value against, never an `Option` it would have to
                // tell apart from a present payload.
                if matches!(op, BinaryOpKind::Eq | BinaryOpKind::NotEq)
                    && (matches!(left, Expr::OptionNone) || matches!(right, Expr::OptionNone))
                {
                    let lhs = self.compile_equality_operand(left)?;
                    let rhs = self.compile_equality_operand(right)?;
                    self.emit(binop_op(*op, dst, lhs, rhs)?);
                    return Ok(());
                }
                let lhs = self.compile_expr_before(left, &[right])?;
                let rhs = self.compile_expr(right)?;
                self.emit(binop_op(*op, dst, lhs, rhs)?);
                Ok(())
            }
            Expr::Not { operand } => {
                let src = self.compile_expr(operand)?;
                self.emit(Op::Not { dst, src });
                Ok(())
            }
            Expr::Call { function, args } => self.compile_call(*function, args, dst),
            Expr::List(items) => {
                let count = u16::try_from(items.len())
                    .map_err(|_| "vm: list literal too long (max 65535 elements)".to_string())?;
                let start = self.reserve_regs(count)?;
                for (i, item) in items.iter().enumerate() {
                    self.compile_expr_into(item, start + i as Reg)?;
                }
                self.emit(Op::NewList { dst, start, count });
                Ok(())
            }
            Expr::New {
                type_name,
                init_fields,
                type_args,
            } => {
                self.compile_new(*type_name, init_fields, dst)?;
                // An EMPTY collection still says what it holds, wherever it is written — bound, pushed
                // or handed to a constructor as a field's first value — and nothing else ever will.
                if let Some(bt) = self.declared_collection(*type_name, type_args) {
                    self.note_declared(bt);
                }
                Ok(())
            }
            Expr::FieldAccess { object, field } => {
                let obj = self.compile_expr(object)?;
                let fname = self.interner.resolve(*field).to_string();
                let fidx = self.add_const(Constant::Text(fname))?;
                self.emit(Op::GetField {
                    dst,
                    obj,
                    field: fidx,
                });
                Ok(())
            }
            Expr::NewVariant {
                enum_name,
                variant,
                fields,
            } => {
                let tname = self.interner.resolve(*enum_name).to_string();
                let cname = self.interner.resolve(*variant).to_string();
                let tidx = self.add_const(Constant::Text(tname))?;
                let cidx = self.add_const(Constant::Text(cname))?;
                // Seat each field at its DECLARED position, not its WRITE position — `Op::
                // NewInductive` reads `args_start..args_start+count` back purely positionally, and
                // `execute_inspect`'s bind arm reads `ind.args[i]` as "the i-th DECLARED field."
                // `a new Circle with label "x" and radius 10` must land `radius` before `label`
                // when the type declares `radius` first, regardless of `with`-clause order.
                // Values are still evaluated in WRITE order (left to right as authored) — only the
                // register each one lands in is reordered. A field the `with` clause omits
                // defaults to `Nothing`, mirroring `struct_defs`' default-fill for structs. Keyed
                // by `(enum_name, variant)` — `Expr::NewVariant` already names its own enum, so two
                // enums sharing a variant name never collide on one shared entry.
                match self.variant_fields.get(&(*enum_name, *variant)).cloned() {
                    Some(names) => {
                        let count = u16::try_from(names.len())
                            .map_err(|_| "vm: too many variant fields".to_string())?;
                        let args_start = self.reserve_regs(count)?;
                        let mut filled = vec![false; names.len()];
                        for (field_name, field_expr) in fields.iter() {
                            match names.iter().position(|n| n == field_name) {
                                Some(pos) => {
                                    self.compile_expr_into(field_expr, args_start + pos as Reg)?;
                                    filled[pos] = true;
                                }
                                None => {
                                    return Err(format!(
                                        "vm: `{}` has no field `{}`",
                                        self.interner.resolve(*variant),
                                        self.interner.resolve(*field_name)
                                    )
                                    .into());
                                }
                            }
                        }
                        for (pos, done) in filled.iter().enumerate() {
                            if !done {
                                let reg = args_start + pos as Reg;
                                let idx = self.add_const(Constant::Nothing)?;
                                self.emit(Op::LoadConst { dst: reg, idx });
                            }
                        }
                        self.emit(Op::NewInductive {
                            dst,
                            type_name: tidx,
                            ctor: cidx,
                            args_start,
                            count,
                        });
                    }
                    None => {
                        let count = u16::try_from(fields.len())
                            .map_err(|_| "vm: too many variant fields".to_string())?;
                        let args_start = self.reserve_regs(count)?;
                        for (i, (_, field_expr)) in fields.iter().enumerate() {
                            self.compile_expr_into(field_expr, args_start + i as Reg)?;
                        }
                        self.emit(Op::NewInductive {
                            dst,
                            type_name: tidx,
                            ctor: cidx,
                            args_start,
                            count,
                        });
                    }
                }
                Ok(())
            }
            Expr::Range { start, end } => {
                let s = self.compile_expr_before(start, &[end])?;
                let e = self.compile_expr(end)?;
                self.emit(Op::NewRange {
                    dst,
                    start: s,
                    end: e,
                });
                Ok(())
            }
            Expr::Length { collection } => {
                let c = self.compile_expr(collection)?;
                self.emit(Op::Length { dst, collection: c });
                Ok(())
            }
            Expr::Index { collection, index } => {
                // Bounds-check elimination: if the Oracle proved this index
                // in `[1, length]` (range analysis, M9), emit the unchecked
                // form — the JIT drops the bounds branch. Checked otherwise.
                let proven = self.index_in_bounds(collection, index);
                let c = self.compile_expr_before(collection, &[index])?;
                let i = self.compile_expr(index)?;
                if proven {
                    self.emit(Op::IndexUnchecked {
                        dst,
                        collection: c,
                        index: i,
                    });
                } else {
                    self.emit(Op::Index {
                        dst,
                        collection: c,
                        index: i,
                    });
                }
                Ok(())
            }
            Expr::Contains { collection, value } => {
                let c = self.compile_expr_before(collection, &[value])?;
                let v = self.compile_expr(value)?;
                self.emit(Op::Contains {
                    dst,
                    collection: c,
                    value: v,
                });
                Ok(())
            }
            Expr::InterpolatedString(parts) => self.compile_interpolation(parts, dst),
            Expr::Slice {
                collection,
                start,
                end,
            } => {
                let c = self.compile_expr_before(collection, &[start, end])?;
                let st = self.compile_expr_before(start, &[end])?;
                let en = self.compile_expr(end)?;
                self.emit(Op::SliceOp {
                    dst,
                    collection: c,
                    start: st,
                    end: en,
                });
                Ok(())
            }
            Expr::Copy { expr } => {
                let src = self.compile_expr(expr)?;
                self.emit(Op::DeepClone { dst, src });
                Ok(())
            }
            Expr::Give { value } => self.compile_expr_into(value, dst),
            Expr::Tuple(items) => {
                let count = u16::try_from(items.len())
                    .map_err(|_| "vm: tuple literal too long".to_string())?;
                let start = self.reserve_regs(count)?;
                for (i, item) in items.iter().enumerate() {
                    self.compile_expr_into(item, start + i as Reg)?;
                }
                self.emit(Op::NewTuple { dst, start, count });
                Ok(())
            }
            Expr::Union { left, right } => {
                let l = self.compile_expr_before(left, &[right])?;
                let r = self.compile_expr(right)?;
                self.emit(Op::UnionOp {
                    dst,
                    lhs: l,
                    rhs: r,
                });
                Ok(())
            }
            Expr::Intersection { left, right } => {
                let l = self.compile_expr_before(left, &[right])?;
                let r = self.compile_expr(right)?;
                self.emit(Op::IntersectOp {
                    dst,
                    lhs: l,
                    rhs: r,
                });
                Ok(())
            }
            // ★ AN OPTION CARRIES ITS OWN TAG. This was the root of the whole defect: `some x`
            // compiled to `compile_expr_into(value, dst)` — the `Some` DISCARDED — and `none` to
            // the `Nothing` sentinel, so presence was inferred from bits rather than carried. Every
            // backend downstream of the bytecode inherited that loss: the VM papered over it with
            // runtime tags, and the tiers that erase to machine words could not tell `some 0` from
            // `none` at all.
            //
            // It lowers as an ordinary two-constructor inductive — the same `Op::NewInductive` a
            // `Result` uses, whose tag every backend already derives from the constructor NAME. So
            // this needs no new op and no ISA change; `TestArm`/`BindArm` already do the right
            // thing for it on all eight backends.
            Expr::OptionSome { value } => self.compile_option_ctor("Some", Some(value), dst),
            Expr::OptionNone => self.compile_option_ctor("None", None, dst),
            Expr::WithCapacity { value, capacity } => {
                if let Expr::New {
                    type_name,
                    init_fields,
                    ..
                } = value
                {
                    if init_fields.is_empty() {
                        match self.interner.resolve(*type_name) {
                            "Seq" | "List" | "Vec" => {
                                let capacity = self.compile_expr(capacity)?;
                                self.emit(Op::NewEmptyListWithCapacity { dst, capacity });
                                return Ok(());
                            }
                            "Map" | "HashMap" => {
                                let capacity = self.compile_expr(capacity)?;
                                self.emit(Op::NewEmptyMapWithCapacity { dst, capacity });
                                return Ok(());
                            }
                            _ => {}
                        }
                    }
                }
                self.compile_expr_into(value, dst)
            }
            Expr::ManifestOf { .. } => {
                self.emit(Op::NewEmptyList { dst });
                Ok(())
            }
            Expr::ChunkAt { .. } => {
                let idx = self.add_const(Constant::Nothing)?;
                self.emit(Op::LoadConst { dst, idx });
                Ok(())
            }
            Expr::Escape { .. } => {
                let idx = self.add_const(Constant::Text(
                    crate::semantics::escape::REFUSAL.to_string(),
                ))?;
                self.emit(Op::FailWith { msg: idx });
                Ok(())
            }
            Expr::Closure { params, body, .. } => self.compile_closure(params, body, dst),
            Expr::CallExpr { callee, args } => {
                let c = self.compile_expr_before(callee, args)?;
                let arg_count =
                    u16::try_from(args.len()).map_err(|_| "vm: too many arguments".to_string())?;
                let args_start = self.reserve_regs(arg_count)?;
                for (i, arg) in args.iter().enumerate() {
                    self.compile_expr_into(arg, args_start + i as Reg)?;
                }
                self.emit(Op::CallValue {
                    dst,
                    callee: c,
                    args_start,
                    arg_count,
                    name_for_err: u32::MAX,
                });
                Ok(())
            }
            // A page query asks about a LIVE DOCUMENT, which this tier has none of — the driving
            // verbs reach the page through a synthesized dispatcher instead. That refusal is
            // correct; falling into the catch-all below for it was not, because "unsupported
            // expression" names neither the sentence nor anything to write instead.
            Expr::Dom(q) => Err(format!(
                "vm: `the {} of` asks about the page, which this tier does not have. \
                 A page query runs on the interpreter (a `## Test` block runs there). \
                 For a collection's size on every tier, write `length of`.",
                q.noun()
            )
            .into()),
            _ => Err("vm: unsupported expression".to_string().into()),
        }
    }

    /// `Repeat for v from a to b` over Int bounds (`Fact::IntRange`): the bounds are evaluated once, when the loop
    /// starts, and `v` takes each Int from `a` up to `b` (none when `a` is past `b`), whatever the body does to `v`. A
    /// counter carries the position, so no list of the range's Ints is ever built, and the loop is the counted loop the
    /// native tier compiles.
    fn compile_counted_range(
        &mut self,
        var: Symbol,
        start: &Expr,
        end: &Expr,
        body: &[Stmt],
    ) -> Result<(), Refusal> {
        // The counter, the last value and the step outlive the body: they are reserved before it, below every register
        // its statements reclaim. Copies, so the body's writes to a bound's variables cannot move them.
        let at = self.alloc_reg()?;
        let lo = self.compile_expr(start)?;
        self.emit(Op::Move { dst: at, src: lo });
        let last = self.alloc_reg()?;
        let hi = self.compile_expr(end)?;
        self.emit(Op::Move { dst: last, src: hi });
        let one = self.add_const(Constant::Int(1))?;
        let step = self.alloc_reg()?;
        self.emit(Op::LoadConst {
            dst: step,
            idx: one,
        });
        let outer_mark = self.enter_block();
        let item = self.let_reg(var)?;
        let loop_start = self.current_pc();
        self.loop_stack.push(loop_start);
        let more = self.alloc_reg()?;
        self.emit(Op::LtEq {
            dst: more,
            lhs: at,
            rhs: last,
        });
        let jexit = self.emit_placeholder_jump_if_false(more);
        self.emit(Op::Move { dst: item, src: at });
        self.flow_stack.push(FlowCtx::Loop {
            breaks: Vec::new(),
            is_repeat: false,
        });
        let body_mark = self.enter_block();
        for st in body {
            self.compile_stmt(st)?;
        }
        self.exit_block(body_mark);
        // The last value has already run. Leaving before its increment avoids constructing
        // a value outside the range, including MAX + 1 in machine-word backends.
        self.emit(Op::Lt {
            dst: more,
            lhs: at,
            rhs: last,
        });
        let jlast = self.emit_placeholder_jump_if_false(more);
        self.emit(Op::Add {
            dst: at,
            lhs: at,
            rhs: step,
        });
        self.loop_stack.pop();
        self.emit(Op::Jump { target: loop_start });
        let exit_pc = self.current_pc();
        self.patch_jump_target(jexit, exit_pc)?;
        self.patch_jump_target(jlast, exit_pc)?;
        if let Some(FlowCtx::Loop { breaks, .. }) = self.flow_stack.pop() {
            for j in breaks {
                self.patch_jump_target(j, exit_pc)?;
            }
        }
        self.exit_block(outer_mark);
        Ok(())
    }

    /// A `Repeat` over a sequence nothing in the loop can change (`Fact::LentIteration`): the sequence is its own
    /// snapshot, so the loop walks it by position (its length once, a counter from 1, each item read in bounds) with no
    /// iterator. Every bytecode backend then sees a plain counted loop, the shape the native tier compiles.
    fn compile_lent_walk(&mut self, var: Symbol, seq: &Expr, body: &[Stmt]) -> Result<(), Refusal> {
        crate::optimize::mark_fired(crate::optimization::Opt::Borrow);
        // The sequence, its length, the position and the step outlive the body: they are reserved before it, below
        // every register its statements reclaim.
        let xs = self.compile_expr(seq)?;
        let n = self.alloc_reg()?;
        self.emit(Op::Length {
            dst: n,
            collection: xs,
        });
        let one = self.add_const(Constant::Int(1))?;
        let i = self.alloc_reg()?;
        self.emit(Op::LoadConst { dst: i, idx: one });
        let step = self.alloc_reg()?;
        self.emit(Op::LoadConst {
            dst: step,
            idx: one,
        });
        // The loop variable lives in a scope spanning the whole loop, as the snapshot walk's does.
        let outer_mark = self.enter_block();
        let item = self.let_reg(var)?;
        let loop_start = self.current_pc();
        self.loop_stack.push(loop_start);
        let more = self.alloc_reg()?;
        self.emit(Op::LtEq {
            dst: more,
            lhs: i,
            rhs: n,
        });
        let jexit = self.emit_placeholder_jump_if_false(more);
        self.emit(Op::IndexUnchecked {
            dst: item,
            collection: xs,
            index: i,
        });
        self.flow_stack.push(FlowCtx::Loop {
            breaks: Vec::new(),
            is_repeat: false,
        });
        let body_mark = self.enter_block();
        for st in body {
            self.compile_stmt(st)?;
        }
        self.exit_block(body_mark);
        self.emit(Op::Add {
            dst: i,
            lhs: i,
            rhs: step,
        });
        self.loop_stack.pop();
        self.emit(Op::Jump { target: loop_start });
        let exit_pc = self.current_pc();
        self.patch_jump_target(jexit, exit_pc)?;
        if let Some(FlowCtx::Loop { breaks, .. }) = self.flow_stack.pop() {
            for j in breaks {
                self.patch_jump_target(j, exit_pc)?;
            }
        }
        self.exit_block(outer_mark);
        Ok(())
    }

    /// Compile a `Repeat` loop (extracted: keeps `compile_stmt`'s recursion
    /// frame small in debug builds).
    fn compile_repeat(
        &mut self,
        repeat: &Stmt,
        pattern: &crate::ast::stmt::Pattern,
        iterable: &Expr,
        body: &[Stmt],
    ) -> Result<(), Refusal> {
        use crate::ast::stmt::Pattern;

        if let Pattern::Identifier(var) = pattern {
            if let Expr::Range { start, end } = iterable {
                if crate::optimize::facts::holds_stmt(
                    crate::optimize::facts::Fact::IntRange,
                    repeat,
                ) {
                    return self.compile_counted_range(*var, start, end, body);
                }
            }
            if crate::optimize::facts::holds_stmt(
                crate::optimize::facts::Fact::LentIteration,
                repeat,
            ) {
                return self.compile_lent_walk(*var, iterable, body);
            }
        }
        let it_reg = self.compile_expr(iterable)?;
        self.emit(Op::IterPrepare { iterable: it_reg });

        // The loop variable lives in a scope spanning the whole loop
        // (the tree-walker pushes ONE scope outside the iteration).
        let outer_mark = self.enter_block();
        let loop_start;
        let next_idx;
        match pattern {
            Pattern::Identifier(sym) => {
                let dst = self.let_reg(*sym)?;
                loop_start = self.current_pc();
                next_idx = self.code.len();
                self.emit(Op::IterNext {
                    dst,
                    exit: usize::MAX,
                });
            }
            Pattern::Tuple(syms) => {
                let tmp = self.alloc_reg()?;
                let count = u16::try_from(syms.len())
                    .map_err(|_| "vm: tuple pattern too long".to_string())?;
                let start = self.reserve_regs(count)?;
                for (i, sym) in syms.iter().enumerate() {
                    self.scopes
                        .last_mut()
                        .unwrap()
                        .insert(*sym, start + i as Reg);
                    self.mark_named(start + i as Reg);
                }
                loop_start = self.current_pc();
                next_idx = self.code.len();
                self.emit(Op::IterNext {
                    dst: tmp,
                    exit: usize::MAX,
                });
                self.emit(Op::DestructureTuple {
                    src: tmp,
                    start,
                    count,
                });
            }
        }

        self.flow_stack.push(FlowCtx::Loop {
            breaks: Vec::new(),
            is_repeat: true,
        });
        let body_mark = self.enter_block();
        for st in body {
            self.compile_stmt(st)?;
        }
        self.exit_block(body_mark);
        self.emit(Op::Jump { target: loop_start });

        let exit_pc = self.current_pc();
        self.patch_jump_target(next_idx, exit_pc)?;
        if let Some(FlowCtx::Loop { breaks, .. }) = self.flow_stack.pop() {
            for j in breaks {
                self.patch_jump_target(j, exit_pc)?;
            }
        }
        self.emit(Op::IterPop);
        self.exit_block(outer_mark);
        Ok(())
    }

    /// Compile a `Zone`: the name is bound to Nothing for the body's duration,
    /// and the zone SWALLOWS Break/Return escaping the body.
    fn compile_zone(&mut self, name: Symbol, body: &[Stmt]) -> Result<(), Refusal> {
        let outer_mark = self.enter_block();
        let name_reg = self.let_reg(name)?;
        let idx = self.add_const(Constant::Nothing)?;
        self.emit(Op::LoadConst { dst: name_reg, idx });
        self.flow_stack.push(FlowCtx::Zone { exits: Vec::new() });
        let body_mark = self.enter_block();
        for st in body {
            self.compile_stmt(st)?;
        }
        self.exit_block(body_mark);
        let end_pc = self.current_pc();
        if let Some(FlowCtx::Zone { exits }) = self.flow_stack.pop() {
            for j in exits {
                self.patch_jump_target(j, end_pc)?;
            }
        }
        self.exit_block(outer_mark);
        Ok(())
    }

    /// `compile_inspect`'s non-exhaustive fallthrough for a STATICALLY known enum (user-declared
    /// or a built-in `Option`/`Result`, matched by name via either `variant_enum` or arm-text —
    /// see the two call sites): test each of `uncovered`'s constructors against the scrutinee `t`
    /// in turn and fail with its OWN compile-time-constant message, so the wording is
    /// byte-identical to the tree-walker's `inspect_unhandled` by construction. An EMPTY
    /// `uncovered` (the arms already name every constructor the enum has) emits NOTHING — no
    /// `TestArm`/`FailWith` chain, no defensive final one either: the fallthrough is provably
    /// unreachable for a sound program, so there is nothing left to test or name.
    fn emit_inspect_fallthrough(
        &mut self,
        t: Reg,
        enum_name: &str,
        uncovered: &[String],
    ) -> Result<(), Refusal> {
        if uncovered.is_empty() {
            return Ok(());
        }
        for vname in uncovered {
            let vidx = self.add_const(Constant::Text(vname.clone()))?;
            let flag = self.alloc_reg()?;
            self.emit(Op::TestArm {
                dst: flag,
                target: t,
                variant: vidx,
            });
            let jnext = self.emit_placeholder_jump_if_false(flag);
            let msg = crate::semantics::builtins::inspect_unhandled_message(enum_name, vname);
            self.emit_fail(&msg)?;
            self.patch_jump_target(jnext, self.current_pc())?;
        }
        // Every constructor of the enum this Inspect matches has now been tested (the covered
        // ones by an ARM, the rest just above) — a value that fails ALL of them is not a value of
        // this enum at all, which a sound program cannot produce. Defensive, not reachable in
        // practice: names the enum, not a specific variant, since none of the KNOWN ones matched.
        self.emit_fail(&format!(
            "Inspect has no arm for the value (a `{enum_name}`, but not one of its \
             known constructors) and no Otherwise (matches must be exhaustive)"
        ))
    }

    /// Compile `Inspect` as a chain of TestArm/JumpIfFalse arms (the
    /// tree-walker's linear arm scan).
    fn compile_inspect(
        &mut self,
        target: &Expr,
        arms: &[crate::ast::stmt::MatchArm],
    ) -> Result<(), Refusal> {
        let t = self.compile_expr(target)?;
        let mut end_jumps: Vec<usize> = Vec::new();
        let mut has_otherwise = false;
        for arm in arms.iter() {
            match arm.variant {
                None => {
                    // Otherwise: unconditional, and no arm after it runs.
                    has_otherwise = true;
                    let mark = self.enter_block();
                    for st in arm.body {
                        self.compile_stmt(st)?;
                    }
                    self.exit_block(mark);
                    break;
                }
                Some(variant) => {
                    let vname = self.interner.resolve(variant).to_string();
                    let vidx = self.add_const(Constant::Text(vname.clone()))?;
                    let flag = self.alloc_reg()?;
                    self.emit(Op::TestArm {
                        dst: flag,
                        target: t,
                        variant: vidx,
                    });
                    let jnext = self.emit_placeholder_jump_if_false(flag);

                    let mark = self.enter_block();
                    // Struct arms bind by field NAME, inductive arms by
                    // POSITION; the flavor is only known at runtime, so
                    // BindArm carries both and dispatches there.
                    for (i, (field_name, binding_name)) in arm.bindings.iter().enumerate() {
                        let dst = self.let_reg(*binding_name)?;
                        // A `Some` arm binds the whole matched value, not a field of it — say so
                        // with a marker rather than leaving `BindArm` to guess from the shape.
                        let fname = if vname == "Some" {
                            crate::vm::SOME_BINDING_MARKER.to_string()
                        } else {
                            self.interner.resolve(*field_name).to_string()
                        };
                        let fidx = self.add_const(Constant::Text(fname))?;
                        self.emit(Op::BindArm {
                            dst,
                            target: t,
                            field: fidx,
                            index: u16::try_from(i)
                                .map_err(|_| "vm: too many arm bindings".to_string())?,
                        });
                    }
                    for st in arm.body {
                        self.compile_stmt(st)?;
                    }
                    self.exit_block(mark);
                    let j = self.emit_placeholder_jump();
                    end_jumps.push(j);
                    self.patch_jump_target(jnext, self.current_pc())?;
                }
            }
        }
        // An unmatched scrutinee (no arm's TestArm fired) falls to HERE. Without an Otherwise it is
        // a non-exhaustive match — LOUD, never a silent no-op. Matched arms jump PAST this via
        // `end_jumps`. Gauntlet finding G5: name the ACTUAL variant and type, with no runtime tag
        // extraction and no new `Op`/`BuiltinId` (LANES.md rule 16) — the compiler already knows,
        // statically, every OTHER constructor the matched enum has (`enum_variants`/`variant_enum`,
        // built from the discovery-pass type registry): test each UNCOVERED one in turn and fail
        // with ITS OWN compile-time-constant message, built by the SAME Rust function the
        // tree-walker calls (`semantics::builtins::inspect_unhandled_message`) — the wording is
        // byte-identical by construction, since both tiers call the identical function with the
        // identical two strings.
        if !has_otherwise {
            let covered: std::collections::HashSet<Symbol> =
                arms.iter().filter_map(|a| a.variant).collect();
            let enum_of = arms
                .iter()
                .find_map(|a| a.variant.and_then(|v| self.variant_enum.get(&v).copied()));
            let user_enum: Option<(String, Vec<String>)> = enum_of
                .and_then(|e| self.enum_variants.get(&e).map(|vs| (e, vs.clone())))
                .map(|(e, vs)| {
                    let enum_name = self.interner.resolve(e).to_string();
                    let uncovered = vs
                        .into_iter()
                        .filter(|v| !covered.contains(v))
                        .map(|v| self.interner.resolve(v).to_string())
                        .collect();
                    (enum_name, uncovered)
                });
            match user_enum {
                Some((enum_name, uncovered)) => {
                    self.emit_inspect_fallthrough(t, &enum_name, &uncovered)?
                }
                None => {
                    // `Option`/`Result` are language built-ins: no `## A ... is either:`
                    // declaration ever puts them in the discovery-pass registry, so they are
                    // matched by the ARM'S OWN TEXT here, not by `variant_enum`'s Symbol lookup.
                    // This also has to work when the Inspect names only ONE of a pair (`When Some
                    // (n):` with no `None` arm at all) — constructing `none` embeds the literal
                    // string "None" directly in bytecode (`compile_option_ctor`), never interning
                    // a Symbol for it, so "None" may not exist as a Symbol in this compilation's
                    // Interner AT ALL; text comparison sidesteps that entirely.
                    let covered_text: std::collections::HashSet<&str> = arms
                        .iter()
                        .filter_map(|a| a.variant)
                        .map(|v| self.interner.resolve(v))
                        .collect();
                    let builtin = [("Option", ["Some", "None"]), ("Result", ["Ok", "Err"])]
                        .into_iter()
                        .find(|(_, ctors)| ctors.iter().any(|c| covered_text.contains(c)));
                    match builtin {
                        Some((enum_name, ctors)) => {
                            let uncovered: Vec<String> = ctors
                                .iter()
                                .filter(|c| !covered_text.contains(**c))
                                .map(|s| s.to_string())
                                .collect();
                            self.emit_inspect_fallthrough(t, enum_name, &uncovered)?;
                        }
                        // No statically known enum (a dynamic value, or `types` was not given at
                        // all — the simple `Compiler::compile` entry point) — S20:
                        // `BuiltinId::InspectUnhandledDynamic` reads the scrutinee's OWN runtime
                        // constructor and type at the ONE place that knows them (there is no
                        // static enum to look up), the same words the tree-walker's own
                        // `inspect_unhandled` builds — fed the ALREADY-EVALUATED scrutinee
                        // register directly, no re-compiled source expression needed.
                        None => {
                            let arg = self.reserve_regs(1)?;
                            self.emit(Op::Move { dst: arg, src: t });
                            let dst = self.alloc_reg()?;
                            self.emit(Op::CallBuiltin {
                                dst,
                                builtin:
                                    crate::semantics::builtins::BuiltinId::InspectUnhandledDynamic,
                                args_start: arg,
                                arg_count: 1,
                            });
                        }
                    }
                }
            }
        }
        let end_pc = self.current_pc();
        for j in end_jumps {
            self.patch_jump_target(j, end_pc)?;
        }
        Ok(())
    }

    /// An operand of an equality: `none` as the absence constant `nothing` (equal to it on every
    /// tier), anything else as itself.
    fn compile_equality_operand(&mut self, e: &Expr) -> Result<Reg, Refusal> {
        if !matches!(e, Expr::OptionNone) {
            return self.compile_expr(e);
        }
        let dst = self.alloc_reg()?;
        let idx = self.add_const(Constant::Nothing)?;
        self.emit(Op::LoadConst { dst, idx });
        Ok(dst)
    }

    /// Compile `a new T …` — collections, or a struct with default-fill.
    /// Lower an Option constructor to the same tagged `Op::NewInductive` a `Result` uses.
    ///
    /// One place, so the type name and both constructor names are spelled exactly once — every
    /// backend derives its discriminant from those strings, so a second spelling would be a silent
    /// tier divergence rather than a compile error.
    fn compile_option_ctor(
        &mut self,
        ctor: &str,
        payload: Option<&Expr>,
        dst: Reg,
    ) -> Result<(), Refusal> {
        let tidx = self.add_const(Constant::Text("Option".to_string()))?;
        let cidx = self.add_const(Constant::Text(ctor.to_string()))?;
        let count: u16 = if payload.is_some() { 1 } else { 0 };
        let args_start = self.reserve_regs(count)?;
        if let Some(value) = payload {
            self.compile_expr_into(value, args_start)?;
        }
        self.emit(Op::NewInductive {
            dst,
            type_name: tidx,
            ctor: cidx,
            args_start,
            count,
        });
        Ok(())
    }

    fn compile_new(
        &mut self,
        type_name: Symbol,
        init_fields: &[(Symbol, &Expr)],
        dst: Reg,
    ) -> Result<(), Refusal> {
        // Collection names win unconditionally (tree-walker order).
        match self.interner.resolve(type_name) {
            "Seq" | "List" => {
                self.emit(Op::NewEmptyList { dst });
                return Ok(());
            }
            "Set" | "HashSet" => {
                self.emit(Op::NewEmptySet { dst });
                return Ok(());
            }
            "Map" | "HashMap" => {
                self.emit(Op::NewEmptyMap { dst });
                return Ok(());
            }
            _ => {}
        }
        let name = self.interner.resolve(type_name).to_string();
        let name_idx = self.add_const(Constant::Text(name))?;
        self.emit(Op::NewStruct {
            dst,
            type_name: name_idx,
        });

        let mut provided: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
        for (field_sym, field_expr) in init_fields {
            provided.insert(*field_sym);
            let fname = self.interner.resolve(*field_sym).to_string();
            let fidx = self.add_const(Constant::Text(fname))?;
            let v = self.compile_expr(field_expr)?;
            self.emit(Op::StructInsert {
                obj: dst,
                field: fidx,
                value: v,
            });
        }

        // Default-fill the declared fields not provided (tree-walker defaults
        // by declared type name).
        if let Some(def) = self.struct_defs.get(&type_name).cloned() {
            for (field_sym, type_sym, _) in def {
                if provided.contains(&field_sym) {
                    continue;
                }
                let fname = self.interner.resolve(field_sym).to_string();
                let fidx = self.add_const(Constant::Text(fname))?;
                let v = self.alloc_reg()?;
                match self.interner.resolve(type_sym) {
                    "Int" | "Byte" => {
                        let i = self.add_const(Constant::Int(0))?;
                        self.emit(Op::LoadConst { dst: v, idx: i });
                    }
                    "Float" => {
                        let i = self.add_const(Constant::Float(0.0))?;
                        self.emit(Op::LoadConst { dst: v, idx: i });
                    }
                    "Bool" => {
                        let i = self.add_const(Constant::Bool(false))?;
                        self.emit(Op::LoadConst { dst: v, idx: i });
                    }
                    "Text" | "String" => {
                        let i = self.add_const(Constant::Text(String::new()))?;
                        self.emit(Op::LoadConst { dst: v, idx: i });
                    }
                    "Char" => {
                        let i = self.add_const(Constant::Char('\0'))?;
                        self.emit(Op::LoadConst { dst: v, idx: i });
                    }
                    "Seq" | "List" => self.emit(Op::NewEmptyList { dst: v }),
                    "Set" | "HashSet" => self.emit(Op::NewEmptySet { dst: v }),
                    "Map" | "HashMap" => self.emit(Op::NewEmptyMap { dst: v }),
                    // A `Shared` struct's CRDT fields default to an empty live CRDT — the SAME
                    // name table the tree-walker's `new`-struct init reads.
                    name if crate::semantics::crdt::CrdtKind::of_type_name(name).is_some() => {
                        let kind = crate::semantics::crdt::CrdtKind::of_type_name(name)
                            .expect("guarded")
                            .new_crdt_code();
                        self.emit(Op::NewCrdt { dst: v, kind })
                    }
                    _ => {
                        let i = self.add_const(Constant::Nothing)?;
                        self.emit(Op::LoadConst { dst: v, idx: i });
                    }
                }
                self.emit(Op::StructInsert {
                    obj: dst,
                    field: fidx,
                    value: v,
                });
            }
        }
        Ok(())
    }

    /// env.lg's `get(name)`, expanded into ops every downstream tier already lowers — the rule
    /// `mapOf` follows: whether the variable is set ([`BuiltinId::EnvHas`]), then `some` of its text
    /// ([`BuiltinId::EnvValue`]) or `none`, built by `NewInductive` as any other `Option` is. The
    /// tree-walker runs [`BuiltinId::EnvGet`] whole; both read the environment through one seam.
    ///
    /// The `none` is built first and the `some` last, so a backend that reads a register's payload
    /// positions from its last construction in program order (`vm/wasm/kind.rs`) binds the `Some`
    /// arm's text from the construction that carries it.
    fn compile_env_get(&mut self, name: &Expr, dst: Reg) -> Result<(), Refusal> {
        use crate::semantics::builtins::BuiltinId;
        // A builtin call clears its argument registers, so the first half reads a copy of the name and
        // the second takes the name itself.
        let key = self.reserve_regs(1)?;
        self.compile_expr_into(name, key)?;
        let asked = self.reserve_regs(1)?;
        self.emit(Op::Move { dst: asked, src: key });
        let present = self.alloc_reg()?;
        self.emit(Op::CallBuiltin {
            dst: present,
            builtin: BuiltinId::EnvHas,
            args_start: asked,
            arg_count: 1,
        });
        let j_some = self.current_pc();
        self.emit(Op::JumpIfTrue {
            cond: present,
            target: usize::MAX,
        });
        let option = self.add_const(Constant::Text("Option".to_string()))?;
        let none = self.add_const(Constant::Text("None".to_string()))?;
        let payload = self.reserve_regs(1)?;
        self.emit(Op::NewInductive {
            dst,
            type_name: option,
            ctor: none,
            args_start: payload,
            count: 0,
        });
        let j_end = self.emit_placeholder_jump();
        let some_pc = self.current_pc();
        self.patch_jump_target(j_some, some_pc)?;
        self.emit(Op::CallBuiltin {
            dst: payload,
            builtin: BuiltinId::EnvValue,
            args_start: key,
            arg_count: 1,
        });
        let some = self.add_const(Constant::Text("Some".to_string()))?;
        self.emit(Op::NewInductive {
            dst,
            type_name: option,
            ctor: some,
            args_start: payload,
            count: 1,
        });
        let end_pc = self.current_pc();
        self.patch_jump_target(j_end, end_pc)?;
        Ok(())
    }

    /// Compile `and`/`or` — logical truthiness with short-circuit, Bool result,
    /// pure control flow (JumpIfTrue/False are `is_truthy`-based, so containers,
    /// floats and Text all follow the one truthiness definition). The right
    /// operand is compiled ONCE and only evaluated when the left doesn't decide.
    ///
    /// ```text
    ///   rL = eval(left)
    ///   JumpIfTrue  rL → eval  (and)  ; or: JumpIfFalse rL → eval
    ///   dst = false            (and)  ; or: dst = true   — short-circuit
    ///   Jump → end
    /// eval:
    ///   rR = eval(right)
    ///   JumpIfTrue  rR → t
    ///   dst = false
    ///   Jump → end
    /// t:
    ///   dst = true
    /// end:
    /// ```
    fn compile_short_circuit(
        &mut self,
        is_and: bool,
        left: &Expr,
        right: &Expr,
        dst: Reg,
    ) -> Result<(), Refusal> {
        let l = self.compile_expr(left)?;

        let j_eval = self.current_pc();
        if is_and {
            self.emit(Op::JumpIfTrue {
                cond: l,
                target: usize::MAX,
            });
        } else {
            self.emit(Op::JumpIfFalse {
                cond: l,
                target: usize::MAX,
            });
        }

        // Short-circuit result: `and` → false, `or` → true.
        let idx_false = self.add_const(Constant::Bool(false))?;
        let idx_true = self.add_const(Constant::Bool(true))?;
        self.emit(Op::LoadConst {
            dst,
            idx: if is_and { idx_false } else { idx_true },
        });
        let j_end = self.emit_placeholder_jump();

        let eval_pc = self.current_pc();
        self.patch_jump_target(j_eval, eval_pc)?;
        let r = self.compile_expr(right)?;
        let j_true = self.current_pc();
        self.emit(Op::JumpIfTrue {
            cond: r,
            target: usize::MAX,
        });
        self.emit(Op::LoadConst {
            dst,
            idx: idx_false,
        });
        let j_end2 = self.emit_placeholder_jump();
        let t_pc = self.current_pc();
        self.patch_jump_target(j_true, t_pc)?;
        self.emit(Op::LoadConst { dst, idx: idx_true });
        self.patch_jump_target(j_end, self.current_pc())?;
        self.patch_jump_target(j_end2, self.current_pc())?;
        Ok(())
    }

    /// Compile a call, writing the result to `dst`. Dispatch order mirrors the
    /// tree-walker exactly: `show` → kernel builtins → user functions →
    /// runtime "Unknown function". Arity errors fire at RUNTIME, BEFORE the
    /// arguments are evaluated.
    /// Compile `Launch a task to f with args` (and the handle-binding variant).
    /// Mirrors [`Self::compile_call`]'s function resolution + contiguous arg
    /// window, then emits `Spawn`/`SpawnHandle`.
    fn compile_spawn(
        &mut self,
        function: Symbol,
        args: &[&Expr],
        handle: Option<Symbol>,
    ) -> Result<(), Refusal> {
        let func = match self.fn_index.get(&function) {
            Some(&f) => f,
            None => {
                return self.emit_fail(&format!(
                    "Unknown task function: {}",
                    self.interner.resolve(function)
                ))
            }
        };
        let arg_count =
            u16::try_from(args.len()).map_err(|_| "vm: too many task arguments".to_string())?;
        let args_start = self.reserve_regs(arg_count)?;
        for (i, arg) in args.iter().enumerate() {
            self.compile_expr_into(arg, args_start + i as Reg)?;
        }
        match handle {
            Some(h) => {
                let dst = self.let_reg(h)?;
                self.emit(Op::SpawnHandle {
                    dst,
                    func,
                    args_start,
                    arg_count,
                });
            }
            None => self.emit(Op::Spawn {
                func,
                args_start,
                arg_count,
            }),
        }
        Ok(())
    }

    fn place_key_count(expr: &Expr) -> Option<usize> {
        match expr {
            Expr::Identifier(_) => Some(0),
            Expr::FieldAccess { object, .. } => Self::place_key_count(object),
            Expr::Index { collection, .. } => Self::place_key_count(collection).map(|n| n + 1),
            _ => None,
        }
    }

    /// Resolve the keys of a mutable argument once, in source order. The value
    /// at that place is taken before later arguments run and restored after the
    /// call using these same keys.
    fn compile_lent_place(
        &mut self,
        expr: &Expr,
        next_key: &mut Reg,
    ) -> Result<Option<LentPlace>, Refusal> {
        match expr {
            Expr::Identifier(root) => Ok(Some(LentPlace {
                root: *root,
                steps: Vec::new(),
            })),
            Expr::FieldAccess { object, field } => {
                let Some(mut place) = self.compile_lent_place(object, next_key)? else {
                    return Ok(None);
                };
                let name = self.interner.resolve(*field).to_string();
                place
                    .steps
                    .push(LentStep::Field(self.add_const(Constant::Text(name))?));
                Ok(Some(place))
            }
            Expr::Index { collection, index } => {
                let Some(mut place) = self.compile_lent_place(collection, next_key)? else {
                    return Ok(None);
                };
                let key = *next_key;
                *next_key += 1;
                self.compile_expr_into(index, key)?;
                place.steps.push(LentStep::Item(key));
                Ok(Some(place))
            }
            _ => Ok(None),
        }
    }

    /// Write a previously resolved place. Each ancestor is made independent of
    /// other bindings before its child is extracted, then rebuilt immediately;
    /// no ancestor remains empty while another argument or the callee runs.
    fn mutate_lent_place(
        &mut self,
        place: &LentPlace,
        mutate: &mut dyn FnMut(&mut Self, Reg) -> Result<(), Refusal>,
    ) -> Result<bool, Refusal> {
        fn descend<'c>(
            c: &mut Compiler<'c>,
            parent: Reg,
            steps: &[LentStep],
            mutate: &mut dyn FnMut(&mut Compiler<'c>, Reg) -> Result<(), Refusal>,
        ) -> Result<(), Refusal> {
            let Some((step, rest)) = steps.split_first() else {
                return mutate(c, parent);
            };
            c.emit(Op::EnsureOwned { reg: parent });
            let child = c.alloc_reg()?;
            let key = match step {
                LentStep::Field(field) => {
                    let key = c.alloc_reg()?;
                    c.emit(Op::LoadConst {
                        dst: key,
                        idx: *field,
                    });
                    c.emit(Op::GetField {
                        dst: child,
                        obj: parent,
                        field: *field,
                    });
                    key
                }
                LentStep::Item(index) => {
                    c.emit(Op::Index {
                        dst: child,
                        collection: parent,
                        index: *index,
                    });
                    *index
                }
            };
            let empty = c.alloc_reg()?;
            let nothing = c.add_const(Constant::Nothing)?;
            c.emit(Op::LoadConst {
                dst: empty,
                idx: nothing,
            });
            c.emit(Op::SetIndex {
                collection: parent,
                index: key,
                value: empty,
            });
            descend(c, child, rest, mutate)?;
            c.emit(Op::SetIndex {
                collection: parent,
                index: key,
                value: child,
            });
            Ok(())
        }
        self.compile_place_mutation(&Expr::Identifier(place.root), &mut |c, root| {
            descend(c, root, &place.steps, mutate)
        })
    }

    /// Call a function the program defines: arity check, the call-site COW barrier for
    /// mutable-borrow parameters, then `Op::Call`.
    fn compile_user_call(
        &mut self,
        function: Symbol,
        args: &[&Expr],
        dst: Reg,
    ) -> Result<(), Refusal> {
        let func = self.fn_index[&function];
        let name = self.interner.resolve(function);
        let param_count = self.functions[func as usize].param_count as usize;
        if args.len() != param_count {
            let msg = format!(
                "Function {} expects {} arguments, got {}",
                name,
                param_count,
                args.len()
            );
            let idx = self.add_const(Constant::Text(msg))?;
            self.emit(Op::FailWith { msg: idx });
            return Ok(());
        }
        let arg_count =
            u16::try_from(args.len()).map_err(|_| "vm: too many arguments".to_string())?;
        let inout = self
            .inout_params
            .get(&function)
            .cloned()
            .unwrap_or_default();
        let keys: usize = args
            .iter()
            .enumerate()
            .filter(|(i, _)| inout.contains(i))
            .map(|(_, arg)| Self::place_key_count(arg).unwrap_or(0))
            .sum();
        let keys =
            u16::try_from(keys).map_err(|_| "vm: too many mutable argument keys".to_string())?;
        // Callee registers overlay the caller from args_start onward. Keys used
        // after return belong below that window, even when their evaluation runs
        // later, at the mutable argument's position in source order.
        let mut next_key = self.reserve_regs(keys)?;
        let args_start = self.reserve_regs(arg_count)?;
        // Call-site COW barrier: before an identifier argument is passed to a
        // mutable-borrow parameter, isolate its buffer if shared (see
        // [`Op::EnsureOwned`]) so the callee's in-place writes stay value-semantic
        // for any aliasing caller. Emitted BEFORE materialization, while the
        // source register still holds the caller's handle. A no-op for the common
        // uniquely-owned `Set x to f(x, …)` consume-reassign.
        // An in-out argument that names a local: the binding's own value is isolated before it is
        // passed, so the callee's in-place writes reach no other copy of it.
        for &i in &inout {
            if let Some(Expr::Identifier(x)) = args.get(i) {
                if let NameRef::Local(x_reg) = self.resolve_name(*x) {
                    self.emit(Op::EnsureOwned { reg: x_reg });
                }
            }
        }
        if let Some(mb_idxs) = self.mut_borrow_params.get(&function).cloned() {
            for (i, arg) in args.iter().enumerate() {
                if mb_idxs.contains(&i) && !inout.contains(&i) {
                    if let Expr::Identifier(x) = arg {
                        // Skip an argument that is itself a COW-exempt local of the
                        // CURRENT function (a self-recursive `f(result, …)` where
                        // `result` aliases our own mutable-borrow param). It has no
                        // external alias — the mutable-borrow analysis proved that —
                        // so the barrier is a runtime no-op; omitting it keeps the
                        // op out of a hot, tier-able function body.
                        if self.current_exempt_syms.contains(x) {
                            continue;
                        }
                        if let NameRef::Local(x_reg) = self.resolve_name(*x) {
                            self.emit(Op::EnsureOwned { reg: x_reg });
                        }
                    }
                }
            }
        }
        let mut lent = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let argument = args_start + i as Reg;
            if inout.contains(&i) {
                if let Some(place) = self.compile_lent_place(arg, &mut next_key)? {
                    let nothing = self.add_const(Constant::Nothing)?;
                    self.mutate_lent_place(&place, &mut |c, slot| {
                        c.emit(Op::Move {
                            dst: argument,
                            src: slot,
                        });
                        c.emit(Op::LoadConst {
                            dst: slot,
                            idx: nothing,
                        });
                        Ok(())
                    })?;
                    self.emit(Op::EnsureOwned { reg: argument });
                    lent.push((i, place));
                    continue;
                }
            }
            self.compile_expr_into(arg, argument)?;
        }
        self.emit(Op::Call {
            dst,
            func,
            args_start,
            arg_count,
        });
        // The call leaves each mutable parameter's final value in its argument
        // register. Return it through the original place, with the original keys.
        for (i, place) in lent {
            let src = args_start + i as Reg;
            self.mutate_lent_place(&place, &mut |c, slot| {
                c.emit(Op::Move { dst: slot, src });
                Ok(())
            })?;
        }
        Ok(())
    }

    fn compile_call(&mut self, function: Symbol, args: &[&Expr], dst: Reg) -> Result<(), Refusal> {
        use crate::semantics::builtins::{builtin_from_name, check_arity, BuiltinId};

        // "Declarer wins": a function the program defines with a body is ITS function, whatever
        // builtin or intercepted native shares the name. Only a native declaration (`## To native
        // args`) defers to the engine's implementation.
        if self.fn_index.contains_key(&function) && !self.native_fns.contains_key(&function) {
            return self.compile_user_call(function, args, dst);
        }
        let name = self.interner.resolve(function);
        if name == "show" {
            // show(a, b, …) emits each argument; result is Nothing.
            for arg in args {
                let src = self.compile_expr(arg)?;
                self.emit(Op::Show { src });
            }
            let idx = self.add_const(Constant::Nothing)?;
            self.emit(Op::LoadConst { dst, idx });
            return Ok(());
        }

        // `args()` is the program-argument system native (declared `## To
        // native args ()`); it resolves to the VM's stored argv, like the
        // compiled binary's `env::args()`. Intercepted by name BEFORE the
        // empty native-decl body would be reached, mirroring `show`.
        if name == "args" {
            self.emit(Op::Args { dst });
            return Ok(());
        }

        // `loaded()` is the DATA-SEGMENT weight blob a chip receives separately from the kernel (like an
        // accelerator's HBM/SRAM): the SoC's UART loader preloads it into the heap as a `Seq of Int` at
        // pointer 0 (`dmem[0]=len, dmem[1..]=weights`) and seeds the bump allocator past it, so `loaded()`
        // is simply that pointer. Declared `## To native loaded () -> Seq of Int` for the type; intercepted
        // here by name (mirroring `args`) to a constant-0 pointer, so `item k of loaded()` reuses the
        // existing `Index` op (`dmem[0+k]`, bounds-checked by the preloaded length) — no new opcode.
        if name == "loaded" {
            let idx = self.add_const(Constant::Int(0))?;
            self.emit(Op::LoadConst { dst, idx });
            return Ok(());
        }

        // `{k: v, …}` / `{a, b, …}` literals lower to `mapOf`/`setOf`; the VM
        // expands them into ops it already has (NewEmptyMap + SetIndex /
        // NewEmptySet + SetAdd) so every downstream tier — JIT, direct-WASM,
        // the PE dialects — sees only cataloged ops, no new CallBuiltin arm.
        if matches!(name, "mapOf" | "setOf") {
            let id = builtin_from_name(name).expect("mapOf/setOf are registered builtins");
            if let Err(msg) = check_arity(id, args.len()) {
                let idx = self.add_const(Constant::Text(msg))?;
                self.emit(Op::FailWith { msg: idx });
                return Ok(());
            }
            if id == BuiltinId::MapOf {
                self.emit(Op::NewEmptyMap { dst });
                for pair in args.chunks(2) {
                    let k = self.compile_expr_before(pair[0], &[pair[1]])?;
                    let v = self.compile_expr(pair[1])?;
                    self.emit(Op::SetIndex {
                        collection: dst,
                        index: k,
                        value: v,
                    });
                }
            } else {
                self.emit(Op::NewEmptySet { dst });
                for arg in args {
                    let v = self.compile_expr(arg)?;
                    self.emit(Op::SetAdd { set: dst, value: v });
                }
            }
            return Ok(());
        }

        if let Some(id) = builtin_from_name(name) {
            if let Err(msg) = check_arity(id, args.len()) {
                let idx = self.add_const(Constant::Text(msg))?;
                self.emit(Op::FailWith { msg: idx });
                return Ok(());
            }
            // `format` evaluates only its first argument (tree-walker laziness).
            let used: &[&Expr] = if id == BuiltinId::Format && !args.is_empty() {
                &args[..1]
            } else {
                args
            };
            let arg_count =
                u16::try_from(used.len()).map_err(|_| "vm: too many arguments".to_string())?;
            let args_start = self.reserve_regs(arg_count)?;
            for (i, arg) in used.iter().enumerate() {
                self.compile_expr_into(arg, args_start + i as Reg)?;
            }
            self.emit(Op::CallBuiltin {
                dst,
                builtin: id,
                args_start,
                arg_count,
            });
            return Ok(());
        }

        // System natives declared in the stdlib (`## To native …`, EMPTY bodies) —
        // compile to the SAME shared implementations the tree-walker calls, so the
        // tiers cannot drift. Guarded on membership in `native_fns`: a
        // user-defined function with a real body is not in the map and compiles as
        // an ordinary call below (declarer-wins).
        if let Some(&rust_path) = self.native_fns.get(&function) {
            // file.lg / random.lg / env.lg / time.lg → the shared `CallBuiltin` seam.
            if let Some(id) = crate::semantics::builtins::system_native(name, args.len()) {
                if id == BuiltinId::EnvGet {
                    return self.compile_env_get(args[0], dst);
                }
                let arg_count =
                    u16::try_from(args.len()).map_err(|_| "vm: too many arguments".to_string())?;
                let args_start = self.reserve_regs(arg_count)?;
                for (i, arg) in args.iter().enumerate() {
                    self.compile_expr_into(arg, args_start + i as Reg)?;
                }
                self.emit(Op::CallBuiltin {
                    dst,
                    builtin: id,
                    args_start,
                    arg_count,
                });
                return Ok(());
            }
            // crypto.lg's native kernels → `CallBuiltin(NativeKernel)` with the kernel's code as
            // argument 0 — the same `NativeKernel::call` the tree-walker runs.
            if let Some(kernel) = crate::semantics::native_kernels::NativeKernel::from_name(name)
                .filter(|k| k.arity() == args.len())
            {
                let arg_count = u16::try_from(args.len() + 1)
                    .map_err(|_| "vm: too many arguments".to_string())?;
                let args_start = self.reserve_regs(arg_count)?;
                let code = self.add_const(Constant::Int(kernel.code()))?;
                self.emit(Op::LoadConst {
                    dst: args_start,
                    idx: code,
                });
                for (i, arg) in args.iter().enumerate() {
                    self.compile_expr_into(arg, args_start + 1 + i as Reg)?;
                }
                self.emit(Op::CallBuiltin {
                    dst,
                    builtin: BuiltinId::NativeKernel,
                    args_start,
                    arg_count,
                });
                return Ok(());
            }
            // tcp.lg → `Op::Tcp` (suspends; the async io driver runs the SAME
            // `TcpRegistry::exec_values` the tree-walker calls).
            if let Some(op) = crate::concurrency::tcp_registry::TcpOp::from_name(name, args.len()) {
                let arg_count =
                    u16::try_from(args.len()).map_err(|_| "vm: too many arguments".to_string())?;
                let args_start = self.reserve_regs(arg_count)?;
                for (i, arg) in args.iter().enumerate() {
                    self.compile_expr_into(arg, args_start + i as Reg)?;
                }
                self.emit(Op::Tcp {
                    dst,
                    kind: op.code(),
                    args_start,
                    arg_count,
                });
                return Ok(());
            }
            // time.lg's `sleep(ms)` is the `Sleep ms.` statement, answering nothing.
            if name == "sleep" && args.len() == 1 {
                let duration = self.compile_expr(args[0])?;
                self.emit(Op::Sleep { duration });
                let idx = self.add_const(Constant::Nothing)?;
                self.emit(Op::LoadConst { dst, idx });
                return Ok(());
            }
            let refusal = crate::semantics::builtins::unimplemented_native(
                name,
                rust_path.map(|p| self.interner.resolve(p)),
            );
            let idx = self.add_const(Constant::Text(refusal))?;
            self.emit(Op::FailWith { msg: idx });
            return Ok(());
        }

        if self.fn_index.contains_key(&function) {
            return self.compile_user_call(function, args, dst);
        }

        // A variable holding a closure: call it by name. A bound non-Function
        // value errors "Unknown function: {name}" at runtime — the
        // tree-walker's by-name fallback.
        if !matches!(self.resolve_name(function), NameRef::Unbound) {
            let callee = self.compile_expr_before(&Expr::Identifier(function), args)?;
            let arg_count =
                u16::try_from(args.len()).map_err(|_| "vm: too many arguments".to_string())?;
            let args_start = self.reserve_regs(arg_count)?;
            for (i, arg) in args.iter().enumerate() {
                self.compile_expr_into(arg, args_start + i as Reg)?;
            }
            let name_idx = self.add_const(Constant::Text(name.to_string()))?;
            self.emit(Op::CallValue {
                dst,
                callee,
                args_start,
                arg_count,
                name_for_err: name_idx,
            });
            return Ok(());
        }

        let msg = format!("Unknown function: {}", name);
        let idx = self.add_const(Constant::Text(msg))?;
        self.emit(Op::FailWith { msg: idx });
        Ok(())
    }

    /// Compile `Show x to f` / `Give x to f` — the value goes to a FUNCTION
    /// recipient via the tree-walker's `call_function_with_values` dispatch,
    /// which knows only `show`, user functions, and closures (NOT the other
    /// builtins).
    fn compile_recipient_call(&mut self, recipient: Symbol, object: &Expr) -> Result<(), Refusal> {
        let name = self.interner.resolve(recipient);
        if name == "show" {
            let src = self.compile_expr(object)?;
            self.emit(Op::Show { src });
            return Ok(());
        }
        if let Some(&func) = self.fn_index.get(&recipient) {
            let param_count = self.functions[func as usize].param_count as usize;
            let args_start = self.reserve_regs(1)?;
            self.compile_expr_into(object, args_start)?;
            if param_count != 1 {
                let msg = format!("Function {} expects {} arguments, got 1", name, param_count);
                let idx = self.add_const(Constant::Text(msg))?;
                self.emit(Op::FailWith { msg: idx });
                return Ok(());
            }
            let dst = self.alloc_reg()?;
            self.emit(Op::Call {
                dst,
                func,
                args_start,
                arg_count: 1,
            });
            return Ok(());
        }
        // A variable holding a closure (the with_values closure fallback).
        if !matches!(self.resolve_name(recipient), NameRef::Unbound) {
            let args_start = self.reserve_regs(1)?;
            self.compile_expr_into(object, args_start)?;
            let callee = self.compile_expr(&Expr::Identifier(recipient))?;
            let dst = self.alloc_reg()?;
            let name_idx = self.add_const(Constant::Text(name.to_string()))?;
            self.emit(Op::CallValue {
                dst,
                callee,
                args_start,
                arg_count: 1,
                name_for_err: name_idx,
            });
            return Ok(());
        }
        // The object's side effects happen before the dispatch failure.
        let scratch = self.alloc_reg()?;
        self.compile_expr_into(object, scratch)?;
        let msg = format!("Unknown function: {}", name);
        let idx = self.add_const(Constant::Text(msg))?;
        self.emit(Op::FailWith { msg: idx });
        Ok(())
    }

    /// Compile an interpolated string: parts accumulate into a Text register
    /// via Concat (Text+Text concatenation — identical to the tree-walker's
    /// push_str building).
    fn compile_interpolation(
        &mut self,
        parts: &[crate::ast::stmt::StringPart],
        dst: Reg,
    ) -> Result<(), Refusal> {
        use crate::ast::stmt::StringPart;

        let empty = self.add_const(Constant::Text(String::new()))?;
        self.emit(Op::LoadConst { dst, idx: empty });
        for part in parts {
            match part {
                StringPart::Literal(sym) => {
                    let idx =
                        self.add_const(Constant::Text(self.interner.resolve(*sym).to_string()))?;
                    let lit = self.alloc_reg()?;
                    self.emit(Op::LoadConst { dst: lit, idx });
                    self.emit(Op::Concat {
                        dst,
                        lhs: dst,
                        rhs: lit,
                    });
                }
                StringPart::Expr {
                    value,
                    format_spec,
                    debug,
                } => {
                    let v = self.compile_expr(value)?;
                    let needs_format = format_spec.is_some() || *debug;
                    let piece = if needs_format {
                        let spec = match format_spec {
                            Some(sym) => self.add_const(Constant::Text(
                                self.interner.resolve(*sym).to_string(),
                            ))?,
                            None => u32::MAX,
                        };
                        let debug_prefix = if *debug {
                            let prefix = match value {
                                Expr::Identifier(sym) => self.interner.resolve(*sym).to_string(),
                                _ => "expr".to_string(),
                            };
                            self.add_const(Constant::Text(prefix))?
                        } else {
                            u32::MAX
                        };
                        let formatted = self.alloc_reg()?;
                        self.emit(Op::FormatValue {
                            dst: formatted,
                            src: v,
                            spec,
                            debug_prefix,
                        });
                        formatted
                    } else {
                        v
                    };
                    self.emit(Op::Concat {
                        dst,
                        lhs: dst,
                        rhs: piece,
                    });
                }
            }
        }
        Ok(())
    }

    /// Compile a closure literal: the body becomes an anonymous function
    /// emitted INLINE (jumped over at the creation site); local captures are
    /// snapshotted from a register window, global captures from the globals
    /// table at creation time. Frame layout:
    /// `[params…, capture values…, capture-present flags…]`.
    fn compile_closure(
        &mut self,
        params: &[(Symbol, &crate::ast::stmt::TypeExpr)],
        body: &crate::ast::stmt::ClosureBody,
        dst: Reg,
    ) -> Result<(), Refusal> {
        use crate::ast::stmt::ClosureBody;
        use crate::interpreter::Interpreter;

        let free = Interpreter::free_vars_in_closure(params, body);
        let mut captures: Vec<(Symbol, Option<u16>)> = Vec::new();
        let mut local_sources: Vec<Reg> = Vec::new();
        for sym in free {
            match self.resolve_name(sym) {
                NameRef::Local(r) => {
                    captures.push((sym, None));
                    local_sources.push(r);
                }
                NameRef::CaptureOrGlobal { value, global, .. } => {
                    // Closure-in-closure over a promoted capture: snapshot the
                    // capture slot (the live-global fall-through nests with
                    // the outer flag at creation time only).
                    let _ = global;
                    captures.push((sym, None));
                    local_sources.push(value);
                }
                NameRef::Global(g) => captures.push((sym, Some(g))),
                NameRef::Unbound => {}
            }
        }

        // Move local capture sources into a contiguous window for MakeClosure.
        let local_count =
            u16::try_from(local_sources.len()).map_err(|_| "vm: too many captures".to_string())?;
        let locals_start = self.reserve_regs(local_count)?;
        for (i, src) in local_sources.iter().enumerate() {
            let w = locals_start + i as Reg;
            if *src != w {
                self.emit(Op::Move { dst: w, src: *src });
            }
        }

        // Register the body as a function and compile it inline, jumped over.
        let func_idx = u16::try_from(self.functions.len())
            .map_err(|_| "vm: too many functions".to_string())?;
        let param_count =
            u16::try_from(params.len()).map_err(|_| "vm: too many parameters".to_string())?;
        let jover = self.emit_placeholder_jump();
        self.functions.push(CompiledFunction {
            name: Symbol::default(),
            entry_pc: self.code.len(),
            param_count,
            register_count: 0,
            captures: captures.clone(),
            named_regs: Vec::new(),
            // Closures carry no declarations — all-Int is the historical
            // entry-guard contract (kept for the VM/JIT tiering path).
            param_kinds: vec![
                Some(super::native_tier::ParamKind::Scalar(
                    super::native_tier::SlotKind::Int
                ));
                param_count as usize
            ],
            ret_kind: None,
            // The closure's DECLARED parameter types — a `(n: Float)` closure must type its param as
            // Float, not the all-Int entry-guard default, or a static backend (the WASM AOT) mis-sizes
            // its signature (f64 arg vs i64 param). Additive: only the AOT reads `param_types` (the VM/
            // JIT use `param_kinds`, left untouched). Scalars resolve with an empty user-type map; a
            // struct/enum closure parameter resolves via the compiler's `user_types` map.
            param_types: params
                .iter()
                .map(|(_, ty)| boundary_of_type_expr(ty, &self.user_types, self.interner))
                .collect(),
            return_type: None,
            // Closures carry no `mutable` declarations.
            mutable_param_regs: Vec::new(),
            cow_exempt_regs: Vec::new(),
        });

        // Shelve the enclosing frame's compilation state.
        let saved_scopes = std::mem::replace(&mut self.scopes, vec![HashMap::new()]);
        let saved_next = self.next_reg;
        let saved_max = self.max_reg;
        let saved_flow = std::mem::take(&mut self.flow_stack);
        let saved_in_fn = self.in_function;
        let saved_ctx = self.closure_ctx.take();
        let saved_named = std::mem::take(&mut self.named);

        self.in_function = true;
        let p = param_count;
        let cap_n = captures.len() as Reg;
        for (i, (psym, _)) in params.iter().enumerate() {
            self.scopes.last_mut().unwrap().insert(*psym, i as Reg);
            self.mark_named(i as Reg);
        }
        let mut ctx: HashMap<Symbol, (Reg, Reg)> = HashMap::new();
        for (k, (sym, global)) in captures.iter().enumerate() {
            let value = p + k as Reg;
            self.scopes.last_mut().unwrap().insert(*sym, value);
            self.mark_named(value);
            if global.is_some() {
                ctx.insert(*sym, (value, p + cap_n + k as Reg));
            }
        }
        self.next_reg = p + 2 * cap_n;
        self.max_reg = self.next_reg;
        self.closure_ctx = Some(ctx);

        let body_result = (|| -> Result<(), Refusal> {
            match body {
                ClosureBody::Expression(e) => {
                    let r = self.compile_expr(e)?;
                    self.emit(Op::Return { src: r });
                }
                ClosureBody::Block(block) => {
                    for st in block.iter() {
                        self.compile_stmt(st)?;
                    }
                    self.emit(Op::ReturnNothing);
                }
            }
            Ok(())
        })();
        self.functions[func_idx as usize].register_count = self.max_reg as usize;
        let mut cnamed = std::mem::replace(&mut self.named, saved_named);
        cnamed.resize(self.functions[func_idx as usize].register_count, false);
        self.functions[func_idx as usize].named_regs = cnamed;

        // Restore the enclosing frame's state (even when the body failed).
        self.scopes = saved_scopes;
        self.next_reg = saved_next;
        self.max_reg = saved_max.max(self.next_reg);
        self.flow_stack = saved_flow;
        self.in_function = saved_in_fn;
        self.closure_ctx = saved_ctx;
        body_result?;

        self.patch_jump_target(jover, self.current_pc())?;
        self.emit(Op::MakeClosure {
            dst,
            func: func_idx,
            locals_start,
        });
        Ok(())
    }
}

/// The SENTENCE a `UiStmt` was written as, for a refusal that points at the author's own words.
///
/// Spelled as the verb reads in a page rather than as the variant is spelled in Rust: somebody
/// reading the message is looking at `Set the title to …`, not at `SetTitle`.
fn ui_verb_phrase(u: &crate::ast::stmt::UiStmt<'_>) -> &'static str {
    use crate::ast::stmt::UiStmt as U;
    match u {
        U::RaiseWindowEvent { .. } => "Raise",
        U::Focus { .. } => "Focus",
        U::Blur { .. } => "Blur",
        U::Hover { .. } => "Hover",
        U::ScrollTo { .. } => "Scroll to",
        U::AddClass { .. } => "Add class … to",
        U::RemoveClass { .. } => "Remove class … from",
        U::SelectText { .. } => "Select the text in",
        U::CopyToClipboard { .. } => "Copy … to the clipboard",
        U::Download { .. } => "Download … as",
        U::Navigate { .. } => "Navigate to",
        U::ReplaceUrl { .. } => "Replace the URL with",
        U::GoBack => "Go back",
        U::SetTitle { .. } => "Set the title to",
        U::Save { .. } => "Save … as",
        U::Load { .. } => "Load … into",
        U::Forget { .. } => "Forget",
        U::Reload => "Reload the page",
        U::Notify { .. } => "Notify",
        U::Publish { .. } => "Publish … on",
        U::Toggle { .. } => "Toggle",
        U::StopTimer { .. } => "Stop the timer",
        U::StartTimer { .. } => "Start the timer",
        U::CallStore { .. } => "Call … 's action",
        U::Click { .. } => "Click",
        U::Submit { .. } => "Submit",
        U::Choose { .. } => "Choose",
        U::Press { .. } => "Press",
        U::TypeInto { .. } => "Type … into",
        U::Mount { .. } => "Mount",
    }
}

/// An operand of a driving verb's failure message: known at compile time (a literal — every
/// selector and key a test actually writes), or held in a register until run time — see
/// [`Compiler::keep_operand`].
enum DriveOperand {
    Known(String),
    Held(Reg),
}

/// One piece of a failure text: words, or a register whose text is spliced in at run time.
#[derive(Clone)]
enum DrivePiece {
    Text(String),
    Plain(Reg),
}

/// A `semantics::drive` failure message as the compiler emits it: `drive` spells every word, and
/// an operand only known at run time is spliced into the place `drive` put it.
struct DriveText {
    pieces: Vec<DrivePiece>,
}

impl DriveText {
    /// Render `message(selector, key)` — a `semantics::drive` function — with each run-time
    /// operand replaced by a marker, then cut at the markers. `drive` quotes a selector as `{:?}`
    /// does, and a key either bare (`a Enter press`) or quoted the same way (the value a `Type`
    /// enters); a run-time operand is spliced where its marker stood — between the same quote marks
    /// for a quoted one, which is that rendering for any text without a `"`, a `\` or a control
    /// character.
    fn render(
        message: impl Fn(&str, Option<&str>) -> String,
        selector: &DriveOperand,
        key: Option<&DriveOperand>,
    ) -> DriveText {
        const SELECTOR_MARK: &str = "\u{1}selector\u{1}";
        const KEY_MARK: &str = "\u{1}key\u{1}";
        let selector_text = match selector {
            DriveOperand::Known(s) => s.as_str(),
            DriveOperand::Held(_) => SELECTOR_MARK,
        };
        let key_text = key.map(|k| match k {
            DriveOperand::Known(s) => s.as_str(),
            DriveOperand::Held(_) => KEY_MARK,
        });
        let text = message(selector_text, key_text);
        let quoted = |r: Reg| {
            vec![
                DrivePiece::Text("\"".to_string()),
                DrivePiece::Plain(r),
                DrivePiece::Text("\"".to_string()),
            ]
        };
        // Each marker a run-time operand left: its text in the message, and what stands for it.
        let mut marks: Vec<(String, Vec<DrivePiece>)> = Vec::new();
        if let DriveOperand::Held(r) = *selector {
            marks.push((format!("{SELECTOR_MARK:?}"), quoted(r)));
        }
        if let Some(DriveOperand::Held(r)) = key {
            marks.push((format!("{KEY_MARK:?}"), quoted(*r)));
            marks.push((KEY_MARK.to_string(), vec![DrivePiece::Plain(*r)]));
        }
        let mut pieces = Vec::new();
        let mut rest = text.as_str();
        while let Some((at, i)) = marks
            .iter()
            .enumerate()
            .filter_map(|(i, (m, _))| rest.find(m.as_str()).map(|at| (at, i)))
            .min()
        {
            if at > 0 {
                pieces.push(DrivePiece::Text(rest[..at].to_string()));
            }
            let (mark, piece) = &marks[i];
            pieces.extend(piece.iter().cloned());
            rest = &rest[at + mark.len()..];
        }
        if !rest.is_empty() || pieces.is_empty() {
            pieces.push(DrivePiece::Text(rest.to_string()));
        }
        // Adjacent words fold into one constant.
        let mut folded: Vec<DrivePiece> = Vec::with_capacity(pieces.len());
        for piece in pieces {
            match (folded.last_mut(), piece) {
                (Some(DrivePiece::Text(prev)), DrivePiece::Text(t)) => prev.push_str(&t),
                (_, piece) => folded.push(piece),
            }
        }
        DriveText { pieces: folded }
    }
}

/// How an LGX construct is named in a refusal: by the header its author wrote.
///
/// A page's component is interned under a lowered name (`__page__about`) that appears nowhere in
/// the file, so a page is named by its `## Page` header, an error page by its status, and a
/// component by its declaration.
fn construct_heading(def: &crate::ast::component::ComponentDef<'_>, interner: &Interner) -> String {
    use crate::ast::component::ComponentKind;
    match &def.kind {
        ComponentKind::Page { path, .. } => format!("## Page {}", interner.resolve(*path)),
        ComponentKind::ErrorPage { status } => format!("## Error page {status}"),
        ComponentKind::Store { .. } => format!("## Store {}", interner.resolve(def.name)),
        ComponentKind::Component => format!("## Component {}", interner.resolve(def.name)),
    }
}

/// The refusal a `Mount` of each component in `stmts` raises, by component: worded around the
/// reason `lowerable` gives and located at the construct it refused.
fn mount_refusals(
    stmts: &[Stmt],
    interner: &Interner,
    types: Option<&crate::analysis::TypeRegistry>,
) -> HashMap<Symbol, Refusal> {
    use logicaffeine_language::parser::lgx_lower::lowerable;
    let defs: Vec<_> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::ComponentDef(d) => Some((**d).clone()),
            _ => None,
        })
        .collect();
    let mut out = HashMap::new();
    for def in &defs {
        let Err(why) = lowerable(def, &defs, types, interner) else {
            continue;
        };
        // `Trigger::undeliverable` is the one table every tier reads for the sentences NO tier
        // delivers; a component waiting on one is refused for that, not for a gap in this tier.
        let tail = if def
            .events
            .iter()
            .any(|e| e.trigger.undeliverable().is_some())
        {
            "No tier delivers that yet, so the page is refused whole rather than rendered with a \
             sentence in it that would never fire."
        } else {
            "The page is refused whole rather than rendered with a piece missing."
        };
        let heading = construct_heading(def, interner);
        out.insert(
            def.name,
            Refusal::at(
                why.span,
                format!(
                    "`{heading}` cannot run on this tier: {}. {tail}",
                    why.message
                ),
            ),
        );
    }
    out
}

/// The refusal each `## Route` this program cannot lower raises, keyed by `(method, path)`.
///
/// Decided once from the WHOLE program by `lgx_lower::route_refusals`, because a `-> View`
/// route's lowerability depends on the components beside it, and located at the node it refused —
/// a `-> View` route's own `NotLowerable` already carries that node's span; a parameter or
/// answer-type refusal carries the route's own header, having nothing more specific to point at.
fn route_refusal_map(
    stmts: &[Stmt],
    interner: &Interner,
    is_record: &dyn Fn(Symbol) -> bool,
) -> HashMap<(Symbol, Symbol), Refusal> {
    logicaffeine_language::parser::lgx_lower::route_refusals(stmts, interner, is_record)
        .into_iter()
        .map(|(method, path, why)| {
            let heading =
                logicaffeine_language::parser::lgx_lower::route_heading(method, path, interner);
            (
                (method, path),
                Refusal::at(
                    why.span,
                    format!("`{heading}` cannot run on this tier yet: {}.", why.message),
                ),
            )
        })
        .collect()
}

/// The refusal an `## Error page` raises when the server-side walk cannot render its template —
/// located at the node the walk refused.
fn error_page_refusal(
    def: &crate::ast::component::ComponentDef<'_>,
    interner: &Interner,
) -> Option<Refusal> {
    use crate::ast::component::ComponentKind;
    let ComponentKind::ErrorPage { .. } = def.kind else {
        return None;
    };
    let heading = construct_heading(def, interner);
    let why = match def.view.as_ref() {
        Some(v) => {
            logicaffeine_language::parser::lgx_lower::view_html_lowerable(&v.nodes, interner)
                .err()?
        }
        None => {
            return Some(Refusal::at(
                def.span,
                format!(
                    "`{heading}` cannot be rendered on this tier yet: it has no `View:` to render."
                ),
            ))
        }
    };
    Some(Refusal::at(
        why.span,
        format!(
            "`{heading}` cannot be rendered on this tier yet: {}.",
            why.message
        ),
    ))
}

/// The names of every record type the program declares — a `## A … has:` or one the discovery
/// pass registered — which is what a route's answer type is checked against.
fn record_names(
    stmts: &[Stmt],
    types: Option<&crate::analysis::TypeRegistry>,
) -> std::collections::HashSet<Symbol> {
    let mut names: std::collections::HashSet<Symbol> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::StructDef { name, .. } => Some(*name),
            _ => None,
        })
        .collect();
    if let Some(registry) = types {
        names.extend(registry.iter_types().filter_map(|(name, def)| {
            matches!(def, crate::analysis::registry::TypeDef::Struct { .. }).then_some(*name)
        }));
    }
    names
}

/// Every refusal this tier raises for an LGX construct in `stmts` — a mounted component or page it
/// cannot lower, a `## Route`, an `## Error page` — each located at the construct its author
/// wrote, in the order a compile meets them.
///
/// A compile stops at the first; an editor shows them all. Both read the refusals the compiler's
/// own arms raise, so the two cannot disagree about what is refused, what it is called, or where.
pub fn lgx_refusals(
    stmts: &[Stmt],
    interner: &Interner,
    types: Option<&crate::analysis::TypeRegistry>,
) -> Vec<Refusal> {
    let mounts = mount_refusals(stmts, interner, types);
    let records = record_names(stmts, types);
    let routes = route_refusal_map(stmts, interner, &|sym| records.contains(&sym));

    fn visit(
        s: &Stmt,
        interner: &Interner,
        mounts: &HashMap<Symbol, Refusal>,
        routes: &HashMap<(Symbol, Symbol), Refusal>,
        out: &mut Vec<Refusal>,
    ) {
        let found = match s {
            Stmt::ComponentDef(def) => error_page_refusal(def, interner),
            Stmt::Ui(crate::ast::stmt::UiStmt::Mount { component, .. }) => {
                mounts.get(component).cloned()
            }
            Stmt::RouteDef { method, path, .. } => routes.get(&(*method, *path)).cloned(),
            _ => None,
        };
        if let Some(r) = found {
            if !out.contains(&r) {
                out.push(r);
            }
        }
        if let Stmt::FunctionDef { body, .. } = s {
            for st in body.iter() {
                visit(st, interner, mounts, routes, out);
            }
        }
        visit_stmt_exprs(s, &mut |e| {
            if let Expr::Closure {
                body: crate::ast::stmt::ClosureBody::Block(block),
                ..
            } = e
            {
                for st in block.iter() {
                    visit(st, interner, mounts, routes, out);
                }
            }
        });
        for_each_child_block(s, &mut |block| {
            for st in block {
                visit(st, interner, mounts, routes, out);
            }
        });
    }

    let mut out = Vec::new();
    // Main's statements compile before any function body, so their refusals come first.
    for s in stmts
        .iter()
        .filter(|s| !matches!(s, Stmt::FunctionDef { .. }))
    {
        visit(s, interner, &mounts, &routes, &mut out);
    }
    for s in stmts
        .iter()
        .filter(|s| matches!(s, Stmt::FunctionDef { .. }))
    {
        visit(s, interner, &mounts, &routes, &mut out);
    }
    out
}

/// Collect identifiers that appear inside function or closure bodies — the
/// names that, when bound at Main top level, must live in the globals table.
/// `at_main` is true while walking Main-level statements (whose own
/// identifiers are frame-local); inside a FunctionDef body or a Closure body
/// every identifier counts.
fn collect_nonlocal_idents_stmt(
    s: &Stmt,
    at_main: bool,
    out: &mut std::collections::HashSet<Symbol>,
) {
    use crate::ast::stmt::ClosureBody;
    use crate::interpreter::Interpreter;

    if let Stmt::FunctionDef { params, body, .. } = s {
        // Everything free in a function body is a nonlocal reference.
        let free = Interpreter::free_vars_in_closure(params, &ClosureBody::Block(body));
        out.extend(free);
        return;
    }
    // Walk this statement's expressions for Closure literals (whose free vars
    // are nonlocal) and recurse into nested statements.
    visit_stmt_exprs(s, &mut |e| {
        if let Expr::Closure { params, body, .. } = e {
            let free = Interpreter::free_vars_in_closure(params, body);
            out.extend(free);
        }
    });
    for_each_child_block(s, &mut |block| {
        for st in block {
            collect_nonlocal_idents_stmt(st, at_main, out);
        }
    });
}

/// Apply `f` to every expression directly held by `s` (and, via the walker in
/// `f` itself when needed, their subexpressions are reached by the free-vars
/// collector — Closure bodies included).
fn visit_stmt_exprs(s: &Stmt, f: &mut dyn FnMut(&Expr)) {
    // Iterative walk: expressions can be arbitrarily deep (degenerate inputs),
    // so no native recursion here.
    fn walk_expr<'a>(root: &'a Expr<'a>, f: &mut dyn FnMut(&Expr)) {
        let mut stack: Vec<&Expr> = vec![root];
        while let Some(e) = stack.pop() {
            f(e);
            match e {
                Expr::BinaryOp { left, right, .. } => {
                    stack.push(left);
                    stack.push(right);
                }
                Expr::Not { operand } => stack.push(operand),
                Expr::Call { args, .. } => stack.extend(args.iter().copied()),
                Expr::CallExpr { callee, args } => {
                    stack.push(callee);
                    stack.extend(args.iter().copied());
                }
                Expr::Index { collection, index } => {
                    stack.push(collection);
                    stack.push(index);
                }
                Expr::Slice {
                    collection,
                    start,
                    end,
                } => {
                    stack.push(collection);
                    stack.push(start);
                    stack.push(end);
                }
                Expr::Copy { expr } => stack.push(expr),
                Expr::Give { value } => stack.push(value),
                Expr::Length { collection } => stack.push(collection),
                Expr::Contains { collection, value } => {
                    stack.push(collection);
                    stack.push(value);
                }
                Expr::Union { left, right } | Expr::Intersection { left, right } => {
                    stack.push(left);
                    stack.push(right);
                }
                Expr::List(items) | Expr::Tuple(items) => stack.extend(items.iter().copied()),
                Expr::Range { start, end } => {
                    stack.push(start);
                    stack.push(end);
                }
                Expr::FieldAccess { object, .. } => stack.push(object),
                Expr::New { init_fields, .. } => {
                    stack.extend(init_fields.iter().map(|(_, fe)| *fe));
                }
                Expr::NewVariant { fields, .. } => {
                    stack.extend(fields.iter().map(|(_, fe)| *fe));
                }
                Expr::OptionSome { value } => stack.push(value),
                Expr::WithCapacity { value, .. } => stack.push(value),
                _ => {}
            }
        }
    }

    use crate::ast::stmt::ReadSource;
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => walk_expr(value, f),
        Stmt::Return { value: Some(e) } => walk_expr(e, f),
        Stmt::Call { args, .. } => {
            for a in args {
                walk_expr(a, f);
            }
        }
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => walk_expr(cond, f),
        Stmt::Repeat { iterable, .. } => walk_expr(iterable, f),
        Stmt::Show { object, .. } | Stmt::Give { object, .. } => walk_expr(object, f),
        Stmt::Push { value, collection }
        | Stmt::Add { value, collection }
        | Stmt::Remove { value, collection } => {
            walk_expr(value, f);
            walk_expr(collection, f);
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            walk_expr(collection, f);
            walk_expr(index, f);
            walk_expr(value, f);
        }
        Stmt::SetField { object, value, .. } => {
            walk_expr(object, f);
            walk_expr(value, f);
        }
        Stmt::Inspect { target, .. } => walk_expr(target, f),
        Stmt::RuntimeAssert {
            condition, reason, ..
        } => {
            walk_expr(condition, f);
            if let Some(r) = reason {
                walk_expr(r, f);
            }
        }
        Stmt::Fail { reason } => walk_expr(reason, f),
        Stmt::Sleep { milliseconds } => walk_expr(milliseconds, f),
        Stmt::IncreaseCrdt { amount, .. } | Stmt::DecreaseCrdt { amount, .. } => {
            walk_expr(amount, f)
        }
        Stmt::MergeCrdt { source, .. } | Stmt::JoinCrdt { source, .. } => walk_expr(source, f),
        Stmt::WriteFile { content, path } => {
            walk_expr(content, f);
            walk_expr(path, f);
        }
        Stmt::ReadFrom {
            source: ReadSource::File(p),
            ..
        } => walk_expr(p, f),
        _ => {}
    }
}

/// Apply `f` to every nested statement block of `s`.
fn for_each_child_block<'a>(s: &Stmt<'a>, f: &mut dyn FnMut(&[Stmt<'a>])) {
    match s {
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            f(then_block);
            if let Some(eb) = else_block {
                f(eb);
            }
        }
        Stmt::While { body, .. }
        | Stmt::Repeat { body, .. }
        | Stmt::Zone { body, .. }
        | Stmt::Reversibly { body } => f(body),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => f(tasks),
        Stmt::Inspect { arms, .. } => {
            for arm in arms {
                f(arm.body);
            }
        }
        _ => {}
    }
}

/// Rewrite the placeholder target of the jump at `idx`. Total: a non-jump op or
/// an out-of-bounds index is a compiler-internal bug surfaced as `Err`.
pub(super) fn patch_jump(code: &mut [Op], idx: usize, target: usize) -> Result<(), String> {
    match code.get_mut(idx) {
        Some(Op::Jump { target: t })
        | Some(Op::JumpIfFalse { target: t, .. })
        | Some(Op::JumpIfTrue { target: t, .. })
        | Some(Op::IterNext { exit: t, .. }) => {
            *t = target;
            Ok(())
        }
        Some(other) => Err(format!("vm: patch_jump on non-jump op {:?}", other)),
        None => Err(format!("vm: patch_jump index {} out of bounds", idx)),
    }
}

fn literal_const(lit: &Literal) -> Result<Constant, String> {
    match lit {
        Literal::Number(n) => Ok(Constant::Int(*n)),
        Literal::Float(f) => Ok(Constant::Float(*f)),
        Literal::Boolean(b) => Ok(Constant::Bool(*b)),
        Literal::Nothing => Ok(Constant::Nothing),
        Literal::Char(c) => Ok(Constant::Char(*c)),
        Literal::Duration(nanos) => Ok(Constant::Duration(*nanos)),
        Literal::Date(days) => Ok(Constant::Date(*days)),
        Literal::Moment(nanos) => Ok(Constant::Moment(*nanos)),
        Literal::Span { months, days } => Ok(Constant::Span {
            months: *months,
            days: *days,
        }),
        Literal::Time(nanos) => Ok(Constant::Time(*nanos)),
        Literal::Text(_) => unreachable!("Text literals are interned and handled by the caller"),
    }
}

/// Match `value` as the left-spine `target + e1 + … + ek` (k ≥ 1), the
/// accumulate shape `Set x to x + …` lowers to an [`Op::AddAssign`] chain —
/// the exact left-fold the expression denotes: same kernel calls, same
/// evaluation order, same error points, but folding into the target register
/// at each step so a sole-owner Text can extend in place.
///
/// Soundness: the first term evaluates before any fold touches the register,
/// so it may mention the target freely. Every LATER term runs after a fold —
/// it must provably never observe the target ([`expr_avoids`]), or the chain
/// would read a partially-folded value the original expression never exposes.
fn add_assign_chain<'a>(target: Symbol, value: &'a Expr<'a>) -> Option<Vec<&'a Expr<'a>>> {
    let mut terms: Vec<&Expr> = Vec::new();
    let mut cur = value;
    loop {
        match cur {
            Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left,
                right,
            } => {
                terms.push(right);
                cur = left;
            }
            Expr::Identifier(sym) if *sym == target => break,
            _ => return None,
        }
    }
    if terms.is_empty() {
        return None;
    }
    terms.reverse();
    if terms[1..].iter().all(|t| expr_avoids(t, target)) {
        Some(terms)
    } else {
        None
    }
}

/// True when `e` provably never observes `sym`: every node comes from a
/// closed set of value-only forms with no `Identifier(sym)` anywhere.
/// Closures are conservatively "may observe" — they snapshot captures at
/// evaluation, and under the AddAssign rewrite that snapshot would see the
/// partially-folded register. Named calls cannot read the caller's locals
/// (and the rewrite only fires for locals), so only their argument
/// expressions need scanning. Anything unrecognized fails closed.
fn expr_avoids(e: &Expr, sym: Symbol) -> bool {
    use crate::ast::stmt::StringPart;
    let mut stack: Vec<&Expr> = vec![e];
    while let Some(e) = stack.pop() {
        match e {
            Expr::Identifier(s) => {
                if *s == sym {
                    return false;
                }
            }
            Expr::Literal(_) | Expr::OptionNone => {}
            Expr::BinaryOp { left, right, .. }
            | Expr::Union { left, right }
            | Expr::Intersection { left, right }
            | Expr::Range {
                start: left,
                end: right,
            } => {
                stack.push(left);
                stack.push(right);
            }
            Expr::Not { operand } => stack.push(operand),
            Expr::Call { args, .. } => stack.extend(args.iter().copied()),
            Expr::CallExpr { callee, args } => {
                stack.push(callee);
                stack.extend(args.iter().copied());
            }
            Expr::Index { collection, index } => {
                stack.push(collection);
                stack.push(index);
            }
            Expr::Slice {
                collection,
                start,
                end,
            } => {
                stack.push(collection);
                stack.push(start);
                stack.push(end);
            }
            Expr::Copy { expr } => stack.push(expr),
            Expr::Length { collection } => stack.push(collection),
            Expr::Contains { collection, value } => {
                stack.push(collection);
                stack.push(value);
            }
            Expr::List(items) | Expr::Tuple(items) => stack.extend(items.iter().copied()),
            Expr::FieldAccess { object, .. } => stack.push(object),
            Expr::New { init_fields, .. } => {
                stack.extend(init_fields.iter().map(|(_, fe)| *fe));
            }
            Expr::NewVariant { fields, .. } => {
                stack.extend(fields.iter().map(|(_, fe)| *fe));
            }
            Expr::OptionSome { value } => stack.push(value),
            Expr::WithCapacity { value, capacity } => {
                stack.push(value);
                stack.push(capacity);
            }
            Expr::InterpolatedString(parts) => {
                for p in parts {
                    if let StringPart::Expr { value, .. } = p {
                        stack.push(value);
                    }
                }
            }
            _ => return false,
        }
    }
    true
}

fn binop_op(op: BinaryOpKind, dst: Reg, lhs: Reg, rhs: Reg) -> Result<Op, String> {
    use BinaryOpKind::*;
    Ok(match op {
        Add => Op::Add { dst, lhs, rhs },
        Subtract => Op::Sub { dst, lhs, rhs },
        Multiply => Op::Mul { dst, lhs, rhs },
        Divide => Op::Div { dst, lhs, rhs },
        ExactDivide => Op::ExactDiv { dst, lhs, rhs },
        FloorDivide => Op::FloorDiv { dst, lhs, rhs },
        Modulo => Op::Mod { dst, lhs, rhs },
        Lt => Op::Lt { dst, lhs, rhs },
        Gt => Op::Gt { dst, lhs, rhs },
        LtEq => Op::LtEq { dst, lhs, rhs },
        GtEq => Op::GtEq { dst, lhs, rhs },
        Eq => Op::Eq { dst, lhs, rhs },
        ApproxEq => Op::ApproxEq { dst, lhs, rhs },
        NotEq => Op::NotEq { dst, lhs, rhs },
        Concat => Op::Concat { dst, lhs, rhs },
        SeqConcat => Op::SeqConcat { dst, lhs, rhs },
        Pow => Op::Pow { dst, lhs, rhs },
        BitXor => Op::BitXor { dst, lhs, rhs },
        BitAnd => Op::BitAnd { dst, lhs, rhs },
        BitOr => Op::BitOr { dst, lhs, rhs },
        Shl => Op::Shl { dst, lhs, rhs },
        Shr => Op::Shr { dst, lhs, rhs },
        // And/Or are compiled by `compile_short_circuit`, never through here.
        And | Or => return Err("vm: internal — And/Or must use compile_short_circuit".to_string()),
    })
}
