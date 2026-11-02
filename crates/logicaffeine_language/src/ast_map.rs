//! An exhaustive, rebuilding traversal of the imperative AST.
//!
//! A pass implements [`AstMap`], overrides the hooks for the nodes it rewrites, and calls the
//! matching `walk_*` function for everything else; the walks rebuild each node from its mapped
//! children into the pass's arenas. Every `Stmt`, `Expr` and `TypeExpr` variant is named in its
//! walk — there is no wildcard arm — so adding a variant to the language stops this file compiling
//! until someone decides how it is traversed, and no pass built on it can silently skip one.
//!
//! A few forms are carried through unchanged, each named: the logic-mode blocks and propositions
//! (`Assert`, `Trust`, `Theorem`, `Definition`, `Axiom`, `Theory`) hold no imperative
//! expressions, and the browser surface (`Ui`, `ComponentDef`, a `RouteDef` body, `View`) has its
//! own lowering (`parser::lgx_lower`), which maps a route's body before it becomes a function; every
//! static backend refuses what is left of that surface before a rewriting pass runs.

use crate::arena::Arena;
use crate::ast::stmt::{
    CaseTable, ClosureBody, DomQuery, Expr, MatchArm, ReadSource, SecurePad, SelectBranch, Stmt,
    StringPart, TypeExpr,
};

/// Where a rewriting pass allocates the nodes it builds.
#[derive(Clone, Copy)]
pub struct Arenas<'a> {
    pub exprs: &'a Arena<Expr<'a>>,
    pub stmts: &'a Arena<Stmt<'a>>,
    pub types: &'a Arena<TypeExpr<'a>>,
}

/// A rewriting pass over the imperative AST. Each hook's default rebuilds the node from its
/// mapped children.
pub trait AstMap<'a> {
    fn arenas(&self) -> Arenas<'a>;

    fn map_expr(&mut self, e: &'a Expr<'a>) -> &'a Expr<'a> {
        walk_expr(self, e)
    }

    fn map_stmt(&mut self, s: &Stmt<'a>) -> Stmt<'a> {
        walk_stmt(self, s)
    }

    fn map_type(&mut self, t: &'a TypeExpr<'a>) -> &'a TypeExpr<'a> {
        walk_type(self, t)
    }

    fn map_arm(&mut self, arm: &MatchArm<'a>) -> MatchArm<'a> {
        walk_arm(self, arm)
    }

    /// A block: by default each statement mapped in turn. A pass that rewrites a block's statement sequence
    /// (replacing or joining adjacent statements) overrides this, and every block in the program reaches it.
    fn map_block(&mut self, block: &'a [Stmt<'a>]) -> &'a [Stmt<'a>] {
        walk_block(self, block)
    }
}

pub fn walk_block<'a, M: AstMap<'a> + ?Sized>(m: &mut M, block: &'a [Stmt<'a>]) -> &'a [Stmt<'a>] {
    let mapped: Vec<Stmt<'a>> = block.iter().map(|s| m.map_stmt(s)).collect();
    m.arenas().stmts.alloc_slice(mapped)
}

fn map_exprs<'a, M: AstMap<'a> + ?Sized>(m: &mut M, exprs: &[&'a Expr<'a>]) -> Vec<&'a Expr<'a>> {
    exprs.iter().map(|e| m.map_expr(e)).collect()
}

fn map_fields<'a, M: AstMap<'a> + ?Sized>(
    m: &mut M,
    fields: &[(crate::intern::Symbol, &'a Expr<'a>)],
) -> Vec<(crate::intern::Symbol, &'a Expr<'a>)> {
    fields.iter().map(|(f, e)| (*f, m.map_expr(e))).collect()
}

fn map_typed_params<'a, M: AstMap<'a> + ?Sized>(
    m: &mut M,
    params: &[(crate::intern::Symbol, &'a TypeExpr<'a>)],
) -> Vec<(crate::intern::Symbol, &'a TypeExpr<'a>)> {
    params.iter().map(|(p, t)| (*p, m.map_type(t))).collect()
}

fn map_type_slice<'a, M: AstMap<'a> + ?Sized>(
    m: &mut M,
    types: &'a [TypeExpr<'a>],
) -> &'a [TypeExpr<'a>] {
    let mapped: Vec<TypeExpr<'a>> = types.iter().map(|t| m.map_type(t).clone()).collect();
    m.arenas().types.alloc_slice(mapped)
}

fn map_secure<'a, M: AstMap<'a> + ?Sized>(
    m: &mut M,
    pad: &Option<SecurePad<'a>>,
) -> Option<SecurePad<'a>> {
    pad.map(|p| SecurePad {
        pad: m.map_expr(p.pad),
        role: p.role,
    })
}

pub fn walk_arm<'a, M: AstMap<'a> + ?Sized>(m: &mut M, arm: &MatchArm<'a>) -> MatchArm<'a> {
    MatchArm {
        enum_name: arm.enum_name,
        variant: arm.variant,
        bindings: arm.bindings.clone(),
        body: m.map_block(arm.body),
    }
}

pub fn walk_type<'a, M: AstMap<'a> + ?Sized>(m: &mut M, t: &'a TypeExpr<'a>) -> &'a TypeExpr<'a> {
    let rebuilt = match t {
        TypeExpr::Primitive(_) | TypeExpr::Named(_) => return t,
        TypeExpr::Generic { base, params } => TypeExpr::Generic {
            base: *base,
            params: map_type_slice(m, params),
        },
        TypeExpr::Tuple(items) => TypeExpr::Tuple(map_type_slice(m, items)),
        TypeExpr::Record {
            field_names,
            field_types,
        } => TypeExpr::Record {
            field_names,
            field_types: map_type_slice(m, field_types),
        },
        TypeExpr::Function { inputs, output } => TypeExpr::Function {
            inputs: map_type_slice(m, inputs),
            output: m.map_type(output),
        },
        TypeExpr::Refinement {
            base,
            var,
            predicate,
        } => TypeExpr::Refinement {
            base: m.map_type(base),
            var: *var,
            predicate,
        },
        TypeExpr::Persistent { inner } => TypeExpr::Persistent {
            inner: m.map_type(inner),
        },
        TypeExpr::Mutable { inner } => TypeExpr::Mutable {
            inner: m.map_type(inner),
        },
    };
    m.arenas().types.alloc(rebuilt)
}

pub fn walk_expr<'a, M: AstMap<'a> + ?Sized>(m: &mut M, e: &'a Expr<'a>) -> &'a Expr<'a> {
    let rebuilt = match e {
        Expr::Literal(_) | Expr::Identifier(_) | Expr::Escape { .. } | Expr::OptionNone => {
            return e
        }
        // Browser surface: refused by every static backend before a rewriting pass runs.
        Expr::View(_) => return e,
        Expr::BinaryOp { op, left, right } => Expr::BinaryOp {
            op: *op,
            left: m.map_expr(left),
            right: m.map_expr(right),
        },
        Expr::Not { operand } => Expr::Not {
            operand: m.map_expr(operand),
        },
        Expr::Call { function, args } => Expr::Call {
            function: *function,
            args: map_exprs(m, args),
        },
        Expr::Index { collection, index } => Expr::Index {
            collection: m.map_expr(collection),
            index: m.map_expr(index),
        },
        Expr::Slice {
            collection,
            start,
            end,
        } => Expr::Slice {
            collection: m.map_expr(collection),
            start: m.map_expr(start),
            end: m.map_expr(end),
        },
        Expr::Copy { expr } => Expr::Copy {
            expr: m.map_expr(expr),
        },
        Expr::Give { value } => Expr::Give {
            value: m.map_expr(value),
        },
        Expr::Length { collection } => Expr::Length {
            collection: m.map_expr(collection),
        },
        Expr::Contains { collection, value } => Expr::Contains {
            collection: m.map_expr(collection),
            value: m.map_expr(value),
        },
        Expr::Union { left, right } => Expr::Union {
            left: m.map_expr(left),
            right: m.map_expr(right),
        },
        Expr::Intersection { left, right } => Expr::Intersection {
            left: m.map_expr(left),
            right: m.map_expr(right),
        },
        Expr::ManifestOf { zone } => Expr::ManifestOf {
            zone: m.map_expr(zone),
        },
        Expr::ChunkAt { index, zone } => Expr::ChunkAt {
            index: m.map_expr(index),
            zone: m.map_expr(zone),
        },
        Expr::List(items) => Expr::List(map_exprs(m, items)),
        Expr::Tuple(items) => Expr::Tuple(map_exprs(m, items)),
        Expr::Range { start, end } => Expr::Range {
            start: m.map_expr(start),
            end: m.map_expr(end),
        },
        Expr::FieldAccess { object, field } => Expr::FieldAccess {
            object: m.map_expr(object),
            field: *field,
        },
        Expr::New {
            type_name,
            type_args,
            init_fields,
        } => Expr::New {
            type_name: *type_name,
            type_args: type_args.iter().map(|t| m.map_type(t).clone()).collect(),
            init_fields: map_fields(m, init_fields),
        },
        Expr::NewVariant {
            enum_name,
            variant,
            fields,
        } => Expr::NewVariant {
            enum_name: *enum_name,
            variant: *variant,
            fields: map_fields(m, fields),
        },
        Expr::OptionSome { value } => Expr::OptionSome {
            value: m.map_expr(value),
        },
        Expr::WithCapacity { value, capacity } => Expr::WithCapacity {
            value: m.map_expr(value),
            capacity: m.map_expr(capacity),
        },
        Expr::Closure {
            params,
            body,
            return_type,
        } => Expr::Closure {
            params: map_typed_params(m, params),
            body: match body {
                ClosureBody::Expression(x) => ClosureBody::Expression(m.map_expr(x)),
                ClosureBody::Block(b) => ClosureBody::Block(m.map_block(b)),
            },
            return_type: return_type.map(|t| m.map_type(t)),
        },
        Expr::CallExpr { callee, args } => Expr::CallExpr {
            callee: m.map_expr(callee),
            args: map_exprs(m, args),
        },
        Expr::InterpolatedString(parts) => Expr::InterpolatedString(
            parts
                .iter()
                .map(|p| match p {
                    StringPart::Literal(s) => StringPart::Literal(*s),
                    StringPart::Expr {
                        value,
                        format_spec,
                        debug,
                    } => StringPart::Expr {
                        value: m.map_expr(value),
                        format_spec: *format_spec,
                        debug: *debug,
                    },
                })
                .collect(),
        ),
        Expr::Dom(q) => Expr::Dom(match q {
            DomQuery::Text { selector } => DomQuery::Text {
                selector: m.map_expr(selector),
            },
            DomQuery::Count { selector } => DomQuery::Count {
                selector: m.map_expr(selector),
            },
            DomQuery::Value { selector } => DomQuery::Value {
                selector: m.map_expr(selector),
            },
            DomQuery::Visible { selector } => DomQuery::Visible {
                selector: m.map_expr(selector),
            },
            DomQuery::Hidden { selector } => DomQuery::Hidden {
                selector: m.map_expr(selector),
            },
            DomQuery::Disabled { selector } => DomQuery::Disabled {
                selector: m.map_expr(selector),
            },
            DomQuery::Checked { selector } => DomQuery::Checked {
                selector: m.map_expr(selector),
            },
            DomQuery::Focused { selector } => DomQuery::Focused {
                selector: m.map_expr(selector),
            },
            DomQuery::Journal => DomQuery::Journal,
            DomQuery::JournalShows { first, second } => DomQuery::JournalShows {
                first: *first,
                second: *second,
            },
            DomQuery::Title => DomQuery::Title,
            DomQuery::Url => DomQuery::Url,
        }),
    };
    m.arenas().exprs.alloc(rebuilt)
}

pub fn walk_stmt<'a, M: AstMap<'a> + ?Sized>(m: &mut M, s: &Stmt<'a>) -> Stmt<'a> {
    match s {
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => Stmt::Let {
            var: *var,
            ty: ty.map(|t| m.map_type(t)),
            value: m.map_expr(value),
            mutable: *mutable,
        },
        Stmt::Set { target, value } => Stmt::Set {
            target: *target,
            value: m.map_expr(value),
        },
        Stmt::Call { function, args } => Stmt::Call {
            function: *function,
            args: map_exprs(m, args),
        },
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => Stmt::If {
            cond: m.map_expr(cond),
            then_block: m.map_block(then_block),
            else_block: else_block.map(|b| m.map_block(b)),
        },
        Stmt::While {
            cond,
            body,
            decreasing,
        } => Stmt::While {
            cond: m.map_expr(cond),
            body: m.map_block(body),
            decreasing: decreasing.map(|d| m.map_expr(d)),
        },
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => Stmt::Repeat {
            pattern: pattern.clone(),
            iterable: m.map_expr(iterable),
            body: m.map_block(body),
        },
        Stmt::Return { value } => Stmt::Return {
            value: value.map(|v| m.map_expr(v)),
        },
        Stmt::RuntimeAssert {
            condition,
            hard,
            dim,
            reason,
        } => Stmt::RuntimeAssert {
            condition: m.map_expr(condition),
            hard: *hard,
            dim: *dim,
            reason: reason.map(|r| m.map_expr(r)),
        },
        Stmt::Fail { reason } => Stmt::Fail {
            reason: m.map_expr(reason),
        },
        Stmt::Give { object, recipient } => Stmt::Give {
            object: m.map_expr(object),
            recipient: m.map_expr(recipient),
        },
        Stmt::Show { object, recipient } => Stmt::Show {
            object: m.map_expr(object),
            recipient: m.map_expr(recipient),
        },
        Stmt::SetField {
            object,
            field,
            value,
        } => Stmt::SetField {
            object: m.map_expr(object),
            field: *field,
            value: m.map_expr(value),
        },
        Stmt::FunctionDef {
            name,
            generics,
            params,
            body,
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
            opt_flags,
        } => Stmt::FunctionDef {
            name: *name,
            generics: generics.clone(),
            params: map_typed_params(m, params),
            body: m.map_block(body),
            return_type: return_type.map(|t| m.map_type(t)),
            is_native: *is_native,
            native_path: *native_path,
            is_exported: *is_exported,
            export_target: *export_target,
            opt_flags: *opt_flags,
        },
        Stmt::TestDef {
            name,
            skip,
            capabilities,
            budget,
            cases,
            body,
        } => Stmt::TestDef {
            name: *name,
            skip: *skip,
            capabilities: *capabilities,
            budget: *budget,
            cases: cases.as_ref().map(|c| CaseTable {
                columns: c.columns.clone(),
                rows: c.rows.iter().map(|row| map_exprs(m, row)).collect(),
            }),
            body: m.map_block(body),
        },
        Stmt::Expect { condition } => Stmt::Expect {
            condition: m.map_expr(condition),
        },
        Stmt::ExpectFail { message, body } => Stmt::ExpectFail {
            message: *message,
            body: m.map_block(body),
        },
        Stmt::AdvanceTime { millis } => Stmt::AdvanceTime {
            millis: m.map_expr(millis),
        },
        Stmt::Observe {
            category,
            message,
            payloads,
            sample,
        } => Stmt::Observe {
            category: *category,
            message: *message,
            payloads: map_fields(m, payloads),
            sample: *sample,
        },
        Stmt::During { label, body } => Stmt::During {
            label: *label,
            body: m.map_block(body),
        },
        Stmt::ForAll {
            vars,
            domain,
            constraint,
            body,
        } => Stmt::ForAll {
            vars: vars.clone(),
            domain: domain.clone(),
            constraint: constraint.map(|c| m.map_expr(c)),
            body: m.map_block(body),
        },
        Stmt::Inspect {
            target,
            arms,
            has_otherwise,
        } => Stmt::Inspect {
            target: m.map_expr(target),
            arms: arms.iter().map(|a| m.map_arm(a)).collect(),
            has_otherwise: *has_otherwise,
        },
        Stmt::Push { value, collection } => Stmt::Push {
            value: m.map_expr(value),
            collection: m.map_expr(collection),
        },
        Stmt::Pop { collection, into } => Stmt::Pop {
            collection: m.map_expr(collection),
            into: *into,
        },
        Stmt::Add { value, collection } => Stmt::Add {
            value: m.map_expr(value),
            collection: m.map_expr(collection),
        },
        Stmt::Remove { value, collection } => Stmt::Remove {
            value: m.map_expr(value),
            collection: m.map_expr(collection),
        },
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => Stmt::SetIndex {
            collection: m.map_expr(collection),
            index: m.map_expr(index),
            value: m.map_expr(value),
        },
        Stmt::Splice { body } => Stmt::Splice {
            body: m.map_block(body),
        },
        Stmt::Reversibly { body } => Stmt::Reversibly {
            body: m.map_block(body),
        },
        Stmt::Zone {
            name,
            capacity,
            source_file,
            body,
        } => Stmt::Zone {
            name: *name,
            capacity: *capacity,
            source_file: source_file.clone(),
            body: m.map_block(body),
        },
        Stmt::Concurrent { tasks } => Stmt::Concurrent {
            tasks: m.map_block(tasks),
        },
        Stmt::Parallel { tasks } => Stmt::Parallel {
            tasks: m.map_block(tasks),
        },
        Stmt::ReadFrom { var, source } => Stmt::ReadFrom {
            var: *var,
            source: match source {
                ReadSource::Console => ReadSource::Console,
                ReadSource::File(path) => ReadSource::File(m.map_expr(path)),
            },
        },
        Stmt::WriteFile { content, path } => Stmt::WriteFile {
            content: m.map_expr(content),
            path: m.map_expr(path),
        },
        Stmt::SendMessage {
            message,
            destination,
            compression,
            cached,
            unchecked,
            layout,
            shared,
            computed,
            indexed,
            deduped,
            unreliable,
        } => Stmt::SendMessage {
            message: m.map_expr(message),
            destination: m.map_expr(destination),
            compression: *compression,
            cached: *cached,
            unchecked: *unchecked,
            layout: *layout,
            shared: *shared,
            computed: *computed,
            indexed: *indexed,
            deduped: *deduped,
            unreliable: *unreliable,
        },
        Stmt::AwaitMessage {
            source,
            into,
            view,
            stream,
        } => Stmt::AwaitMessage {
            source: m.map_expr(source),
            into: *into,
            view: *view,
            stream: *stream,
        },
        Stmt::StreamMessage {
            values,
            destination,
        } => Stmt::StreamMessage {
            values: m.map_expr(values),
            destination: m.map_expr(destination),
        },
        Stmt::MergeCrdt { source, target } => Stmt::MergeCrdt {
            source: m.map_expr(source),
            target: m.map_expr(target),
        },
        Stmt::JoinCrdt { source, target } => Stmt::JoinCrdt {
            source: m.map_expr(source),
            target: m.map_expr(target),
        },
        Stmt::IncreaseCrdt {
            object,
            field,
            amount,
        } => Stmt::IncreaseCrdt {
            object: m.map_expr(object),
            field: *field,
            amount: m.map_expr(amount),
        },
        Stmt::DecreaseCrdt {
            object,
            field,
            amount,
        } => Stmt::DecreaseCrdt {
            object: m.map_expr(object),
            field: *field,
            amount: m.map_expr(amount),
        },
        Stmt::AppendToSequence { sequence, value } => Stmt::AppendToSequence {
            sequence: m.map_expr(sequence),
            value: m.map_expr(value),
        },
        Stmt::ResolveConflict {
            object,
            field,
            value,
        } => Stmt::ResolveConflict {
            object: m.map_expr(object),
            field: *field,
            value: m.map_expr(value),
        },
        Stmt::Listen { address, secure } => Stmt::Listen {
            address: m.map_expr(address),
            secure: map_secure(m, secure),
        },
        Stmt::Serve { port, handler } => Stmt::Serve {
            port: m.map_expr(port),
            handler: m.map_expr(handler),
        },
        Stmt::ConnectTo { address, secure } => Stmt::ConnectTo {
            address: m.map_expr(address),
            secure: map_secure(m, secure),
        },
        Stmt::LetPeerAgent { var, address } => Stmt::LetPeerAgent {
            var: *var,
            address: m.map_expr(address),
        },
        Stmt::Sleep { milliseconds } => Stmt::Sleep {
            milliseconds: m.map_expr(milliseconds),
        },
        Stmt::Sync { var, topic } => Stmt::Sync {
            var: *var,
            topic: m.map_expr(topic),
        },
        Stmt::Fetch {
            method,
            url,
            body,
            as_type,
            then,
        } => Stmt::Fetch {
            method: *method,
            url: m.map_expr(url),
            body: body.map(|b| m.map_expr(b)),
            as_type: m.map_type(as_type),
            then: *then,
        },
        Stmt::GivenFetch {
            method,
            url,
            status,
            body,
        } => Stmt::GivenFetch {
            method: *method,
            url: m.map_expr(url),
            status: m.map_expr(status),
            body: m.map_expr(body),
        },
        Stmt::DirectiveDef {
            name,
            params,
            body,
            removed,
        } => Stmt::DirectiveDef {
            name: *name,
            params: map_typed_params(m, params),
            body: m.map_block(body),
            removed: removed.map(|b| m.map_block(b)),
        },
        Stmt::MiddlewareDef { param, body } => Stmt::MiddlewareDef {
            param: (param.0, m.map_type(param.1)),
            body: m.map_block(body),
        },
        Stmt::Mount { var, path } => Stmt::Mount {
            var: *var,
            path: m.map_expr(path),
        },
        Stmt::LaunchTask { function, args } => Stmt::LaunchTask {
            function: *function,
            args: map_exprs(m, args),
        },
        Stmt::LaunchTaskWithHandle {
            handle,
            function,
            args,
        } => Stmt::LaunchTaskWithHandle {
            handle: *handle,
            function: *function,
            args: map_exprs(m, args),
        },
        Stmt::SendPipe { value, pipe } => Stmt::SendPipe {
            value: m.map_expr(value),
            pipe: m.map_expr(pipe),
        },
        Stmt::ReceivePipe { var, pipe } => Stmt::ReceivePipe {
            var: *var,
            pipe: m.map_expr(pipe),
        },
        Stmt::TrySendPipe {
            value,
            pipe,
            result,
        } => Stmt::TrySendPipe {
            value: m.map_expr(value),
            pipe: m.map_expr(pipe),
            result: *result,
        },
        Stmt::TryReceivePipe { var, pipe } => Stmt::TryReceivePipe {
            var: *var,
            pipe: m.map_expr(pipe),
        },
        Stmt::StopTask { handle } => Stmt::StopTask {
            handle: m.map_expr(handle),
        },
        Stmt::CloseChannel { channel } => Stmt::CloseChannel {
            channel: m.map_expr(channel),
        },
        Stmt::AwaitTask { handle, result } => Stmt::AwaitTask {
            handle: m.map_expr(handle),
            result: *result,
        },
        Stmt::Select { branches } => Stmt::Select {
            branches: branches
                .iter()
                .map(|b| match b {
                    SelectBranch::Receive { var, pipe, body } => SelectBranch::Receive {
                        var: *var,
                        pipe: m.map_expr(pipe),
                        body: m.map_block(body),
                    },
                    SelectBranch::Timeout { milliseconds, body } => SelectBranch::Timeout {
                        milliseconds: m.map_expr(milliseconds),
                        body: m.map_block(body),
                    },
                })
                .collect(),
        },
        // Nothing to traverse: names, literals, and flags only.
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
        | Stmt::Site(_) => s.clone(),
        // Logic mode: propositions and proof blocks hold no imperative expressions.
        Stmt::Assert { .. }
        | Stmt::Trust { .. }
        | Stmt::Theorem(_)
        | Stmt::Definition(_)
        | Stmt::Axiom(_)
        | Stmt::Theory(_) => s.clone(),
        // Browser surface: refused by every static backend before a rewriting pass runs.
        Stmt::Ui(_) | Stmt::ComponentDef(_) | Stmt::RouteDef { .. } => s.clone(),
    }
}
