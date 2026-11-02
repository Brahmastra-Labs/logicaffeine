//! What a statement MAY write — the optimizer's one answer.
//!
//! Every pass that asks "which names does this block assign or mutate?" — CSE invalidation, LICM and
//! the loop passes built on its invariance test, deforestation's aliasing check, SCCP's abstract store,
//! the propagator's and supercompiler's callee-global discovery — gets the answer from [`walk_stmt`], so
//! a statement kind can never be a write in one pass and a no-op in another. The walk is exhaustive over
//! `Stmt` and `Expr` (no wildcard): a new statement or expression kind does not compile until it says what
//! it writes.
//!
//! Two receivers ship here. [`WriteSet`] is the flat, scope-blind over-approximation a pass invalidates
//! or tests invariance against. [`CalleeGlobals`] is the program-wide set of names a CALL may write: what
//! every function or closure body assigns or mutates without declaring it, tracked scope-aware so a
//! `Let` inside an `If` does not make a later `Set` of the same name local. Calls are reported as an
//! effect of their own (`EffectSink::call`) rather than as writes, because LOGOS collections are
//! value-semantic across calls (`Op::EnsureOwned`) — the only names a call can change are those globals
//! and what it lends: a binding passed to a `mutable` parameter is the callee's to write, so the call
//! reports it as written, like a `Set`.
//!
//! The same walk reports READS ([`EffectSink::read`]) and EXCHANGES ([`EffectSink::exchange`], a `Swap`'s two
//! names) to the receivers that need them: [`super::mentions::Mentions`], every name a statement touches, and
//! [`super::swap_classes::SwapClasses`], the names a `Swap` ties to one representation. The write
//! receivers ignore reads, and hear an exchange as the two assignments it is.
//!
//! Sound direction: over-approximate. A binder whose exact scoping is uncertain is reported as an
//! assignment (a global write if undeclared), raw foreign code is `opaque` (anything may have been
//! written), and a definition (function, closure) writes nothing at the point it is defined.

use std::collections::HashSet;

use crate::ast::component::{ComponentDef, RouteBody, Trigger};
use crate::ast::stmt::{ClosureBody, Expr, Pattern, ReadSource, SelectBranch, Stmt, StringPart};
use crate::ast::view::ViewDef;
use crate::intern::Symbol;

/// How a statement binds a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bind {
    /// Introduces the name in the current scope (`Let`, a loop pattern, an arm binding, `Pop … into`, …).
    Declare,
    /// Assigns a name that already exists somewhere (`Set`, `Swap`).
    Assign,
}

/// The receiver of a statement's write effects. [`walk_stmt`] reports every effect of one statement and
/// descends into nested blocks through [`EffectSink::scoped`] / [`EffectSink::transparent`], so a receiver
/// can track scope (or not) by overriding them.
pub(crate) trait EffectSink {
    fn bind(&mut self, sym: Symbol, how: Bind);
    /// An in-place mutation of the value rooted at `root` (an element, field, or CRDT update).
    fn mutate(&mut self, root: Symbol);
    /// The statement runs a call — a `Call` statement, `Expr::Call`/`Expr::CallExpr`, a task launch or
    /// join, an HTTP `Serve` loop. Callee bodies may write the program's globals.
    fn call(&mut self);
    /// Raw foreign code ran: it may have assigned anything.
    fn opaque(&mut self);
    /// The statement READS the binding `sym`: an identifier an expression names, a callee named by a binding, or a
    /// name a statement carries (`Swap`'s two, a `Check`'s subject and object). Only a receiver that tracks every
    /// name a statement touches asks; the write receivers ignore it.
    fn read(&mut self, _sym: Symbol) {}
    /// `Swap a and b`: each name is read and reassigned the other's value. A receiver that must keep the two alike
    /// (one representation for both) hears the pair; every other receiver hears the two reads and the two
    /// assignments.
    fn exchange(&mut self, a: Symbol, b: Symbol) {
        self.read(a);
        self.read(b);
        self.bind(a, Bind::Assign);
        self.bind(b, Bind::Assign);
    }
    /// A nested block with its own scope, `binders` introduced at its top.
    fn scoped(&mut self, binders: &[Symbol], body: &[Stmt]) {
        for b in binders {
            self.bind(*b, Bind::Declare);
        }
        walk_block(body, self);
    }
    /// A nested block transparent to scope (`Splice`, `Reversibly`): its names are the enclosing block's.
    fn transparent(&mut self, body: &[Stmt]) {
        walk_block(body, self);
    }
    /// A closure DEFINITION. Its body runs later, at some call — a definition writes nothing now.
    fn closure(&mut self, _params: &[Symbol], _body: &ClosureBody) {}
    /// A nested function DEFINITION: the callee's scope, not the enclosing one.
    fn function(&mut self, _params: &[Symbol], _body: &[Stmt]) {}
}

/// Report every write effect of every statement in `block`.
pub(crate) fn walk_block<S: EffectSink + ?Sized>(block: &[Stmt], sink: &mut S) {
    for stmt in block {
        walk_stmt(stmt, sink);
    }
}

/// The symbols a loop pattern binds each iteration.
pub(crate) fn pattern_symbols(pattern: &Pattern) -> Vec<Symbol> {
    match pattern {
        Pattern::Identifier(sym) => vec![*sym],
        Pattern::Tuple(syms) => syms.clone(),
    }
}

/// The root name of an in-place mutation target: `xs`, `p's inner's x`, `item i of grid` all root at the
/// bare identifier the chain starts from. A target with no identifier root (a literal, a call result) has
/// nothing nameable to invalidate — the mutation lands on a temporary.
pub(crate) fn mutation_root(expr: &Expr) -> Option<Symbol> {
    match expr {
        Expr::Identifier(sym) => Some(*sym),
        Expr::FieldAccess { object, .. } => mutation_root(object),
        Expr::Index { collection, .. } => mutation_root(collection),
        Expr::Slice { collection, .. } => mutation_root(collection),
        _ => None,
    }
}

fn mutate_root<S: EffectSink + ?Sized>(target: &Expr, sink: &mut S) {
    if let Some(root) = mutation_root(target) {
        sink.mutate(root);
    }
}

/// A call to `function` writes what it lends: each argument at a `mutable` parameter position
/// ([`crate::analysis::callgraph::lent_positions`]) — reassigned whole when the argument is a
/// binding, mutated in place when it is a field or an element of one.
fn lends<S: EffectSink + ?Sized>(function: Symbol, args: &[&Expr], sink: &mut S) {
    for at in crate::analysis::callgraph::lent_positions(function) {
        match args.get(at) {
            Some(Expr::Identifier(sym)) => sink.bind(*sym, Bind::Assign),
            Some(place) => mutate_root(place, sink),
            None => {}
        }
    }
}

fn param_names(params: &[(Symbol, &crate::ast::stmt::TypeExpr)]) -> Vec<Symbol> {
    params.iter().map(|(p, _)| *p).collect()
}

/// Report every write effect of one statement (exhaustive over `Stmt`).
pub(crate) fn walk_stmt<S: EffectSink + ?Sized>(stmt: &Stmt, sink: &mut S) {
    match stmt {
        Stmt::Let { var, value, .. } => {
            walk_expr(value, sink);
            sink.bind(*var, Bind::Declare);
        }
        Stmt::Set { target, value } => {
            walk_expr(value, sink);
            sink.bind(*target, Bind::Assign);
        }
        Stmt::Swap { a, b } => sink.exchange(*a, *b),
        Stmt::Call { function, args } => {
            sink.read(*function);
            for a in args {
                walk_expr(a, sink);
            }
            lends(*function, args, sink);
            sink.call();
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            walk_expr(cond, sink);
            sink.scoped(&[], then_block);
            if let Some(eb) = else_block {
                sink.scoped(&[], eb);
            }
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            walk_expr(cond, sink);
            if let Some(d) = decreasing {
                walk_expr(d, sink);
            }
            sink.scoped(&[], body);
        }
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => {
            walk_expr(iterable, sink);
            sink.scoped(&pattern_symbols(pattern), body);
        }
        Stmt::Return { value } => {
            if let Some(v) = value {
                walk_expr(v, sink);
            }
        }
        Stmt::Break => {}
        // Static proof obligations: nothing runs.
        Stmt::Assert { .. } | Stmt::Trust { .. } => {}
        Stmt::RuntimeAssert {
            condition, reason, ..
        } => {
            walk_expr(condition, sink);
            if let Some(r) = reason {
                walk_expr(r, sink);
            }
        }
        Stmt::Fail { reason } => walk_expr(reason, sink),
        Stmt::Give { object, recipient } | Stmt::Show { object, recipient } => {
            walk_expr(object, sink);
            walk_expr(recipient, sink);
        }
        Stmt::SetField { object, value, .. } => {
            walk_expr(object, sink);
            walk_expr(value, sink);
            mutate_root(object, sink);
        }
        Stmt::StructDef { .. } => {}
        Stmt::FunctionDef { params, body, .. } => sink.function(&param_names(params), body),
        Stmt::TestDef { cases, body, .. } => {
            let mut columns = Vec::new();
            if let Some(table) = cases {
                columns.extend(table.columns.iter().copied());
                for row in &table.rows {
                    for e in row {
                        walk_expr(e, sink);
                    }
                }
            }
            sink.scoped(&columns, body);
        }
        Stmt::Expect { condition } => walk_expr(condition, sink),
        Stmt::ExpectFail { body, .. } => sink.scoped(&[], body),
        Stmt::ExpectOutput { .. } | Stmt::GivenFile { .. } => {}
        Stmt::AdvanceTime { millis } => walk_expr(millis, sink),
        Stmt::Observe { payloads, .. } => {
            for (_, e) in payloads {
                walk_expr(e, sink);
            }
        }
        Stmt::During { body, .. } => sink.scoped(&[], body),
        Stmt::ForAll {
            vars,
            constraint,
            body,
            ..
        } => {
            if let Some(c) = constraint {
                walk_expr(c, sink);
            }
            sink.scoped(vars, body);
        }
        Stmt::Inspect { target, arms, .. } => {
            walk_expr(target, sink);
            for arm in arms {
                let bound: Vec<Symbol> = arm.bindings.iter().map(|(_, b)| *b).collect();
                sink.scoped(&bound, arm.body);
            }
        }
        Stmt::Push { value, collection }
        | Stmt::Add { value, collection }
        | Stmt::Remove { value, collection } => {
            walk_expr(value, sink);
            walk_expr(collection, sink);
            mutate_root(collection, sink);
        }
        Stmt::Pop { collection, into } => {
            walk_expr(collection, sink);
            mutate_root(collection, sink);
            if let Some(v) = into {
                sink.bind(*v, Bind::Declare);
            }
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            walk_expr(collection, sink);
            walk_expr(index, sink);
            walk_expr(value, sink);
            mutate_root(collection, sink);
        }
        Stmt::Splice { body } | Stmt::Reversibly { body } => sink.transparent(body),
        Stmt::Zone { name, body, .. } => sink.scoped(&[*name], body),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => sink.scoped(&[], tasks),
        Stmt::ReadFrom { var, source } => {
            if let ReadSource::File(path) = source {
                walk_expr(path, sink);
            }
            sink.bind(*var, Bind::Declare);
        }
        Stmt::WriteFile { content, path } => {
            walk_expr(content, sink);
            walk_expr(path, sink);
        }
        Stmt::Spawn { name, .. } => sink.bind(*name, Bind::Declare),
        Stmt::SendMessage {
            message,
            destination,
            ..
        }
        | Stmt::StreamMessage {
            values: message,
            destination,
        } => {
            walk_expr(message, sink);
            walk_expr(destination, sink);
        }
        Stmt::AwaitMessage { source, into, .. } => {
            walk_expr(source, sink);
            sink.bind(*into, Bind::Declare);
        }
        Stmt::MergeCrdt { source, target } | Stmt::JoinCrdt { source, target } => {
            walk_expr(source, sink);
            walk_expr(target, sink);
            mutate_root(target, sink);
        }
        Stmt::IncreaseCrdt { object, amount, .. } | Stmt::DecreaseCrdt { object, amount, .. } => {
            walk_expr(object, sink);
            walk_expr(amount, sink);
            mutate_root(object, sink);
        }
        Stmt::AppendToSequence { sequence, value } => {
            walk_expr(sequence, sink);
            walk_expr(value, sink);
            mutate_root(sequence, sink);
        }
        Stmt::ResolveConflict { object, value, .. } => {
            walk_expr(object, sink);
            walk_expr(value, sink);
            mutate_root(object, sink);
        }
        Stmt::Check {
            subject, object, ..
        } => {
            sink.read(*subject);
            if let Some(o) = object {
                sink.read(*o);
            }
        }
        Stmt::Listen { address, secure } | Stmt::ConnectTo { address, secure } => {
            walk_expr(address, sink);
            if let Some(pad) = secure {
                walk_expr(pad.pad, sink);
            }
        }
        Stmt::Serve { port, handler } => {
            walk_expr(port, sink);
            walk_expr(handler, sink);
            sink.call();
        }
        Stmt::LetPeerAgent { var, address } => {
            walk_expr(address, sink);
            sink.bind(*var, Bind::Declare);
        }
        Stmt::Sleep { milliseconds } => walk_expr(milliseconds, sink),
        Stmt::Sync { var, topic: source } | Stmt::Mount { var, path: source } => {
            walk_expr(source, sink);
            sink.bind(*var, Bind::Declare);
        }
        Stmt::LaunchTask { args, .. } => {
            for a in args {
                walk_expr(a, sink);
            }
            sink.call();
        }
        Stmt::LaunchTaskWithHandle { handle, args, .. } => {
            for a in args {
                walk_expr(a, sink);
            }
            sink.call();
            sink.bind(*handle, Bind::Declare);
        }
        Stmt::CreatePipe { var, .. } => sink.bind(*var, Bind::Declare),
        Stmt::SendPipe { value, pipe } => {
            walk_expr(value, sink);
            walk_expr(pipe, sink);
        }
        Stmt::ReceivePipe { var, pipe } | Stmt::TryReceivePipe { var, pipe } => {
            walk_expr(pipe, sink);
            sink.bind(*var, Bind::Declare);
        }
        Stmt::TrySendPipe {
            value,
            pipe,
            result,
        } => {
            walk_expr(value, sink);
            walk_expr(pipe, sink);
            if let Some(r) = result {
                sink.bind(*r, Bind::Declare);
            }
        }
        Stmt::StopTask { handle } | Stmt::CloseChannel { channel: handle } => {
            walk_expr(handle, sink)
        }
        // Joining a task lands the writes its function made while it ran.
        Stmt::AwaitTask { handle, result } => {
            walk_expr(handle, sink);
            sink.call();
            if let Some(r) = result {
                sink.bind(*r, Bind::Declare);
            }
        }
        Stmt::Select { branches } => {
            for branch in branches {
                match branch {
                    SelectBranch::Receive { var, pipe, body } => {
                        walk_expr(pipe, sink);
                        sink.scoped(&[*var], body);
                    }
                    SelectBranch::Timeout { milliseconds, body } => {
                        walk_expr(milliseconds, sink);
                        sink.scoped(&[], body);
                    }
                }
            }
        }
        Stmt::Theorem(_) | Stmt::Definition(_) | Stmt::Axiom(_) | Stmt::Theory(_) => {}
        Stmt::Escape { .. } => sink.opaque(),
        Stmt::Require { .. } => {}
        // The reactive/web surface: definitions whose bodies run later — at an event, a request, a
        // render. A definition writes nothing where it stands; each body is a callee body whose
        // locals are the definition's props, state, and parameters.
        Stmt::ComponentDef(def) => walk_component_def(def, sink),
        Stmt::DirectiveDef {
            params,
            body,
            removed,
            ..
        } => {
            let names = param_names(params);
            sink.function(&names, body);
            if let Some(removed) = removed {
                sink.function(&names, removed);
            }
        }
        Stmt::RouteDef { params, body, .. } => {
            let names = param_names(params);
            match &**body {
                RouteBody::Statements(block) => sink.function(&names, block),
                RouteBody::View(view) => walk_view_def_as_callee(view, &names, sink),
            }
        }
        Stmt::MiddlewareDef { param, body } => sink.function(&[param.0], body),
        Stmt::RedirectDef { .. } => {}
        // `Load … into x` and `Toggle x` write a state field; a fetch's completion runs later.
        Stmt::Ui(u) => {
            if let Some(w) = u.writes() {
                sink.mutate(w);
            }
            for x in u.exprs() {
                walk_expr(x, sink);
            }
        }
        Stmt::ServeFetches | Stmt::Replay { .. } => {}
        Stmt::GivenFetch {
            url, status, body, ..
        } => {
            walk_expr(url, sink);
            walk_expr(status, sink);
            walk_expr(body, sink);
        }
        Stmt::Fetch { url, body, .. } => {
            walk_expr(url, sink);
            if let Some(b) = body {
                walk_expr(b, sink);
            }
        }
        // An opaque failure-site marker: an id, no variable touched.
        Stmt::Site(_) => {}
    }
}

/// A component definition: props, state fields, and every handler/action parameter are the locals of
/// its bodies; the bodies (event handlers, actions, view handlers) are callee bodies.
fn walk_component_def<S: EffectSink + ?Sized>(def: &ComponentDef, sink: &mut S) {
    let mut locals: Vec<Symbol> = def.props.iter().map(|(p, _)| *p).collect();
    locals.extend(def.state.iter().map(|f| f.name));
    for handler in &def.events {
        let mut names = locals.clone();
        match &handler.trigger {
            Trigger::Message { param, .. } | Trigger::FileDropped { param, .. } => {
                names.push(param.0)
            }
            Trigger::ActionFails { param } => names.push(*param),
            Trigger::FetchFails { url, error } => {
                names.push(*url);
                names.push(*error);
            }
            Trigger::After { .. }
            | Trigger::PageLoads
            | Trigger::Appears
            | Trigger::Disappears
            | Trigger::RouteChanges
            | Trigger::Online
            | Trigger::Offline
            | Trigger::Selector { .. }
            | Trigger::Key { .. }
            | Trigger::FieldChanges { .. }
            | Trigger::Every { .. }
            | Trigger::WindowResizes
            | Trigger::ScrollsToBottom { .. } => {}
        }
        sink.function(&names, handler.body);
    }
    for action in &def.actions {
        let mut names = locals.clone();
        names.extend(action.params.iter().map(|(p, _)| *p));
        sink.function(&names, action.body);
    }
    if let Some(view) = &def.view {
        walk_view_def_as_callee(view, &locals, sink);
    }
}

/// A view tree rendered later: its handler blocks are callee bodies over `locals`; its embedded
/// expressions evaluate at render time inside those same bodies' scope.
fn walk_view_def_as_callee<S: EffectSink + ?Sized>(
    view: &ViewDef,
    locals: &[Symbol],
    sink: &mut S,
) {
    let (_, blocks) = view.exprs_and_blocks();
    for block in blocks {
        sink.function(locals, block);
    }
}

/// Report every effect an expression's EVALUATION has (exhaustive over `Expr`): the calls it makes, the
/// foreign code it runs, and the names it reads.
pub(crate) fn walk_expr<S: EffectSink + ?Sized>(expr: &Expr, sink: &mut S) {
    match expr {
        Expr::Literal(_) | Expr::OptionNone => {}
        Expr::Identifier(sym) => sink.read(*sym),
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range {
            start: left,
            end: right,
        }
        | Expr::Contains {
            collection: left,
            value: right,
        }
        | Expr::Index {
            collection: left,
            index: right,
        }
        | Expr::WithCapacity {
            value: left,
            capacity: right,
        }
        | Expr::ChunkAt {
            index: left,
            zone: right,
        } => {
            walk_expr(left, sink);
            walk_expr(right, sink);
        }
        Expr::Not { operand }
        | Expr::Copy { expr: operand }
        | Expr::Give { value: operand }
        | Expr::Length {
            collection: operand,
        }
        | Expr::ManifestOf { zone: operand }
        | Expr::FieldAccess {
            object: operand, ..
        }
        | Expr::OptionSome { value: operand } => walk_expr(operand, sink),
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            walk_expr(collection, sink);
            walk_expr(start, sink);
            walk_expr(end, sink);
        }
        Expr::Call { function, args } => {
            sink.read(*function);
            for a in args {
                walk_expr(a, sink);
            }
            lends(*function, args, sink);
            sink.call();
        }
        Expr::CallExpr { callee, args } => {
            walk_expr(callee, sink);
            for a in args {
                walk_expr(a, sink);
            }
            sink.call();
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for item in items {
                walk_expr(item, sink);
            }
        }
        Expr::New {
            init_fields: fields,
            ..
        }
        | Expr::NewVariant { fields, .. } => {
            for (_, e) in fields {
                walk_expr(e, sink);
            }
        }
        Expr::Escape { .. } => sink.opaque(),
        // A DOM query READS the page — `the text of SEL`, `the count of SEL`, the title, the URL.
        // It writes no LOGOS name; its only effect is evaluating the selector expression, which may
        // itself run a call.
        Expr::Dom(query) => {
            if let Some(selector) = query.selector() {
                walk_expr(selector, sink);
            }
        }
        Expr::Closure { params, body, .. } => sink.closure(&param_names(params), body),
        Expr::InterpolatedString(parts) => {
            for part in parts {
                match part {
                    StringPart::Literal(_) => {}
                    StringPart::Expr { value, .. } => walk_expr(value, sink),
                }
            }
        }
        // A template literal: its embedded expressions evaluate as the view renders (any call they
        // make is a call of this expression), its handler blocks are closures that run at events.
        Expr::View(view) => {
            let (exprs, blocks) = view.exprs_and_blocks();
            for e in exprs {
                walk_expr(e, sink);
            }
            for block in blocks {
                sink.closure(&[], &ClosureBody::Block(block));
            }
        }
    }
}

/// The flat, scope-blind over-approximation of what a block may write: every name it declares
/// (`Let`-like binders), every name it reassigns (`Set`, `Swap`), every root it mutates in place,
/// whether it calls anything, and whether it ran foreign code. A pass invalidates against it
/// ([`WriteSet::symbols`]), tests invariance with [`WriteSet::may_write`], or asks which names are
/// not constants ([`WriteSet::reassigned`]).
#[derive(Debug, Clone, Default)]
pub(crate) struct WriteSet {
    declared: HashSet<Symbol>,
    assigned: HashSet<Symbol>,
    mutated: HashSet<Symbol>,
    calls: bool,
    opaque: bool,
}

impl EffectSink for WriteSet {
    fn bind(&mut self, sym: Symbol, how: Bind) {
        match how {
            Bind::Declare => self.declared.insert(sym),
            Bind::Assign => self.assigned.insert(sym),
        };
    }
    fn mutate(&mut self, root: Symbol) {
        self.mutated.insert(root);
    }
    fn call(&mut self) {
        self.calls = true;
    }
    fn opaque(&mut self) {
        self.opaque = true;
    }
}

impl WriteSet {
    pub(crate) fn of_block(block: &[Stmt]) -> Self {
        let mut w = WriteSet::default();
        walk_block(block, &mut w);
        w
    }

    pub(crate) fn of_stmt(stmt: &Stmt) -> Self {
        let mut w = WriteSet::default();
        walk_stmt(stmt, &mut w);
        w
    }

    /// The effects of `stmt` ITSELF — its own binders, mutations, calls, and foreign code, plus the
    /// names a nested block's header binds (a loop pattern, an arm binding) — without descending into
    /// nested blocks. For a walker that recurses into those blocks on its own, this is the whole of
    /// what the statement adds; it costs O(statement), not O(subtree), so a fixpoint that revisits a
    /// deep nest pays nothing extra per level.
    pub(crate) fn of_stmt_shallow(stmt: &Stmt) -> Self {
        struct Shallow<'w>(&'w mut WriteSet);
        impl EffectSink for Shallow<'_> {
            fn bind(&mut self, sym: Symbol, how: Bind) {
                self.0.bind(sym, how);
            }
            fn mutate(&mut self, root: Symbol) {
                self.0.mutate(root);
            }
            fn call(&mut self) {
                self.0.call();
            }
            fn opaque(&mut self) {
                self.0.opaque();
            }
            fn scoped(&mut self, binders: &[Symbol], _body: &[Stmt]) {
                for b in binders {
                    self.0.bind(*b, Bind::Declare);
                }
            }
            fn transparent(&mut self, _body: &[Stmt]) {}
        }
        let mut w = WriteSet::default();
        walk_stmt(stmt, &mut Shallow(&mut w));
        w
    }

    /// The effects of EVALUATING an expression (its calls and foreign code).
    pub(crate) fn of_expr(expr: &Expr) -> Self {
        let mut w = WriteSet::default();
        walk_expr(expr, &mut w);
        w
    }

    /// Record a name the caller knows is reassigned — a loop's iteration variable, a `Let` the
    /// caller treats as varying.
    pub(crate) fn assign(&mut self, sym: Symbol) {
        self.assigned.insert(sym);
    }

    /// Fold in what the block's calls may write: with `globals` the program's callee-written names, a
    /// block that calls anything may reassign all of them (or anything at all, if some callee is opaque).
    pub(crate) fn with_callee_globals(mut self, globals: &CalleeGlobals) -> Self {
        if self.calls {
            if globals.opaque {
                self.opaque = true;
            } else {
                self.assigned.extend(globals.symbols.iter().copied());
            }
        }
        self
    }

    /// May the block change the value `sym` names — by binding it, mutating it in place, or through
    /// foreign code that could touch anything?
    pub(crate) fn may_write(&self, sym: Symbol) -> bool {
        self.opaque
            || self.declared.contains(&sym)
            || self.assigned.contains(&sym)
            || self.mutated.contains(&sym)
    }

    /// Foreign code ran: every name may have been written. [`WriteSet::symbols`] cannot express that —
    /// a consumer must check this first.
    pub(crate) fn is_opaque(&self) -> bool {
        self.opaque
    }

    /// The block runs at least one call (whose effects on globals [`WriteSet::with_callee_globals`] folds in).
    pub(crate) fn has_calls(&self) -> bool {
        self.calls
    }

    /// Every name the block declares, reassigns, or mutates, each once (meaningful when not
    /// [`WriteSet::is_opaque`]).
    pub(crate) fn symbols(&self) -> impl Iterator<Item = Symbol> + '_ {
        self.declared
            .iter()
            .chain(self.assigned.iter().filter(|s| !self.declared.contains(s)))
            .chain(
                self.mutated
                    .iter()
                    .filter(|s| !self.declared.contains(s) && !self.assigned.contains(s)),
            )
            .copied()
    }

    /// Every name whose value the block may CHANGE after it was bound — reassigned or mutated in
    /// place — each once. A `Let` that is never in this set holds one value for its whole life: a constant.
    pub(crate) fn reassigned(&self) -> impl Iterator<Item = Symbol> + '_ {
        self.assigned
            .iter()
            .chain(self.mutated.iter().filter(|s| !self.assigned.contains(s)))
            .copied()
    }

    /// Every name the block reassigns as a scalar (`Set`, `Swap`, a callee-written global).
    pub(crate) fn assigned(&self) -> &HashSet<Symbol> {
        &self.assigned
    }

    /// Every name declared within the block, including lexical rebindings.
    pub(crate) fn declared(&self) -> &HashSet<Symbol> {
        &self.declared
    }

    /// Every root the block mutates in place (an element, field, or CRDT update).
    pub(crate) fn mutated(&self) -> &HashSet<Symbol> {
        &self.mutated
    }
}

/// How many times `stmts` binds `sym` — every `Let`, `Set`, `Swap`, or other binder of it, anywhere
/// in nested blocks. A call in `stmts` counts as one more write when `globals` says a callee assigns
/// `sym`; foreign code counts as unboundedly many (`usize::MAX`). A pass that needs "written exactly
/// once" (an induction variable's sole step) compares against this.
pub(crate) fn count_writes_of(stmts: &[Stmt], sym: Symbol, globals: &CalleeGlobals) -> usize {
    struct Counter {
        sym: Symbol,
        n: usize,
        calls: bool,
        opaque: bool,
    }
    impl EffectSink for Counter {
        fn bind(&mut self, sym: Symbol, _how: Bind) {
            if sym == self.sym {
                self.n += 1;
            }
        }
        fn mutate(&mut self, root: Symbol) {
            if root == self.sym {
                self.n += 1;
            }
        }
        fn call(&mut self) {
            self.calls = true;
        }
        fn opaque(&mut self) {
            self.opaque = true;
        }
    }
    let mut c = Counter {
        sym,
        n: 0,
        calls: false,
        opaque: false,
    };
    walk_block(stmts, &mut c);
    if c.opaque {
        return usize::MAX;
    }
    if c.calls && globals.contains(&sym) {
        c.n += 1;
    }
    c.n
}

/// The names `stmts` may REBIND: every `Set`/`Swap` target (and callee-written global, when `stmts`
/// calls), plus every name bound by more than one `Let`-like binder (a shadow or rebind). A name
/// outside this set holds the same value from its one declaration on. Conservative across scopes: a
/// name declared once in each of two blocks counts as rebound.
pub(crate) fn rebound_names(stmts: &[Stmt], globals: &CalleeGlobals) -> HashSet<Symbol> {
    struct Rebound {
        assigned: HashSet<Symbol>,
        declared: std::collections::HashMap<Symbol, u32>,
        mutated: HashSet<Symbol>,
        calls: bool,
        opaque: bool,
    }
    impl EffectSink for Rebound {
        fn bind(&mut self, sym: Symbol, how: Bind) {
            match how {
                Bind::Declare => *self.declared.entry(sym).or_insert(0) += 1,
                Bind::Assign => {
                    self.assigned.insert(sym);
                }
            }
        }
        fn mutate(&mut self, root: Symbol) {
            self.mutated.insert(root);
        }
        fn call(&mut self) {
            self.calls = true;
        }
        fn opaque(&mut self) {
            self.opaque = true;
        }
    }
    let mut r = Rebound {
        assigned: HashSet::new(),
        declared: std::collections::HashMap::new(),
        mutated: HashSet::new(),
        calls: false,
        opaque: false,
    };
    walk_block(stmts, &mut r);
    let mut out = r.assigned;
    out.extend(r.mutated);
    for (sym, n) in r.declared {
        if n > 1 {
            out.insert(sym);
        }
    }
    if r.calls {
        out.extend(globals.symbols.iter().copied());
    }
    if r.opaque {
        // Foreign code may rebind anything: every declared name is suspect.
        out.extend(stmts_declared(stmts));
    }
    out
}

fn stmts_declared(stmts: &[Stmt]) -> HashSet<Symbol> {
    WriteSet::of_block(stmts).declared
}

/// The names any CALL may write: every global some function or closure in the program assigns or mutates
/// without declaring it. `opaque` when a callee body holds raw foreign code — then a call may write
/// anything, and [`CalleeGlobals::contains`] says so for every name.
#[derive(Debug, Clone, Default)]
pub(crate) struct CalleeGlobals {
    pub(crate) symbols: HashSet<Symbol>,
    pub(crate) opaque: bool,
}

impl CalleeGlobals {
    pub(crate) fn of_program(stmts: &[Stmt]) -> Self {
        let mut finder = CalleeFinder {
            out: CalleeGlobals::default(),
        };
        walk_block(stmts, &mut finder);
        finder.out
    }

    /// May a call change the value `sym` names?
    pub(crate) fn contains(&self, sym: &Symbol) -> bool {
        self.opaque || self.symbols.contains(sym)
    }

    /// No call in this program writes any global.
    pub(crate) fn is_empty(&self) -> bool {
        !self.opaque && self.symbols.is_empty()
    }
}

/// Finds every callee body in the program — function definitions and closures, however deeply nested —
/// and collects their scope-aware global writes. Effects of the top-level code itself are not callee
/// effects and are ignored.
struct CalleeFinder {
    out: CalleeGlobals,
}

impl EffectSink for CalleeFinder {
    fn bind(&mut self, _sym: Symbol, _how: Bind) {}
    fn mutate(&mut self, _root: Symbol) {}
    fn call(&mut self) {}
    fn opaque(&mut self) {}
    fn closure(&mut self, params: &[Symbol], body: &ClosureBody) {
        let mut scoped = ScopedWrites::new(params, &mut self.out);
        match body {
            ClosureBody::Expression(e) => walk_expr(e, &mut scoped),
            ClosureBody::Block(b) => walk_block(b, &mut scoped),
        }
    }
    fn function(&mut self, params: &[Symbol], body: &[Stmt]) {
        let mut scoped = ScopedWrites::new(params, &mut self.out);
        walk_block(body, &mut scoped);
    }
}

/// Scope-tracking receiver for one callee body: an assignment or mutation of a name no enclosing scope
/// of the body declares is a write to a GLOBAL.
struct ScopedWrites<'o> {
    scopes: Vec<HashSet<Symbol>>,
    out: &'o mut CalleeGlobals,
}

impl<'o> ScopedWrites<'o> {
    fn new(params: &[Symbol], out: &'o mut CalleeGlobals) -> Self {
        ScopedWrites {
            scopes: vec![params.iter().copied().collect()],
            out,
        }
    }

    fn in_scope(&self, sym: Symbol) -> bool {
        self.scopes.iter().any(|s| s.contains(&sym))
    }

    fn write(&mut self, sym: Symbol) {
        if !self.in_scope(sym) {
            self.out.symbols.insert(sym);
        }
    }
}

impl EffectSink for ScopedWrites<'_> {
    fn bind(&mut self, sym: Symbol, how: Bind) {
        match how {
            Bind::Declare => {
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(sym);
                }
            }
            Bind::Assign => self.write(sym),
        }
    }
    fn mutate(&mut self, root: Symbol) {
        self.write(root);
    }
    // A call from inside a callee reaches its own callee's globals, which are collected when THAT
    // definition is walked — the union over all definitions is already transitive.
    fn call(&mut self) {}
    fn opaque(&mut self) {
        self.out.opaque = true;
    }
    fn scoped(&mut self, binders: &[Symbol], body: &[Stmt]) {
        self.scopes.push(binders.iter().copied().collect());
        walk_block(body, self);
        self.scopes.pop();
    }
    // A closure defined inside a callee sees the callee's locals: its body is a nested scope here, so a
    // captured local it assigns is not mistaken for a global (a global it assigns still is one).
    fn closure(&mut self, params: &[Symbol], body: &ClosureBody) {
        self.scopes.push(params.iter().copied().collect());
        match body {
            ClosureBody::Expression(e) => walk_expr(e, self),
            ClosureBody::Block(b) => walk_block(b, self),
        }
        self.scopes.pop();
    }
    // A function defined inside a callee has its own scope chain: it does not see the enclosing locals.
    fn function(&mut self, params: &[Symbol], body: &[Stmt]) {
        let mut inner = ScopedWrites::new(params, self.out);
        walk_block(body, &mut inner);
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/writes/tests.rs"]
mod tests;
