//! Where a stored `nothing` can be.
//!
//! The VM holds any value in any binding, element or field; a compiled tier gives each one a type for its whole life.
//! A program puts `nothing` where it put a value — `Set item 1 of xs to nothing`, or the store the lent-place lowering
//! makes while a call holds what it was lent (`optimize::sequence`) — and from there `nothing` flows wherever a value
//! does: copied into a binding, passed to a parameter, returned, pushed, stored into a field. A tier types every such
//! place as able to hold `nothing` and leaves every other place as it was.
//!
//! The answer is a set of depths for each binding, parameter, return and field: depth 0 the value itself,
//! depth 1 an item of it, depth 2 an item of an item. Value flow relates two values at depths of each: a binding and
//! the value bound to it at depth 0 of each, a list and an item of it at depth 1 of the list and 0 of the item. Each
//! relation is followed both ways, so every value connected by flow has one representation and no copy converts one
//! to another. A depth past [`DEPTH_LIMIT`] is never recorded, which bounds the solution; no LOGOS value nests deeper.
//!
//! A field is one place for every value of its struct. Where the struct an access reads is not known, the access
//! stands for the field of that name in every struct ([`Absence::field`]).

use std::collections::{BTreeSet, HashMap};

use logicaffeine_language::ast_depth::expr_children;

use crate::analysis::types::{LogosType, TypeEnv};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Pattern, Stmt};
use crate::intern::{Interner, Symbol};
use crate::optimize::writes::{walk_stmt, Bind, EffectSink};

/// The depths at which a value can be `nothing`: 0 the value, 1 an item of it, and so on.
pub(crate) type Depths = BTreeSet<usize>;

/// The deepest depth recorded: deeper than any value a program builds.
pub(crate) const DEPTH_LIMIT: usize = 32;

/// Where `nothing` can be stored in one program.
#[derive(Debug, Default, Clone)]
pub(crate) struct Absence {
    bindings: HashMap<usize, Depths>,
    parameters: HashMap<(Symbol, Symbol), Depths>,
    returns: HashMap<Symbol, Depths>,
    fields: HashMap<(Option<Symbol>, Symbol), Depths>,
}

impl Absence {
    /// Whether no place of the program can hold `nothing`.
    pub(crate) fn is_empty(&self) -> bool {
        self.bindings.is_empty() && self.parameters.is_empty() && self.returns.is_empty() && self.fields.is_empty()
    }

    /// The depths at which what the `Let` statement `stmt` binds can be `nothing`.
    pub(crate) fn binding(&self, stmt: &Stmt) -> Option<&Depths> {
        self.bindings.get(&(stmt as *const Stmt as usize))
    }

    /// The depths at which `function`'s parameter `param` can be `nothing`.
    pub(crate) fn parameter(&self, function: Symbol, param: Symbol) -> Option<&Depths> {
        self.parameters.get(&(function, param))
    }

    /// The depths at which what `function` returns can be `nothing`.
    pub(crate) fn returned(&self, function: Symbol) -> Option<&Depths> {
        self.returns.get(&function)
    }

    /// The depths at which field `field` of struct `owner` can be `nothing`: its own, and those of every access to a
    /// field of that name whose struct was not known.
    pub(crate) fn field(&self, owner: Symbol, field: Symbol) -> Option<Depths> {
        let own = self.fields.get(&(Some(owner), field));
        let any = self.fields.get(&(None, field));
        match (own, any) {
            (None, None) => None,
            (a, b) => Some(a.into_iter().chain(b).flatten().copied().collect()),
        }
    }
}

/// Where `nothing` can be stored in the program `stmts` (its `## Main` statements and its function definitions).
pub(crate) fn of_program(stmts: &[Stmt], types: &TypeEnv, interner: &Interner) -> Absence {
    let mut builder = Builder {
        types,
        interner,
        graph: Graph::default(),
        names: HashMap::new(),
        bindings: HashMap::new(),
        expressions: HashMap::new(),
        parameters: HashMap::new(),
        parameter_names: HashMap::new(),
        returns: HashMap::new(),
        fields: HashMap::new(),
        current: None,
    };
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, .. } = stmt {
            for (index, (param, _)) in params.iter().enumerate() {
                let node = builder.graph.node();
                builder.parameters.insert((*name, index), node);
                builder.parameter_names.insert((*name, index), *param);
            }
            let node = builder.graph.node();
            builder.returns.insert(*name, node);
        }
    }
    builder.block(stmts);
    let program = builder.names.clone();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, .. } = stmt {
            builder.names = program.clone();
            builder.current = Some(*name);
            for (index, (param, _)) in params.iter().enumerate() {
                builder.names.insert(*param, builder.parameters[&(*name, index)]);
            }
            builder.block(body);
        }
    }
    builder.finish()
}

type Node = usize;

/// `left` at `left_depth` is `right` at `right_depth`: what is `nothing` at depth `d` of one is `nothing` at depth
/// `d - left_depth + right_depth` of the other, and the other way round.
struct Edge {
    left: Node,
    left_depth: usize,
    right: Node,
    right_depth: usize,
}

#[derive(Default)]
struct Graph {
    shapes: Vec<Depths>,
    edges: Vec<Edge>,
}

impl Graph {
    fn node(&mut self) -> Node {
        self.shapes.push(Depths::new());
        self.shapes.len() - 1
    }

    fn link(&mut self, left: Node, left_depth: usize, right: Node, right_depth: usize) {
        if left != right || left_depth != right_depth {
            self.edges.push(Edge {
                left,
                left_depth,
                right,
                right_depth,
            });
        }
    }

    fn solve(&mut self) {
        loop {
            let mut changed = false;
            for edge in &self.edges {
                for (from, from_depth, to, to_depth) in [
                    (edge.left, edge.left_depth, edge.right, edge.right_depth),
                    (edge.right, edge.right_depth, edge.left, edge.left_depth),
                ] {
                    let reached: Vec<usize> = self.shapes[from]
                        .iter()
                        .filter_map(|d| d.checked_sub(from_depth))
                        .map(|d| d + to_depth)
                        .filter(|d| *d <= DEPTH_LIMIT)
                        .collect();
                    for depth in reached {
                        changed |= self.shapes[to].insert(depth);
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }
}

struct Builder<'t, 'i> {
    types: &'t TypeEnv,
    interner: &'i Interner,
    graph: Graph,
    names: HashMap<Symbol, Node>,
    bindings: HashMap<usize, Node>,
    expressions: HashMap<usize, Node>,
    parameters: HashMap<(Symbol, usize), Node>,
    parameter_names: HashMap<(Symbol, usize), Symbol>,
    returns: HashMap<Symbol, Node>,
    fields: HashMap<(Option<Symbol>, Symbol), Node>,
    current: Option<Symbol>,
}

impl Builder<'_, '_> {
    /// The node of `name` in scope, or a fresh one for a name nothing in scope binds.
    fn name(&mut self, name: Symbol) -> Node {
        match self.names.get(&name) {
            Some(node) => *node,
            None => {
                let node = self.graph.node();
                self.names.insert(name, node);
                node
            }
        }
    }

    fn field(&mut self, object: &Expr, field: Symbol) -> Node {
        let owner = match self.types.infer_expr(object, self.interner) {
            LogosType::UserDefined(owner) => Some(owner),
            _ => None,
        };
        match self.fields.get(&(owner, field)) {
            Some(node) => *node,
            None => {
                let node = self.graph.node();
                self.fields.insert((owner, field), node);
                node
            }
        }
    }

    fn call(&mut self, function: Symbol, args: &[&Expr]) -> Option<Node> {
        for (index, arg) in args.iter().enumerate() {
            let value = self.expr(arg);
            if let Some(param) = self.parameters.get(&(function, index)).copied() {
                self.graph.link(value, 0, param, 0);
            }
        }
        self.returns.get(&function).copied()
    }

    fn expr(&mut self, expr: &Expr) -> Node {
        let key = expr as *const Expr as usize;
        if let Some(node) = self.expressions.get(&key) {
            return *node;
        }
        let node = match expr {
            Expr::Identifier(name) => self.name(*name),
            Expr::FieldAccess { object, field } => {
                self.expr(object);
                self.field(object, *field)
            }
            _ => self.graph.node(),
        };
        self.expressions.insert(key, node);
        match expr {
            Expr::Literal(Literal::Nothing) => {
                self.graph.shapes[node].insert(0);
            }
            Expr::Index { collection, index } => {
                let parent = self.expr(collection);
                self.expr(index);
                self.graph.link(parent, 1, node, 0);
            }
            Expr::List(items) => {
                for item in items.iter() {
                    let child = self.expr(item);
                    self.graph.link(node, 1, child, 0);
                }
            }
            Expr::Copy { expr: value } | Expr::Give { value } => {
                let source = self.expr(value);
                self.graph.link(node, 0, source, 0);
            }
            Expr::Slice { collection, .. } | Expr::WithCapacity { value: collection, .. } => {
                let source = self.expr(collection);
                self.graph.link(node, 0, source, 0);
                for child in expr_children(expr) {
                    self.expr(child);
                }
            }
            Expr::BinaryOp {
                op: BinaryOpKind::SeqConcat,
                left,
                right,
            } => {
                for part in [left, right] {
                    let source = self.expr(part);
                    self.graph.link(node, 0, source, 0);
                }
            }
            Expr::Call { function, args } => {
                if let Some(returned) = self.call(*function, args) {
                    self.graph.link(node, 0, returned, 0);
                }
            }
            Expr::New { init_fields, .. } => {
                let owner = match self.types.infer_expr(expr, self.interner) {
                    LogosType::UserDefined(owner) => Some(owner),
                    _ => None,
                };
                for (field, value) in init_fields.iter() {
                    let source = self.expr(value);
                    let target = match self.fields.get(&(owner, *field)) {
                        Some(target) => *target,
                        None => {
                            let target = self.graph.node();
                            self.fields.insert((owner, *field), target);
                            target
                        }
                    };
                    self.graph.link(target, 0, source, 0);
                }
            }
            Expr::Identifier(_) | Expr::FieldAccess { .. } => {}
            _ => {
                for child in expr_children(expr) {
                    self.expr(child);
                }
            }
        }
        node
    }

    fn block(&mut self, body: &[Stmt]) {
        for stmt in body {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::FunctionDef { .. } => return,
            Stmt::Let { var, value, .. } => {
                let source = self.expr(value);
                let node = self.graph.node();
                self.graph.link(node, 0, source, 0);
                self.bindings.insert(stmt as *const Stmt as usize, node);
                self.names.insert(*var, node);
                return;
            }
            Stmt::Set { target, value } => {
                let source = self.expr(value);
                let node = self.name(*target);
                self.graph.link(node, 0, source, 0);
                return;
            }
            Stmt::Swap { a, b } => {
                let (a, b) = (self.name(*a), self.name(*b));
                self.graph.link(a, 0, b, 0);
                return;
            }
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => {
                let parent = self.expr(collection);
                self.expr(index);
                let child = self.expr(value);
                self.graph.link(parent, 1, child, 0);
                return;
            }
            Stmt::Push { collection, value } | Stmt::Add { collection, value } => {
                let parent = self.expr(collection);
                let child = self.expr(value);
                self.graph.link(parent, 1, child, 0);
                return;
            }
            Stmt::SetField {
                object,
                field,
                value,
            } => {
                self.expr(object);
                let source = self.expr(value);
                let target = self.field(object, *field);
                self.graph.link(target, 0, source, 0);
                return;
            }
            Stmt::Call { function, args } => {
                self.call(*function, args);
                return;
            }
            Stmt::Return { value: Some(value) } => {
                let source = self.expr(value);
                if let Some(returned) = self.current.and_then(|f| self.returns.get(&f).copied()) {
                    self.graph.link(returned, 0, source, 0);
                }
                return;
            }
            Stmt::Pop {
                collection,
                into: Some(into),
            } => {
                let parent = self.expr(collection);
                let node = self.graph.node();
                self.graph.link(parent, 1, node, 0);
                self.names.insert(*into, node);
                return;
            }
            Stmt::Repeat {
                pattern: Pattern::Identifier(item),
                iterable,
                body,
            } => {
                let parent = self.expr(iterable);
                let outer = self.names.clone();
                let node = self.graph.node();
                self.graph.link(parent, 1, node, 0);
                self.names.insert(*item, node);
                self.block(body);
                self.names = outer;
                return;
            }
            _ => {}
        }
        for child in logicaffeine_language::ast_depth::stmt_expr_children(stmt) {
            self.expr(child);
        }
        walk_stmt(stmt, self);
    }

    fn finish(mut self) -> Absence {
        self.graph.solve();
        fn shapes<K: Eq + std::hash::Hash>(nodes: HashMap<K, Node>, graph: &Graph) -> HashMap<K, Depths> {
            nodes
                .into_iter()
                .filter(|(_, node)| !graph.shapes[*node].is_empty())
                .map(|(key, node)| (key, graph.shapes[node].clone()))
                .collect()
        }
        let parameters = self
            .parameters
            .into_iter()
            .map(|((function, index), node)| ((function, self.parameter_names[&(function, index)]), node))
            .collect();
        Absence {
            bindings: shapes(self.bindings, &self.graph),
            parameters: shapes(parameters, &self.graph),
            returns: shapes(self.returns, &self.graph),
            fields: shapes(self.fields, &self.graph),
        }
    }
}

/// The binders of the statements this builder does not relate itself, and their nested blocks in scopes of their own.
impl EffectSink for Builder<'_, '_> {
    fn bind(&mut self, name: Symbol, how: Bind) {
        if matches!(how, Bind::Declare) {
            let node = self.graph.node();
            self.names.insert(name, node);
        }
    }
    fn mutate(&mut self, _root: Symbol) {}
    fn call(&mut self) {}
    fn opaque(&mut self) {}
    fn scoped(&mut self, binders: &[Symbol], body: &[Stmt]) {
        let outer = self.names.clone();
        for binder in binders {
            let node = self.graph.node();
            self.names.insert(*binder, node);
        }
        self.block(body);
        self.names = outer;
    }
    fn transparent(&mut self, body: &[Stmt]) {
        self.block(body);
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/analysis/absence/tests.rs"]
mod tests;
