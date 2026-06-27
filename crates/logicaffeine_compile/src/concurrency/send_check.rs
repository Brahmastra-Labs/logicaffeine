//! Send / escape analysis — the static soundness gate for the concurrency
//! memory model (Phase 4 of `FINISH_INTERPRETER.md`).
//!
//! The memory model is message-passing + CRDT: tasks have isolated heaps, and the
//! only cross-task sharing is channels (move semantics) and CRDT cells. This pass
//! rejects programs that violate that discipline. This first increment implements
//! the **data-race check** for `Simultaneously`/`Attempt all` blocks — branches
//! that share mutable state (the same variable, or the same pipe with conflicting
//! roles) would race once the branches genuinely run in parallel (M:N). The
//! remaining checks (spawned-body free-variable mutation, use-after-send,
//! non-sendable channel element types) and the wiring into every tier's run path
//! land alongside the interpreter lowering.

use super::classify::branches_share_mutable_state;
use crate::ast::stmt::Stmt;

/// A reason a program is rejected by the Send/escape analysis.
///
/// The concurrency AST nodes carry no spans (only `Escape`/`Require` do), so a
/// diagnostic carries its message; spans can be threaded through when the AST
/// grows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendDiagnostic {
    /// A Socratic explanation of the violation and how to fix it.
    pub message: String,
}

/// Run the Send/escape analysis over a whole program. An empty result means the
/// program respects the message-passing + CRDT discipline.
pub fn check_send_escape(stmts: &[Stmt]) -> Vec<SendDiagnostic> {
    let mut diags = Vec::new();
    check_block(stmts, &mut diags);
    diags
}

fn check_block(stmts: &[Stmt], diags: &mut Vec<SendDiagnostic>) {
    for stmt in stmts {
        match stmt {
            Stmt::Parallel { tasks } | Stmt::Concurrent { tasks } => {
                if branches_share_mutable_state(tasks) {
                    diags.push(SendDiagnostic {
                        message: "concurrent branches share mutable state across tasks — \
                                  pass it through a Pipe or make it a CRDT"
                            .to_string(),
                    });
                }
                check_block(tasks, diags);
            }
            Stmt::If { then_block, else_block, .. } => {
                check_block(then_block, diags);
                if let Some(eb) = else_block {
                    check_block(eb, diags);
                }
            }
            Stmt::While { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::Zone { body, .. }
            | Stmt::FunctionDef { body, .. } => check_block(body, diags),
            Stmt::Inspect { arms, .. } => {
                for arm in arms.iter() {
                    check_block(arm.body, diags);
                }
            }
            _ => {}
        }
    }
}
