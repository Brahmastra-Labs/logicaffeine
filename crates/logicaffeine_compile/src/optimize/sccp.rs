//! Sparse conditional constant propagation (SCCP — Wegman & Zadeh, 1991) over
//! the structured LOGOS AST.
//!
//! SCCP folds constants and prunes unreachable branches in a SINGLE lattice
//! sweep, so it discovers constants that the iterated `fold → propagate →
//! deadcode` pipeline structurally cannot:
//!
//!   * `propagate` refuses any variable that is EVER a `Set` target (its
//!     `collect_all_set_targets` is whole-scope), so a mutable that provably
//!     holds one value at a program point is never substituted there.
//!   * `deadcode` prunes an `If` only when the condition is ALREADY a literal;
//!     it never derives the condition's constancy from flow.
//!
//! SCCP threads a per-variable lattice — `⊤` (absent = no constraint yet),
//! `Const(lit)`, `⊥` (provably non-constant) — forward through the block
//! structure, meeting at `If` joins and widening loop-carried variables to `⊥`.
//! The lattice has HEIGHT 2, so a single structured pass suffices: a cell can
//! only ever lower `⊤ → Const → ⊥`, which is exactly why loops need no
//! iteration cap to converge.
//!
//! Everything SCCP emits is semantics-preserving by construction — it only
//!   (a) replaces a `Let`/`Set` value or an `If`/`While` condition with its
//!       substitute-and-fold under PROVEN constants (same value),
//!   (b) drops a branch whose condition folded to a literal (dead code).
//! Constant substitution reuses [`propagate::subst_and_fold`], so SCCP inherits
//! its refusal to substitute into `Index`/`Slice`/`Closure`/interpolated
//! strings — the positions the codegen swap/vec-fill/for-range peepholes depend
//! on. The ONE soundness obligation is that the environment is a sound
//! over-approximation; every statement's write effect is modelled by
//! [`WriteSet`], whose catch-all is OPAQUE (clear the whole environment), so
//! an unmodelled or future statement kind can only cost precision, never
//! soundness.

use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};

use super::writes::{pattern_symbols, CalleeGlobals, WriteSet};
use super::{fold, propagate};

/// A per-variable lattice cell. A symbol ABSENT from the environment is `⊤`
/// ("no constraint yet / not assigned on this path"). `Copy` because it holds
/// only an arena reference.
#[derive(Clone, Copy)]
enum Lat<'a> {
    /// Proven equal to this literal expression (always an `Expr::Literal`).
    Const(&'a Expr<'a>),
    /// Provably not a single compile-time constant.
    Bottom,
}

type Env<'a> = HashMap<Symbol, Lat<'a>>;

/// SCCP entry: sweep a top-level statement list.
pub fn sccp_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let bools = fold::program_bool_syms(&stmts, interner);
    // Globals a CALLEE mutates (`Set counter` inside `bump`) are never a single compile-time constant across a
    // call, so SCCP must NEVER fold them — mirroring `propagate`'s guard. Without this, `Let counter be 0;
    // While …: bump(); Show counter` folds `counter` to `0` even though `bump()` increments it (soundness bug:
    // the `Call` does not clear `counter`'s lattice cell). Whole-program set (from every `FunctionDef`).
    let killed = CalleeGlobals::of_program(&stmts);
    let mut env = Env::new();
    sccp_block(
        stmts, &mut env, &bools, &killed, expr_arena, stmt_arena, interner,
    )
}

/// Sweep a block, threading `env` forward. Each input statement contributes
/// zero or more output statements (a pruned branch splices its taken arm; a
/// `While false` vanishes).
fn sccp_block<'a>(
    block: Vec<Stmt<'a>>,
    env: &mut Env<'a>,
    bools: &fold::BoolSyms,
    killed: &CalleeGlobals,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mut out = Vec::with_capacity(block.len());
    for stmt in block {
        sccp_stmt(stmt, env, bools, killed, ea, sa, it, &mut out);
    }
    out
}

fn sccp_stmt<'a>(
    stmt: Stmt<'a>,
    env: &mut Env<'a>,
    bools: &fold::BoolSyms,
    killed: &CalleeGlobals,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    out: &mut Vec<Stmt<'a>>,
) {
    match stmt {
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => {
            let nv = rewrite(value, env, bools, ea, sa, it);
            apply_kill(env, &WriteSet::of_expr(value).with_callee_globals(killed));
            // A callee-mutated global is never a propagatable constant (a later `Call` can change it).
            env.insert(
                var,
                if killed.contains(&var) {
                    Lat::Bottom
                } else {
                    lat_of(nv)
                },
            );
            out.push(Stmt::Let {
                var,
                ty,
                value: nv,
                mutable,
            });
        }
        Stmt::Set { target, value } => {
            let nv = rewrite(value, env, bools, ea, sa, it);
            apply_kill(env, &WriteSet::of_expr(value).with_callee_globals(killed));
            env.insert(
                target,
                if killed.contains(&target) {
                    Lat::Bottom
                } else {
                    lat_of(nv)
                },
            );
            out.push(Stmt::Set { target, value: nv });
        }
        Stmt::Swap { a, b } => {
            // Precise and sound: exchange the two cells (`⊤` = remove).
            let va = env.get(&a).copied();
            let vb = env.get(&b).copied();
            set_or_top(env, a, vb);
            set_or_top(env, b, va);
            out.push(Stmt::Swap { a, b });
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            // CRITICAL: a block is a SCOPE. SCCP folds the condition and drops the
            // provably-dead arm (the no-poison win — a dead arm's `Set`s never run,
            // so they never widen the environment), but it NEVER flattens the taken
            // arm into the parent: a `Let` inside the `If` shadows only within the
            // block and reverts on exit, so hoisting it out would change semantics
            // (`Let x be 5. If true: Let x be x+1. Show x. Show x.` is `6` then `5`,
            // not `6` then `6`). The literal-conditioned `If` is left for DCE, which
            // handles it exactly as it handles a source-level literal condition.
            let fc = rewrite(cond, env, bools, ea, sa, it);
            apply_kill(env, &WriteSet::of_expr(cond).with_callee_globals(killed));
            let entry = env.clone();
            match as_bool(fc) {
                Some(true) => {
                    let mut te = entry.clone();
                    let then_out =
                        sccp_block(then_block.to_vec(), &mut te, bools, killed, ea, sa, it);
                    *env = export_scope(&entry, &te, then_block);
                    out.push(Stmt::If {
                        cond: fc,
                        then_block: sa.alloc_slice(then_out),
                        else_block: None,
                    });
                }
                Some(false) => {
                    let (else_out, new_env) = if let Some(eb) = else_block {
                        let mut fe = entry.clone();
                        let o = sccp_block(eb.to_vec(), &mut fe, bools, killed, ea, sa, it);
                        (Some(sa.alloc_slice(o)), export_scope(&entry, &fe, eb))
                    } else {
                        (None, entry.clone())
                    };
                    *env = new_env;
                    out.push(Stmt::If {
                        cond: fc,
                        then_block: sa.alloc_slice(Vec::new()),
                        else_block: else_out,
                    });
                }
                // Condition unknown: process both arms from a clone of the entry
                // env, export each arm's OUTER-scope effect (dropping block-local
                // bindings), then MEET the two.
                None => {
                    let mut te = entry.clone();
                    let then_out =
                        sccp_block(then_block.to_vec(), &mut te, bools, killed, ea, sa, it);
                    let et = export_scope(&entry, &te, then_block);
                    let (else_out, ef) = if let Some(eb) = else_block {
                        let mut fe = entry.clone();
                        let o = sccp_block(eb.to_vec(), &mut fe, bools, killed, ea, sa, it);
                        (Some(sa.alloc_slice(o)), export_scope(&entry, &fe, eb))
                    } else {
                        (None, entry.clone())
                    };
                    *env = join_arms(&entry, &et, &ef);
                    out.push(Stmt::If {
                        cond: fc,
                        then_block: sa.alloc_slice(then_out),
                        else_block: else_out,
                    });
                }
            }
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            // A loop-carried variable takes different values across iterations,
            // so every symbol the body may write is widened to `⊥` BEFORE
            // ANYTHING is folded. The condition is re-evaluated each iteration
            // over those changing values, so it too must fold under the widened
            // env — never with a loop variable's pre-loop constant, or `i < 5`
            // (i starting at 0) would fold to `while true`. Loop-INVARIANT
            // constants (symbols the body never writes) keep their entry value
            // and still fold, in both the condition and the body.
            let w = WriteSet::of_block(body);
            let condition_writes = WriteSet::of_expr(cond).with_callee_globals(killed);
            apply_kill(env, &condition_writes);
            let mut benv = env.clone();
            apply_kill(&mut benv, &w);
            let fc = rewrite(cond, &benv, bools, ea, sa, it);
            if let Some(false) = as_bool(fc) {
                // Loop-invariantly false ⇒ the body never runs; only the
                // condition's effects can change the outer environment.
                return;
            }
            let new_body = sccp_block(body.to_vec(), &mut benv, bools, killed, ea, sa, it);
            apply_kill(env, &w);
            out.push(Stmt::While {
                cond: fc,
                body: sa.alloc_slice(new_body),
                decreasing,
            });
        }
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => {
            let fi = rewrite(iterable, env, bools, ea, sa, it);
            apply_kill(
                env,
                &WriteSet::of_expr(iterable).with_callee_globals(killed),
            );
            let mut w = WriteSet::of_block(body);
            for sym in pattern_symbols(&pattern) {
                w.assign(sym);
            }
            let mut benv = env.clone();
            apply_kill(&mut benv, &w);
            let new_body = sccp_block(body.to_vec(), &mut benv, bools, killed, ea, sa, it);
            apply_kill(env, &w);
            out.push(Stmt::Repeat {
                pattern,
                iterable: fi,
                body: sa.alloc_slice(new_body),
            });
        }
        Stmt::FunctionDef {
            name,
            params,
            generics,
            body,
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
            opt_flags,
        } => {
            // A function body is its own scope with its own Bool/Text facts;
            // parameters are unknown inputs (absent = `⊤`, never folded).
            let fbools = fold::function_bool_syms(bools, name, &params, body, it);
            let mut fenv = Env::new();
            let new_body = sccp_block(body.to_vec(), &mut fenv, &fbools, killed, ea, sa, it);
            out.push(Stmt::FunctionDef {
                name,
                params,
                generics,
                body: sa.alloc_slice(new_body),
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            });
        }
        Stmt::Splice { body } => {
            // Scope-transparent (parser desugar): thread the SAME env through it.
            let new_body = sccp_block(body.to_vec(), env, bools, killed, ea, sa, it);
            out.push(Stmt::Splice {
                body: sa.alloc_slice(new_body),
            });
        }
        // Every other statement: SCCP does not rewrite its expressions
        // (`propagate`/`fold` own that). Reflect its write effect on the
        // environment and pass it through byte-identical.
        other => {
            apply_kill(env, &WriteSet::of_stmt(&other));
            out.push(other);
        }
    }
}

/// Substitute proven constants into `e` and fold. Reuses [`propagate`] so the
/// substitution refusals (never into `Index`/`Slice`/`Closure`/interpolated
/// strings) are single-sourced. A no-op when nothing in scope is `Const`, so a
/// program with no SCCP opportunity is left byte-identical.
fn rewrite<'a>(
    e: &'a Expr<'a>,
    env: &Env<'a>,
    bools: &fold::BoolSyms,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
) -> &'a Expr<'a> {
    let cview: HashMap<Symbol, &'a Expr<'a>> = env
        .iter()
        .filter_map(|(k, v)| match v {
            Lat::Const(e) => Some((*k, *e)),
            Lat::Bottom => None,
        })
        .collect();
    if cview.is_empty() {
        return e;
    }
    propagate::subst_and_fold(e, &cview, bools, ea, sa, it)
}

/// The lattice value of an already-folded value expression. Mirrors
/// [`propagate::is_propagatable_literal`] exactly.
fn lat_of<'a>(e: &'a Expr<'a>) -> Lat<'a> {
    match e {
        Expr::Literal(
            Literal::Number(_)
            | Literal::Float(_)
            | Literal::Boolean(_)
            | Literal::Nothing
            | Literal::Text(_),
        ) => Lat::Const(e),
        _ => Lat::Bottom,
    }
}

/// The boolean value of a folded condition, if it reduced to a bare literal —
/// the only case in which a branch may be pruned. A condition containing a call
/// or any non-constant term cannot reduce to a bare `Boolean` literal, so this
/// never drops a side effect.
fn as_bool(e: &Expr) -> Option<bool> {
    match e {
        Expr::Literal(Literal::Boolean(b)) => Some(*b),
        _ => None,
    }
}

/// `⊤ ∧ x = x`; `Const c ∧ Const c = Const c`; anything else ⇒ `⊥`.
fn meet<'a>(a: Option<Lat<'a>>, b: Option<Lat<'a>>) -> Option<Lat<'a>> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(Lat::Bottom), _) | (_, Some(Lat::Bottom)) => Some(Lat::Bottom),
        (Some(Lat::Const(x)), Some(Lat::Const(y))) => match (as_lit(x), as_lit(y)) {
            (Some(lx), Some(ly)) if lx == ly => Some(Lat::Const(x)),
            _ => Some(Lat::Bottom),
        },
    }
}

fn as_lit<'a>(e: &'a Expr<'a>) -> Option<&'a Literal> {
    match e {
        Expr::Literal(l) => Some(l),
        _ => None,
    }
}

/// Meet two arm-end environments over the ENTRY symbols only — a binding
/// introduced inside one arm is arm-local and must not escape the join.
fn join_arms<'a>(entry: &Env<'a>, t: &Env<'a>, f: &Env<'a>) -> Env<'a> {
    let mut out = Env::new();
    for k in entry.keys() {
        if let Some(m) = meet(t.get(k).copied(), f.get(k).copied()) {
            out.insert(*k, m);
        }
    }
    out
}

/// The environment a straight-line block exports to its ENCLOSING scope. A `Let`
/// at the block's own level shadows an outer variable only within the block, so
/// on exit that outer variable is untouched by the shadow — but a `Set` to it
/// modifies the outer binding. A name that is both `Let`-shadowed and `Set` is
/// ambiguous without ordering, so any `Let`-shadowed outer symbol is
/// conservatively widened to ⊥; every other entry symbol takes its arm-end value
/// (a `Set` persists; an untouched symbol is unchanged). Block-local `Let`s
/// (names not in the entry scope) are dropped — they are out of scope.
fn export_scope<'a>(entry: &Env<'a>, arm_end: &Env<'a>, block: &[Stmt]) -> Env<'a> {
    let mut lets = HashSet::new();
    direct_lets(block, &mut lets);
    let mut out = Env::new();
    for s in entry.keys() {
        if lets.contains(s) {
            out.insert(*s, Lat::Bottom);
        } else if let Some(v) = arm_end.get(s) {
            out.insert(*s, *v);
        }
    }
    out
}

/// Symbols `Let`-bound at a block's OWN scope level — the ones that shadow an
/// enclosing binding. A `Let` nested inside an `If`/`While`/… has its own scope
/// (handled by that block's export), so it is not collected here; a
/// scope-transparent `Splice` IS descended into (its bindings run in this scope).
fn direct_lets(block: &[Stmt], out: &mut HashSet<Symbol>) {
    for s in block {
        match s {
            Stmt::Let { var, .. } => {
                out.insert(*var);
            }
            Stmt::Splice { body } => direct_lets(body, out),
            _ => {}
        }
    }
}

fn set_or_top<'a>(env: &mut Env<'a>, k: Symbol, v: Option<Lat<'a>>) {
    match v {
        Some(l) => {
            env.insert(k, l);
        }
        None => {
            env.remove(&k);
        }
    }
}

/// Widen everything `w` may write to `⊥` (or clear the whole env when the block was opaque).
fn apply_kill<'a>(env: &mut Env<'a>, w: &WriteSet) {
    if w.is_opaque() {
        env.clear();
    } else {
        for s in w.symbols() {
            env.insert(s, Lat::Bottom);
        }
    }
}
