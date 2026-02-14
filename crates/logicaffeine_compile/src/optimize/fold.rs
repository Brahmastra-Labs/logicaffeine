use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
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
            value: fold_expr(value, expr_arena, interner),
            mutable,
        },
        Stmt::Set { target, value } => Stmt::Set {
            target,
            value: fold_expr(value, expr_arena, interner),
        },
        Stmt::If { cond, then_block, else_block } => Stmt::If {
            cond: fold_expr(cond, expr_arena, interner),
            then_block: fold_block(then_block, expr_arena, stmt_arena, interner),
            else_block: else_block.map(|b| fold_block(b, expr_arena, stmt_arena, interner)),
        },
        Stmt::While { cond, body, decreasing } => Stmt::While {
            cond: fold_expr(cond, expr_arena, interner),
            body: fold_block(body, expr_arena, stmt_arena, interner),
            decreasing,
        },
        Stmt::Repeat { pattern, iterable, body } => Stmt::Repeat {
            pattern,
            iterable: fold_expr(iterable, expr_arena, interner),
            body: fold_block(body, expr_arena, stmt_arena, interner),
        },
        Stmt::FunctionDef { name, params, body, return_type, is_native, native_path, is_exported, export_target } => Stmt::FunctionDef {
            name,
            params,
            body: fold_block(body, expr_arena, stmt_arena, interner),
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
        },
        Stmt::Show { object, recipient } => Stmt::Show {
            object: fold_expr(object, expr_arena, interner),
            recipient,
        },
        Stmt::Return { value } => Stmt::Return {
            value: value.map(|v| fold_expr(v, expr_arena, interner)),
        },
        Stmt::RuntimeAssert { condition } => Stmt::RuntimeAssert {
            condition: fold_expr(condition, expr_arena, interner),
        },
        Stmt::Push { value, collection } => Stmt::Push {
            value: fold_expr(value, expr_arena, interner),
            collection,
        },
        Stmt::SetField { object, field, value } => Stmt::SetField {
            object,
            field,
            value: fold_expr(value, expr_arena, interner),
        },
        Stmt::SetIndex { collection, index, value } => Stmt::SetIndex {
            collection,
            index: fold_expr(index, expr_arena, interner),
            value: fold_expr(value, expr_arena, interner),
        },
        Stmt::Call { function, args } => Stmt::Call {
            function,
            args: args.into_iter().map(|a| fold_expr(a, expr_arena, interner)).collect(),
        },
        Stmt::Give { object, recipient } => Stmt::Give {
            object: fold_expr(object, expr_arena, interner),
            recipient: fold_expr(recipient, expr_arena, interner),
        },
        Stmt::Inspect { target, arms, has_otherwise } => Stmt::Inspect {
            target: fold_expr(target, expr_arena, interner),
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
            collection: fold_expr(collection, expr_arena, interner),
            into,
        },
        Stmt::Add { value, collection } => Stmt::Add {
            value: fold_expr(value, expr_arena, interner),
            collection: fold_expr(collection, expr_arena, interner),
        },
        Stmt::Remove { value, collection } => Stmt::Remove {
            value: fold_expr(value, expr_arena, interner),
            collection: fold_expr(collection, expr_arena, interner),
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
            content: fold_expr(content, expr_arena, interner),
            path: fold_expr(path, expr_arena, interner),
        },
        Stmt::SendMessage { message, destination } => Stmt::SendMessage {
            message: fold_expr(message, expr_arena, interner),
            destination: fold_expr(destination, expr_arena, interner),
        },
        Stmt::IncreaseCrdt { object, field, amount } => Stmt::IncreaseCrdt {
            object: fold_expr(object, expr_arena, interner),
            field,
            amount: fold_expr(amount, expr_arena, interner),
        },
        Stmt::DecreaseCrdt { object, field, amount } => Stmt::DecreaseCrdt {
            object: fold_expr(object, expr_arena, interner),
            field,
            amount: fold_expr(amount, expr_arena, interner),
        },
        Stmt::Sleep { milliseconds } => Stmt::Sleep {
            milliseconds: fold_expr(milliseconds, expr_arena, interner),
        },
        Stmt::MergeCrdt { source, target } => Stmt::MergeCrdt {
            source: fold_expr(source, expr_arena, interner),
            target: fold_expr(target, expr_arena, interner),
        },
        other => other,
    }
}

pub fn fold_expr<'a>(
    expr: &'a Expr<'a>,
    arena: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
) -> &'a Expr<'a> {
    match expr {
        Expr::BinaryOp { op, left, right } => {
            let folded_left = fold_expr(left, arena, interner);
            let folded_right = fold_expr(right, arena, interner);

            if let Some(result) = try_fold_binary(*op, folded_left, folded_right, interner) {
                arena.alloc(result)
            } else if std::ptr::eq(folded_left, *left) && std::ptr::eq(folded_right, *right) {
                expr
            } else {
                arena.alloc(Expr::BinaryOp { op: *op, left: folded_left, right: folded_right })
            }
        }
        Expr::WithCapacity { value, capacity } => {
            let folded_value = fold_expr(value, arena, interner);
            let folded_cap = fold_expr(capacity, arena, interner);
            if std::ptr::eq(folded_value, *value) && std::ptr::eq(folded_cap, *capacity) {
                expr
            } else {
                arena.alloc(Expr::WithCapacity { value: folded_value, capacity: folded_cap })
            }
        }
        _ => expr,
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
