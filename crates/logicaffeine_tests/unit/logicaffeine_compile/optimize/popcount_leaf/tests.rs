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
            Stmt::If {
                then_block,
                else_block,
                ..
            } => count_count_ones(then_block, c) + else_block.map_or(0, |b| count_count_ones(b, c)),
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
            op: BinaryOpKind::BitAnd,
            left: ea.alloc(Expr::Identifier(all)),
            // `~cols` — the parser's own lowering of the complement.
            right: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::BitXor,
                left: ea.alloc(Expr::Identifier(cols)),
                right: ea.alloc(Expr::Literal(Literal::Number(-1))),
            }),
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
                op: BinaryOpKind::BitAnd,
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
                            op: BinaryOpKind::BitOr,
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
        Stmt::Return {
            value: Some(ea.alloc(Expr::Identifier(count))),
        },
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

    assert_eq!(
        count_count_ones(body, count_ones),
        1,
        "exactly one popcount fast path"
    );
    // The fast path is an `If row == n-1` returning count_ones(available),
    // spliced right after `available`'s Let (index 2 → fast path at index 3).
    match &body[3] {
        Stmt::If {
            cond,
            then_block,
            else_block: None,
        } => {
            assert!(
                matches!(cond, Expr::BinaryOp { op: BinaryOpKind::Eq, right, .. }
                    if matches!(right, Expr::BinaryOp { op: BinaryOpKind::Subtract, .. })),
                "guard is `row == n - 1`"
            );
            match &then_block[0] {
                Stmt::Return {
                    value: Some(Expr::Call { function, args }),
                } => {
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
            Stmt::Return {
                value:
                    Some(Expr::BinaryOp {
                        op: BinaryOpKind::Multiply,
                        left,
                        right,
                    }),
            } => {
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
        Stmt::Let {
            var: available,
            ty: None,
            value: ea.alloc(Expr::Identifier(n)),
            mutable: true,
        },
        Stmt::Let {
            var: count,
            ty: None,
            value: ea.alloc(Expr::Literal(Literal::Number(0))),
            mutable: true,
        },
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
        Stmt::Return {
            value: Some(ea.alloc(Expr::Identifier(count))),
        },
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
