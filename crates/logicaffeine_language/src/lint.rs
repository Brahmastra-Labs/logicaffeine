//! The value-semantics copy lint.
//!
//! Collections and records are VALUES in LOGOS: `Let opens be b's opens.` binds `opens` to a
//! COPY of `b`'s field, not a reference to it. `vdom.lg` did this at four sites —
//!
//! ```text
//! Let opens be b's opens.
//! Pop from opens into at.
//! ```
//!
//! — popping the copy and never touching `b`'s own `opens` again, so the value `b` actually
//! holds never changes. A new author reaches for exactly this shape, because it reads as if it
//! mutates `b` in place.
//!
//! [`CopyLintScope`] fires when a local is bound from a field access (`b's opens`, `a's b's c`)
//! or another local read by name (`Let a be b.` — a local is copied the same way a field is), is
//! then mutated through `Pop`/`Push`/`Add`/`Remove`/`Set item … of …`/`Set …'s field`, and after
//! its LAST such mutation is never read again and never written back to where it came from. It
//! is a WARNING, not an error: using the copy as scratch space and discarding it is legitimate,
//! so a read of any kind (handing it to a function, `Show`ing it, using it as another
//! statement's condition) after the last mutation silences the lint entirely.
//!
//! One scope tracks one function/action/handler body — see
//! [`Parser::parse_statement`](crate::parser::Parser) for where each statement, at every nesting
//! depth (a `Repeat`/`If`/`While` body included — all of them recurse back through the same
//! per-statement dispatch a scope observes through), feeds into whichever scope is currently
//! open. A local's tracked lifetime is until it is next bound to something else (a second `Let`
//! of the same name finalizes the first binding's story right there) or the function ends —
//! deliberately NOT scoped tighter to a nested block, matching how a `Repeat` body's `Let`
//! re-runs and re-copies every iteration regardless of where its own block happens to end.
//!
//! What this deliberately does not chase, so a missed case only ever means a MISSED warning
//! (never a wrong one): a read inside a nested closure body, an interpolated view template, or a
//! page-test `Dom` query. A local's value crossing into one of those is read there in a way this
//! pass has no reason to disbelieve, so it stays conservative rather than silent about the gap.

use crate::ast::stmt::{Expr, Stmt, StringPart};
use crate::token::Span;
use logicaffeine_base::{Interner, Symbol};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindingKind {
    /// A local copied from a field or another local, mutated, and never read or written back
    /// after its last mutation.
    OrphanedCopy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: FindingKind,
    pub severity: Severity,
    pub span: Span,
    pub message: String,
}

/// A place an assignment can write through, as the chain of names that reaches it — `b's opens`
/// is `[b, opens]`, `a's b's c` is `[a, b, c]`, a bare local `b` is `[b]`. Two expressions name
/// the SAME place iff their chains are equal, which is all a write-back check needs: it never
/// has to hold onto the original `Expr` (or its arena lifetime) at all.
#[derive(Clone, PartialEq, Eq)]
struct Place(Vec<Symbol>);

fn place_of(expr: &Expr<'_>) -> Option<Place> {
    fn chain(expr: &Expr<'_>, path: &mut Vec<Symbol>) -> bool {
        match expr {
            Expr::Identifier(sym) => {
                path.push(*sym);
                true
            }
            Expr::FieldAccess { object, field } => {
                if chain(object, path) {
                    path.push(*field);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
    let mut path = Vec::new();
    chain(expr, &mut path).then_some(Place(path))
}

/// Where a tracked local's value came from. Either is a value copy in LOGOS; a fresh literal or
/// constructor is not (there is nothing to write back TO), so `copy_source` never reports one.
#[derive(Clone, PartialEq, Eq)]
enum CopySource {
    /// `Let x be <place>.` where `<place>` is a field-access chain (`b's opens`) — written back
    /// with `Set <place> to x.` (lowered to `Stmt::SetField`).
    Field(Place),
    /// `Let x be y.` where `y` is a bare local/parameter — written back with `Set y to x.`
    /// (`Stmt::Set`).
    Local(Symbol),
}

struct Binding {
    source: CopySource,
    /// A human-readable rendering of `source`, computed once at bind time (`describe_expr`) so
    /// the message never needs the original `Expr` back.
    source_desc: String,
    last_mutation: Option<Span>,
    /// Read, or written back, since `last_mutation` — starts `true` (nothing to settle before
    /// the first mutation) and is cleared by every fresh mutation.
    settled_since_mutation: bool,
}

/// One function/action/handler body's worth of tracking state — see the module doc comment.
#[derive(Default)]
pub struct CopyLintScope {
    bindings: HashMap<Symbol, Binding>,
    findings: Vec<Finding>,
}

impl CopyLintScope {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one just-parsed statement, at whatever nesting depth, into this scope.
    pub fn observe(&mut self, span: Span, stmt: &Stmt<'_>, interner: &Interner) {
        match stmt {
            Stmt::Let { var, value, .. } => {
                self.rebind(*var, interner);
                if let Some(source) = copy_source(value) {
                    let source_desc = describe_source(&source, interner);
                    self.bindings.insert(
                        *var,
                        Binding {
                            source,
                            source_desc,
                            last_mutation: None,
                            settled_since_mutation: true,
                        },
                    );
                } else {
                    mark_reads(value, &mut self.bindings);
                }
            }

            Stmt::Set { target, value } => {
                mark_reads(value, &mut self.bindings);
                for binding in self.bindings.values_mut() {
                    if binding.source == CopySource::Local(*target) {
                        binding.settled_since_mutation = true;
                    }
                }
                self.rebind(*target, interner);
            }

            Stmt::SetField {
                object,
                field,
                value,
            } => {
                mark_reads(value, &mut self.bindings);
                let mut mutated = false;
                if let Expr::Identifier(sym) = object {
                    if let Some(binding) = self.bindings.get_mut(sym) {
                        binding.last_mutation = Some(span);
                        binding.settled_since_mutation = false;
                        mutated = true;
                    }
                }
                if !mutated {
                    if let Some(mut place) = place_of(object) {
                        place.0.push(*field);
                        for binding in self.bindings.values_mut() {
                            if binding.source == CopySource::Field(place.clone()) {
                                binding.settled_since_mutation = true;
                            }
                        }
                    }
                }
            }

            Stmt::Push { value, collection }
            | Stmt::Add { value, collection }
            | Stmt::Remove { value, collection } => {
                mark_reads(value, &mut self.bindings);
                mutate_place(collection, span, &mut self.bindings);
            }
            Stmt::Pop { collection, .. } => {
                mutate_place(collection, span, &mut self.bindings);
            }
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => {
                mark_reads(index, &mut self.bindings);
                mark_reads(value, &mut self.bindings);
                mutate_place(collection, span, &mut self.bindings);
            }

            Stmt::Swap { a, b } => {
                for sym in [a, b] {
                    if let Some(binding) = self.bindings.get_mut(sym) {
                        binding.settled_since_mutation = true;
                    }
                }
            }

            Stmt::If { cond, .. } | Stmt::While { cond, .. } => {
                mark_reads(cond, &mut self.bindings)
            }
            Stmt::Repeat { iterable, .. } => mark_reads(iterable, &mut self.bindings),
            Stmt::Inspect { target, .. } => mark_reads(target, &mut self.bindings),

            Stmt::Return { value: Some(v) } => mark_reads(v, &mut self.bindings),
            Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => {
                mark_reads(object, &mut self.bindings);
                mark_reads(recipient, &mut self.bindings);
            }
            Stmt::Call { args, .. } => {
                for a in args {
                    mark_reads(a, &mut self.bindings);
                }
            }
            Stmt::RuntimeAssert {
                condition, reason, ..
            } => {
                mark_reads(condition, &mut self.bindings);
                if let Some(r) = reason {
                    mark_reads(r, &mut self.bindings);
                }
            }
            Stmt::Expect { condition } => mark_reads(condition, &mut self.bindings),
            Stmt::Fail { reason } => mark_reads(reason, &mut self.bindings),
            Stmt::Observe { payloads, .. } => {
                for (_, v) in payloads {
                    mark_reads(v, &mut self.bindings);
                }
            }

            _ => {}
        }
    }

    /// A `Let`/`Set` naming an already-tracked local again ends that binding's story right
    /// here — whatever comes after belongs to the NEW value, not the copy this local held a
    /// moment ago.
    fn rebind(&mut self, var: Symbol, interner: &Interner) {
        if let Some(old) = self.bindings.remove(&var) {
            self.finalize(var, &old, interner);
        }
    }

    fn finalize(&mut self, var: Symbol, binding: &Binding, interner: &Interner) {
        let Some(span) = binding.last_mutation else {
            return;
        };
        if binding.settled_since_mutation {
            return;
        }
        self.findings.push(Finding {
            kind: FindingKind::OrphanedCopy,
            severity: Severity::Warning,
            span,
            message: orphaned_copy_message(var, &binding.source_desc, interner),
        });
    }

    /// Every binding still open when the function body ends is finalized here — its own last
    /// mutation is the most recent thing that could ever have settled it.
    pub fn finish(mut self, interner: &Interner) -> Vec<Finding> {
        let remaining: Vec<(Symbol, Binding)> = self.bindings.drain().collect();
        for (var, binding) in remaining {
            self.finalize(var, &binding, interner);
        }
        self.findings
    }
}

fn mutate_place(place: &Expr<'_>, span: Span, bindings: &mut HashMap<Symbol, Binding>) {
    if let Expr::Identifier(sym) = place {
        if let Some(binding) = bindings.get_mut(sym) {
            binding.last_mutation = Some(span);
            binding.settled_since_mutation = false;
        }
    }
}

fn copy_source(value: &Expr<'_>) -> Option<CopySource> {
    match value {
        Expr::FieldAccess { .. } => place_of(value).map(CopySource::Field),
        Expr::Identifier(sym) => Some(CopySource::Local(*sym)),
        _ => None,
    }
}

fn describe_source(source: &CopySource, interner: &Interner) -> String {
    match source {
        CopySource::Field(place) => place
            .0
            .iter()
            .map(|s| interner.resolve(*s))
            .collect::<Vec<_>>()
            .join("'s "),
        CopySource::Local(sym) => interner.resolve(*sym).to_string(),
    }
}

fn orphaned_copy_message(var: Symbol, source_desc: &str, interner: &Interner) -> String {
    let var_str = interner.resolve(var);
    format!(
        "'{var_str}' is a copy of '{source_desc}' — collections and records are values in LOGOS, \
        so this mutates the copy, never '{source_desc}' itself. Either write it back (`Set \
        {source_desc} to {var_str}.`), or mutate '{source_desc}' directly."
    )
}

/// Marks every tracked local read anywhere inside `expr` as settled. Recurses through the
/// expression shapes that can hold a local's read (an operand, an argument, a collection
/// element, an interpolation hole); a shape this does not open (a closure body, a view
/// template, a `Dom` query) is a documented, deliberate gap — see the module doc comment.
fn mark_reads(expr: &Expr<'_>, bindings: &mut HashMap<Symbol, Binding>) {
    match expr {
        Expr::Identifier(sym) => {
            if let Some(binding) = bindings.get_mut(sym) {
                binding.settled_since_mutation = true;
            }
        }
        Expr::FieldAccess { object, .. } => mark_reads(object, bindings),
        Expr::BinaryOp { left, right, .. } => {
            mark_reads(left, bindings);
            mark_reads(right, bindings);
        }
        Expr::Not { operand } | Expr::Copy { expr: operand } | Expr::Give { value: operand } => {
            mark_reads(operand, bindings);
        }
        Expr::Call { args, .. } => {
            for a in args {
                mark_reads(a, bindings);
            }
        }
        Expr::CallExpr { callee, args } => {
            mark_reads(callee, bindings);
            for a in args {
                mark_reads(a, bindings);
            }
        }
        Expr::Index { collection, index } => {
            mark_reads(collection, bindings);
            mark_reads(index, bindings);
        }
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            mark_reads(collection, bindings);
            mark_reads(start, bindings);
            mark_reads(end, bindings);
        }
        Expr::Length { collection } | Expr::ManifestOf { zone: collection } => {
            mark_reads(collection, bindings);
        }
        Expr::Contains { collection, value } => {
            mark_reads(collection, bindings);
            mark_reads(value, bindings);
        }
        Expr::Union { left, right } | Expr::Intersection { left, right } => {
            mark_reads(left, bindings);
            mark_reads(right, bindings);
        }
        Expr::ChunkAt { index, zone } => {
            mark_reads(index, bindings);
            mark_reads(zone, bindings);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for i in items {
                mark_reads(i, bindings);
            }
        }
        Expr::Range { start, end } => {
            mark_reads(start, bindings);
            mark_reads(end, bindings);
        }
        Expr::New { init_fields, .. } => {
            for (_, v) in init_fields {
                mark_reads(v, bindings);
            }
        }
        Expr::NewVariant { fields, .. } => {
            for (_, v) in fields {
                mark_reads(v, bindings);
            }
        }
        Expr::OptionSome { value } => mark_reads(value, bindings),
        Expr::WithCapacity { value, capacity } => {
            mark_reads(value, bindings);
            mark_reads(capacity, bindings);
        }
        Expr::InterpolatedString(parts) => {
            for part in parts.iter() {
                if let StringPart::Expr { value, .. } = part {
                    mark_reads(value, bindings);
                }
            }
        }
        _ => {}
    }
}
