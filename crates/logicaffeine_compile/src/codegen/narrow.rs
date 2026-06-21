//! i64 → i32 element-width narrowing for `Seq of Int`.
//!
//! A Logos `Int` is an `i64`, and a `Seq of Int` is a `Vec<i64>`. When EVERY
//! value ever written to a sequence provably fits in `i32`, the sequence can be
//! stored as `Vec<i32>` — halving its memory footprint and the cache pressure of
//! random accesses — with loads sign-extending (`x as i64`, lossless) and stores
//! truncating (`x as i32`, lossless *because* of the proof). This is the cache
//! lever for graph workloads (graph_bfs `adj`: 120 MB → 60 MB, `adjCounts`:
//! 24 MB → 12 MB).
//!
//! SOUNDNESS is per-store: a sequence narrows only if every write is covered by
//! one of three proof sources, and the value range is `⊆ [i32::MIN, i32::MAX]`
//! (statically, or under a runtime guard asserted before first use):
//!
//!  1. **Constant** — a compile-time-constant value in `i32` range. Out of range
//!     disqualifies the sequence.
//!  2. **`% m` element** — the value is `e % m`. For any integer `e` and `m > 0`,
//!     `|e % m| < m`, so `0 < m <= i32::MAX` (checked for a literal `m`, guarded
//!     at runtime for a variable `m`) makes every such value fit.
//!  3. **Accumulator** — a read-modify-write `Set item IDX of C to (item IDX of
//!     C) + d` (`d > 0` constant) with `IDX` affine in one enclosing loop's IV.
//!     For a fixed slot, the store fires at most once per iteration of each
//!     enclosing loop whose IV does NOT occur in `IDX` (those iterations all
//!     target the same slot); the slot value is bounded by
//!     `c0 + d * ∏(trip counts of those loops)`. Narrows only when that product
//!     is a static constant and the bound fits `i32`.
//!
//! Any write the analysis cannot place — an unknown value, an out-of-range
//! constant, a non-affine accumulator index, a non-constant enclosing trip —
//! leaves the sequence `Vec<i64>`. Aliasing/escape is already excluded by the
//! de-Rc precondition (a narrowed buffer is uniquely owned).
//!
//! A sequence whose bound is a *semantic* invariant rather than a static/`% m`
//! fact (e.g. a BFS distance array bounded by the graph diameter) is NOT
//! narrowable here, by design: proving it would need per-store runtime checks
//! that negate the benefit.

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use logicaffeine_base::intern::{Interner, Symbol};
use std::collections::{HashMap, HashSet};

use super::worklist::{const_eval, for_each_stmt_expr, is_new_empty_int_seq, ok_read_only, visit_idents};

const I32_MAX: i64 = i32::MAX as i64;
const I32_MIN: i64 = i32::MIN as i64;

/// How to lower a narrowed sequence: store as `Vec<i32>`, asserting `guards`
/// (e.g. a `% m` divisor bound) once before the buffer is used.
#[derive(Clone, Debug, Default)]
pub(crate) struct NarrowInfo {
    /// Runtime preconditions discharged at the declaration (each a Rust boolean
    /// expression that must hold for the narrowing to be lossless). Empty when
    /// the value range is fully static.
    pub guards: Vec<String>,
}

/// Recognize the `Seq of Int` declarations whose every element provably fits
/// `i32`. Conservative: any unclassified write drops the candidate.
pub(crate) fn detect_narrowable<'a>(
    body: &'a [Stmt<'a>],
    de_rc: &HashSet<Symbol>,
    interner: &Interner,
) -> HashMap<Symbol, NarrowInfo> {
    let mut out = HashMap::new();
    for stmt in body {
        let Stmt::Let { var, value, .. } = stmt else { continue };
        if !is_new_empty_int_seq(value) {
            continue;
        }
        let c = *var;
        // A narrowed buffer is reinterpreted element-by-element; only a uniquely
        // owned (de-Rc'd) sequence is safe — an alias would observe the raw `i32`.
        if !de_rc.contains(&c) {
            continue;
        }
        if let Some(info) = analyze(c, body, interner) {
            out.insert(c, info);
        }
    }
    out
}

/// A loop enclosing a write: its induction variable and (when both the start and
/// bound are literals) its constant trip count.
#[derive(Clone, Copy)]
struct LoopCtx {
    iv: Option<Symbol>,
    trip: Option<i64>,
}

/// Running evidence about every write to the candidate sequence.
struct Writes {
    bail: bool,
    lo: i64,
    hi: i64,
    guards: Vec<String>,
    any: bool,
    /// A write occurs inside a loop with a non-constant trip count, so the
    /// sequence is potentially large — narrowing only pays off there. A sequence
    /// sized purely by constant loops is tiny; narrowing it churns codegen for no
    /// speedup, so we leave it as `Vec<i64>`.
    large: bool,
}

fn analyze<'a>(c: Symbol, body: &'a [Stmt<'a>], interner: &Interner) -> Option<NarrowInfo> {
    // A narrowed buffer is reinterpreted element-by-element; if it is referenced
    // bare anywhere — passed to a function (which expects `Seq of Int` = i64),
    // aliased, returned — reinterpreting it as `Vec<i32>` is unsound. (de-Rc does
    // not by itself forbid passing to a non-mutating callee.)
    if escapes(body, c) {
        return None;
    }
    let mut w = Writes { bail: false, lo: 0, hi: 0, guards: Vec::new(), any: false, large: false };
    let mut rmw: HashMap<Symbol, &'a Expr<'a>> = HashMap::new();
    let mut defs: HashMap<Symbol, &'a Expr<'a>> = HashMap::new();
    walk(body, c, &mut Vec::new(), &mut rmw, &mut defs, &mut w, interner);
    if w.bail || !w.any || !w.large || w.lo < I32_MIN || w.hi > I32_MAX {
        return None;
    }
    let mut seen = HashSet::new();
    let guards = w.guards.into_iter().filter(|g| seen.insert(g.clone())).collect();
    Some(NarrowInfo { guards })
}

fn walk<'a>(
    stmts: &'a [Stmt<'a>],
    c: Symbol,
    loops: &mut Vec<LoopCtx>,
    rmw: &mut HashMap<Symbol, &'a Expr<'a>>,
    defs: &mut HashMap<Symbol, &'a Expr<'a>>,
    w: &mut Writes,
    interner: &Interner,
) {
    for (pos, s) in stmts.iter().enumerate() {
        match s {
            Stmt::Let { var, value, .. } => {
                // Track the defining expression (def-use), so a write of a
                // variable resolves to how it was computed (`neighbor = … % n`).
                defs.insert(*var, value);
                // `Let v be item IDX of c` also records the slot for an accumulator.
                match index_of(value, c) {
                    Some(idx) => {
                        rmw.insert(*var, idx);
                    }
                    None => {
                        rmw.remove(var);
                    }
                }
            }
            Stmt::Push { collection, value } => {
                if names(collection, c) {
                    classify(value, c, None, loops, rmw, defs, 0, &mut *w, interner);
                }
            }
            Stmt::SetIndex { collection, index, value } => {
                if names(collection, c) {
                    classify(value, c, Some(index), loops, rmw, defs, 0, w, interner);
                }
            }
            Stmt::Set { target, .. } => {
                // A rebind of the candidate itself (`Set c to other`) makes its
                // elements come from `other`, which this analysis did not inspect —
                // disqualify.
                if *target == c {
                    w.bail = true;
                }
                rmw.remove(target);
                defs.remove(target);
            }
            Stmt::If { then_block, else_block, .. } => {
                walk(then_block, c, loops, rmw, defs, w, interner);
                if let Some(eb) = else_block {
                    walk(eb, c, loops, rmw, defs, w, interner);
                }
            }
            Stmt::While { cond, body, .. } => {
                loops.push(loop_ctx(cond, &stmts[..pos]));
                // A variable REASSIGNED inside the loop (`Set i to i+1`, the IV) is
                // loop-carried; its pre-loop def is stale within the body, so drop
                // it. Variables freshly `Let`-bound inside the body re-establish
                // their def each iteration and stay resolvable.
                let mut inner_rmw = rmw.clone();
                let mut inner_defs = defs.clone();
                for v in set_targets(body) {
                    inner_defs.remove(&v);
                    inner_rmw.remove(&v);
                }
                walk(body, c, loops, &mut inner_rmw, &mut inner_defs, w, interner);
                loops.pop();
            }
            Stmt::Repeat { body, .. } => {
                loops.push(LoopCtx { iv: None, trip: None });
                let mut inner_rmw = rmw.clone();
                let mut inner_defs = defs.clone();
                for v in set_targets(body) {
                    inner_defs.remove(&v);
                    inner_rmw.remove(&v);
                }
                walk(body, c, loops, &mut inner_rmw, &mut inner_defs, w, interner);
                loops.pop();
            }
            _ => {}
        }
    }
}

/// `true` if `c` is used in any way other than a write (`push` / `Set item _ of
/// c`) or a read (`item _ of c` / `length of c`) — i.e. it escapes (bare
/// reference: function arg, alias, return), making the `Vec<i32>` reinterpret
/// unsound.
fn escapes(stmts: &[Stmt], c: Symbol) -> bool {
    !stmts.iter().all(|s| use_is_local(s, c))
}

fn use_is_local(s: &Stmt, c: Symbol) -> bool {
    match s {
        Stmt::Push { collection, value } => {
            (names(collection, c) || ok_read_only(collection, c)) && ok_read_only(value, c)
        }
        Stmt::SetIndex { collection, index, value } => {
            (names(collection, c) || ok_read_only(collection, c))
                && ok_read_only(index, c)
                && ok_read_only(value, c)
        }
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => ok_read_only(value, c),
        Stmt::SetField { object, value, .. } => ok_read_only(object, c) && ok_read_only(value, c),
        Stmt::If { cond, then_block, else_block } => {
            ok_read_only(cond, c)
                && then_block.iter().all(|x| use_is_local(x, c))
                && else_block.as_ref().map_or(true, |eb| eb.iter().all(|x| use_is_local(x, c)))
        }
        Stmt::While { cond, body, .. } => {
            ok_read_only(cond, c) && body.iter().all(|x| use_is_local(x, c))
        }
        Stmt::Repeat { body, .. } => body.iter().all(|x| use_is_local(x, c)),
        Stmt::Return { value: Some(v) } => ok_read_only(v, c),
        Stmt::Return { value: None } => true,
        Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => {
            ok_read_only(object, c) && ok_read_only(recipient, c)
        }
        Stmt::Call { args, .. } => args.iter().all(|a| ok_read_only(a, c)),
        Stmt::RuntimeAssert { condition } => ok_read_only(condition, c),
        Stmt::Inspect { target, .. } => ok_read_only(target, c),
        // Pop/Remove/Add on `c` change its contents/length outside this analysis.
        Stmt::Pop { collection, .. }
        | Stmt::Remove { collection, .. }
        | Stmt::Add { collection, .. } => !names(collection, c),
        _ => !mentions_top(s, c),
    }
}

/// `c` appears in a top-level expression of `s` (no block recursion; the `_` arm
/// of `use_is_local` covers only leaf statements).
fn mentions_top(s: &Stmt, c: Symbol) -> bool {
    let mut found = false;
    for_each_stmt_expr(s, &mut |e| {
        visit_idents(e, &mut |sym| {
            if sym == c {
                found = true;
            }
        })
    });
    found
}

/// Variables reassigned by a `Set` anywhere in `stmts` (including nested blocks).
fn set_targets(stmts: &[Stmt]) -> HashSet<Symbol> {
    let mut out = HashSet::new();
    fn rec(stmts: &[Stmt], out: &mut HashSet<Symbol>) {
        for s in stmts {
            match s {
                Stmt::Set { target, .. } => {
                    out.insert(*target);
                }
                Stmt::If { then_block, else_block, .. } => {
                    rec(then_block, out);
                    if let Some(eb) = else_block {
                        rec(eb, out);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } => rec(body, out),
                _ => {}
            }
        }
    }
    rec(stmts, &mut out);
    out
}

/// Classify one write of `value` into `c`. `at` is the SetIndex slot index or
/// `None` for a push. Sets `w.bail` on any unclassifiable write.
fn classify<'a>(
    value: &Expr<'a>,
    c: Symbol,
    at: Option<&Expr<'a>>,
    loops: &[LoopCtx],
    rmw: &HashMap<Symbol, &'a Expr<'a>>,
    defs: &HashMap<Symbol, &'a Expr<'a>>,
    depth: u32,
    w: &mut Writes,
    interner: &Interner,
) {
    // Size heuristic: a write inside a non-constant-trip loop means the sequence
    // can grow large enough for narrowing to pay off. (Set once; depth 0 only.)
    if depth == 0 && loops.iter().any(|l| l.trip.is_none()) {
        w.large = true;
    }
    // 0. Def-use: a write of a variable classifies as how the variable was
    //    computed (`Set item _ of adj to neighbor`, `neighbor = … % n`). Bounded
    //    recursion guards against pathological chains; an unresolved variable
    //    (param, loop counter, …) is an unknown write.
    if let Expr::Identifier(v) = value {
        if depth < 8 {
            if let Some(def) = defs.get(v) {
                classify(def, c, at, loops, rmw, defs, depth + 1, w, interner);
                return;
            }
        }
        w.bail = true;
        return;
    }
    // 1. Constant.
    if let Some(k) = const_eval(value) {
        if k < I32_MIN || k > I32_MAX {
            w.bail = true;
        } else {
            w.lo = w.lo.min(k);
            w.hi = w.hi.max(k);
            w.any = true;
        }
        return;
    }
    // 2. `e % m`.
    if let Expr::BinaryOp { op: BinaryOpKind::Modulo, right, .. } = value {
        match right {
            Expr::Literal(Literal::Number(m)) if *m > 0 && *m <= I32_MAX => {
                w.lo = w.lo.min(-(*m - 1));
                w.hi = w.hi.max(*m - 1);
                w.any = true;
                return;
            }
            Expr::Identifier(msym) => {
                let mn = sanitize(interner.resolve(*msym));
                w.guards.push(format!("({mn}) > 0 && ({mn}) <= {I32_MAX}"));
                w.lo = w.lo.min(I32_MIN);
                w.hi = w.hi.max(I32_MAX);
                w.any = true;
                return;
            }
            _ => {}
        }
    }
    // 3. Accumulator: `(item IDX of c) + d` at the slot being written.
    if let Some(idx) = at {
        if let Some(d) = accumulator_delta(value, c, idx, rmw) {
            if d > 0 {
                if let Some(mult) = slot_multiplier(idx, loops) {
                    w.hi = w.hi.saturating_add(d.saturating_mul(mult));
                    w.lo = w.lo.min(0);
                    w.any = true;
                    return;
                }
            }
        }
    }
    w.bail = true;
}

/// `value == X + d` (or `d + X`) where `X` reads `item idx of c` (inline or via
/// an RMW-tracked variable). Returns the positive-or-zero constant `d`.
fn accumulator_delta<'a>(
    value: &Expr<'a>,
    c: Symbol,
    idx: &Expr<'a>,
    rmw: &HashMap<Symbol, &'a Expr<'a>>,
) -> Option<i64> {
    let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = value else { return None };
    let reads_slot = |e: &Expr<'a>| -> bool {
        if let Some(read_idx) = index_of(e, c) {
            return expr_eq(read_idx, idx);
        }
        if let Expr::Identifier(v) = e {
            return rmw.get(v).map_or(false, |r| expr_eq(r, idx));
        }
        false
    };
    if reads_slot(left) {
        return const_eval(right);
    }
    if reads_slot(right) {
        return const_eval(left);
    }
    None
}

/// Max times a single slot is written: the product of trip counts of enclosing
/// loops whose IV does NOT appear in the slot index `idx`. Requires `idx` to
/// reference exactly one enclosing IV (injective in its own loop) and every
/// multiplier loop's trip to be a known constant. `None` if not provably static.
fn slot_multiplier(idx: &Expr, loops: &[LoopCtx]) -> Option<i64> {
    let idx_vars = ident_set(idx);
    let mut mult: i64 = 1;
    let mut index_loop_seen = false;
    for l in loops {
        match l.iv {
            Some(iv) if idx_vars.contains(&iv) => index_loop_seen = true,
            _ => mult = mult.checked_mul(l.trip?)?,
        }
    }
    if !index_loop_seen {
        return None;
    }
    Some(mult)
}

/// Trip count of `while iv (< | <=) bound`, using `iv`'s nearest preceding
/// literal initialization in `prior`. Non-literal start or bound ⟹ no constant
/// trip.
fn loop_ctx(cond: &Expr, prior: &[Stmt]) -> LoopCtx {
    let Expr::BinaryOp { op, left, right } = cond else { return LoopCtx { iv: None, trip: None } };
    let inclusive = match op {
        BinaryOpKind::Lt => false,
        BinaryOpKind::LtEq => true,
        _ => return LoopCtx { iv: None, trip: None },
    };
    let Expr::Identifier(iv) = left else { return LoopCtx { iv: None, trip: None } };
    let iv = *iv;
    // The bound may be a literal or a variable defined as a literal before the
    // loop (`Let n be 50`). Resolving it keeps a small `while i<n` from looking
    // "large" — only a genuinely runtime bound (`n = parseInt(...)`) is large.
    let bound = const_eval(right).or_else(|| match right {
        Expr::Identifier(b) => iv_start(prior, *b),
        _ => None,
    });
    let trip = match (iv_start(prior, iv), bound) {
        (Some(s), Some(b)) => {
            let t = if inclusive { b - s + 1 } else { b - s };
            Some(t.max(0))
        }
        _ => None,
    };
    LoopCtx { iv: Some(iv), trip }
}

/// The literal value `iv` is initialized to by its nearest preceding `Let`/`Set`.
fn iv_start(prior: &[Stmt], iv: Symbol) -> Option<i64> {
    for s in prior.iter().rev() {
        match s {
            Stmt::Let { var, value, .. } if *var == iv => return const_eval(value),
            Stmt::Set { target, value } if *target == iv => return const_eval(value),
            _ => {}
        }
    }
    None
}

/// `item IDX of coll` ⟹ `Some(IDX)` when `coll` is `sym`.
fn index_of<'a>(e: &Expr<'a>, sym: Symbol) -> Option<&'a Expr<'a>> {
    if let Expr::Index { collection, index } = e {
        if names(collection, sym) {
            return Some(index);
        }
    }
    None
}

fn names(e: &Expr, sym: Symbol) -> bool {
    matches!(e, Expr::Identifier(s) if *s == sym)
}

fn ident_set(e: &Expr) -> HashSet<Symbol> {
    let mut out = HashSet::new();
    visit_idents(e, &mut |s| {
        out.insert(s);
    });
    out
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

/// Structural equality of two index expressions (slot identity). Conservative:
/// only the forms an index can take (idents, integer literals, affine ops).
fn expr_eq(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Identifier(x), Expr::Identifier(y)) => x == y,
        (Expr::Literal(Literal::Number(x)), Expr::Literal(Literal::Number(y))) => x == y,
        (
            Expr::BinaryOp { op: o1, left: l1, right: r1 },
            Expr::BinaryOp { op: o2, left: l2, right: r2 },
        ) => o1 == o2 && expr_eq(l1, l2) && expr_eq(r1, r2),
        (Expr::Index { collection: c1, index: i1 }, Expr::Index { collection: c2, index: i2 }) => {
            expr_eq(c1, c2) && expr_eq(i1, i2)
        }
        _ => false,
    }
}
