//! Popcount-leaf recognizer for the AOT path.
//!
//! A recursive bitmask counting search whose base case returns a constant has a
//! degenerate second-to-last level: when the recursion counter `k` is one step
//! from its limit, every child immediately hits the base case and returns the
//! constant, so the whole loop reduces to `CONST * popcount(x)` over the set
//! `x` it iterates. N-queens spends most of its time exactly there (`row==n-1`),
//! and collapsing that level to a single `count_ones` is a win gcc/LLVM don't
//! get (they can't see the constant-return depth relationship).
//!
//! This pass is a pure STRUCTURAL rewrite — no proof kernel. It fires on the
//! shape (fail-closed on any deviation):
//! ```text
//! To f(…, k, …, LIMIT) -> Int:
//!     If k == LIMIT { Return CONST }              # constant base case, FIRST stmt
//!     …                                           # pure setup
//!     Let mutable x be <set>                      # the value iterated
//!     Let mutable acc be 0
//!     While x is not 0:
//!         Let bit be x and (0 - x)                # lowest set bit
//!         Set x to x xor bit                      # clear it
//!         Set acc to acc + f(…, k + 1, …)         # sole accumulation, steps k→k+1
//!     Return acc.
//! ```
//! and splices, right after `x`'s defining `Let`:
//! ```text
//!     If k == LIMIT - 1 { Return CONST * count_ones(x) }
//! ```
//! Soundness is structural: at `k == LIMIT-1` the call `f(…, k+1=LIMIT, …)` hits
//! the base case immediately and returns `CONST` regardless of its other
//! arguments; the loop runs exactly `popcount(x)` times; `acc` starts at 0; so
//! the function returns `CONST * popcount(x)`. We never reason about the other
//! state parameters. Commuted operands (`x & (0-x)` vs `(0-x) & x`, etc.) are
//! accepted to survive egraph canonicalization. Runs BEFORE `inline_recursive`
//! so the fast path is carried into every unrolled clone; `LOGOS_POPCOUNT_LEAF=0`
//! disables it.

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};
use crate::optimization::{Opt, OptimizationConfig};

fn as_ident(e: &Expr) -> Option<Symbol> {
    match e {
        Expr::Identifier(s) => Some(*s),
        _ => None,
    }
}

fn as_int(e: &Expr) -> Option<i64> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        _ => None,
    }
}

/// `0 - x`
fn is_neg_of(e: &Expr, x: Symbol) -> bool {
    matches!(e, Expr::BinaryOp { op: BinaryOpKind::Subtract, left, right }
        if as_int(left) == Some(0) && as_ident(right) == Some(x))
}

/// `x & (0 - x)` (either operand order)
fn is_lowest_bit(e: &Expr, x: Symbol) -> bool {
    if let Expr::BinaryOp { op: BinaryOpKind::And, left, right } = e {
        (as_ident(left) == Some(x) && is_neg_of(right, x))
            || (as_ident(right) == Some(x) && is_neg_of(left, x))
    } else {
        false
    }
}

/// `x ^ bit` (either order)
fn is_xor_of(e: &Expr, x: Symbol, bit: Symbol) -> bool {
    if let Expr::BinaryOp { op: BinaryOpKind::BitXor, left, right } = e {
        (as_ident(left) == Some(x) && as_ident(right) == Some(bit))
            || (as_ident(left) == Some(bit) && as_ident(right) == Some(x))
    } else {
        false
    }
}

/// `k + 1` (either order)
fn is_k_plus_one(e: &Expr, k: Symbol) -> bool {
    if let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = e {
        (as_ident(left) == Some(k) && as_int(right) == Some(1))
            || (as_int(left) == Some(1) && as_ident(right) == Some(k))
    } else {
        false
    }
}

/// A self-call to `fname` that steps the counter `k` to `k + 1`.
fn is_self_step_call(e: &Expr, fname: Symbol, k: Symbol) -> bool {
    matches!(e, Expr::Call { function, args }
        if *function == fname && args.iter().any(|a| is_k_plus_one(a, k)))
}

/// `If k == LIMIT { Return CONST }` (else-less) as the first statement →
/// `(k, LIMIT, CONST)`.
fn match_base_case(s: &Stmt) -> Option<(Symbol, Symbol, i64)> {
    if let Stmt::If { cond, then_block, else_block: None } = s {
        if let Expr::BinaryOp { op: BinaryOpKind::Eq, left, right } = cond {
            let k = as_ident(left)?;
            let limit = as_ident(right)?;
            if then_block.len() == 1 {
                if let Stmt::Return { value: Some(v) } = &then_block[0] {
                    return Some((k, limit, as_int(v)?));
                }
            }
        }
    }
    None
}

/// `While x is not 0` → `x`.
fn match_loop_x(cond: &Expr) -> Option<Symbol> {
    if let Expr::BinaryOp { op: BinaryOpKind::NotEq, left, right } = cond {
        if as_int(right) == Some(0) {
            return as_ident(left);
        }
    }
    None
}

/// The loop body is exactly `[Let bit = x&(0-x)][Set x = x^bit][Set acc = acc + f(…k+1…)]`
/// → the accumulator symbol.
fn match_loop_body(body: Block, x: Symbol, fname: Symbol, k: Symbol) -> Option<Symbol> {
    if body.len() != 3 {
        return None;
    }
    let bit = match &body[0] {
        Stmt::Let { var, value, .. } if is_lowest_bit(value, x) => *var,
        _ => return None,
    };
    match &body[1] {
        Stmt::Set { target, value } if *target == x && is_xor_of(value, x, bit) => {}
        _ => return None,
    }
    match &body[2] {
        Stmt::Set { target, value } => {
            let acc = *target;
            if let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = value {
                let acc_left = as_ident(left) == Some(acc) && is_self_step_call(right, fname, k);
                let acc_right = as_ident(right) == Some(acc) && is_self_step_call(left, fname, k);
                if acc_left || acc_right {
                    return Some(acc);
                }
            }
            None
        }
        _ => None,
    }
}

fn block_has_count_ones(block: Block, count_ones: Symbol) -> bool {
    fn in_expr(e: &Expr, c: Symbol) -> bool {
        match e {
            Expr::Call { function, args } => {
                *function == c || args.iter().any(|a| in_expr(a, c))
            }
            Expr::BinaryOp { left, right, .. } => in_expr(left, c) || in_expr(right, c),
            Expr::Not { operand } => in_expr(operand, c),
            Expr::Index { collection, index } => in_expr(collection, c) || in_expr(index, c),
            Expr::Length { collection } => in_expr(collection, c),
            _ => false,
        }
    }
    block.iter().any(|s| match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, count_ones),
        Stmt::Return { value } => value.map_or(false, |e| in_expr(e, count_ones)),
        Stmt::If { cond, then_block, else_block } => {
            in_expr(cond, count_ones)
                || block_has_count_ones(then_block, count_ones)
                || else_block.map_or(false, |b| block_has_count_ones(b, count_ones))
        }
        Stmt::While { cond, body, .. } => {
            in_expr(cond, count_ones) || block_has_count_ones(body, count_ones)
        }
        _ => false,
    })
}

/// Build `If k == LIMIT - 1 { Return CONST * count_ones(x) }`.
fn build_fast_path<'a>(
    k: Symbol,
    limit: Symbol,
    cnst: i64,
    x: Symbol,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    count_ones: Symbol,
) -> Stmt<'a> {
    let cond = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Eq,
        left: ea.alloc(Expr::Identifier(k)),
        right: ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left: ea.alloc(Expr::Identifier(limit)),
            right: ea.alloc(Expr::Literal(Literal::Number(1))),
        }),
    });
    let popcount = ea.alloc(Expr::Call {
        function: count_ones,
        args: vec![ea.alloc(Expr::Identifier(x))],
    });
    // CONST == 1 collapses to just the popcount; fold would do this anyway.
    let ret_val: &Expr = if cnst == 1 {
        popcount
    } else {
        ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            left: ea.alloc(Expr::Literal(Literal::Number(cnst))),
            right: popcount,
        })
    };
    Stmt::If {
        cond,
        then_block: sa.alloc_slice(vec![Stmt::Return { value: Some(ret_val) }]),
        else_block: None,
    }
}

/// If `body` matches the popcount-leaf shape, return the body with the fast path
/// spliced in after `x`'s defining `Let`; otherwise `None`.
fn try_rewrite_body<'a>(
    fname: Symbol,
    body: Block<'a>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    count_ones: Symbol,
) -> Option<Vec<Stmt<'a>>> {
    if body.len() < 4 {
        return None;
    }
    // Idempotence: never fire twice.
    if block_has_count_ones(body, count_ones) {
        return None;
    }
    let (k, limit, cnst) = match_base_case(&body[0])?;

    // Find the bit-iteration loop and its accumulator.
    let mut found = None;
    for (i, s) in body.iter().enumerate() {
        if let Stmt::While { cond, body: lb, .. } = s {
            if let Some(x) = match_loop_x(cond) {
                if let Some(acc) = match_loop_body(lb, x, fname, k) {
                    found = Some((i, x, acc));
                    break;
                }
            }
        }
    }
    let (while_idx, x, acc) = found?;

    // `acc` initialised to 0 before the loop, and the function ends in `Return acc`.
    let acc_init_ok = body[..while_idx].iter().any(|s| {
        matches!(s, Stmt::Let { var, value, .. } if *var == acc && as_int(value) == Some(0))
    });
    if !acc_init_ok {
        return None;
    }
    match body.last()? {
        Stmt::Return { value: Some(v) } if as_ident(v) == Some(acc) => {}
        _ => return None,
    }

    // `x`'s defining `Let` (the value it iterates) — splice the guard right after.
    let x_let_idx = body[..while_idx]
        .iter()
        .rposition(|s| matches!(s, Stmt::Let { var, .. } if *var == x))?;

    let fast = build_fast_path(k, limit, cnst, x, ea, sa, count_ones);
    let mut out: Vec<Stmt<'a>> = Vec::with_capacity(body.len() + 1);
    out.extend(body[..=x_let_idx].iter().cloned());
    out.push(fast);
    out.extend(body[x_let_idx + 1..].iter().cloned());
    Some(out)
}

/// Insert a popcount fast path into every eligible bitmask counting search.
pub fn popcount_leaf_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
    cfg: &OptimizationConfig,
) -> Vec<Stmt<'a>> {
    if !cfg.is_on(Opt::Popcount) {
        return stmts;
    }
    let count_ones = interner.intern("count_ones");
    stmts
        .into_iter()
        .map(|s| match &s {
            Stmt::FunctionDef {
                name,
                generics,
                params,
                body,
                return_type,
                is_native: false,
                native_path,
                is_exported: false,
                export_target,
                opt_flags,
            } if generics.is_empty()
                && opt_flags.is_on(crate::optimization::Opt::Popcount) =>
            {
                match try_rewrite_body(*name, body, expr_arena, stmt_arena, count_ones) {
                    Some(new_body) => Stmt::FunctionDef {
                        name: *name,
                        generics: generics.clone(),
                        params: params.clone(),
                        body: stmt_arena.alloc_slice(new_body),
                        return_type: *return_type,
                        is_native: false,
                        native_path: *native_path,
                        is_exported: false,
                        export_target: *export_target,
                        opt_flags: opt_flags.clone(),
                    },
                    None => s,
                }
            }
            _ => s,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::stmt::TypeExpr;

    fn count_count_ones(block: Block, c: Symbol) -> usize {
        fn in_expr(e: &Expr, c: Symbol) -> usize {
            match e {
                Expr::Call { function, args } => {
                    (if *function == c { 1 } else { 0 })
                        + args.iter().map(|a| in_expr(a, c)).sum::<usize>()
                }
                Expr::BinaryOp { left, right, .. } => in_expr(left, c) + in_expr(right, c),
                _ => 0,
            }
        }
        block
            .iter()
            .map(|s| match s {
                Stmt::Return { value } => value.map_or(0, |e| in_expr(e, c)),
                Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, c),
                Stmt::If { then_block, else_block, .. } => {
                    count_count_ones(then_block, c)
                        + else_block.map_or(0, |b| count_count_ones(b, c))
                }
                Stmt::While { body, .. } => count_count_ones(body, c),
                _ => 0,
            })
            .sum()
    }

    fn body_of<'a>(stmts: &'a [Stmt<'a>], name: Symbol) -> Block<'a> {
        for s in stmts {
            if let Stmt::FunctionDef { name: n, body, .. } = s {
                if *n == name {
                    return body;
                }
            }
        }
        panic!("function not found");
    }

    /// Build an n-queens-shaped counting search (one state param for brevity):
    /// ```text
    /// To solve(row, cols, n) -> Int:
    ///     If row == n: Return 1.
    ///     Let all be (1 << n) - 1.
    ///     Let mutable available be all and not cols.
    ///     Let mutable count be 0.
    ///     While available is not 0:
    ///         Let bit be available and (0 - available).
    ///         Set available to available xor bit.
    ///         Set count to count + solve(row + 1, cols or bit, n).
    ///     Return count.
    /// ```
    fn build_solve<'a>(
        ea: &'a Arena<Expr<'a>>,
        sa: &'a Arena<Stmt<'a>>,
        ta: &'a Arena<TypeExpr<'a>>,
        it: &mut Interner,
        base_const: i64,
    ) -> (Stmt<'a>, Symbol, Symbol) {
        let solve = it.intern("solve");
        let row = it.intern("row");
        let cols = it.intern("cols");
        let n = it.intern("n");
        let all = it.intern("all");
        let available = it.intern("available");
        let count = it.intern("count");
        let bit = it.intern("bit");
        let int_ty: &TypeExpr = ta.alloc(TypeExpr::Primitive(it.intern("Int")));

        let base = Stmt::If {
            cond: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Eq,
                left: ea.alloc(Expr::Identifier(row)),
                right: ea.alloc(Expr::Identifier(n)),
            }),
            then_block: sa.alloc_slice(vec![Stmt::Return {
                value: Some(ea.alloc(Expr::Literal(Literal::Number(base_const)))),
            }]),
            else_block: None,
        };
        let let_all = Stmt::Let {
            var: all,
            ty: None,
            value: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Subtract,
                left: ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::Shl,
                    left: ea.alloc(Expr::Literal(Literal::Number(1))),
                    right: ea.alloc(Expr::Identifier(n)),
                }),
                right: ea.alloc(Expr::Literal(Literal::Number(1))),
            }),
            mutable: false,
        };
        let let_available = Stmt::Let {
            var: available,
            ty: None,
            value: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::And,
                left: ea.alloc(Expr::Identifier(all)),
                right: ea.alloc(Expr::Not { operand: ea.alloc(Expr::Identifier(cols)) }),
            }),
            mutable: true,
        };
        let let_count = Stmt::Let {
            var: count,
            ty: None,
            value: ea.alloc(Expr::Literal(Literal::Number(0))),
            mutable: true,
        };
        let loop_body = sa.alloc_slice(vec![
            Stmt::Let {
                var: bit,
                ty: None,
                value: ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::And,
                    left: ea.alloc(Expr::Identifier(available)),
                    right: ea.alloc(Expr::BinaryOp {
                        op: BinaryOpKind::Subtract,
                        left: ea.alloc(Expr::Literal(Literal::Number(0))),
                        right: ea.alloc(Expr::Identifier(available)),
                    }),
                }),
                mutable: false,
            },
            Stmt::Set {
                target: available,
                value: ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::BitXor,
                    left: ea.alloc(Expr::Identifier(available)),
                    right: ea.alloc(Expr::Identifier(bit)),
                }),
            },
            Stmt::Set {
                target: count,
                value: ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::Add,
                    left: ea.alloc(Expr::Identifier(count)),
                    right: ea.alloc(Expr::Call {
                        function: solve,
                        args: vec![
                            ea.alloc(Expr::BinaryOp {
                                op: BinaryOpKind::Add,
                                left: ea.alloc(Expr::Identifier(row)),
                                right: ea.alloc(Expr::Literal(Literal::Number(1))),
                            }),
                            ea.alloc(Expr::BinaryOp {
                                op: BinaryOpKind::Or,
                                left: ea.alloc(Expr::Identifier(cols)),
                                right: ea.alloc(Expr::Identifier(bit)),
                            }),
                            ea.alloc(Expr::Identifier(n)),
                        ],
                    }),
                }),
            },
        ]);
        let while_stmt = Stmt::While {
            cond: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::NotEq,
                left: ea.alloc(Expr::Identifier(available)),
                right: ea.alloc(Expr::Literal(Literal::Number(0))),
            }),
            body: loop_body,
            decreasing: None,
        };
        let body = sa.alloc_slice(vec![
            base,
            let_all,
            let_available,
            let_count,
            while_stmt,
            Stmt::Return { value: Some(ea.alloc(Expr::Identifier(count))) },
        ]);
        let func = Stmt::FunctionDef {
            name: solve,
            generics: vec![],
            params: vec![(row, int_ty), (cols, int_ty), (n, int_ty)],
            body,
            return_type: Some(int_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: Default::default(),
        };
        (func, solve, available)
    }

    #[test]
    fn inserts_popcount_fast_path() {
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let (func, solve, available) = build_solve(&ea, &sa, &ta, &mut it, 1);

        let out = popcount_leaf_stmts(vec![func], &ea, &sa, &mut it, &OptimizationConfig::all_on());
        let count_ones = it.intern("count_ones");
        let body = body_of(&out, solve);

        assert_eq!(count_count_ones(body, count_ones), 1, "exactly one popcount fast path");
        // The fast path is an `If row == n-1` returning count_ones(available),
        // spliced right after `available`'s Let (index 2 → fast path at index 3).
        match &body[3] {
            Stmt::If { cond, then_block, else_block: None } => {
                assert!(
                    matches!(cond, Expr::BinaryOp { op: BinaryOpKind::Eq, right, .. }
                        if matches!(right, Expr::BinaryOp { op: BinaryOpKind::Subtract, .. })),
                    "guard is `row == n - 1`"
                );
                match &then_block[0] {
                    Stmt::Return { value: Some(Expr::Call { function, args }) } => {
                        assert_eq!(*function, count_ones);
                        assert_eq!(args.len(), 1);
                        assert_eq!(super::as_ident(args[0]), Some(available));
                    }
                    other => panic!("expected `Return count_ones(available)`, got {other:?}"),
                }
            }
            other => panic!("expected the popcount guard at index 3, got {other:?}"),
        }
    }

    #[test]
    fn const_other_than_one_multiplies() {
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let (func, solve, _) = build_solve(&ea, &sa, &ta, &mut it, 3);

        let out = popcount_leaf_stmts(vec![func], &ea, &sa, &mut it, &OptimizationConfig::all_on());
        let body = body_of(&out, solve);
        match &body[3] {
            Stmt::If { then_block, .. } => match &then_block[0] {
                Stmt::Return { value: Some(Expr::BinaryOp { op: BinaryOpKind::Multiply, left, right }) } => {
                    assert_eq!(as_int(left), Some(3), "CONST factor");
                    assert!(matches!(right, Expr::Call { .. }), "times count_ones(x)");
                }
                other => panic!("expected `Return 3 * count_ones(x)`, got {other:?}"),
            },
            other => panic!("expected the popcount guard, got {other:?}"),
        }
    }

    #[test]
    fn idempotent() {
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let (func, solve, _) = build_solve(&ea, &sa, &ta, &mut it, 1);
        let count_ones = it.intern("count_ones");

        let once = popcount_leaf_stmts(vec![func], &ea, &sa, &mut it, &OptimizationConfig::all_on());
        let twice = popcount_leaf_stmts(once, &ea, &sa, &mut it, &OptimizationConfig::all_on());
        assert_eq!(
            count_count_ones(body_of(&twice, solve), count_ones),
            1,
            "running twice must not insert a second fast path"
        );
    }

    #[test]
    fn disabling_popcount_is_a_noop() {
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let (func, solve, _) = build_solve(&ea, &sa, &ta, &mut it, 1);
        let count_ones = it.intern("count_ones");

        let cfg = OptimizationConfig::all_on().disable(Opt::Popcount);
        let out = popcount_leaf_stmts(vec![func], &ea, &sa, &mut it, &cfg);
        assert_eq!(
            count_count_ones(body_of(&out, solve), count_ones),
            0,
            "disabling Popcount = no-op"
        );
    }

    /// A function whose loop body is NOT the lowest-bit-clearing idiom (no
    /// `x ^ bit`) must be left untouched.
    #[test]
    fn non_bit_iteration_is_ignored() {
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let solve = it.intern("solve");
        let row = it.intern("row");
        let n = it.intern("n");
        let available = it.intern("available");
        let count = it.intern("count");
        let int_ty: &TypeExpr = ta.alloc(TypeExpr::Primitive(it.intern("Int")));
        // Loop just decrements `available` and recurses — not a bit-iteration.
        let body = sa.alloc_slice(vec![
            Stmt::If {
                cond: ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::Eq,
                    left: ea.alloc(Expr::Identifier(row)),
                    right: ea.alloc(Expr::Identifier(n)),
                }),
                then_block: sa.alloc_slice(vec![Stmt::Return {
                    value: Some(ea.alloc(Expr::Literal(Literal::Number(1)))),
                }]),
                else_block: None,
            },
            Stmt::Let { var: available, ty: None, value: ea.alloc(Expr::Identifier(n)), mutable: true },
            Stmt::Let { var: count, ty: None, value: ea.alloc(Expr::Literal(Literal::Number(0))), mutable: true },
            Stmt::While {
                cond: ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::NotEq,
                    left: ea.alloc(Expr::Identifier(available)),
                    right: ea.alloc(Expr::Literal(Literal::Number(0))),
                }),
                body: sa.alloc_slice(vec![
                    Stmt::Set {
                        target: available,
                        value: ea.alloc(Expr::BinaryOp {
                            op: BinaryOpKind::Subtract,
                            left: ea.alloc(Expr::Identifier(available)),
                            right: ea.alloc(Expr::Literal(Literal::Number(1))),
                        }),
                    },
                    Stmt::Set {
                        target: count,
                        value: ea.alloc(Expr::BinaryOp {
                            op: BinaryOpKind::Add,
                            left: ea.alloc(Expr::Identifier(count)),
                            right: ea.alloc(Expr::Call {
                                function: solve,
                                args: vec![
                                    ea.alloc(Expr::BinaryOp {
                                        op: BinaryOpKind::Add,
                                        left: ea.alloc(Expr::Identifier(row)),
                                        right: ea.alloc(Expr::Literal(Literal::Number(1))),
                                    }),
                                    ea.alloc(Expr::Identifier(n)),
                                ],
                            }),
                        }),
                    },
                ]),
                decreasing: None,
            },
            Stmt::Return { value: Some(ea.alloc(Expr::Identifier(count))) },
        ]);
        let func = Stmt::FunctionDef {
            name: solve,
            generics: vec![],
            params: vec![(row, int_ty), (n, int_ty)],
            body,
            return_type: Some(int_ty),
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: Default::default(),
        };
        let count_ones = it.intern("count_ones");
        let out = popcount_leaf_stmts(vec![func], &ea, &sa, &mut it, &OptimizationConfig::all_on());
        assert_eq!(
            count_count_ones(body_of(&out, solve), count_ones),
            0,
            "non-bit-iteration loop must not be rewritten"
        );
    }
}
