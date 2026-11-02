//! Bounded recursive inlining (recursion unrolling) for the AOT path.
//!
//! `gcc -O3` flattens self-recursion several levels deep (its
//! `max-inline-recursive-depth`, default ~8): the recursion tree becomes wide
//! straight-line code with a real recursive call only at the bottom. LLVM —
//! both `clang` and `rustc` — refuses to recursively inline, so it emits one
//! clean recursive call per node. That is the whole reason a compiled-LOGOS
//! recursive kernel (n-queens) and hand-written Rust both trail gcc-C by the
//! same margin: not code quality, but the missing recursive inline.
//!
//! Since LOGOS owns the source-level transform *before* the generated Rust
//! reaches LLVM, this pass does the inlining LLVM won't: each self-call in a
//! recursive function's own body is replaced by an inlined, alpha-renamed copy
//! of that body, whose own self-calls are inlined again, to a bounded depth
//! `k`; at depth `k` a real call bottoms out (so the program still terminates
//! for any runtime recursion depth — we *unroll*, never fully unfold). LLVM
//! then compiles the resulting nested loops as well as gcc does.
//!
//! **Scope is pipeline-dependent** (see [`is_eligible`]):
//!
//! - **AOT** ([`inline_recursive_fns`]): loop-interleaved recursion only (a
//!   self-call inside a loop body — the n-queens shape). Every other codegen
//!   recursion transform — tail-call elimination, accumulator introduction,
//!   double-recursion closed-form, auto-memoization — operates on RETURN-position
//!   recursion and converts it to an O(1)-stack loop / closed form, strictly
//!   better than unrolling. Those run after this pass and need the original
//!   shape, so unrolling them would clobber the better result. Loop-interleaved
//!   recursion is the one shape none of them handle — and exactly why n-queens
//!   trails C — so it is the uncontested territory the AOT pass owns.
//! - **run path** ([`inline_recursive_fns_run`]): loop-interleaved recursion
//!   AND genuine return-position tree recursion (fib/binary_trees). On the live
//!   VM+JIT path the AOT closed-form/memoization transforms never run, and the
//!   post-optimizer accumulator linearizer ignores multi-call returns, so
//!   unrolling is the only per-call-overhead lever for tree recursion there. The
//!   run-path depth is shape-aware: deep for tree recursion (its straight-line
//!   body tiers cleanly when enlarged), shallow for loop-interleaved recursion
//!   (its nested-loop body drops to the bytecode tier past a small cap).
//!
//! The pass is **always able to fire** on an eligible function; code size is
//! not a hard gate. Depth is a flag (`LOGOS_RECURSE_DEPTH`; default 8 for AOT,
//! [`DEFAULT_RUN_DEPTH`] for the run path) and the whole pass has a kill switch
//! (`LOGOS_RECURSE_INLINE=0`). A generous statement budget
//! (`LOGOS_RECURSE_BUDGET`) is only a runaway-loop safety valve — it stops
//! pathological unbounded blow-up, it is not a per-function policy.
//!
//! Soundness rules (each fail-closed — an unrecognised body is left untouched):
//! - **self-recursion only.** We inline a call *iff* it targets the enclosing
//!   function with matching arity. Calls to any other function (recursive or
//!   not) are never inlined, so mutual recursion is simply never triggered —
//!   inlining stays a single-function, always-sound body substitution.
//! - **evaluate args once.** Each argument is bound to a fresh `Let` before the
//!   inlined body, so a parameter used twice never duplicates a non-idempotent
//!   argument.
//! - **in-out parameters alias.** A `mutable` parameter is the caller's own
//!   variable (the language's in-out convention, and how a compiled tier hands a
//!   function the program's bindings), so the inlined copy names the caller's
//!   argument itself rather than a copy that its writes would be lost in. A call
//!   inlines only when each such argument is a plain name that no other argument
//!   mentions; otherwise it stays a real call.
//! - **no capture.** Parameters and locals of every inlined copy are
//!   alpha-renamed to fresh `__r{id}_…` names; bodies that shadow a bound name
//!   are rejected ([`bound_names_unique`]).
//! - **structured returns.** The body must be in a guarded-return normal form
//!   ([`returns_well_placed`] + [`definitely_returns`]): returns appear only as
//!   the last statement of the body or of an `If` branch, never inside a loop.
//!   Each `Return e` becomes `Set result to e` and an early guard
//!   `If c { Return A } …rest` becomes `If c { result = A } else { …rest }`,
//!   so no `goto`/flag is synthesised. Anything else is ineligible.
//! - **numeric fragment.** Eligible bodies use only `Let`/`Set`/`If`/`While`/
//!   `Return` over the arithmetic/bitwise/compare/`Index`/`Length` expression
//!   fragment (plus calls), returning `Int`/`Nat`. Effects, collections, and
//!   control we do not reconstruct make a function ineligible.

use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

/// Default recursive-inline depth for the AOT pipeline — matches gcc's
/// `max-inline-recursive-depth`. LLVM hands the enlarged body to a full
/// optimizing backend, which absorbs depth 8 well.
const DEFAULT_DEPTH: usize = 8;

/// Default recursive-inline depth for the live RUN path. The regalloc JIT tier
/// compiles the inlined body itself, and an interleaved A/B sweep showed two
/// distinct degradation curves by shape, so the run-path depth is shape-aware:
///
/// - **Tree recursion** (fib/binary_trees — straight-line arithmetic, no loop
///   nesting) keeps improving as it deepens (fib ON/OFF 0.64 → 0.54 → 0.50 → 0.49
///   at depth 1→2→3→4); the larger body still tiers cleanly. Depth 4 is the
///   plateau (~2× faster than un-inlined) and the requested run-path depth.
/// - **Loop-interleaved recursion** (n-queens — a self-call inside a `While`)
///   has a hard cliff: depth 2–3 is a win (ON/OFF ≈ 0.94–0.95), depth 4 is
///   neutral, depth 6 ≈ +32%, depth 8 ≈ +430% — the nested-loop region outgrows
///   what the tier handles and the whole function falls back to bytecode. It is
///   therefore capped at [`RUN_LOOP_DEPTH_CAP`], well clear of the cliff.
///
/// The optimizer cost lands inside the measured run, but the A/B confirms the
/// savings dominate at these depths (depth-4 fib is FASTER than depth-2, so the
/// extra unroll cost is paid back).
const DEFAULT_RUN_DEPTH: usize = 4;

/// Run-path depth ceiling for loop-interleaved recursion (the n-queens shape).
/// Its inlined nested loops explode the JIT region past this, dropping the
/// function to the bytecode tier — see [`DEFAULT_RUN_DEPTH`].
const RUN_LOOP_DEPTH_CAP: usize = 2;

/// Runaway-loop safety valve on statements emitted while unrolling one
/// function. Generous: n-queens at depth 8 needs ~80, fib ~512. Once crossed,
/// remaining self-calls bottom out as real calls (still correct).
const DEFAULT_BUDGET: usize = 20_000;

/// A function eligible for self-recursive unrolling: its parameter symbols, its
/// (validated) body, and whether its recursion is loop-interleaved (which caps
/// the run-path unroll depth — see [`RUN_LOOP_DEPTH_CAP`]).
struct RecCand<'a> {
    params: Vec<Symbol>,
    /// Per parameter: declared `mutable`, the caller's variable lent in-out.
    inout: Vec<bool>,
    body: Block<'a>,
    loop_interleaved: bool,
}

/// Whether a self-call with `args` can inline: every in-out argument is a plain name, and no other argument mentions
/// it (two names for one variable would make one parameter's writes visible through another).
fn in_out_args_alias(cand: &RecCand, args: &[&Expr]) -> bool {
    args.iter()
        .enumerate()
        .filter(|(i, _)| cand.inout[*i])
        .all(|(i, a)| match a {
            Expr::Identifier(sym) => args
                .iter()
                .enumerate()
                .all(|(j, other)| j == i || !super::expr_touches(other, *sym)),
            _ => false,
        })
}

// ---------------------------------------------------------------------------
// Eligibility
// ---------------------------------------------------------------------------

/// The arithmetic/bitwise/compare expression fragment (calls allowed — only
/// self-calls are ever inlined; others are left intact).
fn expr_ok(e: &Expr) -> bool {
    match e {
        Expr::Literal(_) | Expr::Identifier(_) => true,
        Expr::BinaryOp { left, right, .. } => expr_ok(left) && expr_ok(right),
        Expr::Not { operand } => expr_ok(operand),
        Expr::Index { collection, index } => expr_ok(collection) && expr_ok(index),
        Expr::Length { collection } => expr_ok(collection),
        Expr::Call { args, .. } => args.iter().all(|a| expr_ok(a)),
        _ => false,
    }
}

/// Statement fragment we can clone, rename and restructure. A `While`/`Repeat`
/// body must contain no `Return` (we never reconstruct loop-internal exits).
fn stmt_ok(s: &Stmt) -> bool {
    match s {
        Stmt::Let { value, .. } => expr_ok(value),
        Stmt::Set { value, .. } => expr_ok(value),
        Stmt::Return { value } => value.map_or(true, |e| expr_ok(e)),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            expr_ok(cond)
                && then_block.iter().all(stmt_ok)
                && else_block.map_or(true, |b| b.iter().all(stmt_ok))
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            expr_ok(cond)
                && decreasing.map_or(true, |d| expr_ok(d))
                && body.iter().all(stmt_ok)
                && !block_has_return(body)
        }
        _ => false,
    }
}

/// Any `Return` anywhere in a block (descending into `If`/`While` bodies).
fn block_has_return(block: Block) -> bool {
    block.iter().any(|s| match s {
        Stmt::Return { .. } => true,
        Stmt::If {
            then_block,
            else_block,
            ..
        } => block_has_return(then_block) || else_block.map_or(false, block_has_return),
        Stmt::While { body, .. } => block_has_return(body),
        _ => false,
    })
}

/// A block returns on every path: its last statement is a `Return`, or an `If`
/// whose both branches definitely return.
fn definitely_returns(block: Block) -> bool {
    match block.last() {
        Some(Stmt::Return { .. }) => true,
        Some(Stmt::If {
            then_block,
            else_block: Some(e),
            ..
        }) => definitely_returns(then_block) && definitely_returns(e),
        _ => false,
    }
}

/// Returns sit only where [`restructure_seq`] can lower them: last in their
/// block (top body or an `If` branch), or as an early guard
/// `If c { …returns… }` (else-less) with code after it. A mid-block `If` must
/// be either such a guard or wholly return-free.
fn returns_well_placed(block: Block) -> bool {
    let n = block.len();
    for (i, s) in block.iter().enumerate() {
        let last = i + 1 == n;
        match s {
            Stmt::Return { .. } => {
                if !last {
                    return false;
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                if last {
                    if !returns_well_placed(then_block) {
                        return false;
                    }
                    if let Some(e) = else_block {
                        if !returns_well_placed(e) {
                            return false;
                        }
                    }
                } else {
                    let guard = else_block.is_none() && definitely_returns(then_block);
                    if guard {
                        if !returns_well_placed(then_block) {
                            return false;
                        }
                    } else {
                        // Plain mid-block conditional: no returns may hide inside.
                        if block_has_return(then_block) {
                            return false;
                        }
                        if let Some(e) = else_block {
                            if block_has_return(e) {
                                return false;
                            }
                        }
                    }
                }
            }
            Stmt::While { body, .. } => {
                if block_has_return(body) {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

/// At least one self-call (`function == fname`) appears in the body.
fn has_self_call(block: Block, fname: Symbol) -> bool {
    fn in_expr(e: &Expr, fname: Symbol) -> bool {
        match e {
            Expr::Call { function, args } => {
                *function == fname || args.iter().any(|a| in_expr(a, fname))
            }
            Expr::BinaryOp { left, right, .. } => in_expr(left, fname) || in_expr(right, fname),
            Expr::Not { operand } => in_expr(operand, fname),
            Expr::Index { collection, index } => {
                in_expr(collection, fname) || in_expr(index, fname)
            }
            Expr::Length { collection } => in_expr(collection, fname),
            _ => false,
        }
    }
    block.iter().any(|s| match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, fname),
        Stmt::Return { value } => value.map_or(false, |e| in_expr(e, fname)),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            in_expr(cond, fname)
                || has_self_call(then_block, fname)
                || else_block.map_or(false, |b| has_self_call(b, fname))
        }
        Stmt::While { cond, body, .. } => in_expr(cond, fname) || has_self_call(body, fname),
        _ => false,
    })
}

/// Number of self-call sites anywhere in the body.
fn count_self_calls(block: Block, fname: Symbol) -> usize {
    fn in_expr(e: &Expr, f: Symbol) -> usize {
        match e {
            Expr::Call { function, args } => {
                (if *function == f { 1 } else { 0 })
                    + args.iter().map(|a| in_expr(a, f)).sum::<usize>()
            }
            Expr::BinaryOp { left, right, .. } => in_expr(left, f) + in_expr(right, f),
            Expr::Not { operand } => in_expr(operand, f),
            Expr::Index { collection, index } => in_expr(collection, f) + in_expr(index, f),
            Expr::Length { collection } => in_expr(collection, f),
            _ => 0,
        }
    }
    block
        .iter()
        .map(|s| match s {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, fname),
            Stmt::Return { value } => value.map_or(0, |e| in_expr(e, fname)),
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                in_expr(cond, fname)
                    + count_self_calls(then_block, fname)
                    + else_block.map_or(0, |b| count_self_calls(b, fname))
            }
            Stmt::While { cond, body, .. } => in_expr(cond, fname) + count_self_calls(body, fname),
            _ => 0,
        })
        .sum()
}

/// A self-call lexically inside a `While` body (recursion interleaved with
/// iteration — the n-queens shape, which loop-conversion cannot linearize).
fn has_self_call_in_loop(block: Block, fname: Symbol) -> bool {
    block.iter().any(|s| match s {
        Stmt::While { body, .. } => has_self_call(body, fname),
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            has_self_call_in_loop(then_block, fname)
                || else_block.map_or(false, |b| has_self_call_in_loop(b, fname))
        }
        _ => false,
    })
}

/// A self-call that is the direct value of a `Return` (tail position, including
/// guard returns) — tail-call elimination's domain.
fn has_tail_self_call(block: Block, fname: Symbol) -> bool {
    block.iter().any(|s| match s {
        Stmt::Return { value: Some(e) } => {
            matches!(e, Expr::Call { function, .. } if *function == fname)
        }
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            has_tail_self_call(then_block, fname)
                || else_block.map_or(false, |b| has_tail_self_call(b, fname))
        }
        Stmt::While { body, .. } => has_tail_self_call(body, fname),
        _ => false,
    })
}

/// True when the existing loop-conversion passes — tail-call elimination and
/// accumulator introduction (see `phase_optimize::tce_*`/`acc_*`) — already
/// linearize this recursion into an O(1)-stack `loop`, which is strictly better
/// than unrolling. We DEFER to them: fire only on recursion they cannot
/// linearize — a self-call interleaved inside a loop (n-queens) or genuine
/// multi-call tree recursion (fib). Single linear recursion in a return (tail,
/// or `Return f(..) op k` accumulator shape) and any tail self-call (ackermann's
/// tail calls included) belong to them.
fn handled_by_loop_conversion(block: Block, fname: Symbol) -> bool {
    has_tail_self_call(block, fname)
        || (count_self_calls(block, fname) == 1 && !has_self_call_in_loop(block, fname))
}

/// `Int`/`Nat` typed (the result temp is initialised to `0`).
fn is_intish(ty: Option<&TypeExpr>, it: &Interner) -> bool {
    matches!(ty, Some(TypeExpr::Primitive(s)) if {
        let n = it.resolve(*s);
        n == "Int" || n == "Nat"
    })
}

/// Every `Let`-bound local anywhere in the body.
fn collect_locals(block: Block, out: &mut HashSet<Symbol>) {
    for s in block {
        match s {
            Stmt::Let { var, .. } => {
                out.insert(*var);
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_locals(then_block, out);
                if let Some(b) = else_block {
                    collect_locals(b, out);
                }
            }
            Stmt::While { body, .. } => collect_locals(body, out),
            _ => {}
        }
    }
}

/// No param or `Let` local may appear as a binder twice — alpha-renaming maps
/// each `Symbol` to one fresh name, so a shadow would merge two bindings.
fn bound_names_unique(params: &[Symbol], body: Block) -> bool {
    fn walk(block: Block, seen: &mut HashSet<Symbol>) -> bool {
        for s in block {
            match s {
                Stmt::Let { var, .. } => {
                    if !seen.insert(*var) {
                        return false;
                    }
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    if !walk(then_block, seen) {
                        return false;
                    }
                    if let Some(b) = else_block {
                        if !walk(b, seen) {
                            return false;
                        }
                    }
                }
                Stmt::While { body, .. } => {
                    if !walk(body, seen) {
                        return false;
                    }
                }
                _ => {}
            }
        }
        true
    }
    let mut seen = HashSet::new();
    for p in params {
        if !seen.insert(*p) {
            return false;
        }
    }
    walk(body, &mut seen)
}

/// Genuine multi-call tree recursion (`fib`/`makeCheck`: two or more self-calls,
/// none in tail or accumulator position) with NO loop interleaving. The AOT
/// pipeline DEFERS this shape to its return-position transforms (double-recursion
/// closed-form, auto-memoization) which subsume unrolling. On the live run path
/// none of those passes run — `optimize_for_run` skips supercompile/closed-form
/// for the recursion shapes, and `tail_call::rewrite_accumulators` (the lone
/// post-optimizer linearizer) only fires on a SINGLE recursive return — so for
/// run-path tree recursion the unroller is the only transform that cuts the
/// per-call VM overhead, and unrolling it competes with nothing.
fn is_tree_recursion(block: Block, fname: Symbol) -> bool {
    count_self_calls(block, fname) >= 2
        && !has_self_call_in_loop(block, fname)
        && !has_tail_self_call(block, fname)
}

/// A function is unrollable when its body is in the supported fragment, returns
/// on every path through legal positions, calls itself, and shadows nothing.
///
/// Which recursion SHAPE qualifies depends on the pipeline:
/// - **AOT** (`run_path == false`): loop-interleaved recursion only (the
///   n-queens shape). Every codegen recursion transform (tail-call elimination,
///   accumulator introduction, double-recursion closed-form, auto-memoization)
///   operates on RETURN-position recursion, which this excludes — so the unroller
///   never competes with the superior, already-present AOT transform. Loop-
///   interleaved recursion is the one shape none of them handle (and exactly why
///   n-queens loses to gcc).
/// - **run path** (`run_path == true`): loop-interleaved recursion AND genuine
///   return-position tree recursion (fib/binary_trees). The AOT closed-form/
///   memoization transforms that own tree recursion do not run on the live path,
///   and the post-optimizer accumulator linearizer ignores multi-call returns,
///   so unrolling is the only per-call-overhead lever for the run-path recursion
///   cluster — and competes with nothing there.
fn is_eligible(
    params: &[Symbol],
    body: Block,
    return_type: Option<&TypeExpr>,
    fname: Symbol,
    run_path: bool,
    it: &Interner,
) -> bool {
    let shape_ok =
        has_self_call_in_loop(body, fname) || (run_path && is_tree_recursion(body, fname));
    is_intish(return_type, it)
        && body.iter().all(stmt_ok)
        && returns_well_placed(body)
        && definitely_returns(body)
        && has_self_call(body, fname)
        && shape_ok
        && !handled_by_loop_conversion(body, fname)
        && bound_names_unique(params, body)
}

// ---------------------------------------------------------------------------
// Alpha-renaming (full fragment, including calls)
// ---------------------------------------------------------------------------

fn ren(sym: Symbol, map: &HashMap<Symbol, Symbol>) -> Symbol {
    map.get(&sym).copied().unwrap_or(sym)
}

fn rename_expr<'a>(
    e: &'a Expr<'a>,
    map: &HashMap<Symbol, Symbol>,
    ea: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    match e {
        Expr::Identifier(s) => match map.get(s) {
            Some(r) => ea.alloc(Expr::Identifier(*r)),
            None => e,
        },
        Expr::Literal(_) => e,
        Expr::BinaryOp { op, left, right } => ea.alloc(Expr::BinaryOp {
            op: *op,
            left: rename_expr(left, map, ea),
            right: rename_expr(right, map, ea),
        }),
        Expr::Not { operand } => ea.alloc(Expr::Not {
            operand: rename_expr(operand, map, ea),
        }),
        Expr::Index { collection, index } => ea.alloc(Expr::Index {
            collection: rename_expr(collection, map, ea),
            index: rename_expr(index, map, ea),
        }),
        Expr::Length { collection } => ea.alloc(Expr::Length {
            collection: rename_expr(collection, map, ea),
        }),
        Expr::Call { function, args } => ea.alloc(Expr::Call {
            function: *function,
            args: args.iter().map(|a| rename_expr(a, map, ea)).collect(),
        }),
        _ => e,
    }
}

fn rename_block<'a>(
    block: Block<'a>,
    map: &HashMap<Symbol, Symbol>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Block<'a> {
    let out: Vec<Stmt<'a>> = block.iter().map(|s| rename_stmt(s, map, ea, sa)).collect();
    sa.alloc_slice(out)
}

fn rename_stmt<'a>(
    s: &Stmt<'a>,
    map: &HashMap<Symbol, Symbol>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Stmt<'a> {
    match s {
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => Stmt::Let {
            var: ren(*var, map),
            ty: *ty,
            value: rename_expr(value, map, ea),
            mutable: *mutable,
        },
        Stmt::Set { target, value } => Stmt::Set {
            target: ren(*target, map),
            value: rename_expr(value, map, ea),
        },
        Stmt::Return { value } => Stmt::Return {
            value: value.map(|e| rename_expr(e, map, ea)),
        },
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => Stmt::If {
            cond: rename_expr(cond, map, ea),
            then_block: rename_block(then_block, map, ea, sa),
            else_block: else_block.map(|b| rename_block(b, map, ea, sa)),
        },
        Stmt::While {
            cond,
            body,
            decreasing,
        } => Stmt::While {
            cond: rename_expr(cond, map, ea),
            body: rename_block(body, map, ea, sa),
            decreasing: decreasing.map(|d| rename_expr(d, map, ea)),
        },
        // The eligibility gate admitted only the arms above.
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Return-form restructuring: returns become assignments to `result`
// ---------------------------------------------------------------------------

/// Lower a (validated) body into statements that assign the function's return
/// value to `result` and fall through, applying alpha-rename `map`. Returns are
/// rewritten; early guards push the remainder into an `else` branch.
fn restructure_seq<'a>(
    block: Block<'a>,
    result: Symbol,
    map: &HashMap<Symbol, Symbol>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    let mut out: Vec<Stmt<'a>> = Vec::new();
    let n = block.len();
    for (i, s) in block.iter().enumerate() {
        let last = i + 1 == n;
        match s {
            Stmt::Return { value: Some(e) } => {
                out.push(Stmt::Set {
                    target: result,
                    value: rename_expr(e, map, ea),
                });
                break; // statements after a return are dead
            }
            Stmt::Return { value: None } => break,
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let guard = !last && else_block.is_none() && definitely_returns(then_block);
                if guard {
                    let rest = &block[i + 1..];
                    out.push(Stmt::If {
                        cond: rename_expr(cond, map, ea),
                        then_block: sa
                            .alloc_slice(restructure_seq(then_block, result, map, ea, sa)),
                        else_block: Some(
                            sa.alloc_slice(restructure_seq(rest, result, map, ea, sa)),
                        ),
                    });
                    break; // the rest now lives in the else branch
                } else if last
                    && (definitely_returns(then_block)
                        || else_block.map_or(false, definitely_returns))
                {
                    out.push(Stmt::If {
                        cond: rename_expr(cond, map, ea),
                        then_block: sa
                            .alloc_slice(restructure_seq(then_block, result, map, ea, sa)),
                        else_block: else_block
                            .map(|e| sa.alloc_slice(restructure_seq(e, result, map, ea, sa))),
                    });
                    break;
                } else {
                    // Plain conditional (no returns inside) — keep, continue.
                    out.push(rename_stmt(s, map, ea, sa));
                }
            }
            _ => out.push(rename_stmt(s, map, ea, sa)),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Self-call inlining driver
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn expand_call<'a>(
    args: &[&'a Expr<'a>],
    depth: usize,
    cand: &RecCand<'a>,
    fname: Symbol,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    counter: &mut usize,
    emitted: &mut usize,
    budget: usize,
    prelude: &mut Vec<Stmt<'a>>,
) -> &'a Expr<'a> {
    let id = *counter;
    *counter += 1;
    // Pre-charge the budget *before* descending so a deep nest trips the guard
    // on the way down (a post-order charge would fully expand first). The body
    // statement count is a fair proxy for the copy this inline emits.
    *emitted += cand.body.len() + cand.params.len() + 1;

    let mut locals = HashSet::new();
    collect_locals(cand.body, &mut locals);

    let mut map: HashMap<Symbol, Symbol> = HashMap::new();
    for (i, p) in cand.params.iter().enumerate() {
        let name = match args[i] {
            Expr::Identifier(caller) if cand.inout[i] => *caller,
            _ => it.intern(&format!("__r{id}_{}", it.resolve(*p))),
        };
        map.insert(*p, name);
    }
    for l in &locals {
        let name = format!("__r{id}_{}", it.resolve(*l));
        map.insert(*l, it.intern(&name));
    }
    let result = it.intern(&format!("__r{id}_result"));

    // Evaluate each argument exactly once into its renamed parameter slot; an in-out parameter already names the
    // caller's variable.
    for (i, (p, arg)) in cand.params.iter().zip(args.iter()).enumerate() {
        if !cand.inout[i] {
            prelude.push(Stmt::Let {
                var: ren(*p, &map),
                ty: None,
                value: arg,
                mutable: true,
            });
        }
    }
    // Declare the result temp; the restructured body assigns it on every path.
    prelude.push(Stmt::Let {
        var: result,
        ty: None,
        value: ea.alloc(Expr::Literal(Literal::Number(0))),
        mutable: true,
    });

    let restructured = restructure_seq(cand.body, result, &map, ea, sa);
    let lowered = rewrite_block_inline(
        &restructured,
        depth - 1,
        cand,
        fname,
        ea,
        sa,
        it,
        counter,
        emitted,
        budget,
    );
    prelude.extend(lowered);

    ea.alloc(Expr::Identifier(result))
}

#[allow(clippy::too_many_arguments)]
fn rewrite_expr_inline<'a>(
    e: &'a Expr<'a>,
    depth: usize,
    cand: &RecCand<'a>,
    fname: Symbol,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    counter: &mut usize,
    emitted: &mut usize,
    budget: usize,
    prelude: &mut Vec<Stmt<'a>>,
) -> &'a Expr<'a> {
    match e {
        Expr::Call { function, args } => {
            let new_args: Vec<&'a Expr<'a>> = args
                .iter()
                .map(|a| {
                    rewrite_expr_inline(
                        a, depth, cand, fname, ea, sa, it, counter, emitted, budget, prelude,
                    )
                })
                .collect();
            if *function == fname
                && new_args.len() == cand.params.len()
                && depth > 0
                && *emitted < budget
                && in_out_args_alias(cand, &new_args)
            {
                return expand_call(
                    &new_args, depth, cand, fname, ea, sa, it, counter, emitted, budget, prelude,
                );
            }
            ea.alloc(Expr::Call {
                function: *function,
                args: new_args,
            })
        }
        Expr::BinaryOp { op, left, right } => {
            let l = rewrite_expr_inline(
                left, depth, cand, fname, ea, sa, it, counter, emitted, budget, prelude,
            );
            // The right operand of `and`/`or` is conditionally evaluated;
            // hoisting a call out of it would always run it.
            let r = if matches!(op, BinaryOpKind::And | BinaryOpKind::Or) {
                *right
            } else {
                rewrite_expr_inline(
                    right, depth, cand, fname, ea, sa, it, counter, emitted, budget, prelude,
                )
            };
            ea.alloc(Expr::BinaryOp {
                op: *op,
                left: l,
                right: r,
            })
        }
        Expr::Not { operand } => {
            let o = rewrite_expr_inline(
                operand, depth, cand, fname, ea, sa, it, counter, emitted, budget, prelude,
            );
            ea.alloc(Expr::Not { operand: o })
        }
        Expr::Index { collection, index } => {
            let c = rewrite_expr_inline(
                collection, depth, cand, fname, ea, sa, it, counter, emitted, budget, prelude,
            );
            let i = rewrite_expr_inline(
                index, depth, cand, fname, ea, sa, it, counter, emitted, budget, prelude,
            );
            ea.alloc(Expr::Index {
                collection: c,
                index: i,
            })
        }
        Expr::Length { collection } => {
            let c = rewrite_expr_inline(
                collection, depth, cand, fname, ea, sa, it, counter, emitted, budget, prelude,
            );
            ea.alloc(Expr::Length { collection: c })
        }
        _ => e,
    }
}

#[allow(clippy::too_many_arguments)]
fn rewrite_stmt_inline<'a>(
    s: &Stmt<'a>,
    depth: usize,
    cand: &RecCand<'a>,
    fname: Symbol,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    counter: &mut usize,
    emitted: &mut usize,
    budget: usize,
    out: &mut Vec<Stmt<'a>>,
) {
    match s {
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => {
            let mut prelude = Vec::new();
            let v = rewrite_expr_inline(
                value,
                depth,
                cand,
                fname,
                ea,
                sa,
                it,
                counter,
                emitted,
                budget,
                &mut prelude,
            );
            out.extend(prelude);
            out.push(Stmt::Let {
                var: *var,
                ty: *ty,
                value: v,
                mutable: *mutable,
            });
        }
        Stmt::Set { target, value } => {
            let mut prelude = Vec::new();
            let v = rewrite_expr_inline(
                value,
                depth,
                cand,
                fname,
                ea,
                sa,
                it,
                counter,
                emitted,
                budget,
                &mut prelude,
            );
            out.extend(prelude);
            out.push(Stmt::Set {
                target: *target,
                value: v,
            });
        }
        Stmt::Return { value } => {
            let mut prelude = Vec::new();
            let v = value.map(|e| {
                rewrite_expr_inline(
                    e,
                    depth,
                    cand,
                    fname,
                    ea,
                    sa,
                    it,
                    counter,
                    emitted,
                    budget,
                    &mut prelude,
                )
            });
            out.extend(prelude);
            out.push(Stmt::Return { value: v });
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            // The condition is evaluated once on arrival → safe to lift.
            let mut prelude = Vec::new();
            let c = rewrite_expr_inline(
                cond,
                depth,
                cand,
                fname,
                ea,
                sa,
                it,
                counter,
                emitted,
                budget,
                &mut prelude,
            );
            out.extend(prelude);
            let tb = rewrite_block_inline(
                then_block, depth, cand, fname, ea, sa, it, counter, emitted, budget,
            );
            let eb = else_block.map(|b| {
                rewrite_block_inline(b, depth, cand, fname, ea, sa, it, counter, emitted, budget)
            });
            out.push(Stmt::If {
                cond: c,
                then_block: sa.alloc_slice(tb),
                else_block: eb.map(|b| sa.alloc_slice(b)),
            });
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            // The condition re-evaluates each iteration → do NOT lift from it.
            let b = rewrite_block_inline(
                body, depth, cand, fname, ea, sa, it, counter, emitted, budget,
            );
            out.push(Stmt::While {
                cond: *cond,
                body: sa.alloc_slice(b),
                decreasing: *decreasing,
            });
        }
        other => out.push(other.clone()),
    }
}

#[allow(clippy::too_many_arguments)]
fn rewrite_block_inline<'a>(
    block: &[Stmt<'a>],
    depth: usize,
    cand: &RecCand<'a>,
    fname: Symbol,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    counter: &mut usize,
    emitted: &mut usize,
    budget: usize,
) -> Vec<Stmt<'a>> {
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(block.len());
    for s in block {
        rewrite_stmt_inline(
            s, depth, cand, fname, ea, sa, it, counter, emitted, budget, &mut out,
        );
    }
    out
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

/// Unroll every eligible self-recursive function's own body to a bounded depth,
/// for the AOT pipeline (loop-interleaved recursion, depth `DEFAULT_DEPTH`).
/// Depth and the runaway budget come from the environment
/// (`LOGOS_RECURSE_DEPTH`, `LOGOS_RECURSE_BUDGET`); `LOGOS_RECURSE_INLINE=0`
/// disables the pass entirely.
pub fn inline_recursive_fns<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    inline_recursive_dispatch(stmts, expr_arena, stmt_arena, interner, false)
}

/// The live RUN-path entry: shallower default depth (`DEFAULT_RUN_DEPTH`) and a
/// wider eligible shape (loop-interleaved AND tree recursion — the run path has
/// no AOT closed-form/memoization transform to defer to). Same env overrides and
/// kill switch as the AOT entry.
pub fn inline_recursive_fns_run<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    inline_recursive_dispatch(stmts, expr_arena, stmt_arena, interner, true)
}

fn inline_recursive_dispatch<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    run_path: bool,
) -> Vec<Stmt<'a>> {
    let default_depth = if run_path {
        DEFAULT_RUN_DEPTH
    } else {
        DEFAULT_DEPTH
    };
    let depth: usize = std::env::var("LOGOS_RECURSE_DEPTH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default_depth);
    let budget: usize = std::env::var("LOGOS_RECURSE_BUDGET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_BUDGET);
    inline_recursive_with(
        stmts, expr_arena, stmt_arena, interner, depth, budget, run_path,
    )
}

/// Core transform with explicit depth/budget (env-independent — the unit tests
/// drive this directly). `run_path` widens the eligible recursion shape to
/// include return-position tree recursion (see [`is_eligible`]).
fn inline_recursive_with<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    depth: usize,
    budget: usize,
    run_path: bool,
) -> Vec<Stmt<'a>> {
    if depth == 0 {
        return stmts;
    }

    let mut cands: HashMap<Symbol, RecCand<'a>> = HashMap::new();
    for s in &stmts {
        if let Stmt::FunctionDef {
            name,
            generics,
            params,
            body,
            return_type,
            is_native: false,
            is_exported: false,
            opt_flags,
            ..
        } = s
        {
            if !generics.is_empty() || !opt_flags.is_on(crate::optimization::Opt::Unfold) {
                continue;
            }
            let param_syms: Vec<Symbol> = params.iter().map(|(p, _)| *p).collect();
            if is_eligible(&param_syms, body, *return_type, *name, run_path, interner) {
                let loop_interleaved = has_self_call_in_loop(body, *name);
                let inout = params
                    .iter()
                    .map(|(_, ty)| matches!(ty, TypeExpr::Mutable { .. }))
                    .collect();
                cands.insert(
                    *name,
                    RecCand {
                        params: param_syms,
                        inout,
                        body,
                        loop_interleaved,
                    },
                );
            }
        }
    }
    if cands.is_empty() {
        return stmts;
    }

    let mut counter: usize = 0;
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
    for s in &stmts {
        if let Stmt::FunctionDef {
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
        } = s
        {
            if let Some(cand) = cands.get(name) {
                // Run-path loop-interleaved recursion (n-queens) hits a tiering
                // cliff past RUN_LOOP_DEPTH_CAP; cap it there. Tree recursion and
                // the entire AOT path keep the requested depth.
                let eff_depth = if run_path && cand.loop_interleaved {
                    depth.min(RUN_LOOP_DEPTH_CAP)
                } else {
                    depth
                };
                let mut emitted: usize = 0;
                let new_body = rewrite_block_inline(
                    body,
                    eff_depth,
                    cand,
                    *name,
                    expr_arena,
                    stmt_arena,
                    interner,
                    &mut counter,
                    &mut emitted,
                    budget,
                );
                out.push(Stmt::FunctionDef {
                    name: *name,
                    generics: generics.clone(),
                    params: params.clone(),
                    body: stmt_arena.alloc_slice(new_body),
                    return_type: *return_type,
                    is_native: *is_native,
                    native_path: *native_path,
                    is_exported: *is_exported,
                    export_target: *export_target,
                    opt_flags: opt_flags.clone(),
                });
                continue;
            }
        }
        out.push(s.clone());
    }
    out
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/inline_recursive/tests.rs"]
mod tests;
