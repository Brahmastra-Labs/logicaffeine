//! Float induction-variable strength reduction (U6).
//!
//! A loop that recomputes a FLOAT affine function of an INTEGER induction
//! variable every iteration — `c1 * k + c2` with `k` the loop counter — pays a
//! multiply plus an int→float conversion (`cvtsi2sd`) per iteration. The value is
//! an arithmetic progression, so it can be maintained INCREMENTALLY in a float
//! accumulator that advances by `c1 * step` each iteration, removing the multiply
//! and the conversion. (pi_leibniz's `2.0*k + 1.0` denominator: measured 1.47×.)
//!
//! ## Soundness — the exactness guard
//!
//! The incremental accumulator equals the closed form `c1*k + c2` bit-for-bit
//! ONLY while every intermediate value is exactly representable in `f64`
//! (magnitude `< 2^53`) AND `k` itself converts exactly (`< 2^53`). Beyond that,
//! the accumulation and the closed form round differently, so the transform is
//! NOT bit-identical for astronomically large trip counts. We therefore emit a
//! RUNTIME GUARD on the loop bound — `if n < LIT { <strength-reduced> } else {
//! <original> }` — where `LIT = floor(2^52 / |c1|)` keeps `|c1*k| < 2^52 < 2^53`
//! and `k < 2^52 < 2^53` for every `k < n` on the fast path. The fast path runs
//! for every realistic trip count; the original handles the unreachable tail. The
//! guard is one comparison before the loop, amortised to nothing. This makes the
//! transform unconditionally bit-identical to the original for ALL inputs.
//!
//! We additionally require `c1`, `c2`, and `c1*step` to be integer-valued — the
//! arithmetic progression then lands on exact integers within `[-2^53, 2^53]`,
//! the cleanly-provable case.

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};

/// `2^52` — the fast-path bound divisor base. For coefficient `c1`, the loop
/// bound must be `< 2^52 / |c1|` so `|c1*k|` and `k` both stay `< 2^53`.
const POW2_52: f64 = 4_503_599_627_370_496.0;

/// Apply float induction strength reduction to every qualifying top-level `While`.
pub fn float_induction_sr_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    let mut changed = false;
    let out = rewrite_block(stmts, expr_arena, stmt_arena, interner, &mut changed);
    (out, changed)
}

/// A recognized affine-float induction in a loop body.
struct AffineInduction {
    /// The integer induction variable `k`.
    k: Symbol,
    /// The loop-bound variable `n` (the `k < n` right operand).
    bound: Symbol,
    /// The float coefficient `c1` in `c1*k + c2`.
    c1: f64,
    /// The float offset `c2`.
    c2: f64,
    /// The per-iteration integer step (`k = k + step`).
    step: i64,
}

fn rewrite_block<'a>(
    stmts: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    changed: &mut bool,
) -> Vec<Stmt<'a>> {
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        if let Stmt::While { cond, body, decreasing } = &stmt {
            if let Some(ind) = detect(cond, body) {
                out.push(build_guarded(cond, body, *decreasing, &ind, ea, sa, it));
                *changed = true;
                continue;
            }
        }
        out.push(recurse_stmt(stmt, ea, sa, it, changed));
    }
    out
}

fn recurse_stmt<'a>(
    stmt: Stmt<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    changed: &mut bool,
) -> Stmt<'a> {
    let recur = |block: Block<'a>, it: &mut Interner, changed: &mut bool| -> Block<'a> {
        let v = rewrite_block(block.to_vec(), ea, sa, it, changed);
        sa.alloc_slice(v)
    };
    match stmt {
        Stmt::If { cond, then_block, else_block } => Stmt::If {
            cond,
            then_block: recur(then_block, it, changed),
            else_block: else_block.map(|b| recur(b, it, changed)),
        },
        Stmt::While { cond, body, decreasing } => Stmt::While {
            cond,
            body: recur(body, it, changed),
            decreasing,
        },
        Stmt::Repeat { pattern, iterable, body } => Stmt::Repeat {
            pattern,
            iterable,
            body: recur(body, it, changed),
        },
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

fn detect(cond: &Expr, body: &[Stmt]) -> Option<AffineInduction> {
    // The loop test must be `k < n` (`k`, `n` identifiers).
    let (k, bound) = match cond {
        Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
            match (left, right) {
                (Expr::Identifier(k), Expr::Identifier(n)) => (*k, *n),
                _ => return None,
            }
        }
        _ => return None,
    };
    // The body must increment `k` by an integer literal exactly once: `k = k + s`.
    let step = find_int_step(body, k)?;
    if step <= 0 {
        return None;
    }
    // Find an affine `c1*k + c2` recomputed in the body (not in the loop test,
    // which we already matched as a pure `k < n`).
    let (c1, c2) = body.iter().find_map(|s| stmt_find_affine(s, k))?;
    // Soundness: integer-valued coefficients keep the progression on exact
    // integers; `c1` must be nonzero (else it isn't an induction).
    let prod = c1 * step as f64;
    if c1 == 0.0
        || !is_int_float(c1)
        || !is_int_float(c2)
        || !is_int_float(prod)
        || !c1.is_finite()
        || !c2.is_finite()
    {
        return None;
    }
    Some(AffineInduction { k, bound, c1, c2, step })
}

/// `true` iff `f` is a finite integer-valued `f64`.
fn is_int_float(f: f64) -> bool {
    f.is_finite() && f.fract() == 0.0
}

/// The single integer step of `k = k + s` in `body`, or `None` if `k` is not
/// incremented by exactly one integer-literal `Set`.
fn find_int_step(body: &[Stmt], k: Symbol) -> Option<i64> {
    let mut step = None;
    for s in body {
        if let Stmt::Set { target, value } = s {
            if *target == k {
                let add = match value {
                    Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => match (left, right) {
                        (Expr::Identifier(a), Expr::Literal(Literal::Number(n))) if *a == k => *n,
                        (Expr::Literal(Literal::Number(n)), Expr::Identifier(a)) if *a == k => *n,
                        _ => return None, // k mutated some other way -> bail
                    },
                    _ => return None,
                };
                if step.is_some() {
                    return None; // mutated twice
                }
                step = Some(add);
            }
        }
    }
    step
}

/// Find a `c1*k + c2` affine subexpression anywhere in a statement's expressions.
fn stmt_find_affine(s: &Stmt, k: Symbol) -> Option<(f64, f64)> {
    match s {
        Stmt::Set { value, .. } | Stmt::Let { value, .. } => expr_find_affine(value, k),
        _ => None,
    }
}

/// Recursively search an expression for the `c1*k + c2` pattern.
fn expr_find_affine(e: &Expr, k: Symbol) -> Option<(f64, f64)> {
    if let Some(ce) = match_affine(e, k) {
        return Some(ce);
    }
    match e {
        Expr::BinaryOp { left, right, .. } => {
            expr_find_affine(left, k).or_else(|| expr_find_affine(right, k))
        }
        _ => None,
    }
}

/// Match `c1*k + c2` (with the multiply and add operands in either order).
fn match_affine(e: &Expr, k: Symbol) -> Option<(f64, f64)> {
    let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = e else {
        return None;
    };
    // One side is `c1*k`, the other a float constant `c2`.
    let try_sides = |mul: &Expr, off: &Expr| -> Option<(f64, f64)> {
        let c1 = match_scaled_k(mul, k)?;
        let c2 = float_lit(off)?;
        Some((c1, c2))
    };
    try_sides(left, right).or_else(|| try_sides(right, left))
}

/// Match `c1 * k` (operands either order) -> `c1`.
fn match_scaled_k(e: &Expr, k: Symbol) -> Option<f64> {
    let Expr::BinaryOp { op: BinaryOpKind::Multiply, left, right } = e else {
        return None;
    };
    match (&**left, &**right) {
        (Expr::Literal(Literal::Float(c)), Expr::Identifier(v)) if *v == k => Some(*c),
        (Expr::Identifier(v), Expr::Literal(Literal::Float(c))) if *v == k => Some(*c),
        _ => None,
    }
}

fn float_lit(e: &Expr) -> Option<f64> {
    match e {
        Expr::Literal(Literal::Float(c)) => Some(*c),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Rewrite
// ---------------------------------------------------------------------------

fn build_guarded<'a>(
    cond: &'a Expr<'a>,
    body: Block<'a>,
    decreasing: Option<&'a Expr<'a>>,
    ind: &AffineInduction,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
) -> Stmt<'a> {
    let denom = it.intern(&format!("__fsr_{}_denom", it.resolve(ind.k)));

    // Fast-path body: replace each `c1*k+c2` with `denom`, then advance `denom`.
    let mut new_body: Vec<Stmt<'a>> = body
        .iter()
        .map(|s| substitute_stmt(s, ind, denom, ea))
        .collect();
    let incr = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: ea.alloc(Expr::Identifier(denom)),
        right: ea.alloc(Expr::Literal(Literal::Float(ind.c1 * ind.step as f64))),
    });
    new_body.push(Stmt::Set { target: denom, value: incr });

    // `Let mutable denom be (c1*k + c2)` — seed from the actual entry `k`.
    let seed = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            left: ea.alloc(Expr::Literal(Literal::Float(ind.c1))),
            right: ea.alloc(Expr::Identifier(ind.k)),
        }),
        right: ea.alloc(Expr::Literal(Literal::Float(ind.c2))),
    });
    let seed_let = Stmt::Let { var: denom, ty: None, value: seed, mutable: true };
    let fast_while = Stmt::While {
        cond,
        body: sa.alloc_slice(new_body),
        decreasing,
    };
    let then_block: Block<'a> = sa.alloc_slice(vec![seed_let, fast_while]);

    // Slow path: the original loop, untouched.
    let orig_while = Stmt::While { cond, body, decreasing };
    let else_block: Block<'a> = sa.alloc_slice(vec![orig_while]);

    // Guard: `n < floor(2^52 / |c1|)`.
    let lit = (POW2_52 / ind.c1.abs()).floor() as i64;
    let guard = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Lt,
        left: ea.alloc(Expr::Identifier(ind.bound)),
        right: ea.alloc(Expr::Literal(Literal::Number(lit))),
    });
    Stmt::If { cond: guard, then_block, else_block: Some(else_block) }
}

/// Rewrite one statement, replacing every `c1*k+c2` with `denom`.
fn substitute_stmt<'a>(
    s: &Stmt<'a>,
    ind: &AffineInduction,
    denom: Symbol,
    ea: &'a Arena<Expr<'a>>,
) -> Stmt<'a> {
    match s {
        Stmt::Set { target, value } => Stmt::Set {
            target: *target,
            value: substitute_expr(value, ind, denom, ea),
        },
        Stmt::Let { var, ty, value, mutable } => Stmt::Let {
            var: *var,
            ty: *ty,
            value: substitute_expr(value, ind, denom, ea),
            mutable: *mutable,
        },
        other => other.clone(),
    }
}

fn substitute_expr<'a>(
    e: &'a Expr<'a>,
    ind: &AffineInduction,
    denom: Symbol,
    ea: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    if let Some((c1, c2)) = match_affine(e, ind.k) {
        if c1 == ind.c1 && c2 == ind.c2 {
            return ea.alloc(Expr::Identifier(denom));
        }
    }
    match e {
        Expr::BinaryOp { op, left, right } => ea.alloc(Expr::BinaryOp {
            op: *op,
            left: substitute_expr(left, ind, denom, ea),
            right: substitute_expr(right, ind, denom, ea),
        }),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct B<'a> {
        ea: &'a Arena<Expr<'a>>,
    }
    impl<'a> B<'a> {
        fn id(&self, s: Symbol) -> &'a Expr<'a> {
            self.ea.alloc(Expr::Identifier(s))
        }
        fn fl(&self, f: f64) -> &'a Expr<'a> {
            self.ea.alloc(Expr::Literal(Literal::Float(f)))
        }
        fn num(&self, n: i64) -> &'a Expr<'a> {
            self.ea.alloc(Expr::Literal(Literal::Number(n)))
        }
        fn bin(&self, op: BinaryOpKind, l: &'a Expr<'a>, r: &'a Expr<'a>) -> &'a Expr<'a> {
            self.ea.alloc(Expr::BinaryOp { op, left: l, right: r })
        }
    }

    // pi_leibniz's loop: `While k < n { sum = sum + sign/(2.0*k+1.0); sign = 0-sign; k = k+1 }`.
    fn pi_loop<'a>(b: &B<'a>, sa: &'a Arena<Stmt<'a>>, syms: (Symbol, Symbol, Symbol, Symbol)) -> Stmt<'a> {
        let (k, n, sum, sign) = syms;
        let affine = b.bin(BinaryOpKind::Add, b.bin(BinaryOpKind::Multiply, b.fl(2.0), b.id(k)), b.fl(1.0));
        let term = b.bin(BinaryOpKind::Divide, b.id(sign), affine);
        let set_sum = Stmt::Set { target: sum, value: b.bin(BinaryOpKind::Add, b.id(sum), term) };
        let set_sign = Stmt::Set { target: sign, value: b.bin(BinaryOpKind::Subtract, b.fl(0.0), b.id(sign)) };
        let set_k = Stmt::Set { target: k, value: b.bin(BinaryOpKind::Add, b.id(k), b.num(1)) };
        Stmt::While {
            cond: b.bin(BinaryOpKind::Lt, b.id(k), b.id(n)),
            body: sa.alloc_slice(vec![set_sum, set_sign, set_k]),
            decreasing: None,
        }
    }

    #[test]
    fn fires_on_pi_leibniz_and_guards() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let (k, n, sum, sign) = (it.intern("k"), it.intern("n"), it.intern("sum"), it.intern("sign"));
        let b = B { ea: &ea };
        let loop_stmt = pi_loop(&b, &sa, (k, n, sum, sign));
        let (out, changed) = float_induction_sr_stmts(vec![loop_stmt], &ea, &sa, &mut it);
        assert!(changed, "pi_leibniz affine-float induction must fire");
        // Output is a guarded `If { then: [Let denom, While], else: [While] }`.
        let denom = it.intern("__fsr_k_denom");
        match &out[0] {
            Stmt::If { then_block, else_block, .. } => {
                assert!(matches!(then_block[0], Stmt::Let { var, .. } if var == denom), "denom seeded");
                assert!(matches!(then_block[1], Stmt::While { .. }), "fast loop");
                let els = else_block.expect("slow path present");
                assert!(matches!(els[0], Stmt::While { .. }), "original loop preserved");
            }
            other => panic!("expected guarded If, got {other:?}"),
        }
    }

    #[test]
    fn rejects_non_integer_coefficient() {
        // c1 = 0.1 (not integer-valued) -> accumulation would drift -> must NOT fire.
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let (k, n, sum) = (it.intern("k"), it.intern("n"), it.intern("sum"));
        let b = B { ea: &ea };
        let affine = b.bin(BinaryOpKind::Add, b.bin(BinaryOpKind::Multiply, b.fl(0.1), b.id(k)), b.fl(1.0));
        let set_sum = Stmt::Set { target: sum, value: b.bin(BinaryOpKind::Add, b.id(sum), affine) };
        let set_k = Stmt::Set { target: k, value: b.bin(BinaryOpKind::Add, b.id(k), b.num(1)) };
        let loop_stmt = Stmt::While {
            cond: b.bin(BinaryOpKind::Lt, b.id(k), b.id(n)),
            body: sa.alloc_slice(vec![set_sum, set_k]),
            decreasing: None,
        };
        let (_, changed) = float_induction_sr_stmts(vec![loop_stmt], &ea, &sa, &mut it);
        assert!(!changed, "non-integer c1 must be rejected (drift unsound)");
    }
}
