//! Where `Break` and `Return` go in the Rust a statement is emitted as.
//!
//! A zone (`Inside a zone called …:`) is a block a `Break` or a `Return` inside it leaves, as the interpreter and the
//! VM leave it: a `Break` stops at the nearest loop or zone, and a `Return` at the nearest zone even with a loop
//! between, the program going on after the zone. A zone is emitted as a labeled block, so both become a `break` to
//! its label. A function or a closure starts with no boundary of its own: its exits are its own.

use std::cell::RefCell;

use super::flow::{replace, Scope};
use crate::ast::stmt::Stmt;

/// A block an exit can stop at.
#[derive(Clone, Copy)]
pub(super) enum Boundary {
    Loop,
    /// A zone, by how many boundaries enclose it.
    Zone(usize),
}

thread_local! {
    static BOUNDARIES: RefCell<Option<Vec<Boundary>>> = const { RefCell::new(None) };
}

/// Emit a function's or a closure's body, with no boundary around it, until the returned guard drops.
pub(super) fn function() -> Scope<Vec<Boundary>> {
    replace(&BOUNDARIES, Some(Vec::new()))
}

/// Emit `stmt` inside the boundary it makes, if it makes one, until the returned guard drops.
pub(super) fn enter(stmt: &Stmt) -> Option<Scope<Vec<Boundary>>> {
    if matches!(stmt, Stmt::FunctionDef { .. }) {
        return Some(function());
    }
    let mut stack = BOUNDARIES.with(|s| s.borrow().clone().unwrap_or_default());
    let boundary = match stmt {
        Stmt::While { .. } | Stmt::Repeat { .. } => Boundary::Loop,
        Stmt::Zone { .. } => Boundary::Zone(stack.len()),
        _ => return None,
    };
    stack.push(boundary);
    Some(replace(&BOUNDARIES, Some(stack)))
}

fn label(depth: usize) -> String {
    format!("__logos_zone_{depth}")
}

/// The label a `Break` leaves by: the nearest boundary's when it is a zone, `None` when it is a loop.
pub(super) fn break_label() -> Option<String> {
    BOUNDARIES.with(|s| match s.borrow().as_ref()?.last()? {
        Boundary::Zone(depth) => Some(label(*depth)),
        Boundary::Loop => None,
    })
}

/// The label a `Return` leaves by: the nearest zone's, past any loop inside it. `None` outside every zone.
pub(super) fn return_label() -> Option<String> {
    BOUNDARIES.with(|s| {
        s.borrow().as_ref()?.iter().rev().find_map(|b| match b {
            Boundary::Zone(depth) => Some(label(*depth)),
            Boundary::Loop => None,
        })
    })
}
