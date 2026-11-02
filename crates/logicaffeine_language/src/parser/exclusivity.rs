//! The law of exclusivity: a binding lent to a `mutable` parameter is the callee's alone until the
//! call returns.
//!
//! A `mutable` parameter is the caller's own value, handed over for the call: the callee reads and
//! writes it, and its writes are the caller's when the call returns. While the call runs nothing
//! else in it may touch the lent binding — another argument reading it would read a value the
//! callee is in the middle of changing, and a second lend would give one value two writers. So a
//! call whose other arguments read or lend again a binding it lends is refused, once, at the end of
//! the parse, where every tier and every tool meets it.
//!
//! A call that lends nothing may still read a binding one of its arguments' calls lends: that inner
//! call has returned before the outer one begins (`joined(a, popLast(a))`).

use std::collections::HashMap;

use crate::ast::component::{ComponentKind, RouteBody, StoreMode};
use crate::ast::stmt::{
    ClosureBody, DomQuery, Expr, ReadSource, SelectBranch, Stmt, StringPart, TypeExpr,
};
use crate::error::{ParseError, ParseErrorKind};
use crate::intern::Symbol;

use super::{ParseResult, Parser};

/// One place a call is made: an expression call, or a call standing as a statement.
enum Site<'s, 'a> {
    Expr(&'a Expr<'a>),
    Call(Symbol, &'s [&'a Expr<'a>]),
}

/// Every expression and call statement `block` holds, at any depth — inside nested blocks, closure
/// bodies, a component's state, actions, event sentences and views, a route's body. Exhaustive
/// over `Stmt` and `Expr`: a new variant does not compile until it says what it holds.
fn visit_block<'a>(block: &[Stmt<'a>], f: &mut dyn FnMut(Site<'_, 'a>)) {
    for stmt in block {
        visit_stmt(stmt, f);
    }
}

fn visit_exprs<'a>(exprs: &[&'a Expr<'a>], f: &mut dyn FnMut(Site<'_, 'a>)) {
    for e in exprs {
        visit_expr(e, f);
    }
}

fn visit_view<'a>(view: &crate::ast::view::ViewDef<'a>, f: &mut dyn FnMut(Site<'_, 'a>)) {
    let (exprs, blocks) = view.exprs_and_blocks();
    visit_exprs(&exprs, f);
    for b in blocks {
        visit_block(b, f);
    }
}

fn visit_stmt<'a>(stmt: &Stmt<'a>, f: &mut dyn FnMut(Site<'_, 'a>)) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => visit_expr(value, f),
        Stmt::Call { function, args } => {
            f(Site::Call(*function, args));
            visit_exprs(args, f);
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            visit_expr(cond, f);
            visit_block(then_block, f);
            if let Some(b) = else_block {
                visit_block(b, f);
            }
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            visit_expr(cond, f);
            visit_block(body, f);
            if let Some(d) = decreasing {
                visit_expr(d, f);
            }
        }
        Stmt::Repeat { iterable, body, .. } => {
            visit_expr(iterable, f);
            visit_block(body, f);
        }
        Stmt::Return { value } => {
            if let Some(v) = value {
                visit_expr(v, f);
            }
        }
        Stmt::RuntimeAssert {
            condition, reason, ..
        } => {
            visit_expr(condition, f);
            if let Some(r) = reason {
                visit_expr(r, f);
            }
        }
        Stmt::Fail { reason } => visit_expr(reason, f),
        Stmt::Give { object, recipient } | Stmt::Show { object, recipient } => {
            visit_expr(object, f);
            visit_expr(recipient, f);
        }
        Stmt::SetField { object, value, .. } => {
            visit_expr(object, f);
            visit_expr(value, f);
        }
        Stmt::FunctionDef { body, .. } => visit_block(body, f),
        Stmt::TestDef { cases, body, .. } => {
            if let Some(table) = cases {
                for row in &table.rows {
                    visit_exprs(row, f);
                }
            }
            visit_block(body, f);
        }
        Stmt::Expect { condition } => visit_expr(condition, f),
        Stmt::ExpectFail { body, .. } | Stmt::During { body, .. } => visit_block(body, f),
        Stmt::AdvanceTime { millis } => visit_expr(millis, f),
        Stmt::Observe { payloads, .. } => {
            for (_, v) in payloads {
                visit_expr(v, f);
            }
        }
        Stmt::ForAll {
            constraint, body, ..
        } => {
            if let Some(c) = constraint {
                visit_expr(c, f);
            }
            visit_block(body, f);
        }
        Stmt::Inspect { target, arms, .. } => {
            visit_expr(target, f);
            for arm in arms {
                visit_block(arm.body, f);
            }
        }
        Stmt::Push { value, collection }
        | Stmt::Add { value, collection }
        | Stmt::Remove { value, collection } => {
            visit_expr(value, f);
            visit_expr(collection, f);
        }
        Stmt::Pop { collection, .. } => visit_expr(collection, f),
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            visit_expr(collection, f);
            visit_expr(index, f);
            visit_expr(value, f);
        }
        Stmt::Splice { body }
        | Stmt::Reversibly { body }
        | Stmt::Zone { body, .. }
        | Stmt::Concurrent { tasks: body }
        | Stmt::Parallel { tasks: body } => visit_block(body, f),
        Stmt::ReadFrom { source, .. } => {
            if let ReadSource::File(path) = source {
                visit_expr(path, f);
            }
        }
        Stmt::WriteFile { content, path } => {
            visit_expr(content, f);
            visit_expr(path, f);
        }
        Stmt::SendMessage {
            message,
            destination,
            ..
        } => {
            visit_expr(message, f);
            visit_expr(destination, f);
        }
        Stmt::AwaitMessage { source, .. } => visit_expr(source, f),
        Stmt::StreamMessage {
            values,
            destination,
        } => {
            visit_expr(values, f);
            visit_expr(destination, f);
        }
        Stmt::MergeCrdt { source, target } | Stmt::JoinCrdt { source, target } => {
            visit_expr(source, f);
            visit_expr(target, f);
        }
        Stmt::IncreaseCrdt { object, amount, .. } | Stmt::DecreaseCrdt { object, amount, .. } => {
            visit_expr(object, f);
            visit_expr(amount, f);
        }
        Stmt::AppendToSequence { sequence, value } => {
            visit_expr(sequence, f);
            visit_expr(value, f);
        }
        Stmt::ResolveConflict { object, value, .. } => {
            visit_expr(object, f);
            visit_expr(value, f);
        }
        Stmt::Listen { address, secure } | Stmt::ConnectTo { address, secure } => {
            visit_expr(address, f);
            if let Some(pad) = secure {
                visit_expr(pad.pad, f);
            }
        }
        Stmt::Serve { port, handler } => {
            visit_expr(port, f);
            visit_expr(handler, f);
        }
        Stmt::LetPeerAgent { address, .. } => visit_expr(address, f),
        Stmt::Sleep { milliseconds } => visit_expr(milliseconds, f),
        Stmt::Sync { topic, .. } => visit_expr(topic, f),
        Stmt::Fetch { url, body, .. } => {
            visit_expr(url, f);
            if let Some(b) = body {
                visit_expr(b, f);
            }
        }
        Stmt::GivenFetch {
            url, status, body, ..
        } => {
            visit_expr(url, f);
            visit_expr(status, f);
            visit_expr(body, f);
        }
        Stmt::DirectiveDef { body, removed, .. } => {
            visit_block(body, f);
            if let Some(r) = removed {
                visit_block(r, f);
            }
        }
        Stmt::MiddlewareDef { body, .. } => visit_block(body, f),
        Stmt::Mount { path, .. } => visit_expr(path, f),
        // A launched task owns its arguments: a spawn is not a call that lends.
        Stmt::LaunchTask { args, .. } | Stmt::LaunchTaskWithHandle { args, .. } => {
            visit_exprs(args, f)
        }
        Stmt::SendPipe { value, pipe } | Stmt::TrySendPipe { value, pipe, .. } => {
            visit_expr(value, f);
            visit_expr(pipe, f);
        }
        Stmt::ReceivePipe { pipe, .. } | Stmt::TryReceivePipe { pipe, .. } => visit_expr(pipe, f),
        Stmt::StopTask { handle } | Stmt::AwaitTask { handle, .. } => visit_expr(handle, f),
        Stmt::CloseChannel { channel } => visit_expr(channel, f),
        Stmt::Select { branches } => {
            for branch in branches {
                match branch {
                    SelectBranch::Receive { pipe, body, .. } => {
                        visit_expr(pipe, f);
                        visit_block(body, f);
                    }
                    SelectBranch::Timeout { milliseconds, body } => {
                        visit_expr(milliseconds, f);
                        visit_block(body, f);
                    }
                }
            }
        }
        Stmt::Ui(u) => visit_exprs(&u.exprs(), f),
        Stmt::ComponentDef(def) => {
            if let ComponentKind::Store {
                mode: StoreMode::Persisted { key: e } | StoreMode::Shared { topic: e },
            } = &def.kind
            {
                visit_expr(e, f);
            }
            for field in &def.state {
                visit_expr(field.init, f);
            }
            for handler in &def.events {
                visit_block(handler.body, f);
            }
            for action in &def.actions {
                visit_block(action.body, f);
            }
            for view in def.view.iter().chain(def.head.iter()) {
                visit_view(view, f);
            }
        }
        Stmt::RouteDef { body, .. } => match &**body {
            RouteBody::Statements(b) => visit_block(b, f),
            RouteBody::View(view) => visit_view(view, f),
        },
        // Names, literals, flags and logic-mode propositions: no imperative expression inside.
        Stmt::Swap { .. }
        | Stmt::Break
        | Stmt::StructDef { .. }
        | Stmt::ExpectOutput { .. }
        | Stmt::GivenFile { .. }
        | Stmt::Check { .. }
        | Stmt::Spawn { .. }
        | Stmt::Replay { .. }
        | Stmt::ServeFetches
        | Stmt::RedirectDef { .. }
        | Stmt::CreatePipe { .. }
        | Stmt::Escape { .. }
        | Stmt::Require { .. }
        | Stmt::Site(_)
        | Stmt::Assert { .. }
        | Stmt::Trust { .. }
        | Stmt::Theorem(_)
        | Stmt::Definition(_)
        | Stmt::Axiom(_)
        | Stmt::Theory(_) => {}
    }
}

fn visit_expr<'a>(expr: &'a Expr<'a>, f: &mut dyn FnMut(Site<'_, 'a>)) {
    f(Site::Expr(expr));
    match expr {
        Expr::Literal(_) | Expr::Identifier(_) | Expr::Escape { .. } | Expr::OptionNone => {}
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range {
            start: left,
            end: right,
        }
        | Expr::Index {
            collection: left,
            index: right,
        }
        | Expr::Contains {
            collection: left,
            value: right,
        }
        | Expr::WithCapacity {
            value: left,
            capacity: right,
        }
        | Expr::ChunkAt {
            index: left,
            zone: right,
        } => {
            visit_expr(left, f);
            visit_expr(right, f);
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
        | Expr::OptionSome { value: operand } => visit_expr(operand, f),
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            visit_expr(collection, f);
            visit_expr(start, f);
            visit_expr(end, f);
        }
        Expr::Call { args, .. } => visit_exprs(args, f),
        Expr::CallExpr { callee, args } => {
            visit_expr(callee, f);
            visit_exprs(args, f);
        }
        Expr::List(items) | Expr::Tuple(items) => visit_exprs(items, f),
        Expr::New { init_fields, .. } => {
            for (_, v) in init_fields {
                visit_expr(v, f);
            }
        }
        Expr::NewVariant { fields, .. } => {
            for (_, v) in fields {
                visit_expr(v, f);
            }
        }
        Expr::Closure { body, .. } => match body {
            ClosureBody::Expression(e) => visit_expr(e, f),
            ClosureBody::Block(b) => visit_block(b, f),
        },
        Expr::InterpolatedString(parts) => {
            for part in parts {
                if let StringPart::Expr { value, .. } = part {
                    visit_expr(value, f);
                }
            }
        }
        Expr::View(view) => visit_view(view, f),
        Expr::Dom(q) => match q {
            DomQuery::Text { selector }
            | DomQuery::Count { selector }
            | DomQuery::Value { selector }
            | DomQuery::Visible { selector }
            | DomQuery::Hidden { selector }
            | DomQuery::Disabled { selector }
            | DomQuery::Checked { selector }
            | DomQuery::Focused { selector } => visit_expr(selector, f),
            DomQuery::Journal | DomQuery::JournalShows { .. } | DomQuery::Title | DomQuery::Url => {
            }
        },
    }
}

/// The binding a place is rooted at — the binding itself, or the one whose field or element it is.
fn root_of(place: &Expr) -> Option<Symbol> {
    match place {
        Expr::Identifier(sym) => Some(*sym),
        Expr::FieldAccess { object, .. } => root_of(object),
        Expr::Index { collection, .. } => root_of(collection),
        _ => None,
    }
}

/// Each function's `mutable` parameters: position and name.
type Lending = HashMap<Symbol, Vec<(usize, Symbol)>>;

fn lending_of(statements: &[Stmt<'_>]) -> Lending {
    let mut lending = Lending::new();
    for stmt in statements {
        if let Stmt::FunctionDef { name, params, .. } = stmt {
            let lent: Vec<(usize, Symbol)> = params
                .iter()
                .enumerate()
                .filter(|(_, (_, ty))| matches!(ty, TypeExpr::Mutable { .. }))
                .map(|(i, (p, _))| (i, *p))
                .collect();
            if !lent.is_empty() {
                lending.insert(*name, lent);
            }
        }
    }
    lending
}

/// How `arg` uses `binding`: `Some(Some(g))` when a call inside it lends the binding to `g`,
/// `Some(None)` when it otherwise mentions it, `None` when it leaves it alone.
fn use_of<'a>(arg: &'a Expr<'a>, binding: Symbol, lending: &Lending) -> Option<Option<Symbol>> {
    let mut used = None;
    visit_expr(arg, &mut |site| {
        let (function, args) = match site {
            Site::Expr(Expr::Identifier(s)) if *s == binding => {
                used.get_or_insert(None);
                return;
            }
            Site::Expr(Expr::Call { function, args }) => (*function, &args[..]),
            _ => return,
        };
        let lends = lending.get(&function).is_some_and(|lent| {
            lent.iter()
                .any(|(at, _)| args.get(*at).and_then(|a| root_of(a)) == Some(binding))
        });
        if lends && !matches!(used, Some(Some(_))) {
            used = Some(Some(function));
        }
    });
    used
}

impl<'a, 'ctx, 'int> Parser<'a, 'ctx, 'int> {
    /// Refuse the first call that lends a binding to a `mutable` parameter and touches it again
    /// in another of its arguments — at the call itself ([`Parser::call_spans`]).
    pub(super) fn check_lends_are_exclusive(&self, statements: &[Stmt<'a>]) -> ParseResult<()> {
        let lending = lending_of(statements);
        if lending.is_empty() {
            return Ok(());
        }
        let mut refusal: Option<ParseError> = None;
        visit_block(statements, &mut |site| {
            if refusal.is_some() {
                return;
            }
            let (function, args) = match site {
                Site::Call(function, args) => (function, args),
                Site::Expr(Expr::Call { function, args }) => (*function, &args[..]),
                Site::Expr(_) => return,
            };
            refusal = self.overlapping_lend(function, args, &lending);
        });
        refusal.map_or(Ok(()), Err)
    }

    fn overlapping_lend(
        &self,
        function: Symbol,
        args: &[&'a Expr<'a>],
        lending: &Lending,
    ) -> Option<ParseError> {
        let lent = lending.get(&function)?;
        for (at, param) in lent {
            let Some(binding) = args.get(*at).and_then(|a| root_of(a)) else {
                continue;
            };
            for (other, arg) in args.iter().enumerate() {
                if other == *at {
                    continue;
                }
                let lent_again_here =
                    lent.iter().any(|(a, _)| *a == other) && root_of(arg) == Some(binding);
                let how = if lent_again_here {
                    "lends it again".to_string()
                } else {
                    match use_of(arg, binding, lending) {
                        None => continue,
                        Some(Some(g)) => {
                            format!("lends it again, to `{}`", self.interner.resolve(g))
                        }
                        Some(None) => "reads it".to_string(),
                    }
                };
                let b = self.interner.resolve(binding);
                let span = self
                    .call_spans
                    .get(&(args[0] as *const Expr as usize))
                    .copied();
                debug_assert!(
                    span.is_some(),
                    "a call that lends `{b}` has its span recorded where it was parsed"
                );
                return Some(ParseError {
                    kind: ParseErrorKind::Custom(format!(
                        "`{b}` is lent to `{f}`'s `mutable` parameter `{p}` for the whole call — it is `{f}`'s alone \
                         until the call returns — and argument {n} of the same call {how}. Pass `copy of {b}` there, \
                         or take what you need from `{b}` into a `Let` before the call.",
                        f = self.interner.resolve(function),
                        p = self.interner.resolve(*param),
                        n = other + 1,
                    )),
                    span: span.unwrap_or_default(),
                });
            }
        }
        None
    }
}
