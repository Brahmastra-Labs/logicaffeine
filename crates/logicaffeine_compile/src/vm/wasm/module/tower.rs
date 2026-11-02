//! The exact extension in a linked module.
//!
//! A value the extension reaches — an infinitesimal, a hyperreal, an algebraic number, a computable real, an
//! ordinal, a residue-dependent or provisional answer, and every list of them — is a handle into the linked value
//! runtime ([`Kind::Tower`]): `logicaffeine_data`'s C ABI, the one the C backends link, so a module computes, prints,
//! compares and fails with the compiler's own values. Which registers hold one is the kind inference's
//! ([`crate::vm::tower`]). A register that holds one holds every value as one: an op that computes a value of
//! another kind into it computes into a register of that kind first, its *twin*, and the value lands converted; a
//! loop whose variable holds one walks the runtime's values of its items.
//!
//! A handle belongs to the one place that holds it. A copy into a second place — a register, a field, a global, a
//! parameter — is a copy of the value (an `Arc` count in the runtime), so a list changed in place through one place
//! changes no other. A self-contained module has no runtime to hold one: it refuses a program the extension reaches
//! by name ([`REFUSAL`]).

use std::collections::{HashMap, HashSet};

use super::*;
use crate::semantics::collections::IntegerPlace;
use crate::vm::instruction::{visit_regs, Reg, RegVisitor, Role};

/// Why a self-contained module refuses a program the exact extension reaches.
pub(super) const REFUSAL: &str = "program the exact extension reaches: a self-contained module does not run the \
     exact extension, whose values live in the value runtime that `wasm-linked` links";

/// Why a self-contained module refuses a program that declares a list of rationals.
pub(super) const RATIONAL_LISTS: &str = "list of rationals: a self-contained module holds no list of exact numbers; \
     the value runtime that `wasm-linked` links holds one, each item in the form the VM keeps it";

/// Every import a module holding a value of the extension may call: the value ABI and its adapters.
const IMPORTS: &[HostFn] = &[
    HostFn::ValueInt,
    HostFn::ValueFloat,
    HostFn::ValueBool,
    HostFn::ValueNothing,
    HostFn::ValueDate,
    HostFn::ValueMoment,
    HostFn::ValueTime,
    HostFn::ValueSpan,
    HostFn::ValueList,
    HostFn::ValueClone,
    HostFn::ValuePush,
    HostFn::ValueSetItem,
    HostFn::ValuePop,
    HostFn::ValueItems,
    HostFn::ValueItemAt,
    HostFn::ValueSlice,
    HostFn::ValueAdd,
    HostFn::ValueSub,
    HostFn::ValueMul,
    HostFn::ValueDiv,
    HostFn::ValueExactDiv,
    HostFn::ValueFloorDiv,
    HostFn::ValueRem,
    HostFn::ValuePow,
    HostFn::ValueApproxEq,
    HostFn::ValueSeqConcat,
    HostFn::ValueBitAnd,
    HostFn::ValueBitOr,
    HostFn::ValueBitXor,
    HostFn::ValueShl,
    HostFn::ValueShr,
    HostFn::ValueMin,
    HostFn::ValueMax,
    HostFn::ValuePower,
    HostFn::ValueItem,
    HostFn::ValueEq,
    HostFn::ValueNe,
    HostFn::ValueLt,
    HostFn::ValueGt,
    HostFn::ValueLe,
    HostFn::ValueGe,
    HostFn::ValueNot,
    HostFn::ValueAbs,
    HostFn::ValueSqrt,
    HostFn::ValueFloor,
    HostFn::ValueCeil,
    HostFn::ValueRound,
    HostFn::ValueChecked,
    HostFn::ValueToText,
    HostFn::ValueOfText,
    HostFn::ValueOfBigint,
    HostFn::ValueOfRational,
    HostFn::ValueOfDecimal,
    HostFn::ValueLengthI64,
    HostFn::ValueTruthI32,
    HostFn::ValueContainsI64,
    HostFn::ValueToI64,
    HostFn::ValueIntegerAt,
    HostFn::ValueApply,
    HostFn::ValueTupleOf,
    HostFn::PrintText,
];

/// The import of a runtime operation, by the name [`crate::vm::tower`] gives it.
fn operation(name: &str) -> R<HostFn> {
    Ok(match name {
        "add" => HostFn::ValueAdd,
        "sub" => HostFn::ValueSub,
        "mul" => HostFn::ValueMul,
        "div" => HostFn::ValueDiv,
        "exact_div" => HostFn::ValueExactDiv,
        "floor_div" => HostFn::ValueFloorDiv,
        "rem" => HostFn::ValueRem,
        "pow" => HostFn::ValuePow,
        "approx_eq" => HostFn::ValueApproxEq,
        "seq_concat" => HostFn::ValueSeqConcat,
        "bit_and" => HostFn::ValueBitAnd,
        "bit_or" => HostFn::ValueBitOr,
        "bit_xor" => HostFn::ValueBitXor,
        "shl" => HostFn::ValueShl,
        "shr" => HostFn::ValueShr,
        "min" => HostFn::ValueMin,
        "max" => HostFn::ValueMax,
        "power" => HostFn::ValuePower,
        "eq" => HostFn::ValueEq,
        "ne" => HostFn::ValueNe,
        "lt" => HostFn::ValueLt,
        "gt" => HostFn::ValueGt,
        "le" => HostFn::ValueLe,
        "ge" => HostFn::ValueGe,
        "floor" => HostFn::ValueFloor,
        "ceil" => HostFn::ValueCeil,
        "round" => HostFn::ValueRound,
        "abs" => HostFn::ValueAbs,
        "sqrt" => HostFn::ValueSqrt,
        other => {
            return Err(WasmLowerError::UnsupportedOwned(format!(
                "runtime operation `{other}` over an exact value"
            )))
        }
    })
}

/// What a plan does with values of the extension: whether it holds one at all, the twin each definition the
/// runtime does not compute writes instead of its register, the machine integer each read the module computes on
/// words narrows to, and the loops whose variable holds one over a list of the module's own.
#[derive(Debug, Default, Clone)]
pub(super) struct Towers {
    holds: bool,
    /// Per pc: the register the op writes in place of `dst`, and `dst`.
    twins: HashMap<usize, (u16, u16)>,
    /// Per pc: each register the op reads as a machine integer that holds a value of the extension or a rational,
    /// the `Int` register it reads instead, and how the value is read as one.
    narrows: HashMap<usize, Vec<Narrow>>,
    /// The `IterPrepare`s (by pc) whose loop variable holds values of the extension over a list of the module's own.
    frames: HashSet<usize>,
}

/// A register an op reads as a machine integer, read into an `Int` register first.
#[derive(Debug, Clone, Copy)]
struct Narrow {
    read: u16,
    word: u16,
    how: Read,
}

/// How a value is read as a machine integer: a value of the extension as a map's key, or at a place the VM takes a
/// machine integer, refused in its words there; a rational at such a place.
#[derive(Debug, Clone, Copy)]
enum Read {
    Key,
    At(IntegerPlace),
    Rational(IntegerPlace),
}

impl Towers {
    pub(super) fn holds(&self) -> bool {
        self.holds
    }
}

/// The i32 scratch locals a plan holding a value of the extension declares: two depths of a list conversion
/// (source, list, index, length, data), the list an op builds, and a loop frame's frame, index, length and items.
const LOCALS: u32 = 15;
const OP_LIST: u32 = 10;
const FRAME: u32 = 11;
const FRAME_INDEX: u32 = 12;
const FRAME_COUNT: u32 = 13;
const FRAME_ITEMS: u32 = 14;

pub(super) fn extra_locals(plan: &Plan) -> u32 {
    if plan.tower.holds {
        LOCALS
    } else {
        0
    }
}

fn local(plan: &Plan, at: u32) -> u32 {
    plan.num_regs + 25 + exact::extra_locals(plan) + absence::extra_locals(plan) + at
}

/// Settle what `plan` does with values of the extension and with rationals, from its inferred kinds: each definition
/// the runtime does not compute into a register holding one gets a twin register of the kind it makes, and the op
/// writes the twin; each read the module computes on machine words gets an `Int` register narrowed from the value or
/// the rational; each loop whose variable holds one over a list of the module's own builds its frame of the
/// runtime's values. A self-contained module has no runtime to read a rational as an integer by.
pub(super) fn settle(plan: &mut Plan, program: &CompiledProgram, linked: bool) -> R<()> {
    let functions = &program.functions;
    let kind = |plan: &Plan, r: u16| plan.kinds.get(r as usize);
    let towers = plan.kinds.holds(Kind::Tower);
    if !towers && !plan.kinds.holds(Kind::Rational) {
        return Ok(());
    }
    let prepares: Vec<usize> = (0..plan.ops.len())
        .filter(|pc| matches!(plan.ops[*pc], Op::IterPrepare { .. }))
        .collect();
    let mut loops = 0usize;
    for pc in 0..plan.ops.len() {
        if let Op::IterNext { dst, .. } = plan.ops[pc] {
            let prepare = prepares.get(loops).copied();
            loops += 1;
            let walks_own = plan
                .iter_iterable
                .get(pc)
                .copied()
                .flatten()
                .is_some_and(|r| kind(plan, r as u16) != Some(Kind::Tower));
            if kind(plan, dst) == Some(Kind::Tower) && walks_own {
                plan.tower.frames.insert(prepare.ok_or(WasmLowerError::Unsupported(
                    "a loop over exact values with no `Repeat` to start it",
                ))?);
            }
            continue;
        }
        if let Some(natural) = plan.kinds.natural(pc) {
            let mut written = Vec::new();
            plan.ops[pc].for_each_written_reg(|w| written.push(w));
            let [dst] = written[..] else { continue };
            let twin = plan.num_regs as u16;
            if !rename(&mut plan.ops[pc], functions, dst, twin, Role::Def) {
                return Err(WasmLowerError::UnsupportedOwned(format!(
                    "{:?} that reads the register it writes, an exact value, as a {natural:?}",
                    plan.ops[pc]
                )));
            }
            plan.num_regs += 1;
            plan.kinds = std::mem::replace(&mut plan.kinds, KindTable::empty())
                .with_register(twin as usize, natural);
            plan.tower.twins.insert(pc, (twin, dst));
        }
        let mut narrowed: Vec<(u16, Read)> = Vec::new();
        let mut at = |r: u16, place: IntegerPlace| match kind(plan, r) {
            Some(Kind::Tower) => narrowed.push((r, Read::At(place))),
            Some(Kind::Rational) => narrowed.push((r, Read::Rational(place))),
            _ => {}
        };
        let keyed = |c: u16| matches!(kind(plan, c), Some(Kind::Map | Kind::Struct));
        match plan.ops[pc] {
            Op::Index {
                collection, index, ..
            }
            | Op::IndexUnchecked {
                collection, index, ..
            } if kind(plan, collection) != Some(Kind::Tower) && !keyed(collection) => {
                let place = match kind(plan, collection) {
                    Some(Kind::Text) => IntegerPlace::Text,
                    Some(Kind::Tuple) => IntegerPlace::Tuple,
                    _ => IntegerPlace::ListRead,
                };
                at(index, place)
            }
            Op::SetIndex {
                collection, index, ..
            }
            | Op::SetIndexUnchecked {
                collection, index, ..
            } if kind(plan, collection) != Some(Kind::Tower) && !keyed(collection) => {
                at(index, IntegerPlace::ListStore)
            }
            Op::Index {
                collection, index, ..
            }
            | Op::IndexUnchecked {
                collection, index, ..
            }
            | Op::SetIndex {
                collection, index, ..
            }
            | Op::SetIndexUnchecked {
                collection, index, ..
            } if keyed(collection) && kind(plan, index) == Some(Kind::Tower) => narrowed.push((index, Read::Key)),
            Op::NewRange { start, end, .. } => {
                at(start, IntegerPlace::Range);
                at(end, IntegerPlace::Range);
            }
            Op::SliceOp {
                collection, start, end, ..
            } if kind(plan, collection) != Some(Kind::Tower) => {
                at(start, IntegerPlace::Slice);
                at(end, IntegerPlace::Slice);
            }
            _ => {}
        }
        for (read, how) in narrowed {
            if matches!(how, Read::Rational(_)) && !linked {
                return Err(WasmLowerError::Unsupported(
                    "a rational read as a machine integer: a self-contained module has no runtime to read it by",
                ));
            }
            let word = plan.num_regs as u16;
            if !rename(&mut plan.ops[pc], functions, read, word, Role::Use) {
                return Err(WasmLowerError::UnsupportedOwned(format!(
                    "{:?} reading an exact value as a machine integer",
                    plan.ops[pc]
                )));
            }
            plan.num_regs += 1;
            plan.kinds = std::mem::replace(&mut plan.kinds, KindTable::empty())
                .with_register(word as usize, Kind::Int);
            plan.tower.narrows.entry(pc).or_default().push(Narrow { read, word, how });
        }
    }
    Ok(())
}

/// Decide whether the program holds a value of the value runtime anywhere, and stamp every planned region with the
/// answer: it reaches the extension, or a region holds one in a register or returns one, a global holds one, or a
/// record declares a field of one (a list of rationals, in a linked module). Every region of such a program can hand
/// the runtime a value (an argument for a parameter that holds one, a value for such a global or field), so every
/// one imports the runtime and declares the conversion locals.
pub(super) fn decide(program: &CompiledProgram, main: &mut Plan, fns: &mut [Plan], globals: &[Option<Kind>]) -> bool {
    let tower = Some(Kind::Tower);
    let held = |p: &Plan| !p.stub && (p.kinds.holds(Kind::Tower) || p.result == tower);
    let holds = crate::vm::tower::reaches(program)
        || held(main)
        || fns.iter().any(held)
        || globals.contains(&tower)
        || program
            .struct_types
            .iter()
            .any(|s| s.fields.iter().any(|(_, t)| kind::boundary_to_kind(t) == tower));
    for plan in std::iter::once(main).chain(fns.iter_mut()) {
        plan.tower.holds = holds && !plan.stub;
    }
    holds
}

/// Rename the one operand of `op` that is register `from` in `role` to `to`; `false` when `from` is not such an
/// operand alone (it is also read where it is written, or sits in a register block).
fn rename(op: &mut Op, functions: &[CompiledFunction], from: u16, to: u16, role: Role) -> bool {
    struct Rename {
        from: Reg,
        to: Reg,
        role: Role,
        renamed: usize,
        blocked: bool,
    }
    impl RegVisitor for Rename {
        fn scalar(&mut self, r: &mut Reg, role: Role) {
            if *r != self.from {
                return;
            }
            if role == self.role {
                *r = self.to;
                self.renamed += 1;
            } else {
                self.blocked = true;
            }
        }
        fn range(&mut self, start: &mut Reg, count: u16, _role: Role) {
            if (*start..start.wrapping_add(count)).contains(&self.from) {
                self.blocked = true;
            }
        }
    }
    let mut v = Rename {
        from,
        to,
        role,
        renamed: 0,
        blocked: false,
    };
    let mut renamed = op.clone();
    visit_regs(&mut renamed, functions, &mut v);
    if v.blocked || v.renamed != 1 {
        return false;
    }
    *op = renamed;
    true
}

/// Note every import of the value runtime for a module whose program holds one of its values, and the reading of a
/// rational as a machine integer for one that reads one so.
pub(super) fn note_imports<'p>(plans: impl IntoIterator<Item = &'p Plan> + Clone, used: &mut Vec<HostFn>) {
    let mut note = |h: HostFn| {
        if !used.contains(&h) {
            used.push(h);
        }
    };
    if plans.clone().into_iter().any(|p| p.tower.holds) {
        for h in IMPORTS {
            note(*h);
        }
    }
    if plans.clone().into_iter().any(defines_reals) {
        note(HostFn::ValueDefine);
    }
    let reads_rationals = plans
        .into_iter()
        .flat_map(|p| p.tower.narrows.values().flatten())
        .any(|n| matches!(n.how, Read::Rational(_)));
    if reads_rationals {
        note(HostFn::RationalAsI64);
    }
}

/// The symbol of a linked module's reader of the closures it defines real numbers by, which the value runtime's
/// `logos_rt_value_define` calls back.
pub(super) const DEFINER_SYMBOL: &str = "logos_program_defines";

/// The reader the linked value runtime calls back for a closure the program defines a real number by:
/// `logos_program_defines(body: i32, n: i64) -> i32`, the closure body `body` called at the place `n` the runtime
/// asks for, as a `CallValue` calls a closure of no captures, and its answer handed back as a handle to the
/// runtime's value of it. A body of another shape — not one `Int` in, or no number out — traps.
pub(super) fn emit_defining_reader(fns: &[Plan], ctx: &Ctx) -> R<Vec<u8>> {
    let n = fns.len();
    let mut body = vec![0x01, 0x02, I64]; // the exact argument and result helpers' two i64 locals
    for _ in 0..=n {
        body.extend([0x02, 0x40]);
    }
    local_get(&mut body, 0);
    body.push(0x0E); // br_table over the body index
    leb_u32(&mut body, n as u32);
    for i in 0..n as u32 {
        leb_u32(&mut body, i);
    }
    leb_u32(&mut body, n as u32);
    for (fi, plan) in fns.iter().enumerate() {
        body.push(0x0B); // the arm of body fi
        let f = &ctx.functions[fi];
        let answers = plan.result.filter(|k| matches!(k, Kind::Int | Kind::BigInt | Kind::Rational | Kind::Tower));
        let (Some(answers), true) = (answers, f.param_count == 1 && f.captures.is_empty() && plan.kinds.valtype(0) == I64)
        else {
            body.push(0x00);
            continue;
        };
        local_get(&mut body, 1);
        exact::trampoline_argument(&mut body, ctx, fi, 2)?;
        body.push(0x10);
        leb_u32(&mut body, ctx.fn_base + fi as u32);
        match answers {
            Kind::Int => {
                exact::trampoline_result(&mut body, ctx, fi, 2, 3)?;
                call(&mut body, ctx, HostFn::ValueInt)?;
            }
            Kind::BigInt => call_checked(&mut body, ctx, HostFn::ValueOfBigint)?,
            Kind::Rational => call_checked(&mut body, ctx, HostFn::ValueOfRational)?,
            _ => {}
        }
        body.push(0x0F); // return
    }
    body.push(0x0B); // the default: no closure body
    body.push(0x00);
    body.push(0x0B);
    Ok(body)
}

/// Whether a plan defines a real number by a closure (`definedReal`).
pub(super) fn defines_reals(plan: &Plan) -> bool {
    plan.ops.iter().any(|op| {
        matches!(
            op,
            Op::CallBuiltin {
                builtin: crate::semantics::builtins::BuiltinId::DefinedReal,
                ..
            }
        )
    })
}

fn import(ctx: &Ctx, h: HostFn) -> R<u32> {
    (ctx.host_index)(h).ok_or(WasmLowerError::Unsupported(
        "a value-runtime import the module did not note",
    ))
}

fn call(code: &mut Vec<u8>, ctx: &Ctx, h: HostFn) -> R<()> {
    code.push(0x10);
    leb_u32(code, import(ctx, h)?);
    Ok(())
}

/// Call `h`, which answers a handle or a status, and stop the program with the runtime's sentence when it has none.
fn call_checked(code: &mut Vec<u8>, ctx: &Ctx, h: HostFn) -> R<()> {
    call(code, ctx, h)?;
    call(code, ctx, HostFn::ValueChecked)
}

/// A handle to a copy of the value register `reg` holds: the runtime's value of it, for a register of another kind.
pub(super) fn push_owned(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, reg: u16) -> R<()> {
    let kind = plan
        .kinds
        .get(reg as usize)
        .ok_or(WasmLowerError::Unsupported("an exact value made from a value of unknown kind"))?;
    push_value(code, plan, ctx, kind, 0, &|c| local_get(c, u32::from(reg)))?;
    if kind == Kind::Tower {
        call_checked(code, ctx, HostFn::ValueClone)?;
    }
    Ok(())
}

/// A handle to the value of register `reg` the runtime reads: its own handle, or the runtime's value of it.
fn push_read(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, reg: u16) -> R<()> {
    let kind = plan
        .kinds
        .get(reg as usize)
        .ok_or(WasmLowerError::Unsupported("an exact value made from a value of unknown kind"))?;
    push_value(code, plan, ctx, kind, 0, &|c| local_get(c, u32::from(reg)))
}

/// Leave a handle to the runtime's value of what `load` pushes, a value of `kind` in the module's representation;
/// a value of the extension's own handle as it is. A list converts item by item at conversion `depth`.
fn push_value(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    kind: Kind,
    depth: u32,
    load: &dyn Fn(&mut Vec<u8>),
) -> R<()> {
    match kind {
        Kind::Tower => load(code),
        Kind::Nothing => call(code, ctx, HostFn::ValueNothing)?,
        Kind::Int => {
            load(code);
            call(code, ctx, HostFn::ValueInt)?;
        }
        Kind::Bool => {
            load(code);
            code.push(0xA7); // i32.wrap_i64
            call(code, ctx, HostFn::ValueBool)?;
        }
        Kind::Float => {
            load(code);
            call(code, ctx, HostFn::ValueFloat)?;
        }
        Kind::Date => {
            load(code);
            call(code, ctx, HostFn::ValueDate)?;
        }
        Kind::Moment => {
            load(code);
            call(code, ctx, HostFn::ValueMoment)?;
        }
        Kind::Time => {
            load(code);
            call(code, ctx, HostFn::ValueTime)?;
        }
        Kind::Span => {
            // months in the high word, days in the low
            load(code);
            code.push(0x42); // i64.const 32
            leb_i64(code, 32);
            code.push(0x87); // i64.shr_s
            code.push(0xA7); // i32.wrap_i64
            load(code);
            code.push(0xA7);
            call(code, ctx, HostFn::ValueSpan)?;
        }
        Kind::Text => {
            load(code);
            call(code, ctx, HostFn::ValueOfText)?;
        }
        Kind::Rational => {
            load(code);
            call_checked(code, ctx, HostFn::ValueOfRational)?;
        }
        Kind::BigInt => {
            load(code);
            call_checked(code, ctx, HostFn::ValueOfBigint)?;
        }
        Kind::Decimal => {
            load(code);
            call_checked(code, ctx, HostFn::ValueOfDecimal)?;
        }
        Kind::SeqAny => {
            i32_const(code, 0);
            i32_const(code, 0);
            call_checked(code, ctx, HostFn::ValueList)?;
        }
        Kind::SeqInt | Kind::SeqBool | Kind::SeqFloat | Kind::SeqText | Kind::SeqSeqInt => {
            if depth > 1 {
                return Err(WasmLowerError::Unsupported(
                    "an exact value made from a list nested past two levels",
                ));
            }
            let at = |i: u32| local(plan, depth * 5 + i);
            let (src, list, index, count, data) = (at(0), at(1), at(2), at(3), at(4));
            let elem = kind.seq_elem().expect("a list kind has an item kind");
            load(code);
            local_set(code, src);
            i32_const(code, 0);
            i32_const(code, 0);
            call_checked(code, ctx, HostFn::ValueList)?;
            local_set(code, list);
            local_get(code, src);
            i32_load(code, 0);
            local_set(code, count);
            local_get(code, src);
            i32_load(code, 8);
            local_set(code, data);
            i32_const(code, 0);
            local_set(code, index);
            code.extend([0x02, 0x40, 0x03, 0x40]); // block, loop
            local_get(code, index);
            local_get(code, count);
            code.push(0x4E); // i32.ge_s
            code.push(0x0D); // br_if $exit
            leb_u32(code, 1);
            local_get(code, list);
            let slot = move |c: &mut Vec<u8>| {
                local_get(c, data);
                local_get(c, index);
                i32_const(c, 8);
                c.push(0x6C); // i32.mul
                c.push(0x6A); // i32.add
                match elem.wasm_valtype() {
                    F64 => f64_load(c, 0),
                    I64 => i64_load(c, 0),
                    _ => i32_load(c, 0),
                }
            };
            push_value(code, plan, ctx, elem, depth + 1, &slot)?;
            call_checked(code, ctx, HostFn::ValuePush)?;
            code.push(0x1A); // drop the status
            local_get(code, index);
            i32_const(code, 1);
            code.push(0x6A);
            local_set(code, index);
            code.push(0x0C); // br $loop
            leb_u32(code, 0);
            code.extend([0x0B, 0x0B]);
            local_get(code, list);
        }
        other => {
            return Err(WasmLowerError::UnsupportedOwned(format!(
                "a {other:?} value where an exact value is held"
            )))
        }
    }
    Ok(())
}

/// A list of the runtime's values of registers `regs`, left as a handle (built in the op's own list local).
fn push_list(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, regs: impl IntoIterator<Item = u16>) -> R<()> {
    let list = local(plan, OP_LIST);
    i32_const(code, 0);
    i32_const(code, 0);
    call_checked(code, ctx, HostFn::ValueList)?;
    local_set(code, list);
    for r in regs {
        local_get(code, list);
        push_read(code, plan, ctx, r)?;
        call_checked(code, ctx, HostFn::ValuePush)?;
        code.push(0x1A); // drop the status
    }
    local_get(code, list);
    Ok(())
}

/// Lower the op at `pc` where it computes with a value of the extension, then hand everything else on; an op the
/// runtime does not compute into a register holding one writes its twin, whose value lands converted.
pub(super) fn lower(
    pc: usize,
    plan: &Plan,
    ctx: &Ctx,
    blocks: &Blocks,
    k: usize,
    code: &mut Vec<u8>,
) -> R<Flow> {
    for narrow in plan.tower.narrows.get(&pc).into_iter().flatten() {
        local_get(code, u32::from(narrow.read));
        match narrow.how {
            Read::Key => call(code, ctx, HostFn::ValueToI64)?,
            Read::At(place) => {
                i32_const(code, place.code());
                call(code, ctx, HostFn::ValueIntegerAt)?;
            }
            Read::Rational(place) => {
                i32_const(code, place.code());
                call(code, ctx, HostFn::RationalAsI64)?;
            }
        }
        local_set(code, u32::from(narrow.word));
    }
    if !plan.tower.holds {
        return absence::lower(pc, plan, ctx, blocks, k, code);
    }
    let flow = match computed(pc, plan, ctx, blocks, k, code)? {
        Some(flow) => flow,
        None => {
            refuse_unlowered(pc, plan, ctx)?;
            absence::lower(pc, plan, ctx, blocks, k, code)?
        }
    };
    if let Some(&(twin, dst)) = plan.tower.twins.get(&pc) {
        let natural = plan.kinds.get(twin as usize).expect("a twin has its kind");
        push_value(code, plan, ctx, natural, 0, &|c| local_get(c, u32::from(twin)))?;
        local_set(code, u32::from(dst));
    }
    Ok(flow)
}

/// An op the module's own lowering takes that reads a value of the extension where it needs one of the module's own
/// kinds is refused by name: the lowering would read the handle as that value.
fn refuse_unlowered(pc: usize, plan: &Plan, ctx: &Ctx) -> R<()> {
    let op = &plan.ops[pc];
    let mut reads_tower = false;
    let (_, uses) = crate::vm::instruction::op_def_uses(op, ctx.functions);
    for r in uses {
        reads_tower |= plan.kinds.get(r as usize) == Some(Kind::Tower);
    }
    let carries = matches!(
        op,
        Op::Move { .. }
            | Op::Call { .. }
            | Op::Return { .. }
            | Op::GlobalSet { .. }
            | Op::StructInsert { .. }
            | Op::NewTuple { .. }
            | Op::Concat { .. }
            | Op::FormatValue { .. }
            | Op::EnsureOwned { .. }
    ) || matches!(op, Op::Add { dst, .. } | Op::AddAssign { dst, .. } if plan.kinds.get(*dst as usize) == Some(Kind::Text))
        || matches!(op, Op::SetIndex { collection, .. } | Op::SetIndexUnchecked { collection, .. }
            if matches!(plan.kinds.get(*collection as usize), Some(Kind::Struct | Kind::Map)));
    if reads_tower && !carries {
        return Err(WasmLowerError::UnsupportedOwned(format!(
            "{op:?} over an exact value is not yet lowered"
        )));
    }
    Ok(())
}

/// The ops the runtime computes, and the ops whose operand or destination is a value of the extension in a way the
/// module's own lowering does not take: lowered here, `None` for everything else.
fn computed(
    pc: usize,
    plan: &Plan,
    ctx: &Ctx,
    blocks: &Blocks,
    k: usize,
    code: &mut Vec<u8>,
) -> R<Option<Flow>> {
    let t = |r: u16| plan.kinds.get(r as usize) == Some(Kind::Tower);
    let op = plan.ops[pc];
    let set = |code: &mut Vec<u8>, dst: u16| local_set(code, u32::from(dst));
    match op {
        Op::CallBuiltin {
            dst,
            builtin: crate::semantics::builtins::BuiltinId::DefinedReal,
            args_start,
            ..
        } => {
            let body = plan.structs.callee_func.get(pc).copied().flatten().ok_or(WasmLowerError::Unsupported(
                "a real number defined by a closure of unknown origin",
            ))?;
            if plan.kinds.get(args_start as usize + 1) != Some(Kind::Int) {
                return Err(WasmLowerError::Unsupported("a real number's base held as no Int"));
            }
            for r in args_start..args_start + 3 {
                local_get(code, u32::from(r));
            }
            i32_const(code, i32::from(body));
            call(code, ctx, HostFn::ValueDefine)?;
            set(code, dst);
        }
        Op::CallBuiltin {
            dst,
            builtin,
            args_start,
            arg_count,
        } if builtin.extension().is_some() => {
            let b = builtin.extension().expect("a builtin of the extension");
            let index = logicaffeine_base::extension::Builtin::ALL
                .iter()
                .position(|x| *x == b)
                .expect("every builtin of the extension is listed");
            i32_const(code, index as i32);
            push_list(code, plan, ctx, (0..arg_count).map(|i| args_start + i))?;
            call_checked(code, ctx, HostFn::ValueApply)?;
            set(code, dst);
        }
        Op::CallBuiltin {
            dst,
            builtin,
            args_start,
            arg_count,
        } if t(dst) && crate::vm::tower::lifts(builtin).is_some() => {
            let f = operation(crate::vm::tower::lifts(builtin).expect("a lifting builtin"))?;
            for i in 0..arg_count {
                push_read(code, plan, ctx, args_start + i)?;
            }
            call_checked(code, ctx, f)?;
            set(code, dst);
        }
        Op::AddAssign { dst, src } if t(dst) => {
            push_read(code, plan, ctx, dst)?;
            push_read(code, plan, ctx, src)?;
            call_checked(code, ctx, HostFn::ValueAdd)?;
            set(code, dst);
        }
        Op::Not { dst, src } if t(src) => {
            local_get(code, u32::from(src));
            call_checked(code, ctx, HostFn::ValueNot)?;
            set(code, dst);
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
        } if t(collection) => {
            local_get(code, u32::from(collection));
            push_read(code, plan, ctx, index)?;
            call_checked(code, ctx, HostFn::ValueItem)?;
            set(code, dst);
        }
        Op::NewList { dst, start, count } if t(dst) => {
            push_list(code, plan, ctx, (0..count).map(|i| start + i))?;
            set(code, dst);
        }
        Op::SliceOp {
            dst,
            collection,
            start,
            end,
        } if t(collection) => {
            local_get(code, u32::from(collection));
            push_read(code, plan, ctx, start)?;
            push_read(code, plan, ctx, end)?;
            call_checked(code, ctx, HostFn::ValueSlice)?;
            set(code, dst);
        }
        Op::NewTuple { dst, start, count } if t(dst) => {
            push_list(code, plan, ctx, (0..count).map(|i| start + i))?;
            call_checked(code, ctx, HostFn::ValueTupleOf)?;
            set(code, dst);
        }
        Op::ListPush { list, value } if t(list) => {
            local_get(code, u32::from(list));
            push_read(code, plan, ctx, value)?;
            call_checked(code, ctx, HostFn::ValuePush)?;
            code.push(0x1A);
        }
        Op::ListPop { list, dst } if t(list) => {
            local_get(code, u32::from(list));
            call_checked(code, ctx, HostFn::ValuePop)?;
            set(code, dst);
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
        } if t(collection) => {
            local_get(code, u32::from(collection));
            push_read(code, plan, ctx, index)?;
            push_read(code, plan, ctx, value)?;
            call_checked(code, ctx, HostFn::ValueSetItem)?;
            code.push(0x1A);
        }
        Op::Length { dst, collection } if t(collection) => {
            local_get(code, u32::from(collection));
            call(code, ctx, HostFn::ValueLengthI64)?;
            set(code, dst);
        }
        Op::Contains {
            dst,
            collection,
            value,
        } if t(collection) || t(value) => {
            push_read(code, plan, ctx, collection)?;
            push_read(code, plan, ctx, value)?;
            call(code, ctx, HostFn::ValueContainsI64)?;
            set(code, dst);
        }
        Op::Show { src } if t(src) => {
            local_get(code, u32::from(src));
            call(code, ctx, HostFn::ValueToText)?;
            call(code, ctx, HostFn::PrintText)?;
        }
        Op::JumpIfFalse { cond, target } | Op::JumpIfTrue { cond, target } if t(cond) => {
            let when_false = matches!(op, Op::JumpIfFalse { .. });
            i32_const(code, blocks.block_of(target) as i32);
            i32_const(code, blocks.block_of(pc + 1) as i32);
            local_get(code, u32::from(cond));
            call(code, ctx, HostFn::ValueTruthI32)?;
            if when_false {
                code.push(0x45); // i32.eqz
            }
            code.push(0x1B); // select
            local_set(code, plan.num_regs);
            code.push(0x0C); // br $loop
            leb_u32(code, blocks.br_loop(k));
            return Ok(Some(Flow::Terminated));
        }
        Op::IterPrepare { iterable } if t(iterable) => {
            local_get(code, u32::from(iterable));
            call_checked(code, ctx, HostFn::ValueItems)?;
            local_set(code, local(plan, FRAME_ITEMS));
            local_get(code, local(plan, FRAME_ITEMS));
            call(code, ctx, HostFn::ValueLengthI64)?;
            code.push(0xA7); // i32.wrap_i64
            local_set(code, local(plan, FRAME_COUNT));
            frame(code, plan, ctx, &|code: &mut Vec<u8>, plan: &Plan| {
                local_get(code, local(plan, FRAME_ITEMS));
                local_get(code, local(plan, FRAME_INDEX));
                code.push(0xAC); // i64.extend_i32_s
                code.push(0x42); // i64.const 1
                leb_i64(code, 1);
                code.push(0x7C); // i64.add
                call_checked(code, ctx, HostFn::ValueItemAt)
            })?;
        }
        Op::IterPrepare { iterable } if plan.tower.frames.contains(&pc) => {
            let seq = plan.kinds.get(iterable as usize).ok_or(WasmLowerError::Unsupported(
                "a loop over exact values whose list has no kind",
            ))?;
            let elem = seq.seq_elem().ok_or(WasmLowerError::UnsupportedOwned(format!(
                "a loop of exact values over a {seq:?}"
            )))?;
            local_get(code, u32::from(iterable));
            local_set(code, local(plan, FRAME_ITEMS));
            local_get(code, local(plan, FRAME_ITEMS));
            i32_load(code, 0);
            local_set(code, local(plan, FRAME_COUNT));
            frame(code, plan, ctx, &|code: &mut Vec<u8>, plan: &Plan| {
                let slot = |c: &mut Vec<u8>| {
                    local_get(c, local(plan, FRAME_ITEMS));
                    i32_load(c, 8);
                    local_get(c, local(plan, FRAME_INDEX));
                    i32_const(c, 8);
                    c.push(0x6C);
                    c.push(0x6A);
                    match elem.wasm_valtype() {
                        F64 => f64_load(c, 0),
                        I64 => i64_load(c, 0),
                        _ => i32_load(c, 0),
                    }
                };
                push_value(code, plan, ctx, elem, 0, &slot)
            })?;
        }
        Op::Move { dst, src } if t(dst) => {
            push_owned(code, plan, ctx, src)?;
            set(code, dst);
        }
        Op::GetField { dst, .. } | Op::GlobalGet { dst, .. } if t(dst) => {
            let flow = absence::lower(pc, plan, ctx, blocks, k, code)?;
            local_get(code, u32::from(dst));
            call_checked(code, ctx, HostFn::ValueClone)?;
            set(code, dst);
            return Ok(Some(flow));
        }
        // A map holds a handle of its own: the register an entry is stored from keeps a copy of its own. What an
        // entry is read into is copied by the `Move` the bytecode binds it with.
        Op::SetIndex {
            collection, value, ..
        }
        | Op::SetIndexUnchecked {
            collection, value, ..
        } if t(value) && plan.kinds.get(collection as usize) == Some(Kind::Map) => {
            let flow = absence::lower(pc, plan, ctx, blocks, k, code)?;
            local_get(code, u32::from(value));
            call_checked(code, ctx, HostFn::ValueClone)?;
            set(code, value);
            return Ok(Some(flow));
        }
        Op::ListPushField { obj, field, src } if exact_field(plan, obj, field).is_some() => {
            let slot = exact_field(plan, obj, field).expect("an exact field");
            if plan.cow_inserts.get(pc).copied().unwrap_or(true) {
                emit_record_copy(code, ctx, &plan.kinds, &plan.structs, plan.num_regs, obj, None)?;
            }
            local_get(code, u32::from(obj));
            i32_load(code, 8);
            i32_load(code, slot * 8);
            push_read(code, plan, ctx, src)?;
            call_checked(code, ctx, HostFn::ValuePush)?;
            code.push(0x1A);
        }
        Op::GlobalSet { idx, src }
            if !t(src)
                && ctx.global_kinds.get(idx as usize) == Some(&Some(Kind::Tower))
                && !plan.place.clear.get(pc).copied().unwrap_or(false) =>
        {
            push_owned(code, plan, ctx, src)?;
            global_set(code, u32::from(idx));
        }
        Op::DeepClone { dst, src } if t(src) => {
            push_owned(code, plan, ctx, src)?;
            set(code, dst);
        }
        Op::GlobalSet { idx, src } if t(src) && !plan.place.clear.get(pc).copied().unwrap_or(false) => {
            if ctx.global_valtypes.get(idx as usize).copied() != Some(I32) {
                return Err(WasmLowerError::Unsupported(
                    "an exact value stored into a global of another kind",
                ));
            }
            push_owned(code, plan, ctx, src)?;
            global_set(code, u32::from(idx));
        }
        Op::Return { src } if plan.result == Some(Kind::Tower) && !t(src) => {
            emit_leave_loops(code, plan, ctx);
            push_owned(code, plan, ctx, src)?;
            push_inout_results(code, plan);
            code.push(0x0F);
            return Ok(Some(Flow::Terminated));
        }
        Op::Return { src } if t(src) && plan.result != Some(Kind::Tower) => {
            return Err(WasmLowerError::UnsupportedOwned(format!(
                "an exact value returned where the function returns a {:?}",
                plan.result
            )));
        }
        ref other => match (crate::vm::tower::binary(other), crate::vm::tower::by_constant(other)) {
            (Some((dst, lhs, rhs, name)), _) if t(dst) => {
                push_read(code, plan, ctx, lhs)?;
                push_read(code, plan, ctx, rhs)?;
                call_checked(code, ctx, operation(name)?)?;
                set(code, dst);
            }
            (_, Some((dst, lhs, divisor, name))) if t(dst) => {
                push_read(code, plan, ctx, lhs)?;
                code.push(0x42); // i64.const
                leb_i64(code, divisor);
                call(code, ctx, HostFn::ValueInt)?;
                call_checked(code, ctx, operation(name)?)?;
                set(code, dst);
            }
            _ => return Ok(None),
        },
    }
    Ok(Some(Flow::Straight))
}

/// Mark as holding a value of the extension every global one of `regions` stores one into (an ownership clear
/// aside). Answers whether any global became one.
pub(super) fn exact_globals<'p>(regions: impl Iterator<Item = &'p Plan>, globals: &mut [Option<Kind>]) -> bool {
    let mut grew = false;
    for plan in regions {
        for (pc, op) in plan.ops.iter().enumerate() {
            let Op::GlobalSet { idx, src } = *op else {
                continue;
            };
            let clear = plan.place.clear.get(pc).copied().unwrap_or(false);
            if !clear && plan.kinds.get(src as usize) == Some(Kind::Tower) {
                if let Some(global) = globals.get_mut(idx as usize) {
                    grew |= *global != Some(Kind::Tower);
                    *global = Some(Kind::Tower);
                }
            }
        }
    }
    grew
}

/// The slot of record `obj`'s field named by constant `field`, where that field holds a value of the extension.
fn exact_field(plan: &Plan, obj: u16, field: u32) -> Option<u32> {
    let slot = record_field_slot(plan, obj, field)?;
    let kind = record_slot_kinds(&plan.kinds, &plan.structs, obj).and_then(|ks| ks.get(slot).copied().flatten());
    (kind == Some(Kind::Tower)).then_some(slot as u32)
}

/// Give record `obj`'s field at `slot`, a value of the extension, a handle of its own: a copy of the value.
pub(super) fn own_field(code: &mut Vec<u8>, ctx: &Ctx, obj: u16, slot: u32) -> R<()> {
    local_get(code, u32::from(obj));
    i32_load(code, 8);
    local_get(code, u32::from(obj));
    i32_load(code, 8);
    i32_load(code, slot * 8);
    call_checked(code, ctx, HostFn::ValueClone)?;
    i32_store(code, slot * 8);
    Ok(())
}

/// Push a loop frame of `FRAME_COUNT` items onto the iterator stack, each slot the handle `item` leaves for the
/// item at `FRAME_INDEX`, as [`lower_iter_prepare`] lays out a frame over a list.
fn frame(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    item: &dyn Fn(&mut Vec<u8>, &Plan) -> R<()>,
) -> R<()> {
    let (frame, index, count) = (
        local(plan, FRAME),
        local(plan, FRAME_INDEX),
        local(plan, FRAME_COUNT),
    );
    local_get(code, count);
    i32_const(code, 8);
    code.push(0x6C);
    i32_const(code, FRAME_PAYLOAD as i32);
    code.push(0x6A);
    emit_alloc(code, ctx, frame, AllocClass::Raw);
    i32_const(code, 0);
    local_set(code, index);
    code.extend([0x02, 0x40, 0x03, 0x40]); // block, loop
    local_get(code, index);
    local_get(code, count);
    code.push(0x4E); // i32.ge_s
    code.push(0x0D);
    leb_u32(code, 1);
    local_get(code, frame);
    local_get(code, index);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    item(code, plan)?;
    i32_store(code, FRAME_PAYLOAD);
    local_get(code, index);
    i32_const(code, 1);
    code.push(0x6A);
    local_set(code, index);
    code.push(0x0C);
    leb_u32(code, 0);
    code.extend([0x0B, 0x0B]);
    local_get(code, frame);
    i32_const(code, 0);
    i32_store(code, FRAME_KIND);
    local_get(code, frame);
    local_get(code, count);
    i32_store(code, FRAME_LEN);
    local_get(code, frame);
    i32_const(code, 0);
    i32_store(code, FRAME_CURSOR);
    local_get(code, frame);
    local_get(code, count);
    i32_const(code, 8);
    code.push(0x6C);
    i32_const(code, FRAME_PAYLOAD as i32);
    code.push(0x6A);
    i32_store(code, FRAME_SIZE);
    local_get(code, frame);
    global_get(code, ctx.iter_global);
    i32_store(code, FRAME_PREV);
    local_get(code, frame);
    global_set(code, ctx.iter_global);
    Ok(())
}

/// The local holding the value a field of record `obj` at `slot` receives from register `value`, where the field or
/// the value is one of the extension: a handle of the field's own, the runtime's value of a value of another kind.
/// `None` for a field and a value of other kinds.
pub(super) fn field_value(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    obj: u16,
    slot: usize,
    value: u16,
) -> R<Option<u32>> {
    if !plan.tower.holds {
        return Ok(None);
    }
    let declared = declared_record_slot_kind(&plan.structs, ctx.struct_types, ctx.constants, obj, slot);
    if declared != Some(Kind::Tower) && plan.kinds.get(value as usize) != Some(Kind::Tower) {
        return Ok(None);
    }
    let held = local(plan, OP_LIST);
    push_owned(code, plan, ctx, value)?;
    local_set(code, held);
    Ok(Some(held))
}

/// A handle in the module's Text form of how the value of register `src` prints, for text it joins.
pub(super) fn text_of(code: &mut Vec<u8>, ctx: &Ctx, src: u32, out: u32) -> R<()> {
    local_get(code, src);
    call(code, ctx, HostFn::ValueToText)?;
    local_set(code, out);
    Ok(())
}

/// Push call argument `arg` at the kind of the callee's parameter: a copy of a value of the extension, or the
/// runtime's value of a value of another kind, for a parameter holding one. `None` when neither side holds one; a
/// parameter some call hands one holds one ([`super::exact_arguments`]), so an exact argument meets no other.
pub(super) fn push_argument(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    arg: u16,
    param: Option<Kind>,
) -> R<Option<()>> {
    let arg_kind = plan.kinds.get(arg as usize);
    match (arg_kind, param) {
        (_, Some(Kind::Tower)) => push_owned(code, plan, ctx, arg).map(Some),
        (Some(Kind::Tower), other) => Err(WasmLowerError::UnsupportedOwned(format!(
            "an exact value passed to a parameter of kind {other:?}"
        ))),
        _ => Ok(None),
    }
}

#[cfg(test)]
#[path = "../../../../../logicaffeine_tests/unit/logicaffeine_compile/vm/wasm/module/tower/tests.rs"]
mod tests;
