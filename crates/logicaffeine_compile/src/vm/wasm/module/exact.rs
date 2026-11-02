//! Exact integers in a self-contained module.
//!
//! A LOGOS `Int` is unbounded. Where the program can make one outgrow a machine word — the decision
//! [`crate::vm::exactness`] takes for every backend that lowers the bytecode — the module holds it as a
//! *tagged word*: an even `i64` `w` is the integer `w >> 1`, one of the integers from −2⁶² up to 2⁶² a
//! word holds with a bit to spare; an odd one, `(h << 1) | 1`, names integer `h` the host holds. An
//! integer outside that range is always held by the host and one inside it never is, so two tagged
//! words are equal exactly when their integers are, two small ones order as their integers do, and
//! zero is the word zero.
//!
//! Arithmetic on two small words runs inline, checked; a sum or difference that overflows the word, a
//! product the doubles put past it, and every operation on a held integer go to the host (`exact_*`),
//! which answers with a tagged word. An op the module does not lower on tagged words reads an exact
//! register narrowed to the machine word it holds — failing with the reference tier's sentence when the
//! integer is past one — and a register it writes is tagged after.
//!
//! A held integer is owned. Each exact register, tagged global and tagged parameter holds a handle of
//! its own: a copy clones it (`exact_clone`), an overwrite and every way out of a function drop it
//! (`exact_drop`), and a result moves to whoever receives it — a returned value to the caller, a
//! `mutable` parameter's final value back into the register the caller lent. A loop over integers past
//! a word runs in the memory of its live values.
//!
//! The decision is per register — every definition of a register agrees on its form — and per
//! boundary: a function's `Int` parameter is tagged when a call passes it an exact value, its result
//! when a `Return` yields one, a global when a store writes one, and each of those makes the values that
//! arrive through it exact in turn, until nothing changes. A function the module cannot see every
//! caller of — a closure body, a function the host enters by name — keeps machine words at its
//! boundary and tags what it needs inside.

mod list;
pub(super) use list::emit_at_helper as emit_seq_at_helper;
pub(super) use list::emit_promote_helper as emit_seq_promote_helper;
pub(super) use list::emit_store_helper as emit_seq_store_helper;
mod map;
pub(super) use map::{emit_cell_helper, emit_promote_helper as emit_map_promote_helper};

use std::collections::HashSet;

use super::*;
use crate::semantics::failcode::FailCode;
use crate::vm::exactness::{self, ExactnessHost, IntForm, Region};

/// The tagged boundaries of a module: which parameters, results and globals hold tagged words.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Signatures {
    /// Per function, per parameter: its argument arrives tagged.
    params: Vec<Vec<bool>>,
    /// Per function: its result is tagged.
    ret: Vec<bool>,
    /// Per global: it holds a tagged word.
    globals: Vec<bool>,
    /// Record Int slots can hold collector-owned integer cells.
    record_ints: bool,
    sequence_ints: bool,
    pub(super) map_ints: bool,
}

impl Signatures {
    pub(super) fn param(&self, func: usize, p: usize) -> bool {
        self.params
            .get(func)
            .and_then(|ps| ps.get(p))
            .copied()
            .unwrap_or(false)
    }

    pub(super) fn ret(&self, func: usize) -> bool {
        self.ret.get(func).copied().unwrap_or(false)
    }

    pub(super) fn global(&self, g: usize) -> bool {
        self.globals.get(g).copied().unwrap_or(false)
    }

    pub(super) fn record_ints(&self) -> bool {
        self.record_ints
    }

    pub(super) fn sequence_ints(&self) -> bool {
        self.sequence_ints
    }

    fn any(&self) -> bool {
        self.record_ints
            || self.sequence_ints
            || self.map_ints
            || self
                .ret
                .iter()
                .chain(&self.globals)
                .chain(self.params.iter().flatten())
                .any(|t| *t)
    }
}

/// The closure body a `CallValue` at `pc` calls: always known where the module lowers one.
fn closure_callee(plan: &Plan, pc: usize) -> Option<usize> {
    plan.structs
        .callee_func
        .get(pc)
        .copied()
        .flatten()
        .map(usize::from)
}

/// A plan's typing on machine words, as the shared decision asks it.
struct PlanInts<'p> {
    program: &'p CompiledProgram,
    plan: &'p Plan,
    sig: &'p Signatures,
}

impl ExactnessHost for PlanInts<'_> {
    fn program(&self) -> &CompiledProgram {
        self.program
    }

    fn op(&self, pc: usize) -> &Op {
        &self.plan.ops[pc]
    }

    fn int_after(&self, pc: usize, reg: u16) -> Option<IntForm> {
        if self.plan.kinds.get(reg as usize) != Some(Kind::Int) {
            return None;
        }
        // A callee's `mutable` parameter comes back tagged into the register it was lent from.
        if let Op::Call {
            func,
            args_start,
            dst,
            ..
        } = self.plan.ops[pc]
        {
            let p = reg.wrapping_sub(args_start);
            if reg != dst
                && reg >= args_start
                && self.sig.param(func as usize, p as usize)
                && exactness::inout_regs(self.program, func as usize).contains(&p)
            {
                return Some(IntForm::Exact);
            }
        }
        Some(IntForm::Word)
    }

    fn arrives_exact(&self, pc: usize) -> bool {
        match self.plan.ops[pc] {
            Op::Call { func, .. } => self.sig.ret(func as usize),
            Op::CallValue { .. } => closure_callee(self.plan, pc).is_some_and(|f| self.sig.ret(f)),
            Op::GlobalGet { idx, .. } => self.sig.global(idx as usize),
            Op::GetField { .. } => self.sig.record_ints,
            Op::Index { collection, .. } | Op::IndexUnchecked { collection, .. }
                if self.plan.kinds.get(collection as usize) == Some(Kind::SeqInt) =>
            {
                self.sig.sequence_ints
            }
            Op::Index { collection, .. } | Op::IndexUnchecked { collection, .. }
                if self.plan.kinds.get(collection as usize) == Some(Kind::Map) =>
            {
                self.sig.map_ints
            }
            Op::ListPop { list, .. }
                if self.plan.kinds.get(list as usize) == Some(Kind::SeqInt) =>
            {
                self.sig.sequence_ints
            }
            Op::IterNext { .. } => {
                self.sig.sequence_ints
                    && self
                        .plan
                        .iter_iterable
                        .get(pc)
                        .copied()
                        .flatten()
                        .is_some_and(|r| self.plan.kinds.get(r) == Some(Kind::SeqInt))
            }

            _ => false,
        }
    }

    fn successors(&self, pc: usize) -> Vec<usize> {
        crate::vm::peephole::successors(pc, &self.plan.ops, &HashSet::new())
    }
}

/// The registers of `plan` (the body of `func`, or Main) that hold tagged words, given the boundaries
/// as they stand: what the shared decision makes exact — its definitions, the parameters an exact value
/// arrives in, the registers a callee's tagged `mutable` parameter comes back into and a swap trades one into.
fn registers(
    program: &CompiledProgram,
    plan: &Plan,
    func: Option<usize>,
    sig: &Signatures,
) -> Vec<bool> {
    let n = plan.num_regs as usize;
    let mut exact = plan.exact.clone();
    exact.resize(n, false);
    if plan.stub {
        return exact;
    }
    let int = |r: usize| plan.kinds.get(r) == Some(Kind::Int);
    let seed: Vec<Option<IntForm>> = (0..n)
        .map(|r| {
            (r < plan.num_params as usize && int(r)).then(|| {
                if exact[r] || func.is_some_and(|fi| sig.param(fi, r)) {
                    IntForm::Exact
                } else {
                    IntForm::Word
                }
            })
        })
        .collect();
    let host = PlanInts { program, plan, sig };
    let decided = exactness::exactness(
        &host,
        &Region {
            start: 0,
            end: plan.ops.len(),
            holes: &[],
            func,
            seed: &seed,
        },
    );
    for pc in decided.defs {
        plan.ops[pc].for_each_written_reg(|w| {
            if int(w as usize) {
                exact[w as usize] = true;
            }
        });
    }
    for r in decided
        .entry_speculative
        .into_iter()
        .chain(decided.outs.into_iter().map(|(_, r)| r))
    {
        exact[r as usize] = true;
    }
    if sig.map_ints {
        for op in &plan.ops {
            let (collection, inputs): (u16, Vec<u16>) = match *op {
                Op::SetIndex {
                    collection,
                    index,
                    value,
                }
                | Op::SetIndexUnchecked {
                    collection,
                    index,
                    value,
                } => (collection, vec![index, value]),
                Op::Index {
                    collection, index, ..
                }
                | Op::IndexUnchecked {
                    collection, index, ..
                } => (collection, vec![index]),
                Op::Contains {
                    collection, value, ..
                }
                | Op::RemoveFrom { collection, value } => (collection, vec![value]),
                _ => continue,
            };
            if plan.kinds.get(collection as usize) == Some(Kind::Map) {
                for input in inputs {
                    if int(input as usize) {
                        exact[input as usize] = true;
                    }
                }
            }
        }
    }
    exact
}

/// Decide the tagged registers of Main and every function, and the tagged boundaries between them, to a
/// fixpoint; stamp each plan with its registers and its boundary. `raw_boundary` names the functions
/// whose callers the module cannot all see. A linked module keeps the runtime's own big integers, and a
/// program no integer of which can outgrow a word is left exactly as it was.
pub(super) fn decide(
    program: &CompiledProgram,
    main: &mut Plan,
    fns: &mut [Plan],
    raw_boundary: &[bool],
    globals: usize,
    linked: bool,
) -> Signatures {
    let mut sig = Signatures {
        params: fns
            .iter()
            .map(|p| vec![false; p.num_params as usize])
            .collect(),
        ret: vec![false; fns.len()],
        globals: vec![false; globals],
        record_ints: false,
        sequence_ints: false,
        map_ints: false,
    };
    if linked {
        return sig;
    }
    let raw = |fi: usize| raw_boundary.get(fi).copied().unwrap_or(false);
    loop {
        let before = (
            sig.clone(),
            main.exact.clone(),
            fns.iter().map(|p| p.exact.clone()).collect::<Vec<_>>(),
        );
        main.exact = registers(program, main, None, &sig);
        for fi in 0..fns.len() {
            fns[fi].exact = registers(program, &fns[fi], Some(fi), &sig);
        }
        let plans: Vec<(Option<usize>, &Plan)> = std::iter::once((None, &*main))
            .chain(fns.iter().enumerate().map(|(fi, p)| (Some(fi), p)))
            .collect();
        for (func, plan) in &plans {
            let tagged = |r: u16| plan.exact.get(r as usize).copied().unwrap_or(false);
            for (pc, op) in plan.ops.iter().enumerate() {
                let passed = match *op {
                    Op::Call {
                        func: callee,
                        args_start,
                        arg_count,
                        ..
                    }
                    | Op::Spawn {
                        func: callee,
                        args_start,
                        arg_count,
                    }
                    | Op::SpawnHandle {
                        func: callee,
                        args_start,
                        arg_count,
                        ..
                    } => Some((callee as usize, args_start, arg_count)),
                    Op::CallValue {
                        args_start,
                        arg_count,
                        ..
                    } => closure_callee(plan, pc).map(|callee| (callee, args_start, arg_count)),
                    _ => None,
                };
                if let Some((callee, args_start, arg_count)) = passed {
                    if raw(callee) || fns.get(callee).is_none_or(|p| p.stub) {
                        continue;
                    }
                    for i in 0..arg_count {
                        if tagged(args_start + i)
                            && fns[callee].kinds.get(i as usize) == Some(Kind::Int)
                        {
                            sig.params[callee][i as usize] = true;
                        }
                    }
                }
                match *op {
                    Op::NewList { dst, start, count } | Op::NewTuple { dst, start, count }
                        if plan.kinds.get(dst as usize) == Some(Kind::SeqInt)
                            && (start..start + count).any(tagged) =>
                    {
                        sig.sequence_ints = true
                    }
                    Op::ListPush { list, value }
                        if plan.kinds.get(list as usize) == Some(Kind::SeqInt) && tagged(value) =>
                    {
                        sig.sequence_ints = true
                    }
                    Op::SetIndex {
                        collection, value, ..
                    }
                    | Op::SetIndexUnchecked {
                        collection, value, ..
                    } if plan.kinds.get(collection as usize) == Some(Kind::SeqInt)
                        && tagged(value) =>
                    {
                        sig.sequence_ints = true
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
                    } if plan.kinds.get(collection as usize) == Some(Kind::Map)
                        && (tagged(index) || tagged(value)) =>
                    {
                        sig.map_ints = true;
                    }
                    Op::ListPushField { src, .. }
                        if tagged(src) && plan.kinds.get(src as usize) == Some(Kind::Int) =>
                    {
                        sig.sequence_ints = true
                    }
                    Op::CallBuiltin {
                        builtin: BuiltinId::RepeatSeq,
                        args_start,
                        ..
                    } if tagged(args_start) => sig.sequence_ints = true,
                    Op::StructInsert { value, .. } if tagged(value) => sig.record_ints = true,
                    Op::SetIndex {
                        collection, value, ..
                    }
                    | Op::SetIndexUnchecked {
                        collection, value, ..
                    } if plan.kinds.get(collection as usize) == Some(Kind::Struct)
                        && tagged(value) =>
                    {
                        sig.record_ints = true;
                    }
                    Op::Return { src } => {
                        if let Some(fi) = func {
                            if !raw(*fi) && tagged(src) && plan.result == Some(Kind::Int) {
                                sig.ret[*fi] = true;
                            }
                        }
                    }
                    Op::GlobalSet { idx, src } => {
                        if tagged(src) {
                            if let Some(g) = sig.globals.get_mut(idx as usize) {
                                *g = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            // A parameter the body holds exact arrives tagged — a declared one: a closure's captures ride its
            // object, as machine words.
            if let Some(fi) = func {
                if !raw(*fi) {
                    let declared = program.functions.get(*fi).map_or(0, |f| f.param_count);
                    for p in 0..declared.min(plan.num_params as u16) {
                        if tagged(p) {
                            sig.params[*fi][p as usize] = true;
                        }
                    }
                }
            }
        }
        if (
            sig.clone(),
            main.exact.clone(),
            fns.iter().map(|p| p.exact.clone()).collect::<Vec<_>>(),
        ) == before
        {
            break;
        }
    }
    let on = sig.any()
        || std::iter::once(&*main)
            .chain(fns.iter())
            .any(|p| p.exact.iter().any(|e| *e));
    for (fi, plan) in fns.iter_mut().enumerate() {
        plan.params_tagged = sig.params[fi].clone();
        plan.ret_tagged = sig.ret[fi];
    }
    for plan in std::iter::once(main).chain(fns.iter_mut()) {
        if on {
            plan.exact.resize(plan.num_regs as usize, false);
        } else {
            plan.exact.clear();
        }
    }
    sig
}

/// Whether the module carries tagged words at all: every plan of one that does has its registers decided
/// (and the scratch locals the lowering below works in), every plan of one that does not, none.
pub(super) fn on(plan: &Plan) -> bool {
    !plan.exact.is_empty()
}

/// Whether register `r` of `plan` holds a tagged word.
pub(super) fn holds(plan: &Plan, r: u16) -> bool {
    plan.exact.get(r as usize).copied().unwrap_or(false)
}

/// The host imports a module carrying tagged words calls: every arithmetic and ownership entry, `Show`'s
/// when an exact register is shown, and the Text one when an exact register is rendered into a Text.
pub(super) fn note_imports(plans: &[&Plan], note: &mut dyn FnMut(HostFn)) {
    if !plans.iter().any(|p| on(p)) {
        return;
    }
    for h in [
        HostFn::ExactBox,
        HostFn::ExactNarrow,
        HostFn::ExactAdd,
        HostFn::ExactSub,
        HostFn::ExactMul,
        HostFn::ExactDiv,
        HostFn::ExactMod,
        HostFn::ExactFloorDiv,
        HostFn::ExactPow,
        HostFn::ExactCmp,
        HostFn::ExactF64,
        HostFn::ExactClone,
        HostFn::ExactDrop,
        HostFn::LogosRtFailAt,
    ] {
        note(h);
    }
    for plan in plans {
        for op in &plan.ops {
            match *op {
                Op::Show { src } if holds(plan, src) => note(HostFn::PrintExact),
                Op::Show { src } | Op::FormatValue { src, .. }
                    if plan.kinds.get(src as usize) == Some(Kind::Map) =>
                {
                    note(HostFn::ExactText)
                }
                Op::StructInsert { value, .. } if holds(plan, value) => note(HostFn::ExactText),
                Op::SetIndex {
                    collection, value, ..
                }
                | Op::SetIndexUnchecked {
                    collection, value, ..
                } if plan.kinds.get(collection as usize) == Some(Kind::Struct)
                    && holds(plan, value) =>
                {
                    note(HostFn::ExactText)
                }
                Op::Concat { lhs, rhs, .. } if holds(plan, lhs) || holds(plan, rhs) => {
                    note(HostFn::ExactText)
                }
                Op::FormatValue { src, .. } if holds(plan, src) => note(HostFn::ExactText),
                Op::CallBuiltin {
                    builtin: BuiltinId::Format | BuiltinId::Fail,
                    args_start,
                    arg_count,
                    ..
                } if arg_count > 0 && holds(plan, args_start) => note(HostFn::ExactText),
                Op::Add { dst, lhs, rhs }
                    if plan.kinds.get(dst as usize) == Some(Kind::Text)
                        && (holds(plan, lhs) || holds(plan, rhs)) =>
                {
                    note(HostFn::ExactText)
                }
                Op::AddAssign { dst, src }
                    if plan.kinds.get(dst as usize) == Some(Kind::Text) && holds(plan, src) =>
                {
                    note(HostFn::ExactText)
                }
                _ => {}
            }
        }
    }
}

/// The i64 scratch locals the lowering works in, after the ones [`encode_locals`] lays out for every
/// body: two operands and a result.
const OPERAND_A: u32 = 25;
const OPERAND_B: u32 = 26;
const RESULT: u32 = 27;
/// Where the first exact register's saved word sits; one i64 per exact register follows.
const SAVED: u32 = 28;

/// How many i64 locals a plan carrying tagged words adds after [`encode_locals`]'s own.
pub(super) fn extra_locals(plan: &Plan) -> u32 {
    if on(plan) {
        SAVED - OPERAND_A + plan.exact.iter().filter(|e| **e).count() as u32
    } else {
        0
    }
}

fn scratch(plan: &Plan, at: u32) -> u32 {
    plan.num_regs + at
}

/// The local an exact register's word is saved in while an op reads it narrowed.
fn saved(plan: &Plan, r: u16) -> u32 {
    plan.num_regs + SAVED + plan.exact[..r as usize].iter().filter(|e| **e).count() as u32
}

fn import(ctx: &Ctx, h: HostFn) -> R<u32> {
    (ctx.host_index)(h).ok_or(WasmLowerError::Unsupported(
        "an exact-integer import the module did not note",
    ))
}

fn call(code: &mut Vec<u8>, f: u32) {
    code.push(0x10);
    leb_u32(code, f);
}

/// Share exact-integer mechanisms when collections carry tagged values: the same
/// ownership and arithmetic checks occur at many collection sites, including loops.
/// Scalar-only loops retain their inline fast path.
fn out_of_line(ctx: &Ctx, f: RuntimeFn) -> Option<u32> {
    if ctx.hot.get() && !ctx.exact.sequence_ints && !ctx.exact.map_ints {
        return None;
    }
    ctx.runtime.and_then(|r| r.index(f))
}

fn i64_const(code: &mut Vec<u8>, v: i64) {
    code.push(0x42);
    leb_i64(code, v);
}

/// `[i64]` → `[i32]`: whether the tagged word on the stack names an integer the host holds.
fn is_held(code: &mut Vec<u8>) {
    i64_const(code, 1);
    code.push(0x83); // i64.and
    code.push(0xA7); // i32.wrap_i64
}

/// Whether `v` fits beside the tag.
fn fits_small(v: i64) -> bool {
    (v << 1) >> 1 == v
}

/// `[raw i64]` → `[tagged]`: shifted when it fits beside the tag, held by the host when not. Works in
/// local `s`.
fn tag_top(code: &mut Vec<u8>, ctx: &Ctx, s: u32) -> R<()> {
    if let Some(tag) = out_of_line(ctx, RuntimeFn::ExactTag) {
        call(code, tag);
        return Ok(());
    }
    tag_inline(code, import(ctx, HostFn::ExactBox)?, s);
    Ok(())
}

/// [`tag_top`]'s body, calling the host's `exact_box` (`boxed`) for a word too wide to sit beside the tag.
fn tag_inline(code: &mut Vec<u8>, boxed: u32, s: u32) {
    local_tee(code, s);
    i64_const(code, 1);
    code.push(0x86); // i64.shl
    i64_const(code, 1);
    code.push(0x87); // i64.shr_s
    local_get(code, s);
    code.push(0x51); // i64.eq
    code.push(0x04);
    code.push(I64); // if (result i64)
    local_get(code, s);
    i64_const(code, 1);
    code.push(0x86);
    code.push(0x05); // else
    local_get(code, s);
    call(code, boxed);
    code.push(0x0B);
}

/// `[tagged]` → `[raw i64]`, failing with `why` when it names an integer past a machine word. Works in
/// local `s`, which it leaves holding the raw word.
fn narrow_top(code: &mut Vec<u8>, ctx: &Ctx, s: u32, why: FailCode) -> R<()> {
    if let Some(narrow) = out_of_line(ctx, RuntimeFn::ExactNarrow) {
        i32_const(code, why.code());
        call(code, narrow);
        return Ok(());
    }
    let narrow = import(ctx, HostFn::ExactNarrow)?;
    let fail = import(ctx, HostFn::LogosRtFailAt)?;
    narrow_inline(code, narrow, fail, s, |code| i32_const(code, why.code()));
    Ok(())
}

/// [`narrow_top`]'s body, calling the host's `exact_narrow` (`narrow`), and failing through
/// `logos_rt_fail_at` (`fail`) with the code `push_why` pushes.
fn narrow_inline(
    code: &mut Vec<u8>,
    narrow: u32,
    fail: u32,
    s: u32,
    push_why: impl Fn(&mut Vec<u8>),
) {
    local_tee(code, s);
    is_held(code);
    code.push(0x45); // i32.eqz
    code.push(0x04);
    code.push(I64); // if (result i64) — a small word
    local_get(code, s);
    i64_const(code, 1);
    code.push(0x87); // i64.shr_s
    code.push(0x05); // else — the host's answer, zero when it is past a word (it never holds zero)
    local_get(code, s);
    call(code, narrow);
    local_tee(code, s);
    code.push(0x50); // i64.eqz
    code.push(0x04);
    code.push(0x40);
    push_why(code);
    i64_const(code, 0);
    call(code, fail);
    code.push(0x00); // unreachable
    code.push(0x0B);
    local_get(code, s);
    code.push(0x0B);
}

/// `[tagged]` → `[a tagged word of its own]`: a held integer cloned, a small one as it is. Works in `s`.
fn clone_top(code: &mut Vec<u8>, ctx: &Ctx, s: u32) -> R<()> {
    if let Some(clone) = out_of_line(ctx, RuntimeFn::ExactClone) {
        call(code, clone);
        return Ok(());
    }
    clone_inline(code, import(ctx, HostFn::ExactClone)?, s);
    Ok(())
}

/// [`clone_top`]'s body, calling the host's `exact_clone` (`clone`).
fn clone_inline(code: &mut Vec<u8>, clone: u32, s: u32) {
    local_tee(code, s);
    is_held(code);
    code.push(0x04);
    code.push(I64);
    local_get(code, s);
    call(code, clone);
    code.push(0x05);
    local_get(code, s);
    code.push(0x0B);
}

/// Give up the integer the tagged word in local `l` holds, when the host holds it.
fn drop_local(code: &mut Vec<u8>, ctx: &Ctx, l: u32) -> R<()> {
    if let Some(release) = out_of_line(ctx, RuntimeFn::ExactRelease) {
        local_get(code, l);
        call(code, release);
        return Ok(());
    }
    drop_inline(code, import(ctx, HostFn::ExactDrop)?, l);
    Ok(())
}

/// [`drop_local`]'s body, calling the host's `exact_drop` (`drop`).
fn drop_inline(code: &mut Vec<u8>, drop: u32, l: u32) {
    local_get(code, l);
    is_held(code);
    code.push(0x04);
    code.push(0x40);
    local_get(code, l);
    call(code, drop);
    code.push(0x0B);
}

/// The integer machine-word register `reg` holds at `pc` when the compiler wrote it there as a constant: the
/// last write to it in `pc`'s basic block (which starts at `block_start`) is a `LoadConst` of an `Int` that
/// fits beside the tag. Its tagged word is then known at compile time.
fn constant_at(plan: &Plan, ctx: &Ctx, block_start: usize, pc: usize, reg: u16) -> Option<i64> {
    if holds(plan, reg) {
        return None;
    }
    for at in (block_start..pc).rev() {
        let op = &plan.ops[at];
        let mut writes = false;
        op.for_each_written_reg(|w| writes |= w == reg);
        if let Op::Call {
            func, args_start, ..
        } = *op
        {
            let lent = ctx
                .fn_inout
                .get(func as usize)
                .is_some_and(|ps| ps.iter().any(|p| args_start + p == reg));
            writes |= lent;
        }
        if writes {
            return match *op {
                Op::LoadConst { idx, .. } => match ctx.constants.get(idx as usize) {
                    Some(Constant::Int(v)) if fits_small(*v) => Some(*v),
                    _ => None,
                },
                _ => None,
            };
        }
    }
    None
}

/// The local holding `reg` as a tagged word: the register itself when it is exact, the constant it holds
/// written into scratch `s` when it is one, else `s` it is tagged into at run time — a word of its own the
/// caller drops (`true`).
fn tagged_operand(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    reg: u16,
    constant: Option<i64>,
    s: u32,
) -> R<(u32, bool)> {
    if holds(plan, reg) {
        return Ok((reg as u32, false));
    }
    if let Some(v) = constant {
        i64_const(code, v << 1);
        local_set(code, s);
        return Ok((s, false));
    }
    local_get(code, reg as u32);
    tag_top(code, ctx, s)?;
    local_set(code, s);
    Ok((s, true))
}

/// Store the tagged word of its own in local `v` into `dst`: an exact register gives up what it held
/// and takes it; a machine-word register takes its integer narrowed, and the word is given up.
fn store_owned(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    dst: u16,
    v: u32,
    why: FailCode,
) -> R<()> {
    if holds(plan, dst) {
        drop_local(code, ctx, dst as u32)?;
        local_get(code, v);
        local_set(code, dst as u32);
    } else {
        local_get(code, v);
        narrow_top(code, ctx, scratch(plan, OPERAND_A), why)?;
        local_set(code, dst as u32);
        drop_local(code, ctx, v)?;
    }
    Ok(())
}

/// Store the raw word on the stack into `dst`: an exact register gives up what it held and takes it
/// tagged; a machine-word register takes it as it is.
pub(super) fn set_word(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, dst: u16) -> R<()> {
    if !holds(plan, dst) {
        local_set(code, dst as u32);
        return Ok(());
    }
    let result = scratch(plan, RESULT);
    tag_top(code, ctx, scratch(plan, OPERAND_A))?;
    local_set(code, result);
    drop_local(code, ctx, dst as u32)?;
    local_get(code, result);
    local_set(code, dst as u32);
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    FloorDiv,
}

impl Arith {
    fn host(self) -> HostFn {
        match self {
            Arith::Add => HostFn::ExactAdd,
            Arith::Sub => HostFn::ExactSub,
            Arith::Mul => HostFn::ExactMul,
            Arith::Div => HostFn::ExactDiv,
            Arith::Mod => HostFn::ExactMod,
            Arith::FloorDiv => HostFn::ExactFloorDiv,
        }
    }

    fn divides(self) -> bool {
        matches!(self, Arith::Div | Arith::Mod | Arith::FloorDiv)
    }

    /// The module's out-of-line form of a ring operation; a division has none.
    fn helper(self) -> Option<RuntimeFn> {
        match self {
            Arith::Add => Some(RuntimeFn::ExactAdd),
            Arith::Sub => Some(RuntimeFn::ExactSub),
            Arith::Mul => Some(RuntimeFn::ExactMul),
            Arith::Div | Arith::Mod | Arith::FloorDiv => None,
        }
    }

    /// The out-of-line form of a ring operation whose right operand is a machine word: the helper tags it for
    /// the operation and gives it back after, so the site holds no tagged word of its own.
    fn word_helper(self) -> Option<RuntimeFn> {
        match self {
            Arith::Add => Some(RuntimeFn::ExactAddW),
            Arith::Sub => Some(RuntimeFn::ExactSubW),
            Arith::Mul => Some(RuntimeFn::ExactMulW),
            Arith::Div | Arith::Mod | Arith::FloorDiv => None,
        }
    }
}

/// `[i64 tagged]` → `[i64 raw]` for a word known small.
fn untag(code: &mut Vec<u8>, l: u32) {
    local_get(code, l);
    i64_const(code, 1);
    code.push(0x87); // i64.shr_s
}

/// `dst = lhs <op> rhs` on tagged words: inline on two small ones, checked, and the host's otherwise. An
/// operand the compiler wrote as a constant (`consts`) is tagged at compile time, and against one the checks
/// fold to a bound on the other operand. Outside a loop a ring operation is a call to its runtime helper.
fn binop(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    dst: u16,
    lhs: u16,
    rhs: u16,
    op: Arith,
    consts: (Option<i64>, Option<i64>),
) -> R<Flow> {
    let commutes = matches!(op, Arith::Add | Arith::Mul);
    if let Some(word_helper) = op.word_helper().and_then(|f| out_of_line(ctx, f)) {
        // A machine-word operand beside an exact one goes to the word form, which tags it and gives it back.
        let word = |reg: u16, constant: Option<i64>| constant.is_none() && !holds(plan, reg);
        let pair = if holds(plan, lhs) && word(rhs, consts.1) {
            Some((lhs, rhs))
        } else if commutes && holds(plan, rhs) && word(lhs, consts.0) {
            Some((rhs, lhs))
        } else {
            None
        };
        if let Some((exact, word)) = pair {
            local_get(code, exact as u32);
            local_get(code, word as u32);
            call(code, word_helper);
            return settle_result(
                code,
                plan,
                ctx,
                dst,
                (exact as u32, false),
                (word as u32, false),
            );
        }
    }
    if let Some(helper) = op.helper().and_then(|f| out_of_line(ctx, f)) {
        let (a, a_own) = tagged_operand(code, plan, ctx, lhs, consts.0, scratch(plan, OPERAND_A))?;
        let (b, b_own) = tagged_operand(code, plan, ctx, rhs, consts.1, scratch(plan, OPERAND_B))?;
        local_get(code, a);
        local_get(code, b);
        call(code, helper);
        return settle_result(code, plan, ctx, dst, (a, a_own), (b, b_own));
    }
    match consts {
        (_, Some(c)) if !(op.divides() && c == 0) => {
            return by_constant(code, plan, ctx, dst, lhs, c, op)
        }
        (Some(c), _) if commutes => return by_constant(code, plan, ctx, dst, rhs, c, op),
        _ => {}
    }
    let host = import(ctx, op.host())?;
    let (a, a_own) = tagged_operand(code, plan, ctx, lhs, consts.0, scratch(plan, OPERAND_A))?;
    let (b, b_own) = tagged_operand(code, plan, ctx, rhs, consts.1, scratch(plan, OPERAND_B))?;
    let r = scratch(plan, RESULT);
    if op.divides() {
        // The divisor zero is the word zero, whatever form it takes.
        let fail = import(ctx, HostFn::LogosRtFailAt)?;
        local_get(code, b);
        code.push(0x50); // i64.eqz
        code.push(0x04);
        code.push(0x40);
        i32_const(code, FailCode::DivisionByZero.code());
        i64_const(code, 0);
        call(code, fail);
        code.push(0x00);
        code.push(0x0B);
    }
    if matches!(op, Arith::Add | Arith::Sub | Arith::Mul) {
        ring_op(code, op, a, b, r, host);
    } else {
        let slow = |code: &mut Vec<u8>| {
            local_get(code, a);
            local_get(code, b);
            call(code, host);
            local_set(code, r);
        };
        if_both_small(code, a, b);
        match op {
            Arith::Div => {
                // A quotient of two small integers is no larger than its dividend, save −2⁶² divided by −1, which
                // is one past the small range: it is tagged as any word is.
                untag(code, a);
                untag(code, b);
                code.push(0x7F); // i64.div_s
                tag_top(code, ctx, r)?;
                local_set(code, r);
            }
            Arith::Mod => {
                // A remainder is smaller than its divisor.
                untag(code, a);
                untag(code, b);
                code.push(0x81); // i64.rem_s
                i64_const(code, 1);
                code.push(0x86);
                local_set(code, r);
            }
            Arith::FloorDiv => {
                // The truncated quotient, one less when the remainder is nonzero and the signs differ.
                untag(code, a);
                untag(code, b);
                code.push(0x7F);
                untag(code, a);
                untag(code, b);
                code.push(0x81);
                i64_const(code, 0);
                code.push(0x52); // i64.ne
                local_get(code, a);
                local_get(code, b);
                code.push(0x85);
                i64_const(code, 0);
                code.push(0x53);
                code.push(0x71); // i32.and
                code.push(0xAD); // i64.extend_i32_u
                code.push(0x7D); // i64.sub
                tag_top(code, ctx, r)?;
                local_set(code, r);
            }
            Arith::Add | Arith::Sub | Arith::Mul => unreachable!("a ring operation lowers above"),
        }
        code.push(0x05); // else — a held operand
        slow(code);
        code.push(0x0B);
    }
    if a_own {
        drop_local(code, ctx, a)?;
    }
    if b_own {
        drop_local(code, ctx, b)?;
    }
    store_owned(code, plan, ctx, dst, r, FailCode::IntTooWide)?;
    Ok(Flow::Straight)
}

/// Open an `if` taken when the tagged words in locals `a` and `b` are both small.
fn if_both_small(code: &mut Vec<u8>, a: u32, b: u32) {
    local_get(code, a);
    local_get(code, b);
    code.push(0x84); // i64.or
    i64_const(code, 1);
    code.push(0x83);
    code.push(0x50); // i64.eqz
    code.push(0x04);
    code.push(0x40);
}

/// `r = a <op> b` for a ring operation (`+ − ×`) on the tagged words in locals `a` and `b`: inline on two
/// small ones, checked, and the host's (`host`) otherwise. The one body an inline site and the module's
/// out-of-line helper ([`emit_ring_helper`]) both run.
fn ring_op(code: &mut Vec<u8>, op: Arith, a: u32, b: u32, r: u32, host: u32) {
    let slow = |code: &mut Vec<u8>| {
        local_get(code, a);
        local_get(code, b);
        call(code, host);
        local_set(code, r);
    };
    if_both_small(code, a, b);
    match op {
        Arith::Add | Arith::Sub => {
            local_get(code, a);
            local_get(code, b);
            code.push(if op == Arith::Add { 0x7C } else { 0x7D });
            local_set(code, r);
            // Overflowed the word: the result's sign disagrees with both operands' (a sum), or with
            // the minuend's where the operands' signs differ (a difference).
            if op == Arith::Add {
                local_get(code, a);
                local_get(code, r);
                code.push(0x85);
                local_get(code, b);
                local_get(code, r);
                code.push(0x85);
            } else {
                local_get(code, a);
                local_get(code, b);
                code.push(0x85);
                local_get(code, a);
                local_get(code, r);
                code.push(0x85);
            }
            code.push(0x83); // i64.and
            i64_const(code, 0);
            code.push(0x53); // i64.lt_s
            code.push(0x04);
            code.push(0x40);
            slow(code);
            code.push(0x0B);
        }
        Arith::Mul => {
            // The doubles' product bounds the integers': under 2^61 it is surely under 2^62, and the
            // shifted product is the tagged one.
            untag(code, a);
            code.push(0xB9); // f64.convert_i64_s
            untag(code, b);
            code.push(0xB9);
            code.push(0xA2); // f64.mul
            code.push(0x99); // f64.abs
            code.push(0x44); // f64.const 2^61
            code.extend_from_slice(&(2f64.powi(61)).to_le_bytes());
            code.push(0x63); // f64.lt
            code.push(0x04);
            code.push(0x40);
            untag(code, a);
            local_get(code, b);
            code.push(0x7E); // i64.mul
            local_set(code, r);
            code.push(0x05);
            slow(code);
            code.push(0x0B);
        }
        Arith::Div | Arith::Mod | Arith::FloorDiv => unreachable!("only a ring operation"),
    }
    code.push(0x05); // else — a held operand
    slow(code);
    code.push(0x0B);
}

/// `[i64 tagged result of its own]` → `dst`: the operands a site tagged for the operation given up, then the
/// result stored — into an exact register, which gives up what it held, or narrowed into a machine word.
fn settle_result(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    dst: u16,
    (a, a_own): (u32, bool),
    (b, b_own): (u32, bool),
) -> R<Flow> {
    if a_own {
        drop_local(code, ctx, a)?;
    }
    if b_own {
        drop_local(code, ctx, b)?;
    }
    if holds(plan, dst) {
        drop_local(code, ctx, dst as u32)?;
        local_set(code, dst as u32);
    } else {
        let r = scratch(plan, RESULT);
        local_set(code, r);
        store_owned(code, plan, ctx, dst, r, FailCode::IntTooWide)?;
    }
    Ok(Flow::Straight)
}

/// `__logos_exact_add`/`_sub`/`_mul(a: i64, b: i64) -> i64` — see [`RuntimeFn::ExactAdd`]: [`ring_op`] over
/// its two parameters, a tagged word of its own answered.
pub(super) fn emit_ring_helper(env: &RuntimeEnv, op: ArithOp) -> Vec<u8> {
    let [_, add, sub, mul, ..] = env
        .exact_host
        .expect("the exact helpers are laid out only in a module that imports exact arithmetic");
    let (arith, host) = match op {
        ArithOp::Add => (Arith::Add, add),
        ArithOp::Sub => (Arith::Sub, sub),
        ArithOp::Mul => (Arith::Mul, mul),
        ArithOp::Div | ArithOp::Mod => unreachable!("a division has no out-of-line helper"),
    };
    let mut c = Vec::new();
    // locals: 2 = the result
    c.push(0x01);
    leb_u32(&mut c, 1);
    c.push(I64);
    ring_op(&mut c, arith, 0, 1, 2, host);
    local_get(&mut c, 2);
    c.push(0x0B); // end function
    c
}

/// `__logos_exact_add_w`/`_sub_w`/`_mul_w(a: i64, w: i64) -> i64` — see [`RuntimeFn::ExactAddW`]: [`ring_op`] over
/// the tagged `a` and the machine word `w`, tagged here and given back after, a tagged word of its own answered.
pub(super) fn emit_ring_word_helper(env: &RuntimeEnv, op: ArithOp) -> Vec<u8> {
    let [boxed, add, sub, mul, ..] = env
        .exact_host
        .expect("the exact helpers are laid out only in a module that imports exact arithmetic");
    let drop = env
        .exact_drop
        .expect("the exact helpers are laid out only in a module that imports exact arithmetic");
    let (arith, host) = match op {
        ArithOp::Add => (Arith::Add, add),
        ArithOp::Sub => (Arith::Sub, sub),
        ArithOp::Mul => (Arith::Mul, mul),
        ArithOp::Div | ArithOp::Mod => unreachable!("a division has no out-of-line helper"),
    };
    let mut c = Vec::new();
    // locals: 2 = `w` tagged, 3 = the result
    c.push(0x01);
    leb_u32(&mut c, 2);
    c.push(I64);
    local_get(&mut c, 1);
    tag_inline(&mut c, boxed, 2);
    local_set(&mut c, 2);
    ring_op(&mut c, arith, 0, 2, 3, host);
    drop_inline(&mut c, drop, 2);
    local_get(&mut c, 3);
    c.push(0x0B); // end function
    c
}

/// `__logos_exact_tag(w: i64) -> i64` — see [`RuntimeFn::ExactTag`]: [`tag_top`]'s inline body over its
/// parameter.
pub(super) fn emit_tag_helper(env: &RuntimeEnv) -> Vec<u8> {
    let [boxed, ..] = env
        .exact_host
        .expect("the exact helpers are laid out only in a module that imports exact arithmetic");
    let mut c = vec![0x00]; // no locals
    local_get(&mut c, 0);
    tag_inline(&mut c, boxed, 0);
    c.push(0x0B); // end function
    c
}

/// `__logos_exact_narrow(t: i64, why: i32) -> i64` — see [`RuntimeFn::ExactNarrow`]: [`narrow_top`]'s inline
/// body over its parameter, failing with the code `why`.
pub(super) fn emit_narrow_helper(env: &RuntimeEnv) -> Vec<u8> {
    let [.., narrow, _] = env
        .exact_host
        .expect("the exact helpers are laid out only in a module that imports exact arithmetic");
    let mut c = vec![0x00]; // no locals
    local_get(&mut c, 0);
    narrow_inline(&mut c, narrow, env.fail_at, 0, |c| local_get(c, 1));
    c.push(0x0B); // end function
    c
}

/// `__logos_exact_clone(t: i64) -> i64` — see [`RuntimeFn::ExactClone`]: [`clone_top`]'s inline body over its
/// parameter.
pub(super) fn emit_clone_helper(env: &RuntimeEnv) -> Vec<u8> {
    let [.., clone] = env
        .exact_host
        .expect("the exact helpers are laid out only in a module that imports exact arithmetic");
    let mut c = vec![0x00]; // no locals
    local_get(&mut c, 0);
    clone_inline(&mut c, clone, 0);
    c.push(0x0B); // end function
    c
}

/// `__logos_exact_release(t: i64)` — see [`RuntimeFn::ExactRelease`]: [`drop_local`]'s inline body over its
/// parameter.
pub(super) fn emit_release_helper(env: &RuntimeEnv) -> Vec<u8> {
    let drop = env
        .exact_drop
        .expect("the exact helpers are laid out only in a module that imports exact arithmetic");
    let mut c = vec![0x00]; // no locals
    drop_inline(&mut c, drop, 0);
    c.push(0x0B); // end function
    c
}

fn floor_div(n: i128, d: i128) -> i128 {
    let q = n / d;
    if n % d != 0 && (n < 0) != (d < 0) {
        q - 1
    } else {
        q
    }
}

fn ceil_div(n: i128, d: i128) -> i128 {
    let q = n / d;
    if n % d != 0 && (n < 0) == (d < 0) {
        q + 1
    } else {
        q
    }
}

/// `dst = x <op> c` for a constant `c` (nonzero for a division): one test that `x` is small and the bound `c`
/// puts on it keeps the result small, the arithmetic on the words, and the host's otherwise.
fn by_constant(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    dst: u16,
    x: u16,
    c: i64,
    op: Arith,
) -> R<Flow> {
    let host = import(ctx, op.host())?;
    let (a, a_own) = tagged_operand(code, plan, ctx, x, None, scratch(plan, OPERAND_A))?;
    let r = scratch(plan, RESULT);
    let tagged_c = c << 1;
    // The range of tagged words `a` the operation keeps within the word: `a op c` neither overflows nor (for
    // a product) leaves the tagged range, which every even word is in.
    let (lo, hi): (i128, i128) = {
        let (min, max) = (i64::MIN as i128, i64::MAX as i128);
        match op {
            Arith::Add => (
                min - (tagged_c as i128).min(0),
                max - (tagged_c as i128).max(0),
            ),
            Arith::Sub => (
                min + (tagged_c as i128).max(0),
                max + (tagged_c as i128).min(0),
            ),
            Arith::Mul if c == 0 => (min, max),
            Arith::Mul if c > 0 => (ceil_div(min, c as i128), floor_div(max, c as i128)),
            Arith::Mul => (ceil_div(max, c as i128), floor_div(min, c as i128)),
            // The small range is one integer wider below than above: −2⁶² divided by −1 is the one quotient it
            // cannot hold, so its word goes to the host.
            Arith::Div | Arith::FloorDiv if c == -1 => (min + 1, max),
            Arith::Div | Arith::Mod | Arith::FloorDiv => (min, max),
        }
    };
    // small and within the bound?
    local_get(code, a);
    is_held(code);
    code.push(0x45); // i32.eqz
    if lo > i64::MIN as i128 {
        local_get(code, a);
        i64_const(code, lo as i64);
        code.push(0x59); // i64.ge_s
        code.push(0x71); // i32.and
    }
    if hi < i64::MAX as i128 {
        local_get(code, a);
        i64_const(code, hi as i64);
        code.push(0x57); // i64.le_s
        code.push(0x71);
    }
    code.push(0x04);
    code.push(0x40);
    match op {
        Arith::Add | Arith::Sub => {
            local_get(code, a);
            i64_const(code, tagged_c);
            code.push(if op == Arith::Add { 0x7C } else { 0x7D });
        }
        Arith::Mul => {
            local_get(code, a);
            i64_const(code, c);
            code.push(0x7E);
        }
        Arith::Div | Arith::Mod => {
            untag(code, a);
            i64_const(code, c);
            code.push(if op == Arith::Div { 0x7F } else { 0x81 });
            i64_const(code, 1);
            code.push(0x86);
        }
        Arith::FloorDiv => {
            untag(code, a);
            i64_const(code, c);
            code.push(0x7F);
            untag(code, a);
            i64_const(code, c);
            code.push(0x81);
            i64_const(code, 0);
            code.push(0x52); // i64.ne
                             // The signs differ: `a` is negative exactly when its word is, and `c`'s sign is known.
            local_get(code, a);
            i64_const(code, 0);
            code.push(if c < 0 { 0x59 } else { 0x53 }); // i64.ge_s / i64.lt_s
            code.push(0x71);
            code.push(0xAD);
            code.push(0x7D);
            i64_const(code, 1);
            code.push(0x86);
        }
    }
    local_set(code, r);
    code.push(0x05); // else — a held operand, or one the bound sends to the host
    local_get(code, a);
    i64_const(code, tagged_c);
    call(code, host);
    local_set(code, r);
    code.push(0x0B);
    if a_own {
        drop_local(code, ctx, a)?;
    }
    store_owned(code, plan, ctx, dst, r, FailCode::IntTooWide)?;
    Ok(Flow::Straight)
}

#[derive(Clone, Copy)]
enum Order {
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
}

/// `dst = lhs <cmp> rhs` on tagged words: two small ones order as their integers do; otherwise the
/// host's three-way comparison against zero. A constant operand is tagged at compile time.
fn compare(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    dst: u16,
    lhs: u16,
    rhs: u16,
    order: Order,
    consts: (Option<i64>, Option<i64>),
) -> R<Flow> {
    let cmp = import(ctx, HostFn::ExactCmp)?;
    let (a, a_own) = tagged_operand(code, plan, ctx, lhs, consts.0, scratch(plan, OPERAND_A))?;
    let (b, b_own) = tagged_operand(code, plan, ctx, rhs, consts.1, scratch(plan, OPERAND_B))?;
    let (on_words, on_order) = match order {
        Order::Lt => (0x53, 0x48),
        Order::Gt => (0x55, 0x4A),
        Order::Le => (0x57, 0x4C),
        Order::Ge => (0x59, 0x4E),
        Order::Eq => (0x51, 0x46),
        Order::Ne => (0x52, 0x47),
    };
    if let Some(helper) = out_of_line(ctx, RuntimeFn::ExactCompare) {
        local_get(code, a);
        local_get(code, b);
        call(code, helper);
        i32_const(code, 0);
        code.push(on_order);
    } else {
        local_get(code, a);
        local_get(code, b);
        code.push(0x84);
        i64_const(code, 1);
        code.push(0x83);
        code.push(0x50);
        code.push(0x04);
        code.push(I32); // if (result i32)
        local_get(code, a);
        local_get(code, b);
        code.push(on_words);
        code.push(0x05);
        local_get(code, a);
        local_get(code, b);
        call(code, cmp);
        i32_const(code, 0);
        code.push(on_order);
        code.push(0x0B);
    }
    code.push(0xAD); // i64.extend_i32_u
    local_set(code, scratch(plan, RESULT));
    if a_own {
        drop_local(code, ctx, a)?;
    }
    if b_own {
        drop_local(code, ctx, b)?;
    }
    local_get(code, scratch(plan, RESULT));
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// Compare borrowed tagged integers once per module, with the small-word path native.
pub(super) fn emit_compare_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = vec![0];
    local_get(&mut c, 0);
    local_get(&mut c, 1);
    c.push(0x84);
    i64_const(&mut c, 1);
    c.extend([0x83, 0x50, 0x04, I32]);
    local_get(&mut c, 0);
    local_get(&mut c, 1);
    c.push(0x55); // lhs > rhs
    local_get(&mut c, 0);
    local_get(&mut c, 1);
    c.extend([0x53, 0x6b]); // (lhs > rhs) - (lhs < rhs)
    c.push(0x05);
    local_get(&mut c, 0);
    local_get(&mut c, 1);
    call(&mut c, env.exact_cmp.expect("exact comparison import"));
    c.extend([0x0b, 0x0b]);
    c
}

/// `dst = base ** exp`, the host's exact power. A negative exponent, or one past what the reference tier
/// raises to, stops the program as the machine-word power does.
fn power(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, dst: u16, base: u16, exp: u16) -> R<Flow> {
    let pow = import(ctx, HostFn::ExactPow)?;
    let e = scratch(plan, OPERAND_B);
    local_get(code, exp as u32);
    if holds(plan, exp) {
        narrow_top(code, ctx, scratch(plan, OPERAND_A), FailCode::IntTooWide)?;
    }
    local_set(code, e);
    local_get(code, e);
    i64_const(code, 0);
    code.push(0x53); // i64.lt_s
    local_get(code, e);
    i64_const(code, u32::MAX as i64);
    code.push(0x55); // i64.gt_s
    code.push(0x72); // i32.or
    code.push(0x04);
    code.push(0x40);
    code.push(0x00);
    code.push(0x0B);
    let (a, a_own) = tagged_operand(code, plan, ctx, base, None, scratch(plan, OPERAND_A))?;
    local_get(code, a);
    local_get(code, e);
    call(code, pow);
    local_set(code, scratch(plan, RESULT));
    if a_own {
        drop_local(code, ctx, a)?;
    }
    store_owned(
        code,
        plan,
        ctx,
        dst,
        scratch(plan, RESULT),
        FailCode::IntTooWide,
    )?;
    Ok(Flow::Straight)
}

/// `dst = lhs <op> rhs` in the Real domain, an exact operand converted as the reference tier converts
/// an `Int` it widens.
fn real_arith(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    dst: u16,
    lhs: u16,
    rhs: u16,
    opcode: u8,
) -> R<Flow> {
    let to_f64 = import(ctx, HostFn::ExactF64)?;
    for r in [lhs, rhs] {
        if holds(plan, r) {
            local_get(code, r as u32);
            call(code, to_f64);
        } else {
            push_as_f64(code, r, plan.kinds.get(r as usize))?;
        }
    }
    code.push(opcode);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// `dst = src`, each side in its own form: a tagged copy cloned, a word tagged into an exact register, an
/// exact value narrowed into a machine word or widened into a Real.
fn copy(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, dst: u16, src: u16) -> R<Flow> {
    if dst == src {
        return Ok(Flow::Straight);
    }
    let result = scratch(plan, RESULT);
    match (holds(plan, dst), holds(plan, src)) {
        (true, true) => {
            // Two small words: the copy is the word. Otherwise the held one is cloned, the old one given up.
            local_get(code, src as u32);
            local_get(code, dst as u32);
            code.push(0x84); // i64.or
            is_held(code);
            code.push(0x45); // i32.eqz
            code.push(0x04);
            code.push(0x40);
            local_get(code, src as u32);
            local_set(code, dst as u32);
            code.push(0x05);
            local_get(code, src as u32);
            clone_top(code, ctx, scratch(plan, OPERAND_A))?;
            local_set(code, result);
            drop_local(code, ctx, dst as u32)?;
            local_get(code, result);
            local_set(code, dst as u32);
            code.push(0x0B);
        }
        (true, false) => {
            local_get(code, src as u32);
            tag_top(code, ctx, scratch(plan, OPERAND_A))?;
            local_set(code, result);
            drop_local(code, ctx, dst as u32)?;
            local_get(code, result);
            local_set(code, dst as u32);
        }
        (false, _) if plan.kinds.get(dst as usize) == Some(Kind::Float) => {
            local_get(code, src as u32);
            call(code, import(ctx, HostFn::ExactF64)?);
            local_set(code, dst as u32);
        }
        (false, _) => {
            local_get(code, src as u32);
            narrow_top(code, ctx, scratch(plan, OPERAND_A), FailCode::IntTooWide)?;
            local_set(code, dst as u32);
        }
    }
    Ok(Flow::Straight)
}

/// `Swap a and b` where one side is exact and the other a machine word: each takes the other's integer
/// in its own form.
fn swap(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, a: u16, b: u16) -> R<Flow> {
    let (exact, word) = if holds(plan, a) { (a, b) } else { (b, a) };
    let result = scratch(plan, RESULT);
    local_get(code, word as u32);
    tag_top(code, ctx, scratch(plan, OPERAND_A))?;
    local_set(code, result);
    local_get(code, exact as u32);
    narrow_top(code, ctx, scratch(plan, OPERAND_A), FailCode::IntTooWide)?;
    local_set(code, word as u32);
    drop_local(code, ctx, exact as u32)?;
    local_get(code, result);
    local_set(code, exact as u32);
    Ok(Flow::Straight)
}

/// `dst = constant`, tagged at compile time.
fn load_const(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, dst: u16, idx: u32) -> R<Flow> {
    let v = match ctx.constants.get(idx as usize) {
        Some(Constant::Int(v)) => *v,
        Some(Constant::Bool(b)) => i64::from(*b),
        Some(Constant::Char(c)) => i64::from(*c as u32),
        Some(Constant::Nothing) => 0,
        _ => {
            return Err(WasmLowerError::Unsupported(
                "a constant of another kind into an exact integer",
            ))
        }
    };
    let result = scratch(plan, RESULT);
    if fits_small(v) {
        i64_const(code, v << 1);
    } else {
        i64_const(code, v);
        call(code, import(ctx, HostFn::ExactBox)?);
    }
    local_set(code, result);
    drop_local(code, ctx, dst as u32)?;
    local_get(code, result);
    local_set(code, dst as u32);
    Ok(Flow::Straight)
}

/// The argument for parameter `p` of `callee`, pushed in the parameter's form. A tagged parameter takes
/// a word of its own — a clone, or a word tagged for it — except a `mutable` one, which takes the
/// caller's own and gives it back.
fn push_argument(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    callee: usize,
    p: u16,
    reg: u16,
    lent: bool,
) -> R<()> {
    if ctx.exact.param(callee, p as usize) {
        local_get(code, reg as u32);
        match (holds(plan, reg), lent) {
            (true, true) => {}
            (true, false) => clone_top(code, ctx, scratch(plan, OPERAND_A))?,
            (false, _) => tag_top(code, ctx, scratch(plan, OPERAND_A))?,
        }
    } else if holds(plan, reg) {
        let param_vt = ctx
            .fn_param_valtypes
            .get(callee)
            .and_then(|v| v.get(p as usize))
            .copied()
            .unwrap_or(I64);
        local_get(code, reg as u32);
        if param_vt == F64 {
            call(code, import(ctx, HostFn::ExactF64)?);
        } else {
            narrow_top(code, ctx, scratch(plan, OPERAND_A), FailCode::IntTooWide)?;
        }
    } else {
        let param_vt = ctx
            .fn_param_valtypes
            .get(callee)
            .and_then(|v| v.get(p as usize))
            .copied()
            .unwrap_or(I64);
        let arg_vt = plan.kinds.valtype(reg as usize);
        if arg_vt == param_vt {
            local_get(code, reg as u32);
        } else if arg_vt == I64 && param_vt == F64 {
            push_as_f64(code, reg, plan.kinds.get(reg as usize))?;
        } else {
            return Err(WasmLowerError::Unsupported(
                "call argument type does not match the parameter",
            ));
        }
    }
    Ok(())
}

/// Store a callee's result on the stack into `dst`, `tagged` as the callee hands it over.
fn take_result(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, dst: u16, tagged: bool) -> R<()> {
    let result = scratch(plan, RESULT);
    match (tagged, holds(plan, dst)) {
        (true, _) => {
            local_set(code, result);
            store_owned(code, plan, ctx, dst, result, FailCode::IntTooWide)?;
        }
        (false, true) => set_word(code, plan, ctx, dst)?,
        (false, false) => local_set(code, dst as u32),
    }
    Ok(())
}

/// `dst = func(args)`, each argument in its parameter's form, each `mutable` parameter's final value back
/// into the register it was lent from, the result in `dst`'s form.
fn direct_call(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    dst: u16,
    func: u16,
    args_start: u16,
    arg_count: u16,
) -> R<Flow> {
    let callee = func as usize;
    if ctx.fn_stubbed.get(callee).copied().unwrap_or(false) {
        code.push(0x00);
        return Ok(Flow::Straight);
    }
    let inout = ctx.fn_inout.get(callee).cloned().unwrap_or_default();
    let pvts = ctx
        .fn_param_valtypes
        .get(callee)
        .ok_or(WasmLowerError::Unsupported("call of unknown function"))?;
    for a in 0..arg_count {
        let arg = args_start + a;
        if counts_aliases(plan.kinds.get(arg as usize)) && !inout.contains(&a) {
            emit_retain(code, ctx, arg);
        }
    }
    for a in 0..arg_count {
        push_argument(
            code,
            plan,
            ctx,
            callee,
            a,
            args_start + a,
            inout.contains(&a),
        )?;
    }
    call(code, ctx.fn_base + func as u32);
    for p in inout.iter().rev() {
        let arg = args_start + p;
        if plan.kinds.valtype(arg as usize) != pvts.get(*p as usize).copied().unwrap_or(I64) {
            return Err(WasmLowerError::Unsupported(
                "a `mutable` argument of another type than its parameter",
            ));
        }
        let tagged = ctx.exact.param(callee, *p as usize);
        // A tagged argument lent from an exact register went to the callee as the register's own word
        // and comes back as this one; anything else the register held is given up for it.
        if tagged && holds(plan, arg) {
            local_set(code, arg as u32);
        } else {
            take_result(code, plan, ctx, arg, tagged)?;
        }
    }
    if ctx.fn_results.get(callee).copied().flatten().is_some() {
        take_result(code, plan, ctx, dst, ctx.exact.ret(callee))?;
    }
    Ok(Flow::Straight)
}

/// A closure call's argument `k` for body `callee`, pushed in its parameter's form ([`push_argument`]).
pub(super) fn push_closure_argument(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    callee: u16,
    k: u16,
    reg: u16,
) -> R<()> {
    if on(plan) {
        push_argument(code, plan, ctx, callee as usize, k, reg, false)
    } else {
        local_get(code, reg as u32);
        Ok(())
    }
}

/// A closure call's result on the stack into `dst`, as body `callee` hands it over ([`take_result`]).
pub(super) fn take_closure_result(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    callee: u16,
    dst: u16,
) -> R<()> {
    if on(plan) {
        take_result(code, plan, ctx, dst, ctx.exact.ret(callee as usize))
    } else {
        local_set(code, dst as u32);
        Ok(())
    }
}

/// The host's argument to a closure it calls back (`__logos_call_closure`), a machine word, in the form the
/// closure's parameter has: tagged in local `s` when the parameter is.
pub(super) fn trampoline_argument(code: &mut Vec<u8>, ctx: &Ctx, closure: usize, s: u32) -> R<()> {
    if ctx.exact.param(closure, 0) {
        tag_top(code, ctx, s)?;
    }
    Ok(())
}

/// A closure's result back to the host that called it, a machine word: a tagged one narrowed and given up.
pub(super) fn trampoline_result(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    closure: usize,
    s: u32,
    held: u32,
) -> R<()> {
    if ctx.exact.ret(closure) {
        local_tee(code, held);
        narrow_top(code, ctx, s, FailCode::IntTooWide)?;
        drop_local(code, ctx, held)?;
    }
    Ok(())
}

/// A task's call: the callee runs to completion here, its arguments in its parameters' forms, its result
/// discarded — given up first when the host holds it.
fn task_call(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    func: u16,
    args_start: u16,
    arg_count: u16,
) -> R<()> {
    let callee = func as usize;
    if ctx.fn_stubbed.get(callee).copied().unwrap_or(false) {
        code.push(0x00);
        return Ok(());
    }
    if ctx.fn_inout.get(callee).is_some_and(|ps| !ps.is_empty()) {
        return Err(WasmLowerError::Unsupported(
            "a task that lends a variable to a `mutable` parameter",
        ));
    }
    for a in 0..arg_count {
        let arg = args_start + a;
        if counts_aliases(plan.kinds.get(arg as usize)) {
            emit_retain(code, ctx, arg);
        }
    }
    for a in 0..arg_count {
        push_argument(code, plan, ctx, callee, a, args_start + a, false)?;
    }
    call(code, ctx.fn_base + func as u32);
    if ctx.fn_results.get(callee).copied().flatten().is_some() {
        if ctx.exact.ret(callee) {
            local_set(code, scratch(plan, RESULT));
            drop_local(code, ctx, scratch(plan, RESULT))?;
        } else {
            code.push(0x1A); // drop
        }
    }
    Ok(())
}

fn global_get_tagged(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, dst: u16, idx: u16) -> R<Flow> {
    global_get(code, idx as u32);
    if ctx.exact.global(idx as usize) {
        clone_top(code, ctx, scratch(plan, OPERAND_A))?;
        local_set(code, scratch(plan, RESULT));
        store_owned(
            code,
            plan,
            ctx,
            dst,
            scratch(plan, RESULT),
            FailCode::IntTooWide,
        )?;
    } else {
        set_word(code, plan, ctx, dst)?;
    }
    Ok(Flow::Straight)
}

fn global_set_tagged(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, idx: u16, src: u16) -> R<Flow> {
    local_get(code, src as u32);
    if ctx.exact.global(idx as usize) {
        if holds(plan, src) {
            clone_top(code, ctx, scratch(plan, OPERAND_A))?;
        } else {
            tag_top(code, ctx, scratch(plan, OPERAND_A))?;
        }
        local_set(code, scratch(plan, RESULT));
        global_get(code, idx as u32);
        local_set(code, scratch(plan, OPERAND_B));
        drop_local(code, ctx, scratch(plan, OPERAND_B))?;
        local_get(code, scratch(plan, RESULT));
    } else {
        narrow_top(code, ctx, scratch(plan, OPERAND_A), FailCode::IntTooWide)?;
    }
    global_set(code, idx as u32);
    Ok(Flow::Straight)
}

/// On every way out of the function: give up each exact register's integer but those in `keep` and the
/// `mutable` parameters, whose words go to the caller.
fn release(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, keep: &[u16]) -> R<()> {
    for r in 0..plan.exact.len() as u16 {
        if holds(plan, r) && !keep.contains(&r) && !plan.inout.contains(&r) {
            drop_local(code, ctx, r as u32)?;
        }
    }
    Ok(())
}

/// A `mutable` parameter the body holds exact but the boundary passes as a machine word goes back
/// narrowed: its integer narrowed into the register, the word given up.
fn settle_inout(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx) -> R<()> {
    for &p in &plan.inout {
        if holds(plan, p) && !plan.params_tagged.get(p as usize).copied().unwrap_or(false) {
            local_get(code, p as u32);
            narrow_top(code, ctx, scratch(plan, OPERAND_A), FailCode::IntTooWide)?;
            local_set(code, scratch(plan, OPERAND_B));
            drop_local(code, ctx, p as u32)?;
            local_get(code, scratch(plan, OPERAND_B));
            local_set(code, p as u32);
        }
    }
    Ok(())
}

/// The way out of a function that falls off its end or returns nothing: its integers given up, its
/// `mutable` parameters handed back.
pub(super) fn leave(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx) -> R<()> {
    if on(plan) {
        settle_inout(code, plan, ctx)?;
        release(code, plan, ctx, &[])?;
    }
    push_inout_results(code, plan);
    Ok(())
}

/// `Return src`: the value in the form the function's result has at its boundary, moved out of `src`
/// when both are tagged.
fn return_value(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, src: u16) -> R<Flow> {
    if plan.result.is_none() {
        return Err(WasmLowerError::Unsupported(
            "value return from a void function",
        ));
    }
    emit_leave_loops(code, plan, ctx);
    if !plan.ret_tagged && !holds(plan, src) {
        // The value keeps its form, whatever it is: read after the integers are given up.
        settle_inout(code, plan, ctx)?;
        release(code, plan, ctx, &[])?;
        local_get(code, src as u32);
        push_inout_results(code, plan);
        code.push(0x0F);
        return Ok(Flow::Terminated);
    }
    let result = scratch(plan, RESULT);
    let mut keep = Vec::new();
    match (plan.ret_tagged, holds(plan, src)) {
        (true, true) if plan.inout.contains(&src) => {
            local_get(code, src as u32);
            clone_top(code, ctx, scratch(plan, OPERAND_A))?;
        }
        (true, true) => {
            local_get(code, src as u32);
            keep.push(src);
        }
        (true, false) => {
            local_get(code, src as u32);
            tag_top(code, ctx, scratch(plan, OPERAND_A))?;
        }
        (false, _) => {
            local_get(code, src as u32);
            narrow_top(code, ctx, scratch(plan, OPERAND_A), FailCode::IntTooWide)?;
        }
    }
    local_set(code, result);
    settle_inout(code, plan, ctx)?;
    release(code, plan, ctx, &keep)?;
    local_get(code, result);
    push_inout_results(code, plan);
    code.push(0x0F);
    Ok(Flow::Terminated)
}

/// A function that keeps machine words at its boundary tags the parameters its body holds exact on entry.
pub(super) fn prologue(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx) -> R<()> {
    for p in 0..plan.num_params as u16 {
        if holds(plan, p) && !plan.params_tagged.get(p as usize).copied().unwrap_or(false) {
            local_get(code, p as u32);
            tag_top(code, ctx, scratch(plan, OPERAND_A))?;
            local_set(code, p as u32);
        }
    }
    Ok(())
}

/// Render exact register `reg` as its Text into local `out`, when it is one.
pub(super) fn display(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, reg: u16, out: u32) -> R<bool> {
    if !holds(plan, reg) {
        return Ok(false);
    }
    local_get(code, reg as u32);
    call(code, import(ctx, HostFn::ExactText)?);
    local_set(code, out);
    Ok(true)
}

/// The sentence an op's narrowing of exact register `r` fails with: the reference tier's own where it
/// refuses an index past a word, and the one for a place the module holds a machine word anywhere else.
fn narrowing_failure(plan: &Plan, op: &Op, r: u16) -> FailCode {
    let sequence = |c: u16| {
        plan.kinds
            .get(c as usize)
            .and_then(Kind::seq_elem)
            .is_some()
    };
    match *op {
        Op::Index {
            collection, index, ..
        }
        | Op::IndexUnchecked {
            collection, index, ..
        } if index == r => {
            if plan.kinds.get(collection as usize) == Some(Kind::Text) {
                FailCode::TextIndexTooWide
            } else if sequence(collection) {
                FailCode::ListIndexTooWide
            } else {
                FailCode::IntTooWide
            }
        }
        Op::SetIndex {
            collection, index, ..
        }
        | Op::SetIndexUnchecked {
            collection, index, ..
        } if index == r && sequence(collection) => FailCode::ListStoreIndexTooWide,
        _ => FailCode::IntTooWide,
    }
}

/// An op the module lowers on machine words only: each exact register it reads narrowed for it and
/// restored after, each it writes tagged after, its earlier integer given up.
fn on_words(
    pc: usize,
    plan: &Plan,
    ctx: &Ctx,
    blocks: &Blocks,
    k: usize,
    code: &mut Vec<u8>,
) -> R<Flow> {
    let op = &plan.ops[pc];
    let mut reads: Vec<u16> = Vec::new();
    op.for_each_read_reg(
        |f| {
            ctx.functions
                .get(f as usize)
                .map_or(0, |f| f.captures.len())
        },
        |r| {
            if holds(plan, r) && !reads.contains(&r) {
                reads.push(r);
            }
        },
    );
    let mut writes: Vec<u16> = Vec::new();
    op.for_each_written_reg(|w| {
        if holds(plan, w) && !writes.contains(&w) {
            writes.push(w);
        }
    });
    if reads.is_empty() && writes.is_empty() {
        return lower_op(pc, plan, ctx, blocks, k, code);
    }
    for &r in reads
        .iter()
        .chain(writes.iter().filter(|w| !reads.contains(w)))
    {
        local_get(code, r as u32);
        local_set(code, saved(plan, r));
    }
    for &r in &reads {
        local_get(code, r as u32);
        narrow_top(
            code,
            ctx,
            scratch(plan, OPERAND_A),
            narrowing_failure(plan, op, r),
        )?;
        local_set(code, r as u32);
    }
    let flow = lower_op(pc, plan, ctx, blocks, k, code)?;
    if flow == Flow::Terminated {
        return Err(WasmLowerError::Unsupported(
            "an exact integer at a branch the module lowers on machine words",
        ));
    }
    for &r in reads.iter().filter(|r| !writes.contains(r)) {
        local_get(code, saved(plan, r));
        local_set(code, r as u32);
    }
    for &w in &writes {
        drop_local(code, ctx, saved(plan, w))?;
        local_get(code, w as u32);
        tag_top(code, ctx, scratch(plan, OPERAND_A))?;
        local_set(code, w as u32);
    }
    Ok(flow)
}

/// Lower one op of a module that carries tagged words: the ops that work on them as they are, and every
/// other one on machine words ([`on_words`]).
pub(super) fn lower(
    pc: usize,
    plan: &Plan,
    ctx: &Ctx,
    blocks: &Blocks,
    k: usize,
    code: &mut Vec<u8>,
) -> R<Flow> {
    if !on(plan) {
        return lower_op(pc, plan, ctx, blocks, k, code);
    }
    if ctx.exact.map_ints {
        if let Some(flow) = map::lower(pc, plan, ctx, code)? {
            return Ok(flow);
        }
    }
    if ctx.exact.sequence_ints {
        if let Some(flow) = list::lower(pc, plan, ctx, blocks, k, code)? {
            return Ok(flow);
        }
    }
    let exact = |r: u16| holds(plan, r);
    let kind = |r: u16| plan.kinds.get(r as usize);
    let int = |r: u16| kind(r) == Some(Kind::Int);
    let words = |code: &mut Vec<u8>| on_words(pc, plan, ctx, blocks, k, code);
    let constant = |r: u16| constant_at(plan, ctx, blocks.start(k), pc, r);
    match plan.ops[pc] {
        // Equality with absence depends on the value kind, never on the
        // integer payload. Keep a held exact integer intact for this check.
        Op::Eq { lhs, rhs, .. } | Op::NotEq { lhs, rhs, .. }
            if kind(lhs) == Some(Kind::Nothing) || kind(rhs) == Some(Kind::Nothing) =>
        {
            lower_op(pc, plan, ctx, blocks, k, code)
        }

        Op::StructInsert { obj, value, .. } if ctx.exact.record_ints && int(value) => {
            let slot =
                plan.structs
                    .slot
                    .get(pc)
                    .copied()
                    .flatten()
                    .ok_or(WasmLowerError::Unsupported(
                        "exact record write without a field slot",
                    ))?;
            if integer_field(plan, ctx, obj, slot) {
                record_integer_store(
                    code,
                    plan,
                    ctx,
                    obj,
                    slot,
                    Some(value),
                    plan.cow_inserts.get(pc).copied().unwrap_or(true),
                )
            } else {
                words(code)
            }
        }
        Op::GetField { dst, obj, .. } if ctx.exact.record_ints && int(dst) => {
            let slot =
                plan.structs
                    .slot
                    .get(pc)
                    .copied()
                    .flatten()
                    .ok_or(WasmLowerError::Unsupported(
                        "exact record read without a field slot",
                    ))?;
            if integer_field(plan, ctx, obj, slot) {
                record_integer_load(code, plan, ctx, obj, slot, dst)
            } else {
                words(code)
            }
        }
        Op::SetIndex {
            collection, value, ..
        }
        | Op::SetIndexUnchecked {
            collection, value, ..
        } if ctx.exact.record_ints && kind(collection) == Some(Kind::Struct) => {
            let slot = plan
                .place
                .field_at
                .get(pc)
                .copied()
                .flatten()
                .and_then(|field| record_field_slot(plan, collection, field));
            if let Some(slot) =
                slot.filter(|slot| integer_field(plan, ctx, collection, *slot as u16))
            {
                let clear = plan.place.clear.get(pc).copied().unwrap_or(false);
                if clear || int(value) {
                    record_integer_store(
                        code,
                        plan,
                        ctx,
                        collection,
                        slot as u16,
                        (!clear).then_some(value),
                        plan.cow_inserts.get(pc).copied().unwrap_or(true),
                    )
                } else {
                    words(code)
                }
            } else {
                words(code)
            }
        }
        Op::LoadConst { dst, idx } if exact(dst) => load_const(code, plan, ctx, dst, idx),
        Op::Move { dst, src } | Op::DeepClone { dst, src } if exact(dst) || exact(src) => {
            copy(code, plan, ctx, dst, src)
        }
        Op::RevSwap { a, b } if exact(a) != exact(b) => swap(code, plan, ctx, a, b),
        // Both tagged or both words: the exchange moves each register's own word.
        Op::RevSwap { .. } | Op::EnsureOwned { .. } => lower_op(pc, plan, ctx, blocks, k, code),
        Op::Add { dst, lhs, rhs }
        | Op::Sub { dst, lhs, rhs }
        | Op::Mul { dst, lhs, rhs }
        | Op::Div { dst, lhs, rhs }
            if kind(dst) == Some(Kind::Float) && (exact(lhs) || exact(rhs)) =>
        {
            let opcode = match plan.ops[pc] {
                Op::Add { .. } => 0xA0,
                Op::Sub { .. } => 0xA1,
                Op::Mul { .. } => 0xA2,
                _ => 0xA3,
            };
            real_arith(code, plan, ctx, dst, lhs, rhs, opcode)
        }
        // A Text `+` renders its operands, an exact one through the host.
        Op::Add { dst, .. } | Op::AddAssign { dst, .. } if kind(dst) == Some(Kind::Text) => {
            lower_op(pc, plan, ctx, blocks, k, code)
        }
        Op::Add { dst, lhs, rhs }
        | Op::Sub { dst, lhs, rhs }
        | Op::Mul { dst, lhs, rhs }
        | Op::Div { dst, lhs, rhs }
        | Op::Mod { dst, lhs, rhs }
        | Op::FloorDiv { dst, lhs, rhs }
            if int(lhs) && int(rhs) && (exact(dst) || exact(lhs) || exact(rhs)) =>
        {
            let op = match plan.ops[pc] {
                Op::Add { .. } => Arith::Add,
                Op::Sub { .. } => Arith::Sub,
                Op::Mul { .. } => Arith::Mul,
                Op::Div { .. } => Arith::Div,
                Op::Mod { .. } => Arith::Mod,
                _ => Arith::FloorDiv,
            };
            binop(
                code,
                plan,
                ctx,
                dst,
                lhs,
                rhs,
                op,
                (constant(lhs), constant(rhs)),
            )
        }
        Op::AddAssign { dst, src } if int(dst) && int(src) && (exact(dst) || exact(src)) => binop(
            code,
            plan,
            ctx,
            dst,
            dst,
            src,
            Arith::Add,
            (constant(dst), constant(src)),
        ),
        // A power-of-two quotient is a quotient by a constant.
        Op::DivPow2 { dst, lhs, k } if exact(lhs) && k < 62 => {
            by_constant(code, plan, ctx, dst, lhs, 1i64 << k, Arith::Div)
        }
        // A held dividend divides as the reference tier divides one: exactly, by the constant the
        // reciprocal stands for. A small one keeps the machine-word lowering, bit for bit.
        Op::DivPow2 { dst, lhs, .. } | Op::MagicDivU { dst, lhs, .. }
            if exact(dst) || exact(lhs) =>
        {
            let (divisor, op) = match plan.ops[pc] {
                Op::DivPow2 { k, .. } => (1i64 << k, Arith::Div),
                Op::MagicDivU {
                    divisor, mul_back, ..
                } => (
                    divisor,
                    if mul_back == 0 {
                        Arith::Div
                    } else {
                        Arith::Mod
                    },
                ),
                _ => unreachable!(),
            };
            divide_by_constant(pc, plan, ctx, blocks, k, code, dst, lhs, divisor, op)
        }
        Op::Pow { dst, lhs, rhs }
            if int(dst) && int(lhs) && int(rhs) && (exact(dst) || exact(lhs) || exact(rhs)) =>
        {
            power(code, plan, ctx, dst, lhs, rhs)
        }
        Op::Lt { dst, lhs, rhs }
        | Op::Gt { dst, lhs, rhs }
        | Op::LtEq { dst, lhs, rhs }
        | Op::GtEq { dst, lhs, rhs }
        | Op::Eq { dst, lhs, rhs }
        | Op::NotEq { dst, lhs, rhs }
            if int(lhs) && int(rhs) && (exact(lhs) || exact(rhs)) =>
        {
            let order = match plan.ops[pc] {
                Op::Lt { .. } => Order::Lt,
                Op::Gt { .. } => Order::Gt,
                Op::LtEq { .. } => Order::Le,
                Op::GtEq { .. } => Order::Ge,
                Op::Eq { .. } => Order::Eq,
                _ => Order::Ne,
            };
            compare(
                code,
                plan,
                ctx,
                dst,
                lhs,
                rhs,
                order,
                (constant(lhs), constant(rhs)),
            )
        }
        Op::Show { src } if exact(src) => {
            local_get(code, src as u32);
            call(code, import(ctx, HostFn::PrintExact)?);
            Ok(Flow::Straight)
        }
        // An interpolation renders its operands; a precision spec formats a Real.
        Op::Concat { .. } => lower_op(pc, plan, ctx, blocks, k, code),
        Op::FormatValue { spec, .. } if !is_precision_spec(ctx, spec) => {
            lower_op(pc, plan, ctx, blocks, k, code)
        }
        // These builtins render their argument through the exact display path.
        Op::CallBuiltin {
            builtin: BuiltinId::Format | BuiltinId::Fail,
            args_start,
            arg_count,
            ..
        } if arg_count > 0 && exact(args_start) => lower_op(pc, plan, ctx, blocks, k, code),
        // Truthiness: a tagged word is zero exactly when its integer is.
        Op::JumpIfFalse { .. } | Op::JumpIfTrue { .. } => lower_op(pc, plan, ctx, blocks, k, code),
        Op::Return { src } => return_value(code, plan, ctx, src),
        Op::ReturnNothing => {
            if plan.result.is_some() {
                code.push(0x00);
            } else {
                emit_leave_loops(code, plan, ctx);
                leave(code, plan, ctx)?;
                code.push(0x0F);
            }
            Ok(Flow::Terminated)
        }
        Op::Halt => {
            emit_leave_loops(code, plan, ctx);
            release(code, plan, ctx, &[])?;
            if ctx.batch_reclaim {
                call(
                    code,
                    ctx.runtime
                        .and_then(|r| r.index(RuntimeFn::Safepoint))
                        .expect("batch safepoint"),
                );
            }
            code.push(0x0F);
            Ok(Flow::Terminated)
        }
        Op::Call {
            dst,
            func,
            args_start,
            arg_count,
        } => direct_call(code, plan, ctx, dst, func, args_start, arg_count),
        // Its arguments and result take the closure body's forms ([`push_closure_argument`]).
        Op::CallValue { .. } => lower_op(pc, plan, ctx, blocks, k, code),
        Op::Spawn {
            func,
            args_start,
            arg_count,
        } => {
            task_call(code, plan, ctx, func, args_start, arg_count)?;
            Ok(Flow::Straight)
        }
        Op::SpawnHandle {
            dst,
            func,
            args_start,
            arg_count,
        } => {
            task_call(code, plan, ctx, func, args_start, arg_count)?;
            i64_const(code, 0);
            set_word(code, plan, ctx, dst)?;
            Ok(Flow::Straight)
        }
        Op::GlobalGet { dst, idx } if ctx.exact.global(idx as usize) || exact(dst) => {
            global_get_tagged(code, plan, ctx, dst, idx)
        }
        Op::GlobalSet { idx, src } if ctx.exact.global(idx as usize) || exact(src) => {
            global_set_tagged(code, plan, ctx, idx, src)
        }
        // The element lands in `dst` through [`set_word`].
        Op::IterNext { .. } => lower_op(pc, plan, ctx, blocks, k, code),
        Op::SelectArmRecv { var, .. } if exact(var) => Err(WasmLowerError::Unsupported(
            "a channel received into an exact integer",
        )),
        _ => words(code),
    }
}

fn is_precision_spec(ctx: &Ctx, spec: u32) -> bool {
    spec != u32::MAX
        && matches!(ctx.constants.get(spec as usize), Some(Constant::Text(s)) if s.starts_with('.'))
}

/// `dst = lhs / divisor` (or `%`) for an op that divides by a constant: on a small dividend the machine-word
/// lowering, bit for bit; on a held one the host's exact division.
#[allow(clippy::too_many_arguments)]
fn divide_by_constant(
    pc: usize,
    plan: &Plan,
    ctx: &Ctx,
    blocks: &Blocks,
    k: usize,
    code: &mut Vec<u8>,
    dst: u16,
    lhs: u16,
    divisor: i64,
    op: Arith,
) -> R<Flow> {
    if !holds(plan, lhs) {
        return on_words(pc, plan, ctx, blocks, k, code);
    }
    let host = import(ctx, op.host())?;
    local_get(code, lhs as u32);
    is_held(code);
    code.push(0x45); // i32.eqz
    code.push(0x04);
    code.push(0x40);
    on_words(pc, plan, ctx, blocks, k, code)?;
    code.push(0x05);
    let divisor_word = scratch(plan, OPERAND_B);
    i64_const(code, divisor);
    tag_top(code, ctx, divisor_word)?;
    local_set(code, divisor_word);
    local_get(code, lhs as u32);
    local_get(code, divisor_word);
    call(code, host);
    local_set(code, scratch(plan, RESULT));
    drop_local(code, ctx, divisor_word)?;
    store_owned(
        code,
        plan,
        ctx,
        dst,
        scratch(plan, RESULT),
        FailCode::IntTooWide,
    )?;
    code.push(0x0B);
    Ok(Flow::Straight)
}

fn integer_field(plan: &Plan, ctx: &Ctx, object: u16, slot: u16) -> bool {
    declared_record_slot_kind(
        &plan.structs,
        ctx.struct_types,
        ctx.constants,
        object,
        slot as usize,
    ) == Some(Kind::Int)
}

/// The existing field trailer distinguishes owned cell handles from raw word values.
fn record_field_is_handle(code: &mut Vec<u8>, object: u32, slot: u16) {
    local_get(code, object);
    i32_load(code, 8);
    local_get(code, object);
    i32_load(code, 0);
    i32_const(code, 8);
    code.push(0x6C);
    code.push(0x6A);
    i32_load8_u(code, slot as u32);
}

fn record_cell_word(code: &mut Vec<u8>, object: u32, slot: u16) {
    local_get(code, object);
    i32_load(code, 8);
    i32_load(code, slot as u32 * 8);
    i32_load(code, 8);
    i64_load(code, 0);
}

fn record_integer_load(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    object: u16,
    slot: u16,
    dst: u16,
) -> R<Flow> {
    record_field_is_handle(code, object as u32, slot);
    code.extend([0x04, I64]);
    record_cell_word(code, object as u32, slot);
    clone_top(code, ctx, scratch(plan, OPERAND_A))?;
    code.push(0x05);
    local_get(code, object as u32);
    i32_load(code, 8);
    i64_load(code, slot as u32 * 8);
    tag_top(code, ctx, scratch(plan, OPERAND_A))?;
    code.push(0x0B);
    local_set(code, scratch(plan, RESULT));
    store_owned(
        code,
        plan,
        ctx,
        dst,
        scratch(plan, RESULT),
        FailCode::IntTooWide,
    )?;
    Ok(Flow::Straight)
}

fn record_integer_store(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    object: u16,
    slot: u16,
    value: Option<u16>,
    cow: bool,
) -> R<Flow> {
    if !ctx.heap_counting() {
        return Err(WasmLowerError::Unsupported(
            "exact record cells require typed heap ownership",
        ));
    }
    if cow {
        emit_record_copy(
            code,
            ctx,
            &plan.kinds,
            &plan.structs,
            plan.num_regs,
            object,
            Some(slot as usize),
        )?;
    }
    record_field_is_handle(code, object as u32, slot);
    code.extend([0x04, 0x40]);
    emit_true_release(code, ctx, |c| {
        local_get(c, object as u32);
        i32_load(c, 8);
        i32_load(c, slot as u32 * 8);
    });
    code.push(0x0B);
    if let Some(value) = value {
        let cell = plan.num_regs + 8;
        local_get(code, value as u32);
        if holds(plan, value) {
            clone_top(code, ctx, scratch(plan, OPERAND_A))?;
        } else {
            tag_top(code, ctx, scratch(plan, OPERAND_A))?;
        }
        call(
            code,
            ctx.runtime
                .and_then(|r| r.index(RuntimeFn::ExactCell))
                .expect("exact records carry the counted integer cell allocator"),
        );
        local_set(code, cell);
        local_get(code, object as u32);
        i32_load(code, 8);
        local_get(code, cell);
        i32_store(code, slot as u32 * 8);
        emit_true_retain_of(code, ctx, |c| local_get(c, cell));
        emit_record_field_is_handle(code, ctx, object, slot as u32, true);
    } else {
        local_get(code, object as u32);
        i32_load(code, 8);
        i64_const(code, 0);
        i64_store(code, slot as u32 * 8);
        emit_record_field_kind(code, ctx, object, slot as u32, None);
    }
    Ok(Flow::Straight)
}

/// Render a record's Int field using its actual stored representation.
pub(super) fn record_integer_text(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    num_regs: u32,
    object: u32,
    slot: u16,
    out: u32,
) -> R<()> {
    record_field_is_handle(code, object, slot);
    code.extend([0x04, 0x40]);
    record_cell_word(code, object, slot);
    call(code, import(ctx, HostFn::ExactText)?);
    local_set(code, out);
    code.push(0x05);
    local_get(code, object);
    i32_load(code, 8);
    i64_load(code, slot as u32 * 8);
    local_set(code, num_regs + 15);
    emit_stringify(code, ctx, num_regs, num_regs + 15, Some(Kind::Int), out)?;
    code.push(0x0B);
    Ok(())
}

/// An explicit record copy shares immutable integer cells with its own counted hold.
pub(super) fn retain_record_integer(code: &mut Vec<u8>, ctx: &Ctx, object: u32, slot: u16) {
    record_field_is_handle(code, object, slot);
    code.extend([0x04, 0x40]);
    emit_true_retain_of(code, ctx, |c| {
        local_get(c, object);
        i32_load(c, 8);
        i32_load(c, slot as u32 * 8);
    });
    code.push(0x0B);
}

pub(super) use list::{
    promote as promote_sequence, stringify as sequence_text, take_tagged as take_sequence_tagged,
};

pub(super) fn clone_sequence_word(code: &mut Vec<u8>, ctx: &Ctx, nr: u32) -> R<()> {
    clone_top(code, ctx, nr + 15)
}

pub(super) use list::push as push_sequence_word;

pub(super) use list::narrow_slot as narrow_sequence_slot;

pub(super) use map::integer_text as map_integer_text;
