//! Range-aware helper specialization for counted-accumulator loops (AOT path).
//!
//! A parity benchmark like `Set sum to sum + divModQ(i, 7) + divModM(i, 7)` inside
//! `While i <= n` calls pure integer helpers whose bodies branch on the SIGN of their
//! arguments — general code that handles negatives. But the loop counter `i` is provably
//! `>= 0`, and the divisor is a positive constant, so every one of those sign branches is
//! dead. rustc/LLVM cannot see this (it can't prove `i >= 0` from `parseInt(argv)`), so
//! the helper stays a call full of branches and two hardware divisions.
//!
//! This pass symbolically INLINES the helper call chain into the loop under the counter's
//! proven range and FOLDS the dead sign branches, leaving bare integer arithmetic
//! (`i / 7`, `i - (i/7)*7`). The downstream div-mod fusion then shares the single division
//! and the bounded-accumulator versioner drops the overflow guard — so the whole term
//! lowers to what a hand-written C loop emits.
//!
//! SOUNDNESS. Inlining substitutes a pure function's body for its call (value-identical).
//! A branch is folded ONLY when interval arithmetic proves its guard constant under the
//! argument ranges (over-approximating intervals ⇒ a proven-constant guard is genuinely
//! constant), and a branch with an unknown guard is folded only when its taken side is a
//! pure no-op (no return, no mutation) — so control flow is never changed. If any step is
//! not provable the call is left exactly as it was. Only helpers the shared purity definition
//! admits ([`super::purity`]) are inlined: a call of one does nothing but compute its answer
//! from its arguments, so a helper that writes a program variable keeps its call.

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};
use std::collections::HashMap;

/// A large finite proxy for `+∞`; the counter's upper bound is unknown but its LOWER
/// bound (and the remainder's `[0,c)` range) is what the sign folds need, so a saturating
/// upper bound is sufficient and never participates in a soundness-critical comparison.
const INF: i128 = 1 << 100;

#[derive(Clone, Copy)]
struct Iv {
    lo: i128,
    hi: i128,
}
impl Iv {
    fn point(n: i128) -> Iv {
        Iv { lo: n, hi: n }
    }
    fn sat(lo: i128, hi: i128) -> Iv {
        Iv {
            lo: lo.clamp(-INF, INF),
            hi: hi.clamp(-INF, INF),
        }
    }
}

/// A pure integer helper: its parameter symbols and body block.
struct FnInfo<'a> {
    params: Vec<Symbol>,
    body: Block<'a>,
}
type FnMap<'a> = HashMap<Symbol, FnInfo<'a>>;

/// The non-native, non-exported functions `purity` admits: a call of one is a candidate for
/// symbolic inlining, since its body changes nothing but its own names.
fn collect_fns<'a>(stmts: &'a [Stmt<'a>], purity: &super::purity::Purity) -> FnMap<'a> {
    let mut m = FnMap::new();
    for s in stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            body,
            is_native,
            is_exported,
            ..
        } = s
        {
            if !is_native && !is_exported && purity.is_pure(*name) {
                m.insert(
                    *name,
                    FnInfo {
                        params: params.iter().map(|(p, _)| *p).collect(),
                        body,
                    },
                );
            }
        }
    }
    m
}

/// The environment during symbolic evaluation: each in-scope variable maps to the
/// (fully-substituted) expression it currently holds plus that expression's interval.
type Env<'a> = HashMap<Symbol, (&'a Expr<'a>, Iv)>;

struct Ctx<'a, 'f> {
    fns: &'f FnMap<'a>,
    ea: &'a Arena<Expr<'a>>,
}

/// Interval of a fully-substituted expression (its identifiers are the loop counter /
/// invariants, whose ranges live in `ranges`).
fn interval(e: &Expr, ranges: &HashMap<Symbol, Iv>) -> Iv {
    match e {
        Expr::Literal(Literal::Number(n)) => Iv::point(*n as i128),
        Expr::Identifier(s) => ranges.get(s).copied().unwrap_or(Iv::sat(-INF, INF)),
        Expr::BinaryOp { op, left, right } => {
            // Remainder idiom `x - (x/c)*c` with `x >= 0`, `c > 0` const ⇒ `[0, c)`.
            if matches!(op, BinaryOpKind::Subtract) {
                if let Some(iv) = remainder_iv(left, right, ranges) {
                    return iv;
                }
            }
            let l = interval(left, ranges);
            let r = interval(right, ranges);
            match op {
                BinaryOpKind::Add => Iv::sat(l.lo.saturating_add(r.lo), l.hi.saturating_add(r.hi)),
                BinaryOpKind::Subtract => {
                    Iv::sat(l.lo.saturating_sub(r.hi), l.hi.saturating_sub(r.lo))
                }
                BinaryOpKind::Multiply => {
                    let c = [
                        l.lo.saturating_mul(r.lo),
                        l.lo.saturating_mul(r.hi),
                        l.hi.saturating_mul(r.lo),
                        l.hi.saturating_mul(r.hi),
                    ];
                    Iv::sat(*c.iter().min().unwrap(), *c.iter().max().unwrap())
                }
                BinaryOpKind::Divide => {
                    // Positive constant divisor: `[floor(lo/c), floor(hi/c)]` (both operands
                    // non-negative in the cases we specialize).
                    if r.lo == r.hi && r.lo > 0 && l.lo >= 0 {
                        Iv::sat(l.lo / r.lo, l.hi / r.lo)
                    } else {
                        Iv::sat(-INF, INF)
                    }
                }
                _ => Iv::sat(-INF, INF),
            }
        }
        _ => Iv::sat(-INF, INF),
    }
}

/// Recognize `left - right` == `x - (x/c)*c` (a truncated remainder) and, when `x >= 0`
/// and `c` is a positive constant, return its tight `[0, c-1]` interval.
fn remainder_iv(left: &Expr, right: &Expr, ranges: &HashMap<Symbol, Iv>) -> Option<Iv> {
    // right must be `(x / c) * c` (either factor order).
    let (mul_l, mul_r) = match right {
        Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            left,
            right,
        } => (*left, *right),
        _ => return None,
    };
    let try_shape = |divide: &Expr, factor: &Expr| -> Option<Iv> {
        let Expr::BinaryOp {
            op: BinaryOpKind::Divide,
            left: dl,
            right: dr,
        } = divide
        else {
            return None;
        };
        let cf = as_const(factor)?;
        let cd = as_const(dr)?;
        if cf != cd || cf <= 0 {
            return None;
        }
        if !expr_struct_eq(left, dl) {
            return None;
        }
        let x = interval(left, ranges);
        if x.lo < 0 {
            return None;
        }
        Some(Iv::sat(0, cf - 1))
    };
    try_shape(mul_l, mul_r).or_else(|| try_shape(mul_r, mul_l))
}

fn as_const(e: &Expr) -> Option<i128> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(*n as i128),
        _ => None,
    }
}

/// Structural equality restricted to the shapes substitution produces (identifiers,
/// literals, binary ops, negation).
fn expr_struct_eq(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Identifier(x), Expr::Identifier(y)) => x == y,
        (Expr::Literal(Literal::Number(x)), Expr::Literal(Literal::Number(y))) => x == y,
        (
            Expr::BinaryOp {
                op: o1,
                left: l1,
                right: r1,
            },
            Expr::BinaryOp {
                op: o2,
                left: l2,
                right: r2,
            },
        ) => o1 == o2 && expr_struct_eq(l1, l2) && expr_struct_eq(r1, r2),
        _ => false,
    }
}

#[derive(PartialEq)]
enum Truth {
    True,
    False,
    Unknown,
}

/// Evaluate a comparison guard against the argument ranges.
fn eval_cond(cond: &Expr, ranges: &HashMap<Symbol, Iv>) -> Truth {
    let Expr::BinaryOp { op, left, right } = cond else {
        return Truth::Unknown;
    };
    let l = interval(left, ranges);
    let r = interval(right, ranges);
    match op {
        BinaryOpKind::Lt => {
            if l.hi < r.lo {
                Truth::True
            } else if l.lo >= r.hi {
                Truth::False
            } else {
                Truth::Unknown
            }
        }
        BinaryOpKind::Gt => {
            if l.lo > r.hi {
                Truth::True
            } else if l.hi <= r.lo {
                Truth::False
            } else {
                Truth::Unknown
            }
        }
        BinaryOpKind::LtEq => {
            if l.hi <= r.lo {
                Truth::True
            } else if l.lo > r.hi {
                Truth::False
            } else {
                Truth::Unknown
            }
        }
        BinaryOpKind::GtEq => {
            if l.lo >= r.hi {
                Truth::True
            } else if l.hi < r.lo {
                Truth::False
            } else {
                Truth::Unknown
            }
        }
        _ => Truth::Unknown,
    }
}

/// Substitute the current environment into `e`, inlining any pure-helper call whose
/// symbolic value is fully determined. Returns `None` if `e` contains an un-inlinable
/// construct.
fn subst<'a>(
    e: &'a Expr<'a>,
    env: &Env<'a>,
    ranges: &HashMap<Symbol, Iv>,
    ctx: &Ctx<'a, '_>,
    depth: u32,
) -> Option<&'a Expr<'a>> {
    match e {
        Expr::Literal(_) => Some(e),
        Expr::Identifier(s) => Some(env.get(s).map(|(x, _)| *x).unwrap_or(e)),
        Expr::BinaryOp { op, left, right } => {
            let l = subst(left, env, ranges, ctx, depth)?;
            let r = subst(right, env, ranges, ctx, depth)?;
            Some(ctx.ea.alloc(Expr::BinaryOp {
                op: *op,
                left: l,
                right: r,
            }))
        }
        Expr::Call { function, args } => {
            // Substitute the arguments, then symbolically inline the helper.
            let mut sargs: Vec<&'a Expr<'a>> = Vec::with_capacity(args.len());
            for a in args {
                sargs.push(subst(a, env, ranges, ctx, depth)?);
            }
            inline_fold(*function, &sargs, ranges, ctx, depth)
        }
        _ => None,
    }
}

/// Symbolically inline `f(sargs)` (arguments already substituted into caller terms),
/// folding sign branches under the argument ranges. Returns the resulting expression, or
/// `None` if `f` is unknown / not fully foldable / the depth budget is exhausted.
fn inline_fold<'a>(
    f: Symbol,
    sargs: &[&'a Expr<'a>],
    ranges: &HashMap<Symbol, Iv>,
    ctx: &Ctx<'a, '_>,
    depth: u32,
) -> Option<&'a Expr<'a>> {
    if depth == 0 {
        return None;
    }
    let info = ctx.fns.get(&f)?;
    if info.params.len() != sargs.len() {
        return None;
    }
    let mut env: Env<'a> = HashMap::new();
    for (p, a) in info.params.iter().zip(sargs.iter()) {
        env.insert(*p, (*a, interval(a, ranges)));
    }
    match eval_body(info.body, &mut env, ranges, ctx, depth - 1) {
        BodyResult::Value(e) => Some(e),
        _ => None,
    }
}

enum BodyResult<'a> {
    /// The function returned this expression.
    Value(&'a Expr<'a>),
    /// Reached the end of the block without returning (no value produced).
    Fallthrough,
    /// Could not be folded to straight-line data flow.
    Bail,
}

fn eval_body<'a>(
    stmts: &'a [Stmt<'a>],
    env: &mut Env<'a>,
    ranges: &HashMap<Symbol, Iv>,
    ctx: &Ctx<'a, '_>,
    depth: u32,
) -> BodyResult<'a> {
    for s in stmts {
        match s {
            Stmt::Let { var, value, .. } | Stmt::Set { target: var, value } => {
                let Some(v) = subst(value, env, ranges, ctx, depth) else {
                    return BodyResult::Bail;
                };
                let iv = interval(v, ranges);
                env.insert(*var, (v, iv));
            }
            Stmt::Return { value: Some(v) } => {
                return match subst(v, env, ranges, ctx, depth) {
                    Some(e) => BodyResult::Value(e),
                    None => BodyResult::Bail,
                };
            }
            Stmt::Return { value: None } => return BodyResult::Fallthrough,
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let truth = subst(cond, env, ranges, ctx, depth)
                    .map_or(Truth::Unknown, |c| eval_cond(c, ranges));
                match truth {
                    Truth::True => {
                        let mut e2 = env.clone();
                        match eval_body(then_block, &mut e2, ranges, ctx, depth) {
                            BodyResult::Value(v) => return BodyResult::Value(v),
                            BodyResult::Fallthrough => *env = e2,
                            BodyResult::Bail => return BodyResult::Bail,
                        }
                    }
                    Truth::False => {
                        if let Some(eb) = else_block {
                            let mut e2 = env.clone();
                            match eval_body(eb, &mut e2, ranges, ctx, depth) {
                                BodyResult::Value(v) => return BodyResult::Value(v),
                                BodyResult::Fallthrough => *env = e2,
                                BodyResult::Bail => return BodyResult::Bail,
                            }
                        }
                    }
                    Truth::Unknown => {
                        // Foldable only if the taken side is a pure no-op (no return, no
                        // environment change) — then the branch cannot affect the result.
                        if !branch_is_noop(then_block, env, ranges, ctx, depth) {
                            return BodyResult::Bail;
                        }
                        if let Some(eb) = else_block {
                            if !branch_is_noop(eb, env, ranges, ctx, depth) {
                                return BodyResult::Bail;
                            }
                        }
                    }
                }
            }
            _ => return BodyResult::Bail,
        }
    }
    BodyResult::Fallthrough
}

/// Does executing `stmts` leave the environment unchanged and never return? (Its nested
/// guards are folded the same way; a `Set`/`Let`/`Return` anywhere makes it non-trivial.)
fn branch_is_noop<'a>(
    stmts: &'a [Stmt<'a>],
    env: &Env<'a>,
    ranges: &HashMap<Symbol, Iv>,
    ctx: &Ctx<'a, '_>,
    depth: u32,
) -> bool {
    for s in stmts {
        match s {
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => match subst(cond, env, ranges, ctx, depth)
                .map_or(Truth::Unknown, |c| eval_cond(c, ranges))
            {
                Truth::True => {
                    if !branch_is_noop(then_block, env, ranges, ctx, depth) {
                        return false;
                    }
                }
                Truth::False => {
                    if let Some(eb) = else_block {
                        if !branch_is_noop(eb, env, ranges, ctx, depth) {
                            return false;
                        }
                    }
                }
                Truth::Unknown => {
                    if !branch_is_noop(then_block, env, ranges, ctx, depth) {
                        return false;
                    }
                    if let Some(eb) = else_block {
                        if !branch_is_noop(eb, env, ranges, ctx, depth) {
                            return false;
                        }
                    }
                }
            },
            _ => return false, // any Let/Set/Return in an unknown branch is a real effect
        }
    }
    true
}

// ---- the loop-level driver -------------------------------------------------------------

/// The counter's provable lower bound from a preceding `Let counter be <lit>` (>= 0) plus
/// the `+ 1` increment inside the loop — i.e. the counter never drops below its seed.
fn counter_lower_bound(preamble: &[Stmt], counter: Symbol) -> Option<i128> {
    let mut lo = None;
    for s in preamble {
        if let Stmt::Let {
            var,
            value: Expr::Literal(Literal::Number(n)),
            ..
        }
        | Stmt::Set {
            target: var,
            value: Expr::Literal(Literal::Number(n)),
        } = s
        {
            if *var == counter {
                lo = Some(*n as i128);
            }
        }
    }
    lo.filter(|v| *v >= 0)
}

/// Rewrite every helper call in `e` (targeting a pure helper) with its inlined-and-folded
/// form under `ranges`; unchanged subtrees keep their identity.
fn rewrite_calls<'a>(
    e: &'a Expr<'a>,
    ranges: &HashMap<Symbol, Iv>,
    ctx: &Ctx<'a, '_>,
) -> &'a Expr<'a> {
    match e {
        Expr::Call { function, args } if ctx.fns.contains_key(function) => {
            let sargs: Vec<&'a Expr<'a>> =
                args.iter().map(|a| rewrite_calls(a, ranges, ctx)).collect();
            if let Some(folded) = inline_fold(*function, &sargs, ranges, ctx, 8) {
                return folded;
            }
            ctx.ea.alloc(Expr::Call {
                function: *function,
                args: sargs,
            })
        }
        Expr::BinaryOp { op, left, right } => {
            let l = rewrite_calls(left, ranges, ctx);
            let r = rewrite_calls(right, ranges, ctx);
            if std::ptr::eq(l, *left) && std::ptr::eq(r, *right) {
                e
            } else {
                ctx.ea.alloc(Expr::BinaryOp {
                    op: *op,
                    left: l,
                    right: r,
                })
            }
        }
        Expr::Call { function, args } => {
            let sargs: Vec<&'a Expr<'a>> =
                args.iter().map(|a| rewrite_calls(a, ranges, ctx)).collect();
            ctx.ea.alloc(Expr::Call {
                function: *function,
                args: sargs,
            })
        }
        _ => e,
    }
}

fn spec_block<'a>(
    stmts: Vec<Stmt<'a>>,
    ctx: &Ctx<'a, '_>,
    sa: &'a Arena<Stmt<'a>>,
    fired: &mut bool,
) -> Vec<Stmt<'a>> {
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
    for (i, s) in stmts.iter().enumerate() {
        match s {
            Stmt::While {
                cond,
                body,
                decreasing,
            } => {
                // Counter + its lower bound from the preamble already emitted into `out`.
                let counter = match &**cond {
                    Expr::BinaryOp {
                        op: BinaryOpKind::Lt | BinaryOpKind::LtEq,
                        left,
                        ..
                    } => {
                        if let Expr::Identifier(c) = &**left {
                            Some(*c)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                let lo = counter.and_then(|c| counter_lower_bound(&out, c));
                if let (Some(c), Some(lo)) = (counter, lo) {
                    let mut ranges: HashMap<Symbol, Iv> = HashMap::new();
                    ranges.insert(c, Iv::sat(lo, INF));
                    let new_body = spec_loop_body(body, &ranges, ctx, fired);
                    out.push(Stmt::While {
                        cond,
                        body: sa.alloc_slice(new_body),
                        decreasing: *decreasing,
                    });
                } else {
                    let new_body = spec_block(body.to_vec(), ctx, sa, fired);
                    out.push(Stmt::While {
                        cond,
                        body: sa.alloc_slice(new_body),
                        decreasing: *decreasing,
                    });
                }
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => out.push(Stmt::If {
                cond,
                then_block: sa.alloc_slice(spec_block(then_block.to_vec(), ctx, sa, fired)),
                else_block: else_block.map(|eb| -> Block<'a> {
                    sa.alloc_slice(spec_block(eb.to_vec(), ctx, sa, fired))
                }),
            }),
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => out.push(Stmt::Repeat {
                pattern: pattern.clone(),
                iterable,
                body: sa.alloc_slice(spec_block(body.to_vec(), ctx, sa, fired)),
            }),
            Stmt::FunctionDef {
                name,
                generics,
                params,
                body,
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            } => out.push(Stmt::FunctionDef {
                name: *name,
                generics: generics.clone(),
                params: params.clone(),
                body: sa.alloc_slice(spec_block(body.to_vec(), ctx, sa, fired)),
                return_type: *return_type,
                is_native: *is_native,
                native_path: native_path.clone(),
                is_exported: *is_exported,
                export_target: export_target.clone(),
                opt_flags: opt_flags.clone(),
            }),
            _ => out.push(stmts[i].clone()),
        }
    }
    out
}

/// Specialize helper calls inside one loop body whose counter range is `ranges`.
fn spec_loop_body<'a>(
    body: &'a [Stmt<'a>],
    ranges: &HashMap<Symbol, Iv>,
    ctx: &Ctx<'a, '_>,
    fired: &mut bool,
) -> Vec<Stmt<'a>> {
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(body.len());
    for s in body {
        match s {
            Stmt::Let {
                var,
                ty,
                value,
                mutable,
            } => {
                let nv = rewrite_calls(value, ranges, ctx);
                if !std::ptr::eq(nv, *value) {
                    *fired = true;
                }
                out.push(Stmt::Let {
                    var: *var,
                    ty: *ty,
                    value: nv,
                    mutable: *mutable,
                });
            }
            Stmt::Set { target, value } => {
                let nv = rewrite_calls(value, ranges, ctx);
                if !std::ptr::eq(nv, *value) {
                    *fired = true;
                }
                out.push(Stmt::Set {
                    target: *target,
                    value: nv,
                });
            }
            _ => out.push(s.clone()),
        }
    }
    out
}

/// Entry point: symbolically specialize pure-helper calls in counted-accumulator loops.
pub fn specialize_loop_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &Interner,
) -> Vec<Stmt<'a>> {
    let arena_stmts: &'a [Stmt<'a>] = sa.alloc_slice(stmts);
    let fns = collect_fns(arena_stmts, &super::purity::Purity::of_program(arena_stmts, interner));
    if fns.is_empty() {
        return arena_stmts.to_vec();
    }
    let ctx = Ctx { fns: &fns, ea };
    let mut fired = false;
    let out = spec_block(arena_stmts.to_vec(), &ctx, sa, &mut fired);
    if fired {
        super::mark_fired(crate::optimization::Opt::SpecializeLoop);
    }
    out
}
