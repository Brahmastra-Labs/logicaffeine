//! Integer induction-variable strength reduction + index linearization.
//!
//! A counted loop that recomputes an INTEGER affine function of its induction
//! variable every iteration — `coeff*j + base`, with `j` the loop counter,
//! `coeff` a constant, and `base` loop-invariant — pays a multiply (inside
//! `base`, e.g. `i*n`) plus the add every iteration. The value is an arithmetic
//! progression, so it is maintained INCREMENTALLY in an accumulator seeded to
//! its loop-entry value and advanced by `coeff*step` each iteration. This is the
//! integer sibling of [`super::float_induction_sr`]; matmul's inner loop indexes
//! `i*n + j + 1` and `k*n + j + 1` are the motivating case (each a `1*j + base`).
//!
//! ## Soundness — no runtime guard needed
//!
//! Integer arithmetic is EXACT: for every `j` in range, the accumulator's
//! running value equals `coeff*j + base` bit-for-bit, and if the closed form
//! `coeff*j + base` would overflow `i64` at some `j`, the accumulator reaches the
//! identical value at the identical iteration (the same sequence of `+coeff`
//! steps). LOGOS integers promote to BigInt on overflow, and the accumulator uses
//! the SAME `Add` op node the source would, so it lowers through the identical
//! promotion path. Unlike the float case there is therefore no exactness window
//! and no version guard — the rewrite is unconditionally bit-identical.
//!
//! Preconditions: the loop is a unit-stride [`CountedLoop`] (so `j`'s sole update
//! is the trailing `+step`), `coeff` is a nonzero integer literal, and every
//! symbol in `base` is loop-invariant (unwritten by the body).

use std::collections::HashSet;

use super::writes::{CalleeGlobals, WriteSet};
use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};
use crate::loop_shape::{extract_counted_repeat, extract_counted_while};

/// Apply integer induction strength reduction to every qualifying counted loop.
///
/// `defer_to_tiler` (set only on the AOT/Rust-codegen call, never the run path)
/// makes the pass LEAVE the square 3-deep matmul nest the codegen tiler
/// (`peephole::try_emit_tiled_inner`) claims — linearizing its indices here
/// would inject `__isr` seed/advance statements that break the tiler's
/// exact-shape match, forfeiting L1 cache tiling (a larger win than removing the
/// index multiply). The run path keeps firing on the same nest (no tiler there),
/// so the interpreter's index arithmetic is still strength-reduced.
///
/// On the same AOT path the flag also makes the pass leave an affine expression
/// RAW in three consumer contexts where a specialized codegen pass — one LLVM's
/// own induction-variable strength reduction cannot replicate — needs the raw
/// form to fire (see [`collect_affine_targets`]): the dividend of a
/// loop-invariant-identifier `%`/`/` (fast-div), a bare affine progression pushed
/// to a collection (affine-array scalarization), and an affine array-WRITE index
/// (the BCE-hoist affine-offset guard). None of these touches matmul's sole AOT
/// site (its build-loop `(i*n+j) % 100` — a LITERAL divisor, not fast-div's).
pub fn int_induction_sr_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    defer_to_tiler: bool,
) -> (Vec<Stmt<'a>>, bool) {
    let mut changed = false;
    let globals = CalleeGlobals::of_program(&stmts);
    let out = rewrite_block(
        stmts,
        expr_arena,
        stmt_arena,
        interner,
        &mut changed,
        defer_to_tiler,
        &globals,
    );
    (out, changed)
}

fn rewrite_block<'a>(
    stmts: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    changed: &mut bool,
    defer: bool,
    globals: &CalleeGlobals,
) -> Vec<Stmt<'a>> {
    // Work over borrowed statements so the counted-`While` recognizer (which
    // spans an init + the `While`) can peek at adjacent statements.
    let refs: Vec<&Stmt<'a>> = stmts.iter().collect();
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
    let mut idx = 0;
    while idx < refs.len() {
        // Counted-`While`: `Let/Set j = start; While j </<= limit { … j = j+step }`.
        if let Some((loop_info, span)) = extract_counted_while(&refs, idx) {
            let init = refs[idx].clone();
            let while_stmt = refs[idx + 1].clone();
            // Leave a codegen-tiler-owned matmul nest pristine on the AOT path.
            if defer && is_square_triple_nest(&while_stmt) {
                out.push(init);
                out.push(while_stmt);
                idx += span;
                continue;
            }
            if let Some((seed_lets, new_while)) = try_reduce(
                loop_info.var,
                loop_info.body_without_increment,
                &while_stmt,
                ea,
                sa,
                it,
                defer,
                globals,
            ) {
                out.push(init);
                out.extend(seed_lets);
                out.push(new_while);
                *changed = true;
                idx += span;
                continue;
            }
            // No reduction here: recurse into the `While` body and emit as-is.
            out.push(init);
            out.push(recurse_stmt(
                while_stmt, ea, sa, it, changed, defer, globals,
            ));
            idx += span;
            continue;
        }
        out.push(recurse_stmt(
            refs[idx].clone(),
            ea,
            sa,
            it,
            changed,
            defer,
            globals,
        ));
        idx += 1;
    }
    out
}

/// The square 3-deep counted-loop nest the codegen tiler claims: `While a < N {
/// Let b=0; While b < N { Let c=0; While c < N {…}; c+=1 }; b+=1 }` — all three
/// bounds the SAME `N`, each outer level's body exactly `[Let =0, While]` before
/// its trailing counter increment. Matches `peephole::try_emit_tiled_inner`.
fn is_square_triple_nest(s: &Stmt) -> bool {
    fn counted<'b, 'a>(s: &'b Stmt<'a>) -> Option<(Symbol, &'a Expr<'a>, &'b [Stmt<'a>])> {
        let Stmt::While { cond, body, .. } = s else {
            return None;
        };
        let Expr::BinaryOp {
            op: BinaryOpKind::Lt,
            left,
            right,
        } = cond
        else {
            return None;
        };
        let Expr::Identifier(iv) = &**left else {
            return None;
        };
        if body.is_empty() || !is_counter_step(&body[body.len() - 1], *iv) {
            return None;
        }
        Some((*iv, right, &body[..body.len() - 1]))
    }
    let Some((_, obound, obody)) = counted(s) else {
        return false;
    };
    if obody.len() != 2 {
        return false;
    }
    if !matches!(
        &obody[0],
        Stmt::Let {
            value: Expr::Literal(Literal::Number(0)),
            ..
        }
    ) {
        return false;
    }
    let Some((_, mbound, mbody)) = counted(&obody[1]) else {
        return false;
    };
    if !expr_eq(obound, mbound) || mbody.len() != 2 {
        return false;
    }
    if !matches!(
        &mbody[0],
        Stmt::Let {
            value: Expr::Literal(Literal::Number(0)),
            ..
        }
    ) {
        return false;
    }
    match counted(&mbody[1]) {
        Some((_, ibound, _)) => expr_eq(obound, ibound),
        None => false,
    }
}

/// Recurse into a statement's nested blocks (so an inner counted loop is reached
/// even when its enclosing statement is not itself reducible).
fn recurse_stmt<'a>(
    stmt: Stmt<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    changed: &mut bool,
    defer: bool,
    globals: &CalleeGlobals,
) -> Stmt<'a> {
    // A codegen-tiler-owned matmul nest reached WITHOUT a preceding counter-init
    // (its `While a < N` is nested directly in another block) is left pristine.
    if defer && is_square_triple_nest(&stmt) {
        return stmt;
    }
    let recur = |block: Block<'a>, it: &mut Interner, changed: &mut bool| -> Block<'a> {
        let v = rewrite_block(block.to_vec(), ea, sa, it, changed, defer, globals);
        sa.alloc_slice(v)
    };
    match stmt {
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => Stmt::If {
            cond,
            then_block: recur(then_block, it, changed),
            else_block: else_block.map(|b| recur(b, it, changed)),
        },
        Stmt::While {
            cond,
            body,
            decreasing,
        } => Stmt::While {
            cond,
            body: recur(body, it, changed),
            decreasing,
        },
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => {
            // A counted `Repeat for j from a to b` is itself a strength-reduction
            // target; try it before recursing into the body.
            if let Some(cl) = extract_counted_repeat(&Stmt::Repeat {
                pattern: pattern.clone(),
                iterable,
                body,
            }) {
                let reduced_body =
                    reduce_repeat_body(cl.var, body, ea, sa, it, changed, defer, globals);
                return Stmt::Repeat {
                    pattern,
                    iterable,
                    body: reduced_body,
                };
            }
            Stmt::Repeat {
                pattern,
                iterable,
                body: recur(body, it, changed),
            }
        }
        other => other,
    }
}

/// A recognized affine-in-`j` expression `coeff*j + base` and its accumulator.
struct Reduced<'a> {
    /// The exact source expression being replaced (matched structurally).
    target: &'a Expr<'a>,
    /// The fresh accumulator symbol.
    acc: Symbol,
    /// `coeff * step` — the per-iteration advance.
    stride: i64,
}

/// Try to strength-reduce a counted-`While`'s body. Returns the seed `Let`s to
/// place before the loop and the rewritten `While`, or `None` if nothing
/// qualifies.
fn try_reduce<'a>(
    j: Symbol,
    body_without_increment: Block<'a>,
    while_stmt: &Stmt<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    defer: bool,
    globals: &CalleeGlobals,
) -> Option<(Vec<Stmt<'a>>, Stmt<'a>)> {
    let Stmt::While {
        cond,
        body,
        decreasing,
    } = while_stmt
    else {
        return None;
    };
    let step = counter_step(body, j)?;
    if step == 0 {
        return None;
    }
    // Use LICM's write set, which records collection MUTATIONS (`SetIndex`/
    // `Push`/`Pop`/`Add`/`Remove` on an identifier collection) as writing that
    // collection — so a base containing `item X of C` where the loop writes `C`
    // is correctly NON-invariant, and its accumulator is never formed.
    let written = super::licm::collect_loop_writes(body, globals);
    // Distinct affine-in-`j` targets, in first-seen order.
    let mut targets: Vec<&'a Expr<'a>> = Vec::new();
    collect_affine_targets(body_without_increment, j, &written, defer, &mut targets);
    if targets.is_empty() {
        return None;
    }
    let mut reduced: Vec<Reduced<'a>> = Vec::new();
    for (n, target) in targets.iter().enumerate() {
        let (coeff, _) = decompose(target, j, &written)?;
        let acc = it.intern(&format!("__isr_{}_{}", it.resolve(j), n));
        reduced.push(Reduced {
            target,
            acc,
            stride: coeff * step,
        });
    }
    // Seed each accumulator BEFORE the loop with the target's entry value
    // (`j` still holds its start value there): `Let mutable acc be <target>`.
    let seeds: Vec<Stmt<'a>> = reduced
        .iter()
        .map(|r| Stmt::Let {
            var: r.acc,
            ty: None,
            value: r.target,
            mutable: true,
        })
        .collect();
    // Rewrite the body: replace each target with its accumulator, then advance
    // every accumulator immediately before the counter's own increment so they
    // stay in lockstep under any control flow.
    let mut new_body: Vec<Stmt<'a>> = body
        .iter()
        .map(|s| {
            if is_counter_step(s, j) {
                s.clone()
            } else {
                replace_targets_in_stmt(s, &reduced, ea, sa)
            }
        })
        .collect();
    // Insert the advances just before the trailing counter increment.
    let advances: Vec<Stmt<'a>> = reduced
        .iter()
        .map(|r| {
            let value = ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left: ea.alloc(Expr::Identifier(r.acc)),
                right: ea.alloc(Expr::Literal(Literal::Number(r.stride))),
            });
            // Preserve the semantic origin of this exact recurrence through
            // the shared AST. A backend may consume the marker only behind a
            // guard proving every value in the progression remains a word.
            crate::optimize::facts::mark_expr(
                crate::optimize::facts::Fact::IntStrengthAdvance,
                value,
            );
            Stmt::Set {
                target: r.acc,
                value,
            }
        })
        .collect();
    let incr_pos = new_body.len().saturating_sub(1);
    for (offset, adv) in advances.into_iter().enumerate() {
        new_body.insert(incr_pos + offset, adv);
    }
    let new_while = Stmt::While {
        cond,
        body: sa.alloc_slice(new_body),
        decreasing: *decreasing,
    };
    Some((seeds, new_while))
}

/// Strength-reduce a counted-`Repeat for j from a to b` body. Because the loop
/// variable is loop-SCOPED (no outer post-loop binding) and has an implicit
/// `+1` step, we cannot seed with an outer `Let`; the closed-form `coeff*j +
/// base` inside the body is left to the closed-form/LICM passes, and we only
/// recurse to reach nested counted-`While`s. (A future extension can prepend a
/// loop-preamble accumulator once the `Repeat` lowering exposes one.)
fn reduce_repeat_body<'a>(
    _j: Symbol,
    body: Block<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    changed: &mut bool,
    defer: bool,
    globals: &CalleeGlobals,
) -> Block<'a> {
    let v = rewrite_block(body.to_vec(), ea, sa, it, changed, defer, globals);
    sa.alloc_slice(v)
}

/// The integer step of the counter's sole `Set j to j + s`, or `None`.
fn counter_step(body: &[Stmt], j: Symbol) -> Option<i64> {
    let mut step = None;
    for s in body {
        if let Stmt::Set { target, value } = s {
            if *target == j {
                let add = match value {
                    Expr::BinaryOp {
                        op: BinaryOpKind::Add,
                        left,
                        right,
                    } => match (&**left, &**right) {
                        (Expr::Identifier(a), Expr::Literal(Literal::Number(n))) if *a == j => *n,
                        (Expr::Literal(Literal::Number(n)), Expr::Identifier(a)) if *a == j => *n,
                        _ => return None,
                    },
                    _ => return None,
                };
                if step.is_some() {
                    return None;
                }
                step = Some(add);
            }
        }
    }
    step
}

fn is_counter_step(s: &Stmt, j: Symbol) -> bool {
    if let Stmt::Set {
        target,
        value:
            Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left,
                right,
            },
    } = s
    {
        if *target == j {
            return matches!((&**left, &**right),
                (Expr::Identifier(a), Expr::Literal(Literal::Number(_))) if *a == j)
                || matches!((&**left, &**right),
                    (Expr::Literal(Literal::Number(_)), Expr::Identifier(a)) if *a == j);
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Affine decomposition
// ---------------------------------------------------------------------------

/// Decompose `e` into `coeff*j + base` where `coeff` is a constant `i64` and
/// `base` does not contain `j`. Returns `None` when `j` appears non-linearly, or
/// with a non-literal coefficient, or under an op that does not distribute
/// (`Div`/`Mod`/etc.). `base` is not materialized here — only the coefficient and
/// a flag are needed; callers match the whole target expression structurally.
fn decompose(e: &Expr, j: Symbol, _written: &WriteSet) -> Option<(i64, bool)> {
    if !contains_symbol(e, j) {
        return Some((0, false)); // pure invariant term (relative to `j`)
    }
    match e {
        Expr::Identifier(s) if *s == j => Some((1, true)),
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => {
            let (cl, _) = decompose(left, j, _written)?;
            let (cr, _) = decompose(right, j, _written)?;
            Some((cl.checked_add(cr)?, true))
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left,
            right,
        } => {
            let (cl, _) = decompose(left, j, _written)?;
            let (cr, _) = decompose(right, j, _written)?;
            Some((cl.checked_sub(cr)?, true))
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            left,
            right,
        } => {
            // `A * j`-shaped only: exactly one side is a `j`-free integer LITERAL,
            // the other carries `j`. `expr(j) * expr(j)` is non-linear → bail.
            let lj = contains_symbol(left, j);
            let rj = contains_symbol(right, j);
            match (lj, rj) {
                (true, false) => {
                    let a = int_literal(right)?;
                    let (c, _) = decompose(left, j, _written)?;
                    Some((c.checked_mul(a)?, true))
                }
                (false, true) => {
                    let a = int_literal(left)?;
                    let (c, _) = decompose(right, j, _written)?;
                    Some((c.checked_mul(a)?, true))
                }
                _ => None,
            }
        }
        // Any other operator carrying `j` is not affine.
        _ => None,
    }
}

fn int_literal(e: &Expr) -> Option<i64> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        _ => None,
    }
}

/// Collect the DISTINCT affine-in-`j` subexpressions of a block worth reducing:
/// `coeff*j + base` with `coeff != 0` and every symbol in `base` loop-invariant.
/// Only the maximal affine expression is taken (we do not descend into a matched
/// target), and a trivial bare `j` is skipped (nothing to strength-reduce).
///
/// On the AOT path (`defer`) two STATEMENT-level consumer contexts keep their
/// affine RAW for a specialized codegen pass (see [`int_induction_sr_stmts`]):
///
///   * a bare affine progression PUSHED to a collection — the affine-array
///     scalarizer deletes the whole array (strictly better than removing one
///     multiply), and the prealloc/bounds-hint passes need the raw form; and
///   * the affine INDEX of an array WRITE — the BCE-hoist affine-offset planner
///     emits one preheader `assert!` per array from the raw `offset + counter`.
///
/// (The dividend of a loop-invariant-identifier `%`/`/` is the third context,
/// handled per-expression in [`collect_in_expr`].)
fn collect_affine_targets<'a>(
    block: &'a [Stmt<'a>],
    j: Symbol,
    written: &WriteSet,
    defer: bool,
    out: &mut Vec<&'a Expr<'a>>,
) {
    for s in block {
        match s {
            // The written value is often a SUBEXPRESSION of the index (`c[i*n+j+1]
            // = i*n+j`), so leaving the index raw means leaving the whole write
            // un-reduced — else replacing the value's `i*n+j` rewrites it inside
            // the index too and the BCE planner never sees the raw offset+counter.
            Stmt::Push { value, .. } if defer && is_reducible_affine(value, j, written) => {}
            Stmt::SetIndex { index, .. } if defer && is_reducible_affine(index, j, written) => {}
            _ => stmt_exprs(s, &mut |e| collect_in_expr(e, j, written, defer, out)),
        }
    }
}

fn collect_in_expr<'a>(
    e: &'a Expr<'a>,
    j: Symbol,
    written: &WriteSet,
    defer: bool,
    out: &mut Vec<&'a Expr<'a>>,
) {
    // AOT context (1): the dividend of `x % n` / `x / n` with a loop-invariant
    // IDENTIFIER divisor is fast-div's domain — kept raw, `detect_fast_div` proves
    // it non-negative (its syntactic `coeff*j + base` proof) and lowers `n` to a
    // magic multiply. A LITERAL divisor (matmul's `% 100`) is NOT fast-div's, so
    // it falls through and is strength-reduced as usual.
    if defer {
        if let Expr::BinaryOp {
            op: BinaryOpKind::Divide | BinaryOpKind::Modulo,
            right,
            ..
        } = e
        {
            if matches!(&**right, Expr::Identifier(d) if !written.may_write(*d)) {
                return;
            }
        }
    }
    if is_reducible_affine(e, j, written) {
        if !out.iter().any(|t| expr_eq(t, e)) {
            out.push(e);
        }
        return; // maximal: don't descend into the matched target
    }
    for child in expr_children(e) {
        collect_in_expr(child, j, written, defer, out);
    }
}

/// Is `e` a `coeff*j + base` with `coeff != 0`, more than the bare `j`, an
/// integer sum/product form, and `base` loop-invariant?
fn is_reducible_affine(e: &Expr, j: Symbol, written: &WriteSet) -> bool {
    // Must be a compound (`Add`/`Subtract` top) so there is real per-iteration
    // arithmetic to fold; a bare `j` or `A*j` alone has nothing to accumulate
    // beyond what codegen already does.
    if !matches!(
        e,
        Expr::BinaryOp {
            op: BinaryOpKind::Add | BinaryOpKind::Subtract,
            ..
        }
    ) {
        return false;
    }
    // The transform pays off only when recomputing `e` each iteration costs a
    // MULTIPLY — the `i*n` of matmul's `i*n + j + 1`, or a `coeff*j` with
    // `|coeff| > 1`. A pure add/subtract chain like `i + 1` recomputes for a
    // single add, so folding it into a seeded accumulator saves nothing AND
    // hides the affine index from the structural loop optimizations that must
    // still see it in raw form (rotate/prealloc/bounds-hint) — the prealloc
    // pass, for one, then mis-sizes a pushed accumulator as a collection
    // (`with_capacity(acc.len())`). Requiring a real multiply keeps the pass on
    // its motivating case (index linearization) and off the ones it only breaks.
    if !contains_multiply(e) {
        return false;
    }
    match decompose(e, j, written) {
        Some((coeff, true)) if coeff != 0 => base_invariant(e, j, written),
        _ => false,
    }
}

/// Whether `e` contains a `Multiply` node anywhere in its arithmetic subtree —
/// the per-iteration cost that strength reduction exists to remove.
fn contains_multiply(e: &Expr) -> bool {
    if matches!(
        e,
        Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            ..
        }
    ) {
        return true;
    }
    expr_children(e).into_iter().any(contains_multiply)
}

/// Every non-`j` symbol reachable in `e` must be loop-invariant (unwritten).
fn base_invariant(e: &Expr, j: Symbol, written: &WriteSet) -> bool {
    let mut syms = HashSet::new();
    collect_symbols(e, &mut syms);
    syms.iter().all(|s| *s == j || !written.may_write(*s))
}

// ---------------------------------------------------------------------------
// Rewrite helpers
// ---------------------------------------------------------------------------

fn replace_targets_in_stmt<'a>(
    s: &Stmt<'a>,
    reduced: &[Reduced<'a>],
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Stmt<'a> {
    map_stmt_exprs(s, sa, &mut |e| replace_targets_in_expr(e, reduced, ea))
}

fn replace_targets_in_expr<'a>(
    e: &'a Expr<'a>,
    reduced: &[Reduced<'a>],
    ea: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    if let Some(r) = reduced.iter().find(|r| expr_eq(r.target, e)) {
        return ea.alloc(Expr::Identifier(r.acc));
    }
    rebuild_expr(e, ea, &mut |c| replace_targets_in_expr(c, reduced, ea))
}

// ---------------------------------------------------------------------------
// Generic AST traversal (integer-expression subset)
// ---------------------------------------------------------------------------

fn contains_symbol(e: &Expr, sym: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s == sym,
        _ => expr_children(e)
            .into_iter()
            .any(|c| contains_symbol(c, sym)),
    }
}

fn collect_symbols(e: &Expr, out: &mut HashSet<Symbol>) {
    if let Expr::Identifier(s) = e {
        out.insert(*s);
    }
    for c in expr_children(e) {
        collect_symbols(c, out);
    }
}

/// Structural equality over the arithmetic/index expression subset (arena
/// pointers differ across occurrences, so compare by value).
fn expr_eq(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Literal(x), Expr::Literal(y)) => x == y,
        (Expr::Identifier(x), Expr::Identifier(y)) => x == y,
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
        ) => o1 == o2 && expr_eq(l1, l2) && expr_eq(r1, r2),
        (Expr::Not { operand: x }, Expr::Not { operand: y }) => expr_eq(x, y),
        (
            Expr::Index {
                collection: c1,
                index: i1,
            },
            Expr::Index {
                collection: c2,
                index: i2,
            },
        ) => expr_eq(c1, c2) && expr_eq(i1, i2),
        (Expr::Length { collection: x }, Expr::Length { collection: y }) => expr_eq(x, y),
        (Expr::OptionSome { value: x }, Expr::OptionSome { value: y }) => expr_eq(x, y),
        _ => false,
    }
}

/// The immediate sub-expressions of `e` (arithmetic + index subset; other node
/// kinds have no arithmetic children we traverse for affine analysis).
fn expr_children<'a>(e: &'a Expr<'a>) -> Vec<&'a Expr<'a>> {
    match e {
        Expr::BinaryOp { left, right, .. } => vec![left, right],
        Expr::Not { operand } => vec![operand],
        Expr::Index { collection, index } => vec![collection, index],
        Expr::Slice {
            collection,
            start,
            end,
        } => vec![collection, start, end],
        Expr::Length { collection } => vec![collection],
        Expr::Contains { collection, value } => vec![collection, value],
        Expr::OptionSome { value } => vec![value],
        Expr::Copy { expr } => vec![expr],
        Expr::Give { value } => vec![value],
        Expr::Range { start, end } => vec![start, end],
        _ => Vec::new(),
    }
}

/// Rebuild `e` with `f` applied to each child (arithmetic + index subset),
/// preserving other node kinds unchanged.
fn rebuild_expr<'a>(
    e: &'a Expr<'a>,
    ea: &'a Arena<Expr<'a>>,
    f: &mut dyn FnMut(&'a Expr<'a>) -> &'a Expr<'a>,
) -> &'a Expr<'a> {
    match e {
        Expr::BinaryOp { op, left, right } => {
            let l = f(left);
            let r = f(right);
            if std::ptr::eq(l, *left) && std::ptr::eq(r, *right) {
                e
            } else {
                ea.alloc(Expr::BinaryOp {
                    op: *op,
                    left: l,
                    right: r,
                })
            }
        }
        Expr::Not { operand } => {
            let o = f(operand);
            if std::ptr::eq(o, *operand) {
                e
            } else {
                ea.alloc(Expr::Not { operand: o })
            }
        }
        Expr::Index { collection, index } => {
            let c = f(collection);
            let i = f(index);
            if std::ptr::eq(c, *collection) && std::ptr::eq(i, *index) {
                e
            } else {
                ea.alloc(Expr::Index {
                    collection: c,
                    index: i,
                })
            }
        }
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            let c = f(collection);
            let s = f(start);
            let en = f(end);
            if std::ptr::eq(c, *collection) && std::ptr::eq(s, *start) && std::ptr::eq(en, *end) {
                e
            } else {
                ea.alloc(Expr::Slice {
                    collection: c,
                    start: s,
                    end: en,
                })
            }
        }
        Expr::Length { collection } => {
            let c = f(collection);
            if std::ptr::eq(c, *collection) {
                e
            } else {
                ea.alloc(Expr::Length { collection: c })
            }
        }
        Expr::Contains { collection, value } => {
            let c = f(collection);
            let v = f(value);
            if std::ptr::eq(c, *collection) && std::ptr::eq(v, *value) {
                e
            } else {
                ea.alloc(Expr::Contains {
                    collection: c,
                    value: v,
                })
            }
        }
        Expr::OptionSome { value } => {
            let v = f(value);
            if std::ptr::eq(v, *value) {
                e
            } else {
                ea.alloc(Expr::OptionSome { value: v })
            }
        }
        Expr::Copy { expr } => {
            let v = f(expr);
            if std::ptr::eq(v, *expr) {
                e
            } else {
                ea.alloc(Expr::Copy { expr: v })
            }
        }
        Expr::Give { value } => {
            let v = f(value);
            if std::ptr::eq(v, *value) {
                e
            } else {
                ea.alloc(Expr::Give { value: v })
            }
        }
        Expr::Range { start, end } => {
            let s = f(start);
            let en = f(end);
            if std::ptr::eq(s, *start) && std::ptr::eq(en, *end) {
                e
            } else {
                ea.alloc(Expr::Range { start: s, end: en })
            }
        }
        _ => e,
    }
}

/// Visit each top-level value expression of a statement (read positions).
fn stmt_exprs<'a>(s: &'a Stmt<'a>, f: &mut dyn FnMut(&'a Expr<'a>)) {
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => f(value),
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            f(collection);
            f(index);
            f(value);
        }
        Stmt::SetField { object, value, .. } => {
            f(object);
            f(value);
        }
        Stmt::Return { value: Some(v) } => f(v),
        Stmt::Show { object, .. } | Stmt::Push { value: object, .. } => f(object),
        Stmt::Call { args, .. } => {
            for a in args {
                f(a);
            }
        }
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => f(cond),
        _ => {}
    }
}

/// Rebuild a statement with `f` applied to each of its value expressions.
fn map_stmt_exprs<'a>(
    s: &Stmt<'a>,
    _sa: &'a Arena<Stmt<'a>>,
    f: &mut dyn FnMut(&'a Expr<'a>) -> &'a Expr<'a>,
) -> Stmt<'a> {
    match s {
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => Stmt::Let {
            var: *var,
            ty: *ty,
            value: f(value),
            mutable: *mutable,
        },
        Stmt::Set { target, value } => Stmt::Set {
            target: *target,
            value: f(value),
        },
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => Stmt::SetIndex {
            collection: f(collection),
            index: f(index),
            value: f(value),
        },
        Stmt::SetField {
            object,
            field,
            value,
        } => Stmt::SetField {
            object: f(object),
            field: *field,
            value: f(value),
        },
        Stmt::Return { value } => Stmt::Return {
            value: value.map(|v| f(v)),
        },
        Stmt::Show { object, recipient } => Stmt::Show {
            object: f(object),
            recipient: *recipient,
        },
        Stmt::Push { value, collection } => Stmt::Push {
            value: f(value),
            collection: *collection,
        },
        Stmt::Call { function, args } => Stmt::Call {
            function: *function,
            args: args.iter().map(|a| f(a)).collect(),
        },
        other => other.clone(),
    }
}

// Base-invariance uses `licm::collect_loop_writes` (see `try_reduce`), which
// records collection MUTATIONS as writing the collection — so a scalar-only
// write set here would be UNSOUND (it would treat a mutated `item X of C` base
// as invariant). No local write-set helper is kept, to prevent that mistake.
