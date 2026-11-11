//! Which values of a program the exact extension reaches: the bindings, function results and
//! parameters that can hold an infinitesimal, a hyperreal, an infinite integer, an algebraic number, a
//! computable real, an ordinal, a residue-dependent value or a provisional answer.
//!
//! The answer is a fixpoint over the whole program. A value is reached when it is made by a builtin of
//! the extension (`infinitesimal`, `standardPart`, `tripCount`, `affinePower`, … — every
//! [`logicaffeine_base::extension::Builtin`]) or by `definedReal`, declared with one of
//! the exact types, or computed from a reached value — arithmetic and comparison lift through it, an
//! item, a run of items and a copy of a reached list are reached, and `not` lifts over it. A binding is
//! reached when anything assigned to it is; a function's result when anything it returns is; a
//! parameter when any call passes it a reached value. `and`/`or` decide their operands, so they reach
//! nothing. A list an item of the extension is stored into is reached; a map or a record stores one under
//! a key and keeps its own kind, as every backend that lowers the bytecode reads it
//! (`vm::tower::exact_write`).
//!
//! Every tier that keeps static types reads it: the optimizer leaves alone the integer-only rewrites
//! of a reached value, and a compiled backend holds a reached binding in its exact value model. The
//! answer over-approximates — names are keyed by the function they live in, not by block — which costs
//! a compiled program speed, never meaning: a value model that keeps canonical forms holds a standard
//! integer as one.

use std::collections::{HashMap, HashSet};

use logicaffeine_base::extension::Builtin;

use crate::ast::stmt::{BinaryOpKind, Block, Expr, Pattern, SelectBranch, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

/// The builtins whose result is reached when an argument is: they round, measure or combine a number
/// in its own kind.
pub(crate) const LIFTING_BUILTINS: &[&str] = &["floor", "ceil", "round", "abs", "min", "max", "pow", "sqrt"];

/// The builtins outside the extension that make one of its values: a real number a function the
/// program defines.
pub(crate) const DEFINERS: &[&str] = &["definedReal"];

/// The declared types that name the extension.
pub(crate) const TOWER_TYPES: &[&str] = &["Algebraic", "ExactReal", "Hyperreal", "Hypernatural", "Ordinal"];

/// Where a name lives: a function's body, or the program's root (`None`).
pub type Scope = Option<Symbol>;

/// The reached bindings, results and parameters of a program.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TowerTypes {
    bindings: HashSet<(Scope, Symbol)>,
    returns: HashSet<Symbol>,
    params: HashMap<Symbol, Vec<Symbol>>,
    /// The program's spellings of the extension's builtins and of [`LIFTING_BUILTINS`].
    makers: HashSet<Symbol>,
    lifters: HashSet<Symbol>,
    /// The program calls a builtin of the extension somewhere.
    touched: bool,
    /// The bindings some assignment makes a map or a record, and those some assignment makes anything else:
    /// a binding only ever a map or a record stores an item under a key.
    keyed: HashSet<(Scope, Symbol)>,
    unkeyed: HashSet<(Scope, Symbol)>,
    /// The names each function binds itself (its parameters, its `Let`s, its loop variables, what it pops into):
    /// any other name a function assigns is the program's root binding.
    locals: HashMap<Symbol, HashSet<Symbol>>,
}

impl TowerTypes {
    /// The program's answer, or `None` when the extension reaches nothing.
    pub fn of_program(stmts: &[Stmt], interner: &Interner) -> Option<TowerTypes> {
        let spelled = |names: &mut dyn Iterator<Item = &str>| {
            names.filter_map(|n| interner.lookup(n)).collect()
        };
        let mut t = TowerTypes {
            makers: spelled(&mut Builtin::ALL.iter().map(|b| b.name()).chain(DEFINERS.iter().copied())),
            lifters: spelled(&mut LIFTING_BUILTINS.iter().copied()),
            ..TowerTypes::default()
        };
        for s in stmts {
            if let Stmt::FunctionDef { name, params, body, .. } = s {
                t.params.insert(*name, params.iter().map(|(p, _)| *p).collect());
                let mut locals: HashSet<Symbol> = params.iter().map(|(p, _)| *p).collect();
                bound_names(body, &mut locals);
                t.locals.insert(*name, locals);
            }
        }
        t.seed_declared(None, stmts, interner);
        t.seed_keyed(None, stmts, interner);
        loop {
            let before = (t.bindings.len(), t.returns.len());
            t.pass(None, stmts);
            for s in stmts {
                if let Stmt::FunctionDef { name, body, .. } = s {
                    t.pass(Some(*name), body);
                }
            }
            if (t.bindings.len(), t.returns.len()) == before {
                break;
            }
        }
        (t.touched || !t.bindings.is_empty() || !t.returns.is_empty()).then_some(t)
    }

    /// Whether `sym`, as named in `scope`, can hold a value of the extension. A function reads the
    /// program's root bindings too.
    pub fn binding(&self, scope: Scope, sym: Symbol) -> bool {
        self.bindings.contains(&(scope, sym)) || self.bindings.contains(&(None, sym))
    }

    /// Whether the function `f` can return a value of the extension.
    pub fn returns(&self, f: Symbol) -> bool {
        self.returns.contains(&f)
    }

    /// Whether the `i`th parameter of the program's function `f` can hold a value of the extension.
    pub fn param_at(&self, f: Symbol, i: usize) -> bool {
        self.params
            .get(&f)
            .and_then(|ps| ps.get(i))
            .is_some_and(|p| self.binding(Some(f), *p))
    }

    /// Whether the program declares a function named `f`, which a call of `f` then means over any
    /// builtin of that name.
    pub fn declares(&self, f: Symbol) -> bool {
        self.params.contains_key(&f)
    }

    /// Every name, in any scope, that can hold a value of the extension.
    pub fn names(&self) -> HashSet<Symbol> {
        self.bindings.iter().map(|(_, s)| *s).collect()
    }

    /// Whether `expr`, evaluated in `scope`, can be a value of the extension.
    pub fn expr(&self, scope: Scope, expr: &Expr) -> bool {
        match expr {
            Expr::Identifier(s) => self.binding(scope, *s),
            Expr::BinaryOp { op, left, right } => {
                !matches!(op, BinaryOpKind::And | BinaryOpKind::Or | BinaryOpKind::Concat)
                    && (self.expr(scope, left) || self.expr(scope, right))
            }
            Expr::Not { operand } => self.expr(scope, operand),
            Expr::Call { function, args } => {
                self.makers.contains(function)
                    || self.returns(*function)
                    || (self.lifters.contains(function) && args.iter().any(|a| self.expr(scope, a)))
            }
            Expr::Index { collection, .. } | Expr::Slice { collection, .. } => self.expr(scope, collection),
            Expr::Copy { expr: inner } => self.expr(scope, inner),
            Expr::List(items) => items.iter().any(|i| self.expr(scope, i)),
            _ => false,
        }
    }

    fn seed_declared(&mut self, scope: Scope, stmts: &[Stmt], interner: &Interner) {
        let exact = |t: &TypeExpr| {
            matches!(t, TypeExpr::Primitive(s) | TypeExpr::Named(s) if TOWER_TYPES.contains(&interner.resolve(*s)))
        };
        for s in stmts {
            match s {
                Stmt::Let { var, ty: Some(t), .. } if exact(t) => {
                    self.bindings.insert((scope, *var));
                }
                Stmt::FunctionDef {
                    name,
                    params,
                    body,
                    return_type,
                    ..
                } => {
                    for (p, t) in params {
                        if exact(t) {
                            self.bindings.insert((Some(*name), *p));
                        }
                    }
                    if return_type.is_some_and(|t| exact(t)) {
                        self.returns.insert(*name);
                    }
                    self.seed_declared(Some(*name), body, interner);
                }
                other => {
                    for b in nested_blocks(other) {
                        self.seed_declared(scope, b, interner);
                    }
                }
            }
        }
    }

    /// Which bindings each assignment of `stmts`, in `scope`, makes a map or a record (`a new Map of …`,
    /// `a new Box with …`) and which it makes anything else.
    fn seed_keyed(&mut self, scope: Scope, stmts: &[Stmt], interner: &Interner) {
        let keyed = |value: &Expr| match value {
            Expr::New { type_name, .. } => {
                !matches!(interner.resolve(*type_name), "Seq" | "List" | "Set" | "HashSet")
            }
            _ => false,
        };
        for s in stmts {
            match s {
                Stmt::Let { var, value, .. } | Stmt::Set { target: var, value } => {
                    if keyed(value) {
                        self.keyed.insert((scope, *var));
                    } else {
                        self.unkeyed.insert((scope, *var));
                    }
                }
                Stmt::FunctionDef { name, body, .. } => self.seed_keyed(Some(*name), body, interner),
                _ => {}
            }
            if !matches!(s, Stmt::FunctionDef { .. }) {
                for b in nested_blocks(s) {
                    self.seed_keyed(scope, b, interner);
                }
            }
        }
    }

    /// Whether `sym`, as named in `scope`, is only ever a map or a record.
    fn keyed(&self, scope: Scope, sym: Symbol) -> bool {
        let at = if self.keyed.contains(&(scope, sym)) || self.unkeyed.contains(&(scope, sym)) {
            (scope, sym)
        } else {
            (None, sym)
        };
        self.keyed.contains(&at) && !self.unkeyed.contains(&at)
    }

    /// Reach `sym` as `scope` names it: a function's own binding, or the program's root binding a function assigns
    /// without binding it itself.
    fn reach(&mut self, scope: Scope, sym: Symbol) {
        let own = scope.is_some_and(|f| self.locals.get(&f).is_some_and(|names| names.contains(&sym)));
        let at = if scope.is_none() || own { scope } else { None };
        self.bindings.insert((at, sym));
    }

    fn reach_args(&mut self, scope: Scope, function: Symbol, args: &[&Expr]) {
        self.touched |= self.makers.contains(&function);
        let reached: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| self.expr(scope, a))
            .map(|(i, _)| i)
            .collect();
        if let Some(params) = self.params.get(&function).cloned() {
            for i in reached {
                if let Some(p) = params.get(i) {
                    self.reach(Some(function), *p);
                }
            }
        }
        for a in args {
            self.calls_in(scope, a);
        }
    }

    /// The calls inside an expression, each passing what it passes.
    fn calls_in(&mut self, scope: Scope, expr: &Expr) {
        match expr {
            Expr::Call { function, args } => self.reach_args(scope, *function, args),
            Expr::BinaryOp { left, right, .. } => {
                self.calls_in(scope, left);
                self.calls_in(scope, right);
            }
            Expr::Not { operand } => self.calls_in(scope, operand),
            Expr::Index { collection, index } => {
                self.calls_in(scope, collection);
                self.calls_in(scope, index);
            }
            Expr::List(items) => {
                for i in items {
                    self.calls_in(scope, i);
                }
            }
            _ => {}
        }
    }

    fn pass(&mut self, scope: Scope, stmts: &[Stmt]) {
        for s in stmts {
            match s {
                Stmt::FunctionDef { .. } => {}
                Stmt::Let { var, value, .. } | Stmt::Set { target: var, value } => {
                    if self.expr(scope, value) {
                        self.reach(scope, *var);
                    }
                    self.calls_in(scope, value);
                }
                Stmt::Repeat {
                    pattern, iterable, ..
                } => {
                    if self.expr(scope, iterable) {
                        match pattern {
                            Pattern::Identifier(x) => self.reach(scope, *x),
                            Pattern::Tuple(xs) => {
                                for x in xs {
                                    self.reach(scope, *x);
                                }
                            }
                        }
                    }
                    self.calls_in(scope, iterable);
                }
                Stmt::Push { value, collection } | Stmt::SetIndex {
                    collection, value, ..
                } => {
                    if self.expr(scope, value) {
                        if let Expr::Identifier(c) = collection {
                            let under_a_key = matches!(s, Stmt::SetIndex { .. }) && self.keyed(scope, *c);
                            if !under_a_key {
                                self.reach(scope, *c);
                            }
                        }
                    }
                    self.calls_in(scope, value);
                }
                Stmt::Return { value: Some(v) } => {
                    if let Some(f) = scope {
                        if self.expr(scope, v) {
                            self.returns.insert(f);
                        }
                    }
                    self.calls_in(scope, v);
                }
                Stmt::Call { function, args } => self.reach_args(scope, *function, args),
                Stmt::Pop {
                    collection: Expr::Identifier(c),
                    into: Some(v),
                } => {
                    if self.binding(scope, *c) {
                        self.reach(scope, *v);
                    }
                }
                Stmt::If { cond, .. } | Stmt::While { cond, .. } => self.calls_in(scope, cond),
                Stmt::Show { object, .. } => self.calls_in(scope, object),
                _ => {}
            }
            for b in nested_blocks(s) {
                self.pass(scope, b);
            }
        }
    }
}

/// The names `stmts` bind, nested blocks included: each `Let`, loop variable and `Pop … into`.
pub(crate) fn bound_names(stmts: &[Stmt], into: &mut HashSet<Symbol>) {
    for s in stmts {
        match s {
            Stmt::Let { var, .. } => {
                into.insert(*var);
            }
            Stmt::Repeat { pattern, .. } => match pattern {
                Pattern::Identifier(x) => {
                    into.insert(*x);
                }
                Pattern::Tuple(xs) => into.extend(xs.iter().copied()),
            },
            Stmt::Pop { into: Some(v), .. } => {
                into.insert(*v);
            }
            _ => {}
        }
        if !matches!(s, Stmt::FunctionDef { .. }) {
            for b in nested_blocks(s) {
                bound_names(b, into);
            }
        }
    }
}

/// The statement blocks nested in `stmt` that share its scope; a function's body is its own scope and
/// is not among them. Exhaustive over `Stmt`: a statement kind added to the language does not compile
/// here until its blocks are named.
pub(crate) fn nested_blocks<'s, 'a>(stmt: &'s Stmt<'a>) -> Vec<Block<'a>> {
    match stmt {
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            let mut v = vec![*then_block];
            v.extend(else_block.iter().copied());
            v
        }
        Stmt::While { body, .. }
        | Stmt::Repeat { body, .. }
        | Stmt::TestDef { body, .. }
        | Stmt::ExpectFail { body, .. }
        | Stmt::During { body, .. }
        | Stmt::ForAll { body, .. }
        | Stmt::Splice { body }
        | Stmt::Reversibly { body }
        | Stmt::Zone { body, .. }
        | Stmt::MiddlewareDef { body, .. } => vec![*body],
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => vec![*tasks],
        Stmt::Inspect { arms, .. } => arms.iter().map(|a| a.body).collect(),
        Stmt::Select { branches } => branches
            .iter()
            .map(|b| match b {
                SelectBranch::Receive { body, .. } | SelectBranch::Timeout { body, .. } => *body,
            })
            .collect(),
        Stmt::DirectiveDef { body, removed, .. } => {
            let mut v = vec![*body];
            v.extend(removed.iter().copied());
            v
        }
        Stmt::RouteDef { body, .. } => match &**body {
            crate::ast::component::RouteBody::Statements(b) => vec![*b],
            crate::ast::component::RouteBody::View(_) => Vec::new(),
        },
        Stmt::ComponentDef(def) => def
            .events
            .iter()
            .map(|e| e.body)
            .chain(def.actions.iter().map(|a| a.body))
            .collect(),
        Stmt::FunctionDef { .. }
        | Stmt::Let { .. }
        | Stmt::Set { .. }
        | Stmt::Swap { .. }
        | Stmt::Call { .. }
        | Stmt::Return { .. }
        | Stmt::Break
        | Stmt::Assert { .. }
        | Stmt::Trust { .. }
        | Stmt::RuntimeAssert { .. }
        | Stmt::Fail { .. }
        | Stmt::Give { .. }
        | Stmt::Show { .. }
        | Stmt::SetField { .. }
        | Stmt::StructDef { .. }
        | Stmt::Expect { .. }
        | Stmt::ExpectOutput { .. }
        | Stmt::AdvanceTime { .. }
        | Stmt::GivenFile { .. }
        | Stmt::Observe { .. }
        | Stmt::Push { .. }
        | Stmt::Pop { .. }
        | Stmt::Add { .. }
        | Stmt::Remove { .. }
        | Stmt::SetIndex { .. }
        | Stmt::ReadFrom { .. }
        | Stmt::WriteFile { .. }
        | Stmt::Spawn { .. }
        | Stmt::SendMessage { .. }
        | Stmt::AwaitMessage { .. }
        | Stmt::StreamMessage { .. }
        | Stmt::MergeCrdt { .. }
        | Stmt::JoinCrdt { .. }
        | Stmt::IncreaseCrdt { .. }
        | Stmt::DecreaseCrdt { .. }
        | Stmt::AppendToSequence { .. }
        | Stmt::ResolveConflict { .. }
        | Stmt::Check { .. }
        | Stmt::Listen { .. }
        | Stmt::Serve { .. }
        | Stmt::ConnectTo { .. }
        | Stmt::LetPeerAgent { .. }
        | Stmt::Sleep { .. }
        | Stmt::Sync { .. }
        | Stmt::Ui(_)
        | Stmt::Fetch { .. }
        | Stmt::Replay { .. }
        | Stmt::ServeFetches
        | Stmt::GivenFetch { .. }
        | Stmt::RedirectDef { .. }
        | Stmt::Mount { .. }
        | Stmt::LaunchTask { .. }
        | Stmt::LaunchTaskWithHandle { .. }
        | Stmt::CreatePipe { .. }
        | Stmt::SendPipe { .. }
        | Stmt::ReceivePipe { .. }
        | Stmt::TrySendPipe { .. }
        | Stmt::TryReceivePipe { .. }
        | Stmt::StopTask { .. }
        | Stmt::CloseChannel { .. }
        | Stmt::AwaitTask { .. }
        | Stmt::Theorem(_)
        | Stmt::Definition(_)
        | Stmt::Axiom(_)
        | Stmt::Theory(_)
        | Stmt::Escape { .. }
        | Stmt::Require { .. }
        | Stmt::Site(_) => Vec::new(),
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/analysis/tower/tests.rs"]
mod tests;
