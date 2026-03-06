use std::collections::HashMap;

use crate::arena::Arena;
use crate::ast::stmt::{Expr, Literal, Stmt, TypeExpr, Block, OptFlag};
use crate::intern::{Interner, Symbol};
use std::collections::HashSet;

use super::bta::{self, BindingTime, Division};

type SpecKey = (Symbol, String);

struct FuncInfo<'a> {
    name: Symbol,
    params: Vec<(Symbol, &'a TypeExpr<'a>)>,
    body: Block<'a>,
    generics: Vec<Symbol>,
    return_type: Option<&'a TypeExpr<'a>>,
    opt_flags: HashSet<OptFlag>,
}

struct SpecRegistry<'a> {
    cache: HashMap<SpecKey, Symbol>,
    new_funcs: Vec<Stmt<'a>>,
    variant_count: HashMap<Symbol, usize>,
    history: Vec<SpecKey>,
}

impl<'a> SpecRegistry<'a> {
    fn new() -> Self {
        SpecRegistry {
            cache: HashMap::new(),
            new_funcs: Vec::new(),
            variant_count: HashMap::new(),
            history: Vec::new(),
        }
    }
}

fn spec_key_embeds(earlier: &SpecKey, later: &SpecKey) -> bool {
    if earlier.0 != later.0 {
        return false;
    }
    let earlier_str = &earlier.1;
    let later_str = &later.1;
    if earlier_str == later_str {
        return false;
    }
    later_str.contains(earlier_str)
}

fn collect_func_defs<'a>(stmts: &[Stmt<'a>]) -> HashMap<Symbol, FuncInfo<'a>> {
    let mut defs = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, generics, return_type, is_native, opt_flags, .. } = stmt {
            if !is_native {
                defs.insert(*name, FuncInfo {
                    name: *name,
                    params: params.clone(),
                    body,
                    generics: generics.clone(),
                    return_type: *return_type,
                    opt_flags: opt_flags.clone(),
                });
            }
        }
    }
    defs
}

fn body_has_io(stmts: &[Stmt]) -> bool {
    for stmt in stmts {
        if stmt_has_io(stmt) {
            return true;
        }
    }
    false
}

fn stmt_has_io(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Show { .. }
        | Stmt::WriteFile { .. }
        | Stmt::SendMessage { .. }
        | Stmt::Sleep { .. }
        | Stmt::SendPipe { .. }
        | Stmt::TrySendPipe { .. }
        | Stmt::ReceivePipe { .. }
        | Stmt::ReadFrom { .. }
        | Stmt::Check { .. } => true,
        Stmt::IncreaseCrdt { .. }
        | Stmt::DecreaseCrdt { .. }
        | Stmt::MergeCrdt { .. } => true,
        Stmt::If { then_block, else_block, .. } => {
            body_has_io(then_block)
                || else_block.map_or(false, |eb| body_has_io(eb))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => body_has_io(body),
        Stmt::Zone { body, .. } => body_has_io(body),
        Stmt::Inspect { arms, .. } => {
            arms.iter().any(|arm| body_has_io(arm.body))
        }
        _ => false,
    }
}

fn body_has_escape(stmts: &[Stmt]) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Escape { .. } => return true,
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => {
                if expr_has_escape(value) {
                    return true;
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                if body_has_escape(then_block) {
                    return true;
                }
                if let Some(eb) = else_block {
                    if body_has_escape(eb) {
                        return true;
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                if body_has_escape(body) {
                    return true;
                }
            }
            Stmt::Return { value } => {
                if let Some(v) = value {
                    if expr_has_escape(v) {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

fn expr_has_escape(expr: &Expr) -> bool {
    match expr {
        Expr::Escape { .. } => true,
        _ => false,
    }
}

fn classify_arg<'a>(arg: &Expr<'a>, division: Option<&Division>) -> Option<Literal> {
    match arg {
        Expr::Literal(lit) => Some(lit.clone()),
        _ => {
            if let Some(div) = division {
                match bta::analyze_expr(arg, div) {
                    BindingTime::Static(lit) => Some(lit),
                    BindingTime::Dynamic => None,
                }
            } else {
                None
            }
        }
    }
}

fn classifications_to_key_string(classifications: &[Option<Literal>]) -> String {
    classifications.iter()
        .map(|c| match c {
            None => "D".to_string(),
            Some(lit) => format!("S{}", literal_to_name_part(lit)),
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn compute_spec_key<'a>(function: Symbol, args: &[&'a Expr<'a>], division: Option<&Division>) -> (SpecKey, Vec<Option<Literal>>) {
    let arg_classifications: Vec<Option<Literal>> = args.iter()
        .map(|a| classify_arg(a, division))
        .collect();
    let key_string = classifications_to_key_string(&arg_classifications);
    let key = (function, key_string);
    (key, arg_classifications)
}

fn is_mixed(classifications: &[Option<Literal>]) -> bool {
    let has_static = classifications.iter().any(|c| c.is_some());
    let has_dynamic = classifications.iter().any(|c| c.is_none());
    has_static && has_dynamic
}

fn make_spec_name(interner: &mut Interner, func_name: Symbol, classifications: &[Option<Literal>]) -> Symbol {
    let base = interner.resolve(func_name);
    let mut name = base.to_string();
    for (i, c) in classifications.iter().enumerate() {
        if let Some(lit) = c {
            name.push_str(&format!("_s{}_{}", i, literal_to_name_part(lit)));
        }
    }
    interner.intern(&name)
}

fn literal_to_name_part(lit: &Literal) -> String {
    match lit {
        Literal::Number(n) => format!("{}", n),
        Literal::Float(f) => format!("{}", f).replace('.', "d").replace('-', "n"),
        Literal::Boolean(b) => format!("{}", b),
        Literal::Text(s) => format!("t{:x}", s.index()),
        Literal::Nothing => "nothing".to_string(),
        _ => "x".to_string(),
    }
}

fn substitute_expr<'a>(
    expr: &'a Expr<'a>,
    substitutions: &HashMap<Symbol, &'a Expr<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    match expr {
        Expr::Identifier(sym) => {
            if let Some(replacement) = substitutions.get(sym) {
                replacement
            } else {
                expr
            }
        }
        Expr::BinaryOp { op, left, right } => {
            let new_left = substitute_expr(left, substitutions, expr_arena);
            let new_right = substitute_expr(right, substitutions, expr_arena);
            if std::ptr::eq(new_left as *const _, *left as *const _)
                && std::ptr::eq(new_right as *const _, *right as *const _) {
                expr
            } else {
                expr_arena.alloc(Expr::BinaryOp {
                    op: *op,
                    left: new_left,
                    right: new_right,
                })
            }
        }
        Expr::Not { operand } => {
            let new_operand = substitute_expr(operand, substitutions, expr_arena);
            if std::ptr::eq(new_operand as *const _, *operand as *const _) {
                expr
            } else {
                expr_arena.alloc(Expr::Not { operand: new_operand })
            }
        }
        Expr::Call { function, args } => {
            let new_args: Vec<&'a Expr<'a>> = args.iter()
                .map(|a| substitute_expr(a, substitutions, expr_arena))
                .collect();
            let changed = new_args.iter().zip(args.iter())
                .any(|(new, old)| !std::ptr::eq(*new as *const _, *old as *const _));
            if changed {
                expr_arena.alloc(Expr::Call { function: *function, args: new_args })
            } else {
                expr
            }
        }
        Expr::Index { collection, index } => {
            let new_coll = substitute_expr(collection, substitutions, expr_arena);
            let new_idx = substitute_expr(index, substitutions, expr_arena);
            if std::ptr::eq(new_coll as *const _, *collection as *const _)
                && std::ptr::eq(new_idx as *const _, *index as *const _) {
                expr
            } else {
                expr_arena.alloc(Expr::Index { collection: new_coll, index: new_idx })
            }
        }
        Expr::Length { collection } => {
            let new_coll = substitute_expr(collection, substitutions, expr_arena);
            if std::ptr::eq(new_coll as *const _, *collection as *const _) {
                expr
            } else {
                expr_arena.alloc(Expr::Length { collection: new_coll })
            }
        }
        Expr::Slice { collection, start, end } => {
            let new_coll = substitute_expr(collection, substitutions, expr_arena);
            let new_start = substitute_expr(start, substitutions, expr_arena);
            let new_end = substitute_expr(end, substitutions, expr_arena);
            if std::ptr::eq(new_coll as *const _, *collection as *const _)
                && std::ptr::eq(new_start as *const _, *start as *const _)
                && std::ptr::eq(new_end as *const _, *end as *const _) {
                expr
            } else {
                expr_arena.alloc(Expr::Slice { collection: new_coll, start: new_start, end: new_end })
            }
        }
        Expr::FieldAccess { object, field } => {
            let new_obj = substitute_expr(object, substitutions, expr_arena);
            if std::ptr::eq(new_obj as *const _, *object as *const _) {
                expr
            } else {
                expr_arena.alloc(Expr::FieldAccess { object: new_obj, field: *field })
            }
        }
        Expr::Contains { collection, value } => {
            let new_coll = substitute_expr(collection, substitutions, expr_arena);
            let new_val = substitute_expr(value, substitutions, expr_arena);
            if std::ptr::eq(new_coll as *const _, *collection as *const _)
                && std::ptr::eq(new_val as *const _, *value as *const _) {
                expr
            } else {
                expr_arena.alloc(Expr::Contains { collection: new_coll, value: new_val })
            }
        }
        _ => expr,
    }
}

fn substitute_stmt<'a>(
    stmt: &Stmt<'a>,
    substitutions: &HashMap<Symbol, &'a Expr<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Stmt<'a> {
    match stmt {
        Stmt::Let { var, value, mutable, ty } => Stmt::Let {
            var: *var,
            value: substitute_expr(value, substitutions, expr_arena),
            mutable: *mutable,
            ty: *ty,
        },
        Stmt::Set { target, value } => Stmt::Set {
            target: *target,
            value: substitute_expr(value, substitutions, expr_arena),
        },
        Stmt::Return { value } => Stmt::Return {
            value: value.map(|v| substitute_expr(v, substitutions, expr_arena)),
        },
        Stmt::Show { object, recipient } => Stmt::Show {
            object: substitute_expr(object, substitutions, expr_arena),
            recipient: *recipient,
        },
        Stmt::Call { function, args } => Stmt::Call {
            function: *function,
            args: args.iter().map(|a| substitute_expr(a, substitutions, expr_arena)).collect(),
        },
        Stmt::If { cond, then_block, else_block } => {
            let new_then: Vec<Stmt<'a>> = then_block.iter()
                .map(|s| substitute_stmt(s, substitutions, expr_arena, stmt_arena))
                .collect();
            let new_else = else_block.map(|eb| {
                let stmts: Vec<Stmt<'a>> = eb.iter()
                    .map(|s| substitute_stmt(s, substitutions, expr_arena, stmt_arena))
                    .collect();
                stmt_arena.alloc_slice(stmts) as &[Stmt<'a>]
            });
            Stmt::If {
                cond: substitute_expr(cond, substitutions, expr_arena),
                then_block: stmt_arena.alloc_slice(new_then),
                else_block: new_else,
            }
        }
        Stmt::While { cond, body, decreasing } => {
            let new_body: Vec<Stmt<'a>> = body.iter()
                .map(|s| substitute_stmt(s, substitutions, expr_arena, stmt_arena))
                .collect();
            Stmt::While {
                cond: substitute_expr(cond, substitutions, expr_arena),
                body: stmt_arena.alloc_slice(new_body),
                decreasing: *decreasing,
            }
        }
        Stmt::Repeat { pattern, iterable, body } => {
            let new_body: Vec<Stmt<'a>> = body.iter()
                .map(|s| substitute_stmt(s, substitutions, expr_arena, stmt_arena))
                .collect();
            Stmt::Repeat {
                pattern: pattern.clone(),
                iterable: substitute_expr(iterable, substitutions, expr_arena),
                body: stmt_arena.alloc_slice(new_body),
            }
        }
        Stmt::SetIndex { collection, index, value } => Stmt::SetIndex {
            collection: substitute_expr(collection, substitutions, expr_arena),
            index: substitute_expr(index, substitutions, expr_arena),
            value: substitute_expr(value, substitutions, expr_arena),
        },
        Stmt::Push { value, collection } => Stmt::Push {
            value: substitute_expr(value, substitutions, expr_arena),
            collection: substitute_expr(collection, substitutions, expr_arena),
        },
        Stmt::SetField { object, field, value } => Stmt::SetField {
            object: substitute_expr(object, substitutions, expr_arena),
            field: *field,
            value: substitute_expr(value, substitutions, expr_arena),
        },
        Stmt::Give { object, recipient } => Stmt::Give {
            object: substitute_expr(object, substitutions, expr_arena),
            recipient: substitute_expr(recipient, substitutions, expr_arena),
        },
        Stmt::Add { value, collection } => Stmt::Add {
            value: substitute_expr(value, substitutions, expr_arena),
            collection: substitute_expr(collection, substitutions, expr_arena),
        },
        Stmt::Remove { value, collection } => Stmt::Remove {
            value: substitute_expr(value, substitutions, expr_arena),
            collection: substitute_expr(collection, substitutions, expr_arena),
        },
        Stmt::Inspect { target, arms, has_otherwise } => {
            let new_arms: Vec<_> = arms.iter().map(|arm| {
                let new_body: Vec<Stmt<'a>> = arm.body.iter()
                    .map(|s| substitute_stmt(s, substitutions, expr_arena, stmt_arena))
                    .collect();
                crate::ast::stmt::MatchArm {
                    enum_name: arm.enum_name,
                    variant: arm.variant,
                    bindings: arm.bindings.clone(),
                    body: stmt_arena.alloc_slice(new_body),
                }
            }).collect();
            Stmt::Inspect {
                target: substitute_expr(target, substitutions, expr_arena),
                arms: new_arms,
                has_otherwise: *has_otherwise,
            }
        }
        Stmt::RuntimeAssert { condition } => Stmt::RuntimeAssert {
            condition: substitute_expr(condition, substitutions, expr_arena),
        },
        other => other.clone(),
    }
}

fn substitute_block<'a>(
    block: Block<'a>,
    substitutions: &HashMap<Symbol, &'a Expr<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    block.iter()
        .map(|s| substitute_stmt(s, substitutions, expr_arena, stmt_arena))
        .collect()
}

fn count_stmts(stmts: &[Stmt]) -> usize {
    let mut count = 0;
    for stmt in stmts {
        count += 1;
        match stmt {
            Stmt::If { then_block, else_block, .. } => {
                count += count_stmts(then_block);
                if let Some(eb) = else_block {
                    count += count_stmts(eb);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                count += count_stmts(body);
            }
            Stmt::FunctionDef { body, .. } => {
                count += count_stmts(body);
            }
            _ => {}
        }
    }
    count
}

fn try_specialize_call<'a>(
    function: Symbol,
    args: &[&'a Expr<'a>],
    func_defs: &HashMap<Symbol, FuncInfo<'a>>,
    registry: &mut SpecRegistry<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Option<(Symbol, Vec<&'a Expr<'a>>)> {
    // Build a BTA division from known literal arguments for enhanced classification
    let mut division = Division::new();
    if let Some(func_info) = func_defs.get(&function) {
        for (i, (param_sym, _)) in func_info.params.iter().enumerate() {
            if let Some(arg) = args.get(i) {
                if let Expr::Literal(lit) = arg {
                    division.insert(*param_sym, BindingTime::Static(lit.clone()));
                }
            }
        }
    }
    let (key, classifications) = compute_spec_key(function, args, Some(&division));

    if !is_mixed(&classifications) {
        return None;
    }

    if let Some(&cached_name) = registry.cache.get(&key) {
        let dynamic_args: Vec<&'a Expr<'a>> = args.iter().zip(classifications.iter())
            .filter(|(_, c)| c.is_none())
            .map(|(a, _)| *a)
            .collect();
        return Some((cached_name, dynamic_args));
    }

    // Check if this specialization embeds in a previous one (termination guard)
    if registry.history.iter().any(|prev| spec_key_embeds(prev, &key)) {
        return None;
    }

    let count = registry.variant_count.get(&function).copied().unwrap_or(0);
    if count >= 8 {
        return None;
    }

    let func_info = func_defs.get(&function)?;

    if body_has_io(func_info.body) {
        return None;
    }

    if body_has_escape(func_info.body) {
        return None;
    }

    let spec_name = make_spec_name(interner, func_info.name, &classifications);

    let mut substitutions: HashMap<Symbol, &'a Expr<'a>> = HashMap::new();
    let mut new_params: Vec<(Symbol, &'a TypeExpr<'a>)> = Vec::new();

    for (i, (param_sym, param_type)) in func_info.params.iter().enumerate() {
        if let Some(Some(lit)) = classifications.get(i) {
            let lit_expr = expr_arena.alloc(Expr::Literal(lit.clone()));
            substitutions.insert(*param_sym, lit_expr);
        } else {
            new_params.push((*param_sym, *param_type));
        }
    }

    let specialized_body = substitute_block(func_info.body, &substitutions, expr_arena, stmt_arena);

    let folded = super::fold::fold_stmts(specialized_body, expr_arena, stmt_arena, interner);
    let optimized = super::dce::eliminate_dead_code(folded, stmt_arena, expr_arena);

    let original_cost = count_stmts(func_info.body) + func_info.params.len();
    let specialized_cost = count_stmts(&optimized) + new_params.len();

    if specialized_cost as f64 > original_cost as f64 * 0.8 {
        return None;
    }

    // Register the specialized function in cache BEFORE cascading to prevent infinite loops
    registry.history.push(key.clone());
    registry.cache.insert(key, spec_name);
    *registry.variant_count.entry(function).or_insert(0) += 1;

    // Cascade: walk specialized body for further specialization candidates
    let cascaded: Vec<Stmt<'a>> = optimized.into_iter()
        .map(|s| specialize_in_stmt(s, func_defs, registry, expr_arena, stmt_arena, interner))
        .collect();

    let new_body = stmt_arena.alloc_slice(cascaded);

    let new_func = Stmt::FunctionDef {
        name: spec_name,
        generics: func_info.generics.clone(),
        params: new_params,
        body: new_body,
        return_type: func_info.return_type,
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: func_info.opt_flags.clone(),
    };

    registry.new_funcs.push(new_func);

    let dynamic_args: Vec<&'a Expr<'a>> = args.iter().zip(classifications.iter())
        .filter(|(_, c)| c.is_none())
        .map(|(a, _)| *a)
        .collect();

    Some((spec_name, dynamic_args))
}

fn specialize_in_expr<'a>(
    expr: &'a Expr<'a>,
    func_defs: &HashMap<Symbol, FuncInfo<'a>>,
    registry: &mut SpecRegistry<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> &'a Expr<'a> {
    match expr {
        Expr::Call { function, args } => {
            let new_args: Vec<&'a Expr<'a>> = args.iter()
                .map(|a| specialize_in_expr(a, func_defs, registry, expr_arena, stmt_arena, interner))
                .collect();

            if let Some((spec_name, dynamic_args)) = try_specialize_call(
                *function, &new_args, func_defs, registry, expr_arena, stmt_arena, interner,
            ) {
                expr_arena.alloc(Expr::Call {
                    function: spec_name,
                    args: dynamic_args,
                })
            } else {
                let changed = new_args.iter().zip(args.iter())
                    .any(|(new, old)| !std::ptr::eq(*new as *const _, *old as *const _));
                if changed {
                    expr_arena.alloc(Expr::Call { function: *function, args: new_args })
                } else {
                    expr
                }
            }
        }
        Expr::BinaryOp { op, left, right } => {
            let new_left = specialize_in_expr(left, func_defs, registry, expr_arena, stmt_arena, interner);
            let new_right = specialize_in_expr(right, func_defs, registry, expr_arena, stmt_arena, interner);
            if std::ptr::eq(new_left as *const _, *left as *const _)
                && std::ptr::eq(new_right as *const _, *right as *const _) {
                expr
            } else {
                expr_arena.alloc(Expr::BinaryOp { op: *op, left: new_left, right: new_right })
            }
        }
        Expr::Not { operand } => {
            let new_op = specialize_in_expr(operand, func_defs, registry, expr_arena, stmt_arena, interner);
            if std::ptr::eq(new_op as *const _, *operand as *const _) {
                expr
            } else {
                expr_arena.alloc(Expr::Not { operand: new_op })
            }
        }
        _ => expr,
    }
}

fn specialize_in_stmt<'a>(
    stmt: Stmt<'a>,
    func_defs: &HashMap<Symbol, FuncInfo<'a>>,
    registry: &mut SpecRegistry<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Stmt<'a> {
    match stmt {
        Stmt::Let { var, value, mutable, ty } => Stmt::Let {
            var,
            value: specialize_in_expr(value, func_defs, registry, expr_arena, stmt_arena, interner),
            mutable,
            ty,
        },
        Stmt::Set { target, value } => Stmt::Set {
            target,
            value: specialize_in_expr(value, func_defs, registry, expr_arena, stmt_arena, interner),
        },
        Stmt::Return { value } => Stmt::Return {
            value: value.map(|v| specialize_in_expr(v, func_defs, registry, expr_arena, stmt_arena, interner)),
        },
        Stmt::Show { object, recipient } => Stmt::Show {
            object: specialize_in_expr(object, func_defs, registry, expr_arena, stmt_arena, interner),
            recipient,
        },
        Stmt::Call { function, args } => {
            let new_args: Vec<&'a Expr<'a>> = args.iter()
                .map(|a| specialize_in_expr(a, func_defs, registry, expr_arena, stmt_arena, interner))
                .collect();

            if let Some((spec_name, dynamic_args)) = try_specialize_call(
                function, &new_args, func_defs, registry, expr_arena, stmt_arena, interner,
            ) {
                Stmt::Call { function: spec_name, args: dynamic_args }
            } else {
                Stmt::Call { function, args: new_args }
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            let new_cond = specialize_in_expr(cond, func_defs, registry, expr_arena, stmt_arena, interner);
            let new_then: Vec<Stmt<'a>> = then_block.iter().cloned()
                .map(|s| specialize_in_stmt(s, func_defs, registry, expr_arena, stmt_arena, interner))
                .collect();
            let new_else = else_block.map(|eb| {
                let stmts: Vec<Stmt<'a>> = eb.iter().cloned()
                    .map(|s| specialize_in_stmt(s, func_defs, registry, expr_arena, stmt_arena, interner))
                    .collect();
                stmt_arena.alloc_slice(stmts) as &[Stmt<'a>]
            });
            Stmt::If {
                cond: new_cond,
                then_block: stmt_arena.alloc_slice(new_then),
                else_block: new_else,
            }
        }
        Stmt::While { cond, body, decreasing } => {
            let new_cond = specialize_in_expr(cond, func_defs, registry, expr_arena, stmt_arena, interner);
            let new_body: Vec<Stmt<'a>> = body.iter().cloned()
                .map(|s| specialize_in_stmt(s, func_defs, registry, expr_arena, stmt_arena, interner))
                .collect();
            Stmt::While {
                cond: new_cond,
                body: stmt_arena.alloc_slice(new_body),
                decreasing,
            }
        }
        Stmt::Repeat { pattern, iterable, body } => {
            let new_iter = specialize_in_expr(iterable, func_defs, registry, expr_arena, stmt_arena, interner);
            let new_body: Vec<Stmt<'a>> = body.iter().cloned()
                .map(|s| specialize_in_stmt(s, func_defs, registry, expr_arena, stmt_arena, interner))
                .collect();
            Stmt::Repeat {
                pattern,
                iterable: new_iter,
                body: stmt_arena.alloc_slice(new_body),
            }
        }
        Stmt::SetIndex { collection, index, value } => Stmt::SetIndex {
            collection: specialize_in_expr(collection, func_defs, registry, expr_arena, stmt_arena, interner),
            index: specialize_in_expr(index, func_defs, registry, expr_arena, stmt_arena, interner),
            value: specialize_in_expr(value, func_defs, registry, expr_arena, stmt_arena, interner),
        },
        Stmt::Push { value, collection } => Stmt::Push {
            value: specialize_in_expr(value, func_defs, registry, expr_arena, stmt_arena, interner),
            collection: specialize_in_expr(collection, func_defs, registry, expr_arena, stmt_arena, interner),
        },
        Stmt::SetField { object, field, value } => Stmt::SetField {
            object: specialize_in_expr(object, func_defs, registry, expr_arena, stmt_arena, interner),
            field,
            value: specialize_in_expr(value, func_defs, registry, expr_arena, stmt_arena, interner),
        },
        Stmt::Give { object, recipient } => Stmt::Give {
            object: specialize_in_expr(object, func_defs, registry, expr_arena, stmt_arena, interner),
            recipient: specialize_in_expr(recipient, func_defs, registry, expr_arena, stmt_arena, interner),
        },
        Stmt::Add { value, collection } => Stmt::Add {
            value: specialize_in_expr(value, func_defs, registry, expr_arena, stmt_arena, interner),
            collection: specialize_in_expr(collection, func_defs, registry, expr_arena, stmt_arena, interner),
        },
        Stmt::Remove { value, collection } => Stmt::Remove {
            value: specialize_in_expr(value, func_defs, registry, expr_arena, stmt_arena, interner),
            collection: specialize_in_expr(collection, func_defs, registry, expr_arena, stmt_arena, interner),
        },
        Stmt::Inspect { target, arms, has_otherwise } => {
            let new_target = specialize_in_expr(target, func_defs, registry, expr_arena, stmt_arena, interner);
            let new_arms: Vec<_> = arms.into_iter().map(|arm| {
                let new_body: Vec<Stmt<'a>> = arm.body.iter().cloned()
                    .map(|s| specialize_in_stmt(s, func_defs, registry, expr_arena, stmt_arena, interner))
                    .collect();
                crate::ast::stmt::MatchArm {
                    enum_name: arm.enum_name,
                    variant: arm.variant,
                    bindings: arm.bindings,
                    body: stmt_arena.alloc_slice(new_body),
                }
            }).collect();
            Stmt::Inspect {
                target: new_target,
                arms: new_arms,
                has_otherwise,
            }
        }
        Stmt::RuntimeAssert { condition } => Stmt::RuntimeAssert {
            condition: specialize_in_expr(condition, func_defs, registry, expr_arena, stmt_arena, interner),
        },
        Stmt::FunctionDef { name, params, generics, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => {
            if is_native {
                return Stmt::FunctionDef { name, params, generics, body, return_type, is_native, native_path, is_exported, export_target, opt_flags };
            }
            let new_body: Vec<Stmt<'a>> = body.iter().cloned()
                .map(|s| specialize_in_stmt(s, func_defs, registry, expr_arena, stmt_arena, interner))
                .collect();
            Stmt::FunctionDef {
                name,
                params,
                generics,
                body: stmt_arena.alloc_slice(new_body),
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

pub fn specialize_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, usize) {
    let mut variant_count = HashMap::new();
    specialize_stmts_with_state(stmts, expr_arena, stmt_arena, interner, &mut variant_count)
}

pub fn specialize_stmts_with_state<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    persistent_variant_count: &mut HashMap<Symbol, usize>,
) -> (Vec<Stmt<'a>>, usize) {
    let func_defs = collect_func_defs(&stmts);
    let mut registry = SpecRegistry::new();
    registry.variant_count = persistent_variant_count.clone();

    let specialized: Vec<Stmt<'a>> = stmts.into_iter()
        .map(|stmt| specialize_in_stmt(stmt, &func_defs, &mut registry, expr_arena, stmt_arena, interner))
        .collect();

    let changes = registry.new_funcs.len();
    *persistent_variant_count = registry.variant_count;
    let mut result = registry.new_funcs;
    result.extend(specialized);
    (result, changes)
}

/// Remove identity bindings and trivial patterns from residual code.
///
/// - `Let x = x` → removed
/// - `Let x = lit; Return x` → `Return lit`
/// - Single-arm `Inspect` with Otherwise only → inline the body
/// - Iterates to fixpoint (max 4 passes)
pub fn cleanup_identities<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    let mut current = stmts;
    for _ in 0..4 {
        let next = cleanup_pass(&current, expr_arena, stmt_arena);
        if next.len() == current.len() {
            // Quick structural check for fixpoint
            let mut same = true;
            for (a, b) in next.iter().zip(current.iter()) {
                if !stmt_structurally_equal(a, b) {
                    same = false;
                    break;
                }
            }
            if same {
                return next;
            }
        }
        current = next;
    }
    current
}

fn cleanup_pass<'a>(
    stmts: &[Stmt<'a>],
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    let mut result = Vec::with_capacity(stmts.len());

    for (i, stmt) in stmts.iter().enumerate() {
        match stmt {
            // Remove Let x = x (identity binding)
            Stmt::Let { var, value: Expr::Identifier(src), .. } if var == src => {
                continue;
            }
            // Collapse Let x = lit; Return x → Return lit
            Stmt::Let { var, value, mutable: false, .. } => {
                if let Some(Stmt::Return { value: Some(Expr::Identifier(ret_var)) }) = stmts.get(i + 1) {
                    if var == ret_var {
                        if let Expr::Literal(_) = value {
                            result.push(Stmt::Return { value: Some(value) });
                            // Skip the next Return statement too — handled below
                            continue;
                        }
                    }
                }
                // Recurse into sub-blocks
                result.push(cleanup_stmt(stmt, expr_arena, stmt_arena));
            }
            // Skip Return that was already folded into a Let
            Stmt::Return { value: Some(Expr::Identifier(ret_var)) } => {
                if i > 0 {
                    if let Stmt::Let { var, value: Expr::Literal(_), mutable: false, .. } = &stmts[i - 1] {
                        if var == ret_var {
                            continue;
                        }
                    }
                }
                result.push(cleanup_stmt(stmt, expr_arena, stmt_arena));
            }
            // Single-arm Inspect with only Otherwise → inline body
            Stmt::Inspect { arms, has_otherwise: true, .. } if arms.len() == 1 => {
                let arm = &arms[0];
                if arm.variant.is_none() {
                    // Otherwise-only: inline body
                    for s in arm.body {
                        result.push(cleanup_stmt(s, expr_arena, stmt_arena));
                    }
                    continue;
                }
                result.push(cleanup_stmt(stmt, expr_arena, stmt_arena));
            }
            _ => {
                result.push(cleanup_stmt(stmt, expr_arena, stmt_arena));
            }
        }
    }

    result
}

fn cleanup_stmt<'a>(
    stmt: &Stmt<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Stmt<'a> {
    match stmt {
        Stmt::If { cond, then_block, else_block } => {
            let new_then = cleanup_pass(then_block, expr_arena, stmt_arena);
            let new_else = else_block.map(|eb| {
                let cleaned = cleanup_pass(eb, expr_arena, stmt_arena);
                stmt_arena.alloc_slice(cleaned) as &[Stmt<'a>]
            });
            Stmt::If {
                cond,
                then_block: stmt_arena.alloc_slice(new_then),
                else_block: new_else,
            }
        }
        Stmt::While { cond, body, decreasing } => {
            let new_body = cleanup_pass(body, expr_arena, stmt_arena);
            Stmt::While {
                cond,
                body: stmt_arena.alloc_slice(new_body),
                decreasing: *decreasing,
            }
        }
        Stmt::FunctionDef { name, params, generics, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => {
            if *is_native {
                return stmt.clone();
            }
            let new_body = cleanup_pass(body, expr_arena, stmt_arena);
            Stmt::FunctionDef {
                name: *name,
                params: params.clone(),
                generics: generics.clone(),
                body: stmt_arena.alloc_slice(new_body),
                return_type: *return_type,
                is_native: *is_native,
                native_path: *native_path,
                is_exported: *is_exported,
                export_target: *export_target,
                opt_flags: opt_flags.clone(),
            }
        }
        Stmt::Repeat { pattern, iterable, body } => {
            let new_body = cleanup_pass(body, expr_arena, stmt_arena);
            Stmt::Repeat {
                pattern: pattern.clone(),
                iterable,
                body: stmt_arena.alloc_slice(new_body),
            }
        }
        other => other.clone(),
    }
}

fn stmt_structurally_equal(a: &Stmt, b: &Stmt) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}
