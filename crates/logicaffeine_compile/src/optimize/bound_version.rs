//! O9 — bound-versioned loop nests.
//!
//! The spectral_norm shape: a counted loop nest (`i`, `j` ∈ `[0, n)`) whose
//! hot body evaluates an Int chain of the nest's IVs — directly, or through a
//! tiny pure helper (`aVal(i, j) = 1/((i+j)(i+j+1)/2 + i + 1)`). Exact-Int
//! semantics makes the chain checked (its leaves are unbounded i64 in
//! general), and ANY per-iteration guard or `LogosInt` round-trip blocks the
//! loop vectorization that the raw chain gets for free.
//!
//! The proof, however, is ONE comparison on the loop-invariant bound: with
//! `n <= B` every IV is in `[0, B]`, and `B` is solved at compile time so the
//! whole chain provably fits i64. So: version the NEST, not the operation —
//!
//! ```text
//! If n <= B:
//!     <fast clone: helper calls inlined, chains registered proven-raw>
//! Otherwise:
//!     <the original loop, untouched — exact semantics bit for bit>
//! ```
//!
//! Fail-closed rules (each is load-bearing):
//! - the nest is `init; While` counted loops: IV starts at a literal `>= 0`,
//!   `iv < bound` / `iv <= bound` with the SAME loop-invariant bound
//!   expression per level, sole IV write is the trailing `iv = iv + 1`;
//! - a chain qualifies only when every leaf is a literal or a nest IV, and
//!   every `/`/`%` divisor is a NONZERO literal (raw lowering keeps the
//!   canonical zero-divisor error trivially: there is none);
//! - a helper call folds into the fast clone only when every argument is a
//!   literal or nest IV and the helper body is exactly `Return <expr>` over
//!   the pure fragment (literals/identifiers/binops/not) — `inline_tiny`'s
//!   candidate rule, applied to the CLONE only, so the program's global
//!   codegen surface is unchanged;
//! - the fast clone is a DEEP COPY over a closed statement/expression
//!   fragment; any construct outside it refuses the whole versioning.
//!
//! The fast clone's qualifying chain nodes are registered in the proven-raw
//! registry (the guard is their proof); the interpreter evaluates the same
//! transformed AST and takes identical branches, so every tier agrees.

use std::collections::HashMap;

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::Symbol;

/// Minimum worthwhile leaf bound: below `2^16` the fast region is too small
/// to matter and the guard would almost always fail.
const MIN_BOUND: u128 = 1 << 16;

pub fn bound_version_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    // Tiny pure helpers (`fn f(..) = Return <pure expr>`) visible to fast
    // clones, program-wide. Param SYMBOLS are owned (the body expr is
    // arena-owned) so the map never borrows the statement vector.
    let mut helpers: Helpers = HashMap::new();
    for s in &stmts {
        if let Stmt::FunctionDef { name, params, body, is_native: false, .. } = s {
            if let [Stmt::Return { value: Some(e) }] = body {
                if pure_fragment(e) {
                    helpers.insert(*name, (params.iter().map(|(p, _)| *p).collect(), e));
                }
            }
        }
    }
    version_block(stmts, &helpers, ea, sa)
}

type Helpers<'a> = HashMap<Symbol, (Vec<Symbol>, &'a Expr<'a>)>;

fn version_block<'a>(
    stmts: Vec<Stmt<'a>>,
    helpers: &Helpers<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    let mut result: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
    let mut i = 0;
    while i < stmts.len() {
        if i + 1 < stmts.len() {
            if let (init @ (Stmt::Let { .. } | Stmt::Set { .. }), Stmt::While { .. }) =
                (&stmts[i], &stmts[i + 1])
            {
                if let Some(versioned) =
                    try_version_nest(init, &stmts[i + 1], helpers, ea, sa)
                {
                    result.push(versioned);
                    i += 2;
                    continue;
                }
            }
        }
        // Recurse into structural statements so nests inside functions and
        // branches are seen.
        let s = match stmts[i].clone() {
            Stmt::FunctionDef {
                name, generics, params, body, return_type,
                is_native, native_path, is_exported, export_target, opt_flags,
            } => Stmt::FunctionDef {
                name, generics, params,
                body: sa.alloc_slice(version_block(body.to_vec(), helpers, ea, sa)),
                return_type, is_native, native_path, is_exported, export_target, opt_flags,
            },
            Stmt::If { cond, then_block, else_block } => Stmt::If {
                cond,
                then_block: sa.alloc_slice(version_block(then_block.to_vec(), helpers, ea, sa)),
                else_block: else_block
                    .map(|eb| -> Block<'a> { sa.alloc_slice(version_block(eb.to_vec(), helpers, ea, sa)) }),
            },
            Stmt::While { cond, body, decreasing } => Stmt::While {
                cond,
                body: sa.alloc_slice(version_block(body.to_vec(), helpers, ea, sa)),
                decreasing,
            },
            other => other,
        };
        result.push(s);
        i += 1;
    }
    result
}

/// One counted level of the nest.
struct Level<'a> {
    iv: Symbol,
    bound: &'a Expr<'a>,
}

fn counted_level<'a>(init: &Stmt<'a>, w: &Stmt<'a>) -> Option<Level<'a>> {
    let (iv, start) = match init {
        Stmt::Let { var, value, .. } => (*var, *value),
        Stmt::Set { target, value } => (*target, *value),
        _ => return None,
    };
    if !matches!(start, Expr::Literal(Literal::Number(n)) if *n >= 0) {
        return None;
    }
    let Stmt::While { cond, body, .. } = w else { return None };
    let Expr::BinaryOp { op: BinaryOpKind::Lt | BinaryOpKind::LtEq, left, right } = cond else {
        return None;
    };
    if !matches!(left, Expr::Identifier(s) if *s == iv) {
        return None;
    }
    // Sole IV write: the trailing `Set iv to iv + 1`.
    let Some(Stmt::Set { target, value }) = body.last() else { return None };
    if *target != iv {
        return None;
    }
    let Expr::BinaryOp { op: BinaryOpKind::Add, left: il, right: ir } = value else { return None };
    if !matches!(il, Expr::Identifier(s) if *s == iv)
        || !matches!(ir, Expr::Literal(Literal::Number(1)))
    {
        return None;
    }
    if count_writes(body, iv) != 1 {
        return None;
    }
    // The bound must be a plain invariant identifier (never written in the
    // body) or a literal — the guard reuses it verbatim.
    match right {
        Expr::Identifier(b) => {
            if count_writes(body, *b) != 0 {
                return None;
            }
        }
        Expr::Literal(Literal::Number(_)) => {}
        _ => return None,
    }
    Some(Level { iv, bound: right })
}

fn count_writes(stmts: &[Stmt], sym: Symbol) -> usize {
    let mut n = 0;
    for s in stmts {
        match s {
            Stmt::Set { target, .. } if *target == sym => n += 1,
            Stmt::Let { var, .. } if *var == sym => n += 1,
            Stmt::If { then_block, else_block, .. } => {
                n += count_writes(then_block, sym);
                if let Some(eb) = else_block {
                    n += count_writes(eb, sym);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => n += count_writes(body, sym),
            _ => {}
        }
    }
    n
}

fn try_version_nest<'a>(
    init: &Stmt<'a>,
    w: &Stmt<'a>,
    helpers: &Helpers<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Option<Stmt<'a>> {
    let outer = counted_level(init, w)?;
    let Stmt::While { body, .. } = w else { return None };

    // Collect the nest's IVs and bounds: this level plus every directly
    // nested counted level (any depth).
    let mut ivs = vec![outer.iv];
    let mut bounds: Vec<&Expr> = vec![outer.bound];
    collect_nested_levels(body, &mut ivs, &mut bounds);

    // Find qualifying chains (direct + through helper calls) and solve the
    // shared leaf bound B.
    let mut chains: Vec<&Expr> = Vec::new();
    collect_chains_block(body, &ivs, helpers, ea, &mut chains);
    if chains.is_empty() {
        return None;
    }
    // Only worth versioning when some chain would otherwise be CHECKED: its
    // magnitude with unbounded i64 leaves escapes i64.
    if !chains.iter().any(|c| {
        chain_magnitude(c, &ivs, 1u128 << 63).is_none_or(|m| m > i64::MAX as u128)
    }) {
        return None;
    }
    let mut bound_b: u128 = 0;
    for exp in (16..=62).rev() {
        let b = 1u128 << exp;
        if chains.iter().all(|c| {
            chain_magnitude(c, &ivs, b).is_some_and(|m| m <= i64::MAX as u128)
        }) {
            bound_b = b;
            break;
        }
    }
    if bound_b < MIN_BOUND {
        return None;
    }

    // Deep-copy init + loop into the fast clone, inlining qualifying helper
    // calls; refuse on anything outside the closed fragment.
    let fast_init = copy_stmt(init, &ivs, helpers, ea, sa)?;
    let fast_loop = copy_stmt(w, &ivs, helpers, ea, sa)?;
    // Register every qualifying chain node in the CLONE as proven-raw.
    mark_chains_stmt(&fast_loop, &ivs, ea);

    // Guard: every distinct bound expression `V <= B` (a literal bound folds
    // at compile time via the same comparison).
    let b_lit = ea.alloc(Expr::Literal(Literal::Number(bound_b as i64)));
    let mut guard: Option<&Expr> = None;
    let mut seen: Vec<&Expr> = Vec::new();
    for v in bounds {
        if seen.iter().any(|s| bounds_equal(s, v)) {
            continue;
        }
        seen.push(v);
        let term = ea.alloc(Expr::BinaryOp { op: BinaryOpKind::LtEq, left: v, right: b_lit });
        guard = Some(match guard {
            None => term,
            Some(g) => ea.alloc(Expr::BinaryOp { op: BinaryOpKind::And, left: g, right: term }),
        });
    }
    let guard = guard?;

    let then_block = sa.alloc_slice(vec![fast_init, fast_loop]);
    let else_block = sa.alloc_slice(vec![init.clone(), w.clone()]);
    super::mark_fired(crate::optimization::Opt::LoopSplit);
    Some(Stmt::If { cond: guard, then_block, else_block: Some(else_block) })
}

fn collect_nested_levels<'a>(body: Block<'a>, ivs: &mut Vec<Symbol>, bounds: &mut Vec<&'a Expr<'a>>) {
    let mut i = 0;
    while i < body.len() {
        if i + 1 < body.len() {
            if let (init @ (Stmt::Let { .. } | Stmt::Set { .. }), w @ Stmt::While { .. }) =
                (&body[i], &body[i + 1])
            {
                if let Some(l) = counted_level(init, w) {
                    ivs.push(l.iv);
                    bounds.push(l.bound);
                    if let Stmt::While { body: inner, .. } = w {
                        collect_nested_levels(inner, ivs, bounds);
                    }
                    i += 2;
                    continue;
                }
            }
        }
        if let Stmt::If { then_block, else_block, .. } = &body[i] {
            collect_nested_levels(then_block, ivs, bounds);
            if let Some(eb) = else_block {
                collect_nested_levels(eb, ivs, bounds);
            }
        }
        i += 1;
    }
}

fn bounds_equal(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Identifier(x), Expr::Identifier(y)) => x == y,
        (Expr::Literal(Literal::Number(x)), Expr::Literal(Literal::Number(y))) => x == y,
        _ => false,
    }
}

/// The pure, duplication-safe expression fragment (inline_tiny's rule).
fn pure_fragment(e: &Expr) -> bool {
    match e {
        Expr::Literal(_) | Expr::Identifier(_) => true,
        Expr::BinaryOp { left, right, .. } => pure_fragment(left) && pure_fragment(right),
        Expr::Not { operand } => pure_fragment(operand),
        _ => false,
    }
}

fn is_int_arith(op: BinaryOpKind) -> bool {
    matches!(
        op,
        BinaryOpKind::Add
            | BinaryOpKind::Subtract
            | BinaryOpKind::Multiply
            | BinaryOpKind::Divide
            | BinaryOpKind::Modulo
    )
}

/// Does `e` qualify as a raw-provable chain: every leaf a literal or nest
/// IV, every `/`/`%` divisor a nonzero literal?
fn chain_qualifies(e: &Expr, ivs: &[Symbol]) -> bool {
    match e {
        Expr::Literal(Literal::Number(_)) => true,
        Expr::Identifier(s) => ivs.contains(s),
        Expr::BinaryOp { op, left, right } if is_int_arith(*op) => {
            if matches!(op, BinaryOpKind::Divide | BinaryOpKind::Modulo)
                && !matches!(right, Expr::Literal(Literal::Number(d)) if *d != 0)
            {
                return false;
            }
            chain_qualifies(left, ivs) && chain_qualifies(right, ivs)
        }
        _ => false,
    }
}

/// Worst-case magnitude with IV leaves capped at `b` (literals exact).
/// `None` = not a qualifying chain node or the bound arithmetic overflows.
fn chain_magnitude(e: &Expr, ivs: &[Symbol], b: u128) -> Option<u128> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(n.unsigned_abs() as u128),
        Expr::Identifier(s) if ivs.contains(s) => Some(b),
        Expr::BinaryOp { op, left, right } if is_int_arith(*op) => {
            let l = chain_magnitude(left, ivs, b)?;
            let r = chain_magnitude(right, ivs, b)?;
            match op {
                BinaryOpKind::Add | BinaryOpKind::Subtract => l.checked_add(r),
                BinaryOpKind::Multiply => l.checked_mul(r),
                BinaryOpKind::Divide => Some(l),
                BinaryOpKind::Modulo => Some(r.min(l)),
                _ => unreachable!("is_int_arith covers exactly these five"),
            }
        }
        _ => None,
    }
}

/// Collect every MAXIMAL qualifying chain in the block — directly, and as
/// the SUBSTITUTED body of a qualifying tiny-helper call.
/// Is `value` an induction variable's own affine self-update — `iv`, `iv ± c`,
/// or `c + iv` for a literal `c`? Such an update is the loop's counter step,
/// bounded by the loop condition, and must not be treated as a versionable chain.
fn is_iv_self_update(value: &Expr, iv: Symbol) -> bool {
    let is_iv = |e: &Expr| matches!(e, Expr::Identifier(s) if *s == iv);
    let is_lit = |e: &Expr| matches!(e, Expr::Literal(Literal::Number(_)));
    match value {
        Expr::Identifier(s) => *s == iv,
        Expr::BinaryOp { op, left, right }
            if matches!(op, BinaryOpKind::Add | BinaryOpKind::Subtract) =>
        {
            (is_iv(left) && is_lit(right)) || (is_lit(left) && is_iv(right))
        }
        _ => false,
    }
}

fn collect_chains_block<'a>(
    stmts: Block<'a>,
    ivs: &[Symbol],
    helpers: &Helpers<'a>,
    ea: &'a Arena<Expr<'a>>,
    out: &mut Vec<&'a Expr<'a>>,
) {
    for s in stmts {
        match s {
            Stmt::Let { value, .. } => collect_chains_expr(value, ivs, helpers, ea, out),
            Stmt::Set { target, value } => {
                // An IV's own counter increment (`Set iv to iv ± c`) is bounded by the
                // loop condition, so it can never overflow — collecting it as a chain
                // versions the loop pointlessly and, when the counter is read after the
                // loop, wraps it in a dead guard that puts the counter out of scope.
                // Skip it; real accumulator chains (`total + f(iv)`, whose target is NOT
                // an IV) are still collected, so the spectral-nest case is unaffected.
                if !(ivs.contains(target) && is_iv_self_update(value, *target)) {
                    collect_chains_expr(value, ivs, helpers, ea, out);
                }
            }
            // A collection store (`Set item i of m to i*7`, `Push i*5 to a`) is a
            // build/fill loop owned by the affine-array, capacity-scaling and
            // with-capacity codegen passes — every one of which requires the loop to
            // stay a FLAT top-level `While`. Treating the stored value as a versionable
            // chain wraps the loop in `if B { … } else { … }`, hiding it from those
            // passes and duplicating the counter `Let` into both arms (dense-map empty
            // window, C `i undeclared`, VM undefined-`i`). The genuine spectral-nest
            // case versions on a SCALAR accumulator (`Set total to total + f(iv)`,
            // handled above), never a store, so excluding stores leaves it intact.
            Stmt::SetIndex { .. } | Stmt::Push { .. } => {}
            Stmt::If { cond, then_block, else_block } => {
                collect_chains_expr(cond, ivs, helpers, ea, out);
                collect_chains_block(then_block, ivs, helpers, ea, out);
                if let Some(eb) = else_block {
                    collect_chains_block(eb, ivs, helpers, ea, out);
                }
            }
            Stmt::While { cond, body, .. } => {
                collect_chains_expr(cond, ivs, helpers, ea, out);
                collect_chains_block(body, ivs, helpers, ea, out);
            }
            _ => {}
        }
    }
}

fn collect_chains_expr<'a>(
    e: &'a Expr<'a>,
    ivs: &[Symbol],
    helpers: &Helpers<'a>,
    ea: &'a Arena<Expr<'a>>,
    out: &mut Vec<&'a Expr<'a>>,
) {
    if matches!(e, Expr::BinaryOp { op, .. } if is_int_arith(*op)) && chain_qualifies(e, ivs) {
        out.push(e);
        return;
    }
    match e {
        Expr::BinaryOp { left, right, .. } => {
            collect_chains_expr(left, ivs, helpers, ea, out);
            collect_chains_expr(right, ivs, helpers, ea, out);
        }
        Expr::Not { operand } => collect_chains_expr(operand, ivs, helpers, ea, out),
        Expr::Index { collection, index } => {
            collect_chains_expr(collection, ivs, helpers, ea, out);
            collect_chains_expr(index, ivs, helpers, ea, out);
        }
        Expr::Length { collection } => collect_chains_expr(collection, ivs, helpers, ea, out),
        Expr::Call { function, args } => {
            // A qualifying tiny-helper call contributes its SUBSTITUTED body's
            // chains (the same substitution the fast clone will perform — the
            // transient nodes here only feed the B-solve magnitudes).
            if let Some((params, body)) = helpers.get(function) {
                let arg_ok = args.iter().all(|a| {
                    matches!(a, Expr::Literal(_))
                        || matches!(a, Expr::Identifier(s) if ivs.contains(s))
                });
                if arg_ok && params.len() == args.len() {
                    let bind: HashMap<Symbol, &Expr> =
                        params.iter().copied().zip(args.iter().copied()).collect();
                    collect_chains_expr(subst_expr(body, &bind, ea), ivs, helpers, ea, out);
                    return;
                }
            }
            for a in args {
                collect_chains_expr(a, ivs, helpers, ea, out);
            }
        }
        _ => {}
    }
}

/// Substitute helper params with the call's argument expressions on fresh
/// arena nodes.
fn subst_expr<'a>(
    e: &'a Expr<'a>,
    bind: &HashMap<Symbol, &'a Expr<'a>>,
    ea: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    match e {
        Expr::Identifier(s) => bind.get(s).copied().unwrap_or(e),
        Expr::Literal(_) => e,
        Expr::BinaryOp { op, left, right } => ea.alloc(Expr::BinaryOp {
            op: *op,
            left: subst_expr(left, bind, ea),
            right: subst_expr(right, bind, ea),
        }),
        Expr::Not { operand } => ea.alloc(Expr::Not { operand: subst_expr(operand, bind, ea) }),
        _ => e,
    }
}

// =============================================================================
// Fast-clone construction (deep copy over the closed fragment)
// =============================================================================

fn copy_stmt<'a>(
    s: &Stmt<'a>,
    ivs: &[Symbol],
    helpers: &Helpers<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Option<Stmt<'a>> {
    Some(match s {
        Stmt::Let { var, ty, value, mutable } => Stmt::Let {
            var: *var,
            ty: *ty,
            value: copy_expr(value, ivs, helpers, ea)?,
            mutable: *mutable,
        },
        Stmt::Set { target, value } => Stmt::Set {
            target: *target,
            value: copy_expr(value, ivs, helpers, ea)?,
        },
        Stmt::SetIndex { collection, index, value } => Stmt::SetIndex {
            collection: copy_expr(collection, ivs, helpers, ea)?,
            index: copy_expr(index, ivs, helpers, ea)?,
            value: copy_expr(value, ivs, helpers, ea)?,
        },
        Stmt::Push { collection, value } => Stmt::Push {
            collection: copy_expr(collection, ivs, helpers, ea)?,
            value: copy_expr(value, ivs, helpers, ea)?,
        },
        Stmt::If { cond, then_block, else_block } => Stmt::If {
            cond: copy_expr(cond, ivs, helpers, ea)?,
            then_block: copy_block(then_block, ivs, helpers, ea, sa)?,
            else_block: match else_block {
                None => None,
                Some(eb) => Some(copy_block(eb, ivs, helpers, ea, sa)?),
            },
        },
        Stmt::While { cond, body, decreasing } => Stmt::While {
            cond: copy_expr(cond, ivs, helpers, ea)?,
            body: copy_block(body, ivs, helpers, ea, sa)?,
            decreasing: *decreasing,
        },
        Stmt::Break => Stmt::Break,
        _ => return None,
    })
}

fn copy_block<'a>(
    b: Block<'a>,
    ivs: &[Symbol],
    helpers: &Helpers<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Option<Block<'a>> {
    let mut out = Vec::with_capacity(b.len());
    for s in b {
        out.push(copy_stmt(s, ivs, helpers, ea, sa)?);
    }
    Some(sa.alloc_slice(out))
}

/// Deep-copy an expression; a qualifying tiny-helper call is replaced by its
/// substituted body (fresh nodes).
fn copy_expr<'a>(
    e: &'a Expr<'a>,
    ivs: &[Symbol],
    helpers: &Helpers<'a>,
    ea: &'a Arena<Expr<'a>>,
) -> Option<&'a Expr<'a>> {
    Some(match e {
        Expr::Literal(_) | Expr::Identifier(_) => e,
        Expr::BinaryOp { op, left, right } => ea.alloc(Expr::BinaryOp {
            op: *op,
            left: copy_expr(left, ivs, helpers, ea)?,
            right: copy_expr(right, ivs, helpers, ea)?,
        }),
        Expr::Not { operand } => ea.alloc(Expr::Not { operand: copy_expr(operand, ivs, helpers, ea)? }),
        Expr::Index { collection, index } => ea.alloc(Expr::Index {
            collection: copy_expr(collection, ivs, helpers, ea)?,
            index: copy_expr(index, ivs, helpers, ea)?,
        }),
        Expr::Length { collection } => ea.alloc(Expr::Length {
            collection: copy_expr(collection, ivs, helpers, ea)?,
        }),
        Expr::Call { function, args } => {
            if let Some((params, body)) = helpers.get(function) {
                let arg_ok = args.iter().all(|a| {
                    matches!(a, Expr::Literal(_))
                        || matches!(a, Expr::Identifier(s) if ivs.contains(s))
                });
                if arg_ok && params.len() == args.len() {
                    let bind: HashMap<Symbol, &Expr> =
                        params.iter().copied().zip(args.iter().copied()).collect();
                    return Some(subst_expr(body, &bind, ea));
                }
            }
            let mut new_args = Vec::with_capacity(args.len());
            for a in args {
                new_args.push(copy_expr(a, ivs, helpers, ea)?);
            }
            ea.alloc(Expr::Call { function: *function, args: new_args })
        }
        _ => return None,
    })
}

// =============================================================================
// Proven-raw marking over the fast clone
// =============================================================================

fn mark_chains_stmt<'a>(s: &Stmt<'a>, ivs: &[Symbol], ea: &'a Arena<Expr<'a>>) {
    let _ = ea;
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => mark_chains_expr(value, ivs),
        Stmt::SetIndex { collection, index, value } => {
            mark_chains_expr(collection, ivs);
            mark_chains_expr(index, ivs);
            mark_chains_expr(value, ivs);
        }
        Stmt::Push { collection, value } => {
            mark_chains_expr(collection, ivs);
            mark_chains_expr(value, ivs);
        }
        Stmt::If { cond, then_block, else_block } => {
            mark_chains_expr(cond, ivs);
            for t in *then_block {
                mark_chains_stmt(t, ivs, ea);
            }
            if let Some(eb) = else_block {
                for t in *eb {
                    mark_chains_stmt(t, ivs, ea);
                }
            }
        }
        Stmt::While { cond, body, .. } => {
            mark_chains_expr(cond, ivs);
            for t in *body {
                mark_chains_stmt(t, ivs, ea);
            }
        }
        _ => {}
    }
}

/// Register every node of every qualifying chain (each interior op is itself
/// in range: sub-chains of a bounded chain are bounded).
fn mark_chains_expr(e: &Expr, ivs: &[Symbol]) {
    if matches!(e, Expr::BinaryOp { op, .. } if is_int_arith(*op)) && chain_qualifies(e, ivs) {
        mark_all_ops(e);
        return;
    }
    match e {
        Expr::BinaryOp { left, right, .. } => {
            mark_chains_expr(left, ivs);
            mark_chains_expr(right, ivs);
        }
        Expr::Not { operand } => mark_chains_expr(operand, ivs),
        Expr::Index { collection, index } => {
            mark_chains_expr(collection, ivs);
            mark_chains_expr(index, ivs);
        }
        Expr::Length { collection } => mark_chains_expr(collection, ivs),
        Expr::Call { args, .. } => {
            for a in args {
                mark_chains_expr(a, ivs);
            }
        }
        _ => {}
    }
}

fn mark_all_ops(e: &Expr) {
    if let Expr::BinaryOp { left, right, .. } = e {
        super::mark_proven_raw_int_op(e);
        mark_all_ops(left);
        mark_all_ops(right);
    }
}
