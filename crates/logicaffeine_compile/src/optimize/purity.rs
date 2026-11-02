//! Which of a program's functions are pure: a call does nothing but compute its result from its arguments.
//!
//! Memoization answers a repeated call from a table, compile-time evaluation runs a call once while compiling, and the
//! symmetry rewrite runs half a search and doubles its count. Each is sound only for a call whose whole effect is its
//! answer, and whose answer depends on nothing but its arguments. This is the one definition every pass asks.
//!
//! The rules admit by name, never by default: a body is pure when every statement and expression in it is one of the
//! forms below, and anything else makes it impure.
//! - It changes only its own names: `Let`, and `Set`, `Push`, `Pop` or an item store to a parameter or a name the body
//!   binds. A `mutable` parameter is the caller's variable, so a function with one is never pure.
//! - It reads its own names, and program variables nothing in the program ever changes after binding them (a constant
//!   of Main's); a variable some function or Main changes could give the same call two answers.
//! - It calls pure functions of the program and pure builtins ([`BuiltinEffect::Pure`]) only; a name the program
//!   defines is never taken for a builtin.
//! - It branches, loops, returns and breaks freely.
//!
//! The set is the greatest fixpoint over the call graph: every candidate starts pure and one whose body breaks a rule
//! under the current set is dropped, until nothing changes, so a recursion is pure when its body is. Names are compared
//! by symbol, which can only make a name the parser spelled twice look like someone else's: impure, never the reverse.

use std::collections::HashSet;

use crate::ast::stmt::{Block, Expr, MatchArm, Pattern, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::semantics::builtins::{builtin_from_name, BuiltinEffect};

use super::writes::WriteSet;

/// The pure functions of one program.
pub(crate) struct Purity {
    pure: HashSet<Symbol>,
}

impl Purity {
    /// The pure functions of `stmts` (see the module doc).
    pub(crate) fn of_program(stmts: &[Stmt], interner: &Interner) -> Self {
        let defined: HashSet<Symbol> = stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef { name, .. } => Some(*name),
                _ => None,
            })
            .collect();
        // Every name the program changes after binding it, in Main or in any function.
        let mut changed: HashSet<Symbol> = HashSet::new();
        let main: Vec<Stmt> = stmts
            .iter()
            .filter(|s| !matches!(s, Stmt::FunctionDef { .. }))
            .cloned()
            .collect();
        changed.extend(WriteSet::of_block(&main).reassigned());
        for s in stmts {
            if let Stmt::FunctionDef { body, .. } = s {
                changed.extend(WriteSet::of_block(body).reassigned());
            }
        }
        let candidates: Vec<(Symbol, HashSet<Symbol>, Block)> = stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef {
                    name,
                    params,
                    body,
                    is_native: false,
                    ..
                } if !params
                    .iter()
                    .any(|(_, t)| matches!(t, TypeExpr::Mutable { .. })) =>
                {
                    let mut own: HashSet<Symbol> = params.iter().map(|(p, _)| *p).collect();
                    own.extend(WriteSet::of_block(body).declared().iter().copied());
                    Some((*name, own, *body))
                }
                _ => None,
            })
            .collect();
        let mut pure: HashSet<Symbol> = candidates.iter().map(|(n, _, _)| *n).collect();
        loop {
            let before = pure.len();
            for (name, own, body) in &candidates {
                if pure.contains(name) {
                    let rules = Rules {
                        own,
                        changed: &changed,
                        pure: &pure,
                        defined: &defined,
                        interner,
                    };
                    if !rules.block(body) {
                        pure.remove(name);
                    }
                }
            }
            if pure.len() == before {
                return Purity { pure };
            }
        }
    }

    /// Whether a call of `function` does nothing but compute its result from its arguments.
    pub(crate) fn is_pure(&self, function: Symbol) -> bool {
        self.pure.contains(&function)
    }

    /// The pure functions, as a set.
    pub(crate) fn into_set(self) -> HashSet<Symbol> {
        self.pure
    }
}

/// The admission rules for one candidate's body, under the current set of pure functions.
struct Rules<'r> {
    own: &'r HashSet<Symbol>,
    changed: &'r HashSet<Symbol>,
    pure: &'r HashSet<Symbol>,
    defined: &'r HashSet<Symbol>,
    interner: &'r Interner,
}

impl Rules<'_> {
    fn block(&self, body: &[Stmt]) -> bool {
        body.iter().all(|s| self.stmt(s))
    }

    fn stmt(&self, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Let { value, .. } => self.expr(value),
            Stmt::Set { target, value } => self.own.contains(target) && self.expr(value),
            Stmt::Return { value } => value.map_or(true, |v| self.expr(v)),
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                self.expr(cond)
                    && self.block(then_block)
                    && else_block.map_or(true, |b| self.block(b))
            }
            Stmt::While { cond, body, .. } => self.expr(cond) && self.block(body),
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => {
                matches!(pattern, Pattern::Identifier(_) | Pattern::Tuple(_))
                    && self.expr(iterable)
                    && self.block(body)
            }
            Stmt::Push { value, collection } => self.own_place(collection) && self.expr(value),
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => self.own_place(collection) && self.expr(index) && self.expr(value),
            Stmt::Pop { collection, .. } => self.own_place(collection),
            Stmt::Break => true,
            Stmt::Call { function, args } => {
                self.callee(*function) && args.iter().all(|a| self.expr(a))
            }
            Stmt::Inspect { target, arms, .. } => {
                self.expr(target) && arms.iter().all(|arm| self.arm(arm))
            }
            _ => false,
        }
    }

    fn arm(&self, arm: &MatchArm) -> bool {
        self.block(arm.body)
    }

    /// A collection changed in place: a name of the function's own.
    fn own_place(&self, collection: &Expr) -> bool {
        matches!(collection, Expr::Identifier(name) if self.own.contains(name))
    }

    fn expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Literal(_) | Expr::OptionNone => true,
            Expr::Identifier(name) => self.own.contains(name) || !self.changed.contains(name),
            Expr::BinaryOp { left, right, .. } => self.expr(left) && self.expr(right),
            Expr::Not { operand } => self.expr(operand),
            Expr::Call { function, args } => {
                self.callee(*function) && args.iter().all(|a| self.expr(a))
            }
            Expr::Length { collection } => self.expr(collection),
            Expr::FieldAccess { object, .. } => self.expr(object),
            Expr::Index { collection, index } => self.expr(collection) && self.expr(index),
            _ => false,
        }
    }

    /// A pure function of the program, or a pure builtin whose name the program does not define.
    fn callee(&self, function: Symbol) -> bool {
        // Local function values shadow global functions and builtins. Their
        // callback effects are unknown even when the spelling names a pure callee.
        if self.own.contains(&function) {
            return false;
        }
        if self.defined.contains(&function) {
            return self.pure.contains(&function);
        }
        builtin_from_name(self.interner.resolve(function))
            .is_some_and(|b| b.effect() == BuiltinEffect::Pure)
    }
}
