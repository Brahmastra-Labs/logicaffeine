use super::*;
use crate::arena::Arena;
use crate::intern::Interner;

fn lit_num<'a>(ea: &'a Arena<Expr<'a>>, n: i64) -> &'a Expr<'a> {
    ea.alloc(Expr::Literal(Literal::Number(n)))
}
fn ident<'a>(ea: &'a Arena<Expr<'a>>, s: Symbol) -> &'a Expr<'a> {
    ea.alloc(Expr::Identifier(s))
}
fn add<'a>(ea: &'a Arena<Expr<'a>>, l: &'a Expr<'a>, r: &'a Expr<'a>) -> &'a Expr<'a> {
    ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: l,
        right: r,
    })
}
fn new_seq<'a>(ea: &'a Arena<Expr<'a>>, name: Symbol) -> &'a Expr<'a> {
    ea.alloc(Expr::New {
        type_name: name,
        type_args: vec![],
        init_fields: vec![],
    })
}
fn let_stmt<'a>(var: Symbol, value: &'a Expr<'a>) -> Stmt<'a> {
    Stmt::Let {
        var,
        ty: None,
        value,
        mutable: true,
    }
}

#[test]
fn let_chain_tracks_type_and_interval() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let mut it = Interner::new();
    let x = it.intern("x");
    let y = it.intern("y");
    // Let x be 5. Let y be x + 7.
    let five = lit_num(&ea, 5);
    let xref = ident(&ea, x);
    let seven = lit_num(&ea, 7);
    let sum = add(&ea, xref, seven);
    let stmts = vec![let_stmt(x, five), let_stmt(y, sum)];

    let (_out, st) = rich_abstract_interp_stmts(stmts, &ea, &sa);
    let vx = st.value_of(x);
    let vy = st.value_of(y);
    assert_eq!(vx.ty, TypeAbstraction::Concrete(TypeTag::Int));
    assert_eq!(vx.interval.is_exact(), Some(5));
    assert_eq!(vy.ty, TypeAbstraction::Concrete(TypeTag::Int));
    assert_eq!(vy.interval.is_exact(), Some(12));
    assert_eq!(vx.nullability, Nullability::Definite);
}

#[test]
fn new_and_push_track_shape() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let mut it = Interner::new();
    let items = it.intern("items");
    let seq_ty = it.intern("Seq");
    // Let items be a new Seq. Push 10 to items.
    let new_e = new_seq(&ea, seq_ty);
    let ten = lit_num(&ea, 10);
    let items_ref = ident(&ea, items);
    let stmts = vec![
        let_stmt(items, new_e),
        Stmt::Push {
            value: ten,
            collection: items_ref,
        },
    ];
    let (_out, st) = rich_abstract_interp_stmts(stmts, &ea, &sa);
    let v = st.value_of(items);
    assert_eq!(v.shape, CollectionShape::Singleton);
    assert!(v.shape.is_definitely_nonempty());
}

#[test]
fn alias_push_keeps_aliases_consistent() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let mut it = Interner::new();
    let items = it.intern("items");
    let a = it.intern("a");
    let seq_ty = it.intern("Seq");
    // Let items be a new Seq. Push 1 to items. Let a be items. Push 2 to a.
    let new_e = new_seq(&ea, seq_ty);
    let one = lit_num(&ea, 1);
    let two = lit_num(&ea, 2);
    let items_ref1 = ident(&ea, items);
    let items_ref2 = ident(&ea, items);
    let a_ref = ident(&ea, a);
    let stmts = vec![
        let_stmt(items, new_e),
        Stmt::Push {
            value: one,
            collection: items_ref1,
        },
        let_stmt(a, items_ref2), // a aliases items
        Stmt::Push {
            value: two,
            collection: a_ref,
        }, // mutate through alias
    ];
    let (_out, st) = rich_abstract_interp_stmts(stmts, &ea, &sa);
    let vitems = st.value_of(items);
    let va = st.value_of(a);
    // pushing through `a` updated `items` (shared Rc) — both see size 2.
    assert!(vitems.shape.is_definitely_nonempty());
    assert_eq!(vitems.shape, va.shape);
    assert_eq!(vitems.shape, CollectionShape::KnownSize(2));
}

#[test]
fn if_branches_join() {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let mut it = Interner::new();
    let x = it.intern("x");
    let n = it.intern("n"); // unbound → dynamic condition
                            // Let mutable x be 0. If n > 5: Set x to 10. Otherwise: Set x to 20.
    let zero = lit_num(&ea, 0);
    let nref = ident(&ea, n);
    let five = lit_num(&ea, 5);
    let cond = ea.alloc(Expr::BinaryOp {
        op: BinaryOpKind::Gt,
        left: nref,
        right: five,
    });
    let ten = lit_num(&ea, 10);
    let twenty = lit_num(&ea, 20);
    let then_block: &[Stmt] = sa.alloc_slice(vec![Stmt::Set {
        target: x,
        value: ten,
    }]);
    let else_block: &[Stmt] = sa.alloc_slice(vec![Stmt::Set {
        target: x,
        value: twenty,
    }]);
    let stmts = vec![
        let_stmt(x, zero),
        Stmt::If {
            cond,
            then_block,
            else_block: Some(else_block),
        },
    ];
    let (_out, st) = rich_abstract_interp_stmts(stmts, &ea, &sa);
    let vx = st.value_of(x);
    // x is 10 or 20 after the join → interval [10, 20], type Int
    assert_eq!(vx.ty, TypeAbstraction::Concrete(TypeTag::Int));
    assert_eq!(vx.interval.lo, Bound::Finite(10));
    assert_eq!(vx.interval.hi, Bound::Finite(20));
}

fn cmp<'a>(
    ea: &'a Arena<Expr<'a>>,
    op: BinaryOpKind,
    l: &'a Expr<'a>,
    r: &'a Expr<'a>,
) -> &'a Expr<'a> {
    ea.alloc(Expr::BinaryOp {
        op,
        left: l,
        right: r,
    })
}

/// Whether `iv` admits `v`.
fn admits(iv: &Interval, v: i64) -> bool {
    iv.lo.cmp_bound(&Bound::Finite(v)) != std::cmp::Ordering::Greater
        && Bound::Finite(v).cmp_bound(&iv.hi) != std::cmp::Ordering::Greater
}

/// `Let x be 0. Let i be 0. While i < 10: <body>` with `body` built from `x`, `i` and `n` (a value the walk
/// knows nothing of); the state after the loop.
fn after_counting_loop(
    body: impl for<'a> Fn(
        &'a Arena<Expr<'a>>,
        &'a Arena<Stmt<'a>>,
        Symbol,
        Symbol,
        Symbol,
    ) -> Vec<Stmt<'a>>,
    ask: impl Fn(&RichAbstractState, Symbol, Symbol),
) {
    let ea: Arena<Expr> = Arena::new();
    let sa: Arena<Stmt> = Arena::new();
    let mut it = Interner::new();
    let (x, i, n) = (it.intern("x"), it.intern("i"), it.intern("n"));
    let cond = cmp(&ea, BinaryOpKind::Lt, ident(&ea, i), lit_num(&ea, 10));
    let stmts = vec![
        let_stmt(x, lit_num(&ea, 0)),
        let_stmt(i, lit_num(&ea, 0)),
        Stmt::While {
            cond,
            body: sa.alloc_slice(body(&ea, &sa, x, i, n)),
            decreasing: None,
        },
    ];
    let (_out, st) = rich_abstract_interp_stmts(stmts, &ea, &sa);
    ask(&st, x, i);
}

/// `If i == n: Break.`
fn break_at_n<'a>(
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    i: Symbol,
    n: Symbol,
) -> Stmt<'a> {
    Stmt::If {
        cond: cmp(ea, BinaryOpKind::Eq, ident(ea, i), ident(ea, n)),
        then_block: sa.alloc_slice(vec![Stmt::Break]),
        else_block: None,
    }
}

/// `Set i to i + 1.`
fn step<'a>(ea: &'a Arena<Expr<'a>>, i: Symbol) -> Stmt<'a> {
    Stmt::Set {
        target: i,
        value: add(ea, ident(ea, i), lit_num(ea, 1)),
    }
}

#[test]
fn a_loop_left_by_break_keeps_what_its_last_iteration_wrote() {
    // While i < 10: Set x to 50. If i == n: Break. Set x to 7. Set i to i + 1.
    // The loop head only ever sees x = 0 or 7; the iteration that breaks leaves x = 50.
    after_counting_loop(
        |ea, sa, x, i, n| {
            vec![
                Stmt::Set {
                    target: x,
                    value: lit_num(ea, 50),
                },
                break_at_n(ea, sa, i, n),
                Stmt::Set {
                    target: x,
                    value: lit_num(ea, 7),
                },
                step(ea, i),
            ]
        },
        |st, x, _| {
            let vx = st.value_of(x);
            assert!(
                admits(&vx.interval, 50),
                "x may be 50 after a loop that breaks right after writing 50: {:?}",
                vx.interval
            );
        },
    );
}

#[test]
fn a_loop_left_by_break_promises_nothing_about_its_condition() {
    // While i < 10: If i == n: Break. Set i to i + 1.
    after_counting_loop(
        |ea, sa, _, i, n| vec![break_at_n(ea, sa, i, n), step(ea, i)],
        |st, _, i| {
            let vi = st.value_of(i);
            assert!(
                admits(&vi.interval, 3),
                "a Break may leave i at 3, below the bound: {:?}",
                vi.interval
            );
        },
    );
}

#[test]
fn a_loop_left_only_by_its_condition_leaves_its_counter_past_the_bound() {
    // While i < 10: Set i to i + 1. — the exit says i >= 10, and the walk keeps saying so.
    after_counting_loop(
        |ea, _, _, i, _| vec![step(ea, i)],
        |st, _, i| {
            let vi = st.value_of(i);
            assert_eq!(
                vi.interval.lo,
                Bound::Finite(10),
                "the condition failed when the loop ended: {:?}",
                vi.interval
            );
            assert!(!admits(&vi.interval, 3), "{:?}", vi.interval);
        },
    );
}
