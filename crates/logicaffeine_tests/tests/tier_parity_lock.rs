//! ════════════════════════════════════════════════════════════════════════════════════════════
//! TIER PARITY LOCK — one statement, every tier, locked together so they can NEVER drift.
//!
//! LOGOS runs the same language on the tree-walker, the bytecode VM, AOT-compiled Rust, and through
//! THREE Futamura partial-evaluator dialects + a decompiler. A statement that works on the
//! tree-walker but is dropped, mis-handled, or compiled to a SILENT no-op on any other surface is a
//! miscompile that ordinary tests miss. This file locks them together at TWO levels:
//!
//!   • COMPILE-TIME — `every_statement_has_a_tier_disposition` is an EXHAUSTIVE match over `Stmt`
//!     with NO wildcard arm. The instant a statement is added to the language, THIS FILE STOPS
//!     COMPILING until the new statement is classified here — which forces you to go handle it in
//!     every tier (tree-walker, VM, projection-1 encoder, the three PE dialects, decompiler) before
//!     you can even build. You cannot silently skip or forget a tier.
//!
//!   • BEHAVIOURAL — peer networking runs NATIVELY on the VM (`run_vm_net_async`), byte-identically
//!     to the tree-walker (`vm_net_cross_tier`), and every networking/streaming statement survives
//!     the projection-1 encoder. (The end-to-end "survives all three PE dialects" lock lives in
//!     `phase_futamura::stream_survives_*`, where the proven per-dialect residualization harness +
//!     correctly-augmented catalog already exist.)
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS FILE TO MAKE A RED CASE PASS.  ⚠️
//!  A RED (or a non-exhaustive-match build error) means a tier dropped or diverged on a statement.
//!  Fix the TIER — `vm::compiler` (loud refusal / real opcodes), `compile::encode_stmt_src` (the
//!  encoder arm), `optimize/pe_*_source.logos` (the per-dialect `When` case with the PER-DIALECT
//!  helper: `peExpr`/`peExprM`/`peExprB`), `optimize/decompile_source.logos` — NEVER by relaxing an
//!  assertion or adding a `_ =>` wildcard here. Strictly monotone: add coverage, never remove it.
//! ════════════════════════════════════════════════════════════════════════════════════════════

use logicaffeine_compile::ast::stmt::Stmt;
use logicaffeine_compile::compile::{pe_bti_source_text, pe_mini_source_text, pe_source_text};

/// How a statement is dispatched across the execution tiers. Every `Stmt` variant maps to exactly
/// one of these — see the no-wildcard match below.
#[derive(Debug, PartialEq, Eq)]
enum TierDisposition {
    /// Runs on BOTH the tree-walker and the bytecode VM (compute, control flow, data, CRDT ops,
    /// channels/tasks). The cross-tier byte-for-byte equality of this class is locked by
    /// `concurrency_differential`.
    Portable,
    /// Async peer networking. Runs on the tree-walker AND **natively on the bytecode VM** via
    /// `run_vm_net_async` (opcodes `Op::Net*` → the async net driver → the shared `NetInbox`), so a
    /// `Send`/`Stream`/`Await` is byte-identical on both tiers — locked by
    /// `vm_net_cross_tier`. (The cooperative-scheduler VM path, `run_vm_concurrent`, does not drive
    /// networking; such programs route to `run_vm_net_async`.) In the browser this runs over the
    /// web-sys WebSocket `Net` relay.
    AsyncNetworking,
    /// Async host I/O (file read/write, sleep, mount). `needs_async` routes it to the tree-walker;
    /// the VM also carries working arms but never sees it in production.
    AsyncHostIo,
    /// Compile-time / verification / declaration — no runtime effect (struct & function
    /// declarations, `Assert`/`Trust`/`Require`/`Theorem`/`Definition`).
    DeclarationOrMeta,
    /// Refused on the VM for soundness (escape analysis); tree-walker only.
    TreeWalkerOnly,
}

/// ★ THE COMPILE-TIME LOCK ★ — an exhaustive match over every `Stmt` variant. There is NO `_ =>`
/// arm ON PURPOSE: adding a statement to the language makes this fail to compile until it is
/// classified, which is the forcing function that stops a tier from being silently skipped. Keep the
/// classification HONEST — it must match what `vm::compiler`, `interpreter::needs_async`, and the PE
/// dialects actually do.
#[allow(dead_code)]
fn every_statement_has_a_tier_disposition(s: &Stmt<'_>) -> TierDisposition {
    match s {
        // ── async peer networking: runs on the tree-walker AND the VM (`run_vm_net_async`) ──
        Stmt::SendMessage { .. }
        | Stmt::StreamMessage { .. }
        | Stmt::AwaitMessage { .. }
        | Stmt::Listen { .. }
        | Stmt::ConnectTo { .. }
        | Stmt::Sync { .. }
        | Stmt::LetPeerAgent { .. } => TierDisposition::AsyncNetworking,

        // ── async host I/O: routed to the tree-walker by `needs_async` ──
        Stmt::ReadFrom { .. }
        | Stmt::WriteFile { .. }
        | Stmt::Sleep { .. }
        | Stmt::Mount { .. } => TierDisposition::AsyncHostIo,

        // ── declaration / verification / meta: no runtime effect ──
        Stmt::StructDef { .. }
        | Stmt::FunctionDef { .. }
        | Stmt::Assert { .. }
        | Stmt::Trust { .. }
        | Stmt::Require { .. }
        | Stmt::Theorem(_)
        | Stmt::Definition(_)
        | Stmt::Axiom(_)
        | Stmt::Theory(_) => TierDisposition::DeclarationOrMeta,

        // ── refused on the VM for soundness ──
        Stmt::Escape { .. } => TierDisposition::TreeWalkerOnly,

        // ── everything else: portable across tree-walker + VM ──
        // (`Splice` is parser-desugar output — a scope-transparent sequence of
        // the portable statements below; the PE encoder lowers it to an
        // always-taken `CIf`, so the PE dialects and decompiler need no arm.)
        Stmt::Let { .. }
        | Stmt::Set { .. }
        | Stmt::SetField { .. }
        | Stmt::SetIndex { .. }
        | Stmt::Splice { .. }
        | Stmt::Return { .. }
        | Stmt::Break
        | Stmt::Call { .. }
        | Stmt::Show { .. }
        | Stmt::Give { .. }
        | Stmt::If { .. }
        | Stmt::While { .. }
        | Stmt::Repeat { .. }
        | Stmt::Inspect { .. }
        | Stmt::Push { .. }
        | Stmt::Pop { .. }
        | Stmt::Add { .. }
        | Stmt::Remove { .. }
        | Stmt::AppendToSequence { .. }
        | Stmt::RuntimeAssert { .. }
        | Stmt::Check { .. }
        | Stmt::Zone { .. }
        | Stmt::Concurrent { .. }
        | Stmt::Parallel { .. }
        | Stmt::Spawn { .. }
        | Stmt::LaunchTask { .. }
        | Stmt::LaunchTaskWithHandle { .. }
        | Stmt::StopTask { .. }
        | Stmt::Select { .. }
        | Stmt::CreatePipe { .. }
        | Stmt::SendPipe { .. }
        | Stmt::ReceivePipe { .. }
        | Stmt::TrySendPipe { .. }
        | Stmt::TryReceivePipe { .. }
        | Stmt::IncreaseCrdt { .. }
        | Stmt::DecreaseCrdt { .. }
        | Stmt::MergeCrdt { .. }
        | Stmt::ResolveConflict { .. } => TierDisposition::Portable,
    }
}

// BEHAVIOURAL LOCK (networking runs on the VM): the VM now NATIVELY executes peer networking via
// `run_vm_net_async`, byte-identically to the tree-walker — `Send`/`Stream`/`Await` cross-tier
// equality is locked by `vm_net_cross_tier.rs`. (This replaces the old interim
// `vm_loudly_refuses_async_networking` lock, which asserted the VM *refused* networking before the
// async net driver existed — the VM handles it now, it does not refuse it.)

// The projection-1 ENCODER lock (every networking/streaming statement emits its `C…` constructor,
// never silently dropped) lives in `futamura_statement_lock.rs`
// (`every_networking_and_streaming_statement_survives_futamura_projection`) — co-locked with this file.

/// PE-ALL DIALECT-PARITY LOCK — the THREE Futamura specialization dialects (`pe_source`/`peBlock`,
/// `pe_mini`/`peBlockM`, `pe_bti`/`peBlockB`) must dispatch on an IDENTICAL set of `C…` constructors.
/// Each dialect's `Inspect` ends in `Otherwise: skip`, so a constructor one dialect has a `When` arm
/// for but another LACKS is SILENTLY DROPPED when that laggard dialect specializes a program using it.
/// This is precisely the regression class that opened this campaign — `When CStreamMessage` was added
/// to all three but with the wrong per-dialect helper, and earlier a statement could be handled in one
/// dialect and missed in another. This lock makes that impossible: add a `When` to one dialect and you
/// MUST add it to all three, or the build's tests go red.
///
///  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK.  ⚠️  A divergence means a statement is dropped from one
///  projection — add the missing `When` arm to the laggard dialect, never relax this assertion.
#[test]
fn all_three_pe_dialects_dispatch_on_identical_constructors() {
    fn whens(src: &str) -> std::collections::BTreeSet<String> {
        src.lines()
            .filter_map(|l| {
                l.trim().strip_prefix("When C").map(|rest| {
                    let name: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
                    format!("C{name}")
                })
            })
            .collect()
    }
    let src = whens(pe_source_text());
    let mini = whens(pe_mini_source_text());
    let bti = whens(pe_bti_source_text());
    assert!(!src.is_empty(), "PE-ALL lock is vacuous: pe_source has no `When C…` arms");
    assert_eq!(
        src, mini,
        "PE-ALL DIALECT DRIFT (pe_source vs pe_mini): only in pe_source {:?}; only in pe_mini {:?}. \
         A statement handled in one dialect but not another is dropped from that projection — add the \
         missing `When` arm, never weaken this lock.",
        src.difference(&mini).collect::<Vec<_>>(),
        mini.difference(&src).collect::<Vec<_>>()
    );
    assert_eq!(
        src, bti,
        "PE-ALL DIALECT DRIFT (pe_source vs pe_bti): only in pe_source {:?}; only in pe_bti {:?}. \
         Add the missing `When` arm, never weaken this lock.",
        src.difference(&bti).collect::<Vec<_>>(),
        bti.difference(&src).collect::<Vec<_>>()
    );
}

/// Tie the COMPILE-TIME lock into the test run so its purpose is discoverable from the suite (the
/// real enforcement is the no-wildcard match above, which is checked every build).
#[test]
fn tier_disposition_classification_is_exhaustive_and_referenced() {
    // The function exists and the match is total — if a `Stmt` variant were unclassified this file
    // would not have compiled. This test documents that the lock is active.
    let _f: fn(&Stmt<'_>) -> TierDisposition = every_statement_has_a_tier_disposition;
}
