//! Compile-Time Function Evaluation (CTFE).
//!
//! Evaluates pure function calls with all-literal arguments at compile time,
//! replacing the call with its result. Step-limited to prevent infinite loops.

use std::collections::{HashMap, HashSet};

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

fn is_pure_stmt(stmt: &Stmt, locals: &HashSet<Symbol>) -> bool {
    match stmt {
        Stmt::Let { value, .. } => is_pure_expr(value),
        // A `Set` to one of the function's OWN locals is pure; a `Set` to anything else is a write to an
        // enclosing/global variable — an EFFECT that escapes the call, so the function is NOT comptime-pure.
        // (Without this, `bump()` whose body is `Set counter to counter + 1` — writing the global `counter` —
        // is wrongly deemed pure, and CTFE folds `Let counter be 0; …bump()…; Show counter` to `Show 0`.)
        Stmt::Set { target, value } => locals.contains(target) && is_pure_expr(value),
        Stmt::Return { value } => value.map_or(true, |v| is_pure_expr(v)),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            is_pure_expr(cond)
                && then_block.iter().all(|s| is_pure_stmt(s, locals))
                && else_block.map_or(true, |eb| eb.iter().all(|s| is_pure_stmt(s, locals)))
        }
        Stmt::While { cond, body, .. } => {
            is_pure_expr(cond) && body.iter().all(|s| is_pure_stmt(s, locals))
        }
        Stmt::Call { .. } => true, // Allow calls (purity checked transitively)
        // IO, mutations to collections, escape blocks are impure
        Stmt::Show { .. }
        | Stmt::Escape { .. }
        | Stmt::Push { .. }
        | Stmt::Pop { .. }
        | Stmt::Add { .. }
        | Stmt::Remove { .. }
        | Stmt::SetIndex { .. }
        | Stmt::SetField { .. }
        | Stmt::Give { .. }
        | Stmt::WriteFile { .. }
        | Stmt::SendMessage { .. }
        | Stmt::StreamMessage { .. }
        | Stmt::Sleep { .. }
        | Stmt::Spawn { .. }
        | Stmt::Break => false,
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

/// Every `Let`-declared symbol in a statement (recursively through nested blocks) — a function's genuine
/// locals, distinct from the globals it may `Set`.
fn collect_let_decls(stmt: &Stmt, decls: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Let { var, .. } => {
            decls.insert(*var);
        }
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            for s in *then_block {
                collect_let_decls(s, decls);
            }
            if let Some(eb) = else_block {
                for s in *eb {
                    collect_let_decls(s, decls);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
            for s in *body {
                collect_let_decls(s, decls);
            }
        }
        Stmt::Inspect { arms, .. } => {
            for arm in arms {
                for s in arm.body {
                    collect_let_decls(s, decls);
                }
            }
        }
        _ => {}
    }
}

fn collect_functions<'a>(stmts: &[Stmt<'a>]) -> HashMap<Symbol, FuncInfo<'a>> {
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
            if *is_native {
                continue;
            }
            let param_symbols: Vec<Symbol> = params.iter().map(|(name, _)| *name).collect();
            // The function's own locals — params + every `Let`-declared var. A `Set` to anything else writes
            // an enclosing/global variable, which makes the function impure (not comptime-evaluable).
            let mut locals: HashSet<Symbol> = param_symbols.iter().copied().collect();
            for s in *body {
                collect_let_decls(s, &mut locals);
            }
            let is_pure = body.iter().all(|s| is_pure_stmt(s, &locals));
            if is_pure {
                funcs.insert(
                    *name,
                    FuncInfo {
                        params: param_symbols,
                        body,
                        is_pure,
                    },
                );
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
            if !func.is_pure {
                return None;
            }
            let mut arg_values = Vec::new();
            for arg in args {
                arg_values.push(eval_expr(arg, locals, funcs, steps, depth, interner)?);
            }
            if arg_values.len() != func.params.len() {
                return None;
            }
            let mut call_locals = HashMap::new();
            for (param, val) in func.params.iter().zip(arg_values) {
                call_locals.insert(*param, val);
            }
            eval_block(
                func.body,
                &mut call_locals,
                funcs,
                steps,
                depth + 1,
                interner,
            )
        }
        _ => None,
    }
}

fn eval_binop(op: BinaryOpKind, lv: Value, rv: Value, interner: &mut Interner) -> Option<Value> {
    match (op, &lv, &rv) {
        // Fold ONLY when the result fits i64; on overflow return None so the exact
        // (promoting) runtime computes the BigInt — never bake a wrapped constant.
        (BinaryOpKind::Add, Value::Int(a), Value::Int(b)) => a.checked_add(*b).map(Value::Int),
        (BinaryOpKind::Subtract, Value::Int(a), Value::Int(b)) => a.checked_sub(*b).map(Value::Int),
        (BinaryOpKind::Multiply, Value::Int(a), Value::Int(b)) => a.checked_mul(*b).map(Value::Int),
        (BinaryOpKind::Divide, Value::Int(a), Value::Int(b)) if *b != 0 => {
            a.checked_div(*b).map(Value::Int)
        }
        (BinaryOpKind::Modulo, Value::Int(a), Value::Int(b)) if *b != 0 => {
            a.checked_rem(*b).map(Value::Int)
        }
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
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
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

// ── ASCII-class bitmask (Opt::AsciiClassMask) ────────────────────────────────
// A pure `Int -> Bool` predicate whose true-set lies entirely in the byte domain [0,256) collapses
// to a 256-bit membership mask (built by CTFE-evaluating it over 0..256). Soundness for the WHOLE
// i64 domain rests on: `c` is used only as the direct operand of a comparison against an integer
// constant in [0,256). Then the predicate is piecewise-constant with every breakpoint in range, so
// its value on all of (-inf,0) equals `pred(-1)` and on all of [256,inf) equals `pred(256)`; verify
// both are false and the guarded emit `(0<=c<256) && maskbit` is exact everywhere.

fn is_cmp_op(op: BinaryOpKind) -> bool {
    matches!(
        op,
        BinaryOpKind::Lt
            | BinaryOpKind::Gt
            | BinaryOpKind::LtEq
            | BinaryOpKind::GtEq
            | BinaryOpKind::Eq
            | BinaryOpKind::NotEq
    )
}

/// Every appearance of `p` in `e` must be the direct operand of a comparison against an integer
/// literal in `[0,256)`. Any other use — arithmetic on `p`, `p` as a call argument, a bare read —
/// breaks the piecewise-constant-with-in-range-breakpoints property, so we return false (bail).
/// Conservative: an expression form we don't model returns false (sound: we just skip the mask).
fn param_uses_safe_in_expr(e: &Expr, p: Symbol) -> bool {
    let is_p = |x: &Expr| matches!(x, Expr::Identifier(s) if *s == p);
    let in_range =
        |x: &Expr| matches!(x, Expr::Literal(Literal::Number(k)) if (0..256).contains(k));
    match e {
        Expr::BinaryOp { op, left, right } if is_cmp_op(*op) => {
            if (is_p(left) && in_range(right)) || (in_range(left) && is_p(right)) {
                true
            } else {
                param_uses_safe_in_expr(left, p) && param_uses_safe_in_expr(right, p)
            }
        }
        Expr::Identifier(s) => *s != p,
        Expr::Literal(_) | Expr::OptionNone => true,
        Expr::BinaryOp { left, right, .. } => {
            param_uses_safe_in_expr(left, p) && param_uses_safe_in_expr(right, p)
        }
        Expr::Not { operand } => param_uses_safe_in_expr(operand, p),
        Expr::Call { args, .. } => args.iter().all(|a| param_uses_safe_in_expr(a, p)),
        _ => false,
    }
}

fn param_uses_safe_in_stmt(s: &Stmt, p: Symbol) -> bool {
    match s {
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            param_uses_safe_in_expr(cond, p)
                && then_block.iter().all(|s| param_uses_safe_in_stmt(s, p))
                && else_block.map_or(true, |eb| eb.iter().all(|s| param_uses_safe_in_stmt(s, p)))
        }
        Stmt::Return { value } => value.map_or(true, |v| param_uses_safe_in_expr(v, p)),
        Stmt::Let { value, .. } => param_uses_safe_in_expr(value, p),
        Stmt::Set { value, .. } => param_uses_safe_in_expr(value, p),
        Stmt::While { cond, body, .. } => {
            param_uses_safe_in_expr(cond, p) && body.iter().all(|s| param_uses_safe_in_stmt(s, p))
        }
        _ => false,
    }
}

/// A self-contained Int/Bool value — the only kinds a classification predicate produces. Keeping this
/// separate from `Value` lets the predicate evaluator run WITHOUT an interner (Int/Bool ops neither
/// resolve nor intern; calls resolve by `Symbol` through `funcs`), which is what codegen needs.
#[derive(Clone, Copy)]
enum PVal {
    Int(i64),
    Bool(bool),
}

enum PFlow {
    Next,
    Ret(PVal),
}

fn peval_binop(op: BinaryOpKind, l: PVal, r: PVal) -> Option<PVal> {
    use BinaryOpKind::*;
    match (op, l, r) {
        (Add, PVal::Int(a), PVal::Int(b)) => a.checked_add(b).map(PVal::Int),
        (Subtract, PVal::Int(a), PVal::Int(b)) => a.checked_sub(b).map(PVal::Int),
        (Multiply, PVal::Int(a), PVal::Int(b)) => a.checked_mul(b).map(PVal::Int),
        (Divide, PVal::Int(a), PVal::Int(b)) if b != 0 => a.checked_div(b).map(PVal::Int),
        (Modulo, PVal::Int(a), PVal::Int(b)) if b != 0 => a.checked_rem(b).map(PVal::Int),
        (Eq, PVal::Int(a), PVal::Int(b)) => Some(PVal::Bool(a == b)),
        (NotEq, PVal::Int(a), PVal::Int(b)) => Some(PVal::Bool(a != b)),
        (Lt, PVal::Int(a), PVal::Int(b)) => Some(PVal::Bool(a < b)),
        (Gt, PVal::Int(a), PVal::Int(b)) => Some(PVal::Bool(a > b)),
        (LtEq, PVal::Int(a), PVal::Int(b)) => Some(PVal::Bool(a <= b)),
        (GtEq, PVal::Int(a), PVal::Int(b)) => Some(PVal::Bool(a >= b)),
        (And, PVal::Bool(a), PVal::Bool(b)) => Some(PVal::Bool(a && b)),
        (Or, PVal::Bool(a), PVal::Bool(b)) => Some(PVal::Bool(a || b)),
        (Eq, PVal::Bool(a), PVal::Bool(b)) => Some(PVal::Bool(a == b)),
        (NotEq, PVal::Bool(a), PVal::Bool(b)) => Some(PVal::Bool(a != b)),
        _ => None,
    }
}

fn peval_expr(
    e: &Expr,
    locals: &HashMap<Symbol, PVal>,
    funcs: &HashMap<Symbol, FuncInfo>,
    steps: &mut usize,
    depth: usize,
) -> Option<PVal> {
    if *steps >= MAX_STEPS || depth >= MAX_DEPTH {
        return None;
    }
    *steps += 1;
    match e {
        Expr::Literal(Literal::Number(n)) => Some(PVal::Int(*n)),
        Expr::Literal(Literal::Boolean(b)) => Some(PVal::Bool(*b)),
        Expr::Identifier(s) => locals.get(s).copied(),
        Expr::Not { operand } => match peval_expr(operand, locals, funcs, steps, depth)? {
            PVal::Bool(b) => Some(PVal::Bool(!b)),
            _ => None,
        },
        Expr::BinaryOp { op, left, right } => {
            let l = peval_expr(left, locals, funcs, steps, depth)?;
            let r = peval_expr(right, locals, funcs, steps, depth)?;
            peval_binop(*op, l, r)
        }
        Expr::Call { function, args } => {
            let f = funcs.get(function)?;
            if !f.is_pure || args.len() != f.params.len() {
                return None;
            }
            let mut call_locals = HashMap::new();
            for (param, a) in f.params.iter().zip(args.iter()) {
                call_locals.insert(*param, peval_expr(a, locals, funcs, steps, depth)?);
            }
            match peval_block(f.body, &mut call_locals, funcs, steps, depth + 1)? {
                PFlow::Ret(v) => Some(v),
                PFlow::Next => None,
            }
        }
        _ => None,
    }
}

fn peval_block(
    stmts: &[Stmt],
    locals: &mut HashMap<Symbol, PVal>,
    funcs: &HashMap<Symbol, FuncInfo>,
    steps: &mut usize,
    depth: usize,
) -> Option<PFlow> {
    for s in stmts {
        if *steps >= MAX_STEPS {
            return None;
        }
        *steps += 1;
        match s {
            Stmt::Return { value } => {
                let v = peval_expr(value.as_ref()?, locals, funcs, steps, depth)?;
                return Some(PFlow::Ret(v));
            }
            Stmt::Let { var, value, .. } => {
                let v = peval_expr(value, locals, funcs, steps, depth)?;
                locals.insert(*var, v);
            }
            // The purity check (collect_functions) already guarantees a `Set` targets a local.
            Stmt::Set { target, value } => {
                let v = peval_expr(value, locals, funcs, steps, depth)?;
                locals.insert(*target, v);
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => match peval_expr(cond, locals, funcs, steps, depth)? {
                PVal::Bool(true) => {
                    if let PFlow::Ret(v) = peval_block(then_block, locals, funcs, steps, depth)? {
                        return Some(PFlow::Ret(v));
                    }
                }
                PVal::Bool(false) => {
                    if let Some(eb) = else_block {
                        if let PFlow::Ret(v) = peval_block(eb, locals, funcs, steps, depth)? {
                            return Some(PFlow::Ret(v));
                        }
                    }
                }
                _ => return None,
            },
            Stmt::While { cond, body, .. } => loop {
                if *steps >= MAX_STEPS {
                    return None;
                }
                match peval_expr(cond, locals, funcs, steps, depth)? {
                    PVal::Bool(true) => {
                        if let PFlow::Ret(v) = peval_block(body, locals, funcs, steps, depth)? {
                            return Some(PFlow::Ret(v));
                        }
                    }
                    PVal::Bool(false) => break,
                    _ => return None,
                }
            },
            _ => return None,
        }
    }
    Some(PFlow::Next)
}

fn eval_pred(body: &[Stmt], p: Symbol, c: i64, funcs: &HashMap<Symbol, FuncInfo>) -> Option<bool> {
    let mut locals = HashMap::new();
    locals.insert(p, PVal::Int(c));
    let mut steps = 0usize;
    match peval_block(body, &mut locals, funcs, &mut steps, 1)? {
        PFlow::Ret(PVal::Bool(b)) => Some(b),
        _ => None,
    }
}

/// Is the declared parameter type `Int`? The mask emitter rewrites the signature to
/// `fn name(c: i64) -> bool`, so a predicate over anything else would be given a parameter of a
/// type it does not have.
fn is_int_param(ty: &crate::ast::stmt::TypeExpr, interner: &Interner) -> bool {
    use crate::ast::stmt::TypeExpr;
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            matches!(interner.resolve(*sym), "Int" | "Nat")
        }
        TypeExpr::Refinement { base, .. } => is_int_param(base, interner),
        _ => false,
    }
}

/// For each pure single-`Int`-param predicate that classifies `c` purely by in-range comparison
/// (see the soundness note above), returns its 256-bit membership mask (`[u64;4]`, bit `c` set iff
/// `pred(c)` is true). Codegen replaces the predicate body with the branchless mask test.
///
/// ★ THE PARAMETER'S DECLARED TYPE IS PART OF THE CANDIDATE TEST, not just its arity. The doc above
/// has always said "single-`Int`-param predicate" while the code checked only `params.len() != 1`,
/// so the pass also fired on a `Bool`-param predicate — and the emitter
/// (`codegen/program.rs`) rewrites the signature to `fn name(c: i64) -> bool` unconditionally.
/// `## To test (x: Bool) -> Bool: Let result be x and false.` folds to `Return false`, which is
/// trivially param-independent, CTFEs false across the whole domain, and was replaced by a 256-byte
/// all-zero table whose parameter had changed type. It survived only because that program's single
/// call site was itself const-folded; a caller passing a runtime `Bool` would not have built at all.
/// Fails `phase_optimize::fold_bool_and_false`.
///
/// A predicate that does not READ its parameter is rejected for a second, independent reason: its
/// mask is a constant, so the table is pure pessimization — 256 bytes and a load in place of a
/// literal.
pub(crate) fn ascii_class_masks(stmts: &[Stmt], interner: &Interner) -> HashMap<Symbol, [u64; 4]> {
    let funcs = collect_functions(stmts);
    let mut out = HashMap::new();
    for stmt in stmts {
        let Stmt::FunctionDef {
            name,
            params,
            body,
            is_native,
            ..
        } = stmt
        else {
            continue;
        };
        if *is_native || params.len() != 1 || !funcs.contains_key(name) {
            continue;
        }
        if !is_int_param(&params[0].1, interner) {
            continue;
        }
        let body: &[Stmt] = body;
        let p = params[0].0;
        if !body.iter().all(|s| param_uses_safe_in_stmt(s, p)) {
            continue;
        }
        let mut mask = [0u64; 4];
        let mut ok = true;
        for c in 0..256i64 {
            match eval_pred(body, p, c, &funcs) {
                Some(true) => mask[(c >> 6) as usize] |= 1u64 << (c & 63),
                Some(false) => {}
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        // A constant answer is not a classification. `[0,0,0,0]` and an all-ones mask both mean the
        // predicate ignored its parameter, and a table that says what a literal says is slower than
        // the literal.
        if mask == [0u64; 4] || mask == [u64::MAX; 4] {
            continue;
        }
        // Soundness boundaries: the whole below/above regions must be false (piecewise-constant with
        // every breakpoint in [0,256), so these two points witness the entire out-of-range domain).
        if eval_pred(body, p, -1, &funcs) != Some(false)
            || eval_pred(body, p, 256, &funcs) != Some(false)
        {
            continue;
        }
        out.insert(*name, mask);
    }
    out
}

fn try_ctfe_expr<'a>(
    expr: &'a Expr<'a>,
    env: &CtfeEnv<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
) -> Option<&'a Expr<'a>> {
    if let Expr::Call { function, args } = expr {
        let func = env.funcs.get(function)?;
        if !func.is_pure {
            return None;
        }
        if args.len() != func.params.len() {
            return None;
        }

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
                expr_arena.alloc(Expr::BinaryOp {
                    op: *op,
                    left: fl,
                    right: fr,
                })
            }
        }
        Expr::Call { function, args } => {
            let fa: Vec<&'a Expr<'a>> = args
                .iter()
                .map(|a| ctfe_expr(a, env, expr_arena, interner))
                .collect();
            let changed = fa
                .iter()
                .zip(args.iter())
                .any(|(f, o)| !std::ptr::eq(*f, *o));
            let new_expr = if changed {
                expr_arena.alloc(Expr::Call {
                    function: *function,
                    args: fa,
                })
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
            if std::ptr::eq(fo, *operand) {
                expr
            } else {
                expr_arena.alloc(Expr::Not { operand: fo })
            }
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
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => Stmt::Let {
            var,
            ty,
            mutable,
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
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => Stmt::If {
            cond: ctfe_expr(cond, env, expr_arena, interner),
            then_block: ctfe_block(then_block, env, expr_arena, stmt_arena, interner),
            else_block: else_block.map(|eb| ctfe_block(eb, env, expr_arena, stmt_arena, interner)),
        },
        Stmt::While {
            cond,
            body,
            decreasing,
        } => Stmt::While {
            cond: ctfe_expr(cond, env, expr_arena, interner),
            body: ctfe_block(body, env, expr_arena, stmt_arena, interner),
            decreasing,
        },
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
        } => Stmt::FunctionDef {
            name,
            params,
            generics,
            body: ctfe_block(body, env, expr_arena, stmt_arena, interner),
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
            opt_flags,
        },
        Stmt::Call { function, args } => Stmt::Call {
            function,
            args: args
                .into_iter()
                .map(|a| ctfe_expr(a, env, expr_arena, interner))
                .collect(),
        },
        Stmt::Push { value, collection } => Stmt::Push {
            value: ctfe_expr(value, env, expr_arena, interner),
            collection,
        },
        Stmt::RuntimeAssert {
            condition,
            hard,
            dim,
            reason,
        } => Stmt::RuntimeAssert {
            condition: ctfe_expr(condition, env, expr_arena, interner),
            hard,
            dim,
            reason: reason.map(|r| ctfe_expr(r, env, expr_arena, interner)),
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
    let processed: Vec<Stmt<'a>> = block
        .iter()
        .cloned()
        .map(|s| ctfe_stmt(s, env, expr_arena, stmt_arena, interner))
        .collect();
    stmt_arena.alloc_slice(processed)
}

// ===========================================================================
// Comptime table-ization.
//
// A pure function called with arguments drawn from a small, statically-bounded
// domain (`f(i mod K + C)`, `binom(30, i mod 16)`, …) is replaced by a lookup
// into a table precomputed at compile time via CTFE. LOGOS then does a load
// where C recomputes — and the i64-valued result keeps the surrounding
// accumulator on its fast path. Gated by `Opt::Comptime` (it IS compile-time
// evaluation). Sound by construction: `eval_binop` bails on i64 overflow, so a
// value that would exceed i64 is never tabled — the exact runtime computes it.
// ===========================================================================

/// Largest arg-domain product we will materialize as a table.
const MAX_TABLE_ENTRIES: i64 = 4096;

/// A syntactic `[lo, hi]` range for a bounded integer expression: literals,
/// `e mod K` (→ `[0, K-1]` for K > 0), and constant add/sub shifts. `None` when
/// not provably bounded.
fn derive_range(e: &Expr) -> Option<(i64, i64)> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some((*n, *n)),
        Expr::BinaryOp { op, left, right } => match op {
            BinaryOpKind::Modulo => match right {
                Expr::Literal(Literal::Number(k)) if *k > 0 => Some((0, k - 1)),
                _ => None,
            },
            BinaryOpKind::Add => {
                let (a, b) = derive_range(left)?;
                let (c, d) = derive_range(right)?;
                Some((a.checked_add(c)?, b.checked_add(d)?))
            }
            BinaryOpKind::Subtract => {
                let (a, b) = derive_range(left)?;
                let (c, d) = derive_range(right)?;
                Some((a.checked_sub(d)?, b.checked_sub(c)?))
            }
            _ => None,
        },
        _ => None,
    }
}

/// CTFE-evaluate `func` at integer `args`; `Some` only when it runs to a plain
/// `Int` within budget (and never overflowed i64 — `eval_binop` guarantees it).
fn eval_fn_int<'a>(
    func: &FuncInfo<'a>,
    args: &[i64],
    funcs: &HashMap<Symbol, FuncInfo<'a>>,
    interner: &mut Interner,
) -> Option<i64> {
    if func.params.len() != args.len() {
        return None;
    }
    let mut locals = HashMap::new();
    for (p, v) in func.params.iter().zip(args) {
        locals.insert(*p, Value::Int(*v));
    }
    let mut steps = 0usize;
    match eval_block(func.body, &mut locals, funcs, &mut steps, 0, interner)? {
        Value::Int(n) => Some(n),
        _ => None,
    }
}

/// If `call` is a table-izable pure-function call, return `(table values in
/// flattened domain order, the 1-indexed lookup-index expression)`.
fn try_tableize_call<'a>(
    function: Symbol,
    args: &[&'a Expr<'a>],
    funcs: &HashMap<Symbol, FuncInfo<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
) -> Option<(Vec<i64>, &'a Expr<'a>)> {
    let func = funcs.get(&function)?;
    if !func.is_pure || func.params.len() != args.len() || args.is_empty() {
        return None;
    }
    let ranges: Vec<(i64, i64)> = args
        .iter()
        .map(|a| derive_range(a))
        .collect::<Option<_>>()?;
    let widths: Vec<i64> = ranges.iter().map(|(lo, hi)| hi - lo + 1).collect();
    if widths.iter().any(|w| *w <= 0) {
        return None;
    }
    // At least one argument must actually vary (else the call is a single
    // constant that plain CTFE already folds).
    if widths.iter().all(|w| *w == 1) {
        return None;
    }
    let total: i64 = widths.iter().try_fold(1i64, |acc, w| acc.checked_mul(*w))?;
    if total > MAX_TABLE_ENTRIES {
        return None;
    }
    // Strides for the flattened (arg 0 = most significant) layout.
    let n = widths.len();
    let stride = |i: usize| -> i64 { widths[i + 1..].iter().product() };

    // Precompute every entry; bail (no table) if any point escapes i64 or CTFE.
    let mut values = Vec::with_capacity(total as usize);
    for flat in 0..total {
        let mut rem = flat;
        let mut pt = Vec::with_capacity(n);
        for i in 0..n {
            let s = stride(i);
            pt.push(ranges[i].0 + rem / s);
            rem %= s;
        }
        values.push(eval_fn_int(func, &pt, funcs, interner)?);
    }

    // Build `Σ (argᵢ - loᵢ) * strideᵢ + 1` over the varying args (const args add 0).
    let lit = |v: i64| -> &'a Expr<'a> { expr_arena.alloc(Expr::Literal(Literal::Number(v))) };
    let mut terms: Vec<&'a Expr<'a>> = Vec::new();
    for i in 0..n {
        if widths[i] == 1 {
            continue;
        }
        let (lo, _) = ranges[i];
        let shifted: &Expr = if lo == 0 {
            args[i]
        } else {
            expr_arena.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Subtract,
                left: args[i],
                right: lit(lo),
            })
        };
        let s = stride(i);
        let term: &Expr = if s == 1 {
            shifted
        } else {
            expr_arena.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Multiply,
                left: shifted,
                right: lit(s),
            })
        };
        terms.push(term);
    }
    let mut idx = terms[0];
    for t in &terms[1..] {
        idx = expr_arena.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left: idx,
            right: t,
        });
    }
    let one_indexed = expr_arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: idx,
        right: lit(1),
    });
    Some((values, one_indexed))
}

/// Rewrite table-izable calls inside an expression to table lookups, recording
/// each new table in `tables`.
fn tableize_rewrite_expr<'a>(
    e: &'a Expr<'a>,
    funcs: &HashMap<Symbol, FuncInfo<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
    tables: &mut Vec<(Symbol, Vec<i64>)>,
) -> &'a Expr<'a> {
    match e {
        Expr::Call { function, args } => {
            if let Some((values, index)) =
                try_tableize_call(*function, args, funcs, expr_arena, interner)
            {
                let sym = interner.intern(&format!("__ctfe_tbl_{}", tables.len()));
                tables.push((sym, values));
                return expr_arena.alloc(Expr::Index {
                    collection: expr_arena.alloc(Expr::Identifier(sym)),
                    index,
                });
            }
            let new_args: Vec<&'a Expr<'a>> = args
                .iter()
                .map(|a| tableize_rewrite_expr(a, funcs, expr_arena, interner, tables))
                .collect();
            expr_arena.alloc(Expr::Call {
                function: *function,
                args: new_args,
            })
        }
        Expr::BinaryOp { op, left, right } => expr_arena.alloc(Expr::BinaryOp {
            op: *op,
            left: tableize_rewrite_expr(left, funcs, expr_arena, interner, tables),
            right: tableize_rewrite_expr(right, funcs, expr_arena, interner, tables),
        }),
        Expr::Not { operand } => expr_arena.alloc(Expr::Not {
            operand: tableize_rewrite_expr(operand, funcs, expr_arena, interner, tables),
        }),
        Expr::Index { collection, index } => expr_arena.alloc(Expr::Index {
            collection: tableize_rewrite_expr(collection, funcs, expr_arena, interner, tables),
            index: tableize_rewrite_expr(index, funcs, expr_arena, interner, tables),
        }),
        _ => e,
    }
}

/// Rewrite table-izable calls inside a statement (and nested blocks).
fn tableize_rewrite_stmt<'a>(
    s: &Stmt<'a>,
    funcs: &HashMap<Symbol, FuncInfo<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    tables: &mut Vec<(Symbol, Vec<i64>)>,
) -> Stmt<'a> {
    let re = |e, t: &mut Vec<(Symbol, Vec<i64>)>, it: &mut Interner| {
        tableize_rewrite_expr(e, funcs, expr_arena, it, t)
    };
    match s {
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => Stmt::Let {
            var: *var,
            ty: *ty,
            value: re(value, tables, interner),
            mutable: *mutable,
        },
        Stmt::Set { target, value } => Stmt::Set {
            target: *target,
            value: re(value, tables, interner),
        },
        Stmt::Return { value } => Stmt::Return {
            value: value.map(|v| re(v, tables, interner)),
        },
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            let cond = re(cond, tables, interner);
            let tb: Vec<Stmt<'a>> = then_block
                .iter()
                .map(|s| tableize_rewrite_stmt(s, funcs, expr_arena, stmt_arena, interner, tables))
                .collect();
            let eb = else_block.map(|b| {
                let v: Vec<Stmt<'a>> = b
                    .iter()
                    .map(|s| {
                        tableize_rewrite_stmt(s, funcs, expr_arena, stmt_arena, interner, tables)
                    })
                    .collect();
                &*stmt_arena.alloc_slice(v)
            });
            Stmt::If {
                cond,
                then_block: stmt_arena.alloc_slice(tb),
                else_block: eb,
            }
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            let cond = re(cond, tables, interner);
            let b: Vec<Stmt<'a>> = body
                .iter()
                .map(|s| tableize_rewrite_stmt(s, funcs, expr_arena, stmt_arena, interner, tables))
                .collect();
            Stmt::While {
                cond,
                body: stmt_arena.alloc_slice(b),
                decreasing: *decreasing,
            }
        }
        other => other.clone(),
    }
}

/// Comptime table-ization pass entry point (gated by `Opt::Comptime`).
pub fn tableize_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let funcs = collect_functions(&stmts);
    if funcs.is_empty() {
        return stmts;
    }
    let mut tables: Vec<(Symbol, Vec<i64>)> = Vec::new();
    let mut fn_defs: Vec<Stmt<'a>> = Vec::new();
    let mut main_body: Vec<Stmt<'a>> = Vec::new();
    for s in &stmts {
        if matches!(s, Stmt::FunctionDef { .. }) {
            fn_defs.push(s.clone());
        } else {
            main_body.push(tableize_rewrite_stmt(
                s,
                &funcs,
                expr_arena,
                stmt_arena,
                interner,
                &mut tables,
            ));
        }
    }
    if tables.is_empty() {
        return stmts;
    }
    // Hoist each precomputed table as a `Let __tbl be [v0, v1, …]` between the
    // function definitions and the (rewritten) main body.
    let mut out = fn_defs;
    for (sym, values) in &tables {
        let items: Vec<&'a Expr<'a>> = values
            .iter()
            .map(|v| &*expr_arena.alloc(Expr::Literal(Literal::Number(*v))))
            .collect();
        let list = expr_arena.alloc(Expr::List(items));
        out.push(Stmt::Let {
            var: *sym,
            ty: None,
            value: list,
            mutable: false,
        });
    }
    out.extend(main_body);
    out
}
