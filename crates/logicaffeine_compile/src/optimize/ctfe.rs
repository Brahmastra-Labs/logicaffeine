//! Compile-Time Function Evaluation (CTFE).
//!
//! Evaluates pure function calls with all-literal arguments at compile time,
//! replacing the call with its result. Step-limited to prevent infinite loops.

use std::collections::HashMap;

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};

const MAX_STEPS: usize = 10_000;
const MAX_DEPTH: usize = 16;

#[derive(Debug, Clone)]
enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(Symbol),
    Nothing,
}

struct FuncInfo<'a> {
    params: Vec<Symbol>,
    body: Block<'a>,
    is_pure: bool,
}

struct CtfeEnv<'a> {
    funcs: HashMap<Symbol, FuncInfo<'a>>,
}

fn is_pure_stmt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Let { value, .. } => is_pure_expr(value),
        Stmt::Set { value, .. } => is_pure_expr(value),
        Stmt::Return { value } => value.map_or(true, |v| is_pure_expr(v)),
        Stmt::If { cond, then_block, else_block } => {
            is_pure_expr(cond)
                && then_block.iter().all(is_pure_stmt)
                && else_block.map_or(true, |eb| eb.iter().all(is_pure_stmt))
        }
        Stmt::While { cond, body, .. } => {
            is_pure_expr(cond) && body.iter().all(is_pure_stmt)
        }
        Stmt::Call { .. } => true, // Allow calls (purity checked transitively)
        // IO, mutations to collections, escape blocks are impure
        Stmt::Show { .. } | Stmt::Escape { .. } | Stmt::Push { .. }
        | Stmt::Pop { .. } | Stmt::Add { .. } | Stmt::Remove { .. }
        | Stmt::SetIndex { .. } | Stmt::SetField { .. } | Stmt::Give { .. }
        | Stmt::WriteFile { .. } | Stmt::SendMessage { .. }
        | Stmt::Sleep { .. } | Stmt::Spawn { .. } | Stmt::Break => false,
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
        // Allocating expressions, closures, escape blocks are impure
        _ => false,
    }
}

fn collect_functions<'a>(stmts: &[Stmt<'a>]) -> HashMap<Symbol, FuncInfo<'a>> {
    let mut funcs = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, is_native, .. } = stmt {
            if *is_native { continue; }
            let param_symbols: Vec<Symbol> = params.iter().map(|(name, _)| *name).collect();
            let is_pure = body.iter().all(is_pure_stmt);
            if is_pure {
                funcs.insert(*name, FuncInfo {
                    params: param_symbols,
                    body,
                    is_pure,
                });
            }
        }
    }
    funcs
}

fn eval_expr(
    expr: &Expr,
    locals: &HashMap<Symbol, Value>,
    funcs: &HashMap<Symbol, FuncInfo>,
    steps: &mut usize,
    depth: usize,
    interner: &mut Interner,
) -> Option<Value> {
    if *steps >= MAX_STEPS || depth >= MAX_DEPTH {
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
            let lv = eval_expr(left, locals, funcs, steps, depth, interner)?;
            let rv = eval_expr(right, locals, funcs, steps, depth, interner)?;
            eval_binop(*op, lv, rv, interner)
        }
        Expr::Not { operand } => {
            if let Value::Bool(b) = eval_expr(operand, locals, funcs, steps, depth, interner)? {
                Some(Value::Bool(!b))
            } else {
                None
            }
        }
        Expr::Call { function, args } => {
            let func = funcs.get(function)?;
            if !func.is_pure { return None; }
            let mut arg_values = Vec::new();
            for arg in args {
                arg_values.push(eval_expr(arg, locals, funcs, steps, depth, interner)?);
            }
            if arg_values.len() != func.params.len() { return None; }
            let mut call_locals = HashMap::new();
            for (param, val) in func.params.iter().zip(arg_values) {
                call_locals.insert(*param, val);
            }
            eval_block(func.body, &mut call_locals, funcs, steps, depth + 1, interner)
        }
        _ => None,
    }
}

fn eval_binop(op: BinaryOpKind, lv: Value, rv: Value, interner: &mut Interner) -> Option<Value> {
    match (op, &lv, &rv) {
        (BinaryOpKind::Add, Value::Int(a), Value::Int(b)) => Some(Value::Int(a.wrapping_add(*b))),
        (BinaryOpKind::Subtract, Value::Int(a), Value::Int(b)) => Some(Value::Int(a.wrapping_sub(*b))),
        (BinaryOpKind::Multiply, Value::Int(a), Value::Int(b)) => Some(Value::Int(a.wrapping_mul(*b))),
        (BinaryOpKind::Divide, Value::Int(a), Value::Int(b)) if *b != 0 => Some(Value::Int(a / b)),
        (BinaryOpKind::Modulo, Value::Int(a), Value::Int(b)) if *b != 0 => Some(Value::Int(a % b)),
        (BinaryOpKind::Eq, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a == b)),
        (BinaryOpKind::NotEq, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a != b)),
        (BinaryOpKind::Lt, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a < b)),
        (BinaryOpKind::Gt, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a > b)),
        (BinaryOpKind::LtEq, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a <= b)),
        (BinaryOpKind::GtEq, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a >= b)),
        (BinaryOpKind::And, Value::Bool(a), Value::Bool(b)) => Some(Value::Bool(*a && *b)),
        (BinaryOpKind::Or, Value::Bool(a), Value::Bool(b)) => Some(Value::Bool(*a || *b)),
        (BinaryOpKind::Add | BinaryOpKind::Concat, Value::Text(a), Value::Text(b)) => {
            let a_str = interner.resolve(*a);
            let b_str = interner.resolve(*b);
            let combined = format!("{}{}", a_str, b_str);
            let sym = interner.intern(&combined);
            Some(Value::Text(sym))
        }
        (BinaryOpKind::Eq, Value::Text(a), Value::Text(b)) => Some(Value::Bool(a == b)),
        (BinaryOpKind::NotEq, Value::Text(a), Value::Text(b)) => Some(Value::Bool(a != b)),
        _ => None,
    }
}

enum StmtResult {
    Continue,
    Return(Value),
}

fn eval_block(
    stmts: &[Stmt],
    locals: &mut HashMap<Symbol, Value>,
    funcs: &HashMap<Symbol, FuncInfo>,
    steps: &mut usize,
    depth: usize,
    interner: &mut Interner,
) -> Option<Value> {
    for stmt in stmts {
        match eval_stmt(stmt, locals, funcs, steps, depth, interner)? {
            StmtResult::Continue => {}
            StmtResult::Return(v) => return Some(v),
        }
    }
    Some(Value::Nothing)
}

fn eval_stmt(
    stmt: &Stmt,
    locals: &mut HashMap<Symbol, Value>,
    funcs: &HashMap<Symbol, FuncInfo>,
    steps: &mut usize,
    depth: usize,
    interner: &mut Interner,
) -> Option<StmtResult> {
    if *steps >= MAX_STEPS {
        return None;
    }
    *steps += 1;

    match stmt {
        Stmt::Let { var, value, .. } => {
            let v = eval_expr(value, locals, funcs, steps, depth, interner)?;
            locals.insert(*var, v);
            Some(StmtResult::Continue)
        }
        Stmt::Set { target, value } => {
            let v = eval_expr(value, locals, funcs, steps, depth, interner)?;
            locals.insert(*target, v);
            Some(StmtResult::Continue)
        }
        Stmt::Return { value } => {
            if let Some(v) = value {
                let result = eval_expr(v, locals, funcs, steps, depth, interner)?;
                Some(StmtResult::Return(result))
            } else {
                Some(StmtResult::Return(Value::Nothing))
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            let cv = eval_expr(cond, locals, funcs, steps, depth, interner)?;
            if let Value::Bool(b) = cv {
                if b {
                    for s in *then_block {
                        match eval_stmt(s, locals, funcs, steps, depth, interner)? {
                            StmtResult::Continue => {}
                            StmtResult::Return(v) => return Some(StmtResult::Return(v)),
                        }
                    }
                } else if let Some(eb) = else_block {
                    for s in *eb {
                        match eval_stmt(s, locals, funcs, steps, depth, interner)? {
                            StmtResult::Continue => {}
                            StmtResult::Return(v) => return Some(StmtResult::Return(v)),
                        }
                    }
                }
                Some(StmtResult::Continue)
            } else {
                None
            }
        }
        Stmt::While { cond, body, .. } => {
            loop {
                let cv = eval_expr(cond, locals, funcs, steps, depth, interner)?;
                if let Value::Bool(false) = cv {
                    break;
                }
                if !matches!(cv, Value::Bool(true)) {
                    return None;
                }
                for s in *body {
                    match eval_stmt(s, locals, funcs, steps, depth, interner)? {
                        StmtResult::Continue => {}
                        StmtResult::Return(v) => return Some(StmtResult::Return(v)),
                    }
                }
            }
            Some(StmtResult::Continue)
        }
        _ => None,
    }
}

fn value_to_expr<'a>(val: &Value, arena: &'a Arena<Expr<'a>>) -> Option<&'a Expr<'a>> {
    match val {
        Value::Int(n) => Some(arena.alloc(Expr::Literal(Literal::Number(*n)))),
        Value::Float(f) => Some(arena.alloc(Expr::Literal(Literal::Float(*f)))),
        Value::Bool(b) => Some(arena.alloc(Expr::Literal(Literal::Boolean(*b)))),
        Value::Text(s) => Some(arena.alloc(Expr::Literal(Literal::Text(*s)))),
        Value::Nothing => Some(arena.alloc(Expr::Literal(Literal::Nothing))),
    }
}

fn try_ctfe_expr<'a>(
    expr: &'a Expr<'a>,
    env: &CtfeEnv<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
) -> Option<&'a Expr<'a>> {
    if let Expr::Call { function, args } = expr {
        let func = env.funcs.get(function)?;
        if !func.is_pure { return None; }
        if args.len() != func.params.len() { return None; }

        // All args must be literals
        let mut arg_values = Vec::new();
        for arg in args {
            match arg {
                Expr::Literal(Literal::Number(n)) => arg_values.push(Value::Int(*n)),
                Expr::Literal(Literal::Float(f)) => arg_values.push(Value::Float(*f)),
                Expr::Literal(Literal::Boolean(b)) => arg_values.push(Value::Bool(*b)),
                Expr::Literal(Literal::Text(s)) => arg_values.push(Value::Text(*s)),
                Expr::Literal(Literal::Nothing) => arg_values.push(Value::Nothing),
                _ => return None,
            }
        }

        let mut locals = HashMap::new();
        for (param, val) in func.params.iter().zip(arg_values) {
            locals.insert(*param, val);
        }

        let mut steps = 0;
        let result = eval_block(func.body, &mut locals, &env.funcs, &mut steps, 0, interner)?;
        value_to_expr(&result, expr_arena)
    } else {
        None
    }
}

fn ctfe_expr<'a>(
    expr: &'a Expr<'a>,
    env: &CtfeEnv<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
) -> &'a Expr<'a> {
    // Try CTFE on this expression first
    if let Some(result) = try_ctfe_expr(expr, env, expr_arena, interner) {
        return result;
    }
    // Recursively process sub-expressions
    match expr {
        Expr::BinaryOp { op, left, right } => {
            let fl = ctfe_expr(left, env, expr_arena, interner);
            let fr = ctfe_expr(right, env, expr_arena, interner);
            if std::ptr::eq(fl, *left) && std::ptr::eq(fr, *right) {
                expr
            } else {
                expr_arena.alloc(Expr::BinaryOp { op: *op, left: fl, right: fr })
            }
        }
        Expr::Call { function, args } => {
            let fa: Vec<&'a Expr<'a>> = args.iter().map(|a| ctfe_expr(a, env, expr_arena, interner)).collect();
            let changed = fa.iter().zip(args.iter()).any(|(f, o)| !std::ptr::eq(*f, *o));
            let new_expr = if changed {
                expr_arena.alloc(Expr::Call { function: *function, args: fa })
            } else {
                expr
            };
            // Try CTFE again after processing args
            if let Some(result) = try_ctfe_expr(new_expr, env, expr_arena, interner) {
                result
            } else {
                new_expr
            }
        }
        Expr::Not { operand } => {
            let fo = ctfe_expr(operand, env, expr_arena, interner);
            if std::ptr::eq(fo, *operand) { expr }
            else { expr_arena.alloc(Expr::Not { operand: fo }) }
        }
        _ => expr,
    }
}

pub fn ctfe_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let funcs = collect_functions(&stmts);
    let env = CtfeEnv { funcs };
    let mut result = Vec::with_capacity(stmts.len());

    for stmt in stmts {
        result.push(ctfe_stmt(stmt, &env, expr_arena, stmt_arena, interner));
    }

    result
}

fn ctfe_stmt<'a>(
    stmt: Stmt<'a>,
    env: &CtfeEnv<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Stmt<'a> {
    match stmt {
        Stmt::Let { var, ty, value, mutable } => Stmt::Let {
            var, ty, mutable,
            value: ctfe_expr(value, env, expr_arena, interner),
        },
        Stmt::Set { target, value } => Stmt::Set {
            target,
            value: ctfe_expr(value, env, expr_arena, interner),
        },
        Stmt::Return { value } => Stmt::Return {
            value: value.map(|v| ctfe_expr(v, env, expr_arena, interner)),
        },
        Stmt::Show { object, recipient } => Stmt::Show {
            object: ctfe_expr(object, env, expr_arena, interner),
            recipient,
        },
        Stmt::If { cond, then_block, else_block } => Stmt::If {
            cond: ctfe_expr(cond, env, expr_arena, interner),
            then_block: ctfe_block(then_block, env, expr_arena, stmt_arena, interner),
            else_block: else_block.map(|eb| ctfe_block(eb, env, expr_arena, stmt_arena, interner)),
        },
        Stmt::While { cond, body, decreasing } => Stmt::While {
            cond: ctfe_expr(cond, env, expr_arena, interner),
            body: ctfe_block(body, env, expr_arena, stmt_arena, interner),
            decreasing,
        },
        Stmt::FunctionDef { name, params, generics, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => {
            Stmt::FunctionDef {
                name, params, generics,
                body: ctfe_block(body, env, expr_arena, stmt_arena, interner),
                return_type, is_native, native_path, is_exported, export_target, opt_flags,
            }
        }
        Stmt::Call { function, args } => Stmt::Call {
            function,
            args: args.into_iter().map(|a| ctfe_expr(a, env, expr_arena, interner)).collect(),
        },
        Stmt::Push { value, collection } => Stmt::Push {
            value: ctfe_expr(value, env, expr_arena, interner),
            collection,
        },
        Stmt::RuntimeAssert { condition } => Stmt::RuntimeAssert {
            condition: ctfe_expr(condition, env, expr_arena, interner),
        },
        other => other,
    }
}

fn ctfe_block<'a>(
    block: &'a [Stmt<'a>],
    env: &CtfeEnv<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> &'a [Stmt<'a>] {
    let processed: Vec<Stmt<'a>> = block.iter().cloned()
        .map(|s| ctfe_stmt(s, env, expr_arena, stmt_arena, interner))
        .collect();
    stmt_arena.alloc_slice(processed)
}
