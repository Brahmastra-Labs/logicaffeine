//! The caller-gate shared by `Opt::TextSliceReturn` and `Opt::ByteSliceReturn`.
//!
//! Both passes rewrite a trim-family function to return a BORROWED sub-slice of one of its
//! parameters (`-> &'a str`, `-> &'a [i64]`) instead of building an owned copy. That is only sound
//! while no caller needs an owned value back, so both passes need one question answered about the
//! whole program: does this function's result ever reach a position that owns it?
//!
//! # Why this is counted rather than blacklisted
//!
//! The gate this replaces answered by listing the statements that own their value — `Let`, `Set`,
//! `SetField`, `Push`, `Add`, `Return`, `Give` — and treating every other position as safe. Its own
//! note said so: *"Inline uses (`length of f(s)`, a condition, an arg) OK."* Two of those three are
//! genuinely safe. **An argument is not**, unless the callee takes that parameter by borrow — and
//! `## To startsWith (s: Text) and (p: Text)` does not.
//!
//! `assets/std/web.lg` writes exactly that, inside a condition:
//!
//! ```text
//! If startsWith(trim(body), brace):
//! ```
//!
//! `bodyParam` is pulled into every `## Route` program, so on 2026-09-11 every LOGOS web server
//! failed to build with `E0308: expected String, found &str` — `todo_deploy_e2e`,
//! `real_site_deploy_lock` and `json_decode_deploys_lock`, ~991s of the baseline, each ending in
//! *"the generated Rust failed to compile — this is a LOGOS compiler bug, not an error in your
//! program"*. The old walker could not even see the call: `flag` inspected only the TOP-LEVEL
//! expression of a statement, so an argument was invisible, and the `Stmt::If` arm recursed into the
//! blocks while never looking at the CONDITION.
//!
//! So the polarity is inverted here, because for a soundness gate that is the only polarity that
//! degrades safely:
//!
//! * **Total occurrences must be complete.** They come from [`known_calls::harvest_calls`], the one
//!   exhaustive whole-program walker — its no-wildcard matches fail the build when a new `Stmt` or
//!   `Expr` variant appears, so a position can never be silently missed.
//! * **Safe occurrences may be incomplete.** They are recognised by [`count_borrow_safe`] below.
//!
//! A candidate survives only when every occurrence is a recognised safe one. Anything new, unusual,
//! or unreached therefore *declines the optimization* — the failure direction is a slower program,
//! never a miscompiled one. That is the opposite of what happened here, where the gate's own comment
//! predicted the outcome ("any missed escape surfaces as a LOUD `&str`-vs-`String` compile error")
//! and shipped it anyway.

use std::collections::{HashMap, HashSet};

use crate::analysis::known_calls;
use crate::ast::stmt::{BinaryOpKind, Expr, Stmt};
use crate::intern::Symbol;

/// Parameter indices a function takes by borrow, as assembled by the program codegen.
pub(crate) type BorrowedParams = HashMap<Symbol, HashSet<usize>>;

/// The candidates whose borrowed-slice return survives whole-program use.
///
/// `cand` maps a candidate function to its source-parameter index; `borrowed` is the caller's
/// borrow-parameter map, which decides whether an argument position owns its value. Returns the
/// subset of `cand` that is safe to lower to a borrowed return.
pub(crate) fn surviving_candidates(
    stmts: &[Stmt],
    cand: &HashMap<Symbol, usize>,
    borrowed: &BorrowedParams,
) -> HashSet<Symbol> {
    if cand.is_empty() {
        return HashSet::new();
    }
    let total = count_all_calls(stmts, cand);
    let mut safe: HashMap<Symbol, usize> = HashMap::new();
    for s in stmts {
        count_borrow_safe(s, cand, borrowed, &mut safe);
    }
    cand.keys()
        .copied()
        .filter(|f| safe.get(f).copied().unwrap_or(0) >= total.get(f).copied().unwrap_or(0))
        .collect()
}

/// Every occurrence of a candidate call in the program — COMPLETE, by construction.
///
/// `harvest_calls` records one `CallSite` per `Expr::Call` node anywhere (it recurses into arguments
/// after pushing the site), and its walkers match without a wildcard arm, so this census is the part
/// of the gate that cannot silently miss a position.
fn count_all_calls(stmts: &[Stmt], cand: &HashMap<Symbol, usize>) -> HashMap<Symbol, usize> {
    let mut total: HashMap<Symbol, usize> = HashMap::new();
    for site in &known_calls::harvest_calls(stmts).calls {
        if cand.contains_key(&site.function) {
            *total.entry(site.function).or_insert(0) += 1;
        }
    }
    total
}

/// Occurrences in a position that provably never materialises an owned value. Deliberately partial:
/// an unrecognised position simply is not counted, which drops the candidate.
fn count_borrow_safe(
    stmt: &Stmt,
    cand: &HashMap<Symbol, usize>,
    borrowed: &BorrowedParams,
    safe: &mut HashMap<Symbol, usize>,
) {
    // `Show f(s)` emits `show(&f(s))` — read-only, and the one position the trim-family locks use.
    if let Stmt::Show { object, .. } = stmt {
        credit(object, cand, safe);
    }
    for e in stmt_exprs(stmt) {
        scan_expr(e, cand, borrowed, safe);
    }
    for block in stmt_blocks(stmt) {
        for s in block {
            count_borrow_safe(s, cand, borrowed, safe);
        }
    }
}

fn scan_expr(
    e: &Expr,
    cand: &HashMap<Symbol, usize>,
    borrowed: &BorrowedParams,
    safe: &mut HashMap<Symbol, usize>,
) {
    match e {
        // `length of f(s)` → `.len()` on the borrowed slice.
        Expr::Length { collection } => credit(collection, cand, safe),
        // A comparison borrows both sides (`&str` and `String` compare in either order).
        Expr::BinaryOp { op, left, right }
            if matches!(
                op,
                BinaryOpKind::Eq
                    | BinaryOpKind::NotEq
                    | BinaryOpKind::Lt
                    | BinaryOpKind::LtEq
                    | BinaryOpKind::Gt
                    | BinaryOpKind::GtEq
            ) =>
        {
            credit(left, cand, safe);
            credit(right, cand, safe);
        }
        // An argument is safe only where the CALLEE takes that parameter by borrow. This is the
        // distinction the old gate collapsed, and the whole reason every `## Route` program broke.
        Expr::Call { function, args } => {
            let takes = borrowed.get(function);
            for (i, arg) in args.iter().enumerate() {
                if takes.is_some_and(|b| b.contains(&i)) {
                    credit(arg, cand, safe);
                }
            }
        }
        _ => {}
    }
    for child in expr_children(e) {
        scan_expr(child, cand, borrowed, safe);
    }
}

fn credit(e: &Expr, cand: &HashMap<Symbol, usize>, safe: &mut HashMap<Symbol, usize>) {
    if let Expr::Call { function, .. } = e {
        if cand.contains_key(function) {
            *safe.entry(*function).or_insert(0) += 1;
        }
    }
}

/// Sub-expressions, for reaching nested positions. Partial by the module's contract.
fn expr_children<'a>(e: &'a Expr<'a>) -> Vec<&'a Expr<'a>> {
    match e {
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right } => vec![left, right],
        Expr::Not { operand } => vec![operand],
        Expr::Index { collection, index } => vec![collection, index],
        Expr::Slice {
            collection,
            start,
            end,
        } => vec![collection, start, end],
        Expr::Length { collection } => vec![collection],
        Expr::Contains { collection, value } => vec![collection, value],
        Expr::Range { start, end } => vec![start, end],
        Expr::OptionSome { value } => vec![value],
        Expr::Copy { expr } => vec![expr],
        Expr::Give { value } => vec![value],
        Expr::WithCapacity { value, capacity } => vec![value, capacity],
        Expr::FieldAccess { object, .. } => vec![object],
        Expr::Call { args, .. } => args.to_vec(),
        Expr::CallExpr { callee, args } => std::iter::once(*callee)
            .chain(args.iter().copied())
            .collect(),
        Expr::List(elems) | Expr::Tuple(elems) => elems.to_vec(),
        Expr::New { init_fields, .. } => init_fields.iter().map(|(_, v)| *v).collect(),
        Expr::NewVariant { fields, .. } => fields.iter().map(|(_, v)| *v).collect(),
        _ => Vec::new(),
    }
}

/// The expressions a statement holds directly. Partial by the module's contract.
fn stmt_exprs<'a>(s: &'a Stmt<'a>) -> Vec<&'a Expr<'a>> {
    match s {
        Stmt::Let { value, .. }
        | Stmt::Set { value, .. }
        | Stmt::SetField { value, .. }
        | Stmt::Push { value, .. }
        | Stmt::Add { value, .. } => vec![value],
        Stmt::Return { value: Some(v) } => vec![v],
        Stmt::Give { object, .. } | Stmt::Show { object, .. } => vec![object],
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => vec![cond],
        Stmt::Call { args, .. } => args.to_vec(),
        _ => Vec::new(),
    }
}

/// The nested blocks a statement holds. Partial by the module's contract.
fn stmt_blocks<'a>(s: &'a Stmt<'a>) -> Vec<&'a [Stmt<'a>]> {
    match s {
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            let mut v: Vec<&[Stmt]> = vec![then_block];
            if let Some(e) = else_block {
                v.push(e);
            }
            v
        }
        Stmt::While { body, .. }
        | Stmt::Repeat { body, .. }
        | Stmt::During { body, .. }
        | Stmt::Zone { body, .. }
        | Stmt::Splice { body }
        | Stmt::Reversibly { body }
        | Stmt::FunctionDef { body, .. } => vec![body],
        _ => Vec::new(),
    }
}
