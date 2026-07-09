//! Native RECURSOR-aware compiled decisions (N) — the general backend. `native_compile_decide`
//! compiles the FULL computational fragment (recursion via `fix`, pattern matching via `match`,
//! δ-unfolding of definitions) to native closures, so a RECURSIVE Bool decision runs as closure
//! calls with no `Term`-tree walking. The contract is that its verdict is IDENTICAL to the
//! tree-walking interpreter (`eval_bool_tree`) and to the kernel's own `normalize` — the
//! soundness backing the `reduceBool` hook that consumes it. These tests pin that contract on
//! genuinely recursive decisions (built-in `decEqNat`, user-defined `even`/`le_nat`), plus the
//! end-to-end `native_decide` proof and the fail-safe declines.

use logicaffeine_kernel::interface::Repl;
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    double_check, eval_bool_tree, infer_type, is_subtype, native_compile_decide, native_decide,
    normalize, Context, DoubleCheck, Literal, Term,
};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn nat(n: usize) -> Term {
    let mut t = g("Zero");
    for _ in 0..n {
        t = app(g("Succ"), t);
    }
    t
}
fn eqn(a: Term, b: Term) -> Term {
    apps(g("Eq"), &[g("Nat"), a, b])
}
fn normalize_bool(ctx: &Context, t: &Term) -> Option<bool> {
    match normalize(ctx, t) {
        Term::Global(n) if n == "true" => Some(true),
        Term::Global(n) if n == "false" => Some(false),
        _ => None,
    }
}

/// The core contract: the compiled decider, the tree-walking interpreter, and `normalize` all
/// agree — and the verdict is what we expect. This is the trust check for a compiled decision.
#[track_caller]
fn assert_all_agree(ctx: &Context, t: &Term, expected: Option<bool>) {
    let compiled = native_compile_decide(ctx, t);
    let tree = eval_bool_tree(ctx, t);
    let norm = normalize_bool(ctx, t);
    assert_eq!(compiled, tree, "compiled must equal the tree-walker for {t}");
    assert_eq!(compiled, norm, "compiled must equal normalize for {t}");
    assert_eq!(compiled, expected, "compiled verdict for {t}");
}

// ---------------------------------------------------------------------------
// Built-in recursive decision procedure: decEqNat (structural recursion on Nat).
// ---------------------------------------------------------------------------

#[test]
fn compiled_dec_eq_nat_matches_tree_and_normalize() {
    // `decide (Eq Nat a b) (decEqNat a b)` recurses structurally on both numerals. The compiled
    // path must agree with the tree-walker and normalize for every pair — the whole grid.
    let ctx = std_ctx();
    for a in 0..8usize {
        for b in 0..8usize {
            let dec = apps(g("decide"), &[eqn(nat(a), nat(b)), apps(g("decEqNat"), &[nat(a), nat(b)])]);
            assert_all_agree(&ctx, &dec, Some(a == b));
        }
    }
}

// ---------------------------------------------------------------------------
// User-defined recursive Bool functions, defined through the REPL (real surface
// definitions, self-recursion auto-bound to `fix`), then decided by compilation.
// ---------------------------------------------------------------------------

fn even_ctx() -> Context {
    let mut repl = Repl::new();
    // even n = true iff n is even — self-recursive on the DIRECT child `m` (via a boolNot
    // helper), which the kernel's structural termination guard accepts. Also exercises a
    // compiled decision delegating to another compiled definition (`boolNot`).
    repl.execute(
        "Definition boolNot : Bool -> Bool := fun b : Bool => \
         match b with | true => false | false => true end.",
    )
    .expect("define boolNot");
    repl.execute(
        "Definition even : Nat -> Bool := fun n : Nat => \
         match n with | Zero => true | Succ m => boolNot (even m) end.",
    )
    .expect("define even");
    repl.context().clone()
}

#[test]
fn compiled_even_matches_tree_and_normalize() {
    let ctx = even_ctx();
    for n in 0..14usize {
        assert_all_agree(&ctx, &app(g("even"), nat(n)), Some(n % 2 == 0));
    }
}

fn le_nat_ctx() -> Context {
    let mut repl = Repl::new();
    repl.execute(
        "Definition le_nat : Nat -> Nat -> Bool := fun a : Nat => fun b : Nat => \
         match a with | Zero => true | Succ a2 => \
           match b with | Zero => false | Succ b2 => le_nat a2 b2 end end.",
    )
    .expect("define le_nat");
    repl.context().clone()
}

#[test]
fn compiled_le_nat_matches_tree_and_normalize() {
    let ctx = le_nat_ctx();
    for a in 0..7usize {
        for b in 0..7usize {
            assert_all_agree(&ctx, &apps(g("le_nat"), &[nat(a), nat(b)]), Some(a <= b));
        }
    }
}

#[test]
fn compiled_decision_delegating_across_definitions_agrees() {
    // A definition that calls ANOTHER definition (non-recursive delegation) — exercises the
    // per-definition compiled-body slots resolving one def from inside another.
    let mut repl = Repl::new();
    repl.execute(
        "Definition boolNot : Bool -> Bool := fun b : Bool => \
         match b with | true => false | false => true end.",
    )
    .expect("define boolNot");
    repl.execute(
        "Definition even : Nat -> Bool := fun n : Nat => \
         match n with | Zero => true | Succ m => boolNot (even m) end.",
    )
    .expect("define even");
    repl.execute("Definition evenSucc : Nat -> Bool := fun n : Nat => even (Succ n).")
        .expect("define evenSucc");
    let ctx = repl.context().clone();
    for n in 0..10usize {
        assert_all_agree(&ctx, &app(g("evenSucc"), nat(n)), Some((n + 1) % 2 == 0));
    }
}

// ---------------------------------------------------------------------------
// End to end: native_decide on a RECURSOR decision, both-kernel certified.
// ---------------------------------------------------------------------------

#[test]
fn native_decide_proves_a_recursive_decision_two_kernel() {
    // `decEqNat 6 6` is a recursive decision; native_decide (now backed by the compiled path)
    // produces a proof of `Eq Nat 6 6` that BOTH kernels certify.
    let ctx = std_ctx();
    let prop = eqn(nat(6), nat(6));
    let inst = apps(g("decEqNat"), &[nat(6), nat(6)]);
    let proof = native_decide(&ctx, &prop, &inst).expect("native_decide proves 6 = 6");
    let ty = infer_type(&ctx, &proof).expect("proof type-checks via the reduceBool hook");
    assert!(is_subtype(&ctx, &ty, &prop), "proof : Eq Nat 6 6");
    match double_check(&ctx, &proof) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify the native_decide proof, got {other:?}"),
    }
}

#[test]
fn native_decide_declines_a_false_recursive_decision() {
    let ctx = std_ctx();
    let prop = eqn(nat(6), nat(7));
    let inst = apps(g("decEqNat"), &[nat(6), nat(7)]);
    assert!(native_decide(&ctx, &prop, &inst).is_none(), "must decline 6 = 7");
}

// ---------------------------------------------------------------------------
// Fail-safe: the compiler declines (None) outside the decidable fragment, so
// eval_bool falls back to the interpreter — never a wrong answer.
// ---------------------------------------------------------------------------

#[test]
fn compiler_declines_non_bool_and_open_terms() {
    let ctx = even_ctx();
    // A non-Bool head (`Zero`) is not a decision.
    assert_eq!(native_compile_decide(&ctx, &g("Zero")), None, "Zero is not a Bool decision");
    // An open term (`even n` with `n` free) cannot be decided.
    assert_eq!(
        native_compile_decide(&ctx, &app(g("even"), Term::Var("n".to_string()))),
        None,
        "a free variable is not decidable"
    );
    // An opaque global (an inductive type name) declines.
    assert_eq!(native_compile_decide(&ctx, &g("Bool")), None, "an inductive type is not a decision");
}

#[test]
fn compiled_and_tree_agree_on_a_large_recursive_decision() {
    // A deeper recursion where compiling to closures pays off — the two engines must still
    // give the identical verdict (and it is what we expect).
    let ctx = even_ctx();
    assert_all_agree(&ctx, &app(g("even"), nat(40)), Some(true));
    assert_all_agree(&ctx, &app(g("even"), nat(41)), Some(false));
}

// ---------------------------------------------------------------------------
// The compiled path still handles the ground arithmetic fragment (via a ctx),
// agreeing with the tree-walker and normalize — a superset of native_compile_bool.
// ---------------------------------------------------------------------------

#[test]
fn compiled_decide_handles_ground_arithmetic_too() {
    let ctx = std_ctx();
    let i = |n: i64| Term::Lit(Literal::Int(n));
    let cases: &[(Term, Option<bool>)] = &[
        (apps(g("le"), &[apps(g("add"), &[i(2), i(3)]), i(5)]), Some(true)),
        (apps(g("lt"), &[apps(g("mul"), &[i(4), i(5)]), i(20)]), Some(false)),
        (apps(g("ge"), &[apps(g("sub"), &[i(10), i(3)]), i(7)]), Some(true)),
    ];
    for (t, expected) in cases {
        assert_all_agree(&ctx, t, *expected);
    }
}
