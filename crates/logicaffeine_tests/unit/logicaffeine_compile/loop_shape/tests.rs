use super::*;
use crate::arena::Arena;
use crate::intern::Interner;

fn num<'a>(ea: &'a Arena<Expr<'a>>, n: i64) -> &'a Expr<'a> {
    ea.alloc(Expr::Literal(Literal::Number(n)))
}

fn var<'a>(ea: &'a Arena<Expr<'a>>, s: Symbol) -> &'a Expr<'a> {
    ea.alloc(Expr::Identifier(s))
}

fn op<'a>(
    ea: &'a Arena<Expr<'a>>,
    op: BinaryOpKind,
    left: &'a Expr<'a>,
    right: &'a Expr<'a>,
) -> &'a Expr<'a> {
    ea.alloc(Expr::BinaryOp { op, left, right })
}

/// `Set i to i + 1.`
fn step<'a>(ea: &'a Arena<Expr<'a>>, i: Symbol) -> Stmt<'a> {
    Stmt::Set {
        target: i,
        value: op(ea, BinaryOpKind::Add, var(ea, i), num(ea, 1)),
    }
}

/// `If i equals k: Break.`
fn break_at<'a>(ea: &'a Arena<Expr<'a>>, sa: &'a Arena<Stmt<'a>>, i: Symbol, k: i64) -> Stmt<'a> {
    Stmt::If {
        cond: op(ea, BinaryOpKind::Eq, var(ea, i), num(ea, k)),
        then_block: sa.alloc_slice(vec![Stmt::Break]),
        else_block: None,
    }
}

/// `Let i be 0. While i < 100: <body> Set i to i + 1.`, as the recognizer sees it.
fn counted_while<'a>(
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    i: Symbol,
    mut body: Vec<Stmt<'a>>,
) -> [Stmt<'a>; 2] {
    body.push(step(ea, i));
    [
        Stmt::Let {
            var: i,
            ty: None,
            value: num(ea, 0),
            mutable: true,
        },
        Stmt::While {
            cond: op(ea, BinaryOpKind::Lt, var(ea, i), num(ea, 100)),
            body: sa.alloc_slice(body),
            decreasing: None,
        },
    ]
}

#[test]
fn a_counted_while_is_counted() {
    let (ea, sa, mut it) = (Arena::new(), Arena::new(), Interner::new());
    let i = it.intern("i");
    let loop_ = counted_while(&ea, &sa, i, vec![]);
    let refs: Vec<&Stmt> = loop_.iter().collect();
    let (cl, consumed) =
        extract_counted_while(&refs, 0).expect("a unit-stride loop to a fixed bound");
    assert_eq!((cl.var, cl.inclusive, consumed), (i, false, 2));
}

#[test]
fn a_while_a_break_of_its_own_can_end_is_not_counted() {
    let (ea, sa, mut it) = (Arena::new(), Arena::new(), Interner::new());
    let i = it.intern("i");
    let loop_ = counted_while(&ea, &sa, i, vec![break_at(&ea, &sa, i, 7)]);
    let refs: Vec<&Stmt> = loop_.iter().collect();
    assert!(
        extract_counted_while(&refs, 0).is_none(),
        "a Break leaves the counter where it broke, not at the bound: the loop runs no fixed number of times"
    );
}

#[test]
fn a_break_that_belongs_to_a_nested_loop_leaves_the_outer_loop_counted() {
    let (ea, sa, mut it) = (Arena::new(), Arena::new(), Interner::new());
    let (i, j) = (it.intern("i"), it.intern("j"));
    let inner = Stmt::While {
        cond: op(&ea, BinaryOpKind::Lt, var(&ea, j), num(&ea, 10)),
        body: sa.alloc_slice(vec![break_at(&ea, &sa, j, 2), step(&ea, j)]),
        decreasing: None,
    };
    let loop_ = counted_while(&ea, &sa, i, vec![inner]);
    let refs: Vec<&Stmt> = loop_.iter().collect();
    assert!(
        extract_counted_while(&refs, 0).is_some(),
        "the inner loop's Break ends only the inner loop"
    );
    assert!(!leaves_by_break(&[Stmt::While {
        cond: op(&ea, BinaryOpKind::Lt, var(&ea, j), num(&ea, 10)),
        body: sa.alloc_slice(vec![Stmt::Break]),
        decreasing: None,
    }]));
}

#[test]
fn a_counted_repeat_a_break_of_its_own_can_end_is_not_counted() {
    let (ea, sa, mut it) = (Arena::new(), Arena::new(), Interner::new());
    let k = it.intern("k");
    let range = ea.alloc(Expr::Range {
        start: num(&ea, 1),
        end: num(&ea, 100),
    });
    let counted = Stmt::Repeat {
        pattern: Pattern::Identifier(k),
        iterable: range,
        body: sa.alloc_slice(vec![step(&ea, k)]),
    };
    assert!(extract_counted_repeat(&counted).is_some());
    let breaking = Stmt::Repeat {
        pattern: Pattern::Identifier(k),
        iterable: range,
        body: sa.alloc_slice(vec![break_at(&ea, &sa, k, 4)]),
    };
    assert!(
        extract_counted_repeat(&breaking).is_none(),
        "a Break ends a counted Repeat early"
    );
}
