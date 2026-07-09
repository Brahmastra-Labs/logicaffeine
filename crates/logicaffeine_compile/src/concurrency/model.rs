//! Concurrency memory model — the normative determinacy table.
//!
//! See `docs/CONCURRENCY_MODEL.md` and `work/FINISH_INTERPRETER.md` §3 for the full
//! specification. The short version:
//!
//! - **Determinate fragment** = `LaunchTask` + FIFO `Pipe`/`Send`/`Receive` +
//!   data-independent `Concurrent`/`Parallel` + CRDT shared state. By Kahn's
//!   determinacy theorem (Kahn Process Networks, 1974) the observable output of
//!   such a program is *scheduling-independent*.
//! - **Nondeterminate fragment** = the moment a program reaches a `Select`
//!   (`Await the first of:`), an `After` timeout branch, a `Try to send/receive`,
//!   or a `Stop` it can observe *which* event won a race — a function of timing,
//!   not of channel histories. Those four constructs are the determinacy frontier.

use crate::ast::stmt::{SelectBranch, Stmt};

/// Whole-program determinacy verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Determinacy {
    /// No reachable construct can introduce scheduling-dependent nondeterminism;
    /// output is a pure function of input (Kahn determinacy + CRDT convergence).
    Determinate,
    /// At least one nondeterminism source is present.
    Nondeterminate {
        /// The constructs that forced nondeterminism, in source order.
        witnesses: Vec<NondetWitness>,
    },
}

impl Determinacy {
    /// True iff the program is in the determinate fragment.
    pub fn is_determinate(&self) -> bool {
        matches!(self, Determinacy::Determinate)
    }

    /// The kinds of nondeterminism present (empty when determinate).
    pub fn nondet_kinds(&self) -> Vec<NondetKind> {
        match self {
            Determinacy::Determinate => Vec::new(),
            Determinacy::Nondeterminate { witnesses } => witnesses.iter().map(|w| w.kind).collect(),
        }
    }
}

/// A single nondeterminism source found in the program.
///
/// The concurrency AST nodes do not carry source spans (only `Escape`/`Require`
/// do), so a witness records the *kind* of construct. Spans can be threaded
/// through here when the AST grows them, without changing the classifier's shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NondetWitness {
    /// The construct that introduced nondeterminism.
    pub kind: NondetKind,
}

/// The constructs that force nondeterminism — the determinate↔nondeterminate frontier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NondetKind {
    /// `Await the first of:` — nondeterministic choice over the ready branch set.
    Select,
    /// `After N seconds:` — a timer event racing the other branches.
    AfterTimer,
    /// `Try to receive x from ch.` — outcome depends on instantaneous buffer occupancy.
    TryRecv,
    /// `Try to send v into ch.`
    TrySend,
    /// `Stop handle.` — cancellation at the task's next scheduling point.
    StopTask,
    /// Two or more concurrently-running threads (the main flow + spawned tasks +
    /// `Concurrent`/`Parallel` blocks) can each write to the shared output sink,
    /// so the *interleaving* of their `Show` lines is scheduling-dependent. This
    /// is the one nondeterminism source Kahn determinacy does NOT cover — stdout
    /// is a shared resource the channel-history argument never modelled.
    ConcurrentPrint,
}

/// Append the *direct* nondeterminism witnesses contributed by a single statement.
///
/// This does NOT recurse into nested blocks — descending into the program is the
/// classifier's job ([`super::classify`]). It only reports what `stmt` itself is.
pub(crate) fn direct_nondet_witnesses(stmt: &Stmt, out: &mut Vec<NondetWitness>) {
    match stmt {
        Stmt::Select { branches } => {
            out.push(NondetWitness { kind: NondetKind::Select });
            for branch in branches {
                if let SelectBranch::Timeout { .. } = branch {
                    out.push(NondetWitness { kind: NondetKind::AfterTimer });
                }
            }
        }
        Stmt::TryReceivePipe { .. } => out.push(NondetWitness { kind: NondetKind::TryRecv }),
        Stmt::TrySendPipe { .. } => out.push(NondetWitness { kind: NondetKind::TrySend }),
        Stmt::StopTask { .. } => out.push(NondetWitness { kind: NondetKind::StopTask }),
        _ => {}
    }
}
