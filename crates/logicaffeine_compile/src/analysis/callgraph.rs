use std::collections::{HashMap, HashSet};

use logicaffeine_base::{Interner, Symbol};
use logicaffeine_language::ast::stmt::{ClosureBody, ReadSource, SelectBranch, StringPart};
use logicaffeine_language::ast::{Expr, Stmt};

/// Whole-program call graph for the LOGOS compilation pipeline.
///
/// Captures all direct and closure-embedded call edges between user-defined
/// functions. Used by `ReadonlyParams` for transitive mutation detection and
/// by liveness analysis for inter-procedural precision.
pub struct CallGraph {
    /// Direct call edges: fn_sym → set of directly called function symbols.
    pub edges: HashMap<Symbol, HashSet<Symbol>>,
    /// Set of native (extern) function symbols.
    pub native_fns: HashSet<Symbol>,
    /// Strongly connected components (Kosaraju's algorithm).
    pub sccs: Vec<Vec<Symbol>>,
}

impl CallGraph {
    /// Build the call graph from a program's top-level statements.
    ///
    /// Walks all `FunctionDef` bodies, collecting `Stmt::Call` and
    /// `Expr::Call` targets, including calls inside closure bodies.
    pub fn build(stmts: &[Stmt<'_>], _interner: &Interner) -> Self {
        let mut edges: HashMap<Symbol, HashSet<Symbol>> = HashMap::new();
        let mut native_fns: HashSet<Symbol> = HashSet::new();

        for stmt in stmts {
            if let Stmt::FunctionDef {
                name,
                body,
                is_native,
                ..
            } = stmt
            {
                edges.entry(*name).or_default();
                if *is_native {
                    native_fns.insert(*name);
                } else {
                    let callees = edges.entry(*name).or_default();
                    collect_calls_from_stmts(body, callees);
                }
            }
        }

        let sccs = compute_sccs(&edges);

        Self {
            edges,
            native_fns,
            sccs,
        }
    }

    /// Returns all functions reachable from `fn_sym` via the call graph.
    ///
    /// Does not include `fn_sym` itself unless it is part of a cycle.
    pub fn reachable_from(&self, fn_sym: Symbol) -> HashSet<Symbol> {
        let mut visited = HashSet::new();
        let mut stack = Vec::new();

        if let Some(callees) = self.edges.get(&fn_sym) {
            for &c in callees {
                if c != fn_sym {
                    stack.push(c);
                }
            }
        }

        while let Some(f) = stack.pop() {
            if visited.insert(f) {
                if let Some(callees) = self.edges.get(&f) {
                    for &c in callees {
                        if !visited.contains(&c) {
                            stack.push(c);
                        }
                    }
                }
            }
        }

        visited
    }

    /// Returns `true` if `fn_sym` participates in a recursive cycle
    /// (direct self-call or mutual recursion via SCC membership).
    pub fn is_recursive(&self, fn_sym: Symbol) -> bool {
        // Direct self-edge
        if self
            .edges
            .get(&fn_sym)
            .map(|s| s.contains(&fn_sym))
            .unwrap_or(false)
        {
            return true;
        }
        // Mutual recursion: fn_sym is in an SCC with more than one member
        for scc in &self.sccs {
            if scc.len() > 1 && scc.contains(&fn_sym) {
                return true;
            }
        }
        false
    }
}

// =============================================================================
// Call collection from AST
// =============================================================================

/// The set of functions a block of statements calls *synchronously* (`Stmt::Call`
/// / `Expr::Call` / closures), NOT including spawned (`LaunchTask`) targets — a
/// spawn is a distinct concurrent thread, not a same-thread call. Used by the
/// determinacy classifier to attribute a thread's reachable print sites.
pub(crate) fn calls_in_stmts(stmts: &[Stmt<'_>]) -> HashSet<Symbol> {
    let mut calls = HashSet::new();
    collect_calls_from_stmts(stmts, &mut calls);
    calls
}

pub(crate) fn collect_calls_from_stmts(stmts: &[Stmt<'_>], calls: &mut HashSet<Symbol>) {
    for_each_call_site(stmts, &mut |function, _| {
        calls.insert(function);
    });
}

/// Every call site `stmts` holds, wherever a call can sit — statement and expression calls,
/// closure bodies, view holes, component and route bodies — as `visit(function, arguments)`. A
/// fetch's completion action and a store call are calls by name and carry no arguments here; a
/// spawn's target is not a call, though its arguments are walked. The one traversal the call graph
/// is built from.
pub(crate) fn for_each_call_site<'a>(
    stmts: &[Stmt<'a>],
    visit: &mut dyn FnMut(Symbol, &[&'a Expr<'a>]),
) {
    for stmt in stmts {
        walk_call_sites_stmt(stmt, visit);
    }
}

thread_local! {
    /// The program's functions that take `mutable` parameters, by those parameters' positions.
    /// Installed per program by [`install_lending_parameters`].
    static LENDING_PARAMETERS: std::cell::RefCell<HashMap<Symbol, Vec<usize>>> =
        std::cell::RefCell::new(HashMap::new());
}

/// The functions a program built ahead of time compiles: every function that does not need a
/// document. A function needs one when it is the browser half of a page's lowering (`__render_`,
/// `__act_`, …, `lgx_lower::is_browser_only_lowering`), when it calls a builtin only a browser has
/// (`BuiltinId::is_browser_only` — the DOM, a page's patch stream, storage, signals, WebRTC), or
/// when it calls or names a function that needs one. A binary has no document, so it carries none of
/// them: not a page's dispatcher, not vdom.lg's patch application; every other function — the
/// author's own, the server renders, the library code they run — compiles as it always did.
pub(crate) fn aot_compiled_functions(stmts: &[Stmt<'_>], interner: &Interner) -> HashSet<Symbol> {
    use logicaffeine_language::ast_depth::{for_each_node, Visited};
    let functions: HashMap<Symbol, &[Stmt<'_>]> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef { name, body, .. } => Some((*name, *body)),
            _ => None,
        })
        .collect();
    let browser_builtin = |f: Symbol| {
        crate::semantics::builtins::builtin_from_name(interner.resolve(f))
            .is_some_and(|b| b.is_browser_only())
    };
    let mut needs_a_document: HashSet<Symbol> = HashSet::new();
    let mut callers: HashMap<Symbol, Vec<Symbol>> = HashMap::new();
    for (&function, body) in &functions {
        let mut direct = logicaffeine_language::parser::lgx_lower::is_browser_only_lowering(
            interner.resolve(function),
        );
        for_each_node(body, |node| {
            let (named, called) = match node {
                Visited::Expr(Expr::Call { function, .. })
                | Visited::Stmt(Stmt::Call { function, .. }) => (*function, true),
                Visited::Expr(Expr::Identifier(sym)) => (*sym, false),
                _ => return,
            };
            if functions.contains_key(&named) {
                callers.entry(named).or_default().push(function);
            } else if called && browser_builtin(named) {
                direct = true;
            }
        });
        if direct {
            needs_a_document.insert(function);
        }
    }
    let mut pending: Vec<Symbol> = needs_a_document.iter().copied().collect();
    while let Some(f) = pending.pop() {
        for caller in callers.get(&f).into_iter().flatten() {
            if needs_a_document.insert(*caller) {
                pending.push(*caller);
            }
        }
    }
    functions
        .into_keys()
        .filter(|f| !needs_a_document.contains(f))
        .collect()
}

/// Install which positions of which of `stmts`' functions take a `mutable` parameter. A call lends
/// the binding it passes at such a position: the callee writes it, so the call is a write of that
/// binding — for the optimizer's write analysis and codegen's `let mut` alike, both of which ask
/// [`lent_positions`] of the program installed here.
pub(crate) fn install_lending_parameters(stmts: &[Stmt<'_>]) {
    let mut lending = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            is_native: false,
            ..
        } = stmt
        {
            let at: Vec<usize> = params
                .iter()
                .enumerate()
                .filter(|(_, (_, ty))| {
                    matches!(
                        ty,
                        logicaffeine_language::ast::stmt::TypeExpr::Mutable { .. }
                    )
                })
                .map(|(i, _)| i)
                .collect();
            if !at.is_empty() {
                lending.insert(*name, at);
            }
        }
    }
    LENDING_PARAMETERS.with(|l| *l.borrow_mut() = lending);
}

/// The positions of `function`'s `mutable` parameters, in the program last installed.
pub(crate) fn lent_positions(function: Symbol) -> Vec<usize> {
    LENDING_PARAMETERS.with(|l| l.borrow().get(&function).cloned().unwrap_or_default())
}

/// Every call site inside `expr`, itself included, as [`for_each_call_site`] visits them.
pub(crate) fn for_each_call_site_in_expr<'a>(
    expr: &Expr<'a>,
    visit: &mut dyn FnMut(Symbol, &[&'a Expr<'a>]),
) {
    walk_call_sites_expr(expr, visit);
}

fn walk_call_sites_stmt<'a>(stmt: &Stmt<'a>, visit: &mut dyn FnMut(Symbol, &[&'a Expr<'a>])) {
    match stmt {
        Stmt::ComponentDef(def) => {
            for f in &def.state {
                walk_call_sites_expr(f.init, visit);
            }
            for ev in &def.events {
                for_each_call_site(ev.body, visit);
            }
            for a in &def.actions {
                for_each_call_site(a.body, visit);
            }
            if let Some(view) = &def.view {
                let (view_exprs, view_blocks) = view.exprs_and_blocks();
                for e in view_exprs {
                    walk_call_sites_expr(e, visit);
                }
                for b in view_blocks {
                    for_each_call_site(b, visit);
                }
            }
        }
        Stmt::DirectiveDef { body, removed, .. } => {
            for_each_call_site(body, visit);
            if let Some(r) = removed {
                for_each_call_site(r, visit);
            }
        }
        Stmt::RouteDef { body, .. } => match &**body {
            logicaffeine_language::ast::component::RouteBody::Statements(b) => {
                for_each_call_site(b, visit)
            }
            logicaffeine_language::ast::component::RouteBody::View(view) => {
                let (view_exprs, view_blocks) = view.exprs_and_blocks();
                for e in view_exprs {
                    walk_call_sites_expr(e, visit);
                }
                for b in view_blocks {
                    for_each_call_site(b, visit);
                }
            }
        },
        Stmt::MiddlewareDef { body, .. } => for_each_call_site(body, visit),
        Stmt::RedirectDef { .. } => {}
        // A fetch's completion action and a store call are CALLS by name.
        Stmt::Fetch {
            url, body, then, ..
        } => {
            visit(*then, &[]);
            walk_call_sites_expr(url, visit);
            if let Some(b) = body {
                walk_call_sites_expr(b, visit);
            }
        }
        Stmt::Ui(u) => {
            if let logicaffeine_language::ast::stmt::UiStmt::CallStore { action, .. } = u {
                visit(*action, &[]);
            }
            for x in u.exprs() {
                walk_call_sites_expr(x, visit);
            }
        }
        Stmt::Call { function, args } => {
            visit(*function, args);
            for arg in args {
                walk_call_sites_expr(arg, visit);
            }
        }
        Stmt::Let { value, .. } => walk_call_sites_expr(value, visit),
        Stmt::Set { value, .. } => walk_call_sites_expr(value, visit),
        Stmt::Return { value: Some(v) } => walk_call_sites_expr(v, visit),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            walk_call_sites_expr(cond, visit);
            for_each_call_site(then_block, visit);
            if let Some(else_b) = else_block {
                for_each_call_site(else_b, visit);
            }
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            walk_call_sites_expr(cond, visit);
            for_each_call_site(body, visit);
            if let Some(d) = decreasing {
                walk_call_sites_expr(d, visit);
            }
        }
        Stmt::Repeat { iterable, body, .. } => {
            walk_call_sites_expr(iterable, visit);
            for_each_call_site(body, visit);
        }
        Stmt::Push { value, collection } => {
            walk_call_sites_expr(value, visit);
            walk_call_sites_expr(collection, visit);
        }
        Stmt::Pop { collection, .. } => walk_call_sites_expr(collection, visit),
        Stmt::Add { value, collection } => {
            walk_call_sites_expr(value, visit);
            walk_call_sites_expr(collection, visit);
        }
        Stmt::Remove { value, collection } => {
            walk_call_sites_expr(value, visit);
            walk_call_sites_expr(collection, visit);
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            walk_call_sites_expr(collection, visit);
            walk_call_sites_expr(index, visit);
            walk_call_sites_expr(value, visit);
        }
        Stmt::SetField { object, value, .. } => {
            walk_call_sites_expr(object, visit);
            walk_call_sites_expr(value, visit);
        }
        Stmt::Inspect { target, arms, .. } => {
            walk_call_sites_expr(target, visit);
            for arm in arms {
                for_each_call_site(arm.body, visit);
            }
        }
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            for_each_call_site(tasks, visit);
        }
        Stmt::Zone { body, .. } => for_each_call_site(body, visit),
        // Expression-bearing statements whose operands can contain calls. Notably
        // `Show` (the primary Main-body output form) — omitting these left calls
        // there invisible to callers of this walk (e.g. per-file scope checking).
        Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => {
            walk_call_sites_expr(object, visit);
            walk_call_sites_expr(recipient, visit);
        }
        Stmt::RuntimeAssert { condition, .. } => walk_call_sites_expr(condition, visit),
        // A scope-transparent desugar sequence: its body runs in the enclosing
        // scope, so its calls are this statement's calls.
        Stmt::Splice { body } | Stmt::Reversibly { body } => for_each_call_site(body, visit),
        // Test-family statements — their operands and bodies bear calls that a
        // per-file scope check must see (a leaked reference is just as invalid
        // inside a `## Test` as in Main).
        Stmt::TestDef { body, cases, .. } => {
            for_each_call_site(body, visit);
            if let Some(table) = cases {
                for row in &table.rows {
                    for cell in row {
                        walk_call_sites_expr(cell, visit);
                    }
                }
            }
        }
        Stmt::Expect { condition } => walk_call_sites_expr(condition, visit),
        Stmt::ExpectFail { body, .. } => for_each_call_site(body, visit),
        Stmt::AdvanceTime { millis } => walk_call_sites_expr(millis, visit),
        Stmt::Observe { payloads, .. } => {
            for (_, value) in payloads {
                walk_call_sites_expr(value, visit);
            }
        }
        Stmt::During { body, .. } => for_each_call_site(body, visit),
        Stmt::ForAll {
            constraint, body, ..
        } => {
            if let Some(c) = constraint {
                walk_call_sites_expr(c, visit);
            }
            for_each_call_site(body, visit);
        }
        // I/O and networking — every operand is evaluated same-thread.
        Stmt::ReadFrom { source, .. } => {
            if let ReadSource::File(path) = source {
                walk_call_sites_expr(path, visit);
            }
        }
        Stmt::WriteFile { content, path } => {
            walk_call_sites_expr(content, visit);
            walk_call_sites_expr(path, visit);
        }
        Stmt::SendMessage {
            message,
            destination,
            ..
        } => {
            walk_call_sites_expr(message, visit);
            walk_call_sites_expr(destination, visit);
        }
        Stmt::AwaitMessage { source, .. } => walk_call_sites_expr(source, visit),
        Stmt::StreamMessage {
            values,
            destination,
        } => {
            walk_call_sites_expr(values, visit);
            walk_call_sites_expr(destination, visit);
        }
        Stmt::MergeCrdt { source, target } | Stmt::JoinCrdt { source, target } => {
            walk_call_sites_expr(source, visit);
            walk_call_sites_expr(target, visit);
        }
        Stmt::IncreaseCrdt { object, amount, .. } | Stmt::DecreaseCrdt { object, amount, .. } => {
            walk_call_sites_expr(object, visit);
            walk_call_sites_expr(amount, visit);
        }
        Stmt::AppendToSequence { sequence, value } => {
            walk_call_sites_expr(sequence, visit);
            walk_call_sites_expr(value, visit);
        }
        Stmt::ResolveConflict { object, value, .. } => {
            walk_call_sites_expr(object, visit);
            walk_call_sites_expr(value, visit);
        }
        Stmt::Listen { address, .. } | Stmt::ConnectTo { address, .. } => {
            walk_call_sites_expr(address, visit);
        }
        Stmt::Serve { port, handler } => {
            walk_call_sites_expr(port, visit);
            walk_call_sites_expr(handler, visit);
        }
        Stmt::LetPeerAgent { address, .. } => walk_call_sites_expr(address, visit),
        Stmt::Sleep { milliseconds } => walk_call_sites_expr(milliseconds, visit),
        Stmt::Sync { topic, .. } => walk_call_sites_expr(topic, visit),
        Stmt::Mount { path, .. } => walk_call_sites_expr(path, visit),
        // Green-thread pipes: operands are same-thread; the SPAWN targets of
        // `LaunchTask`/`LaunchTaskWithHandle` are deliberately NOT treated as
        // call edges (a spawn is a distinct thread, per `calls_in_stmts`), but
        // their arguments ARE evaluated here and must be walked.
        Stmt::LaunchTask { args, .. } | Stmt::LaunchTaskWithHandle { args, .. } => {
            for arg in args {
                walk_call_sites_expr(arg, visit);
            }
        }
        Stmt::SendPipe { value, pipe } | Stmt::TrySendPipe { value, pipe, .. } => {
            walk_call_sites_expr(value, visit);
            walk_call_sites_expr(pipe, visit);
        }
        Stmt::ReceivePipe { pipe, .. } | Stmt::TryReceivePipe { pipe, .. } => {
            walk_call_sites_expr(pipe, visit);
        }
        Stmt::StopTask { handle } => walk_call_sites_expr(handle, visit),
        Stmt::Select { branches } => {
            for branch in branches {
                match branch {
                    SelectBranch::Receive { pipe, body, .. } => {
                        walk_call_sites_expr(pipe, visit);
                        for_each_call_site(body, visit);
                    }
                    SelectBranch::Timeout { milliseconds, body } => {
                        walk_call_sites_expr(milliseconds, visit);
                        for_each_call_site(body, visit);
                    }
                }
            }
        }
        _ => {}
    }
}

fn walk_call_sites_expr<'a>(expr: &Expr<'a>, visit: &mut dyn FnMut(Symbol, &[&'a Expr<'a>])) {
    match expr {
        Expr::Dom(q) => {
            if let Some(sel) = q.selector() {
                walk_call_sites_expr(sel, visit);
            }
        }
        Expr::View(v) => {
            let (view_exprs, view_blocks) = v.exprs_and_blocks();
            for e in view_exprs {
                walk_call_sites_expr(e, visit);
            }
            for b in view_blocks {
                for_each_call_site(b, visit);
            }
        }
        Expr::Call { function, args } => {
            visit(*function, args);
            for arg in args {
                walk_call_sites_expr(arg, visit);
            }
        }
        Expr::Closure { body, .. } => match body {
            ClosureBody::Expression(e) => walk_call_sites_expr(e, visit),
            ClosureBody::Block(stmts) => for_each_call_site(stmts, visit),
        },
        Expr::BinaryOp { left, right, .. } => {
            walk_call_sites_expr(left, visit);
            walk_call_sites_expr(right, visit);
        }
        Expr::Index { collection, index } => {
            walk_call_sites_expr(collection, visit);
            walk_call_sites_expr(index, visit);
        }
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            walk_call_sites_expr(collection, visit);
            walk_call_sites_expr(start, visit);
            walk_call_sites_expr(end, visit);
        }
        Expr::Length { collection } => walk_call_sites_expr(collection, visit),
        Expr::Contains { collection, value } => {
            walk_call_sites_expr(collection, visit);
            walk_call_sites_expr(value, visit);
        }
        Expr::Union { left, right } | Expr::Intersection { left, right } => {
            walk_call_sites_expr(left, visit);
            walk_call_sites_expr(right, visit);
        }
        Expr::FieldAccess { object, .. } => walk_call_sites_expr(object, visit),
        Expr::List(items) | Expr::Tuple(items) => {
            for item in items {
                walk_call_sites_expr(item, visit);
            }
        }
        Expr::Range { start, end } => {
            walk_call_sites_expr(start, visit);
            walk_call_sites_expr(end, visit);
        }
        Expr::Copy { expr } | Expr::Give { value: expr } => {
            walk_call_sites_expr(expr, visit);
        }
        Expr::OptionSome { value } => walk_call_sites_expr(value, visit),
        Expr::WithCapacity { value, capacity } => {
            walk_call_sites_expr(value, visit);
            walk_call_sites_expr(capacity, visit);
        }
        Expr::CallExpr { callee, args } => {
            walk_call_sites_expr(callee, visit);
            for arg in args {
                walk_call_sites_expr(arg, visit);
            }
        }
        Expr::Not { operand } => walk_call_sites_expr(operand, visit),
        Expr::ManifestOf { zone } => walk_call_sites_expr(zone, visit),
        Expr::ChunkAt { index, zone } => {
            walk_call_sites_expr(index, visit);
            walk_call_sites_expr(zone, visit);
        }
        // A `{f(x)}` hole is a real call — one that carries the component View's
        // policy calls — and was previously dropped by the wildcard arm, so the
        // per-file scope check never saw it. The `New`/`NewVariant` field values
        // are ordinary expressions too and can hold calls.
        Expr::InterpolatedString(parts) => {
            for part in parts {
                if let StringPart::Expr { value, .. } = part {
                    walk_call_sites_expr(value, visit);
                }
            }
        }
        Expr::New { init_fields, .. } => {
            for (_, value) in init_fields {
                walk_call_sites_expr(value, visit);
            }
        }
        Expr::NewVariant { fields, .. } => {
            for (_, value) in fields {
                walk_call_sites_expr(value, visit);
            }
        }
        // Leaves and non-expression-bearing forms carry no call site. Kept
        // EXHAUSTIVE (no wildcard) so a new `Expr` variant with an embedded
        // expression fails this build until its call positions are wired.
        Expr::Literal(_) | Expr::Identifier(_) | Expr::OptionNone | Expr::Escape { .. } => {}
    }
}

// =============================================================================
// Type-reference collection (for per-file privacy of TYPES)
// =============================================================================

/// Every user-type name a block of statements REFERENCES — constructions
/// (`a new Point`), enum constructions, and type annotations. Used by the per-file
/// scope check to gate a `private` type: a reference to it from any file other
/// than its owner is rejected, exactly like a private function call. The traversal
/// mirrors the call walk so no reference position is missed.
pub(crate) fn collect_type_refs_from_stmts(stmts: &[Stmt<'_>], types: &mut HashSet<Symbol>) {
    for stmt in stmts {
        collect_type_refs_from_stmt(stmt, types);
    }
}

pub(crate) fn type_names_in_type_expr(
    ty: &logicaffeine_language::ast::stmt::TypeExpr<'_>,
    types: &mut HashSet<Symbol>,
) {
    use logicaffeine_language::ast::stmt::TypeExpr;
    match ty {
        // `Named` is a user-defined type; `Primitive` (Int/Text/…) never is.
        TypeExpr::Named(sym) => {
            types.insert(*sym);
        }
        TypeExpr::Generic { base, params } => {
            types.insert(*base);
            for p in params.iter() {
                type_names_in_type_expr(p, types);
            }
        }
        TypeExpr::Tuple(params) => {
            for p in params.iter() {
                type_names_in_type_expr(p, types);
            }
        }
        TypeExpr::Record { field_types, .. } => {
            for p in field_types.iter() {
                type_names_in_type_expr(p, types);
            }
        }
        TypeExpr::Function { inputs, output } => {
            for i in inputs.iter() {
                type_names_in_type_expr(i, types);
            }
            type_names_in_type_expr(output, types);
        }
        TypeExpr::Refinement { base, .. } => type_names_in_type_expr(base, types),
        TypeExpr::Persistent { inner } | TypeExpr::Mutable { inner } => {
            type_names_in_type_expr(inner, types)
        }
        TypeExpr::Primitive(_) => {}
    }
}

fn collect_type_refs_from_stmt(stmt: &Stmt<'_>, types: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::ComponentDef(def) => {
            for (_, t) in &def.props {
                type_names_in_type_expr(t, types);
            }
            for f in &def.state {
                if let Some(t) = f.ty {
                    type_names_in_type_expr(t, types);
                }
                collect_type_refs_from_expr(f.init, types);
            }
            for ev in &def.events {
                for s in ev.body.iter() {
                    collect_type_refs_from_stmt(s, types);
                }
            }
            for a in &def.actions {
                for (_, t) in &a.params {
                    type_names_in_type_expr(t, types);
                }
                for s in a.body.iter() {
                    collect_type_refs_from_stmt(s, types);
                }
            }
            if let Some(view) = &def.view {
                let (view_exprs, view_blocks) = view.exprs_and_blocks();
                for e in view_exprs {
                    collect_type_refs_from_expr(e, types);
                }
                for b in view_blocks {
                    for s in b.iter() {
                        collect_type_refs_from_stmt(s, types);
                    }
                }
            }
        }
        Stmt::DirectiveDef {
            params,
            body,
            removed,
            ..
        } => {
            for (_, t) in params {
                type_names_in_type_expr(t, types);
            }
            for s in body.iter() {
                collect_type_refs_from_stmt(s, types);
            }
            if let Some(r) = removed {
                for s in r.iter() {
                    collect_type_refs_from_stmt(s, types);
                }
            }
        }
        Stmt::RouteDef {
            params,
            return_type,
            body,
            ..
        } => {
            for (_, t) in params {
                type_names_in_type_expr(t, types);
            }
            if let Some(t) = return_type {
                type_names_in_type_expr(t, types);
            }
            match &**body {
                logicaffeine_language::ast::component::RouteBody::Statements(b) => {
                    for s in b.iter() {
                        collect_type_refs_from_stmt(s, types);
                    }
                }
                logicaffeine_language::ast::component::RouteBody::View(view) => {
                    let (view_exprs, view_blocks) = view.exprs_and_blocks();
                    for e in view_exprs {
                        collect_type_refs_from_expr(e, types);
                    }
                    for b in view_blocks {
                        for s in b.iter() {
                            collect_type_refs_from_stmt(s, types);
                        }
                    }
                }
            }
        }
        Stmt::MiddlewareDef { param, body } => {
            type_names_in_type_expr(param.1, types);
            for s in body.iter() {
                collect_type_refs_from_stmt(s, types);
            }
        }
        Stmt::RedirectDef { .. } => {}
        Stmt::Fetch {
            url, body, as_type, ..
        } => {
            type_names_in_type_expr(as_type, types);
            collect_type_refs_from_expr(url, types);
            if let Some(b) = body {
                collect_type_refs_from_expr(b, types);
            }
        }
        Stmt::Ui(u) => {
            for x in u.exprs() {
                collect_type_refs_from_expr(x, types);
            }
        }
        Stmt::Call { args, .. } => {
            for arg in args {
                collect_type_refs_from_expr(arg, types);
            }
        }
        Stmt::Let { value, ty, .. } => {
            collect_type_refs_from_expr(value, types);
            if let Some(t) = ty {
                type_names_in_type_expr(t, types);
            }
        }
        Stmt::Set { value, .. } => collect_type_refs_from_expr(value, types),
        Stmt::Return { value: Some(v) } => collect_type_refs_from_expr(v, types),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            collect_type_refs_from_expr(cond, types);
            collect_type_refs_from_stmts(then_block, types);
            if let Some(else_b) = else_block {
                collect_type_refs_from_stmts(else_b, types);
            }
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            collect_type_refs_from_expr(cond, types);
            collect_type_refs_from_stmts(body, types);
            if let Some(d) = decreasing {
                collect_type_refs_from_expr(d, types);
            }
        }
        Stmt::Repeat { iterable, body, .. } => {
            collect_type_refs_from_expr(iterable, types);
            collect_type_refs_from_stmts(body, types);
        }
        Stmt::Push { value, collection }
        | Stmt::Add { value, collection }
        | Stmt::Remove { value, collection } => {
            collect_type_refs_from_expr(value, types);
            collect_type_refs_from_expr(collection, types);
        }
        Stmt::Pop { collection, .. } => collect_type_refs_from_expr(collection, types),
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            collect_type_refs_from_expr(collection, types);
            collect_type_refs_from_expr(index, types);
            collect_type_refs_from_expr(value, types);
        }
        Stmt::SetField { object, value, .. } => {
            collect_type_refs_from_expr(object, types);
            collect_type_refs_from_expr(value, types);
        }
        Stmt::Inspect { target, arms, .. } => {
            collect_type_refs_from_expr(target, types);
            for arm in arms {
                collect_type_refs_from_stmts(arm.body, types);
            }
        }
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            collect_type_refs_from_stmts(tasks, types)
        }
        Stmt::Zone { body, .. }
        | Stmt::Splice { body }
        | Stmt::During { body, .. }
        | Stmt::Reversibly { body } => collect_type_refs_from_stmts(body, types),
        Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => {
            collect_type_refs_from_expr(object, types);
            collect_type_refs_from_expr(recipient, types);
        }
        Stmt::RuntimeAssert { condition, .. } | Stmt::Expect { condition } => {
            collect_type_refs_from_expr(condition, types)
        }
        Stmt::TestDef { body, cases, .. } => {
            collect_type_refs_from_stmts(body, types);
            if let Some(table) = cases {
                for row in &table.rows {
                    for cell in row {
                        collect_type_refs_from_expr(cell, types);
                    }
                }
            }
        }
        Stmt::ExpectFail { body, .. } => collect_type_refs_from_stmts(body, types),
        Stmt::AdvanceTime { millis } => collect_type_refs_from_expr(millis, types),
        Stmt::Observe { payloads, .. } => {
            for (_, value) in payloads {
                collect_type_refs_from_expr(value, types);
            }
        }
        Stmt::ForAll {
            constraint, body, ..
        } => {
            if let Some(c) = constraint {
                collect_type_refs_from_expr(c, types);
            }
            collect_type_refs_from_stmts(body, types);
        }
        Stmt::ReadFrom { source, .. } => {
            if let ReadSource::File(path) = source {
                collect_type_refs_from_expr(path, types);
            }
        }
        Stmt::WriteFile { content, path } => {
            collect_type_refs_from_expr(content, types);
            collect_type_refs_from_expr(path, types);
        }
        Stmt::SendMessage {
            message,
            destination,
            ..
        } => {
            collect_type_refs_from_expr(message, types);
            collect_type_refs_from_expr(destination, types);
        }
        Stmt::AwaitMessage { source, .. } => collect_type_refs_from_expr(source, types),
        Stmt::StreamMessage {
            values,
            destination,
        } => {
            collect_type_refs_from_expr(values, types);
            collect_type_refs_from_expr(destination, types);
        }
        Stmt::MergeCrdt { source, target } | Stmt::JoinCrdt { source, target } => {
            collect_type_refs_from_expr(source, types);
            collect_type_refs_from_expr(target, types);
        }
        Stmt::IncreaseCrdt { object, amount, .. } | Stmt::DecreaseCrdt { object, amount, .. } => {
            collect_type_refs_from_expr(object, types);
            collect_type_refs_from_expr(amount, types);
        }
        Stmt::AppendToSequence { sequence, value } => {
            collect_type_refs_from_expr(sequence, types);
            collect_type_refs_from_expr(value, types);
        }
        Stmt::ResolveConflict { object, value, .. } => {
            collect_type_refs_from_expr(object, types);
            collect_type_refs_from_expr(value, types);
        }
        Stmt::Listen { address, .. }
        | Stmt::ConnectTo { address, .. }
        | Stmt::LetPeerAgent { address, .. } => collect_type_refs_from_expr(address, types),
        Stmt::Serve { port, handler } => {
            collect_type_refs_from_expr(port, types);
            collect_type_refs_from_expr(handler, types);
        }
        Stmt::Sleep { milliseconds } => collect_type_refs_from_expr(milliseconds, types),
        Stmt::Sync { topic, .. } => collect_type_refs_from_expr(topic, types),
        Stmt::Mount { path, .. } => collect_type_refs_from_expr(path, types),
        Stmt::LaunchTask { args, .. } | Stmt::LaunchTaskWithHandle { args, .. } => {
            for arg in args {
                collect_type_refs_from_expr(arg, types);
            }
        }
        Stmt::SendPipe { value, pipe } | Stmt::TrySendPipe { value, pipe, .. } => {
            collect_type_refs_from_expr(value, types);
            collect_type_refs_from_expr(pipe, types);
        }
        Stmt::ReceivePipe { pipe, .. } | Stmt::TryReceivePipe { pipe, .. } => {
            collect_type_refs_from_expr(pipe, types)
        }
        Stmt::StopTask { handle } => collect_type_refs_from_expr(handle, types),
        Stmt::Select { branches } => {
            for branch in branches {
                match branch {
                    SelectBranch::Receive { pipe, body, .. } => {
                        collect_type_refs_from_expr(pipe, types);
                        collect_type_refs_from_stmts(body, types);
                    }
                    SelectBranch::Timeout { milliseconds, body } => {
                        collect_type_refs_from_expr(milliseconds, types);
                        collect_type_refs_from_stmts(body, types);
                    }
                }
            }
        }
        _ => {}
    }
}

fn collect_type_refs_from_expr(expr: &Expr<'_>, types: &mut HashSet<Symbol>) {
    match expr {
        Expr::Dom(q) => {
            if let Some(sel) = q.selector() {
                collect_type_refs_from_expr(sel, types);
            }
        }
        Expr::View(v) => {
            let (view_exprs, view_blocks) = v.exprs_and_blocks();
            for e in view_exprs {
                collect_type_refs_from_expr(e, types);
            }
            for b in view_blocks {
                for s in b.iter() {
                    collect_type_refs_from_stmt(s, types);
                }
            }
        }
        Expr::New {
            type_name,
            type_args,
            init_fields,
        } => {
            types.insert(*type_name);
            for t in type_args.iter() {
                type_names_in_type_expr(t, types);
            }
            for (_, value) in init_fields {
                collect_type_refs_from_expr(value, types);
            }
        }
        Expr::NewVariant {
            enum_name, fields, ..
        } => {
            types.insert(*enum_name);
            for (_, value) in fields {
                collect_type_refs_from_expr(value, types);
            }
        }
        Expr::Call { args, .. } => {
            for arg in args {
                collect_type_refs_from_expr(arg, types);
            }
        }
        Expr::Closure { body, .. } => match body {
            ClosureBody::Expression(e) => collect_type_refs_from_expr(e, types),
            ClosureBody::Block(stmts) => collect_type_refs_from_stmts(stmts, types),
        },
        Expr::BinaryOp { left, right, .. } => {
            collect_type_refs_from_expr(left, types);
            collect_type_refs_from_expr(right, types);
        }
        Expr::Index { collection, index } => {
            collect_type_refs_from_expr(collection, types);
            collect_type_refs_from_expr(index, types);
        }
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            collect_type_refs_from_expr(collection, types);
            collect_type_refs_from_expr(start, types);
            collect_type_refs_from_expr(end, types);
        }
        Expr::Length { collection } => collect_type_refs_from_expr(collection, types),
        Expr::Contains { collection, value } => {
            collect_type_refs_from_expr(collection, types);
            collect_type_refs_from_expr(value, types);
        }
        Expr::Union { left, right } | Expr::Intersection { left, right } => {
            collect_type_refs_from_expr(left, types);
            collect_type_refs_from_expr(right, types);
        }
        Expr::FieldAccess { object, .. } => collect_type_refs_from_expr(object, types),
        Expr::List(items) | Expr::Tuple(items) => {
            for item in items {
                collect_type_refs_from_expr(item, types);
            }
        }
        Expr::Range { start, end } => {
            collect_type_refs_from_expr(start, types);
            collect_type_refs_from_expr(end, types);
        }
        Expr::Copy { expr } | Expr::Give { value: expr } => {
            collect_type_refs_from_expr(expr, types)
        }
        Expr::OptionSome { value } => collect_type_refs_from_expr(value, types),
        Expr::WithCapacity { value, capacity } => {
            collect_type_refs_from_expr(value, types);
            collect_type_refs_from_expr(capacity, types);
        }
        Expr::CallExpr { callee, args } => {
            collect_type_refs_from_expr(callee, types);
            for arg in args {
                collect_type_refs_from_expr(arg, types);
            }
        }
        Expr::Not { operand } => collect_type_refs_from_expr(operand, types),
        Expr::ManifestOf { zone } => collect_type_refs_from_expr(zone, types),
        Expr::ChunkAt { index, zone } => {
            collect_type_refs_from_expr(index, types);
            collect_type_refs_from_expr(zone, types);
        }
        // A `{a new Point}` hole references a type just like a bare construction;
        // the wildcard arm used to drop it, leaking a private type used only in a
        // component's interpolated view. Kept EXHAUSTIVE (no wildcard) so a new
        // expression variant fails this build until its type positions are wired.
        Expr::InterpolatedString(parts) => {
            for part in parts {
                if let StringPart::Expr { value, .. } = part {
                    collect_type_refs_from_expr(value, types);
                }
            }
        }
        Expr::Literal(_) | Expr::Identifier(_) | Expr::OptionNone | Expr::Escape { .. } => {}
    }
}

/// The records and enums the program's generated code defines: every one its registry holds except
/// a compiler-provided struct (`TypeRegistry::is_builtin_struct`, `HttpError`/`File`) the program
/// never names — in its own type references (a construction, an annotation), or in a field of a type
/// it defines. The one answer the AOT-Rust and C emitters share, so a program pays for a builtin
/// struct only when it uses one.
pub(crate) fn program_type_defs(
    stmts: &[Stmt<'_>],
    registry: &crate::analysis::TypeRegistry,
) -> HashSet<Symbol> {
    use crate::analysis::TypeDef;
    let mut named = HashSet::new();
    collect_type_refs_from_stmts(stmts, &mut named);
    let mut defined: HashSet<Symbol> = registry
        .iter_types()
        .filter(|(sym, def)| {
            matches!(def, TypeDef::Struct { .. } | TypeDef::Enum { .. })
                && (!registry.is_builtin_struct(**sym) || named.contains(*sym))
        })
        .map(|(sym, _)| *sym)
        .collect();
    loop {
        let mut reached = HashSet::new();
        for sym in &defined {
            let fields: Vec<&crate::analysis::FieldType> = match registry.get(*sym) {
                Some(TypeDef::Struct { fields, .. }) => fields.iter().map(|f| &f.ty).collect(),
                Some(TypeDef::Enum { variants, .. }) => variants
                    .iter()
                    .flat_map(|v| v.fields.iter().map(|f| &f.ty))
                    .collect(),
                _ => Vec::new(),
            };
            for ty in fields {
                field_type_names(ty, &mut reached);
            }
        }
        let before = defined.len();
        defined.extend(
            reached
                .into_iter()
                .filter(|sym| registry.is_builtin_struct(*sym)),
        );
        if defined.len() == before {
            return defined;
        }
    }
}

/// Every type name a registry field type mentions.
fn field_type_names(ty: &crate::analysis::FieldType, names: &mut HashSet<Symbol>) {
    use crate::analysis::FieldType;
    match ty {
        FieldType::Primitive(sym) | FieldType::Named(sym) | FieldType::TypeParam(sym) => {
            names.insert(*sym);
        }
        FieldType::Generic { base, params } => {
            names.insert(*base);
            for p in params {
                field_type_names(p, names);
            }
        }
        FieldType::Tuple(items) => {
            for t in items {
                field_type_names(t, names);
            }
        }
        FieldType::Record(fields) => {
            for (_, t) in fields {
                field_type_names(t, names);
            }
        }
    }
}

// =============================================================================
// Kosaraju's SCC algorithm
// =============================================================================

fn compute_sccs(edges: &HashMap<Symbol, HashSet<Symbol>>) -> Vec<Vec<Symbol>> {
    let nodes: Vec<Symbol> = edges.keys().copied().collect();

    // Step 1: DFS on forward graph to compute finish order
    let mut visited: HashSet<Symbol> = HashSet::new();
    let mut finish_order: Vec<Symbol> = Vec::new();

    for &v in &nodes {
        if !visited.contains(&v) {
            dfs_finish(v, edges, &mut visited, &mut finish_order);
        }
    }

    // Step 2: Build reversed graph
    let mut rev_edges: HashMap<Symbol, HashSet<Symbol>> = HashMap::new();
    for (&src, callees) in edges {
        for &dst in callees {
            rev_edges.entry(dst).or_default().insert(src);
        }
    }

    // Step 3: DFS on reversed graph in reverse finish order to collect SCCs
    let mut visited2: HashSet<Symbol> = HashSet::new();
    let mut sccs: Vec<Vec<Symbol>> = Vec::new();

    for &v in finish_order.iter().rev() {
        if !visited2.contains(&v) {
            let mut scc = Vec::new();
            dfs_collect(v, &rev_edges, &mut visited2, &mut scc);
            sccs.push(scc);
        }
    }

    sccs
}

fn dfs_finish(
    v: Symbol,
    edges: &HashMap<Symbol, HashSet<Symbol>>,
    visited: &mut HashSet<Symbol>,
    finish_order: &mut Vec<Symbol>,
) {
    if !visited.insert(v) {
        return;
    }
    if let Some(callees) = edges.get(&v) {
        for &callee in callees {
            dfs_finish(callee, edges, visited, finish_order);
        }
    }
    finish_order.push(v);
}

fn dfs_collect(
    v: Symbol,
    edges: &HashMap<Symbol, HashSet<Symbol>>,
    visited: &mut HashSet<Symbol>,
    scc: &mut Vec<Symbol>,
) {
    if !visited.insert(v) {
        return;
    }
    scc.push(v);
    if let Some(callees) = edges.get(&v) {
        for &callee in callees {
            dfs_collect(callee, edges, visited, scc);
        }
    }
}
