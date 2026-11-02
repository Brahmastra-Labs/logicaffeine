use super::*;

struct B<'a> {
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
}
impl<'a> B<'a> {
    fn id(&self, s: Symbol) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Identifier(s))
    }
    fn num(&self, n: i64) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Literal(Literal::Number(n)))
    }
    fn bin(&self, op: BinaryOpKind, l: &'a Expr<'a>, r: &'a Expr<'a>) -> &'a Expr<'a> {
        self.ea.alloc(Expr::BinaryOp {
            op,
            left: l,
            right: r,
        })
    }
    fn index(&self, coll: &'a Expr<'a>, idx: &'a Expr<'a>) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Index {
            collection: coll,
            index: idx,
        })
    }
    fn block(&self, v: Vec<Stmt<'a>>) -> Block<'a> {
        self.sa.alloc_slice(v)
    }
}

/// Names used across the knapsack-shaped fixtures.
struct Names {
    w: Symbol,
    cap: Symbol,
    wi: Symbol,
    vi: Symbol,
    prev: Symbol,
    curr: Symbol,
    best: Symbol,
    take: Symbol,
}
fn names(it: &mut Interner) -> Names {
    Names {
        w: it.intern("w"),
        cap: it.intern("capacity"),
        wi: it.intern("wi"),
        vi: it.intern("vi"),
        prev: it.intern("prev"),
        curr: it.intern("curr"),
        best: it.intern("best"),
        take: it.intern("take"),
    }
}

/// `[ Let w be 0 ; While w <= capacity: <body> ]` with the knapsack inner
/// body, parameterized so the negative tests can perturb one piece.
fn knapsack_loop<'a>(
    b: &B<'a>,
    n: &Names,
    guard: &'a Expr<'a>,
    step: i64,
    with_break: bool,
    guarded_read: bool,
) -> Vec<Stmt<'a>> {
    use BinaryOpKind::*;
    let let_best = Stmt::Let {
        var: n.best,
        ty: None,
        value: b.index(b.id(n.prev), b.bin(Add, b.id(n.w), b.num(1))),
        mutable: true,
    };
    // then-block of the guard: `Let take be item(w-wi+1) of prev + vi; If take > best: Set best to take`
    let take_index = b.index(
        b.id(n.prev),
        b.bin(Add, b.bin(Subtract, b.id(n.w), b.id(n.wi)), b.num(1)),
    );
    let take_val = if guarded_read {
        b.bin(Add, take_index, b.id(n.vi))
    } else {
        // no IV-indexed read in the guarded branch (heuristic should decline)
        b.bin(Add, b.id(n.wi), b.id(n.vi))
    };
    let let_take = Stmt::Let {
        var: n.take,
        ty: None,
        value: take_val,
        mutable: false,
    };
    let inner_if = Stmt::If {
        cond: b.bin(Gt, b.id(n.take), b.id(n.best)),
        then_block: b.block(vec![Stmt::Set {
            target: n.best,
            value: b.id(n.take),
        }]),
        else_block: None,
    };
    let guard_if = Stmt::If {
        cond: guard,
        then_block: b.block(vec![let_take, inner_if]),
        else_block: None,
    };
    let push = Stmt::Push {
        value: b.id(n.best),
        collection: b.id(n.curr),
    };
    let inc = Stmt::Set {
        target: n.w,
        value: b.bin(Add, b.id(n.w), b.num(step)),
    };
    let mut body = vec![let_best, guard_if, push];
    if with_break {
        body.push(Stmt::Break);
    }
    body.push(inc);
    let while_stmt = Stmt::While {
        cond: b.bin(LtEq, b.id(n.w), b.id(n.cap)),
        body: b.block(body),
        decreasing: None,
    };
    vec![
        Stmt::Let {
            var: n.w,
            ty: None,
            value: b.num(0),
            mutable: true,
        },
        while_stmt,
    ]
}

fn run<'a>(
    input: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
) -> Vec<Stmt<'a>> {
    loop_split_stmts(input, ea, sa, it, &OptimizationConfig::all_on())
}

/// A `Stmt::If` whose condition is `iv >= sym` (the outer guard), at any depth.
fn has_iv_ge_guard(stmts: &[Stmt], iv: Symbol) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            let here = matches!(cond, Expr::BinaryOp { op: BinaryOpKind::GtEq, left, .. }
                if matches!(&**left, Expr::Identifier(x) if *x == iv));
            here || has_iv_ge_guard(then_block, iv)
                || else_block.is_some_and(|eb| has_iv_ge_guard(eb, iv))
        }
        Stmt::While { body, .. } => has_iv_ge_guard(body, iv),
        _ => false,
    })
}

fn mentions_sym(stmts: &[Stmt], sym: Symbol) -> bool {
    fn e(x: &Expr, s: Symbol) -> bool {
        mentions(x, s)
    }
    stmts.iter().any(|st| match st {
        Stmt::Let { var, value, .. } => *var == sym || e(value, sym),
        Stmt::Set { value, .. } => e(value, sym),
        Stmt::Push { value, collection } => e(value, sym) || e(collection, sym),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            e(cond, sym)
                || mentions_sym(then_block, sym)
                || else_block.is_some_and(|eb| mentions_sym(eb, sym))
        }
        Stmt::While { body, .. } => mentions_sym(body, sym),
        _ => false,
    })
}

#[test]
fn splits_ge_guard_into_two_loops() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let n = names(&mut it);
    let b = B { ea: &ea, sa: &sa };
    let guard = b.bin(BinaryOpKind::GtEq, b.id(n.w), b.id(n.wi));
    let input = knapsack_loop(&b, &n, guard, 1, false, true);
    let out = run(input, &ea, &sa, &mut it);

    // Output: [ Let w be 0 , If version { ... } else { ... } ]
    assert_eq!(out.len(), 2, "init kept, loop replaced by one version-If");
    assert!(matches!(out[0], Stmt::Let { var, .. } if var == n.w));
    let (then_block, else_block) = match &out[1] {
        Stmt::If {
            then_block,
            else_block: Some(eb),
            ..
        } => (*then_block, *eb),
        other => panic!("expected version-If with else, got {other:?}"),
    };
    // then = [Set w, While(prefix), Set w, While(suffix)]
    assert_eq!(then_block.len(), 4, "re-init, prefix, re-init, suffix");
    let prefix = match &then_block[1] {
        Stmt::While { body, .. } => *body,
        other => panic!("expected prefix While, got {other:?}"),
    };
    let suffix = match &then_block[3] {
        Stmt::While { body, .. } => *body,
        other => panic!("expected suffix While, got {other:?}"),
    };
    // The outer `w >= wi` guard is GONE from both sub-loops (dropped / inlined).
    assert!(!has_iv_ge_guard(prefix, n.w), "prefix has no outer guard");
    assert!(!has_iv_ge_guard(suffix, n.w), "suffix has no outer guard");
    // The prefix dropped the then-block: no `take`.
    assert!(!mentions_sym(prefix, n.take), "prefix has no `take`");
    // The suffix inlined the then-block: it has `take` and the inner if.
    assert!(mentions_sym(suffix, n.take), "suffix inlines `take`");
    // The fallback (else) preserves the original guarded loop.
    assert!(
        has_iv_ge_guard(&else_block, n.w),
        "fallback keeps the guard"
    );
    // Push order preserved: both sub-loops still push curr.
    assert!(mentions_sym(prefix, n.curr) && mentions_sym(suffix, n.curr));
}

#[test]
fn gt_guard_splits_at_threshold_plus_one() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let n = names(&mut it);
    let b = B { ea: &ea, sa: &sa };
    let guard = b.bin(BinaryOpKind::Gt, b.id(n.w), b.id(n.wi));
    let input = knapsack_loop(&b, &n, guard, 1, false, true);
    let out = run(input, &ea, &sa, &mut it);
    // suffix re-init `Set w to (wi + 1)` for a strict `>` guard.
    let then_block = match &out[1] {
        Stmt::If { then_block, .. } => *then_block,
        other => panic!("expected version-If, got {other:?}"),
    };
    match &then_block[2] {
        Stmt::Set { target, value } if *target == n.w => assert!(
            matches!(value, Expr::BinaryOp { op: BinaryOpKind::Add, right, .. }
                if matches!(&**right, Expr::Literal(Literal::Number(1)))),
            "suffix starts at wi + 1, got {value:?}"
        ),
        other => panic!("expected `Set w to wi + 1`, got {other:?}"),
    }
}

#[test]
fn no_fire_on_break() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let n = names(&mut it);
    let b = B { ea: &ea, sa: &sa };
    let guard = b.bin(BinaryOpKind::GtEq, b.id(n.w), b.id(n.wi));
    let input = knapsack_loop(&b, &n, guard, 1, true, true);
    let out = run(input, &ea, &sa, &mut it);
    assert!(
        matches!(out[1], Stmt::While { .. }),
        "loop left intact (Break present)"
    );
}

#[test]
fn no_fire_without_affine_indexed_read() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let n = names(&mut it);
    let b = B { ea: &ea, sa: &sa };
    let guard = b.bin(BinaryOpKind::GtEq, b.id(n.w), b.id(n.wi));
    let input = knapsack_loop(&b, &n, guard, 1, false, false);
    let out = run(input, &ea, &sa, &mut it);
    assert!(
        matches!(out[1], Stmt::While { .. }),
        "no IV-indexed read → no split"
    );
}

#[test]
fn no_fire_on_forward_indexed_read() {
    // A guarded FORWARD read `prev[w + 1]` is already speculatable, so the
    // guard isn't load-protecting and the split would not unblock the
    // vectorizer — the pass declines.
    use BinaryOpKind::*;
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let n = names(&mut it);
    let b = B { ea: &ea, sa: &sa };
    let let_best = Stmt::Let {
        var: n.best,
        ty: None,
        value: b.index(b.id(n.prev), b.bin(Add, b.id(n.w), b.num(1))),
        mutable: true,
    };
    // guarded branch reads prev[w + 1] (forward, no backref).
    let fwd = b.index(b.id(n.prev), b.bin(Add, b.id(n.w), b.num(1)));
    let let_take = Stmt::Let {
        var: n.take,
        ty: None,
        value: b.bin(Add, fwd, b.id(n.vi)),
        mutable: false,
    };
    let guard_if = Stmt::If {
        cond: b.bin(GtEq, b.id(n.w), b.id(n.wi)),
        then_block: b.block(vec![let_take]),
        else_block: None,
    };
    let push = Stmt::Push {
        value: b.id(n.best),
        collection: b.id(n.curr),
    };
    let inc = Stmt::Set {
        target: n.w,
        value: b.bin(Add, b.id(n.w), b.num(1)),
    };
    let while_stmt = Stmt::While {
        cond: b.bin(LtEq, b.id(n.w), b.id(n.cap)),
        body: b.block(vec![let_best, guard_if, push, inc]),
        decreasing: None,
    };
    let input = vec![
        Stmt::Let {
            var: n.w,
            ty: None,
            value: b.num(0),
            mutable: true,
        },
        while_stmt,
    ];
    let out = run(input, &ea, &sa, &mut it);
    assert!(
        matches!(out[1], Stmt::While { .. }),
        "forward read → no split"
    );
}

#[test]
fn no_fire_non_unit_step() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let n = names(&mut it);
    let b = B { ea: &ea, sa: &sa };
    let guard = b.bin(BinaryOpKind::GtEq, b.id(n.w), b.id(n.wi));
    let input = knapsack_loop(&b, &n, guard, 2, false, true);
    let out = run(input, &ea, &sa, &mut it);
    assert!(matches!(out[1], Stmt::While { .. }), "step != 1 → no split");
}

#[test]
fn no_fire_when_threshold_written() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let n = names(&mut it);
    let b = B { ea: &ea, sa: &sa };
    let guard = b.bin(BinaryOpKind::GtEq, b.id(n.w), b.id(n.wi));
    let mut input = knapsack_loop(&b, &n, guard, 1, false, true);
    // Inject `Set wi to wi + 1` into the loop body → threshold not invariant.
    if let Stmt::While {
        body,
        cond,
        decreasing,
    } = input.pop().unwrap()
    {
        let mut v: Vec<Stmt> = body.to_vec();
        v.insert(
            0,
            Stmt::Set {
                target: n.wi,
                value: b.bin(BinaryOpKind::Add, b.id(n.wi), b.num(1)),
            },
        );
        input.push(Stmt::While {
            cond,
            body: b.block(v),
            decreasing,
        });
    }
    let out = run(input, &ea, &sa, &mut it);
    assert!(
        matches!(out[1], Stmt::While { .. }),
        "threshold mutated → no split"
    );
}

#[test]
fn disabling_loop_split_leaves_loop_intact() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let n = names(&mut it);
    let b = B { ea: &ea, sa: &sa };
    let guard = b.bin(BinaryOpKind::GtEq, b.id(n.w), b.id(n.wi));
    let input = knapsack_loop(&b, &n, guard, 1, false, true);
    let cfg = OptimizationConfig::all_on().disable(Opt::LoopSplit);
    let out = loop_split_stmts(input, &ea, &sa, &mut it, &cfg);
    assert!(
        matches!(out[1], Stmt::While { .. }),
        "disabling LoopSplit leaves loop intact"
    );
}
