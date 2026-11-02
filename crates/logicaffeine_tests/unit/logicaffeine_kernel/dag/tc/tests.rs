use super::*;

/// Two proven equalities `a ≡ b` and `b ≡ c` never answer `a ≡ c`: the cache is not closed
/// under transitivity, which is what the Arena's non-transitivity exploits need.
#[test]
fn proven_equalities_are_not_closed_transitively() {
    let mut k = Kernel::new();
    k.begin_decl(Rc::from(Vec::new()), Safety::Safe);
    let prop = k.s.prop();
    let (a, b, c) = (
        k.push_local(prop, None),
        k.push_local(prop, None),
        k.push_local(prop, None),
    );
    k.eqv.insert(a, b);
    k.eqv.insert(b, c);
    assert!(k.eqv_is_equiv(a, b) && k.eqv_is_equiv(b, c));
    assert!(!k.eqv_is_equiv(a, c), "a ≡ c was never proven");
    let f = k.push_local(prop, None);
    let (fa, fb) = (k.s.app(f, a), k.s.app(f, b));
    assert!(k.eqv_is_equiv(fa, fb), "congruence: f a ≡ f b from a ≡ b");
    let fc = k.s.app(f, c);
    assert!(
        !k.eqv_is_equiv(fa, fc),
        "congruence never smuggles in transitivity"
    );
    k.end_decl();
}

/// A `Nat` literal reads as the constructor it names only while that constructor is `Nat`'s. With
/// `Succ : Nat → Nat → Nat`, `match 1 …` in LOGOS's theory is stuck, never the second case applied
/// to the literal below, and `0` is still `Zero`.
#[test]
fn a_literal_reads_only_as_the_constructors_its_family_has() {
    use crate::term::{Literal, Term, Universe};
    let g = |n: &str| Term::Global(n.to_string());
    let pi = |p: &str, a: Term, b: Term| Term::Pi {
        param: p.to_string(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    };
    let lam = |p: &str, a: Term, b: Term| Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(a),
        body: Box::new(b),
    };
    let mut ctx = crate::Context::new();
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("Zero", "Nat", g("Nat"));
    ctx.add_constructor(
        "Succ",
        "Nat",
        pi("a", g("Nat"), pi("b", g("Nat"), g("Nat"))),
    );
    ctx.add_inductive("Bool", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("false", "Bool", g("Bool"));
    ctx.add_constructor("true", "Bool", g("Bool"));
    let (mut k, replayed) = crate::dag::logos_env::replay(&ctx);
    assert!(
        replayed.iter().all(|r| r.verdict.is_ok()),
        "the core admits the setup"
    );
    let literal = |n: i64| Term::Lit(Literal::Nat(crate::BigInt::from_i64(n)));
    let on = |n: i64| Term::Match {
        discriminant: Box::new(literal(n)),
        motive: Box::new(lam("n", g("Nat"), g("Bool"))),
        cases: vec![
            g("true"),
            lam("a", g("Nat"), lam("b", g("Nat"), g("false"))),
        ],
    };
    k.begin_decl(Rc::from(Vec::new()), Safety::Safe);
    let one = crate::dag::logos::lower(&mut k.s, &on(1)).expect("lowers");
    let reduced = k.whnf(one).expect("reduces");
    assert!(
        matches!(k.s.node(reduced), Node::Match(..)),
        "`match 1` is stuck where `Succ` is not `Nat`'s successor"
    );
    let zero = crate::dag::logos::lower(&mut k.s, &on(0)).expect("lowers");
    let reduced = k.whnf(zero).expect("reduces");
    let true_name = k.s.names.str(NameId::ANON, "true");
    let tru = k.s.cnst(true_name, LevelsId::EMPTY);
    assert_eq!(reduced, tru, "`match 0` takes the `Zero` case");
    k.end_decl();
}

/// Inferring only, a LOGOS `match` reads its type off its motive applied to the discriminant, as
/// the indexed `match` does: the motive's domain is held to the discriminant's type where the
/// `match` is checked, not where the type of a reduct of a checked term is read.
#[test]
fn inferring_only_reads_a_match_type_off_its_motive() {
    use crate::term::{Term, Universe};
    let g = |n: &str| Term::Global(n.to_string());
    let mut ctx = crate::Context::new();
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("Zero", "Nat", g("Nat"));
    ctx.add_inductive("Bool", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("false", "Bool", g("Bool"));
    let (mut k, replayed) = crate::dag::logos_env::replay(&ctx);
    assert!(replayed.iter().all(|r| r.verdict.is_ok()));
    let mismatched = Term::Match {
        discriminant: Box::new(g("Zero")),
        motive: Box::new(Term::Lambda {
            param: "b".to_string(),
            param_type: Box::new(g("Bool")),
            body: Box::new(g("Nat")),
        }),
        cases: vec![g("Zero")],
    };
    k.begin_decl(Rc::from(Vec::new()), Safety::Safe);
    let e = crate::dag::logos::lower(&mut k.s, &mismatched).expect("lowers");
    assert!(
        k.infer(e, false).is_err(),
        "checked, the motive's domain `Bool` is not the discriminant's type `Nat`"
    );
    let inferred = k.infer(e, true).expect("inferring only reads the motive");
    let inferred = k.whnf(inferred).expect("reduces");
    let nat_name = k.s.names.str(NameId::ANON, "Nat");
    let nat = k.s.cnst(nat_name, LevelsId::EMPTY);
    assert_eq!(inferred, nat);
    k.end_decl();
}

/// Lean content in LOGOS's theory computes its validated primitives natively: `Nat.beq`, named as
/// the Lean importer names it in a LOGOS context and checked by its equations, decides
/// `beq 1000000 1000001` as `false` with no δ-unfolding.
#[test]
fn a_validated_lean_primitive_computes_natively_in_logos_theory() {
    use crate::interface::lean::reference::kernel_name_of_components as lean;
    use crate::term::{Literal, Term, Universe};
    let g = |n: &str| Term::Global(n.to_string());
    let app = |f: Term, a: Term| Term::App(Box::new(f), Box::new(a));
    let lam = |p: &str, a: Term, b: Term| Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(a),
        body: Box::new(b),
    };
    let pi = |a: Term, b: Term| Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    };
    let (bool_, tru, fal, beq) = (
        lean(&["Bool"]),
        lean(&["Bool", "true"]),
        lean(&["Bool", "false"]),
        lean(&["Nat", "beq"]),
    );
    let mut ctx = crate::Context::new();
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("Zero", "Nat", g("Nat"));
    ctx.add_constructor("Succ", "Nat", pi(g("Nat"), g("Nat")));
    ctx.add_inductive(&bool_, Term::Sort(Universe::Type(0)));
    ctx.add_constructor(&fal, &bool_, g(&bool_));
    ctx.add_constructor(&tru, &bool_, g(&bool_));
    let on = |d: Term, motive_codomain: Term, zero: Term, succ: Term| Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(lam("_", g("Nat"), motive_codomain)),
        cases: vec![zero, succ],
    };
    let body = Term::Fix {
        name: "rec".to_string(),
        body: Box::new(lam(
            "a",
            g("Nat"),
            lam(
                "b",
                g("Nat"),
                on(
                    Term::Var("a".to_string()),
                    g(&bool_),
                    on(
                        Term::Var("b".to_string()),
                        g(&bool_),
                        g(&tru),
                        lam("q", g("Nat"), g(&fal)),
                    ),
                    lam(
                        "p",
                        g("Nat"),
                        on(
                            Term::Var("b".to_string()),
                            g(&bool_),
                            g(&fal),
                            lam(
                                "q",
                                g("Nat"),
                                app(
                                    app(Term::Var("rec".to_string()), Term::Var("p".to_string())),
                                    Term::Var("q".to_string()),
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        )),
    };
    ctx.declare(&beq, pi(g("Nat"), pi(g("Nat"), g(&bool_))), body)
        .expect("the Term kernel admits Lean's `Nat.beq`");
    let (mut k, replayed) = crate::dag::logos_env::replay(&ctx);
    assert!(
        replayed.iter().all(|r| r.verdict.is_ok()),
        "the core admits the setup"
    );
    assert!(
        k.validated_primitives.contains(&k.wk.nat_beq),
        "the importer's `Nat.beq` is the primitive, checked by its equations"
    );
    let literal = |n: i64| Term::Lit(Literal::Nat(crate::BigInt::from_i64(n)));
    k.begin_decl(Rc::from(Vec::new()), Safety::Safe);
    let call = crate::dag::logos::lower(
        &mut k.s,
        &app(app(g(&beq), literal(1_000_000)), literal(1_000_001)),
    )
    .expect("lowers");
    let before = k.unfold_steps();
    let reduced = k.whnf(call).expect("reduces");
    let false_name = k.s.names.str(NameId::ANON, &fal);
    assert_eq!(reduced, k.s.cnst(false_name, LevelsId::EMPTY));
    assert_eq!(
        k.unfold_steps(),
        before,
        "computed natively, with no δ-unfolding of `Nat.beq`"
    );
    k.end_decl();
}

#[path = "../../lean_scale_export.rs"]
mod lean_scale_export;

/// How long a test over a `lean_scale` import may run before it fails: several times what the
/// import, the replay and the checks take in a debug build.
const LEAN_SCALE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(600);

/// `f` on a thread with the core's stack (1 GiB, which its recursion bound declines inside), failed
/// rather than waited on past `deadline`: a native computation turned into an unbounded unfolding
/// fails the test instead of hanging it or overflowing the stack.
fn within(deadline: std::time::Duration, f: impl FnOnce() + Send + 'static) {
    let (sent, received) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            f();
            let _ = sent.send(());
        })
        .expect("spawns");
    match received.recv_timeout(deadline) {
        Ok(()) => {}
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("did not finish within {deadline:?}")
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            panic!("failed (its message is above)")
        }
    }
}

/// Lean's own `Nat.div` and `Nat.mod` (`lean_scale/LG_Div`, Lean 4.14.0) in LOGOS's theory: the
/// unary programs `Nat.div._unary` and `Nat.modCore._unary`, which the `Term` kernel's evaluator
/// certifies as the quotient and the remainder by their recurrence, compute natively on a pair of
/// literals with no δ-unfolding of their well-founded recursion, and conversion decides division
/// at 2^128 and by zero by computing it.
#[test]
fn a_certified_division_program_computes_natively_in_logos_theory() {
    use crate::term::{Literal, Term};
    use lean_scale_export::{environment_without, fixture, spelled};
    within(LEAN_SCALE_DEADLINE, || {
        let (ctx, _) =
            crate::interface::lean::import(&environment_without(&fixture("LG_Div"), "LG_DV."))
                .expect("Lean's own division imports");
        let (mut k, replayed) = crate::dag::logos_env::replay(&ctx);
        let refused: Vec<String> = replayed
            .iter()
            .filter_map(|r| r.verdict.as_ref().err().map(|e| format!("{}: {e}", r.name)))
            .collect();
        assert!(
            refused.is_empty(),
            "the core admits Lean's division: {refused:#?}"
        );
        let nat = |n: &str| Term::Lit(Literal::Nat(crate::BigInt::parse_decimal(n).unwrap()));
        let call = |f: &str, x: &str, y: &str| {
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global(spelled(f))),
                    Box::new(nat(x)),
                )),
                Box::new(nat(y)),
            )
        };
        k.begin_decl(Rc::from(Vec::new()), Safety::Safe);
        let w = &k.wk;
        let binary = [
            w.nat_add,
            w.nat_sub,
            w.nat_mul,
            w.nat_pow,
            w.nat_gcd,
            w.nat_mod,
            w.nat_div,
            w.nat_beq,
            w.nat_ble,
            w.nat_land,
            w.nat_lor,
            w.nat_xor,
            w.nat_shift_left,
            w.nat_shift_right,
        ];
        let validated: Vec<NameId> = binary
            .into_iter()
            .filter(|op| k.validated_primitives.contains(op))
            .collect();
        assert!(
            !validated.is_empty(),
            "the import's `Nat` operations are validated primitives"
        );
        for op in validated {
            let shown = k.s.names.display(op);
            let operands = [k.s.nat_lit(Nat::from_u64(7)), k.s.nat_lit(Nat::from_u64(3))];
            let head = k.s.cnst(op, LevelsId::EMPTY);
            let call = k.s.mk_app_n(head, &operands);
            let before = k.unfold_steps();
            let value = k.whnf(call).expect("reduces");
            assert!(
                matches!(k.s.node(value), Node::NatLit(_) | Node::Const(..)),
                "{shown} 7 3 is a value"
            );
            assert_eq!(
                k.unfold_steps(),
                before,
                "{shown} 7 3 is computed natively, with no δ-unfolding"
            );
        }
        for (wrapper, program, x, y, answer) in [
            ("Nat.div", "Nat.div._unary", "17", "5", "3"),
            ("Nat.modCore", "Nat.modCore._unary", "17", "5", "2"),
        ] {
            let applied = crate::dag::logos::lower(&mut k.s, &call(wrapper, x, y)).expect("lowers");
            let unfolded = k.unfold_definition(applied).expect("the wrapper unfolds");
            let unary = k.whnf_core(unfolded, false).expect("β");
            assert_eq!(
                k.s.const_head(unary).map(|(n, _)| k.s.names.display(n)),
                Some(spelled(program)),
                "{wrapper} {x} {y} is {program} on a pair"
            );
            let before = k.unfold_steps();
            let reduced = k.whnf(unary).expect("reduces");
            let expected = crate::dag::logos::lower(&mut k.s, &nat(answer)).expect("lowers");
            assert_eq!(reduced, expected, "{program} ⟨{x}, {y}⟩ = {answer}");
            assert_eq!(
                k.unfold_steps(),
                before,
                "{program} is computed natively, with no δ-unfolding"
            );
        }
        let two_64 = "18446744073709551616";
        let two_128 = "340282366920938463463374607431768211456";
        for (f, x, y, answer, wrong) in [
            ("Nat.div", two_128, two_64, two_64, "18446744073709551617"),
            (
                "Nat.mod",
                "340282366920938463463374607431768223801",
                "1000000007",
                "279644622",
                "279644623",
            ),
            ("Nat.div", "17", "0", "0", "1"),
            ("Nat.mod", "17", "0", "17", "16"),
        ] {
            let lhs = crate::dag::logos::lower(&mut k.s, &call(f, x, y)).expect("lowers");
            let right = crate::dag::logos::lower(&mut k.s, &nat(answer)).expect("lowers");
            let other = crate::dag::logos::lower(&mut k.s, &nat(wrong)).expect("lowers");
            assert!(
                k.is_def_eq(lhs, right).expect("decides"),
                "{f} {x} {y} ≡ {answer}"
            );
            assert!(
                !k.is_def_eq(lhs, other).expect("decides"),
                "{f} {x} {y} ≢ {wrong}"
            );
        }
        k.end_decl();
    })
}

/// Programs shaped like Lean's `Nat.modCore` (`lean_scale/LG_DivImpostor`) are never computed as
/// `%` in LOGOS's theory: `badMod`, which answers `x + 1` where the modulus answers `x`, takes its own
/// value at `5 5`, and `axMod`, which subtracts through an axiom, stays stuck at it.
#[test]
fn a_program_shaped_like_the_modulus_is_never_computed_as_one_in_logos_theory() {
    use crate::term::{Literal, Term};
    use lean_scale_export::{fixture, spelled};
    within(LEAN_SCALE_DEADLINE, || {
        let (ctx, _) = crate::interface::lean::import(&fixture("LG_DivImpostor"))
            .expect("Lean accepts both definitions");
        let (mut k, replayed) = crate::dag::logos_env::replay(&ctx);
        assert!(
            replayed.iter().all(|r| r.verdict.is_ok()),
            "the core admits the impostors"
        );
        let nat = |n: u64| Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)));
        let call = |f: &str| {
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global(spelled(f))),
                    Box::new(nat(5)),
                )),
                Box::new(nat(5)),
            )
        };
        k.begin_decl(Rc::from(Vec::new()), Safety::Safe);
        let bad = crate::dag::logos::lower(&mut k.s, &call("LG_DI.badMod")).expect("lowers");
        let (zero, one) = (
            crate::dag::logos::lower(&mut k.s, &nat(0)).expect("lowers"),
            crate::dag::logos::lower(&mut k.s, &nat(1)).expect("lowers"),
        );
        assert!(
            k.is_def_eq(bad, one).expect("decides"),
            "badMod 5 5 is badMod 0 5 = 1"
        );
        assert!(
            !k.is_def_eq(bad, zero).expect("decides"),
            "badMod 5 5 is never computed as 5 % 5"
        );
        let ax = crate::dag::logos::lower(&mut k.s, &call("LG_DI.axMod")).expect("lowers");
        assert!(
            !k.is_def_eq(ax, zero).expect("decides"),
            "axMod 5 5 is stuck at the axiom, never computed as 5 % 5"
        );
        k.end_decl();
    })
}

/// A LOGOS kernel computes by the certificates of the context it follows: a simp rule registered in
/// a later context withdraws the evaluator's acceleration there, and the kernel that follows that
/// context computes nothing natively from a certificate it read before.
#[test]
fn a_kernel_that_follows_a_context_reads_its_certificates_again() {
    use crate::term::{Literal, Term, Universe};
    use lean_scale_export::{environment_without, fixture, spelled};
    within(LEAN_SCALE_DEADLINE, || {
        let (ctx, _) =
            crate::interface::lean::import(&environment_without(&fixture("LG_Div"), "LG_DV."))
                .expect("Lean's own division imports");
        let (mut k, _) = crate::dag::logos_env::replay(&ctx);
        let g = |n: &str| Term::Global(n.to_string());
        let app = |f: Term, a: Term| Term::App(Box::new(f), Box::new(a));
        let pi = |p: &str, a: Term, b: Term| Term::Pi {
            param: p.to_string(),
            param_type: Box::new(a),
            body_type: Box::new(b),
        };
        let nat = |n: u64| Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)));
        let div = |x: u64, y: u64| app(app(g(&spelled("Nat.div")), nat(x)), nat(y));
        k.begin_decl(Rc::from(Vec::new()), Safety::Safe);
        let applied = crate::dag::logos::lower(&mut k.s, &div(17, 5)).expect("lowers");
        let unfolded = k.unfold_definition(applied).expect("`Nat.div` unfolds");
        let unary = k.whnf_core(unfolded, false).expect("β");
        assert!(
            k.reduce_certified_program(unary)
                .expect("reduces")
                .is_some(),
            "`Nat.div._unary` computes natively in the import's context"
        );
        let mut later: crate::Context = (*ctx).clone();
        later.add_declaration(
            "Eq",
            pi(
                "A",
                Term::Sort(Universe::Type(0)),
                pi(
                    "_",
                    Term::Var("A".into()),
                    pi("_", Term::Var("A".into()), Term::Sort(Universe::Prop)),
                ),
            ),
        );
        later.add_declaration(
            "div_one",
            app(
                app(app(g("Eq"), g("Nat")), div(1, 1)),
                app(g("Succ"), g("Zero")),
            ),
        );
        later
            .add_simp_lemma("div_one")
            .expect("`Nat.div 1 1 = Succ Zero` is a rewrite rule");
        k.follow(&later);
        assert!(
            k.reduce_certified_program(unary)
                .expect("reduces")
                .is_none(),
            "the followed context's simp rule withdraws the acceleration"
        );
        k.end_decl();
    })
}
