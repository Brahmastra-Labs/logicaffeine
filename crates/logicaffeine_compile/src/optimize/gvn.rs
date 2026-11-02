//! Common Subexpression Elimination via Value Numbering, DOMINATOR-SCOPED.
//!
//! Identifies identical subexpressions and replaces redundant computations with
//! references to previously computed values. The value table is NOT reset at
//! control-flow boundaries: it threads INTO an `If`'s arms and a loop's body (so
//! a subexpression computed before a branch is reused inside it), and after the
//! region the parent table is restored with the region's writes invalidated (so
//! a value computed before a loop the loop does not write survives past it).
//! Every reuse stays RELOAD-CORRECT: any read whose dependency symbol is written
//! — including an `item k of arr` across a write to `arr` — is invalidated, so a
//! stale value is never forwarded. What a statement writes comes from the one
//! shared answer, [`super::writes`]: a `Set` in an `Inspect` arm, a `Swap`, a push
//! through a field, a call to a function that assigns a global — every one of
//! them invalidates here. A loop body is LOOP-CARRIED: whatever it may write is
//! unavailable at the top of every iteration, the first included, so the body is
//! processed against the pre-loop table MINUS its own writes. `gvn_scope_lock`
//! pins the cross-boundary reuse; `cse_loop_soundness_lock` and
//! `optimizer_write_effects_lock` pin the reload-correctness.

use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};

use super::writes::{CalleeGlobals, WriteSet};

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
        ExprKey::Ident(sym) => {
            out.insert(*sym);
        }
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
    /// Maps each holder symbol to the cache keys whose value it holds — reassigning the holder
    /// kills those entries too (the value is gone even though the key's inputs are intact).
    held: HashMap<Symbol, Vec<ExprKey>>,
    /// Counter for generating unique temp variable names.
    counter: u32,
    /// What any call in this program may write.
    callee_globals: CalleeGlobals,
}

/// The availability table at one point, restored after a region whose effects were invalidated.
struct CseSnapshot {
    cache: HashMap<ExprKey, Symbol>,
    deps: HashMap<Symbol, Vec<ExprKey>>,
    held: HashMap<Symbol, Vec<ExprKey>>,
}

impl CseState {
    fn new(callee_globals: CalleeGlobals) -> Self {
        Self {
            cache: HashMap::new(),
            deps: HashMap::new(),
            held: HashMap::new(),
            counter: 0,
            callee_globals,
        }
    }

    /// Add an expression to the cache, associated with the given symbol.
    fn insert(&mut self, key: ExprKey, sym: Symbol) {
        let mut syms = HashSet::new();
        collect_key_symbols(&key, &mut syms);
        for s in syms {
            self.deps.entry(s).or_default().push(key.clone());
        }
        self.held.entry(sym).or_default().push(key.clone());
        self.cache.insert(key, sym);
    }

    /// Invalidate every cache entry that reads the given symbol, or whose value it holds.
    fn invalidate(&mut self, sym: Symbol) {
        if let Some(keys) = self.deps.remove(&sym) {
            for key in keys {
                self.cache.remove(&key);
            }
        }
        if let Some(keys) = self.held.remove(&sym) {
            for key in keys {
                self.cache.remove(&key);
            }
        }
    }

    /// Forget everything: foreign code ran and may have written any name.
    fn clear(&mut self) {
        self.cache.clear();
        self.deps.clear();
        self.held.clear();
    }

    /// What `stmt` may write, with the program's callee-written globals folded in when it calls.
    fn effects_of(&self, stmt: &Stmt) -> WriteSet {
        WriteSet::of_stmt(stmt).with_callee_globals(&self.callee_globals)
    }

    /// What EVALUATING `expr` may write (through the calls it makes).
    fn effects_of_expr(&self, expr: &Expr) -> WriteSet {
        WriteSet::of_expr(expr).with_callee_globals(&self.callee_globals)
    }

    /// Drop every entry a statement with these effects may have made stale.
    fn invalidate_effects(&mut self, effects: &WriteSet) {
        if effects.is_opaque() {
            self.clear();
            return;
        }
        for sym in effects.symbols() {
            self.invalidate(sym);
        }
    }

    fn snapshot(&self) -> CseSnapshot {
        CseSnapshot {
            cache: self.cache.clone(),
            deps: self.deps.clone(),
            held: self.held.clone(),
        }
    }

    fn restore(&mut self, snapshot: CseSnapshot) {
        self.cache = snapshot.cache;
        self.deps = snapshot.deps;
        self.held = snapshot.held;
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
        Expr::Length { collection }
        | Expr::Not {
            operand: collection,
        } => {
            count_subexpressions(collection, counts);
        }
        Expr::FieldAccess { object, .. } => {
            count_subexpressions(object, counts);
        }
        Expr::Index { collection, index }
        | Expr::Contains {
            collection,
            value: index,
        } => {
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
            let new_left = rewrite_expr(
                left,
                intra_counts,
                local_cache,
                state,
                hoisted,
                interner,
                expr_arena,
                stmt_arena,
            );
            let new_right = rewrite_expr(
                right,
                intra_counts,
                local_cache,
                state,
                hoisted,
                interner,
                expr_arena,
                stmt_arena,
            );
            if std::ptr::eq(*left, new_left) && std::ptr::eq(*right, new_right) {
                expr
            } else {
                expr_arena.alloc(Expr::BinaryOp {
                    op: *op,
                    left: new_left,
                    right: new_right,
                })
            }
        }
        Expr::Length { collection } => {
            let new_coll = rewrite_expr(
                collection,
                intra_counts,
                local_cache,
                state,
                hoisted,
                interner,
                expr_arena,
                stmt_arena,
            );
            if std::ptr::eq(*collection, new_coll) {
                expr
            } else {
                expr_arena.alloc(Expr::Length {
                    collection: new_coll,
                })
            }
        }
        Expr::Not { operand } => {
            let new_op = rewrite_expr(
                operand,
                intra_counts,
                local_cache,
                state,
                hoisted,
                interner,
                expr_arena,
                stmt_arena,
            );
            if std::ptr::eq(*operand, new_op) {
                expr
            } else {
                expr_arena.alloc(Expr::Not { operand: new_op })
            }
        }
        Expr::FieldAccess { object, field } => {
            let new_obj = rewrite_expr(
                object,
                intra_counts,
                local_cache,
                state,
                hoisted,
                interner,
                expr_arena,
                stmt_arena,
            );
            if std::ptr::eq(*object, new_obj) {
                expr
            } else {
                expr_arena.alloc(Expr::FieldAccess {
                    object: new_obj,
                    field: *field,
                })
            }
        }
        Expr::Index { collection, index } => {
            let new_coll = rewrite_expr(
                collection,
                intra_counts,
                local_cache,
                state,
                hoisted,
                interner,
                expr_arena,
                stmt_arena,
            );
            let new_idx = rewrite_expr(
                index,
                intra_counts,
                local_cache,
                state,
                hoisted,
                interner,
                expr_arena,
                stmt_arena,
            );
            if std::ptr::eq(*collection, new_coll) && std::ptr::eq(*index, new_idx) {
                expr
            } else {
                expr_arena.alloc(Expr::Index {
                    collection: new_coll,
                    index: new_idx,
                })
            }
        }
        Expr::Contains { collection, value } => {
            let new_coll = rewrite_expr(
                collection,
                intra_counts,
                local_cache,
                state,
                hoisted,
                interner,
                expr_arena,
                stmt_arena,
            );
            let new_val = rewrite_expr(
                value,
                intra_counts,
                local_cache,
                state,
                hoisted,
                interner,
                expr_arena,
                stmt_arena,
            );
            if std::ptr::eq(*collection, new_coll) && std::ptr::eq(*value, new_val) {
                expr
            } else {
                expr_arena.alloc(Expr::Contains {
                    collection: new_coll,
                    value: new_val,
                })
            }
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
        // Everything this statement may write: its own binds and in-place mutations, whatever its
        // nested blocks write, and every global a call it makes may change.
        let effects = state.effects_of(&stmt);
        match stmt {
            Stmt::Let {
                var,
                ty,
                value,
                mutable,
            } => {
                // Count intra-expression subexpression occurrences
                let mut intra_counts = HashMap::new();
                count_subexpressions(value, &mut intra_counts);

                let has_intra_dupes = intra_counts.values().any(|&c| c > 1);

                let new_value = if has_intra_dupes {
                    // Full rewrite with potential hoisting
                    let mut hoisted = Vec::new();
                    let mut local_cache = HashMap::new();
                    let new_value = rewrite_expr(
                        value,
                        &intra_counts,
                        &mut local_cache,
                        state,
                        &mut hoisted,
                        interner,
                        expr_arena,
                        stmt_arena,
                    );
                    result.extend(hoisted);
                    new_value
                } else {
                    cse_expr_cross_stmt(value, state, expr_arena)
                };
                // The value is computed (and any call in it has run) before `var` takes it: entries a
                // call made stale, and entries the OLD `var` held, die before the new one is recorded.
                state.invalidate_effects(&effects);
                if let Some(key) = compute_key(new_value) {
                    if is_non_trivial(new_value) {
                        state.insert(key, var);
                    }
                }
                result.push(Stmt::Let {
                    var,
                    ty,
                    value: new_value,
                    mutable,
                });
            }
            // Control flow: each arm is processed from the table as it stands after the condition;
            // afterwards the table is restored with everything either arm may have written invalidated.
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let cond_effects = state.effects_of_expr(cond);
                state.invalidate_effects(&cond_effects);
                let before = state.snapshot();

                let new_then =
                    cse_block(then_block.to_vec(), state, expr_arena, stmt_arena, interner);
                let new_then_block: Block = stmt_arena.alloc_slice(new_then);

                state.restore(before);
                let before = state.snapshot();

                let new_else = else_block.map(|eb| {
                    let processed = cse_block(eb.to_vec(), state, expr_arena, stmt_arena, interner);
                    let b: Block = stmt_arena.alloc_slice(processed);
                    b
                });

                state.restore(before);
                state.invalidate_effects(&effects);

                result.push(Stmt::If {
                    cond,
                    then_block: new_then_block,
                    else_block: new_else,
                });
            }
            // Loops are LOOP-CARRIED: a value the body may write is unavailable at the top of every
            // iteration, the first included. Invalidate the body's effects first, process the body
            // against that table, then return to it (the body's own entries do not outlive the loop).
            Stmt::While {
                cond,
                body,
                decreasing,
            } => {
                state.invalidate_effects(&effects);
                let before = state.snapshot();

                let new_body = cse_block(body.to_vec(), state, expr_arena, stmt_arena, interner);
                let new_body_block: Block = stmt_arena.alloc_slice(new_body);

                state.restore(before);
                result.push(Stmt::While {
                    cond,
                    body: new_body_block,
                    decreasing,
                });
            }
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => {
                state.invalidate_effects(&effects);
                let before = state.snapshot();

                let new_body = cse_block(body.to_vec(), state, expr_arena, stmt_arena, interner);
                let new_body_block: Block = stmt_arena.alloc_slice(new_body);

                state.restore(before);
                result.push(Stmt::Repeat {
                    pattern,
                    iterable,
                    body: new_body_block,
                });
            }
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
            } => {
                // Process function body with fresh state
                let mut fn_state = CseState::new(state.callee_globals.clone());
                fn_state.counter = state.counter;
                let new_body = cse_block(
                    body.to_vec(),
                    &mut fn_state,
                    expr_arena,
                    stmt_arena,
                    interner,
                );
                state.counter = fn_state.counter;
                let new_body_block: Block = stmt_arena.alloc_slice(new_body);
                result.push(Stmt::FunctionDef {
                    name,
                    generics,
                    params,
                    body: new_body_block,
                    return_type,
                    is_native,
                    native_path,
                    is_exported,
                    export_target,
                    opt_flags,
                });
            }
            // Every other statement passes through unchanged; whatever it may write is gone from the table.
            other => {
                state.invalidate_effects(&effects);
                result.push(other);
            }
        }
    }

    result
}

/// Entry point: run local CSE on a program.
pub fn cse_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mut state = CseState::new(CalleeGlobals::of_program(&stmts));
    cse_block(stmts, &mut state, expr_arena, stmt_arena, interner)
}
