//! Nesting through a carried POSITIVE function field, decided as Lean 4.14 decides it, through both
//! doors of the kernel: the Lean reader and the `NestedDecl` compiler.
//!
//! `C`'s `cons` carries `Nat → A` beside the element, and `T` nests through `C T`. In the block Lean
//! reduces the nesting to, that field is `Nat → T`: a strictly positive, reflexive occurrence whose
//! induction hypothesis is pointwise, `(a : Nat) → motive_1 (f a)`. The original witness
//! `fixtures/lean_semantics/LF_NestedFunctionField.lean` (Lean 4.14.0, pinned lean4export) pins
//! Lean's recursors, `brecOn` and the refusal of every neighbour with `#guard_msgs`, so the file
//! compiles only while Lean decides each of them exactly so.
//!
//! The neighbours stay refused: the element in a domain (`A → Nat`), in the domain of a domain
//! (`(A → Nat) → A`), a nesting in the domain of the inductive's own field, and a field naming a
//! type that is not declared.
use logicaffeine_kernel::interface::lean::pipeline::{check_export, LeanEvidence, Limits};
use logicaffeine_kernel::interface::lean::receipt::StageStatus;
use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode, Verdict};
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    axiom_dependencies_of, defeq_for_test, derive_recursor, double_check, infer_type, is_subtype,
    recheck, Context, DoubleCheck, KernelError, NestedDecl, NestedInfo, ReCheckError, Term,
    Universe,
};
use serde_json::{json, Value};
use std::collections::HashMap;

// ---- The NestedDecl compiler ------------------------------------------------------------------

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
}
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn lam(p: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// A list-like container `name : Type → Type` whose cons carries `carried(A)` beside the element:
/// `nameCons : Π(A). A → carried(A) → name A → name A`.
fn container(ctx: &mut Context, name: &str, carried: impl Fn(Term) -> Term) {
    ctx.add_inductive(name, arrow(ty0(), ty0()));
    ctx.add_constructor(
        &format!("{name}Nil"),
        name,
        pi("A", ty0(), app(g(name), v("A"))),
    );
    ctx.add_constructor(
        &format!("{name}Cons"),
        name,
        pi(
            "A",
            ty0(),
            arrow(
                v("A"),
                arrow(
                    carried(v("A")),
                    arrow(app(g(name), v("A")), app(g(name), v("A"))),
                ),
            ),
        ),
    );
}
/// `tree := tree_node : container tree → tree`.
fn nested_through(tree: &str, container: &str) -> NestedDecl {
    NestedDecl {
        name: tree.to_string(),
        sort: ty0(),
        constructors: vec![(
            format!("{tree}_node"),
            arrow(app(g(container), g(tree)), g(tree)),
        )],
    }
}
fn registers_nothing(ctx: &Context, tree: &str) {
    assert!(
        !ctx.is_inductive(tree)
            && ctx.get_global(&format!("{tree}_node")).is_none()
            && ctx.get_global(&format!("{tree}$TList")).is_none(),
        "a refused nesting of `{tree}` registers nothing"
    );
}
fn both_kernels_certify(ctx: &Context, info: &NestedInfo, tree: &str) {
    for iso in &info.isos {
        for name in [&iso.to_generic, &iso.from_generic] {
            assert_eq!(
                double_check(ctx, &g(name)),
                DoubleCheck::Agreed,
                "both kernels certify iso {name}"
            );
        }
    }
    let (_, recursor) = derive_recursor(ctx, tree).expect("the block's recursor derives");
    assert_eq!(
        double_check(ctx, &recursor),
        DoubleCheck::Agreed,
        "both kernels certify {tree}'s recursor"
    );
}
/// `a ≡ b` in the primary kernel, and `(λh : a = b. h) (refl a)` typed by the independent one.
fn both_kernels_equate(ctx: &Context, ty: &Term, a: &Term, b: &Term) {
    assert!(
        defeq_for_test(ctx, a, b),
        "the primary kernel equates {a} and {b}"
    );
    let eq = |x: &Term, y: &Term| apps(g("Eq"), &[ty.clone(), x.clone(), y.clone()]);
    let witness = app(
        lam("h", eq(a, b), v("h")),
        apps(g("refl"), &[ty.clone(), a.clone()]),
    );
    recheck(ctx, &witness)
        .unwrap_or_else(|e| panic!("the independent kernel equates {a} and {b}: {e:?}"));
}

/// Lean accepts `inductive TFn | node : CFn TFn → TFn` over `cons : α → (Nat → α) → CFn α → CFn α`
/// (the witness). The field rides into the specialised sibling as `Nat → FnTree` — the element's
/// own slot under a binder — and converts as the identity, since nothing under the binder nests.
#[test]
fn a_carried_positive_function_field_nests_as_lean_nests_it() {
    let mut ctx = std_ctx();
    container(&mut ctx, "FnList", |a| arrow(g("Nat"), a));
    let info = ctx
        .add_nested_inductive(&nested_through("FnTree", "FnList"))
        .unwrap_or_else(|e| {
            panic!("`Nat → A` is strictly positive, and Lean nests through it: {e}")
        });
    assert_eq!(
        info.siblings,
        vec!["FnTree$FnList".to_string()],
        "one container level, one sibling"
    );
    let sibling = infer_type(&ctx, &g("FnTree$FnList_FnListCons")).unwrap();
    let expected = arrow(
        g("FnTree"),
        arrow(
            arrow(g("Nat"), g("FnTree")),
            arrow(g("FnTree$FnList"), g("FnTree$FnList")),
        ),
    );
    assert!(
        is_subtype(&ctx, &sibling, &expected) && is_subtype(&ctx, &expected, &sibling),
        "the sibling carries the function field in its own slot: {sibling}"
    );
    both_kernels_certify(&ctx, &info, "FnTree");

    let iso = &info.isos[0];
    let leaf = app(g("FnTree_node"), g("FnTree$FnList_FnListNil"));
    let fanned = lam("n", g("Nat"), leaf.clone());
    let sib = apps(
        g("FnTree$FnList_FnListCons"),
        &[leaf.clone(), fanned.clone(), g("FnTree$FnList_FnListNil")],
    );
    let gen = apps(
        g("FnListCons"),
        &[
            g("FnTree"),
            leaf.clone(),
            fanned,
            app(g("FnListNil"), g("FnTree")),
        ],
    );
    let sibling_ty = g("FnTree$FnList");
    let generic_ty = app(g("FnList"), g("FnTree"));
    both_kernels_equate(
        &ctx,
        &generic_ty,
        &app(g(&iso.to_generic), sib.clone()),
        &gen,
    );
    both_kernels_equate(
        &ctx,
        &sibling_ty,
        &app(g(&iso.from_generic), gen.clone()),
        &sib,
    );
    both_kernels_equate(
        &ctx,
        &sibling_ty,
        &app(g(&iso.from_generic), app(g(&iso.to_generic), sib.clone())),
        &sib,
    );
    both_kernels_equate(
        &ctx,
        &generic_ty,
        &app(g(&iso.to_generic), app(g(&iso.from_generic), gen.clone())),
        &gen,
    );
}

/// Lean nests `TFnList` through `cons : α → (Nat → List α) → CFnList α → CFnList α` (the witness
/// source): the nesting under the binder is specialised too, and its conversion is pointwise —
/// `λf. λx. to (f x)` — which has to invert like any other.
#[test]
fn a_function_field_whose_codomain_nests_converts_pointwise() {
    let mut ctx = std_ctx();
    container(&mut ctx, "FnTList", |a| arrow(g("Nat"), app(g("TList"), a)));
    let info = ctx
        .add_nested_inductive(&nested_through("LTree", "FnTList"))
        .unwrap_or_else(|e| panic!("Lean nests through `Nat → List α`: {e}"));
    assert_eq!(
        info.siblings,
        vec!["LTree$TList".to_string(), "LTree$FnTList".to_string()],
        "the inner nesting is specialised first"
    );
    let sibling = infer_type(&ctx, &g("LTree$FnTList_FnTListCons")).unwrap();
    let expected = arrow(
        g("LTree"),
        arrow(
            arrow(g("Nat"), g("LTree$TList")),
            arrow(g("LTree$FnTList"), g("LTree$FnTList")),
        ),
    );
    assert!(
        is_subtype(&ctx, &sibling, &expected) && is_subtype(&ctx, &expected, &sibling),
        "the function field returns the specialised inner sibling: {sibling}"
    );
    both_kernels_certify(&ctx, &info, "LTree");

    let outer = info
        .isos
        .iter()
        .find(|i| i.sibling == "LTree$FnTList")
        .unwrap();
    let leaf = app(g("LTree_node"), g("LTree$FnTList_FnTListNil"));
    let one = apps(
        g("LTree$TList_TCons"),
        &[leaf.clone(), g("LTree$TList_TNil")],
    );
    let sib = apps(
        g("LTree$FnTList_FnTListCons"),
        &[
            leaf.clone(),
            lam("n", g("Nat"), one),
            g("LTree$FnTList_FnTListNil"),
        ],
    );
    let one_generic = apps(
        g("TCons"),
        &[g("LTree"), leaf.clone(), app(g("TNil"), g("LTree"))],
    );
    let gen = apps(
        g("FnTListCons"),
        &[
            g("LTree"),
            leaf,
            lam("n", g("Nat"), one_generic),
            app(g("FnTListNil"), g("LTree")),
        ],
    );
    let sibling_ty = g("LTree$FnTList");
    let generic_ty = app(g("FnTList"), g("LTree"));
    both_kernels_equate(
        &ctx,
        &generic_ty,
        &app(g(&outer.to_generic), sib.clone()),
        &gen,
    );
    both_kernels_equate(
        &ctx,
        &sibling_ty,
        &app(g(&outer.from_generic), gen.clone()),
        &sib,
    );
    both_kernels_equate(
        &ctx,
        &sibling_ty,
        &app(
            g(&outer.from_generic),
            app(g(&outer.to_generic), sib.clone()),
        ),
        &sib,
    );
    both_kernels_equate(
        &ctx,
        &generic_ty,
        &app(
            g(&outer.to_generic),
            app(g(&outer.from_generic), gen.clone()),
        ),
        &gen,
    );
}

/// Lean nests `TCurry` through `cons : (Nat → Bool → α) → CCurry α → CCurry α` (the witness source).
#[test]
fn a_curried_function_field_nests() {
    let mut ctx = std_ctx();
    container(&mut ctx, "CurList", |a| {
        arrow(g("Nat"), arrow(g("Bool"), a))
    });
    let info = ctx
        .add_nested_inductive(&nested_through("CurTree", "CurList"))
        .unwrap_or_else(|e| panic!("Lean nests through `Nat → Bool → α`: {e}"));
    assert_eq!(info.siblings, vec!["CurTree$CurList".to_string()]);
    both_kernels_certify(&ctx, &info, "CurTree");
    let iso = &info.isos[0];
    let leaf = app(g("CurTree_node"), g("CurTree$CurList_CurListNil"));
    let field = lam("n", g("Nat"), lam("b", g("Bool"), leaf.clone()));
    let sib = apps(
        g("CurTree$CurList_CurListCons"),
        &[leaf.clone(), field.clone(), g("CurTree$CurList_CurListNil")],
    );
    let gen = apps(
        g("CurListCons"),
        &[
            g("CurTree"),
            leaf,
            field,
            app(g("CurListNil"), g("CurTree")),
        ],
    );
    both_kernels_equate(
        &ctx,
        &app(g("CurList"), g("CurTree")),
        &app(g(&iso.to_generic), sib.clone()),
        &gen,
    );
    both_kernels_equate(
        &ctx,
        &g("CurTree$CurList"),
        &app(g(&iso.from_generic), gen),
        &sib,
    );
}

/// Lean nests `inductive GTree | node : (Nat → List GTree) → GTree` (the witness source): the
/// inductive's OWN field nests under a binder, and is specialised there.
#[test]
fn the_inductives_own_field_nests_under_a_binder() {
    let mut ctx = std_ctx();
    let decl = NestedDecl {
        name: "GTree".to_string(),
        sort: ty0(),
        constructors: vec![(
            "GTree_node".to_string(),
            arrow(arrow(g("Nat"), app(g("TList"), g("GTree"))), g("GTree")),
        )],
    };
    let info = ctx
        .add_nested_inductive(&decl)
        .unwrap_or_else(|e| panic!("Lean nests `Nat → List GTree`: {e}"));
    assert_eq!(info.siblings, vec!["GTree$TList".to_string()]);
    let node = infer_type(&ctx, &g("GTree_node")).unwrap();
    let expected = arrow(arrow(g("Nat"), g("GTree$TList")), g("GTree"));
    assert!(
        is_subtype(&ctx, &node, &expected) && is_subtype(&ctx, &expected, &node),
        "the own field takes the specialised sibling under its binder: {node}"
    );
    both_kernels_certify(&ctx, &info, "GTree");
}

/// Each neighbour of the positive function field, refused as Lean 4.14 refuses it on this box
/// (`#guard_msgs` in the witness source): "(kernel) arg #2 of '_nested.LF_NF.CNeg_1.cons' has a non
/// positive occurrence", the same for `CDbl_1.cons` and for `GNeg.node`, and "unknown identifier
/// 'NotYetDeclared'". Nothing is registered by any of them.
#[test]
fn the_neighbours_of_a_positive_function_field_are_refused_as_lean_refuses_them() {
    for (tree, carried) in [
        (
            "NegTree",
            (|a: Term| arrow(a, g("Nat"))) as fn(Term) -> Term,
        ),
        ("DblTree", |a: Term| arrow(arrow(a.clone(), g("Nat")), a)),
    ] {
        let list = format!("{tree}List");
        let mut ctx = std_ctx();
        container(&mut ctx, &list, carried);
        match ctx.add_nested_inductive(&nested_through(tree, &list)) {
            Err(KernelError::PositivityViolation { .. }) => {}
            other => panic!("{tree}: an element in a domain is a positivity violation: {other:?}"),
        }
        registers_nothing(&ctx, tree);
    }

    let mut pending = std_ctx();
    container(&mut pending, "PendList", |_| {
        arrow(g("Nat"), g("NotYetDeclared"))
    });
    assert!(
        pending
            .add_nested_inductive(&nested_through("PendTree", "PendList"))
            .is_err(),
        "a function field into an undeclared type is refused"
    );
    registers_nothing(&pending, "PendTree");

    let mut own = std_ctx();
    let decl = NestedDecl {
        name: "GNeg".to_string(),
        sort: ty0(),
        constructors: vec![(
            "GNeg_node".to_string(),
            arrow(arrow(app(g("TList"), g("GNeg")), g("Nat")), g("GNeg")),
        )],
    };
    match own.add_nested_inductive(&decl) {
        Err(KernelError::PositivityViolation { .. }) => {}
        other => {
            panic!("a nesting in the domain of the own field is a positivity violation: {other:?}")
        }
    }
    registers_nothing(&own, "GNeg");
}

// ---- The Lean reader, on Lean's own export ----------------------------------------------------

const SOURCE: &str = include_str!("fixtures/lean_semantics/LF_NestedFunctionField.lean");
const SOURCE_SHA: &str = "2a70d62280bd154844d805ee955d8d734ba440b458fb939c4741e0eea9f27b78";
const EXPORT: &str = include_str!("fixtures/lean_semantics/LF_NestedFunctionField.ndjson");
const PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_NestedFunctionField.provenance.json");
const EXPORT_SHA: &str = "37c7ab20a6e41d9e9328d2aa7629a0f16ae78df1118f4e402c4c5859f3679b18";

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}

/// Lean's export, read as Lean wrote it: names, levels and expressions by index.
struct Lean {
    rows: Vec<Value>,
    names: HashMap<u64, Value>,
    levels: HashMap<u64, Value>,
    exprs: HashMap<u64, Value>,
}

impl Lean {
    fn parse(input: &str) -> Lean {
        let rows: Vec<Value> = input
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let index = |key: &str| {
            rows.iter()
                .filter_map(|r| Some((r.get(key)?.as_u64()?, r.clone())))
                .collect()
        };
        Lean {
            names: index("in"),
            levels: index("il"),
            exprs: index("ie"),
            rows,
        }
    }
    fn name(&self, id: &Value) -> String {
        let id = id.as_u64().unwrap();
        if id == 0 {
            return String::new();
        }
        let r = &self.names[&id];
        let (pre, last) = match (r.get("str"), r.get("num")) {
            (Some(s), _) => (&s["pre"], s["str"].as_str().unwrap().to_string()),
            (_, Some(k)) => (&k["pre"], k["i"].to_string()),
            _ => panic!("name record {r}"),
        };
        match self.name(pre) {
            p if p.is_empty() => last,
            p => format!("{p}.{last}"),
        }
    }
    fn level(&self, id: &Value) -> String {
        let id = id.as_u64().unwrap();
        if id == 0 {
            return "0".into();
        }
        let r = &self.levels[&id];
        if let Some(l) = r.get("succ") {
            return format!("{}+1", self.level(l));
        }
        if let Some(p) = r.get("param") {
            return self.name(p);
        }
        for kind in ["max", "imax"] {
            if let Some(ab) = r.get(kind) {
                return format!("{kind}({},{})", self.level(&ab[0]), self.level(&ab[1]));
            }
        }
        panic!("level record {r}")
    }
    /// An expression in de Bruijn form, binder names left out: they are Lean's hygienic spellings,
    /// which depend on where the source wrote them rather than on what the term is.
    fn expr(&self, id: &Value) -> String {
        let r = &self.exprs[&id.as_u64().unwrap()];
        if let Some(k) = r.get("bvar") {
            return format!("#{k}");
        }
        if let Some(l) = r.get("sort") {
            return format!("Sort {}", self.level(l));
        }
        if let Some(c) = r.get("const") {
            let us: Vec<String> = c["us"]
                .as_array()
                .unwrap()
                .iter()
                .map(|u| self.level(u))
                .collect();
            return match us.is_empty() {
                true => self.name(&c["name"]),
                false => format!("{}.{{{}}}", self.name(&c["name"]), us.join(",")),
            };
        }
        if r.get("app").is_some() {
            let mut args = Vec::new();
            let mut head = r;
            while let Some(a) = head.get("app") {
                args.push(self.expr(&a["arg"]));
                head = &self.exprs[&a["fn"].as_u64().unwrap()];
            }
            args.reverse();
            let head = self.expr(&head["ie"]);
            return format!("({head} {})", args.join(" "));
        }
        if let Some(b) = r.get("lam") {
            return format!(
                "(fun {} => {})",
                self.expr(&b["type"]),
                self.expr(&b["body"])
            );
        }
        if let Some(b) = r.get("forallE") {
            return format!("(Π {}, {})", self.expr(&b["type"]), self.expr(&b["body"]));
        }
        if let Some(k) = r.get("natVal") {
            return k.as_str().unwrap().to_string();
        }
        if let Some(m) = r.get("mdata") {
            return self.expr(&m["expr"]);
        }
        if let Some(p) = r.get("proj") {
            return format!(
                "{}.{}({})",
                self.name(&p["typeName"]),
                p["idx"],
                self.expr(&p["struct"])
            );
        }
        panic!("expression record {r}")
    }
    /// The body under `count` binders of `kind` (`lam` or `forallE`).
    fn under(&self, id: &Value, kind: &str, count: usize) -> Value {
        (0..count).fold(id.clone(), |at, _| {
            self.exprs[&at.as_u64().unwrap()][kind]["body"].clone()
        })
    }
    /// The inductive record declaring `family`, and its line.
    fn block(&self, family: &str) -> (usize, &Value) {
        self.rows
            .iter()
            .enumerate()
            .find(|(_, r)| {
                r["inductive"]["types"]
                    .as_array()
                    .is_some_and(|ts| ts.iter().any(|t| self.name(&t["name"]) == family))
            })
            .map(|(i, r)| (i, &r["inductive"]))
            .unwrap()
    }
    fn declaration(&self, kind: &str, name: &str) -> &Value {
        self.rows
            .iter()
            .find_map(|r| r.get(kind).filter(|d| self.name(&d["name"]) == name))
            .unwrap()
    }
}

#[test]
fn the_witness_is_leans_own_and_pins_every_decision_it_makes() {
    let p: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(
        p["source"],
        "crates/logicaffeine_tests/tests/fixtures/lean_semantics/LF_NestedFunctionField.lean"
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
        (Some(0), Some(0)),
        "Lean compiled the source, so every `#guard_msgs` in it held"
    );
    assert_eq!(p["export_sha256"], EXPORT_SHA);
    assert_eq!(p["records"], EXPORT.lines().count());
    assert_eq!(
        p["declarations"],
        json!(["LF_NF.mirror_mixed", "LF_NF.TFn.swap"])
    );
    assert_eq!(
        p["lean_print_axioms"],
        json!([
            "'LF_NF.mirror_mixed' does not depend on any axioms",
            "'LF_NF.TFn.swap' does not depend on any axioms",
        ])
    );
    assert_eq!(
        SOURCE.matches("#guard_msgs in").count(),
        8,
        "four pins and four refusals"
    );
    for pinned in [
        "#check @TFn.rec\n",
        "#check @TFn.rec_1\n",
        "#check @TFn.brecOn\n",
        "#print TFn.swap\n",
        "((a : Nat) → motive_1 (a_1 a))",
        "fun x =>\n  x.brecOn",
        "CFn.cons (x.2.1.1 7) (fun x_1 => x.1.1) x.2.2.1",
    ] {
        assert!(SOURCE.contains(pinned), "the source pins {pinned:?}");
    }
    for refusal in [
        "error: (kernel) arg #2 of '_nested.LF_NF.CNeg_1.cons' has a non positive occurrence of the datatypes being declared",
        "error: (kernel) arg #2 of '_nested.LF_NF.CDbl_1.cons' has a non positive occurrence of the datatypes being declared",
        "error: (kernel) arg #1 of 'LF_NF.GNeg.node' has a non positive occurrence of the datatypes being declared",
        "error: unknown identifier 'NotYetDeclared'",
    ] {
        assert!(SOURCE.contains(refusal), "Lean's refusal is pinned: {refusal}");
    }
    for accepted in [
        "| cons : α → (Nat → α) → CFn α → CFn α",
        "| cons : α → (Nat → List α) → CFnList α → CFnList α",
        "| cons : (Nat → Bool → α) → CCurry α → CCurry α",
        "| node : (Nat → List GTree) → GTree",
    ] {
        assert!(SOURCE.contains(accepted), "Lean accepts {accepted:?}");
    }
}

/// Lean's recursors for the block, as its export records them: `TFn.rec_1` for the auxiliary
/// `CFn TFn` first, then `TFn.rec`; the `cons` rule hands the minor premise the pointwise
/// hypothesis `fun a => TFn.rec … (f a)`; `brecOn` pairs the function field's results pointwise.
#[test]
fn leans_recursors_rules_and_brecon_for_the_function_field_are_pinned() {
    let lean = Lean::parse(EXPORT);
    let (_, block) = lean.block("LF_NF.TFn");
    let family = &block["types"][0];
    assert_eq!(
        (
            family["numNested"].as_u64(),
            family["isReflexive"].as_bool(),
            family["isRec"].as_bool()
        ),
        (Some(1), Some(true), Some(true)),
        "one nested occurrence, and a reflexive block"
    );
    let recs = block["recs"].as_array().unwrap();
    let inventory: Vec<(String, Vec<(String, u64)>)> = recs
        .iter()
        .map(|r| {
            (
                lean.name(&r["name"]),
                r["rules"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|rule| (lean.name(&rule["ctor"]), rule["nfields"].as_u64().unwrap()))
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        inventory,
        vec![
            (
                "LF_NF.TFn.rec_1".to_string(),
                vec![
                    ("LF_NF.CFn.nil".to_string(), 0),
                    ("LF_NF.CFn.cons".to_string(), 3)
                ]
            ),
            (
                "LF_NF.TFn.rec".to_string(),
                vec![("LF_NF.TFn.node".to_string(), 1)]
            ),
        ]
    );
    for r in recs {
        let counts: Vec<u64> = ["numParams", "numMotives", "numMinors", "numIndices"]
            .iter()
            .map(|k| r[*k].as_u64().unwrap())
            .collect();
        assert_eq!(counts, vec![0, 2, 3, 0]);
        assert_eq!(
            (r["k"].as_bool(), r["levelParams"].as_array().unwrap().len()),
            (Some(false), 1)
        );
        // The `cons` minor premise, after the two motives and the `node`/`nil` minors.
        let minor =
            &lean.exprs[&lean.under(&r["type"], "forallE", 4).as_u64().unwrap()]["forallE"]["type"];
        assert_eq!(lean.expr(minor),
            "(Π LF_NF.TFn, (Π (Π Nat, LF_NF.TFn), (Π (LF_NF.CFn LF_NF.TFn), (Π (#6 #2), (Π (Π Nat, (#8 (#3 #0))), \
             (Π (#7 #2), (#8 (LF_NF.CFn.cons LF_NF.TFn #5 #4 #3))))))))");
    }
    let rule = |rec: usize, index: usize, binders: usize| {
        lean.expr(&lean.under(&recs[rec]["rules"][index]["rhs"], "lam", binders))
    };
    assert_eq!(rule(0, 0, 5), "#1", "nil: the nil minor");
    assert_eq!(rule(0, 1, 8),
        "(#3 #2 #1 #0 (LF_NF.TFn.rec.{u} #7 #6 #5 #4 #3 #2) (fun Nat => (LF_NF.TFn.rec.{u} #8 #7 #6 #5 #4 (#2 #0))) \
         (LF_NF.TFn.rec_1.{u} #7 #6 #5 #4 #3 #0))",
        "cons: the head's hypothesis, the POINTWISE hypothesis at each point of the function field, the tail's");
    assert_eq!(
        rule(1, 0, 6),
        "(#3 #0 (LF_NF.TFn.rec_1.{u} #5 #4 #3 #2 #1 #0))",
        "node: the container's hypothesis"
    );

    let brec_on = lean.declaration("def", "LF_NF.TFn.brecOn");
    assert_eq!(lean.expr(&brec_on["type"]),
        "(Π (Π LF_NF.TFn, Sort u+1), (Π (Π (LF_NF.CFn LF_NF.TFn), Sort u+1), (Π LF_NF.TFn, (Π (Π LF_NF.TFn, \
         (Π (LF_NF.TFn.below.{u} #3 #2 #0), (#4 #1))), (Π (Π (LF_NF.CFn LF_NF.TFn), (Π (LF_NF.TFn.below_1.{u} #4 #3 #0), \
         (#4 #1))), (#4 #2))))))");
    assert!(lean.expr(&brec_on["value"]).contains(
        "(PProd.mk.{u+1,u+1} (Π Nat, (#11 (#5 #0))) (Π Nat, (LF_NF.TFn.below.{u} #11 #10 (#5 #0))) \
         (fun Nat => PProd.0((#2 #0))) (fun Nat => PProd.1((#2 #0))))"),
        "brecOn pairs the function field's results and their `below` pointwise");
}

/// The reader reconstructs Lean's recursors for the block exactly (it compares its own against
/// the export's records), and both kernels check every declaration of the witness: the recursors,
/// `below`, `brecOn`, the structural `swap` compiled through it, and `mirror mixed = swapped`, which
/// computes through the pointwise rule at the point 7. A universe-polymorphic declaration is
/// referenced at its own level parameters, and each is typed by the kernel before its agreement is
/// read: `DoubleCheck::Agreed` alone also means both kernels refused.
#[test]
fn logos_reconstructs_leans_recursors_and_both_kernels_check_the_witness() {
    let (ctx, report) =
        adapter::import_audited(EXPORT).unwrap_or_else(|v| panic!("Lean's witness imports: {v:?}"));
    for family in ["LF_NF.TFn", "LF_NF.CFn"] {
        assert!(
            ctx.is_inductive(&n(family)),
            "{family} is a family, never an axiom"
        );
    }
    for recursor in ["LF_NF.TFn.rec", "LF_NF.TFn.rec_1"] {
        let (levels, _, _) = ctx
            .get_universe_poly(&n(recursor))
            .unwrap_or_else(|| panic!("{recursor} is registered"));
        assert_eq!(levels.len(), 1, "{recursor} has Lean's one motive universe");
    }
    for declaration in [
        "LF_NF.TFn.rec",
        "LF_NF.TFn.rec_1",
        "LF_NF.TFn.below",
        "LF_NF.TFn.below_1",
        "LF_NF.TFn.brecOn",
        "LF_NF.TFn.swap",
        "LF_NF.mirror",
        "LF_NF.mirror_mixed",
    ] {
        let name = n(declaration);
        let reference = match ctx.poly_declaration(&name) {
            Some((levels, _)) if !levels.is_empty() => Term::Const {
                name: name.clone(),
                levels: levels.iter().cloned().map(Universe::Var).collect(),
            },
            _ => Term::Global(name.clone()),
        };
        assert!(
            infer_type(&ctx, &reference).is_ok(),
            "the kernel types {declaration}"
        );
        assert_eq!(
            double_check(&ctx, &reference),
            DoubleCheck::Agreed,
            "both kernels type {declaration}"
        );
        let usage = axiom_dependencies_of(&ctx, &name).unwrap();
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{declaration}: {usage:?}"
        );
    }
    // `CFn.swap` is spelled by the export (inside `CFn.swap.match_1`) but lies outside the closure of
    // its two roots, so Lean exported no declaration of it and nothing is registered under its name.
    let absent = n("LF_NF.CFn.swap");
    assert!(
        ctx.get_global(&absent).is_none()
            && ctx.poly_declaration(&absent).is_none()
            && axiom_dependencies_of(&ctx, &absent).is_none(),
        "CFn.swap is not a declaration of the export"
    );
    for root in ["LF_NF.mirror_mixed", "LF_NF.TFn.swap"] {
        let audit = report
            .declarations
            .iter()
            .find(|d| d.name == n(root))
            .unwrap();
        assert!(
            audit.axioms.is_empty(),
            "Lean: {root} depends on no axioms; got {:?}",
            audit.axioms
        );
    }
    // A package is built only for a result whose Lean acceptance was observed. The witness records
    // it: Lean 4.14.0 compiled the source (every `#guard_msgs` held) and `#print axioms` reported no
    // axiom for either root.
    let p: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(
        (p["compiler_exit"].as_i64(), p["exporter_exit"].as_i64()),
        (Some(0), Some(0))
    );
    let evidence = LeanEvidence::Accepted {
        axioms: Vec::new(),
        sources: [(
            "LF_NestedFunctionField.lean".to_string(),
            SOURCE.as_bytes().to_vec(),
        )]
        .into_iter()
        .collect(),
        toolchain: p["toolchain"].as_str().unwrap().into(),
        exporter: format!("lean4export {}", p["exporter_commit"].as_str().unwrap()),
    };
    for root in ["LF_NF.mirror_mixed", "LF_NF.TFn.swap"] {
        let checked = check_export(EXPORT, root, &evidence, &Limits::default());
        for stage in [
            "lean_accepted",
            "translated",
            "primary_check",
            "independent_recheck",
            "assumption_policy",
            "emission_reparsed",
        ] {
            assert_eq!(
                checked.result.status(stage),
                StageStatus::Passed,
                "{root} {stage}: {}",
                checked.result.to_json()
            );
        }
        assert!(
            checked.package.is_some(),
            "{root} is published: {}",
            checked.result.to_json()
        );
    }
}

/// The computation is the rule's, not a coincidence of the witness: claiming `mirror mixed` is the
/// unmirrored tree, or the tree the head's hypothesis would give at the function field's point, is
/// refuted by both kernels, while the true claim is checked by both.
#[test]
fn the_pointwise_hypothesis_is_computed_at_its_point_in_both_kernels() {
    let (ctx, _) = adapter::import_audited(EXPORT).unwrap();
    let tfn = g(&n("LF_NF.TFn"));
    let mirrored = app(g(&n("LF_NF.mirror")), g(&n("LF_NF.mixed")));
    let claim = |rhs: &Term| {
        let level = vec![Universe::Type(0)];
        let eq = apps(
            Term::Const {
                name: n("Eq"),
                levels: level.clone(),
            },
            &[tfn.clone(), mirrored.clone(), rhs.clone()],
        );
        let refl = apps(
            Term::Const {
                name: n("rfl"),
                levels: level,
            },
            &[tfn.clone(), mirrored.clone()],
        );
        app(lam("h", eq, v("h")), refl)
    };
    let truth = claim(&g(&n("LF_NF.swapped")));
    infer_type(&ctx, &truth).expect("the primary kernel computes mirror mixed = swapped");
    recheck(&ctx, &truth).expect("the independent kernel computes mirror mixed = swapped");
    for (what, wrong) in [
        ("the unmirrored tree", g(&n("LF_NF.mixed"))),
        ("the fan", g(&n("LF_NF.fan"))),
    ] {
        let claim = claim(&wrong);
        assert!(
            infer_type(&ctx, &claim).is_err(),
            "the primary kernel refutes mirror mixed = {what}"
        );
        match recheck(&ctx, &claim) {
            Err(ReCheckError::Ill(_)) => {}
            other => panic!("the independent kernel refutes mirror mixed = {what}: {other:?}"),
        }
    }
}

/// Forged copies of Lean's export: new expression records are added just before the block and
/// the record under test is pointed at them, so every other byte is Lean's.
struct Forge<'a> {
    lean: &'a Lean,
    next: u64,
    fresh: Vec<Value>,
}

impl<'a> Forge<'a> {
    fn new(lean: &'a Lean) -> Self {
        Forge {
            lean,
            next: lean.exprs.keys().max().unwrap() + 1,
            fresh: Vec::new(),
        }
    }
    fn add(&mut self, mut expr: Value) -> Value {
        let id = self.next;
        self.next += 1;
        expr["ie"] = json!(id);
        self.fresh.push(expr);
        json!(id)
    }
    /// `id` with the subterm at `path` replaced by `leaf`: every expression on the path is copied,
    /// so a subterm the export shares elsewhere is never changed there.
    fn replace(&mut self, id: &Value, path: &[(&str, &str)], leaf: &Value) -> Value {
        let Some(((kind, field), rest)) = path.split_first() else {
            return leaf.clone();
        };
        let mut expr = self.lean.exprs[&id.as_u64().unwrap()].clone();
        let child = expr[*kind][*field].clone();
        expr[*kind][*field] = self.replace(&child, rest, leaf);
        expr.as_object_mut().unwrap().remove("ie");
        self.add(expr)
    }
    fn at(&self, id: &Value, path: &[(&str, &str)]) -> Value {
        path.iter().fold(id.clone(), |at, (kind, field)| {
            self.lean.exprs[&at.as_u64().unwrap()][*kind][*field].clone()
        })
    }
    /// The export with the block's record replaced by `block` and the new expressions before it.
    fn text(self, block: Value) -> String {
        let (line, _) = self.lean.block("LF_NF.TFn");
        let mut rows = self.lean.rows.clone();
        rows[line] = json!({ "inductive": block });
        rows.splice(line..line, self.fresh);
        rows.iter().map(|r| format!("{r}\n")).collect()
    }
}

const LAM: (&str, &str) = ("lam", "body");
const PI: (&str, &str) = ("forallE", "body");
const FN: (&str, &str) = ("app", "fn");
const ARG: (&str, &str) = ("app", "arg");

/// A wrong induction hypothesis, a wrong minor premise and a swapped field, each forged into Lean's
/// export: the reader rejects every one as an invalid proof and publishes nothing.
#[test]
fn a_forged_recursor_for_the_function_field_is_rejected() {
    let lean = Lean::parse(EXPORT);
    let (_, block) = lean.block("LF_NF.TFn");
    let cons_rule = |b: &Value| b["recs"][0]["rules"][1]["rhs"].clone();
    // The cons rule body after its 8 binders: `cons a f t ih_a (fun x => rec (f x)) ih_t`.
    let body: Vec<(&str, &str)> = vec![LAM; 8];
    let mut forged = Vec::new();

    // 1. The pointwise hypothesis taken at the HEAD, `fun x => rec a`, instead of at `f x`.
    let mut forge = Forge::new(&lean);
    let mut b = block.clone();
    let path = [&body[..], &[FN, ARG, LAM, ARG]].concat();
    assert_eq!(
        lean.expr(&forge.at(&cons_rule(block), &path)),
        "(#2 #0)",
        "the pointwise point `f x`"
    );
    let head = forge.add(json!({"bvar": 3}));
    b["recs"][0]["rules"][1]["rhs"] = forge.replace(&cons_rule(block), &path, &head);
    forged.push(("a wrong induction hypothesis", forge.text(b)));

    // 2. `TFn.rec`'s minor premise for cons with the pointwise hypothesis replaced by the head's.
    let mut forge = Forge::new(&lean);
    let mut b = block.clone();
    let ty = block["recs"][1]["type"].clone();
    let path = [
        &[PI; 4][..],
        &[("forallE", "type")],
        &[PI; 4],
        &[("forallE", "type")],
    ]
    .concat();
    assert_eq!(
        lean.expr(&forge.at(&ty, &path)),
        "(Π Nat, (#8 (#3 #0)))",
        "the pointwise hypothesis"
    );
    let motive = forge.add(json!({"bvar": 7}));
    let at_head = forge.add(json!({"bvar": 3}));
    let head_hypothesis = forge.add(json!({"app": {"fn": motive, "arg": at_head}}));
    b["recs"][1]["type"] = forge.replace(&ty, &path, &head_hypothesis);
    forged.push(("a wrong minor premise", forge.text(b)));

    // 3. The cons rule hands the minor premise the head and the tail swapped.
    let mut forge = Forge::new(&lean);
    let mut b = block.clone();
    let head_at = [&body[..], &[FN, FN, FN, FN, FN, ARG]].concat();
    let tail_at = [&body[..], &[FN, FN, FN, ARG]].concat();
    let (head, tail) = (
        forge.at(&cons_rule(block), &head_at),
        forge.at(&cons_rule(block), &tail_at),
    );
    assert_eq!(
        (lean.expr(&head), lean.expr(&tail)),
        ("#2".to_string(), "#0".to_string())
    );
    let swapped_once = forge.replace(&cons_rule(block), &head_at, &tail);
    let swapped = {
        let mut rows = lean.exprs.clone();
        for row in &forge.fresh {
            rows.insert(row["ie"].as_u64().unwrap(), row.clone());
        }
        let relean = Lean {
            rows: Vec::new(),
            names: HashMap::new(),
            levels: HashMap::new(),
            exprs: rows,
        };
        let mut again = Forge {
            lean: &relean,
            next: forge.next,
            fresh: Vec::new(),
        };
        let id = again.replace(&swapped_once, &tail_at, &head);
        let (next, fresh) = (again.next, again.fresh);
        forge.next = next;
        forge.fresh.extend(fresh);
        id
    };
    b["recs"][0]["rules"][1]["rhs"] = swapped;
    forged.push(("a swapped field", forge.text(b)));

    for (what, text) in forged {
        let verdict = adapter::check_str(&text).expect_err(what);
        assert!(matches!(verdict, Verdict::Reject(_)), "{what}: {verdict:?}");
        assert_eq!(
            verdict.reason().code(),
            DiagnosticCode::InvalidProof,
            "{what}: {verdict:?}"
        );
    }
    adapter::check_str(EXPORT).expect("the unforged export is accepted");
}
