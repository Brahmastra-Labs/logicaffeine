use super::*;
use crate::intern::Interner;

struct Vars {
    i: Interner,
}
impl Vars {
    fn new() -> Self {
        Vars { i: Interner::new() }
    }
    fn s(&mut self, name: &str) -> Symbol {
        self.i.intern(name)
    }
}

/// `e + n`.
fn plus(e: &LinearExpr, n: i64) -> LinearExpr {
    e.add(&konst(n))
}
/// `c·x`.
fn term(x: Symbol, c: i64) -> LinearExpr {
    var(x).scale(&Rational::from_i64(c))
}

// ---- prove: the three hard-loss relations the engine exists to discharge ----

#[test]
fn prove_knapsack_lower_bound() {
    // Guard w >= wi ⊢ (w - wi + 1) >= 1, i.e. (w - wi) >= 0.
    let mut v = Vars::new();
    let (w, wi) = (v.s("w"), v.s("wi"));
    let guard = ge(&var(w), &var(wi)); // w >= wi
    let goal = var(w).sub(&var(wi)); // w - wi >= 0
    assert!(prove(&[guard], &goal));
}

#[test]
fn prove_knapsack_upper_bound() {
    // Loop w <= cap, element wi >= 0, length(prev) = cap+1 ⊢
    // (w - wi + 1) <= cap + 1, i.e. cap - w + wi >= 0.
    let mut v = Vars::new();
    let (w, wi, cap) = (v.s("w"), v.s("wi"), v.s("cap"));
    let facts = vec![le(&var(w), &var(cap)), nonneg(&var(wi))];
    let goal = var(cap).sub(&var(w)).add(&var(wi)); // cap - w + wi
    assert!(prove(&facts, &goal));
}

#[test]
fn knapsack_upper_unprovable_without_element_bound() {
    // Without wi >= 0 the upper bound does NOT hold (wi could be negative).
    let mut v = Vars::new();
    let (w, wi, cap) = (v.s("w"), v.s("wi"), v.s("cap"));
    let facts = vec![le(&var(w), &var(cap))];
    let goal = var(cap).sub(&var(w)).add(&var(wi));
    assert!(!prove(&facts, &goal));
}

#[test]
fn prove_string_search_window() {
    // Outer i <= T - P, inner j <= P - 1, i >= 0, j >= 0 ⊢ i + j <= T - 1,
    // i.e. (T - 1) - (i + j) >= 0.
    let mut v = Vars::new();
    let (i, j, t, p) = (v.s("i"), v.s("j"), v.s("t"), v.s("p"));
    let facts = vec![
        le(&var(i), &var(t).sub(&var(p))), // i <= T - P
        le(&var(j), &plus(&var(p), -1)),   // j <= P - 1
        nonneg(&var(i)),
        nonneg(&var(j)),
    ];
    let ij = var(i).add(&var(j));
    let goal = plus(&var(t), -1).sub(&ij); // (T - 1) - (i + j)
    assert!(prove(&facts, &goal));
}

#[test]
fn prove_graph_bfs_element_bound() {
    // u is an element of adj, all values in [0, n-1]; length(dist) = n.
    // Prove dist[u] (1-based u+1): lower u >= 0 and upper n - (u+1) >= 0.
    let mut v = Vars::new();
    let (u, n) = (v.s("u"), v.s("n"));
    let facts = vec![nonneg(&var(u)), le(&var(u), &plus(&var(n), -1))]; // 0 <= u <= n-1
    assert!(prove(&facts, &var(u))); // lower: (u+1) - 1 = u >= 0
    let upper = var(n).sub(&plus(&var(u), 1)); // n - (u + 1)
    assert!(prove(&facts, &upper));
}

#[test]
fn prove_is_not_overeager() {
    // i <= n is NOT enough to prove i < n (the strict goal n - i - 1 >= 0).
    let mut v = Vars::new();
    let (i, n) = (v.s("i"), v.s("n"));
    let weak = le(&var(i), &var(n)); // i <= n
    let strict_goal = plus(&var(n).sub(&var(i)), -1); // n - i - 1
    assert!(!prove(&[weak], &strict_goal));
    // But i < n DOES prove i <= n (n - i >= 0).
    let strong = lt(&var(i), &var(n)); // i < n
    let nonstrict_goal = var(n).sub(&var(i)); // n - i
    assert!(prove(&[strong], &nonstrict_goal));
}

#[test]
fn prove_empty_facts_only_proves_trivial() {
    let mut v = Vars::new();
    let x = v.s("x");
    assert!(!prove(&[], &var(x))); // cannot prove x >= 0 with no facts
    assert!(prove(&[], &konst(5))); // 5 >= 0 holds unconditionally
    assert!(!prove(&[], &konst(-1))); // -1 >= 0 never holds
}

#[test]
fn prove_flattened_2d_constant_stride() {
    // arr[i*10 + j] with i,j ∈ [0,9], length(arr) = 100. The constant-stride
    // flattened access the single-var recognizer cannot match: prove
    // 1 <= i*10 + j + 1 <= 100.
    let mut v = Vars::new();
    let (i, j) = (v.s("i"), v.s("j"));
    let facts = vec![
        nonneg(&var(i)),
        le(&var(i), &konst(9)),
        nonneg(&var(j)),
        le(&var(j), &konst(9)),
    ];
    let idx0 = term(i, 10).add(&var(j)); // i*10 + j (0-based)
    assert!(prove(&facts, &idx0)); // lower: idx0 >= 0
    let upper = konst(99).sub(&idx0); // 99 - (i*10 + j) >= 0  (≤ len-1)
    assert!(prove(&facts, &upper));
}

// ---- lin_of: AST reification ----

fn num<'a>(arena: &'a crate::arena::Arena<Expr<'a>>, n: i64) -> &'a Expr<'a> {
    arena.alloc(Expr::Literal(Literal::Number(n)))
}
fn ident<'a>(arena: &'a crate::arena::Arena<Expr<'a>>, s: Symbol) -> &'a Expr<'a> {
    arena.alloc(Expr::Identifier(s))
}
fn binop<'a>(
    arena: &'a crate::arena::Arena<Expr<'a>>,
    op: BinaryOpKind,
    l: &'a Expr<'a>,
    r: &'a Expr<'a>,
) -> &'a Expr<'a> {
    arena.alloc(Expr::BinaryOp {
        op,
        left: l,
        right: r,
    })
}

#[test]
fn lin_of_extracts_affine_forms() {
    let arena = crate::arena::Arena::new();
    let mut v = Vars::new();
    let (w, wi) = (v.s("w"), v.s("wi"));
    // w - wi + 1
    let e = binop(
        &arena,
        BinaryOpKind::Add,
        binop(
            &arena,
            BinaryOpKind::Subtract,
            ident(&arena, w),
            ident(&arena, wi),
        ),
        num(&arena, 1),
    );
    let lin = lin_of(e).unwrap();
    assert_eq!(lin, plus(&var(w).sub(&var(wi)), 1));
}

#[test]
fn lin_of_scales_by_constant_factor() {
    let arena = crate::arena::Arena::new();
    let mut v = Vars::new();
    let i = v.s("i");
    let e1 = binop(
        &arena,
        BinaryOpKind::Multiply,
        num(&arena, 3),
        ident(&arena, i),
    );
    let e2 = binop(
        &arena,
        BinaryOpKind::Multiply,
        ident(&arena, i),
        num(&arena, 3),
    );
    assert_eq!(lin_of(e1).unwrap(), term(i, 3));
    assert_eq!(lin_of(e2).unwrap(), term(i, 3));
}

#[test]
fn lin_of_rejects_nonlinear() {
    let arena = crate::arena::Arena::new();
    let mut v = Vars::new();
    let (i, j) = (v.s("i"), v.s("j"));
    let prod = binop(
        &arena,
        BinaryOpKind::Multiply,
        ident(&arena, i),
        ident(&arena, j),
    );
    assert!(lin_of(prod).is_none()); // i·j nonlinear
    let m = binop(
        &arena,
        BinaryOpKind::Modulo,
        ident(&arena, i),
        num(&arena, 3),
    );
    assert!(lin_of(m).is_none()); // i % 3 not affine
}
