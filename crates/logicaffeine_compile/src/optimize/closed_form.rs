//! Closed-Form Loop Recognition.
//!
//! Detects simple accumulator loops and replaces them with closed-form formulas:
//! - `sum += i` over range [start, limit] → Gauss formula
//! - `count += 1` over range [start, limit] → `limit - start + 1`

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};

#[derive(Debug)]
enum AccumPattern {
    SumOfCounter,
    Count,
    MulByTwo,
}

struct Candidate {
    accum: Symbol,
    counter: Symbol,
    pattern: AccumPattern,
}

fn try_extract_candidate(body: &[Stmt], while_cond: &Expr) -> Option<Candidate> {
    let sets: Vec<_> = body.iter().filter(|s| matches!(s, Stmt::Set { .. })).collect();
    if sets.len() != 2 {
        return None;
    }

    for s in body {
        match s {
            Stmt::Set { .. } | Stmt::Let { .. } => {}
            _ => return None,
        }
    }

    // Identify the loop counter from the While condition
    let cond_counter = match while_cond {
        Expr::BinaryOp { left, .. } => {
            if let Expr::Identifier(sym) = &**left { Some(*sym) } else { None }
        }
        _ => None,
    }?;

    // Find the counter increment: Set cond_counter to cond_counter + 1
    let mut counter_found = false;
    let mut accum_stmt_idx = None;

    for (idx, s) in body.iter().enumerate() {
        if let Stmt::Set { target, value } = s {
            if *target == cond_counter {
                if let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = &**value {
                    if let Expr::Identifier(lhs) = &**left {
                        if *lhs == cond_counter {
                            if let Expr::Literal(Literal::Number(1)) = &**right {
                                counter_found = true;
                                continue;
                            }
                        }
                    }
                }
                return None; // counter is modified but not as +1
            }
            accum_stmt_idx = Some(idx);
        }
    }

    if !counter_found { return None; }
    let counter = cond_counter;
    let accum_idx = accum_stmt_idx?;

    if let Stmt::Set { target: accum, value } = &body[accum_idx] {
        // Additive patterns: accum = accum + counter (SumOfCounter), accum = accum + 1 (Count)
        if let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = &**value {
            if let Expr::Identifier(lhs) = &**left {
                if *lhs == *accum {
                    if let Expr::Identifier(rhs) = &**right {
                        if *rhs == counter {
                            return Some(Candidate { accum: *accum, counter, pattern: AccumPattern::SumOfCounter });
                        }
                    }
                    if let Expr::Literal(Literal::Number(1)) = &**right {
                        return Some(Candidate { accum: *accum, counter, pattern: AccumPattern::Count });
                    }
                }
            }
        }
        // Multiplicative pattern: accum = accum * 2 → power-of-2 (accum << count)
        // Also handles the strength-reduced form: accum = accum << 1
        if let Expr::BinaryOp { op, left, right } = &**value {
            let is_mul_by_2 = match op {
                BinaryOpKind::Multiply => {
                    match (left, right) {
                        (Expr::Identifier(lhs), Expr::Literal(Literal::Number(2))) if *lhs == *accum => true,
                        (Expr::Literal(Literal::Number(2)), Expr::Identifier(rhs)) if *rhs == *accum => true,
                        _ => false,
                    }
                }
                BinaryOpKind::Shl => {
                    // accum << 1 (strength-reduced form from fold pass)
                    match (left, right) {
                        (Expr::Identifier(lhs), Expr::Literal(Literal::Number(1))) if *lhs == *accum => true,
                        _ => false,
                    }
                }
                _ => false,
            };
            if is_mul_by_2 {
                return Some(Candidate { accum: *accum, counter, pattern: AccumPattern::MulByTwo });
            }
        }
    }

    None
}

fn extract_while_limit<'a>(cond: &'a Expr<'a>, counter: Symbol) -> Option<(&'a Expr<'a>, bool)> {
    match cond {
        Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
            if let Expr::Identifier(sym) = &**left {
                if *sym == counter { return Some((right, false)); }
            }
            None
        }
        Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
            if let Expr::Identifier(sym) = &**left {
                if *sym == counter { return Some((right, true)); }
            }
            None
        }
        _ => None,
    }
}

fn find_init_value(stmts: &[Stmt], sym: Symbol) -> Option<i64> {
    for s in stmts.iter().rev() {
        match s {
            Stmt::Let { var, value, .. } if *var == sym => {
                if let Expr::Literal(Literal::Number(n)) = &**value {
                    return Some(*n);
                }
                return None;
            }
            Stmt::Set { target, value } if *target == sym => {
                if let Expr::Literal(Literal::Number(n)) = &**value {
                    return Some(*n);
                }
                return None;
            }
            // Control flow may modify the variable — bail conservatively
            Stmt::If { .. } | Stmt::While { .. } | Stmt::Repeat { .. }
            | Stmt::Call { .. } | Stmt::Escape { .. } | Stmt::Zone { .. } => {
                return None;
            }
            _ => {}
        }
    }
    None
}

fn mk_int<'a>(n: i64, arena: &'a Arena<Expr<'a>>) -> &'a Expr<'a> {
    arena.alloc(Expr::Literal(Literal::Number(n)))
}

fn mk_binop<'a>(
    op: BinaryOpKind,
    left: &'a Expr<'a>,
    right: &'a Expr<'a>,
    arena: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    arena.alloc(Expr::BinaryOp { op, left, right })
}

fn build_formula<'a>(
    pattern: &AccumPattern,
    init: i64,
    start: i64,
    limit: &'a Expr<'a>,
    inclusive: bool,
    ea: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    match pattern {
        AccumPattern::Count => {
            // count = limit - start + 1 (inclusive) or limit - start (exclusive)
            let count = if inclusive {
                mk_binop(BinaryOpKind::Add,
                    mk_binop(BinaryOpKind::Subtract, limit, mk_int(start, ea), ea),
                    mk_int(1, ea), ea)
            } else {
                mk_binop(BinaryOpKind::Subtract, limit, mk_int(start, ea), ea)
            };
            if init == 0 { count } else {
                mk_binop(BinaryOpKind::Add, mk_int(init, ea), count, ea)
            }
        }
        AccumPattern::MulByTwo => {
            // init * 2^count = init << count
            let count = if inclusive {
                mk_binop(BinaryOpKind::Add,
                    mk_binop(BinaryOpKind::Subtract, limit, mk_int(start, ea), ea),
                    mk_int(1, ea), ea)
            } else if start == 0 {
                limit
            } else {
                mk_binop(BinaryOpKind::Subtract, limit, mk_int(start, ea), ea)
            };
            mk_binop(BinaryOpKind::Shl, mk_int(init, ea), count, ea)
        }
        AccumPattern::SumOfCounter => {
            // Gauss: effective_limit * (effective_limit + 1) / 2
            let eff_limit = if inclusive { limit } else {
                mk_binop(BinaryOpKind::Subtract, limit, mk_int(1, ea), ea)
            };
            let gauss_top = mk_binop(BinaryOpKind::Divide,
                mk_binop(BinaryOpKind::Multiply, eff_limit,
                    mk_binop(BinaryOpKind::Add, eff_limit, mk_int(1, ea), ea), ea),
                mk_int(2, ea), ea);

            let sum = if start <= 1 {
                gauss_top
            } else {
                let start_part = mk_binop(BinaryOpKind::Divide,
                    mk_binop(BinaryOpKind::Multiply,
                        mk_int(start - 1, ea), mk_int(start, ea), ea),
                    mk_int(2, ea), ea);
                mk_binop(BinaryOpKind::Subtract, gauss_top, start_part, ea)
            };

            if init == 0 { sum } else {
                mk_binop(BinaryOpKind::Add, mk_int(init, ea), sum, ea)
            }
        }
    }
}

pub fn closed_form_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let mut result = Vec::with_capacity(stmts.len());

    for stmt in stmts {
        match stmt {
            Stmt::While { cond, body, decreasing } => {
                let replaced = try_replace_with_closed_form(
                    cond, body, &result, expr_arena, stmt_arena,
                );
                if let Some(replacement_stmts) = replaced {
                    result.extend(replacement_stmts);
                    continue;
                }

                let new_body = closed_form_stmts(body.to_vec(), expr_arena, stmt_arena, interner);
                result.push(Stmt::While {
                    cond,
                    body: stmt_arena.alloc_slice(new_body),
                    decreasing,
                });
            }
            Stmt::FunctionDef { name, generics, params, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => {
                let new_body = closed_form_stmts(body.to_vec(), expr_arena, stmt_arena, interner);
                result.push(Stmt::FunctionDef {
                    name, generics, params,
                    body: stmt_arena.alloc_slice(new_body),
                    return_type, is_native, native_path, is_exported, export_target, opt_flags,
                });
            }
            Stmt::If { cond, then_block, else_block } => {
                let new_then = closed_form_stmts(then_block.to_vec(), expr_arena, stmt_arena, interner);
                let new_else = else_block.map(|eb| {
                    let processed = closed_form_stmts(eb.to_vec(), expr_arena, stmt_arena, interner);
                    let b: Block = stmt_arena.alloc_slice(processed);
                    b
                });
                result.push(Stmt::If {
                    cond,
                    then_block: stmt_arena.alloc_slice(new_then),
                    else_block: new_else,
                });
            }
            other => result.push(other),
        }
    }

    result
}

fn try_replace_with_closed_form<'a>(
    cond: &'a Expr<'a>,
    body: Block<'a>,
    preceding: &[Stmt<'a>],
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Option<Vec<Stmt<'a>>> {
    let candidate = try_extract_candidate(body, cond)?;
    let (limit_expr, inclusive) = extract_while_limit(cond, candidate.counter)?;
    let init = find_init_value(preceding, candidate.accum)?;
    let start = find_init_value(preceding, candidate.counter)?;

    // For additive patterns (Sum, Count), only fire for start >= 1;
    // the for-range codegen peephole handles start=0 loops efficiently.
    // For multiplicative patterns (MulByTwo), allow start=0 because
    // the loop body cannot be eliminated by the for-range peephole.
    match candidate.pattern {
        AccumPattern::MulByTwo => {
            if start < 0 { return None; }
        }
        _ => {
            if start < 1 { return None; }
        }
    }

    let formula = build_formula(
        &candidate.pattern, init, start, limit_expr, inclusive, expr_arena,
    );

    // Build: If limit >= start (for inclusive) or limit > start (for exclusive):
    //   Set accum to formula.
    //   Set counter to limit + 1 (inclusive) or limit (exclusive).
    let guard_cond = if inclusive {
        expr_arena.alloc(Expr::BinaryOp {
            op: BinaryOpKind::GtEq,
            left: limit_expr,
            right: mk_int(start, expr_arena),
        })
    } else {
        expr_arena.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Gt,
            left: limit_expr,
            right: mk_int(start, expr_arena),
        })
    };

    let counter_final = if inclusive {
        mk_binop(BinaryOpKind::Add, limit_expr, mk_int(1, expr_arena), expr_arena)
    } else {
        limit_expr
    };

    let body_stmts = vec![
        Stmt::Set { target: candidate.accum, value: formula },
        Stmt::Set { target: candidate.counter, value: counter_final },
    ];

    let guarded = Stmt::If {
        cond: guard_cond,
        then_block: stmt_arena.alloc_slice(body_stmts),
        else_block: None,
    };

    Some(vec![guarded])
}
