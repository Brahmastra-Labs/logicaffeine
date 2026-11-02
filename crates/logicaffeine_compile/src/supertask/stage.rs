//! A supertask inside a supertask: the stage past every stage `ω·k`.
//!
//! The engine resolves the innermost supertask first, so the pass of an outer one may hold an inner
//! one already resolved, over locals the pass binds afresh. That part of the pass reads nothing the
//! passes carry from one to the next — the state the outer loop changes, its clock — so it computes
//! the same values at every pass: it runs once, before the outer supertask's end state, and the rest
//! of the pass is an ordinary map of the state it carries, which the outer strategies resolve. An outer
//! loop whose every pass runs an inner supertask of `⌈1/dt⌉` steps reaches stage `ω²`; its counter
//! ends at `1/dt^2`.
//!
//! A statement of the pass runs once ahead of the rest only when:
//!
//! - it touches nothing the passes carry — every name a statement of the pass writes that the pass
//!   does not bind, the clock among them — nor anything a statement left in the pass writes or
//!   binds; so every name it writes is a local of the pass or one it binds itself, and every name it
//!   reads is bound before the pass or ahead of it;
//! - no statement before it in the pass, left behind, touches a name it writes;
//! - and it does nothing but compute: assignments, branches, loops over the same, failures, and the
//!   exact extension's operations — no output, no call of the program's own functions, no `Break`
//!   or `Return` that would leave the pass.
//!
//! The pass arrives with every binding of a name it had already used renamed apart
//! ([`super::scope`]), so each name is one binding throughout it. Names are compared as names, as
//! the optimizer's walks compare them ([`Mentions`]).

use std::collections::HashSet;

use logicaffeine_base::Symbol;

use crate::ast::stmt::{Expr, Stmt};
use crate::optimize::mentions::Mentions;
use crate::optimize::writes::WriteSet;
use crate::semantics::builtins::builtin_from_name;

/// The statements of a pass that run once ahead of the rest, and the rest in their order, with the
/// position of `kept` (the clock's own step) among the rest; `None` when no statement runs ahead.
pub(super) struct Split<'a> {
    pub(super) ahead: Vec<Stmt<'a>>,
    pub(super) rest: Vec<Stmt<'a>>,
    pub(super) kept_at: usize,
}

/// Split the pass `body` of a supertask whose clock steps at `step_at`.
pub(super) fn split<'a>(body: &'a [Stmt<'a>], step_at: usize, interner: &crate::intern::Interner) -> Option<Split<'a>> {
    let ahead = ahead(body, &[], interner)?;
    if !ahead.iter().any(|a| *a) {
        return None;
    }
    let mut split = Split {
        ahead: Vec::new(),
        rest: Vec::new(),
        kept_at: 0,
    };
    for (i, s) in body.iter().enumerate() {
        if ahead[i] {
            split.ahead.push(s.clone());
        } else {
            if i == step_at {
                split.kept_at = split.rest.len();
            }
            split.rest.push(s.clone());
        }
    }
    Some(split)
}

/// Which statements of the pass `body` run once ahead of the rest, when the names `varying` change
/// too — from one run of the pass to the next, outside it; `None` when a statement's writes are
/// unknown.
pub(super) fn ahead(body: &[Stmt<'_>], varying: &[Symbol], interner: &crate::intern::Interner) -> Option<Vec<bool>> {
    let name = |s: Symbol| interner.resolve(s);
    let locals: HashSet<&str> = body
        .iter()
        .filter_map(|s| match s {
            Stmt::Let { var, .. } => Some(name(*var)),
            _ => None,
        })
        .collect();
    let writes: Vec<WriteSet> = body.iter().map(WriteSet::of_stmt).collect();
    if writes.iter().any(WriteSet::is_opaque) {
        return None;
    }
    let written: Vec<HashSet<&str>> = writes.iter().map(|w| w.symbols().map(name).collect()).collect();
    // What a statement writes outside itself: a name it binds inside a block of its own is its own,
    // and a name it binds for the rest of the pass is not.
    let external: Vec<HashSet<&str>> = (0..body.len())
        .map(|i| {
            let mut declared: HashSet<&str> = writes[i].declared().iter().map(|s| name(*s)).collect();
            if let Stmt::Let { var, .. } = &body[i] {
                declared.remove(name(*var));
            }
            written[i].iter().copied().filter(|w| !declared.contains(w)).collect()
        })
        .collect();
    let carried: HashSet<&str> = external
        .iter()
        .flatten()
        .copied()
        .filter(|s| !locals.contains(s))
        .chain(varying.iter().map(|s| name(*s)))
        .collect();
    let touched: Vec<HashSet<&str>> = body.iter().map(|s| Mentions::of_stmt(s).symbols().map(name).collect()).collect();
    let mut ahead: Vec<bool> = body.iter().map(|s| computes(s, interner)).collect();
    loop {
        let variant: HashSet<&str> = carried
            .iter()
            .copied()
            .chain((0..body.len()).filter(|i| !ahead[*i]).flat_map(|i| external[i].iter().copied()))
            .collect();
        let next: Vec<bool> = (0..body.len())
            .map(|i| {
                ahead[i]
                    && touched[i].is_disjoint(&variant)
                    && written[i]
                        .iter()
                        .all(|w| (0..i).filter(|j| !ahead[*j]).all(|j| !touched[j].contains(w)))
            })
            .collect();
        if next == ahead {
            break;
        }
        ahead = next;
    }
    Some(ahead)
}

/// Whether a statement only computes: assignments, branches and loops over the same, failures, and
/// expressions over the exact extension's operations.
fn computes(s: &Stmt<'_>, interner: &crate::intern::Interner) -> bool {
    let block = |b: &[Stmt<'_>]| b.iter().all(|s| computes(s, interner));
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => pure(value, interner),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => pure(cond, interner) && block(then_block) && else_block.is_none_or(|b| block(b)),
        Stmt::While { cond, body, .. } => pure(cond, interner) && block(body),
        Stmt::Splice { body } => block(body),
        Stmt::Push { value, collection } => pure(value, interner) && pure(collection, interner),
        Stmt::Fail { .. } => true,
        _ => false,
    }
}

/// Whether an expression only computes over values: arithmetic, comparisons, lists and the exact
/// extension's operations, never a call of the program's own functions.
pub(super) fn pure(e: &Expr<'_>, interner: &crate::intern::Interner) -> bool {
    let p = |e: &Expr<'_>| pure(e, interner);
    match e {
        Expr::Literal(_) | Expr::Identifier(_) => true,
        Expr::BinaryOp { left, right, .. } => p(left) && p(right),
        Expr::Not { operand } => p(operand),
        Expr::List(items) => items.iter().all(|i| p(i)),
        Expr::Index { collection, index } => p(collection) && p(index),
        Expr::Length { collection } => p(collection),
        Expr::Contains { collection, value } => p(collection) && p(value),
        Expr::Call { function, args } => {
            builtin_from_name(interner.resolve(*function)).is_some_and(|b| b.extension().is_some())
                && args.iter().all(|a| p(a))
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/supertask/stage/tests.rs"]
mod tests;
