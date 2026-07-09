//! Concurrency determinacy model + classifier (Phase 0 of `work/FINISH_INTERPRETER.md`).
//!
//! This module is the single source of truth for the language's concurrency
//! *memory model* and the *determinacy classifier* that labels a program as
//! belonging to the determinate (Kahn-deterministic) or nondeterminate fragment.
//! The classifier is consumed by AOT mode selection (Phase 8) and translation
//! validation (Phase 11), so both agree on one boundary.

pub mod bridge;
pub mod driver;
pub mod vm_driver;
pub mod channel;
pub mod classify;
pub mod fec;
pub mod marshal;
pub mod model;
pub(crate) mod net_inbox;
pub mod pnp;
pub mod send_check;
pub mod stream;

use crate::ast::stmt::Stmt;

/// Does the program use Go-like concurrency (channels / tasks / select) that must
/// run on the scheduler-driven interpreter path?
pub fn uses_scheduler(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_uses_scheduler)
}

fn stmt_uses_scheduler(s: &Stmt) -> bool {
    match s {
        Stmt::LaunchTask { .. }
        | Stmt::LaunchTaskWithHandle { .. }
        | Stmt::CreatePipe { .. }
        | Stmt::SendPipe { .. }
        | Stmt::ReceivePipe { .. }
        | Stmt::TrySendPipe { .. }
        | Stmt::TryReceivePipe { .. }
        | Stmt::Select { .. }
        | Stmt::StopTask { .. } => true,
        Stmt::If { then_block, else_block, .. } => {
            then_block.iter().any(stmt_uses_scheduler)
                || else_block.map_or(false, |b| b.iter().any(stmt_uses_scheduler))
        }
        Stmt::While { body, .. }
        | Stmt::Repeat { body, .. }
        | Stmt::Zone { body, .. }
        | Stmt::FunctionDef { body, .. } => body.iter().any(stmt_uses_scheduler),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            tasks.iter().any(stmt_uses_scheduler)
        }
        Stmt::Inspect { arms, .. } => {
            arms.iter().any(|a| a.body.iter().any(stmt_uses_scheduler))
        }
        _ => false,
    }
}

pub use classify::{branches_independent, branches_share_mutable_state, classify_program};
pub use marshal::{materialize, rebuild, MarshalError};
pub use model::{Determinacy, NondetKind, NondetWitness};
pub use net_inbox::{net_is_offline, set_net_offline};
pub use send_check::{check_send_escape, SendDiagnostic};
