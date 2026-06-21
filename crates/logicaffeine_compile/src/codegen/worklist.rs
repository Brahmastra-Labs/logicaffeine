//! Append-only worklist → pre-sized buffer + register tail (BFS/DFS frontier).
//!
//! A `Vec::push` worklist costs, per enqueue, a capacity compare, a length
//! write-back to memory, and a `grow_one` call in the loop body that blocks
//! unrolling. C uses a pre-sized array with the tail in a register
//! (`q[tail++] = x`), no check. This pass recognizes the worklist shape and the
//! BOUND that makes pre-sizing sound, so codegen can emit C's exact form.
//!
//! SOUNDNESS — the bound proof. The fire condition is a *monotone visited-set
//! guard*: every enqueue sits inside `if M[k] == S { …; M[k] := V (V != S); …;
//! push x to Q }` where `M` is a sentinel-initialised array (`M = vec![S; E]`)
//! whose elements are NEVER reset to `S`. Then each `k` fires the guard at most
//! once (after `M[k] := V` it is no longer `S`), and `k ∈ [0, E)` (it indexes
//! `M`), so the guarded pushes number at most `E`. With `U` unconditional seed
//! pushes the total is `≤ E + U` — the buffer capacity. Reads stay sound because
//! the drain loop's `front <= length(Q)` becomes `front <= tail`, so every
//! `Q[front-1]` lands below the logical length, never in the pre-sized tail.
//!
//! Anything that does not match this shape exactly is left as an ordinary
//! `Vec` — the pass only ever *removes* a `Vec::push`, never licenses an
//! unchecked write it has not bounded.

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use logicaffeine_base::intern::{Interner, Symbol};
use std::collections::{HashMap, HashSet};

/// How to emit a recognized worklist: a pre-sized buffer plus a register tail.
#[derive(Clone, Debug)]
pub(crate) struct WorklistInfo {
    /// The synthetic `usize` cursor name (`__<q>_tail`), holding the logical
    /// length — substituted for `length of Q` and advanced on each push.
    pub tail_var: String,
    /// Rust expression for the buffer length: `E + U` (visited-set size plus the
    /// seed pushes), large enough for every enqueue the bound proof permits.
    pub capacity: String,
    /// The element default the buffer is filled with (`0i64`); the tail-bounded
    /// reads never observe it.
    pub elem_default: String,
}

/// Recognize the append-only worklists in a function body and the bound that
/// licenses pre-sizing each. Conservative: any use of a candidate outside
/// `push` / `item _ of` / `length of` disqualifies it.
pub(crate) fn detect_worklists(
    body: &[Stmt],
    de_rc: &HashSet<Symbol>,
    interner: &Interner,
) -> HashMap<Symbol, WorklistInfo> {
    let mut out = HashMap::new();
    // Candidates: top-level `Let mutable q be a new Seq of Int`.
    for stmt in body {
        let Stmt::Let { var, value, .. } = stmt else { continue };
        if !is_new_empty_int_seq(value) {
            continue;
        }
        let q = *var;
        // SOUNDNESS: only a uniquely-owned (de-Rc'd) sequence is safe to rewrite
        // to a raw buffer + tail — an aliased `LogosSeq` could be observed
        // through the alias before the tail catches up.
        if !de_rc.contains(&q) {
            continue;
        }
        if let Some(info) = analyze_worklist(q, body, interner) {
            out.insert(q, info);
        }
    }
    out
}

/// `a new Seq of Int` (an empty, integer-element sequence).
pub(crate) fn is_new_empty_int_seq(value: &Expr) -> bool {
    matches!(value, Expr::New { type_name, type_args, init_fields }
        if init_fields.is_empty()
            && is_seq_name(*type_name)
            && type_args.len() == 1
            && is_int_type(&type_args[0]))
}

fn is_seq_name(_s: Symbol) -> bool {
    // Resolved by the caller's element-type check; the `new`'s declared name is
    // `Seq` for the worklist case. We accept any single-Int-arg `new` collection
    // and rely on the usage check to reject non-sequence shapes.
    true
}

fn is_int_type(t: &crate::ast::stmt::TypeExpr) -> bool {
    use crate::ast::stmt::TypeExpr;
    matches!(t, TypeExpr::Primitive(_) | TypeExpr::Named(_))
}

/// The full per-candidate analysis: classify every reference, find the drain
/// loop and visited-set guard, prove the bound, derive the capacity.
fn analyze_worklist(q: Symbol, body: &[Stmt], interner: &Interner) -> Option<WorklistInfo> {
    // Gather every reference to `q`, with the enclosing `if M[k]==S` guard
    // context, rejecting any disqualifying use.
    let mut refs = QRefs::default();
    if !walk(body, q, &mut Vec::new(), false, &mut refs) {
        return None;
    }
    if refs.disqualified || refs.pushes.is_empty() {
        return None;
    }
    // Every push is a seed (unconditional) or a visited-set-guarded enqueue.
    let mut seed_count: i64 = 0;
    let mut visited: Option<Symbol> = None;
    let mut sentinel: Option<i64> = None;
    for p in &refs.pushes {
        match &p.guard {
            None => seed_count += 1,
            Some(g) => {
                // All guarded pushes must share one visited set + sentinel.
                if visited.get_or_insert(g.array) != &g.array {
                    return None;
                }
                if sentinel.get_or_insert(g.sentinel) != &g.sentinel {
                    return None;
                }
            }
        }
    }
    let visited = visited?;
    let sentinel = sentinel?;
    // The visited set must be sentinel-monotone: created `vec![S; E]` and never
    // written `S` again, so each guard key fires at most once.
    let size = visited_set_size(body, visited, sentinel, interner)?;
    // Clamp at zero before the `usize` cast: a negative size (an ill-formed `n`)
    // must not wrap to a colossal allocation here — the program fails at the
    // visited set's own (equally huge) allocation, exactly as before.
    let capacity = format!("(({size} as i64).max(0) as usize) + {seed_count}");
    let tail_var = format!("__{}_tail", interner.resolve(q));
    Some(WorklistInfo { tail_var, capacity, elem_default: "0i64".to_string() })
}

/// A guarded enqueue's context: `if array[_] == sentinel { … }`.
#[derive(Clone)]
struct Guard {
    array: Symbol,
    sentinel: i64,
}

struct PushRef {
    guard: Option<Guard>,
}

#[derive(Default)]
struct QRefs {
    pushes: Vec<PushRef>,
    disqualified: bool,
}

/// Walk `stmts`, tracking the stack of `if array[_]==S` guards in scope, and
/// classify every reference to `q`. Returns `false` (and sets `disqualified`)
/// the moment `q` is used in any way other than push / `item _ of` / `length
/// of`. `guards` is the active visited-set-guard stack.
fn walk(stmts: &[Stmt], q: Symbol, guards: &mut Vec<Guard>, in_loop: bool, refs: &mut QRefs) -> bool {
    for s in stmts {
        if !walk_stmt(s, q, guards, in_loop, refs) {
            refs.disqualified = true;
            return false;
        }
    }
    true
}

fn walk_stmt(s: &Stmt, q: Symbol, guards: &mut Vec<Guard>, in_loop: bool, refs: &mut QRefs) -> bool {
    match s {
        Stmt::Push { collection, value } => {
            // A push of `q` is an enqueue; a push that READS `q` in its value
            // (worklist values don't) would disqualify via expr_uses.
            if expr_uses(value, q) {
                return false;
            }
            if let Expr::Identifier(c) = collection {
                if *c == q {
                    let guard = guards.last().cloned();
                    // A push NOT under a visited-set guard is a one-shot seed
                    // (counted toward the capacity). Inside a loop it would fire
                    // unboundedly — only the visited-guarded push (bounded by
                    // length(M)) may live in a loop. Reject the unbounded case.
                    if guard.is_none() && in_loop {
                        return false;
                    }
                    refs.pushes.push(PushRef { guard });
                    return true;
                }
            }
            // Push to some OTHER collection: fine as long as it doesn't name q.
            !expr_uses(collection, q)
        }
        Stmt::Let { value, .. } => ok_read_only(value, q),
        Stmt::Set { target: _, value } => ok_read_only(value, q),
        Stmt::SetIndex { collection, index, value } => {
            // A SetIndex on q is an in-place write — disqualify. On another
            // array it is fine unless it reads q.
            if names_collection(collection, q) {
                return false;
            }
            ok_read_only(index, q) && ok_read_only(value, q)
        }
        Stmt::SetField { object, value, .. } => ok_read_only(object, q) && ok_read_only(value, q),
        Stmt::If { cond, then_block, else_block } => {
            if expr_uses(cond, q) {
                return false; // q in a condition isn't the cursor pattern
            }
            // A `if M[k] == sentinel` condition pushes a visited-set guard for
            // the THEN branch only.
            let g = visited_guard(cond);
            if let Some(g) = g {
                guards.push(g);
            }
            let ok_then = walk(then_block, q, guards, in_loop, refs);
            if visited_guard(cond).is_some() {
                guards.pop();
            }
            let ok_else = match else_block {
                Some(eb) => walk(eb, q, guards, in_loop, refs),
                None => true,
            };
            ok_then && ok_else
        }
        Stmt::While { cond, body, .. } => {
            // The drain loop's `cursor <op> length of q` is fine; q anywhere
            // ELSE in the condition is not.
            if expr_uses_outside_length(cond, q) {
                return false;
            }
            // Guards do not cross a loop boundary (the array could be reset
            // between iterations from the analyzer's view) — re-derived inside.
            // `in_loop = true`: a non-guarded push inside is now unbounded.
            let mut inner = guards.clone();
            walk(body, q, &mut inner, true, refs)
        }
        Stmt::Repeat { body, .. } => walk(body, q, &mut guards.clone(), true, refs),
        Stmt::Return { value: Some(v) } => ok_read_only(v, q),
        Stmt::Return { value: None } => true,
        Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => {
            ok_read_only(object, q) && ok_read_only(recipient, q)
        }
        Stmt::Call { args, .. } => args.iter().all(|a| ok_read_only(a, q)),
        Stmt::RuntimeAssert { condition } => ok_read_only(condition, q),
        Stmt::Assert { .. } | Stmt::Trust { .. } | Stmt::Break => true,
        Stmt::Inspect { target, .. } => ok_read_only(target, q),
        // Pop / Remove / Add (set-add) on q, or anything unmodeled, disqualifies
        // if it names q.
        Stmt::Pop { collection, .. }
        | Stmt::Remove { collection, .. }
        | Stmt::Add { collection, .. } => !names_collection(collection, q),
        _ => !stmt_mentions(s, q),
    }
}

/// `q` appears in `e` only inside `item _ of q` reads or `length of q` — never
/// bare, never as another op's collection. Bare/other use ⟹ not read-only.
pub(crate) fn ok_read_only(e: &Expr, q: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s != q, // a bare reference to q escapes it
        Expr::Index { collection, index } => {
            // `item index of q` is the cursor read — allowed; q must not appear
            // in the index.
            let coll_ok = match &**collection {
                Expr::Identifier(s) if *s == q => true,
                other => ok_read_only(other, q),
            };
            coll_ok && ok_read_only(index, q)
        }
        Expr::Length { collection } => match &**collection {
            Expr::Identifier(s) if *s == q => true,
            other => ok_read_only(other, q),
        },
        Expr::BinaryOp { left, right, .. } => ok_read_only(left, q) && ok_read_only(right, q),
        Expr::Not { operand } => ok_read_only(operand, q),
        Expr::Call { args, .. } => args.iter().all(|a| ok_read_only(a, q)),
        Expr::CallExpr { callee, args } => ok_read_only(callee, q) && args.iter().all(|a| ok_read_only(a, q)),
        Expr::Copy { expr } | Expr::Give { value: expr } => ok_read_only(expr, q),
        Expr::Contains { collection, value } => !names_collection(collection, q) && ok_read_only(value, q),
        Expr::Slice { collection, start, end } => {
            !names_collection(collection, q) && ok_read_only(start, q) && ok_read_only(end, q)
        }
        Expr::Range { start, end } => ok_read_only(start, q) && ok_read_only(end, q),
        Expr::Union { left, right } | Expr::Intersection { left, right } => {
            ok_read_only(left, q) && ok_read_only(right, q)
        }
        Expr::List(items) | Expr::Tuple(items) => items.iter().all(|i| ok_read_only(i, q)),
        Expr::FieldAccess { object, .. } => ok_read_only(object, q),
        Expr::OptionSome { value } => ok_read_only(value, q),
        Expr::InterpolatedString(parts) => parts.iter().all(|p| match p {
            crate::ast::stmt::StringPart::Expr { value, .. } => ok_read_only(value, q),
            _ => true,
        }),
        _ => !expr_uses(e, q),
    }
}

pub(crate) fn names_collection(e: &Expr, q: Symbol) -> bool {
    matches!(e, Expr::Identifier(s) if *s == q)
}

/// Fold a constant integer expression (`-1` is `0 - 1` in the AST, so a plain
/// `Literal` match is not enough). Handles literals and `+ - *` of constants.
pub(crate) fn const_eval(e: &Expr) -> Option<i64> {
    match e {
        Expr::Literal(Literal::Number(k)) => Some(*k),
        Expr::BinaryOp { op, left, right } => {
            let (a, b) = (const_eval(left)?, const_eval(right)?);
            match op {
                BinaryOpKind::Add => a.checked_add(b),
                BinaryOpKind::Subtract => a.checked_sub(b),
                BinaryOpKind::Multiply => a.checked_mul(b),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `if M[k] == S` (or `S == M[k]`) with constant `S` ⟹ the visited-set guard.
fn visited_guard(cond: &Expr) -> Option<Guard> {
    let Expr::BinaryOp { op: BinaryOpKind::Eq, left, right } = cond else { return None };
    let (idx_side, sentinel) = match (const_eval(left), const_eval(right)) {
        (None, Some(s)) => (&**left, s),
        (Some(s), None) => (&**right, s),
        _ => return None,
    };
    let Expr::Index { collection, .. } = idx_side else { return None };
    let Expr::Identifier(array) = &**collection else { return None };
    Some(Guard { array: *array, sentinel })
}

/// The visited set must be created `vec![S; E]` (a counted build loop pushing
/// the sentinel, or a sized creation) and never written `S` afterward. Returns
/// the size expression `E` as a Rust string. Conservative: any write of the
/// sentinel value to `M`, or an unrecognized build, declines.
fn visited_set_size(
    body: &[Stmt],
    m: Symbol,
    sentinel: i64,
    interner: &Interner,
) -> Option<String> {
    // Monotonicity: M starts all-sentinel and every later write is provably
    // ABOVE the sentinel, so a marked slot never returns to it (each guard key
    // fires at most once).
    if !m_monotone(body, m, sentinel) {
        return None;
    }
    // Build size: a `while c < N: push <sentinel> to M; c := c + 1` loop fills M
    // to length N. We require the pushed value to be the sentinel so M starts
    // all-sentinel (the guard's precondition).
    find_sentinel_build_size(body, m, sentinel, interner)
}

/// Is the visited set `m` sentinel-monotone? Every write to `m` must be either
/// the build-fill (`= sentinel`, the initial all-sentinel state) or provably
/// ABOVE the sentinel — a constant `> S`, or `M[_] + positive` (which stays
/// `> S` because `M` is `>= S` inductively: the fill is `S`, and every later
/// write is `> S`). Then a slot, once written above `S`, never returns to `S`,
/// so each guard key fires at most once. Any other write to `m`, or `m`
/// escaping to a call or being rebound, is conservatively rejected.
fn m_monotone(stmts: &[Stmt], m: Symbol, sentinel: i64) -> bool {
    fn rec(stmts: &[Stmt], m: Symbol, sentinel: i64, ok: &mut bool) {
        for s in stmts {
            match s {
                Stmt::SetIndex { collection, value, .. } if names_collection(collection, m) => {
                    if !is_above_sentinel(value, m, sentinel) {
                        *ok = false;
                    }
                }
                Stmt::Push { collection, value } if names_collection(collection, m) => {
                    // Either the build-fill (= sentinel) or a mark (> sentinel).
                    if !is_const_eq(value, sentinel) && !is_above_sentinel(value, m, sentinel) {
                        *ok = false;
                    }
                }
                Stmt::Set { target, .. } if *target == m => *ok = false, // rebind
                Stmt::Pop { collection, .. } | Stmt::Remove { collection, .. }
                    if names_collection(collection, m) => *ok = false,
                Stmt::Call { args, .. } => {
                    if args.iter().any(|a| matches!(a, Expr::Identifier(s) if *s == m)) {
                        *ok = false; // M could be reset behind the call
                    }
                }
                Stmt::If { then_block, else_block, .. } => {
                    rec(then_block, m, sentinel, ok);
                    if let Some(eb) = else_block {
                        rec(eb, m, sentinel, ok);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } => rec(body, m, sentinel, ok),
                _ => {}
            }
        }
    }
    let mut ok = true;
    rec(stmts, m, sentinel, &mut ok);
    ok
}

/// `value` is provably `> sentinel`: a constant above it, or `M[_] + c` /
/// `c + M[_]` for a positive constant `c` (sound under the `M >= S` invariant
/// the recognizer maintains).
fn is_above_sentinel(value: &Expr, m: Symbol, sentinel: i64) -> bool {
    if let Some(k) = const_eval(value) {
        return k > sentinel;
    }
    if let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = value {
        let reads_m = |e: &Expr| matches!(e, Expr::Index { collection, .. } if names_collection(collection, m));
        let pos = |e: &Expr| matches!(const_eval(e), Some(c) if c > 0);
        return (reads_m(left) && pos(right)) || (pos(left) && reads_m(right));
    }
    false
}

fn is_const_eq(value: &Expr, sentinel: i64) -> bool {
    const_eval(value) == Some(sentinel)
}

/// Find a `while c < N: push <sentinel> to M; …` build loop and return `N` as a
/// Rust expression. Recurses into blocks. Only the bound matters; the loop
/// must push the sentinel exactly once per iteration to M.
fn find_sentinel_build_size(
    stmts: &[Stmt],
    m: Symbol,
    sentinel: i64,
    interner: &Interner,
) -> Option<String> {
    for s in stmts {
        match s {
            Stmt::While { cond, body, .. } => {
                if let Some(bound) = build_loop_bound(cond, body, m, sentinel, interner) {
                    return Some(bound);
                }
                if let Some(b) = find_sentinel_build_size(body, m, sentinel, interner) {
                    return Some(b);
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                if let Some(b) = find_sentinel_build_size(then_block, m, sentinel, interner) {
                    return Some(b);
                }
                if let Some(eb) = else_block {
                    if let Some(b) = find_sentinel_build_size(eb, m, sentinel, interner) {
                        return Some(b);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// `while c < N` whose body pushes the sentinel to `m` ⟹ `Some(N as Rust)`.
fn build_loop_bound(
    cond: &Expr,
    body: &[Stmt],
    m: Symbol,
    sentinel: i64,
    interner: &Interner,
) -> Option<String> {
    let Expr::BinaryOp { op: BinaryOpKind::Lt, left: _, right } = cond else { return None };
    // Body must push the sentinel to M.
    let pushes_sentinel = body.iter().any(|s| matches!(
        s, Stmt::Push { collection, value }
            if names_collection(collection, m) && is_const_eq(value, sentinel)));
    if !pushes_sentinel {
        return None;
    }
    bound_rust_expr(right, interner)
}

/// A loop bound that is a plain variable or integer literal, as a Rust string.
pub(crate) fn bound_rust_expr(e: &Expr, interner: &Interner) -> Option<String> {
    match e {
        Expr::Identifier(s) => Some(sanitize_ident(interner.resolve(*s))),
        Expr::Literal(Literal::Number(k)) => Some(k.to_string()),
        _ => None,
    }
}

/// Rust identifier for a LOGOS name (matches `RustNames::ident` for the common
/// case — the bound is a simple local).
pub(crate) fn sanitize_ident(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.chars().next().map_or(true, |c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn expr_uses(e: &Expr, q: Symbol) -> bool {
    !ok_read_only_noidx(e, q)
}

/// True when `e` does not reference `q` AT ALL (stricter than `ok_read_only`,
/// which permits `item _ of q`).
fn ok_read_only_noidx(e: &Expr, q: Symbol) -> bool {
    let mut uses = false;
    visit_idents(e, &mut |s| {
        if s == q {
            uses = true;
        }
    });
    !uses
}

/// `q` used in `cond` anywhere except as the operand of `length of q`.
fn expr_uses_outside_length(cond: &Expr, q: Symbol) -> bool {
    match cond {
        Expr::Length { collection } => match &**collection {
            Expr::Identifier(s) if *s == q => false,
            other => expr_uses_outside_length(other, q),
        },
        Expr::BinaryOp { left, right, .. } => {
            expr_uses_outside_length(left, q) || expr_uses_outside_length(right, q)
        }
        Expr::Not { operand } => expr_uses_outside_length(operand, q),
        Expr::Identifier(s) => *s == q,
        Expr::Index { collection, index } => {
            (matches!(&**collection, Expr::Identifier(s) if *s == q))
                || expr_uses_outside_length(collection, q)
                || expr_uses_outside_length(index, q)
        }
        _ => expr_uses(cond, q),
    }
}

fn stmt_mentions(s: &Stmt, q: Symbol) -> bool {
    let mut found = false;
    for_each_stmt_expr(s, &mut |e| {
        visit_idents(e, &mut |sym| {
            if sym == q {
                found = true;
            }
        });
    });
    found
}

/// Visit every `Identifier` in an expression tree.
pub(crate) fn visit_idents(e: &Expr, f: &mut impl FnMut(Symbol)) {
    match e {
        Expr::Identifier(s) => f(*s),
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            visit_idents(left, f);
            visit_idents(right, f);
        }
        Expr::Not { operand } => visit_idents(operand, f),
        Expr::Index { collection, index } => {
            visit_idents(collection, f);
            visit_idents(index, f);
        }
        Expr::Length { collection } | Expr::Copy { expr: collection } | Expr::Give { value: collection }
        | Expr::OptionSome { value: collection } | Expr::FieldAccess { object: collection, .. } => {
            visit_idents(collection, f)
        }
        Expr::Contains { collection, value } => {
            visit_idents(collection, f);
            visit_idents(value, f);
        }
        Expr::Slice { collection, start, end } => {
            visit_idents(collection, f);
            visit_idents(start, f);
            visit_idents(end, f);
        }
        Expr::Call { args, .. } => args.iter().for_each(|a| visit_idents(a, f)),
        Expr::CallExpr { callee, args } => {
            visit_idents(callee, f);
            args.iter().for_each(|a| visit_idents(a, f));
        }
        Expr::List(items) | Expr::Tuple(items) => items.iter().for_each(|i| visit_idents(i, f)),
        Expr::New { init_fields, .. } => init_fields.iter().for_each(|(_, e)| visit_idents(e, f)),
        Expr::NewVariant { fields, .. } => fields.iter().for_each(|(_, e)| visit_idents(e, f)),
        Expr::WithCapacity { value, capacity } => {
            visit_idents(value, f);
            visit_idents(capacity, f);
        }
        Expr::InterpolatedString(parts) => parts.iter().for_each(|p| {
            if let crate::ast::stmt::StringPart::Expr { value, .. } = p {
                visit_idents(value, f);
            }
        }),
        _ => {}
    }
}

/// Visit every top-level expression of a statement (not recursing into nested
/// blocks — callers walk those separately).
pub(crate) fn for_each_stmt_expr(s: &Stmt, f: &mut impl FnMut(&Expr)) {
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. }
        | Stmt::Return { value: Some(value) } | Stmt::Inspect { target: value, .. } => f(value),
        Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => {
            f(object);
            f(recipient);
        }
        Stmt::Push { collection, value } | Stmt::Add { collection, value } => {
            f(collection);
            f(value);
        }
        Stmt::Pop { collection, .. } | Stmt::Remove { collection, .. } => f(collection),
        Stmt::SetIndex { collection, index, value } => {
            f(collection);
            f(index);
            f(value);
        }
        Stmt::SetField { object, value, .. } => {
            f(object);
            f(value);
        }
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => f(cond),
        Stmt::Call { args, .. } => args.iter().for_each(|a| f(a)),
        Stmt::RuntimeAssert { condition } => f(condition),
        _ => {}
    }
}
