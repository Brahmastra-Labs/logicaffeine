//! The AST depth gate — "parsed ⇒ bounded".
//!
//! Every consumer downstream of the parser (optimizer, codegen, tree-walker,
//! VM compiler, transpiler) walks the AST **recursively**; a tree nested
//! thousands of levels deep (a 10,000-term `1+1+…` chain, a parenthesis
//! tower, a generated block pyramid) overflows their stacks and aborts the
//! process — on every surface at once (CLI, REPL, LSP, web Studio).
//!
//! The gate enforces one contract at the single choke point
//! ([`Parser::parse_program`](crate::Parser)): any program that parses has
//! nesting depth ≤ the enforced limit ([`max_ast_depth`]), so downstream
//! recursion is bounded by construction. Programs past the limit get a graceful
//! [`AstTooDeep`](crate::error::ParseErrorKind::AstTooDeep) diagnostic with
//! a Socratic fix (split into `Let` bindings).
//!
//! The walker itself is **iterative** (an explicit work stack) — measuring
//! the depth must never be the thing that overflows. Its matches are
//! **exhaustive with no wildcard**: adding an AST variant with children
//! fails compilation here, forcing the author to classify the new
//! children — the ratchet that keeps the gate honest as the language grows.
//!
//! Boxed noun-phrase payloads (`Categorical`, `Relation`, `NeoEvent`,
//! comparatives) carry [`Term`](crate::ast::logic::Term)s whose nesting is
//! bounded by English sentence structure, not by program size — they count
//! as one level.

use crate::ast::logic::LogicExpr;
use crate::ast::stmt::{ClosureBody, Expr, ReadSource, SelectBranch, Stmt, StringPart, TypeExpr};
use crate::error::{ParseError, ParseErrorKind};
use crate::token::Span;

/// The DEFAULT maximum AST nesting depth.
///
/// The budget, measured: the parser's own descent costs ~7 KiB of stack per
/// nesting level in debug builds (the fattest case — release and the
/// downstream walkers are far leaner), and the tightest standard
/// environments give ~2 MiB (worker threads) or ~1 MiB (browser wasm).
/// 128 levels keeps the worst case inside the smallest stack with margin,
/// while being ~3× deeper than any hand-written program observed in the
/// corpus. Machines with deep stacks raise it via `LOGOS_MAX_AST_DEPTH`
/// (see [`max_ast_depth`]) — the limit protects the environment, so the
/// environment gets to size it. Enforced at parse time (recursion guard)
/// AND on the built tree, covering parenthesis towers, block pyramids, and
/// iteratively-built operator chains alike.
pub const DEFAULT_MAX_AST_DEPTH: usize = 128;

/// The smallest limit the override accepts — below this, ordinary programs
/// stop parsing.
pub const MIN_AST_DEPTH: usize = 16;

/// The EFFECTIVE depth limit: [`DEFAULT_MAX_AST_DEPTH`] unless the
/// `LOGOS_MAX_AST_DEPTH` environment variable overrides it.
///
/// The default is sized for the tightest standard environment; machines
/// with deep stacks (a big `ulimit -s` / `RUST_MIN_STACK`) can raise it,
/// and constrained embedders can lower it — no rebuild required. Read once
/// per process.
pub fn max_ast_depth() -> usize {
    static LIMIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| limit_from(std::env::var("LOGOS_MAX_AST_DEPTH").ok().as_deref()))
}

/// Pure resolution of the override value (unit-testable without touching
/// process environment): a parseable value is clamped to at least
/// [`MIN_AST_DEPTH`]; anything else falls back to the default.
pub fn limit_from(env_value: Option<&str>) -> usize {
    match env_value.and_then(|v| v.trim().parse::<usize>().ok()) {
        Some(n) => n.max(MIN_AST_DEPTH),
        None => DEFAULT_MAX_AST_DEPTH,
    }
}

/// The parse-time recursion budget: an EIGHTH of the AST limit (min the floor).
///
/// The parser's own descent is BY FAR the fattest stack consumer — each parenthesis level
/// re-enters the whole precedence/condition chain — so it gets a much tighter budget than the
/// built-tree limit. The number that matters is the ratio between one level's frame and the
/// smallest stack the parser must survive on:
///
/// | measured                                    | value       |
/// |---------------------------------------------|-------------|
/// | one level of descent, debug build (2026-09) | ~64 KiB     |
/// | smallest stack the gate is tested on        | 2 MiB       |
/// | levels before the real stack dies           | 32          |
/// | this budget at the default limit            | 16          |
///
/// The budget must fire BEFORE the stack dies, or the guard is decorative: a `SIGABRT` is
/// exactly the crash it exists to prevent. It was a QUARTER of the limit, which put it at 32 —
/// the cliff itself, with no margin — so any growth of the descent's frame turned a graceful
/// rejection into a stack overflow, and one did. An eighth spends half the stack instead of
/// all of it, which is room for the parser to keep growing.
///
/// Still scales with `LOGOS_MAX_AST_DEPTH`: raising it asserts a deeper stack, and raises both
/// together as before.
pub fn max_parse_recursion() -> usize {
    (max_ast_depth() / 8).max(MIN_AST_DEPTH)
}

/// A work-stack node: any AST reference that can contain further nesting.
enum Node<'w, 'a> {
    S(&'w Stmt<'a>),
    E(&'w Expr<'a>),
    T(&'w TypeExpr<'a>),
    L(&'w LogicExpr<'a>),
}

/// Measure the maximum nesting depth of a parsed program, iteratively.
pub fn program_depth(stmts: &[Stmt<'_>]) -> usize {
    let mut work: Vec<(Node<'_, '_>, usize)> = stmts.iter().map(|s| (Node::S(s), 1)).collect();
    let mut max_depth = 0usize;

    while let Some((node, depth)) = work.pop() {
        max_depth = max_depth.max(depth);
        let d = depth + 1;
        match node {
            Node::S(stmt) => push_stmt_children(stmt, d, &mut work),
            Node::E(expr) => push_expr_children(expr, d, &mut work),
            Node::T(ty) => push_type_children(ty, d, &mut work),
            Node::L(logic) => push_logic_children(logic, d, &mut work),
        }
    }
    max_depth
}

/// A node [`for_each_node`] hands its visitor.
pub enum Visited<'w, 'a> {
    Stmt(&'w Stmt<'a>),
    Expr(&'w Expr<'a>),
}

/// Visit every statement and expression in `stmts`, each once — the same exhaustive child
/// enumeration the depth gate walks, so a new variant's children are reached here the moment the
/// gate reaches them. Iterative, like the gate: visiting must never be what overflows.
pub fn for_each_node<'w, 'a>(stmts: &'w [Stmt<'a>], mut visit: impl FnMut(Visited<'w, 'a>)) {
    walk_nodes(stmts, |node| {
        visit(node);
        Walk::Into
    });
}

/// Whether a [`walk_nodes`] visitor goes on into the children of the node it was handed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Walk {
    /// Visit the node's children too.
    Into,
    /// Leave the node's children unvisited — another scope's, say: a function's body is not the
    /// scope that defines the function.
    Past,
}

/// [`for_each_node`], with the visitor answering for each statement and expression whether the walk
/// goes on into its children. Exhaustive in the same way: a child is left unvisited only because a
/// visitor said so, never because the walk does not know the kind of node it is in.
pub fn walk_nodes<'w, 'a>(stmts: &'w [Stmt<'a>], mut visit: impl FnMut(Visited<'w, 'a>) -> Walk) {
    let mut work: Vec<(Node<'w, 'a>, usize)> = stmts.iter().map(|s| (Node::S(s), 1)).collect();
    while let Some((node, depth)) = work.pop() {
        let d = depth + 1;
        match node {
            Node::S(stmt) => {
                if visit(Visited::Stmt(stmt)) == Walk::Into {
                    push_stmt_children(stmt, d, &mut work);
                }
            }
            Node::E(expr) => {
                if visit(Visited::Expr(expr)) == Walk::Into {
                    push_expr_children(expr, d, &mut work);
                }
            }
            Node::T(ty) => push_type_children(ty, d, &mut work),
            Node::L(logic) => push_logic_children(logic, d, &mut work),
        }
    }
}

/// The expressions an expression evaluates directly — its operands, arguments, elements — in the same
/// exhaustive enumeration the depth gate walks. A closure's body is not among them: it runs when the
/// closure is called, not when the closure expression is evaluated.
pub fn expr_children<'w, 'a>(expr: &'w Expr<'a>) -> Vec<&'w Expr<'a>> {
    if matches!(expr, Expr::Closure { .. }) {
        return Vec::new();
    }
    let mut work = Vec::new();
    push_expr_children(expr, 0, &mut work);
    work.into_iter()
        .filter_map(|(node, _)| match node {
            Node::E(e) => Some(e),
            _ => None,
        })
        .collect()
}

/// The expressions a statement evaluates directly (its value, condition, target, arguments — not the
/// statements of its nested blocks).
pub fn stmt_expr_children<'w, 'a>(stmt: &'w Stmt<'a>) -> Vec<&'w Expr<'a>> {
    let mut work = Vec::new();
    push_stmt_children(stmt, 0, &mut work);
    work.into_iter()
        .filter_map(|(node, _)| match node {
            Node::E(e) => Some(e),
            _ => None,
        })
        .collect()
}

/// Validate a parsed program against the effective limit
/// ([`max_ast_depth`]).
pub fn validate_program_depth(stmts: &[Stmt<'_>], span: Span) -> Result<(), ParseError> {
    let limit = max_ast_depth();
    let depth = program_depth(stmts);
    if depth > limit {
        return Err(ParseError {
            kind: ParseErrorKind::AstTooDeep {
                depth,
                max_depth: limit,
            },
            span,
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_language/ast_depth/tests.rs"]
mod tests;

fn push_stmt_children<'w, 'a>(stmt: &'w Stmt<'a>, d: usize, work: &mut Vec<(Node<'w, 'a>, usize)>) {
    macro_rules! e {
        ($x:expr) => {
            work.push((Node::E($x), d))
        };
    }
    macro_rules! block {
        ($b:expr) => {
            for s in $b.iter() {
                work.push((Node::S(s), d));
            }
        };
    }
    match stmt {
        Stmt::Let {
            ty,
            value,
            var: _,
            mutable: _,
        } => {
            if let Some(t) = ty {
                work.push((Node::T(t), d));
            }
            e!(value);
        }
        Stmt::Set { value, target: _ } => e!(value),
        // `Swap a and b` has only two symbol operands — no child expressions to bound.
        Stmt::Swap { a: _, b: _ } => {}
        Stmt::Call { args, function: _ } => {
            for a in args {
                e!(a);
            }
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            e!(cond);
            block!(then_block);
            if let Some(b) = else_block {
                block!(b);
            }
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            e!(cond);
            if let Some(x) = decreasing {
                e!(x);
            }
            block!(body);
        }
        Stmt::Repeat {
            iterable,
            body,
            pattern: _,
        } => {
            e!(iterable);
            block!(body);
        }
        Stmt::Return { value } => {
            if let Some(x) = value {
                e!(x);
            }
        }
        Stmt::Break => {}
        Stmt::Assert { proposition } => work.push((Node::L(proposition), d)),
        Stmt::Trust {
            proposition,
            justification: _,
        } => work.push((Node::L(proposition), d)),
        Stmt::RuntimeAssert {
            condition,
            hard: _,
            dim: _,
            reason,
        } => {
            e!(condition);
            if let Some(r) = reason {
                e!(r);
            }
        }
        Stmt::Fail { reason } => e!(reason),
        Stmt::Give { object, recipient } => {
            e!(object);
            e!(recipient);
        }
        Stmt::Show { object, recipient } => {
            e!(object);
            e!(recipient);
        }
        Stmt::SetField {
            object,
            value,
            field: _,
        } => {
            e!(object);
            e!(value);
        }
        Stmt::StructDef { .. } => {}
        Stmt::TestDef { cases, body, .. } => {
            if let Some(table) = cases {
                for row in &table.rows {
                    for cell in row {
                        e!(cell);
                    }
                }
            }
            block!(body);
        }
        Stmt::Expect { condition } => e!(condition),
        Stmt::ExpectFail { body, message: _ } => block!(body),
        Stmt::ExpectOutput { .. } => {}
        Stmt::AdvanceTime { millis } => e!(millis),
        Stmt::GivenFile { .. } => {}
        Stmt::Observe { payloads, .. } => {
            for (_, expr) in payloads {
                e!(expr);
            }
        }
        Stmt::During { body, .. } => block!(body),
        Stmt::ForAll {
            constraint, body, ..
        } => {
            if let Some(c) = constraint {
                e!(c);
            }
            block!(body);
        }
        Stmt::FunctionDef {
            params,
            body,
            return_type,
            ..
        } => {
            for (_, t) in params {
                work.push((Node::T(t), d));
            }
            if let Some(t) = return_type {
                work.push((Node::T(t), d));
            }
            block!(body);
        }
        Stmt::Inspect {
            target,
            arms,
            has_otherwise: _,
        } => {
            e!(target);
            for arm in arms {
                block!(arm.body);
            }
        }
        Stmt::Push { value, collection } => {
            e!(value);
            e!(collection);
        }
        Stmt::Pop {
            collection,
            into: _,
        } => e!(collection),
        Stmt::Add { value, collection } => {
            e!(value);
            e!(collection);
        }
        Stmt::Remove { value, collection } => {
            e!(value);
            e!(collection);
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            e!(collection);
            e!(index);
            e!(value);
        }
        Stmt::Splice { body } => block!(body),
        Stmt::ComponentDef(def) => {
            for (_, t) in &def.props {
                work.push((Node::T(t), d));
            }
            if let crate::ast::component::ComponentKind::Page { params, .. } = &def.kind {
                for (_, t) in params {
                    work.push((Node::T(t), d));
                }
            }
            for f in &def.state {
                if let Some(t) = f.ty {
                    work.push((Node::T(t), d));
                }
                e!(f.init);
            }
            for ev in &def.events {
                block!(ev.body);
            }
            for a in &def.actions {
                for (_, t) in &a.params {
                    work.push((Node::T(t), d));
                }
                block!(a.body);
            }
            if let Some(v) = &def.view {
                let (es, bs) = v.exprs_and_blocks();
                for x in es {
                    e!(x);
                }
                for b in bs {
                    block!(b);
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
                work.push((Node::T(t), d));
            }
            block!(body);
            if let Some(r) = removed {
                block!(r);
            }
        }
        Stmt::RouteDef {
            params,
            return_type,
            body,
            ..
        } => {
            for (_, t) in params {
                work.push((Node::T(t), d));
            }
            if let Some(t) = return_type {
                work.push((Node::T(t), d));
            }
            match &**body {
                crate::ast::component::RouteBody::Statements(b) => block!(b),
                crate::ast::component::RouteBody::View(v) => {
                    let (es, bs) = v.exprs_and_blocks();
                    for x in es {
                        e!(x);
                    }
                    for b in bs {
                        block!(b);
                    }
                }
            }
        }
        Stmt::MiddlewareDef { param, body } => {
            work.push((Node::T(param.1), d));
            block!(body);
        }
        Stmt::RedirectDef { .. } => {}
        Stmt::Ui(u) => {
            for x in u.exprs() {
                e!(x);
            }
        }
        Stmt::ServeFetches | Stmt::Replay { .. } => {}
        Stmt::GivenFetch {
            url, status, body, ..
        } => {
            e!(url);
            e!(status);
            e!(body);
        }
        Stmt::Fetch {
            url, body, as_type, ..
        } => {
            e!(url);
            if let Some(b) = body {
                e!(b);
            }
            work.push((Node::T(as_type), d));
        }
        Stmt::Reversibly { body } => block!(body),
        Stmt::Zone {
            body,
            name: _,
            capacity: _,
            source_file: _,
        } => block!(body),
        Stmt::Concurrent { tasks } => block!(tasks),
        Stmt::Parallel { tasks } => block!(tasks),
        Stmt::ReadFrom { source, var: _ } => match source {
            ReadSource::Console => {}
            ReadSource::File(path) => e!(path),
        },
        Stmt::WriteFile { content, path } => {
            e!(content);
            e!(path);
        }
        Stmt::Spawn { .. } => {}
        Stmt::SendMessage {
            message,
            destination,
            ..
        } => {
            e!(message);
            e!(destination);
        }
        Stmt::AwaitMessage { source, .. } => e!(source),
        Stmt::StreamMessage {
            values,
            destination,
        } => {
            e!(values);
            e!(destination);
        }
        Stmt::MergeCrdt { source, target } | Stmt::JoinCrdt { source, target } => {
            e!(source);
            e!(target);
        }
        Stmt::IncreaseCrdt {
            object,
            amount,
            field: _,
        } => {
            e!(object);
            e!(amount);
        }
        Stmt::DecreaseCrdt {
            object,
            amount,
            field: _,
        } => {
            e!(object);
            e!(amount);
        }
        Stmt::AppendToSequence { sequence, value } => {
            e!(sequence);
            e!(value);
        }
        Stmt::ResolveConflict {
            object,
            value,
            field: _,
        } => {
            e!(object);
            e!(value);
        }
        Stmt::Check { .. } => {}
        Stmt::Listen { address, secure } => {
            e!(address);
            if let Some(pad) = secure {
                e!(pad.pad);
            }
        }
        Stmt::Serve { port, handler } => {
            e!(port);
            e!(handler);
        }
        Stmt::ConnectTo { address, secure } => {
            e!(address);
            if let Some(pad) = secure {
                e!(pad.pad);
            }
        }
        Stmt::LetPeerAgent { address, var: _ } => e!(address),
        Stmt::Sleep { milliseconds } => e!(milliseconds),
        Stmt::Sync { topic, var: _ } => e!(topic),
        Stmt::Mount { path, var: _ } => e!(path),
        Stmt::LaunchTask { args, function: _ } => {
            for a in args {
                e!(a);
            }
        }
        Stmt::LaunchTaskWithHandle {
            args,
            handle: _,
            function: _,
        } => {
            for a in args {
                e!(a);
            }
        }
        Stmt::CreatePipe { .. } => {}
        Stmt::SendPipe { value, pipe } => {
            e!(value);
            e!(pipe);
        }
        Stmt::ReceivePipe { pipe, var: _ } => e!(pipe),
        Stmt::TrySendPipe {
            value,
            pipe,
            result: _,
        } => {
            e!(value);
            e!(pipe);
        }
        Stmt::TryReceivePipe { pipe, var: _ } => e!(pipe),
        Stmt::StopTask { handle } => e!(handle),
        Stmt::CloseChannel { channel } => e!(channel),
        Stmt::AwaitTask { handle, result: _ } => e!(handle),
        Stmt::Select { branches } => {
            for branch in branches {
                match branch {
                    SelectBranch::Receive { pipe, body, var: _ } => {
                        e!(pipe);
                        block!(body);
                    }
                    SelectBranch::Timeout { milliseconds, body } => {
                        e!(milliseconds);
                        block!(body);
                    }
                }
            }
        }
        // Formal blocks carry owned proof ASTs consumed by the proof layer,
        // whose own conversion is bounded separately; they count one level.
        Stmt::Theorem(_) | Stmt::Definition(_) | Stmt::Axiom(_) | Stmt::Theory(_) => {}
        Stmt::Escape { .. } => {}
        Stmt::Require { .. } => {}
        // An opaque failure-site marker: an id, no expression or nested block to bound.
        Stmt::Site(_) => {}
    }
}

fn push_expr_children<'w, 'a>(expr: &'w Expr<'a>, d: usize, work: &mut Vec<(Node<'w, 'a>, usize)>) {
    macro_rules! e {
        ($x:expr) => {
            work.push((Node::E($x), d))
        };
    }
    match expr {
        Expr::Literal(_) | Expr::Identifier(_) | Expr::OptionNone | Expr::Escape { .. } => {}
        Expr::Dom(q) => {
            if let Some(sel) = q.selector() {
                e!(sel);
            }
        }
        Expr::View(v) => {
            let (es, bs) = v.exprs_and_blocks();
            for x in es {
                e!(x);
            }
            for b in bs {
                for st in b.iter() {
                    work.push((Node::S(st), d));
                }
            }
        }
        Expr::BinaryOp { left, right, op: _ } => {
            e!(left);
            e!(right);
        }
        Expr::Not { operand } => e!(operand),
        Expr::Call { args, function: _ } => {
            for a in args {
                e!(a);
            }
        }
        Expr::Index { collection, index } => {
            e!(collection);
            e!(index);
        }
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            e!(collection);
            e!(start);
            e!(end);
        }
        Expr::Copy { expr } => e!(expr),
        Expr::Give { value } => e!(value),
        Expr::Length { collection } => e!(collection),
        Expr::Contains { collection, value } => {
            e!(collection);
            e!(value);
        }
        Expr::Union { left, right } => {
            e!(left);
            e!(right);
        }
        Expr::Intersection { left, right } => {
            e!(left);
            e!(right);
        }
        Expr::ManifestOf { zone } => e!(zone),
        Expr::ChunkAt { index, zone } => {
            e!(index);
            e!(zone);
        }
        Expr::List(items) => {
            for x in items {
                e!(x);
            }
        }
        Expr::Tuple(items) => {
            for x in items {
                e!(x);
            }
        }
        Expr::Range { start, end } => {
            e!(start);
            e!(end);
        }
        Expr::FieldAccess { object, field: _ } => e!(object),
        Expr::New {
            type_args,
            init_fields,
            type_name: _,
        } => {
            for t in type_args {
                work.push((Node::T(t), d));
            }
            for (_, x) in init_fields {
                e!(x);
            }
        }
        Expr::NewVariant {
            fields,
            enum_name: _,
            variant: _,
        } => {
            for (_, x) in fields {
                e!(x);
            }
        }
        Expr::OptionSome { value } => e!(value),
        Expr::WithCapacity { value, capacity } => {
            e!(value);
            e!(capacity);
        }
        Expr::Closure {
            params,
            body,
            return_type,
        } => {
            for (_, t) in params {
                work.push((Node::T(t), d));
            }
            if let Some(t) = return_type {
                work.push((Node::T(t), d));
            }
            match body {
                ClosureBody::Expression(x) => e!(x),
                ClosureBody::Block(b) => {
                    for s in *b {
                        work.push((Node::S(s), d));
                    }
                }
            }
        }
        Expr::CallExpr { callee, args } => {
            e!(callee);
            for a in args {
                e!(a);
            }
        }
        Expr::InterpolatedString(parts) => {
            for part in parts {
                match part {
                    StringPart::Literal(_) => {}
                    StringPart::Expr {
                        value,
                        format_spec: _,
                        debug: _,
                    } => e!(value),
                }
            }
        }
    }
}

fn push_type_children<'w, 'a>(
    ty: &'w TypeExpr<'a>,
    d: usize,
    work: &mut Vec<(Node<'w, 'a>, usize)>,
) {
    match ty {
        TypeExpr::Primitive(_) | TypeExpr::Named(_) => {}
        TypeExpr::Generic { params, base: _ } => {
            for t in *params {
                work.push((Node::T(t), d));
            }
        }
        TypeExpr::Tuple(params) => {
            for t in *params {
                work.push((Node::T(t), d));
            }
        }
        TypeExpr::Record { field_types, .. } => {
            for t in *field_types {
                work.push((Node::T(t), d));
            }
        }
        TypeExpr::Function { inputs, output } => {
            for t in *inputs {
                work.push((Node::T(t), d));
            }
            work.push((Node::T(output), d));
        }
        TypeExpr::Refinement {
            base,
            predicate,
            var: _,
        } => {
            work.push((Node::T(base), d));
            work.push((Node::L(predicate), d));
        }
        TypeExpr::Persistent { inner } | TypeExpr::Mutable { inner } => {
            work.push((Node::T(inner), d));
        }
    }
}

fn push_logic_children<'w, 'a>(
    logic: &'w LogicExpr<'a>,
    d: usize,
    work: &mut Vec<(Node<'w, 'a>, usize)>,
) {
    macro_rules! l {
        ($x:expr) => {
            work.push((Node::L($x), d))
        };
    }
    match logic {
        // Terms and noun-phrase payloads nest by English sentence structure,
        // not program size — leaves for depth purposes.
        LogicExpr::Predicate { .. }
        | LogicExpr::Identity { .. }
        | LogicExpr::Metaphor { .. }
        | LogicExpr::Atom(_)
        | LogicExpr::Categorical(_)
        | LogicExpr::Relation(_)
        | LogicExpr::NeoEvent(_)
        | LogicExpr::Comparative { .. }
        | LogicExpr::Superlative { .. } => {}
        LogicExpr::Quantifier { body, .. } => l!(body),
        LogicExpr::Modal { operand, .. } => l!(operand),
        LogicExpr::Temporal { body, .. } => l!(body),
        LogicExpr::TemporalBinary { left, right, .. } => {
            l!(left);
            l!(right);
        }
        LogicExpr::Aspectual { body, .. } => l!(body),
        LogicExpr::Voice { body, .. } => l!(body),
        LogicExpr::BinaryOp { left, right, .. } => {
            l!(left);
            l!(right);
        }
        LogicExpr::UnaryOp { operand, .. } => l!(operand),
        LogicExpr::Question { body, .. } => l!(body),
        LogicExpr::YesNoQuestion { body } => l!(body),
        LogicExpr::Lambda { body, .. } => l!(body),
        LogicExpr::App { function, argument } => {
            l!(function);
            l!(argument);
        }
        LogicExpr::Intensional { content, .. } => l!(content),
        LogicExpr::Event { predicate, .. } => l!(predicate),
        LogicExpr::Imperative { action } => l!(action),
        LogicExpr::Exclamative { body, .. } => l!(body),
        LogicExpr::Optative { wish } => l!(wish),
        LogicExpr::Implicature {
            assertion,
            implicature,
        } => {
            l!(assertion);
            l!(implicature);
        }
        LogicExpr::SpeechAct { content, .. } => l!(content),
        LogicExpr::Counterfactual {
            antecedent,
            consequent,
        } => {
            l!(antecedent);
            l!(consequent);
        }
        LogicExpr::Causal { effect, cause } => {
            l!(effect);
            l!(cause);
        }
        LogicExpr::Concessive { main, concession } => {
            l!(main);
            l!(concession);
        }
        LogicExpr::Scopal { body, .. } => l!(body),
        LogicExpr::Control { infinitive, .. } => l!(infinitive),
        LogicExpr::Presupposition {
            assertion,
            presupposition,
        } => {
            l!(assertion);
            l!(presupposition);
        }
        LogicExpr::Focus { scope, .. } => l!(scope),
        LogicExpr::TemporalAnchor { body, .. } => l!(body),
        LogicExpr::Distributive { predicate } => l!(predicate),
        LogicExpr::GroupQuantifier {
            restriction, body, ..
        } => {
            l!(restriction);
            l!(body);
        }
    }
}
