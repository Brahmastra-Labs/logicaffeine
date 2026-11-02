//! Bridge from the optimizer's AST to the kernel's linear-integer-arithmetic
//! decision procedure (`logicaffeine_kernel::lia`).
//!
//! The interval domain proves `x ∈ [lo, hi]` but cannot express the
//! cross-variable relations a loop nest establishes: `i + j < n` (nested
//! affine, string_search's `text[i+j]`), `w - wi + 1 <= cap + 1` under a
//! guard `w >= wi` (multi-variable relational, knapsack's `prev[w-wi+1]`),
//! or `u < length(dist)` where `u` is an element of an array whose values
//! are all `< n` and `length(dist) = n` (symbolic element bound,
//! graph_bfs's `dist[u]`). These are not three problems — they are one
//! decision: **is a goal inequality implied by a conjunction of affine facts
//! over the integers?**
//!
//! The kernel already answers it. `logicaffeine_kernel::lia` carries a
//! complete Fourier–Motzkin solver (`LinearExpr`, `Constraint`,
//! `fourier_motzkin_unsat`) that was, until now, only wired to the kernel's
//! own theorem-proving tactics — orphaned from the optimizer one crate over.
//! This module is the missing wire: it reifies AST index/bound expressions
//! into the kernel's `LinearExpr`, gathers loop/path guards as kernel
//! `Constraint`s, and discharges `1 <= idx <= length` through the kernel
//! prover. No second engine — the theorem prover IS the bounds prover.
//!
//! The single-variable relational recognizer (`affine_of`/`guard_proves` in
//! `abstract_interp`) is the special case of this with one variable and one
//! guard; this generalizes it to an arbitrary affine system.

use crate::ast::stmt::{BinaryOpKind, Expr, Literal};
use crate::intern::Symbol;
use logicaffeine_kernel::lia::{fourier_motzkin_unsat, LinearExpr, Rational};

/// The kernel's linear-arithmetic types, re-exported under the optimizer's
/// names so callers (the abstract interpreter's `scalar_def` map, the bounds
/// prover) need not reach into the kernel crate directly. `LinExpr` aliases
/// `LinearExpr`.
pub(crate) use logicaffeine_kernel::lia::{Constraint, LinearExpr as LinExpr};

/// The kernel's `LinearExpr` indexes variables by `i64`; an interned symbol's
/// dense index is a stable, collision-free choice (we only ever build
/// variables this way, so the kernel's name-hash indices never clash).
fn vidx(s: Symbol) -> i64 {
    s.index() as i64
}

/// The linear expression `x` for a symbol.
pub(crate) fn var(s: Symbol) -> LinearExpr {
    LinearExpr::var(vidx(s))
}

/// The prover variable index of a scalar symbol.
pub(crate) fn var_index(s: Symbol) -> i64 {
    vidx(s)
}

/// The prover variable index of `length(c)`: an index of its own, below every symbol's, so a length and a scalar
/// never share a variable.
pub(crate) fn len_index(c: Symbol) -> i64 {
    -(c.index() as i64) - 1
}

/// The collection whose length a prover variable index names, if it names one.
pub(crate) fn len_owner(index: i64) -> Option<Symbol> {
    (index < 0 && index > ENTRY_BASE).then(|| Symbol::from_index((-(index + 1)) as usize))
}

/// The linear expression `length(c)` for a collection. Only a proof system holds one; [`expr_of`] declines it.
pub(crate) fn len_var(c: Symbol) -> LinearExpr {
    LinearExpr::var(len_index(c))
}

/// Indices at or below this name the length a function's parameter had on entry, one per parameter position,
/// past every length index.
const ENTRY_BASE: i64 = i64::MIN / 2;

/// The prover variable index of the length parameter `position` had when its function was entered.
pub(crate) fn entry_len_index(position: usize) -> i64 {
    ENTRY_BASE - position as i64
}

/// The constant linear expression `n`.
pub(crate) fn konst(n: i64) -> LinearExpr {
    LinearExpr::constant(Rational::from_i64(n))
}

/// `a <= b` as a kernel constraint (`a - b <= 0`).
pub(crate) fn le(a: &LinearExpr, b: &LinearExpr) -> Constraint {
    Constraint {
        expr: a.sub(b),
        strict: false,
    }
}

/// `a < b` as a kernel constraint (`a - b < 0`).
pub(crate) fn lt(a: &LinearExpr, b: &LinearExpr) -> Constraint {
    Constraint {
        expr: a.sub(b),
        strict: true,
    }
}

/// `a >= b` as a kernel constraint.
pub(crate) fn ge(a: &LinearExpr, b: &LinearExpr) -> Constraint {
    le(b, a)
}

/// `a > b` as a kernel constraint.
pub(crate) fn gt(a: &LinearExpr, b: &LinearExpr) -> Constraint {
    lt(b, a)
}

/// `e >= 0` as a kernel constraint (`-e <= 0`).
pub(crate) fn nonneg(e: &LinearExpr) -> Constraint {
    Constraint {
        expr: e.neg(),
        strict: false,
    }
}

/// Are `facts` (each a kernel `expr <= 0` / `< 0`) mutually SATISFIABLE? A
/// contradictory fact set proves every goal vacuously, so a bounds proof must
/// refuse to fire on one — a defensive net against any false hypothesis
/// (a stale or self-referential fact) slipping into the system.
pub(crate) fn consistent(facts: &[Constraint]) -> bool {
    !fourier_motzkin_unsat(facts)
}

/// Is `goal >= 0` implied by `facts` (each already a kernel `expr <= 0` / `< 0`)
/// over the integers? The integer negation of `goal >= 0` is `goal <= -1`, i.e.
/// `goal + 1 <= 0`; if `facts ∧ (goal + 1 <= 0)` is unsatisfiable, the goal is
/// valid. Soundness is the kernel's (rational-unsat ⟹ integer-unsat) and is
/// additionally certified against Z3 by the differential below.
pub(crate) fn prove(facts: &[Constraint], goal: &LinearExpr) -> bool {
    let neg = goal.add(&LinearExpr::constant(Rational::from_i64(1)));
    let mut system: Vec<Constraint> = facts.to_vec();
    system.push(Constraint {
        expr: neg,
        strict: false,
    });
    fourier_motzkin_unsat(&system)
}

/// Reify an AST expression into a kernel `LinearExpr`, or `None` if it is not
/// affine over symbols (a product of two variables, a modulo, a call, …). The
/// multi-variable generalization of `abstract_interp::affine_of`.
pub(crate) fn lin_of(e: &Expr) -> Option<LinearExpr> {
    // Cap literals well below i64: the callers render coefficients back to
    // `i64` program constants, and a larger literal is not a bounds
    // expression worth proving, so decline it. The `LinearExpr` arithmetic
    // itself is exact at any magnitude.
    const LIT_CAP: u64 = 1 << 50;
    match e {
        Expr::Identifier(s) => Some(var(*s)),
        Expr::Literal(Literal::Number(n)) => (n.unsigned_abs() <= LIT_CAP).then(|| konst(*n)),
        Expr::BinaryOp { op, left, right } => match op {
            BinaryOpKind::Add => Some(lin_of(left)?.add(&lin_of(right)?)),
            BinaryOpKind::Subtract => Some(lin_of(left)?.sub(&lin_of(right)?)),
            BinaryOpKind::Multiply => {
                let (l, r) = (lin_of(left)?, lin_of(right)?);
                if l.is_constant() {
                    Some(r.scale(&l.constant))
                } else if r.is_constant() {
                    Some(l.scale(&r.constant))
                } else {
                    None // nonlinear (x·y)
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// [`lin_of`] with `length of c` read as [`len_var`]`(c)`, for a collection `stable` says keeps its length wherever
/// the expression's value is used.
pub(crate) fn lin_of_with_lengths(e: &Expr, stable: &dyn Fn(Symbol) -> bool) -> Option<LinearExpr> {
    match e {
        Expr::Length { collection } => match collection {
            Expr::Identifier(c) if stable(*c) => Some(len_var(*c)),
            _ => None,
        },
        Expr::BinaryOp { op, left, right } => match op {
            BinaryOpKind::Add => {
                Some(lin_of_with_lengths(left, stable)?.add(&lin_of_with_lengths(right, stable)?))
            }
            BinaryOpKind::Subtract => {
                Some(lin_of_with_lengths(left, stable)?.sub(&lin_of_with_lengths(right, stable)?))
            }
            BinaryOpKind::Multiply => {
                let (l, r) = (
                    lin_of_with_lengths(left, stable)?,
                    lin_of_with_lengths(right, stable)?,
                );
                if l.is_constant() {
                    Some(r.scale(&l.constant))
                } else if r.is_constant() {
                    Some(l.scale(&r.constant))
                } else {
                    None
                }
            }
            _ => lin_of(e),
        },
        _ => lin_of(e),
    }
}

/// The AST expression for an integer linear expression (`c₀ + Σ cᵢ·xᵢ`), or `None` when a coefficient or the
/// constant is fractional or leaves i64. The inverse of [`lin_of`] over its image: a variable index is the symbol's
/// own index ([`var`]).
pub(crate) fn expr_of<'a>(
    e: &LinearExpr,
    ea: &'a crate::arena::Arena<Expr<'a>>,
) -> Option<&'a Expr<'a>> {
    let int = |r: &Rational| if r.is_integer() { r.to_i64() } else { None };
    let lit = |n: i64| -> &'a Expr<'a> { ea.alloc(Expr::Literal(Literal::Number(n))) };
    let mut out: Option<&'a Expr<'a>> = None;
    for (v, c) in &e.coefficients {
        let c = int(c)?;
        if c == 0 {
            continue;
        }
        let x: &'a Expr<'a> = ea.alloc(Expr::Identifier(Symbol::from_index(
            usize::try_from(*v).ok()?,
        )));
        let term = if c == 1 {
            x
        } else {
            ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Multiply,
                left: lit(c),
                right: x,
            })
        };
        out = Some(match out {
            None => term,
            Some(acc) => ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left: acc,
                right: term,
            }),
        });
    }
    let k = int(&e.constant)?;
    Some(match out {
        None => lit(k),
        Some(acc) if k == 0 => acc,
        Some(acc) => ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left: acc,
            right: lit(k),
        }),
    })
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/affine/tests.rs"]
mod tests;

#[cfg(all(test, feature = "verification"))]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/affine/z3_certifier.rs"]
mod z3_certifier;
