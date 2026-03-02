use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, ClosureBody, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};

pub fn fold_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    stmts
        .into_iter()
        .map(|stmt| fold_stmt(stmt, expr_arena, stmt_arena, interner))
        .collect()
}

fn fold_block<'a>(
    block: &'a [Stmt<'a>],
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> &'a [Stmt<'a>] {
    let folded: Vec<Stmt<'a>> = block
        .iter()
        .cloned()
        .map(|stmt| fold_stmt(stmt, expr_arena, stmt_arena, interner))
        .collect();
    stmt_arena.alloc_slice(folded)
}

fn fold_stmt<'a>(
    stmt: Stmt<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Stmt<'a> {
    match stmt {
        Stmt::Let { var, ty, value, mutable } => Stmt::Let {
            var,
            ty,
            value: fold_expr(value, expr_arena, stmt_arena, interner),
            mutable,
        },
        Stmt::Set { target, value } => Stmt::Set {
            target,
            value: fold_expr(value, expr_arena, stmt_arena, interner),
        },
        Stmt::If { cond, then_block, else_block } => Stmt::If {
            cond: fold_expr(cond, expr_arena, stmt_arena, interner),
            then_block: fold_block(then_block, expr_arena, stmt_arena, interner),
            else_block: else_block.map(|b| fold_block(b, expr_arena, stmt_arena, interner)),
        },
        Stmt::While { cond, body, decreasing } => Stmt::While {
            cond: fold_expr(cond, expr_arena, stmt_arena, interner),
            body: fold_block(body, expr_arena, stmt_arena, interner),
            decreasing,
        },
        Stmt::Repeat { pattern, iterable, body } => Stmt::Repeat {
            pattern,
            iterable: fold_expr(iterable, expr_arena, stmt_arena, interner),
            body: fold_block(body, expr_arena, stmt_arena, interner),
        },
        Stmt::FunctionDef { name, params, generics, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => Stmt::FunctionDef {
            name,
            params,
            generics,
            body: fold_block(body, expr_arena, stmt_arena, interner),
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
            opt_flags,
        },
        Stmt::Show { object, recipient } => Stmt::Show {
            object: fold_expr(object, expr_arena, stmt_arena, interner),
            recipient,
        },
        Stmt::Return { value } => Stmt::Return {
            value: value.map(|v| fold_expr(v, expr_arena, stmt_arena, interner)),
        },
        Stmt::RuntimeAssert { condition } => Stmt::RuntimeAssert {
            condition: fold_expr(condition, expr_arena, stmt_arena, interner),
        },
        Stmt::Push { value, collection } => Stmt::Push {
            value: fold_expr(value, expr_arena, stmt_arena, interner),
            collection,
        },
        Stmt::SetField { object, field, value } => Stmt::SetField {
            object,
            field,
            value: fold_expr(value, expr_arena, stmt_arena, interner),
        },
        Stmt::SetIndex { collection, index, value } => Stmt::SetIndex {
            collection,
            index: fold_expr(index, expr_arena, stmt_arena, interner),
            value: fold_expr(value, expr_arena, stmt_arena, interner),
        },
        Stmt::Call { function, args } => Stmt::Call {
            function,
            args: args.into_iter().map(|a| fold_expr(a, expr_arena, stmt_arena, interner)).collect(),
        },
        Stmt::Give { object, recipient } => Stmt::Give {
            object: fold_expr(object, expr_arena, stmt_arena, interner),
            recipient: fold_expr(recipient, expr_arena, stmt_arena, interner),
        },
        Stmt::Inspect { target, arms, has_otherwise } => Stmt::Inspect {
            target: fold_expr(target, expr_arena, stmt_arena, interner),
            arms: arms.into_iter().map(|arm| {
                crate::ast::stmt::MatchArm {
                    enum_name: arm.enum_name,
                    variant: arm.variant,
                    bindings: arm.bindings,
                    body: fold_block(arm.body, expr_arena, stmt_arena, interner),
                }
            }).collect(),
            has_otherwise,
        },
        Stmt::Pop { collection, into } => Stmt::Pop {
            collection: fold_expr(collection, expr_arena, stmt_arena, interner),
            into,
        },
        Stmt::Add { value, collection } => Stmt::Add {
            value: fold_expr(value, expr_arena, stmt_arena, interner),
            collection: fold_expr(collection, expr_arena, stmt_arena, interner),
        },
        Stmt::Remove { value, collection } => Stmt::Remove {
            value: fold_expr(value, expr_arena, stmt_arena, interner),
            collection: fold_expr(collection, expr_arena, stmt_arena, interner),
        },
        Stmt::Zone { name, capacity, source_file, body } => Stmt::Zone {
            name,
            capacity,
            source_file,
            body: fold_block(body, expr_arena, stmt_arena, interner),
        },
        Stmt::Concurrent { tasks } => Stmt::Concurrent {
            tasks: fold_block(tasks, expr_arena, stmt_arena, interner),
        },
        Stmt::Parallel { tasks } => Stmt::Parallel {
            tasks: fold_block(tasks, expr_arena, stmt_arena, interner),
        },
        Stmt::WriteFile { content, path } => Stmt::WriteFile {
            content: fold_expr(content, expr_arena, stmt_arena, interner),
            path: fold_expr(path, expr_arena, stmt_arena, interner),
        },
        Stmt::SendMessage { message, destination } => Stmt::SendMessage {
            message: fold_expr(message, expr_arena, stmt_arena, interner),
            destination: fold_expr(destination, expr_arena, stmt_arena, interner),
        },
        Stmt::IncreaseCrdt { object, field, amount } => Stmt::IncreaseCrdt {
            object: fold_expr(object, expr_arena, stmt_arena, interner),
            field,
            amount: fold_expr(amount, expr_arena, stmt_arena, interner),
        },
        Stmt::DecreaseCrdt { object, field, amount } => Stmt::DecreaseCrdt {
            object: fold_expr(object, expr_arena, stmt_arena, interner),
            field,
            amount: fold_expr(amount, expr_arena, stmt_arena, interner),
        },
        Stmt::Sleep { milliseconds } => Stmt::Sleep {
            milliseconds: fold_expr(milliseconds, expr_arena, stmt_arena, interner),
        },
        Stmt::MergeCrdt { source, target } => Stmt::MergeCrdt {
            source: fold_expr(source, expr_arena, stmt_arena, interner),
            target: fold_expr(target, expr_arena, stmt_arena, interner),
        },
        other => other,
    }
}

pub fn fold_expr<'a>(
    expr: &'a Expr<'a>,
    arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> &'a Expr<'a> {
    match expr {
        Expr::BinaryOp { op, left, right } => {
            let folded_left = fold_expr(left, arena, stmt_arena, interner);
            let folded_right = fold_expr(right, arena, stmt_arena, interner);

            if let Some(result) = try_fold_binary(*op, folded_left, folded_right, interner) {
                arena.alloc(result)
            } else if let Some(simplified) = try_simplify_algebraic(*op, folded_left, folded_right, arena) {
                simplified
            } else if std::ptr::eq(folded_left, *left) && std::ptr::eq(folded_right, *right) {
                expr
            } else {
                arena.alloc(Expr::BinaryOp { op: *op, left: folded_left, right: folded_right })
            }
        }
        Expr::WithCapacity { value, capacity } => {
            let fv = fold_expr(value, arena, stmt_arena, interner);
            let fc = fold_expr(capacity, arena, stmt_arena, interner);
            if std::ptr::eq(fv, *value) && std::ptr::eq(fc, *capacity) {
                expr
            } else {
                arena.alloc(Expr::WithCapacity { value: fv, capacity: fc })
            }
        }

        // Two sub-expressions
        Expr::Index { collection, index } => {
            let fc = fold_expr(collection, arena, stmt_arena, interner);
            let fi = fold_expr(index, arena, stmt_arena, interner);
            if std::ptr::eq(fc, *collection) && std::ptr::eq(fi, *index) {
                expr
            } else {
                arena.alloc(Expr::Index { collection: fc, index: fi })
            }
        }
        Expr::Slice { collection, start, end } => {
            let fc = fold_expr(collection, arena, stmt_arena, interner);
            let fs = fold_expr(start, arena, stmt_arena, interner);
            let fe = fold_expr(end, arena, stmt_arena, interner);
            if std::ptr::eq(fc, *collection) && std::ptr::eq(fs, *start) && std::ptr::eq(fe, *end) {
                expr
            } else {
                arena.alloc(Expr::Slice { collection: fc, start: fs, end: fe })
            }
        }
        Expr::Contains { collection, value } => {
            let fc = fold_expr(collection, arena, stmt_arena, interner);
            let fv = fold_expr(value, arena, stmt_arena, interner);
            if std::ptr::eq(fc, *collection) && std::ptr::eq(fv, *value) {
                expr
            } else {
                arena.alloc(Expr::Contains { collection: fc, value: fv })
            }
        }
        Expr::Union { left, right } => {
            let fl = fold_expr(left, arena, stmt_arena, interner);
            let fr = fold_expr(right, arena, stmt_arena, interner);
            if std::ptr::eq(fl, *left) && std::ptr::eq(fr, *right) {
                expr
            } else {
                arena.alloc(Expr::Union { left: fl, right: fr })
            }
        }
        Expr::Intersection { left, right } => {
            let fl = fold_expr(left, arena, stmt_arena, interner);
            let fr = fold_expr(right, arena, stmt_arena, interner);
            if std::ptr::eq(fl, *left) && std::ptr::eq(fr, *right) {
                expr
            } else {
                arena.alloc(Expr::Intersection { left: fl, right: fr })
            }
        }
        Expr::Range { start, end } => {
            let fs = fold_expr(start, arena, stmt_arena, interner);
            let fe = fold_expr(end, arena, stmt_arena, interner);
            if std::ptr::eq(fs, *start) && std::ptr::eq(fe, *end) {
                expr
            } else {
                arena.alloc(Expr::Range { start: fs, end: fe })
            }
        }
        Expr::ChunkAt { index, zone } => {
            let fi = fold_expr(index, arena, stmt_arena, interner);
            let fz = fold_expr(zone, arena, stmt_arena, interner);
            if std::ptr::eq(fi, *index) && std::ptr::eq(fz, *zone) {
                expr
            } else {
                arena.alloc(Expr::ChunkAt { index: fi, zone: fz })
            }
        }

        // One sub-expression
        Expr::Copy { expr: inner } => {
            let fi = fold_expr(inner, arena, stmt_arena, interner);
            if std::ptr::eq(fi, *inner) { expr } else { arena.alloc(Expr::Copy { expr: fi }) }
        }
        Expr::Give { value } => {
            let fv = fold_expr(value, arena, stmt_arena, interner);
            if std::ptr::eq(fv, *value) { expr } else { arena.alloc(Expr::Give { value: fv }) }
        }
        Expr::Length { collection } => {
            let fc = fold_expr(collection, arena, stmt_arena, interner);
            if std::ptr::eq(fc, *collection) { expr } else { arena.alloc(Expr::Length { collection: fc }) }
        }
        Expr::ManifestOf { zone } => {
            let fz = fold_expr(zone, arena, stmt_arena, interner);
            if std::ptr::eq(fz, *zone) { expr } else { arena.alloc(Expr::ManifestOf { zone: fz }) }
        }
        Expr::FieldAccess { object, field } => {
            let fo = fold_expr(object, arena, stmt_arena, interner);
            if std::ptr::eq(fo, *object) { expr } else { arena.alloc(Expr::FieldAccess { object: fo, field: *field }) }
        }
        Expr::OptionSome { value } => {
            let fv = fold_expr(value, arena, stmt_arena, interner);
            if std::ptr::eq(fv, *value) { expr } else { arena.alloc(Expr::OptionSome { value: fv }) }
        }
        Expr::Not { operand } => {
            let fo = fold_expr(operand, arena, stmt_arena, interner);
            if let Expr::Literal(Literal::Boolean(b)) = fo {
                arena.alloc(Expr::Literal(Literal::Boolean(!b)))
            } else if std::ptr::eq(fo, *operand) {
                expr
            } else {
                arena.alloc(Expr::Not { operand: fo })
            }
        }

        // Vec of sub-expressions
        Expr::Call { function, args } => {
            let folded_args: Vec<&'a Expr<'a>> = args.iter().map(|a| fold_expr(a, arena, stmt_arena, interner)).collect();
            let changed = folded_args.iter().zip(args.iter()).any(|(f, o)| !std::ptr::eq(*f, *o));
            if changed {
                arena.alloc(Expr::Call { function: *function, args: folded_args })
            } else {
                expr
            }
        }
        Expr::CallExpr { callee, args } => {
            let fc = fold_expr(callee, arena, stmt_arena, interner);
            let folded_args: Vec<&'a Expr<'a>> = args.iter().map(|a| fold_expr(a, arena, stmt_arena, interner)).collect();
            let args_changed = folded_args.iter().zip(args.iter()).any(|(f, o)| !std::ptr::eq(*f, *o));
            if std::ptr::eq(fc, *callee) && !args_changed {
                expr
            } else {
                arena.alloc(Expr::CallExpr { callee: fc, args: folded_args })
            }
        }
        Expr::List(elems) => {
            let folded: Vec<&'a Expr<'a>> = elems.iter().map(|e| fold_expr(e, arena, stmt_arena, interner)).collect();
            let changed = folded.iter().zip(elems.iter()).any(|(f, o)| !std::ptr::eq(*f, *o));
            if changed { arena.alloc(Expr::List(folded)) } else { expr }
        }
        Expr::Tuple(elems) => {
            let folded: Vec<&'a Expr<'a>> = elems.iter().map(|e| fold_expr(e, arena, stmt_arena, interner)).collect();
            let changed = folded.iter().zip(elems.iter()).any(|(f, o)| !std::ptr::eq(*f, *o));
            if changed { arena.alloc(Expr::Tuple(folded)) } else { expr }
        }

        // Named field sub-expressions
        Expr::New { type_name, type_args, init_fields } => {
            let folded_fields: Vec<(Symbol, &'a Expr<'a>)> = init_fields
                .iter()
                .map(|(name, val)| (*name, fold_expr(val, arena, stmt_arena, interner)))
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
                .map(|(name, val)| (*name, fold_expr(val, arena, stmt_arena, interner)))
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
                    let fb = fold_expr(body_expr, arena, stmt_arena, interner);
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
                    let fb = fold_block(block, arena, stmt_arena, interner);
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

fn try_simplify_algebraic<'a>(
    op: BinaryOpKind,
    left: &'a Expr<'a>,
    right: &'a Expr<'a>,
    arena: &'a Arena<Expr<'a>>,
) -> Option<&'a Expr<'a>> {
    match op {
        // x + 0 = x, 0 + x = x (int and float)
        BinaryOpKind::Add => {
            if is_int_zero(right) || is_float_zero(right) { return Some(left); }
            if is_int_zero(left) || is_float_zero(left) { return Some(right); }
            None
        }
        // x - 0 = x (int and float)
        BinaryOpKind::Subtract => {
            if is_int_zero(right) || is_float_zero(right) { return Some(left); }
            None
        }
        // x * 1 = x, 1 * x = x, x * 0 = 0, 0 * x = 0 (int and float)
        BinaryOpKind::Multiply => {
            if is_int_one(right) || is_float_one(right) { return Some(left); }
            if is_int_one(left) || is_float_one(left) { return Some(right); }
            if is_int_zero(right) { return Some(right); }
            if is_int_zero(left) { return Some(left); }
            if is_float_zero(right) { return Some(arena.alloc(Expr::Literal(Literal::Float(0.0)))); }
            if is_float_zero(left) { return Some(arena.alloc(Expr::Literal(Literal::Float(0.0)))); }
            None
        }
        // x / 1 = x (int and float)
        BinaryOpKind::Divide => {
            if is_int_one(right) || is_float_one(right) { return Some(left); }
            None
        }
        _ => None,
    }
}

fn fold_int_op(op: BinaryOpKind, l: i64, r: i64) -> Option<Expr<'static>> {
    match op {
        BinaryOpKind::Add => Some(Expr::Literal(Literal::Number(l.wrapping_add(r)))),
        BinaryOpKind::Subtract => Some(Expr::Literal(Literal::Number(l.wrapping_sub(r)))),
        BinaryOpKind::Multiply => Some(Expr::Literal(Literal::Number(l.wrapping_mul(r)))),
        BinaryOpKind::Divide if r != 0 => Some(Expr::Literal(Literal::Number(l / r))),
        BinaryOpKind::Modulo if r != 0 => Some(Expr::Literal(Literal::Number(l % r))),
        BinaryOpKind::Eq => Some(Expr::Literal(Literal::Boolean(l == r))),
        BinaryOpKind::NotEq => Some(Expr::Literal(Literal::Boolean(l != r))),
        BinaryOpKind::Lt => Some(Expr::Literal(Literal::Boolean(l < r))),
        BinaryOpKind::Gt => Some(Expr::Literal(Literal::Boolean(l > r))),
        BinaryOpKind::LtEq => Some(Expr::Literal(Literal::Boolean(l <= r))),
        BinaryOpKind::GtEq => Some(Expr::Literal(Literal::Boolean(l >= r))),
        _ => None,
    }
}

fn fold_float_op(op: BinaryOpKind, l: f64, r: f64) -> Option<Expr<'static>> {
    match op {
        BinaryOpKind::Add => Some(Expr::Literal(Literal::Float(l + r))),
        BinaryOpKind::Subtract => Some(Expr::Literal(Literal::Float(l - r))),
        BinaryOpKind::Multiply => Some(Expr::Literal(Literal::Float(l * r))),
        BinaryOpKind::Divide if r != 0.0 => Some(Expr::Literal(Literal::Float(l / r))),
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
        BinaryOpKind::Concat => {
            let l_str = interner.resolve(l);
            let r_str = interner.resolve(r);
            let combined = format!("{}{}", l_str, r_str);
            let sym = interner.intern(&combined);
            Some(Expr::Literal(Literal::Text(sym)))
        }
        _ => None,
    }
}
