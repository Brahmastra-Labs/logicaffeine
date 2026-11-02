//! Whether a program's pages talk to the relay — and so whether the backend serving them answers
//! `/_relay` itself.
//!
//! A page reaches the relay only through the server that served it: the browser dials `/_relay` on
//! the page's own origin. `largo serve` answers it; a deployed backend answered nothing, so every
//! `Publish`, message handler and `shared on` store died the moment an app left development. The
//! backend serving such pages now answers it from its own `Serve`: the interpreter for the pages
//! its program declares, a compiled binary for the `pages/` it embeds (`logos_assets::USES_RELAY`,
//! from [`source_uses_the_relay`]).

use crate::ast::component::{ComponentDef, ComponentKind, StoreMode, Trigger};
use crate::ast::stmt::{SelectBranch, Stmt, UiStmt};

/// Whether any page, component or store this program declares talks to the relay: a
/// `## Store … shared on`, a `When a message arrives on …` handler, or a `Publish … on …` in any
/// action or handler, at any depth.
pub fn pages_use_the_relay(stmts: &[Stmt]) -> bool {
    stmts
        .iter()
        .any(|s| matches!(s, Stmt::ComponentDef(def) if component_uses_the_relay(def)))
}

/// [`pages_use_the_relay`] for a program's source — a page file a site bundles. A source that does
/// not parse uses nothing (its build refuses it on its own terms).
pub fn source_uses_the_relay(source: &str) -> bool {
    crate::ui_bridge::with_parsed_program(source, |parsed, _| {
        parsed.is_ok_and(|(stmts, _, _)| pages_use_the_relay(stmts))
    })
}

/// Whether one declared page, component or store talks to the relay (see [`pages_use_the_relay`]).
pub fn component_uses_the_relay(def: &ComponentDef) -> bool {
    matches!(
        def.kind,
        ComponentKind::Store {
            mode: StoreMode::Shared { .. }
        }
    ) || def
        .events
        .iter()
        .any(|h| matches!(h.trigger, Trigger::Message { .. }) || publishes(h.body))
        || def.actions.iter().any(|a| publishes(a.body))
}

/// Whether these statements, at any depth, `Publish` on a topic.
fn publishes(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Ui(UiStmt::Publish { .. }) => true,
        Stmt::If {
            then_block,
            else_block,
            ..
        } => publishes(then_block) || else_block.is_some_and(publishes),
        Stmt::While { body, .. }
        | Stmt::Repeat { body, .. }
        | Stmt::Zone { body, .. }
        | Stmt::Splice { body } => publishes(body),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => publishes(tasks),
        Stmt::Inspect { arms, .. } => arms.iter().any(|a| publishes(a.body)),
        Stmt::Select { branches } => branches.iter().any(|b| match b {
            SelectBranch::Receive { body, .. } | SelectBranch::Timeout { body, .. } => {
                publishes(body)
            }
        }),
        _ => false,
    })
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/analysis/relay/tests.rs"]
mod tests;
