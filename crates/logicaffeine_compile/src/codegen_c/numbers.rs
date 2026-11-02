//! Locals of an AST → C scope whose bindings give them numbers of different forms ([`crate::analysis::numbers`]).
//!
//! A C local has one type for its whole scope, so a name the program binds to an integer and then to a fraction is
//! declared a rational, each integer it is given converted as it is stored, and one bound to an integer and then to a
//! real is declared a real only where nothing the program does can tell its integer phase from the real it widens
//! to. A name no C type holds is refused by name where it is first bound. The types of the scope's other locals are
//! settled as the walk meets them, so each binding is typed under what is known of every name it reads.

use std::collections::{HashMap, HashSet};

use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};

use super::types::{infer_expr_type, resolve_type_expr_with_registry, seq_element, CContext, CType};
use crate::analysis::numbers::{exact_in_a_double, integer_phase_hidden};
use crate::ast::stmt::{Expr, Pattern, Stmt};
use crate::intern::Symbol;

/// The scope's number locals: the C type each widened local is declared with, and the locals no C type holds.
#[derive(Debug, Default, Clone)]
pub(super) struct NumberLocals {
    pub(super) types: HashMap<Symbol, CType>,
    pub(super) refused: HashSet<Symbol>,
}

/// The rounds the scope's local types take to hold still: each round only widens a name's type.
const SETTLE_ROUNDS: usize = 64;

/// The number locals of `body`, typed under `ctx` (its parameters and the program's statics already in `ctx.vars`).
pub(super) fn number_locals(body: &[Stmt], ctx: &mut CContext) -> NumberLocals {
    let given: HashSet<Symbol> = ctx.vars.keys().copied().collect();
    let mut bindings: HashMap<Symbol, Vec<&Expr>> = HashMap::new();
    let mut lets: HashMap<Symbol, usize> = HashMap::new();
    let mut declared: HashMap<Symbol, CType> = HashMap::new();
    let mut other: HashSet<Symbol> = HashSet::new();
    let mut loops: Vec<(Symbol, &Expr)> = Vec::new();
    walk_nodes(body, |node| {
        match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => return Walk::Past,
            Visited::Stmt(Stmt::Let { var, value, ty: None, .. }) => {
                *lets.entry(*var).or_default() += 1;
                bindings.entry(*var).or_default().push(value);
            }
            Visited::Stmt(Stmt::Let { var, ty: Some(ty), .. }) => {
                declared.insert(*var, resolve_type_expr_with_registry(ty, ctx.interner, Some(ctx.registry)));
                other.insert(*var);
            }
            Visited::Stmt(Stmt::Set { target, value }) => bindings.entry(*target).or_default().push(value),
            Visited::Stmt(Stmt::Repeat { pattern, iterable, .. }) => match pattern {
                Pattern::Identifier(x) => {
                    loops.push((*x, iterable));
                    other.insert(*x);
                }
                Pattern::Tuple(xs) => other.extend(xs.iter().copied()),
            },
            Visited::Stmt(Stmt::Pop { into: Some(v), .. }) => {
                other.insert(*v);
            }
            Visited::Stmt(Stmt::Inspect { arms, .. }) => {
                for arm in arms.iter() {
                    other.extend(arm.bindings.iter().map(|(_, b)| *b));
                }
            }
            _ => {}
        }
        Walk::Into
    });
    // One name is one C local where the scope binds it with one `Let` and changes it with `Set`s; a name bound
    // again by a `Let` in another block is another local there, typed by its own value. Where the values every
    // such local is given agree on a type, a read of the name has that type, whichever of them it reads.
    let shadowed: HashMap<Symbol, Vec<&Expr>> = bindings
        .iter()
        .filter(|(var, _)| lets.get(var).is_some_and(|n| *n > 1) && !other.contains(var) && !given.contains(var))
        .map(|(var, values)| (*var, values.clone()))
        .collect();
    bindings.retain(|var, _| lets.get(var) == Some(&1) && !other.contains(var) && !given.contains(var));

    // Each round types every binding under the types the last round settled, so a name read before its own binding
    // is typed (a name not yet typed reads as an integer) gives way to what that binding settles.
    let saved = ctx.vars.clone();
    let mut base = saved.clone();
    base.extend(declared);
    let mut typed: HashMap<Symbol, CType> = HashMap::new();
    let mut joined: HashMap<Symbol, CType> = HashMap::new();
    let mut read: HashMap<Symbol, CType> = HashMap::new();
    for _ in 0..SETTLE_ROUNDS {
        ctx.vars = base.clone();
        ctx.vars.extend(read.iter().map(|(k, v)| (*k, v.clone())));
        ctx.vars.extend(typed.iter().map(|(k, v)| (*k, v.clone())));
        let mut next_typed: HashMap<Symbol, CType> = HashMap::new();
        let mut next_joined: HashMap<Symbol, CType> = HashMap::new();
        let mut next_read: HashMap<Symbol, CType> = HashMap::new();
        for (var, values) in &shadowed {
            let types: Vec<CType> = values.iter().map(|v| infer_expr_type(v, ctx)).collect();
            if types.iter().all(|t| *t == types[0]) {
                next_read.insert(*var, types[0].clone());
            }
        }
        for (x, iterable) in &loops {
            let item = match iterable {
                Expr::Range { .. } => Some(CType::Int64),
                other => seq_element(&infer_expr_type(other, ctx), ctx.registry),
            };
            if let Some(item) = item {
                next_typed.insert(*x, item);
            }
        }
        for (var, values) in &bindings {
            let types: Vec<CType> = values.iter().map(|v| infer_expr_type(v, ctx)).collect();
            let integer = |t: &CType| matches!(t, CType::Int64 | CType::BigInt);
            let to = if types.iter().all(|t| *t == types[0]) {
                Some(types[0].clone())
            } else if types.contains(&CType::Rational) && types.iter().all(|t| integer(t) || *t == CType::Rational) {
                Some(CType::Rational)
            } else if types.iter().all(|t| matches!(t, CType::Int64 | CType::Float64)) {
                Some(CType::Float64)
            } else if types.iter().any(|t| *t == CType::Float64)
                && types.iter().all(|t| integer(t) || matches!(t, CType::Rational | CType::Float64))
            {
                None
            } else {
                continue;
            };
            match to {
                Some(to) => {
                    if types.iter().any(|t| *t != to) {
                        next_joined.insert(*var, to.clone());
                    }
                    next_typed.insert(*var, to);
                }
                None => {
                    next_joined.insert(*var, CType::Void);
                }
            }
        }
        let settled = next_typed == typed && next_joined == joined && next_read == read;
        typed = next_typed;
        joined = next_joined;
        read = next_read;
        if settled {
            break;
        }
    }
    ctx.vars = base;
    ctx.vars.extend(read);
    ctx.vars.extend(typed);

    let show = ctx.interner.lookup("show");
    let mut locals = NumberLocals::default();
    for (var, to) in &joined {
        let values = &bindings[var];
        let hidden = *to == CType::Float64
            && values
                .iter()
                .all(|v| infer_expr_type(v, ctx) == CType::Float64 || exact_in_a_double(v))
            && integer_phase_hidden(body, *var, show, |e| infer_expr_type(e, ctx) == CType::Float64);
        match to {
            CType::Rational => {
                locals.types.insert(*var, CType::Rational);
            }
            CType::Float64 if hidden => {
                locals.types.insert(*var, CType::Float64);
            }
            _ => {
                locals.refused.insert(*var);
            }
        }
    }
    ctx.vars = saved;
    locals
}
