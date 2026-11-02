//! Which registers of VM bytecode hold a value of the exact extension — the rules every backend that lowers the
//! bytecode onto the linked value runtime reads (the C walker, linked WebAssembly, RISC-V).
//!
//! A value of the extension (an infinitesimal, a hyperreal, an algebraic number, an ordinal, a residue-dependent or
//! provisional answer, and every list of them) is the runtime's own value, a handle into `logicaffeine_data`'s value
//! model. What makes a register hold one is written here once: an op writes one when it is a builtin of the
//! extension, or computes in the value's own kind over an operand that is one ([`exact_write`]); and a value an op
//! hands the runtime must already be one ([`demands`]). Each backend answers which registers hold one, which hold
//! text and which a map, and keeps the rest of its typing. A backend that types each register per program point
//! holds a variable in one kind across every point it reaches, so it decides by [`webs`]: the definitions reaching
//! one read are one variable, exact when any of them is.

use std::collections::BTreeSet;

use crate::semantics::builtins::BuiltinId;
use crate::vm::variables::{Flow, Site, Variables};
use crate::vm::{BoundaryType, CompiledProgram, Op};

/// The builtins that round, measure or combine a number in its own kind, which answer a value of the extension for
/// an argument that is one, by the runtime operation of that name.
pub(crate) fn lifts(builtin: BuiltinId) -> Option<&'static str> {
    Some(match builtin {
        BuiltinId::Floor => "floor",
        BuiltinId::Ceil => "ceil",
        BuiltinId::Round => "round",
        BuiltinId::Abs => "abs",
        BuiltinId::Sqrt => "sqrt",
        BuiltinId::Min => "min",
        BuiltinId::Max => "max",
        BuiltinId::Pow => "power",
        _ => return None,
    })
}

/// A binary op as the runtime computes it over an operand of the extension: its destination, operands and the
/// runtime operation's name.
pub(crate) fn binary(op: &Op) -> Option<(u16, u16, u16, &'static str)> {
    Some(match *op {
        Op::Add { dst, lhs, rhs } => (dst, lhs, rhs, "add"),
        Op::Sub { dst, lhs, rhs } => (dst, lhs, rhs, "sub"),
        Op::Mul { dst, lhs, rhs } => (dst, lhs, rhs, "mul"),
        Op::Div { dst, lhs, rhs } => (dst, lhs, rhs, "div"),
        Op::ExactDiv { dst, lhs, rhs } => (dst, lhs, rhs, "exact_div"),
        Op::FloorDiv { dst, lhs, rhs } => (dst, lhs, rhs, "floor_div"),
        Op::Mod { dst, lhs, rhs } => (dst, lhs, rhs, "rem"),
        Op::Pow { dst, lhs, rhs } => (dst, lhs, rhs, "pow"),
        Op::Lt { dst, lhs, rhs } => (dst, lhs, rhs, "lt"),
        Op::Gt { dst, lhs, rhs } => (dst, lhs, rhs, "gt"),
        Op::LtEq { dst, lhs, rhs } => (dst, lhs, rhs, "le"),
        Op::GtEq { dst, lhs, rhs } => (dst, lhs, rhs, "ge"),
        Op::Eq { dst, lhs, rhs } => (dst, lhs, rhs, "eq"),
        Op::NotEq { dst, lhs, rhs } => (dst, lhs, rhs, "ne"),
        Op::ApproxEq { dst, lhs, rhs } => (dst, lhs, rhs, "approx_eq"),
        Op::BitAnd { dst, lhs, rhs } => (dst, lhs, rhs, "bit_and"),
        Op::BitOr { dst, lhs, rhs } => (dst, lhs, rhs, "bit_or"),
        Op::BitXor { dst, lhs, rhs } => (dst, lhs, rhs, "bit_xor"),
        Op::Shl { dst, lhs, rhs } => (dst, lhs, rhs, "shl"),
        Op::Shr { dst, lhs, rhs } => (dst, lhs, rhs, "shr"),
        Op::SeqConcat { dst, lhs, rhs } => (dst, lhs, rhs, "seq_concat"),
        _ => return None,
    })
}

/// A division by a constant the op carries, as the operator the VM computes it with over any value: `DivPow2` is
/// `lhs / 2^k`, and `MagicDivU` is `lhs / divisor`, or `lhs % divisor` where it multiplies back. The destination,
/// the dividend, the divisor and the runtime operation's name.
pub(crate) fn by_constant(op: &Op) -> Option<(u16, u16, i64, &'static str)> {
    match *op {
        Op::DivPow2 { dst, lhs, k } => Some((dst, lhs, 1i64 << k, "div")),
        Op::MagicDivU {
            dst,
            lhs,
            mul_back,
            divisor,
            ..
        } => Some((dst, lhs, divisor, if mul_back == 0 { "div" } else { "rem" })),
        _ => None,
    }
}

/// The register `op` writes with a value of the extension, given which registers hold one (`exact`), which hold
/// text (`text`) and which a map or a record (`keyed`): a builtin answering one, a lifting builtin over an
/// argument that is one, an operator over an operand that is one (`+` beside a text stays the text it joins), an
/// item or a run of items of a list that is one, a list holding one, a list one is pushed onto or stored into (a map
/// or a record stores one under a key and keeps its own kind).
pub(crate) fn exact_write(
    op: &Op,
    exact: impl Fn(u16) -> bool,
    text: impl Fn(u16) -> bool,
    keyed: impl Fn(u16) -> bool,
) -> Option<u16> {
    let when = |dst: u16, yes: bool| yes.then_some(dst);
    match *op {
        Op::CallBuiltin {
            dst,
            builtin,
            args_start,
            arg_count,
        } => {
            if builtin.answers_exact() {
                return Some(dst);
            }
            when(
                dst,
                lifts(builtin).is_some() && (0..arg_count).any(|i| exact(args_start + i)),
            )
        }
        Op::Add { dst, lhs, rhs } => when(dst, !text(lhs) && !text(rhs) && (exact(lhs) || exact(rhs))),
        Op::AddAssign { dst, src } => when(dst, !text(dst) && !text(src) && (exact(dst) || exact(src))),
        Op::Not { dst, src } => when(dst, exact(src)),
        Op::Index { dst, collection, .. } | Op::IndexUnchecked { dst, collection, .. } => {
            when(dst, exact(collection))
        }
        Op::NewList { dst, start, count } => when(dst, (0..count).any(|i| exact(start + i))),
        Op::ListPush { list, value } => when(list, exact(list) || exact(value)),
        Op::ListPop { list, dst } => when(dst, exact(list)),
        Op::SliceOp { dst, collection, .. } => when(dst, exact(collection)),
        Op::SetIndex {
            collection, value, ..
        }
        | Op::SetIndexUnchecked {
            collection, value, ..
        } => when(
            collection,
            !keyed(collection) && (exact(collection) || exact(value)),
        ),
        ref other => binary(other)
            .and_then(|(dst, lhs, rhs, _)| when(dst, exact(lhs) || exact(rhs)))
            .or_else(|| by_constant(other).and_then(|(dst, lhs, _, _)| when(dst, exact(lhs)))),
    }
}

/// The registers whose values `op` hands the runtime as values of the extension, when what `op` writes is one
/// (`lands_exact`): every argument of a builtin of the extension, and every item of a list literal that lands as
/// the runtime's list, down to the innermost list written in place.
pub(crate) fn demands(op: &Op, lands_exact: bool) -> Vec<u16> {
    match *op {
        Op::CallBuiltin {
            builtin,
            args_start,
            arg_count,
            ..
        } if builtin.extension().is_some() => (0..arg_count).map(|i| args_start + i).collect(),
        Op::NewList { start, count, .. } if lands_exact => (0..count).map(|i| start + i).collect(),
        _ => Vec::new(),
    }
}

/// What a region's exact webs hold: the definitions (by pc) that write a value of the extension, and the registers
/// that must arrive as one.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Webs {
    pub(crate) defs: BTreeSet<usize>,
    pub(crate) entries: BTreeSet<u16>,
}

/// The exact webs of `region`, whose first `entries` registers are live on entry: its variables ([`Variables`]), an op
/// that changes the register it reads (a push, an item store, `+=`) or copies one continuing that variable, and every
/// web exact that has a definition making a value of the extension (`exact_def(pc, register)`, by the kinds after the
/// op) or an entry arriving as one (`exact_entry`). What an extension builtin reads is one, and a list that lands as
/// one holds its items as values of the extension, so the webs those reads reach are exact too, to the innermost list
/// written in place ([`demands`]).
pub(crate) fn webs(
    code: &[Op],
    region: &impl Flow,
    entries: u16,
    exact_entry: impl Fn(u16) -> bool,
    exact_def: impl Fn(usize, u16) -> bool,
) -> Webs {
    let vars = Variables::of(code, region, entries, |op, w, r| w == r || matches!(op, Op::Move { .. }));
    let pcs = region.pcs();
    let mut exact_roots: BTreeSet<usize> = vars
        .sites()
        .filter(|(d, site)| match *site {
            Site::Entry(reg) => exact_entry(reg),
            Site::Op(pc) => exact_def(pc, vars.register(*d)),
        })
        .map(|(d, _)| vars.variable(d))
        .collect();
    loop {
        let before = exact_roots.len();
        for &pc in &pcs {
            let lands_exact = vars
                .definition(pc)
                .is_some_and(|(_, d)| exact_roots.contains(&vars.variable(d)));
            for r in demands(&code[pc], lands_exact) {
                let reached: Vec<usize> = vars.reaching(pc, r).map(|d| vars.variable(d)).collect();
                exact_roots.extend(reached);
            }
        }
        if exact_roots.len() == before {
            break;
        }
    }
    let mut webs = Webs::default();
    for (d, site) in vars.sites() {
        if exact_roots.contains(&vars.variable(d)) {
            match site {
                Site::Entry(reg) => {
                    webs.entries.insert(reg);
                }
                Site::Op(pc) => {
                    webs.defs.insert(pc);
                }
            }
        }
    }
    webs
}

/// Whether a declared type is a value of the extension: one of its types, or a list of one (one value, the runtime's
/// list).
pub(crate) fn declared(bt: &BoundaryType) -> bool {
    match bt {
        BoundaryType::Builtin(name) => crate::analysis::tower::TOWER_TYPES.contains(&name.as_str()),
        BoundaryType::Seq(elem) => declared(elem),
        _ => false,
    }
}

/// Whether `program` computes with the extension at all: a builtin answering one of its values, or a value declared
/// of one of its types (a parameter, a result, a field, a global).
pub(crate) fn reaches(program: &CompiledProgram) -> bool {
    let declared_at = |t: &Option<BoundaryType>| t.as_ref().is_some_and(declared);
    program
        .code
        .iter()
        .any(|op| matches!(op, Op::CallBuiltin { builtin, .. } if builtin.answers_exact()))
        || program
            .functions
            .iter()
            .any(|f| declared_at(&f.return_type) || f.param_types.iter().any(declared_at))
        || program
            .struct_types
            .iter()
            .any(|s| s.fields.iter().any(|(_, t)| declared(t)))
        || program.global_types.iter().any(declared_at)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/tower/tests.rs"]
mod tests;
