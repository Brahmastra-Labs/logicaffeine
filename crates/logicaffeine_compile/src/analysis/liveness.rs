use std::collections::{HashMap, HashSet};

use logicaffeine_base::Symbol;
use logicaffeine_language::ast::{Expr, Stmt};
use logicaffeine_language::ast::stmt::{ClosureBody, Pattern};

/// Liveness analysis result for a whole program.
///
/// For each user-defined function, stores a per-statement `live_after` set:
/// `live_after[i]` is the set of variables live immediately *after* top-level
/// statement `i` in that function's body.
///
/// Used by codegen to decide when an argument can be *moved* into a call
/// instead of cloned: if a variable is not live after the call statement,
/// the old value is never read again and ownership can be transferred.
pub struct LivenessResult {
    functions: HashMap<Symbol, FunctionLiveness>,
}

/// Per-function liveness table.
pub struct FunctionLiveness {
    /// `live_after[i]` = variables live after top-level statement `i`.
    pub live_after: Vec<HashSet<Symbol>>,
}

impl LivenessResult {
    /// Compute liveness for every `FunctionDef` in `stmts`.
    ///
    /// Algorithm: backward dataflow over the top-level statement list of each
    /// function body.  `Return` is treated as a terminator — variables used in
    /// subsequent (dead-code) statements do not affect liveness before the
    /// `Return`.
    pub fn analyze(stmts: &[Stmt<'_>]) -> Self {
        let mut functions = HashMap::new();
        for stmt in stmts {
            if let Stmt::FunctionDef { name, body, .. } = stmt {
                functions.insert(*name, analyze_function(body));
            }
        }
        Self { functions }
    }

    /// Returns `true` if `var` is live immediately after top-level statement
    /// `stmt_idx` in function `fn_sym`.
    ///
    /// Returns `false` for unknown functions, out-of-bounds indices, or when
    /// the variable is definitely dead.
    pub fn is_live_after(&self, fn_sym: Symbol, stmt_idx: usize, var: Symbol) -> bool {
        self.functions
            .get(&fn_sym)
            .and_then(|fl| fl.live_after.get(stmt_idx))
            .map(|s| s.contains(&var))
            .unwrap_or(false)
    }

    /// Returns the live-after set for statement `stmt_idx` in `fn_sym`.
    ///
    /// Returns a reference to an empty set when the function or index is
    /// unknown.
    pub fn live_after(&self, fn_sym: Symbol, stmt_idx: usize) -> &HashSet<Symbol> {
        static EMPTY: std::sync::OnceLock<HashSet<Symbol>> = std::sync::OnceLock::new();
        self.functions
            .get(&fn_sym)
            .and_then(|fl| fl.live_after.get(stmt_idx))
            .unwrap_or_else(|| EMPTY.get_or_init(HashSet::new))
    }
}

// =============================================================================
// Per-function analysis
// =============================================================================

fn analyze_function(body: &[Stmt<'_>]) -> FunctionLiveness {
    let n = body.len();
    let mut live_after = vec![HashSet::<Symbol>::new(); n];
    let mut current: HashSet<Symbol> = HashSet::new();

    for i in (0..n).rev() {
        if is_terminator(&body[i]) {
            // Return (or similar terminator): nothing is live after it,
            // and dead code that follows does not influence pre-Return liveness.
            live_after[i] = HashSet::new();
            current = gen_stmt(&body[i]);
        } else {
            live_after[i] = current.clone();
            current = live_before_stmt(&body[i], &current);
        }
    }

    FunctionLiveness { live_after }
}

fn is_terminator(stmt: &Stmt<'_>) -> bool {
    matches!(stmt, Stmt::Return { .. })
}

// =============================================================================
// Live-before computation
// =============================================================================

/// Variables generated (used) by a statement, ignoring control flow.
/// Used only for terminators (Return).
fn gen_stmt(stmt: &Stmt<'_>) -> HashSet<Symbol> {
    let mut out = HashSet::new();
    match stmt {
        Stmt::Return { value: Some(v) } => gen_expr(v, &mut out),
        _ => {}
    }
    out
}

/// Compute the live-before set for a single statement given what is live after it.
fn live_before_stmt(stmt: &Stmt<'_>, live_out: &HashSet<Symbol>) -> HashSet<Symbol> {
    match stmt {
        Stmt::Return { .. } => gen_stmt(stmt),

        Stmt::Let { var, value, .. } => {
            let mut result = live_out.clone();
            result.remove(var);
            gen_expr(value, &mut result);
            result
        }

        Stmt::Set { target, value } => {
            let mut result = live_out.clone();
            result.remove(target);
            gen_expr(value, &mut result);
            result
        }

        Stmt::Call { args, .. } => {
            let mut result = live_out.clone();
            for a in args.iter() {
                gen_expr(a, &mut result);
            }
            result
        }

        Stmt::Push { value, collection } => {
            let mut result = live_out.clone();
            gen_expr(value, &mut result);
            gen_expr(collection, &mut result);
            result
        }

        Stmt::Pop { collection, into } => {
            let mut result = live_out.clone();
            if let Some(v) = into {
                result.remove(v);
            }
            gen_expr(collection, &mut result);
            result
        }

        Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
            let mut result = live_out.clone();
            gen_expr(value, &mut result);
            gen_expr(collection, &mut result);
            result
        }

        Stmt::SetIndex { collection, index, value } => {
            let mut result = live_out.clone();
            gen_expr(collection, &mut result);
            gen_expr(index, &mut result);
            gen_expr(value, &mut result);
            result
        }

        Stmt::SetField { object, value, .. } => {
            let mut result = live_out.clone();
            gen_expr(object, &mut result);
            gen_expr(value, &mut result);
            result
        }

        Stmt::If { cond, then_block, else_block } => {
            let then_lb = live_before_block(then_block, live_out);
            let else_lb = match else_block {
                Some(b) => live_before_block(b, live_out),
                None => live_out.clone(),
            };
            let mut result = HashSet::new();
            gen_expr(cond, &mut result);
            result.extend(then_lb);
            result.extend(else_lb);
            result
        }

        Stmt::While { cond, body, .. } => {
            // Fixed-point: loop_live = live_out ∪ gen(cond) ∪ body_live_before(loop_live)
            let mut loop_live: HashSet<Symbol> = live_out.clone();
            gen_expr(cond, &mut loop_live);
            loop {
                let body_before = live_before_block(body, &loop_live);
                let mut new_live = live_out.clone();
                gen_expr(cond, &mut new_live);
                new_live.extend(body_before);
                if new_live == loop_live {
                    break;
                }
                loop_live = new_live;
            }
            loop_live
        }

        Stmt::Repeat { pattern, iterable, body } => {
            let body_before = live_before_block(body, live_out);
            let pattern_syms: HashSet<Symbol> = match pattern {
                Pattern::Identifier(s) => [*s].into_iter().collect(),
                Pattern::Tuple(syms) => syms.iter().copied().collect(),
            };
            let mut result = live_out.clone();
            gen_expr(iterable, &mut result);
            for sym in body_before {
                if !pattern_syms.contains(&sym) {
                    result.insert(sym);
                }
            }
            result
        }

        Stmt::Inspect { target, arms, .. } => {
            let mut result = HashSet::new();
            for arm in arms.iter() {
                let arm_lb = live_before_block(arm.body, live_out);
                result.extend(arm_lb);
            }
            gen_expr(target, &mut result);
            result
        }

        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            live_before_block(tasks, live_out)
        }

        Stmt::Zone { body, .. } => live_before_block(body, live_out),

        _ => live_out.clone(),
    }
}

/// Compute live-before for a block of statements.
fn live_before_block(stmts: &[Stmt<'_>], live_out: &HashSet<Symbol>) -> HashSet<Symbol> {
    let mut current = live_out.clone();
    for stmt in stmts.iter().rev() {
        if is_terminator(stmt) {
            current = gen_stmt(stmt);
        } else {
            current = live_before_stmt(stmt, &current);
        }
    }
    current
}

// =============================================================================
// Gen set for expressions
// =============================================================================

/// Collects all variable identifiers referenced (used) in an expression.
///
/// Function names in `Expr::Call { function, .. }` are NOT collected since
/// they are global names, not local variables.
fn gen_expr(expr: &Expr<'_>, out: &mut HashSet<Symbol>) {
    match expr {
        Expr::Identifier(sym) => {
            out.insert(*sym);
        }
        Expr::BinaryOp { left, right, .. } => {
            gen_expr(left, out);
            gen_expr(right, out);
        }
        Expr::Call { args, .. } => {
            for a in args.iter() {
                gen_expr(a, out);
            }
        }
        Expr::CallExpr { callee, args } => {
            gen_expr(callee, out);
            for a in args.iter() {
                gen_expr(a, out);
            }
        }
        Expr::Length { collection } => gen_expr(collection, out),
        Expr::Index { collection, index } => {
            gen_expr(collection, out);
            gen_expr(index, out);
        }
        Expr::Slice { collection, start, end } => {
            gen_expr(collection, out);
            gen_expr(start, out);
            gen_expr(end, out);
        }
        Expr::Contains { collection, value } => {
            gen_expr(collection, out);
            gen_expr(value, out);
        }
        Expr::Union { left, right } | Expr::Intersection { left, right } => {
            gen_expr(left, out);
            gen_expr(right, out);
        }
        Expr::ManifestOf { zone } => gen_expr(zone, out),
        Expr::ChunkAt { index, zone } => {
            gen_expr(index, out);
            gen_expr(zone, out);
        }
        Expr::FieldAccess { object, .. } => gen_expr(object, out),
        Expr::List(items) | Expr::Tuple(items) => {
            for i in items.iter() {
                gen_expr(i, out);
            }
        }
        Expr::Range { start, end } => {
            gen_expr(start, out);
            gen_expr(end, out);
        }
        Expr::Copy { expr } | Expr::Give { value: expr } | Expr::Not { operand: expr } => gen_expr(expr, out),
        Expr::OptionSome { value } => gen_expr(value, out),
        Expr::WithCapacity { value, capacity } => {
            gen_expr(value, out);
            gen_expr(capacity, out);
        }
        Expr::New { init_fields, .. } => {
            for (_, v) in init_fields.iter() {
                gen_expr(v, out);
            }
        }
        Expr::NewVariant { fields, .. } => {
            for (_, v) in fields.iter() {
                gen_expr(v, out);
            }
        }
        Expr::Closure { body, .. } => match body {
            ClosureBody::Expression(e) => gen_expr(e, out),
            ClosureBody::Block(stmts) => {
                for s in stmts.iter() {
                    gen_stmt_exprs(s, out);
                }
            }
        },
        Expr::InterpolatedString(parts) => {
            for part in parts.iter() {
                if let logicaffeine_language::ast::stmt::StringPart::Expr { value, .. } = part {
                    gen_expr(value, out);
                }
            }
        }
        Expr::Literal(_) | Expr::OptionNone | Expr::Escape { .. } => {}
    }
}

/// Collects all variable identifiers referenced in a statement's expressions.
/// Used for closure bodies to conservatively compute the gen set.
fn gen_stmt_exprs(stmt: &Stmt<'_>, out: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Let { value, .. } => gen_expr(value, out),
        Stmt::Set { value, .. } => gen_expr(value, out),
        Stmt::Return { value: Some(v) } => gen_expr(v, out),
        Stmt::Call { args, .. } => {
            for a in args.iter() { gen_expr(a, out); }
        }
        Stmt::Push { value, collection } => {
            gen_expr(value, out);
            gen_expr(collection, out);
        }
        Stmt::If { cond, then_block, else_block } => {
            gen_expr(cond, out);
            for s in then_block.iter() { gen_stmt_exprs(s, out); }
            if let Some(b) = else_block {
                for s in b.iter() { gen_stmt_exprs(s, out); }
            }
        }
        Stmt::While { cond, body, .. } => {
            gen_expr(cond, out);
            for s in body.iter() { gen_stmt_exprs(s, out); }
        }
        _ => {}
    }
}
