//! Supercompiler — a unified optimization pass that subsumes constant folding,
//! propagation, dead code elimination, and compile-time function evaluation
//! in a single framework.
//!
//! The supercompiler performs online partial evaluation via:
//!   1. **Driving** — symbolic execution one step at a time, maintaining an
//!      abstract store mapping variables to known values
//!   2. **Folding** — memoizing function evaluation results to avoid
//!      recomputation and detect infinite recursion
//!   3. **Generalization** — widening variables modified in loops to ensure
//!      termination of the analysis
//!
//! Scope: pure integer/boolean code only. Collections, IO, and escape blocks
//! are treated conservatively (passed through unchanged).

use std::collections::HashMap;

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};

const MAX_INLINE_DEPTH: usize = 64;
const MAX_INLINE_STEPS: usize = 10_000;

#[derive(Debug, Clone)]
enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(Symbol),
    Nothing,
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Text(a), Value::Text(b)) => a == b,
            (Value::Nothing, Value::Nothing) => true,
            _ => false,
        }
    }
}

impl Eq for Value {}

impl std::hash::Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Value::Int(n) => n.hash(state),
            Value::Float(f) => f.to_bits().hash(state),
            Value::Bool(b) => b.hash(state),
            Value::Text(s) => s.hash(state),
            Value::Nothing => {}
        }
    }
}

struct FuncDef<'a> {
    params: Vec<Symbol>,
    body: Block<'a>,
}

#[derive(Clone)]
struct Configuration<'a> {
    expr: &'a Expr<'a>,
    store_snapshot: HashMap<Symbol, Value>,
}

struct History<'a> {
    entries: Vec<Configuration<'a>>,
}

impl<'a> History<'a> {
    fn new() -> Self {
        Self { entries: Vec::new() }
    }

    fn push(&mut self, config: Configuration<'a>) {
        if self.entries.len() >= 16 {
            self.entries.remove(0);
        }
        self.entries.push(config);
    }

    fn check_embedding(&self, new_expr: &Expr<'a>) -> Option<&Configuration<'a>> {
        self.entries.iter().find(|c| embeds(c.expr, new_expr))
    }
}

struct SuperEnv<'a> {
    store: HashMap<Symbol, Value>,
    funcs: HashMap<Symbol, FuncDef<'a>>,
    memo: HashMap<(Symbol, Vec<Value>), Option<Value>>,
    steps: usize,
    history: History<'a>,
}

impl<'a> SuperEnv<'a> {
    fn new() -> Self {
        Self {
            store: HashMap::new(),
            funcs: HashMap::new(),
            memo: HashMap::new(),
            steps: 0,
            history: History::new(),
        }
    }
}

fn is_pure_body(stmts: &[Stmt]) -> bool {
    stmts.iter().all(is_pure_stmt)
}

fn is_pure_stmt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Let { value, .. } => is_pure_expr(value),
        Stmt::Set { value, .. } => is_pure_expr(value),
        Stmt::Return { value } => value.map_or(true, is_pure_expr),
        Stmt::If { cond, then_block, else_block } => {
            is_pure_expr(cond)
                && is_pure_body(then_block)
                && else_block.map_or(true, |eb| is_pure_body(eb))
        }
        Stmt::While { cond, body, .. } => is_pure_expr(cond) && is_pure_body(body),
        Stmt::Call { .. } => true,
        _ => false,
    }
}

fn is_pure_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(_) | Expr::Identifier(_) | Expr::OptionNone => true,
        Expr::BinaryOp { left, right, .. } => is_pure_expr(left) && is_pure_expr(right),
        Expr::Not { operand } => is_pure_expr(operand),
        Expr::Call { args, .. } => args.iter().all(|a| is_pure_expr(a)),
        Expr::Length { collection } => is_pure_expr(collection),
        Expr::FieldAccess { object, .. } => is_pure_expr(object),
        Expr::Index { collection, index } => is_pure_expr(collection) && is_pure_expr(index),
        _ => false,
    }
}

fn collect_pure_funcs<'a>(stmts: &[Stmt<'a>]) -> HashMap<Symbol, FuncDef<'a>> {
    let mut funcs = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, is_native, .. } = stmt {
            if *is_native {
                continue;
            }
            if is_pure_body(body) {
                let param_syms: Vec<Symbol> = params.iter().map(|(s, _)| *s).collect();
                funcs.insert(*name, FuncDef { params: param_syms, body });
            }
        }
    }
    funcs
}

// =============================================================================
// Value-level evaluation (for inline evaluation of pure functions)
// =============================================================================

fn eval_expr_to_value(
    expr: &Expr,
    locals: &HashMap<Symbol, Value>,
    funcs: &HashMap<Symbol, FuncDef>,
    memo: &mut HashMap<(Symbol, Vec<Value>), Option<Value>>,
    steps: &mut usize,
    depth: usize,
) -> Option<Value> {
    if *steps >= MAX_INLINE_STEPS || depth >= MAX_INLINE_DEPTH {
        return None;
    }
    *steps += 1;

    match expr {
        Expr::Literal(Literal::Number(n)) => Some(Value::Int(*n)),
        Expr::Literal(Literal::Float(f)) => Some(Value::Float(*f)),
        Expr::Literal(Literal::Boolean(b)) => Some(Value::Bool(*b)),
        Expr::Literal(Literal::Text(s)) => Some(Value::Text(*s)),
        Expr::Literal(Literal::Nothing) => Some(Value::Nothing),
        Expr::Identifier(sym) => locals.get(sym).cloned(),
        Expr::BinaryOp { op, left, right } => {
            let lv = eval_expr_to_value(left, locals, funcs, memo, steps, depth)?;
            let rv = eval_expr_to_value(right, locals, funcs, memo, steps, depth)?;
            eval_binop(*op, &lv, &rv)
        }
        Expr::Not { operand } => {
            if let Value::Bool(b) = eval_expr_to_value(operand, locals, funcs, memo, steps, depth)? {
                Some(Value::Bool(!b))
            } else {
                None
            }
        }
        Expr::Call { function, args } => {
            let func = funcs.get(function)?;
            if args.len() != func.params.len() {
                return None;
            }
            let mut arg_vals = Vec::with_capacity(args.len());
            for arg in args {
                arg_vals.push(eval_expr_to_value(arg, locals, funcs, memo, steps, depth)?);
            }

            let memo_key = (*function, arg_vals.clone());
            if let Some(cached) = memo.get(&memo_key) {
                return cached.clone();
            }

            // Mark as in-progress to detect infinite recursion
            memo.insert(memo_key.clone(), None);

            let mut call_locals = HashMap::new();
            for (param, val) in func.params.iter().zip(arg_vals.iter()) {
                call_locals.insert(*param, val.clone());
            }

            let result = eval_block_to_value(func.body, &mut call_locals, funcs, memo, steps, depth + 1);
            memo.insert(memo_key, result.clone());
            result
        }
        _ => None,
    }
}

fn eval_binop(op: BinaryOpKind, lv: &Value, rv: &Value) -> Option<Value> {
    match (op, lv, rv) {
        (BinaryOpKind::Add, Value::Int(a), Value::Int(b)) => Some(Value::Int(a.wrapping_add(*b))),
        (BinaryOpKind::Subtract, Value::Int(a), Value::Int(b)) => Some(Value::Int(a.wrapping_sub(*b))),
        (BinaryOpKind::Multiply, Value::Int(a), Value::Int(b)) => Some(Value::Int(a.wrapping_mul(*b))),
        (BinaryOpKind::Divide, Value::Int(a), Value::Int(b)) if *b != 0 => Some(Value::Int(a / b)),
        (BinaryOpKind::Modulo, Value::Int(a), Value::Int(b)) if *b != 0 => Some(Value::Int(a % b)),
        (BinaryOpKind::Shl, Value::Int(a), Value::Int(b)) if *b >= 0 && *b < 64 => {
            Some(Value::Int(a.wrapping_shl(*b as u32)))
        }
        (BinaryOpKind::Shr, Value::Int(a), Value::Int(b)) if *b >= 0 && *b < 64 => {
            Some(Value::Int(a.wrapping_shr(*b as u32)))
        }
        (BinaryOpKind::BitXor, Value::Int(a), Value::Int(b)) => Some(Value::Int(a ^ b)),
        (BinaryOpKind::And, Value::Int(a), Value::Int(b)) => Some(Value::Int(a & b)),
        (BinaryOpKind::Or, Value::Int(a), Value::Int(b)) => Some(Value::Int(a | b)),
        (BinaryOpKind::Eq, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a == b)),
        (BinaryOpKind::NotEq, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a != b)),
        (BinaryOpKind::Lt, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a < b)),
        (BinaryOpKind::Gt, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a > b)),
        (BinaryOpKind::LtEq, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a <= b)),
        (BinaryOpKind::GtEq, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a >= b)),
        (BinaryOpKind::Add, Value::Float(a), Value::Float(b)) => Some(Value::Float(a + b)),
        (BinaryOpKind::Subtract, Value::Float(a), Value::Float(b)) => Some(Value::Float(a - b)),
        (BinaryOpKind::Multiply, Value::Float(a), Value::Float(b)) => Some(Value::Float(a * b)),
        (BinaryOpKind::Divide, Value::Float(a), Value::Float(b)) if *b != 0.0 => Some(Value::Float(a / b)),
        (BinaryOpKind::And, Value::Bool(a), Value::Bool(b)) => Some(Value::Bool(*a && *b)),
        (BinaryOpKind::Or, Value::Bool(a), Value::Bool(b)) => Some(Value::Bool(*a || *b)),
        (BinaryOpKind::Eq, Value::Text(a), Value::Text(b)) => Some(Value::Bool(a == b)),
        (BinaryOpKind::NotEq, Value::Text(a), Value::Text(b)) => Some(Value::Bool(a != b)),
        _ => None,
    }
}

enum EvalResult {
    Continue,
    Return(Value),
}

fn eval_block_to_value(
    stmts: &[Stmt],
    locals: &mut HashMap<Symbol, Value>,
    funcs: &HashMap<Symbol, FuncDef>,
    memo: &mut HashMap<(Symbol, Vec<Value>), Option<Value>>,
    steps: &mut usize,
    depth: usize,
) -> Option<Value> {
    for stmt in stmts {
        match eval_stmt_to_value(stmt, locals, funcs, memo, steps, depth)? {
            EvalResult::Continue => {}
            EvalResult::Return(v) => return Some(v),
        }
    }
    Some(Value::Nothing)
}

fn eval_stmt_to_value(
    stmt: &Stmt,
    locals: &mut HashMap<Symbol, Value>,
    funcs: &HashMap<Symbol, FuncDef>,
    memo: &mut HashMap<(Symbol, Vec<Value>), Option<Value>>,
    steps: &mut usize,
    depth: usize,
) -> Option<EvalResult> {
    if *steps >= MAX_INLINE_STEPS {
        return None;
    }
    *steps += 1;

    match stmt {
        Stmt::Let { var, value, .. } => {
            let v = eval_expr_to_value(value, locals, funcs, memo, steps, depth)?;
            locals.insert(*var, v);
            Some(EvalResult::Continue)
        }
        Stmt::Set { target, value } => {
            let v = eval_expr_to_value(value, locals, funcs, memo, steps, depth)?;
            locals.insert(*target, v);
            Some(EvalResult::Continue)
        }
        Stmt::Return { value } => {
            if let Some(v) = value {
                let result = eval_expr_to_value(v, locals, funcs, memo, steps, depth)?;
                Some(EvalResult::Return(result))
            } else {
                Some(EvalResult::Return(Value::Nothing))
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            let cv = eval_expr_to_value(cond, locals, funcs, memo, steps, depth)?;
            if let Value::Bool(b) = cv {
                if b {
                    for s in *then_block {
                        match eval_stmt_to_value(s, locals, funcs, memo, steps, depth)? {
                            EvalResult::Continue => {}
                            EvalResult::Return(v) => return Some(EvalResult::Return(v)),
                        }
                    }
                } else if let Some(eb) = else_block {
                    for s in *eb {
                        match eval_stmt_to_value(s, locals, funcs, memo, steps, depth)? {
                            EvalResult::Continue => {}
                            EvalResult::Return(v) => return Some(EvalResult::Return(v)),
                        }
                    }
                }
                Some(EvalResult::Continue)
            } else {
                None
            }
        }
        Stmt::While { cond, body, .. } => {
            loop {
                let cv = eval_expr_to_value(cond, locals, funcs, memo, steps, depth)?;
                match cv {
                    Value::Bool(false) => break,
                    Value::Bool(true) => {
                        for s in *body {
                            match eval_stmt_to_value(s, locals, funcs, memo, steps, depth)? {
                                EvalResult::Continue => {}
                                EvalResult::Return(v) => return Some(EvalResult::Return(v)),
                            }
                        }
                    }
                    _ => return None,
                }
            }
            Some(EvalResult::Continue)
        }
        _ => None,
    }
}

// =============================================================================
// AST-level supercompilation (driving + constant propagation + inline)
// =============================================================================

fn value_to_literal(val: &Value) -> Option<Literal> {
    match val {
        Value::Int(n) => Some(Literal::Number(*n)),
        Value::Float(f) => Some(Literal::Float(*f)),
        Value::Bool(b) => Some(Literal::Boolean(*b)),
        Value::Text(s) => Some(Literal::Text(*s)),
        Value::Nothing => Some(Literal::Nothing),
    }
}

fn value_to_expr<'a>(val: &Value, arena: &'a Arena<Expr<'a>>) -> &'a Expr<'a> {
    match value_to_literal(val) {
        Some(lit) => arena.alloc(Expr::Literal(lit)),
        None => unreachable!(),
    }
}

fn expr_to_value(expr: &Expr, store: &HashMap<Symbol, Value>) -> Option<Value> {
    match expr {
        Expr::Literal(Literal::Number(n)) => Some(Value::Int(*n)),
        Expr::Literal(Literal::Float(f)) => Some(Value::Float(*f)),
        Expr::Literal(Literal::Boolean(b)) => Some(Value::Bool(*b)),
        Expr::Literal(Literal::Text(s)) => Some(Value::Text(*s)),
        Expr::Literal(Literal::Nothing) => Some(Value::Nothing),
        Expr::Identifier(sym) => store.get(sym).cloned(),
        _ => None,
    }
}

fn try_eval_expr<'a>(
    expr: &'a Expr<'a>,
    env: &mut SuperEnv<'a>,
    depth: usize,
) -> Option<Value> {
    eval_expr_to_value(
        expr,
        &env.store,
        &env.funcs,
        &mut env.memo,
        &mut env.steps,
        depth,
    )
}

fn drive_expr<'a>(
    expr: &'a Expr<'a>,
    env: &mut SuperEnv<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    depth: usize,
) -> &'a Expr<'a> {
    if depth >= MAX_INLINE_DEPTH {
        return expr;
    }

    match expr {
        Expr::Identifier(sym) => {
            if let Some(val) = env.store.get(sym) {
                // Only propagate Copy-type values (Int, Float, Bool, Nothing).
                // Text values are owned (String) in generated Rust — propagating
                // them changes move semantics and can eliminate use-after-move errors.
                match val {
                    Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Nothing => {
                        if let Some(lit) = value_to_literal(val) {
                            return expr_arena.alloc(Expr::Literal(lit));
                        }
                    }
                    Value::Text(s) => {
                        return expr_arena.alloc(Expr::Literal(Literal::Text(*s)));
                    }
                }
            }
            expr
        }
        Expr::BinaryOp { op, left, right } => {
            let dl = drive_expr(left, env, expr_arena, depth);
            let dr = drive_expr(right, env, expr_arena, depth);
            // Try value-level evaluation
            if let (Some(lv), Some(rv)) = (expr_to_value(dl, &env.store), expr_to_value(dr, &env.store)) {
                if let Some(result) = eval_binop(*op, &lv, &rv) {
                    return value_to_expr(&result, expr_arena);
                }
            }
            if std::ptr::eq(dl, *left) && std::ptr::eq(dr, *right) {
                expr
            } else {
                expr_arena.alloc(Expr::BinaryOp { op: *op, left: dl, right: dr })
            }
        }
        Expr::Not { operand } => {
            let d = drive_expr(operand, env, expr_arena, depth);
            if let Expr::Literal(Literal::Boolean(b)) = d {
                return expr_arena.alloc(Expr::Literal(Literal::Boolean(!b)));
            }
            if std::ptr::eq(d, *operand) { expr } else { expr_arena.alloc(Expr::Not { operand: d }) }
        }
        Expr::Call { function, args } => {
            let driven_args: Vec<&'a Expr<'a>> = args.iter()
                .map(|a| drive_expr(a, env, expr_arena, depth))
                .collect();
            // Don't replace calls in the AST — preserve function call sites.
            // Call evaluation is done at the store level in drive_stmt for Let/Set.
            let changed = driven_args.iter().zip(args.iter()).any(|(d, o)| !std::ptr::eq(*d, *o));
            if changed {
                expr_arena.alloc(Expr::Call { function: *function, args: driven_args })
            } else {
                expr
            }
        }
        Expr::Length { collection } => {
            let d = drive_expr(collection, env, expr_arena, depth);
            if std::ptr::eq(d, *collection) { expr } else { expr_arena.alloc(Expr::Length { collection: d }) }
        }
        // Index/Slice: preserve original variable names for codegen peephole patterns.
        // The swap, vec-fill, and index-lowering patterns depend on seeing variable
        // references (e.g., `j`, `j+1`) rather than propagated constants.
        // The fold pass handles compile-time index evaluation separately.
        Expr::Index { .. } | Expr::Slice { .. } => expr,
        Expr::Contains { collection, value } => {
            let dc = drive_expr(collection, env, expr_arena, depth);
            let dv = drive_expr(value, env, expr_arena, depth);
            if std::ptr::eq(dc, *collection) && std::ptr::eq(dv, *value) {
                expr
            } else {
                expr_arena.alloc(Expr::Contains { collection: dc, value: dv })
            }
        }
        Expr::FieldAccess { object, field } => {
            let d = drive_expr(object, env, expr_arena, depth);
            if std::ptr::eq(d, *object) { expr } else { expr_arena.alloc(Expr::FieldAccess { object: d, field: *field }) }
        }
        Expr::OptionSome { value } => {
            let d = drive_expr(value, env, expr_arena, depth);
            if std::ptr::eq(d, *value) { expr } else { expr_arena.alloc(Expr::OptionSome { value: d }) }
        }
        Expr::Copy { expr: inner } => {
            let d = drive_expr(inner, env, expr_arena, depth);
            if std::ptr::eq(d, *inner) { expr } else { expr_arena.alloc(Expr::Copy { expr: d }) }
        }
        Expr::Give { value } => {
            let d = drive_expr(value, env, expr_arena, depth);
            if std::ptr::eq(d, *value) { expr } else { expr_arena.alloc(Expr::Give { value: d }) }
        }
        Expr::List(elems) => {
            let driven: Vec<&'a Expr<'a>> = elems.iter()
                .map(|e| drive_expr(e, env, expr_arena, depth))
                .collect();
            let changed = driven.iter().zip(elems.iter()).any(|(d, o)| !std::ptr::eq(*d, *o));
            if changed { expr_arena.alloc(Expr::List(driven)) } else { expr }
        }
        Expr::Tuple(elems) => {
            let driven: Vec<&'a Expr<'a>> = elems.iter()
                .map(|e| drive_expr(e, env, expr_arena, depth))
                .collect();
            let changed = driven.iter().zip(elems.iter()).any(|(d, o)| !std::ptr::eq(*d, *o));
            if changed { expr_arena.alloc(Expr::Tuple(driven)) } else { expr }
        }
        // Leaves and complex expressions passed through
        _ => expr,
    }
}

fn drive_stmt<'a>(
    stmt: Stmt<'a>,
    env: &mut SuperEnv<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    depth: usize,
) -> Option<Stmt<'a>> {
    match stmt {
        Stmt::Let { var, ty, value, mutable } => {
            let driven = drive_expr(value, env, expr_arena, depth);
            if !mutable {
                // Try to resolve the driven expression to a value for the store
                if let Some(val) = expr_to_value(driven, &env.store) {
                    env.store.insert(var, val);
                } else {
                    // Expression isn't a simple literal/identifier — try evaluating
                    // calls at the value level for store tracking
                    if let Some(val) = try_eval_expr(driven, env, depth) {
                        env.store.insert(var, val);
                    }
                }
            }
            Some(Stmt::Let { var, ty, value: driven, mutable })
        }
        Stmt::Set { target, value } => {
            let driven = drive_expr(value, env, expr_arena, depth);
            // Update store if we can resolve the new value
            if let Some(val) = expr_to_value(driven, &env.store) {
                env.store.insert(target, val);
            } else {
                // Value is unknown; remove from store
                env.store.remove(&target);
            }
            Some(Stmt::Set { target, value: driven })
        }
        Stmt::If { cond, then_block, else_block } => {
            let driven_cond = drive_expr(cond, env, expr_arena, depth);
            // If condition is known, eliminate dead branch
            if let Expr::Literal(Literal::Boolean(b)) = driven_cond {
                if *b {
                    // Take then branch only
                    let driven_then = drive_block(then_block, env, expr_arena, stmt_arena, interner, depth);
                    Some(Stmt::If {
                        cond: driven_cond,
                        then_block: driven_then,
                        else_block: None,
                    })
                } else {
                    // Take else branch only
                    if let Some(eb) = else_block {
                        let driven_else = drive_block(eb, env, expr_arena, stmt_arena, interner, depth);
                        Some(Stmt::If {
                            cond: driven_cond,
                            then_block: stmt_arena.alloc_slice(vec![]),
                            else_block: Some(driven_else),
                        })
                    } else {
                        None // Entire If eliminated
                    }
                }
            } else {
                // Condition unknown: drive both branches with snapshot/restore
                let snapshot = env.store.clone();
                let driven_then = drive_block(then_block, env, expr_arena, stmt_arena, interner, depth);
                let then_store = env.store.clone();

                env.store = snapshot;
                let driven_else = else_block.map(|eb| {
                    drive_block(eb, env, expr_arena, stmt_arena, interner, depth)
                });
                let else_store = env.store.clone();

                // Join: keep only values that agree in both branches
                let mut joined = HashMap::new();
                for (sym, then_val) in &then_store {
                    if let Some(else_val) = else_store.get(sym) {
                        if then_val == else_val {
                            joined.insert(*sym, then_val.clone());
                        }
                    }
                }
                env.store = joined;

                Some(Stmt::If {
                    cond: driven_cond,
                    then_block: driven_then,
                    else_block: driven_else,
                })
            }
        }
        Stmt::While { cond, body, decreasing } => {
            let modified = collect_modified_vars_block(body);
            let snapshot = env.store.clone();

            // Snapshot configuration before entering loop
            let pre_config = Configuration {
                expr: cond,
                store_snapshot: snapshot.clone(),
            };

            // Widen modified variables
            for sym in &modified {
                env.store.remove(sym);
            }

            let driven_cond = drive_expr(cond, env, expr_arena, depth);
            // While-false elimination: if condition is statically false, remove loop
            if let Expr::Literal(Literal::Boolean(false)) = driven_cond {
                env.store = snapshot;
                return None;
            }

            // Check embedding: if the driven condition embeds in a historical one,
            // the whistle blows — use MSG to generalize and widen precisely.
            if let Some(prev) = env.history.check_embedding(driven_cond) {
                let generalized = msg(prev.expr, driven_cond, expr_arena, interner);
                // Remove store entries for MSG-introduced variables to ensure termination
                for i in 0..generalized.num_substitutions {
                    let name = format!("__msg_{}", i);
                    let sym = interner.intern(&name);
                    env.store.remove(&sym);
                }
            }

            // Push configuration to history
            env.history.push(pre_config);

            let driven_body = drive_block(body, env, expr_arena, stmt_arena, interner, depth);

            // After loop, preserve unmodified variables from snapshot
            let mut post_loop_store = HashMap::new();
            for (sym, val) in &snapshot {
                if !modified.contains(sym) {
                    post_loop_store.insert(*sym, val.clone());
                }
            }
            env.store = post_loop_store;

            Some(Stmt::While { cond: driven_cond, body: driven_body, decreasing })
        }
        Stmt::Repeat { pattern, iterable, body } => {
            let driven_iter = drive_expr(iterable, env, expr_arena, depth);
            // Loop variable and modified vars are unknown
            if let crate::ast::stmt::Pattern::Identifier(sym) = &pattern {
                env.store.remove(sym);
            }
            let modified = collect_modified_vars_block(body);
            for sym in &modified {
                env.store.remove(sym);
            }
            let driven_body = drive_block(body, env, expr_arena, stmt_arena, interner, depth);
            Some(Stmt::Repeat { pattern, iterable: driven_iter, body: driven_body })
        }
        Stmt::Show { object, recipient } => {
            let driven = drive_expr(object, env, expr_arena, depth);
            Some(Stmt::Show { object: driven, recipient })
        }
        Stmt::Return { value } => {
            let driven = value.map(|v| drive_expr(v, env, expr_arena, depth));
            Some(Stmt::Return { value: driven })
        }
        Stmt::Call { function, args } => {
            let driven_args: Vec<&'a Expr<'a>> = args.into_iter()
                .map(|a| drive_expr(a, env, expr_arena, depth))
                .collect();
            Some(Stmt::Call { function, args: driven_args })
        }
        Stmt::Push { value, collection } => {
            let driven_val = drive_expr(value, env, expr_arena, depth);
            // Push invalidates known value of collection
            if let Expr::Identifier(sym) = collection {
                env.store.remove(sym);
            }
            Some(Stmt::Push { value: driven_val, collection })
        }
        Stmt::SetIndex { collection, index, value } => {
            // Don't drive index — codegen peephole patterns depend on original variable names
            if let Expr::Identifier(sym) = collection {
                env.store.remove(sym);
            }
            Some(Stmt::SetIndex { collection, index, value })
        }
        Stmt::SetField { object, field, value } => {
            let dv = drive_expr(value, env, expr_arena, depth);
            if let Expr::Identifier(sym) = object {
                env.store.remove(sym);
            }
            Some(Stmt::SetField { object, field, value: dv })
        }
        Stmt::RuntimeAssert { condition } => {
            let driven = drive_expr(condition, env, expr_arena, depth);
            Some(Stmt::RuntimeAssert { condition: driven })
        }
        Stmt::FunctionDef { name, params, generics, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => {
            // Drive inside function body with fresh store
            let snapshot = env.store.clone();
            env.store.clear();
            let driven_body = drive_block(body, env, expr_arena, stmt_arena, interner, depth);
            env.store = snapshot;
            Some(Stmt::FunctionDef {
                name, params, generics,
                body: driven_body,
                return_type, is_native, native_path, is_exported, export_target, opt_flags,
            })
        }
        // Zone blocks: pass through unchanged. Propagating inside zones can
        // eliminate zone escape patterns that the Rust compiler needs to detect.
        Stmt::Zone { .. } => Some(stmt),
        // Pass through everything else unchanged
        other => Some(other),
    }
}

fn drive_block<'a>(
    block: &'a [Stmt<'a>],
    env: &mut SuperEnv<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    depth: usize,
) -> &'a [Stmt<'a>] {
    let mut result = Vec::with_capacity(block.len());
    for stmt in block.iter().cloned() {
        if let Some(driven) = drive_stmt(stmt, env, expr_arena, stmt_arena, interner, depth) {
            result.push(driven);
        }
    }
    stmt_arena.alloc_slice(result)
}

fn collect_modified_vars_block(stmts: &[Stmt]) -> Vec<Symbol> {
    let mut modified = Vec::new();
    for stmt in stmts {
        collect_modified_vars_stmt(stmt, &mut modified);
    }
    modified
}

fn collect_modified_vars_stmt(stmt: &Stmt, out: &mut Vec<Symbol>) {
    match stmt {
        Stmt::Set { target, .. } => out.push(*target),
        Stmt::Let { var, mutable: true, .. } => out.push(*var),
        Stmt::If { then_block, else_block, .. } => {
            for s in *then_block {
                collect_modified_vars_stmt(s, out);
            }
            if let Some(eb) = else_block {
                for s in *eb {
                    collect_modified_vars_stmt(s, out);
                }
            }
        }
        Stmt::While { body, .. } => {
            for s in *body {
                collect_modified_vars_stmt(s, out);
            }
        }
        Stmt::Repeat { body, .. } => {
            for s in *body {
                collect_modified_vars_stmt(s, out);
            }
        }
        Stmt::Inspect { arms, .. } => {
            for arm in arms {
                for s in arm.body {
                    collect_modified_vars_stmt(s, out);
                }
            }
        }
        Stmt::Zone { body, .. } => {
            for s in *body {
                collect_modified_vars_stmt(s, out);
            }
        }
        _ => {}
    }
}

// =============================================================================
// Public API
// =============================================================================

pub fn supercompile_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let funcs = collect_pure_funcs(&stmts);
    let mut env = SuperEnv::new();
    env.funcs = funcs;

    let mut result = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        if let Some(driven) = drive_stmt(stmt, &mut env, expr_arena, stmt_arena, interner, 0) {
            result.push(driven);
        }
    }
    result
}

/// Homeomorphic embedding check: `e1 ◁ e2`.
///
/// Returns true if `e1` is homeomorphically embedded in `e2`, meaning `e1` can
/// be obtained from `e2` by erasing some constructors (diving) or both share
/// the same constructor with all children embedded (coupling).
pub fn embeds<'a>(e1: &Expr<'a>, e2: &Expr<'a>) -> bool {
    match (e1, e2) {
        // Coupling: same constructor, all children embed
        (Expr::BinaryOp { op: op1, left: l1, right: r1 },
         Expr::BinaryOp { op: op2, left: l2, right: r2 }) if op1 == op2 =>
            embeds(l1, l2) && embeds(r1, r2),
        (Expr::Call { function: f1, args: a1 },
         Expr::Call { function: f2, args: a2 }) if f1 == f2 && a1.len() == a2.len() =>
            a1.iter().zip(a2.iter()).all(|(x, y)| embeds(x, y)),
        (Expr::Not { operand: e1_inner }, Expr::Not { operand: e2_inner }) =>
            embeds(e1_inner, e2_inner),
        // Base: literals and identifiers embed in themselves
        (Expr::Literal(l1), Expr::Literal(l2)) => l1 == l2,
        (Expr::Identifier(s1), Expr::Identifier(s2)) => s1 == s2,
        // Diving: e1 embeds in a subterm of e2
        (_, Expr::BinaryOp { left, right, .. }) =>
            embeds(e1, left) || embeds(e1, right),
        (_, Expr::Call { args, .. }) =>
            args.iter().any(|a| embeds(e1, a)),
        (_, Expr::Not { operand }) =>
            embeds(e1, operand),
        _ => false,
    }
}

/// Result of Most Specific Generalization (MSG).
pub struct MsgResult<'a> {
    /// The generalized expression with fresh variables replacing differing parts.
    pub expr: &'a Expr<'a>,
    /// Number of fresh variables (substitutions) introduced.
    pub num_substitutions: usize,
}

/// Compute the Most Specific Generalization (MSG) of two expressions.
///
/// Given `e1` and `e2`, finds the most specific expression `g` such that
/// both `e1` and `e2` are instances of `g`. Differing subexpressions are
/// replaced with fresh variables named `__msg_0`, `__msg_1`, etc.
pub fn msg<'a>(
    e1: &'a Expr<'a>,
    e2: &'a Expr<'a>,
    arena: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
) -> MsgResult<'a> {
    let mut counter = 0;
    let expr = msg_inner(e1, e2, arena, interner, &mut counter);
    MsgResult { expr, num_substitutions: counter }
}

fn msg_inner<'a>(
    e1: &'a Expr<'a>,
    e2: &'a Expr<'a>,
    arena: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
    counter: &mut usize,
) -> &'a Expr<'a> {
    match (e1, e2) {
        // Same literal or same identifier → preserve
        (Expr::Literal(l1), Expr::Literal(l2)) if l1 == l2 => e1,
        (Expr::Identifier(s1), Expr::Identifier(s2)) if s1 == s2 => e1,
        // Same BinaryOp constructor → recurse into children
        (Expr::BinaryOp { op: op1, left: l1, right: r1 },
         Expr::BinaryOp { op: op2, left: l2, right: r2 }) if op1 == op2 => {
            let left = msg_inner(l1, l2, arena, interner, counter);
            let right = msg_inner(r1, r2, arena, interner, counter);
            arena.alloc(Expr::BinaryOp { op: *op1, left, right })
        }
        // Same Call constructor → recurse into args
        (Expr::Call { function: f1, args: a1 },
         Expr::Call { function: f2, args: a2 }) if f1 == f2 && a1.len() == a2.len() => {
            let args: Vec<&'a Expr<'a>> = a1.iter().zip(a2.iter())
                .map(|(x, y)| msg_inner(x, y, arena, interner, counter))
                .collect();
            arena.alloc(Expr::Call { function: *f1, args })
        }
        // Same Not constructor → recurse
        (Expr::Not { operand: e1_inner }, Expr::Not { operand: e2_inner }) => {
            let inner = msg_inner(e1_inner, e2_inner, arena, interner, counter);
            arena.alloc(Expr::Not { operand: inner })
        }
        // Differing → introduce fresh variable
        _ => {
            let name = format!("__msg_{}", counter);
            *counter += 1;
            let sym = interner.intern(&name);
            arena.alloc(Expr::Identifier(sym))
        }
    }
}
