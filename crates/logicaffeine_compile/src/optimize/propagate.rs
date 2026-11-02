use std::collections::{HashMap, HashSet};

use super::capture::{self, EntryBinders};
use super::fold;
use super::writes::{CalleeGlobals, WriteSet};
use crate::arena::Arena;
use crate::ast::stmt::{Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};

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
/// A binding ends what was known of the name it shadows — a later `Let`, a `Receive`, a loop's pattern
/// inside the loop, a match arm's bindings inside the arm — whether or not the new value is a literal.
///
/// Only substitutes inside `Let`/`Set` value expressions to preserve
/// readable codegen in conditions, returns, and other contexts.
pub fn propagate_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let program = WriteSet::of_block(&stmts);
    if program.is_opaque() {
        // Raw foreign code at the top level may rebind anything: nothing is a propagatable constant.
        return stmts;
    }
    let mut mutated: HashSet<Symbol> = program.reassigned().collect();
    // A variable a FUNCTION assigns without declaring locally is a GLOBAL it mutates. A call to that function
    // can change it under the caller, so it must never be treated as a propagatable constant anywhere — even
    // though it looks like a plain `Let`-bound local at the call site. (Soundness: `Set counter` inside `bump`
    // makes `counter` global, so `Let counter be 0; bump(); Show counter` must NOT fold to `0`.)
    let globals = CalleeGlobals::of_program(&stmts);
    if globals.opaque {
        // Some callee runs raw foreign code: a call may write anything, so nothing is a propagatable constant.
        return stmts;
    }
    mutated.extend(globals.symbols.iter().copied());
    // The Bool/Text symbol analysis for this scope — passed to `subst_and_fold` so
    // the folder it invokes refuses `s + 0 → s` on a Text `s` (an empty `BoolSyms`
    // here would drop the "0", the bug this pass otherwise reintroduces after fold).
    let bools = fold::program_bool_syms(&stmts, interner);
    let mut env: HashMap<Symbol, &'a Expr<'a>> = HashMap::new();
    propagate_block_stmts(
        stmts, &mut env, &mutated, &globals, &bools, expr_arena, stmt_arena, interner,
    )
}

fn propagate_block_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    env: &mut HashMap<Symbol, &'a Expr<'a>>,
    mutated: &HashSet<Symbol>,
    globals: &CalleeGlobals,
    bools: &fold::BoolSyms,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    stmts
        .into_iter()
        .map(|stmt| {
            propagate_stmt(
                stmt, env, mutated, globals, bools, expr_arena, stmt_arena, interner,
            )
        })
        .collect()
}

fn propagate_nested_block<'a>(
    block: &'a [Stmt<'a>],
    env: &HashMap<Symbol, &'a Expr<'a>>,
    binders: &[Symbol],
    mutated: &HashSet<Symbol>,
    globals: &CalleeGlobals,
    bools: &fold::BoolSyms,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> &'a [Stmt<'a>] {
    let mut child_env = env.clone();
    for b in binders {
        child_env.remove(b);
    }
    let folded: Vec<Stmt<'a>> = block
        .iter()
        .cloned()
        .map(|stmt| {
            propagate_stmt(
                stmt,
                &mut child_env,
                mutated,
                globals,
                bools,
                expr_arena,
                stmt_arena,
                interner,
            )
        })
        .collect();
    stmt_arena.alloc_slice(folded)
}

fn propagate_stmt<'a>(
    stmt: Stmt<'a>,
    env: &mut HashMap<Symbol, &'a Expr<'a>>,
    mutated: &HashSet<Symbol>,
    globals: &CalleeGlobals,
    bools: &fold::BoolSyms,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Stmt<'a> {
    let entry = EntryBinders::of(&stmt);
    let declared = match &stmt {
        Stmt::Let { .. } => HashSet::new(),
        other => capture::own_names(std::slice::from_ref(other)),
    };
    let out = match stmt {
        // Substitute + fold in Let values — this is where cascading happens
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => {
            let propagated = subst_and_fold(value, env, bools, expr_arena, stmt_arena, interner);
            env.remove(&var);
            if !mutable && !mutated.contains(&var) && is_propagatable_literal(propagated) {
                env.insert(var, propagated);
            }
            Stmt::Let {
                var,
                ty,
                value: propagated,
                mutable,
            }
        }
        // Substitute + fold in Set values, and kill the target from env
        Stmt::Set { target, value } => {
            let propagated = subst_and_fold(value, env, bools, expr_arena, stmt_arena, interner);
            env.remove(&target);
            Stmt::Set {
                target,
                value: propagated,
            }
        }
        // Substitute + fold in conditions and recurse into nested blocks
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => Stmt::If {
            cond: subst_and_fold(cond, env, bools, expr_arena, stmt_arena, interner),
            then_block: propagate_nested_block(
                then_block,
                env,
                entry.of_block(then_block),
                mutated,
                globals,
                bools,
                expr_arena,
                stmt_arena,
                interner,
            ),
            else_block: else_block.map(|b| {
                propagate_nested_block(
                    b,
                    env,
                    entry.of_block(b),
                    mutated,
                    globals,
                    bools,
                    expr_arena,
                    stmt_arena,
                    interner,
                )
            }),
        },
        Stmt::While {
            cond,
            body,
            decreasing,
        } => Stmt::While {
            cond: subst_and_fold(cond, env, bools, expr_arena, stmt_arena, interner),
            body: propagate_nested_block(
                body,
                env,
                entry.of_block(body),
                mutated,
                globals,
                bools,
                expr_arena,
                stmt_arena,
                interner,
            ),
            decreasing,
        },
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => Stmt::Repeat {
            pattern,
            iterable: subst_and_fold(iterable, env, bools, expr_arena, stmt_arena, interner),
            body: propagate_nested_block(
                body,
                env,
                entry.of_block(body),
                mutated,
                globals,
                bools,
                expr_arena,
                stmt_arena,
                interner,
            ),
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
        } => {
            let func_writes = WriteSet::of_block(body).with_callee_globals(globals);
            let func_mutated: HashSet<Symbol> = if func_writes.is_opaque() {
                // Raw foreign code in the body may rebind anything: treat every name as mutated.
                WriteSet::of_block(body)
                    .symbols()
                    .chain(params.iter().map(|(p, _)| *p))
                    .collect()
            } else {
                func_writes.reassigned().collect()
            };
            let func_bools = fold::function_bool_syms(bools, name, &params, body, interner);
            let mut func_env: HashMap<Symbol, &'a Expr<'a>> = HashMap::new();
            let new_body: Vec<Stmt<'a>> = body
                .iter()
                .cloned()
                .map(|stmt| {
                    propagate_stmt(
                        stmt,
                        &mut func_env,
                        &func_mutated,
                        globals,
                        &func_bools,
                        expr_arena,
                        stmt_arena,
                        interner,
                    )
                })
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
        Stmt::Inspect {
            target,
            arms,
            has_otherwise,
        } => Stmt::Inspect {
            target,
            arms: arms
                .into_iter()
                .map(|arm| crate::ast::stmt::MatchArm {
                    enum_name: arm.enum_name,
                    variant: arm.variant,
                    bindings: arm.bindings,
                    body: propagate_nested_block(
                        arm.body,
                        env,
                        entry.of_block(arm.body),
                        mutated,
                        globals,
                        bools,
                        expr_arena,
                        stmt_arena,
                        interner,
                    ),
                })
                .collect(),
            has_otherwise,
        },
        Stmt::Zone {
            name,
            capacity,
            source_file,
            body,
        } => Stmt::Zone {
            name,
            capacity,
            source_file,
            body: propagate_zone_block(
                body,
                env,
                entry.of_block(body),
                mutated,
                globals,
                bools,
                expr_arena,
                stmt_arena,
                interner,
            ),
        },
        Stmt::Concurrent { tasks } => Stmt::Concurrent {
            tasks: propagate_nested_block(
                tasks,
                env,
                entry.of_block(tasks),
                mutated,
                globals,
                bools,
                expr_arena,
                stmt_arena,
                interner,
            ),
        },
        Stmt::Parallel { tasks } => Stmt::Parallel {
            tasks: propagate_nested_block(
                tasks,
                env,
                entry.of_block(tasks),
                mutated,
                globals,
                bools,
                expr_arena,
                stmt_arena,
                interner,
            ),
        },
        // Substitute + fold in Return, Show, Push, Call, and other expression-bearing stmts
        Stmt::Return { value } => Stmt::Return {
            value: value.map(|v| subst_and_fold(v, env, bools, expr_arena, stmt_arena, interner)),
        },
        // A recipient is the function the value is handed to, never a value read: a local that
        // shares its name must not be substituted in (a call always means the function).
        Stmt::Show { object, recipient } => Stmt::Show {
            object: subst_and_fold(object, env, bools, expr_arena, stmt_arena, interner),
            recipient,
        },
        Stmt::Push { value, collection } => Stmt::Push {
            value: subst_and_fold(value, env, bools, expr_arena, stmt_arena, interner),
            collection,
        },
        Stmt::Call { function, args } => Stmt::Call {
            function,
            args: args
                .into_iter()
                .map(|a| subst_and_fold(a, env, bools, expr_arena, stmt_arena, interner))
                .collect(),
        },
        Stmt::RuntimeAssert {
            condition,
            hard,
            dim,
            reason,
        } => Stmt::RuntimeAssert {
            condition: subst_and_fold(condition, env, bools, expr_arena, stmt_arena, interner),
            hard,
            dim,
            reason: reason.map(|r| subst_and_fold(r, env, bools, expr_arena, stmt_arena, interner)),
        },
        // Don't substitute into SetIndex index — preserves AST shape for
        // swap pattern detection in codegen (matching Expr::Index behavior)
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => Stmt::SetIndex {
            collection,
            index,
            value: subst_and_fold(value, env, bools, expr_arena, stmt_arena, interner),
        },
        Stmt::SetField {
            object,
            field,
            value,
        } => Stmt::SetField {
            object,
            field,
            value: subst_and_fold(value, env, bools, expr_arena, stmt_arena, interner),
        },
        Stmt::Give { object, recipient } => Stmt::Give {
            object: subst_and_fold(object, env, bools, expr_arena, stmt_arena, interner),
            recipient,
        },
        // All other statements pass through unchanged
        other => other,
    };
    for name in &declared {
        env.remove(name);
    }
    out
}

/// Propagate inside a zone body without registering zone-scoped bindings.
///
/// Zone-scoped variables must remain as identifiers so the escape checker
/// can detect assignments to outer-scope variables (the zone containment rule).
/// Substituting them with literals would hide escape violations.
fn propagate_zone_block<'a>(
    block: &'a [Stmt<'a>],
    env: &HashMap<Symbol, &'a Expr<'a>>,
    binders: &[Symbol],
    mutated: &HashSet<Symbol>,
    globals: &CalleeGlobals,
    bools: &fold::BoolSyms,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> &'a [Stmt<'a>] {
    let mut child_env = env.clone();
    for b in binders {
        child_env.remove(b);
    }
    let folded: Vec<Stmt<'a>> = block
        .iter()
        .cloned()
        .map(|stmt| {
            match stmt {
                // Substitute in Let values but do NOT add zone-scoped bindings to env
                Stmt::Let {
                    var,
                    ty,
                    value,
                    mutable,
                } => {
                    let propagated =
                        subst_and_fold(value, &child_env, bools, expr_arena, stmt_arena, interner);
                    child_env.remove(&var);
                    // Intentionally do NOT insert into child_env — zone-scoped vars
                    // must stay as identifiers for escape analysis
                    Stmt::Let {
                        var,
                        ty,
                        value: propagated,
                        mutable,
                    }
                }
                // For other statements, delegate to normal propagation
                other => propagate_stmt(
                    other,
                    &mut child_env,
                    mutated,
                    globals,
                    bools,
                    expr_arena,
                    stmt_arena,
                    interner,
                ),
            }
        })
        .collect();
    stmt_arena.alloc_slice(folded)
}

fn is_propagatable_literal(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Literal(
            Literal::Number(_)
                | Literal::Float(_)
                | Literal::Boolean(_)
                | Literal::Nothing
                | Literal::Text(_)
        )
    )
}

/// Substitute identifiers from env, then fold the result. `pub(crate)` so SCCP
/// ([`super::sccp`]) reuses the SAME substitution — single-sourcing the refusal
/// to substitute into `Index`/`Slice`/`Closure`/interpolated strings.
pub(crate) fn subst_and_fold<'a>(
    expr: &'a Expr<'a>,
    env: &HashMap<Symbol, &'a Expr<'a>>,
    bools: &fold::BoolSyms,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> &'a Expr<'a> {
    // A mutable argument names storage, and a later operand may read its new
    // value. Constants known before this expression cannot replace either use.
    let writes = WriteSet::of_expr(expr);
    let stable;
    let env = if env.keys().any(|name| writes.may_write(*name)) {
        stable = env
            .iter()
            .filter(|(name, _)| !writes.may_write(**name))
            .map(|(name, value)| (*name, *value))
            .collect();
        &stable
    } else {
        env
    };
    let substituted = substitute_identifiers(expr, env, expr_arena);
    fold::fold_expr(substituted, expr_arena, stmt_arena, interner, bools)
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
            if let Some(value) = env.get(sym) {
                value
            } else {
                expr
            }
        }
        Expr::BinaryOp { op, left, right } => {
            let sl = substitute_identifiers(left, env, arena);
            let sr = substitute_identifiers(right, env, arena);
            if std::ptr::eq(sl, *left) && std::ptr::eq(sr, *right) {
                expr
            } else {
                arena.alloc(Expr::BinaryOp {
                    op: *op,
                    left: sl,
                    right: sr,
                })
            }
        }
        Expr::Call { function, args } => {
            let sa: Vec<&'a Expr<'a>> = args
                .iter()
                .map(|a| substitute_identifiers(a, env, arena))
                .collect();
            let changed = sa
                .iter()
                .zip(args.iter())
                .any(|(s, o)| !std::ptr::eq(*s, *o));
            if changed {
                arena.alloc(Expr::Call {
                    function: *function,
                    args: sa,
                })
            } else {
                expr
            }
        }
        Expr::CallExpr { callee, args } => {
            let sc = substitute_identifiers(callee, env, arena);
            let sa: Vec<&'a Expr<'a>> = args
                .iter()
                .map(|a| substitute_identifiers(a, env, arena))
                .collect();
            let args_changed = sa
                .iter()
                .zip(args.iter())
                .any(|(s, o)| !std::ptr::eq(*s, *o));
            if std::ptr::eq(sc, *callee) && !args_changed {
                expr
            } else {
                arena.alloc(Expr::CallExpr {
                    callee: sc,
                    args: sa,
                })
            }
        }
        // Don't substitute inside Index/Slice — preserves AST shape for
        // swap/for-range/vec-fill pattern detection in codegen
        Expr::Index { .. } => expr,
        Expr::Slice { .. } => expr,
        Expr::Contains { collection, value } => {
            let sc = substitute_identifiers(collection, env, arena);
            let sv = substitute_identifiers(value, env, arena);
            if std::ptr::eq(sc, *collection) && std::ptr::eq(sv, *value) {
                expr
            } else {
                arena.alloc(Expr::Contains {
                    collection: sc,
                    value: sv,
                })
            }
        }
        Expr::Union { left, right } => {
            let sl = substitute_identifiers(left, env, arena);
            let sr = substitute_identifiers(right, env, arena);
            if std::ptr::eq(sl, *left) && std::ptr::eq(sr, *right) {
                expr
            } else {
                arena.alloc(Expr::Union {
                    left: sl,
                    right: sr,
                })
            }
        }
        Expr::Intersection { left, right } => {
            let sl = substitute_identifiers(left, env, arena);
            let sr = substitute_identifiers(right, env, arena);
            if std::ptr::eq(sl, *left) && std::ptr::eq(sr, *right) {
                expr
            } else {
                arena.alloc(Expr::Intersection {
                    left: sl,
                    right: sr,
                })
            }
        }
        Expr::Range { start, end } => {
            let ss = substitute_identifiers(start, env, arena);
            let se = substitute_identifiers(end, env, arena);
            if std::ptr::eq(ss, *start) && std::ptr::eq(se, *end) {
                expr
            } else {
                arena.alloc(Expr::Range { start: ss, end: se })
            }
        }
        Expr::ChunkAt { index, zone } => {
            let si = substitute_identifiers(index, env, arena);
            let sz = substitute_identifiers(zone, env, arena);
            if std::ptr::eq(si, *index) && std::ptr::eq(sz, *zone) {
                expr
            } else {
                arena.alloc(Expr::ChunkAt {
                    index: si,
                    zone: sz,
                })
            }
        }
        Expr::WithCapacity { value, capacity } => {
            let sv = substitute_identifiers(value, env, arena);
            let sc = substitute_identifiers(capacity, env, arena);
            if std::ptr::eq(sv, *value) && std::ptr::eq(sc, *capacity) {
                expr
            } else {
                arena.alloc(Expr::WithCapacity {
                    value: sv,
                    capacity: sc,
                })
            }
        }
        Expr::Copy { expr: inner } => {
            let si = substitute_identifiers(inner, env, arena);
            if std::ptr::eq(si, *inner) {
                expr
            } else {
                arena.alloc(Expr::Copy { expr: si })
            }
        }
        Expr::Give { value } => {
            let sv = substitute_identifiers(value, env, arena);
            if std::ptr::eq(sv, *value) {
                expr
            } else {
                arena.alloc(Expr::Give { value: sv })
            }
        }
        Expr::Length { collection } => {
            let sc = substitute_identifiers(collection, env, arena);
            if std::ptr::eq(sc, *collection) {
                expr
            } else {
                arena.alloc(Expr::Length { collection: sc })
            }
        }
        Expr::ManifestOf { zone } => {
            let sz = substitute_identifiers(zone, env, arena);
            if std::ptr::eq(sz, *zone) {
                expr
            } else {
                arena.alloc(Expr::ManifestOf { zone: sz })
            }
        }
        Expr::FieldAccess { object, field } => {
            let so = substitute_identifiers(object, env, arena);
            if std::ptr::eq(so, *object) {
                expr
            } else {
                arena.alloc(Expr::FieldAccess {
                    object: so,
                    field: *field,
                })
            }
        }
        Expr::OptionSome { value } => {
            let sv = substitute_identifiers(value, env, arena);
            if std::ptr::eq(sv, *value) {
                expr
            } else {
                arena.alloc(Expr::OptionSome { value: sv })
            }
        }
        Expr::Not { operand } => {
            let so = substitute_identifiers(operand, env, arena);
            if std::ptr::eq(so, *operand) {
                expr
            } else {
                arena.alloc(Expr::Not { operand: so })
            }
        }
        Expr::List(elems) => {
            let se: Vec<&'a Expr<'a>> = elems
                .iter()
                .map(|e| substitute_identifiers(e, env, arena))
                .collect();
            let changed = se
                .iter()
                .zip(elems.iter())
                .any(|(s, o)| !std::ptr::eq(*s, *o));
            if changed {
                arena.alloc(Expr::List(se))
            } else {
                expr
            }
        }
        Expr::Tuple(elems) => {
            let se: Vec<&'a Expr<'a>> = elems
                .iter()
                .map(|e| substitute_identifiers(e, env, arena))
                .collect();
            let changed = se
                .iter()
                .zip(elems.iter())
                .any(|(s, o)| !std::ptr::eq(*s, *o));
            if changed {
                arena.alloc(Expr::Tuple(se))
            } else {
                expr
            }
        }
        Expr::New {
            type_name,
            type_args,
            init_fields,
        } => {
            let sf: Vec<(Symbol, &'a Expr<'a>)> = init_fields
                .iter()
                .map(|(n, v)| (*n, substitute_identifiers(v, env, arena)))
                .collect();
            let changed = sf
                .iter()
                .zip(init_fields.iter())
                .any(|((_, sv), (_, ov))| !std::ptr::eq(*sv, *ov));
            if changed {
                arena.alloc(Expr::New {
                    type_name: *type_name,
                    type_args: type_args.clone(),
                    init_fields: sf,
                })
            } else {
                expr
            }
        }
        Expr::NewVariant {
            enum_name,
            variant,
            fields,
        } => {
            let sf: Vec<(Symbol, &'a Expr<'a>)> = fields
                .iter()
                .map(|(n, v)| (*n, substitute_identifiers(v, env, arena)))
                .collect();
            let changed = sf
                .iter()
                .zip(fields.iter())
                .any(|((_, sv), (_, ov))| !std::ptr::eq(*sv, *ov));
            if changed {
                arena.alloc(Expr::NewVariant {
                    enum_name: *enum_name,
                    variant: *variant,
                    fields: sf,
                })
            } else {
                expr
            }
        }
        // Don't propagate into closures (captured variables may change)
        Expr::Closure { .. } => expr,
        // Interpolated strings — don't substitute (preserves format string shape)
        Expr::InterpolatedString(_) | Expr::View(_) | Expr::Dom(_) => expr,
        // Leaves
        Expr::Literal(_) | Expr::OptionNone | Expr::Escape { .. } => expr,
    }
}
