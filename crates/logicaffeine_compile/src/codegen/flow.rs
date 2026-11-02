//! What the variables do after the statement being emitted: the facts that let the emitter MOVE a value into place
//! instead of copying it.
//!
//! [`crate::analysis::liveness`] computes, for every statement of every body, the variables live after it. While a
//! program is emitted that table is installed here ([`install`]), keyed by each statement's address in the arena the
//! analysis walked; a statement the emitter synthesized later has no entry, and nothing in it moves.
//!
//! While one statement is emitted, [`enter`] publishes its flow: the variables live after it, how many times it uses
//! each (reads and in-place changes; foreign code in it uses anything), and the variable it reassigns (`Set x to …`:
//! the old value of `x` is dead the moment the statement ends, whatever follows). [`moves_at_last_use`] answers from
//! it whether a variable named in the statement may be moved. A statement whose own expressions run more than once —
//! a loop header — publishes nothing, and a closure body suspends the flow ([`suspend`]): a value moved there would be
//! gone the next time it runs.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::thread::LocalKey;

use crate::analysis::liveness::stmt_key;
use crate::ast::stmt::Stmt;
use crate::intern::Symbol;

/// The flow of the statement being emitted.
pub(crate) struct Flow {
    live_after: Option<HashSet<Symbol>>,
    uses: Option<HashMap<Symbol, usize>>,
    reassigns: Option<Symbol>,
}

thread_local! {
    static LIVE_AFTER: RefCell<Option<Rc<HashMap<usize, HashSet<Symbol>>>>> = const { RefCell::new(None) };
    static FLOW: RefCell<Option<Flow>> = const { RefCell::new(None) };
}

/// Restores what a thread-local held before a scope replaced it, when the scope ends.
pub(crate) struct Scope<T: 'static> {
    key: &'static LocalKey<RefCell<Option<T>>>,
    prev: Option<Option<T>>,
}

impl<T: 'static> Drop for Scope<T> {
    fn drop(&mut self) {
        if let Some(prev) = self.prev.take() {
            self.key.with(|c| *c.borrow_mut() = prev);
        }
    }
}

pub(super) fn replace<T: 'static>(key: &'static LocalKey<RefCell<Option<T>>>, value: Option<T>) -> Scope<T> {
    Scope {
        key,
        prev: Some(key.with(|c| c.replace(value))),
    }
}

/// Install a program's per-statement liveness for the length of its emission.
pub(crate) fn install(
    table: HashMap<usize, HashSet<Symbol>>,
) -> Scope<Rc<HashMap<usize, HashSet<Symbol>>>> {
    replace(&LIVE_AFTER, Some(Rc::new(table)))
}

/// The variables live after `stmt`, when the installed analysis saw it.
pub(crate) fn live_after(stmt: &Stmt) -> Option<HashSet<Symbol>> {
    LIVE_AFTER.with(|t| {
        t.borrow()
            .as_ref()
            .and_then(|t| t.get(&stmt_key(stmt)).cloned())
    })
}

/// Publish `stmt`'s flow for the length of its emission.
pub(crate) fn enter(stmt: &Stmt) -> Scope<Flow> {
    let runs_once = matches!(
        stmt,
        Stmt::Let { .. }
            | Stmt::Set { .. }
            | Stmt::Show { .. }
            | Stmt::Return { .. }
            | Stmt::Call { .. }
            | Stmt::Push { .. }
            | Stmt::Add { .. }
            | Stmt::Remove { .. }
            | Stmt::SetIndex { .. }
            | Stmt::SetField { .. }
            | Stmt::Give { .. }
            | Stmt::If { .. }
            | Stmt::Inspect { .. }
    );
    if !runs_once {
        return replace(&FLOW, None);
    }
    let (uses, opaque) = crate::optimize::use_counts(stmt);
    let reassigns = match stmt {
        Stmt::Set { target, .. } => Some(*target),
        _ => None,
    };
    replace(
        &FLOW,
        Some(Flow {
            live_after: live_after(stmt),
            uses: (!opaque).then_some(uses),
            reassigns,
        }),
    )
}

/// Withdraw the flow for the length of a closure body.
pub(crate) fn suspend() -> Scope<Flow> {
    replace(&FLOW, None)
}

/// Whether the statement being emitted may move `sym`: it uses `sym` exactly `named` times — every one of them in the
/// construction asking, whose last mention moves — and either reassigns `sym` or is its last use.
pub(crate) fn moves_at_last_use(sym: Symbol, named: usize) -> bool {
    FLOW.with(|f| {
        f.borrow().as_ref().is_some_and(|flow| {
            flow.uses
                .as_ref()
                .is_some_and(|u| u.get(&sym) == Some(&named))
                && (flow.reassigns == Some(sym)
                    || flow
                        .live_after
                        .as_ref()
                        .is_some_and(|live| !live.contains(&sym)))
        })
    })
}
