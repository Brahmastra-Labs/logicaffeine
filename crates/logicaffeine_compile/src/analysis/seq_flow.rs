//! The flow of `Seq of Int` handles through a program, shared by every backend that holds such a list's elements in
//! two forms (machine words, and exact integers once one is past the word).
//!
//! A list's handle travels — a binding, a `copy of`, a `Swap`, an argument, a return — and every slot it travels
//! between must read its elements the same way, so a backend decides per FLOW CLASS of slots. This module finds the
//! classes: which slots share a list's elements, which reach a reader or a source that knows only machine words (a
//! builtin, a pattern, a task, a closure, another body's binding), and where the list literals go. What makes a class
//! hold exact integers, and how, is each backend's: C promotes a `Seq_i64` in place (`codegen_c::exact`), the
//! generated Rust an `IntVec`.

use std::collections::{HashMap, HashSet};

use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};

use crate::ast::stmt::{Expr, Stmt};
use crate::intern::{Interner, Symbol};

/// A place a `Seq of Int`'s cells live in and travel between: a body's local (`None` is Main; a function's parameter
/// is one of its locals), a function's return, a record field by its name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum SeqSlot {
    Local(Option<Symbol>, Symbol),
    Return(Symbol),
    Field(Symbol),
}

/// The flow of `Seq of Int` handles through a program: which slots share cells, which reach a reader or a source
/// that knows nothing of tagged cells, and where the list literals go. It does not depend on which integers are
/// exact, so it is read once, before the fixpoint.
#[derive(Default)]
pub(crate) struct SeqFlow<'s, 'a> {
    parent: HashMap<SeqSlot, SeqSlot>,
    pub(crate) raw: HashSet<SeqSlot>,
    /// Each list literal stored into a slot, with the body it is written in.
    pub(crate) stored_literals: Vec<(&'s Expr<'a>, SeqSlot, Option<Symbol>)>,
    /// Each list literal read in place (`item k of [a, b]`), with the body it is written in.
    pub(crate) read_literals: Vec<(&'s Expr<'a>, Option<Symbol>)>,
    /// The slot expressions, by node address, in a position that decodes cells or carries them to another slot.
    placed: HashSet<usize>,
}

impl SeqFlow<'_, '_> {
    pub(crate) fn find(&mut self, slot: SeqSlot) -> SeqSlot {
        let up = *self.parent.entry(slot).or_insert(slot);
        if up == slot {
            return slot;
        }
        let root = self.find(up);
        self.parent.insert(slot, root);
        root
    }

    fn union(&mut self, a: SeqSlot, b: SeqSlot) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.parent.insert(a, b);
        }
    }

    fn mark_raw(&mut self, slot: SeqSlot) {
        self.find(slot);
        self.raw.insert(slot);
    }

    /// Every slot the flow has seen.
    pub(crate) fn slots(&self) -> Vec<SeqSlot> {
        self.parent.keys().copied().collect()
    }
}

pub(crate) fn address(e: &Expr) -> usize {
    e as *const Expr as usize
}

/// The slot whose cells the value of `e` is, in the body `owner` names: a local, a user function's return, a field,
/// or what a `copy of` or a give carries unchanged.
fn slot_of(e: &Expr, owner: Option<Symbol>, fns: &HashMap<Symbol, Vec<Symbol>>) -> Option<SeqSlot> {
    match e {
        Expr::Identifier(t) => Some(SeqSlot::Local(owner, *t)),
        Expr::Call { function, .. } if fns.contains_key(function) => Some(SeqSlot::Return(*function)),
        Expr::FieldAccess { field, .. } => Some(SeqSlot::Field(*field)),
        Expr::Copy { expr } => slot_of(expr, owner, fns),
        Expr::Give { value } => slot_of(value, owner, fns),
        _ => None,
    }
}

/// The flow of one body, `owner`'s with `params` its parameters, into `flow`; `fns` names every user function with
/// its parameters, in order.
pub(crate) fn record_seq_flow<'s, 'a>(
    body: &'s [Stmt<'a>],
    owner: Option<Symbol>,
    params: &[Symbol],
    fns: &HashMap<Symbol, Vec<Symbol>>,
    interner: &Interner,
    flow: &mut SeqFlow<'s, 'a>,
) {
    let local = |name: Symbol| SeqSlot::Local(owner, name);
    // A value moving into `dst`: the cells of another slot, a literal's, a new empty sequence's, or anything else's,
    // which no tag reaches.
    let into = |flow: &mut SeqFlow<'s, 'a>, dst: SeqSlot, value: &'s Expr<'a>| {
        if let Some(src) = slot_of(value, owner, fns) {
            flow.union(dst, src);
            flow.placed.insert(address(value));
        } else if let Expr::List(_) = value {
            flow.stored_literals.push((value, dst, owner));
        } else if !matches!(value, Expr::New { init_fields, .. } if init_fields.is_empty()) {
            flow.mark_raw(dst);
        }
    };
    // A collection read where its cells are decoded: a slot's handle, or a literal read in place.
    let decoded = |flow: &mut SeqFlow<'s, 'a>, collection: &'s Expr<'a>| {
        if slot_of(collection, owner, fns).is_some() {
            flow.placed.insert(address(collection));
        } else if let Expr::List(_) = collection {
            flow.read_literals.push((collection, owner));
        }
    };
    let call = |flow: &mut SeqFlow<'s, 'a>, function: Symbol, args: &[&'s Expr<'a>]| {
        if let Some(f) = fns.get(&function) {
            for (arg, p) in args.iter().zip(f) {
                into(flow, SeqSlot::Local(Some(function), *p), arg);
            }
        }
    };
    // A function whose parameters something other than a direct call fills, and whose result goes somewhere no
    // call site names: a task, a function used as a value.
    let detached = |flow: &mut SeqFlow<'s, 'a>, function: Symbol| {
        if let Some(f) = fns.get(&function) {
            for p in f {
                flow.mark_raw(SeqSlot::Local(Some(function), *p));
            }
            flow.mark_raw(SeqSlot::Return(function));
        }
    };

    let mut bound: HashSet<Symbol> = params.iter().copied().collect();
    for_each_bound_name(body, |target, is_value| {
        if is_value {
            bound.insert(target);
        }
    });

    walk_nodes(body, |node| {
        match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) => return Walk::Past,
            Visited::Expr(closure @ Expr::Closure { .. }) => {
                mark_all_raw(closure, owner, fns, flow);
                return Walk::Past;
            }
            Visited::Stmt(Stmt::Let { var, value, .. }) => into(flow, local(*var), value),
            Visited::Stmt(Stmt::Set { target, value }) => into(flow, local(*target), value),
            Visited::Stmt(Stmt::Swap { a, b }) => flow.union(local(*a), local(*b)),
            Visited::Stmt(Stmt::Return { value: Some(v) }) => {
                if let Some(f) = owner {
                    into(flow, SeqSlot::Return(f), v);
                }
            }
            Visited::Stmt(Stmt::Push { collection, .. } | Stmt::SetIndex { collection, .. }) => {
                if slot_of(collection, owner, fns).is_some() {
                    flow.placed.insert(address(collection));
                }
            }
            Visited::Stmt(Stmt::Show { object, recipient }) => match recipient {
                Expr::Identifier(f) if interner.resolve(*f) == "show" => decoded(flow, object),
                Expr::Identifier(f) => call(flow, *f, &[*object]),
                _ => {}
            },
            Visited::Stmt(Stmt::Repeat { pattern, iterable, .. }) => {
                decoded(flow, iterable);
                match pattern {
                    crate::ast::stmt::Pattern::Identifier(x) => flow.mark_raw(local(*x)),
                    crate::ast::stmt::Pattern::Tuple(xs) => {
                        for x in xs {
                            flow.mark_raw(local(*x));
                        }
                    }
                }
            }
            Visited::Stmt(Stmt::Call { function, args }) => call(flow, *function, args),
            Visited::Expr(Expr::Call { function, args }) => call(flow, *function, args),
            Visited::Stmt(Stmt::LaunchTask { function, .. } | Stmt::LaunchTaskWithHandle { function, .. }) => {
                detached(flow, *function)
            }
            Visited::Stmt(Stmt::SetField { field, value, .. }) => into(flow, SeqSlot::Field(*field), value),
            Visited::Expr(Expr::New { init_fields, .. }) => {
                for (field, value) in init_fields {
                    into(flow, SeqSlot::Field(*field), value);
                }
            }
            Visited::Expr(
                Expr::Index { collection, .. } | Expr::Length { collection } | Expr::Contains { collection, .. },
            ) => decoded(flow, collection),
            Visited::Expr(Expr::Copy { expr: inner } | Expr::Give { value: inner }) => {
                if slot_of(inner, owner, fns).is_some() {
                    flow.placed.insert(address(inner));
                }
            }
            Visited::Expr(Expr::Identifier(t)) if fns.contains_key(t) && !bound.contains(t) => detached(flow, *t),
            Visited::Stmt(Stmt::Inspect { arms, .. }) => {
                for arm in arms {
                    for (_, b) in &arm.bindings {
                        flow.mark_raw(local(*b));
                    }
                }
            }
            Visited::Stmt(
                Stmt::Pop { into: Some(x), .. }
                | Stmt::AwaitTask { result: Some(x), .. }
                | Stmt::ReceivePipe { var: x, .. }
                | Stmt::TryReceivePipe { var: x, .. }
                | Stmt::AwaitMessage { into: x, .. }
                | Stmt::ReadFrom { var: x, .. }
                | Stmt::LetPeerAgent { var: x, .. }
                | Stmt::Sync { var: x, .. }
                | Stmt::CreatePipe { var: x, .. }
                | Stmt::Zone { name: x, .. },
            ) => flow.mark_raw(local(*x)),
            Visited::Stmt(Stmt::Select { branches }) => {
                for branch in branches {
                    if let crate::ast::stmt::SelectBranch::Receive { var, .. } = branch {
                        flow.mark_raw(local(*var));
                    }
                }
            }
            _ => {}
        }
        Walk::Into
    });

    // Every slot expression no rule above placed is read by code that knows nothing of tagged cells, and every name
    // this body reads that it does not bind is filled by code that tags nothing: another body's, or Main's.
    walk_nodes(body, |node| match node {
        Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => Walk::Past,
        Visited::Expr(e) => {
            if let Some(slot) = slot_of(e, owner, fns) {
                if !flow.placed.contains(&address(e)) {
                    flow.mark_raw(slot);
                }
            }
            if let Expr::Identifier(t) = e {
                if !bound.contains(t) {
                    flow.mark_raw(local(*t));
                    flow.mark_raw(SeqSlot::Local(None, *t));
                }
            }
            Walk::Into
        }
        _ => Walk::Into,
    });
}

/// Everything a closure reads or binds: a closure is lowered on its own, so no tag crosses into it.
fn mark_all_raw(closure: &Expr, owner: Option<Symbol>, fns: &HashMap<Symbol, Vec<Symbol>>, flow: &mut SeqFlow) {
    let Expr::Closure { body, .. } = closure else { return };
    let mark = |e: &Expr, flow: &mut SeqFlow| {
        if let Some(slot) = slot_of(e, owner, fns) {
            flow.mark_raw(slot);
        }
    };
    match body {
        crate::ast::stmt::ClosureBody::Expression(e) => {
            let mut work = vec![*e];
            while let Some(e) = work.pop() {
                mark(e, flow);
                if matches!(e, Expr::Closure { .. }) {
                    mark_all_raw(e, owner, fns, flow);
                }
                work.extend(logicaffeine_language::ast_depth::expr_children(e));
            }
        }
        crate::ast::stmt::ClosureBody::Block(stmts) => {
            for_each_bound_name(stmts, |target, _| flow.mark_raw(SeqSlot::Local(owner, target)));
            walk_nodes(stmts, |node| {
                if let Visited::Expr(e) = node {
                    mark(e, flow);
                }
                Walk::Into
            });
        }
    }
}

/// Every name `body` binds — by a `Let` or a `Set` (`true`: bound to a value), or in an `Inspect` arm (`false`) — wherever
/// it sits in the body's blocks. A nested function's body and a closure's are scopes of their own: the walk does not
/// enter them.
fn for_each_bound_name(body: &[Stmt], mut f: impl FnMut(Symbol, bool)) {
    walk_nodes(body, |node| match node {
        Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => Walk::Past,
        Visited::Stmt(Stmt::Let { var, .. }) => {
            f(*var, true);
            Walk::Into
        }
        Visited::Stmt(Stmt::Set { target, .. }) => {
            f(*target, true);
            Walk::Into
        }
        Visited::Stmt(Stmt::Inspect { arms, .. }) => {
            for arm in arms {
                for (_, binding) in &arm.bindings {
                    f(*binding, false);
                }
            }
            Walk::Into
        }
        _ => Walk::Into,
    });
}
