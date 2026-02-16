use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};
use super::fold;

/// Constant propagation pass.
///
/// Replaces references to immutable variables with their constant values
/// inside `Let` and `Set` value expressions, enabling cascading constant folding.
///
/// Only propagates variables that are:
/// - Declared without `mutable` keyword
/// - Never reassigned via `Set`
/// - Bound to a literal value (after previous fold pass)
///
/// Only substitutes inside `Let`/`Set` value expressions to preserve
/// readable codegen in conditions, returns, and other contexts.
pub fn propagate_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mutated = collect_all_set_targets(&stmts);
    let mut env: HashMap<Symbol, &'a Expr<'a>> = HashMap::new();
    propagate_block_stmts(stmts, &mut env, &mutated, expr_arena, stmt_arena, interner)
}

fn propagate_block_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    env: &mut HashMap<Symbol, &'a Expr<'a>>,
    mutated: &HashSet<Symbol>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    stmts.into_iter().map(|stmt| {
        propagate_stmt(stmt, env, mutated, expr_arena, stmt_arena, interner)
    }).collect()
}

fn propagate_nested_block<'a>(
    block: &'a [Stmt<'a>],
    env: &HashMap<Symbol, &'a Expr<'a>>,
    mutated: &HashSet<Symbol>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> &'a [Stmt<'a>] {
    let mut child_env = env.clone();
    let folded: Vec<Stmt<'a>> = block.iter().cloned().map(|stmt| {
        propagate_stmt(stmt, &mut child_env, mutated, expr_arena, stmt_arena, interner)
    }).collect();
    stmt_arena.alloc_slice(folded)
}

fn propagate_stmt<'a>(
    stmt: Stmt<'a>,
    env: &mut HashMap<Symbol, &'a Expr<'a>>,
    mutated: &HashSet<Symbol>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Stmt<'a> {
    match stmt {
        // Substitute + fold in Let values — this is where cascading happens
        Stmt::Let { var, ty, value, mutable } => {
            let propagated = subst_and_fold(value, env, expr_arena, stmt_arena, interner);
            if !mutable && !mutated.contains(&var) && is_propagatable_literal(propagated) {
                env.insert(var, propagated);
            }
            Stmt::Let { var, ty, value: propagated, mutable }
        }
        // Substitute + fold in Set values, and kill the target from env
        Stmt::Set { target, value } => {
            let propagated = subst_and_fold(value, env, expr_arena, stmt_arena, interner);
            env.remove(&target);
            Stmt::Set { target, value: propagated }
        }
        // Recurse into nested blocks (propagation env carries through)
        Stmt::If { cond, then_block, else_block } => Stmt::If {
            cond,
            then_block: propagate_nested_block(then_block, env, mutated, expr_arena, stmt_arena, interner),
            else_block: else_block.map(|b| propagate_nested_block(b, env, mutated, expr_arena, stmt_arena, interner)),
        },
        Stmt::While { cond, body, decreasing } => Stmt::While {
            cond,
            body: propagate_nested_block(body, env, mutated, expr_arena, stmt_arena, interner),
            decreasing,
        },
        Stmt::Repeat { pattern, iterable, body } => Stmt::Repeat {
            pattern,
            iterable,
            body: propagate_nested_block(body, env, mutated, expr_arena, stmt_arena, interner),
        },
        Stmt::FunctionDef { name, params, body, return_type, is_native, native_path, is_exported, export_target } => {
            let func_mutated = collect_all_set_targets_from_block(body);
            let mut func_env: HashMap<Symbol, &'a Expr<'a>> = HashMap::new();
            let new_body: Vec<Stmt<'a>> = body.iter().cloned().map(|stmt| {
                propagate_stmt(stmt, &mut func_env, &func_mutated, expr_arena, stmt_arena, interner)
            }).collect();
            Stmt::FunctionDef {
                name, params,
                body: stmt_arena.alloc_slice(new_body),
                return_type, is_native, native_path, is_exported, export_target,
            }
        }
        Stmt::Inspect { target, arms, has_otherwise } => Stmt::Inspect {
            target,
            arms: arms.into_iter().map(|arm| {
                crate::ast::stmt::MatchArm {
                    enum_name: arm.enum_name,
                    variant: arm.variant,
                    bindings: arm.bindings,
                    body: propagate_nested_block(arm.body, env, mutated, expr_arena, stmt_arena, interner),
                }
            }).collect(),
            has_otherwise,
        },
        Stmt::Zone { name, capacity, source_file, body } => Stmt::Zone {
            name, capacity, source_file,
            body: propagate_zone_block(body, env, mutated, expr_arena, stmt_arena, interner),
        },
        Stmt::Concurrent { tasks } => Stmt::Concurrent {
            tasks: propagate_nested_block(tasks, env, mutated, expr_arena, stmt_arena, interner),
        },
        Stmt::Parallel { tasks } => Stmt::Parallel {
            tasks: propagate_nested_block(tasks, env, mutated, expr_arena, stmt_arena, interner),
        },
        // All other statements pass through unchanged
        other => other,
    }
}

/// Propagate inside a zone body without registering zone-scoped bindings.
///
/// Zone-scoped variables must remain as identifiers so the escape checker
/// can detect assignments to outer-scope variables (Hotel California rule).
/// Substituting them with literals would hide escape violations.
fn propagate_zone_block<'a>(
    block: &'a [Stmt<'a>],
    env: &HashMap<Symbol, &'a Expr<'a>>,
    mutated: &HashSet<Symbol>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> &'a [Stmt<'a>] {
    let mut child_env = env.clone();
    let folded: Vec<Stmt<'a>> = block.iter().cloned().map(|stmt| {
        match stmt {
            // Substitute in Let values but do NOT add zone-scoped bindings to env
            Stmt::Let { var, ty, value, mutable } => {
                let propagated = subst_and_fold(value, &child_env, expr_arena, stmt_arena, interner);
                // Intentionally do NOT insert into child_env — zone-scoped vars
                // must stay as identifiers for escape analysis
                Stmt::Let { var, ty, value: propagated, mutable }
            }
            // For other statements, delegate to normal propagation
            other => propagate_stmt(other, &mut child_env, mutated, expr_arena, stmt_arena, interner),
        }
    }).collect();
    stmt_arena.alloc_slice(folded)
}

/// Only propagate Copy-type literals. Text literals produce heap-allocated
/// `String` values in Rust codegen — substituting them replaces a move with
/// independent allocations, which changes ownership semantics and hides
/// move-after-use errors (E0382) from rustc.
fn is_propagatable_literal(expr: &Expr) -> bool {
    matches!(expr, Expr::Literal(Literal::Number(_) | Literal::Float(_) | Literal::Boolean(_) | Literal::Nothing))
}

/// Substitute identifiers from env, then fold the result.
fn subst_and_fold<'a>(
    expr: &'a Expr<'a>,
    env: &HashMap<Symbol, &'a Expr<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> &'a Expr<'a> {
    let substituted = substitute_identifiers(expr, env, expr_arena);
    fold::fold_expr(substituted, expr_arena, stmt_arena, interner)
}

/// Recursively substitute identifiers with their constant values from env.
fn substitute_identifiers<'a>(
    expr: &'a Expr<'a>,
    env: &HashMap<Symbol, &'a Expr<'a>>,
    arena: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    if env.is_empty() {
        return expr;
    }
    match expr {
        Expr::Identifier(sym) => {
            if let Some(value) = env.get(sym) { value } else { expr }
        }
        Expr::BinaryOp { op, left, right } => {
            let sl = substitute_identifiers(left, env, arena);
            let sr = substitute_identifiers(right, env, arena);
            if std::ptr::eq(sl, *left) && std::ptr::eq(sr, *right) {
                expr
            } else {
                arena.alloc(Expr::BinaryOp { op: *op, left: sl, right: sr })
            }
        }
        Expr::Call { function, args } => {
            let sa: Vec<&'a Expr<'a>> = args.iter().map(|a| substitute_identifiers(a, env, arena)).collect();
            let changed = sa.iter().zip(args.iter()).any(|(s, o)| !std::ptr::eq(*s, *o));
            if changed { arena.alloc(Expr::Call { function: *function, args: sa }) } else { expr }
        }
        Expr::CallExpr { callee, args } => {
            let sc = substitute_identifiers(callee, env, arena);
            let sa: Vec<&'a Expr<'a>> = args.iter().map(|a| substitute_identifiers(a, env, arena)).collect();
            let args_changed = sa.iter().zip(args.iter()).any(|(s, o)| !std::ptr::eq(*s, *o));
            if std::ptr::eq(sc, *callee) && !args_changed { expr }
            else { arena.alloc(Expr::CallExpr { callee: sc, args: sa }) }
        }
        // Don't substitute inside Index/Slice — preserves AST shape for
        // swap/for-range/vec-fill pattern detection in codegen
        Expr::Index { .. } => expr,
        Expr::Slice { .. } => expr,
        Expr::Contains { collection, value } => {
            let sc = substitute_identifiers(collection, env, arena);
            let sv = substitute_identifiers(value, env, arena);
            if std::ptr::eq(sc, *collection) && std::ptr::eq(sv, *value) { expr }
            else { arena.alloc(Expr::Contains { collection: sc, value: sv }) }
        }
        Expr::Union { left, right } => {
            let sl = substitute_identifiers(left, env, arena);
            let sr = substitute_identifiers(right, env, arena);
            if std::ptr::eq(sl, *left) && std::ptr::eq(sr, *right) { expr }
            else { arena.alloc(Expr::Union { left: sl, right: sr }) }
        }
        Expr::Intersection { left, right } => {
            let sl = substitute_identifiers(left, env, arena);
            let sr = substitute_identifiers(right, env, arena);
            if std::ptr::eq(sl, *left) && std::ptr::eq(sr, *right) { expr }
            else { arena.alloc(Expr::Intersection { left: sl, right: sr }) }
        }
        Expr::Range { start, end } => {
            let ss = substitute_identifiers(start, env, arena);
            let se = substitute_identifiers(end, env, arena);
            if std::ptr::eq(ss, *start) && std::ptr::eq(se, *end) { expr }
            else { arena.alloc(Expr::Range { start: ss, end: se }) }
        }
        Expr::ChunkAt { index, zone } => {
            let si = substitute_identifiers(index, env, arena);
            let sz = substitute_identifiers(zone, env, arena);
            if std::ptr::eq(si, *index) && std::ptr::eq(sz, *zone) { expr }
            else { arena.alloc(Expr::ChunkAt { index: si, zone: sz }) }
        }
        Expr::WithCapacity { value, capacity } => {
            let sv = substitute_identifiers(value, env, arena);
            let sc = substitute_identifiers(capacity, env, arena);
            if std::ptr::eq(sv, *value) && std::ptr::eq(sc, *capacity) { expr }
            else { arena.alloc(Expr::WithCapacity { value: sv, capacity: sc }) }
        }
        Expr::Copy { expr: inner } => {
            let si = substitute_identifiers(inner, env, arena);
            if std::ptr::eq(si, *inner) { expr } else { arena.alloc(Expr::Copy { expr: si }) }
        }
        Expr::Give { value } => {
            let sv = substitute_identifiers(value, env, arena);
            if std::ptr::eq(sv, *value) { expr } else { arena.alloc(Expr::Give { value: sv }) }
        }
        Expr::Length { collection } => {
            let sc = substitute_identifiers(collection, env, arena);
            if std::ptr::eq(sc, *collection) { expr } else { arena.alloc(Expr::Length { collection: sc }) }
        }
        Expr::ManifestOf { zone } => {
            let sz = substitute_identifiers(zone, env, arena);
            if std::ptr::eq(sz, *zone) { expr } else { arena.alloc(Expr::ManifestOf { zone: sz }) }
        }
        Expr::FieldAccess { object, field } => {
            let so = substitute_identifiers(object, env, arena);
            if std::ptr::eq(so, *object) { expr } else { arena.alloc(Expr::FieldAccess { object: so, field: *field }) }
        }
        Expr::OptionSome { value } => {
            let sv = substitute_identifiers(value, env, arena);
            if std::ptr::eq(sv, *value) { expr } else { arena.alloc(Expr::OptionSome { value: sv }) }
        }
        Expr::List(elems) => {
            let se: Vec<&'a Expr<'a>> = elems.iter().map(|e| substitute_identifiers(e, env, arena)).collect();
            let changed = se.iter().zip(elems.iter()).any(|(s, o)| !std::ptr::eq(*s, *o));
            if changed { arena.alloc(Expr::List(se)) } else { expr }
        }
        Expr::Tuple(elems) => {
            let se: Vec<&'a Expr<'a>> = elems.iter().map(|e| substitute_identifiers(e, env, arena)).collect();
            let changed = se.iter().zip(elems.iter()).any(|(s, o)| !std::ptr::eq(*s, *o));
            if changed { arena.alloc(Expr::Tuple(se)) } else { expr }
        }
        Expr::New { type_name, type_args, init_fields } => {
            let sf: Vec<(Symbol, &'a Expr<'a>)> = init_fields.iter()
                .map(|(n, v)| (*n, substitute_identifiers(v, env, arena)))
                .collect();
            let changed = sf.iter().zip(init_fields.iter()).any(|((_, sv), (_, ov))| !std::ptr::eq(*sv, *ov));
            if changed { arena.alloc(Expr::New { type_name: *type_name, type_args: type_args.clone(), init_fields: sf }) }
            else { expr }
        }
        Expr::NewVariant { enum_name, variant, fields } => {
            let sf: Vec<(Symbol, &'a Expr<'a>)> = fields.iter()
                .map(|(n, v)| (*n, substitute_identifiers(v, env, arena)))
                .collect();
            let changed = sf.iter().zip(fields.iter()).any(|((_, sv), (_, ov))| !std::ptr::eq(*sv, *ov));
            if changed { arena.alloc(Expr::NewVariant { enum_name: *enum_name, variant: *variant, fields: sf }) }
            else { expr }
        }
        // Don't propagate into closures (captured variables may change)
        Expr::Closure { .. } => expr,
        // Leaves
        Expr::Literal(_) | Expr::OptionNone | Expr::Escape { .. } => expr,
    }
}

/// Collect all variables that appear as `Set` targets (recursively through all blocks).
fn collect_all_set_targets(stmts: &[Stmt]) -> HashSet<Symbol> {
    let mut targets = HashSet::new();
    for stmt in stmts {
        collect_set_targets_in_stmt(stmt, &mut targets);
    }
    targets
}

fn collect_all_set_targets_from_block(block: &[Stmt]) -> HashSet<Symbol> {
    let mut targets = HashSet::new();
    for stmt in block {
        collect_set_targets_in_stmt(stmt, &mut targets);
    }
    targets
}

fn collect_set_targets_in_stmt(stmt: &Stmt, targets: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Set { target, .. } => { targets.insert(*target); }
        Stmt::If { then_block, else_block, .. } => {
            for s in *then_block { collect_set_targets_in_stmt(s, targets); }
            if let Some(eb) = else_block {
                for s in *eb { collect_set_targets_in_stmt(s, targets); }
            }
        }
        Stmt::While { body, .. } => {
            for s in *body { collect_set_targets_in_stmt(s, targets); }
        }
        Stmt::Repeat { body, .. } => {
            for s in *body { collect_set_targets_in_stmt(s, targets); }
        }
        Stmt::Zone { body, .. } => {
            for s in *body { collect_set_targets_in_stmt(s, targets); }
        }
        Stmt::Concurrent { tasks } => {
            for s in *tasks { collect_set_targets_in_stmt(s, targets); }
        }
        Stmt::Parallel { tasks } => {
            for s in *tasks { collect_set_targets_in_stmt(s, targets); }
        }
        Stmt::Inspect { arms, .. } => {
            for arm in arms {
                for s in arm.body { collect_set_targets_in_stmt(s, targets); }
            }
        }
        _ => {}
    }
}
