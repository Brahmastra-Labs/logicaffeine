//! W03 nested inductives, dimension 2: universe handling.
//!
//! The witness (`fixtures/lean_semantics/LF_W03_NestedUniverses.lean`) is an original Lean 4.14.0
//! module: `UTree.{u} (α : Type u) : Type u` nests itself through the universe-polymorphic `List`.
//! Lean generates `UTree.rec.{u_1, u}` and `UTree.rec_1.{u_1, u}` (a shared motive universe, then
//! the family's own), and its kernel checks `rfl` computations with the family at `Type` and at
//! `Type 1`. The references below are written from Lean's `#check` output, never from the reader.
use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode, Verdict};
use logicaffeine_kernel::{
    axiom_dependencies_of, defeq_for_test, infer_type, normalize_checked, recheck, BigInt, Context,
    KernelError, Literal, ReCheckError, Term, Universe,
};
use serde_json::{json, Value};

const SOURCE: &str = include_str!("fixtures/lean_semantics/LF_W03_NestedUniverses.lean");
const INPUT: &str = include_str!("fixtures/lean_semantics/LF_W03_NestedUniverses.ndjson");
const PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_W03_NestedUniverses.provenance.json");

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}
fn w(short: &str) -> String {
    n(&format!("LF_W03U.{short}"))
}
fn g(s: &str) -> Term {
    Term::Global(s.into())
}
fn v(s: &str) -> Term {
    Term::Var(s.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: Vec<Term>) -> Term {
    xs.into_iter().fold(f, app)
}
fn pi(x: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: x.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn u() -> Universe {
    Universe::Var(n("u"))
}
fn motive_level() -> Universe {
    Universe::Var(n("u_1"))
}
fn at(name: &str, levels: Vec<Universe>) -> Term {
    Term::Const {
        name: name.into(),
        levels,
    }
}
fn utree(level: Universe, a: Term) -> Term {
    app(at(&w("UTree"), vec![level]), a)
}
fn list(level: Universe, t: Term) -> Term {
    app(at(&n("List"), vec![level]), t)
}
fn nat() -> Term {
    g("Nat")
}
fn lit(k: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(k)))
}

fn original() -> (adapter::ImportedContext, adapter::ImportReport) {
    let p: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(
        p["source_sha256"],
        "90c012dea67c25c2b2518b44e7b96606acf3335c3440be1eeb4a064709013b07"
    );
    assert_eq!(
        p["export_sha256"],
        "3af53ea8d2188128e3c082c5234df1e500ac4642fd668af852fc19db630aa1a4"
    );
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
    assert_eq!(p["records"], INPUT.lines().count());
    assert_eq!(INPUT.len(), 52_635);
    assert_eq!(SOURCE.len(), 941);
    assert!(SOURCE.contains("inductive UTree (α : Type u) : Type u"));
    adapter::import_audited(INPUT)
        .expect("the unchanged original universe-polymorphic witness imports")
}

fn checks(ctx: &Context, term: &Term, expected: &Term) {
    let main = infer_type(ctx, term).expect("main kernel");
    assert!(
        defeq_for_test(ctx, &main, expected),
        "main: {main} vs {expected}"
    );
    let independent = recheck(ctx, term).expect("independent kernel");
    assert!(
        defeq_for_test(ctx, &independent, expected),
        "independent: {independent} vs {expected}"
    );
}

/// Lean's `#check @LF_W03U.UTree.rec` / `rec_1`.
fn recursor(major_is_list: bool) -> Term {
    let a = || v("α");
    let tree = || utree(u(), a());
    let m1 = |t: Term| app(v("motive_1"), t);
    let m2 = |t: Term| app(v("motive_2"), t);
    let sort = || Term::Sort(motive_level());
    let node = pi(
        "x",
        a(),
        pi(
            "c",
            list(u(), tree()),
            pi(
                "ih",
                m2(v("c")),
                m1(apps(
                    at(&w("UTree.node"), vec![u()]),
                    vec![a(), v("x"), v("c")],
                )),
            ),
        ),
    );
    let nil = m2(app(at(&n("List.nil"), vec![u()]), tree()));
    let cons = pi(
        "h",
        tree(),
        pi(
            "t",
            list(u(), tree()),
            pi(
                "ih1",
                m1(v("h")),
                pi(
                    "ih2",
                    m2(v("t")),
                    m2(apps(
                        at(&n("List.cons"), vec![u()]),
                        vec![tree(), v("h"), v("t")],
                    )),
                ),
            ),
        ),
    );
    let major = if major_is_list {
        pi("t", list(u(), tree()), m2(v("t")))
    } else {
        pi("t", tree(), m1(v("t")))
    };
    pi(
        "α",
        Term::Sort(u().succ()),
        pi(
            "motive_1",
            pi("t", tree(), sort()),
            pi(
                "motive_2",
                pi("t", list(u(), tree()), sort()),
                pi("node", node, pi("nil", nil, pi("cons", cons, major))),
            ),
        ),
    )
}

#[test]
fn the_universe_polymorphic_family_keeps_its_universe_parameter() {
    let (ctx, _) = original();
    let family = w("UTree");
    assert!(ctx.is_inductive(&family));
    assert_eq!(
        (
            ctx.inductive_num_params(&family),
            ctx.inductive_num_indices(&family)
        ),
        (1, 0)
    );
    let (levels, arity) = ctx
        .poly_declaration(&family)
        .expect("UTree is universe polymorphic");
    assert_eq!(levels, [n("u")]);
    assert!(defeq_for_test(
        &ctx,
        arity,
        &pi("α", Term::Sort(u().succ()), Term::Sort(u().succ()))
    ));
    let node = at(&w("UTree.node"), vec![u()]);
    let local = ctx.clone();
    checks(
        &local,
        &node,
        &pi(
            "α",
            Term::Sort(u().succ()),
            pi(
                "x",
                v("α"),
                pi("c", list(u(), utree(u(), v("α"))), utree(u(), v("α"))),
            ),
        ),
    );
    let usage = axiom_dependencies_of(&ctx, &family).unwrap();
    assert!(usage.axioms.is_empty() && usage.unresolved.is_empty());
}

#[test]
fn the_nested_recursors_are_leans_at_their_exact_universe_telescopes() {
    let (ctx, _) = original();
    for (suffix, major_is_list) in [("rec", false), ("rec_1", true)] {
        let (levels, ty, _) = ctx
            .get_universe_poly(&w(&format!("UTree.{suffix}")))
            .unwrap();
        assert_eq!(
            levels,
            &vec![n("u_1"), n("u")],
            "{suffix}: the motive universe, then the family's"
        );
        assert!(
            defeq_for_test(&ctx, ty, &recursor(major_is_list)),
            "{suffix}: {ty}"
        );
        assert!(!defeq_for_test(&ctx, ty, &recursor(!major_is_list)));
    }
    // An instance must supply both universes.
    let rec = |levels: Vec<Universe>| app(at(&w("UTree.rec"), levels), nat());
    infer_type(&ctx, &rec(vec![Universe::Type(0), Universe::Prop])).expect("rec.{1, 0} at Nat");
    for wrong in [
        vec![Universe::Type(0)],
        vec![],
        vec![Universe::Type(0), Universe::Prop, Universe::Prop],
    ] {
        assert!(infer_type(&ctx, &rec(wrong.clone())).is_err(), "{wrong:?}");
        assert!(recheck(&ctx, &rec(wrong)).is_err());
    }
}

#[test]
fn the_family_computes_at_type_and_at_type_one() {
    let (ctx, report) = original();
    for (tree, carrier, level, count) in [
        (w("small"), nat(), Universe::Prop, 3u64),
        (
            w("large"),
            Term::Sort(Universe::Type(0)),
            Universe::Type(0),
            2,
        ),
    ] {
        checks(&ctx, &g(&tree), &utree(level.clone(), carrier.clone()));
        let size = apps(at(&w("size"), vec![level]), vec![carrier, g(&tree)]);
        checks(&ctx, &size, &nat());
        let normal = normalize_checked(&ctx, &size).unwrap();
        assert!(
            defeq_for_test(&ctx, &normal, &lit(count)),
            "{tree}: {normal}"
        );
        let eq = |k: u64| {
            apps(
                at(&n("Eq"), vec![Universe::Type(0)]),
                vec![nat(), size.clone(), lit(k)],
            )
        };
        let refl = apps(
            at(&n("Eq.refl"), vec![Universe::Type(0)]),
            vec![nat(), lit(count)],
        );
        checks(&ctx, &refl, &eq(count));
        let wrong = Term::Let {
            name: "claimed".into(),
            ty: Box::new(eq(count + 1)),
            value: Box::new(refl),
            body: Box::new(v("claimed")),
        };
        assert!(matches!(
            infer_type(&ctx, &wrong),
            Err(KernelError::TypeMismatch { .. })
        ));
        assert!(matches!(recheck(&ctx, &wrong), Err(ReCheckError::Ill(_))));
    }
    // Lean: neither theorem depends on an axiom.
    for theorem in ["size_small", "size_large"] {
        let row = report
            .declarations
            .iter()
            .find(|d| d.name == w(theorem))
            .unwrap();
        assert_eq!(row.kind, "thm");
        assert!(row.axioms.is_empty(), "{theorem}: {:?}", row.axioms);
    }
    // The family at `Type 1` is not the family at `Type`.
    assert!(!defeq_for_test(
        &ctx,
        &utree(Universe::Type(0), nat()),
        &utree(Universe::Prop, nat())
    ));
}

// ==== corruption controls ====

fn rows() -> Vec<Value> {
    INPUT
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
fn text(rows: &[Value]) -> String {
    rows.iter().map(|r| format!("{r}\n")).collect()
}
fn block(rows: &mut [Value]) -> &mut Value {
    rows.iter_mut()
        .find(|r| r["inductive"]["types"][0]["numNested"] == 1)
        .unwrap()
        .get_mut("inductive")
        .unwrap()
}
fn rejected(rows: &[Value], what: &str) {
    let verdict = adapter::check_str(&text(rows)).expect_err(what);
    assert!(matches!(verdict, Verdict::Reject(_)), "{what}: {verdict:?}");
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::InvalidProof,
        "{what}: {verdict:?}"
    );
}

#[test]
fn universe_corruptions_are_rejected() {
    // The recursor without its motive universe, or without the family's.
    for keep in [0usize, 1] {
        let mut r = rows();
        let recs = block(&mut r)["recs"].as_array_mut().unwrap();
        let levels = recs[0]["levelParams"].as_array().unwrap().clone();
        recs[0]["levelParams"] = json!([levels[keep]]);
        rejected(&r, "a recursor missing one of its two universes");
    }
    // The constructor's nested field at a different universe: `List.{u+1} (UTree.{u} α)`.
    let mut r = rows();
    let ctor_ty = block(&mut r)["ctors"][0]["type"].clone();
    let find = |r: &[Value], id: &Value| r.iter().position(|row| row["ie"] == *id).unwrap();
    let alpha = r[find(&r, &ctor_ty)]["forallE"].clone();
    let x = r[find(&r, &alpha["body"])]["forallE"].clone();
    let c = r[find(&r, &x["body"])]["forallE"].clone();
    let field = r[find(&r, &c["type"])]["app"].clone();
    let list_u = r[find(&r, &field["fn"])]["const"].clone();
    let u_level = list_u["us"][0].clone();
    let at_block = r
        .iter()
        .position(|row| row["inductive"]["types"][0]["numNested"] == 1)
        .unwrap();
    let mut next = r.iter().filter_map(|row| row["ie"].as_u64()).max().unwrap() + 1;
    let next_level = r.iter().filter_map(|row| row["il"].as_u64()).max().unwrap() + 1;
    let mut add = |r: &mut Vec<Value>, row: Value| {
        let mut row = row;
        let at = r
            .iter()
            .position(|row| row["inductive"]["types"][0]["numNested"] == 1)
            .unwrap();
        if row.get("il").is_none() {
            row["ie"] = json!(next);
            next += 1;
        }
        let id = row.get("ie").cloned().unwrap_or_else(|| row["il"].clone());
        r.insert(at, row);
        id
    };
    let _ = at_block;
    let succ = add(&mut r, json!({"il": next_level, "succ": u_level}));
    let list_succ = add(
        &mut r,
        json!({"const": {"name": list_u["name"], "us": [succ]}}),
    );
    let widened = add(
        &mut r,
        json!({"app": {"fn": list_succ, "arg": field["arg"]}}),
    );
    let mut c2 = c.clone();
    c2["type"] = widened;
    let c2 = add(&mut r, json!({"forallE": c2}));
    let mut x2 = x.clone();
    x2["body"] = c2;
    let x2 = add(&mut r, json!({"forallE": x2}));
    let mut alpha2 = alpha.clone();
    alpha2["body"] = x2;
    let alpha2 = add(&mut r, json!({"forallE": alpha2}));
    block(&mut r)["ctors"][0]["type"] = alpha2;
    rejected(&r, "List.{u+1} (UTree.{u} α) in a Type u family");
}
