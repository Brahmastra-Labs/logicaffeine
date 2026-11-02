//! Definition-time resolution of CALLED NAMES — a call to a name that exists
//! NOWHERE in the program is a check error, not a native-build E0425 or a
//! runtime surprise.
//!
//! This is a NAME pass, not a type pass: it never unifies, so it cannot change
//! any inference outcome. A called name is KNOWN when it is any of:
//!
//!   - a `## To` function (user, prelude `.lg`, or `## To native`) anywhere in
//!     the program — forward references and mutual recursion resolve, exactly
//!     as [`super::check`]'s preregistration and the VM's `fn_index` do;
//!   - a kernel builtin ([`crate::semantics::builtins::builtin_from_name`]) or
//!     one of the VM's by-name intercepts (`show`, `args`, `loaded`);
//!   - an enum variant constructor (the registry's callable spellings);
//!   - a `pub fn` export of a mixed document's bundled proven module — proven
//!     functions, `check_*` predicates, and constructor wrappers, in scope via
//!     `use proven::*;` ([`proven_module_exports`], threaded through
//!     [`check_unknown_calls_with_exports`] by the mixed compile seam);
//!   - a name BOUND anywhere as a variable, parameter, pattern, or handle —
//!     the closure-called-by-name form (`Let f be (x: Int) -> x. Show f(42).`).
//!
//! The binding rule is deliberately program-wide rather than scope-precise: a
//! false accept merely defers to the runtime tiers' existing
//! `Unknown function: <name>` error, while a false reject would refuse a valid
//! program. Every runtime tier keeps its own call-time dispatch; this pass
//! only closes the "name exists nowhere" hole, definition-independently (a
//! call in a never-taken branch is still an error).
//!
//! CALLERS: the post-prelude compile seams (`compile_program_full_with_proven`,
//! `compile_to_rust_with_registry_full` — i.e. `largo check`/`largo build` and
//! every string-compile entry). The LSP's raw-source diagnostics pass must NOT
//! run this (it cannot see the prelude, so every stdlib call would misfire);
//! its deep flycheck covers unknown names through rustc translation.
//!
//! Both walkers match EXHAUSTIVELY — no wildcard arm — so a new `Stmt`/`Expr`
//! variant fails this build until its call and binder positions are wired.

use std::collections::HashSet;

use crate::analysis::unify::TypeError;
use crate::analysis::{TypeDef, TypeRegistry};
use crate::ast::stmt::{
    ClosureBody, Expr, MatchArm, Pattern, ReadSource, SelectBranch, Stmt, StringPart,
};
use crate::intern::{Interner, Symbol};

/// Reject the first call (in source order) to a name that exists nowhere in
/// the post-prelude program. See the module docs for the KNOWN-name rule.
pub fn check_unknown_calls(
    stmts: &[Stmt],
    interner: &Interner,
    registry: &TypeRegistry,
) -> Result<(), TypeError> {
    check_unknown_calls_with_exports(stmts, interner, registry, &HashSet::new())
}

/// [`check_unknown_calls`] for the mixed-document seam: `exports` is the
/// callable surface of a bundled proven module (see [`proven_module_exports`]),
/// whose names the imperative half calls by design (`use proven::*;`).
pub fn check_unknown_calls_with_exports(
    stmts: &[Stmt],
    interner: &Interner,
    registry: &TypeRegistry,
    exports: &HashSet<String>,
) -> Result<(), TypeError> {
    let walk = harvest_calls(stmts);

    let mut variants: HashSet<Symbol> = HashSet::new();
    for (_, def) in registry.iter_types() {
        if let TypeDef::Enum { variants: vs, .. } = def {
            for v in vs {
                variants.insert(v.name);
            }
        }
    }

    for site in &walk.calls {
        let called = &site.function;
        if walk.known.contains(called) || variants.contains(called) {
            continue;
        }
        let name = interner.resolve(*called);
        // The VM's by-name intercepts, resolved before any lookup on every tier.
        if matches!(name, "show" | "args" | "loaded") {
            continue;
        }
        if crate::semantics::builtins::builtin_from_name(name).is_some() {
            continue;
        }
        if exports.contains(name) {
            continue;
        }
        return Err(TypeError::UnknownFunction { name: *called });
    }
    Ok(())
}

/// The first call, in source order, to a builtin the language no longer has
/// ([`crate::semantics::builtins::retired_builtin`]), refused in that builtin's
/// own sentence. A name the program defines or binds is its own and is called.
/// Every tier's front end runs this before the program runs, so each refuses
/// the call in the same words, whether or not the call is ever reached.
pub fn refuse_retired_calls(stmts: &[Stmt], interner: &Interner) -> Result<(), String> {
    let walk = harvest_calls(stmts);
    for site in &walk.calls {
        if walk.known.contains(&site.function) {
            continue;
        }
        if let Some(refusal) =
            crate::semantics::builtins::retired_builtin(interner.resolve(site.function))
        {
            return Err(refusal.to_string());
        }
    }
    Ok(())
}

/// The callable surface of a bundled proven module: every `pub fn` name. The
/// Forge emits proven functions, `check_*` predicates, and non-nullary enum
/// constructors all as `pub fn`s (a nullary constructor is a `pub const`,
/// which is an identifier reference, not a call — outside this pass's domain).
pub fn proven_module_exports(proven: &str) -> HashSet<String> {
    proven
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix("pub fn ")?;
            let end = rest
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            (end > 0).then(|| rest[..end].to_string())
        })
        .collect()
}

/// A by-name call site with its argument expressions, for consumers that need
/// more than the called name (e.g. the exact-HOF call-site analysis in
/// `codegen::bigint_promote`).
pub(crate) struct CallSite<'a> {
    pub(crate) function: Symbol,
    pub(crate) args: &'a [&'a Expr<'a>],
}

/// One traversal, three harvests: every by-name call site in source order
/// (with arguments), every name the program DEFINES or BINDS (functions,
/// variables, params, patterns, handles, zone names, case-table columns), and
/// every bare `Identifier` reference (a name used as a VALUE — aliasing that
/// makes its call sites statically unknowable).
#[derive(Default)]
pub(crate) struct Walk<'a> {
    pub(crate) calls: Vec<CallSite<'a>>,
    pub(crate) known: HashSet<Symbol>,
    pub(crate) value_refs: HashSet<Symbol>,
}

/// Run the exhaustive walker over a whole program. The walker's no-wildcard
/// matches keep every harvest complete: a new `Stmt`/`Expr` variant fails this
/// build until its call, binder, and value-reference positions are wired.
pub(crate) fn harvest_calls<'a>(stmts: &'a [Stmt<'a>]) -> Walk<'a> {
    let mut walk = Walk::default();
    walk.walk_stmts(stmts);
    walk
}

impl<'a> Walk<'a> {
    fn walk_stmts(&mut self, stmts: &'a [Stmt<'a>]) {
        for stmt in stmts {
            self.walk_stmt(stmt);
        }
    }

    fn bind_pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Identifier(sym) => {
                self.known.insert(*sym);
            }
            Pattern::Tuple(syms) => {
                for sym in syms {
                    self.known.insert(*sym);
                }
            }
        }
    }

    fn walk_stmt(&mut self, stmt: &'a Stmt<'a>) {
        match stmt {
            // A component's actions are callable by name from its view and handlers.
            Stmt::ComponentDef(def) => {
                for a in &def.actions {
                    self.known.insert(a.name);
                }
                for f in &def.state {
                    self.walk_expr(f.init);
                }
                for ev in &def.events {
                    self.walk_stmts(ev.body);
                }
                for a in &def.actions {
                    self.walk_stmts(a.body);
                }
                if let Some(view) = &def.view {
                    let (view_exprs, view_blocks) = view.exprs_and_blocks();
                    for e in view_exprs {
                        self.walk_expr(e);
                    }
                    for b in view_blocks {
                        self.walk_stmts(b);
                    }
                }
            }
            Stmt::DirectiveDef {
                name,
                body,
                removed,
                ..
            } => {
                self.known.insert(*name);
                self.walk_stmts(body);
                if let Some(r) = removed {
                    self.walk_stmts(r);
                }
            }
            Stmt::RouteDef { body, .. } => match &**body {
                logicaffeine_language::ast::component::RouteBody::Statements(b) => {
                    self.walk_stmts(b)
                }
                logicaffeine_language::ast::component::RouteBody::View(view) => {
                    let (view_exprs, view_blocks) = view.exprs_and_blocks();
                    for e in view_exprs {
                        self.walk_expr(e);
                    }
                    for b in view_blocks {
                        self.walk_stmts(b);
                    }
                }
            },
            Stmt::MiddlewareDef { body, .. } => self.walk_stmts(body),
            Stmt::RedirectDef { .. } => {}
            // The completion action a fetch names must exist, like any other call.
            Stmt::ServeFetches => {}
            Stmt::Replay { actions } => {
                for a in actions {
                    self.known.insert(*a);
                }
            }
            Stmt::GivenFetch {
                url, status, body, ..
            } => {
                self.walk_expr(url);
                self.walk_expr(status);
                self.walk_expr(body);
            }
            Stmt::Fetch {
                url, body, then, ..
            } => {
                self.known.insert(*then);
                self.walk_expr(url);
                if let Some(b) = body {
                    self.walk_expr(b);
                }
            }
            Stmt::Ui(u) => {
                for x in u.exprs() {
                    self.walk_expr(x);
                }
            }
            Stmt::Let { var, value, .. } => {
                self.known.insert(*var);
                self.walk_expr(value);
            }
            Stmt::Set { target, value } => {
                self.known.insert(*target);
                self.walk_expr(value);
            }
            Stmt::Swap { a, b } => {
                self.known.insert(*a);
                self.known.insert(*b);
            }
            Stmt::Call { function, args } => {
                self.calls.push(CallSite {
                    function: *function,
                    args,
                });
                for arg in args {
                    self.walk_expr(arg);
                }
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                self.walk_expr(cond);
                self.walk_stmts(then_block);
                if let Some(else_b) = else_block {
                    self.walk_stmts(else_b);
                }
            }
            Stmt::While {
                cond,
                body,
                decreasing,
            } => {
                self.walk_expr(cond);
                self.walk_stmts(body);
                if let Some(d) = decreasing {
                    self.walk_expr(d);
                }
            }
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => {
                self.bind_pattern(pattern);
                self.walk_expr(iterable);
                self.walk_stmts(body);
            }
            Stmt::Return { value } => {
                if let Some(v) = value {
                    self.walk_expr(v);
                }
            }
            Stmt::Break => {}
            // Logic-kernel bridges carry no imperative call sites.
            Stmt::Assert { .. } | Stmt::Trust { .. } => {}
            Stmt::RuntimeAssert {
                condition, reason, ..
            } => {
                self.walk_expr(condition);
                if let Some(r) = reason {
                    self.walk_expr(r);
                }
            }
            Stmt::Fail { reason } => self.walk_expr(reason),
            Stmt::Give { object, recipient } | Stmt::Show { object, recipient } => {
                self.walk_expr(object);
                self.walk_expr(recipient);
            }
            Stmt::SetField { object, value, .. } => {
                self.walk_expr(object);
                self.walk_expr(value);
            }
            Stmt::StructDef { .. } => {}
            Stmt::FunctionDef {
                name, params, body, ..
            } => {
                self.known.insert(*name);
                for (param, _) in params {
                    self.known.insert(*param);
                }
                self.walk_stmts(body);
            }
            Stmt::TestDef { cases, body, .. } => {
                if let Some(table) = cases {
                    for column in &table.columns {
                        self.known.insert(*column);
                    }
                    for row in &table.rows {
                        for cell in row {
                            self.walk_expr(cell);
                        }
                    }
                }
                self.walk_stmts(body);
            }
            Stmt::Expect { condition } => self.walk_expr(condition),
            Stmt::ExpectFail { body, .. } => self.walk_stmts(body),
            Stmt::ExpectOutput { .. } => {}
            Stmt::AdvanceTime { millis } => self.walk_expr(millis),
            Stmt::GivenFile { .. } => {}
            Stmt::Observe { payloads, .. } => {
                for (_, value) in payloads {
                    self.walk_expr(value);
                }
            }
            Stmt::During { body, .. } => self.walk_stmts(body),
            Stmt::ForAll {
                vars,
                constraint,
                body,
                ..
            } => {
                for var in vars {
                    self.known.insert(*var);
                }
                if let Some(c) = constraint {
                    self.walk_expr(c);
                }
                self.walk_stmts(body);
            }
            Stmt::Inspect { target, arms, .. } => {
                self.walk_expr(target);
                for MatchArm { bindings, body, .. } in arms {
                    for (_, binding) in bindings {
                        self.known.insert(*binding);
                    }
                    self.walk_stmts(body);
                }
            }
            Stmt::Push { value, collection }
            | Stmt::Add { value, collection }
            | Stmt::Remove { value, collection } => {
                self.walk_expr(value);
                self.walk_expr(collection);
            }
            Stmt::Pop { collection, into } => {
                if let Some(var) = into {
                    self.known.insert(*var);
                }
                self.walk_expr(collection);
            }
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => {
                self.walk_expr(collection);
                self.walk_expr(index);
                self.walk_expr(value);
            }
            Stmt::Splice { body } | Stmt::Reversibly { body } => self.walk_stmts(body),
            Stmt::Zone { name, body, .. } => {
                self.known.insert(*name);
                self.walk_stmts(body);
            }
            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => self.walk_stmts(tasks),
            Stmt::ReadFrom { var, source } => {
                self.known.insert(*var);
                if let ReadSource::File(path) = source {
                    self.walk_expr(path);
                }
            }
            Stmt::WriteFile { content, path } => {
                self.walk_expr(content);
                self.walk_expr(path);
            }
            Stmt::Spawn { .. } => {}
            Stmt::SendMessage {
                message,
                destination,
                ..
            } => {
                self.walk_expr(message);
                self.walk_expr(destination);
            }
            Stmt::AwaitMessage { source, into, .. } => {
                self.known.insert(*into);
                self.walk_expr(source);
            }
            Stmt::StreamMessage {
                values,
                destination,
            } => {
                self.walk_expr(values);
                self.walk_expr(destination);
            }
            Stmt::MergeCrdt { source, target } | Stmt::JoinCrdt { source, target } => {
                self.walk_expr(source);
                self.walk_expr(target);
            }
            Stmt::IncreaseCrdt { object, amount, .. }
            | Stmt::DecreaseCrdt { object, amount, .. } => {
                self.walk_expr(object);
                self.walk_expr(amount);
            }
            Stmt::AppendToSequence { sequence, value } => {
                self.walk_expr(sequence);
                self.walk_expr(value);
            }
            Stmt::ResolveConflict { object, value, .. } => {
                self.walk_expr(object);
                self.walk_expr(value);
            }
            Stmt::Check { .. } => {}
            Stmt::Listen { address, secure } | Stmt::ConnectTo { address, secure } => {
                self.walk_expr(address);
                if let Some(pad) = secure {
                    self.walk_expr(pad.pad);
                }
            }
            Stmt::Serve { port, handler } => {
                self.walk_expr(port);
                self.walk_expr(handler);
            }
            Stmt::LetPeerAgent { var, address } => {
                self.known.insert(*var);
                self.walk_expr(address);
            }
            Stmt::Sleep { milliseconds } => self.walk_expr(milliseconds),
            Stmt::Sync { var, topic } => {
                self.known.insert(*var);
                self.walk_expr(topic);
            }
            Stmt::Mount { var, path } => {
                self.known.insert(*var);
                self.walk_expr(path);
            }
            // Spawn targets are a distinct dispatch (`Unknown task function`,
            // resolved against fn_index alone) — their ARGUMENTS are calls here.
            Stmt::LaunchTask { args, .. } => {
                for arg in args {
                    self.walk_expr(arg);
                }
            }
            Stmt::LaunchTaskWithHandle { handle, args, .. } => {
                self.known.insert(*handle);
                for arg in args {
                    self.walk_expr(arg);
                }
            }
            Stmt::CreatePipe { var, .. } => {
                self.known.insert(*var);
            }
            Stmt::SendPipe { value, pipe } => {
                self.walk_expr(value);
                self.walk_expr(pipe);
            }
            Stmt::ReceivePipe { var, pipe } => {
                self.known.insert(*var);
                self.walk_expr(pipe);
            }
            Stmt::TrySendPipe {
                value,
                pipe,
                result,
            } => {
                if let Some(var) = result {
                    self.known.insert(*var);
                }
                self.walk_expr(value);
                self.walk_expr(pipe);
            }
            Stmt::TryReceivePipe { var, pipe } => {
                self.known.insert(*var);
                self.walk_expr(pipe);
            }
            Stmt::StopTask { handle } => self.walk_expr(handle),
            Stmt::CloseChannel { channel } => self.walk_expr(channel),
            Stmt::AwaitTask { handle, result } => {
                if let Some(var) = result {
                    self.known.insert(*var);
                }
                self.walk_expr(handle);
            }
            Stmt::Select { branches } => {
                for branch in branches {
                    match branch {
                        SelectBranch::Receive { var, pipe, body } => {
                            self.known.insert(*var);
                            self.walk_expr(pipe);
                            self.walk_stmts(body);
                        }
                        SelectBranch::Timeout { milliseconds, body } => {
                            self.walk_expr(milliseconds);
                            self.walk_stmts(body);
                        }
                    }
                }
            }
            Stmt::Theorem(_) | Stmt::Definition(_) | Stmt::Axiom(_) | Stmt::Theory(_) => {}
            Stmt::Escape { .. } => {}
            Stmt::Require { .. } => {}
            // An opaque failure-site marker: no name, no child block, nothing to walk.
            Stmt::Site(_) => {}
        }
    }

    fn walk_expr(&mut self, expr: &'a Expr<'a>) {
        match expr {
            Expr::Dom(q) => {
                if let Some(sel) = q.selector() {
                    self.walk_expr(sel);
                }
            }
            Expr::View(v) => {
                let (view_exprs, view_blocks) = v.exprs_and_blocks();
                for e in view_exprs {
                    self.walk_expr(e);
                }
                for b in view_blocks {
                    self.walk_stmts(b);
                }
            }
            Expr::Literal(_) | Expr::OptionNone | Expr::Escape { .. } => {}
            Expr::Identifier(sym) => {
                self.value_refs.insert(*sym);
            }
            Expr::BinaryOp { left, right, .. }
            | Expr::Union { left, right }
            | Expr::Intersection { left, right } => {
                self.walk_expr(left);
                self.walk_expr(right);
            }
            Expr::Not { operand } => self.walk_expr(operand),
            Expr::Call { function, args } => {
                self.calls.push(CallSite {
                    function: *function,
                    args,
                });
                for arg in args {
                    self.walk_expr(arg);
                }
            }
            Expr::Index { collection, index } => {
                self.walk_expr(collection);
                self.walk_expr(index);
            }
            Expr::Slice {
                collection,
                start,
                end,
            } => {
                self.walk_expr(collection);
                self.walk_expr(start);
                self.walk_expr(end);
            }
            Expr::Copy { expr } => self.walk_expr(expr),
            Expr::Give { value } => self.walk_expr(value),
            Expr::Length { collection } => self.walk_expr(collection),
            Expr::Contains { collection, value } => {
                self.walk_expr(collection);
                self.walk_expr(value);
            }
            Expr::ManifestOf { zone } => self.walk_expr(zone),
            Expr::ChunkAt { index, zone } => {
                self.walk_expr(index);
                self.walk_expr(zone);
            }
            Expr::List(items) | Expr::Tuple(items) => {
                for item in items {
                    self.walk_expr(item);
                }
            }
            Expr::Range { start, end } => {
                self.walk_expr(start);
                self.walk_expr(end);
            }
            Expr::FieldAccess { object, .. } => self.walk_expr(object),
            Expr::New { init_fields, .. } => {
                for (_, value) in init_fields {
                    self.walk_expr(value);
                }
            }
            Expr::NewVariant { fields, .. } => {
                for (_, value) in fields {
                    self.walk_expr(value);
                }
            }
            Expr::OptionSome { value } => self.walk_expr(value),
            Expr::WithCapacity { value, capacity } => {
                self.walk_expr(value);
                self.walk_expr(capacity);
            }
            Expr::Closure { params, body, .. } => {
                for (param, _) in params {
                    self.known.insert(*param);
                }
                match body {
                    ClosureBody::Expression(e) => self.walk_expr(e),
                    ClosureBody::Block(stmts) => self.walk_stmts(stmts),
                }
            }
            Expr::CallExpr { callee, args } => {
                self.walk_expr(callee);
                for arg in args {
                    self.walk_expr(arg);
                }
            }
            Expr::InterpolatedString(parts) => {
                for part in parts {
                    if let StringPart::Expr { value, .. } = part {
                        self.walk_expr(value);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/analysis/known_calls/tests.rs"]
mod tests;
