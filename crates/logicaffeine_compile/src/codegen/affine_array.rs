//! Affine read-only array scalarization — delete a CSR-style offset array and
//! substitute its closed form.
//!
//! An array `A` built by a counted loop whose ONLY write to `A` is one
//! unconditional `push f(i) to A`, with `f(i)` an AFFINE function of the
//! induction variable (`i*5`, `i*3 + 7`, a constant) and the IV starting at `0`
//! and stepping by `1`, holds the invariant `A[p] = f(p)` for every position
//! `p`. If `A` is then never mutated, never aliased, and read only via `item _
//! of A` / `length of A` (all of which occur lexically AFTER the build), each
//! read is the pure arithmetic `f(k-1)`. Codegen can therefore delete the array
//! and its build push and substitute the closed form at every read.
//!
//! This turns graph_bfs's `adjStarts[v]` (a random load from a 24 MB CSR offset
//! array per dequeued vertex) into C's `v * 5` shift — eliminating both the
//! array and the cache miss.
//!
//! SOUNDNESS. The rewrite reproduces the EXACT i64 arithmetic the push computed
//! (`coeff * p + offset`), so wrapping/overflow semantics are identical. The
//! pass only ever removes an array whose every value it can recompute; any shape
//! it does not recognize — an in-place write, a conditional/multiple push, a
//! non-affine value, a non-unit step, an alias or escape, or a read before the
//! build completes — leaves `A` an ordinary `Vec`.

use crate::ast::stmt::{BinaryOpKind, Expr, Stmt};
use logicaffeine_base::intern::{Interner, Symbol};
use std::collections::{HashMap, HashSet};

use super::worklist::{
    bound_rust_expr, const_eval, for_each_stmt_expr, is_new_empty_int_seq, names_collection,
    ok_read_only, visit_idents,
};

/// How to emit a recognized affine array: the value at 0-based position `p` is
/// `coeff * p + offset`, and `length of A` is `trip`. (The IV is required to
/// start at 0, so position == iteration and these are the push's own constants.)
#[derive(Clone, Debug)]
pub(crate) struct AffineArrayInfo {
    pub coeff: i64,
    pub offset: i64,
    /// Rust expression for the element count (the value of `length of A`).
    pub trip: String,
}

/// Recognize the affine read-only arrays in a body. Conservative: every
/// condition in the module doc must hold, else the candidate is dropped.
pub(crate) fn detect_affine_arrays(
    body: &[Stmt],
    de_rc: &HashSet<Symbol>,
    interner: &Interner,
) -> HashMap<Symbol, AffineArrayInfo> {
    let mut out = HashMap::new();
    // Kill-switch (A/B and attribution), matching `LOGOS_NO_NARROW`.
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Affine) {
        return out;
    }
    for (di, stmt) in body.iter().enumerate() {
        let Stmt::Let { var, value, .. } = stmt else { continue };
        if !is_new_empty_int_seq(value) {
            continue;
        }
        let a = *var;
        // SOUNDNESS: only a uniquely-owned (de-Rc'd) sequence is safe to delete —
        // an aliased `LogosSeq` could be observed through the alias.
        if !de_rc.contains(&a) {
            continue;
        }
        if let Some(info) = analyze(a, di, body, interner) {
            out.insert(a, info);
        }
    }
    if !out.is_empty() {
        crate::optimize::mark_fired(crate::optimization::Opt::Affine);
    }
    out
}

/// A CONSTANT-TABLE local: a `Seq` built once from constant pushes and thereafter only indexed. Unlike
/// an affine array (deleted and replaced by a closed-form formula), its reads use a DYNAMIC index
/// (`item (i % 4) of r1`), so the table must be KEPT — but emitted as a stack array `[T; N]` (in
/// `.rodata`, zero heap, direct index) instead of a `Vec` rebuilt on every call. The MD5 per-round
/// shift tables. Keeps crypto fast IN LOGOS: the source is unchanged; the compiler stops allocating.
#[derive(Clone, Debug)]
pub(crate) struct ConstTableInfo {
    /// Rust element type of the array (e.g. `Word32`, `i64`).
    pub elem_ty: String,
    /// The constant element values, codegen'd to Rust, in push order.
    pub values: Vec<String>,
}

/// Recognize constant-table locals: `let mut V be a new Seq of T` immediately followed by ONLY
/// `Push <const> to V` (>= 1), with V uniquely owned (de-Rc'd) and thereafter read-only. `read_only_stmt`
/// (via `ok_read_only`) treats a BARE reference to V as an escape, so this matches only tables that are
/// purely indexed/length-read and never passed out — the safe subset; anything that escapes stays a
/// `Vec`. Requires an actual `item _ of V` read (a length-only table is left to the fill/length passes).
pub(crate) fn detect_const_tables(
    body: &[Stmt],
    all_stmts: &[Stmt],
    de_rc: &HashSet<Symbol>,
    borrow: &HashMap<Symbol, HashSet<usize>>,
    interner: &Interner,
) -> HashMap<Symbol, ConstTableInfo> {
    let mut out = HashMap::new();
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Affine) {
        return out;
    }
    let no_vars: HashSet<Symbol> = HashSet::new();
    for (di, stmt) in body.iter().enumerate() {
        let Stmt::Let { var, value, .. } = stmt else { continue };
        let a = *var;
        if !de_rc.contains(&a) {
            continue;
        }
        let (elem_ty, values, after): (String, Vec<String>, usize) = match value {
            Expr::New { type_name, type_args, init_fields }
                if interner.resolve(*type_name) == "Seq" && init_fields.is_empty() && type_args.len() == 1 =>
            {
                let mut vi = di + 1;
                let mut vals = Vec::new();
                while let Some(Stmt::Push { collection, value }) = body.get(vi) {
                    if names_collection(collection, a) && const_scalar_expr(value, interner) {
                        vals.push(super::expr::codegen_expr(value, interner, &no_vars));
                        vi += 1;
                    } else {
                        break;
                    }
                }
                if vals.is_empty() {
                    continue;
                }
                (super::types::codegen_type_expr(&type_args[0], interner), vals, vi)
            }
            // Function-call constant tables: `Let kk be md5Constants()` binds a niladic constant-table
            // function's result. Inline that function's constants here so the binding becomes a stack
            // array `[T; N]`, exactly like a local `new Seq` table — the md5 round constants passed by
            // `&[Word32]` borrow into `md5Compress`.
            Expr::Call { function, args } if args.is_empty() => {
                match niladic_const_table(*function, all_stmts, interner) {
                    Some((elem_ty, vals)) => (elem_ty, vals, di + 1),
                    None => continue,
                }
            }
            _ => continue,
        };
        if body[..di].iter().any(|s| mentions_anywhere(s, a)) {
            continue;
        }
        if !body[after..].iter().all(|s| const_table_read_only_stmt(s, a, borrow)) {
            continue;
        }
        if !body[after..].iter().any(|s| mentions_anywhere(s, a)) {
            continue;
        }
        out.insert(a, ConstTableInfo { elem_ty, values });
    }
    out
}

/// A fixed-size SCRATCH buffer: a loop-local `Seq` built by a COUNTED loop of statically-known trip
/// count (one push per iteration, per-iteration VALUES), thereafter read-only and non-escaping. Unlike a
/// constant table (constant values, hoisted once), a scratch buffer's fill reads per-iteration state, so
/// it is emitted IN PLACE as `let w: [T; N] = ::std::array::from_fn(|__k| { let j = LO + __k; …; <val> })`
/// — zero heap, one stack array — replacing a `Vec` that a per-entry loop otherwise reallocates. The
/// MD5 per-block message-schedule buffer `w` (16 words rebuilt each block from `padded`).
#[derive(Clone, Debug)]
pub(crate) struct ScratchInfo {
    /// Rust element type of the array (e.g. `Word32`).
    pub elem_ty: String,
    /// The compile-time trip count = element count.
    pub len: usize,
    /// The counted loop's inclusive start (the induction variable's first value).
    pub lo: i64,
}

/// Only element types whose Rust repr is a plain `Copy`/`Clone` scalar are lowered to a `[T; N]` stack
/// array (indexed reads `.clone()` cheaply, `&w` coerces to `&[T]`). This is the crypto leaf set — the
/// same shapes the constant-table pass already ships (`Word32`, `i64`, …).
fn stack_scalar_elem(t: &str) -> bool {
    matches!(
        t,
        "i64" | "u64" | "i32" | "u32" | "usize" | "isize" | "bool" | "char" | "u8"
            | "f64" | "f32" | "Word8" | "Word16" | "Word32" | "Word64"
    )
}

/// `w` is mentioned anywhere in `e`.
fn expr_mentions(e: &Expr, w: Symbol) -> bool {
    let mut hit = false;
    visit_idents(e, &mut |s| { if s == w { hit = true; } });
    hit
}

/// The fill loop's body must be a straight-line build: only `Let`s of non-`w`, non-loop-var locals
/// (computing intermediates) plus EXACTLY ONE unconditional top-level `Push <val> to w`, where neither
/// the pushed value nor any intermediate reads `w`. Any control flow, `Set`, extra `Push`, or self-read
/// disqualifies (from_fn evaluates the body once per index; a conditional or extra write would change the
/// count or order).
fn scratch_fill_body_ok(lbody: &[Stmt], w: Symbol, loop_var: Symbol) -> bool {
    let mut pushes = 0usize;
    for s in lbody {
        match s {
            Stmt::Push { collection: Expr::Identifier(c), value } if *c == w => {
                if expr_mentions(value, w) {
                    return false;
                }
                pushes += 1;
            }
            Stmt::Let { var, value, .. } => {
                if *var == w || *var == loop_var || expr_mentions(value, w) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    pushes == 1
}

/// Recognize fixed-size scratch buffers. Conservative: a `Let mutable w be a new Seq of T` (T a stack
/// scalar), uniquely owned (de-Rc'd), whose FIRST use is a constant-trip counted `Repeat for j from LO to
/// HI` that straight-line-fills it once per iteration, and which is thereafter read-only + non-escaping
/// (borrow-aware, via `const_table_read_only_stmt`). Anything else — a returned/escaping buffer, a
/// variable trip count, a post-fill mutation, a conditional push — is left an ordinary `Vec`.
pub(crate) fn detect_scratch_buffers(
    body: &[Stmt],
    de_rc: &HashSet<Symbol>,
    borrow: &HashMap<Symbol, HashSet<usize>>,
    interner: &Interner,
) -> HashMap<Symbol, ScratchInfo> {
    let mut out = HashMap::new();
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Affine) {
        return out;
    }
    // A scratch buffer's decl, fill loop, and reads all live in one block scope — and that block may be
    // NESTED (MD5's `w` is declared inside the per-block loop). Detect per-block and recurse into every
    // nested body; the flat `scratch_buffers` map + `variable_types` registration then reach it wherever
    // its fill loop is codegen'd.
    detect_scratch_in_block(body, de_rc, borrow, interner, &mut out);
    out
}

/// Per-block scratch-buffer detection (see `detect_scratch_buffers`), then recurse into nested blocks.
fn detect_scratch_in_block(
    body: &[Stmt],
    de_rc: &HashSet<Symbol>,
    borrow: &HashMap<Symbol, HashSet<usize>>,
    interner: &Interner,
    out: &mut HashMap<Symbol, ScratchInfo>,
) {
    use crate::ast::stmt::Pattern;
    for (di, stmt) in body.iter().enumerate() {
        let Stmt::Let { var, value, mutable: true, .. } = stmt else { continue };
        let w = *var;
        if !de_rc.contains(&w) {
            continue;
        }
        let elem_ty = match value {
            Expr::New { type_name, type_args, init_fields }
                if interner.resolve(*type_name) == "Seq" && init_fields.is_empty() && type_args.len() == 1 =>
            {
                super::types::codegen_type_expr(&type_args[0], interner)
            }
            _ => continue,
        };
        if !stack_scalar_elem(&elem_ty) {
            continue;
        }
        // The first statement after the decl that mentions `w` must be its fill loop; nothing may touch
        // `w` in between (a read of a partially-built buffer would change behavior).
        let Some(fi) = ((di + 1)..body.len()).find(|&k| mentions_anywhere(&body[k], w)) else { continue };
        let Stmt::Repeat { pattern: Pattern::Identifier(loop_var), iterable: Expr::Range { start, end }, body: lbody } = &body[fi] else { continue };
        let (Some(lo), Some(hi)) = (const_eval(start), const_eval(end)) else { continue };
        if hi < lo {
            continue;
        }
        let len = (hi - lo + 1) as usize;
        if !scratch_fill_body_ok(lbody, w, *loop_var) {
            continue;
        }
        // Read-only + non-escaping after the fill (borrow-aware: `&w` into a `&[T]` param is fine, a
        // return / field-store / re-push / bare alias is not), and actually used (else leave to DCE).
        if !body[fi + 1..].iter().all(|s| const_table_read_only_stmt(s, w, borrow)) {
            continue;
        }
        if !body[fi + 1..].iter().any(|s| mentions_anywhere(s, w)) {
            continue;
        }
        out.insert(w, ScratchInfo { elem_ty, len, lo });
    }
    // Recurse into nested blocks — a scratch buffer may be declared inside a loop / branch (MD5's `w`).
    for stmt in body {
        match stmt {
            Stmt::If { then_block, else_block, .. } => {
                detect_scratch_in_block(then_block, de_rc, borrow, interner, out);
                if let Some(eb) = else_block {
                    detect_scratch_in_block(eb, de_rc, borrow, interner, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                detect_scratch_in_block(body, de_rc, borrow, interner, out);
            }
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    detect_scratch_in_block(arm.body, de_rc, borrow, interner, out);
                }
            }
            _ => {}
        }
    }
}

// =============================================================================
// Fixed-size return-buffer scalarization (step 3b)
// =============================================================================
//
// A function whose result is a `Seq` built by a CONSTANT number N of pushes and
// returned — read-only, non-escaping except the return — has its return type
// changed to a stack array `[T; N]` returned BY VALUE (zero heap) instead of a
// heap `LogosSeq`. This fires ATOMICALLY with typing every call-site result
// variable as `[T; N]`: the accumulator `h` that `Set h to f(h, …)` reassigns
// each block rides the array end to end. `[T; N]` is `Copy`, so `h = f(&h, …)`
// needs no clone — cleaner than both the `Rc` bump (`LogosSeq`) and the deep copy
// (`Vec`) the reassign-through-self-borrow would otherwise force. The MD5
// `md5Compress` → `h` state.

/// Element type + length of an accepted fixed-size return buffer.
#[derive(Clone, Debug)]
pub(crate) struct ArrayReturnInfo {
    pub len: usize,
    pub elem_ty: String,
    /// The buffer is built by a LOOP over a fixed-size array (the digest shape) rather than straight-line
    /// pushes, so its `[T; N]` array is filled through a RUNTIME cursor (`out[__i]=…; __i+=1`).
    pub loop_built: bool,
}

/// Total number of `Return` statements anywhere in `body` (including nested blocks).
fn count_returns(body: &[Stmt]) -> usize {
    let mut n = 0;
    for s in body {
        match s {
            Stmt::Return { .. } => n += 1,
            Stmt::If { then_block, else_block, .. } => {
                n += count_returns(then_block);
                if let Some(eb) = else_block {
                    n += count_returns(eb);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => n += count_returns(body),
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    n += count_returns(arm.body);
                }
            }
            _ => {}
        }
    }
    n
}

/// If `body`'s LAST three regions are `Let mutable out be a new Seq of T`, then N ≥ 1 top-level `Push
/// <val> to out` (no `val` referencing `out`), then `Return out` — with `out` a stack scalar, the function's
/// ONLY return, and never mentioned before its decl — return `(N, rust_elem_ty)`.
fn fixed_return_buffer(body: &[Stmt], interner: &Interner) -> Option<(usize, String)> {
    let Some(Stmt::Return { value: Some(Expr::Identifier(out)) }) = body.last() else { return None };
    let out = *out;
    if count_returns(body) != 1 {
        return None;
    }
    let ret_idx = body.len().checked_sub(1)?;
    let mut i = ret_idx;
    while i > 0 {
        if let Stmt::Push { collection: Expr::Identifier(c), value } = &body[i - 1] {
            if *c == out && !expr_mentions(value, out) {
                i -= 1;
                continue;
            }
        }
        break;
    }
    let n = ret_idx - i;
    if n == 0 {
        return None;
    }
    let decl_idx = i.checked_sub(1)?;
    let elem_ty = match &body[decl_idx] {
        Stmt::Let { var, value: Expr::New { type_name, type_args, init_fields }, mutable: true, .. }
            if *var == out
                && interner.resolve(*type_name) == "Seq"
                && init_fields.is_empty()
                && type_args.len() == 1 =>
        {
            super::types::codegen_type_expr(&type_args[0], interner)
        }
        _ => return None,
    };
    if !stack_scalar_elem(&elem_ty) {
        return None;
    }
    if body[..decl_idx].iter().any(|s| mentions_anywhere(s, out)) {
        return None;
    }
    Some((n, elem_ty))
}

/// The fixed length of a LOCAL Seq built by K straight-line pushes at the top of `body` (a seeded fixed
/// array). `None` if `sym` is not a straight-line-seeded local (e.g. a param — runtime length).
fn local_seq_len(body: &[Stmt], sym: Symbol) -> Option<usize> {
    for (di, s) in body.iter().enumerate() {
        if let Stmt::Let { var, value: Expr::New { type_name, init_fields, .. }, mutable: true, .. } = s {
            if *var == sym && init_fields.is_empty() {
                let _ = type_name;
                let mut k = di + 1;
                let mut n = 0usize;
                while let Some(Stmt::Push { collection: Expr::Identifier(c), .. }) = body.get(k) {
                    if *c == sym {
                        n += 1;
                        k += 1;
                    } else {
                        break;
                    }
                }
                return if n > 0 { Some(n) } else { None };
            }
        }
    }
    None
}

/// A LOOP-built return buffer: `body`'s last three regions are `Let mutable out be new Seq of T`, a
/// `Repeat for X in H: <exactly K straight-line Push to out>` iterating a fixed-size LOCAL array `H`
/// (length M — its `[T; N]` size is `M*K`, statically known), then `Return out`. The streaming-hash digest
/// (`for word in h: push 4 bytes; return out`). Returns `(N, elem_ty)`.
fn loop_return_buffer(body: &[Stmt], interner: &Interner) -> Option<(usize, String)> {
    use crate::ast::stmt::Pattern;
    let Some(Stmt::Return { value: Some(Expr::Identifier(out)) }) = body.last() else { return None };
    let out = *out;
    if count_returns(body) != 1 {
        return None;
    }
    let ret_idx = body.len().checked_sub(1)?;
    let loop_idx = ret_idx.checked_sub(1)?;
    let Stmt::Repeat { pattern: Pattern::Identifier(_), iterable: Expr::Identifier(coll), body: lbody } = &body[loop_idx] else {
        return None;
    };
    let mut k = 0usize;
    for s in lbody.iter() {
        match s {
            Stmt::Push { collection: Expr::Identifier(c), value } if *c == out => {
                if expr_mentions(value, out) {
                    return None;
                }
                k += 1;
            }
            other => {
                if mentions_anywhere(other, out) {
                    return None;
                }
            }
        }
    }
    if k == 0 {
        return None;
    }
    let decl_idx = loop_idx.checked_sub(1)?;
    let elem_ty = match &body[decl_idx] {
        Stmt::Let { var, value: Expr::New { type_name, type_args, init_fields }, mutable: true, .. }
            if *var == out
                && interner.resolve(*type_name) == "Seq"
                && init_fields.is_empty()
                && type_args.len() == 1 =>
        {
            super::types::codegen_type_expr(&type_args[0], interner)
        }
        _ => return None,
    };
    if !stack_scalar_elem(&elem_ty) {
        return None;
    }
    if body[..decl_idx].iter().any(|s| mentions_anywhere(s, out)) {
        return None;
    }
    let m = local_seq_len(body, *coll)?;
    Some((m.checked_mul(k)?, elem_ty))
}

/// Total `Push … to x` count WITHIN one scope (nested blocks yes, nested function bodies NO — `x` is a
/// scope-local; another function's identically-named `x` is a different variable).
fn count_pushes_to(scope: &[Stmt], x: Symbol) -> usize {
    let mut n = 0;
    for s in scope {
        if let Stmt::Push { collection: Expr::Identifier(c), .. } = s {
            if *c == x {
                n += 1;
            }
        }
        match s {
            Stmt::If { then_block, else_block, .. } => {
                n += count_pushes_to(then_block, x);
                if let Some(eb) = else_block {
                    n += count_pushes_to(eb, x);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => n += count_pushes_to(body, x),
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    n += count_pushes_to(arm.body, x);
                }
            }
            _ => {}
        }
    }
    n
}

/// The declaration form of a caller result variable `x` and its seed size, found at the TOP LEVEL of its
/// scope. `Some((seed_count, is_seq_seed))`: a `Let mutable x be new Seq` seed (is_seq_seed=true,
/// seed_count = the immediately-following top-level pushes) or a direct `Let x be <array-fn call>` bind
/// (is_seq_seed=false, 0). `None` if `x` has no suitable top-level declaration in this scope.
fn caller_seed(
    scope: &[Stmt],
    x: Symbol,
    n: usize,
    cand: &HashMap<Symbol, ArrayReturnInfo>,
    interner: &Interner,
) -> Option<(usize, bool)> {
    // `x` may be declared at this block's top level OR nested (e.g. `Let d be f(...)` inside a hot loop),
    // so search this block then recurse. The declaration is unique, so the first `Let var==x` is `x`'s.
    for (di, s) in scope.iter().enumerate() {
        let Stmt::Let { var, value, .. } = s else { continue };
        if *var != x {
            continue;
        }
        return match value {
            Expr::Call { function, .. } if cand.get(function).map_or(false, |i| i.len == n) => Some((0, false)),
            Expr::New { type_name, type_args, init_fields }
                if interner.resolve(*type_name) == "Seq" && init_fields.is_empty() && type_args.len() == 1 =>
            {
                let mut sc = 0;
                let mut k = di + 1;
                while let Some(Stmt::Push { collection: Expr::Identifier(c), .. }) = scope.get(k) {
                    if *c == x {
                        sc += 1;
                        k += 1;
                    } else {
                        break;
                    }
                }
                Some((sc, true))
            }
            _ => None,
        };
    }
    for s in scope {
        let r = match s {
            Stmt::If { then_block, else_block, .. } => caller_seed(then_block, x, n, cand, interner)
                .or_else(|| else_block.as_ref().and_then(|eb| caller_seed(eb, x, n, cand, interner))),
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => caller_seed(body, x, n, cand, interner),
            Stmt::Inspect { arms, .. } => arms.iter().find_map(|a| caller_seed(a.body, x, n, cand, interner)),
            _ => None,
        };
        if r.is_some() {
            return r;
        }
    }
    None
}

/// Every use of `x` in `s` is array-compatible: a reassignment by an accepted array-return fn of the same
/// N (with `x` only at that call's borrow args), a seed push (all pushes are seed — count-verified by the
/// caller), an index/length read, a `for _ in x` iteration, or a `&[T]` borrow argument. A bare `x` (in
/// arithmetic, an owned arg, a `Return`, a store) is NOT array-compatible.
fn array_var_stmt_ok(
    s: &Stmt,
    x: Symbol,
    n: usize,
    cand: &HashMap<Symbol, ArrayReturnInfo>,
    borrow: &HashMap<Symbol, HashSet<usize>>,
) -> bool {
    match s {
        Stmt::FunctionDef { .. } => true,
        Stmt::Set { target, value } if *target == x => match value {
            Expr::Call { function, args } => {
                cand.get(function).map_or(false, |i| i.len == n) && call_args_ok(*function, args, x, borrow)
            }
            _ => false,
        },
        Stmt::Set { value, .. } => cok(value, x, borrow),
        Stmt::Let { var, value, .. } if *var == x => matches!(value, Expr::New { .. })
            || matches!(value, Expr::Call { function, .. } if cand.get(function).map_or(false, |i| i.len == n)),
        Stmt::Let { value, .. } => cok(value, x, borrow),
        Stmt::Push { collection: Expr::Identifier(c), value } if *c == x => !expr_mentions(value, x),
        Stmt::Push { collection, value } => !names_collection(collection, x) && cok(value, x, borrow),
        Stmt::Repeat { iterable, body, .. } => {
            (matches!(iterable, Expr::Identifier(s) if *s == x) || cok(iterable, x, borrow))
                && body.iter().all(|b| array_var_stmt_ok(b, x, n, cand, borrow))
        }
        Stmt::While { cond, body, .. } => {
            cok(cond, x, borrow) && body.iter().all(|b| array_var_stmt_ok(b, x, n, cand, borrow))
        }
        Stmt::If { cond, then_block, else_block } => {
            cok(cond, x, borrow)
                && then_block.iter().all(|b| array_var_stmt_ok(b, x, n, cand, borrow))
                && else_block.as_ref().map_or(true, |eb| eb.iter().all(|b| array_var_stmt_ok(b, x, n, cand, borrow)))
        }
        Stmt::Inspect { target, arms, .. } => {
            cok(target, x, borrow) && arms.iter().all(|a| a.body.iter().all(|b| array_var_stmt_ok(b, x, n, cand, borrow)))
        }
        Stmt::Return { value: Some(v) } => cok(v, x, borrow),
        Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => cok(object, x, borrow) && cok(recipient, x, borrow),
        Stmt::Call { function, args } => call_args_ok(*function, args, x, borrow),
        Stmt::RuntimeAssert { condition, .. } => cok(condition, x, borrow),
        _ => !mentions_anywhere(s, x),
    }
}

/// A caller result variable `x` (of an accepted array-return fn returning `[T; N]`) is array-compatible
/// end to end WITHIN its `scope`: a suitable seeded/direct declaration, no push beyond its seed, and every
/// use array-safe. Scoped so an identically-named local in another function never interferes.
fn array_var_ok(
    x: Symbol,
    n: usize,
    scope: &[Stmt],
    cand: &HashMap<Symbol, ArrayReturnInfo>,
    borrow: &HashMap<Symbol, HashSet<usize>>,
    interner: &Interner,
) -> bool {
    let Some((seed_count, is_seq_seed)) = caller_seed(scope, x, n, cand, interner) else { return false };
    if is_seq_seed && seed_count != n {
        return false;
    }
    if count_pushes_to(scope, x) != seed_count {
        return false;
    }
    scope.iter().all(|s| array_var_stmt_ok(s, x, n, cand, borrow))
}

/// Collect every `(x, f)` where a `Let/Set x to f(args)` in this scope binds a candidate array-return fn
/// `f` (nested blocks yes, nested function bodies NO — each function scope is analyzed separately).
fn collect_array_bindings(scope: &[Stmt], cand: &HashMap<Symbol, ArrayReturnInfo>, out: &mut Vec<(Symbol, Symbol)>) {
    for s in scope {
        let binding = match s {
            Stmt::Let { var, value, .. } => Some((*var, value)),
            Stmt::Set { target, value } => Some((*target, value)),
            _ => None,
        };
        if let Some((x, Expr::Call { function, .. })) = binding {
            if cand.contains_key(function) {
                out.push((x, *function));
            }
        }
        match s {
            Stmt::If { then_block, else_block, .. } => {
                collect_array_bindings(then_block, cand, out);
                if let Some(eb) = else_block {
                    collect_array_bindings(eb, cand, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => collect_array_bindings(body, cand, out),
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    collect_array_bindings(arm.body, cand, out);
                }
            }
            _ => {}
        }
    }
}

/// Flag (for removal) any candidate array-return fn used in a NON-binding (inline) position — its result
/// would be a `[T; N]` where a `LogosSeq` is expected. The single safe consuming position is a top-level
/// `Let/Set x to f(...)` binding (its ARGS are still inline positions).
fn flag_inline_array_calls(stmts: &[Stmt], cand: &HashMap<Symbol, ArrayReturnInfo>, remove: &mut HashSet<Symbol>) {
    // `symbol_appears_in_expr` visits CALLEE symbols (`f(...)`), which `expr_mentions`/`visit_idents` do
    // not — so an inline call like `splatAll(md5Constants())` correctly disqualifies `md5Constants`.
    let flag_expr = |e: &Expr, remove: &mut HashSet<Symbol>| {
        for f in cand.keys() {
            if super::detection::symbol_appears_in_expr(*f, e) {
                remove.insert(*f);
            }
        }
    };
    for s in stmts {
        match s {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => {
                if let Expr::Call { args, .. } = value {
                    for a in args {
                        flag_expr(a, remove);
                    }
                } else {
                    flag_expr(value, remove);
                }
            }
            Stmt::If { cond, then_block, else_block } => {
                flag_expr(cond, remove);
                flag_inline_array_calls(then_block, cand, remove);
                if let Some(eb) = else_block {
                    flag_inline_array_calls(eb, cand, remove);
                }
            }
            Stmt::While { cond, body, .. } => {
                flag_expr(cond, remove);
                flag_inline_array_calls(body, cand, remove);
            }
            Stmt::Repeat { iterable, body, .. } => {
                flag_expr(iterable, remove);
                flag_inline_array_calls(body, cand, remove);
            }
            Stmt::FunctionDef { body, .. } => flag_inline_array_calls(body, cand, remove),
            other => {
                for f in cand.keys() {
                    if super::detection::symbol_appears_in_stmt(*f, other) {
                        remove.insert(*f);
                    }
                }
            }
        }
    }
}

/// Recognize array-return functions (step 3b). A greatest fixpoint: start with every fixed-size-return-
/// buffer candidate accepted, then remove any whose result is used inline OR bound to a variable that is
/// not array-compatible end to end — which itself depends on the currently-accepted set (a reassignment
/// `Set x to g(...)` is array-safe only if `g` is still accepted). Monotone shrinking ⇒ converges.
pub(crate) fn collect_array_return_fns(
    stmts: &[Stmt],
    borrow: &HashMap<Symbol, HashSet<usize>>,
    interner: &Interner,
) -> HashMap<Symbol, ArrayReturnInfo> {
    let mut cand: HashMap<Symbol, ArrayReturnInfo> = HashMap::new();
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Unbox) {
        return cand;
    }
    for s in stmts {
        if let Stmt::FunctionDef { name, body, return_type: Some(rt), is_native: false, .. } = s {
            if super::detection::is_vec_type_expr(rt, interner) {
                if let Some((len, elem_ty)) = fixed_return_buffer(body, interner) {
                    cand.insert(*name, ArrayReturnInfo { len, elem_ty, loop_built: false });
                } else if let Some((len, elem_ty)) = loop_return_buffer(body, interner) {
                    cand.insert(*name, ArrayReturnInfo { len, elem_ty, loop_built: true });
                }
            }
        }
    }
    if cand.is_empty() {
        return cand;
    }
    // Analyze each variable scope independently: the top-level (Main) body and every function body. A
    // caller result variable is local to ONE scope, so `array_var_ok` never conflates two functions'
    // identically-named accumulators (`h`, `out`).
    let mut scopes: Vec<&[Stmt]> = vec![stmts];
    for s in stmts {
        if let Stmt::FunctionDef { body, .. } = s {
            scopes.push(body);
        }
    }
    loop {
        let mut remove: HashSet<Symbol> = HashSet::new();
        flag_inline_array_calls(stmts, &cand, &mut remove);
        for scope in &scopes {
            let mut bindings = Vec::new();
            collect_array_bindings(scope, &cand, &mut bindings);
            for (x, f) in &bindings {
                if remove.contains(f) {
                    continue;
                }
                let n = cand[f].len;
                if !array_var_ok(*x, n, scope, &cand, borrow, interner) {
                    remove.insert(*f);
                }
            }
        }
        if remove.is_empty() {
            break;
        }
        cand.retain(|f, _| !remove.contains(f));
    }
    if !cand.is_empty() {
        crate::optimize::mark_fired(crate::optimization::Opt::Unbox);
    }
    cand
}

/// The `[T; N]` type registrations for one function body under the accepted array-return set: the
/// function's OWN return buffer (when `this_fn_return` is `Some`) plus every caller result variable that a
/// `Let/Set x to f(...)` binds to an array-return fn. Each pairs a representative symbol with its Rust
/// array type; the caller registers the type for every name-matching symbol. The return buffer and the
/// seeded caller accumulators then ride the existing O3 `[T; N]` scalarization (stack decl + indexed fill).
pub(crate) fn array_var_types(
    body: &[Stmt],
    this_fn_return: Option<&ArrayReturnInfo>,
    array_return_fns: &HashMap<Symbol, ArrayReturnInfo>,
) -> Vec<(Symbol, String)> {
    let mut out = Vec::new();
    if let Some(info) = this_fn_return {
        if let Some(Stmt::Return { value: Some(Expr::Identifier(buf)) }) = body.last() {
            out.push((*buf, format!("[{}; {}]", info.elem_ty, info.len)));
        }
    }
    let mut bindings = Vec::new();
    collect_array_bindings(body, array_return_fns, &mut bindings);
    for (x, f) in bindings {
        if let Some(info) = array_return_fns.get(&f) {
            out.push((x, format!("[{}; {}]", info.elem_ty, info.len)));
        }
    }
    out
}

// =============================================================================
// Indexed-write fixed-size buffer scalarization (step 7)
// =============================================================================
//
// A local `Seq` ZERO-INITIALIZED to a constant size N by a counted loop, then
// mutated ONLY by indexed writes (`Set item i of buf to …`) — never pushed
// again, never escaping — is a fixed-size mutable buffer. It lowers to a stack
// array `[T; N]` (`[0; N]`, indexed stores) instead of a heap `Vec`. This is the
// memcpy-style buffer the streaming-hash 64-byte block wants: copy some slots,
// set the rest by index. (Scratch handles push-built read-only buffers; this
// handles zero-init + indexed-written ones — the complementary shape.)

#[derive(Clone, Debug)]
pub(crate) struct IndexedBufInfo {
    pub elem_ty: String,
    pub len: usize,
}

/// The loop body is exactly one `Push 0 to v` (a constant-zero fill).
fn is_zero_init_loop(body: &[Stmt], v: Symbol) -> bool {
    matches!(body, [Stmt::Push { collection: Expr::Identifier(c), value }]
        if *c == v && const_eval(value) == Some(0))
}

/// Any `Set item _ of v to _` (indexed write) in `s`, including nested blocks.
fn has_setindex_to(s: &Stmt, v: Symbol) -> bool {
    match s {
        Stmt::SetIndex { collection: Expr::Identifier(c), .. } => *c == v,
        Stmt::If { then_block, else_block, .. } => {
            then_block.iter().any(|x| has_setindex_to(x, v))
                || else_block.as_ref().map_or(false, |eb| eb.iter().any(|x| has_setindex_to(x, v)))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => body.iter().any(|x| has_setindex_to(x, v)),
        Stmt::Inspect { arms, .. } => arms.iter().any(|a| a.body.iter().any(|x| has_setindex_to(x, v))),
        _ => false,
    }
}

/// `v` is used only as a fixed-size buffer in `s`: written by `Set item _ of v` (its own indexed store),
/// read via `item _ of v` / `length of v`, never PUSHED (would grow it) and never escaping. Mirrors
/// `read_only_stmt` but permits the buffer's own indexed store.
fn indexed_buf_ok_stmt(s: &Stmt, v: Symbol) -> bool {
    match s {
        Stmt::SetIndex { collection: Expr::Identifier(c), index, value } if *c == v => {
            ok_read_only(index, v) && ok_read_only(value, v)
        }
        Stmt::SetIndex { collection, index, value } => {
            !names_collection(collection, v) && ok_read_only(index, v) && ok_read_only(value, v)
        }
        Stmt::Push { collection, value } => !names_collection(collection, v) && ok_read_only(value, v),
        Stmt::Pop { collection, .. } | Stmt::Remove { collection, .. } | Stmt::Add { collection, .. } => !names_collection(collection, v),
        Stmt::Let { var, value, .. } => *var != v && ok_read_only(value, v),
        Stmt::Set { target, value } => *target != v && ok_read_only(value, v),
        Stmt::SetField { object, value, .. } => ok_read_only(object, v) && ok_read_only(value, v),
        Stmt::If { cond, then_block, else_block } => {
            ok_read_only(cond, v)
                && then_block.iter().all(|x| indexed_buf_ok_stmt(x, v))
                && else_block.as_ref().map_or(true, |eb| eb.iter().all(|x| indexed_buf_ok_stmt(x, v)))
        }
        Stmt::While { cond, body, .. } => ok_read_only(cond, v) && body.iter().all(|x| indexed_buf_ok_stmt(x, v)),
        Stmt::Repeat { body, .. } => body.iter().all(|x| indexed_buf_ok_stmt(x, v)),
        Stmt::Return { value: Some(x) } => ok_read_only(x, v),
        Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => ok_read_only(object, v) && ok_read_only(recipient, v),
        Stmt::Call { args, .. } => args.iter().all(|x| ok_read_only(x, v)),
        Stmt::RuntimeAssert { condition, .. } => ok_read_only(condition, v),
        Stmt::Inspect { target, .. } => ok_read_only(target, v),
        _ => !mentions_anywhere(s, v),
    }
}

/// Recognize zero-init + indexed-write fixed-size buffers, at any nesting (the streaming-hash block is
/// declared inside the per-block loop). Conservative: a de-Rc'd `Let mutable V be new Seq of T` (T a stack
/// scalar) immediately followed by a constant-trip `Push 0 to V` loop, thereafter written only by its own
/// `SetIndex` and read, never pushed, never escaping — and actually indexed-written at least once.
pub(crate) fn detect_indexed_buffers(
    body: &[Stmt],
    de_rc: &HashSet<Symbol>,
    interner: &Interner,
) -> HashMap<Symbol, IndexedBufInfo> {
    let mut out = HashMap::new();
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Affine) {
        return out;
    }
    detect_indexed_in_block(body, de_rc, interner, &mut out);
    out
}

fn detect_indexed_in_block(
    block: &[Stmt],
    de_rc: &HashSet<Symbol>,
    interner: &Interner,
    out: &mut HashMap<Symbol, IndexedBufInfo>,
) {
    use crate::ast::stmt::Pattern;
    for (di, stmt) in block.iter().enumerate() {
        if let Stmt::Let { var, value: Expr::New { type_name, type_args, init_fields }, mutable: true, .. } = stmt {
            let v = *var;
            if de_rc.contains(&v)
                && interner.resolve(*type_name) == "Seq"
                && init_fields.is_empty()
                && type_args.len() == 1
            {
                let elem_ty = super::types::codegen_type_expr(&type_args[0], interner);
                if stack_scalar_elem(&elem_ty) {
                    if let Some(Stmt::Repeat { pattern: Pattern::Identifier(_), iterable: Expr::Range { start, end }, body: init_body }) = block.get(di + 1) {
                        if let (Some(lo), Some(hi)) = (const_eval(start), const_eval(end)) {
                            let rest = &block[di + 2..];
                            if hi >= lo
                                && is_zero_init_loop(init_body, v)
                                && !block[..di].iter().any(|s| mentions_anywhere(s, v))
                                && rest.iter().all(|s| indexed_buf_ok_stmt(s, v))
                                && rest.iter().any(|s| has_setindex_to(s, v))
                            {
                                out.insert(v, IndexedBufInfo { elem_ty, len: (hi - lo + 1) as usize });
                            }
                        }
                    }
                }
            }
        }
        match stmt {
            Stmt::If { then_block, else_block, .. } => {
                detect_indexed_in_block(then_block, de_rc, interner, out);
                if let Some(eb) = else_block {
                    detect_indexed_in_block(eb, de_rc, interner, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => detect_indexed_in_block(body, de_rc, interner, out),
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    detect_indexed_in_block(arm.body, de_rc, interner, out);
                }
            }
            _ => {}
        }
    }
}

/// The zero-init `Repeat … Push 0 to V` loop of an indexed buffer — dropped from codegen (the `[0; N]`
/// array literal the `Let` emits already zeroes every slot).
pub(crate) fn is_indexed_init_loop(body: &[Stmt], ctx_has: impl Fn(Symbol) -> bool) -> bool {
    matches!(body, [Stmt::Push { collection: Expr::Identifier(c), value }]
        if ctx_has(*c) && const_eval(value) == Some(0))
}

// =============================================================================
// Straight-line-push fixed-size buffer scalarization (step 8)
// =============================================================================
//
// A local `Seq` built by a fixed number K of STRAIGHT-LINE pushes (any values,
// unlike the constant-table pass), thereafter read-only + borrow-only + non-
// escaping, is a fixed-size buffer → `[T; K]` (the O3 `Let` emits `[0; K]` and
// each push becomes `buf[k] = expr`). The existing scalarizer (`collect_
// scalarizable_seqs`) disqualifies ANY var passed to a call — even a read-only
// borrow — so a `Let msg … Push×4; f(msg)` (the caller's small input buffer)
// stayed a heap `Vec`. This pass is BORROW-AWARE (reuses `const_table_read_only_
// stmt`), so `&msg` into a `&[T]` param keeps `msg` a stack array.

/// Recognize straight-line-push fixed-size buffers, at any nesting.
pub(crate) fn detect_straightline_buffers(
    body: &[Stmt],
    de_rc: &HashSet<Symbol>,
    borrow: &HashMap<Symbol, HashSet<usize>>,
    interner: &Interner,
) -> HashMap<Symbol, (String, usize)> {
    let mut out = HashMap::new();
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Scalarize) {
        return out;
    }
    detect_sl_in_block(body, de_rc, borrow, interner, &mut out);
    out
}

fn detect_sl_in_block(
    block: &[Stmt],
    de_rc: &HashSet<Symbol>,
    borrow: &HashMap<Symbol, HashSet<usize>>,
    interner: &Interner,
    out: &mut HashMap<Symbol, (String, usize)>,
) {
    for (di, stmt) in block.iter().enumerate() {
        if let Stmt::Let { var, value: Expr::New { type_name, type_args, init_fields }, mutable: true, .. } = stmt {
            let v = *var;
            if de_rc.contains(&v)
                && interner.resolve(*type_name) == "Seq"
                && init_fields.is_empty()
                && type_args.len() == 1
            {
                let elem_ty = super::types::codegen_type_expr(&type_args[0], interner);
                if stack_scalar_elem(&elem_ty) {
                    let mut k = di + 1;
                    let mut n = 0usize;
                    while let Some(Stmt::Push { collection: Expr::Identifier(c), value }) = block.get(k) {
                        if *c == v && !expr_mentions(value, v) {
                            n += 1;
                            k += 1;
                        } else {
                            break;
                        }
                    }
                    let rest = &block[k..];
                    if n > 0
                        && !block[..di].iter().any(|s| mentions_anywhere(s, v))
                        && rest.iter().all(|s| const_table_read_only_stmt(s, v, borrow))
                        && rest.iter().any(|s| mentions_anywhere(s, v))
                    {
                        out.insert(v, (elem_ty, n));
                    }
                }
            }
        }
        match stmt {
            Stmt::If { then_block, else_block, .. } => {
                detect_sl_in_block(then_block, de_rc, borrow, interner, out);
                if let Some(eb) = else_block {
                    detect_sl_in_block(eb, de_rc, borrow, interner, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => detect_sl_in_block(body, de_rc, borrow, interner, out),
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    detect_sl_in_block(arm.body, de_rc, borrow, interner, out);
                }
            }
            _ => {}
        }
    }
}

/// If `f` is a niladic function whose body is exactly `let mut V be a new Seq of T`, then only constant
/// `Push`es to V, then `Return V`, return `(rust_elem_ty, codegen'd_value_exprs)`.
fn niladic_const_table(f: Symbol, all_stmts: &[Stmt], interner: &Interner) -> Option<(String, Vec<String>)> {
    for s in all_stmts {
        let Stmt::FunctionDef { name, params, body, is_native: false, .. } = s else { continue };
        if *name != f || !params.is_empty() || body.len() < 2 {
            continue;
        }
        let (var, elem_te) = match &body[0] {
            Stmt::Let { var, value: Expr::New { type_name, type_args, init_fields }, .. }
                if interner.resolve(*type_name) == "Seq" && init_fields.is_empty() && type_args.len() == 1 =>
            {
                (*var, &type_args[0])
            }
            _ => return None,
        };
        match body.last() {
            Some(Stmt::Return { value: Some(Expr::Identifier(s)) }) if *s == var => {}
            _ => return None,
        }
        let no_vars: HashSet<Symbol> = HashSet::new();
        let mut vals = Vec::new();
        for st in &body[1..body.len() - 1] {
            match st {
                Stmt::Push { collection: Expr::Identifier(c), value } if *c == var && const_scalar_expr(value, interner) => {
                    vals.push(super::expr::codegen_expr(value, interner, &no_vars));
                }
                _ => return None,
            }
        }
        if vals.is_empty() {
            return None;
        }
        return Some((super::types::codegen_type_expr(elem_te, interner), vals));
    }
    None
}

/// A side-effect-free constant scalar: int literals, `wordN(...)` of constants, `Not`, and const arithmetic.
fn const_scalar_expr(e: &Expr, interner: &Interner) -> bool {
    use crate::ast::stmt::Literal;
    match e {
        Expr::Literal(Literal::Number(_)) => true,
        Expr::Not { operand } => const_scalar_expr(operand, interner),
        Expr::BinaryOp { left, right, .. } => const_scalar_expr(left, interner) && const_scalar_expr(right, interner),
        Expr::Call { function, args } => {
            matches!(interner.resolve(*function), "word8" | "word16" | "word32" | "word64")
                && args.iter().all(|a| const_scalar_expr(a, interner))
        }
        _ => false,
    }
}

/// Like `read_only_stmt`, but also permits `a` at a BORROW (`&[T]`) parameter position (`borrow[f]` index).
fn const_table_read_only_stmt(s: &Stmt, a: Symbol, borrow: &HashMap<Symbol, HashSet<usize>>) -> bool {
    match s {
        Stmt::Push { collection, value } => !names_collection(collection, a) && cok(value, a, borrow),
        Stmt::Pop { collection, .. } | Stmt::Remove { collection, .. } | Stmt::Add { collection, .. } => !names_collection(collection, a),
        Stmt::SetIndex { collection, index, value } => !names_collection(collection, a) && cok(index, a, borrow) && cok(value, a, borrow),
        Stmt::Let { var, value, .. } => *var != a && cok(value, a, borrow),
        Stmt::Set { target, value } => *target != a && cok(value, a, borrow),
        Stmt::SetField { object, value, .. } => cok(object, a, borrow) && cok(value, a, borrow),
        Stmt::If { cond, then_block, else_block } => {
            cok(cond, a, borrow)
                && then_block.iter().all(|x| const_table_read_only_stmt(x, a, borrow))
                && else_block.as_ref().map_or(true, |eb| eb.iter().all(|x| const_table_read_only_stmt(x, a, borrow)))
        }
        Stmt::While { cond, body, .. } => cok(cond, a, borrow) && body.iter().all(|x| const_table_read_only_stmt(x, a, borrow)),
        Stmt::Repeat { body, .. } => body.iter().all(|x| const_table_read_only_stmt(x, a, borrow)),
        Stmt::Return { value: Some(v) } => cok(v, a, borrow),
        Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => cok(object, a, borrow) && cok(recipient, a, borrow),
        Stmt::Call { function, args } => call_args_ok(*function, args, a, borrow),
        Stmt::RuntimeAssert { condition, .. } => cok(condition, a, borrow),
        Stmt::Inspect { target, .. } => cok(target, a, borrow),
        _ => !mentions_anywhere(s, a),
    }
}

fn call_args_ok(f: Symbol, args: &[&Expr], a: Symbol, borrow: &HashMap<Symbol, HashSet<usize>>) -> bool {
    let bset = borrow.get(&f);
    args.iter().enumerate().all(|(i, arg)| {
        if matches!(arg, Expr::Identifier(s) if *s == a) {
            bset.map_or(false, |b| b.contains(&i))
        } else {
            cok(arg, a, borrow)
        }
    })
}

fn cok(e: &Expr, a: Symbol, borrow: &HashMap<Symbol, HashSet<usize>>) -> bool {
    use crate::ast::stmt::StringPart;
    match e {
        Expr::Identifier(s) => *s != a,
        Expr::Index { collection, index } => {
            let coll_ok = matches!(&**collection, Expr::Identifier(s) if *s == a) || cok(collection, a, borrow);
            coll_ok && cok(index, a, borrow)
        }
        Expr::Length { collection } => matches!(&**collection, Expr::Identifier(s) if *s == a) || cok(collection, a, borrow),
        Expr::BinaryOp { left, right, .. } => cok(left, a, borrow) && cok(right, a, borrow),
        Expr::Not { operand } => cok(operand, a, borrow),
        Expr::Call { function, args } => call_args_ok(*function, args, a, borrow),
        Expr::CallExpr { callee, args } => cok(callee, a, borrow) && args.iter().all(|x| cok(x, a, borrow)),
        Expr::Copy { expr } | Expr::Give { value: expr } => cok(expr, a, borrow),
        Expr::Contains { collection, value } => !names_collection(collection, a) && cok(value, a, borrow),
        Expr::Slice { collection, start, end } => !names_collection(collection, a) && cok(start, a, borrow) && cok(end, a, borrow),
        Expr::Range { start, end } => cok(start, a, borrow) && cok(end, a, borrow),
        Expr::Union { left, right } | Expr::Intersection { left, right } => cok(left, a, borrow) && cok(right, a, borrow),
        Expr::List(items) | Expr::Tuple(items) => items.iter().all(|i| cok(i, a, borrow)),
        Expr::FieldAccess { object, .. } => cok(object, a, borrow),
        Expr::OptionSome { value } => cok(value, a, borrow),
        Expr::InterpolatedString(parts) => parts.iter().all(|p| match p {
            StringPart::Expr { value, .. } => cok(value, a, borrow),
            _ => true,
        }),
        _ => {
            let mut used = false;
            visit_idents(e, &mut |s| { if s == a { used = true; } });
            !used
        }
    }
}

struct AffineFit {
    coeff: i64,
    offset: i64,
    trip: String,
}

fn analyze(a: Symbol, di: usize, body: &[Stmt], interner: &Interner) -> Option<AffineArrayInfo> {
    // Find the build loop: the first top-level `While i < N` after the decl
    // whose body affinely builds `A`.
    let mut build: Option<(usize, AffineFit)> = None;
    for bi in (di + 1)..body.len() {
        let Stmt::While { cond, body: lbody, .. } = &body[bi] else { continue };
        if let Some(fit) = match_build_loop(a, cond, lbody, body, bi, interner) {
            build = Some((bi, fit));
            break;
        }
    }
    let (bi, fit) = build?;

    // No reference to `A` before the build completes (a read of a partially-built
    // array would change behavior). The decl itself names no expression, so it is
    // exempt.
    for (idx, s) in body.iter().enumerate() {
        if idx >= bi {
            break;
        }
        if idx == di {
            continue;
        }
        if mentions_anywhere(s, a) {
            return None;
        }
    }

    // Read-only after the build: every later reference is `item _ of A` /
    // `length of A`; any write, push, alias, or escape disqualifies.
    for s in &body[bi + 1..] {
        if !read_only_stmt(s, a) {
            return None;
        }
    }

    // The payoff of this pass is turning `item k of A` reads into arithmetic
    // (graph_bfs's `adjStarts[v]` → `v*5`). An array read ONLY via `length of A`
    // (e.g. a constant fill whose count is all that's used) is not our target —
    // leave it to the fill / length-hoist passes rather than deleting it here.
    if !reads_item_of(&body[bi + 1..], a) {
        return None;
    }
    // `length of A` substitutes the trip count — but the for-range loop-bound
    // path renders it via a context-free codegen (`arr.len()`) that never sees
    // our rewrite, so deleting `A` would dangle that `.len()`. Decline whenever
    // `length of A` is used (graph_bfs reads `adjStarts` only via `item`, so it
    // is unaffected). Strictly conservative: the array simply stays a `Vec`.
    if reads_length_of(body, a) {
        return None;
    }

    Some(AffineArrayInfo { coeff: fit.coeff, offset: fit.offset, trip: fit.trip })
}

/// `while i < N` (or `i <= N`) whose body is exactly: one unconditional affine
/// `push f(i) to A`, one `Set i to i + 1`, and statements touching neither `A`
/// nor `i`. The IV must start at 0 (so position == iteration).
fn match_build_loop(
    a: Symbol,
    cond: &Expr,
    lbody: &[Stmt],
    body: &[Stmt],
    bi: usize,
    interner: &Interner,
) -> Option<AffineFit> {
    let Expr::BinaryOp { op, left, right } = cond else { return None };
    let inclusive = match op {
        BinaryOpKind::Lt => false,
        BinaryOpKind::LtEq => true,
        _ => return None,
    };
    let Expr::Identifier(iv) = left else { return None };
    let iv = *iv;
    let n_str = bound_rust_expr(right, interner)?;

    // The trip count `length of A` substitutes the bound expression verbatim, so a
    // variable bound must hold its build-time value: reject if it is ever
    // reassigned (a literal bound is always stable).
    if let Expr::Identifier(bound_sym) = right {
        if is_set_anywhere(body, *bound_sym) {
            return None;
        }
    }

    if !iv_starts_at_zero(body, bi, iv) {
        return None;
    }

    let mut push_fit: Option<(i64, i64)> = None;
    let mut iv_increments = 0;
    for s in lbody {
        match s {
            // The build push of A.
            Stmt::Push { collection, value } if names_collection(collection, a) => {
                if push_fit.is_some() {
                    return None; // more than one push to A
                }
                push_fit = Some(extract_affine(value, iv)?);
            }
            // The IV step: must be exactly `i + 1`, exactly once.
            Stmt::Set { target, value } if *target == iv => {
                if !is_increment_by_one(value, iv) {
                    return None;
                }
                iv_increments += 1;
            }
            // Anything else must touch neither A (no read/write inside the build)
            // nor the IV (a shadowing rebind would break position == iteration).
            other => {
                if mentions_anywhere(other, a) || assigns_var(other, iv) {
                    return None;
                }
            }
        }
    }

    let (coeff, offset) = push_fit?;
    // This pass exists to turn `item k of A` into the slope arithmetic `coeff*k`
    // (graph_bfs's `adjStarts[v]` → `v*5`). A constant array (coeff == 0) is not a
    // CSR offset table — leave it to the fill / with_capacity passes, which other
    // optimizations and tests expect to handle it.
    if coeff == 0 {
        return None;
    }
    if iv_increments != 1 {
        return None;
    }

    // IV starts at 0 and steps by 1, so the element count is the trip count.
    let trip = if inclusive { format!("({} + 1)", n_str) } else { n_str };
    Some(AffineFit { coeff, offset, trip })
}

/// The nearest assignment to `iv` before the build loop sets it to literal 0.
fn iv_starts_at_zero(body: &[Stmt], bi: usize, iv: Symbol) -> bool {
    for idx in (0..bi).rev() {
        match &body[idx] {
            Stmt::Let { var, value, .. } if *var == iv => return const_eval(value) == Some(0),
            Stmt::Set { target, value } if *target == iv => return const_eval(value) == Some(0),
            _ => {}
        }
    }
    false
}

/// Reify `e` as `coeff * iv + offset` with constant `coeff`/`offset`, referencing
/// only `iv` and integer constants. `None` for any non-affine or other-variable
/// term (`i*i`, `i*stride`, `i + j`).
fn extract_affine(e: &Expr, iv: Symbol) -> Option<(i64, i64)> {
    if let Some(c) = const_eval(e) {
        return Some((0, c));
    }
    match e {
        Expr::Identifier(s) if *s == iv => Some((1, 0)),
        Expr::BinaryOp { op, left, right } => match op {
            BinaryOpKind::Add => {
                let (lc, lo) = extract_affine(left, iv)?;
                let (rc, ro) = extract_affine(right, iv)?;
                Some((lc.checked_add(rc)?, lo.checked_add(ro)?))
            }
            BinaryOpKind::Subtract => {
                let (lc, lo) = extract_affine(left, iv)?;
                let (rc, ro) = extract_affine(right, iv)?;
                Some((lc.checked_sub(rc)?, lo.checked_sub(ro)?))
            }
            BinaryOpKind::Multiply => {
                let l = extract_affine(left, iv)?;
                let r = extract_affine(right, iv)?;
                // Affine × affine is affine only when one factor is a constant.
                match (l, r) {
                    ((0, k), (c, o)) | ((c, o), (0, k)) => {
                        Some((c.checked_mul(k)?, o.checked_mul(k)?))
                    }
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// `e == iv + 1` (or `1 + iv`).
fn is_increment_by_one(e: &Expr, iv: Symbol) -> bool {
    let is_iv = |x: &Expr| matches!(x, Expr::Identifier(s) if *s == iv);
    matches!(e, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
        if (is_iv(left) && const_eval(right) == Some(1))
            || (const_eval(left) == Some(1) && is_iv(right)))
}

/// `s` is a `Let`/`Set` that assigns `v`.
fn assigns_var(s: &Stmt, v: Symbol) -> bool {
    matches!(s, Stmt::Let { var, .. } if *var == v)
        || matches!(s, Stmt::Set { target, .. } if *target == v)
}

/// `v` is reassigned by a `Set` anywhere in the body, including nested blocks.
/// (The initial `Let` definition does not count — only mutation.)
fn is_set_anywhere(body: &[Stmt], v: Symbol) -> bool {
    body.iter().any(|s| match s {
        Stmt::Set { target, .. } if *target == v => true,
        Stmt::If { then_block, else_block, .. } => {
            is_set_anywhere(then_block, v)
                || else_block.as_ref().map_or(false, |eb| is_set_anywhere(eb, v))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => is_set_anywhere(body, v),
        _ => false,
    })
}

/// `a` appears anywhere in the statement, including nested blocks.
fn mentions_anywhere(s: &Stmt, a: Symbol) -> bool {
    let mut found = false;
    for_each_stmt_expr(s, &mut |e| {
        visit_idents(e, &mut |sym| {
            if sym == a {
                found = true;
            }
        });
    });
    if found {
        return true;
    }
    match s {
        Stmt::If { then_block, else_block, .. } => {
            then_block.iter().any(|x| mentions_anywhere(x, a))
                || else_block.as_ref().map_or(false, |eb| eb.iter().any(|x| mentions_anywhere(x, a)))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
            body.iter().any(|x| mentions_anywhere(x, a))
        }
        _ => false,
    }
}

/// Does any expression in `stmts` (including nested blocks) read `item _ of a`?
fn reads_item_of(stmts: &[Stmt], a: Symbol) -> bool {
    stmts.iter().any(|s| {
        let mut hit = false;
        for_each_stmt_expr(s, &mut |e| {
            if expr_has_index_of(e, a) {
                hit = true;
            }
        });
        hit || match s {
            Stmt::If { then_block, else_block, .. } => {
                reads_item_of(then_block, a)
                    || else_block.as_ref().map_or(false, |eb| reads_item_of(eb, a))
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => reads_item_of(body, a),
            _ => false,
        }
    })
}

/// Does any expression in `stmts` (including nested blocks) read `length of a`?
fn reads_length_of(stmts: &[Stmt], a: Symbol) -> bool {
    stmts.iter().any(|s| {
        let mut hit = false;
        for_each_stmt_expr(s, &mut |e| {
            if expr_has_length_of(e, a) {
                hit = true;
            }
        });
        hit || match s {
            Stmt::If { then_block, else_block, .. } => {
                reads_length_of(then_block, a)
                    || else_block.as_ref().map_or(false, |eb| reads_length_of(eb, a))
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => reads_length_of(body, a),
            _ => false,
        }
    })
}

/// `length of a` appears anywhere in the expression tree.
fn expr_has_length_of(e: &Expr, a: Symbol) -> bool {
    match e {
        Expr::Length { collection } => names_collection(collection, a) || expr_has_length_of(collection, a),
        Expr::Index { collection, index } => expr_has_length_of(collection, a) || expr_has_length_of(index, a),
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            expr_has_length_of(left, a) || expr_has_length_of(right, a)
        }
        Expr::Not { operand } => expr_has_length_of(operand, a),
        Expr::Copy { expr } | Expr::Give { value: expr } | Expr::OptionSome { value: expr } => {
            expr_has_length_of(expr, a)
        }
        Expr::FieldAccess { object, .. } => expr_has_length_of(object, a),
        Expr::Contains { collection, value } => {
            expr_has_length_of(collection, a) || expr_has_length_of(value, a)
        }
        Expr::Slice { collection, start, end } => {
            expr_has_length_of(collection, a) || expr_has_length_of(start, a) || expr_has_length_of(end, a)
        }
        Expr::Call { args, .. } => args.iter().any(|x| expr_has_length_of(x, a)),
        Expr::CallExpr { callee, args } => {
            expr_has_length_of(callee, a) || args.iter().any(|x| expr_has_length_of(x, a))
        }
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|x| expr_has_length_of(x, a)),
        Expr::InterpolatedString(parts) => parts.iter().any(|p| {
            matches!(p, crate::ast::stmt::StringPart::Expr { value, .. } if expr_has_length_of(value, a))
        }),
        _ => false,
    }
}

/// `item _ of a` appears anywhere in the expression tree.
fn expr_has_index_of(e: &Expr, a: Symbol) -> bool {
    match e {
        Expr::Index { collection, index } => {
            names_collection(collection, a)
                || expr_has_index_of(collection, a)
                || expr_has_index_of(index, a)
        }
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            expr_has_index_of(left, a) || expr_has_index_of(right, a)
        }
        Expr::Not { operand } => expr_has_index_of(operand, a),
        Expr::Length { collection } => expr_has_index_of(collection, a),
        Expr::Copy { expr } | Expr::Give { value: expr } | Expr::OptionSome { value: expr } => {
            expr_has_index_of(expr, a)
        }
        Expr::FieldAccess { object, .. } => expr_has_index_of(object, a),
        Expr::Contains { collection, value } => {
            expr_has_index_of(collection, a) || expr_has_index_of(value, a)
        }
        Expr::Slice { collection, start, end } => {
            expr_has_index_of(collection, a) || expr_has_index_of(start, a) || expr_has_index_of(end, a)
        }
        Expr::Call { args, .. } => args.iter().any(|x| expr_has_index_of(x, a)),
        Expr::CallExpr { callee, args } => {
            expr_has_index_of(callee, a) || args.iter().any(|x| expr_has_index_of(x, a))
        }
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|x| expr_has_index_of(x, a)),
        Expr::InterpolatedString(parts) => parts.iter().any(|p| {
            matches!(p, crate::ast::stmt::StringPart::Expr { value, .. } if expr_has_index_of(value, a))
        }),
        _ => false,
    }
}

/// `a` is used read-only in `s`: only via `item _ of a` / `length of a`, never
/// written (push/setindex/pop/remove/add), never bare (aliased/escaped).
fn read_only_stmt(s: &Stmt, a: Symbol) -> bool {
    match s {
        Stmt::Push { collection, value } => !names_collection(collection, a) && ok_read_only(value, a),
        Stmt::Pop { collection, .. }
        | Stmt::Remove { collection, .. }
        | Stmt::Add { collection, .. } => !names_collection(collection, a),
        Stmt::SetIndex { collection, index, value } => {
            !names_collection(collection, a) && ok_read_only(index, a) && ok_read_only(value, a)
        }
        // A rebind of `a` itself (`Set a to …` / a shadowing `Let a be …`) means
        // the array is no longer the affine sequence we proved — disqualify.
        Stmt::Let { var, value, .. } => *var != a && ok_read_only(value, a),
        Stmt::Set { target, value } => *target != a && ok_read_only(value, a),
        Stmt::SetField { object, value, .. } => ok_read_only(object, a) && ok_read_only(value, a),
        Stmt::If { cond, then_block, else_block } => {
            ok_read_only(cond, a)
                && then_block.iter().all(|x| read_only_stmt(x, a))
                && else_block.as_ref().map_or(true, |eb| eb.iter().all(|x| read_only_stmt(x, a)))
        }
        Stmt::While { cond, body, .. } => {
            ok_read_only(cond, a) && body.iter().all(|x| read_only_stmt(x, a))
        }
        Stmt::Repeat { body, .. } => body.iter().all(|x| read_only_stmt(x, a)),
        Stmt::Return { value: Some(v) } => ok_read_only(v, a),
        Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => {
            ok_read_only(object, a) && ok_read_only(recipient, a)
        }
        Stmt::Call { args, .. } => args.iter().all(|x| ok_read_only(x, a)),
        Stmt::RuntimeAssert { condition, .. } => ok_read_only(condition, a),
        Stmt::Inspect { target, .. } => ok_read_only(target, a),
        _ => !mentions_anywhere(s, a),
    }
}
