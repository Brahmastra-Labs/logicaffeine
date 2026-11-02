use super::*;
use crate::ast::stmt::TypeExpr;

/// Count real (un-inlined) calls to `target` anywhere in a block.
fn count_calls(block: Block, target: Symbol) -> usize {
    fn in_expr(e: &Expr, t: Symbol) -> usize {
        match e {
            Expr::Call { function, args } => {
                (if *function == t { 1 } else { 0 })
                    + args.iter().map(|a| in_expr(a, t)).sum::<usize>()
            }
            Expr::BinaryOp { left, right, .. } => in_expr(left, t) + in_expr(right, t),
            Expr::Not { operand } => in_expr(operand, t),
            Expr::Index { collection, index } => in_expr(collection, t) + in_expr(index, t),
            Expr::Length { collection } => in_expr(collection, t),
            _ => 0,
        }
    }
    fn in_block(b: Block, t: Symbol) -> usize {
        b.iter()
            .map(|s| match s {
                Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, t),
                Stmt::Return { value } => value.map_or(0, |e| in_expr(e, t)),
                Stmt::If {
                    cond,
                    then_block,
                    else_block,
                } => {
                    in_expr(cond, t)
                        + in_block(then_block, t)
                        + else_block.map_or(0, |x| in_block(x, t))
                }
                Stmt::While { cond, body, .. } => in_expr(cond, t) + in_block(body, t),
                _ => 0,
            })
            .sum()
    }
    in_block(block, target)
}

fn count_result_temps(block: Block, it: &Interner) -> usize {
    fn in_block(b: Block, it: &Interner) -> usize {
        b.iter()
            .map(|s| match s {
                Stmt::Let { var, .. } => {
                    if it.resolve(*var).ends_with("_result") {
                        1
                    } else {
                        0
                    }
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => in_block(then_block, it) + else_block.map_or(0, |x| in_block(x, it)),
                Stmt::While { body, .. } => in_block(body, it),
                _ => 0,
            })
            .sum()
    }
    in_block(block, it)
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

/// Build the n-queens-shaped self-recursive `f` whose single self-call sits
/// inside a `While` (branching factor 1), with an early-return base case.
///
/// ```text
/// To f(row, n) -> Int:
///     If row == n: Return 1.
///     Let mutable count be 0.
///     While count < n: Set count to count + f(row + 1, n).
///     Return count.
/// ```
fn build_f<'a>(
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    ta: &'a Arena<TypeExpr<'a>>,
    it: &mut Interner,
) -> (Stmt<'a>, Symbol) {
    let f = it.intern("f");
    let row = it.intern("row");
    let n = it.intern("n");
    let count = it.intern("count");
    let int_ty: &TypeExpr = ta.alloc(TypeExpr::Primitive(it.intern("Int")));

    let guard = Stmt::If {
        cond: ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Eq,
            left: ea.alloc(Expr::Identifier(row)),
            right: ea.alloc(Expr::Identifier(n)),
        }),
        then_block: sa.alloc_slice(vec![Stmt::Return {
            value: Some(ea.alloc(Expr::Literal(Literal::Number(1)))),
        }]),
        else_block: None,
    };
    let self_call = ea.alloc(Expr::Call {
        function: f,
        args: vec![
            ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left: ea.alloc(Expr::Identifier(row)),
                right: ea.alloc(Expr::Literal(Literal::Number(1))),
            }),
            ea.alloc(Expr::Identifier(n)),
        ],
    });
    let loop_stmt = Stmt::While {
        cond: ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Lt,
            left: ea.alloc(Expr::Identifier(count)),
            right: ea.alloc(Expr::Identifier(n)),
        }),
        body: sa.alloc_slice(vec![Stmt::Set {
            target: count,
            value: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left: ea.alloc(Expr::Identifier(count)),
                right: self_call,
            }),
        }]),
        decreasing: None,
    };
    let body = sa.alloc_slice(vec![
        guard,
        Stmt::Let {
            var: count,
            ty: None,
            value: ea.alloc(Expr::Literal(Literal::Number(0))),
            mutable: true,
        },
        loop_stmt,
        Stmt::Return {
            value: Some(ea.alloc(Expr::Identifier(count))),
        },
    ]);
    let func = Stmt::FunctionDef {
        name: f,
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
    (func, f)
}

#[test]
fn unrolls_self_recursion_to_depth_and_bottoms_out() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();
    let (func, f) = build_f(&ea, &sa, &ta, &mut it);

    let out = inline_recursive_with(vec![func], &ea, &sa, &mut it, 3, DEFAULT_BUDGET, false);

    let body = body_of(&out, f);
    // Depth 3 with a single self-call site (b=1): exactly one real residual
    // call to `f` survives at the bottom of the unrolled nest.
    assert_eq!(
        count_calls(body, f),
        1,
        "exactly one residual call bottoms out"
    );
    // Three inlined copies → three result temps (one per inlined level).
    assert_eq!(
        count_result_temps(body, &it),
        3,
        "one result temp per inlined level"
    );
    // The early-return base case became a structured `If { result = 1 } else …`,
    // so the inlined copies introduce `If` with an `else` branch (the guard
    // restructuring) — the top body's own guard stays else-less.
    let inlined_has_else = {
        fn any_guard_else(b: Block) -> bool {
            b.iter().any(|s| match s {
                Stmt::If {
                    else_block: Some(_),
                    ..
                } => true,
                Stmt::If {
                    then_block,
                    else_block: None,
                    ..
                } => any_guard_else(then_block),
                Stmt::While { body, .. } => any_guard_else(body),
                _ => false,
            })
        }
        any_guard_else(body)
    };
    assert!(
        inlined_has_else,
        "guard restructuring produced an else branch"
    );
}

#[test]
fn depth_zero_is_a_noop() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();
    let (func, f) = build_f(&ea, &sa, &ta, &mut it);

    // Depth 0 is the kill switch's effect: no inlining at all.
    let out = inline_recursive_with(vec![func], &ea, &sa, &mut it, 0, DEFAULT_BUDGET, false);

    let body = body_of(&out, f);
    assert_eq!(
        count_calls(body, f),
        1,
        "depth 0 leaves the single original call"
    );
    assert_eq!(count_result_temps(body, &it), 0, "no inlining happened");
}

#[test]
fn budget_clamps_runaway_unrolling() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();
    let (func, f) = build_f(&ea, &sa, &ta, &mut it);

    // A tiny budget stops the unroll early; correctness is preserved because
    // the remaining self-call is left as a real recursive call.
    let out = inline_recursive_with(vec![func], &ea, &sa, &mut it, 50, 5, false);

    let body = body_of(&out, f);
    // Still exactly one residual real call (the bottom-out), and far fewer
    // than 50 inlined levels.
    assert_eq!(count_calls(body, f), 1, "a real call still bottoms out");
    assert!(
        count_result_temps(body, &it) < 50,
        "budget clamped the depth"
    );
    assert!(
        count_result_temps(body, &it) >= 1,
        "at least one level inlined"
    );
}

#[test]
fn return_inside_loop_is_ineligible() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();

    let g = it.intern("g");
    let n = it.intern("n");
    let int_ty: &TypeExpr = ta.alloc(TypeExpr::Primitive(it.intern("Int")));
    // A self-recursive function whose `Return` hides inside a `While` —
    // we never reconstruct loop-internal exits, so it must be left alone.
    let loop_stmt = Stmt::While {
        cond: ea.alloc(Expr::Literal(Literal::Number(1))),
        body: sa.alloc_slice(vec![Stmt::Return {
            value: Some(ea.alloc(Expr::Call {
                function: g,
                args: vec![ea.alloc(Expr::Identifier(n))],
            })),
        }]),
        decreasing: None,
    };
    let body = sa.alloc_slice(vec![
        loop_stmt,
        Stmt::Return {
            value: Some(ea.alloc(Expr::Literal(Literal::Number(0)))),
        },
    ]);
    let func = Stmt::FunctionDef {
        name: g,
        generics: vec![],
        params: vec![(n, int_ty)],
        body,
        return_type: Some(int_ty),
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    };

    let out = inline_recursive_with(vec![func], &ea, &sa, &mut it, 3, DEFAULT_BUDGET, false);
    let body = body_of(&out, g);
    assert_eq!(count_result_temps(body, &it), 0, "ineligible: no inlining");
}

/// A single-statement self-recursive function with `params`/`body`, returning
/// `Int`, for the deferral tests below.
fn make_fn<'a>(
    name: Symbol,
    params: Vec<Symbol>,
    body: Block<'a>,
    ta: &'a Arena<TypeExpr<'a>>,
    it: &mut Interner,
) -> Stmt<'a> {
    let int_ty: &TypeExpr = ta.alloc(TypeExpr::Primitive(it.intern("Int")));
    Stmt::FunctionDef {
        name,
        generics: vec![],
        params: params.into_iter().map(|p| (p, int_ty)).collect(),
        body,
        return_type: Some(int_ty),
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: Default::default(),
    }
}

/// Tail recursion (`Return cd(n - 1)`) belongs to TCE, not the unroller —
/// it must be DEFERRED so the existing loop-conversion can fire.
#[test]
fn tail_recursion_is_deferred_to_tce() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();
    let cd = it.intern("cd");
    let n = it.intern("n");
    let body = sa.alloc_slice(vec![
        Stmt::If {
            cond: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Eq,
                left: ea.alloc(Expr::Identifier(n)),
                right: ea.alloc(Expr::Literal(Literal::Number(0))),
            }),
            then_block: sa.alloc_slice(vec![Stmt::Return {
                value: Some(ea.alloc(Expr::Literal(Literal::Number(0)))),
            }]),
            else_block: None,
        },
        Stmt::Return {
            value: Some(ea.alloc(Expr::Call {
                function: cd,
                args: vec![ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::Subtract,
                    left: ea.alloc(Expr::Identifier(n)),
                    right: ea.alloc(Expr::Literal(Literal::Number(1))),
                })],
            })),
        },
    ]);
    let func = make_fn(cd, vec![n], body, &ta, &mut it);
    let out = inline_recursive_with(vec![func], &ea, &sa, &mut it, 4, DEFAULT_BUDGET, false);
    assert_eq!(
        count_result_temps(body_of(&out, cd), &it),
        0,
        "tail recursion deferred"
    );
}

/// Single linear recursion (`Return n * fac(n - 1)`) is the accumulator
/// transform's job — DEFERRED, not unrolled.
#[test]
fn linear_recursion_is_deferred_to_accumulator() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();
    let fac = it.intern("fac");
    let n = it.intern("n");
    let body = sa.alloc_slice(vec![
        Stmt::If {
            cond: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::LtEq,
                left: ea.alloc(Expr::Identifier(n)),
                right: ea.alloc(Expr::Literal(Literal::Number(1))),
            }),
            then_block: sa.alloc_slice(vec![Stmt::Return {
                value: Some(ea.alloc(Expr::Literal(Literal::Number(1)))),
            }]),
            else_block: None,
        },
        Stmt::Return {
            value: Some(ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Multiply,
                left: ea.alloc(Expr::Identifier(n)),
                right: ea.alloc(Expr::Call {
                    function: fac,
                    args: vec![ea.alloc(Expr::BinaryOp {
                        op: BinaryOpKind::Subtract,
                        left: ea.alloc(Expr::Identifier(n)),
                        right: ea.alloc(Expr::Literal(Literal::Number(1))),
                    })],
                }),
            })),
        },
    ]);
    let func = make_fn(fac, vec![n], body, &ta, &mut it);
    let out = inline_recursive_with(vec![func], &ea, &sa, &mut it, 4, DEFAULT_BUDGET, false);
    assert_eq!(
        count_result_temps(body_of(&out, fac), &it),
        0,
        "linear recursion deferred"
    );
}

/// Return-position tree recursion (fib: `Return fib(n-1) + fib(n-2)`, no
/// loop) is DEFERRED — codegen's recursion transforms (closed-form,
/// memoization) own return-position recursion; the unroller stays out of
/// their way and fires only on loop-interleaved recursion.
#[test]
fn tree_recursion_without_loop_is_deferred() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();
    let fib = it.intern("fib");
    let n = it.intern("n");
    let fib_minus = |sub: i64| -> &Expr {
        ea.alloc(Expr::Call {
            function: fib,
            args: vec![ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Subtract,
                left: ea.alloc(Expr::Identifier(n)),
                right: ea.alloc(Expr::Literal(Literal::Number(sub))),
            })],
        })
    };
    let body = sa.alloc_slice(vec![
        Stmt::If {
            cond: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Lt,
                left: ea.alloc(Expr::Identifier(n)),
                right: ea.alloc(Expr::Literal(Literal::Number(2))),
            }),
            then_block: sa.alloc_slice(vec![Stmt::Return {
                value: Some(ea.alloc(Expr::Identifier(n))),
            }]),
            else_block: None,
        },
        Stmt::Return {
            value: Some(ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left: fib_minus(1),
                right: fib_minus(2),
            })),
        },
    ]);
    let func = make_fn(fib, vec![n], body, &ta, &mut it);
    let out = inline_recursive_with(vec![func], &ea, &sa, &mut it, 2, DEFAULT_BUDGET, false);
    let body = body_of(&out, fib);
    assert_eq!(
        count_result_temps(body, &it),
        0,
        "return-position recursion deferred"
    );
    assert_eq!(
        count_calls(body, fib),
        2,
        "the two original self-calls are untouched"
    );
}

/// Build the fib-shaped tree recursion `Return fib(n-1) + fib(n-2)` with a
/// `If n < 2 { Return n }` guard — return-position double recursion, no loop.
fn build_fib<'a>(
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    ta: &'a Arena<TypeExpr<'a>>,
    it: &mut Interner,
) -> (Stmt<'a>, Symbol) {
    let fib = it.intern("fib");
    let n = it.intern("n");
    let fib_minus = |sub: i64| -> &Expr {
        ea.alloc(Expr::Call {
            function: fib,
            args: vec![ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Subtract,
                left: ea.alloc(Expr::Identifier(n)),
                right: ea.alloc(Expr::Literal(Literal::Number(sub))),
            })],
        })
    };
    let body = sa.alloc_slice(vec![
        Stmt::If {
            cond: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Lt,
                left: ea.alloc(Expr::Identifier(n)),
                right: ea.alloc(Expr::Literal(Literal::Number(2))),
            }),
            then_block: sa.alloc_slice(vec![Stmt::Return {
                value: Some(ea.alloc(Expr::Identifier(n))),
            }]),
            else_block: None,
        },
        Stmt::Return {
            value: Some(ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left: fib_minus(1),
                right: fib_minus(2),
            })),
        },
    ]);
    (make_fn(fib, vec![n], body, ta, it), fib)
}

/// On the RUN path, return-position tree recursion (fib) IS unrolled — the
/// AOT closed-form/memoization transforms that own it never run on the live
/// path, so the unroller is the only per-call-overhead lever there.
#[test]
fn tree_recursion_is_unrolled_on_run_path() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();
    let (func, fib) = build_fib(&ea, &sa, &ta, &mut it);
    // Depth 1: each of the two self-call sites is inlined once → two result
    // temps; each inlined copy bottoms out with its own two real self-calls,
    // so 4 residual calls remain (still terminating for any runtime depth).
    let out = inline_recursive_with(vec![func], &ea, &sa, &mut it, 1, DEFAULT_BUDGET, true);
    let body = body_of(&out, fib);
    assert_eq!(
        count_result_temps(body, &it),
        2,
        "both self-call sites inlined once"
    );
    assert_eq!(
        count_calls(body, fib),
        4,
        "each inlined copy bottoms out with real calls"
    );
}

/// The SAME fib body is left untouched on the AOT path — the run-path shape
/// extension must not bleed into AOT, where closed-form/memoization win.
#[test]
fn tree_recursion_still_deferred_on_aot_path() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();
    let (func, fib) = build_fib(&ea, &sa, &ta, &mut it);
    let out = inline_recursive_with(vec![func], &ea, &sa, &mut it, 2, DEFAULT_BUDGET, false);
    let body = body_of(&out, fib);
    assert_eq!(
        count_result_temps(body, &it),
        0,
        "AOT defers tree recursion"
    );
    assert_eq!(
        count_calls(body, fib),
        2,
        "the two original self-calls are untouched"
    );
}

/// Even on the run path, tail recursion is DEFERRED — `tail_call` rewrites it
/// to a constant-stack loop after the optimizer, so the unroller must never
/// fire on it (unrolling would clobber the strictly-better linearization).
#[test]
fn tail_recursion_still_deferred_on_run_path() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();
    let cd = it.intern("cd");
    let n = it.intern("n");
    let body = sa.alloc_slice(vec![
        Stmt::If {
            cond: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Eq,
                left: ea.alloc(Expr::Identifier(n)),
                right: ea.alloc(Expr::Literal(Literal::Number(0))),
            }),
            then_block: sa.alloc_slice(vec![Stmt::Return {
                value: Some(ea.alloc(Expr::Literal(Literal::Number(0)))),
            }]),
            else_block: None,
        },
        Stmt::Return {
            value: Some(ea.alloc(Expr::Call {
                function: cd,
                args: vec![ea.alloc(Expr::BinaryOp {
                    op: BinaryOpKind::Subtract,
                    left: ea.alloc(Expr::Identifier(n)),
                    right: ea.alloc(Expr::Literal(Literal::Number(1))),
                })],
            })),
        },
    ]);
    let func = make_fn(cd, vec![n], body, &ta, &mut it);
    let out = inline_recursive_with(vec![func], &ea, &sa, &mut it, 4, DEFAULT_BUDGET, true);
    assert_eq!(
        count_result_temps(body_of(&out, cd), &it),
        0,
        "tail recursion deferred to tail_call even on the run path"
    );
}

/// Single-linear accumulator recursion (`Return n * fac(n-1)`) is DEFERRED on
/// the run path too — `tail_call::rewrite_accumulators` strength-reduces it to
/// a loop after the optimizer; unrolling it would compete with that.
#[test]
fn linear_recursion_still_deferred_on_run_path() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();
    let fac = it.intern("fac");
    let n = it.intern("n");
    let body = sa.alloc_slice(vec![
        Stmt::If {
            cond: ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::LtEq,
                left: ea.alloc(Expr::Identifier(n)),
                right: ea.alloc(Expr::Literal(Literal::Number(1))),
            }),
            then_block: sa.alloc_slice(vec![Stmt::Return {
                value: Some(ea.alloc(Expr::Literal(Literal::Number(1)))),
            }]),
            else_block: None,
        },
        Stmt::Return {
            value: Some(ea.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Multiply,
                left: ea.alloc(Expr::Identifier(n)),
                right: ea.alloc(Expr::Call {
                    function: fac,
                    args: vec![ea.alloc(Expr::BinaryOp {
                        op: BinaryOpKind::Subtract,
                        left: ea.alloc(Expr::Identifier(n)),
                        right: ea.alloc(Expr::Literal(Literal::Number(1))),
                    })],
                }),
            })),
        },
    ]);
    let func = make_fn(fac, vec![n], body, &ta, &mut it);
    let out = inline_recursive_with(vec![func], &ea, &sa, &mut it, 4, DEFAULT_BUDGET, true);
    assert_eq!(
        count_result_temps(body_of(&out, fac), &it),
        0,
        "single-linear recursion deferred to the accumulator transform"
    );
}

/// On the run path, loop-interleaved recursion (n-queens) is capped at
/// `RUN_LOOP_DEPTH_CAP` even when a deeper depth is requested — its inlined
/// nested loops blow past the JIT region and drop to bytecode beyond the cap.
#[test]
fn loop_interleaved_recursion_is_depth_capped_on_run_path() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();
    let (func, f) = build_f(&ea, &sa, &ta, &mut it);
    // Request depth 8; the run-path cap clamps the n-queens shape to
    // RUN_LOOP_DEPTH_CAP (2) inlined levels — one result temp per level.
    let out = inline_recursive_with(vec![func], &ea, &sa, &mut it, 8, DEFAULT_BUDGET, true);
    let body = body_of(&out, f);
    assert_eq!(
        count_result_temps(body, &it),
        RUN_LOOP_DEPTH_CAP,
        "loop-interleaved recursion capped at RUN_LOOP_DEPTH_CAP on the run path"
    );
    assert_eq!(
        count_calls(body, f),
        1,
        "a single real call still bottoms out"
    );
}

/// The AOT path is NOT capped — loop-interleaved recursion keeps the full
/// requested depth there (the original n-queens-beats-gcc lever). LLVM's
/// optimizing backend absorbs the deep body that the JIT tier cannot.
#[test]
fn loop_interleaved_recursion_is_not_capped_on_aot_path() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();
    let (func, f) = build_f(&ea, &sa, &ta, &mut it);
    let out = inline_recursive_with(vec![func], &ea, &sa, &mut it, 5, DEFAULT_BUDGET, false);
    let body = body_of(&out, f);
    assert_eq!(
        count_result_temps(body, &it),
        5,
        "AOT loop-interleaved recursion unrolls to the full requested depth"
    );
}

/// Tree recursion is NOT subject to the loop cap on the run path — it gets
/// the full (deeper) requested depth, since its straight-line body tiers
/// cleanly even when enlarged.
#[test]
fn tree_recursion_is_not_depth_capped_on_run_path() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let ta: Arena<TypeExpr> = Arena::new();
    let mut it = Interner::new();
    let (func, fib) = build_fib(&ea, &sa, &ta, &mut it);
    // Depth 3 on a binary tree: each level doubles the inlined copies
    // (2 + 4 + 8 = 14 result temps), with no loop cap applied.
    let out = inline_recursive_with(vec![func], &ea, &sa, &mut it, 3, DEFAULT_BUDGET, true);
    let body = body_of(&out, fib);
    assert_eq!(
        count_result_temps(body, &it),
        14,
        "tree recursion unrolls uncapped"
    );
}
