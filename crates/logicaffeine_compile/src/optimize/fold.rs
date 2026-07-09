use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, ClosureBody, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

/// Symbols statically known to hold a Bool in the current scope — the license
/// for identity rewrites (`x && true → x`, `!!x → x`) that RETURN an operand
/// unwrapped. Conservative: a symbol qualifies via a `Bool` param/`Let`
/// annotation, or when EVERY write to it (Let and Set alike) is a
/// Bool-producing expression. Anything aliased or uncertain stays out.
pub type BoolSyms = std::collections::HashSet<Symbol>;

fn type_is_bool(ty: &TypeExpr, interner: &Interner) -> bool {
    matches!(ty, TypeExpr::Primitive(s) | TypeExpr::Named(s) if interner.resolve(*s) == "Bool")
}

/// Every `Let`/`Set` write in `stmts` (nested blocks included; nested
/// `FunctionDef`s excluded — they are their own scope).
fn collect_writes<'a, 'e>(stmts: &'a [Stmt<'e>], out: &mut Vec<(Symbol, Option<&'a TypeExpr<'e>>, &'a Expr<'e>)>) {
    for st in stmts {
        match st {
            Stmt::Let { var, ty, value, .. } => out.push((*var, *ty, value)),
            Stmt::Set { target, value } => out.push((*target, None, value)),
            Stmt::If { then_block, else_block, .. } => {
                collect_writes(then_block, out);
                if let Some(b) = else_block {
                    collect_writes(b, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                collect_writes(body, out)
            }
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    collect_writes(arm.body, out);
                }
            }
            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => collect_writes(tasks, out),
            _ => {}
        }
    }
}

fn collect_bool_syms(params: &[(Symbol, &TypeExpr)], body: &[Stmt], interner: &Interner) -> BoolSyms {
    let mut bools: BoolSyms = params
        .iter()
        .filter(|(_, ty)| type_is_bool(ty, interner))
        .map(|(s, _)| *s)
        .collect();
    let mut writes = Vec::new();
    collect_writes(body, &mut writes);
    // Two rounds admit `Let y be x.` chains off a Bool param; each round only
    // ADDS symbols whose every write is boolish under the current set, then
    // re-verifies, so the result is sound at any iteration count.
    for _ in 0..2 {
        let mut changed = false;
        let candidates: std::collections::HashSet<Symbol> =
            writes.iter().map(|(s, _, _)| *s).collect();
        for sym in candidates {
            if bools.contains(&sym) {
                continue;
            }
            let all_bool = writes.iter().filter(|(s, _, _)| *s == sym).all(|(_, ty, v)| {
                ty.map(|t| type_is_bool(t, interner)).unwrap_or(false) || expr_is_boolish(v, &bools)
            });
            if all_bool {
                bools.insert(sym);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Re-verify: drop anything whose writes stopped qualifying (a symbol
    // admitted early but also written non-boolishly elsewhere can't survive
    // because `all` above already spans every write — this is belt-and-braces
    // for order effects between rounds).
    let snapshot = bools.clone();
    bools.retain(|sym| {
        writes.iter().filter(|(s, _, _)| s == sym).all(|(_, ty, v)| {
            ty.map(|t| type_is_bool(t, interner)).unwrap_or(false) || expr_is_boolish(v, &snapshot)
        }) || params.iter().any(|(p, ty)| p == sym && type_is_bool(ty, interner))
    });
    bools
}

pub fn fold_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let bools = collect_bool_syms(&[], &stmts, interner);
    stmts
        .into_iter()
        .map(|stmt| fold_stmt(stmt, expr_arena, stmt_arena, interner, &bools))
        .collect()
}

fn fold_block<'a>(
    block: &'a [Stmt<'a>],
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    bools: &BoolSyms,
) -> &'a [Stmt<'a>] {
    let folded: Vec<Stmt<'a>> = block
        .iter()
        .cloned()
        .map(|stmt| fold_stmt(stmt, expr_arena, stmt_arena, interner, bools))
        .collect();
    stmt_arena.alloc_slice(folded)
}

fn fold_stmt<'a>(
    stmt: Stmt<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    bools: &BoolSyms,
) -> Stmt<'a> {
    match stmt {
        Stmt::Let { var, ty, value, mutable } => Stmt::Let {
            var,
            ty,
            value: fold_expr(value, expr_arena, stmt_arena, interner, bools),
            mutable,
        },
        Stmt::Set { target, value } => Stmt::Set {
            target,
            value: fold_expr(value, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::If { cond, then_block, else_block } => Stmt::If {
            cond: fold_expr(cond, expr_arena, stmt_arena, interner, bools),
            then_block: fold_block(then_block, expr_arena, stmt_arena, interner, bools),
            else_block: else_block.map(|b| fold_block(b, expr_arena, stmt_arena, interner, bools)),
        },
        Stmt::While { cond, body, decreasing } => Stmt::While {
            cond: fold_expr(cond, expr_arena, stmt_arena, interner, bools),
            body: fold_block(body, expr_arena, stmt_arena, interner, bools),
            decreasing,
        },
        Stmt::Repeat { pattern, iterable, body } => Stmt::Repeat {
            pattern,
            iterable: fold_expr(iterable, expr_arena, stmt_arena, interner, bools),
            body: fold_block(body, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::FunctionDef { name, params, generics, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => {
            // A function body is its own scope: derive its Bool symbols from
            // the typed params plus its own writes.
            let fn_bools = collect_bool_syms(&params, body, interner);
            Stmt::FunctionDef {
            name,
            params,
            generics,
            body: fold_block(body, expr_arena, stmt_arena, interner, &fn_bools),
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
            opt_flags,
        }},
        Stmt::Show { object, recipient } => Stmt::Show {
            object: fold_expr(object, expr_arena, stmt_arena, interner, bools),
            recipient,
        },
        Stmt::Return { value } => Stmt::Return {
            value: value.map(|v| fold_expr(v, expr_arena, stmt_arena, interner, bools)),
        },
        Stmt::RuntimeAssert { condition, hard } => Stmt::RuntimeAssert {
            condition: fold_expr(condition, expr_arena, stmt_arena, interner, bools),
            hard,
        },
        Stmt::Push { value, collection } => Stmt::Push {
            value: fold_expr(value, expr_arena, stmt_arena, interner, bools),
            collection,
        },
        Stmt::SetField { object, field, value } => Stmt::SetField {
            object,
            field,
            value: fold_expr(value, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::SetIndex { collection, index, value } => Stmt::SetIndex {
            collection,
            index: fold_expr(index, expr_arena, stmt_arena, interner, bools),
            value: fold_expr(value, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::Call { function, args } => Stmt::Call {
            function,
            args: args.into_iter().map(|a| fold_expr(a, expr_arena, stmt_arena, interner, bools)).collect(),
        },
        Stmt::Give { object, recipient } => Stmt::Give {
            object: fold_expr(object, expr_arena, stmt_arena, interner, bools),
            recipient: fold_expr(recipient, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::Inspect { target, arms, has_otherwise } => Stmt::Inspect {
            target: fold_expr(target, expr_arena, stmt_arena, interner, bools),
            arms: arms.into_iter().map(|arm| {
                crate::ast::stmt::MatchArm {
                    enum_name: arm.enum_name,
                    variant: arm.variant,
                    bindings: arm.bindings,
                    body: fold_block(arm.body, expr_arena, stmt_arena, interner, bools),
                }
            }).collect(),
            has_otherwise,
        },
        Stmt::Pop { collection, into } => Stmt::Pop {
            collection: fold_expr(collection, expr_arena, stmt_arena, interner, bools),
            into,
        },
        Stmt::Add { value, collection } => Stmt::Add {
            value: fold_expr(value, expr_arena, stmt_arena, interner, bools),
            collection: fold_expr(collection, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::Remove { value, collection } => Stmt::Remove {
            value: fold_expr(value, expr_arena, stmt_arena, interner, bools),
            collection: fold_expr(collection, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::Zone { name, capacity, source_file, body } => Stmt::Zone {
            name,
            capacity,
            source_file,
            body: fold_block(body, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::Concurrent { tasks } => Stmt::Concurrent {
            tasks: fold_block(tasks, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::Parallel { tasks } => Stmt::Parallel {
            tasks: fold_block(tasks, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::WriteFile { content, path } => Stmt::WriteFile {
            content: fold_expr(content, expr_arena, stmt_arena, interner, bools),
            path: fold_expr(path, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::SendMessage { message, destination, compression, cached, unchecked, layout, shared, computed, indexed, deduped } => Stmt::SendMessage {
            message: fold_expr(message, expr_arena, stmt_arena, interner, bools),
            destination: fold_expr(destination, expr_arena, stmt_arena, interner, bools),
            compression,
            cached,
            unchecked,
            layout,
            shared,
            computed,
            indexed,
            deduped,
        },
        Stmt::StreamMessage { values, destination } => Stmt::StreamMessage {
            values: fold_expr(values, expr_arena, stmt_arena, interner, bools),
            destination: fold_expr(destination, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::IncreaseCrdt { object, field, amount } => Stmt::IncreaseCrdt {
            object: fold_expr(object, expr_arena, stmt_arena, interner, bools),
            field,
            amount: fold_expr(amount, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::DecreaseCrdt { object, field, amount } => Stmt::DecreaseCrdt {
            object: fold_expr(object, expr_arena, stmt_arena, interner, bools),
            field,
            amount: fold_expr(amount, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::Sleep { milliseconds } => Stmt::Sleep {
            milliseconds: fold_expr(milliseconds, expr_arena, stmt_arena, interner, bools),
        },
        Stmt::MergeCrdt { source, target } => Stmt::MergeCrdt {
            source: fold_expr(source, expr_arena, stmt_arena, interner, bools),
            target: fold_expr(target, expr_arena, stmt_arena, interner, bools),
        },
        other => other,
    }
}

/// May this expression be DELETED from the residual without erasing a
/// runtime error? Fail-closed: only literals, identifiers and total
/// operators over them qualify (division, modulo, indexing, calls and
/// anything unrecognized can error or have effects).
fn expr_is_total(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(_) | Expr::Identifier(_) => true,
        Expr::BinaryOp { op, left, right } => {
            !matches!(op, BinaryOpKind::Divide | BinaryOpKind::ExactDivide | BinaryOpKind::Modulo)
                && expr_is_total(left)
                && expr_is_total(right)
        }
        Expr::Not { operand } => expr_is_total(operand),
        _ => false,
    }
}

/// Statically Bool-producing. `and`/`or`/`not` yield Bool via truthiness, so
/// an identity rewrite may RETURN an operand unwrapped only when that operand
/// is already Bool (`5 and true` is `true`, not `5`) — and `not (not x)` is
/// `x` only for a Bool `x`. Fail-closed: unknown shapes are not boolish.
fn expr_is_boolish(expr: &Expr, bools: &BoolSyms) -> bool {
    match expr {
        Expr::Literal(Literal::Boolean(_)) => true,
        Expr::Identifier(s) => bools.contains(s),
        Expr::Not { .. } => true,
        Expr::Contains { .. } => true,
        Expr::BinaryOp { op, .. } => matches!(
            op,
            BinaryOpKind::Eq
                | BinaryOpKind::NotEq
                | BinaryOpKind::ApproxEq
                | BinaryOpKind::Lt
                | BinaryOpKind::Gt
                | BinaryOpKind::LtEq
                | BinaryOpKind::GtEq
                | BinaryOpKind::And
                | BinaryOpKind::Or
        ),
        _ => false,
    }
}

pub fn fold_expr<'a>(
    expr: &'a Expr<'a>,
    arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    bools: &BoolSyms,
) -> &'a Expr<'a> {
    match expr {
        Expr::BinaryOp { op, left, right } => {
            let folded_left = fold_expr(left, arena, stmt_arena, interner, bools);
            let folded_right = fold_expr(right, arena, stmt_arena, interner, bools);

            if let Some(result) = try_fold_binary(*op, folded_left, folded_right, interner) {
                arena.alloc(result)
            } else if let Some(simplified) = try_simplify_algebraic(*op, folded_left, folded_right, arena, bools) {
                simplified
            } else if std::ptr::eq(folded_left, *left) && std::ptr::eq(folded_right, *right) {
                expr
            } else {
                arena.alloc(Expr::BinaryOp { op: *op, left: folded_left, right: folded_right })
            }
        }
        Expr::WithCapacity { value, capacity } => {
            let fv = fold_expr(value, arena, stmt_arena, interner, bools);
            let fc = fold_expr(capacity, arena, stmt_arena, interner, bools);
            if std::ptr::eq(fv, *value) && std::ptr::eq(fc, *capacity) {
                expr
            } else {
                arena.alloc(Expr::WithCapacity { value: fv, capacity: fc })
            }
        }

        // Two sub-expressions
        Expr::Index { collection, index } => {
            let fc = fold_expr(collection, arena, stmt_arena, interner, bools);
            let fi = fold_expr(index, arena, stmt_arena, interner, bools);
            if std::ptr::eq(fc, *collection) && std::ptr::eq(fi, *index) {
                expr
            } else {
                arena.alloc(Expr::Index { collection: fc, index: fi })
            }
        }
        Expr::Slice { collection, start, end } => {
            let fc = fold_expr(collection, arena, stmt_arena, interner, bools);
            let fs = fold_expr(start, arena, stmt_arena, interner, bools);
            let fe = fold_expr(end, arena, stmt_arena, interner, bools);
            if std::ptr::eq(fc, *collection) && std::ptr::eq(fs, *start) && std::ptr::eq(fe, *end) {
                expr
            } else {
                arena.alloc(Expr::Slice { collection: fc, start: fs, end: fe })
            }
        }
        Expr::Contains { collection, value } => {
            let fc = fold_expr(collection, arena, stmt_arena, interner, bools);
            let fv = fold_expr(value, arena, stmt_arena, interner, bools);
            if std::ptr::eq(fc, *collection) && std::ptr::eq(fv, *value) {
                expr
            } else {
                arena.alloc(Expr::Contains { collection: fc, value: fv })
            }
        }
        Expr::Union { left, right } => {
            let fl = fold_expr(left, arena, stmt_arena, interner, bools);
            let fr = fold_expr(right, arena, stmt_arena, interner, bools);
            if std::ptr::eq(fl, *left) && std::ptr::eq(fr, *right) {
                expr
            } else {
                arena.alloc(Expr::Union { left: fl, right: fr })
            }
        }
        Expr::Intersection { left, right } => {
            let fl = fold_expr(left, arena, stmt_arena, interner, bools);
            let fr = fold_expr(right, arena, stmt_arena, interner, bools);
            if std::ptr::eq(fl, *left) && std::ptr::eq(fr, *right) {
                expr
            } else {
                arena.alloc(Expr::Intersection { left: fl, right: fr })
            }
        }
        Expr::Range { start, end } => {
            let fs = fold_expr(start, arena, stmt_arena, interner, bools);
            let fe = fold_expr(end, arena, stmt_arena, interner, bools);
            if std::ptr::eq(fs, *start) && std::ptr::eq(fe, *end) {
                expr
            } else {
                arena.alloc(Expr::Range { start: fs, end: fe })
            }
        }
        Expr::ChunkAt { index, zone } => {
            let fi = fold_expr(index, arena, stmt_arena, interner, bools);
            let fz = fold_expr(zone, arena, stmt_arena, interner, bools);
            if std::ptr::eq(fi, *index) && std::ptr::eq(fz, *zone) {
                expr
            } else {
                arena.alloc(Expr::ChunkAt { index: fi, zone: fz })
            }
        }

        // One sub-expression
        Expr::Copy { expr: inner } => {
            let fi = fold_expr(inner, arena, stmt_arena, interner, bools);
            if std::ptr::eq(fi, *inner) { expr } else { arena.alloc(Expr::Copy { expr: fi }) }
        }
        Expr::Give { value } => {
            let fv = fold_expr(value, arena, stmt_arena, interner, bools);
            if std::ptr::eq(fv, *value) { expr } else { arena.alloc(Expr::Give { value: fv }) }
        }
        Expr::Length { collection } => {
            let fc = fold_expr(collection, arena, stmt_arena, interner, bools);
            if std::ptr::eq(fc, *collection) { expr } else { arena.alloc(Expr::Length { collection: fc }) }
        }
        Expr::ManifestOf { zone } => {
            let fz = fold_expr(zone, arena, stmt_arena, interner, bools);
            if std::ptr::eq(fz, *zone) { expr } else { arena.alloc(Expr::ManifestOf { zone: fz }) }
        }
        Expr::FieldAccess { object, field } => {
            let fo = fold_expr(object, arena, stmt_arena, interner, bools);
            if std::ptr::eq(fo, *object) { expr } else { arena.alloc(Expr::FieldAccess { object: fo, field: *field }) }
        }
        Expr::OptionSome { value } => {
            let fv = fold_expr(value, arena, stmt_arena, interner, bools);
            if std::ptr::eq(fv, *value) { expr } else { arena.alloc(Expr::OptionSome { value: fv }) }
        }
        Expr::Not { operand } => {
            let fo = fold_expr(operand, arena, stmt_arena, interner, bools);
            if let Expr::Literal(Literal::Boolean(b)) = fo {
                arena.alloc(Expr::Literal(Literal::Boolean(!b)))
            } else if let Expr::Literal(Literal::Number(n)) = fo {
                // Logical not of a numeric literal: truthiness → Bool.
                arena.alloc(Expr::Literal(Literal::Boolean(*n == 0)))
            } else if let Expr::Not { operand: inner } = fo {
                // !!x → x only for a Bool x — on anything else the double
                // negation is the TRUTHINESS of x as a Bool, not x itself.
                if expr_is_boolish(inner, bools) {
                    inner
                } else {
                    arena.alloc(Expr::Not { operand: fo })
                }
            } else if std::ptr::eq(fo, *operand) {
                expr
            } else {
                arena.alloc(Expr::Not { operand: fo })
            }
        }

        // Vec of sub-expressions
        Expr::Call { function, args } => {
            let folded_args: Vec<&'a Expr<'a>> = args.iter().map(|a| fold_expr(a, arena, stmt_arena, interner, bools)).collect();
            let changed = folded_args.iter().zip(args.iter()).any(|(f, o)| !std::ptr::eq(*f, *o));
            if changed {
                arena.alloc(Expr::Call { function: *function, args: folded_args })
            } else {
                expr
            }
        }
        Expr::CallExpr { callee, args } => {
            let fc = fold_expr(callee, arena, stmt_arena, interner, bools);
            let folded_args: Vec<&'a Expr<'a>> = args.iter().map(|a| fold_expr(a, arena, stmt_arena, interner, bools)).collect();
            let args_changed = folded_args.iter().zip(args.iter()).any(|(f, o)| !std::ptr::eq(*f, *o));
            if std::ptr::eq(fc, *callee) && !args_changed {
                expr
            } else {
                arena.alloc(Expr::CallExpr { callee: fc, args: folded_args })
            }
        }
        Expr::List(elems) => {
            let folded: Vec<&'a Expr<'a>> = elems.iter().map(|e| fold_expr(e, arena, stmt_arena, interner, bools)).collect();
            let changed = folded.iter().zip(elems.iter()).any(|(f, o)| !std::ptr::eq(*f, *o));
            if changed { arena.alloc(Expr::List(folded)) } else { expr }
        }
        Expr::Tuple(elems) => {
            let folded: Vec<&'a Expr<'a>> = elems.iter().map(|e| fold_expr(e, arena, stmt_arena, interner, bools)).collect();
            let changed = folded.iter().zip(elems.iter()).any(|(f, o)| !std::ptr::eq(*f, *o));
            if changed { arena.alloc(Expr::Tuple(folded)) } else { expr }
        }

        // Named field sub-expressions
        Expr::New { type_name, type_args, init_fields } => {
            let folded_fields: Vec<(Symbol, &'a Expr<'a>)> = init_fields
                .iter()
                .map(|(name, val)| (*name, fold_expr(val, arena, stmt_arena, interner, bools)))
                .collect();
            let changed = folded_fields.iter().zip(init_fields.iter())
                .any(|((_, fv), (_, ov))| !std::ptr::eq(*fv, *ov));
            if changed {
                arena.alloc(Expr::New { type_name: *type_name, type_args: type_args.clone(), init_fields: folded_fields })
            } else {
                expr
            }
        }
        Expr::NewVariant { enum_name, variant, fields } => {
            let folded_fields: Vec<(Symbol, &'a Expr<'a>)> = fields
                .iter()
                .map(|(name, val)| (*name, fold_expr(val, arena, stmt_arena, interner, bools)))
                .collect();
            let changed = folded_fields.iter().zip(fields.iter())
                .any(|((_, fv), (_, ov))| !std::ptr::eq(*fv, *ov));
            if changed {
                arena.alloc(Expr::NewVariant { enum_name: *enum_name, variant: *variant, fields: folded_fields })
            } else {
                expr
            }
        }

        // Closure — fold into body
        Expr::Closure { params, body, return_type } => {
            match body {
                ClosureBody::Expression(body_expr) => {
                    let fb = fold_expr(body_expr, arena, stmt_arena, interner, bools);
                    if std::ptr::eq(fb, *body_expr) {
                        expr
                    } else {
                        arena.alloc(Expr::Closure {
                            params: params.clone(),
                            body: ClosureBody::Expression(fb),
                            return_type: *return_type,
                        })
                    }
                }
                ClosureBody::Block(block) => {
                    let fb = fold_block(block, arena, stmt_arena, interner, bools);
                    if std::ptr::eq(fb.as_ptr(), block.as_ptr()) {
                        expr
                    } else {
                        arena.alloc(Expr::Closure {
                            params: params.clone(),
                            body: ClosureBody::Block(fb),
                            return_type: *return_type,
                        })
                    }
                }
            }
        }

        // Interpolated strings — fold sub-expressions in holes
        Expr::InterpolatedString(_) => expr,

        // Leaves — no sub-expressions to fold
        Expr::Literal(_) | Expr::Identifier(_) | Expr::OptionNone | Expr::Escape { .. } => expr,
    }
}

fn try_fold_binary<'a>(
    op: BinaryOpKind,
    left: &Expr<'a>,
    right: &Expr<'a>,
    interner: &mut Interner,
) -> Option<Expr<'a>> {
    match (left, right) {
        (Expr::Literal(Literal::Number(l)), Expr::Literal(Literal::Number(r))) => {
            fold_int_op(op, *l, *r)
        }
        (Expr::Literal(Literal::Float(l)), Expr::Literal(Literal::Float(r))) => {
            fold_float_op(op, *l, *r)
        }
        (Expr::Literal(Literal::Boolean(l)), Expr::Literal(Literal::Boolean(r))) => {
            fold_bool_op(op, *l, *r)
        }
        (Expr::Literal(Literal::Text(l)), Expr::Literal(Literal::Text(r))) => {
            fold_text_op(op, *l, *r, interner)
        }
        _ => None,
    }
}

fn is_int_zero(e: &Expr) -> bool {
    matches!(e, Expr::Literal(Literal::Number(0)))
}

fn is_int_one(e: &Expr) -> bool {
    matches!(e, Expr::Literal(Literal::Number(1)))
}

fn is_float_zero(e: &Expr) -> bool {
    matches!(e, Expr::Literal(Literal::Float(v)) if *v == 0.0)
}

fn is_float_one(e: &Expr) -> bool {
    matches!(e, Expr::Literal(Literal::Float(v)) if *v == 1.0)
}

fn is_power_of_two(n: i64) -> Option<u32> {
    if n > 1 && (n & (n - 1)) == 0 {
        Some(n.trailing_zeros())
    } else {
        None
    }
}

fn try_simplify_algebraic<'a>(
    op: BinaryOpKind,
    left: &'a Expr<'a>,
    right: &'a Expr<'a>,
    arena: &'a Arena<Expr<'a>>,
    bools: &BoolSyms,
) -> Option<&'a Expr<'a>> {
    match op {
        // x + 0 = x, 0 + x = x — INTEGER ONLY. For floats this is unsound: `-0.0 + 0.0 == +0.0` flips
        // the sign bit, so the e-graph forbids it ("Float operands never rewrite", egraph/rules.rs).
        // (Subtraction below keeps `x - 0.0 → x`: that one preserves the sign, `-0.0 - 0.0 == -0.0`.)
        BinaryOpKind::Add => {
            if is_int_zero(right) { return Some(left); }
            if is_int_zero(left) { return Some(right); }
            None
        }
        // x - 0 = x
        BinaryOpKind::Subtract => {
            if is_int_zero(right) || is_float_zero(right) { return Some(left); }
            None
        }
        // x * 1 = x, 1 * x = x (multiplying by one is the IEEE identity, any type); x * 0 = 0, 0 * x = 0
        // INTEGER ONLY. Float `x * 0.0` is NOT folded to 0.0: `inf * 0.0 == NaN` and `NaN * 0.0 == NaN`,
        // so collapsing to 0.0 is unsound — the e-graph refuses float rewrites for exactly this reason.
        //
        // NOTE: `x * 2^k → x << k` strength reduction is NOT done here. Integer `*`
        // is EXACT (it promotes to BigInt on overflow), but `<<` WRAPS — so the
        // rewrite changes the value at the overflow boundary (i64::MAX * 2 promotes
        // to 2^64-2, but i64::MAX << 1 wraps to -2). Like `x / 2^k → shift`, it
        // belongs in the BACKEND, gated on the Oracle proving the product fits i64.
        BinaryOpKind::Multiply => {
            if is_int_one(right) || is_float_one(right) { return Some(left); }
            if is_int_one(left) || is_float_one(left) { return Some(right); }
            if is_int_zero(right) && expr_is_total(left) { return Some(right); }
            if is_int_zero(left) && expr_is_total(right) { return Some(left); }
            None
        }
        // x / 1 = x (int and float)
        BinaryOpKind::Divide => {
            if is_int_one(right) || is_float_one(right) { return Some(left); }
            // NOTE: `x / 2^k → shift` strength reduction belongs in the BACKEND
            // (post-Oracle JIT/codegen lowering), NOT here. At the fold stage it
            // runs before the Oracle's bounds analysis (A1 modulo + A2 element
            // interval), which recognizes the literal `x / 2^k` to derive index
            // intervals (`oracle_hints_element_indexed_scatter`); rewriting the
            // division away blinds it. Do it where the division has already been
            // analyzed.
            None
        }
        // x % 2^k masking is handled by the Architect's mod-pow2-and rule,
        // which carries the REQUIRED Oracle non-negativity proof — kernel
        // modulo takes the sign of the dividend, the mask never does.
        // Boolean algebra: x && true → x, x && false → false, x || true → true,
        // x || false → x. Identities that RETURN an operand unwrapped need that
        // operand boolish — `and`/`or` yield Bool via truthiness, so returning
        // a bare Int would change the result (`5 and true` is `true`, not `5`).
        BinaryOpKind::And => {
            if let Expr::Literal(Literal::Boolean(true)) = right {
                if expr_is_boolish(left, bools) {
                    return Some(left);
                }
            }
            if let Expr::Literal(Literal::Boolean(true)) = left {
                if expr_is_boolish(right, bools) {
                    return Some(right);
                }
            }
            if let Expr::Literal(Literal::Boolean(false)) = right {
                if expr_is_total(left) {
                    return Some(arena.alloc(Expr::Literal(Literal::Boolean(false))));
                }
            }
            // false && x → false is short-circuit-exact: the runtime never
            // evaluates x either.
            if let Expr::Literal(Literal::Boolean(false)) = left {
                return Some(arena.alloc(Expr::Literal(Literal::Boolean(false))));
            }
            None
        }
        BinaryOpKind::Or => {
            if let Expr::Literal(Literal::Boolean(true)) = right {
                if expr_is_total(left) {
                    return Some(arena.alloc(Expr::Literal(Literal::Boolean(true))));
                }
            }
            // true || x → true: short-circuit-exact.
            if let Expr::Literal(Literal::Boolean(true)) = left {
                return Some(arena.alloc(Expr::Literal(Literal::Boolean(true))));
            }
            if let Expr::Literal(Literal::Boolean(false)) = right {
                if expr_is_boolish(left, bools) {
                    return Some(left);
                }
            }
            if let Expr::Literal(Literal::Boolean(false)) = left {
                if expr_is_boolish(right, bools) {
                    return Some(right);
                }
            }
            None
        }
        // Divisibility TEST `x % 2^k == 0` (or `!= 0`) → `(x & (2^k - 1)) == 0`.
        // Sign-agnostic: the low k bits are zero iff 2^k divides x, for ANY sign
        // of x. So — unlike the general `x % 2^k → x & mask` VALUE reduction,
        // which the Architect must gate on an Oracle x≥0 proof (kernel modulo
        // takes the dividend's sign, the mask never does) — the comparison
        // against zero needs NO proof. This is the gap a sign-unknown `k % 2 ==
        // 0` (collatz) falls into: it would otherwise emit a hardware `idiv`
        // every iteration. A `Number` divisor implies an integer dividend (the
        // same assumption `x * 2^k → x << k` above relies on).
        BinaryOpKind::Eq | BinaryOpKind::NotEq => {
            let mask_of = |m: &'a Expr<'a>| -> Option<&'a Expr<'a>> {
                if let Expr::BinaryOp { op: BinaryOpKind::Modulo, left: dividend, right: divisor } = m {
                    if let Expr::Literal(Literal::Number(d)) = divisor {
                        if is_power_of_two(*d).is_some() {
                            return Some(arena.alloc(Expr::BinaryOp {
                                op: BinaryOpKind::BitAnd,
                                left: dividend,
                                right: arena.alloc(Expr::Literal(Literal::Number(d - 1))),
                            }));
                        }
                    }
                }
                None
            };
            if is_int_zero(right) {
                if let Some(masked) = mask_of(left) {
                    return Some(arena.alloc(Expr::BinaryOp { op, left: masked, right }));
                }
            }
            if is_int_zero(left) {
                if let Some(masked) = mask_of(right) {
                    return Some(arena.alloc(Expr::BinaryOp { op, left, right: masked }));
                }
            }
            None
        }
        // Note: self-comparison identities (x==x→true, x-x→0, etc.) are NOT safe
        // on raw identifiers because we lack type info — NaN breaks IEEE 754 equality.
        // These cases are already handled when both sides are known literals via try_fold_binary.
        _ => None,
    }
}

fn fold_int_op(op: BinaryOpKind, l: i64, r: i64) -> Option<Expr<'static>> {
    match op {
        // Fold only when it fits i64; overflow → None so the exact runtime promotes.
        BinaryOpKind::Add => l.checked_add(r).map(|n| Expr::Literal(Literal::Number(n))),
        BinaryOpKind::Subtract => l.checked_sub(r).map(|n| Expr::Literal(Literal::Number(n))),
        BinaryOpKind::Multiply => l.checked_mul(r).map(|n| Expr::Literal(Literal::Number(n))),
        BinaryOpKind::Divide if r != 0 => l.checked_div(r).map(|n| Expr::Literal(Literal::Number(n))),
        BinaryOpKind::Modulo if r != 0 => l.checked_rem(r).map(|n| Expr::Literal(Literal::Number(n))),
        BinaryOpKind::Eq => Some(Expr::Literal(Literal::Boolean(l == r))),
        BinaryOpKind::NotEq => Some(Expr::Literal(Literal::Boolean(l != r))),
        BinaryOpKind::Lt => Some(Expr::Literal(Literal::Boolean(l < r))),
        BinaryOpKind::Gt => Some(Expr::Literal(Literal::Boolean(l > r))),
        BinaryOpKind::LtEq => Some(Expr::Literal(Literal::Boolean(l <= r))),
        BinaryOpKind::GtEq => Some(Expr::Literal(Literal::Boolean(l >= r))),
        BinaryOpKind::BitXor => Some(Expr::Literal(Literal::Number(l ^ r))),
        BinaryOpKind::Shl if r >= 0 && r < 64 => Some(Expr::Literal(Literal::Number(l.wrapping_shl(r as u32)))),
        BinaryOpKind::Shr if r >= 0 && r < 64 => Some(Expr::Literal(Literal::Number(l.wrapping_shr(r as u32)))),
        // `and`/`or` are logical: truthiness in, Bool out (`&`/`|` are bitwise).
        BinaryOpKind::And => Some(Expr::Literal(Literal::Boolean(l != 0 && r != 0))),
        BinaryOpKind::Or => Some(Expr::Literal(Literal::Boolean(l != 0 || r != 0))),
        BinaryOpKind::BitAnd => Some(Expr::Literal(Literal::Number(l & r))),
        BinaryOpKind::BitOr => Some(Expr::Literal(Literal::Number(l | r))),
        _ => None,
    }
}

fn fold_float_op(op: BinaryOpKind, l: f64, r: f64) -> Option<Expr<'static>> {
    match op {
        BinaryOpKind::Add => Some(Expr::Literal(Literal::Float(l + r))),
        BinaryOpKind::Subtract => Some(Expr::Literal(Literal::Float(l - r))),
        BinaryOpKind::Multiply => Some(Expr::Literal(Literal::Float(l * r))),
        BinaryOpKind::Divide if r != 0.0 => Some(Expr::Literal(Literal::Float(l / r))),
        // IEEE 754 comparison semantics (NaN propagates correctly via Rust's f64 ops)
        BinaryOpKind::Eq => Some(Expr::Literal(Literal::Boolean(l == r))),
        BinaryOpKind::NotEq => Some(Expr::Literal(Literal::Boolean(l != r))),
        BinaryOpKind::Lt => Some(Expr::Literal(Literal::Boolean(l < r))),
        BinaryOpKind::Gt => Some(Expr::Literal(Literal::Boolean(l > r))),
        BinaryOpKind::LtEq => Some(Expr::Literal(Literal::Boolean(l <= r))),
        BinaryOpKind::GtEq => Some(Expr::Literal(Literal::Boolean(l >= r))),
        _ => None,
    }
}

fn fold_bool_op(op: BinaryOpKind, l: bool, r: bool) -> Option<Expr<'static>> {
    match op {
        BinaryOpKind::And => Some(Expr::Literal(Literal::Boolean(l && r))),
        BinaryOpKind::Or => Some(Expr::Literal(Literal::Boolean(l || r))),
        BinaryOpKind::Eq => Some(Expr::Literal(Literal::Boolean(l == r))),
        BinaryOpKind::NotEq => Some(Expr::Literal(Literal::Boolean(l != r))),
        _ => None,
    }
}

fn fold_text_op(op: BinaryOpKind, l: Symbol, r: Symbol, interner: &mut Interner) -> Option<Expr<'static>> {
    match op {
        BinaryOpKind::Concat | BinaryOpKind::Add => {
            let l_str = interner.resolve(l);
            let r_str = interner.resolve(r);
            let combined = format!("{}{}", l_str, r_str);
            let sym = interner.intern(&combined);
            Some(Expr::Literal(Literal::Text(sym)))
        }
        BinaryOpKind::Eq => Some(Expr::Literal(Literal::Boolean(l == r))),
        BinaryOpKind::NotEq => Some(Expr::Literal(Literal::Boolean(l != r))),
        _ => None,
    }
}

#[cfg(test)]
mod float_safety_tests {
    use super::*;

    /// CRITIQUE #8 — IEEE float safety. `x * 0.0 → 0.0` is wrong for `Inf`/`NaN` (both give `NaN`, not
    /// `0.0`), and `x + 0.0 → x` / `0.0 + x → x` is wrong for `-0.0` (`-0.0 + 0.0 == +0.0`, flipping the
    /// sign bit). The project's own e-graph already refuses float rewrites for exactly this reason
    /// (`egraph/rules.rs`: "Float operands never rewrite"); the scalar fold pass must not apply them
    /// either. A non-literal float operand `x` must leave these expressions untouched.
    #[test]
    fn float_zero_algebraic_identities_are_not_applied() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = ea.alloc(Expr::Identifier(it.intern("x")));
        let zero_f = ea.alloc(Expr::Literal(Literal::Float(0.0)));

        assert!(
            try_simplify_algebraic(BinaryOpKind::Multiply, x, zero_f, &ea, &BoolSyms::new()).is_none(),
            "x * 0.0 must NOT fold to 0.0 — inf*0.0 = NaN"
        );
        assert!(
            try_simplify_algebraic(BinaryOpKind::Multiply, zero_f, x, &ea, &BoolSyms::new()).is_none(),
            "0.0 * x must NOT fold to 0.0"
        );
        assert!(
            try_simplify_algebraic(BinaryOpKind::Add, x, zero_f, &ea, &BoolSyms::new()).is_none(),
            "x + 0.0 must NOT fold to x — -0.0 + 0.0 = +0.0 (sign flip)"
        );
        assert!(
            try_simplify_algebraic(BinaryOpKind::Add, zero_f, x, &ea, &BoolSyms::new()).is_none(),
            "0.0 + x must NOT fold to x"
        );
    }

    /// Regression: the INTEGER identities are exact (`+` promotes to BigInt; `*` likewise) and must keep
    /// folding. Removing the float rewrites must not touch these.
    #[test]
    fn integer_zero_algebraic_identities_still_apply() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = ea.alloc(Expr::Identifier(it.intern("x")));
        let zero_i = ea.alloc(Expr::Literal(Literal::Number(0)));

        assert!(try_simplify_algebraic(BinaryOpKind::Add, x, zero_i, &ea, &BoolSyms::new()).is_some(), "x + 0 → x still folds (int)");
        assert!(
            try_simplify_algebraic(BinaryOpKind::Multiply, x, zero_i, &ea, &BoolSyms::new()).is_some(),
            "x * 0 → 0 still folds (int, x total)"
        );
    }
}
