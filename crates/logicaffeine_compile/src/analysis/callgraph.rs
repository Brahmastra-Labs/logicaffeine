use std::collections::{HashMap, HashSet};

use logicaffeine_base::{Interner, Symbol};
use logicaffeine_language::ast::{Expr, Stmt};
use logicaffeine_language::ast::stmt::ClosureBody;

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
            if let Stmt::FunctionDef { name, body, is_native, .. } = stmt {
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

        Self { edges, native_fns, sccs }
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
        if self.edges.get(&fn_sym).map(|s| s.contains(&fn_sym)).unwrap_or(false) {
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

fn collect_calls_from_stmts(stmts: &[Stmt<'_>], calls: &mut HashSet<Symbol>) {
    for stmt in stmts {
        collect_calls_from_stmt(stmt, calls);
    }
}

fn collect_calls_from_stmt(stmt: &Stmt<'_>, calls: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Call { function, args } => {
            calls.insert(*function);
            for arg in args {
                collect_calls_from_expr(arg, calls);
            }
        }
        Stmt::Let { value, .. } => collect_calls_from_expr(value, calls),
        Stmt::Set { value, .. } => collect_calls_from_expr(value, calls),
        Stmt::Return { value: Some(v) } => collect_calls_from_expr(v, calls),
        Stmt::If { cond, then_block, else_block } => {
            collect_calls_from_expr(cond, calls);
            collect_calls_from_stmts(then_block, calls);
            if let Some(else_b) = else_block {
                collect_calls_from_stmts(else_b, calls);
            }
        }
        Stmt::While { cond, body, .. } => {
            collect_calls_from_expr(cond, calls);
            collect_calls_from_stmts(body, calls);
        }
        Stmt::Repeat { iterable, body, .. } => {
            collect_calls_from_expr(iterable, calls);
            collect_calls_from_stmts(body, calls);
        }
        Stmt::Push { value, collection } => {
            collect_calls_from_expr(value, calls);
            collect_calls_from_expr(collection, calls);
        }
        Stmt::Pop { collection, .. } => collect_calls_from_expr(collection, calls),
        Stmt::Add { value, collection } => {
            collect_calls_from_expr(value, calls);
            collect_calls_from_expr(collection, calls);
        }
        Stmt::Remove { value, collection } => {
            collect_calls_from_expr(value, calls);
            collect_calls_from_expr(collection, calls);
        }
        Stmt::SetIndex { collection, index, value } => {
            collect_calls_from_expr(collection, calls);
            collect_calls_from_expr(index, calls);
            collect_calls_from_expr(value, calls);
        }
        Stmt::SetField { object, value, .. } => {
            collect_calls_from_expr(object, calls);
            collect_calls_from_expr(value, calls);
        }
        Stmt::Inspect { target, arms, .. } => {
            collect_calls_from_expr(target, calls);
            for arm in arms {
                collect_calls_from_stmts(arm.body, calls);
            }
        }
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            collect_calls_from_stmts(tasks, calls);
        }
        Stmt::Zone { body, .. } => collect_calls_from_stmts(body, calls),
        _ => {}
    }
}

fn collect_calls_from_expr(expr: &Expr<'_>, calls: &mut HashSet<Symbol>) {
    match expr {
        Expr::Call { function, args } => {
            calls.insert(*function);
            for arg in args {
                collect_calls_from_expr(arg, calls);
            }
        }
        Expr::Closure { body, .. } => match body {
            ClosureBody::Expression(e) => collect_calls_from_expr(e, calls),
            ClosureBody::Block(stmts) => collect_calls_from_stmts(stmts, calls),
        },
        Expr::BinaryOp { left, right, .. } => {
            collect_calls_from_expr(left, calls);
            collect_calls_from_expr(right, calls);
        }
        Expr::Index { collection, index } => {
            collect_calls_from_expr(collection, calls);
            collect_calls_from_expr(index, calls);
        }
        Expr::Slice { collection, start, end } => {
            collect_calls_from_expr(collection, calls);
            collect_calls_from_expr(start, calls);
            collect_calls_from_expr(end, calls);
        }
        Expr::Length { collection } => collect_calls_from_expr(collection, calls),
        Expr::Contains { collection, value } => {
            collect_calls_from_expr(collection, calls);
            collect_calls_from_expr(value, calls);
        }
        Expr::Union { left, right } | Expr::Intersection { left, right } => {
            collect_calls_from_expr(left, calls);
            collect_calls_from_expr(right, calls);
        }
        Expr::FieldAccess { object, .. } => collect_calls_from_expr(object, calls),
        Expr::List(items) | Expr::Tuple(items) => {
            for item in items {
                collect_calls_from_expr(item, calls);
            }
        }
        Expr::Range { start, end } => {
            collect_calls_from_expr(start, calls);
            collect_calls_from_expr(end, calls);
        }
        Expr::Copy { expr } | Expr::Give { value: expr } => {
            collect_calls_from_expr(expr, calls);
        }
        Expr::OptionSome { value } => collect_calls_from_expr(value, calls),
        Expr::WithCapacity { value, capacity } => {
            collect_calls_from_expr(value, calls);
            collect_calls_from_expr(capacity, calls);
        }
        Expr::CallExpr { callee, args } => {
            collect_calls_from_expr(callee, calls);
            for arg in args {
                collect_calls_from_expr(arg, calls);
            }
        }
        _ => {}
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
