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

use super::capture::{self, EntryBinders};
use super::writes::{CalleeGlobals, WriteSet};
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
    /// A statically-known array (a `Seq`/`List` built from literals under constant-trip loops). Its presence
    /// in the store lets a matmul/reduction over it be *executed at compile time* — the loops and the array
    /// itself vanish, leaving only the result constants. A `Seq` value never propagates as a literal (only its
    /// elements do, via `Index` folding).
    Seq(Vec<Value>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Text(a), Value::Text(b)) => a == b,
            (Value::Nothing, Value::Nothing) => true,
            (Value::Seq(a), Value::Seq(b)) => a == b,
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
            Value::Seq(v) => v.hash(state),
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
}

struct History<'a> {
    entries: Vec<Configuration<'a>>,
}

impl<'a> History<'a> {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
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
    /// Whether ARRAY supercompilation (Value::Seq tracking, Index folding, concrete static-loop execution) is
    /// enabled for THIS program. Off unless the whole program is in the pure static-array fragment
    /// ([`is_pure_static_array_program`]) — so a program that Pops/slices/aliases an array, calls a native or
    /// user function, or takes runtime input is driven EXACTLY as the original supercompiler (no perturbation,
    /// no unsound stale-array fold). Confines the tensor fold to the fragment it soundly models.
    array_fold: bool,
    /// Globals that some function mutates (writes without declaring locally). A `Call` can change any of these
    /// under the driver, so they are dropped from `store` at every call site — otherwise a global written by a
    /// callee would be folded to its stale pre-call value (the `Set counter` in `bump` soundness bug). Opaque
    /// (a callee runs foreign code) means a call may write anything: the whole store is dropped.
    callee_globals: CalleeGlobals,
}

impl<'a> SuperEnv<'a> {
    fn new() -> Self {
        Self {
            store: HashMap::new(),
            funcs: HashMap::new(),
            memo: HashMap::new(),
            steps: 0,
            history: History::new(),
            array_fold: false,
            callee_globals: CalleeGlobals::default(),
        }
    }

    /// Everything a statement or block may write ([`WriteSet`]) — its binders, assignments, in-place
    /// mutations, what it lends, and the callee-written globals when it calls anything.
    fn writes_of(&self, written: WriteSet) -> WriteSet {
        written.with_callee_globals(&self.callee_globals)
    }
}

/// Drop from `store` every name `written` may change — all of them when foreign code ran.
fn forget(store: &mut HashMap<Symbol, Value>, written: &WriteSet) {
    if written.is_opaque() {
        store.clear();
    } else {
        for sym in written.symbols() {
            store.remove(&sym);
        }
    }
}

/// Whether this evaluator can run a body: only the statement and expression shapes it executes. Whether running it
/// is sound is [`super::purity::Purity`]'s question, which every caller asks too. `pub(crate)` so the run-path
/// memoizer ([`super::memoize_run`]) transforms only the shapes this evaluator agrees on.
pub(crate) fn evaluable_body(stmts: &[Stmt]) -> bool {
    stmts.iter().all(is_pure_stmt)
}

fn is_pure_stmt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Let { value, .. } => is_pure_expr(value),
        Stmt::Set { value, .. } => is_pure_expr(value),
        Stmt::Return { value } => value.map_or(true, is_pure_expr),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            is_pure_expr(cond)
                && evaluable_body(then_block)
                && else_block.map_or(true, |eb| evaluable_body(eb))
        }
        Stmt::While { cond, body, .. } => is_pure_expr(cond) && evaluable_body(body),
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

fn collect_pure_funcs<'a>(stmts: &[Stmt<'a>], interner: &Interner) -> HashMap<Symbol, FuncDef<'a>> {
    let purity = super::purity::Purity::of_program(stmts, interner);
    let mut funcs = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            body,
            is_native,
            ..
        } = stmt
        {
            // A function with a `mutable` parameter writes its caller's binding: its answer is
            // not the whole of what a call to it does, so it is never evaluated to a value.
            if *is_native
                || params
                    .iter()
                    .any(|(_, ty)| matches!(ty, crate::ast::stmt::TypeExpr::Mutable { .. }))
            {
                continue;
            }
            if purity.is_pure(*name) && evaluable_body(body) {
                let param_syms: Vec<Symbol> = params.iter().map(|(s, _)| *s).collect();
                funcs.insert(
                    *name,
                    FuncDef {
                        params: param_syms,
                        body,
                    },
                );
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
            if let Value::Bool(b) = eval_expr_to_value(operand, locals, funcs, memo, steps, depth)?
            {
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

            let result =
                eval_block_to_value(func.body, &mut call_locals, funcs, memo, steps, depth + 1);
            memo.insert(memo_key, result.clone());
            result
        }
        _ => None,
    }
}

fn eval_binop(op: BinaryOpKind, lv: &Value, rv: &Value) -> Option<Value> {
    match (op, lv, rv) {
        // Constant-fold integer arithmetic ONLY when the result fits i64; on overflow
        // return None so the now-exact runtime computes the promoted (BigInt) value.
        // The folder must never bake a WRAPPED constant the runtime would not produce.
        (BinaryOpKind::Add, Value::Int(a), Value::Int(b)) => a.checked_add(*b).map(Value::Int),
        (BinaryOpKind::Subtract, Value::Int(a), Value::Int(b)) => a.checked_sub(*b).map(Value::Int),
        (BinaryOpKind::Multiply, Value::Int(a), Value::Int(b)) => a.checked_mul(*b).map(Value::Int),
        (BinaryOpKind::Divide, Value::Int(a), Value::Int(b)) if *b != 0 => {
            a.checked_div(*b).map(Value::Int)
        }
        (BinaryOpKind::Modulo, Value::Int(a), Value::Int(b)) if *b != 0 => {
            a.checked_rem(*b).map(Value::Int)
        }
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
        (BinaryOpKind::Divide, Value::Float(a), Value::Float(b)) if *b != 0.0 => {
            Some(Value::Float(a / b))
        }
        // Exponentiation: Float `a ** b` via powf (softmax's `e^x`); exact Int power that fits i64.
        (BinaryOpKind::Pow, Value::Float(a), Value::Float(b)) => Some(Value::Float(a.powf(*b))),
        (BinaryOpKind::Pow, Value::Int(a), Value::Int(b)) if *b >= 0 && *b <= 63 => {
            a.checked_pow(*b as u32).map(Value::Int)
        }
        (BinaryOpKind::Lt, Value::Float(a), Value::Float(b)) => Some(Value::Bool(a < b)),
        (BinaryOpKind::Gt, Value::Float(a), Value::Float(b)) => Some(Value::Bool(a > b)),
        (BinaryOpKind::LtEq, Value::Float(a), Value::Float(b)) => Some(Value::Bool(a <= b)),
        (BinaryOpKind::GtEq, Value::Float(a), Value::Float(b)) => Some(Value::Bool(a >= b)),
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
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
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
        Value::Seq(_) => None,
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
        // `item i of A` over a statically-known `Seq` + a known 1-based index → the element.
        Expr::Index { collection, index } => {
            let seq = match expr_to_value(collection, store)? {
                Value::Seq(v) => v,
                _ => return None,
            };
            let i = match expr_to_value(index, store)? {
                Value::Int(n) => n,
                _ => return None,
            };
            (i >= 1 && (i as usize) <= seq.len()).then(|| seq[(i - 1) as usize].clone())
        }
        Expr::Length { collection } => match expr_to_value(collection, store)? {
            Value::Seq(v) => Some(Value::Int(v.len() as i64)),
            _ => None,
        },
        _ => None,
    }
}

fn try_eval_expr<'a>(expr: &'a Expr<'a>, env: &mut SuperEnv<'a>, depth: usize) -> Option<Value> {
    eval_expr_to_value(
        expr,
        &env.store,
        &env.funcs,
        &mut env.memo,
        &mut env.steps,
        depth,
    )
}

/// A call's arguments, driven with the pre-call store — except each one at a `mutable` parameter
/// position ([`crate::analysis::callgraph::lent_positions`]), which is the place the callee writes
/// and stays as written. The call writes every binding it lends, so each leaves the abstract store:
/// a later read of one is never folded to its value from before the call.
fn drive_call_arguments<'a>(
    function: Symbol,
    args: &[&'a Expr<'a>],
    env: &mut SuperEnv<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    depth: usize,
) -> Vec<&'a Expr<'a>> {
    let lent = crate::analysis::callgraph::lent_positions(function);
    let driven = args
        .iter()
        .enumerate()
        .map(|(i, a)| {
            if lent.contains(&i) {
                *a
            } else {
                drive_expr(a, env, expr_arena, depth)
            }
        })
        .collect();
    for at in lent {
        if let Some(root) = args.get(at).and_then(|a| lent_root(a)) {
            env.store.remove(&root);
        }
    }
    driven
}

/// The binding a lent place is rooted at: the binding itself, or the record or list a field or
/// an element of it belongs to.
fn lent_root(place: &Expr) -> Option<Symbol> {
    match place {
        Expr::Identifier(sym) => Some(*sym),
        Expr::FieldAccess { object, .. } => lent_root(object),
        Expr::Index { collection, .. } => lent_root(collection),
        _ => None,
    }
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
                    // A `Seq` value never propagates as a literal (only its elements do, via `Index` folding) —
                    // leave the identifier in place.
                    Value::Seq(_) => {}
                }
            }
            expr
        }
        Expr::BinaryOp { op, left, right } => {
            let dl = drive_expr(left, env, expr_arena, depth);
            let dr = drive_expr(right, env, expr_arena, depth);
            // Try value-level evaluation
            if let (Some(lv), Some(rv)) =
                (expr_to_value(dl, &env.store), expr_to_value(dr, &env.store))
            {
                if let Some(result) = eval_binop(*op, &lv, &rv) {
                    return value_to_expr(&result, expr_arena);
                }
            }
            if std::ptr::eq(dl, *left) && std::ptr::eq(dr, *right) {
                expr
            } else {
                expr_arena.alloc(Expr::BinaryOp {
                    op: *op,
                    left: dl,
                    right: dr,
                })
            }
        }
        Expr::Not { operand } => {
            let d = drive_expr(operand, env, expr_arena, depth);
            if let Expr::Literal(Literal::Boolean(b)) = d {
                return expr_arena.alloc(Expr::Literal(Literal::Boolean(!b)));
            }
            if std::ptr::eq(d, *operand) {
                expr
            } else {
                expr_arena.alloc(Expr::Not { operand: d })
            }
        }
        Expr::Call { function, args } => {
            let driven_args = drive_call_arguments(*function, args, env, expr_arena, depth);
            // Don't replace calls in the AST — preserve function call sites.
            // Call evaluation is done at the store level in drive_stmt for Let/Set.
            let changed = driven_args
                .iter()
                .zip(args.iter())
                .any(|(d, o)| !std::ptr::eq(*d, *o));
            if changed {
                expr_arena.alloc(Expr::Call {
                    function: *function,
                    args: driven_args,
                })
            } else {
                expr
            }
        }
        Expr::Length { collection } => {
            // A statically-known `Seq`'s length is a constant (array-fold programs only).
            if env.array_fold {
                if let Some(val) = expr_to_value(expr, &env.store) {
                    if let Some(lit) = value_to_literal(&val) {
                        return expr_arena.alloc(Expr::Literal(lit));
                    }
                }
            }
            let d = drive_expr(collection, env, expr_arena, depth);
            if std::ptr::eq(d, *collection) {
                expr
            } else {
                expr_arena.alloc(Expr::Length { collection: d })
            }
        }
        // Index over a statically-known array folds to the element literal — ONLY for array-fold programs (the
        // array is being constant-folded away entirely). For every other program the read is left untouched, so
        // the affine / oracle-bounds / BCE array passes see exactly what they did before.
        Expr::Index { .. } => {
            if env.array_fold {
                if let Some(val) = expr_to_value(expr, &env.store) {
                    if let Some(lit) = value_to_literal(&val) {
                        return expr_arena.alloc(Expr::Literal(lit));
                    }
                }
            }
            expr
        }
        Expr::Slice { .. } => expr,
        Expr::Contains { collection, value } => {
            let dc = drive_expr(collection, env, expr_arena, depth);
            let dv = drive_expr(value, env, expr_arena, depth);
            if std::ptr::eq(dc, *collection) && std::ptr::eq(dv, *value) {
                expr
            } else {
                expr_arena.alloc(Expr::Contains {
                    collection: dc,
                    value: dv,
                })
            }
        }
        Expr::FieldAccess { object, field } => {
            let d = drive_expr(object, env, expr_arena, depth);
            if std::ptr::eq(d, *object) {
                expr
            } else {
                expr_arena.alloc(Expr::FieldAccess {
                    object: d,
                    field: *field,
                })
            }
        }
        Expr::OptionSome { value } => {
            let d = drive_expr(value, env, expr_arena, depth);
            if std::ptr::eq(d, *value) {
                expr
            } else {
                expr_arena.alloc(Expr::OptionSome { value: d })
            }
        }
        Expr::Copy { expr: inner } => {
            let d = drive_expr(inner, env, expr_arena, depth);
            if std::ptr::eq(d, *inner) {
                expr
            } else {
                expr_arena.alloc(Expr::Copy { expr: d })
            }
        }
        Expr::Give { value } => {
            let d = drive_expr(value, env, expr_arena, depth);
            if std::ptr::eq(d, *value) {
                expr
            } else {
                expr_arena.alloc(Expr::Give { value: d })
            }
        }
        Expr::List(elems) => {
            let driven: Vec<&'a Expr<'a>> = elems
                .iter()
                .map(|e| drive_expr(e, env, expr_arena, depth))
                .collect();
            let changed = driven
                .iter()
                .zip(elems.iter())
                .any(|(d, o)| !std::ptr::eq(*d, *o));
            if changed {
                expr_arena.alloc(Expr::List(driven))
            } else {
                expr
            }
        }
        Expr::Tuple(elems) => {
            let driven: Vec<&'a Expr<'a>> = elems
                .iter()
                .map(|e| drive_expr(e, env, expr_arena, depth))
                .collect();
            let changed = driven
                .iter()
                .zip(elems.iter())
                .any(|(d, o)| !std::ptr::eq(*d, *o));
            if changed {
                expr_arena.alloc(Expr::Tuple(driven))
            } else {
                expr
            }
        }
        // Leaves and complex expressions passed through
        _ => expr,
    }
}

/// Drive `block`, a block nested in the statement being driven, against a copy of the store without
/// the names it binds on entry (`binders`). The store it leaves is the one it started from, less
/// everything the block may write, with what the block assigns to names it never declares — the
/// enclosing scope's bindings — at the values the drive gave them. A name the block declares ends with
/// the block, whatever it held there.
fn drive_nested_block<'a>(
    block: &'a [Stmt<'a>],
    binders: &[Symbol],
    env: &mut SuperEnv<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    depth: usize,
) -> &'a [Stmt<'a>] {
    let outer = env.store.clone();
    for b in binders {
        env.store.remove(b);
    }
    let driven = drive_block(block, env, expr_arena, stmt_arena, interner, depth);
    let inner = std::mem::take(&mut env.store);
    let written = env.writes_of(WriteSet::of_block(block));
    let mut after = outer;
    forget(&mut after, &written);
    if !written.is_opaque() {
        let declared = capture::frame_names(binders, block);
        for sym in written.reassigned() {
            if declared.contains(&sym) {
                continue;
            }
            if let Some(value) = inner.get(&sym) {
                after.insert(sym, value.clone());
            }
        }
    }
    env.store = after;
    driven
}

fn drive_stmt<'a>(
    stmt: Stmt<'a>,
    env: &mut SuperEnv<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    depth: usize,
) -> Option<Stmt<'a>> {
    // Every name a statement binds where it stands takes a new value there, and nothing an earlier
    // binding of the name gave it survives: a `Let` forgets its name once it has read its value, and a
    // statement driven through unchanged forgets everything it may write, its binders included. A
    // block nested in the statement starts without the names it binds on entry.
    let entry = EntryBinders::of(&stmt);
    match stmt {
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => {
            let driven = drive_expr(value, env, expr_arena, depth);
            // A `Let` shadows every earlier binding of its name, known or not.
            env.store.remove(&var);
            if env.array_fold {
                // Resolve to a store value when statically known: a `new Seq`, a leaf literal, a pure
                // arithmetic/array-read/builtin expression (`eval_static` — folds a reduction's
                // `denom = sqrt(ss/n + eps)`), or a pure call. Text on a MUTABLE binding is left owned;
                // everything else is output-safe (`Set` overwrites, a `While` widens). Lets a following
                // constant-trip loop run concretely.
                let resolved = if let Some(seq) = new_seq_value(driven, interner) {
                    Some(seq)
                } else if let Some(v) = expr_to_value(driven, &env.store) {
                    Some(v)
                } else {
                    let mut s = 0usize;
                    if let Some(v) = eval_static(driven, &env.store, interner, &mut s) {
                        Some(v)
                    } else if !mutable {
                        try_eval_expr(driven, env, depth)
                    } else {
                        None
                    }
                };
                if let Some(val) = resolved {
                    if !(mutable && matches!(val, Value::Text(_))) {
                        env.store.insert(var, val);
                    }
                }
            } else if !mutable {
                // Original behavior: track only immutable scalar bindings (literals / pure calls).
                if let Some(val) = expr_to_value(driven, &env.store) {
                    env.store.insert(var, val);
                } else if let Some(val) = try_eval_expr(driven, env, depth) {
                    env.store.insert(var, val);
                }
            }
            Some(Stmt::Let {
                var,
                ty,
                value: driven,
                mutable,
            })
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
            Some(Stmt::Set {
                target,
                value: driven,
            })
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            let driven_cond = drive_expr(cond, env, expr_arena, depth);
            // If condition is known, eliminate dead branch
            if let Expr::Literal(Literal::Boolean(b)) = driven_cond {
                if *b {
                    // Take then branch only
                    let driven_then = drive_nested_block(
                        then_block,
                        entry.of_block(then_block),
                        env,
                        expr_arena,
                        stmt_arena,
                        interner,
                        depth,
                    );
                    Some(Stmt::If {
                        cond: driven_cond,
                        then_block: driven_then,
                        else_block: None,
                    })
                } else {
                    // Take else branch only
                    if let Some(eb) = else_block {
                        let driven_else = drive_nested_block(
                            eb,
                            entry.of_block(eb),
                            env,
                            expr_arena,
                            stmt_arena,
                            interner,
                            depth,
                        );
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
                // Condition unknown: drive both branches from the same store, then join.
                let snapshot = env.store.clone();
                let driven_then = drive_nested_block(
                    then_block,
                    entry.of_block(then_block),
                    env,
                    expr_arena,
                    stmt_arena,
                    interner,
                    depth,
                );
                let then_store = std::mem::replace(&mut env.store, snapshot);
                let driven_else = else_block.map(|eb| {
                    drive_nested_block(
                        eb,
                        entry.of_block(eb),
                        env,
                        expr_arena,
                        stmt_arena,
                        interner,
                        depth,
                    )
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
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            // Concrete-execution fast path for a fully-static ARRAY-building loop (a fill / matmul / reduction
            // nest). The ordinary driver widens modified vars and never runs the loop; here, if the loop
            // mutates an array and every read resolves against the current store, we EXECUTE it to completion,
            // updating the store with the concrete post-loop array values — so the reads AFTER it fold to
            // constants. The loop is KEPT in the residual (sound: the runtime array is still built the same
            // way); it becomes dead once all its reads fold and DCE removes it. Confined to array loops, so
            // pure-scalar loops keep their exact widening behavior.
            if env.array_fold && loop_touches_array(body) {
                let snap = env.store.clone();
                let mut steps = 0usize;
                if run_static_loop(cond, body, &mut env.store, interner, &mut steps).is_some() {
                    return Some(Stmt::While {
                        cond,
                        body,
                        decreasing,
                    });
                }
                env.store = snap; // not fully static (or over budget) — fall back to widening
            }

            // `run_static_loop` above could NOT fully execute this loop (array_fold off, the loop
            // doesn't touch an array, or a read/stmt was unresolvable — e.g. fannkuch's outer loop
            // allocates a fresh `perm` each iteration). On that fallback path we WIDEN everything the
            // body may write — every name it binds or assigns, every array or record it writes in place
            // (`perm1` rotated through `Set item i of perm1` must not keep its `[0,1,2,…]` value), every
            // binding it lends, every global a callee it or its condition calls writes — and drive the body
            // once.
            let written = env.writes_of(WriteSet::of_stmt(&stmt));
            let snapshot = env.store.clone();

            // Snapshot configuration before entering loop
            let pre_config = Configuration { expr: cond };

            forget(&mut env.store, &written);

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

            // After the loop, what the body never writes keeps its value from before it.
            let mut post_loop_store = snapshot;
            forget(&mut post_loop_store, &written);
            env.store = post_loop_store;

            Some(Stmt::While {
                cond: driven_cond,
                body: driven_body,
                decreasing,
            })
        }
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => {
            let driven_iter = drive_expr(iterable, env, expr_arena, depth);
            // Each iteration starts without the loop's binders and without anything an earlier iteration
            // (or the iterable's evaluation) wrote.
            let evaluated = env.writes_of(WriteSet::of_expr(iterable));
            let written = env.writes_of(WriteSet::of_block(body));
            forget(&mut env.store, &evaluated);
            forget(&mut env.store, &written);
            let driven_body = drive_nested_block(
                body,
                entry.of_block(body),
                env,
                expr_arena,
                stmt_arena,
                interner,
                depth,
            );
            // The body was driven ONCE, but the loop runs any number of times, none included: nothing it
            // writes holds its single-drive value afterward — neither a collection it pushes to (`Repeat for
            // i from 1 to 10: Push 0 to nums` must not fold `length of nums` to 1) nor a name it sets (a
            // loop over an empty list leaves it as it was).
            forget(&mut env.store, &written);
            Some(Stmt::Repeat {
                pattern,
                iterable: driven_iter,
                body: driven_body,
            })
        }
        Stmt::Show { object, recipient } => {
            let driven = drive_expr(object, env, expr_arena, depth);
            Some(Stmt::Show {
                object: driven,
                recipient,
            })
        }
        Stmt::Return { value } => {
            let driven = value.map(|v| drive_expr(v, env, expr_arena, depth));
            Some(Stmt::Return { value: driven })
        }
        Stmt::Call { function, args } => {
            // Arguments are evaluated with the PRE-call store (they see the current values)...
            let driven_args = drive_call_arguments(function, &args, env, expr_arena, depth);
            // ...then the call can mutate any global some function writes — drop those from the abstract store
            // so a later read of one is NOT folded to its stale pre-call value.
            if env.callee_globals.opaque {
                env.store.clear();
            } else if !env.callee_globals.is_empty() {
                let globals: Vec<Symbol> = env.callee_globals.symbols.iter().copied().collect();
                for g in globals {
                    env.store.remove(&g);
                }
            }
            Some(Stmt::Call {
                function,
                args: driven_args,
            })
        }
        Stmt::Push { value, collection } => {
            let driven_val = drive_expr(value, env, expr_arena, depth);
            if let Expr::Identifier(sym) = collection {
                // Append to a statically-known array when the pushed value is known; otherwise the array
                // becomes runtime-opaque (invalidate). The statement itself is KEPT (builds the runtime array).
                match (env.store.get(sym), expr_to_value(driven_val, &env.store)) {
                    (Some(Value::Seq(_)), Some(elem)) => {
                        if let Some(Value::Seq(seq)) = env.store.get_mut(sym) {
                            seq.push(elem);
                        }
                    }
                    _ => {
                        env.store.remove(sym);
                    }
                }
            }
            Some(Stmt::Push {
                value: driven_val,
                collection,
            })
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            // `Set item i of A to v`: update a statically-known array in place when i and v are known;
            // otherwise invalidate. The statement is KEPT (the runtime array is still written). Index and value
            // are NOT driven — codegen peephole patterns depend on the original variable names.
            if let Expr::Identifier(sym) = collection {
                let iv = expr_to_value(index, &env.store);
                let vv = expr_to_value(value, &env.store);
                match (env.store.get(sym), iv, vv) {
                    (Some(Value::Seq(_)), Some(Value::Int(i)), Some(v)) if i >= 1 => {
                        if let Some(Value::Seq(seq)) = env.store.get_mut(sym) {
                            let idx = (i - 1) as usize;
                            if idx >= seq.len() {
                                seq.resize(idx + 1, Value::Nothing);
                            }
                            seq[idx] = v;
                        }
                    }
                    _ => {
                        env.store.remove(sym);
                    }
                }
            }
            Some(Stmt::SetIndex {
                collection,
                index,
                value,
            })
        }
        Stmt::SetField {
            object,
            field,
            value,
        } => {
            let dv = drive_expr(value, env, expr_arena, depth);
            if let Expr::Identifier(sym) = object {
                env.store.remove(sym);
            }
            Some(Stmt::SetField {
                object,
                field,
                value: dv,
            })
        }
        Stmt::RuntimeAssert {
            condition,
            hard,
            dim,
            reason,
        } => {
            let driven = drive_expr(condition, env, expr_arena, depth);
            Some(Stmt::RuntimeAssert {
                condition: driven,
                hard,
                dim,
                reason,
            })
        }
        Stmt::FunctionDef {
            name,
            params,
            generics,
            body,
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
            opt_flags,
        } => {
            // Drive inside function body with fresh store
            let snapshot = env.store.clone();
            env.store.clear();
            let driven_body = drive_block(body, env, expr_arena, stmt_arena, interner, depth);
            env.store = snapshot;
            Some(Stmt::FunctionDef {
                name,
                params,
                generics,
                body: driven_body,
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            })
        }
        // A `Splice` is a transparent desugar container (its `Let`s leak to the enclosing scope). For array-fold
        // programs we drive its body against the SAME store so tensor-sugar loops (which land inside Splices)
        // fold even before splice-fusion flattens them; otherwise it passes through unchanged (original).
        Stmt::Splice { body } if env.array_fold => {
            let driven = drive_block(body, env, expr_arena, stmt_arena, interner, depth);
            Some(Stmt::Splice { body: driven })
        }
        // Zone blocks: pass through unchanged. Propagating inside zones can
        // eliminate zone escape patterns that the Rust compiler needs to detect. Everything else passes
        // through undriven too. Either way nothing it may write — a name an `Inspect` arm sets, the
        // binding `Pop … into` fills — keeps a value from before it.
        other => {
            let written = env.writes_of(WriteSet::of_stmt(&other));
            forget(&mut env.store, &written);
            Some(other)
        }
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

// =============================================================================
// Public API
// =============================================================================

pub fn supercompile_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let funcs = collect_pure_funcs(&stmts, interner);
    let mut env = SuperEnv::new();
    // Array supercompilation only engages for a WHOLE program in the pure static-array fragment — everything
    // else is driven exactly as the original supercompiler (sound + byte-identical, no perturbation of the
    // array/codegen passes and no unsound stale-array fold across ops we don't model, e.g. Pop).
    env.array_fold = is_pure_static_array_program(&stmts, interner);
    env.funcs = funcs;
    env.callee_globals = CalleeGlobals::of_program(&stmts);

    let mut result = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        if let Some(driven) = drive_stmt(stmt, &mut env, expr_arena, stmt_arena, interner, 0) {
            result.push(driven);
        }
    }
    result
}

/// Whether the WHOLE program is in the fragment array supercompilation soundly models: only
/// `Let`/`Set`/`Push`/`Set item`/`While`/`If`/`Show`/`Splice`/`Return` statements, over
/// literal/identifier/arithmetic/`Index`/`Length`/`new Seq`/pure-math-builtin expressions. Anything else — a
/// native or user function, `Pop`/`slice`/`remove`/`add`/aliasing, a `Repeat`, runtime input via `parseInt`,
/// … — makes the program INELIGIBLE, so the supercompiler behaves exactly as before for it.
fn is_pure_static_array_program(stmts: &[Stmt], interner: &Interner) -> bool {
    stmts.iter().all(|s| stmt_in_fragment(s, interner))
}

fn stmt_in_fragment(stmt: &Stmt, interner: &Interner) -> bool {
    match stmt {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_in_fragment(value, interner),
        Stmt::Push { value, collection } => {
            expr_in_fragment(value, interner) && expr_in_fragment(collection, interner)
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            expr_in_fragment(collection, interner)
                && expr_in_fragment(index, interner)
                && expr_in_fragment(value, interner)
        }
        Stmt::While { cond, body, .. } => {
            expr_in_fragment(cond, interner) && is_pure_static_array_program(body, interner)
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            expr_in_fragment(cond, interner)
                && is_pure_static_array_program(then_block, interner)
                && else_block.map_or(true, |eb| is_pure_static_array_program(eb, interner))
        }
        Stmt::Show { object, .. } => expr_in_fragment(object, interner),
        Stmt::Splice { body } => is_pure_static_array_program(body, interner),
        Stmt::Return { value } => value.map_or(true, |v| expr_in_fragment(v, interner)),
        _ => false, // FunctionDef, Pop, RemoveFrom, SetAdd, Repeat, Call, Zone, … → not in the fragment
    }
}

fn expr_in_fragment(expr: &Expr, interner: &Interner) -> bool {
    match expr {
        Expr::Literal(_) | Expr::Identifier(_) => true,
        Expr::BinaryOp { left, right, .. } => {
            expr_in_fragment(left, interner) && expr_in_fragment(right, interner)
        }
        Expr::Not { operand } => expr_in_fragment(operand, interner),
        Expr::Index { collection, index } => {
            expr_in_fragment(collection, interner) && expr_in_fragment(index, interner)
        }
        Expr::Length { collection } => expr_in_fragment(collection, interner),
        Expr::New { .. } => true,
        // Only the pure math builtins the reductions use — NOT native (`parseInt`) or user functions.
        Expr::Call { function, args } => {
            matches!(
                interner.resolve(*function),
                "sqrt" | "max" | "min" | "round" | "floor" | "ceil" | "abs"
            ) && args.iter().all(|a| expr_in_fragment(a, interner))
        }
        _ => false, // Slice, Contains, SeqConcat, List, Tuple, closures, … → not in the fragment
    }
}

/// Homeomorphic embedding check: `e1 ◁ e2`.
///
/// Returns true if `e1` is homeomorphically embedded in `e2`, meaning `e1` can
/// be obtained from `e2` by erasing some constructors (diving) or both share
/// the same constructor with all children embedded (coupling).
pub fn embeds<'a>(e1: &Expr<'a>, e2: &Expr<'a>) -> bool {
    match (e1, e2) {
        // Coupling: same constructor, all children embed
        (
            Expr::BinaryOp {
                op: op1,
                left: l1,
                right: r1,
            },
            Expr::BinaryOp {
                op: op2,
                left: l2,
                right: r2,
            },
        ) if op1 == op2 => embeds(l1, l2) && embeds(r1, r2),
        (
            Expr::Call {
                function: f1,
                args: a1,
            },
            Expr::Call {
                function: f2,
                args: a2,
            },
        ) if f1 == f2 && a1.len() == a2.len() => {
            a1.iter().zip(a2.iter()).all(|(x, y)| embeds(x, y))
        }
        (Expr::Not { operand: e1_inner }, Expr::Not { operand: e2_inner }) => {
            embeds(e1_inner, e2_inner)
        }
        // Base: literals and identifiers embed in themselves
        (Expr::Literal(l1), Expr::Literal(l2)) => l1 == l2,
        (Expr::Identifier(s1), Expr::Identifier(s2)) => s1 == s2,
        // Diving: e1 embeds in a subterm of e2
        (_, Expr::BinaryOp { left, right, .. }) => embeds(e1, left) || embeds(e1, right),
        (_, Expr::Call { args, .. }) => args.iter().any(|a| embeds(e1, a)),
        (_, Expr::Not { operand }) => embeds(e1, operand),
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
    MsgResult {
        expr,
        num_substitutions: counter,
    }
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
        (
            Expr::BinaryOp {
                op: op1,
                left: l1,
                right: r1,
            },
            Expr::BinaryOp {
                op: op2,
                left: l2,
                right: r2,
            },
        ) if op1 == op2 => {
            let left = msg_inner(l1, l2, arena, interner, counter);
            let right = msg_inner(r1, r2, arena, interner, counter);
            arena.alloc(Expr::BinaryOp {
                op: *op1,
                left,
                right,
            })
        }
        // Same Call constructor → recurse into args
        (
            Expr::Call {
                function: f1,
                args: a1,
            },
            Expr::Call {
                function: f2,
                args: a2,
            },
        ) if f1 == f2 && a1.len() == a2.len() => {
            let args: Vec<&'a Expr<'a>> = a1
                .iter()
                .zip(a2.iter())
                .map(|(x, y)| msg_inner(x, y, arena, interner, counter))
                .collect();
            arena.alloc(Expr::Call {
                function: *f1,
                args,
            })
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

// =============================================================================
// Static array evaluation — the machinery that folds a constant-data tensor op
// (matmul / reduction over literal-filled arrays) to its result constants.
// =============================================================================

/// `Let A be a new Seq/List/Vec` → a statically-known (empty) array value.
fn new_seq_value(driven: &Expr, interner: &Interner) -> Option<Value> {
    if let Expr::New { type_name, .. } = driven {
        if matches!(interner.resolve(*type_name), "Seq" | "List" | "Vec") {
            return Some(Value::Seq(Vec::new()));
        }
    }
    None
}

/// Whether a loop body reads OR writes an array — the gate for attempting concrete execution. Pure-scalar
/// loops (no array touch) keep their exact widening behavior; a loop that merely MIGHT be static is still safe
/// to attempt, because `run_static_loop` bails (and the caller restores) the moment a read/write can't resolve.
fn loop_touches_array(body: &[Stmt]) -> bool {
    body.iter().any(stmt_touches_array)
}

fn stmt_touches_array(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Push { .. } | Stmt::SetIndex { .. } => true,
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_reads_array(value),
        Stmt::Return { value } => value.map_or(false, expr_reads_array),
        Stmt::While { cond, body, .. } => expr_reads_array(cond) || loop_touches_array(body),
        Stmt::Repeat { body, .. } => loop_touches_array(body),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            expr_reads_array(cond)
                || loop_touches_array(then_block)
                || else_block.map_or(false, |eb| loop_touches_array(eb))
        }
        _ => false,
    }
}

fn expr_reads_array(expr: &Expr) -> bool {
    match expr {
        Expr::Index { .. } | Expr::Length { .. } => true,
        Expr::BinaryOp { left, right, .. } => expr_reads_array(left) || expr_reads_array(right),
        Expr::Not { operand } => expr_reads_array(operand),
        Expr::Call { args, .. } => args.iter().any(|a| expr_reads_array(a)),
        _ => false,
    }
}

/// Concretely execute a constant-trip loop over the store (arrays included) to completion. Returns `Some(())`
/// with the store holding the post-loop values, or `None` (leaving the store partially mutated — the caller
/// restores its snapshot) if any read is unknown or the step budget is exceeded.
fn run_static_loop(
    cond: &Expr,
    body: &[Stmt],
    store: &mut HashMap<Symbol, Value>,
    interner: &Interner,
    steps: &mut usize,
) -> Option<()> {
    loop {
        *steps += 1;
        if *steps > MAX_INLINE_STEPS {
            return None;
        }
        match eval_static(cond, store, interner, steps)? {
            Value::Bool(true) => {
                for s in body {
                    exec_static_stmt(s, store, interner, steps)?;
                }
            }
            Value::Bool(false) => return Some(()),
            _ => return None,
        }
    }
}

/// Concretely execute one statement over the store. Only effect-free, statically-resolvable forms are handled;
/// anything else (a `Show`, a `Return`, an unresolved read) returns `None` to abort the concrete run.
fn exec_static_stmt(
    stmt: &Stmt,
    store: &mut HashMap<Symbol, Value>,
    interner: &Interner,
    steps: &mut usize,
) -> Option<()> {
    *steps += 1;
    if *steps > MAX_INLINE_STEPS {
        return None;
    }
    match stmt {
        Stmt::Let { var, value, .. } => {
            let v = eval_static(value, store, interner, steps)?;
            store.insert(*var, v);
            Some(())
        }
        Stmt::Set { target, value } => {
            let v = eval_static(value, store, interner, steps)?;
            store.insert(*target, v);
            Some(())
        }
        Stmt::Push { value, collection } => {
            let sym = as_ident_sym(collection)?;
            let v = eval_static(value, store, interner, steps)?;
            match store.get_mut(&sym) {
                Some(Value::Seq(seq)) => {
                    seq.push(v);
                    Some(())
                }
                _ => None,
            }
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            let sym = as_ident_sym(collection)?;
            let i = as_int_val(eval_static(index, store, interner, steps)?)?;
            if i < 1 {
                return None;
            }
            let v = eval_static(value, store, interner, steps)?;
            match store.get_mut(&sym) {
                Some(Value::Seq(seq)) => {
                    let idx = (i - 1) as usize;
                    if idx >= seq.len() {
                        seq.resize(idx + 1, Value::Nothing);
                    }
                    seq[idx] = v;
                    Some(())
                }
                _ => None,
            }
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => match eval_static(cond, store, interner, steps)? {
            Value::Bool(true) => {
                for s in *then_block {
                    exec_static_stmt(s, store, interner, steps)?;
                }
                Some(())
            }
            Value::Bool(false) => {
                if let Some(eb) = else_block {
                    for s in *eb {
                        exec_static_stmt(s, store, interner, steps)?;
                    }
                }
                Some(())
            }
            _ => None,
        },
        Stmt::While { cond, body, .. } => run_static_loop(cond, body, store, interner, steps),
        _ => None,
    }
}

/// A pure concrete evaluator over `Value` (arrays included): literals, identifiers, arithmetic, array reads,
/// and the pure math builtins the reductions use (`sqrt`/`max`/`min`/`round`/`floor`/`ceil`/`abs`). Returns
/// `None` for anything it cannot fully resolve.
fn eval_static(
    expr: &Expr,
    store: &HashMap<Symbol, Value>,
    interner: &Interner,
    steps: &mut usize,
) -> Option<Value> {
    *steps += 1;
    if *steps > MAX_INLINE_STEPS {
        return None;
    }
    match expr {
        Expr::Literal(Literal::Number(n)) => Some(Value::Int(*n)),
        Expr::Literal(Literal::Float(f)) => Some(Value::Float(*f)),
        Expr::Literal(Literal::Boolean(b)) => Some(Value::Bool(*b)),
        Expr::Literal(Literal::Text(s)) => Some(Value::Text(*s)),
        Expr::Literal(Literal::Nothing) => Some(Value::Nothing),
        Expr::Identifier(sym) => store.get(sym).cloned(),
        Expr::BinaryOp { op, left, right } => {
            let l = eval_static(left, store, interner, steps)?;
            let r = eval_static(right, store, interner, steps)?;
            eval_binop(*op, &l, &r)
        }
        Expr::Not { operand } => match eval_static(operand, store, interner, steps)? {
            Value::Bool(b) => Some(Value::Bool(!b)),
            _ => None,
        },
        Expr::Index { collection, index } => {
            let seq = match eval_static(collection, store, interner, steps)? {
                Value::Seq(v) => v,
                _ => return None,
            };
            let i = as_int_val(eval_static(index, store, interner, steps)?)?;
            (i >= 1 && (i as usize) <= seq.len()).then(|| seq[(i - 1) as usize].clone())
        }
        Expr::Length { collection } => match eval_static(collection, store, interner, steps)? {
            Value::Seq(v) => Some(Value::Int(v.len() as i64)),
            _ => None,
        },
        Expr::Call { function, args } => {
            let mut vals = Vec::with_capacity(args.len());
            for a in args.iter() {
                vals.push(eval_static(a, store, interner, steps)?);
            }
            eval_static_builtin(interner.resolve(*function), &vals)
        }
        _ => None,
    }
}

/// The pure math builtins the reduction desugars use. Semantics mirror the runtime (`f64` libm + `round`
/// half-away-from-zero), so a folded constant equals what the tier would compute.
fn eval_static_builtin(name: &str, vals: &[Value]) -> Option<Value> {
    match (name, vals) {
        ("sqrt", [Value::Float(x)]) => Some(Value::Float(x.sqrt())),
        ("sqrt", [Value::Int(x)]) => Some(Value::Float((*x as f64).sqrt())),
        ("max", [Value::Float(a), Value::Float(b)]) => Some(Value::Float(a.max(*b))),
        ("max", [Value::Int(a), Value::Int(b)]) => Some(Value::Int((*a).max(*b))),
        ("min", [Value::Float(a), Value::Float(b)]) => Some(Value::Float(a.min(*b))),
        ("min", [Value::Int(a), Value::Int(b)]) => Some(Value::Int((*a).min(*b))),
        ("round", [Value::Float(x)]) => Some(Value::Float(x.round())),
        ("floor", [Value::Float(x)]) => Some(Value::Float(x.floor())),
        ("ceil", [Value::Float(x)]) => Some(Value::Float(x.ceil())),
        ("abs", [Value::Int(x)]) => Some(Value::Int(x.abs())),
        ("abs", [Value::Float(x)]) => Some(Value::Float(x.abs())),
        _ => None,
    }
}

fn as_ident_sym(expr: &Expr) -> Option<Symbol> {
    if let Expr::Identifier(s) = expr {
        Some(*s)
    } else {
        None
    }
}

fn as_int_val(v: Value) -> Option<i64> {
    if let Value::Int(n) = v {
        Some(n)
    } else {
        None
    }
}
