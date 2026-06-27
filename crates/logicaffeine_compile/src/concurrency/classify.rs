//! Determinacy classifier — a whole-program AST walk.
//!
//! Mirrors the shape of `codegen::detection::requires_async`. A program
//! is [`Determinacy::Nondeterminate`] iff a nondeterminism source
//! (`Select`/`After`/`Try`/`Stop`) appears anywhere in the program — including
//! function bodies, which is how transitivity through `Call`/`LaunchTask` is
//! covered: the construct lives in the callee's body and is found by the scan.
//!
//! Soundness: this never reports `Determinate` for a program that can make a
//! nondeterministic choice (the scan visits every reachable construct). It is a
//! deliberate *over-approximation* in one direction only — a nondeterminism
//! construct sitting in a defined-but-never-called function is conservatively
//! counted; a future call-graph reachability refinement can tighten that without
//! weakening soundness.

use std::collections::{HashMap, HashSet};

use crate::analysis::callgraph::calls_in_stmts;
use crate::ast::stmt::{Expr, SelectBranch, Stmt};
use crate::intern::Symbol;

use super::model::{direct_nondet_witnesses, Determinacy, NondetKind, NondetWitness};

/// Classify the determinacy of a whole program.
pub fn classify_program(stmts: &[Stmt]) -> Determinacy {
    let mut witnesses = Vec::new();
    for stmt in stmts {
        collect_witnesses_stmt(stmt, &mut witnesses);
    }
    // A whole-program source the per-statement scan cannot see: two concurrent
    // threads racing on the shared stdout sink.
    collect_concurrent_print(stmts, &mut witnesses);
    if witnesses.is_empty() {
        Determinacy::Determinate
    } else {
        Determinacy::Nondeterminate { witnesses }
    }
}

fn collect_witnesses_stmt(stmt: &Stmt, out: &mut Vec<NondetWitness>) {
    // What this statement is, directly.
    direct_nondet_witnesses(stmt, out);
    // Descend into every nested block.
    match stmt {
        Stmt::If { then_block, else_block, .. } => {
            for s in *then_block {
                collect_witnesses_stmt(s, out);
            }
            if let Some(eb) = else_block {
                for s in *eb {
                    collect_witnesses_stmt(s, out);
                }
            }
        }
        Stmt::While { body, .. }
        | Stmt::Repeat { body, .. }
        | Stmt::Zone { body, .. }
        | Stmt::FunctionDef { body, .. } => {
            for s in *body {
                collect_witnesses_stmt(s, out);
            }
        }
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            for s in *tasks {
                collect_witnesses_stmt(s, out);
            }
        }
        Stmt::Inspect { arms, .. } => {
            for arm in arms.iter() {
                for s in arm.body.iter() {
                    collect_witnesses_stmt(s, out);
                }
            }
        }
        Stmt::Select { branches } => {
            for branch in branches {
                match branch {
                    SelectBranch::Receive { body, .. } | SelectBranch::Timeout { body, .. } => {
                        for s in body.iter() {
                            collect_witnesses_stmt(s, out);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

// ─── Concurrent-print (stdout race) analysis ────────────────────────────────
//
// Kahn determinacy proves channel *histories* are schedule-independent, but it
// says nothing about stdout — a shared sink every thread writes to. If two or
// more concurrently-running threads (the main flow, spawned tasks, `Concurrent`/
// `Parallel` branches) can each reach a `Show`, the *interleaving* of their lines
// is a race. We count print-capable concurrent threads; two or more ⇒ a
// `ConcurrentPrint` witness. The count is a sound over-approximation: we never
// under-count a printing thread, so a racy program is never called Determinate.

/// Append a `ConcurrentPrint` witness when ≥2 concurrent threads can print.
fn collect_concurrent_print(stmts: &[Stmt], out: &mut Vec<NondetWitness>) {
    // Index every function body (defs may nest in Main or other functions).
    let mut fn_bodies: HashMap<Symbol, &[Stmt]> = HashMap::new();
    for_each_stmt(stmts, &mut |s| {
        if let Stmt::FunctionDef { name, body, .. } = s {
            fn_bodies.insert(*name, body);
        }
    });

    // Functions whose OWN body directly reaches a `Show` (recursing ordinary
    // control flow, but not across call / spawn / definition boundaries).
    let direct_show_fns: HashSet<Symbol> = fn_bodies
        .iter()
        .filter(|(_, body)| block_directly_shows(body))
        .map(|(name, _)| *name)
        .collect();

    // Does executing `f` reach a print, following SYNCHRONOUS calls (a spawn is a
    // different thread, counted on its own)?
    let fn_prints = |start: Symbol| -> bool {
        let mut stack = vec![start];
        let mut seen = HashSet::new();
        while let Some(g) = stack.pop() {
            if !seen.insert(g) {
                continue;
            }
            if direct_show_fns.contains(&g) {
                return true;
            }
            if let Some(body) = fn_bodies.get(&g) {
                stack.extend(calls_in_stmts(body));
            }
        }
        false
    };

    let mut printers = 0usize;

    // Thread 1 — the root (Main top-level flow): a direct top-level `Show`, or a
    // synchronous call that reaches a printer.
    if block_directly_shows(stmts) || calls_in_stmts(stmts).into_iter().any(&fn_prints) {
        printers += 1;
    }

    // Each spawned task is its own concurrent thread.
    let mut launch_targets: Vec<Symbol> = Vec::new();
    for_each_stmt(stmts, &mut |s| match s {
        Stmt::LaunchTask { function, .. } | Stmt::LaunchTaskWithHandle { function, .. } => {
            launch_targets.push(*function)
        }
        _ => {}
    });
    for f in launch_targets {
        if printers >= 2 {
            break;
        }
        if fn_prints(f) {
            printers += 1;
        }
    }

    // Each `Show` inside a `Concurrent`/`Parallel` block is a concurrent writer
    // in its own right (sound over-approximation — every such line is a racer).
    if printers < 2 {
        let mut blocks: Vec<&[Stmt]> = Vec::new();
        for_each_stmt(stmts, &mut |s| {
            if let Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } = s {
                blocks.push(tasks);
            }
        });
        printers += blocks.iter().map(|b| count_shows(b)).sum::<usize>();
    }

    if printers >= 2 {
        out.push(NondetWitness { kind: NondetKind::ConcurrentPrint });
    }
}

/// Visit every statement in the tree (recursing all nested blocks).
fn for_each_stmt<'a>(stmts: &'a [Stmt<'a>], f: &mut impl FnMut(&'a Stmt<'a>)) {
    for s in stmts {
        f(s);
        match s {
            Stmt::If { then_block, else_block, .. } => {
                for_each_stmt(then_block, f);
                if let Some(eb) = else_block {
                    for_each_stmt(eb, f);
                }
            }
            Stmt::While { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::Zone { body, .. }
            | Stmt::FunctionDef { body, .. } => for_each_stmt(body, f),
            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => for_each_stmt(tasks, f),
            Stmt::Inspect { arms, .. } => {
                for arm in arms.iter() {
                    for_each_stmt(arm.body, f);
                }
            }
            Stmt::Select { branches } => {
                for branch in branches {
                    match branch {
                        SelectBranch::Receive { body, .. } | SelectBranch::Timeout { body, .. } => {
                            for_each_stmt(body, f)
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Does this block directly reach a `Show`, recursing ordinary control flow but
/// NOT crossing call / spawn / function-definition / concurrent-block boundaries
/// (those are handled as their own threads)?
fn block_directly_shows(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_directly_shows)
}

fn stmt_directly_shows(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Show { .. } => true,
        Stmt::If { then_block, else_block, .. } => {
            block_directly_shows(then_block)
                || else_block.map_or(false, |eb| block_directly_shows(eb))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
            block_directly_shows(body)
        }
        Stmt::Inspect { arms, .. } => arms.iter().any(|arm| block_directly_shows(arm.body)),
        Stmt::Select { branches } => branches.iter().any(|branch| match branch {
            SelectBranch::Receive { body, .. } | SelectBranch::Timeout { body, .. } => {
                block_directly_shows(body)
            }
        }),
        _ => false,
    }
}

/// Count every `Show` anywhere in the subtree.
fn count_shows(stmts: &[Stmt]) -> usize {
    let mut n = 0;
    for_each_stmt(stmts, &mut |s| {
        if matches!(s, Stmt::Show { .. }) {
            n += 1;
        }
    });
    n
}

/// Are the branches of a `Parallel`/`Concurrent` block data-independent?
///
/// Conservative (errs toward "dependent"): two branches conflict if they touch
/// the same `Pipe` channel symbol, both mutate the same variable, or one mutates
/// a variable the other references. Independent branches are determinate (their
/// fork-join is order-free); dependent branches that share non-CRDT mutable state
/// are a data race the Send/escape analysis (Phase 4) will reject.
pub fn branches_independent(tasks: &[Stmt]) -> bool {
    let infos: Vec<BranchInfo> = tasks.iter().map(BranchInfo::of).collect();
    for i in 0..infos.len() {
        for j in (i + 1)..infos.len() {
            if infos[i].conflicts_with(&infos[j]) {
                return false;
            }
        }
    }
    true
}

/// Do any two branches of a `Parallel`/`Concurrent` block share *non-channel*
/// mutable state — a variable or collection one writes and another reads or
/// writes? This is the data-race predicate the Send/escape analysis rejects on.
///
/// It deliberately differs from [`branches_independent`] on one axis: a shared
/// `Pipe` is **not** a violation here. A channel is safe under concurrent access
/// (it is the sanctioned message-passing mechanism), so branches that only
/// communicate through a pipe are race-free — they are merely *nondeterminate*,
/// which is the determinacy classifier's concern, not safety's.
pub fn branches_share_mutable_state(tasks: &[Stmt]) -> bool {
    let infos: Vec<BranchInfo> = tasks.iter().map(BranchInfo::of).collect();
    for i in 0..infos.len() {
        for j in (i + 1)..infos.len() {
            if infos[i].shares_mutable_state_with(&infos[j]) {
                return true;
            }
        }
    }
    false
}

#[derive(Default)]
struct BranchInfo {
    /// Variables this branch mutates (Set/Push/Pop/SetIndex/Add/Remove targets).
    writes: HashSet<Symbol>,
    /// Variables this branch references in expressions.
    refs: HashSet<Symbol>,
    /// Pipe/channel symbols this branch sends to or receives from.
    pipes: HashSet<Symbol>,
}

impl BranchInfo {
    fn of(stmt: &Stmt) -> Self {
        let mut info = BranchInfo::default();
        info.scan_stmt(stmt);
        info
    }

    fn conflicts_with(&self, other: &BranchInfo) -> bool {
        !self.pipes.is_disjoint(&other.pipes) || self.shares_mutable_state_with(other)
    }

    /// The data-race core of [`Self::conflicts_with`], minus the channel axis: a
    /// write/write or write/read overlap on a shared variable or collection.
    fn shares_mutable_state_with(&self, other: &BranchInfo) -> bool {
        !self.writes.is_disjoint(&other.writes)
            || !self.writes.is_disjoint(&other.refs)
            || !other.writes.is_disjoint(&self.refs)
    }

    fn scan_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { value, .. } => self.scan_expr(value),
            Stmt::Set { target, value } => {
                self.writes.insert(*target);
                self.scan_expr(value);
            }
            Stmt::Push { collection, value } | Stmt::Add { collection, value } | Stmt::Remove { collection, value } => {
                self.note_mutated(collection);
                self.scan_expr(value);
            }
            Stmt::Pop { collection, .. } => self.note_mutated(collection),
            Stmt::SetIndex { collection, index, value } => {
                self.note_mutated(collection);
                self.scan_expr(index);
                self.scan_expr(value);
            }
            Stmt::Show { object, .. } => self.scan_expr(object),
            Stmt::Return { value } => {
                if let Some(v) = value {
                    self.scan_expr(v);
                }
            }
            Stmt::Call { args, .. } => {
                for a in args.iter() {
                    self.scan_expr(a);
                }
            }
            Stmt::LaunchTask { args, .. } | Stmt::LaunchTaskWithHandle { args, .. } => {
                for a in args.iter() {
                    self.scan_expr(a);
                }
            }
            Stmt::SendPipe { value, pipe } | Stmt::TrySendPipe { value, pipe, .. } => {
                self.note_pipe(pipe);
                self.scan_expr(value);
            }
            Stmt::ReceivePipe { pipe, .. } | Stmt::TryReceivePipe { pipe, .. } => {
                self.note_pipe(pipe);
            }
            Stmt::If { cond, then_block, else_block } => {
                self.scan_expr(cond);
                for s in *then_block {
                    self.scan_stmt(s);
                }
                if let Some(eb) = else_block {
                    for s in *eb {
                        self.scan_stmt(s);
                    }
                }
            }
            Stmt::While { cond, body, .. } => {
                self.scan_expr(cond);
                for s in *body {
                    self.scan_stmt(s);
                }
            }
            Stmt::Repeat { iterable, body, .. } => {
                self.scan_expr(iterable);
                for s in *body {
                    self.scan_stmt(s);
                }
            }
            Stmt::Zone { body, .. } => {
                for s in *body {
                    self.scan_stmt(s);
                }
            }
            _ => {}
        }
    }

    fn note_mutated(&mut self, collection: &Expr) {
        if let Some(root) = root_symbol(collection) {
            self.writes.insert(root);
        }
    }

    fn note_pipe(&mut self, pipe: &Expr) {
        if let Expr::Identifier(sym) = pipe {
            self.pipes.insert(*sym);
        }
    }

    fn scan_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Identifier(sym) => {
                self.refs.insert(*sym);
            }
            Expr::BinaryOp { left, right, .. } => {
                self.scan_expr(left);
                self.scan_expr(right);
            }
            Expr::Call { args, .. } => {
                for a in args.iter() {
                    self.scan_expr(a);
                }
            }
            Expr::CallExpr { callee, args } => {
                self.scan_expr(callee);
                for a in args.iter() {
                    self.scan_expr(a);
                }
            }
            Expr::Index { collection, index } => {
                self.scan_expr(collection);
                self.scan_expr(index);
            }
            Expr::FieldAccess { object, .. } => self.scan_expr(object),
            Expr::Length { collection } => self.scan_expr(collection),
            Expr::Not { operand } => self.scan_expr(operand),
            Expr::List(items) | Expr::Tuple(items) => {
                for i in items.iter() {
                    self.scan_expr(i);
                }
            }
            _ => {}
        }
    }
}

/// The root variable a (possibly nested field-access) lvalue mutates.
fn root_symbol(expr: &Expr) -> Option<Symbol> {
    match expr {
        Expr::Identifier(sym) => Some(*sym),
        Expr::FieldAccess { object, .. } => root_symbol(object),
        Expr::Index { collection, .. } => root_symbol(collection),
        _ => None,
    }
}
