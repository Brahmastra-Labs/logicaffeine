//! Pass 1 — bounded-accumulator overflow-guard elision (AOT / Rust-codegen path).
//!
//! A hot integer loop like `isqrt`'s Newton step `y = (x + n/x)/2` pays a
//! PER-ITERATION overflow guard because `Int` is arbitrary-precision and the
//! interval fixpoint widens the loop-carried accumulator to ±∞ (it cannot prove
//! `x + n/x` fits i64, so codegen wraps every add/mul in `{ if in-range { raw }
//! else { promote to BigInt } }`). This pass proves the loop is i64-safe *whenever
//! its inputs fit a compile-time bound B*, and VERSIONS the (preamble + loop):
//!
//! ```text
//! If <every loop-invariant input v satisfies -B <= v <= B> {
//!     <fast clone of the preamble+loop; every arithmetic op marked proven-raw>
//! } Else {
//!     <the ORIGINAL preamble+loop, unchanged — keeps its promoting guards>
//! }
//! ```
//!
//! The common (small/medium) inputs take the fast bare-i64 branch; a genuinely huge
//! input falls to the ORIGINAL promoting path and is still exact (NEVER an abort —
//! this is why we version rather than `assert!`, unlike the array-bounds `entry_guard`).
//!
//! SOUNDNESS — an inductive-invariant check by interval arithmetic. Assume every loop
//! variable (accumulators + inputs) lies in `[-B, B]`. For each arithmetic node,
//! compute the result interval under that assumption in i128, with saturating corner
//! arithmetic. Only an operand already outside i64 can carry a corner past i128, and the
//! op that produced that operand has already marked the region unsafe, so saturation never
//! changes a verdict; it keeps the analysis itself from overflowing:
//!   * `a+b`,`a-b`,`a*b` → corner interval; SAFE iff ⊆ [i64::MIN,i64::MAX]. `*` of
//!     two `[-B,B]` gives `[-B²,B²]` (needs `B²<=i64::MAX`); `+`/`-` give `[-2B,2B]`.
//!   * `a/b`,`a%b` → ALWAYS safe, result ⊆ `[-|a|,|a|] ⊆ [-B,B]`: `|a/b|<=|a|<=B`,
//!     `|a%b|<|b|<=B`; the only i64 division overflow is `i64::MIN/-1`, impossible as
//!     `|a|<=B<i64::MAX ⇒ a!=i64::MIN`; a zero divisor traps identically in both
//!     branches (behaviour preserved).
//! `[-B,B]` is an INDUCTIVE INVARIANT iff every `Set v to RHS` / `Let v be RHS` in the
//! versioned region has `interval(RHS) ⊆ [-B,B]` and no op escapes i64. Then the fast
//! body cannot overflow, so its ops lower raw, discharged by the guard `inputs ∈
//! [-B,B]` (which also bounds the accumulator SEEDS, computed from the inputs in the
//! preamble). Interval arithmetic OVER-approximates ⇒ a pass that fires is sound; it
//! can only fail to fire. Marks are pointer-keyed to the fast clone's fresh nodes, so
//! they never leak into the fallback.

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};
use std::collections::HashSet;

/// `2^62-1` keeps `a+b` (`<= 2·(2^62-1) < i64::MAX`) safe; `⌊√(i64::MAX)⌋` keeps
/// `a*b` safe. Widest first: an add/div-only loop gets the wide bound, a loop with a
/// multiply needs the mul-safe one.
const B_ADD_SAFE: i64 = (1i64 << 62) - 1;
const B_MUL_SAFE: i64 = 3_037_000_499; // ⌊√(i64::MAX)⌋; 3037000499² < i64::MAX

#[derive(Clone, Copy)]
struct Iv {
    lo: i128,
    hi: i128,
}
impl Iv {
    fn point(n: i128) -> Iv {
        Iv { lo: n, hi: n }
    }
    fn band(b: i64) -> Iv {
        Iv {
            lo: -(b as i128),
            hi: b as i128,
        }
    }
    fn fits_i64(self) -> bool {
        self.lo >= i64::MIN as i128 && self.hi <= i64::MAX as i128
    }
    fn within(self, b: i64) -> bool {
        self.lo >= -(b as i128) && self.hi <= b as i128
    }
}

/// Interval of `e` under "every variable is in `[-B,B]`", or `None` if `e` contains a
/// construct outside the supported integer-arithmetic grammar (which disqualifies the
/// region). `unsafe_op` is set if any `+`/`-`/`*` result escapes i64 (⇒ this B fails).
fn interval_of(e: &Expr, b: i64, unsafe_op: &mut bool, interner: &Interner) -> Option<Iv> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(Iv::point(*n as i128)),
        Expr::Identifier(_) => Some(Iv::band(b)),
        // `abs(x)` is bounded by |x|: interval `[0, max(|lo|,|hi|)]`. Its argument's ops
        // are still checked for i64 escape (the arg is marked/lowered raw too). Any OTHER
        // call is an unknown value — un-boundable, so the region is ineligible.
        Expr::Call { function, args }
            if args.len() == 1 && interner.resolve(*function) == "abs" =>
        {
            let a = interval_of(args[0], b, unsafe_op, interner)?;
            let m = a.lo.saturating_abs().max(a.hi.saturating_abs());
            Some(Iv { lo: 0, hi: m })
        }
        Expr::BinaryOp { op, left, right } => {
            let l = interval_of(left, b, unsafe_op, interner)?;
            let r = interval_of(right, b, unsafe_op, interner)?;
            let arith = |iv: Iv, unsafe_op: &mut bool| {
                if !iv.fits_i64() {
                    *unsafe_op = true;
                }
                Some(iv)
            };
            match op {
                BinaryOpKind::Add => {
                    arith(Iv { lo: l.lo.saturating_add(r.lo), hi: l.hi.saturating_add(r.hi) }, unsafe_op)
                }
                BinaryOpKind::Subtract => {
                    arith(Iv { lo: l.lo.saturating_sub(r.hi), hi: l.hi.saturating_sub(r.lo) }, unsafe_op)
                }
                BinaryOpKind::Multiply => {
                    let c = [
                        l.lo.saturating_mul(r.lo),
                        l.lo.saturating_mul(r.hi),
                        l.hi.saturating_mul(r.lo),
                        l.hi.saturating_mul(r.hi),
                    ];
                    arith(
                        Iv {
                            lo: *c.iter().min().unwrap(),
                            hi: *c.iter().max().unwrap(),
                        },
                        unsafe_op,
                    )
                }
                BinaryOpKind::Divide | BinaryOpKind::Modulo => Some(Iv::band(b)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The `(target, RHS)` of every `Set`/`Let` directly in `stmts` (recursing into
/// nested `If`/`While`). Any statement we do not model (I/O, index, push, call, …)
/// sets `ok=false` — the region is then ineligible.
fn collect_sets<'a>(stmts: &'a [Stmt<'a>], out: &mut Vec<(Symbol, &'a Expr<'a>)>, ok: &mut bool) {
    for s in stmts {
        match s {
            Stmt::Set { target, value }
            | Stmt::Let {
                var: target, value, ..
            } => out.push((*target, value)),
            Stmt::If {
                cond: _,
                then_block,
                else_block,
            } => {
                collect_sets(then_block, out, ok);
                if let Some(e) = else_block {
                    collect_sets(e, out, ok);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => collect_sets(body, out, ok),
            _ => *ok = false,
        }
    }
}

/// Every identifier read anywhere in `e`, in left-to-right order (deterministic;
/// deduped by the caller). Covers the arithmetic grammar + comparisons/`Not` (so a
/// loop guard `y < x` contributes its reads too).
fn reads(e: &Expr, out: &mut Vec<Symbol>) {
    match e {
        Expr::Identifier(s) => out.push(*s),
        Expr::BinaryOp { left, right, .. } => {
            reads(left, out);
            reads(right, out);
        }
        Expr::Not { operand } => reads(operand, out),
        Expr::Call { args, .. } => args.iter().for_each(|a| reads(a, out)),
        _ => {}
    }
}

/// Deep-clone an expression into fresh arena nodes so the fast branch's address-keyed
/// proven-raw marks never touch the fallback's nodes.
fn clone_expr<'a>(e: &'a Expr<'a>, ea: &'a Arena<Expr<'a>>) -> &'a Expr<'a> {
    match e {
        Expr::BinaryOp { op, left, right } => ea.alloc(Expr::BinaryOp {
            op: *op,
            left: clone_expr(left, ea),
            right: clone_expr(right, ea),
        }),
        Expr::Not { operand } => ea.alloc(Expr::Not {
            operand: clone_expr(operand, ea),
        }),
        // A call node itself carries no proven-raw mark, but its ARGUMENTS may contain the
        // arithmetic (e.g. `abs(i - n/2)`) we mark — so clone the args into fresh nodes.
        Expr::Call { function, args } => ea.alloc(Expr::Call {
            function: *function,
            args: args.iter().map(|a| clone_expr(a, ea)).collect(),
        }),
        // Leaves (Identifier/Literal/…) are immutable and unmarked — `Expr` is not
        // `Clone`, and sharing an unmarked leaf between the fast and fallback branches
        // is sound (only `BinaryOp` nodes get proven-raw marks, and those are fresh).
        _ => e,
    }
}

/// Clone a statement subtree with fresh arithmetic nodes, marking every `+`/`-`/`*`/
/// `/`/`%` node proven-raw (the region is proven i64-safe by the enclosing guard).
fn clone_marked_stmt<'a>(
    s: &Stmt<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Stmt<'a> {
    let ce = |e: &'a Expr<'a>| -> &'a Expr<'a> {
        let c = clone_expr(e, ea);
        mark_all_arith(c);
        c
    };
    match s {
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => Stmt::Let {
            var: *var,
            ty: *ty,
            value: ce(value),
            mutable: *mutable,
        },
        Stmt::Set { target, value } => Stmt::Set {
            target: *target,
            value: ce(value),
        },
        Stmt::While {
            cond,
            body,
            decreasing,
        } => Stmt::While {
            cond: clone_expr(cond, ea),
            body: clone_marked_block(body, ea, sa),
            decreasing: *decreasing,
        },
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => Stmt::If {
            cond: clone_expr(cond, ea),
            then_block: clone_marked_block(then_block, ea, sa),
            else_block: else_block.map(|e| clone_marked_block(e, ea, sa)),
        },
        // Only the modelled statement kinds reach here (collect_sets gated it).
        other => other.clone(),
    }
}

fn clone_marked_block<'a>(
    stmts: &[Stmt<'a>],
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
) -> Block<'a> {
    let v: Vec<Stmt<'a>> = stmts.iter().map(|s| clone_marked_stmt(s, ea, sa)).collect();
    sa.alloc_slice(v)
}

/// Mark every arithmetic node in `e` proven-raw (recursively).
fn mark_all_arith(e: &Expr) {
    if let Expr::BinaryOp {
        op:
            BinaryOpKind::Add
            | BinaryOpKind::Subtract
            | BinaryOpKind::Multiply
            | BinaryOpKind::Divide
            | BinaryOpKind::Modulo,
        ..
    } = e
    {
        super::facts::mark_expr(super::facts::Fact::RawIntOp, e);
    }
    match e {
        Expr::BinaryOp { left, right, .. } => {
            mark_all_arith(left);
            mark_all_arith(right);
        }
        Expr::Not { operand } => mark_all_arith(operand),
        Expr::Call { args, .. } => args.iter().for_each(|a| mark_all_arith(a)),
        _ => {}
    }
}

/// Build `-b <= v && v <= b` (i.e. `v >= -b && v <= b`) for one input `v`.
fn band_guard<'a>(v: Symbol, b: i64, ea: &'a Arena<Expr<'a>>) -> &'a Expr<'a> {
    let id = ea.alloc(Expr::Identifier(v));
    let lo = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::GtEq,
        left: id,
        right: ea.alloc(Expr::Literal(Literal::Number(-b))),
    });
    let hi = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::LtEq,
        left: id,
        right: ea.alloc(Expr::Literal(Literal::Number(b))),
    });
    ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::And,
        left: lo,
        right: hi,
    })
}

/// Try to version `preamble` (contiguous Let/Set) + `while_stmt`. Returns the
/// replacement statements (a single `If`), or `None` when ineligible/unprovable.
fn try_version<'a>(
    preamble: &[Stmt<'a>],
    while_stmt: &Stmt<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &Interner,
) -> Option<Stmt<'a>> {
    let Stmt::While { cond, body, .. } = while_stmt else {
        return None;
    };

    // Gather all modelled assignments across preamble + loop body.
    let mut sets = Vec::new();
    let mut ok = true;
    collect_sets(preamble, &mut sets, &mut ok);
    collect_sets(body, &mut sets, &mut ok);
    if !ok || sets.is_empty() {
        return None;
    }
    // The versioned region must contain at least one guarded-in-i64 arithmetic op —
    // otherwise there is nothing to elide.
    let has_arith = sets
        .iter()
        .any(|(_, rhs)| matches!(rhs, Expr::BinaryOp { .. }));
    if !has_arith {
        return None;
    }

    // assigned = every accumulator; inputs = everything read but not assigned, in a
    // deterministic first-read order (byte-identical codegen depends on it).
    let assigned: HashSet<Symbol> = sets.iter().map(|(t, _)| *t).collect();
    let mut ordered: Vec<Symbol> = Vec::new();
    for (_, rhs) in &sets {
        reads(rhs, &mut ordered);
    }
    reads(cond, &mut ordered);
    let mut seen: HashSet<Symbol> = HashSet::new();
    let inputs: Vec<Symbol> = ordered
        .into_iter()
        .filter(|s| !assigned.contains(s) && seen.insert(*s))
        .collect();
    if inputs.is_empty() {
        // Nothing to bound in a guard (e.g. all values are accumulators seeded by
        // literals) — the constant-fold / closed-form passes own that case.
        return None;
    }

    // Prove [-B,B] is an inductive invariant for some B: every RHS re-fits [-B,B]
    // and no op escapes i64.
    let prove = |b: i64| -> bool {
        sets.iter().all(|(_, rhs)| {
            let mut unsafe_op = false;
            matches!(interval_of(rhs, b, &mut unsafe_op, interner), Some(iv) if !unsafe_op && iv.within(b))
        })
    };
    let b = if prove(B_ADD_SAFE) {
        B_ADD_SAFE
    } else if prove(B_MUL_SAFE) {
        B_MUL_SAFE
    } else {
        return None;
    };

    // Guard: every input in [-B,B], AND-folded (inputs already deterministically ordered).
    let mut guard = band_guard(inputs[0], b, ea);
    for &v in &inputs[1..] {
        guard = ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::And,
            left: guard,
            right: band_guard(v, b, ea),
        });
    }

    // Version ONLY the loop: the preamble stays OUTSIDE (keeping its one-time guards),
    // so the accumulators remain in scope for both branches AND for any code after the
    // loop (e.g. `return x`). Moving the `let`s into a branch would put them out of
    // scope downstream — the bug the adversarial `isqrt(i64::MAX)` lock caught. The
    // preamble interval-check above still proves the accumulators ENTER the loop within
    // [-B,B] whenever `inputs <= B`, so the fast (marked-raw) loop body is sound.
    let fast_while = clone_marked_stmt(while_stmt, ea, sa);
    let then_block = sa.alloc_slice(vec![fast_while]);
    let else_block = sa.alloc_slice(vec![while_stmt.clone()]);
    Some(Stmt::If {
        cond: guard,
        then_block,
        else_block: Some(else_block),
    })
}

/// `counter </<= limit` → `(counter, limit, inclusive)`. The counter is the loop variable
/// (left operand, an identifier); the limit is the right operand.
fn extract_counter_limit<'a>(cond: &'a Expr<'a>) -> Option<(Symbol, &'a Expr<'a>, bool)> {
    match cond {
        Expr::BinaryOp {
            op: BinaryOpKind::Lt,
            left,
            right,
        } => match &**left {
            Expr::Identifier(c) => Some((*c, *right, false)),
            _ => None,
        },
        Expr::BinaryOp {
            op: BinaryOpKind::LtEq,
            left,
            right,
        } => match &**left {
            Expr::Identifier(c) => Some((*c, *right, true)),
            _ => None,
        },
        _ => None,
    }
}

/// Is `value` exactly `counter + 1` (either operand order)?
fn is_plus_one(value: &Expr, counter: Symbol) -> bool {
    let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    } = value
    else {
        return false;
    };
    let one = |e: &Expr| matches!(e, Expr::Literal(Literal::Number(1)));
    let cv = |e: &Expr| matches!(e, Expr::Identifier(s) if *s == counter);
    (cv(left) && one(right)) || (one(left) && cv(right))
}

/// Flatten a `sum`-accumulator update `sum = sum + t1 + t2 + …` (a left-/right-associated `+`
/// tree) into its increment terms `[t1, t2, …]`. `sum` must appear EXACTLY ONCE as a leaf and no
/// term may contain `sum`. Returns `None` if the top expression is not such a `+`-tree. Subsumes
/// the single-term case (`sum + f` → `[f]`).
fn accumulator_terms<'a>(value: &'a Expr<'a>, sum: Symbol) -> Option<Vec<&'a Expr<'a>>> {
    fn walk<'a>(
        e: &'a Expr<'a>,
        sum: Symbol,
        terms: &mut Vec<&'a Expr<'a>>,
        sum_count: &mut i32,
    ) -> bool {
        match e {
            Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left,
                right,
            } => walk(left, sum, terms, sum_count) && walk(right, sum, terms, sum_count),
            Expr::Identifier(s) if *s == sum => {
                *sum_count += 1;
                true
            }
            other => {
                // A term must be a pure function of the counter/invariants — never the accumulator.
                if reads_contains(other, sum) {
                    return false;
                }
                terms.push(other);
                true
            }
        }
    }
    let mut terms = Vec::new();
    let mut sum_count = 0;
    if !walk(value, sum, &mut terms, &mut sum_count) {
        return None;
    }
    if sum_count != 1 || terms.is_empty() {
        return None;
    }
    Some(terms)
}

/// Does `e` read the identifier `sym` anywhere?
fn reads_contains(e: &Expr, sym: Symbol) -> bool {
    let mut r = Vec::new();
    reads(e, &mut r);
    r.contains(&sym)
}

/// The literal a variable is seeded with in the preamble (its last `Let`/`Set` to a number),
/// or `None` if it is not a plain literal seed.
fn find_init_value(preamble: &[Stmt], var: Symbol) -> Option<i64> {
    let mut init = None;
    for s in preamble {
        if let Stmt::Let { var: v, value, .. } | Stmt::Set { target: v, value } = s {
            if *v == var {
                init = match value {
                    Expr::Literal(Literal::Number(n)) => Some(*n),
                    _ => None,
                };
            }
        }
    }
    init
}

/// Try to version a LINEAR-ACCUMULATOR loop `while counter </<= limit { sum = sum + f(counter);
/// counter = counter + 1 }` so the fast branch accumulates with a PLAIN i64 `+` (which LLVM
/// vectorizes) instead of the promoting `logos_add_i64` call. Distinct from `try_version`,
/// which needs a CLOSED `[-B,B]` system — here `sum` GROWS; we bound its FINAL magnitude by
/// `iterations × max|f| ≤ limit·M ≤ B·M` and require `(B+1)·M ≤ i64::MAX` under the entry
/// guard `every input ∈ [-B,B]`. Interval arithmetic over-approximates ⇒ firing is sound.
fn try_version_linear<'a>(
    while_stmt: &Stmt<'a>,
    preamble: &[Stmt<'a>],
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &Interner,
) -> Option<LinearVer<'a>> {
    let Stmt::While { cond, body, .. } = while_stmt else {
        return None;
    };
    let (counter, limit_expr, incl) = extract_counter_limit(cond)?;
    if body.len() != 2 {
        return None;
    }
    // Identify the accumulator set + the counter increment (either statement order).
    let mut sum: Option<Symbol> = None;
    let mut terms: Option<Vec<&'a Expr<'a>>> = None;
    let mut counter_inc = false;
    for s in body.iter() {
        match s {
            Stmt::Set { target, value } if *target == counter => {
                if is_plus_one(value, counter) {
                    counter_inc = true;
                } else {
                    return None;
                }
            }
            Stmt::Set { target, value } => {
                let ts = accumulator_terms(value, *target)?;
                sum = Some(*target);
                terms = Some(ts);
            }
            _ => return None,
        }
    }
    if !counter_inc {
        return None;
    }
    let (sum, terms) = (sum?, terms?);
    // The accumulator must be seeded with a small literal (in range for any candidate B).
    let init = find_init_value(preamble, sum)?;
    if init.unsigned_abs() > (1u64 << 31) {
        return None;
    }
    // Largest B in the ladder where every op in every term fits i64 AND the running sum cannot
    // overflow: `(B+1)·ΣM ≤ i64::MAX`, `ΣM = Σ max|term_k|` under all-vars-in-[-B,B] (the per-
    // iteration increment is `Σ term_k`, bounded by `ΣM`; the sum by `iterations·ΣM ≤ B·ΣM`).
    const CANDS: [i64; 5] = [
        B_MUL_SAFE,
        2_000_000_000,
        1_000_000_000,
        500_000_000,
        100_000_000,
    ];
    let mut chosen_b: Option<i64> = None;
    for &b in &CANDS {
        let mut total_m: i128 = 0;
        let mut ok = true;
        for t in &terms {
            let mut unsafe_op = false;
            match interval_of(t, b, &mut unsafe_op, interner) {
                Some(iv) if !unsafe_op => {
                    total_m = total_m.saturating_add(iv.lo.abs().max(iv.hi.abs()));
                }
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && (b as i128 + 1).saturating_mul(total_m) <= i64::MAX as i128 {
            chosen_b = Some(b);
            break;
        }
    }
    let b = chosen_b?;
    // Guard inputs = identifiers read by any term or the limit, minus the loop counter
    // (bounded by the loop) and the accumulator, in deterministic first-read order.
    let mut ordered: Vec<Symbol> = Vec::new();
    for t in &terms {
        reads(t, &mut ordered);
    }
    reads(limit_expr, &mut ordered);
    let mut seen: HashSet<Symbol> = HashSet::new();
    let inputs: Vec<Symbol> = ordered
        .into_iter()
        .filter(|s| *s != counter && *s != sum && seen.insert(*s))
        .collect();
    if inputs.is_empty() {
        return None;
    }
    let mut guard = band_guard(inputs[0], b, ea);
    for &v in &inputs[1..] {
        guard = ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::And,
            left: guard,
            right: band_guard(v, b, ea),
        });
    }
    // Fast branch: the loop with the accumulator `+` and every term arithmetic node (incl.
    // inside `abs(...)`) marked proven-raw ⇒ codegen emits plain i64 `+` and drops the term's
    // per-iteration overflow guard. Fallback: the ORIGINAL promoting loop, untouched.
    let fast_while = clone_marked_stmt(while_stmt, ea, sa);
    Some(LinearVer {
        guard,
        counter,
        bound: limit_expr,
        incl,
        fast_while,
        orig_while: while_stmt.clone(),
    })
}

/// The pieces of a linear-accumulator versioning, assembled by `version_block` into either
/// the plain if/else form or the tail-into-branches + `black_box` form (which lets the fast
/// loop keep hand-C register quality — see `version_block`).
struct LinearVer<'a> {
    guard: &'a Expr<'a>,
    counter: Symbol,
    bound: &'a Expr<'a>,
    incl: bool,
    fast_while: Stmt<'a>,
    orig_while: Stmt<'a>,
}

/// Does this preamble statement bind or assign any of `syms`? Used to decide whether the preamble
/// can travel into the versioned branches with its loop.
fn binds_any(s: &Stmt, syms: &[Symbol]) -> bool {
    match s {
        Stmt::Let { var, .. } => syms.contains(var),
        Stmt::Set { target, .. } => syms.contains(target),
        _ => true,
    }
}

/// Assemble a linear-accumulator versioning that keeps the fast loop at hand-C register
/// quality. The plain `if guard { fast } else { orig }` form leaves `sum` live across the
/// branch merge, forcing spills; worse, once the guard proves `n <= B`, LLVM exploits that
/// range into a spilling OVER-UNROLL that is slower than the bare loop. So instead the whole
/// TAIL (the statements after the loop, which consume the accumulator) moves INTO both
/// branches — the accumulator is then dead after the `If`, no merge — and the fast branch
/// reads its bound through `std::hint::black_box`, a runtime identity that hides the range
/// from the unroller. Measured: turns the `divmod` fast branch from `0.82x` to `~1.05x` vs C.
fn build_linear_versioned<'a>(
    lv: LinearVer<'a>,
    preamble: Vec<Stmt<'a>>,
    tail: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    ctr: &mut usize,
) -> Vec<Stmt<'a>> {
    let Stmt::While {
        body: fast_body,
        decreasing,
        ..
    } = lv.fast_while
    else {
        // Not a While (unreachable for a linear version); fall back to the plain if/else.
        let mut t = preamble.clone();
        t.push(lv.fast_while);
        let mut e = preamble;
        e.push(lv.orig_while);
        let then_block = sa.alloc_slice(t);
        let else_block = sa.alloc_slice(e);
        return vec![Stmt::If {
            cond: lv.guard,
            then_block,
            else_block: Some(else_block),
        }];
    };
    // `let __bbndK = <bound>` with the bound marked to emit inside `black_box(...)`.
    let bb_name = interner.intern(&format!("__bbnd{}", *ctr));
    *ctr += 1;
    // A FRESH copy of the bound so the black-box mark lands only on the fast branch's
    // binding, never leaking onto the fallback loop (which shares the original bound node).
    let bb_val: &Expr = match lv.bound {
        Expr::Identifier(s) => ea.alloc(Expr::Identifier(*s)),
        other => clone_expr(other, ea),
    };
    super::facts::mark_expr(super::facts::Fact::BlackBox, bb_val);
    let bb_let = Stmt::Let {
        var: bb_name,
        ty: None,
        value: bb_val,
        mutable: false,
    };
    // Fast loop reads its bound through the black-boxed binding.
    let op = if lv.incl {
        BinaryOpKind::LtEq
    } else {
        BinaryOpKind::Lt
    };
    let fast_cond = ea.alloc(Expr::BinaryOp {
        op,
        left: ea.alloc(Expr::Identifier(lv.counter)),
        right: ea.alloc(Expr::Identifier(bb_name)),
    });
    let fast_while = Stmt::While {
        cond: fast_cond,
        body: fast_body,
        decreasing,
    };
    // The tail is duplicated into BOTH branches (each self-contained ⇒ the accumulator is
    // dead after the If). Recurse so any nested versionable loop in the tail is handled.
    let tail_fast = version_block(tail.clone(), ea, sa, interner, ctr);
    let tail_slow = version_block(tail, ea, sa, interner, ctr);
    // ★ THE COUNTER INIT TRAVELS WITH ITS LOOP. The preamble used to stay in the OUTER block while
    // the `While` moved a level down into this `If`, which silently cost the fast loop its `for`
    // form: `codegen::peephole::try_emit_for_range_pattern` reads the shared recognizer
    // `loop_shape::extract_counted_while`, whose contract is `stmts[idx]` = the counter init and
    // `stmts[idx+1]` = the `While`, ADJACENT IN ONE BLOCK. Separated by the branch, the recognizer
    // returned `None` and `for i in 0..n` was never emitted — on EVERY function whose loop bound is
    // a parameter, since a literal bound makes this pass decline. Only `phase_optimize_v2`'s
    // opt1a/opt1b/opt7 noticed, and only because they assert the emitted SHAPE; the outputs stayed
    // correct the whole time, which is exactly why it went unnoticed. `bb_let` leads, so the
    // preamble's last statement — the counter init — stays flush against the loop.
    let mut then_v = vec![bb_let];
    then_v.extend(preamble.iter().cloned());
    then_v.push(fast_while);
    then_v.extend(tail_fast);
    let mut else_v = preamble;
    else_v.push(lv.orig_while);
    else_v.extend(tail_slow);
    vec![Stmt::If {
        cond: lv.guard,
        then_block: sa.alloc_slice(then_v),
        else_block: Some(sa.alloc_slice(else_v)),
    }]
}

/// Recurse through a block, versioning each `<contiguous Let/Set preamble> + While`
/// region and descending into nested function/branch/loop bodies.
fn version_block<'a>(
    stmts: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    ctr: &mut usize,
) -> Vec<Stmt<'a>> {
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
    let mut i = 0;
    while i < stmts.len() {
        if let Stmt::While { .. } = &stmts[i] {
            // The contiguous Let/Set run immediately preceding this While in `out`.
            let mut pre_len = 0;
            while pre_len < out.len()
                && matches!(
                    out[out.len() - 1 - pre_len],
                    Stmt::Let { .. } | Stmt::Set { .. }
                )
            {
                pre_len += 1;
            }
            let preamble: Vec<Stmt<'a>> = out[out.len() - pre_len..].to_vec();
            // Closed-[-B,B] versioning first (isqrt/powMod): plain if/else (its result `y`
            // stays live but the loop is not a growing accumulator, so no tail move).
            if let Some(if_stmt) = try_version(&preamble, &stmts[i], ea, sa, &*interner) {
                out.push(if_stmt);
                i += 1;
                continue;
            }
            // Linear-accumulator versioning (abs/divmod: a growing `sum`): tail-into-branches
            // + black_box so the fast loop keeps hand-C quality. Consumes the trailing tail.
            if let Some(lv) = try_version_linear(&stmts[i], &preamble, ea, sa, &*interner) {
                let tail: Vec<Stmt<'a>> = stmts[i + 1..].to_vec();
                // The preamble may move into the branches only if the GUARD does not read anything
                // it binds — the guard is evaluated at the `If`, before either branch runs, so a
                // preamble that computes the bound (`Let n be length of xs.`) has to stay outside.
                // Guard inputs are parameters in the common case, so the move is the common case.
                let mut guard_reads: Vec<Symbol> = Vec::new();
                reads(lv.guard, &mut guard_reads);
                let moved = if preamble.iter().any(|s| binds_any(s, &guard_reads)) {
                    Vec::new()
                } else {
                    out.truncate(out.len() - pre_len);
                    preamble.clone()
                };
                out.extend(build_linear_versioned(
                    lv, moved, tail, ea, sa, interner, ctr,
                ));
                i = stmts.len();
                continue;
            }
        }
        // Descend into structural statements (nested nests / branches / functions).
        let s = match stmts[i].clone() {
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
            } => Stmt::FunctionDef {
                name,
                generics,
                params,
                body: sa.alloc_slice(version_block(body.to_vec(), ea, sa, interner, ctr)),
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            },
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => Stmt::If {
                cond,
                then_block: sa.alloc_slice(version_block(
                    then_block.to_vec(),
                    ea,
                    sa,
                    interner,
                    ctr,
                )),
                else_block: else_block.map(|eb| -> Block<'a> {
                    sa.alloc_slice(version_block(eb.to_vec(), ea, sa, interner, ctr))
                }),
            },
            Stmt::While {
                cond,
                body,
                decreasing,
            } => Stmt::While {
                cond,
                body: sa.alloc_slice(version_block(body.to_vec(), ea, sa, interner, ctr)),
                decreasing,
            },
            other => other,
        };
        out.push(s);
        i += 1;
    }
    out
}

/// Pass entry — mirrors `bound_version::bound_version_stmts`.
pub fn accum_bound_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mut ctr = 0usize;
    version_block(stmts, ea, sa, interner, &mut ctr)
}
