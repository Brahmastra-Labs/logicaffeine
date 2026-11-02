//! The quotient computation rule behind a DEFINED head, decided the same way by both kernels.
//!
//! Finding (lane LG, Arena `mathlib`, `logs/lean-lg/arena-full214-cand4-20260923-145456/042.stderr`):
//! the primary kernel accepted Lean core's `Quotient.lift₂._proof_2` and the independent re-checker
//! reported it `Ill`. Lean's kernel accepts the declaration, so the re-checker was wrong. The proof
//! needs `Quotient.lift f h (Quotient.mk s a) ≡ f a`, which holds only after δ: `Quotient.lift`
//! unfolds to the PARTIAL primitive `Quot.lift f`, so the respect proof and the major premise reach
//! `Quot.lift` from the enclosing application. The re-checker's weak-head machine offered the
//! quotient rule to the spine it was handed, but never to the spine a resumed frame rebuilds after
//! the head has been reduced, so it returned `Quot.lift A r B f h (Quot.mk A r a)` as a normal form.
//!
//! Original witnesses (`fixtures/lean_semantics/LF_QuotientLift.lean`, Lean 4.14.0, pinned
//! lean4export): Lean core's `Quotient.lift₂` itself; five theorems Lean's kernel accepts (the rule
//! through `Quotient.lift`, `Quotient.liftOn`, `Quotient.lift₂`, `Quot.rec`, and the bare primitive);
//! and two corrupted statements Lean's kernel REJECTS (recorded in each provenance), which both
//! kernels here must refuse as well.
use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode, Verdict};
use logicaffeine_kernel::quotient::{self, Kind};
use logicaffeine_kernel::{infer_type, is_subtype, recheck, Context, Term, Universe};
use serde_json::Value;

const SOURCE: &str = include_str!("fixtures/lean_semantics/LF_QuotientLift.lean");
const SOURCE_SHA: &str = "eb20c742ba40aa5c2d525519042918602241990f91cc3f895ca6c72179885221";
const LIFT2: &str = include_str!("fixtures/lean_semantics/LF_QuotientLift.lift2.ndjson");
const LIFT2_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_QuotientLift.lift2.provenance.json");
const POSITIVE: &str = include_str!("fixtures/lean_semantics/LF_QuotientLift.positive.ndjson");
const POSITIVE_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_QuotientLift.positive.provenance.json");
const WRONG_REP: &str = include_str!("fixtures/lean_semantics/LF_QuotientLift.wrongRep.ndjson");
const WRONG_REP_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_QuotientLift.wrongRep.provenance.json");
const SWAPPED: &str = include_str!("fixtures/lean_semantics/LF_QuotientLift.swapped.ndjson");
const SWAPPED_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_QuotientLift.swapped.provenance.json");

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}
fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: impl IntoIterator<Item = Term>) -> Term {
    xs.into_iter().fold(f, app)
}
fn pi(x: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: x.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(x: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: x.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn claim(proof: Term, statement: Term) -> Term {
    Term::Let {
        name: "claim".into(),
        ty: Box::new(statement),
        value: Box::new(proof),
        body: Box::new(v("claim")),
    }
}

/// The provenance of one export: the pinned toolchain and exporter, the source this file reads,
/// and the export it includes, byte for byte.
fn pinned(provenance: &str, export_sha: &str, input: &str, roots: &[&str]) -> Value {
    let p: Value = serde_json::from_str(provenance).unwrap();
    assert_eq!(
        p["source"],
        "crates/logicaffeine_tests/tests/fixtures/lean_semantics/LF_QuotientLift.lean"
    );
    assert_eq!(p["source_sha256"], SOURCE_SHA);
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(p["expression_equality"], "structural");
    assert_eq!(
        (p["compiler_exit"].as_i64(), p["exporter_exit"].as_i64()),
        (Some(0), Some(0))
    );
    assert_eq!(p["export_sha256"], export_sha);
    assert_eq!(p["records"], input.lines().count());
    assert_eq!(p["export_header"]["meta"]["lean"]["version"], "4.14.0");
    let declared: Vec<&str> = p["declarations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap())
        .collect();
    assert_eq!(declared, roots);
    p
}

/// Both kernels, directly: the published proof of `name` is checked by the primary kernel and by
/// the independent re-checker, and each inferred type is exactly the declared statement.
fn both_kernels_check_the_published_proof(ctx: &Context, name: &str) {
    let statement = ctx
        .get_global(name)
        .unwrap_or_else(|| panic!("{name} is published"))
        .clone();
    let proof = ctx
        .get_definition_body(name)
        .unwrap_or_else(|| panic!("{name} keeps its proof"))
        .clone();
    let main = infer_type(ctx, &proof).unwrap_or_else(|e| panic!("{name}: primary kernel: {e:?}"));
    let independent =
        recheck(ctx, &proof).unwrap_or_else(|e| panic!("{name}: independent kernel: {e:?}"));
    for inferred in [&main, &independent] {
        assert!(
            is_subtype(ctx, inferred, &statement) && is_subtype(ctx, &statement, inferred),
            "{name}: the proof proves exactly the declared statement"
        );
    }
}

#[test]
fn leans_own_binary_lift_is_accepted_by_both_kernels() {
    pinned(
        LIFT2_PROVENANCE,
        "8eb1b1aa1d2ca9542ca16d82891b0579b6941e307d21e006f304ee9ab86bc32e",
        LIFT2,
        &["Quotient.lift₂"],
    );
    let (ctx, report) = adapter::import_audited(LIFT2).unwrap_or_else(|v| {
        panic!("Lean's kernel accepts Quotient.lift₂; so must both kernels here: {v:?}")
    });
    let lift2 = report
        .declarations
        .iter()
        .find(|d| d.name == n("Quotient.lift₂"))
        .expect("the root is audited");
    assert_eq!(lift2.kind, "def");
    assert!(lift2.axioms.is_empty(), "{:?}", lift2.axioms);
    // The declaration the two kernels disagreed on, and its sibling, rechecked on their own.
    for proof in ["Quotient.lift₂.proof_1", "Quotient.lift₂.proof_2"] {
        both_kernels_check_the_published_proof(&ctx, &n(proof));
    }
}

#[test]
fn the_rule_behind_every_defined_head_is_accepted_by_both_kernels() {
    let roots = [
        "LF_QL.liftMk",
        "LF_QL.liftOnMk",
        "LF_QL.lift₂Mk",
        "LF_QL.recMk",
        "LF_QL.primLiftMk",
    ];
    let p = pinned(
        POSITIVE_PROVENANCE,
        "3c54461f148a23e70de69266dafd6a113182e578760e8af9cbf72fc349468b04",
        POSITIVE,
        &roots,
    );
    for root in &roots[..3] {
        assert!(
            SOURCE.contains(&format!("theorem {}", root.trim_start_matches("LF_QL."))),
            "{root} is in the source"
        );
    }
    let (ctx, report) = adapter::import_audited(POSITIVE)
        .unwrap_or_else(|v| panic!("Lean's kernel accepts every positive witness: {v:?}"));
    // Lean's `#print axioms` for each root, and the importer's own assumption audit, agree.
    let lean_axioms: Vec<&str> = p["lean_print_axioms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    for root in roots {
        let audit = report
            .declarations
            .iter()
            .find(|d| d.name == n(root))
            .unwrap_or_else(|| panic!("{root} is audited"));
        assert_eq!(audit.kind, "thm", "{root}");
        let expected: Vec<String> = if root == "LF_QL.recMk" {
            vec![n("Quot.sound")]
        } else {
            vec![]
        };
        assert_eq!(audit.axioms, expected, "{root}");
        let line = lean_axioms
            .iter()
            .find(|l| l.starts_with(&format!("'{root}'")))
            .unwrap();
        assert_eq!(
            expected.is_empty(),
            line.ends_with("does not depend on any axioms"),
            "{line}"
        );
        both_kernels_check_the_published_proof(&ctx, &n(root));
    }
}

fn refused_by_the_importer(input: &str, name: &str) {
    let verdict = adapter::check_str(input).expect_err("Lean's kernel rejects this declaration");
    assert!(
        matches!(verdict, Verdict::Reject(_)),
        "{name}: a false statement is rejected, never declined: {verdict:?}"
    );
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::InvalidProof,
        "{name}: {verdict:?}"
    );
}

fn lean_rejected(provenance: &Value, name: &str) {
    let verdicts: Vec<&str> = provenance["lean_kernel_verdicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    assert!(
        verdicts.iter().any(|l| l.starts_with(&format!(
            "{name}: kernel REJECTED: (kernel) declaration type mismatch"
        ))),
        "Lean 4.14.0's kernel refused {name}: {verdicts:?}"
    );
    assert!(
        !verdicts.iter().any(|l| l.contains("kernel ACCEPTED")),
        "{verdicts:?}"
    );
}

#[test]
fn the_rule_claimed_at_the_wrong_representative_is_refused() {
    let p = pinned(
        WRONG_REP_PROVENANCE,
        "2481958dbcda1b4efc564d9c8ea0837d5be891b985b43025d6164a3041638385",
        WRONG_REP,
        &["LF_QL.liftMkWrongRep"],
    );
    lean_rejected(&p, "LF_QL.liftMkWrongRep");
    refused_by_the_importer(WRONG_REP, "LF_QL.liftMkWrongRep");
}

#[test]
fn the_binary_lift_with_exchanged_representatives_is_refused() {
    let p = pinned(
        SWAPPED_PROVENANCE,
        "44fccc30ed4c3dd7b77a5e2b2679bb6fa1bf18a62725d19b9bf87f3209bd1f80",
        SWAPPED,
        &["LF_QL.lift₂MkSwapped"],
    );
    lean_rejected(&p, "LF_QL.lift₂MkSwapped");
    refused_by_the_importer(SWAPPED, "LF_QL.lift₂MkSwapped");
}

/// Peel `count` leading Π-binders off a statement.
fn telescope(mut t: &Term, count: usize) -> (Vec<(String, Term)>, &Term) {
    let mut binders = Vec::new();
    for _ in 0..count {
        let Term::Pi {
            param,
            param_type,
            body_type,
        } = t
        else {
            panic!("a {count}-binder statement")
        };
        binders.push((param.clone(), (**param_type).clone()));
        t = body_type;
    }
    (binders, t)
}
/// `@Eq.{l} A lhs rhs` → `(levels, A, lhs, rhs)`.
fn equation(t: &Term) -> (Vec<Universe>, Term, Term, Term) {
    let Term::App(f, rhs) = t else {
        panic!("an equation")
    };
    let Term::App(f, lhs) = f.as_ref() else {
        panic!("an equation")
    };
    let Term::App(eq, carrier) = f.as_ref() else {
        panic!("an equation")
    };
    let Term::Const { name, levels } = eq.as_ref() else {
        panic!("@Eq.{{l}}: {eq:?}")
    };
    assert_eq!(name, &n("Eq"));
    (
        levels.clone(),
        (**carrier).clone(),
        (**lhs).clone(),
        (**rhs).clone(),
    )
}
fn close(binders: &[(String, Term)], body: Term, binder: fn(&str, Term, Term) -> Term) -> Term {
    binders
        .iter()
        .rev()
        .fold(body, |b, (x, a)| binder(x, a.clone(), b))
}

/// Both kernels, on hand-built claims inside the imported environment: `liftMk`'s own left side is
/// `f a` by `rfl`, and is refused at `f b` for a fresh `b` — so the rule computes the RIGHT value in
/// each kernel, not merely some value.
#[test]
fn both_kernels_compute_the_lift_to_its_representative_and_to_nothing_else() {
    let (ctx, _) = adapter::import_audited(POSITIVE).expect("the positive witnesses import");
    let statement = ctx.get_global(&n("LF_QL.liftMk")).unwrap().clone();
    let (mut binders, body) = telescope(&statement, 6);
    let (levels, carrier, lhs, rhs) = equation(body);
    let refl = |x: Term| {
        apps(
            Term::Const {
                name: n("Eq.refl"),
                levels: levels.clone(),
            },
            [carrier.clone(), x],
        )
    };
    let eq = |l: Term, r: Term| {
        apps(
            Term::Const {
                name: n("Eq"),
                levels: levels.clone(),
            },
            [carrier.clone(), l, r],
        )
    };
    let Term::App(f, a) = &rhs else {
        panic!("the right side is `f a`: {rhs:?}")
    };
    let good = close(
        &binders,
        claim(refl(rhs.clone()), eq(lhs.clone(), rhs.clone())),
        lam,
    );
    infer_type(&ctx, &good).expect("primary kernel: Quotient.lift f h (Quotient.mk s a) ≡ f a");
    recheck(&ctx, &good).expect("independent kernel: Quotient.lift f h (Quotient.mk s a) ≡ f a");
    let alpha = binders[0].0.clone();
    let other = "other_representative".to_string();
    assert!(binders.iter().all(|(x, _)| *x != other));
    binders.push((other.clone(), v(&alpha)));
    let wrong_value = app((**f).clone(), v(&other));
    assert_ne!(**a, v(&other));
    let bad = close(
        &binders,
        claim(refl(wrong_value.clone()), eq(lhs, wrong_value)),
        lam,
    );
    assert!(
        infer_type(&ctx, &bad).is_err(),
        "primary kernel refuses the lift at another representative"
    );
    assert!(
        recheck(&ctx, &bad).is_err(),
        "independent kernel refuses the lift at another representative"
    );
}

// ---------------------------------------------------------------------------------------------
// The mechanism, in LOGOS's own vocabulary: the certified quotient primitives behind definitions.
// ---------------------------------------------------------------------------------------------

fn quotient_environment() -> Context {
    let mut c = Context::new();
    let u = Universe::Var("u".into());
    c.add_universe_poly_inductive(
        "Eq",
        vec!["u".into()],
        pi(
            "A",
            Term::Sort(u.clone()),
            pi("a", v("A"), pi("b", v("A"), Term::Sort(Universe::Prop))),
        ),
        2,
    );
    c.add_constructor(
        "refl",
        "Eq",
        pi(
            "A",
            Term::Sort(u.clone()),
            pi(
                "a",
                v("A"),
                apps(
                    Term::Const {
                        name: "Eq".into(),
                        levels: vec![u],
                    },
                    [v("A"), v("a"), v("a")],
                ),
            ),
        ),
    );
    for kind in [Kind::Type, Kind::Constructor, Kind::Lift, Kind::Induction] {
        let params = if kind == Kind::Lift {
            vec!["u".into(), "v".into()]
        } else {
            vec!["u".into()]
        };
        quotient::install(&mut c, kind, params, "Eq").unwrap();
    }
    let type0 = || Term::Sort(Universe::Type(0));
    c.add_declaration("A", type0());
    c.add_declaration("a", g("A"));
    c.add_declaration("b", g("A"));
    c.add_declaration(
        "R",
        pi("x", g("A"), pi("y", g("A"), Term::Sort(Universe::Prop))),
    );
    c.add_declaration("B", type0());
    c.add_declaration("f", pi("x", g("A"), g("B")));
    c.add_declaration("respect", respect(g("B"), g("f")));
    c.add_declaration("k", pi("x", g("A"), pi("y", g("A"), g("A"))));
    c.add_declaration("respect_k", respect(pi("y", g("A"), g("A")), g("k")));
    // `lift` is what Lean's `Quotient.lift` is: a definition whose body is the PARTIAL primitive.
    // `mk` is what `Quotient.mk` is: a definition over the primitive constructor.
    let primitive_lift = Term::Const {
        name: Kind::Lift.name().into(),
        levels: vec![Universe::Type(0), Universe::Type(0)],
    };
    for (name, target) in [("lift", g("B")), ("lift_fn", pi("y", g("A"), g("A")))] {
        let function = pi("x", g("A"), target.clone());
        let body = lam(
            "fn",
            function,
            apps(primitive_lift.clone(), [g("A"), g("R"), target, v("fn")]),
        );
        let ty = infer_type(&c, &body).unwrap();
        c.add_definition(name.into(), ty, body);
    }
    let body = lam(
        "x",
        g("A"),
        apps(
            Term::Const {
                name: Kind::Constructor.name().into(),
                levels: vec![Universe::Type(0)],
            },
            [g("A"), g("R"), v("x")],
        ),
    );
    let ty = infer_type(&c, &body).unwrap();
    c.add_definition("mk".into(), ty, body);
    c
}
fn respect(target: Term, function: Term) -> Term {
    pi(
        "x",
        g("A"),
        pi(
            "y",
            g("A"),
            pi(
                "h",
                apps(g("R"), [v("x"), v("y")]),
                apps(
                    Term::Const {
                        name: "Eq".into(),
                        levels: vec![Universe::Type(0)],
                    },
                    [target, app(function.clone(), v("x")), app(function, v("y"))],
                ),
            ),
        ),
    )
}
fn primitive_mk(x: Term) -> Term {
    apps(
        Term::Const {
            name: Kind::Constructor.name().into(),
            levels: vec![Universe::Type(0)],
        },
        [g("A"), g("R"), x],
    )
}
fn eq0(carrier: Term, l: Term, r: Term) -> Term {
    apps(
        Term::Const {
            name: "Eq".into(),
            levels: vec![Universe::Type(0)],
        },
        [carrier, l, r],
    )
}
fn refl0(carrier: Term, x: Term) -> Term {
    apps(
        Term::Const {
            name: "refl".into(),
            levels: vec![Universe::Type(0)],
        },
        [carrier, x],
    )
}
fn both_accept(c: &Context, what: &str, t: &Term) {
    infer_type(c, t).unwrap_or_else(|e| panic!("{what}: primary kernel: {e:?}"));
    recheck(c, t).unwrap_or_else(|e| panic!("{what}: independent kernel: {e:?}"));
}
fn both_refuse(c: &Context, what: &str, t: &Term) {
    assert!(
        infer_type(c, t).is_err(),
        "{what}: primary kernel must refuse"
    );
    assert!(
        recheck(c, t).is_err(),
        "{what}: independent kernel must refuse"
    );
}

#[test]
fn a_defined_partial_lift_computes_in_both_kernels() {
    let c = quotient_environment();
    let fa = app(g("f"), g("a"));
    let direct = apps(
        Term::Const {
            name: Kind::Lift.name().into(),
            levels: vec![Universe::Type(0), Universe::Type(0)],
        },
        [
            g("A"),
            g("R"),
            g("B"),
            g("f"),
            g("respect"),
            primitive_mk(g("a")),
        ],
    );
    both_accept(
        &c,
        "the primitive spine",
        &claim(refl0(g("B"), fa.clone()), eq0(g("B"), direct, fa.clone())),
    );
    let defined = apps(g("lift"), [g("f"), g("respect"), primitive_mk(g("a"))]);
    both_accept(
        &c,
        "a defined head over the partial primitive",
        &claim(refl0(g("B"), fa.clone()), eq0(g("B"), defined, fa.clone())),
    );
    let both_defined = apps(g("lift"), [g("f"), g("respect"), app(g("mk"), g("a"))]);
    both_accept(
        &c,
        "a defined head and a defined representative",
        &claim(
            refl0(g("B"), fa.clone()),
            eq0(g("B"), both_defined.clone(), fa.clone()),
        ),
    );
    let fb = app(g("f"), g("b"));
    both_refuse(
        &c,
        "the lift at another representative",
        &claim(refl0(g("B"), fb.clone()), eq0(g("B"), both_defined, fb)),
    );
}

#[test]
fn surplus_arguments_after_a_defined_lift_stay_applied_to_the_reduct() {
    // `lift_fn k respect_k (mk a) b ≡ k a b`: the rule fires on its own six arguments and the
    // seventh, still waiting in the enclosing application, is applied to the reduct.
    let c = quotient_environment();
    let kab = apps(g("k"), [g("a"), g("b")]);
    let lifted = apps(
        g("lift_fn"),
        [g("k"), g("respect_k"), app(g("mk"), g("a")), g("b")],
    );
    both_accept(
        &c,
        "an over-applied defined lift",
        &claim(refl0(g("A"), kab.clone()), eq0(g("A"), lifted.clone(), kab)),
    );
    let kba = apps(g("k"), [g("b"), g("a")]);
    both_refuse(
        &c,
        "an over-applied lift with its arguments exchanged",
        &claim(refl0(g("A"), kba.clone()), eq0(g("A"), lifted, kba)),
    );
}
