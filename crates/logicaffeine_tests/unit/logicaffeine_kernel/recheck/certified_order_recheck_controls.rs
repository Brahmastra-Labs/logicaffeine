use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn app(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
fn lam(t: Db, b: Db) -> Db {
    Db::Lam(DbBox::new(t), DbBox::new(b))
}
fn pi(t: Db, b: Db) -> Db {
    Db::Pi(DbBox::new(t), DbBox::new(b))
}
fn lit(n: u64) -> Db {
    Db::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}
fn order(a: Db, b: Db) -> Db {
    app(app(g("order.program"), a), b)
}
fn signature() -> Db {
    pi(g("Nat"), pi(g("Nat"), g("Choice")))
}
fn program(recur: bool) -> Db {
    // walk,m,n,pm,pn: recursion under five binders must use index four.
    let call = if recur {
        app(app(Db::Var(4), Db::Var(1)), Db::Var(0))
    } else {
        g("Choice.no")
    };
    let inner = Db::Match {
        disc: DbBox::new(Db::Var(1)),
        motive: DbBox::new(lam(g("Nat"), g("Choice"))),
        cases: vec![g("Choice.no"), lam(g("Nat"), call)],
    };
    let outer = Db::Match {
        disc: DbBox::new(Db::Var(1)),
        motive: DbBox::new(lam(g("Nat"), g("Choice"))),
        cases: vec![g("Choice.yes"), lam(g("Nat"), inner)],
    };
    Db::Fix(FixBody::new(lam(g("Nat"), lam(g("Nat"), outer))))
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", Term::Global("Nat".into()));
    c.add_constructor(
        "Succ",
        "Nat",
        from_db_with_external(&pi(g("Nat"), g("Nat")), 0, &[]),
    );
    c.add_inductive("Choice", Term::Sort(Universe::Type(0)));
    c.add_constructor("Choice.yes", "Choice", Term::Global("Choice".into()));
    c.add_constructor("Choice.no", "Choice", Term::Global("Choice".into()));
    c.declare(
        "order.program",
        from_db_with_external(&signature(), 0, &[]),
        from_db_with_external(&program(true), 0, &[]),
    )
    .unwrap();
    c
}
struct Restore(usize, usize, bool);
impl Drop for Restore {
    fn drop(&mut self) {
        WHNF_DEPTH.with(|x| x.set(self.0));
        WHNF_FUEL.with(|x| x.set(self.1));
        WHNF_EXHAUSTED.with(|x| x.set(self.2));
    }
}
fn observe(c: &Context, t: &Db, fuel: usize) -> (Option<Db>, bool, usize) {
    let _restore = Restore(
        WHNF_DEPTH.with(|x| x.replace(1)),
        WHNF_FUEL.with(|x| x.replace(fuel)),
        WHNF_EXHAUSTED.with(|x| x.replace(false)),
    );
    let answer = try_certified_nat_order_db_in(c, &[], t);
    (
        answer,
        WHNF_EXHAUSTED.with(|x| x.get()),
        WHNF_FUEL.with(|x| x.get()),
    )
}
#[test]
fn independently_certifies_symbolic_equations_for_large_inputs() {
    let c = context();
    for (a, b, want) in [
        (0, 0, true),
        (1_114_113, 4_294_967_296, true),
        (4_294_967_296, 1_114_113, false),
        (u64::MAX, u64::MAX, true),
    ] {
        let (answer, exhausted, left) = observe(&c, &order(lit(a), lit(b)), 100_000);
        assert_eq!(
            answer,
            Some(g(if want { "Choice.yes" } else { "Choice.no" }))
        );
        assert!(!exhausted);
        assert!(left > 0 && left < 100_000);
    }
}
#[test]
fn refuses_an_incorrect_successor_equation() {
    let mut c = context();
    c.declare(
        "order.program",
        from_db_with_external(&signature(), 0, &[]),
        from_db_with_external(&program(false), 0, &[]),
    )
    .unwrap();
    assert_eq!(
        observe(&c, &order(lit(1000000), lit(1000001)), 100_000).0,
        None
    );
}
#[test]
fn changed_dependencies_are_recertified() {
    let mut c = context();
    let ty = from_db_with_external(&signature(), 0, &[]);
    c.declare(
        "dependency",
        ty.clone(),
        from_db_with_external(&program(true), 0, &[]),
    )
    .unwrap();
    c.declare(
        "order.program",
        ty.clone(),
        Term::Global("dependency".into()),
    )
    .unwrap();
    let t = order(lit(1000000), lit(1000001));
    assert_eq!(observe(&c, &t, 100_000).0, Some(g("Choice.yes")));
    let changed = from_db_with_external(&program(false), 0, &[]);
    let kept = c
        .refused_meaning_change_for_test(|c| c.declare("dependency", ty.clone(), changed.clone()))
        .expect("the gate keeps `dependency` under `order.program`");
    assert_eq!(observe(&kept, &t, 100_000).0, Some(g("Choice.yes")));
    c.permit_meaning_changes_for_cache_tests();
    c.declare("dependency", ty, changed).unwrap();
    assert_eq!(observe(&c, &t, 100_000).0, None);
}
#[test]
fn spent_fuel_never_produces_a_certificate_or_resets_the_budget() {
    let c = context();
    let t = order(lit(1000000), lit(1000001));
    for fuel in [0, 1, 8] {
        let (answer, exhausted, left) = observe(&c, &t, fuel);
        assert_eq!(answer, None);
        assert!(exhausted);
        assert_eq!(left, 0);
    }
}
#[test]
fn recurrence_probe_context_disables_self_certification() {
    let c = context().order_recurrence_context();
    assert_eq!(
        observe(&c, &order(lit(1000000), lit(1000001)), 100_000).0,
        None
    );
}
#[test]
fn explicit_universe_work_is_bounded_before_independent_lowering() {
    let check = |annotation: Term| {
        let mut c = context();
        let ty = from_db_with_external(&signature(), 0, &[]);
        let program = Term::Let {
            name: "unused".into(),
            ty: Box::new(annotation),
            value: Box::new(Term::Sort(Universe::Type(0))),
            body: Box::new(from_db_with_external(&program(true), 0, &[])),
        };
        c.declare("order.program", ty, program).unwrap();
        observe(&c, &order(lit(1_000_000), lit(1_000_001)), 100_000).0
    };
    assert_eq!(
        check(Term::Sort(Universe::Succ(Box::new(Universe::Type(0))))),
        Some(g("Choice.yes"))
    );
    let mut deep = Universe::Type(0);
    for _ in 0..=crate::type_checker::MAX_TERM_DEPTH {
        deep = Universe::Succ(Box::new(deep));
    }
    assert_eq!(check(Term::Sort(deep)), None);
    assert_eq!(
        check(Term::Const {
            name: "unused.annotation".into(),
            levels: vec![Universe::Type(0); 4096]
        }),
        None
    );
}

// ------------------------------------------------------------------------------------------------
// A program that matches on BOTH operands at zero (Lean's `Nat.ble`) is certified by the two cases
// `f 0 0 = yes` and `f 0 (n+1) = yes`, never by `f 0 n = yes` for a free `n`: for a right operand
// stuck on a local variable the program is stuck too. The prefix decision answers only from an
// observed right operand, so `Nat.ble 2 (n+2)` meets its own stuck reduct (Lean's
// `Nat.log2_terminates` compares the two) and `Nat.ble 0 n ≡ true` stays refused, as Lean refuses it.
// ------------------------------------------------------------------------------------------------

mod split_zero_case {
    use super::*;
    use crate::prelude::StandardLibrary;

    fn tg(n: &str) -> Term {
        Term::Global(n.into())
    }
    fn tv(n: &str) -> Term {
        Term::Var(n.into())
    }
    fn tapp(f: Term, a: Term) -> Term {
        Term::App(Box::new(f), Box::new(a))
    }
    fn tapps(f: Term, xs: impl IntoIterator<Item = Term>) -> Term {
        xs.into_iter().fold(f, tapp)
    }
    fn tlam(n: &str, ty: Term, body: Term) -> Term {
        Term::Lambda {
            param: n.into(),
            param_type: Box::new(ty),
            body: Box::new(body),
        }
    }
    fn tpi(ty: Term, body: Term) -> Term {
        Term::Pi {
            param: "_".into(),
            param_type: Box::new(ty),
            body_type: Box::new(body),
        }
    }
    fn tnat(n: u64) -> Term {
        Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
    }
    fn succ(t: Term) -> Term {
        tapp(tg("Succ"), t)
    }
    fn to_bool() -> Term {
        tlam("_", tg("Nat"), tg("Bool"))
    }
    fn on(d: Term, zero: Term, successor: Term) -> Term {
        Term::Match {
            discriminant: Box::new(d),
            motive: Box::new(to_bool()),
            cases: vec![zero, successor],
        }
    }
    /// `n` stuck on its own: both of its cases are `true`.
    fn zero_case(n: Term) -> Term {
        on(n, tg("true"), tlam("pn", tg("Nat"), tg("true")))
    }
    /// `ble m n` with Lean's `Nat.ble` case split: `0, _ => true | m+1, 0 => false | m+1, n+1 => ble m n`.
    fn split() -> Term {
        let step = on(
            tv("n"),
            tg("false"),
            tlam("pn", tg("Nat"), tapps(tv("walk"), [tv("pm"), tv("pn")])),
        );
        let body = tlam(
            "m",
            tg("Nat"),
            tlam(
                "n",
                tg("Nat"),
                on(tv("m"), zero_case(tv("n")), tlam("pm", tg("Nat"), step)),
            ),
        );
        Term::Fix {
            name: "walk".into(),
            body: Box::new(body),
        }
    }
    /// The same order with the uniform zero case `0, _ => true`.
    fn uniform() -> Term {
        let step = on(
            tv("n"),
            tg("false"),
            tlam("pn", tg("Nat"), tapps(tv("walk"), [tv("pm"), tv("pn")])),
        );
        let body = tlam(
            "m",
            tg("Nat"),
            tlam(
                "n",
                tg("Nat"),
                on(tv("m"), tg("true"), tlam("pm", tg("Nat"), step)),
            ),
        );
        Term::Fix {
            name: "walk".into(),
            body: Box::new(body),
        }
    }
    fn standard() -> Context {
        let mut c = Context::new();
        StandardLibrary::register(&mut c);
        let signature = tpi(tg("Nat"), tpi(tg("Nat"), tg("Bool")));
        c.declare("order.split", signature.clone(), split())
            .unwrap();
        c.declare("order.uniform", signature, uniform()).unwrap();
        c
    }
    fn call(program: &str, m: Term, n: Term) -> Term {
        tapps(tg(program), [m, n])
    }
    fn eq_bool(a: Term, b: Term) -> Term {
        tapps(tg("Eq"), [tg("Bool"), a, b])
    }
    fn refl_bool(a: Term) -> Term {
        tapps(tg("refl"), [tg("Bool"), a])
    }
    /// `λ(x : Nat). (refl proof : statement)`, the claim checked for a free natural `x`.
    fn for_all_x(proof: Term, statement: Term) -> Term {
        tlam(
            "x",
            tg("Nat"),
            Term::Let {
                name: "claim".into(),
                ty: Box::new(statement),
                value: Box::new(proof),
                body: Box::new(tv("claim")),
            },
        )
    }
    fn observe_open(c: &Context, t: &Term) -> Option<Db> {
        let locals = [Db::Global("Nat".into())];
        let db = to_db(t, &mut vec!["x".to_string()]).expect("lowers under x");
        let _restore = Restore(
            WHNF_DEPTH.with(|x| x.replace(1)),
            WHNF_FUEL.with(|x| x.replace(100_000)),
            WHNF_EXHAUSTED.with(|x| x.replace(false)),
        );
        try_certified_nat_order_db_in(c, &locals, &db)
    }

    #[test]
    fn an_unobserved_right_operand_is_never_answered_by_a_split_zero_case() {
        let c = standard();
        let x = || tv("x");
        assert_eq!(
            observe_open(&c, &call("order.split", tnat(0), x())),
            None,
            "ble 0 x is stuck on x"
        );
        assert_eq!(
            observe_open(&c, &call("order.split", tnat(2), succ(succ(x())))),
            None,
            "ble 2 (x+2) is stuck on x"
        );
        assert_eq!(
            observe_open(&c, &call("order.split", tnat(0), succ(x()))),
            Some(Db::Global("true".into())),
            "an observed successor is the certified case `f 0 (n+1) = yes`"
        );
        assert_eq!(
            observe_open(&c, &call("order.split", tnat(3), tnat(3))),
            Some(Db::Global("true".into()))
        );
        let zero_of_x = tapp(tlam("m", tg("Nat"), tg("Zero")), x());
        assert_eq!(observe_open(&c, &call("order.split", tnat(2), succ(succ(zero_of_x)))), Some(Db::Global("true".into())),
            "a right operand observed to be zero after the left one ran out is the certified case `f 0 0 = yes`");
        assert_eq!(
            observe_open(&c, &call("order.split", tnat(4), succ(succ(x())))),
            None,
            "the left operand is not exhausted"
        );
        assert_eq!(
            observe_open(&c, &call("order.uniform", tnat(0), x())),
            Some(Db::Global("true".into())),
            "a uniform zero case `f 0 n = yes` holds by reduction for a free n"
        );
    }

    #[test]
    fn a_split_zero_case_converts_with_its_stuck_reduct_in_both_kernels() {
        let c = standard();
        let x = || tv("x");
        let goal = for_all_x(
            refl_bool(zero_case(x())),
            eq_bool(
                call("order.split", tnat(2), succ(succ(x()))),
                zero_case(x()),
            ),
        );
        crate::infer_type(&c, &goal)
            .expect("primary kernel: ble 2 (x+2) reduces to its stuck zero case");
        crate::recheck(&c, &goal).expect("independent kernel: the same reduct, never `true`");
    }

    #[test]
    fn a_stuck_order_is_not_its_certified_answer_in_either_kernel() {
        let c = standard();
        let x = || tv("x");
        let goal = for_all_x(
            refl_bool(tg("true")),
            eq_bool(call("order.split", tnat(0), x()), tg("true")),
        );
        assert!(
            crate::infer_type(&c, &goal).is_err(),
            "primary kernel: ble 0 x is stuck on x"
        );
        assert!(
            crate::recheck(&c, &goal).is_err(),
            "independent kernel: ble 0 x is stuck on x"
        );
        let goal = for_all_x(
            refl_bool(tg("true")),
            eq_bool(call("order.split", tnat(0), succ(x())), tg("true")),
        );
        crate::infer_type(&c, &goal).expect("primary kernel: ble 0 (x+1) = true");
        crate::recheck(&c, &goal).expect("independent kernel: ble 0 (x+1) = true");
        let goal = for_all_x(
            refl_bool(tg("true")),
            eq_bool(call("order.uniform", tnat(0), x()), tg("true")),
        );
        crate::infer_type(&c, &goal).expect("primary kernel: a uniform zero case reduces");
        crate::recheck(&c, &goal).expect("independent kernel: a uniform zero case reduces");
    }
}
