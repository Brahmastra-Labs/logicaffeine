//! Lent values: what the emitter borrows instead of copying, and the proof that the borrow is unobservable.
//!
//! **Enum values.** A value of a program's own enum type — a `Tree`, an `Expr` — is walked far more often than it is
//! kept. A function whose every mention of such a parameter is the scrutinee of an `Inspect`, or an argument another
//! borrowing parameter takes, lowers the parameter to `&T`; inside an arm, a binding held on the same terms is bound
//! as a borrow of the matched field instead of an owned copy of it. Anything else — returning it, storing it, handing
//! it to a parameter that owns it, reading it in an expression — keeps the owned copy, so the analysis only ever
//! removes a clone the program could not observe. The parameter half is a greatest fixpoint over the call graph:
//! every enum parameter of an eligible function starts borrowed, and one whose uses need a slot that is no longer
//! borrowed is dropped, until nothing changes. A recursive walk (`check(left)` inside `check`) therefore borrows, and a
//! chain of helpers borrows exactly as far as every link does.
//!
//! **Sequence iteration** is proven once for every backend, by the shared optimizer
//! ([`crate::optimize::iteration`]): a `Repeat` that may walk its sequence in place reads it through a shared
//! borrow here.
//!
//! Uses are found with [`crate::ast_map`]'s exhaustive walk and writes with the optimizer's exhaustive write walk
//! ([`crate::optimize::WriteSet`]) — every `Stmt` and `Expr` variant named, no wildcard — and matched by NAME, since the
//! parser may give one variable a distinct symbol per occurrence. A form nobody taught them about is a use or a write.

use std::collections::{HashMap, HashSet};

use crate::analysis::registry::{TypeDef, TypeRegistry};
use crate::arena::Arena;
use crate::ast::stmt::{Expr, Stmt, TypeExpr};
use crate::ast_map::{self, Arenas, AstMap};
use crate::intern::{Interner, Symbol};

/// Whether `ty` names one of the program's own enum types (generic or not).
pub(crate) fn is_user_enum(ty: &TypeExpr, registry: &TypeRegistry) -> bool {
    let name = match ty {
        TypeExpr::Named(s) | TypeExpr::Primitive(s) => *s,
        TypeExpr::Generic { base, .. } => *base,
        _ => return false,
    };
    matches!(registry.get(name), Some(TypeDef::Enum { .. }))
}

/// Whether `block` never writes the name `sym` spells and every mention of it is a borrowed one: the scrutinee of an
/// `Inspect`, or an argument at a position `borrowed_at(callee, index)` lends. A write — `Set`, a rebinding `Let`, an
/// in-place change, foreign code — is found by the optimizer's one write walk ([`crate::optimize::WriteSet`]).
/// Occurrences are matched by NAME: the parser may give one variable a distinct symbol per occurrence, and a mention
/// the probe did not recognise would be a use it never judged.
pub(crate) fn only_borrowed_uses(
    block: &[Stmt],
    sym: Symbol,
    interner: &Interner,
    borrowed_at: &dyn Fn(Symbol, usize) -> bool,
) -> bool {
    let name = interner.resolve(sym);
    let writes = crate::optimize::WriteSet::of_block(block);
    if writes.is_opaque() || writes.symbols().any(|w| interner.resolve(w) == name) {
        return false;
    }
    let exprs: Arena<Expr> = Arena::new();
    let stmts: Arena<Stmt> = Arena::new();
    let _facts = crate::optimize::facts::scope();
    let types: Arena<TypeExpr> = Arena::new();
    let mut probe = UseProbe {
        arenas: Arenas {
            exprs: &exprs,
            stmts: &stmts,
            types: &types,
        },
        is_name: &|s: Symbol| interner.resolve(s) == name,
        borrowed_at,
        lent: HashSet::new(),
        owned_use: false,
    };
    for stmt in block {
        probe.map_stmt(stmt);
        if probe.owned_use {
            return false;
        }
    }
    true
}

/// Walks a block and records whether `sym` is mentioned anywhere but a lent position. Lent occurrences are marked by
/// node identity before the walk reaches them.
struct UseProbe<'a, 'f> {
    arenas: Arenas<'a>,
    is_name: &'f dyn Fn(Symbol) -> bool,
    borrowed_at: &'f dyn Fn(Symbol, usize) -> bool,
    lent: HashSet<*const Expr<'a>>,
    owned_use: bool,
}

impl<'a, 'f> UseProbe<'a, 'f> {
    fn lend_arguments(&mut self, function: Symbol, args: &[&'a Expr<'a>]) {
        for (i, arg) in args.iter().enumerate() {
            if matches!(arg, Expr::Identifier(s) if (self.is_name)(*s))
                && (self.borrowed_at)(function, i)
            {
                self.lent.insert(*arg as *const Expr<'a>);
            }
        }
    }
}

impl<'a, 'f> AstMap<'a> for UseProbe<'a, 'f> {
    fn arenas(&self) -> Arenas<'a> {
        self.arenas
    }

    fn map_expr(&mut self, e: &'a Expr<'a>) -> &'a Expr<'a> {
        match e {
            Expr::Identifier(s) if (self.is_name)(*s) => {
                if !self.lent.contains(&(e as *const Expr<'a>)) {
                    self.owned_use = true;
                }
                e
            }
            Expr::Call { function, args } => {
                self.lend_arguments(*function, args);
                ast_map::walk_expr(self, e)
            }
            _ => ast_map::walk_expr(self, e),
        }
    }

    fn map_stmt(&mut self, s: &Stmt<'a>) -> Stmt<'a> {
        match s {
            Stmt::Inspect { target, .. } if matches!(target, Expr::Identifier(t) if (self.is_name)(*t)) =>
            {
                self.lent.insert(*target as *const Expr<'a>);
            }
            Stmt::Call { function, args } => self.lend_arguments(*function, args),
            _ => {}
        }
        ast_map::walk_stmt(self, s)
    }
}

/// The eligible functions' enum parameters that are only ever lent: `function → parameter indices`. `eligible` says
/// which function definitions may change their parameters' passing at all (the same gates the sequence borrow uses).
pub(crate) fn enum_borrow_params(
    stmts: &[Stmt],
    registry: &TypeRegistry,
    interner: &Interner,
    eligible: &dyn Fn(&Stmt) -> bool,
) -> HashMap<Symbol, HashSet<usize>> {
    let mut borrowed: HashMap<Symbol, HashSet<usize>> = HashMap::new();
    let mut bodies: Vec<(Symbol, Vec<Symbol>, &[Stmt])> = Vec::new();
    for stmt in stmts {
        if let Stmt::FunctionDef {
            name, params, body, ..
        } = stmt
        {
            let mut syms = Vec::with_capacity(params.len());
            let mut slots = HashSet::new();
            for (i, (sym, ty)) in params.iter().enumerate() {
                syms.push(*sym);
                if eligible(stmt) && is_user_enum(ty, registry) {
                    slots.insert(i);
                }
            }
            if !slots.is_empty() {
                borrowed.insert(*name, slots);
            }
            bodies.push((*name, syms, body));
        }
    }
    loop {
        let mut dropped: Vec<(Symbol, usize)> = Vec::new();
        for (name, params, body) in &bodies {
            let Some(slots) = borrowed.get(name) else {
                continue;
            };
            for &i in slots {
                let lent_at =
                    |f: Symbol, j: usize| borrowed.get(&f).is_some_and(|s| s.contains(&j));
                if !only_borrowed_uses(body, params[i], interner, &lent_at) {
                    dropped.push((*name, i));
                }
            }
        }
        if dropped.is_empty() {
            break;
        }
        for (name, i) in dropped {
            if let Some(slots) = borrowed.get_mut(&name) {
                slots.remove(&i);
                if slots.is_empty() {
                    borrowed.remove(&name);
                }
            }
        }
    }
    borrowed
}
