//! Lib-unit coverage for the TREE-WALKING evaluator (`eval_bool_tree` → `eval`/`lookup`/`apply`)
//! — the fallback `native_decide` uses when the compiler declines, so the integration suite's
//! compiled path never exercises it. Drives Bool decisions through the Lit/Global-prim/App/
//! Lambda(β)/Var(lookup) arms and the `== "true"`/`== "false"` result guards.
use super::*;
use crate::prelude::StandardLibrary;
use crate::term::{Literal, Term};

fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn g(s: &str) -> Term {
    Term::Global(s.to_string())
}
fn v(s: &str) -> Term {
    Term::Var(s.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn nat(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn lam(p: &str, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(g("Nat")),
        body: Box::new(body),
    }
}
fn cmp(op: &str, a: Term, b: Term) -> Term {
    app(app(g(op), a), b)
}

#[test]
fn eval_bool_tree_decides_primitives_beta_and_arithmetic() {
    let c = ctx();
    // primitive comparisons (Lit + Global-prim + App/apply + prim_compute + the Bool guards)
    assert_eq!(eval_bool_tree(&c, &cmp("le", nat(2), nat(3))), Some(true));
    assert_eq!(eval_bool_tree(&c, &cmp("le", nat(3), nat(2))), Some(false));
    assert_eq!(eval_bool_tree(&c, &cmp("lt", nat(2), nat(2))), Some(false));
    assert_eq!(eval_bool_tree(&c, &cmp("ge", nat(3), nat(3))), Some(true));
    assert_eq!(eval_bool_tree(&c, &cmp("gt", nat(3), nat(2))), Some(true));
    // β-reduction + variable lookup: (λx. le x 3) 2  →  le 2 3  →  true
    assert_eq!(
        eval_bool_tree(&c, &app(lam("x", cmp("le", v("x"), nat(3))), nat(2))),
        Some(true)
    );
    // arithmetic feeding a comparison: le (add 2 3) 5  →  le 5 5  →  true
    let add_2_3 = app(app(g("add"), nat(2)), nat(3));
    assert_eq!(eval_bool_tree(&c, &cmp("le", add_2_3, nat(5))), Some(true));
    // a non-Bool / stuck term declines (fail-safe None, never a wrong verdict).
    assert_eq!(eval_bool_tree(&c, &g("Nat")), None);
    assert_eq!(eval_bool_tree(&c, &v("unbound")), None);
    // a NON-Bool constructor (Nat's `Zero`) also declines — kills the `c == "false"` guard
    // mutant (which would report a non-Bool constructor as `false`).
    assert_eq!(
        eval_bool_tree(&c, &g("Zero")),
        None,
        "Zero is not a Bool — must decline"
    );
}

#[test]
fn native_compile_bool_decides_the_ground_arithmetic_fragment() {
    // native_compile_bool is public — drive the COMPILED path (compile_int add/sub/mul,
    // compile_bool le/lt/ge/gt + and/or) directly, pinning each op's value so every operator
    // and arm mutant flips a result.
    let b = |t: &Term| native_compile_bool(t);
    assert_eq!(b(&g("true")), Some(true));
    assert_eq!(b(&g("false")), Some(false));
    // comparison boundaries (le/lt/ge/gt + their operators)
    assert_eq!(b(&cmp("le", nat(3), nat(3))), Some(true));
    assert_eq!(b(&cmp("lt", nat(3), nat(3))), Some(false));
    assert_eq!(b(&cmp("ge", nat(3), nat(3))), Some(true));
    assert_eq!(b(&cmp("gt", nat(3), nat(3))), Some(false));
    assert_eq!(b(&cmp("gt", nat(4), nat(3))), Some(true));
    // distinct operands pin lt vs the `_ => gt` fall-through arm (equal operands can't).
    assert_eq!(b(&cmp("lt", nat(2), nat(3))), Some(true));
    assert_eq!(b(&cmp("ge", nat(2), nat(3))), Some(false));
    // arithmetic: pin add/sub/mul EXACTLY via ge∧le (any operator mutant flips one side).
    let pin = |op: &str, x: i64, y: i64, val: i64| {
        let e = cmp(op, nat(x), nat(y));
        assert_eq!(
            b(&cmp("ge", e.clone(), nat(val))),
            Some(true),
            "{op} {x} {y} ≥ {val}"
        );
        assert_eq!(
            b(&cmp("le", e, nat(val))),
            Some(true),
            "{op} {x} {y} ≤ {val}"
        );
    };
    pin("add", 2, 3, 5);
    pin("sub", 5, 2, 3);
    pin("mul", 2, 3, 6);
    // and/or: T&&F = F, F||T = T (kills `&& -> ||` and `|| -> &&`).
    let t = cmp("le", nat(2), nat(3));
    let f = cmp("le", nat(4), nat(3));
    assert_eq!(
        b(&cmp("and", t.clone(), f.clone())),
        Some(false),
        "T && F = F"
    );
    assert_eq!(
        b(&cmp("and", t.clone(), t.clone())),
        Some(true),
        "T && T = T"
    );
    assert_eq!(
        b(&cmp("or", f.clone(), t.clone())),
        Some(true),
        "F || T = T"
    );
    assert_eq!(
        b(&cmp("or", f.clone(), f.clone())),
        Some(false),
        "F || F = F"
    );
    // outside the compilable fragment → None (interpreter fallback).
    assert_eq!(b(&g("Nat")), None);
    assert_eq!(b(&v("x")), None);
}

#[test]
fn native_compile_decide_runs_the_recursor_backend() {
    // native_compile_decide is public — drive the recursor-aware Compiler (Var/Lit/Global/
    // Lambda/App via clookup/capply) with β-redexes, arithmetic, and a higher-order redex.
    let c = ctx();
    let d = |t: &Term| native_compile_decide(&c, t);
    // (λx:Nat. le x 3) 2 → le 2 3 → true   (Var/Lambda/App/lookup)
    assert_eq!(
        d(&app(lam("x", cmp("le", v("x"), nat(3))), nat(2))),
        Some(true)
    );
    assert_eq!(
        d(&app(lam("x", cmp("le", v("x"), nat(3))), nat(5))),
        Some(false)
    );
    // Lit + Global-prim + arithmetic through the Compiler
    assert_eq!(d(&cmp("le", nat(2), nat(3))), Some(true));
    assert_eq!(
        d(&cmp("le", app(app(g("add"), nat(2)), nat(3)), nat(5))),
        Some(true)
    );
    // higher-order: (λf. λx. f x) (λy. le y 3) 2 → le 2 3 → true
    let apply_fn = lam("f", lam("x", app(v("f"), v("x"))));
    let le3 = lam("y", cmp("le", v("y"), nat(3)));
    assert_eq!(d(&app(app(apply_fn, le3), nat(2))), Some(true));
    // native_compile_decide declines a non-Bool constructor — kills the `c == "false"` guard.
    assert_eq!(d(&g("Zero")), None, "Zero is not a Bool decision");
}

#[test]
fn eval_bool_dispatches_compiled_then_falls_back() {
    // eval_bool (the public dispatcher) tries the compiled path, then the tree-walker — exercise
    // it directly (kills `eval_bool -> None/Some(true)/Some(false)`).
    let c = ctx();
    assert_eq!(eval_bool(&c, &cmp("le", nat(2), nat(3))), Some(true));
    assert_eq!(eval_bool(&c, &cmp("gt", nat(2), nat(3))), Some(false));
    assert_eq!(eval_bool(&c, &g("Nat")), None, "a non-decision declines");
}

#[test]
fn compile_int_reads_a_small_valued_bigint_literal() {
    // A `Literal::BigInt` whose value fits i64 only arises hand-constructed (`int_lit` makes an
    // `Int` for small magnitudes), but `compile_int` MUST still read it — else a compiled
    // comparison that happens to carry one silently declines. Kills the "delete the
    // Term::Lit(Literal::BigInt) arm in compile_int" mutant.
    use logicaffeine_base::BigInt;
    let big5 = Term::Lit(Literal::BigInt(BigInt::from_u64(5)));
    assert_eq!(
        native_compile_bool(&cmp("le", big5.clone(), nat(5))),
        Some(true)
    );
    assert_eq!(native_compile_bool(&cmp("lt", big5, nat(5))), Some(false));
}

#[test]
fn count_type_params_counts_leading_sort_domains() {
    use crate::term::Universe;
    let sort0 = || Term::Sort(Universe::Type(0));
    let pi = |pt: Term, bt: Term| Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(pt),
        body_type: Box::new(bt),
    };
    // Π(A:Type0). Π(B:Type0). Π(x:Nat). Nat — 2 leading Sort-domain params, stops at x:Nat.
    assert_eq!(
        count_type_params(&pi(sort0(), pi(sort0(), pi(g("Nat"), g("Nat"))))),
        2
    );
    assert_eq!(count_type_params(&pi(sort0(), pi(g("Nat"), g("Nat")))), 1);
    assert_eq!(
        count_type_params(&pi(g("Nat"), g("Nat"))),
        0,
        "a value domain is not a param"
    );
    assert_eq!(count_type_params(&g("Nat")), 0, "no Π ⇒ 0");
}
