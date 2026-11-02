//! Div-mod fusion (AOT / Rust-codegen path).
//!
//! rustc/LLVM does NOT common-subexpression-eliminate a `a / b` against a `a % b`: they
//! lower to a distinct `sdiv` and `srem`, and for a constant `b` each becomes its own
//! magic-multiply — so a loop computing BOTH pays two multiplies where gcc pays one
//! (gcc keeps the quotient and derives `r = a - q*b`). Measured: the `(i/7)+(i%7)` sum
//! loop is `0.55s` two-multiply vs `0.44s` one-multiply — a 1.25x cliff, and the exact
//! gap that makes the `divmod`/`quorem` parity caps lose to their C rivals.
//!
//! This pass restores the fusion our own codegen must do because LLVM won't: when one
//! statement's RHS contains BOTH `a / b` and `a % b` with the SAME side-effect-free
//! operands, bind `q = a / b` once immediately before the statement and rewrite
//! `a / b -> q`, `a % b -> a - q*b`. The result is the single-division shape.
//!
//! SOUNDNESS. `a` and `b` are restricted to identifiers / integer literals — re-reading
//! them in the introduced `let` and in `a - q*b` is value-identical (no side effects, no
//! reordered evaluation). Trap behaviour is preserved: `q = a / b` traps on `b == 0` and
//! on `i64::MIN / -1` exactly where the original `a / b` (and `a % b`) did; the derived
//! `a - q*b` never re-traps. The pass runs LAST (after the bounded-loop versioners) and
//! propagates each rewritten/created arithmetic node's proven-raw mark from the node it
//! replaces, so a fused accumulator fast-branch stays bare-i64 all the way to codegen.

use super::facts::{self, Fact};
use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};

pub fn divmod_fuse_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mut ctr = 0usize;
    fuse_block(stmts, ea, sa, interner, &mut ctr)
}

/// Are two operand expressions the SAME side-effect-free value (identifier or int literal)?
fn simple_eq(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Identifier(x), Expr::Identifier(y)) => x == y,
        (Expr::Literal(Literal::Number(x)), Expr::Literal(Literal::Number(y))) => x == y,
        _ => false,
    }
}

/// A fuseable operand is a bare identifier or integer literal (pure, cheap to re-read).
fn is_simple(e: &Expr) -> bool {
    matches!(e, Expr::Identifier(_) | Expr::Literal(Literal::Number(_)))
}

/// Collect every `(a, b)` worth fusing: a shared quotient pays off when there is at least
/// one real `a / b` to bind AND two-or-more total demands for it (`a/b` + `a%b`, or `a/b`
/// twice) — otherwise binding a quotient adds a division. `a` and `b` are side-effect-free
/// (identifier / int literal). Deduplicated by `simple_eq`.
fn find_pairs<'a>(e: &'a Expr<'a>, out: &mut Vec<(&'a Expr<'a>, &'a Expr<'a>)>) {
    let mut divs: Vec<(&Expr, &Expr)> = Vec::new();
    let mut mods: Vec<(&Expr, &Expr)> = Vec::new();
    collect_ops(e, &mut divs, &mut mods);
    for (a, b) in divs.iter().chain(mods.iter()) {
        if !is_simple(a) || !is_simple(b) {
            continue;
        }
        if out
            .iter()
            .any(|(oa, ob)| simple_eq(oa, a) && simple_eq(ob, b))
        {
            continue;
        }
        let nd = divs
            .iter()
            .filter(|(da, db)| simple_eq(da, a) && simple_eq(db, b))
            .count();
        let nm = mods
            .iter()
            .filter(|(ma, mb)| simple_eq(ma, a) && simple_eq(mb, b))
            .count();
        if nd >= 1 && nd + nm >= 2 {
            out.push((*a, *b));
        }
    }
}

fn collect_ops<'a>(
    e: &'a Expr<'a>,
    divs: &mut Vec<(&'a Expr<'a>, &'a Expr<'a>)>,
    mods: &mut Vec<(&'a Expr<'a>, &'a Expr<'a>)>,
) {
    match e {
        Expr::BinaryOp {
            op: BinaryOpKind::Divide,
            left,
            right,
        } => {
            divs.push((left, right));
            collect_ops(left, divs, mods);
            collect_ops(right, divs, mods);
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Modulo,
            left,
            right,
        } => {
            mods.push((left, right));
            collect_ops(left, divs, mods);
            collect_ops(right, divs, mods);
        }
        Expr::BinaryOp { left, right, .. } => {
            collect_ops(left, divs, mods);
            collect_ops(right, divs, mods);
        }
        Expr::Not { operand } => collect_ops(operand, divs, mods),
        Expr::Call { args, .. } => args.iter().for_each(|a| collect_ops(a, divs, mods)),
        _ => {}
    }
}

/// Rewrite `e`, replacing `a/b -> q` and `a%b -> (a - q*b)` for the one `(a,b)` pair, and
/// propagating proven-raw marks from each replaced/rebuilt node so a raw chain stays raw.
fn rewrite<'a>(
    e: &'a Expr<'a>,
    a: &'a Expr<'a>,
    b: &'a Expr<'a>,
    q: Symbol,
    ea: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    match e {
        Expr::BinaryOp {
            op: BinaryOpKind::Divide,
            left,
            right,
        } if simple_eq(left, a) && simple_eq(right, b) => ea.alloc(Expr::Identifier(q)),
        Expr::BinaryOp {
            op: BinaryOpKind::Modulo,
            left,
            right,
        } if simple_eq(left, a) && simple_eq(right, b) => {
            let was_raw = facts::holds_expr(Fact::RawIntOp, e);
            let qref = ea.alloc(Expr::Identifier(q));
            let mul = ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Multiply,
                left: qref,
                right: b,
            });
            let sub = ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Subtract,
                left: a,
                right: mul,
            });
            if was_raw {
                facts::mark_expr(Fact::RawIntOp, mul);
                facts::mark_expr(Fact::RawIntOp, sub);
            }
            sub
        }
        Expr::BinaryOp { op, left, right } => {
            let nl = rewrite(left, a, b, q, ea);
            let nr = rewrite(right, a, b, q, ea);
            if std::ptr::eq(nl, *left) && std::ptr::eq(nr, *right) {
                return e;
            }
            let n = ea.alloc(Expr::BinaryOp {
                op: *op,
                left: nl,
                right: nr,
            });
            if facts::holds_expr(Fact::RawIntOp, e) {
                facts::mark_expr(Fact::RawIntOp, n);
            }
            n
        }
        Expr::Not { operand } => {
            let no = rewrite(operand, a, b, q, ea);
            if std::ptr::eq(no, *operand) {
                e
            } else {
                ea.alloc(Expr::Not { operand: no })
            }
        }
        Expr::Call { function, args } => {
            let mut changed = false;
            let new_args: Vec<&Expr> = args
                .iter()
                .map(|arg| {
                    let na = rewrite(arg, a, b, q, ea);
                    if !std::ptr::eq(na, *arg) {
                        changed = true;
                    }
                    na
                })
                .collect();
            if changed {
                ea.alloc(Expr::Call {
                    function: *function,
                    args: new_args,
                })
            } else {
                e
            }
        }
        _ => e,
    }
}

/// Fuse every div-mod pair in one statement's value expression, returning the introduced
/// `let q = a/b` bindings (in order) plus the rewritten value.
fn fuse_value<'a>(
    value: &'a Expr<'a>,
    ea: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
    ctr: &mut usize,
) -> Option<(Vec<Stmt<'a>>, &'a Expr<'a>)> {
    let mut pairs = Vec::new();
    find_pairs(value, &mut pairs);
    if pairs.is_empty() {
        return None;
    }
    let mut lets: Vec<Stmt<'a>> = Vec::new();
    let mut cur = value;
    for (a, b) in pairs {
        let q = interner.intern(&format!("__divq{}", *ctr));
        *ctr += 1;
        // The shared quotient `q = a / b`, marked raw iff the fused `%` node was raw.
        let div = ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Divide,
            left: a,
            right: b,
        });
        if find_pair_raw(cur, a, b) {
            facts::mark_expr(Fact::RawIntOp, div);
        }
        lets.push(Stmt::Let {
            var: q,
            ty: None,
            value: div,
            mutable: false,
        });
        cur = rewrite(cur, a, b, q, ea);
    }
    Some((lets, cur))
}

/// Was either the `a/b` or `a%b` node for this pair proven-raw in `e`? (Determines whether
/// the introduced quotient binding is bare-i64.)
fn find_pair_raw(e: &Expr, a: &Expr, b: &Expr) -> bool {
    match e {
        Expr::BinaryOp {
            op: BinaryOpKind::Divide | BinaryOpKind::Modulo,
            left,
            right,
        } if simple_eq(left, a) && simple_eq(right, b) => facts::holds_expr(Fact::RawIntOp, e),
        Expr::BinaryOp { left, right, .. } => {
            find_pair_raw(left, a, b) || find_pair_raw(right, a, b)
        }
        Expr::Not { operand } => find_pair_raw(operand, a, b),
        Expr::Call { args, .. } => args.iter().any(|arg| find_pair_raw(arg, a, b)),
        _ => false,
    }
}

fn fuse_block<'a>(
    stmts: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    ctr: &mut usize,
) -> Vec<Stmt<'a>> {
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
    for s in stmts {
        match s {
            Stmt::Set { target, value } => {
                if let Some((lets, nv)) = fuse_value(value, ea, interner, ctr) {
                    super::mark_fired(crate::optimization::Opt::DivModFuse);
                    out.extend(lets);
                    out.push(Stmt::Set { target, value: nv });
                } else {
                    out.push(Stmt::Set { target, value });
                }
            }
            Stmt::Let {
                var,
                ty,
                value,
                mutable,
            } => {
                if let Some((lets, nv)) = fuse_value(value, ea, interner, ctr) {
                    super::mark_fired(crate::optimization::Opt::DivModFuse);
                    out.extend(lets);
                    out.push(Stmt::Let {
                        var,
                        ty,
                        value: nv,
                        mutable,
                    });
                } else {
                    out.push(Stmt::Let {
                        var,
                        ty,
                        value,
                        mutable,
                    });
                }
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => out.push(Stmt::If {
                cond,
                then_block: sa.alloc_slice(fuse_block(then_block.to_vec(), ea, sa, interner, ctr)),
                else_block: else_block.map(|eb| -> Block<'a> {
                    sa.alloc_slice(fuse_block(eb.to_vec(), ea, sa, interner, ctr))
                }),
            }),
            Stmt::While {
                cond,
                body,
                decreasing,
            } => out.push(Stmt::While {
                cond,
                body: sa.alloc_slice(fuse_block(body.to_vec(), ea, sa, interner, ctr)),
                decreasing,
            }),
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => out.push(Stmt::Repeat {
                pattern,
                iterable,
                body: sa.alloc_slice(fuse_block(body.to_vec(), ea, sa, interner, ctr)),
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
                name,
                generics,
                params,
                body: sa.alloc_slice(fuse_block(body.to_vec(), ea, sa, interner, ctr)),
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            }),
            other => out.push(other),
        }
    }
    out
}
