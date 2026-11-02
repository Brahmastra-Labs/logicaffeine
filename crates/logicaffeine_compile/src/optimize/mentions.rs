//! Every name a statement touches.
//!
//! An analysis that proves a binding is left alone (read only in the ways it permits, never aliased, never passed
//! on) has to see every place the binding can appear, including the forms it has no case of its own for. The answer
//! comes from the optimizer's one exhaustive effect walk ([`super::writes`]): every name a statement reads, declares,
//! assigns or mutates, the two a `Swap` exchanges, and whatever a closure defined in it captures. Foreign code
//! touches every name. A nested function definition is its own scope and is not entered.
//!
//! Names are compared as NAMES: the parser may mint a distinct symbol per occurrence of one variable, so a symbol
//! comparison can miss a mention, and a missed mention is an unsound proof.

use std::collections::{HashMap, HashSet};

use crate::ast::stmt::{ClosureBody, Expr, Stmt};
use crate::intern::{Interner, Symbol};

use super::writes::{walk_block, walk_expr, walk_stmt, Bind, EffectSink};

/// The names one statement, block or expression touches, and whether it ran foreign code.
#[derive(Debug, Default)]
pub(crate) struct Mentions {
    names: HashSet<Symbol>,
    opaque: bool,
}

impl EffectSink for Mentions {
    fn bind(&mut self, sym: Symbol, _how: Bind) {
        self.names.insert(sym);
    }
    fn mutate(&mut self, root: Symbol) {
        self.names.insert(root);
    }
    fn call(&mut self) {}
    fn opaque(&mut self) {
        self.opaque = true;
    }
    fn read(&mut self, sym: Symbol) {
        self.names.insert(sym);
    }
    /// A closure captures what its body names; its parameters shadow, and are counted, so the answer only errs
    /// toward "touched".
    fn closure(&mut self, params: &[Symbol], body: &ClosureBody) {
        self.names.extend(params.iter().copied());
        match body {
            ClosureBody::Expression(e) => walk_expr(e, self),
            ClosureBody::Block(b) => walk_block(b, self),
        }
    }
}

impl Mentions {
    pub(crate) fn of_stmt(stmt: &Stmt) -> Self {
        let mut m = Self::default();
        walk_stmt(stmt, &mut m);
        m
    }

    pub(crate) fn of_block(block: &[Stmt]) -> Self {
        let mut m = Self::default();
        walk_block(block, &mut m);
        m
    }

    pub(crate) fn of_expr(expr: &Expr) -> Self {
        let mut m = Self::default();
        walk_expr(expr, &mut m);
        m
    }

    /// Whether a binding spelled `name` is touched. Foreign code touches every name.
    pub(crate) fn touches(&self, name: &str, interner: &Interner) -> bool {
        self.opaque || self.names.iter().any(|s| interner.resolve(*s) == name)
    }

    /// Whether foreign code ran: it may touch any name.
    pub(crate) fn is_opaque(&self) -> bool {
        self.opaque
    }

    /// The symbols touched (by the symbol each occurrence carries).
    pub(crate) fn symbols(&self) -> impl Iterator<Item = Symbol> + '_ {
        self.names.iter().copied()
    }
}

/// Whether `stmt` touches the binding `sym`, compared by symbol: [`Mentions::of_stmt`] without collecting the rest.
pub(crate) fn stmt_touches(stmt: &Stmt, sym: Symbol) -> bool {
    let mut f = Finder { sym, found: false };
    walk_stmt(stmt, &mut f);
    f.found
}

/// Whether `expr` touches the binding `sym`, compared by symbol.
pub(crate) fn expr_touches(expr: &Expr, sym: Symbol) -> bool {
    let mut f = Finder { sym, found: false };
    walk_expr(expr, &mut f);
    f.found
}

/// How many times `stmt` USES each binding — reads it or changes it in place, a closure's capture included — by
/// symbol, and whether it ran foreign code (which may use any binding any number of times). A declaration or a
/// reassignment is not a use: it gives the name a new value rather than consuming the old one.
pub(crate) fn use_counts(stmt: &Stmt) -> (HashMap<Symbol, usize>, bool) {
    let mut c = Counter::default();
    walk_stmt(stmt, &mut c);
    (c.counts, c.opaque)
}

#[derive(Default)]
struct Counter {
    counts: HashMap<Symbol, usize>,
    opaque: bool,
}

impl EffectSink for Counter {
    fn bind(&mut self, _sym: Symbol, _how: Bind) {}
    fn mutate(&mut self, root: Symbol) {
        *self.counts.entry(root).or_insert(0) += 1;
    }
    fn call(&mut self) {}
    fn opaque(&mut self) {
        self.opaque = true;
    }
    fn read(&mut self, sym: Symbol) {
        *self.counts.entry(sym).or_insert(0) += 1;
    }
    fn closure(&mut self, _params: &[Symbol], body: &ClosureBody) {
        match body {
            ClosureBody::Expression(e) => walk_expr(e, self),
            ClosureBody::Block(b) => walk_block(b, self),
        }
    }
}

/// Looks for one symbol; foreign code finds it.
struct Finder {
    sym: Symbol,
    found: bool,
}

impl EffectSink for Finder {
    fn bind(&mut self, sym: Symbol, _how: Bind) {
        self.found |= sym == self.sym;
    }
    fn mutate(&mut self, root: Symbol) {
        self.found |= root == self.sym;
    }
    fn call(&mut self) {}
    fn opaque(&mut self) {
        self.found = true;
    }
    fn read(&mut self, sym: Symbol) {
        self.found |= sym == self.sym;
    }
    fn closure(&mut self, params: &[Symbol], body: &ClosureBody) {
        self.found |= params.contains(&self.sym);
        match body {
            ClosureBody::Expression(e) => walk_expr(e, self),
            ClosureBody::Block(b) => walk_block(b, self),
        }
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/mentions/tests.rs"]
mod tests;
