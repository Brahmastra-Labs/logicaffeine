//! Group 4 — DEFUNCTIONALIZATION (Reynolds 1972), stage 1: DIRECT
//! closure elimination.
//!
//! A closure bound to an IMMUTABLE `Let` whose every visible use is a
//! direct call `f(args)` is an illusion: it lifts to a top-level
//! `FunctionDef` whose leading parameters are the captured variables
//! (same symbols — the body needs no rewriting), the captures
//! materialize as immutable SNAPSHOT bindings at the closure's creation
//! point (value semantics preserved by construction), and every call
//! becomes a first-order `Call` passing the snapshots. The closure value
//! — the heap `ClosureValue` box and its cloned environment — never
//! exists, and the residual is plain first-order code every downstream
//! tier (PE, e-graph, VM, JIT) already devours.
//!
//! Everything else FAILS CLOSED: escaping uses (closure passed as a
//! value, stored, returned, referenced inside another closure), mutable
//! rebinding, captures whose declared type cannot be derived, or body
//! constructs outside the analyzed fragment — the closure stays, and the
//! MakeClosure path keeps its exact semantics.

use std::collections::{BTreeMap, HashMap};

use crate::arena::Arena;
use crate::ast::stmt::{ClosureBody, Expr, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

/// An eliminated closure visible in the current scope.
#[derive(Clone)]
struct Lifted {
    fn_name: Symbol,
    snapshots: Vec<Symbol>,
    arity: usize,
}

struct Cx<'a, 'i> {
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &'i mut Interner,
    /// Lifted definitions, hoisted to the program top level at the end.
    lifted: Vec<Stmt<'a>>,
    counter: usize,
    /// Declared return types — a binding initialized by `f(...)` derives
    /// its capture type from f's declaration.
    fn_returns: HashMap<Symbol, &'a TypeExpr<'a>>,
}

pub fn defunctionalize_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mut fn_returns: HashMap<Symbol, &'a TypeExpr<'a>> = HashMap::new();
    for s in &stmts {
        if let Stmt::FunctionDef { name, return_type: Some(ty), .. } = s {
            fn_returns.insert(*name, ty);
        }
    }
    let mut cx = Cx {
        expr_arena,
        stmt_arena,
        interner,
        lifted: Vec::new(),
        counter: 0,
        fn_returns,
    };
    let env: HashMap<Symbol, Lifted> = HashMap::new();
    let tyenv: HashMap<Symbol, &'a TypeExpr<'a>> = HashMap::new();
    let body = cx.rewrite_block(stmts, &env, &tyenv);
    let mut out = cx.lifted;
    out.extend(body);
    out
}

impl<'a> Cx<'a, '_> {
    fn rewrite_block(
        &mut self,
        stmts: Vec<Stmt<'a>>,
        env_in: &HashMap<Symbol, Lifted>,
        tyenv_in: &HashMap<Symbol, &'a TypeExpr<'a>>,
    ) -> Vec<Stmt<'a>> {
        let mut env = env_in.clone();
        let mut tyenv = tyenv_in.clone();
        let mut out: Vec<Stmt<'a>> = Vec::new();
        let mut iter = stmts.into_iter();
        while let Some(s) = iter.next() {
            match s {
                Stmt::Let { var, ty, value, mutable: false }
                    if matches!(value, Expr::Closure { .. }) =>
                {
                    // The remaining statements of THIS block are the
                    // closure's visible region.
                    let rest: Vec<Stmt<'a>> = iter.clone().collect();
                    if let Some(plan) = self.plan_elimination(var, value, &tyenv, &rest) {
                        // Snapshot bindings at the creation point.
                        for (orig, snap) in &plan.snapshot_pairs {
                            out.push(Stmt::Let {
                                var: *snap,
                                ty: None,
                                value: self
                                    .expr_arena
                                    .alloc(Expr::Identifier(*orig)),
                                mutable: false,
                            });
                        }
                        env.insert(
                            var,
                            Lifted {
                                fn_name: plan.fn_name,
                                snapshots: plan
                                    .snapshot_pairs
                                    .iter()
                                    .map(|(_, s)| *s)
                                    .collect(),
                                arity: plan.arity,
                            },
                        );
                        // The original Let vanishes — the closure value
                        // never exists.
                        continue;
                    }
                    // Declined: keep the binding (children still rewrite),
                    // and the name shadows any outer lifted closure.
                    env.remove(&var);
                    let value = self.rewrite_expr(value, &env);
                    if let Some(t) = ty {
                        tyenv.insert(var, t);
                    } else {
                        tyenv.remove(&var);
                    }
                    out.push(Stmt::Let { var, ty, value, mutable: false });
                }
                other => {
                    let rewritten = self.rewrite_stmt(other, &mut env, &mut tyenv);
                    out.push(rewritten);
                }
            }
        }
        out
    }

    fn rewrite_stmt(
        &mut self,
        s: Stmt<'a>,
        env: &mut HashMap<Symbol, Lifted>,
        tyenv: &mut HashMap<Symbol, &'a TypeExpr<'a>>,
    ) -> Stmt<'a> {
        match s {
            Stmt::Let { var, ty, value, mutable } => {
                let value = self.rewrite_expr(value, env);
                // The new binding shadows any same-named lifted closure.
                env.remove(&var);
                // Record the binding's derivable type for capture typing.
                if let Some(t) = ty {
                    tyenv.insert(var, t);
                } else if let Expr::Call { function, .. } = value {
                    match self.fn_returns.get(function) {
                        Some(t) => {
                            tyenv.insert(var, t);
                        }
                        None => {
                            tyenv.remove(&var);
                        }
                    }
                } else {
                    tyenv.remove(&var);
                }
                Stmt::Let { var, ty, value, mutable }
            }
            Stmt::Set { target, value } => {
                let value = self.rewrite_expr(value, env);
                env.remove(&target);
                Stmt::Set { target, value }
            }
            Stmt::Show { object, recipient } => Stmt::Show {
                object: self.rewrite_expr(object, env),
                recipient,
            },
            Stmt::Return { value } => Stmt::Return {
                value: value.map(|v| self.rewrite_expr(v, env)),
            },
            Stmt::RuntimeAssert { condition, hard } => Stmt::RuntimeAssert {
                condition: self.rewrite_expr(condition, env),
                hard,
            },
            Stmt::Push { value, collection } => Stmt::Push {
                value: self.rewrite_expr(value, env),
                collection: self.rewrite_expr(collection, env),
            },
            Stmt::SetIndex { collection, index, value } => Stmt::SetIndex {
                collection: self.rewrite_expr(collection, env),
                index: self.rewrite_expr(index, env),
                value: self.rewrite_expr(value, env),
            },
            Stmt::SetField { object, field, value } => Stmt::SetField {
                object: self.rewrite_expr(object, env),
                field,
                value: self.rewrite_expr(value, env),
            },
            Stmt::Call { function, args } => Stmt::Call {
                function,
                args: args.into_iter().map(|a| self.rewrite_expr(a, env)).collect(),
            },
            Stmt::If { cond, then_block, else_block } => {
                let cond = self.rewrite_expr(cond, env);
                let tb = self.rewrite_nested(then_block, env, tyenv);
                let eb = else_block.map(|b| self.rewrite_nested(b, env, tyenv));
                Stmt::If { cond, then_block: tb, else_block: eb }
            }
            Stmt::While { cond, body, decreasing } => {
                let cond = self.rewrite_expr(cond, env);
                let body = self.rewrite_nested(body, env, tyenv);
                Stmt::While { cond, body, decreasing }
            }
            Stmt::Repeat { pattern, iterable, body } => {
                let iterable = self.rewrite_expr(iterable, env);
                let body = self.rewrite_nested(body, env, tyenv);
                Stmt::Repeat { pattern, iterable, body }
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
                // Function bodies are their own worlds: params typed,
                // no outer lifted closures visible.
                let inner_env: HashMap<Symbol, Lifted> = HashMap::new();
                let mut inner_ty: HashMap<Symbol, &'a TypeExpr<'a>> = HashMap::new();
                for (p, t) in &params {
                    inner_ty.insert(*p, t);
                }
                let body_vec =
                    self.rewrite_block(body.to_vec(), &inner_env, &inner_ty);
                Stmt::FunctionDef {
                    name,
                    generics,
                    params,
                    body: self.stmt_arena.alloc_slice(body_vec),
                    return_type,
                    is_native,
                    native_path,
                    is_exported,
                    export_target,
                    opt_flags,
                }
            }
            other => other,
        }
    }

    fn rewrite_nested(
        &mut self,
        block: &'a [Stmt<'a>],
        env: &HashMap<Symbol, Lifted>,
        tyenv: &HashMap<Symbol, &'a TypeExpr<'a>>,
    ) -> &'a [Stmt<'a>] {
        let walked = self.rewrite_block(block.to_vec(), env, tyenv);
        self.stmt_arena.alloc_slice(walked)
    }

    fn rewrite_expr(
        &mut self,
        e: &'a Expr<'a>,
        env: &HashMap<Symbol, Lifted>,
    ) -> &'a Expr<'a> {
        match e {
            Expr::CallExpr { callee, args } => {
                if let Expr::Identifier(f) = callee {
                    if let Some(l) = env.get(f) {
                        if args.len() == l.arity {
                            let mut call_args: Vec<&'a Expr<'a>> = l
                                .snapshots
                                .iter()
                                .map(|s| {
                                    &*self.expr_arena.alloc(Expr::Identifier(*s))
                                })
                                .collect();
                            for a in args {
                                call_args.push(self.rewrite_expr(a, env));
                            }
                            return self.expr_arena.alloc(Expr::Call {
                                function: l.fn_name,
                                args: call_args,
                            });
                        }
                    }
                }
                let callee = self.rewrite_expr(callee, env);
                let args: Vec<&'a Expr<'a>> =
                    args.iter().map(|a| self.rewrite_expr(a, env)).collect();
                self.expr_arena.alloc(Expr::CallExpr { callee, args })
            }
            Expr::BinaryOp { op, left, right } => {
                let l = self.rewrite_expr(left, env);
                let r = self.rewrite_expr(right, env);
                if std::ptr::eq(l, *left) && std::ptr::eq(r, *right) {
                    e
                } else {
                    self.expr_arena.alloc(Expr::BinaryOp { op: *op, left: l, right: r })
                }
            }
            Expr::Not { operand } => {
                let o = self.rewrite_expr(operand, env);
                if std::ptr::eq(o, *operand) {
                    e
                } else {
                    self.expr_arena.alloc(Expr::Not { operand: o })
                }
            }
            Expr::Call { function, args } => {
                // Direct closure calls PARSE as named calls (the grammar
                // cannot distinguish them; the interpreter resolves the
                // name to a closure variable at runtime) — this is the
                // primary rewrite site.
                if let Some(l) = env.get(function) {
                    if args.len() == l.arity {
                        let mut call_args: Vec<&'a Expr<'a>> = Vec::new();
                        for s in &l.snapshots {
                            call_args.push(self.expr_arena.alloc(Expr::Identifier(*s)));
                        }
                        let fn_name = l.fn_name;
                        for a in args {
                            call_args.push(self.rewrite_expr(a, env));
                        }
                        return self
                            .expr_arena
                            .alloc(Expr::Call { function: fn_name, args: call_args });
                    }
                }
                let new_args: Vec<&'a Expr<'a>> =
                    args.iter().map(|a| self.rewrite_expr(a, env)).collect();
                let changed =
                    new_args.iter().zip(args.iter()).any(|(n, o)| !std::ptr::eq(*n, *o));
                if changed {
                    self.expr_arena.alloc(Expr::Call { function: *function, args: new_args })
                } else {
                    e
                }
            }
            Expr::Index { collection, index } => {
                let c = self.rewrite_expr(collection, env);
                let i = self.rewrite_expr(index, env);
                if std::ptr::eq(c, *collection) && std::ptr::eq(i, *index) {
                    e
                } else {
                    self.expr_arena.alloc(Expr::Index { collection: c, index: i })
                }
            }
            Expr::Length { collection } => {
                let c = self.rewrite_expr(collection, env);
                if std::ptr::eq(c, *collection) {
                    e
                } else {
                    self.expr_arena.alloc(Expr::Length { collection: c })
                }
            }
            // Everything else passes through verbatim: a lifted closure's
            // name can only appear where the USE SCAN admitted it (direct
            // callee position), so unknown shapes cannot smuggle one out.
            _ => e,
        }
    }

    // ----- elimination planning ------------------------------------------

    fn plan_elimination(
        &mut self,
        var: Symbol,
        closure: &'a Expr<'a>,
        tyenv: &HashMap<Symbol, &'a TypeExpr<'a>>,
        rest: &[Stmt<'a>],
    ) -> Option<Plan> {
        let Expr::Closure { params, body, return_type } = closure else {
            return None;
        };
        // Every visible use must be a direct, arity-matching call.
        if !uses_are_direct_calls(rest, var, params.len()) {
            return None;
        }
        // Free variables of the body, minus parameters — the captures.
        let mut bound: Vec<Symbol> = params.iter().map(|(p, _)| *p).collect();
        let mut free: BTreeMap<u32, Symbol> = BTreeMap::new();
        let ok = match body {
            ClosureBody::Expression(e) => free_vars_expr(e, &mut bound, &mut free),
            ClosureBody::Block(b) => free_vars_block(b, &mut bound, &mut free),
        };
        if !ok {
            return None;
        }
        // Each capture needs a DERIVABLE declared type.
        let mut snapshot_pairs: Vec<(Symbol, Symbol)> = Vec::new();
        let mut lifted_params: Vec<(Symbol, &'a TypeExpr<'a>)> = Vec::new();
        let id = self.counter;
        self.counter += 1;
        for (_, cap) in &free {
            let ty = *tyenv.get(cap)?;
            let snap_name = {
                let cap_s = self.interner.resolve(*cap).to_string();
                self.interner.intern(&format!("__defunc{id}_cap_{cap_s}"))
            };
            snapshot_pairs.push((*cap, snap_name));
            // The lifted parameter KEEPS the captured symbol, so the body
            // needs no rewriting at all.
            lifted_params.push((*cap, ty));
        }
        for (p, t) in params {
            lifted_params.push((*p, *t));
        }
        let fn_name = {
            let var_s = self.interner.resolve(var).to_string();
            self.interner.intern(&format!("__defunc{id}_{var_s}"))
        };
        let body_stmts: Vec<Stmt<'a>> = match body {
            ClosureBody::Expression(e) => vec![Stmt::Return { value: Some(e) }],
            ClosureBody::Block(b) => b.to_vec(),
        };
        // Closures inside the lifted body lift recursively (their world
        // is the lifted function's parameters).
        let inner_env: HashMap<Symbol, Lifted> = HashMap::new();
        let mut inner_ty: HashMap<Symbol, &'a TypeExpr<'a>> = HashMap::new();
        for (p, t) in &lifted_params {
            inner_ty.insert(*p, *t);
        }
        let body_stmts = self.rewrite_block(body_stmts, &inner_env, &inner_ty);
        self.lifted.push(Stmt::FunctionDef {
            name: fn_name,
            generics: Vec::new(),
            params: lifted_params,
            body: self.stmt_arena.alloc_slice(body_stmts),
            return_type: *return_type,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: Default::default(),
        });
        Some(Plan { fn_name, snapshot_pairs, arity: params.len() })
    }
}

struct Plan {
    fn_name: Symbol,
    snapshot_pairs: Vec<(Symbol, Symbol)>,
    arity: usize,
}

// =====================================================================
// Use scanning: every visible occurrence of `var` must be the callee of
// an arity-matching CallExpr. ANYTHING unrecognized fails closed.
// =====================================================================

fn uses_are_direct_calls(stmts: &[Stmt], var: Symbol, arity: usize) -> bool {
    let mut shadowed = false;
    for s in stmts {
        if shadowed {
            return true;
        }
        if !stmt_uses_ok(s, var, arity, &mut shadowed) {
            return false;
        }
    }
    true
}

fn block_uses_ok(stmts: &[Stmt], var: Symbol, arity: usize) -> bool {
    let mut shadowed = false;
    for s in stmts {
        if shadowed {
            return true;
        }
        if !stmt_uses_ok(s, var, arity, &mut shadowed) {
            return false;
        }
    }
    true
}

fn stmt_uses_ok(s: &Stmt, var: Symbol, arity: usize, shadowed: &mut bool) -> bool {
    match s {
        Stmt::Let { var: v, value, .. } => {
            if !expr_uses_ok(value, var, arity) {
                return false;
            }
            if *v == var {
                *shadowed = true;
            }
            true
        }
        Stmt::Set { target, value } => {
            // Rebinding the closure's name disqualifies (only possible
            // for mutable bindings, which never reach here, but a same-
            // named OTHER variable is fine: it shadows nothing — Set
            // requires an existing binding, so this is the closure name
            // itself only when the binding was mutable).
            if *target == var {
                return false;
            }
            expr_uses_ok(value, var, arity)
        }
        Stmt::Show { object, .. } => expr_uses_ok(object, var, arity),
        Stmt::Return { value } => value.map(|v| expr_uses_ok(v, var, arity)).unwrap_or(true),
        Stmt::RuntimeAssert { condition, .. } => expr_uses_ok(condition, var, arity),
        Stmt::Push { value, collection } => {
            expr_uses_ok(value, var, arity) && expr_uses_ok(collection, var, arity)
        }
        Stmt::SetIndex { collection, index, value } => {
            expr_uses_ok(collection, var, arity)
                && expr_uses_ok(index, var, arity)
                && expr_uses_ok(value, var, arity)
        }
        Stmt::SetField { object, value, .. } => {
            expr_uses_ok(object, var, arity) && expr_uses_ok(value, var, arity)
        }
        Stmt::Call { args, .. } => args.iter().all(|a| expr_uses_ok(a, var, arity)),
        Stmt::If { cond, then_block, else_block } => {
            expr_uses_ok(cond, var, arity)
                && block_uses_ok(then_block, var, arity)
                && else_block.map(|b| block_uses_ok(b, var, arity)).unwrap_or(true)
        }
        Stmt::While { cond, body, .. } => {
            expr_uses_ok(cond, var, arity) && block_uses_ok(body, var, arity)
        }
        Stmt::Repeat { iterable, body, .. } => {
            expr_uses_ok(iterable, var, arity) && block_uses_ok(body, var, arity)
        }
        // Top-level function bodies cannot reference enclosing locals.
        Stmt::FunctionDef { .. } => true,
        // Unmodeled statement: admit it only if it cannot mention the
        // name at all — fail closed otherwise.
        other => !stmt_may_mention(other, var),
    }
}

/// Conservative mention check for statements outside the modeled set.
fn stmt_may_mention(_s: &Stmt, _var: Symbol) -> bool {
    // Pop/Add/Remove/Inspect/Zone/Concurrent/... could reference the
    // name in expressions this pass does not enumerate — assume they do.
    true
}

fn expr_uses_ok(e: &Expr, var: Symbol, arity: usize) -> bool {
    match e {
        Expr::Identifier(s) => *s != var,
        Expr::Literal(_) | Expr::OptionNone => true,
        Expr::CallExpr { callee, args } => {
            let callee_ok = match callee {
                Expr::Identifier(s) if *s == var => args.len() == arity,
                other => expr_uses_ok(other, var, arity),
            };
            callee_ok && args.iter().all(|a| expr_uses_ok(a, var, arity))
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_uses_ok(left, var, arity) && expr_uses_ok(right, var, arity)
        }
        Expr::Not { operand } => expr_uses_ok(operand, var, arity),
        Expr::Call { args, .. } => args.iter().all(|a| expr_uses_ok(a, var, arity)),
        Expr::Index { collection, index } => {
            expr_uses_ok(collection, var, arity) && expr_uses_ok(index, var, arity)
        }
        Expr::Slice { collection, start, end } => {
            expr_uses_ok(collection, var, arity)
                && expr_uses_ok(start, var, arity)
                && expr_uses_ok(end, var, arity)
        }
        Expr::Copy { expr } => expr_uses_ok(expr, var, arity),
        Expr::Length { collection } => expr_uses_ok(collection, var, arity),
        Expr::Contains { collection, value } => {
            expr_uses_ok(collection, var, arity) && expr_uses_ok(value, var, arity)
        }
        Expr::List(items) | Expr::Tuple(items) => {
            items.iter().all(|i| expr_uses_ok(i, var, arity))
        }
        Expr::Range { start, end, .. } => {
            expr_uses_ok(start, var, arity) && expr_uses_ok(end, var, arity)
        }
        Expr::FieldAccess { object, .. } => expr_uses_ok(object, var, arity),
        Expr::OptionSome { value } => expr_uses_ok(value, var, arity),
        Expr::Give { value } => expr_uses_ok(value, var, arity),
        Expr::WithCapacity { value, capacity } => {
            expr_uses_ok(value, var, arity) && expr_uses_ok(capacity, var, arity)
        }
        // A use inside ANOTHER closure's body escapes this analysis (the
        // closure may itself lift or outlive the binding) — fail closed
        // when the body could mention the name.
        Expr::Closure { params, body, .. } => {
            if params.iter().any(|(p, _)| *p == var) {
                return true; // shadowed throughout the body
            }
            match body {
                ClosureBody::Expression(b) => !expr_mentions(b, var),
                ClosureBody::Block(stmts) => !stmts.iter().any(|s| stmt_mentions(s, var)),
            }
        }
        // Unrecognized expression shape: admit only if provably silent.
        other => !expr_mentions(other, var),
    }
}

// =====================================================================
// Mention detection — conservative TRUE for unrecognized shapes.
// =====================================================================

fn expr_mentions(e: &Expr, var: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s == var,
        Expr::Literal(_) | Expr::OptionNone => false,
        Expr::BinaryOp { left, right, .. } => {
            expr_mentions(left, var) || expr_mentions(right, var)
        }
        Expr::Not { operand } => expr_mentions(operand, var),
        Expr::Call { args, .. } => args.iter().any(|a| expr_mentions(a, var)),
        Expr::CallExpr { callee, args } => {
            expr_mentions(callee, var) || args.iter().any(|a| expr_mentions(a, var))
        }
        Expr::Index { collection, index } => {
            expr_mentions(collection, var) || expr_mentions(index, var)
        }
        Expr::Slice { collection, start, end } => {
            expr_mentions(collection, var)
                || expr_mentions(start, var)
                || expr_mentions(end, var)
        }
        Expr::Copy { expr } => expr_mentions(expr, var),
        Expr::Length { collection } => expr_mentions(collection, var),
        Expr::Contains { collection, value } => {
            expr_mentions(collection, var) || expr_mentions(value, var)
        }
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|i| expr_mentions(i, var)),
        Expr::Range { start, end, .. } => {
            expr_mentions(start, var) || expr_mentions(end, var)
        }
        Expr::FieldAccess { object, .. } => expr_mentions(object, var),
        Expr::OptionSome { value } => expr_mentions(value, var),
        Expr::Give { value } => expr_mentions(value, var),
        Expr::WithCapacity { value, capacity } => {
            expr_mentions(value, var) || expr_mentions(capacity, var)
        }
        Expr::Closure { params, body, .. } => {
            if params.iter().any(|(p, _)| *p == var) {
                return false;
            }
            match body {
                ClosureBody::Expression(b) => expr_mentions(b, var),
                ClosureBody::Block(stmts) => stmts.iter().any(|s| stmt_mentions(s, var)),
            }
        }
        _ => true,
    }
}

fn stmt_mentions(s: &Stmt, var: Symbol) -> bool {
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_mentions(value, var),
        Stmt::Show { object, .. } => expr_mentions(object, var),
        Stmt::Return { value } => value.map(|v| expr_mentions(v, var)).unwrap_or(false),
        Stmt::RuntimeAssert { condition, .. } => expr_mentions(condition, var),
        Stmt::Push { value, collection } => {
            expr_mentions(value, var) || expr_mentions(collection, var)
        }
        Stmt::SetIndex { collection, index, value } => {
            expr_mentions(collection, var)
                || expr_mentions(index, var)
                || expr_mentions(value, var)
        }
        Stmt::SetField { object, value, .. } => {
            expr_mentions(object, var) || expr_mentions(value, var)
        }
        Stmt::Call { args, .. } => args.iter().any(|a| expr_mentions(a, var)),
        Stmt::If { cond, then_block, else_block } => {
            expr_mentions(cond, var)
                || then_block.iter().any(|b| stmt_mentions(b, var))
                || else_block
                    .map(|eb| eb.iter().any(|b| stmt_mentions(b, var)))
                    .unwrap_or(false)
        }
        Stmt::While { cond, body, .. } => {
            expr_mentions(cond, var) || body.iter().any(|b| stmt_mentions(b, var))
        }
        Stmt::Repeat { iterable, body, .. } => {
            expr_mentions(iterable, var) || body.iter().any(|b| stmt_mentions(b, var))
        }
        Stmt::FunctionDef { .. } => false,
        _ => true,
    }
}

// =====================================================================
// Free variables of a closure body. `bound` carries parameters and
// body-local bindings; returns false (DECLINE) on unmodeled constructs.
// =====================================================================

fn free_vars_expr(e: &Expr, bound: &mut Vec<Symbol>, free: &mut BTreeMap<u32, Symbol>) -> bool {
    match e {
        Expr::Identifier(s) => {
            if !bound.contains(s) {
                free.insert(s.index() as u32, *s);
            }
            true
        }
        Expr::Literal(_) | Expr::OptionNone => true,
        Expr::BinaryOp { left, right, .. } => {
            free_vars_expr(left, bound, free) && free_vars_expr(right, bound, free)
        }
        Expr::Not { operand } => free_vars_expr(operand, bound, free),
        Expr::Call { args, .. } => args.iter().all(|a| free_vars_expr(a, bound, free)),
        Expr::Index { collection, index } => {
            free_vars_expr(collection, bound, free) && free_vars_expr(index, bound, free)
        }
        Expr::Length { collection } => free_vars_expr(collection, bound, free),
        Expr::Contains { collection, value } => {
            free_vars_expr(collection, bound, free) && free_vars_expr(value, bound, free)
        }
        Expr::List(items) | Expr::Tuple(items) => {
            items.iter().all(|i| free_vars_expr(i, bound, free))
        }
        Expr::Range { start, end, .. } => {
            free_vars_expr(start, bound, free) && free_vars_expr(end, bound, free)
        }
        // Nested closures, calls-of-values, slices, field access and the
        // long tail DECLINE stage 1 — fail closed.
        _ => false,
    }
}

fn free_vars_block(
    stmts: &[Stmt],
    bound: &mut Vec<Symbol>,
    free: &mut BTreeMap<u32, Symbol>,
) -> bool {
    let depth = bound.len();
    for s in stmts {
        let ok = match s {
            Stmt::Let { var, value, .. } => {
                let v_ok = free_vars_expr(value, bound, free);
                bound.push(*var);
                v_ok
            }
            Stmt::Set { target, value } => {
                // A Set to a CAPTURED variable makes the closure stateful
                // across calls (the env copy persists); a lifted function
                // would reset per call — DECLINE.
                bound.contains(target) && free_vars_expr(value, bound, free)
            }
            Stmt::Show { object, .. } => free_vars_expr(object, bound, free),
            Stmt::Return { value } => {
                value.map(|v| free_vars_expr(v, bound, free)).unwrap_or(true)
            }
            Stmt::If { cond, then_block, else_block } => {
                free_vars_expr(cond, bound, free)
                    && free_vars_block(then_block, bound, free)
                    && else_block
                        .map(|b| free_vars_block(b, bound, free))
                        .unwrap_or(true)
            }
            Stmt::While { cond, body, .. } => {
                free_vars_expr(cond, bound, free) && free_vars_block(body, bound, free)
            }
            _ => false,
        };
        if !ok {
            bound.truncate(depth);
            return false;
        }
    }
    bound.truncate(depth);
    true
}
