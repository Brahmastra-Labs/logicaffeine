//! W03 nested inductives, dimension 1: parameter preservation.
//!
//! The witness is an original Lean 4.14.0 module (`fixtures/lean_semantics/LF_W03_NestedParams.lean`)
//! exported unchanged by the pinned structural lean4export. Lean accepts every family in it and
//! its kernel checks every `rfl` computation, so the export is independent evidence of the
//! families, their generated nested recursors and those recursors' computation rules.
//!
//! Each test compares the imported artifact with a reference written here by hand from Lean's own
//! `#check` output — never with the importer's output — and checks it in both kernels. The
//! corruption controls each change one thing about the original export (a parameter count, an
//! index count, a universe, a constructor, a rule, the positivity of a field) and require the
//! exact refusal Lean's kernel gives the same input.
use logicaffeine_kernel::inductive_compile::{
    compile_nested_with_params, derive_native_recursors_with_params, NestedDecl,
};
use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode, Verdict};
use logicaffeine_kernel::{
    axiom_dependencies_of, defeq_for_test, double_check, infer_type, normalize_checked, recheck,
    BigInt, Context, DoubleCheck, KernelError, Literal, MutualInductive, ReCheckError, Term,
    Universe,
};
use serde_json::{json, Value};

const SOURCE: &str = include_str!("fixtures/lean_semantics/LF_W03_NestedParams.lean");
const INPUT: &str = include_str!("fixtures/lean_semantics/LF_W03_NestedParams.ndjson");
const PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_W03_NestedParams.provenance.json");

fn n(path: &str) -> String {
    path.split('.').map(|part| format!("/s:{part:?}")).collect()
}
fn w(short: &str) -> String {
    n(&format!("LF_W03.{short}"))
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
fn pi(p: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: p.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(p: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn nat() -> Term {
    g("Nat")
}
fn lit(k: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(k)))
}
fn motive_sort() -> Term {
    Term::Sort(Universe::Var(n("u")))
}
fn at0(name: &str) -> Term {
    Term::Const {
        name: n(name),
        levels: vec![Universe::Prop],
    }
}
fn list(t: Term) -> Term {
    app(at0("List"), t)
}
fn prod(a: Term, b: Term) -> Term {
    apps(
        Term::Const {
            name: n("Prod"),
            levels: vec![Universe::Prop, Universe::Prop],
        },
        vec![a, b],
    )
}
fn ptree(a: Term) -> Term {
    app(g(&w("PTree")), a)
}

/// The exact original export, pinned to its provenance.
fn original() -> (adapter::ImportedContext, adapter::ImportReport) {
    let p: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(
        p["source_sha256"],
        "328e9f0379b0816c18b6dcffc05698cf0799d3e7fe742aa0cfd2c104b5fdee3d"
    );
    assert_eq!(
        p["export_sha256"],
        "730315be5cdfc0b4f9ece1158aa842487e22ceb9cf5f752070dd93b10f29d7a8"
    );
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(p["expression_equality"], "structural");
    assert_eq!(p["compiler_exit"], 0);
    assert_eq!(p["exporter_exit"], 0);
    assert_eq!(p["records"], INPUT.lines().count());
    assert_eq!(INPUT.len(), 712_570);
    assert_eq!(SOURCE.len(), 3_166);
    assert_eq!(
        serde_json::from_str::<Value>(INPUT.lines().next().unwrap()).unwrap(),
        p["export_header"]
    );
    for family in ["PTree", "NTree", "PairTree", "DeepTree"] {
        assert!(
            SOURCE.contains(&format!("inductive {family}")),
            "witness source declares {family}"
        );
    }
    adapter::import_audited(INPUT).expect("the unchanged original nested-parameter witness imports")
}

fn checks(ctx: &Context, term: &Term, expected: &Term) {
    let main = infer_type(ctx, term).expect("main kernel checks the original term");
    assert!(
        defeq_for_test(ctx, &main, expected),
        "main type {main} is not {expected}"
    );
    let independent = recheck(ctx, term).expect("independent kernel checks the original term");
    assert!(
        defeq_for_test(ctx, &independent, expected),
        "independent type {independent} is not {expected}"
    );
}

fn declared(ctx: &Context, name: &str) -> Term {
    ctx.get_global(name)
        .unwrap_or_else(|| panic!("{name} is declared"))
        .clone()
}

fn poly(ctx: &Context, name: &str) -> (Vec<String>, Term) {
    let (levels, ty, _) = ctx
        .get_universe_poly(name)
        .unwrap_or_else(|| panic!("{name} is universe polymorphic"));
    (levels.clone(), ty.clone())
}

/// Lean's `#check @LF_W03.PTree.rec` (and `rec_1`, differing only in the major premise).
fn ptree_recursor(major_is_list: bool) -> Term {
    let a = || v("α");
    let tree = || ptree(a());
    let m1 = |t: Term| app(v("motive_1"), t);
    let m2 = |t: Term| app(v("motive_2"), t);
    let node = pi(
        "x",
        a(),
        pi(
            "children",
            list(tree()),
            pi(
                "ih",
                m2(v("children")),
                m1(apps(g(&w("PTree.node")), vec![a(), v("x"), v("children")])),
            ),
        ),
    );
    let nil = m2(app(at0("List.nil"), tree()));
    let cons = pi(
        "head",
        tree(),
        pi(
            "tail",
            list(tree()),
            pi(
                "head_ih",
                m1(v("head")),
                pi(
                    "tail_ih",
                    m2(v("tail")),
                    m2(apps(at0("List.cons"), vec![tree(), v("head"), v("tail")])),
                ),
            ),
        ),
    );
    let major = if major_is_list {
        pi("t", list(tree()), m2(v("t")))
    } else {
        pi("t", tree(), m1(v("t")))
    };
    pi(
        "α",
        ty0(),
        pi(
            "motive_1",
            pi("t", tree(), motive_sort()),
            pi(
                "motive_2",
                pi("t", list(tree()), motive_sort()),
                pi("node", node, pi("nil", nil, pi("cons", cons, major))),
            ),
        ),
    )
}

/// The first `names.len()` binder domains of a telescope, each binder renamed to `names[i]` so
/// later domains mention earlier binders by the caller's names.
fn leading_domains(mut ty: Term, names: &[&str]) -> Vec<Term> {
    let mut domains = Vec::new();
    for name in names {
        let Term::Pi {
            param,
            param_type,
            body_type,
        } = ty
        else {
            panic!("short recursor telescope")
        };
        domains.push(*param_type);
        ty = logicaffeine_kernel::substitute(&body_type, &param, &v(name));
    }
    domains
}

#[test]
fn original_parameterized_nested_families_keep_their_exact_parameters() {
    let (ctx, report) = original();
    assert!(report.runtime_declarations.is_empty());
    for (family, params) in [("PTree", 1), ("NTree", 1), ("PairTree", 1), ("DeepTree", 0)] {
        let name = w(family);
        assert!(
            ctx.is_inductive(&name),
            "{family} is an inductive family, not an axiom"
        );
        assert_eq!(
            ctx.inductive_num_params(&name),
            params,
            "{family} keeps its parameters"
        );
        assert_eq!(
            ctx.inductive_num_indices(&name),
            0,
            "{family} has no indices to invent"
        );
        let usage = axiom_dependencies_of(&ctx, &name).unwrap();
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{family}: {usage:?}"
        );
    }
    let arities = [
        ("PTree", pi("α", ty0(), ty0())),
        ("NTree", pi("n", nat(), ty0())),
        ("PairTree", pi("α", ty0(), ty0())),
        ("DeepTree", ty0()),
    ];
    for (family, arity) in arities {
        assert!(
            defeq_for_test(&ctx, &declared(&ctx, &w(family)), &arity),
            "{family} arity"
        );
    }
    let constructors = [
        (
            "PTree.node",
            pi(
                "α",
                ty0(),
                pi("x", v("α"), pi("c", list(ptree(v("α"))), ptree(v("α")))),
            ),
        ),
        (
            "NTree.node",
            pi(
                "n",
                nat(),
                pi(
                    "i",
                    app(g(&n("Fin")), v("n")),
                    pi(
                        "c",
                        list(app(g(&w("NTree")), v("n"))),
                        app(g(&w("NTree")), v("n")),
                    ),
                ),
            ),
        ),
        (
            "PairTree.node",
            pi(
                "α",
                ty0(),
                pi(
                    "c",
                    list(prod(v("α"), app(g(&w("PairTree")), v("α")))),
                    app(g(&w("PairTree")), v("α")),
                ),
            ),
        ),
        (
            "DeepTree.node",
            pi("c", list(list(g(&w("DeepTree")))), g(&w("DeepTree"))),
        ),
    ];
    for (constructor, expected) in constructors {
        let name = w(constructor);
        assert!(ctx.is_constructor(&name));
        checks(&ctx, &g(&name), &expected);
    }
    // A non-uniform application of a family is a different type, not a rewriting of it.
    assert!(!defeq_for_test(&ctx, &ptree(nat()), &ptree(g(&n("Bool")))));
}

#[test]
fn original_ptree_recursors_are_leans_generated_nested_recursors() {
    let (ctx, _) = original();
    for (suffix, major_is_list) in [("rec", false), ("rec_1", true)] {
        let name = w(&format!("PTree.{suffix}"));
        let (levels, ty) = poly(&ctx, &name);
        assert_eq!(
            levels,
            vec![n("u")],
            "{suffix} keeps Lean's single motive universe"
        );
        let expected = ptree_recursor(major_is_list);
        assert!(
            defeq_for_test(&ctx, &ty, &expected),
            "{suffix}: {ty}\nexpected {expected}"
        );
        // The other recursor's telescope is a different recursor.
        assert!(!defeq_for_test(&ctx, &ty, &ptree_recursor(!major_is_list)));
        let usage = axiom_dependencies_of(&ctx, &name).unwrap();
        assert!(usage.axioms.is_empty() && usage.unresolved.is_empty());
    }
    // The parameter stays a binder of the recursor: an instance at Nat is the recursor at Nat,
    // and the recursor cannot be applied to a non-type.
    let at = |a: Term| {
        app(
            Term::Const {
                name: w("PTree.rec"),
                levels: vec![Universe::Type(0)],
            },
            a,
        )
    };
    infer_type(&ctx, &at(nat())).expect("the recursor takes its parameter first");
    assert!(infer_type(&ctx, &at(lit(3))).is_err());
    assert!(recheck(&ctx, &at(lit(3))).is_err());
}

#[test]
fn original_nested_recursors_follow_leans_discovery_order() {
    let (ctx, _) = original();
    // PairTree: `List (Prod α (PairTree α))` is discovered before the `Prod α (PairTree α)`
    // inside it, so it owns motive_2, and Prod owns motive_3.
    let pair = || app(g(&w("PairTree")), v("α"));
    let (_, ty) = poly(&ctx, &w("PairTree.rec"));
    let domains = leading_domains(ty, &["α", "motive_1", "motive_2", "motive_3"]);
    let expected = [
        ty0(),
        pi("t", pair(), motive_sort()),
        pi("t", list(prod(v("α"), pair())), motive_sort()),
        pi("t", prod(v("α"), pair()), motive_sort()),
    ];
    let mut local = ctx.clone();
    for (i, (actual, want)) in domains.iter().zip(expected.iter()).enumerate() {
        assert!(
            defeq_for_test(&local, actual, want),
            "PairTree.rec binder {i}: {actual}"
        );
        if i == 0 {
            local = local.extend("α", ty0());
        }
    }
    for (suffix, carrier) in [
        ("rec", pair()),
        ("rec_1", list(prod(v("α"), pair()))),
        ("rec_2", prod(v("α"), pair())),
    ] {
        let (_, ty) = poly(&ctx, &w(&format!("PairTree.{suffix}")));
        let domains = leading_domains(
            ty,
            &["α", "m1", "m2", "m3", "node", "nil", "cons", "mk", "t"],
        );
        assert!(
            defeq_for_test(&local, &domains[8], &carrier),
            "PairTree.{suffix} major is {carrier}"
        );
    }
    // DeepTree: no parameters, the same outer-first order through a container argument.
    let deep = || g(&w("DeepTree"));
    for (suffix, carrier) in [
        ("rec", deep()),
        ("rec_1", list(list(deep()))),
        ("rec_2", list(deep())),
    ] {
        let (_, ty) = poly(&ctx, &w(&format!("DeepTree.{suffix}")));
        // Three motives, five minors (node; nil and cons twice), then the major premise.
        let domains = leading_domains(
            ty,
            &[
                "m1", "m2", "m3", "node", "nil", "cons", "nil'", "cons'", "t",
            ],
        );
        assert!(
            defeq_for_test(&ctx, &domains[8], &carrier),
            "DeepTree.{suffix} major: {}",
            domains[8]
        );
        assert!(defeq_for_test(
            &ctx,
            &domains[1],
            &pi("t", list(list(deep())), motive_sort())
        ));
        assert!(defeq_for_test(
            &ctx,
            &domains[2],
            &pi("t", list(deep()), motive_sort())
        ));
    }
}

fn computes(ctx: &Context, value: Term, expected: u64) {
    checks(ctx, &value, &nat());
    let normal = normalize_checked(ctx, &value).expect("the original computation normalizes");
    assert!(
        defeq_for_test(ctx, &normal, &lit(expected)),
        "{value} computes {normal}, not {expected}"
    );
    let eq = |rhs: Term| {
        apps(
            Term::Const {
                name: n("Eq"),
                levels: vec![Universe::Type(0)],
            },
            vec![nat(), value.clone(), rhs],
        )
    };
    let refl = apps(
        Term::Const {
            name: n("Eq.refl"),
            levels: vec![Universe::Type(0)],
        },
        vec![nat(), lit(expected)],
    );
    checks(ctx, &refl, &eq(lit(expected)));
    // The wrong value is refused by both kernels: the recursor really computes.
    let wrong = Term::Let {
        name: "claimed".into(),
        ty: Box::new(eq(lit(expected + 1))),
        value: Box::new(refl),
        body: Box::new(v("claimed")),
    };
    assert!(matches!(
        infer_type(ctx, &wrong),
        Err(KernelError::TypeMismatch { .. })
    ));
    assert!(matches!(recheck(ctx, &wrong), Err(ReCheckError::Ill(_))));
}

#[test]
fn original_nested_computations_are_checked_by_both_kernels() {
    let (ctx, report) = original();
    computes(&ctx, apps(g(&w("count")), vec![nat(), g(&w("sample"))]), 4);
    computes(&ctx, app(g(&w("labels")), g(&w("sample"))), 10);
    computes(
        &ctx,
        apps(g(&w("finSum")), vec![lit(5), g(&w("finSample"))]),
        7,
    );
    computes(
        &ctx,
        apps(g(&w("pairCount")), vec![nat(), g(&w("pairSample"))]),
        4,
    );
    computes(&ctx, app(g(&w("deepCount")), g(&w("deepSample"))), 5);
    // Lean's own `rfl` theorems are imported and checked with exactly Lean's axiom sets
    // (`#print axioms` under Lean 4.14.0): `finSum_sample` inherits `propext` from the `Fin`
    // literal `4 : Fin 5`; the others depend on no axiom.
    for (theorem, lean_axioms) in [
        ("count_sample", vec![]),
        ("labels_sample", vec![]),
        ("finSum_sample", vec![n("propext")]),
        ("pairCount_sample", vec![]),
        ("deepCount_sample", vec![]),
    ] {
        let row = report
            .declarations
            .iter()
            .find(|d| d.name == w(theorem))
            .unwrap_or_else(|| panic!("{theorem} is audited"));
        assert_eq!(row.kind, "thm");
        assert_eq!(row.axioms, lean_axioms, "{theorem}");
    }
}

// ==== Corruption controls: each changes ONE fact of the original export ====

struct Export {
    rows: Vec<Value>,
}

impl Export {
    fn original() -> Self {
        Export {
            rows: INPUT
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect(),
        }
    }
    fn text(&self) -> String {
        self.rows.iter().map(|r| format!("{r}\n")).collect()
    }
    fn name_id(&self, dotted: &str) -> u64 {
        let mut id = 0;
        for part in dotted.split('.') {
            id = self
                .rows
                .iter()
                .find(|r| r["str"]["pre"] == id && r["str"]["str"] == part)
                .unwrap_or_else(|| panic!("name {dotted}"))["in"]
                .as_u64()
                .unwrap();
        }
        id
    }
    fn block_position(&self, family: &str) -> usize {
        let id = self.name_id(family);
        self.rows
            .iter()
            .position(|r| r["inductive"]["types"][0]["name"] == id)
            .unwrap()
    }
    fn block(&mut self, family: &str) -> &mut Value {
        let at = self.block_position(family);
        &mut self.rows[at]["inductive"]
    }
    fn expr(&self, id: &Value) -> &Value {
        self.rows.iter().find(|r| r["ie"] == *id).unwrap()
    }
    /// Insert a new expression before the family's block and return its index.
    fn add(&mut self, family: &str, kind: &str, body: Value) -> Value {
        let next = self
            .rows
            .iter()
            .filter_map(|r| r["ie"].as_u64())
            .max()
            .unwrap()
            + 1;
        let at = self.block_position(family);
        self.rows.insert(at, json!({ "ie": next, kind: body }));
        json!(next)
    }
    fn find_expr(&self, pred: impl Fn(&Value) -> bool) -> Value {
        self.rows
            .iter()
            .find(|r| r.get("ie").is_some() && pred(r))
            .unwrap()["ie"]
            .clone()
    }
}

fn refused(export: &Export, code: i32, diagnostic: DiagnosticCode, what: &str) -> String {
    let verdict = adapter::check_str(&export.text()).expect_err(what);
    assert_eq!(verdict.code(), code, "{what}: {verdict:?}");
    assert_eq!(verdict.reason().code(), diagnostic, "{what}: {verdict:?}");
    assert!(matches!(
        (&verdict, code),
        (Verdict::Reject(_), 1) | (Verdict::Decline(_), 2)
    ));
    verdict.reason().message().to_string()
}

#[test]
fn the_unchanged_original_is_accepted_by_the_same_door_the_controls_use() {
    adapter::check_str(&Export::original().text()).expect("control baseline");
}

#[test]
fn erasing_the_parameter_is_rejected() {
    let mut e = Export::original();
    e.block("LF_W03.PTree")["types"][0]["numParams"] = json!(0);
    refused(&e, 1, DiagnosticCode::InvalidProof, "numParams 1 → 0");
}

#[test]
fn inventing_an_index_is_rejected() {
    let mut e = Export::original();
    e.block("LF_W03.PTree")["types"][0]["numIndices"] = json!(1);
    refused(&e, 1, DiagnosticCode::InvalidProof, "numIndices 0 → 1");
    let mut e = Export::original();
    let recs = e.block("LF_W03.PTree")["recs"].as_array_mut().unwrap();
    recs[0]["numIndices"] = json!(1);
    refused(
        &e,
        1,
        DiagnosticCode::InvalidProof,
        "recursor numIndices 0 → 1",
    );
}

#[test]
fn a_recursor_telescope_at_a_different_parameter_count_is_rejected() {
    let mut e = Export::original();
    e.block("LF_W03.PTree")["recs"][1]["numParams"] = json!(0);
    refused(&e, 1, DiagnosticCode::InvalidProof, "rec_1 numParams 1 → 0");
}

/// Rebuild `PTree.node`'s type `Π{α}. Π(x:α). Π(c:List (PTree α)). PTree α` with a new field
/// domain for `c`, copying the binder path so no shared expression is mutated.
fn replace_children_field(e: &mut Export, field: impl FnOnce(&mut Export) -> Value) {
    let ctor_ty = e.block("LF_W03.PTree")["ctors"][0]["type"].clone();
    let alpha = e.expr(&ctor_ty)["forallE"].clone();
    let x = e.expr(&alpha["body"])["forallE"].clone();
    let c = e.expr(&x["body"])["forallE"].clone();
    let domain = field(e);
    let mut c2 = c.clone();
    c2["type"] = domain;
    let c2 = e.add("LF_W03.PTree", "forallE", c2);
    let mut x2 = x.clone();
    x2["body"] = c2;
    let x2 = e.add("LF_W03.PTree", "forallE", x2);
    let mut alpha2 = alpha.clone();
    alpha2["body"] = x2;
    let alpha2 = e.add("LF_W03.PTree", "forallE", alpha2);
    e.block("LF_W03.PTree")["ctors"][0]["type"] = alpha2;
}

#[test]
fn a_non_uniform_nested_occurrence_is_rejected_as_lean_rejects_it() {
    // Lean 4.14.0 refuses `node : α → List (PTree (List α)) → PTree α` with "inductive datatype
    // parameter mismatch: List α, expected α". The same change made to the original export is
    // rejected as an invalid declaration, not declined as an unsupported shape.
    let mut e = Export::original();
    replace_children_field(&mut e, |e| {
        let ptree = e.name_id("LF_W03.PTree");
        let tree_const =
            e.find_expr(|r| r["const"]["name"] == ptree && r["const"]["us"] == json!([]));
        let list_id = e.name_id("List");
        let list0 = e.find_expr(|r| {
            r["const"]["name"] == list_id && r["const"]["us"].as_array().map(|u| u.len()) == Some(1)
        });
        let alpha = e.find_expr(|r| r["bvar"] == 1);
        let list_alpha = e.add("LF_W03.PTree", "app", json!({"fn": list0, "arg": alpha}));
        let at_list = e.add(
            "LF_W03.PTree",
            "app",
            json!({"fn": tree_const, "arg": list_alpha}),
        );
        e.add("LF_W03.PTree", "app", json!({"fn": list0, "arg": at_list}))
    });
    let message = refused(
        &e,
        1,
        DiagnosticCode::InvalidProof,
        "PTree (List α) inside PTree α",
    );
    assert!(!message.contains("nested-compile: container"), "{message}");
}

#[test]
fn a_negative_nested_occurrence_is_rejected_as_lean_rejects_it() {
    // Lean's kernel: "arg #2 of 'Adv2.Neg.node' has a non positive occurrence".
    let mut e = Export::original();
    let ctor_ty = e.block("LF_W03.PTree")["ctors"][0]["type"].clone();
    let alpha = e.expr(&ctor_ty)["forallE"].clone();
    let x = e.expr(&alpha["body"])["forallE"].clone();
    let ptree = e.name_id("LF_W03.PTree");
    let list_id = e.name_id("List");
    let tree_const = e.find_expr(|r| r["const"]["name"] == ptree && r["const"]["us"] == json!([]));
    let list0 = e.find_expr(|r| {
        r["const"]["name"] == list_id && r["const"]["us"].as_array().map(|u| u.len()) == Some(1)
    });
    let b0 = e.find_expr(|r| r["bvar"] == 0);
    let b1 = e.find_expr(|r| r["bvar"] == 1);
    let tree_alpha = e.add("LF_W03.PTree", "app", json!({"fn": tree_const, "arg": b0}));
    let children = e.add(
        "LF_W03.PTree",
        "app",
        json!({"fn": list0, "arg": tree_alpha}),
    );
    let negative = e.add(
        "LF_W03.PTree",
        "forallE",
        json!({"name": 0, "type": children, "body": b1, "binderInfo": "default"}),
    );
    let mut x2 = x.clone();
    x2["type"] = negative;
    let x2 = e.add("LF_W03.PTree", "forallE", x2);
    let mut alpha2 = alpha.clone();
    alpha2["body"] = x2;
    let alpha2 = e.add("LF_W03.PTree", "forallE", alpha2);
    e.block("LF_W03.PTree")["ctors"][0]["type"] = alpha2;
    refused(
        &e,
        1,
        DiagnosticCode::InvalidProof,
        "(List (PTree α) → α) field",
    );
}

#[test]
fn a_nested_container_at_the_wrong_universe_is_rejected() {
    // `List.{1} (PTree α)`: Lean has no cumulativity, and neither does this door.
    let mut e = Export::original();
    replace_children_field(&mut e, |e| {
        let list_id = e.name_id("List");
        let one = e.rows.iter().find(|r| r["succ"] == 0).unwrap()["il"].clone();
        let list1 = e.add(
            "LF_W03.PTree",
            "const",
            json!({"name": list_id, "us": [one]}),
        );
        let c = {
            let ctor_ty = e.block("LF_W03.PTree")["ctors"][0]["type"].clone();
            let alpha = e.expr(&ctor_ty)["forallE"].clone();
            let x = e.expr(&alpha["body"])["forallE"].clone();
            e.expr(&x["body"])["forallE"].clone()
        };
        let tree_alpha = e.expr(&c["type"])["app"]["arg"].clone();
        e.add(
            "LF_W03.PTree",
            "app",
            json!({"fn": list1, "arg": tree_alpha}),
        )
    });
    refused(&e, 1, DiagnosticCode::InvalidProof, "List.{1} (PTree α)");
}

#[test]
fn altered_constructor_metadata_is_rejected() {
    let mut e = Export::original();
    e.block("LF_W03.PTree")["ctors"][0]["numFields"] = json!(3);
    refused(&e, 1, DiagnosticCode::InvalidProof, "numFields 2 → 3");
    let mut e = Export::original();
    e.block("LF_W03.PTree")["ctors"][0]["cidx"] = json!(1);
    refused(&e, 1, DiagnosticCode::InvalidProof, "cidx 0 → 1");
    let mut e = Export::original();
    e.block("LF_W03.PTree")["ctors"][0]["numParams"] = json!(0);
    refused(
        &e,
        1,
        DiagnosticCode::InvalidProof,
        "constructor numParams 1 → 0",
    );
}

#[test]
fn forged_nested_recursors_are_rejected() {
    let mut e = Export::original();
    e.block("LF_W03.PTree")["recs"][0]["numMotives"] = json!(3);
    refused(&e, 1, DiagnosticCode::InvalidProof, "numMotives 2 → 3");
    let mut e = Export::original();
    e.block("LF_W03.PTree")["recs"]
        .as_array_mut()
        .unwrap()
        .pop();
    refused(&e, 1, DiagnosticCode::InvalidProof, "rec_1 dropped");
    let mut e = Export::original();
    let block = e.block("LF_W03.PTree");
    let nil_rule = block["recs"][1]["rules"][0]["rhs"].clone();
    block["recs"][0]["rules"][0]["rhs"] = nil_rule;
    refused(
        &e,
        1,
        DiagnosticCode::InvalidProof,
        "node rule replaced by the nil rule",
    );
    let mut e = Export::original();
    let block = e.block("LF_W03.PTree");
    let rec_1_type = block["recs"][1]["type"].clone();
    block["recs"][0]["type"] = rec_1_type;
    refused(
        &e,
        1,
        DiagnosticCode::InvalidProof,
        "rec given rec_1's telescope",
    );
    let mut e = Export::original();
    e.block("LF_W03.PTree")["recs"][0]["k"] = json!(true);
    refused(&e, 1, DiagnosticCode::InvalidProof, "K flag");
}

// ==== The compiler itself, below the importer: capture and shadowing controls ====

fn data(c: &mut Context, name: &str, constructors: Vec<(&str, Term)>) {
    c.add_mutual_inductives(&[MutualInductive {
        name: name.into(),
        sort: ty0(),
        num_params: 0,
        constructors: constructors
            .into_iter()
            .map(|(n, t)| (n.to_string(), t))
            .collect(),
    }])
    .unwrap();
}

/// `List`, a one-value type `A`, a two-value type `B` and unary `N`, in an empty context.
fn containers() -> Context {
    let mut c = Context::new();
    c.add_mutual_inductives(&[MutualInductive {
        name: "List".into(),
        sort: pi("A", ty0(), ty0()),
        num_params: 1,
        constructors: vec![
            ("nil".into(), pi("A", ty0(), app(g("List"), v("A")))),
            (
                "cons".into(),
                pi(
                    "A",
                    ty0(),
                    pi(
                        "head",
                        v("A"),
                        pi("tail", app(g("List"), v("A")), app(g("List"), v("A"))),
                    ),
                ),
            ),
        ],
    }])
    .unwrap();
    data(&mut c, "A", vec![("a", g("A"))]);
    data(&mut c, "B", vec![("bt", g("B")), ("bf", g("B"))]);
    data(
        &mut c,
        "N",
        vec![("z", g("N")), ("s", pi("n", g("N"), g("N")))],
    );
    c
}

/// A two-parameter family whose source binder names are the compiler's own canonical and
/// instantiation names, in the WRONG order, with fields named like fresh instance names.
fn adversarial_names() -> (Context, NestedDecl) {
    let mut c = containers();
    let (p0, p1) = ("nested.parameter.1", "nested.parameter.0");
    let tree = |a: Term, b: Term| apps(g("T"), vec![a, b]);
    let decl = NestedDecl {
        name: "T".into(),
        sort: pi(p0, ty0(), pi(p1, ty0(), ty0())),
        constructors: vec![
            (
                "leaf".into(),
                pi(
                    p0,
                    ty0(),
                    pi(
                        p1,
                        ty0(),
                        pi("native.instance.0", v(p1), tree(v(p0), v(p1))),
                    ),
                ),
            ),
            (
                "node".into(),
                pi(
                    p0,
                    ty0(),
                    pi(
                        p1,
                        ty0(),
                        pi(
                            "native.instance.1",
                            v(p0),
                            pi(
                                "kids",
                                app(g("List"), tree(v(p0), v(p1))),
                                tree(v(p0), v(p1)),
                            ),
                        ),
                    ),
                ),
            ),
        ],
    };
    c.add_mutual_inductives(&[MutualInductive {
        name: decl.name.clone(),
        sort: decl.sort.clone(),
        num_params: 2,
        constructors: decl.constructors.clone(),
    }])
    .unwrap();
    (c, decl)
}

fn t_ab() -> Term {
    apps(g("T"), vec![g("A"), g("B")])
}
fn leaf_ab(b: &str) -> Term {
    apps(g("leaf"), vec![g("A"), g("B"), g(b)])
}
fn kids_ab() -> Term {
    apps(
        g("cons"),
        vec![
            t_ab(),
            leaf_ab("bt"),
            apps(
                g("cons"),
                vec![t_ab(), leaf_ab("bf"), app(g("nil"), t_ab())],
            ),
        ],
    )
}

#[test]
fn canonical_parameter_names_cannot_capture_or_permute_source_parameters() {
    let (mut c, decl) = adversarial_names();
    let recursors = derive_native_recursors_with_params(&c, &decl, 2).unwrap();
    assert_eq!(recursors.len(), 2);
    for (i, rec) in recursors.into_iter().enumerate() {
        assert_eq!(rec.parameters.len(), 2);
        assert!(matches!(double_check(&c, &rec.body), DoubleCheck::Agreed));
        c.declare_universe_poly(&format!("rec{i}"), rec.levels, rec.ty, rec.body)
            .unwrap();
    }
    // The first parameter is the node label's type, the second the leaf's: `T A B`, never `T B A`.
    checks(&c, &leaf_ab("bt"), &t_ab());
    let tree = apps(g("node"), vec![g("A"), g("B"), g("a"), kids_ab()]);
    checks(&c, &tree, &t_ab());
    assert!(infer_type(&c, &apps(g("leaf"), vec![g("A"), g("B"), g("a")])).is_err());
    // Length of the top-level child list, through both motives and all four minors.
    let motives = vec![
        lam("t", t_ab(), g("N")),
        lam("l", app(g("List"), t_ab()), g("N")),
    ];
    let minors = vec![
        lam("b", g("B"), g("z")),
        lam(
            "x",
            g("A"),
            lam("k", app(g("List"), t_ab()), lam("ih", g("N"), v("ih"))),
        ),
        g("z"),
        lam(
            "h",
            t_ab(),
            lam(
                "tl",
                app(g("List"), t_ab()),
                lam("ih1", g("N"), lam("ih2", g("N"), app(g("s"), v("ih2")))),
            ),
        ),
    ];
    let mut args = vec![g("A"), g("B")];
    args.extend(motives);
    args.extend(minors);
    args.push(tree);
    let call = apps(
        Term::Const {
            name: "rec0".into(),
            levels: vec![Universe::Type(0); 2],
        },
        args,
    );
    checks(&c, &call, &g("N"));
    assert!(matches!(double_check(&c, &call), DoubleCheck::Agreed));
    assert_eq!(
        normalize_checked(&c, &call).unwrap(),
        app(g("s"), app(g("s"), g("z")))
    );
}

#[test]
fn parameterized_conversion_isos_check_and_round_trip() {
    let (c, decl) = adversarial_names();
    let compiled = compile_nested_with_params(&c, &decl, 2).unwrap();
    let mut scratch = c.clone();
    scratch.add_mutual_inductives(&compiled.block).unwrap();
    for member in &compiled.block {
        assert_eq!(
            member.num_params, 2,
            "{} shares both parameters",
            member.name
        );
    }
    for (name, ty, body) in &compiled.isos {
        let inferred = infer_type(&scratch, body).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            defeq_for_test(&scratch, &inferred, ty),
            "{name}: {inferred} ≠ {ty}"
        );
        assert!(
            matches!(double_check(&scratch, body), DoubleCheck::Agreed),
            "{name}"
        );
        scratch.add_definition(name.clone(), ty.clone(), body.clone());
    }
    let iso = &compiled.iso_names[0];
    let round = app(
        apps(g(&iso.to_generic), vec![g("A"), g("B")]),
        app(apps(g(&iso.from_generic), vec![g("A"), g("B")]), kids_ab()),
    );
    assert_eq!(
        normalize_checked(&scratch, &round).unwrap(),
        normalize_checked(&scratch, &kids_ab()).unwrap()
    );
}

#[test]
fn a_parameter_count_the_sort_cannot_supply_is_refused() {
    let (c, decl) = adversarial_names();
    assert!(matches!(
        compile_nested_with_params(&c, &decl, 3),
        Err(KernelError::CertificationError(_))
    ));
    assert!(derive_native_recursors_with_params(&c, &decl, 1).is_err());
}
