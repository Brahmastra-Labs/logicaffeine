//! Native closure-compiled decisions (N) — a ground Bool decision is compiled ONCE into a
//! tree of rustc-compiled native closures and executed with no Term-walking at run time.
//! Its verdict is IDENTICAL to the `eval_bool` interpreter and `normalize` (differential),
//! so the `ofReduceBool` certificate stays sound; the compiled form is the fast path.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{eval_bool, native_compile_bool, normalize, Context, Literal, Term};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn i(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn bin(op: &str, x: Term, y: Term) -> Term {
    app(app(g(op), x), y)
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn normalize_bool(ctx: &Context, t: &Term) -> Option<bool> {
    match normalize(ctx, t) {
        Term::Global(n) if n == "true" => Some(true),
        Term::Global(n) if n == "false" => Some(false),
        _ => None,
    }
}

#[test]
fn compiles_and_runs_a_ground_decision_natively() {
    // `le (add 2 3) 5`  →  compiled to native closures  →  true.
    let d = bin("le", bin("add", i(2), i(3)), i(5));
    assert_eq!(native_compile_bool(&d), Some(true), "le (2+3) 5 = true, natively compiled");

    // `lt (mul 4 5) 20`  →  false.
    let d2 = bin("lt", bin("mul", i(4), i(5)), i(20));
    assert_eq!(native_compile_bool(&d2), Some(false), "lt (4*5) 20 = false");
}

#[test]
fn native_compilation_agrees_with_the_interpreter_exhaustively() {
    // The native-compiled verdict must MATCH `eval_bool` and `normalize` for every case —
    // the soundness contract that lets `native_decide` trust it.
    let ctx = std_ctx();
    let ops = ["le", "lt", "ge", "gt"];
    for op in ops {
        for a in -4i64..5 {
            for b in -4i64..5 {
                // (a+1) OP (b*2 - 1), exercising nested add/sub/mul.
                let lhs = bin("add", i(a), i(1));
                let rhs = bin("sub", bin("mul", i(b), i(2)), i(1));
                let d = bin(op, lhs, rhs);
                let native = native_compile_bool(&d);
                assert_eq!(native, eval_bool(&ctx, &d), "native == eval_bool for {op} {a} {b}");
                assert_eq!(native, normalize_bool(&ctx, &d), "native == normalize for {op} {a} {b}");
            }
        }
    }
}

#[test]
fn boolean_connectives_compile() {
    // `and (le 1 2) (or (gt 3 9) (le 0 0))`  →  true.
    let d = bin(
        "and",
        bin("le", i(1), i(2)),
        bin("or", bin("gt", i(3), i(9)), bin("le", i(0), i(0))),
    );
    assert_eq!(native_compile_bool(&d), Some(true), "compiled boolean connectives");
}

#[test]
fn declines_outside_the_compilable_fragment() {
    // A non-ground / non-arithmetic term is declined (`None`) so the caller falls back to the
    // interpreter — never a wrong answer.
    assert_eq!(native_compile_bool(&g("Zero")), None, "a non-Bool term is declined");
    assert_eq!(
        native_compile_bool(&bin("le", g("x"), i(0))),
        None,
        "a free variable is not in the compilable fragment"
    );
}
