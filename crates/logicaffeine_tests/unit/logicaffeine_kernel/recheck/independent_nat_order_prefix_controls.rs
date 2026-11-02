use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn lit(n: i64) -> Db {
    Db::Lit(Literal::Nat(n.into()))
}
fn app(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
fn succ(t: Db) -> Db {
    app(g("Succ"), t)
}
fn op(name: &str, a: Db, b: Db) -> Db {
    app(app(g(name), a), b)
}
fn beta(t: Db) -> Db {
    app(Db::Lam(DbBox::new(g("Nat")), DbBox::new(Db::Var(0))), t)
}
fn zeta(t: Db) -> Db {
    Db::Let(DbBox::new(g("Nat")), DbBox::new(t), DbBox::new(Db::Var(0)))
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c.add_declaration("opaque_nat", Term::Global("Nat".into()));
    c
}
struct Restore(usize, usize, bool, Option<usize>, Option<usize>);
impl Drop for Restore {
    fn drop(&mut self) {
        WHNF_DEPTH.with(|v| v.set(self.0));
        WHNF_FUEL.with(|v| v.set(self.1));
        WHNF_EXHAUSTED.with(|v| v.set(self.2));
        NAT_ORDER_WORK.with(|v| v.set(self.3));
        WHNF_INPUT_CLONES.with(|v| v.set(self.4));
    }
}
struct Observation {
    result: Option<Db>,
    fuel: usize,
    exhausted: bool,
    work: usize,
    clones: usize,
}
fn observe(
    c: &Context,
    locals: &dyn LocalTypes,
    t: &Db,
    fuel: usize,
    depth: usize,
    exhausted: bool,
) -> Observation {
    let _restore = Restore(
        WHNF_DEPTH.with(|v| v.replace(depth)),
        WHNF_FUEL.with(|v| v.replace(fuel)),
        WHNF_EXHAUSTED.with(|v| v.replace(exhausted)),
        NAT_ORDER_WORK.with(|v| v.replace(Some(0))),
        WHNF_INPUT_CLONES.with(|v| v.replace(Some(0))),
    );
    let result = try_nat_order_db_in(c, locals, t);
    assert_eq!(
        WHNF_DEPTH.with(|v| v.get()),
        depth,
        "optional probe restores caller depth"
    );
    Observation {
        result,
        fuel: WHNF_FUEL.with(|v| v.get()),
        exhausted: WHNF_EXHAUSTED.with(|v| v.get()),
        work: NAT_ORDER_WORK.with(|v| v.get().unwrap()),
        clones: WHNF_INPUT_CLONES.with(|v| v.get().unwrap()),
    }
}
fn answer(c: &Context, t: Db, expected: bool) {
    let before = t.clone();
    let o = observe(c, &[], &t, 4096, 1, false);
    assert_eq!(o.result, Some(g(if expected { "true" } else { "false" })));
    assert!(!o.exhausted);
    assert!(
        o.fuel < 4096,
        "actual work must consume the existing budget"
    );
    assert_eq!(t, before, "fallback input remains exact");
}
#[test]
fn computed_constructor_and_literal_operands_decide_both_orders() {
    let c = context();
    for (a, b) in [(0, 0), (0, 1), (1, 0), (2, 2), (2, 3), (3, 2)] {
        let x = if a == 0 {
            beta(g("Zero"))
        } else {
            zeta(succ(lit(a - 1)))
        };
        let y = if b == 0 {
            g("Zero")
        } else {
            succ(beta(lit(b - 1)))
        };
        answer(&c, op("natLe", x.clone(), y.clone()), a <= b);
        answer(&c, op("natLt", x, y), a < b);
    }
}
#[test]
fn computed_division_tail_reuses_the_independent_whnf_scheduler() {
    let c = context();
    let remainder = op("Nat_mod", lit(7), lit(4));
    assert_eq!(infer(&c, &[], &remainder).unwrap(), g("Nat"));
    answer(&c, op("natLe", remainder.clone(), succ(lit(2))), true);
    answer(&c, op("natLt", remainder.clone(), lit(3)), false);
    answer(&c, op("natLe", lit(4), remainder), false);
}
#[test]
fn large_literal_tails_are_cancelled_without_unary_materialization() {
    let c = context();
    for (name, n, expected) in [
        ("natLe", 1000001, true),
        ("natLt", 1000001, false),
        ("natLe", 1000000, false),
        ("natLt", 1000002, true),
    ] {
        let t = op(name, succ(zeta(lit(1000000))), lit(n));
        let o = observe(&c, &[], &t, 128, 1, false);
        assert_eq!(o.result, Some(g(if expected { "true" } else { "false" })));
        assert!(!o.exhausted);
        assert!(
            o.work <= 64,
            "count actual cursor/WHNF dispatches, not numeral magnitude"
        );
        assert!(
            o.clones <= 16,
            "only the small demanded computed tail may be cloned"
        );
    }
}
#[test]
fn nonstrict_zero_and_successor_boundaries_preserve_lazy_demand() {
    let c = context();
    answer(&c, op("natLe", g("Zero"), g("opaque_nat")), true);
    answer(&c, op("natLe", succ(g("opaque_nat")), g("Zero")), false);
    answer(
        &c,
        op("natLe", succ(g("Zero")), succ(g("opaque_nat"))),
        true,
    );
    let o = observe(
        &c,
        &[],
        &op("natLe", g("opaque_nat"), g("Zero")),
        128,
        1,
        false,
    );
    assert_eq!(o.result, None);
    assert!(!o.exhausted);
    let o = observe(
        &c,
        &[],
        &op("natLe", succ(g("opaque_nat")), succ(g("Zero"))),
        128,
        1,
        false,
    );
    assert_eq!(o.result, None);
    assert!(!o.exhausted);
}
#[test]
fn strict_synthetic_successor_does_not_force_original_left_for_right_zero() {
    let c = context();
    answer(&c, op("natLt", g("opaque_nat"), g("Zero")), false);
    answer(&c, op("natLt", g("Zero"), succ(g("opaque_nat"))), true);
    let o = observe(
        &c,
        &[],
        &op("natLt", g("opaque_nat"), succ(g("Zero"))),
        128,
        1,
        false,
    );
    assert_eq!(o.result, None);
    assert!(!o.exhausted);
    // Ill-typed raw beta cycle is deliberately used only in an unused tail.
    let omega_lam = Db::Lam(
        DbBox::new(g("Nat")),
        DbBox::new(app(Db::Var(0), Db::Var(0))),
    );
    let omega = app(omega_lam.clone(), omega_lam);
    assert!(infer(&c, &[], &omega).is_err());
    let o = observe(&c, &[], &op("natLt", omega, g("Zero")), 32, 1, false);
    assert_eq!(o.result, Some(g("false")));
    assert!(!o.exhausted);
    assert_eq!(o.clones, 0);
}
#[test]
fn computed_local_tails_keep_debruijn_scope_and_neutral_refusal() {
    let c = context();
    let locals = [g("Nat"), g("Nat")];
    for i in [0, 1] {
        let t = op("natLe", succ(beta(Db::Var(i))), succ(lit(0)));
        let o = observe(&c, &locals, &t, 256, 1, false);
        assert_eq!(o.result, None);
        assert!(!o.exhausted);
        assert_eq!(infer(&c, &locals, &t).unwrap(), g("Bool"));
    }
    let t = op("natLe", zeta(g("Zero")), succ(Db::Var(1)));
    let o = observe(&c, &locals, &t, 256, 1, false);
    assert_eq!(o.result, Some(g("true")));
    assert!(!o.exhausted);
}
#[test]
fn operation_rebinding_and_dependency_rebinding_disable_the_shortcut() {
    let original = context();
    let mut changed = original.clone();
    let ty = changed.get_definition_type("natLe").unwrap().clone();
    let body = Term::Lambda {
        param: "m".into(),
        param_type: Box::new(Term::Global("Nat".into())),
        body: Box::new(Term::Lambda {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body: Box::new(Term::Global("false".into())),
        }),
    };
    let kept = changed
        .refused_meaning_change_for_test(|c| {
            c.with_equations_suspended(|c| c.declare("natLe", ty.clone(), body.clone()))
        })
        .expect("the gate keeps `natLe` under its dependents");
    answer(&kept, op("natLe", g("Zero"), lit(1)), true);
    changed.permit_meaning_changes_for_cache_tests();
    changed
        .with_equations_suspended(|c| c.declare("natLe", ty, body))
        .unwrap();
    for name in ["natLe", "natLt"] {
        assert_eq!(
            observe(&changed, &[], &op(name, g("Zero"), lit(1)), 128, 1, false).result,
            None
        );
    }
    answer(&original, op("natLe", g("Zero"), lit(1)), true);
    assert_eq!(whnf(&changed, &op("natLe", g("Zero"), lit(1))), g("false"));
}
#[test]
fn carrier_metadata_changes_are_context_local_and_not_cached() {
    let c = context();
    let t = op("natLe", succ(lit(2)), lit(3));
    answer(&c, t.clone(), true);
    let kept = c
        .refused_meaning_change_for_test(|c| {
            c.set_inductive_params("Nat", 1);
            Ok(())
        })
        .expect("the gate keeps `Nat`'s parameters under its dependents");
    answer(&kept, t.clone(), true);
    let mut changed = c.clone();
    changed.permit_meaning_changes_for_cache_tests();
    changed.set_inductive_params("Nat", 1);
    assert_eq!(observe(&changed, &[], &t, 128, 1, false).result, None);
    answer(&c, t, true);
}
#[test]
fn reducibility_scope_does_not_change_full_kernel_order_semantics() {
    let mut c = context();
    let t = op("natLt", succ(lit(2)), lit(4));
    c.push_scope();
    c.set_reducibility("natLe", crate::Reducibility::Irreducible);
    answer(&c, t.clone(), true);
    c.pop_scope();
    answer(&c, t, true);
}
#[test]
fn explicit_levels_and_partial_or_excess_applications_keep_fallback() {
    let c = context();
    for levels in [vec![], vec![Universe::Type(0)]] {
        let t = app(
            app(
                Db::Const {
                    name: "natLe".into(),
                    levels,
                },
                g("Zero"),
            ),
            lit(1),
        );
        assert_eq!(observe(&c, &[], &t, 128, 1, false).result, None);
    }
    for t in [
        app(g("natLe"), lit(0)),
        app(op("natLe", lit(0), lit(1)), lit(2)),
    ] {
        assert_eq!(observe(&c, &[], &t, 128, 1, false).result, None);
    }
}
#[test]
fn malformed_nat_holes_and_other_literal_tags_still_fail_typing() {
    let c = context();
    for t in [
        op("natLe", lit(-1), lit(0)),
        op("natLt", Db::Lit(Literal::Int(1)), lit(0)),
    ] {
        assert!(infer(&c, &[], &t).is_err());
        let named = from_db(&t, 0);
        assert!(crate::infer_type(&c, &named).is_err());
        assert!(matches!(
            crate::recheck(&c, &named),
            Err(crate::ReCheckError::Ill(_))
        ));
    }
    // Db has no Hole variant: the public door refuses it before conversion.
    let hole = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("natLe".into())),
            Box::new(Term::Global("Zero".into())),
        )),
        Box::new(Term::Hole),
    );
    assert!(crate::infer_type(&c, &hole).is_err());
    assert!(matches!(
        crate::recheck(&c, &hole),
        Err(crate::ReCheckError::Unsupported(_))
    ));
    // Existing reduction spelling is kept; this is not a typing certificate.
    answer(&c, op("natLe", lit(-1), lit(0)), true);
    answer(&c, op("natLt", lit(-1), lit(0)), false);
}
#[test]
fn exhausted_shared_fuel_cannot_restart_or_emit_a_partial_answer() {
    let c = context();
    let t = op("natLe", succ(zeta(lit(7))), lit(8));
    for fuel in [0, 1, 2] {
        let o = observe(&c, &[], &t, fuel, 1, false);
        assert_eq!(o.result, None);
        assert!(o.exhausted);
        assert_eq!(o.fuel, 0);
        assert!(o.work <= fuel);
    }
    let o = observe(&c, &[], &t, 128, 1, true);
    assert_eq!(o.result, None);
    assert!(o.exhausted);
    assert_eq!(o.fuel, 128);
    assert_eq!(o.work, 0);
    assert_eq!(o.clones, 0);
}
#[test]
fn computed_tail_depth_refusal_precedes_input_clone() {
    let c = context();
    let mut tail = lit(0);
    for _ in 0..(MAX_RECHECK_WHNF_DEPTH + 1) {
        tail = zeta(tail);
    }
    let t = op("natLe", tail, lit(0));
    let o = observe(&c, &[], &t, 4096, 1, false);
    assert_eq!(o.result, None);
    assert!(o.exhausted);
    assert_eq!(o.clones, 0);
    assert!(o.work <= 4096);
    let shallow = op("natLe", lit(0), lit(0));
    let o = observe(&c, &[], &shallow, 128, MAX_RECHECK_WHNF_DEPTH, false);
    assert_eq!(o.result, None);
    assert!(o.exhausted);
    assert_eq!(o.clones, 0);
}

#[test]
fn borrowed_admission_accepts_the_exact_depth_boundary_before_clone() {
    let _restore = Restore(
        WHNF_DEPTH.with(|v| v.replace(1)),
        WHNF_FUEL.with(|v| v.replace(16384)),
        WHNF_EXHAUSTED.with(|v| v.replace(false)),
        NAT_ORDER_WORK.with(|v| v.replace(Some(0))),
        WHNF_INPUT_CLONES.with(|v| v.replace(Some(0))),
    );
    let mut t = lit(0);
    for _ in 1..MAX_RECHECK_WHNF_DEPTH {
        t = zeta(t);
    }
    assert_eq!(
        nat_order_admit(&t),
        Some(3 * (MAX_RECHECK_WHNF_DEPTH - 1) + 1)
    );
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
    assert!(WHNF_FUEL.with(|v| v.get()) < 16384);
    assert!(NAT_ORDER_WORK.with(|v| v.get().unwrap()) > 0);
    assert_eq!(WHNF_INPUT_CLONES.with(|v| v.get()), Some(0));
}

#[test]
fn wide_computed_tail_admission_stops_before_clone_or_frontier_growth() {
    let c = context();
    for tail in [
        Db::Match {
            disc: DbBox::new(g("Zero")),
            motive: DbBox::new(g("Nat")),
            cases: vec![lit(0); 64],
        },
        Db::MutualFix {
            defs: vec![lit(0); 64],
            index: 0,
        },
    ] {
        let t = op("natLe", tail, lit(0));
        let original = t.clone();
        let o = observe(&c, &[], &t, 16, 1, false);
        assert_eq!(o.result, None);
        assert!(o.exhausted);
        assert_eq!(o.fuel, 0);
        assert!(o.work <= 16);
        assert_eq!(o.clones, 0);
        assert_eq!(t, original);
    }
}
