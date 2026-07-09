//! Type-directed division resolution.
//!
//! The parser emits an ambiguous [`BinaryOpKind::Divide`] for `/`. This pass rewrites
//! a `Divide` to [`BinaryOpKind::ExactDivide`] wherever its result is a `Rational`.
//! `Divide` floors (`7 / 2 → 3`, the integer default that every existing program
//! relies on); the rewritten `ExactDivide` keeps the quotient exact (`7 / 2 → 7/2`).
//! Because the DEFAULT stays floor, existing code is untouched — only `Rational`-typed
//! code ever sees the new op.
//!
//! Resolution is **bidirectional**, a small forward dataflow over a set of
//! known-`Rational` variables:
//!   * TOP-DOWN — a `Rational` context (a `Let x: Rational be …`, a `Rational`-returning
//!     function, an assignment to a `Rational` variable) pushes the expectation down the
//!     arithmetic spine (`+ − × ÷`); every `Divide` it reaches becomes exact.
//!   * BOTTOM-UP — a `Divide` (or `+ − ×`) with a `Rational` OPERAND is itself
//!     `Rational`, so the `Divide` becomes exact and its result-variable joins the
//!     known-`Rational` set, propagating onward.
//! We deliberately do NOT descend into an index, a call argument, or a comparison: those
//! carry their own (`Int`) context, so a `/` there keeps flooring — algorithms are safe.
//!
//! Marking these divisions `ExactDivide` also keeps them away from the integer-only
//! division strength reductions (MagicDivU / DivPow2), which would miscompile a Rational.

use std::collections::HashSet;

use logicaffeine_base::{Arena, Interner, Rational, Symbol};

use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt, TypeExpr};

/// Rewrite `Divide → ExactDivide` in every `Rational` context. Returns the rewritten
/// statements, or `None` if nothing changed (so callers keep the original).
///
/// `fold_constants` is the [`Opt::Comptime`](crate::optimization::Opt::Comptime) toggle:
/// the always-on default collapses a constant Rational chain (`1/3 + 1/6`) to its closed
/// form at compile time; `## No comptime` turns that folding off (the chain still runs,
/// still exact). The `Divide → ExactDivide` resolution itself is semantic, never optional.
pub(crate) fn resolve_divisions<'a>(
    stmts: &'a [Stmt<'a>],
    stmt_arena: &'a Arena<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    interner: &Interner,
    fold_constants: bool,
) -> Option<&'a [Stmt<'a>]> {
    let mut cx = Cx { stmt_arena, expr_arena, interner, fold_constants };
    let mut rats = HashSet::new();
    cx.block(stmts, &mut rats, false)
}

struct Cx<'a, 'i> {
    stmt_arena: &'a Arena<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    interner: &'i Interner,
    /// Whether the closed-form constant fold (the `Opt::Comptime` optimization) is on.
    fold_constants: bool,
}

impl<'a, 'i> Cx<'a, 'i> {
    /// Resolve a statement list in order, threading the known-`Rational` variable set.
    /// `ret_rational` is true inside a `Rational`-returning function (so its `Return`s
    /// are exact).
    fn block(
        &mut self,
        stmts: &'a [Stmt<'a>],
        rats: &mut HashSet<Symbol>,
        ret_rational: bool,
    ) -> Option<&'a [Stmt<'a>]> {
        let mut out: Option<Vec<Stmt<'a>>> = None;
        for (i, s) in stmts.iter().enumerate() {
            let rewritten = self.stmt(s, rats, ret_rational);
            match (&mut out, rewritten) {
                (Some(v), Some(ns)) => v.push(ns),
                (Some(v), None) => v.push(s.clone()),
                (None, Some(ns)) => {
                    let mut v: Vec<Stmt<'a>> = stmts[..i].to_vec();
                    v.push(ns);
                    out = Some(v);
                }
                (None, None) => {}
            }
        }
        out.map(|v| self.stmt_arena.alloc_slice(v))
    }

    fn stmt(
        &mut self,
        s: &Stmt<'a>,
        rats: &mut HashSet<Symbol>,
        ret_rational: bool,
    ) -> Option<Stmt<'a>> {
        match s {
            Stmt::Let { var, ty, value, mutable } => {
                let expected = self.is_rational_ty(*ty);
                let (nv, is_rat) = self.expr(value, rats, expected);
                if expected || is_rat {
                    rats.insert(*var);
                }
                nv.map(|v| Stmt::Let { var: *var, ty: *ty, value: v, mutable: *mutable })
            }
            Stmt::Set { target, value } => {
                let expected = rats.contains(target);
                let (nv, is_rat) = self.expr(value, rats, expected);
                if is_rat {
                    rats.insert(*target);
                }
                nv.map(|v| Stmt::Set { target: *target, value: v })
            }
            Stmt::Return { value: Some(v) } => {
                let (nv, _) = self.expr(v, rats, ret_rational);
                nv.map(|nv| Stmt::Return { value: Some(nv) })
            }
            Stmt::FunctionDef { return_type, body, params, .. } => {
                // A fresh scope: parameters typed `Rational` seed the known set.
                let mut inner: HashSet<Symbol> = params
                    .iter()
                    .filter(|(_, ty)| self.is_rational_ty(Some(ty)))
                    .map(|(p, _)| *p)
                    .collect();
                let returns_rational = self.is_rational_ty(*return_type);
                let nb = self.block(body, &mut inner, returns_rational)?;
                let mut fd = s.clone();
                if let Stmt::FunctionDef { body: b, .. } = &mut fd {
                    *b = nb;
                }
                Some(fd)
            }
            Stmt::If { cond, then_block, else_block } => {
                // Branches share the outer Rational set (a Rational `Let` in one branch
                // is scoped to it, but the conservative shared set never makes a floor
                // division exact, so it is sound).
                let nt = self.block(then_block, rats, ret_rational);
                let ne = else_block.and_then(|e| self.block(e, rats, ret_rational));
                if nt.is_none() && ne.is_none() {
                    return None;
                }
                Some(Stmt::If {
                    cond: *cond,
                    then_block: nt.unwrap_or(then_block),
                    else_block: ne.or(*else_block),
                })
            }
            Stmt::While { cond, body, decreasing } => {
                let nb = self.block(body, rats, ret_rational)?;
                Some(Stmt::While { cond: *cond, body: nb, decreasing: *decreasing })
            }
            Stmt::Repeat { pattern, iterable, body } => {
                let nb = self.block(body, rats, ret_rational)?;
                Some(Stmt::Repeat { pattern: pattern.clone(), iterable: *iterable, body: nb })
            }
            _ => None,
        }
    }

    /// Does `expr` evaluate to a `Rational`? A bare `Int / Int` floors (NOT rational),
    /// but a `/` (or `+ − ×`) with a `Rational` operand is exact (rational). Used to
    /// flow the expectation SIDEWAYS: if one operand of `+ − ×` is rational, the whole
    /// op is rational, so the other operand's divisions must be exact too.
    fn is_rat_expr(&self, expr: &Expr, rats: &HashSet<Symbol>) -> bool {
        match expr {
            Expr::Identifier(sym) => rats.contains(sym),
            Expr::BinaryOp { op: BinaryOpKind::ExactDivide, .. } => true,
            Expr::BinaryOp {
                op:
                    BinaryOpKind::Add
                    | BinaryOpKind::Subtract
                    | BinaryOpKind::Multiply
                    | BinaryOpKind::Divide,
                left,
                right,
            } => self.is_rat_expr(left, rats) || self.is_rat_expr(right, rats),
            _ => false,
        }
    }

    /// Rewrite the arithmetic spine of `expr`. `expected` is true when the context
    /// demands a `Rational`. Returns `(rewritten_or_none, is_rational)`.
    fn expr(
        &mut self,
        expr: &'a Expr<'a>,
        rats: &HashSet<Symbol>,
        expected: bool,
    ) -> (Option<&'a Expr<'a>>, bool) {
        match expr {
            Expr::Identifier(sym) => (None, rats.contains(sym)),
            Expr::BinaryOp { op, left, right } => match op {
                BinaryOpKind::Add
                | BinaryOpKind::Subtract
                | BinaryOpKind::Multiply
                | BinaryOpKind::Divide
                | BinaryOpKind::ExactDivide => {
                    // The operation is in a Rational context if the context above expects
                    // one OR EITHER operand is itself a Rational. That single fact is the
                    // effective expectation for BOTH operands — so a sibling Rational
                    // (`c - 1/6`) makes the other side's divisions exact too.
                    let op_rational = expected
                        || self.is_rat_expr(left, rats)
                        || self.is_rat_expr(right, rats);
                    // LIGHTNING CHAINING (the `Opt::Comptime` / CTFE optimization): a
                    // fully-CONSTANT Rational subtree collapses to its closed form right
                    // here, at compile time — `1/3 + 1/6 + 1/2` becomes the single constant
                    // `1`, and `1/3 + 1/6` becomes one reduced `1/2`, instead of a chain of
                    // runtime divides and adds. `## No comptime` skips it (the chain still
                    // runs, still exact).
                    if op_rational && self.fold_constants {
                        if let Some(rat) = const_rational(expr) {
                            if let Some(closed) = self.closed_form(&rat) {
                                return (Some(closed), true);
                            }
                        }
                    }
                    let (nl, _) = self.expr(left, rats, op_rational);
                    let (nr, _) = self.expr(right, rats, op_rational);
                    let new_op = if matches!(op, BinaryOpKind::Divide) && op_rational {
                        BinaryOpKind::ExactDivide
                    } else {
                        *op
                    };
                    let is_rational = op_rational || matches!(new_op, BinaryOpKind::ExactDivide);
                    let changed = nl.is_some() || nr.is_some() || new_op != *op;
                    let result = changed.then(|| {
                        &*self.expr_arena.alloc(Expr::BinaryOp {
                            op: new_op,
                            left: nl.unwrap_or(left),
                            right: nr.unwrap_or(right),
                        })
                    });
                    (result, is_rational)
                }
                // Modulo / comparison / bitwise / shift do not carry a Rational
                // expectation to a `/` operand, and their result is not a Rational.
                _ => (None, false),
            },
            _ => (None, false),
        }
    }

    fn is_rational_ty(&self, ty: Option<&TypeExpr>) -> bool {
        matches!(
            ty,
            Some(TypeExpr::Primitive(s)) | Some(TypeExpr::Named(s))
                if self.interner.resolve(*s).eq_ignore_ascii_case("Rational")
        )
    }

    /// The closed-form AST for an evaluated constant `Rational`: a bare `Number` when
    /// it is whole, else a single reduced `num / den` (`ExactDivide`). `None` when a
    /// term escapes i64 (then the chain is left to run, still exact, at runtime).
    fn closed_form(&self, rat: &Rational) -> Option<&'a Expr<'a>> {
        if let Some(n) = rat.to_i64() {
            return Some(self.expr_arena.alloc(Expr::Literal(Literal::Number(n))));
        }
        let num = rat.numerator().to_i64()?;
        let den = rat.denominator().to_i64()?;
        let n = self.expr_arena.alloc(Expr::Literal(Literal::Number(num)));
        let d = self.expr_arena.alloc(Expr::Literal(Literal::Number(den)));
        Some(self.expr_arena.alloc(Expr::BinaryOp {
            op: BinaryOpKind::ExactDivide,
            left: n,
            right: d,
        }))
    }
}

/// Evaluate a CONSTANT arithmetic expression to its exact `Rational` — the closed form
/// of a `1/3 + 1/6 + …` chain. `None` if any leaf is non-constant (a variable, a call,
/// …) or a sub-divide is by zero. Both `Divide` and `ExactDivide` evaluate exactly here;
/// the caller only folds where it has already established the result is a `Rational`, so
/// an integer `7 / 2` (which floors) is never reached.
fn const_rational(expr: &Expr) -> Option<Rational> {
    match expr {
        Expr::Literal(Literal::Number(n)) => Some(Rational::from_i64(*n)),
        Expr::BinaryOp { op, left, right } => {
            let l = const_rational(left)?;
            let r = const_rational(right)?;
            match op {
                BinaryOpKind::Add => Some(l.add(&r)),
                BinaryOpKind::Subtract => Some(l.sub(&r)),
                BinaryOpKind::Multiply => Some(l.mul(&r)),
                BinaryOpKind::Divide | BinaryOpKind::ExactDivide => l.div(&r),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num<'a>(a: &'a Arena<Expr<'a>>, v: i64) -> &'a Expr<'a> {
        a.alloc(Expr::Literal(Literal::Number(v)))
    }
    fn bin<'a>(
        a: &'a Arena<Expr<'a>>,
        op: BinaryOpKind,
        l: &'a Expr<'a>,
        r: &'a Expr<'a>,
    ) -> &'a Expr<'a> {
        a.alloc(Expr::BinaryOp { op, left: l, right: r })
    }

    #[test]
    fn const_rational_collapses_a_constant_chain_to_its_exact_value() {
        let a: Arena<Expr> = Arena::new();
        // 1/3 + 1/6  =  1/2.
        let e = bin(
            &a,
            BinaryOpKind::Add,
            bin(&a, BinaryOpKind::Divide, num(&a, 1), num(&a, 3)),
            bin(&a, BinaryOpKind::Divide, num(&a, 1), num(&a, 6)),
        );
        assert_eq!(const_rational(e).unwrap().to_string(), "1/2");

        // 1/3 + 1/3 + 1/3  =  1 (a whole number — distinct from the fractional case).
        let third = bin(&a, BinaryOpKind::ExactDivide, num(&a, 1), num(&a, 3));
        let e = bin(&a, BinaryOpKind::Add, bin(&a, BinaryOpKind::Add, third, third), third);
        assert_eq!(const_rational(e).unwrap().to_string(), "1");

        // (2/3 * 3/4)  =  1/2 — multiplication folds too.
        let e = bin(
            &a,
            BinaryOpKind::Multiply,
            bin(&a, BinaryOpKind::ExactDivide, num(&a, 2), num(&a, 3)),
            bin(&a, BinaryOpKind::ExactDivide, num(&a, 3), num(&a, 4)),
        );
        assert_eq!(const_rational(e).unwrap().to_string(), "1/2");
    }

    #[test]
    fn const_rational_refuses_a_non_constant_or_a_zero_divisor() {
        let a: Arena<Expr> = Arena::new();
        // A divide-by-zero subtree does NOT fold — it is left to error at runtime.
        let e = bin(&a, BinaryOpKind::ExactDivide, num(&a, 1), num(&a, 0));
        assert!(const_rational(e).is_none());
        // A variable leaf is not constant — no fold.
        let v = a.alloc(Expr::Identifier(Symbol::from_index(0)));
        let e = bin(&a, BinaryOpKind::Add, num(&a, 1), v);
        assert!(const_rational(e).is_none());
    }

    #[test]
    fn closed_form_is_a_bare_number_when_whole_and_a_reduced_divide_otherwise() {
        let stmt_arena: Arena<Stmt> = Arena::new();
        let expr_arena: Arena<Expr> = Arena::new();
        let interner = Interner::new();
        let cx = Cx {
            stmt_arena: &stmt_arena,
            expr_arena: &expr_arena,
            interner: &interner,
            fold_constants: true,
        };

        // A whole value → a bare Number literal (no division at runtime).
        let whole = cx.closed_form(&Rational::from_i64(3)).unwrap();
        assert!(matches!(whole, Expr::Literal(Literal::Number(3))));

        // A fraction → a single reduced ExactDivide(num, den).
        let frac = Rational::from_ratio_i64(4, 6).unwrap(); // reduces to 2/3
        match cx.closed_form(&frac).unwrap() {
            Expr::BinaryOp { op: BinaryOpKind::ExactDivide, left, right } => {
                assert!(matches!(left, Expr::Literal(Literal::Number(2))));
                assert!(matches!(right, Expr::Literal(Literal::Number(3))));
            }
            other => panic!("expected a reduced ExactDivide, got {other:?}"),
        }
    }
}
