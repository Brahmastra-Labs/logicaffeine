//! The C storage of what a stored `nothing` reaches ([`crate::analysis::absence`]).
//!
//! A binding, parameter or result that can itself be `nothing` is a value-or-nothing (`logos_opt`, [`CType::Opt`]):
//! present, it holds the value it would hold anyway; absent, it is `nothing`, and reading it where a value is needed
//! fails as the VM fails. A list or a map whose items can be `nothing` holds values-or-nothing (`Seq_logos_opt`,
//! `Map_str_logos_opt`, `Map_i64_logos_opt`), at whatever depth of its items they can be. One that can never be
//! `nothing` keeps the type it has without this module. A field is one place for every value of its struct.

use std::rc::Rc;

use super::types::{CContext, CType};
use crate::analysis::absence::{Absence, Depths};
use crate::analysis::types::TypeEnv;
use crate::analysis::TypeRegistry;
use crate::ast::stmt::Stmt;
use crate::intern::{Interner, Symbol};

/// Where a stored `nothing` reaches in the program `stmts`, as this tier reads it.
pub(super) fn of_program(stmts: &[Stmt], registry: &TypeRegistry, interner: &Interner) -> Rc<Absence> {
    let types = TypeEnv::infer_program(stmts, interner, registry);
    Rc::new(crate::analysis::absence::of_program(stmts, &types, interner))
}

/// Whether a value of `ty` holds a value-or-nothing, itself or at any depth.
pub(super) fn holds_nothing(ty: &CType) -> bool {
    match ty {
        CType::Opt(_) => true,
        CType::SeqValue(inner) | CType::SeqSeq(inner) | CType::MapStrValue(inner) | CType::MapI64Value(inner) => {
            holds_nothing(inner)
        }
        CType::Tuple(items) | CType::SeqTuple(items) => items.iter().any(holds_nothing),
        CType::Record(fields) => fields.iter().any(|(_, ty)| holds_nothing(ty)),
        _ => false,
    }
}

/// `ty`, the C type of a place, as the storage of a value that can be `nothing` at `depths`: a value-or-nothing at
/// depth 0, a list or a map of values-or-nothing at depth 1, and so on down its items.
fn holding(ty: CType, depths: Option<&Depths>, registry: &TypeRegistry) -> CType {
    match depths {
        Some(depths) => at_depth(ty, depths, 0, registry),
        None => ty,
    }
}

fn at_depth(ty: CType, depths: &Depths, depth: usize, registry: &TypeRegistry) -> CType {
    if depths.iter().all(|d| *d < depth) {
        return ty;
    }
    let held = match ty {
        CType::Opt(_) | CType::Void | CType::Tower => return ty,
        map if super::types::map_entry_types(&map).is_some() || matches!(map, CType::MapStrValue(_) | CType::MapI64Value(_)) => {
            let (key, value) = match &map {
                CType::MapStrValue(value) => (CType::String, (**value).clone()),
                CType::MapI64Value(value) => (CType::Int64, (**value).clone()),
                other => super::types::map_entry_types(other).expect("a map"),
            };
            super::types::map_ctype(&key, &at_depth(value, depths, depth + 1, registry))
        }
        other => match super::types::seq_element(&other, registry) {
            Some(item) if super::types::is_seq_ctype(&other) => {
                super::types::seq_of(at_depth(item, depths, depth + 1, registry))
            }
            _ => other,
        },
    };
    if depths.contains(&depth) {
        CType::Opt(Box::new(held))
    } else {
        held
    }
}

/// The C type of what the `Let` statement `stmt` binds, `ty` being the type its value gives it.
pub(super) fn binding(stmt: &Stmt, ty: CType, ctx: &CContext) -> CType {
    holding(ty, ctx.absence.binding(stmt), ctx.registry)
}

/// The C type of `function`'s parameter `param`, `ty` being its declared type.
pub(super) fn parameter(function: Symbol, param: Symbol, ty: CType, ctx: &CContext) -> CType {
    holding(ty, ctx.absence.parameter(function, param), ctx.registry)
}

/// The C type of what `function` returns, `ty` being its declared or answered type.
pub(super) fn returned(function: Symbol, ty: CType, ctx: &CContext) -> CType {
    holding(ty, ctx.absence.returned(function), ctx.registry)
}

/// The C type of field `field` of `owner`, `ty` being its declared type.
pub(super) fn field(owner: Symbol, field: Symbol, ty: CType, absence: &Absence, registry: &TypeRegistry) -> CType {
    holding(ty, absence.field(owner, field).as_ref(), registry)
}
