//! Loop-invariant libdivide detection (O9).
//!
//! Finds scalar integer divisors that are *loop-invariant* and *positive*, whose
//! `% n` / `/ n` lives in a hot loop, so codegen can replace the hardware
//! `div`/`idiv` with a precomputed [`logicaffeine_data::LogosDivU64`] magic
//! multiply. gcc and rustc both leave a runtime-invariant divisor as a real
//! division, so this is a strict win over the C baseline on division-hot loops.
//!
//! A divisor `n` qualifies only when EVERY one of its `% n` / `/ n` uses is
//! provably sound, because the codegen rewrite is keyed by the divisor variable
//! and fires at every such site:
//!
//!  * `n` is immutable (never reassigned) — the magic stays valid for the
//!    binding's whole life, so the helper is computed once at `n`'s definition.
//!  * `n >= 1` at the use: the use sits inside a `while c < n` loop whose counter
//!    `c` the oracle proves `>= 0`, so the body running implies `n > c >= 0`.
//!  * the dividend is oracle-proven `>= 0`, so reinterpreting the operands as
//!    `u64` is value-preserving (truncated and unsigned `%`/`/` agree there).
//!
//! Any use that fails these is unprovable, which disqualifies the divisor
//! outright rather than miscompiling a single site.

use std::collections::{HashMap, HashSet};

use crate::analysis::types::RustNames;
use crate::ast::stmt::{BinaryOpKind, Expr, Stmt};
use crate::intern::{Interner, Symbol};
use crate::optimize::OracleFacts;

use super::detection::collect_mutable_vars;

/// The Rust variable holding the precomputed `LogosDivU64` for divisor `n`.
pub(super) fn helper_name(divisor_ident: &str) -> String {
    format!("__lcdiv_{}", divisor_ident)
}

/// Map each qualifying divisor symbol to its helper variable name. Empty without
/// an oracle (the soundness proofs need it).
pub(super) fn detect_fast_div(
    stmts: &[Stmt],
    oracle: Option<&OracleFacts>,
    interner: &Interner,
) -> HashMap<Symbol, String> {
    let oracle = match oracle {
        Some(o) => o,
        None => return HashMap::new(),
    };
    let mutated = collect_mutable_vars(stmts);
    let mut w = Walk {
        oracle,
        mutated: &mutated,
        sound: HashSet::new(),
        unsound: HashSet::new(),
    };
    w.block(stmts, &Scope::default());
    let names = RustNames::new(interner);
    w.sound
        .difference(&w.unsound)
        .map(|&n| (n, helper_name(&names.ident(n))))
        .collect()
}

/// The loop facts known at a program point: variables proven `>= 1` (loop upper
/// bounds) and `>= 0` (loop counters), both established from `while c < n` guards
/// where the oracle proves `c >= 0`.
#[derive(Clone, Default)]
struct Scope {
    /// Variables proven `>= 1` here (an enclosing `while c < v` upper bound).
    pos: HashSet<Symbol>,
    /// Variables proven `>= 0` here (the matching loop counter `c`).
    nonneg: HashSet<Symbol>,
}

struct Walk<'a> {
    oracle: &'a OracleFacts,
    mutated: &'a HashSet<Symbol>,
    /// Divisors with at least one provably sound, in-loop use.
    sound: HashSet<Symbol>,
    /// Divisors with a use we cannot prove safe — disqualifies them entirely.
    unsound: HashSet<Symbol>,
}

impl<'a> Walk<'a> {
    fn block(&mut self, stmts: &[Stmt], sc: &Scope) {
        for s in stmts {
            self.stmt(s, sc);
        }
    }

    fn stmt(&mut self, s: &Stmt, sc: &Scope) {
        match s {
            Stmt::While { cond, body, .. } => {
                self.expr(cond, sc);
                let mut inner = sc.clone();
                // `c < n` with the oracle proving `c >= 0` makes `n >= 1` hold
                // whenever the body runs (`n > c >= 0`), and `c` itself `>= 0`.
                // `<=` would only give `n >= 0`, so require strict `<`.
                if let Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } = cond {
                    if let (Expr::Identifier(c), Expr::Identifier(n)) = (left, right) {
                        if self.oracle.expr_int_range(left).is_some_and(|(lo, _)| lo >= 0) {
                            inner.pos.insert(*n);
                            inner.nonneg.insert(*c);
                        }
                    }
                }
                self.block(body, &inner);
            }
            Stmt::Repeat { iterable, body, .. } => {
                self.expr(iterable, sc);
                self.block(body, sc);
            }
            Stmt::If { cond, then_block, else_block, .. } => {
                self.expr(cond, sc);
                self.block(then_block, sc);
                if let Some(e) = else_block {
                    self.block(e, sc);
                }
            }
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => self.expr(value, sc),
            Stmt::Return { value: Some(e) } => self.expr(e, sc),
            Stmt::Show { object, .. } | Stmt::Give { object, .. } => self.expr(object, sc),
            Stmt::Push { value, collection, .. } => {
                self.expr(value, sc);
                self.expr(collection, sc);
            }
            Stmt::Add { value, .. } | Stmt::Remove { value, .. } => self.expr(value, sc),
            Stmt::SetIndex { collection, index, value } => {
                self.expr(collection, sc);
                self.expr(index, sc);
                self.expr(value, sc);
            }
            Stmt::SetField { value, .. } => self.expr(value, sc),
            Stmt::Call { args, .. } => {
                for a in args {
                    self.expr(a, sc);
                }
            }
            Stmt::RuntimeAssert { condition, .. } => self.expr(condition, sc),
            _ => {}
        }
    }

    fn expr(&mut self, e: &Expr, sc: &Scope) {
        match e {
            Expr::BinaryOp { op: BinaryOpKind::Modulo | BinaryOpKind::Divide, left, right } => {
                if let Expr::Identifier(n) = right {
                    self.consider(*n, left, sc);
                }
                self.expr(left, sc);
                self.expr(right, sc);
            }
            Expr::BinaryOp { left, right, .. } => {
                self.expr(left, sc);
                self.expr(right, sc);
            }
            Expr::Not { operand } => self.expr(operand, sc),
            Expr::Index { collection, index } => {
                self.expr(collection, sc);
                self.expr(index, sc);
            }
            Expr::Length { collection } => self.expr(collection, sc),
            Expr::Call { args, .. } => {
                for a in args {
                    self.expr(a, sc);
                }
            }
            Expr::Contains { collection, value } => {
                self.expr(collection, sc);
                self.expr(value, sc);
            }
            _ => {}
        }
    }

    /// Record a `% n` / `/ n` use: sound if `n` is immutable, `n >= 1` here, and
    /// the dividend is `>= 0`; otherwise the divisor is disqualified.
    fn consider(&mut self, n: Symbol, dividend: &Expr, sc: &Scope) {
        if self.mutated.contains(&n) {
            self.unsound.insert(n);
            return;
        }
        let n_ge_1 = sc.pos.contains(&n);
        let dividend_ge_0 = self.expr_nonneg(dividend, sc);
        if n_ge_1 && dividend_ge_0 {
            self.sound.insert(n);
        } else {
            self.unsound.insert(n);
        }
    }

    /// Is `e` provably `>= 0`? Syntactic over the non-negative-closed operations
    /// (non-negative loop counters and literals, `+`, `*`, `%`, `len`), with the
    /// oracle's interval as the fallback for opaque leaves like `arr[i]`.
    fn expr_nonneg(&self, e: &Expr, sc: &Scope) -> bool {
        match e {
            Expr::Literal(crate::ast::stmt::Literal::Number(k)) => *k >= 0,
            Expr::Identifier(v) => sc.nonneg.contains(v),
            Expr::Length { .. } => true,
            Expr::BinaryOp { op: BinaryOpKind::Add | BinaryOpKind::Multiply, left, right } => {
                self.expr_nonneg(left, sc) && self.expr_nonneg(right, sc)
            }
            // `a % m` and `a / m` with a non-negative dividend stay non-negative.
            Expr::BinaryOp { op: BinaryOpKind::Modulo | BinaryOpKind::Divide, left, .. } => {
                self.expr_nonneg(left, sc)
            }
            _ => self.oracle.expr_int_range(e).is_some_and(|(lo, _)| lo >= 0),
        }
    }
}
