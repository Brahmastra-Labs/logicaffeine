//! Which integer variables must be stored as the overflow-promoting `LogosInt`
//! (rather than a bare `i64`) so the compiled AOT matches the tree-walker's
//! unbounded-integer semantics.
//!
//! A variable is *promotable* when some assignment to it can produce a value
//! beyond `i64`:
//!   - a compile-time bignum constant (`2 ** 100`),
//!   - an expression referencing an already-promotable variable (`big * big`),
//!   - a multiplicative / exponentiating self-accumulator (`p = p * i`), or
//!   - a doubling additive accumulator (`x = x + x`).
//!
//! Linear counters (`i = i + 1`) are deliberately NOT promoted — they stay on
//! the `i64` fast path. Promotion propagates to a fixpoint, so a chain of
//! promotable derivations all become `LogosInt`.

use std::collections::HashSet;

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

/// The whole-program set of `Int`-returning functions whose return value can
/// exceed i64 (so their signature becomes `-> LogosInt`). Fixpoint over the call
/// graph: a function returning the result of another bignum function is itself
/// bignum-returning.
pub(super) fn bigint_returning_fns(stmts: &[Stmt], interner: &Interner) -> HashSet<Symbol> {
    let is_int_ret = |rt: Option<&TypeExpr>| {
        matches!(rt, Some(TypeExpr::Primitive(s)) if {
            let n = interner.resolve(*s);
            n == "Int" || n == "Nat"
        })
    };
    let fns: Vec<(Symbol, &[Stmt])> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef { name, body, return_type, .. } if is_int_ret(*return_type) => {
                Some((*name, &body[..]))
            }
            _ => None,
        })
        .collect();

    let mut bigint: HashSet<Symbol> = HashSet::new();
    loop {
        let mut changed = false;
        for &(name, body) in &fns {
            if !bigint.contains(&name) && returns_bigint(body, &bigint) {
                bigint.insert(name);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    bigint
}

/// The set of integer variables in `stmts` (including nested blocks) that must
/// be stored as `LogosInt`. `bigint_fns` is the set of functions that RETURN a
/// `LogosInt` — a call to one is itself a bignum-producing expression.
pub(super) fn promotable_int_vars(stmts: &[Stmt], bigint_fns: &HashSet<Symbol>) -> HashSet<Symbol> {
    let mut assigns: Vec<(Symbol, &Expr)> = Vec::new();
    collect_assigns(stmts, &mut assigns);

    let mut promotable: HashSet<Symbol> = HashSet::new();
    loop {
        let mut changed = false;
        for &(target, value) in &assigns {
            if !promotable.contains(&target) && rhs_can_grow(value, target, &promotable, bigint_fns) {
                promotable.insert(target);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    promotable
}

/// Does this function body RETURN a value that can exceed i64 — i.e. some
/// `Return <expr>` whose expression reads a promotable local or calls a
/// `bigint_fn`? Used to compute the fixpoint of bignum-returning functions.
pub(super) fn returns_bigint(body: &[Stmt], bigint_fns: &HashSet<Symbol>) -> bool {
    let promotable = promotable_int_vars(body, bigint_fns);
    let mut found = false;
    for_each_return(body, &mut |e| {
        if mentions_promotable(e, &promotable) || mentions_bigint_call(e, bigint_fns) {
            found = true;
        }
    });
    found
}

fn for_each_return<'a>(stmts: &'a [Stmt<'a>], f: &mut impl FnMut(&'a Expr<'a>)) {
    for s in stmts {
        match s {
            Stmt::Return { value: Some(e) } => f(e),
            Stmt::If { then_block, else_block, .. } => {
                for_each_return(then_block, f);
                if let Some(e) = else_block {
                    for_each_return(e, f);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                for_each_return(body, f)
            }
            _ => {}
        }
    }
}

fn mentions_bigint_call(e: &Expr, bigint_fns: &HashSet<Symbol>) -> bool {
    match e {
        Expr::Call { function, .. } => bigint_fns.contains(function),
        Expr::BinaryOp { left, right, .. } => {
            mentions_bigint_call(left, bigint_fns) || mentions_bigint_call(right, bigint_fns)
        }
        Expr::Not { operand } => mentions_bigint_call(operand, bigint_fns),
        _ => false,
    }
}

fn collect_assigns<'a>(stmts: &'a [Stmt<'a>], out: &mut Vec<(Symbol, &'a Expr<'a>)>) {
    for s in stmts {
        match s {
            Stmt::Let { var, value, .. } => out.push((*var, value)),
            Stmt::Set { target, value } => out.push((*target, value)),
            Stmt::If { then_block, else_block, .. } => {
                collect_assigns(then_block, out);
                if let Some(e) = else_block {
                    collect_assigns(e, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                collect_assigns(body, out)
            }
            _ => {}
        }
    }
}

/// Can this assignment RHS produce a value beyond `i64`?
fn rhs_can_grow(value: &Expr, target: Symbol, promotable: &HashSet<Symbol>, bigint_fns: &HashSet<Symbol>) -> bool {
    match value {
        // A call to a bignum-returning function (directly, or inside arithmetic).
        Expr::Call { function, .. } if bigint_fns.contains(function) => true,
        Expr::BinaryOp { op, left, right } => match op {
            // Multiplicative growth / exponentiation: a bignum constant, a
            // promotable operand, a bignum call, or the target itself.
            BinaryOpKind::Multiply | BinaryOpKind::Pow => {
                const_overflows_i64(value)
                    || mentions(left, target)
                    || mentions(right, target)
                    || mentions_promotable(left, promotable)
                    || mentions_promotable(right, promotable)
                    || mentions_bigint_call(left, bigint_fns)
                    || mentions_bigint_call(right, bigint_fns)
            }
            // Additive: a bignum constant, a promotable operand, a bignum call,
            // or a DOUBLING accumulator (`x + x`) — never the linear `i + 1`.
            BinaryOpKind::Add | BinaryOpKind::Subtract => {
                const_overflows_i64(value)
                    || mentions_promotable(left, promotable)
                    || mentions_promotable(right, promotable)
                    || mentions_bigint_call(left, bigint_fns)
                    || mentions_bigint_call(right, bigint_fns)
                    || (mentions(left, target) && mentions(right, target))
            }
            _ => false,
        },
        _ => false,
    }
}

fn mentions(e: &Expr, sym: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s == sym,
        Expr::BinaryOp { left, right, .. } => mentions(left, sym) || mentions(right, sym),
        Expr::Not { operand } => mentions(operand, sym),
        _ => false,
    }
}

fn mentions_promotable(e: &Expr, promotable: &HashSet<Symbol>) -> bool {
    match e {
        Expr::Identifier(s) => promotable.contains(s),
        Expr::BinaryOp { left, right, .. } => {
            mentions_promotable(left, promotable) || mentions_promotable(right, promotable)
        }
        Expr::Not { operand } => mentions_promotable(operand, promotable),
        _ => false,
    }
}

/// True iff `e` is a constant integer expression whose exact value exceeds `i64`.
fn const_overflows_i64(e: &Expr) -> bool {
    const_eval_bigint(e).map_or(false, |v| v.to_i64().is_none())
}

fn const_eval_bigint(e: &Expr) -> Option<logicaffeine_base::BigInt> {
    use logicaffeine_base::BigInt;
    match e {
        Expr::Literal(Literal::Number(n)) => Some(BigInt::from_i64(*n)),
        Expr::BinaryOp { op, left, right } => {
            let l = const_eval_bigint(left)?;
            let r = const_eval_bigint(right)?;
            match op {
                BinaryOpKind::Add => Some(l.add(&r)),
                BinaryOpKind::Subtract => Some(l.sub(&r)),
                BinaryOpKind::Multiply => Some(l.mul(&r)),
                BinaryOpKind::Pow => {
                    let exp = u32::try_from(r.to_i64()?).ok()?;
                    Some(l.pow(exp))
                }
                _ => None,
            }
        }
        _ => None,
    }
}
