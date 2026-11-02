//! Searches over every natural: `Repeat for every natural n:`. Such a loop has no last pass, so
//! the engine writes the statements that give its outcome:
//!
//! - **the linear reading**: a body `If P(n): …` whose effects either end the search (`Break`,
//!   `Return`) or set variables to values that do not depend on `n`, with `P` built from linear
//!   terms in `n` and remainders of them, runs exactly at the naturals `searchCandidates` names —
//!   where `P`'s truth can first change — and that decides the search;
//! - **the provisional answer**: otherwise, or when the coefficients at run time are not ones the
//!   linear reading decides, the body runs on the first `K` naturals (`checking up to K cases`). A
//!   witness, a `Break` or a `Return` settles the outcome; a search that ran out of cases leaves
//!   every variable it sets as a provisional value carrying the cases checked and how often the
//!   value changed, and every answer a function returns after it is provisional too.

use logicaffeine_base::Symbol;

use super::build::{add, Build};
use super::clock::reads;
use super::linear::{invariant_in, linear, remainder};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};

/// What the linear reading of a predicate needs at run time: the terms whose sign change may change
/// its truth, and the remainders whose period it repeats with.
#[derive(Default)]
struct Reading<'a> {
    thresholds: Vec<(&'a Expr<'a>, &'a Expr<'a>)>,
    periods: Vec<(&'a Expr<'a>, &'a Expr<'a>)>,
}

fn read_predicate<'a>(
    p: &'a Expr<'a>,
    n: Symbol,
    assigned: &[Symbol],
    b: &Build<'a, '_>,
    out: &mut Reading<'a>,
) -> Option<()> {
    if invariant_in(p, n, assigned) {
        return Some(());
    }
    match p {
        Expr::Not { operand } => read_predicate(operand, n, assigned, b, out),
        Expr::BinaryOp {
            op: BinaryOpKind::And | BinaryOpKind::Or,
            left,
            right,
        } => {
            read_predicate(left, n, assigned, b, out)?;
            read_predicate(right, n, assigned, b, out)
        }
        Expr::BinaryOp {
            op:
                BinaryOpKind::Eq
                | BinaryOpKind::NotEq
                | BinaryOpKind::Lt
                | BinaryOpKind::Gt
                | BinaryOpKind::LtEq
                | BinaryOpKind::GtEq,
            left,
            right,
        } => {
            for (side, other) in [(*left, *right), (*right, *left)] {
                if let Some((term, m)) = remainder(side, n, assigned, b) {
                    if invariant_in(other, n, assigned) {
                        out.periods.push((term.a.unwrap_or_else(|| b.number(0)), m));
                        return Some(());
                    }
                    return None;
                }
            }
            let l = linear(left, n, assigned, b)?;
            let r = linear(right, n, assigned, b)?;
            let a = add(b, l.a, r.a, true).unwrap_or_else(|| b.number(0));
            let c = add(b, l.b, r.b, true).unwrap_or_else(|| b.number(0));
            out.thresholds.push((a, c));
            Some(())
        }
        _ => None,
    }
}

/// The shape the linear reading and the settled-by-a-witness answer need: a body `If P: effects`
/// with no `Otherwise`, whose effects end the search or set what `P` does not read to values that
/// do not depend on `n` — so the first witness alone decides the outcome.
struct Guarded<'a> {
    predicate: &'a Expr<'a>,
    effects: &'a [Stmt<'a>],
    /// The effects end the search (`Break` or `Return`), so only the first witness runs them.
    ends: bool,
    /// The variables the effects set.
    sets: Vec<Symbol>,
}

fn guarded<'a>(body: &'a [Stmt<'a>], n: Symbol) -> Option<Guarded<'a>> {
    let [Stmt::If {
        cond,
        then_block,
        else_block: None,
    }] = body
    else {
        return None;
    };
    let ends = then_block
        .iter()
        .any(|s| matches!(s, Stmt::Break | Stmt::Return { .. }));
    let mut sets = Vec::new();
    for s in then_block.iter() {
        match s {
            Stmt::Set { target, value } => {
                if !ends && !invariant_in(value, n, &[]) {
                    return None;
                }
                sets.push(*target);
            }
            _ if ends => {}
            _ => return None,
        }
    }
    let mut names = Vec::new();
    if reads(cond, &mut names).is_some() && names.iter().any(|s| sets.contains(s)) {
        return None;
    }
    Some(Guarded {
        predicate: cond,
        effects: then_block,
        ends,
        sets,
    })
}

/// Every variable the body sets, at any depth, and the ones it declares itself.
fn sets_and_locals(block: &[Stmt<'_>], sets: &mut Vec<Symbol>, locals: &mut Vec<Symbol>) {
    for s in block {
        match s {
            Stmt::Set { target, .. } => {
                if !sets.contains(target) {
                    sets.push(*target);
                }
            }
            Stmt::Let { var, .. } => locals.push(*var),
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                sets_and_locals(then_block, sets, locals);
                if let Some(b) = else_block {
                    sets_and_locals(b, sets, locals);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => {
                sets_and_locals(body, sets, locals)
            }
            _ => {}
        }
    }
}

/// `Break` statements that leave this loop (not an inner one), each preceded by `before`.
fn mark_breaks<'a>(block: &'a [Stmt<'a>], before: &Stmt<'a>, b: &Build<'a, '_>) -> &'a [Stmt<'a>] {
    let mut out = Vec::with_capacity(block.len());
    for s in block {
        match s {
            Stmt::Break => {
                out.push(before.clone());
                out.push(Stmt::Break);
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => out.push(Stmt::If {
                cond,
                then_block: mark_breaks(then_block, before, b),
                else_block: else_block.map(|e| mark_breaks(e, before, b)),
            }),
            Stmt::Splice { body } => out.push(Stmt::Splice {
                body: mark_breaks(body, before, b),
            }),
            other => out.push(other.clone()),
        }
    }
    b.block(out)
}

/// The statements that replace a search over every natural `n` checking `cases` cases, and the
/// name of the flag that is true once its outcome is settled.
pub(super) fn resolve<'a>(n: Symbol, cases: i64, body: &'a [Stmt<'a>], b: &mut Build<'a, '_>) -> (Stmt<'a>, Symbol) {
    let proved = b.fresh("proved");
    let mut out = vec![b.let_(proved, b.boolean(false))];
    let shape = guarded(body, n);
    let mut reading = Reading::default();
    let linear_reading = shape
        .as_ref()
        .and_then(|g| read_predicate(g.predicate, n, &g.sets, b, &mut reading));
    let provisional = match &shape {
        Some(g) => witness_search(n, cases, g, proved, b),
        None => limit_search(n, cases, body, proved, b),
    };
    match linear_reading {
        Some(()) => {
            let candidates = b.fresh("candidates");
            let thresholds = b.list(reading.thresholds.iter().map(|(a, c)| b.list(vec![a, c])).collect());
            let periods = b.list(reading.periods.iter().map(|(a, m)| b.list(vec![a, m])).collect());
            let found = b.call("searchCandidates", vec![thresholds, periods]);
            out.push(b.let_(candidates, found));
            let decided = b.binary(BinaryOpKind::Gt, b.length(b.ident(candidates)), b.number(0));
            let exact = vec![
                b.repeat(n, b.ident(candidates), body),
                b.set(proved, b.boolean(true)),
            ];
            out.push(b.if_(decided, exact, Some(provisional)));
        }
        None => out.extend(provisional),
    }
    (b.splice(out), proved)
}

/// The search run on the first `cases` naturals, stopping at the first witness; when none turns
/// up, what its effects set is provisional.
fn witness_search<'a>(n: Symbol, cases: i64, g: &Guarded<'a>, proved: Symbol, b: &mut Build<'a, '_>) -> Vec<Stmt<'a>> {
    let mut then = vec![b.set(proved, b.boolean(true))];
    then.extend(g.effects.iter().cloned());
    if !g.ends {
        then.push(Stmt::Break);
    }
    let run = b.block(vec![b.if_(g.predicate, then, None)]);
    let mut out = vec![b.repeat_upto(n, cases - 1, run)];
    let mut unsettled = Vec::new();
    for s in &g.sets {
        let value = b.call("provisional", vec![b.ident(*s), b.number(cases), b.number(0)]);
        unsettled.push(b.set(*s, value));
    }
    if !unsettled.is_empty() {
        out.push(b.if_(b.not(b.ident(proved)), unsettled, None));
    }
    out
}

/// The limit lemma's reading of a general search: the body run on the first `cases` naturals,
/// counting how often each variable it sets changes; a `Break` settles the outcome, and running out
/// of cases leaves each value provisional with its revisions.
fn limit_search<'a>(n: Symbol, cases: i64, body: &'a [Stmt<'a>], proved: Symbol, b: &mut Build<'a, '_>) -> Vec<Stmt<'a>> {
    let (mut sets, mut locals) = (Vec::new(), Vec::new());
    sets_and_locals(body, &mut sets, &mut locals);
    let tracked: Vec<Symbol> = sets.into_iter().filter(|s| !locals.contains(s)).collect();
    let mut out = Vec::new();
    let mut before = Vec::new();
    let mut revisions = Vec::new();
    for i in 0..tracked.len() {
        let (was, changed) = (b.fresh(&format!("was{i}")), b.fresh(&format!("revisions{i}")));
        out.push(b.let_(changed, b.number(0)));
        before.push(was);
        revisions.push(changed);
    }
    let settle = b.set(proved, b.boolean(true));
    let mut step: Vec<Stmt<'a>> = tracked
        .iter()
        .zip(&before)
        .map(|(s, was)| b.let_(*was, b.ident(*s)))
        .collect();
    step.extend(mark_breaks(body, &settle, b).iter().cloned());
    for ((s, was), changed) in tracked.iter().zip(&before).zip(&revisions) {
        let differs = b.binary(BinaryOpKind::NotEq, b.ident(*was), b.ident(*s));
        let count = b.set(*changed, b.binary(BinaryOpKind::Add, b.ident(*changed), b.number(1)));
        step.push(b.if_(differs, vec![count], None));
    }
    out.push(b.repeat_upto(n, cases - 1, b.block(step)));
    let mut unsettled = Vec::new();
    for (s, changed) in tracked.iter().zip(&revisions) {
        let value = b.call("provisional", vec![b.ident(*s), b.number(cases), b.ident(*changed)]);
        unsettled.push(b.set(*s, value));
    }
    if !unsettled.is_empty() {
        out.push(b.if_(b.not(b.ident(proved)), unsettled, None));
    }
    out
}

/// The number of cases a search names: `everyNatural(K)` with a whole-number literal `K`.
pub(super) fn cases_of(iterable: &Expr<'_>, every_natural: Symbol) -> Option<i64> {
    match iterable {
        Expr::Call { function, args } if *function == every_natural => match args.as_slice() {
            [Expr::Literal(Literal::Number(k))] if *k > 0 => Some(*k),
            _ => None,
        },
        _ => None,
    }
}
