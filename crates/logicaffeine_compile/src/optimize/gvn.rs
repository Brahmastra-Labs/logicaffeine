//! Local Common Subexpression Elimination via Value Numbering.
//!
//! Identifies identical subexpressions within a basic block and replaces
//! redundant computations with references to previously computed values.
//! Fresh table per basic block (If/While/Repeat boundaries).

use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};

/// Structural key for expressions, used for equality comparison in CSE.
#[derive(Hash, Eq, PartialEq, Clone)]
enum ExprKey {
    Ident(Symbol),
    LitInt(i64),
    LitBool(bool),
    LitText(Symbol),
    BinaryOp(BinaryOpKind, Box<ExprKey>, Box<ExprKey>),
    Length(Box<ExprKey>),
    Not(Box<ExprKey>),
    FieldAccess(Box<ExprKey>, Symbol),
    Index(Box<ExprKey>, Box<ExprKey>),
    Contains(Box<ExprKey>, Box<ExprKey>),
}

/// Compute a structural key for an expression.
/// Returns None for expressions that should not be CSE'd (calls, closures, etc.).
fn compute_key(expr: &Expr) -> Option<ExprKey> {
    match expr {
        Expr::Identifier(sym) => Some(ExprKey::Ident(*sym)),
        Expr::Literal(Literal::Number(n)) => Some(ExprKey::LitInt(*n)),
        Expr::Literal(Literal::Boolean(b)) => Some(ExprKey::LitBool(*b)),
        Expr::Literal(Literal::Text(s)) => Some(ExprKey::LitText(*s)),
        Expr::BinaryOp { op, left, right } => {
            let lk = compute_key(left)?;
            let rk = compute_key(right)?;
            Some(ExprKey::BinaryOp(*op, Box::new(lk), Box::new(rk)))
        }
        Expr::Length { collection } => {
            let ck = compute_key(collection)?;
            Some(ExprKey::Length(Box::new(ck)))
        }
        Expr::Not { operand } => {
            let ok = compute_key(operand)?;
            Some(ExprKey::Not(Box::new(ok)))
        }
        Expr::FieldAccess { object, field } => {
            let ok = compute_key(object)?;
            Some(ExprKey::FieldAccess(Box::new(ok), *field))
        }
        Expr::Index { collection, index } => {
            let ck = compute_key(collection)?;
            let ik = compute_key(index)?;
            Some(ExprKey::Index(Box::new(ck), Box::new(ik)))
        }
        Expr::Contains { collection, value } => {
            let ck = compute_key(collection)?;
            let vk = compute_key(value)?;
            Some(ExprKey::Contains(Box::new(ck), Box::new(vk)))
        }
        // Don't CSE: function calls (may have side effects), literals (trivial),
        // closures, new constructors, escape blocks, etc.
        _ => None,
    }
}

/// Returns true if the expression is non-trivial (worth hoisting into a temp).
fn is_non_trivial(expr: &Expr) -> bool {
    !matches!(expr, Expr::Identifier(_) | Expr::Literal(_))
}

/// Collect all symbols referenced in an expression key.
fn collect_key_symbols(key: &ExprKey, out: &mut HashSet<Symbol>) {
    match key {
        ExprKey::Ident(sym) => { out.insert(*sym); }
        ExprKey::LitInt(_) | ExprKey::LitBool(_) | ExprKey::LitText(_) => {}
        ExprKey::BinaryOp(_, l, r) | ExprKey::Index(l, r) | ExprKey::Contains(l, r) => {
            collect_key_symbols(l, out);
            collect_key_symbols(r, out);
        }
        ExprKey::Length(inner) | ExprKey::Not(inner) => {
            collect_key_symbols(inner, out);
        }
        ExprKey::FieldAccess(obj, _) => {
            collect_key_symbols(obj, out);
        }
    }
}

/// CSE state for a basic block.
struct CseState {
    /// Maps expression key to the symbol that holds the computed value.
    cache: HashMap<ExprKey, Symbol>,
    /// Maps each symbol to the set of cache keys that depend on it.
    deps: HashMap<Symbol, Vec<ExprKey>>,
    /// Counter for generating unique temp variable names.
    counter: u32,
}

impl CseState {
    fn new() -> Self {
        Self {
            cache: HashMap::new(),
            deps: HashMap::new(),
            counter: 0,
        }
    }

    /// Add an expression to the cache, associated with the given symbol.
    fn insert(&mut self, key: ExprKey, sym: Symbol) {
        let mut syms = HashSet::new();
        collect_key_symbols(&key, &mut syms);
        for s in syms {
            self.deps.entry(s).or_default().push(key.clone());
        }
        self.cache.insert(key, sym);
    }

    /// Invalidate all cache entries that reference the given symbol.
    fn invalidate(&mut self, sym: Symbol) {
        if let Some(keys) = self.deps.remove(&sym) {
            for key in keys {
                self.cache.remove(&key);
            }
        }
    }

    /// Look up an expression key in the cache.
    fn lookup(&self, key: &ExprKey) -> Option<Symbol> {
        self.cache.get(key).copied()
    }
}

/// Count occurrences of each subexpression key within an expression tree.
fn count_subexpressions(expr: &Expr, counts: &mut HashMap<ExprKey, usize>) {
    if let Some(key) = compute_key(expr) {
        if is_non_trivial(expr) {
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    // Recurse into sub-expressions
    match expr {
        Expr::BinaryOp { left, right, .. } => {
            count_subexpressions(left, counts);
            count_subexpressions(right, counts);
        }
        Expr::Length { collection } | Expr::Not { operand: collection } => {
            count_subexpressions(collection, counts);
        }
        Expr::FieldAccess { object, .. } => {
            count_subexpressions(object, counts);
        }
        Expr::Index { collection, index } | Expr::Contains { collection, value: index } => {
            count_subexpressions(collection, counts);
            count_subexpressions(index, counts);
        }
        _ => {}
    }
}

/// Rewrite an expression, replacing subexpressions that appear multiple times
/// with hoisted temporaries, and replacing expressions found in the cross-statement cache.
fn rewrite_expr<'a>(
    expr: &'a Expr<'a>,
    intra_counts: &HashMap<ExprKey, usize>,
    local_cache: &mut HashMap<ExprKey, Symbol>,
    state: &mut CseState,
    hoisted: &mut Vec<Stmt<'a>>,
    interner: &mut Interner,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> &'a Expr<'a> {
    let key = compute_key(expr);

    // Check local (intra-expression) cache first
    if let Some(ref k) = key {
        if let Some(sym) = local_cache.get(k) {
            return expr_arena.alloc(Expr::Identifier(*sym));
        }
        // Check cross-statement cache
        if is_non_trivial(expr) {
            if let Some(sym) = state.lookup(k) {
                return expr_arena.alloc(Expr::Identifier(sym));
            }
        }
    }

    // Recursively process sub-expressions
    let new_expr: &'a Expr<'a> = match expr {
        Expr::BinaryOp { op, left, right } => {
            let new_left = rewrite_expr(left, intra_counts, local_cache, state, hoisted, interner, expr_arena, stmt_arena);
            let new_right = rewrite_expr(right, intra_counts, local_cache, state, hoisted, interner, expr_arena, stmt_arena);
            if std::ptr::eq(*left, new_left) && std::ptr::eq(*right, new_right) {
                expr
            } else {
                expr_arena.alloc(Expr::BinaryOp { op: *op, left: new_left, right: new_right })
            }
        }
        Expr::Length { collection } => {
            let new_coll = rewrite_expr(collection, intra_counts, local_cache, state, hoisted, interner, expr_arena, stmt_arena);
            if std::ptr::eq(*collection, new_coll) { expr }
            else { expr_arena.alloc(Expr::Length { collection: new_coll }) }
        }
        Expr::Not { operand } => {
            let new_op = rewrite_expr(operand, intra_counts, local_cache, state, hoisted, interner, expr_arena, stmt_arena);
            if std::ptr::eq(*operand, new_op) { expr }
            else { expr_arena.alloc(Expr::Not { operand: new_op }) }
        }
        Expr::FieldAccess { object, field } => {
            let new_obj = rewrite_expr(object, intra_counts, local_cache, state, hoisted, interner, expr_arena, stmt_arena);
            if std::ptr::eq(*object, new_obj) { expr }
            else { expr_arena.alloc(Expr::FieldAccess { object: new_obj, field: *field }) }
        }
        Expr::Index { collection, index } => {
            let new_coll = rewrite_expr(collection, intra_counts, local_cache, state, hoisted, interner, expr_arena, stmt_arena);
            let new_idx = rewrite_expr(index, intra_counts, local_cache, state, hoisted, interner, expr_arena, stmt_arena);
            if std::ptr::eq(*collection, new_coll) && std::ptr::eq(*index, new_idx) { expr }
            else { expr_arena.alloc(Expr::Index { collection: new_coll, index: new_idx }) }
        }
        Expr::Contains { collection, value } => {
            let new_coll = rewrite_expr(collection, intra_counts, local_cache, state, hoisted, interner, expr_arena, stmt_arena);
            let new_val = rewrite_expr(value, intra_counts, local_cache, state, hoisted, interner, expr_arena, stmt_arena);
            if std::ptr::eq(*collection, new_coll) && std::ptr::eq(*value, new_val) { expr }
            else { expr_arena.alloc(Expr::Contains { collection: new_coll, value: new_val }) }
        }
        // Non-compound expressions: return as-is
        _ => expr,
    };

    // After processing children, check if this subexpression should be hoisted
    // (appears more than once intra-expression)
    if let Some(ref k) = key {
        if is_non_trivial(new_expr) {
            let count = intra_counts.get(k).copied().unwrap_or(0);
            if count > 1 {
                // Hoist into a temp variable
                let tmp_name = format!("__cse_{}", state.counter);
                state.counter += 1;
                let tmp_sym = interner.intern(&tmp_name);
                let let_stmt = stmt_arena.alloc(Stmt::Let {
                    var: tmp_sym,
                    ty: None,
                    value: new_expr,
                    mutable: false,
                });
                hoisted.push(let_stmt.clone());
                local_cache.insert(k.clone(), tmp_sym);
                state.insert(k.clone(), tmp_sym);
                return expr_arena.alloc(Expr::Identifier(tmp_sym));
            }
        }
    }

    new_expr
}

/// Process a single expression for CSE opportunities (cross-statement only, no hoisting).
/// Returns the potentially-replaced expression.
fn cse_expr_cross_stmt<'a>(
    expr: &'a Expr<'a>,
    state: &CseState,
    expr_arena: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    if let Some(key) = compute_key(expr) {
        if is_non_trivial(expr) {
            if let Some(sym) = state.lookup(&key) {
                return expr_arena.alloc(Expr::Identifier(sym));
            }
        }
    }
    expr
}

/// Run CSE on a block of statements, returning the optimized block.
fn cse_block<'a>(
    stmts: Vec<Stmt<'a>>,
    state: &mut CseState,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mut result = Vec::with_capacity(stmts.len());

    for stmt in stmts {
        match stmt {
            Stmt::Let { var, ty, value, mutable } => {
                // Count intra-expression subexpression occurrences
                let mut intra_counts = HashMap::new();
                count_subexpressions(value, &mut intra_counts);

                let has_intra_dupes = intra_counts.values().any(|&c| c > 1);

                if has_intra_dupes {
                    // Full rewrite with potential hoisting
                    let mut hoisted = Vec::new();
                    let mut local_cache = HashMap::new();
                    let new_value = rewrite_expr(
                        value, &intra_counts, &mut local_cache,
                        state, &mut hoisted, interner, expr_arena, stmt_arena,
                    );
                    result.extend(hoisted);
                    // Add the whole expression to cross-statement cache
                    if let Some(key) = compute_key(new_value) {
                        if is_non_trivial(new_value) {
                            state.insert(key, var);
                        }
                    }
                    result.push(Stmt::Let { var, ty, value: new_value, mutable });
                } else {
                    // Simple cross-statement check
                    let new_value = cse_expr_cross_stmt(value, state, expr_arena);
                    // Add to cache
                    if let Some(key) = compute_key(value) {
                        if is_non_trivial(value) {
                            state.insert(key, var);
                        }
                    }
                    result.push(Stmt::Let { var, ty, value: new_value, mutable });
                }
            }
            Stmt::Set { target, value } => {
                state.invalidate(target);
                result.push(Stmt::Set { target, value });
            }
            Stmt::Push { value, collection } => {
                if let Expr::Identifier(sym) = collection {
                    state.invalidate(*sym);
                }
                result.push(Stmt::Push { value, collection });
            }
            Stmt::Pop { collection, into } => {
                if let Expr::Identifier(sym) = collection {
                    state.invalidate(*sym);
                }
                result.push(Stmt::Pop { collection, into });
            }
            Stmt::Add { value, collection } => {
                if let Expr::Identifier(sym) = collection {
                    state.invalidate(*sym);
                }
                result.push(Stmt::Add { value, collection });
            }
            Stmt::Remove { value, collection } => {
                if let Expr::Identifier(sym) = collection {
                    state.invalidate(*sym);
                }
                result.push(Stmt::Remove { value, collection });
            }
            Stmt::SetIndex { collection, index, value } => {
                if let Expr::Identifier(sym) = collection {
                    state.invalidate(*sym);
                }
                result.push(Stmt::SetIndex { collection, index, value });
            }
            Stmt::SetField { object, field, value } => {
                if let Expr::Identifier(sym) = object {
                    state.invalidate(*sym);
                }
                result.push(Stmt::SetField { object, field, value });
            }
            // Control flow: process inner blocks with fresh state, then clear outer cache
            Stmt::If { cond, then_block, else_block } => {
                let saved_cache = state.cache.clone();
                let saved_deps = state.deps.clone();

                let new_then = cse_block(then_block.to_vec(), state, expr_arena, stmt_arena, interner);
                let new_then_block: Block = stmt_arena.alloc_slice(new_then);

                // Restore state for else block
                state.cache = saved_cache.clone();
                state.deps = saved_deps.clone();

                let new_else = else_block.map(|eb| {
                    let processed = cse_block(eb.to_vec(), state, expr_arena, stmt_arena, interner);
                    let b: Block = stmt_arena.alloc_slice(processed);
                    b
                });

                // After If: conservatively clear cache (variables may have been modified)
                state.cache = saved_cache;
                state.deps = saved_deps;
                // Invalidate everything that might be modified in the branches
                invalidate_block_writes(then_block, state);
                if let Some(eb) = else_block {
                    invalidate_block_writes(eb, state);
                }

                result.push(Stmt::If { cond, then_block: new_then_block, else_block: new_else });
            }
            Stmt::While { cond, body, decreasing } => {
                let saved_cache = state.cache.clone();
                let saved_deps = state.deps.clone();

                let new_body = cse_block(body.to_vec(), state, expr_arena, stmt_arena, interner);
                let new_body_block: Block = stmt_arena.alloc_slice(new_body);

                // After While: conservatively clear for variables modified in loop
                state.cache = saved_cache;
                state.deps = saved_deps;
                invalidate_block_writes(body, state);

                result.push(Stmt::While { cond, body: new_body_block, decreasing });
            }
            Stmt::Repeat { pattern, iterable, body } => {
                let saved_cache = state.cache.clone();
                let saved_deps = state.deps.clone();

                let new_body = cse_block(body.to_vec(), state, expr_arena, stmt_arena, interner);
                let new_body_block: Block = stmt_arena.alloc_slice(new_body);

                state.cache = saved_cache;
                state.deps = saved_deps;
                invalidate_block_writes(body, state);

                result.push(Stmt::Repeat { pattern, iterable, body: new_body_block });
            }
            Stmt::FunctionDef { name, generics, params, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => {
                // Process function body with fresh state
                let mut fn_state = CseState::new();
                fn_state.counter = state.counter;
                let new_body = cse_block(body.to_vec(), &mut fn_state, expr_arena, stmt_arena, interner);
                state.counter = fn_state.counter;
                let new_body_block: Block = stmt_arena.alloc_slice(new_body);
                result.push(Stmt::FunctionDef {
                    name, generics, params, body: new_body_block, return_type,
                    is_native, native_path, is_exported, export_target, opt_flags,
                });
            }
            // All other statements pass through unchanged
            other => result.push(other),
        }
    }

    result
}

/// Scan a block for all write targets and invalidate them in the CSE state.
fn invalidate_block_writes(stmts: &[Stmt], state: &mut CseState) {
    for stmt in stmts {
        match stmt {
            Stmt::Set { target, .. } => state.invalidate(*target),
            Stmt::Push { collection, .. } | Stmt::Pop { collection, .. }
            | Stmt::Add { collection, .. } | Stmt::Remove { collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    state.invalidate(*sym);
                }
            }
            Stmt::SetIndex { collection, .. } | Stmt::SetField { object: collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    state.invalidate(*sym);
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                invalidate_block_writes(then_block, state);
                if let Some(eb) = else_block {
                    invalidate_block_writes(eb, state);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                invalidate_block_writes(body, state);
            }
            Stmt::Zone { body, .. } => {
                invalidate_block_writes(body, state);
            }
            _ => {}
        }
    }
}

use crate::ast::stmt::Block;

/// Entry point: run local CSE on a program.
pub fn cse_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mut state = CseState::new();
    cse_block(stmts, &mut state, expr_arena, stmt_arena, interner)
}
