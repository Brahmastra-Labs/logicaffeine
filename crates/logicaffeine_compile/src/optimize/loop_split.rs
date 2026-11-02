//! Guard-based loop index-set splitting.
//!
//! Splits a counted loop whose body is gated by a single affine-monotone guard
//! on the induction variable (IV) against a loop-invariant threshold into two
//! consecutive loops at the threshold, so each runs with the branch pre-resolved
//! to a constant. The guard-true ("suffix") loop becomes branch-free with an
//! UNCONDITIONAL contiguous load, which LLVM autovectorizes — the lever that
//! lets compiled-LOGOS knapsack match/beat Zig's masked-load vectorization while
//! the prefix collapses to a memcpy.
//!
//! The canonical subject is the 0/1-knapsack inner DP loop:
//! ```text
//! While w is at most capacity:
//!     Let best be item (w + 1) of prev.
//!     If w is at least wi:                       # guard: w >= wi  (wi invariant)
//!         Let take be item (w - wi + 1) of prev + vi.
//!         If take is greater than best: Set best to take.
//!     Push best to curr.
//!     Set w to w + 1.
//! ```
//! becomes (versioned — the `If` keeps the original loop for an out-of-range `wi`,
//! so the split path's single-comparison sub-loops are unconditionally correct):
//! ```text
//! Let w be 0.                                    # original init (declares w)
//! If (0 <= wi) and (wi <= capacity + 1):
//!     Set w to 0.
//!     While w < wi:                              # PREFIX — then-block dropped → memcpy
//!         Let best be item (w + 1) of prev.
//!         Push best to curr.
//!         Set w to w + 1.
//!     Set w to wi.
//!     While w is at most capacity:              # SUFFIX — then-block inlined, branch-free
//!         Let best be item (w + 1) of prev.
//!         Let take be item (w - wi + 1) of prev + vi.
//!         If take is greater than best: Set best to take.
//!         Push best to curr.
//!         Set w to w + 1.
//! Otherwise:
//!     Set w to 0.
//!     While w is at most capacity: <original body>
//! ```
//!
//! SOUNDNESS. The split is semantics-preserving when (1) the IV is monotone
//! unit-stride (`Set iv to iv + 1` as its sole write and the body's last
//! statement) and (2) the threshold is loop-invariant. The version guard
//! `start <= split <= excl_end` restricts the literal-`split` sub-loops to the
//! case where the threshold is interior; any other `wi` runs the unchanged
//! original loop (identical iterations, identical order, identical panic
//! behaviour on a degenerate index). The suffix's IV initialised to the literal
//! threshold lets the affine bounds oracle re-prove the now-unconditional
//! `prev[w - wi]` access in range (its monotone lower bound is exactly `wi`), so
//! codegen keeps emitting the unchecked load the vectoriser needs.
//!
//! This is an AOT-only transform: the vectorisation payoff exists only on the
//! Rust-emitting path, so it is wired into `optimize_program` and NOT into the
//! interpreter/Futamura pipelines. `LOGOS_LOOP_SPLIT=0` disables it.

use super::abstract_interp::self_increment;
use super::licm::{collect_loop_writes, is_loop_invariant};
use super::writes::count_writes_of;
use super::writes::CalleeGlobals;
use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::codegen::peephole::is_simple_expr;
use crate::intern::{Interner, Symbol};
use crate::optimization::{Opt, OptimizationConfig};

/// A recognized IV guard: the iteration space splits at `split_point`, with the
/// guard's then-block belonging to the high (suffix) range when `then_is_suffix`.
struct Guard<'a> {
    /// The invariant RHS of the comparison — checked for loop-invariance.
    threshold: &'a Expr<'a>,
    /// The value the IV crosses; the suffix loop starts here (`>=` → threshold,
    /// `>` → threshold + 1).
    split_point: &'a Expr<'a>,
    then_is_suffix: bool,
}

/// Process a block, splitting each qualifying `init; While` pair in place.
pub fn loop_split_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    cfg: &OptimizationConfig,
) -> Vec<Stmt<'a>> {
    if !cfg.is_on(Opt::LoopSplit) {
        return stmts;
    }
    let globals = CalleeGlobals::of_program(&stmts);
    split_block(stmts, expr_arena, stmt_arena, interner, &globals)
}

fn split_block<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    globals: &CalleeGlobals,
) -> Vec<Stmt<'a>> {
    let mut result: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
    let mut i = 0;
    while i < stmts.len() {
        // An `init; While` pair is the only splittable shape — mirror the
        // counted-loop recognizer's adjacency requirement exactly.
        if i + 1 < stmts.len() {
            if let (init @ (Stmt::Let { .. } | Stmt::Set { .. }), w @ Stmt::While { .. }) =
                (&stmts[i], &stmts[i + 1])
            {
                if let Some(split) =
                    try_split_pair(init, w, expr_arena, stmt_arena, interner, globals)
                {
                    result.extend(split);
                    i += 2;
                    continue;
                }
            }
        }
        result.push(recurse_into(
            stmts[i].clone(),
            expr_arena,
            stmt_arena,
            interner,
            globals,
        ));
        i += 1;
    }
    result
}

/// Recurse into a statement's child blocks (so nested loops still split) without
/// attempting a split at this node — the pair scan in `split_block` owns that.
fn recurse_into<'a>(
    stmt: Stmt<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    globals: &CalleeGlobals,
) -> Stmt<'a> {
    match stmt {
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            let nb = split_block(body.to_vec(), expr_arena, stmt_arena, interner, globals);
            Stmt::While {
                cond,
                body: stmt_arena.alloc_slice(nb),
                decreasing,
            }
        }
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => {
            let nb = split_block(body.to_vec(), expr_arena, stmt_arena, interner, globals);
            Stmt::Repeat {
                pattern,
                iterable,
                body: stmt_arena.alloc_slice(nb),
            }
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            let nt = split_block(
                then_block.to_vec(),
                expr_arena,
                stmt_arena,
                interner,
                globals,
            );
            let ne = else_block.map(|eb| {
                let b: Block = stmt_arena.alloc_slice(split_block(
                    eb.to_vec(),
                    expr_arena,
                    stmt_arena,
                    interner,
                    globals,
                ));
                b
            });
            Stmt::If {
                cond,
                then_block: stmt_arena.alloc_slice(nt),
                else_block: ne,
            }
        }
        Stmt::Zone {
            name,
            capacity,
            source_file,
            body,
        } => {
            let nb = split_block(body.to_vec(), expr_arena, stmt_arena, interner, globals);
            Stmt::Zone {
                name,
                capacity,
                source_file,
                body: stmt_arena.alloc_slice(nb),
            }
        }
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
        } => {
            let nb = split_block(body.to_vec(), expr_arena, stmt_arena, interner, globals);
            Stmt::FunctionDef {
                name,
                generics,
                params,
                body: stmt_arena.alloc_slice(nb),
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            }
        }
        other => other,
    }
}

fn try_split_pair<'a>(
    init: &Stmt<'a>,
    while_stmt: &Stmt<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    globals: &CalleeGlobals,
) -> Option<Vec<Stmt<'a>>> {
    // --- the loop ---
    let (while_cond, body, decreasing) = match while_stmt {
        Stmt::While {
            cond,
            body,
            decreasing,
        } => (*cond, *body, *decreasing),
        _ => return None,
    };
    // MVP: a termination variant would have to be re-derived per sub-loop.
    if decreasing.is_some() || body.is_empty() {
        return None;
    }

    // --- the init: `Let/Set iv = start`, start a simple expression ---
    let (iv, start) = match init {
        Stmt::Let { var, value, .. } => (*var, *value),
        Stmt::Set { target, value } => (*target, *value),
        _ => return None,
    };
    if !is_simple_expr(start) {
        return None;
    }

    // --- the while condition: `iv <op> U`, op in {<=, <} ---
    let (cond_iv, upper, inclusive) = match while_cond {
        Expr::BinaryOp {
            op: BinaryOpKind::LtEq,
            left,
            right,
        } => match left {
            Expr::Identifier(s) => (*s, *right, true),
            _ => return None,
        },
        Expr::BinaryOp {
            op: BinaryOpKind::Lt,
            left,
            right,
        } => match left {
            Expr::Identifier(s) => (*s, *right, false),
            _ => return None,
        },
        _ => return None,
    };
    if cond_iv != iv {
        return None;
    }

    // --- unit-stride induction: the trailing `Set iv to iv + 1` is iv's only write ---
    match body.last()? {
        Stmt::Set { target, value } if *target == iv => {
            if self_increment(iv, value) != Some(1) {
                return None;
            }
        }
        _ => return None,
    }
    if count_writes_of(body, iv, globals) != 1 {
        return None;
    }

    // --- no control flow that escapes the iteration space ---
    if contains_break_or_return(body) {
        return None;
    }

    // --- the guard: first top-level `If` recognized as an affine IV guard ---
    let if_idx = body.iter().position(|s| match s {
        Stmt::If { cond, .. } => normalize_guard(cond, iv, expr_arena).is_some(),
        _ => false,
    })?;
    // The guard must precede the trailing increment (which is the last statement).
    if if_idx >= body.len() - 1 {
        return None;
    }
    let (guard_cond, then_block, else_block) = match &body[if_idx] {
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => (*cond, *then_block, *else_block),
        _ => return None,
    };
    let g = normalize_guard(guard_cond, iv, expr_arena)?;

    // --- the threshold must be loop-invariant ---
    let writes = collect_loop_writes(body, globals);
    if !is_loop_invariant(g.threshold, &writes) {
        return None;
    }

    // --- payoff gate: the suffix carries an IV-dependent indexed read (the load
    //     whose conditionality blocks vectorisation today). Without one, the
    //     split only bloats code. ---
    let (prefix_src, suffix_src): (Block, Block) = if g.then_is_suffix {
        (else_block.unwrap_or(&[]), then_block)
    } else {
        (then_block, else_block.unwrap_or(&[]))
    };
    if !block_has_guarded_backref_read(suffix_src, iv) {
        return None;
    }

    // --- build the three loop bodies ---
    let before = &body[..if_idx];
    let after = &body[if_idx + 1..]; // includes the trailing increment

    let compose = |src: Block<'a>| -> Vec<Stmt<'a>> {
        let mut v: Vec<Stmt<'a>> = before.iter().cloned().collect();
        v.extend(src.iter().cloned());
        v.extend(after.iter().cloned());
        v
    };
    // Recurse so a deeper guard inside either branch splits too.
    let prefix_body = split_block(
        compose(prefix_src),
        expr_arena,
        stmt_arena,
        interner,
        globals,
    );
    let suffix_body = split_block(
        compose(suffix_src),
        expr_arena,
        stmt_arena,
        interner,
        globals,
    );
    let prefix_body: Block = stmt_arena.alloc_slice(prefix_body);
    let suffix_body: Block = stmt_arena.alloc_slice(suffix_body);

    // --- expressions: excl_end (the loop's exclusive end) and the version guard ---
    let one = expr_arena.alloc(Expr::Literal(Literal::Number(1)));
    let excl_end: &Expr = if inclusive {
        expr_arena.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left: upper,
            right: one,
        })
    } else {
        upper
    };
    // version: start <= split  &&  split <= excl_end  (the threshold is interior)
    let lo_ok = expr_arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::LtEq,
        left: start,
        right: g.split_point,
    });
    let hi_ok = expr_arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::LtEq,
        left: g.split_point,
        right: excl_end,
    });
    let version_cond = expr_arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::And,
        left: lo_ok,
        right: hi_ok,
    });

    // prefix condition: `iv < split_point` (exclusive; sound because the version
    // guard pins split_point <= excl_end, so iv < split_point ⇒ iv in range).
    let prefix_cond = expr_arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Lt,
        left: expr_arena.alloc(Expr::Identifier(iv)),
        right: g.split_point,
    });

    let set_iv = |val: &'a Expr<'a>| Stmt::Set {
        target: iv,
        value: val,
    };

    // version-true branch: re-init iv, run prefix then suffix
    let mut then_branch: Vec<Stmt<'a>> = Vec::with_capacity(4);
    then_branch.push(set_iv(start));
    then_branch.push(Stmt::While {
        cond: prefix_cond,
        body: prefix_body,
        decreasing: None,
    });
    then_branch.push(set_iv(g.split_point));
    then_branch.push(Stmt::While {
        cond: while_cond,
        body: suffix_body,
        decreasing: None,
    });

    // version-false branch: the original loop, unchanged
    let else_branch: Vec<Stmt<'a>> = vec![
        set_iv(start),
        Stmt::While {
            cond: while_cond,
            body,
            decreasing: None,
        },
    ];

    let version_if = Stmt::If {
        cond: version_cond,
        then_block: stmt_arena.alloc_slice(then_branch),
        else_block: Some(stmt_arena.alloc_slice(else_branch)),
    };

    // The original init runs first (declares iv, keeps it in scope after the
    // loop); each branch re-inits iv so the recognizer sees `init; While`.
    Some(vec![init.clone(), version_if])
}

/// Recognize `iv >= T`, `iv > T`, `T <= iv`, `T < iv` (T the invariant
/// threshold). The then-block is the guard-TRUE branch, which is the high
/// (suffix) range for all four forms.
fn normalize_guard<'a>(
    cond: &'a Expr<'a>,
    iv: Symbol,
    expr_arena: &'a Arena<Expr<'a>>,
) -> Option<Guard<'a>> {
    let is_iv = |e: &Expr| matches!(e, Expr::Identifier(s) if *s == iv);
    let plus_one = |e: &'a Expr<'a>| -> &'a Expr<'a> {
        expr_arena.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left: e,
            right: expr_arena.alloc(Expr::Literal(Literal::Number(1))),
        })
    };
    match cond {
        Expr::BinaryOp { op, left, right } => {
            // iv on the left
            if is_iv(left) && !mentions(right, iv) {
                return match op {
                    BinaryOpKind::GtEq => Some(Guard {
                        threshold: right,
                        split_point: right,
                        then_is_suffix: true,
                    }),
                    BinaryOpKind::Gt => Some(Guard {
                        threshold: right,
                        split_point: plus_one(right),
                        then_is_suffix: true,
                    }),
                    _ => None,
                };
            }
            // iv on the right (`T <= iv`, `T < iv`)
            if is_iv(right) && !mentions(left, iv) {
                return match op {
                    BinaryOpKind::LtEq => Some(Guard {
                        threshold: left,
                        split_point: left,
                        then_is_suffix: true,
                    }),
                    BinaryOpKind::Lt => Some(Guard {
                        threshold: left,
                        split_point: plus_one(left),
                        then_is_suffix: true,
                    }),
                    _ => None,
                };
            }
            None
        }
        _ => None,
    }
}

/// True if `expr` references `sym` anywhere.
fn mentions(expr: &Expr, sym: Symbol) -> bool {
    match expr {
        Expr::Identifier(s) => *s == sym,
        Expr::Literal(_) => false,
        Expr::BinaryOp { left, right, .. }
        | Expr::Contains {
            collection: left,
            value: right,
        }
        | Expr::Index {
            collection: left,
            index: right,
        }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right } => mentions(left, sym) || mentions(right, sym),
        Expr::Not { operand }
        | Expr::Length {
            collection: operand,
        }
        | Expr::Copy { expr: operand }
        | Expr::Give { value: operand } => mentions(operand, sym),
        Expr::Slice {
            collection,
            start,
            end,
        } => mentions(collection, sym) || mentions(start, sym) || mentions(end, sym),
        Expr::Call { args, .. } => args.iter().any(|a| mentions(a, sym)),
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|a| mentions(a, sym)),
        _ => true, // unknown shape → conservatively assume it might mention sym
    }
}

/// True if `e` contains a subtraction whose left side references the IV — the
/// `arr[iv - k]` backward-stencil index whose validity the guard `iv >= k`
/// protects. This is the precise vectorization-payoff signal: only such a
/// guarded BACKWARD load is unspeculatable today, so only removing its guard
/// (via the split) unblocks the vectorizer. A forward load `arr[iv + k]` is
/// already safe, so its guard isn't load-protecting and the split would not pay.
fn index_has_iv_backref(e: &Expr, iv: Symbol) -> bool {
    match e {
        Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left,
            right,
        } => {
            mentions(left, iv) || index_has_iv_backref(left, iv) || index_has_iv_backref(right, iv)
        }
        Expr::BinaryOp { left, right, .. } => {
            index_has_iv_backref(left, iv) || index_has_iv_backref(right, iv)
        }
        _ => false,
    }
}

/// True if the block contains an `item ... of arr` (`Expr::Index`) whose index
/// is a guarded IV backward reference — the vectorization-payoff signal.
fn block_has_guarded_backref_read(stmts: &[Stmt], iv: Symbol) -> bool {
    fn in_expr(e: &Expr, iv: Symbol) -> bool {
        match e {
            Expr::Index { collection, index } => {
                index_has_iv_backref(index, iv) || in_expr(collection, iv) || in_expr(index, iv)
            }
            Expr::BinaryOp { left, right, .. }
            | Expr::Contains {
                collection: left,
                value: right,
            }
            | Expr::Union { left, right }
            | Expr::Intersection { left, right } => in_expr(left, iv) || in_expr(right, iv),
            Expr::Not { operand }
            | Expr::Length {
                collection: operand,
            }
            | Expr::Copy { expr: operand }
            | Expr::Give { value: operand } => in_expr(operand, iv),
            Expr::Slice {
                collection,
                start,
                end,
            } => in_expr(collection, iv) || in_expr(start, iv) || in_expr(end, iv),
            Expr::Call { args, .. } => args.iter().any(|a| in_expr(a, iv)),
            Expr::List(items) | Expr::Tuple(items) => items.iter().any(|a| in_expr(a, iv)),
            _ => false,
        }
    }
    fn in_stmt(s: &Stmt, iv: Symbol) -> bool {
        let mut found = false;
        match s {
            Stmt::Let { value, .. } => found |= in_expr(value, iv),
            Stmt::Set { value, .. } => found |= in_expr(value, iv),
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => {
                found |= in_expr(collection, iv) || in_expr(index, iv) || in_expr(value, iv);
            }
            Stmt::Push { value, collection } => {
                found |= in_expr(value, iv) || in_expr(collection, iv);
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                found |= in_expr(cond, iv) || block_has_guarded_backref_read(then_block, iv);
                if let Some(eb) = else_block {
                    found |= block_has_guarded_backref_read(eb, iv);
                }
            }
            Stmt::While { cond, body, .. } => {
                found |= in_expr(cond, iv) || block_has_guarded_backref_read(body, iv);
            }
            Stmt::Repeat { body, .. } => found |= block_has_guarded_backref_read(body, iv),
            _ => {}
        }
        found
    }
    stmts.iter().any(|s| in_stmt(s, iv))
}

/// True if the block contains a `Break` or `Return` at any depth.
fn contains_break_or_return(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Break | Stmt::Return { .. } => true,
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            contains_break_or_return(then_block)
                || else_block.is_some_and(|eb| contains_break_or_return(eb))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
            contains_break_or_return(body)
        }
        _ => false,
    })
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/loop_split/tests.rs"]
mod tests;
