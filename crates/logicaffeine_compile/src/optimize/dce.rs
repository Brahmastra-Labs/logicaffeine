use crate::arena::Arena;
use crate::ast::stmt::{Expr, Literal, MatchArm, Stmt};

pub fn eliminate_dead_code<'a>(stmts: Vec<Stmt<'a>>, stmt_arena: &'a Arena<Stmt<'a>>) -> Vec<Stmt<'a>> {
    let mut result = Vec::with_capacity(stmts.len());

    for stmt in stmts {
        match stmt {
            Stmt::If { cond, then_block, else_block } => {
                if let Expr::Literal(Literal::Boolean(val)) = cond {
                    if *val {
                        result.extend(then_block.iter().cloned());
                    } else if let Some(else_stmts) = else_block {
                        result.extend(else_stmts.iter().cloned());
                    }
                } else {
                    let then_dce = dce_block(then_block, stmt_arena);
                    let else_dce = else_block.map(|b| dce_block(b, stmt_arena));
                    result.push(Stmt::If { cond, then_block: then_dce, else_block: else_dce });
                }
            }
            Stmt::While { cond, body, decreasing } => {
                if let Expr::Literal(Literal::Boolean(false)) = cond {
                    // While false: ... is dead code — eliminate entirely
                } else {
                    result.push(Stmt::While {
                        cond,
                        body: dce_block(body, stmt_arena),
                        decreasing,
                    });
                }
            }
            Stmt::Repeat { pattern, iterable, body } => {
                result.push(Stmt::Repeat {
                    pattern,
                    iterable,
                    body: dce_block(body, stmt_arena),
                });
            }
            Stmt::FunctionDef { name, params, generics, body, return_type, is_native, native_path, is_exported, export_target } => {
                result.push(Stmt::FunctionDef {
                    name,
                    params,
                    generics,
                    body: dce_block(body, stmt_arena),
                    return_type,
                    is_native,
                    native_path,
                    is_exported,
                    export_target,
                });
            }
            Stmt::Zone { name, capacity, source_file, body } => {
                result.push(Stmt::Zone {
                    name,
                    capacity,
                    source_file,
                    body: dce_block(body, stmt_arena),
                });
            }
            Stmt::Concurrent { tasks } => {
                result.push(Stmt::Concurrent {
                    tasks: dce_block(tasks, stmt_arena),
                });
            }
            Stmt::Parallel { tasks } => {
                result.push(Stmt::Parallel {
                    tasks: dce_block(tasks, stmt_arena),
                });
            }
            Stmt::Inspect { target, arms, has_otherwise } => {
                let arms_dce: Vec<MatchArm<'a>> = arms.into_iter().map(|arm| {
                    MatchArm {
                        enum_name: arm.enum_name,
                        variant: arm.variant,
                        bindings: arm.bindings,
                        body: dce_block(arm.body, stmt_arena),
                    }
                }).collect();
                result.push(Stmt::Inspect { target, arms: arms_dce, has_otherwise });
            }
            other => result.push(other),
        }
    }

    if let Some(pos) = result.iter().position(|s| matches!(s, Stmt::Return { .. })) {
        result.truncate(pos + 1);
    }

    result
}

fn dce_block<'a>(block: &'a [Stmt<'a>], stmt_arena: &'a Arena<Stmt<'a>>) -> &'a [Stmt<'a>] {
    let stmts: Vec<Stmt<'a>> = block.iter().cloned().collect();
    let dce_result = eliminate_dead_code(stmts, stmt_arena);
    stmt_arena.alloc_slice(dce_result)
}
