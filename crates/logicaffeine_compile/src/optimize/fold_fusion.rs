//! Fold/build fusion: a function that only takes a datatype apart, applied to what a function builds of it, compiles
//! to one function that never builds it (deforestation for a catamorphism over a builder; Wadler 1988).
//!
//! `check(bottomUpTree(d))` builds a tree of `2^(d+1) - 1` nodes so that `check` can visit each once and count it.
//! The fused `check_of_bottomUpTree(d)` is `bottomUpTree`'s own recursion with each construction replaced by `check`'s
//! arm for that variant: `If d = 0: Return 1. Return 1 + f(d - 1) + f(d - 1)`. Nothing is allocated, and the
//! function is in the shape the compiler already collapses (a double recursion's closed form, or memoization).
//!
//! The rewrite holds when:
//! - the builder `B` returns the enum `E` from a body of `Let`s, `If`s and `Return`s whose every path returns either a
//!   new variant of `E` or a call of `B`, with conditions, `Let` values and field values that cannot fail: names,
//!   literals, exact `+ - *`, comparisons, `not`/`and`/`or`, and `B`'s own recursion in a field;
//! - the consumer `C` takes one `E` and is one `Inspect` of it whose arms each `Return` an expression that cannot
//!   fail, made of literals, exact `+ - *`, the arm's bindings of fields that are not `E` (a payload), and `C` applied
//!   to the arm's bindings of fields that are `E`; every binding a field of `E` binds is used, so no builder call is
//!   dropped;
//! - the call is `C(B(args))`, or `Let x be B(args)` whose one use is `C(x)`.
//!
//! Neither function can fail outside its recursion, so interleaving the builder's calls with the consumer's arms
//! changes no output and no failure; the fused recursion makes the same calls in the same nesting the builder made,
//! so it runs out of stack where the builder would have.

use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, MatchArm, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::optimization::{Opt, OptimizationConfig};

/// One builder: the enum it builds, its parameters, its body and its optimizations.
struct Builder<'a> {
    enum_name: Symbol,
    params: Vec<(Symbol, &'a TypeExpr<'a>)>,
    body: Block<'a>,
    opt_flags: OptimizationConfig,
}

/// One consumer: the enum it takes apart, its result type, and its `Return` expression per variant (with the arm's
/// field bindings), plus a wildcard arm if it has one, and its optimizations.
struct Consumer<'a> {
    enum_name: Symbol,
    return_type: &'a TypeExpr<'a>,
    arms: HashMap<Symbol, (&'a Expr<'a>, Vec<(Symbol, Symbol)>)>,
    otherwise: Option<&'a Expr<'a>>,
    opt_flags: OptimizationConfig,
}

/// Fuse every consumer-of-builder call in `stmts`; true when one fused.
pub fn fold_fusion_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    let mut builders: HashMap<Symbol, Builder<'a>> = HashMap::new();
    let mut consumers: HashMap<Symbol, Consumer<'a>> = HashMap::new();
    for s in &stmts {
        if let Stmt::FunctionDef {
            name,
            generics,
            params,
            body,
            return_type: Some(ret),
            is_native: false,
            opt_flags,
            ..
        } = s
        {
            if !generics.is_empty() || !opt_flags.is_on(Opt::FoldFusion) {
                continue;
            }
            if let Some(b) = builder_of(*name, params, body, ret, *opt_flags) {
                builders.insert(*name, b);
            }
            if let Some(c) = consumer_of(*name, params, body, ret, *opt_flags) {
                consumers.insert(*name, c);
            }
        }
    }
    if builders.is_empty() || consumers.is_empty() {
        return (stmts, false);
    }
    let taken: HashSet<Symbol> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef { name, .. } => Some(*name),
            _ => None,
        })
        .collect();
    let mut cx = Fuser {
        builders,
        consumers,
        fused: HashMap::new(),
        pending: Vec::new(),
        taken,
        renamed: HashMap::new(),
        expr_arena,
        stmt_arena,
    };
    // The program's top level is one block (Main's statements among the definitions), so a `Let` in Main fuses too.
    let top: Block<'a> = stmt_arena.alloc_slice(stmts);
    let mut out: Vec<Stmt<'a>> = cx.block(top, interner).to_vec();
    // A fused function's body can itself hold fusable calls (its fields' consumer applications); synthesize until none
    // is pending.
    while let Some((c, b, fused_name)) = cx.pending.pop() {
        let def = cx.synthesize(c, b, fused_name, interner);
        out.push(def);
    }
    let fired = !cx.fused.is_empty();
    (out, fired)
}

struct Fuser<'a> {
    builders: HashMap<Symbol, Builder<'a>>,
    consumers: HashMap<Symbol, Consumer<'a>>,
    /// `(consumer, builder) -> fused function`.
    fused: HashMap<(Symbol, Symbol), Symbol>,
    /// Fused functions named but not yet synthesized.
    pending: Vec<(Symbol, Symbol, Symbol)>,
    taken: HashSet<Symbol>,
    /// Names bound in scope to a fused consumer's result: `x -> C`, whose one use `C(x)` reads `x` itself.
    renamed: HashMap<Symbol, Symbol>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
}

impl<'a> Fuser<'a> {
    /// The fused function for `consumer ∘ builder`, named on first use.
    fn fused_name(&mut self, consumer: Symbol, builder: Symbol, interner: &mut Interner) -> Symbol {
        if let Some(&f) = self.fused.get(&(consumer, builder)) {
            return f;
        }
        let base = format!(
            "{}_of_{}",
            interner.resolve(consumer),
            interner.resolve(builder)
        );
        let mut name = interner.intern(&base);
        let mut k = 2;
        while self.taken.contains(&name) {
            name = interner.intern(&format!("{base}_{k}"));
            k += 1;
        }
        self.taken.insert(name);
        self.fused.insert((consumer, builder), name);
        self.pending.push((consumer, builder, name));
        name
    }

    /// Whether `consumer` applied to a value `builder` builds can fuse: both are recognized and agree on the enum.
    fn fusable(&self, consumer: Symbol, builder: Symbol) -> bool {
        match (self.consumers.get(&consumer), self.builders.get(&builder)) {
            (Some(c), Some(b)) => c.enum_name == b.enum_name,
            _ => false,
        }
    }

    fn stmt(&mut self, s: Stmt<'a>, interner: &mut Interner) -> Stmt<'a> {
        match s {
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
                let body = if opt_flags.is_on(Opt::FoldFusion) {
                    self.block(body, interner)
                } else {
                    body
                };
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
                }
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => Stmt::If {
                cond: self.expr(cond, interner),
                then_block: self.block(then_block, interner),
                else_block: else_block.map(|b| self.block(b, interner)),
            },
            Stmt::While {
                cond,
                body,
                decreasing,
            } => Stmt::While {
                cond: self.expr(cond, interner),
                body: self.block(body, interner),
                decreasing,
            },
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => Stmt::Repeat {
                pattern,
                iterable: self.expr(iterable, interner),
                body: self.block(body, interner),
            },
            Stmt::Let {
                var,
                ty,
                value,
                mutable,
            } => Stmt::Let {
                var,
                ty,
                value: self.expr(value, interner),
                mutable,
            },
            Stmt::Set { target, value } => Stmt::Set {
                target,
                value: self.expr(value, interner),
            },
            Stmt::Return { value } => Stmt::Return {
                value: value.map(|v| self.expr(v, interner)),
            },
            Stmt::Show { object, recipient } => Stmt::Show {
                object: self.expr(object, interner),
                recipient,
            },
            other => other,
        }
    }

    /// `block` with every fusable call rewritten, and a `Let x be B(args)` whose one use after it is `C(x)` computing
    /// `C`'s result in its place.
    fn block(&mut self, block: Block<'a>, interner: &mut Interner) -> Block<'a> {
        let mut out: Vec<Stmt<'a>> = Vec::with_capacity(block.len());
        let mut scoped: Vec<(Symbol, Option<Symbol>)> = Vec::new();
        for (at, s) in block.iter().enumerate() {
            if let Stmt::Let {
                var,
                value: Expr::Call { function: b, args },
                mutable: false,
                ..
            } = s
            {
                if let Some(c) = self.sole_consumer_use(*var, *b, &block[at + 1..]) {
                    let f = self.fused_name(c, *b, interner);
                    let args: Vec<&'a Expr<'a>> =
                        args.iter().map(|a| self.expr(a, interner)).collect();
                    out.push(Stmt::Let {
                        var: *var,
                        ty: None,
                        value: self.expr_arena.alloc(Expr::Call { function: f, args }),
                        mutable: false,
                    });
                    scoped.push((*var, self.renamed.insert(*var, c)));
                    continue;
                }
            }
            out.push(self.stmt(s.clone(), interner));
        }
        for (var, before) in scoped.into_iter().rev() {
            match before {
                Some(c) => self.renamed.insert(var, c),
                None => self.renamed.remove(&var),
            };
        }
        self.stmt_arena.alloc_slice(out)
    }

    /// The consumer `C` when `x`, bound to a call of the builder `b`, is used exactly once after it in `rest`, as the
    /// whole argument of `C(x)` in a place this pass rewrites, and is never written or rebound.
    fn sole_consumer_use(&self, x: Symbol, b: Symbol, rest: &[Stmt<'a>]) -> Option<Symbol> {
        if !self.builders.contains_key(&b) {
            return None;
        }
        let mut found = None;
        let mut uses = 0usize;
        if !rest.iter().all(|s| scan_stmt(s, x, &mut found, &mut uses)) {
            return None;
        }
        let c = found?;
        (uses == 1 && self.fusable(c, b)).then_some(c)
    }

    fn expr(&mut self, e: &'a Expr<'a>, interner: &mut Interner) -> &'a Expr<'a> {
        match e {
            Expr::Call { function, args } => {
                if let [Expr::Identifier(x)] = args.as_slice() {
                    if self.renamed.get(x) == Some(function) {
                        return self.expr_arena.alloc(Expr::Identifier(*x));
                    }
                }
                if let [Expr::Call {
                    function: b,
                    args: inner,
                }] = args.as_slice()
                {
                    if self.fusable(*function, *b) {
                        let f = self.fused_name(*function, *b, interner);
                        let inner: Vec<&'a Expr<'a>> =
                            inner.iter().map(|a| self.expr(a, interner)).collect();
                        return self.expr_arena.alloc(Expr::Call {
                            function: f,
                            args: inner,
                        });
                    }
                }
                let args: Vec<&'a Expr<'a>> = args.iter().map(|a| self.expr(a, interner)).collect();
                self.expr_arena.alloc(Expr::Call {
                    function: *function,
                    args,
                })
            }
            Expr::BinaryOp { op, left, right } => self.expr_arena.alloc(Expr::BinaryOp {
                op: *op,
                left: self.expr(left, interner),
                right: self.expr(right, interner),
            }),
            Expr::Not { operand } => self.expr_arena.alloc(Expr::Not {
                operand: self.expr(operand, interner),
            }),
            _ => e,
        }
    }

    /// The fused function `name` for `consumer ∘ builder`: the builder's body with each construction replaced by the
    /// consumer's arm for its variant.
    fn synthesize(
        &mut self,
        consumer: Symbol,
        builder: Symbol,
        name: Symbol,
        interner: &mut Interner,
    ) -> Stmt<'a> {
        let (params, body, builder_flags) = {
            let b = &self.builders[&builder];
            (b.params.clone(), b.body, b.opt_flags)
        };
        let (return_type, consumer_flags) = {
            let c = &self.consumers[&consumer];
            (c.return_type, c.opt_flags)
        };
        let body = self.fused_block(body, consumer, builder, interner);
        Stmt::FunctionDef {
            name,
            generics: Vec::new(),
            params,
            body,
            return_type: Some(return_type),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: builder_flags.merged(&consumer_flags),
        }
    }

    fn fused_block(
        &mut self,
        block: Block<'a>,
        c: Symbol,
        b: Symbol,
        interner: &mut Interner,
    ) -> Block<'a> {
        let out: Vec<Stmt<'a>> = block
            .iter()
            .map(|s| match s {
                Stmt::Return { value: Some(v) } => Stmt::Return {
                    value: Some(self.fused_value(v, c, b, interner)),
                },
                Stmt::If {
                    cond,
                    then_block,
                    else_block,
                } => Stmt::If {
                    cond,
                    then_block: self.fused_block(then_block, c, b, interner),
                    else_block: else_block.map(|e| self.fused_block(e, c, b, interner)),
                },
                other => other.clone(),
            })
            .collect();
        self.stmt_arena.alloc_slice(out)
    }

    /// The consumer's result for a value the builder returns: its arm for a new variant, with each field it consumes
    /// replaced by the fused call on a builder call, or the consumer applied to any other field value.
    fn fused_value(
        &mut self,
        v: &'a Expr<'a>,
        c: Symbol,
        b: Symbol,
        interner: &mut Interner,
    ) -> &'a Expr<'a> {
        match v {
            Expr::Call { function, args } if *function == b => {
                let f = self.fused_name(c, b, interner);
                self.expr_arena.alloc(Expr::Call {
                    function: f,
                    args: args.clone(),
                })
            }
            Expr::NewVariant {
                variant, fields, ..
            } => {
                let (arm, bindings) = {
                    let cons = &self.consumers[&c];
                    match cons.arms.get(variant) {
                        Some((e, bs)) => (*e, bs.clone()),
                        None => (
                            cons.otherwise.expect("checked by the consumer's coverage"),
                            Vec::new(),
                        ),
                    }
                };
                let mut by_binding: HashMap<Symbol, &'a Expr<'a>> = HashMap::new();
                for (field, binding) in &bindings {
                    if let Some((_, value)) = fields.iter().find(|(f, _)| f == field) {
                        by_binding.insert(*binding, *value);
                    }
                }
                self.substitute(arm, c, b, &by_binding, interner)
            }
            _ => unreachable!("checked by the builder's shape"),
        }
    }

    fn substitute(
        &mut self,
        e: &'a Expr<'a>,
        c: Symbol,
        b: Symbol,
        by_binding: &HashMap<Symbol, &'a Expr<'a>>,
        interner: &mut Interner,
    ) -> &'a Expr<'a> {
        match e {
            Expr::Call { function, args } if *function == c => {
                let [Expr::Identifier(binding)] = args.as_slice() else {
                    unreachable!("checked by the consumer's shape")
                };
                let value = by_binding[binding];
                match value {
                    Expr::Call {
                        function: inner,
                        args,
                    } if *inner == b => {
                        let f = self.fused_name(c, b, interner);
                        self.expr_arena.alloc(Expr::Call {
                            function: f,
                            args: args.clone(),
                        })
                    }
                    _ => self.expr_arena.alloc(Expr::Call {
                        function: c,
                        args: vec![value],
                    }),
                }
            }
            Expr::Identifier(binding) if by_binding.contains_key(binding) => by_binding[binding],
            Expr::BinaryOp { op, left, right } => self.expr_arena.alloc(Expr::BinaryOp {
                op: *op,
                left: self.substitute(left, c, b, by_binding, interner),
                right: self.substitute(right, c, b, by_binding, interner),
            }),
            _ => e,
        }
    }
}

/// `builder_of(name, …)`: `name` builds the enum its return type names, by the shape the module documents.
fn builder_of<'a>(
    name: Symbol,
    params: &[(Symbol, &'a TypeExpr<'a>)],
    body: Block<'a>,
    ret: &'a TypeExpr<'a>,
    opt_flags: OptimizationConfig,
) -> Option<Builder<'a>> {
    let (TypeExpr::Named(enum_name) | TypeExpr::Primitive(enum_name)) = ret else {
        return None;
    };
    let mut names: HashSet<Symbol> = params.iter().map(|(p, _)| *p).collect();
    builds_on_every_path(body, name, *enum_name, &mut names).then(|| Builder {
        enum_name: *enum_name,
        params: params.to_vec(),
        body,
        opt_flags,
    })
}

/// Every path through `block` returns a new variant of `e` or a call of `b`, after `Let`s and `If`s that cannot
/// fail; `names` are the names in scope (parameters and earlier `Let`s).
fn builds_on_every_path(block: &[Stmt], b: Symbol, e: Symbol, names: &mut HashSet<Symbol>) -> bool {
    for (at, s) in block.iter().enumerate() {
        let last = at + 1 == block.len();
        match s {
            Stmt::Let {
                var,
                value,
                mutable: false,
                ..
            } if safe(value, names, Some(b)) && !last => {
                names.insert(*var);
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } if safe(cond, names, None) => {
                if !builds_on_every_path(then_block, b, e, &mut names.clone()) {
                    return false;
                }
                match else_block {
                    Some(eb) => return builds_on_every_path(eb, b, e, &mut names.clone()) && last,
                    None if last => return false,
                    None => {}
                }
            }
            Stmt::Return { value: Some(v) } => {
                return last && returns_built(v, b, e, names);
            }
            _ => return false,
        }
    }
    false
}

/// `v` is a new variant of `e` whose fields cannot fail (a field may call the builder `b`), or a call of `b` itself.
fn returns_built(v: &Expr, b: Symbol, e: Symbol, names: &HashSet<Symbol>) -> bool {
    match v {
        Expr::NewVariant {
            enum_name, fields, ..
        } => *enum_name == e && fields.iter().all(|(_, f)| safe(f, names, Some(b))),
        Expr::Call { function, args } => {
            *function == b && args.iter().all(|a| safe(a, names, None))
        }
        _ => false,
    }
}

/// `e` cannot fail and has no effect: names in scope, literals, exact `+ - *`, comparisons, `not`/`and`/`or`, and,
/// when `recursion` names one, calls of that function on such arguments.
fn safe(e: &Expr, names: &HashSet<Symbol>, recursion: Option<Symbol>) -> bool {
    use BinaryOpKind::*;
    match e {
        Expr::Identifier(s) => names.contains(s),
        Expr::Literal(Literal::Number(_) | Literal::Boolean(_)) => true,
        Expr::BinaryOp { op, left, right } => {
            matches!(
                op,
                Add | Subtract | Multiply | Eq | NotEq | Lt | LtEq | Gt | GtEq | And | Or
            ) && safe(left, names, recursion)
                && safe(right, names, recursion)
        }
        Expr::Not { operand } => safe(operand, names, recursion),
        Expr::Call { function, args } => {
            Some(*function) == recursion && args.iter().all(|a| safe(a, names, None))
        }
        _ => false,
    }
}

/// `consumer_of(name, …)`: `name` takes one value of an enum apart, by the shape the module documents.
fn consumer_of<'a>(
    name: Symbol,
    params: &[(Symbol, &'a TypeExpr<'a>)],
    body: Block<'a>,
    ret: &'a TypeExpr<'a>,
    opt_flags: OptimizationConfig,
) -> Option<Consumer<'a>> {
    let [(p, TypeExpr::Named(enum_name) | TypeExpr::Primitive(enum_name))] = params else {
        return None;
    };
    let [Stmt::Inspect { target, arms, .. }] = body else {
        return None;
    };
    if !matches!(target, Expr::Identifier(t) if t == p) {
        return None;
    }
    let mut by_variant = HashMap::new();
    let mut otherwise = None;
    for arm in arms {
        let MatchArm {
            enum_name: arm_enum,
            variant,
            bindings,
            body,
        } = arm;
        if arm_enum.is_some_and(|a| a != *enum_name) {
            return None;
        }
        let [Stmt::Return { value: Some(value) }] = *body else {
            return None;
        };
        if !folds(value, name, bindings) {
            return None;
        }
        match variant {
            Some(v) => {
                by_variant.insert(*v, (*value, bindings.clone()));
            }
            None if bindings.is_empty() => otherwise = Some(*value),
            None => return None,
        }
    }
    Some(Consumer {
        enum_name: *enum_name,
        return_type: ret,
        arms: by_variant,
        otherwise,
        opt_flags,
    })
}

/// An arm's result folds: it cannot fail, names only the arm's bindings, and each binding appears either inside a
/// call `c(binding)` (the recursion on a field) or bare (a payload field), never both ways; every binding is used.
fn folds(value: &Expr, c: Symbol, bindings: &[(Symbol, Symbol)]) -> bool {
    let bound: HashSet<Symbol> = bindings.iter().map(|(_, b)| *b).collect();
    let mut recursed: HashSet<Symbol> = HashSet::new();
    let mut bare: HashSet<Symbol> = HashSet::new();
    fn walk(
        e: &Expr,
        c: Symbol,
        bound: &HashSet<Symbol>,
        recursed: &mut HashSet<Symbol>,
        bare: &mut HashSet<Symbol>,
    ) -> bool {
        use BinaryOpKind::*;
        match e {
            Expr::Literal(Literal::Number(_)) => true,
            Expr::Identifier(s) if bound.contains(s) => {
                bare.insert(*s);
                true
            }
            Expr::Call { function, args } if *function == c => match args.as_slice() {
                [Expr::Identifier(s)] if bound.contains(s) => {
                    recursed.insert(*s);
                    true
                }
                _ => false,
            },
            Expr::BinaryOp { op, left, right } => {
                matches!(op, Add | Subtract | Multiply)
                    && walk(left, c, bound, recursed, bare)
                    && walk(right, c, bound, recursed, bare)
            }
            _ => false,
        }
    }
    walk(value, c, &bound, &mut recursed, &mut bare)
        && recursed.is_disjoint(&bare)
        && bound
            .iter()
            .all(|b| recursed.contains(b) || bare.contains(b))
}

/// Whether every mention of `x` in `s` sits in a place [`Fuser::stmt`] rewrites (a `Let`, `Set` or `Return` value, a
/// `Show`n object, a condition or an iterable, nested blocks included) and nothing writes or rebinds it; the mentions
/// that are the whole argument of a one-argument call are counted, with that call's function.
fn scan_stmt(s: &Stmt, x: Symbol, found: &mut Option<Symbol>, uses: &mut usize) -> bool {
    use super::mentions::{expr_touches, stmt_touches};
    if crate::optimize::WriteSet::of_stmt(s).may_write(x) {
        return false;
    }
    let all = |b: &[Stmt], found: &mut Option<Symbol>, uses: &mut usize| {
        b.iter().all(|t| scan_stmt(t, x, found, uses))
    };
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } | Stmt::Return { value: Some(value) } => {
            scan_expr(value, x, found, uses)
        }
        Stmt::Show { object, recipient } => {
            scan_expr(object, x, found, uses) && !expr_touches(recipient, x)
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            scan_expr(cond, x, found, uses)
                && all(then_block, found, uses)
                && else_block.map_or(true, |eb| all(eb, found, uses))
        }
        Stmt::While { cond, body, .. } => scan_expr(cond, x, found, uses) && all(body, found, uses),
        Stmt::Repeat { iterable, body, .. } => {
            scan_expr(iterable, x, found, uses) && all(body, found, uses)
        }
        _ => !stmt_touches(s, x),
    }
}

/// [`scan_stmt`] for one expression: `x` appears only as the whole argument of a one-argument call, inside the
/// calls, arithmetic and negations [`Fuser::expr`] rewrites.
fn scan_expr(e: &Expr, x: Symbol, found: &mut Option<Symbol>, uses: &mut usize) -> bool {
    match e {
        Expr::Call { function, args } => {
            if let [Expr::Identifier(s)] = args.as_slice() {
                if *s == x {
                    *uses += 1;
                    *found = Some(*function);
                    return true;
                }
            }
            args.iter().all(|a| scan_expr(a, x, found, uses))
        }
        Expr::BinaryOp { left, right, .. } => {
            scan_expr(left, x, found, uses) && scan_expr(right, x, found, uses)
        }
        Expr::Not { operand } => scan_expr(operand, x, found, uses),
        other => !super::mentions::expr_touches(other, x),
    }
}
